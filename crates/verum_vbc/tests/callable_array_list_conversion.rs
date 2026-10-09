//! T1700: declared List returns must own List storage after packed-array coercion.
//! Actual parsed source, serialization and interpreter execution; no stdlib bake.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn runs(source: &str) {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("array_list_returns"))
        .compile_module(&ast).expect("source lowering");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("wire encode");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire decode");
    let entry = module.functions.iter().find(|function| module.get_string(function.name)
        .is_some_and(|name| name == "probe" || name.ends_with(".probe")))
        .expect("declared probe").id;
    let actual = Interpreter::new(Arc::new(module)).execute_function(entry)
        .expect("declared List supports indexing, length and growth");
    assert_eq!(actual.as_i64(), 1);
}

#[test]
fn list_literal_return_remains_growable() {
    runs(r#"
fn make() -> List<Byte> { [1 as Byte, 2 as Byte] }
fn probe() -> Int { let mut xs = make(); xs.push(3 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1); assert_eq(xs[2], 3); 1 }
"#);
}

#[test]
fn packed_byte_local_tail_becomes_a_list() {
    runs(r#"
fn make() -> List<Byte> { let xs: [Byte; 2] = [1, 2]; xs }
fn probe() -> Int { let mut xs = make(); xs.push(3 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1); assert_eq(xs[2], 3); 1 }
"#);
}

#[test]
fn packed_byte_explicit_return_becomes_a_list() {
    runs(r#"
fn make() -> List<Byte> { let xs: [Byte; 2] = [1, 2]; return xs; }
fn probe() -> Int { let mut xs = make(); xs.push(3 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1); assert_eq(xs[2], 3); 1 }
"#);
}

#[test]
fn packed_nonbyte_tail_preserves_element_width() {
    runs(r#"
fn make() -> List<UInt32> { let mut xs: [UInt32; 2] = [0; 2];
    xs[0] = 65537; xs[1] = 262147; xs }
fn probe() -> Int { let mut xs = make(); xs.push(7 as UInt32);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 65537); assert_eq(xs[1], 262147);
    assert_eq(xs[2], 7); 1 }
"#);
}

#[test]
fn packed_float_tail_preserves_values() {
    runs(r#"
fn make() -> List<Float> { let mut xs: [Float; 2] = [0.0; 2];
    xs[0] = 1.25; xs[1] = -2.5; xs }
fn probe() -> Int { let mut xs = make(); xs.push(3.75);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1.25); assert_eq(xs[1], -2.5);
    assert_eq(xs[2], 3.75); 1 }
"#);
}

#[test]
fn empty_packed_return_can_grow() {
    runs(r#"
fn make() -> List<Byte> { let xs: [Byte; 0] = []; xs }
fn probe() -> Int { let mut xs = make(); xs.push(7 as Byte);
    assert_eq(xs.len(), 1); assert_eq(xs[0], 7); 1 }
"#);
}

#[test]
fn explicit_closure_list_return_has_its_own_boundary() {
    runs(r#"
fn probe() -> Int {
    let make = || -> List<Byte> { let xs: [Byte; 2] = [1, 2]; return xs; };
    let mut xs = make(); xs.push(3 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1); assert_eq(xs[2], 3); 1
}
"#);
}

#[test]
fn fixed_array_return_inside_list_function_stays_fixed() {
    runs(r#"
fn make() -> List<Byte> {
    let fixed = || -> [Byte; 2] { let xs: [Byte; 2] = [4, 5]; xs };
    let xs: [Byte; 2] = fixed();
    [xs[0], xs[1]]
}
fn probe() -> Int { let mut xs = make(); xs.push(6 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 4); assert_eq(xs[2], 6); 1 }
"#);
}

#[test]
fn list_input_forwarding_keeps_existing_storage() {
    runs(r#"
fn forward(xs: List<Byte>) -> List<Byte> { xs }
fn probe() -> Int { let input: List<Byte> = [1, 2];
    let mut xs = forward(input); xs.push(3 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 1); assert_eq(xs[2], 3); 1 }
"#);
}

// A declared fixed array is not physical packed-storage proof. These callees
// intentionally use the existing List-backed literal producer for the same type.
#[test]
fn an_inferred_array_call_backed_by_a_list_is_not_reinterpreted() {
    runs(r#"
fn array_value() -> [Byte; 2] { [11 as Byte, 12 as Byte] }
fn make() -> List<Byte> { let xs = array_value(); xs }
fn probe() -> Int { let mut xs = make(); xs.push(13 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 11); assert_eq(xs[2], 13); 1 }
"#);
}

#[test]
fn replacing_a_packed_binding_does_not_reuse_its_old_storage_fact() {
    runs(r#"
fn array_value() -> [Byte; 2] { [11 as Byte, 12 as Byte] }
fn make() -> List<Byte> {
    let mut xs: [Byte; 2] = [1, 2]; xs = array_value(); xs
}
fn probe() -> Int { let mut xs = make(); xs.push(13 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 11); assert_eq(xs[2], 13); 1 }
"#);
}

#[test]
fn nested_blocks_preserve_the_actual_packed_result_move() {
    runs(r#"
fn make() -> List<Byte> { { let xs: [Byte; 2] = [4, 5]; xs } }
fn probe() -> Int { let mut xs = make(); xs.push(6 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 4); assert_eq(xs[2], 6); 1 }
"#);
}

#[test]
fn temporary_register_reuse_does_not_make_a_list_packed() {
    runs(r#"
fn make() -> List<Byte> {
    { let xs: [Byte; 2] = [4, 5]; }
    let xs: List<Byte> = [7, 8]; xs
}
fn probe() -> Int { let mut xs = make(); xs.push(9 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 7); assert_eq(xs[2], 9); 1 }
"#);
}

#[test]
fn deferred_packed_assignment_does_not_relabel_the_current_list() {
    runs(r#"
fn array_value() -> [Byte; 2] { [11 as Byte, 12 as Byte] }
fn make() -> List<Byte> {
    let mut xs = array_value();
    defer { xs = { let replacement: [Byte; 2] = [1, 2]; replacement }; }
    return xs;
}
fn probe() -> Int { let mut xs = make(); xs.push(13 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 11); assert_eq(xs[1], 12); 1 }
"#);
}

#[test]
fn deferred_call_does_not_erase_the_current_packed_producer() {
    runs(r#"
fn cleanup() -> Int { 7 }
fn make() -> List<Byte> {
    let xs: [Byte; 2] = [4, 5];
    defer { cleanup(); }
    return xs;
}
fn probe() -> Int { let mut xs = make(); xs.push(6 as Byte);
    assert_eq(xs.len(), 3); assert_eq(xs[0], 4); assert_eq(xs[1], 5); 1 }
"#);
}
