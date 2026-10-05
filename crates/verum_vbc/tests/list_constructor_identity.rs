//! List storage selection follows the declared element identity.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{codegen::VbcCodegen, interpreter::Interpreter, module::VbcModule};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    VbcCodegen::new().compile_module(&ast).expect("source VBC")
}
fn stride(declarations: &str, element: &str, constructor: &str) -> i64 {
    let source = format!(
        r#"{declarations}
        fn probe() -> Int {{
            let values: List<{element}> = List<{element}>.{constructor};
            @intrinsic("list_storage_stride", values)
        }}"#
    );
    let module = compile(&source);
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .unwrap()
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .unwrap()
        .as_i64()
}
#[test]
fn nominal_byte_keeps_value_slot_storage_for_both_constructors() {
    for constructor in ["new()", "with_capacity(3)"] {
        assert_eq!(
            stride("type Byte is { x: Int, y: Int };", "Byte", constructor),
            8,
            "{constructor}"
        );
    }
}
#[test]
fn canonical_byte_keeps_packed_storage_for_both_constructors() {
    for constructor in ["new()", "with_capacity(3)"] {
        assert_eq!(stride("", "Byte", constructor), 1, "{constructor}");
    }
}
#[test]
fn qualified_same_leaf_element_keeps_its_declaration_identity() {
    for declarations in [
        "module alpha { public type Byte is { x:Int }; } module beta { public type Byte is { x:Int,y:Int }; }",
        "module beta { public type Byte is { x:Int,y:Int }; } module alpha { public type Byte is { x:Int }; }",
    ] {
        for element in ["alpha.Byte", "beta.Byte"] {
            assert_eq!(
                stride(declarations, element, "with_capacity(3)"),
                8,
                "{element}"
            );
        }
    }
}
#[test]
fn a_reference_to_byte_is_a_value_slot_not_a_packed_byte() {
    assert_eq!(stride("", "&Byte", "new()"), 8);
}
#[test]
fn generic_parameter_named_byte_does_not_select_a_concrete_encoding() {
    let module = compile("fn construct<Byte>() -> List<Byte> { List<Byte>.new() }");
    let function = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("construct"))
        .unwrap();
    let body = &module.bytecode[function.bytecode_offset as usize
        ..(function.bytecode_offset + function.bytecode_length) as usize];
    let ops = verum_vbc::bytecode::decode_instructions(body).unwrap();
    assert!(
        !ops.iter().any(|op| matches!(
            op,
            verum_vbc::instruction::Instruction::MemExtended { sub_op: 0x06, .. }
        )),
        "unresolved parameter became canonical Byte: {ops:?}"
    );
}
#[test]
fn alias_of_nominal_byte_does_not_acquire_packed_storage() {
    assert_eq!(
        stride(
            "type Byte is { x:Int,y:Int }; type Element is Byte;",
            "Element",
            "with_capacity(3)"
        ),
        8
    );
}
