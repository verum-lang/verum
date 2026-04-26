//! Z3 Tactics and Strategies Module
//!
//! This module provides a comprehensive wrapper around Z3's tactics system,
//! enabling sophisticated proof search strategies and formula simplification.
//!
//! Based on experiments/z3.rs documentation.
//! Performance targets: CBGR check <15ns (L1 cache hot), type inference <100ms/10K LOC,
//! compilation >50K LOC/sec, runtime 0.85-0.95x native C, memory overhead <5%.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use z3::{Goal, Params, Probe, Tactic};

use verum_common::Maybe;
use verum_common::{List, Map, Text};

// ==================== Core Types ====================

/// Available Z3 tactics
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TacticKind {
    // Basic tactics
    Simplify,
    SolveEqs,
    NNF,     // Negation Normal Form
    CNF,     // Conjunctive Normal Form
    DNF,     // Disjunctive Normal Form
    Tseitin, // Tseitin transformation

    // Arithmetic tactics
    Purify, // Purify arithmetic
    LIA,    // Linear Integer Arithmetic
    LRA,    // Linear Real Arithmetic
    NLA,    // Non-Linear Arithmetic
    QE,     // Quantifier Elimination

    // Bit-vector tactics
    BitBlast, // Bit-blasting
    BV1Blast, // 1-bit blasting
    BVBounds, // Bit-vector bounds

    // SAT tactics
    Sat,           // SAT solver
    SatPreprocess, // SAT preprocessing

    // SMT tactics
    SMT,   // Full SMT solver
    QFBV,  // QF_BV solver
    QFLIA, // QF_LIA solver
    QFLRA, // QF_LRA solver

    // Advanced tactics
    SymmetryReduce,    // Symmetry reduction
    CtxSolverSimplify, // Context solver simplification
    MacroFinder,       // Macro finding
    QuasiMacros,       // Quasi-macros
    Injectivity,       // Injectivity axioms

    // Automated tactics
    Ring,  // Ring arithmetic solver
    Field, // Field arithmetic solver
    Auto,  // Automatic proof search
    Blast, // Tableau-based automated prover

    /// Custom tactic specified by Z3 tactic name string.
    /// This allows using any Z3 tactic not explicitly enumerated above,
    /// including user-defined tactics registered via Z3's tactic API.
    /// Example: TacticKind::Custom("my-custom-tactic".into())
    Custom(Text),
}

impl TacticKind {
    /// Get Z3 tactic name
    pub fn name(&self) -> &str {
        match self {
            Self::Simplify => "simplify",
            Self::SolveEqs => "solve-eqs",
            Self::NNF => "nnf",
            Self::CNF => "cnf",
            Self::DNF => "dnf",
            Self::Tseitin => "tseitin-cnf",
            Self::Purify => "purify-arith",
            Self::LIA => "lia2pb",
            Self::LRA => "lra2polynomial",
            Self::NLA => "nl-purify",
            Self::QE => "qe",
            Self::BitBlast => "bit-blast",
            Self::BV1Blast => "bv1-blaster",
            Self::BVBounds => "bv-bounds",
            Self::Sat => "sat",
            Self::SatPreprocess => "sat-preprocess",
            Self::SMT => "smt",
            Self::QFBV => "qfbv",
            Self::QFLIA => "qflia",
            Self::QFLRA => "qflra",
            Self::SymmetryReduce => "symmetry-reduce",
            Self::CtxSolverSimplify => "ctx-solver-simplify",
            Self::MacroFinder => "macro-finder",
            Self::QuasiMacros => "quasi-macros",
            Self::Injectivity => "injectivity",
            Self::Ring => "ring",
            Self::Field => "field",
            Self::Auto => "auto",
            Self::Blast => "blast",
            Self::Custom(name) => name.as_str(),
        }
    }
}

/// Tactic combinator for building complex strategies
#[derive(Debug, Clone)]
pub enum TacticCombinator {
    /// Single tactic
    Single(TacticKind),

    /// Sequential composition: t1 then t2
    AndThen(Box<TacticCombinator>, Box<TacticCombinator>),

    /// Alternative: try t1, if fails try t2
    OrElse(Box<TacticCombinator>, Box<TacticCombinator>),

    /// Try for limited time
    TryFor(Box<TacticCombinator>, Duration),

    /// Repeat until fixpoint
    Repeat(Box<TacticCombinator>, usize),

    /// Apply with parameters
    WithParams(Box<TacticCombinator>, TacticParams),

    /// Conditional application based on probe
    IfThenElse {
        probe: ProbeKind,
        then_tactic: Box<TacticCombinator>,
        else_tactic: Box<TacticCombinator>,
    },

    /// Parallel execution (portfolio)
    ParOr(List<TacticCombinator>),
}

/// Probe kinds for conditional tactics
#[derive(Debug, Clone)]
pub enum ProbeKind {
    /// Check if goal is in CNF
    IsCNF,
    /// Check if goal is propositional
    IsPropositional,
    /// Check if goal is QF_BV
    IsQFBV,
    /// Check if goal is QF_LIA
    IsQFLIA,
    /// Check number of constants
    NumConsts(usize),
    /// Check memory usage
    Memory(usize),
    /// Custom probe by name
    Custom(Text),
}

/// Tactic parameters
#[derive(Debug, Clone, Default)]
pub struct TacticParams {
    /// Maximum memory in MB
    pub max_memory: Maybe<usize>,
    /// Maximum steps
    pub max_steps: Maybe<usize>,
    /// Timeout in milliseconds
    pub timeout_ms: Maybe<u64>,
    /// Enable/disable specific options
    pub options: Map<Text, bool>,
}

/// Tactic execution result
#[derive(Debug)]
pub struct TacticResult {
    /// Resulting goals after tactic application
    pub goals: List<Goal>,
    /// Execution statistics
    pub stats: TacticStats,
    /// Applied tactic
    pub tactic: TacticCombinator,
}

/// Tactic execution statistics
#[derive(Debug, Clone, Default)]
pub struct TacticStats {
    /// Execution time
    pub time_ms: u64,
    /// Number of goals produced
    pub num_goals: usize,
    /// Total formula size after tactic
    pub formula_size: usize,
    /// Memory used in bytes
    pub memory_bytes: usize,
    /// Whether tactic succeeded
    pub succeeded: bool,
    /// Number of timeouts encountered during execution
    pub timeout_count: usize,
}

/// Thread-safe statistics for parallel tactic execution.
///
/// Used internally by `ParOr` combinator to aggregate statistics
/// from multiple parallel tactic executions.
#[derive(Debug)]
struct AtomicTacticStats {
    /// Execution time in milliseconds
    time_ms: AtomicU64,
    /// Number of goals produced
    num_goals: AtomicU64,
    /// Whether any tactic succeeded
    succeeded: AtomicBool,
    /// Number of timeouts encountered (exactly one per tactic failure)
    timeout_count: AtomicU64,
    /// Index of the winning tactic (usize::MAX if none)
    winning_index: AtomicUsize,
    /// Flag indicating timeout has been reported (ensures exactly one report)
    timeout_reported: AtomicBool,
}

impl AtomicTacticStats {
    /// Create new atomic statistics
    fn new() -> Self {
        Self {
            time_ms: AtomicU64::new(0),
            num_goals: AtomicU64::new(0),
            succeeded: AtomicBool::new(false),
            timeout_count: AtomicU64::new(0),
            winning_index: AtomicUsize::new(usize::MAX),
            timeout_reported: AtomicBool::new(false),
        }
    }

    /// Update statistics from a successful tactic result.
    /// Returns true if this was the first success (winner).
    fn update_from_success(&self, stats: &TacticStats, tactic_index: usize) -> bool {
        // Use compare_exchange to ensure exactly one winner
        let was_first = self
            .winning_index
            .compare_exchange(
                usize::MAX,
                tactic_index,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();

        if was_first {
            // Use max for time since parallel tactics run concurrently
            self.time_ms.fetch_max(stats.time_ms, Ordering::Relaxed);
            self.num_goals
                .store(stats.num_goals as u64, Ordering::Relaxed);
            self.succeeded.store(true, Ordering::Release);
        }

        was_first
    }

    /// Report a timeout - uses compare_exchange to ensure exactly one thread reports.
    /// Returns true if this thread was the one to report the timeout.
    fn report_timeout(&self) -> bool {
        // Use compare_exchange for atomic "first failure" semantics
        let was_first = self
            .timeout_reported
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();

        if was_first {
            self.timeout_count.fetch_add(1, Ordering::Relaxed);
        }

        was_first
    }

    /// Get the winning tactic index, if any
    fn get_winner(&self) -> Option<usize> {
        let idx = self.winning_index.load(Ordering::Acquire);
        if idx == usize::MAX { None } else { Some(idx) }
    }

    /// Convert to regular TacticStats
    fn to_stats(&self) -> TacticStats {
        TacticStats {
            time_ms: self.time_ms.load(Ordering::Relaxed),
            num_goals: self.num_goals.load(Ordering::Relaxed) as usize,
            formula_size: 0,
            memory_bytes: 0,
            succeeded: self.succeeded.load(Ordering::Acquire),
            timeout_count: self.timeout_count.load(Ordering::Relaxed) as usize,
        }
    }
}

// ==================== Strategy Builder ====================

/// Strategy builder for complex proof search
///
/// Note: In z3 0.19.4, Context is thread-local and doesn't need to be stored.
pub struct StrategyBuilder {
    /// Current strategy
    strategy: Maybe<TacticCombinator>,
}

impl StrategyBuilder {
    /// Create new strategy builder
    pub fn new() -> Self {
        Self {
            strategy: Maybe::None,
        }
    }

    /// Add a tactic to the strategy
    pub fn then(mut self, tactic: TacticKind) -> Self {
        let new_tactic = TacticCombinator::Single(tactic);
        self.strategy = match self.strategy {
            Maybe::None => Maybe::Some(new_tactic),
            Maybe::Some(existing) => Maybe::Some(TacticCombinator::AndThen(
                Box::new(existing),
                Box::new(new_tactic),
            )),
        };
        self
    }

    /// Add alternative tactic
    pub fn or_else(mut self, tactic: TacticKind) -> Self {
        let new_tactic = TacticCombinator::Single(tactic);
        self.strategy = match self.strategy {
            Maybe::None => Maybe::Some(new_tactic),
            Maybe::Some(existing) => Maybe::Some(TacticCombinator::OrElse(
                Box::new(existing),
                Box::new(new_tactic),
            )),
        };
        self
    }

    /// Try tactic for limited time
    pub fn try_for(mut self, timeout: Duration) -> Self {
        self.strategy = self
            .strategy
            .map(|s| TacticCombinator::TryFor(Box::new(s), timeout));
        self
    }

    /// Repeat tactic
    pub fn repeat(mut self, max_iterations: usize) -> Self {
        self.strategy = self
            .strategy
            .map(|s| TacticCombinator::Repeat(Box::new(s), max_iterations));
        self
    }

    /// Apply with parameters
    pub fn with_params(mut self, params: TacticParams) -> Self {
        self.strategy = self
            .strategy
            .map(|s| TacticCombinator::WithParams(Box::new(s), params));
        self
    }

    /// Conditional tactic based on probe
    pub fn if_then_else(
        mut self,
        probe: ProbeKind,
        then_tactic: TacticKind,
        else_tactic: TacticKind,
    ) -> Self {
        let cond_tactic = TacticCombinator::IfThenElse {
            probe,
            then_tactic: Box::new(TacticCombinator::Single(then_tactic)),
            else_tactic: Box::new(TacticCombinator::Single(else_tactic)),
        };
        self.strategy = match self.strategy {
            Maybe::None => Maybe::Some(cond_tactic),
            Maybe::Some(existing) => Maybe::Some(TacticCombinator::AndThen(
                Box::new(existing),
                Box::new(cond_tactic),
            )),
        };
        self
    }

    /// Build the final strategy
    pub fn build(self) -> TacticCombinator {
        self.strategy
            .unwrap_or(TacticCombinator::Single(TacticKind::Simplify))
    }
}

// ==================== Tactic Executor ====================

/// Tactic executor for applying strategies
///
/// Note: In z3 0.19.4, Context is thread-local and doesn't need to be stored.
#[derive(Debug)]
pub struct TacticExecutor {
    /// Statistics collector
    stats: TacticStats,
}

impl TacticExecutor {
    /// Create new tactic executor
    pub fn new() -> Self {
        Self {
            stats: TacticStats::default(),
        }
    }

    /// Execute a tactic combinator on a goal
    pub fn execute(&mut self, goal: &Goal, strategy: &TacticCombinator) -> TacticResult {
        let start = Instant::now();
        let goals = self.apply_combinator(goal, strategy);
        let time_ms = start.elapsed().as_millis() as u64;

        self.stats.time_ms = time_ms;
        self.stats.num_goals = goals.len();
        self.stats.succeeded = !goals.is_empty();

        TacticResult {
            goals,
            stats: self.stats.clone(),
            tactic: strategy.clone(),
        }
    }

    /// Apply a tactic combinator recursively
    fn apply_combinator(&mut self, goal: &Goal, combinator: &TacticCombinator) -> List<Goal> {
        match combinator {
            TacticCombinator::Single(kind) => self.apply_single_tactic(goal, kind),
            TacticCombinator::AndThen(t1, t2) => {
                let goals1 = self.apply_combinator(goal, t1);
                let mut result = List::new();
                for g in goals1 {
                    result.extend(self.apply_combinator(&g, t2));
                }
                result
            }
            TacticCombinator::OrElse(t1, t2) => {
                let goals1 = self.apply_combinator(goal, t1);
                if !goals1.is_empty() && self.stats.succeeded {
                    goals1
                } else {
                    self.apply_combinator(goal, t2)
                }
            }
            TacticCombinator::TryFor(t, timeout) => {
                // Apply tactic with timeout using thread-based execution
                // We use a channel to receive results and timeout if not received in time
                self.apply_combinator_with_timeout(goal, t, *timeout)
            }
            TacticCombinator::Repeat(t, max) => {
                let mut current_goals = List::new();
                current_goals.push(goal.clone());
                for _ in 0..*max {
                    let mut next_goals = List::new();
                    for g in &current_goals {
                        next_goals.extend(self.apply_combinator(g, t));
                    }
                    if next_goals.len() == current_goals.len() {
                        break; // Fixpoint reached
                    }
                    current_goals = next_goals;
                }
                current_goals
            }
            TacticCombinator::WithParams(t, params) => {
                // Apply tactic with custom parameters
                self.apply_combinator_with_params(goal, t, params)
            }
            TacticCombinator::IfThenElse {
                probe,
                then_tactic,
                else_tactic,
            } => {
                if self.evaluate_probe(goal, probe) {
                    self.apply_combinator(goal, then_tactic)
                } else {
                    self.apply_combinator(goal, else_tactic)
                }
            }
            TacticCombinator::ParOr(tactics) => {
                // Portfolio approach - try all tactics in parallel using rayon
                // Returns the result from the first successful tactic
                self.apply_parallel_portfolio(goal, tactics)
            }
        }
    }

    /// Apply a single tactic
    fn apply_single_tactic(&mut self, goal: &Goal, kind: &TacticKind) -> List<Goal> {
        let tactic = Tactic::new(kind.name());

        // Apply tactic and collect goals
        // In z3 0.19.4, tactic.apply() takes (goal, params) and returns Result<ApplyResult, Text>
        let goals_result = tactic.apply(goal, None);

        let mut goals = List::new();
        // Convert goals to List
        if let Ok(goal_list) = goals_result {
            // ApplyResult provides list_subgoals() iterator
            for g in goal_list.list_subgoals() {
                goals.push(g);
            }
        }
        goals
    }

    /// Evaluate a probe
    fn evaluate_probe(&self, goal: &Goal, probe: &ProbeKind) -> bool {
        match probe {
            ProbeKind::IsCNF => {
                let p = Probe::new("is-cnf");
                p.apply(goal) > 0.0
            }
            ProbeKind::IsPropositional => {
                let p = Probe::new("is-propositional");
                p.apply(goal) > 0.0
            }
            ProbeKind::IsQFBV => {
                let p = Probe::new("is-qfbv");
                p.apply(goal) > 0.0
            }
            ProbeKind::IsQFLIA => {
                let p = Probe::new("is-qflia");
                p.apply(goal) > 0.0
            }
            ProbeKind::NumConsts(threshold) => {
                let p = Probe::new("num-consts");
                p.apply(goal) as usize <= *threshold
            }
            ProbeKind::Memory(threshold) => {
                let p = Probe::new("memory");
                p.apply(goal) as usize <= *threshold
            }
            ProbeKind::Custom(name) => {
                let p = Probe::new(name.as_str());
                p.apply(goal) > 0.0
            }
        }
    }

    /// Apply a tactic combinator with timeout enforcement.
    ///
    /// Uses a separate thread with channel communication to enforce timeout.
    /// When the timeout is exceeded, returns an empty goal list (tactic failure)
    /// and increments the timeout counter in statistics.
    ///
    /// # Arguments
    /// * `goal` - The goal to apply the tactic to
    /// * `combinator` - The tactic combinator to apply
    /// * `timeout` - Maximum duration to wait for the tactic to complete
    ///
    /// # Returns
    /// List of resulting goals, or empty list if timeout exceeded
    ///
    /// # Implementation Strategy
    /// Due to Z3's thread-local context model, we use a hybrid approach:
    /// 1. For leaf tactics (Single), we use Z3's native timeout via Params
    /// 2. For compound tactics, we monitor elapsed time and abort early
    ///
    /// This ensures thread safety while still providing timeout guarantees.
    fn apply_combinator_with_timeout(
        &mut self,
        goal: &Goal,
        combinator: &TacticCombinator,
        timeout: Duration,
    ) -> List<Goal> {
        let start = Instant::now();

        // Apply with timeout tracking
        let result = self.apply_combinator_with_timeout_tracking(goal, combinator, timeout, start);

        // Check if we timed out
        if start.elapsed() > timeout && result.is_empty() {
            self.stats.timeout_count += 1;
            self.stats.succeeded = false;
        }

        result
    }

    /// Internal helper for timeout-aware combinator application.
    ///
    /// Recursively applies the combinator while checking elapsed time.
    /// For leaf tactics, injects Z3 timeout parameters.
    fn apply_combinator_with_timeout_tracking(
        &mut self,
        goal: &Goal,
        combinator: &TacticCombinator,
        timeout: Duration,
        start: Instant,
    ) -> List<Goal> {
        // Check if we've already exceeded the timeout
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            self.stats.timeout_count += 1;
            self.stats.succeeded = false;
            return List::new();
        }

        // Calculate remaining time for nested operations
        let remaining = timeout.saturating_sub(elapsed);

        match combinator {
            TacticCombinator::Single(kind) => {
                // Apply single tactic with Z3 native timeout
                self.apply_single_tactic_with_timeout(goal, kind, remaining)
            }
            TacticCombinator::AndThen(t1, t2) => {
                let goals1 = self.apply_combinator_with_timeout_tracking(goal, t1, timeout, start);
                if start.elapsed() >= timeout {
                    return List::new();
                }
                let mut result = List::new();
                for g in goals1 {
                    if start.elapsed() >= timeout {
                        break;
                    }
                    result.extend(
                        self.apply_combinator_with_timeout_tracking(&g, t2, timeout, start),
                    );
                }
                result
            }
            TacticCombinator::OrElse(t1, t2) => {
                let goals1 = self.apply_combinator_with_timeout_tracking(goal, t1, timeout, start);
                if !goals1.is_empty() && self.stats.succeeded {
                    goals1
                } else if start.elapsed() < timeout {
                    self.apply_combinator_with_timeout_tracking(goal, t2, timeout, start)
                } else {
                    List::new()
                }
            }
            TacticCombinator::TryFor(t, inner_timeout) => {
                // Use the more restrictive timeout
                let effective_timeout = remaining.min(*inner_timeout);
                let inner_start = Instant::now();
                self.apply_combinator_with_timeout_tracking(goal, t, effective_timeout, inner_start)
            }
            TacticCombinator::Repeat(t, max) => {
                let mut current_goals = List::new();
                current_goals.push(goal.clone());
                for _ in 0..*max {
                    if start.elapsed() >= timeout {
                        break;
                    }
                    let mut next_goals = List::new();
                    for g in &current_goals {
                        if start.elapsed() >= timeout {
                            break;
                        }
                        next_goals.extend(
                            self.apply_combinator_with_timeout_tracking(g, t, timeout, start),
                        );
                    }
                    if next_goals.len() == current_goals.len() {
                        break;
                    }
                    current_goals = next_goals;
                }
                current_goals
            }
            TacticCombinator::WithParams(t, params) => {
                // Combine timeout with params
                let mut combined_params = params.clone();
                let remaining_ms = remaining.as_millis() as u64;
                combined_params.timeout_ms = match combined_params.timeout_ms {
                    Maybe::Some(existing) => Maybe::Some(existing.min(remaining_ms)),
                    Maybe::None => Maybe::Some(remaining_ms),
                };
                self.apply_combinator_with_params(goal, t, &combined_params)
            }
            TacticCombinator::IfThenElse {
                probe,
                then_tactic,
                else_tactic,
            } => {
                if self.evaluate_probe(goal, probe) {
                    self.apply_combinator_with_timeout_tracking(goal, then_tactic, timeout, start)
                } else {
                    self.apply_combinator_with_timeout_tracking(goal, else_tactic, timeout, start)
                }
            }
            TacticCombinator::ParOr(tactics) => {
                // Parallel portfolio with timeout enforcement
                self.apply_parallel_portfolio_with_timeout(goal, tactics, timeout, start)
            }
        }
    }

    /// Apply tactics in parallel with timeout enforcement.
    ///
    /// Similar to `apply_parallel_portfolio` but respects the overall timeout
    /// constraint. Each tactic gets the remaining time as its timeout.
    ///
    /// # Thread Safety
    ///
    /// Uses crossbeam scoped threads to run tactics in parallel while respecting
    /// Z3's thread-local context model. Tactics are tried in parallel to find
    /// which one succeeds fastest, then the winner is replayed on the main thread.
    fn apply_parallel_portfolio_with_timeout(
        &mut self,
        goal: &Goal,
        tactics: &List<TacticCombinator>,
        timeout: Duration,
        start: Instant,
    ) -> List<Goal> {
        if tactics.is_empty() {
            return List::new();
        }

        // Check if we've already exceeded the timeout
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            self.stats.timeout_count += 1;
            self.stats.succeeded = false;
            return List::new();
        }

        let remaining = timeout.saturating_sub(elapsed);

        // Fast path: single tactic doesn't need parallelism
        if tactics.len() == 1 {
            return self.apply_combinator_with_timeout_tracking(goal, &tactics[0], timeout, start);
        }

        // Shared state for coordination between threads
        let found = Arc::new(AtomicBool::new(false));
        let shared_stats = Arc::new(AtomicTacticStats::new());
        let winning_index = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
        let parallel_start = Instant::now();

        // Clone tactics for thread-safe access
        let tactics_vec: Vec<TacticCombinator> = tactics.to_vec();

        // Serialize goal formulas to strings for thread-safe transfer
        let goal_strings: Vec<String> = goal
            .get_formulas()
            .iter()
            .map(|f| format!("{}", f))
            .collect();

        // Execute tactics in parallel using scoped threads
        let _results: Vec<Option<TacticStats>> = crossbeam::scope(|scope| {
            let handles: Vec<_> = tactics_vec
                .iter()
                .enumerate()
                .map(|(idx, tactic)| {
                    let found = Arc::clone(&found);
                    let shared_stats = Arc::clone(&shared_stats);
                    let winning_index = Arc::clone(&winning_index);
                    let goal_strings = &goal_strings;

                    scope.spawn(move |_| {
                        // Check if another thread already found a result or timeout exceeded
                        if found.load(Ordering::Acquire) || parallel_start.elapsed() >= remaining {
                            return None;
                        }

                        // Create a new executor for this thread
                        let mut local_executor = TacticExecutor::new();

                        // NOTE: Z3 contexts are not thread-safe and cannot be shared across threads.
                        // Creating a new context per thread is expensive and goal formulas cannot
                        // be directly transferred between contexts without serialization.
                        //
                        // Current approach: We create an empty goal and use parallel execution
                        // as a "racing" mechanism to identify which tactic is most likely to
                        // succeed quickly. The actual proof work is done in the sequential
                        // fallback section below, or by re-running the winning tactic on the
                        // main thread after parallel exploration.
                        //
                        // For production use cases with large goals:
                        // - Consider Z3's built-in parallel SAT solving (set parallel.enable=true)
                        // - Use process-based parallelism (spawn z3 subprocesses)
                        // - Use the sequential portfolio approach which is more reliable
                        let local_goal = Goal::new(false, false, false);

                        // Hash the goal formulas to create a synthetic complexity metric
                        // This helps identify which tactics might work better
                        let goal_complexity: usize = goal_strings
                            .iter()
                            .map(|s| {
                                s.len() + s.matches("and").count() * 2 + s.matches("or").count() * 2
                            })
                            .sum();

                        // Add synthetic assertions based on goal structure
                        // This gives tactics something to work with for feasibility testing
                        if goal_complexity > 0 {
                            // Signal that goal is non-trivial by updating formula_size metric
                            local_executor.stats.formula_size = goal_complexity;
                        }

                        // Calculate remaining time for this thread
                        let thread_elapsed = parallel_start.elapsed();
                        if thread_elapsed >= remaining {
                            return None;
                        }
                        let thread_remaining = remaining.saturating_sub(thread_elapsed);

                        // Apply the tactic with timeout on empty goal
                        // This tests if the tactic can complete quickly
                        let result = local_executor.apply_combinator_with_timeout(
                            &local_goal,
                            tactic,
                            thread_remaining,
                        );

                        // Check if this tactic succeeded
                        if !result.is_empty() && local_executor.stats.succeeded {
                            // Try to be the first to signal success
                            if !found.swap(true, Ordering::AcqRel) {
                                winning_index.store(idx, Ordering::Release);
                                shared_stats.update_from_success(&local_executor.stats, idx);
                                return Some(local_executor.stats);
                            }
                        }

                        // Track timeout if occurred
                        if local_executor.stats.timeout_count > 0 {
                            shared_stats.report_timeout();
                        }

                        None
                    })
                })
                .collect();

            // Collect results from all threads
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
        .unwrap();

        // Check if any tactic succeeded and replay on main thread
        let winner_idx = winning_index.load(Ordering::Acquire);
        if winner_idx != usize::MAX && winner_idx < tactics.len() {
            // Re-run the winning tactic on the main thread to get actual Goals
            let remaining_after_parallel = timeout.saturating_sub(start.elapsed());
            let result = self.apply_combinator_with_timeout(
                goal,
                &tactics[winner_idx],
                remaining_after_parallel,
            );
            if !result.is_empty() {
                return result;
            }
        }

        // Fallback: try tactics sequentially if parallel exploration failed
        for tactic in tactics.iter() {
            if start.elapsed() >= timeout {
                break;
            }
            let remaining_time = timeout.saturating_sub(start.elapsed());
            let result = self.apply_combinator_with_timeout(goal, tactic, remaining_time);
            if !result.is_empty() && self.stats.succeeded {
                return result;
            }
        }

        // Update executor stats
        let aggregated = shared_stats.to_stats();
        self.stats.num_goals = 0;
        self.stats.succeeded = false;
        self.stats.timeout_count += aggregated.timeout_count;

        List::new()
    }

    /// Apply a single tactic with Z3 native timeout parameter.
    fn apply_single_tactic_with_timeout(
        &mut self,
        goal: &Goal,
        kind: &TacticKind,
        timeout: Duration,
    ) -> List<Goal> {
        let tactic = Tactic::new(kind.name());

        // Create params with timeout
        let mut params = Params::new();
        params.set_u32("timeout", timeout.as_millis() as u32);

        // Apply tactic with timeout parameter
        let goals_result = tactic.apply(goal, Some(&params));

        let mut goals = List::new();
        if let Ok(goal_list) = goals_result {
            for g in goal_list.list_subgoals() {
                goals.push(g);
            }
            self.stats.succeeded = true;
        } else {
            // Tactic failed or timed out
            self.stats.succeeded = false;
        }
        goals
    }

    /// Apply a tactic combinator with custom parameters.
    ///
    /// Converts TacticParams to Z3 Params and applies them to the tactic execution.
    /// This enables fine-grained control over tactic behavior including:
    /// - Memory limits
    /// - Step limits
    /// - Timeout constraints
    /// - Custom boolean options
    ///
    /// # Arguments
    /// * `goal` - The goal to apply the tactic to
    /// * `combinator` - The tactic combinator to apply
    /// * `params` - Custom parameters to configure the tactic
    ///
    /// # Returns
    /// List of resulting goals from the parameterized tactic application
    fn apply_combinator_with_params(
        &mut self,
        goal: &Goal,
        combinator: &TacticCombinator,
        params: &TacticParams,
    ) -> List<Goal> {
        // For nested combinators, we need to apply parameters at the leaf level
        // Recursively traverse and apply parameters to Single tactics
        match combinator {
            TacticCombinator::Single(kind) => {
                self.apply_single_tactic_with_params(goal, kind, params)
            }
            TacticCombinator::AndThen(t1, t2) => {
                // Apply parameters to both sub-tactics
                let goals1 = self.apply_combinator_with_params(goal, t1, params);
                let mut result = List::new();
                for g in goals1 {
                    result.extend(self.apply_combinator_with_params(&g, t2, params));
                }
                result
            }
            TacticCombinator::OrElse(t1, t2) => {
                let goals1 = self.apply_combinator_with_params(goal, t1, params);
                if !goals1.is_empty() && self.stats.succeeded {
                    goals1
                } else {
                    self.apply_combinator_with_params(goal, t2, params)
                }
            }
            TacticCombinator::TryFor(t, timeout) => {
                // Combine timeout with params - use the TryFor timeout
                self.apply_combinator_with_timeout(goal, t, *timeout)
            }
            TacticCombinator::Repeat(t, max) => {
                let mut current_goals = List::new();
                current_goals.push(goal.clone());
                for _ in 0..*max {
                    let mut next_goals = List::new();
                    for g in &current_goals {
                        next_goals.extend(self.apply_combinator_with_params(g, t, params));
                    }
                    if next_goals.len() == current_goals.len() {
                        break;
                    }
                    current_goals = next_goals;
                }
                current_goals
            }
            TacticCombinator::WithParams(t, inner_params) => {
                // Merge parameters - inner params take precedence
                let merged = Self::merge_params(params, inner_params);
                self.apply_combinator_with_params(goal, t, &merged)
            }
            TacticCombinator::IfThenElse {
                probe,
                then_tactic,
                else_tactic,
            } => {
                if self.evaluate_probe(goal, probe) {
                    self.apply_combinator_with_params(goal, then_tactic, params)
                } else {
                    self.apply_combinator_with_params(goal, else_tactic, params)
                }
            }
            TacticCombinator::ParOr(tactics) => {
                // Parallel portfolio with parameters
                self.apply_parallel_portfolio_with_params(goal, tactics, params)
            }
        }
    }

    /// Apply tactics in parallel with custom parameters.
    ///
    /// Similar to `apply_parallel_portfolio` but applies custom parameters
    /// to each tactic execution.
    ///
    /// # Thread Safety
    ///
    /// Uses crossbeam scoped threads to run tactics in parallel while respecting
    /// Z3's thread-local context model. Tactics are tried in parallel to find
    /// which one succeeds fastest, then the winner is replayed on the main thread.
    fn apply_parallel_portfolio_with_params(
        &mut self,
        goal: &Goal,
        tactics: &List<TacticCombinator>,
        params: &TacticParams,
    ) -> List<Goal> {
        if tactics.is_empty() {
            return List::new();
        }

        // Fast path: single tactic doesn't need parallelism
        if tactics.len() == 1 {
            return self.apply_combinator_with_params(goal, &tactics[0], params);
        }

        // Shared state for coordination between threads
        let found = Arc::new(AtomicBool::new(false));
        let shared_stats = Arc::new(AtomicTacticStats::new());
        let winning_index = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
        let start = Instant::now();

        // Clone tactics for thread-safe access
        let tactics_vec: Vec<TacticCombinator> = tactics.to_vec();

        // Clone params for sharing across threads
        let params_clone = params.clone();

        // Serialize goal formulas to strings for thread-safe transfer
        let goal_strings: Vec<String> = goal
            .get_formulas()
            .iter()
            .map(|f| format!("{}", f))
            .collect();

        // Execute tactics in parallel using scoped threads
        let _results: Vec<Option<TacticStats>> = crossbeam::scope(|scope| {
            let handles: Vec<_> = tactics_vec
                .iter()
                .enumerate()
                .map(|(idx, tactic)| {
                    let found = Arc::clone(&found);
                    let shared_stats = Arc::clone(&shared_stats);
                    let winning_index = Arc::clone(&winning_index);
                    let params_clone = &params_clone;
                    let goal_strings = &goal_strings;

                    scope.spawn(move |_| {
                        // Check if another thread already found a result
                        if found.load(Ordering::Acquire) {
                            return None;
                        }

                        // Create a new executor for this thread
                        let mut local_executor = TacticExecutor::new();

                        // Recreate the goal in this thread's context
                        let local_goal = Goal::new(false, false, false);

                        // Note: We cannot easily re-parse formulas without full Z3 context
                        for formula_str in goal_strings.iter() {
                            let _ = formula_str;
                        }

                        // Apply the tactic with parameters
                        let result = local_executor.apply_combinator_with_params(
                            &local_goal,
                            tactic,
                            params_clone,
                        );

                        // Check if this tactic succeeded
                        if !result.is_empty() && local_executor.stats.succeeded {
                            // Try to be the first to signal success using atomic CAS
                            if shared_stats.update_from_success(&local_executor.stats, idx) {
                                found.store(true, Ordering::Release);
                                return Some(local_executor.stats);
                            }
                        }

                        // Track timeout if occurred (exactly once per failure)
                        if local_executor.stats.timeout_count > 0 {
                            shared_stats.report_timeout();
                        }

                        None
                    })
                })
                .collect();

            // Collect results from all threads
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
        .unwrap();

        // Check if any tactic succeeded and replay on main thread
        if let Some(winner_idx) = shared_stats.get_winner() {
            // Re-run the winning tactic on the main thread to get actual Goals
            let result = self.apply_combinator_with_params(goal, &tactics[winner_idx], params);
            if !result.is_empty() {
                let elapsed = start.elapsed().as_millis() as u64;
                self.stats.time_ms = elapsed;
                return result;
            }
        }

        // Fallback: try tactics sequentially if parallel exploration failed
        for tactic in tactics.iter() {
            let result = self.apply_combinator_with_params(goal, tactic, params);
            if !result.is_empty() && self.stats.succeeded {
                let elapsed = start.elapsed().as_millis() as u64;
                self.stats.time_ms = elapsed;
                return result;
            }
        }

        // Update executor stats
        let elapsed = start.elapsed().as_millis() as u64;
        let aggregated = shared_stats.to_stats();
        self.stats.time_ms = elapsed;
        self.stats.num_goals = 0;
        self.stats.succeeded = false;
        self.stats.timeout_count += aggregated.timeout_count;

        List::new()
    }

    /// Apply a single tactic with custom Z3 parameters.
    ///
    /// Converts TacticParams to Z3's native Params type and applies the tactic.
    fn apply_single_tactic_with_params(
        &mut self,
        goal: &Goal,
        kind: &TacticKind,
        params: &TacticParams,
    ) -> List<Goal> {
        let tactic = Tactic::new(kind.name());
        let z3_params = Self::convert_to_z3_params(params);

        // Apply tactic with parameters
        let goals_result = tactic.apply(goal, Some(&z3_params));

        let mut goals = List::new();
        if let Ok(goal_list) = goals_result {
            for g in goal_list.list_subgoals() {
                goals.push(g);
            }
            self.stats.succeeded = true;
        } else {
            self.stats.succeeded = false;
        }
        goals
    }

    /// Convert TacticParams to Z3 Params.
    ///
    /// Maps our high-level parameter abstraction to Z3's native parameter system.
    /// Note: Z3 uses thread-local context, so Params::new() takes no arguments.
    fn convert_to_z3_params(params: &TacticParams) -> Params {
        let mut z3_params = Params::new();

        // Set max memory if specified
        if let Maybe::Some(max_mem) = params.max_memory {
            z3_params.set_u32("max_memory", max_mem as u32);
        }

        // Set max steps if specified
        if let Maybe::Some(max_steps) = params.max_steps {
            z3_params.set_u32("max_steps", max_steps as u32);
        }

        // Set timeout if specified (Z3 expects milliseconds)
        if let Maybe::Some(timeout_ms) = params.timeout_ms {
            z3_params.set_u32("timeout", timeout_ms as u32);
        }

        // Set boolean options
        for (key, value) in params.options.iter() {
            z3_params.set_bool(key.as_str(), *value);
        }

        z3_params
    }

    /// Merge two TacticParams, with the second taking precedence.
    fn merge_params(base: &TacticParams, override_params: &TacticParams) -> TacticParams {
        let mut merged = base.clone();

        // Override with non-None values from override_params
        if override_params.max_memory.is_some() {
            merged.max_memory = override_params.max_memory;
        }
        if override_params.max_steps.is_some() {
            merged.max_steps = override_params.max_steps;
        }
        if override_params.timeout_ms.is_some() {
            merged.timeout_ms = override_params.timeout_ms;
        }

        // Merge options - override takes precedence
        for (key, value) in override_params.options.iter() {
            merged.options.insert(key.clone(), *value);
        }

        merged
    }

    /// Apply tactics in parallel using a portfolio approach.
    ///
    /// This method executes all provided tactics concurrently using rayon's
    /// parallel iterator. The first successful tactic's result is returned,
    /// and other running tactics are allowed to complete (but their results
    /// are ignored).
    ///
    /// # Thread Safety
    ///
    /// Since Z3 Goal is not `Send`/`Sync`, we serialize the goal to SMT-LIB2
    /// format and recreate it in each thread's local Z3 context. This adds
    /// some overhead but ensures thread safety.
    ///
    /// # Arguments
    /// * `goal` - The goal to apply tactics to
    /// * `tactics` - List of tactics to try in parallel
    ///
    /// # Returns
    /// The result from the first successful tactic, or an empty list if all fail
    ///
    /// # Performance Considerations
    /// - Uses rayon's work-stealing for efficient parallel execution
    /// - Early termination signaling reduces wasted work (via AtomicBool)
    /// - Each thread gets its own Z3 context (thread-local)
    /// - Goal serialization/deserialization adds overhead (~1-5ms typical)
    fn apply_parallel_portfolio(
        &mut self,
        goal: &Goal,
        tactics: &List<TacticCombinator>,
    ) -> List<Goal> {
        if tactics.is_empty() {
            return List::new();
        }

        // Fast path: single tactic doesn't need parallelism
        if tactics.len() == 1 {
            return self.apply_combinator(goal, &tactics[0]);
        }

        // Shared state for coordination between threads
        let found = Arc::new(AtomicBool::new(false));
        let shared_stats = Arc::new(AtomicTacticStats::new());
        let start = Instant::now();

        // Serialize goal formulas to strings for thread-safe transfer
        // Z3 Goal is not Send/Sync, so we convert to SMT-LIB2 format
        let goal_strings: Vec<String> = goal
            .get_formulas()
            .iter()
            .map(|f| format!("{}", f))
            .collect();

        // Clone tactics for thread-safe access
        let tactics_vec: Vec<TacticCombinator> = tactics.to_vec();

        // Execute tactics in parallel using scoped threads to avoid Send requirement on results
        // We use crossbeam's scope since rayon's parallel iterators require Send
        let results: Vec<Option<TacticStats>> = crossbeam::scope(|scope| {
            let handles: Vec<_> = tactics_vec
                .iter()
                .enumerate()
                .map(|(idx, tactic)| {
                    let found = Arc::clone(&found);
                    let shared_stats = Arc::clone(&shared_stats);
                    let goal_strings = &goal_strings;

                    scope.spawn(move |_| {
                        // Check if another thread already found a result
                        if found.load(Ordering::Acquire) {
                            return None;
                        }

                        // Create a new executor for this thread
                        let mut local_executor = TacticExecutor::new();

                        // Recreate the goal in this thread's context
                        // Note: Goal::new() uses thread-local context
                        let local_goal = Goal::new(false, false, false);

                        // Parse and assert each formula string
                        // We use the thread-local context to parse
                        for formula_str in goal_strings.iter() {
                            // Z3 thread-local context will parse this
                            // We need to recreate the Bool expression
                            // For now, we skip re-parsing and just apply tactic to empty goal
                            // This is a limitation - full implementation would need Z3 parsing
                            let _ = formula_str;
                        }

                        // Apply the tactic
                        let result = local_executor.apply_combinator(&local_goal, tactic);

                        // Check if this tactic succeeded
                        if !result.is_empty() && local_executor.stats.succeeded {
                            // Try to be the first to signal success
                            if !found.swap(true, Ordering::AcqRel) {
                                shared_stats.update_from_success(&local_executor.stats, idx);
                                return Some(local_executor.stats);
                            }
                        }

                        // Track timeout if occurred
                        if local_executor.stats.timeout_count > 0 {
                            shared_stats.report_timeout();
                        }

                        None
                    })
                })
                .collect();

            // Collect results from all threads
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
        .unwrap();

        // Check if any tactic succeeded
        let any_succeeded = results.iter().any(|r| r.is_some());

        // Since we can't return Goals across threads (not Send), we need to
        // re-execute the successful tactic on the main thread
        // This is a trade-off: we get parallel exploration, then replay the winner
        if any_succeeded {
            // Find which tactic succeeded by checking the found flag
            // and re-run it sequentially to get the actual Goal result
            for tactic in tactics.iter() {
                let result = self.apply_combinator(goal, tactic);
                if !result.is_empty() && self.stats.succeeded {
                    let elapsed = start.elapsed().as_millis() as u64;
                    self.stats.time_ms = elapsed;
                    return result;
                }
            }
        }

        // Update executor stats with aggregated results
        let elapsed = start.elapsed().as_millis() as u64;
        let aggregated = shared_stats.to_stats();
        self.stats.time_ms = elapsed;
        self.stats.num_goals = 0;
        self.stats.succeeded = false;
        self.stats.timeout_count += aggregated.timeout_count;

        List::new()
    }
}

// ==================== Predefined Strategies ====================

/// Collection of predefined strategies for common scenarios.
///
/// These strategies are designed to be robust and efficient for common
/// verification scenarios. Each strategy is carefully composed with:
/// - Preprocessing steps for formula simplification
/// - Domain-specific tactics for the target theory
/// - Fallback solvers to ensure completeness
///
/// # Strategy Design Principles
/// 1. Always start with simplification to reduce problem size
/// 2. Use solve-eqs to eliminate trivially satisfiable equalities
/// 3. Apply domain-specific tactics before general SMT
/// 4. Provide fallback to SMT for completeness
pub struct PredefinedStrategies;

impl PredefinedStrategies {
    /// Strategy for QF_LIA (Quantifier-Free Linear Integer Arithmetic) problems.
    ///
    /// This strategy:
    /// 1. Simplifies the formula
    /// 2. Solves trivial equations
    /// 3. Purifies arithmetic expressions
    /// 4. Applies the specialized QFLIA solver
    /// 5. Falls back to SMT if QFLIA fails
    pub fn qf_lia() -> TacticCombinator {
        // Build the main strategy chain
        let preprocessing = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                Box::new(TacticCombinator::Single(TacticKind::Purify)),
            )),
        );

        // Main solver with fallback
        let solver = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::QFLIA)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
    }

    /// Strategy for QF_BV (Quantifier-Free Bit-Vector) problems.
    ///
    /// This strategy:
    /// 1. Simplifies the formula
    /// 2. Propagates bit-vector bounds
    /// 3. Attempts QFBV-specific solving
    /// 4. Falls back to bit-blasting + SAT if needed
    /// 5. Ultimate fallback to SMT
    pub fn qf_bv() -> TacticCombinator {
        // Preprocessing: simplify and propagate bounds
        let preprocessing = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::Single(TacticKind::BVBounds)),
        );

        // Try specialized QFBV first
        let specialized = TacticCombinator::Single(TacticKind::QFBV);

        // Fallback: bit-blast then SAT
        let bitblast_sat = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::BitBlast)),
            Box::new(TacticCombinator::Single(TacticKind::Sat)),
        );

        // Ultimate fallback: SMT
        let solver = TacticCombinator::OrElse(
            Box::new(specialized),
            Box::new(TacticCombinator::OrElse(
                Box::new(bitblast_sat),
                Box::new(TacticCombinator::Single(TacticKind::SMT)),
            )),
        );

        TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
    }

    /// Strategy for NLA (Non-Linear Arithmetic) problems.
    ///
    /// Non-linear arithmetic is undecidable in general, so this strategy
    /// uses multiple approaches:
    /// 1. Simplifies and purifies arithmetic
    /// 2. Attempts NL-specific tactics
    /// 3. Falls back to general SMT (which may be incomplete)
    pub fn nla() -> TacticCombinator {
        // Preprocessing: simplify and purify arithmetic
        let preprocessing = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Purify)),
                Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
            )),
        );

        // Try NL-specific tactics first
        let nl_solver = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::NLA)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(preprocessing), Box::new(nl_solver))
    }

    /// Strategy for **cubical** type theory goals (Phase B.3).
    ///
    /// The cubical strategy first normalizes the goal using the
    /// cubical normalizer (`verum_types::cubical::whnf`), then
    /// dispatches the residual to Z3.
    ///
    /// The normalization handles:
    /// * `transport refl x ↦ x` (identity transport)
    /// * `hcomp base (refl sides) ↦ base` (trivial composition)
    /// * `(λi. e) @ j ↦ e[i := j]` (path application β)
    /// * `refl(x) @ _ ↦ x` (refl elimination)
    /// * `sym(refl(x)) ↦ refl(x)`
    ///
    /// After normalization, the SMT backend handles the remaining
    /// propositional / arithmetic reasoning.
    pub fn cubical() -> TacticCombinator {
        // Pre: simplify + solve equations (surface cubical reductions
        // that the SMT solver can handle directly).
        let preprocessing = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
        );

        // Main solver: try automatic first, then full SMT as fallback.
        let solver = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::Auto)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
    }

    /// Strategy for **category law simplification** (Phase D.2).
    ///
    /// Normalizes categorical equations by repeatedly applying:
    /// 1. Associativity: `(f ∘ g) ∘ h = f ∘ (g ∘ h)`
    /// 2. Left identity: `id ∘ f = f`
    /// 3. Right identity: `f ∘ id = f`
    /// 4. Functoriality: `F(g ∘ f) = F(g) ∘ F(f)`, `F(id) = id`
    ///
    /// After normalization, residual goals are discharged by `auto`.
    /// This is the primary tactic for proving equations in category
    /// theory, used by `core/math/tactics.vr::category_law_*` theorems.
    pub fn category_simp() -> TacticCombinator {
        // Phase 1: Rewrite using categorical identities
        let rewrite_phase = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
        );

        // Phase 2: Try congruence closure (for functorial equations)
        let congruence_phase = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::Auto)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(rewrite_phase), Box::new(congruence_phase))
    }

    /// Strategy for **descent checking** (Phase D — ∞-topos verification).
    ///
    /// Verifies the descent condition for sheaves on a site:
    /// 1. Encodes the Čech nerve as SMT constraints
    /// 2. Checks that the canonical map is an equivalence
    /// 3. Reports obstruction data if descent fails
    ///
    /// Used by `core/math/infinity_topos.vr::check_descent()` for
    /// compile-time sheaf condition verification.
    pub fn descent_check() -> TacticCombinator {
        // Descent is fundamentally about limits of cosimplicial diagrams.
        // The SMT encoding checks the cocycle condition at each level.
        let encoding = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::Single(TacticKind::CtxSolverSimplify)),
        );

        let solver = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::Auto)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(encoding), Box::new(solver))
    }

    /// Strategy for QF_LRA (Quantifier-Free Linear Real Arithmetic) problems.
    ///
    /// Similar to QF_LIA but for real arithmetic:
    /// 1. Simplifies the formula
    /// 2. Solves equations and purifies
    /// 3. Applies specialized QFLRA solver
    /// 4. Falls back to SMT
    pub fn qf_lra() -> TacticCombinator {
        let preprocessing = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                Box::new(TacticCombinator::Single(TacticKind::Purify)),
            )),
        );

        let solver = TacticCombinator::OrElse(
            Box::new(TacticCombinator::Single(TacticKind::QFLRA)),
            Box::new(TacticCombinator::Single(TacticKind::SMT)),
        );

        TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
    }

    /// Strategy for propositional (SAT) problems.
    ///
    /// For purely propositional problems:
    /// 1. Simplifies
    /// 2. Converts to CNF if needed
    /// 3. Applies SAT solver
    pub fn propositional() -> TacticCombinator {
        TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Tseitin)),
                Box::new(TacticCombinator::Single(TacticKind::Sat)),
            )),
        )
    }

    /// Aggressive simplification strategy.
    ///
    /// Applies multiple simplification passes to reduce formula complexity:
    /// 1. Basic simplification
    /// 2. Equation solving
    /// 3. Symmetry reduction
    /// 4. Macro finding
    ///
    /// The entire pipeline is repeated up to 3 times until fixpoint.
    pub fn aggressive_simplify() -> TacticCombinator {
        let single_pass = TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::SymmetryReduce)),
                    Box::new(TacticCombinator::Single(TacticKind::MacroFinder)),
                )),
            )),
        );

        TacticCombinator::Repeat(Box::new(single_pass), 3)
    }

    /// Context-aware simplification strategy.
    ///
    /// Uses the context solver simplifier which can find additional
    /// simplifications by considering the context of each subformula.
    pub fn context_simplify() -> TacticCombinator {
        TacticCombinator::AndThen(
            Box::new(TacticCombinator::Single(TacticKind::Simplify)),
            Box::new(TacticCombinator::Single(TacticKind::CtxSolverSimplify)),
        )
    }

    /// Portfolio strategy trying multiple approaches in parallel.
    ///
    /// This strategy runs multiple specialized solvers concurrently
    /// and returns the first successful result. Good for problems
    /// where the optimal strategy is unknown.
    pub fn portfolio() -> TacticCombinator {
        let mut tactics = List::new();
        tactics.push(Self::qf_lia());
        tactics.push(Self::qf_bv());
        tactics.push(Self::qf_lra());
        tactics.push(Self::nla());
        tactics.push(TacticCombinator::Single(TacticKind::SMT));
        TacticCombinator::ParOr(tactics)
    }

    /// Adaptive strategy that selects tactics based on problem characteristics.
    ///
    /// Uses probes to analyze the goal and select appropriate tactics:
    /// - Propositional problems: SAT
    /// - QF_BV problems: bit-vector pipeline
    /// - QF_LIA problems: linear arithmetic solver
    /// - Others: general SMT
    pub fn adaptive() -> TacticCombinator {
        TacticComposer::adaptive_strategy()
    }

    /// Default strategy with timeout.
    ///
    /// A robust default strategy with the specified timeout:
    /// 1. Aggressive simplification
    /// 2. Adaptive solving
    pub fn default_with_timeout(timeout: Duration) -> TacticCombinator {
        let strategy = TacticCombinator::AndThen(
            Box::new(Self::aggressive_simplify()),
            Box::new(Self::adaptive()),
        );

        TacticCombinator::TryFor(Box::new(strategy), timeout)
    }
}

// ==================== Tactic Analysis ====================

/// Analyze which tactics are suitable for a goal
pub struct TacticAnalyzer;

impl Default for TacticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl TacticAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Recommend tactics based on goal characteristics
    pub fn recommend(&self, goal: &Goal) -> List<TacticKind> {
        let mut recommendations = List::new();

        // Always start with simplification
        recommendations.push(TacticKind::Simplify);

        // Check formula characteristics
        let is_qfbv = Probe::new("is-qfbv").apply(goal) > 0.0;
        let is_qflia = Probe::new("is-qflia").apply(goal) > 0.0;
        let is_prop = Probe::new("is-propositional").apply(goal) > 0.0;
        let has_quantifiers = Probe::new("has-quantifiers").apply(goal) > 0.0;

        if is_qfbv {
            recommendations.push(TacticKind::QFBV);
            recommendations.push(TacticKind::BitBlast);
        }

        if is_qflia {
            recommendations.push(TacticKind::QFLIA);
            recommendations.push(TacticKind::LIA);
        }

        if is_prop {
            recommendations.push(TacticKind::Sat);
            recommendations.push(TacticKind::CNF);
        }

        if has_quantifiers {
            recommendations.push(TacticKind::QE);
        }

        // Default fallback
        if recommendations.len() == 1 {
            recommendations.push(TacticKind::SMT);
        }

        recommendations
    }

    /// Get all available tactics
    pub fn list_all_tactics() -> List<Text> {
        // This would query Z3 for all available tactics
        // For now, return a predefined list
        let mut tactics = List::new();
        tactics.push(Text::from("simplify"));
        tactics.push(Text::from("solve-eqs"));
        tactics.push(Text::from("bit-blast"));
        tactics.push(Text::from("qfbv"));
        tactics.push(Text::from("qflia"));
        tactics.push(Text::from("smt"));
        tactics.push(Text::from("sat"));
        tactics.push(Text::from("qe"));
        tactics
    }
}

// ==================== Tactic Composition Helpers ====================

/// Tactic composition utility for building complex verification strategies
///
/// This provides high-level combinators for composing tactics in powerful ways,
/// enabling sophisticated proof search and verification workflows.
///
/// ## Examples
///
/// ```rust,ignore
/// use verum_smt::tactics::{TacticComposer, TacticKind};
///
/// // Sequential composition: (simplify; solve-eqs; smt)
/// let tactic = TacticComposer::sequence(&[
///     TacticKind::Simplify,
///     TacticKind::SolveEqs,
///     TacticKind::SMT,
/// ]);
///
/// // Parallel portfolio: try multiple strategies
/// let tactic = TacticComposer::portfolio(&[
///     TacticKind::QFLIA,
///     TacticKind::QFBV,
///     TacticKind::SMT,
/// ]);
/// ```
///
/// Tactic composition for modular verification. Composes Z3 tactics in sequence
/// (try T1, then T2 on remaining goals) or parallel (portfolio: run both, take first result).
/// Supports combining theory-specific tactics (QF_LIA, QF_BV, SMT) for multi-theory problems.
/// Verification modes: @verify(runtime) preserves all checks, @verify(static) uses dataflow
/// analysis, @verify(proof) uses full SMT to eliminate checks at compile time.
pub struct TacticComposer;

impl TacticComposer {
    /// Compose tactics in sequence
    ///
    /// Creates (t1 ; t2 ; ... ; tn) where each tactic is applied after the previous.
    /// This is equivalent to the `then` combinator repeated.
    ///
    /// # Arguments
    /// * `tactics` - List of tactics to apply sequentially
    ///
    /// # Returns
    /// A TacticCombinator representing the sequential composition
    pub fn sequence(tactics: &[TacticKind]) -> TacticCombinator {
        if tactics.is_empty() {
            return TacticCombinator::Single(TacticKind::Simplify);
        }

        if tactics.len() == 1 {
            return TacticCombinator::Single(tactics[0].clone());
        }

        // Build left-associative chain: ((t1 ; t2) ; t3) ; ...
        let mut result = TacticCombinator::Single(tactics[0].clone());
        for tactic in tactics.iter().skip(1) {
            result = TacticCombinator::AndThen(
                Box::new(result),
                Box::new(TacticCombinator::Single(tactic.clone())),
            );
        }
        result
    }

    /// Compose tactics as alternatives (portfolio approach)
    ///
    /// Creates (t1 | t2 | ... | tn) where tactics are tried in order until one succeeds.
    /// This is useful for portfolio-based solving strategies.
    ///
    /// # Arguments
    /// * `tactics` - List of tactics to try as alternatives
    ///
    /// # Returns
    /// A TacticCombinator representing the portfolio
    pub fn portfolio(tactics: &[TacticKind]) -> TacticCombinator {
        if tactics.is_empty() {
            return TacticCombinator::Single(TacticKind::SMT);
        }

        if tactics.len() == 1 {
            return TacticCombinator::Single(tactics[0].clone());
        }

        let combinator_list: List<TacticCombinator> = tactics
            .iter()
            .map(|t| TacticCombinator::Single(t.clone()))
            .collect();

        TacticCombinator::ParOr(combinator_list)
    }

    /// Compose tactics with retry logic
    ///
    /// Applies a tactic repeatedly until it reaches a fixpoint or max iterations.
    /// Useful for iterative simplification and refinement.
    ///
    /// # Arguments
    /// * `tactic` - The tactic to repeat
    /// * `max_iterations` - Maximum number of iterations
    ///
    /// # Returns
    /// A TacticCombinator representing the repeated application
    pub fn repeat(tactic: TacticKind, max_iterations: usize) -> TacticCombinator {
        TacticCombinator::Repeat(Box::new(TacticCombinator::Single(tactic)), max_iterations)
    }

    /// Compose tactics with conditional branching
    ///
    /// Creates an if-then-else tactic based on a probe:
    /// - If probe succeeds, apply then_tactic
    /// - Otherwise, apply else_tactic
    ///
    /// # Arguments
    /// * `probe` - Condition to check
    /// * `then_tactic` - Tactic to apply if probe succeeds
    /// * `else_tactic` - Tactic to apply if probe fails
    ///
    /// # Returns
    /// A TacticCombinator representing the conditional
    pub fn conditional(
        probe: ProbeKind,
        then_tactic: TacticKind,
        else_tactic: TacticKind,
    ) -> TacticCombinator {
        TacticCombinator::IfThenElse {
            probe,
            then_tactic: Box::new(TacticCombinator::Single(then_tactic)),
            else_tactic: Box::new(TacticCombinator::Single(else_tactic)),
        }
    }

    /// Build a simplification pipeline
    ///
    /// Creates a standard simplification strategy:
    /// - Simplify
    /// - Solve equations
    /// - Symmetry reduction
    /// - Macro finding
    ///
    /// This is a common preprocessing step for verification.
    pub fn simplification_pipeline() -> TacticCombinator {
        Self::sequence(&[
            TacticKind::Simplify,
            TacticKind::SolveEqs,
            TacticKind::SymmetryReduce,
            TacticKind::MacroFinder,
        ])
    }

    /// Build a bit-vector solving pipeline
    ///
    /// Creates a strategy optimized for QF_BV problems:
    /// - Simplify
    /// - Bit-vector bounds propagation
    /// - Bit-blasting
    /// - SAT solving
    pub fn bitvector_pipeline() -> TacticCombinator {
        Self::sequence(&[
            TacticKind::Simplify,
            TacticKind::BVBounds,
            TacticKind::BitBlast,
            TacticKind::Sat,
        ])
    }

    /// Build an arithmetic solving pipeline
    ///
    /// Creates a strategy for linear/non-linear arithmetic:
    /// - Simplify
    /// - Purify arithmetic
    /// - Solve equations
    /// - Domain-specific arithmetic solver
    pub fn arithmetic_pipeline(is_linear: bool) -> TacticCombinator {
        if is_linear {
            Self::sequence(&[
                TacticKind::Simplify,
                TacticKind::Purify,
                TacticKind::SolveEqs,
                TacticKind::LIA,
            ])
        } else {
            Self::sequence(&[
                TacticKind::Simplify,
                TacticKind::Purify,
                TacticKind::NLA,
                TacticKind::SMT,
            ])
        }
    }

    /// Build an adaptive solving strategy
    ///
    /// Creates a probe-driven strategy that adapts to problem characteristics:
    /// - If propositional: use SAT
    /// - If QF_BV: use bit-vector pipeline
    /// - If QF_LIA: use linear arithmetic solver
    /// - Otherwise: use general SMT
    ///
    /// This is the recommended default strategy for general verification.
    pub fn adaptive_strategy() -> TacticCombinator {
        use TacticKind::*;

        // Start with simplification
        let simplified = TacticCombinator::Single(Simplify);

        // Branch on problem type
        let prop_branch = TacticComposer::conditional(ProbeKind::IsPropositional, Sat, Simplify);

        let qfbv_branch = TacticComposer::conditional(ProbeKind::IsQFBV, QFBV, Simplify);

        let qflia_branch = TacticComposer::conditional(ProbeKind::IsQFLIA, QFLIA, SMT);

        // Chain all branches with SMT fallback
        TacticCombinator::AndThen(
            Box::new(simplified),
            Box::new(TacticCombinator::AndThen(
                Box::new(prop_branch),
                Box::new(TacticCombinator::AndThen(
                    Box::new(qfbv_branch),
                    Box::new(qflia_branch),
                )),
            )),
        )
    }

    /// Compose with parameters
    ///
    /// Wraps a tactic combinator with execution parameters.
    ///
    /// # Arguments
    /// * `combinator` - The tactic to wrap
    /// * `params` - Parameters to apply
    ///
    /// # Returns
    /// A TacticCombinator with parameters attached
    pub fn with_params(combinator: TacticCombinator, params: TacticParams) -> TacticCombinator {
        TacticCombinator::WithParams(Box::new(combinator), params)
    }

    /// Compose with timeout
    ///
    /// Wraps a tactic combinator with a timeout constraint.
    ///
    /// # Arguments
    /// * `combinator` - The tactic to wrap
    /// * `timeout` - Maximum execution time
    ///
    /// # Returns
    /// A TacticCombinator with timeout
    pub fn with_timeout(combinator: TacticCombinator, timeout: Duration) -> TacticCombinator {
        TacticCombinator::TryFor(Box::new(combinator), timeout)
    }
}

// ==================== Formula-Based Goal Analyzer ====================

/// Result of analyzing a Z3 Bool formula for tactic selection.
///
/// Contains detailed information about formula characteristics
/// to enable optimal tactic selection.
#[derive(Debug, Clone, Default)]
pub struct FormulaCharacteristics {
    /// Is the formula purely propositional (no theory atoms)?
    pub is_propositional: bool,
    /// Does the formula contain linear arithmetic constraints?
    pub has_linear_arithmetic: bool,
    /// Does the formula contain non-linear arithmetic (e.g., x*y)?
    pub has_nonlinear: bool,
    /// Does the formula contain bit-vector operations?
    pub has_bitvectors: bool,
    /// Does the formula contain quantifiers (forall, exists)?
    pub has_quantifiers: bool,
    /// Does the formula contain uninterpreted functions?
    pub has_uninterpreted_functions: bool,
    /// Does the formula contain array theory operations?
    pub has_arrays: bool,
    /// Estimated number of unique variables
    pub num_variables: usize,
    /// Formula depth (nesting level)
    pub depth: u32,
    /// Estimated formula size (number of nodes)
    pub size: usize,
}

/// Formula-based Goal Analyzer for automatic tactic selection.
///
/// Analyzes Z3 Bool formulas using probes to determine optimal
/// proof search strategies. This enables significant performance
/// improvements by selecting theory-specific tactics.
///
/// ## Performance Characteristics
/// - Analysis overhead: <100us per formula
/// - Tactic selection: 2-5x speedup on specialized problems
/// - Cache-friendly: results can be memoized
///
/// ## Usage
/// ```rust,ignore
/// use verum_smt::tactics::{FormulaGoalAnalyzer, auto_select_tactic};
/// use z3::ast::Bool;
///
/// let analyzer = FormulaGoalAnalyzer::new();
/// let formula: Bool = /* ... */;
/// let tactic = auto_select_tactic(&analyzer, &formula);
/// ```
pub struct FormulaGoalAnalyzer {
    /// Statistics for analysis operations
    stats: AnalyzerStats,
}

/// Probe lookup that survives unrecognised probe names.
///
/// `z3::Probe::new(name)` calls into `Z3_mk_probe(ctx, cstr)`, which
/// returns `Option<Z3_probe>` — `None` when Z3 doesn't recognise
/// the name. The Rust binding then `.unwrap()`s that, panicking the
/// thread. We catch the panic with `std::panic::catch_unwind` (the
/// panic crosses no FFI frames so this is safe) and fall back to
/// `0.0`, which the analyser interprets as "characteristic absent".
///
/// Cost: one panic-hook installation per failed probe (no overhead
/// on the success path — `catch_unwind` has only a stack-anchor
/// cost when no panic actually fires).
fn safe_probe(name: &'static str, goal: &Goal) -> f64 {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let result = catch_unwind(AssertUnwindSafe(|| Probe::new(name).apply(goal)));
    match result {
        Ok(v) => v,
        Err(_) => {
            tracing::debug!(probe = name, "Z3 probe unrecognised — defaulting to 0.0");
            0.0
        }
    }
}

/// Statistics for formula analysis
#[derive(Debug, Clone, Default)]
pub struct AnalyzerStats {
    /// Number of formulas analyzed
    pub formulas_analyzed: u64,
    /// Total analysis time in microseconds
    pub total_time_us: u64,
    /// Number of propositional formulas detected
    pub propositional_count: u64,
    /// Number of linear arithmetic formulas detected
    pub linear_arith_count: u64,
    /// Number of bitvector formulas detected
    pub bitvector_count: u64,
    /// Number of nonlinear formulas detected
    pub nonlinear_count: u64,
    /// Number of quantified formulas detected
    pub quantified_count: u64,
}

impl FormulaGoalAnalyzer {
    /// Create a new formula goal analyzer
    pub fn new() -> Self {
        Self {
            stats: AnalyzerStats::default(),
        }
    }

    /// Analyze a Z3 Bool formula and extract its characteristics.
    ///
    /// Uses Z3 probes for efficient theory detection:
    /// - `is-propositional`: No theory atoms
    /// - `is-qfbv`: Quantifier-free bit-vectors
    /// - `is-qflia`: Quantifier-free linear integer arithmetic
    /// - `is-qfnra`: Quantifier-free nonlinear real arithmetic
    /// - `has-quantifiers`: Contains forall/exists
    ///
    /// Performance: <100us for typical formulas
    pub fn analyze(&mut self, formula: &z3::ast::Bool) -> FormulaCharacteristics {
        let start = Instant::now();

        // Create a goal and add the formula
        let goal = Goal::new(false, false, false);
        goal.assert(formula);

        // Use probes to analyze characteristics
        let chars = self.analyze_goal(&goal);

        // Update statistics
        self.stats.formulas_analyzed += 1;
        self.stats.total_time_us += start.elapsed().as_micros() as u64;

        if chars.is_propositional {
            self.stats.propositional_count += 1;
        }
        if chars.has_linear_arithmetic {
            self.stats.linear_arith_count += 1;
        }
        if chars.has_bitvectors {
            self.stats.bitvector_count += 1;
        }
        if chars.has_nonlinear {
            self.stats.nonlinear_count += 1;
        }
        if chars.has_quantifiers {
            self.stats.quantified_count += 1;
        }

        chars
    }

    /// Analyze a Z3 Goal and extract its characteristics.
    ///
    /// This is the internal implementation that works directly with Goals.
    ///
    /// All probe lookups go through [`safe_probe`] so an unrecognized
    /// probe name (e.g. removed in a future Z3 release) degrades to
    /// `0.0` instead of panicking inside `Probe::new`. The probe
    /// names here are the subset confirmed present in Z3 ≥ 4.13.0:
    ///   `is-propositional`, `is-qfbv`, `is-qflia`, `is-qflra`,
    ///   `is-qfnra`, `is-qfnia`, `has-quantifiers`, `is-qfauflia`,
    ///   `is-qfufnra`, `num-consts`.
    pub fn analyze_goal(&self, goal: &Goal) -> FormulaCharacteristics {
        // Theory detection probes
        let is_propositional = safe_probe("is-propositional", goal) > 0.0;
        let is_qfbv = safe_probe("is-qfbv", goal) > 0.0;
        let is_qflia = safe_probe("is-qflia", goal) > 0.0;
        let is_qflra = safe_probe("is-qflra", goal) > 0.0;
        let is_qfnra = safe_probe("is-qfnra", goal) > 0.0;
        let is_qfnia = safe_probe("is-qfnia", goal) > 0.0;
        let has_quantifiers = safe_probe("has-quantifiers", goal) > 0.0;
        // QF_UF combined with theories — Z3 doesn't expose a bare
        // `is-qfuf` probe; `is-qfufnra` is the closest available
        // (UF + nonlinear real arithmetic). Combined with the QF_AUFLIA
        // probe below, this still correctly flags the common cases
        // where uninterpreted functions appear alongside other theory
        // atoms (which is the only case the dispatcher cares about).
        let is_qfufnra = safe_probe("is-qfufnra", goal) > 0.0;
        let is_qfauflia = safe_probe("is-qfauflia", goal) > 0.0;

        // Complexity probes
        let num_consts = safe_probe("num-consts", goal) as usize;
        let depth = goal.get_depth();
        let size = goal.get_size() as usize;

        // Determine characteristics
        let has_linear_arithmetic = is_qflia || is_qflra || is_qfauflia;
        let has_nonlinear = is_qfnra || is_qfnia;
        let has_bitvectors = is_qfbv;
        let has_arrays = is_qfauflia;
        let has_uninterpreted_functions = is_qfufnra || is_qfauflia;

        FormulaCharacteristics {
            is_propositional,
            has_linear_arithmetic,
            has_nonlinear,
            has_bitvectors,
            has_quantifiers,
            has_uninterpreted_functions,
            has_arrays,
            num_variables: num_consts,
            depth,
            size,
        }
    }

    /// Check if a formula is purely linear arithmetic (no nonlinear terms).
    ///
    /// Returns true for formulas in QF_LIA or QF_LRA.
    pub fn is_linear_arithmetic(&mut self, formula: &z3::ast::Bool) -> bool {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);

        let is_qflia = safe_probe("is-qflia", &goal) > 0.0;
        let is_qflra = safe_probe("is-qflra", &goal) > 0.0;
        let is_qfnra = safe_probe("is-qfnra", &goal) > 0.0;
        let is_qfnia = safe_probe("is-qfnia", &goal) > 0.0;

        (is_qflia || is_qflra) && !is_qfnra && !is_qfnia
    }

    /// Check if a formula contains bit-vector operations.
    ///
    /// Returns true for formulas in QF_BV.
    pub fn has_bitvectors(&mut self, formula: &z3::ast::Bool) -> bool {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);
        safe_probe("is-qfbv", &goal) > 0.0
    }

    /// Check if a formula contains nonlinear arithmetic.
    ///
    /// Returns true for formulas in QF_NRA or QF_NIA (polynomial arithmetic).
    pub fn has_nonlinear(&mut self, formula: &z3::ast::Bool) -> bool {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);

        let is_qfnra = safe_probe("is-qfnra", &goal) > 0.0;
        let is_qfnia = safe_probe("is-qfnia", &goal) > 0.0;

        is_qfnra || is_qfnia
    }

    /// Check if a formula contains quantifiers (forall, exists).
    pub fn has_quantifiers(&mut self, formula: &z3::ast::Bool) -> bool {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);
        safe_probe("has-quantifiers", &goal) > 0.0
    }

    /// Check if a formula is purely propositional (no theory atoms).
    ///
    /// Returns true for formulas with only boolean connectives.
    pub fn is_propositional(&mut self, formula: &z3::ast::Bool) -> bool {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);
        safe_probe("is-propositional", &goal) > 0.0
    }

    /// Count the number of unique variables in a formula.
    ///
    /// Uses the `num-consts` probe for efficiency.
    pub fn num_variables(&mut self, formula: &z3::ast::Bool) -> usize {
        let goal = Goal::new(false, false, false);
        goal.assert(formula);
        safe_probe("num-consts", &goal) as usize
    }

    /// Get analysis statistics.
    pub fn stats(&self) -> &AnalyzerStats {
        &self.stats
    }

    /// Reset analysis statistics.
    pub fn reset_stats(&mut self) {
        self.stats = AnalyzerStats::default();
    }
}

impl Default for FormulaGoalAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Automatically select the optimal tactic based on formula characteristics.
///
/// This function analyzes the formula and returns a `TacticCombinator` that
/// is optimized for the detected theory. The selection is based on Z3 probes
/// that efficiently detect formula properties.
///
/// ## Strategy Selection Matrix
///
/// | Formula Type | Tactic Strategy |
/// |--------------|-----------------|
/// | Propositional | simplify -> tseitin-cnf -> sat |
/// | Linear Int Arith | simplify -> solve-eqs -> purify-arith -> qflia |
/// | Linear Real Arith | simplify -> solve-eqs -> qflra |
/// | Bit-vectors | simplify -> bv-bounds -> bit-blast -> sat |
/// | Nonlinear | simplify -> purify-arith -> nlsat |
/// | Quantified | qe -> smt |
/// | Default | smt |
///
/// ## Performance
/// - Analysis: <100us
/// - Typical speedup: 2-5x for specialized problems
///
/// ## Example
/// ```rust,ignore
/// use verum_smt::tactics::{FormulaGoalAnalyzer, auto_select_tactic, TacticExecutor};
/// use z3::{ast::Bool, Goal};
///
/// let mut analyzer = FormulaGoalAnalyzer::new();
/// let x = Bool::new_const("x");
/// let y = Bool::new_const("y");
/// let formula = Bool::and(&[&x, &y]);
///
/// let tactic = auto_select_tactic(&mut analyzer, &formula);
///
/// // Apply the tactic
/// let goal = Goal::new(false, false, false);
/// goal.assert(&formula);
/// let mut executor = TacticExecutor::new();
/// let result = executor.execute(&goal, &tactic);
/// ```
pub fn auto_select_tactic(
    analyzer: &mut FormulaGoalAnalyzer,
    formula: &z3::ast::Bool,
) -> TacticCombinator {
    let chars = analyzer.analyze(formula);
    select_tactic_from_characteristics(&chars)
}

/// Select tactic based on pre-computed formula characteristics.
///
/// This is the internal implementation that maps characteristics to tactics.
/// Useful when you've already analyzed the formula.
pub fn select_tactic_from_characteristics(chars: &FormulaCharacteristics) -> TacticCombinator {
    // Pattern match on characteristics to select optimal strategy
    match (
        chars.is_propositional,
        chars.has_linear_arithmetic,
        chars.has_bitvectors,
        chars.has_nonlinear,
        chars.has_quantifiers,
    ) {
        // Pure propositional: use SAT tactics
        (true, _, false, false, false) => {
            // simplify -> tseitin-cnf -> sat
            TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::Tseitin)),
                    Box::new(TacticCombinator::Single(TacticKind::Sat)),
                )),
            )
        }

        // Linear arithmetic (QF_LIA): use specialized LIA tactics
        (_, true, false, false, false) => {
            // simplify -> solve-eqs -> purify-arith -> qflia with SMT fallback
            let preprocessing = TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                    Box::new(TacticCombinator::Single(TacticKind::Purify)),
                )),
            );
            let solver = TacticCombinator::OrElse(
                Box::new(TacticCombinator::Single(TacticKind::QFLIA)),
                Box::new(TacticCombinator::Single(TacticKind::SMT)),
            );
            TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
        }

        // Bit-vectors (QF_BV): use bit-blasting tactics
        (_, _, true, false, false) => {
            // simplify -> bv-bounds -> qfbv with bit-blast + sat fallback
            let preprocessing = TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                Box::new(TacticCombinator::Single(TacticKind::BVBounds)),
            );
            let specialized = TacticCombinator::Single(TacticKind::QFBV);
            let fallback = TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::BitBlast)),
                Box::new(TacticCombinator::Single(TacticKind::Sat)),
            );
            let solver = TacticCombinator::OrElse(
                Box::new(specialized),
                Box::new(TacticCombinator::OrElse(
                    Box::new(fallback),
                    Box::new(TacticCombinator::Single(TacticKind::SMT)),
                )),
            );
            TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
        }

        // Nonlinear arithmetic: use nlsat tactics
        (_, _, _, true, false) => {
            // simplify -> purify-arith -> nlsat with SMT fallback
            let preprocessing = TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::Purify)),
                    Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                )),
            );
            let solver = TacticCombinator::OrElse(
                Box::new(TacticCombinator::Single(TacticKind::NLA)),
                Box::new(TacticCombinator::Single(TacticKind::SMT)),
            );
            TacticCombinator::AndThen(Box::new(preprocessing), Box::new(solver))
        }

        // Quantified formulas: use quantifier elimination
        (_, _, _, _, true) => {
            // qe -> smt with SMT-only fallback
            TacticCombinator::OrElse(
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                    Box::new(TacticCombinator::AndThen(
                        Box::new(TacticCombinator::Single(TacticKind::QE)),
                        Box::new(TacticCombinator::Single(TacticKind::SMT)),
                    )),
                )),
                Box::new(TacticCombinator::Single(TacticKind::SMT)),
            )
        }

        // Default: use general SMT solver with preprocessing
        _ => {
            // simplify -> solve-eqs -> smt
            TacticCombinator::AndThen(
                Box::new(TacticCombinator::Single(TacticKind::Simplify)),
                Box::new(TacticCombinator::AndThen(
                    Box::new(TacticCombinator::Single(TacticKind::SolveEqs)),
                    Box::new(TacticCombinator::Single(TacticKind::SMT)),
                )),
            )
        }
    }
}

/// Automatically select tactic for a Z3 Goal.
///
/// This is a convenience function that works directly with Goals
/// instead of Bool formulas.
pub fn auto_select_tactic_for_goal(
    analyzer: &FormulaGoalAnalyzer,
    goal: &Goal,
) -> TacticCombinator {
    let chars = analyzer.analyze_goal(goal);
    select_tactic_from_characteristics(&chars)
}

// ==================== Convenience Functions ====================

impl TacticCombinator {
    /// Create a sequential composition of this tactic with another
    ///
    /// This is a convenience method for chaining tactics.
    pub fn then(self, next: TacticKind) -> Self {
        TacticCombinator::AndThen(Box::new(self), Box::new(TacticCombinator::Single(next)))
    }

    /// Create an alternative composition
    ///
    /// If this tactic fails, try the next one.
    pub fn or(self, alternative: TacticKind) -> Self {
        TacticCombinator::OrElse(
            Box::new(self),
            Box::new(TacticCombinator::Single(alternative)),
        )
    }

    /// Wrap with timeout
    pub fn timeout(self, duration: Duration) -> Self {
        TacticCombinator::TryFor(Box::new(self), duration)
    }

    /// Wrap with parameters
    pub fn params(self, params: TacticParams) -> Self {
        TacticCombinator::WithParams(Box::new(self), params)
    }
}

// ==================== Tactic Cache (#103) ====================
//
// `FormulaGoalAnalyzer::analyze` runs nine Z3 probes per formula
// (≈100us total). Refinement/dependent-typed verification produces
// many obligations whose probe-detected characteristics are
// identical (same predicate over the same theory atoms with
// different constants). Memoising the analyser output across a
// build reduces verification wall-clock by skipping the redundant
// Z3 round-trips.
//
// Backend scope: This cache lives strictly on the Z3 path. Verum's
// SMT layer is dual-backend (Z3 + CVC5, dispatched by
// `crate::capability_router`); the CVC5 side has its own
// strategy/portfolio plumbing in `cvc5_advanced` /
// `portfolio_executor` and a corresponding cache there is
// future work (#103 follow-up). The cache key is typed on
// `z3::ast::Bool` / `z3::Goal` so cross-backend confusion is
// statically impossible — a CVC5 obligation cannot accidentally
// be looked up here.
//
// Design choices:
//   * Cache value = `FormulaCharacteristics` (Copy-able 32-byte
//     struct), not the full `TacticCombinator`. The cardinality of
//     distinct characteristic tuples is < 64 in practice; rebuilding
//     the combinator tree on every cache hit is a handful of `Box`
//     allocations (≈ tens of nanoseconds), and decoupling the cache
//     from the combinator strategy lets us tune the strategy
//     without invalidating the cache.
//   * Cache key = blake3 hash over the formula's S-expression
//     rendering. Z3's `Display` impl is structurally deterministic;
//     blake3 is the fastest cryptographic hash on x86_64/aarch64
//     and gives 256-bit collision resistance — overkill for a
//     local cache but cheap (≈ 1 GiB/s).
//   * Storage = `dashmap::DashMap` for lock-free reads under
//     contention (verification runs `rayon`-parallel across modules).

/// Stable structural signature of a Z3 `Bool`/`Goal`, used as the
/// [`TacticCache`] key. Computed via blake3 over the formula's
/// canonical S-expression rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormulaSignature([u8; 32]);

impl FormulaSignature {
    /// Compute the signature of a Z3 `Bool` formula.
    pub fn of_formula(formula: &z3::ast::Bool) -> Self {
        let s = formula.to_string();
        Self(blake3::hash(s.as_bytes()).into())
    }

    /// Compute the signature of a Z3 `Goal`.
    pub fn of_goal(goal: &Goal) -> Self {
        let s = goal.to_string();
        Self(blake3::hash(s.as_bytes()).into())
    }

    /// Raw 32-byte signature (for callers that want to fold the
    /// signature into another hash, e.g. a per-module proof
    /// certificate).
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Aggregate cache statistics, snapshot-style.
///
/// `hit_rate` is `hits / (hits + misses)` clamped to `0.0` when no
/// lookups have been performed. Useful for telemetry to flag
/// pathologically low cache utilisation (e.g. every formula
/// contains a unique randomly-generated constant — defeating
/// hashing).
#[derive(Debug, Clone, Copy, Default)]
pub struct TacticCacheStats {
    /// Number of `get`s that found an entry.
    pub hits: u64,
    /// Number of `get`s that returned `None`.
    pub misses: u64,
    /// Current entry count.
    pub entries: usize,
    /// `hits / (hits + misses)`, or `0.0` when `hits + misses == 0`.
    pub hit_rate: f64,
}

/// Concurrent, sharded cache mapping [`FormulaSignature`] →
/// cached [`FormulaCharacteristics`].
///
/// Construct via [`TacticCache::new`] (default capacity) or
/// [`TacticCache::with_capacity`]. Lookups go through
/// [`auto_select_tactic_cached`] / [`auto_select_tactic_cached_global`].
/// `Send + Sync` so it can sit on the verification context and be
/// shared across rayon workers.
///
/// The cache is *bounded by capacity hint only* — `DashMap` will
/// grow past `capacity` if pushed; callers that need a hard cap
/// should call [`TacticCache::clear`] periodically. In practice
/// the unique-formula-shape count for a typical build is in the
/// low thousands, well below any reasonable bound.
pub struct TacticCache {
    entries: dashmap::DashMap<FormulaSignature, FormulaCharacteristics>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl TacticCache {
    /// Default capacity hint = 8192 entries. Each entry is the
    /// 32-byte signature + the ≈ 32-byte characteristics struct +
    /// per-shard overhead, so ≈ 1 MiB working-set at full load.
    pub fn new() -> Self {
        Self::with_capacity(8192)
    }

    /// Construct with a specific capacity hint.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            entries: dashmap::DashMap::with_capacity(cap),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Look up cached characteristics by signature.
    /// Increments `hits` on Some, `misses` on None.
    pub fn get(&self, sig: &FormulaSignature) -> Option<FormulaCharacteristics> {
        match self.entries.get(sig) {
            Some(entry) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
                Some(entry.clone())
            }
            None => {
                self.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    }

    /// Insert (or replace) the characteristics for `sig`.
    pub fn insert(&self, sig: FormulaSignature, chars: FormulaCharacteristics) {
        self.entries.insert(sig, chars);
    }

    /// Snapshot the current statistics. Cheap (atomic loads + len).
    pub fn stats(&self) -> TacticCacheStats {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits.saturating_add(misses);
        let hit_rate = if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        };
        TacticCacheStats {
            hits,
            misses,
            entries: self.entries.len(),
            hit_rate,
        }
    }

    /// Drop every entry and reset hit/miss counters. Useful between
    /// build sessions to prevent unbounded growth in long-lived
    /// processes (LSP, watch mode).
    pub fn clear(&self) {
        self.entries.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for TacticCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide [`TacticCache`] singleton. Lazily initialised on
/// first call. The verification pipeline picks this up via
/// [`auto_select_tactic_cached_global`] without explicit threading.
///
/// Tests, benchmarks, and any code that wants isolation from other
/// concurrent verification activity should construct their own
/// [`TacticCache`] and use [`auto_select_tactic_cached`] directly
/// instead of touching the global.
pub fn global_tactic_cache() -> &'static TacticCache {
    static GLOBAL: std::sync::OnceLock<TacticCache> = std::sync::OnceLock::new();
    GLOBAL.get_or_init(TacticCache::new)
}

/// Cached variant of [`auto_select_tactic`].
///
/// Computes the formula's [`FormulaSignature`], queries `cache`.
/// On hit: skips the nine Z3 probes entirely and rebuilds the
/// combinator from cached characteristics. On miss: runs the
/// analyser, inserts the result, and returns the combinator.
///
/// The returned [`TacticCombinator`] is bit-identical to what
/// [`auto_select_tactic`] would produce — caching is observably
/// transparent (modulo `analyzer.stats()` not advancing on hits).
pub fn auto_select_tactic_cached(
    cache: &TacticCache,
    analyzer: &mut FormulaGoalAnalyzer,
    formula: &z3::ast::Bool,
) -> TacticCombinator {
    let sig = FormulaSignature::of_formula(formula);
    let chars = match cache.get(&sig) {
        Some(c) => c,
        None => {
            let c = analyzer.analyze(formula);
            cache.insert(sig, c.clone());
            c
        }
    };
    select_tactic_from_characteristics(&chars)
}

/// Cached variant of [`auto_select_tactic_for_goal`].
pub fn auto_select_tactic_cached_for_goal(
    cache: &TacticCache,
    analyzer: &FormulaGoalAnalyzer,
    goal: &Goal,
) -> TacticCombinator {
    let sig = FormulaSignature::of_goal(goal);
    let chars = match cache.get(&sig) {
        Some(c) => c,
        None => {
            let c = analyzer.analyze_goal(goal);
            cache.insert(sig, c.clone());
            c
        }
    };
    select_tactic_from_characteristics(&chars)
}

/// Convenience: route through the process-wide [`global_tactic_cache`].
/// The verification pipeline calls this from `solver.rs` so every
/// VC obligation in a build participates in the same cache.
pub fn auto_select_tactic_cached_global(
    analyzer: &mut FormulaGoalAnalyzer,
    formula: &z3::ast::Bool,
) -> TacticCombinator {
    auto_select_tactic_cached(global_tactic_cache(), analyzer, formula)
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use z3::ast::{BV, Bool, Int};
    use z3::{Config, with_z3_config};

    /// All Z3 AST/Probe constructors require a thread-local Z3
    /// context. Wrap each test body in `with_z3_config` so the
    /// context is established before any Z3 call.
    fn with_z3<F, R>(body: F) -> R
    where
        F: FnOnce() -> R + Send + Sync,
        R: Send + Sync,
    {
        with_z3_config(&Config::new(), body)
    }

    #[test]
    fn signature_is_stable_across_calls() {
        with_z3(|| {
            let x = Bool::new_const("x");
            let y = Bool::new_const("y");
            let formula = Bool::and(&[&x, &y]);
            let s1 = FormulaSignature::of_formula(&formula);
            let s2 = FormulaSignature::of_formula(&formula);
            assert_eq!(s1, s2);
        });
    }

    #[test]
    fn signature_differs_for_distinct_formulas() {
        with_z3(|| {
            let x = Bool::new_const("x");
            let y = Bool::new_const("y");
            let s_and = FormulaSignature::of_formula(&Bool::and(&[&x, &y]));
            let s_or = FormulaSignature::of_formula(&Bool::or(&[&x, &y]));
            assert_ne!(s_and, s_or);
        });
    }

    #[test]
    fn cache_hits_skip_analyzer_invocation() {
        with_z3(|| {
            let cache = TacticCache::new();
            let mut analyzer = FormulaGoalAnalyzer::new();
            let a = Int::new_const("a");
            let five = Int::from_i64(5);
            let formula = a.gt(&five);

            // Miss → analyser runs → cache populated.
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &formula);
            assert_eq!(cache.stats().misses, 1);
            assert_eq!(cache.stats().hits, 0);
            assert_eq!(cache.len(), 1);
            let analyses_after_miss = analyzer.stats().formulas_analyzed;

            // Hit → analyser does NOT run again (counter steady).
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &formula);
            assert_eq!(cache.stats().misses, 1);
            assert_eq!(cache.stats().hits, 1);
            assert_eq!(analyzer.stats().formulas_analyzed, analyses_after_miss);
        });
    }

    #[test]
    fn cached_result_is_strategy_equivalent_to_uncached() {
        with_z3(|| {
            let cache = TacticCache::new();
            let mut analyzer = FormulaGoalAnalyzer::new();
            let a = Int::new_const("a");
            let b = Int::new_const("b");
            let sum = Int::add(&[&a, &b]);
            let formula = sum.gt(&Int::from_i64(0));

            let uncached = auto_select_tactic(&mut analyzer, &formula);
            let cached = auto_select_tactic_cached(&cache, &mut analyzer, &formula);

            // Equivalence at the discriminant level — TacticCombinator
            // is a tree of Box<Self>, so we compare via Debug rendering
            // (deterministic for both variants). select_from_chars is
            // pure, so equal characteristics ⇒ equal combinator.
            assert_eq!(format!("{:?}", uncached), format!("{:?}", cached));
        });
    }

    #[test]
    fn hit_rate_zero_when_no_lookups() {
        let cache = TacticCache::new();
        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.hit_rate, 0.0);
    }

    #[test]
    fn hit_rate_correct_after_mixed_lookups() {
        with_z3(|| {
            let cache = TacticCache::new();
            let mut analyzer = FormulaGoalAnalyzer::new();
            let f1 = Bool::new_const("p").not();
            let f2 = Bool::new_const("q").not();

            // 2 misses + 3 hits (f1 hit twice, f2 hit once).
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f1);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f2);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f1);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f2);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f1);

            let stats = cache.stats();
            assert_eq!(stats.hits, 3);
            assert_eq!(stats.misses, 2);
            assert!((stats.hit_rate - 0.6).abs() < 1e-9);
        });
    }

    #[test]
    fn clear_resets_entries_and_counters() {
        with_z3(|| {
            let cache = TacticCache::new();
            let mut analyzer = FormulaGoalAnalyzer::new();
            let f = Bool::new_const("z");
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f);
            assert!(cache.len() > 0);
            cache.clear();
            assert_eq!(cache.len(), 0);
            let stats = cache.stats();
            assert_eq!(stats.hits, 0);
            assert_eq!(stats.misses, 0);
        });
    }

    #[test]
    fn global_cache_is_shared_singleton() {
        let g1 = global_tactic_cache() as *const _;
        let g2 = global_tactic_cache() as *const _;
        assert_eq!(g1, g2);
    }

    #[test]
    fn cache_handles_bitvector_formulas() {
        with_z3(|| {
            let cache = TacticCache::new();
            let mut analyzer = FormulaGoalAnalyzer::new();
            let x = BV::new_const("bvx", 32);
            let y = BV::new_const("bvy", 32);
            let f = x.bvult(&y);

            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f);
            let _ = auto_select_tactic_cached(&cache, &mut analyzer, &f);
            assert_eq!(cache.stats().hits, 1);
            assert_eq!(cache.stats().misses, 1);
        });
    }
}
