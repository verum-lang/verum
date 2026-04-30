//! Kernel-bridge discharge validation pre-pass.
//!
//! Task #135 / MSFS-L4.2 — the load-bearing piece that converts the
//! bridge-discharge audit's *observation* (task #134's
//! `verum audit --bridge-discharge`) into a *compile-time gate*.
//!
//! ## Architecture
//!
//! Every `apply kernel_<verb>_strict(literal_args...)` invocation in
//! a proof body must be a legitimate discharge through Verum's
//! kernel.  This pre-pass walks the proof body BEFORE the SMT proof
//! engine sees it, finds those invocations, and replays each through
//! [`verum_kernel::dispatch_intrinsic`].  When the dispatcher returns
//! `Decision { holds: false }`, the discharge is rejected at compile
//! time — the user's proof claim "this discharges through the kernel"
//! is no longer trusted on faith.
//!
//! ## Architectural rationale (high-performance, no per-bridge hardcoding)
//!
//!   * **One code path, all bridges.**  The pre-pass is parameterised
//!     over `verum_kernel::available_intrinsics()` + `dispatch_intrinsic`.
//!     Adding a new `kernel_<verb>_strict` bridge requires only
//!     registering its dispatcher entry; this validator picks it up
//!     automatically with no edit here.
//!
//!   * **Pre-Z3 cost.**  Pure AST walk + hashmap dispatch — no SMT
//!     solver invocation, no kernel-recheck round-trip.  Microsecond-
//!     scale per proof body.  The expensive part stays downstream.
//!
//!   * **Conservative on non-literal args.**  Args that aren't
//!     literally evaluable at parse time (parameter references,
//!     arithmetic expressions, function calls) are passed through —
//!     downstream layers (#136 / SMT engine) handle them.  We only
//!     gate the literal-arg path because that's where the dispatcher
//!     can run statically with full information.
//!
//!   * **Error preservation.**  When the dispatcher rejects, the
//!     compile error carries the dispatcher's `reason` text verbatim
//!     so the user sees exactly *why* the kernel refused the args
//!     (e.g., "grothendieck::build_grothendieck preconditions:
//!     |fibres|=0 must be > 0").

use std::path::PathBuf;

use verum_ast::decl::{ProofBody, ProofStep, ProofStepKind, TacticExpr};
use verum_ast::literal::StringLit;
use verum_ast::{Expr, ExprKind, Literal, LiteralKind};
use verum_common::Text;
use verum_kernel::intrinsic_dispatch::{IntrinsicValue, dispatch_intrinsic};

/// A single kernel-bridge discharge that the dispatcher rejected.
///
/// Used to thread compile-time rejection diagnostics back to the
/// caller, which folds them into the verification phase's error
/// surface.
#[derive(Debug, Clone)]
pub struct BridgeDischargeError {
    /// Theorem / lemma name owning the failing proof body.
    pub item_name: Text,
    /// Bare bridge name (e.g. `kernel_grothendieck_construction`).
    /// `_strict` suffix stripped — the dispatcher table is keyed on
    /// the bare form.
    pub bridge_name: String,
    /// Args as written in the source (literal text rendering for the
    /// diagnostic message).
    pub args_rendered: Vec<String>,
    /// Dispatcher's stated reason for the rejection.  Preserved
    /// verbatim from `verum_kernel`'s `Decision { holds: false, reason }`.
    pub reason: String,
    /// Source path, surfaced to the diagnostic builder.  May be empty
    /// when the verification phase didn't carry a path through.
    pub source_path: PathBuf,
}

/// Walk a theorem's proof body and accumulate any kernel-bridge
/// discharges that the dispatcher rejects.
///
/// Returns an empty `Vec` when every bridge invocation either:
///   * has all-literal args AND the dispatcher returned `holds: true`
///   * has non-literal args (deferred to downstream layers)
///   * does not target a `kernel_*_strict` bridge at all
///
/// Returns `Vec<BridgeDischargeError>` when one or more invocations
/// failed the dispatcher's structural check.
pub fn validate_proof_body_bridges(
    proof_body: &ProofBody,
    item_name: &Text,
    source_path: &std::path::Path,
) -> Vec<BridgeDischargeError> {
    let mut errors = Vec::new();
    let ctx = WalkContext {
        item_name: item_name.clone(),
        source_path: source_path.to_path_buf(),
    };
    walk_proof_body(proof_body, &ctx, &mut errors);
    errors
}

/// Walking-context bundle threaded through the recursion.  Avoids
/// passing the same `(item_name, source_path)` pair down every
/// recursive call site.
struct WalkContext {
    item_name: Text,
    source_path: PathBuf,
}

fn walk_proof_body(
    body: &ProofBody,
    ctx: &WalkContext,
    errors: &mut Vec<BridgeDischargeError>,
) {
    match body {
        ProofBody::Structured(s) => {
            for step in s.steps.iter() {
                walk_proof_step(step, ctx, errors);
            }
        }
        ProofBody::Tactic(t) => walk_tactic(t, ctx, errors),
        ProofBody::Term(_) | ProofBody::ByMethod(_) => {}
    }
}

fn walk_proof_step(
    step: &ProofStep,
    ctx: &WalkContext,
    errors: &mut Vec<BridgeDischargeError>,
) {
    match &step.kind {
        ProofStepKind::Tactic(t) => walk_tactic(t, ctx, errors),
        ProofStepKind::Have { justification, .. }
        | ProofStepKind::Show { justification, .. }
        | ProofStepKind::Suffices { justification, .. } => {
            walk_tactic(justification, ctx, errors);
        }
        ProofStepKind::Cases { cases, .. } => {
            for case in cases.iter() {
                for sub in case.proof.iter() {
                    walk_proof_step(sub, ctx, errors);
                }
            }
        }
        ProofStepKind::Focus { steps, .. } => {
            for sub in steps.iter() {
                walk_proof_step(sub, ctx, errors);
            }
        }
        ProofStepKind::Calc(chain) => {
            for cstep in chain.steps.iter() {
                walk_tactic(&cstep.justification, ctx, errors);
            }
        }
        ProofStepKind::Let { .. } | ProofStepKind::Obtain { .. } => {}
    }
}

fn walk_tactic(
    tactic: &TacticExpr,
    ctx: &WalkContext,
    errors: &mut Vec<BridgeDischargeError>,
) {
    match tactic {
        TacticExpr::Apply { lemma, args } => {
            // Two parser shapes — match both.  The fast parser emits
            // Apply{lemma:Call{func, args}, args:[]} for `apply name(literal)`;
            // the structured form has Apply{lemma:Path, args:[...]}.
            let owned_args: Vec<Expr>;
            let (effective_lemma, arg_refs): (&Expr, Vec<&Expr>) = match &lemma.kind {
                ExprKind::Call { func, args: call_args, .. } if args.is_empty() => {
                    owned_args = call_args.iter().cloned().collect();
                    let refs: Vec<&Expr> = owned_args.iter().collect();
                    (&**func, refs)
                }
                _ => {
                    let refs: Vec<&Expr> = args.iter().collect();
                    (&**lemma, refs)
                }
            };
            check_apply_callsite(effective_lemma, &arg_refs, ctx, errors);
        }
        TacticExpr::Try(inner)
        | TacticExpr::Repeat(inner)
        | TacticExpr::AllGoals(inner)
        | TacticExpr::Focus(inner) => walk_tactic(inner, ctx, errors),
        TacticExpr::TryElse { body, fallback } => {
            walk_tactic(body, ctx, errors);
            walk_tactic(fallback, ctx, errors);
        }
        TacticExpr::Seq(tacs) | TacticExpr::Alt(tacs) => {
            for t in tacs.iter() {
                walk_tactic(t, ctx, errors);
            }
        }
        // Other tactic forms don't carry an `apply lemma` payload.
        _ => {}
    }
}

/// Process a single `apply lemma(args...)` callsite.  When it's a
/// `kernel_*_strict` bridge with all-literal args, dispatch through
/// the kernel and record any rejection as a `BridgeDischargeError`.
fn check_apply_callsite(
    lemma: &Expr,
    args: &[&Expr],
    ctx: &WalkContext,
    errors: &mut Vec<BridgeDischargeError>,
) {
    let raw_name = match expr_as_path_name(lemma) {
        Some(n) => n,
        None => return,
    };
    if !raw_name.starts_with("kernel_") {
        return;
    }

    // Dispatcher table is keyed on the bare form; strict-suffix bridges
    // share the entry.
    let bare_name = raw_name
        .strip_suffix("_strict")
        .unwrap_or(&raw_name)
        .to_string();

    // Convert each arg to an IntrinsicValue if literal; bail out
    // (non-literal → deferred to downstream) on any non-literal arg.
    let mut intrinsic_args: Vec<IntrinsicValue> = Vec::with_capacity(args.len());
    let mut args_rendered: Vec<String> = Vec::with_capacity(args.len());
    for a in args {
        match expr_to_intrinsic_value(a) {
            Some(v) => intrinsic_args.push(v),
            None => {
                // Non-literal arg — defer to downstream layers.  We
                // still render the arg-text for diagnostics in case the
                // failure diagnostic later surfaces.
                args_rendered.push(expr_to_text(a));
                return;
            }
        }
        args_rendered.push(expr_to_text(a));
    }

    // Invoke dispatcher.  Bare-call (zero args) form is handled by
    // some bridges via a separate name; for non-zero args, dispatch
    // with the resolved IntrinsicValues.
    let decision = dispatch_intrinsic(&bare_name, &intrinsic_args);

    match decision {
        Some(IntrinsicValue::Decision { holds: false, reason }) => {
            errors.push(BridgeDischargeError {
                item_name: ctx.item_name.clone(),
                bridge_name: bare_name,
                args_rendered,
                reason,
                source_path: ctx.source_path.clone(),
            });
        }
        Some(IntrinsicValue::Bool(false)) => {
            errors.push(BridgeDischargeError {
                item_name: ctx.item_name.clone(),
                bridge_name: bare_name,
                args_rendered,
                reason: "dispatcher returned Bool(false) for the supplied args".to_string(),
                source_path: ctx.source_path.clone(),
            });
        }
        // None / Decision{holds: true} / Bool(true) / other: accept.
        // The audit gate (#134) reports the full classification;
        // here we only fail the build on outright rejection.
        _ => {}
    }
}

fn expr_as_path_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Path(path) => path.as_ident().map(|i| i.as_str().to_string()),
        _ => None,
    }
}

fn expr_to_intrinsic_value(e: &Expr) -> Option<IntrinsicValue> {
    match &e.kind {
        ExprKind::Literal(Literal { kind, .. }) => match kind {
            LiteralKind::Int(int_lit) => {
                let v = int_lit.value;
                if (i64::MIN as i128..=i64::MAX as i128).contains(&v) {
                    Some(IntrinsicValue::Int(v as i64))
                } else {
                    None
                }
            }
            LiteralKind::Bool(b) => Some(IntrinsicValue::Bool(*b)),
            LiteralKind::Text(s) => {
                let text = match s {
                    StringLit::Regular(t) | StringLit::MultiLine(t) => t.as_str().to_string(),
                };
                Some(IntrinsicValue::Text(text.into()))
            }
            _ => None,
        },
        ExprKind::Paren(inner) => expr_to_intrinsic_value(inner),
        _ => None,
    }
}

fn expr_to_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Literal(Literal { kind, .. }) => match kind {
            LiteralKind::Int(int_lit) => int_lit.value.to_string(),
            LiteralKind::Bool(b) => b.to_string(),
            LiteralKind::Text(s) => match s {
                StringLit::Regular(t) | StringLit::MultiLine(t) => {
                    format!("\"{}\"", t.as_str())
                }
            },
            LiteralKind::Float(f) => f.value.to_string(),
            _ => "<literal>".to_string(),
        },
        ExprKind::Path(path) => match path.as_ident() {
            Some(i) => i.as_str().to_string(),
            None => "<path>".to_string(),
        },
        ExprKind::Paren(inner) => expr_to_text(inner),
        _ => "<expr>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{ProofStructure, ProofStep};
    
    use verum_ast::ty::{Ident, Path};
    use verum_ast::{Span};
    use verum_common::Heap;

    fn ident_path_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(Path::single(Ident::new(name, Span::dummy()))),
            Span::dummy(),
        )
    }

    fn int_literal_expr(n: i64) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::int(n as i128, Span::dummy())),
            Span::dummy(),
        )
    }

    fn make_apply_step(lemma_name: &str, args: Vec<Expr>) -> ProofStep {
        // Match what the parser would produce: lemma=Call{func: Path(name), args=[...]}, args=[]
        let lemma_call = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_path_expr(lemma_name)),
                args: args.into_iter().collect(),
                type_args: verum_common::List::new(),
            },
            Span::dummy(),
        );
        let tactic = TacticExpr::Apply {
            lemma: Heap::new(lemma_call),
            args: verum_common::List::new(),
        };
        ProofStep {
            kind: ProofStepKind::Tactic(tactic),
            span: Span::dummy(),
        }
    }

    fn structured_proof_body(steps: Vec<ProofStep>) -> ProofBody {
        ProofBody::Structured(ProofStructure {
            steps: steps.into_iter().collect(),
            conclusion: verum_common::Maybe::None,
            span: Span::dummy(),
        })
    }

    #[test]
    fn passing_kernel_grothendieck_with_strictpos_arg_emits_no_error() {
        // `kernel_grothendieck_construction(1)` — 1 > 0 satisfies StrictPos.
        let body = structured_proof_body(vec![
            make_apply_step("kernel_grothendieck_construction_strict", vec![int_literal_expr(1)]),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert!(errors.is_empty(), "passing dispatcher arg must not produce errors: {:?}", errors);
    }

    #[test]
    fn failing_kernel_grothendieck_with_zero_arg_emits_error() {
        // `kernel_grothendieck_construction(0)` — 0 violates StrictPos.
        let body = structured_proof_body(vec![
            make_apply_step("kernel_grothendieck_construction_strict", vec![int_literal_expr(0)]),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert_eq!(errors.len(), 1, "dispatcher rejection must produce exactly one error");
        let e = &errors[0];
        assert_eq!(e.item_name.as_str(), "test_thm");
        assert_eq!(e.bridge_name, "kernel_grothendieck_construction");
        assert_eq!(e.args_rendered, vec!["0".to_string()]);
        assert!(
            e.reason.contains(">"),
            "reason must mention the StrictPos check; got: {}",
            e.reason,
        );
    }

    #[test]
    fn non_kernel_apply_is_ignored() {
        // `apply some_lemma(0)` is not a kernel bridge — the validator
        // must not check it.
        let body = structured_proof_body(vec![
            make_apply_step("some_user_lemma", vec![int_literal_expr(0)]),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert!(errors.is_empty(), "non-kernel apply must be ignored");
    }

    #[test]
    fn non_literal_args_are_deferred() {
        // Args that aren't literals (here, a Path expression) are
        // deferred to downstream layers; the validator must not error.
        let body = structured_proof_body(vec![
            make_apply_step(
                "kernel_grothendieck_construction_strict",
                vec![ident_path_expr("n")],
            ),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert!(
            errors.is_empty(),
            "non-literal arg must be deferred to downstream layers, not erored",
        );
    }

    #[test]
    fn multiple_failing_callsites_aggregate() {
        // Two failing kernel bridges in one proof body — both should
        // appear in the error list.
        let body = structured_proof_body(vec![
            make_apply_step("kernel_grothendieck_construction_strict", vec![int_literal_expr(0)]),
            make_apply_step("kernel_compute_colimit_strict", vec![int_literal_expr(0)]),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert_eq!(errors.len(), 2, "both failing callsites must surface");
        let names: Vec<&str> = errors.iter().map(|e| e.bridge_name.as_str()).collect();
        assert!(names.contains(&"kernel_grothendieck_construction"));
        assert!(names.contains(&"kernel_compute_colimit"));
    }

    #[test]
    fn passing_then_failing_yields_one_error() {
        // Mixed pass/fail — the failing one is reported, the passing
        // one isn't.
        let body = structured_proof_body(vec![
            make_apply_step("kernel_grothendieck_construction_strict", vec![int_literal_expr(1)]),
            make_apply_step("kernel_grothendieck_construction_strict", vec![int_literal_expr(0)]),
        ]);
        let errors = validate_proof_body_bridges(
            &body,
            &Text::from("test_thm"),
            std::path::Path::new("test.vr"),
        );
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].args_rendered, vec!["0".to_string()]);
    }
}
