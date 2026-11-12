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
