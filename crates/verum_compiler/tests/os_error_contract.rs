//! Execute the unchanged OS error declaration and conversion bodies across modules.
//! This isolates their public contract; the full stdlib bake is a separate gate.
use std::sync::Arc;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_common::{List, Map, Set, Text};
use verum_compiler::{
    CompilationPipeline, CompilerOptions, CoreConfig, Session, archive_ctx_loader::ArchiveCtxCache,
};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
use verum_vbc::{
    deserialize::deserialize_module, interpreter::Interpreter, serialize::serialize_module,
};

fn declaration_text(source: &str, span: verum_ast::Span) -> &str {
    // Parser item spans begin after visibility/modifier tokens. Keep the
    // original declaration line so this fixture cannot change its API policy.
    let start = source[..span.start as usize]
        .rfind('\n')
        .map_or(0, |line| line + 1);
    &source[start..span.end as usize]
}

fn source_contract(source: &str, owner: &str, methods: &[&str]) -> Text {
    let ast = Parser::new(source).parse_module().expect("stdlib grammar");
    let declaration = ast
        .items
        .iter()
        .find(|item| matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == owner))
        .expect("actual type declaration");
    let mut span = declaration.span;
    for attribute in &declaration.attributes {
        span.start = span.start.min(attribute.span.start);
    }
    let mut selected: Text = declaration_text(source, span).into();
    if methods.is_empty() {
        return selected;
    }
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
    let function = ast
        .items
        .iter()
        .find(
            |item| matches!(&item.kind, ItemKind::Function(function) if function.name.name == name),
        )
        .expect("actual free function");
    declaration_text(source, function.span)
}

fn execute_contract(modules: &[(&str, Text)], consumer: &str, target: &str) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    std::fs::create_dir_all(core.join("sys")).unwrap();
    std::fs::write(
        core.join("mod.vr"),
        "module core; public module base; public module sys; public module consumer;",
    )
    .unwrap();
    std::fs::create_dir_all(core.join("base")).unwrap();
    std::fs::write(
        core.join("base/mod.vr"),
        "module core.base; public module result;",
    )
    .unwrap();
    std::fs::write(
        core.join("base/result.vr"),
        format!(
            "module core.base.result;\n{}",
            source_contract(include_str!("../../../core/base/result.vr"), "Result", &[])
        ),
    )
    .unwrap();
    let mut bodies: Map<Text, Text> = Map::new();
    let mut children: Map<Text, Set<Text>> = Map::new();
    for (name, source) in modules {
        assert!(
            bodies.insert((*name).into(), source.clone()).is_none(),
            "duplicate fixture owner {name}"
        );
        let mut owner = *name;
        loop {
            let (parent, child) = owner.rsplit_once('.').unwrap_or(("", owner));
            children
                .entry(parent.into())
                .or_default()
                .insert(child.into());
            if parent.is_empty() {
                break;
            }
            owner = parent;
        }
    }
    for (name, entries) in children {
        let body = bodies.entry(name).or_default();
        let mut entries: List<_> = entries.into_iter().collect();
        entries.sort();
        for entry in entries {
            body.push_str(&format!("\npublic module {entry};"));
        }
    }
    for (name, source) in bodies {
        let (path, owner) = if name.is_empty() {
            (core.join("sys/mod.vr"), Text::from("core.sys"))
        } else {
            (
                core.join("sys")
                    .join(format!("{}.vr", name.replace(".", "/"))),
                format!("core.sys.{name}").into(),
            )
        };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("module {owner};\n{source}")).unwrap();
    }
    std::fs::write(
        core.join("consumer.vr"),
        consumer.replace(
            "module core.consumer;",
            "module core.consumer; mount core.base.result.Result;",
        ),
    )
    .unwrap();
    let output = root.path().join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        target_triple: Some(target.into()),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap_or_else(|error| panic!("{error}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = verum_vbc::archive::read_archive_from_file(&output).unwrap();
    let app = Parser::new("module app; mount core.consumer.probe; fn main() -> Int { probe() }")
        .parse_module()
        .unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut codegen, &app, &[])
        .unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&[&app], ItemFailurePolicy::Strict)
        .unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let restored = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [module, restored] {
        let entry = module.find_function_by_name("app.main").unwrap();
        let module = Arc::new(module);
        let result = Interpreter::new(module.clone())
            .execute_function(entry)
            .unwrap_or_else(|error| {
                panic!(
                    "{error:?}\n{}",
                    verum_vbc::disassemble::disassemble_module(&module)
                )
            });
        assert_eq!(result.try_as_i64(), Some(73));
    }
}

fn source_constants(source: &str, prefix: &str) -> Text {
    let ast = Parser::new(source).parse_module().unwrap();
    ast.items
        .iter()
        .filter(|item| {
            matches!(&item.kind,
        ItemKind::Const(declaration) if declaration.name.name.starts_with(prefix))
        })
        .map(|item| declaration_text(source, item.span))
        .collect::<List<_>>()
        .join("\n")
        .into()
}

fn winerror_exports() -> Text {
    let source = include_str!("../../../core/sys/windows/kernel32.vr");
    let ast = Parser::new(source).parse_module().unwrap();
    ast.items
        .iter()
        .find_map(|item| {
            let text = declaration_text(source, item.span);
            text.starts_with("public mount super.winerror.")
                .then(|| text.into())
        })
        .expect("kernel32 exports the single WinError declaration owner")
}

fn native_error_modules() -> List<(&'static str, Text)> {
    [
        (
            "error",
            include_str!("../../../core/sys/error.vr")
                .split_once('\n')
                .unwrap()
                .1
                .into(),
        ),
        (
            "linux.errno",
            source_constants(include_str!("../../../core/sys/linux/errno.vr"), "E"),
        ),
        (
            "darwin.errno",
            source_constants(include_str!("../../../core/sys/darwin/errno.vr"), "E"),
        ),
        ("windows.kernel32", winerror_exports()),
        (
            "windows.winerror",
            include_str!("../../../core/sys/windows/winerror.vr")
                .split_once('\n')
                .unwrap()
                .1
                .into(),
        ),
    ]
    .into_iter()
    .collect()
}

fn os_error_source() -> Text {
    format!(
        "mount core.sys.error.{{OSErrorKind, native_error_kind, native_error_message}};\n{}",
        source_contract(
            include_str!("../../../core/sys/common.vr"),
            "OSError",
            &[
                "new",
                "from_code",
                "into_message",
                "kind",
                "is_success",
                "would_block",
                "interrupted"
            ]
        )
    )
    .into()
}

fn engine_error_source() -> Text {
    format!("mount core.base.result.Result;\nmount core.sys.common.OSError;\nmount core.sys.error.{{OSErrorKind, native_error_kind}};\n{}",
        source_contract(include_str!("../../../core/sys/io_engine.vr"), "EngineIoError",
            &["from_os_code", "from_os"])).into()
}

fn completion_source() -> Text {
    let source = include_str!("../../../core/sys/io_engine.vr");
    let ast = Parser::new(source).parse_module().unwrap();
    let mut declarations = Text::new();
    for name in ["BytesTransferred", "ErrnoCode"] {
        let item = ast
            .items
            .iter()
            .find(|item| {
                matches!(&item.kind,
            ItemKind::Type(declaration) if declaration.name.name == name)
            })
            .unwrap();
        declarations.push_str(declaration_text(source, item.span));
        declarations.push('\n');
    }
    declarations.push_str(&source_contract(
        source,
        "CompletionResult",
        &["error", "to_result", "was_cancelled", "was_timeout"],
    ));
    declarations
}

#[test]
fn native_error_domains_preserve_classification_and_unknown_data() {
    for (target, cases) in [
        (
            "aarch64-unknown-linux-gnu",
            &[
                (11, "WouldBlock"),
                (35, "Unknown"),
                (4, "Interrupted"),
                (54, "Unknown"),
                (60, "Unknown"),
                (61, "Unknown"),
                (62, "TimedOut"),
                (78, "Unknown"),
                (89, "Unknown"),
                (95, "NotSupported"),
                (104, "ConnectionReset"),
                (107, "NotConnected"),
                (110, "TimedOut"),
                (111, "ConnectionRefused"),
                (125, "Cancelled"),
                (10035, "Unknown"),
            ][..],
        ),
        (
            "aarch64-apple-darwin",
            &[
                (11, "Unknown"),
                (35, "WouldBlock"),
                (4, "Interrupted"),
                (38, "Unknown"),
                (45, "NotSupported"),
                (54, "ConnectionReset"),
                (57, "NotConnected"),
                (60, "TimedOut"),
                (61, "ConnectionRefused"),
                (62, "Unknown"),
                (78, "NotSupported"),
                (89, "Cancelled"),
                (101, "TimedOut"),
                (102, "NotSupported"),
                (110, "Unknown"),
                (111, "Unknown"),
                (125, "Unknown"),
            ][..],
        ),
        (
            "x86_64-pc-windows-msvc",
            &[
                (4, "Unknown"),
                (5, "PermissionDenied"),
                (11, "Unknown"),
                (13, "Unknown"),
                (35, "Unknown"),
                (38, "EndOfFile"),
                (87, "InvalidInput"),
                (109, "BrokenPipe"),
                (121, "TimedOut"),
                (258, "TimedOut"),
                (995, "Cancelled"),
                (997, "WouldBlock"),
                (1225, "ConnectionRefused"),
                (1236, "Unknown"),
                (1460, "TimedOut"),
                (10004, "Interrupted"),
                (10035, "WouldBlock"),
                (10053, "Unknown"),
                (10054, "ConnectionReset"),
                (10057, "NotConnected"),
                (10060, "TimedOut"),
                (10061, "ConnectionRefused"),
            ][..],
        ),
        (
            "x86_64-unknown-freebsd",
            &[
                (4, "Unknown"),
                (11, "Unknown"),
                (35, "Unknown"),
                (995, "Unknown"),
            ][..],
        ),
    ] {
        let mut modules = native_error_modules();
        modules.push(("common", os_error_source()));
        modules.push((
            "io_engine",
            format!("{}\n{}", engine_error_source(), completion_source()).into(),
        ));
        let mut consumer = Text::from(
            "module core.consumer; mount core.sys.common.OSError; mount core.sys.error.OSErrorKind; mount core.sys.io_engine.{EngineIoError, CompletionResult}; public fn probe() -> Int {\n",
        );
        for (index, (code, kind)) in cases.iter().enumerate() {
            let failure = index + 1;
            consumer.push_str(&format!(
                r#"
                let error = OSError.new({code}, "native detail");
                if !(error.kind() is OSErrorKind.{kind}) {{ return {failure}; }}
                if error.would_block() != {would_block} {{ return {failure}; }}
                if error.interrupted() != {interrupted} {{ return {failure}; }}
                let completion = CompletionResult.error(7, {code});
                if completion.was_timeout() != {timed_out} {{ return {failure}; }}
                if completion.was_cancelled() != {cancelled} {{ return {failure}; }}
            "#,
                would_block = *kind == "WouldBlock",
                interrupted = *kind == "Interrupted",
                timed_out = *kind == "TimedOut",
                cancelled = *kind == "Cancelled"
            ));
            let pattern = match *kind {
                "Unknown" => "Other(message)",
                "InvalidInput" => "InvalidInput { reason: message }",
                _ => kind,
            };
            let condition = match *kind {
                "Unknown" => format!("message == \"OS error {code}: native detail\""),
                "InvalidInput" => "message == \"native detail\"".into(),
                _ => "true".into(),
            };
            consumer.push_str(&format!("if !(match EngineIoError.from_os(error) {{ EngineIoError.{pattern} => {condition}, _ => false }}) {{ return {failure}; }}\n"));
            consumer.push_str(&format!("if !(match completion.to_result() {{ Result.Err(EngineIoError.{pattern}) => true, _ => false }}) {{ return {failure}; }}\n"));
        }
        // A narrowing cast must not turn these unclassified values into errno.
        for code in [-1_i64, 4_294_967_295] {
            consumer.push_str(&format!("if !(match EngineIoError.from_os(OSError.new({code}, \"wide native detail\")) {{ EngineIoError.Other(message) => message == \"OS error {code}: wide native detail\", _ => false }}) {{ return 72; }}\n"));
        }
        if target == "x86_64-pc-windows-msvc" {
            consumer = consumer.replace(
                "public fn probe()",
                "mount core.sys.windows.kernel32 as win32; public fn probe()",
            );
            consumer.push_str("if win32.ERROR_TIMEOUT != 1460 { return 71; }\n");
        }
        consumer.push_str("73 }");

        execute_contract(&modules, &consumer, target);
    }
}

#[test]
fn os_error_data_and_adapter_survive_cross_module_archive_execution() {
    let mut modules = native_error_modules();
    modules.push(("common", os_error_source()));
    modules.push(("io_engine", engine_error_source()));
    execute_contract(
        &modules,
        r#"
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
    "#,
        "aarch64-apple-darwin",
    );
}

#[test]
fn platform_error_adapters_preserve_categories_and_failure_details() {
    let mut modules = native_error_modules();
    modules.push(("common", os_error_source()));
    modules.push(("io_engine", engine_error_source()));
    for (module, source, owner) in [
        (
            "linux",
            include_str!("../../../core/sys/linux/io.vr"),
            "LinuxIoDriverError",
        ),
        (
            "darwin",
            include_str!("../../../core/sys/darwin/io.vr"),
            "DarwinIoDriverError",
        ),
        (
            "windows",
            include_str!("../../../core/sys/windows/io.vr"),
            "WindowsIoDriverError",
        ),
    ] {
        modules.push((
            module,
            format!(
                "mount core.sys.io_engine.EngineIoError;\n{}\n{}",
                source_contract(source, owner, &[]),
                source_function(source, "convert_error")
            )
            .into(),
        ));
    }
    let mut consumer = Text::from(
        r#"module core.consumer;
        mount core.sys.io_engine.EngineIoError;
        mount core.sys.linux.{LinuxIoDriverError, convert_error as linux_error};
        mount core.sys.darwin.{DarwinIoDriverError, convert_error as darwin_error};
        mount core.sys.windows.{WindowsIoDriverError, convert_error as windows_error};
        public fn probe() -> Int {
    "#,
    );
    let checks = [
        (
            "linux_error(LinuxIoDriverError.UringSetupFailed { code: 19 })",
            "Other(message)",
            "message == \"io_uring setup failed: errno 19\"",
        ),
        (
            "linux_error(LinuxIoDriverError.MmapFailed { code: 12 })",
            "Other(message)",
            "message == \"mmap failed: errno 12\"",
        ),
        (
            "linux_error(LinuxIoDriverError.RegisterFailed { errno: 13 })",
            "PermissionDenied",
            "true",
        ),
        (
            "linux_error(LinuxIoDriverError.EpollFailed { code: 2 })",
            "NotFound { path }",
            "path == \"\"",
        ),
        (
            "linux_error(LinuxIoDriverError.QueueFull)",
            "SubmissionQueueFull",
            "true",
        ),
        (
            "linux_error(LinuxIoDriverError.Unsupported)",
            "NotSupported",
            "true",
        ),
        (
            "darwin_error(DarwinIoDriverError.Other { code: -73, message: \"native failure\" })",
            "Other(message)",
            "message == \"kqueue error -73: native failure\"",
        ),
        (
            "darwin_error(DarwinIoDriverError.NotFound { path: \"missing-file\" })",
            "NotFound { path }",
            "path == \"missing-file\"",
        ),
        (
            "darwin_error(DarwinIoDriverError.InvalidInput { reason: \"invalid-change\" })",
            "InvalidInput { reason }",
            "reason == \"invalid-change\"",
        ),
        ("darwin_error(DarwinIoDriverError.Closed)", "Closed", "true"),
        (
            "windows_error(WindowsIoDriverError.Other { code: 4294967295_u32, message: \"native failure\" })",
            "Other(message)",
            "message == \"IOCP error 4294967295: native failure\"",
        ),
        (
            "windows_error(WindowsIoDriverError.NotFound { path: \"missing-file\" })",
            "NotFound { path }",
            "path == \"missing-file\"",
        ),
        (
            "windows_error(WindowsIoDriverError.InvalidInput { reason: \"invalid-handle\" })",
            "InvalidInput { reason }",
            "reason == \"invalid-handle\"",
        ),
        (
            "windows_error(WindowsIoDriverError.Cancelled)",
            "Cancelled",
            "true",
        ),
        (
            "windows_error(WindowsIoDriverError.EndOfFile)",
            "EndOfFile",
            "true",
        ),
    ];
    for (index, (expression, pattern, condition)) in checks.iter().enumerate() {
        consumer.push_str(&format!("if !(match {expression} {{ EngineIoError.{pattern} => {condition}, _ => false }}) {{ return {}; }}\n", index + 1));
    }
    consumer.push_str("73 }");
    execute_contract(&modules, &consumer, "aarch64-unknown-linux-gnu");
}
