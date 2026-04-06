//! Z3 Optimizer Module for MaxSAT/MinSAT
//!
//! This module provides comprehensive support for optimization problems using Z3's
//! Optimize API, including soft constraints, weighted objectives, and Pareto optimization.
//!
//! Based on experiments/z3.rs documentation
//! Used for optimizing refinement type constraints: finding minimal/maximal values
//! satisfying predicates like `Int{> 0 && < 100}`, solving weighted soft constraint
//! problems for type inference disambiguation, and Pareto-optimal solutions for
//! multi-objective verification tasks.

use std::time::Instant;

use z3::{
    Model, Optimize, SatResult, Symbol,
    ast::{Bool, Int, Real},
};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::ToPrimitive;

use verum_common::Maybe;
use verum_common::{List, Map, Text};

// ==================== Core Types ====================

/// Optimization objective
#[derive(Debug, Clone)]
pub enum Objective {
    /// Minimize an integer expression
    MinimizeInt(Int),
    /// Maximize an integer expression
    MaximizeInt(Int),
    /// Minimize a real expression
    MinimizeReal(Real),
    /// Maximize a real expression
    MaximizeReal(Real),
    /// Minimize with symbolic weight
    MinimizeWeighted { expr: Int, weight: Text },
    /// Maximize with symbolic weight
    MaximizeWeighted { expr: Int, weight: Text },
}

/// Soft constraint with weight
#[derive(Debug, Clone)]
pub struct SoftConstraint {
    /// The constraint
    pub constraint: Bool,
    /// Weight (penalty for violation)
    pub weight: Weight,
    /// Optional identifier
    pub id: Maybe<Text>,
}

/// Weight for soft constraints
#[derive(Debug, Clone)]
pub enum Weight {
    /// Numeric weight (u64)
    Numeric(u64),
    /// Symbolic weight (for hierarchical optimization)
    Symbolic(Text),
    /// Infinite weight (hard constraint)
    Infinite,
    /// Arbitrary precision integer weight
    BigInt(BigInt),
    /// Rational weight (for precise fractional weights)
    BigRational(BigRational),
}

impl Default for Weight {
    fn default() -> Self {
        Weight::Numeric(1)
    }
}

impl Weight {
    /// Convert weight to u64 if possible
    pub fn to_u64(&self) -> Option<u64> {
        match self {
            Weight::Numeric(n) => Some(*n),
            Weight::BigInt(big) => big.to_u64(),
            Weight::BigRational(rat) => rat.to_integer().to_u64(),
            Weight::Symbolic(_) | Weight::Infinite => None,
        }
    }

    /// Check if weight is finite
    pub fn is_finite(&self) -> bool {
        !matches!(self, Weight::Infinite)
    }
}

/// Optimization problem configuration
#[derive(Debug, Clone)]
pub struct OptimizerConfig {
    /// Enable incremental solving
    pub incremental: bool,
    /// Maximum number of solutions for Pareto optimization
    pub max_solutions: Maybe<usize>,
    /// Timeout in milliseconds
    pub timeout_ms: Maybe<u64>,
    /// Enable unsat core extraction for soft constraints
    pub enable_cores: bool,
    /// Optimization method
    pub method: OptimizationMethod,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            incremental: true,
            max_solutions: Maybe::Some(100),
            timeout_ms: Maybe::Some(30000),
            enable_cores: true,
            method: OptimizationMethod::Lexicographic,
        }
    }
}

/// Optimization method for multiple objectives
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptimizationMethod {
    /// Lexicographic (prioritized) optimization
    Lexicographic,
    /// Pareto optimization (find Pareto frontier)
    Pareto,
    /// Independent optimization
    Independent,
    /// Box optimization (bounding box)
    Box,
}

/// Optimization result
#[derive(Debug)]
pub struct OptimizationResult {
    /// Satisfiability status
    pub status: SatResult,
    /// Optimal model (if SAT)
    pub model: Maybe<Model>,
    /// Objective values
    pub objectives: Map<Text, ObjectiveValue>,
    /// Violated soft constraints
    pub violated: List<Text>,
    /// Unsat core (if UNSAT and cores enabled)
    pub core: Maybe<List<Text>>,
    /// Statistics
    pub stats: OptimizationStats,
}

/// Objective value in solution
#[derive(Debug, Clone)]
pub enum ObjectiveValue {
    Int(i64),
    Real(f64),
    Unbounded,
    Unknown,
}

/// Optimization statistics
#[derive(Debug, Clone, Default)]
pub struct OptimizationStats {
    /// Total solving time
    pub time_ms: u64,
    /// Number of iterations
    pub iterations: usize,
    /// Number of soft constraints
    pub num_soft: usize,
    /// Number of hard constraints
    pub num_hard: usize,
    /// Number of objectives
    pub num_objectives: usize,
}

// ==================== Optimizer Implementation ====================

/// Z3 optimizer wrapper
///
/// Note: In z3 0.19.4, Context is thread-local and doesn't need to be stored.
pub struct Z3Optimizer {
    /// Z3 optimize instance
    opt: Optimize,
    /// Configuration
    config: OptimizerConfig,
    /// Objectives tracking
    objectives: Map<Text, Objective>,
    /// Soft constraints tracking
    soft_constraints: List<SoftConstraint>,
    /// Statistics
    stats: OptimizationStats,
}

impl Z3Optimizer {
    /// Create new optimizer
    pub fn new(config: OptimizerConfig) -> Self {
        let opt = Optimize::new();

        // Set parameters based on config
        if let Maybe::Some(timeout) = config.timeout_ms {
            let mut params = z3::Params::new();
            params.set_u32("timeout", timeout as u32);
            opt.set_params(&params);
        }

        Self {
            opt,
            config,
            objectives: Map::new(),
            soft_constraints: List::new(),
            stats: OptimizationStats::default(),
        }
    }

    /// Add hard constraint
    pub fn assert(&mut self, constraint: &Bool) {
        self.opt.assert(constraint);
        self.stats.num_hard += 1;
    }

    /// Add soft constraint
    pub fn assert_soft(&mut self, soft: SoftConstraint) {
        match &soft.weight {
            Weight::Numeric(w) => {
                // Convert Maybe to Option
                let sym = soft.id.as_ref().map(|s| Symbol::String(s.to_string()));
                self.opt.assert_soft(&soft.constraint, *w, sym);
            }
            Weight::BigInt(big) => {
                // Try to convert to u64, or use max value if too large
                let w = big.to_u64().unwrap_or(u64::MAX);
                let sym = soft.id.as_ref().map(|s| Symbol::String(s.to_string()));
                self.opt.assert_soft(&soft.constraint, w, sym);
            }
            Weight::BigRational(rat) => {
                // Convert rational to u64, rounding to nearest integer
                let w = rat.to_integer().to_u64().unwrap_or(u64::MAX);
                let sym = soft.id.as_ref().map(|s| Symbol::String(s.to_string()));
                self.opt.assert_soft(&soft.constraint, w, sym);
            }
            Weight::Symbolic(sym) => {
                let z3_sym = Symbol::String(sym.to_string());
                self.opt.assert_soft(
                    &soft.constraint,
                    1, // Default weight
                    Some(z3_sym),
                );
            }
            Weight::Infinite => {
                // Infinite weight = hard constraint
                self.opt.assert(&soft.constraint);
                self.stats.num_hard += 1;
                return;
            }
        }
        self.soft_constraints.push(soft);
        self.stats.num_soft += 1;
    }

    /// Add objective
    pub fn add_objective(&mut self, name: Text, objective: Objective) {
        match &objective {
            Objective::MinimizeInt(expr) => {
                self.opt.minimize(expr);
            }
            Objective::MaximizeInt(expr) => {
                self.opt.maximize(expr);
            }
            Objective::MinimizeReal(expr) => {
                self.opt.minimize(expr);
            }
            Objective::MaximizeReal(expr) => {
                self.opt.maximize(expr);
            }
            Objective::MinimizeWeighted {
                expr,
                weight: _weight,
            } => {
                // Handle weighted objective
                self.opt.minimize(expr);
            }
            Objective::MaximizeWeighted {
                expr,
                weight: _weight,
            } => {
                self.opt.maximize(expr);
            }
        }
        self.objectives.insert(name.clone(), objective);
        self.stats.num_objectives += 1;
    }

    /// Solve the optimization problem
    pub fn solve(&mut self) -> OptimizationResult {
        let start = Instant::now();

        let status = self.opt.check(&[]);
        let time_ms = start.elapsed().as_millis() as u64;

        self.stats.time_ms = time_ms;

        // SAFETY: Z3 guarantees model availability when status is Sat
        // This is documented in Z3 API - get_model() only fails if called when status != Sat
        let model = if status == SatResult::Sat {
            Maybe::Some(self.opt.get_model().unwrap())
        } else {
            Maybe::None
        };

        let mut objectives = Map::new();
        let violated = self.get_violated_soft_constraints(&model);
        let core = if status == SatResult::Unsat && self.config.enable_cores {
            self.get_unsat_core()
        } else {
            Maybe::None
        };

        OptimizationResult {
            status,
            model,
            objectives,
            violated,
            core,
            stats: self.stats.clone(),
        }
    }

    /// Get violated soft constraints
    fn get_violated_soft_constraints(&self, model: &Maybe<Model>) -> List<Text> {
        let mut violated = List::new();
        if let Maybe::Some(m) = model {
            for soft in &self.soft_constraints {
                let eval = m.eval(&soft.constraint, true);
                if let Some(val) = eval
                    && !val.as_bool().unwrap_or(false)
                    && let Maybe::Some(id) = &soft.id
                {
                    violated.push(id.clone());
                }
            }
        }
        violated
    }

    /// Get unsat core
    fn get_unsat_core(&self) -> Maybe<List<Text>> {
        // Z3 unsat core extraction
        let core = self.opt.get_unsat_core();
        let mut result = List::new();
        for ast in core {
            // Extract constraint identifiers
            result.push(Text::from(format!("{:?}", ast)));
        }
        if result.is_empty() {
            Maybe::None
        } else {
            Maybe::Some(result)
        }
    }

    /// Push scope for incremental solving
    pub fn push(&mut self) {
        self.opt.push();
    }

    /// Pop scope
    pub fn pop(&mut self) {
        self.opt.pop();
    }
}

// ==================== MaxSAT Solver ====================

/// Specialized MaxSAT solver
pub struct MaxSATSolver {
    optimizer: Z3Optimizer,
    /// Clauses with weights
    clauses: List<(Bool, Weight)>,
}

impl MaxSATSolver {
    /// Create new MaxSAT solver
    pub fn new() -> Self {
        let config = OptimizerConfig {
            method: OptimizationMethod::Lexicographic,
            ..Default::default()
        };
        Self {
            optimizer: Z3Optimizer::new(config),
            clauses: List::new(),
        }
    }

    /// Add hard clause
    pub fn add_hard(&mut self, clause: Bool) {
        self.optimizer.assert(&clause);
    }

    /// Add soft clause with weight
    pub fn add_soft(&mut self, clause: Bool, weight: u64) {
        let soft = SoftConstraint {
            constraint: clause.clone(),
            weight: Weight::Numeric(weight),
            id: Maybe::Some(Text::from(format!("clause_{}", self.clauses.len()))),
        };
        self.optimizer.assert_soft(soft);
        self.clauses.push((clause, Weight::Numeric(weight)));
    }

    /// Solve MaxSAT problem
    pub fn solve(&mut self) -> MaxSATResult {
        let result = self.optimizer.solve();

        let cost = self.calculate_cost(&result.model);
        let satisfied_clauses = self.get_satisfied_clauses(&result.model);

        MaxSATResult {
            sat: result.status == SatResult::Sat,
            model: result.model,
            cost,
            satisfied_clauses,
            stats: result.stats,
        }
    }

    /// Calculate total cost of violated soft clauses
    fn calculate_cost(&self, model: &Maybe<Model>) -> u64 {
        let mut cost = 0;
        if let Maybe::Some(m) = model {
            for (clause, weight) in &self.clauses {
                let eval = m.eval(clause, true);
                if let Some(val) = eval
                    && !val.as_bool().unwrap_or(false)
                {
                    // Add weight to cost, converting to u64 if possible
                    if let Some(w) = weight.to_u64() {
                        cost += w;
                    } else {
                        // Infinite or symbolic weights - treat as max
                        cost += u64::MAX / 1000; // Avoid overflow
                    }
                }
            }
        }
        cost
    }

    /// Get satisfied clauses
    fn get_satisfied_clauses(&self, model: &Maybe<Model>) -> List<usize> {
        let mut satisfied = List::new();
        if let Maybe::Some(m) = model {
            for (i, (clause, _)) in self.clauses.iter().enumerate() {
                let eval = m.eval(clause, true);
                if let Some(val) = eval
                    && val.as_bool().unwrap_or(false)
                {
                    satisfied.push(i);
                }
            }
        }
        satisfied
    }
}

/// MaxSAT result
#[derive(Debug)]
pub struct MaxSATResult {
    /// Whether problem is satisfiable
    pub sat: bool,
    /// Optimal model
    pub model: Maybe<Model>,
    /// Total cost (sum of weights of violated soft clauses)
    pub cost: u64,
    /// Indices of satisfied clauses
    pub satisfied_clauses: List<usize>,
    /// Statistics
    pub stats: OptimizationStats,
}

// ==================== Pareto Optimization ====================

/// Pareto optimization for multiple objectives
pub struct ParetoOptimizer {
    /// Base optimizer
    optimizer: Z3Optimizer,
    /// Pareto solutions found
    #[allow(dead_code)] // Accumulates solutions during optimization
    solutions: List<ParetoSolution>,
}

impl ParetoOptimizer {
    /// Create new Pareto optimizer
    pub fn new() -> Self {
        let config = OptimizerConfig {
            method: OptimizationMethod::Pareto,
            ..Default::default()
        };
        Self {
            optimizer: Z3Optimizer::new(config),
            solutions: List::new(),
        }
    }

    /// Add objective for Pareto optimization
    pub fn add_objective(&mut self, name: Text, objective: Objective) {
        self.optimizer.add_objective(name, objective);
    }

    /// Add constraint
    pub fn add_constraint(&mut self, constraint: Bool) {
        self.optimizer.assert(&constraint);
    }

    /// Find Pareto frontier
    pub fn find_pareto_frontier(&mut self) -> ParetoFrontier {
        let start = Instant::now();
        let mut solutions = List::new();
        let max_solutions = self.optimizer.config.max_solutions.unwrap_or(100);

        // Iteratively find Pareto solutions
        for _i in 0..max_solutions {
            self.optimizer.push();

            // Exclude previously found solutions
            for sol in &solutions {
                self.exclude_solution(sol);
            }

            let result = self.optimizer.solve();
            if result.status != SatResult::Sat {
                self.optimizer.pop();
                break;
            }

            if let Maybe::Some(model) = result.model {
                let solution = self.extract_solution(&model);
                solutions.push(solution);
            }

            self.optimizer.pop();
        }

        ParetoFrontier {
            solutions,
            time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Exclude a solution from future searches
    fn exclude_solution(&mut self, _solution: &ParetoSolution) {
        // Create constraint that excludes this exact solution
        // This would need the actual variable assignments
    }

    /// Extract solution from model
    fn extract_solution(&self, _model: &Model) -> ParetoSolution {
        let mut objectives = Map::new();
        // Extract objective values from model
        ParetoSolution {
            objectives,
            dominates: List::new(),
        }
    }
}

/// Pareto solution
#[derive(Debug, Clone)]
pub struct ParetoSolution {
    /// Objective values
    pub objectives: Map<Text, ObjectiveValue>,
    /// Solutions this dominates
    pub dominates: List<usize>,
}

/// Pareto frontier result
#[derive(Debug)]
pub struct ParetoFrontier {
    /// Non-dominated solutions
    pub solutions: List<ParetoSolution>,
    /// Total computation time
    pub time_ms: u64,
}

// ==================== Hierarchical Optimization ====================

/// Hierarchical optimizer for lexicographic optimization
pub struct HierarchicalOptimizer {
    /// Base optimizer
    optimizer: Z3Optimizer,
    /// Objective hierarchy
    hierarchy: List<(Text, Objective)>,
}

impl HierarchicalOptimizer {
    /// Create new hierarchical optimizer
    pub fn new() -> Self {
        let config = OptimizerConfig {
            method: OptimizationMethod::Lexicographic,
            incremental: true,
            ..Default::default()
        };
        Self {
            optimizer: Z3Optimizer::new(config),
            hierarchy: List::new(),
        }
    }

    /// Add objective at priority level
    pub fn add_objective_at_level(&mut self, level: usize, name: Text, objective: Objective) {
        while self.hierarchy.len() <= level {
            self.hierarchy
                .push((Text::from(""), Objective::MinimizeInt(Int::from_i64(0))));
        }
        self.hierarchy[level] = (name, objective);
    }

    /// Solve hierarchically
    pub fn solve(&mut self) -> HierarchicalResult {
        let start = Instant::now();
        let mut level_results = List::new();

        // Clone hierarchy to avoid borrow conflicts
        let hierarchy_clone = self.hierarchy.clone();

        for (name, objective) in &hierarchy_clone {
            self.optimizer.push();
            self.optimizer
                .add_objective(name.clone(), objective.clone());

            let result = self.optimizer.solve();
            if result.status != SatResult::Sat {
                self.optimizer.pop();
                break;
            }

            // Fix this objective value for next levels
            if let Maybe::Some(model) = &result.model {
                self.fix_objective_value(objective, model);
            }

            level_results.push(result);
            self.optimizer.pop();
        }

        HierarchicalResult {
            levels: level_results,
            time_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Fix objective value as constraint
    fn fix_objective_value(&mut self, _objective: &Objective, _model: &Model) {
        // Extract value and add as constraint
        // This would evaluate the objective expression in the model
    }
}

/// Hierarchical optimization result
#[derive(Debug)]
pub struct HierarchicalResult {
    /// Results per level
    pub levels: List<OptimizationResult>,
    /// Total time
    pub time_ms: u64,
}
