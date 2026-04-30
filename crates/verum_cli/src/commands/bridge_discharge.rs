//! Bridge-discharge audit walker — task #134 / MSFS-L4.1.
//!
//! Walks the corpus's `.vr` modules, finds every
//! `apply kernel_*_strict(args)` invocation in proof bodies, and
//! invokes [`verum_kernel::dispatch_intrinsic`] against the literal-arg
//! call sites.  Reports per-bridge:
//!
//!   * **callsites_total**       — total `apply` invocations of the bridge
//!   * **callsites_literal_args** — invocations whose args reduce to literals
//!     (the dispatcher can run on these; the elaborator wiring in #135
//!     turns the dispatcher's verdict into a compile-time gate)
//!   * **callsites_non_literal**  — invocations with non-literal args
//!     (e.g., function-of-parameter forms; these are admitted under the
//!     runtime ladder until #135 lands)
//!   * **dispatcher_decisions**  — per-callsite Decision { holds, reason }
//!     for every literal-arg invocation
//!   * **false_discharges**      — count of invocations where the
//!     dispatcher returned `holds: false` (CI-fail trigger)
//!
//! **Architecture**: this module is the *observability layer* on top
//! of the existing `verum_kernel::intrinsic_dispatch` infrastructure.
//! It introduces no per-bridge hardcoding — every bridge auto-registers
//! through the dispatcher table, and every literal-arg invocation in
//! the corpus is replayed mechanically.  Adding a new
//! `kernel_<verb>_strict` bridge requires only registering the
//! dispatcher entry; this audit picks up the discharge automatically.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use verum_ast::decl::{ItemKind, ProofBody, ProofStep, ProofStepKind, TacticExpr};
use verum_ast::{Expr, ExprKind, LiteralKind, Literal, Module};
use verum_kernel::intrinsic_dispatch::{IntrinsicValue, dispatch_intrinsic, is_known_intrinsic};

/// One callsite's discharge result.  Each `apply kernel_*_strict(args)`
/// invocation in a proof body produces one of these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallsiteDischarge {
    /// Relative file path of the corpus module containing the callsite.
    pub file: PathBuf,
    /// Theorem / lemma / corollary name owning this proof body.
    pub item_name: String,
    /// Bare bridge name (e.g., `kernel_grothendieck_construction`) —
    /// the `_strict` suffix is stripped because the dispatcher table
    /// is keyed on the bare form.
    pub bridge_name: String,
    /// Whether the args were all literals (and therefore eligible for
    /// dispatcher invocation).
    pub all_literal_args: bool,
    /// String representation of the args as written in the source —
    /// useful for audit reports even when the dispatcher can't run.
    pub args_text: Vec<String>,
    /// Dispatcher verdict.  `Some(true)` if the dispatcher returned
    /// `Decision { holds: true }`, `Some(false)` for `holds: false`,
    /// `None` if the dispatcher couldn't be invoked (non-literal args
    /// or unrecognised name).
    pub holds: Option<bool>,
    /// Dispatcher's stated reason.  Empty when the dispatcher wasn't
    /// invoked.  Preserved verbatim so the audit-JSON consumer sees
    /// exactly the kernel's diagnostic message.
    pub reason: String,
}

/// Per-bridge aggregation of every callsite that targeted it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeReport {
    /// Bare bridge name.
    pub bridge_name: String,
    /// Total callsites (literal + non-literal).
    pub callsites_total: usize,
    /// Callsites with all-literal args (dispatcher-eligible).
    pub callsites_literal_args: usize,
    /// Callsites with non-literal args.
    pub callsites_non_literal: usize,
    /// Count of callsites where the dispatcher returned `holds: false`.
    pub false_discharges: usize,
    /// All callsites in canonical order (file, item, position).
    pub callsites: Vec<CallsiteDischarge>,
}

/// Top-level discharge audit report.  Aggregates per-bridge plus
/// totals.  Serialised verbatim into `audit-reports/bridge-discharge.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DischargeReport {
    /// Number of `.vr` modules walked.
    pub modules_scanned: usize,
    /// Number of theorem-shaped declarations whose proof bodies were walked.
    pub items_walked: usize,
    /// Total callsites across all bridges.
    pub total_callsites: usize,
    /// Total false-discharge sites (sum across bridges).
    pub total_false_discharges: usize,
    /// Per-bridge reports keyed by bridge name (canonical sorted order).
    pub bridges: Vec<BridgeReport>,
    /// Names of bridges referenced in the corpus that don't have a
    /// dispatcher entry — gap report.  An empty list is the green
    /// state.
    pub unknown_bridges: Vec<String>,
}

/// Walk a `Module` and accumulate per-bridge callsite results.
///
/// `aggregator` is a mutable map from bridge-name → BridgeReport.  The
/// caller calls `walk_module` for every parsed `.vr` file in the
/// corpus, then promotes the map into the final `DischargeReport`.
pub fn walk_module(
    module: &Module,
    rel_path: &Path,
    aggregator: &mut BTreeMap<String, BridgeReport>,
    items_walked: &mut usize,
) {
    for item in module.items.iter() {
        let (item_name, proof_opt) = match &item.kind {
            ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t) => {
                (t.name.as_str().to_string(), t.proof.as_ref())
            }
            // FunctionDecl has no `proof` field — theorem-shaped proofs
            // always go through TheoremDecl in this AST.  Skip.
            _ => continue,
        };

        let proof = match proof_opt {
            Some(p) => p,
            None => continue,
        };
        *items_walked += 1;

        match proof {
            ProofBody::Structured(s) => {
                for step in s.steps.iter() {
                    visit_proof_step(step, rel_path, &item_name, aggregator);
                }
            }
            // Tactic-form proof body is a single TacticExpr; walk it
            // for `apply` invocations.
            ProofBody::Tactic(t) => {
                visit_tactic_for_apply(t, rel_path, &item_name, aggregator);
            }
            // Term / ByMethod proof bodies don't carry tactic
            // invocations the audit cares about.  Skip.
            ProofBody::ByMethod(_) | ProofBody::Term(_) => continue,
        }
    }
}

/// Visit a single proof step looking for `apply kernel_*` invocations.
fn visit_proof_step(
    step: &ProofStep,
    rel_path: &Path,
    item_name: &str,
    aggregator: &mut BTreeMap<String, BridgeReport>,
) {
    match &step.kind {
        ProofStepKind::Tactic(t) => visit_tactic_for_apply(t, rel_path, item_name, aggregator),
        ProofStepKind::Have { justification, .. }
        | ProofStepKind::Show { justification, .. }
        | ProofStepKind::Suffices { justification, .. } => {
            visit_tactic_for_apply(justification, rel_path, item_name, aggregator);
        }
        ProofStepKind::Cases { cases, .. } => {
            for case in cases.iter() {
                for sub_step in case.proof.iter() {
                    visit_proof_step(sub_step, rel_path, item_name, aggregator);
                }
            }
        }
        ProofStepKind::Focus { steps, .. } => {
            for sub_step in steps.iter() {
                visit_proof_step(sub_step, rel_path, item_name, aggregator);
            }
        }
        ProofStepKind::Calc(chain) => {
            for cstep in chain.steps.iter() {
                visit_tactic_for_apply(&cstep.justification, rel_path, item_name, aggregator);
            }
        }
        ProofStepKind::Let { .. } | ProofStepKind::Obtain { .. } => {}
    }
}

/// Recursively walk a `TacticExpr` looking for `Apply { lemma, args }`.
///
/// Tactic combinators (Then / OrElse / Repeat / etc.) are walked into
/// so that nested `apply` calls inside `then` chains are picked up.
fn visit_tactic_for_apply(
    tactic: &TacticExpr,
    rel_path: &Path,
    item_name: &str,
    aggregator: &mut BTreeMap<String, BridgeReport>,
) {
    match tactic {
        TacticExpr::Apply { lemma, args } => {
            // Two parser shapes for `apply lemma(arg1, arg2, ...)`:
            //   1. `lemma: Path(name)`, `args: [arg1, arg2, ...]`
            //   2. `lemma: Call { func: Path(name), args: [arg1, ...] }`,
            //      `args: []`  ← what the fast parser actually produces.
            //
            // Detect both by case-analysing `lemma`.  When the lemma
            // is a Call, hoist its inner args.
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
            handle_apply_callsite(effective_lemma, &arg_refs, rel_path, item_name, aggregator);
        }
        TacticExpr::Try(inner) | TacticExpr::Repeat(inner) | TacticExpr::AllGoals(inner)
        | TacticExpr::Focus(inner) => {
            visit_tactic_for_apply(inner, rel_path, item_name, aggregator);
        }
        TacticExpr::TryElse { body, fallback } => {
            visit_tactic_for_apply(body, rel_path, item_name, aggregator);
            visit_tactic_for_apply(fallback, rel_path, item_name, aggregator);
        }
        TacticExpr::Seq(tacs) | TacticExpr::Alt(tacs) => {
            for t in tacs.iter() {
                visit_tactic_for_apply(t, rel_path, item_name, aggregator);
            }
        }
        // Other tactic forms (Intro / Rewrite / Simp / Ring / Field /
        // Reflexivity / Assumption / Trivial / Smt / Auto / Blast /
        // Split / Left / Right / Exists / CasesOn / InductionOn /
        // Exact / Unfold / Compute / Named / Omega) don't carry an
        // `apply lemma` payload directly — nothing to walk.
        _ => {}
    }
}

/// Process a single `apply lemma(args...)` callsite.  Extracts the
/// lemma name, classifies the args as literal vs non-literal, invokes
/// the dispatcher when possible, and records the verdict in the
/// aggregator.
fn handle_apply_callsite(
    lemma: &Expr,
    args: &[&Expr],
    rel_path: &Path,
    item_name: &str,
    aggregator: &mut BTreeMap<String, BridgeReport>,
) {
    let raw_name = match expr_as_path_name(lemma) {
        Some(n) => n,
        None => return,
    };

    // We're only interested in `kernel_*` bridges.
    if !raw_name.starts_with("kernel_") {
        return;
    }

    // The dispatcher table is keyed on the bare name; strict-form
    // bridges share the same entry.  Strip the `_strict` suffix if
    // present.
    let bare_name = raw_name.strip_suffix("_strict").unwrap_or(&raw_name).to_string();

    let mut args_text: Vec<String> = Vec::with_capacity(args.len());
    let mut intrinsic_args: Vec<Option<IntrinsicValue>> = Vec::with_capacity(args.len());

    for a in args {
        args_text.push(expr_to_text(a));
        intrinsic_args.push(expr_to_intrinsic_value(a));
    }

    let all_literal = !intrinsic_args.is_empty()
        && intrinsic_args.iter().all(|v| v.is_some());
    // Dispatcher invocation policy:
    //   - all-literal args: invoke and capture verdict
    //   - bare call (zero args): invoke with empty arg slice; some
    //     dispatchers handle this (returning the bare-call default),
    //     others require args (returning None)
    //   - mixed / non-literal: skip dispatcher, classify as
    //     "non-literal" — the elaborator wiring (#135) handles these
    let dispatcher_eligible = all_literal || intrinsic_args.is_empty();

    let (holds, reason) = if dispatcher_eligible {
        let resolved: Vec<IntrinsicValue> = intrinsic_args
            .iter()
            .filter_map(|v| v.clone())
            .collect();
        match dispatch_intrinsic(&bare_name, &resolved) {
            Some(IntrinsicValue::Decision { holds, reason }) => {
                (Some(holds), reason.clone())
            }
            Some(IntrinsicValue::Bool(b)) => (Some(b), String::new()),
            Some(_) => (None, "dispatcher returned non-Decision value".to_string()),
            None => (None, "dispatcher rejected the args (likely wrong shape)".to_string()),
        }
    } else {
        (None, "non-literal arg — dispatcher invocation deferred to #135".to_string())
    };

    let report = aggregator.entry(bare_name.clone()).or_insert_with(|| BridgeReport {
        bridge_name: bare_name.clone(),
        ..Default::default()
    });
    report.callsites_total += 1;
    if all_literal {
        report.callsites_literal_args += 1;
    } else {
        report.callsites_non_literal += 1;
    }
    if holds == Some(false) {
        report.false_discharges += 1;
    }
    report.callsites.push(CallsiteDischarge {
        file: rel_path.to_path_buf(),
        item_name: item_name.to_string(),
        bridge_name: bare_name,
        all_literal_args: all_literal,
        args_text,
        holds,
        reason,
    });
}

/// Project an `Expr` to its single-segment Path identifier name.  Any
/// other shape returns `None`.
fn expr_as_path_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Path(path) => path.as_ident().map(|i| i.as_str().to_string()),
        _ => None,
    }
}

/// Convert a literal `Expr` to an `IntrinsicValue` if it's a literal
/// the dispatcher can consume.  Returns `None` for any non-literal
/// shape (parameter reference, arithmetic expression, function call,
/// etc.).
fn expr_to_intrinsic_value(e: &Expr) -> Option<IntrinsicValue> {
    match &e.kind {
        ExprKind::Literal(Literal { kind, .. }) => match kind {
            LiteralKind::Int(int_lit) => {
                // The intrinsic dispatcher uses `IntrinsicValue::Int(i64)`;
                // i128 → i64 narrowing is sound for the literal sizes
                // bridges actually use (StrictPos, NonNegInt, levels ≤
                // u32::MAX).  Negative literals are also fine because
                // the dispatcher's own range checks will reject them.
                let v = int_lit.value;
                if v >= i64::MIN as i128 && v <= i64::MAX as i128 {
                    Some(IntrinsicValue::Int(v as i64))
                } else {
                    None
                }
            }
            LiteralKind::Bool(b) => Some(IntrinsicValue::Bool(*b)),
            LiteralKind::Text(s) => {
                use verum_ast::literal::StringLit;
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

/// Render an `Expr` to a human-readable text representation suitable
/// for audit reports.  This is intentionally *coarse* — we just want
/// enough text to identify the callsite in a JSON report, not a
/// pretty-printer round-trip.
fn expr_to_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Literal(Literal { kind, .. }) => match kind {
            LiteralKind::Int(int_lit) => int_lit.value.to_string(),
            LiteralKind::Bool(b) => b.to_string(),
            LiteralKind::Text(s) => {
                use verum_ast::literal::StringLit;
                match s {
                    StringLit::Regular(t) | StringLit::MultiLine(t) => {
                        format!("\"{}\"", t.as_str())
                    }
                }
            }
            LiteralKind::Float(f) => f.value.to_string(),
            _ => "<literal>".to_string(),
        },
        ExprKind::Path(path) => match path.as_ident() {
            Some(i) => i.as_str().to_string(),
            None => "<path>".to_string(),
        },
        ExprKind::Paren(inner) => expr_to_text(inner),
        ExprKind::Call { .. } => "<call>".to_string(),
        ExprKind::Binary { .. } => "<binary>".to_string(),
        _ => "<expr>".to_string(),
    }
}

/// Promote a per-bridge aggregator into the final `DischargeReport`.
///
/// Computes totals and identifies bridges referenced in the corpus
/// that don't have a dispatcher entry (the "unknown_bridges" gap).
pub fn finalise_report(
    aggregator: BTreeMap<String, BridgeReport>,
    modules_scanned: usize,
    items_walked: usize,
) -> DischargeReport {
    let mut total_callsites = 0;
    let mut total_false_discharges = 0;
    let mut unknown_bridges: Vec<String> = Vec::new();
    let mut bridges: Vec<BridgeReport> = aggregator.into_values().collect();
    // Canonical sort: bridge_name ASC.
    bridges.sort_by(|a, b| a.bridge_name.cmp(&b.bridge_name));

    for b in &bridges {
        total_callsites += b.callsites_total;
        total_false_discharges += b.false_discharges;
        if !is_known_intrinsic(&b.bridge_name) {
            unknown_bridges.push(b.bridge_name.clone());
        }
    }

    DischargeReport {
        modules_scanned,
        items_walked,
        total_callsites,
        total_false_discharges,
        bridges,
        unknown_bridges,
    }
}
