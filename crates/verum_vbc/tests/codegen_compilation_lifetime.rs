#![cfg(feature = "codegen")]
//! A new compilation retains explicit inputs and discards prior unit authority.

use std::sync::Arc;
use verum_ast::cfg::TargetConfig;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction,
    bytecode::decode_instructions,
    codegen::{CodegenConfig, ExprId, ForeignInterfaces, SourceExports, TierContext, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    module::VbcModule,
    serialize::serialize_module,
    types::CbgrTier,
};

fn parse(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().unwrap()
}

fn wire(module: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(module).unwrap()).unwrap()
}

fn execute(module: VbcModule, name: &str) -> i64 {
    let entry = module.find_function_by_name(name).unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap()
        .try_as_i64()
        .unwrap()
}

#[test]
fn a_new_compilation_preserves_target_tiers_and_explicit_flags() {
    let source = parse(
        r#"
        module probe;
        @cfg(target_os = "linux") public fn selected()->Int {73}
        @cfg(target_os = "windows") public fn selected()->Int {29}
        @test fn example() { assert(selected() > 0); }
    "#,
    );
    for (target, expected) in [
        (TargetConfig::linux_x86_64(), 73),
        (TargetConfig::windows_x86_64(), 29),
    ] {
        let config = CodegenConfig::new("probe")
            .with_target(target)
            .with_debug_info()
            .with_source_map()
            .with_optimization_level(2)
            .with_validation()
            .with_strict_codegen();
        let expected_config = format!("{config:?}");
        let target_os = config.target_config.target_os.clone();
        let mut codegen = VbcCodegen::with_config(config);
        let mut tiers = TierContext::new();
        tiers.enabled = true;
        tiers.default_tier = CbgrTier::Tier1;
        tiers.set_tier(ExprId(42), CbgrTier::Tier0);
        codegen.set_tier_context(tiers);
        codegen.set_prefer_existing_functions(true);
        codegen.set_propagate_test_attr(false);
        for _ in 0..3 {
            let module = wire(&codegen.compile_module(&source).unwrap());
            assert_eq!(format!("{:?}", codegen.config()), expected_config);
            assert_eq!(codegen.ctx_mut().target_os, target_os.as_str());
            assert!(codegen.ctx_mut().prefer_existing_functions);
            assert_eq!(
                codegen.ctx_mut().tier_context.get_tier(ExprId(42)),
                CbgrTier::Tier0
            );
            assert_eq!(
                codegen.ctx_mut().tier_context.get_tier(ExprId(43)),
                CbgrTier::Tier1
            );
            let example = module.find_function_by_name("probe.example").unwrap();
            assert!(!module.get_function(example).unwrap().is_test);
            assert_eq!(execute(module, "probe.selected"), expected);
        }
        codegen.set_propagate_test_attr(true);
        codegen.set_prefer_existing_functions(false);
        let module = codegen.compile_module(&source).unwrap();
        assert!(
            module
                .get_function(module.find_function_by_name("probe.example").unwrap())
                .unwrap()
                .is_test
        );
        assert!(!codegen.ctx_mut().prefer_existing_functions);
    }
}

#[test]
fn a_failed_body_cannot_authorize_the_next_compilation() {
    let failed = parse(
        r#"
        module probe;
        type Previous is {value:Int};
        type Alias is Previous;
        fn broken()->Int {
            unsafe { let abandoned = Previous {value:73}; unavailable() }
        }
    "#,
    );
    let clean = parse("module probe; public fn selected()->Int {73}");
    let stale = parse("module probe; fn rejected(value:Alias)->Int {value.value}");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("probe"));
    for _ in 0..2 {
        let error = codegen.compile_module(&failed).unwrap_err().to_string();
        assert!(error.contains("unavailable"), "{error}");
        assert!(!codegen.ctx_mut().is_unsafe());
        let module = codegen.compile_module(&clean).unwrap();
        assert!(codegen.ctx_mut().lookup_var("abandoned").is_none());
        assert!(!codegen.ctx_mut().is_unsafe());
        assert!(
            module
                .types
                .iter()
                .all(|ty| module.get_string(ty.name) != Some("Previous"))
        );
        assert_eq!(execute(wire(&module), "probe.selected"), 73);
        let error = codegen.compile_module(&stale).unwrap_err().to_string();
        assert!(error.contains("Alias"), "{error}");
    }
    // An interrupted incremental unsafe scope is transient even when the
    // caller supplied persistent tier decisions before the next compilation.
    let _ = codegen.ctx_mut().enter_unsafe();
    assert!(codegen.ctx_mut().is_unsafe());
    codegen.compile_module(&clean).unwrap();
    assert!(!codegen.ctx_mut().is_unsafe());
}

#[test]
fn shared_declaration_catalogs_survive_reset_without_sharing_unit_state() {
    let provider = parse(
        r#"
        module provider;
        extern {
            public fn bridge(value:Int32, ...) -> Int32;
            fn hidden(value:Int32, ...) -> Int32;
        }
    "#,
    );
    let facade = parse("module facade; public mount provider.bridge as selected;");
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("provider"));
    producer
        .collect_unit_declarations(&[&provider, &facade])
        .unwrap();
    let declarations = producer.finalize_module_from_state().unwrap();
    let functions = producer.export_functions();
    let mut interfaces = ForeignInterfaces::default();
    producer.export_foreign_interfaces(&declarations, &mut interfaces);
    let mut exports = SourceExports::default();
    producer.publish_source_exports(&mut exports);
    let interfaces = Arc::new(interfaces);
    let exports = Arc::new(exports);
    let caller = parse(
        r#"
        module consumer;
        mount facade.selected as invoke;
        public fn probe()->Int32 {unsafe {invoke(7, 3)}}
    "#,
    );
    let private = parse(
        r#"
        module consumer;
        mount provider.hidden as invoke;
        public fn probe()->Int32 {unsafe {invoke(7, 3)}}
    "#,
    );
    for _ in 0..2 {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        consumer.set_foreign_interfaces(interfaces.clone());
        consumer.set_source_exports(exports.clone());
        for _ in 0..2 {
            consumer.initialize();
            consumer.import_functions(&functions);
            let module = wire(&consumer.compile_additional_module(&caller).unwrap());
            assert_eq!(module.ffi_symbols.len(), 1);
            assert_eq!(
                module.get_string(module.ffi_symbols[0].name),
                Some("bridge")
            );
            assert!(module.ffi_symbols[0].signature.is_variadic);
            let function = module
                .get_function(module.find_function_by_name("consumer.probe").unwrap())
                .unwrap();
            let begin = function.bytecode_offset as usize;
            let instructions = decode_instructions(
                &module.bytecode[begin..begin + function.bytecode_length as usize],
            )
            .unwrap();
            assert!(instructions.iter().any(|instruction| matches!(
                instruction,
                Instruction::FfiExtended { sub_op: 0x10, .. }
            )));
            consumer.initialize();
            consumer.import_functions(&functions);
            let error = consumer
                .compile_additional_module(&private)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("inaccessible") && error.contains("provider.hidden"),
                "{error}"
            );
        }
        let empty = parse("module consumer; public fn main()->Int {73}");
        consumer.compile_module(&empty).unwrap();
        assert!(
            consumer
                .ctx_mut()
                .lookup_function("provider.bridge")
                .is_none()
        );
        assert!(!consumer.is_ffi_function("provider.bridge"));
    }
}

#[test]
fn a_failed_protocol_registration_preserves_the_explicit_precedence_policy() {
    let failed = parse(
        "module probe; type Rule is protocol { fn inspect(&self); }; type Value is {item:Int}; implement Rule for Value {fn inspect(&self, extra:Missing) {}}",
    );
    let clean = parse("module probe; public fn selected()->Int {73}");
    for preference in [false, true] {
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("probe"));
        codegen.set_prefer_existing_functions(preference);
        let error = codegen.compile_module(&failed).unwrap_err().to_string();
        assert!(error.contains("Missing"), "{error}");
        assert_eq!(codegen.ctx_mut().prefer_existing_functions, preference);
        let module = codegen.compile_module(&clean).unwrap();
        assert_eq!(codegen.ctx_mut().prefer_existing_functions, preference);
        assert_eq!(execute(wire(&module), "probe.selected"), 73);
    }
}
