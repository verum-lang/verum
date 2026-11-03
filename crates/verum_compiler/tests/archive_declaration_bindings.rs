//! Archived declaration ASTs replay their selected lexical type environment.
use verum_common::{List, Shared, Text};
use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_types::{core_metadata::CoreMetadata, TypeChecker};
use verum_vbc::{
    archive::ArchiveBuilder,
    codegen::{CodegenConfig, VbcCodegen},
};

fn metadata(generic: bool, imported: &str) -> CoreMetadata {
    metadata_with_alias(generic, imported, false)
}

fn metadata_with_alias(generic: bool, imported: &str, alias: bool) -> CoreMetadata {
    let root = Parser::new("module core; public module provider; public module decoy; public module prelude { public mount core.provider.Carrier; }").parse_module().unwrap();
    let args = if generic { "<T>" } else { "" };
    let value = if generic { "T" } else { "Int" };
    let provider_source = if alias {
        assert!(generic);
        "module core.provider; public type Cell<T> is { public value: T }; public type Carrier<T> is Cell<(&T, T)>;".to_owned()
    } else {
        format!("module core.provider; public type Carrier{args} is {{ public value: {value} }};")
    };
    let provider = Parser::new(&provider_source).parse_module().unwrap();
    let decoy = Parser::new(&format!(
        "module core.decoy; public type Carrier{args} is {{ public value: {value} }};"
    ))
    .parse_module()
    .unwrap();
    let consumer = Parser::new(&format!("module core.consumer; {imported} public type Holder{args} is {{ public value: Carrier{args}, public witness: Type }};")).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("core"))
        .collect_owned_declaration_interface(&[
            ("core", &root),
            ("core", &provider),
            ("core", &decoy),
            ("core", &consumer),
        ])
        .unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut builder = ArchiveBuilder::new();
    builder.add_module("core", &module, &[]).unwrap();
    let mut bytes = List::new();
    verum_vbc::archive::write_archive(&builder.finish(), &mut bytes).unwrap();
    let archive = verum_vbc::archive::read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap();
    let metadata = archive_to_core_metadata(&archive).unwrap();
    verum_types::core_metadata::decode_metadata(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap(),
    )
    .unwrap()
}

fn errors(metadata: &CoreMetadata, source: &str, eager: bool) -> List<Text> {
    let mut registry = verum_modules::ModuleRegistry::new();
    for (index, owner) in ["core.provider", "core.consumer", "core.decoy"]
        .iter()
        .enumerate()
    {
        registry.register(verum_modules::ModuleInfo::new(
            verum_modules::ModuleId::new(index as u32),
            verum_modules::ModulePath::from_str(owner),
            Parser::new("").parse_module().unwrap(),
            verum_ast::FileId::new(index as u32),
            Text::new(),
        ));
    }
    let metadata = Shared::new(metadata.clone()).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.register_primitives();
    checker.set_current_module_path("reader");
    checker.set_module_registry_direct(registry);
    let ast = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors: List<Text> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| format!("{e:?}").into())
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|e| format!("{e:?}").into()),
    );
    errors
}

fn replay(generic: bool, imported: &str, eager: bool) {
    let metadata = metadata(generic, imported);
    let args = if generic { "<Int>" } else { "" };
    {
        let good = errors(&metadata, &format!("fn read(x: core.consumer.Holder{args}) -> core.provider.Carrier{args} {{ x.value }}"), eager);
        assert!(
            good.is_empty(),
            "generic={generic} imported={imported} eager={eager}: {good:?}"
        );
        let bad = errors(
            &metadata,
            &format!(
                "fn wrong(x: core.consumer.Holder{args}) -> core.decoy.Carrier{args} {{ x.value }}"
            ),
            eager,
        );
        assert!(
            bad.iter()
                .any(|message| message.to_lowercase().contains("mismatch")),
            "wrong owner, generic={generic} imported={imported} eager={eager}: {bad:?}"
        );
    }
}

#[test]
fn implicit_record_lazy() {
    replay(false, "", false);
}

#[test]
fn implicit_record_eager() {
    replay(false, "", true);
}

#[test]
fn implicit_generic_lazy() {
    replay(true, "", false);
}

#[test]
fn implicit_generic_eager() {
    replay(true, "", true);
}

#[test]
fn glob_record_lazy() {
    replay(false, "mount core.prelude.*;", false);
}

#[test]
fn glob_record_eager() {
    replay(false, "mount core.prelude.*;", true);
}

#[test]
fn glob_generic_lazy() {
    replay(true, "mount core.prelude.*;", false);
}

#[test]
fn glob_generic_eager() {
    replay(true, "mount core.prelude.*;", true);
}

#[test]
fn named_record_lazy() {
    replay(false, "mount core.prelude.Carrier;", false);
}

#[test]
fn named_record_eager() {
    replay(false, "mount core.prelude.Carrier;", true);
}

#[test]
fn named_generic_lazy() {
    replay(true, "mount core.prelude.Carrier;", false);
}

#[test]
fn named_generic_eager() {
    replay(true, "mount core.prelude.Carrier;", true);
}
#[test]
fn direct_record_lazy() {
    replay(false, "mount core.provider.Carrier;", false);
}
#[test]
fn direct_record_eager() {
    replay(false, "mount core.provider.Carrier;", true);
}
#[test]
fn direct_generic_lazy() {
    replay(true, "mount core.provider.Carrier;", false);
}
#[test]
fn direct_generic_eager() {
    replay(true, "mount core.provider.Carrier;", true);
}

#[test]
fn named_type_wins_over_conflicting_glob_candidates() {
    replay(
        true,
        "mount core.provider.Carrier; mount core.decoy.*;",
        false,
    );
    replay(
        true,
        "mount core.provider.Carrier; mount core.decoy.*;",
        true,
    );
}

#[test]
fn a_missing_archived_binding_does_not_select_a_foreign_same_leaf_type() {
    let mut metadata = metadata(true, "");
    metadata
        .declaration_type_bindings
        .get_mut(&Text::from("core.consumer"))
        .expect("source declaration scope")
        .remove(&Text::from("Carrier"));
    for eager in [false, true] {
        let rejected = errors(
            &metadata,
            "fn read(x: core.consumer.Holder<Int>) -> core.provider.Carrier<Int> { x.value }",
            eager,
        );
        assert!(
            rejected
                .iter()
                .any(|message| message.contains("Carrier") && message.contains("TypeNotFound")),
            "eager={eager}: {rejected:?}"
        );
    }
}

fn assert_invalid_source(provider: &str, consumer: &str, extra: &[&str]) {
    let mut modules = vec![
        Parser::new("module core; public module provider; public module decoy; public module prelude { public mount core.provider.Carrier; }").parse_module().unwrap(),
        Parser::new(provider).parse_module().unwrap(),
        Parser::new("module core.decoy; public type Carrier is { public value: Int };").parse_module().unwrap(),
        Parser::new(consumer).parse_module().unwrap(),
    ];
    modules.extend(
        extra
            .iter()
            .map(|source| Parser::new(source).parse_module().unwrap()),
    );
    let owned: Vec<_> = modules.iter().map(|module| ("core", module)).collect();
    let error = VbcCodegen::with_config(CodegenConfig::new("core"))
        .collect_owned_declaration_interface(&owned)
        .err()
        .expect("invalid source binding must refuse before archiving");
    assert!(
        matches!(&error.kind, verum_vbc::codegen::CodegenErrorKind::InvalidSourceSignature(message)
        if message.contains("Carrier")),
        "{error:?}"
    );
}

#[test]
fn ambiguous_lexical_globs_cannot_publish_a_selected_owner() {
    assert_invalid_source(
        "module core.provider; public type Carrier is { public value: Int };",
        "module core.consumer; mount core.decoy.*; public type Holder is { public value: Carrier, public witness: Type };",
        &[],
    );
}

#[test]
fn public_reexport_cannot_expose_a_private_type() {
    assert_invalid_source(
        "module core.provider; type Carrier is { public value: Int };",
        "module core.consumer; public type Holder is { public value: Carrier, public witness: Type };",
        &[],
    );
}

#[test]
fn unresolved_named_type_cannot_fall_back_to_the_prelude() {
    assert_invalid_source(
        "module core.provider; public type Carrier is { public value: Int };",
        "module core.consumer; mount core.absent.Carrier; public type Holder is { public value: Carrier, public witness: Type };",
        &[],
    );
}

#[test]
fn local_type_keeps_precedence_over_the_implicit_prelude() {
    let metadata = metadata(true, "public type Carrier<T> is { public local: T };");
    for eager in [false, true] {
        let accepted = errors(
            &metadata,
            "fn local(x: core.consumer.Holder<Int>) -> core.consumer.Carrier<Int> { x.value }",
            eager,
        );
        assert!(accepted.is_empty(), "eager={eager}: {accepted:?}");
        let rejected = errors(
            &metadata,
            "fn foreign(x: core.consumer.Holder<Int>) -> core.provider.Carrier<Int> { x.value }",
            eager,
        );
        assert!(
            rejected
                .iter()
                .any(|message| message.to_lowercase().contains("mismatch")),
            "eager={eager}: {rejected:?}"
        );
    }
}

#[test]
fn glob_alias_retains_its_selected_declaration_and_generic_target() {
    let metadata = metadata_with_alias(true, "", true);
    assert_eq!(
        metadata
            .declaration_type_bindings
            .get(&Text::from("core.consumer"))
            .unwrap()
            .get(&Text::from("Carrier")),
        Some(&Some("core.provider.Carrier".into()))
    );
    for eager in [false, true] {
        let accepted = errors(
            &metadata,
            "fn alias(x: core.consumer.Holder<Int>) -> core.provider.Cell<(&Int, Int)> { x.value }",
            eager,
        );
        assert!(accepted.is_empty(), "eager={eager}: {accepted:?}");
        let rejected = errors(
            &metadata,
            "fn foreign(x: core.consumer.Holder<Int>) -> core.decoy.Carrier<Int> { x.value }",
            eager,
        );
        assert!(
            rejected
                .iter()
                .any(|message| message.to_lowercase().contains("mismatch")),
            "eager={eager}: {rejected:?}"
        );
    }
}

#[test]
fn unresolved_glob_reexport_cannot_select_another_globs_valid_type() {
    assert_invalid_source(
        "module core.provider; public type Carrier is { public value: Int };",
        "module core.consumer; mount core.broken.*; public type Holder is { public value: Carrier, public witness: Type };",
        &["module core.broken; public mount core.absent.Carrier;"],
    );
}

#[test]
fn unrelated_glob_without_the_type_does_not_block_a_valid_import() {
    let imports = "public module other { public type Unrelated is { value: Int }; } mount core.consumer.other.*;";
    replay(true, imports, false);
    replay(true, imports, true);
}
