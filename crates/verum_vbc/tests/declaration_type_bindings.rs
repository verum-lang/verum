//! Lexical imports are part of an archived declaration's meaning.
use verum_common::{OrderedMap, Text};
use verum_vbc::{
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::{serialize_archive_member, serialize_module},
};

#[test]
fn scopes_roundtrip_without_other_extension_records() {
    let mut module = VbcModule::new("api".into());
    module.declaration_type_bindings.insert(
        "api".into(),
        OrderedMap::from_iter([
            ("Renamed".into(), Some("hidden.owner.Token".into())),
            ("ns".into(), Some("hidden.namespace".into())),
            ("Ambiguous".into(), None),
        ]),
    );
    module
        .declaration_type_bindings
        .insert("empty.scope".into(), OrderedMap::new());
    for bytes in [
        serialize_module(&module).unwrap(),
        serialize_archive_member(&module).unwrap(),
    ] {
        let decoded = deserialize_module(&bytes).unwrap();
        assert_eq!(decoded.header.version_minor, 32);
        assert_eq!(
            decoded.declaration_type_bindings,
            module.declaration_type_bindings
        );
        assert!(decoded.mount_aliases.is_empty());
        assert!(decoded.cfg_keys.is_empty());
    }
}

#[test]
fn duplicate_lexical_bindings_are_rejected_without_last_writer_wins() {
    let mut module = VbcModule::new("api".into());
    module.declaration_type_bindings.insert(
        "api".into(),
        OrderedMap::from_iter([
            ("first".into(), Some("left.Token".into())),
            ("other".into(), Some("right.Token".into())),
        ]),
    );
    let mut bytes = serialize_module(&module).unwrap();
    let position = bytes
        .windows(b"\"other\"".len())
        .position(|window| window == b"\"other\"")
        .unwrap();
    bytes[position..position + 7].copy_from_slice(b"\"first\"");
    let error = deserialize_module(&bytes).unwrap_err().to_string();
    assert!(error.contains("duplicate lexical binding"), "{error}");
}

#[test]
fn oversized_and_empty_scope_names_cannot_be_published() {
    let mut module = VbcModule::new("api".into());
    module
        .declaration_type_bindings
        .insert(Text::new(), OrderedMap::new());
    assert!(serialize_module(&module).is_err());
    module.declaration_type_bindings.clear();
    module.declaration_type_bindings.insert(
        "api".into(),
        OrderedMap::from_iter([("Token".into(), Some("a".repeat(4 * 1024 * 1024).into()))]),
    );
    assert!(serialize_module(&module).is_err());
}

#[cfg(feature = "codegen")]
mod codegen {
    use super::*;
    use verum_fast_parser::Parser;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    fn interface(source: &str) -> VbcModule {
        let ast = Parser::new(source).parse_module().unwrap();
        VbcCodegen::with_config(CodegenConfig::new("api"))
            .collect_declaration_interface(&[&ast])
            .unwrap()
    }

    #[test]
    fn private_renames_and_namespace_prefixes_belong_to_the_source_owner() {
        let module = interface(
            "module api; mount hidden.owner.Token as Renamed; mount hidden.space as ns; public type Carrier is { value: Type }; ",
        );
        let scope = module
            .declaration_type_bindings
            .get(&Text::from("api"))
            .unwrap();
        assert_eq!(
            scope.get(&Text::from("Renamed")).unwrap().as_deref(),
            Some("hidden.owner.Token")
        );
        assert_eq!(
            scope.get(&Text::from("ns")).unwrap().as_deref(),
            Some("hidden.space")
        );
        let bytes = serialize_module(&module).unwrap();
        assert_eq!(
            deserialize_module(&bytes)
                .unwrap()
                .declaration_type_bindings,
            module.declaration_type_bindings
        );
    }

    #[test]
    fn imported_and_completed_scopes_cannot_be_reinterpreted() {
        let source =
            "module api; mount left.Token as Local; public type Carrier is { value: Type };";
        let first = interface(source);
        let mut conflicting = first.clone();
        conflicting
            .declaration_type_bindings
            .get_mut(&Text::from("api"))
            .unwrap()
            .insert("Local".into(), Some("right.Token".into()));
        for (left, right) in [(&first, &conflicting), (&conflicting, &first)] {
            let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
            consumer.import_archive_module_types(left);
            consumer.import_archive_module_types(right);
            let error = consumer
                .finalize_module_from_state()
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("conflicting lexical type bindings"),
                "{error}"
            );
        }
        let mut completion = VbcCodegen::with_config(CodegenConfig::new("api"));
        completion.import_archive_module_types(&first);
        let ast = Parser::new(source).parse_module().unwrap();
        assert_eq!(
            completion
                .collect_declaration_interface(&[&ast])
                .unwrap()
                .declaration_type_bindings,
            first.declaration_type_bindings
        );

        let mut changed = VbcCodegen::with_config(CodegenConfig::new("api"));
        changed.import_archive_module_types(&first);
        // Keep the declaration's spans identical, so the scope receipt itself
        // must reject this change rather than AST span inequality doing so.
        let ast = Parser::new(&source.replace("left.Token", "evil.Token"))
            .parse_module()
            .unwrap();
        let error = changed
            .collect_declaration_interface(&[&ast])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("source completion changed lexical type bindings"),
            "{error}"
        );
    }
}
