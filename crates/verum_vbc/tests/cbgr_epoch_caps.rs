//! Snapshot introspection must read reference metadata, never pointee bytes.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
    value::{Capabilities, FatRef, ThinRef, Value},
};

fn reader() -> Interpreter {
    let source = Parser::new(
        "fn epoch_caps(reference: &Int) -> UInt32 { @vbc(CBGR_GET_EPOCH_CAPS, reference) }",
    )
    .parse_module()
    .expect("actual source intrinsic grammar");
    let source = VbcCodegen::new()
        .compile_module(&source)
        .expect("source bytecode");
    let wire = deserialize_module(&serialize_module(&source).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).expect("serialized instruction contract");
    Interpreter::new(wire.into())
}

fn read(vm: &mut Interpreter, value: Value) -> verum_vbc::interpreter::InterpreterResult<Value> {
    let function = vm.state.module.find_function_by_name("epoch_caps").unwrap();
    vm.execute_function_with_args(function, &[value])
}

#[test]
fn canonical_thin_and_fat_reference_snapshots_ignore_pointee_bytes() {
    let mut vm = reader();
    // The pointee is deliberately unrelated to the reference metadata and is
    // large enough for the historical wrong offset to remain addressable.
    let mut data = [17u64, 29, 31, 37];
    for epoch in [0u16, 1, 0x1234, u16::MAX] {
        for caps in [
            0u16,
            Capabilities::READ_ONLY.0,
            Capabilities::MUT_EXCLUSIVE.0,
            0x8001,
        ] {
            let expected = (u32::from(epoch) << verum_common::layout::CAPS_BITS) | u32::from(caps);
            for value in [
                Value::from_thin_ref(ThinRef::new(
                    data.as_mut_ptr().cast(),
                    7,
                    epoch,
                    Capabilities(caps),
                )),
                Value::from_fat_ref(FatRef::new(
                    data.as_mut_ptr().cast(),
                    7,
                    epoch,
                    Capabilities(caps),
                    4,
                )),
            ] {
                assert_eq!(
                    read(&mut vm, value).unwrap().try_as_i64(),
                    Some(i64::from(expected)),
                    "the snapshot is independent of the pointee and unrelated current epoch"
                );
            }
        }
    }
}

#[test]
fn an_integer_cannot_impersonate_a_reference_snapshot() {
    let mut vm = reader();
    for value in [
        Value::from_i64(0),
        Value::from_i64(73),
        Value::from_i64(-1),
        Value::from_bool(true),
        Value::nil(),
    ] {
        assert!(
            read(&mut vm, value).is_err(),
            "non-reference {value:?} must be refused"
        );
    }
}

#[test]
fn an_addressable_ordinary_pointer_is_not_a_snapshot() {
    let mut vm = reader();
    let mut data = [17u64, 29, 31, 37];
    assert!(
        read(&mut vm, Value::from_ptr(data.as_mut_ptr())).is_err(),
        "an address alone cannot authorize a ThinRef layout read"
    );
}

#[test]
fn a_real_source_slice_producer_preserves_its_packed_snapshot_through_wire() {
    let source = Parser::new(
        r#"
        fn main() -> UInt32 {
            let mut values: [Int; 4] = [17, 29, 31, 37];
            let part = &mut values[..];
            @vbc(CBGR_GET_EPOCH_CAPS, part)
        }
    "#,
    )
    .parse_module()
    .expect("slice producer grammar");
    let original = VbcCodegen::new()
        .compile_module(&source)
        .expect("actual source producer");
    let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    for module in [original, wire] {
        verum_vbc::validate::validate_module(&module).unwrap();
        let function = module.find_function_by_name("main").unwrap();
        let mut vm = Interpreter::new(module.into());
        vm.state.cbgr_epoch = 0xA55A;
        let expected = (0xA55Au32 << verum_common::layout::CAPS_BITS)
            | u32::from(Capabilities::MUT_EXCLUSIVE.0);
        assert_eq!(
            vm.execute_function(function).unwrap().try_as_i64(),
            Some(i64::from(expected))
        );
    }
}

#[test]
fn metadata_read_does_not_dereference_a_null_or_retired_pointee() {
    let mut vm = reader();
    let expected = (0xBEEFu32 << verum_common::layout::CAPS_BITS) | 0x8001;
    for pointer in [std::ptr::null_mut(), std::ptr::without_provenance_mut(1)] {
        let value = Value::from_thin_ref(ThinRef::new(pointer, 0, 0xBEEF, Capabilities(0x8001)));
        assert_eq!(
            read(&mut vm, value).unwrap().try_as_i64(),
            Some(i64::from(expected))
        );
    }
}

#[test]
fn a_malformed_fat_reference_table_index_is_refused_without_unwinding() {
    let value = Value::from_fat_ref(FatRef::new(std::ptr::null_mut(), 0, 0, Capabilities(0), 0));
    // Preserve the real tag while selecting an absent entry in its 45-bit index.
    let absent = Value::from_bits(value.to_bits() | ((1u64 << 45) - 1));
    assert!(absent.is_fat_ref());
    assert!(absent.try_as_fat_ref().is_none());
    assert!(matches!(
        read(&mut reader(), absent),
        Err(verum_vbc::interpreter::InterpreterError::InvalidOperand { .. })
    ));
}

#[test]
fn a_source_register_reference_reports_its_missing_snapshot() {
    let source = Parser::new(
        r#"
        fn main() -> UInt32 {
            let value = 73;
            @vbc(CBGR_GET_EPOCH_CAPS, &value)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let main = wire.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::new(wire.into());
    assert!(
        matches!(vm.execute_function(main), Err(verum_vbc::interpreter::InterpreterError::InvalidOperand { message }) if message.contains("preserved reference"))
    );
}

#[test]
fn a_source_managed_field_uses_its_issued_snapshot() {
    let source = Parser::new(
        r#"
        type Packet is { count: Int };
        fn main() -> UInt32 {
            let mut packet = Packet { count: 73 };
            let field = &mut packet.count;
            @vbc(CBGR_GET_EPOCH_CAPS, field)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let main = wire.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::new(wire.into());
    vm.state.cbgr_epoch = 0xA55A;
    let expected = (u32::from(vm.state.heap.current_epoch()) << verum_common::layout::CAPS_BITS)
        | u32::from(Capabilities::MUT_EXCLUSIVE.0);
    assert_eq!(
        vm.execute_function(main).unwrap().try_as_i64(),
        Some(i64::from(expected))
    );
}

#[test]
fn legacy_named_intrinsic_cannot_report_a_fabricated_zero_snapshot() {
    let source = Parser::new(
        r#"
        @intrinsic("verum_cbgr_get_epoch_caps")
        fn legacy(reference: &Int) -> UInt32;
        fn main() -> UInt32 {
            let value = 73;
            let operation = legacy;
            operation(&value)
        }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let main = wire.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::new(wire.into());
    assert!(
        matches!(vm.execute_function(main), Err(verum_vbc::interpreter::InterpreterError::InvalidOperand { message }) if message.contains("registered CBGR instruction"))
    );
}

#[test]
fn an_ordinary_same_spelling_function_keeps_its_declared_body() {
    let source = Parser::new(
        r#"
        fn verum_cbgr_get_epoch_caps(value: Int) -> Int { value }
        fn main() -> Int { verum_cbgr_get_epoch_caps(73) }
    "#,
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let main = wire.find_function_by_name("main").unwrap();
    assert_eq!(
        Interpreter::new(wire.into())
            .execute_function(main)
            .unwrap()
            .try_as_i64(),
        Some(73)
    );
}
