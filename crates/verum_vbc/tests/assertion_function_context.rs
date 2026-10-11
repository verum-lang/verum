//! T1756: failed assertions identify the active function without changing payloads.
#![cfg(feature = "codegen")]

use verum_common::Shared;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, Value,
    bytecode::decode_instruction,
    codegen::{CodegenConfig, VbcCodegen},
    interpreter::{Interpreter, InterpreterError},
    module::VbcModule,
};

const SOURCE: &str = r#"
module diagnostic.assertion;
fn first(flag: Bool) -> Int { assert(flag, "retained failure"); 99 }
fn second(flag: Bool) -> Int { assert(flag, "retained failure"); 99 }
fn caller() -> Int { first(false) }
fn passing() -> Int { first(true) }
"#;

fn source_or_wire(wire: bool) -> (&'static str, VbcModule) {
    let ast = Parser::new(SOURCE).parse_module().expect("fixture grammar");
    let source = VbcCodegen::with_config(CodegenConfig::new("diagnostic.assertion"))
        .compile_module(&ast)
        .expect("parsed source codegen");
    if wire {
        let bytes = verum_vbc::serialize::serialize_module(&source).expect("serialize source");
        let decoded =
            verum_vbc::deserialize::deserialize_module(&bytes).expect("decode source wire");
        ("wire", decoded)
    } else {
        ("source", source)
    }
}

fn assertion_end_pc(module: &VbcModule, name: &str) -> usize {
    let id = module.find_function_by_name(name).expect("exact function");
    let descriptor = module.get_function(id).expect("exact descriptor");
    let begin = descriptor.bytecode_offset as usize;
    let bytes = &module.bytecode[begin..begin + descriptor.bytecode_length as usize];
    let mut pc = 0;
    let mut assertion = None;
    while pc < bytes.len() {
        if matches!(
            decode_instruction(bytes, &mut pc).expect("decode instruction"),
            Instruction::Assert { .. }
        ) {
            assert!(assertion.is_none(), "one assertion expected in {name}");
            assertion = Some(pc);
        }
    }
    assertion.expect("actual Assert instruction")
}

fn check_error(error: InterpreterError, name: &str, expected_pc: usize, route: &str) {
    let InterpreterError::AssertionFailed {
        message,
        pc,
        function,
    } = &error
    else {
        panic!("{route}: wrong error class: {error:?}");
    };
    assert_eq!(
        message, "retained failure",
        "{route}: original panic payload"
    );
    assert_eq!(*pc, expected_pc, "{route}: post-operand function-local PC");
    assert_eq!(
        function.as_ref().map(|value| value.as_str()),
        Some(name),
        "{route}: exact structured function identity"
    );
    assert_eq!(
        error.to_string(),
        format!("Assertion failed at pc {expected_pc} in {name}: retained failure"),
        "{route}: only the active function is added to the diagnostic",
    );
}

fn nested_failure(wire: bool) {
    let (route, module) = source_or_wire(wire);
    let name = "diagnostic.assertion.first";
    let pc = assertion_end_pc(&module, name);
    let caller = module
        .find_function_by_name("diagnostic.assertion.caller")
        .unwrap();
    let error = Interpreter::new(Shared::new(module).into_arc())
        .execute_function(caller)
        .expect_err("nested assertion must fail");
    check_error(error, name, pc, route);
}

fn same_local_pc(wire: bool) {
    let (route, module) = source_or_wire(wire);
    let first = "diagnostic.assertion.first";
    let second = "diagnostic.assertion.second";
    let pc = assertion_end_pc(&module, first);
    assert_eq!(assertion_end_pc(&module, second), pc, "same-PC control");
    let module = Shared::new(module);
    for name in [first, second] {
        let function = module.find_function_by_name(name).unwrap();
        let error = Interpreter::new(module.clone().into_arc())
            .execute_function_with_args(function, &[Value::from_bool(false)])
            .expect_err("both assertions must fail");
        check_error(error, name, pc, route);
    }
}

fn passing_assertion(wire: bool) {
    let (route, module) = source_or_wire(wire);
    let function = module
        .find_function_by_name("diagnostic.assertion.passing")
        .unwrap();
    let result = Interpreter::new(Shared::new(module).into_arc())
        .execute_function(function)
        .expect("true assertion still succeeds");
    assert_eq!(result.as_i64(), 99, "{route}");
}

#[test]
fn source_nested_failure_names_the_executing_callee() {
    nested_failure(false);
}
#[test]
fn wire_nested_failure_names_the_executing_callee() {
    nested_failure(true);
}
#[test]
fn source_identical_local_pcs_keep_distinct_function_identities() {
    same_local_pc(false);
}
#[test]
fn wire_identical_local_pcs_keep_distinct_function_identities() {
    same_local_pc(true);
}
#[test]
fn source_passing_assertion_preserves_its_return() {
    passing_assertion(false);
}
#[test]
fn wire_passing_assertion_preserves_its_return() {
    passing_assertion(true);
}

#[test]
fn unavailable_function_name_preserves_the_legacy_diagnostic() {
    for wire in [false, true] {
        let (route, mut module) = source_or_wire(wire);
        let name = "diagnostic.assertion.first";
        let pc = assertion_end_pc(&module, name);
        let id = module.find_function_by_name(name).unwrap();
        module
            .functions
            .iter_mut()
            .find(|function| function.id == id)
            .unwrap()
            .name = verum_vbc::types::StringId(u32::MAX);
        let error = Interpreter::new(Shared::new(module).into_arc())
            .execute_function_with_args(id, &[Value::from_bool(false)])
            .expect_err("unnamed descriptor still executes its assertion");
        let InterpreterError::AssertionFailed {
            message,
            pc: actual_pc,
            function,
        } = &error
        else {
            panic!("{route}: wrong error class: {error:?}");
        };
        assert_eq!(message, "retained failure");
        assert_eq!(*actual_pc, pc);
        assert!(function.is_none());
        assert_eq!(
            error.to_string(),
            format!("Assertion failed at pc {pc}: retained failure")
        );
    }
}
