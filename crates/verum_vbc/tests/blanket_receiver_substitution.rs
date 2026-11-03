#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    types::{TypeParamId, TypeRef},
};

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn method<'a>(module: &'a VbcModule, name: &str) -> &'a verum_vbc::FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|function| {
            module.get_string(function.name).is_some_and(|candidate| {
                candidate == name || candidate.ends_with(&format!(".{name}"))
            })
        })
        .expect("materialized blanket method")
}

fn receiver(module: &VbcModule) -> TypeRef {
    let id = module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Cell")
                && ty.origin_module.and_then(|owner| module.get_string(owner))
                    == Some("core.consumer")
        })
        .expect("actual receiver declaration")
        .id;
    TypeRef::Instantiated {
        base: id,
        args: vec![TypeRef::Generic(TypeParamId(0))],
    }
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
        public type Base is protocol {};
        public type Lifted is protocol {
            fn carry(self) -> Self;
            fn wrap(self) -> Wrapper<Self>;
            fn choose<I>(self, value: I) -> I;
            fn quantified(self, transform: fn<R>(Self, R) -> Self) -> Self;
            fn shadowed(self, transform: fn<I>(I) -> I) -> Self;
        };
        public type Wrapper<T> is { inner: T };
        type I is { wrong: Bool };
        implement<I: Base> Lifted for I {
            fn carry(self) -> I { self }
            fn wrap(self) -> Wrapper<I> { Wrapper { inner: self } }
            fn choose<I>(self, value: I) -> I { value }
            fn quantified(self, transform: fn<R>(I, R) -> I) -> I { self }
            fn shadowed(self, transform: fn<I>(I) -> I) -> I { self }
        }
    "#,
    )
    .unwrap();
    let source = r#"
        module core.consumer;
        mount core.provider.Base;
        type Wrapper<T> is { wrong: T };
        type Cell<T> is { value: T };
        implement<I> Base for Cell<I> {}
        fn main() -> Int {
            let cell: Cell<Int> = Cell { value: 73 };
            let carried = cell.carry();
            let other: Cell<Bool> = Cell { value: false };
            let packed: Cell<Int> = Cell { value: 9 };
            let wrapped = packed.wrap();
            if other.choose<Bool>(true) { carried.value + wrapped.inner.value } else { 0 }
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
fn blanket_receiver_is_substituted_without_an_extra_callable_witness() {
    let (module, codegen) = project();
    let exports = codegen.export_functions();
    for module in [module.clone(), roundtrip(&module)] {
        let expected = receiver(&module);
        for name in ["carry", "quantified", "shadowed"] {
            let function = method(&module, &format!("Cell.{name}"));
            assert_eq!(
                function
                    .type_params
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>(),
                vec![TypeParamId(0)]
            );
            assert_eq!(function.return_type, expected);
            assert_eq!(function.semantic_return.as_ref(), Some(&expected));
            assert_eq!(
                function.semantic_params.as_ref().unwrap()[0],
                Some(expected.clone())
            );
            assert_eq!(
                function.origin_module.and_then(|id| module.get_string(id)),
                Some("core.consumer")
            );
            assert_eq!(
                exports
                    .get(&format!("core.consumer.Cell.{name}"))
                    .unwrap()
                    .return_type
                    .as_ref(),
                Some(&expected)
            );
        }
        let entry = module.find_function_by_name("core.consumer.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(82));
    }
}

#[test]
fn blanket_substitution_avoids_method_and_quantified_binder_capture() {
    let (module, _) = project();
    for module in [module.clone(), roundtrip(&module)] {
        let actual = receiver(&module);
        let TypeRef::Instantiated { base, .. } = actual.clone() else {
            unreachable!()
        };
        let quantified = method(&module, "Cell.quantified");
        let lifted = TypeRef::Instantiated {
            base,
            args: vec![TypeRef::Generic(TypeParamId(1))],
        };
        let expected = TypeRef::Rank2Function {
            type_param_count: 1,
            params: vec![lifted.clone(), TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(lifted),
            contexts: Default::default(),
        };
        assert_eq!(
            quantified.semantic_params.as_ref().unwrap()[1],
            Some(expected)
        );
        let shadowed = method(&module, "Cell.shadowed");
        assert_eq!(
            shadowed.semantic_params.as_ref().unwrap()[1],
            Some(TypeRef::Rank2Function {
                type_param_count: 1,
                params: vec![TypeRef::Generic(TypeParamId(0))],
                return_type: Box::new(TypeRef::Generic(TypeParamId(0))),
                contexts: Default::default(),
            })
        );
        let chosen = method(&module, "Cell.choose");
        assert_eq!(chosen.return_type, TypeRef::Generic(TypeParamId(0x8000)));
        assert_eq!(
            chosen.semantic_return.as_ref(),
            Some(&TypeRef::Generic(TypeParamId(0x8000)))
        );
        assert_eq!(
            chosen.explicit_type_param_ids,
            vec![Some(TypeParamId(0x8000))]
        );
        let wrapper = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Wrapper")
                    && ty.origin_module.and_then(|owner| module.get_string(owner))
                        == Some("core.provider")
            })
            .unwrap()
            .id;
        let wrapped = TypeRef::Instantiated {
            base: wrapper,
            args: vec![actual],
        };
        let function = method(&module, "Cell.wrap");
        assert_eq!(function.return_type, wrapped);
        assert_eq!(function.semantic_return.as_ref(), Some(&wrapped));
    }
}

#[test]
fn an_uninstantiated_blanket_parameter_cannot_capture_an_implementor_binder() {
    let source = r#"
        type Base is protocol {};
        type Derived is protocol { fn identity(self) -> Self; };
        implement<I: Base, X> Derived for I { fn identity(self) -> I { self } }
        type Cell<X> is { value: X };
        implement<X> Base for Cell<X> {}
    "#;
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::new()
        .compile_module(&ast)
        .expect_err("the unrelated implementor X cannot instantiate blanket X");
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
            .contains("uninstantiated parameter other than its receiver"),
        "{error}"
    );
}
