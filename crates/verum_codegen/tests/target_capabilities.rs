use std::sync::Arc;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, target_settings::TargetSettings};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel, context::Context, module::Linkage, passes::PassBuilderOptions,
};
use verum_vbc::{
    codegen::VbcCodegen,
    interpreter::Interpreter,
    mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
};

#[test]
fn backend_facts_follow_selected_target_and_reject_unknown_settings() {
    for (features, width) in [("-a", 0), ("+a", 32)] {
        let settings = TargetSettings::resolve(
            Some("riscv32-unknown-elf"),
            Some("generic-rv32"),
            Some(features),
        )
        .unwrap();
        let facts = settings.capabilities().unwrap();
        assert_eq!(facts.max_atomic_bits, width);
        assert_eq!(facts.pointer_bits, 32);
        assert_eq!(facts.has_feature("a"), features.starts_with('+'));
    }
    for (cpu, features) in [
        ("nonexistent-cpu", ""),
        ("generic", "+nonexistent-feature"),
        ("generic", "sse2"),
    ] {
        let settings =
            TargetSettings::resolve(Some("x86_64-unknown-linux-gnu"), Some(cpu), Some(features))
                .unwrap();
        assert!(settings.capabilities().is_err());
    }
    assert!(
        TargetSettings::resolve(None, None, None)
            .unwrap()
            .capabilities()
            .is_ok()
    );
}

#[test]
fn source_queries_agree_in_vm_and_native_after_real_specialization() {
    let source = r#"
        meta fn has_atomic<T>() -> Bool { @intrinsic("target_has_atomic", T) }
        fn outer<T>() -> Bool { has_atomic<T>() }
        fn layout<T>() -> Int {
            (@intrinsic("size_of", T) + @intrinsic("align_of", T)
                + @intrinsic("stride_of", T) + @intrinsic("bits_of", T)) as Int
        }
        type Word is UInt32;
        meta fn type_id<T>() -> UInt64 { @intrinsic("type_id", T) }
        type Tag<T> is { value:T };
        type Alias<T> is Tag<T>;
        fn identity_probe() -> UInt64 { type_id<Tag<Word>>() }
        fn atomic_probe() -> Int { if outer<Word>() && layout<Word>() == 44
            && type_id<Tag<Word>>() == type_id<Alias<UInt32>>()
            && type_id<Tag<Word>>() != type_id<Tag<UInt64>>() { 73 } else { 19 } }
        fn feature_probe(name: Text) -> Bool { @intrinsic("target_has_feature", name) }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let original = VbcCodegen::new().compile_module(&ast).unwrap();
    let mut vbc = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&original).unwrap(),
    )
    .unwrap();
    for function in &mut vbc.functions {
        function.is_generic = !function.type_params.is_empty();
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions =
            verum_vbc::bytecode::decode_instructions(&vbc.bytecode[start..end]).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    let mut graph = InstantiationGraph::new();
    for function in &vbc.functions {
        let body = &vbc.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        discover_call_instantiations(
            &vbc,
            &verum_vbc::bytecode::decode_instructions(body).unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .unwrap();
    }
    assert!(!graph.is_empty());
    let vbc = monomorphize_minimal(vbc, &graph).unwrap().module;
    let settings = TargetSettings::resolve(None, None, None).unwrap();
    let facts = Arc::new(settings.capabilities().unwrap());
    let entry = vbc.find_function_by_name("atomic_probe").unwrap();
    let mut vm = Interpreter::new(Arc::new(vbc.clone()));
    vm.state.config.target_capabilities = Some(facts.clone());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    let identity_entry = vbc.find_function_by_name("identity_probe").unwrap();
    let expected_identity = vm.execute_function(identity_entry).unwrap().as_i64() as u64;
    let context = Context::create();
    let mut config = LoweringConfig::debug("target_queries")
        .with_debug_info(false)
        .with_target(settings.triple.clone());
    config.target_cpu = settings.cpu.clone();
    config.target_features = settings.features.clone();
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering.lower_module(&vbc).unwrap();
    let module = lowering.into_module();
    module.verify().unwrap();
    for name in ["atomic_probe", "feature_probe", "identity_probe"] {
        assert!(
            module
                .get_function(name)
                .is_some_and(|function| function.count_basic_blocks() > 0),
            "missing native body for {name}"
        );
    }
    // Ordinary LLVM dead-code elimination keeps the actual reachable bodies,
    // including the native Text readers. No test substitutes their behavior.
    for function in module.get_functions() {
        if function.count_basic_blocks() > 0 {
            function.set_linkage(
                if matches!(
                    function.get_name().to_str().unwrap(),
                    "atomic_probe" | "feature_probe" | "identity_probe"
                ) {
                    Linkage::External
                } else {
                    Linkage::Internal
                },
            );
        }
    }
    module
        .run_passes(
            "globaldce",
            &settings.machine().unwrap(),
            PassBuilderOptions::create(),
        )
        .unwrap();
    module.verify().unwrap();
    for name in ["atomic_probe", "feature_probe", "identity_probe"] {
        assert!(
            module.get_function(name).is_some(),
            "missing {name} after DCE"
        );
    }
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    // SAFETY: these source functions use the native Int and Text argument ABI;
    // each Text descriptor and its exact byte range remain live during the call.
    unsafe {
        let atomic = engine
            .get_function::<unsafe extern "C" fn() -> i64>("atomic_probe")
            .unwrap();
        assert_eq!(atomic.call(), 73);
        let identity = engine
            .get_function::<unsafe extern "C" fn() -> u64>("identity_probe")
            .unwrap();
        assert_eq!(identity.call(), expected_identity);
        let feature = engine
            .get_function::<unsafe extern "C" fn(u64) -> bool>("feature_probe")
            .unwrap();
        for (name, expected) in facts.features.iter() {
            let descriptor = [
                name.as_bytes().as_ptr() as u64,
                name.len() as u64,
                name.len() as u64,
            ];
            assert_eq!(
                feature.call(descriptor.as_ptr() as u64),
                *expected,
                "{name}"
            );
        }
        for name in ["", "unknown", "sse2\0", "s\0e2"] {
            let descriptor = [
                name.as_bytes().as_ptr() as u64,
                name.len() as u64,
                name.len() as u64,
            ];
            assert!(!feature.call(descriptor.as_ptr() as u64), "{name:?}");
        }
    }
}
