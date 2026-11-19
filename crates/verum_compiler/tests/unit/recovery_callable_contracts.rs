//! Recovery policies are checked with the complete target-selected stdlib graph.
use super::*;

#[test]
fn recovery_reset_preserves_existing_payload_origins() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for body in [
        "fn run(mut policy:RecoveryRetryPolicy)->RecoveryRetryPolicy {policy.reset();policy}",
        "fn run(policy:&mut RecoveryRetryPolicy) {policy.reset();}",
        "fn run<T>(policy:&mut RecoveryRetryPolicy,value:T)->T {policy.reset();value}",
        "async fn run(policy:&mut RecoveryRetryPolicy) {policy.reset();}",
    ] {
        let source = format!(
            "module recovery_reset_probe; mount core.runtime.recovery.RecoveryRetryPolicy; {body}"
        );
        let module = Parser::new(&source).parse_module().unwrap();
        let mut checker = phase.configured_checker();
        let result = check_source_module(&mut checker, "recovery_reset_probe", &module, &registry);
        result.unwrap_or_else(|errors| panic!("{body}: {errors:#?}"));
    }
}

#[test]
fn actual_recovery_module_uses_canonical_callable_contracts() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let owner = "core.runtime.recovery";
    let module = registry.read().get_by_path(owner).unwrap().ast.clone();
    let mut checker = phase.configured_checker();
    let result = check_source_types(&mut checker, owner, &module, &registry, |error| {
        error.to_diagnostic_with_span(Some(|span| session.convert_span(span)))
    });
    result.unwrap_or_else(|errors| panic!("{owner}: {errors:#?}"));
}

#[test]
fn recovery_factory_requires_the_selected_future_result() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (factory, accepted) in [
        (
            "fn make()->ReadyFuture<Result<Int,Text>> { ready(Result.Ok(73)) }",
            true,
        ),
        ("fn make()->ReadyFuture<Int> { ready(73) }", false),
        (
            "type Pretend is { value:Int }; fn make()->Pretend { Pretend{value:73} }",
            false,
        ),
    ] {
        for consumer in [
            "async fn run(policy: &mut RecoveryRetryPolicy) -> Result<Int, Text> {
                 execute_with_retry(policy, make).await
             }",
            "async fn run(breaker: &RecoveryCircuitBreaker) -> Result<Int, CircuitBreakerError<Text>> {
                 execute_with_circuit_breaker(breaker, make).await
             }",
        ] {
            let source = format!(r#"
                module recovery_probe;
                mount core.runtime.recovery.{{RecoveryRetryPolicy, RecoveryCircuitBreaker, CircuitBreakerError,
                    execute_with_retry, execute_with_circuit_breaker}};
                mount core.async.future.{{ready, ReadyFuture}};
                mount core.base.result.Result;
                {factory}
                {consumer}
            "#);
            let module = Parser::new(&source).parse_module().unwrap();
            let result = check_source_module(&mut phase.configured_checker(), "recovery_probe", &module, &registry);
            if accepted {
                result.unwrap_or_else(|errors| panic!("{consumer} / {factory}: {errors:#?}"));
            } else {
                let errors = result.expect_err("a factory without the exact Future.Output contract was accepted");
                assert!(errors.iter().any(|error| matches!(error.code(), Some("E400" | "E405"))), "{consumer} / {factory}: {errors:#?}");
            }
        }
    }
}
