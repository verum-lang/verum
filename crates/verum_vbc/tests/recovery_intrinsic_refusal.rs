//! Compatibility refusal for retired opaque recovery declarations, through
//! ordinary source and serialized VBC. Actual typed recovery execution belongs
//! to a separate runtime gate; this tests the removed opaque boundary.
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::{Interpreter, InterpreterError},
    module::VbcModule,
};

fn compile_probe(body: &str) -> VbcModule {
    // A historical declaration is intentionally retained as an input fixture:
    // references to the retired API must fail instead of bypassing its policy.
    let source = format!(
        r#"
        module core.runtime.async_ops;
        type RecoveryContextOpaque is (Int);
        @intrinsic("exec_with_recovery")
        fn exec_with_recovery(ctx: RecoveryContextOpaque, f: fn() -> ()) -> Int;
        {body}
    "#
    );
    let module = Parser::new(&source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("main").with_validation());
    codegen.collect_unit_declarations(&[&module]).unwrap();
    codegen
        .compile_unit_items(&[&module], ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn routes(body: &str) -> [VbcModule; 2] {
    let source = compile_probe(body);
    let wire = verum_vbc::serialize::serialize_module(&source).unwrap();
    [
        source,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ]
}

fn execute(module: VbcModule) -> Result<verum_vbc::value::Value, InterpreterError> {
    let id = module
        .find_function_by_name("core.runtime.async_ops.probe")
        .unwrap();
    Interpreter::new(Arc::new(module)).execute_function(id)
}

fn assert_legacy_refusal(body: &str) {
    for module in routes(body) {
        let result = execute(module);
        assert!(
            matches!(&result, Err(InterpreterError::UnsupportedIntrinsic { declaration, intrinsic })
                if declaration.as_str() == "core.runtime.async_ops.exec_with_recovery"
                    && intrinsic.as_str() == "exec_with_recovery"),
            "the legacy call must refuse before factory execution, got {result:?}"
        );
    }
}

#[test]
fn direct_legacy_declaration_refuses_instead_of_bypassing_policy() {
    assert_legacy_refusal(
        r#"
        fn factory() { panic("recovery factory must not execute without a policy"); }
        fn probe() -> Int { exec_with_recovery(RecoveryContextOpaque(0), factory) }
    "#,
    );
}

#[test]
fn first_class_legacy_declaration_refuses_instead_of_returning_unit() {
    assert_legacy_refusal(
        r#"
        fn factory() { panic("recovery factory must not execute without a policy"); }
        fn probe() -> Int {
            let execute = exec_with_recovery;
            execute(RecoveryContextOpaque(0), factory)
        }
    "#,
    );
}

#[test]
fn removed_raw_intrinsic_traps_before_factory() {
    for module in routes(
        r#"
        fn factory() { panic("recovery factory must not execute without a policy"); }
        fn probe() -> Int { @intrinsic("exec_with_recovery", 0, factory) }
    "#,
    ) {
        let result = execute(module);
        assert!(
            matches!(&result, Err(InterpreterError::Panic { message })
            if message.contains("@intrinsic(\"exec_with_recovery\") is not implemented in this build")),
            "expected the exact removed-intrinsic trap, got {result:?}"
        );
    }
}

#[test]
fn missing_raw_arguments_cannot_return_a_successful_nil() {
    for module in routes("fn probe() -> Int { @intrinsic(\"exec_with_recovery\") }") {
        let result = execute(module);
        assert!(
            matches!(&result, Err(InterpreterError::Panic { message })
            if message.contains("@intrinsic(\"exec_with_recovery\") is not implemented in this build")),
            "an invalid raw recovery call must not return nil, got {result:?}"
        );
    }
}

#[test]
fn ordinary_function_value_remains_callable() {
    for module in routes("fn factory() -> Int { 17 } fn probe() -> Int { let f=factory; f() }") {
        assert_eq!(execute(module).unwrap().try_as_i64(), Some(17));
    }
}

#[test]
fn first_class_intrinsic_uses_declared_identity_instead_of_source_alias() {
    for module in routes(
        r#"
        @intrinsic("sqrt") fn root_alias(x: Float) -> Float;
        fn probe() -> Float { let operation = root_alias; operation(9.0) }
    "#,
    ) {
        assert_eq!(execute(module).unwrap().as_f64(), 3.0);
    }
}

#[test]
fn actual_empty_source_body_is_not_an_intrinsic_stub() {
    for module in routes(
        r#"
        @intrinsic("unknown_source_annotation") fn source_unit() {}
        fn probe() { let operation = source_unit; operation(); }
    "#,
    ) {
        assert!(execute(module).unwrap().is_unit());
    }
}

#[test]
fn unsupported_intrinsic_uses_attribute_identity_instead_of_source_alias() {
    for module in routes(
        r#"
        @intrinsic("missing_operation") fn renamed() -> Int;
        fn probe() -> Int { let operation = renamed; operation() }
    "#,
    ) {
        let result = execute(module);
        assert!(
            matches!(&result, Err(InterpreterError::UnsupportedIntrinsic { declaration, intrinsic })
            if declaration.as_str() == "core.runtime.async_ops.renamed"
                && intrinsic.as_str() == "missing_operation"),
            "{result:?}"
        );
    }
}

#[test]
fn unknown_qualified_identity_cannot_borrow_a_math_suffix() {
    for key in ["foreign.sqrt", "llvm.vendor.sqrt"] {
        for module in routes(&format!(
            r#"
            @intrinsic("{key}") fn root_alias(x: Float) -> Float;
            fn probe() -> Float {{ let operation = root_alias; operation(9.0) }}
        "#
        )) {
            let result = execute(module);
            assert!(
                matches!(&result, Err(InterpreterError::UnsupportedIntrinsic { declaration, intrinsic })
                if declaration.as_str() == "core.runtime.async_ops.root_alias"
                    && intrinsic.as_str() == key),
                "{result:?}"
            );
        }
    }
}

#[test]
fn explicit_registered_llvm_alias_remains_supported() {
    for module in routes(
        r#"
        @intrinsic("llvm.sqrt.f64") fn root_alias(x: Float) -> Float;
        fn probe() -> Float { let operation = root_alias; operation(9.0) }
    "#,
    ) {
        assert_eq!(execute(module).unwrap().as_f64(), 3.0);
    }
}

#[test]
fn actual_stdlib_zero_argument_intrinsic_is_a_callable_value() {
    use verum_ast::ItemKind;
    let mut parsed = Parser::new(include_str!("../../../core/intrinsics/runtime/time.vr"))
        .parse_module()
        .expect("actual stdlib source");
    let mut declarations = 0;
    parsed.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Function(function) if function.name.name.as_str() == "num_cpus" => {
            declarations += 1;
            assert!(function.body.is_none());
            true
        }
        _ => false,
    });
    assert_eq!(declarations, 1);
    for entry in [
        "fn probe() -> Int { num_cpus() }",
        "fn probe() -> Int { let operation = num_cpus; operation() }",
    ] {
        let mut unit = parsed.clone();
        unit.items
            .extend(Parser::new(entry).parse_module().unwrap().items);
        let source = VbcCodegen::new()
            .compile_module(&unit)
            .expect("actual stdlib VBC");
        let probe = source
            .functions
            .iter()
            .find(|f| {
                source
                    .strings
                    .get(f.name)
                    .is_some_and(|n| n.ends_with(".probe"))
            })
            .unwrap();
        assert!(
            !probe
                .instructions
                .as_ref()
                .unwrap()
                .iter()
                .any(|i| matches!(i, verum_vbc::instruction::Instruction::LoadI { .. })),
            "runtime CPU count cannot be replaced with the compiler host's count"
        );
        let wire = verum_vbc::serialize::serialize_module(&source).unwrap();
        for module in [
            source,
            verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
        ] {
            let entry = module
                .find_function_by_name("core.intrinsics.runtime.time.probe")
                .unwrap();
            let result = Interpreter::new(Arc::new(module))
                .execute_function(entry)
                .expect("call actual num_cpus declaration");
            assert!(
                result.try_as_i64().is_some_and(|value| value > 0),
                "{result:?}"
            );
        }
    }
}

#[test]
fn marked_ordinary_body_keeps_its_value_for_direct_and_first_class_calls() {
    for body in [
        r#"@intrinsic("num_cpus") fn operation() -> Int { 93284713 }
            fn probe() -> Int { operation() }"#,
        r#"@intrinsic("num_cpus") fn operation() -> Int { 93284713 }
            fn probe() -> Int { let selected = operation; selected() }"#,
    ] {
        for module in routes(body) {
            assert_eq!(execute(module).unwrap().try_as_i64(), Some(93284713));
        }
    }
}

#[test]
fn raw_runtime_intrinsic_requires_a_selected_declaration() {
    let parsed = Parser::new(r#"fn probe() -> Int { @intrinsic("num_cpus") }"#)
        .parse_module()
        .unwrap();
    let error = VbcCodegen::new()
        .compile_module(&parsed)
        .expect_err("unbound raw runtime intrinsic");
    assert!(
        error
            .to_string()
            .contains("runtime intrinsic 'num_cpus' requires a declaration-owned callable"),
        "{error}"
    );
}

#[test]
fn declared_intrinsic_arity_is_checked_for_direct_and_first_class_calls() {
    for entry in [
        "fn probe() -> Int { operation(7) }",
        "fn probe() -> Int { let selected = operation; selected(7) }",
    ] {
        let body = format!(r#"@intrinsic("num_cpus") fn operation(extra: Int) -> Int; {entry}"#);
        for module in routes(&body) {
            let result = execute(module);
            assert!(
                matches!(&result, Err(InterpreterError::InvalidBytecode { message, .. })
                if message == "intrinsic 'num_cpus' contract expects 0 arguments, declaration provides 1"),
                "{result:?}"
            );
        }
    }
}

#[test]
fn declared_intrinsic_value_keeps_its_permission_gate() {
    use verum_vbc::interpreter::{PermissionDecision, PermissionScope};
    for entry in [
        "fn probe() -> Int { operation(37123) }",
        "fn probe() -> Int { let selected = operation; selected(37123) }",
    ] {
        let body = format!(r#"@intrinsic("syscall0") fn operation(number: Int) -> Int; {entry}"#);
        for module in routes(&body) {
            let id = module
                .find_function_by_name("core.runtime.async_ops.probe")
                .unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter.state.set_permission_policy(|scope, target| {
                assert_eq!(scope, PermissionScope::Syscall);
                assert_eq!(target, 37123);
                PermissionDecision::Deny
            });
            let result = interpreter.execute_function(id);
            assert!(
                matches!(&result, Err(InterpreterError::Panic { message })
                if message == "permission denied: Syscall(37123)"),
                "{result:?}"
            );
        }
    }
}

#[test]
fn source_body_owns_inline_strategy_annotations_too() {
    for entry in [
        "fn probe() -> Int { operation(7) }",
        "fn probe() -> Int { let selected = operation; selected(7) }",
    ] {
        let body = format!(
            r#"@intrinsic("popcnt") fn operation(value: Int) -> Int {{ 93284713 }} {entry}"#
        );
        for module in routes(&body) {
            assert_eq!(execute(module).unwrap().try_as_i64(), Some(93284713));
        }
    }
}

#[test]
fn opaque_future_poll_refuses_instead_of_inventing_pending() {
    for entry in [
        "fn probe() -> Bool { operation(7) }",
        "fn probe() -> Bool { let selected = operation; selected(7) }",
    ] {
        let body =
            format!(r#"@intrinsic("future_poll_sync") fn operation(future: Int) -> Bool; {entry}"#);
        for module in routes(&body) {
            let result = execute(module);
            assert!(
                matches!(&result, Err(InterpreterError::UnsupportedIntrinsic { declaration, intrinsic })
                if declaration.as_str() == "core.runtime.async_ops.operation" && intrinsic.as_str() == "future_poll_sync"),
                "{result:?}"
            );
        }
    }
}

#[test]
fn ordinary_future_poll_name_keeps_its_source_body() {
    for entry in [
        "fn probe() -> Int { future_poll_sync(7) }",
        "fn probe() -> Int { let selected = future_poll_sync; selected(7) }",
    ] {
        let body = format!("fn future_poll_sync(future: Int) -> Int {{ future + 31 }} {entry}");
        for module in routes(&body) {
            assert_eq!(execute(module).unwrap().try_as_i64(), Some(38));
        }
    }
}

#[test]
fn removed_raw_future_poll_traps_explicitly() {
    for module in routes(r#"fn probe() -> Bool { @intrinsic("future_poll_sync", 7) }"#) {
        let result = execute(module);
        assert!(
            matches!(&result, Err(InterpreterError::Panic { message })
            if message.contains("@intrinsic(\"future_poll_sync\") is not implemented in this build")),
            "{result:?}"
        );
    }
}
