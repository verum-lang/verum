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
//! verum_ast::ProofStep ├──► proof_verification ──► verum_smt::ProofTactic
//! verum_ast::ProofMethod │ verum_smt::ProofSearchEngine
//! verum_ast::ProofBody ──┘ ──► VerificationResult
//! ```
//!
//! # Proof Body Variants
//!
//! - **Term**: Curry-Howard proof term — type-checked as an expression whose type
//!  matches the proposition.
//! - **Tactic**: Single tactic expression converted to `ProofTactic` and executed.
//! - **Structured**: Sequence of `ProofStep`s building toward a conclusion, each
//!  step producing or consuming subgoals.
//! - **ByMethod**: Dispatches induction, case analysis, or contradiction with
//!  appropriate subgoal generation.

use std::time::{Duration, Instant};

use verum_ast::decl::{
    CalcRelation, CalculationChain, FunctionParamKind, ProofBody, ProofCase, ProofMethod,
    ProofStep, ProofStepKind, ProofStructure, TacticExpr, TacticMatchArm, TheoremDecl,
};
use verum_ast::expr::ExprKind;
use verum_ast::literal::LiteralKind;
use verum_ast::pretty::format_expr;
use verum_ast::{Expr, Ident};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::context::Context;
use verum_smt::proof_search::{MatchArm, ProofError, ProofGoal, ProofSearchEngine, ProofTactic};
use verum_verification::tactic_evaluation::{Goal, GoalMetadata, Hypothesis};
use verum_verification::tactic_heuristics::{TacticSuggestion, suggest_next_tactics};

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

/// Evidence produced by this tactic/SMT verifier. None of these variants
/// carries a kernel-replayed proof of the proposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofEvidence {
    /// A declaration without a proof body is an explicit assumption.
    Assumed,
    /// The tactic engine discharged the goal, possibly using SMT.
    TacticDischarge,
    /// The written proof contains an explicit admit/sorry escape hatch.
    Admitted,
}

/// Verification metadata, not a replayable kernel certificate.
#[derive(Debug, Clone)]
pub struct ProofSummary {
    /// Theorem name that was verified.
    pub theorem_name: Text,
    /// Ordered list of steps accepted by the tactic engine.
    pub steps: List<VerifiedStep>,
    /// Total wall-clock time for the entire proof.
    pub total_duration: Duration,
    /// Exact elaborated proposition and initial hypotheses checked or assumed.
    pub goal: ProofGoal,
    /// How this result was accepted; never inferred from an empty step list.
    pub evidence: ProofEvidence,
}

impl ProofSummary {
    /// Whether the source proof explicitly admits an unresolved obligation.
    pub fn has_incomplete_steps(&self) -> bool {
        self.evidence == ProofEvidence::Admitted
    }
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
    /// The goal was accepted under the evidence recorded in the summary.
    Verified(ProofSummary),
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
            let (callee, arguments) = match &lemma.kind {
                ExprKind::Call { func, type_args, args: call_args } if args.is_empty() && type_args.is_empty() => (func.as_ref(), call_args),
                ExprKind::Call { .. } => return ProofTactic::Fail {
                    message: "apply requires unambiguous arguments and checked generic instantiation".into(),
                },
                _ => (lemma.as_ref(), args),
            };
            let lemma = format_expr(callee);
            if arguments.is_empty() { ProofTactic::Apply { lemma } }
            else { ProofTactic::ApplyWith { lemma, args: arguments.clone() } }
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
                let lemma_texts: List<Text> = lemmas.iter().map(|l| format_expr(l)).collect();
                if at_target.is_some() {
                    ProofTactic::Fail { message: "targeted simplification requires checked hypothesis transformation, which this backend does not support".into() }
                } else {
                    ProofTactic::SimpWith { lemmas: lemma_texts }
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
            term: format_expr(proof_operand(term)),
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

        TacticExpr::TryElse { body, fallback } => ProofTactic::Alt(
            Heap::new(convert_tactic(body)),
            Heap::new(convert_tactic(fallback)),
        ),

        TacticExpr::Repeat(inner) => ProofTactic::Repeat(Heap::new(convert_tactic(inner))),

        TacticExpr::Seq(tactics) => convert_tactic_sequence(tactics),

        TacticExpr::Alt(tactics) => {
            let converted: List<ProofTactic> = tactics.iter().map(|t| convert_tactic(t)).collect();
            build_alt(converted)
        }

        TacticExpr::AllGoals(inner) => ProofTactic::AllGoals(Heap::new(convert_tactic(inner))),

        TacticExpr::Focus(inner) => ProofTactic::Focus(Heap::new(convert_tactic(inner))),

        TacticExpr::Named { generic_args, .. } if !generic_args.is_empty() => ProofTactic::Fail {
            message: "generic tactic arguments require checked instantiation, which this backend does not support".into(),
        },
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

fn proof_operand(mut expression: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expression.kind {
        expression = inner;
    }
    expression
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
            let prefix: List<ProofTactic> = tactics.iter().take(i).map(convert_tactic).collect();
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
///  a
///  = b by h1
///  = c by h2
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
        let proposition =
            build_relation_expr(&current_expr, &step.target, &step.relation, step.span);

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

        ProofMethod::WellFoundedInduction {
            relation,
            on,
            cases,
        } => {
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
            let scrutinee = match &on.kind {
                ExprKind::Tuple(items) if items.is_empty() => None,
                _ => Some(on.as_ref()),
            };
            verify_guarded_cases(engine, smt_ctx, scrutinee, cases, goal)
        }

        ProofMethod::Contradiction { assumption, proof } => {
            verify_contradiction(engine, smt_ctx, assumption, proof, goal)
        }
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

    let var_name: Text = match on {
        Maybe::Some(ident) => ident.name.clone(),
        Maybe::None => return Err(ProofVerificationError::MethodFailed {
            method: "induction".into(), reason: "induction requires an explicit variable".into(),
        }),
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

    let kind_label = if strong {
        "strong induction"
    } else {
        "induction"
    };
    steps.push(VerifiedStep {
        description: Text::from(format!("{} on '{}'", kind_label, var_name)),
        tactic_used: Text::from(kind_label),
        duration: step_start.elapsed(),
    });

    // Verify each case with InductionCase context so `have IH: …;`
    // without an explicit `by` clause is admitted as the IH.
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
    let start = Instant::now();
    // Classical contradiction: H, not G |- false. Introducing an unrelated
    // implication, or merely creating a false subgoal, is not a proof of it.
    let mut hypotheses = goal.hypotheses.clone();
    hypotheses.push(Expr::new(ExprKind::Unary {
        op: verum_ast::UnOp::Not,
        expr: Heap::new(goal.goal.clone()),
    }, goal.goal.span));
    let false_goal = proof_bool(false, goal.goal.span);
    let mut steps = verify_proof_steps_accumulating(
        engine, smt_ctx, proof_steps, &mut hypotheses, &false_goal,
    )?;
    discharge_subgoals(engine, smt_ctx, &List::from_iter([
        ProofGoal::with_hypotheses(false_goal, hypotheses),
    ]), "contradiction")?;
    steps.push(VerifiedStep {
        description: format!("derive contradiction under '{}'", assumption.name).into(),
        tactic_used: "contradiction".into(),
        duration: start.elapsed(),
    });
    Ok(steps)
}

fn proof_bool(value: bool, span: verum_common::Span) -> Expr {
    Expr::new(ExprKind::Literal(verum_ast::Literal::bool(value, span)), span)
}

fn proof_binary(op: verum_ast::BinOp, left: Expr, right: Expr) -> Expr {
    let span = left.span.merge(right.span);
    Expr::new(ExprKind::Binary { op, left: Heap::new(left), right: Heap::new(right) }, span)
}

/// A binding-free pattern denotes a proposition about the scrutinee. Bindings
/// and constructors need typed eliminator evidence; diagnostic labels and
/// branch positions cannot provide that evidence.
fn proof_case_guard(
    pattern: &verum_ast::Pattern,
    scrutinee: Option<&Expr>,
) -> Result<Expr, ProofVerificationError> {
    use verum_ast::{BinOp, PatternKind};
    match &pattern.kind {
        PatternKind::Wildcard => Ok(proof_bool(true, pattern.span)),
        PatternKind::Paren(inner) => proof_case_guard(inner, scrutinee),
        PatternKind::Literal(literal) => {
            let value = Expr::new(ExprKind::Literal(literal.clone()), pattern.span);
            match scrutinee {
                Some(scrutinee) => Ok(proof_binary(BinOp::Eq, scrutinee.clone(), value)),
                None if matches!(literal.kind, LiteralKind::Bool(_)) => Ok(value),
                None => Err(ProofVerificationError::MethodFailed {
                    method: "cases".into(),
                    reason: "literal case patterns require a scrutinee".into(),
                }),
            }
        }
        PatternKind::Or(patterns) => {
            let mut guard = proof_bool(false, pattern.span);
            for pattern in patterns {
                guard = proof_binary(BinOp::Or, guard, proof_case_guard(pattern, scrutinee)?);
            }
            Ok(guard)
        }
        _ => Err(ProofVerificationError::MethodFailed {
            method: "cases".into(),
            reason: "this case pattern requires checked typed binding or constructor correspondence, which this backend does not support".into(),
        }),
    }
}

/// Case elimination checks coverage before assuming a guard. Overlap is safe:
/// each guard must imply the same original goal under the original context.
fn verify_guarded_cases(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    scrutinee: Option<&Expr>,
    cases: &List<ProofCase>,
    goal: &ProofGoal,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let start = Instant::now();
    if cases.is_empty() {
        return Err(ProofVerificationError::MethodFailed {
            method: "cases".into(), reason: "case analysis requires explicit covering branches".into(),
        });
    }
    let mut guards = Vec::with_capacity(cases.len());
    let mut coverage = proof_bool(false, goal.goal.span);
    for case in cases {
        let pattern_guard = proof_case_guard(&case.pattern, scrutinee)?;
        let guard = match &case.condition {
            Maybe::Some(condition) if matches!(case.pattern.kind, verum_ast::PatternKind::Wildcard) => condition.as_ref().clone(),
            Maybe::Some(condition) => proof_binary(verum_ast::BinOp::And, pattern_guard, condition.as_ref().clone()),
            Maybe::None => pattern_guard,
        };
        coverage = proof_binary(verum_ast::BinOp::Or, coverage, guard.clone());
        guards.push(guard);
    }
    discharge_subgoals(engine, smt_ctx, &List::from_iter([
        ProofGoal::with_hypotheses(coverage, goal.hypotheses.clone()),
    ]), "case coverage")?;
    let mut steps = List::from_iter([VerifiedStep {
        description: "case guards cover the original context".into(),
        tactic_used: "cases (coverage)".into(), duration: start.elapsed(),
    }]);
    for (case, guard) in cases.iter().zip(guards) {
        let mut branch = goal.clone();
        branch.add_hypothesis(guard);
        steps.extend(verify_case_body(engine, smt_ctx, case, &branch)?);
    }
    Ok(steps)
}

/// Generated induction obligations must be covered exactly. This legacy
/// backend has no typed pattern substitution certificate, so only wildcard
/// cases are admissible here. Conditions must follow from the generated
/// context before they can be used in a body.
fn verify_proof_cases(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    cases: &List<ProofCase>,
    sub_goals: &List<ProofGoal>,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    if cases.len() != sub_goals.len() {
        return Err(ProofVerificationError::IncompleteCases {
            expected: sub_goals.len(), provided: cases.len(),
            uncovered_count: sub_goals.len().saturating_sub(cases.len()),
        });
    }
    let mut steps = List::new();
    for (case, goal) in cases.iter().zip(sub_goals) {
        if !matches!(case.pattern.kind, verum_ast::PatternKind::Wildcard) {
            return Err(ProofVerificationError::MethodFailed {
                method: "induction cases".into(),
                reason: "named induction patterns require checked typed constructor correspondence; use wildcard obligations or a certified eliminator".into(),
            });
        }
        if let Maybe::Some(condition) = &case.condition {
            discharge_subgoals(engine, smt_ctx, &List::from_iter([
                ProofGoal::with_hypotheses(condition.as_ref().clone(), goal.hypotheses.clone()),
            ]), "generated case condition")?;
        }
        steps.extend(verify_case_body(engine, smt_ctx, case, goal)?);
    }
    Ok(steps)
}

fn verify_case_body(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    case: &ProofCase,
    goal: &ProofGoal,
) -> Result<List<VerifiedStep>, ProofVerificationError> {
    let start = Instant::now();
    let mut hypotheses = goal.hypotheses.clone();
    let mut steps = verify_proof_steps_accumulating(
        engine, smt_ctx, &case.proof, &mut hypotheses, &goal.goal,
    )?;
    discharge_subgoals(engine, smt_ctx, &List::from_iter([
        ProofGoal::with_hypotheses(goal.goal.clone(), hypotheses),
    ]), "case conclusion")?;
    steps.push(VerifiedStep {
        description: format!("case concludes {}", format_expr(&goal.goal)).into(),
        tactic_used: "case".into(), duration: start.elapsed(),
    });
    Ok(steps)
}

// ---------------------------------------------------------------------------
// Structured proof step verification
// ---------------------------------------------------------------------------

/// Check steps under one actual goal. Only discharged propositions enter the
/// context; no proof method grants a privileged assumption channel.
fn verify_proof_steps_accumulating(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    steps: &List<ProofStep>,
    hypotheses: &mut List<Expr>,
    current_goal: &Expr,
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
                let sub_goals = engine.execute_tactic(&tactic, &goal).map_err(|e| {
                    ProofVerificationError::StepFailed {
                        step: format!("have {}", name.name).into(), reason: e.to_string().into(),
                    }
                })?;
                discharge_subgoals(engine, smt_ctx, &sub_goals, &format!("have {}", name.name))?;
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
                let sub_goals = engine.execute_tactic(&tactic, &goal).map_err(|e| {
                    ProofVerificationError::StepFailed {
                        step: Text::from("show"),
                        reason: format!("{}", e).into(),
                    }
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
                let sub_goals = engine.execute_tactic(&tactic, &goal).map_err(|e| {
                    ProofVerificationError::StepFailed {
                        step: Text::from("suffices"),
                        reason: format!("{}", e).into(),
                    }
                })?;

                discharge_subgoals(engine, smt_ctx, &sub_goals, "suffices")?;

                // The sufficient condition replaces the active goal context;
                // add it as a hypothesis for subsequent steps.
                hypotheses.push(proposition.as_ref().clone());

                verified.push(VerifiedStep {
                    description: Text::from(format!("suffices {}", format_expr(proposition))),
                    tactic_used: format!("{:?}", justification).into(),
                    duration: step_start.elapsed(),
                });
            }

            // ---- Let: bind a local definition ----
            ProofStepKind::Let { .. } => {
                // A definition is not a proposition. Scoped substitution,
                // annotation checking and definedness need evidence before a
                // local binding can affect the logical context.
                return Err(ProofVerificationError::StepFailed {
                    step: Text::from("let"),
                    reason: Text::from("local proof definitions require checked scoped binding semantics, which this backend does not support"),
                });
            }

            // Witness introduction must preserve scope and freshness.
            ProofStepKind::Obtain { .. } => {
                return Err(ProofVerificationError::StepFailed {
                    step: "obtain".into(),
                    reason: "existential witnesses require checked scoped binding semantics, which this backend does not support".into(),
                });
            }

            // ---- Calc: equational reasoning chain ----
            ProofStepKind::Calc(chain) => {
                let calc_steps = verify_calc_chain(engine, smt_ctx, chain, &hypotheses)?;
                verified.extend(calc_steps);
            }

            ProofStepKind::Cases { scrutinee, cases } => {
                let goal = ProofGoal::with_hypotheses(current_goal.clone(), hypotheses.clone());
                verified.extend(verify_guarded_cases(engine, smt_ctx, Some(scrutinee), cases, &goal)?);
                hypotheses.push(current_goal.clone());
            }

            ProofStepKind::Focus { goal_index, steps: inner_steps } => {
                if *goal_index != 0 {
                    return Err(ProofVerificationError::StepFailed {
                        step: "focus".into(),
                        reason: "structured proofs have one active goal; only focus(0) is available".into(),
                    });
                }
                verified.extend(verify_proof_steps_accumulating(
                    engine, smt_ctx, inner_steps, hypotheses, current_goal,
                )?);
            }

            // ---- Tactic: bare tactic application ----
            ProofStepKind::Tactic(tactic_expr) => {
                let tactic = convert_tactic(tactic_expr);

                // A bare `apply lemma(args…);` step is natural-deduction
                // forward reasoning: instantiate the lemma, discharge its
                // premises against the current hypotheses, and add its
                // CONCLUSION to the hypothesis set — i.e. it is `have
                // <instantiated conclusion> by apply`. Before this arm
                // did that, the step executed against a placeholder
                // `true` goal and its result was DISCARDED (`let _ =`),
                // so an applied axiom contributed nothing and the
                // ensures goal failed with the axiom's ensures VERBATIM
                // in hand (T0842 trace, d1_repro).
                let apply_parts = match &tactic {
                    ProofTactic::Apply { lemma } => {
                        Some((lemma.clone(), List::<Expr>::new()))
                    }
                    ProofTactic::ApplyWith { lemma, args } => {
                        Some((lemma.clone(), args.clone()))
                    }
                    _ => None,
                };
                if let Some((lemma, args)) = apply_parts {
                    let (premises, conclusion) = engine
                        .instantiate_lemma(&lemma, &args)
                        .map_err(|e| ProofVerificationError::StepFailed {
                            step: format!("apply {}", lemma).into(),
                            reason: format!("{}", e).into(),
                        })?;
                    // Each premise is an obligation HERE, against the
                    // hypotheses accumulated so far.
                    let premise_goals: List<ProofGoal> = premises
                        .iter()
                        .map(|p| ProofGoal::with_hypotheses(p.clone(), hypotheses.clone()))
                        .collect();
                    discharge_subgoals(
                        engine,
                        smt_ctx,
                        &premise_goals,
                        &format!("apply {} premises", lemma),
                    )?;
                    // The instantiated conclusion is now a fact.
                    hypotheses.push(conclusion.clone());
                    verified.push(VerifiedStep {
                        description: Text::from(format!(
                            "apply {} ⟹ {}",
                            lemma,
                            format_expr(&conclusion)
                        )),
                        tactic_used: format!("{:?}", tactic_expr).into(),
                        duration: step_start.elapsed(),
                    });
                } else {
                    let goal = ProofGoal::with_hypotheses(current_goal.clone(), hypotheses.clone());
                    let sub_goals = engine.execute_tactic(&tactic, &goal).map_err(|e| {
                        ProofVerificationError::StepFailed {
                            step: format!("tactic {:?}", tactic_expr).into(), reason: e.to_string().into(),
                        }
                    })?;
                    discharge_subgoals(engine, smt_ctx, &sub_goals, "structured tactic")?;
                    hypotheses.push(current_goal.clone());
                    verified.push(VerifiedStep {
                        description: Text::from(format!("tactic {:?}", tactic_expr)),
                        tactic_used: format!("{:?}", tactic_expr).into(),
                        duration: step_start.elapsed(),
                    });
                }
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
///  the proposition. Since full dependent type checking is beyond this module's
///  scope, we delegate to the SMT engine to verify the proposition directly,
///  using the term as a hint.
///
/// - **Tactic**: A single tactic expression is converted to `ProofTactic` and
///  executed against the proposition as a goal.
///
/// - **Structured**: Each `ProofStep` is verified in sequence, threading
///  hypotheses through. A final conclusion tactic (if present) closes the
///  remaining goal.
///
/// - **ByMethod**: Dispatches to induction, case analysis, or contradiction
///  handlers that generate and verify the appropriate subgoals.
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
    verify_proof_body_with_aliases_and_graph(engine, smt_ctx, theorem, alias_map, None, None)
}

/// Same as [`verify_proof_body_with_aliases`] but accepts an optional
/// workspace-wide [`ApplyGraph`](verum_kernel::soundness::apply_graph::ApplyGraph).
/// When provided, kernel-bridge discharge failures are enriched with a
/// dependent-theorems note listing every theorem whose transitive
/// proof depends on the failing axiom.
///
/// **Why an `Option`** (#188 / #318): single-file `verum check` flows
/// don't have a workspace graph; only the orchestrator
/// (`pipeline::theorem_proofs::verify_theorem_proofs`) builds one.
/// The optional shape keeps the legacy entry zero-overhead while
/// enabling the rich-diagnostic path when context is available.
/// **Why `module` is an `Option` (T1151).** A parameter typed by a
/// record, variant or newtype translates to an opaque sort
/// `Verum!Name`, and registering that sort is safe only when the module
/// actually DECLARES `Name` — otherwise the solver is told a value has
/// a sort nothing in scope defines, which is the shape the corpus
/// regression below was made of. `verify_cmd.rs` answers the same
/// question with three layout registries; here the module itself is the
/// registry, and it is optional for the same reason `apply_graph` is:
/// the legacy single-theorem entries have no module to hand and must
/// stay zero-overhead.
pub fn verify_proof_body_with_aliases_and_graph(
    engine: &mut ProofSearchEngine,
    smt_ctx: &Context,
    theorem: &TheoremDecl,
    alias_map: &std::collections::HashMap<Text, Vec<Expr>>,
    apply_graph: Option<&verum_kernel::soundness::apply_graph::ApplyGraph>,
    module: Option<&verum_ast::Module>,
) -> ProofVerificationResult {
    // RECORD LAYOUTS, for the same reason and from the same authority
    // (T0905). `verify_cmd.rs:948` registers these and this entry point
    // did not, so a goal mentioning `r.v` could not build the
    // projection the reflection side emits and fell to a free constant:
    //
    //     definition   (= (on_rec r) (> (Verum!proj!Rec!v r) 0))
    //     goal         (not (= (on_rec r) (> field_r__v 0)))
    //
    // `field_r__v` is a nullary constant unrelated to `r`, so `r.v > 0`
    // — the SAME expression on both sides of the source — reached Z3 as
    // two unconnected terms and nothing could be decided. Measured: the
    // record claim proves under `verum verify` and fails under `verum
    // check`, same binary, same file.
    //
    // The fallback that produced `field_r__v` documents itself as being
    // "for a receiver whose type the obligation never declared" — and
    // on this path NO record type was ever declared, so it was not a
    // fallback here but the only route.
    if let Some(m) = module {
        use verum_ast::decl::TypeDeclBody;
        for item in &m.items {
            let verum_ast::ItemKind::Type(td) = &item.kind else {
                continue;
            };
            let TypeDeclBody::Record(fields) = &td.body else {
                continue;
            };
            let mut layout: std::collections::HashMap<String, (String, Option<String>)> =
                std::collections::HashMap::new();
            for f in fields.iter() {
                layout.insert(
                    f.name.name.as_str().to_string(),
                    verum_smt::expr_to_smtlib::type_to_sort_and_name(&f.ty),
                );
            }
            engine.register_record_type(td.name.name.clone(), layout);
        }
    }

    // Type names this module declares. `declared_sort_of` in
    // `verify_cmd.rs` asks the same question of three separate
    // registries — records, variants, positional (newtype) layouts —
    // and all three reduce to "is this name declared here". The module
    // is the one authority that answers for all three at once, and it
    // is what this entry point has.
    let declared_types: std::collections::HashSet<String> = module
        .map(|m| {
            m.items
                .iter()
                .filter_map(|it| match &it.kind {
                    verum_ast::ItemKind::Type(td) => Some(td.name.name.as_str().to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let proof_start = Instant::now();
    let theorem_name = theorem.name.name.clone();

    // Declared parameter types, so a goal that projects a field of a
    // parameter can resolve the receiver's record layout. Without
    // this the translator has no type for `w` in `w.flag` and falls
    // back to an Int constant — which a Bool field cannot be.
    for p in &theorem.params {
        if let FunctionParamKind::Regular { pattern, ty, .. } = &p.kind {
            if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                let (sort, type_name) = verum_smt::expr_to_smtlib::type_to_sort_and_name(ty);
                // Borrowed, not moved: the opaque-sort test below needs
                // the same name, and moving it here is what stopped it
                // being asked at all.
                if let Some(tn) = &type_name {
                    engine.register_value_type(name.name.clone(), Text::from(tn.as_str()));
                }
                // A Bool parameter must be known to be Bool: `cases b`
                // on a `b: Bool` parameter failed "Hypothesis 'b' not
                // found" because the engine had been told nothing about
                // `b` — `type_to_sort_and_name` answers `("Bool", None)`
                // for a primitive, so the name-only registration above
                // dropped every primitive parameter on the floor (T0960).
                //
                // Only the Bool fact is registered here, deliberately.
                // Registering the SORT of every parameter as well
                // measured WORSE on the corpus — 23/68 specs and 275
                // unproved theorems became 21/68 and 337 — so the sort
                // registry means something to the solver context that
                // a theorem parameter must not be pushed into blindly.
                // That is its own question, not this fix's.
                //
                // T1151 — AND THE ANSWER IS "NOT BLINDLY", NOT "NOT AT
                // ALL". The regression above is real and this does not
                // override it; it narrows what gets registered to the
                // half that cannot cause it.
                //
                // The defect: a theorem's parameter reaches the solver
                // as an Int whatever its declared type, so a goal
                // applies a symbol the reflection registry declared
                // over `Bool` to an Int and the application is refused —
                //
                //     over_bool (argument 0 is Int, declared "Bool")
                //
                // reported as an unproved goal, indistinguishable from
                // a claim the solver could not show. Measured, three
                // theorems in one file with the control first, since
                // Int is the default and passes either way:
                //
                //     over_int(n: Int)   -> PROVES   (control)
                //     over_bool(b: Bool) -> E0319
                //     over_text(t: Text) -> E0319
                //
                // `verum verify` proves all three, because
                // `verify_cmd.rs` registers the sort. One binary, two
                // entry points, opposite answers — so the fix is known
                // to work and the only question was which half of it is
                // safe.
                //
                // A PRIMITIVE SORT IS SAFE; AN OPAQUE ONE IS THE ONE
                // THAT REGRESSED. `type_to_sort_and_name` answers
                // `Verum!X` for any named type it cannot resolve, and
                // pushing that into the solver context is what the
                // corpus measured. `verify_cmd.rs` does not have that
                // problem because it computes the sort with
                // `declared_sort_of`, which consults the record,
                // variant and positional layout registries — registries
                // this entry point does not have.
                //
                // So: register Int / Bool / Real / String, which are
                // exactly the sorts that cannot be an unresolved name,
                // and leave `Verum!X` alone until the registries reach
                // here. That larger change keeps the corpus figures
                // above as its negative pole.
                // A primitive sort cannot be an unresolved name, so it is
                // always safe. An OPAQUE sort is safe exactly when this
                // module declares the type it names — which is the test
                // `declared_sort_of` performs against its three
                // registries, asked here of the module instead.
                //
                // Without the second half, eight of the proving ground's
                // ten obligations stay unstatable: their predicates take
                // `&CachedEntry`, `&Hash`, `&Version`, and the goal
                // applies a symbol declared over `Verum!Hash` to an Int.
                let safe_sort = matches!(sort.as_str(), "Int" | "Bool" | "Real" | "String")
                    || (sort.starts_with("Verum!")
                        && type_name
                            .as_deref()
                            .is_some_and(|n| declared_types.contains(n)));
                if safe_sort {
                    engine.register_value_sort(name.name.clone(), Text::from(sort.as_str()));
                }
                if sort == "Bool" {
                    engine.register_bool_hypothesis(name.name.clone());
                }
                // Likewise for integers, and for the same reason: an
                // `Int` parameter has no constructors, so `induction n`
                // could not infer a type for it and the tactic failed
                // outright.  The sort is the property that matters —
                // every type translating to SMT `Int` inducts the same
                // way — so this is keyed on the sort, not on a list of
                // type spellings.
                if sort == "Int" {
                    engine.register_integer_variable(name.name.clone());
                }
            }
        }
    }

    // Extract the proof body; axioms (no proof) are accepted elsewhere
    let proof_body = match &theorem.proof {
        Maybe::Some(body) => body,
        Maybe::None => {
            // No proof body — treat as axiom, nothing to verify here
            return ProofVerificationResult::Verified(ProofSummary {
                theorem_name,
                steps: List::new(),
                total_duration: proof_start.elapsed(),
                goal: ProofGoal::with_hypotheses(
                    theorem.proposition.as_ref().clone(),
                    theorem.requires.clone(),
                ),
                evidence: ProofEvidence::Assumed,
            });
        }
    };

    // ----------------------------------------------------------------
    // Pre-pass: kernel-bridge discharge validation (task #135 / MSFS-L4.2)
    // ----------------------------------------------------------------
    //

    // For every `apply kernel_<verb>_strict(literal_args)` invocation
    // in the proof body, replay the call through
    // `verum_kernel::dispatch_intrinsic`. When the dispatcher returns
    // `Decision { holds: false }`, the discharge claim is false at
    // the kernel level — short-circuit with a verification failure
    // before the SMT engine wastes time on a doomed goal.
    //

    // **Architectural significance.** This is the load-bearing piece
    // that converts the bridge-discharge audit's *observation* (task
    // #134) into a *compile-time gate*. Pre-fix the
    // `kernel_*_strict` axioms were treated as opaque trusted
    // assertions — the user's claim "this discharges through the
    // kernel" was honoured on faith. Post-fix the dispatcher's
    // structural check runs at compile time and rejects pathological
    // arg shapes before they reach SMT.
    //

    // **Performance**: pure AST walk + hashmap dispatch — no SMT
    // invocation, no kernel re-check round-trip. Microsecond-scale
    // per proof body, so adding this pre-pass to every theorem is
    // negligible.
    let bridge_errors = match apply_graph {
        Some(graph) => {
            // #188 / #318: graph-aware path — every dispatcher
            // rejection gets enriched with a dependent-theorems note.
            crate::phases::bridge_discharge_check::validate_proof_body_bridges_with_graph(
                proof_body,
                &theorem_name,
                std::path::Path::new(""),
                graph,
            )
        }
        None => {
            // Legacy path — single-file flows don't have a workspace
            // graph; the diagnostic gets the dispatcher reason only.
            crate::phases::bridge_discharge_check::validate_proof_body_bridges(
                proof_body,
                &theorem_name,
                std::path::Path::new(""),
            )
        }
    };
    if !bridge_errors.is_empty() {
        let unproved: List<UnprovedSubgoal> = bridge_errors
            .into_iter()
            .map(|e| {
                let goal_text = format!(
                    "kernel-bridge discharge `apply {}({})` rejected by dispatcher",
                    e.bridge_name,
                    e.args_rendered.join(", "),
                );
                let suggestion = format!("kernel `{}` reports: {}", e.bridge_name, e.reason,);
                let mut suggestions = List::new();
                suggestions.push(Text::from(suggestion));
                // #188 / #318: when the orchestrator pre-built an
                // ApplyGraph and called the graph-aware variant
                // (`validate_proof_body_bridges_with_graph`), the
                // error carries a pre-rendered "downstream theorems"
                // note. Push it as an additional suggestion so the
                // diagnostic surface displays the full transitive
                // impact alongside the dispatcher's stated reason.
                if !e.dependents_note.is_empty() {
                    suggestions.push(Text::from(e.dependents_note));
                }
                UnprovedSubgoal {
                    goal: Text::from(goal_text),
                    hypotheses: List::new(),
                    suggestions,
                }
            })
            .collect();
        return ProofVerificationResult::Failed {
            verified_steps: List::new(),
            unproved,
        };
    }

    // Source calls keep their declared meanings. A spelling such as `emp`
    // or `points_to` is not authority to interpret a call as a spatial atom.
    // Explicit SepAssertion entailment belongs to the SMT API; this entry
    // verifies the actual proposition using the supplied proof body.

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
    // The convention applies ONLY while the name `result` is free:
    // a theorem may legitimately declare a PARAMETER named `result`
    // (`theorem t(result: TranslationResult) ensures
    // result.quality >= 0`), and substituting the literal `true`
    // over it rewrote the goal into nonsense
    // (`true.quality >= 0`) — a binder capture: the convention's
    // substitution walked into a scope where the name was already
    // bound. A declared parameter always wins over the elaboration
    // convention.
    let result_is_a_parameter = theorem.params.iter().any(|p| {
        matches!(
            &p.kind,
            FunctionParamKind::Regular { pattern, .. }
                if matches!(
                    &pattern.kind,
                    verum_ast::pattern::PatternKind::Ident { name, .. }
                        if name.as_str() == "result"
                )
        )
    });
    let raw_proposition = theorem.proposition.as_ref().clone();
    let proposition = if result_is_a_parameter {
        raw_proposition.clone()
    } else {
        substitute_result_with_true(&raw_proposition)
    };
    let mut hypotheses: List<Expr> = theorem
        .requires
        .iter()
        .map(|e| {
            if result_is_a_parameter {
                e.clone()
            } else {
                substitute_result_with_true(e)
            }
        })
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

            // The supplied term must justify this exact goal. A different
            // automatically found proof cannot stand in for invalid evidence.
            let result = engine.execute_tactic(&ProofTactic::Exact {
                term: format_expr(proof_term),
            }, &primary_goal).map_err(ProofVerificationError::from).and_then(|sub_goals| {
                discharge_subgoals(engine, smt_ctx, &sub_goals, "proof term")
            });

            match result {
                Ok(()) => {
                    let mut steps = List::new();
                    steps.push(VerifiedStep {
                        description: Text::from(format!("proof term: {}", format_expr(proof_term))),
                        tactic_used: Text::from("exact"),
                        duration: step_start.elapsed(),
                    });
                    ProofVerificationResult::Verified(ProofSummary {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        goal: primary_goal.clone(),
                        evidence: ProofEvidence::TacticDischarge,
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    let mut suggestions =
                        heuristic_suggestions(&proposition, &hypotheses, Maybe::None);
                    suggestions.extend(build_suggestions_from_pv_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
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
                    // Recursive: `try admit` / `first [admit, …]` are
                    // admitted proofs too, not just a bare top-level
                    // `admit`/`sorry`.
                    let has_incomplete = tactic_expr.contains_unsafe();
                    let mut steps = List::new();
                    steps.push(VerifiedStep {
                        description: Text::from(format!("tactic: {:?}", tactic_expr)),
                        tactic_used: format!("{:?}", tactic_expr).into(),
                        duration: step_start.elapsed(),
                    });
                    ProofVerificationResult::Verified(ProofSummary {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        goal: primary_goal.clone(),
                evidence: if has_incomplete {
                    ProofEvidence::Admitted
                } else {
                    ProofEvidence::TacticDischarge
                },
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    // What happened first, guesses after — the report
                    // shows the leading suggestion.
                    let mut suggestions = build_suggestions_from_pv_error(&e);
                    suggestions.extend(heuristic_suggestions(
                        &proposition,
                        &hypotheses,
                        pv_error_tactic_name(&e),
                    ));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
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
        ProofBody::Structured(structure) => verify_structured_proof(
            engine,
            smt_ctx,
            structure,
            &primary_goal,
            &theorem_name,
            proof_start,
        ),

        // ----------------------------------------------------------------
        // Proof by method (induction / cases / contradiction)
        // ----------------------------------------------------------------
        ProofBody::ByMethod(method) => {
            match verify_proof_method(engine, smt_ctx, method, &primary_goal) {
                Ok(steps) => {
                    // AST-level authority: the previous detector
                    // string-matched lowercase "admit" against a Debug
                    // render that produces "Admit" and never fired.
                    let has_incomplete = method.contains_unsafe();
                    ProofVerificationResult::Verified(ProofSummary {
                        theorem_name,
                        steps,
                        total_duration: proof_start.elapsed(),
                        goal: primary_goal.clone(),
                evidence: if has_incomplete {
                    ProofEvidence::Admitted
                } else {
                    ProofEvidence::TacticDischarge
                },
                    })
                }
                Err(e) => {
                    let mut unproved = List::new();
                    // What happened first, guesses after — the report
                    // shows the leading suggestion.
                    let mut suggestions = build_suggestions_from_pv_error(&e);
                    suggestions.extend(heuristic_suggestions(
                        &proposition,
                        &hypotheses,
                        pv_error_tactic_name(&e),
                    ));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
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
        &proposition,
    ) {
        Ok(mut verified_steps) => {
            let hypotheses = accumulated_hypotheses.clone();
            // Apply conclusion tactic if present
            if let Maybe::Some(conclusion) = &structure.conclusion {
                let conclusion_start = Instant::now();
                let tactic = convert_tactic(conclusion);

                // The conclusion tactic closes the original goal with
                // all hypotheses accumulated during step verification.
                let enriched_goal =
                    ProofGoal::with_hypotheses(proposition.clone(), hypotheses.clone());

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
                                hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
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
                            hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
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
                let closer_goal =
                    ProofGoal::with_hypotheses(proposition.clone(), hypotheses.clone());
                let closer_result = engine
                    .execute_tactic(&ProofTactic::Auto, &closer_goal)
                    .and_then(|sub_goals| {
                        if sub_goals.is_empty() {
                            Ok(())
                        } else {
                            // Auto opened further subgoals (nested
                            // conjunction split, etc.). Recurse.
                            discharge_subgoals(engine, smt_ctx, &sub_goals, "closer").map_err(
                                |pe| ProofError::TacticFailed(format!("closer: {:?}", pe).into()),
                            )
                        }
                    });
                if let Err(e) = closer_result {
                    let mut unproved = List::new();
                    let mut suggestions =
                        heuristic_suggestions(&proposition, &hypotheses, Maybe::None);
                    suggestions.extend(build_suggestions_from_proof_error(&e));
                    unproved.push(UnprovedSubgoal {
                        goal: format_expr(&proposition),
                        hypotheses: hypotheses.iter().map(|h| format_expr(h)).collect(),
                        suggestions,
                    });
                    return ProofVerificationResult::Failed {
                        verified_steps,
                        unproved,
                    };
                }
            }

            // AST-level authority (the old string sniff compared
            // lowercase "admit" with the Debug render "Admit" and was
            // dead); covers step justifications, nested cases, calc
            // chains, and the conclusion tactic.
            let has_incomplete = structure.contains_unsafe();

            ProofVerificationResult::Verified(ProofSummary {
                theorem_name: theorem_name.clone(),
                steps: verified_steps,
                total_duration: proof_start.elapsed(),
                goal: primary_goal.clone(),
                evidence: if has_incomplete {
                    ProofEvidence::Admitted
                } else {
                    ProofEvidence::TacticDischarge
                },
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
    TacticFailed { tactic: Text, reason: Text },

    /// A proof step (have/show/suffices/obtain) failed.
    StepFailed { step: Text, reason: Text },

    /// A calculation chain step could not be justified.
    CalcStepFailed {
        from: Text,
        to: Text,
        relation: Text,
        reason: Text,
    },

    /// A subgoal could not be discharged.
    SubgoalFailed { goal: Text, reason: Text },

    /// A proof method (induction/cases/contradiction) failed.
    MethodFailed { method: Text, reason: Text },

    /// The number of written cases differs from the generated obligations.
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
                    "case count mismatch: {} cases expected, {} provided ({} uncovered)",
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
                reason: Text::from(format!("hypothesis `{}` is not in scope", name.as_str())),
            },
            ProofError::NotEquality(detail) => Self::TacticFailed {
                tactic: Text::from("rewrite"),
                reason: Text::from(format!("target is not an equality: {}", detail.as_str())),
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
        if !first.is_empty() && first.chars().all(|c| c.is_alphanumeric() || c == '_') {
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

/// Build suggestions from a `ProofVerificationError`.
/// Suggestions derived from what ACTUALLY happened.
///
/// These lead, and the machine-generated heuristics follow, because the
/// report shows the first one. It used to be the other way round: a
/// goal that never reached the solver — an untranslatable formula, a
/// non-proposition, a sort that did not line up — was reported as
/// "unproved" with a guess appended ("try `smt`"), identical to a goal
/// the solver had considered and could not settle. Two very different
/// situations under one word, and the guess is useless for the first.
///
/// The `reason` was carried on the error the whole time and discarded
/// by a `..` in this match. Two sort-alignment attempts were made blind
/// because of it, and each made things worse (T0901).
fn build_suggestions_from_pv_error(err: &ProofVerificationError) -> List<Text> {
    let mut suggestions = List::new();
    match err {
        ProofVerificationError::TacticFailed { tactic, reason } => {
            suggestions.push(Text::from(format!(
                "tactic '{}' failed: {}",
                tactic, reason
            )));
        }
        ProofVerificationError::StepFailed { step, reason } => {
            suggestions.push(Text::from(format!(
                "step '{}' could not be justified: {}",
                step, reason
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
        ProofVerificationError::MethodFailed { method, reason } => {
            // The reason is the whole answer here: it distinguishes "your
            // induction variable is wrong" from "induction over this type
            // is not implemented".  Every sibling arm renders its reason;
            // this one used to drop it with `..` and replace it with advice
            // that named a cause the author had no way to act on.
            suggestions.push(Text::from(format!(
                "Proof method '{}' failed: {}",
                method, reason
            )));
        }
        ProofVerificationError::IncompleteCases { expected, provided, .. } => {
            suggestions.push(Text::from(format!(
                "case count mismatch: expected {expected}, received {provided}; cover each generated obligation exactly once"
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
        ProofVerificationError::TacticFailed { tactic, .. } => Maybe::Some(tactic.as_str()),
        ProofVerificationError::MethodFailed { method, .. } => Maybe::Some(method.as_str()),
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

    let ranked: Vec<TacticSuggestion> = suggest_next_tactics(&goal, &exhausted_vec);

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
/// ```text
/// theorem id_trivial<P: Prop>(p: P): P { proof by auto }
/// ```
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
    let mut prop_generic_names: std::collections::HashSet<Text> = std::collections::HashSet::new();
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
                            verum_ast::expr::ExprKind::Path(verum_ast::ty::Path::single(
                                id.clone(),
                            )),
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
        let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else {
            continue;
        };
        let PatternKind::Ident {
            name: param_name, ..
        } = &pattern.kind
        else {
            continue;
        };
        let TypeKind::Path(path) = &ty.kind else {
            continue;
        };
        let Some(type_id) = path.as_ident() else {
            continue;
        };
        let Some(ctors) = variant_map.get(&type_id.name) else {
            continue;
        };
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
            let qualified_expr = Expr::new(ExprKind::Path(qualified_path), ty.span);
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
        TypeKind::Bounded { base, .. } => collect_refinements(base, param_ident, out, alias_map),
        // Nominal type reference (`n: FanoDim`). If the name resolves to an
        // entry in the precomputed alias map, contribute every stored
        // predicate with `self` rewritten to the parameter name. The map is
        // already flattened: nested alias chains have been walked once at
        // module load time, so we don't need to recurse further here.
        TypeKind::Path(path) if path.segments.len() == 1 => {
            if let verum_ast::PathSegment::Name(id) = &path.segments[0] {
                if let Some(preds) = alias_map.get(&id.name) {
                    for pred in preds {
                        let rewritten =
                            substitute_ident(pred, &[(Text::from("self"), param_ident.clone())]);
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
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;

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
                    let variant_ident = verum_ast::ty::Ident::new(v_name.as_str(), type_span);
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

    let param_names = |params: &List<verum_ast::decl::FunctionParam>| -> List<Text> {
        params
            .iter()
            .filter_map(|p| match &p.kind {
                FunctionParamKind::Regular { pattern, .. } => match &pattern.kind {
                    verum_ast::pattern::PatternKind::Ident { name, .. } => {
                        Some(name.name.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };

    for item in &module.items {
        let (name, proposition, params) = match &item.kind {
            ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t) => (
                t.name.name.clone(),
                t.proposition.as_ref().clone(),
                param_names(&t.params),
            ),
            ItemKind::Axiom(a) => (
                a.name.name.clone(),
                a.proposition.as_ref().clone(),
                param_names(&a.params),
            ),
            _ => continue,
        };

        let hint = LemmaHint {
            // Declaration-order parameter names: the authority for
            // positional instantiation of `apply lemma(args…)`.
            params,
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
/// For every RECORD declared in the module, its fields whose declared
/// type is a refinement alias: `Version { major: Component, … }` with
/// `Component is n: Int where n >= 0` yields `("Version", [("major",
/// "Component"), …])`.
///
/// Needed because a refinement on a PARAMETER reaches the solver as a
/// hypothesis and the same refinement on a FIELD did not — measured,
/// `fn get(b: Box) -> Int ensures result >= 0 { b.v }` with
/// `Box { v: NonNeg }` FAILED while the parameter form proved. The
/// argument for refinement types is that the invariant travels with the
/// type; a refinement that survives a parameter but not a field
/// survives only the shape nobody writes, since real domain types are
/// records (T0994).
pub fn build_record_field_alias_map(
    module: &verum_ast::Module,
) -> std::collections::HashMap<Text, Vec<(Text, Text)>> {
    use std::collections::HashMap;
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;

    let mut out: HashMap<Text, Vec<(Text, Text)>> = HashMap::new();
    for item in &module.items {
        let ItemKind::Type(td) = &item.kind else {
            continue;
        };
        let TypeDeclBody::Record(fields) = &td.body else {
            continue;
        };
        let mut carried: Vec<(Text, Text)> = Vec::new();
        for field in fields.iter() {
            if let verum_ast::ty::TypeKind::Path(p) = &field.ty.kind
                && let Some(id) = p.as_ident()
            {
                carried.push((field.name.name.clone(), id.name.clone()));
            }
        }
        if !carried.is_empty() {
            out.insert(td.name.name.clone(), carried);
        }
    }
    out
}

pub fn build_refinement_alias_map(
    module: &verum_ast::Module,
) -> std::collections::HashMap<Text, Vec<Expr>> {
    use std::collections::HashMap;
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;

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

/// The UNDERLYING type of every refinement alias in the module —
/// `type Pos is Int{it > 0}` answers `Int`.
///
/// The predicate map above says WHAT a refinement claims; this says
/// what the value IS. Both are needed, and only the first existed: a
/// function returning `Pos` had `result` declared at the alias's own
/// opaque sort, so the synthesised `result > 0` failed to translate
/// with "incompatible types for binary operation > (left is Verum!Pos,
/// right is Int)" — the obligation was stated and could not be read.
/// The inline spelling `-> Int{it > 0}` worked throughout, because
/// there the base is right there in the AST node (T0964).
pub fn build_refinement_alias_base_map(
    module: &verum_ast::Module,
) -> std::collections::HashMap<Text, verum_ast::ty::Type> {
    use std::collections::HashMap;
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;

    let mut raw: HashMap<Text, verum_ast::ty::Type> = HashMap::new();
    for item in &module.items {
        if let ItemKind::Type(td) = &item.kind {
            match &td.body {
                TypeDeclBody::Alias(t) | TypeDeclBody::Newtype(t) => {
                    raw.insert(td.name.name.clone(), t.clone());
                }
                _ => {}
            }
        }
    }
    let mut out: HashMap<Text, verum_ast::ty::Type> = HashMap::new();
    for (name, ty) in raw.iter() {
        let mut visited = std::collections::HashSet::new();
        if let Some(base) = unrefined_base(ty, &raw, &mut visited) {
            out.insert(name.clone(), base);
        }
    }
    out
}

/// Peel `Refined` wrappers and follow alias references to the first
/// type that is neither — the sort a value of the alias really has.
fn unrefined_base(
    ty: &verum_ast::ty::Type,
    raw: &std::collections::HashMap<Text, verum_ast::ty::Type>,
    visited: &mut std::collections::HashSet<Text>,
) -> Option<verum_ast::ty::Type> {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Refined { base, .. } => unrefined_base(base, raw, visited),
        TypeKind::Path(path) if path.segments.len() == 1 => {
            if let verum_ast::PathSegment::Name(id) = &path.segments[0] {
                if let Some(next) = raw.get(&id.name) {
                    if visited.insert(id.name.clone()) {
                        return unrefined_base(next, raw, visited);
                    }
                    // A cycle: no base to answer with.
                    return None;
                }
            }
            Some(ty.clone())
        }
        _ => Some(ty.clone()),
    }
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
                    &[(binder.name.clone(), Ident::new("self", predicate.span))],
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
///  `id.name` matches one of the `from` entries.
/// * `PathSegment::SelfValue` — the `self` keyword used in refinement
///  predicates (Rule 1, `T { self > 0 }`). The AST stores `self` as a
///  dedicated segment kind rather than an identifier, so we match it
///  explicitly against a `from` entry of the literal text `"self"`.
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
        ExprKind::Call {
            func,
            type_args,
            args,
        } => {
            let new_args: List<Expr> = args.iter().map(|a| substitute_ident(a, from_to)).collect();
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
            let new_args: List<Expr> = args.iter().map(|a| substitute_ident(a, from_to)).collect();
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
        // Everything else is STRUCTURAL: rebuild with each child
        // substituted, through the `verum_ast` walk that covers all 73
        // variants.
        //
        // This arm was `expr.clone()`, so substitution stopped at every
        // form not listed above — including the QUANTIFIERS. Measured:
        // `type B is m: Int where forall k in 0..3 => m >= k` had its
        // binder normalised to `self` and then `self` replaced by
        // `result` everywhere EXCEPT inside the quantifier body, so the
        // obligation spoke about a free `m` and came back with the
        // counterexample `m = 0, result = 9` — a claim about a variable
        // nothing constrained (T0964).
        //
        // A binder is entered only when it does not rebind one of the
        // names being replaced; that keeps `forall k. … self …`
        // substitutable while refusing `forall self. … self …`. This is
        // the third copy of this rule in the tree — the others are
        // `verum_types::refinement::substitute_in_expr` and
        // `verum_smt::expr_to_smtlib::substitute_ident`, both fixed the
        // same way and both found the same way: by a name surviving a
        // walk that was supposed to replace it.
        _ => {
            if verum_ast::visit_mut::introduces_bindings(&expr.kind)
                && binder_shadows_any(&expr.kind, from_to)
            {
                return expr.clone();
            }
            let mut rebuilt = expr.clone();
            verum_ast::visit_mut::each_child_expr_mut(&mut rebuilt, &mut |child| {
                *child = substitute_ident(child, from_to);
            });
            rebuilt
        }
    }
}

/// Does this binding form rebind one of the names being substituted?
///
/// Only the quantifiers are answered precisely, because they are the
/// forms a refinement predicate actually contains; every other binder
/// answers `true` and is left alone, which is the sound direction.
fn binder_shadows_any(kind: &ExprKind, from_to: &[(Text, Ident)]) -> bool {
    use verum_ast::pattern::PatternKind;
    let bindings = match kind {
        ExprKind::Forall { bindings, .. } | ExprKind::Exists { bindings, .. } => bindings,
        _ => return true,
    };
    bindings.iter().any(|b| match &b.pattern.kind {
        PatternKind::Ident { name, .. } => {
            from_to.iter().any(|(from, _)| name.name == *from)
        }
        // A pattern this does not read could bind anything.
        _ => true,
    })
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
///  ensures <predicate involving result>
///  proof by <tactic>
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
        ExprKind::Call {
            func,
            type_args,
            args,
        } => {
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
///  1. Discharged by an explicit `proof axiom_name by tactic;` clause in
///  the impl block — the tactic is converted via `convert_tactic` and
///  executed against the self-substituted proposition.
///  2. Discharged by `ProofSearchEngine::auto_prove` with a bounded budget.
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
        if let Some(tactic) = find_proof_clause_for(impl_decl, obligation.axiom_name.name.as_str())
        {
            let proof_tactic = convert_tactic(tactic);
            let goal = ProofGoal::new(obligation.proposition.clone());
            match engine.execute_tactic(&proof_tactic, &goal) {
                Ok(_subgoals) => {
                    verified.push((axiom_name_text.clone(), tactic_summary(&proof_tactic)));
                }
                Err(err) => {
                    unverified.push(ImplObligationFailure {
                        axiom_name: axiom_name_text,
                        reason: Text::from(format!("explicit tactic failed: {}", err)),
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
