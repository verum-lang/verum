//! Interactive Theorem Proving
//!
//! Implements an interactive proof mode for step-by-step theorem proving.
//!
//! Implements the interactive proof assistant mode (`@interactive` annotation on theorems).
//! Supports goal-directed proving with intro/split/apply/destruct tactics, induction with
//! IH hypotheses, proof by reflection (quote goal + decision procedure), and Ltac-style
//! scripts with repeat-match patterns. Proofs can be exported to Coq, Lean, and Dedukti.
//!
//! ## Features
//!
//! - **Goal-Directed Proving**: Track current proof goals with context
//! - **Tactic Application**: Apply tactics interactively with pattern matching
//! - **Proof History**: Undo/redo support with full state restoration
//! - **Proof Scripts**: Record and replay proof sequences (Ltac-style)
//! - **Proof State Display**: Show current goals and hypotheses
//! - **Focus Management**: Focus on specific goals or subgoals
//! - **Proof by Reflection**: Quote goals and run decision procedures
//!
//! ## Performance Targets
//!
//! - Tactic application: < 10ms
//! - State display: < 5ms
//! - Undo/redo: < 1ms
//! - Pattern matching: < 5ms

use std::time::Instant;

use verum_ast::{BinOp, Expr, ExprKind, Pattern};
use verum_common::{List, Map, Maybe, Text};

use crate::context::Context;
use crate::proof_search::{
    HintsDatabase, ProofError, ProofGoal, ProofSearchEngine, ProofTactic, ProofTree,
};

// ==================== Interactive Prover ====================

/// Interactive theorem prover with full goal tracking and state management
///
/// Implements interactive proof development with goal tracking, undo/redo, and focus
/// management. Goals display hypotheses (H0, H1, ...) and turnstile notation (|-).
/// Supports `intro x y` to introduce variables, induction with case splitting,
/// simp for simplification, and `apply IH` for hypothesis application.
#[derive(Debug)]
pub struct InteractiveProver {
    /// Current proof goals (stack-based)
    goals: List<ProofGoal>,

    /// Current proof state/tree
    proof_state: ProofTree,

    /// Command history for undo/redo
    history: List<ProofCommand>,

    /// Redo stack
    redo_stack: List<ProofCommand>,

    /// Underlying proof engine
    engine: ProofSearchEngine,

    /// Configuration
    config: ProverConfig,

    /// Focus stack for nested goal focusing
    focus_stack: List<usize>,

    /// Statistics
    stats: ProverStats,
}

/// Prover configuration
#[derive(Debug, Clone)]
pub struct ProverConfig {
    /// Maximum history size
    pub max_history: usize,

    /// Enable auto-simplification
    pub auto_simplify: bool,

    /// Verbose output
    pub verbose: bool,

    /// Maximum auto-proof depth
    pub max_auto_depth: usize,

    /// Enable proof by reflection
    pub enable_reflection: bool,

    /// Timeout for decision procedures (ms)
    pub decision_timeout_ms: u64,
}

impl Default for ProverConfig {
    fn default() -> Self {
        Self {
            max_history: 1000,
            auto_simplify: true,
            verbose: false,
            max_auto_depth: 100,
            enable_reflection: true,
            decision_timeout_ms: 5000,
        }
    }
}

/// Proof command (for history) with full state capture
#[derive(Debug, Clone)]
pub struct ProofCommand {
    /// The tactic that was applied
    pub tactic: ProofTactic,

    /// Previous goals (for undo)
    pub previous_goals: List<ProofGoal>,

    /// New goals (after tactic)
    pub new_goals: List<ProofGoal>,

    /// Focus stack at time of command
    pub focus_stack: List<usize>,

    /// Timestamp
    pub timestamp: u64,
}

/// Proof state for display
///
/// Proof state display showing numbered goals with hypotheses and turnstile notation.
/// Example: "2 goals: x: Nat, y: Nat |- P(x, y)" and "x: Nat, y: Nat |- Q(x, y)".
#[derive(Debug, Clone)]
pub struct ProofState {
    /// Current goals
    pub goals: List<ProofGoal>,

    /// Status message
    pub message: Text,

    /// Whether proof is complete
    pub complete: bool,

    /// Current focus (if any)
    pub focus: Maybe<usize>,

    /// Number of steps taken
    pub steps: usize,
}

impl ProofState {
    /// Create a new proof state
    pub fn new(goals: List<ProofGoal>, message: Text) -> Self {
        let complete = goals.is_empty();
        Self {
            goals,
            message,
            complete,
            focus: Maybe::None,
            steps: 0,
        }
    }

    /// Display goals count
    pub fn goals_message(&self) -> Text {
        if self.complete {
            "No goals remaining. Proof complete!".into()
        } else if self.goals.len() == 1 {
            "1 goal remaining".into()
        } else {
            format!("{} goals remaining", self.goals.len()).into()
        }
    }

    /// Get current goal (first in list)
    pub fn current_goal(&self) -> Maybe<&ProofGoal> {
        self.goals.first()
    }

    /// Set focus
    pub fn with_focus(mut self, index: usize) -> Self {
        self.focus = Maybe::Some(index);
        self
    }

    /// Set step count
    pub fn with_steps(mut self, steps: usize) -> Self {
        self.steps = steps;
        self
    }
}

/// Prover statistics
#[derive(Debug, Clone, Default)]
pub struct ProverStats {
    /// Total tactics applied
    pub tactics_applied: usize,

    /// Successful tactics
    pub tactics_succeeded: usize,

    /// Failed tactics
    pub tactics_failed: usize,

    /// Auto-proofs attempted
    pub auto_attempts: usize,

    /// Auto-proofs succeeded
    pub auto_successes: usize,

    /// Total time spent (microseconds)
    pub total_time_us: u64,

    /// Undo operations
    pub undo_count: usize,

    /// Redo operations
    pub redo_count: usize,
}

impl InteractiveProver {
    /// Create a new interactive prover
    pub fn new(initial_goal: ProofGoal) -> Self {
        let goals = List::from_iter(vec![initial_goal.clone()]);
        let proof_state = ProofTree::new(initial_goal);

        Self {
            goals,
            proof_state,
            history: List::new(),
            redo_stack: List::new(),
            engine: ProofSearchEngine::new(),
            config: ProverConfig::default(),
            focus_stack: List::new(),
            stats: ProverStats::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(initial_goal: ProofGoal, config: ProverConfig) -> Self {
        let mut prover = Self::new(initial_goal);
        prover.config = config;
        prover
    }

    /// Create with hints database
    pub fn with_hints(initial_goal: ProofGoal, hints: HintsDatabase) -> Self {
        let mut prover = Self::new(initial_goal);
        prover.engine = ProofSearchEngine::with_hints(hints);
        prover
    }

    /// Apply a tactic to current goal
    ///
    /// Apply a tactic to the current goal, updating the proof state.
    /// Forward reasoning (have), backward reasoning (suffices), case analysis,
    /// and induction are supported. Records the step in history for undo.
    pub fn step(&mut self, tactic: ProofTactic) -> Result<ProofState, ProofError> {
        let start = Instant::now();
        self.stats.tactics_applied += 1;

        // Get current goal
        let current_goal = self
            .current_goal()
            .ok_or_else(|| ProofError::TacticFailed("No goals remaining".into()))?
            .clone();

        // Honour `ProverConfig.verbose` — emit a structured trace
        // for every tactic application so users debugging a stuck
        // proof can follow what the prover is doing. Without this
        // gate the field was inert: setting verbose = true had no
        // observable effect on the prover's output.
        if self.config.verbose {
            tracing::info!(
                "interactive prover: applying {:?} to goal #{} of {}",
                tactic,
                self.goals.len() - self.goals.iter().position(|g| {
                    std::ptr::eq(g, &current_goal)
                }).unwrap_or(0),
                self.goals.len()
            );
        }

        // Execute tactic
        let result = self.engine.execute_tactic(&tactic, &current_goal);

        match result {
            Ok(new_goals) => {
                self.stats.tactics_succeeded += 1;

                // Save to history
                let command = ProofCommand {
                    tactic: tactic.clone(),
                    previous_goals: self.goals.clone(),
                    new_goals: new_goals.clone(),
                    focus_stack: self.focus_stack.clone(),
                    timestamp: start.elapsed().as_micros() as u64,
                };

                self.history.push(command);

                // Clear redo stack (new action invalidates redo)
                self.redo_stack.clear();

                // Trim history if needed
                if self.history.len() > self.config.max_history {
                    self.history.remove(0);
                }

                // Update goals: replace current with new goals
                self.goals.remove(0);
                for goal in new_goals.iter().rev() {
                    self.goals.insert(0, goal.clone());
                }

                // Update statistics
                self.stats.total_time_us += start.elapsed().as_micros() as u64;

                // Create state message
                let message = if self.goals.is_empty() {
                    "Proof complete!".into()
                } else if new_goals.is_empty() {
                    "Goal proved ✓".into()
                } else {
                    format!(
                        "{} goals remaining (added {} subgoals)",
                        self.goals.len(),
                        new_goals.len()
                    )
                    .into()
                };

                Ok(ProofState::new(self.goals.clone(), message)
                    .with_steps(self.stats.tactics_applied))
            }
            Err(e) => {
                self.stats.tactics_failed += 1;
                self.stats.total_time_us += start.elapsed().as_micros() as u64;
                Err(e)
            }
        }
    }

    /// Undo last tactic
    ///
    /// Undo last tactic application, restoring the previous proof state from history.
    pub fn undo(&mut self) -> Result<ProofState, ProofError> {
        // Pop last command from history
        let command = self
            .history
            .pop()
            .ok_or_else(|| ProofError::TacticFailed("Nothing to undo".into()))?;

        // Save to redo stack
        self.redo_stack.push(command.clone());

        // Restore previous goals
        self.goals = command.previous_goals;

        // Restore focus stack
        self.focus_stack = command.focus_stack;

        self.stats.undo_count += 1;

        let message = format!("Undone. {} goals remaining", self.goals.len()).into();
        Ok(ProofState::new(self.goals.clone(), message).with_steps(self.stats.tactics_applied))
    }

    /// Redo last undone tactic
    pub fn redo(&mut self) -> Result<ProofState, ProofError> {
        // Pop from redo stack
        let command = self
            .redo_stack
            .pop()
            .ok_or_else(|| ProofError::TacticFailed("Nothing to redo".into()))?;

        // Re-apply the tactic
        self.stats.redo_count += 1;
        self.step(command.tactic)
    }

    /// Get current proof state
    pub fn state(&self) -> ProofState {
        let message = if self.goals.is_empty() {
            "No goals remaining. Proof complete!".into()
        } else {
            format!("{} goals remaining", self.goals.len()).into()
        };

        ProofState::new(self.goals.clone(), message).with_steps(self.stats.tactics_applied)
    }

    /// Get current goal
    pub fn current_goal(&self) -> Maybe<&ProofGoal> {
        self.goals.first()
    }

    /// Check if proof is complete
    pub fn is_complete(&self) -> bool {
        self.goals.is_empty()
    }

    /// Get all current goals
    pub fn goals(&self) -> &List<ProofGoal> {
        &self.goals
    }

    /// Get command history
    pub fn history(&self) -> &List<ProofCommand> {
        &self.history
    }

    /// Get the underlying proof search engine
    pub fn engine(&self) -> &ProofSearchEngine {
        &self.engine
    }

    /// Get mutable access to proof search engine
    pub fn engine_mut(&mut self) -> &mut ProofSearchEngine {
        &mut self.engine
    }

    /// Get statistics
    pub fn stats(&self) -> &ProverStats {
        &self.stats
    }

    /// Try to finish proof automatically
    pub fn auto_finish(&mut self) -> Result<ProofState, ProofError> {
        self.stats.auto_attempts += 1;

        let mut steps = 0;
        while !self.is_complete() {
            // Try auto tactic
            match self.step(ProofTactic::Auto) {
                Ok(state) => {
                    if state.complete {
                        self.stats.auto_successes += 1;
                        return Ok(state);
                    }
                }
                Err(_) => {
                    return Err(ProofError::TacticFailed(
                        "Auto tactic failed to complete proof".into(),
                    ));
                }
            }

            // Prevent infinite loops
            steps += 1;
            if steps > self.config.max_auto_depth {
                return Err(ProofError::TacticFailed(
                    format!(
                        "Auto proof exceeded step limit ({})",
                        self.config.max_auto_depth
                    )
                    .into(),
                ));
            }
        }

        self.stats.auto_successes += 1;
        Ok(self.state())
    }

    /// Focus on a specific goal
    ///
    /// Focus on a specific goal by index, using `{ }` brackets in proof scripts.
    /// Only the focused goal can be modified until focus is released.
    pub fn focus(&mut self, index: usize) -> Result<(), ProofError> {
        if index >= self.goals.len() {
            return Err(ProofError::TacticFailed(
                format!("Goal index {} out of range", index).into(),
            ));
        }

        // Push current focus to stack
        self.focus_stack.push(index);

        // Move goal to front
        if index > 0
            && let Some(goal) = self.goals.get(index).cloned()
        {
            self.goals.remove(index);
            self.goals.insert(0, goal);
        }

        Ok(())
    }

    /// Unfocus - pop focus stack
    pub fn unfocus(&mut self) -> Result<(), ProofError> {
        if self.focus_stack.is_empty() {
            return Err(ProofError::TacticFailed("Not in focus mode".into()));
        }

        self.focus_stack.pop();
        Ok(())
    }

    /// Get current focus depth
    pub fn focus_depth(&self) -> usize {
        self.focus_stack.len()
    }

    /// Reset proof to initial state
    pub fn reset(&mut self) {
        if let Some(initial) = self.history.first().map(|c| &c.previous_goals) {
            self.goals = initial.clone();
        }
        self.history.clear();
        self.redo_stack.clear();
        self.focus_stack.clear();
        self.stats = ProverStats::default();
    }

    /// Quote current goal for reflection
    ///
    /// Quote the current goal as an expression for proof by reflection.
    /// The quoted goal can be passed to a decision procedure for automatic solving.
    pub fn quote_goal(&self) -> Result<Expr, ProofError> {
        let goal = self
            .current_goal()
            .ok_or_else(|| ProofError::TacticFailed("No goal to quote".into()))?;

        Ok(goal.goal.clone())
    }

    /// Run decision procedure on quoted goal
    ///
    /// Proof by reflection: quote the goal formula, run a decision procedure
    /// (e.g., SMT solver), and if valid, close the goal automatically.
    ///
    /// This implements proof by reflection:
    /// 1. Translate the formula to SMT-LIB via Z3
    /// 2. Check validity by asserting ¬formula and checking for UNSAT
    /// 3. If UNSAT, the formula is valid (proven)
    /// 4. If SAT, return counterexample information
    pub fn run_decision_procedure(&mut self, formula: &Expr) -> Result<ProofState, ProofError> {
        use crate::translate::Translator;

        if !self.config.enable_reflection {
            return Err(ProofError::TacticFailed("Reflection is disabled".into()));
        }

        // Create Z3 context with decision timeout
        let mut config = crate::context::ContextConfig::default();
        config.timeout = std::time::Duration::from_millis(self.config.decision_timeout_ms).into();
        let ctx = Context::with_config(config);

        // Create translator
        let translator = Translator::new(&ctx);

        // Translate formula to Z3
        let z3_formula = match translator.translate_expr(formula) {
            Ok(ast) => ast,
            Err(e) => {
                return Err(ProofError::TacticFailed(
                    format!("Failed to translate formula to SMT: {:?}", e).into(),
                ));
            }
        };

        // Get the Bool AST
        let z3_bool = match z3_formula.as_bool() {
            Some(b) => b,
            None => {
                return Err(ProofError::TacticFailed(
                    "Formula does not translate to a boolean constraint".into(),
                ));
            }
        };

        // Create solver
        let solver = ctx.solver();

        // To check validity of P, we check if ¬P is UNSAT
        // If ¬P is UNSAT, then P is valid (a tautology)
        let negated = z3_bool.not();
        solver.assert(&negated);

        // Check satisfiability
        match solver.check() {
            z3::SatResult::Unsat => {
                // ¬P is UNSAT, so P is valid (proven!)
                // Apply the SMT tactic to close the goal
                self.step(ProofTactic::Smt {
                    solver: Maybe::Some("z3".into()),
                    timeout_ms: Maybe::Some(self.config.decision_timeout_ms),
                })
            }
            z3::SatResult::Sat => {
                // ¬P is SAT, so P is not valid
                // Get counterexample from model
                let counterexample = if let Some(model) = solver.get_model() {
                    format!("Counterexample found: {}", model)
                } else {
                    "Formula is not valid (counterexample exists)".to_string()
                };

                Err(ProofError::TacticFailed(counterexample.into()))
            }
            z3::SatResult::Unknown => {
                // Solver could not determine satisfiability
                // This usually means timeout or resource limits
                Err(ProofError::TacticFailed(
                    "Decision procedure timed out or returned unknown".into(),
                ))
            }
        }
    }

    /// Apply tactic with goal pattern matching
    ///
    /// Apply a tactic based on goal pattern matching (Ltac-style `match goal with`).
    /// Patterns include conjunction (split), implication (intro), existential (destruct),
    /// and hypothesis patterns for automated tactic dispatch.
    pub fn apply_with_match(
        &mut self,
        pattern: GoalPattern,
        tactic: ProofTactic,
    ) -> Result<ProofState, ProofError> {
        let current_goal = self
            .current_goal()
            .ok_or_else(|| ProofError::TacticFailed("No goal to match".into()))?;

        if pattern.matches(&current_goal.goal) {
            self.step(tactic)
        } else {
            Err(ProofError::TacticFailed(
                "Goal pattern does not match".into(),
            ))
        }
    }

    /// Add hypothesis to current goal
    pub fn add_hypothesis(&mut self, name: Text, hyp: Expr) -> Result<(), ProofError> {
        if self.goals.is_empty() {
            return Err(ProofError::TacticFailed("No current goal".into()));
        }

        // Modify the first goal to add the hypothesis
        if let Some(goal) = self.goals.get_mut(0) {
            goal.add_hypothesis(hyp);
        }

        Ok(())
    }

    /// Show proof tree structure
    pub fn proof_tree(&self) -> &ProofTree {
        &self.proof_state
    }
}

// ==================== Goal Pattern Matching ====================

/// Pattern for matching proof goals
///
/// Patterns for matching proof goals in Ltac-style tactic scripts.
/// Used in `match goal with | pattern => tactic` constructs for automated
/// tactic dispatch based on goal structure.
#[derive(Debug, Clone)]
pub enum GoalPattern {
    /// Match conjunction: _ ∧ _
    Conjunction,

    /// Match implication: _ → _
    Implication,

    /// Match negation: ¬_
    Negation,

    /// Match universal quantifier: ∀x. _
    ForAll,

    /// Match existential quantifier: ∃x. _
    Exists,

    /// Match equality: _ = _
    Equality,

    /// Match disjunction: _ ∨ _
    Disjunction,

    /// Match any goal
    Any,

    /// Custom pattern with predicate
    Custom(fn(&Expr) -> bool),
}

impl GoalPattern {
    /// Check if pattern matches expression
    pub fn matches(&self, expr: &Expr) -> bool {
        match self {
            GoalPattern::Conjunction => {
                matches!(&expr.kind, ExprKind::Binary { op: BinOp::And, .. })
            }
            GoalPattern::Implication => matches!(
                &expr.kind,
                ExprKind::Binary {
                    op: BinOp::Imply,
                    ..
                }
            ),
            GoalPattern::Negation => matches!(&expr.kind, ExprKind::Unary { .. }),
            GoalPattern::Equality => matches!(&expr.kind, ExprKind::Binary { op: BinOp::Eq, .. }),
            GoalPattern::Disjunction => {
                matches!(&expr.kind, ExprKind::Binary { op: BinOp::Or, .. })
            }
            GoalPattern::ForAll => {
                // Check for forall quantifier
                matches!(&expr.kind, ExprKind::Forall { .. })
            }
            GoalPattern::Exists => {
                // Check for exists quantifier
                matches!(&expr.kind, ExprKind::Exists { .. })
            }
            GoalPattern::Any => true,
            GoalPattern::Custom(pred) => pred(expr),
        }
    }
}

// ==================== Proof Scripts ====================

/// Proof script (sequence of tactics)
///
/// Ltac-style proof scripts: recorded sequences of tactics that can be replayed.
/// Scripts support repeat-match patterns for automated proving and can include
/// reflection steps that quote goals and run decision procedures.
#[derive(Debug, Clone)]
pub struct ProofScript {
    /// Script name
    pub name: Text,

    /// Tactics in the script
    pub tactics: List<TacticStep>,

    /// Script description
    pub description: Text,
}

/// Single step in proof script
#[derive(Debug, Clone)]
pub enum TacticStep {
    /// Single tactic
    Tactic(ProofTactic),

    /// Conditional: match goal pattern then apply tactic
    Match {
        pattern: GoalPattern,
        tactic: Box<TacticStep>,
    },

    /// Repeat until fixpoint or max iterations
    Repeat {
        tactic: Box<TacticStep>,
        max_iterations: usize,
    },

    /// Try tactic, continue on failure
    Try(Box<TacticStep>),

    /// Sequence of tactics
    Sequence(List<TacticStep>),

    /// First successful tactic
    First(List<TacticStep>),

    /// No-op (idtac in Coq/Ltac)
    Idtac,
}

impl ProofScript {
    /// Create a new proof script
    pub fn new(name: Text) -> Self {
        Self {
            name,
            tactics: List::new(),
            description: Text::new(),
        }
    }

    /// Add tactic to script
    pub fn add_tactic(&mut self, tactic: ProofTactic) {
        self.tactics.push(TacticStep::Tactic(tactic));
    }

    /// Add tactic step to script
    pub fn add_step(&mut self, step: TacticStep) {
        self.tactics.push(step);
    }

    /// Execute script on prover
    pub fn execute(&self, prover: &mut InteractiveProver) -> Result<ProofState, ProofError> {
        for step in &self.tactics {
            Self::execute_step(step, prover)?;
        }

        Ok(prover.state())
    }

    /// Execute a single tactic step
    fn execute_step(step: &TacticStep, prover: &mut InteractiveProver) -> Result<(), ProofError> {
        match step {
            TacticStep::Tactic(tactic) => {
                prover.step(tactic.clone())?;
                Ok(())
            }
            TacticStep::Match { pattern, tactic } => {
                if let Some(goal) = prover.current_goal()
                    && pattern.matches(&goal.goal)
                {
                    Self::execute_step(tactic, prover)?;
                }
                Ok(())
            }
            TacticStep::Repeat {
                tactic,
                max_iterations,
            } => {
                for _ in 0..*max_iterations {
                    match Self::execute_step(tactic, prover) {
                        Ok(_) => {}
                        Err(_) => break, // Stop on failure
                    }
                    // Check if we've reached a fixpoint
                    if prover.is_complete() {
                        break;
                    }
                }
                Ok(())
            }
            TacticStep::Try(tactic) => {
                // Try tactic, ignore failure
                let _ = Self::execute_step(tactic, prover);
                Ok(())
            }
            TacticStep::Sequence(steps) => {
                for s in steps {
                    Self::execute_step(s, prover)?;
                }
                Ok(())
            }
            TacticStep::First(alternatives) => {
                for alt in alternatives {
                    if Self::execute_step(alt, prover).is_ok() {
                        return Ok(());
                    }
                }
                Err(ProofError::TacticFailed("All alternatives failed".into()))
            }
            TacticStep::Idtac => Ok(()), // No-op
        }
    }

    /// Create script from history
    pub fn from_history(prover: &InteractiveProver) -> Self {
        let mut script = Self::new("recorded_proof".into());
        script.description = "Proof recorded from interactive session".into();

        for command in prover.history() {
            script.add_tactic(command.tactic.clone());
        }

        script
    }

    /// Create a repeat-match script (Ltac-style)
    ///
    /// Create a classic Ltac-style repeat-match script:
    /// `repeat { match goal with | |- _ /\ _ => split | |- _ -> _ => intro | _ => idtac }; auto`
    pub fn ltac_style(name: Text) -> Self {
        let mut script = Self::new(name);

        // Build the classic Ltac pattern:
        // repeat { match goal with | pattern => tactic | ... end }; auto

        let mut alternatives = List::new();

        // ⊢ _ ∧ _ => split
        alternatives.push(TacticStep::Match {
            pattern: GoalPattern::Conjunction,
            tactic: Box::new(TacticStep::Tactic(ProofTactic::Split)),
        });

        // ⊢ _ → _ => intro
        alternatives.push(TacticStep::Match {
            pattern: GoalPattern::Implication,
            tactic: Box::new(TacticStep::Tactic(ProofTactic::Intro)),
        });

        // _ => idtac
        alternatives.push(TacticStep::Idtac);

        let match_step = TacticStep::First(alternatives);

        let repeat_step = TacticStep::Repeat {
            tactic: Box::new(match_step),
            max_iterations: 100,
        };

        script.add_step(repeat_step);
        script.add_tactic(ProofTactic::Auto);

        script
    }

    /// Create reflection script
    ///
    /// Create a proof-by-reflection script: quote the goal, run a decision procedure,
    /// and use the resulting proof term via `exact`.
    pub fn reflection_script(name: Text) -> Self {
        let mut script = Self::new(name);
        script.description = "Proof by reflection using decision procedure".into();

        // In a full implementation, this would:
        // 1. Quote the goal
        // 2. Run decision procedure
        // 3. Exact the proof term

        script.add_tactic(ProofTactic::Auto);
        script
    }
}

// ==================== Proof Script Library ====================

/// Library of reusable proof scripts
pub struct ScriptLibrary {
    /// Scripts indexed by name
    scripts: Map<Text, ProofScript>,
}

impl ScriptLibrary {
    /// Create a new script library
    pub fn new() -> Self {
        Self {
            scripts: Map::new(),
        }
    }

    /// Add script to library
    pub fn add(&mut self, script: ProofScript) {
        self.scripts.insert(script.name.clone(), script);
    }

    /// Find script by name
    pub fn find(&self, name: &Text) -> Maybe<&ProofScript> {
        self.scripts.get(name)
    }

    /// Get all scripts
    pub fn all(&self) -> List<ProofScript> {
        self.scripts.values().cloned().collect()
    }

    /// Create library with standard tactics
    pub fn with_standard() -> Self {
        let mut lib = Self::new();

        // Auto tactic
        let mut auto = ProofScript::new("auto".into());
        auto.description = "Automatic proof search".into();
        auto.add_tactic(ProofTactic::Auto);
        lib.add(auto);

        // Intro-split pattern
        let mut intro_split = ProofScript::new("intro_split".into());
        intro_split.description = "Introduce hypothesis then split".into();
        intro_split.add_tactic(ProofTactic::Intro);
        intro_split.add_tactic(ProofTactic::Split);
        lib.add(intro_split);

        // Ltac-style script
        let ltac = ProofScript::ltac_style("ltac_auto".into());
        lib.add(ltac);

        // Reflection script
        let reflect = ProofScript::reflection_script("reflect".into());
        lib.add(reflect);

        // Induction script
        let mut induction = ProofScript::new("induction_auto".into());
        induction.description = "Induction with automatic subgoal solving".into();
        induction.add_tactic(ProofTactic::Induction {
            var: Text::from("x"),
        });
        induction.add_tactic(ProofTactic::Auto);
        lib.add(induction);

        lib
    }

    /// List all script names
    pub fn list_scripts(&self) -> List<Text> {
        self.scripts.keys().cloned().collect()
    }

    /// Get script count
    pub fn count(&self) -> usize {
        self.scripts.len()
    }
}

impl Default for ScriptLibrary {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Pretty Printing ====================

/// Format proof goal for display
///
/// Format a proof goal for display: show hypotheses (H0: ..., H1: ...) and the
/// goal with turnstile notation (|- proposition).
pub fn format_goal(goal: &ProofGoal, index: usize) -> Text {
    let mut output = Text::new();

    // Show goal header
    if let Some(label) = &goal.label {
        output.push_str(&format!("Goal {} ({}):\n", index + 1, label));
    } else {
        output.push_str(&format!("Goal {}:\n", index + 1));
    }

    // Show hypotheses
    if !goal.hypotheses.is_empty() {
        output.push_str("  Hypotheses:\n");
        for (i, hyp) in goal.hypotheses.iter().enumerate() {
            output.push_str(&format!("    H{}: {}\n", i, format_expr(hyp)));
        }
        output.push('\n');
    }

    // Show goal
    output.push_str(&format!("  ⊢ {}\n", format_expr(&goal.goal)));

    output
}

/// Format proof state for display
///
/// Format the full proof state: show completion status, remaining goal count,
/// focused goal indicator, and per-goal hypothesis/turnstile display.
pub fn format_state(state: &ProofState) -> Text {
    let mut output = Text::new();

    if state.complete {
        output.push_str("✓ Proof complete!\n");
        output.push_str(&format!("  Completed in {} steps\n", state.steps));
    } else {
        output.push_str(&format!("{}\n\n", state.goals_message()));

        // Show focus if active
        if let Maybe::Some(focus) = state.focus {
            output.push_str(&format!("(Focused on goal {})\n\n", focus + 1));
        }

        for (i, goal) in state.goals.iter().enumerate() {
            let formatted = format_goal(goal, i);
            output.push_str(formatted.as_str());
            output.push('\n');
        }
    }

    output
}

/// Format pattern for display
fn format_pattern(pattern: &Pattern) -> Text {
    use verum_ast::PatternKind;

    match &pattern.kind {
        PatternKind::Ident { name, .. } => name.name.clone().into(),
        PatternKind::Wildcard => "_".into(),
        PatternKind::Tuple(patterns) => {
            let patterns_str: Vec<String> = patterns
                .iter()
                .map(|p| format_pattern(p).to_string())
                .collect();
            format!("({})", patterns_str.join(", ")).into()
        }
        _ => format!("{:?}", pattern).into(),
    }
}

/// Format expression for display
fn format_expr(expr: &Expr) -> Text {
    match &expr.kind {
        ExprKind::Path(path) => format!("{:?}", path).into(),
        ExprKind::Literal(lit) => format!("{:?}", lit.kind).into(),
        ExprKind::Binary { op, left, right } => format!(
            "{} {} {}",
            format_expr(left),
            op.as_str(),
            format_expr(right)
        )
        .into(),
        ExprKind::Unary { op, expr } => format!("{}{}", op.as_str(), format_expr(expr)).into(),
        ExprKind::Paren(e) => format!("({})", format_expr(e)).into(),
        ExprKind::Call { func, args, .. } => {
            let args_str: Vec<String> = args.iter().map(|a| format_expr(a).to_string()).collect();
            format!("{}({})", format_expr(func), args_str.join(", ")).into()
        }
        ExprKind::Forall { bindings, body } => {
            let bindings_str: Vec<String> = bindings
                .iter()
                .map(|b| format_pattern(&b.pattern).to_string())
                .collect();
            format!("∀{}. {}", bindings_str.join(", "), format_expr(body)).into()
        }
        ExprKind::Exists { bindings, body } => {
            let bindings_str: Vec<String> = bindings
                .iter()
                .map(|b| format_pattern(&b.pattern).to_string())
                .collect();
            format!("∃{}. {}", bindings_str.join(", "), format_expr(body)).into()
        }
        _ => format!("{:?}", expr).into(),
    }
}

/// Format proof command for display
pub fn format_command(cmd: &ProofCommand) -> Text {
    format!("{:?} ({}μs)", cmd.tactic, cmd.timestamp).into()
}

/// Format proof history
pub fn format_history(history: &List<ProofCommand>) -> Text {
    let mut output = Text::new();
    output.push_str("Proof History:\n");

    for (i, cmd) in history.iter().enumerate() {
        output.push_str(&format!("  {}: {}\n", i + 1, format_command(cmd)));
    }

    output
}

/// Format prover statistics
pub fn format_stats(stats: &ProverStats) -> Text {
    let mut output = Text::new();
    output.push_str("Proof Statistics:\n");
    output.push_str(&format!("  Tactics applied: {}\n", stats.tactics_applied));
    output.push_str(&format!(
        "  Tactics succeeded: {}\n",
        stats.tactics_succeeded
    ));
    output.push_str(&format!("  Tactics failed: {}\n", stats.tactics_failed));
    output.push_str(&format!("  Auto attempts: {}\n", stats.auto_attempts));
    output.push_str(&format!("  Auto successes: {}\n", stats.auto_successes));
    output.push_str(&format!("  Total time: {}μs\n", stats.total_time_us));
    output.push_str(&format!("  Undo count: {}\n", stats.undo_count));
    output.push_str(&format!("  Redo count: {}\n", stats.redo_count));

    let success_rate = if stats.tactics_applied > 0 {
        (stats.tactics_succeeded as f64 / stats.tactics_applied as f64) * 100.0
    } else {
        0.0
    };
    output.push_str(&format!("  Success rate: {:.1}%\n", success_rate));

    output
}

// ==================== REPL Integration ====================

/// Commands for REPL integration
#[derive(Debug, Clone)]
pub enum InteractiveCommand {
    /// Apply tactic
    Step(ProofTactic),

    /// Undo last step
    Undo,

    /// Redo last undone step
    Redo,

    /// Show current state
    Show,

    /// Show proof history
    History,

    /// Show statistics
    Stats,

    /// Focus on goal
    Focus(usize),

    /// Unfocus
    Unfocus,

    /// Run script
    RunScript(Text),

    /// Auto-finish proof
    AutoFinish,

    /// Quote goal
    Quote,

    /// Reset proof
    Reset,

    /// Show help
    Help,
}

/// Parse interactive command from string
pub fn parse_command(input: &str) -> Result<InteractiveCommand, Text> {
    let input = input.trim();

    if input.starts_with("undo") {
        Ok(InteractiveCommand::Undo)
    } else if input.starts_with("redo") {
        Ok(InteractiveCommand::Redo)
    } else if input.starts_with("show") {
        Ok(InteractiveCommand::Show)
    } else if input.starts_with("history") {
        Ok(InteractiveCommand::History)
    } else if input.starts_with("stats") {
        Ok(InteractiveCommand::Stats)
    } else if input.starts_with("focus") {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.len() < 2 {
            return Err("Usage: focus <index>".into());
        }
        let index: usize = parts[1].parse().map_err(|_| Text::from("Invalid index"))?;
        Ok(InteractiveCommand::Focus(index))
    } else if input.starts_with("unfocus") {
        Ok(InteractiveCommand::Unfocus)
    } else if input.starts_with("auto") {
        Ok(InteractiveCommand::AutoFinish)
    } else if input.starts_with("quote") {
        Ok(InteractiveCommand::Quote)
    } else if input.starts_with("reset") {
        Ok(InteractiveCommand::Reset)
    } else if input.starts_with("help") {
        Ok(InteractiveCommand::Help)
    } else if input.starts_with("intro") {
        Ok(InteractiveCommand::Step(ProofTactic::Intro))
    } else if input.starts_with("split") {
        Ok(InteractiveCommand::Step(ProofTactic::Split))
    } else if input.starts_with("refl") {
        Ok(InteractiveCommand::Step(ProofTactic::Reflexivity))
    } else if input.starts_with("assumption") {
        Ok(InteractiveCommand::Step(ProofTactic::Assumption))
    } else {
        Err(format!("Unknown command: {}", input).into())
    }
}

/// Get help text for interactive mode
pub fn help_text() -> Text {
    Text::from(
        r#"Interactive Proof Assistant Commands:

Tactics:
  intro          - Introduce hypothesis
  split          - Split conjunction
  refl           - Apply reflexivity
  assumption     - Use assumption
  auto           - Automatic proof search

Navigation:
  show           - Show current state
  history        - Show proof history
  stats          - Show statistics
  focus <n>      - Focus on goal n
  unfocus        - Exit focus mode

Control:
  undo           - Undo last step
  redo           - Redo last undone step
  reset          - Reset to initial state
  quote          - Quote current goal

Advanced:
  auto           - Auto-finish proof
  help           - Show this help
"#,
    )
}
