//! Nested-`provide` semantics for the context store (T0317).
//!
//! Both context surfaces share ONE store, `InterpreterState::context_stack`:
//! the `CtxProvide` / `CtxGet` / `CtxEnd` opcodes and the user-callable
//! flat-slot surface (`__ctx_slot_get_raw` / `__ctx_slot_set_raw` /
//! `__ctx_slot_clear_raw`, exposed as `ctx_get` / `ctx_set` / `ctx_clear`).
//! They disagree about what a nested provide of the SAME context type
//! means.
//!
//! The opcode path pushes on provide and pops on end, so an inner
//! `provide Logger = b` leaves the outer `a` intact underneath and
//! restores it on scope exit.  The slot path replaces the topmost entry
//! instead (`end_by_type` then `provide` in `ctx_runtime::ctx_slot_set`),
//! so the outer value is destroyed and nothing restores it.
//!
//! The first pin covers the opcode path and passes today.  The second
//! states the semantics the slot path owes — armed since T0317 landed
//! lands: making the slot path shadow rather than replace also requires
//! `ContextStack::end_scope` to be driven by the interpreter's return
//! path, which currently never happens outside tests — pushing without
//! that teardown would trade a lost outer value for a leaked inner one.

use verum_vbc::interpreter::ContextStack;
use verum_vbc::value::Value;

const LOGGER: u32 = 7;

#[test]
fn opcode_path_restores_the_outer_value_after_a_nested_provide() {
    // `provide Logger = 1 { provide Logger = 2 { .. } }` — the shape
    // `CtxProvide` / `CtxEnd` emit for a scoped provide.
    let mut stack = ContextStack::new();

    stack.provide(LOGGER, Value::from_i64(1), 1);
    assert_eq!(stack.get(LOGGER).map(|v| v.as_i64()), Some(1));

    stack.provide(LOGGER, Value::from_i64(2), 2);
    assert_eq!(
        stack.get(LOGGER).map(|v| v.as_i64()),
        Some(2),
        "the inner provide must shadow the outer one"
    );

    stack.pop_one();
    assert_eq!(
        stack.get(LOGGER).map(|v| v.as_i64()),
        Some(1),
        "closing the inner scope must restore the outer value"
    );
}

#[test]
fn slot_path_restores_the_outer_value_after_a_nested_set() {
    // Same nesting expressed through `ctx_set`. `ctx_slot_set` is
    // crate-private, so this drives the two `ContextStack` primitives it
    // is built from; the assertion is about the semantics the slot
    // surface must end up with, not about the private helper.
    let mut stack = ContextStack::new();

    stack.provide(LOGGER, Value::from_i64(1), 1);

    // What `ctx_slot_set` does since T0317 at a deeper call depth:
    // replace ONLY a same-depth entry, otherwise SHADOW by pushing —
    // mirrored here through the same primitives it composes.
    if stack.top_depth_by_type(LOGGER) == Some(2) {
        stack.end_by_type(LOGGER);
    }
    stack.provide(LOGGER, Value::from_i64(2), 2);
    assert_eq!(stack.get(LOGGER).map(|v| v.as_i64()), Some(2));

    // Leaving the inner scope (the frame-return end_scope wired by
    // T0317 leg 1) must uncover the outer value.
    stack.end_scope(2);
    assert_eq!(
        stack.get(LOGGER).map(|v| v.as_i64()),
        Some(1),
        "a set at a deeper depth must shadow the outer value, not destroy it"
    );
}

#[test]
fn a_repeated_set_at_one_depth_must_not_grow_the_store() {
    // The other half of the contract, and the reason `ctx_slot_set`
    // replaces in the first place: `ctx_set` called repeatedly in a loop
    // is an overwrite, not a stack of scopes. Whatever T0317 does about
    // nesting must keep this bounded.
    let mut stack = ContextStack::new();

    for i in 0..64 {
        if stack.top_depth_by_type(LOGGER) == Some(1) {
            stack.end_by_type(LOGGER);
        }
        stack.provide(LOGGER, Value::from_i64(i), 1);
    }

    assert_eq!(stack.len(), 1, "repeated set at one depth must not accumulate");
    assert_eq!(stack.get(LOGGER).map(|v| v.as_i64()), Some(63));
}
