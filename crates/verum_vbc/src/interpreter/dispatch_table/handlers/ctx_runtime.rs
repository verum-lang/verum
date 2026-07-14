//! CTX-STORE-AUTHORITY-1 — slot-flat view over the ONE `ContextStack`.
//!
//! `core/sys/common.vr` exposes the user-callable context-slot surface
//! (`ctx_get` / `ctx_set` / `ctx_clear`).  Under the Tier-0 interpreter
//! those functions delegate to the bodiless `@intrinsic` declarations
//! `__ctx_slot_get_raw` / `__ctx_slot_set_raw` / `__ctx_slot_clear_raw`
//! (plus the tier oracle `__ctx_store_tier0_raw`), which the by-name
//! dispatch in `calls.rs::try_dispatch_intrinsic_by_name` routes to the
//! helpers below.  The helpers operate on `state.context_stack` — the
//! SAME store the `CtxProvide` / `CtxGet` / `CtxEnd` opcode handlers
//! (`context.rs`, opcodes 0xB0-0xB2) use — so `provide`/`using` and the
//! user-callable surface can never drift.
//!
//! Semantics are slot-flat, mirroring the Tier-1 platform TLS slot
//! table (`core/sys/<os>/tls.vr`), where each slot holds exactly one
//! value:
//!   * set    — replace the topmost entry for the slot, else push one
//!   * clear  — remove EVERY entry for the slot
//!   * get    — topmost entry's value, or 0 when empty
//!
//! Key namespace: the slot id shares the u32 ctx_type namespace the
//! opcodes use — intentionally.  At Tier-1 the AOT `CtxGet` lowering
//! passes its ctx_type as the `env_ctx_get(slot_id)` argument (see
//! `verum_codegen/src/llvm/instruction.rs::lower_ctx_get`), so "slot
//! ids and ctx types are the same namespace" is the cross-tier
//! contract.
//!
//! History: this module previously held `try_intercept_ctx_runtime`, a
//! pre-gate intercept for the V-LLSI `__ctx_*_raw` / `__defer_*_raw`
//! intrinsics.  That path was superseded by the widened
//! `bytecode_length` gate + per-name match arms inside
//! `try_dispatch_intrinsic_by_name` (see the `__ctx_get_raw` /
//! `__defer_push_raw` arms in `calls.rs`), the module declaration was
//! dropped, and the file was orphaned.  The duplicate intercept has
//! been removed — `calls.rs` arms are the single dispatch authority;
//! this module now carries only the slot-store semantics (pure
//! functions, unit-tested below).

use super::super::super::state::ContextStack;
use crate::value::Value;

/// Number of context slots — MUST mirror `CONTEXT_SLOT_COUNT` in
/// `core/sys/common.vr` (and the codegen constant-table entry
/// `("CONTEXT_SLOT_COUNT", 256)` in `verum_vbc/src/codegen/mod.rs`).
pub(in super::super) const CTX_SLOT_COUNT: i64 = 256;

/// Validate a slot id and convert it to the u32 ctx_type key.
/// Out-of-range slots (negative or >= CTX_SLOT_COUNT) yield `None`
/// — defence in depth mirroring the `ctx_bridge.vr` guards.
#[inline]
fn slot_key(slot: i64) -> Option<u32> {
    if (0..CTX_SLOT_COUNT).contains(&slot) {
        Some(slot as u32)
    } else {
        None
    }
}

/// Slot read: topmost value for the slot, or 0 when empty / out of range.
pub(in super::super) fn ctx_slot_get(stack: &ContextStack, slot: i64) -> i64 {
    match slot_key(slot) {
        Some(key) => stack.get(key).map(|v| v.as_i64()).unwrap_or(0),
        None => 0,
    }
}

/// Slot overwrite: replace the topmost entry for the slot, else push a
/// new one at `stack_depth`.  Out-of-range slots are silently dropped.
pub(in super::super) fn ctx_slot_set(
    stack: &mut ContextStack,
    slot: i64,
    value: i64,
    stack_depth: usize,
) {
    if let Some(key) = slot_key(slot) {
        // Replace-topmost-else-push via the existing stack primitives —
        // no parallel storage, no new ContextStack surface required.
        stack.end_by_type(key);
        stack.provide(key, Value::from_i64(value), stack_depth);
    }
}

/// Slot clear: remove EVERY entry for the slot (flat-slot semantics —
/// after clear the slot reads as empty regardless of provide nesting,
/// matching the Tier-1 TLS `ctx_clear` which zeroes the slot outright).
pub(in super::super) fn ctx_slot_clear(stack: &mut ContextStack, slot: i64) {
    if let Some(key) = slot_key(slot) {
        while stack.end_by_type(key) {}
    }
}

// ============================================================================
// Tests — CTX-STORE-AUTHORITY-1 slot-flat semantics (pure functions)
// ============================================================================

#[cfg(test)]
mod ctx_slot_tests {
    use super::*;

    #[test]
    fn get_on_empty_slot_returns_zero() {
        let stack = ContextStack::new();
        assert_eq!(ctx_slot_get(&stack, 8), 0);
    }

    #[test]
    fn set_get_round_trip() {
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, 8, 0xBEEF, 1);
        assert_eq!(ctx_slot_get(&stack, 8), 0xBEEF);
    }

    #[test]
    fn set_overwrites_keeping_single_entry() {
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, 10, 100, 1);
        ctx_slot_set(&mut stack, 10, 200, 2);
        assert_eq!(ctx_slot_get(&stack, 10), 200);
        // Flat-slot invariant: overwrite replaced, not stacked.
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn slots_are_isolated() {
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, 11, 0xDEAD, 1);
        ctx_slot_set(&mut stack, 12, 0xBEEF, 1);
        assert_eq!(ctx_slot_get(&stack, 11), 0xDEAD);
        assert_eq!(ctx_slot_get(&stack, 12), 0xBEEF);
    }

    #[test]
    fn clear_empties_the_slot() {
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, 9, 0xCAFE, 1);
        ctx_slot_clear(&mut stack, 9);
        assert_eq!(ctx_slot_get(&stack, 9), 0);
        assert!(stack.is_empty());
    }

    #[test]
    fn clear_removes_every_entry_for_the_slot() {
        // Nested `provide` of the same key + user clear: flat-slot
        // semantics say the slot reads empty afterwards (matching the
        // Tier-1 TLS ctx_clear, which zeroes the slot outright).
        let mut stack = ContextStack::new();
        stack.provide(9, Value::from_i64(1), 1);
        stack.provide(9, Value::from_i64(2), 2);
        ctx_slot_clear(&mut stack, 9);
        assert_eq!(ctx_slot_get(&stack, 9), 0);
        assert!(stack.is_empty());
    }

    #[test]
    fn out_of_range_slots_are_guarded() {
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, -5, 0xDEAD, 1);
        ctx_slot_set(&mut stack, CTX_SLOT_COUNT, 0xDEAD, 1);
        ctx_slot_set(&mut stack, 1_000_000, 0xDEAD, 1);
        assert!(stack.is_empty());
        assert_eq!(ctx_slot_get(&stack, -5), 0);
        assert_eq!(ctx_slot_get(&stack, 1_000_000), 0);
        // Clear on garbage must not panic.
        ctx_slot_clear(&mut stack, -1);
        ctx_slot_clear(&mut stack, 1_000_000);
    }

    #[test]
    fn shares_store_with_opcode_provide() {
        // ONE-authority pin: an entry pushed by the CtxProvide opcode
        // path (ContextStack::provide) is visible to the user-callable
        // slot surface, and vice versa — same key namespace, same store.
        let mut stack = ContextStack::new();
        stack.provide(42, Value::from_i64(7), 3);
        assert_eq!(ctx_slot_get(&stack, 42), 7);

        ctx_slot_set(&mut stack, 42, 9, 3);
        assert_eq!(stack.get(42).map(|v| v.as_i64()), Some(9));
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn set_zero_reads_back_as_empty_sentinel() {
        // The .vr surface treats 0 as "empty" (`if v == 0 { Maybe.None }`),
        // mirroring the TLS table where entry.value == 0 means unset.
        let mut stack = ContextStack::new();
        ctx_slot_set(&mut stack, 8, 0, 1);
        assert_eq!(ctx_slot_get(&stack, 8), 0);
    }
}
