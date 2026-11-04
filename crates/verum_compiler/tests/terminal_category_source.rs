//! Whole stdlib declarations and implementations through source checking and VBC wire execution.
//! These tests establish the terminal model, not universal accessibility or kernel law evidence.
use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
const CONSUMER: &str = r#"module terminal_test;
mount core.math.category.{Category};
mount core.math.concrete_accessible.{TerminalCategory,terminal_left_identity,terminal_right_identity,terminal_associativity};
fn generic_laws<C:Category<(),()>>() -> Bool {
 let arrow:()=();
 C.left_identity(&arrow) && C.right_identity(&arrow) && C.associativity(&arrow,&arrow,&arrow)
}
fn probe()->Int { let arrow:()=();
 if terminal_left_identity(&arrow) && terminal_right_identity(&arrow) && terminal_associativity(&arrow,&arrow,&arrow) && generic_laws<TerminalCategory>() { 7 } else { 0 }
}
"#;
fn modules() -> List<(&'static str, Text, Module)> {
    [
        (
            "core.math.category",
            Text::from(std::fs::read_to_string(root().join("core/math/category.vr")).unwrap()),
        ),
        (
            "core.math.concrete_accessible",
            Text::from(
                std::fs::read_to_string(root().join("core/math/concrete_accessible.vr")).unwrap(),
            ),
        ),
        ("terminal_test", Text::from(CONSUMER)),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (owner, source))| {
        let ast = FastParser::new()
            .parse_module_str(&source, FileId::new(i as u32))
            .unwrap();
        (owner, source, ast)
    })
    .collect()
}
#[test]
fn actual_modules_admit_terminal_and_generic_consumer() {
    let modules = modules();
    let mut registry = ModuleRegistry::new();
    for (i, (owner, source, ast)) in modules.iter().enumerate() {
        let id = ModuleId::new(i as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(
            id,
            path.clone(),
            ast.clone(),
            FileId::new(i as u32),
            source.clone().into(),
        );
        info.exports = extract_exports_from_module(ast, id, &path).unwrap();
        registry.register(info);
    }
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_module_registry_direct(registry.clone());
    for (owner, _, ast) in &modules {
        checker.set_current_module_path(*owner);
        for item in &ast.items {
            if let ItemKind::Mount(m) = &item.kind {
                checker.process_import(m, owner, &registry).unwrap();
            }
        }
        let e = checker.prepare_module_declarations(ast);
        assert!(e.is_empty(), "{owner}: {e:?}");
        for item in &ast.items {
            checker
                .check_item(item)
                .unwrap_or_else(|e| panic!("{owner}: {e:?}"));
        }
        assert!(
            checker.take_deferred_errors().is_empty(),
            "{owner}: deferred"
        );
        let e: List<Text> = checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(d.message()))
            .collect();
        assert!(e.is_empty(), "{owner}: {e:?}");
    }
}
#[test]
fn actual_modules_execute_through_wire() {
    let modules = modules();
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("terminal_test").with_validation());
    for (_, _, ast) in &modules {
        cg.collect_protocol_definitions(ast);
    }
    for (_, _, ast) in &modules {
        cg.collect_non_protocol_declarations(ast).unwrap();
    }
    for (owner, _, ast) in &modules {
        cg.compile_module_items(ast)
            .unwrap_or_else(|e| panic!("{owner}: {e:?}"));
    }
    let m = cg.finalize_module().unwrap();
    let bytes = serialize_module(&m).unwrap();
    let restored = deserialize_module(&bytes).unwrap();
    for m in [m, restored] {
        let entry = m.find_function_by_name("terminal_test.probe").unwrap();
        let value = Interpreter::new(Arc::new(m))
            .execute_function(entry)
            .unwrap();
        assert_eq!(value.try_as_i64(), Some(7));
    }
}
