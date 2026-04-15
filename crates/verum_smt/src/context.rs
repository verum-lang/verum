//! Z3 context management and configuration.
//!
//! This module provides a safe wrapper around Z3's context and solver,
//! with configuration options for timeout, memory limits, and more.

use std::sync::Arc;
use std::time::Duration;

/// Z3 context wrapper with configuration.
///
/// The context manages Z3's internal state and provides access to the solver.
/// It's designed to be cheap to clone (uses Arc internally).
#[derive(Debug, Clone)]
pub struct Context {
    inner: Arc<ContextInner>,
}

#[derive(Debug)]
struct ContextInner {
    config: ContextConfig,
    /// Optional shared routing-stats collector.
    ///
    /// When set, every `Context::check(...)` call records the routing
    /// choice (`Z3Only`) plus its outcome and elapsed time into the
    /// collector. This is how `verum build --smt-stats` learns about
    /// real solver work: the compiler installs the session's shared
    /// `Arc<RoutingStats>` on the Context at construction time.
    ///
    /// `None` = no telemetry overhead, existing behavior unchanged.
    routing_stats: Option<Arc<crate::routing_stats::RoutingStats>>,
}

impl Context {
    /// Create a new Z3 context with default configuration.
    pub fn new() -> Self {
        Self::with_config(ContextConfig::default())
    }

    /// Create a new Z3 context with custom configuration.
    pub fn with_config(config: ContextConfig) -> Self {
        // Z3 0.19.2 uses a global context, so we just store config
        Self {
            inner: Arc::new(ContextInner {
                config,
                routing_stats: None,
            }),
        }
    }

    /// Install a shared routing-stats collector on this context.
    ///
    /// Returns a new `Context` value (internal state is in an Arc, so
    /// this is cheap). Once installed, every call to
    /// [`Context::check`] records a Z3-routed query into the collector
    /// — this is how the compiler's `session.routing_stats()` learns
    /// about real solver work.
    pub fn with_routing_stats(
        self,
        stats: Arc<crate::routing_stats::RoutingStats>,
    ) -> Self {
        let inner = ContextInner {
            config: self.inner.config.clone(),
            routing_stats: Some(stats),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Access the routing-stats handle, if one was installed.
    pub fn routing_stats(&self) -> Option<&Arc<crate::routing_stats::RoutingStats>> {
        self.inner.routing_stats.as_ref()
    }

    /// Get the configuration.
    pub fn config(&self) -> &ContextConfig {
        &self.inner.config
    }

    /// Create a new solver instance.
    pub fn solver(&self) -> z3::Solver {
        let solver = z3::Solver::new();

        // Apply timeout if configured
        if let Some(timeout) = self.inner.config.timeout {
            let mut params = z3::Params::new();
            params.set_u32("timeout", timeout.as_millis() as u32);
            solver.set_params(&params);
        }

        solver
    }

    /// Create a new optimizer instance.
    pub fn optimizer(&self) -> z3::Optimize {
        z3::Optimize::new()
    }

    /// Push a new scope on the solver stack.
    pub fn push(&self, solver: &z3::Solver) {
        solver.push();
    }

    /// Pop a scope from the solver stack.
    pub fn pop(&self, solver: &z3::Solver) {
        solver.pop(1);
    }

    /// Reset the solver state.
    pub fn reset(&self, solver: &z3::Solver) {
        solver.reset();
    }

    /// Check if the solver assertions are satisfiable.
    ///
    /// When a routing-stats collector is installed on this context,
    /// records the call as `SolverChoice::Z3Only` plus the outcome and
    /// elapsed time, so `verum smt-stats` reflects real work.
    pub fn check(&self, solver: &z3::Solver) -> z3::SatResult {
        let start = std::time::Instant::now();
        let verdict = solver.check();
        self.record_check(&verdict, start.elapsed());
        verdict
    }

    /// Check satisfiability with assumptions.
    ///
    /// Also records into the installed routing-stats collector (if any).
    pub fn check_assumptions(
        &self,
        solver: &z3::Solver,
        assumptions: &[z3::ast::Bool],
    ) -> z3::SatResult {
        let start = std::time::Instant::now();
        let verdict = solver.check_assumptions(assumptions);
        self.record_check(&verdict, start.elapsed());
        verdict
    }

    /// Internal: record one Z3 `check()` outcome into the shared
    /// routing-stats collector, if present. No-op otherwise.
    fn record_check(&self, verdict: &z3::SatResult, elapsed: std::time::Duration) {
        let Some(stats) = self.inner.routing_stats.as_ref() else {
            return;
        };
        use crate::capability_router::SolverChoice;
        use crate::portfolio_executor::SolverVerdict;
        use crate::routing_stats::TheoryClass;

        // Contract / refinement verification uses quantified first-order
        // logic — bucket as Quantified. A future enhancement can
        // classify per-goal from the translated assertions.
        let theory = TheoryClass::Quantified;
        stats.record_routing(
            &SolverChoice::Z3Only {
                confidence: 1.0,
                reason: "verum Z3 solver".to_string(),
            },
            theory,
        );
        let smt_verdict = match verdict {
            z3::SatResult::Sat => SolverVerdict::Sat,
            z3::SatResult::Unsat => SolverVerdict::Unsat,
            z3::SatResult::Unknown => SolverVerdict::Unknown {
                reason: "z3 returned unknown".to_string(),
            },
        };
        stats.record_outcome(theory, &smt_verdict, elapsed);
    }

    /// Get the model from the solver (if SAT).
    pub fn get_model(&self, solver: &z3::Solver) -> Option<z3::Model> {
        solver.get_model()
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for the Z3 context.
#[derive(Debug, Clone)]
pub struct ContextConfig {
    /// Timeout for SMT queries (default: 30 seconds)
    pub timeout: Option<Duration>,

    /// Memory limit in megabytes (default: None)
    pub memory_limit_mb: Option<u64>,

    /// Enable model generation for counterexamples (default: true)
    pub model_generation: bool,

    /// Enable unsat core generation (default: false)
    pub unsat_core: bool,

    /// Enable proof generation (default: false)
    pub proof_generation: bool,

    /// Random seed for reproducibility (default: None)
    pub random_seed: Option<u32>,

    /// Enable simplification before solving (default: true)
    pub simplify: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            memory_limit_mb: None,
            model_generation: true,
            unsat_core: false,
            proof_generation: false,
            random_seed: None,
            simplify: true,
        }
    }
}

impl ContextConfig {
    /// Create a configuration for fast verification (shorter timeout).
    pub fn fast() -> Self {
        Self {
            timeout: Some(Duration::from_secs(5)),
            simplify: true,
            ..Default::default()
        }
    }

    /// Create a configuration for thorough verification (longer timeout).
    pub fn thorough() -> Self {
        Self {
            timeout: Some(Duration::from_secs(120)),
            simplify: true,
            ..Default::default()
        }
    }

    /// Create a configuration for debugging with proof generation.
    pub fn debug() -> Self {
        Self {
            timeout: None,
            model_generation: true,
            unsat_core: true,
            proof_generation: true,
            simplify: false,
            ..Default::default()
        }
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Disable timeout.
    pub fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Set memory limit.
    pub fn with_memory_limit(mut self, mb: u64) -> Self {
        self.memory_limit_mb = Some(mb);
        self
    }

    /// Enable model generation.
    pub fn with_models(mut self) -> Self {
        self.model_generation = true;
        self
    }

    /// Enable unsat core generation.
    pub fn with_unsat_core(mut self) -> Self {
        self.unsat_core = true;
        self
    }

    /// Set random seed for reproducibility.
    pub fn with_seed(mut self, seed: u32) -> Self {
        self.random_seed = Some(seed);
        self
    }
}

/// Statistics collected from the Z3 solver.
#[derive(Debug, Clone, Default)]
pub struct SolverStats {
    /// Total time spent in the solver (milliseconds)
    pub time_ms: u64,

    /// Number of solver checks performed
    pub num_checks: u64,

    /// Number of satisfiable results
    pub num_sat: u64,

    /// Number of unsatisfiable results
    pub num_unsat: u64,

    /// Number of unknown results
    pub num_unknown: u64,

    /// Number of timeouts
    pub num_timeouts: u64,

    /// Peak memory usage (bytes)
    pub peak_memory_bytes: u64,
}

impl SolverStats {
    /// Create a new empty statistics object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a satisfiable result.
    pub fn record_sat(&mut self, time_ms: u64) {
        self.num_checks += 1;
        self.num_sat += 1;
        self.time_ms += time_ms;
    }

    /// Record an unsatisfiable result.
    pub fn record_unsat(&mut self, time_ms: u64) {
        self.num_checks += 1;
        self.num_unsat += 1;
        self.time_ms += time_ms;
    }

    /// Record an unknown result.
    pub fn record_unknown(&mut self, time_ms: u64) {
        self.num_checks += 1;
        self.num_unknown += 1;
        self.time_ms += time_ms;
    }

    /// Record a timeout.
    pub fn record_timeout(&mut self, time_ms: u64) {
        self.num_checks += 1;
        self.num_timeouts += 1;
        self.time_ms += time_ms;
    }

    /// Update peak memory usage.
    pub fn update_memory(&mut self, bytes: u64) {
        self.peak_memory_bytes = self.peak_memory_bytes.max(bytes);
    }

    /// Get average time per check.
    pub fn avg_time_ms(&self) -> f64 {
        if self.num_checks == 0 {
            0.0
        } else {
            self.time_ms as f64 / self.num_checks as f64
        }
    }

    /// Get success rate (sat + unsat / total).
    pub fn success_rate(&self) -> f64 {
        if self.num_checks == 0 {
            0.0
        } else {
            (self.num_sat + self.num_unsat) as f64 / self.num_checks as f64
        }
    }
}
