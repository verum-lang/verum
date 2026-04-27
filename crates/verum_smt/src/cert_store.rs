//! persistent certificate store.
//!
//! Bridges the gap between certificate **production** (the SMT
//! verification phase emits an [`SmtCertificate`] per discharged
//! obligation) and certificate **consumption** (the export
//! pipeline calls a [`ProofReplayBackend`] that needs to find the
//! cert for a given declaration).
//!
//! Architectural shape:
//!
//!   • [`CertificateStore`] — the trait every backing store
//!     implements. Save / load / list / remove operations are
//!     keyed by *declaration name*, mirroring the way the export
//!     pipeline resolves theorems.
//!   • [`FileSystemCertificateStore`] — the production backing,
//!     persists each cert as a single JSON file under
//!     `<root>/<sanitised-decl-name>.smt-cert.json`. The disk
//!     layout is intentionally one-cert-per-file so different
//!     theorems' certs can be regenerated independently without
//!     touching siblings (CI-friendly, git-friendly).
//!   • [`InMemoryCertificateStore`] — test-only backing for unit
//!     tests; identical contract.
//!
//! Per VVA semantic-honesty rule the public API surfaces use
//! [`verum_common::Maybe`] and [`verum_common::Text`] so the API
//! reads as a Verum-shaped store even though the implementation
//! is Rust-native (kernel-side TCB stays at the JSON envelope —
//! the kernel does not trust on-disk format; certificates are
//! re-replayed at consumption time).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use verum_common::{List, Maybe, Text};
use verum_kernel::SmtCertificate;

/// error surface for
/// certificate-store operations. Distinct from
/// [`verum_kernel::KernelError`] so callers can route I/O failures
/// without conflating them with kernel-typing failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertStoreError {
    /// I/O failure (disk not writable, permission denied, …).
    /// Carries a human-readable message; callers typically log and
    /// fall through to the admitted scaffold.
    Io(Text),
    /// JSON serialisation / deserialisation failure. Indicates a
    /// schema mismatch on disk; the store will refuse to load
    /// rather than silently produce a malformed cert.
    Codec(Text),
    /// Declaration name contains characters that cannot be safely
    /// mapped to a filename (path-separators, NUL, etc.). The
    /// store rejects the operation rather than producing an
    /// ambiguous file path.
    InvalidName(Text),
}

impl std::fmt::Display for CertStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "cert-store I/O: {}", msg),
            Self::Codec(msg) => write!(f, "cert-store codec: {}", msg),
            Self::InvalidName(name) => write!(
                f,
                "cert-store: declaration name `{}` cannot be mapped to a filesystem path",
                name
            ),
        }
    }
}

impl std::error::Error for CertStoreError {}

/// common contract every
/// certificate-store backing implements.
///
/// Keyed by declaration name (stable across runs); lookup
/// returns `Maybe::None` rather than an error so callers can
/// gracefully fall through to the admitted scaffold when no cert
/// is yet on-disk for a theorem.
pub trait CertificateStore: Send + Sync {
    /// Persist a certificate under the given declaration name.
    /// Overwrites any existing cert for the same name.
    fn save(
        &mut self,
        decl_name: &str,
        cert: &SmtCertificate,
    ) -> Result<(), CertStoreError>;

    /// Look up the certificate for a declaration. Returns
    /// `Maybe::None` when no cert is stored — callers fall
    /// through to the admitted scaffold path.
    fn load(&self, decl_name: &str) -> Maybe<SmtCertificate>;

    /// Enumerate every declaration name with a stored cert.
    /// Returned list is sorted (BTreeMap-style) so the order is
    /// deterministic for CI diffs.
    fn list(&self) -> List<Text>;

    /// Remove the cert for a declaration. Idempotent — removing
    /// a non-existent cert is not an error.
    fn remove(&mut self, decl_name: &str) -> Result<(), CertStoreError>;
}

// =============================================================================
// FileSystemCertificateStore
// =============================================================================

/// on-disk certificate store.
///
/// Layout: each cert lives at `<root>/<sanitised-name>.smt-cert.json`.
/// The sanitisation rule maps declaration names to a filesystem-safe
/// shape — only alphanumeric ASCII, underscore, dash, and dot are
/// preserved; everything else is replaced with `_`. Names that
/// sanitise to the empty string are rejected with
/// [`CertStoreError::InvalidName`].
///
/// The store creates `root` lazily on first `save`; readers gracefully
/// handle a missing root directory (returns `Maybe::None`).
pub struct FileSystemCertificateStore {
    root: PathBuf,
}

impl FileSystemCertificateStore {
    /// Construct a store rooted at the given directory. The
    /// directory is created lazily on first save (zero-cost when
    /// only reading).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Convention-driven location for a project: `<manifest_dir>/.verum/cache/certificates/`.
    /// Pre-creates the parent path-component skeleton when called.
    pub fn for_project(manifest_dir: &Path) -> Self {
        let root = manifest_dir
            .join(".verum")
            .join("cache")
            .join("certificates");
        Self::new(root)
    }

    /// Map a declaration name to its on-disk file path. Returns
    /// `Err(InvalidName)` for names that don't sanitise to a
    /// non-empty string.
    fn path_for(&self, decl_name: &str) -> Result<PathBuf, CertStoreError> {
        let sanitised = sanitise_name(decl_name);
        if sanitised.is_empty() {
            return Err(CertStoreError::InvalidName(Text::from(decl_name)));
        }
        Ok(self.root.join(format!("{}.smt-cert.json", sanitised)))
    }
}

impl CertificateStore for FileSystemCertificateStore {
    fn save(
        &mut self,
        decl_name: &str,
        cert: &SmtCertificate,
    ) -> Result<(), CertStoreError> {
        let path = self.path_for(decl_name)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CertStoreError::Io(Text::from(format!(
                    "creating {}: {}",
                    parent.display(),
                    e
                )))
            })?;
        }
        let json = serde_json::to_string_pretty(cert).map_err(|e| {
            CertStoreError::Codec(Text::from(format!(
                "serialising cert for `{}`: {}",
                decl_name, e
            )))
        })?;
        std::fs::write(&path, json.as_bytes()).map_err(|e| {
            CertStoreError::Io(Text::from(format!(
                "writing {}: {}",
                path.display(),
                e
            )))
        })
    }

    fn load(&self, decl_name: &str) -> Maybe<SmtCertificate> {
        let path = match self.path_for(decl_name) {
            Ok(p) => p,
            Err(_) => return Maybe::None,
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Maybe::None,
        };
        match serde_json::from_slice::<SmtCertificate>(&bytes) {
            Ok(cert) => Maybe::Some(cert),
            // Codec errors silently surface as None — callers
            // always have the admitted-scaffold fallback. The
            // disk file isn't lost; an external tool can inspect
            // it for the schema mismatch.
            Err(_) => Maybe::None,
        }
    }

    fn list(&self) -> List<Text> {
        let mut out = List::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return out,
        };
        // Sort via BTreeSet for deterministic order.
        let mut names: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let stem = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s,
                    None => continue,
                };
                if let Some(name) = stem.strip_suffix(".smt-cert") {
                    names.insert(name.to_string());
                }
            }
        }
        for n in names {
            out.push(Text::from(n));
        }
        out
    }

    fn remove(&mut self, decl_name: &str) -> Result<(), CertStoreError> {
        let path = self.path_for(decl_name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            // Idempotent: removing a non-existent file is a no-op.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CertStoreError::Io(Text::from(format!(
                "removing {}: {}",
                path.display(),
                e
            )))),
        }
    }
}

/// Map a declaration name to a filesystem-safe form. Preserves
/// alphanumeric ASCII, underscore, dash, dot, and `+`; everything
/// else (including non-ASCII) is replaced with `_`. Avoids path
/// traversal (`/`, `\`, `..`) and shell-special bytes by
/// construction.
fn sanitise_name(decl_name: &str) -> String {
    let mut out = String::with_capacity(decl_name.len());
    for ch in decl_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '+' {
            out.push(ch);
        } else if ch == '.' {
            // Leading-dot would shadow filesystem hidden-file
            // conventions; we treat dot as a safe separator
            // BUT prepend `_` if it would otherwise be the first
            // character.
            if out.is_empty() {
                out.push('_');
            }
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

// =============================================================================
// InMemoryCertificateStore — test-only
// =============================================================================

/// in-memory backing for tests.
/// Honours the same contract as [`FileSystemCertificateStore`]
/// without touching the disk; ideal for fast unit tests.
#[derive(Default)]
pub struct InMemoryCertificateStore {
    entries: BTreeMap<String, SmtCertificate>,
}

impl InMemoryCertificateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CertificateStore for InMemoryCertificateStore {
    fn save(
        &mut self,
        decl_name: &str,
        cert: &SmtCertificate,
    ) -> Result<(), CertStoreError> {
        if sanitise_name(decl_name).is_empty() {
            return Err(CertStoreError::InvalidName(Text::from(decl_name)));
        }
        self.entries.insert(decl_name.to_string(), cert.clone());
        Ok(())
    }

    fn load(&self, decl_name: &str) -> Maybe<SmtCertificate> {
        match self.entries.get(decl_name) {
            Some(c) => Maybe::Some(c.clone()),
            None => Maybe::None,
        }
    }

    fn list(&self) -> List<Text> {
        let mut out = List::new();
        for k in self.entries.keys() {
            out.push(Text::from(k.clone()));
        }
        out
    }

    fn remove(&mut self, decl_name: &str) -> Result<(), CertStoreError> {
        self.entries.remove(decl_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::List as VList;

    fn dummy_cert() -> SmtCertificate {
        SmtCertificate::new(
            Text::from("z3"),
            Text::from("4.12.0"),
            VList::new(),
            Text::from("blake3:abc"),
        )
    }

    #[test]
    fn sanitise_name_preserves_safe_chars() {
        assert_eq!(sanitise_name("plus_comm"), "plus_comm");
        assert_eq!(sanitise_name("Foo.Bar-baz_42"), "Foo.Bar-baz_42");
        assert_eq!(sanitise_name("alpha+beta"), "alpha+beta");
    }

    #[test]
    fn sanitise_name_replaces_unsafe_chars() {
        assert_eq!(sanitise_name("a/b"), "a_b");
        assert_eq!(sanitise_name("foo bar"), "foo_bar");
        assert_eq!(sanitise_name("a:b:c"), "a_b_c");
        // Dots are preserved (Verum module references like `foo.bar.Baz`
        // are common); leading dot gets `_` prefix to avoid hidden-file
        // shadowing. `/` becomes `_`. So `../etc` → `_.._etc`. The
        // result is a single filename — no path traversal possible
        // (the join target is `root/<sanitised>.smt-cert.json`).
        assert_eq!(sanitise_name("../etc"), "_.._etc");
    }

    #[test]
    fn sanitise_name_handles_leading_dot() {
        // Leading dot is prefixed with `_` to avoid hidden-file
        // shadowing.
        assert_eq!(sanitise_name(".hidden"), "_.hidden");
    }

    #[test]
    fn in_memory_store_round_trips_save_load() {
        let mut store = InMemoryCertificateStore::new();
        let cert = dummy_cert();
        store.save("plus_comm", &cert).unwrap();
        let loaded = store.load("plus_comm");
        match loaded {
            Maybe::Some(c) => assert_eq!(c.backend.as_str(), "z3"),
            Maybe::None => panic!("loaded must be Some"),
        }
    }

    #[test]
    fn in_memory_store_load_missing_returns_none() {
        let store = InMemoryCertificateStore::new();
        assert!(matches!(store.load("nope"), Maybe::None));
    }

    #[test]
    fn in_memory_store_list_returns_sorted_names() {
        let mut store = InMemoryCertificateStore::new();
        store.save("zeta", &dummy_cert()).unwrap();
        store.save("alpha", &dummy_cert()).unwrap();
        store.save("mu", &dummy_cert()).unwrap();
        let listed = store.list();
        let names: Vec<&str> = listed.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn in_memory_store_remove_is_idempotent() {
        let mut store = InMemoryCertificateStore::new();
        store.save("a", &dummy_cert()).unwrap();
        assert!(store.remove("a").is_ok());
        assert!(store.remove("a").is_ok()); // second remove also OK
        assert!(matches!(store.load("a"), Maybe::None));
    }

    #[test]
    fn in_memory_store_rejects_empty_name() {
        let mut store = InMemoryCertificateStore::new();
        let result = store.save("", &dummy_cert());
        assert!(matches!(result, Err(CertStoreError::InvalidName(_))));
    }

    #[test]
    fn fs_store_round_trips_via_disk() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        let cert = dummy_cert();
        store.save("yoneda", &cert).unwrap();
        // Reload via fresh store instance — same root dir.
        let store2 = FileSystemCertificateStore::new(temp.path().to_path_buf());
        let loaded = store2.load("yoneda");
        match loaded {
            Maybe::Some(c) => {
                assert_eq!(c.backend.as_str(), "z3");
                assert_eq!(c.obligation_hash.as_str(), "blake3:abc");
            }
            Maybe::None => panic!("disk round-trip failed"),
        }
    }

    #[test]
    fn fs_store_load_missing_returns_none() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        assert!(matches!(store.load("nope"), Maybe::None));
    }

    #[test]
    fn fs_store_load_from_nonexistent_root_returns_none() {
        let store = FileSystemCertificateStore::new(
            PathBuf::from("/nonexistent/path/to/nowhere"),
        );
        assert!(matches!(store.load("anything"), Maybe::None));
    }

    #[test]
    fn fs_store_list_returns_sorted_decl_names() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        store.save("zeta", &dummy_cert()).unwrap();
        store.save("alpha", &dummy_cert()).unwrap();
        store.save("mu", &dummy_cert()).unwrap();
        let listed = store.list();
        let names: Vec<&str> = listed.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn fs_store_remove_deletes_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        store.save("a", &dummy_cert()).unwrap();
        assert!(matches!(store.load("a"), Maybe::Some(_)));
        store.remove("a").unwrap();
        assert!(matches!(store.load("a"), Maybe::None));
    }

    #[test]
    fn fs_store_remove_missing_is_idempotent() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        // Removing nothing should not error.
        assert!(store.remove("nope").is_ok());
    }

    #[test]
    fn fs_store_path_for_uses_sanitised_name() {
        let store = FileSystemCertificateStore::new(PathBuf::from("/tmp/store"));
        let p = store.path_for("foo/bar").unwrap();
        assert!(p.to_string_lossy().contains("foo_bar.smt-cert.json"));
    }

    #[test]
    fn fs_store_path_for_rejects_empty_name() {
        let store = FileSystemCertificateStore::new(PathBuf::from("/tmp/store"));
        let result = store.path_for("");
        assert!(matches!(result, Err(CertStoreError::InvalidName(_))));
    }

    #[test]
    fn for_project_uses_dot_verum_cache_certificates() {
        let store = FileSystemCertificateStore::for_project(Path::new("/proj"));
        let p = store.path_for("foo").unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains(".verum"));
        assert!(s.contains("cache"));
        assert!(s.contains("certificates"));
    }

    #[test]
    fn fs_store_corrupt_json_returns_none_on_load() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FileSystemCertificateStore::new(temp.path().to_path_buf());
        let path = store.path_for("corrupt").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not valid json {{").unwrap();
        // Codec error surfaces as None; caller falls through to admitted.
        assert!(matches!(store.load("corrupt"), Maybe::None));
    }

    #[test]
    fn cert_store_error_display_messages_distinct() {
        let e1 = CertStoreError::Io(Text::from("disk full"));
        let e2 = CertStoreError::Codec(Text::from("schema mismatch"));
        let e3 = CertStoreError::InvalidName(Text::from(""));
        assert_ne!(format!("{}", e1), format!("{}", e2));
        assert_ne!(format!("{}", e2), format!("{}", e3));
    }
}
