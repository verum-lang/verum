//! Retain-copy validates the declared handle and does not publish failed retains.
#![cfg(feature = "codegen")]
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module, interpreter::Interpreter,
    serialize::serialize_module,
};

pub const SOURCE: &str = r#"
    type Handle is { generation: UInt32, value: &Int };
    type Counter is { strong: UInt32 };
    fn retain(handle: &Handle, count: &mut UInt32) -> Handle {
        unsafe { @intrinsic("retain_copy", handle, count) }
    }
    fn main() -> Int {
        let value = 73;
        let mut handle = Handle { generation: 2, value: &value };
        let mut counts = Counter { strong: 1 };
        let copied = retain(&handle, &mut counts.strong);
        if counts.strong != 2 { return 0; }
        handle.generation = 9;
        if copied.generation != 2 { return 0; }
        *copied.value
    }
"#;
fn wire(source: &str) -> verum_vbc::VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("source codegen");
    let wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).expect("checked wire");
    wire
}
#[test]
fn retained_handle_preserves_borrowed_payload_and_increments_its_explicit_counter() {
    let module = wire(SOURCE);
    let entry = module.find_function_by_name("main").unwrap();
    let mut vm = Interpreter::new(module.into());
    assert_eq!(
        vm.execute_function(entry)
            .expect("retained handle")
            .try_as_i64(),
        Some(73)
    );
}
#[test]
fn zero_and_overflow_refuse_and_release_the_unpublished_wrapper() {
    for count in [0, u32::MAX] {
        let module = wire(&SOURCE.replace("strong: 1", &format!("strong: {count}")));
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = Interpreter::new(module.into());
        let error = vm
            .execute_function(entry)
            .expect_err("invalid retain must fail");
        assert!(
            error.to_string().contains("nonzero, non-overflowing"),
            "{error}"
        );
        assert_eq!(
            vm.state.heap.stats().total_frees,
            1,
            "unpublished wrapper is released"
        );
    }
}
#[test]
fn owning_and_unknown_fields_cannot_be_shallow_copied() {
    for field in ["child: Owned", "child: dyn Unknown"] {
        let source = format!(
            "type Unknown is protocol {{}}; type Owned is {{ value: Int }}; type Handle is {{ {field} }}; fn main()->Int{{73}}"
        );
        let module = wire(&source);
        let owner = module
            .types
            .iter()
            .find(|d| {
                module
                    .get_string(d.name)
                    .is_some_and(|n| n == "Handle" || n.ends_with(".Handle"))
            })
            .unwrap()
            .id;
        assert!(
            verum_vbc::retain_copy::layout(&module, &verum_vbc::TypeRef::Concrete(owner)).is_err()
        );
    }
}
#[test]
fn readonly_or_wrong_width_counter_is_refused_by_source_codegen() {
    for target in ["&UInt32", "&mut UInt64"] {
        let source = SOURCE.replace("count: &mut UInt32", &format!("count: {target}"));
        let ast = Parser::new(&source).parse_module().unwrap();
        let error = VbcCodegen::new()
            .compile_module(&ast)
            .expect_err("counter declaration is mandatory");
        assert!(error.to_string().contains("mutable UInt32"), "{error}");
    }
}

#[test]
fn caught_failed_retain_preserves_the_source_count() {
    for count in [0, u32::MAX] {
        let prefix = SOURCE.split("    fn main()").next().unwrap();
        let source = format!(
            r#"{prefix}
            fn main() -> Int {{
                let value = 73;
                let handle = Handle {{ generation: 2, value: &value }};
                let mut counts = Counter {{ strong: {count} }};
                let refused = try {{
                    let copied = retain(&handle, &mut counts.strong);
                    0
                }} recover |_| {{ 1 }};
                if refused != 1 {{ return 0; }}
                if counts.strong != {count} {{ return 1; }}
                *handle.value
            }}
        "#
        );
        let module = wire(&source);
        let entry = module.find_function_by_name("main").unwrap();
        let mut vm = Interpreter::new(module.into());
        assert_eq!(
            vm.execute_function(entry)
                .expect("caught retain error")
                .try_as_i64(),
            Some(73)
        );
        assert_eq!(vm.state.heap.stats().total_frees, 1);
    }
}

#[test]
fn retain_copy_requires_an_explicit_unsafe_contract() {
    let source = SOURCE.replace(
        "unsafe { @intrinsic(\"retain_copy\", handle, count) }",
        "@intrinsic(\"retain_copy\", handle, count)",
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let error = VbcCodegen::new()
        .compile_module(&ast)
        .expect_err("unsafe counter/handle linkage must be explicit");
    assert!(error.to_string().contains("requires unsafe"), "{error}");
}

#[test]
fn runtime_rejects_foreign_owner_readonly_counter_and_revoked_projection() {
    use verum_vbc::value::{Capabilities, ThinRef, Value};
    let source = format!(
        "{SOURCE} type Foreign is {{ generation: UInt32, value: &Int }}; fn counter(c: &mut Counter)->&mut UInt32 {{ &mut c.strong }}"
    );
    for case in 0..4 {
        let module = wire(&source);
        let retain = module.find_function_by_name("retain").unwrap();
        let counter_fn = module.find_function_by_name("counter").unwrap();
        let name = if case == 0 { "Foreign" } else { "Handle" };
        let owner = module
            .types
            .iter()
            .find(|d| module.get_string(d.name) == Some(name))
            .unwrap()
            .id;
        let counter_owner = module
            .types
            .iter()
            .find(|d| module.get_string(d.name) == Some("Counter"))
            .unwrap()
            .id;
        let mut vm = Interpreter::new(module.into());
        let handle = vm.state.heap.alloc(owner, 16).unwrap();
        let count_object = vm.state.heap.alloc(counter_owner, 8).unwrap();
        let count_slot = count_object.data_ptr().cast::<Value>();
        // SAFETY: both live allocations contain exactly the initialized Value slots.
        unsafe {
            handle.data_ptr().cast::<Value>().write(Value::from_i64(3));
            handle.data_ptr().cast::<Value>().add(1).write(Value::nil());
            count_object
                .data_ptr()
                .cast::<Value>()
                .write(Value::from_i64(1));
        }
        let mut handle_slot = Value::from_ptr(handle.as_ptr().cast::<u8>());
        let caps = if case == 3 {
            Capabilities(0)
        } else {
            Capabilities::READ_ONLY
        };
        let handle_ref = Value::from_thin_ref(ThinRef::new(
            (&mut handle_slot as *mut Value).cast(),
            0,
            0,
            caps,
        ));
        let counter_ref = if case == 2 {
            let reference = vm
                .execute_function_with_args(
                    counter_fn,
                    &[Value::from_ptr(count_object.as_ptr().cast::<u8>())],
                )
                .unwrap();
            // SAFETY: this test deliberately revokes the projection's known owner.
            unsafe {
                vm.state.heap.free(count_object);
            }
            reference
        } else {
            let caps = if case == 1 {
                Capabilities::READ_ONLY
            } else {
                Capabilities::MUT_EXCLUSIVE
            };
            // SAFETY: the count owner remains live through the call below.
            let slot = unsafe {
                count_object
                    .as_ptr()
                    .cast::<u8>()
                    .add(verum_common::layout::OBJECT_HEADER_SIZE as usize)
            };
            Value::from_thin_ref(ThinRef::new(slot, 0, 0, caps))
        };
        let before = vm.state.heap.stats().total_allocs;
        let error = vm
            .execute_function_with_args(retain, &[handle_ref, counter_ref])
            .expect_err("forged caller cannot grant ownership");
        if case == 0 {
            assert!(
                error.to_string().contains("differs from its declaration"),
                "{error:?}"
            );
        }
        if case == 1 || case == 3 {
            assert!(
                matches!(
                    error,
                    verum_vbc::interpreter::InterpreterError::CapabilityDenied { .. }
                ),
                "{error:?}"
            );
        }
        if case == 2 {
            assert!(
                matches!(
                    error,
                    verum_vbc::interpreter::InterpreterError::CbgrViolation { .. }
                ),
                "{error:?}"
            );
        }
        assert_eq!(
            vm.state.heap.stats().total_allocs,
            before,
            "reject before allocating a copied wrapper"
        );
        if case != 2 {
            // SAFETY: the counter is still live and no concurrent thread accesses it.
            assert_eq!(unsafe { count_slot.read() }.try_as_i64(), Some(1));
        }
    }
}

#[test]
fn object_header_offsets_match_the_declared_native_carrier() {
    use verum_vbc::interpreter::ObjectHeader;
    assert_eq!(
        std::mem::offset_of!(ObjectHeader, type_id) as u64,
        verum_common::layout::OBJECT_HEADER_TYPE_ID_OFFSET
    );
    assert_eq!(
        std::mem::offset_of!(ObjectHeader, size) as u64,
        verum_common::layout::OBJECT_HEADER_SIZE_OFFSET
    );
}

#[test]
fn empty_record_has_the_same_retain_contract() {
    let module = wire(
        r#"type Handle is {}; fn main()->Int {
        let handle=Handle{}; let mut count:UInt32=1;
        let copied=unsafe { @intrinsic("retain_copy", &handle, &mut count) };
        if count==2 {73} else {0}
    }"#,
    );
    let entry = module.find_function_by_name("main").unwrap();
    assert_eq!(
        Interpreter::new(module.into())
            .execute_function(entry)
            .unwrap()
            .try_as_i64(),
        Some(73)
    );
}
