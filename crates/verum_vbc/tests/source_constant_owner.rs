//! T1761: constants belong to their declaring source module, including methods.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn check(reverse: bool, entry: &str, expected: i64) {
    check_registered(reverse, entry, expected, false);
}
fn check_registered(reverse: bool, entry: &str, expected: i64, repeated: bool) {
    let first = Parser::new(
        r#"
module first;
const TABLE: [Int; 2] = [11, 13];
fn read() -> Int { TABLE[0] }
"#,
    )
    .parse_module()
    .expect("first grammar");
    let second = Parser::new(
        r#"
module second;
const TABLE: [Int; 2] = [17, 19];
type Reader is {};
implement Reader { fn read(&self) -> Int { TABLE[1] } }
fn read() -> Int { TABLE[0] }
fn method() -> Int { let reader = Reader {}; reader.read() }
"#,
    )
    .parse_module()
    .expect("second grammar");
    let modules = if reverse {
        [&second, &first]
    } else {
        [&first, &second]
    };
    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    if repeated {
        codegen
            .collect_unit_declarations(&[&second])
            .expect("dependency declarations");
    }
    codegen
        .collect_unit_declarations(&modules)
        .expect("declarations");
    codegen
        .compile_unit_items(&modules, ItemFailurePolicy::Strict)
        .expect("source bodies");
    let module = codegen.finalize_module().expect("source VBC");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("decode");
    for (route, module) in [("source", module), ("wire", wire)] {
        let id = module.find_function_by_name(entry).expect("source entry");
        let mut interpreter = Interpreter::new(Arc::new(module));
        let actual = interpreter.execute_function(id).expect("execute").as_i64();
        assert_eq!(actual, expected, "{route} {entry}, reversed={reverse}");
    }
}
#[test]
fn first_constant_is_not_shadowed_by_later_module() {
    check(false, "first.read", 11);
}
#[test]
fn later_constant_keeps_its_declaring_module() {
    check(false, "second.read", 17);
}
#[test]
fn method_reads_its_declaring_module_constant() {
    check(false, "second.method", 19);
}
#[test]
fn first_constant_survives_reversed_declaration_order() {
    check(true, "first.read", 11);
}
#[test]
fn later_constant_survives_reversed_declaration_order() {
    check(true, "second.read", 17);
}
#[test]
fn method_constant_survives_reversed_declaration_order() {
    check(true, "second.method", 19);
}

#[test]
fn repeated_source_declaration_keeps_constant_body_identity() {
    check_registered(false, "second.read", 17, true);
}
#[test]
fn repeated_method_declaration_keeps_constant_body_identity() {
    check_registered(false, "second.method", 19, true);
}

#[test]
fn archived_socket_classifier_uses_its_target_errno_table() {
    use verum_ast::{ItemKind, cfg::TargetConfig};
    let source = include_str!("../../../core/io/protocols.vr");
    let mut ast = Parser::new(source).parse_module().unwrap();
    ast.items.retain(|item| match &item.kind {
        ItemKind::Type(decl) => decl.name.name == "IoErrorKind",
        ItemKind::Function(decl) => decl.name.name == "io_error_kind_from_os_code",
        _ => false,
    });
    for (target, expected) in [
        (TargetConfig::linux_x86_64(), 1), (TargetConfig::macos_aarch64(), 10),
        (TargetConfig::windows_x86_64(), 100),
    ] {
        let mut unit = ast.clone();
        let probe = Parser::new("fn classify(n:Int)->Int { match io_error_kind_from_os_code(n) { IoErrorKind.WouldBlock => 1, _ => 0 } }
            fn probe()->Int { classify(11) + 10*classify(35) + 100*classify(10035) }").parse_module().unwrap();
        unit.items.extend(probe.items);
        let mut codegen = VbcCodegen::with_config(verum_vbc::codegen::CodegenConfig::new("core.io.protocols").with_target(target));
        let module = codegen.compile_module(&unit).unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let entry = module.functions.iter().find(|f| module.get_string(f.name).is_some_and(|n| n == "probe" || n.ends_with(".probe"))).unwrap().id;
        assert_eq!(Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64(), expected);
    }
}

const SYSCALL_CONSTANTS: &[(&str, i64)] = &[
    ("TCGETS", 0x5401), ("TCSETS", 0x5402), ("TCSETSW", 0x5403), ("TCSETSF", 0x5404),
    ("TIOCGWINSZ", 0x5413), ("TIOCSWINSZ", 0x5414),
    ("F_GETFL", 3), ("F_SETFL", 4), ("O_NONBLOCK", 0o4000),
];

#[test]
fn syscall_constants_remain_distinct_when_compiled_from_their_declarations() {
    let mut ast = Parser::new(include_str!("../../../core/sys/linux/syscall.vr")).parse_module().unwrap();
    ast.items.retain(|item| match &item.kind {
        verum_ast::ItemKind::Module(_) => true,
        verum_ast::ItemKind::Const(decl) => SYSCALL_CONSTANTS.iter().any(|&(name, _)| name == decl.name.name.as_str()),
        _ => false,
    });
    let module = VbcCodegen::new().compile_module(&ast).unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    check_syscall_constants(verum_vbc::deserialize::deserialize_module(&bytes).unwrap());
}

#[test]
#[ignore = "requires the freshly built VERUM_TEST_FROZEN_STDLIB archive"]
fn published_syscall_constants_execute_from_distinct_declared_accessors() {
    let path = std::env::var_os("VERUM_TEST_FROZEN_STDLIB").expect("fresh stdlib archive path");
    let archive = verum_vbc::archive::read_archive_from_file(path).unwrap();
    check_syscall_constants(archive.load_module("core.sys.linux").unwrap());
}

fn check_syscall_constants(module: verum_vbc::module::VbcModule) {
    let module = Arc::new(module);
    let mut interpreter = Interpreter::new(module.clone());
    let mut ids = std::collections::HashSet::new();
    for &(name, expected) in SYSCALL_CONSTANTS {
        let qualified = format!("core.sys.linux.syscall.{name}");
        let mut matches = module.functions.iter().filter(|f| module.get_string(f.name) == Some(&qualified));
        let accessor = matches.next().unwrap_or_else(|| panic!("missing {qualified}"));
        assert!(matches.next().is_none(), "multiple {qualified} accessors");
        assert!(accessor.is_const && ids.insert(accessor.id), "invalid {qualified} slot");
        let entry = module.find_function_by_name(&qualified).unwrap();
        let value = interpreter.execute_function(entry).unwrap();
        assert!(value.is_int(), "{qualified}: {value:?} at {entry:?} (descriptor {:?})", accessor.id);
        assert_eq!(value.as_i64(), expected, "{qualified}");
    }
}
