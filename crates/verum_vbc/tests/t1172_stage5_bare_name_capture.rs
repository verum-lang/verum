//! T1172 — a stage-5 mount-miss stub must not be recorded as an
//! EXPLICIT MOUNT.
//!
//! `compile_call`'s mount-miss path (`codegen/expressions.rs`, the
//! `stage5_pending` branch) mints a placeholder `FunctionInfo` and binds
//! it to both the mount's qualified spelling and the bare local alias.
//! The bare binding used to go through `register_function_authoritative`,
//! which is the registration a real `mount` uses. Its side effect that
//! matters is `explicit_mount_names`: membership there makes the
//! INTRINSIC-MOUNT-COLLISION rule in `resolve_qualified_overload` prefer
//! the bare-slot binding "over an unmounted same-name function selected
//! by arg-type overload". A placeholder minted because a mount MISSED
//! must never be able to present itself as the user's explicit choice.
//!
//! WHAT THIS DOES NOT FIX, PINNED BELOW SO NOBODY CREDITS IT WITH MORE
//! THAN IT DOES: `<name>#<arity>` still ends up on the stub. The
//! authoritative form writes that key directly; the plain form arrives
//! at the same place through `register_function`'s own collision branch,
//! which demotes a displaced entry to `<name>#<its arity>`. So the
//! arity-fallback failure — declare `open` variadic, the real call's
//! arity drops from 3 to 2, `lookup_function_with_arity` falls to
//! `open#3` and finds the stub — is NOT addressed by the plain
//! registration. Both polarities are asserted here for exactly that
//! reason: a test that only checked the column that moves would read as
//! a cure for the column that does not.
#![cfg(feature = "codegen")]

use verum_vbc::codegen::{CodegenContext, FunctionInfo};
use verum_vbc::module::FunctionId;
use verum_vbc::stub_ranges::{STAGE5_BASE, in_stage5};

const QUALIFIED: &str = "core.sys.darwin.libsystem.open";
const REAL_ID: u32 = 4242;

fn stub(arity: usize) -> FunctionInfo {
    // Stub shape as the mint site builds it: no return type, synthetic
    // parameter names, arity taken from the CALL SITE.
    FunctionInfo {
        id: FunctionId(STAGE5_BASE),
        param_count: arity,
        param_names: (0..arity).map(|i| format!("_arg{}", i)).collect(),
        ..Default::default()
    }
}

fn real(arity: usize) -> FunctionInfo {
    FunctionInfo {
        id: FunctionId(REAL_ID),
        param_count: arity,
        param_names: (0..arity).map(|i| format!("p{}", i)).collect(),
        return_type_name: Some("Int".to_string()),
        ..Default::default()
    }
}

struct Outcome {
    bare: Option<u32>,
    arity3: Option<u32>,
    arity2: Option<u32>,
    explicit_mount: bool,
}

/// The bake's sequence: the stub is minted first (its module compiles
/// before the producing one), then the real declaration arrives.
/// `authoritative` selects the pre-fix bare registration.
fn register_stub_then_real(authoritative: bool) -> Outcome {
    let mut ctx = CodegenContext::new();
    let s = stub(3);

    ctx.register_function(QUALIFIED.to_string(), s.clone());
    if authoritative {
        ctx.register_function_authoritative("open".to_string(), s);
    } else {
        ctx.register_function("open".to_string(), s);
    }

    // verum-35's trigger: `open` declared variadic, so its DECLARED
    // arity is 2 while the real call sites pass 3.
    let r = real(2);
    ctx.register_function(QUALIFIED.to_string(), r.clone());
    ctx.register_function("open".to_string(), r);

    Outcome {
        bare: ctx.lookup_function("open").map(|f| f.id.0),
        arity3: ctx.lookup_function_with_arity("open", 3).map(|f| f.id.0),
        arity2: ctx.lookup_function_with_arity("open", 2).map(|f| f.id.0),
        explicit_mount: ctx.explicit_mount_names.contains("open"),
    }
}

/// The invariant the fix establishes.
#[test]
fn a_mount_miss_stub_is_not_an_explicit_mount() {
    let post = register_stub_then_real(false);
    assert!(
        !post.explicit_mount,
        "a stage-5 placeholder must not enter `explicit_mount_names` — \
         membership makes `mount_preferred` pick it over arg-type overload \
         ranking, i.e. a missed mount would outrank a real function"
    );
}

/// The authoritative form is what grants that privilege — kept so the
/// assertion above is a statement about a CHOICE, not about an
/// unreachable state.
#[test]
fn the_authoritative_form_is_what_grants_mount_privilege() {
    let pre = register_stub_then_real(true);
    assert!(
        pre.explicit_mount,
        "register_function_authoritative is expected to record an explicit \
         mount; if this ever stops being true the fix at the stage-5 mint \
         site is measuring nothing and should be re-derived"
    );
}

/// Measured 2026-09-09: three of the four observables are IDENTICAL in
/// both polarities. Pinned so the fix is never read as a cure for the
/// arity-fallback failure.
#[test]
fn the_arity_slot_holds_the_stub_in_both_polarities() {
    for authoritative in [true, false] {
        let o = register_stub_then_real(authoritative);
        assert_eq!(
            o.bare,
            Some(REAL_ID),
            "bare slot (authoritative={}): the real declaration wins it via \
             register_function's richness promotion, in both polarities",
            authoritative
        );
        assert_eq!(
            o.arity2,
            Some(REAL_ID),
            "open#2 (authoritative={}) is the real function's own arity key",
            authoritative
        );
        let a3 = o.arity3.expect("open#3 must be bound");
        assert!(
            in_stage5(a3),
            "open#3 (authoritative={}) holds the STUB in BOTH polarities \
             (id {}): the authoritative form writes it directly, the plain \
             form reaches it through the collision branch's demotion to \
             `<name>#<its arity>`. This line is the reason the mount-miss \
             fix must not be described as fixing the arity fallback.",
            authoritative,
            a3
        );
    }
}

// ---------------------------------------------------------------------
// The gate proper: the invariant above is only worth anything if the
// MINT SITE is the one honouring it. The three tests above build both
// registrations by hand and would pass whichever one
// `codegen/expressions.rs` calls. This one compiles a module through
// the real entry point and asks the resulting context.
// ---------------------------------------------------------------------

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

/// A braced mount whose target module does not exist leaves a pending
/// alias; the call below then misses every table and mints the stage-5
/// placeholder.
const MOUNT_MISS: &str = r#"
module probe.t1172;

mount nowhere.absent.module.{ open };

public fn use_it(a: Int, b: Int, c: Int) -> Int {
    open(a, b, c)
}
"#;

#[test]
fn the_mint_site_registers_the_bare_alias_without_mount_privilege() {
    let mut parser = Parser::new(MOUNT_MISS);
    let module_ast = parser.parse_module().expect("probe source must parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("t1172_probe.vr"));
    // A mount miss is the POINT of this fixture, so a compile error here
    // is not automatically a failure — what matters is the registration
    // the miss path leaves behind.
    let _ = codegen.compile_module_with_mounts(&module_ast, "t1172_probe.vr", ".");

    let ctx = codegen.ctx_mut();
    let bare = ctx.lookup_function("open").map(|f| f.id.0);
    assert!(
        bare.is_some_and(in_stage5),
        "fixture no longer mints a stage-5 stub for `open` (bare slot: {:?}) \
         — the test is measuring nothing; re-derive the mount-miss shape \
         before trusting the assertion below",
        bare
    );
    assert!(
        !ctx.explicit_mount_names.contains("open"),
        "the stage-5 mint site registered the bare alias AUTHORITATIVELY: \
         a placeholder for a mount that MISSED is now indistinguishable \
         from a mount the user wrote, and `mount_preferred` will pick it \
         over arg-type overload ranking (T1172)"
    );
}
