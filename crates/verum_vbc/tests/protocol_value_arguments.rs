#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str) -> Result<VbcModule, verum_vbc::codegen::CodegenError> {
    let ast = Parser::new(source)
        .parse_module()
        .unwrap_or_else(|error| panic!("{source}\n{error:?}"));
    VbcCodegen::with_config(CodegenConfig::new("applications")).compile_module(&ast)
}

fn run(source: &str, expected: i64) {
    let module = compile(source).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("applications.main").unwrap();
        let result = Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(expected));
    }
}

#[test]
fn protocol_meta_int_reaches_inherited_default_through_wire_and_vm() {
    run(
        r#"
        module applications;
        type Dimension<N: meta Int> is protocol { fn dimension(&self) -> Int { N } };
        type Point is { value: Int };
        implement Dimension<73> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.dimension() }
    "#,
        73,
    );
}

#[test]
fn protocol_legacy_const_uses_the_same_value_substitution() {
    run(
        r#"
        module applications;
        type Dimension<const N: Int> is protocol { fn dimension(&self) -> Int { N } };
        type Point is { value: Int };
        implement Dimension<17> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.dimension() }
    "#,
        17,
    );
}

#[test]
fn protocol_meta_values_compose_through_superprotocols() {
    run(
        r#"
        module applications;
        type Parent<N: meta Int> is protocol { fn dimension(&self) -> Int { N } };
        type Child<M: meta Int> is protocol extends Parent<M> {};
        type Point is { value: Int };
        implement Child<29> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.dimension() }
    "#,
        29,
    );
}

#[test]
fn protocol_value_expressions_use_checked_arithmetic() {
    run(
        r#"
        module applications;
        type Parent<N: meta Int> is protocol { fn dimension(&self) -> Int { N } };
        type Child<M: meta Int> is protocol extends Parent<M + 2> {};
        type Point is { value: Int };
        implement Child<6 * 7> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.dimension() }
    "#,
        44,
    );
}

#[test]
fn protocol_values_keep_the_implementing_declaration_slot() {
    run(
        r#"
        module applications;
        type Dimension<N: meta Int> is protocol { fn dimension(&self) -> Int { N } };
        type Point<T, M: meta Int> is { value: T };
        implement<T, M: meta Int> Dimension<M> for Point<T, M> {}
        fn main() -> Int { let point: Point<Bool, 31> = Point { value: false }; point.dimension() }
    "#,
        31,
    );
}

#[test]
fn protocol_type_defaults_and_local_type_shadowing_preserve_value_kind() {
    run(
        r#"
        module applications;
        type Dimension<N: meta Int, T = Int> is protocol {
            fn dimension(&self) -> Int { N }
            fn echo(&self, value: T) -> T { value }
            fn choose<N>(&self, value: N) -> N { value }
        };
        type Point is { value: Int };
        implement Dimension<19> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.dimension() + point.echo(21) + point.choose<Int>(2) }
    "#,
        42,
    );
}

#[test]
fn protocol_receiver_default_is_instantiated_only_for_selected_implementation() {
    let source = r#"module applications;
        type Combine<Rhs = Self> is protocol { fn echo(&self, rhs: Rhs) -> Rhs { rhs } };
        type Point is { value: Int }; implement Combine for Point {}
        fn main() -> Int { let point = Point { value: 73 }; point.echo(point).value }
    "#;
    let module = compile(source).unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let protocol = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Combine"))
            .unwrap();
        assert!(
            protocol.type_params[0].default.is_none(),
            "receiver template is not a concrete runtime default"
        );
        let source = protocol
            .source_declaration
            .as_ref()
            .expect("default template survives wire");
        assert!(matches!(&source.generics[0].kind,
            verum_ast::ty::GenericParamKind::Type { default: Some(ty), .. }
                if matches!(&ty.kind, verum_ast::TypeKind::Path(path)
                    if path.segments.as_slice() == [verum_ast::PathSegment::SelfValue])));
        let entry = module.find_function_by_name("applications.main").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn nested_nominal_application_rejects_missing_required_type_and_value_arguments() {
    for parameter in ["N: meta Int", "N"] {
        let source = format!(
            r#"module applications;
            type Sized<A, {parameter}> is {{ value: A }};
            type Carrier<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Point is {{ value: Int }};
            implement Carrier<Sized<Int>> for Point {{}}
        "#
        );
        let error = compile(&source).expect_err("an incomplete witness must not reach the wire");
        assert!(
            matches!(
                error.kind,
                verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
            ),
            "{error}"
        );
        assert!(error.span.is_some(), "{error}");
        assert!(
            error.to_string().contains("missing required argument 'N'"),
            "{error}"
        );
    }
}

#[test]
fn nested_nominal_defaults_materialize_exact_arguments_in_source_wire_and_vm() {
    let module = compile(
        r#"module applications;
        type Pair<A, B = A> is { first: A, second: B };
        type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        type Point is { value: Int }; implement Carrier<Pair<Int>> for Point {}
        fn main() -> Int {
            let point = Point { value: 0 };
            let input: Pair<Int> = Pair { first: 17, second: 56 };
            let result = point.echo(input); result.first + result.second
        }
    "#,
    )
    .unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let echo = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name.ends_with("Point.echo"))
            })
            .unwrap();
        let pair = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Pair"))
            .unwrap()
            .id;
        let expected = verum_vbc::types::TypeRef::Instantiated {
            base: pair,
            args: vec![verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::INT); 2],
        };
        assert_eq!(echo.return_type, expected);
        assert_eq!(echo.semantic_return.as_ref(), Some(&expected));
        assert_eq!(
            echo.semantic_params.as_ref().unwrap()[1].as_ref(),
            Some(&expected)
        );
        let entry = module.find_function_by_name("applications.main").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn declared_type_slot_normalizes_ambiguous_brackets_without_changing_value_slots() {
    run(
        r#"module applications;
        type Pair<T, N: meta Int> is { value: T };
        type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        type Point is { value: Int }; implement Carrier<Pair<[Int; 3], 3>> for Point {}
        fn main() -> Int { let point = Point { value: 0 };
            let input: Pair<[Int; 3], 3> = Pair { value: [17, 25, 31] };
            let output = point.echo(input); output.value[0] + output.value[1] + output.value[2]
        }
    "#,
        73,
    );
    for arguments in ["(), ()", "3, 3"] {
        let text = format!(
            r#"module applications;
            type Pair<T, N: meta Int> is {{ value: T }};
            type Carrier<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Point is {{ value: Int }}; implement Carrier<Pair<{arguments}>> for Point {{}}
        "#
        );
        let error = compile(&text).expect_err("argument roles belong to the declaration");
        assert!(
            matches!(
                error.kind,
                verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
            ),
            "{error}"
        );
        assert!(error.span.is_some(), "{error}");
    }
}

#[test]
fn imported_nominal_default_retains_source_scope_and_exact_arguments() {
    let api = Parser::new("module api; public type Pair<A, B = A> is { first: A, second: B };")
        .parse_module()
        .unwrap();
    let archive = VbcCodegen::with_config(CodegenConfig::new("api"))
        .compile_module(&api)
        .unwrap();
    let archive = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&archive).unwrap(),
    )
    .unwrap();
    let source = Parser::new(
        r#"module consumer; mount api.Pair;
        fn echo(value: Pair<Int>) -> Pair<Int> { value }
        fn main() -> Int { let input: Pair<Int> = Pair { first: 17, second: 56 };
            let output = echo(input); output.first + output.second }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen
        .import_bootstrap_nominal_dependencies(&[&source], &[&archive])
        .unwrap();
    codegen.collect_unit_declarations(&[&source]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&source], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let echo = module
            .get_function(module.find_function_by_name("consumer.echo").unwrap())
            .unwrap();
        let verum_vbc::types::TypeRef::Instantiated { args, .. } = &echo.return_type else {
            panic!("{:?}", echo.return_type)
        };
        assert_eq!(
            args,
            &vec![verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::INT); 2]
        );
        assert_eq!(echo.semantic_return.as_ref(), Some(&echo.return_type));
        let entry = module.find_function_by_name("consumer.main").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn recursive_omitted_defaults_are_refused_without_inventing_a_type() {
    let error = compile(
        r#"module applications;
        type Recursive<A, B = Recursive<A>> is { value: A };
        type Carrier<T> is protocol { fn echo(&self, value: T) -> T { value } };
        type Point is { value: Int }; implement Carrier<Recursive<Int>> for Point {}
    "#,
    )
    .expect_err("recursive omitted default cannot establish a complete witness");
    assert!(
        matches!(
            error.kind,
            verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
        ),
        "{error}"
    );
    assert!(error.span.is_some(), "{error}");
}

fn rejects(source: &str, needle: &str) {
    let error = compile(source).unwrap_err();
    assert!(
        matches!(
            error.kind,
            verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
        ),
        "{error}"
    );
    assert!(error.span.is_some(), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("applications.Dimension") && message.contains("parameter 'N'"),
        "{message}"
    );
    assert!(message.contains(needle), "{message}");
}

#[test]
fn protocol_meta_rejects_type_argument_and_type_binder() {
    for declaration in [
        "implement Dimension<Int> for Point {}",
        "implement<N> Dimension<N> for Point {}",
    ] {
        rejects(
            &format!(
                r#"module applications;
            type Dimension<N: meta Int> is protocol {{}};
            type Point is {{ value: Int }};
            {declaration}
        "#
            ),
            "value",
        );
    }
}

#[test]
fn protocol_type_slot_rejects_value_binder_and_value_default() {
    for source in [
        r#"module applications; type Dimension<N> is protocol {};
           type Point is { value: Int }; implement<M: meta Int> Dimension<M> for Point {}"#,
        r#"module applications; type Dimension<M: meta Int, N = M> is protocol {};
           type Point is { value: Int }; implement Dimension<3> for Point {}"#,
    ] {
        rejects(source, "type argument names a value parameter");
    }
}

#[test]
fn protocol_compound_type_positions_cannot_smuggle_value_parameters() {
    for argument in ["(M, Int)", "&M", "[M]", "fn() -> M", "Packet<M>"] {
        rejects(
            &format!(
                r#"module applications;
            type Dimension<N> is protocol {{}};
            type Packet<T> is {{ value: T }};
            type Point is {{ value: Int }};
            implement<M: meta Int> Dimension<{argument}> for Point {{}}
        "#
            ),
            "type argument names a value parameter",
        );
    }
}

#[test]
fn protocol_nested_value_slots_and_rank2_shadowing_are_legal() {
    run(
        r#"module applications;
        type Sized<N: meta Int> is { value: Int };
        type Carrier<T> is protocol { fn constant(&self) -> Int { 73 } };
        type Dimension<N: meta Int> is protocol extends Carrier<Sized<N>> {};
        type Point is { value: Int };
        implement Dimension<3> for Point {}
        fn main() -> Int { let point = Point { value: 0 }; point.constant() }
    "#,
        73,
    );
    // The inner N is a type binder; it shadows the surrounding value binder.
    compile(
        r#"module applications;
        type Carrier<T> is protocol {};
        type Dimension<N: meta Int> is protocol extends Carrier<fn<N>(N) -> N> {};
        type Point is { value: Int }; implement Dimension<3> for Point {}
    "#,
    )
    .unwrap();
}

#[test]
fn protocol_meta_cannot_erase_a_refined_alias() {
    rejects(
        r#"module applications;
        type Positive is Int{> 0};
        type Dimension<N: meta Positive> is protocol {};
        type Point is { value: Int }; implement Dimension<-1> for Point {}
    "#,
        "representation",
    );
}

#[test]
fn protocol_nested_value_slots_reject_type_witnesses_and_refined_carriers() {
    for (slot, argument) in [
        ("Int", "T"),
        ("Int", "Int"),
        ("Bool", "1"),
        ("Int{> 0}", "-1"),
    ] {
        rejects(
            &format!(
                r#"module applications;
            type Sized<M: meta {slot}> is {{ value: Int }};
            type Dimension<N> is protocol {{}};
            type Point is {{ value: Int }};
            implement<T> Dimension<Sized<{argument}>> for Point {{}}
        "#
            ),
            "protocol",
        );
    }
    rejects(
        r#"module applications;
        type Sized<M: meta Int> is { value: Int };
        type Dimension<N> is protocol {};
        type Point is { value: Int };
        implement<M: meta Int> Dimension<fn<M>() -> Sized<M>> for Point {}
    "#,
        "locally quantified type",
    );
}

#[test]
fn protocol_nested_arithmetic_keeps_exact_value_in_signature_wire_and_execution() {
    for argument in ["N + 2", "(N + 2)"] {
        let source = format!(
            r#"module applications;
            type Sized<M: meta Int> is {{ value: Int }};
            type Carrier<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Dimension<N: meta Int> is protocol extends Carrier<Sized<{argument}>> {{}};
            type Point is {{ value: Int }}; implement Dimension<3> for Point {{}}
            fn main() -> Int {{
                let point = Point {{ value: 0 }};
                let input: Sized<5> = Sized {{ value: 73 }};
                point.echo(input).value
            }}
        "#
        );
        let module = compile(&source).unwrap();
        let restored = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, restored] {
            let echo = module
                .functions
                .iter()
                .find(|function| {
                    module
                        .get_string(function.name)
                        .is_some_and(|name| name.ends_with("Point.echo"))
                })
                .unwrap();
            let sized = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some("Sized"))
                .unwrap()
                .id;
            let expected = verum_vbc::types::TypeRef::Instantiated {
                base: sized,
                args: vec![verum_vbc::types::TypeRef::ConstValue(5)],
            };
            assert_eq!(echo.return_type, expected);
            assert_eq!(echo.semantic_return.as_ref(), Some(&expected));
            assert_eq!(
                echo.semantic_params.as_ref().unwrap()[1].as_ref(),
                Some(&expected)
            );
            let entry = module.find_function_by_name("applications.main").unwrap();
            assert_eq!(
                Interpreter::new(Arc::new(module))
                    .execute_function(entry)
                    .unwrap()
                    .try_as_i64(),
                Some(73)
            );
        }
    }
}

#[test]
fn protocol_nested_symbolic_arithmetic_cannot_invent_a_generic_slot() {
    let error = compile(
        r#"module applications;
        type Sized<M: meta Int> is { value: Int };
        type Dimension<N> is protocol {};
        type Point<T, M: meta Int> is { value: T };
        implement<T, M: meta Int> Dimension<Sized<M + 2>> for Point<T, M> {}
    "#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("exactly representable"),
        "{error}"
    );
}

#[test]
fn source_value_environment_lifts_under_local_type_quantifiers() {
    let module = compile(
        r#"module applications;
        type Sized<M: meta Int> is { value: Int };
        type Carrier<T> is protocol { fn signature(&self, value: T) -> Int { 1 } };
        type Dimension<N: meta Int> is protocol extends Carrier<fn<R>(Sized<N + 2>, R) -> R> {};
        type Point is { value: Int }; implement Dimension<3> for Point {}
    "#,
    )
    .unwrap();
    let restored = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    for module in [module, restored] {
        let signature = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name.ends_with("Point.signature"))
            })
            .unwrap();
        let verum_vbc::types::TypeRef::Rank2Function {
            params,
            return_type,
            ..
        } = &signature.params[1].type_ref
        else {
            panic!("missing rank2 function: {:?}", signature.params);
        };
        assert!(
            matches!(&params[0], verum_vbc::types::TypeRef::Instantiated { args, .. }
            if args == &[verum_vbc::types::TypeRef::ConstValue(5)])
        );
        assert_eq!(
            params[1],
            verum_vbc::types::TypeRef::Generic(verum_vbc::types::TypeParamId(0))
        );
        assert_eq!(return_type.as_ref(), &params[1]);
    }
}

#[test]
fn protocol_value_carriers_cannot_be_invented_or_refinements_erased() {
    for carrier in ["Bool", "Text", "i32", "Int{> 0}"] {
        rejects(
            &format!(
                r#"module applications;
            type Dimension<N: meta {carrier}> is protocol {{}};
            type Point is {{ value: Int }}; implement Dimension<1> for Point {{}}
        "#
            ),
            if carrier.contains('{') {
                "refinement witness"
            } else {
                "representation"
            },
        );
    }
}

#[test]
fn protocol_values_reject_overflow_wrong_scalar_and_literal_suffix() {
    for argument in [
        "true",
        "\"text\"",
        "9223372036854775807 + 1",
        "1 / 0",
        "1i32",
    ] {
        let source = format!(
            r#"module applications;
            type Dimension<N: meta Int> is protocol {{}};
            type Point is {{ value: Int }}; implement Dimension<{argument}> for Point {{}}
        "#
        );
        let error = compile(&source).unwrap_err();
        assert!(
            error.to_string().contains("applications.Dimension"),
            "{error}"
        );
    }
}

#[test]
fn protocol_application_without_source_header_cannot_accept_arguments() {
    let ast = Parser::new(
        r#"module provider;
        public type Dimension<N: meta Int> is protocol { fn required(&self) -> Int { 1 } };
    "#,
    )
    .parse_module()
    .unwrap();
    let mut provider = VbcCodegen::with_config(CodegenConfig::new("provider"));
    provider.collect_unit_declarations(&[&ast]).unwrap();
    // A legacy bare compatibility view is not the declaration-owned header.
    let mut headers = provider.export_protocols();
    assert!(headers.remove("provider.Dimension").is_some());
    assert!(headers.contains_key("Dimension"));
    let receiver = Parser::new(
        r#"module consumer;
        mount provider.Dimension;
        type Point is { value: Int };
        implement Dimension<true> for Point { fn required(&self) -> Int { 1 } }
    "#,
    )
    .parse_module()
    .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.import_protocols(&headers);
    let result = (|| {
        codegen.collect_unit_declarations(&[&receiver])?;
        codegen.resolve_pending_imports();
        codegen
            .compile_default_methods_with_policy(verum_vbc::codegen::ItemFailurePolicy::Strict)?;
        codegen.compile_unit_items(&[&receiver], verum_vbc::codegen::ItemFailurePolicy::Strict)?;
        Ok::<(), verum_vbc::codegen::CodegenError>(())
    })();
    assert!(
        result.is_err(),
        "compatibility header accepted an unchecked value argument"
    );
    let error = result.err().unwrap();
    assert!(
        error.to_string().contains("protocol application")
            && error.to_string().contains("declaration"),
        "{error}"
    );
}
