//! Exercise the complete standard-library blocking pool through serialized VBC.
use verum_ast::{ItemKind, Module};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::{Interpreter, InterpreterError},
    module::VbcModule,
};

fn parse(source: &str) -> Module {
    Parser::new(source).parse_module().expect("source grammar")
}

fn compiled(modules: &[&Module]) -> VbcModule {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen.collect_unit_declarations(modules).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(modules, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = codegen.finalize_module_from_state().unwrap();
    assert!(module.resolve_external_bands().is_empty());
    module.synthesize_intrinsic_band_wrappers();
    module.resolve_protocol_dispatch();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    module
}

fn execute(module: VbcModule) -> Result<i64, InterpreterError> {
    let entry = module.find_function_by_name("app.probe").expect("probe");
    let mut vm = Interpreter::new(module.into());
    vm.run_global_ctors()?;
    Ok(vm
        .execute_function(entry)?
        .try_as_i64()
        .expect("integer result"))
}

fn standard_pool() -> (Module, Module) {
    let mut protocols = parse(include_str!("../../../core/base/protocols.vr"));
    protocols.items.retain(|item| {
        matches!(item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Drop")
    });
    assert!(
        protocols
            .items
            .iter()
            .any(|item| item.kind.type_or_protocol_name().is_some())
    );
    let pool = parse(include_str!("../../../core/runtime/pool.vr"));
    (protocols, pool)
}

#[test]
fn public_pool_lifecycle_examples_execute_from_the_actual_test_sources() {
    let (protocols, pool) = standard_pool();
    let tests = parse(include_str!(
        "../../../core-tests/runtime/pool/unit_test.vr"
    ));
    let module = compiled(&[&protocols, &pool, &tests]);
    let mut vm = Interpreter::new(module.into());
    vm.run_global_ctors().unwrap();
    for name in [
        "worker_count_default_executes",
        "completed_task_outlives_its_local_pool",
        "a_job_can_be_discarded_before_another_is_collected",
    ] {
        let entry = vm
            .module()
            .find_function_by_name(&format!("pool_contracts.{name}"))
            .unwrap_or_else(|| panic!("missing registered test {name}"));
        vm.execute_function(entry)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn worker_results_remain_owned_by_the_original_handle() {
    let (protocols, pool) = standard_pool();
    let app = parse(
        r#"
       module app;
       mount core.runtime.pool.ThreadPool;
       fn triple(x: Int) -> Int { x * 3 }
       fn probe() -> Int {
           let pool = ThreadPool.new(2);
           let mut first = pool.submit(triple, 15);
           let initial = first.join();
           let mut second = ThreadPool.global_submit(triple, 9);
           let repeated = first.join();
           initial + repeated + second.join() + 1
       }
    "#,
    );
    assert_eq!(execute(compiled(&[&protocols, &pool, &app])).unwrap(), 73);
}

#[test]
fn exact_registered_identities_work_through_renamed_declarations() {
    let app = parse(
        r#"
       module app;
       @intrinsic("pool_create") fn open(n: Int) -> Int;
       @intrinsic("pool_submit") fn schedule(id: Int, callback: fn(Int) -> Int, value: Int) -> Int;
       @intrinsic("pool_await") fn read(id: Int) -> Int;
       @intrinsic("pool_destroy") fn close(id: Int);
       fn increment(x: Int) -> Int { x + 1 }
       fn probe() -> Int {
           let pool = open(1); let id = schedule(pool, increment, 41);
           close(pool); read(id)
       }
    "#,
    );
    assert_eq!(execute(compiled(&[&app])).unwrap(), 42);
}

#[test]
fn transfer_requires_a_captureless_callable_even_behind_an_indirect_parameter() {
    let (protocols, pool) = standard_pool();
    for (setup, callback, expected) in [
        ("", "double", Some(42)),
        ("let bias = 3;", "|x| x + bias", None),
        (
            "let mut local = 3; let reference = &mut local;",
            "|x| { *reference += x; *reference }",
            None,
        ),
    ] {
        let app = parse(&format!(
            r#"
            module app;
            mount core.runtime.pool.{{ThreadPool, PoolTaskHandle}};
            fn double(x: Int) -> Int {{ x * 2 }}
            fn queue(callback: fn(Int) -> Int) -> PoolTaskHandle {{
                ThreadPool.global_submit(callback, 21)
            }}
            fn submit_from_caller() -> PoolTaskHandle {{
                {setup} let callback = {callback}; queue(callback)
            }}
            fn probe() -> Int {{ let mut handle = submit_from_caller(); handle.join() }}
        "#
        ));
        let result = execute(compiled(&[&protocols, &pool, &app]));
        if let Some(value) = expected {
            assert_eq!(result.unwrap(), value);
        } else {
            assert!(
                matches!(&result, Err(InterpreterError::Panic { message })
                if message == verum_vbc::intrinsics::registry::PoolOperation::CAPTURE_REFUSAL),
                "{setup}: {result:?}"
            );
        }
    }
}

#[test]
fn foreign_identity_and_retired_pool_cannot_run_a_callback() {
    let app = parse(
        r#"
        module app;
        @intrinsic("foreign.pool_global_submit") fn impostor(f: fn(Int) -> Int, n: Int) -> Int;
        fn double(x: Int) -> Int { x * 2 }
        fn probe() -> Int { impostor(double, 17) }
    "#,
    );
    assert!(matches!(execute(compiled(&[&app])),
        Err(InterpreterError::UnsupportedIntrinsic { intrinsic, .. }) if intrinsic.as_str() == "foreign.pool_global_submit"));

    let app = parse(
        r#"
        module app;
        @intrinsic("pool_submit") fn schedule(p: Int, f: fn(Int) -> Int, n: Int) -> Int;
        @intrinsic("pool_create") fn open(n: Int) -> Int;
        @intrinsic("pool_destroy") fn close(n: Int);
        fn double(x: Int) -> Int { x * 2 }
        fn probe() -> Int { let b = open(2); close(b); schedule(b, double, 17) }
    "#,
    );
    assert!(matches!(execute(compiled(&[&app])),
        Err(InterpreterError::Panic { message }) if message.contains("closed or unknown")));
}

#[test]
fn erased_integer_or_wrong_callback_signature_cannot_enter_the_pool() {
    for signature in ["Int", "fn(Bool) -> Int", "fn(Int) -> Bool"] {
        let app = parse(&format!(
            r#"
            module app;
            @intrinsic("pool_global_submit") fn schedule(f: {signature}, n: Int) -> Int;
            fn probe() -> Int {{ schedule(0, 0) }}
        "#
        ));
        let result = execute(compiled(&[&app]));
        assert!(
            matches!(&result, Err(InterpreterError::InvalidBytecode { message, .. })
            if message.contains("incompatible worker pool signature")),
            "{signature}: {result:?}"
        );
    }
}

#[test]
fn invalid_worker_counts_cannot_publish_a_live_pool() {
    for count in ["0", "-1", "9223372036854775807"] {
        let app = parse(&format!(
            r#"
            module app;
            @intrinsic("pool_create") fn open(n: Int) -> Int;
            fn probe() -> Int {{ open({count}) }}
        "#
        ));
        let result = execute(compiled(&[&app]));
        assert!(
            matches!(result, Err(InterpreterError::Panic { message }) if message == "invalid blocking worker count")
        );
    }
}
