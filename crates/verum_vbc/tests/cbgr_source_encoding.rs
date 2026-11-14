//! The actual standard-library codec must agree with runtime ThinRef bits.
#![cfg(feature = "codegen")]
use verum_ast::ItemKind;
use verum_common::layout;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module, value::Value,
};

#[test]
fn actual_stdlib_epoch_codec_agrees_with_runtime_reference_fields() {
    let mut source = Parser::new(include_str!("../../../core/mem/capability.vr"))
        .parse_module()
        .expect("actual capability grammar");
    source.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Function(function) => matches!(
            function.name.name.as_str(),
            "pack_epoch_caps" | "unpack_epoch" | "unpack_caps"
        ),
        _ => false,
    });
    assert_eq!(
        source.items.len(),
        4,
        "header plus three original functions"
    );
    let original = VbcCodegen::new().compile_module(&source).unwrap();
    let wire = deserialize_module(&serialize_module(&original).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let pack = wire
        .find_function_by_name("core.mem.capability.pack_epoch_caps")
        .unwrap();
    let epoch_fn = wire
        .find_function_by_name("core.mem.capability.unpack_epoch")
        .unwrap();
    let caps_fn = wire
        .find_function_by_name("core.mem.capability.unpack_caps")
        .unwrap();
    let mut vm = Interpreter::new(wire.into());
    for epoch in [0u16, 1, 0x1234, u16::MAX] {
        for caps in [0u16, 1, 3, 0x00ff] {
            let expected = (u32::from(epoch) << layout::CAPS_BITS) | u32::from(caps);
            let packed = vm
                .execute_function_with_args(
                    pack,
                    &[Value::from_i64(epoch.into()), Value::from_i64(caps.into())],
                )
                .unwrap();
            assert_eq!(packed.try_as_i64(), Some(i64::from(expected)));
            for (function, expected) in [(epoch_fn, epoch), (caps_fn, caps)] {
                let result = vm.execute_function_with_args(function, &[packed]).unwrap();
                assert_eq!(result.try_as_i64(), Some(i64::from(expected)));
            }
        }
    }
}
fn bridge_checks() -> Interpreter {
    let mut source = Parser::new(include_str!("../../../core/intrinsics/runtime/cbgr.vr"))
        .parse_module()
        .expect("actual CBGR intrinsic grammar");
    source.items.retain(|item| match &item.kind {
        ItemKind::Module(declaration) => declaration.items.is_none(),
        ItemKind::Function(function) => matches!(
            function.name.name.as_str(),
            "cbgr_allocate"
                | "cbgr_deallocate"
                | "cbgr_check"
                | "cbgr_check_fat"
                | "cbgr_check_write"
        ),
        _ => false,
    });
    assert_eq!(source.items.len(), 6, "module and five actual declarations");
    let wrappers = Parser::new(
        r#"
        fn allocate() -> Int { unsafe { cbgr_allocate(64, 8) } }
        fn release(address: Int) { unsafe { cbgr_deallocate(address); } }
        fn check(address: Int) -> Int { unsafe { cbgr_check(address) } }
        fn check_fat(address: Int) -> Int { unsafe { cbgr_check_fat(address) } }
        fn check_write(address: Int) -> Int { unsafe { cbgr_check_write(address) } }
    "#,
    )
    .parse_module()
    .unwrap();
    source.items.extend(wrappers.items);
    let module = VbcCodegen::new().compile_module(&source).unwrap();
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    Interpreter::new(module.into())
}

fn bridge_call(vm: &mut Interpreter, name: &str, arguments: &[Value]) -> Value {
    let identity = format!("core.intrinsics.runtime.cbgr.{name}");
    let function = vm.state.module.find_function_by_name(&identity).unwrap();
    vm.execute_function_with_args(function, arguments).unwrap()
}

fn check_bridge_reference(vm: &mut Interpreter, reference: &verum_vbc::value::FatRef) -> [i64; 3] {
    let address = Value::from_i64(std::ptr::from_ref(reference) as usize as i64);
    ["check", "check_fat", "check_write"]
        .map(|operation| bridge_call(vm, operation, &[address]).try_as_i64().unwrap())
}

#[test]
fn actual_vm_bridge_checks_use_the_canonical_epoch_and_capability_fields() {
    use verum_vbc::value::{Capabilities, FatRef};
    let mut vm = bridge_checks();
    let user = bridge_call(&mut vm, "allocate", &[]).try_as_i64().unwrap();
    assert!(user > 0);
    let header = user as usize - layout::ALLOCATION_HEADER_SIZE as usize;
    // SAFETY: the real bridge allocator owns this complete live header.
    let generation =
        unsafe { *((header + layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize) as *const u32) };
    for epoch in [0u16, 1, 0x1234, u16::MAX] {
        // SAFETY: change only the live epoch; the reference snapshots it below.
        unsafe {
            *((header + layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize) as *mut u16) = epoch;
        }
        let readonly = FatRef::new(
            user as usize as *mut u8,
            generation,
            epoch,
            Capabilities::READ_ONLY,
            64,
        );
        assert_eq!(
            check_bridge_reference(&mut vm, &readonly),
            [1, 1, 0],
            "readonly epoch={epoch}"
        );
        let mutable = FatRef::new(
            user as usize as *mut u8,
            generation,
            epoch,
            Capabilities::MUT_EXCLUSIVE,
            64,
        );
        assert_eq!(
            check_bridge_reference(&mut vm, &mutable),
            [1, 1, 1],
            "mutable epoch={epoch}"
        );
        let no_access = FatRef::new(
            user as usize as *mut u8,
            generation,
            epoch,
            Capabilities::new(0),
            64,
        );
        assert_eq!(check_bridge_reference(&mut vm, &no_access), [0, 0, 0]);
    }
    bridge_call(&mut vm, "release", &[Value::from_i64(user)]);
}

#[test]
fn actual_vm_bridge_checks_reject_revocation_before_reading_a_released_header() {
    use verum_vbc::value::{Capabilities, FatRef};
    let mut vm = bridge_checks();
    let user = bridge_call(&mut vm, "allocate", &[]).try_as_i64().unwrap();
    assert!(user > 0);
    let header = user as usize - layout::ALLOCATION_HEADER_SIZE as usize;
    // SAFETY: all fields belong to the complete live header returned by the allocator.
    let (generation, epoch, caps) = unsafe {
        (
            *((header + layout::ALLOCATION_HEADER_GENERATION_OFFSET as usize) as *const u32),
            *((header + layout::ALLOCATION_HEADER_EPOCH_OFFSET as usize) as *const u16),
            *((header + layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *const u16),
        )
    };
    let reference = FatRef::new(
        user as usize as *mut u8,
        generation,
        epoch,
        Capabilities::MUT_EXCLUSIVE,
        64,
    );
    assert_eq!(check_bridge_reference(&mut vm, &reference), [1, 1, 1]);
    for invalid in [
        FatRef::new(
            reference.ptr(),
            generation.wrapping_add(1),
            epoch,
            Capabilities::MUT_EXCLUSIVE,
            64,
        ),
        FatRef::new(
            reference.ptr(),
            generation,
            epoch.wrapping_add(1),
            Capabilities::MUT_EXCLUSIVE,
            64,
        ),
        FatRef::new(reference.ptr(), 0, epoch, Capabilities::MUT_EXCLUSIVE, 64),
        FatRef::new(
            std::ptr::null_mut(),
            generation,
            epoch,
            Capabilities::MUT_EXCLUSIVE,
            64,
        ),
    ] {
        assert_eq!(check_bridge_reference(&mut vm, &invalid), [0, 0, 0]);
    }
    // SAFETY: revoke write in the existing owner, without freeing its metadata.
    unsafe {
        *((header + layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *mut u16) =
            Capabilities::READ_ONLY.0;
    }
    assert_eq!(check_bridge_reference(&mut vm, &reference), [1, 1, 0]);
    // SAFETY: revoke all access in that same live owner.
    unsafe {
        *((header + layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *mut u16) = 0;
    }
    assert_eq!(check_bridge_reference(&mut vm, &reference), [0, 0, 0]);
    // SAFETY: restore its initialized capability field before ordinary release.
    unsafe {
        *((header + layout::ALLOCATION_HEADER_CAPABILITIES_OFFSET as usize) as *mut u16) = caps;
    }
    bridge_call(&mut vm, "release", &[Value::from_i64(user)]);
    assert_eq!(check_bridge_reference(&mut vm, &reference), [0, 0, 0]);
}
