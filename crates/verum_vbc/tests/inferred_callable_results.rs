//! A callable result is established by all of its source exits, never by the
//! default ABI return type or one successful branch. T2854.
#![cfg(feature = "codegen")]

use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    Value, VbcModule,
    codegen::VbcCodegen,
    interpreter::Interpreter,
    types::{TypeId, TypeRef},
};

fn modules(source: &str) -> [VbcModule; 2] {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("source codegen");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    [module, wire]
}

fn signature(module: &VbcModule, name: &str, result: Option<TypeId>) {
    let function = module
        .get_function(module.find_function_by_name(name).unwrap())
        .unwrap();
    assert_eq!(
        function.semantic_return,
        result.map(TypeRef::Concrete),
        "{name}"
    );
    if let Some(result) = result {
        assert_eq!(
            function.return_type,
            TypeRef::Concrete(result),
            "executable result {name}"
        );
        assert!(
            matches!(function.semantic_callable_signature(), Some(TypeRef::Function { return_type, .. })
            if *return_type == TypeRef::Concrete(result)),
            "callable signature {name}"
        );
    } else {
        assert!(
            function.semantic_callable_signature().is_none(),
            "unknown result became a callable contract"
        );
    }
}

fn run(module: VbcModule, flag: bool) -> Result<i64, verum_vbc::interpreter::InterpreterError> {
    let entry = module.find_function_by_name("probe").unwrap();
    Interpreter::try_new_validated(module.into())
        .unwrap()
        .execute_function_with_args(entry, &[Value::from_bool(flag)])
        .map(|result| result.try_as_i64().expect("Int result"))
}

fn callback_source(body: &str, result: &str) -> Text {
    format!(
        r#"
        fn candidate(flag: Bool) {{ {body} }}
        fn accept(callback: fn(Bool) -> {result}, flag: Bool) -> Int {{ callback(flag); 73 }}
        fn probe(flag: Bool) -> Int {{ accept(candidate, flag) }}
    "#
    )
    .into()
}

#[test]
fn inlined_intrinsic_results_keep_their_source_type_at_indirect_calls() {
    let source = r#"
        @intrinsic("ctpop") fn count(value: Int) -> Int;
        fn plus_one(value: Int) -> Int { value + 1 }
        fn probe() -> Int {
            let bits = count(15);
            let callback = plus_one;
            callback(bits) + callback(bits)
        }
    "#;
    for module in modules(source) {
        let entry = module.find_function_by_name("probe").unwrap();
        let result = Interpreter::try_new_validated(module.into()).unwrap()
            .execute_function(entry).unwrap();
        assert_eq!(result.try_as_i64(), Some(10));
    }
}

#[test]
fn nonempty_unit_callbacks_preserve_their_complete_contract() {
    for body in [
        "let mut count = 0; count = count + 1;",
        "unsafe { let mut count = 0; count = count + 1; }",
        "if flag { return; } let count = 4;",
        "if flag { return (); } ()",
        "if flag { let value = 7; } else { let other = 9; }",
        "fn nested() { return 17; } let value = nested();",
        "let nested = || { return 17; }; let value = nested();",
        "defer { let mut value = 7; value = value + 1; }; let other = 9;",
    ] {
        for module in modules(&callback_source(body, "()")) {
            signature(&module, "candidate", Some(TypeId::UNIT));
            for flag in [false, true] {
                assert_eq!(run(module.clone(), flag).unwrap(), 73, "{body}");
            }
        }
    }
}

#[test]
fn inferred_values_keep_their_actual_result_and_executable_carrier() {
    for body in [
        "73",
        "if flag { return 21; } 73",
        "if flag { 21 } else { 73 }",
    ] {
        let source = format!(
            r#"
            fn candidate(flag: Bool) {{ {body} }}
            fn accept(callback: fn(Bool) -> Int, flag: Bool) -> Int {{ callback(flag) }}
            fn probe(flag: Bool) -> Int {{ accept(candidate, flag) }}
        "#
        );
        for module in modules(&source) {
            signature(&module, "candidate", Some(TypeId::INT));
            assert_eq!(run(module.clone(), false).unwrap(), 73, "{body}");
            assert_eq!(
                run(module, true).unwrap(),
                if body == "73" { 73 } else { 21 },
                "{body}"
            );
        }
    }
}

#[test]
fn a_value_returning_function_cannot_claim_a_unit_callback_contract() {
    for module in modules(&callback_source("73", "()")) {
        signature(&module, "candidate", Some(TypeId::INT));
        let error = run(module, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the parameter signature"),
            "{error}"
        );
    }
}

#[test]
fn disagreement_between_explicit_returns_and_fallthrough_is_not_erased() {
    for body in [
        "if flag { return 31; } let value = 4;",
        "if flag { return 31; } true",
        "if flag { 31 } else { true }",
        "let value = if flag { return 31; } else { 4 };",
    ] {
        for module in modules(&callback_source(body, "()")) {
            signature(&module, "candidate", None);
            let error = run(module, false).unwrap_err();
            assert!(
                error.to_string().contains("lacks its semantic signature"),
                "{body}: {error}"
            );
        }
    }
}

#[test]
fn a_forward_result_remains_unknown_instead_of_inheriting_the_unit_abi_default() {
    let source = format!(
        "fn pending(flag: Bool);\n{}",
        callback_source("pending(flag)", "()")
    );
    for module in modules(&source) {
        signature(&module, "candidate", None);
        let caller = module.find_function_by_name("candidate").unwrap();
        let callee = module.find_function_by_name("pending").unwrap();
        let error = run(module, false).unwrap_err();
        // An unresolved forward also lacks the owned-call inventory. Either
        // boundary may refuse first; neither admits a semantic unit result.
        match error {
            verum_vbc::interpreter::InterpreterError::InvalidOperand { message } => assert!(
                message.contains("lacks its semantic signature"),
                "{message}"
            ),
            verum_vbc::interpreter::InterpreterError::InvalidBytecode { message, .. } => {
                assert_eq!(message, format!(
                    "incompatible owned parameter calling conventions: candidate [#{}] calls pending [#{}]",
                    caller.0, callee.0
                ))
            }
            other => panic!("unexpected forward refusal: {other}"),
        }
    }
}

#[test]
fn deferred_return_instructions_cannot_borrow_another_exits_evidence() {
    for body in ["defer { return 31; };", "defer { return (); }; 31"] {
        for module in modules(&callback_source(body, "()")) {
            signature(&module, "candidate", None);
            let error = run(module, false).unwrap_err();
            assert!(
                error.to_string().contains("lacks its semantic signature"),
                "{body}: {error}"
            );
        }
    }
}

#[test]
fn eager_async_exits_preserve_the_declared_computation_and_result() {
    for (body, result) in [("", TypeId::UNIT), ("let value = 1;", TypeId::UNIT),
        ("if flag { return 21; } 73", TypeId::INT)] {
        let result_name = if result == TypeId::UNIT { "()" } else { "Int" };
        let source = callback_source(body, result_name)
            .replace("fn candidate", "async fn candidate")
            .replace("callback: fn(", "callback: async fn(");
        for module in modules(&source) {
            signature(&module, "candidate", Some(result));
            let candidate = module.get_function(module.find_function_by_name("candidate").unwrap()).unwrap();
            assert!(matches!(candidate.semantic_callable_signature(), Some(TypeRef::Function { is_async: true, .. })));
            assert_eq!(run(module.clone(), false).unwrap(), 73);
            assert_eq!(run(module, true).unwrap(), 73);
        }
    }
}

#[test]
fn eager_async_inference_cannot_hide_disagreement_or_a_sync_only_contract() {
    let source = callback_source("73", "Int").replace("fn candidate", "async fn candidate");
    for module in modules(&source) {
        signature(&module, "candidate", Some(TypeId::INT));
        assert!(run(module, false).unwrap_err().to_string().contains("does not match the parameter signature"));
    }
    let source = callback_source("if flag { return 31; } true", "Int")
        .replace("fn candidate", "async fn candidate")
        .replace("callback: fn(", "callback: async fn(");
    for module in modules(&source) {
        signature(&module, "candidate", None);
        assert!(run(module, false).unwrap_err().to_string().contains("lacks its semantic signature"));
    }
}
