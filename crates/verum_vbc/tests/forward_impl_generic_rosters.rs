#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenErrorKind, VbcCodegen},
    types::{CbgrTier, Mutability, TypeParamId, TypeRef},
};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new("forward"))
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}

fn roundtrip(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).unwrap(),
    )
    .unwrap()
}

fn method<'a>(module: &'a VbcModule, suffix: &str) -> &'a verum_vbc::FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == suffix || name.ends_with(&format!(".{suffix}")))
        })
        .unwrap_or_else(|| panic!("missing {suffix}"))
}

fn execute(module: VbcModule, suffix: &str) -> i64 {
    let entry = method(&module, suffix).id;
    verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .unwrap()
        .try_as_i64()
        .unwrap()
}

#[test]
fn bare_impls_inherit_the_same_roster_before_and_after_the_type() {
    let declaration = "type Cell<Z, A> is { first: Z, second: A };";
    let implementation = "implement Cell<Z, A> { fn pick(&self, value: A) -> A { value } }";
    for before in [true, false] {
        let source = if before {
            format!("{declaration} {implementation}")
        } else {
            format!("{implementation} {declaration}")
        } + r#"
            fn main() -> Int {
                let cell: Cell<Int, Bool> = Cell { first: 5, second: false };
                if cell.pick(true) { 73 } else { 0 }
            }
        "#;
        let module = compile(&source);
        for module in [module.clone(), roundtrip(&module)] {
            let pick = method(&module, "Cell.pick");
            assert_eq!(pick.return_type, TypeRef::Generic(TypeParamId(1)));
            assert_eq!(pick.semantic_return, Some(TypeRef::Generic(TypeParamId(1))));
            let names: Vec<_> = pick
                .type_params
                .iter()
                .map(|parameter| (module.get_string(parameter.name).unwrap(), parameter.id))
                .collect();
            assert_eq!(names, vec![("Z", TypeParamId(0)), ("A", TypeParamId(1))]);
            assert_eq!(execute(module, "main"), 73);
        }
    }
}

#[test]
fn forward_marker_implementation_requires_no_fake_receiver_parameter() {
    for before in [true, false] {
        let declaration = "type Cell<T> is { value: T };";
        let implementation = "implement Send for Cell<T> where T: Send {}";
        let source = "type Send is protocol {};".to_owned()
            + &if before {
                format!("{declaration} {implementation}")
            } else {
                format!("{implementation} {declaration}")
            };
        let module = compile(&source);
        for module in [module.clone(), roundtrip(&module)] {
            let cell = module
                .types
                .iter()
                .find(|ty| module.get_string(ty.name) == Some("Cell"))
                .unwrap();
            assert_eq!(cell.type_params.len(), 1);
            assert_eq!(module.get_string(cell.type_params[0].name), Some("T"));
            assert_eq!(cell.protocols.len(), 1);
            let protocol = module
                .types
                .iter()
                .find(|ty| ty.id.0 == cell.protocols[0].protocol.0)
                .unwrap();
            assert_eq!(module.get_string(protocol.name), Some("Send"));
        }
    }
}

fn mounted(source: &str) -> Result<VbcModule, verum_vbc::codegen::CodegenError> {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(
        core.join("provider.vr"),
        r#"
        module core.provider;
        public type Cell<Z, A> is { first: Z, second: A };
    "#,
    )
    .unwrap();
    let path = core.join("consumer.vr");
    std::fs::write(&path, source).unwrap();
    VbcCodegen::with_config(CodegenConfig::new("core.consumer")).compile_module_with_mounts(
        &Parser::new(source).parse_module().unwrap(),
        path.to_str().unwrap(),
        core.to_str().unwrap(),
    )
}

#[test]
fn a_same_leaf_declaration_cannot_supply_an_imported_receivers_roster() {
    let module = mounted(
        r#"
        module core.consumer;
        mount core.provider.Cell as ImportedCell;
        type Marker is protocol {};
        implement Marker for ImportedCell<Z, A> {}
        implement ImportedCell<Z, A> { fn pick(&self, value: A) -> A { value } }
        type Cell<Other> is { wrong: Other };
        fn main() -> Int {
            let cell: ImportedCell<Int, Bool> = ImportedCell { first: 5, second: false };
            if cell.pick(true) { 73 } else { 0 }
        }
    "#,
    )
    .unwrap();
    for module in [module.clone(), roundtrip(&module)] {
        let selected = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Cell")
                    && ty.origin_module.and_then(|id| module.get_string(id))
                        == Some("core.provider")
            })
            .unwrap();
        let decoy = module
            .types
            .iter()
            .find(|ty| {
                module.get_string(ty.name) == Some("Cell")
                    && ty.origin_module.and_then(|id| module.get_string(id))
                        == Some("core.consumer")
            })
            .unwrap();
        assert_eq!(selected.protocols.len(), 1);
        assert!(decoy.protocols.is_empty());
        let pick = method(&module, "ImportedCell.pick");
        assert_eq!(pick.return_type, TypeRef::Generic(TypeParamId(1)));
        assert_eq!(pick.semantic_return, Some(TypeRef::Generic(TypeParamId(1))));
        assert_eq!(execute(module, "main"), 73);
    }
}

#[test]
fn an_undeclared_argument_cannot_be_inherited_from_another_nominal_owner() {
    let error = mounted(
        r#"
        module core.consumer;
        mount core.provider.Cell as ImportedCell;
        type Cell<Other> is { wrong: Other };
        implement ImportedCell<Other, A> { fn pick(&self, value: Other) -> Other { value } }
    "#,
    )
    .unwrap_err();
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("unresolved nominal type 'Other'"),
        "{error}"
    );
    let source = Parser::new("implement Cell<Missing> { fn pick(&self, value: Missing)->Missing { value } } type Cell<T> is { value:T };")
        .parse_module().unwrap();
    let error = VbcCodegen::new().compile_module(&source).unwrap_err();
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(_)),
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
fn inherited_meta_parameters_keep_their_value_kind_and_slot() {
    for before in [true, false] {
        let declaration = "type Sized<Z, N: meta Int> is { value: Z };";
        let implementation = "implement Sized<Z, N> { fn size(&self) -> Int { N } }";
        let source = if before {
            format!("{declaration} {implementation}")
        } else {
            format!("{implementation} {declaration}")
        } + "fn main()->Int { let value: Sized<Bool, 73> = Sized { value: true }; value.size() }";
        let module = compile(&source);
        for module in [module.clone(), roundtrip(&module)] {
            let size = method(&module, "Sized.size");
            assert_eq!(
                size.type_params
                    .iter()
                    .map(|parameter| (module.get_string(parameter.name).unwrap(), parameter.id))
                    .collect::<Vec<_>>(),
                vec![("Z", TypeParamId(0)), ("N", TypeParamId(1))]
            );
            assert_eq!(execute(module, "main"), 73);
        }
    }
}

#[test]
fn nested_implementation_cannot_capture_the_enclosing_receiver_roster() {
    let module = compile(
        r#"
        type Local<Z, A> is { first: Z, second: A };
        type Outer<A> is { value: A };
        implement<A> Outer<A> {
            fn probe(&self) -> Int {
                implement<Z, A> Local<Z, A> {
                    fn pick(&self, value: A) -> A { value }
                }
                let local: Local<Bool, Int> = Local { first: false, second: 7 };
                local.pick(73)
            }
        }
        fn main() -> Int {
            let outer: Outer<Int> = Outer { value: 1 };
            outer.probe()
        }
    "#,
    );
    for module in [module.clone(), roundtrip(&module)] {
        let local = module
            .types
            .iter()
            .find(|ty| module.get_string(ty.name) == Some("Local"))
            .unwrap()
            .id;
        let pick = method(&module, "Local.pick");
        let expected_receiver = TypeRef::Reference {
            inner: Box::new(TypeRef::Instantiated {
                base: local,
                args: vec![
                    TypeRef::Generic(TypeParamId(0)),
                    TypeRef::Generic(TypeParamId(1)),
                ],
            }),
            tier: CbgrTier::Tier0,
            mutability: Mutability::Immutable,
        };
        assert_eq!(
            pick.semantic_params.as_ref().unwrap()[0],
            Some(expected_receiver)
        );
        assert_eq!(pick.return_type, TypeRef::Generic(TypeParamId(1)));
        assert_eq!(pick.semantic_return, Some(TypeRef::Generic(TypeParamId(1))));
        assert_eq!(execute(module, "main"), 73);
    }
}
