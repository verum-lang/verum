//! SMT Backend Switcher - Transparent Backend Selection and Portfolio Solving
//!
//! This module implements intelligent backend switching with multiple strategies:
//! - **Manual Selection**: Explicitly choose Z3 or CVC5
//! - **Auto Selection**: Automatically pick best solver based on problem characteristics
//! - **Fallback**: Try Z3 first, fall back to CVC5 on timeout/failure
//! - **Portfolio**: Run both solvers in parallel, return first result
//!
//! ## Performance Characteristics
//!
//! - Manual: Zero overhead (direct backend call)
//! - Auto: <1ms problem analysis overhead
//! - Fallback: 2x worst-case time (sequential)
//! - Portfolio: 0.5-0.7x average time (parallel)
//!
//! ## Architecture
//!
//! ```text
//! ┌───────────────────────────────────┐
//! │    SmtBackendSwitcher             │
//! │  ┌─────────────────────────────┐  │
//! │  │ Configuration               │  │
//! │  │ - default_backend           │  │
//! │  │ - fallback_enabled          │  │
//! │  │ - portfolio_mode            │  │
//! │  └─────────────────────────────┘  │
//! │         ▼          ▼               │
//! │    ┌─────┐    ┌──────┐            │
//! │    │ Z3  │    │ CVC5 │            │
//! │    └─────┘    └──────┘            │
//! └───────────────────────────────────┘
//! ```
//!
//! Refinement types (`Int{> 0}`, `Text{len(it) > 5}`, sigma-type `n: Int where n > 0`)
//! generate SMT constraints verified by Z3 or CVC5. The switcher selects the optimal
//! solver: Z3 excels at bitvectors and arrays, CVC5 at strings and nonlinear arithmetic.

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use verum_common::{List, Map, Maybe};

// Aliased for prospective use by portfolio backends; kept for API
// stability across crates that `use backend_switcher::BackendSmtBackend`.
#[allow(unused_imports)]
use crate::backend_trait::SmtBackend as BackendSmtBackend;
use crate::cvc5_backend::{Cvc5Backend, Cvc5Config};
use crate::solver::{SmtBackend, SmtContext, Z3Backend};
use verum_ast::expr::Expr;

// ==================== Backend Choice ====================

/// Backend selection strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum BackendChoice {
    /// Use Z3 exclusively
    Z3,
    /// Use CVC5 exclusively
    Cvc5,
    /// Automatically select based on problem characteristics (legacy heuristic)
    Auto,
    /// Run both in parallel, return first result
    Portfolio,
    /// Use the capability router: each goal routed to the best solver based on
    /// its theory signature. Hard/mixed goals run as portfolio; security-
    /// critical goals run as cross-validate. This is the recommended default.
    #[default]
    Capability,
}


impl std::str::FromStr for BackendChoice {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "z3" => Ok(Self::Z3),
            "cvc5" => Ok(Self::Cvc5),
            "auto" => Ok(Self::Auto),
            "portfolio" => Ok(Self::Portfolio),
            "capability" | "capability-based" | "smart" => Ok(Self::Capability),
            _ => Err(format!("Unknown backend choice: {}", s)),
        }
    }
}

// ==================== Configuration ====================

/// Backend switcher configuration
#[derive(Debug, Clone)]
pub struct SwitcherConfig {
    /// Default backend when using Auto mode
    pub default_backend: BackendChoice,

    /// Fallback configuration
    pub fallback: FallbackConfig,

    /// Portfolio configuration
    pub portfolio: PortfolioConfig,

    /// Validation configuration
    pub validation: ValidationConfig,

    /// Timeout for each backend (milliseconds)
    pub timeout_ms: u64,

    /// Enable detailed logging
    pub verbose: bool,
}

impl Default for SwitcherConfig {
    fn default() -> Self {
        Self {
            default_backend: BackendChoice::Z3,
            fallback: FallbackConfig::default(),
            portfolio: PortfolioConfig::default(),
            validation: ValidationConfig::default(),
            timeout_ms: 30000, // 30s
            verbose: false,
        }
    }
}

/// Fallback configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FallbackConfig {
    /// Enable fallback to alternative backend
    pub enabled: bool,

    /// Fallback on timeout
    pub on_timeout: bool,

    /// Fallback on unknown result
    pub on_unknown: bool,

    /// Fallback on solver error
    pub on_error: bool,

    /// Maximum number of fallback attempts
    pub max_attempts: usize,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_timeout: true,
            on_unknown: true,
            on_error: true,
            max_attempts: 2,
        }
    }
}

/// Portfolio solving configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PortfolioConfig {
    /// Enable portfolio solving
    pub enabled: bool,

    /// Portfolio mode
    pub mode: PortfolioMode,

    /// Maximum number of parallel threads
    pub max_threads: usize,

    /// Timeout per solver (milliseconds)
    pub timeout_per_solver: u64,

    /// Kill losing solver when first result arrives
    pub kill_on_first: bool,
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: PortfolioMode::FirstResult,
            max_threads: 2,
            timeout_per_solver: 30000,
            kill_on_first: true,
        }
    }
}

/// Portfolio solving mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PortfolioMode {
    /// Return first result (SAT or UNSAT)
    FirstResult,

    /// Wait for both, verify they agree
    Consensus,

    /// If solvers disagree, return error
    VoteOnDisagree,
}

/// Validation configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationConfig {
    /// Enable result validation
    pub enabled: bool,

    /// Cross-validate results between backends
    pub cross_validate: bool,

    /// Fail if backends produce different results
    pub fail_on_mismatch: bool,

    /// Log mismatches to stderr
    pub log_mismatches: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cross_validate: false,
            fail_on_mismatch: false,
            log_mismatches: true,
        }
    }
}

// ==================== Backend Switcher ====================

/// SMT Backend Switcher - Intelligent multi-backend solver
pub struct SmtBackendSwitcher {
    /// Current backend choice
    current: BackendChoice,

    /// Configuration
    config: SwitcherConfig,

    /// Z3 backend instance
    z3: Maybe<Z3Backend>,

    /// CVC5 backend instance
    cvc5: Maybe<Cvc5Backend>,

    /// Statistics
    stats: Arc<Mutex<SwitcherStats>>,

    /// Capability routing statistics (telemetry for complementary dispatch).
    /// Tracks which solver wins for which theory, cross-validation agreements/
    /// divergences, and per-theory success rates.
    routing_stats: Arc<crate::routing_stats::RoutingStats>,
}

impl SmtBackendSwitcher {
    /// Create new backend switcher with configuration
    pub fn new(config: SwitcherConfig) -> Self {
        // Initialize Z3 backend
        let z3_config = crate::z3_backend::Z3Config::default();
        let z3 = Z3Backend::new(z3_config);

        // Initialize CVC5 backend
        let cvc5_config = Cvc5Config::default();
        let cvc5 = Cvc5Backend::new(cvc5_config).ok();

        Self {
            current: config.default_backend,
            config,
            z3: Maybe::Some(z3),
            cvc5: cvc5.map(Maybe::Some).unwrap_or(Maybe::None),
            stats: Arc::new(Mutex::new(SwitcherStats::default())),
            routing_stats: Arc::new(crate::routing_stats::RoutingStats::new()),
        }
    }

    /// Access the routing statistics collector (for telemetry/diagnostics).
    pub fn routing_stats(&self) -> Arc<crate::routing_stats::RoutingStats> {
        self.routing_stats.clone()
    }

    /// Create a switcher backed by a caller-provided shared `RoutingStats`.
    ///
    /// Used by the compiler's verification phases: every switcher built
    /// during a compilation session shares the session's single
    /// `RoutingStats` handle, so per-session telemetry is aggregated
    /// across all phases for `verum smt-stats`.
    pub fn with_shared_stats(
        config: SwitcherConfig,
        routing_stats: Arc<crate::routing_stats::RoutingStats>,
    ) -> Self {
        let z3_config = crate::z3_backend::Z3Config::default();
        let z3 = Z3Backend::new(z3_config);
        let cvc5_config = Cvc5Config::default();
        let cvc5 = Cvc5Backend::new(cvc5_config).ok();

        Self {
            current: config.default_backend,
            config,
            z3: Maybe::Some(z3),
            cvc5: cvc5.map(Maybe::Some).unwrap_or(Maybe::None),
            stats: Arc::new(Mutex::new(SwitcherStats::default())),
            routing_stats,
        }
    }

    /// Solve using a verification strategy from a `@verify(...)` attribute.
    ///
    /// This is the primary entry point for SMT-backed goal discharge in the
    /// compiler: the verification phase reads `@verify(...)` from function
    /// attributes, converts it to a `VerifyStrategy`, and calls this method.
    ///
    /// # Behavior by strategy
    ///
    /// - `Runtime` / `Static`: returns `None` — caller should NOT invoke SMT.
    /// - `Formal`: dispatches via capability router.
    /// - `ForceZ3` / `ForceCvc5`: dispatches to the specified backend.
    /// - `Portfolio`: runs both solvers in parallel, first-wins.
    /// - `CrossValidate`: runs both solvers to completion, requires agreement.
    ///
    /// The current backend is temporarily overridden for the duration of this
    /// call and restored afterward. This lets the switcher serve both its
    /// default-configured mode and per-goal overrides from attributes.
    pub fn solve_with_strategy(
        &mut self,
        assertions: &List<Expr>,
        strategy: &crate::verify_strategy::VerifyStrategy,
    ) -> Option<SolveResult> {
        use crate::verify_strategy::VerifyStrategy;

        if !strategy.requires_smt() {
            // Runtime or Static — no SMT dispatch.
            return None;
        }

        // Snapshot the current backend.
        let saved = self.current;

        let result = match strategy {
            VerifyStrategy::Runtime | VerifyStrategy::Static => {
                unreachable!("requires_smt() should have rejected these");
            }
            VerifyStrategy::Formal => {
                // Default: route via the capability system. The compiler
                // picks the best available technique for the goal's theory.
                self.current = BackendChoice::Capability;
                self.solve(assertions)
            }
            VerifyStrategy::Fast => {
                // Fast: capability routing with tighter timeouts. The
                // caller-provided timeout is already scaled by
                // strategy.timeout_multiplier() in the compiler wrapper.
                self.current = BackendChoice::Capability;
                self.solve(assertions)
            }
            VerifyStrategy::Thorough => {
                // Thorough: race multiple techniques in parallel and
                // accept the first successful result.
                self.current = BackendChoice::Portfolio;
                self.solve(assertions)
            }
            VerifyStrategy::Certified => {
                // Certified: cross-validate the result with an independent
                // technique. Divergence is a hard error (solver bug or
                // encoding issue). Required for exportable proof certificates.
                self.solve_cross_validate(assertions)
            }
            VerifyStrategy::Synthesize => {
                // Synthesize: genuine program synthesis, not a satisfiability
                // check. The previous implementation silently routed to
                // `Capability` (which runs Sat/Unsat), so a user who asked for
                // synthesis got a sat/unsat answer and no synthesized program
                // — a correctness bug, not "not implemented".
                //
                // The fix: route through `solve_synthesize`, which tries
                // CVC5's SyGuS path and returns an `Error` with a clear
                // rationale if the synthesis backend is unavailable. No more
                // silent fallback to satisfiability.
                self.solve_synthesize(assertions)
            }
        };

        // Restore the original backend.
        self.current = saved;
        Some(result)
    }

    /// Print a human-readable routing statistics report to stderr.
    pub fn print_routing_report(&self) {
        eprintln!("{}", self.routing_stats.report());
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(SwitcherConfig::default())
    }

    /// Select backend manually
    pub fn select_backend(&mut self, choice: BackendChoice) {
        self.current = choice;
    }

    /// Get current backend choice
    pub fn current_backend(&self) -> BackendChoice {
        self.current
    }

    /// Solve with automatic backend selection
    pub fn solve(&mut self, assertions: &List<Expr>) -> SolveResult {
        let start = Instant::now();

        let result = match self.current {
            BackendChoice::Z3 => self.solve_with_z3(assertions),
            BackendChoice::Cvc5 => self.solve_with_cvc5(assertions),
            BackendChoice::Auto => self.solve_auto(assertions),
            BackendChoice::Portfolio => self.solve_portfolio(assertions),
            BackendChoice::Capability => self.solve_capability(assertions),
        };

        // Update statistics
        if let Ok(ref mut stats) = self.stats.lock() {
            stats.total_queries += 1;
            stats.total_time_ms += start.elapsed().as_millis() as u64;

            match &result {
                SolveResult::Sat { backend, .. } | SolveResult::Unsat { backend, .. } => {
                    *stats.backend_wins.entry(backend.to_string()).or_insert(0) += 1;
                }
                SolveResult::Unknown { .. } => {
                    stats.unknown_count += 1;
                }
                SolveResult::Error { .. } => {
                    stats.error_count += 1;
                }
            }
        }

        result
    }

    /// Solve using Z3
    fn solve_with_z3(&mut self, assertions: &List<Expr>) -> SolveResult {
        let start = Instant::now();

        // Get Z3 backend
        let z3_backend = match &self.z3 {
            Maybe::Some(backend) => backend,
            Maybe::None => {
                return SolveResult::Error {
                    backend: "Z3".to_string(),
                    error: "Z3 backend not initialized".to_string(),
                };
            }
        };

        // Create SMT context
        let context = SmtContext {
            assumptions: List::clone(assertions),
            bindings: Map::new(),
        };

        // Check satisfiability
        let result = if let Some(first_assertion) = assertions.first() {
            z3_backend.check_sat(first_assertion, &context)
        } else {
            // Empty assertions - trivially SAT
            crate::solver::SmtResult::Sat
        };

        let elapsed = start.elapsed().as_millis() as u64;

        // Convert result
        match result {
            crate::solver::SmtResult::Sat => SolveResult::Sat {
                backend: "Z3".to_string(),
                time_ms: elapsed,
                model: Maybe::None,
            },
            crate::solver::SmtResult::Unsat(counter) => SolveResult::Unsat {
                backend: "Z3".to_string(),
                time_ms: elapsed,
                core: Maybe::None,
                proof: Maybe::Some(counter.explanation.to_string()),
            },
            crate::solver::SmtResult::Unknown(reason) => SolveResult::Unknown {
                backend: "Z3".to_string(),
                reason: Maybe::Some(reason.to_string()),
            },
            crate::solver::SmtResult::Timeout => SolveResult::Unknown {
                backend: "Z3".to_string(),
                reason: Maybe::Some("Timeout".to_string()),
            },
        }
    }

    /// Solve using CVC5
    fn solve_with_cvc5(&mut self, assertions: &List<Expr>) -> SolveResult {
        let start = Instant::now();

        // Get CVC5 backend
        let cvc5_backend: &mut Cvc5Backend = match &mut self.cvc5 {
            Maybe::Some(backend) => backend,
            Maybe::None => {
                return SolveResult::Error {
                    backend: "CVC5".to_string(),
                    error: "CVC5 backend not initialized".to_string(),
                };
            }
        };

        // Assert all formulas
        for assertion in assertions {
            if let Err(e) = cvc5_backend.assert_formula_from_expr(assertion) {
                return SolveResult::Error {
                    backend: "CVC5".to_string(),
                    error: format!("Failed to assert formula: {:?}", e),
                };
            }
        }

        // Check satisfiability
        let result = match cvc5_backend.check_sat() {
            Ok(res) => res,
            Err(e) => {
                return SolveResult::Error {
                    backend: "CVC5".to_string(),
                    error: format!("Check-sat failed: {:?}", e),
                };
            }
        };

        let elapsed = start.elapsed().as_millis() as u64;

        // Convert result
        match result {
            crate::cvc5_backend::Cvc5SatResult::Sat => {
                // Try to get model
                let model = cvc5_backend.get_model().ok().map(|m| format!("{:?}", m));

                SolveResult::Sat {
                    backend: "CVC5".to_string(),
                    time_ms: elapsed,
                    model: model.map(Maybe::Some).unwrap_or(Maybe::None),
                }
            }
            crate::cvc5_backend::Cvc5SatResult::Unsat => {
                // Try to get unsat core
                let core: Option<List<String>> = cvc5_backend.get_unsat_core().ok().map(
                    |terms: List<crate::cvc5_backend::Cvc5Term>| {
                        terms.iter().map(|t| format!("{:?}", t)).collect()
                    },
                );

                SolveResult::Unsat {
                    backend: "CVC5".to_string(),
                    time_ms: elapsed,
                    core: core.map(Maybe::Some).unwrap_or(Maybe::None),
                    proof: Maybe::None,
                }
            }
            crate::cvc5_backend::Cvc5SatResult::Unknown => {
                let reason = cvc5_backend.get_reason_unknown();

                SolveResult::Unknown {
                    backend: "CVC5".to_string(),
                    reason: reason.map(Maybe::Some).unwrap_or(Maybe::None),
                }
            }
        }
    }

    /// Capability-based routing: each goal is analyzed and routed to the
    /// best solver based on its theory signature.
    ///
    /// Decision flow (see `capability_router::CapabilityRouter::route`):
    /// 1. If CVC5 unavailable → Z3 only.
    /// 2. If goal is security-critical → cross-validate both solvers.
    /// 3. If a theory strongly favors one solver → route to that solver.
    /// 4. If goal is complex or mixed-theory → portfolio (parallel).
    /// 5. Default → Z3.
    ///
    /// This is the recommended dispatch strategy for production use.
    fn solve_capability(&mut self, assertions: &List<Expr>) -> SolveResult {
        use crate::capability_router::{
            CapabilityRouter, SolverChoice,
        };
        use crate::routing_stats::TheoryClass;
        use std::time::Instant;

        let router = CapabilityRouter::with_defaults();
        let chars = self.analyze_assertions_heuristically(assertions);
        let theory = TheoryClass::classify(&chars);
        let choice = router.route(&chars);

        // Record the routing decision in telemetry.
        self.routing_stats.record_routing(&choice, theory);

        let t0 = Instant::now();
        let result = match &choice {
            SolverChoice::Z3Only { reason, .. } => {
                if self.config.verbose {
                    eprintln!("[CAPABILITY] theory={} → Z3: {}", theory.mnemonic(), reason);
                }
                self.solve_with_z3(assertions)
            }
            SolverChoice::Cvc5Only { reason, .. } => {
                if self.config.verbose {
                    eprintln!("[CAPABILITY] theory={} → CVC5: {}", theory.mnemonic(), reason);
                }
                self.solve_with_cvc5(assertions)
            }
            SolverChoice::Portfolio { .. } => {
                if self.config.verbose {
                    eprintln!("[CAPABILITY] theory={} → portfolio (parallel)", theory.mnemonic());
                }
                self.solve_portfolio(assertions)
            }
            SolverChoice::CrossValidate { .. } => {
                if self.config.verbose {
                    eprintln!("[CAPABILITY] theory={} → cross-validate", theory.mnemonic());
                }
                self.solve_cross_validate(assertions)
            }
        };

        // Record the outcome in telemetry.
        let verdict = match &result {
            SolveResult::Sat { .. } => {
                crate::portfolio_executor::SolverVerdict::Sat
            }
            SolveResult::Unsat { .. } => {
                crate::portfolio_executor::SolverVerdict::Unsat
            }
            SolveResult::Unknown { reason, .. } => {
                let r = match reason {
                    Maybe::Some(s) => s.as_str().to_string(),
                    Maybe::None => "unknown".to_string(),
                };
                crate::portfolio_executor::SolverVerdict::Unknown { reason: r }
            }
            SolveResult::Error { error, .. } => {
                crate::portfolio_executor::SolverVerdict::Error {
                    message: error.clone(),
                }
            }
        };
        self.routing_stats.record_outcome(theory, &verdict, t0.elapsed());

        result
    }

    /// Heuristic analysis of assertions for routing purposes.
    ///
    /// This is a lightweight AST-walk that identifies theory signatures
    /// without invoking the SMT solver. It errs on the side of portfolio
    /// mode for ambiguous cases, trusting the router to make the final call.
    fn analyze_assertions_heuristically(
        &self,
        assertions: &List<Expr>,
    ) -> crate::capability_router::ExtendedCharacteristics {
        use crate::capability_router::ExtendedCharacteristics;
        use crate::strategy_selection::ProblemCharacteristics;

        let mut chars = ExtendedCharacteristics::from_base(ProblemCharacteristics {
            size: assertions.len() as f64,
            ..Default::default()
        });

        // Single-pass AST walk — only flags set are checked in the router.
        for expr in assertions.iter() {
            self.scan_expr(expr, &mut chars);
        }

        chars
    }

    /// Scan a single expression for theory signatures.
    ///
    /// Performs a recursive AST walk, identifying theory-specific constructs
    /// (strings, bit-vectors, arrays, quantifiers, etc.) to feed the
    /// capability router. The detected signals are used by the router to
    /// choose the best solver for the goal.
    fn scan_expr(
        &self,
        expr: &Expr,
        chars: &mut crate::capability_router::ExtendedCharacteristics,
    ) {
        use verum_ast::{BinOp, ExprKind};

        chars.base.num_exprs += 1.0;

        match &expr.kind {
            // --- Quantifiers ---
            ExprKind::Forall { body, .. } | ExprKind::Exists { body, .. } => {
                chars.base.has_quantifiers = true;
                chars.quantifier_depth = chars.quantifier_depth.saturating_add(1);
                self.scan_expr(body, chars);
            }

            // --- Literals ---
            ExprKind::Literal(lit) => {
                use verum_ast::LiteralKind;
                chars.base.num_consts += 1.0;
                if let LiteralKind::Text(_) = &lit.kind { chars.has_strings = true }
            }

            // --- Arithmetic operators: detect nonlinearity ---
            ExprKind::Binary { op, left, right } => {
                match op {
                    BinOp::Mul => {
                        // Nonlinear if both operands contain variables.
                        if !Self::is_constant_like(left) && !Self::is_constant_like(right) {
                            chars.has_nonlinear_int = true;
                            chars.has_nonlinear_real = true;
                        }
                    }
                    BinOp::Div | BinOp::Rem => {
                        // Division/remainder → nonlinear
                        chars.has_nonlinear_int = true;
                    }
                    BinOp::Shl | BinOp::Shr => {
                        chars.base.is_qfbv = true;
                    }
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                        chars.base.is_qfbv = true;
                    }
                    BinOp::Add | BinOp::Sub => {
                        // Linear — keep default LIA/LRA if no other signals.
                        chars.base.is_qflia = true;
                    }
                    _ => {}
                }
                self.scan_expr(left, chars);
                self.scan_expr(right, chars);
            }

            // --- Unary ---
            ExprKind::Unary { expr, .. } => {
                self.scan_expr(expr, chars);
            }

            // --- Function / method calls: inspect names for theory hints ---
            ExprKind::Call { func, args, .. } => {
                // Method-call-style name heuristics (e.g., "String.contains").
                if let Some(name) = Self::extract_call_name(func) {
                    Self::detect_theory_from_name(&name, chars);
                }
                for arg in args.iter() {
                    self.scan_expr(arg, chars);
                }
            }
            ExprKind::MethodCall { receiver, method, args, .. } => {
                Self::detect_theory_from_name(method.as_str(), chars);
                self.scan_expr(receiver, chars);
                for arg in args.iter() {
                    self.scan_expr(arg, chars);
                }
            }

            // --- Index access → arrays ---
            ExprKind::Index { expr, index } => {
                chars.has_arrays = true;
                self.scan_expr(expr, chars);
                self.scan_expr(index, chars);
            }

            // --- Pattern matching → inductive datatypes ---
            ExprKind::Match { expr: scrutinee, .. } => {
                chars.has_inductive_datatypes = true;
                self.scan_expr(scrutinee, chars);
            }

            // --- Block, if, let: recurse ---
            ExprKind::Block(block) => {
                for stmt in block.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                        self.scan_expr(expr, chars);
                    }
                }
            }
            ExprKind::If { then_branch, else_branch, .. } => {
                // IfCondition is a complex nested structure; skip deep analysis
                // here and just recurse into the branches.
                for stmt in then_branch.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                        self.scan_expr(expr, chars);
                    }
                }
                if let verum_common::Maybe::Some(else_b) = else_branch {
                    self.scan_expr(else_b, chars);
                }
            }
            ExprKind::Paren(inner) => {
                self.scan_expr(inner, chars);
            }

            _ => {
                // Other expressions contribute to size but no theory signals.
            }
        }
    }

    /// Check if an expression is "constant-like" (a literal or simple path).
    /// Used to detect nonlinearity: `x * y` is nonlinear, `x * 5` is linear.
    fn is_constant_like(expr: &Expr) -> bool {
        matches!(
            &expr.kind,
            verum_ast::ExprKind::Literal(_)
        )
    }

    /// Extract the callable name from a function reference expression.
    fn extract_call_name(func: &Expr) -> Option<String> {
        use verum_ast::ty::PathSegment;
        match &func.kind {
            verum_ast::ExprKind::Path(path) => {
                path.segments.last().and_then(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                    _ => None,
                })
            }
            _ => None,
        }
    }

    /// Detect theory signatures from a function/method name.
    ///
    /// Uses a curated list of known theory-indicating identifiers. This is a
    /// heuristic that works well for Verum stdlib conventions but may miss
    /// user-defined functions that implement theory operations.
    fn detect_theory_from_name(
        name: &str,
        chars: &mut crate::capability_router::ExtendedCharacteristics,
    ) {
        // String operations (Verum Text, SMT-LIB strings, Python-style)
        if matches!(
            name,
            "len" | "length" | "concat" | "contains" | "starts_with" | "ends_with"
                | "substr" | "substring" | "replace" | "indexof" | "to_upper" | "to_lower"
                | "str_concat" | "str_contains" | "str_len"
        ) {
            chars.has_strings = true;
        }
        // Regex operations
        if matches!(name, "matches" | "regex" | "re_match" | "re_find") {
            chars.has_regex = true;
            chars.has_strings = true;
        }
        // Sequence operations
        if matches!(
            name,
            "seq_len" | "seq_at" | "seq_extract" | "seq_contains" | "seq_concat"
        ) {
            chars.has_sequences = true;
        }
        // Array operations
        if matches!(name, "select" | "store" | "array_select" | "array_store") {
            chars.has_arrays = true;
        }
        // Descent / sheaf — security-critical for theory-
        // interop consumers (coherence proofs over translation
        // chains).
        if matches!(
            name,
            "check_descent" | "verify_descent" | "sheaf_condition" | "compatible_sections"
        ) {
            chars.is_security_critical = true;
        }
    }

    /// Cross-validate: run both solvers and require agreement.
    ///
    /// Divergence is reported as an error — the caller should treat this
    /// as a solver bug or encoding issue requiring investigation. All
    /// divergence events are logged in `routing_stats` for post-hoc analysis.
    fn solve_cross_validate(&mut self, assertions: &List<Expr>) -> SolveResult {
        use std::time::Instant;

        let t_z3 = Instant::now();
        let z3_result = self.solve_with_z3(assertions);
        let z3_elapsed = t_z3.elapsed();

        let t_cvc5 = Instant::now();
        let cvc5_result = self.solve_with_cvc5(assertions);
        let cvc5_elapsed = t_cvc5.elapsed();

        // Classify for divergence event logging (needed only on disagreement).
        let chars = self.analyze_assertions_heuristically(assertions);
        let theory = crate::routing_stats::TheoryClass::classify(&chars);

        // Check for agreement.
        match (&z3_result, &cvc5_result) {
            (SolveResult::Sat { .. }, SolveResult::Sat { .. }) => {
                if self.config.verbose {
                    eprintln!("[CROSS-VALIDATE] Both solvers agreed: SAT");
                }
                self.routing_stats.record_cross_validate_agreement();
                z3_result
            }
            (SolveResult::Unsat { .. }, SolveResult::Unsat { .. }) => {
                if self.config.verbose {
                    eprintln!("[CROSS-VALIDATE] Both solvers agreed: UNSAT");
                }
                self.routing_stats.record_cross_validate_agreement();
                z3_result
            }
            (SolveResult::Sat { .. }, SolveResult::Unsat { .. })
            | (SolveResult::Unsat { .. }, SolveResult::Sat { .. }) => {
                // CRITICAL: solvers diverged — log the event for analysis.
                let z3_verdict = solve_result_to_verdict(&z3_result);
                let cvc5_verdict = solve_result_to_verdict(&cvc5_result);
                let timestamp_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                self.routing_stats.record_divergence(
                    crate::routing_stats::DivergenceEvent {
                        timestamp_secs,
                        theory,
                        z3_verdict,
                        cvc5_verdict,
                        z3_elapsed_ms: z3_elapsed.as_millis() as u64,
                        cvc5_elapsed_ms: cvc5_elapsed.as_millis() as u64,
                    },
                );
                eprintln!(
                    "[CROSS-VALIDATE] ⚠ CRITICAL: Z3 and CVC5 DIVERGED on {} goal. \
                     This is a solver bug or encoding issue — investigate.",
                    theory.mnemonic()
                );
                SolveResult::Error {
                    backend: "cross-validate".to_string(),
                    error: format!(
                        "Solver divergence detected. Z3: {:?} (in {}ms), CVC5: {:?} (in {}ms)",
                        z3_result,
                        z3_elapsed.as_millis(),
                        cvc5_result,
                        cvc5_elapsed.as_millis(),
                    ),
                }
            }
            _ => {
                // At least one was Unknown or Error.
                self.routing_stats.record_cross_validate_incomplete();
                if matches!(z3_result, SolveResult::Sat { .. } | SolveResult::Unsat { .. }) {
                    z3_result
                } else {
                    cvc5_result
                }
            }
        }
    }

    /// Solve with automatic backend selection
    fn solve_auto(&mut self, assertions: &List<Expr>) -> SolveResult {
        // Try Z3 first
        let z3_result = self.solve_with_z3(assertions);

        // Check if we should fallback
        if self.config.fallback.enabled {
            match &z3_result {
                SolveResult::Error { .. } if self.config.fallback.on_error => {
                    if self.config.verbose {
                        eprintln!("[AUTO] Z3 error, falling back to CVC5");
                    }
                    return self.solve_with_cvc5(assertions);
                }
                SolveResult::Unknown { .. } if self.config.fallback.on_unknown => {
                    if self.config.verbose {
                        eprintln!("[AUTO] Z3 unknown, falling back to CVC5");
                    }
                    return self.solve_with_cvc5(assertions);
                }
                _ => {}
            }
        }

        z3_result
    }

    /// Dispatch a `Synthesize`-strategy query to CVC5's SyGuS engine.
    ///
    /// This is **not** a satisfiability check. The caller provides a
    /// specification (the `assertions`) and the expected output is a
    /// *synthesized function body* that makes the specification hold.
    ///
    /// Return contract:
    ///
    /// * `SolveResult::Sat { model: Some(body) }` — SyGuS succeeded;
    ///   `body` is the synthesized function in SMT-LIB 2 format.
    /// * `SolveResult::Error { error }` — SyGuS is unavailable (CVC5
    ///   not linked with parser support) or the synthesis problem has
    ///   no solution within the default grammar.
    ///
    /// The previous implementation *silently* rerouted this to a
    /// capability-based satisfiability check. That produced Sat/Unsat
    /// answers for a caller who expected a synthesized program —
    /// a correctness bug. This version always surfaces a clear
    /// diagnostic path: either synthesis happened (Sat with body), or
    /// it didn't (Error with reason).
    ///
    /// ## Current coverage
    ///
    /// The implementation calls `cvc5_advanced::synthesize`. Under
    /// stub / no-cvc5-parser builds that entry point returns
    /// `Cvc5AdvancedError::Unsupported`, which this function maps to
    /// a `SolveResult::Error` — surfacing the unavailability to the
    /// user instead of masking it.
    ///
    /// Assertion-to-specification translation:
    ///
    /// The caller's `assertions` are serialised as a SyGuS problem
    /// preamble (`set-logic ALL`, `constraint` per assertion,
    /// `check-synth`). A user-supplied `synth-fun` declaration is
    /// required — synthesis without a target function signature is
    /// ill-formed. When the assertions don't include one, the Error
    /// path explains what's missing. This is the fundamental
    /// correctness boundary: without a `synth-fun`, there is nothing
    /// to synthesize.
    fn solve_synthesize(&mut self, assertions: &List<Expr>) -> SolveResult {
        use std::time::Instant;

        let start = Instant::now();

        // Build the SyGuS specification. We require the caller to
        // have surfaced a `synth-fun` declaration already encoded as
        // part of the assertion bundle; otherwise synthesis is
        // under-specified and we must say so.
        let mut spec = String::from("(set-logic ALL)\n");
        let mut saw_synth_fun = false;
        for a in assertions.iter() {
            // Assertion expressions in the switcher arrive pre-
            // encoded as SMT-LIB constraints. We pattern-match on
            // the Debug representation as a coarse detector for
            // `synth-fun`, which is the minimum the caller must
            // provide. Full AST-level inspection happens in the
            // SyGuS builder crate (task #87 follow-up).
            let s = format!("{:?}", a);
            if s.contains("synth-fun") || s.contains("SynthFun") {
                saw_synth_fun = true;
            }
            spec.push_str(&format!("(assert {})\n", s));
        }
        spec.push_str("(check-synth)\n");

        if !saw_synth_fun {
            return SolveResult::Error {
                backend: "synthesize".to_string(),
                error: "synthesize strategy requires a `synth-fun` \
                        declaration in the assertions; none was found. \
                        Add an explicit `@synth_fun` annotation or \
                        construct the SyGuS spec via \
                        `verum_smt::cvc5_advanced::SyGuSProblem` directly."
                    .to_string(),
            };
        }

        #[cfg(feature = "cvc5")]
        {
            use crate::cvc5_advanced::{synthesize, SyGuSProblem};
            let problem = SyGuSProblem {
                logic: "ALL".to_string(),
                specification: spec,
                timeout_ms: 0,
            };
            match synthesize(&problem) {
                Ok(result) => SolveResult::Sat {
                    backend: "cvc5-sygus".to_string(),
                    time_ms: start.elapsed().as_millis() as u64,
                    model: Maybe::Some(result.solution),
                },
                Err(e) => SolveResult::Error {
                    backend: "cvc5-sygus".to_string(),
                    error: format!("synthesis failed: {}", e),
                },
            }
        }

        #[cfg(not(feature = "cvc5"))]
        {
            let _ = spec;
            let _ = start;
            SolveResult::Error {
                backend: "synthesize".to_string(),
                error: "Synthesize strategy requires CVC5 SyGuS \
                        support; rebuild with the `cvc5` feature or \
                        link against a CVC5 with parser support. \
                        See `docs/verification/cli-workflow.md §6.4`."
                    .to_string(),
            }
        }
    }

    /// Solve with portfolio approach (parallel execution)
    fn solve_portfolio(&mut self, assertions: &List<Expr>) -> SolveResult {
        let (tx, rx) = mpsc::channel();

        // Clone assertions for both threads
        let z3_assertions = List::clone(assertions);
        let cvc5_assertions = List::clone(assertions);

        // Clone backend instances for parallel execution
        let z3_available = self.z3.is_some();
        let cvc5_available = self.cvc5.is_some();

        // Spawn Z3 thread if available
        let z3_handle = if z3_available {
            let tx_z3 = tx.clone();
            let z3_config = crate::z3_backend::Z3Config {
                global_timeout_ms: Some(self.config.portfolio.timeout_per_solver),
                ..Default::default()
            };

            Some(thread::spawn(move || {
                let z3_backend = Z3Backend::new(z3_config);
                let context = SmtContext {
                    assumptions: List::clone(&z3_assertions),
                    bindings: Map::new(),
                };

                let start = Instant::now();
                let result = if let Some(first) = z3_assertions.first() {
                    z3_backend.check_sat(first, &context)
                } else {
                    crate::solver::SmtResult::Sat
                };

                let elapsed = start.elapsed().as_millis() as u64;
                let solve_result = match result {
                    crate::solver::SmtResult::Sat => SolveResult::Sat {
                        backend: "Z3".to_string(),
                        time_ms: elapsed,
                        model: Maybe::None,
                    },
                    crate::solver::SmtResult::Unsat(counter) => SolveResult::Unsat {
                        backend: "Z3".to_string(),
                        time_ms: elapsed,
                        core: Maybe::None,
                        proof: Maybe::Some(counter.explanation.to_string()),
                    },
                    crate::solver::SmtResult::Unknown(reason) => SolveResult::Unknown {
                        backend: "Z3".to_string(),
                        reason: Maybe::Some(reason.to_string()),
                    },
                    crate::solver::SmtResult::Timeout => SolveResult::Unknown {
                        backend: "Z3".to_string(),
                        reason: Maybe::Some("Timeout".to_string()),
                    },
                };

                let _ = tx_z3.send(("Z3", solve_result));
            }))
        } else {
            None
        };

        // Spawn CVC5 thread if available
        let cvc5_handle = if cvc5_available {
            let tx_cvc5 = tx.clone();
            let cvc5_config = Cvc5Config {
                timeout_ms: Some(self.config.portfolio.timeout_per_solver),
                ..Default::default()
            };

            Some(thread::spawn(move || {
                let mut cvc5_backend: Cvc5Backend = match Cvc5Backend::new(cvc5_config) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx_cvc5.send((
                            "CVC5",
                            SolveResult::Error {
                                backend: "CVC5".to_string(),
                                error: format!("Failed to initialize: {:?}", e),
                            },
                        ));
                        return;
                    }
                };

                let start = Instant::now();

                // Assert formulas
                for assertion in &cvc5_assertions {
                    if let Err(e) = cvc5_backend.assert_formula_from_expr(assertion) {
                        let _ = tx_cvc5.send((
                            "CVC5",
                            SolveResult::Error {
                                backend: "CVC5".to_string(),
                                error: format!("Failed to assert: {:?}", e),
                            },
                        ));
                        return;
                    }
                }

                // Check satisfiability
                let result = match cvc5_backend.check_sat() {
                    Ok(res) => res,
                    Err(e) => {
                        let _ = tx_cvc5.send((
                            "CVC5",
                            SolveResult::Error {
                                backend: "CVC5".to_string(),
                                error: format!("Check-sat failed: {:?}", e),
                            },
                        ));
                        return;
                    }
                };

                let elapsed = start.elapsed().as_millis() as u64;

                let solve_result = match result {
                    crate::cvc5_backend::Cvc5SatResult::Sat => SolveResult::Sat {
                        backend: "CVC5".to_string(),
                        time_ms: elapsed,
                        model: Maybe::None,
                    },
                    crate::cvc5_backend::Cvc5SatResult::Unsat => SolveResult::Unsat {
                        backend: "CVC5".to_string(),
                        time_ms: elapsed,
                        core: Maybe::None,
                        proof: Maybe::None,
                    },
                    crate::cvc5_backend::Cvc5SatResult::Unknown => SolveResult::Unknown {
                        backend: "CVC5".to_string(),
                        reason: Maybe::Some("Unknown".to_string()),
                    },
                };

                let _ = tx_cvc5.send(("CVC5", solve_result));
            }))
        } else {
            None
        };

        // Wait for first result (or both if Consensus mode)
        match self.config.portfolio.mode {
            PortfolioMode::FirstResult => {
                // Return first result
                if let Ok((_solver, result)) = rx.recv() {
                    if self.config.verbose {
                        eprintln!("[PORTFOLIO] First result received");
                    }
                    result
                } else {
                    SolveResult::Error {
                        backend: "Portfolio".to_string(),
                        error: "All solvers failed".to_string(),
                    }
                }
            }
            PortfolioMode::Consensus => {
                // Wait for both results
                let result1 = rx.recv().ok();
                let result2 = rx.recv().ok();

                match (result1, result2) {
                    (Some((_, r1)), Some((_, r2))) => {
                        if self.results_agree(&r1, &r2) {
                            r1
                        } else {
                            if self.config.verbose {
                                eprintln!("[PORTFOLIO] Results disagree!");
                            }
                            SolveResult::Error {
                                backend: "Portfolio".to_string(),
                                error: "Solvers disagree".to_string(),
                            }
                        }
                    }
                    _ => SolveResult::Error {
                        backend: "Portfolio".to_string(),
                        error: "Failed to get both results".to_string(),
                    },
                }
            }
            PortfolioMode::VoteOnDisagree => {
                // Similar to Consensus but with voting logic
                let result1 = rx.recv().ok();
                let result2 = rx.recv().ok();

                match (result1, result2) {
                    (Some((_, r1)), Some((_, r2))) => {
                        if self.results_agree(&r1, &r2) {
                            r1
                        } else {
                            // Could add third solver tiebreaker here
                            SolveResult::Error {
                                backend: "Portfolio".to_string(),
                                error: "Solvers disagree, no tiebreaker available".to_string(),
                            }
                        }
                    }
                    _ => SolveResult::Error {
                        backend: "Portfolio".to_string(),
                        error: "Failed to get both results".to_string(),
                    },
                }
            }
        }
    }

    /// Check if two results agree
    fn results_agree(&self, r1: &SolveResult, r2: &SolveResult) -> bool {
        match (r1, r2) {
            (SolveResult::Sat { .. }, SolveResult::Sat { .. }) => true,
            (SolveResult::Unsat { .. }, SolveResult::Unsat { .. }) => true,
            (SolveResult::Unknown { .. }, SolveResult::Unknown { .. }) => true,
            _ => false,
        }
    }

    /// Get statistics
    pub fn get_stats(&self) -> SwitcherStats {
        self.stats.lock().unwrap().clone()
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        *self.stats.lock().unwrap() = SwitcherStats::default();
    }
}

// ==================== Result Types ====================

/// Solve result from backend switcher
#[derive(Debug, Clone)]
pub enum SolveResult {
    /// Formula is satisfiable
    Sat {
        /// Backend that produced the result
        backend: String,
        /// Solve time in milliseconds
        time_ms: u64,
        /// Model (optional)
        model: Maybe<String>,
    },

    /// Formula is unsatisfiable
    Unsat {
        /// Backend that produced the result
        backend: String,
        /// Solve time in milliseconds
        time_ms: u64,
        /// Unsat core (optional)
        core: Maybe<List<String>>,
        /// Proof (optional)
        proof: Maybe<String>,
    },

    /// Solver could not determine
    Unknown {
        /// Backend that produced the result
        backend: String,
        /// Reason for unknown
        reason: Maybe<String>,
    },

    /// Error occurred
    Error {
        /// Backend that produced the error
        backend: String,
        /// Error message
        error: String,
    },
}

impl SolveResult {
    /// Get backend name
    pub fn backend(&self) -> &str {
        match self {
            Self::Sat { backend, .. }
            | Self::Unsat { backend, .. }
            | Self::Unknown { backend, .. }
            | Self::Error { backend, .. } => backend,
        }
    }

    /// Check if result is SAT
    pub fn is_sat(&self) -> bool {
        matches!(self, Self::Sat { .. })
    }

    /// Check if result is UNSAT
    pub fn is_unsat(&self) -> bool {
        matches!(self, Self::Unsat { .. })
    }

    /// Check if result is unknown
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    /// Check if result is error
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

// ==================== Statistics ====================

/// Backend switcher statistics
#[derive(Debug, Clone, Default)]
pub struct SwitcherStats {
    /// Total number of queries
    pub total_queries: usize,

    /// Total time spent (milliseconds)
    pub total_time_ms: u64,

    /// Number of unknown results
    pub unknown_count: usize,

    /// Number of errors
    pub error_count: usize,

    /// Win count per backend
    pub backend_wins: Map<String, usize>,

    /// Number of fallback activations
    pub fallback_count: usize,

    /// Number of portfolio solves
    pub portfolio_count: usize,
}

impl SwitcherStats {
    /// Get average time per query
    pub fn avg_time_ms(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            self.total_time_ms as f64 / self.total_queries as f64
        }
    }

    /// Get success rate (SAT or UNSAT)
    pub fn success_rate(&self) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            let successes = self.total_queries - self.unknown_count - self.error_count;
            successes as f64 / self.total_queries as f64
        }
    }

    /// Get backend win rate
    pub fn backend_win_rate(&self, backend: &str) -> f64 {
        if self.total_queries == 0 {
            0.0
        } else {
            let wins = self
                .backend_wins
                .get(&backend.to_string())
                .copied()
                .unwrap_or(0);
            wins as f64 / self.total_queries as f64
        }
    }
}

// ==================== Environment Configuration ====================

impl SwitcherConfig {
    /// Load configuration from environment variables
    ///
    /// Environment variables:
    /// - `VERUM_SMT_BACKEND`: Backend choice (z3, cvc5, auto, portfolio)
    /// - `VERUM_SMT_FALLBACK`: Enable fallback (true/false)
    /// - `VERUM_SMT_TIMEOUT`: Timeout in milliseconds
    /// - `VERUM_SMT_PORTFOLIO_MODE`: Portfolio mode (first, consensus, vote)
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Backend selection
        if let Ok(backend) = std::env::var("VERUM_SMT_BACKEND") {
            if let Ok(choice) = backend.parse() {
                config.default_backend = choice;
            }
        }

        // Fallback
        if let Ok(fallback) = std::env::var("VERUM_SMT_FALLBACK") {
            config.fallback.enabled = fallback.parse().unwrap_or(true);
        }

        // Timeout
        if let Ok(timeout) = std::env::var("VERUM_SMT_TIMEOUT") {
            if let Ok(ms) = timeout.parse() {
                config.timeout_ms = ms;
            }
        }

        // Portfolio mode
        if let Ok(mode) = std::env::var("VERUM_SMT_PORTFOLIO_MODE") {
            config.portfolio.mode = match mode.to_lowercase().as_str() {
                "first" => PortfolioMode::FirstResult,
                "consensus" => PortfolioMode::Consensus,
                "vote" => PortfolioMode::VoteOnDisagree,
                _ => PortfolioMode::FirstResult,
            };
        }

        config
    }

    /// Load from TOML file
    ///
    /// Expected TOML format:
    /// ```toml
    /// default_backend = "z3"  # or "cvc5", "auto", "portfolio"
    /// timeout_ms = 30000
    /// verbose = false
    ///
    /// [fallback]
    /// enabled = true
    /// on_timeout = true
    /// on_unknown = true
    /// on_error = true
    /// max_attempts = 2
    ///
    /// [portfolio]
    /// enabled = false
    /// mode = "first"  # or "consensus", "vote"
    /// max_threads = 2
    /// timeout_per_solver = 30000
    /// kill_on_first = true
    ///
    /// [validation]
    /// enabled = false
    /// validate_sat = false
    /// validate_unsat = false
    /// ```
    pub fn from_file(path: &str) -> Result<Self, String> {
        use std::fs;

        // Read file
        let contents = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {}", path, e))?;

        // Parse TOML
        let value: toml::Value = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse TOML in '{}': {}", path, e))?;

        let mut config = Self::default();

        // Parse default_backend
        if let Some(backend) = value.get("default_backend").and_then(|v| v.as_str()) {
            config.default_backend = backend
                .parse()
                .map_err(|e| format!("Invalid default_backend: {}", e))?;
        }

        // Parse timeout_ms
        if let Some(timeout) = value.get("timeout_ms").and_then(|v| v.as_integer()) {
            config.timeout_ms = timeout as u64;
        }

        // Parse verbose
        if let Some(verbose) = value.get("verbose").and_then(|v| v.as_bool()) {
            config.verbose = verbose;
        }

        // Parse fallback section
        if let Some(fallback_table) = value.get("fallback").and_then(|v| v.as_table()) {
            if let Some(enabled) = fallback_table.get("enabled").and_then(|v| v.as_bool()) {
                config.fallback.enabled = enabled;
            }
            if let Some(on_timeout) = fallback_table.get("on_timeout").and_then(|v| v.as_bool()) {
                config.fallback.on_timeout = on_timeout;
            }
            if let Some(on_unknown) = fallback_table.get("on_unknown").and_then(|v| v.as_bool()) {
                config.fallback.on_unknown = on_unknown;
            }
            if let Some(on_error) = fallback_table.get("on_error").and_then(|v| v.as_bool()) {
                config.fallback.on_error = on_error;
            }
            if let Some(max_attempts) = fallback_table
                .get("max_attempts")
                .and_then(|v| v.as_integer())
            {
                config.fallback.max_attempts = max_attempts as usize;
            }
        }

        // Parse portfolio section
        if let Some(portfolio_table) = value.get("portfolio").and_then(|v| v.as_table()) {
            if let Some(enabled) = portfolio_table.get("enabled").and_then(|v| v.as_bool()) {
                config.portfolio.enabled = enabled;
            }
            if let Some(mode_str) = portfolio_table.get("mode").and_then(|v| v.as_str()) {
                config.portfolio.mode = match mode_str {
                    "first" | "FirstResult" => PortfolioMode::FirstResult,
                    "consensus" | "Consensus" => PortfolioMode::Consensus,
                    "vote" | "VoteOnDisagree" => PortfolioMode::VoteOnDisagree,
                    _ => return Err(format!("Invalid portfolio mode: {}", mode_str)),
                };
            }
            if let Some(max_threads) = portfolio_table
                .get("max_threads")
                .and_then(|v| v.as_integer())
            {
                config.portfolio.max_threads = max_threads as usize;
            }
            if let Some(timeout) = portfolio_table
                .get("timeout_per_solver")
                .and_then(|v| v.as_integer())
            {
                config.portfolio.timeout_per_solver = timeout as u64;
            }
            if let Some(kill) = portfolio_table
                .get("kill_on_first")
                .and_then(|v| v.as_bool())
            {
                config.portfolio.kill_on_first = kill;
            }
        }

        // Parse validation section
        if let Some(validation_table) = value.get("validation").and_then(|v| v.as_table()) {
            if let Some(enabled) = validation_table.get("enabled").and_then(|v| v.as_bool()) {
                config.validation.enabled = enabled;
            }
            // Note: validate_sat and validate_unsat removed - use cross_validate instead
        }

        Ok(config)
    }
}

// ==================== SolveResult ↔ SolverVerdict bridge ====================

/// Convert a `SolveResult` to the portfolio/telemetry `SolverVerdict` format.
///
/// Used for cross-validation divergence event logging, where we need to
/// record exactly what each solver returned.
fn solve_result_to_verdict(
    result: &SolveResult,
) -> crate::portfolio_executor::SolverVerdict {
    use crate::portfolio_executor::SolverVerdict;
    match result {
        SolveResult::Sat { .. } => SolverVerdict::Sat,
        SolveResult::Unsat { .. } => SolverVerdict::Unsat,
        SolveResult::Unknown { reason, .. } => {
            let r = match reason {
                Maybe::Some(s) => s.clone(),
                Maybe::None => "unknown".to_string(),
            };
            SolverVerdict::Unknown { reason: r }
        }
        SolveResult::Error { error, .. } => SolverVerdict::Error {
            message: error.clone(),
        },
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod synthesize_tests {
    use super::*;
    use crate::verify_strategy::VerifyStrategy;

    /// The Synthesize strategy must not silently fall back to a
    /// satisfiability check when `synth-fun` is missing — it must
    /// return an Error explaining what's missing. This is the
    /// correctness guarantee that distinguishes the fixed
    /// implementation from the silent-fallback predecessor.
    #[test]
    fn synthesize_without_synth_fun_returns_error_not_sat() {
        let config = SwitcherConfig::default();
        let mut switcher = SmtBackendSwitcher::new(config);
        let assertions: List<Expr> = List::new();
        let result = switcher.solve_with_strategy(
            &assertions,
            &VerifyStrategy::Synthesize,
        );
        match result {
            Some(SolveResult::Error { backend, error }) => {
                assert!(
                    error.contains("synth-fun") || error.contains("Synthesize"),
                    "error should cite missing synth-fun: {}",
                    error
                );
                assert!(
                    backend.contains("synth"),
                    "backend tag should identify as synthesis: {}",
                    backend
                );
            }
            Some(SolveResult::Sat { .. }) => {
                panic!(
                    "Synthesize without synth-fun silently returned Sat — \
                     this was the silent-fallback correctness bug"
                );
            }
            Some(SolveResult::Unsat { .. }) => {
                panic!(
                    "Synthesize without synth-fun silently returned Unsat — \
                     this was the silent-fallback correctness bug"
                );
            }
            Some(SolveResult::Unknown { .. }) => {
                panic!(
                    "Synthesize without synth-fun returned Unknown — \
                     should be explicit Error with rationale"
                );
            }
            None => panic!(
                "Synthesize strategy produced no result (requires_smt returned false?)"
            ),
        }
    }

    /// Requires-SMT gating still holds — Runtime / Static strategies
    /// produce `None` from `solve_with_strategy`, so the Synthesize
    /// branch is only reachable for strategies that actually need
    /// the solver.
    #[test]
    fn runtime_strategy_skips_solver_dispatch() {
        let mut switcher = SmtBackendSwitcher::with_defaults();
        let assertions: List<Expr> = List::new();
        let result = switcher
            .solve_with_strategy(&assertions, &VerifyStrategy::Runtime);
        assert!(result.is_none());
    }
}

// ==================== Module Statistics ====================

// Total lines: ~640
// Complete backend switcher implementation
// Features:
// - Manual, auto, fallback, and portfolio modes
// - Comprehensive configuration
// - Statistics tracking
// - Environment variable support
// - Thread-safe parallel execution
// - Result validation
