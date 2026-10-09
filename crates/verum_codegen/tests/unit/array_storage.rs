//! T1704: native selected-body and instruction-boundary proof controls.
use super::*;
use verum_fast_parser::Parser;
use verum_llvm::context::Context;
use verum_vbc::codegen::VbcCodegen;

#[test]
fn final_source_body_and_unique_selected_target_own_the_summary() {
    let source = r#"
fn packed() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes }
fn listed() -> [Byte; 2] { [7 as Byte, 9 as Byte] }
"#;
    let mut module = VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("grammar"))
        .expect("source bodies");
    let context = Context::create();
    let native = context.create_module("selected_array_returns");
    let signature = context.i64_type().fn_type(&[], false);
    let mut targets = Map::new();
    for descriptor in &module.functions {
        let name = module.get_string(descriptor.name).expect("source name");
        targets.insert(descriptor.id.0, native.add_function(name, signature, None));
    }
    let packed = module
        .functions
        .iter()
        .position(|f| module.get_string(f.name) == Some("packed"))
        .unwrap();
    let listed = module
        .functions
        .iter()
        .position(|f| module.get_string(f.name) == Some("listed"))
        .unwrap();
    let packed_target = targets[&module.functions[packed].id.0];
    let listed_target = targets[&module.functions[listed].id.0];
    let summaries = selected_source_returns(&module, |id| targets.get(&id).copied());
    assert_eq!(
        summaries.get(&packed_target),
        Some(&ArrayResultFact::Packed {
            width: 1,
            float: false,
            count: 2
        })
    );
    assert_eq!(summaries.get(&listed_target), Some(&ArrayResultFact::List));
    module.functions[packed].has_source_body = false;
    assert!(
        !selected_source_returns(&module, |id| targets.get(&id).copied())
            .contains_key(&packed_target)
    );
    module.functions[packed].has_source_body = true;
    let saved_body = module.functions[packed].instructions.clone();
    module.functions[packed].instructions = None;
    assert!(
        !selected_source_returns(&module, |id| targets.get(&id).copied())
            .contains_key(&packed_target)
    );
    module.functions[packed].instructions = saved_body;
    let block = context.append_basic_block(packed_target, "preexisting_native_body");
    let builder = context.create_builder();
    builder.position_at_end(block);
    builder
        .build_return(Some(&context.i64_type().const_zero()))
        .unwrap();
    assert!(
        !selected_source_returns(&module, |id| targets.get(&id).copied())
            .contains_key(&packed_target)
    );
    // Matching signatures or names cannot select one of two descriptors that
    // the actual backend map resolves to the same native function.
    assert!(selected_source_returns(&module, |_| Some(listed_target)).is_empty());
}

#[test]
fn native_instruction_transfer_snapshots_same_register_moves_and_overwrites() {
    let body = [Instruction::GetE {
        dst: Reg(301),
        arr: Reg(300),
        idx: Reg(128),
    }];
    let mut storage = ArrayStorage::for_body(&body);
    let call = Instruction::Call {
        dst: Reg(270),
        func_id: 42,
        args: verum_vbc::instruction::RegRange {
            start: Reg(0),
            count: 0,
        },
    };
    storage.begin_instruction(&call);
    storage.forget_value(Reg(270));
    let packed = ArrayResultFact::Packed {
        width: 1,
        float: false,
        count: 2,
    };
    storage.call_result(Reg(270), Some(packed)).unwrap();
    storage.finish_instruction(&call).unwrap();
    for destination in [Reg(270), Reg(300)] {
        let mov = Instruction::Mov {
            dst: destination,
            src: Reg(270),
        };
        storage.begin_instruction(&mov);
        storage.forget_value(destination); // actual native set_register
        storage.finish_instruction(&mov).unwrap();
        assert_eq!(storage.get(destination), Some(packed));
        assert!(storage.is_array(destination));
    }
    let overwrite = Instruction::NewList {
        dst: Reg(300),
        capacity_hint: 0,
    };
    storage.begin_instruction(&overwrite);
    storage.forget_value(Reg(300));
    storage.finish_instruction(&overwrite).unwrap();
    assert_eq!(storage.get(Reg(300)), Some(ArrayResultFact::List));
    assert!(!storage.is_array(Reg(300)));
}

#[test]
fn unknown_native_control_flow_cannot_authorize_array_call_reads() {
    let body = [
        Instruction::Jmp { offset: 1 },
        Instruction::GetE {
            dst: Reg(2),
            arr: Reg(0),
            idx: Reg(1),
        },
    ];
    for fact in [
        None,
        Some(ArrayResultFact::Packed {
            width: 1,
            float: false,
            count: 2,
        }),
    ] {
        let mut storage = ArrayStorage::for_body(&body);
        assert!(matches!(
            storage.call_result(Reg(0), fact),
            Err(LlvmLoweringError::UnprovenArrayStorage(_))
        ));
    }
    let mut storage = ArrayStorage::for_body(&body);
    storage
        .call_result(Reg(0), Some(ArrayResultFact::List))
        .unwrap();
}

#[test]
fn indexed_store_preserves_the_straight_line_selected_result() {
    let call = Instruction::Call {
        dst: Reg(270),
        func_id: 42,
        args: verum_vbc::instruction::RegRange {
            start: Reg(0),
            count: 0,
        },
    };
    let store = Instruction::SetE {
        arr: Reg(270),
        idx: Reg(128),
        value: Reg(257),
    };
    let read = Instruction::GetE {
        dst: Reg(300),
        arr: Reg(270),
        idx: Reg(128),
    };
    let mut storage = ArrayStorage::for_body(&[call.clone(), store.clone(), read]);
    let fact = ArrayResultFact::Packed {
        width: 1,
        float: false,
        count: 2,
    };
    storage.begin_instruction(&call);
    storage
        .call_result(Reg(270), Some(fact))
        .expect("straight-line mutation");
    storage.finish_instruction(&call).unwrap();
    storage.begin_instruction(&store);
    storage.finish_instruction(&store).unwrap();
    assert_eq!(storage.get(Reg(270)), Some(fact));
    assert!(storage.is_array(Reg(270)));
}

#[test]
fn caller_cleanup_allows_prior_access_but_discards_every_storage_fact() {
    let call = Instruction::Call {
        dst: Reg(270),
        func_id: 42,
        args: verum_vbc::instruction::RegRange {
            start: Reg(0),
            count: 0,
        },
    };
    let read = Instruction::GetE {
        dst: Reg(300),
        arr: Reg(270),
        idx: Reg(128),
    };
    let cleanup = Instruction::DropRef { src: Reg(1) };
    let mut storage = ArrayStorage::for_body(&[
        call.clone(),
        read.clone(),
        cleanup.clone(),
        Instruction::Ret { value: Reg(300) },
    ]);
    let fact = ArrayResultFact::Packed {
        width: 1,
        float: false,
        count: 2,
    };
    storage.begin_instruction(&call);
    storage
        .call_result(Reg(270), Some(fact))
        .expect("cleanup is not a local branch");
    storage.finish_instruction(&call).unwrap();
    storage.begin_instruction(&read);
    storage.finish_instruction(&read).unwrap();
    assert_eq!(storage.get(Reg(270)), Some(fact));
    storage.begin_instruction(&cleanup);
    storage.finish_instruction(&cleanup).unwrap();
    assert_eq!(storage.get(Reg(270)), None, "user drop glue discards proof");
    assert!(
        storage.is_array(Reg(270)),
        "a later unknown read must still refuse"
    );
}
