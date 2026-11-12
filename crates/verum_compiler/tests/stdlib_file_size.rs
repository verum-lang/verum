//! Execute the shipped file-size dispatcher and Darwin wrapper against an OS fd.
//! The small archive retains the selected production declarations verbatim;
//! unrelated stdlib modules are validated by the complete bake separately.
#![cfg(target_os = "macos")]

use std::{fs, os::fd::AsRawFd, path::Path, sync::Arc};
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::Parser;
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

fn selected(path: &str, names: &[&str]) -> Text {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = fs::read_to_string(root.join(path)).unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut remaining: List<&str> = names.iter().copied().collect();
    let mut output = Text::new();
    for item in &ast.items {
        let (name, visibility) = match &item.kind {
            ItemKind::Function(decl) => (decl.name.name.as_str(), &decl.visibility),
            ItemKind::Type(decl) => (decl.name.name.as_str(), &decl.visibility),
            _ => continue,
        };
        if let Some(index) = remaining.iter().position(|candidate| *candidate == name) {
            remaining.remove(index);
            output.push_str(&declaration_text(&source, item.span, visibility));
            output.push_str("\n");
        }
    }
    assert!(
        remaining.is_empty(),
        "missing production declarations: {remaining:?}"
    );
    output
}

fn declaration_text(
    source: &str,
    span: verum_ast::Span,
    visibility: &verum_ast::Visibility,
) -> Text {
    let text = &source[span.start as usize..span.end as usize];
    // Declaration spans start at the keyword, after visibility modifiers.
    if *visibility == verum_ast::Visibility::Public && !text.trim_start().starts_with("public ") {
        format!("public {text}").into()
    } else {
        text.into()
    }
}

#[test]
fn production_file_size_reads_and_rejects_real_descriptors() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    let mut common = selected("core/sys/common.vr", &["OSError", "FileDesc", "file_size"]);
    // Retain the actual constructor body without importing unrelated methods.
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/sys/common.vr"))
            .unwrap();
    let ast = Parser::new(&source).parse_module().unwrap();
    let constructor = ast
        .items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Impl(decl) => Some(decl),
            _ => None,
        })
        .flat_map(|decl| decl.items.iter())
        .find_map(|item| match &item.kind {
            verum_ast::decl::ImplItemKind::Function(decl)
                if decl.name.name == "new"
                    && source[decl.span.start as usize..decl.span.end as usize]
                        .contains("-> OSError") =>
            {
                Some(decl)
            }
            _ => None,
        })
        .expect("OSError.new production constructor");
    common.push_str("implement OSError {\n");
    common.push_str(&declaration_text(
        &source,
        constructor.span,
        &constructor.visibility,
    ));
    common.push_str("\n}\n");

    let mut darwin = selected(
        "core/sys/darwin/libsystem.vr",
        &[
            "DarwinStat",
            "DarwinTimespec",
            "safe_stat_zeroed",
            "safe_fstat",
            "errno",
            "errno_to_message",
        ],
    );
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/sys/darwin/libsystem.vr"),
    )
    .unwrap();
    darwin.push_str("@ffi(\"libSystem.B.dylib\") extern {\n");
    for name in ["fstat", "__error"] {
        let start = source
            .find(&format!("fn {name}("))
            .expect("production OS binding");
        let end = start
            + source[start..]
                .find(';')
                .expect("foreign declaration terminator")
            + 1;
        darwin.push_str(&source[start..end]);
        darwin.push_str("\n");
    }
    darwin.push_str("}\n");
    let errno = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/sys/darwin/errno.vr"),
    )
    .unwrap();
    for (path, source) in [
        ("mod.vr", Text::from("module core; public module base; public module sys;")),
        ("base/mod.vr", Text::from("module core.base; public module result;")),
        ("base/result.vr", format!("module core.base.result; {}", selected("core/base/result.vr", &["Result"])).into()),
        ("sys/mod.vr", Text::from("module core.sys; public module common; public module darwin;")),
        ("sys/common.vr", format!("module core.sys.common; mount core.base.result.Result; {common}").into()),
        ("sys/darwin/mod.vr", Text::from("module core.sys.darwin; public module libsystem; public module errno;")),
        ("sys/darwin/libsystem.vr", format!("module core.sys.darwin.libsystem; mount core.base.result.Result; mount core.sys.common.{{OSError, FileDesc}}; {darwin}").into()),
        ("sys/darwin/errno.vr", errno.into()),
    ] {
        let path = core.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source.as_bytes()).unwrap();
    }
    let archive = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&archive))
            .compile_core()
            .unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&archive).unwrap();
    let file = tempfile::tempfile().unwrap();
    for length in [0, 73, 8193] {
        file.set_len(length).unwrap();
        let source = format!("module app; mount core.sys.common.{{file_size, FileDesc}}; mount core.base.result.Result;
            fn main() -> Int {{ match file_size(FileDesc({})) {{ Result.Ok(size) => size, Result.Err(_) => -1 }} }}", file.as_raw_fd());
        assert_eq!(execute(&archive, &source), length as i64);
    }
    let source = "module app; mount core.sys.common.{file_size, FileDesc}; mount core.base.result.Result;
        fn main() -> Int { match file_size(FileDesc(-1)) { Result.Err(_) => 1, Result.Ok(_) => 0 } }";
    assert_eq!(execute(&archive, source), 1);
}

fn execute(archive: &verum_vbc::archive::VbcArchive, source: &str) -> i64 {
    let app = Parser::new(source).parse_module().unwrap();
    let mut codegen =
        verum_vbc::codegen::VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("app"));
    verum_compiler::archive_ctx_loader::ArchiveCtxCache::new(archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], verum_vbc::codegen::ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let main = module.find_function_by_name("app.main").unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(main)
        .unwrap()
        .try_as_i64()
        .unwrap()
}
