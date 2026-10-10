//! T1724: actual stdlib data declarations keep their field policy on archive wire.
#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Visibility as AstVisibility, decl::TypeDeclBody};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    archive::{ArchiveBuilder, read_archive, write_archive},
    codegen::VbcCodegen,
    module::VbcModule,
    types::{DeclaredFieldVisibility, Visibility},
};

const HANDLER: &str = include_str!("../../../core/net/weft/handler.vr");
const JSON: &str = include_str!("../../../core/encoding/json.vr");

fn data_field_policy(
    source: &str,
    owner: &str,
    record_name: &str,
    field_names: &[&str],
    public: bool,
) {
    let mut ast = Parser::new(source)
        .parse_module()
        .expect("complete source grammar");
    let declaration = ast
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) if declaration.name.as_str() == record_name => {
                Some(declaration.clone())
            }
            _ => None,
        })
        .expect("actual stdlib declaration");
    let TypeDeclBody::Record(fields) = &declaration.body else {
        panic!("{owner}.{record_name} must remain a record");
    };
    let ast_policy = if public {
        AstVisibility::Public
    } else {
        AstVisibility::Private
    };
    assert_eq!(
        fields.len(),
        field_names.len(),
        "complete source field roster"
    );
    for (field, expected_name) in fields.iter().zip(field_names) {
        assert_eq!(field.name.as_str(), *expected_name);
        assert_eq!(
            field.visibility, ast_policy,
            "source field {owner}.{record_name}.{expected_name}"
        );
    }

    // This is a declaration-policy gate, not a whole-module execution gate.
    // Parse the complete real source, then retain its exact record, owner and
    // mounts. No field type, visibility, generic parameter or descriptor is
    // manufactured; unrelated implementation bodies are outside this gate.
    ast.items.retain(|item| match &item.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Mount(_) => true,
        ItemKind::Type(declaration) => declaration.name.as_str() == record_name,
        _ => false,
    });
    assert!(
        ast.items.iter().any(|item| {
            matches!(&item.kind, ItemKind::Type(retained) if retained == &declaration)
        }),
        "the complete original declaration is retained unchanged"
    );
    let produced = VbcCodegen::new()
        .compile_module(&ast)
        .expect("declaration lowering");
    let mut builder = ArchiveBuilder::new();
    builder
        .add_module("bundle.plain_data", &produced, &[])
        .expect("archive member");
    let mut bytes: List<u8> = List::new();
    write_archive(&builder.finish(), &mut bytes).expect("archive encode");
    let archive = read_archive(std::io::Cursor::new(bytes.as_slice())).expect("archive decode");
    let decoded = archive
        .load_module("bundle.plain_data")
        .expect("decode member");
    for (route, module) in [("source", &produced), ("archive", &decoded)] {
        assert_policy(module, route, owner, record_name, field_names, public);
    }
}

fn assert_policy(
    module: &VbcModule,
    route: &str,
    owner: &str,
    record_name: &str,
    field_names: &[&str],
    public: bool,
) {
    let records: List<_> = module
        .types
        .iter()
        .filter(|record| {
            record.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
                && module
                    .get_string(record.name)
                    .is_some_and(|name| name.rsplit('.').next() == Some(record_name))
        })
        .collect();
    assert_eq!(records.len(), 1, "{route}: one exact declaring owner");
    let fields = &records[0].fields;
    assert_eq!(
        fields.len(),
        field_names.len(),
        "{route}: complete field roster"
    );
    let coarse = if public {
        Visibility::Public
    } else {
        Visibility::Private
    };
    let declared = if public {
        DeclaredFieldVisibility::Public
    } else {
        DeclaredFieldVisibility::Private
    };
    for (field, expected_name) in fields.iter().zip(field_names) {
        assert_eq!(module.get_string(field.name), Some(*expected_name));
        assert_eq!(
            field.visibility, coarse,
            "{route}: {owner}.{record_name}.{expected_name}"
        );
        assert_eq!(
            field.declared_visibility,
            Some(declared),
            "{route}: declared {owner}.{record_name}.{expected_name}"
        );
    }
}

#[test]
fn weft_request_transport_fields_are_public_in_source_and_archive() {
    data_field_policy(
        HANDLER,
        "core.net.weft.handler",
        "WeftRequest",
        &[
            "method",
            "path",
            "raw_query",
            "headers",
            "body",
            "path_params",
            "peer_addr",
        ],
        true,
    );
}

#[test]
fn json_error_diagnostic_fields_are_public_in_source_and_archive() {
    data_field_policy(
        JSON,
        "core.encoding.json",
        "JsonError",
        &["kind", "line", "column", "message"],
        true,
    );
}

#[test]
fn json_parser_state_remains_private_in_source_and_archive() {
    data_field_policy(
        JSON,
        "core.encoding.json",
        "Parser",
        &[
            "bytes",
            "pos",
            "line",
            "column",
            "depth",
            "reject_duplicate_keys",
        ],
        false,
    );
}

#[test]
fn weft_extracted_parameter_storage_remains_private_in_source_and_archive() {
    data_field_policy(
        HANDLER,
        "core.net.weft.handler",
        "PathParam",
        &["value"],
        false,
    );
}
