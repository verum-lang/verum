//! Generic calls use the ABI of the declaration selected by VBC identity.
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::context::Context;
use verum_vbc::{
    Instruction,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    module::VbcModule,
};

fn pointer_calls() -> VbcModule {
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .expect("actual intrinsic source");
    memory.items.retain(|item| match &item.kind {
        verum_ast::ItemKind::Module(decl) => decl.items.is_none(),
        verum_ast::ItemKind::Function(decl) => decl.name.name.as_str() == "ptr_is_null",
        _ => false,
    });
    let consumer = Parser::new(
        r#"
        module consumer;
        mount core.intrinsics.memory.ptr_is_null;
        fn query<T>(address: *const T) -> Bool { ptr_is_null<T>(address) }
        fn probe(address: *const Int) -> Bool { query<Int>(address) }
    "#,
    )
    .parse_module()
    .unwrap();
    let roots = [&memory, &consumer];
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("consumer").with_validation());
    generator.collect_unit_declarations(&roots).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&roots, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module_from_state().unwrap();
    assert!(module.resolve_external_bands().is_empty());
    module.synthesize_intrinsic_band_wrappers();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let mut module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut body =
            verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    let query = module
        .get_function(module.find_function_by_name("consumer.query").unwrap())
        .unwrap();
    assert!(
        query
            .instructions
            .as_ref()
            .unwrap()
            .iter()
            .any(|op| matches!(op,
        Instruction::CallG { func_id, .. }
            if module.get_function(module.resolved_function_id(*func_id))
                .and_then(|callee| module.get_string(callee.name))
                == Some("core.intrinsics.memory.ptr_is_null")))
    );
    module
}

#[test]
fn generic_pointer_call_uses_the_selected_intrinsic_parameter_abi() {
    let module = pointer_calls();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("generic_abi").with_debug_info(false),
    );
    lowering.lower_module(&module).expect("native lowering");
    assert!(
        lowering
            .module()
            .get_function("consumer.query")
            .unwrap()
            .count_basic_blocks()
            > 0
    );
    lowering.module().verify().expect("valid selected call ABI");
}

#[test]
fn generic_call_preserves_float_and_narrow_integer_parameter_types() {
    let ast = Parser::new(
        r#"
        fn choose<T>(tag: T, value: Float, narrow: UInt32) -> Float { value }
        fn probe(value: Float, narrow: UInt32) -> Float { choose<Int>(1, value, narrow) }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let probe = module
        .get_function(module.find_function_by_name("probe").unwrap())
        .unwrap();
    assert!(
        probe
            .instructions
            .as_ref()
            .unwrap()
            .iter()
            .any(|op| matches!(op, Instruction::CallG { .. }))
    );
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("scalar_abi").with_debug_info(false),
    );
    lowering.lower_module(&module).expect("native lowering");
    assert!(
        lowering
            .module()
            .get_function("probe")
            .unwrap()
            .count_basic_blocks()
            > 0
    );
    lowering.module().verify().expect("valid scalar call ABI");
}

#[test]
fn malformed_generic_call_arity_is_refused_before_emitting_the_call() {
    for count in [0, 2] {
        let mut module = pointer_calls();
        let query = module.find_function_by_name("consumer.query").unwrap();
        let body = module
            .functions
            .iter_mut()
            .find(|function| function.id == query)
            .unwrap()
            .instructions
            .as_mut()
            .unwrap();
        let arguments = body
            .iter_mut()
            .find_map(|op| match op {
                Instruction::CallG { args, .. } => Some(args),
                _ => None,
            })
            .unwrap();
        arguments.count = count;
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("invalid_abi").with_debug_info(false),
        );
        let error = lowering
            .lower_module(&module)
            .expect_err("invalid declared arity")
            .to_string();
        assert!(
            error.contains("selected call") && error.contains("declaration requires 1"),
            "{error}"
        );
    }
}

#[test]
fn incompatible_native_carrier_is_not_replaced_with_a_null_pointer() {
    // Deliberately corrupt the decoded body: the native boundary must refuse
    // an incompatible carrier even if earlier VBC validation was bypassed.
    let mut module = pointer_calls();
    let query = module.find_function_by_name("consumer.query").unwrap();
    let body = module
        .functions
        .iter_mut()
        .find(|function| function.id == query)
        .unwrap()
        .instructions
        .as_mut()
        .unwrap();
    let (position, argument) = body
        .iter()
        .enumerate()
        .find_map(|(position, op)| match op {
            Instruction::CallG { args, .. } => Some((position, args.start)),
            _ => None,
        })
        .unwrap();
    body.insert(
        position,
        Instruction::LoadF {
            dst: argument,
            value: 0.5,
        },
    );
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("invalid_carrier").with_debug_info(false),
    );
    let error = lowering
        .lower_module(&module)
        .expect_err("incompatible carrier")
        .to_string();
    assert!(error.contains("cannot adapt native value"), "{error}");
}
