//! T1745: the declaration of the runtime Text record keeps its reserved carrier.
#![cfg(feature = "codegen")]

use verum_ast::ItemKind;
use verum_common::{List, Maybe, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind};

const TEXT_SOURCE: &str = include_str!("../../../core/text/text.vr");
const OWNER: &str = "core.text.text";

fn wire(module: &VbcModule) -> VbcModule {
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(module).expect("serialize archive module"),
    )
    .expect("decode archive module")
}

fn declaration(owner: &str) -> VbcModule {
    declaration_with_protocol(owner, "")
}

fn declaration_with_protocol(owner: &str, protocol: &str) -> VbcModule {
    // Keep the exact production record and module declaration. Only the
    // negative twins change the declaring module; the test methods are local.
    let source = TEXT_SOURCE.replace("module core.text.text;", &format!("module {owner};"));
    let mut ast = Parser::new(&source)
        .parse_module()
        .expect("parse production Text");
    ast.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(declaration) => declaration.name.name.as_str() == "Text",
        _ => false,
    });
    let methods = Parser::new(
        "implement Text { public fn archive_marker(&self) -> Int { 37 } \
         public fn archive_empty(&self) -> Unit {} }",
    )
    .parse_module()
    .expect("parse declared method controls");
    ast.items.extend(methods.items);
    ast.items.extend(
        Parser::new(protocol)
            .parse_module()
            .expect("parse record protocol")
            .items,
    );
    let module = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .expect("compile production Text declaration");
    let record = text(&module, Some(owner));
    assert_eq!(record.kind, TypeKind::Record);
    let fields: List<_> = record
        .fields
        .iter()
        .map(|field| module.get_string(field.name).unwrap())
        .collect();
    assert_eq!(fields.as_slice(), &["ptr", "len", "cap"]);
    module
}

fn text<'a>(module: &'a VbcModule, owner: Maybe<&str>) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| {
            module.get_string(ty.name) == Some("Text")
                && ty.origin_module.and_then(|id| module.get_string(id)) == owner
        })
        .expect("exact declaring Text descriptor")
}

fn method<'a>(module: &'a VbcModule, parent: TypeId, suffix: &str) -> &'a FunctionDescriptor {
    module
        .functions
        .iter()
        .find(|function| {
            function.parent_type == Some(parent) && module.get_string(function.name) == Some(suffix)
        })
        .expect("exact parent-owned method")
}

fn import_codegen(sources: &[&VbcModule]) -> VbcCodegen {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for source in sources {
        codegen.import_archive_module_types(source);
    }
    for (index, source) in sources.iter().enumerate() {
        let remap = source
            .functions
            .iter()
            .enumerate()
            .map(|(offset, function)| {
                (
                    function.id.0,
                    FunctionId(4000 + index as u32 * 1000 + offset as u32),
                )
            })
            .collect();
        codegen.merge_archive_function_bodies(source, &remap);
    }
    codegen
}

fn import(sources: &[&VbcModule]) -> VbcModule {
    import_codegen(sources)
        .finalize_module_from_state()
        .expect("finalize imported bodies")
}

fn assert_canonical(module: &VbcModule) {
    let record = text(module, Some(OWNER));
    assert_eq!(
        record.id,
        TypeId::TEXT,
        "canonical Text must agree with the runtime carrier"
    );
    let body = method(module, TypeId::TEXT, "Text.archive_marker");
    assert!(body.has_source_body && body.bytecode_length > 0);
    let empty = method(module, TypeId::TEXT, "Text.archive_empty");
    assert!(
        empty.has_source_body && empty.bytecode_length > 0,
        "a real empty method retains source-body authority"
    );
}

#[test]
fn parsed_canonical_text_record_and_methods_keep_runtime_identity_after_import() {
    let source = declaration(OWNER);
    // Source-local IDs are free to differ from the consumer runtime ABI.
    for source in [&source, &wire(&source)] {
        let imported = import(&[source]);
        assert_canonical(&imported);
        assert_canonical(&wire(&imported));
    }
}

#[test]
fn repeated_canonical_import_is_idempotent() {
    let source = wire(&declaration(OWNER));
    let imported = import(&[&source, &source]);
    assert_canonical(&imported);
    assert_eq!(
        imported
            .types
            .iter()
            .filter(|ty| {
                ty.origin_module.and_then(|id| imported.get_string(id)) == Some(OWNER)
                    && imported.get_string(ty.name) == Some("Text")
            })
            .count(),
        1
    );
}

fn paired(reverse: bool) {
    let canonical = wire(&declaration(OWNER));
    let foreign = wire(&declaration("foreign.text"));
    let sources = if reverse {
        [&foreign, &canonical]
    } else {
        [&canonical, &foreign]
    };
    let imported = wire(&import(&sources));
    assert_canonical(&imported);
    let foreign_id = text(&imported, Some("foreign.text")).id;
    assert_ne!(
        foreign_id,
        TypeId::TEXT,
        "same spelling does not establish the runtime declaration"
    );
    assert!(method(&imported, foreign_id, "Text.archive_marker").has_source_body);
}

#[test]
fn canonical_then_foreign_same_leaf_keeps_both_owners() {
    paired(false);
}

#[test]
fn foreign_then_canonical_same_leaf_keeps_both_owners() {
    paired(true);
}

#[test]
fn foreign_record_alone_cannot_claim_reserved_text_identity() {
    let source = wire(&declaration("foreign.text"));
    let imported = wire(&import(&[&source]));
    assert_ne!(text(&imported, Some("foreign.text")).id, TypeId::TEXT);
}

#[test]
fn bundle_name_without_declaring_owner_cannot_authorize_text_identity() {
    let mut source = wire(&declaration(OWNER));
    source.name = "core.text".into();
    source
        .types
        .iter_mut()
        .for_each(|ty| ty.origin_module = None);
    let imported = wire(&import(&[&source]));
    assert_ne!(text(&imported, None).id, TypeId::TEXT);
}

#[test]
fn archive_bundle_spelling_is_not_the_text_declaring_owner() {
    let source = wire(&declaration("core.text"));
    let imported = wire(&import(&[&source]));
    assert_ne!(text(&imported, Some("core.text")).id, TypeId::TEXT);
}

#[test]
fn bootstrap_dependency_keeps_canonical_text_identity() {
    let source = wire(&declaration(OWNER));
    let ast = Parser::new(
        "module consumer; mount core.text.text.Text; public fn relay(value: Text) -> Text { value }",
    )
    .parse_module()
    .expect("parse exact-owner bootstrap consumer");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    assert_eq!(
        codegen
            .import_bootstrap_nominal_dependencies(&[&ast], &[&source])
            .expect("import declared bootstrap dependency"),
        1
    );
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("collect bootstrap consumer");
    let imported = codegen
        .compile_function_bodies(&ast)
        .expect("compile bootstrap consumer");
    let record = text(&imported, Some(OWNER));
    assert_eq!(record.id, TypeId::TEXT);
    let relay = imported
        .functions
        .iter()
        .find(|function| {
            matches!(
                imported.get_string(function.name),
                Some("relay" | "consumer.relay")
            )
        })
        .expect("exact consumer function");
    assert_eq!(
        relay.return_type,
        verum_vbc::types::TypeRef::Concrete(TypeId::TEXT)
    );
    assert_eq!(
        relay.params[0].type_ref,
        verum_vbc::types::TypeRef::Concrete(TypeId::TEXT)
    );
    assert_eq!(text(&wire(&imported), Some(OWNER)).id, TypeId::TEXT);
}

const LOCAL_PROTOCOL: &str = "type DeclaredOnly is protocol { type Output; }; \
    implement DeclaredOnly for Text { type Output = Int; }";
const EXTENSION_SOURCE: &str = "module text_extension; type ExtensionOnly is protocol { type Output; }; \
    implement ExtensionOnly for Text { type Output = Byte; }";

fn extension() -> VbcModule {
    let ast = Parser::new(EXTENSION_SOURCE)
        .parse_module()
        .expect("parse Text extension");
    let module = VbcCodegen::with_config(CodegenConfig::new("text_extension"))
        .compile_module(&ast)
        .expect("compile scalar implementation carrier");
    let carrier = text(&module, Some("text_extension"));
    assert_eq!(carrier.kind, TypeKind::Primitive);
    assert_eq!(carrier.id, TypeId::TEXT);
    assert_eq!(carrier.protocols.len(), 1);
    module
}

fn assert_record_and_extensions(module: &VbcModule) {
    let record = text(module, Some(OWNER));
    assert_eq!(record.id, TypeId::TEXT);
    assert_eq!(record.kind, TypeKind::Record);
    let fields: List<_> = record
        .fields
        .iter()
        .map(|field| module.get_string(field.name).unwrap())
        .collect();
    assert_eq!(fields.as_slice(), &["ptr", "len", "cap"]);
    assert_eq!(
        module
            .types
            .iter()
            .filter(|ty| ty.id == TypeId::TEXT)
            .count(),
        1,
        "one authoritative runtime Text descriptor before and after the wire boundary"
    );
    let mut protocols: List<Text> = record
        .protocols
        .iter()
        .map(|implementation| {
            let protocol = module
                .get_type(TypeId(implementation.protocol.0))
                .expect("remapped protocol descriptor");
            let name = module.get_string(protocol.name).expect("protocol name");
            assert_eq!(implementation.associated_types.len(), 1);
            let (binding, value) = &implementation.associated_types[0];
            assert_eq!(module.get_string(*binding), Some("Output"));
            let expected = if name == "DeclaredOnly" {
                TypeId::INT
            } else {
                TypeId::BYTE
            };
            assert_eq!(*value, verum_vbc::types::TypeRef::Concrete(expected));
            name.into()
        })
        .collect();
    protocols.sort();
    assert_eq!(
        protocols.as_slice(),
        &[Text::from("DeclaredOnly"), Text::from("ExtensionOnly")]
    );
}

fn record_and_carrier(reverse: bool) {
    let declaration = declaration_with_protocol(OWNER, LOCAL_PROTOCOL);
    let carrier = extension();
    for (record, carrier) in [
        (&declaration, &carrier),
        (&wire(&declaration), &wire(&carrier)),
    ] {
        let mut codegen = if reverse {
            import_codegen(&[carrier, record])
        } else {
            import_codegen(&[record, carrier])
        };
        let imported = codegen
            .finalize_module()
            .expect("strict runtime finalization");
        assert!(codegen.verify_global_type_table_consistency().is_clean());
        assert_canonical(&imported);
        assert_record_and_extensions(&imported);
        assert_record_and_extensions(&wire(&imported));
    }
}

#[test]
fn canonical_record_then_primitive_extension_preserves_one_record_and_all_protocols() {
    record_and_carrier(false);
}

#[test]
fn primitive_extension_then_canonical_record_preserves_all_protocols() {
    record_and_carrier(true);
}

#[test]
fn source_extension_attaches_to_an_imported_canonical_record() {
    let source = wire(&declaration_with_protocol(OWNER, LOCAL_PROTOCOL));
    let ast = Parser::new(EXTENSION_SOURCE)
        .parse_module()
        .expect("parse source attachment");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("text_extension"));
    codegen.import_archive_module_types(&source);
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("collect Text implementation");
    let module = codegen
        .compile_function_bodies(&ast)
        .expect("finalize source attachment");
    assert_record_and_extensions(&module);
    assert_record_and_extensions(&wire(&module));
}

fn bootstrap_and_carrier(carrier_first: bool) {
    let source = wire(&declaration_with_protocol(OWNER, LOCAL_PROTOCOL));
    let carrier = wire(&extension());
    let ast = Parser::new(
        "module consumer; mount core.text.text.Text; public fn relay(value: Text) -> Text { value }"
    ).parse_module().expect("parse bootstrap declaration consumer");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    if carrier_first {
        codegen.import_archive_module_types(&carrier);
    }
    codegen
        .import_bootstrap_nominal_dependencies(&[&ast], &[&source])
        .expect("import canonical bootstrap dependency");
    if !carrier_first {
        codegen.import_archive_module_types(&carrier);
    }
    codegen
        .collect_unit_declarations(&[&ast])
        .expect("collect bootstrap consumer");
    let module = codegen
        .compile_function_bodies(&ast)
        .expect("finalize bootstrap coexistence");
    assert_record_and_extensions(&module);
    assert_record_and_extensions(&wire(&module));
}

#[test]
fn bootstrap_record_then_primitive_extension_preserves_one_descriptor() {
    bootstrap_and_carrier(false);
}

#[test]
fn primitive_extension_then_bootstrap_record_preserves_all_protocols() {
    bootstrap_and_carrier(true);
}
