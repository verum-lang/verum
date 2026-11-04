#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    types::{TypeId, TypeParamId, TypeRef},
};

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn nominal(module: &VbcModule, owner: &str, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some(name)
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
        })
        .unwrap_or_else(|| panic!("missing {owner}.{name}"))
        .id
}

fn method<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::FunctionDescriptor {
    let name = format!("Pair.{name}");
    module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|actual| actual == name || actual.ends_with(&format!(".{name}")))
        })
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn project() -> (VbcModule, VbcCodegen) {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(
        core.join("provider.vr"),
        r#"
        module core.provider;
        public type Packet<T> is { value: T };
        public type Extend<T> is protocol {
            fn echo(&self, value: T) -> T { let preserved: T = value; preserved }
            fn pack(&self, value: T) -> Packet<T> { Packet { value } }
            fn choose<T>(&self, value: T) -> T { value }
            fn quantified(&self, transform: fn<R>(T, R) -> T) -> Int { 1 }
            fn shadowed(&self, transform: fn<T>(T) -> T) -> Int { 1 }
        };
    "#,
    )
    .unwrap();
    let source = r#"
        module core.consumer;
        mount core.provider.Extend as ImportedExtend;
        type Extend<X> is protocol { fn wrong(&self, value: X) -> X { value } };
        type T is { wrong: Bool };
        type Packet<T> is { wrong: T };
        type Pair<K, V> is { first: K, second: V };
        implement<K, T> ImportedExtend<(T, K)> for Pair<K, T> {}
        fn main() -> Int {
            let pair: Pair<Bool, Int> = Pair { first: false, second: 7 };
            let value = pair.echo((17, true));
            let wrapped = pair.pack((25, false));
            let chosen = pair.choose<Int>(40);
            let (a, _) = value;
            let (b, _) = wrapped.value;
            a + b + chosen
        }
    "#;
    let path = core.join("consumer.vr");
    std::fs::write(&path, source).unwrap();
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("core.consumer"));
    let module = codegen
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), core.to_str().unwrap())
        .unwrap();
    (module, codegen)
}

#[test]
fn inherited_application_uses_exact_argument_in_signature_body_and_wire() {
    let (module, codegen) = project();
    let exports = codegen.export_functions();
    for module in [module.clone(), roundtrip(&module)] {
        let argument = TypeRef::Tuple(vec![
            TypeRef::Generic(TypeParamId(1)),
            TypeRef::Generic(TypeParamId(0)),
        ]);
        let echo = method(&module, "echo");
        assert_eq!(
            echo.type_params.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![TypeParamId(0), TypeParamId(1)]
        );
        assert_eq!(echo.return_type, argument);
        assert_eq!(echo.semantic_return.as_ref(), Some(&argument));
        assert_eq!(
            echo.semantic_params.as_ref().unwrap()[1],
            Some(argument.clone())
        );
        assert_eq!(
            echo.origin_module.and_then(|id| module.get_string(id)),
            Some("core.consumer")
        );
        assert_eq!(
            exports["core.consumer.Pair.echo"].return_type.as_ref(),
            Some(&argument)
        );
        let packed = TypeRef::Instantiated {
            base: nominal(&module, "core.provider", "Packet"),
            args: vec![argument],
        };
        assert_eq!(method(&module, "pack").return_type, packed);
        assert_eq!(
            method(&module, "pack").semantic_return.as_ref(),
            Some(&packed)
        );
        let entry = module.find_function_by_name("core.consumer.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(82));
    }
}

#[test]
fn inherited_application_avoids_method_and_rank2_capture() {
    let (module, _) = project();
    for module in [module.clone(), roundtrip(&module)] {
        let choose = method(&module, "choose");
        assert_eq!(choose.return_type, TypeRef::Generic(TypeParamId(0x8000)));
        assert_eq!(
            choose.semantic_return,
            Some(TypeRef::Generic(TypeParamId(0x8000)))
        );
        assert_eq!(
            choose.explicit_type_param_ids,
            vec![Some(TypeParamId(0x8000))]
        );
        let lifted = TypeRef::Tuple(vec![
            TypeRef::Generic(TypeParamId(2)),
            TypeRef::Generic(TypeParamId(1)),
        ]);
        assert_eq!(
            method(&module, "quantified")
                .semantic_params
                .as_ref()
                .unwrap()[1],
            Some(TypeRef::Rank2Function {
                is_async: false,
                type_param_count: 1,
                params: vec![lifted.clone(), TypeRef::Generic(TypeParamId(0))],
                return_type: Box::new(lifted),
                contexts: Default::default(),
            },)
        );
        assert_eq!(
            method(&module, "shadowed")
                .semantic_params
                .as_ref()
                .unwrap()[1],
            Some(TypeRef::Rank2Function {
                is_async: false,
                type_param_count: 1,
                params: vec![TypeRef::Generic(TypeParamId(0))],
                return_type: Box::new(TypeRef::Generic(TypeParamId(0))),
                contexts: Default::default(),
            },)
        );
    }
}

#[test]
fn implementation_application_and_default_receiver_share_binder_identity() {
    // The receiver's declaration uses A/B, while this implementation uses
    // Y/Z. Reordering its telescope cannot change the meaning of Read<Z>.
    for generics in ["Z, Y", "Y, Z"] {
        let source = format!(r#"
            module generic_order;
            type Read<T> is protocol {{
                fn echo(&self, value: T) -> T {{ value }}
                fn choose<Z>(&self, value: Z) -> Z {{ value }}
            }};
            type Pair<A, B> is {{ first: A, second: B }};
            implement<{generics}> Read<Z> for Pair<Y, Z> {{}}
            fn main() -> Int {{
                let pair: Pair<Bool, Int> = Pair {{ first: false, second: 3 }};
                pair.echo(17) + pair.choose<Int>(25)
            }}
        "#);
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        for module in [module.clone(), roundtrip(&module)] {
            let echo = method(&module, "echo");
            assert_eq!(echo.return_type, TypeRef::Generic(TypeParamId(1)), "{generics}");
            assert_eq!(echo.semantic_return, Some(TypeRef::Generic(TypeParamId(1))));
            assert_eq!(echo.semantic_params.as_ref().unwrap()[1], Some(TypeRef::Generic(TypeParamId(1))));
            let choose = method(&module, "choose");
            assert_eq!(choose.semantic_return, Some(TypeRef::Generic(TypeParamId(0x8000))));
            let entry = module.find_function_by_name("generic_order.main").unwrap();
            let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
                .execute_function(entry).unwrap();
            assert_eq!(result.try_as_i64(), Some(42));
        }
    }
}

#[test]
fn explicit_methods_share_the_receiver_without_capturing_local_binders() {
    for generics in ["Z, Y", "Y, Z"] {
        let source = format!(r#"
            module explicit_scope;
            type Pair<A, B> is {{ first: A, second: B }};
            implement<{generics}> Pair<Y, Z> {{
                fn echo(&self, value: Z) -> Z {{ value }}
                fn choose<Y>(&self, value: Y) -> Y {{ value }}
                fn quantified(&self, transform: fn<R>(Z, R) -> Z) -> Int {{ 1 }}
            }}
            fn main() -> Int {{
                let pair: Pair<Bool, Int> = Pair {{ first: false, second: 3 }};
                pair.echo(17) + pair.choose<Int>(25)
            }}
        "#);
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        for module in [module.clone(), roundtrip(&module)] {
            let pair = nominal(&module, "explicit_scope", "Pair");
            for name in ["echo", "choose", "quantified"] {
                let function = method(&module, name);
                let receiver = function.semantic_params.as_ref().unwrap()[0].as_ref().unwrap();
                let TypeRef::Reference { inner, .. } = receiver else { panic!("{receiver:?}") };
                assert_eq!(**inner, TypeRef::Instantiated {
                    base: pair, args: vec![TypeRef::Generic(TypeParamId(0)), TypeRef::Generic(TypeParamId(1))],
                }, "{generics}: {name}");
            }
            assert_eq!(method(&module, "echo").semantic_return, Some(TypeRef::Generic(TypeParamId(1))));
            assert_eq!(method(&module, "choose").semantic_return, Some(TypeRef::Generic(TypeParamId(0x8000))));
            let function = method(&module, "quantified");
            let TypeRef::Rank2Function { params, return_type, .. } = function.semantic_params.as_ref().unwrap()[1].as_ref().unwrap()
                else { panic!("{:?}", function.semantic_params) };
            assert_eq!(params, &[TypeRef::Generic(TypeParamId(2)), TypeRef::Generic(TypeParamId(0))]);
            assert_eq!(**return_type, TypeRef::Generic(TypeParamId(2)));
            let entry = module.find_function_by_name("explicit_scope.main").unwrap();
            let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
                .execute_function(entry).unwrap();
            assert_eq!(result.try_as_i64(), Some(42));
        }
    }
}

#[test]
fn protocol_application_cannot_drop_or_invent_positional_arguments() {
    for application in ["Extend", "Extend<Int, Bool>", "Extend<Missing>"] {
        let source = format!(
            r#"
            type Extend<T> is protocol {{ fn echo(&self, value: T) -> T {{ value }} }};
            type Pair is {{ value: Int }};
            implement {application} for Pair {{}}
        "#
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
        assert!(
            matches!(
                error.kind,
                verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
            ),
            "{error}"
        );
        let message = error.to_string();
        if application == "Extend<Missing>" {
            assert!(
                message.contains("unresolved nominal type 'Missing'"),
                "{message}"
            );
        } else {
            assert!(
                message.contains("protocol application") && message.contains("argument"),
                "{message}"
            );
        }
    }
}

#[test]
fn blanket_and_superprotocol_applications_compose_in_source_scope() {
    let source = r#"
        module applications;
        type Parent<A> is protocol { fn keep(self, value: A) -> A { value } };
        type Child<B> is protocol extends Parent<(Bool, B)> {};
        type Base is protocol {};
        implement<I: Base> Child<I> for I {}
        type Cell<U> is { value: U };
        implement<U> Base for Cell<U> {}
        fn main() -> Int {
            let first: Cell<Int> = Cell { value: 0 };
            let second: Cell<Int> = Cell { value: 73 };
            let (flag, result) = first.keep((true, second));
            if flag { result.value } else { 0 }
        }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("applications"))
        .compile_module(&ast)
        .unwrap();
    for module in [module.clone(), roundtrip(&module)] {
        let cell = TypeRef::Instantiated {
            base: nominal(&module, "applications", "Cell"),
            args: vec![TypeRef::Generic(TypeParamId(0))],
        };
        let expected = TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::BOOL), cell]);
        let keep = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name.ends_with("Cell.keep"))
            })
            .unwrap();
        assert_eq!(keep.return_type, expected);
        assert_eq!(keep.semantic_return.as_ref(), Some(&expected));
        assert_eq!(keep.type_params.len(), 1);
        let entry = module.find_function_by_name("applications.main").unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(73));
    }
}

#[test]
fn instantiated_associated_equalities_use_the_same_application() {
    let source = r#"
        type Gate<T> is protocol {
            type Item;
            fn allowed(&self) -> Int where Self.Item = T { 73 }
        };
        type Good is { value: Int };
        type Wrong is { value: Int };
        implement Gate<Int> for Good { type Item = Int; }
        implement Gate<Int> for Wrong { type Item = Bool; }
        fn main() -> Int { let good = Good { value: 0 }; good.allowed() }
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    for module in [module.clone(), roundtrip(&module)] {
        assert!(!module.functions.iter().any(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name.ends_with("Wrong.allowed"))
        }));
        let entry = module.find_function_by_name("main").unwrap();
        let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(73));
    }
}

#[test]
fn inherited_argument_cannot_capture_an_unrelated_implementor_parameter() {
    let source = r#"
        type Parent<T> is protocol { fn keep(self, value: T) -> T { value } };
        type Child is protocol extends Parent<Missing> {};
        type Cell<Missing> is { value: Missing };
        implement<Missing> Child for Cell<Missing> {}
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        matches!(
            error.kind,
            verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
        ),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Missing'"),
        "{error}"
    );
}

#[test]
fn conflicting_inherited_applications_cannot_be_collapsed_to_a_bare_name() {
    let source = r#"
        type Parent<T> is protocol { fn keep(self, value: T) -> T { value } };
        type Child is protocol extends Parent<Int> + Parent<Bool> {};
        type Cell is { value: Int };
        implement Child for Cell {}
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        matches!(
            error.kind,
            verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(_)
        ),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("conflicting inherited protocol application"),
        "{error}"
    );
}
