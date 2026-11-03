#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::cfg::TargetConfig;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenError, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(body: &str, triple: &str) -> Result<VbcModule, CodegenError> {
    let source = format!("module cfg_probe; {body}");
    let ast = Parser::new(&source).parse_module().unwrap();
    VbcCodegen::with_config(
        CodegenConfig::new("cfg_probe")
            .with_target(TargetConfig::for_target(triple))
            .with_validation(),
    )
    .compile_module(&ast)
}

const CONDITIONAL_SIGNATURE: &str = r#"
    type Cell is { n: Int };
    implement Cell {
        @cfg(target_os = "linux")
        fn platform(&self, value: &PlatformOnly) -> Int { 91 }
        fn keep(&self) -> Int { self.n }
    }
    fn run() -> Int { Cell { n: 7 }.keep() }
"#;

#[test]
fn unavailable_platform_signature_is_not_lowered() {
    compile(CONDITIONAL_SIGNATURE, "aarch64-apple-darwin").unwrap();
}

#[test]
fn available_platform_signature_still_requires_its_type() {
    let error = compile(CONDITIONAL_SIGNATURE, "x86_64-unknown-linux-gnu").unwrap_err();
    assert!(error.to_string().contains("PlatformOnly"), "{error:?}");
}

const CONDITIONAL_OVERRIDE: &str = r#"
    type Value is protocol { fn value(&self) -> Int { 17 } };
    type Cell is { n: Int };
    implement Value for Cell {
        @cfg(target_os = "linux")
        fn value(&self) -> Int { 91 }
    }
    fn run() -> Int { Cell { n: 7 }.value() }
"#;

fn executes(triple: &str, expected: i64) {
    let module = compile(CONDITIONAL_OVERRIDE, triple).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let id = module.find_function_by_name("cfg_probe.run").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn excluded_override_cannot_suppress_a_protocol_default() {
    executes("aarch64-apple-darwin", 17);
}

#[test]
fn selected_override_executes_on_its_actual_target() {
    executes("x86_64-unknown-linux-gnu", 91);
}

#[test]
fn malformed_cfg_after_false_is_still_a_declaration_error() {
    let error = compile(
        r#"
        type Cell is { n: Int };
        implement Cell {
            @cfg(target_os = "linux") @cfg(42)
            fn value(&self) -> Int { 91 }
        }
    "#,
        "aarch64-apple-darwin",
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalid @cfg"), "{error:?}");
    assert!(error.span.is_some());
}

#[test]
fn associated_type_evidence_contains_only_the_selected_alternative() {
    let source = r#"
        type Source is protocol { type Item; };
        type Cell is { n: Int };
        implement Source for Cell {
            @cfg(target_os = "macos") type Item = Int;
            @cfg(target_os = "linux") type Item = Bool;
        }
    "#;
    for (target, expected) in [
        ("aarch64-apple-darwin", verum_vbc::types::TypeId::INT),
        ("x86_64-unknown-linux-gnu", verum_vbc::types::TypeId::BOOL),
    ] {
        let module = compile(source, target).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
        ] {
            let cell = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some("Cell"))
                .unwrap();
            assert_eq!(cell.protocols.len(), 1);
            let bindings = &cell.protocols[0].associated_types;
            assert_eq!(bindings.len(), 1);
            assert_eq!(module.get_string(bindings[0].0), Some("Item"));
            assert_eq!(bindings[0].1, verum_vbc::types::TypeRef::Concrete(expected));
        }
    }
}

#[test]
fn concrete_receiver_condition_uses_only_active_associated_types() {
    let source = r#"
        type Outcome<T> is Done(T) | Empty;
        type Source is protocol {
            type Item;
            fn tagged(&self) -> Int where Self.Item = Outcome<Int> { 17 }
        };
        type Cell<T> is { n: T };
        implement Source for Cell<Int> {
            @cfg(target_os = "macos") type Item = Outcome<Int>;
            @cfg(target_os = "linux") type Item = Bool;
        }
        @cfg(target_os = "macos")
        fn run() -> Int { Cell { n: 7 }.tagged() }
    "#;
    for (target, expected) in [
        ("aarch64-apple-darwin", true),
        ("x86_64-unknown-linux-gnu", false),
    ] {
        let module = compile(source, target).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
        ] {
            let receiver = module
                .types
                .iter()
                .find(|ty| {
                    module.get_string(ty.name) == Some("Cell")
                        && ty.origin_module.and_then(|id| module.get_string(id))
                            == Some("cfg_probe")
                })
                .unwrap();
            // Local names and their declaration owner are separate wire fields.
            let present = module.functions.iter().any(|function| {
                module.get_string(function.name) == Some("Cell.tagged")
                    && function.origin_module.and_then(|id| module.get_string(id))
                        == Some("cfg_probe")
                    && function.parent_type == Some(receiver.id)
            });
            assert_eq!(present, expected, "{target}");
            if expected {
                let entry = module.find_function_by_name("cfg_probe.run").unwrap();
                let value = Interpreter::new(Arc::new(module))
                    .execute_function(entry)
                    .unwrap();
                assert_eq!(value.try_as_i64(), Some(17));
            }
        }
    }
}

#[test]
fn associated_constants_follow_the_same_target_selection() {
    let source = r#"
        type Cell is { n: Int };
        implement Cell {
            @cfg(target_os = "linux") const CODE: Int = 91;
            @cfg(target_os = "macos") const CODE: Int = 17;
        }
        fn run() -> Int { Cell.CODE }
    "#;
    for (target, expected) in [
        ("aarch64-apple-darwin", 17),
        ("x86_64-unknown-linux-gnu", 91),
    ] {
        let module = compile(source, target).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
        ] {
            let entry = module.find_function_by_name("cfg_probe.run").unwrap();
            let value = Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap();
            assert_eq!(value.try_as_i64(), Some(expected));
        }
    }
}

#[test]
fn blanket_override_selection_matches_ordinary_implementations() {
    let source = r#"
        type Base is protocol {};
        type Value is protocol { fn value(&self) -> Int { 17 } };
        type Cell is { n: Int };
        implement<I: Base> Value for I {
            @cfg(target_os = "linux") fn value(&self) -> Int { 91 }
        }
        implement Base for Cell {}
        fn run() -> Int { Cell { n: 7 }.value() }
    "#;
    for (target, expected) in [
        ("aarch64-apple-darwin", 17),
        ("x86_64-unknown-linux-gnu", 91),
    ] {
        let module = compile(source, target).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
        ] {
            let entry = module.find_function_by_name("cfg_probe.run").unwrap();
            let value = Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap();
            assert_eq!(value.try_as_i64(), Some(expected));
        }
    }
}
