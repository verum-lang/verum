use verum_common::{List, Map, Text};
use verum_types::computational_properties::{
    ComputationalProperty, PropertyRequirement, PropertySet,
};
use verum_types::ty::{Type, TypeVar};
use verum_types::type_exporter::{
    ExportedType, ModuleExports, TypeDefinition, TypeExporter, deserialize_module_exports,
    serialize_module_exports,
};

fn snapshot(ty: Type) -> ModuleExports {
    ModuleExports {
        path: Text::from("sample"),
        types: vec![ExportedType {
            name: Text::from("Callback"),
            type_params: List::new(),
            definition: TypeDefinition::Alias { target: ty },
            is_public: true,
            doc: None,
        }]
        .into(),
        ..Default::default()
    }
}
fn callable(requirement: PropertyRequirement, properties: Option<PropertySet>) -> Type {
    Type::Function {
        params: vec![Type::Int].into(),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: List::new(),
        property_requirement: requirement,
        properties,
    }
}
fn roundtrip(ty: Type) {
    let bytes = serialize_module_exports(&snapshot(ty.clone())).expect("supported snapshot");
    let restored = deserialize_module_exports(&bytes).expect("v2 replay");
    let TypeDefinition::Alias { target } = &restored.types[0].definition else {
        panic!("alias")
    };
    assert_eq!(&ty, target);
}
#[test]
fn exact_callable_contract_and_known_or_unknown_footprint() {
    let asynchronous = PropertySet::single(ComputationalProperty::Async);
    for requirement in [
        PropertyRequirement::Unrestricted,
        PropertyRequirement::Excludes(asynchronous.clone()),
        PropertyRequirement::AtMost(asynchronous.clone()),
    ] {
        for observed in [None, Some(PropertySet::pure()), Some(asynchronous.clone())] {
            roundtrip(callable(requirement.clone(), observed));
        }
    }
}
#[test]
fn nested_callable_and_qualified_nominal_identity_survive() {
    let inner = callable(PropertyRequirement::source_callable(true), None);
    roundtrip(Type::Reference {
        mutable: true,
        inner: Box::new(Type::Tuple(vec![inner.clone()].into())),
    });
    roundtrip(Type::Named {
        path: verum_ast::ty::Path::new(
            vec![
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "owner",
                    verum_ast::Span::dummy(),
                )),
                verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(
                    "Callback",
                    verum_ast::Span::dummy(),
                )),
            ]
            .into(),
            verum_ast::Span::dummy(),
        ),
        args: vec![inner].into(),
    });
}
#[test]
fn unsupported_local_binders_are_never_erased_even_when_nested() {
    for ty in [
        Type::Var(TypeVar::new(42)),
        Type::Forall {
            vars: vec![TypeVar::new(42)].into(),
            body: Box::new(Type::Int),
        },
        Type::Meta {
            name: "N".into(),
            ty: Box::new(Type::Int),
            refinement: None,
            value: None,
        },
    ] {
        let nested = Type::Reference {
            mutable: false,
            inner: Box::new(ty),
        };
        assert!(serialize_module_exports(&snapshot(nested)).is_err());
    }
    let mut local = callable(PropertyRequirement::source_callable(true), None);
    if let Type::Function { type_params, .. } = &mut local {
        type_params.push(verum_types::context::TypeParam::new(
            "T",
            verum_ast::Span::dummy(),
        ));
    }
    assert!(serialize_module_exports(&snapshot(local)).is_err());
}
#[test]
fn stale_truncated_and_trailing_wire_data_are_refused() {
    let bytes = serialize_module_exports(&snapshot(callable(
        PropertyRequirement::source_callable(true),
        None,
    )))
    .unwrap();
    for end in 0..bytes.len() {
        assert!(
            deserialize_module_exports(&bytes.as_slice()[..end]).is_none(),
            "prefix {end}"
        );
    }
    let mut stale = bytes.clone();
    stale[4] = 1;
    assert!(deserialize_module_exports(&stale).is_none());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(deserialize_module_exports(&trailing).is_none());
}
#[test]
fn source_preflight_refuses_contracts_before_syntactic_lowering() {
    for source in [
        "module sample; pub fn run(f: fn(Int) -> Int using [Trace]) -> Int { f(1) }",
        "module sample; pub fn run(f: fn<T>(T) -> T) -> Int { 1 }",
        "module sample; pub fn run<T>(x: T) -> T { x }",
        "module sample; pub fn run() { 41 }",
        "module sample; pub fn run(x: Int) -> Int requires x > 0 { x }",
        "module sample; pub type Indexed<N: meta Int> is { value: Int };",
    ] {
        let module = verum_fast_parser::Parser::new(source)
            .parse_module()
            .expect(source);
        let methods = Map::new();
        let mut exporter = TypeExporter::new(&methods);
        assert!(
            exporter.export_module(&module, "sample").is_err(),
            "{source}"
        );
    }
}
#[test]
fn supported_source_async_callable_is_not_unit_or_sync() {
    let module = verum_fast_parser::Parser::new(
        "module sample; pub fn run(f: async fn(Int) -> Int) -> Int { f(1) }",
    )
    .parse_module()
    .unwrap();
    let methods = Map::new();
    let mut exporter = TypeExporter::new(&methods);
    let bytes =
        serialize_module_exports(exporter.export_module(&module, "sample").unwrap()).unwrap();
    let restored = deserialize_module_exports(&bytes).unwrap();
    let Type::Function {
        property_requirement,
        properties,
        return_type,
        ..
    } = &restored.functions[0].params[0].ty
    else {
        panic!("callable retained")
    };
    assert_eq!(
        *property_requirement,
        PropertyRequirement::source_callable(true)
    );
    assert!(properties.is_none());
    assert_eq!(**return_type, Type::Int);
}

#[test]
fn context_identity_is_refused_instead_of_using_runtime_type_ids() {
    let mut contextual = callable(PropertyRequirement::source_callable(true), None);
    if let Type::Function { contexts, .. } = &mut contextual {
        *contexts = Some(verum_types::di::ContextExpr::variable(TypeVar::new(19)));
    }
    assert!(serialize_module_exports(&snapshot(contextual)).is_err());
}
#[test]
fn malformed_boolean_and_callable_contract_are_refused() {
    let bytes =
        serialize_module_exports(&snapshot(callable(PropertyRequirement::Unrestricted, None)))
            .unwrap();
    let mut malformed = bytes.clone();
    // After the alias visibility and absent documentation, three empty section counts remain.
    let trailing_sections = 3 * std::mem::size_of::<u32>();
    let flag = malformed.len() - trailing_sections - 2;
    assert_eq!(malformed[flag], 1);
    malformed[flag] = 2;
    assert!(deserialize_module_exports(&malformed).is_none());
    let mut contract = bytes;
    let at = contract
        .as_slice()
        .windows(b"Unrestricted".len())
        .position(|s| s == b"Unrestricted")
        .unwrap();
    contract[at] = b'X';
    assert!(deserialize_module_exports(&contract).is_none());
}
