//! Parsed source must retain its declared predicate and actual proposition.
use verum_ast::ItemKind;
use verum_common::span::FileId;
use verum_compiler::phases::proof_verification::{
    ProofEvidence, ProofVerificationResult, verify_proof_body_with_aliases_and_graph,
};
use verum_fast_parser::FastParser;
use verum_smt::Context;
use verum_smt::expr_to_smtlib::{ReflectionTypeEnv, try_reflect_function_with_env};
use verum_smt::proof_search::ProofSearchEngine;
use verum_smt::refinement_reflection::RefinementReflectionRegistry;

fn verify(source: &str) -> ProofVerificationResult {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("valid source");
    let env = ReflectionTypeEnv::from_module(&module);
    let mut reflection = RefinementReflectionRegistry::new();
    let mut engine = ProofSearchEngine::new();
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            let Some(reflected) = try_reflect_function_with_env(function, &env) else {
                continue;
            };
            engine.register_callee_signature(
                reflected.name.clone(),
                reflected.parameter_sorts.clone().into_iter().collect(),
                reflected.return_sort.clone(),
            );
            reflection.register(reflected).expect("unique declaration");
        }
    }
    engine.set_reflection_registry(reflection);
    let theorem = module
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Theorem(theorem) => Some(theorem),
            _ => None,
        })
        .expect("theorem");
    verify_proof_body_with_aliases_and_graph(
        &mut engine,
        &Context::new(),
        theorem,
        &Default::default(),
        None,
        Some(&module),
    )
}

#[test]
fn matching_contracts_cannot_discharge_a_different_proposition() {
    let result = verify(
        r#"
        fn emp() -> Bool { true }
        theorem false_claim() requires emp() ensures emp() : false
            proof by auto;
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Failed { .. }),
        "{result:?}"
    );
}

#[test]
fn user_predicate_named_emp_does_not_acquire_builtin_truth() {
    let result = verify(
        r#"
        fn emp() -> Bool { false }
        theorem false_claim() ensures emp() proof by auto;
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Failed { .. }),
        "{result:?}"
    );
}

#[test]
fn user_predicate_named_points_to_keeps_its_declared_meaning() {
    let result = verify(
        r#"
        fn points_to(address: Int, value: Int) -> Bool { address == value }
        theorem reflexive(n: Int) ensures points_to(n, n) proof by auto;
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Verified(_)),
        "{result:?}"
    );
}

#[test]
fn renamed_predicate_has_the_same_truth_value() {
    let result = verify(
        r#"
        fn user_predicate(value: Bool) -> Bool { false }
        theorem false_claim() ensures user_predicate(true) proof by auto;
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Failed { .. }),
        "{result:?}"
    );
}

#[test]
fn unsupported_source_calls_cannot_discharge_false_propositions() {
    let result = verify(
        r#"
        theorem false_claim() requires points_to(unknown_call(), 0)
            ensures points_to(unknown_call(), 0) : false proof by auto;
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Failed { .. }),
        "{result:?}"
    );
}

#[test]
fn matching_contracts_do_not_hide_a_failing_tactic() {
    let result = verify(
        r#"
        theorem explicit_failure() requires emp() ensures emp()
            proof by fail("deliberate refusal");
    "#,
    );
    assert!(
        matches!(result, ProofVerificationResult::Failed { .. }),
        "{result:?}"
    );
}

#[test]
fn matching_contracts_do_not_erase_an_admitted_proof() {
    let result = verify(
        r#"
        theorem unfinished() requires emp() ensures emp() proof by admit;
    "#,
    );
    let ProofVerificationResult::Verified(summary) = result else {
        panic!("{result:?}");
    };
    assert!(summary.has_incomplete_steps(), "{summary:?}");
}

#[test]
fn proof_summary_preserves_the_goal_and_evidence_kind() {
    let result = verify("theorem reflexive(n: Int) requires n > 0 : n == n proof by auto;");
    let ProofVerificationResult::Verified(summary) = result else {
        panic!("{result:?}");
    };
    assert_eq!(summary.evidence, ProofEvidence::TacticDischarge);
    assert_eq!(summary.goal.hypotheses.len(), 1);
    assert!(matches!(
        summary.goal.goal.kind,
        verum_ast::ExprKind::Binary {
            op: verum_ast::BinOp::Eq,
            ..
        }
    ));
    let result = verify("theorem assumption() : false;");
    let ProofVerificationResult::Verified(summary) = result else {
        panic!("{result:?}");
    };
    assert_eq!(summary.evidence, ProofEvidence::Assumed);
    assert!(!summary.has_incomplete_steps());
}
