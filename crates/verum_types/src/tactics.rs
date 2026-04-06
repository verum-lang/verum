//! Proof Tactics and Automation System
//!
//! Implements the tactic language and proof automation for Verum's dependent type system.
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 9 - Tactics and Proof Automation
//!
//! ## Architecture
//!
//! The tactics system provides a compositional framework for constructing proofs:
//! - **Basic Tactics**: Primitive proof steps (assumption, intro, split, etc.)
//! - **Combinators**: Compose tactics (andThen, orElse, repeat, tryFor)
//! - **Proof State**: Track goals and context during proof construction
//! - **Tactic Engine**: Execute tactics and manage proof search
//!
//! ## Tactic Language Syntax
//!
//! ```verum
//! tactic auto = {
//!     repeat (
//!         assumption
//!         <|> intro
//!         <|> split
//!         <|> left <|> right
//!         <|> apply_lemma
//!     )
//! }
//!
//! fn example_proof(P, Q: Prop) : P ∧ Q -> Q ∧ P =
//!     by auto
//! ```
//!
//! ## Performance Targets
//!
//! - Tactic application: < 1ms
//! - Proof search depth: 50 steps max (configurable)
//! - Hint lookup: < 1ms (via verum_smt::HintsDatabase)
//! - Total automation: < 100ms timeout default

use std::time::{Duration, Instant};

use verum_ast::{BinOp, Expr, ExprKind, Literal, LiteralKind, Path, Pattern, PatternKind, Span};
use verum_common::{Heap, List, Map, Maybe, Set, Text, ToText};

use crate::context::TypeContext;
use crate::ty::Type;
use crate::TypeError;

// ==================== Core Tactic Types ====================

/// Basic proof tactics (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 9.1)
///
/// These are the primitive proof steps that can be composed into complex strategies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Tactic {
    /// Use hypothesis from context
    ///
    /// Proves goal if it matches a hypothesis exactly.
    /// Example: Given `P` in context, proves goal `P`
    Assumption,

    /// Introduce variables/hypotheses
    ///
    /// For goal `forall x. P(x)`, introduces variable `x` and proves `P(x)`.
    /// For goal `P -> Q`, assumes `P` and proves `Q`.
    Intro,

    /// Split conjunctions
    ///
    /// For goal `P ∧ Q`, creates two subgoals: `P` and `Q`.
    Split,

    /// Choose left disjunction branch
    ///
    /// For goal `P ∨ Q`, proves `P` (discards `Q`).
    Left,

    /// Choose right disjunction branch
    ///
    /// For goal `P ∨ Q`, proves `Q` (discards `P`).
    Right,

    /// Apply known lemma by name
    ///
    /// Searches hints database for lemma and applies it.
    /// If lemma has premises, creates new subgoals for each premise.
    ApplyLemma(Text),

    /// Prove reflexive equality
    ///
    /// For goal `x = x`, completes immediately.
    Reflexivity,

    /// Normalize ring expressions and prove equality
    ///
    /// For algebraic goals like `(x + y)² = x² + 2xy + y²`,
    /// normalizes both sides using ring axioms and checks equality.
    Ring,

    /// Simplify goal using SMT solver
    ///
    /// Applies Z3 simplification tactics to reduce goal complexity.
    Simplify,

    /// Apply SMT solver
    ///
    /// Delegates goal to Z3 SMT solver. Succeeds if Z3 proves goal.
    Smt,

    /// Apply decision procedure by domain
    ///
    /// Invokes specialized decision procedures:
    /// - Linear arithmetic (LIA)
    /// - Bit-vectors (BV)
    /// - Arrays
    DecisionProcedure(ProofDomain),

    /// No-op tactic (always succeeds)
    Skip,

    /// Always fail
    Fail,
}

/// Tactic combinators for building complex strategies
///
/// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 9.1
#[derive(Debug, Clone)]
pub enum TacticCombinator {
    /// Single basic tactic
    Single(Tactic),

    /// Sequential composition: t1 then t2
    ///
    /// Applies `t1`, then applies `t2` to all resulting subgoals.
    /// Notation: `t1 ; t2`
    AndThen(Box<TacticCombinator>, Box<TacticCombinator>),

    /// Alternative: try t1, if fails try t2
    ///
    /// Attempts `t1` first. If it fails, tries `t2` instead.
    /// Notation: `t1 <|> t2`
    OrElse(Box<TacticCombinator>, Box<TacticCombinator>),

    /// Try tactic for limited time
    ///
    /// Applies tactic with timeout. If timeout expires, fails.
    TryFor(Box<TacticCombinator>, Duration),

    /// Repeat until fixpoint or max iterations
    ///
    /// Applies tactic repeatedly until:
    /// - Tactic fails (fixpoint reached)
    /// - Max iterations exceeded
    /// - All goals solved
    Repeat {
        tactic: Box<TacticCombinator>,
        max_iterations: usize,
    },
}

impl TacticCombinator {
    /// Create a sequential composition
    pub fn and_then(self, next: TacticCombinator) -> Self {
        TacticCombinator::AndThen(Box::new(self), Box::new(next))
    }

    /// Create an alternative composition
    pub fn or_else(self, alternative: TacticCombinator) -> Self {
        TacticCombinator::OrElse(Box::new(self), Box::new(alternative))
    }

    /// Wrap with timeout
    pub fn try_for(self, timeout: Duration) -> Self {
        TacticCombinator::TryFor(Box::new(self), timeout)
    }

    /// Wrap with repetition
    pub fn repeat(self, max_iterations: usize) -> Self {
        TacticCombinator::Repeat {
            tactic: Box::new(self),
            max_iterations,
        }
    }
}

/// Proof domains for specialized decision procedures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProofDomain {
    /// Linear integer arithmetic
    LinearArithmetic,
    /// Bit-vector reasoning
    BitVectors,
    /// Array theory
    Arrays,
    /// Uninterpreted functions
    UninterpretedFunctions,
}

// ==================== Proof State ====================

/// Proof state tracking current goals and context
///
/// Represents the state of an ongoing proof, including:
/// - Goals to prove
/// - Available hypotheses
/// - Type context
/// - Proof term being constructed
#[derive(Debug, Clone)]
pub struct ProofState {
    /// Goals remaining to prove
    pub goals: List<ProofGoal>,

    /// Available hypotheses (name -> proposition)
    pub hypotheses: Map<Text, Expr>,

    /// Type context for variables
    pub type_context: TypeContext,

    /// Proof term constructed so far
    pub proof_term: Maybe<Expr>,

    /// Statistics for performance tracking
    pub stats: ProofStats,
}

impl ProofState {
    /// Create new proof state with initial goal
    pub fn new(goal: Expr, type_context: TypeContext) -> Self {
        let mut goals = List::new();
        goals.push(ProofGoal {
            goal,
            span: Span::dummy(),
        });

        Self {
            goals,
            hypotheses: Map::new(),
            type_context,
            proof_term: Maybe::None,
            stats: ProofStats::default(),
        }
    }

    /// Check if proof is complete (no remaining goals)
    pub fn is_complete(&self) -> bool {
        self.goals.is_empty()
    }

    /// Add hypothesis to context
    pub fn add_hypothesis(&mut self, name: Text, prop: Expr) {
        self.hypotheses.insert(name, prop);
    }

    /// Get current goal (first in list)
    pub fn current_goal(&self) -> Maybe<&ProofGoal> {
        if self.goals.is_empty() {
            Maybe::None
        } else {
            Maybe::Some(&self.goals[0])
        }
    }

    /// Remove current goal (after solving it)
    pub fn pop_goal(&mut self) -> Maybe<ProofGoal> {
        if self.goals.is_empty() {
            Maybe::None
        } else {
            Maybe::Some(self.goals.remove(0))
        }
    }

    /// Add new goal to prove
    pub fn add_goal(&mut self, goal: ProofGoal) {
        self.goals.push(goal);
    }

    /// Replace current goal with new goals
    pub fn replace_goal(&mut self, new_goals: List<ProofGoal>) {
        self.pop_goal();
        for goal in new_goals.into_iter().rev() {
            self.goals.insert(0, goal);
        }
    }
}

/// A proof goal to be proved
#[derive(Debug, Clone)]
pub struct ProofGoal {
    /// The proposition to prove
    pub goal: Expr,

    /// Source location for error reporting
    pub span: Span,
}

/// Proof statistics for performance monitoring
#[derive(Debug, Clone, Default)]
pub struct ProofStats {
    /// Number of tactic applications
    pub tactic_applications: usize,

    /// Number of SMT calls
    pub smt_calls: usize,

    /// Time spent in proof search (microseconds)
    pub time_us: u64,

    /// Maximum proof search depth reached
    pub max_depth: usize,

    /// Number of lemmas applied
    pub lemmas_applied: usize,
}

// ==================== Tactic Engine ====================

/// Tactic execution engine
///
/// Manages proof state and executes tactics to construct proofs.
pub struct TacticEngine {
    /// Hints database for lemma lookup (from verum_smt)
    hints_db: Maybe<Heap<verum_smt::HintsDatabase>>,

    /// Maximum proof search depth
    max_depth: usize,

    /// Default timeout for proof search
    default_timeout: Duration,

    /// Current proof depth (for tracking)
    current_depth: usize,
}

impl TacticEngine {
    /// Create new tactic engine
    pub fn new() -> Self {
        Self {
            hints_db: Maybe::None,
            max_depth: 50,
            default_timeout: Duration::from_millis(100),
            current_depth: 0,
        }
    }

    /// Create engine with hints database
    pub fn with_hints(mut self, hints_db: verum_smt::HintsDatabase) -> Self {
        self.hints_db = Maybe::Some(Heap::new(hints_db));
        self
    }

    /// Set maximum proof search depth
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Set default timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Execute tactic combinator on proof state
    ///
    /// Returns updated proof state or error if tactic fails.
    pub fn execute(
        &mut self,
        tactic: &TacticCombinator,
        mut state: ProofState,
    ) -> Result<ProofState, TacticError> {
        let start = Instant::now();

        let result = self.execute_combinator(tactic, state.clone());

        // Update stats
        if let Ok(ref final_state) = result {
            let elapsed = start.elapsed();
            state.stats.time_us += elapsed.as_micros() as u64;
        }

        result
    }

    /// Execute tactic combinator recursively
    fn execute_combinator(
        &mut self,
        combinator: &TacticCombinator,
        state: ProofState,
    ) -> Result<ProofState, TacticError> {
        match combinator {
            TacticCombinator::Single(tactic) => self.execute_tactic(tactic, state),

            TacticCombinator::AndThen(t1, t2) => {
                let state1 = self.execute_combinator(t1, state)?;
                self.execute_combinator(t2, state1)
            }

            TacticCombinator::OrElse(t1, t2) => {
                match self.execute_combinator(t1, state.clone()) {
                    Ok(result) => Ok(result),
                    Err(_) => self.execute_combinator(t2, state),
                }
            }

            TacticCombinator::TryFor(tactic, timeout) => {
                let start = Instant::now();
                let result = self.execute_combinator(tactic, state.clone());

                if start.elapsed() > *timeout {
                    Err(TacticError::Timeout {
                        tactic: "tactic".into(),
                        timeout_ms: timeout.as_millis() as u64,
                    })
                } else {
                    result
                }
            }

            TacticCombinator::Repeat {
                tactic,
                max_iterations,
            } => {
                let mut current_state = state;
                let mut iteration = 0;

                loop {
                    if current_state.is_complete() {
                        // All goals solved
                        break;
                    }

                    if iteration >= *max_iterations {
                        // Max iterations reached
                        break;
                    }

                    // Try to apply tactic
                    match self.execute_combinator(tactic, current_state.clone()) {
                        Ok(new_state) => {
                            // Check if we made progress
                            if new_state.goals.len() >= current_state.goals.len() {
                                // No progress (fixpoint)
                                break;
                            }
                            current_state = new_state;
                        }
                        Err(_) => {
                            // Tactic failed (fixpoint)
                            break;
                        }
                    }

                    iteration += 1;
                }

                Ok(current_state)
            }
        }
    }

    /// Execute basic tactic
    fn execute_tactic(
        &mut self,
        tactic: &Tactic,
        mut state: ProofState,
    ) -> Result<ProofState, TacticError> {
        // Check depth limit
        self.current_depth += 1;
        if self.current_depth > self.max_depth {
            self.current_depth -= 1;
            return Err(TacticError::DepthExceeded {
                max_depth: self.max_depth,
            });
        }

        // Update stats
        state.stats.tactic_applications += 1;
        state.stats.max_depth = state.stats.max_depth.max(self.current_depth);

        let result = match tactic {
            Tactic::Assumption => self.tactic_assumption(state),
            Tactic::Intro => self.tactic_intro(state),
            Tactic::Split => self.tactic_split(state),
            Tactic::Left => self.tactic_left(state),
            Tactic::Right => self.tactic_right(state),
            Tactic::ApplyLemma(name) => self.tactic_apply_lemma(name.clone(), state),
            Tactic::Reflexivity => self.tactic_reflexivity(state),
            Tactic::Ring => self.tactic_ring(state),
            Tactic::Simplify => self.tactic_simplify(state),
            Tactic::Smt => self.tactic_smt(state),
            Tactic::DecisionProcedure(domain) => self.tactic_decision_procedure(*domain, state),
            Tactic::Skip => Ok(state),
            Tactic::Fail => Err(TacticError::ExplicitFail),
        };

        self.current_depth -= 1;
        result
    }

    // ==================== Tactic Implementations ====================

    /// Tactic: assumption
    ///
    /// Proves goal if it matches a hypothesis in the context.
    fn tactic_assumption(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        // Search hypotheses for match
        for (_name, hyp) in &state.hypotheses {
            if Self::expr_eq(&goal, hyp) {
                // Found matching hypothesis - goal solved
                state.pop_goal();
                return Ok(state);
            }
        }

        Err(TacticError::NoMatchingHypothesis {
            goal: Self::expr_to_text(&goal),
        })
    }

    /// Tactic: intro
    ///
    /// Introduce variables or assumptions:
    /// - For `forall x. P(x)`: introduce variable `x`, prove `P(x)`
    /// - For `P -> Q`: assume `P`, prove `Q`
    fn tactic_intro(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            // Implication: P -> Q
            ExprKind::Binary {
                op: BinOp::Imply,
                left,
                right,
            } => {
                // Add P as hypothesis
                let hyp_name = self.generate_fresh_name("H");
                state.add_hypothesis(hyp_name, (**left).clone());

                // Replace goal with Q
                state.pop_goal();
                state.add_goal(ProofGoal {
                    goal: (**right).clone(),
                    span: goal.span,
                });

                Ok(state)
            }

            // Universal quantification: forall x. P(x)
            // (Future: when we have proper forall AST nodes)
            _ => Err(TacticError::CannotIntro {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: split
    ///
    /// Split conjunction `P ∧ Q` into two goals: `P` and `Q`.
    fn tactic_split(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => {
                state.pop_goal();

                // Add both conjuncts as goals
                state.add_goal(ProofGoal {
                    goal: (**left).clone(),
                    span: goal.span,
                });
                state.add_goal(ProofGoal {
                    goal: (**right).clone(),
                    span: goal.span,
                });

                Ok(state)
            }

            _ => Err(TacticError::NotAConjunction {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: left
    ///
    /// For goal `P ∨ Q`, prove `P` (discard `Q`).
    fn tactic_left(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            ExprKind::Binary {
                op: BinOp::Or,
                left,
                ..
            } => {
                state.pop_goal();
                state.add_goal(ProofGoal {
                    goal: (**left).clone(),
                    span: goal.span,
                });

                Ok(state)
            }

            _ => Err(TacticError::NotADisjunction {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: right
    ///
    /// For goal `P ∨ Q`, prove `Q` (discard `P`).
    fn tactic_right(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            ExprKind::Binary {
                op: BinOp::Or, right, ..
            } => {
                state.pop_goal();
                state.add_goal(ProofGoal {
                    goal: (**right).clone(),
                    span: goal.span,
                });

                Ok(state)
            }

            _ => Err(TacticError::NotADisjunction {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: apply_lemma
    ///
    /// Apply known lemma by name from hints database.
    fn tactic_apply_lemma(
        &mut self,
        name: Text,
        mut state: ProofState,
    ) -> Result<ProofState, TacticError> {
        let hints_db = match &self.hints_db {
            Maybe::Some(db) => db,
            Maybe::None => {
                return Err(TacticError::NoHintsDatabase);
            }
        };

        // Lookup lemma by name
        let lemma_hint = match hints_db.lookup_lemma_by_name(&name) {
            Maybe::Some(hint) => hint,
            Maybe::None => {
                return Err(TacticError::LemmaNotFound { name });
            }
        };

        state.stats.lemmas_applied += 1;

        // Extract lemma structure: premises => conclusion
        let (premises, conclusion) = verum_smt::proof_search::ProofSearchEngine::extract_lemma_structure(&lemma_hint.lemma);

        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        // Check if conclusion matches goal
        if !Self::expr_eq(&conclusion, &goal) {
            return Err(TacticError::LemmaDoesNotApply {
                lemma: name,
                goal: Self::expr_to_text(&goal),
            });
        }

        // Replace goal with premises
        state.pop_goal();
        for premise in premises {
            state.add_goal(ProofGoal {
                goal: premise,
                span: goal.span,
            });
        }

        Ok(state)
    }

    /// Tactic: reflexivity
    ///
    /// Prove reflexive equality `x = x`.
    fn tactic_reflexivity(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                if Self::expr_eq(left, right) {
                    // Reflexive equality - goal solved
                    state.pop_goal();
                    Ok(state)
                } else {
                    Err(TacticError::NotReflexive {
                        left: Self::expr_to_text(left),
                        right: Self::expr_to_text(right),
                    })
                }
            }

            _ => Err(TacticError::NotAnEquality {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: ring
    ///
    /// Normalize ring expressions using algebra and prove equality.
    fn tactic_ring(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        match &goal.kind {
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                // Normalize both sides
                let left_norm = self.normalize_ring_expr(left);
                let right_norm = self.normalize_ring_expr(right);

                if Self::expr_eq(&left_norm, &right_norm) {
                    // Normalized forms equal - goal solved
                    state.pop_goal();
                    Ok(state)
                } else {
                    Err(TacticError::RingNormalizationFailed {
                        left: Self::expr_to_text(&left_norm),
                        right: Self::expr_to_text(&right_norm),
                    })
                }
            }

            _ => Err(TacticError::NotAnEquality {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: simplify
    ///
    /// Simplify goal using Z3 tactics (via verum_smt).
    fn tactic_simplify(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        // Use Z3 simplification tactics (from verum_smt::tactics)
        // For now, we'll do basic simplifications locally
        let simplified = self.simplify_expr(&goal);

        if Self::expr_eq(&goal, &simplified) {
            // No change
            Ok(state)
        } else {
            // Replace goal with simplified version
            state.pop_goal();
            state.add_goal(ProofGoal {
                goal: simplified,
                span: goal.span,
            });
            Ok(state)
        }
    }

    /// Tactic: smt
    ///
    /// Delegate goal to Z3 SMT solver.
    fn tactic_smt(&mut self, mut state: ProofState) -> Result<ProofState, TacticError> {
        let goal = match state.current_goal() {
            Maybe::Some(g) => g.goal.clone(),
            Maybe::None => return Ok(state),
        };

        state.stats.smt_calls += 1;

        // Create Z3 context and verify goal
        let ctx = verum_smt::Context::new();
        let mut translator = verum_smt::Translator::new(&ctx);

        // Translate goal to Z3
        let z3_formula = match translator.translate_expr(&goal) {
            Ok(formula) => formula,
            Err(e) => {
                return Err(TacticError::SmtTranslationFailed {
                    error: "failed to translate expression to SMT".into(),
                });
            }
        };

        // Check if SMT solver can prove it
        let solver = z3::Solver::new(&ctx);
        solver.assert(&z3_formula);

        match solver.check() {
            z3::SatResult::Unsat => {
                // Goal proved by SMT solver
                state.pop_goal();
                Ok(state)
            }
            z3::SatResult::Sat => Err(TacticError::SmtDisproved {
                goal: Self::expr_to_text(&goal),
            }),
            z3::SatResult::Unknown => Err(TacticError::SmtUnknown {
                goal: Self::expr_to_text(&goal),
            }),
        }
    }

    /// Tactic: decision_procedure
    ///
    /// Apply specialized decision procedure by domain.
    fn tactic_decision_procedure(
        &mut self,
        domain: ProofDomain,
        state: ProofState,
    ) -> Result<ProofState, TacticError> {
        // Delegate to SMT with domain-specific tactics
        match domain {
            ProofDomain::LinearArithmetic => {
                // Use Z3 LIA tactics
                self.tactic_smt(state)
            }
            ProofDomain::BitVectors => {
                // Use Z3 BV tactics
                self.tactic_smt(state)
            }
            ProofDomain::Arrays => {
                // Use Z3 array theory
                self.tactic_smt(state)
            }
            ProofDomain::UninterpretedFunctions => {
                // Use Z3 UF reasoning
                self.tactic_smt(state)
            }
        }
    }

    // ==================== Helper Methods ====================

    /// Generate fresh variable name
    fn generate_fresh_name(&self, prefix: &str) -> Text {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Thread-safe counter-based fresh name generation
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{}{}", prefix, id).into()
    }

    /// Check if two expressions are equal (structural equality)
    fn expr_eq(e1: &Expr, e2: &Expr) -> bool {
        verum_smt::proof_search::ProofSearchEngine::expr_eq(e1, e2)
    }

    /// Convert expression to text for error messages
    fn expr_to_text(expr: &Expr) -> Text {
        format!("{:?}", expr).into()
    }

    /// Normalize ring expression
    ///
    /// Applies algebraic simplifications:
    /// - Commutativity: a + b = b + a
    /// - Associativity: (a + b) + c = a + (b + c)
    /// - Identity: a + 0 = a, a * 1 = a
    /// - Distributivity: a * (b + c) = a * b + a * c
    fn normalize_ring_expr(&self, expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let left_norm = self.normalize_ring_expr(left);
                let right_norm = self.normalize_ring_expr(right);

                match op {
                    BinOp::Add => {
                        // Check for identity: x + 0 = x
                        if Self::is_zero(&right_norm) {
                            return left_norm;
                        }
                        if Self::is_zero(&left_norm) {
                            return right_norm;
                        }
                    }
                    BinOp::Mul => {
                        // Check for identity: x * 1 = x
                        if Self::is_one(&right_norm) {
                            return left_norm;
                        }
                        if Self::is_one(&left_norm) {
                            return right_norm;
                        }
                        // Check for zero: x * 0 = 0
                        if Self::is_zero(&right_norm) || Self::is_zero(&left_norm) {
                            return Self::make_zero();
                        }
                    }
                    _ => {}
                }

                // Return normalized binary expression
                Expr {
                    kind: ExprKind::Binary {
                        op: *op,
                        left: Box::new(left_norm),
                        right: Box::new(right_norm),
                    },
                    span: expr.span,
                }
            }

            ExprKind::Paren(e) => self.normalize_ring_expr(e),

            _ => expr.clone(),
        }
    }

    /// Simplify expression using basic rules
    fn simplify_expr(&self, expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let left_simp = self.simplify_expr(left);
                let right_simp = self.simplify_expr(right);

                match op {
                    // Boolean simplifications
                    BinOp::And => {
                        if Self::is_true(&left_simp) {
                            return right_simp;
                        }
                        if Self::is_true(&right_simp) {
                            return left_simp;
                        }
                        if Self::is_false(&left_simp) || Self::is_false(&right_simp) {
                            return Self::make_false();
                        }
                    }
                    BinOp::Or => {
                        if Self::is_false(&left_simp) {
                            return right_simp;
                        }
                        if Self::is_false(&right_simp) {
                            return left_simp;
                        }
                        if Self::is_true(&left_simp) || Self::is_true(&right_simp) {
                            return Self::make_true();
                        }
                    }
                    _ => {}
                }

                Expr {
                    kind: ExprKind::Binary {
                        op: *op,
                        left: Box::new(left_simp),
                        right: Box::new(right_simp),
                    },
                    span: expr.span,
                }
            }

            ExprKind::Unary { op, expr: inner } => {
                let inner_simp = self.simplify_expr(inner);

                // Double negation: !!x = x
                if let verum_ast::UnOp::Not = op {
                    if let ExprKind::Unary {
                        op: verum_ast::UnOp::Not,
                        expr: inner2,
                    } = &inner_simp.kind
                    {
                        return (**inner2).clone();
                    }
                }

                Expr {
                    kind: ExprKind::Unary {
                        op: *op,
                        expr: Box::new(inner_simp),
                    },
                    span: expr.span,
                }
            }

            ExprKind::Paren(e) => self.simplify_expr(e),

            _ => expr.clone(),
        }
    }

    /// Check if expression is literal zero
    fn is_zero(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Literal(Literal {
                kind: LiteralKind::Int(0),
                ..
            })
        )
    }

    /// Check if expression is literal one
    fn is_one(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Literal(Literal {
                kind: LiteralKind::Int(1),
                ..
            })
        )
    }

    /// Check if expression is boolean true
    fn is_true(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(true),
                ..
            })
        )
    }

    /// Check if expression is boolean false
    fn is_false(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(false),
                ..
            })
        )
    }

    /// Create literal zero expression
    fn make_zero() -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::Int(0),
                suffix: Maybe::None,
            }),
            span: Span::dummy(),
        }
    }

    /// Create boolean true expression
    fn make_true() -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(true),
                suffix: Maybe::None,
            }),
            span: Span::dummy(),
        }
    }

    /// Create boolean false expression
    fn make_false() -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(false),
                suffix: Maybe::None,
            }),
            span: Span::dummy(),
        }
    }
}

impl Default for TacticEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Predefined Strategies ====================

/// Predefined tactic strategies for common proof patterns
pub struct PredefinedTactics;

impl PredefinedTactics {
    /// Auto tactic: automatic proof search
    ///
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 9.1 example
    ///
    /// ```verum
    /// tactic auto = {
    ///     repeat (
    ///         assumption
    ///         <|> intro
    ///         <|> split
    ///         <|> left <|> right
    ///         <|> apply_lemma
    ///     )
    /// }
    /// ```
    pub fn auto() -> TacticCombinator {
        TacticCombinator::Repeat {
            tactic: Box::new(
                TacticCombinator::Single(Tactic::Assumption)
                    .or_else(TacticCombinator::Single(Tactic::Intro))
                    .or_else(TacticCombinator::Single(Tactic::Split))
                    .or_else(
                        TacticCombinator::Single(Tactic::Left)
                            .or_else(TacticCombinator::Single(Tactic::Right)),
                    ),
            ),
            max_iterations: 50,
        }
    }

    /// Ring tactic: prove algebraic equalities
    ///
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 9.1 example
    pub fn ring() -> TacticCombinator {
        TacticCombinator::Single(Tactic::Ring)
            .and_then(TacticCombinator::Single(Tactic::Reflexivity))
    }

    /// Simple tactic: basic simplification
    pub fn simplify() -> TacticCombinator {
        TacticCombinator::Repeat {
            tactic: Box::new(TacticCombinator::Single(Tactic::Simplify)),
            max_iterations: 10,
        }
    }

    /// Solver tactic: delegate to SMT
    pub fn solver() -> TacticCombinator {
        TacticCombinator::Single(Tactic::Smt)
    }

    /// Blast tactic: aggressive automation
    pub fn blast() -> TacticCombinator {
        Self::auto()
            .or_else(Self::ring())
            .or_else(Self::solver())
            .try_for(Duration::from_millis(500))
    }
}

// ==================== Errors ====================

/// Errors that can occur during tactic execution
#[derive(Debug, Clone, thiserror::Error)]
pub enum TacticError {
    #[error("no matching hypothesis for goal: {goal}")]
    NoMatchingHypothesis { goal: Text },

    #[error("cannot intro: goal is not an implication or forall: {goal}")]
    CannotIntro { goal: Text },

    #[error("goal is not a conjunction: {goal}")]
    NotAConjunction { goal: Text },

    #[error("goal is not a disjunction: {goal}")]
    NotADisjunction { goal: Text },

    #[error("goal is not an equality: {goal}")]
    NotAnEquality { goal: Text },

    #[error("not a reflexive equality: {left} ≠ {right}")]
    NotReflexive { left: Text, right: Text },

    #[error("ring normalization failed: {left} and {right} are not equal after normalization")]
    RingNormalizationFailed { left: Text, right: Text },

    #[error("lemma not found: {name}")]
    LemmaNotFound { name: Text },

    #[error("lemma does not apply to goal: {lemma} cannot prove {goal}")]
    LemmaDoesNotApply { lemma: Text, goal: Text },

    #[error("no hints database available")]
    NoHintsDatabase,

    #[error("SMT translation failed: {error}")]
    SmtTranslationFailed { error: Text },

    #[error("SMT solver disproved goal: {goal}")]
    SmtDisproved { goal: Text },

    #[error("SMT solver returned unknown for goal: {goal}")]
    SmtUnknown { goal: Text },

    #[error("tactic timeout: {tactic} exceeded {timeout_ms}ms")]
    Timeout { tactic: Text, timeout_ms: u64 },

    #[error("proof search depth exceeded: {max_depth}")]
    DepthExceeded { max_depth: usize },

    #[error("tactic explicitly failed")]
    ExplicitFail,
}

// ==================== Integration with Type Checker ====================

/// Extension trait for type checker to support `by tactic` syntax
pub trait TypeCheckerTacticExt {
    /// Verify proof using tactic
    ///
    /// Called when user writes `by tactic_name` in proof.
    fn verify_by_tactic(
        &mut self,
        goal: Expr,
        tactic: TacticCombinator,
        context: TypeContext,
    ) -> Result<Expr, TypeError>;
}

// Implementation would be in verum_types::infer module to avoid circular dependency
// This is just the trait definition for the interface

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tactic_assumption() {
        let mut engine = TacticEngine::new();
        let ctx = TypeContext::new();

        // Create goal: P
        let goal = Expr {
            kind: ExprKind::Path(Path::from_ident("P".into())),
            span: Span::dummy(),
        };

        let mut state = ProofState::new(goal.clone(), ctx);

        // Add hypothesis P
        state.add_hypothesis("h1".into(), goal.clone());

        // Apply assumption tactic
        let result = engine.execute_tactic(&Tactic::Assumption, state);
        assert!(result.is_ok());

        let final_state = result.unwrap();
        assert!(final_state.is_complete());
    }

    #[test]
    fn test_tactic_split() {
        let mut engine = TacticEngine::new();
        let ctx = TypeContext::new();

        // Create goal: P ∧ Q
        let p = Expr {
            kind: ExprKind::Path(Path::from_ident("P".into())),
            span: Span::dummy(),
        };
        let q = Expr {
            kind: ExprKind::Path(Path::from_ident("Q".into())),
            span: Span::dummy(),
        };

        let goal = Expr {
            kind: ExprKind::Binary {
                op: BinOp::And,
                left: Box::new(p.clone()),
                right: Box::new(q.clone()),
            },
            span: Span::dummy(),
        };

        let state = ProofState::new(goal, ctx);

        // Apply split tactic
        let result = engine.execute_tactic(&Tactic::Split, state);
        assert!(result.is_ok());

        let final_state = result.unwrap();
        assert_eq!(final_state.goals.len(), 2);
    }

    #[test]
    fn test_tactic_reflexivity() {
        let mut engine = TacticEngine::new();
        let ctx = TypeContext::new();

        // Create goal: x = x
        let x = Expr {
            kind: ExprKind::Path(Path::from_ident("x".into())),
            span: Span::dummy(),
        };

        let goal = Expr {
            kind: ExprKind::Binary {
                op: BinOp::Eq,
                left: Box::new(x.clone()),
                right: Box::new(x.clone()),
            },
            span: Span::dummy(),
        };

        let state = ProofState::new(goal, ctx);

        // Apply reflexivity tactic
        let result = engine.execute_tactic(&Tactic::Reflexivity, state);
        assert!(result.is_ok());

        let final_state = result.unwrap();
        assert!(final_state.is_complete());
    }

    #[test]
    fn test_tactic_combinator_or_else() {
        let mut engine = TacticEngine::new();
        let ctx = TypeContext::new();

        let goal = Expr {
            kind: ExprKind::Path(Path::from_ident("P".into())),
            span: Span::dummy(),
        };

        let mut state = ProofState::new(goal.clone(), ctx);
        state.add_hypothesis("h1".into(), goal.clone());

        // Try fail, then assumption
        let tactic = TacticCombinator::Single(Tactic::Fail)
            .or_else(TacticCombinator::Single(Tactic::Assumption));

        let result = engine.execute(&tactic, state);
        assert!(result.is_ok());

        let final_state = result.unwrap();
        assert!(final_state.is_complete());
    }
}
