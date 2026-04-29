//! AOT permission policy lowering — closes the script-mode security
//! gap on the Tier-1 path.
//!
//! The Tier-0 interpreter enforces script permissions through a runtime
//! [`PermissionRouter`](verum_vbc::interpreter::permission::PermissionRouter)
//! installed by the CLI before script execution. The same source
//! compiled via `--aot` produces a native binary that lacked any
//! enforcement: every `PermissionAssert` opcode hit the catch-all
//! `Unimplemented VBC instruction` arm and either failed to build or
//! ran without checks (depending on whether the build pipeline saw it).
//!
//! This module fixes that by baking the resolved policy into the
//! generated binary at compile time. The CLI still computes the same
//! `PermissionSet`, but instead of passing only a closure to the
//! interpreter, it also serialises the grants into an
//! [`AotPermissionPolicy`] which the LLVM lowerer consumes when
//! emitting each `PermissionAssert` site.
//!
//! ## Why bake into the binary
//!
//! Three alternatives were considered:
//!
//! * A separate `verum_runtime_permissions` cdylib that the binary
//!   loads at process start — adds a build-time and link-time
//!   dependency, complicates packaging, and the env-var hand-off
//!   protocol is itself an attack surface (anyone able to set the
//!   process environment could relax the policy).
//! * A runtime helper function defined in IR that scans a global
//!   table — works but pays an indirect call on every gated
//!   intrinsic.
//! * **Compile-time inlining at every call site** (chosen). Each
//!   `PermissionAssert` becomes a small `switch` over the constant
//!   set of allowed `target_id` values. LLVM optimises the entire
//!   structure: scopes that are unconditionally allowed have **zero**
//!   runtime overhead (the assert is elided); scopes with no grants
//!   become an unconditional panic that LLVM can hoist or merge with
//!   neighbours. The policy is sealed in the binary — there is no
//!   env-var or external table to tamper with.
//!
//! ## What is enforced
//!
//! Permission grants live in
//! [`PermissionScope`](verum_vbc::interpreter::permission::PermissionScope)
//! space and are tagged via
//! [`to_wire_tag`](verum_vbc::interpreter::permission::PermissionScope::to_wire_tag).
//! The same scope-tag mapping the interpreter uses (Syscall=0,
//! FileSystem=1, Network=2, Process=3, Memory=4, Cryptography=5,
//! Time=6) is the contract this module assumes.
//!
//! Policy semantics:
//!
//! * `always_allow` — scope is passed through with no check (script
//!   policy treats Memory and Cryptography this way today).
//! * `wildcards` — any `target_id` for the scope is allowed (matches
//!   `permissions = ["net"]` shape — no specific target listed).
//! * `specific` — only the listed `(scope, target_id)` pairs allow
//!   the call site (matches `permissions = ["net=api.example.com"]`
//!   after target hashing).
//! * Anything else → deny → panic with code 143.
//!
//! Exit code 143 mirrors `SIGTERM` semantics — the binary is being
//! shut down because it stepped outside its declared capability
//! envelope, not because of a logic error in the script. Tooling can
//! distinguish capability violations from other panics by the code.

use std::collections::BTreeSet;

/// Compile-time-known permission policy baked into an AOT binary.
///
/// Constructed by the CLI from the resolved `PermissionSet` that
/// frontmatter + `--allow*` flags produce. Passed to the LLVM
/// lowerer through `LoweringConfig::permission_policy`.
///
/// `None` (the absence of this whole policy at the lowering site)
/// is the trusted-application path — no script-mode enforcement is
/// needed because the source has neither frontmatter nor CLI
/// permission flags. In that mode every `PermissionAssert` is a
/// no-op; the same opcode is what the interpreter's allow-all
/// router would also accept.
#[derive(Debug, Clone, Default)]
pub struct AotPermissionPolicy {
    /// Specific `(scope_tag, target_id)` allow entries. `scope_tag`
    /// uses the wire encoding from
    /// [`PermissionScope::to_wire_tag`](verum_vbc::interpreter::permission::PermissionScope::to_wire_tag).
    /// Empty for wildcard-only or always-allow-only policies.
    pub specific: BTreeSet<(u8, u64)>,

    /// Scope tags whose checks pass for any `target_id`. Populated
    /// by frontmatter grants without a target qualifier
    /// (`permissions = ["net"]`).
    pub wildcards: BTreeSet<u8>,

    /// Scope tags that are always allowed regardless of any other
    /// configuration. The script-mode policy seeds this with Memory
    /// and Cryptography because the script-permission vocabulary
    /// has no kind that maps to either today; future tightening can
    /// drop entries from this set without affecting the wire-format.
    pub always_allow: BTreeSet<u8>,
}

impl AotPermissionPolicy {
    /// `true` when this scope passes every check unconditionally —
    /// either in the always-allow set or carrying a wildcard grant.
    /// The lowerer elides the entire `PermissionAssert` site when
    /// this returns `true`, leaving zero runtime cost for scopes
    /// the script declared open.
    pub fn is_scope_unconditionally_allowed(&self, scope_tag: u8) -> bool {
        self.always_allow.contains(&scope_tag) || self.wildcards.contains(&scope_tag)
    }

    /// `true` when no specific or wildcard grant covers this scope
    /// and it is not in the always-allow set. The lowerer emits an
    /// unconditional panic at the call site — LLVM treats subsequent
    /// instructions as unreachable, removing the original intrinsic
    /// body entirely.
    pub fn is_scope_fully_denied(&self, scope_tag: u8) -> bool {
        !self.always_allow.contains(&scope_tag)
            && !self.wildcards.contains(&scope_tag)
            && !self.specific.iter().any(|(s, _)| *s == scope_tag)
    }

    /// Specific `target_id` grants for `scope_tag`, in deterministic
    /// order. Used by the lowerer to construct a `switch` whose case
    /// labels accept the listed targets.
    pub fn specific_targets_for_scope(&self, scope_tag: u8) -> Vec<u64> {
        self.specific
            .iter()
            .filter_map(|(s, t)| if *s == scope_tag { Some(*t) } else { None })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies_every_scope() {
        let p = AotPermissionPolicy::default();
        for scope in 0..=6_u8 {
            assert!(p.is_scope_fully_denied(scope));
            assert!(!p.is_scope_unconditionally_allowed(scope));
            assert!(p.specific_targets_for_scope(scope).is_empty());
        }
    }

    #[test]
    fn always_allow_overrides_other_state() {
        let mut p = AotPermissionPolicy::default();
        p.always_allow.insert(4); // Memory
        assert!(p.is_scope_unconditionally_allowed(4));
        assert!(!p.is_scope_fully_denied(4));
        // Other scopes still denied.
        assert!(p.is_scope_fully_denied(0));
    }

    #[test]
    fn wildcard_grant_unconditionally_allows() {
        let mut p = AotPermissionPolicy::default();
        p.wildcards.insert(2); // Network
        assert!(p.is_scope_unconditionally_allowed(2));
        assert!(!p.is_scope_fully_denied(2));
    }

    #[test]
    fn specific_grants_are_neither_unconditional_nor_denied() {
        let mut p = AotPermissionPolicy::default();
        p.specific.insert((1, 0xDEADBEEF));
        // Has a grant — not fully denied.
        assert!(!p.is_scope_fully_denied(1));
        // No wildcard — not unconditional.
        assert!(!p.is_scope_unconditionally_allowed(1));
        assert_eq!(p.specific_targets_for_scope(1), vec![0xDEADBEEF]);
    }

    #[test]
    fn specific_targets_returned_in_deterministic_order() {
        let mut p = AotPermissionPolicy::default();
        p.specific.insert((1, 30));
        p.specific.insert((1, 10));
        p.specific.insert((1, 20));
        p.specific.insert((2, 99));
        // BTreeSet iteration is sorted — order is stable across runs
        // so the lowerer's output is bit-deterministic.
        assert_eq!(p.specific_targets_for_scope(1), vec![10, 20, 30]);
        assert_eq!(p.specific_targets_for_scope(2), vec![99]);
    }
}
