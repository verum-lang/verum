//! T1704: executable bodies, not identical array signatures, establish storage.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    array_storage::{ArrayResultFact, straight_line_array_return},
    codegen::VbcCodegen,
    module::VbcModule,
};

fn source() -> VbcModule {
    let text = r#"
fn packed() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes }
fn listed() -> [Byte; 2] { [7 as Byte, 9 as Byte] }
fn forwarded(bytes: [Byte; 2]) -> [Byte; 2] { bytes }
fn called() -> [Byte; 2] { packed() }
fn mixed(flag: Bool) -> [Byte; 2] {
    if flag { let bytes: [Byte; 2] = [7, 9]; bytes }
    else { [7 as Byte, 9 as Byte] }
}
"#;
    VbcCodegen::new()
        .compile_module(&Parser::new(text).parse_module().expect("source grammar"))
        .expect("source lowering")
}

#[test]
fn source_and_decoded_bodies_keep_same_signature_different_storage() {
    let source = source();
    let mut wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).expect("wire encode"),
    )
    .expect("wire decode");
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end])
            .expect("wire body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    for (route, module) in [("source", &source), ("wire", &wire)] {
        let exact = |name| {
            module
                .functions
                .iter()
                .find(|function| module.get_string(function.name) == Some(name))
                .unwrap_or_else(|| panic!("missing exact source function {name}"))
        };
        assert_eq!(exact("packed").return_type, exact("listed").return_type);
        for (name, expected) in [
            (
                "packed",
                Some(ArrayResultFact::Packed {
                    width: 1,
                    float: false,
                    count: 2,
                }),
            ),
            ("listed", Some(ArrayResultFact::List)),
            ("forwarded", None),
            ("called", None),
            ("mixed", None),
        ] {
            assert_eq!(
                straight_line_array_return(
                    exact(name).instructions.as_deref().expect("completed body")
                ),
                expected,
                "{route}: {name}"
            );
        }
    }
}


#[test]
fn inferred_and_annotated_calls_do_not_guess_storage_from_the_signature() {
    use verum_vbc::instruction::{Instruction, MemSubOpcode};
    for callee_first in [true, false] {
        let callee = "fn packed() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes }";
        let callers = r#"
fn inferred() -> Byte { let bytes = packed(); bytes[1] }
fn annotated() -> Byte { let bytes: [Byte; 2] = packed(); bytes[1] }
"#;
        let text = if callee_first {
            verum_common::Text::from(format!("{callee}\n{callers}"))
        } else {
            verum_common::Text::from(format!("{callers}\n{callee}"))
        };
        let module = VbcCodegen::new()
            .compile_module(&Parser::new(&text).parse_module().expect("source grammar"))
            .expect("source lowering");
        for name in ["inferred", "annotated"] {
            let body = module.functions.iter()
                .find(|f| module.get_string(f.name) == Some(name))
                .and_then(|f| f.instructions.as_deref())
                .expect("exact caller body");
            assert!(body.iter().any(|i| matches!(i, Instruction::GetE { .. })),
                "{name}: source order {callee_first}: {body:?}");
            assert!(!body.iter().any(|i| matches!(i,
                Instruction::MemExtended { sub_op, .. }
                if *sub_op == MemSubOpcode::ByteArrayLoad.to_byte()
                    || *sub_op == MemSubOpcode::TypedArrayLoad.to_byte())),
                "{name}: the declaration cannot authorize a packed load");
        }
    }
}
