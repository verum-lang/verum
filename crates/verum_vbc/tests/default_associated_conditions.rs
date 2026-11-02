#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};

#[test]
fn only_contradicted_associated_equalities_exclude_a_default() {
    let source = Parser::new(
        r#"
        module gates;
        type Outcome<T> is Count(T) | Done;
        type Source is protocol {
            type Item;
            fn restricted<T>(&self) -> Bool where Self.Item = Outcome<T> { true }
            fn general(&self) -> Bool { true }
        };
        type Marker is protocol extends Source {};
        type Unrelated is protocol { type Item; };
        type Missing is { n: Int };
        type Fixed is { n: Int };
        type Possible is { n: Int };
        type Generic<A> is { value: A };
        type Scalar<A> is { value: A };
        type Mixed<A> is { value: A };
        implement Unrelated for Missing { type Item = Int; }
        implement Source for Missing {}
        implement Source for Fixed { type Item = Int; }
        implement Source for Possible { type Item = Outcome<Int>; }
        implement<A> Source for Generic<A> { type Item = A; }
        implement Marker for Scalar<USize> {}
        implement Marker for Mixed<ISize> {}
        implement gates.Source for Scalar<ISize> { type Item = ISize; }
        implement Source for Scalar<USize> { type Item = USize; }
        implement Source for Scalar<Int> { type Item = Int; }
        implement Source for Scalar<Bool> { type Item = Bool; }
        implement Source for Mixed<ISize> { type Item = ISize; }
        implement Source for Mixed<USize> { type Item = Outcome<USize>; }
        fn main() -> Int {
            let ready = Possible { n: 0 };
            let ordinary = Fixed { n: 0 };
            if ready.restricted<Int>() && ordinary.general() { 73 } else { 0 }
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("gates"));
    cg.collect_unit_declarations(&[&source]).unwrap();
    cg.resolve_pending_imports();
    cg.compile_default_methods_with_policy(ItemFailurePolicy::Strict)
        .unwrap();
    cg.compile_unit_items(&[&source], ItemFailurePolicy::Strict)
        .unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let present = |ty: &str, method: &str| {
        let name = format!("{ty}.{method}");
        module.functions.iter().any(|f| {
            module
                .get_string(f.name)
                .is_some_and(|key| key == name || key.ends_with(&format!(".{name}")))
        })
    };
    assert!(!present("Fixed", "restricted"));
    assert!(!present("Scalar", "restricted"));
    for ty in ["Fixed", "Scalar", "Possible", "Generic", "Mixed"] {
        assert!(present(ty, "general"), "{ty}");
    }
    for ty in ["Possible", "Generic", "Mixed", "Missing"] {
        assert!(present(ty, "restricted"), "{ty}");
    }
    let entry = module.find_function_by_name("gates.main").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .unwrap();
    assert_eq!(value.try_as_i64(), Some(73));
}

#[test]
fn imported_protocol_defaults_keep_their_declaration_identity() {
    let provider = Parser::new(
        r#"
        module library.protocols;
        public type Source is protocol {
            type Item;
            fn restricted<T, E>(&self) -> Bool where Self.Item = Result<T, E> { true }
            fn general(&self) -> Bool { true }
        };
    "#,
    )
    .parse_module()
    .unwrap();
    let receiver = Parser::new(
        r#"
        module app;
        mount library.protocols.{Source};
        type Cursor is { position: Int };
        implement Source for Cursor { type Item = Int; }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("library.protocols"));
    producer.collect_unit_declarations(&[&provider]).unwrap();
    let protocols = producer.export_protocols();
    let provider = producer.finalize_module_from_state().unwrap();
    for load_descriptor in [false, true] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("app"));
        cg.import_protocols(&protocols);
        if load_descriptor {
            cg.import_bootstrap_nominal_dependencies(&[&receiver], &[&provider])
                .unwrap();
        }
        cg.collect_unit_declarations(&[&receiver]).unwrap();
        cg.resolve_pending_imports();
        cg.compile_default_methods_with_policy(ItemFailurePolicy::Strict)
            .unwrap();
        let functions = cg.export_functions();
        assert!(functions.contains_key("Cursor.general"));
        assert!(!functions.contains_key("Cursor.restricted"));
        let module = cg.finalize_module_from_state().unwrap();
        let cursor = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Cursor"))
            .unwrap();
        let id = cursor.protocols[0].protocol;
        let owner = module.types.iter().find(|ty| ty.id.0 == id.0).unwrap();
        assert_eq!(
            owner.origin_module.and_then(|name| module.get_string(name)),
            Some("library.protocols")
        );
    }
}
