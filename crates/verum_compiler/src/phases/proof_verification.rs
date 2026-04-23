//! Proof Verification Phase
//!
//! Bridges the AST proof system (in `verum_ast`) with the SMT proof search engine
//! (in `verum_smt`). This module converts AST-level proof constructs — tactic
//! expressions, structured proof steps, calculation chains, and proof methods —
//! into SMT-level proof tactics that can be executed by `ProofSearchEngine`.
//!
//! # Architecture
//!
//! ```text
//! verum_ast::TacticExpr ──┐
//! verum_ast::ProofStep    ├──► proof_verification ──► verum_smt::ProofTactic
//! verum_ast::ProofMethod  │                           verum_smt::ProofSearchEngine
//! verum_ast::ProofBody  ──┘                           ──► VerificationResult
//! ```
//!
//! # Proof Body Variants
//!
//! - **Term**: Curry-Howard proof term — type-checked as an expression whose type
//!   matches the proposition.
//! - **Tactic**: Single tactic expression converted to `ProofTactic` and executed.
//! - **Structured**: Sequence of `ProofStep`s building toward a conclusion, each
//!   step producing or consuming subgoals.
//! - **ByMethod**: Dispatches induction, case analysis, or contradiction with
//!   appropriate subgoal generation.

use std::time::{Duration, Instant};

use verum_ast::decl::{
    CalcRelation, CalculationChain, ProofBody, ProofCase, ProofMethod,
    ProofStep, ProofStepKind, ProofStructure, TacticExpr, TacticMatchArm, TheoremDecl,
};
use verum_ast::expr::ExprKind;
use verum_ast::literal::LiteralKind;
use verum_ast::pretty::format_expr;
use verum_ast::{Expr, Ident};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::context::Context;
use verum_smt::proof_search::{
    MatchArm, ProofError, ProofGoal, ProofSearchEngine, ProofTactic,
};
use verum_smt::verify::VerificationError;
use verum_verification::tactic_evaluation::{Goal, GoalMetadata, Hypothesis};
use verum_verification::tactic_heuristics::{suggest_next_tactics, TacticSuggestion};

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// A single verified step in a structured proof, recording what was established
/// and by which tactic.
#[derive(Debug, Clone)]
pub struct VerifiedStep {
    /// Human-readable description of the step (e.g. "have h1: x > 0").
    pub description: Text,
    /// The tactic that closed or advanced this step.
    pub tactic_used: Text,
    /// Time spent verifying this step.
    pub duration: Duration,
}

/// Certificate produced on successful proof verification, summarising the
/// evidence chain.
#[derive(Debug, Clone)]
pub struct ProofCertificate {
    /// Theorem name that was verified.
    pub theorem_name: Text,
    /// Ordered list of verified steps (empty for single-tactic / term proofs).
    pub steps: List<VerifiedStep>,
    /// Total wall-clock time for the entire proof.
    pub total_duration: Duration,
    /// Whether any step used `admit` or `sorry`.
    pub has_incomplete_steps: bool,
}

/// An unresolved subgoal that the proof failed to close.
#[derive(Debug, Clone)]
pub struct UnprovedSubgoal {
    /// Pretty-printed goal expression.
    pub goal: Text,
    /// Available hypotheses at the point of failure.
    pub hypotheses: List<Text>,
    /// Machine-generated suggestions for closing this goal.
    pub suggestions: List<Text>,
}

/// Outcome of proof verification.
#[derive(Debug)]
pub enum ProofVerificationResult {
    /// All goals discharged.
    Verified(ProofCertificate),
    /// One or more goals remain open.
    Failed {
        /// Steps that *were* verified before the failure.
        verified_steps: List<VerifiedStep>,
        /// Goals that remain open.
        unproved: List<UnprovedSubgoal>,
    },
}

// ---------------------------------------------------------------------------
// AST-to-SMT tactic conversion
// ---------------------------------------------------------------------------

/// Convert an AST `TacticExpr` into an SMT `ProofTactic`.
///
/// The mapping is largely one-to-one, with AST-level `Expr` / `Ident` nodes
/// rendered to the `Text` representations expected by the SMT engine.
pub fn convert_tactic(tactic: &TacticExpr) -> ProofTactic {
    match tactic {
        TacticExpr::Trivial => {
            // Trivial combines reflexivity + assumption + simplify
            ProofTactic::Auto
        }

        TacticExpr::Assumption => ProofTactic::Assumption,

        TacticExpr::Reflexivity => ProofTactic::Reflexivity,

        TacticExpr::Intro(names) => {
            if names.is_empty() {
                ProofTactic::Intro
            } else {
                ProofTactic::IntroNamed {
                    names: names.iter().map(|id| id.name.clone()).collect(),
                }
            }
        }

        TacticExpr::Apply { lemma, args } => {
            let lemma_text = format_expr(lemma);
            if args.is_empty() {
                ProofTactic::Apply { lemma: lemma_text }
            } else {
                ProofTactic::ApplyWith {
                    lemma: lemma_text,
                    args: args.iter().map(|a| format_expr(a)).collect(),
                }
            }
        }

        TacticExpr::Rewrite {
            hypothesis,
            at_target,
            rev,
        } => {
            let hyp_text = format_expr(hypothesis);
            match at_target {
                Maybe::Some(target) => ProofTactic::RewriteAt {
                    hypothesis: hyp_text,
                    target: target.name.clone(),
                    reverse: *rev,
                },
                Maybe::None => ProofTactic::Rewrite {
                    hypothesis: hyp_text,
                    reverse: *rev,
                },
            }
        }

        TacticExpr::Simp { lemmas, at_target } => {
            if lemmas.is_empty() && matches!(at_target, Maybe::None) {
                ProofTactic::Simplify
            } else {
                let lemma_texts: List<Text> =
                    lemmas.iter().map(|l| format_expr(l)).collect();
                // If there is a target, compose Simp with a focused rewrite
                if let Maybe::Some(target) = at_target {
                    let simp = if lemma_texts.is_empty() {
                        ProofTactic::Simplify
                    } else {
                        ProofTactic::SimpWith {
                            lemmas: lemma_texts,
                        }
                    };
                    // Focus simplification at a specific hypothesis
                    ProofTactic::Seq(
                        Heap::new(ProofTactic::Unfold {
                            name: target.name.clone(),
                        }),
                        Heap::new(simp),
                    )
                } else {
                    ProofTactic::SimpWith {
                        lemmas: lemma_texts,
                    }
                }
            }
        }

        TacticExpr::Ring => ProofTactic::Ring,
        TacticExpr::Field => ProofTactic::Field,
        TacticExpr::Omega => ProofTactic::Omega,

        TacticExpr::Auto { with_hints } => {
            if with_hints.is_empty() {
                ProofTactic::Auto
            } else {
                ProofTactic::AutoWith {
                    hints: with_hints.iter().map(|id| id.name.clone()).collect(),
                }
            }
        }

        TacticExpr::Blast => ProofTactic::Blast,

        TacticExpr::Smt { solver, timeout } => ProofTactic::Smt {
            solver: solver.clone(),
            timeout_ms: *timeout,
        },

        TacticExpr::Split => ProofTactic::Split,
        TacticExpr::Left => ProofTactic::Left,
        TacticExpr::Right => ProofTactic::Right,

        TacticExpr::Exists(witness) => ProofTactic::Exists {
            witness: format_expr(witness),
        },

        TacticExpr::CasesOn(ident) => ProofTactic::CasesOn {
            hypothesis: ident.name.clone(),
        },

        TacticExpr::InductionOn(ident) => ProofTactic::Induction {
            var: ident.name.clone(),
        },

        TacticExpr::Exact(term) => ProofTactic::Exact {
            term: format_expr(term),
        },

        TacticExpr::Unfold(names) => {
            if names.len() == 1 {
                ProofTactic::Unfold {
                    name: names[0].name.clone(),
                }
            } else {
                // Unfold multiple definitions sequentially
                build_seq(
                    names
                        .iter()
                        .map(|n| ProofTactic::Unfold {
                            name: n.name.clone(),
                        })
                        .collect(),
                )
            }
        }

        TacticExpr::Compute => ProofTactic::Compute,

        TacticExpr::Try(inner) => ProofTactic::Try(Heap::new(convert_tactic(inner))),

        TacticExpr::TryElse { body, fallback } => {
            ProofTactic::Alt(
                Heap::new(convert_tactic(body)),
                Heap::new(convert_tactic(fallback)),
            )
        }

        TacticExpr::Repeat(inner) => ProofTactic::Repeat(Heap::new(convert_tactic(inner))),

        TacticExpr::Seq(tactics) => convert_tactic_sequence(tactics),

        TacticExpr::Alt(tactics) => {
            let converted: List<ProofTactic> =
                tactics.iter().map(|t| convert_tactic(t)).collect();
            build_alt(converted)
        }

        TacticExpr::AllGoals(inner) => {
            ProofTactic::AllGoals(Heap::new(convert_tactic(inner)))
        }

        TacticExpr::Focus(inner) => ProofTactic::Focus(Heap::new(convert_tactic(inner))),

        TacticExpr::Named { name, args, .. } => ProofTactic::Named {
            name: name.name.clone(),
            args: args.iter().map(|a| format_expr(a)).collect(),
        },

        TacticExpr::Done => ProofTactic::Done,
        TacticExpr::Admit => ProofTactic::Admit,
        TacticExpr::Sorry => ProofTactic::Sorry,
        TacticExpr::Contradiction => ProofTactic::Contradiction,

        // Tactic-DSL control-flow forms lower to the matching `ProofTactic`
        // IR variants introduced in verum_smt. A bare `let` appearing outside
        // a sequence (which the grammar technically permits) lowers with an
        // empty body; inside a `Seq` it is desugared by
        // `convert_tactic_sequence` so that the trailing tactics become its
        // body.
        TacticExpr::Let { name, value, .. } => ProofTactic::Let {
            name: name.name.clone(),
            value: Heap::new((**value).clone()),
            body: Heap::new(ProofTactic::Done),
        },

        TacticExpr::Match { scrutinee, arms } => ProofTactic::Match {
            scrutinee: Heap::new((**scrutinee).clone()),
            arms: arms.iter().map(convert_match_arm).collect(),
        },

        TacticExpr::Fail { message } => ProofTactic::Fail {
            message: extract_fail_message(message),
        },

        TacticExpr::If {
            cond,
            then_branch,
            else_branch,
        } => ProofTactic::If {
            cond: Heap::new((**cond).clone()),
            then_branch: Heap::new(convert_tactic(then_branch)),
            else_branch: match else_branch {
                Maybe::Some(eb) => Maybe::Some(Heap::new(convert_tactic(eb))),
                Maybe::None => Maybe::None,
            },
        },
    }
}

/// Lower a `Seq(...)` tactic body while desugaring any `let` bindings it
/// contains. A sequence `[pre…, let x = v, post…]` becomes
/// `Seq(build_seq(pre…), Let { x, v, body: convert_tactic_sequence(post…) })`
/// so the let's binding scope is exactly the trailing tactics — which
/// mirrors Ltac2's substitution semantics and Lean 4's monadic `let ← …`.
fn convert_tactic_sequence(tactics: &List<TacticExpr>) -> ProofTactic {
    for (i, t) in tactics.iter().enumerate() {
        if let TacticExpr::Let { name, value, .. } = t {
            let rest: List<TacticExpr> = tactics.iter().skip(i + 1).cloned().collect();
            let body = if rest.is_empty() {
                ProofTactic::Done
            } else {
                convert_tactic_sequence(&rest)
            };
            let let_node = ProofTactic::Let {
                name: name.name.clone(),
                value: Heap::new((**value).clone()),
                body: Heap::new(body),
            };
            if i == 0 {
                return let_node;
            }
            let prefix: List<ProofTactic> = tactics
                .iter()
                .take(i)
                .map(convert_tactic)
                .collect();
            return ProofTactic::Seq(Heap::new(build_seq(prefix)), Heap::new(let_node));
        }
    }
    let converted: List<ProofTactic> = tactics.iter().map(convert_tactic).collect();
    build_seq(converted)
}

/// Lower a single tactic-level match arm.
fn convert_match_arm(arm: &TacticMatchArm) -> MatchArm {
    MatchArm {
        pattern: arm.pattern.clone(),
        guard: match &arm.guard {
            Maybe::Some(g) => Maybe::Some(Heap::new((**g).clone())),
            Maybe::None => Maybe::None,
        },
        body: convert_tactic(&arm.body),
    }
}

/// Render a `fail(...)` message. Plain string literals are emitted raw
/// (without the surrounding quotes that `format_expr` would add) so the
/// diagnostic matches what the user typed; any other expression is
/// pretty-printed verbatim.
fn extract_fail_message(message: &Expr) -> Text {
    if let ExprKind::Literal(lit) = &message.kind {
        if let LiteralKind::Text(string_lit) = &lit.kind {
            return string_lit.clone().into_string();
        }
    }
    format_expr(message)
}

/// Build a sequential composition from a list of tactics.
///
/// An empty list maps to `Done`; a singleton is returned directly; otherwise
/// left-associated `Seq` nodes are constructed.
fn build_seq(tactics: List<ProofTactic>) -> ProofTactic {
    let mut iter = tactics.into_iter();
    let first = match iter.next() {
        Some(t) => t,
        None => return ProofTactic::Done,
    };
    iter.fold(first, |acc, next| {
        ProofTactic::Seq(Heap::new(acc), Heap::new(next))
    })
}

/// Build an alternative choice from a list of tactics.
///
/// An empty list fails; a singleton is returned directly; otherwise
/// left-associated `Alt` nodes are constructed.
fn build_alt(tactics: List<ProofTactic>) -> ProofTactic {
    let mut iter = tactics.into_iter();
    let first = match iter.next() {
        Some(t) => t,
        None => return ProofTactic::Done,
    };
    iter.fold(first, |acc, next| {
        ProofTactic::Alt(Heap::new(acc), Heap::new(next))
    })
}

// ---------------------------------------------------------------------------
// Calculation chain verification
// ---------------------------------------------------------------------------

/// Convert a `CalcRelation` to the SMT-level lemma name that justifies
/// transitivity for that relation.
fn calc_relation_name(rel: &CalcRelation) -> &'static str {
    match rel {
        CalcRelation::Eq => "eq",
        CalcRelation::Ne => "ne",
        CalcRelation::Lt => "lt",
        CalcRelation::Le => "le",
        CalcRelation::Gt => "gt",
        CalcRelation::Ge => "ge",
        CalcRelation::Implies => "implies",
        CalcRelation::Iff => "iff",
        CalcRelation::Subset => "subset",
        CalcRelation::Superset => "superset",
        CalcRelation::Divides => "divides",
        CalcRelation::Congruent => "congruent",
    }
}

/// Verify a calculation chain by checking each step's justification.
///
/// A calc block like:
/// ```text
/// calc {
///     a
///       = b  by h1
///       = c  by h2
/// }
/// ```
/// produces subgoals `a = b` (justified by `h1`) and `b = c` (justified by `h2`).
/// The overall conclusion `a = c` follows by transitivity.
fn verify_calc_chain(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    chain: &CalculationChain,
    hypotheses: &List<Expr>,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut verified_steps = List::new();
    let mut current_expr = chain.start.as_ref().clone();

    for step in &chain.steps {
        let step_start = Instant::now();

        // Build the relational proposition: current_expr <rel> target
        let proposition = build_relation_expr(
            &current_expr,
            &step.target,
            &step.relation,
            step.span,
        );

        // Create a goal with current hypotheses
        let goal = ProofGoal::with_hypotheses(proposition, hypotheses.clone());

        // Convert the justification tactic and execute it
        let tactic = convert_tactic(&step.justification);
        let sub_goals = engine.execute_tactic(&tactic, &goal).map_err(|e| {
            ProofVerificationError::CalcStepFailed {
                from: format_expr(&current_expr),
                to: format_expr(&step.target),
                relation: calc_relation_name(&step.relation).into(),
                reason: format!("{}", e).into(),
            }
        })?;

        // All subgoals produced by the tactic must be dischargeable.
        // Route through the `Auto` pipeline (trivial-close → structural
        // → SMT fallback) so calc-step residuals see the full automated
        // proof ladder, not just hint-based search.
        discharge_subgoals(engine, smt_ctx, &sub_goals, "calc")?;

        verified_steps.push(VerifiedStep {
            description: Text::from(format!(
                "calc: {} {} {}",
                format_expr(&current_expr),
                calc_relation_name(&step.relation),
                format_expr(&step.target),
            )),
            tactic_used: format!("{:?}", step.justification).into(),
            duration: step_start.elapsed(),
        });

        // Advance the chain
        current_expr = step.target.as_ref().clone();
    }

    Ok(verified_steps)
}

/// Construct a binary relation expression from two sides and a relation.
///
/// For equality this produces `lhs == rhs`; for ordering relations the
/// corresponding binary operator is used. Relations without a direct AST
/// binary operator (e.g. Divides, Congruent) are encoded as named function
/// calls that the SMT layer can interpret.
fn build_relation_expr(
    lhs: &Expr,
    rhs: &Expr,
    relation: &CalcRelation,
    span: verum_ast::Span,
) -> Expr {
    use verum_ast::expr::BinOp;

    let op = match relation {
        CalcRelation::Eq => Some(BinOp::Eq),
        CalcRelation::Ne => Some(BinOp::Ne),
        CalcRelation::Lt => Some(BinOp::Lt),
        CalcRelation::Le => Some(BinOp::Le),
        CalcRelation::Gt => Some(BinOp::Gt),
        CalcRelation::Ge => Some(BinOp::Ge),
        // These don't have direct BinOp equivalents; fall through to function call encoding.
        CalcRelation::Implies
        | CalcRelation::Iff
        | CalcRelation::Subset
        | CalcRelation::Superset
        | CalcRelation::Divides
        | CalcRelation::Congruent => None,
    };

    if let Some(bin_op) = op {
        Expr::new(
            ExprKind::Binary {
                op: bin_op,
                left: Heap::new(lhs.clone()),
                right: Heap::new(rhs.clone()),
            },
            span,
        )
    } else {
        // Encode as a named call: `<relation>(lhs, rhs)`
        let rel_name = calc_relation_name(relation);
        let mut path_segments = List::new();
        path_segments.push(verum_ast::PathSegment::Name(Ident::new(
            Text::from(rel_name),
            span,
        )));
        let func_expr = Expr::new(
            ExprKind::Path(verum_ast::Path::new(path_segments, span)),
            span,
        );
        let mut call_args = List::new();
        call_args.push(lhs.clone());
        call_args.push(rhs.clone());
        Expr::new(
            ExprKind::Call {
                func: Heap::new(func_expr),
                type_args: List::new(),
                args: call_args,
            },
            span,
        )
    }
}

// ---------------------------------------------------------------------------
// Proof method dispatch
// ---------------------------------------------------------------------------

/// Verify a `ProofMethod` (induction, cases, contradiction) against a goal.
fn verify_proof_method(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    method: &ProofMethod,
    goal: &ProofGoal,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    match method {
        ProofMethod::Induction { on, cases } => {
            verify_induction(engine, smt_ctx, on, cases, goal, /* strong */ false)
        }

        ProofMethod::StrongInduction { on, cases } => {
            verify_induction(
                engine,
                smt_ctx,
                &Maybe::Some(on.clone()),
                cases,
                goal,
                /* strong */ true,
            )
        }

        ProofMethod::WellFoundedInduction { relation, on, cases } => {
            let mut steps = List::new();
            let step_start = Instant::now();

            // Apply well-founded induction tactic
            let tactic = ProofTactic::WellFoundedInduction {
                var: on.name.clone(),
                relation: format_expr(relation),
            };
            let sub_goals = engine.execute_tactic(&tactic, goal).map_err(|e| {
                ProofVerificationError::MethodFailed {
                    method: "well-founded induction".into(),
                    reason: format!("{}", e).into(),
                }
            })?;

            steps.push(VerifiedStep {
                description: Text::from(format!(
                    "well-founded induction on '{}' with relation",
                    on.name
                )),
                tactic_used: Text::from("wf_induction"),
                duration: step_start.elapsed(),
            });

            // Verify each case
            let case_steps = verify_proof_cases(engine, smt_ctx, cases, &sub_goals)?;
            steps.extend(case_steps);
            Ok(steps)
        }

        ProofMethod::Cases { on, cases } => {
            let mut steps = List::new();
            let step_start = Instant::now();

            // When the `by cases` method has no explicit scrutinee (the
            // parser leaves an empty tuple placeholder) we fall back to
            // the goal's disjunction structure: split `A || B || …` into
            // one subgoal per disjunct. This matches the idiomatic usage
            //     proof by cases {
            //         case x >= 0 => { … }
            //         case x < 0  => { … }
            //     }
            // on a theorem like `ensures x >= 0 || x < 0`, where there
            // *is* no hypothesis to case-analyse — the user is partitioning
            // the disjunctive goal itself.
            let is_empty_scrutinee = matches!(
                &on.kind,
                ExprKind::Tuple(items) if items.is_empty()
            );

            // Try closing the whole goal outright with `Auto` first.
            // A classical disjunctive tautology like `x >= 0 || x < 0`
            // or `b == true || b == false` needs no real case analysis
            // — the SMT decides it directly. Covers both the empty-
            // scrutinee and the `on <param>` surfaces: in practice
            // users write the case block as documentation of the
            // intended case structure even when the ambient goal is
            // a tautology that auto can dispatch outright. Only if
            // Auto produces further subgoals do we fall through to
            // the explicit Split / CasesOn dispatch below.
            if let Ok(auto_sub) = engine.execute_tactic(&ProofTactic::Auto, goal) {
                if auto_sub.is_empty() {
                    steps.push(VerifiedStep {
                        description: Text::from("cases (goal closed by auto)"),
                        tactic_used: Text::from("cases→auto"),
                        duration: step_start.elapsed(),
                    });
                    return Ok(steps);
                }
            }

            let (tactic, tactic_label) = if is_empty_scrutinee {
                (ProofTactic::Split, "cases (split goal)")
            } else {
                (
                    ProofTactic::CasesOn {
                        hypothesis: format_expr(on),
                    },
                    "cases (on scrutinee)",
                )
            };

            let sub_goals = engine.execute_tactic(&tactic, goal).map_err(|e| {
                ProofVerificationError::MethodFailed {
                    method: "cases".into(),
                    reason: format!("{}", e).into(),
                }
            })?;

            steps.push(VerifiedStep {
                description: Text::from(format!(
                    "{} on {}",
                    tactic_label,
                    if is_empty_scrutinee {
                        Text::from("goal disjunction")
                    } else {
                        format_expr(on)
                    }
                )),
                tactic_used: Text::from(tactic_label),
                duration: step_start.elapsed(),
            });

            let case_steps = verify_proof_cases(engine, smt_ctx, cases, &sub_goals)?;
            steps.extend(case_steps);
            Ok(steps)
        }

        ProofMethod::Contradiction {
            assumption,
            proof,
        } => verify_contradiction(engine, smt_ctx, assumption, proof, goal),
    }
}

/// Verify induction (regular or strong) by applying the induction tactic and
/// then verifying each case branch.
fn verify_induction(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    on: &Maybe<Ident>,
    cases: &List<ProofCase>,
    goal: &ProofGoal,
    strong: bool,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut steps = List::new();
    let step_start = Instant::now();

    // Auto-first: if the SMT solver can close the claim outright
    // (e.g. `n * 2 >= 0` given `n >= 0`) there's no need to run the
    // full case-split machinery. Many textbook induction-shaped
    // goals in L3 specs are actually decidable arithmetic; the
    // induction block serves as exposition of *why* the claim holds,
    // not as the only way to check it. Same pattern as `by cases`.
    if let Ok(auto_sub) = engine.execute_tactic(&ProofTactic::Auto, goal) {
        if auto_sub.is_empty() {
            let label = if strong { "strong induction" } else { "induction" };
            steps.push(VerifiedStep {
                description: Text::from(format!("{label} (goal closed by auto)")),
                tactic_used: Text::from(format!("{label}→auto")),
                duration: step_start.elapsed(),
            });
            return Ok(steps);
        }
    }

    // Determine the variable name (auto-detect if None)
    let var_name: Text = match on {
        Maybe::Some(ident) => ident.name.clone(),
        Maybe::None => Text::from("_auto"),
    };

    let tactic = if strong {
        ProofTactic::StrongInduction {
            var: var_name.clone(),
        }
    } else {
        ProofTactic::Induction {
            var: var_name.clone(),
        }
    };

    let sub_goals = engine.execute_tactic(&tactic, goal).map_err(|e| {
        let kind = if strong {
            "strong induction"
        } else {
            "induction"
        };
        ProofVerificationError::MethodFailed {
            method: kind.into(),
            reason: format!("{}", e).into(),
        }
    })?;

    let kind_label = if strong { "strong induction" } else { "induction" };
    steps.push(VerifiedStep {
        description: Text::from(format!("{} on '{}'", kind_label, var_name)),
        tactic_used: Text::from(kind_label),
        duration: step_start.elapsed(),
    });

    // Verify each case with InductionCase context so `have IH: …;`
    // without an explicit `by` clause is admitted as the IH.
    let case_steps = verify_proof_cases_ctx(
        engine, smt_ctx, cases, &sub_goals, StepContext::InductionCase,
    )?;
    steps.extend(case_steps);
    Ok(steps)
}

/// Verify a proof by contradiction: assume the negation, then verify the
/// steps derive `False`.
fn verify_contradiction(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    assumption: &Ident,
    proof_steps: &List<ProofStep>,
    goal: &ProofGoal,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut steps = List::new();
    let step_start = Instant::now();

    // Introduce the negation as a hypothesis
    let intro_tactic = ProofTactic::IntroNamed {
        names: {
            let mut ns = List::new();
            ns.push(assumption.name.clone());
            ns
        },
    };
    let sub_goals = engine
        .execute_tactic(&intro_tactic, goal)
        .map_err(|e| ProofVerificationError::MethodFailed {
            method: "contradiction (intro)".into(),
            reason: format!("{}", e).into(),
        })?;

    steps.push(VerifiedStep {
        description: Text::from(format!(
            "assume negation as '{}'",
            assumption.name
        )),
        tactic_used: Text::from("intro (contradiction)"),
        duration: step_start.elapsed(),
    });

    // Verify the inner proof steps against the resulting goals, passing
    // along accumulated hypotheses.
    let inner_hypotheses: List<Expr> = if let Some(sg) = sub_goals.first() {
        sg.hypotheses.clone()
    } else {
        goal.hypotheses.clone()
    };

    let inner_steps = verify_proof_steps(engine, smt_ctx, proof_steps, &inner_hypotheses)?;
    steps.extend(inner_steps);

    // After the steps, the goal should be `False`; close with contradiction
    let contra_start = Instant::now();
    let false_goal = if let Some(sg) = sub_goals.first() {
        sg.clone()
    } else {
        goal.clone()
    };
    engine
        .execute_tactic(&ProofTactic::Contradiction, &false_goal)
        .or_else(|_| engine.execute_tactic(&ProofTactic::Exfalso, &false_goal))
        .map_err(|e| ProofVerificationError::MethodFailed {
            method: "contradiction (close)".into(),
            reason: format!("{}", e).into(),
        })?;

    steps.push(VerifiedStep {
        description: Text::from("derive contradiction"),
        tactic_used: Text::from("contradiction"),
        duration: contra_start.elapsed(),
    });

    Ok(steps)
}

/// Verify each branch of a case proof against the subgoals generated by the
/// case split tactic.
fn verify_proof_cases(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    cases: &List<ProofCase>,
    sub_goals: &List<ProofGoal>,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    verify_proof_cases_ctx(engine, smt_ctx, cases, sub_goals, StepContext::CaseBody)
}

fn verify_proof_cases_ctx(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    cases: &List<ProofCase>,
    sub_goals: &List<ProofGoal>,
    ctx_kind: StepContext,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut steps = List::new();

    // Match cases to subgoals positionally. If there are more cases than
    // subgoals (or vice versa), we still verify what we can and error on
    // unmatched subgoals.
    for (i, case) in cases.iter().enumerate() {
        let case_start = Instant::now();

        // Determine the goal for this case
        let case_goal = if i < sub_goals.len() {
            sub_goals[i].clone()
        } else {
            // Extra case beyond subgoals — still verify the steps
            // using a synthetic goal from the case pattern
            ProofGoal::new(Expr::new(
                ExprKind::Literal(verum_ast::Literal::bool(true, case.span)),
                case.span,
            ))
        };

        // Thread the case's condition expression as a hypothesis into
        // the body. For `case a >= b => { ... }` the parser stored the
        // expression `a >= b` in `case.condition`; inside that case's
        // proof body it's a known fact. Without this, the verifier
        // treated the case's condition as irrelevant and decidable
        // arithmetic like `max(a, b) == a` (under `a >= b`) failed
        // even though the SMT could discharge it in microseconds given
        // the hypothesis.
        let mut case_hypotheses = case_goal.hypotheses.clone();
        if let Maybe::Some(cond) = &case.condition {
            case_hypotheses.push(cond.as_ref().clone());
        }

        // Verify the proof steps within this case with the right
        // context kind so `have` steps are admitted only when the
        // parent is an induction case (the IH surface).
        let mut acc = case_hypotheses.clone();
        let inner_steps = verify_proof_steps_accumulating_ctx(
            engine, smt_ctx, &case.proof, &mut acc, ctx_kind,
        )?;

        steps.push(VerifiedStep {
            description: Text::from(format!("case {}: pattern {:?}", i + 1, case.pattern)),
            tactic_used: Text::from("case"),
            duration: case_start.elapsed(),
        });
        steps.extend(inner_steps);
    }

    // Check for subgoals not covered by any case
    if sub_goals.len() > cases.len() {
        let uncovered = sub_goals.len() - cases.len();
        return Err(ProofVerificationError::IncompleteCases {
            expected: sub_goals.len(),
            provided: cases.len(),
            uncovered_count: uncovered,
        });
    }

    Ok(steps)
}

// ---------------------------------------------------------------------------
// Structured proof step verification
// ---------------------------------------------------------------------------

/// Verify a sequence of `ProofStep`s, threading hypotheses through.
///
/// Each step may introduce new hypotheses (via `Have`, `Obtain`, `Let`) or
/// discharge intermediate goals (via `Show`, `Suffices`, `Tactic`). The
/// function tracks a mutable hypothesis list that grows as the proof
/// progresses.
fn verify_proof_steps(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    steps: &List<ProofStep>,
    initial_hypotheses: &List<Expr>,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut accumulated = initial_hypotheses.clone();
    verify_proof_steps_accumulating_ctx(engine, smt_ctx, steps, &mut accumulated, StepContext::TopLevel)
}

/// Context in which a step sequence is being verified. Admitted
/// `have` statements (no justification) are only sound inside an
/// inductive case body — there the missing justification is the
/// induction hypothesis. Anywhere else an un-justified `have` is a
/// soundness hole and must be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepContext {
    /// Root-level proof body. Un-justified `have` is rejected.
    TopLevel,
    /// Inside a `proof by induction { case … => … }` body.
    /// Un-justified `have` is admitted as the induction hypothesis.
    InductionCase,
    /// Inside a `proof by cases { case … => … }` body. Un-justified
    /// `have` is rejected (cases-bodies have access to the case
    /// condition but no additional axioms).
    CaseBody,
    /// Inside a `proof by contradiction` body. Un-justified `have`
    /// is rejected.
    Contradiction,
}

/// Inner worker for [`verify_proof_steps`]. Accumulates intermediate
/// `have` / `show` / `let` / `suffices` / `obtain` propositions into
/// the caller's hypothesis list so the outer closer tactic can see
/// them. The outer wrapper drops the mutated list; structured-proof
/// verification in `verify_structured_proof` calls this worker
/// directly to reuse the extended context when discharging the final
/// goal.
fn verify_proof_steps_accumulating(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    steps: &List<ProofStep>,
    hypotheses: &mut List<Expr>,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    verify_proof_steps_accumulating_ctx(
        engine,
        smt_ctx,
        steps,
        hypotheses,
        StepContext::TopLevel,
    )
}

fn verify_proof_steps_accumulating_ctx(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    steps: &List<ProofStep>,
    hypotheses: &mut List<Expr>,
    ctx_kind: StepContext,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let mut verified = List::new();

    for step in steps {
        let step_start = Instant::now();

        match &step.kind {
            // ---- Have: introduce a local hypothesis ----
            ProofStepKind::Have {
                name,
                proposition,
                justification,
            } => {
                let goal =
                    ProofGoal::with_hypotheses(proposition.as_ref().clone(), hypotheses.clone());
                let tactic = convert_tactic(justification);
                let exec_result = engine.execute_tactic(&tactic, &goal);

                // `have NAME: P;` without an explicit `by CLAUSE` is
                // the idiomatic induction-hypothesis form: inside
                // `case succ(k) => ...` the IH for `k` is available
                // without needing to be proved again — it's what
                // induction provides. The parser sugars the missing
                // justification to `TacticExpr::Trivial`, so the
                // check for "implicit IH" is `justification is
                // Trivial AND P is not itself trivially closable".
                // When exec_result succeeds, treat as normal Have;
                // when it fails under an implicit Trivial, admit the
                // proposition as an axiomatic hypothesis (with an
                // admit-flag in the tactic_used name so the
                // certificate still tracks it).
                let implicit_trivial = matches!(
                    justification,
                    verum_ast::decl::TacticExpr::Trivial
                );
                let mut admitted = false;
                match exec_result {
                    Ok(sub_goals) => {
                        discharge_subgoals(
                            engine,
                            smt_ctx,
                            &sub_goals,
                            &format!("have {}", name.name),
                        )?;
                    }
                    Err(e) => {
                        // Only admit un-justified `have` inside an
                        // inductive case body — that's the IH
                        // surface. Anywhere else (top-level,
                        // cases body, contradiction body) the
                        // unjustified hypothesis is a soundness
                        // hole and must be rejected.
                        if implicit_trivial && ctx_kind == StepContext::InductionCase {
                            admitted = true;
                        } else {
                            return Err(ProofVerificationError::StepFailed {
                                step: format!("have {}", name.name).into(),
                                reason: format!("{}", e).into(),
                            });
                        }
                    }
                }

                // Add the proposition as a hypothesis for subsequent steps
                hypotheses.push(proposition.as_ref().clone());

                let tactic_label = if admitted {
                    Text::from(format!("admitted(implicit IH: {:?})", justification))
                } else {
                    format!("{:?}", justification).into()
                };

                verified.push(VerifiedStep {
                    description: Text::from(format!(
                        "have {}: {}",
                        name.name,
                        format_expr(proposition)
                    )),
                    tactic_used: tactic_label,
                    duration: step_start.elapsed(),
                });
            }

            // ---- Show: verify an intermediate result ----
            ProofStepKind::Show {
                proposition,
                justification,
            } => {
                let goal =
                    ProofGoal::with_hypotheses(proposition.as_ref().clone(), hypotheses.clone());
                let tactic = convert_tactic(justification);
                let sub_goals =
                    engine
                        .execute_tactic(&tactic, &goal)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: Text::from("show"),
                            reason: format!("{}", e).into(),
                        })?;

                discharge_subgoals(engine, smt_ctx, &sub_goals, "show")?;

                // A shown proposition also becomes a hypothesis
                hypotheses.push(proposition.as_ref().clone());

                verified.push(VerifiedStep {
                    description: Text::from(format!("show {}", format_expr(proposition))),
                    tactic_used: format!("{:?}", justification).into(),
                    duration: step_start.elapsed(),
                });
            }

            // ---- Suffices: reduce the current goal ----
            ProofStepKind::Suffices {
                proposition,
                justification,
            } => {
                // "suffices to show P" means: if we prove P and P implies the
                // goal, then the goal follows. We verify the justification
                // establishes the implication.
                let goal =
                    ProofGoal::with_hypotheses(proposition.as_ref().clone(), hypotheses.clone());
                let tactic = convert_tactic(justification);
                let sub_goals =
                    engine
                        .execute_tactic(&tactic, &goal)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: Text::from("suffices"),
                            reason: format!("{}", e).into(),
                        })?;

                discharge_subgoals(engine, smt_ctx, &sub_goals, "suffices")?;

                // The sufficient condition replaces the active goal context;
                // add it as a hypothesis for subsequent steps.
                hypotheses.push(proposition.as_ref().clone());

                verified.push(VerifiedStep {
                    description: Text::from(format!(
                        "suffices {}",
                        format_expr(proposition)
                    )),
                    tactic_used: format!("{:?}", justification).into(),
                    duration: step_start.elapsed(),
                });
            }

            // ---- Let: bind a local definition ----
            ProofStepKind::Let { pattern: _, value } => {
                // Let bindings in proofs introduce definitional equalities.
                // We add the value expression as a hypothesis.
                hypotheses.push(value.as_ref().clone());

                verified.push(VerifiedStep {
                    description: Text::from(format!("let := {}", format_expr(value))),
                    tactic_used: Text::from("let"),
                    duration: step_start.elapsed(),
                });
            }

            // ---- Obtain: extract witness from existential ----
            ProofStepKind::Obtain { pattern, from } => {
                // `obtain x from h` destructs an existential hypothesis `h`
                // into a witness `x` and its proof. We execute a destruct
                // tactic on the `from` expression.
                let from_text = format_expr(from);
                let goal =
                    ProofGoal::with_hypotheses(from.as_ref().clone(), hypotheses.clone());
                let tactic = ProofTactic::Destruct {
                    hypothesis: from_text.clone(),
                };

                let sub_goals =
                    engine
                        .execute_tactic(&tactic, &goal)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: Text::from("obtain"),
                            reason: format!("{}", e).into(),
                        })?;

                // Add the destructed components as hypotheses
                for sg in &sub_goals {
                    hypotheses.push(sg.goal.clone());
                }

                verified.push(VerifiedStep {
                    description: Text::from(format!(
                        "obtain {:?} from {}",
                        pattern, from_text
                    )),
                    tactic_used: Text::from("obtain/destruct"),
                    duration: step_start.elapsed(),
                });
            }

            // ---- Calc: equational reasoning chain ----
            ProofStepKind::Calc(chain) => {
                let calc_steps = verify_calc_chain(engine, smt_ctx, chain, &hypotheses)?;
                verified.extend(calc_steps);
            }

            // ---- Cases: case analysis within structured proof ----
            ProofStepKind::Cases { scrutinee, cases } => {
                let goal =
                    ProofGoal::with_hypotheses(scrutinee.as_ref().clone(), hypotheses.clone());
                let tactic = ProofTactic::CasesOn {
                    hypothesis: format_expr(scrutinee),
                };
                let sub_goals =
                    engine
                        .execute_tactic(&tactic, &goal)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: Text::from("cases"),
                            reason: format!("{}", e).into(),
                        })?;

                let case_steps = verify_proof_cases(engine, smt_ctx, cases, &sub_goals)?;
                verified.extend(case_steps);
            }

            // ---- Focus: work on a specific subgoal ----
            ProofStepKind::Focus {
                goal_index: _,
                steps: inner_steps,
            } => {
                let inner =
                    verify_proof_steps(engine, smt_ctx, inner_steps, &hypotheses)?;
                verified.extend(inner);
            }

            // ---- Tactic: bare tactic application ----
            ProofStepKind::Tactic(tactic_expr) => {
                // A standalone tactic step operates on whatever the current
                // "ambient" goal is. Since structured proofs thread goals
                // implicitly, we create a trivial goal and execute the tactic
                // for its side-effects on the hypothesis context.
                let tactic = convert_tactic(tactic_expr);
                let trivial_goal = ProofGoal::with_hypotheses(
                    Expr::new(
                        ExprKind::Literal(verum_ast::Literal::bool(true, step.span)),
                        step.span,
                    ),
                    hypotheses.clone(),
                );

                // Execute the tactic; we don't require it to close the goal
                // (it may just transform hypotheses or simplify).
                let _ = engine.execute_tactic(&tactic, &trivial_goal);

                verified.push(VerifiedStep {
                    description: Text::from(format!("tactic {:?}", tactic_expr)),
                    tactic_used: format!("{:?}", tactic_expr).into(),
                    duration: step_start.elapsed(),
                });
            }
        }
    }

    Ok(verified)
}

/// Attempt to discharge a list of subgoals via `auto_prove`. Returns an error
/// if any subgoal cannot be closed.
fn discharge_subgoals(
    engine: &mut ProofSearchEngine,
    _smt_ctx: &Context,
    sub_goals: &List<ProofGoal>,
    context_label: &str,
) -> Result<(), ProofVerificationError> {
    // Recursively close each subgoal with the `Auto` tactic. Using
    // `execute_tactic(Auto, ...)` — not the hint-based `auto_prove` — is
    // load-bearing: `try_auto` has the trivial-true fast path, the
    // structural pass, and the SMT fallback wired in, and it sees the
    // subgoal's full hypothesis context (essential when the parent
    // tactic was a conjunctive `Split` that copied the hypotheses into
    // each conjunct). `auto_prove` by contrast runs hint-based
    // iterative-deepening search with no SMT fallback, so it
    // systematically failed on arithmetic subgoals produced by splits.
    for (i, sg) in sub_goals.iter().enumerate() {
        let result = engine.execute_tactic(&ProofTactic::Auto, sg);
        match result {
            Ok(more) if more.is_empty() => {
                // Closed outright.
            }
            Ok(more) => {
                // Auto produced further subgoals (e.g. a nested split
                // on a conjunct). Recurse.
                discharge_subgoals(engine, _smt_ctx, &more, context_label)?;
            }
            Err(e) => {
                return Err(ProofVerificationError::SubgoalFailed {
                    goal: Text::from(format!(
                        "{} subgoal {}: {:?}",
                        context_label,
                        i + 1,
                        sg.goal
                    )),
                    reason: format!("{}", e).into(),
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Verify the proof body of a theorem declaration.
///
/// This is the primary entry point for proof verification. It dispatches on
/// the `ProofBody` variant:
///
/// - **Term**: The proof term is treated as an expression whose type must match
///   the proposition. Since full dependent type checking is beyond this module's
///   scope, we delegate to the SMT engine to verify the proposition directly,
///   using the term as a hint.
///
/// - **Tactic**: A single tactic expression is converted to `ProofTactic` and
///   executed against the proposition as a goal.
///
/// - **Structured**: Each `ProofStep` is verified in sequence, threading
///   hypotheses through. A final conclusion tactic (if present) closes the
///   remaining goal.
///
/// - **ByMethod**: Dispatches to induction, case analysis, or contradiction
///   handlers that generate and verify the appropriate subgoals.
///
/// # Arguments
///
/// * `engine` - The proof search engine (may be reused across theorems).
/// * `smt_ctx` - SMT context for formula translation.
/// * `theorem` - The theorem declaration containing the proof body.
///
/// # Returns
///
/// `ProofVerificationResult::Verified` with a certificate on success, or
/// `ProofVerificationResult::Failed` with the list of unproved subgoals.
pub fn verify_proof_body(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    theorem: &TheoremDecl,
) -> ProofVerificationResult {
    verify_proof_body_with_aliases(engine, smt_ctx, theorem, &std::collections::HashMap::new())
}

/// Same as [`verify_proof_body`] but takes a precomputed alias map so nominal
/// refinement-type aliases (e.g. `type FanoDim is Dimension { self == 7 }`)
/// can contribute predicates to the hypothesis list. The map is keyed by the
/// alias's unqualified name and stores the already-composed chain of
/// refinement predicates rooted in an implicit `self` binder.
///
/// Callers that don't care about nominal resolution use [`verify_proof_body`]
/// above; it passes an empty map and the behaviour degrades to "inline
/// refinements only", which is the pre-existing contract.
pub fn verify_proof_body_with_aliases(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    theorem: &TheoremDecl,
    alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
) -> ProofVerificationResult {
    let proof_start = Instant::now();
    let theorem_name = theorem.name.name.clone();

    // Extract the proof body; axioms (no proof) are accepted elsewhere
    let proof_body = match &theorem.proof {
        Maybe::Some(body) => body,
        Maybe::None => {
            // No proof body — treat as axiom, nothing to verify here
            return ProofVerificationResult::Verified(ProofCertificate {
                theorem_name,
                steps: List::new(),
                total_duration: proof_start.elapsed(),
                has_incomplete_steps: false,
            });
        }
    };

    // Build the primary goal from the theorem's proposition and requires clauses.
    //
    // Proof-body semantics for `theorem t(..) -> Bool ensures E proof by …`:
    // the proof body is evidence for `E`, not for the shape of the returned
    // `Bool`. Concretely the declaration means "there exists a proof of `E`,
    // and the theorem's returned `Bool` is a witness of that proof — so
    // `result = true`". Elaboration bakes this convention in by substituting
    // every free occurrence of the reserved `result` identifier with the
    // boolean literal `true` before the VC reaches SMT. Without this step,
    // ensures like `result == (n == 7)` contain an unbound `result` and the
    // translator just encodes it as a fresh uninterpreted integer, making
    // every `-> Bool` theorem with a `proof by …` body unprovable for
    // reasons entirely unrelated to the mathematical content of the claim.
    let raw_proposition = theorem.proposition.as_ref().clone();
    let proposition = substitute_result_with_true(&raw_proposition);
    let mut hypotheses: List<Expr> = theorem
        .requires
        .iter()
        .map(|e| substitute_result_with_true(e))
        .collect();

    // Inline refinement hypotheses.
    //
    // For every theorem parameter whose *inline* type is a refinement — i.e.
    // `n: Int { self == 7 }` (Rule 1) or `n: Int where |x| x > 0` (Rule 2) or
    // `x: Int where x > 0` (Rule 3, Sigma) — the predicate is a fact about
    // that parameter that the proof body is allowed to rely on. We rewrite
    // the predicate by substituting the binder (`self` / `it` / the explicit
    // name) with the actual parameter identifier and push it into the
    // hypothesis set so the SMT sees the same constraint the type system is
    // enforcing at call sites.
    //
    // Nominal refinements (`n: FanoDim` where `FanoDim is Dimension { self == 7 }`)
    // are *not* resolved here — that requires the type-alias chain from the
    // type registry and is tracked as a separate follow-up. The conservative
    // behaviour (omit the nominal refinement) is sound: the proof obligation
    // is merely harder, not wrong.
    for hyp in refinement_hypotheses_from_params(&theorem.params, alias_map) {
        hypotheses.push(hyp);
    }

    // Propositional-witness hypotheses. Curry-Howard reading: when a
    // theorem is quantified over `<P: Prop>` and takes a parameter
    // `p: P`, the parameter's existence is a witness that `P` holds.
    // The SMT translator sees `P` as a Bool-kinded uninterpreted
    // constant (via the Bool-coercion arm for `&&`, `||`, `==` that
    // involve it), so injecting `P` as a hypothesis tells the solver
    // the proposition is true — exactly what identity-style proofs
    // over Prop generics need to close.
    for hyp in propositional_witness_hypotheses(theorem) {
        hypotheses.push(hyp);
    }

    // Variant-exhaustiveness hypotheses. For every parameter typed
    // as a declared variant type `T is A | B | C;`, add the
    // disjunctive claim `p == T.A || p == T.B || p == T.C`. Paired
    // with the pairwise-disjointness axioms this encodes the
    // complete ADT semantics Z3 needs for variant-indexed
    // reasoning. The registry lives on the engine — populated by
    // `verify_cmd::verify_module` from the module's `TypeDecl`s.
    for hyp in variant_exhaustiveness_hypotheses(theorem, engine.variant_map()) {
        hypotheses.push(hyp);
    }

    let primary_goal = ProofGoal::with_hypotheses(proposition.clone(), hypotheses.clone());

    match proof_body {
        // ----------------------------------------------------------------
        // Term proof (Curry-Howard)
        // ----------------------------------------------------------------
        ProofBody::Term(proof_term) => {
            let step_start = Instant::now();

            // In a full dependent type system the proof term's type would be
            // checked against the proposition. Here we approximate: use the
            // proof term expression as an `exact` hint, then fall back to
            // `auto_prove` if that fails.
            let exact_tactic = ProofTactic::Exact {
                term: format_expr(proof_term),
            };

            // execute_tactic returns Result<_, ProofError>, auto_prove returns
            // Result<_, VerificationError>. Unify to VerificationError.
            let result: Result<(), VerificationError> = engine
                .execute_tactic(&exact_tactic, &primary_goal)
                .map_err(|e| VerificationError::Unknown(format!("{}", e).into()))
                .and_then(|sub_goals| {
                    // If exact produces subgoals, try to discharge them
                    for sg in &sub_goals {
                        engine.auto_prove(smt_ctx, &sg.goal)?;
                    }
                    Ok(())
                })
                .or_else(|_: VerificationError| {
                    // Fall back to auto_prove on the proposition
                    engine.auto_prove(smt_ctx, &proposition).map(|_| ())
                });

            match result {
                Ok(()) => {
                    let mut steps = List::new();
                    steps.push(VerifiedStep {
                        description: Text::from(format!(
                            "proof term: {}",
                            format_expr(proof_term)
                        )),
                        tactic_used: Text::from("exact/auto"),
                        duration: step_start.elapsed(),
                    });
                    ProofVerificationResult::Verified(ProofCertificate {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        has_incomplete_steps: false,
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    let mut suggestions =
                        heuristic_suggestions(&proposition, &hypotheses, Maybe::None);
                    suggestions.extend(build_suggestions_from_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions,
                    });
                    ProofVerificationResult::Failed {
                        verified_steps: List::new(),
                        unproved,
                    }
                }
            }
        }

        // ----------------------------------------------------------------
        // Single tactic proof
        // ----------------------------------------------------------------
        ProofBody::Tactic(tactic_expr) => {
            let step_start = Instant::now();
            let tactic = convert_tactic(tactic_expr);

            let result = engine
                .execute_tactic(&tactic, &primary_goal)
                .map_err(ProofVerificationError::from)
                .and_then(|sub_goals| {
                    discharge_subgoals(engine, smt_ctx, &sub_goals, "tactic proof")?;
                    Ok(())
                });

            match result {
                Ok(()) => {
                    let has_incomplete = tactic_expr.is_unsafe();
                    let mut steps = List::new();
                    steps.push(VerifiedStep {
                        description: Text::from(format!("tactic: {:?}", tactic_expr)),
                        tactic_used: format!("{:?}", tactic_expr).into(),
                        duration: step_start.elapsed(),
                    });
                    ProofVerificationResult::Verified(ProofCertificate {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        has_incomplete_steps: has_incomplete,
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    let mut suggestions = heuristic_suggestions(
                        &proposition,
                        &hypotheses,
                        pv_error_tactic_name(&e),
                    );
                    suggestions.extend(build_suggestions_from_pv_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions,
                    });
                    ProofVerificationResult::Failed {
                        verified_steps: List::new(),
                        unproved,
                    }
                }
            }
        }

        // ----------------------------------------------------------------
        // Structured proof
        // ----------------------------------------------------------------
        ProofBody::Structured(structure) => {
            verify_structured_proof(engine, smt_ctx, structure, &primary_goal, &theorem_name, proof_start)
        }

        // ----------------------------------------------------------------
        // Proof by method (induction / cases / contradiction)
        // ----------------------------------------------------------------
        ProofBody::ByMethod(method) => {
            match verify_proof_method(engine, smt_ctx, method, &primary_goal) {
                Ok(steps) => {
                    let has_incomplete = steps.iter().any(|s| {
                        s.tactic_used.contains("admit") || s.tactic_used.contains("sorry")
                    });
                    ProofVerificationResult::Verified(ProofCertificate {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        has_incomplete_steps: has_incomplete,
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    let mut suggestions = heuristic_suggestions(
                        &proposition,
                        &hypotheses,
                        pv_error_tactic_name(&e),
                    );
                    suggestions.extend(build_suggestions_from_pv_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions,
                    });
                    ProofVerificationResult::Failed {
                        verified_steps: List::new(),
                        unproved,
                    }
                }
            }
        }
    }
}

/// Verify a structured proof with its sequence of steps and optional
/// conclusion tactic.
fn verify_structured_proof(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    structure: &ProofStructure,
    primary_goal: &ProofGoal,
    theorem_name: &Text,
    proof_start: Instant,
) -> ProofVerificationResult {
    let proposition = primary_goal.goal.clone();
    let mut accumulated_hypotheses = primary_goal.hypotheses.clone();

    // Verify each step, accumulating every intermediate proposition
    // (`have`, `show`, `suffices`, `let`, `obtain`) into the hypothesis
    // list so the implicit closer below can cite them.
    match verify_proof_steps_accumulating(
        engine,
        smt_ctx,
        &structure.steps,
        &mut accumulated_hypotheses,
    ) {
        Ok(mut verified_steps) => {
            let hypotheses = accumulated_hypotheses.clone();
            // Apply conclusion tactic if present
            if let Maybe::Some(conclusion) = &structure.conclusion {
                let conclusion_start = Instant::now();
                let tactic = convert_tactic(conclusion);

                // The conclusion tactic closes the original goal with
                // all hypotheses accumulated during step verification.
                let enriched_goal = ProofGoal::with_hypotheses(
                    proposition.clone(),
                    hypotheses.clone(),
                );

                match engine.execute_tactic(&tactic, &enriched_goal) {
                    Ok(sub_goals) => {
                        if let Err(e) =
                            discharge_subgoals(engine, smt_ctx, &sub_goals, "conclusion")
                        {
                            let mut unproved = List::new();
                            let mut suggestions = heuristic_suggestions(
                                &proposition,
                                &hypotheses,
                                pv_error_tactic_name(&e),
                            );
                            suggestions.extend(build_suggestions_from_pv_error(&e));
                            unproved.push(UnprovedSubgoal {
                                goal: format_expr(&proposition),
                                hypotheses: hypotheses
                                    .iter()
                                    .map(|h| format_expr(h))
                                    .collect(),
                                suggestions,
                            });
                            return ProofVerificationResult::Failed {
                                verified_steps,
                                unproved,
                            };
                        }

                        verified_steps.push(VerifiedStep {
                            description: Text::from("conclusion"),
                            tactic_used: format!("{:?}", conclusion).into(),
                            duration: conclusion_start.elapsed(),
                        });
                    }
                    Err(e) => {
                        let mut unproved = List::new();
                        let mut suggestions =
                            heuristic_suggestions(&proposition, &hypotheses, Maybe::None);
                        suggestions.extend(build_suggestions_from_proof_error(&e));
                        unproved.push(UnprovedSubgoal {
                            goal: format_expr(&proposition),
                            hypotheses: hypotheses
                                .iter()
                                .map(|h| format_expr(h))
                                .collect(),
                            suggestions,
                        });
                        return ProofVerificationResult::Failed {
                            verified_steps,
                            unproved,
                        };
                    }
                }
            } else {
                // No explicit conclusion — close the goal with `Auto`
                // applied to an enriched `ProofGoal` that carries every
                // hypothesis accumulated by the `have` / `show` steps
                // plus the theorem's own `requires`.
                let closer_goal = ProofGoal::with_hypotheses(
                    proposition.clone(),
                    hypotheses.clone(),
                );
                let closer_result = engine
                    .execute_tactic(&ProofTactic::Auto, &closer_goal)
                    .and_then(|sub_goals| {
                        if sub_goals.is_empty() {
                            Ok(())
                        } else {
                            // Auto opened further subgoals (nested
                            // conjunction split, etc.). Recurse.
                            discharge_subgoals(engine, smt_ctx, &sub_goals, "closer")
                                .map_err(|pe| ProofError::TacticFailed(
                                    format!("closer: {:?}", pe).into(),
                                ))
                        }
                    });
                if let Err(e) = closer_result {
                    let mut unproved = List::new();
                    let mut suggestions =
                        heuristic_suggestions(&proposition, &hypotheses, Maybe::None);
                    suggestions.extend(build_suggestions_from_proof_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions,
                    });
                    return ProofVerificationResult::Failed {
                        verified_steps,
                        unproved,
                    };
                }
            }

            let has_incomplete = verified_steps.iter().any(|s| {
                s.tactic_used.contains("admit") || s.tactic_used.contains("sorry")
            });

            ProofVerificationResult::Verified(ProofCertificate {
                theorem_name: theorem_name.clone(),
                steps: verified_steps,
                total_duration: proof_start.elapsed(),
                has_incomplete_steps: has_incomplete,
            })
        }
        Err(e) => {
            let mut unproved = List::new();
            let mut suggestions = heuristic_suggestions(
                &proposition,
                &accumulated_hypotheses,
                pv_error_tactic_name(&e),
            );
            suggestions.extend(build_suggestions_from_pv_error(&e));
            unproved.push(UnprovedSubgoal {
                goal: format_expr(&proposition),
                hypotheses: accumulated_hypotheses
                    .iter()
                    .map(|h| format_expr(h))
                    .collect(),
                suggestions,
            });
            ProofVerificationResult::Failed {
                verified_steps: List::new(),
                unproved,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Internal error type for proof verification, providing detailed context
/// about which step or method failed.
#[derive(Debug, Clone)]
pub enum ProofVerificationError {
    /// A tactic execution failed.
    TacticFailed {
        tactic: Text,
        reason: Text,
    },

    /// A proof step (have/show/suffices/obtain) failed.
    StepFailed {
        step: Text,
        reason: Text,
    },

    /// A calculation chain step could not be justified.
    CalcStepFailed {
        from: Text,
        to: Text,
        relation: Text,
        reason: Text,
    },

    /// A subgoal could not be discharged.
    SubgoalFailed {
        goal: Text,
        reason: Text,
    },

    /// A proof method (induction/cases/contradiction) failed.
    MethodFailed {
        method: Text,
        reason: Text,
    },

    /// Case analysis is incomplete (fewer cases than subgoals).
    IncompleteCases {
        expected: usize,
        provided: usize,
        uncovered_count: usize,
    },
}

impl std::fmt::Display for ProofVerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TacticFailed { tactic, reason } => {
                write!(f, "tactic '{}' failed: {}", tactic, reason)
            }
            Self::StepFailed { step, reason } => {
                write!(f, "proof step '{}' failed: {}", step, reason)
            }
            Self::CalcStepFailed {
                from,
                to,
                relation,
                reason,
            } => {
                write!(
                    f,
                    "calc step '{} {} {}' failed: {}",
                    from, relation, to, reason
                )
            }
            Self::SubgoalFailed { goal, reason } => {
                write!(f, "subgoal '{}' not discharged: {}", goal, reason)
            }
            Self::MethodFailed { method, reason } => {
                write!(f, "proof method '{}' failed: {}", method, reason)
            }
            Self::IncompleteCases {
                expected,
                provided,
                uncovered_count,
            } => {
                write!(
                    f,
                    "incomplete case analysis: {} cases expected, {} provided ({} uncovered)",
                    expected, provided, uncovered_count
                )
            }
        }
    }
}

impl std::error::Error for ProofVerificationError {}

impl From<ProofError> for ProofVerificationError {
    fn from(e: ProofError) -> Self {
        // Map each `ProofError` variant to the strongest structured
        // `ProofVerificationError` we can — in particular, extract
        // the tactic name from `TacticFailed(Text)` rather than
        // collapsing everything to a "(unknown)" string. This keeps
        // user-facing diagnostics actionable ("tactic 'auto' failed
        // on goal …") instead of opaque.
        match e {
            ProofError::TacticFailed(reason) => {
                // The reason often encodes the tactic name as
                // "tactic_name: message" or "'tactic_name' failed:
                // message". Parse heuristically so the downstream
                // diagnostic names the right tactic.
                let reason_str = reason.as_str();
                let (tactic, rest) = split_tactic_reason(reason_str);
                Self::TacticFailed {
                    tactic,
                    reason: rest,
                }
            }
            ProofError::SmtTimeout => Self::TacticFailed {
                tactic: Text::from("smt"),
                reason: Text::from("SMT solver timed out"),
            },
            ProofError::UnificationFailed(detail) => Self::TacticFailed {
                tactic: Text::from("apply"),
                reason: detail,
            },
            ProofError::NotInContext(name) => Self::TacticFailed {
                tactic: Text::from("assumption"),
                reason: Text::from(format!(
                    "hypothesis `{}` is not in scope",
                    name.as_str()
                )),
            },
            ProofError::NotEquality(detail) => Self::TacticFailed {
                tactic: Text::from("rewrite"),
                reason: Text::from(format!(
                    "target is not an equality: {}",
                    detail.as_str()
                )),
            },
            ProofError::InvalidProof(detail) => Self::TacticFailed {
                tactic: Text::from("proof"),
                reason: detail,
            },
        }
    }
}

/// Split a "'tactic': reason" or "tactic_name failed: reason"
/// formatted string into (tactic, rest). Falls back to a reasonable
/// default when the shape doesn't match.
fn split_tactic_reason(input: &str) -> (Text, Text) {
    // Pattern 1: 'name' - strip leading apostrophe-quoted token.
    if let Some(rest) = input.strip_prefix('\'') {
        if let Some((name, tail)) = rest.split_once('\'') {
            let tail = tail.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
            return (Text::from(name), Text::from(tail));
        }
    }
    // Pattern 2: bare leading identifier token up to the first space.
    if let Some((first, rest)) = input.split_once(char::is_whitespace) {
        if !first.is_empty()
            && first.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            let rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
            return (Text::from(first), Text::from(rest));
        }
    }
    // Fall-through: we couldn't identify the tactic name. Surface
    // the full message as the reason and a neutral marker as the
    // tactic so the diagnostic still carries useful content.
    (Text::from("tactic"), Text::from(input))
}

// ---------------------------------------------------------------------------
// Suggestion generation
// ---------------------------------------------------------------------------

/// Build suggestions from a `VerificationError`.
fn build_suggestions_from_error(err: &VerificationError) -> List<Text> {
    let mut suggestions = List::new();
    match err {
        VerificationError::CannotProve {
            suggestions: smt_suggs,
            ..
        } => {
            suggestions.extend(smt_suggs.clone());
        }
        VerificationError::Timeout { .. } => {
            suggestions.push(Text::from(
                "Try increasing the SMT timeout or simplifying the goal",
            ));
            suggestions.push(Text::from(
                "Consider breaking the proof into smaller lemmas",
            ));
        }
        VerificationError::Unknown(_) => {
            suggestions.push(Text::from("Try providing explicit proof steps"));
            suggestions.push(Text::from(
                "Consider using 'simp', 'ring', or 'omega' for decidable fragments",
            ));
        }
        VerificationError::Translation(_) => {
            suggestions.push(Text::from(
                "The goal could not be translated to SMT — check for unsupported constructs",
            ));
        }
        VerificationError::SolverError(_) => {
            suggestions.push(Text::from(
                "SMT solver encountered an internal error — try a different tactic",
            ));
        }
    }
    suggestions
}

/// Build suggestions from a `ProofVerificationError`.
fn build_suggestions_from_pv_error(err: &ProofVerificationError) -> List<Text> {
    let mut suggestions = List::new();
    match err {
        ProofVerificationError::TacticFailed { tactic, .. } => {
            suggestions.push(Text::from(format!(
                "Tactic '{}' failed — try 'simp', 'auto', or 'omega'",
                tactic
            )));
        }
        ProofVerificationError::StepFailed { step, .. } => {
            suggestions.push(Text::from(format!(
                "Step '{}' could not be justified — check the justification tactic",
                step
            )));
        }
        ProofVerificationError::CalcStepFailed {
            from, to, relation, ..
        } => {
            suggestions.push(Text::from(format!(
                "Could not verify {} {} {} — try adding intermediate steps",
                from, relation, to
            )));
        }
        ProofVerificationError::SubgoalFailed { .. } => {
            suggestions.push(Text::from(
                "A subgoal remained after tactic execution — try adding explicit proof steps",
            ));
        }
        ProofVerificationError::MethodFailed { method, .. } => {
            suggestions.push(Text::from(format!(
                "Proof method '{}' failed — verify the induction variable or case split is correct",
                method
            )));
        }
        ProofVerificationError::IncompleteCases {
            uncovered_count, ..
        } => {
            suggestions.push(Text::from(format!(
                "{} case(s) not covered — add missing case branches",
                uncovered_count
            )));
        }
    }
    suggestions
}

/// Build suggestions from a `ProofError` (from the SMT engine).
fn build_suggestions_from_proof_error(err: &ProofError) -> List<Text> {
    let mut suggestions = List::new();
    suggestions.push(Text::from(format!(
        "Proof engine error: {} — try a different tactic",
        err
    )));
    suggestions
}

/// Extract the failed-tactic name from a `ProofVerificationError`, if the
/// variant carries one. Used to feed the heuristic engine's
/// `exhausted` parameter so it will not re-suggest the tactic that
/// just failed.
fn pv_error_tactic_name(err: &ProofVerificationError) -> Maybe<&str> {
    match err {
        ProofVerificationError::TacticFailed { tactic, .. } => {
            Maybe::Some(tactic.as_str())
        }
        ProofVerificationError::MethodFailed { method, .. } => {
            Maybe::Some(method.as_str())
        }
        _ => Maybe::None,
    }
}

/// Build goal-aware tactic suggestions using the
/// `verum_verification::tactic_heuristics` shape-level heuristic engine.
///
/// This is the structured counterpart to `build_suggestions_from_*`:
/// instead of hard-coded prose, it inspects the goal expression and
/// already-tried tactic name and emits ranked suggestions
/// ("try `refl`", "try `split`", …) with a short rationale.
///
/// Empty result is fine — it signals "no shape-matching rule fires;
/// fall back to the error-kind prose suggestions above".
fn heuristic_suggestions(
    goal_expr: &Expr,
    hypothesis_exprs: &List<Expr>,
    exhausted_tactic: Maybe<&str>,
) -> List<Text> {
    let hypotheses: List<Hypothesis> = hypothesis_exprs
        .iter()
        .enumerate()
        .map(|(idx, h)| Hypothesis {
            name: Text::from(format!("h{}", idx)),
            proposition: Heap::new(h.clone()),
            ty: Maybe::None,
            source: verum_verification::tactic_evaluation::HypothesisSource::User,
        })
        .collect();

    let goal = Goal {
        id: 0,
        proposition: Heap::new(goal_expr.clone()),
        hypotheses,
        meta: GoalMetadata::default(),
    };

    let exhausted_vec: Vec<&str> = match exhausted_tactic {
        Maybe::Some(name) => vec![name],
        Maybe::None => vec![],
    };

    let ranked: Vec<TacticSuggestion> =
        suggest_next_tactics(&goal, &exhausted_vec);

    ranked
        .into_iter()
        .map(|s| {
            Text::from(format!(
                "try `{}` — {} ({:?} confidence)",
                s.tactic, s.rationale, s.confidence
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Refinement-type → hypothesis elaboration
// ---------------------------------------------------------------------------

/// For every parameter whose type is an inline refinement (`Refined` with the
/// implicit `self` / `it` binder, `Sigma` with an explicit binder), produce
/// the corresponding predicate with the binder substituted by the actual
/// parameter identifier. The resulting expressions are ready to be threaded
/// as hypotheses into the proof goal.
///
/// This is what lets the proof engine "see" the refinement: a theorem
/// `theorem t(n: Int { self >= 0 }) ensures ...` must carry `n >= 0` as a
/// hypothesis, not just as a refinement on the type. Type-checking still
/// enforces the refinement at call sites; this function merely threads it
/// through the verification layer where the goal solver lives.
/// Collect propositional-witness hypotheses for `<Q: Prop>` generics.
///
/// Rationale (Curry-Howard): a theorem quantified over a Prop-kinded
/// type variable `Q` that also takes a parameter `p: Q` is saying
/// "the caller supplies a proof of `Q`". Inside the body we can treat
/// `Q` as `true`. Before this elaboration, goals of shape
///
///     theorem id_trivial<P: Prop>(p: P): P { proof by auto }
///
/// died in the SMT with "Goal is not boolean" because `P` was a bare
/// Path with no context telling the solver that its value is fixed
/// to `true`. The fix threads `P` itself as a hypothesis (i.e. a
/// propositional fact), via the Bool-coercion the translator already
/// applies to bare identifiers under propositional operators.
pub fn propositional_witness_hypotheses(theorem: &TheoremDecl) -> Vec<Expr> {
    use verum_ast::decl::FunctionParamKind;
    use verum_ast::ty::{GenericParamKind, TypeBoundKind, TypeKind};

    // Collect names of generics bound by the `Prop` protocol.
    let mut prop_generic_names: std::collections::HashSet<Text> =
        std::collections::HashSet::new();
    for g in &theorem.generics {
        if let GenericParamKind::Type { name, bounds, .. } = &g.kind {
            for b in bounds {
                if let TypeBoundKind::Protocol(path) = &b.kind {
                    if path
                        .as_ident()
                        .map(|id| id.name.as_str() == "Prop")
                        .unwrap_or(false)
                    {
                        prop_generic_names.insert(name.name.clone());
                    }
                }
            }
        }
    }
    if prop_generic_names.is_empty() {
        return Vec::new();
    }

    // For every parameter whose type is one of those generic names,
    // emit the hypothesis `Q` (as a bare identifier Expr). The
    // translator's Bool-coercion will render it as a Bool constant;
    // asserting that constant in the solver means the Prop holds.
    let mut out: Vec<Expr> = Vec::new();
    let mut emitted: std::collections::HashSet<Text> = std::collections::HashSet::new();
    for p in &theorem.params {
        if let FunctionParamKind::Regular { ty, .. } = &p.kind {
            if let TypeKind::Path(path) = &ty.kind {
                if let Some(id) = path.as_ident() {
                    if prop_generic_names.contains(&id.name) && emitted.insert(id.name.clone()) {
                        out.push(Expr::new(
                            verum_ast::expr::ExprKind::Path(
                                verum_ast::ty::Path::single(id.clone()),
                            ),
                            ty.span,
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Emit exhaustiveness hypotheses for every parameter typed as a
/// declared variant type. Given a parameter `p: T` where
/// `T is A | B | C`, produce the expression
/// `p == T.A || p == T.B || p == T.C`.
///
/// Uses the variant registry `variant_map` (populated from the
/// module's `TypeDecl`s at `verify_module` time). Parameters typed
/// with non-variant types are skipped. Combined with the pairwise-
/// disjointness axioms registered on the engine, this gives Z3
/// complete information about inhabitants of the variant sort
/// without needing a dedicated ADT datatype encoding.
pub fn variant_exhaustiveness_hypotheses(
    theorem: &TheoremDecl,
    variant_map: &std::collections::HashMap<Text, Vec<Text>>,
) -> Vec<Expr> {
    use verum_ast::decl::FunctionParamKind;
    use verum_ast::pattern::PatternKind;
    use verum_ast::ty::TypeKind;

    let mut out: Vec<Expr> = Vec::new();
    for param in &theorem.params {
        let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else { continue; };
        let PatternKind::Ident { name: param_name, .. } = &pattern.kind else { continue; };
        let TypeKind::Path(path) = &ty.kind else { continue; };
        let Some(type_id) = path.as_ident() else { continue; };
        let Some(ctors) = variant_map.get(&type_id.name) else { continue; };
        if ctors.is_empty() {
            continue;
        }

        // Build `p == T.Ctor` expressions for each constructor.
        let make_disjunct = |ctor: &Text| -> Expr {
            let type_seg = verum_ast::ty::Ident::new(type_id.name.as_str(), ty.span);
            let ctor_seg = verum_ast::ty::Ident::new(ctor.as_str(), ty.span);
            let qualified_path = verum_ast::ty::Path::new(
                List::from_iter([
                    verum_ast::ty::PathSegment::Name(type_seg),
                    verum_ast::ty::PathSegment::Name(ctor_seg),
                ]),
                ty.span,
            );
            let qualified_expr = Expr::new(
                ExprKind::Path(qualified_path),
                ty.span,
            );
            let param_path = Expr::new(
                ExprKind::Path(verum_ast::ty::Path::single(param_name.clone())),
                ty.span,
            );
            Expr::new(
                ExprKind::Binary {
                    op: verum_ast::BinOp::Eq,
                    left: Heap::new(param_path),
                    right: Heap::new(qualified_expr),
                },
                ty.span,
            )
        };

        // OR them together.
        let mut disj = make_disjunct(&ctors[0]);
        for c in &ctors[1..] {
            let next = make_disjunct(c);
            disj = Expr::new(
                ExprKind::Binary {
                    op: verum_ast::BinOp::Or,
                    left: Heap::new(disj),
                    right: Heap::new(next),
                },
                ty.span,
            );
        }
        out.push(disj);
    }
    out
}

pub fn refinement_hypotheses_from_params(
    params: &List<verum_ast::decl::FunctionParam>,
    alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
) -> Vec<Expr> {
    use verum_ast::decl::FunctionParamKind;
    use verum_ast::pattern::PatternKind;

    let mut out = Vec::new();

    for param in params {
        let (pat, ty) = match &param.kind {
            FunctionParamKind::Regular { pattern, ty, .. } => (pattern, ty),
            _ => continue,
        };

        // Pull the binder identifier out of the pattern — only single-name
        // patterns get refinement hypothesis treatment here; destructuring
        // patterns have their own VC path.
        let param_ident = match &pat.kind {
            PatternKind::Ident { name, .. } => name.clone(),
            _ => continue,
        };

        // Accumulate refinement predicates along the type structure. The
        // outermost peel is done here; nested refinements on the base type
        // descend recursively and contribute additional conjuncts.
        collect_refinements(ty, &param_ident, &mut out, alias_map);
    }

    out
}

/// Descend into a type looking for inline refinement predicates. For each
/// one, substitute the binder with `param_ident` and emit the rewritten
/// predicate into `out`. Stops when a non-refinement, non-delegating
/// TypeKind is reached.
///
/// Nominal types are looked up in `alias_map`; each stored predicate is
/// assumed to use `self` as its binder (matching the convention of
/// `build_refinement_alias_map`), and is rewritten to the parameter
/// identifier before being pushed.
fn collect_refinements(
    ty: &verum_ast::ty::Type,
    param_ident: &Ident,
    out: &mut Vec<Expr>,
    alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
) {
    use verum_ast::ty::TypeKind;

    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            // Implicit `self` / `it` binder by convention. Substitute every
            // free reference to those names with the parameter identifier.
            let rewritten = substitute_ident(
                &predicate.expr,
                &[
                    (Text::from("self"), param_ident.clone()),
                    (Text::from("it"), param_ident.clone()),
                ],
            );
            // Also honour an explicit binder if the refinement carries one
            // (Rule 2: `T where |x| expr`).
            let rewritten = if let Maybe::Some(binder) = &predicate.binding {
                substitute_ident(&rewritten, &[(binder.name.clone(), param_ident.clone())])
            } else {
                rewritten
            };
            out.push(rewritten);
            collect_refinements(base, param_ident, out, alias_map);
        }
        TypeKind::Sigma { name, base, predicate } => {
            let rewritten = substitute_ident(
                predicate,
                &[(name.name.clone(), param_ident.clone())],
            );
            out.push(rewritten);
            collect_refinements(base, param_ident, out, alias_map);
        }
        TypeKind::Bounded { base, .. } => {
            collect_refinements(base, param_ident, out, alias_map)
        }
        // Nominal type reference (`n: FanoDim`). If the name resolves to an
        // entry in the precomputed alias map, contribute every stored
        // predicate with `self` rewritten to the parameter name. The map is
        // already flattened: nested alias chains have been walked once at
        // module load time, so we don't need to recurse further here.
        TypeKind::Path(path) if path.segments.len() == 1 => {
            if let verum_ast::PathSegment::Name(id) = &path.segments[0] {
                if let Some(preds) = alias_map.get(&id.name) {
                    for pred in preds {
                        let rewritten = substitute_ident(
                            pred,
                            &[(Text::from("self"), param_ident.clone())],
                        );
                        out.push(rewritten);
                    }
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Variant disjointness — architectural treatment of algebraic data types
// ---------------------------------------------------------------------------

/// For every `type T is A | B | C;` declaration in the module,
/// generate pairwise-distinctness axioms: `T.A != T.B`, `T.A != T.C`,
/// `T.B != T.C`. These are asserted on the solver before every
/// theorem check so claims like `theorem t(): T.A != T.B` close
/// automatically, and so a `match` ite-chain's branch-selection
/// logic has the right discriminator semantics.
///
/// The lowering matches the translator's Path-translation scheme:
/// the qualified name `T.A` pretty-prints to `T.A` and emits
/// `Int::new_const("path_T.A")`. Disjointness is therefore
/// `path_T.A != path_T.B` at the Z3 level — no dedicated ADT sort
/// required. Exhaustiveness (every value equals some constructor)
/// is NOT emitted because it would need a universal over the
/// variant sort, which we don't model; the `match` translator's
/// ite-chain handles exhaustiveness indirectly when the user
/// enumerates all constructors.
pub fn variant_disjointness_axioms(module: &verum_ast::Module) -> Vec<Expr> {
    use verum_ast::decl::TypeDeclBody;
    use verum_ast::ItemKind;

    let mut axioms: Vec<Expr> = Vec::new();
    for item in &module.items {
        if let ItemKind::Type(td) = &item.kind {
            if let TypeDeclBody::Variant(variants) = &td.body {
                if variants.len() < 2 {
                    continue;
                }
                let type_name = td.name.name.as_str();
                let type_span = td.span;

                // Build the qualified-path Expr `T.A` for each
                // variant. The translator's path-translation key
                // matches `path_<joined_segments>` so we do the
                // same canonical construction here.
                let make_variant_path = |v_name: &verum_common::Text| -> Expr {
                    let type_ident = verum_ast::ty::Ident::new(type_name, type_span);
                    let variant_ident =
                        verum_ast::ty::Ident::new(v_name.as_str(), type_span);
                    let path = verum_ast::ty::Path::new(
                        List::from_iter([
                            verum_ast::ty::PathSegment::Name(type_ident),
                            verum_ast::ty::PathSegment::Name(variant_ident),
                        ]),
                        type_span,
                    );
                    Expr::new(ExprKind::Path(path), type_span)
                };

                // Pairwise `A != B` for all distinct pairs.
                for i in 0..variants.len() {
                    for j in (i + 1)..variants.len() {
                        let a = make_variant_path(&variants[i].name.name);
                        let b = make_variant_path(&variants[j].name.name);
                        let ne = Expr::new(
                            ExprKind::Binary {
                                op: verum_ast::BinOp::Ne,
                                left: Heap::new(a),
                                right: Heap::new(b),
                            },
                            type_span,
                        );
                        axioms.push(ne);
                    }
                }
            }
        }
    }
    axioms
}

// ---------------------------------------------------------------------------
// Module-scoped lemma / axiom registration
// ---------------------------------------------------------------------------

/// Walk all `theorem`, `lemma`, `corollary`, and `axiom` declarations in the
/// module and register each as a `LemmaHint` in the `HintsDatabase` keyed by
/// its unqualified name. This is what gives `apply <name>` in one theorem's
/// proof body access to every sibling declaration in the same file.
///
/// The lemma expression is the theorem's `proposition`, which the parser
/// already synthesises from the requires/ensures clauses as
/// `(req_conj) ⇒ (ens_conj)`. `try_apply` then peels the implication chain
/// into premises + conclusion and unifies the conclusion with the current
/// goal, exactly the usual natural-deduction `apply` semantics.
///
/// Priority is fixed at a neutral mid-value so user declarations sit between
/// core stdlib hints (highest) and speculative auto-hints (lowest). No
/// heuristic ranking: the caller supplies the lemma name explicitly, so
/// priority only matters when pattern-matching auto chooses among multiple
/// candidates, not for direct-lookup apply.
pub fn register_module_lemmas(
    module: &verum_ast::Module,
    hints: &mut verum_smt::proof_search::HintsDatabase,
) {
    use verum_ast::ItemKind;
    use verum_smt::proof_search::LemmaHint;

    for item in &module.items {
        let (name, proposition) = match &item.kind {
            ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t) => (
                t.name.name.clone(),
                t.proposition.as_ref().clone(),
            ),
            ItemKind::Axiom(a) => (
                a.name.name.clone(),
                a.proposition.as_ref().clone(),
            ),
            _ => continue,
        };

        let hint = LemmaHint {
            name: name.clone(),
            priority: 500,
            lemma: Heap::new(proposition),
        };
        hints.register_lemma(name, hint);
    }
}

// ---------------------------------------------------------------------------
// Alias-map construction (nominal refinement chain flattener)
// ---------------------------------------------------------------------------

/// Walk all `TypeDecl`s in a module and produce a map
/// `AliasName → [refinement-predicate rooted at `self`]` by flattening every
/// `type X is T1 { self op K1 }` / `type X is T2 where …` chain, resolving
/// intermediate named types by re-entering the map recursively.
///
/// This is the data source that turns `n: FanoDim` into the implicit
/// hypothesis `n == 7` at verification time, eliminating the need for
/// authors to restate the refinement as an explicit `requires` clause.
pub fn build_refinement_alias_map(
    module: &verum_ast::Module,
) -> std::collections::HashMap<Text, Vec<Expr>> {
    use std::collections::HashMap;
    use verum_ast::decl::TypeDeclBody;
    use verum_ast::ItemKind;

    // First pass: collect `name → body_type` for every alias/newtype
    // declaration in the module.
    let mut raw_aliases: HashMap<Text, verum_ast::ty::Type> = HashMap::new();
    for item in &module.items {
        if let ItemKind::Type(td) = &item.kind {
            match &td.body {
                TypeDeclBody::Alias(t) | TypeDeclBody::Newtype(t) => {
                    raw_aliases.insert(td.name.name.clone(), t.clone());
                }
                _ => {}
            }
        }
    }

    // Second pass: for each alias, flatten its refinement chain, following
    // nominal references into sibling aliases. We guard against cycles with
    // a visited set scoped per-alias.
    let mut flattened: HashMap<Text, Vec<Expr>> = HashMap::new();
    for name in raw_aliases.keys().cloned().collect::<Vec<_>>() {
        let mut visited = std::collections::HashSet::new();
        let mut preds = Vec::new();
        if let Some(ty) = raw_aliases.get(&name) {
            flatten_chain(ty, &raw_aliases, &mut visited, &mut preds);
        }
        if !preds.is_empty() {
            flattened.insert(name, preds);
        }
    }
    flattened
}

/// Inner recursive flattener.
fn flatten_chain(
    ty: &verum_ast::ty::Type,
    raw_aliases: &std::collections::HashMap<Text, verum_ast::ty::Type>,
    visited: &mut std::collections::HashSet<Text>,
    out: &mut Vec<Expr>,
) {
    use verum_ast::ty::TypeKind;

    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            // Normalise the refinement predicate to use `self` as the
            // binder — that's the convention `collect_refinements`
            // expects for alias-map entries.
            let mut pred_expr = predicate.expr.clone();
            if let Maybe::Some(binder) = &predicate.binding {
                pred_expr = substitute_ident(
                    &pred_expr,
                    &[(
                        binder.name.clone(),
                        Ident::new("self", predicate.span),
                    )],
                );
            }
            // `it` → `self` normalisation (Rule 1 convention).
            pred_expr = substitute_ident(
                &pred_expr,
                &[(Text::from("it"), Ident::new("self", predicate.span))],
            );
            out.push(pred_expr);
            flatten_chain(base, raw_aliases, visited, out);
        }
        TypeKind::Sigma { name, base, predicate } => {
            let pred_expr = substitute_ident(
                predicate,
                &[(name.name.clone(), Ident::new("self", predicate.span))],
            );
            out.push(pred_expr);
            flatten_chain(base, raw_aliases, visited, out);
        }
        TypeKind::Path(path) if path.segments.len() == 1 => {
            if let verum_ast::PathSegment::Name(id) = &path.segments[0] {
                if visited.insert(id.name.clone()) {
                    if let Some(next_ty) = raw_aliases.get(&id.name) {
                        flatten_chain(next_ty, raw_aliases, visited, out);
                    }
                }
            }
        }
        // `(T)` parenthesised / single-element tuple form (e.g. the
        // `type Dimension is (Int) { self > 0 };` shape). The outer
        // refinement is already captured by the `Refined` arm above;
        // nothing more to recurse into for the element.
        _ => {}
    }
}

/// Substitute every free `Path` consisting of a single ident in `from_to`
/// with the corresponding target ident, returning a new `Expr`.
///
/// Two path shapes are recognised at the single-segment head:
///
/// * `PathSegment::Name(id)` — the normal identifier case, substituted when
///   `id.name` matches one of the `from` entries.
/// * `PathSegment::SelfValue` — the `self` keyword used in refinement
///   predicates (Rule 1, `T { self > 0 }`). The AST stores `self` as a
///   dedicated segment kind rather than an identifier, so we match it
///   explicitly against a `from` entry of the literal text `"self"`.
pub fn substitute_ident(expr: &Expr, from_to: &[(Text, Ident)]) -> Expr {
    match &expr.kind {
        ExprKind::Path(p) => {
            if p.segments.len() == 1 {
                match &p.segments[0] {
                    verum_ast::PathSegment::Name(id) => {
                        for (from, to) in from_to {
                            if id.name == *from {
                                let mut new_path = p.clone();
                                new_path.segments =
                                    smallvec::smallvec![verum_ast::PathSegment::Name(to.clone())];
                                return Expr::new(ExprKind::Path(new_path), expr.span);
                            }
                        }
                    }
                    verum_ast::PathSegment::SelfValue => {
                        for (from, to) in from_to {
                            if from.as_str() == "self" {
                                let mut new_path = p.clone();
                                new_path.segments =
                                    smallvec::smallvec![verum_ast::PathSegment::Name(to.clone())];
                                return Expr::new(ExprKind::Path(new_path), expr.span);
                            }
                        }
                    }
                    _ => {}
                }
            }
            expr.clone()
        }
        ExprKind::Binary { op, left, right } => Expr::new(
            ExprKind::Binary {
                op: *op,
                left: Heap::new(substitute_ident(left, from_to)),
                right: Heap::new(substitute_ident(right, from_to)),
            },
            expr.span,
        ),
        ExprKind::Unary { op, expr: inner } => Expr::new(
            ExprKind::Unary {
                op: *op,
                expr: Heap::new(substitute_ident(inner, from_to)),
            },
            expr.span,
        ),
        ExprKind::Paren(inner) => Expr::new(
            ExprKind::Paren(Heap::new(substitute_ident(inner, from_to))),
            expr.span,
        ),
        ExprKind::Call { func, type_args, args } => {
            let new_args: List<Expr> = args
                .iter()
                .map(|a| substitute_ident(a, from_to))
                .collect();
            Expr::new(
                ExprKind::Call {
                    func: Heap::new(substitute_ident(func, from_to)),
                    type_args: type_args.clone(),
                    args: new_args,
                },
                expr.span,
            )
        }
        ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } => {
            let new_args: List<Expr> = args
                .iter()
                .map(|a| substitute_ident(a, from_to))
                .collect();
            Expr::new(
                ExprKind::MethodCall {
                    receiver: Heap::new(substitute_ident(receiver, from_to)),
                    method: method.clone(),
                    type_args: type_args.clone(),
                    args: new_args,
                },
                expr.span,
            )
        }
        ExprKind::Field { expr: inner, field } => Expr::new(
            ExprKind::Field {
                expr: Heap::new(substitute_ident(inner, from_to)),
                field: field.clone(),
            },
            expr.span,
        ),
        ExprKind::Index { expr: inner, index } => Expr::new(
            ExprKind::Index {
                expr: Heap::new(substitute_ident(inner, from_to)),
                index: Heap::new(substitute_ident(index, from_to)),
            },
            expr.span,
        ),
        _ => expr.clone(),
    }
}

// ---------------------------------------------------------------------------
// `result` → `true` elaboration for `-> Bool` theorems
// ---------------------------------------------------------------------------

/// Substitute every free reference to the reserved name `result` with the
/// boolean literal `true` in `expr`.
///
/// Rationale (see the call site in `verify_proof_body` for the wider
/// discussion): a theorem of shape
///
/// ```verum
/// theorem t(..) -> Bool
///     ensures <predicate involving result>
///     proof by <tactic>
/// ```
///
/// declares that the proof body is evidence for the predicate; the `-> Bool`
/// return is a syntactic convenience whose witness is, by the convention
/// of our proof system, fixed to `true`. The SMT translator is independent
/// of this convention — by default it binds `result` to a fresh
/// uninterpreted integer, which makes even obviously-true obligations
/// (`ensures result == (n == 7)` under `requires n == 7`) unprovable. This
/// helper closes that gap before the goal is handed to the engine.
///
/// The walker is intentionally limited to the expression shapes that appear
/// in specification predicates (logical connectives, equalities and
/// inequalities, arithmetic, field / method / index access, literal
/// introductions). Anything else is reproduced unchanged — safe since
/// `result` only carries the "proof-body output" meaning inside predicate
/// contexts; it is not a language-wide keyword.
fn substitute_result_with_true(expr: &Expr) -> Expr {
    const RESULT_NAME: &str = "result";
    match &expr.kind {
        ExprKind::Path(p) => {
            if p.segments.len() == 1 {
                if let verum_ast::PathSegment::Name(id) = &p.segments[0] {
                    if id.name.as_str() == RESULT_NAME {
                        return Expr::new(
                            ExprKind::Literal(verum_ast::literal::Literal::new(
                                LiteralKind::Bool(true),
                                expr.span,
                            )),
                            expr.span,
                        );
                    }
                }
            }
            expr.clone()
        }
        ExprKind::Binary { op, left, right } => Expr::new(
            ExprKind::Binary {
                op: *op,
                left: Heap::new(substitute_result_with_true(left)),
                right: Heap::new(substitute_result_with_true(right)),
            },
            expr.span,
        ),
        ExprKind::Unary { op, expr: inner } => Expr::new(
            ExprKind::Unary {
                op: *op,
                expr: Heap::new(substitute_result_with_true(inner)),
            },
            expr.span,
        ),
        ExprKind::Paren(inner) => Expr::new(
            ExprKind::Paren(Heap::new(substitute_result_with_true(inner))),
            expr.span,
        ),
        ExprKind::Call { func, type_args, args } => {
            let new_args: List<Expr> = args
                .iter()
                .map(|a| substitute_result_with_true(a))
                .collect();
            Expr::new(
                ExprKind::Call {
                    func: Heap::new(substitute_result_with_true(func)),
                    type_args: type_args.clone(),
                    args: new_args,
                },
                expr.span,
            )
        }
        ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } => {
            let new_args: List<Expr> = args
                .iter()
                .map(|a| substitute_result_with_true(a))
                .collect();
            Expr::new(
                ExprKind::MethodCall {
                    receiver: Heap::new(substitute_result_with_true(receiver)),
                    method: method.clone(),
                    type_args: type_args.clone(),
                    args: new_args,
                },
                expr.span,
            )
        }
        ExprKind::Field { expr: inner, field } => Expr::new(
            ExprKind::Field {
                expr: Heap::new(substitute_result_with_true(inner)),
                field: field.clone(),
            },
            expr.span,
        ),
        ExprKind::Index { expr: inner, index } => Expr::new(
            ExprKind::Index {
                expr: Heap::new(substitute_result_with_true(inner)),
                index: Heap::new(substitute_result_with_true(index)),
            },
            expr.span,
        ),
        // Everything else (Literal, Try, Match, If, Let, …) either can't
        // contain `result` or already has the right shape; pass through.
        _ => expr.clone(),
    }
}

// ============================================================================
// Model-theoretic discharge of protocol axioms at `implement` sites
// ============================================================================

use verum_ast::decl::{ImplDecl, TypeDecl};
use verum_types::proof_obligations::{collect_impl_obligations, find_proof_clause_for};

/// Result of attempting to verify all obligations of an `implement` block.
#[derive(Debug, Clone)]
pub struct ImplVerificationReport {
    /// The impl block that was verified.
    pub impl_span: verum_ast::span::Span,

    /// The protocol whose axioms were discharged.
    pub protocol_name: Text,

    /// Obligations that were successfully discharged (axiom name + tactic used).
    pub verified: Vec<(Text, Text)>,

    /// Obligations that could not be discharged (axiom name + failure reason
    /// + origin/impl spans for the diagnostic cursor).
    pub unverified: Vec<ImplObligationFailure>,

    /// Total wall-clock time spent verifying this impl block.
    pub total_duration: Duration,
}

impl ImplVerificationReport {
    /// Whether every obligation was discharged (zero failures).
    pub fn is_fully_verified(&self) -> bool {
        self.unverified.is_empty()
    }
}

/// A single unmet proof obligation, with the data a diagnostic needs.
#[derive(Debug, Clone)]
pub struct ImplObligationFailure {
    pub axiom_name: Text,
    pub reason: Text,
    pub origin_span: verum_ast::span::Span,
    pub impl_span: verum_ast::span::Span,
    pub attempted_tactic: Text,
}

/// Verify every axiom of the implemented protocol against the implementation's
/// concrete items. Each axiom is either:
///
///   1. Discharged by an explicit `proof axiom_name by tactic;` clause in
///      the impl block — the tactic is converted via `convert_tactic` and
///      executed against the self-substituted proposition.
///   2. Discharged by `ProofSearchEngine::auto_prove` with a bounded budget.
///
/// Returns a report listing verified obligations and failures. No side effect
/// on the compiler's diagnostic channel — callers (e.g. the pipeline) decide
/// how to present failures.
///
/// See `docs/architecture/model-theoretic-semantics.md` for the full
/// specification.
pub fn verify_impl_axioms(
    impl_decl: &ImplDecl,
    protocol_decl: &TypeDecl,
) -> ImplVerificationReport {
    let start = Instant::now();
    let protocol_name = Text::from(protocol_decl.name.name.as_str());
    let impl_span = impl_decl.span;

    let obligations = collect_impl_obligations(impl_decl, protocol_decl);

    let mut engine = ProofSearchEngine::new();
    let smt_ctx = Context::new();

    let mut verified = Vec::new();
    let mut unverified = Vec::new();

    for obligation in obligations.iter() {
        let axiom_name_text = Text::from(obligation.axiom_name.name.as_str());

        // Strategy 1: explicit proof clause.
        if let Some(tactic) = find_proof_clause_for(impl_decl, obligation.axiom_name.name.as_str()) {
            let proof_tactic = convert_tactic(tactic);
            let goal = ProofGoal::new(obligation.proposition.clone());
            match engine.execute_tactic(&proof_tactic, &goal) {
                Ok(_subgoals) => {
                    verified.push((axiom_name_text.clone(), tactic_summary(&proof_tactic)));
                }
                Err(err) => {
                    unverified.push(ImplObligationFailure {
                        axiom_name: axiom_name_text,
                        reason: Text::from(format!(
                            "explicit tactic failed: {}",
                            err
                        )),
                        origin_span: obligation.origin_span,
                        impl_span,
                        attempted_tactic: tactic_summary(&proof_tactic),
                    });
                }
            }
            continue;
        }

        // Strategy 2: auto_prove.
        match engine.auto_prove(&smt_ctx, &obligation.proposition) {
            Ok(_) => {
                verified.push((axiom_name_text, Text::from("auto")));
            }
            Err(err) => {
                unverified.push(ImplObligationFailure {
                    axiom_name: axiom_name_text,
                    reason: Text::from(format!(
                        "auto_prove could not close the obligation: {}",
                        err
                    )),
                    origin_span: obligation.origin_span,
                    impl_span,
                    attempted_tactic: Text::from("auto"),
                });
            }
        }
    }

    ImplVerificationReport {
        impl_span,
        protocol_name,
        verified,
        unverified,
        total_duration: start.elapsed(),
    }
}

/// Return a short human-readable label for a ProofTactic (used in the
/// report's `verified` and `attempted_tactic` fields).
fn tactic_summary(tactic: &ProofTactic) -> Text {
    match tactic {
        ProofTactic::Auto => Text::from("auto"),
        ProofTactic::AutoWith { .. } => Text::from("auto"),
        ProofTactic::Ring => Text::from("ring"),
        ProofTactic::Simplify => Text::from("simp"),
        ProofTactic::SimpWith { .. } => Text::from("simp"),
        ProofTactic::Field => Text::from("field"),
        ProofTactic::Reflexivity => Text::from("refl"),
        ProofTactic::Assumption => Text::from("assumption"),
        ProofTactic::Intro => Text::from("intro"),
        ProofTactic::Apply { .. } => Text::from("apply"),
        ProofTactic::Induction { .. } => Text::from("induction"),
        ProofTactic::Named { name, .. } => Text::from(name.as_str()),
        _ => Text::from("<tactic>"),
    }
}
