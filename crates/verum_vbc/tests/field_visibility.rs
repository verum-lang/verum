//! T1714: parsed declaration visibility must survive module serialization.
#![cfg(feature = "codegen")]
#[path = "fixtures/field_visibility.rs"]
mod fixture;

use verum_ast::{ItemKind, Visibility as AstVisibility, decl::TypeDeclBody};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, module::VbcModule, types::Visibility};

fn source_and_wire(field_name: &str, source_visibility: AstVisibility) -> [VbcModule; 2] {
    let parsed = Parser::new(fixture::SOURCE)
        .parse_module()
        .expect("fixture grammar");
    let fields = parsed
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) => match &declaration.body {
                TypeDeclBody::Record(fields) => Some(fields),
                _ => None,
            },
            _ => None,
        })
        .expect("declared record");
    let declared = fields
        .iter()
        .find(|field| field.name.name == field_name)
        .expect("source field");
    assert_eq!(
        declared.visibility, source_visibility,
        "parser source authority"
    );
    let source = VbcCodegen::new()
        .compile_module(&parsed)
        .expect("source lowering");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).expect("encode module"),
    )
    .expect("decode module");
    [source, wire]
}

fn coarse(field_name: &str, source_visibility: AstVisibility, expected: Visibility) {
    let mut failures: List<Text> = List::new();
    for (route, module) in ["source", "wire"]
        .into_iter()
        .zip(source_and_wire(field_name, source_visibility))
    {
        let descriptor = module
            .types
            .iter()
            .find(|descriptor| {
                descriptor
                    .origin_module
                    .and_then(|id| module.get_string(id))
                    == Some(fixture::OWNER)
                    && module
                        .get_string(descriptor.name)
                        .is_some_and(|name| name.rsplit('.').next() == Some("Vault"))
            })
            .expect("exact declared owner");
        let field = descriptor
            .fields
            .iter()
            .find(|field| module.get_string(field.name) == Some(field_name))
            .expect("emitted field");
        if field.visibility != expected {
            failures.push(
                format!(
                    "{route} {field_name}: expected {expected:?}, found {:?}",
                    field.visibility
                )
                .into(),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn public_field_keeps_public_coarse_visibility() {
    coarse("visible", AstVisibility::Public, Visibility::Public);
}
#[test]
fn implicit_private_field_is_not_promoted() {
    coarse(
        "implicit_private",
        AstVisibility::Private,
        Visibility::Private,
    );
}
#[test]
fn explicit_private_field_is_not_promoted() {
    coarse(
        "explicit_private",
        AstVisibility::Private,
        Visibility::Private,
    );
}
#[test]
fn cog_field_keeps_cog_coarse_visibility() {
    coarse("cog_only", AstVisibility::PublicCrate, Visibility::Cog);
}
#[test]
fn parent_field_has_conservative_coarse_visibility() {
    coarse(
        "parent_only",
        AstVisibility::PublicSuper,
        Visibility::Private,
    );
}
#[test]
fn internal_field_has_conservative_coarse_visibility() {
    coarse(
        "internal_only",
        AstVisibility::Internal,
        Visibility::Private,
    );
}
#[test]
fn protected_field_has_conservative_coarse_visibility() {
    coarse(
        "protected_only",
        AstVisibility::Protected,
        Visibility::Private,
    );
}
