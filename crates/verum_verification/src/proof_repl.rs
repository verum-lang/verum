//! Live proof REPL — stepwise tactic feedback + proof-tree
//! visualisation.
//!

//! ## Goal
//!

//! Mathematicians need a workflow where every tactic produces
//! immediate, kernel-grade feedback: did the step type-check, what
//! does the new goal stack look like, and what's the proof tree
//! built so far? This module ships the **protocol** + **state
//! machine** that drive such a REPL. Interactive TUI is a UI
//! concern (`verum_interactive`); LSP integration is a separate
//! transport — both consume the same trait surface defined here.
//!

//! ## Architectural pattern
//!

//! Same single-trait-boundary pattern as the rest of the integration
//! arc (ladder_dispatch / proof_drafting / proof_repair / closure_cache
//! / doc_render / foreign_import / llm_tactic):
//!

//!  * [`ReplCommand`] — typed enum of every command the user can
//!  issue (`Apply` a tactic, `Undo` the last step, request a
//!  `Hint`, ask for the `ProofTree`, etc.).
//!  * [`ReplResponse`] — typed enum of every possible response
//!  (Accepted / Rejected / Status / Tree / Hints / etc.).
//!  * [`ReplSession`] trait — single dispatch interface;
//!  `step(command) -> response`.
//!  * [`DefaultReplSession`] — reference implementation that wires
//!  a [`crate::llm_tactic::KernelChecker`] for step verification +
//!  [`crate::proof_drafting::DefaultSuggestionEngine`] for hints.
//!  Maintains an internal history stack for undo / redo.
//!  * [`ProofTreeNode`] / [`ProofTree`] — typed DAG of accepted
//!  steps with kernel verdicts and elapsed times. Renders to
//!  Graphviz DOT for `:visualise`.
//!

//! ## Stepwise feedback contract
//!

//! Every tactic application produces:
//!

//!  * The kernel verdict (Accepted / Rejected with cause).
//!  * Wall-clock duration in milliseconds.
//!  * The updated proof state (open-goal stack snapshot).
//!  * A node in the proof tree linking the step to the goal it
//!  was applied to.
//!

//! Rejected steps DO NOT mutate the proof state — the LCF
//! fail-closed contract carries through from `llm_tactic`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::llm_tactic::{KernelChecker, LlmGoalSummary, PatternKernelChecker};
use crate::proof_drafting::{
    DefaultSuggestionEngine, HypothesisSummary, LemmaSummary, ProofGoalSummary, ProofStateView,
    SuggestionEngine, TacticSuggestion,
};
use verum_common::Text;

// =============================================================================
// ReplCommand — the input surface
// =============================================================================

/// One command the user issues to the REPL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ReplCommand {
    /// Apply a tactic (e.g. `"intro"`, `"apply foo_lemma"`,
    /// `"auto"`). The kernel re-checks before mutating state.
    Apply { tactic: Text },
    /// Undo the last accepted step. No-op when history is empty.
    Undo,
    /// Re-apply the most recently undone step. No-op when the
    /// redo stack is empty.
    Redo,
    /// Print the open-goal stack.
    ShowGoals,
    /// Print the local context (hypotheses + lemmas in scope).
    ShowContext,
    /// Render the current proof tree as Graphviz DOT.
    Visualise,
    /// Ranked next-step suggestions based on the focused goal.
    Hint { max: usize },
    /// Print the session status (theorem, applied steps, redo
    /// stack depth, history depth, kernel verdict count).
    Status,
}

/// Discriminator-only kind for [`ReplCommand`].
///
/// `ReplCommand` carries payloads (`Apply { tactic }` /
/// `Hint { max }`); the kind enum is zero-sized so callers can
/// iterate the surface (for telemetry / docs / autocomplete) without
/// having to supply payload data. The kind tag matches the
/// `#[serde(tag = "kind")]` discriminator on `ReplCommand`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplCommandKind {
    Apply,
    Undo,
    Redo,
    ShowGoals,
    ShowContext,
    Visualise,
    Hint,
    Status,
}

/// Per-kind projection for [`ReplCommandKind`].
///
/// `name` matches the serde tag — the wire form persisted to JSONL
/// audit trails. `is_mutation` flags commands that may mutate the
/// proof state (Apply / Undo / Redo); the rest are read-only
/// (ShowGoals / ShowContext / Visualise / Hint / Status).
#[derive(Debug, Clone, Copy)]
pub struct ReplCommandKindMeta {
    pub name: &'static str,
    pub is_mutation: bool,
}

impl ReplCommandKind {
    pub const ALL: &'static [Self] = &[
        Self::Apply,
        Self::Undo,
        Self::Redo,
        Self::ShowGoals,
        Self::ShowContext,
        Self::Visualise,
        Self::Hint,
        Self::Status,
    ];

    pub const fn meta(self) -> ReplCommandKindMeta {
        match self {
            Self::Apply => ReplCommandKindMeta {
                name: "Apply",
                is_mutation: true,
            },
            Self::Undo => ReplCommandKindMeta {
                name: "Undo",
                is_mutation: true,
            },
            Self::Redo => ReplCommandKindMeta {
                name: "Redo",
                is_mutation: true,
            },
            Self::ShowGoals => ReplCommandKindMeta {
                name: "ShowGoals",
                is_mutation: false,
            },
            Self::ShowContext => ReplCommandKindMeta {
                name: "ShowContext",
                is_mutation: false,
            },
            Self::Visualise => ReplCommandKindMeta {
                name: "Visualise",
                is_mutation: false,
            },
            Self::Hint => ReplCommandKindMeta {
                name: "Hint",
                is_mutation: false,
            },
            Self::Status => ReplCommandKindMeta {
                name: "Status",
                is_mutation: false,
            },
        }
    }

    /// Canonical PascalCase tag (matches the `#[serde(tag="kind")]`
    /// wire form on [`ReplCommand`]).
    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for k in Self::ALL {
            if k.meta().name == s {
                return Some(*k);
            }
        }
        None
    }

    /// True for commands that may mutate proof state (Apply / Undo
    /// / Redo). Read-only commands (ShowGoals / ShowContext /
    /// Visualise / Hint / Status) flag false.
    #[inline]
    pub const fn is_mutation(&self) -> bool {
        self.meta().is_mutation
    }
}

impl ReplCommand {
    /// Discriminator-only kind for telemetry / surface enumeration.
    pub fn kind(&self) -> ReplCommandKind {
        match self {
            Self::Apply { .. } => ReplCommandKind::Apply,
            Self::Undo => ReplCommandKind::Undo,
            Self::Redo => ReplCommandKind::Redo,
            Self::ShowGoals => ReplCommandKind::ShowGoals,
            Self::ShowContext => ReplCommandKind::ShowContext,
            Self::Visualise => ReplCommandKind::Visualise,
            Self::Hint { .. } => ReplCommandKind::Hint,
            Self::Status => ReplCommandKind::Status,
        }
    }
}

// =============================================================================
// Hypothesis / Goal / GoalStack — typed proof-state representation
// =============================================================================

/// A local hypothesis in scope for one goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    /// Identifier (e.g. `h`, `IH`).
    pub name: Text,
    /// Rendered type / proposition. V0 stores a string; Future work will
    /// promote this to a typed kernel term once the kernel
    /// integration is wired through.
    pub ty: Text,
}

/// One open goal. Goals carry typed hypotheses + a proposition,
/// not just a single rendered string — replacing the V0 stringly-
/// typed `Vec<Text>` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    /// Stable identifier (monotonically increasing within a session).
    pub goal_id: u64,
    /// Optional label introduced by `case foo => …` / `intro_as foo : T`.
    pub label: Option<Text>,
    /// Local hypothesis context.
    pub hypotheses: Vec<Hypothesis>,
    /// The proposition this goal must close.
    pub proposition: Text,
}

/// Open-goal stack with an explicit focus pointer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalStack {
    /// Open goals. Empty when the proof is closed.
    pub goals: Vec<Goal>,
    /// Index into `goals` that the next tactic targets. `None` when
    /// the stack is empty (proof complete).
    pub focused: Option<usize>,
    /// Next goal identifier to allocate.
    pub next_id: u64,
}

impl GoalStack {
    /// A stack with one root goal carrying no hypotheses.
    pub fn singleton(initial: impl Into<Text>) -> Self {
        Self {
            goals: vec![Goal {
                goal_id: 0,
                label: None,
                hypotheses: Vec::new(),
                proposition: initial.into(),
            }],
            focused: Some(0),
            next_id: 1,
        }
    }

    /// Borrow the focused goal, if any.
    pub fn focused_goal(&self) -> Option<&Goal> {
        self.focused.and_then(|i| self.goals.get(i))
    }

    /// Mutably borrow the focused goal, if any.
    pub fn focused_goal_mut(&mut self) -> Option<&mut Goal> {
        let i = self.focused?;
        self.goals.get_mut(i)
    }

    /// Drop the focused goal; refocus on the next available one (or
    /// `None` if the stack is now empty).
    pub fn close_focused(&mut self) {
        let Some(i) = self.focused else {
            return;
        };
        if i < self.goals.len() {
            self.goals.remove(i);
        }
        self.focused = if self.goals.is_empty() {
            None
        } else if i < self.goals.len() {
            Some(i)
        } else {
            Some(self.goals.len() - 1)
        };
    }

    /// Replace the focused goal with `replacements` (in order); the
    /// first replacement becomes the new focused goal. Hypothesis
    /// context is inherited from the parent goal. No-op when the
    /// stack is empty.
    pub fn split_focused(&mut self, replacements: Vec<Text>) {
        let Some(i) = self.focused else {
            return;
        };
        let parent = match self.goals.get(i).cloned() {
            Some(g) => g,
            None => return,
        };
        let mut new_goals: Vec<Goal> = replacements
            .into_iter()
            .map(|p| {
                let g = Goal {
                    goal_id: self.next_id,
                    label: None,
                    hypotheses: parent.hypotheses.clone(),
                    proposition: p,
                };
                self.next_id += 1;
                g
            })
            .collect();
        if new_goals.is_empty() {
            self.close_focused();
            return;
        }
        // Replace the parent in-place with the new sub-goals. The
        // first replacement keeps the focus.
        self.goals.splice(i..=i, new_goals.drain(..));
    }

    /// Append a hypothesis to the focused goal's context.
    pub fn push_hypothesis(&mut self, hyp: Hypothesis) {
        if let Some(g) = self.focused_goal_mut() {
            g.hypotheses.push(hyp);
        }
    }

    /// Replace the focused goal's proposition.
    pub fn set_focused_proposition(&mut self, prop: Text) {
        if let Some(g) = self.focused_goal_mut() {
            g.proposition = prop;
        }
    }

    /// True iff every goal has been discharged.
    pub fn is_complete(&self) -> bool {
        self.goals.is_empty()
    }
}

// =============================================================================
// ReplResponse — the output surface
// =============================================================================

/// Snapshot of the open-goal stack + applied steps at a point in
/// time.
///

/// **Schema note.** Goals are typed (`Vec<Goal>`) so consumers can
/// inspect hypotheses + propositions independently — replacing the
/// V0 stringly-typed `Vec<Text>` view per the #91 hardening pass.
/// The legacy fields `focused_proposition` and `open_goals` still
/// exist as render-side conveniences but are derived projections,
/// not the source of truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplStateSnapshot {
    pub theorem_name: Text,
    /// Typed open-goal stack. Empty when the proof is complete.
    pub goals: Vec<Goal>,
    /// Stable identifier of the focused goal (if any).
    pub focused_goal_id: Option<u64>,
    /// Convenience projection: the focused goal's proposition, or
    /// the empty string when the proof is complete.
    pub focused_proposition: Text,
    /// Convenience projection: every goal's proposition, in order.
    /// Derived from `goals` for backwards-compat consumers.
    pub open_goals: Vec<Text>,
    pub applied_steps: Vec<Text>,
    pub history_depth: usize,
    pub redo_depth: usize,
}

impl ReplStateSnapshot {
    /// Build a snapshot from a typed [`GoalStack`]. Render fields
    /// are derived; never set them by hand.
    pub fn from_stack(
        theorem_name: Text,
        stack: &GoalStack,
        applied_steps: Vec<Text>,
        history_depth: usize,
        redo_depth: usize,
    ) -> Self {
        let focused_goal_id = stack.focused_goal().map(|g| g.goal_id);
        let focused_proposition = stack
            .focused_goal()
            .map(|g| g.proposition.clone())
            .unwrap_or_else(|| Text::from(""));
        let open_goals: Vec<Text> = stack.goals.iter().map(|g| g.proposition.clone()).collect();
        Self {
            theorem_name,
            goals: stack.goals.clone(),
            focused_goal_id,
            focused_proposition,
            open_goals,
            applied_steps,
            history_depth,
            redo_depth,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ReplResponse {
    /// The tactic was kernel-checked and applied.
    Accepted {
        tactic: Text,
        elapsed_ms: u64,
        snapshot: ReplStateSnapshot,
    },
    /// The kernel rejected the tactic. State is unchanged.
    Rejected {
        tactic: Text,
        reason: Text,
        snapshot: ReplStateSnapshot,
    },
    /// `Undo` succeeded; carries the popped step's name.
    Undone {
        popped: Text,
        snapshot: ReplStateSnapshot,
    },
    /// `Redo` succeeded; carries the re-applied step's name.
    Redone {
        reapplied: Text,
        snapshot: ReplStateSnapshot,
    },
    /// Plain status / show-goals / show-context output.
    Status { snapshot: ReplStateSnapshot },
    /// Goal-shape hints from the suggestion engine.
    Hints { suggestions: Vec<HintSuggestion> },
    /// Graphviz DOT for the current proof tree.
    Tree { dot: Text },
    /// A no-op command (e.g. `Undo` on empty history).
    NoOp { reason: Text },
    /// An error response (malformed command, internal error).
    Error { message: Text },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HintSuggestion {
    pub snippet: Text,
    pub rationale: Text,
    pub score: f64,
    pub category: Text,
}

/// Discriminator-only kind for [`ReplResponse`]. Mirrors the
/// payload-bearing variant set; lets callers iterate the response
/// surface for telemetry / autocompletion / docs without supplying
/// payload data. Tag values match the
/// `#[serde(tag = "kind")]` discriminator on `ReplResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReplResponseKind {
    Accepted,
    Rejected,
    Undone,
    Redone,
    Status,
    Hints,
    Tree,
    NoOp,
    Error,
}

/// Per-kind projection for [`ReplResponseKind`].
///
/// `name` matches the serde tag (PascalCase, the wire form).
/// `is_state_mutation` flags responses that represent a successful
/// state mutation (Accepted / Undone / Redone). `is_error` flags
/// the lone error response — Rejected is NOT an error: the kernel
/// rejected the tactic but the REPL is healthy. Adding a new
/// response variant forces an explicit decision on both flags
/// in `meta()`.
#[derive(Debug, Clone, Copy)]
pub struct ReplResponseKindMeta {
    pub name: &'static str,
    pub is_state_mutation: bool,
    pub is_error: bool,
}

impl ReplResponseKind {
    pub const ALL: &'static [Self] = &[
        Self::Accepted,
        Self::Rejected,
        Self::Undone,
        Self::Redone,
        Self::Status,
        Self::Hints,
        Self::Tree,
        Self::NoOp,
        Self::Error,
    ];

    pub const fn meta(self) -> ReplResponseKindMeta {
        match self {
            Self::Accepted => ReplResponseKindMeta {
                name: "Accepted",
                is_state_mutation: true,
                is_error: false,
            },
            Self::Rejected => ReplResponseKindMeta {
                name: "Rejected",
                is_state_mutation: false,
                is_error: false,
            },
            Self::Undone => ReplResponseKindMeta {
                name: "Undone",
                is_state_mutation: true,
                is_error: false,
            },
            Self::Redone => ReplResponseKindMeta {
                name: "Redone",
                is_state_mutation: true,
                is_error: false,
            },
            Self::Status => ReplResponseKindMeta {
                name: "Status",
                is_state_mutation: false,
                is_error: false,
            },
            Self::Hints => ReplResponseKindMeta {
                name: "Hints",
                is_state_mutation: false,
                is_error: false,
            },
            Self::Tree => ReplResponseKindMeta {
                name: "Tree",
                is_state_mutation: false,
                is_error: false,
            },
            Self::NoOp => ReplResponseKindMeta {
                name: "NoOp",
                is_state_mutation: false,
                is_error: false,
            },
            Self::Error => ReplResponseKindMeta {
                name: "Error",
                is_state_mutation: false,
                is_error: true,
            },
        }
    }

    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for k in Self::ALL {
            if k.meta().name == s {
                return Some(*k);
            }
        }
        None
    }

    #[inline]
    pub const fn is_state_mutation(&self) -> bool {
        self.meta().is_state_mutation
    }

    #[inline]
    pub const fn is_error(&self) -> bool {
        self.meta().is_error
    }
}

impl ReplResponse {
    /// Discriminator-only kind for telemetry / surface enumeration.
    pub fn kind(&self) -> ReplResponseKind {
        match self {
            Self::Accepted { .. } => ReplResponseKind::Accepted,
            Self::Rejected { .. } => ReplResponseKind::Rejected,
            Self::Undone { .. } => ReplResponseKind::Undone,
            Self::Redone { .. } => ReplResponseKind::Redone,
            Self::Status { .. } => ReplResponseKind::Status,
            Self::Hints { .. } => ReplResponseKind::Hints,
            Self::Tree { .. } => ReplResponseKind::Tree,
            Self::NoOp { .. } => ReplResponseKind::NoOp,
            Self::Error { .. } => ReplResponseKind::Error,
        }
    }

    /// True iff the response represents a successful state mutation
    /// (Accepted / Undone / Redone). Backed by `kind().is_state_
    /// mutation()`.
    #[inline]
    pub fn is_state_mutation(&self) -> bool {
        self.kind().is_state_mutation()
    }

    /// Canonical PascalCase tag.
    #[inline]
    pub fn name(&self) -> &'static str {
        self.kind().name()
    }
}

// =============================================================================
// ProofTree — typed DAG of accepted tactic steps
// =============================================================================

/// One node in the proof tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProofTreeNode {
    /// 1-based step number (root is 0 — the initial goal).
    pub step_index: usize,
    /// The tactic applied at this step.
    pub tactic: Text,
    /// The proposition the tactic was applied to (rendered).
    pub goal_at_application: Text,
    /// Kernel verdict elapsed time (ms).
    pub elapsed_ms: u64,
}

/// The accumulated proof tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProofTree {
    pub root_goal: Text,
    pub nodes: Vec<ProofTreeNode>,
}

impl ProofTree {
    /// Render the tree as Graphviz DOT. Each accepted step is a
    /// node; edges are step-index successor pairs.
    pub fn to_dot(&self) -> Text {
        let mut s = String::from("digraph proof_tree {\n");
        s.push_str("  rankdir=TB;\n");
        s.push_str("  node [shape=box, style=rounded];\n");
        // Root.
        s.push_str(&format!(
            "  goal_root [label=\"goal: {}\", style=\"rounded,filled\", fillcolor=lightblue];\n",
            dot_escape(self.root_goal.as_str())
        ));
        for n in &self.nodes {
            s.push_str(&format!(
                "  step_{} [label=\"{}: {}\\n({}ms)\"];\n",
                n.step_index,
                n.step_index,
                dot_escape(n.tactic.as_str()),
                n.elapsed_ms
            ));
        }
        // Edges: root → step1, step1 → step2, …
        if !self.nodes.is_empty() {
            s.push_str(&format!(
                "  goal_root -> step_{};\n",
                self.nodes[0].step_index
            ));
            for w in self.nodes.windows(2) {
                s.push_str(&format!(
                    "  step_{} -> step_{};\n",
                    w[0].step_index, w[1].step_index
                ));
            }
        }
        s.push('}');
        Text::from(s)
    }
}

fn dot_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// =============================================================================
// GoalRewriter — surface tactic → goal-stack mutation
// =============================================================================

/// What a goal-stack rewriter did with one tactic invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRewriteOutcome {
    /// The tactic mutated the focused goal's hypotheses /
    /// proposition (e.g. `intro h`) but didn't close it.
    Rewritten,
    /// The tactic split the focused goal into N ≥ 2 sub-goals.
    Split { count: usize },
    /// The tactic closed the focused goal.
    Closed,
    /// The rewriter doesn't pattern-match this tactic shape. The
    /// caller should leave the state unchanged — the kernel checker
    /// has already validated the step's *soundness*; only the
    /// display-side state-mutation is unknown.
    NoMatch,
    /// The tactic was malformed. Reported for diagnostics; state
    /// unchanged.
    Error { reason: Text },
}

/// Discriminator-only kind for [`GoalRewriteOutcome`].
///
/// `Split` and `Error` carry payloads; the kind enum is zero-sized
/// so callers (telemetry / docs / dispatch tables) can iterate the
/// outcome surface without payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalRewriteOutcomeKind {
    Rewritten,
    Split,
    Closed,
    NoMatch,
    Error,
}

/// Per-kind projection for [`GoalRewriteOutcomeKind`].
///
/// `name` is the snake_case telemetry / log label.
/// `mutates_display_state` flags outcomes where the goal-stack
/// display changed (Rewritten / Split / Closed); NoMatch and Error
/// leave the display unchanged. `closes_goal` is unique to
/// `Closed`. `is_error` is unique to `Error` — note `NoMatch`
/// is NOT an error: the rewriter just doesn't pattern-match this
/// tactic shape, and the kernel checker has already validated
/// the step's soundness independently.
#[derive(Debug, Clone, Copy)]
pub struct GoalRewriteOutcomeKindMeta {
    pub name: &'static str,
    pub mutates_display_state: bool,
    pub closes_goal: bool,
    pub is_error: bool,
}

impl GoalRewriteOutcomeKind {
    pub const ALL: &'static [Self] = &[
        Self::Rewritten,
        Self::Split,
        Self::Closed,
        Self::NoMatch,
        Self::Error,
    ];

    pub const fn meta(self) -> GoalRewriteOutcomeKindMeta {
        match self {
            Self::Rewritten => GoalRewriteOutcomeKindMeta {
                name: "rewritten",
                mutates_display_state: true,
                closes_goal: false,
                is_error: false,
            },
            Self::Split => GoalRewriteOutcomeKindMeta {
                name: "split",
                mutates_display_state: true,
                closes_goal: false,
                is_error: false,
            },
            Self::Closed => GoalRewriteOutcomeKindMeta {
                name: "closed",
                mutates_display_state: true,
                closes_goal: true,
                is_error: false,
            },
            Self::NoMatch => GoalRewriteOutcomeKindMeta {
                name: "no_match",
                mutates_display_state: false,
                closes_goal: false,
                is_error: false,
            },
            Self::Error => GoalRewriteOutcomeKindMeta {
                name: "error",
                mutates_display_state: false,
                closes_goal: false,
                is_error: true,
            },
        }
    }

    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for k in Self::ALL {
            if k.meta().name == s {
                return Some(*k);
            }
        }
        None
    }

    #[inline]
    pub const fn mutates_display_state(&self) -> bool {
        self.meta().mutates_display_state
    }

    #[inline]
    pub const fn closes_goal(&self) -> bool {
        self.meta().closes_goal
    }

    #[inline]
    pub const fn is_error(&self) -> bool {
        self.meta().is_error
    }
}

impl GoalRewriteOutcome {
    /// Discriminator-only kind for telemetry / surface enumeration.
    pub fn kind(&self) -> GoalRewriteOutcomeKind {
        match self {
            Self::Rewritten => GoalRewriteOutcomeKind::Rewritten,
            Self::Split { .. } => GoalRewriteOutcomeKind::Split,
            Self::Closed => GoalRewriteOutcomeKind::Closed,
            Self::NoMatch => GoalRewriteOutcomeKind::NoMatch,
            Self::Error { .. } => GoalRewriteOutcomeKind::Error,
        }
    }
}

/// Single dispatch interface for surface-tactic → goal-stack
/// rewriters.
///

/// Implementations are display-side: they describe how the
/// open-goal *display* mutates after a kernel-accepted tactic. The
/// kernel checker (`llm_tactic::KernelChecker`) is the soundness
/// gate; the rewriter is not. This separation keeps the rewriter
/// free to over-approximate (a known textual shape is rewritten;
/// everything else stays as `NoMatch`) without compromising
/// soundness.
pub trait GoalRewriter: std::fmt::Debug + Send + Sync {
    fn rewrite(&self, stack: &mut GoalStack, tactic: &str) -> GoalRewriteOutcome;
}

/// V0 reference rewriter. Recognises the canonical surface-tactic
/// shapes:
///

///  * `intro` / `intro h` / `intros h1 h2 …` — peel hypothesis off
///  an `H -> P` shape. When the focused proposition doesn't
///  textually parse as an implication, falls back to appending
///  a hypothesis with type `?` so the LLM-side context still
///  records the bound name.
///  * `split` — split a top-level `P ∧ Q` (or `P /\ Q`) into two
///  sub-goals. When the proposition isn't a textual conjunction,
///  no-ops with `NoMatch`.
///  * `assumption` / `assumption h` — close the focused goal when
///  a hypothesis matches the proposition (textually).
///  * `apply X` / `apply X with [...]` / `exact X` — close the
///  focused goal (the kernel checker already accepted, so the
///  application is sound).
///  * `auto` / `simp` / `ring` / `nlinarith` / `lia` / `decide` /
///  `congruence` / `eauto` / `smt` — decision-procedure stand-
///  ins; close the focused goal.
///

/// Anything else returns `NoMatch`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultGoalRewriter;

impl DefaultGoalRewriter {
    pub fn new() -> Self {
        Self
    }
}

impl GoalRewriter for DefaultGoalRewriter {
    fn rewrite(&self, stack: &mut GoalStack, tactic: &str) -> GoalRewriteOutcome {
        let trimmed = tactic.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return GoalRewriteOutcome::Error {
                reason: Text::from("empty tactic"),
            };
        }

        // Tokenise on whitespace; the head determines the shape.
        let mut parts = trimmed.split_whitespace();
        let head = match parts.next() {
            Some(h) => h,
            None => {
                return GoalRewriteOutcome::Error {
                    reason: Text::from("empty tactic"),
                };
            }
        };

        match head {
            // ----- intro family -----
            "intro" => {
                let name = parts.next().unwrap_or("h");
                rewrite_intro(stack, name)
            }
            "intros" => {
                let names: Vec<&str> = parts.collect();
                if names.is_empty() {
                    rewrite_intro(stack, "h")
                } else {
                    let mut last = GoalRewriteOutcome::NoMatch;
                    for n in names {
                        last = rewrite_intro(stack, n);
                        if matches!(last, GoalRewriteOutcome::Error { .. }) {
                            return last;
                        }
                    }
                    last
                }
            }

            // ----- split / destruct on conjunction -----
            // `split` is strict — only fires on top-level
            // conjunction; non-conjunction goals return NoMatch so
            // the suggestion engine doesn't misrank. `destruct` is
            // broader — also used for case-analysis on a hypothesis;
            // falls through to Rewritten on non-conjunction goals.
            "split" => rewrite_split_conjunction(stack),
            "destruct" => match rewrite_split_conjunction(stack) {
                GoalRewriteOutcome::NoMatch => GoalRewriteOutcome::Rewritten,
                other => other,
            },

            // ----- assumption family -----
            "assumption" => rewrite_assumption(stack, parts.next()),
            "exact" => {
                // `exact X` closes (kernel verified).
                if parts.next().is_some() {
                    close_focused(stack)
                } else {
                    GoalRewriteOutcome::Error {
                        reason: Text::from("`exact` requires an argument"),
                    }
                }
            }
            "apply" => {
                if parts.next().is_some() {
                    close_focused(stack)
                } else {
                    GoalRewriteOutcome::Error {
                        reason: Text::from("`apply` requires a lemma name"),
                    }
                }
            }

            // ----- decision procedures + arithmetic deciders -----
            // (#109) — surface aligned with verum_verification::llm_tactic
            // CANONICAL_TACTICS so every tactic the kernel-checker
            // admits also has a matching state-mutation outcome.
            "auto" | "simp" | "ring" | "nlinarith" | "linarith" | "lia" | "nlia" | "lra"
            | "nra" | "decide" | "congruence" | "eauto" | "smt" | "trivial" | "reflexivity"
            | "refl" | "assumption." | "tauto" | "omega" | "field" | "blast" | "norm_num" => {
                close_focused(stack)
            }

            // ----- contradiction family — closes the focused goal -----
            "contradiction" | "by_contradiction" | "exfalso" => close_focused(stack),

            // ----- constructor / branch selection -----
            "constructor" => close_focused(stack),
            "left" => {
                // Left disjunction-introduction. We don't track which
                // branch we're committing to in the typed goal-stack;
                // best-effort: leave the goal in place (Rewritten),
                // letting the soundness gate (kernel-checker) decide
                // admissibility.
                GoalRewriteOutcome::Rewritten
            }
            "right" => GoalRewriteOutcome::Rewritten,
            "exists" => {
                // Existential-introduction. Like left/right, we
                // don't symbolically substitute the witness — best-
                // effort Rewritten.
                GoalRewriteOutcome::Rewritten
            }

            // ----- equality manipulation — leaves goal open -----
            // These tactics rewrite the focused proposition under
            // some hypothesis but don't close the goal. We mark the
            // outcome Rewritten so consumers know the typed view
            // changed; an actual symbolic substitution is future work.
            "unfold" | "fold" | "subst" | "rewrite" | "rw" | "simplify" | "compute" => {
                GoalRewriteOutcome::Rewritten
            }

            // ----- inductive / case analysis -----
            // `cases h` / `induction n` would normally produce
            // multiple sub-goals (one per constructor); the rewriter
            // doesn't reconstruct constructor arity yet, so we
            // report Rewritten and let the focused goal stand.
            "cases" | "case" | "induction" | "revert" => GoalRewriteOutcome::Rewritten,

            _ => GoalRewriteOutcome::NoMatch,
        }
    }
}

/// `intro h` / `intro` — peel a hypothesis off the focused
/// proposition's leading implication if there is one; otherwise
/// just push a placeholder hypothesis bound to `name`.
fn rewrite_intro(stack: &mut GoalStack, name: &str) -> GoalRewriteOutcome {
    let Some(focused) = stack.focused_goal().cloned() else {
        return GoalRewriteOutcome::Error {
            reason: Text::from("no focused goal"),
        };
    };
    if let Some((head, tail)) = split_top_implication(focused.proposition.as_str()) {
        stack.push_hypothesis(Hypothesis {
            name: Text::from(name),
            ty: Text::from(head),
        });
        stack.set_focused_proposition(Text::from(tail));
    } else {
        stack.push_hypothesis(Hypothesis {
            name: Text::from(name),
            ty: Text::from("?"),
        });
    }
    GoalRewriteOutcome::Rewritten
}

/// `split` — split a top-level `P ∧ Q` / `P /\ Q` into two sub-goals.
fn rewrite_split_conjunction(stack: &mut GoalStack) -> GoalRewriteOutcome {
    let Some(focused) = stack.focused_goal().cloned() else {
        return GoalRewriteOutcome::Error {
            reason: Text::from("no focused goal"),
        };
    };
    let conjuncts = split_top_conjunction(focused.proposition.as_str());
    if conjuncts.len() < 2 {
        return GoalRewriteOutcome::NoMatch;
    }
    let count = conjuncts.len();
    stack.split_focused(conjuncts.into_iter().map(Text::from).collect());
    GoalRewriteOutcome::Split { count }
}

/// `assumption [h]` — close the focused goal when a hypothesis
/// (named `h` if supplied, else any) matches the proposition by
/// textual equality.
fn rewrite_assumption(stack: &mut GoalStack, name_filter: Option<&str>) -> GoalRewriteOutcome {
    let Some(focused) = stack.focused_goal() else {
        return GoalRewriteOutcome::Error {
            reason: Text::from("no focused goal"),
        };
    };
    let prop = focused.proposition.clone();
    let matches: bool = match name_filter {
        Some(name) => focused
            .hypotheses
            .iter()
            .any(|h| h.name.as_str() == name && h.ty == prop),
        None => focused.hypotheses.iter().any(|h| h.ty == prop),
    };
    if matches {
        close_focused(stack)
    } else {
        // Kernel accepted the step but no matching hypothesis is
        // recorded in the V0 typed view — close anyway since the
        // soundness gate has already approved.
        close_focused(stack)
    }
}

fn close_focused(stack: &mut GoalStack) -> GoalRewriteOutcome {
    if stack.focused_goal().is_none() {
        return GoalRewriteOutcome::Error {
            reason: Text::from("no focused goal"),
        };
    }
    stack.close_focused();
    GoalRewriteOutcome::Closed
}

/// Best-effort split of a `H -> P` proposition into `(H, P)`.
/// Recognises `->` and `→` at the top level (parenthesis-aware).
/// Returns `None` when the textual shape is not an implication.
fn split_top_implication(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            // ASCII `->`
            if c == '-' && i + 1 < bytes.len() && bytes[i + 1] as char == '>' {
                let head = s[..i].trim();
                let tail = s[i + 2..].trim();
                if !head.is_empty() && !tail.is_empty() {
                    return Some((head, tail));
                }
            }
            // Unicode `→` (U+2192).
            if c == '\u{2192}' {
                let after = i + c.len_utf8();
                let head = s[..i].trim();
                let tail = s[after..].trim();
                if !head.is_empty() && !tail.is_empty() {
                    return Some((head, tail));
                }
            }
        }
    }
    None
}

/// Best-effort split of a top-level conjunction. Recognises `/\`
/// and `∧` (U+2227). Returns the conjuncts in source order, or a
/// single-element vector when the proposition is not a top-level
/// conjunction. Parenthesis-aware: `(A /\ B) -> C` is *not* split.
fn split_top_conjunction(s: &str) -> Vec<String> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let mut out: Vec<String> = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            // ASCII `/\`
            if c == '/' && i + 1 < bytes.len() && bytes[i + 1] as char == '\\' {
                let chunk = s[start..i].trim().to_string();
                if !chunk.is_empty() {
                    out.push(chunk);
                }
                start = i + 2;
                continue;
            }
            // Unicode `∧` (U+2227).
            if c == '\u{2227}' {
                let chunk = s[start..i].trim().to_string();
                if !chunk.is_empty() {
                    out.push(chunk);
                }
                start = i + c.len_utf8();
                continue;
            }
        }
    }
    let last = s[start..].trim().to_string();
    if !last.is_empty() {
        out.push(last);
    }
    if out.len() < 2 {
        // Wasn't a conjunction at the top level — return the
        // singleton so callers can detect the no-split case.
        return vec![s.to_string()];
    }
    out
}

// =============================================================================
// ReplSession trait
// =============================================================================

/// Single dispatch interface for a proof-REPL session.
pub trait ReplSession: std::fmt::Debug + Send {
    /// Execute one command + return its response.
    fn step(&mut self, command: ReplCommand) -> ReplResponse;

    /// Read-only view of the current proof tree. Used by the CLI's
    /// `tree` mode to dump DOT after a non-interactive batch run.
    fn proof_tree(&self) -> ProofTree;

    /// Read-only snapshot of the current state.
    fn snapshot(&self) -> ReplStateSnapshot;
}

// =============================================================================
// DefaultReplSession — reference implementation
// =============================================================================

/// V0 reference REPL session.
///

/// Wires:
///

///  * A [`PatternKernelChecker`] for step verification (the
///  soundness gate).
///  * A [`DefaultGoalRewriter`] for typed goal-stack mutation
///  (the display-side state machine; #91 hardening).
///  * A [`DefaultSuggestionEngine`] for hints.
///

/// Maintains an internal history stack for undo / redo. Each
/// history entry snapshots the *full* `GoalStack` at the time of
/// application, so undo restores the prior state byte-for-byte
/// rather than heuristically reverting one rewriter step. The
/// proof-tree records every accepted step regardless.
#[derive(Debug, Clone)]
pub struct DefaultReplSession {
    theorem_name: Text,
    initial_proposition: Text,
    /// Lemmas in scope (passed at session creation; the kernel
    /// checker uses them to validate `apply NAME` steps).
    lemmas_in_scope: Vec<LemmaSummary>,
    /// Typed open-goal stack — replaces V0's hypothesis-list +
    /// initial-proposition simulation.
    stack: GoalStack,
    /// Stack of accepted steps with their pre-application goal
    /// snapshots (for byte-exact undo).
    history: Vec<HistoryFrame>,
    /// Stack of undone frames awaiting redo.
    redo_stack: Vec<HistoryFrame>,
    /// All accepted nodes in the proof tree (history + every step
    /// that has ever been accepted, for the tree visualisation).
    proof_tree_nodes: Vec<ProofTreeNode>,
    /// Total kernel verdicts (accepted + rejected) issued in this
    /// session.
    verdict_count: usize,
}

/// One history entry — pairs the proof-tree node with the
/// pre-application goal-stack snapshot (so undo restores state
/// exactly, regardless of how many goals the rewriter introduced
/// or closed).
#[derive(Debug, Clone)]
struct HistoryFrame {
    node: ProofTreeNode,
    /// The goal stack BEFORE the tactic was applied — undo restores
    /// from this.
    pre_stack: GoalStack,
}

impl DefaultReplSession {
    pub fn new(
        theorem_name: impl Into<Text>,
        proposition: impl Into<Text>,
        lemmas: Vec<LemmaSummary>,
    ) -> Self {
        let proposition: Text = proposition.into();
        Self {
            theorem_name: theorem_name.into(),
            initial_proposition: proposition.clone(),
            lemmas_in_scope: lemmas,
            stack: GoalStack::singleton(proposition),
            history: Vec::new(),
            redo_stack: Vec::new(),
            proof_tree_nodes: Vec::new(),
            verdict_count: 0,
        }
    }

    /// Read-only access to the typed open-goal stack.
    pub fn goal_stack(&self) -> &GoalStack {
        &self.stack
    }

    fn current_proposition(&self) -> Text {
        self.stack
            .focused_goal()
            .map(|g| g.proposition.clone())
            .unwrap_or_else(|| Text::from(""))
    }

    /// Build the goal summary used by the kernel checker + LLM
    /// renderer. Pulls hypotheses + proposition from the focused
    /// goal so the LLM sees the actual context.
    pub fn build_goal_summary(&self) -> LlmGoalSummary {
        let focused_proposition = self.current_proposition();
        let mut g = LlmGoalSummary::new(self.theorem_name.clone(), focused_proposition);
        g.lemmas_in_scope = self
            .lemmas_in_scope
            .iter()
            .map(|l| (l.name.clone(), l.signature.clone()))
            .collect();
        g.hypotheses = self
            .stack
            .focused_goal()
            .map(|goal| {
                goal.hypotheses
                    .iter()
                    .map(|h| (h.name.clone(), h.ty.clone()))
                    .collect()
            })
            .unwrap_or_default();
        g.recent_tactic_history = self.history.iter().map(|f| f.node.tactic.clone()).collect();
        g
    }

    fn build_suggestion_view(&self) -> ProofStateView {
        let focused_id = self.stack.focused.unwrap_or(0);
        let goals: Vec<ProofGoalSummary> = self
            .stack
            .goals
            .iter()
            .enumerate()
            .map(|(i, g)| ProofGoalSummary {
                goal_id: g.goal_id as usize,
                proposition: g.proposition.clone(),
                hypotheses: g
                    .hypotheses
                    .iter()
                    .map(|h| HypothesisSummary {
                        name: h.name.clone(),
                        ty: h.ty.clone(),
                    })
                    .collect(),
                is_focused: i == focused_id,
            })
            .collect();
        ProofStateView {
            theorem_name: self.theorem_name.clone(),
            goals: if goals.is_empty() {
                vec![ProofGoalSummary {
                    goal_id: 0,
                    proposition: Text::from(""),
                    hypotheses: Vec::new(),
                    is_focused: true,
                }]
            } else {
                goals
            },
            available_lemmas: self.lemmas_in_scope.clone(),
            history: self.history.iter().map(|f| f.node.tactic.clone()).collect(),
        }
    }

    fn snapshot_internal(&self) -> ReplStateSnapshot {
        ReplStateSnapshot::from_stack(
            self.theorem_name.clone(),
            &self.stack,
            self.history.iter().map(|f| f.node.tactic.clone()).collect(),
            self.history.len(),
            self.redo_stack.len(),
        )
    }

    fn handle_apply(&mut self, tactic: Text) -> ReplResponse {
        let started = std::time::Instant::now();
        let goal = self.build_goal_summary();
        let checker = PatternKernelChecker::new();
        self.verdict_count += 1;
        match checker.check_step(&goal, tactic.as_str()) {
            Ok(()) => {
                // Soundness gate passed — apply the rewriter to
                // mutate the typed goal stack. Snapshot the prior
                // stack so undo can restore it byte-exact.
                let pre_stack = self.stack.clone();
                let goal_at_application = self.current_proposition();
                let rewriter = DefaultGoalRewriter::new();
                let _ = rewriter.rewrite(&mut self.stack, tactic.as_str());
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let step_index = self.proof_tree_nodes.len() + 1;
                let node = ProofTreeNode {
                    step_index,
                    tactic: tactic.clone(),
                    goal_at_application,
                    elapsed_ms,
                };
                self.history.push(HistoryFrame {
                    node: node.clone(),
                    pre_stack,
                });
                self.proof_tree_nodes.push(node);
                self.redo_stack.clear();
                ReplResponse::Accepted {
                    tactic,
                    elapsed_ms,
                    snapshot: self.snapshot_internal(),
                }
            }
            Err(reason) => ReplResponse::Rejected {
                tactic,
                reason,
                snapshot: self.snapshot_internal(),
            },
        }
    }

    fn handle_undo(&mut self) -> ReplResponse {
        match self.history.pop() {
            Some(frame) => {
                let popped = frame.node.tactic.clone();
                // Restore the goal stack to the pre-application snapshot.
                self.stack = frame.pre_stack.clone();
                self.redo_stack.push(frame);
                ReplResponse::Undone {
                    popped,
                    snapshot: self.snapshot_internal(),
                }
            }
            None => ReplResponse::NoOp {
                reason: Text::from("history is empty — nothing to undo"),
            },
        }
    }

    fn handle_redo(&mut self) -> ReplResponse {
        match self.redo_stack.pop() {
            Some(frame) => {
                let reapplied = frame.node.tactic.clone();
                // Re-apply: the rewriter mutates the stack again from
                // the (now-restored) post-undo state. Save the new
                // pre-stack snapshot so the next undo works.
                let pre_stack = self.stack.clone();
                let rewriter = DefaultGoalRewriter::new();
                let _ = rewriter.rewrite(&mut self.stack, frame.node.tactic.as_str());
                let new_frame = HistoryFrame {
                    node: frame.node,
                    pre_stack,
                };
                self.history.push(new_frame);
                ReplResponse::Redone {
                    reapplied,
                    snapshot: self.snapshot_internal(),
                }
            }
            None => ReplResponse::NoOp {
                reason: Text::from("redo stack is empty — nothing to redo"),
            },
        }
    }

    fn handle_hint(&self, max: usize) -> ReplResponse {
        let max = max.max(1);
        let view = self.build_suggestion_view();
        let engine = DefaultSuggestionEngine::new();
        let suggestions = engine.suggest(&view, max);
        let mapped = suggestions
            .into_iter()
            .map(|s: TacticSuggestion| HintSuggestion {
                snippet: s.snippet,
                rationale: s.rationale,
                score: s.score,
                category: Text::from(s.category.name()),
            })
            .collect();
        ReplResponse::Hints {
            suggestions: mapped,
        }
    }

    fn handle_visualise(&self) -> ReplResponse {
        ReplResponse::Tree {
            dot: self.proof_tree().to_dot(),
        }
    }
}

impl ReplSession for DefaultReplSession {
    fn step(&mut self, command: ReplCommand) -> ReplResponse {
        match command {
            ReplCommand::Apply { tactic } => self.handle_apply(tactic),
            ReplCommand::Undo => self.handle_undo(),
            ReplCommand::Redo => self.handle_redo(),
            ReplCommand::ShowGoals | ReplCommand::ShowContext | ReplCommand::Status => {
                ReplResponse::Status {
                    snapshot: self.snapshot_internal(),
                }
            }
            ReplCommand::Visualise => self.handle_visualise(),
            ReplCommand::Hint { max } => self.handle_hint(max),
        }
    }

    fn proof_tree(&self) -> ProofTree {
        ProofTree {
            root_goal: self.initial_proposition.clone(),
            nodes: self.history.iter().map(|f| f.node.clone()).collect(),
        }
    }

    fn snapshot(&self) -> ReplStateSnapshot {
        self.snapshot_internal()
    }
}

// =============================================================================
// Batch driver — convenience for non-interactive runs
// =============================================================================

/// Run a sequence of commands against a session and return every
/// response in order. Used by the CLI's batch mode + tests.
pub fn run_batch<S: ReplSession>(session: &mut S, commands: Vec<ReplCommand>) -> Vec<ReplResponse> {
    commands.into_iter().map(|c| session.step(c)).collect()
}

/// Aggregate response counts across a transcript. Used for batch-
/// run summary output.
pub fn summarise(responses: &[ReplResponse]) -> BTreeMap<&'static str, usize> {
    let mut by_kind: BTreeMap<&'static str, usize> = BTreeMap::new();
    for r in responses {
        *by_kind.entry(r.name()).or_insert(0) += 1;
    }
    by_kind
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lemmas_with(names: &[(&str, &str)]) -> Vec<LemmaSummary> {
        names
            .iter()
            .map(|(n, s)| LemmaSummary {
                name: Text::from(*n),
                signature: Text::from(*s),
                lineage: Text::from("test"),
            })
            .collect()
    }

    fn fresh_session() -> DefaultReplSession {
        DefaultReplSession::new("thm", "P(x)", lemmas_with(&[("foo_lemma", "P(x)")]))
    }

    // ----- ReplCommand / ReplResponse round-trip -----

    #[test]
    fn repl_command_serde_round_trip() {
        let cmd = ReplCommand::Apply {
            tactic: Text::from("intro"),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: ReplCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn repl_response_name_stable() {
        let s = ReplStateSnapshot {
            theorem_name: Text::from("t"),
            goals: vec![],
            focused_goal_id: None,
            focused_proposition: Text::from("P"),
            open_goals: vec![],
            applied_steps: vec![],
            history_depth: 0,
            redo_depth: 0,
        };
        assert_eq!(
            ReplResponse::Status {
                snapshot: s.clone()
            }
            .name(),
            "Status"
        );
        assert_eq!(
            ReplResponse::NoOp {
                reason: Text::from("x")
            }
            .name(),
            "NoOp"
        );
    }

    // ----- DefaultReplSession.handle_apply -----

    #[test]
    fn apply_canonical_tactic_accepted() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        match r {
            ReplResponse::Accepted { snapshot, .. } => {
                assert_eq!(snapshot.history_depth, 1);
                assert_eq!(snapshot.applied_steps[0].as_str(), "intro");
            }
            other => panic!("expected Accepted, got {:?}", other),
        }
    }

    #[test]
    fn apply_with_in_scope_lemma_accepted() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Apply {
            tactic: Text::from("apply foo_lemma"),
        });
        assert!(matches!(r, ReplResponse::Accepted { .. }));
    }

    #[test]
    fn apply_with_unknown_lemma_rejected() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Apply {
            tactic: Text::from("apply unknown_lemma"),
        });
        match r {
            ReplResponse::Rejected { reason, .. } => {
                assert!(reason.as_str().contains("not in scope"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn rejected_step_does_not_mutate_history() {
        let mut s = fresh_session();
        let _ = s.step(ReplCommand::Apply {
            tactic: Text::from("garbage_step"),
        });
        let snap = s.snapshot();
        assert_eq!(snap.history_depth, 0);
    }

    #[test]
    fn intro_with_named_hypothesis_brings_it_into_scope() {
        let mut s = fresh_session();
        let _ = s.step(ReplCommand::Apply {
            tactic: Text::from("intro h"),
        });
        // Now `apply h` should fail (we don't auto-promote
        // hypotheses to lemmas — but we do record the hypothesis
        // name). At minimum the goal summary records `h`.
        let goal = s.build_goal_summary();
        assert!(goal.hypotheses.iter().any(|(n, _)| n.as_str() == "h"));
    }

    // ----- Undo / Redo -----

    #[test]
    fn undo_pops_last_step() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        s.step(ReplCommand::Apply {
            tactic: Text::from("auto"),
        });
        let r = s.step(ReplCommand::Undo);
        match r {
            ReplResponse::Undone { popped, snapshot } => {
                assert_eq!(popped.as_str(), "auto");
                assert_eq!(snapshot.history_depth, 1);
                assert_eq!(snapshot.redo_depth, 1);
            }
            other => panic!("expected Undone, got {:?}", other),
        }
    }

    #[test]
    fn undo_on_empty_history_is_noop() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Undo);
        assert!(matches!(r, ReplResponse::NoOp { .. }));
    }

    #[test]
    fn redo_replays_last_undone_step() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        s.step(ReplCommand::Undo);
        let r = s.step(ReplCommand::Redo);
        match r {
            ReplResponse::Redone {
                reapplied,
                snapshot,
            } => {
                assert_eq!(reapplied.as_str(), "intro");
                assert_eq!(snapshot.history_depth, 1);
                assert_eq!(snapshot.redo_depth, 0);
            }
            other => panic!("expected Redone, got {:?}", other),
        }
    }

    #[test]
    fn redo_on_empty_redo_stack_is_noop() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Redo);
        assert!(matches!(r, ReplResponse::NoOp { .. }));
    }

    #[test]
    fn new_apply_clears_redo_stack() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        s.step(ReplCommand::Undo);
        // Now redo_depth = 1. Apply a fresh step.
        let r = s.step(ReplCommand::Apply {
            tactic: Text::from("auto"),
        });
        match r {
            ReplResponse::Accepted { snapshot, .. } => {
                assert_eq!(snapshot.redo_depth, 0, "fresh apply must clear redo stack");
            }
            _ => panic!("unexpected"),
        }
    }

    // ----- Status / Hints / Visualise -----

    #[test]
    fn status_snapshot_carries_history_depth() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        s.step(ReplCommand::Apply {
            tactic: Text::from("auto"),
        });
        let r = s.step(ReplCommand::Status);
        match r {
            ReplResponse::Status { snapshot } => assert_eq!(snapshot.history_depth, 2),
            _ => panic!(),
        }
    }

    #[test]
    fn hint_returns_at_least_one_suggestion() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Hint { max: 5 });
        match r {
            ReplResponse::Hints { suggestions } => assert!(!suggestions.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn hint_max_zero_clamped_to_one() {
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Hint { max: 0 });
        match r {
            ReplResponse::Hints { suggestions } => {
                assert!(!suggestions.is_empty(), "max=0 must clamp to 1");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn visualise_emits_dot() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        let r = s.step(ReplCommand::Visualise);
        match r {
            ReplResponse::Tree { dot } => {
                let s = dot.as_str();
                assert!(s.starts_with("digraph proof_tree"));
                assert!(s.contains("step_1"));
                assert!(s.contains("intro"));
                assert!(s.ends_with('}'));
            }
            _ => panic!(),
        }
    }

    // ----- ProofTree -----

    #[test]
    fn proof_tree_contains_every_accepted_step() {
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        s.step(ReplCommand::Apply {
            tactic: Text::from("auto"),
        });
        let tree = s.proof_tree();
        assert_eq!(tree.nodes.len(), 2);
    }

    #[test]
    fn proof_tree_root_goal_is_initial_proposition() {
        let s = fresh_session();
        let tree = s.proof_tree();
        assert_eq!(tree.root_goal.as_str(), "P(x)");
    }

    // ----- Batch driver -----

    #[test]
    fn run_batch_returns_one_response_per_command() {
        let mut s = fresh_session();
        let cmds = vec![
            ReplCommand::Apply {
                tactic: Text::from("intro"),
            },
            ReplCommand::Apply {
                tactic: Text::from("auto"),
            },
            ReplCommand::Status,
        ];
        let responses = run_batch(&mut s, cmds);
        assert_eq!(responses.len(), 3);
    }

    #[test]
    fn summarise_groups_responses_by_kind() {
        let mut s = fresh_session();
        let cmds = vec![
            ReplCommand::Apply {
                tactic: Text::from("intro"),
            },
            ReplCommand::Apply {
                tactic: Text::from("garbage"),
            },
            ReplCommand::Status,
        ];
        let responses = run_batch(&mut s, cmds);
        let summary = summarise(&responses);
        assert_eq!(summary.get("Accepted").copied(), Some(1));
        assert_eq!(summary.get("Rejected").copied(), Some(1));
        assert_eq!(summary.get("Status").copied(), Some(1));
    }

    // ----- Acceptance criteria pin -----

    #[test]
    fn task_75_stepwise_feedback_with_kernel_verdict_and_elapsed_time() {
        // Each accepted tactic returns elapsed_ms + new state.
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        if let ReplResponse::Accepted {
            elapsed_ms: _,
            snapshot,
            ..
        } = r
        {
            assert_eq!(snapshot.applied_steps.len(), 1);
        } else {
            panic!("expected Accepted");
        }
    }

    #[test]
    fn task_75_visualise_emits_proof_tree_dot() {
        // §5: emit current proof tree as DOT.
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        let r = s.step(ReplCommand::Visualise);
        match r {
            ReplResponse::Tree { dot } => assert!(dot.as_str().contains("digraph")),
            _ => panic!(),
        }
    }

    #[test]
    fn task_75_undo_redo_navigation() {
        // §4: :undo / :redo for proof-state navigation.
        let mut s = fresh_session();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro"),
        });
        let undo = s.step(ReplCommand::Undo);
        let redo = s.step(ReplCommand::Redo);
        assert!(matches!(undo, ReplResponse::Undone { .. }));
        assert!(matches!(redo, ReplResponse::Redone { .. }));
    }

    // ----- GoalStack invariants -----

    #[test]
    fn goal_stack_singleton_starts_with_one_focused_goal() {
        let s = GoalStack::singleton("P");
        assert_eq!(s.goals.len(), 1);
        assert_eq!(s.focused, Some(0));
        assert_eq!(s.focused_goal().unwrap().proposition.as_str(), "P");
        assert!(s.focused_goal().unwrap().hypotheses.is_empty());
    }

    #[test]
    fn goal_stack_close_focused_drops_goal_and_refocuses() {
        let mut s = GoalStack::singleton("P");
        s.split_focused(vec![Text::from("A"), Text::from("B")]);
        assert_eq!(s.goals.len(), 2);
        assert_eq!(s.focused, Some(0));
        s.close_focused();
        assert_eq!(s.goals.len(), 1);
        assert_eq!(s.focused_goal().unwrap().proposition.as_str(), "B");
        s.close_focused();
        assert!(s.is_complete());
        assert_eq!(s.focused, None);
    }

    #[test]
    fn goal_stack_split_inherits_hypotheses() {
        let mut s = GoalStack::singleton("A and B");
        s.push_hypothesis(Hypothesis {
            name: Text::from("h"),
            ty: Text::from("X"),
        });
        s.split_focused(vec![Text::from("A"), Text::from("B")]);
        for g in &s.goals {
            assert_eq!(g.hypotheses.len(), 1);
            assert_eq!(g.hypotheses[0].name.as_str(), "h");
        }
    }

    #[test]
    fn goal_stack_split_assigns_unique_ids() {
        let mut s = GoalStack::singleton("P");
        s.split_focused(vec![Text::from("A"), Text::from("B"), Text::from("C")]);
        let ids: Vec<u64> = s.goals.iter().map(|g| g.goal_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "goal ids must be unique");
    }

    // ----- DefaultGoalRewriter -----

    #[test]
    fn rewriter_intro_on_implication_pulls_off_head() {
        let mut s = GoalStack::singleton("Bool -> Bool");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "intro h");
        assert!(matches!(outcome, GoalRewriteOutcome::Rewritten));
        let g = s.focused_goal().unwrap();
        assert_eq!(g.proposition.as_str(), "Bool");
        assert_eq!(g.hypotheses.len(), 1);
        assert_eq!(g.hypotheses[0].name.as_str(), "h");
        assert_eq!(g.hypotheses[0].ty.as_str(), "Bool");
    }

    #[test]
    fn rewriter_intro_unicode_implication() {
        let mut s = GoalStack::singleton("Nat → Nat");
        let r = DefaultGoalRewriter::new();
        let _ = r.rewrite(&mut s, "intro n");
        let g = s.focused_goal().unwrap();
        assert_eq!(g.proposition.as_str(), "Nat");
        assert_eq!(g.hypotheses[0].ty.as_str(), "Nat");
    }

    #[test]
    fn rewriter_intro_on_non_implication_appends_placeholder() {
        let mut s = GoalStack::singleton("P(x)");
        let r = DefaultGoalRewriter::new();
        let _ = r.rewrite(&mut s, "intro h");
        let g = s.focused_goal().unwrap();
        assert_eq!(g.proposition.as_str(), "P(x)");
        assert_eq!(g.hypotheses[0].name.as_str(), "h");
        assert_eq!(g.hypotheses[0].ty.as_str(), "?");
    }

    #[test]
    fn rewriter_intros_pulls_two_hypotheses() {
        let mut s = GoalStack::singleton("A -> B -> C");
        let r = DefaultGoalRewriter::new();
        let _ = r.rewrite(&mut s, "intros a b");
        let g = s.focused_goal().unwrap();
        assert_eq!(g.proposition.as_str(), "C");
        assert_eq!(g.hypotheses.len(), 2);
        assert_eq!(g.hypotheses[0].ty.as_str(), "A");
        assert_eq!(g.hypotheses[1].ty.as_str(), "B");
    }

    #[test]
    fn rewriter_split_forks_top_level_conjunction_ascii() {
        let mut s = GoalStack::singleton("P /\\ Q");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "split");
        match outcome {
            GoalRewriteOutcome::Split { count } => assert_eq!(count, 2),
            other => panic!("expected Split, got {:?}", other),
        }
        assert_eq!(s.goals.len(), 2);
        assert_eq!(s.goals[0].proposition.as_str(), "P");
        assert_eq!(s.goals[1].proposition.as_str(), "Q");
    }

    #[test]
    fn rewriter_split_forks_top_level_conjunction_unicode() {
        let mut s = GoalStack::singleton("P ∧ Q ∧ R");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "split");
        match outcome {
            GoalRewriteOutcome::Split { count } => assert_eq!(count, 3),
            other => panic!("expected Split, got {:?}", other),
        }
        assert_eq!(s.goals.len(), 3);
    }

    #[test]
    fn rewriter_split_paren_aware_does_not_descend_into_subterm() {
        let mut s = GoalStack::singleton("(A /\\ B) -> C");
        let r = DefaultGoalRewriter::new();
        // Top-level shape is implication, not conjunction.
        let outcome = r.rewrite(&mut s, "split");
        assert!(matches!(outcome, GoalRewriteOutcome::NoMatch));
    }

    #[test]
    fn rewriter_apply_closes_focused_goal() {
        let mut s = GoalStack::singleton("P(x)");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "apply foo_lemma");
        assert!(matches!(outcome, GoalRewriteOutcome::Closed));
        assert!(s.is_complete());
    }

    #[test]
    fn rewriter_decision_procedure_closes() {
        for tac in ["auto", "simp", "ring", "nlinarith", "lia", "decide"] {
            let mut s = GoalStack::singleton("P");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Closed),
                "tactic `{}` did not close",
                tac
            );
            assert!(s.is_complete());
        }
    }

    #[test]
    fn rewriter_unknown_tactic_returns_no_match() {
        let mut s = GoalStack::singleton("P");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "garbage_step");
        assert!(matches!(outcome, GoalRewriteOutcome::NoMatch));
        assert_eq!(s.goals.len(), 1, "NoMatch must not mutate stack");
    }

    // -- Surface alignment with CANONICAL_TACTICS (#109) ----------------

    #[test]
    fn rewriter_extended_decision_procedures_close() {
        // Aligned with verum_verification::llm_tactic::CANONICAL_TACTICS.
        for tac in [
            "linarith",
            "nlia",
            "lra",
            "nra",
            "field",
            "blast",
            "norm_num",
            "tauto",
            "reflexivity",
        ] {
            let mut s = GoalStack::singleton("P");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Closed),
                "tactic `{}` should close the focused goal",
                tac
            );
            assert!(s.is_complete());
        }
    }

    #[test]
    fn rewriter_contradiction_family_closes() {
        for tac in ["contradiction", "by_contradiction", "exfalso"] {
            let mut s = GoalStack::singleton("P");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Closed),
                "tactic `{}` should close",
                tac
            );
        }
    }

    #[test]
    fn rewriter_constructor_closes() {
        let mut s = GoalStack::singleton("P");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "constructor");
        assert!(matches!(outcome, GoalRewriteOutcome::Closed));
    }

    #[test]
    fn rewriter_left_right_exists_yield_rewritten() {
        for tac in ["left", "right", "exists witness"] {
            let mut s = GoalStack::singleton("P \\/ Q");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Rewritten),
                "tactic `{}` should produce Rewritten",
                tac
            );
            assert_eq!(s.goals.len(), 1, "{} must not close the goal", tac);
        }
    }

    #[test]
    fn rewriter_equality_manipulation_yields_rewritten() {
        for tac in [
            "rewrite h",
            "rw eq",
            "subst x",
            "unfold foo",
            "fold bar",
            "simplify",
            "compute",
        ] {
            let mut s = GoalStack::singleton("x = y");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Rewritten),
                "tactic `{}` should produce Rewritten",
                tac
            );
        }
    }

    #[test]
    fn rewriter_inductive_family_yields_rewritten() {
        for tac in ["cases h", "case Some", "induction n", "revert h"] {
            let mut s = GoalStack::singleton("P n");
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                matches!(outcome, GoalRewriteOutcome::Rewritten),
                "tactic `{}` should produce Rewritten",
                tac
            );
        }
    }

    #[test]
    fn task_109_rewriter_recognises_every_canonical_tactic_head() {
        // Pin: every head in `verum_verification::llm_tactic::CANONICAL_TACTICS`
        // is recognised by the GoalRewriter (returns something other
        // than NoMatch on a goal whose textual shape lets the
        // tactic fire). Exceptions:
        //  * `skip` / `fail` — combinator sentinels handled at a
        //  higher layer (no single-goal semantic).
        //  * `split` — strict on top-level conjunctions; tested
        //  with a conjunction goal.
        let exempt: std::collections::BTreeSet<&str> = ["skip", "fail"].iter().copied().collect();
        for tac in crate::llm_tactic::canonical_tactics() {
            if exempt.contains(tac) {
                continue;
            }
            // Use a conjunction goal for split (it's strict);
            // generic predicate goal for everything else.
            let goal = if *tac == "split" { "P /\\ Q" } else { "P x" };
            let mut s = GoalStack::singleton(goal);
            let r = DefaultGoalRewriter::new();
            let outcome = r.rewrite(&mut s, tac);
            assert!(
                !matches!(outcome, GoalRewriteOutcome::NoMatch),
                "canonical tactic `{}` returned NoMatch — surface drift",
                tac
            );
        }
    }

    #[test]
    fn rewriter_empty_tactic_errors() {
        let mut s = GoalStack::singleton("P");
        let r = DefaultGoalRewriter::new();
        let outcome = r.rewrite(&mut s, "   ");
        assert!(matches!(outcome, GoalRewriteOutcome::Error { .. }));
    }

    // ----- Session goal-stack semantics -----

    #[test]
    fn session_intro_records_typed_hypothesis_in_focused_goal() {
        let mut s = DefaultReplSession::new("thm", "P -> Q", lemmas_with(&[("foo_lemma", "P")]));
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro h"),
        });
        let goal = s.goal_stack().focused_goal().unwrap();
        assert_eq!(goal.proposition.as_str(), "Q");
        assert_eq!(goal.hypotheses.len(), 1);
        assert_eq!(goal.hypotheses[0].name.as_str(), "h");
        assert_eq!(goal.hypotheses[0].ty.as_str(), "P");
    }

    #[test]
    fn session_undo_restores_pre_application_stack_byte_exact() {
        let mut s = DefaultReplSession::new("thm", "P -> Q", lemmas_with(&[("foo_lemma", "Q")]));
        let before = s.goal_stack().clone();
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro h"),
        });
        // Stack now mutated: hypothesis pushed, prop is Q.
        assert_ne!(s.goal_stack(), &before);
        s.step(ReplCommand::Undo);
        // Stack restored.
        assert_eq!(s.goal_stack(), &before);
    }

    #[test]
    fn session_apply_then_undo_then_redo_re_runs_rewriter() {
        let mut s = DefaultReplSession::new("thm", "A -> B", lemmas_with(&[("foo", "B")]));
        s.step(ReplCommand::Apply {
            tactic: Text::from("intro h"),
        });
        let after_apply = s.goal_stack().clone();
        s.step(ReplCommand::Undo);
        s.step(ReplCommand::Redo);
        // Stack equivalent to post-apply (same proposition, same hypothesis).
        let post_redo = s.goal_stack();
        assert_eq!(post_redo.goals.len(), after_apply.goals.len());
        let g = post_redo.focused_goal().unwrap();
        assert_eq!(g.proposition.as_str(), "B");
        assert_eq!(g.hypotheses[0].name.as_str(), "h");
    }

    #[test]
    fn session_snapshot_carries_typed_goals_field() {
        let s = DefaultReplSession::new("thm", "P", Vec::new());
        let snap = s.snapshot();
        assert_eq!(snap.goals.len(), 1);
        assert_eq!(snap.goals[0].proposition.as_str(), "P");
        assert_eq!(snap.focused_goal_id, Some(0));
        // Backwards-compat projections.
        assert_eq!(snap.focused_proposition.as_str(), "P");
        assert_eq!(snap.open_goals.len(), 1);
    }

    #[test]
    fn session_snapshot_after_close_reports_proof_complete() {
        let mut s = DefaultReplSession::new("thm", "P", lemmas_with(&[("foo", "P")]));
        s.step(ReplCommand::Apply {
            tactic: Text::from("apply foo"),
        });
        let snap = s.snapshot();
        assert!(snap.goals.is_empty());
        assert_eq!(snap.focused_goal_id, None);
        assert_eq!(snap.focused_proposition.as_str(), "");
    }

    #[test]
    fn task_91_typed_goal_stack_replaces_string_state() {
        // Pin the #91 hardening contract: the snapshot is produced
        // from a `GoalStack` of typed goals, not from a single
        // rendered string. The stringly-typed `open_goals` /
        // `focused_proposition` fields are derived projections.
        let s = DefaultReplSession::new("thm", "A -> B", Vec::new());
        let snap = s.snapshot();
        assert_eq!(snap.goals.len(), 1);
        assert_eq!(snap.goals[0].proposition.as_str(), "A -> B");
        // Derived projections.
        let derived: Vec<&str> = snap.goals.iter().map(|g| g.proposition.as_str()).collect();
        let projected: Vec<&str> = snap.open_goals.iter().map(|t| t.as_str()).collect();
        assert_eq!(derived, projected);
    }

    #[test]
    fn task_75_hint_proposes_plausible_next_steps() {
        // §7: :hint proposes 3-5 plausible next steps based on goal shape.
        let mut s = fresh_session();
        let r = s.step(ReplCommand::Hint { max: 5 });
        match r {
            ReplResponse::Hints { suggestions } => {
                assert!(!suggestions.is_empty());
                for h in &suggestions {
                    assert!(!h.snippet.as_str().is_empty());
                    assert!(!h.rationale.as_str().is_empty());
                    assert!(h.score >= 0.0 && h.score <= 1.0);
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn meta_pin_repl_command_kind_round_trip_and_mutation_partition() {
        assert_eq!(ReplCommandKind::ALL.len(), 8);
        let mut seen = Vec::new();
        for k in ReplCommandKind::ALL {
            let s = k.name();
            assert_eq!(
                ReplCommandKind::from_str(s),
                Some(*k),
                "ReplCommandKind::{:?}: '{}' round-trip",
                k,
                s
            );
            assert!(!seen.contains(&s), "duplicate name '{}'", s);
            seen.push(s);
        }
        // Mutation partition: Apply / Undo / Redo = 3.
        let mutation_count = ReplCommandKind::ALL
            .iter()
            .filter(|k| k.is_mutation())
            .count();
        assert_eq!(mutation_count, 3);
        assert!(ReplCommandKind::Apply.is_mutation());
        assert!(ReplCommandKind::Undo.is_mutation());
        assert!(ReplCommandKind::Redo.is_mutation());
        // Read-only: 5 (ShowGoals / ShowContext / Visualise / Hint /
        // Status).
        assert!(!ReplCommandKind::ShowGoals.is_mutation());
        assert!(!ReplCommandKind::Status.is_mutation());

        // Cross-pin: ReplCommand::kind() agrees with the kind tag
        // for every payload-bearing variant.
        assert_eq!(
            ReplCommand::Apply { tactic: Text::from("intro") }.kind(),
            ReplCommandKind::Apply
        );
        assert_eq!(
            ReplCommand::Hint { max: 5 }.kind(),
            ReplCommandKind::Hint
        );
        assert_eq!(ReplCommand::Undo.kind(), ReplCommandKind::Undo);
    }

    #[test]
    fn meta_pin_repl_response_kind_round_trip_and_partitions() {
        assert_eq!(ReplResponseKind::ALL.len(), 9);
        for k in ReplResponseKind::ALL {
            let s = k.name();
            assert_eq!(ReplResponseKind::from_str(s), Some(*k));
        }
        // is_state_mutation partition: Accepted / Undone / Redone = 3.
        let mutation_count = ReplResponseKind::ALL
            .iter()
            .filter(|k| k.is_state_mutation())
            .count();
        assert_eq!(mutation_count, 3);
        assert!(ReplResponseKind::Accepted.is_state_mutation());
        assert!(ReplResponseKind::Undone.is_state_mutation());
        assert!(ReplResponseKind::Redone.is_state_mutation());
        assert!(!ReplResponseKind::Rejected.is_state_mutation());
        // is_error partition: Error only (1). Rejected is NOT an
        // error — the kernel rejected the tactic but the REPL is
        // healthy.
        let error_count = ReplResponseKind::ALL
            .iter()
            .filter(|k| k.is_error())
            .count();
        assert_eq!(error_count, 1);
        assert!(ReplResponseKind::Error.is_error());
        assert!(!ReplResponseKind::Rejected.is_error());
        // Cross-pin: state mutation and error are disjoint.
        for k in ReplResponseKind::ALL {
            assert!(
                !(k.is_state_mutation() && k.is_error()),
                "ReplResponseKind::{:?}: mutation ⊥ error must be disjoint",
                k
            );
        }
    }

    #[test]
    fn meta_pin_goal_rewrite_outcome_kind_classification() {
        assert_eq!(GoalRewriteOutcomeKind::ALL.len(), 5);
        for k in GoalRewriteOutcomeKind::ALL {
            let s = k.name();
            assert_eq!(GoalRewriteOutcomeKind::from_str(s), Some(*k));
        }
        // Wire form (snake_case for telemetry).
        assert_eq!(GoalRewriteOutcomeKind::Rewritten.name(), "rewritten");
        assert_eq!(GoalRewriteOutcomeKind::Split.name(), "split");
        assert_eq!(GoalRewriteOutcomeKind::Closed.name(), "closed");
        assert_eq!(GoalRewriteOutcomeKind::NoMatch.name(), "no_match");
        assert_eq!(GoalRewriteOutcomeKind::Error.name(), "error");
        // mutates_display_state: Rewritten / Split / Closed = 3;
        // NoMatch / Error = 2 don't.
        let mutate_count = GoalRewriteOutcomeKind::ALL
            .iter()
            .filter(|k| k.mutates_display_state())
            .count();
        assert_eq!(mutate_count, 3);
        // closes_goal is unique to Closed.
        let close_count = GoalRewriteOutcomeKind::ALL
            .iter()
            .filter(|k| k.closes_goal())
            .count();
        assert_eq!(close_count, 1);
        assert!(GoalRewriteOutcomeKind::Closed.closes_goal());
        // is_error is unique to Error.
        let err_count = GoalRewriteOutcomeKind::ALL
            .iter()
            .filter(|k| k.is_error())
            .count();
        assert_eq!(err_count, 1);
        assert!(GoalRewriteOutcomeKind::Error.is_error());
        // Cross-cutting: closes_goal ⇒ mutates_display_state.
        for k in GoalRewriteOutcomeKind::ALL {
            if k.closes_goal() {
                assert!(
                    k.mutates_display_state(),
                    "GoalRewriteOutcomeKind::{:?}: closes_goal ⇒ mutates_display_state",
                    k
                );
            }
        }
        // Cross-cutting: NoMatch is the only kind that's neither
        // mutation nor error — it's a "rewriter doesn't recognize
        // this shape" signal.
        for k in GoalRewriteOutcomeKind::ALL {
            let neither = !k.mutates_display_state() && !k.is_error();
            assert_eq!(
                neither,
                *k == GoalRewriteOutcomeKind::NoMatch,
                "GoalRewriteOutcomeKind::{:?}: NoMatch is the lone non-mutation non-error",
                k
            );
        }
    }
}
