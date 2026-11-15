//! Mixed generations refuse before an owned parameter body or cleanup executes.
#![cfg(feature = "codegen")]
use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    FunctionId, Instruction, Reg, TypeId, Value, VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module,
    instruction::RegRange,
    interpreter::Interpreter,
    module::FunctionDescriptor,
    serialize::serialize_module,
};

fn compile(source: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new("convention"))
        .compile_module(&Parser::new(source).parse_module().unwrap())
        .unwrap()
}
fn wire(module: &VbcModule) -> VbcModule {
    deserialize_module(&serialize_module(module).unwrap()).unwrap()
}
fn id(module: &VbcModule, name: &str) -> FunctionId {
    module
        .find_function_by_name(&format!("convention.{name}"))
        .unwrap()
}
fn legacy_body(module: &mut VbcModule, function: FunctionId, body: &[Instruction]) {
    let mut encoded = List::new().into();
    verum_vbc::bytecode::encode_instructions(body, &mut encoded);
    let start = module.append_bytecode(&encoded);
    let descriptor = module
        .functions
        .iter_mut()
        .find(|item| item.id == function)
        .unwrap();
    descriptor.owned_value_parameters = false;
    descriptor.value_uses = None;
    descriptor.source_definition = None;
    descriptor.instructions = None;
    descriptor.bytecode_offset = start;
    descriptor.bytecode_length = encoded.len() as u32;
}

#[test]
fn emitted_parameter_convention_roundtrips_with_its_body_and_call() {
    let module = compile("fn take(value: Int) -> Int { value } fn probe() -> Int { take(73) }");
    for module in [module.clone(), wire(&module)] {
        for function in &module.functions {
            assert!(function.has_source_body && function.owned_value_parameters);
        }
        let entry = id(&module, "probe");
        assert_eq!(
            Interpreter::new(module.into())
                .execute_function(entry)
                .unwrap()
                .try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn legacy_caller_and_tail_call_refuse_before_current_parameter_cleanup() {
    for tail in [false, true] {
        let mut module = compile(
            "type Cell is { value: Int }; fn take(value: Cell) {} fn probe(value: Cell) { take(value); }",
        );
        let take = id(&module, "take");
        let probe = id(&module, "probe");
        // A legacy caller passes its original register without the new handoff.
        let call = if tail {
            Instruction::TailCall {
                func_id: take.0,
                args: RegRange {
                    start: Reg(0),
                    count: 1,
                },
            }
        } else {
            Instruction::Call {
                dst: Reg(1),
                func_id: take.0,
                args: RegRange {
                    start: Reg(0),
                    count: 1,
                },
            }
        };
        legacy_body(&mut module, probe, &[call, Instruction::RetV]);
        let module = wire(&module);
        let cell = module
            .types
            .iter()
            .find(|ty| {
                module
                    .get_string(ty.name)
                    .is_some_and(|name| name.ends_with("Cell"))
            })
            .unwrap()
            .id;
        let mut vm = Interpreter::new(module.into());
        let object = vm.state.heap.alloc_array(cell, 1).unwrap();
        // SAFETY: the live allocation contains one initialized Value slot.
        unsafe {
            object.data_ptr().cast::<Value>().write(Value::from_i64(73));
        }
        let error = vm
            .execute_function_with_args(probe, &[Value::from_ptr(object.as_ptr().cast::<u8>())])
            .unwrap_err();
        assert!(
            error.to_string().contains("incompatible owned parameter"),
            "{error}"
        );
        assert!(
            vm.state.heap.contains(object.as_ptr()),
            "callee cleanup retired the caller object"
        );
        // SAFETY: exact live heap membership was checked after the refusal.
        assert_eq!(
            unsafe { object.data_ptr().cast::<Value>().read() }.try_as_i64(),
            Some(73)
        );
    }
}

#[test]
fn current_direct_and_indirect_calls_refuse_legacy_body_before_it_executes() {
    let mut module = compile(
        "fn old(value: Int) {} fn direct() { old(73); } fn indirect(selected: fn(Int)) { selected(73); }",
    );
    let old = id(&module, "old");
    let message_id = module.intern_string("legacy body executed").0;
    legacy_body(
        &mut module,
        old,
        &[Instruction::Panic { message_id }, Instruction::RetV],
    );
    let module = wire(&module);
    for (entry, args) in [
        (id(&module, "direct"), List::new()),
        (
            id(&module, "indirect"),
            List::from(&[Value::from_function(old)][..]),
        ),
    ] {
        let error = Interpreter::new(module.clone().into())
            .execute_function_with_args(entry, &args)
            .unwrap_err();
        assert!(
            error.to_string().contains("incompatible owned parameter"),
            "{error}"
        );
        assert!(!error.to_string().contains("legacy body executed"));
    }
    // The same legacy body is observable when entered under its own convention.
    let error = Interpreter::new(module.into())
        .execute_function_with_args(old, &[Value::from_i64(73)])
        .unwrap_err();
    assert!(
        error.to_string().contains("legacy body executed"),
        "{error}"
    );
}

#[test]
fn bodyless_declarations_cannot_certify_cleanup_and_old_versions_cannot_carry_the_flag() {
    let mut module = VbcModule::new("forward".into());
    let mut function = FunctionDescriptor::new(module.intern_string("forward"));
    function.owned_value_parameters = true;
    module.functions.push(function);
    assert!(
        serialize_module(&module)
            .unwrap_err()
            .to_string()
            .contains("emitted synchronous source body")
    );
    module.functions[0].owned_value_parameters = false;
    let mut bytes = serialize_module(&module).unwrap();
    let offset = deserialize_module(&bytes)
        .unwrap()
        .header
        .function_table_offset as usize;
    // Exact codec: id:u32, name:u32, absent parent:u8, visibility:u8, flags:u8.
    bytes[offset + 10] |= 0x10;
    assert!(
        deserialize_module(&bytes)
            .unwrap_err()
            .to_string()
            .contains("emitted synchronous source body")
    );
    let module = compile("fn probe(value: Int) -> Int { value }");
    let mut bytes = serialize_module(&module).unwrap();
    bytes[6..8].copy_from_slice(&59u16.to_le_bytes());
    assert!(
        deserialize_module(&bytes)
            .unwrap_err()
            .to_string()
            .contains("requires VBC minor 60")
    );
    let mut unsupported = module.clone();
    unsupported.header.version_minor = verum_vbc::VERSION_MINOR + 1;
    assert!(matches!(
        verum_vbc::validate::validate_module(&unsupported),
        Err(verum_vbc::VbcError::UnsupportedVersion { .. })
    ));
    let mut legacy = module;
    for function in &mut legacy.functions {
        function.owned_value_parameters = false;
        function.value_uses = None;
    }
    let mut bytes = serialize_module(&legacy).unwrap();
    bytes[6..8].copy_from_slice(&59u16.to_le_bytes());
    assert!(
        deserialize_module(&bytes)
            .unwrap()
            .functions
            .iter()
            .all(|function| !function.owned_value_parameters)
    );
}

#[test]
fn changing_parameter_convention_invalidates_a_body_use_receipt() {
    let module = compile("fn probe(value: Int) -> Int { let copied = value; copied }");
    let descriptor = module.get_function(id(&module, "probe")).unwrap();
    let plan = descriptor.value_uses.as_ref().expect("source use receipt");
    assert!(plan.matches_signature(descriptor));
    let mut changed = descriptor.clone();
    changed.owned_value_parameters = false;
    assert!(!plan.matches_signature(&changed));
    assert_eq!(FunctionDescriptor::default().owned_value_parameters, false);
    assert_eq!(TypeId::INT, descriptor.return_type.base_type_id().unwrap());
}

#[test]
fn incomplete_parameter_inventory_is_refused_instead_of_certified() {
    let ast = Parser::new("fn take((left, right): (Int, Int)) {}")
        .parse_module()
        .unwrap();
    let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("complete declaration and binding inventory"),
        "{error}"
    );
}

#[test]
fn an_async_mutable_parameter_retains_its_existing_copy_boundary() {
    let module = wire(&compile(
        "type Cell is { value: Int }; async fn alter(mut value: Cell) { value.value = 0; }",
    ));
    let alter = id(&module, "alter");
    assert!(!module.get_function(alter).unwrap().owned_value_parameters);
    let cell = module
        .types
        .iter()
        .find(|ty| {
            module
                .get_string(ty.name)
                .is_some_and(|name| name.ends_with("Cell"))
        })
        .unwrap()
        .id;
    let mut vm = Interpreter::new(module.into());
    let object = vm.state.heap.alloc_array(cell, 1).unwrap();
    // SAFETY: this live allocation owns exactly one Value slot.
    unsafe {
        object.data_ptr().cast::<Value>().write(Value::from_i64(73));
    }
    vm.execute_function_with_args(alter, &[Value::from_ptr(object.as_ptr().cast::<u8>())])
        .unwrap();
    assert!(vm.state.heap.contains(object.as_ptr()));
    // SAFETY: exact live heap membership was checked after execution.
    assert_eq!(
        unsafe { object.data_ptr().cast::<Value>().read() }.try_as_i64(),
        Some(73)
    );
}

#[test]
fn an_async_caller_keeps_the_legacy_outgoing_value_boundary() {
    let mut module = compile(
        "type Cell is { value: Int }; fn old(mut value: Cell) { value.value = 41; } async fn probe(value: Cell) -> Int { old(value); value.value }",
    );
    let old = id(&module, "old");
    let probe = id(&module, "probe");
    assert!(!module.get_function(probe).unwrap().owned_value_parameters);
    legacy_body(
        &mut module,
        old,
        &[
            Instruction::LoadI {
                dst: Reg(1),
                value: 41,
            },
            Instruction::SetF {
                obj: Reg(0),
                field_idx: 0,
                value: Reg(1),
            },
            Instruction::RetV,
        ],
    );
    let module = wire(&module);
    let cell = module
        .types
        .iter()
        .find(|ty| {
            module
                .get_string(ty.name)
                .is_some_and(|name| name.ends_with("Cell"))
        })
        .unwrap()
        .id;
    let mut vm = Interpreter::new(module.into());
    let object = vm.state.heap.alloc_array(cell, 1).unwrap();
    // SAFETY: this live allocation owns exactly one Value slot.
    unsafe {
        object.data_ptr().cast::<Value>().write(Value::from_i64(73));
    }
    let result = vm
        .execute_function_with_args(probe, &[Value::from_ptr(object.as_ptr().cast::<u8>())])
        .unwrap();
    assert_eq!(
        result.try_as_i64(),
        Some(41),
        "an unflagged caller changed its outgoing convention"
    );
    assert!(vm.state.heap.contains(object.as_ptr()));
    // SAFETY: exact live heap membership was checked after execution.
    assert_eq!(
        unsafe { object.data_ptr().cast::<Value>().read() }.try_as_i64(),
        Some(41)
    );
}

#[test]
fn raw_pointer_declarations_do_not_fabricate_alias_witnesses() {
    for (declarations, parameter) in [
        ("", "*const Int"),
        ("type Address is *const Int;", "Address"),
        (
            "type Address is *const Int; type Alias is Address;",
            "Alias",
        ),
    ] {
        let source = format!(
            "{declarations} unsafe fn raw(value: {parameter}) -> Int {{ 73 }} fn probe() -> Int {{ let value = 11; unsafe {{ raw(&value as *const Int) }} }}"
        );
        if !declarations.is_empty() {
            // Pointer aliases still lack a complete refinement target in the
            // source producer. An ownership exclusion must not fabricate it.
            let ast = Parser::new(&source).parse_module().unwrap();
            let error = VbcCodegen::with_config(CodegenConfig::new("convention"))
                .compile_module(&ast)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("refinement alias has no declared target"),
                "{parameter}: {error}"
            );
            continue;
        }
        let module = wire(&compile(&source));
        let raw = id(&module, "raw");
        let probe = id(&module, "probe");
        assert!(module.get_function(raw).unwrap().owned_value_parameters);
        let mut vm = Interpreter::new(module.into());
        assert_eq!(
            vm.execute_function(probe).unwrap().try_as_i64(),
            Some(73),
            "{parameter}"
        );
        // A raw address is never read as an owned heap header by cleanup.
        assert_eq!(
            vm.execute_function_with_args(
                raw,
                &[Value::from_ptr(
                    std::ptr::NonNull::<u8>::dangling().as_ptr()
                )]
            )
            .unwrap()
            .try_as_i64(),
            Some(73),
            "{parameter}"
        );
    }
}

#[test]
fn a_callers_raw_alias_cannot_exclude_a_foreign_value_parameter() {
    let provider = "public module provider { public type Address is { public value: Int }; public fn make() -> Address { Address { value: 73 } } public fn alter(mut value: Address) { value.value = 0; } }";
    let consumer = "module consumer { mount super.provider as selected; type Address is *const Int; fn probe() -> Int { let original = selected.make(); selected.alter(original); original.value } }";
    for reverse in [false, true] {
        let source = if reverse {
            format!("{consumer} {provider}")
        } else {
            format!("{provider} {consumer}")
        };
        let module = compile(&source);
        let entry = id(&module, "consumer.probe");
        for module in [module.clone(), wire(&module)] {
            assert_eq!(
                Interpreter::new(module.into())
                    .execute_function(entry)
                    .unwrap()
                    .try_as_i64(),
                Some(73),
                "declaration order reversed={reverse}"
            );
        }
    }
}
