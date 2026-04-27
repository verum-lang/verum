//! End-to-end script-mode context (P3.3 + P1.7 + P2.2 + P3.1 glue).
//!
//! The script runner needs to gather a lot of state before it can
//! execute: source bytes, source hash, frontmatter, merged permission
//! set, cache key, lockfile path. Doing this inline at every call site
//! invites drift; this module centralises it as a single
//! [`ScriptContext`] value built in one shot from a path + caller
//! options.
//!
//! ```text
//!   path on disk
//!        │
//!        ▼
//!   read bytes  ──►  blake3 hash  ──►  cache key
//!        │
//!        ▼
//!   extract frontmatter  ──►  validate
//!        │
//!        ▼
//!   merge frontmatter + CLI flags  ──►  PermissionSet
//!        │
//!        ▼
//!   ScriptContext { everything above }
//! ```
//!
//! No I/O happens beyond `fs::read` of the script itself — cache and
//! lockfile *paths* are computed but not opened. The caller decides
//! when to consult them via [`ScriptContext::cache_lookup`] /
//! [`ScriptContext::cache_store`] / [`ScriptContext::lockfile_path`].

use crate::script::cache::{key_for, CacheEntry, CacheError, CacheKey, ScriptCache};
use crate::script::frontmatter::{self, Frontmatter, FrontmatterError};
use crate::script::lockfile::ScriptLockfile;
use crate::script::permission_flags::{
    build_permission_set, BuildError as PermissionBuildError, PermissionFlags,
};
use crate::script::permissions::PermissionSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// All script-mode state a runner needs to start executing.
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// Canonical path to the script on disk. Best-effort canonical —
    /// falls back to the input path if the OS canonicaliser refuses
    /// (Windows long paths, missing intermediate symlinks, etc.).
    pub source_path: PathBuf,

    /// Raw source bytes. Held in memory because every downstream step
    /// (lex, hash, frontmatter extract) needs them; reading once is
    /// cheaper than re-reading.
    pub source: Vec<u8>,

    /// blake3 hex digest of `source`. Used as both the lockfile
    /// integrity check and (mixed with compiler + flags) the cache
    /// key.
    pub source_hash: String,

    /// Parsed inline metadata, if any. `None` for plain scripts with
    /// no `// /// script` block.
    pub frontmatter: Option<Frontmatter>,

    /// Merged permission set: frontmatter + run defaults + CLI flags,
    /// or the [`--allow-all`/`--deny-all`] override result.
    pub permissions: PermissionSet,

    /// Cache key for this (source, compiler, extra-flags) tuple.
    pub cache_key: CacheKey,

    /// Toolchain version that built this context. Pinned so cache
    /// stores carry the right diagnostic and lockfile verification
    /// keys on the same string.
    pub compiler_version: String,
}

/// Caller-supplied options. Toolchain + flags are required; permission
/// CLI flags are optional (default = no extra grants).
#[derive(Debug, Clone, Default)]
pub struct ScriptContextOptions {
    /// CLI permission flags (`--allow`, `--allow-all`, `--deny-all`).
    pub flags: PermissionFlags,

    /// Toolchain version stamp. Mixed into the cache key so a compiler
    /// upgrade invalidates the cache, and stored verbatim in the
    /// lockfile so verify_against can detect drift.
    pub compiler_version: String,

    /// Extra cache-key contributors: profile tier, opt-level, verify
    /// mode, etc. Order is preserved and contributes to the digest, so
    /// callers should canonicalise (e.g. sort) before passing.
    pub extra_cache_flags: Vec<String>,
}

/// Failure mode for [`ScriptContext::from_path`].
#[derive(Debug)]
pub enum ScriptContextError {
    /// I/O error reading the script file. Path is the offending one.
    Io { path: PathBuf, source: io::Error },
    /// Frontmatter extraction or validation failed.
    Frontmatter(FrontmatterError),
    /// Permission flag merging failed (bad scope, conflicting overrides).
    Permissions(PermissionBuildError),
}

impl std::fmt::Display for ScriptContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "read script {}: {source}", path.display())
            }
            Self::Frontmatter(e) => write!(f, "frontmatter: {e}"),
            Self::Permissions(e) => write!(f, "permissions: {e}"),
        }
    }
}

impl std::error::Error for ScriptContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Frontmatter(e) => Some(e),
            Self::Permissions(e) => Some(e),
        }
    }
}

impl From<FrontmatterError> for ScriptContextError {
    fn from(e: FrontmatterError) -> Self {
        Self::Frontmatter(e)
    }
}

impl From<PermissionBuildError> for ScriptContextError {
    fn from(e: PermissionBuildError) -> Self {
        Self::Permissions(e)
    }
}

impl ScriptContext {
    /// Build a context from a path on disk + caller options. Reads the
    /// file, extracts and validates the frontmatter, computes the source
    /// hash and cache key, and merges permission sources.
    pub fn from_path(
        path: &Path,
        opts: &ScriptContextOptions,
    ) -> Result<Self, ScriptContextError> {
        let source = fs::read(path).map_err(|source| ScriptContextError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_source(path, source, opts)
    }

    /// Variant that takes already-read source bytes — useful for tests
    /// and for callers that have a synthetic in-memory source (e.g.
    /// `verum -e '<inline-program>'`).
    pub fn from_source(
        path: &Path,
        source: Vec<u8>,
        opts: &ScriptContextOptions,
    ) -> Result<Self, ScriptContextError> {
        let source_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let source_hash = ScriptLockfile::hash_source(&source);
        let frontmatter = match std::str::from_utf8(&source) {
            Ok(text) => frontmatter::extract_and_validate(text)?.map(|e| e.frontmatter),
            // Non-UTF-8 source can't have a frontmatter (line comments
            // are required to be UTF-8). Skip rather than reject — the
            // downstream lexer will diagnose as needed.
            Err(_) => None,
        };
        let permissions = build_permission_set(frontmatter.as_ref(), &opts.flags)?;

        let extra_refs: Vec<&str> = opts.extra_cache_flags.iter().map(|s| s.as_str()).collect();
        let cache_key = key_for(&source, &opts.compiler_version, &extra_refs);

        Ok(Self {
            source_path,
            source,
            source_hash,
            frontmatter,
            permissions,
            cache_key,
            compiler_version: opts.compiler_version.clone(),
        })
    }

    /// Conventional sidecar lockfile path: `<script>.lock` next to the
    /// source. Computed lazily — caller decides whether to actually
    /// read/write it.
    pub fn lockfile_path(&self) -> PathBuf {
        ScriptLockfile::sidecar_path(&self.source_path)
    }

    /// Cache lookup using this context's [`cache_key`]. Returns
    /// `Ok(Some(entry))` on hit, `Ok(None)` on miss, `Err(e)` on I/O
    /// failure or schema-incompatibility.
    pub fn cache_lookup(&self, cache: &ScriptCache) -> Result<Option<CacheEntry>, CacheError> {
        cache.lookup(self.cache_key)
    }

    /// Cache store for the compiled VBC produced from this context's
    /// source. Consistent metadata: source path + length + compiler
    /// version are pinned to the values this context already holds.
    pub fn cache_store(&self, cache: &ScriptCache, vbc: &[u8]) -> Result<(), CacheError> {
        cache.store(
            self.cache_key,
            vbc,
            self.source_path.display().to_string(),
            self.source.len() as u64,
            self.compiler_version.clone(),
        )
    }

    /// Construct a fresh lockfile for this run. Caller fills in `deps`
    /// from whichever resolver handled the frontmatter dependency list;
    /// this convenience pre-fills the source hash, path, and compiler.
    pub fn fresh_lockfile(
        &self,
        deps: Vec<crate::script::lockfile::LockedDep>,
    ) -> ScriptLockfile {
        ScriptLockfile::new(
            self.source_path.clone(),
            self.source_hash.clone(),
            self.compiler_version.clone(),
            deps,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::cache::ScriptCache;
    use crate::script::permissions::PermissionRequest;
    use std::fs;
    use tempfile::TempDir;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).expect("write script");
        path
    }

    fn opts_with_compiler(version: &str) -> ScriptContextOptions {
        ScriptContextOptions {
            flags: PermissionFlags::default(),
            compiler_version: version.to_string(),
            extra_cache_flags: Vec::new(),
        }
    }

    #[test]
    fn from_path_plain_script_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "plain.vr", "fn main() { 0 }\n");
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        assert!(ctx.frontmatter.is_none());
        assert!(ctx.permissions.is_empty());
        assert_eq!(ctx.compiler_version, "0.1.0");
        assert_eq!(ctx.source, b"fn main() { 0 }\n");
        assert_eq!(ctx.source_hash.len(), 64);
    }

    #[test]
    fn from_path_with_frontmatter_and_permissions() {
        let tmp = TempDir::new().unwrap();
        let body = "// /// script\n\
                    // permissions = [\"net=api.example.com:443\"]\n\
                    // ///\n\
                    fn main() { 0 }\n";
        let path = write_script(tmp.path(), "x.vr", body);
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        assert!(ctx.frontmatter.is_some());
        assert_eq!(ctx.frontmatter.as_ref().unwrap().permissions.len(), 1);
        assert!(ctx
            .permissions
            .check(&PermissionRequest::Net {
                host: "api.example.com",
                port: Some(443),
            })
            .is_ok());
    }

    #[test]
    fn from_path_returns_io_error_for_missing_script() {
        let tmp = TempDir::new().unwrap();
        let absent = tmp.path().join("absent.vr");
        let err = ScriptContext::from_path(&absent, &opts_with_compiler("0.1.0")).unwrap_err();
        assert!(matches!(err, ScriptContextError::Io { .. }));
    }

    #[test]
    fn cli_flags_extend_frontmatter_grants() {
        let tmp = TempDir::new().unwrap();
        let body = "// /// script\n// permissions = [\"net=api.x\"]\n// ///\nfn main() {}\n";
        let path = write_script(tmp.path(), "x.vr", body);
        let opts = ScriptContextOptions {
            flags: PermissionFlags {
                allow: vec!["fs:read=./data".to_string()],
                ..Default::default()
            },
            compiler_version: "0.1.0".to_string(),
            extra_cache_flags: Vec::new(),
        };
        let ctx = ScriptContext::from_path(&path, &opts).unwrap();
        assert_eq!(ctx.permissions.len(), 2);
        assert!(ctx
            .permissions
            .check(&PermissionRequest::FsRead(Path::new("./data/file")))
            .is_ok());
    }

    #[test]
    fn deny_all_overrides_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let body = "// /// script\n// permissions = [\"net\"]\n// ///\nfn main() {}\n";
        let path = write_script(tmp.path(), "x.vr", body);
        let opts = ScriptContextOptions {
            flags: PermissionFlags {
                deny_all: true,
                ..Default::default()
            },
            compiler_version: "0.1.0".to_string(),
            extra_cache_flags: Vec::new(),
        };
        let ctx = ScriptContext::from_path(&path, &opts).unwrap();
        assert!(ctx.permissions.is_empty());
    }

    #[test]
    fn invalid_frontmatter_permission_surfaces_as_error() {
        let tmp = TempDir::new().unwrap();
        let body = "// /// script\n// permissions = [\"kernel=/dev/mem\"]\n// ///\nfn main() {}\n";
        let path = write_script(tmp.path(), "x.vr", body);
        let err = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap_err();
        // Frontmatter validate catches it first.
        assert!(matches!(err, ScriptContextError::Frontmatter(_)));
    }

    #[test]
    fn cache_key_changes_with_compiler() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "x.vr", "fn main() {}\n");
        let a = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        let b = ScriptContext::from_path(&path, &opts_with_compiler("0.2.0")).unwrap();
        assert_ne!(a.cache_key, b.cache_key);
    }

    #[test]
    fn cache_key_changes_with_extra_flags() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "x.vr", "fn main() {}\n");
        let plain = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        let with_flags = ScriptContext::from_path(
            &path,
            &ScriptContextOptions {
                compiler_version: "0.1.0".into(),
                extra_cache_flags: vec!["tier=1".into(), "opt=2".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(plain.cache_key, with_flags.cache_key);
    }

    #[test]
    fn cache_key_stable_across_runs() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "x.vr", "fn main() { 1 + 1 }\n");
        let a = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        let b = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        assert_eq!(a.cache_key, b.cache_key);
        assert_eq!(a.source_hash, b.source_hash);
    }

    #[test]
    fn lockfile_path_is_sidecar() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "script.vr", "fn main() {}\n");
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        let lock_path = ctx.lockfile_path();
        assert!(lock_path.to_string_lossy().ends_with("script.vr.lock"));
    }

    #[test]
    fn cache_store_then_lookup_round_trip() {
        let tmp = TempDir::new().unwrap();
        let cache_root = tmp.path().join("cache");
        let cache = ScriptCache::at(cache_root).unwrap();
        let path = write_script(tmp.path(), "x.vr", "fn main() { 7 }\n");
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();

        // Initial lookup is a miss.
        assert!(ctx.cache_lookup(&cache).unwrap().is_none());

        // Store + re-lookup hits.
        ctx.cache_store(&cache, b"\x01\x02\x03 fake VBC").unwrap();
        let entry = ctx.cache_lookup(&cache).unwrap().expect("hit");
        assert_eq!(entry.vbc, b"\x01\x02\x03 fake VBC");
        assert_eq!(entry.meta.compiler_version, "0.1.0");
        assert_eq!(entry.meta.source_len, ctx.source.len() as u64);
        assert!(entry.meta.source_path.contains("x.vr"));
    }

    #[test]
    fn fresh_lockfile_pins_context_metadata() {
        let tmp = TempDir::new().unwrap();
        let path = write_script(tmp.path(), "x.vr", "fn main() {}\n");
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.7.3")).unwrap();
        let lf = ctx.fresh_lockfile(Vec::new());
        assert_eq!(lf.source_hash, ctx.source_hash);
        assert_eq!(lf.compiler_version, "0.7.3");
        assert!(lf.deps.is_empty());
    }

    #[test]
    fn from_source_skips_disk_io() {
        let path = Path::new("/non/existent/synthetic.vr");
        let ctx = ScriptContext::from_source(
            path,
            b"fn main() {}\n".to_vec(),
            &opts_with_compiler("0.1.0"),
        )
        .unwrap();
        // Source path falls back to input when canonicalize fails.
        assert!(ctx.source_path.to_string_lossy().contains("synthetic.vr"));
        assert!(ctx.frontmatter.is_none());
    }

    #[test]
    fn non_utf8_source_skips_frontmatter_silently() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bin.vr");
        // Mid-byte invalid UTF-8 sequence.
        fs::write(&path, &[0xFFu8, 0xFE, 0xFD, b'\n']).unwrap();
        let ctx = ScriptContext::from_path(&path, &opts_with_compiler("0.1.0")).unwrap();
        assert!(ctx.frontmatter.is_none());
        assert!(ctx.permissions.is_empty());
    }
}
