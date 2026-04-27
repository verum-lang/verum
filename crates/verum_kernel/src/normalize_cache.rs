//! NormalizeCache — DashMap memo for [`crate::support::normalize`] /
//! [`normalize_with_axioms`] / [`normalize_with_inductives`] keyed
//! on a stable structural hash of the input term (#100, task #42).
//!
//! Mirror of `verum_smt::tactics::TacticCache` (#103) and
//! `verum_smt::capability_router::CvcStrategyCache` (#6) — same
//! `DashMap + AtomicU64 hits/misses + blake3 sig` shape so per-pass
//! telemetry is uniform across the verification stack.
//!
//! # Why caching pays off here
//!
//! `mount core.*` brings the entire stdlib (~thousands of
//! refinement-typed functions) into scope.  Type-checking each
//! function calls `normalize` repeatedly on the SAME small set of
//! constant types (`Int`, `Bool`, `Maybe<T>` for various `T`,
//! refinement predicates like `it > 0` etc.).  Without memoisation
//! every occurrence re-walks the term recursively, with substitution,
//! producing identical output each time.  Caching turns the second-
//! and-onward visits into a single `blake3 hash + DashMap.get` —
//! O(N) for the hash but with very small constant; in practice the
//! dominant cost moves to hash computation, which is ~1 GiB/s.
//!
//! # Cache key — stable structural hash
//!
//! [`StructuralHash`] is computed via `blake3` over the term's
//! `Debug` rendering.  `Debug` is `#[derive]`'d on `CoreTerm` and
//! every constituent type, so the rendering is deterministic across
//! processes and versions (modulo intentional struct evolution).
//! Same approach as `tactics::FormulaSignature` and
//! `capability_router::AssertionSetSignature`.
//!
//! # Cache invalidation
//!
//! The cache stores **pure normalization results** —
//! `normalize(t) = t'` is a function of `t` alone (the axiom-aware
//! variants take an `AxiomRegistry` snapshot, so when axioms change
//! the cache key MUST include the axiom-set fingerprint; see
//! [`AxiomAwareKey`]).  Per-session bounding via [`NormalizeCache::clear`]
//! between top-level decls keeps working-set predictable.
//!
//! # Thread safety
//!
//! `Send + Sync` so the cache sits on a verification context shared
//! across rayon workers.  `DashMap`'s sharded interior makes
//! concurrent `get`/`insert` lock-free under typical contention.

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::term::CoreTerm;

/// Stable structural signature of a `CoreTerm`.  Computed via
/// blake3 over the term's `Debug` rendering — same approach as
/// `tactics::FormulaSignature` for symmetric reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructuralHash([u8; 32]);

impl StructuralHash {
    /// Compute the hash of a `CoreTerm`.
    pub fn of(term: &CoreTerm) -> Self {
        let s = format!("{:?}", term);
        Self(blake3::hash(s.as_bytes()).into())
    }

    /// Raw 32-byte signature, useful for callers that fold it into
    /// another hash (proof-cert lineage, error-context dedup, etc.).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Composite key for the axiom-aware variant.  Carries the input
/// term hash + a fingerprint of the axiom registry that was active
/// when the cached result was computed.  Keys with different
/// axiom-fingerprints are distinct entries — δ-reduction depends on
/// the axiom set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxiomAwareKey {
    /// Term structural hash.
    pub term_hash: StructuralHash,
    /// Fingerprint of the axiom registry at compute-time.  Same
    /// blake3-of-Debug strategy.  Caller-supplied because the kernel
    /// doesn't own the registry's identity.
    pub axiom_fingerprint: [u8; 32],
}

/// Aggregate cache statistics — same shape as
/// `tactics::TacticCacheStats` for symmetric reporting.
#[derive(Debug, Clone, Copy, Default)]
pub struct NormalizeCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub hit_rate: f64,
}

/// Concurrent, sharded cache for plain (axiom-free) normalization
/// results.
///
/// Construct via [`NormalizeCache::new`] (default capacity = 16K) or
/// [`NormalizeCache::with_capacity`].  `Send + Sync` so it sits on
/// the verification context and can be shared across rayon workers.
pub struct NormalizeCache {
    entries: dashmap::DashMap<StructuralHash, CoreTerm>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for NormalizeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl NormalizeCache {
    /// Default capacity hint = 16384 entries.
    ///
    /// Each entry is the 32-byte signature + the cached `CoreTerm`
    /// (variable size, typically tens-to-hundreds of bytes for
    /// stdlib refinement predicates) + per-shard overhead.  At full
    /// load the working-set is bounded by per-session `clear`
    /// (typically called between top-level decls).
    ///
    /// 16K vs `TacticCache`'s 8K — kernel normalization touches more
    /// distinct terms than Z3 probe characterisation because the
    /// kernel walks every refinement predicate body, not just the
    /// formulas that reach the SMT solver.
    pub fn new() -> Self {
        Self::with_capacity(16384)
    }

    /// Construct with a specific capacity hint.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: dashmap::DashMap::with_capacity(cap),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Look up cached normalisation by structural hash.  Increments
    /// `hits` on Some, `misses` on None.
    pub fn get(&self, sig: &StructuralHash) -> Option<CoreTerm> {
        match self.entries.get(sig) {
            Some(entry) => {
                self.hits.fetch_add(1, AtomicOrdering::Relaxed);
                Some(entry.clone())
            }
            None => {
                self.misses.fetch_add(1, AtomicOrdering::Relaxed);
                None
            }
        }
    }

    /// Insert (or replace) the normalised term for `sig`.
    pub fn insert(&self, sig: StructuralHash, term: CoreTerm) {
        self.entries.insert(sig, term);
    }

    /// Drop all cached entries; reset stats.  Called between
    /// independent verification sessions to keep working-set
    /// bounded.
    pub fn clear(&self) {
        self.entries.clear();
        self.hits.store(0, AtomicOrdering::Relaxed);
        self.misses.store(0, AtomicOrdering::Relaxed);
    }

    /// Snapshot the current statistics.
    pub fn stats(&self) -> NormalizeCacheStats {
        let hits = self.hits.load(AtomicOrdering::Relaxed);
        let misses = self.misses.load(AtomicOrdering::Relaxed);
        let total = hits.saturating_add(misses);
        let hit_rate = if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        };
        NormalizeCacheStats {
            hits,
            misses,
            entries: self.entries.len(),
            hit_rate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{CoreTerm, UniverseLevel};

    fn sample_term() -> CoreTerm {
        CoreTerm::Universe(UniverseLevel::Concrete(0))
    }

    fn other_term() -> CoreTerm {
        CoreTerm::Universe(UniverseLevel::Concrete(1))
    }

    #[test]
    fn hash_is_stable_across_calls() {
        let t = sample_term();
        let h1 = StructuralHash::of(&t);
        let h2 = StructuralHash::of(&t);
        assert_eq!(h1, h2);
        assert_eq!(h1.as_bytes(), h2.as_bytes());
    }

    #[test]
    fn hash_differs_for_distinct_terms() {
        let h1 = StructuralHash::of(&sample_term());
        let h2 = StructuralHash::of(&other_term());
        assert_ne!(h1, h2);
    }

    #[test]
    fn cache_hit_miss_counters_track_lookups() {
        let cache = NormalizeCache::new();
        let t = sample_term();
        let sig = StructuralHash::of(&t);

        // Miss
        assert!(cache.get(&sig).is_none());
        let s0 = cache.stats();
        assert_eq!(s0.hits, 0);
        assert_eq!(s0.misses, 1);

        // Insert + hit
        cache.insert(sig, t.clone());
        let got = cache.get(&sig).expect("inserted");
        assert_eq!(got, t);
        let s1 = cache.stats();
        assert_eq!(s1.hits, 1);
        assert_eq!(s1.misses, 1);
        assert!((s1.hit_rate - 0.5).abs() < f64::EPSILON);

        // clear() resets entries + counters
        cache.clear();
        let s2 = cache.stats();
        assert_eq!(s2.hits, 0);
        assert_eq!(s2.misses, 0);
        assert_eq!(s2.entries, 0);
        assert!(cache.get(&sig).is_none());
    }
}
