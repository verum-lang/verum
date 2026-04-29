//! Per-theorem closure-hash incremental verification cache.
//!
//! ## Goal
//!
//! For corpora with hundreds of theorems (MSFS = 30, Diakrisis = 142,
//! Mathlib-class = thousands), full re-check on every save is
//! unacceptable.  This module provides closure-hash incremental
//! verification per #79's contract:
//!
//!   * Per-theorem [`ClosureFingerprint`] = blake3 over signature +
//!     proof body + transitive @framework citations + kernel version.
//!   * Cache key MUST include `verum_kernel::VVA_VERSION` so any
//!     kernel-rule edit invalidates ALL cached verdicts.
//!   * Cached `Ok` verdict + matching fingerprint ⇒ **skip** the
//!     kernel re-check entirely.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as ladder_dispatch /
//! tactic_combinator / proof_repair:
//!
//!   * [`IncrementalCacheStore`] — single dispatch interface.
//!   * [`MemoryCacheStore`] — V0 reference impl (in-process map).
//!   * [`FilesystemCacheStore`] — V0 disk-backed impl (one JSON file
//!     per theorem under `target/.verum_cache/closure-hashes/`).
//!   * Future adapters (S3-backed for distributed CI cache, see
//!     `--distributed-cache` flag plumbing in
//!     `verum_cli::commands::verify::ProfileConfig`) plug in via the
//!     same trait without touching consumer call-sites.
//!
//! ## Decision model
//!
//! Callers don't ask the cache "do I have this entry?" — they ask
//! [`decide`]: given a theorem name + its current
//! [`ClosureFingerprint`], should I skip the recheck or run it?  The
//! answer is a typed [`CacheDecision`] with a [`RecheckReason`]
//! cause attached when recheck is required.  This avoids the silent-
//! fall-through anti-pattern: every recheck is *traceable* to a
//! specific cause (no entry / hash drift / kernel-version drift /
//! previous failure).
//!
//! ## Foundation-neutral
//!
//! The module knows nothing about the kernel verification pipeline —
//! callers compute the fingerprint themselves from the elaborated
//! theorem (signature hash from CoreType, body hash from CoreTerm,
//! citations hash from the @framework attribute set).  The cache is
//! the storage + decision layer; kernel re-check is the operational
//! layer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use verum_common::Text;

/// The kernel version stamped into every cache fingerprint.  Re-export
/// so consumer crates that don't depend on `verum_kernel` directly can
/// still reach the canonical value through the verification crate.
pub use verum_kernel::VVA_VERSION as KERNEL_VERSION;

// =============================================================================
// ClosureFingerprint — the cache key
// =============================================================================

/// Composite fingerprint of a theorem's verification closure.  Two
/// theorems with the same fingerprint are operationally equivalent
/// from the kernel's standpoint — a cached Ok-verdict for one is
/// reusable for the other.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClosureFingerprint {
    /// `verum_kernel::VVA_VERSION` at the time the fingerprint was
    /// computed.  Cache entries from a different kernel version are
    /// invalidated unconditionally — any kernel-rule edit changes
    /// the trust boundary.
    pub kernel_version: Text,
    /// Blake3 of the theorem's elaborated signature (return type +
    /// hypothesis types).  Hex-encoded for storage portability.
    pub signature_hash: Text,
    /// Blake3 of the theorem's elaborated proof body / obligation
    /// term.  Hex-encoded.
    pub body_hash: Text,
    /// Blake3 of the *sorted, deduplicated* set of @framework
    /// citation strings reachable via this theorem's proof.
    /// Hex-encoded.  Sorting + dedup are mandatory: two theorems
    /// citing `[A, B]` and `[B, A]` MUST collide.
    pub citations_hash: Text,
}

impl ClosureFingerprint {
    /// Construct a fingerprint by hashing concrete payloads.
    /// Convenience wrapper: callers that already have the
    /// individual hashes can build the struct directly.
    pub fn compute(
        kernel_version: &str,
        signature: &[u8],
        body: &[u8],
        citations: &[&str],
    ) -> Self {
        let mut sorted: Vec<&str> = citations.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let citations_blob = sorted.join("\n");
        Self {
            kernel_version: Text::from(kernel_version),
            signature_hash: Text::from(hex32(blake3::hash(signature).as_bytes())),
            body_hash: Text::from(hex32(blake3::hash(body).as_bytes())),
            citations_hash: Text::from(hex32(
                blake3::hash(citations_blob.as_bytes()).as_bytes(),
            )),
        }
    }

    /// Top-level closure hash — folds all four components into a
    /// single 64-char hex string.  Used as the cache-file basename
    /// + the audit-trail identifier.
    pub fn closure_hash(&self) -> Text {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.kernel_version.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(self.signature_hash.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(self.body_hash.as_str().as_bytes());
        hasher.update(b"\n");
        hasher.update(self.citations_hash.as_str().as_bytes());
        Text::from(hex32(hasher.finalize().as_bytes()))
    }
}

fn hex32(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// =============================================================================
// CachedVerdict + CacheEntry — what we store
// =============================================================================

/// A previously-recorded verification verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CachedVerdict {
    /// Kernel accepted the theorem.
    Ok {
        /// Wall time consumed during the recorded check (ms).
        elapsed_ms: u64,
    },
    /// Kernel rejected the theorem.  Stored so the cache can short-
    /// circuit obviously-broken obligations under
    /// `--no-revert-failed` mode without re-running the kernel.
    Failed {
        /// Diagnostic reason snippet.
        reason: Text,
    },
}

impl CachedVerdict {
    pub fn is_ok(&self) -> bool {
        matches!(self, CachedVerdict::Ok { .. })
    }
}

/// One stored cache record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Stable theorem identifier (e.g. fully-qualified path).
    pub theorem_name: Text,
    /// Fingerprint when the verdict was recorded.
    pub fingerprint: ClosureFingerprint,
    /// The recorded verdict.
    pub verdict: CachedVerdict,
    /// Unix timestamp (seconds) at write time.
    pub recorded_at: u64,
}

// =============================================================================
// CacheDecision + RecheckReason — typed dispatcher output
// =============================================================================

/// What the cache says when asked about a theorem + its current
/// fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheDecision {
    /// Skip the kernel re-check; the cached entry's verdict is
    /// authoritative for this fingerprint.
    Skip { cached: CacheEntry },
    /// Run the kernel re-check; the carried [`RecheckReason`] cites
    /// the specific cause.  Callers should record the new verdict
    /// via [`IncrementalCacheStore::put`] after the recheck completes.
    Recheck { reason: RecheckReason },
}

/// Why a recheck is required.
#[derive(Debug, Clone, PartialEq)]
pub enum RecheckReason {
    /// No cache entry exists for this theorem name.
    NoCacheEntry,
    /// The cached fingerprint does not match the current one — the
    /// theorem's signature / body / citations changed.
    FingerprintMismatch {
        previous_closure_hash: Text,
        current_closure_hash: Text,
    },
    /// The cached entry was recorded under a different kernel
    /// version — the trust boundary has shifted, so the cache is
    /// invalidated unconditionally.
    KernelVersionChanged {
        cached_version: Text,
        current_version: Text,
    },
    /// The cached verdict was a failure; under default policy the
    /// failure must be re-confirmed (fingerprint may match but the
    /// kernel might have learned new rules in the meantime).
    PreviousVerdictFailed { reason: Text },
}

impl RecheckReason {
    /// Human-readable label (used by docs / CLI / metrics).
    pub fn label(&self) -> &'static str {
        match self {
            RecheckReason::NoCacheEntry => "no_cache_entry",
            RecheckReason::FingerprintMismatch { .. } => "fingerprint_mismatch",
            RecheckReason::KernelVersionChanged { .. } => "kernel_version_changed",
            RecheckReason::PreviousVerdictFailed { .. } => "previous_verdict_failed",
        }
    }
}

/// Decide whether the cache lets us skip the recheck.
///
/// Pure function — no I/O.  `current_fp.kernel_version` MUST be the
/// running kernel's `VVA_VERSION`; any drift between that and the
/// cached entry's kernel-version is treated as
/// [`RecheckReason::KernelVersionChanged`].
pub fn decide(
    store: &dyn IncrementalCacheStore,
    theorem: &str,
    current_fp: &ClosureFingerprint,
) -> CacheDecision {
    let cached = match store.get(theorem) {
        Some(e) => e,
        None => {
            return CacheDecision::Recheck {
                reason: RecheckReason::NoCacheEntry,
            }
        }
    };

    // Kernel-version drift takes precedence over fingerprint drift.
    if cached.fingerprint.kernel_version != current_fp.kernel_version {
        return CacheDecision::Recheck {
            reason: RecheckReason::KernelVersionChanged {
                cached_version: cached.fingerprint.kernel_version.clone(),
                current_version: current_fp.kernel_version.clone(),
            },
        };
    }

    // Fingerprint drift.
    let cached_hash = cached.fingerprint.closure_hash();
    let current_hash = current_fp.closure_hash();
    if cached_hash != current_hash {
        return CacheDecision::Recheck {
            reason: RecheckReason::FingerprintMismatch {
                previous_closure_hash: cached_hash,
                current_closure_hash: current_hash,
            },
        };
    }

    // Previous failure ⇒ re-confirm.
    if let CachedVerdict::Failed { reason } = &cached.verdict {
        return CacheDecision::Recheck {
            reason: RecheckReason::PreviousVerdictFailed {
                reason: reason.clone(),
            },
        };
    }

    CacheDecision::Skip { cached }
}

// =============================================================================
// cached_check — high-level orchestration helper
// =============================================================================

/// Outcome reported by [`cached_check`].  Distinguishes the cache-hit
/// path (cached verdict served verbatim) from the cache-miss path
/// (verdict freshly computed AND persisted).
#[derive(Debug, Clone, PartialEq)]
pub enum CachedCheckOutcome {
    /// The cache had a valid Ok-entry for this fingerprint; the
    /// stored verdict is authoritative for this run.
    Hit {
        cached: CacheEntry,
        /// What the decision said before serving (for telemetry).
        reason_skipped: Text,
    },
    /// The cache did not skip — the verify closure ran and the new
    /// verdict was persisted (best-effort: storage I/O failures are
    /// reported in `persist_error` but DO NOT shadow the verdict).
    Miss {
        verdict: CachedVerdict,
        /// Why the cache missed (NoCacheEntry / FingerprintMismatch /
        /// KernelVersionChanged / PreviousVerdictFailed).
        recheck_reason: RecheckReason,
        /// Storage write error, if any.  `None` means the new verdict
        /// was persisted successfully.
        persist_error: Option<Text>,
    },
}

impl CachedCheckOutcome {
    /// True iff the cache served the verdict (no kernel work was done).
    pub fn was_hit(&self) -> bool {
        matches!(self, CachedCheckOutcome::Hit { .. })
    }

    /// The verdict — either the cached one (Hit) or the freshly
    /// computed one (Miss).
    pub fn verdict(&self) -> &CachedVerdict {
        match self {
            CachedCheckOutcome::Hit { cached, .. } => &cached.verdict,
            CachedCheckOutcome::Miss { verdict, .. } => verdict,
        }
    }
}

/// Cache-aware verification orchestration.
///
/// Glues `decide` + the user's verify closure + `put` into a single
/// call.  The verify closure is invoked **only when the cache misses**
/// (NoCacheEntry / FingerprintMismatch / KernelVersionChanged /
/// PreviousVerdictFailed).  On miss, the freshly-computed verdict is
/// persisted to the store before returning.
///
/// This is the pipeline-side entry point: production callers
/// (`verum_compiler::pipeline::verify_theorem_proofs`) hand the cache
/// + a closure that runs the actual kernel re-check, and get back a
/// typed [`CachedCheckOutcome`].  Hit → skip the kernel call entirely;
/// Miss → the kernel ran and the result is now cached for next time.
///
/// **Persist failures do not poison the verdict**: a cache write
/// error is recorded in `Miss::persist_error` but the verify-closure's
/// verdict is still returned authoritatively.  This matches the
/// "cache is optimisation, not source of truth" contract.
pub fn cached_check<F>(
    store: &dyn IncrementalCacheStore,
    theorem_name: &str,
    fingerprint: &ClosureFingerprint,
    verify: F,
) -> CachedCheckOutcome
where
    F: FnOnce() -> CachedVerdict,
{
    match decide(store, theorem_name, fingerprint) {
        CacheDecision::Skip { cached } => CachedCheckOutcome::Hit {
            cached,
            reason_skipped: Text::from("cache-hit"),
        },
        CacheDecision::Recheck { reason } => {
            let verdict = verify();
            let entry = CacheEntry {
                theorem_name: Text::from(theorem_name),
                fingerprint: fingerprint.clone(),
                verdict: verdict.clone(),
                recorded_at: now_secs(),
            };
            let persist_error = match store.put(&entry) {
                Ok(()) => None,
                Err(e) => Some(Text::from(format!("{}", e))),
            };
            CachedCheckOutcome::Miss {
                verdict,
                recheck_reason: reason,
                persist_error,
            }
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =============================================================================
// IncrementalCacheStore — the trait boundary
// =============================================================================

/// Per-theorem cache store.  Implementations may be in-memory
/// (test / playbook) or disk-backed (production), and may layer
/// (in-memory L1 in front of disk L2).
///
/// Contract:
///
///   * `get(name)` returns `Some(entry)` only for previously-`put`
///     names; never fabricates entries.
///   * `put(entry)` overwrites any existing entry for the same
///     `theorem_name` (last writer wins).
///   * `clear()` removes every entry; returns the count cleared.
///   * `stats()` reflects the current state of the store.
///
/// Errors are reported via [`CacheError`] — disk I/O failures
/// surface here rather than panicking; callers can choose to
/// degrade to "always recheck" when storage is unavailable.
pub trait IncrementalCacheStore: std::fmt::Debug + Send + Sync {
    fn get(&self, theorem_name: &str) -> Option<CacheEntry>;
    fn put(&self, entry: &CacheEntry) -> Result<(), CacheError>;
    fn clear(&self) -> Result<usize, CacheError>;
    fn stats(&self) -> CacheStats;
    /// Enumerate the theorem names currently cached.  Used by `verum
    /// cache-closure list`.
    fn names(&self) -> Vec<Text>;
}

/// Cache-store error.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheError {
    /// Underlying storage I/O failure.
    Io(Text),
    /// Cache file could not be parsed (corrupted or older schema).
    Parse(Text),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CacheError::Io(t) => write!(f, "cache I/O error: {}", t.as_str()),
            CacheError::Parse(t) => write!(f, "cache parse error: {}", t.as_str()),
        }
    }
}

impl std::error::Error for CacheError {}

/// Aggregate statistics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheStats {
    /// Number of stored entries.
    pub entries: usize,
    /// Number of successful `decide → Skip` calls (live counter,
    /// reset on clear).
    pub hits: u64,
    /// Number of `decide → Recheck` calls (live counter, reset on
    /// clear).
    pub misses: u64,
    /// On-disk size in bytes (0 for memory-only stores).
    pub size_bytes: u64,
}

impl CacheStats {
    /// Hit ratio in [0, 1].  Returns 0.0 when there are no decisions yet.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f64 / total as f64
    }
}

// =============================================================================
// MemoryCacheStore — V0 in-memory reference
// =============================================================================

/// In-memory store.  Used by tests and the LSP-backed playbook (no
/// disk persistence needed for live editing).
#[derive(Debug, Default)]
pub struct MemoryCacheStore {
    entries: Mutex<BTreeMap<Text, CacheEntry>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl MemoryCacheStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a hit (call from a higher-level wrapper that drives
    /// `decide`).  Exposed so disjoint store impls share the
    /// same telemetry shape.
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, AtomicOrdering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

impl IncrementalCacheStore for MemoryCacheStore {
    fn get(&self, theorem_name: &str) -> Option<CacheEntry> {
        self.entries
            .lock()
            .ok()?
            .get(theorem_name)
            .cloned()
    }

    fn put(&self, entry: &CacheEntry) -> Result<(), CacheError> {
        let mut map = self
            .entries
            .lock()
            .map_err(|_| CacheError::Io(Text::from("memory store mutex poisoned")))?;
        map.insert(entry.theorem_name.clone(), entry.clone());
        Ok(())
    }

    fn clear(&self) -> Result<usize, CacheError> {
        let mut map = self
            .entries
            .lock()
            .map_err(|_| CacheError::Io(Text::from("memory store mutex poisoned")))?;
        let n = map.len();
        map.clear();
        self.hits.store(0, AtomicOrdering::Relaxed);
        self.misses.store(0, AtomicOrdering::Relaxed);
        Ok(n)
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.lock().map(|m| m.len()).unwrap_or(0),
            hits: self.hits.load(AtomicOrdering::Relaxed),
            misses: self.misses.load(AtomicOrdering::Relaxed),
            size_bytes: 0,
        }
    }

    fn names(&self) -> Vec<Text> {
        self.entries
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

// =============================================================================
// FilesystemCacheStore — V0 disk-backed reference
// =============================================================================

/// Disk-backed store.  One JSON file per theorem under
/// `<root>/<sanitized_name>.json`.  The root is typically
/// `target/.verum_cache/closure-hashes/`.
#[derive(Debug)]
pub struct FilesystemCacheStore {
    root: PathBuf,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl FilesystemCacheStore {
    /// Construct a store rooted at `root`, creating the directory if
    /// it doesn't exist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, CacheError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| {
            CacheError::Io(Text::from(format!(
                "creating cache root {}: {}",
                root.display(),
                e
            )))
        })?;
        Ok(Self {
            root,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    /// Map a theorem name to the canonical cache-file path.  The
    /// name is sanitised to a filesystem-safe form (alphanumeric +
    /// dot + dash + underscore preserved; everything else replaced
    /// with `_`) so collision-resistant blake3 of the original name
    /// is appended to keep the mapping injective.
    fn path_for(&self, theorem_name: &str) -> PathBuf {
        let sanitized: String = theorem_name
            .chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
                _ => '_',
            })
            .collect();
        let suffix = hex32(blake3::hash(theorem_name.as_bytes()).as_bytes());
        // Truncate to 8 chars for readability — blake3's collision
        // resistance at 8 chars is still ~32 bits, which is enough
        // for theorem-name-disambiguation in practice.
        let short = &suffix[..8];
        self.root.join(format!("{}-{}.json", sanitized, short))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn record_hit(&self) {
        self.hits.fetch_add(1, AtomicOrdering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, AtomicOrdering::Relaxed);
    }

    fn dir_size(&self) -> u64 {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            Err(_) => return 0,
        };
        let mut total = 0u64;
        for e in entries.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        total
    }
}

impl IncrementalCacheStore for FilesystemCacheStore {
    fn get(&self, theorem_name: &str) -> Option<CacheEntry> {
        let path = self.path_for(theorem_name);
        let raw = std::fs::read(&path).ok()?;
        // Parse failures are silently treated as "no entry" — this
        // is the correct behaviour during schema migration: the
        // caller will recheck and overwrite the bad file.
        serde_json::from_slice::<CacheEntry>(&raw).ok()
    }

    fn put(&self, entry: &CacheEntry) -> Result<(), CacheError> {
        let path = self.path_for(entry.theorem_name.as_str());
        let serialized = serde_json::to_vec_pretty(entry)
            .map_err(|e| CacheError::Parse(Text::from(format!("serialize: {}", e))))?;
        std::fs::write(&path, serialized).map_err(|e| {
            CacheError::Io(Text::from(format!(
                "writing {}: {}",
                path.display(),
                e
            )))
        })
    }

    fn clear(&self) -> Result<usize, CacheError> {
        let entries = std::fs::read_dir(&self.root).map_err(|e| {
            CacheError::Io(Text::from(format!(
                "listing {}: {}",
                self.root.display(),
                e
            )))
        })?;
        let mut n = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "json") {
                std::fs::remove_file(&path).map_err(|e| {
                    CacheError::Io(Text::from(format!(
                        "removing {}: {}",
                        path.display(),
                        e
                    )))
                })?;
                n += 1;
            }
        }
        self.hits.store(0, AtomicOrdering::Relaxed);
        self.misses.store(0, AtomicOrdering::Relaxed);
        Ok(n)
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.names().len(),
            hits: self.hits.load(AtomicOrdering::Relaxed),
            misses: self.misses.load(AtomicOrdering::Relaxed),
            size_bytes: self.dir_size(),
        }
    }

    fn names(&self) -> Vec<Text> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<Text> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e != "json") {
                continue;
            }
            if let Ok(raw) = std::fs::read(&path) {
                if let Ok(e) = serde_json::from_slice::<CacheEntry>(&raw) {
                    out.push(e.theorem_name);
                }
            }
        }
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn fp_v(version: &str) -> ClosureFingerprint {
        ClosureFingerprint::compute(version, b"sig", b"body", &["framework_a"])
    }

    fn entry_ok(name: &str, fp: ClosureFingerprint) -> CacheEntry {
        CacheEntry {
            theorem_name: Text::from(name),
            fingerprint: fp,
            verdict: CachedVerdict::Ok { elapsed_ms: 42 },
            recorded_at: now_secs(),
        }
    }

    // ----- ClosureFingerprint -----

    #[test]
    fn fingerprint_is_deterministic() {
        let a = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c1", "c2"]);
        let b = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c1", "c2"]);
        assert_eq!(a, b);
        assert_eq!(a.closure_hash(), b.closure_hash());
    }

    #[test]
    fn fingerprint_citations_sort_and_dedup() {
        // Same citation set in different order ⇒ same fingerprint.
        let a = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c1", "c2"]);
        let b = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c2", "c1"]);
        assert_eq!(a.citations_hash, b.citations_hash);
        // Duplicates are collapsed.
        let c = ClosureFingerprint::compute(
            "2.6.0",
            b"sig",
            b"body",
            &["c1", "c2", "c1", "c2"],
        );
        assert_eq!(a.citations_hash, c.citations_hash);
    }

    #[test]
    fn fingerprint_changes_with_each_component() {
        let base = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["c"]);
        let v_diff = ClosureFingerprint::compute("2.7.0", b"sig", b"body", &["c"]);
        let s_diff = ClosureFingerprint::compute("2.6.0", b"sig2", b"body", &["c"]);
        let b_diff = ClosureFingerprint::compute("2.6.0", b"sig", b"body2", &["c"]);
        let c_diff = ClosureFingerprint::compute("2.6.0", b"sig", b"body", &["d"]);

        assert_ne!(base.closure_hash(), v_diff.closure_hash());
        assert_ne!(base.closure_hash(), s_diff.closure_hash());
        assert_ne!(base.closure_hash(), b_diff.closure_hash());
        assert_ne!(base.closure_hash(), c_diff.closure_hash());
    }

    #[test]
    fn closure_hash_is_64_chars_hex() {
        let h = ClosureFingerprint::compute("2.6.0", b"x", b"y", &["z"]).closure_hash();
        assert_eq!(h.as_str().len(), 64);
        assert!(h.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ----- CacheEntry roundtrip -----

    #[test]
    fn cache_entry_serde_roundtrip() {
        let e = entry_ok("thm.foo", fp_v("2.6.0"));
        let json = serde_json::to_string(&e).unwrap();
        let back: CacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    // ----- decide -----

    #[test]
    fn decide_no_entry_says_recheck_no_cache() {
        let store = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        match decide(&store, "thm.x", &fp) {
            CacheDecision::Recheck {
                reason: RecheckReason::NoCacheEntry,
            } => {}
            other => panic!("expected NoCacheEntry, got {:?}", other),
        }
    }

    #[test]
    fn decide_matching_fp_ok_says_skip() {
        let store = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        store.put(&entry_ok("thm.x", fp.clone())).unwrap();
        match decide(&store, "thm.x", &fp) {
            CacheDecision::Skip { cached } => {
                assert_eq!(cached.theorem_name.as_str(), "thm.x");
                assert!(cached.verdict.is_ok());
            }
            other => panic!("expected Skip, got {:?}", other),
        }
    }

    #[test]
    fn decide_kernel_version_drift_takes_precedence() {
        let store = MemoryCacheStore::new();
        let cached_fp = fp_v("2.6.0");
        store.put(&entry_ok("thm.x", cached_fp)).unwrap();
        // Same payloads, different kernel version.
        let current_fp = fp_v("2.7.0");
        match decide(&store, "thm.x", &current_fp) {
            CacheDecision::Recheck {
                reason: RecheckReason::KernelVersionChanged {
                    cached_version,
                    current_version,
                },
            } => {
                assert_eq!(cached_version.as_str(), "2.6.0");
                assert_eq!(current_version.as_str(), "2.7.0");
            }
            other => panic!("expected KernelVersionChanged, got {:?}", other),
        }
    }

    #[test]
    fn decide_fingerprint_mismatch_when_body_changes() {
        let store = MemoryCacheStore::new();
        let cached_fp =
            ClosureFingerprint::compute("2.6.0", b"sig", b"body_v1", &["c"]);
        store.put(&entry_ok("thm.x", cached_fp)).unwrap();
        let current_fp =
            ClosureFingerprint::compute("2.6.0", b"sig", b"body_v2", &["c"]);
        match decide(&store, "thm.x", &current_fp) {
            CacheDecision::Recheck {
                reason:
                    RecheckReason::FingerprintMismatch {
                        previous_closure_hash,
                        current_closure_hash,
                    },
            } => {
                assert_ne!(previous_closure_hash, current_closure_hash);
            }
            other => panic!("expected FingerprintMismatch, got {:?}", other),
        }
    }

    #[test]
    fn decide_previous_failure_forces_recheck_even_on_match() {
        let store = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        store
            .put(&CacheEntry {
                theorem_name: Text::from("thm.x"),
                fingerprint: fp.clone(),
                verdict: CachedVerdict::Failed {
                    reason: Text::from("z3 timeout"),
                },
                recorded_at: now_secs(),
            })
            .unwrap();
        match decide(&store, "thm.x", &fp) {
            CacheDecision::Recheck {
                reason: RecheckReason::PreviousVerdictFailed { reason },
            } => assert!(reason.as_str().contains("timeout")),
            other => panic!("expected PreviousVerdictFailed, got {:?}", other),
        }
    }

    #[test]
    fn recheck_reason_label_is_stable() {
        assert_eq!(RecheckReason::NoCacheEntry.label(), "no_cache_entry");
        assert_eq!(
            RecheckReason::FingerprintMismatch {
                previous_closure_hash: Text::from("a"),
                current_closure_hash: Text::from("b"),
            }
            .label(),
            "fingerprint_mismatch"
        );
        assert_eq!(
            RecheckReason::KernelVersionChanged {
                cached_version: Text::from("a"),
                current_version: Text::from("b"),
            }
            .label(),
            "kernel_version_changed"
        );
        assert_eq!(
            RecheckReason::PreviousVerdictFailed {
                reason: Text::from("x"),
            }
            .label(),
            "previous_verdict_failed"
        );
    }

    // ----- MemoryCacheStore -----

    #[test]
    fn memory_store_put_get_round_trips() {
        let s = MemoryCacheStore::new();
        let e = entry_ok("thm.foo", fp_v("2.6.0"));
        s.put(&e).unwrap();
        assert_eq!(s.get("thm.foo"), Some(e));
    }

    #[test]
    fn memory_store_clear_removes_everything() {
        let s = MemoryCacheStore::new();
        s.put(&entry_ok("a", fp_v("2.6.0"))).unwrap();
        s.put(&entry_ok("b", fp_v("2.6.0"))).unwrap();
        assert_eq!(s.stats().entries, 2);
        let n = s.clear().unwrap();
        assert_eq!(n, 2);
        assert_eq!(s.stats().entries, 0);
        assert_eq!(s.stats().hits, 0);
        assert_eq!(s.stats().misses, 0);
    }

    #[test]
    fn memory_store_names_sorted() {
        let s = MemoryCacheStore::new();
        s.put(&entry_ok("c", fp_v("2.6.0"))).unwrap();
        s.put(&entry_ok("a", fp_v("2.6.0"))).unwrap();
        s.put(&entry_ok("b", fp_v("2.6.0"))).unwrap();
        let names = s.names();
        // BTreeMap iteration order is sorted.
        assert_eq!(
            names.iter().map(|t| t.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn memory_store_hit_miss_counters() {
        let s = MemoryCacheStore::new();
        s.record_hit();
        s.record_hit();
        s.record_miss();
        let st = s.stats();
        assert_eq!(st.hits, 2);
        assert_eq!(st.misses, 1);
        assert!((st.hit_ratio() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn cache_stats_hit_ratio_zero_division_guard() {
        let s = CacheStats::default();
        assert_eq!(s.hit_ratio(), 0.0);
    }

    // ----- FilesystemCacheStore -----

    #[test]
    fn fs_store_put_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        let e = entry_ok("thm.qualified.name", fp_v("2.6.0"));
        s.put(&e).unwrap();
        let back = s.get("thm.qualified.name").unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn fs_store_path_sanitises_special_chars() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        // Names with `:` `/` `<` etc must produce filesystem-safe paths.
        let p = s.path_for("module::Type<Int>::method");
        let basename = p.file_name().unwrap().to_string_lossy();
        for c in basename.chars() {
            assert!(
                c.is_alphanumeric() || c == '.' || c == '-' || c == '_',
                "unsafe char in `{}`",
                basename
            );
        }
    }

    #[test]
    fn fs_store_path_is_injective_for_distinct_names() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        let p1 = s.path_for("module::Foo::bar");
        let p2 = s.path_for("module/Foo/bar");
        // Different names → different files (suffix differs even
        // when sanitised stem collides).
        assert_ne!(p1, p2);
    }

    #[test]
    fn fs_store_clear_removes_files() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        s.put(&entry_ok("a", fp_v("2.6.0"))).unwrap();
        s.put(&entry_ok("b", fp_v("2.6.0"))).unwrap();
        let n = s.clear().unwrap();
        assert_eq!(n, 2);
        assert!(s.get("a").is_none());
        assert!(s.get("b").is_none());
    }

    #[test]
    fn fs_store_corrupt_file_treated_as_no_entry() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        let path = s.path_for("thm.x");
        std::fs::write(&path, b"not valid JSON {{{").unwrap();
        // get() returns None on parse failure (degrade gracefully).
        assert!(s.get("thm.x").is_none());
    }

    #[test]
    fn fs_store_names_returns_only_valid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        s.put(&entry_ok("good", fp_v("2.6.0"))).unwrap();
        // Drop a stray file in the cache dir.
        std::fs::write(dir.path().join("garbage.json"), b"{").unwrap();
        let names = s.names();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].as_str(), "good");
    }

    #[test]
    fn fs_store_stats_tracks_size() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        let pre = s.stats().size_bytes;
        s.put(&entry_ok("a", fp_v("2.6.0"))).unwrap();
        let post = s.stats().size_bytes;
        assert!(post > pre, "size_bytes must grow after put: {} → {}", pre, post);
    }

    // ----- decide + FilesystemCacheStore end-to-end -----

    #[test]
    fn decide_works_against_filesystem_store() {
        let dir = tempfile::tempdir().unwrap();
        let s = FilesystemCacheStore::new(dir.path()).unwrap();
        let fp = fp_v("2.6.0");
        s.put(&entry_ok("thm.x", fp.clone())).unwrap();
        assert!(matches!(decide(&s, "thm.x", &fp), CacheDecision::Skip { .. }));
        let new_fp = fp_v("2.7.0");
        assert!(matches!(
            decide(&s, "thm.x", &new_fp),
            CacheDecision::Recheck {
                reason: RecheckReason::KernelVersionChanged { .. }
            }
        ));
    }

    // ----- Coverage of #79 acceptance criteria -----

    #[test]
    fn task_79_kernel_version_invalidates_cache() {
        // #79 §3: any kernel-rule edit invalidates ALL caches.
        // VVA_VERSION-based fingerprinting + KernelVersionChanged
        // reason is the mechanism.
        let s = MemoryCacheStore::new();
        let fp_old = ClosureFingerprint::compute("2.6.0", b"s", b"b", &["c"]);
        let fp_new = ClosureFingerprint::compute("2.7.0", b"s", b"b", &["c"]);
        s.put(&entry_ok("thm", fp_old)).unwrap();
        assert!(matches!(
            decide(&s, "thm", &fp_new),
            CacheDecision::Recheck {
                reason: RecheckReason::KernelVersionChanged { .. }
            }
        ));
    }

    // ----- cached_check helper -----

    #[test]
    fn cached_check_first_call_runs_verify_closure() {
        let s = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        let calls = std::cell::Cell::new(0);
        let outcome = cached_check(&s, "thm.x", &fp, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 7 }
        });
        assert_eq!(calls.get(), 1, "verify closure must run on miss");
        assert!(!outcome.was_hit());
        match &outcome {
            CachedCheckOutcome::Miss {
                recheck_reason,
                verdict,
                persist_error,
            } => {
                assert!(matches!(recheck_reason, RecheckReason::NoCacheEntry));
                assert!(verdict.is_ok());
                assert!(persist_error.is_none());
            }
            _ => panic!("expected Miss, got {:?}", outcome),
        }
        // Verdict is now persisted.
        assert!(s.get("thm.x").is_some());
    }

    #[test]
    fn cached_check_second_call_serves_from_cache() {
        let s = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        let calls = std::cell::Cell::new(0);
        // Warm the cache.
        cached_check(&s, "thm.x", &fp, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 7 }
        });
        // Second call must hit cache.
        let outcome = cached_check(&s, "thm.x", &fp, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 99 }
        });
        assert_eq!(calls.get(), 1, "verify closure must NOT run on hit");
        assert!(outcome.was_hit());
        // The hit's verdict is the *cached* one (elapsed_ms=7), not
        // whatever the (un-run) closure would have returned.
        if let CachedVerdict::Ok { elapsed_ms } = outcome.verdict() {
            assert_eq!(*elapsed_ms, 7);
        } else {
            panic!("expected Ok verdict");
        }
    }

    #[test]
    fn cached_check_kernel_version_change_triggers_rerun() {
        let s = MemoryCacheStore::new();
        let fp_old = fp_v("2.6.0");
        let calls = std::cell::Cell::new(0);
        cached_check(&s, "thm.x", &fp_old, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 1 }
        });
        // Bump kernel version → must miss with KernelVersionChanged.
        let fp_new = fp_v("2.7.0");
        let outcome = cached_check(&s, "thm.x", &fp_new, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 2 }
        });
        assert_eq!(calls.get(), 2, "kernel-version drift must re-run");
        match outcome {
            CachedCheckOutcome::Miss {
                recheck_reason: RecheckReason::KernelVersionChanged { .. },
                ..
            } => {}
            other => panic!("expected KernelVersionChanged, got {:?}", other),
        }
    }

    #[test]
    fn cached_check_persists_failed_verdict_so_subsequent_calls_skip_kernel_when_unchanged() {
        // Failed verdicts MUST trigger a re-run on the next call (the
        // PreviousVerdictFailed reason).  Pin that contract through
        // the helper.
        let s = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        let calls = std::cell::Cell::new(0);
        cached_check(&s, "thm.x", &fp, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Failed {
                reason: Text::from("z3 timeout"),
            }
        });
        let outcome = cached_check(&s, "thm.x", &fp, || {
            calls.set(calls.get() + 1);
            CachedVerdict::Ok { elapsed_ms: 1 }
        });
        assert_eq!(
            calls.get(),
            2,
            "failed cached verdict must re-run on next call"
        );
        match outcome {
            CachedCheckOutcome::Miss {
                recheck_reason: RecheckReason::PreviousVerdictFailed { .. },
                ..
            } => {}
            other => panic!("expected PreviousVerdictFailed miss, got {:?}", other),
        }
    }

    #[test]
    fn cached_check_persist_error_does_not_poison_verdict() {
        // Cache backend that always fails on put.
        #[derive(Debug)]
        struct FailingStore;
        impl IncrementalCacheStore for FailingStore {
            fn get(&self, _: &str) -> Option<CacheEntry> {
                None
            }
            fn put(&self, _: &CacheEntry) -> Result<(), CacheError> {
                Err(CacheError::Io(Text::from("disk full")))
            }
            fn clear(&self) -> Result<usize, CacheError> {
                Ok(0)
            }
            fn stats(&self) -> CacheStats {
                CacheStats::default()
            }
            fn names(&self) -> Vec<Text> {
                Vec::new()
            }
        }
        let s = FailingStore;
        let fp = fp_v("2.6.0");
        let outcome = cached_check(&s, "thm.x", &fp, || CachedVerdict::Ok {
            elapsed_ms: 5,
        });
        match outcome {
            CachedCheckOutcome::Miss {
                verdict,
                persist_error,
                ..
            } => {
                // Verdict survives — caller still gets the truth.
                assert!(verdict.is_ok());
                // Persist error is reported.
                let e = persist_error.expect("persist_error must be set");
                assert!(e.as_str().contains("disk full"));
            }
            other => panic!("expected Miss with persist_error, got {:?}", other),
        }
    }

    #[test]
    fn task_79_skip_only_for_ok_verdicts() {
        // #79 §2: skip-mode only fires when the cached verdict was
        // OK.  PreviousVerdictFailed surfaces the reason.
        let s = MemoryCacheStore::new();
        let fp = fp_v("2.6.0");
        s.put(&CacheEntry {
            theorem_name: Text::from("thm"),
            fingerprint: fp.clone(),
            verdict: CachedVerdict::Failed {
                reason: Text::from("kernel rejected"),
            },
            recorded_at: 0,
        })
        .unwrap();
        assert!(matches!(
            decide(&s, "thm", &fp),
            CacheDecision::Recheck {
                reason: RecheckReason::PreviousVerdictFailed { .. }
            }
        ));
    }
}
