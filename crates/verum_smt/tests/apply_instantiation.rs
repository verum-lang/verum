//! T0842 pin: positional lemma instantiation over DECLARED params.
//!
//! `instantiate_lemma` is the one carrier both apply surfaces use:
//! goal-directed `try_apply_with` and the structured-proof bare
//! `apply` step (which turns the instantiated conclusion into a
//! hypothesis). The substitution targets the lemma's declared
//! parameter names in declaration order — the free-variable
//! occurrence order inside the proposition is NOT the calling
//! convention and must not be consulted when params are present.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::{Ident, Path};
use verum_ast::Span;
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::proof_search::{HintsDatabase, LemmaHint, ProofSearchEngine};

fn sp() -> Span {
    Span::default()
}

fn var(name: &str) -> Expr {
    Expr {
        kind: ExprKind::Path(Path::single(Ident::new(name, sp()))),
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
        resolved_call_target: None,
    }
}

fn call(fname: &str, args: Vec<Expr>) -> Expr {
    Expr {
        kind: ExprKind::Call {
            func: Heap::new(var(fname)),
            type_args: List::default(),
            args: args.into_iter().collect(),
        },
        span: sp(),
        ref_kind: None,
        check_eliminated: false,
        resolved_call_target: None,
    }
}

fn render(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Path(p) => p.as_ident().map(|i| i.as_str().to_string()).unwrap(),
        ExprKind::Call { func, args, .. } => format!(
            "{}({})",
            render(func),
            args.iter().map(render).collect::<Vec<_>>().join(",")
        ),
        _ => "?".into(),
    }
}

/// An axiom `grounding(w) ensures P(w)` applied as `apply
/// grounding(v)` must yield conclusion `P(v)` — the caller's
/// argument, not the axiom's own parameter name.
#[test]
fn declared_params_drive_positional_instantiation() {
    let mut hints = HintsDatabase::with_core();
    let mut params = List::new();
    params.push(Text::from("w"));
    hints.register_lemma(
        Text::from("grounding"),
        LemmaHint {
            name: Text::from("grounding"),
            priority: 500,
            lemma: Heap::new(call("P", vec![var("w")])),
            params,
        },
    );
    let engine = ProofSearchEngine::with_hints(hints);

    let mut args = List::new();
    args.push(Text::from("v"));
    let (premises, conclusion) = engine
        .instantiate_lemma(&Text::from("grounding"), &args)
        .expect("registered lemma instantiates");

    assert!(premises.is_empty(), "a bare ensures has no premises");
    assert_eq!(
        render(&conclusion),
        "P(v)",
        "the conclusion must carry the CALLER's argument positionally \
         substituted for the declared parameter"
    );
}

/// A missing lemma is a named error, not a silent no-op — the step
/// that consumes this surface must be able to fail loudly.
#[test]
fn unknown_lemma_is_a_named_error() {
    let engine = ProofSearchEngine::with_hints(HintsDatabase::with_core());
    let args: List<Text> = List::new();
    let err = engine
        .instantiate_lemma(&Text::from("no_such_lemma"), &args)
        .expect_err("unknown lemma must error");
    let msg = format!("{}", err);
    assert!(
        msg.contains("no_such_lemma"),
        "the error must NAME the missing lemma; got: {msg}"
    );
}

/// `Maybe` is imported to keep the test honest about the engine API
/// surface without an unused-import warning.
#[allow(dead_code)]
fn _maybe_surface(_: Maybe<()>) {}
