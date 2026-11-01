use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use verum_ast::decl::{ProofBody, ProofStep, ProofStepKind, ProofStructure, TacticExpr};
use verum_ast::{Expr, ExprKind, Ident, Literal, Path, Span, Type, TypeKind};
use verum_common::{Heap, List, Maybe};
use verum_kernel::soundness::proof_body_translate::{
    AgdaProofBodyRenderer, CoqProofBodyRenderer, DeduktiProofBodyRenderer,
    IsabelleProofBodyRenderer, LeanProofBodyRenderer, ProofBodyRenderer,
};

fn span() -> Span {
    Span::dummy()
}
fn var(name: &str) -> Expr {
    Expr::path(Path::from_ident(Ident::new(name, span())))
}
fn application(argument: i128) -> ProofBody {
    ProofBody::Tactic(TacticExpr::Apply {
        lemma: Heap::new(var("witness")),
        args: List::from_iter([Expr::literal(Literal::int(argument, span()))]),
    })
}
fn renderers() -> Vec<Box<dyn ProofBodyRenderer>> {
    vec![
        Box::new(LeanProofBodyRenderer::new()),
        Box::new(CoqProofBodyRenderer::new()),
        Box::new(AgdaProofBodyRenderer::new()),
        Box::new(DeduktiProofBodyRenderer::new()),
        Box::new(IsabelleProofBodyRenderer::new()),
    ]
}

#[test]
fn intermediate_propositions_cannot_disappear_from_a_translated_body() {
    let falsehood = Heap::new(Expr::literal(Literal::bool(false, span())));
    for step in [
        ProofStepKind::Have {
            name: Ident::new("invented", span()),
            proposition: falsehood.clone(),
            justification: TacticExpr::Trivial,
        },
        ProofStepKind::Show {
            proposition: falsehood.clone(),
            justification: TacticExpr::Trivial,
        },
        ProofStepKind::Suffices {
            proposition: falsehood,
            justification: TacticExpr::Trivial,
        },
    ] {
        let body = ProofBody::Structured(ProofStructure {
            steps: List::from_iter([ProofStep {
                kind: step,
                span: span(),
            }]),
            conclusion: Maybe::Some(TacticExpr::Trivial),
            span: span(),
        });
        for renderer in renderers() {
            assert!(
                !renderer.render(&body).is_translated(),
                "{} erased a proposition",
                renderer.id()
            );
        }
    }
}

#[test]
fn generic_arguments_cannot_disappear_inside_proof_terms_or_apply_arguments() {
    let generic = Expr::new(
        ExprKind::Call {
            func: Heap::new(var("generic")),
            type_args: List::from_iter([verum_ast::GenericArg::Type(Type::new(
                TypeKind::Never,
                span(),
            ))]),
            args: List::new(),
        },
        span(),
    );
    for body in [
        ProofBody::Term(Heap::new(generic.clone())),
        ProofBody::Tactic(TacticExpr::Apply {
            lemma: Heap::new(var("witness")),
            args: List::from_iter([generic]),
        }),
    ] {
        for renderer in renderers() {
            assert!(
                !renderer.render(&body).is_translated(),
                "{} erased a generic argument",
                renderer.id()
            );
        }
    }
}

#[test]
#[ignore = "requires independent Lean; set VERUM_LEAN_EXECUTABLE and run with --include-ignored"]
fn actual_rendered_arguments_are_checked_by_independent_lean() {
    let lean = std::env::var_os("VERUM_LEAN_EXECUTABLE").expect("independent Lean executable");
    let directory = std::env::temp_dir().join(format!(
        "verum-foreign-evidence-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    for (argument, accepted) in [(1, true), (2, false)] {
        let rendered = LeanProofBodyRenderer::new().render(&application(argument));
        let path = directory.join(format!("argument-{argument}.lean"));
        let axiom_check = if accepted {
            "#print axioms actual\n"
        } else {
            ""
        };
        fs::write(&path, format!(
            "theorem witness (n : Nat) : n = n := rfl\ntheorem actual : (1 : Nat) = 1 := {}\n{axiom_check}",
            rendered.text().expect("complete argument translation"),
        )).unwrap();
        let output = Command::new(&lean)
            .args(["-M", "1024", "--json"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostics: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(output.status.success(), accepted, "{diagnostics:?}");
        if accepted {
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert!(
                diagnostics[0]["data"]
                    .as_str()
                    .is_some_and(|s| s.ends_with("does not depend on any axioms")),
                "{diagnostics:?}"
            );
        } else {
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            assert!(
                diagnostics[0]["severity"] == "error"
                    && diagnostics[0]["data"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("Tactic `apply` failed:")),
                "{diagnostics:?}"
            );
        }
    }
    fs::remove_dir_all(directory).unwrap();
}
