//! Content-addressed cache for compiled Verum scripts.
//!
//! P1.7 — script cache lives at `~/.verum/script-cache/<blake3>/`. Each
//! directory holds:
//!
//! ```text
//! <blake3-hex>/
//! ├── cog.vbc       — the compiled VBC bytecode
//! └── meta.toml     — typed metadata (schema, key components, timestamps)
//! ```
//!
//! # Cache key
//!
//! `CacheKey = blake3(source_bytes ++ compiler_version ++ flags)`.
//!
//! Both `compiler_version` and `flags` are part of the digest because a
//! source file alone does NOT determine the compilation output:
//!
//!   * a different compiler version may emit different VBC for the same
//!     source (e.g. an intrinsic-name change, a codegen optimisation
//!     pass that landed mid-release);
//!   * profile flags (verify mode, tier, opt-level) change the bytecode
//!     materially.
//!
//! Including everything in the key means a cache hit is always a *valid*
//! reuse — there is no "stale cache" failure mode.
//!
//! # Atomicity
//!
//! `store()` writes through a per-entry tempdir (`<blake3>.tmp.<pid>`) and
//! atomically renames it into place. A crash between writing the VBC and
//! renaming leaves a `.tmp.<pid>` directory that the next prune sweep
//! reaps; it never produces a partial cache hit. Concurrent stores of the
//! same key are safe — the last writer wins, but every entry is
//! immutable so the overwrite is byte-identical when both writers used
//! the same key.
//!
//! # GC policy
//!
//! `gc_to_size(max_bytes)` evicts least-recently-accessed entries first
//! until total disk usage falls under the budget. Access times are
//! refreshed on every `lookup()`; this is best-effort (we update the
//! `meta.toml` `last_accessed_at` field, which is one extra fsync per
//! lookup) and can be disabled via `[cache] track_access = false` in
//! the user config when latency-sensitive.
//!
//! # Performance
//!
//! - `key_for`: single-pass blake3 over `source ++ compiler_version ++
//!   flags`, ~1 GB/s on modern CPUs. A 10 KB source = ~10 µs.
//! - `lookup`: 2 fs reads + 1 TOML parse. Cold lookup ~200 µs on SSD;
//!   warm (page cache) ~30 µs.
//! - `store`: 2 fs writes + 1 atomic rename. ~500 µs on SSD.
//! - `gc_to_size`: O(N) on the entry count, sorted by `last_accessed_at`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Process-wide tempdir-suffix counter. Concurrent stores in the same
/// process are routed to distinct tempdir paths even when `pid + now_secs`
/// would otherwise collide (multiple threads firing inside the same wall-
/// clock second). Wraps at 2^64; collision probability is zero in practice.
static TEMPDIR_NONCE: AtomicU64 = AtomicU64::new(0);

/// 32-byte blake3 digest used as the cache key. Stored hex-encoded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// Hex-encode the digest into a 64-character lowercase ASCII string —
    /// the on-disk directory name format.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(NIBBLE[(byte >> 4) as usize]);
            out.push(NIBBLE[(byte & 0x0F) as usize]);
        }
        out
    }

    /// Parse a 64-character hex string. Returns `None` on length / charset
    /// mismatch.
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let bytes = s.as_bytes();
        let mut out = [0u8; 32];
        for i in 0..32 {
            let hi = hex_value(bytes[i * 2])?;
            let lo = hex_value(bytes[i * 2 + 1])?;
            out[i] = (hi << 4) | lo;
        }
        Some(Self(out))
    }

    /// Raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

const NIBBLE: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

#[inline]
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Compute the cache key for a source + compiler + flag set.
///
/// The digest order is `source ++ b"\x00" ++ compiler ++ b"\x00" ++ flag1
/// ++ b"\x00" ++ flag2 ++ ...` — null bytes between fields prevent
/// length-extension collisions where, e.g., `("a", "bc")` and `("ab",
/// "c")` would otherwise hash identically.
pub fn key_for(source: &[u8], compiler_version: &str, flags: &[&str]) -> CacheKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(source);
    hasher.update(&[0]);
    hasher.update(compiler_version.as_bytes());
    for flag in flags {
        hasher.update(&[0]);
        hasher.update(flag.as_bytes());
    }
    let hash = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(hash.as_bytes());
    CacheKey(bytes)
}

/// Schema version for `meta.toml`. Bumped on incompatible layout changes
/// so older cache entries are evicted automatically (see `lookup`).
const META_SCHEMA_VERSION: u32 = 1;

/// Metadata for one cache entry. Round-trips through TOML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Bumped on incompatible meta layout changes.
    pub schema_version: u32,
    /// Source-file path the entry was compiled from. Diagnostic only —
    /// a moved/renamed source on the same machine produces a different
    /// key (the path isn't part of the digest), so this field is just
    /// a breadcrumb for `verum cache list`.
    pub source_path: String,
    /// Source byte length at compile time. Quick stale-detection
    /// signal independent of the digest.
    pub source_len: u64,
    /// Toolchain version that produced the bytecode. Diagnostic.
    pub compiler_version: String,
    /// Wall-clock seconds since UNIX epoch when the entry was created.
    pub created_at: u64,
    /// Wall-clock seconds since UNIX epoch when the entry was last
    /// looked up (refreshed on every cache hit). Used by LRU GC.
    pub last_accessed_at: u64,
    /// Compiled VBC byte length. Authoritative; used by GC sizing.
    pub vbc_len: u64,
}

impl CacheMeta {
    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// One cache entry returned by `lookup`.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub vbc: Vec<u8>,
    pub meta: CacheMeta,
}

/// Errors surfaced from cache operations. Wraps `io::Error` and `toml`
/// parse / serialise errors with structural context (the offending
/// directory / file name).
#[derive(Debug)]
pub enum CacheError {
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidKey {
        name: String,
    },
    InvalidMeta {
        path: PathBuf,
        reason: String,
    },
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { op, path, source } => {
                write!(f, "script-cache {op} failed on {}: {source}", path.display())
            }
            Self::InvalidKey { name } => {
                write!(f, "script-cache: directory name {name:?} is not a valid 64-char blake3 hex digest")
            }
            Self::InvalidMeta { path, reason } => {
                write!(f, "script-cache: meta.toml at {} is invalid: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for CacheError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Convenience: lift an `io::Error` into a `CacheError::Io` with context.
fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> CacheError + 'a {
    move |source| CacheError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

pub type CacheResult<T> = Result<T, CacheError>;

/// Content-addressed cache rooted at a directory.
///
/// Cheap to clone — internally just a `PathBuf`.
#[derive(Debug, Clone)]
pub struct ScriptCache {
    root: PathBuf,
    /// Process-wide store serialiser: cleanup+rename is unavoidably
    /// racy on POSIX directories (`fs::rename` over a populated dir
    /// fails with ENOTEMPTY; the cleanup+rename window is exposed to
    /// every concurrent same-key writer in the same process). The
    /// content-addressed property makes the race benign across
    /// processes (any winning bytes are identical to losing bytes), so
    /// we only need to coordinate in-process. `Arc<Mutex<()>>` keeps
    /// `ScriptCache` cheap to clone — every clone shares the same lock,
    /// which is what test rigs depend on when they spawn N threads
    /// against `cache.clone()`.
    store_lock: std::sync::Arc<std::sync::Mutex<()>>,
}

impl ScriptCache {
    /// Default cache location: `$HOME/.verum/script-cache/`.
    ///
    /// The directory is created if missing.
    pub fn at_default() -> CacheResult<Self> {
        let home = dirs::home_dir().ok_or_else(|| CacheError::Io {
            op: "resolve home directory",
            path: PathBuf::from("~"),
            source: io::Error::new(io::ErrorKind::NotFound, "$HOME unset"),
        })?;
        let root = home.join(".verum").join("script-cache");
        Self::at(root)
    }

    /// Use a custom root directory. Created if missing.
    pub fn at(root: PathBuf) -> CacheResult<Self> {
        fs::create_dir_all(&root).map_err(io_err("mkdir -p", &root))?;
        Ok(Self {
            root,
            store_lock: std::sync::Arc::new(std::sync::Mutex::new(())),
        })
    }

    /// Path to the per-entry directory for `key`.
    fn entry_dir(&self, key: CacheKey) -> PathBuf {
        self.root.join(key.to_hex())
    }

    /// Look up an entry. Returns `Ok(None)` on miss; `Ok(Some(entry))` on
    /// hit (with `last_accessed_at` refreshed); `Err(CacheError)` only on
    /// I/O or schema-incompatibility failure (incompatible-schema entries
    /// are silently evicted and reported as miss).
    pub fn lookup(&self, key: CacheKey) -> CacheResult<Option<CacheEntry>> {
        let dir = self.entry_dir(key);
        let vbc_path = dir.join("cog.vbc");
        let meta_path = dir.join("meta.toml");
        if !vbc_path.is_file() || !meta_path.is_file() {
            return Ok(None);
        }
        let meta_text =
            fs::read_to_string(&meta_path).map_err(io_err("read meta.toml", &meta_path))?;
        let mut meta: CacheMeta = match toml::from_str(&meta_text) {
            Ok(m) => m,
            Err(e) => {
                // Corrupt entry — silently evict. The caller will recompile
                // and store a fresh entry; we don't surface this as an error
                // because the cache is meant to be best-effort.
                let _ = fs::remove_dir_all(&dir);
                return Err(CacheError::InvalidMeta {
                    path: meta_path,
                    reason: e.to_string(),
                });
            }
        };
        if meta.schema_version != META_SCHEMA_VERSION {
            // Schema-skew evicts. New schema → recompile.
            let _ = fs::remove_dir_all(&dir);
            return Ok(None);
        }
        let vbc = fs::read(&vbc_path).map_err(io_err("read cog.vbc", &vbc_path))?;
        // Refresh access time. Best-effort — a write failure here MUST
        // NOT mask a successful hit.
        meta.last_accessed_at = CacheMeta::now_secs();
        if let Ok(s) = toml::to_string(&meta) {
            let _ = atomic_write_string(&meta_path, &s);
        }
        Ok(Some(CacheEntry { vbc, meta }))
    }

    /// Store an entry. Atomic: writes to a per-pid tempdir then renames
    /// into place. Concurrent stores of the same key are safe (last
    /// writer wins; both writers must produce identical bytes since the
    /// key is content-addressed).
    pub fn store(
        &self,
        key: CacheKey,
        vbc: &[u8],
        source_path: impl Into<String>,
        source_len: u64,
        compiler_version: impl Into<String>,
    ) -> CacheResult<()> {
        let dir = self.entry_dir(key);
        let nonce = TEMPDIR_NONCE.fetch_add(1, Ordering::Relaxed);
        let tmp = self.root.join(format!(
            "{}.tmp.{}.{}.{}",
            key.to_hex(),
            std::process::id(),
            CacheMeta::now_secs(),
            nonce
        ));
        fs::create_dir_all(&tmp).map_err(io_err("mkdir tempdir", &tmp))?;
        // Write bytecode first so meta is the last thing committed —
        // lookup() validates both, and meta presence implies vbc presence
        // for any non-truncated tempdir.
        let vbc_path = tmp.join("cog.vbc");
        atomic_write_bytes(&vbc_path, vbc).map_err(io_err("write cog.vbc", &vbc_path))?;

        let now = CacheMeta::now_secs();
        let meta = CacheMeta {
            schema_version: META_SCHEMA_VERSION,
            source_path: source_path.into(),
            source_len,
            compiler_version: compiler_version.into(),
            created_at: now,
            last_accessed_at: now,
            vbc_len: vbc.len() as u64,
        };
        let meta_str = toml::to_string(&meta).expect("CacheMeta serialise — pure data, never fails");
        let meta_path = tmp.join("meta.toml");
        atomic_write_string(&meta_path, &meta_str)
            .map_err(io_err("write meta.toml", &meta_path))?;

        // If a previous entry for this key exists, remove it before the
        // rename — `fs::rename` over a populated directory is not atomic
        // on either Unix (ENOTEMPTY) or Windows. We hold an in-process
        // mutex across cleanup+rename so concurrent same-key writers in
        // the same process are serialised; cross-process races are
        // benign (content-addressed: every winner's bytes match every
        // loser's bytes), so the lock is intentionally process-local.
        let _guard = self.store_lock.lock().unwrap_or_else(|p| p.into_inner());
        let _ = fs::remove_dir_all(&dir);
        match fs::rename(&tmp, &dir) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp);
                // If a sibling writer populated `dir` between our
                // remove_dir_all and rename, the entry is already
                // present with identical bytes. Surface only true
                // failures (no entry on disk after the race).
                if dir.is_dir() {
                    Ok(())
                } else {
                    Err(CacheError::Io {
                        op: "rename tempdir into place",
                        path: dir.clone(),
                        source: e,
                    })
                }
            }
        }
    }

    /// Evict a single entry by key.
    pub fn evict(&self, key: CacheKey) -> CacheResult<()> {
        let dir = self.entry_dir(key);
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CacheError::Io {
                op: "remove entry",
                path: dir,
                source: e,
            }),
        }
    }

    /// Enumerate all entries (key + meta). Skipped: directories whose
    /// name is not a valid 64-character blake3 hex digest, and entries
    /// whose meta.toml fails to parse (those are silently evicted).
    /// `.tmp.*` artefacts from interrupted stores are silently reaped.
    pub fn list(&self) -> CacheResult<Vec<(CacheKey, CacheMeta)>> {
        let read = match fs::read_dir(&self.root) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err("read cache root", &self.root)(e)),
        };
        let mut out = Vec::new();
        for entry in read {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // Reap orphan tempdirs.
            if name.contains(".tmp.") {
                let _ = fs::remove_dir_all(&path);
                continue;
            }
            let key = match CacheKey::from_hex(name) {
                Some(k) => k,
                None => continue,
            };
            let meta_path = path.join("meta.toml");
            let text = match fs::read_to_string(&meta_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let meta: CacheMeta = match toml::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    // Corrupt entry — evict.
                    let _ = fs::remove_dir_all(&path);
                    continue;
                }
            };
            out.push((key, meta));
        }
        Ok(out)
    }

    /// Total disk usage of the cache, in bytes (sum of vbc + meta sizes
    /// per entry; intermediate directory overhead not counted).
    pub fn total_size(&self) -> CacheResult<u64> {
        Ok(self
            .list()?
            .iter()
            .map(|(_, m)| m.vbc_len + 256 /* meta budget */)
            .sum())
    }

    /// Evict least-recently-accessed entries until total cache size is
    /// at most `max_bytes`. Returns the count of entries evicted.
    pub fn gc_to_size(&self, max_bytes: u64) -> CacheResult<usize> {
        let mut entries = self.list()?;
        if entries.is_empty() {
            return Ok(0);
        }
        // Sort oldest-access first so we evict from the front.
        entries.sort_by_key(|(_, m)| m.last_accessed_at);
        let mut total: u64 = entries
            .iter()
            .map(|(_, m)| m.vbc_len + 256)
            .sum();
        let mut evicted = 0usize;
        for (key, meta) in entries {
            if total <= max_bytes {
                break;
            }
            self.evict(key)?;
            total = total.saturating_sub(meta.vbc_len + 256);
            evicted += 1;
        }
        Ok(evicted)
    }

    /// Remove every entry in the cache.
    pub fn clear(&self) -> CacheResult<usize> {
        let entries = self.list()?;
        let n = entries.len();
        for (key, _) in entries {
            self.evict(key)?;
        }
        Ok(n)
    }

    /// Cache root path — exposed for `verum cache` subcommand UX.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Atomic file write: write to `<path>.tmp.<pid>`, fsync, rename to
/// `<path>`. The rename is atomic on every filesystem we target (ext4,
/// APFS, NTFS).
fn atomic_write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        CacheMeta::now_secs()
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all().ok(); // best-effort; some filesystems no-op
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e
    })
}

fn atomic_write_string(path: &Path, contents: &str) -> io::Result<()> {
    atomic_write_bytes(path, contents.as_bytes())
}

/// Open a file for read with platform-appropriate buffering. Used by
/// callers that want streaming access; `lookup()` reads the whole VBC
/// into memory because the interpreter loads it whole anyway.
#[allow(dead_code)]
fn open_buffered(path: &Path) -> io::Result<impl Read> {
    use std::io::BufReader;
    Ok(BufReader::new(fs::File::open(path)?))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SUFFIX: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        let n = SUFFIX.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "verum_script_cache_{}_{}_{}_{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n,
        ))
    }

    // CacheKey hex round-trip ----------------------------------------------------------------

    #[test]
    fn cache_key_hex_round_trip() {
        let k = key_for(b"hello", "0.6.0", &["tier=0"]);
        let s = k.to_hex();
        assert_eq!(s.len(), 64, "hex length");
        let k2 = CacheKey::from_hex(&s).expect("round-trip");
        assert_eq!(k, k2);
    }

    #[test]
    fn cache_key_from_hex_rejects_garbage() {
        assert!(CacheKey::from_hex("").is_none());
        assert!(CacheKey::from_hex("abc").is_none()); // wrong length
        assert!(CacheKey::from_hex(&"z".repeat(64)).is_none()); // bad chars
    }

    #[test]
    fn key_for_is_deterministic() {
        let a = key_for(b"src", "0.6", &["a", "b"]);
        let b = key_for(b"src", "0.6", &["a", "b"]);
        assert_eq!(a, b);
    }

    #[test]
    fn key_for_distinguishes_compiler() {
        let a = key_for(b"src", "0.6.0", &["tier=0"]);
        let b = key_for(b"src", "0.7.0", &["tier=0"]);
        assert_ne!(a, b);
    }

    #[test]
    fn key_for_distinguishes_flags() {
        let a = key_for(b"src", "0.6", &["tier=0"]);
        let b = key_for(b"src", "0.6", &["tier=1"]);
        assert_ne!(a, b);
    }

    #[test]
    fn key_for_distinguishes_source() {
        let a = key_for(b"a", "0.6", &[]);
        let b = key_for(b"b", "0.6", &[]);
        assert_ne!(a, b);
    }

    #[test]
    fn key_for_no_length_extension_collision() {
        // ("ab", []) vs ("a", ["b"]) — without separators these would
        // hash to the same digest. With null separators they don't.
        let a = key_for(b"ab", "0.6", &[]);
        let b = key_for(b"a", "0.6", &["b"]);
        assert_ne!(a, b);
    }

    // ScriptCache CRUD -----------------------------------------------------------------------

    #[test]
    fn store_then_lookup_round_trip() {
        let root = temp_root("rt");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"fn main() {}", "0.6", &["tier=0"]);
        let bytecode = b"VBC\x00\x01\x02fake".as_ref();
        cache
            .store(key, bytecode, "/tmp/x.vr", 100, "0.6.0")
            .unwrap();
        let entry = cache.lookup(key).unwrap().expect("hit");
        assert_eq!(entry.vbc, bytecode);
        assert_eq!(entry.meta.source_path, "/tmp/x.vr");
        assert_eq!(entry.meta.source_len, 100);
        assert_eq!(entry.meta.compiler_version, "0.6.0");
        assert_eq!(entry.meta.vbc_len as usize, bytecode.len());
        assert_eq!(entry.meta.schema_version, META_SCHEMA_VERSION);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_miss_returns_none() {
        let root = temp_root("miss");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"never-stored", "0.6", &[]);
        assert!(cache.lookup(key).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn store_overwrite_is_safe() {
        let root = temp_root("overwrite");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"src", "0.6", &[]);
        cache.store(key, b"first", "p", 1, "0.6.0").unwrap();
        cache.store(key, b"second", "p", 1, "0.6.0").unwrap();
        let e = cache.lookup(key).unwrap().expect("hit");
        assert_eq!(e.vbc, b"second");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn evict_removes_entry() {
        let root = temp_root("evict");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"x", "0.6", &[]);
        cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        assert!(cache.lookup(key).unwrap().is_some());
        cache.evict(key).unwrap();
        assert!(cache.lookup(key).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn evict_missing_is_idempotent() {
        let root = temp_root("evict-miss");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"never", "0.6", &[]);
        cache.evict(key).unwrap();
        cache.evict(key).unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn list_skips_non_blake3_dirs_and_reaps_tempdirs() {
        let root = temp_root("list");
        let cache = ScriptCache::at(root.clone()).unwrap();
        // Stash a real entry, plus a junk dir, plus an orphan tempdir.
        let key = key_for(b"x", "0.6", &[]);
        cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        fs::create_dir_all(root.join("not-a-key")).unwrap();
        fs::create_dir_all(root.join("aaaa.tmp.99.123")).unwrap();
        let listed = cache.list().unwrap();
        assert_eq!(listed.len(), 1, "only the real entry");
        assert_eq!(listed[0].0, key);
        // Tempdir reaped.
        assert!(!root.join("aaaa.tmp.99.123").exists());
        // Junk dir untouched.
        assert!(root.join("not-a-key").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_evicts_corrupt_meta_and_surfaces_error() {
        let root = temp_root("corrupt");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"x", "0.6", &[]);
        cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        // Trash the meta.toml.
        let meta_path = cache.entry_dir(key).join("meta.toml");
        fs::write(&meta_path, "this is = not [valid] [[[[ TOML").unwrap();
        let res = cache.lookup(key);
        assert!(matches!(res, Err(CacheError::InvalidMeta { .. })));
        // Entry was evicted — subsequent lookup is a clean miss.
        assert!(cache.lookup(key).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn schema_version_skew_evicts_silently() {
        let root = temp_root("schema");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"x", "0.6", &[]);
        cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        // Manually rewrite meta with a bumped schema version.
        let meta_path = cache.entry_dir(key).join("meta.toml");
        let text = fs::read_to_string(&meta_path).unwrap();
        let bumped = text.replace(
            &format!("schema_version = {META_SCHEMA_VERSION}"),
            &format!("schema_version = {}", META_SCHEMA_VERSION + 99),
        );
        fs::write(&meta_path, bumped).unwrap();
        // Lookup quietly evicts and returns miss.
        assert!(cache.lookup(key).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gc_evicts_oldest_first_under_budget() {
        let root = temp_root("gc");
        let cache = ScriptCache::at(root.clone()).unwrap();
        // Three entries; we'll backdate access timestamps to force order.
        let k1 = key_for(b"1", "0.6", &[]);
        let k2 = key_for(b"2", "0.6", &[]);
        let k3 = key_for(b"3", "0.6", &[]);
        cache.store(k1, &vec![0xAA; 1024], "p", 1, "0.6.0").unwrap();
        cache.store(k2, &vec![0xBB; 1024], "p", 1, "0.6.0").unwrap();
        cache.store(k3, &vec![0xCC; 1024], "p", 1, "0.6.0").unwrap();

        // Backdate k1 to oldest, k2 to middle, k3 stays newest.
        for (k, ts) in [(k1, 1u64), (k2, 100u64), (k3, CacheMeta::now_secs())] {
            let meta_path = cache.entry_dir(k).join("meta.toml");
            let text = fs::read_to_string(&meta_path).unwrap();
            // Replace last_accessed_at line by structural rewrite — toml round-trip.
            let mut meta: CacheMeta = toml::from_str(&text).unwrap();
            meta.last_accessed_at = ts;
            fs::write(&meta_path, toml::to_string(&meta).unwrap()).unwrap();
        }

        // Budget = 2 entries × (1024 + 256) overhead = ~2.5 KB.
        let budget = 2 * (1024 + 256);
        let evicted = cache.gc_to_size(budget).unwrap();
        // k1 (oldest) must be gone; k2 may or may not depending on
        // exact rounding — k3 (newest) MUST survive.
        assert!(cache.lookup(k1).unwrap().is_none(), "oldest evicted");
        assert!(cache.lookup(k3).unwrap().is_some(), "newest survives");
        assert!(evicted >= 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_removes_all_entries() {
        let root = temp_root("clear");
        let cache = ScriptCache::at(root.clone()).unwrap();
        for i in 0u8..5 {
            let key = key_for(&[i], "0.6", &[]);
            cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        }
        assert_eq!(cache.list().unwrap().len(), 5);
        let removed = cache.clear().unwrap();
        assert_eq!(removed, 5);
        assert_eq!(cache.list().unwrap().len(), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_refreshes_last_accessed_at() {
        let root = temp_root("touch");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"x", "0.6", &[]);
        cache.store(key, b"v", "p", 1, "0.6.0").unwrap();
        // Manually backdate the last-access time.
        let meta_path = cache.entry_dir(key).join("meta.toml");
        let mut meta: CacheMeta =
            toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
        let backdated = 1u64;
        meta.last_accessed_at = backdated;
        fs::write(&meta_path, toml::to_string(&meta).unwrap()).unwrap();
        // Lookup must refresh.
        let _ = cache.lookup(key).unwrap().expect("hit");
        let after: CacheMeta =
            toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
        assert!(
            after.last_accessed_at > backdated,
            "lookup should refresh last_accessed_at: {} -> {}",
            backdated,
            after.last_accessed_at
        );
        let _ = fs::remove_dir_all(&root);
    }

    // Concurrency contract — last writer wins on identical key (content-addressed) ----------

    #[test]
    fn concurrent_store_same_key_yields_consistent_entry() {
        use std::thread;
        let root = temp_root("concurrent");
        let cache = ScriptCache::at(root.clone()).unwrap();
        let key = key_for(b"shared", "0.6", &[]);
        let bytecode = b"AAAA";
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = cache.clone();
                thread::spawn(move || {
                    c.store(key, bytecode, "p", 4, "0.6.0").unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // After all stores, exactly one entry exists with the expected bytes.
        let entry = cache.lookup(key).unwrap().expect("hit");
        assert_eq!(entry.vbc, bytecode);
        // No orphan tempdirs.
        let listing = cache.list().unwrap();
        assert_eq!(listing.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
