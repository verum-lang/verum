//! Permission router for intrinsic gating (#12 / P3.2).
//!
//! Every `Syscall`-category intrinsic and any other intrinsic
//! tagged with [`IntrinsicHint::RequiresPermission`](crate::intrinsics::IntrinsicHint::RequiresPermission)
//! is the unconditional trust boundary of the interpreter: before
//! the bytecode reaches the corresponding handler, codegen
//! inserts a call to [`PermissionRouter::check`] keyed on
//! `(scope, target_id)`. A `Deny` short-circuits the call site
//! into a `PermissionDenied` error instead of executing the
//! syscall.
//!
//! ## Performance budget
//!
//! The warm-path target is **≤2ns** per check. The router caches
//! the most-recent `(scope, target_id, decision)` triple in a
//! single field; a repeated request for the same target hits the
//! one-entry cache with a single equality compare + branch, which
//! is ~1ns on contemporary x86_64 / aarch64 cores. The cold path
//! falls through to the user-supplied policy and back-fills the
//! one-entry cache on the way out.
//!
//! A larger backing map (currently `std::collections::HashMap`
//! keyed on `(scope, target_id)`) is reserved for the multi-loop
//! case where the one-entry cache thrashes between two callers.
//! It is consulted before the user policy and updated on every
//! decision.
//!
//! ## Default policy
//!
//! With no [`PermissionRouter::set_policy`] configured the router
//! is **allow-all**. Production deployments wire a policy
//! callback that consults a host-supplied capability table; the
//! callback is invoked *only* on cache misses, so the cost of an
//! elaborate policy lookup is amortised across the loop.
//!
//! ## Why a runtime router
//!
//! The compile-time SMT verifier already discharges most
//! capability obligations (a function annotated `using
//! [Filesystem]` that escapes the obligation gets a verifier
//! diagnostic). The runtime router exists for the residual cases
//! that the verifier cannot statically prove safe — dynamic paths
//! whose target is only known at runtime, or libraries compiled
//! against a wider-than-actual capability profile. Even when SMT
//! catches a violation up front, the runtime check is the
//! defence-in-depth backstop.

use std::collections::HashMap;

/// Coarse-grained namespace for capability checks.
///
/// Mirrors the family of [`IntrinsicCategory`](crate::intrinsics::IntrinsicCategory)
/// values that the compiler can tag with `RequiresPermission`,
/// plus broader stdlib-level scopes (file-open, socket-bind)
/// that desugar to the same router under the hood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionScope {
    /// Raw `syscallN` intrinsics.  `target_id` is the syscall
    /// number on Linux/macOS or the platform-equivalent.
    Syscall,
    /// File-system access — `open`, `unlink`, `mkdir`, etc.
    /// `target_id` is a stable hash of the resolved path.
    FileSystem,
    /// Network operations — `bind`, `connect`, `listen`.
    /// `target_id` is a stable hash of the address tuple.
    Network,
    /// Process / thread management — `fork`, `exec`, signal
    /// dispatch.  `target_id` is the operation kind.
    Process,
    /// Direct memory operations bypassing CBGR — `mmap`,
    /// `munmap`, raw `alloc`.  `target_id` is the requested
    /// region kind / size class.
    Memory,
    /// Cryptographic primitives that touch host RNG / HSM.
    /// `target_id` is the algorithm tag.
    Cryptography,
    /// Wall-clock / monotonic time.  `target_id` is the clock
    /// id.  Most observational time intrinsics are
    /// **not** gated; this scope exists for the rare
    /// privileged clock (`clock_settime`).
    Time,
}

impl PermissionScope {
    /// Stable byte encoding for wire-format use.
    ///
    /// The bytecode encoder writes `PermissionAssert::scope_tag`
    /// using these values (see `bytecode.rs::encode_instruction`).
    /// The dispatch handler decodes them via the inverse
    /// `from_wire_tag`. Both sides — and any AOT permission gate
    /// emitted by the LLVM lowerer — must agree on this mapping,
    /// so it lives on the type itself rather than as a hand-rolled
    /// match scattered across the codebase.
    pub fn to_wire_tag(self) -> u8 {
        match self {
            PermissionScope::Syscall => 0,
            PermissionScope::FileSystem => 1,
            PermissionScope::Network => 2,
            PermissionScope::Process => 3,
            PermissionScope::Memory => 4,
            PermissionScope::Cryptography => 5,
            PermissionScope::Time => 6,
        }
    }

    /// Inverse of [`to_wire_tag`]. Unknown bytes collapse to
    /// `Syscall` — the most-restricted scope — so a malformed
    /// call site errs on stronger gating, not weaker.
    pub fn from_wire_tag(tag: u8) -> Self {
        match tag {
            0 => PermissionScope::Syscall,
            1 => PermissionScope::FileSystem,
            2 => PermissionScope::Network,
            3 => PermissionScope::Process,
            4 => PermissionScope::Memory,
            5 => PermissionScope::Cryptography,
            6 => PermissionScope::Time,
            _ => PermissionScope::Syscall,
        }
    }
}

/// Opaque target identifier within a [`PermissionScope`].
///
/// The router treats it as an arbitrary `u64` — interpretation is
/// scope-specific. Codegen passes a stable hash (path, addr,
/// algorithm) so that repeated calls with the same logical
/// target hit the one-entry cache.
pub type PermissionTargetId = u64;

/// Result of routing a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Caller is permitted to invoke the gated operation.
    Allow,
    /// Caller is denied.  Codegen lowers this into a
    /// `PermissionDenied` Verum error at the call site.
    Deny,
}

/// Trait alias for the user-supplied policy callback.
///
/// The closure must be `Send + Sync` because the router is
/// shared across threads in the multi-worker scheduler hook
/// (T1-I).  It is invoked **only on cache misses**, so the cost
/// of a database lookup or RPC round-trip is amortised across
/// the surrounding hot loop.
pub type PolicyFn =
    dyn Fn(PermissionScope, PermissionTargetId) -> PermissionDecision + Send + Sync;

/// Statistics recorded by the router.
///
/// Used by the diagnostic surface (`verum audit --capabilities`)
/// and by performance benchmarks to prove the warm-path budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PermissionRouterStats {
    /// Total checks routed (cache hits + misses).
    pub total: u64,
    /// One-entry cache hits — the cheap path.
    pub last_entry_hits: u64,
    /// Backing-map cache hits (warm but not most-recent).
    pub map_hits: u64,
    /// Cache misses — policy invoked.
    pub policy_invocations: u64,
    /// Decisions denied (across all paths).
    pub denials: u64,
}

/// Cached snapshot of the most-recent request, used as the
/// warm-path lookup.
#[derive(Debug, Clone, Copy)]
struct LastEntry {
    scope: PermissionScope,
    target_id: PermissionTargetId,
    decision: PermissionDecision,
}

/// Routes intrinsic permission checks (#12 / P3.2).
///
/// Construct with [`PermissionRouter::allow_all`] for the default
/// permissive policy, or [`PermissionRouter::with_policy`] to
/// install a host-supplied callback. Either form is mutated
/// in-place by [`PermissionRouter::check`] as it back-fills the
/// caches.
pub struct PermissionRouter {
    /// One-entry warm-path cache.  Single equality compare
    /// against this field hits ≤2ns on contemporary cores.
    last: Option<LastEntry>,
    /// Backing cache for the multi-loop case where two callers
    /// alternate targets and would thrash `last`.
    map: HashMap<(PermissionScope, PermissionTargetId), PermissionDecision>,
    /// User-supplied policy.  `None` means "allow all".
    policy: Option<Box<PolicyFn>>,
    /// Diagnostic counters.
    pub stats: PermissionRouterStats,
}

impl std::fmt::Debug for PermissionRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionRouter")
            .field("last", &self.last)
            .field("map_size", &self.map.len())
            .field("policy_set", &self.policy.is_some())
            .field("stats", &self.stats)
            .finish()
    }
}

impl Default for PermissionRouter {
    fn default() -> Self {
        Self::allow_all()
    }
}

impl PermissionRouter {
    /// Construct a router that allows every check.  This is the
    /// default policy when the embedder wires nothing else; it
    /// preserves prior interpreter behaviour for code that
    /// hasn't migrated to capability-aware execution yet.
    pub fn allow_all() -> Self {
        Self {
            last: None,
            map: HashMap::new(),
            policy: None,
            stats: PermissionRouterStats::default(),
        }
    }

    /// Construct a router that consults `policy` on every cache
    /// miss.  The closure is invoked *only* on misses, so the
    /// cost of an elaborate lookup is amortised across the
    /// surrounding loop.
    pub fn with_policy<F>(policy: F) -> Self
    where
        F: Fn(PermissionScope, PermissionTargetId) -> PermissionDecision + Send + Sync + 'static,
    {
        Self {
            last: None,
            map: HashMap::new(),
            policy: Some(Box::new(policy)),
            stats: PermissionRouterStats::default(),
        }
    }

    /// Replace the policy callback in place.  The caches survive
    /// — entries decided by the old policy remain Allow/Deny
    /// even after `set_policy`.  Call [`PermissionRouter::clear_cache`]
    /// to drop them.
    pub fn set_policy<F>(&mut self, policy: F)
    where
        F: Fn(PermissionScope, PermissionTargetId) -> PermissionDecision + Send + Sync + 'static,
    {
        self.policy = Some(Box::new(policy));
    }

    /// Clear `last` and the backing map.  Used by host code
    /// that wants to re-evaluate every target after a policy
    /// switch.
    pub fn clear_cache(&mut self) {
        self.last = None;
        self.map.clear();
    }

    /// Route a single check.
    ///
    /// Path order:
    ///   1. `last` one-entry cache  → ≤2ns warm path
    ///   2. backing `map` lookup    → ~10–30ns
    ///   3. user policy callback    → cost-dominated by the
    ///      policy itself
    ///   4. allow-all fallback when no policy is wired
    ///
    /// The decision is back-filled into both caches so the next
    /// matching request hits the warm path.
    #[inline]
    pub fn check(
        &mut self,
        scope: PermissionScope,
        target_id: PermissionTargetId,
    ) -> PermissionDecision {
        self.stats.total = self.stats.total.saturating_add(1);

        // (1) one-entry warm path — ≤2ns, single compare.
        if let Some(last) = self.last
            && last.scope == scope && last.target_id == target_id {
                self.stats.last_entry_hits = self.stats.last_entry_hits.saturating_add(1);
                if last.decision == PermissionDecision::Deny {
                    self.stats.denials = self.stats.denials.saturating_add(1);
                }
                return last.decision;
            }

        // (2) backing map.
        if let Some(decision) = self.map.get(&(scope, target_id)).copied() {
            self.stats.map_hits = self.stats.map_hits.saturating_add(1);
            self.last = Some(LastEntry { scope, target_id, decision });
            if decision == PermissionDecision::Deny {
                self.stats.denials = self.stats.denials.saturating_add(1);
            }
            return decision;
        }

        // (3) / (4) cold path.
        let decision = match &self.policy {
            Some(policy) => {
                self.stats.policy_invocations =
                    self.stats.policy_invocations.saturating_add(1);
                policy(scope, target_id)
            }
            None => PermissionDecision::Allow,
        };

        self.map.insert((scope, target_id), decision);
        self.last = Some(LastEntry { scope, target_id, decision });
        if decision == PermissionDecision::Deny {
            self.stats.denials = self.stats.denials.saturating_add(1);
        }
        decision
    }

    /// `true` when the router carries a host-supplied policy.
    /// Used by diagnostic surfaces that want to distinguish
    /// "default permissive" from "production capability-gated".
    pub fn has_policy(&self) -> bool {
        self.policy.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn wire_tag_round_trip_is_lossless_for_all_scopes() {
        for scope in [
            PermissionScope::Syscall,
            PermissionScope::FileSystem,
            PermissionScope::Network,
            PermissionScope::Process,
            PermissionScope::Memory,
            PermissionScope::Cryptography,
            PermissionScope::Time,
        ] {
            let tag = scope.to_wire_tag();
            assert_eq!(
                PermissionScope::from_wire_tag(tag),
                scope,
                "round-trip failed for {scope:?} via tag {tag}"
            );
        }
    }

    #[test]
    fn unknown_wire_tag_collapses_to_syscall() {
        for bad in [7, 8, 100, 255] {
            assert_eq!(
                PermissionScope::from_wire_tag(bad),
                PermissionScope::Syscall,
                "unknown tag {bad} must collapse to Syscall"
            );
        }
    }

    #[test]
    fn allow_all_router_passes_every_request() {
        let mut router = PermissionRouter::allow_all();
        assert_eq!(
            router.check(PermissionScope::Syscall, 1),
            PermissionDecision::Allow
        );
        assert_eq!(
            router.check(PermissionScope::FileSystem, 0xDEADBEEF),
            PermissionDecision::Allow
        );
        assert_eq!(router.stats.total, 2);
        assert_eq!(router.stats.denials, 0);
        assert!(!router.has_policy());
    }

    #[test]
    fn one_entry_cache_hits_on_repeat() {
        let invocations = Arc::new(Mutex::new(0u64));
        let inv2 = invocations.clone();
        let mut router = PermissionRouter::with_policy(move |_, _| {
            *inv2.lock().unwrap() += 1;
            PermissionDecision::Allow
        });

        // First call invokes policy.
        router.check(PermissionScope::Syscall, 42);
        assert_eq!(*invocations.lock().unwrap(), 1);
        assert_eq!(router.stats.policy_invocations, 1);
        assert_eq!(router.stats.last_entry_hits, 0);

        // Repeats hit the warm path — policy is not consulted.
        for _ in 0..1_000 {
            router.check(PermissionScope::Syscall, 42);
        }
        assert_eq!(
            *invocations.lock().unwrap(),
            1,
            "policy must run only on cache miss"
        );
        assert_eq!(router.stats.last_entry_hits, 1_000);
    }

    #[test]
    fn map_serves_thrash_pattern() {
        // Two callers alternating targets — `last` thrashes
        // between them, but the backing map keeps both warm.
        let invocations = Arc::new(Mutex::new(0u64));
        let inv2 = invocations.clone();
        let mut router = PermissionRouter::with_policy(move |_, _| {
            *inv2.lock().unwrap() += 1;
            PermissionDecision::Allow
        });

        for _ in 0..100 {
            router.check(PermissionScope::Syscall, 1);
            router.check(PermissionScope::Syscall, 2);
        }

        assert_eq!(
            *invocations.lock().unwrap(),
            2,
            "policy must run exactly once per (scope, target_id)"
        );
        // 1 last-entry hit per pair (the second call to a target
        // hits the just-stored `last`); the alternating call
        // hits the map.
        assert_eq!(router.stats.policy_invocations, 2);
        assert_eq!(router.stats.map_hits + router.stats.last_entry_hits, 198);
    }

    #[test]
    fn deny_short_circuits_subsequent_checks() {
        let mut router = PermissionRouter::with_policy(|scope, target| {
            if scope == PermissionScope::Network && target == 0xBAD {
                PermissionDecision::Deny
            } else {
                PermissionDecision::Allow
            }
        });

        assert_eq!(
            router.check(PermissionScope::Network, 0xBAD),
            PermissionDecision::Deny
        );
        // Repeats hit the warm path with the cached Deny — the
        // gated operation is never reached.
        for _ in 0..50 {
            assert_eq!(
                router.check(PermissionScope::Network, 0xBAD),
                PermissionDecision::Deny
            );
        }
        assert_eq!(router.stats.denials, 51);
        assert_eq!(router.stats.policy_invocations, 1);
    }

    #[test]
    fn scope_disambiguates_target() {
        // Same target_id under two scopes must route
        // independently — callers can't bypass a denial in one
        // scope by exploiting cache collision in another.
        let mut router = PermissionRouter::with_policy(|scope, _target| match scope {
            PermissionScope::Syscall => PermissionDecision::Deny,
            _ => PermissionDecision::Allow,
        });

        assert_eq!(
            router.check(PermissionScope::Syscall, 99),
            PermissionDecision::Deny
        );
        assert_eq!(
            router.check(PermissionScope::FileSystem, 99),
            PermissionDecision::Allow
        );
        // A second Syscall(99) must still see Deny — not
        // poisoned by the FileSystem allow.
        assert_eq!(
            router.check(PermissionScope::Syscall, 99),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn clear_cache_re_invokes_policy() {
        let invocations = Arc::new(Mutex::new(0u64));
        let inv2 = invocations.clone();
        let mut router = PermissionRouter::with_policy(move |_, _| {
            *inv2.lock().unwrap() += 1;
            PermissionDecision::Allow
        });

        router.check(PermissionScope::Time, 7);
        router.check(PermissionScope::Time, 7);
        assert_eq!(*invocations.lock().unwrap(), 1);

        router.clear_cache();
        router.check(PermissionScope::Time, 7);
        assert_eq!(
            *invocations.lock().unwrap(),
            2,
            "clear_cache must force policy re-evaluation"
        );
    }

    #[test]
    fn set_policy_preserves_caches_until_clear() {
        let mut router = PermissionRouter::with_policy(|_, _| PermissionDecision::Allow);
        router.check(PermissionScope::Memory, 1);
        // Switch to a deny-all policy.
        router.set_policy(|_, _| PermissionDecision::Deny);
        // Cached Allow survives the policy swap.
        assert_eq!(
            router.check(PermissionScope::Memory, 1),
            PermissionDecision::Allow
        );
        // New target hits the new policy.
        assert_eq!(
            router.check(PermissionScope::Memory, 2),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn warm_path_under_a_million_iterations() {
        // Smoke-tests the warm path doesn't drift into the
        // policy under a tight loop.  Not a perf benchmark —
        // benches/ owns the actual ≤2ns measurement — but the
        // cache-hit invariant is verifiable here.
        let mut router = PermissionRouter::allow_all();
        for _ in 0..1_000_000 {
            router.check(PermissionScope::Syscall, 1);
        }
        assert_eq!(router.stats.total, 1_000_000);
        assert_eq!(router.stats.policy_invocations, 0);
        assert_eq!(router.stats.last_entry_hits, 999_999);
    }

    #[test]
    fn has_policy_reflects_constructor() {
        assert!(!PermissionRouter::allow_all().has_policy());
        let with =
            PermissionRouter::with_policy(|_, _| PermissionDecision::Allow);
        assert!(with.has_policy());
    }

    #[test]
    fn default_is_allow_all() {
        let router = PermissionRouter::default();
        assert!(!router.has_policy());
    }
}
