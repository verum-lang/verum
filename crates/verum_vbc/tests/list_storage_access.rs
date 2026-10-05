//! Container-owned accesses preserve storage encoding and check capacity ranges.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    interpreter::{Interpreter, InterpreterError},
};

fn execute(source: &str) -> Result<verum_vbc::Value, InterpreterError> {
    let mut ast = Parser::new(source).parse_module().expect("source grammar");
    let mut memory = Parser::new(include_str!("../../../core/intrinsics/memory.vr"))
        .parse_module()
        .expect("memory grammar");
    memory.items.retain(|item| matches!(&item.kind, verum_ast::ItemKind::Function(f) if ["list_storage_read", "list_storage_write", "list_storage_move"].contains(&f.name.name.as_str())));
    ast.items.extend(memory.items);
    let module = VbcCodegen::new().compile_module(&ast).expect("source VBC");
    let wire = verum_vbc::serialize::serialize_module(&module).expect("source wire");
    let module = verum_vbc::deserialize::deserialize_module(&wire).expect("source roundtrip");
    assert!(module.header.version_minor >= 21);
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .unwrap()
        .id;
    Interpreter::new(Arc::new(module)).execute_function(entry)
}
#[test]
fn byte_and_value_slot_reads_and_writes_agree() {
    for ty in ["Byte", "Int"] {
        let source = format!(
            r#"fn probe()->Int {{
            let mut values:List<{ty}> =List<{ty}>.with_capacity(3);
            @intrinsic("list_storage_write", values, 0, 0);
            @intrinsic("list_storage_write", values, 1, 255);
            @intrinsic("list_storage_write", values, 2, 42);
            @intrinsic("list_storage_read", values, 0) + @intrinsic("list_storage_read", values, 1) + @intrinsic("list_storage_read", values, 2)
        }}"#
        );
        assert_eq!(execute(&source).unwrap().as_i64(), 297, "{ty}");
    }
}
#[test]
fn overlapping_moves_preserve_both_directions() {
    for ty in ["Byte", "Int"] {
        for (src, dst) in [(0, 1), (1, 0)] {
            let source = format!(
                r#"fn probe()->Int {{
                let mut values:List<{ty}> =List<{ty}>.with_capacity(4);
                @intrinsic("list_storage_write", values, 0, 11);
                @intrinsic("list_storage_write", values, 1, 22);
                @intrinsic("list_storage_write", values, 2, 33);
                @intrinsic("list_storage_write", values, 3, 44);
                @intrinsic("list_storage_move", values, {src}, {dst}, 3);
                @intrinsic("list_storage_read", values, 0)*1000000 + @intrinsic("list_storage_read", values, 1)*10000 + @intrinsic("list_storage_read", values, 2)*100 + @intrinsic("list_storage_read", values, 3)
            }}"#
            );
            let expected = if src == 0 { 11112233 } else { 22334444 };
            assert_eq!(
                execute(&source).unwrap().as_i64(),
                expected,
                "{ty} {src}->{dst}"
            );
        }
    }
}
#[test]
fn record_and_float_values_are_not_reinterpreted_as_raw_inline_bytes() {
    let record = r#"type Cell is {x:Int,y:Int,z:Int}; fn probe()->Int {
        let values:List<Cell> =List<Cell>.with_capacity(2);
        @intrinsic("list_storage_write", values, 0, Cell{x:11,y:37,z:99});
        @intrinsic("list_storage_move", values, 0, 1, 1);
        let result:Cell = @intrinsic("list_storage_read", values, 1);
        result.y
    }"#;
    assert_eq!(execute(record).unwrap().as_i64(), 37);
    let float = r#"fn probe()->Float { let values:List<Float> =List<Float>.with_capacity(1); @intrinsic("list_storage_write", values, 0, 1.25); @intrinsic("list_storage_read", values, 0) }"#;
    assert_eq!(execute(float).unwrap().as_f64(), 1.25);
}
#[test]
fn invalid_ranges_are_refused_before_memory_access() {
    for operation in [
        "@intrinsic(\"list_storage_read\", values, -1)",
        "@intrinsic(\"list_storage_read\", values, 2)",
        "@intrinsic(\"list_storage_write\", values, 2, 7)",
        "@intrinsic(\"list_storage_move\", values, 0, 1, 2)",
        "@intrinsic(\"list_storage_move\", values, 0, 0, -1)",
    ] {
        let source = format!(
            "fn probe()->Int {{let values:List<Int> =List<Int>.with_capacity(2); {operation}; 0}}"
        );
        let error = execute(&source).unwrap_err().to_string();
        assert!(error.contains("List storage range"), "{operation}: {error}");
    }
}
#[test]
fn zero_length_move_accepts_empty_storage_without_dereferencing_null() {
    let source = r#"fn probe()->Int { let values:List<Int> =List<Int>.new(); @intrinsic("list_storage_move", values, 0, 0, 0); 7 }"#;
    assert_eq!(execute(source).unwrap().as_i64(), 7);
}
#[test]
fn foreign_nominal_cannot_supply_a_storage_address() {
    let source = r#"type Foreign is {len:Int,cap:Int,ptr:Int}; fn probe()->Int { let values=Foreign{len:0,cap:1,ptr:0}; @intrinsic("list_storage_read", values, 0) }"#;
    let error = execute(source).unwrap_err().to_string();
    assert!(error.contains("unknown List storage type"), "{error}");
}

#[test]
fn public_unsafe_declarations_preserve_slot_values() {
    let source = r#"type Triple is {x:Int,y:Int,z:Int}; fn probe()->Int {
        let mut values:List<Triple> =List<Triple>.with_capacity(2);
        unsafe {
            list_storage_write(&mut values,0,Triple{x:11,y:37,z:99});
            list_storage_move(&mut values,0,1,1);
            let result:Triple = list_storage_read(&values,1);
            result.y
        }
    }"#;
    assert_eq!(execute(source).unwrap().as_i64(), 37);
}
