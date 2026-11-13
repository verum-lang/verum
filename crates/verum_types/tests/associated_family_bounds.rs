//! Source and archive declarations retain the full associated-family telescope.
use std::sync::Arc;
use verum_ast::{
    ItemKind, TypeDecl, TypeDeclBody,
    decl::ProtocolItemKind,
    ty::{GenericArg, TypeBoundKind},
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{Type, TypeChecker, core_metadata::*};

const SOURCE: &str = "public type Family<A> is protocol { type Rows<R>: Stream<A, Item = R>; fn select<R>(&self, value: R) -> R; };";
fn declaration(source: &str) -> TypeDecl {
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("type declaration")
    };
    declaration.clone()
}

#[test]
fn conditional_family_arguments_use_the_callers_associated_equality() {
    use verum_ast::{Ident, Path, PathSegment, Span};
    use verum_types::{
        protocol::{ProtocolBound, WhereClause},
        ty::TypeVar,
    };

    let source = r#"
        type Channel is protocol { type Item; };
        type Other is protocol { type Item; };
        type Family is protocol { type PairWith<K = Int>; };
        type Envelope<F> is { value: F };
        implement<F, E> Family for Envelope<F> where F: Channel<Item = E> {
            type PairWith<K> = (E, K);
        }
    "#;
    let module = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("families");
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    assert!(checker.prepare_module_declarations(&module).is_empty());
    for item in &module.items {
        checker.check_item(item).unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
    let parameter = Type::Var(TypeVar::fresh());
    let path = |name: &str| {
        let span = Span::dummy();
        let segments = ["families", name].map(|part| PathSegment::Name(Ident::new(part, span)));
        Path::new(segments.into_iter().collect(), span)
    };
    let protocols = checker.protocol_checker.read();
    let receiver = Type::Named {
        path: path("Envelope"),
        args: List::from_iter([parameter.clone()]),
    };
    let premise = |name: &str| {
        let mut bound = ProtocolBound::positive(path(name), List::new());
        bound.associated_types.insert("Item".into(), Type::Bool);
        WhereClause { ty: parameter.clone(), bounds: List::from_iter([bound]) }
    };
    let member = Text::from("PairWith");
    let assumptions = [premise("Channel")];
    let applied = protocols.resolve_associated_family(&receiver, &member, &[Type::Char], &assumptions)
        .expect("the caller determines E").expect("declared family");
    assert_eq!(applied.result, Type::Tuple(List::from_iter([Type::Bool, Type::Char])));
    let omitted = protocols.resolve_omitted_family_arguments(&receiver, &member, &assumptions)
        .expect("omitting K preserves the same evidence").expect("declared default");
    assert_eq!(omitted.result, Type::Tuple(List::from_iter([Type::Bool, Type::Int])));
    for invalid in [&[][..], &[premise("Other")][..]] {
        assert!(protocols.resolve_associated_family(&receiver, &member, &[Type::Char], invalid).is_err());
        assert!(protocols.resolve_omitted_family_arguments(&receiver, &member, invalid).is_err());
    }
}

fn assert_telescope(checker: &TypeChecker, owner: &str, bound_owner: &str, shadowed: bool) {
    let registry = checker.protocol_checker.read();
    let protocol = registry
        .get_protocol(&owner.into())
        .expect("registered protocol");
    let associated = protocol.associated_types.get(&Text::from("Rows")).unwrap();
    assert_eq!(associated.binders.owner.len(), 1);
    assert_eq!(associated.binders.local.len(), 1);
    assert_ne!(associated.binders.owner[0].1, associated.binders.local[0]);
    assert_eq!(associated.bounds.len(), 1);
    let bound = &associated.bounds[0];
    let actual_owner: Vec<_> = bound
        .protocol
        .segments
        .iter()
        .map(|segment| {
            let verum_ast::ty::PathSegment::Name(name) = segment else {
                panic!("named protocol owner");
            };
            name.name.as_str()
        })
        .collect();
    assert_eq!(actual_owner, bound_owner.split('.').collect::<Vec<_>>());
    let verum_types::protocol::ProtocolConstraints::Declared {
        self_type,
        parameters,
        ..
    } = &protocol.constraints
    else {
        panic!("source declaration must retain its telescope");
    };
    assert_eq!(parameters[0], associated.binders.owner[0].1);
    assert_eq!(associated.binders.self_type, Some(*self_type));
    let as_type = |binder: &verum_types::context::GenericBinder| match binder {
        verum_types::context::GenericBinder::Type(variable) => Type::Var(*variable),
        _ => panic!("ordinary type binder"),
    };
    assert_eq!(bound.args[0], as_type(if shadowed {
        &associated.binders.local[0]
    } else { &associated.binders.owner[0].1 }));
    assert_eq!(
        bound.associated_types.get(&Text::from("Item")),
        Some(&as_type(&associated.binders.local[0]))
    );
}
#[test]
fn source_family_arguments_and_equalities_keep_independent_binders() {
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("provider");
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker
        .register_type_declaration(&declaration(
            "public type Stream<A> is protocol { type Item = A; };",
        ))
        .unwrap();
    checker
        .register_type_declaration(&declaration(SOURCE))
        .unwrap();
    assert_telescope(&checker, "provider.Family", "provider.Stream", false);
}
#[test]
fn family_parameter_shadows_owner_without_reusing_its_identity() {
    let mut checker = TypeChecker::new();
    checker.set_current_module_path("provider");
    checker.set_generic_associated_types_enabled(true);
    checker.register_primitives();
    checker
        .register_type_declaration(&declaration(
            "public type Stream<A> is protocol { type Item = A; };",
        ))
        .unwrap();
    checker
        .register_type_declaration(&declaration(
            "type Family<A> is protocol { type Rows<A>: Stream<A, Item = A>; };",
        ))
        .unwrap();
    assert_telescope(&checker, "provider.Family", "provider.Stream", true);
}
#[test]
fn complete_bound_survives_sidecar_and_eager_replay() {
    let source = declaration(SOURCE);
    let saved = verum_ast::source_signature::source_type_declaration(&source)
        .unwrap()
        .unwrap();
    let descriptor = TypeDescriptor {
        source_declaration: Some(saved.clone()),
        name: "Family".into(),
        module_path: "provider".into(),
        origin_module_path: None,
        generic_params: vec![GenericParam {
            name: "A".into(),
            bounds: List::new(),
            default: None,
            type_bounds: List::new(),
            pid: Some(0),
        }]
        .into(),
        kind: TypeDescriptorKind::Protocol {
            super_protocols: List::new(),
            associated_types: List::new(),
            required_methods: List::new(),
            default_methods: List::new(),
        },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
    };
    let mut stream = descriptor.clone();
    stream.name = "Stream".into();
    stream.source_declaration = verum_ast::source_signature::source_type_declaration(&declaration(
        "public type Stream<A> is protocol { type Item = A; };",
    ))
    .unwrap();
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
    metadata
        .type_declaration_order
        .push("provider.Stream".into());
    metadata.types.insert("provider.Stream".into(), stream);
    metadata
        .type_declaration_order
        .push("provider.Family".into());
    metadata.types.insert("provider.Family".into(), descriptor);
    let bytes = encode_metadata(&metadata).unwrap();
    let restored = decode_metadata(&bytes).unwrap();
    assert_eq!(
        restored
            .types
            .get(&Text::from("provider.Family"))
            .unwrap()
            .source_declaration
            .as_ref(),
        Some(&saved)
    );
    let restored = Arc::new(restored);
    let mut disabled = TypeChecker::new_with_core_eager(Arc::clone(&restored));
    assert!(disabled.take_deferred_errors().iter().any(|error| error.to_string().contains("generic_associated_types")));
    let mut checker = TypeChecker::with_minimal_context();
    checker.set_generic_associated_types_enabled(true);
    checker.load_core_eager(restored);
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostics().is_empty());
    assert_telescope(&checker, "provider.Family", "provider.Stream", false);
    let mut stale = bytes;
    stale[8..12].copy_from_slice(&(CORE_METADATA_VERSION - 1).to_le_bytes());
    assert!(decode_metadata(&stale).is_err());
}
#[test]
fn parser_and_pretty_printer_preserve_associated_equality() {
    let module = Parser::new(SOURCE).parse_module().unwrap();
    let rendered = verum_ast::pretty::PrettyPrinter::default_printer().format_module(&module);
    let parsed = Parser::new(&rendered).parse_module().unwrap();
    let ItemKind::Type(decl) = &parsed.items[0].kind else {
        panic!("type")
    };
    let TypeDeclBody::Protocol(body) = &decl.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Type { bounds, .. } = &body.items[0].kind else {
        panic!("family")
    };
    let TypeBoundKind::GenericProtocol(ty) = &bounds[0].kind else {
        panic!("generic bound")
    };
    let verum_ast::TypeKind::Generic { args, .. } = &ty.kind else {
        panic!("arguments")
    };
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[1], GenericArg::Binding(binding) if binding.name.as_str() == "Item"));
}
