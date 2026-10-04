#![cfg(feature = "codegen")]
//! T1537: preserve a raw-address conversion without changing interpreter aliasing.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::instruction::{CbgrSubOpcode, Instruction};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str, expected: i64) {
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::new().compile_module(&ast).expect("compile");
    assert!(
        decode_instructions(&module.bytecode)
            .unwrap()
            .iter()
            .any(|i| matches!(i,
        Instruction::CbgrExtended { sub_op, .. } if *sub_op == CbgrSubOpcode::ToRawPtr as u8))
    );
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .unwrap()
        .id;
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), expected);
}

#[test]
fn unsafe_cast_of_aliased_field_updates_original_storage() {
    run(
        r#"
        type Cell is { padding: Int, value: Int };
        fn probe() -> Int {
            let mut cell = Cell { padding: 41, value: 17 };
            let alias = &mut cell.value;
            let raw = alias as &unsafe Int;
            unsafe { *raw = 77; }
            cell.padding * 100 + cell.value
        }
    "#,
        4177,
    );
}

#[test]
fn forwarded_scalar_reference_keeps_its_pointer_cast() {
    run(
        r#"
        type Cell is { padding: Int, value: Int };
        fn read_raw(p: &Int) -> Int { unsafe { *(p as &unsafe Int) } }
        fn probe() -> Int {
            let cell = Cell { padding: 41, value: 17 };
            read_raw(&cell.value)
        }
    "#,
        17,
    );
}

#[cfg(target_os = "macos")]
#[test]
fn forwarded_field_reference_reaches_futex_mismatch_precheck() {
    run(
        r#"
        type Cell is { padding: Int, value: Int };
        fn wait_once(addr: &Int) -> Int {
            @intrinsic("futex_wait", addr as &unsafe Byte, 8, 0_u64)
        }
        fn probe() -> Int {
            let cell = Cell { padding: 41, value: 7 };
            wait_once(&cell.value)
        }
    "#,
        -11,
    );
}
