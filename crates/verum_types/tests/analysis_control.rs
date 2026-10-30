use std::time::{Duration, Instant};
use verum_types::{AnalysisControl, TypeError};

#[test]
fn cancellation_is_shared_and_never_recovers_implicitly() {
    let control = AnalysisControl::default();
    assert!(control.check().is_ok());
    let worker = control.clone();
    std::thread::spawn(move || worker.cancel()).join().unwrap();
    assert!(matches!(control.check(), Err(TypeError::AnalysisCancelled)));
    assert!(control.is_cancelled());
    assert!(AnalysisControl::default().check().is_ok());
}

#[test]
fn expired_deadline_has_no_type_check_verdict() {
    let past = AnalysisControl::with_deadline(Instant::now() - Duration::from_secs(1));
    assert!(matches!(past.check(), Err(TypeError::AnalysisCancelled)));
    let future = AnalysisControl::with_deadline(Instant::now() + Duration::from_secs(60));
    assert!(future.check().is_ok());
    future.cancel();
    assert!(future.is_cancelled());
}

#[test]
fn interrupted_checker_is_not_rearmed_with_partially_populated_state() {
    use verum_ast::FileId;
    use verum_fast_parser::VerumParser;
    use verum_types::TypeChecker;
    let module = VerumParser::new()
        .parse_module_str("fn identity(x: Int) -> Int { x }", FileId::new(1))
        .unwrap();
    let mut checker = TypeChecker::new();
    checker.register_builtins();
    let control = AnalysisControl::default();
    checker.set_analysis_control(control.clone());
    control.cancel();
    assert!(matches!(
        checker.check_item(&module.items[0]),
        Err(TypeError::AnalysisCancelled)
    ));
    checker.set_analysis_control(AnalysisControl::default());
    assert!(matches!(
        checker.check_item(&module.items[0]),
        Err(TypeError::AnalysisCancelled)
    ));
    let mut fresh = TypeChecker::new();
    fresh.register_builtins();
    assert!(fresh.check_item(&module.items[0]).is_ok());
}
