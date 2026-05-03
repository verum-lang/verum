//! # Interactive proof-state explorer — current surface (#319 / #189)
//!

//! ## Why this module exists
//!

//! Coq has `Show.` and `--coq-proofview`; Lean 4 has VS Code's
//! "View Goal". Verum has neither — a `proof { … }` block is opaque
//! until the kernel verdict comes back, which makes scaling to large
//! corpora (mathlib4-class) prohibitively painful. Mathematicians
//! using Verum can't see the intermediate state of a proof, so they
//! can't tell *what step is wrong* when something fails.
//!

//! This module ships the current surface: a **static, heuristic snapshot**
//! of a theorem's proof state. No kernel invocation; no live tactic
//! evaluation; no goal-state simulation. The snapshot answers two
//! concrete questions:
//!

//!  1. *What tactics did I write?* ([`ProofState::applied_steps`])
//!  2. *How many steps does this proof claim to make?*
//!  ([`ProofState::applied_steps`].len())
//!

//! That's it. The user gets a structured, serializable view of their
//! proof body that they can pretty-print (`verum proofview <file>:<thm>`),
//! emit as JSON (`verum proofview --format json`), or feed into a
//! tutorial-style onboarding aid.
//!

//! ## scope (this slice)
//!

//! 1. Data types — [`ProofGoal`], [`NamedHypothesis`], [`ProofState`],
//!  [`ProofStepSnapshot`], [`ContextSnapshot`]. Every type derives
//!  `Debug + Clone + PartialEq + Eq + Serialize + Deserialize` so
//!  the snapshot round-trips cleanly through JSON.
//! 2. [`snapshot_proof_state`] — walks a `TheoremDecl`'s proof body
//!  (`ProofBody::Tactic` / `ProofBody::Structured` /
//!  `ProofBody::ByMethod` / `ProofBody::Term`) and produces a
//!  snapshot WITHOUT invoking the kernel. `applied_steps` carries
//!  the literal list of tactic names + their indices; `remaining_goals`
//!  is the post-pass count assumed if every step closes a goal
//!  (which is the heuristic the current surface ships — no goal
//!  simulation).
//!

//! ## non-goals (deferred to V1+)
//!

//! - **Live proof state.** V0 doesn't run the elaborator or the
//!  kernel. Hypotheses introduced by `intro` aren't tracked; the
//!  `current_context` field stays at depth zero. Future work will plug
//!  into [`crate::tactic_elaborator`]'s ElabContext to surface
//!  real bindings.
//! - **Goal-shape simulation.** V0 reports `remaining_goals = 0`
//!  when the proof body has at least one terminal step, and
//!  `remaining_goals = 1` for an empty body. The kernel-driven
//!  "what is the actual remaining goal type" answer requires
//!  tactic semantics.
//! - **Interactive REPL / LSP integration.** V0 is a one-shot
//!  snapshot exposed via `verum proofview`. An interactive
//!  experience composes with V1's live proof state.
//!

//! ## Architectural significance
//!

//! Even the current surface is a load-bearing onboarding aid. A new
//! mathematician using Verum can run `verum proofview my_file.vr:my_thm`
//! and see "your proof has 5 steps; step 3 applies `commutativity`;
//! step 5 closes the goal" — which is the basic introspection any
//! production proof assistant exposes. Without this surface, a
//! Verum proof is opaque; with it, the user has a literal map of
//! what they wrote.
//!

//! See `docs/architecture/verum-verification-architecture.md` for the
//! V1 / V2 promotion path (live state + LSP integration).

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use verum_ast::decl::{
    ProofBody, ProofMethod, ProofStep, ProofStepKind, ProofStructure, TacticExpr, TheoremDecl,
};

use crate::reflection::ReflectedTerm;

// =============================================================================
// NamedHypothesis — a single named binding in the proof context
// =============================================================================

/// A named hypothesis: `name : ty`.
///

/// current surface treats every hypothesis as opaquely-typed; the reflected
/// type uses an [`opaque_placeholder`] sentinel because translating
/// real hypothesis types requires the elaborator and is V1 work.
/// Snapshots produced by [`snapshot_proof_state`] currently emit zero
/// hypotheses (V0 doesn't simulate `intro`); the type stays in the
/// surface so V1 can plug actual context entries through unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedHypothesis {
    /// User-visible binder name.
    pub name: String,

    /// Reflected type of the binding. uses an opaque placeholder
    /// (`Universe(0)`) when the source type is structurally
    /// non-trivial; Future work plugs the elaborator output.
    pub ty: ReflectedTerm,
}

impl NamedHypothesis {
    /// Construct a hypothesis with explicit reflected type.
    pub fn new(name: impl Into<String>, ty: ReflectedTerm) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }

    /// Construct a hypothesis whose type is the V0 opaque placeholder.
    /// Use this when [`snapshot_proof_state`] can't determine the
    /// type from the AST alone.
    pub fn opaque(name: impl Into<String>) -> Self {
        Self::new(name, opaque_placeholder())
    }
}

// =============================================================================
// ProofGoal — one outstanding subgoal (proposition + local hypotheses)
// =============================================================================

/// One proof obligation: a proposition to prove + the local
/// hypotheses available when discharging it.
///

/// V0 snapshots emit at most one goal — the theorem's stated
/// proposition. Future work will surface per-subgoal entries once the
/// elaborator's goal-stack model is wired in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofGoal {
    /// The proposition to prove. falls back to
    /// [`opaque_placeholder`] when the surface AST shape can't be
    /// reflected without invoking the elaborator.
    pub proposition: ReflectedTerm,

    /// Hypotheses in scope for this goal. emits empty for
    /// theorem-level goals; V1 tracks `intro`-bound names + their
    /// reflected types.
    pub hypotheses: Vec<NamedHypothesis>,
}

impl ProofGoal {
    /// Construct a goal with the given proposition and no local
    /// hypotheses. Used for theorem-level goals in V0.
    pub fn closed(proposition: ReflectedTerm) -> Self {
        Self {
            proposition,
            hypotheses: Vec::new(),
        }
    }

    /// Construct a goal with the given proposition and explicit
    /// hypothesis list.
    pub fn with_hypotheses(proposition: ReflectedTerm, hypotheses: Vec<NamedHypothesis>) -> Self {
        Self {
            proposition,
            hypotheses,
        }
    }
}

// =============================================================================
// ProofStepSnapshot — one applied tactic, statically observed
// =============================================================================

/// One entry in [`ProofState::applied_steps`].
///

/// Every snapshot is a *static* observation: `goals_before` and
/// `goals_after` are heuristic counts derived from the AST shape,
/// not live kernel state. conservatively assumes each step
/// reduces the goal count by one until none remain (clamping at
/// zero); Future work will replace this with kernel-driven goal counting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofStepSnapshot {
    /// Zero-based index in the applied-step sequence.
    pub step_index: usize,

    /// Tactic name as observed in the AST — `"apply"`, `"intro"`,
    /// `"rewrite"`, `"smt"`, `"induction"`, … Compound forms
    /// (`"apply foo"`) preserve the apply target; combinators
    /// (`"try"`, `"repeat"`) strip the wrapper and report the inner
    /// tactic's name with a wrapper-name comment.
    pub tactic_name: String,

    /// Heuristic count of goals before this step ran.
    pub goals_before: usize,

    /// Heuristic count of goals after this step ran (clamped to 0).
    pub goals_after: usize,

    /// Free-form annotation: source-derived hint such as the apply
    /// target, the inducted-on variable, or the rewrite hypothesis
    /// name. Empty when no extra context is observable.
    pub comment: String,
}

impl ProofStepSnapshot {
    /// Construct a snapshot entry.
    pub fn new(
        step_index: usize,
        tactic_name: impl Into<String>,
        goals_before: usize,
        goals_after: usize,
        comment: impl Into<String>,
    ) -> Self {
        Self {
            step_index,
            tactic_name: tactic_name.into(),
            goals_before,
            goals_after,
            comment: comment.into(),
        }
    }
}

// =============================================================================
// ContextSnapshot — visible bindings at the snapshot point
// =============================================================================

/// Static-snapshot view of the surrounding context — declared
/// hypotheses + their count.
///

/// emits an empty context unless the theorem declares explicit
/// `requires` clauses; in that case each clause becomes a synthetic
/// `pre_<n>` named-hypothesis entry with an opaque placeholder type.
/// Future work will plug the elaborator's ElabContext through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// Cardinality of [`Self::declared_at`]. Carried explicitly for
    /// the JSON consumer that doesn't want to walk the list.
    pub hypothesis_count: usize,

    /// Hypotheses declared in the source position (theorem
    /// `requires`, surrounding `let` bindings, etc.).
    pub declared_at: Vec<NamedHypothesis>,
}

impl ContextSnapshot {
    /// Construct an empty context snapshot.
    pub fn empty() -> Self {
        Self {
            hypothesis_count: 0,
            declared_at: Vec::new(),
        }
    }

    /// Construct a context snapshot from a hypothesis list. The
    /// `hypothesis_count` field is derived from the list length.
    pub fn from_hypotheses(declared_at: Vec<NamedHypothesis>) -> Self {
        Self {
            hypothesis_count: declared_at.len(),
            declared_at,
        }
    }
}

// =============================================================================
// ProofState — the snapshot's top-level shape
// =============================================================================

/// Full static snapshot of a theorem's proof body at V0 fidelity.
///

/// Produced by [`snapshot_proof_state`]. Serializes cleanly so the
/// CLI can emit it as JSON (`verum proofview --format json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofState {
    /// Outstanding proof obligations. emits exactly one goal
    /// (the theorem's stated proposition) when no terminal tactic
    /// has been observed; emits zero when the proof body is judged
    /// to close the goal.
    pub remaining_goals: Vec<ProofGoal>,

    /// Sequence of applied tactic steps in source order.
    pub applied_steps: Vec<ProofStepSnapshot>,

    /// Snapshot of the surrounding context at the proof entry point.
    pub current_context: ContextSnapshot,
}

impl ProofState {
    /// Construct an explicit proof state. Most callers should use
    /// [`snapshot_proof_state`] instead; this constructor exists for
    /// Future-work wiring and round-trip tests.
    pub fn new(
        remaining_goals: Vec<ProofGoal>,
        applied_steps: Vec<ProofStepSnapshot>,
        current_context: ContextSnapshot,
    ) -> Self {
        Self {
            remaining_goals,
            applied_steps,
            current_context,
        }
    }

    /// Total step count — convenience accessor for callers that
    /// don't need the individual snapshots.
    pub fn step_count(&self) -> usize {
        self.applied_steps.len()
    }

    /// Whether the snapshot judges the proof body as closing every
    /// goal (i.e. `remaining_goals` is empty). heuristic.
    pub fn is_closed(&self) -> bool {
        self.remaining_goals.is_empty()
    }
}

// =============================================================================
// snapshot_proof_state — the static-analysis hook
// =============================================================================

/// Walk the AST proof body of a `TheoremDecl` and produce a static
/// snapshot of its V0 proof state. Returns `None` for theorems
/// with no proof body (e.g. axioms or unproven theorems).
///

/// **No kernel invocation.** V0 is a heuristic, not a verdict — it
/// answers "what tactics did I write?" without claiming "and they
/// produce a valid proof".
///

/// ## Heuristic for [`ProofState::remaining_goals`]
///

///  * **Empty body** (`proof { }` with zero steps) → 1 goal
///  (the theorem's proposition) remains open.
///  * **Body with at least one step** → 0 goals remaining. V0
///  assumes every written step closes a goal; Future work will plug the
///  real kernel verdict in.
///

/// ## Heuristic for [`ProofState::applied_steps`]
///

/// Sequential walk of the body, producing one
/// [`ProofStepSnapshot`] per tactic-shaped item. Combinators
/// (`Seq`, `Try`, `Repeat`, `AllGoals`, `Focus`) flatten to their
/// inner tactics with a wrapper-comment annotation. Structured
/// proofs (`ProofBody::Structured`) emit one entry per
/// [`ProofStep`].
pub fn snapshot_proof_state(theorem_decl: &TheoremDecl) -> Option<ProofState> {
    let body = match &theorem_decl.proof {
        verum_common::Maybe::Some(b) => b,
        verum_common::Maybe::None => return None,
    };

    // Build the synthetic context from the theorem's declared
    // `requires` clauses. emits one synthetic hypothesis per
    // clause with an opaque type (real type would require elaborator).
    let context = synthetic_context_from_theorem(theorem_decl);

    // Collect the applied-step list by walking the body.
    let mut steps: Vec<ProofStepSnapshot> = Vec::new();
    walk_proof_body(body, &mut steps);

    // Re-index steps in linear order — combinator flattening may
    // have produced indices in walk order, but we want the final
    // sequence to be 0..N-1 contiguous.
    for (idx, snap) in steps.iter_mut().enumerate() {
        snap.step_index = idx;
    }

    // Heuristic remaining-goals count. See doc comment.
    let proposition = ReflectedTerm::Universe { level: 0 };
    let remaining_goals = if steps.is_empty() {
        vec![ProofGoal::with_hypotheses(
            proposition,
            context.declared_at.clone(),
        )]
    } else {
        Vec::new()
    };

    Some(ProofState::new(remaining_goals, steps, context))
}

// =============================================================================
// Internals — proof-body walker + tactic-name extraction
// =============================================================================

/// Construct the synthetic context from a theorem's `requires` clauses.
/// One opaque hypothesis per clause, named `pre_<n>`.
fn synthetic_context_from_theorem(theorem_decl: &TheoremDecl) -> ContextSnapshot {
    let hypotheses: Vec<NamedHypothesis> = theorem_decl
        .requires
        .iter()
        .enumerate()
        .map(|(i, _)| NamedHypothesis::opaque(format!("pre_{}", i)))
        .collect();
    ContextSnapshot::from_hypotheses(hypotheses)
}

/// Walk a `ProofBody` and append one [`ProofStepSnapshot`] per
/// observed tactic. Index assignment happens in
/// [`snapshot_proof_state`] after the walk completes.
fn walk_proof_body(body: &ProofBody, out: &mut Vec<ProofStepSnapshot>) {
    match body {
        ProofBody::Term(_) => {
            // Direct Curry-Howard term — represented as one synthetic
            // step labelled `"term"` so the snapshot reflects the
            // single act of providing the term.
            out.push(make_step_snapshot(
                0, // re-indexed later
                "term",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                "explicit proof term provided",
            ));
        }
        ProofBody::Tactic(tactic) => {
            walk_tactic(tactic, "", out);
        }
        ProofBody::Structured(structure) => {
            walk_structure(structure, out);
        }
        ProofBody::ByMethod(method) => {
            walk_method(method, out);
        }
    }
}

/// Append one snapshot per tactic in a [`TacticExpr`] tree. The
/// `wrapper_comment` carries the surrounding combinator's name so
/// flattened tactics retain their context (e.g. `try apply foo;`
/// emits one entry with `tactic_name = "apply"` and
/// `comment = "wrapped in try; apply target: foo"`).
fn walk_tactic(tactic: &TacticExpr, wrapper_comment: &str, out: &mut Vec<ProofStepSnapshot>) {
    match tactic {
        TacticExpr::Seq(items) => {
            for item in items.iter() {
                walk_tactic(item, wrapper_comment, out);
            }
        }
        TacticExpr::Try(inner) => {
            walk_tactic(
                inner.as_ref(),
                &combine_comment(wrapper_comment, "wrapped in try"),
                out,
            );
        }
        TacticExpr::TryElse { body, fallback } => {
            walk_tactic(
                body.as_ref(),
                &combine_comment(wrapper_comment, "wrapped in try-else (body)"),
                out,
            );
            walk_tactic(
                fallback.as_ref(),
                &combine_comment(wrapper_comment, "wrapped in try-else (fallback)"),
                out,
            );
        }
        TacticExpr::Repeat(inner) => {
            walk_tactic(
                inner.as_ref(),
                &combine_comment(wrapper_comment, "wrapped in repeat"),
                out,
            );
        }
        TacticExpr::Alt(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_tactic(
                    item,
                    &combine_comment(wrapper_comment, &format!("wrapped in alt branch {}", i)),
                    out,
                );
            }
        }
        TacticExpr::AllGoals(inner) => {
            walk_tactic(
                inner.as_ref(),
                &combine_comment(wrapper_comment, "applied to all goals"),
                out,
            );
        }
        TacticExpr::Focus(inner) => {
            walk_tactic(
                inner.as_ref(),
                &combine_comment(wrapper_comment, "wrapped in focus"),
                out,
            );
        }
        // Leaf tactics — emit one snapshot apiece.
        leaf => {
            let (name, comment) = describe_leaf_tactic(leaf);
            let combined_comment = combine_comment(wrapper_comment, &comment);
            out.push(make_step_snapshot(
                0, // re-indexed later
                name,
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                combined_comment,
            ));
        }
    }
}

/// Walk a structured proof. Each [`ProofStep`] becomes one snapshot
/// entry; the optional concluding tactic appends one more.
fn walk_structure(structure: &ProofStructure, out: &mut Vec<ProofStepSnapshot>) {
    for step in structure.steps.iter() {
        walk_step(step, out);
    }
    if let verum_common::Maybe::Some(concl) = &structure.conclusion {
        walk_tactic(concl, "concluding tactic", out);
    }
}

/// Walk one [`ProofStep`].
fn walk_step(step: &ProofStep, out: &mut Vec<ProofStepSnapshot>) {
    match &step.kind {
        ProofStepKind::Have {
            name,
            justification,
            ..
        } => {
            walk_tactic(justification, &format!("have {}", name.as_str()), out);
        }
        ProofStepKind::Show { justification, .. } => {
            walk_tactic(justification, "show", out);
        }
        ProofStepKind::Suffices { justification, .. } => {
            walk_tactic(justification, "suffices", out);
        }
        ProofStepKind::Let { .. } => {
            out.push(make_step_snapshot(
                0,
                "let",
                snapshot_step_count(out),
                snapshot_step_count(out),
                "local binding",
            ));
        }
        ProofStepKind::Obtain { .. } => {
            out.push(make_step_snapshot(
                0,
                "obtain",
                snapshot_step_count(out),
                snapshot_step_count(out),
                "existential witness",
            ));
        }
        ProofStepKind::Calc(_) => {
            out.push(make_step_snapshot(
                0,
                "calc",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                "calculation chain",
            ));
        }
        ProofStepKind::Cases { cases, .. } => {
            out.push(make_step_snapshot(
                0,
                "cases",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!("case split into {} branches", cases.len()),
            ));
            for case in cases.iter() {
                for s in case.proof.iter() {
                    walk_step(s, out);
                }
            }
        }
        ProofStepKind::Focus { goal_index, steps } => {
            for s in steps.iter() {
                walk_step(s, out);
            }
            // Override the wrapper comment on the final focused step
            // by appending a focus-marker snapshot.
            out.push(make_step_snapshot(
                0,
                "focus",
                snapshot_step_count(out),
                snapshot_step_count(out),
                format!("focused on goal {}", goal_index),
            ));
        }
        ProofStepKind::Tactic(t) => {
            walk_tactic(t, "", out);
        }
    }
}

/// Walk a [`ProofMethod`]. Each method emits one summary snapshot
/// plus per-case sub-steps.
fn walk_method(method: &ProofMethod, out: &mut Vec<ProofStepSnapshot>) {
    match method {
        ProofMethod::Induction { on, cases } => {
            let on_name = match on {
                verum_common::Maybe::Some(n) => format!("on {}", n.as_str()),
                verum_common::Maybe::None => "automatic".to_string(),
            };
            out.push(make_step_snapshot(
                0,
                "induction",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!("proof by induction ({}); {} cases", on_name, cases.len()),
            ));
            for case in cases.iter() {
                for s in case.proof.iter() {
                    walk_step(s, out);
                }
            }
        }
        ProofMethod::Cases { cases, .. } => {
            out.push(make_step_snapshot(
                0,
                "cases",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!("proof by cases; {} cases", cases.len()),
            ));
            for case in cases.iter() {
                for s in case.proof.iter() {
                    walk_step(s, out);
                }
            }
        }
        ProofMethod::Contradiction { assumption, proof } => {
            out.push(make_step_snapshot(
                0,
                "contradiction",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!("proof by contradiction (assume {})", assumption.as_str()),
            ));
            for s in proof.iter() {
                walk_step(s, out);
            }
        }
        ProofMethod::StrongInduction { on, cases } => {
            out.push(make_step_snapshot(
                0,
                "strong_induction",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!(
                    "proof by strong induction (on {}); {} cases",
                    on.as_str(),
                    cases.len()
                ),
            ));
            for case in cases.iter() {
                for s in case.proof.iter() {
                    walk_step(s, out);
                }
            }
        }
        ProofMethod::WellFoundedInduction { on, cases, .. } => {
            out.push(make_step_snapshot(
                0,
                "well_founded_induction",
                snapshot_step_count(out),
                snapshot_step_count(out).saturating_sub(1),
                format!(
                    "proof by well-founded induction (on {}); {} cases",
                    on.as_str(),
                    cases.len()
                ),
            ));
            for case in cases.iter() {
                for s in case.proof.iter() {
                    walk_step(s, out);
                }
            }
        }
    }
}

/// Render a leaf [`TacticExpr`] as `(name, comment)`. Combinator
/// variants must be intercepted upstream by [`walk_tactic`]; if a
/// combinator falls through here it's treated as opaque.
fn describe_leaf_tactic(tactic: &TacticExpr) -> (&'static str, String) {
    match tactic {
        TacticExpr::Trivial => ("trivial", String::new()),
        TacticExpr::Assumption => ("assumption", String::new()),
        TacticExpr::Reflexivity => ("refl", String::new()),
        TacticExpr::Intro(idents) => {
            let names: Vec<&str> = idents.iter().map(|i| i.as_str()).collect();
            (
                "intro",
                if names.is_empty() {
                    String::new()
                } else {
                    format!("introduces {}", names.join(", "))
                },
            )
        }
        TacticExpr::Apply { lemma, .. } => {
            let target = render_apply_target(lemma);
            (
                "apply",
                if target.is_empty() {
                    String::new()
                } else {
                    format!("apply target: {}", target)
                },
            )
        }
        TacticExpr::Rewrite {
            hypothesis, rev, ..
        } => {
            let target = render_apply_target(hypothesis);
            let direction = if *rev { "reverse" } else { "forward" };
            (
                "rewrite",
                format!("rewrite ({}) using {}", direction, target),
            )
        }
        TacticExpr::Simp { lemmas, .. } => (
            "simp",
            if lemmas.is_empty() {
                String::new()
            } else {
                format!("with {} lemma(s)", lemmas.len())
            },
        ),
        TacticExpr::Ring => ("ring", String::new()),
        TacticExpr::Field => ("field", String::new()),
        TacticExpr::Omega => ("omega", String::new()),
        TacticExpr::Auto { with_hints } => (
            "auto",
            if with_hints.is_empty() {
                String::new()
            } else {
                format!(
                    "with hints: {}",
                    with_hints
                        .iter()
                        .map(|i| i.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        ),
        TacticExpr::Blast => ("blast", String::new()),
        TacticExpr::Smt { solver, timeout } => {
            let mut bits: Vec<String> = Vec::new();
            if let verum_common::Maybe::Some(s) = solver {
                bits.push(format!("solver={}", s.as_str()));
            }
            if let verum_common::Maybe::Some(t) = timeout {
                bits.push(format!("timeout={}ms", t));
            }
            ("smt", bits.join(", "))
        }
        TacticExpr::Split => ("split", String::new()),
        TacticExpr::Left => ("left", String::new()),
        TacticExpr::Right => ("right", String::new()),
        TacticExpr::Exists(_) => ("exists", "existential witness".to_string()),
        TacticExpr::CasesOn(name) => ("cases", format!("on {}", name.as_str())),
        TacticExpr::InductionOn(name) => ("induction", format!("on {}", name.as_str())),
        TacticExpr::Exact(_) => ("exact", "explicit term".to_string()),
        TacticExpr::Unfold(idents) => (
            "unfold",
            idents
                .iter()
                .map(|i| i.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
        TacticExpr::Compute => ("compute", String::new()),
        TacticExpr::Named { name, args, .. } => {
            // Static dispatch on a named tactic — surface the name
            // verbatim plus the arity for context.
            // We can't return a borrowed &'static str for the user-
            // chosen name, so use a sentinel and stash the real name
            // in the comment.
            (
                "named",
                format!("invoke `{}` with {} argument(s)", name.as_str(), args.len()),
            )
        }
        TacticExpr::Let { name, .. } => ("let", format!("bind {}", name.as_str())),
        TacticExpr::Match { arms, .. } => ("match", format!("{} arm(s)", arms.len())),
        TacticExpr::Fail { .. } => ("fail", "explicit failure".to_string()),
        TacticExpr::If { .. } => ("if", "conditional tactic".to_string()),
        TacticExpr::Done => ("done", String::new()),
        TacticExpr::Admit => ("admit", "leaves goal unproven".to_string()),
        TacticExpr::Sorry => ("sorry", "leaves goal unproven".to_string()),
        TacticExpr::Contradiction => ("contradiction", String::new()),
        // Combinator forms — should not reach here since walk_tactic
        // intercepts them upstream. If they do, treat them as opaque.
        TacticExpr::Seq(_)
        | TacticExpr::Try(_)
        | TacticExpr::TryElse { .. }
        | TacticExpr::Repeat(_)
        | TacticExpr::Alt(_)
        | TacticExpr::AllGoals(_)
        | TacticExpr::Focus(_) => ("compound", "tactic combinator".to_string()),
    }
}

/// Render an apply / rewrite target expression to a string. Falls
/// back to `"<expr>"` when the expression isn't a simple path.
fn render_apply_target(expr: &verum_ast::expr::Expr) -> String {
    match &expr.kind {
        verum_ast::expr::ExprKind::Path(path) => {
            let segments: Vec<String> = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                    _ => None,
                })
                .collect();
            if segments.is_empty() {
                "<unknown>".to_string()
            } else {
                segments.join(".")
            }
        }
        _ => "<expr>".to_string(),
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Convenience constructor used by the walker. `step_index` is set
/// to a placeholder of zero — [`snapshot_proof_state`] re-indexes the
/// final list contiguously after the walk completes.
fn make_step_snapshot(
    step_index: usize,
    tactic_name: impl Into<String>,
    goals_before: usize,
    goals_after: usize,
    comment: impl Into<String>,
) -> ProofStepSnapshot {
    ProofStepSnapshot::new(step_index, tactic_name, goals_before, goals_after, comment)
}

/// Combine a wrapper-context comment with a leaf-tactic comment.
/// Empty pieces are dropped so we don't end up with `"; foo"` /
/// `"foo; "`.
fn combine_comment(wrapper: &str, inner: &str) -> String {
    match (wrapper.is_empty(), inner.is_empty()) {
        (true, true) => String::new(),
        (true, false) => inner.to_string(),
        (false, true) => wrapper.to_string(),
        (false, false) => format!("{}; {}", wrapper, inner),
    }
}

/// Convenience: number of steps already collected, used for the
/// `goals_before` heuristic (V0 assumes one goal at the top of the
/// body and decrements toward zero). Returns `1` while the list is
/// empty and `1` once any step has been emitted (V0 doesn't
/// simulate multi-goal proofs).
fn snapshot_step_count(out: &[ProofStepSnapshot]) -> usize {
    if out.is_empty() { 1 } else { 1 }
}

/// placeholder for "we couldn't reflect the real type". Always
/// returns `Universe(0)` so downstream consumers see a syntactically
/// valid reflected term and don't have to handle a `None` case.
pub fn opaque_placeholder() -> ReflectedTerm {
    ReflectedTerm::Universe { level: 0 }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use verum_ast::Ident;
    use verum_ast::decl::{
        ProofBody, ProofMethod, ProofStep, ProofStepKind, ProofStructure, TacticExpr, TheoremDecl,
    };
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::Literal;
    use verum_ast::span::Span;
    use verum_ast::ty::Path;
    use verum_common::{Heap, List, Maybe};

    // -------------------------------------------------------------------------
    // Test fixtures
    // -------------------------------------------------------------------------

    fn dummy_span() -> Span {
        Span::dummy()
    }

    /// Build a trivial `true` proposition expression.
    fn trivial_proposition() -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::bool(true, dummy_span())),
            dummy_span(),
        )
    }

    /// Build a `Path` expression like `foo` or `lemma_name`.
    fn path_expr(name: &str) -> Expr {
        let ident = Ident::new(name, dummy_span());
        let path = Path::single(ident);
        Expr::new(ExprKind::Path(path), dummy_span())
    }

    /// Build a theorem with the given proof body.
    fn theorem_with_proof(name: &str, proof: Option<ProofBody>) -> TheoremDecl {
        let mut t = TheoremDecl::new(
            Ident::new(name, dummy_span()),
            trivial_proposition(),
            dummy_span(),
        );
        t.proof = match proof {
            Some(b) => Maybe::Some(b),
            None => Maybe::None,
        };
        t
    }

    fn empty_proof_body() -> ProofBody {
        ProofBody::Structured(ProofStructure {
            steps: List::new(),
            conclusion: Maybe::None,
            span: dummy_span(),
        })
    }

    fn apply_tactic(target: &str) -> TacticExpr {
        TacticExpr::Apply {
            lemma: Heap::new(path_expr(target)),
            args: List::new(),
        }
    }

    fn three_step_structured_body() -> ProofBody {
        let mut steps: List<ProofStep> = List::new();
        steps.push(ProofStep {
            kind: ProofStepKind::Tactic(apply_tactic("foo")),
            span: dummy_span(),
        });
        steps.push(ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Reflexivity),
            span: dummy_span(),
        });
        steps.push(ProofStep {
            kind: ProofStepKind::Tactic(TacticExpr::Done),
            span: dummy_span(),
        });
        ProofBody::Structured(ProofStructure {
            steps,
            conclusion: Maybe::None,
            span: dummy_span(),
        })
    }

    // -------------------------------------------------------------------------
    // snapshot_proof_state behavioural tests
    // -------------------------------------------------------------------------

    #[test]
    fn snapshot_returns_none_for_axiom() {
        let theorem = theorem_with_proof("axiom_like", None);
        assert!(snapshot_proof_state(&theorem).is_none());
    }

    #[test]
    fn snapshot_empty_proof_body_yields_one_open_goal_zero_steps() {
        let theorem = theorem_with_proof("trivial_thm", Some(empty_proof_body()));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 0);
        assert_eq!(state.remaining_goals.len(), 1);
        assert!(!state.is_closed());
    }

    #[test]
    fn snapshot_single_apply_step_zero_goals_one_step() {
        let body = ProofBody::Tactic(apply_tactic("foo"));
        let theorem = theorem_with_proof("apply_only", Some(body));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 1);
        assert_eq!(state.remaining_goals.len(), 0);
        assert!(state.is_closed());

        let step = &state.applied_steps[0];
        assert_eq!(step.tactic_name, "apply");
        assert_eq!(step.step_index, 0);
        assert!(
            step.comment.contains("foo"),
            "expected apply target in comment, got {:?}",
            step.comment
        );
    }

    #[test]
    fn snapshot_three_step_structured_body_emits_three_steps() {
        let theorem = theorem_with_proof("three_steps", Some(three_step_structured_body()));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 3);
        assert_eq!(state.remaining_goals.len(), 0);
        assert_eq!(state.applied_steps[0].tactic_name, "apply");
        assert_eq!(state.applied_steps[1].tactic_name, "refl");
        assert_eq!(state.applied_steps[2].tactic_name, "done");
        // Verify contiguous indexing.
        for (i, s) in state.applied_steps.iter().enumerate() {
            assert_eq!(s.step_index, i);
        }
    }

    #[test]
    fn snapshot_proof_by_induction_uses_by_method_shape() {
        let body = ProofBody::ByMethod(ProofMethod::Induction {
            on: Maybe::Some(Ident::new("n", dummy_span())),
            cases: List::new(),
        });
        let theorem = theorem_with_proof("induction_thm", Some(body));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 1);
        assert_eq!(state.applied_steps[0].tactic_name, "induction");
        assert!(
            state.applied_steps[0].comment.contains("on n"),
            "expected `on n` in comment, got {:?}",
            state.applied_steps[0].comment,
        );
        assert_eq!(state.remaining_goals.len(), 0);
    }

    #[test]
    fn snapshot_term_body_emits_synthetic_term_step() {
        let body = ProofBody::Term(Heap::new(trivial_proposition()));
        let theorem = theorem_with_proof("term_thm", Some(body));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 1);
        assert_eq!(state.applied_steps[0].tactic_name, "term");
    }

    #[test]
    fn snapshot_seq_combinator_flattens_to_inner_tactics() {
        let mut items: List<TacticExpr> = List::new();
        items.push(apply_tactic("first_lemma"));
        items.push(TacticExpr::Reflexivity);
        items.push(TacticExpr::Done);
        let body = ProofBody::Tactic(TacticExpr::Seq(items));
        let theorem = theorem_with_proof("seq_thm", Some(body));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 3);
        assert_eq!(state.applied_steps[0].tactic_name, "apply");
        assert_eq!(state.applied_steps[1].tactic_name, "refl");
        assert_eq!(state.applied_steps[2].tactic_name, "done");
    }

    #[test]
    fn snapshot_try_combinator_annotates_inner_tactic() {
        let body = ProofBody::Tactic(TacticExpr::Try(Heap::new(apply_tactic("optional_lemma"))));
        let theorem = theorem_with_proof("try_thm", Some(body));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.applied_steps.len(), 1);
        let step = &state.applied_steps[0];
        assert_eq!(step.tactic_name, "apply");
        assert!(step.comment.contains("try"));
        assert!(step.comment.contains("optional_lemma"));
    }

    // -------------------------------------------------------------------------
    // Serde round-trip tests
    // -------------------------------------------------------------------------

    #[test]
    fn serde_roundtrip_named_hypothesis() {
        let h = NamedHypothesis::new("x", ReflectedTerm::Universe { level: 1 });
        let json = serde_json::to_string(&h).expect("serialise");
        let back: NamedHypothesis = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(h, back);
    }

    #[test]
    fn serde_roundtrip_proof_goal() {
        let g = ProofGoal::with_hypotheses(
            ReflectedTerm::Var { index: 0 },
            vec![NamedHypothesis::opaque("h1")],
        );
        let json = serde_json::to_string(&g).expect("serialise");
        let back: ProofGoal = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(g, back);
    }

    #[test]
    fn serde_roundtrip_proof_step_snapshot() {
        let s = ProofStepSnapshot::new(2, "apply", 1, 0, "apply target: foo");
        let json = serde_json::to_string(&s).expect("serialise");
        let back: ProofStepSnapshot = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(s, back);
    }

    #[test]
    fn serde_roundtrip_context_snapshot() {
        let c = ContextSnapshot::from_hypotheses(vec![
            NamedHypothesis::opaque("a"),
            NamedHypothesis::opaque("b"),
        ]);
        let json = serde_json::to_string(&c).expect("serialise");
        let back: ContextSnapshot = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(c, back);
        assert_eq!(back.hypothesis_count, 2);
    }

    #[test]
    fn serde_roundtrip_proof_state_full_shape() {
        let theorem = theorem_with_proof("rt_thm", Some(three_step_structured_body()));
        let state = snapshot_proof_state(&theorem).expect("body present");
        let json = serde_json::to_string_pretty(&state).expect("serialise");
        let back: ProofState = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(state, back);
    }

    // -------------------------------------------------------------------------
    // Helpers + opaque placeholder
    // -------------------------------------------------------------------------

    #[test]
    fn opaque_placeholder_is_universe_zero() {
        assert_eq!(opaque_placeholder(), ReflectedTerm::Universe { level: 0 });
    }

    #[test]
    fn step_count_helper_returns_zero_for_empty_state() {
        let theorem = theorem_with_proof("empty", Some(empty_proof_body()));
        let state = snapshot_proof_state(&theorem).expect("body present");
        assert_eq!(state.step_count(), 0);
    }
}
