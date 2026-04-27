//! Verum lockfile v3 (P4.2).
//!
//! Successor to the legacy v1 [`super::lockfile::Lockfile`] (sha-256, no
//! load-time verification). Three concrete improvements:
//!
//! 1. **Algorithm-tagged integrity strings.** Every entry's hash is
//!    encoded as `"blake3:<64-hex>"` rather than a bare hex string.
//!    A future migration to a stronger algorithm becomes a parser-side
//!    discriminant rather than a silent re-interpretation.
//!
//! 2. **Lockfile self-integrity.** A blake3 digest over the
//!    canonicalised lockfile body (entries + workspace + manifest hash,
//!    in fixed sort order) is embedded at the top. Tampering with any
//!    entry's `integrity` field — the most attack-relevant part — flips
//!    the self-digest, surfaced by [`LockfileV3::verify_self_integrity`].
//!
//! 3. **Verify-on-every-load.** [`LockfileV3::from_file`] always
//!    re-runs self-integrity. Callers who hold a [`ContentStore`] can
//!    additionally call [`LockfileV3::verify_against_store`] to
//!    cross-check that every locked package's blob is present and
//!    integrity-clean — the same guarantee P5.1 provides per-blob, but
//!    extended to the whole lock.
//!
//! # Layout
//!
//! ```toml
//! version             = 3
//! root                = "myapp"
//! self_integrity      = "blake3:9bdc..."
//! manifest_integrity  = "blake3:c1aa..."   # optional: hash of verum.toml
//! created_at          = 1714161000
//! updated_at          = 1714161005
//! cli_version         = "0.6.0"
//!
//! [[package]]
//! name      = "json"
//! version   = "1.4.0"
//! source    = "registry+https://cogs.verum-lang.org"
//! integrity = "blake3:7bde..."
//! dependencies = [["http", "0.2.1"], ["text", "1.0.0"]]
//! features  = ["std"]
//! ```
//!
//! Sort order is canonical: packages by `(name, version)`, dependency
//! pairs and features are also sorted, so byte-equal inputs produce
//! byte-equal lockfiles independent of resolver traversal order.

use crate::registry::content_store::{ContentStore, Digest, StoreError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Lockfile schema version. Bumped on incompatible layout changes —
/// `from_file` rejects mismatched versions rather than re-interpreting.
pub const SCHEMA_VERSION: u32 = 3;

/// Algorithm tag for a typed integrity string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashAlgorithm {
    Blake3,
    /// Reserved for forward compatibility — supports parsing a
    /// `"sha256:..."` integrity tag without acting on it. Cooperative
    /// readers can still surface the algorithm to the user; v3 itself
    /// always *writes* `Blake3`.
    Sha256,
}

impl HashAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3",
            Self::Sha256 => "sha256",
        }
    }
}

/// Algorithm-tagged integrity string. Round-trips through `parse` /
/// `to_string`. Equality compares both algorithm and hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Integrity {
    pub algorithm: HashAlgorithm,
    pub hex: String,
}

impl Integrity {
    /// Compute the blake3 integrity of a byte slice.
    pub fn blake3_of(bytes: &[u8]) -> Self {
        Self {
            algorithm: HashAlgorithm::Blake3,
            hex: blake3::hash(bytes).to_hex().to_string(),
        }
    }

    /// Parse `"<algorithm>:<hex>"` form. Returns `None` on shape error
    /// or on hex strings that don't match the declared algorithm's
    /// expected length (blake3 = 64 chars, sha256 = 64 chars).
    pub fn parse(s: &str) -> Option<Self> {
        let (algo, hex) = s.split_once(':')?;
        let algorithm = match algo {
            "blake3" => HashAlgorithm::Blake3,
            "sha256" => HashAlgorithm::Sha256,
            _ => return None,
        };
        let expected_len = match algorithm {
            HashAlgorithm::Blake3 => 64,
            HashAlgorithm::Sha256 => 64,
        };
        if hex.len() != expected_len {
            return None;
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self {
            algorithm,
            hex: hex.to_ascii_lowercase(),
        })
    }

    /// Render as `"<algorithm>:<hex>"`.
    pub fn to_wire(&self) -> String {
        format!("{}:{}", self.algorithm.as_str(), self.hex)
    }

    /// Promote a v3-blake3 integrity to a [`Digest`] for direct
    /// comparison against the content-addressed store. Returns `None`
    /// for non-blake3 algorithms or malformed hex.
    pub fn to_blake3_digest(&self) -> Option<Digest> {
        if self.algorithm != HashAlgorithm::Blake3 {
            return None;
        }
        Digest::from_hex(&self.hex)
    }
}

impl Serialize for Integrity {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_wire())
    }
}

impl<'de> Deserialize<'de> for Integrity {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "expected `<algorithm>:<hex>` integrity (got {s:?})"
            ))
        })
    }
}

/// One locked package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    /// Source descriptor — same shape as
    /// [`crate::registry::content_store::Meta::source`].
    pub source: String,
    /// Typed integrity over the resolved blob bytes.
    pub integrity: Integrity,
    /// Direct dependencies as `[name, version]` pairs. Sorted by
    /// `(name, version)` so byte-identical lockfiles result from
    /// equivalent resolution traversals.
    #[serde(default)]
    pub dependencies: Vec<[String; 2]>,
    /// Enabled feature flags. Sorted.
    #[serde(default)]
    pub features: Vec<String>,
    /// Optional dependency: the lockfile records it but builds may
    /// skip materialising it.
    #[serde(default)]
    pub optional: bool,
}

/// V3 lockfile. Single source of truth for a workspace's resolved
/// dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockfileV3 {
    pub version: u32,
    pub root: String,
    /// Self-integrity digest computed via [`compute_self_integrity`].
    /// On read, [`from_file`] verifies; on write, [`to_file`]
    /// recomputes.
    pub self_integrity: Integrity,
    /// Optional integrity over the producing `verum.toml`. Lets a
    /// caller cheaply detect manifest drift without diffing the lock.
    #[serde(default)]
    pub manifest_integrity: Option<Integrity>,
    pub created_at: i64,
    pub updated_at: i64,
    pub cli_version: String,
    /// Locked packages, sorted by `(name, version)`.
    #[serde(default, rename = "package")]
    pub packages: Vec<LockedPackage>,
}

/// Failure modes for v3 lockfile operations.
#[derive(Debug)]
pub enum LockError {
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// `version` field didn't match [`SCHEMA_VERSION`].
    SchemaSkew {
        found: u32,
        expected: u32,
    },
    /// TOML parse failure on a syntactically broken lockfile.
    ParseError {
        path: PathBuf,
        reason: String,
    },
    /// `self_integrity` didn't match the recomputed digest. Lockfile
    /// has been tampered with — caller must reject.
    SelfIntegrityFailure {
        recorded: Integrity,
        computed: Integrity,
    },
    /// Cross-check against the content store revealed missing or
    /// integrity-broken packages. Both lists are non-empty in the
    /// reported cases.
    StoreVerificationFailed {
        missing: Vec<String>,
        broken: Vec<(String, StoreError)>,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { op, path, source } => {
                write!(f, "lockfile {op} on {}: {source}", path.display())
            }
            Self::SchemaSkew { found, expected } => write!(
                f,
                "lockfile schema v{found} (expected v{expected}); regenerate with `verum update`"
            ),
            Self::ParseError { path, reason } => {
                write!(f, "lockfile {} is malformed: {reason}", path.display())
            }
            Self::SelfIntegrityFailure { recorded, computed } => write!(
                f,
                "lockfile self-integrity failure — recorded {}, recomputed {}",
                recorded.to_wire(),
                computed.to_wire()
            ),
            Self::StoreVerificationFailed { missing, broken } => write!(
                f,
                "lockfile vs store: {} missing, {} integrity-broken",
                missing.len(),
                broken.len()
            ),
        }
    }
}

impl std::error::Error for LockError {}

pub type LockResult<T> = Result<T, LockError>;

impl LockfileV3 {
    /// Construct an empty v3 lockfile for `root`. Self-integrity is
    /// computed at construction so a caller may immediately serialise.
    pub fn new(root: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp();
        let mut lf = Self {
            version: SCHEMA_VERSION,
            root: root.into(),
            self_integrity: Integrity::blake3_of(b""), // placeholder
            manifest_integrity: None,
            created_at: now,
            updated_at: now,
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            packages: Vec::new(),
        };
        lf.refresh_self_integrity();
        lf
    }

    /// Read a lockfile from disk. Fails if:
    ///   - The file is missing or unreadable.
    ///   - The TOML is malformed.
    ///   - The `version` field doesn't match [`SCHEMA_VERSION`].
    ///   - The `self_integrity` digest doesn't match the recomputed
    ///     digest (tamper detection).
    ///
    /// Verify-on-every-load means the caller never has to remember to
    /// run a separate verify step for the lockfile itself. Cross-store
    /// verification is opt-in via [`verify_against_store`].
    pub fn from_file(path: &Path) -> LockResult<Self> {
        let text = fs::read_to_string(path).map_err(|source| LockError::Io {
            op: "read",
            path: path.to_path_buf(),
            source,
        })?;
        let lf: Self = toml::from_str(&text).map_err(|e| LockError::ParseError {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        if lf.version != SCHEMA_VERSION {
            return Err(LockError::SchemaSkew {
                found: lf.version,
                expected: SCHEMA_VERSION,
            });
        }
        let recomputed = compute_self_integrity(&lf);
        if recomputed != lf.self_integrity {
            return Err(LockError::SelfIntegrityFailure {
                recorded: lf.self_integrity.clone(),
                computed: recomputed,
            });
        }
        Ok(lf)
    }

    /// Atomically write to disk: refresh self-integrity, canonicalise
    /// sort order, write through tempfile + rename. The `updated_at`
    /// timestamp is bumped before serialisation.
    pub fn to_file(&mut self, path: &Path) -> LockResult<()> {
        self.canonicalise();
        self.updated_at = chrono::Utc::now().timestamp();
        self.refresh_self_integrity();
        let body = toml::to_string_pretty(self).map_err(|e| LockError::ParseError {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| LockError::Io {
                    op: "mkdir parent",
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }
        let tmp = path.with_extension(format!(
            "lock.tmp.{}.{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp).map_err(|source| LockError::Io {
                op: "create tempfile",
                path: tmp.clone(),
                source,
            })?;
            f.write_all(body.as_bytes()).map_err(|source| LockError::Io {
                op: "write body",
                path: tmp.clone(),
                source,
            })?;
            f.sync_all().ok();
        }
        fs::rename(&tmp, path).map_err(|source| {
            let _ = fs::remove_file(&tmp);
            LockError::Io {
                op: "rename into place",
                path: path.to_path_buf(),
                source,
            }
        })?;
        Ok(())
    }

    /// Sort packages by (name, version), and per-package dependencies
    /// + features. Keeps lockfile diffs minimal across resolver runs.
    pub fn canonicalise(&mut self) {
        self.packages
            .sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        for pkg in &mut self.packages {
            pkg.dependencies
                .sort_by(|a, b| (&a[0], &a[1]).cmp(&(&b[0], &b[1])));
            pkg.features.sort();
            pkg.features.dedup();
        }
    }

    /// Recompute and store `self_integrity` from current package set.
    /// Idempotent.
    pub fn refresh_self_integrity(&mut self) {
        // Two-pass: temporarily zero the field so the digest doesn't
        // include the previous digest. (compute_self_integrity already
        // ignores `self_integrity`, but doing this makes the
        // computation observable: any caller can recompute the same
        // value by replicating the function.)
        self.self_integrity = compute_self_integrity(self);
    }

    /// Insert / replace a locked package. Returns `true` if a previous
    /// entry was overwritten.
    pub fn upsert_package(&mut self, pkg: LockedPackage) -> bool {
        let replaced = self
            .packages
            .iter()
            .position(|p| p.name == pkg.name && p.version == pkg.version);
        match replaced {
            Some(i) => {
                self.packages[i] = pkg;
                true
            }
            None => {
                self.packages.push(pkg);
                false
            }
        }
    }

    /// Remove every entry matching `name`. Returns the count removed.
    pub fn remove_by_name(&mut self, name: &str) -> usize {
        let before = self.packages.len();
        self.packages.retain(|p| p.name != name);
        before - self.packages.len()
    }

    /// Find a package by exact `(name, version)`.
    pub fn find(&self, name: &str, version: &str) -> Option<&LockedPackage> {
        self.packages
            .iter()
            .find(|p| p.name == name && p.version == version)
    }

    /// All package versions for `name` (preserves declared order).
    pub fn versions_of<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a LockedPackage> {
        self.packages.iter().filter(move |p| p.name == name)
    }

    /// Verify that the `self_integrity` field matches the recomputed
    /// digest. [`from_file`] runs this for you; call directly if you
    /// constructed a lockfile by hand and want belt-and-braces.
    pub fn verify_self_integrity(&self) -> LockResult<()> {
        let recomputed = compute_self_integrity(self);
        if recomputed != self.self_integrity {
            return Err(LockError::SelfIntegrityFailure {
                recorded: self.self_integrity.clone(),
                computed: recomputed,
            });
        }
        Ok(())
    }

    /// Cross-check every locked package against `store`:
    ///
    ///   - Every package's blake3 integrity must resolve to a present
    ///     blob in the store.
    ///   - Every present blob must pass [`ContentStore::lookup_by_digest`]'s
    ///     own re-hash check (catches local tampering).
    ///
    /// Non-blake3 integrity entries are skipped (nothing to compare
    /// against the blake3-keyed store). Returns `Ok(())` only if every
    /// blake3 entry is materialised and integrity-clean. A failure
    /// reports both lists in one structured error so the caller can
    /// surface a complete repair plan.
    pub fn verify_against_store(&self, store: &ContentStore) -> LockResult<()> {
        let mut missing = Vec::new();
        let mut broken = Vec::new();
        for pkg in &self.packages {
            let digest = match pkg.integrity.to_blake3_digest() {
                Some(d) => d,
                None => continue, // non-blake3 — not store-keyed
            };
            match store.lookup_by_digest(digest) {
                Ok(Some(_)) => {}
                Ok(None) => missing.push(format!("{} {}", pkg.name, pkg.version)),
                Err(e) => broken.push((format!("{} {}", pkg.name, pkg.version), e)),
            }
        }
        if missing.is_empty() && broken.is_empty() {
            Ok(())
        } else {
            Err(LockError::StoreVerificationFailed { missing, broken })
        }
    }
}

/// Canonical bytes that the self-integrity digest is computed over.
/// Excludes the `self_integrity` field itself (else the digest would
/// be self-referential), and excludes timestamps + cli_version since
/// those are administrative metadata that legitimately changes per-write.
pub fn self_integrity_payload(lf: &LockfileV3) -> Vec<u8> {
    let mut canonical = LockfileCanonical {
        version: lf.version,
        root: &lf.root,
        manifest_integrity: lf
            .manifest_integrity
            .as_ref()
            .map(|i| i.to_wire())
            .unwrap_or_default(),
        packages: BTreeMap::new(),
    };
    for pkg in &lf.packages {
        let key = format!("{}:{}", pkg.name, pkg.version);
        let mut deps: Vec<String> = pkg
            .dependencies
            .iter()
            .map(|p| format!("{}={}", p[0], p[1]))
            .collect();
        deps.sort();
        let mut feats = pkg.features.clone();
        feats.sort();
        canonical.packages.insert(
            key,
            CanonicalPackage {
                source: &pkg.source,
                integrity: pkg.integrity.to_wire(),
                dependencies: deps,
                features: feats,
                optional: pkg.optional,
            },
        );
    }
    // Hand-rolled deterministic encoding — no JSON / TOML round-trip
    // (their formatters drift across versions).
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(b"v3\0");
    buf.extend_from_slice(format!("{}\0", canonical.version).as_bytes());
    buf.extend_from_slice(canonical.root.as_bytes());
    buf.push(0);
    buf.extend_from_slice(canonical.manifest_integrity.as_bytes());
    buf.push(0);
    for (key, pkg) in &canonical.packages {
        buf.extend_from_slice(key.as_bytes());
        buf.push(0);
        buf.extend_from_slice(pkg.source.as_bytes());
        buf.push(0);
        buf.extend_from_slice(pkg.integrity.as_bytes());
        buf.push(0);
        for d in &pkg.dependencies {
            buf.extend_from_slice(d.as_bytes());
            buf.push(b',');
        }
        buf.push(0);
        for f in &pkg.features {
            buf.extend_from_slice(f.as_bytes());
            buf.push(b',');
        }
        buf.push(0);
        buf.push(if pkg.optional { 1 } else { 0 });
        buf.push(0);
    }
    buf
}

/// Compute a blake3 self-integrity digest for `lf` (excluding the
/// `self_integrity` field and administrative timestamps).
pub fn compute_self_integrity(lf: &LockfileV3) -> Integrity {
    Integrity::blake3_of(&self_integrity_payload(lf))
}

#[derive(Debug)]
struct LockfileCanonical<'a> {
    version: u32,
    root: &'a str,
    manifest_integrity: String,
    packages: BTreeMap<String, CanonicalPackage<'a>>,
}

#[derive(Debug)]
struct CanonicalPackage<'a> {
    source: &'a str,
    integrity: String,
    dependencies: Vec<String>,
    features: Vec<String>,
    optional: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::content_store::{ContentStore, Meta};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::TempDir;

    static SUFFIX: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        let n = SUFFIX.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "verum_lockfile_v3_{}_{}_{}_{}",
            label,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n,
        ))
    }

    fn pkg(name: &str, version: &str, blob: &[u8]) -> LockedPackage {
        LockedPackage {
            name: name.to_string(),
            version: version.to_string(),
            source: "registry+https://cogs.verum-lang.org".to_string(),
            integrity: Integrity::blake3_of(blob),
            dependencies: Vec::new(),
            features: Vec::new(),
            optional: false,
        }
    }

    // ── Integrity ────────────────────────────────────────────────────

    #[test]
    fn integrity_blake3_round_trip() {
        let i = Integrity::blake3_of(b"hello");
        let s = i.to_wire();
        assert!(s.starts_with("blake3:"));
        let parsed = Integrity::parse(&s).unwrap();
        assert_eq!(parsed, i);
    }

    #[test]
    fn integrity_parse_rejects_bad_input() {
        assert!(Integrity::parse("").is_none());
        assert!(Integrity::parse("blake3").is_none());
        assert!(Integrity::parse("blake3:short").is_none());
        assert!(Integrity::parse(&format!("blake3:{}", "g".repeat(64))).is_none());
        assert!(Integrity::parse(&format!("md5:{}", "a".repeat(32))).is_none());
    }

    #[test]
    fn integrity_recognises_sha256_for_compat() {
        let s = format!("sha256:{}", "a".repeat(64));
        let i = Integrity::parse(&s).unwrap();
        assert_eq!(i.algorithm, HashAlgorithm::Sha256);
        assert!(i.to_blake3_digest().is_none());
    }

    #[test]
    fn integrity_to_blake3_digest_succeeds_for_blake3() {
        let i = Integrity::blake3_of(b"x");
        let d = i.to_blake3_digest().expect("blake3");
        assert_eq!(d.to_hex(), i.hex);
    }

    #[test]
    fn integrity_serde_via_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
        struct Wrap {
            i: Integrity,
        }
        let w = Wrap {
            i: Integrity::blake3_of(b"z"),
        };
        let s = toml::to_string(&w).unwrap();
        assert!(s.contains("blake3:"));
        let back: Wrap = toml::from_str(&s).unwrap();
        assert_eq!(back, w);
    }

    // ── construction + canonicalisation ──────────────────────────────

    #[test]
    fn new_lockfile_passes_self_integrity() {
        let lf = LockfileV3::new("root");
        lf.verify_self_integrity().unwrap();
        assert_eq!(lf.version, SCHEMA_VERSION);
        assert_eq!(lf.root, "root");
        assert!(lf.packages.is_empty());
    }

    #[test]
    fn upsert_replaces_same_name_version() {
        let mut lf = LockfileV3::new("r");
        let p1 = pkg("a", "1.0.0", b"v1");
        let p2 = pkg("a", "1.0.0", b"v2");
        assert_eq!(lf.upsert_package(p1.clone()), false);
        assert_eq!(lf.upsert_package(p2.clone()), true);
        assert_eq!(lf.packages.len(), 1);
        assert_eq!(lf.packages[0], p2);
    }

    #[test]
    fn remove_by_name_clears_all_versions() {
        let mut lf = LockfileV3::new("r");
        lf.upsert_package(pkg("a", "1.0.0", b"x"));
        lf.upsert_package(pkg("a", "1.0.1", b"y"));
        lf.upsert_package(pkg("b", "1.0.0", b"z"));
        assert_eq!(lf.remove_by_name("a"), 2);
        assert_eq!(lf.packages.len(), 1);
        assert_eq!(lf.packages[0].name, "b");
    }

    #[test]
    fn canonicalise_sorts_packages_deps_features() {
        let mut lf = LockfileV3::new("r");
        let mut p = pkg("z", "1.0.0", b"x");
        p.dependencies = vec![
            ["banana".into(), "1".into()],
            ["apple".into(), "2".into()],
            ["apple".into(), "1".into()],
        ];
        p.features = vec!["f2".into(), "f1".into(), "f1".into()];
        lf.upsert_package(p);
        lf.upsert_package(pkg("a", "2.0.0", b"y"));
        lf.upsert_package(pkg("a", "1.0.0", b"z"));
        lf.canonicalise();
        let names: Vec<_> = lf
            .packages
            .iter()
            .map(|p| (p.name.clone(), p.version.clone()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("a".into(), "1.0.0".into()),
                ("a".into(), "2.0.0".into()),
                ("z".into(), "1.0.0".into()),
            ]
        );
        let z = &lf.packages[2];
        assert_eq!(z.dependencies[0], ["apple".to_string(), "1".to_string()]);
        assert_eq!(z.dependencies[1], ["apple".to_string(), "2".to_string()]);
        assert_eq!(z.dependencies[2], ["banana".to_string(), "1".to_string()]);
        assert_eq!(z.features, vec!["f1".to_string(), "f2".to_string()]);
    }

    // ── self-integrity ───────────────────────────────────────────────

    #[test]
    fn self_integrity_changes_when_a_package_changes() {
        let mut lf = LockfileV3::new("r");
        lf.refresh_self_integrity();
        let i_empty = lf.self_integrity.clone();
        lf.upsert_package(pkg("a", "1.0.0", b"x"));
        lf.refresh_self_integrity();
        assert_ne!(lf.self_integrity, i_empty);
    }

    #[test]
    fn self_integrity_independent_of_timestamps() {
        let mut a = LockfileV3::new("r");
        a.upsert_package(pkg("x", "1.0.0", b"data"));
        a.refresh_self_integrity();
        let mut b = a.clone();
        b.created_at = a.created_at + 999_999;
        b.updated_at = a.updated_at + 999_999;
        b.cli_version = "future".into();
        let payload_a = self_integrity_payload(&a);
        let payload_b = self_integrity_payload(&b);
        assert_eq!(payload_a, payload_b);
    }

    #[test]
    fn self_integrity_invariant_under_deps_features_reordering() {
        let mut a = LockfileV3::new("r");
        let mut pa = pkg("x", "1.0.0", b"data");
        pa.dependencies = vec![["y".into(), "1".into()], ["z".into(), "1".into()]];
        pa.features = vec!["f1".into(), "f2".into()];
        a.upsert_package(pa);

        let mut b = LockfileV3::new("r");
        let mut pb = pkg("x", "1.0.0", b"data");
        pb.dependencies = vec![["z".into(), "1".into()], ["y".into(), "1".into()]];
        pb.features = vec!["f2".into(), "f1".into()];
        b.upsert_package(pb);

        // Different declared order, but canonicalised payload should match.
        assert_eq!(self_integrity_payload(&a), self_integrity_payload(&b));
    }

    #[test]
    fn verify_self_integrity_detects_tampering() {
        let mut lf = LockfileV3::new("r");
        lf.upsert_package(pkg("a", "1.0.0", b"clean"));
        lf.refresh_self_integrity();
        // Tamper an entry without re-running refresh_self_integrity.
        lf.packages[0].integrity = Integrity::blake3_of(b"tampered");
        let err = lf.verify_self_integrity().unwrap_err();
        assert!(matches!(err, LockError::SelfIntegrityFailure { .. }));
    }

    // ── round-trip via disk ──────────────────────────────────────────

    #[test]
    fn write_then_read_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("verum.lock");
        let mut lf = LockfileV3::new("myapp");
        lf.upsert_package(pkg("json", "1.0.0", b"json-blob"));
        lf.upsert_package(pkg("http", "0.2.0", b"http-blob"));
        lf.to_file(&path).unwrap();

        let read = LockfileV3::from_file(&path).unwrap();
        assert_eq!(read.root, "myapp");
        assert_eq!(read.packages.len(), 2);
        // Canonical sort: http < json.
        assert_eq!(read.packages[0].name, "http");
        assert_eq!(read.packages[1].name, "json");
        // Self-integrity verified during from_file.
        read.verify_self_integrity().unwrap();
    }

    #[test]
    fn from_file_rejects_schema_skew() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("verum.lock");
        let body = "version = 99\n\
                    root = \"r\"\n\
                    self_integrity = \"blake3:00\"\n\
                    created_at = 0\n\
                    updated_at = 0\n\
                    cli_version = \"x\"\n";
        // The integrity literal is malformed (length wrong) so this
        // fails at parse if version check runs late. We explicitly want
        // the SchemaSkew variant — verify by writing a syntactically
        // valid v99 file with a 64-char blake3.
        let placeholder = "0".repeat(64);
        let body = body.replace("\"blake3:00\"", &format!("\"blake3:{placeholder}\""));
        fs::write(&path, body).unwrap();
        let err = LockfileV3::from_file(&path).unwrap_err();
        assert!(
            matches!(err, LockError::SchemaSkew { found: 99, .. }),
            "expected SchemaSkew, got {err:?}"
        );
    }

    #[test]
    fn from_file_rejects_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("verum.lock");
        let err = LockfileV3::from_file(&path).unwrap_err();
        assert!(matches!(err, LockError::Io { .. }));
    }

    #[test]
    fn from_file_rejects_garbage_toml() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("verum.lock");
        fs::write(&path, b":::not toml:::").unwrap();
        let err = LockfileV3::from_file(&path).unwrap_err();
        assert!(matches!(err, LockError::ParseError { .. }));
    }

    #[test]
    fn from_file_detects_self_integrity_failure() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("verum.lock");
        let mut lf = LockfileV3::new("r");
        lf.upsert_package(pkg("a", "1.0.0", b"x"));
        lf.to_file(&path).unwrap();
        // Tamper the on-disk file: replace a package version without
        // recomputing the self-integrity digest.
        let text = fs::read_to_string(&path).unwrap();
        let tampered = text.replace("\"1.0.0\"", "\"9.9.9\"");
        fs::write(&path, tampered).unwrap();
        let err = LockfileV3::from_file(&path).unwrap_err();
        assert!(matches!(err, LockError::SelfIntegrityFailure { .. }));
    }

    #[test]
    fn write_is_atomic_no_tempfile_residue() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("verum.lock");
        let mut lf = LockfileV3::new("r");
        lf.to_file(&path).unwrap();
        assert!(path.is_file());
        for entry in fs::read_dir(path.parent().unwrap()).unwrap() {
            let name = entry.unwrap().file_name();
            let s = name.to_string_lossy();
            assert!(
                !s.contains(".lock.tmp."),
                "leftover tempfile: {s}"
            );
        }
    }

    // ── verify_against_store ─────────────────────────────────────────

    #[test]
    fn verify_against_store_passes_when_all_blobs_present() {
        let root = temp_root("store_pass");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob_a = b"blob-a";
        let blob_b = b"blob-b";
        store
            .insert(
                blob_a,
                None,
                Meta {
                    schema_version:
                        crate::registry::content_store::META_SCHEMA_VERSION,
                    name: "a".into(),
                    version: "1.0.0".into(),
                    source: "registry".into(),
                    retrieved_at: 0,
                    last_accessed_at: 0,
                    size: 0,
                },
            )
            .unwrap();
        store
            .insert(
                blob_b,
                None,
                Meta {
                    schema_version:
                        crate::registry::content_store::META_SCHEMA_VERSION,
                    name: "b".into(),
                    version: "1.0.0".into(),
                    source: "registry".into(),
                    retrieved_at: 0,
                    last_accessed_at: 0,
                    size: 0,
                },
            )
            .unwrap();

        let mut lf = LockfileV3::new("r");
        lf.upsert_package(pkg("a", "1.0.0", blob_a));
        lf.upsert_package(pkg("b", "1.0.0", blob_b));
        lf.refresh_self_integrity();
        lf.verify_against_store(&store).unwrap();

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn verify_against_store_reports_missing_packages() {
        let root = temp_root("store_missing");
        let store = ContentStore::at(root.clone()).unwrap();
        let mut lf = LockfileV3::new("r");
        lf.upsert_package(pkg("a", "1.0.0", b"never-stored"));
        lf.upsert_package(pkg("b", "2.0.0", b"also-missing"));
        lf.refresh_self_integrity();

        let err = lf.verify_against_store(&store).unwrap_err();
        match err {
            LockError::StoreVerificationFailed { missing, broken } => {
                assert_eq!(missing.len(), 2);
                assert!(missing.iter().any(|m| m.starts_with("a 1.0.0")));
                assert!(missing.iter().any(|m| m.starts_with("b 2.0.0")));
                assert!(broken.is_empty());
            }
            other => panic!("expected StoreVerificationFailed, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn verify_against_store_skips_non_blake3_entries() {
        let root = temp_root("store_skip");
        let store = ContentStore::at(root.clone()).unwrap();
        let mut lf = LockfileV3::new("r");
        let mut p = pkg("a", "1.0.0", b"x");
        // Force a sha256 integrity tag.
        p.integrity = Integrity::parse(&format!("sha256:{}", "a".repeat(64))).unwrap();
        lf.upsert_package(p);
        lf.refresh_self_integrity();
        // No blake3 entries to check → trivially passes.
        lf.verify_against_store(&store).unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
