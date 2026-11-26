//! A selected panic intrinsic is synchronous; spelling alone grants no callable bound.
#[path = "support/callable_diagnostics.rs"]
mod callable_diagnostics;
use callable_diagnostics::{assert_open_callback_rejected, errors};
use verum_common::List;

#[test]
fn builtin_panic_can_terminate_a_synchronous_callback() {
    for source in [
        "fn invoke<F:fn()->Int>(f:F)->Int {f()} fn probe()->Int {invoke(||panic(\"stop\"))}",
        "static mut HANDLER:fn()->Int = ||panic(\"stop\");",
        "fn probe()->Int {let callback:fn()->Int=||panic(\"stop\"); callback()}",
    ] {
        let found = errors(source);
        assert!(found.is_empty(), "{source}: {found:?}");
    }
}

#[test]
fn the_selected_intrinsic_and_builtin_panic_have_the_same_bound() {
    let found = errors(
        "fn invoke<F:fn()->Int>(f:F)->Int {f()} fn probe()->Int {invoke(||@intrinsic(\"panic\", \"stop\"))}",
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_async_source_homonym_keeps_its_declared_bound() {
    let found = errors(
        "async fn panic(message:&Text)->Int {73} fn invoke<F:fn()->Int>(f:F)->Int {f()} fn probe()->Int {invoke(||panic(\"stop\"))}",
    );
    assert!(
        found
            .iter()
            .any(|e| e.contains("Mismatch") && e.contains("async")),
        "{found:?}"
    );
}

#[test]
fn a_shadowing_callback_cannot_borrow_the_builtin_contract() {
    let source = "fn invoke<F:fn()->Int>(f:F)->Int {f()} fn probe(panic:async fn()->Int)->Int {invoke(||panic())}";
    assert_open_callback_rejected(source, "||panic()", List::new());
}

#[test]
fn a_synchronous_panic_contract_does_not_assert_purity() {
    let found = errors("pure fn probe()->! {panic(\"stop\")}");
    assert!(
        found.iter().any(|e| e.contains("ImpurePureFunction")),
        "{found:?}"
    );
}
