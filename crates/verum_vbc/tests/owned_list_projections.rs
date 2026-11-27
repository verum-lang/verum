//! VM sequence references retain their owner's lifetime, encoding and rights.
use verum_common::List;
use verum_vbc::{
    bytecode::encode_instruction,
    deserialize::deserialize_module,
    instruction::{CbgrSubOpcode as C, Instruction as I, MemSubOpcode, Reg, RegRange},
    interpreter::{Interpreter, InterpreterError, InterpreterResult, Object},
    module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcModule},
    serialize::serialize_module,
    types::TypeId,
    value::{Capabilities, Value},
};

fn cbgr(op: C, args: &[u8]) -> I {
    I::CbgrExtended {
        sub_op: op as u8,
        operands: args.into(),
    }
}
fn fixture() -> Interpreter {
    let mut module = VbcModule::new("projection_contract".into());
    let receiver_identity = module.strings.intern("owner.receiver_identity");
    let bodies: List<(&str, List<I>)> = [
        (
            "mutable",
            [
                I::RefMut {
                    dst: Reg(4),
                    src: Reg(0),
                },
                cbgr(C::RefSlice, &[5, 4, 1, 2]),
                I::Ret { value: Reg(5) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "readonly",
            [
                I::Ref {
                    dst: Reg(4),
                    src: Reg(0),
                },
                cbgr(C::RefSlice, &[5, 4, 1, 2]),
                I::Ret { value: Reg(5) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "subview",
            [
                cbgr(C::SliceSubslice, &[4, 0, 1, 2]),
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "shared_element",
            [cbgr(C::RefListElement, &[4, 0, 1, 1]), I::Ret { value: Reg(4) }]
                .into_iter().collect(),
        ),
        (
            "shared_range",
            [cbgr(C::RefSlice, &[4, 0, 1, 2, 0, 1]), I::Ret { value: Reg(4) }]
                .into_iter().collect(),
        ),
        (
            "element",
            [
                cbgr(C::RefListElement, &[4, 0, 1]),
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "read",
            [
                I::Deref {
                    dst: Reg(4),
                    ref_reg: Reg(0),
                },
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "write",
            [
                I::DerefMut {
                    ref_reg: Reg(0),
                    value: Reg(1),
                },
                I::RetV,
            ]
            .into_iter()
            .collect(),
        ),
        (
            "get",
            [cbgr(C::SliceGet, &[4, 0, 1]), I::Ret { value: Reg(4) }]
                .into_iter()
                .collect(),
        ),
        (
            "len",
            [cbgr(C::SliceLen, &[4, 0]), I::Ret { value: Reg(4) }]
                .into_iter()
                .collect(),
        ),
        (
            "attenuate",
            [
                cbgr(
                    C::CapAttenuate,
                    &[
                        4,
                        0,
                        Capabilities::READ_ONLY.0 as u8,
                        (Capabilities::READ_ONLY.0 >> 8) as u8,
                    ],
                ),
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "resize",
            [
                I::MemExtended {
                    sub_op: MemSubOpcode::ListStorageResize as u8,
                    operands: [4, 0, 1].into_iter().collect(),
                },
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "forward",
            [
                I::Mov {
                    dst: Reg(4),
                    src: Reg(0),
                },
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "cursor",
            [I::IterNew { dst: Reg(4), iterable: Reg(0) }, I::Ret { value: Reg(4) }]
                .into_iter().collect(),
        ),
        (
            "advance",
            [I::IterNext { dst: Reg(4), has_next: Reg(5), iter: Reg(0) }, I::Ret { value: Reg(4) }]
                .into_iter().collect(),
        ),
        (
            "allocate",
            [cbgr(C::AllocateUser, &[4, 0, 1]), I::Ret { value: Reg(4) }]
                .into_iter()
                .collect(),
        ),
        (
            "free",
            [cbgr(C::DeallocUser, &[4, 0]), I::RetV]
                .into_iter()
                .collect(),
        ),
        (
            "invalidate",
            [cbgr(C::Invalidate, &[0]), I::RetV].into_iter().collect(),
        ),
        (
            "without_exclusive",
            [
                cbgr(
                    C::CapAttenuate,
                    &[4, 0, (Capabilities::READ | Capabilities::WRITE) as u8, 0],
                ),
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "owner.receiver_identity",
            [
                I::Deref {
                    dst: Reg(4),
                    ref_reg: Reg(0),
                },
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "method",
            [
                I::CallM {
                    dst: Reg(4),
                    receiver: Reg(0),
                    method_id: receiver_identity.0,
                    args: RegRange::new(Reg(1), 0),
                },
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "writeonly",
            [
                cbgr(C::CapAttenuate, &[4, 0, Capabilities::WRITE as u8, 0]),
                I::Ret { value: Reg(4) },
            ]
            .into_iter()
            .collect(),
        ),
        (
            "raw",
            [cbgr(C::Unslice, &[4, 0]), I::Ret { value: Reg(4) }]
                .into_iter()
                .collect(),
        ),
    ]
    .into_iter()
    .collect();
    for (name, body) in bodies {
        let mut descriptor = FunctionDescriptor::new(module.strings.intern(name));
        descriptor.id = FunctionId(module.functions.len() as u32);
        if name == "owner.receiver_identity" {
            descriptor.params.push(ParamDescriptor {
                name: module.strings.intern("self"),
                ..Default::default()
            });
        }
        descriptor.register_count = 8;
        descriptor.bytecode_offset = module.bytecode.len() as u32;
        for instruction in body {
            encode_instruction(&instruction, &mut module.bytecode);
        }
        descriptor.bytecode_length = module.bytecode.len() as u32 - descriptor.bytecode_offset;
        module.functions.push(descriptor);
    }
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    Interpreter::try_new_validated(wire.into()).expect("validated VM instruction boundary")
}
fn call(vm: &mut Interpreter, name: &str, args: &[Value]) -> InterpreterResult<Value> {
    let id = vm.state.module.find_function_by_name(name).unwrap();
    vm.execute_function_with_args(id, args)
}
fn list(vm: &mut Interpreter, bytes: bool) -> (Object, Object) {
    let backing = vm
        .state
        .heap
        .alloc(
            if bytes { TypeId::U8 } else { TypeId::INT },
            if bytes { 3 } else { 24 },
        )
        .unwrap();
    let owner = vm
        .state
        .heap
        .alloc_array(
            if bytes {
                TypeId::BYTE_LIST
            } else {
                TypeId::LIST
            },
            3,
        )
        .unwrap();
    // SAFETY: the allocations have exactly three elements and three owner fields.
    unsafe {
        for (index, value) in [11, 42, 99].into_iter().enumerate() {
            if bytes {
                backing.data_ptr().add(index).write(value as u8);
            } else {
                backing
                    .data_ptr()
                    .cast::<Value>()
                    .add(index)
                    .write(Value::from_i64(value));
            }
        }
        owner.data_ptr().cast::<Value>().write(Value::from_i64(3));
        owner
            .data_ptr()
            .cast::<Value>()
            .add(1)
            .write(Value::from_i64(3));
        owner
            .data_ptr()
            .cast::<Value>()
            .add(2)
            .write(Value::from_ptr(backing.as_ptr()));
    }
    (owner, backing)
}
fn full(vm: &mut Interpreter, owner: &Object, mutable: bool) -> Value {
    call(
        vm,
        if mutable { "mutable" } else { "readonly" },
        &[
            Value::from_ptr(owner.as_ptr()),
            Value::from_i64(0),
            Value::from_i64(3),
        ],
    )
    .unwrap()
}
fn element(vm: &mut Interpreter, span: Value, index: i64) -> Value {
    call(vm, "element", &[span, Value::from_i64(index)]).unwrap()
}
fn capability(error: InterpreterError) {
    assert!(
        matches!(error, InterpreterError::CapabilityDenied { .. }),
        "{error:?}"
    );
}
fn stale(error: InterpreterError) {
    assert!(
        matches!(error, InterpreterError::CbgrViolation { .. }),
        "{error:?}"
    );
}
#[test]
fn shared_elements_coexist_with_ranges_and_retire_on_exclusive_access() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let collection = Value::from_ptr(owner.as_ptr());
    let zero = Value::from_i64(0);
    let one = Value::from_i64(1);
    let left = call(&mut vm, "shared_element", &[collection, zero]).unwrap();
    let right = call(&mut vm, "shared_element", &[collection, one]).unwrap();
    let range = call(&mut vm, "shared_range", &[collection, one, one]).unwrap();
    assert_eq!(call(&mut vm, "read", &[left]).unwrap().try_as_i64(), Some(11));
    assert_eq!(call(&mut vm, "get", &[range, zero]).unwrap().try_as_i64(), Some(42));
    capability(call(&mut vm, "write", &[right, zero]).unwrap_err());
    assert_eq!(call(&mut vm, "read", &[right]).unwrap().try_as_i64(), Some(42));
    full(&mut vm, &owner, true);
    stale(call(&mut vm, "read", &[left]).unwrap_err());
    stale(call(&mut vm, "read", &[right]).unwrap_err());
    stale(call(&mut vm, "get", &[range, zero]).unwrap_err());
}

#[test]
fn value_slot_view_and_subview_return_live_addresses() {
    let mut vm = fixture();
    let (owner, backing) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    let sub = call(
        &mut vm,
        "subview",
        &[span, Value::from_i64(1), Value::from_i64(3)],
    )
    .unwrap();
    let sub = call(&mut vm, "forward", &[sub]).unwrap();
    let reference = element(&mut vm, sub, 0);
    call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap();
    assert_eq!(
        call(&mut vm, "get", &[span, Value::from_i64(1)])
            .unwrap()
            .try_as_i64(),
        Some(73)
    );
    // SAFETY: backing is still live and has three Value slots.
    assert_eq!(
        unsafe { backing.data_ptr().cast::<Value>().add(1).read() }.try_as_i64(),
        Some(73)
    );
    assert_eq!(call(&mut vm, "len", &[sub]).unwrap().try_as_i64(), Some(2));
}
#[test]
fn packed_byte_projection_updates_exactly_one_byte() {
    let mut vm = fixture();
    let (owner, backing) = list(&mut vm, true);
    let span = full(&mut vm, &owner, true);
    let reference = element(&mut vm, span, 1);
    assert_eq!(
        call(&mut vm, "read", &[reference]).unwrap().try_as_i64(),
        Some(42)
    );
    call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap();
    // SAFETY: the live packed allocation is exactly three bytes.
    assert_eq!(
        unsafe { std::slice::from_raw_parts(backing.data_ptr(), 3) },
        &[11, 73, 99]
    );
}
#[test]
fn readonly_input_stays_readonly_through_subviews_and_element_borrows() {
    for bytes in [false, true] {
        let mut vm = fixture();
        let (owner, _) = list(&mut vm, bytes);
        let span = full(&mut vm, &owner, false);
        let sub = call(
            &mut vm,
            "subview",
            &[span, Value::from_i64(1), Value::from_i64(2)],
        )
        .unwrap();
        let reference = element(&mut vm, sub, 0);
        capability(call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap_err());
        assert_eq!(
            call(&mut vm, "read", &[reference]).unwrap().try_as_i64(),
            Some(42)
        );
    }
}
#[test]
fn attenuation_preserves_the_owner_without_revoking_its_parent() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    let readonly = call(&mut vm, "attenuate", &[span]).unwrap();
    let reference = element(&mut vm, readonly, 0);
    capability(call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap_err());
    assert_eq!(
        call(&mut vm, "get", &[span, Value::from_i64(0)])
            .unwrap()
            .try_as_i64(),
        Some(11)
    );
}
#[test]
fn a_new_exclusive_view_revokes_previous_view_and_derived_element() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let old = full(&mut vm, &owner, false);
    let old_element = element(&mut vm, old, 0);
    let new = full(&mut vm, &owner, true);
    stale(call(&mut vm, "get", &[old, Value::from_i64(0)]).unwrap_err());
    stale(call(&mut vm, "read", &[old_element]).unwrap_err());
    assert_eq!(
        call(&mut vm, "get", &[new, Value::from_i64(0)])
            .unwrap()
            .try_as_i64(),
        Some(11)
    );
}
#[test]
fn resizing_revokes_view_even_when_old_managed_backing_remains_live() {
    for bytes in [false, true] {
        let mut vm = fixture();
        let (owner, backing) = list(&mut vm, bytes);
        let old = full(&mut vm, &owner, false);
        assert!(
            call(
                &mut vm,
                "resize",
                &[Value::from_ptr(owner.as_ptr()), Value::from_i64(6)]
            )
            .unwrap()
            .as_bool()
        );
        assert!(vm.state.heap.contains(backing.as_ptr()));
        stale(call(&mut vm, "get", &[old, Value::from_i64(0)]).unwrap_err());
    }
}
#[test]
fn retired_owner_refuses_access_before_touching_its_backing() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let old = full(&mut vm, &owner, false);
    // SAFETY: the test explicitly retires this owner to check the saved borrow.
    unsafe { vm.state.heap.free(owner) };
    stale(call(&mut vm, "get", &[old, Value::from_i64(0)]).unwrap_err());
}
#[test]
fn retired_backing_refuses_element_reads_and_writes() {
    let mut vm = fixture();
    let (owner, backing) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    let reference = element(&mut vm, span, 1);
    // SAFETY: stale ticket must validate membership before accessing this allocation.
    unsafe { vm.state.heap.free(backing) };
    stale(call(&mut vm, "read", &[reference]).unwrap_err());
    stale(call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap_err());
}
#[test]
fn invalid_subview_and_element_ranges_are_refused() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let span = full(&mut vm, &owner, false);
    for (start, end) in [(-1, 2), (2, 1), (0, 4)] {
        assert!(
            call(
                &mut vm,
                "subview",
                &[span, Value::from_i64(start), Value::from_i64(end)]
            )
            .is_err()
        );
    }
    assert!(call(&mut vm, "element", &[span, Value::from_i64(3)]).is_err());
    let empty = call(
        &mut vm,
        "subview",
        &[span, Value::from_i64(3), Value::from_i64(3)],
    )
    .unwrap();
    assert_eq!(
        call(&mut vm, "len", &[empty]).unwrap().try_as_i64(),
        Some(0)
    );
    assert!(call(&mut vm, "element", &[empty, Value::from_i64(0)]).is_err());
}

#[test]
fn raw_backing_uses_the_same_allocator_revocation_as_managed_projection() {
    for packed in [false, true] {
        let mut vm = fixture();
        let width = if packed {
            1
        } else {
            std::mem::size_of::<Value>()
        };
        let raw = call(
            &mut vm,
            "allocate",
            &[Value::from_i64((3 * width) as i64), Value::from_i64(8)],
        )
        .unwrap();
        let address = raw.as_i64() as *mut u8;
        assert!(!address.is_null());
        let owner = vm
            .state
            .heap
            .alloc_array(
                if packed {
                    TypeId::BYTE_LIST
                } else {
                    TypeId::LIST
                },
                3,
            )
            .unwrap();
        // SAFETY: the actual VM allocator registered these three complete elements.
        unsafe {
            for (index, value) in [11, 42, 99].into_iter().enumerate() {
                if packed {
                    address.add(index).write(value as u8);
                } else {
                    address
                        .cast::<Value>()
                        .add(index)
                        .write(Value::from_i64(value));
                }
            }
            owner.data_ptr().cast::<Value>().write(Value::from_i64(3));
            owner
                .data_ptr()
                .cast::<Value>()
                .add(1)
                .write(Value::from_i64(3));
            owner.data_ptr().cast::<Value>().add(2).write(raw);
        }
        let cursor = call(&mut vm, "cursor", &[Value::from_ptr(owner.as_ptr())]).unwrap();
        assert_eq!(call(&mut vm, "advance", &[cursor]).unwrap().try_as_i64(), Some(11));
        let span = full(&mut vm, &owner, true);
        let reference = element(&mut vm, span, 1);
        call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap();
        assert_eq!(call(&mut vm, "advance", &[cursor]).unwrap().try_as_i64(), Some(73));
        assert_eq!(
            call(&mut vm, "get", &[span, Value::from_i64(1)])
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
        call(&mut vm, "free", &[raw]).unwrap();
        assert!(matches!(call(&mut vm, "advance", &[cursor]), Err(InterpreterError::Panic { message })
            if message.contains("no live allocation extent")));
        stale(call(&mut vm, "read", &[reference]).unwrap_err());
        stale(call(&mut vm, "get", &[span, Value::from_i64(1)]).unwrap_err());
    }
}
#[test]
fn packed_projection_rejects_out_of_range_writes_without_touching_neighbors() {
    let mut vm = fixture();
    let (owner, backing) = list(&mut vm, true);
    let span = full(&mut vm, &owner, true);
    let reference = element(&mut vm, span, 1);
    for value in [-1, 256] {
        assert!(call(&mut vm, "write", &[reference, Value::from_i64(value)]).is_err());
    }
    // SAFETY: backing remains the same live three-byte allocation.
    assert_eq!(
        unsafe { std::slice::from_raw_parts(backing.data_ptr(), 3) },
        &[11, 42, 99]
    );
}
#[test]
fn a_live_replaced_backing_does_not_authorize_old_views() {
    let mut vm = fixture();
    let (owner, old_backing) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    let replacement = vm.state.heap.alloc_array(TypeId::INT, 3).unwrap();
    // SAFETY: the owner still has its checked three-field layout. This models
    // an explicit owner update without relying on the resize revocation path.
    unsafe {
        owner
            .data_ptr()
            .cast::<Value>()
            .add(2)
            .write(Value::from_ptr(replacement.as_ptr()));
    }
    assert!(vm.state.heap.contains(old_backing.as_ptr()));
    stale(call(&mut vm, "get", &[span, Value::from_i64(0)]).unwrap_err());
}
#[test]
fn owner_snapshot_drift_is_refused_while_the_allocation_remains_live() {
    for change_epoch in [false, true] {
        let mut vm = fixture();
        let (mut owner, _) = list(&mut vm, false);
        let span = full(&mut vm, &owner, true);
        if change_epoch {
            owner.header_mut().epoch += 1;
        } else {
            owner.header_mut().generation += 1;
        }
        stale(call(&mut vm, "get", &[span, Value::from_i64(0)]).unwrap_err());
    }
}
#[test]
fn write_permission_without_exclusive_authority_is_refused() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    let aliased = call(&mut vm, "without_exclusive", &[span]).unwrap();
    let reference = element(&mut vm, aliased, 0);
    capability(call(&mut vm, "write", &[reference, Value::from_i64(73)]).unwrap_err());
}
#[test]
fn a_checked_span_is_not_exported_as_an_untracked_raw_pointer() {
    let mut vm = fixture();
    let (owner, _) = list(&mut vm, false);
    let span = full(&mut vm, &owner, true);
    assert!(call(&mut vm, "raw", &[span]).is_err());
}

#[test]
fn raw_invalidation_retires_existing_and_new_sequence_projections() {
    let mut vm = fixture();
    let (owner, _unused_backing) = list(&mut vm, false);
    let raw = call(
        &mut vm,
        "allocate",
        &[Value::from_i64(24), Value::from_i64(8)],
    )
    .unwrap();
    let address = raw.as_i64() as *mut Value;
    assert!(!address.is_null());
    // SAFETY: AllocateUser supplied three complete Value slots, and the live
    // List owner has the same three-field layout used by every other control.
    unsafe {
        for (index, value) in [11, 42, 99].into_iter().enumerate() {
            address.add(index).write(Value::from_i64(value));
        }
        owner.data_ptr().cast::<Value>().add(2).write(raw);
    }
    let span = full(&mut vm, &owner, true);
    let reference = element(&mut vm, span, 1);
    call(&mut vm, "invalidate", &[raw]).unwrap();
    stale(call(&mut vm, "get", &[span, Value::from_i64(1)]).unwrap_err());
    stale(call(&mut vm, "read", &[reference]).unwrap_err());
    let error = call(
        &mut vm,
        "mutable",
        &[
            Value::from_ptr(owner.as_ptr()),
            Value::from_i64(0),
            Value::from_i64(3),
        ],
    )
    .unwrap_err();
    assert!(
        matches!(error, InterpreterError::Panic { ref message }
        if message == "List storage backing has no live allocation extent"),
        "{error:?}"
    );
    // Retirement leaves allocator ownership intact until explicit deallocation.
    call(&mut vm, "free", &[raw]).unwrap();
}

#[test]
fn method_calls_preserve_sequence_tickets_and_subview_offsets() {
    for bytes in [false, true] {
        let mut vm = fixture();
        let (owner, _) = list(&mut vm, bytes);
        let span = full(&mut vm, &owner, true);
        let sub = call(
            &mut vm,
            "subview",
            &[span, Value::from_i64(1), Value::from_i64(3)],
        )
        .unwrap();
        for (input, count, first) in [(span, 3, 11), (sub, 2, 42)] {
            let output = call(&mut vm, "method", &[input]).unwrap();
            assert_eq!(
                output.bits(),
                input.bits(),
                "the method must retain its owner ticket"
            );
            assert!(output.try_as_thin_ref().is_none());
            assert_eq!(
                call(&mut vm, "len", &[output]).unwrap().try_as_i64(),
                Some(count)
            );
            assert_eq!(
                call(&mut vm, "get", &[output, Value::from_i64(0)])
                    .unwrap()
                    .try_as_i64(),
                Some(first)
            );
        }
        let selected = element(&mut vm, sub, 0);
        call(&mut vm, "write", &[selected, Value::from_i64(73)]).unwrap();
        assert_eq!(
            call(&mut vm, "get", &[span, Value::from_i64(1)])
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn method_calls_read_elements_using_their_exact_encoding() {
    for bytes in [false, true] {
        let mut vm = fixture();
        let (owner, _) = list(&mut vm, bytes);
        let span = full(&mut vm, &owner, true);
        let selected = element(&mut vm, span, 1);
        let expected = if bytes {
            Value::from_i64(254)
        } else {
            Value::from_f64(-3.25)
        };
        call(&mut vm, "write", &[selected, expected]).unwrap();
        assert_eq!(
            call(&mut vm, "method", &[selected]).unwrap().bits(),
            expected.bits()
        );
        assert_eq!(
            call(&mut vm, "get", &[span, Value::from_i64(0)])
                .unwrap()
                .try_as_i64(),
            Some(11)
        );
        assert_eq!(
            call(&mut vm, "get", &[span, Value::from_i64(2)])
                .unwrap()
                .try_as_i64(),
            Some(99)
        );
    }
}

#[test]
fn method_calls_respect_projection_read_capabilities() {
    for bytes in [false, true] {
        let mut vm = fixture();
        let (owner, _) = list(&mut vm, bytes);
        let span = full(&mut vm, &owner, true);
        let readonly = call(&mut vm, "attenuate", &[span]).unwrap();
        let readonly = call(&mut vm, "method", &[readonly]).unwrap();
        let selected = element(&mut vm, readonly, 1);
        assert_eq!(
            call(&mut vm, "method", &[selected]).unwrap().try_as_i64(),
            Some(42)
        );
        capability(call(&mut vm, "write", &[selected, Value::from_i64(73)]).unwrap_err());
        let mutable_element = element(&mut vm, span, 1);
        for input in [span, mutable_element] {
            let writeonly = call(&mut vm, "writeonly", &[input]).unwrap();
            let result = call(&mut vm, "method", &[writeonly]);
            assert!(
                matches!(result, Err(InterpreterError::CapabilityDenied { required, .. })
                    if required & u64::from(Capabilities::READ) != 0),
                "{result:?}"
            );
        }
    }
}

#[test]
fn method_calls_reject_retired_owners_and_backings() {
    for bytes in [false, true] {
        for retire_owner in [false, true] {
            let mut vm = fixture();
            let (owner, backing) = list(&mut vm, bytes);
            let span = full(&mut vm, &owner, true);
            let selected = element(&mut vm, span, 1);
            // SAFETY: after retirement only checked tickets access the retired allocation.
            unsafe {
                vm.state
                    .heap
                    .free(if retire_owner { owner } else { backing })
            };
            stale(call(&mut vm, "method", &[span]).unwrap_err());
            stale(call(&mut vm, "method", &[selected]).unwrap_err());
        }
    }
}

#[test]
fn method_calls_reject_replaced_or_revoked_sequence_owners() {
    for bytes in [false, true] {
        for replace_backing in [false, true] {
            let mut vm = fixture();
            let (owner, old_backing) = list(&mut vm, bytes);
            let span = full(&mut vm, &owner, true);
            let selected = element(&mut vm, span, 1);
            if replace_backing {
                let (_, replacement) = list(&mut vm, bytes);
                // SAFETY: both allocations are live; update only the owner's backing slot.
                unsafe {
                    owner
                        .data_ptr()
                        .cast::<Value>()
                        .add(2)
                        .write(Value::from_ptr(replacement.as_ptr()));
                }
            } else {
                let fresh = full(&mut vm, &owner, true);
                assert_eq!(
                    call(&mut vm, "method", &[fresh]).unwrap().bits(),
                    fresh.bits()
                );
            }
            assert!(vm.state.heap.contains(old_backing.as_ptr()));
            stale(call(&mut vm, "method", &[span]).unwrap_err());
            stale(call(&mut vm, "method", &[selected]).unwrap_err());
        }
    }
}

#[test]
fn method_calls_reject_foreign_projection_tickets() {
    for bytes in [false, true] {
        let mut issuer = fixture();
        let (owner, _) = list(&mut issuer, bytes);
        let span = full(&mut issuer, &owner, true);
        let selected = element(&mut issuer, span, 1);
        let mut foreign = fixture();
        stale(call(&mut foreign, "method", &[span]).unwrap_err());
        stale(call(&mut foreign, "method", &[selected]).unwrap_err());
    }
}
