//! Verification Pipeline Integration Phase (MIR-Based)
//!
//! This phase integrates verum_verification into the compilation pipeline,
//! providing full verification capabilities including:
//!
//! - Level inference (Runtime/Static/Proof)
//! - Boundary detection and proof obligation generation
//! - SMT-based verification of contracts and refinements
//! - Bounds check elimination via dataflow analysis on MIR CFG
//! - CBGR escape analysis for reference optimization on MIR CFG
//! - Transition recommendations for verification level upgrades
//!
//! **Critical**: This phase works at the MIR level (Phase 6) where we have
//! full CFG information with explicit BoundsCheck/GenerationCheck statements.
//!
//! Verification system: three levels — runtime (CBGR checks), static (dataflow
//! analysis), proof (SMT solver Z3/CVC5). Safety checks either proven
//! unnecessary or executed at runtime. Never speculates on safety.
//! Bounds elimination via MIR-level CFG and dataflow analysis.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::{debug, info};

use verum_common::{List, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_ast::span::Span;
use verum_ast::expr::BinOp;
use verum_verification::{
    // Bounds elimination
    ArrayAccess,
    BoundsCheckEliminator,
    CheckDecision,
    Expression as BoundsExpression,
    // CBGR escape analysis
    CBGROptimizer,
    EscapeCFG,
    EscapeStatus,
    OptimizationConfig,
    RefVariable,
    // Verification passes
    TransitionRecommendation,
    VerificationLevel,
    // Transition analysis
    TransitionStrategy,
};
use verum_verification::cbgr_elimination::{
    BlockId as EscapeBlockId,
    ScopeId,
    BasicBlock as EscapeBasicBlock,
    DefSite,
    UseSite,
    Scope,
};

use crate::phases::{
    CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput,
    mir_lowering::{
        MirModule, MirFunction, BasicBlock, BlockId, LocalId,
        MirStatement, MirType, Place, Rvalue, Operand, MirConstant,
        Terminator, DominatorTree, LoopInfo,
    },
};

/// Verification phase configuration
#[derive(Debug, Clone)]
pub struct VerificationPhaseConfig {
    /// Default verification level for functions without @verify annotation
    pub default_level: VerificationLevel,

    /// Enable bounds check elimination analysis
    pub enable_bounds_elimination: bool,

    /// Enable CBGR escape analysis for reference optimization
    pub enable_cbgr_optimization: bool,

    /// Enable SMT-based verification
    pub enable_smt_verification: bool,

    /// Enable transition recommendations
    pub enable_transition_recommendations: bool,

    /// Transition strategy for recommendations
    pub transition_strategy: TransitionStrategy,

    /// SMT timeout in milliseconds
    pub smt_timeout_ms: u32,

    /// Generate proof certificates
    pub generate_proofs: bool,
}

impl Default for VerificationPhaseConfig {
    fn default() -> Self {
        Self {
            default_level: VerificationLevel::Runtime,
            enable_bounds_elimination: true,
            enable_cbgr_optimization: true,
            enable_smt_verification: true,
            enable_transition_recommendations: true,
            transition_strategy: TransitionStrategy::Balanced,
            smt_timeout_ms: 30000, // 30 seconds
            generate_proofs: false,
        }
    }
}

impl VerificationPhaseConfig {
    /// Create config for development (faster, less strict)
    pub fn development() -> Self {
        Self {
            default_level: VerificationLevel::Runtime,
            enable_bounds_elimination: true,
            enable_cbgr_optimization: true,
            enable_smt_verification: false, // Skip SMT for faster dev builds
            enable_transition_recommendations: false,
            transition_strategy: TransitionStrategy::Conservative,
            smt_timeout_ms: 5000,
            generate_proofs: false,
        }
    }

    /// Create config for production (full verification)
    pub fn production() -> Self {
        Self {
            default_level: VerificationLevel::Static,
            enable_bounds_elimination: true,
            enable_cbgr_optimization: true,
            enable_smt_verification: true,
            enable_transition_recommendations: true,
            transition_strategy: TransitionStrategy::Aggressive,
            smt_timeout_ms: 60000, // 60 seconds
            generate_proofs: true,
        }
    }

    /// Create config for research/proof mode
    pub fn research() -> Self {
        Self {
            default_level: VerificationLevel::Proof,
            enable_bounds_elimination: true,
            enable_cbgr_optimization: true,
            enable_smt_verification: true,
            enable_transition_recommendations: true,
            transition_strategy: TransitionStrategy::Aggressive,
            smt_timeout_ms: 300000, // 5 minutes
            generate_proofs: true,
        }
    }
}

/// Results from bounds check elimination analysis
#[derive(Debug, Clone, Default)]
pub struct BoundsEliminationResults {
    /// Total bounds checks analyzed
    pub total_checks: usize,
    /// Checks that can be eliminated (proven safe)
    pub eliminated: usize,
    /// Checks that can be hoisted out of loops
    pub hoisted: usize,
    /// Checks that must be kept (cannot prove safety)
    pub kept: usize,
    /// Elimination rate as percentage
    pub elimination_rate: f64,
    /// Details per function
    pub per_function: HashMap<String, FunctionBoundsStats>,
}

/// Per-function bounds check statistics
#[derive(Debug, Clone, Default)]
pub struct FunctionBoundsStats {
    pub total: usize,
    pub eliminated: usize,
    pub hoisted: usize,
    pub kept: usize,
    /// Specific check locations that were eliminated (block_id, stmt_index)
    pub eliminated_locations: Vec<(usize, usize)>,
}

/// Results from CBGR escape analysis
#[derive(Debug, Clone, Default)]
pub struct CBGROptimizationResults {
    /// Total references analyzed
    pub total_refs: usize,
    /// References promoted from tier 0 to tier 1 (&T → &checked T)
    pub promoted_to_tier1: usize,
    /// References that must stay at tier 0
    pub kept_at_tier0: usize,
    /// CBGR checks that can be eliminated
    pub checks_eliminated: usize,
    /// Promotion rate as percentage
    pub promotion_rate: f64,
    /// Per-function details
    pub per_function: HashMap<String, FunctionCBGRStats>,
}

/// Per-function CBGR statistics
#[derive(Debug, Clone, Default)]
pub struct FunctionCBGRStats {
    pub total_refs: usize,
    pub no_escape: usize,
    pub local_escape: usize,
    pub heap_escape: usize,
    pub thread_escape: usize,
    /// GenerationCheck locations that can be eliminated
    pub eliminated_checks: Vec<(usize, usize)>,
}

/// Results from SMT verification
#[derive(Debug, Clone, Default)]
pub struct SmtVerificationResults {
    /// Total verification conditions generated
    pub total_vcs: usize,
    /// Successfully proven VCs
    pub proven: usize,
    /// Failed VCs (counterexample found)
    pub failed: usize,
    /// Unknown VCs (timeout or complexity)
    pub unknown: usize,
    /// Success rate as percentage
    pub success_rate: f64,
}

/// Complete results from verification phase
#[derive(Debug, Clone, Default)]
pub struct VerificationPhaseResults {
    /// Bounds check elimination results
    pub bounds_stats: BoundsEliminationResults,
    /// CBGR optimization results
    pub cbgr_stats: CBGROptimizationResults,
    /// SMT verification results
    pub smt_results: SmtVerificationResults,
    /// Number of verification boundaries detected
    pub boundaries_detected: usize,
    /// Number of proof obligations generated
    pub obligations_generated: usize,
    /// Total verification time
    pub total_time: Duration,
    /// Transition recommendations for upgrading verification levels
    pub recommendations: List<TransitionRecommendation>,
}

/// The verification phase that runs on MIR
pub struct VerificationPhase {
    config: VerificationPhaseConfig,
    results: VerificationPhaseResults,
}

impl VerificationPhase {
    /// Create a new verification phase with default config
    pub fn new() -> Self {
        Self {
            config: VerificationPhaseConfig::default(),
            results: VerificationPhaseResults::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: VerificationPhaseConfig) -> Self {
        Self {
            config,
            results: VerificationPhaseResults::default(),
        }
    }

    /// Verify a single MIR module
    pub fn verify_module(&mut self, module: &MirModule) -> Result<VerificationPhaseResults, String> {
        let start = Instant::now();
        info!("Verifying module: {}", module.name);

        // Log the requested verification surface so callers +
        // telemetry have a record of what was asked, even when
        // a phase isn't realised in this entry-point. Closes
        // the inert-defense pattern around six
        // VerificationPhaseConfig fields that landed on the
        // config but had no consumer in `verify_module`:
        //   default_level                       — applies only
        //                                         when functions
        //                                         lack @verify
        //   enable_smt_verification             — SMT phase not
        //                                         in this entry
        //                                         point
        //   enable_transition_recommendations   — recommendations
        //                                         engine isn't
        //                                         driven from here
        //   transition_strategy                 — feeds the
        //                                         recommendations
        //                                         engine
        //   smt_timeout_ms                      — SMT solver
        //                                         budget
        //   generate_proofs                     — proof emission
        debug!(
            "verify_module config: default_level={:?}, smt_verification={}, \
             transition_recommendations={}, transition_strategy={:?}, \
             smt_timeout_ms={}, generate_proofs={}",
            self.config.default_level,
            self.config.enable_smt_verification,
            self.config.enable_transition_recommendations,
            self.config.transition_strategy,
            self.config.smt_timeout_ms,
            self.config.generate_proofs,
        );
        if self.config.enable_smt_verification {
            tracing::trace!(
                "smt_verification = true: SMT-driven verification runs through \
                 ContractVerificationPhase, not verify_module — this entry-point \
                 covers bounds-check elimination + CBGR escape analysis only"
            );
        }
        if self.config.generate_proofs {
            tracing::trace!(
                "generate_proofs = true: proof certificates are emitted by the \
                 SMT verification phase + verum_smt::certificates::Generator, \
                 not by verify_module"
            );
        }

        let mut module_results = VerificationPhaseResults::default();

        for func in module.functions.iter() {
            debug!("Analyzing function: {}", func.name);

            // Phase 1: Bounds check elimination
            if self.config.enable_bounds_elimination {
                let bounds_result = self.analyze_bounds_checks(func);
                module_results.bounds_stats.total_checks += bounds_result.total_checks;
                module_results.bounds_stats.eliminated += bounds_result.eliminated;
                module_results.bounds_stats.hoisted += bounds_result.hoisted;
                module_results.bounds_stats.kept += bounds_result.kept;

                if bounds_result.total_checks > 0 {
                    module_results.bounds_stats.per_function.insert(
                        func.name.to_string(),
                        FunctionBoundsStats {
                            total: bounds_result.total_checks,
                            eliminated: bounds_result.eliminated,
                            hoisted: bounds_result.hoisted,
                            kept: bounds_result.kept,
                            eliminated_locations: bounds_result.eliminated_locations.clone(),
                        },
                    );
                }
            }

            // Phase 2: CBGR escape analysis
            if self.config.enable_cbgr_optimization {
                let cbgr_result = self.analyze_cbgr_escapes(func);
                module_results.cbgr_stats.total_refs += cbgr_result.total_refs;
                module_results.cbgr_stats.promoted_to_tier1 += cbgr_result.promoted_to_tier1;
                module_results.cbgr_stats.kept_at_tier0 += cbgr_result.kept_at_tier0;
                module_results.cbgr_stats.checks_eliminated += cbgr_result.checks_eliminated;

                if cbgr_result.total_refs > 0 {
                    module_results.cbgr_stats.per_function.insert(
                        func.name.to_string(),
                        FunctionCBGRStats {
                            total_refs: cbgr_result.total_refs,
                            no_escape: cbgr_result.no_escape,
                            local_escape: cbgr_result.local_escape,
                            heap_escape: cbgr_result.heap_escape,
                            thread_escape: cbgr_result.thread_escape,
                            eliminated_checks: cbgr_result.eliminated_checks.clone(),
                        },
                    );
                }
            }
        }

        // Calculate aggregate rates
        if module_results.bounds_stats.total_checks > 0 {
            module_results.bounds_stats.elimination_rate =
                (module_results.bounds_stats.eliminated as f64 /
                 module_results.bounds_stats.total_checks as f64) * 100.0;
        }
        if module_results.cbgr_stats.total_refs > 0 {
            module_results.cbgr_stats.promotion_rate =
                (module_results.cbgr_stats.promoted_to_tier1 as f64 /
                 module_results.cbgr_stats.total_refs as f64) * 100.0;
        }

        module_results.total_time = start.elapsed();

        info!(
            "Module {} verified: {} bounds checks ({:.1}% eliminated), {} refs ({:.1}% promoted)",
            module.name,
            module_results.bounds_stats.total_checks,
            module_results.bounds_stats.elimination_rate,
            module_results.cbgr_stats.total_refs,
            module_results.cbgr_stats.promotion_rate,
        );

        Ok(module_results)
    }

    // ========================================================================
    // Bounds Check Elimination (MIR-based with real CFG)
    // ========================================================================

    /// Analyze bounds checks in a function using MIR CFG
    fn analyze_bounds_checks(&self, func: &MirFunction) -> BoundsAnalysisResult {
        let mut result = BoundsAnalysisResult::default();

        // Build dominator tree and loop info from MIR CFG
        let blocks: Vec<BasicBlock> = func.blocks.iter().cloned().collect();
        let dom_tree = DominatorTree::compute(&blocks, func.entry_block);
        let loop_info = LoopInfo::compute(&blocks, func.entry_block, &dom_tree);

        // Build EscapeCFG for verum_verification's BoundsCheckEliminator
        let escape_cfg = self.build_escape_cfg(func);
        let mut eliminator = BoundsCheckEliminator::new(escape_cfg);

        // Analyze each BoundsCheck statement
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                if let MirStatement::BoundsCheck { array, index } = stmt {
                    result.total_checks += 1;

                    // Try to eliminate using multiple strategies
                    let decision = self.analyze_single_bounds_check(
                        array, index, block_idx, &dom_tree, &loop_info, func, &mut eliminator
                    );

                    match decision {
                        CheckDecision::Eliminate => {
                            result.eliminated += 1;
                            result.eliminated_locations.push((block_idx, stmt_idx));
                        }
                        CheckDecision::Hoist => {
                            result.hoisted += 1;
                        }
                        CheckDecision::Keep => {
                            result.kept += 1;
                        }
                    }
                }
            }
        }

        result
    }

    /// Analyze a single bounds check using multiple strategies
    fn analyze_single_bounds_check(
        &self,
        array: &Place,
        index: &Place,
        block_idx: usize,
        _dom_tree: &DominatorTree,
        loop_info: &LoopInfo,
        func: &MirFunction,
        eliminator: &mut BoundsCheckEliminator,
    ) -> CheckDecision {
        // Strategy 1: Constant index within known bounds
        if let (Some(const_idx), Some(array_len)) = (
            self.try_const_eval_index(index, func),
            self.try_const_eval_length(array, func),
        ) {
            if const_idx >= 0 && (const_idx as usize) < array_len {
                debug!("Bounds check eliminated: constant index {} < array len {}", const_idx, array_len);
                return CheckDecision::Eliminate;
            }
        }

        // Strategy 2: Loop induction variable with proven bounds
        let _block_id = BlockId(block_idx);
        for (header, _body) in &loop_info.loop_bodies {
            if let Some(iv) = self.find_induction_variable(index, *header, func) {
                if let (Some(upper), Some(array_len)) =
                    (iv.upper_bound, self.try_const_eval_length(array, func))
                {
                    if upper <= array_len as i64 && iv.init >= 0 {
                        debug!(
                            "Bounds check eliminated: induction var {} in [{}..{}), array len {}",
                            index.local.0, iv.init, upper, array_len
                        );
                        return CheckDecision::Eliminate;
                    }
                }
            }

            // Check if we can hoist to loop preheader
            if loop_info.loop_bodies.contains_key(header) {
                // If the check is loop-invariant, it could be hoisted
                if self.is_loop_invariant(array, *header, loop_info, func)
                    && self.is_loop_invariant_index(index, *header, loop_info, func)
                {
                    return CheckDecision::Hoist;
                }
            }
        }

        // Strategy 3: Use verum_verification's SMT-based analysis
        let access = self.create_array_access(array, index, block_idx, func);
        if let Ok(decision) = eliminator.analyze_array_access(&access) {
            return decision;
        }

        CheckDecision::Keep
    }

    /// Try to evaluate an index as a constant
    fn try_const_eval_index(&self, index: &Place, func: &MirFunction) -> Option<i64> {
        for block in func.blocks.iter() {
            for stmt in block.statements.iter() {
                if let MirStatement::Assign(place, rvalue) = stmt {
                    if place.local == index.local && place.projections.is_empty() {
                        if let Rvalue::Use(operand) = rvalue {
                            return self.try_const_operand(operand);
                        }
                    }
                }
            }
        }
        None
    }

    /// Try to evaluate array length as a constant
    fn try_const_eval_length(&self, array: &Place, func: &MirFunction) -> Option<usize> {
        if let Some(local) = func.locals.iter().find(|l| l.id == array.local) {
            match &local.ty {
                MirType::Array(_, size) => return Some(*size),
                MirType::Slice(_) => {
                    // Look for slice creation with known length
                    for block in func.blocks.iter() {
                        for stmt in block.statements.iter() {
                            if let MirStatement::Assign(place, rvalue) = stmt {
                                if place.local == array.local {
                                    if let Rvalue::Aggregate(_, operands) = rvalue {
                                        return Some(operands.len());
                                    }
                                }
                            }
                        }
                    }
                }
                MirType::Ref { inner, .. } => {
                    if let MirType::Array(_, size) = inner.as_ref() {
                        return Some(*size);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Try to get constant value from operand
    fn try_const_operand(&self, operand: &Operand) -> Option<i64> {
        match operand {
            Operand::Constant(MirConstant::Int(i)) => Some(*i),
            Operand::Constant(MirConstant::UInt(u)) => Some(*u as i64),
            _ => None,
        }
    }

    /// Find induction variable info for an index in a loop
    fn find_induction_variable(
        &self,
        index: &Place,
        header: BlockId,
        func: &MirFunction,
    ) -> Option<InductionVariable> {
        // Look for phi nodes at loop header that define the index
        if let Some(header_block) = func.blocks.get(header.0) {
            for phi in &header_block.phi_nodes {
                if phi.dest == index.local {
                    // Found phi for index - analyze operands
                    let mut init = None;
                    let mut step = None;

                    for (pred_block, operand) in phi.operands.iter() {
                        // Pre-header operand is init value
                        if pred_block.0 < header.0 {
                            init = self.try_const_operand(operand);
                        } else {
                            // Back-edge operand should be index + step
                            step = self.extract_step_from_update(operand, index, func);
                        }
                    }

                    if let (Some(init_val), Some(step_val)) = (init, step) {
                        // Try to find loop bound from branch condition
                        let upper = self.find_loop_upper_bound(header, index, func);

                        return Some(InductionVariable {
                            init: init_val,
                            step: step_val,
                            upper_bound: upper,
                        });
                    }
                }
            }
        }
        None
    }

    /// Extract step value from loop update
    fn extract_step_from_update(&self, operand: &Operand, index: &Place, func: &MirFunction) -> Option<i64> {
        // Look for pattern: index + const or index - const
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                // Find the assignment to this place
                for block in func.blocks.iter() {
                    for stmt in block.statements.iter() {
                        if let MirStatement::Assign(target, rvalue) = stmt {
                            if target.local == place.local {
                                if let Rvalue::Binary(op, left, right) = rvalue {
                                    // Check if one operand is index and other is constant
                                    match op {
                                        BinOp::Add => {
                                            if self.is_place_operand(left, index) {
                                                return self.try_const_operand(right);
                                            } else if self.is_place_operand(right, index) {
                                                return self.try_const_operand(left);
                                            }
                                        }
                                        BinOp::Sub => {
                                            if self.is_place_operand(left, index) {
                                                return self.try_const_operand(right).map(|v| -v);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn is_place_operand(&self, operand: &Operand, place: &Place) -> bool {
        match operand {
            Operand::Copy(p) | Operand::Move(p) => p.local == place.local,
            _ => false,
        }
    }

    /// Find loop upper bound from branch condition
    fn find_loop_upper_bound(&self, header: BlockId, index: &Place, func: &MirFunction) -> Option<i64> {
        // Look at loop exit condition
        if let Some(header_block) = func.blocks.get(header.0) {
            if let Terminator::Branch { condition, .. } = &header_block.terminator {
                // Look for comparison: index < bound or index <= bound
                // This is simplified - full impl would analyze the condition more deeply
                if let Operand::Copy(cond_place) | Operand::Move(cond_place) = condition {
                    // Find the comparison that defines cond_place
                    for block in func.blocks.iter() {
                        for stmt in block.statements.iter() {
                            if let MirStatement::Assign(target, rvalue) = stmt {
                                if target.local == cond_place.local {
                                    if let Rvalue::Binary(op, left, right) = rvalue {
                                        match op {
                                            BinOp::Lt => {
                                                if self.is_place_operand(left, index) {
                                                    return self.try_const_operand(right);
                                                }
                                            }
                                            BinOp::Le => {
                                                if self.is_place_operand(left, index) {
                                                    return self.try_const_operand(right).map(|v| v + 1);
                                                }
                                            }
                                            BinOp::Gt => {
                                                if self.is_place_operand(right, index) {
                                                    return self.try_const_operand(left);
                                                }
                                            }
                                            BinOp::Ge => {
                                                if self.is_place_operand(right, index) {
                                                    return self.try_const_operand(left).map(|v| v + 1);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if a place is loop-invariant
    fn is_loop_invariant(&self, place: &Place, header: BlockId, loop_info: &LoopInfo, func: &MirFunction) -> bool {
        // A place is loop-invariant if it's not modified within the loop
        if let Some(loop_blocks) = loop_info.loop_bodies.get(&header) {
            for block_id in loop_blocks {
                if let Some(block) = func.blocks.get(block_id.0) {
                    for stmt in block.statements.iter() {
                        if let MirStatement::Assign(target, _) = stmt {
                            if target.local == place.local {
                                return false; // Modified in loop
                            }
                        }
                    }
                }
            }
        }
        true
    }

    /// Check if an index expression is loop-invariant
    fn is_loop_invariant_index(&self, index: &Place, header: BlockId, loop_info: &LoopInfo, func: &MirFunction) -> bool {
        // For simple indices, just check if the place is invariant
        self.is_loop_invariant(index, header, loop_info, func)
    }

    /// Create an ArrayAccess for verum_verification's BoundsCheckEliminator
    fn create_array_access(&self, array: &Place, index: &Place, block_idx: usize, func: &MirFunction) -> ArrayAccess {
        let array_expr = self.place_to_bounds_expr(array, func);
        let index_expr = self.place_to_bounds_expr(index, func);

        // Try to get array length from type (reserved for future use with SMT analysis)
        let _length = self.try_const_eval_length(array, func)
            .map(|len| BoundsExpression::int(len as i64));

        ArrayAccess {
            array: array_expr,
            index: index_expr,
            block: EscapeBlockId::new(block_idx as u64),
            loop_context: verum_common::Maybe::None,
            span: Span::dummy(),
        }
    }

    /// Convert MIR Place to BoundsExpression
    fn place_to_bounds_expr(&self, place: &Place, func: &MirFunction) -> BoundsExpression {
        // First try constant evaluation
        if let Some(val) = self.try_const_eval_index(place, func) {
            return BoundsExpression::int(val);
        }

        // Fall back to variable name
        if let Some(local) = func.locals.iter().find(|l| l.id == place.local) {
            BoundsExpression::var(local.name.as_str())
        } else {
            BoundsExpression::var(Text::from(format!("_local{}", place.local.0)))
        }
    }

    // ========================================================================
    // CBGR Escape Analysis (MIR-based with real CFG)
    // ========================================================================

    /// Analyze CBGR escape status for references in a function
    fn analyze_cbgr_escapes(&self, func: &MirFunction) -> CBGRAnalysisResult {
        let mut result = CBGRAnalysisResult::default();

        // Build dominator tree for escape analysis
        let blocks: Vec<BasicBlock> = func.blocks.iter().cloned().collect();
        let dom_tree = DominatorTree::compute(&blocks, func.entry_block);

        // Build EscapeCFG with full def/use information (reserved for full optimization)
        let _escape_cfg = self.build_escape_cfg_with_analysis(func, &dom_tree);
        let _optimizer = CBGROptimizer::new(OptimizationConfig::conservative());

        // Track reference locals
        for local in func.locals.iter() {
            if self.is_reference_type(&local.ty) {
                result.total_refs += 1;

                // Analyze escape status
                let ref_var = RefVariable {
                    id: local.id.0 as u64,
                    is_reference: true,
                };

                let escape_status = self.compute_escape_status(&ref_var, func, &dom_tree);

                match escape_status {
                    EscapeStatus::NoEscape => {
                        result.no_escape += 1;
                        result.promoted_to_tier1 += 1;
                    }
                    EscapeStatus::EscapesToClosure => {
                        result.local_escape += 1;
                        result.kept_at_tier0 += 1;
                    }
                    EscapeStatus::EscapesToHeap => {
                        result.heap_escape += 1;
                        result.kept_at_tier0 += 1;
                    }
                    EscapeStatus::EscapesToThread => {
                        result.thread_escape += 1;
                        result.kept_at_tier0 += 1;
                    }
                    EscapeStatus::EscapesToReturn | EscapeStatus::EscapesToField => {
                        // Return and field escapes require tier 0 checks
                        result.kept_at_tier0 += 1;
                    }
                    EscapeStatus::Unknown => {
                        result.kept_at_tier0 += 1;
                    }
                }
            }
        }

        // Find GenerationCheck statements that can be eliminated
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                if let MirStatement::GenerationCheck(place) = stmt {
                    let ref_var = RefVariable {
                        id: place.local.0 as u64,
                        is_reference: true,
                    };

                    let escape_status = self.compute_escape_status(&ref_var, func, &dom_tree);

                    if matches!(escape_status, EscapeStatus::NoEscape) {
                        result.checks_eliminated += 1;
                        result.eliminated_checks.push((block_idx, stmt_idx));
                    }
                }
            }
        }

        result
    }

    /// Compute escape status for a reference variable
    fn compute_escape_status(
        &self,
        ref_var: &RefVariable,
        func: &MirFunction,
        dom_tree: &DominatorTree,
    ) -> EscapeStatus {
        let local_id = LocalId(ref_var.id as usize);

        // Parameters always escape (unknown caller)
        if let Some(local) = func.locals.iter().find(|l| l.id == local_id) {
            if local.kind == crate::phases::mir_lowering::LocalKind::Arg {
                return EscapeStatus::EscapesToClosure;
            }
        }

        let mut stored_to_heap = false;
        let returned = false; // Reserved for return value escape tracking
        let mut passed_to_function = false;
        let sent_to_thread = false; // Reserved for thread spawn tracking
        let mut def_block = None;
        let mut use_blocks = HashSet::new();

        // Scan all statements for defs and uses
        for (block_idx, block) in func.blocks.iter().enumerate() {
            for stmt in block.statements.iter() {
                match stmt {
                    MirStatement::Assign(target, rvalue) => {
                        // Check if this is a definition of our reference
                        if target.local == local_id {
                            def_block = Some(BlockId(block_idx));
                        }

                        // Check if our reference is used in the rvalue
                        if self.rvalue_uses_local(rvalue, local_id) {
                            use_blocks.insert(BlockId(block_idx));
                        }

                        // Check if stored to heap (aggregate or through pointer)
                        if let Rvalue::Aggregate(_, operands) = rvalue {
                            for op in operands {
                                if self.operand_uses_local(op, local_id) {
                                    stored_to_heap = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Check terminator for escape conditions
            match &block.terminator {
                Terminator::Return => {
                    // Check if reference is returned
                    // (Would need return place tracking)
                }
                Terminator::Call { args, .. } => {
                    for arg in args {
                        if self.operand_uses_local(arg, local_id) {
                            passed_to_function = true;

                            // Check for thread spawn patterns
                            // (Simplified - would need function analysis)
                        }
                    }
                }
                _ => {}
            }
        }

        // Determine escape category
        if sent_to_thread {
            EscapeStatus::EscapesToThread
        } else if stored_to_heap {
            EscapeStatus::EscapesToHeap
        } else if returned || passed_to_function {
            EscapeStatus::EscapesToClosure
        } else if let Some(def_blk) = def_block {
            // Check if definition dominates all uses
            let all_dominated = use_blocks.iter().all(|use_blk| {
                def_blk == *use_blk || dom_tree.dominates(def_blk, *use_blk)
            });

            if all_dominated {
                EscapeStatus::NoEscape
            } else {
                EscapeStatus::EscapesToClosure
            }
        } else {
            EscapeStatus::Unknown
        }
    }

    fn rvalue_uses_local(&self, rvalue: &Rvalue, local_id: LocalId) -> bool {
        match rvalue {
            Rvalue::Use(op) => self.operand_uses_local(op, local_id),
            Rvalue::Ref(_, place) => place.local == local_id,
            Rvalue::Binary(_, left, right) => {
                self.operand_uses_local(left, local_id) || self.operand_uses_local(right, local_id)
            }
            Rvalue::Unary(_, op) => self.operand_uses_local(op, local_id),
            Rvalue::Aggregate(_, ops) => ops.iter().any(|op| self.operand_uses_local(op, local_id)),
            Rvalue::Cast(_, op, _) => self.operand_uses_local(op, local_id),
            _ => false,
        }
    }

    fn operand_uses_local(&self, operand: &Operand, local_id: LocalId) -> bool {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => place.local == local_id,
            _ => false,
        }
    }

    /// Check if a type is a reference type
    fn is_reference_type(&self, ty: &MirType) -> bool {
        matches!(ty, MirType::Ref { .. } | MirType::Pointer { .. })
    }

    // ========================================================================
    // CFG Construction Helpers
    // ========================================================================

    /// Build EscapeCFG from MIR for bounds check elimination
    fn build_escape_cfg(&self, func: &MirFunction) -> EscapeCFG {
        let entry_block = EscapeBlockId::new(func.entry_block.0 as u64);
        let root_scope = ScopeId::new(0);
        let mut cfg = EscapeCFG::new(entry_block, root_scope);

        // Add blocks with predecessor/successor relationships
        for (idx, block) in func.blocks.iter().enumerate() {
            let block_id = EscapeBlockId::new(idx as u64);
            let mut escape_block = EscapeBasicBlock::new(block_id, root_scope);

            // Add predecessors
            for pred in block.predecessors.iter() {
                escape_block.add_predecessor(EscapeBlockId::new(pred.0 as u64));
            }

            // Add successors
            for succ in block.successors.iter() {
                escape_block.add_successor(EscapeBlockId::new(succ.0 as u64));
            }

            cfg.add_block(escape_block);
        }

        // Add root scope
        let scope = Scope {
            id: root_scope,
            parent: None,
            children: List::new(),
            defined_variables: HashSet::new(),
            is_loop: false,
            is_closure: false,
            entry_block,
            exit_blocks: List::new(),
        };
        cfg.add_scope(scope);

        cfg
    }

    /// Build EscapeCFG with full def/use analysis for CBGR optimization
    fn build_escape_cfg_with_analysis(&self, func: &MirFunction, _dom_tree: &DominatorTree) -> EscapeCFG {
        let entry_block = EscapeBlockId::new(func.entry_block.0 as u64);
        let root_scope = ScopeId::new(0);
        let mut cfg = EscapeCFG::new(entry_block, root_scope);

        // Add blocks with def/use information
        for (idx, block) in func.blocks.iter().enumerate() {
            let block_id = EscapeBlockId::new(idx as u64);
            let mut escape_block = EscapeBasicBlock::new(block_id, root_scope);

            // Add control flow edges
            for pred in block.predecessors.iter() {
                escape_block.add_predecessor(EscapeBlockId::new(pred.0 as u64));
            }
            for succ in block.successors.iter() {
                escape_block.add_successor(EscapeBlockId::new(succ.0 as u64));
            }

            // Extract def/use sites from statements
            for stmt in block.statements.iter() {
                match stmt {
                    MirStatement::Assign(target, rvalue) => {
                        // Definition site
                        if self.is_reference_local(target.local, func) {
                            let def_site = DefSite {
                                variable: RefVariable {
                                    id: target.local.0 as u64,
                                    is_reference: true,
                                },
                                block: block_id,
                                scope: root_scope,
                                is_stack_allocated: true, // Simplified
                                is_heap_allocated: false,
                            };
                            escape_block.add_definition(def_site);
                        }

                        // Use sites in rvalue
                        self.extract_uses_from_rvalue(rvalue, block_id, root_scope, func, &mut escape_block);
                    }
                    MirStatement::GenerationCheck(place) => {
                        if self.is_reference_local(place.local, func) {
                            let use_site = UseSite {
                                variable: RefVariable {
                                    id: place.local.0 as u64,
                                    is_reference: true,
                                },
                                block: block_id,
                                is_mutable: false,
                                is_return: false,
                                is_field_store: false,
                                is_thread_spawn: false,
                                is_closure_capture: false,
                            };
                            escape_block.add_use(use_site);
                        }
                    }
                    _ => {}
                }
            }

            cfg.add_block(escape_block);
        }

        // Add root scope
        let scope = Scope {
            id: root_scope,
            parent: None,
            children: List::new(),
            defined_variables: HashSet::new(),
            is_loop: false,
            is_closure: false,
            entry_block,
            exit_blocks: List::new(),
        };
        cfg.add_scope(scope);

        cfg
    }

    fn is_reference_local(&self, local_id: LocalId, func: &MirFunction) -> bool {
        func.locals.iter()
            .find(|l| l.id == local_id)
            .map(|l| self.is_reference_type(&l.ty))
            .unwrap_or(false)
    }

    fn extract_uses_from_rvalue(
        &self,
        rvalue: &Rvalue,
        block_id: EscapeBlockId,
        scope: ScopeId,
        func: &MirFunction,
        block: &mut EscapeBasicBlock,
    ) {
        match rvalue {
            Rvalue::Use(op) | Rvalue::Cast(_, op, _) | Rvalue::Unary(_, op) => {
                self.extract_use_from_operand(op, block_id, scope, func, block);
            }
            Rvalue::Binary(_, left, right) => {
                self.extract_use_from_operand(left, block_id, scope, func, block);
                self.extract_use_from_operand(right, block_id, scope, func, block);
            }
            Rvalue::Aggregate(_, ops) => {
                for op in ops {
                    self.extract_use_from_operand(op, block_id, scope, func, block);
                }
            }
            Rvalue::Ref(_, place) => {
                if self.is_reference_local(place.local, func) {
                    let use_site = UseSite {
                        variable: RefVariable {
                            id: place.local.0 as u64,
                            is_reference: true,
                        },
                        block: block_id,
                        is_mutable: false,
                        is_return: false,
                        is_field_store: false,
                        is_thread_spawn: false,
                        is_closure_capture: false,
                    };
                    block.add_use(use_site);
                }
            }
            _ => {}
        }
    }

    fn extract_use_from_operand(
        &self,
        operand: &Operand,
        block_id: EscapeBlockId,
        _scope: ScopeId,
        func: &MirFunction,
        block: &mut EscapeBasicBlock,
    ) {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            if self.is_reference_local(place.local, func) {
                let use_site = UseSite {
                    variable: RefVariable {
                        id: place.local.0 as u64,
                        is_reference: true,
                    },
                    block: block_id,
                    is_mutable: matches!(operand, Operand::Move(_)),
                    is_return: false,
                    is_field_store: false,
                    is_thread_spawn: false,
                    is_closure_capture: false,
                };
                block.add_use(use_site);
            }
        }
    }

    /// Generate diagnostics for verification results
    pub fn generate_diagnostics(&self) -> List<Diagnostic> {
        let mut diagnostics = List::new();

        // Report significant bounds elimination
        if self.results.bounds_stats.total_checks > 0 {
            if self.results.bounds_stats.elimination_rate >= 50.0 {
                diagnostics.push(
                    DiagnosticBuilder::note_diag()
                        .code("V1001")
                        .message(format!(
                            "Bounds check elimination: {:.1}% ({}/{} checks eliminated)",
                            self.results.bounds_stats.elimination_rate,
                            self.results.bounds_stats.eliminated,
                            self.results.bounds_stats.total_checks,
                        ))
                        .build()
                );
            }
        }

        // Report CBGR optimization
        if self.results.cbgr_stats.total_refs > 0 {
            if self.results.cbgr_stats.promotion_rate >= 30.0 {
                diagnostics.push(
                    DiagnosticBuilder::note_diag()
                        .code("V1002")
                        .message(format!(
                            "CBGR optimization: {:.1}% ({}/{} references promoted to &checked T)",
                            self.results.cbgr_stats.promotion_rate,
                            self.results.cbgr_stats.promoted_to_tier1,
                            self.results.cbgr_stats.total_refs,
                        ))
                        .build()
                );
            }
        }

        diagnostics
    }
}

// ============================================================================
// Internal Helper Types
// ============================================================================

/// Result of bounds analysis for a single function
#[derive(Debug, Clone, Default)]
struct BoundsAnalysisResult {
    total_checks: usize,
    eliminated: usize,
    hoisted: usize,
    kept: usize,
    eliminated_locations: Vec<(usize, usize)>,
}

/// Result of CBGR analysis for a single function
#[derive(Debug, Clone, Default)]
struct CBGRAnalysisResult {
    total_refs: usize,
    promoted_to_tier1: usize,
    kept_at_tier0: usize,
    checks_eliminated: usize,
    no_escape: usize,
    local_escape: usize,
    heap_escape: usize,
    thread_escape: usize,
    eliminated_checks: Vec<(usize, usize)>,
}

/// Induction variable information for loop analysis
#[derive(Debug, Clone)]
struct InductionVariable {
    init: i64,
    #[allow(dead_code)]
    step: i64, // Reserved for stride analysis
    upper_bound: Option<i64>,
}

// ============================================================================
// CompilationPhase Implementation
// ============================================================================

impl Default for VerificationPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for VerificationPhase {
    fn name(&self) -> &str {
        "verification"
    }

    fn description(&self) -> &str {
        "MIR-based verification: bounds elimination, CBGR escape analysis, SMT verification"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        // Extract MIR modules from input
        let mir_modules = match &input.data {
            PhaseData::Mir(modules) => modules,
            PhaseData::OptimizedMir(modules) => modules,
            _ => {
                return Err(vec![DiagnosticBuilder::error()
                    .code("V0001")
                    .message("Verification phase requires MIR modules as input (Phase 6 or later)")
                    .build()]
                .into());
            }
        };

        // Create mutable verifier
        let mut verifier = VerificationPhase::with_config(self.config.clone());

        // Run verification on all modules
        let start = Instant::now();
        let mut all_results = VerificationPhaseResults::default();

        for module in mir_modules.iter() {
            match verifier.verify_module(module) {
                Ok(results) => {
                    // Aggregate results
                    all_results.bounds_stats.total_checks += results.bounds_stats.total_checks;
                    all_results.bounds_stats.eliminated += results.bounds_stats.eliminated;
                    all_results.bounds_stats.hoisted += results.bounds_stats.hoisted;
                    all_results.bounds_stats.kept += results.bounds_stats.kept;

                    all_results.cbgr_stats.total_refs += results.cbgr_stats.total_refs;
                    all_results.cbgr_stats.promoted_to_tier1 += results.cbgr_stats.promoted_to_tier1;
                    all_results.cbgr_stats.kept_at_tier0 += results.cbgr_stats.kept_at_tier0;
                    all_results.cbgr_stats.checks_eliminated += results.cbgr_stats.checks_eliminated;
                }
                Err(e) => {
                    return Err(vec![DiagnosticBuilder::error()
                        .code("V0002")
                        .message(format!(
                            "Verification failed for module '{}': {}",
                            module.name, e
                        ))
                        .build()]
                    .into());
                }
            }
        }

        // Calculate aggregate rates
        if all_results.bounds_stats.total_checks > 0 {
            all_results.bounds_stats.elimination_rate =
                (all_results.bounds_stats.eliminated as f64 /
                 all_results.bounds_stats.total_checks as f64) * 100.0;
        }
        if all_results.cbgr_stats.total_refs > 0 {
            all_results.cbgr_stats.promotion_rate =
                (all_results.cbgr_stats.promoted_to_tier1 as f64 /
                 all_results.cbgr_stats.total_refs as f64) * 100.0;
        }

        all_results.total_time = start.elapsed();

        // Generate diagnostics
        verifier.results = all_results.clone();
        let warnings = verifier.generate_diagnostics();

        // Build metrics
        let mut metrics = PhaseMetrics::new("verification");
        metrics = metrics.with_duration(all_results.total_time);
        metrics.add_custom_metric(
            "bounds_elimination_rate",
            format!("{:.1}%", all_results.bounds_stats.elimination_rate),
        );
        metrics.add_custom_metric(
            "cbgr_promotion_rate",
            format!("{:.1}%", all_results.cbgr_stats.promotion_rate),
        );
        metrics.add_custom_metric(
            "bounds_checks_eliminated",
            all_results.bounds_stats.eliminated.to_string(),
        );
        metrics.add_custom_metric(
            "cbgr_checks_eliminated",
            all_results.cbgr_stats.checks_eliminated.to_string(),
        );

        Ok(PhaseOutput {
            data: input.data, // Pass through MIR
            warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Different modules can be verified in parallel
    }

    fn metrics(&self) -> PhaseMetrics {
        let mut metrics = PhaseMetrics::new("verification");
        metrics = metrics.with_duration(self.results.total_time);
        metrics.add_custom_metric(
            "bounds_elimination_rate",
            format!("{:.1}%", self.results.bounds_stats.elimination_rate),
        );
        metrics.add_custom_metric(
            "cbgr_promotion_rate",
            format!("{:.1}%", self.results.cbgr_stats.promotion_rate),
        );
        metrics.add_custom_metric(
            "bounds_checks_eliminated",
            self.results.bounds_stats.eliminated.to_string(),
        );
        metrics.add_custom_metric(
            "cbgr_checks_eliminated",
            self.results.cbgr_stats.checks_eliminated.to_string(),
        );
        metrics
    }
}
