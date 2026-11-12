//! Execute the unchanged OS error declaration and conversion bodies across modules.
//! This isolates their public contract; the full stdlib bake is a separate gate.
use std::sync::Arc;
use verum_ast::{ItemKind, decl::{ImplItemKind, ImplKind}, ty::TypeKind};
use verum_common::{List, Text};
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

fn source_function<'a>(source: &'a str, name: &str) -> &'a str {
    let ast = Parser::new(source).parse_module().expect("stdlib grammar");
    let function = ast.items.iter().find(|item| {
        matches!(&item.kind, ItemKind::Function(function) if function.name.name == name)
    }).expect("actual free function");
    declaration_text(source, function.span)
}

fn execute_contract(modules: &[(&str, Text)], consumer: &str) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir_all(core.join("sys")).unwrap();
    std::fs::write(core.join("mod.vr"), "module core; public module sys; public module consumer;").unwrap();
    let mut namespace = Text::from("module core.sys;");
    for (name, source) in modules {
        namespace.push_str(&format!(" public module {name};"));
        std::fs::write(core.join("sys").join(format!("{name}.vr")), format!("module core.sys.{name};\n{source}")).unwrap();
    }
    std::fs::write(core.join("sys/mod.vr"), namespace).unwrap();
    std::fs::write(core.join("consumer.vr"), consumer).unwrap();
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

#[test]
fn os_error_data_and_adapter_survive_cross_module_archive_execution() {
    let error = source_contract(include_str!("../../../core/sys/common.vr"), "OSError", &["new", "into_message"]);
    let engine = source_contract(include_str!("../../../core/sys/io_engine.vr"), "EngineIoError", &["from_errno", "from_os"]);
    execute_contract(&[("common", error), ("io_engine", format!("mount core.sys.common.OSError;\n{engine}").into())], r#"
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
    "#);
}

#[test]
fn platform_error_adapters_preserve_categories_and_failure_details() {
    let mut modules = List::new();
    modules.push(("io_engine", source_contract(
        include_str!("../../../core/sys/io_engine.vr"), "EngineIoError", &["from_errno"],
    )));
    for (module, source, owner) in [
        ("linux", include_str!("../../../core/sys/linux/io.vr"), "LinuxIoDriverError"),
        ("darwin", include_str!("../../../core/sys/darwin/io.vr"), "DarwinIoDriverError"),
        ("windows", include_str!("../../../core/sys/windows/io.vr"), "WindowsIoDriverError"),
    ] {
        modules.push((module, format!("mount core.sys.io_engine.EngineIoError;\n{}\n{}",
            source_contract(source, owner, &[]), source_function(source, "convert_error")).into()));
    }
    let mut consumer = Text::from(r#"module core.consumer;
        mount core.sys.io_engine.EngineIoError;
        mount core.sys.linux.{LinuxIoDriverError, convert_error as linux_error};
        mount core.sys.darwin.{DarwinIoDriverError, convert_error as darwin_error};
        mount core.sys.windows.{WindowsIoDriverError, convert_error as windows_error};
        public fn probe() -> Int {
    "#);
    let checks = [
        ("linux_error(LinuxIoDriverError.UringSetupFailed { code: 19 })", "Other(message)", "message == \"io_uring setup failed: errno 19\""),
        ("linux_error(LinuxIoDriverError.MmapFailed { code: 12 })", "Other(message)", "message == \"mmap failed: errno 12\""),
        ("linux_error(LinuxIoDriverError.RegisterFailed { errno: 13 })", "PermissionDenied", "true"),
        ("linux_error(LinuxIoDriverError.EpollFailed { code: 2 })", "NotFound { path }", "path == \"\""),
        ("linux_error(LinuxIoDriverError.QueueFull)", "SubmissionQueueFull", "true"),
        ("linux_error(LinuxIoDriverError.Unsupported)", "NotSupported", "true"),
        ("darwin_error(DarwinIoDriverError.Other { code: -73, message: \"native failure\" })", "Other(message)", "message == \"kqueue error -73: native failure\""),
        ("darwin_error(DarwinIoDriverError.NotFound { path: \"missing-file\" })", "NotFound { path }", "path == \"missing-file\""),
        ("darwin_error(DarwinIoDriverError.InvalidInput { reason: \"invalid-change\" })", "InvalidInput { reason }", "reason == \"invalid-change\""),
        ("darwin_error(DarwinIoDriverError.Closed)", "Closed", "true"),
        ("windows_error(WindowsIoDriverError.Other { code: 4294967295_u32, message: \"native failure\" })", "Other(message)", "message == \"IOCP error 4294967295: native failure\""),
        ("windows_error(WindowsIoDriverError.NotFound { path: \"missing-file\" })", "NotFound { path }", "path == \"missing-file\""),
        ("windows_error(WindowsIoDriverError.InvalidInput { reason: \"invalid-handle\" })", "InvalidInput { reason }", "reason == \"invalid-handle\""),
        ("windows_error(WindowsIoDriverError.Cancelled)", "Cancelled", "true"),
        ("windows_error(WindowsIoDriverError.EndOfFile)", "EndOfFile", "true"),
    ];
    for (index, (expression, pattern, condition)) in checks.iter().enumerate() {
        consumer.push_str(&format!("if !(match {expression} {{ EngineIoError.{pattern} => {condition}, _ => false }}) {{ return {}; }}\n", index + 1));
    }
    consumer.push_str("73 }");
    execute_contract(&modules, &consumer);
}
