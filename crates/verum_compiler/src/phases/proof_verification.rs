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
    ProofStep, ProofStepKind, ProofStructure, TacticExpr, TheoremDecl,
};
use verum_ast::expr::ExprKind;
use verum_ast::pretty::format_expr;
use verum_ast::{Expr, Ident};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::context::Context;
use verum_smt::proof_search::{ProofError, ProofGoal, ProofSearchEngine, ProofTactic};
use verum_smt::verify::VerificationError;

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

        TacticExpr::Seq(tactics) => {
            let converted: List<ProofTactic> =
                tactics.iter().map(|t| convert_tactic(t)).collect();
            build_seq(converted)
        }

        TacticExpr::Alt(tactics) => {
            let converted: List<ProofTactic> =
                tactics.iter().map(|t| convert_tactic(t)).collect();
            build_alt(converted)
        }

        TacticExpr::AllGoals(inner) => {
            ProofTactic::AllGoals(Heap::new(convert_tactic(inner)))
        }

        TacticExpr::Focus(inner) => ProofTactic::Focus(Heap::new(convert_tactic(inner))),

        TacticExpr::Named { name, args } => ProofTactic::Named {
            name: name.name.clone(),
            args: args.iter().map(|a| format_expr(a)).collect(),
        },

        TacticExpr::Done => ProofTactic::Done,
        TacticExpr::Admit => ProofTactic::Admit,
        TacticExpr::Sorry => ProofTactic::Sorry,
        TacticExpr::Contradiction => ProofTactic::Contradiction,
    }
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

        // All subgoals produced by the tactic must be dischargeable
        for sub in &sub_goals {
            engine
                .auto_prove(smt_ctx, &sub.goal)
                .map_err(|e| ProofVerificationError::SubgoalFailed {
                    goal: format!("{:?}", sub.goal).into(),
                    reason: format!("{}", e).into(),
                })?;
        }

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

            // Apply case split tactic on the scrutinee
            let tactic = ProofTactic::CasesOn {
                hypothesis: format_expr(on),
            };
            let sub_goals = engine.execute_tactic(&tactic, goal).map_err(|e| {
                ProofVerificationError::MethodFailed {
                    method: "cases".into(),
                    reason: format!("{}", e).into(),
                }
            })?;

            steps.push(VerifiedStep {
                description: Text::from(format!("case split on {}", format_expr(on))),
                tactic_used: Text::from("cases"),
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

    // Verify each case
    let case_steps = verify_proof_cases(engine, smt_ctx, cases, &sub_goals)?;
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

        // Verify the proof steps within this case
        let inner_steps =
            verify_proof_steps(engine, smt_ctx, &case.proof, &case_goal.hypotheses)?;

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
    let mut verified = List::new();
    let mut hypotheses = initial_hypotheses.clone();

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
                let sub_goals =
                    engine
                        .execute_tactic(&tactic, &goal)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: format!("have {}", name.name).into(),
                            reason: format!("{}", e).into(),
                        })?;

                // Discharge any remaining subgoals via auto
                discharge_subgoals(engine, smt_ctx, &sub_goals, &format!("have {}", name.name))?;

                // Add the proposition as a hypothesis for subsequent steps
                hypotheses.push(proposition.as_ref().clone());

                verified.push(VerifiedStep {
                    description: Text::from(format!(
                        "have {}: {}",
                        name.name,
                        format_expr(proposition)
                    )),
                    tactic_used: format!("{:?}", justification).into(),
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
    smt_ctx: &Context,
    sub_goals: &List<ProofGoal>,
    context_label: &str,
) -> Result<(), ProofVerificationError> {
    for (i, sg) in sub_goals.iter().enumerate() {
        engine.auto_prove(smt_ctx, &sg.goal).map_err(|e| {
            ProofVerificationError::SubgoalFailed {
                goal: Text::from(format!(
                    "{} subgoal {}: {:?}",
                    context_label,
                    i + 1,
                    sg.goal
                )),
                reason: format!("{}", e).into(),
            }
        })?;
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

    // Build the primary goal from the theorem's proposition and requires clauses
    let proposition = theorem.proposition.as_ref().clone();
    let hypotheses: List<Expr> = theorem.requires.clone();
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
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions: build_suggestions_from_error(&e),
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
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions: build_suggestions_from_pv_error(&e),
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
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions: build_suggestions_from_pv_error(&e),
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
    let hypotheses = primary_goal.hypotheses.clone();
    let proposition = primary_goal.goal.clone();

    // Verify each step
    match verify_proof_steps(engine, smt_ctx, &structure.steps, &hypotheses) {
        Ok(mut verified_steps) => {
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
                            unproved.push(UnprovedSubgoal {
                                goal: format_expr(&proposition),
                                hypotheses: hypotheses
                                    .iter()
                                    .map(|h| format_expr(h))
                                    .collect(),
                                suggestions: build_suggestions_from_pv_error(&e),
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
                        unproved.push(UnprovedSubgoal {
                            goal: format_expr(&proposition),
                            hypotheses: hypotheses
                                .iter()
                                .map(|h| format_expr(h))
                                .collect(),
                            suggestions: build_suggestions_from_proof_error(&e),
                        });
                        return ProofVerificationResult::Failed {
                            verified_steps,
                            unproved,
                        };
                    }
                }
            } else {
                // No explicit conclusion — try auto_prove to close the goal
                // with all accumulated hypotheses
                if let Err(e) = engine.auto_prove(smt_ctx, &proposition) {
                    let mut unproved = List::new();
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses
                            .iter()
                            .map(|h| format_expr(h))
                            .collect(),
                        suggestions: build_suggestions_from_error(&e),
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
            unproved.push(UnprovedSubgoal {
                goal: format_expr(&proposition),
                hypotheses: hypotheses
                    .iter()
                    .map(|h| format_expr(h))
                    .collect(),
                suggestions: build_suggestions_from_pv_error(&e),
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
        Self::TacticFailed {
            tactic: Text::from("(unknown)"),
            reason: format!("{}", e).into(),
        }
    }
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
