use verum_ast::decl::{ProofBody, ProofMethod, ProofStepKind, TacticExpr};
use verum_ast::{FileId, ItemKind};
use verum_common::Maybe;
use verum_fast_parser::FastParser;

fn body(source: &str) -> ProofBody {
    let module = FastParser::new().parse_module_str(source, FileId::new(0)).unwrap();
    let ItemKind::Theorem(theorem) = &module.items[0].kind else { panic!("theorem") };
    let Maybe::Some(proof) = &theorem.proof else { panic!("proof") };
    proof.clone()
}

#[test]
fn unsupported_input_cannot_be_erased_or_reinterpreted() {
    for source in [
        "theorem t() proof by auto using [missing];",
        "theorem t() proof by omega { theory: missing };",
        "theorem t() proof by auto { depth: 0 };",
        "theorem t() proof by blast { depth: 0 };",
        "theorem t() proof by smt { timeout: 0 };",
        "theorem t() proof by arbitrary_method on x {};",
        "theorem t() proof by contrapositive { trivial; };",
        "theorem t() proof by mutual_induction on x, y {};",
        "theorem t() proof by strong_induction on (x, y) {};",
        "theorem t() proof by well_founded_induction on x {};",
        "theorem t() proof { let a, b such that true; }",
        "theorem t() proof { let a such that true; }",
        "theorem t() proof { assume h: false; }",
        "theorem t() proof { take x: Missing; }",
        "theorem t() proof { proof = false; }",
        "theorem t() proof { proof { show false by trivial; } }",
        "theorem t() proof { proof by induction on x { case 0 => { trivial; } } }",
        "theorem t() proof { by contradiction h { trivial; } }",
        "theorem t() proof { by strong_induction on x {} }",
        "theorem t() proof by induction on x { case succ n => { trivial; } }",
        "theorem t() given missing: false proof by trivial;",
        "corollary t() from missing proof by trivial;",
    ] {
        let errors = FastParser::new().parse_module_str(source, FileId::new(0))
            .expect_err("accepted proof input must preserve its meaning");
        assert!(!errors.is_empty(), "{source}");
        assert!(errors[0].span.end as usize <= source.len(), "{source}: {errors:?}");
    }
}

#[test]
fn named_induction_preserves_method_target_relation_and_branch_evidence() {
    let ProofBody::ByMethod(ProofMethod::StrongInduction { on, cases }) = body(
        "theorem t(n:Int) proof by strong_induction on n { case 0 => { exact(seed); } };"
    ) else { panic!("strong induction must not become ordinary induction") };
    assert_eq!(on.name.as_str(), "n");
    assert_eq!(cases.len(), 1);
    assert!(matches!(cases[0].proof[0].kind, ProofStepKind::Tactic(TacticExpr::Exact(_))));
    let source = "theorem t(n:Int) proof by well_founded_induction on n ordered_by strict_order { case 0 => { exact(seed); } };";
    let ProofBody::ByMethod(ProofMethod::WellFoundedInduction { relation, on, cases }) = body(source)
        else { panic!("well-founded induction must retain its relation") };
    assert_eq!(on.name.as_str(), "n");
    assert_eq!(&source[relation.span.start as usize..relation.span.end as usize], "strict_order");
    assert_eq!(cases.len(), 1);
}

#[test]
fn annotated_structured_bindings_retain_the_exact_type() {
    let source = "theorem t() proof { let x: AnnotationOnly<Int> = value; let y = x; exact(y); }";
    let ProofBody::Structured(proof) = body(source) else { panic!("structured proof") };
    let ProofStepKind::Let { ty: Maybe::Some(ty), .. } = &proof.steps[0].kind
        else { panic!("annotation disappeared") };
    assert_eq!(&source[ty.span.start as usize..ty.span.end as usize], "AnnotationOnly<Int>");
    assert!(matches!(proof.steps[1].kind, ProofStepKind::Let { ty: Maybe::None, .. }));
}

#[test]
fn nested_tactic_sequences_and_explicit_arguments_remain_intact() {
    let ProofBody::Structured(proof) = body(
        "theorem t() proof { by induction path; exact(seed); proof by custom<Bool>(seed); }"
    ) else { panic!("structured proof") };
    let ProofStepKind::Tactic(TacticExpr::Seq(tactics)) = &proof.steps[0].kind else { panic!("sequence") };
    assert!(matches!(tactics[0], TacticExpr::InductionOn(_)));
    assert!(matches!(tactics[1], TacticExpr::Exact(_)));
    let ProofStepKind::Tactic(TacticExpr::Named { name, generic_args, args }) = &proof.steps[1].kind
        else { panic!("named tactic") };
    assert_eq!(name.name.as_str(), "custom");
    assert_eq!(generic_args.len(), 1);
    assert_eq!(args.len(), 1);
}

#[test]
fn case_tactics_keep_the_ambient_goal_and_guards_are_conjoined() {
    let source = "theorem t(b:Bool) proof by cases { case b == true where b != false => exact(seed), case _ { trivial; } };";
    let ProofBody::ByMethod(ProofMethod::Cases { cases, .. }) = body(source)
        else { panic!("cases") };
    assert_eq!(cases.len(), 2);
    assert!(matches!(cases[0].proof[0].kind, ProofStepKind::Tactic(TacticExpr::Exact(_))));
    assert!(matches!(cases[0].pattern.kind, verum_ast::PatternKind::Wildcard));
    let Maybe::Some(condition) = &cases[0].condition else { panic!("condition") };
    let verum_ast::ExprKind::Binary { op: verum_ast::BinOp::And, left, right } = &condition.kind
        else { panic!("both written guards must survive") };
    assert_eq!(&source[left.span.start as usize..left.span.end as usize], "b == true");
    assert_eq!(&source[right.span.start as usize..right.span.end as usize], "b != false");
    assert!(matches!(cases[1].proof[0].kind, ProofStepKind::Tactic(TacticExpr::Trivial)));
}
