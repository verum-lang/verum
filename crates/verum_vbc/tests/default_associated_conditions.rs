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

#[test]
fn inherited_default_equality_uses_the_protocols_nominal_dependency() {
    let provider = r#"
        module lib.stream;
        public type Verdict<T, E> is | Accepted(T) | Rejected(E);
        public type Signal is | Ready | Failed(Int);
        public type Stream is protocol {
            type Item;
            fn local_tag(&self) -> Int {
                let value: Signal = Signal.Failed(7);
                match value { Signal.Failed(n) => n, Signal.Ready => 0 }
            }
            fn take(&mut self) -> Self.Item;
            fn tag(&self, value: Verdict<Int, Int>) -> Int {
                match value { Verdict.Accepted(_) => 1, Verdict.Rejected(_) => 2 }
            }
            fn inspect<T, E>(mut self) -> Int where Self.Item = Verdict<T, E> {
                match self.take() { Verdict.Accepted(_) => 1, Verdict.Rejected(_) => 2 }
            }
        };
    "#;
    let foreign = Parser::new("module decoy; public type Verdict<T, E> is | Unrelated(T, E);")
        .parse_module()
        .unwrap();
    let decoy = VbcCodegen::with_config(CodegenConfig::new("decoy"))
        .compile_module(&foreign)
        .unwrap();
    for (imports, item, take, guard, expected) in [
        ("", "Piece", "Piece.Root", true, Ok(false)),
        (
            "mount decoy.{Verdict};",
            "Verdict<Int, Int>",
            "Verdict.Unrelated(1, 2)",
            true,
            Ok(false),
        ),
        (
            "mount lib.stream.{Verdict};",
            "Verdict<Int, Int>",
            "Verdict.Accepted(4)",
            true,
            Ok(true),
        ),
        (
            "mount decoy.{Verdict}; mount lib.stream.{Verdict as Original};",
            "Original<Int, Int>",
            "Original.Accepted(4)",
            true,
            Ok(true),
        ),
        ("", "Piece", "Piece.Root", false, Err("no declared tag")),
    ] {
        let src = if guard {
            provider.into()
        } else {
            provider.replace(" where Self.Item = Verdict<T, E>", "")
        };
        let src = Parser::new(&src).parse_module().unwrap();
        let mut producer = VbcCodegen::with_config(CodegenConfig::new("lib"));
        producer.collect_unit_declarations(&[&src]).unwrap();
        let protocols = producer.export_protocols();
        let module = producer.finalize_module_from_state().unwrap();

        let src = format!(
            "module app.path; mount lib.stream.{{Stream}}; {imports}
            type Piece is | Root | Leaf(Int);
            type Cursor is {{ at: Int }};
            implement Stream for Cursor {{
                type Item = {item};
                fn take(&mut self) -> {item} {{ {take} }}
            }}"
        );
        let receiver = Parser::new(&src).parse_module().unwrap();
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("app"));
        compiler.import_protocols(&protocols);
        compiler
            .import_bootstrap_nominal_dependencies(&[&receiver], &[&module, &decoy])
            .unwrap();
        compiler.collect_unit_declarations(&[&receiver]).unwrap();
        compiler.resolve_pending_imports();
        let result = compiler.compile_default_methods_with_policy(ItemFailurePolicy::Strict);
        match expected {
            Ok(present) => {
                result.unwrap_or_else(|e| panic!("{imports} {item}: {e}"));
                assert!(compiler.export_functions().contains_key("Cursor.local_tag"));
                assert!(compiler.export_functions().contains_key("Cursor.tag"));
                assert_eq!(
                    compiler.export_functions().contains_key("Cursor.inspect"),
                    present,
                    "{imports}"
                );
            }
            Err(message) => assert!(result.unwrap_err().to_string().contains(message)),
        }
    }
}

#[test]
fn a_referenced_protocol_does_not_own_the_implementors_module_scope() {
    let facade = Parser::new("module app; public mount .platform.{Timer};")
        .parse_module()
        .unwrap();
    let implementor = Parser::new(
        "module app.marker; public type Marker is { n: Int };
        implement Eq for Marker { fn eq(&self, other: &Self) -> Bool { true } }",
    )
    .parse_module()
    .unwrap();
    let source = Parser::new("module app.platform.clock; public type Timer is { n: Int };")
        .parse_module()
        .unwrap();
    let unit = [&facade, &implementor];
    let first = VbcCodegen::with_config(CodegenConfig::new("app"))
        .collect_declaration_interface(&unit)
        .unwrap();
    let foreign = VbcCodegen::with_config(CodegenConfig::new("app.platform"))
        .collect_declaration_interface(&[&source])
        .unwrap();
    assert!(
        first
            .types
            .iter()
            .any(|ty| first.get_string(ty.name) == Some("Eq")
                && ty.kind == verum_vbc::types::TypeKind::Protocol
                && ty.origin_module.is_none())
    );
    let mut resolved = VbcCodegen::with_config(CodegenConfig::new("app"));
    resolved
        .import_bootstrap_nominal_dependencies_with_resolver(&unit, &[&first, &foreign], |name| {
            (name == "app.platform.Timer").then(|| "app.platform.clock.Timer".into())
        })
        .unwrap();
    let resolved = resolved.collect_declaration_interface(&unit).unwrap();
    for phase in [&first, &resolved] {
        assert!(!phase.declaration_type_bindings.contains_key(&"app".into()));
    }
}

#[test]
fn a_default_requires_the_protocol_of_this_applied_receiver() {
    let source = r#"
        module bounded;
        type SizedCursor is protocol { fn count(&self) -> Int; };
        type Walking is protocol {
            fn report(&self) -> Int where Self: SizedCursor { self.count() }
        };
        type Cursor<T> is { value: T };
        implement Walking for Cursor<Int> {}
        implement Walking for Cursor<Bool> {}
        implement Walking for Cursor<Text> {}
        implement SizedCursor for Cursor<Bool> { fn count(&self) -> Int { 17 } }
        implement SizedCursor for Cursor<Text> { fn count(&self) -> Int { 31 } }
        fn probe() -> Int {
            let b: Cursor<Bool> = Cursor { value: true };
            let t: Cursor<Text> = Cursor { value: "t" };
            b.report() * 100 + t.report()
        }
    "#;
    let build = |source: &str| {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("bounded"));
        cg.collect_unit_declarations(&[&ast])?;
        cg.resolve_pending_imports();
        cg.compile_default_methods_with_policy(ItemFailurePolicy::Strict)?;
        cg.compile_unit_items(&[&ast], ItemFailurePolicy::Strict)?;
        cg.finalize_module_from_state()
    };
    let compiled = build(source).expect("unavailable default is omitted for the wide cursor");
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
    ).unwrap();
    let entry = module.find_function_by_name("bounded.probe").unwrap();
    let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry).unwrap();
    assert_eq!(result.try_as_i64(), Some(1731));
    assert!(build(&format!("{source} fn denied(c: Cursor<Int>) -> Int {{ c.report() }}")).is_err());

    let blanket = source.replace(
        "implement SizedCursor for Cursor<Bool> { fn count(&self) -> Int { 17 } }",
        "type Mark is protocol {}; implement Mark for Cursor<Bool> {} \
         implement<C: Mark> SizedCursor for C { fn count(&self) -> Int { 17 } }",
    ).replace("implement Walking for Cursor<Int> {}", "");
    let module = build(&blanket).expect("a blanket declaration can supply the missing sibling");
    let entry = module.find_function_by_name("bounded.probe").unwrap();
    let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry).unwrap();
    assert_eq!(result.try_as_i64(), Some(1731));
}

#[test]
fn projected_static_calls_execute_the_implemented_identity_after_serialization() {
    let source = r#"
        module projected;
        type Seed is protocol { fn seed() -> Self; };
        type Family is protocol {
            type Element;
            fn starting_value(&self) -> Self.Element where Self.Element: Seed { Self.Element.seed() }
        };
        type Offset is { bits: Int }; type Meter is { bits: Int };
        implement Seed for Offset { fn seed() -> Self { Offset { bits: 17 } } }
        implement Seed for Meter { fn seed() -> Self { Meter { bits: 31 } } }
        type Cursor<A> is { value: A };
        implement<A> Family for Cursor<A> { type Element = A; }
        fn probe() -> Int {
            let a: Cursor<Offset> = Cursor { value: Offset {bits: 0} };
            let b: Cursor<Meter> = Cursor { value: Meter {bits: 0} };
            let first: Offset = a.starting_value();
            let second: Meter = b.starting_value();
            first.bits * 100 + second.bits
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("projected"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_default_methods_with_policy(ItemFailurePolicy::Strict).unwrap();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
    let compiled = codegen.finalize_module_from_state().unwrap();
    let mut module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
    ).unwrap();
    assert!(module.resolve_external_bands().is_empty());
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("projected.probe").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry).unwrap();
    assert_eq!(value.try_as_i64(), Some(1731));
}

#[test]
fn a_projected_initializer_types_a_sibling_callback_in_a_generic_method() {
    let ast = Parser::new(r#"
        module siblings;
        type Seed is protocol { fn seed() -> Self; };
        type Offset is { value: Int };
        implement Seed for Offset { fn seed() -> Self { Offset { value: 17 } } }
        type Accumulate is protocol { fn combine(self, rhs: Self) -> Self; };
        implement Accumulate for Offset { fn combine(self, rhs: Offset) -> Offset {
            Offset { value: self.value + rhs.value - 10 }
        } }
        type Family is protocol {
            type Item;
            fn item(&self) -> Self.Item;
            fn with_seed<T,F:fn(T,Self.Item)->T>(&self, initial:T, combine:F) -> T {
                combine(initial, self.item())
            }
            fn accumulated(&self) -> Self.Item where Self.Item: Seed + Accumulate {
                self.with_seed(Self.Item.seed(), |a, b| a.combine(b))
            }
        };
        type Cursor is {};
        implement Family for Cursor {
            type Item = Offset;
            fn item(&self) -> Offset { Offset { value: 20 } }
        }
        fn probe() -> Int { let value: Offset = Cursor {}.accumulated(); value.value }
    "#).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("siblings"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_default_methods_with_policy(ItemFailurePolicy::Strict).unwrap();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
    let compiled = codegen.finalize_module_from_state().unwrap();
    let mut module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&compiled).unwrap(),
    ).unwrap();
    assert!(module.resolve_external_bands().is_empty());
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("siblings.probe").unwrap();
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry).unwrap();
    assert_eq!(value.try_as_i64(), Some(27));
}
