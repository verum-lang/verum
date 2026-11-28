//! An ordering is a checked run-time operand, including after archive reload.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, Reg, Value, VbcModule, bytecode::encode_instructions, codegen::VbcCodegen,
    deserialize::deserialize_module, instruction::SyncSubOpcode, interpreter::Interpreter,
    module::FunctionDescriptor, serialize::serialize_module, validate::validate_module,
};

fn wire(module: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(module).unwrap()).unwrap()
}

#[test]
fn dynamic_fences_accept_only_the_declared_scalar_orders() {
    for intrinsic in ["memory_fence", "atomic_fence", "compiler_fence"] {
        let source =
            format!("fn probe(order:Int)->Int {{ @intrinsic(\"{intrinsic}\", order); 73 }}");
        let ast = Parser::new(&source).parse_module().unwrap();
        let original = VbcCodegen::new().compile_module(&ast).unwrap();
        for module in [wire(&original), original] {
            let entry = module.find_function_by_name("probe").unwrap();
            for code in [-1, 0, 1, 2, 3, 4, 5, 260, i64::MAX] {
                let mut vm = Interpreter::try_new_validated(module.clone().into()).unwrap();
                let result = vm.execute_function_with_args(entry, &[Value::from_i64(code)]);
                if (0..=4).contains(&code) {
                    assert_eq!(result.unwrap().try_as_i64(), Some(73), "{intrinsic} {code}");
                } else {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("invalid memory fence ordering"),
                        "{intrinsic} {code}"
                    );
                }
            }
        }
    }
}

fn instructions(fence: Instruction) -> VbcModule {
    let mut module = VbcModule::new("fences".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.register_count = 2;
    let body = [
        fence,
        Instruction::LoadI {
            dst: Reg(1),
            value: 73,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    function.bytecode_length = encode_instructions(&body, &mut module.bytecode) as u32;
    module.add_function(function);
    module
}

#[test]
fn immediate_fences_consume_their_operand_before_the_following_instruction() {
    for ordering in [0, 1, 2, 3, 4, 5, 7, 255] {
        let module = wire(&instructions(Instruction::AtomicFence { ordering }));
        let entry = module.find_function_by_name("probe").unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    }
}

#[test]
fn wire_validator_checks_fence_version_register_and_operand_boundary() {
    for ordering in [6, 8, 254] {
        assert!(validate_module(&instructions(Instruction::AtomicFence { ordering })).is_err());
    }
    for scope in [SyncSubOpcode::ThreadFence, SyncSubOpcode::CompilerFence] {
        let make = |operands| {
            instructions(Instruction::SyncExtended {
                sub_op: scope as u8,
                operands,
            })
        };
        let mut valid = make(vec![1]);
        validate_module(&wire(&valid)).unwrap();
        valid.header.version_minor = verum_vbc::format::ORDERED_FENCE_VERSION - 1;
        assert!(validate_module(&valid).is_err());
        for invalid in [vec![], vec![0, 1], vec![9], vec![0x80]] {
            assert!(validate_module(&make(invalid)).is_err());
        }
    }
}
