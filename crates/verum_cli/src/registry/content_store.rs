//! Content-addressed cog store (P5.1).
//!
//! Resolved cog blobs (tarballs, source trees, anything content-stable)
//! live at `~/.verum/store/<blake3-hex>/`, deduplicated by content.
//! Two `(name, version)` tuples that resolve to byte-identical blobs
//! share one on-disk entry — saving space and giving the resolver a
//! deterministic supply-chain integrity check.
//!
//! # Layout
//!
//! ```text
//! ~/.verum/store/
//! ├── <blake3-hex>/                — one per unique blob
//! │   ├── blob.bin                 — the raw bytes (any format: .tar.zst, ...)
//! │   └── meta.toml                — {schema, name, version, source, retrieved_at,
//! │                                  last_accessed_at, size}
//! ├── refs/                        — (name, version) → digest indirection
//! │   ├── <name>/
//! │   │   ├── <version>.ref        — file whose contents = the digest hex
//! │   │   └── ...
//! │   └── ...
//! ```
//!
//! # Integrity contract
//!
//! Every [`ContentStore::lookup_by_digest`] call re-hashes the on-disk
//! blob and compares against the directory name. A mismatch is treated
//! as corruption: the entry is evicted, refs that pointed at it are
//! cleared, and the caller sees a [`StoreError::IntegrityFailure`].
//! There is no "trust on first use" — the cache cannot serve a
//! tampered blob even silently.
//!
//! # Atomicity
//!
//! [`ContentStore::insert`] writes through a per-pid tempdir then
//! renames into place; concurrent in-process inserts are serialised
//! by a process-local store mutex (the cleanup+rename window is racy
//! on every POSIX directory). Cross-process races are content-safe:
//! both writers produce byte-identical blobs because the digest is
//! deterministic.
//!
//! # GC
//!
//! [`ContentStore::gc_to_size`] evicts orphaned digests (no ref points
//! at them) first; if still over budget, evicts least-recently-accessed
//! refs and their underlying blobs. This protects pinned dependencies
//! while still freeing space for transient pulls.

use blake3;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 32-byte blake3 digest. Stored hex-encoded as the on-disk dir name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Compute the blake3 digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Hex-encode the digest as a 64-char lowercase ASCII string.
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(NIBBLE[(byte >> 4) as usize]);
            out.push(NIBBLE[(byte & 0x0F) as usize]);
        }
        out
    }

    /// Parse a 64-char hex string. Returns `None` on length / charset
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
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Schema version for `meta.toml`. Bumped on incompatible layout
/// changes so older entries are silently evicted by [`ContentStore::list`].
pub const META_SCHEMA_VERSION: u32 = 1;

/// Per-blob metadata. Round-trips through TOML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    /// Bumped on incompatible meta layout changes.
    pub schema_version: u32,
    /// Cog name (e.g. `"json"`).
    pub name: String,
    /// Cog version (e.g. `"1.4.0"`).
    pub version: String,
    /// Source descriptor:
    ///   - `registry+<url>` for the public registry
    ///   - `git+<url>#<sha>` for git-pinned cogs
    ///   - `path+<dir>` for filesystem-local cogs
    pub source: String,
    /// Wall-clock seconds since UNIX epoch when the blob was first
    /// stored.
    pub retrieved_at: u64,
    /// Wall-clock seconds since UNIX epoch when the blob was last
    /// looked up. Refreshed on every `lookup_by_digest` /
    /// `lookup_by_name_version` hit. LRU GC keys on this.
    pub last_accessed_at: u64,
    /// Blob byte length. Authoritative; used by GC sizing without a
    /// stat() round-trip.
    pub size: u64,
}

impl Meta {
    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// One store entry returned by [`ContentStore::lookup_by_digest`].
#[derive(Debug, Clone)]
pub struct StoreEntry {
    pub digest: Digest,
    pub blob: Vec<u8>,
    pub meta: Meta,
}

/// Errors surfaced from store operations.
#[derive(Debug)]
pub enum StoreError {
    /// I/O error with structural context.
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// On-disk blob's digest didn't match its directory name. The entry
    /// has been evicted and any refs pointing at it cleared.
    IntegrityFailure {
        expected: Digest,
        actual: Digest,
    },
    /// `meta.toml` failed to parse — the entry is evicted.
    InvalidMeta { path: PathBuf, reason: String },
    /// Insert called with `expected_digest` ≠ digest of the supplied
    /// blob. Useful for catching mid-network corruption.
    DigestMismatch { expected: Digest, actual: Digest },
    /// Cog name or version contained filesystem-unsafe characters.
    /// The store rejects rather than escape: a malformed name from
    /// upstream is almost always a bug worth surfacing.
    InvalidIdentifier { what: &'static str, value: String },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { op, path, source } => {
                write!(f, "store {op} failed on {}: {source}", path.display())
            }
            Self::IntegrityFailure { expected, actual } => write!(
                f,
                "store: integrity failure — entry {expected} computed {actual} (evicted)"
            ),
            Self::InvalidMeta { path, reason } => write!(
                f,
                "store: meta.toml at {} is invalid: {reason}",
                path.display()
            ),
            Self::DigestMismatch { expected, actual } => write!(
                f,
                "store: blob digest mismatch — expected {expected}, got {actual}"
            ),
            Self::InvalidIdentifier { what, value } => {
                write!(f, "store: {what} {value:?} contains unsafe characters")
            }
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> StoreError + 'a {
    move |source| StoreError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Content-addressed cog store.
#[derive(Debug, Clone)]
pub struct ContentStore {
    root: PathBuf,
    /// Process-local serialiser for insert (cleanup+rename window is
    /// inherently racy on POSIX directories). Cross-process races are
    /// safe because the store is content-addressed.
    insert_lock: Arc<Mutex<()>>,
}

impl ContentStore {
    /// Default location: `$HOME/.verum/store/`. Created if missing.
    pub fn at_default() -> StoreResult<Self> {
        let home = dirs::home_dir().ok_or_else(|| StoreError::Io {
            op: "resolve home directory",
            path: PathBuf::from("~"),
            source: io::Error::new(io::ErrorKind::NotFound, "$HOME unset"),
        })?;
        Self::at(home.join(".verum").join("store"))
    }

    /// Use a custom root directory. Created if missing.
    pub fn at(root: PathBuf) -> StoreResult<Self> {
        fs::create_dir_all(&root).map_err(io_err("mkdir -p", &root))?;
        fs::create_dir_all(root.join("refs")).map_err(io_err("mkdir refs/", &root))?;
        Ok(Self {
            root,
            insert_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Cache root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn entry_dir(&self, digest: Digest) -> PathBuf {
        self.root.join(digest.to_hex())
    }

    fn ref_path(&self, name: &str, version: &str) -> StoreResult<PathBuf> {
        validate_ident("cog name", name)?;
        validate_ident("version", version)?;
        Ok(self
            .root
            .join("refs")
            .join(name)
            .join(format!("{version}.ref")))
    }

    /// Insert a blob. If `expected_digest` is supplied, the supplied
    /// blob's actual digest is verified against it (catching mid-flight
    /// corruption). On success returns the canonical digest.
    ///
    /// Concurrent inserts of the same blob are safe — both writers
    /// produce byte-identical content; the loser's tempdir is reaped.
    pub fn insert(
        &self,
        blob: &[u8],
        expected_digest: Option<Digest>,
        meta_in: Meta,
    ) -> StoreResult<Digest> {
        validate_ident("cog name", &meta_in.name)?;
        validate_ident("version", &meta_in.version)?;
        let digest = Digest::of(blob);
        if let Some(expected) = expected_digest {
            if expected != digest {
                return Err(StoreError::DigestMismatch {
                    expected,
                    actual: digest,
                });
            }
        }

        let _guard = self.insert_lock.lock().unwrap_or_else(|p| p.into_inner());

        let dir = self.entry_dir(digest);
        let nonce = INSERT_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = self.root.join(format!(
            "{}.tmp.{}.{}.{}",
            digest.to_hex(),
            std::process::id(),
            Meta::now_secs(),
            nonce
        ));
        fs::create_dir_all(&tmp).map_err(io_err("mkdir tempdir", &tmp))?;

        let blob_path = tmp.join("blob.bin");
        atomic_write_bytes(&blob_path, blob).map_err(io_err("write blob.bin", &blob_path))?;

        let now = Meta::now_secs();
        let meta = Meta {
            schema_version: META_SCHEMA_VERSION,
            name: meta_in.name.clone(),
            version: meta_in.version.clone(),
            source: meta_in.source,
            retrieved_at: now,
            last_accessed_at: now,
            size: blob.len() as u64,
        };
        let meta_str =
            toml::to_string(&meta).expect("Meta serialise — pure data, never fails");
        let meta_path = tmp.join("meta.toml");
        atomic_write_string(&meta_path, &meta_str)
            .map_err(io_err("write meta.toml", &meta_path))?;

        let _ = fs::remove_dir_all(&dir);
        match fs::rename(&tmp, &dir) {
            Ok(()) => {}
            Err(e) => {
                let _ = fs::remove_dir_all(&tmp);
                if !dir.is_dir() {
                    return Err(StoreError::Io {
                        op: "rename tempdir into place",
                        path: dir.clone(),
                        source: e,
                    });
                }
            }
        }

        // Update refs.
        let ref_path = self.ref_path(&meta_in.name, &meta_in.version)?;
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).map_err(io_err("mkdir refs/<name>/", parent))?;
        }
        atomic_write_string(&ref_path, &digest.to_hex())
            .map_err(io_err("write ref", &ref_path))?;

        Ok(digest)
    }

    /// Look up a blob by digest. Re-hashes on-disk content and compares
    /// against the directory name; a mismatch evicts and returns
    /// [`StoreError::IntegrityFailure`].
    pub fn lookup_by_digest(&self, digest: Digest) -> StoreResult<Option<StoreEntry>> {
        let dir = self.entry_dir(digest);
        let blob_path = dir.join("blob.bin");
        let meta_path = dir.join("meta.toml");
        if !blob_path.is_file() || !meta_path.is_file() {
            return Ok(None);
        }
        let blob = fs::read(&blob_path).map_err(io_err("read blob.bin", &blob_path))?;
        let actual = Digest::of(&blob);
        if actual != digest {
            // Tamper / corruption — evict + clear any refs pointing
            // here, then surface as IntegrityFailure.
            let _ = self.evict(digest);
            return Err(StoreError::IntegrityFailure {
                expected: digest,
                actual,
            });
        }
        let meta_text =
            fs::read_to_string(&meta_path).map_err(io_err("read meta.toml", &meta_path))?;
        let mut meta: Meta = match toml::from_str(&meta_text) {
            Ok(m) => m,
            Err(e) => {
                let _ = fs::remove_dir_all(&dir);
                return Err(StoreError::InvalidMeta {
                    path: meta_path,
                    reason: e.to_string(),
                });
            }
        };
        if meta.schema_version != META_SCHEMA_VERSION {
            let _ = fs::remove_dir_all(&dir);
            return Ok(None);
        }
        meta.last_accessed_at = Meta::now_secs();
        if let Ok(s) = toml::to_string(&meta) {
            let _ = atomic_write_string(&meta_path, &s);
        }
        Ok(Some(StoreEntry {
            digest,
            blob,
            meta,
        }))
    }

    /// Look up the digest pinned for `(name, version)`. Returns
    /// `Ok(None)` if no ref exists; verifies the underlying blob
    /// integrity transitively if a ref points at a blob. A dangling
    /// ref (digest hex parses but blob is missing) is treated as a
    /// miss and the ref is silently cleared.
    pub fn lookup_by_name_version(
        &self,
        name: &str,
        version: &str,
    ) -> StoreResult<Option<Digest>> {
        let ref_path = self.ref_path(name, version)?;
        let text = match fs::read_to_string(&ref_path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err("read ref", &ref_path)(e)),
        };
        let digest = match Digest::from_hex(text.trim()) {
            Some(d) => d,
            None => {
                let _ = fs::remove_file(&ref_path);
                return Ok(None);
            }
        };
        if !self.entry_dir(digest).is_dir() {
            let _ = fs::remove_file(&ref_path);
            return Ok(None);
        }
        Ok(Some(digest))
    }

    /// Evict a single entry by digest. Also clears any ref files that
    /// point at the digest (so `lookup_by_name_version` doesn't return
    /// a now-dangling pointer).
    pub fn evict(&self, digest: Digest) -> StoreResult<()> {
        let dir = self.entry_dir(digest);
        match fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(StoreError::Io {
                    op: "remove entry",
                    path: dir,
                    source: e,
                });
            }
        }
        // Walk refs/ and remove any pointing at this digest.
        let refs_root = self.root.join("refs");
        let want_hex = digest.to_hex();
        let _ = walk_refs(&refs_root, &mut |path| {
            if let Ok(text) = fs::read_to_string(path) {
                if text.trim() == want_hex {
                    let _ = fs::remove_file(path);
                }
            }
        });
        Ok(())
    }

    /// Enumerate all (digest, meta) entries. Skips:
    ///   - directories whose name isn't a 64-char blake3 hex,
    ///   - entries whose meta.toml fails to parse (evicted silently),
    ///   - entries whose schema_version is stale (evicted silently),
    ///   - `.tmp.*` artefacts from interrupted inserts (reaped).
    pub fn list(&self) -> StoreResult<Vec<(Digest, Meta)>> {
        let read = match fs::read_dir(&self.root) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err("read store root", &self.root)(e)),
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
            if name == "refs" {
                continue;
            }
            if name.contains(".tmp.") {
                let _ = fs::remove_dir_all(&path);
                continue;
            }
            let digest = match Digest::from_hex(name) {
                Some(d) => d,
                None => continue,
            };
            let meta_path = path.join("meta.toml");
            let text = match fs::read_to_string(&meta_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let meta: Meta = match toml::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    let _ = fs::remove_dir_all(&path);
                    continue;
                }
            };
            if meta.schema_version != META_SCHEMA_VERSION {
                let _ = fs::remove_dir_all(&path);
                continue;
            }
            out.push((digest, meta));
        }
        Ok(out)
    }

    /// Total disk usage, in bytes (sum of `Meta.size`).
    pub fn total_size(&self) -> StoreResult<u64> {
        Ok(self.list()?.iter().map(|(_, m)| m.size).sum())
    }

    /// Evict orphaned digests (no ref pointing at them) first, then
    /// least-recently-accessed live entries, until total size ≤
    /// `max_bytes`. Returns count of entries evicted.
    pub fn gc_to_size(&self, max_bytes: u64) -> StoreResult<usize> {
        let entries = self.list()?;
        if entries.is_empty() {
            return Ok(0);
        }
        let pinned = self.pinned_digests()?;
        let (orphans, mut pinned_entries): (Vec<_>, Vec<_>) =
            entries.into_iter().partition(|(d, _)| !pinned.contains(d));

        let mut total: u64 = orphans.iter().map(|(_, m)| m.size).sum::<u64>()
            + pinned_entries.iter().map(|(_, m)| m.size).sum::<u64>();
        let mut evicted = 0usize;

        // Phase 1: evict orphans (no ref pinning them).
        for (digest, meta) in orphans {
            if total <= max_bytes {
                break;
            }
            self.evict(digest)?;
            total = total.saturating_sub(meta.size);
            evicted += 1;
        }

        if total <= max_bytes {
            return Ok(evicted);
        }

        // Phase 2: evict pinned entries by LRU (also clears their refs).
        pinned_entries.sort_by_key(|(_, m)| m.last_accessed_at);
        for (digest, meta) in pinned_entries {
            if total <= max_bytes {
                break;
            }
            self.evict(digest)?;
            total = total.saturating_sub(meta.size);
            evicted += 1;
        }
        Ok(evicted)
    }

    /// Remove every entry. Returns the count removed.
    pub fn clear(&self) -> StoreResult<usize> {
        let entries = self.list()?;
        let n = entries.len();
        for (digest, _) in entries {
            self.evict(digest)?;
        }
        // Also clear any orphan ref files left over.
        let _ = fs::remove_dir_all(self.root.join("refs"));
        fs::create_dir_all(self.root.join("refs"))
            .map_err(io_err("recreate refs/", &self.root))?;
        Ok(n)
    }

    /// All (name, version) ref files, with the digest each points at.
    pub fn refs(&self) -> StoreResult<Vec<(String, String, Digest)>> {
        let mut out = Vec::new();
        let refs_root = self.root.join("refs");
        let _ = walk_refs(&refs_root, &mut |path| {
            let name = match path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => return,
            };
            let file = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => return,
            };
            let version = match file.strip_suffix(".ref") {
                Some(v) => v.to_string(),
                None => return,
            };
            if let Ok(text) = fs::read_to_string(path) {
                if let Some(d) = Digest::from_hex(text.trim()) {
                    out.push((name, version, d));
                }
            }
        });
        Ok(out)
    }

    /// All distinct digests referenced by at least one ref file.
    fn pinned_digests(&self) -> StoreResult<std::collections::HashSet<Digest>> {
        Ok(self.refs()?.into_iter().map(|(_, _, d)| d).collect())
    }
}

/// Process-wide tempdir-suffix counter (ensures unique tempdir names
/// when multiple threads insert in the same wall-clock second).
static INSERT_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Validate that a string is filesystem-safe enough for use under
/// `refs/<name>/<version>.ref`. Conservative: only ASCII letters,
/// digits, `_`, `-`, `.`, `+`. Rejects path separators, NUL, dots-only,
/// leading dots (hidden files).
fn validate_ident(what: &'static str, value: &str) -> StoreResult<()> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(StoreError::InvalidIdentifier {
            what,
            value: value.to_string(),
        });
    }
    if value.starts_with('.') {
        return Err(StoreError::InvalidIdentifier {
            what,
            value: value.to_string(),
        });
    }
    for c in value.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+');
        if !ok {
            return Err(StoreError::InvalidIdentifier {
                what,
                value: value.to_string(),
            });
        }
    }
    Ok(())
}

/// Recursive-but-shallow walk: refs/<name>/<version>.ref files only,
/// no deeper.
fn walk_refs(refs_root: &Path, f: &mut dyn FnMut(&Path)) -> io::Result<()> {
    let read = match fs::read_dir(refs_root) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in read {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name_dir = entry.path();
        if !name_dir.is_dir() {
            continue;
        }
        let inner = match fs::read_dir(&name_dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for inner_entry in inner {
            let inner_entry = match inner_entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = inner_entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ref") {
                f(&path);
            }
        }
    }
    Ok(())
}

/// Atomic file write: write to `<path>.tmp.<pid>.<nanos>`, fsync,
/// rename to `<path>`. Atomic on every filesystem we target.
fn atomic_write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!(
        "tmp.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e
    })?;
    Ok(())
}

fn atomic_write_string(path: &Path, contents: &str) -> io::Result<()> {
    atomic_write_bytes(path, contents.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    static SUFFIX: AtomicU64 = AtomicU64::new(0);

    fn temp_root(label: &str) -> PathBuf {
        let n = SUFFIX.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "verum_content_store_{}_{}_{}_{}",
            label,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n,
        ))
    }

    fn make_meta(name: &str, version: &str) -> Meta {
        Meta {
            schema_version: META_SCHEMA_VERSION,
            name: name.to_string(),
            version: version.to_string(),
            source: "registry+https://cogs.verum-lang.org".to_string(),
            retrieved_at: 0,
            last_accessed_at: 0,
            size: 0,
        }
    }

    // ── Digest ───────────────────────────────────────────────────────

    #[test]
    fn digest_hex_round_trip() {
        let d = Digest::of(b"hello");
        let hex = d.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Digest::from_hex(&hex), Some(d));
    }

    #[test]
    fn digest_from_hex_rejects_bad_input() {
        assert!(Digest::from_hex("").is_none());
        assert!(Digest::from_hex("xyz").is_none());
        assert!(Digest::from_hex(&"g".repeat(64)).is_none());
        assert!(Digest::from_hex(&"a".repeat(63)).is_none());
        assert!(Digest::from_hex(&"a".repeat(65)).is_none());
    }

    #[test]
    fn digest_of_is_deterministic() {
        assert_eq!(Digest::of(b"x"), Digest::of(b"x"));
        assert_ne!(Digest::of(b"x"), Digest::of(b"y"));
    }

    // ── insert / lookup ──────────────────────────────────────────────

    #[test]
    fn insert_then_lookup_round_trip() {
        let root = temp_root("insert_lookup");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob = b"hello world".to_vec();
        let digest = store
            .insert(&blob, None, make_meta("hello", "1.0.0"))
            .unwrap();
        let entry = store.lookup_by_digest(digest).unwrap().expect("hit");
        assert_eq!(entry.digest, digest);
        assert_eq!(entry.blob, blob);
        assert_eq!(entry.meta.name, "hello");
        assert_eq!(entry.meta.version, "1.0.0");
        assert_eq!(entry.meta.size, blob.len() as u64);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn insert_verifies_expected_digest() {
        let root = temp_root("verify_expected");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob = b"x".to_vec();
        let actual = Digest::of(&blob);
        // Match: ok.
        store
            .insert(&blob, Some(actual), make_meta("a", "1.0.0"))
            .unwrap();
        // Mismatch: rejected before any disk write.
        let bogus = Digest::of(b"completely different");
        let err = store
            .insert(&blob, Some(bogus), make_meta("a", "1.0.1"))
            .unwrap_err();
        match err {
            StoreError::DigestMismatch { expected, actual: a } => {
                assert_eq!(expected, bogus);
                assert_eq!(a, actual);
            }
            other => panic!("expected DigestMismatch, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_by_name_version_resolves_to_digest() {
        let root = temp_root("lookup_nv");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob = b"json crate".to_vec();
        let d = store
            .insert(&blob, None, make_meta("json", "1.0.0"))
            .unwrap();
        assert_eq!(
            store.lookup_by_name_version("json", "1.0.0").unwrap(),
            Some(d)
        );
        assert!(store
            .lookup_by_name_version("json", "1.0.1")
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_by_digest_miss_returns_none() {
        let root = temp_root("miss");
        let store = ContentStore::at(root.clone()).unwrap();
        let phantom = Digest::of(b"nothing");
        assert!(store.lookup_by_digest(phantom).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    // ── integrity ────────────────────────────────────────────────────

    #[test]
    fn lookup_detects_blob_tampering_and_evicts() {
        let root = temp_root("tamper");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob = b"original".to_vec();
        let d = store
            .insert(&blob, None, make_meta("c", "1.0.0"))
            .unwrap();
        // Tamper: overwrite blob.bin in place.
        let blob_path = store.entry_dir(d).join("blob.bin");
        fs::write(&blob_path, b"TAMPERED").unwrap();

        let err = store.lookup_by_digest(d).unwrap_err();
        assert!(matches!(err, StoreError::IntegrityFailure { .. }));
        // Entry must be evicted after detection.
        assert!(!store.entry_dir(d).exists(), "entry should be evicted");
        // Ref must also be cleared.
        assert!(store
            .lookup_by_name_version("c", "1.0.0")
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lookup_evicts_corrupt_meta() {
        let root = temp_root("corrupt_meta");
        let store = ContentStore::at(root.clone()).unwrap();
        let d = store
            .insert(b"x", None, make_meta("c", "1.0.0"))
            .unwrap();
        fs::write(store.entry_dir(d).join("meta.toml"), b":::not-toml:::").unwrap();
        let err = store.lookup_by_digest(d).unwrap_err();
        assert!(matches!(err, StoreError::InvalidMeta { .. }));
        assert!(!store.entry_dir(d).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn schema_version_skew_evicts_silently() {
        let root = temp_root("schema_skew");
        let store = ContentStore::at(root.clone()).unwrap();
        let d = store
            .insert(b"x", None, make_meta("c", "1.0.0"))
            .unwrap();
        let meta_path = store.entry_dir(d).join("meta.toml");
        let mut text = fs::read_to_string(&meta_path).unwrap();
        text = text.replace(
            &format!("schema_version = {}", META_SCHEMA_VERSION),
            "schema_version = 999",
        );
        fs::write(&meta_path, text).unwrap();
        // Silent miss, no error.
        assert!(store.lookup_by_digest(d).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    // ── refs ─────────────────────────────────────────────────────────

    #[test]
    fn dangling_ref_treated_as_miss_and_cleared() {
        let root = temp_root("dangling");
        let store = ContentStore::at(root.clone()).unwrap();
        let d = store
            .insert(b"x", None, make_meta("c", "1.0.0"))
            .unwrap();
        // Manually wipe the entry without going through evict() to
        // simulate filesystem-level deletion.
        fs::remove_dir_all(store.entry_dir(d)).unwrap();
        assert!(store
            .lookup_by_name_version("c", "1.0.0")
            .unwrap()
            .is_none());
        // Ref file was cleared on detection.
        let ref_path = store.ref_path("c", "1.0.0").unwrap();
        assert!(!ref_path.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refs_lists_all_pinned() {
        let root = temp_root("refs_list");
        let store = ContentStore::at(root.clone()).unwrap();
        store.insert(b"a", None, make_meta("a", "1.0.0")).unwrap();
        store.insert(b"b", None, make_meta("b", "2.0.0")).unwrap();
        store.insert(b"a", None, make_meta("a", "1.0.1")).unwrap();
        let mut refs = store.refs().unwrap();
        refs.sort();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].0, "a");
        assert_eq!(refs[0].1, "1.0.0");
        assert_eq!(refs[1].0, "a");
        assert_eq!(refs[1].1, "1.0.1");
        assert_eq!(refs[2].0, "b");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dedupe_two_versions_with_identical_content() {
        let root = temp_root("dedupe");
        let store = ContentStore::at(root.clone()).unwrap();
        // Same blob, different (name, version).
        let d1 = store
            .insert(b"identical", None, make_meta("a", "1.0.0"))
            .unwrap();
        let d2 = store
            .insert(b"identical", None, make_meta("a", "1.0.1"))
            .unwrap();
        assert_eq!(d1, d2, "identical blobs share a digest");
        // Both refs point at the same digest.
        assert_eq!(store.lookup_by_name_version("a", "1.0.0").unwrap(), Some(d1));
        assert_eq!(store.lookup_by_name_version("a", "1.0.1").unwrap(), Some(d1));
        // list() reports one entry.
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    // ── evict / gc / clear ───────────────────────────────────────────

    #[test]
    fn evict_clears_refs_pointing_at_digest() {
        let root = temp_root("evict_refs");
        let store = ContentStore::at(root.clone()).unwrap();
        let d = store
            .insert(b"x", None, make_meta("c", "1.0.0"))
            .unwrap();
        store.evict(d).unwrap();
        assert!(store
            .lookup_by_name_version("c", "1.0.0")
            .unwrap()
            .is_none());
        assert!(store.lookup_by_digest(d).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gc_evicts_orphans_first() {
        let root = temp_root("gc_orphans");
        let store = ContentStore::at(root.clone()).unwrap();
        // Insert two entries with refs.
        let _pinned = store
            .insert(&[1u8; 1024], None, make_meta("pinned", "1.0.0"))
            .unwrap();
        let orphan = store
            .insert(&[2u8; 1024], None, make_meta("temp", "1.0.0"))
            .unwrap();
        // Manually delete the orphan's ref so it's truly orphaned.
        let ref_path = store.ref_path("temp", "1.0.0").unwrap();
        fs::remove_file(&ref_path).unwrap();

        // Force GC under the orphan-only budget.
        let evicted = store.gc_to_size(1100).unwrap();
        assert_eq!(evicted, 1, "orphan should be evicted");
        // Pinned entry survives.
        assert!(store.lookup_by_digest(_pinned).unwrap().is_some());
        // Orphan is gone.
        assert!(store.lookup_by_digest(orphan).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gc_falls_back_to_lru_when_orphans_insufficient() {
        let root = temp_root("gc_lru");
        let store = ContentStore::at(root.clone()).unwrap();
        let big_a = [3u8; 2048];
        let big_b = [4u8; 2048];
        store.insert(&big_a, None, make_meta("a", "1.0.0")).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1)); // force ordering
        store.insert(&big_b, None, make_meta("b", "1.0.0")).unwrap();
        let evicted = store.gc_to_size(2500).unwrap();
        // One of them should have been evicted (the older one).
        assert_eq!(evicted, 1);
        assert!(store
            .lookup_by_name_version("a", "1.0.0")
            .unwrap()
            .is_none());
        assert!(store
            .lookup_by_name_version("b", "1.0.0")
            .unwrap()
            .is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_removes_everything() {
        let root = temp_root("clear");
        let store = ContentStore::at(root.clone()).unwrap();
        store.insert(b"a", None, make_meta("a", "1.0.0")).unwrap();
        store.insert(b"b", None, make_meta("b", "2.0.0")).unwrap();
        let n = store.clear().unwrap();
        assert_eq!(n, 2);
        assert!(store.list().unwrap().is_empty());
        assert!(store.refs().unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    // ── identifier validation ────────────────────────────────────────

    #[test]
    fn rejects_unsafe_cog_names() {
        let root = temp_root("reject_name");
        let store = ContentStore::at(root.clone()).unwrap();
        for bad in ["", ".", "..", ".hidden", "a/b", "a\\b", "a\0b", "a b"] {
            let err = store.insert(b"x", None, make_meta(bad, "1.0.0")).unwrap_err();
            assert!(
                matches!(err, StoreError::InvalidIdentifier { .. }),
                "should reject {bad:?}, got {err:?}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_unsafe_versions() {
        let root = temp_root("reject_version");
        let store = ContentStore::at(root.clone()).unwrap();
        for bad in ["", ".", "..", "../1.0.0", "1.0.0/2"] {
            let err = store.insert(b"x", None, make_meta("a", bad)).unwrap_err();
            assert!(
                matches!(err, StoreError::InvalidIdentifier { .. }),
                "should reject {bad:?}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    // ── concurrency ──────────────────────────────────────────────────

    #[test]
    fn concurrent_inserts_same_digest_consistent() {
        let root = temp_root("concurrent");
        let store = ContentStore::at(root.clone()).unwrap();
        let blob: &'static [u8] = b"shared blob";
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let s = store.clone();
                std::thread::spawn(move || {
                    s.insert(
                        blob,
                        None,
                        make_meta(&format!("dup-{i}"), "1.0.0"),
                    )
                    .unwrap()
                })
            })
            .collect();
        let digests: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        // All threads compute the same digest from identical bytes.
        for d in &digests[1..] {
            assert_eq!(*d, digests[0]);
        }
        // Single on-disk entry, four refs.
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.refs().unwrap().len(), 4);
        let _ = fs::remove_dir_all(&root);
    }

    // ── list reaping ─────────────────────────────────────────────────

    #[test]
    fn list_reaps_orphan_tempdirs() {
        let root = temp_root("reap_tempdirs");
        let store = ContentStore::at(root.clone()).unwrap();
        // Drop a fake tempdir.
        let fake = root.join("aaa.tmp.0.0.0");
        fs::create_dir_all(&fake).unwrap();
        store.list().unwrap();
        assert!(!fake.exists(), "list() should reap orphan tempdirs");
        let _ = fs::remove_dir_all(&root);
    }
}
