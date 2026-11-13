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
fn byte_literals_preserve_the_source_extent_and_unsigned_octets() {
    let result = run(r#"
fn length(bytes: &[Byte]) -> Int { @intrinsic("slice_len", bytes) }
fn middle(bytes: &[Byte]) -> Int { bytes[2] as Int }
fn probe() -> Int {
    let bytes = b"a\0\xFFb";
    length(b"") + length(bytes) * 1000 + middle(bytes) * 10 + bytes[1] as Int
}
"#)
        .expect("literal slice");
    assert_eq!(result.as_i64(), 6550);
}

#[test]
fn literal_storage_is_reused_until_the_interpreter_is_reset() {
    let mut module = verum_vbc::module::VbcModule::new("literal_lifetime".into());
    let constant = module.add_constant(verum_vbc::module::Constant::Bytes(vec![0, 128, 255]));
    let mut function = verum_vbc::module::FunctionDescriptor::new(verum_vbc::types::StringId::EMPTY);
    function.id = verum_vbc::FunctionId(0);
    function.register_count = 1;
    for instruction in [
        verum_vbc::Instruction::LoadK { dst: verum_vbc::Reg(0), const_id: constant.0 },
        verum_vbc::Instruction::Ret { value: verum_vbc::Reg(0) },
    ] {
        verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    let wire = verum_vbc::serialize::serialize_module(&module).expect("serialize");
    let module = verum_vbc::deserialize::deserialize_module(&wire).expect("reload");
    let mut vm = Interpreter::new(Arc::new(module));
    let first = vm.execute_function(verum_vbc::FunctionId(0)).expect("first load");
    let allocations = vm.state.heap.stats().total_allocs;
    for _ in 0..20 {
        assert_eq!(vm.execute_function(verum_vbc::FunctionId(0)).expect("reload literal"), first);
        assert_eq!(vm.state.heap.stats().total_allocs, allocations);
    }
    vm.reset();
    assert!(vm.state.literal_cache.is_empty());
    let fresh = vm.execute_function(verum_vbc::FunctionId(0)).expect("after reset");
    let (ptr, len) = verum_vbc::interpreter::value_as_byte_slice(&fresh).expect("byte view");
    assert_eq!(len, 3);
    // SAFETY: the live interpreter owns the literal storage for the extent above.
    assert_eq!(unsafe { std::slice::from_raw_parts(ptr, len as usize) }, &[0, 128, 255]);
}

#[test]
fn literal_borrows_survive_the_producing_scope_and_repeated_calls() {
    let value = run(r#"
fn bytes() -> &[Byte] { let data = b"ABC"; data }
fn read() -> Int { let first = bytes(); let second = bytes(); first[0] as Int + second[2] as Int }
fn probe() -> Int { read() + read() }
"#).expect("module-owned bytes");
    assert_eq!(value.as_i64(), 264);
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

#[test]
fn subslices_of_projected_fixed_arrays_use_the_selected_container() {
    let value = run(r#"
        fn read(a: &[Byte;4], b: &[Byte;4]) -> Int {
            let left = &a[1..4];
            let right = &b[..];
            (left[2] as Int) * 10 + right[3] as Int
        }
        fn from_rows(rows: &List<[Byte;4]>) -> Int { read(&rows[0], &rows[1]) }
        fn probe() -> Int {
            let mut rows: List<[Byte;4]> = [];
            rows.push([3_u8;4]); rows.push([5_u8;4]);
            from_rows(&rows)
        }
    "#).expect("borrowed rows");
    assert_eq!(value.as_i64(), 35);
}
