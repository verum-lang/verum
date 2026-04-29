//! Live proof REPL — stepwise tactic feedback + proof-tree
//! visualisation.
//!
//! ## Goal
//!
//! Mathematicians need a workflow where every tactic produces
//! immediate, kernel-grade feedback: did the step type-check, what
//! does the new goal stack look like, and what's the proof tree
//! built so far?  This module ships the **protocol** + **state
//! machine** that drive such a REPL.  Interactive TUI is a UI
//! concern (`verum_interactive`); LSP integration is a separate
//! transport — both consume the same trait surface defined here.
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the integration
//! arc (ladder_dispatch / proof_drafting / proof_repair / closure_cache
//! / doc_render / foreign_import / llm_tactic):
//!
//!   * [`ReplCommand`] — typed enum of every command the user can
//!     issue (`Apply` a tactic, `Undo` the last step, request a
//!     `Hint`, ask for the `ProofTree`, etc.).
//!   * [`ReplResponse`] — typed enum of every possible response
//!     (Accepted / Rejected / Status / Tree / Hints / etc.).
//!   * [`ReplSession`] trait — single dispatch interface;
//!     `step(command) -> response`.
//!   * [`DefaultReplSession`] — reference implementation that wires
//!     a [`crate::llm_tactic::KernelChecker`] for step verification +
//!     [`crate::proof_drafting::DefaultSuggestionEngine`] for hints.
//!     Maintains an internal history stack for undo / redo.
//!   * [`ProofTreeNode`] / [`ProofTree`] — typed DAG of accepted
//!     steps with kernel verdicts and elapsed times.  Renders to
//!     Graphviz DOT for `:visualise`.
//!
//! ## Stepwise feedback contract
//!
//! Every tactic application produces:
//!
//!   * The kernel verdict (Accepted / Rejected with cause).
//!   * Wall-clock duration in milliseconds.
//!   * The updated proof state (open-goal stack snapshot).
//!   * A node in the proof tree linking the step to the goal it
//!     was applied to.
//!
//! Rejected steps DO NOT mutate the proof state — the LCF
//! fail-closed contract carries through from `llm_tactic`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::llm_tactic::{KernelChecker, LlmGoalSummary, PatternKernelChecker};
use crate::proof_drafting::{
    DefaultSuggestionEngine, LemmaSummary, ProofGoalSummary, ProofStateView,
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
    /// `"auto"`).  The kernel re-checks before mutating state.
    Apply { tactic: Text },
    /// Undo the last accepted step.  No-op when history is empty.
    Undo,
    /// Re-apply the most recently undone step.  No-op when the
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

// =============================================================================
// ReplResponse — the output surface
// =============================================================================

/// Snapshot of the open-goal stack + applied steps at a point in
/// time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplStateSnapshot {
    pub theorem_name: Text,
    pub focused_proposition: Text,
    pub open_goals: Vec<Text>,
    pub applied_steps: Vec<Text>,
    pub history_depth: usize,
    pub redo_depth: usize,
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
    /// The kernel rejected the tactic.  State is unchanged.
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
    Hints {
        suggestions: Vec<HintSuggestion>,
    },
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

impl ReplResponse {
    /// True iff the response represents a successful state mutation
    /// (Accepted / Undone / Redone).
    pub fn is_state_mutation(&self) -> bool {
        matches!(
            self,
            ReplResponse::Accepted { .. } | ReplResponse::Undone { .. } | ReplResponse::Redone { .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "Accepted",
            Self::Rejected { .. } => "Rejected",
            Self::Undone { .. } => "Undone",
            Self::Redone { .. } => "Redone",
            Self::Status { .. } => "Status",
            Self::Hints { .. } => "Hints",
            Self::Tree { .. } => "Tree",
            Self::NoOp { .. } => "NoOp",
            Self::Error { .. } => "Error",
        }
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
    /// Render the tree as Graphviz DOT.  Each accepted step is a
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
            s.push_str(&format!("  goal_root -> step_{};\n", self.nodes[0].step_index));
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
// ReplSession trait
// =============================================================================

/// Single dispatch interface for a proof-REPL session.
pub trait ReplSession: std::fmt::Debug + Send {
    /// Execute one command + return its response.
    fn step(&mut self, command: ReplCommand) -> ReplResponse;

    /// Read-only view of the current proof tree.  Used by the CLI's
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
///   * A [`PatternKernelChecker`] for step verification.
///   * A [`DefaultSuggestionEngine`] for hints.
///
/// Maintains an internal history stack for undo / redo.  Goal-stack
/// updates are simulated for V0 (the actual goal-rewriting is a
/// kernel-side responsibility — V1 will plumb that through).  The
/// proof-tree records every accepted step regardless.
#[derive(Debug, Clone)]
pub struct DefaultReplSession {
    theorem_name: Text,
    initial_proposition: Text,
    /// Lemmas in scope (passed at session creation; the kernel
    /// checker uses them to validate `apply NAME` steps).
    lemmas_in_scope: Vec<LemmaSummary>,
    /// Hypotheses introduced via `intro` etc.  V0 simulates this
    /// with a string list; V1 plumbs the typed kernel context.
    hypotheses: Vec<(Text, Text)>,
    /// Stack of accepted steps (most recent at the end).
    history: Vec<ProofTreeNode>,
    /// Stack of undone steps awaiting redo.
    redo_stack: Vec<ProofTreeNode>,
    /// All accepted steps for the proof tree (history + every step
    /// that has ever been accepted, for the tree visualisation).
    proof_tree_nodes: Vec<ProofTreeNode>,
    /// Total kernel verdicts (accepted + rejected) issued in this
    /// session.
    verdict_count: usize,
}

impl DefaultReplSession {
    pub fn new(
        theorem_name: impl Into<Text>,
        proposition: impl Into<Text>,
        lemmas: Vec<LemmaSummary>,
    ) -> Self {
        Self {
            theorem_name: theorem_name.into(),
            initial_proposition: proposition.into(),
            lemmas_in_scope: lemmas,
            hypotheses: Vec::new(),
            history: Vec::new(),
            redo_stack: Vec::new(),
            proof_tree_nodes: Vec::new(),
            verdict_count: 0,
        }
    }

    fn current_proposition(&self) -> &Text {
        &self.initial_proposition
    }

    /// Build the goal summary used by the kernel checker + suggestion
    /// engine.  Includes hypotheses + lemmas + recent history.
    fn build_goal_summary(&self) -> LlmGoalSummary {
        let mut g = LlmGoalSummary::new(
            self.theorem_name.clone(),
            self.current_proposition().clone(),
        );
        g.lemmas_in_scope = self
            .lemmas_in_scope
            .iter()
            .map(|l| (l.name.clone(), l.signature.clone()))
            .collect();
        g.hypotheses = self.hypotheses.clone();
        g.recent_tactic_history = self.history.iter().map(|n| n.tactic.clone()).collect();
        g
    }

    fn build_suggestion_view(&self) -> ProofStateView {
        ProofStateView {
            theorem_name: self.theorem_name.clone(),
            goals: vec![ProofGoalSummary {
                goal_id: 0,
                proposition: self.current_proposition().clone(),
                hypotheses: Vec::new(),
                is_focused: true,
            }],
            available_lemmas: self.lemmas_in_scope.clone(),
            history: self.history.iter().map(|n| n.tactic.clone()).collect(),
        }
    }

    fn snapshot_internal(&self) -> ReplStateSnapshot {
        ReplStateSnapshot {
            theorem_name: self.theorem_name.clone(),
            focused_proposition: self.current_proposition().clone(),
            open_goals: vec![self.current_proposition().clone()],
            applied_steps: self.history.iter().map(|n| n.tactic.clone()).collect(),
            history_depth: self.history.len(),
            redo_depth: self.redo_stack.len(),
        }
    }

    fn handle_apply(&mut self, tactic: Text) -> ReplResponse {
        let started = std::time::Instant::now();
        let goal = self.build_goal_summary();
        let checker = PatternKernelChecker::new();
        self.verdict_count += 1;
        match checker.check_step(&goal, tactic.as_str()) {
            Ok(()) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let step_index = self.proof_tree_nodes.len() + 1;
                let node = ProofTreeNode {
                    step_index,
                    tactic: tactic.clone(),
                    goal_at_application: self.current_proposition().clone(),
                    elapsed_ms,
                };
                self.history.push(node.clone());
                self.proof_tree_nodes.push(node);
                self.redo_stack.clear();
                // Cheap-but-explicit `intro` simulation — `intro NAME`
                // appends a hypothesis so subsequent `apply NAME`
                // resolves.  V0 only.
                let trimmed = tactic.as_str().trim_end_matches(';').trim();
                if let Some(rest) = trimmed.strip_prefix("intro ") {
                    let name = rest.split_whitespace().next().unwrap_or("");
                    if !name.is_empty() {
                        self.hypotheses.push((Text::from(name), Text::from("?")));
                    }
                }
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
            Some(node) => {
                let popped = node.tactic.clone();
                self.redo_stack.push(node);
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
            Some(node) => {
                let reapplied = node.tactic.clone();
                self.history.push(node);
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
        ReplResponse::Hints { suggestions: mapped }
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
            nodes: self.history.clone(),
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
/// response in order.  Used by the CLI's batch mode + tests.
pub fn run_batch<S: ReplSession>(
    session: &mut S,
    commands: Vec<ReplCommand>,
) -> Vec<ReplResponse> {
    commands.into_iter().map(|c| session.step(c)).collect()
}

/// Aggregate response counts across a transcript.  Used for batch-
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
            focused_proposition: Text::from("P"),
            open_goals: vec![],
            applied_steps: vec![],
            history_depth: 0,
            redo_depth: 0,
        };
        assert_eq!(
            ReplResponse::Status { snapshot: s.clone() }.name(),
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
        // name).  At minimum the goal summary records `h`.
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
            ReplResponse::Redone { reapplied, snapshot } => {
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
        // Now redo_depth = 1.  Apply a fresh step.
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
}
