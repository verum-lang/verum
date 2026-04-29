//! AOT permission policy lowering — IR-level guardrails.
//!
//! These tests build a minimal VBC module containing a single
//! `PermissionAssert` site, lower it under each policy shape, and
//! pin the emitted LLVM IR. The contract under test is the
//! script-mode security gap closer (SCRIPT-5d):
//!
//! * **No policy installed.** AOT trusted-application path. The
//!   assert is elided — the produced IR contains neither the
//!   permission-denied message string nor an `_exit` call.
//! * **Policy denies the scope entirely.** Deny-by-default with no
//!   matching grant. Lowering emits an unconditional panic — IR
//!   contains the message and an `_exit(143)` call.
//! * **Policy grants the scope (wildcard).** Unconditional pass —
//!   IR contains neither the message nor `_exit`.
//! * **Policy grants the scope (always-allow).** Same shape as
//!   wildcard for IR purposes. Memory and Cryptography behave this
//!   way under the script policy.
//! * **Policy lists specific targets.** IR contains a `switch i64`
//!   over the listed values; default case branches into the panic
//!   block.

use verum_codegen::llvm::permissions::AotPermissionPolicy;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_llvm::context::Context;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::module::{FunctionDescriptor, VbcModule};

/// Build a VBC module with a single `main` function whose body is
/// `LoadI 42 r0; PermissionAssert(scope_tag, r0); Ret r0`. The
/// function descriptor declares 4 registers (more than enough for
/// the trivial body) and intern the name "main" so the lowerer
/// produces a recognisable LLVM define.
fn build_module_with_perm_assert(scope_tag: u8) -> VbcModule {
    let mut module = VbcModule::new("perm_test".to_string());
    let name_id = module.intern_string("main");
    let mut desc = FunctionDescriptor::new(name_id);
    desc.register_count = 4;
    desc.return_type = verum_vbc::types::TypeRef::concrete(verum_vbc::types::TypeId::INT);
    let instructions = vec![
        Instruction::LoadI { dst: Reg(0), value: 42 },
        Instruction::PermissionAssert { scope_tag, target_id: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ];
    desc.instructions = Some(instructions);
    module.add_function(desc);
    module
}

/// Lower `module` under `policy` and return the textual LLVM IR.
fn lower_with_policy(
    module: &VbcModule,
    policy: Option<AotPermissionPolicy>,
) -> String {
    let context = Context::create();
    let config = LoweringConfig::debug("perm_test").with_permission_policy(policy);
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering
        .lower_module(module)
        .expect("module should lower under any permission policy");
    lowering.get_ir().to_string()
}

#[test]
fn no_policy_elides_permission_assert_entirely() {
    let module = build_module_with_perm_assert(0);
    let ir = lower_with_policy(&module, None);
    assert!(
        !ir.contains("permission denied"),
        "no policy installed → assert must be elided, but IR contains the denial message:\n{ir}"
    );
    assert!(
        !ir.contains("perm_denied_msg"),
        "no policy installed → assert must be elided, but IR contains the panic global:\n{ir}"
    );
}

#[test]
fn empty_policy_emits_unconditional_panic_for_unknown_scope() {
    let module = build_module_with_perm_assert(0);
    // Empty policy — no grants of any shape, not in always_allow.
    // The lowerer must emit an unconditional panic at the call
    // site since no target_id can satisfy the policy.
    let policy = AotPermissionPolicy::default();
    let ir = lower_with_policy(&module, Some(policy));
    assert!(
        ir.contains("permission denied"),
        "deny-everything policy must surface the denial message in IR:\n{ir}"
    );
    assert!(
        ir.contains("@_exit"),
        "deny-everything policy must call _exit:\n{ir}"
    );
}

#[test]
fn wildcard_grant_elides_assert_completely() {
    let module = build_module_with_perm_assert(2); // Network
    let mut policy = AotPermissionPolicy::default();
    policy.wildcards.insert(2);
    let ir = lower_with_policy(&module, Some(policy));
    assert!(
        !ir.contains("permission denied"),
        "wildcard grant elides the assert; IR must not carry the denial message:\n{ir}"
    );
    assert!(
        !ir.contains("perm_denied_msg"),
        "wildcard grant elides the assert; IR must not carry the panic global:\n{ir}"
    );
}

#[test]
fn always_allow_scope_elides_assert_completely() {
    let module = build_module_with_perm_assert(4); // Memory
    let mut policy = AotPermissionPolicy::default();
    policy.always_allow.insert(4);
    let ir = lower_with_policy(&module, Some(policy));
    assert!(
        !ir.contains("permission denied"),
        "always-allow scope elides the assert; IR must not carry the denial message:\n{ir}"
    );
}

#[test]
fn specific_target_grant_emits_switch_against_target_id() {
    let module = build_module_with_perm_assert(1); // FileSystem
    let mut policy = AotPermissionPolicy::default();
    policy.specific.insert((1, 0xDEADBEEF));
    policy.specific.insert((1, 0xCAFE));
    let ir = lower_with_policy(&module, Some(policy));
    // The lowerer emits an i64 switch with one case per allowed
    // target and a default branch into the panic block.
    assert!(
        ir.contains("switch i64"),
        "specific-target grant must lower to an i64 switch over target_id:\n{ir}"
    );
    assert!(
        ir.contains("permission denied"),
        "default case still contains the panic message:\n{ir}"
    );
}

#[test]
fn cross_scope_grant_does_not_leak_to_other_scopes() {
    // Scope 1 has a wildcard; scope 0 has nothing → assert on
    // scope 0 must still panic. Pin that the lowerer doesn't
    // accidentally treat any wildcard as a global pass.
    let module = build_module_with_perm_assert(0); // Syscall
    let mut policy = AotPermissionPolicy::default();
    policy.wildcards.insert(1); // FileSystem wildcard
    let ir = lower_with_policy(&module, Some(policy));
    assert!(
        ir.contains("permission denied"),
        "wildcard for a different scope must not satisfy this assert:\n{ir}"
    );
}
