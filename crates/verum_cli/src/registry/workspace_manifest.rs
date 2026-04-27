//! Verum workspace manifest (`verum.work`) — P8.
//!
//! A workspace bundles multiple cogs under one root for shared
//! dependency declarations, unified build, and atomic publish. The
//! manifest is a TOML file at the workspace root; the conventional
//! filename is **`verum.work`** (lowercase, matching `verum.toml` /
//! `verum.lock`).
//!
//! # Example
//!
//! ```toml
//! [workspace]
//! members         = ["crates/api", "crates/runtime", "examples/*"]
//! default-members = ["crates/api"]
//! exclude         = ["target", "vendored/*"]
//!
//! [workspace.dependencies]
//! json = "1.4"
//! http = { version = "0.2", features = ["std"] }
//!
//! [workspace.metadata]
//! description = "Multi-cog Verum workspace"
//! ```
//!
//! Members support glob patterns (`crates/*`); they are expanded
//! relative to the workspace root at load time. `default-members` must
//! be a subset of `members` (after expansion) — that's enforced at
//! parse time so a typo doesn't silently disappear.
//!
//! `[workspace.dependencies]` declares deps that members can inherit
//! via `foo.workspace = true` in their own `[dependencies]` table —
//! the canonical Cargo-style approach for keeping versions consistent
//! across a workspace without copying them everywhere.
//!
//! # Discovery
//!
//! [`WorkspaceManifest::discover`] walks up from a starting directory
//! looking for `verum.work`. The first hit wins; `None` means the cwd
//! is not inside a workspace. Tooling (`verum build`, `verum test`,
//! `verum publish`) calls this on entry to decide whether to operate
//! in single-cog or workspace mode.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Conventional filename for the workspace manifest. Lowercase to
/// align with `verum.toml`, `verum.lock`, `verum.work`.
pub const MANIFEST_FILENAME: &str = "verum.work";

/// One workspace dependency declaration. Mirrors Cargo's short / long
/// forms — `version = "1.0"` or `{ version = "1.0", features = [...] }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Bare-version form: `json = "1.4"`.
    Version(String),
    /// Table form with optional features / source overrides.
    Table(DependencyTable),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyTable {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub git: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

/// Parsed `verum.work` manifest. The on-disk format is a top-level
/// `[workspace]` table; the public Rust struct flattens that for
/// ergonomic access.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceManifest {
    /// Member entries as declared (before glob expansion). Preserved
    /// for round-trip serialisation and diagnostics.
    pub members: Vec<String>,
    /// Default-member entries. Must be a (textually) declared subset
    /// of `members` after parse. Empty = inherit `members`.
    pub default_members: Vec<String>,
    /// Glob patterns to exclude from auto-discovery. Applied during
    /// member-glob expansion.
    pub exclude: Vec<String>,
    /// Shared dependency table. Members can opt in via
    /// `[dependencies] foo.workspace = true`.
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// Free-form `[workspace.metadata]` — ignored by the resolver
    /// but preserved for tooling that wants to extend the manifest.
    pub metadata: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OnDisk {
    workspace: OnDiskWorkspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OnDiskWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default, rename = "default-members")]
    default_members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default)]
    metadata: Option<toml::Value>,
}

/// Parse / discovery / validation failures.
#[derive(Debug)]
pub enum ManifestError {
    /// I/O error reading or stat'ing a path.
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// TOML parse failure.
    ParseError {
        path: PathBuf,
        reason: String,
    },
    /// `members` list was empty.
    EmptyMembers { path: PathBuf },
    /// `default-members` mentions a name not present in `members`.
    DefaultNotInMembers {
        path: PathBuf,
        missing: String,
    },
    /// A member glob pattern was malformed.
    InvalidGlob {
        path: PathBuf,
        pattern: String,
        reason: String,
    },
    /// A member entry was an absolute path (workspace members must be
    /// relative to the workspace root for portability).
    AbsoluteMember {
        path: PathBuf,
        member: String,
    },
    /// A member entry contained `..` (escapes the workspace root).
    MemberEscapesRoot {
        path: PathBuf,
        member: String,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { op, path, source } => {
                write!(f, "workspace manifest {op} on {}: {source}", path.display())
            }
            Self::ParseError { path, reason } => {
                write!(f, "workspace manifest {} is malformed: {reason}", path.display())
            }
            Self::EmptyMembers { path } => write!(
                f,
                "workspace manifest {} has empty `members` — at least one is required",
                path.display()
            ),
            Self::DefaultNotInMembers { path, missing } => write!(
                f,
                "workspace manifest {}: `default-members` entry {missing:?} is not declared in `members`",
                path.display()
            ),
            Self::InvalidGlob { path, pattern, reason } => write!(
                f,
                "workspace manifest {}: glob pattern {pattern:?} is invalid: {reason}",
                path.display()
            ),
            Self::AbsoluteMember { path, member } => write!(
                f,
                "workspace manifest {}: member {member:?} must be a relative path",
                path.display()
            ),
            Self::MemberEscapesRoot { path, member } => write!(
                f,
                "workspace manifest {}: member {member:?} escapes the workspace root via `..`",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type ManifestResult<T> = Result<T, ManifestError>;

impl WorkspaceManifest {
    /// Read and parse `verum.work` at `path`. Validates the result
    /// (members non-empty, default-members ⊆ members, no absolute /
    /// `..` paths, glob patterns well-formed) before returning.
    pub fn from_file(path: &Path) -> ManifestResult<Self> {
        let text = fs::read_to_string(path).map_err(|source| ManifestError::Io {
            op: "read",
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// Parse from in-memory TOML, attributing errors to `origin` (used
    /// in error messages). Useful for tests + `verum init` previews.
    pub fn parse(text: &str, origin: &Path) -> ManifestResult<Self> {
        let raw: OnDisk = toml::from_str(text).map_err(|e| ManifestError::ParseError {
            path: origin.to_path_buf(),
            reason: e.to_string(),
        })?;

        let manifest = Self {
            members: raw.workspace.members,
            default_members: raw.workspace.default_members,
            exclude: raw.workspace.exclude,
            dependencies: raw.workspace.dependencies,
            metadata: raw.workspace.metadata,
        };
        manifest.validate(origin)?;
        Ok(manifest)
    }

    /// Walk up the directory tree from `start` looking for a
    /// `verum.work` file. Returns `(workspace_root, manifest)` on the
    /// first hit, or `None` if the walk reaches the filesystem root
    /// without finding one.
    pub fn discover(start: &Path) -> ManifestResult<Option<(PathBuf, Self)>> {
        let mut cur = match fs::canonicalize(start) {
            Ok(c) => c,
            Err(_) => start.to_path_buf(),
        };
        loop {
            let candidate = cur.join(MANIFEST_FILENAME);
            if candidate.is_file() {
                let manifest = Self::from_file(&candidate)?;
                return Ok(Some((cur, manifest)));
            }
            match cur.parent() {
                Some(parent) if parent != cur => cur = parent.to_path_buf(),
                _ => return Ok(None),
            }
        }
    }

    /// Expand member globs against `workspace_root`. Returns the
    /// resolved absolute paths in declaration order. Patterns that
    /// match no files are an error (matches Cargo's behaviour).
    pub fn member_dirs(&self, workspace_root: &Path) -> ManifestResult<Vec<PathBuf>> {
        let mut out = Vec::with_capacity(self.members.len());
        let exclude_globs: Vec<glob::Pattern> = self
            .exclude
            .iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();

        for entry in &self.members {
            // Bare entry (no glob meta-chars): just join.
            if !entry.chars().any(|c| matches!(c, '*' | '?' | '[' | ']')) {
                let path = workspace_root.join(entry);
                if !is_excluded(&path, workspace_root, &exclude_globs) {
                    out.push(path);
                }
                continue;
            }
            // Glob expansion. We resolve relative to workspace_root.
            let pattern = workspace_root
                .join(entry)
                .to_string_lossy()
                .into_owned();
            let walk = glob::glob(&pattern).map_err(|e| ManifestError::InvalidGlob {
                path: workspace_root.join(MANIFEST_FILENAME),
                pattern: entry.clone(),
                reason: e.to_string(),
            })?;
            let mut matched = false;
            for hit in walk {
                let p = match hit {
                    Ok(p) => p,
                    Err(e) => {
                        return Err(ManifestError::InvalidGlob {
                            path: workspace_root.join(MANIFEST_FILENAME),
                            pattern: entry.clone(),
                            reason: e.to_string(),
                        });
                    }
                };
                if !p.is_dir() {
                    continue;
                }
                if is_excluded(&p, workspace_root, &exclude_globs) {
                    continue;
                }
                out.push(p);
                matched = true;
            }
            if !matched {
                return Err(ManifestError::InvalidGlob {
                    path: workspace_root.join(MANIFEST_FILENAME),
                    pattern: entry.clone(),
                    reason: "no directories match the pattern".to_string(),
                });
            }
        }
        // Stable order, dedupe.
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Default member set: explicit if non-empty, else fall back to
    /// every member. Returns paths relative to `workspace_root`.
    pub fn default_member_dirs(&self, workspace_root: &Path) -> ManifestResult<Vec<PathBuf>> {
        if self.default_members.is_empty() {
            return self.member_dirs(workspace_root);
        }
        let mut out = Vec::with_capacity(self.default_members.len());
        for entry in &self.default_members {
            out.push(workspace_root.join(entry));
        }
        Ok(out)
    }

    /// `true` iff `path` lies under one of the resolved member dirs.
    /// Caller-supplied paths are canonicalised best-effort; non-
    /// existent paths are matched lexically.
    pub fn contains(&self, workspace_root: &Path, path: &Path) -> ManifestResult<bool> {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for member in self.member_dirs(workspace_root)? {
            let canon_member = fs::canonicalize(&member).unwrap_or(member);
            if canon.starts_with(&canon_member) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Validate parse-time invariants. Caller passes `origin` so error
    /// messages can pinpoint the offending file.
    fn validate(&self, origin: &Path) -> ManifestResult<()> {
        if self.members.is_empty() {
            return Err(ManifestError::EmptyMembers {
                path: origin.to_path_buf(),
            });
        }
        for member in &self.members {
            if Path::new(member).is_absolute() {
                return Err(ManifestError::AbsoluteMember {
                    path: origin.to_path_buf(),
                    member: member.clone(),
                });
            }
            if member.contains("..") {
                return Err(ManifestError::MemberEscapesRoot {
                    path: origin.to_path_buf(),
                    member: member.clone(),
                });
            }
            // Validate glob syntax now (cheap; surfaces typos at parse).
            if member.chars().any(|c| matches!(c, '*' | '?' | '[' | ']')) {
                if let Err(e) = glob::Pattern::new(member) {
                    return Err(ManifestError::InvalidGlob {
                        path: origin.to_path_buf(),
                        pattern: member.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }
        for default in &self.default_members {
            if !self.members.contains(default) {
                return Err(ManifestError::DefaultNotInMembers {
                    path: origin.to_path_buf(),
                    missing: default.clone(),
                });
            }
        }
        for exclude in &self.exclude {
            if let Err(e) = glob::Pattern::new(exclude) {
                return Err(ManifestError::InvalidGlob {
                    path: origin.to_path_buf(),
                    pattern: exclude.clone(),
                    reason: e.to_string(),
                });
            }
        }
        Ok(())
    }
}

fn is_excluded(path: &Path, root: &Path, excludes: &[glob::Pattern]) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    excludes
        .iter()
        .any(|p| p.matches(&rel_str) || p.matches_path(rel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(tmp: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = tmp.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, body).unwrap();
        path
    }

    fn write_manifest(tmp: &TempDir, body: &str) -> PathBuf {
        write(tmp, MANIFEST_FILENAME, body)
    }

    fn mkdir(tmp: &TempDir, dir: &str) -> PathBuf {
        let p = tmp.path().join(dir);
        fs::create_dir_all(&p).unwrap();
        p
    }

    // ── parse ────────────────────────────────────────────────────────

    #[test]
    fn parses_minimal_manifest() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"a\"]\n");
        let m = WorkspaceManifest::from_file(&path).unwrap();
        assert_eq!(m.members, vec!["a".to_string()]);
        assert!(m.default_members.is_empty());
        assert!(m.exclude.is_empty());
        assert!(m.dependencies.is_empty());
    }

    #[test]
    fn parses_full_manifest() {
        let tmp = TempDir::new().unwrap();
        let body = r#"
[workspace]
members         = ["crates/a", "crates/b"]
default-members = ["crates/a"]
exclude         = ["target"]

[workspace.dependencies]
json = "1.4"
http = { version = "0.2", features = ["std"] }

[workspace.metadata]
description = "test"
"#;
        let path = write_manifest(&tmp, body);
        let m = WorkspaceManifest::from_file(&path).unwrap();
        assert_eq!(m.members, vec!["crates/a", "crates/b"]);
        assert_eq!(m.default_members, vec!["crates/a"]);
        assert_eq!(m.exclude, vec!["target"]);
        assert_eq!(m.dependencies.len(), 2);
        assert!(m.metadata.is_some());

        match &m.dependencies["json"] {
            DependencySpec::Version(v) => assert_eq!(v, "1.4"),
            _ => panic!("expected Version"),
        }
        match &m.dependencies["http"] {
            DependencySpec::Table(t) => {
                assert_eq!(t.version.as_deref(), Some("0.2"));
                assert_eq!(t.features, vec!["std"]);
            }
            _ => panic!("expected Table"),
        }
    }

    // ── validate ─────────────────────────────────────────────────────

    #[test]
    fn rejects_empty_members() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = []\n");
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ManifestError::EmptyMembers { .. }));
    }

    #[test]
    fn rejects_absolute_member() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"/etc/x\"]\n");
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ManifestError::AbsoluteMember { .. }));
    }

    #[test]
    fn rejects_dotdot_member() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"../escape\"]\n");
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ManifestError::MemberEscapesRoot { .. }));
    }

    #[test]
    fn rejects_default_member_not_in_members() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(
            &tmp,
            "[workspace]\nmembers = [\"a\"]\ndefault-members = [\"b\"]\n",
        );
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        match err {
            ManifestError::DefaultNotInMembers { missing, .. } => assert_eq!(missing, "b"),
            other => panic!("expected DefaultNotInMembers, got {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_glob_in_member() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"a/[bad\"]\n");
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidGlob { .. }));
    }

    #[test]
    fn rejects_garbage_toml() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, ":::not-toml:::");
        let err = WorkspaceManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ManifestError::ParseError { .. }));
    }

    #[test]
    fn from_file_errors_on_missing_file() {
        let tmp = TempDir::new().unwrap();
        let err = WorkspaceManifest::from_file(&tmp.path().join(MANIFEST_FILENAME)).unwrap_err();
        assert!(matches!(err, ManifestError::Io { .. }));
    }

    // ── member_dirs / globs ──────────────────────────────────────────

    #[test]
    fn member_dirs_resolves_literal_entries() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp, "crates/a");
        mkdir(&tmp, "crates/b");
        let path = write_manifest(
            &tmp,
            "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
        );
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let dirs = m.member_dirs(tmp.path()).unwrap();
        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().any(|p| p.ends_with("crates/a")));
        assert!(dirs.iter().any(|p| p.ends_with("crates/b")));
    }

    #[test]
    fn member_dirs_expands_globs() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp, "crates/a");
        mkdir(&tmp, "crates/b");
        mkdir(&tmp, "crates/c");
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"crates/*\"]\n");
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let dirs = m.member_dirs(tmp.path()).unwrap();
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn member_dirs_glob_with_no_match_errors() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"crates/*\"]\n");
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let err = m.member_dirs(tmp.path()).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidGlob { reason, .. } if reason.contains("no directories")));
    }

    #[test]
    fn member_dirs_excludes_filter_results() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp, "crates/a");
        mkdir(&tmp, "crates/b_skip");
        let path = write_manifest(
            &tmp,
            "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/*_skip\"]\n",
        );
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let dirs = m.member_dirs(tmp.path()).unwrap();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("crates/a"));
    }

    #[test]
    fn default_member_dirs_falls_back_to_members() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp, "a");
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"a\"]\n");
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let defaults = m.default_member_dirs(tmp.path()).unwrap();
        assert_eq!(defaults.len(), 1);
    }

    #[test]
    fn default_member_dirs_uses_explicit_when_present() {
        let tmp = TempDir::new().unwrap();
        let path = write_manifest(
            &tmp,
            "[workspace]\nmembers = [\"a\", \"b\"]\ndefault-members = [\"a\"]\n",
        );
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let defaults = m.default_member_dirs(tmp.path()).unwrap();
        assert_eq!(defaults.len(), 1);
        assert!(defaults[0].ends_with("a"));
    }

    // ── discover ─────────────────────────────────────────────────────

    #[test]
    fn discover_walks_up_to_find_manifest() {
        let tmp = TempDir::new().unwrap();
        let nested = mkdir(&tmp, "deep/inner/dir");
        mkdir(&tmp, "a");
        write_manifest(&tmp, "[workspace]\nmembers = [\"a\"]\n");
        let (root, _) = WorkspaceManifest::discover(&nested).unwrap().unwrap();
        let canon_tmp = fs::canonicalize(tmp.path()).unwrap();
        assert_eq!(fs::canonicalize(&root).unwrap(), canon_tmp);
    }

    #[test]
    fn discover_returns_none_when_no_manifest_above() {
        let tmp = TempDir::new().unwrap();
        let nested = mkdir(&tmp, "no/manifest/here");
        let result = WorkspaceManifest::discover(&nested).unwrap();
        // Could be Some only if a real verum.work exists somewhere
        // up to the filesystem root; the test tmpdir is fine to be
        // None unless the user has a verum.work in /tmp ancestry.
        // Accept either Some(at-far-ancestor) or None.
        if let Some((root, _)) = result {
            assert!(!root.starts_with(tmp.path()));
        }
    }

    // ── contains ─────────────────────────────────────────────────────

    #[test]
    fn contains_reports_membership() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp, "crates/a");
        mkdir(&tmp, "outside");
        let path = write_manifest(&tmp, "[workspace]\nmembers = [\"crates/a\"]\n");
        let m = WorkspaceManifest::from_file(&path).unwrap();
        let inside = tmp.path().join("crates/a/src/lib.vr");
        let outside = tmp.path().join("outside/lib.vr");
        assert!(m.contains(tmp.path(), &inside).unwrap());
        assert!(!m.contains(tmp.path(), &outside).unwrap());
    }

    // ── canonicalisation: filename casing ────────────────────────────

    #[test]
    fn canonical_manifest_filename_is_lowercase() {
        // Architectural invariant: matches verum.toml / verum.lock.
        assert_eq!(MANIFEST_FILENAME, "verum.work");
    }
}
