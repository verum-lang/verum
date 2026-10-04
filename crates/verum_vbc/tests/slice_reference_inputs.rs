#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::{Interpreter, InterpreterError};
use verum_vbc::value::Value;

fn run(source: &str) -> Result<Value, InterpreterError> {
    let ast = Parser::new(source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("slice_refs"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    Interpreter::new(Arc::new(module)).execute_function(entry)
}

#[test]
fn subslice_of_a_borrowed_collection_reads_its_elements() {
    let result = run(r#"
fn read(values: &[Int]) -> Int {
    let part = &values[1..3];
    part[0] + part[1]
}
fn probe() -> Int { let values = [11, 42, 99]; read(&values) }
"#)
    .expect("borrowed collection subslice");
    assert_eq!(result.as_i64(), 141);
}

#[test]
fn subslice_of_an_existing_slice_preserves_its_offset() {
    let result = run(r#"
fn read(values: &[Int]) -> Int { let part = &values[1..2]; part[0] }
fn probe() -> Int { let values = [11, 42, 99]; read(&values[1..3]) }
"#)
    .expect("subslice");
    assert_eq!(result.as_i64(), 99);
}

#[test]
fn subslice_preserves_packed_byte_stride() {
    let result = run(r#"
fn read(values: &[Byte]) -> Int {
    let part = &values[1..3];
    (part[0] as Int) + (part[1] as Int)
}
fn probe() -> Int {
    let values: [Byte; 3] = [11 as Byte, 42 as Byte, 99 as Byte];
    read(&values)
}
"#)
    .expect("packed byte subslice");
    assert_eq!(result.as_i64(), 141);
}

#[test]
fn empty_source_allows_an_empty_subslice() {
    let result = run(r#"
fn length(values: &[Int]) -> Int { let part = &values[0..0]; part.len() }
fn probe() -> Int { let values: [Int; 0] = []; length(&values) }
"#)
    .expect("empty source slice");
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn mutable_subslice_writes_into_the_original_collection() {
    let result = run(r#"
fn write(values: &mut [Int]) {
    let part = &mut values[1..3];
    part[0] = 7;
    part[1] = 9;
}
fn probe() -> Int {
    let mut values = [11, 42, 99];
    write(&mut values);
    values[0] * 100 + values[1] * 10 + values[2]
}
"#)
    .expect("write through subslice");
    assert_eq!(result.as_i64(), 1179);
}

#[test]
fn empty_subslice_at_the_end_is_valid() {
    let result = run(r#"
fn length(values: &[Int]) -> Int { let part = &values[3..3]; part.len() }
fn probe() -> Int { let values = [11, 42, 99]; length(&values) }
"#)
    .expect("empty subslice");
    assert_eq!(result.as_i64(), 0);
}

#[test]
fn subslice_rejects_out_of_bounds_before_exposing_a_pointer() {
    for argument in ["&values", "&values[..]"] {
        let source = format!(
            r#"
fn length(values: &[Int]) -> Int {{ let part = &values[1..4]; part.len() }}
fn probe() -> Int {{ let values = [11, 42, 99]; length({argument}) }}
"#
        );
        let error = run(&source).expect_err("out-of-bounds subslice must fail at construction");
        assert!(error.to_string().contains("slice"), "{error}");
    }
}

#[test]
fn subslice_rejects_negative_start_and_reversed_ranges() {
    for range in ["-1..2", "2..1"] {
        let source = format!(
            r#"
fn length(values: &[Int]) -> Int {{ let part = &values[{range}]; part.len() }}
fn probe() -> Int {{ let values = [11, 42, 99]; length(&values) }}
"#
        );
        let error = run(&source).expect_err("invalid subslice must fail at construction");
        assert!(error.to_string().contains("slice"), "{error}");
    }
}
