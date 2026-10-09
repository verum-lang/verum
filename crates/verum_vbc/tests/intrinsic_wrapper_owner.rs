//! T1688: wrapper fallback must retain the recorded callee's identity.
//! These assembled-module controls pin the resolver boundary independently of
//! source loading and of the interpreter's existing lenient TLS policy.

use verum_common::{List, Shared};
use verum_vbc::bytecode::encode_instructions;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcModule, XMOD_CALL_ID_BAND_BASE};
use verum_vbc::types::{StringId, TypeId, TypeRef};

const BAND: u32 = XMOD_CALL_ID_BAND_BASE + 43;

fn function(module: &mut VbcModule, name: &str, params: usize, instructions: List<Instruction>, decoded: bool) -> FunctionId {
    let mut descriptor = FunctionDescriptor::new(module.intern_string(name));
    descriptor.bytecode_offset = module.bytecode.len() as u32;
    descriptor.bytecode_length = encode_instructions(&instructions, &mut module.bytecode) as u32;
    descriptor.register_count = 8;
    descriptor.return_type = TypeRef::Concrete(TypeId::INT);
    descriptor.params = (0..params).map(|_| ParamDescriptor {
        name: StringId::EMPTY,
        type_ref: TypeRef::Concrete(TypeId::INT),
        is_mut: false,
        default: None,
        type_name: StringId::EMPTY,
    }).collect();
    descriptor.instructions = decoded.then_some(instructions);
    module.add_function(descriptor)
}

fn constant(module: &mut VbcModule, name: &str, value: i64, decoded: bool) -> FunctionId {
    function(module, name, 1, [
        Instruction::LoadI { dst: Reg(0), value },
        Instruction::Ret { value: Reg(0) },
    ].into_iter().collect(), decoded)
}

fn call(module: &mut VbcModule, name: &str, argc: u8) -> FunctionId {
    let name = module.intern_string(name);
    module.external_function_names.push((FunctionId(BAND), name));
    function(module, "main", 0, [
        Instruction::LoadI { dst: Reg(1), value: 1 },
        Instruction::LoadI { dst: Reg(2), value: 2 },
        Instruction::Call { dst: Reg(0), func_id: BAND, args: RegRange { start: Reg(1), count: argc } },
        Instruction::Ret { value: Reg(0) },
    ].into_iter().collect(), true)
}

fn alias(module: &mut VbcModule, name: &str, target: &str, archived_id: FunctionId) {
    let name = module.intern_string(name);
    let target = module.intern_string(target);
    module.mount_aliases.push((name, archived_id, target));
}

#[test]
fn missing_constructor_never_uses_another_types_new() {
    for (expected, unrelated) in [
        ("core.sync.atomic.AtomicU64.new", "ArenaPool.new"),
        ("core.example.Counter.new", "Resource.new"),
    ] {
        let mut module = VbcModule::new("constructor_owner".into());
        constant(&mut module, unrelated, 99, true);
        let main = call(&mut module, expected, 1);
        assert_eq!(module.resolve_external_bands().len(), 1);
        assert_eq!(module.synthesize_intrinsic_band_wrappers(), 0, "{expected}");
        assert_eq!(module.resolve_band_id(BAND), None);
        let error = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(main).expect_err("missing callee must remain a named error");
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn declared_constructor_still_resolves_before_wrapper_fallback() {
    let mut module = VbcModule::new("declared_constructor".into());
    constant(&mut module, "ArenaPool.new", 99, true);
    let actual = constant(&mut module, "AtomicU64.new", 17, true);
    let main = call(&mut module, "core.sync.atomic.AtomicU64.new", 1);
    assert!(module.resolve_external_bands().is_empty());
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 0);
    assert_eq!(module.resolve_band_id(BAND), Some(actual));
    assert_eq!(Interpreter::new(Shared::new(module).into_arc()).execute_function(main).unwrap().as_i64(), 17);
}

#[test]
fn unrelated_method_cannot_capture_a_genuine_intrinsic_wrapper() {
    let mut module = VbcModule::new("intrinsic_owner".into());
    constant(&mut module, "Other.eq", 99, true);
    let main = call(&mut module, "core.base.primitives.eq", 2);
    assert_eq!(module.resolve_external_bands().len(), 1);
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 1);
    let target = module.resolve_band_id(BAND).unwrap();
    assert_eq!(module.get_string(module.get_function(target).unwrap().name), Some("__band_wrapper$core$base$primitives$eq"));
    assert!(!Interpreter::new(Shared::new(module).into_arc()).execute_function(main).unwrap().as_bool());
}

#[test]
fn carried_alias_uses_canonical_body_not_same_leaf_or_archive_local_id() {
    let mut module = VbcModule::new("alias_owner".into());
    let foreign = constant(&mut module, "Foreign.cbgr_dealloc", 99, true);
    let actual = constant(&mut module, "core.mem.allocator.cbgr_dealloc", 17, true);
    let spelling = "core.base.memory.cbgr_dealloc";
    alias(&mut module, spelling, "core.mem.allocator.cbgr_dealloc", foreign);
    let main = call(&mut module, spelling, 1);
    assert_eq!(module.resolve_external_bands().len(), 1);
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 1);
    assert_eq!(module.resolve_band_id(BAND), Some(actual));
    assert_eq!(Interpreter::new(Shared::new(module).into_arc()).execute_function(main).unwrap().as_i64(), 17);
}

#[test]
fn carried_alias_accepts_encoded_body_without_decoded_instructions() {
    let mut module = VbcModule::new("encoded_owner".into());
    let actual = constant(&mut module, "core.mem.allocator.cbgr_dealloc", 17, false);
    let spelling = "core.base.memory.cbgr_dealloc";
    alias(&mut module, spelling, "core.mem.allocator.cbgr_dealloc", FunctionId(123456));
    call(&mut module, spelling, 1);
    assert_eq!(module.resolve_external_bands().len(), 1);
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 1);
    assert_eq!(module.resolve_band_id(BAND), Some(actual));
    let count = module.functions.len();
    module.resolve_external_bands();
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 1);
    assert_eq!(module.resolve_band_id(BAND), Some(actual));
    assert_eq!(module.functions.len(), count);
}

#[test]
fn missing_declared_alias_body_is_not_replaced_by_a_different_arity_intrinsic() {
    let mut module = VbcModule::new("missing_alias".into());
    let spelling = "core.base.memory.cbgr_dealloc";
    alias(&mut module, spelling, "core.mem.allocator.cbgr_dealloc", FunctionId(123456));
    call(&mut module, spelling, 1);
    assert_eq!(module.resolve_external_bands().len(), 1);
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 0);
    assert_eq!(module.resolve_band_id(BAND), None);
}

#[test]
fn exact_declaration_outranks_intrinsic_even_with_same_leaf_elsewhere() {
    let mut module = VbcModule::new("exact_owner".into());
    constant(&mut module, "Foreign.eq", 99, true);
    let actual = constant(&mut module, "core.example.eq", 17, true);
    call(&mut module, "core.example.eq", 1);
    assert!(module.resolve_external_bands().is_empty());
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 0);
    assert_eq!(module.resolve_band_id(BAND), Some(actual));
}

#[test]
fn non_core_name_does_not_acquire_an_intrinsic_wrapper() {
    let mut module = VbcModule::new("user_owner".into());
    call(&mut module, "app.eq", 2);
    assert_eq!(module.resolve_external_bands().len(), 1);
    assert_eq!(module.synthesize_intrinsic_band_wrappers(), 0);
    assert_eq!(module.resolve_band_id(BAND), None);
}
