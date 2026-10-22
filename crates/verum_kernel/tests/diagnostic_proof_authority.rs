//! Adversarial separation of positive diagnostics from proof authority (T1774).
use verum_kernel::{
    intrinsic_dispatch::{Evidence as IntrinsicEvidence, IntrinsicValue, dispatch_intrinsic},
    reflection_tower::{build_tower_report, walk_stability_up_to},
    soundness::apply_graph::{ApplyGraph, LeafComposition, SymbolEntry, walk_transitive},
    verdict::{DischargeMethod, Evidence, MultiVerdict, VerificationVerdict},
};

#[test]
fn positive_flags_and_arbitrary_citations_never_certify_a_theorem() {
    for value in [
        IntrinsicValue::Bool(true),
        IntrinsicValue::Bool(false),
        IntrinsicValue::Decision {
            holds: true,
            evidence: IntrinsicEvidence::Computed,
            reason: "numeric predicate".into(),
        },
        IntrinsicValue::Decision {
            holds: true,
            evidence: IntrinsicEvidence::Cited {
                source: "Any publication, page 1".into(),
            },
            reason: "unreplayed assertion".into(),
        },
    ] {
        let verdict = value.proof_verdict("arbitrary_intrinsic");
        assert_eq!(verdict.tag(), "structural_only");
        assert!(!verdict.is_discharged());
    }
    let cited = dispatch_intrinsic("kernel_soundness_v0", &[]).unwrap();
    assert_eq!(cited.as_bool(), Some(true));
    assert!(!cited.proof_verdict("kernel_soundness_v0").is_discharged());
}

#[test]
fn reflection_samples_preserve_observations_without_proving_universal_stability() {
    let report = build_tower_report();
    assert!(report.is_structurally_consistent());
    assert!(report.structurally_positive_stage_count() > 0);
    assert!(report.structurally_positive_sample_count() > 0);
    assert_eq!(report.proof_verdict().tag(), "structural_only");
    assert!(!report.is_load_bearing());
    assert_eq!(report.discharged_count(), 0);
    assert_eq!(report.constructive_discharged_count(), 0);
    for bound in [0, 1, 10, 100] {
        let walk = walk_stability_up_to(bound);
        assert!(walk.is_structurally_stable());
        assert!(!walk.is_load_bearing());
    }
}

#[test]
fn resolved_dependencies_and_empty_graphs_cannot_promote_a_proof() {
    let empty = LeafComposition::default();
    assert!(empty.has_resolved_dependencies());
    assert!(!empty.is_l4_load_bearing());
    let mut graph = ApplyGraph::default();
    graph.insert("kernel_fiction_strict", SymbolEntry::KernelBridge);
    let resolved = walk_transitive(&graph, "kernel_fiction_strict", 16);
    assert!(resolved.has_resolved_dependencies());
    assert!(!resolved.is_l4_load_bearing());
}

#[test]
fn aggregation_does_not_launder_structural_obligations_through_unrelated_success() {
    let structural = IntrinsicValue::Bool(true).proof_verdict("sample");
    let claimed_success = VerificationVerdict::Discharged {
        method: DischargeMethod::KernelChecker {
            name: "other_statement",
        },
        evidence: Evidence::from_summary("unrelated acceptance"),
    };
    let verdicts = MultiVerdict {
        verdicts: vec![structural, claimed_success],
    };
    assert!(!verdicts.aggregate().is_discharged());
    assert_eq!(verdicts.aggregate().tag(), "structural_only");
}

#[test]
fn native_dependent_identity_still_passes_both_actual_checkers() {
    use verum_proof::proof_checker::{Certificate, Level, Term};
    let u = Term::Universe(Level::Concrete(0));
    // (A : U0) -> A -> A, not a numeric or citation witness.
    let claimed_type = Term::Pi(
        Box::new(u.clone()),
        Box::new(Term::Pi(Box::new(Term::Var(0)), Box::new(Term::Var(1)))),
    );
    let term = Term::Lam(
        Box::new(u),
        Box::new(Term::Lam(Box::new(Term::Var(0)), Box::new(Term::Var(0)))),
    );
    let cert = Certificate {
        term,
        claimed_type,
        metadata: Default::default(),
    };
    cert.verify().unwrap();
    verum_proof::proof_checker_nbe::verify_certificate(&cert).unwrap();
}
