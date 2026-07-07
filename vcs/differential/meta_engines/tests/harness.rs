//! META-EXEC-CONVERGENCE-1 harness as a test suite.
//!

//! One test drives the full fixture corpus (agreement surface + pins) —
//! engine runs happen inside `with_quiet_panics`, and assertions fire only
//! after the hook is restored, so failures report normally. Unit tests for
//! the vendored comparator and the extractor domain sit alongside.

use meta_engines::{
    all_fixtures, compare_outcomes, run_all, with_quiet_panics, Comparable, EngineOutcome, Status,
};

/// The whole corpus: every agreement fixture agrees, every pin holds, and
/// nothing lands in NEW-diverge.
#[test]
fn corpus_agreement_holds_and_pins_hold() {
    let (reports, totals) = with_quiet_panics(|| {
        let fixtures = all_fixtures();
        run_all(&fixtures)
    });

    let mut failures = Vec::new();
    for report in &reports {
        if report.status == Status::NewDiverge {
            failures.push(format!(
                "[{}] {} — verdict {:?}: {}",
                report.status.label(),
                report.name,
                report.verdict.as_ref().map(|v| v.to_string()),
                report.explanation
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "NEW divergences ({} of {} fixtures):\n{}",
        failures.len(),
        totals.total(),
        failures.join("\n")
    );
    // The corpus must actually exercise both classes.
    assert!(totals.agree > 0, "agreement surface is empty");
    assert!(totals.known_diverge > 0, "no pinned divergence ran");
    assert_eq!(totals.total(), reports.len());
}

/// The i128-overflow pin exists to OBSERVE an engine panic as a divergence:
/// the harness must survive it (this test completing at all is the point).
#[test]
fn engine_panic_is_observed_not_fatal() {
    let (reports, _totals) = with_quiet_panics(|| {
        let fixtures: Vec<_> = all_fixtures()
            .into_iter()
            .filter(|f| f.name == "pin_i128_overflow_panic")
            .collect();
        assert_eq!(fixtures.len(), 1);
        run_all(&fixtures)
    });
    let report = &reports[0];
    // Regardless of build profile the fixture must NOT be a new divergence:
    // overflow-checks ON -> pinned Value/Panic outcome-mismatch holds;
    // OFF -> both engines wrap to 0 and agree.
    assert_ne!(
        report.status,
        Status::NewDiverge,
        "i128-overflow fixture unexpected: {:?} — {}",
        report.verdict,
        report.explanation
    );
}

// ===================== comparator unit tests =====================

#[test]
fn comparator_float_epsilon() {
    use meta_engines::compare::float_eq;
    assert!(float_eq(1.0, 1.0));
    assert!(float_eq(1.0, 1.0 + 1e-12));
    assert!(!float_eq(1.0, 1.0 + 1e-6));
    // Relative: large magnitudes tolerate proportionally larger deltas.
    assert!(float_eq(1e12, 1e12 + 1.0));
    assert!(!float_eq(1e12, 1e12 + 1e6));
    // NaN agrees with NaN (semantics, not bit payloads).
    assert!(float_eq(f64::NAN, f64::NAN));
    assert!(!float_eq(f64::NAN, 1.0));
    // Infinities: same sign agrees, mixed does not.
    assert!(float_eq(f64::INFINITY, f64::INFINITY));
    assert!(!float_eq(f64::INFINITY, f64::NEG_INFINITY));
    assert!(!float_eq(f64::INFINITY, 1.0));
}

#[test]
fn comparator_opaque_never_agrees() {
    let opaque = EngineOutcome::Value(Comparable::Opaque("vbc-list"));
    let opaque2 = EngineOutcome::Value(Comparable::Opaque("tree-array"));
    let int = EngineOutcome::Value(Comparable::Int(3));

    let both = compare_outcomes(&opaque, &opaque2);
    assert!(
        matches!(both, meta_engines::Verdict::OpaqueBoth { .. }),
        "two opaques must be OpaqueBoth, got {both}"
    );
    let mixed = compare_outcomes(&opaque, &int);
    assert!(
        matches!(mixed, meta_engines::Verdict::TypeMismatch { .. }),
        "opaque vs scalar must be TypeMismatch, got {mixed}"
    );
}

#[test]
fn comparator_shapes() {
    use meta_engines::Verdict;
    let val = EngineOutcome::Value(Comparable::Int(1));
    let val2 = EngineOutcome::Value(Comparable::Int(1));
    let other = EngineOutcome::Value(Comparable::Int(2));
    let float = EngineOutcome::Value(Comparable::Float(1.0));
    let err = EngineOutcome::Error("boom".into());
    let err2 = EngineOutcome::Error("different wording".into());
    let panic = EngineOutcome::Panic("overflow".into());

    assert!(compare_outcomes(&val, &val2).is_agree());
    assert!(matches!(
        compare_outcomes(&val, &other),
        Verdict::ValueMismatch { .. }
    ));
    assert!(matches!(
        compare_outcomes(&val, &float),
        Verdict::TypeMismatch { .. }
    ));
    // Cross-engine error taxonomies are disjoint: shape-level agreement.
    assert!(compare_outcomes(&err, &err2).is_agree());
    assert!(matches!(
        compare_outcomes(&val, &err),
        Verdict::OutcomeMismatch { .. }
    ));
    // A panic is never agreement — even against another panic.
    assert!(matches!(
        compare_outcomes(&panic, &panic.clone()),
        Verdict::BothPanicked { .. }
    ));
    assert!(matches!(
        compare_outcomes(&val, &panic),
        Verdict::OutcomeMismatch { .. }
    ));
}

// ===================== extractor unit tests =====================

#[test]
fn extractor_tree_walk_domain() {
    use meta_engines::extractor::from_tree_walk;
    use verum_ast::MetaValue;
    use verum_common::{List, Text};

    assert_eq!(from_tree_walk(&MetaValue::Unit), Comparable::Unit);
    assert_eq!(from_tree_walk(&MetaValue::Bool(true)), Comparable::Bool(true));
    assert_eq!(from_tree_walk(&MetaValue::Int(-7)), Comparable::Int(-7));
    // UInt folds into the Int domain when lossless…
    assert_eq!(from_tree_walk(&MetaValue::UInt(5)), Comparable::Int(5));
    // …and refuses (Opaque) when it cannot.
    assert_eq!(
        from_tree_walk(&MetaValue::UInt(u128::MAX)),
        Comparable::Opaque("tree-uint-out-of-i128-range")
    );
    assert_eq!(from_tree_walk(&MetaValue::Char('A')), Comparable::Char('A'));
    assert_eq!(
        from_tree_walk(&MetaValue::Text(Text::from("hi"))),
        Comparable::Text("hi".to_string())
    );
    // Collections are opaque by contract.
    assert_eq!(
        from_tree_walk(&MetaValue::Array(List::new())),
        Comparable::Opaque("tree-array")
    );
}
