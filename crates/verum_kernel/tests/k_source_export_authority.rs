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

fn closed_reference_judgments() -> Vec<(verum_proof::admission::Judgment, bool)> {
    use verum_proof::admission::{Judgment, PROFILE};
    use verum_kernel::proof_checker::Certificate;
    use verum_kernel::tactic_elaborator::{close_over_axioms, resolve_apply_target};
    let mut ctx = ElabContext::new();
    let base = Term::universe(0);
    let identity = Term::pi(base.clone(), base.clone());
    ctx.register_axiom("z_function", identity.clone());
    ctx.register_axiom("a_type", base.clone());
    let mut cases = Vec::new();
    for (name, domain, other) in [
        ("z_function", &identity, &base),
        ("a_type", &base, &identity),
    ] {
        let reference = resolve_apply_target(&ctx, name).unwrap();
        assert_eq!(ctx.synthesize_type(&reference).as_ref(), Some(domain));
        for (ty, accepted) in [(domain, true), (other, false)] {
            let (term, claimed_type) = close_over_axioms(&ctx, reference.clone(), ty.clone());
            let statement = Term::pi(identity.clone(), Term::pi(base.clone(), ty.clone()));
            assert_eq!(claimed_type, statement, "{name} : {ty:?}");
            cases.push((Judgment {
                profile: PROFILE.to_owned(),
                statement,
                certificate: Certificate { term, claimed_type, metadata: Default::default() },
            }, accepted));
        }
    }
    cases
}

#[test]
fn closing_the_context_preserves_the_type_of_each_named_reference() {
    for (judgment, accepted) in closed_reference_judgments() {
        let cert = &judgment.certificate;
        assert_eq!(cert.verify().is_ok(), accepted, "{judgment:?}");
        assert_eq!(verum_kernel::proof_checker_nbe::verify_certificate(cert).is_ok(), accepted);
    }
}

#[test]
#[ignore = "requires an installed Lean kernel"]
fn lean_checks_closed_named_references_against_independent_statements() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let lean = std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into());
    for (judgment, accepted) in closed_reference_judgments() {
        let bytes = serde_json::to_vec(&judgment).unwrap();
        let source = verum_proof::lean_export::source(&bytes).unwrap();
        let mut child = Command::new(&lean).arg("--stdin")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().expect("start independent Lean kernel");
        child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        let log = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.success(), accepted, "{log}\n{}", String::from_utf8_lossy(&output.stderr));
        if accepted {
            assert!(log.contains("does not depend on any axioms"), "{log}");
        } else {
            assert!(log.contains("Type mismatch") || log.contains("type mismatch"), "{log}");
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
