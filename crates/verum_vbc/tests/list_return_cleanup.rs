//! T1710: callable conversion precedes lexical cleanup without removing it.
#![cfg(feature = "codegen")]
#[path = "fixtures/list_return_cleanup.rs"]
mod fixtures;

use verum_common::{List, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    array_storage::{ArrayResultFact, ArrayResultFacts},
    codegen::VbcCodegen,
    instruction::{Instruction, MemSubOpcode},
    interpreter::Interpreter,
    module::VbcModule,
};

fn source_and_wire(source: &str) -> [(&'static str, VbcModule); 2] {
    let parsed = Parser::new(source).parse_module().expect("fixture grammar");
    let module = VbcCodegen::new()
        .compile_module(&parsed)
        .expect("source lowering");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("encode"),
    )
    .expect("decode");
    [("source", module), ("wire", wire)]
}

fn body(module: &VbcModule, name: &str) -> List<Instruction> {
    let function = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some(name))
        .expect("exact function");
    let start = function.bytecode_offset as usize;
    verum_vbc::bytecode::decode_instructions(
        &module.bytecode[start..start + function.bytecode_length as usize],
    )
    .expect("decode actual instructions")
    .into_iter()
    .collect()
}

fn check_payload(source: &str, expected: &[i64]) {
    let mut failures: List<Text> = List::new();
    for (route, module) in source_and_wire(source) {
        let probe = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some("probe"))
            .expect("exact probe")
            .id;
        let mut interpreter = Interpreter::new(Shared::new(module).into_arc());
        let value = match interpreter.execute_function(probe) {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!("{route}: {error}").into());
                continue;
            }
        };
        if !value.is_ptr() || !interpreter.state.heap.contains(value.as_ptr()) {
            failures.push(format!("{route}: returned value is not an owned object").into());
            continue;
        }
        let elements = interpreter.state.list_elements(value);
        let actual = elements.as_ref().and_then(|values| {
            values
                .iter()
                .map(|value| value.is_int().then(|| value.as_i64()))
                .collect::<Option<List<_>>>()
        });
        if actual.as_deref() != Some(expected) {
            failures.push(
                format!(
                    "{route}: expected actual boxed List payload {expected:?}, got {elements:?}"
                )
                .into(),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

macro_rules! signed_case {
    ($name:ident, $fixture:ident) => {
        #[test]
        fn $name() {
            check_payload(fixtures::$fixture, &[-32768, -1, 1]);
        }
    };
}
signed_case!(return_block_converts_before_scalar_cleanup, BLOCK_SIGNED);
signed_case!(explicit_return_keeps_current_conversion, EXPLICIT);
signed_case!(explicit_block_converts_before_its_cleanup, EXPLICIT_BLOCK);
signed_case!(expression_body_converts_before_its_cleanup, EXPRESSION_BODY);
signed_case!(nested_tail_block_converts_before_its_cleanup, NESTED_TAIL);
signed_case!(branch_tail_converts_before_its_cleanup, BRANCH_TAIL);
signed_case!(closure_uses_its_own_list_return_contract, LIST_CLOSURE);
signed_case!(
    array_closure_does_not_inherit_outer_list_return,
    ARRAY_CLOSURE
);
signed_case!(
    return_conversion_preserves_defer_and_owned_drop_order,
    OWNED_CLEANUP
);
signed_case!(
    return_conversion_does_not_destroy_borrowed_owner,
    BORROWED_OWNER
);

#[test]
fn unsigned_return_payload_keeps_high_bits_before_cleanup() {
    check_payload(fixtures::BLOCK_UNSIGNED, &[32768, 65535, 1]);
}

#[test]
fn callable_return_context_does_not_convert_array_closure_storage() {
    for (route, module) in source_and_wire(fixtures::ARRAY_CLOSURE) {
        let closure = module
            .functions
            .iter()
            .find(|function| {
                function.has_source_body
                    && module
                        .get_string(function.name)
                        .is_some_and(|name| name.contains("__closure"))
            })
            .expect("actual closure");
        let name = module.get_string(closure.name).unwrap();
        let instructions = body(&module, name);
        assert!(instructions.iter().any(|instruction| matches!(instruction,
            Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewTypedArray.to_byte()
        )), "{route}: array closure lost its packed producer: {instructions:?}");
        assert!(
            !instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::NewList { .. })),
            "{route}: array closure acquired an enclosing List conversion: {instructions:?}"
        );
    }
}

#[test]
fn owned_cleanup_and_unknown_calls_still_revoke_storage_authority() {
    for (route, module) in source_and_wire(fixtures::OWNED_CLEANUP) {
        let instructions = body(&module, "make");
        let mut facts = ArrayResultFacts::default();
        let (register, producer) = instructions
            .iter()
            .find_map(|instruction| {
                facts.observe(instruction);
                let Instruction::MemExtended { sub_op, operands } = instruction else {
                    return None;
                };
                if *sub_op != MemSubOpcode::NewTypedArray.to_byte() {
                    return None;
                }
                let mut cursor = 0;
                let register = verum_vbc::encoding::decode_reg(operands, &mut cursor)
                    .expect("canonical producer");
                let fact = facts.get(register)?;
                Some((register, fact))
            })
            .expect("actual packed producer");
        assert!(matches!(producer, ArrayResultFact::Packed { .. }));
        let cleanup = instructions
            .iter()
            .find(|instruction| matches!(instruction, Instruction::DropRef { .. }))
            .expect("actual scope cleanup");
        facts.observe(cleanup);
        assert_eq!(
            facts.get(register),
            None,
            "{route}: cleanup must revoke unknown authority"
        );
        let mut fresh = ArrayResultFacts::default();
        for instruction in &instructions {
            fresh.observe(instruction);
            if fresh.get(register).is_some() {
                break;
            }
        }
        let call = body(&module, "probe")
            .into_iter()
            .find(|instruction| {
                matches!(
                    instruction,
                    Instruction::Call { .. } | Instruction::CallM { .. }
                )
            })
            .expect("actual source call");
        fresh.observe(&call);
        assert_eq!(
            fresh.get(register),
            None,
            "{route}: unknown call must revoke authority"
        );
    }
}
