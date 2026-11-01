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
fn local_values_are_neither_assumptions_nor_equations_about_free_parameters() {
    for source in [
        "theorem t(): false proof { let x = false; show false by assumption; }",
        "theorem t(): false proof { let x: Bool = false; show false by assumption; }",
        "theorem t(x:Bool): x == false proof by { let x = false; assumption };",
        "theorem t(x:Bool): x == false proof by { let x: Bool = false; assumption };",
    ] {
        let result = verify(source);
        let ProofVerificationResult::Failed { unproved, .. } = result else {
            panic!("unsupported local binding verified: {source}\n{result:?}");
        };
        assert!(format!("{unproved:?}").contains("checked scoped binding semantics"), "{source}: {unproved:?}");
    }
    assert!(matches!(verify("theorem t(x:Bool) requires x == false : x == false proof by assumption;"),
        ProofVerificationResult::Verified(_)));
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

#[test]
fn written_branch_and_term_evidence_cannot_be_replaced_by_automatic_success() {
    for source in [
        "theorem t(b:Bool): true proof by cases on b { case true => fail(\"invalid evidence\"), case false => trivial };",
        "theorem t(b:Bool): true proof by cases on b { case true => { fail(\"invalid evidence\"); } case false => {} };",
        "theorem t(n:Int): true proof by induction on n { case _ => { have invented: false; } case _ => {} case _ => {} };",
        "theorem t(): true proof { fail(\"invalid evidence\"); }",
        "theorem t(): true proof { exact(missing); }",
        "theorem t(): true proof = missing;",
        "theorem t(): true proof by custom<Bool>();",
        "theorem t(): true proof by simp at absent;",
    ] {
        let result = verify(source);
        assert!(matches!(result, ProofVerificationResult::Failed { .. }), "{source}: {result:?}");
    }
}

#[test]
fn case_coverage_and_each_actual_conclusion_are_obligations() {
    for source in [
        "theorem t(b:Bool): b proof by cases on b { case true => {} case false => {} };",
        "theorem t(b:Bool): false proof by cases { case b == true => {} case b == false => {} };",
        "theorem t(b:Bool): true proof by cases { case b == true => trivial };",
        "theorem t(b:Bool): true proof by cases on b { case true where false => {} case false => {} };",
        "theorem t(b:Bool): true proof by cases { case b == true where false => {} case b == false => {} };",
        "theorem t(b:Bool): false proof by cases { case b && !b => {} };",
        "theorem t(): true proof by cases {};",
        "theorem t(b:Bool): true proof by cases on b { case unused => trivial };",
        "theorem t(n:Int): true proof by induction on n { case _ => {} };",
        "theorem t(n:Int): true proof by induction on n { case _ => {} case _ => {} case _ => {} case _ => {} };",
    ] {
        let result = verify(source);
        assert!(matches!(result, ProofVerificationResult::Failed { .. }), "{source}: {result:?}");
    }
}

#[test]
fn guarded_elimination_retains_valid_boolean_and_arithmetic_proofs() {
    for source in [
        "theorem t(b:Bool): b || !b proof by cases on b { case true => trivial, case false => trivial };",
        "theorem t(b:Bool): b || !b proof { cases b { case true => { show b by auto; } case false => { show !b by auto; } } }",
        "theorem t(n:Int): n >= 0 || n < 0 proof by cases { case n >= 0 => { show n >= 0 by assumption; } case n < 0 => { show n < 0 by assumption; } };",
        "theorem t(n:Int) requires n == 0 || n == 1 : n >= 0 proof by cases on n { case 0 => auto, case 1 => auto };",
        "theorem t(b:Bool): true proof by cases { case b == true => trivial, case b == false => trivial, case _ => trivial };",
        "theorem t(): true proof { show true by trivial; trivial; }",
        "theorem t(): true proof by exact(true);",
        "theorem t(): true proof by contradiction { show false by auto; };",
        "theorem nonnegative_double(n:Int) requires n >= 0 : n * 2 >= 0 { proof { have lower_bound: n >= 0 by assumption; show n * 2 >= 0 by omega; } }",
        "theorem nonnegative_double(n:Int) requires n >= 0 : n * 2 >= 0 proof by induction on n { case _ => auto, case _ => auto, case _ => auto };",
        "theorem nonnegative_double(n:Int) requires n >= 0 : n * 2 >= 0 proof by { induction n; all_goals { auto } };",
    ] {
        let result = verify(source);
        let ProofVerificationResult::Verified(summary) = result else { panic!("{source}: {result:?}") };
        assert!(!summary.has_incomplete_steps(), "{source}: {summary:?}");
    }
}

#[test]
fn contradiction_cannot_discard_an_unproved_false_goal() {
    for source in [
        "theorem t(): true => false proof by contradiction {};",
        "theorem t(b:Bool): b proof by contradiction { trivial; };",
        "theorem t(): true proof by contradiction { fail(\"invalid evidence\"); };",
    ] {
        let result = verify(source);
        assert!(matches!(result, ProofVerificationResult::Failed { .. }), "{source}: {result:?}");
    }
}

#[test]
fn integer_induction_requires_negative_and_positive_obligations() {
    for source in [
        "theorem t(n:Int): n >= 0 proof by induction on n { case _ => {} case _ => {} case _ => {} };",
        "theorem t(n:Int): n >= 0 proof by { induction n; auto };",
        "theorem t(n:Int): true proof by induction on n { case _ => {} case _ => {} };",
        "theorem t(n:Int): true proof by strong_induction on n { case _ => trivial };",
        "theorem t(n:Int): true proof by well_founded_induction on n ordered_by invalid { case _ => trivial };",
    ] {
        let result = verify(source);
        assert!(matches!(result, ProofVerificationResult::Failed { .. }), "{source}: {result:?}");
    }
    let source = "theorem t(n:Int) requires n >= 0 : n * 2 >= 0 proof by induction on n { case _ => {} case _ => { have ih: n >= 0 => n * 2 >= 0 by assumption; } case _ => {} };";
    let result = verify(source);
    assert!(matches!(result, ProofVerificationResult::Verified(_)), "{result:?}");
}
