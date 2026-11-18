//! Exception ownership must survive recovery, propagation and value handoff.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    deserialize::deserialize_module_validated,
    interpreter::Interpreter,
    serialize::serialize_module,
};

fn check(source: &str) {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("exception_ownership"));
    canonical_drop::initialize(&mut codegen);
    let module = codegen.compile_additional_module(&ast).expect("source VBC");
    let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("exception_ownership.probe")
            .unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        assert_eq!(
            vm.execute_function(entry).expect(route).try_as_i64(),
            Some(73)
        );
        assert_eq!(
            vm.state.heap.stats().total_frees,
            vm.state.heap.stats().total_allocs,
            "all exception payload owners must be retired, {route}"
        );
    }
}

#[test]
fn ignored_panic_payload_is_released_at_the_recovery_boundary() {
    check("fn probe()->Int { try { assert(false); 0 } recover |_| { 73 } }");
}

#[test]
fn named_unused_panic_payload_is_released_at_the_recovery_boundary() {
    check("fn probe()->Int { try { assert(false); 0 } recover |error| { 73 } }");
}

#[test]
fn propagated_panic_payload_is_released_only_by_its_final_handler() {
    check(
        r#"
        fn probe()->Int {
            try { try { assert(false); 0 } recover |error| { throw error } }
            recover |_| { 73 }
        }
    "#,
    );
}

#[test]
fn a_shadowed_recovery_binding_does_not_publish_the_panic() {
    check("fn probe()->Int { try { assert(false); 0 } recover |error| { let error = 73; error } }");
}

#[test]
fn observing_a_thrown_value_preserves_the_selected_value_and_handler_work() {
    check(
        r#"
        fn probe()->Int {
            let mut count = 0;
            let forwarded = try {
                try { throw 10 } recover |error| { count = count + 1; throw error + 2 }
            } recover |error| { error };
            let replaced = try { try { throw 37 } recover |error| { throw 60 } }
                recover |error| { error };
            count + forwarded + replaced
        }
    "#,
    );
}

fn wire_body(body: &[verum_vbc::Instruction]) -> verum_common::List<u8> {
    use verum_vbc::{
        VbcModule, bytecode::encode_instructions_with_fixup, module::FunctionDescriptor,
    };
    let mut module = VbcModule::new("exception_wire".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.register_count = 2;
    function.bytecode_length = encode_instructions_with_fixup(body, &mut module.bytecode) as u32;
    module.add_function(function);
    serialize_module(&module).unwrap().into()
}

#[test]
fn dispositions_require_the_new_wire_version_a_handler_edge_and_valid_registers() {
    use verum_vbc::{Instruction as I, instruction::Reg};
    for disposition in [I::DiscardException { dst: Reg(0) }, I::RethrowException] {
        let valid = wire_body(&[
            I::TryBegin { handler_offset: 2 },
            I::Unreachable,
            disposition.clone(),
            I::RetV,
        ]);
        deserialize_module_validated(&valid).expect("dedicated handler entry");
        let mut old = valid;
        old.as_mut_slice()[6..8].copy_from_slice(&64u16.to_le_bytes());
        let error = deserialize_module_validated(&old).unwrap_err().to_string();
        assert!(error.contains("requires VBC minor 65"), "{error}");
        for (body, wanted) in [
            (
                vec![I::RetV, disposition.clone()],
                "requires a handler entry",
            ),
            (
                vec![I::TryBegin { handler_offset: 1 }, disposition.clone()],
                "exclusively exceptional",
            ),
            (
                vec![
                    I::TryBegin { handler_offset: 3 },
                    I::Jmp { offset: 2 },
                    I::Unreachable,
                    disposition.clone(),
                ],
                "ordinary control flow",
            ),
        ] {
            let error = deserialize_module_validated(&wire_body(&body))
                .unwrap_err()
                .to_string();
            assert!(error.contains(wanted), "{error}");
        }
    }
    let invalid = wire_body(&[
        I::TryBegin { handler_offset: 2 },
        I::Unreachable,
        I::DiscardException { dst: Reg(2) },
        I::RetV,
    ]);
    let error = deserialize_module_validated(&invalid)
        .unwrap_err()
        .to_string();
    assert!(error.contains("register r2 out of bounds"), "{error}");

    let mut legacy = wire_body(&[
        I::LoadI {
            dst: Reg(0),
            value: 73,
        },
        I::TryBegin { handler_offset: 2 },
        I::Throw { error: Reg(0) },
        I::GetException { dst: Reg(1) },
        I::Ret { value: Reg(1) },
    ]);
    legacy.as_mut_slice()[6..8].copy_from_slice(&64u16.to_le_bytes());
    let decoded = deserialize_module_validated(&legacy).unwrap();
    let mut vm = Interpreter::try_new_validated(decoded.into()).unwrap();
    assert_eq!(
        vm.execute_function(verum_vbc::FunctionId(0))
            .unwrap()
            .try_as_i64(),
        Some(73)
    );
}

#[test]
fn a_raw_throw_cannot_issue_ownership_for_a_matching_caller_owned_record() {
    use verum_vbc::{FunctionId, Instruction as I, TypeId, Value, instruction::Reg};
    for final_handler in [
        I::DiscardException { dst: Reg(1) },
        I::GetException { dst: Reg(1) },
    ] {
        let wire = wire_body(&[
            I::TryBegin { handler_offset: 4 },
            I::TryBegin { handler_offset: 2 },
            I::Throw { error: Reg(0) },
            I::RethrowException,
            final_handler.clone(),
            I::Ret { value: Reg(1) },
        ]);
        let mut vm =
            Interpreter::try_new_validated(deserialize_module_validated(&wire).unwrap().into())
                .unwrap();
        let record = vm
            .state
            .heap
            .alloc(TypeId::TUPLE, std::mem::size_of::<Value>())
            .unwrap();
        // SAFETY: this test owns the full, live one-slot allocation.
        unsafe { record.data_ptr().cast::<Value>().write(Value::from_i64(73)) };
        let foreign = Value::from_ptr(record.as_ptr());
        let value = vm
            .execute_function_with_args(FunctionId(0), &[foreign])
            .unwrap();
        assert_eq!(
            value,
            if matches!(final_handler, I::GetException { .. }) {
                foreign
            } else {
                Value::unit()
            }
        );
        assert!(vm.state.heap.contains(record.as_ptr()));
        assert_eq!(vm.state.heap.stats().total_frees, 0);
        // SAFETY: the caller retained its original allocation throughout both handlers.
        unsafe { vm.state.heap.free(record) };
    }
}

#[test]
fn uncaught_rethrow_keeps_the_original_message_and_retires_the_pending_owner() {
    use verum_vbc::{FunctionId, Instruction as I, interpreter::InterpreterError};
    let wire = wire_body(&[
        I::TryBegin { handler_offset: 2 },
        I::Unreachable,
        I::RethrowException,
    ]);
    let module = deserialize_module_validated(&wire).unwrap();
    let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
    let error = vm.execute_function(FunctionId(0)).unwrap_err();
    assert!(
        matches!(error, InterpreterError::Panic { ref message } if message == "entered unreachable code"),
        "{error}"
    );
    let stats = vm.state.heap.stats();
    assert_eq!((stats.total_allocs, stats.total_frees), (2, 2));
}

#[test]
fn a_recovered_owned_value_can_escape_the_handler() {
    check(
        r#"
        mount core.base.protocols.Drop;
        type Count is { value: Int };
        type Payload is { count: &mut Count, value: Int };
        implement Drop for Payload {
            fn drop(&mut self) { self.count.value = self.count.value + 1; }
        }
        fn probe()->Int {
            let mut count = Count { value: 0 };
            {
                let recovered: Payload = try {
                    throw Payload { count: &mut count, value: 73 }
                } recover |payload| { payload };
                if count.value != 0 { return 1; }
                if recovered.value != 73 { return 2; }
            }
            if count.value == 1 { 73 } else { 3 }
        }
    "#,
    );
}
