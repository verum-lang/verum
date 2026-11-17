use super::*;
use crate::{
    TypeId, VbcModule,
    interpreter::{Interpreter, heap::Object},
};
use verum_common::List;

fn interpreter() -> Interpreter {
    Interpreter::new(VbcModule::new("owned_exception_payloads".into()).into())
}

fn record(state: &mut InterpreterState, fields: &[Value]) -> Object {
    let object = state
        .heap
        .alloc(TypeId::TUPLE, std::mem::size_of_val(fields))
        .unwrap();
    for (index, value) in fields.iter().enumerate() {
        // SAFETY: the exact producer owns the complete Value-slot extent.
        unsafe {
            object.data_ptr().cast::<Value>().add(index).write(*value);
        }
    }
    object
}

#[test]
fn discarding_a_declared_packet_reuses_recursive_owned_cleanup() {
    let mut vm = interpreter();
    let state = &mut vm.state;
    let text =
        super::super::string_helpers::alloc_string_value(state, "owned exception text").unwrap();
    let object = record(state, &[text]);
    let ticket = own_payload(
        state,
        OwnedPayload {
            value: Value::from_ptr(object.as_ptr()),
            declaration: TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::TEXT)]),
        },
    )
    .unwrap();
    assert_eq!(state.heap.stats().total_frees, 0);
    discard_payload(state, ticket).unwrap();
    assert_eq!(
        state.heap.stats().total_frees,
        state.heap.stats().total_allocs
    );
    assert!(discard_payload(state, ticket).is_err());
}

#[test]
fn copying_a_packet_keeps_both_payload_owners_independent() {
    let mut vm = interpreter();
    let state = &mut vm.state;
    let text =
        super::super::string_helpers::alloc_string_value(state, "independent exception text")
            .unwrap();
    let object = record(state, &[text]);
    let original = own_payload(
        state,
        OwnedPayload {
            value: Value::from_ptr(object.as_ptr()),
            declaration: TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::TEXT)]),
        },
    )
    .unwrap();
    let (source, declaration) = state.heap.borrow_owned_packet(original).unwrap();
    let declaration = declaration.clone();
    let value = super::super::copy_typed::copy_declared(state, source, &declaration).unwrap();
    let copied = own_payload(state, OwnedPayload { value, declaration }).unwrap();
    let (a, declaration) = state.heap.borrow_owned_packet(original).unwrap();
    let declaration = declaration.clone();
    let (b, copied_declaration) = state.heap.borrow_owned_packet(copied).unwrap();
    assert_eq!(declaration, *copied_declaration);
    assert_ne!(a, b);
    discard_payload(state, original).unwrap();
    assert!(state.heap.contains(b.as_ptr()));
    discard_payload(state, copied).unwrap();
    assert_eq!(
        state.heap.stats().total_frees,
        state.heap.stats().total_allocs
    );
}

#[test]
fn a_payload_transfer_does_not_run_destruction_and_can_be_rethrown() {
    let mut vm = interpreter();
    let state = &mut vm.state;
    let object = record(state, &[Value::from_i64(73)]);
    let original = own_payload(
        state,
        OwnedPayload {
            value: Value::from_ptr(object.as_ptr()),
            declaration: TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::I64)]),
        },
    )
    .unwrap();
    let transferred = state.heap.take_owned_packet(original).unwrap();
    assert!(state.heap.contains(object.as_ptr()));
    assert_eq!(state.heap.stats().total_frees, 1);
    let propagated = own_payload(state, transferred).unwrap();
    assert_ne!(original, propagated);
    assert!(state.heap.take_owned_packet(original).is_err());
    discard_payload(state, propagated).unwrap();
    assert_eq!(
        state.heap.stats().total_frees,
        state.heap.stats().total_allocs
    );
}

#[test]
fn a_payload_with_a_mismatched_declaration_refuses_before_carrier_allocation() {
    let mut vm = interpreter();
    let state = &mut vm.state;
    let before = state.heap.stats().total_allocs;
    let error = own_payload(
        state,
        OwnedPayload {
            value: Value::from_i64(73),
            declaration: TypeRef::Tuple(vec![TypeRef::Concrete(TypeId::I64)]),
        },
    );
    assert!(error.is_err());
    assert_eq!(before, state.heap.stats().total_allocs);
}

fn executable_modules(body: &[crate::Instruction]) -> [VbcModule; 2] {
    use crate::{
        Instruction,
        bytecode::encode_instruction,
        module::{FunctionDescriptor, FunctionId},
    };

    let mut module = VbcModule::new("unread_panic".into());
    for (name, instructions) in [("caller", &[Instruction::RetV][..]), ("probe", body)] {
        let mut function = FunctionDescriptor::new(module.intern_string(name));
        function.id = FunctionId(module.functions.len() as u32);
        function.register_count = 4;
        function.bytecode_offset = module.bytecode.len() as u32;
        for instruction in instructions {
            encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        module.functions.push(function);
    }
    module.header.function_table_count = module.functions.len() as u32;
    let bytes = crate::serialize::serialize_module(&module).unwrap();
    let decoded = crate::deserialize::deserialize_module_validated(&bytes).unwrap();
    [module, decoded]
}

fn catch_without_reading(body: &mut List<crate::Instruction>) {
    use crate::Instruction;
    // The handler starts immediately after the one-byte Unreachable instruction.
    body.extend([
        Instruction::TryBegin { handler_offset: 1 },
        Instruction::Unreachable,
    ]);
}

#[test]
fn vm_retires_both_unread_panics_on_replacement_and_normal_return() {
    use crate::{Instruction, instruction::Reg, module::FunctionId};

    let mut body = List::new();
    catch_without_reading(&mut body);
    catch_without_reading(&mut body);
    body.extend([
        Instruction::LoadI {
            dst: Reg(0),
            value: 73,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    for module in executable_modules(&body) {
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(FunctionId(1)).unwrap().try_as_i64(),
            Some(73)
        );
        let stats = vm.state.heap.stats();
        assert_eq!((stats.total_allocs, stats.total_frees), (4, 4));
        assert!(vm.state.current_exception.is_none());
    }
}

#[test]
fn vm_discards_the_unread_inner_panic_on_failed_and_successful_sync_returns() {
    use crate::interpreter::dispatch_table::call_function_sync;
    use crate::{Instruction, instruction::Reg, module::FunctionId};

    for fail in [false, true] {
        let mut body = List::new();
        catch_without_reading(&mut body);
        body.push(if fail {
            Instruction::Unreachable
        } else {
            Instruction::RetV
        });
        for module in executable_modules(&body) {
            let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
            vm.state
                .call_stack
                .push_frame(FunctionId(0), 4, 0, Reg(0))
                .unwrap();
            vm.state.registers.push_frame(4);
            let result = call_function_sync(&mut vm.state, FunctionId(1), &[]);
            if fail {
                assert!(
                    matches!(result, Err(InterpreterError::Unreachable { .. })),
                    "{result:?}"
                );
            } else {
                assert!(result.unwrap().is_unit());
            }
            assert_eq!(vm.state.call_stack.depth(), 1);
            assert!(vm.state.current_exception.is_none());
            let stats = vm.state.heap.stats();
            assert_eq!((stats.total_allocs, stats.total_frees), (2, 2));
        }
    }
}

#[test]
fn vm_replaces_an_unread_panic_without_claiming_the_raw_throw_operand() {
    use crate::{Instruction, instruction::Reg, module::FunctionId};

    let mut body = List::new();
    catch_without_reading(&mut body);
    let throw = Instruction::Throw { error: Reg(0) };
    let throw_size = crate::bytecode::instruction_size(&throw);
    body.extend([
        Instruction::TryBegin {
            handler_offset: throw_size as i32,
        },
        throw,
        Instruction::GetException { dst: Reg(1) },
        Instruction::Ret { value: Reg(1) },
    ]);
    for module in executable_modules(&body) {
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        let object = record(&mut vm.state, &[Value::from_i64(73)]);
        let foreign = Value::from_ptr(object.as_ptr());
        assert_eq!(
            vm.execute_function_with_args(FunctionId(1), &[foreign])
                .unwrap(),
            foreign
        );
        assert!(vm.state.heap.contains(object.as_ptr()));
        let stats = vm.state.heap.stats();
        assert_eq!((stats.total_allocs, stats.total_frees), (3, 2));
        // SAFETY: the VM returned the original live caller-owned allocation.
        unsafe { vm.state.heap.free(object) };
    }
}

#[test]
fn vm_get_exception_preserves_the_v64_record_across_a_normal_child_call() {
    use crate::{
        Instruction,
        instruction::{Reg, RegRange},
        module::FunctionId,
    };

    let mut body = List::new();
    catch_without_reading(&mut body);
    body.extend([
        Instruction::Call {
            dst: Reg(1),
            func_id: 0,
            args: RegRange::new(Reg(0), 0),
        },
        Instruction::GetException { dst: Reg(0) },
        Instruction::Ret { value: Reg(0) },
    ]);
    for module in executable_modules(&body) {
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        let value = vm.execute_function(FunctionId(1)).unwrap();
        assert!(vm.state.current_exception.is_none());
        assert!(vm.state.heap.contains(value.as_ptr()));
        let stats = vm.state.heap.stats();
        assert_eq!((stats.total_allocs, stats.total_frees), (2, 0));
        // SAFETY: GetException transferred the runtime producer's live one-Value record.
        let object = unsafe { Object::from_raw(value.as_ptr()) }.unwrap();
        let message = unsafe { object.data_ptr().cast::<Value>().read() };
        let text = super::super::text_storage::TextStorage::checked(&vm.state, message).unwrap();
        // SAFETY: checked validated the complete byte extent on this live heap.
        assert_eq!(
            unsafe { std::slice::from_raw_parts(text.data, text.len) },
            b"entered unreachable code"
        );
        text.destroy(&mut vm.state).unwrap();
        unsafe {
            vm.state.heap.free(object);
        }
        assert_eq!(vm.state.heap.stats().total_frees, 2);
    }
}

#[test]
fn vm_refuses_panic_packet_formation_without_leaking_the_message() {
    use crate::{Instruction, module::FunctionId};

    let mut body = List::new();
    catch_without_reading(&mut body);
    body.push(Instruction::RetV);
    for module in executable_modules(&body) {
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        vm.state.config.typed_copy_limits.max_work = 0;
        let error = vm.execute_function(FunctionId(1)).unwrap_err();
        assert!(
            matches!(error, InterpreterError::InvalidOperand { ref message }
            if message.contains("work budget")),
            "{error:?}"
        );
        assert!(vm.state.current_exception.is_none());
        let stats = vm.state.heap.stats();
        assert_eq!((stats.total_allocs, stats.total_frees), (1, 1));
    }
}
