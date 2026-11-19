#![cfg(feature = "codegen")]
//! Nominal parameter diagnostics retain the selected header before layout exists.
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, CodegenErrorKind, VbcCodegen};

#[test]
fn forward_arity_refusal_names_the_declaring_owner() {
    for first in [false, true] {
        let declaration = "type Closed is ();";
        let field = "type Container is { value: Closed<Int> };";
        let source = if first {
            format!("module headers; {declaration} {field}")
        } else {
            format!("module headers; {field} {declaration}")
        };
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::with_config(CodegenConfig::new("headers"))
            .compile_module(&ast)
            .unwrap_err();
        assert!(
            matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref message)
            if message.contains("1 arguments for 0 declared parameters")),
            "{error:?}"
        );
        assert!(
            error
                .context
                .as_ref()
                .is_some_and(|context| context.contains("nominal application of 'headers.Closed'")),
            "first={first}: {error:?}"
        );
    }
}

#[test]
fn an_imported_header_does_not_report_a_same_leaf_local_owner() {
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    std::fs::write(core.join("mod.vr"), "module core;").unwrap();
    std::fs::write(
        core.join("provider.vr"),
        "module core.provider; public type Cell is ();",
    )
    .unwrap();
    let source = r#"
        module core.consumer;
        mount core.provider.Cell as ImportedCell;
        type Cell<T> is { value: T };
        type Container is { value: ImportedCell<Int> };
    "#;
    let path = core.join("consumer.vr");
    std::fs::write(&path, source).unwrap();
    let ast = Parser::new(source).parse_module().unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("core.consumer"))
        .compile_module_with_mounts(&ast, path.to_str().unwrap(), core.to_str().unwrap())
        .unwrap_err();
    assert!(
        matches!(error.kind, CodegenErrorKind::InvalidSourceSignature(ref message)
        if message.contains("1 arguments for 0 declared parameters")),
        "{error:?}"
    );
    assert!(
        error
            .context
            .as_ref()
            .is_some_and(|context| context.contains("nominal application of 'core.provider.Cell'")),
        "{error:?}"
    );
}

#[test]
fn source_unit_mounts_share_the_selected_file_across_consumers() {
    use verum_ast::{FileId, SourceFile};
    use verum_common::Shared;
    use verum_vbc::codegen::{CodegenError, ItemFailurePolicy, SourceMountSession};
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    let root = core.join("mod.vr");
    let provider = core.join("provider.vr");
    std::fs::write(&root, "module changed;").unwrap();
    std::fs::write(&provider, "module core.provider; public type DiskDecoy is ();").unwrap();
    let root_source = "module core; public module provider;";
    let provider_source = "module core.provider; public type Carried is { public value: Int };";
    let open = |path: &std::path::Path| {
        let (id, source) = if path.ends_with("core/mod.vr") { (31, root_source) }
            else if path.ends_with("core/provider.vr") { (41, provider_source) }
            else { return Err(CodegenError::internal(format!("unexpected source {path:?}"))); };
        Ok(Shared::new(SourceFile::new(FileId::new(id), path.display().to_string(), source.to_owned())))
    };
    let entry = verum_fast_parser::FastParser::new().parse_module_str(
        "module app.entry; mount core.provider.Carried; mount app.reader.read; fn main() -> Int { read(Carried { value: 73 }) }",
        FileId::new(3),
    ).unwrap();
    let reader = verum_fast_parser::FastParser::new().parse_module_str(
        "module app.reader; mount core.provider.Carried; public fn read(value: Carried) -> Int { value.value }",
        FileId::new(4),
    ).unwrap();
    let fresh = std::cell::Cell::new(80);
    let next = || { let id = fresh.get(); fresh.set(id + 1); FileId::new(id) };
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("app"));
    cg.initialize();
    cg.resolve_unit_mounts(&[(&entry, "entry.vr"), (&reader, "reader.vr")],
        core.to_str().unwrap(), Some(SourceMountSession { open: &open, allocate_id: &next })).unwrap();
    cg.collect_unit_declarations(&[&entry, &reader]).unwrap();
    cg.compile_unit_items(&[&entry, &reader], ItemFailurePolicy::Strict).unwrap();
    let module = cg.finalize_module_from_state().unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    for value in [module, wire] {
        let declarations: Vec<_> = value.types.iter().filter_map(|ty| ty.source_declaration.as_ref())
            .filter(|declaration| declaration.name.name == "Carried").collect();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].span.file_id, FileId::new(41));
        let function = value.find_function_by_name("app.entry.main").unwrap();
        let result = verum_vbc::interpreter::Interpreter::new(value.into()).execute_function(function).unwrap();
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
