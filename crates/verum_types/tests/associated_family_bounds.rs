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
fn assert_telescope(checker: &TypeChecker, owner: &str, shadowed: bool) {
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
    assert_eq!(bound.protocol.to_string(), "Stream");
    assert_eq!(
        bound.args[0],
        Type::Var(if shadowed {
            associated.binders.local[0]
        } else {
            associated.binders.owner[0].1
        })
    );
    assert_eq!(
        bound.associated_types.get(&Text::from("Item")),
        Some(&Type::Var(associated.binders.local[0]))
    );
}
#[test]
fn source_family_arguments_and_equalities_keep_independent_binders() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker
        .register_type_declaration(&declaration(SOURCE))
        .unwrap();
    assert_telescope(&checker, "Family", false);
}
#[test]
fn family_parameter_shadows_owner_without_reusing_its_identity() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker
        .register_type_declaration(&declaration(
            "type Family<A> is protocol { type Rows<A>: Stream<A, Item = A>; };",
        ))
        .unwrap();
    assert_telescope(&checker, "Family", true);
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
    let mut metadata = CoreMetadata {
        version: CORE_METADATA_VERSION,
        ..Default::default()
    };
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
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(restored));
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostics().is_empty());
    assert_telescope(&checker, "provider.Family", false);
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
