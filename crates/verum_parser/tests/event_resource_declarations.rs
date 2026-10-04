//! T1556: resource qualifiers must survive the public event-tree-to-AST route.
use verum_ast::{FileId, ItemKind, ResourceModifier, TypeDecl, TypeDeclBody, Visibility};
use verum_parser::{syntax_to_ast, EventBasedParser, Parser};

fn event_types(source: &str) -> Vec<TypeDecl> {
    let parsed = EventBasedParser::new().parse(source, FileId::new(17));
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let converted = syntax_to_ast(source, &parsed.syntax(), FileId::new(17));
    assert!(converted.errors.is_empty(), "{:?}", converted.errors);
    converted
        .module
        .items
        .into_iter()
        .filter_map(|item| match item.kind {
            ItemKind::Type(decl) => Some(decl),
            _ => None,
        })
        .collect()
}

#[test]
fn event_resource_qualifiers_preserve_record_declarations() {
    for (qualifier, modifier) in [
        ("affine", ResourceModifier::Affine),
        ("linear", ResourceModifier::Linear),
    ] {
        let source = format!("type {qualifier} Handle is {{ value: Int }};");
        let semantic = Parser::new(&source).parse_module().expect("valid source");
        let ItemKind::Type(reference) = &semantic.items[0].kind else {
            panic!("type declaration")
        };
        let types = event_types(&source);
        assert_eq!(types.len(), 1, "{source}");
        let decl = &types[0];
        assert_eq!(decl.name.name.as_str(), "Handle");
        assert_eq!(decl.resource_modifier, Some(modifier));
        assert_eq!(decl.resource_modifier, reference.resource_modifier);
        let TypeDeclBody::Record(fields) = &decl.body else {
            panic!("record body")
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name.name.as_str(), "value");
    }
}

#[test]
fn event_resource_preserves_public_generic_owner() {
    let types = event_types("public type affine Handle<T> is { value: T };");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].visibility, Visibility::Public);
    assert_eq!(types[0].generics.len(), 1);
    let verum_ast::ty::GenericParamKind::Type { name, .. } = &types[0].generics[0].kind else {
        panic!("type parameter")
    };
    assert_eq!(name.name.as_str(), "T");
    assert_eq!(types[0].resource_modifier, Some(ResourceModifier::Affine));
}

#[test]
fn event_ordinary_type_does_not_inherit_resource_modifier() {
    let types = event_types("type affine Handle is { id: Int }; type Plain is { id: Int };");
    assert_eq!(types.len(), 2);
    assert_eq!(types[1].name.name.as_str(), "Plain");
    assert_eq!(types[1].resource_modifier, None);
}

#[test]
fn event_type_resource_attributes_remain_on_their_declaration() {
    let types = event_types("@must_consume @opaque public type affine Handle is { id: Int }; type Plain is { id: Int };");
    assert_eq!(types.len(), 2);
    let attrs: Vec<_> = types[0]
        .attributes
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(attrs, ["must_consume", "opaque"]);
    assert_eq!(types[0].resource_modifier, Some(ResourceModifier::Affine));
    assert!(types[1].attributes.is_empty());
}

#[test]
fn event_unqualified_resource_attribute_is_preserved() {
    let types = event_types("@must_consume type Handle is { id: Int };");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].resource_modifier, None);
    assert_eq!(types[0].attributes.len(), 1);
    assert_eq!(types[0].attributes[0].name.as_str(), "must_consume");
}

#[test]
fn malformed_resource_declaration_has_a_diagnostic() {
    for source in [
        "type affine is { id: Int };",
        "type linear Broken { id: Int };",
        "type affine linear Broken is { id: Int };",
        "type affine Broken is;",
        "type linear Broken is",
    ] {
        assert!(
            Parser::new(source).parse_module().is_err(),
            "semantic parser must reject: {source}"
        );
        let parsed = EventBasedParser::new().parse(source, FileId::new(19));
        let converted = syntax_to_ast(source, &parsed.syntax(), FileId::new(19));
        assert!(
            !parsed.errors.is_empty() || !converted.errors.is_empty(),
            "malformed declaration disappeared silently: {source}"
        );
    }
}

#[test]
fn ordinary_record_control() {
    let types = event_types("type Plain is { id: Int };");
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name.name.as_str(), "Plain");
    assert_eq!(types[0].resource_modifier, None);
}

#[test]
fn lossless_public_api_preserves_resource_declaration() {
    let source = "@must_consume public type affine Handle<T> is { value: T };";
    let parsed = verum_parser::LosslessParser::new().parse(source, FileId::new(23));
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.module.items.len(), 1);
    let ItemKind::Type(decl) = &parsed.module.items[0].kind else {
        panic!("type declaration")
    };
    assert_eq!(decl.name.name.as_str(), "Handle");
    assert_eq!(decl.resource_modifier, Some(ResourceModifier::Affine));
    assert_eq!(decl.attributes[0].name.as_str(), "must_consume");
    assert_eq!(decl.span.file_id, FileId::new(23));
}

#[test]
fn malformed_resource_does_not_steal_next_type_or_its_attributes() {
    let source = "@must_consume type affine is { id: Int }; type Plain is { id: Int };";
    let parsed = EventBasedParser::new().parse(source, FileId::new(29));
    let converted = syntax_to_ast(source, &parsed.syntax(), FileId::new(29));
    assert!(!parsed.errors.is_empty() || !converted.errors.is_empty());
    let types: Vec<_> = converted
        .module
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Type(decl) => Some(decl),
            _ => None,
        })
        .collect();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0].name.name.as_str(), "Plain");
    assert_eq!(types[0].resource_modifier, None);
    assert!(types[0].attributes.is_empty());
}
