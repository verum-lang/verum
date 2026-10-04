//! T1571: field receiver layout comes from its allocation producer, not its numeric ID.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::bytecode::encode_instruction;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::instruction::{CbgrSubOpcode, Instruction, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, StringTable, VbcModule};
use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind, VariantDescriptor, VariantKind};

const HIGH: u32 = 36_161;

fn source_module(source: &str, high: bool) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::new();
    codegen.initialize();
    if high {
        let mut strings = StringTable::new();
        codegen.register_archive_type_qualified(
            TypeDescriptor {
                id: TypeId(HIGH),
                name: strings.intern("ExistingArchiveType"),
                kind: TypeKind::Record,
                ..Default::default()
            },
            "ExistingArchiveType".into(),
            Some("archive"),
            Some(&strings),
        );
    }
    codegen
        .collect_all_declarations(&ast)
        .expect("declarations");
    codegen.compile_module_items(&ast).expect("bodies");
    let module = codegen.finalize_module().expect("finalize");
    let outer = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some("Outer"))
        .expect("Outer");
    assert_eq!(outer.id.0 >= 0x8000, high, "actual allocated nominal ID");
    module
}

fn run(module: VbcModule) -> i64 {
    let entry = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .expect("probe")
        .id;
    Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute")
        .as_i64()
}

#[test]
fn source_nested_record_read_is_independent_of_archive_type_count() {
    let source = r#"
        type Cell is { padding: Int, value: Int };
        type Outer is { padding: Int, inner: Cell };
        fn probe() -> Int {
            let outer = Outer { padding: 91, inner: Cell { padding: 93, value: 7 } };
            outer.inner.value
        }
    "#;
    assert_eq!(run(source_module(source, false)), 7);
    assert_eq!(run(source_module(source, true)), 7);
}

#[test]
fn source_nested_record_reference_keeps_original_field_cell() {
    let source = r#"
        type Cell is { padding: Int, value: Int };
        type Outer is { padding: Int, inner: Cell };
        fn read_raw(p: &Int) -> Int { unsafe { *(p as &unsafe Int) } }
        fn probe() -> Int {
            let outer = Outer { padding: 91, inner: Cell { padding: 93, value: 7 } };
            read_raw(&outer.inner.value)
        }
    "#;
    assert_eq!(run(source_module(source, false)), 7);
    assert_eq!(run(source_module(source, true)), 7);
}

#[test]
fn source_high_record_write_changes_its_own_slot() {
    let source = r#"
        type Cell is { padding: Int, value: Int };
        type Outer is { padding: Int, inner: Cell };
        fn probe() -> Int {
            let mut outer = Outer { padding: 91, inner: Cell { padding: 93, value: 7 } };
            outer.padding = 37;
            outer.padding * 100 + outer.inner.padding
        }
    "#;
    assert_eq!(run(source_module(source, false)), 3793);
    assert_eq!(run(source_module(source, true)), 3793);
}

fn opcode_module(kind: Option<TypeKind>, type_id: u32, variant: bool, address: bool) -> VbcModule {
    let mut module = VbcModule::new("field_kind".into());
    if let Some(kind) = kind {
        let name = module.intern_string("Wrapper");
        let tag_name = module.intern_string("Wrapped");
        let mut descriptor = TypeDescriptor {
            id: TypeId(type_id),
            name,
            kind,
            ..Default::default()
        };
        if kind == TypeKind::Sum {
            descriptor.variants.push(VariantDescriptor {
                name: tag_name,
                tag: 0,
                kind: VariantKind::Tuple,
                arity: 1,
                ..Default::default()
            });
        }
        module.types.push(descriptor);
    }
    let mut instructions: List<Instruction> = List::new();
    instructions.push(Instruction::New {
        dst: Reg(1),
        type_id: 20,
        field_count: 2,
    });
    instructions.push(Instruction::LoadI {
        dst: Reg(2),
        value: 7,
    });
    instructions.push(Instruction::SetF {
        obj: Reg(1),
        field_idx: 1,
        value: Reg(2),
    });
    if variant {
        instructions.push(if kind.is_some() {
            Instruction::MakeVariantTyped {
                dst: Reg(0),
                type_id,
                tag: 0,
                field_count: 1,
            }
        } else {
            Instruction::MakeVariant {
                dst: Reg(0),
                tag: 0,
                field_count: 1,
            }
        });
        instructions.push(Instruction::SetVariantData {
            variant: Reg(0),
            field: 0,
            value: Reg(1),
        });
    } else {
        instructions.push(Instruction::New {
            dst: Reg(0),
            type_id,
            field_count: 2,
        });
        instructions.push(Instruction::SetF {
            obj: Reg(0),
            field_idx: 1,
            value: Reg(1),
        });
    }
    if address {
        instructions.push(Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::RefField as u8,
            operands: List::from(&[2, 0, 1][..]).into(),
        });
        instructions.push(Instruction::Deref {
            dst: Reg(3),
            ref_reg: Reg(2),
        });
    } else {
        instructions.push(Instruction::GetF {
            dst: Reg(3),
            obj: Reg(0),
            field_idx: 1,
        });
    }
    if !variant {
        // A record receiver must yield the nested record, never its scalar field.
        instructions.push(Instruction::GetF {
            dst: Reg(3),
            obj: Reg(3),
            field_idx: 1,
        });
    }
    instructions.push(Instruction::Ret { value: Reg(3) });
    let name = module.intern_string("probe");
    let mut function = FunctionDescriptor::new(name);
    function.id = FunctionId(0);
    function.register_count = 4;
    for instruction in instructions.iter() {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    module
}

#[test]
fn descriptor_high_record_survives_direct_ref_field() {
    assert_eq!(
        run(opcode_module(Some(TypeKind::Record), HIGH, false, true)),
        7
    );
}

#[test]
fn declared_variants_keep_record_payload_read_and_reference() {
    for id in [27, HIGH] {
        for address in [false, true] {
            assert_eq!(
                run(opcode_module(Some(TypeKind::Sum), id, true, address)),
                7
            );
        }
    }
}

#[test]
fn legacy_variant_without_descriptor_keeps_record_payload() {
    for address in [false, true] {
        assert_eq!(run(opcode_module(None, 0, true, address)), 7);
    }
}

#[test]
fn synthetic_record_without_descriptor_is_not_a_variant() {
    for address in [false, true] {
        assert_eq!(
            run(opcode_module(
                None,
                verum_common::layout::SYNTHETIC_RECORD_TYPE_ID,
                false,
                address
            )),
            7
        );
    }
}

fn machine(instructions: &[Instruction]) -> Interpreter {
    let mut module = VbcModule::new("kind_machine".into());
    let name = module.intern_string("probe");
    let mut function = FunctionDescriptor::new(name);
    function.id = FunctionId(0);
    function.register_count = 8;
    for instruction in instructions {
        encode_instruction(instruction, &mut module.bytecode);
    }
    function.bytecode_length = module.bytecode.len() as u32;
    module.functions.push(function);
    Interpreter::new(Arc::new(module))
}

#[test]
fn nominal_record_and_legacy_variant_with_same_id_coexist() {
    // Both objects have ID 0x8000 and a 16-byte data area. Neither ID,
    // descriptor kind nor apparent size can decide their representation.
    let id = verum_common::layout::synthetic_variant_type_id(0);
    let mut module = opcode_module(Some(TypeKind::Record), id, false, false);
    module.bytecode.clear();
    let instructions = [
        Instruction::New {
            dst: Reg(1),
            type_id: 20,
            field_count: 2,
        },
        Instruction::LoadI {
            dst: Reg(2),
            value: 7,
        },
        Instruction::SetF {
            obj: Reg(1),
            field_idx: 1,
            value: Reg(2),
        },
        Instruction::New {
            dst: Reg(0),
            type_id: id,
            field_count: 2,
        },
        Instruction::SetF {
            obj: Reg(0),
            field_idx: 1,
            value: Reg(1),
        },
        Instruction::MakeVariant {
            dst: Reg(4),
            tag: 0,
            field_count: 1,
        },
        Instruction::SetVariantData {
            variant: Reg(4),
            field: 0,
            value: Reg(1),
        },
        Instruction::GetF {
            dst: Reg(2),
            obj: Reg(0),
            field_idx: 1,
        },
        Instruction::GetF {
            dst: Reg(2),
            obj: Reg(2),
            field_idx: 1,
        },
        Instruction::GetF {
            dst: Reg(3),
            obj: Reg(4),
            field_idx: 1,
        },
        Instruction::BinaryI {
            op: verum_vbc::instruction::BinaryIntOp::Add,
            dst: Reg(3),
            a: Reg(2),
            b: Reg(3),
        },
        Instruction::Ret { value: Reg(3) },
    ];
    for instruction in &instructions {
        encode_instruction(instruction, &mut module.bytecode);
    }
    module.functions[0].register_count = 5;
    module.functions[0].bytecode_length = module.bytecode.len() as u32;
    assert_eq!(run(module), 14);
}

#[test]
fn descriptorless_high_record_is_not_peeled() {
    for address in [false, true] {
        assert_eq!(run(opcode_module(None, HIGH, false, address)), 7);
    }
}

#[test]
fn copies_keep_only_representation_and_work_through_register_references() {
    use verum_vbc::interpreter::{ObjectFlags, ObjectHeader};
    use verum_vbc::value::Value;
    for variant in [false, true] {
        let mut interpreter = machine(&[
            Instruction::Clone {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Ret { value: Reg(1) },
        ]);
        let mut original = if variant {
            interpreter
                .state
                .heap
                .alloc_variant(TypeId(HIGH), 0, 1)
                .unwrap()
        } else {
            interpreter.state.heap.alloc(TypeId(HIGH), 16).unwrap()
        };
        original.header_mut().flags |= ObjectFlags::BORROWED | ObjectFlags::MARKED;
        let value = Value::from_ptr(original.as_ptr().cast::<u8>());
        let cloned = interpreter
            .execute_function_with_args(FunctionId(0), &[value])
            .unwrap();
        assert_ne!(cloned.bits(), value.bits());
        let header = unsafe { &*cloned.as_ptr::<ObjectHeader>() };
        assert_eq!(
            header.flags,
            if variant {
                ObjectFlags::VARIANT
            } else {
                ObjectFlags::empty()
            }
        );
        assert!(
            original
                .header()
                .flags
                .contains(ObjectFlags::BORROWED | ObjectFlags::MARKED)
        );
    }
    // Register references retain the same allocation-kind authority.
    for variant in [false, true] {
        for address in [false, true] {
            let mut instructions: List<Instruction> = List::new();
            instructions.push(Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            });
            if address {
                instructions.push(Instruction::CbgrExtended {
                    sub_op: CbgrSubOpcode::RefField as u8,
                    operands: List::from(&[2, 1, 1][..]).into(),
                });
                instructions.push(Instruction::Deref {
                    dst: Reg(2),
                    ref_reg: Reg(2),
                });
            } else {
                instructions.push(Instruction::GetF {
                    dst: Reg(2),
                    obj: Reg(1),
                    field_idx: 1,
                });
            }
            if !variant {
                instructions.push(Instruction::GetF {
                    dst: Reg(2),
                    obj: Reg(2),
                    field_idx: 1,
                });
            }
            instructions.push(Instruction::Ret { value: Reg(2) });
            let mut interpreter = machine(instructions.as_slice());
            let inner = interpreter.state.heap.alloc(TypeId(20), 16).unwrap();
            unsafe {
                *inner.data_ptr().cast::<Value>().add(1) = Value::from_i64(7);
            }
            let outer = if variant {
                interpreter
                    .state
                    .heap
                    .alloc_variant(TypeId(HIGH), 0, 1)
                    .unwrap()
            } else {
                interpreter.state.heap.alloc(TypeId(HIGH), 16).unwrap()
            };
            unsafe {
                *outer.data_ptr().cast::<Value>().add(1) =
                    Value::from_ptr(inner.as_ptr().cast::<u8>());
            }
            let result = interpreter
                .execute_function_with_args(
                    FunctionId(0),
                    &[Value::from_ptr(outer.as_ptr().cast::<u8>())],
                )
                .unwrap();
            assert_eq!(result.as_i64(), 7);
        }
    }
}

#[test]
fn malformed_variant_prefix_and_payload_are_rejected_by_all_field_doors() {
    use verum_vbc::value::Value;
    for short_prefix in [false, true] {
        for operation in [
            Instruction::GetF {
                dst: Reg(1),
                obj: Reg(0),
                field_idx: 1,
            },
            Instruction::SetF {
                obj: Reg(0),
                field_idx: 1,
                value: Reg(1),
            },
            Instruction::CbgrExtended {
                sub_op: CbgrSubOpcode::RefField as u8,
                operands: List::from(&[1, 0, 1][..]).into(),
            },
        ] {
            let mut interpreter = machine(&[operation, Instruction::Ret { value: Reg(1) }]);
            let mut object = interpreter
                .state
                .heap
                .alloc_variant(TypeId(HIGH), 0, 0)
                .unwrap();
            if short_prefix {
                object.header_mut().size = 4;
            } else {
                unsafe {
                    *object.data_ptr().cast::<u32>().add(1) = 2;
                }
            }
            let error = interpreter
                .execute_function_with_args(
                    FunctionId(0),
                    &[Value::from_ptr(object.as_ptr().cast::<u8>())],
                )
                .unwrap_err();
            assert!(
                format!("{error}").contains(if short_prefix {
                    "lacks tag/count"
                } else {
                    "payload exceeds"
                }),
                "{error}"
            );
            // Restore the allocation's real extent before Heap deallocation.
            object.header_mut().size = 8;
        }
    }
}

#[test]
fn unit_and_multiple_payload_variants_do_not_read_past_their_allocation() {
    use verum_vbc::value::Value;
    let mut interpreter = machine(&[
        Instruction::GetF {
            dst: Reg(1),
            obj: Reg(0),
            field_idx: 1,
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    let unit = interpreter
        .state
        .heap
        .alloc_variant(TypeId(HIGH), 0, 0)
        .unwrap();
    assert!(
        interpreter
            .execute_function_with_args(
                FunctionId(0),
                &[Value::from_ptr(unit.as_ptr().cast::<u8>())]
            )
            .is_err()
    );
    let inner = interpreter.state.heap.alloc(TypeId(20), 16).unwrap();
    unsafe {
        *inner.data_ptr().cast::<Value>().add(1) = Value::from_i64(7);
    }
    let multiple = interpreter
        .state
        .heap
        .alloc_variant(TypeId(HIGH), 0, 2)
        .unwrap();
    unsafe {
        *multiple.data_ptr().cast::<Value>().add(1) = Value::from_ptr(inner.as_ptr().cast::<u8>());
        *multiple.data_ptr().cast::<Value>().add(2) = Value::from_i64(99);
    }
    let result = interpreter
        .execute_function_with_args(
            FunctionId(0),
            &[Value::from_ptr(multiple.as_ptr().cast::<u8>())],
        )
        .unwrap();
    assert_eq!(result.as_i64(), 7);
}

#[test]
fn public_variant_producer_survives_clone_and_field_read() {
    use verum_vbc::interpreter::ObjectFlags;
    use verum_vbc::value::Value;
    let mut interpreter = machine(&[
        Instruction::Clone {
            dst: Reg(1),
            src: Reg(0),
        },
        Instruction::Ref {
            dst: Reg(2),
            src: Reg(1),
        },
        Instruction::GetF {
            dst: Reg(3),
            obj: Reg(2),
            field_idx: 1,
        },
        Instruction::Ret { value: Reg(3) },
    ]);
    let inner = interpreter.state.heap.alloc(TypeId(HIGH), 16).unwrap();
    unsafe {
        *inner.data_ptr().cast::<Value>().add(1) = Value::from_i64(37);
    }
    let wrapper = interpreter
        .alloc_variant(0, &[Value::from_ptr(inner.as_ptr().cast::<u8>())])
        .unwrap();
    let header = unsafe { &*wrapper.as_ptr::<verum_vbc::interpreter::ObjectHeader>() };
    assert!(header.flags.contains(ObjectFlags::VARIANT));
    let result = interpreter
        .execute_function_with_args(FunctionId(0), &[wrapper])
        .unwrap();
    assert_eq!(result.as_i64(), 37);
}
