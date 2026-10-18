//! T1769: a well-typed replacement goal is not evidence for a source judgment.
use verum_ast::decl::{ProofBody, TacticExpr, TheoremDecl};
use verum_ast::{Expr, ExprKind, Ident, Literal, Path, PathSegment};
use verum_common::{Heap, List, Maybe, Span};
use verum_kernel::proof_checker::Term;
use verum_kernel::tactic_elaborator::{ElabContext, ElabError, elaborate_theorem};
fn path(name: &str) -> Expr {
    let span = Span::dummy();
    let mut segments = List::new();
    segments.push(PathSegment::Name(Ident {
        name: name.into(),
        span,
    }));
    Expr::new(ExprKind::Path(Path::new(segments, span)), span)
}
fn theorem(prop: Expr) -> TheoremDecl {
    let span = Span::dummy();
    let mut theorem = TheoremDecl::new(
        Ident {
            name: "source_obligation".into(),
            span,
        },
        prop,
        span,
    );
    theorem.proof = Maybe::Some(ProofBody::Tactic(TacticExpr::Apply {
        lemma: Heap::new(path("witness")),
        args: List::new(),
    }));
    theorem
}
#[test]
fn boolean_truth_and_falsity_cannot_borrow_a_universe_witness() {
    for value in [false, true] {
        let span = Span::dummy();
        let mut ctx = ElabContext::new();
        ctx.register_axiom("witness", Term::universe(0));
        let source = theorem(Expr::new(
            ExprKind::Literal(Literal::bool(value, span)),
            span,
        ));
        assert!(matches!(
            elaborate_theorem(&source, &mut ctx),
            Err(ElabError::UnsupportedExpression(_))
        ));
    }
}
#[test]
fn failed_proposition_translation_never_emits_a_placeholder_certificate() {
    let mut ctx = ElabContext::new();
    ctx.register_axiom("witness", Term::universe(0));
    let source = theorem(Expr::new(
        ExprKind::Binary {
            op: verum_ast::BinOp::Add,
            left: Heap::new(path("a")),
            right: Heap::new(path("b")),
        },
        Span::dummy(),
    ));
    assert!(matches!(
        elaborate_theorem(&source, &mut ctx),
        Err(ElabError::UnsupportedExpression(_))
    ));
}
#[test]
fn native_identity_source_is_checked_and_unequal_endpoints_are_rejected() {
    for right in ["a", "b"] {
        let mut source = theorem(Expr::new(
            ExprKind::Binary {
                op: verum_ast::BinOp::Eq,
                left: Heap::new(path("a")),
                right: Heap::new(path(right)),
            },
            Span::dummy(),
        ));
        source.proof = Maybe::Some(ProofBody::Tactic(TacticExpr::Reflexivity));
        let result = elaborate_theorem(&source, &mut ElabContext::new());
        if right == "a" {
            let cert = result.unwrap();
            cert.verify().unwrap();
            verum_kernel::proof_checker_nbe::verify_certificate(&cert).unwrap();
            assert_eq!(
                cert.metadata.get("claimed_type_source").map(|s| s.as_str()),
                Some("verification_goal")
            );
        } else {
            assert!(matches!(result, Err(ElabError::KernelRejection(_))));
        }
    }
}

#[test]
fn generic_and_value_parameter_types_cannot_be_replaced_by_a_universe() {
    use verum_ast::decl::{FunctionParam, FunctionParamKind};
    use verum_ast::pattern::Pattern;
    use verum_ast::ty::{GenericParam, GenericParamKind, Type, TypeKind};
    let span = Span::dummy();
    for generic in [true, false] {
        let mut source = theorem(path("witness"));
        if generic {
            source.generics.push(GenericParam {
                kind: GenericParamKind::Type {
                    name: Ident {
                        name: "A".into(),
                        span,
                    },
                    bounds: List::new(),
                    default: Maybe::None,
                },
                is_implicit: false,
                span,
            });
        } else {
            source.params.push(FunctionParam::new(
                FunctionParamKind::Regular {
                    pattern: Pattern::ident(
                        Ident {
                            name: "x".into(),
                            span,
                        },
                        false,
                        span,
                    ),
                    ty: Type {
                        kind: TypeKind::Int,
                        span,
                    },
                    default_value: Maybe::None,
                },
                span,
            ));
        }
        let mut ctx = ElabContext::new();
        ctx.register_axiom("witness", Term::universe(0));
        assert!(matches!(
            elaborate_theorem(&source, &mut ctx),
            Err(ElabError::UnsupportedExpression(_))
        ));
    }
}

#[test]
fn source_requires_cannot_capture_ambient_proposition_references() {
    let mut source = theorem(path("P"));
    source.requires.push(path("P"));
    let mut ctx = ElabContext::new();
    ctx.register_axiom("P", Term::universe(0));
    assert!(matches!(
        elaborate_theorem(&source, &mut ctx),
        Err(ElabError::UnsupportedExpression(_))
    ));
}

#[test]
fn explicitly_scoped_pi_goal_preserves_outer_type_and_rejects_capture() {
    use verum_kernel::proof_checker::Certificate;
    use verum_kernel::verification_goal::{GoalSource, VerificationGoal};
    // In ambient A:U0, h:A adds one binder. The conclusion A is then Var(1).
    let goal = VerificationGoal::new(
        vec![Term::var(0)],
        Term::var(1),
        GoalSource::Theorem {
            name: "scoped_identity".into(),
        },
    );
    let term = Term::lam(Term::universe(0), Term::lam(Term::var(0), Term::var(0)));
    let cert = Certificate {
        term: term.clone(),
        claimed_type: Term::pi(Term::universe(0), goal.to_term()),
        metadata: Default::default(),
    };
    cert.verify().unwrap();
    verum_kernel::proof_checker_nbe::verify_certificate(&cert).unwrap();
    let captured = VerificationGoal::new(vec![Term::var(0)], Term::var(0), goal.source.clone());
    let bad = Certificate {
        term,
        claimed_type: Term::pi(Term::universe(0), captured.to_term()),
        metadata: Default::default(),
    };
    assert!(bad.verify().is_err());
    assert!(verum_kernel::proof_checker_nbe::verify_certificate(&bad).is_err());
}
