//! Bare call selection follows retained source receiver roles, not parameter text.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_ast::FunctionParamKind;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenErrorKind, VbcCodegen, context::FunctionInfo},
    interpreter::Interpreter,
    module::FunctionId,
};

fn ast(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().expect("source grammar")
}
fn compile(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&ast(source))
        .expect("source VBC")
}
fn check_result(module: VbcModule, expected: i64) {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, wire] {
        let entry = module.find_function_by_name("main").unwrap();
        let result = Interpreter::try_new_validated(Arc::new(module))
            .unwrap()
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn explicitly_typed_self_is_regular_before_or_after_the_caller() {
    for name in ["self", "Receiver"] {
        let declaration = format!("fn pick({name}: Int) -> Int {{ {name} + 30 }}");
        let caller = "fn main() -> Int { pick(7) }";
        for source in [
            format!("{declaration} {caller}"),
            format!("{caller} {declaration}"),
        ] {
            let module = compile(&source);
            let target = module
                .get_function(module.find_function_by_name("pick").unwrap())
                .unwrap();
            assert!(matches!(
                target.source_signature.as_ref().unwrap().params[0].kind,
                FunctionParamKind::Regular { .. }
            ));
            check_result(module, 37);
        }
    }
}

#[test]
fn same_named_instance_method_does_not_capture_the_regular_declaration() {
    check_result(
        compile(
            r#"
        type Key is { value: Int };
        implement Key { fn pick(&self) -> Int { 0 } }
        fn pick(self: Int) -> Int { self + 30 }
        fn main() -> Int { pick(7) }
    "#,
        ),
        37,
    );
}

#[test]
fn mounted_regular_self_parameter_keeps_the_selected_body() {
    check_result(
        compile(
            r#"
        module provider { public fn pick(self: Int) -> Int { self + 30 } }
        mount provider.pick as selected;
        fn main() -> Int { selected(7) }
    "#,
        ),
        37,
    );
}

#[test]
fn lexical_function_inside_an_implementation_is_still_a_free_declaration() {
    check_result(
        compile(
            r#"
        type Key is { value: Int };
        implement Key {
            fn pick(&self) -> Int {
                fn increment(self: Int) -> Int { self + 30 }
                increment(43)
            }
        }
        fn main() -> Int { let key = Key { value: 0 }; key.pick() }
    "#,
        ),
        73,
    );
}

#[test]
fn method_receiver_cannot_be_erased_by_optional_function_info_names() {
    let source = ast(r#"
        type Key is { value: Int };
        implement Key { fn pick(&self) -> Int { 0 } }
        fn main() -> Int { pick(7) }
    "#);
    let mut codegen = VbcCodegen::new();
    codegen.collect_unit_declarations(&[&source]).unwrap();
    let mut method = codegen.ctx_mut().functions.get("Key.pick").unwrap().clone();
    method.param_names = ["value".to_owned()].into_iter().collect();
    method.parent_type_name = None;
    codegen
        .ctx_mut()
        .register_function("pick".to_owned(), method);
    let error = codegen.compile_function_bodies(&source).unwrap_err();
    assert!(
        matches!(error.kind, CodegenErrorKind::UndefinedFunction(ref name) if name == "pick"),
        "{error}"
    );
}

#[test]
fn unknown_callable_metadata_does_not_prove_a_free_source_body() {
    let mut codegen = VbcCodegen::new();
    codegen.ctx_mut().register_function(
        "unproved".to_owned(),
        FunctionInfo {
            id: FunctionId(500),
            param_count: 1,
            has_source_body: true,
            param_names: ["value".to_owned()].into_iter().collect(),
            ..FunctionInfo::default()
        },
    );
    let error = codegen
        .compile_module(&ast("fn main() -> Int { unproved(7) }"))
        .unwrap_err();
    assert!(
        matches!(error.kind, CodegenErrorKind::UndefinedFunction(ref name) if name == "unproved"),
        "{error}"
    );
}

#[test]
fn regular_forward_signature_is_callable_without_claiming_a_body() {
    let module = compile("fn pending(self: Int) -> Int; fn main() -> Int { pending(7) }");
    let declaration = module
        .get_function(module.find_function_by_name("pending").unwrap())
        .unwrap();
    assert!(!declaration.has_source_body);
    assert!(matches!(
        declaration.source_signature.as_ref().unwrap().params[0].kind,
        FunctionParamKind::Regular { .. }
    ));
    // Admission preserves the exact forward; execution still requires its body.
    verum_vbc::validate::validate_module(&module).unwrap();
}

#[test]
fn reserved_self_type_keyword_is_not_a_regular_parameter_binding() {
    assert!(Parser::new("fn pick(Self: Int) -> Int { Self + 30 }")
        .parse_module().is_err());
}

#[test]
fn declared_nominal_constructors_are_bare_callables_without_function_headers() {
    for source in [
        "type Number is (Int); fn main() -> Int { Number(73).0 }",
        "type Wrapped<T> is (T); fn main() -> Int { let value = Wrapped(73); value.0 }",
        "type Pair is {left: Int, right: Int}; fn main() -> Int { let value = Pair(30, 43); value.left + value.right }",
    ] {
        check_result(compile(source), 73);
    }
}

#[test]
fn a_local_callable_still_shadows_a_nominal_constructor() {
    check_result(compile(r#"
        type Number is (Int);
        fn main() -> Int {
            let Number = |value: Int| value + 30;
            Number(43)
        }
    "#), 73);
}

#[test]
fn a_synthetic_constructor_requires_its_declared_nominal_owner() {
    use verum_vbc::types::{TypeId, TypeRef};
    let mut codegen = VbcCodegen::new();
    codegen.ctx_mut().register_function("unproved".to_owned(), FunctionInfo {
        id: FunctionId(u32::MAX / 2),
        param_count: 1,
        return_type: Some(TypeRef::Concrete(TypeId(50000))),
        has_source_body: false,
        ..FunctionInfo::default()
    });
    let error = codegen.compile_module(&ast("fn main() -> Int { unproved(73) }")).unwrap_err();
    assert!(matches!(error.kind, CodegenErrorKind::UndefinedFunction(ref name)
        if name == "unproved"), "{error}");
}
