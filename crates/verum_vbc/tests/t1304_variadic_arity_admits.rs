//! T1304 — a variadic FFI declaration must stay reachable by a call that
//! carries a tail.
//!
//! `CodegenContext` resolves functions by EXACT arity, and `FunctionInfo`
//! carries no variadic flag. So `open(path, flags, mode)` could not reach a
//! declaration written `open(path, oflag, ...)`: the extern was thrown out
//! on arity, and the call fell through to whatever else answered to the
//! name — on darwin, `core.sys.linux.syscall.open`, whose body cannot run
//! there. The program got a panic-stub instead of a file.
//!
//! `arity_admits` is the one place that decision is now made. It is pinned
//! here and not only through the end-to-end spec because it is
//! ONE-DIRECTIONAL by design, and that direction is the whole safety
//! argument: it may ADMIT a call that used to be refused, and must never
//! refuse one that used to be admitted. An exact-arity regression would be
//! invisible in a spec that only checks the variadic case passes.

//! `codegen` is not a default feature of `verum_vbc`, so without this
//! gate the file does not compile at all under the default-feature jobs.
//! The CI step that runs it is "Every gated test target"
//! (`cargo test -p verum_vbc --features codegen,ffi --tests`) — the same
//! step that was widened in 2026-09-09 after five gated files were found
//! executing in no job whatsoever.
#![cfg(feature = "codegen")]

use verum_vbc::codegen::{CodegenContext, FunctionInfo};
use verum_vbc::module::FunctionId;

/// An `extern` declaration, as `register_ffi_extern_function` builds it:
/// the `u32::MAX` sentinel says "not callable through `Call`", which is
/// exactly the property that makes a tail meaningful.
fn extern_decl(param_count: usize) -> FunctionInfo {
    FunctionInfo {
        id: FunctionId(u32::MAX),
        param_count,
        ..Default::default()
    }
}

/// An ordinary Verum function with a real id and a body.
fn bodied(param_count: usize) -> FunctionInfo {
    FunctionInfo {
        id: FunctionId(7),
        param_count,
        ..Default::default()
    }
}

/// The behaviour every name keeps when nothing variadic is registered.
#[test]
fn without_a_variadic_declaration_arity_is_matched_exactly() {
    let ctx = CodegenContext::default();
    assert!(ctx.variadic_ffi_fns.is_empty());

    assert!(ctx.arity_admits("close", &extern_decl(1), 1), "exact arity");
    assert!(!ctx.arity_admits("close", &extern_decl(1), 2), "a tail is refused");
    assert!(!ctx.arity_admits("close", &extern_decl(2), 1), "too few is refused");
    assert!(ctx.arity_admits("f", &bodied(0), 0), "the nullary case is exact");
}

/// The defect itself: a tail argument on a variadic declaration.
#[test]
fn a_variadic_declaration_admits_a_call_that_carries_a_tail() {
    let mut ctx = CodegenContext::default();
    ctx.variadic_ffi_fns.insert("open".to_string());

    assert!(ctx.arity_admits("open", &extern_decl(2), 2), "no tail is legal too");
    assert!(ctx.arity_admits("open", &extern_decl(2), 3), "one tail argument");
    assert!(ctx.arity_admits("open", &extern_decl(2), 7), "several");
}

/// Variadic does not mean "any call at all". Fewer arguments than the
/// FIXED part is a real error and stays one — `open()` with no path is not
/// a variadic call, it is a missing argument.
#[test]
fn a_variadic_declaration_still_refuses_fewer_than_its_fixed_parameters() {
    let mut ctx = CodegenContext::default();
    ctx.variadic_ffi_fns.insert("open".to_string());

    assert!(!ctx.arity_admits("open", &extern_decl(2), 1));
    assert!(!ctx.arity_admits("open", &extern_decl(2), 0));
}

/// THE CONDITION THAT KEEPS THE SET FROM LEAKING ACROSS MODULES.
///
/// The set is context-wide and a bake puts every module in one context, so
/// membership means "somewhere a variadic extern named `open` exists" — not
/// "this candidate is it". A module's own `fn open(a, b)` must NOT start
/// accepting three arguments because another module declared a variadic
/// extern of the same name.
#[test]
fn a_bodied_function_of_the_same_name_gets_no_relaxation() {
    let mut ctx = CodegenContext::default();
    ctx.variadic_ffi_fns.insert("open".to_string());

    assert!(ctx.arity_admits("open", &bodied(3), 3), "exact arity still binds");
    assert!(
        !ctx.arity_admits("open", &bodied(2), 3),
        "a real id has a body, and a body cannot read a tail"
    );
}

/// A call site inside another module spells the name
/// `core.sys.darwin.libsystem.open` while the FFI registration is keyed by
/// the bare symbol. Both spellings must find the same fact, or the
/// relaxation would work for a local call and silently not for a qualified
/// one — which is the exact shape of defect this task was.
#[test]
fn a_qualified_spelling_finds_the_same_declaration_as_the_bare_one() {
    let mut ctx = CodegenContext::default();
    ctx.variadic_ffi_fns.insert("open".to_string());

    assert!(ctx.arity_admits("core.sys.darwin.libsystem.open", &extern_decl(2), 3));
    assert!(ctx.arity_admits("darwin.libsystem.open", &extern_decl(2), 3));

    // ...and it must not decay into a suffix match on the wrong name.
    // `reopen` ends in `open` as a STRING; it is not the same symbol.
    assert!(!ctx.arity_admits("reopen", &extern_decl(2), 3));
    assert!(!ctx.arity_admits("core.sys.reopen", &extern_decl(2), 3));
}

/// The set may also be keyed by a qualified name (a module registering
/// under its own path). Both directions of the lookup are live, so both
/// are pinned.
#[test]
fn a_declaration_registered_under_a_qualified_key_is_found_verbatim() {
    let mut ctx = CodegenContext::default();
    ctx.variadic_ffi_fns
        .insert("core.sys.darwin.libsystem.snprintf".to_string());

    assert!(ctx.arity_admits("core.sys.darwin.libsystem.snprintf", &extern_decl(3), 5));
    // The bare spelling is NOT registered, so a bare call gets no
    // relaxation — stated as the deliberate asymmetry it is.
    assert!(!ctx.arity_admits("snprintf", &extern_decl(3), 5));
}
