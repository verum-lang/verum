//! Sized scalar operations preserve checked projection storage and rights.
use verum_vbc::{
    bytecode::encode_instruction,
    deserialize::deserialize_module,
    instruction::{CbgrSubOpcode as C, Instruction as I, MemSubOpcode as M, Reg},
    interpreter::{Interpreter, InterpreterError, InterpreterResult, Object},
    module::{FunctionDescriptor, FunctionId, VbcModule},
    serialize::serialize_module,
    types::TypeId,
    value::{Capabilities, Value},
};

fn cbgr(op: C, operands: &[u8]) -> I {
    I::CbgrExtended {
        sub_op: op as u8,
        operands: operands.into(),
    }
}
fn mem(op: M, operands: &[u8]) -> I {
    I::MemExtended {
        sub_op: op as u8,
        operands: operands.into(),
    }
}
fn function(module: &mut VbcModule, name: &str, body: &[I]) {
    let mut descriptor = FunctionDescriptor::new(module.strings.intern(name));
    descriptor.id = FunctionId(module.functions.len() as u32);
    descriptor.register_count = 8;
    descriptor.bytecode_offset = module.bytecode.len() as u32;
    for instruction in body {
        encode_instruction(instruction, &mut module.bytecode);
    }
    descriptor.bytecode_length = module.bytecode.len() as u32 - descriptor.bytecode_offset;
    module.functions.push(descriptor);
}
fn fixture() -> Interpreter {
    let mut module = VbcModule::new("managed_scalar_projection".into());
    function(
        &mut module,
        "field",
        &[cbgr(C::RefField, &[4, 0, 1]), I::Ret { value: Reg(4) }],
    );
    for (name, allowed) in [
        ("readonly", Capabilities::READ),
        ("writeonly", Capabilities::WRITE),
    ] {
        function(
            &mut module,
            name,
            &[
                cbgr(
                    C::CapAttenuate,
                    &[4, 0, allowed as u8, (allowed >> 8) as u8],
                ),
                I::Ret { value: Reg(4) },
            ],
        );
    }
    function(
        &mut module,
        "span",
        &[
            I::RefMut {
                dst: Reg(4),
                src: Reg(0),
            },
            cbgr(C::RefSlice, &[5, 4, 1, 2]),
            I::Ret { value: Reg(5) },
        ],
    );
    function(
        &mut module,
        "element",
        &[
            cbgr(C::RefListElement, &[4, 0, 1]),
            I::Ret { value: Reg(4) },
        ],
    );
    for width in [1, 2, 4, 8] {
        function(
            &mut module,
            &format!("read{width}"),
            &[mem(M::DerefRaw, &[4, 0, width]), I::Ret { value: Reg(4) }],
        );
        function(
            &mut module,
            &format!("signed{width}"),
            &[
                mem(M::DerefRawSigned, &[4, 0, width]),
                I::Ret { value: Reg(4) },
            ],
        );
        function(
            &mut module,
            &format!("write{width}"),
            &[mem(M::DerefMutRaw, &[0, 1, width]), I::RetV],
        );
    }
    let wire = serialize_module(&module).unwrap();
    let decoded = deserialize_module(&wire).unwrap();
    Interpreter::try_new_validated(decoded.into()).expect("validated wire consumer")
}
fn call(vm: &mut Interpreter, name: &str, args: &[Value]) -> InterpreterResult<Value> {
    let id = vm.state.module.find_function_by_name(name).unwrap();
    vm.execute_function_with_args(id, args)
}
fn field(vm: &mut Interpreter, initial: Value) -> (Object, Value) {
    let owner = vm.state.heap.alloc(TypeId(40_001), 24).unwrap();
    // SAFETY: the fresh allocation has exactly three initialized Value slots.
    unsafe {
        let slots = owner.data_ptr().cast::<Value>();
        slots.write(Value::from_i64(91));
        slots.add(1).write(initial);
        slots.add(2).write(Value::from_i64(93));
    }
    let projection = call(vm, "field", &[Value::from_ptr(owner.as_ptr())]).unwrap();
    (owner, projection)
}
fn assert_slots(owner: &Object, middle: Value) {
    // SAFETY: all callers retain this live three-slot allocation.
    let slots = unsafe { std::slice::from_raw_parts(owner.data_ptr().cast::<Value>(), 3) };
    assert_eq!(slots, &[Value::from_i64(91), middle, Value::from_i64(93)]);
}
fn packed(vm: &mut Interpreter) -> (Object, Object, Value, Value) {
    let backing = vm.state.heap.alloc(TypeId::U8, 3).unwrap();
    let owner = vm.state.heap.alloc_array(TypeId::BYTE_LIST, 3).unwrap();
    // SAFETY: the backing is three bytes; the owner has three Value fields.
    unsafe {
        std::ptr::copy_nonoverlapping([11u8, 254, 99].as_ptr(), backing.data_ptr(), 3);
        let slots = owner.data_ptr().cast::<Value>();
        slots.write(Value::from_i64(3));
        slots.add(1).write(Value::from_i64(3));
        slots.add(2).write(Value::from_ptr(backing.as_ptr()));
    }
    let span = call(
        vm,
        "span",
        &[
            Value::from_ptr(owner.as_ptr()),
            Value::from_i64(0),
            Value::from_i64(3),
        ],
    )
    .unwrap();
    let element = call(vm, "element", &[span, Value::from_i64(1)]).unwrap();
    (owner, backing, span, element)
}
fn assert_bytes(backing: &Object, middle: u8) {
    // SAFETY: the caller retains the live three-byte backing.
    assert_eq!(
        unsafe { std::slice::from_raw_parts(backing.data_ptr(), 3) },
        &[11, middle, 99]
    );
}
fn denied(result: InterpreterResult<Value>, required: u16) {
    assert!(
        matches!(result, Err(InterpreterError::CapabilityDenied { required: actual, .. }) if actual & u64::from(required) != 0),
        "{result:?}"
    );
}
fn stale(result: InterpreterResult<Value>) {
    assert!(
        matches!(result, Err(InterpreterError::CbgrViolation { .. })),
        "{result:?}"
    );
}
fn invalid(result: InterpreterResult<Value>) {
    assert!(
        matches!(result, Err(InterpreterError::InvalidOperand { .. })),
        "{result:?}"
    );
}

#[test]
fn full_width_loads_preserve_managed_value_encoding() {
    for expected in [
        Value::from_i64(-73),
        Value::from_f64(3.25),
        Value::from_bool(true),
        Value::from_char('λ'),
    ] {
        let mut vm = fixture();
        let (owner, projection) = field(&mut vm, expected);
        assert_eq!(
            call(&mut vm, "read8", &[projection]).unwrap().bits(),
            expected.bits()
        );
        assert_slots(&owner, expected);
    }
    let mut vm = fixture();
    let (_, projection) = field(&mut vm, Value::from_i64(-73));
    assert_eq!(
        call(&mut vm, "signed8", &[projection])
            .unwrap()
            .try_as_i64(),
        Some(-73)
    );
}

#[test]
fn full_width_writes_update_only_the_selected_value_slot() {
    for expected in [
        Value::from_i64(73),
        Value::from_f64(-1.5),
        Value::from_bool(false),
        Value::from_char('界'),
    ] {
        let mut vm = fixture();
        let (owner, projection) = field(&mut vm, Value::from_i64(7));
        call(&mut vm, "write8", &[projection, expected]).unwrap();
        assert_slots(&owner, expected);
        assert_eq!(
            call(&mut vm, "read8", &[projection]).unwrap().bits(),
            expected.bits()
        );
    }
}

#[test]
fn readonly_scalar_projection_reads_but_refuses_writes() {
    let mut vm = fixture();
    let (owner, projection) = field(&mut vm, Value::from_i64(7));
    let readonly = call(&mut vm, "readonly", &[projection]).unwrap();
    assert_eq!(
        call(&mut vm, "read8", &[readonly]).unwrap().try_as_i64(),
        Some(7)
    );
    denied(
        call(&mut vm, "write8", &[readonly, Value::from_i64(99)]),
        Capabilities::WRITE,
    );
    assert_slots(&owner, Value::from_i64(7));
}

#[test]
fn writeonly_scalar_projection_cannot_authorize_a_read() {
    let mut vm = fixture();
    let (owner, projection) = field(&mut vm, Value::from_i64(7));
    let writeonly = call(&mut vm, "writeonly", &[projection]).unwrap();
    denied(call(&mut vm, "read8", &[writeonly]), Capabilities::READ);
    call(&mut vm, "write8", &[writeonly, Value::from_i64(73)]).unwrap();
    assert_slots(&owner, Value::from_i64(73));
}

#[test]
fn retired_scalar_owner_refuses_reads_and_writes() {
    let mut vm = fixture();
    let (owner, projection) = field(&mut vm, Value::from_i64(7));
    // SAFETY: no direct access follows retirement; only the saved checked ticket is used.
    unsafe {
        vm.state.heap.free(owner);
    }
    let _replacement = field(&mut vm, Value::from_i64(99));
    stale(call(&mut vm, "read8", &[projection]));
    stale(call(&mut vm, "signed8", &[projection]));
    stale(call(&mut vm, "write8", &[projection, Value::from_i64(73)]));
}

#[test]
fn a_foreign_heap_cannot_use_the_projection_ticket() {
    let mut issuer = fixture();
    let (_, projection) = field(&mut issuer, Value::from_i64(7));
    let mut foreign = fixture();
    stale(call(&mut foreign, "read8", &[projection]));
    stale(call(
        &mut foreign,
        "write8",
        &[projection, Value::from_i64(73)],
    ));
}

#[test]
fn partial_access_cannot_reinterpret_or_corrupt_a_value_slot() {
    let mut vm = fixture();
    let (owner, projection) = field(&mut vm, Value::from_f64(3.25));
    for width in [1, 2, 4] {
        invalid(call(&mut vm, &format!("read{width}"), &[projection]));
        invalid(call(&mut vm, &format!("signed{width}"), &[projection]));
        invalid(call(
            &mut vm,
            &format!("write{width}"),
            &[projection, Value::from_i64(0)],
        ));
        assert_slots(&owner, Value::from_f64(3.25));
    }
}

#[test]
fn packed_byte_scalar_access_uses_the_declared_encoding() {
    let mut vm = fixture();
    let (_, backing, _, projection) = packed(&mut vm);
    assert_eq!(
        call(&mut vm, "read1", &[projection]).unwrap().try_as_i64(),
        Some(254)
    );
    assert_eq!(
        call(&mut vm, "signed1", &[projection])
            .unwrap()
            .try_as_i64(),
        Some(-2)
    );
    call(&mut vm, "write1", &[projection, Value::from_i64(73)]).unwrap();
    assert_bytes(&backing, 73);
}

#[test]
fn packed_byte_rejects_wider_access_and_out_of_range_values() {
    let mut vm = fixture();
    let (_, backing, _, projection) = packed(&mut vm);
    for width in [2, 4, 8] {
        invalid(call(&mut vm, &format!("read{width}"), &[projection]));
        invalid(call(
            &mut vm,
            &format!("write{width}"),
            &[projection, Value::from_i64(0)],
        ));
    }
    for value in [
        Value::from_i64(-1),
        Value::from_i64(256),
        Value::from_bool(true),
    ] {
        invalid(call(&mut vm, "write1", &[projection, value]));
    }
    assert_bytes(&backing, 254);
}

#[test]
fn scalar_consumers_reject_sequence_views() {
    let mut vm = fixture();
    let (_, backing, span, _) = packed(&mut vm);
    invalid(call(&mut vm, "read1", &[span]));
    invalid(call(&mut vm, "write1", &[span, Value::from_i64(0)]));
    assert_bytes(&backing, 254);
}

#[test]
fn packed_readonly_and_retired_backing_retain_their_guards() {
    let mut vm = fixture();
    let (_, backing, _, projection) = packed(&mut vm);
    let readonly = call(&mut vm, "readonly", &[projection]).unwrap();
    assert_eq!(
        call(&mut vm, "read1", &[readonly]).unwrap().try_as_i64(),
        Some(254)
    );
    denied(
        call(&mut vm, "write1", &[readonly, Value::from_i64(0)]),
        Capabilities::WRITE,
    );
    // SAFETY: retirement is followed only by checked-ticket operations.
    unsafe {
        vm.state.heap.free(backing);
    }
    stale(call(&mut vm, "read1", &[projection]));
    stale(call(&mut vm, "write1", &[projection, Value::from_i64(0)]));
}

#[test]
fn scalar_projection_never_exports_an_unchecked_address() {
    let mut vm = fixture();
    let (_, projection) = field(&mut vm, Value::from_i64(7));
    assert!(projection.try_as_thin_ref().is_none());
}

#[test]
fn native_scalar_pointer_reads_and_writes_keep_their_contract() {
    let mut vm = fixture();
    let mut storage = -42_i32;
    let pointer = Value::from_ptr(std::ptr::from_mut(&mut storage));
    assert_eq!(
        call(&mut vm, "signed4", &[pointer]).unwrap().try_as_i64(),
        Some(-42)
    );
    assert_eq!(
        call(&mut vm, "read4", &[pointer]).unwrap().try_as_i64(),
        Some(u32::MAX as i64 - 41)
    );
    call(&mut vm, "write4", &[pointer, Value::from_i64(73)]).unwrap();
    assert_eq!(storage, 73);
}
