#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, target::TargetCapabilities};

fn module() -> verum_vbc::module::VbcModule {
    let source = r#"
        meta fn target_has_atomic<T>() -> Bool { @intrinsic("target_has_atomic", T) }
        fn nested<U>() -> Bool { target_has_atomic<U>() }
        type Word is UInt32;
        type Impostor is { value: UInt32 };
        fn narrow() -> Bool { nested<Byte>() }
        fn word() -> Bool { nested<Word>() }
        fn wide() -> Bool { nested<UInt64>() }
        fn too_wide() -> Bool { nested<UInt128>() }
        fn record() -> Bool { nested<Impostor>() }
        meta fn target_has_feature(name: Text) -> Bool { @intrinsic("target_has_feature", name) }
        fn enabled() -> Bool { target_has_feature("test-enabled") }
        fn disabled() -> Bool { target_has_feature("test-disabled") }
        fn unknown() -> Bool { target_has_feature("unknown") }
    "#;
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}

#[test]
fn generic_queries_rebind_to_the_execution_target_after_archive_roundtrip() {
    let original = module();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&original).unwrap(),
    )
    .unwrap();
    for module in [original, restored] {
        let module = Arc::new(module);
        for max_atomic_bits in [0, 32, 64, 128] {
            let mut facts = TargetCapabilities {
                max_atomic_bits,
                pointer_bits: 32,
                features: Default::default(),
            };
            facts.features.insert("test-enabled".into(), true);
            facts.features.insert("test-disabled".into(), false);
            let target = Arc::new(facts);
            for (name, expected) in [
                ("narrow", max_atomic_bits >= 8),
                ("word", max_atomic_bits >= 32),
                ("wide", max_atomic_bits >= 64),
                ("too_wide", false),
                ("record", false),
                ("enabled", true),
                ("disabled", false),
                ("unknown", false),
            ] {
                let id = module.find_function_by_name(name).unwrap();
                let mut vm = Interpreter::new(module.clone());
                vm.state.config.target_capabilities = Some(target.clone());
                let value = vm
                    .execute_function(id)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(
                    value.try_as_bool(),
                    Some(expected),
                    "{name} with atomic limit {max_atomic_bits}"
                );
            }
        }
    }
}

#[test]
fn absent_target_and_unbound_generic_refuse_instead_of_answering_false() {
    let module = Arc::new(module());
    let id = module.find_function_by_name("word").unwrap();
    let error = Interpreter::new(module.clone())
        .execute_function(id)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("target capabilities were not supplied"),
        "{error}"
    );
    let mut vm = Interpreter::new(module.clone());
    vm.state.config.target_capabilities = Some(Arc::new(TargetCapabilities {
        max_atomic_bits: 64,
        pointer_bits: 64,
        features: Default::default(),
    }));
    let id = module.find_function_by_name("target_has_atomic").unwrap();
    assert!(vm.execute_function(id).is_err());
}
