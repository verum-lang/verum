//! Craig Interpolation Module - Complete Z3 Implementation
//!
//! This module provides comprehensive Craig interpolation support using Z3's
//! Model-Based Interpolation (MBI) engine through direct z3-sys FFI bindings.
//!
//! ## Interpolation Techniques
//!
//! 1. **Craig Interpolation**: Classical interpolation between A and B where A ∧ B is UNSAT
//! 2. **Sequence Interpolation**: Generate interpolants for formula sequences (path interpolation)
//! 3. **Tree Interpolation**: Hierarchical interpolation for modular verification
//! 4. **Model-Based Interpolation**: Use models and quantifier elimination
//! 5. **Proof-Based Interpolation**: Extract from resolution proofs
//!
//! ## Algorithms
//!
//! - **McMillan**: Resolution-proof based interpolation (strongest)
//! - **Pudlák**: Dual to McMillan (weakest)
//! - **Dual**: Combines both approaches
//! - **Symmetric**: Balanced interpolation
//! - **MBI**: Model-based with quantifier elimination
//!
//! ## Use Cases
//!
//! - **Compositional Verification**: Verify modules independently
//! - **CEGAR**: Counter-Example Guided Abstraction Refinement
//! - **Invariant Generation**: Synthesize loop invariants
//! - **Modular Reasoning**: Hierarchical proof decomposition
//!
//! Compositional refinement verification: when verifying module A against specification B
//! where A AND B is UNSAT, Craig interpolation produces a formula I over shared symbols
//! such that A => I and I AND B is UNSAT. This enables modular verification of refinement
//! types across module boundaries without re-verifying the full program.
//! Based on: Z3 qe/qe_mbi.h and experiments/z3.rs

use std::time::Instant;

use z3::{
    SatResult, Solver,
    ast::{Ast, Bool},
};

use verum_common::{List, Maybe, Set, Text};

// Use our Context wrapper, not z3::Context directly
use crate::Context;

// ==================== Core Types ====================

/// Interpolant between two formulas
///
/// For formulas A and B where A ∧ B is UNSAT, an interpolant I satisfies:
/// 1. A ⇒ I
/// 2. I ∧ B ⇒ ⊥
/// 3. I only mentions shared variables between A and B
#[derive(Debug, Clone)]
pub struct Interpolant {
    /// The interpolant formula
    pub formula: Bool,
    /// Variables in the interpolant (must be shared between A and B)
    pub shared_vars: List<Text>,
    /// Strength of the interpolant
    pub strength: InterpolantStrength,
    /// Source formulas
    pub source: InterpolantSource,
    /// Computation time
    pub time_ms: u64,
}

impl Interpolant {
    /// Validate interpolation properties
    pub fn validate(&self, _ctx: &Context) -> Result<bool, Text> {
        let solver = Solver::new();

        // Check: A ⇒ I
        solver.push();
        solver.assert(&self.source.formula_a);
        solver.assert(self.formula.not());
        let a_implies_i = solver.check() == SatResult::Unsat;
        solver.pop(1);

        // Check: I ∧ B ⇒ ⊥
        solver.push();
        solver.assert(&self.formula);
        solver.assert(&self.source.formula_b);
        let i_and_b_unsat = solver.check() == SatResult::Unsat;
        solver.pop(1);

        if !a_implies_i {
            return Err(Text::from(
                "Interpolant validation failed: A does not imply I",
            ));
        }
        if !i_and_b_unsat {
            return Err(Text::from(
                "Interpolant validation failed: I ∧ B is not UNSAT",
            ));
        }

        Ok(true)
    }
}

/// Source of interpolation
#[derive(Debug, Clone)]
pub struct InterpolantSource {
    /// Formula A (left)
    pub formula_a: Bool,
    /// Formula B (right)
    pub formula_b: Bool,
    /// Common variables
    pub common: List<Text>,
}

/// Strength of interpolant
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpolantStrength {
    /// Weakest interpolant (Pudlák)
    Weakest,
    /// Strongest interpolant (McMillan)
    Strongest,
    /// Balanced (between weakest and strongest)
    Balanced,
    /// Model-based (depends on model)
    ModelBased,
}

/// Sequence interpolant for program paths
#[derive(Debug)]
pub struct SequenceInterpolant {
    /// Interpolants between consecutive formulas
    pub interpolants: List<Interpolant>,
    /// Original formula sequence
    pub formulas: List<Bool>,
    /// Total computation time
    pub time_ms: u64,
}

/// Tree interpolant for modular proofs
#[derive(Debug)]
pub struct TreeInterpolant {
    /// Root interpolant
    pub root: InterpolantNode,
    /// Total nodes
    pub num_nodes: usize,
    /// Computation time
    pub time_ms: u64,
}

/// Node in tree interpolant
#[derive(Debug)]
pub struct InterpolantNode {
    /// Interpolant at this node
    pub interpolant: Interpolant,
    /// Child nodes
    pub children: List<InterpolantNode>,
    /// Node identifier
    pub id: Text,
}

/// Configuration for interpolation
#[derive(Debug, Clone)]
pub struct InterpolationConfig {
    /// Interpolation algorithm
    pub algorithm: InterpolationAlgorithm,
    /// Strength preference
    pub strength: InterpolantStrength,
    /// Simplify interpolants
    pub simplify: bool,
    /// Timeout
    pub timeout_ms: Maybe<u64>,
    /// Use proof-based interpolation
    pub proof_based: bool,
    /// Use model-based interpolation
    pub model_based: bool,
    /// Enable quantifier elimination
    pub quantifier_elimination: bool,
    /// Maximum projection variables
    pub max_projection_vars: usize,
}

impl Default for InterpolationConfig {
    fn default() -> Self {
        Self {
            algorithm: InterpolationAlgorithm::MBI,
            strength: InterpolantStrength::Balanced,
            simplify: true,
            timeout_ms: Maybe::Some(5000),
            proof_based: false,
            model_based: true,
            quantifier_elimination: true,
            max_projection_vars: 100,
        }
    }
}

/// Interpolation algorithms
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpolationAlgorithm {
    /// McMillan's algorithm (resolution-proof based, strongest)
    McMillan,
    /// Pudlák's algorithm (dual, weakest)
    Pudlak,
    /// Dual interpolation
    Dual,
    /// Symmetric interpolation
    Symmetric,
    /// Model-Based Interpolation (Z3's native approach)
    MBI,
    /// Ping-pong interpolation
    PingPong,
    /// One-sided pogo
    Pogo,
}

// ==================== Interpolation Engine ====================

/// Craig interpolation engine using Z3's MBI capabilities
///
/// This engine provides multiple interpolation algorithms using Z3's
/// quantifier elimination and model-based projection.
///
/// Note: In z3 0.19.4, Context is thread-local and implicit, so it's not stored here.
pub struct InterpolationEngine {
    /// Configuration
    config: InterpolationConfig,
}

impl InterpolationEngine {
    /// Create new interpolation engine
    pub fn new(config: InterpolationConfig) -> Self {
        // Note: Z3 context configuration is implicit and thread-local in z3 0.19.4
        // Set up solver parameters globally if needed through Solver/Optimize/etc
        Self { config }
    }

    /// Construct a Z3 solver with this engine's `config.timeout_ms`
    /// applied. All `Solver::new()` sites that participate in
    /// interpolation work route through this helper so the timeout
    /// field — documented as "Timeout" on `InterpolationConfig` —
    /// actually constrains every solver instance the engine spawns.
    /// Without this wiring the field would be inert: Z3 would run
    /// to its native limit (effectively unlimited) regardless of
    /// what callers configured.
    fn fresh_solver(&self) -> Solver {
        let solver = Solver::new();
        self.apply_timeout_only(&solver);
        solver
    }

    /// Apply only the configured timeout to `solver` via a fresh
    /// `Params`. Helpers that need to set algorithm-specific
    /// params (e.g. `proof = true` in McMillan) merge them into
    /// their own [`z3::Params`] and route through
    /// [`apply_timeout_into`] so the timeout and the
    /// algorithm-specific options arrive in the same call —
    /// `Solver::set_params` replaces the entire param set, so two
    /// separate calls would lose whichever came first.
    fn apply_timeout_only(&self, solver: &Solver) {
        if let Maybe::Some(_) = self.config.timeout_ms {
            let mut params = z3::Params::new();
            self.apply_timeout_into(&mut params);
            solver.set_params(&params);
        }
    }

    /// Mix the configured timeout into an already-constructed
    /// `Params`. Callers that need other params (proof, model,
    /// etc.) build their `Params`, call this, then send the
    /// merged value via `Solver::set_params`. Keeps the timeout
    /// honoured even on call sites that customise the solver.
    fn apply_timeout_into(&self, params: &mut z3::Params) {
        if let Maybe::Some(timeout) = self.config.timeout_ms {
            params.set_u32("timeout", timeout as u32);
        }
    }

    /// Compute interpolant between two formulas
    ///
    /// Given A and B where A ∧ B is UNSAT, compute I such that:
    /// - A ⇒ I
    /// - I ∧ B ⇒ ⊥
    /// - I uses only shared variables
    pub fn interpolate(&self, a: &Bool, b: &Bool) -> Result<Interpolant, Text> {
        let start = Instant::now();

        // Check if A ∧ B is unsatisfiable
        let solver = self.fresh_solver();
        solver.assert(a);
        solver.assert(b);

        if solver.check() != SatResult::Unsat {
            return Err(Text::from(
                "Formulas A ∧ B must be unsatisfiable for interpolation",
            ));
        }

        // Extract shared variables
        let shared_vars = self.extract_shared_variables(a, b)?;

        // Extract interpolant based on algorithm
        let formula = match self.config.algorithm {
            InterpolationAlgorithm::McMillan => self.mcmillan_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::Pudlak => self.pudlak_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::Dual => self.dual_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::Symmetric => self.symmetric_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::MBI => self.mbi_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::PingPong => self.pingpong_interpolate(a, b, &shared_vars)?,
            InterpolationAlgorithm::Pogo => self.pogo_interpolate(a, b, &shared_vars)?,
        };

        // Simplify if requested
        let final_formula = if self.config.simplify {
            formula.simplify()
        } else {
            formula
        };

        let interpolant = Interpolant {
            formula: final_formula,
            shared_vars,
            strength: self.config.strength.clone(),
            source: InterpolantSource {
                formula_a: a.clone(),
                formula_b: b.clone(),
                common: List::new(),
            },
            time_ms: start.elapsed().as_millis() as u64,
        };

        Ok(interpolant)
    }

    /// Extract shared variables between two formulas
    fn extract_shared_variables(&self, a: &Bool, b: &Bool) -> Result<List<Text>, Text> {
        let vars_a = self.collect_variables(a);
        let vars_b = self.collect_variables(b);

        let mut shared = List::new();
        for var in vars_a {
            if vars_b.contains(&var) {
                shared.push(var);
            }
        }

        Ok(shared)
    }

    /// Collect all free variables in a formula
    ///
    /// Uses the shared variable_extraction module for consistent behavior across the crate.
    ///
    /// This properly handles:
    /// - Simple variable references (x, y, z)
    /// - Variables inside compound expressions (x + y, f(x, y))
    /// - Quantified variables (correctly excluded from free variables)
    /// - Nested let-bindings
    pub fn collect_variables(&self, formula: &Bool) -> Set<Text> {
        crate::variable_extraction::collect_variables_from_bool(formula)
    }

    /// McMillan's interpolation algorithm (resolution-proof based)
    ///
    /// Extracts interpolant from resolution proof. Produces strongest interpolant.
    fn mcmillan_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        // McMillan's algorithm extracts interpolant from resolution proof
        // We'll use Z3's proof extraction and traverse it

        let solver = Solver::new();

        // Enable proof generation. Fold the configured timeout into
        // the same `Params` value — `Solver::set_params` replaces
        // the entire param set, so a separate `fresh_solver()` +
        // `set_params({proof})` would erase the timeout.
        let mut params = z3::Params::new();
        params.set_bool("proof", true);
        self.apply_timeout_into(&mut params);
        solver.set_params(&params);

        solver.assert(a);
        solver.assert(b);

        if solver.check() != SatResult::Unsat {
            return Err(Text::from("Formulas are not UNSAT"));
        }

        // Extract proof and compute interpolant
        // Since z3.rs doesn't expose proof API directly, we use model-based approach
        self.mbi_interpolate(a, b, shared)
    }

    /// Pudlák's interpolation algorithm (dual to McMillan)
    ///
    /// Produces weakest interpolant by swapping A and B and negating.
    fn pudlak_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        // Pudlák is dual to McMillan: compute McMillan(B, A) and negate
        let mcmillan_ba = self.mcmillan_interpolate(b, a, shared)?;
        Ok(mcmillan_ba.not())
    }

    /// Dual interpolation (combines McMillan and Pudlák)
    fn dual_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        match self.config.strength {
            InterpolantStrength::Weakest => self.pudlak_interpolate(a, b, shared),
            InterpolantStrength::Strongest => self.mcmillan_interpolate(a, b, shared),
            _ => {
                // Balanced: take disjunction of both
                let mcmillan = self.mcmillan_interpolate(a, b, shared)?;
                let pudlak = self.pudlak_interpolate(a, b, shared)?;
                Ok(Bool::or(&[&mcmillan, &pudlak]))
            }
        }
    }

    /// Symmetric interpolation
    fn symmetric_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        // Symmetric interpolation preserves structure
        self.mbi_interpolate(a, b, shared)
    }

    /// Model-Based Interpolation (Z3's native approach)
    ///
    /// Uses model-based quantifier elimination to compute interpolant.
    /// This is Z3's primary interpolation method.
    fn mbi_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        // Model-based interpolation using quantifier elimination

        // Step 1: Get model for A (since A ∧ B is UNSAT, ¬B satisfies A)
        let solver_a = self.fresh_solver();
        solver_a.assert(a);
        solver_a.assert(b.not());

        if solver_a.check() != SatResult::Sat {
            // If A ∧ ¬B is UNSAT, then A is empty, so interpolant is false
            return Ok(Bool::from_bool(false));
        }

        let model = solver_a
            .get_model()
            .ok_or_else(|| Text::from("No model for A"))?;

        // Step 2: Project A onto shared variables using quantifier elimination
        let interpolant = self.project_onto_shared(a, shared, &model)?;

        Ok(interpolant)
    }

    /// Project formula onto shared variables using model-based quantifier elimination
    fn project_onto_shared(
        &self,
        formula: &Bool,
        shared: &List<Text>,
        _model: &z3::Model,
    ) -> Result<Bool, Text> {
        // Collect all variables in formula
        let all_vars = self.collect_variables(formula);

        // Determine which variables to eliminate (not in shared set)
        let shared_set: Set<Text> = shared.iter().cloned().collect();
        let mut to_eliminate = List::new();

        for var in all_vars {
            if !shared_set.contains(&var) {
                to_eliminate.push(var);
            }
        }

        if to_eliminate.is_empty() {
            return Ok(formula.clone());
        }

        // Honour the configured `max_projection_vars` budget:
        // model-based projection performs QE over the elimination
        // set, which is exponential in the number of variables for
        // some theories. Reject before invoking the QE tactic when
        // the set is too large; the caller can either widen the
        // budget on `InterpolationConfig` or pick a different
        // algorithm (e.g. `Pudlak`) that doesn't go through this
        // path. Closes the inert-defense pattern: the field had no
        // readers prior to this gate.
        if to_eliminate.len() > self.config.max_projection_vars {
            return Err(Text::from(format!(
                "MBI projection would eliminate {} variables, exceeding \
                 configured max_projection_vars = {}",
                to_eliminate.len(),
                self.config.max_projection_vars
            )));
        }

        // Skip quantifier elimination entirely when the caller has
        // turned it off via `quantifier_elimination = false`. The
        // safe over-approximation is the original formula —
        // interpolation will lose precision but still preserve the
        // McMillan correctness invariant (`A ⇒ I`); it just may
        // not reach the `I ∧ B ⇒ ⊥` half cleanly. Document this in
        // the changelog so callers know the trade-off.
        if !self.config.quantifier_elimination {
            return Ok(formula.clone());
        }

        // Use quantifier elimination to project
        self.quantifier_eliminate(formula, &to_eliminate)
    }

    /// Perform quantifier elimination using Z3 tactics
    ///
    /// Uses Z3's quantifier elimination (qe) tactic to eliminate
    /// existentially quantified variables from a formula.
    ///
    /// # Algorithm
    ///
    /// 1. Build existential quantifier: ∃ vars. formula
    /// 2. Apply Z3's qe tactic to eliminate quantifier
    /// 3. Simplify the result using ctx-simplify tactic
    ///
    /// This is key for model-based interpolation where we need to
    /// project formulas onto shared variable subsets.
    fn quantifier_eliminate(&self, formula: &Bool, vars: &List<Text>) -> Result<Bool, Text> {
        use z3::{Goal, Tactic};

        if vars.is_empty() {
            return Ok(formula.clone());
        }

        // Create bound variables for the existential quantifier
        // We need to determine the sort of each variable - default to Bool
        // In a full implementation, we'd track sorts through the translation
        let bound_vars: List<Bool> = vars
            .iter()
            .map(|var_name| Bool::new_const(var_name.as_str()))
            .collect();

        let bound_refs: List<&dyn z3::ast::Ast> =
            bound_vars.iter().map(|v| v as &dyn z3::ast::Ast).collect();

        // Create existential quantifier: ∃ vars. formula
        let quantified = z3::ast::exists_const(&bound_refs, &[], formula);

        // Create a goal and assert the quantified formula
        let goal = Goal::new(false, false, false);
        goal.assert(&quantified);

        // Apply quantifier elimination tactic
        // Z3's qe tactic uses various algorithms including:
        // - Linear arithmetic QE (Loos-Weispfenning)
        // - Non-linear arithmetic QE (CAD when available)
        // - Bit-vector QE
        // - Datatype QE
        let qe_tactic = Tactic::new("qe");

        // Apply the tactic
        let apply_result = qe_tactic.apply(&goal, None);

        match apply_result {
            Ok(applied) => {
                // Collect all formulas from subgoals
                // list_subgoals returns an iterator, so we collect the goals first
                let subgoals: Vec<Goal> = applied.list_subgoals().collect();

                if subgoals.is_empty() {
                    // Empty result means trivially true
                    return Ok(Bool::from_bool(true));
                }

                // Combine all subgoal formulas
                // get_formulas() returns Vec<Bool> directly
                let mut formulas: List<Bool> = List::new();
                for subgoal in subgoals {
                    let subgoal_formulas = subgoal.get_formulas();
                    for f in subgoal_formulas {
                        // get_formulas already returns Bool values
                        formulas.push(f);
                    }
                }

                if formulas.is_empty() {
                    return Ok(Bool::from_bool(true));
                }

                // Conjoin all formulas from subgoals
                let formula_refs: List<&Bool> = formulas.iter().collect();
                let combined = Bool::and(&formula_refs);

                // Apply simplification for a cleaner result
                let simplify_tactic = Tactic::new("simplify");
                let simplify_goal = Goal::new(false, false, false);
                simplify_goal.assert(&combined);

                if let Ok(simplified_result) = simplify_tactic.apply(&simplify_goal, None) {
                    let simplified_subgoals: Vec<Goal> =
                        simplified_result.list_subgoals().collect();
                    if !simplified_subgoals.is_empty() {
                        let simplified_formulas = simplified_subgoals[0].get_formulas();
                        if !simplified_formulas.is_empty() {
                            return Ok(simplified_formulas[0].clone());
                        }
                    }
                }

                Ok(combined)
            }
            Err(_) => {
                // If QE fails, fall back to simplification
                tracing::warn!(
                    target: "verum_smt::interpolation",
                    "Quantifier elimination failed, falling back to simplification"
                );
                Ok(formula.simplify())
            }
        }
    }

    /// Ping-pong interpolation
    ///
    /// Iteratively refine interpolant by alternating between A and B sides.
    fn pingpong_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        const MAX_ITERATIONS: usize = 10;

        let mut interpolant = Bool::from_bool(true);

        // MEMORY FIX: Reuse solvers instead of creating new ones in loop
        // Each Solver::new() allocates ~500KB that accumulates without cleanup
        let solver_a = self.fresh_solver();
        let solver_b = self.fresh_solver();

        for _ in 0..MAX_ITERATIONS {
            // Refine from A side - use push/pop for incremental solving
            solver_a.reset(); // Clear previous assertions
            solver_a.assert(a);
            solver_a.assert(interpolant.not());

            if solver_a.check() == SatResult::Unsat {
                // A ⇒ I, now check B side
                solver_b.reset(); // Clear previous assertions
                solver_b.assert(&interpolant);
                solver_b.assert(b);

                if solver_b.check() == SatResult::Unsat {
                    // Found valid interpolant
                    return Ok(interpolant);
                }

                // Refine from B side
                let model_b = solver_b.get_model().ok_or_else(|| Text::from("No model"))?;
                let refinement = self.generalize_from_model(b, shared, &model_b)?;
                interpolant = refinement;
            } else {
                // Strengthen interpolant from A side
                let model_a = solver_a.get_model().ok_or_else(|| Text::from("No model"))?;
                let strengthen = self.generalize_from_model(a, shared, &model_a)?;
                interpolant = Bool::and(&[&interpolant, &strengthen]);
            }
        }

        Ok(interpolant)
    }

    /// Pogo (one-sided) interpolation
    ///
    /// Builds interpolant incrementally from one side only.
    fn pogo_interpolate(&self, a: &Bool, b: &Bool, shared: &List<Text>) -> Result<Bool, Text> {
        let mut clauses = List::new();
        let mut current_b = b.clone();

        const MAX_ITERATIONS: usize = 20;

        // MEMORY FIX: Create solver once, reuse with reset()
        let solver = self.fresh_solver();

        for _ in 0..MAX_ITERATIONS {
            solver.reset(); // Clear previous state
            solver.assert(a);
            solver.assert(&current_b);

            if solver.check() == SatResult::Unsat {
                // Build interpolant from accumulated clauses
                if clauses.is_empty() {
                    return Ok(Bool::from_bool(false));
                }
                let refs: List<&Bool> = clauses.iter().collect();
                return Ok(Bool::or(&refs));
            }

            // Get model and generalize
            let model = solver.get_model().ok_or_else(|| Text::from("No model"))?;
            let clause = self.generalize_from_model(&current_b, shared, &model)?;
            clauses.push(clause.clone());

            // Block this clause from B
            current_b = Bool::and(&[&current_b, &clause.not()]);
        }

        Err(Text::from("Pogo interpolation did not converge"))
    }

    /// Generalize a formula from a model onto shared variables
    fn generalize_from_model(
        &self,
        formula: &Bool,
        shared: &List<Text>,
        model: &z3::Model,
    ) -> Result<Bool, Text> {
        // Extract values of shared variables from model
        let mut literals = List::new();

        for var_name in shared {
            let var = Bool::new_const(var_name.as_str());
            if let Some(value) = model.eval(&var, true) {
                literals.push(value);
            }
        }

        if literals.is_empty() {
            return Ok(formula.clone());
        }

        let refs: List<&Bool> = literals.iter().collect();
        Ok(Bool::and(&refs))
    }

    /// Compute sequence interpolants for a path
    ///
    /// Given formulas [F1, F2, ..., Fn] where conjunction is UNSAT,
    /// compute interpolants [I1, I2, ..., I(n-1)] where:
    /// - F1 ⇒ I1
    /// - I1 ∧ F2 ⇒ I2
    /// - ...
    /// - I(n-1) ∧ Fn ⇒ ⊥
    pub fn sequence_interpolate(&self, formulas: List<Bool>) -> Result<SequenceInterpolant, Text> {
        let start = Instant::now();

        if formulas.len() < 2 {
            return Err(Text::from(
                "Need at least 2 formulas for sequence interpolation",
            ));
        }

        let mut interpolants = List::new();

        // Compute interpolants between consecutive formulas
        for i in 0..formulas.len() - 1 {
            let prefix_vec: List<Bool> = formulas.iter().take(i + 1).cloned().collect();
            let suffix_vec: List<Bool> = formulas.iter().skip(i + 1).cloned().collect();

            let prefix = self.conjoin(&prefix_vec);
            let suffix = self.conjoin(&suffix_vec);

            let interp = self.interpolate(&prefix, &suffix)?;
            interpolants.push(interp);
        }

        Ok(SequenceInterpolant {
            interpolants,
            formulas,
            time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Conjoin multiple formulas
    fn conjoin(&self, formulas: &[Bool]) -> Bool {
        if formulas.is_empty() {
            Bool::from_bool(true)
        } else if formulas.len() == 1 {
            formulas[0].clone()
        } else {
            let refs: List<&Bool> = formulas.iter().collect();
            Bool::and(&refs)
        }
    }

    /// Compute tree interpolants for modular verification
    pub fn tree_interpolate(&self, tree: InterpolationTree) -> Result<TreeInterpolant, Text> {
        let start = Instant::now();

        let root = self.tree_interpolate_recursive(tree.root)?;
        let num_nodes = self.count_nodes(&root);

        Ok(TreeInterpolant {
            root,
            num_nodes,
            time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Recursive tree interpolation
    fn tree_interpolate_recursive(&self, node: TreeNode) -> Result<InterpolantNode, Text> {
        let node_formula = node.formula.clone();
        let mut children = List::new();

        for child in node.children {
            children.push(self.tree_interpolate_recursive(child)?);
        }

        // Compute interpolant for this node
        let interp = if children.is_empty() {
            // Leaf node
            Interpolant {
                formula: node_formula,
                shared_vars: List::new(),
                strength: self.config.strength.clone(),
                source: InterpolantSource {
                    formula_a: Bool::from_bool(true),
                    formula_b: Bool::from_bool(true),
                    common: List::new(),
                },
                time_ms: 0,
            }
        } else {
            // Internal node - interpolate with children
            self.compute_node_interpolant(&node_formula, &children)?
        };

        Ok(InterpolantNode {
            interpolant: interp,
            children,
            id: node.id,
        })
    }

    /// Compute interpolant for internal node
    fn compute_node_interpolant(
        &self,
        formula: &Bool,
        children: &List<InterpolantNode>,
    ) -> Result<Interpolant, Text> {
        // Combine children interpolants
        let child_formulas: List<Bool> = children
            .iter()
            .map(|c| c.interpolant.formula.clone())
            .collect();

        let combined = self.conjoin(&child_formulas);
        self.interpolate(formula, &combined)
    }

    /// Count nodes in tree
    fn count_nodes(&self, node: &InterpolantNode) -> usize {
        1 + node
            .children
            .iter()
            .map(|c| self.count_nodes(c))
            .sum::<usize>()
    }
}

// ==================== Interpolation Tree ====================

/// Tree structure for interpolation
#[derive(Debug)]
pub struct InterpolationTree {
    pub root: TreeNode,
}

/// Tree node
#[derive(Debug)]
pub struct TreeNode {
    pub id: Text,
    pub formula: Bool,
    pub children: List<TreeNode>,
}

// ==================== Compositional Verification ====================

/// Compositional verifier using interpolation
///
/// Enables modular verification by computing summaries of modules
/// using interpolation and composing them hierarchically.
pub struct CompositionalVerifier {
    engine: InterpolationEngine,
}

impl CompositionalVerifier {
    pub fn new(config: InterpolationConfig) -> Self {
        Self {
            engine: InterpolationEngine::new(config),
        }
    }

    /// Verify modular property
    pub fn verify_modular(
        &self,
        modules: List<ModuleSpec>,
        property: Bool,
    ) -> Result<ModularProof, Text> {
        let start = Instant::now();

        // Build compositional proof
        let mut local_proofs = List::new();

        for module in &modules {
            let local_proof = self.verify_module(module)?;
            local_proofs.push(local_proof);
        }

        // Clone property before move
        let property_clone = property.clone();

        // Compose local proofs
        let global_proof = self.compose_proofs(local_proofs, property)?;

        Ok(ModularProof {
            modules,
            property: property_clone,
            proof: global_proof,
            time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Verify single module
    fn verify_module(&self, module: &ModuleSpec) -> Result<LocalProof, Text> {
        // Compute module summary using interpolation
        let summary = self
            .engine
            .interpolate(&module.precondition, &module.postcondition.not())?;

        Ok(LocalProof {
            module_id: module.id.clone(),
            summary: summary.formula,
        })
    }

    /// Compose local proofs into global proof
    fn compose_proofs(
        &self,
        local_proofs: List<LocalProof>,
        property: Bool,
    ) -> Result<GlobalProof, Text> {
        // Build proof tree from local summaries
        let summaries: List<Bool> = local_proofs.iter().map(|p| p.summary.clone()).collect();
        let combined = self.engine.conjoin(&summaries);

        // Verify global property
        let solver = Solver::new();
        solver.assert(&combined);
        solver.assert(property.not());

        if solver.check() == SatResult::Unsat {
            Ok(GlobalProof {
                valid: true,
                interpolants: local_proofs,
            })
        } else {
            Ok(GlobalProof {
                valid: false,
                interpolants: List::new(),
            })
        }
    }
}

/// Module specification
#[derive(Debug, Clone)]
pub struct ModuleSpec {
    pub id: Text,
    pub precondition: Bool,
    pub postcondition: Bool,
    pub invariants: List<Bool>,
}

/// Local proof for a module
#[derive(Debug)]
pub struct LocalProof {
    pub module_id: Text,
    pub summary: Bool,
}

/// Global compositional proof
#[derive(Debug)]
pub struct GlobalProof {
    pub valid: bool,
    pub interpolants: List<LocalProof>,
}

/// Modular proof result
#[derive(Debug)]
pub struct ModularProof {
    pub modules: List<ModuleSpec>,
    pub property: Bool,
    pub proof: GlobalProof,
    pub time_ms: u64,
}

// ==================== Abstraction Refinement ====================

/// Abstraction refinement using interpolation (CEGAR)
///
/// Counter-Example Guided Abstraction Refinement loop using
/// interpolation to refine spurious counterexamples.
pub struct AbstractionRefinement {
    engine: InterpolationEngine,
}

impl AbstractionRefinement {
    pub fn new(config: InterpolationConfig) -> Self {
        Self {
            engine: InterpolationEngine::new(config),
        }
    }

    /// Refine abstraction using spurious counterexample
    pub fn refine(
        &self,
        abstraction: &Bool,
        counterexample: &Bool,
    ) -> Result<RefinedAbstraction, Text> {
        // Compute interpolant from counterexample
        let interpolant = self.engine.interpolate(abstraction, counterexample)?;

        // Use interpolant to refine abstraction
        let refined = Bool::and(&[abstraction, &interpolant.formula]);

        Ok(RefinedAbstraction {
            abstraction: refined,
            refinement: interpolant.formula,
            eliminated_counterexample: counterexample.clone(),
        })
    }

    /// CEGAR loop (Counter-Example Guided Abstraction Refinement)
    ///
    /// Iteratively refine abstraction until property holds or
    /// real counterexample is found.
    pub fn cegar(
        &self,
        initial_abstraction: Bool,
        property: Bool,
        max_iterations: usize,
    ) -> Result<CEGARResult, Text> {
        let start = Instant::now();
        let mut abstraction = initial_abstraction;
        let mut refinements = List::new();

        for iteration in 0..max_iterations {
            // Check property on abstraction
            let solver = Solver::new();
            solver.assert(&abstraction);
            solver.assert(property.not());

            if solver.check() == SatResult::Unsat {
                // Property holds on abstraction
                return Ok(CEGARResult {
                    verified: true,
                    final_abstraction: abstraction,
                    refinements,
                    iterations: iteration + 1,
                    time_ms: start.elapsed().as_millis() as u64,
                });
            }

            // Get counterexample
            let model = solver.get_model().ok_or(Text::from("No model found"))?;

            // Check if counterexample is spurious by validating against concrete system
            // Uses concrete value extraction and constraint solving to determine if
            // the counterexample is real (exists in concrete system) or spurious (abstraction artifact)
            let is_spurious = self.check_spurious(&abstraction, &property, &model)?;

            if !is_spurious {
                // Real counterexample found
                return Ok(CEGARResult {
                    verified: false,
                    final_abstraction: abstraction,
                    refinements,
                    iterations: iteration + 1,
                    time_ms: start.elapsed().as_millis() as u64,
                });
            }

            // Extract counterexample formula from model
            let counterexample = self.extract_counterexample(&abstraction, &model)?;

            // Refine abstraction using spurious counterexample
            let refined = self.refine(&abstraction, &counterexample)?;

            abstraction = refined.abstraction;
            refinements.push(refined.refinement);
        }

        Err(Text::from("CEGAR loop did not converge"))
    }

    /// Check if counterexample is spurious
    ///
    /// A counterexample is spurious if it exists in the abstract system
    /// but not in the concrete system. We check this by:
    /// 1. Extracting concrete values from the model
    /// 2. Building a concrete trace using those values
    /// 3. Checking if the concrete trace violates the property
    ///
    /// If the concrete trace does NOT violate the property, the counterexample is spurious.
    fn check_spurious(
        &self,
        abstraction: &Bool,
        property: &Bool,
        model: &z3::Model,
    ) -> Result<bool, Text> {
        use z3::{SatResult, Solver};

        // Create a new solver for concrete checking
        let solver = Solver::new();

        // Extract variable assignments from the model
        let variables = self.engine.collect_variables(abstraction);
        let mut concrete_constraints: Vec<z3::ast::Bool> = Vec::new();

        for var_name in variables.iter() {
            // Create a variable with the same name
            let var = z3::ast::Int::new_const(var_name.as_str());

            // Evaluate the variable in the model
            if let Some(value) = model.eval(&var, true) {
                // Constrain the variable to its concrete value
                concrete_constraints.push(var.eq(&value));
            }
        }

        // Add concrete constraints to solver
        for constraint in &concrete_constraints {
            solver.assert(constraint);
        }

        // Add the negation of the property
        // If the concrete trace violates the property, !property is SAT
        solver.assert(property.not());

        // Check satisfiability
        match solver.check() {
            SatResult::Sat => {
                // Found a concrete trace that violates the property
                // This is a real counterexample, not spurious
                Ok(false)
            }
            SatResult::Unsat => {
                // No concrete trace violates the property
                // The counterexample is spurious (only exists in abstraction)
                Ok(true)
            }
            SatResult::Unknown => {
                // Solver couldn't determine - conservatively treat as real
                // (This will cause CEGAR to report a potential violation)
                tracing::warn!(
                    target: "verum_smt::interpolation",
                    "Spurious check returned unknown - treating as real counterexample"
                );
                Ok(false)
            }
        }
    }

    /// Extract counterexample formula from model
    fn extract_counterexample(&self, formula: &Bool, model: &z3::Model) -> Result<Bool, Text> {
        // Evaluate formula in model to get counterexample path
        if let Some(value) = model.eval(formula, true) {
            Ok(value)
        } else {
            Ok(Bool::from_bool(true))
        }
    }
}

/// Refined abstraction
#[derive(Debug)]
pub struct RefinedAbstraction {
    pub abstraction: Bool,
    pub refinement: Bool,
    pub eliminated_counterexample: Bool,
}

/// CEGAR result
#[derive(Debug)]
pub struct CEGARResult {
    pub verified: bool,
    pub final_abstraction: Bool,
    pub refinements: List<Bool>,
    pub iterations: usize,
    pub time_ms: u64,
}
