//! T1554: a declared TLS value cannot be flattened into a module namespace.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    instruction::Instruction,
    interpreter::Interpreter,
    module::{FunctionId, VbcModule},
};

const OWNER: &str = r#"
type Cell is { value: Int };
implement Cell {
    fn read(&self) -> Int { self.value }
    fn make() -> Cell { Cell { value: 31 } }
}
fn read() -> Int { 99 }
"#;

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("parse source control");
    VbcCodegen::with_config(CodegenConfig::new("static_receiver"))
        .compile_module(&ast)
        .expect("compile source control")
}
fn function(module: &VbcModule, name: &str) -> FunctionId {
    module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|actual| actual == name || actual == format!("static_receiver.{name}"))
        })
        .unwrap_or_else(|| panic!("missing exact function {name}"))
        .id
}
fn value_receiver_module(source: &str) -> (VbcModule, FunctionId) {
    let module = compile(&format!("{OWNER}\n{source}"));
    let entry = function(&module, "probe");
    let free = function(&module, "read");
    let body = module
        .get_function(entry)
        .unwrap()
        .instructions
        .as_ref()
        .unwrap();
    assert!(
        !body.iter().any(|op| match op {
            Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } =>
                *func_id == free.0,
            _ => false,
        }),
        "value receiver must never call the unrelated free read(): {body:?}"
    );
    (module, entry)
}
fn execute_static(module: VbcModule, entry: FunctionId) -> i64 {
    let mut interpreter = Interpreter::new(Arc::new(module));
    interpreter.run_global_ctors().unwrap();
    interpreter.execute_function(entry).unwrap().as_i64()
}
fn verify_receiver(source: &str) {
    let (module, entry) = value_receiver_module(source);
    let body = module
        .get_function(entry)
        .unwrap()
        .instructions
        .as_ref()
        .unwrap();
    assert!(
        body.iter().any(|op| match op {
            Instruction::CallM { method_id, .. } => module
                .get_string(verum_vbc::types::StringId(*method_id))
                .is_some_and(|name| name == "Cell.read" || name == "static_receiver.Cell.read"),
            Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } => {
                module
                    .get_function(FunctionId(*func_id))
                    .and_then(|f| module.get_string(f.name))
                    .is_some_and(|name| name == "Cell.read" || name == "static_receiver.Cell.read")
            }
            _ => false,
        }),
        "receiver's declared method owner must survive: {body:?}"
    );
    assert_eq!(execute_static(module, entry), 7);
}

#[test]
fn immutable_record_static_uses_its_declared_method() {
    verify_receiver("static CELL: Cell = Cell { value: 7 }; fn probe() -> Int { CELL.read() }");
}
#[test]
fn mutable_record_static_uses_its_declared_method() {
    verify_receiver("static mut CELL: Cell = Cell { value: 7 }; fn probe() -> Int { CELL.read() }");
}
#[test]
fn explicit_thread_local_value_uses_its_declared_method() {
    verify_receiver(
        "@thread_local static CELL: Cell = Cell { value: 7 }; fn probe() -> Int { CELL.read() }",
    );
}
#[test]
fn function_local_static_uses_its_scoped_value_identity() {
    verify_receiver("fn probe() -> Int { static CELL: Cell = Cell { value: 7 }; CELL.read() }");
}
#[test]
fn field_of_static_is_a_value_receiver_chain() {
    let (module, entry) = value_receiver_module(
        r#"
type Holder is { inner: Cell };
static HOLDER: Holder = Holder { inner: Cell { value: 7 } };
fn probe() -> Int { HOLDER.inner.read() }
"#,
    );
    let body = module
        .get_function(entry)
        .unwrap()
        .instructions
        .as_ref()
        .unwrap();
    // This checks the flattener's boundary: dispatch consumes the extracted
    // field value. Exact field-method qualification is a separate producer fact.
    let field = body
        .iter()
        .find_map(|op| match op {
            Instruction::GetF { dst, .. } => Some(*dst),
            _ => None,
        })
        .expect("the static's field must actually be evaluated");
    assert!(
        body.iter().any(|op| matches!(op,
            Instruction::CallM { receiver, .. } if *receiver == field
        )),
        "call must receive the extracted field: {body:?}"
    );
    assert_eq!(execute_static(module, entry), 7);
}
#[test]
fn ordinary_local_receiver_keeps_its_existing_precedence() {
    verify_receiver("fn probe() -> Int { let cell = Cell { value: 7 }; cell.read() }");
}
#[test]
fn constant_record_receiver_keeps_its_existing_precedence() {
    verify_receiver("const CELL: Cell = Cell { value: 7 }; fn probe() -> Int { CELL.read() }");
}
#[test]
fn qualified_module_call_keeps_the_requested_free_function() {
    let module = compile(
        r#"
module helper { public fn read() -> Int { 31 } }
fn read() -> Int { 99 }
fn probe() -> Int { helper.read() }
"#,
    );
    let entry = function(&module, "probe");
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap()
            .as_i64(),
        31
    );
}
#[test]
fn type_namespace_call_keeps_the_requested_associated_function() {
    let module = compile(&format!(
        "{OWNER}\nfn make() -> Int {{ 99 }}\nfn probe() -> Int {{ Cell.make().value }}"
    ));
    let entry = function(&module, "probe");
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(entry)
            .unwrap()
            .as_i64(),
        31
    );
}
