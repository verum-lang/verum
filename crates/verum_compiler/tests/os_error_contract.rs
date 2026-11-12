//! Execute the unchanged OS error declaration and conversion bodies across modules.
//! This isolates their public contract; the full stdlib bake is a separate gate.
use std::sync::Arc;
use verum_ast::{ItemKind, decl::{ImplItemKind, ImplKind}, ty::TypeKind};
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache};
use verum_fast_parser::Parser;
use verum_vbc::{deserialize::deserialize_module, interpreter::Interpreter, serialize::serialize_module};
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};

fn declaration_text(source: &str, span: verum_ast::Span) -> &str {
    // Parser item spans begin after visibility/modifier tokens. Keep the
    // original declaration line so this fixture cannot change its API policy.
    let start = source[..span.start as usize].rfind('\n').map_or(0, |line| line + 1);
    &source[start..span.end as usize]
}

fn source_contract(source: &str, owner: &str, methods: &[&str]) -> Text {
    let ast = Parser::new(source).parse_module().expect("stdlib grammar");
    let declaration = ast.items.iter().find(|item| {
        matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == owner)
    }).expect("actual type declaration");
    let mut selected: Text = declaration_text(source, declaration.span).into();
    selected.push_str(&format!("\nimplement {owner} {{\n"));
    for name in methods {
        let method = ast.items.iter().filter_map(|item| match &item.kind {
            ItemKind::Impl(decl) if matches!(&decl.kind,
                ImplKind::Inherent(ty) if matches!(&ty.kind,
                    TypeKind::Path(path) if path.as_ident().is_some_and(|ident| ident.name == owner))) => Some(decl),
            _ => None,
        }).flat_map(|decl| &decl.items).find(|item| {
            matches!(&item.kind, ImplItemKind::Function(function) if function.name.name == *name)
        }).expect("actual inherent method");
        selected.push_str(declaration_text(source, method.span));
        selected.push('\n');
    }
    selected.push_str("}\n");
    selected
}

#[test]
fn os_error_data_and_adapter_survive_cross_module_archive_execution() {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir_all(core.join("sys")).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module sys; public module consumer;").unwrap();
    std::fs::write(core.join("sys/mod.vr"), "module core.sys; public module common; public module io_engine;").unwrap();
    let error = source_contract(include_str!("../../../core/sys/common.vr"), "OSError", &["new", "into_message"]);
    let engine = source_contract(include_str!("../../../core/sys/io_engine.vr"), "EngineIoError", &["from_errno", "from_os"]);
    std::fs::write(core.join("sys/common.vr"), format!("module core.sys.common;\n{error}")).unwrap();
    std::fs::write(core.join("sys/io_engine.vr"), format!("module core.sys.io_engine; mount core.sys.common.OSError;\n{engine}")).unwrap();
    std::fs::write(core.join("consumer.vr"), r#"
        module core.consumer;
        mount core.sys.common.OSError;
        mount core.sys.io_engine.EngineIoError;
        public fn probe() -> Int {
            let error = OSError.new(2, "missing item");
            if error.code != 2 { return 1; }
            if error.message != "missing item" { return 2; }
            let converted = EngineIoError.from_os(error);
            match converted {
                EngineIoError.NotFound { path } => if path != "" { return 3; },
                _ => return 4,
            }
            let data = OSError { code: 13, message: "access refused" };
            if data.into_message() != "access refused" { return 5; }
            match EngineIoError.from_os(OSError.new(13, "denied")) {
                EngineIoError.PermissionDenied => 73,
                _ => 6,
            }
        }
    "#).unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions { input: core.join("mod.vr"), ..Default::default() });
    let result = CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
        .compile_core().unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let app = Parser::new("module app; mount core.consumer.probe; fn main() -> Int { probe() }")
        .parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive).apply_lazy_with_types(&mut codegen, &app, &[]).unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&app], ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("app.main").unwrap();
        let module = Arc::new(module);
        let result = Interpreter::new(module.clone()).execute_function(entry).unwrap_or_else(|error| {
            panic!("{error:?}\n{}", verum_vbc::disassemble::disassemble_module(&module))
        });
        assert_eq!(result.try_as_i64(), Some(73));
    }
}
