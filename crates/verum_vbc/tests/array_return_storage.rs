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
            let body = module
                .functions
                .iter()
                .find(|f| module.get_string(f.name) == Some(name))
                .and_then(|f| f.instructions.as_deref())
                .expect("exact caller body");
            assert!(
                body.iter().any(|i| matches!(i, Instruction::GetE { .. })),
                "{name}: source order {callee_first}: {body:?}"
            );
            assert!(
                !body.iter().any(|i| matches!(i,
                Instruction::MemExtended { sub_op, .. }
                if *sub_op == MemSubOpcode::ByteArrayLoad.to_byte()
                    || *sub_op == MemSubOpcode::TypedArrayLoad.to_byte())),
                "{name}: the declaration cannot authorize a packed load"
            );
        }
    }
}

fn execute_both(source: &str, expected: i64) {
    use verum_common::Shared;
    use verum_vbc::interpreter::Interpreter;
    let module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("grammar"))
        .expect("source lowering");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("wire encode"),
    )
    .expect("wire decode");
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .functions
            .iter()
            .find(|f| module.get_string(f.name) == Some("probe"))
            .expect("exact probe")
            .id;
        let actual = Interpreter::new(Shared::new(module).into_arc())
            .execute_function(entry)
            .unwrap_or_else(|error| panic!("{route}: {error}"));
        assert_eq!(actual.as_i64(), expected, "{route}");
    }
}

#[test]
fn interpreter_array_call_bindings_keep_storage_and_mutation_in_both_orders() {
    for first in [true, false] {
        for binding in ["let mut bytes =", "let mut bytes: [Byte; 2] ="] {
            for body in [
                "let bytes: [Byte; 2] = [7, 9]; bytes",
                "[7 as Byte, 9 as Byte]",
            ] {
                let callee = format!("fn selected() -> [Byte; 2] {{ {body} }}");
                let caller = format!(
                    "fn probe() -> Int {{ {binding} selected(); bytes[1] = 11; (bytes[1] as Int) + bytes.len() }}"
                );
                execute_both(
                    &if first {
                        format!("{callee} {caller}")
                    } else {
                        format!("{caller} {callee}")
                    },
                    13,
                );
            }
        }
    }
}

#[test]
fn interpreter_mixed_array_returns_and_loop_consumers_remain_valid() {
    for flag in ["true", "false"] {
        execute_both(
            &format!(
                r#"
fn selected(flag: Bool) -> [Byte; 2] {{
    if flag {{ let bytes: [Byte; 2] = [7, 9]; bytes }}
    else {{ [7 as Byte, 9 as Byte] }}
}}
fn probe() -> Int {{
    let bytes = selected({flag});
    let mut index = 0;
    let mut total = 0;
    while index < 2 {{ total = total + bytes[index] as Int; index = index + 1; }}
    total
}}
"#
            ),
            16,
        );
    }
}

#[test]
fn interpreter_array_call_bounds_are_actual_allocation_bounds() {
    use verum_common::Shared;
    use verum_vbc::interpreter::Interpreter;
    for index in [-1, 2] {
        let source = format!(
            "fn selected() -> [Byte; 2] {{ let bytes: [Byte; 2] = [7, 9]; bytes }} fn probe() -> Int {{ let bytes = selected(); bytes[{index}] as Int }}"
        );
        let module = VbcCodegen::new()
            .compile_module(&Parser::new(&source).parse_module().expect("grammar"))
            .expect("source lowering");
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).expect("wire"),
        )
        .expect("reload");
        for (route, module) in [("source", module), ("wire", wire)] {
            let entry = module
                .functions
                .iter()
                .find(|f| module.get_string(f.name) == Some("probe"))
                .unwrap()
                .id;
            let error = Interpreter::new(Shared::new(module).into_arc())
                .execute_function(entry)
                .expect_err("outside returned allocation");
            assert!(
                error.to_string().contains("Index out of bounds"),
                "{route}: {error}"
            );
        }
    }
}

#[test]
fn interpreter_array_results_keep_declared_integer_and_float_elements() {
    execute_both(
        "fn selected() -> [UInt32; 2] { let values: [UInt32; 2] = [1, 65539]; values } fn probe() -> Int { let values = selected(); assert_eq(values[1], 65539); 1 }",
        1,
    );
    execute_both(
        "fn selected() -> [Float; 2] { let values: [Float; 2] = [1.25, -0.5]; values } fn probe() -> Int { let values = selected(); assert_eq(values[1], -0.5); 1 }",
        1,
    );
}
