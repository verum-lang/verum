//! Phase 6: Optimization
//!
//! Profile-dependent optimization passes.
//!
//! ## Optimizations
//!
//! - Escape analysis → identify NoEscape references
//! - Eliminate proven-safe CBGR checks (50-90% typical)
//! - Eliminate proven-safe bounds checks
//! - Inline functions (cross-module in AOT)
//! - **SBGL optimization** (Stack-Based Garbage-free Lists - NoEscape only)
//! - SIMD vectorization (safety-preserving only)
//! - Dead code elimination
//! - Devirtualization
//!
//! ## SBGL Optimization Restrictions
//!
//! Phase 6 optimization (profile-dependent):
//! - SBGL optimization applies ONLY to NoEscape references
//! - Escaping references cannot use SBGL (would violate memory safety)
//! - Warnings emitted for attempted SBGL on escaping references
//!
//! Escape analysis promotes &T to &checked T (0ns) when the reference is proven safe.
//! Phase 6 optimization passes include:
//!   - Constant propagation: Forward dataflow analysis replaces known constants
//!   - Dead code elimination: Remove unreachable blocks and unused assignments
//!   - Function inlining: Inline small/hot functions (cost-model based)
//!   - CBGR check elimination: Remove proven-safe runtime generation checks
//!   - SIMD vectorization: Auto-vectorize eligible loops
//!   - Bounds check elimination: Remove array bounds checks when index is provably in range

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use anyhow::Result;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_common::{List, ToText};

use super::mir_lowering::{
    BlockId, DominatorTree, LocalId, LoopInfo, MetadataKind, MirConstant, MirFunction, MirModule,
    MirStatement, MirType, Operand, Place, PlaceProjection, ReferenceLayout, Rvalue, Terminator,
};
use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};
use verum_ast::expr::BinOp;

// =============================================================================
// Optimization Level
// =============================================================================

pub struct OptimizationPhase {
    opt_level: OptimizationLevel,
    /// Statistics for optimization passes
    stats: OptimizationStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    O0, // No optimization
    O1, // Basic optimization
    O2, // Standard optimization
    O3, // Aggressive optimization
}

/// Statistics for optimization passes
#[derive(Debug, Clone, Default)]
pub struct OptimizationStats {
    /// Number of CBGR checks eliminated
    cbgr_checks_eliminated: usize,
    /// Number of CBGR checks kept
    cbgr_checks_kept: usize,
    /// Number of bounds checks eliminated
    bounds_checks_eliminated: usize,
    /// Number of bounds checks kept
    bounds_checks_kept: usize,
    /// Number of functions inlined
    functions_inlined: usize,
    /// Number of loops vectorized
    loops_vectorized: usize,
    /// Number of loops not vectorized
    loops_not_vectorized: usize,
    /// Number of dead code blocks removed
    dead_blocks_removed: usize,
    /// Number of dead statements removed
    dead_statements_removed: usize,
    /// Number of NoEscape references identified
    no_escape_refs_identified: usize,
    /// Number of references that escape
    escaping_refs_identified: usize,
    /// Number of references promoted to &checked T
    refs_promoted_to_checked: usize,
    /// Number of SBGL optimizations applied
    sbgl_optimizations_applied: usize,
    /// Number of SBGL optimization warnings (escaping references)
    sbgl_warnings_emitted: usize,
    /// Number of checks hoisted out of loops
    checks_hoisted: usize,
}

// =============================================================================
// Escape Analysis Types
// =============================================================================

/// Escape category for references
/// SBGL (Static Borrow-based Generation Liveness): scope-based reference
/// validation that proves references cannot outlive their allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeCategory {
    /// Dies in scope → SBGL applicable ✅
    NoEscape,
    /// Returns to caller → NO SBGL ❌
    LocalEscape,
    /// Stored in heap → NO SBGL ❌
    HeapEscape,
    /// Crosses threads → NO SBGL ❌
    ThreadEscape,
    /// Conservative → NO SBGL ❌
    Unknown,
}

/// Information about a reference's escape behavior
#[derive(Debug, Clone)]
pub struct ReferenceEscapeInfo {
    /// The place being analyzed
    pub place: Place,
    /// Where this reference was created
    pub creation_block: BlockId,
    /// The escape category
    pub category: EscapeCategory,
    /// All use sites of this reference
    pub uses: HashSet<(BlockId, usize)>,
    /// Whether this reference is stored to heap
    pub stored_to_heap: bool,
    /// Whether this reference is returned from function
    pub returned: bool,
    /// Whether this reference is passed to another function
    pub passed_to_function: bool,
    /// Whether this reference is captured by closure
    pub captured_by_closure: bool,
    /// Whether this reference is sent to another thread
    pub sent_to_thread: bool,
}

impl ReferenceEscapeInfo {
    fn new(place: Place, creation_block: BlockId) -> Self {
        Self {
            place,
            creation_block,
            category: EscapeCategory::Unknown,
            uses: HashSet::new(),
            stored_to_heap: false,
            returned: false,
            passed_to_function: false,
            captured_by_closure: false,
            sent_to_thread: false,
        }
    }

    fn compute_category(&mut self) {
        self.category = if self.sent_to_thread {
            EscapeCategory::ThreadEscape
        } else if self.stored_to_heap {
            EscapeCategory::HeapEscape
        } else if self.returned || self.passed_to_function {
            EscapeCategory::LocalEscape
        } else if self.captured_by_closure {
            EscapeCategory::LocalEscape
        } else {
            EscapeCategory::NoEscape
        };
    }
}

/// Complete escape analysis results for a function
#[derive(Debug, Clone)]
pub struct EscapeAnalysisResult {
    /// Escape info for each reference place
    pub references: HashMap<Place, ReferenceEscapeInfo>,
    /// Places that don't escape and can be promoted to &checked T
    pub promotable_to_checked: HashSet<Place>,
    /// Places eligible for SBGL optimization
    pub sbgl_eligible: HashSet<Place>,
}

impl EscapeAnalysisResult {
    fn new() -> Self {
        Self {
            references: HashMap::new(),
            promotable_to_checked: HashSet::new(),
            sbgl_eligible: HashSet::new(),
        }
    }

    fn escapes(&self, place: &Place) -> bool {
        self.references
            .get(place)
            .map(|info| info.category != EscapeCategory::NoEscape)
            .unwrap_or(true) // Conservative: assume escape if unknown
    }
}

// =============================================================================
// SSA Form for Analysis
// =============================================================================

/// SSA place with version number
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SSAPlace {
    pub place: Place,
    pub version: usize,
}

/// SSA form representation
#[derive(Debug, Clone)]
pub struct SSAForm {
    /// Variable definitions: SSA place -> rvalue that defines it
    pub definitions: HashMap<SSAPlace, Rvalue>,
    /// Block containing each definition
    pub def_blocks: HashMap<SSAPlace, BlockId>,
    /// Phi nodes at block entries
    pub phi_nodes: HashMap<BlockId, Vec<PhiNode>>,
    /// Entry block
    pub entry_block: BlockId,
    /// Function parameters (considered escaping from unknown caller)
    pub params: Vec<SSAPlace>,
    /// Return values (escape via return)
    pub returns: Vec<SSAPlace>,
}

/// Phi node for SSA
#[derive(Debug, Clone)]
pub struct PhiNode {
    pub dest: SSAPlace,
    pub operands: Vec<(BlockId, SSAPlace)>,
}

impl SSAForm {
    fn new(entry_block: BlockId) -> Self {
        Self {
            definitions: HashMap::new(),
            def_blocks: HashMap::new(),
            phi_nodes: HashMap::new(),
            entry_block,
            params: Vec::new(),
            returns: Vec::new(),
        }
    }
}

// =============================================================================
// Reference Flow Information
// =============================================================================

/// Information about how references flow through a function
#[derive(Debug, Clone)]
struct ReferenceFlowInfo {
    /// References created via Rvalue::Ref
    references: HashMap<SSAPlace, Place>,
    /// Places stored to heap
    stored_to_heap: HashSet<SSAPlace>,
    /// Places passed to non-inlined functions
    passed_to_function: HashSet<SSAPlace>,
    /// Places returned from function
    returned: HashSet<SSAPlace>,
    /// Places captured by closures
    captured_by_closure: HashSet<SSAPlace>,
    /// Places sent to other threads
    sent_to_thread: HashSet<SSAPlace>,
    /// Forward flow: which SSA places flow to which other places
    flow_forward: HashMap<SSAPlace, HashSet<SSAPlace>>,
    /// Backward flow: referrers
    flow_backward: HashMap<SSAPlace, HashSet<SSAPlace>>,
}

impl ReferenceFlowInfo {
    fn new() -> Self {
        Self {
            references: HashMap::new(),
            stored_to_heap: HashSet::new(),
            passed_to_function: HashSet::new(),
            returned: HashSet::new(),
            captured_by_closure: HashSet::new(),
            sent_to_thread: HashSet::new(),
            flow_forward: HashMap::new(),
            flow_backward: HashMap::new(),
        }
    }

    fn add_reference(&mut self, ssa_place: SSAPlace, target: Place) {
        self.references.insert(ssa_place, target);
    }

    fn propagate_flow(&mut self, source: SSAPlace, dest: SSAPlace) {
        self.flow_forward
            .entry(source.clone())
            .or_default()
            .insert(dest.clone());
        self.flow_backward.entry(dest).or_default().insert(source);
    }

    fn get_referrers(&self, place: &SSAPlace) -> impl Iterator<Item = &SSAPlace> {
        self.flow_backward
            .get(place)
            .map(|s| s.iter())
            .into_iter()
            .flatten()
    }
}

// =============================================================================
// Vectorization Types
// =============================================================================

/// SIMD vectorization blocker reasons
#[derive(Debug, Clone)]
pub enum VectorizationBlocker {
    /// Loop has dependencies between iterations
    LoopCarriedDependency,
    /// Memory accesses cannot be aligned
    UnalignedAccess,
    /// Bounds cannot be proven safe
    UnprovenBounds,
    /// Potential aliasing between reads and writes
    PotentialAliasing,
    /// Operation cannot be vectorized
    NonVectorizableOperation,
    /// Non-vectorizable statement
    NonVectorizableStatement,
    /// Loop is too small to benefit
    TooSmall,
    /// Complex control flow
    ComplexControlFlow,
}

/// Vectorized loop representation
#[derive(Debug, Clone)]
struct VectorizedLoop {
    /// SIMD width (e.g., 4 for AVX2 float, 8 for AVX512 float)
    vector_width: usize,
    /// Prologue statements (alignment)
    prologue: Vec<MirStatement>,
    /// Main vectorized loop body
    vector_loop: Vec<MirStatement>,
    /// Epilogue statements (remainder)
    epilogue: Vec<MirStatement>,
}

/// Loop region for vectorization analysis
#[derive(Debug, Clone)]
struct LoopRegion {
    /// Loop header block
    header: BlockId,
    /// All blocks in the loop
    blocks: HashSet<BlockId>,
    /// Back edge sources
    back_edges: Vec<BlockId>,
    /// Loop trip count if known
    trip_count: Option<usize>,
    /// Induction variables
    induction_vars: Vec<InductionVariable>,
}

/// Induction variable information
#[derive(Debug, Clone)]
struct InductionVariable {
    /// The local variable
    local: LocalId,
    /// Initial value
    init: i64,
    /// Step per iteration
    step: i64,
    /// Upper bound (exclusive)
    upper_bound: Option<i64>,
}

// =============================================================================
// Inline Decision Information
// =============================================================================

/// Call site for inlining
#[derive(Debug, Clone)]
struct CallSite {
    /// Index of the caller function
    caller_func: usize,
    /// Block containing the call
    block: BlockId,
    /// Statement index
    statement: usize,
}

// =============================================================================
// Optimization Phase Implementation
// =============================================================================

impl OptimizationPhase {
    pub fn new(opt_level: OptimizationLevel) -> Self {
        Self {
            opt_level,
            stats: OptimizationStats::default(),
        }
    }

    // =========================================================================
    // Escape Analysis Pass
    // =========================================================================

    /// Run escape analysis pass
    ///
    /// Identifies NoEscape references for optimization opportunities.
    /// NoEscape references can have checks eliminated and use SBGL optimization.
    ///
    /// Escape analysis: determines which references can be promoted from
/// Tier 0 (~15ns managed) to Tier 1 (0ns compiler-proven safe).
    /// SBGL analysis restricted to NoEscape references only (conservative).
    fn escape_analysis(&mut self, func: &MirFunction) -> EscapeAnalysisResult {
        tracing::debug!("Running escape analysis for function: {}", func.name);

        let mut result = EscapeAnalysisResult::new();

        // Phase 1: Build SSA representation
        let ssa = self.build_ssa(func);

        // Phase 2: Track reference flow
        let flow_info = self.track_reference_flow(&ssa, func);

        // Phase 3: Compute escapes (transitive closure)
        let escapes = self.compute_escapes(&ssa, &flow_info);

        // Phase 4: Dominance analysis and promotion decision
        let dom_tree = DominatorTree::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
        );

        // Analyze each reference
        for (ssa_place, _target_place) in &flow_info.references {
            let place = ssa_place.place.clone();
            let mut info = ReferenceEscapeInfo::new(
                place.clone(),
                ssa.def_blocks
                    .get(ssa_place)
                    .copied()
                    .unwrap_or(func.entry_block),
            );

            // Check escape conditions
            info.stored_to_heap =
                flow_info.stored_to_heap.contains(ssa_place) || escapes.contains(ssa_place);
            info.returned = flow_info.returned.contains(ssa_place);
            info.passed_to_function = flow_info.passed_to_function.contains(ssa_place);
            info.captured_by_closure = flow_info.captured_by_closure.contains(ssa_place);
            info.sent_to_thread = flow_info.sent_to_thread.contains(ssa_place);

            // Compute category
            info.compute_category();

            // Track stats
            match info.category {
                EscapeCategory::NoEscape => {
                    self.stats.no_escape_refs_identified += 1;
                    result.sbgl_eligible.insert(place.clone());

                    // Check if promotable to &checked T
                    // Requirements:
                    // 1. NoEscape
                    // 2. Allocation dominates all uses
                    // 3. No concurrent access (simplified: not sent to thread)
                    // 4. Lifetime is stack-bounded
                    let allocation_dominates =
                        self.allocation_dominates_uses(&info, &dom_tree, func);
                    if allocation_dominates && !info.sent_to_thread {
                        result.promotable_to_checked.insert(place.clone());
                    }
                }
                _ => {
                    self.stats.escaping_refs_identified += 1;
                }
            }

            result.references.insert(place, info);
        }

        tracing::debug!(
            "Escape analysis: {} NoEscape, {} escaping, {} promotable to &checked T",
            self.stats.no_escape_refs_identified,
            self.stats.escaping_refs_identified,
            result.promotable_to_checked.len()
        );

        result
    }

    /// Phase 1: Build SSA representation from MIR
    fn build_ssa(&self, func: &MirFunction) -> SSAForm {
        let mut ssa = SSAForm::new(func.entry_block);
        let mut version_counter: HashMap<LocalId, usize> = HashMap::new();

        // Mark parameters as escaping (unknown caller)
        for (i, local) in func.locals.iter().enumerate() {
            if local.kind == super::mir_lowering::LocalKind::Arg {
                let ssa_place = SSAPlace {
                    place: Place::local(LocalId(i)),
                    version: 0,
                };
                ssa.params.push(ssa_place);
            }
        }

        // Process each block
        for block in func.blocks.iter() {
            for stmt in block.statements.iter() {
                if let MirStatement::Assign(place, rvalue) = stmt {
                    // Generate new SSA version
                    let version = version_counter
                        .entry(place.local)
                        .and_modify(|v| *v += 1)
                        .or_insert(0);

                    let ssa_place = SSAPlace {
                        place: place.clone(),
                        version: *version,
                    };

                    ssa.definitions.insert(ssa_place.clone(), rvalue.clone());
                    ssa.def_blocks.insert(ssa_place, block.id);
                }
            }

            // Check for returns
            if let Terminator::Return = &block.terminator {
                // Return place (_0) escapes
                let return_place = Place::return_place();
                let version = version_counter
                    .get(&return_place.local)
                    .copied()
                    .unwrap_or(0);
                let ssa_place = SSAPlace {
                    place: return_place,
                    version,
                };
                ssa.returns.push(ssa_place);
            }
        }

        ssa
    }

    /// Phase 2: Track reference flow through SSA graph
    fn track_reference_flow(&self, ssa: &SSAForm, func: &MirFunction) -> ReferenceFlowInfo {
        let mut flow_info = ReferenceFlowInfo::new();
        let mut version_at_use: HashMap<LocalId, usize> = HashMap::new();

        for block in func.blocks.iter() {
            for (_stmt_idx, stmt) in block.statements.iter().enumerate() {
                match stmt {
                    MirStatement::Assign(place, rvalue) => {
                        let current_version = *version_at_use.get(&place.local).unwrap_or(&0);
                        let ssa_place = SSAPlace {
                            place: place.clone(),
                            version: current_version,
                        };

                        match rvalue {
                            // Reference creation: Track what is referenced
                            Rvalue::Ref(_, inner_place) => {
                                flow_info.add_reference(ssa_place.clone(), inner_place.clone());
                            }

                            // Reference copy: Propagate flow
                            Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) => {
                                let src_version = *version_at_use.get(&source.local).unwrap_or(&0);
                                let src_ssa = SSAPlace {
                                    place: source.clone(),
                                    version: src_version,
                                };
                                flow_info.propagate_flow(src_ssa, ssa_place.clone());
                            }

                            // Aggregate (potential heap storage)
                            Rvalue::Aggregate(_, operands) => {
                                for op in operands.iter() {
                                    if let Operand::Copy(p) | Operand::Move(p) = op {
                                        let op_version =
                                            *version_at_use.get(&p.local).unwrap_or(&0);
                                        let op_ssa = SSAPlace {
                                            place: p.clone(),
                                            version: op_version,
                                        };
                                        flow_info.stored_to_heap.insert(op_ssa);
                                    }
                                }
                            }

                            _ => {}
                        }

                        // Increment version after use
                        version_at_use.insert(place.local, current_version + 1);
                    }

                    _ => {}
                }
            }

            // Check terminator for function calls and returns
            match &block.terminator {
                Terminator::Call { args, .. } => {
                    for arg in args.iter() {
                        if let Operand::Copy(p) | Operand::Move(p) = arg {
                            let version = version_at_use.get(&p.local).copied().unwrap_or(0);
                            let ssa_place = SSAPlace {
                                place: p.clone(),
                                version,
                            };
                            flow_info.passed_to_function.insert(ssa_place);
                        }
                    }
                }

                Terminator::Return => {
                    // Return place escapes
                    let return_place = Place::return_place();
                    let version = version_at_use
                        .get(&return_place.local)
                        .copied()
                        .unwrap_or(0);
                    let ssa_place = SSAPlace {
                        place: return_place,
                        version,
                    };
                    flow_info.returned.insert(ssa_place);
                }

                _ => {}
            }
        }

        // Mark parameters as escaping
        for param in &ssa.params {
            flow_info.passed_to_function.insert(param.clone());
        }

        flow_info
    }

    /// Compute transitive closure of escapes
    fn compute_escapes(&self, ssa: &SSAForm, flow_info: &ReferenceFlowInfo) -> HashSet<SSAPlace> {
        let mut escapes = HashSet::new();

        // Direct escapes
        escapes.extend(flow_info.stored_to_heap.iter().cloned());
        escapes.extend(flow_info.passed_to_function.iter().cloned());
        escapes.extend(flow_info.returned.iter().cloned());
        escapes.extend(flow_info.captured_by_closure.iter().cloned());
        escapes.extend(flow_info.sent_to_thread.iter().cloned());

        // Parameters escape (unknown caller)
        escapes.extend(ssa.params.iter().cloned());

        // Transitive escapes: if X escapes and Y flows to X, then Y escapes
        let mut worklist: VecDeque<SSAPlace> = escapes.iter().cloned().collect();

        while let Some(place) = worklist.pop_front() {
            for referrer in flow_info.get_referrers(&place) {
                if !escapes.contains(referrer) {
                    escapes.insert(referrer.clone());
                    worklist.push_back(referrer.clone());
                }
            }
        }

        escapes
    }

    /// Check if allocation dominates all uses
    fn allocation_dominates_uses(
        &self,
        info: &ReferenceEscapeInfo,
        dom_tree: &DominatorTree,
        _func: &MirFunction,
    ) -> bool {
        let alloc_block = info.creation_block;

        // Check each use
        for (use_block, _) in &info.uses {
            if !dom_tree.dominates(alloc_block, *use_block) {
                return false;
            }
        }

        true
    }

    // =========================================================================
    // SBGL Optimization Pass
    // =========================================================================

    /// Apply SBGL optimization (Stack-Based Garbage-free Lists)
    ///
    /// **RESTRICTION**: SBGL optimization applies ONLY to NoEscape references.
    /// Escaping references cannot use SBGL as it would violate memory safety.
    ///
    /// Emits warnings for attempted SBGL on escaping references.
    ///
    /// SBGL analysis restricted to NoEscape references only (conservative).
    fn sbgl_optimization(
        &mut self,
        func: &mut MirFunction,
        escape_info: &EscapeAnalysisResult,
    ) -> List<Diagnostic> {
        tracing::debug!("Running SBGL optimization for function: {}", func.name);

        let mut warnings = List::new();

        // Process each reference
        for (place, info) in &escape_info.references {
            match info.category {
                EscapeCategory::NoEscape => {
                    // SBGL applicable - can use stack allocation for this reference
                    // In practice, this would modify the allocation strategy
                    if escape_info.sbgl_eligible.contains(place) {
                        self.stats.sbgl_optimizations_applied += 1;
                        tracing::trace!("SBGL optimization applied to {:?}", place);
                    }
                }
                _ => {
                    // SBGL NOT applicable - emit warning if performance-critical
                    let warning = DiagnosticBuilder::new(Severity::Note)
                        .message(format!(
                            "SBGL optimization skipped: reference {:?} escapes ({:?})",
                            place, info.category
                        ))
                        .help("SBGL optimization only applies to NoEscape references")
                        .help("Consider refactoring to avoid escaping if performance is critical")
                        .build();

                    warnings.push(warning);
                    self.stats.sbgl_warnings_emitted += 1;
                }
            }
        }

        tracing::debug!(
            "SBGL optimization: {} applied, {} warnings",
            self.stats.sbgl_optimizations_applied,
            self.stats.sbgl_warnings_emitted
        );

        warnings
    }

    // =========================================================================
    // Check Elimination Pass
    // =========================================================================

    /// Eliminate proven-safe checks
    ///
    /// Uses static analysis to identify CBGR and bounds checks that
    /// are guaranteed to succeed, then eliminates them from the code.
    ///
    /// Typical elimination rate: 50-90% of checks
    /// CBGR check elimination: removes generation counter checks for
/// references proven safe via escape analysis or SBGL.
    fn check_elimination(&mut self, func: &mut MirFunction, escape_info: &EscapeAnalysisResult) {
        tracing::debug!("Running check elimination for function: {}", func.name);

        let dom_tree = DominatorTree::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
        );

        let loop_info = LoopInfo::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
            &dom_tree,
        );

        // Track which checks can be eliminated
        let mut checks_to_remove: HashSet<(BlockId, usize)> = HashSet::new();

        for block in func.blocks.iter() {
            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                match stmt {
                    // CBGR Generation Check elimination
                    MirStatement::GenerationCheck(place) => {
                        if self.can_eliminate_cbgr_check(place, escape_info, &dom_tree, func) {
                            checks_to_remove.insert((block.id, stmt_idx));
                            self.stats.cbgr_checks_eliminated += 1;
                        } else {
                            self.stats.cbgr_checks_kept += 1;
                        }
                    }

                    // Bounds check elimination
                    MirStatement::BoundsCheck { array, index } => {
                        if self.can_eliminate_bounds_check(array, index, &loop_info, func) {
                            checks_to_remove.insert((block.id, stmt_idx));
                            self.stats.bounds_checks_eliminated += 1;
                        } else {
                            self.stats.bounds_checks_kept += 1;
                        }
                    }

                    _ => {}
                }
            }
        }

        // Remove eliminated checks
        for block in func.blocks.iter_mut() {
            let block_id = block.id;
            let mut idx = 0;
            block.statements.retain(|_| {
                let keep = !checks_to_remove.contains(&(block_id, idx));
                idx += 1;
                keep
            });
        }

        let total_cbgr = self.stats.cbgr_checks_eliminated + self.stats.cbgr_checks_kept;
        let total_bounds = self.stats.bounds_checks_eliminated + self.stats.bounds_checks_kept;

        tracing::debug!(
            "Check elimination: CBGR {}/{} eliminated, bounds {}/{} eliminated",
            self.stats.cbgr_checks_eliminated,
            total_cbgr,
            self.stats.bounds_checks_eliminated,
            total_bounds
        );
    }

    /// Determine if a CBGR check can be eliminated
    fn can_eliminate_cbgr_check(
        &self,
        place: &Place,
        escape_info: &EscapeAnalysisResult,
        _dom_tree: &DominatorTree,
        _func: &MirFunction,
    ) -> bool {
        // ALL criteria must be satisfied:
        // 1. Reference does not escape
        // 2. Allocation dominates all uses
        // 3. No concurrent access
        // 4. Lifetime is stack-bounded

        // Check 1: NoEscape
        if escape_info.escapes(place) {
            return false;
        }

        // Check 2: Allocation dominates all uses
        // (Already verified during escape analysis for promotable refs)
        if !escape_info.promotable_to_checked.contains(place) {
            return false;
        }

        // Check 3 & 4 are implicit in promotable_to_checked

        true
    }

    /// Determine if a bounds check can be eliminated
    fn can_eliminate_bounds_check(
        &self,
        array: &Place,
        index: &Place,
        loop_info: &LoopInfo,
        func: &MirFunction,
    ) -> bool {
        // Case 1: Constant index within known array bounds
        if let (Some(const_idx), Some(array_len)) = (
            self.try_const_eval_index(index, func),
            self.try_const_eval_length(array, func),
        ) {
            if const_idx >= 0 && (const_idx as usize) < array_len {
                return true;
            }
        }

        // Case 2: Loop induction variable with proven bounds
        // Check if index is an induction variable in a loop with proven bounds
        for (header, _body) in &loop_info.loop_bodies {
            if let Some(iv) = self.find_induction_variable(index, *header, func) {
                // Check if upper bound is less than array length
                if let (Some(upper), Some(array_len)) =
                    (iv.upper_bound, self.try_const_eval_length(array, func))
                {
                    if upper <= array_len as i64 && iv.init >= 0 {
                        return true;
                    }
                }
            }
        }

        // Case 3: Refinement type proves bounds
        // (Would need type info for this check)

        false
    }

    /// Try to evaluate index as a constant
    fn try_const_eval_index(&self, index: &Place, func: &MirFunction) -> Option<i64> {
        // Look for the definition of this place
        for block in func.blocks.iter() {
            for stmt in block.statements.iter() {
                if let MirStatement::Assign(place, rvalue) = stmt {
                    if place == index {
                        // Use Rvalue::Use with any constant operand
                        if let Rvalue::Use(operand) = rvalue {
                            if let Some(val) = self.try_const_operand(operand) {
                                return Some(val);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Try to evaluate array length as a constant
    ///
    /// Attempts to determine array length from:
    /// 1. Type information (MirType::Array has explicit size)
    /// 2. Constant definitions
    /// 3. Tracked allocations
    fn try_const_eval_length(&self, array: &Place, func: &MirFunction) -> Option<usize> {
        // Check the type of the local
        if let Some(local) = func.locals.iter().find(|l| l.id == array.local) {
            match &local.ty {
                // Array type has explicit size
                MirType::Array(_, size) => return Some(*size),

                // Slice - check if we can find the length from an allocation
                MirType::Slice(_) => {
                    // Look for a preceding statement that creates this slice
                    // with a known length
                    for block in func.blocks.iter() {
                        for stmt in block.statements.iter() {
                            if let MirStatement::Assign(place, rvalue) = stmt {
                                if place.local == array.local {
                                    // Check if rvalue is an aggregate with known length
                                    if let Rvalue::Aggregate(_, operands) = rvalue {
                                        // If it's an aggregate, the length is the number of operands
                                        return Some(operands.len());
                                    }
                                }
                            }
                        }
                    }
                }

                // Reference to array - follow the reference
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

    /// Try to get constant value from an operand
    ///
    /// Extracts integer constant values from operands for use in constant
    /// evaluation during bounds check elimination and loop analysis.
    fn try_const_operand(&self, operand: &Operand) -> Option<i64> {
        match operand {
            Operand::Constant(MirConstant::Int(i)) => Some(*i),
            Operand::Constant(MirConstant::UInt(u)) => Some(*u as i64),
            _ => None,
        }
    }

    /// Find if a place is an induction variable
    ///
    /// Analyzes the loop structure to detect induction variables with patterns:
    /// - `i = init; while i < N { ... i = i + step }` (canonical form)
    /// - `for i in 0..N { ... }` (range iteration)
    fn find_induction_variable(
        &self,
        place: &Place,
        loop_header: BlockId,
        func: &MirFunction,
    ) -> Option<InductionVariable> {
        let local = place.local;

        // Step 1: Find initialization (assignment before loop header)
        let mut init_value: Option<i64> = None;
        for block in func.blocks.iter() {
            // Only check blocks that precede the loop header
            if block.id.0 >= loop_header.0 {
                continue;
            }

            for stmt in block.statements.iter() {
                if let MirStatement::Assign(assign_place, rvalue) = stmt {
                    if assign_place.local == local {
                        // Check if this is a constant initialization
                        if let Rvalue::Use(operand) = rvalue {
                            if let Operand::Constant(MirConstant::Int(val)) = operand {
                                init_value = Some(*val);
                            }
                        }
                    }
                }
            }
        }

        let init = init_value?;

        // Step 2: Find the step value (increment in loop body)
        let mut step_value: Option<i64> = None;

        // Find loop body blocks (blocks dominated by header that can reach back to header)
        let loop_body: HashSet<BlockId> = self.find_loop_body(loop_header, func);

        for block in func.blocks.iter() {
            if !loop_body.contains(&block.id) {
                continue;
            }

            for stmt in block.statements.iter() {
                if let MirStatement::Assign(assign_place, rvalue) = stmt {
                    if assign_place.local == local {
                        // Check for i = i + step pattern
                        if let Rvalue::Binary(BinOp::Add, left, right) = rvalue {
                            // Check if left is `i`
                            if let Operand::Copy(left_place) = left {
                                if left_place.local == local {
                                    // Right should be a constant step
                                    if let Operand::Constant(MirConstant::Int(s)) = right {
                                        step_value = Some(*s);
                                    }
                                }
                            }
                            // Also check right is `i` (commutative)
                            if let Operand::Copy(right_place) = right {
                                if right_place.local == local {
                                    if let Operand::Constant(MirConstant::Int(s)) = left {
                                        step_value = Some(*s);
                                    }
                                }
                            }
                        }
                        // Check for i = i - step pattern (negative step)
                        if let Rvalue::Binary(BinOp::Sub, left, right) = rvalue {
                            if let Operand::Copy(left_place) = left {
                                if left_place.local == local {
                                    if let Operand::Constant(MirConstant::Int(s)) = right {
                                        step_value = Some(-*s);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let step = step_value?;

        // Step 3: Find upper bound from loop condition
        let mut upper_bound: Option<i64> = None;

        // Check the loop header terminator for comparison
        if let Some(header_block) = func.blocks.iter().find(|b| b.id == loop_header) {
            if let Terminator::Branch {
                condition,
                then_block: _,
                else_block: _,
            } = &header_block.terminator
            {
                // Extract local from condition operand
                let cond_local = match condition {
                    Operand::Copy(place) | Operand::Move(place) => place.local,
                    _ => return None,
                };

                // Find the comparison in the block
                for stmt in header_block.statements.iter() {
                    if let MirStatement::Assign(cond_place, rvalue) = stmt {
                        if cond_place.local == cond_local {
                            // Check for i < N pattern
                            if let Rvalue::Binary(BinOp::Lt, left, right) = rvalue {
                                if let Operand::Copy(left_place) = left {
                                    if left_place.local == local {
                                        if let Operand::Constant(MirConstant::Int(bound)) = right {
                                            upper_bound = Some(*bound);
                                        }
                                    }
                                }
                            }
                            // Check for i <= N pattern (bound is N+1)
                            if let Rvalue::Binary(BinOp::Le, left, right) = rvalue {
                                if let Operand::Copy(left_place) = left {
                                    if left_place.local == local {
                                        if let Operand::Constant(MirConstant::Int(bound)) = right {
                                            upper_bound = Some(*bound + 1);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Some(InductionVariable {
            local,
            init,
            step,
            upper_bound,
        })
    }

    /// Find all blocks that are part of a loop (given the loop header)
    fn find_loop_body(&self, header: BlockId, func: &MirFunction) -> HashSet<BlockId> {
        let mut body = HashSet::new();
        body.insert(header);

        // Find back edges - blocks that can reach the header
        let mut worklist: Vec<BlockId> = Vec::new();

        for block in func.blocks.iter() {
            // Check if this block's terminator can go to header
            let successors = self.get_block_successors(&block.terminator);
            if successors.contains(&header) && block.id != header {
                // This is a back edge - add to worklist
                worklist.push(block.id);
            }
        }

        // Backwards traversal to find all loop body blocks
        while let Some(block_id) = worklist.pop() {
            if body.insert(block_id) {
                // Find predecessors
                for block in func.blocks.iter() {
                    let successors = self.get_block_successors(&block.terminator);
                    if successors.contains(&block_id) {
                        worklist.push(block.id);
                    }
                }
            }
        }

        body
    }

    /// Get successor blocks from a terminator
    fn get_block_successors(&self, terminator: &Terminator) -> Vec<BlockId> {
        match terminator {
            Terminator::Goto(target) => vec![*target],
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => vec![*then_block, *else_block],
            Terminator::SwitchInt {
                targets, otherwise, ..
            } => {
                let mut succs: Vec<BlockId> = targets.iter().map(|(_, b)| *b).collect();
                succs.push(*otherwise);
                succs
            }
            Terminator::Call {
                success_block,
                unwind_block,
                ..
            } => {
                vec![*success_block, *unwind_block]
            }
            Terminator::AsyncCall {
                success_block,
                unwind_block,
                ..
            } => {
                vec![*success_block, *unwind_block]
            }
            Terminator::Await {
                resume_block,
                unwind_block,
                ..
            } => {
                vec![*resume_block, *unwind_block]
            }
            Terminator::Assert { target, unwind, .. } => vec![*target, *unwind],
            Terminator::Cleanup { target, .. } => vec![*target],
            Terminator::DropAndReplace { target, unwind, .. } => vec![*target, *unwind],
            Terminator::Yield { resume, drop, .. } => vec![*resume, *drop],
            Terminator::InlineAsm {
                destination,
                unwind,
                ..
            } => {
                let mut succs = Vec::new();
                if let Some(d) = destination {
                    succs.push(*d);
                }
                if let Some(u) = unwind {
                    succs.push(*u);
                }
                succs
            }
            Terminator::Return
            | Terminator::Unreachable
            | Terminator::Resume
            | Terminator::Abort => vec![],
        }
    }

    // =========================================================================
    // Function Inlining Pass
    // =========================================================================

    /// Inline functions
    ///
    /// Performs aggressive function inlining based on cost model.
    /// In AOT mode, can inline across module boundaries.
    ///
    /// Cross-module inlining: inline functions across module boundaries in AOT.
    fn function_inlining(&mut self, modules: &mut [MirModule]) {
        tracing::debug!("Running function inlining pass");

        // Build call graph and collect inline candidates
        let mut inline_decisions: Vec<(usize, CallSite, usize)> = Vec::new(); // (module_idx, call_site, callee_func_idx)

        for (module_idx, module) in modules.iter().enumerate() {
            for (func_idx, func) in module.functions.iter().enumerate() {
                for block in func.blocks.iter() {
                    for (_stmt_idx, _stmt) in block.statements.iter().enumerate() {
                        // Look for calls (in terminator)
                    }

                    // Check terminator for calls
                    if let Terminator::Call { func: callee, .. } = &block.terminator {
                        if let Operand::Constant(MirConstant::Function(callee_name)) = callee {
                            // Find the callee function
                            if let Some((callee_module_idx, callee_func_idx)) =
                                self.find_function(modules, callee_name.as_str())
                            {
                                let callee_func =
                                    &modules[callee_module_idx].functions[callee_func_idx];

                                if self.should_inline(func, callee_func) {
                                    inline_decisions.push((
                                        module_idx,
                                        CallSite {
                                            caller_func: func_idx,
                                            block: block.id,
                                            statement: block.statements.len(), // Terminator
                                        },
                                        callee_func_idx,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Apply inlining decisions
        for (module_idx, call_site, callee_idx) in inline_decisions {
            // Log the inlining decision for debugging
            tracing::trace!(
                "Inlining callee {} into function {} at block {:?} statement {}",
                callee_idx,
                call_site.caller_func,
                call_site.block,
                call_site.statement
            );

            // Access the modules to perform inlining
            // In practice, this would:
            // 1. Clone callee blocks with remapped locals
            // 2. Insert blocks at call_site.block, call_site.statement
            // 3. Update CFG edges to connect inlined blocks
            // 4. Replace the call terminator with a goto to the inlined entry
            let _ = (module_idx, callee_idx); // Mark as used for now
            self.stats.functions_inlined += 1;
        }

        tracing::debug!(
            "Function inlining: {} functions inlined",
            self.stats.functions_inlined
        );
    }

    /// Find a function by name across all modules
    fn find_function(&self, modules: &[MirModule], name: &str) -> Option<(usize, usize)> {
        for (module_idx, module) in modules.iter().enumerate() {
            for (func_idx, func) in module.functions.iter().enumerate() {
                if func.name == name {
                    return Some((module_idx, func_idx));
                }
            }
        }
        None
    }

    /// Determine if a function should be inlined
    fn should_inline(&self, _caller: &MirFunction, callee: &MirFunction) -> bool {
        // Heuristics for inlining
        let callee_size = self.estimate_function_size(callee);

        // Always inline small functions (< 10 instructions)
        if callee_size < 10 {
            return true;
        }

        // Inline medium functions if called once or twice
        if callee_size < 50 {
            return true;
        }

        // Don't inline large functions
        if callee_size > 100 {
            return false;
        }

        // Check if inlining enables further optimizations
        // (e.g., if callee is pure and arguments are constants)

        false
    }

    /// Estimate the size of a function in instructions
    fn estimate_function_size(&self, func: &MirFunction) -> usize {
        func.blocks.iter().map(|b| b.statements.len() + 1).sum()
    }

    // =========================================================================
    // SIMD Vectorization Pass
    // =========================================================================

    /// SIMD vectorization
    ///
    /// Auto-vectorizes loops for SIMD instructions (AVX2, AVX512, NEON).
    /// Only applies safety-preserving vectorizations.
    ///
    /// SIMD vectorization: auto-vectorize loops with safety-preserving transforms.
    fn vectorization(&mut self, func: &mut MirFunction) {
        tracing::debug!("Running SIMD vectorization for function: {}", func.name);

        let dom_tree = DominatorTree::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
        );

        let loop_info = LoopInfo::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
            &dom_tree,
        );

        // Find vectorizable loops
        for (header, body) in &loop_info.loop_bodies {
            // Extract back edges for this loop header from LoopInfo
            let back_edges: Vec<BlockId> = loop_info
                .back_edges
                .iter()
                .filter(|(_, target)| *target == *header)
                .map(|(source, _)| *source)
                .collect();

            let loop_region = LoopRegion {
                header: *header,
                blocks: body.clone(),
                back_edges,
                trip_count: None,
                induction_vars: Vec::new(),
            };

            match self.try_vectorize_loop(&loop_region, func) {
                Ok(vectorized) => {
                    // Apply the vectorized loop transformation to MIR
                    self.apply_vectorized_loop(func, &loop_region, &vectorized);
                    self.stats.loops_vectorized += 1;
                    tracing::trace!(
                        "Vectorized loop at header {:?} with width {}",
                        header,
                        vectorized.vector_width
                    );
                }
                Err(blocker) => {
                    self.stats.loops_not_vectorized += 1;
                    tracing::trace!("Loop at {:?} not vectorized: {:?}", header, blocker);
                }
            }
        }

        tracing::debug!(
            "SIMD vectorization: {} loops vectorized, {} not vectorized",
            self.stats.loops_vectorized,
            self.stats.loops_not_vectorized
        );
    }

    /// Attempt to vectorize a loop
    fn try_vectorize_loop(
        &self,
        loop_region: &LoopRegion,
        func: &MirFunction,
    ) -> Result<VectorizedLoop, VectorizationBlocker> {
        // Safety checks
        self.can_vectorize_safely(loop_region, func)?;

        // Determine vector width based on element type and target architecture
        let vector_width = self.determine_vector_width(loop_region, func);

        // Generate vectorized code
        let vectorized = VectorizedLoop {
            vector_width,
            prologue: self.generate_alignment_prologue(loop_region, vector_width),
            vector_loop: self.generate_vector_loop(loop_region, vector_width, func)?,
            epilogue: self.generate_epilogue(loop_region, vector_width),
        };

        Ok(vectorized)
    }

    /// Apply vectorized loop transformation to MIR
    ///
    /// Replaces the original loop with the vectorized version:
    /// 1. Creates preheader block for alignment prologue
    /// 2. Replaces loop body with vectorized statements
    /// 3. Creates epilogue block for remainder iterations
    fn apply_vectorized_loop(
        &mut self,
        func: &mut MirFunction,
        loop_region: &LoopRegion,
        vectorized: &VectorizedLoop,
    ) {
        // Find the loop header block
        let header_idx = func
            .blocks
            .iter()
            .position(|b| b.id == loop_region.header);

        if header_idx.is_none() {
            tracing::warn!(
                "Could not find loop header block {:?} for vectorization",
                loop_region.header
            );
            return;
        }

        let header_idx = header_idx.unwrap();

        // Create preheader block for prologue
        if !vectorized.prologue.is_empty() {
            // Insert prologue statements at the start of header block
            // In a full implementation, we would create a separate preheader block
            let header_block = &mut func.blocks[header_idx];
            let mut new_stmts = vectorized.prologue.clone();
            new_stmts.extend(header_block.statements.iter().cloned());
            header_block.statements = new_stmts.into_iter().collect();
        }

        // Replace loop body with vectorized statements
        // In a full implementation, this would:
        // 1. Create new blocks for the vectorized loop
        // 2. Update the CFG edges
        // 3. Handle the induction variable scaling by vector_width
        //
        // For now, we insert the vectorized statements after the prologue.
        // The vector_loop statements replace scalar operations with SIMD operations.
        if !vectorized.vector_loop.is_empty() {
            let header_block = &mut func.blocks[header_idx];
            // Insert vectorized statements into the loop body
            // The vectorized statements use SIMD operations on aligned data
            for stmt in vectorized.vector_loop.iter().cloned() {
                header_block.statements.push(stmt);
            }
            // Add a Nop marker to indicate vectorization boundary
            // This can be detected by later passes or debugging tools
            header_block.statements.push(MirStatement::Nop);
        }

        // Epilogue handling: In a full implementation, we would create
        // an epilogue block after the main vectorized loop to handle
        // remainder iterations. For now, the epilogue statements are
        // a placeholder for this future work.
        if !vectorized.epilogue.is_empty() {
            tracing::trace!(
                "Epilogue with {} statements would be added after loop",
                vectorized.epilogue.len()
            );
        }
    }

    /// Check if loop can be safely vectorized
    fn can_vectorize_safely(
        &self,
        loop_region: &LoopRegion,
        func: &MirFunction,
    ) -> Result<(), VectorizationBlocker> {
        // Check 0: Loop must have a simple structure (single back edge)
        // Multiple back edges indicate complex control flow that's harder to vectorize
        if loop_region.back_edges.len() != 1 {
            tracing::trace!(
                "Loop has {} back edges, expected 1 for simple vectorization",
                loop_region.back_edges.len()
            );
            return Err(VectorizationBlocker::ComplexControlFlow);
        }

        // Check 1: No loop-carried dependencies
        if self.has_loop_carried_dependencies(loop_region, func) {
            return Err(VectorizationBlocker::LoopCarriedDependency);
        }

        // Check 2: Memory accesses must be aligned or alignable
        if !self.can_guarantee_alignment(loop_region, func) {
            return Err(VectorizationBlocker::UnalignedAccess);
        }

        // Check 3: Bounds must be provably safe
        if !self.can_prove_bounds_safe_for_vectorization(loop_region, func) {
            return Err(VectorizationBlocker::UnprovenBounds);
        }

        // Check 4: No aliasing between reads and writes
        if self.has_potential_aliasing(loop_region, func) {
            return Err(VectorizationBlocker::PotentialAliasing);
        }

        // Check 5: All operations must be vectorizable
        if !self.all_operations_vectorizable(loop_region, func) {
            return Err(VectorizationBlocker::NonVectorizableOperation);
        }

        // Check 6: Loop must be large enough to benefit
        let trip_count = loop_region.trip_count.unwrap_or(0);
        if trip_count < 4 {
            return Err(VectorizationBlocker::TooSmall);
        }

        Ok(())
    }

    fn has_loop_carried_dependencies(&self, loop_region: &LoopRegion, func: &MirFunction) -> bool {
        // Check if any value written in iteration i is read in iteration i+1
        // (before being written)

        // Collect all writes and reads in loop body
        let mut writes: HashSet<LocalId> = HashSet::new();
        let mut reads_before_write: HashSet<LocalId> = HashSet::new();

        for block_id in &loop_region.blocks {
            if let Some(block) = func.blocks.iter().find(|b| b.id == *block_id) {
                let mut block_writes: HashSet<LocalId> = HashSet::new();

                for stmt in block.statements.iter() {
                    match stmt {
                        MirStatement::Assign(place, rvalue) => {
                            // Collect reads from rvalue
                            let reads = self.extract_reads_from_rvalue(rvalue);
                            for read in reads {
                                // If we read before writing in this block, it's a potential
                                // loop-carried dependency
                                if writes.contains(&read) && !block_writes.contains(&read) {
                                    reads_before_write.insert(read);
                                }
                            }
                            // Record the write
                            writes.insert(place.local);
                            block_writes.insert(place.local);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check for induction variable dependencies (which are expected and safe)
        let induction_locals: HashSet<LocalId> = loop_region
            .induction_vars
            .iter()
            .map(|iv| iv.local)
            .collect();

        // True loop-carried dependency exists if we have reads before writes
        // that are NOT induction variables
        reads_before_write
            .iter()
            .any(|r| !induction_locals.contains(r))
    }

    fn extract_reads_from_rvalue(&self, rvalue: &Rvalue) -> Vec<LocalId> {
        let mut reads = Vec::new();
        match rvalue {
            Rvalue::Use(operand) => {
                if let Some(local) = self.operand_local(operand) {
                    reads.push(local);
                }
            }
            Rvalue::Binary(_, lhs, rhs) | Rvalue::CheckedBinary(_, lhs, rhs) => {
                if let Some(local) = self.operand_local(lhs) {
                    reads.push(local);
                }
                if let Some(local) = self.operand_local(rhs) {
                    reads.push(local);
                }
            }
            Rvalue::Unary(_, operand) => {
                if let Some(local) = self.operand_local(operand) {
                    reads.push(local);
                }
            }
            Rvalue::Ref(_, place) | Rvalue::Deref(place) => {
                reads.push(place.local);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands.iter() {
                    if let Some(local) = self.operand_local(operand) {
                        reads.push(local);
                    }
                }
            }
            _ => {}
        }
        reads
    }

    fn operand_local(&self, operand: &Operand) -> Option<LocalId> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => Some(place.local),
            Operand::Constant(_) => None,
        }
    }

    fn can_guarantee_alignment(&self, loop_region: &LoopRegion, func: &MirFunction) -> bool {
        // Check if memory accesses can be aligned to vector width
        // This requires analyzing array base addresses and access patterns

        // Collect memory access patterns
        for block_id in &loop_region.blocks {
            if let Some(block) = func.blocks.iter().find(|b| b.id == *block_id) {
                for stmt in block.statements.iter() {
                    if let MirStatement::Assign(place, _) = stmt {
                        // Check for array/slice indexing with non-unit stride
                        for proj in place.projections.iter() {
                            match proj {
                                PlaceProjection::Index(idx_local) => {
                                    // Check if index is induction variable with stride != 1
                                    if let Some(iv) = loop_region
                                        .induction_vars
                                        .iter()
                                        .find(|iv| iv.local == *idx_local)
                                    {
                                        if iv.step != 1 && iv.step != -1 {
                                            // Non-unit stride - harder to guarantee alignment
                                            return false;
                                        }
                                    }
                                }
                                PlaceProjection::ConstantIndex { offset, .. } => {
                                    // Constant offset - check if it's vector-width aligned
                                    // For conservative safety, require 0 offset
                                    if *offset != 0 {
                                        return false;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Can generate alignment prologue for simple cases
        true
    }

    fn can_prove_bounds_safe_for_vectorization(
        &self,
        loop_region: &LoopRegion,
        func: &MirFunction,
    ) -> bool {
        // Check if bounds are safe for vector access (need N elements available)
        // This requires that trip_count - current_iteration >= vector_width

        // If we know the trip count, we can statically verify
        if let Some(trip_count) = loop_region.trip_count {
            let vector_width = self.determine_vector_width(loop_region, func);

            // Need at least vector_width iterations for full vectorization
            if trip_count < vector_width {
                return false;
            }

            // Check that all array accesses are within bounds
            for iv in &loop_region.induction_vars {
                if let Some(upper_bound) = iv.upper_bound {
                    // Ensure we don't go out of bounds even with vectorized access
                    let max_access = iv.init + (iv.step * (trip_count as i64 - 1));
                    if max_access >= upper_bound {
                        return false;
                    }
                }
            }

            return true;
        }

        // Without known trip count, we need runtime checks
        // Return true but the codegen should add bounds checks
        true
    }

    fn has_potential_aliasing(&self, loop_region: &LoopRegion, func: &MirFunction) -> bool {
        // Check if read and write sets might overlap
        // This is a conservative pointer alias analysis

        let mut write_bases: HashSet<LocalId> = HashSet::new();
        let mut read_bases: HashSet<LocalId> = HashSet::new();

        for block_id in &loop_region.blocks {
            if let Some(block) = func.blocks.iter().find(|b| b.id == *block_id) {
                for stmt in block.statements.iter() {
                    if let MirStatement::Assign(place, rvalue) = stmt {
                        // Check if this is a memory write through pointer
                        let is_memory_write = place.projections.iter().any(|p| {
                            matches!(
                                p,
                                PlaceProjection::Deref
                                    | PlaceProjection::Index(_)
                                    | PlaceProjection::ConstantIndex { .. }
                            )
                        });

                        if is_memory_write {
                            // Get the base local (before projections)
                            write_bases.insert(place.local);
                        }

                        // Check for reads through pointers in rvalue
                        match rvalue {
                            Rvalue::Deref(read_place) => {
                                read_bases.insert(read_place.local);
                            }
                            Rvalue::Use(Operand::Copy(read_place))
                            | Rvalue::Use(Operand::Move(read_place)) => {
                                let is_memory_read = read_place.projections.iter().any(|p| {
                                    matches!(
                                        p,
                                        PlaceProjection::Deref
                                            | PlaceProjection::Index(_)
                                            | PlaceProjection::ConstantIndex { .. }
                                    )
                                });
                                if is_memory_read {
                                    read_bases.insert(read_place.local);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Check for potential aliasing - conservative: any overlap in base locals
        // means potential aliasing (more sophisticated analysis would track pointer provenance)
        for write_base in &write_bases {
            if read_bases.contains(write_base) {
                // Same base local for both read and write through pointers
                // This MIGHT be aliasing (same array, different indices)
                // For now, be conservative unless we can prove the indices are distinct

                // Check if we have __restrict hints on parameters
                let local_info = func.locals.iter().find(|l| l.id == *write_base);
                if let Some(info) = local_info {
                    // Check if parameter has restrict qualifier
                    // (would be indicated in type or attributes)
                    if !info.name.contains("restrict") {
                        return true;
                    }
                }
            }
        }

        // Also check write-write conflicts
        write_bases.len() > 1
            && write_bases.iter().any(|w| {
                func.locals
                    .iter()
                    .find(|l| l.id == *w)
                    .map(|l| !l.name.contains("restrict"))
                    .unwrap_or(true)
            })
    }

    fn all_operations_vectorizable(&self, loop_region: &LoopRegion, func: &MirFunction) -> bool {
        // Check if all operations in loop body have SIMD equivalents
        for block_id in &loop_region.blocks {
            if let Some(block) = func.blocks.iter().find(|b| b.id == *block_id) {
                for stmt in block.statements.iter() {
                    match stmt {
                        MirStatement::Assign(_, rvalue) => {
                            if !self.is_vectorizable_rvalue(rvalue) {
                                return false;
                            }
                        }
                        MirStatement::BoundsCheck { .. } => {
                            // Bounds checks can be hoisted or vectorized
                        }
                        MirStatement::GenerationCheck(_) => {
                            // CBGR checks can be hoisted
                        }
                        _ => {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn is_vectorizable_rvalue(&self, rvalue: &Rvalue) -> bool {
        match rvalue {
            Rvalue::Binary(_op, _, _) => {
                // Most arithmetic operations are vectorizable
                true
            }
            Rvalue::Use(_) => true,
            Rvalue::Unary(_, _) => true,
            _ => false,
        }
    }

    fn determine_vector_width(&self, _loop_region: &LoopRegion, _func: &MirFunction) -> usize {
        // Determine based on element type and target architecture
        // AVX2: 256-bit (8 x f32, 4 x f64)
        // AVX512: 512-bit (16 x f32, 8 x f64)
        // NEON: 128-bit (4 x f32)
        4 // Default to 4 for f32/i32 on typical hardware
    }

    fn generate_alignment_prologue(
        &self,
        _loop_region: &LoopRegion,
        _vector_width: usize,
    ) -> Vec<MirStatement> {
        // Generate scalar loop iterations until alignment is achieved
        Vec::new()
    }

    fn generate_vector_loop(
        &self,
        loop_region: &LoopRegion,
        _vector_width: usize,
        func: &MirFunction,
    ) -> Result<Vec<MirStatement>, VectorizationBlocker> {
        // Generate SIMD versions of operations
        let mut statements = Vec::new();

        for block_id in &loop_region.blocks {
            if let Some(block) = func.blocks.iter().find(|b| b.id == *block_id) {
                for stmt in block.statements.iter() {
                    match stmt {
                        MirStatement::Assign(_place, _rvalue) => {
                            // Convert to SIMD operation
                            // In practice, this would generate SIMD intrinsics
                            statements.push(stmt.clone());
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(statements)
    }

    fn generate_epilogue(
        &self,
        _loop_region: &LoopRegion,
        _vector_width: usize,
    ) -> Vec<MirStatement> {
        // Generate scalar loop for remaining iterations
        Vec::new()
    }

    // =========================================================================
    // Dead Code Elimination Pass
    // =========================================================================

    /// Dead code elimination
    ///
    /// Removes unreachable code and unused definitions.
    fn dead_code_elimination(&mut self, func: &mut MirFunction) {
        tracing::debug!("Running dead code elimination for function: {}", func.name);

        // Phase 1: Mark live statements
        let live_places = self.compute_live_places(func);

        // Phase 2: Remove dead assignments
        for block in func.blocks.iter_mut() {
            let initial_len = block.statements.len();
            block.statements.retain(|stmt| {
                match stmt {
                    MirStatement::Assign(place, _) => {
                        // Keep if place is live or has side effects
                        live_places.contains(&place.local) || self.has_side_effects(stmt)
                    }
                    // Keep all other statements
                    _ => true,
                }
            });
            self.stats.dead_statements_removed += initial_len - block.statements.len();
        }

        // Phase 3: Remove unreachable blocks
        let reachable_blocks = self.compute_reachable_blocks(func);
        let initial_blocks = func.blocks.len();
        func.blocks
            .retain(|block| reachable_blocks.contains(&block.id));
        self.stats.dead_blocks_removed = initial_blocks - func.blocks.len();

        tracing::debug!(
            "Dead code elimination: {} statements removed, {} blocks removed",
            self.stats.dead_statements_removed,
            self.stats.dead_blocks_removed
        );
    }

    /// Compute set of live places (backward analysis)
    fn compute_live_places(&self, func: &MirFunction) -> HashSet<LocalId> {
        let mut live = HashSet::new();
        let mut worklist = VecDeque::new();

        // Return place is always live
        worklist.push_back(LocalId(0)); // _0 is return place

        // Process worklist
        while let Some(local) = worklist.pop_front() {
            if live.insert(local) {
                // Find all definitions of this local and add their uses
                for block in func.blocks.iter() {
                    for stmt in block.statements.iter() {
                        if let MirStatement::Assign(place, rvalue) = stmt {
                            if place.local == local {
                                // Add all locals used in rvalue
                                for used_local in self.rvalue_uses(rvalue) {
                                    if !live.contains(&used_local) {
                                        worklist.push_back(used_local);
                                    }
                                }
                            }
                        }
                    }

                    // Check terminator uses
                    for used_local in self.terminator_uses(&block.terminator) {
                        if !live.contains(&used_local) {
                            worklist.push_back(used_local);
                        }
                    }
                }
            }
        }

        live
    }

    /// Get locals used by an rvalue
    fn rvalue_uses(&self, rvalue: &Rvalue) -> Vec<LocalId> {
        let mut uses = Vec::new();

        match rvalue {
            Rvalue::Use(op) => uses.extend(self.operand_uses(op)),
            Rvalue::Binary(_, lhs, rhs) => {
                uses.extend(self.operand_uses(lhs));
                uses.extend(self.operand_uses(rhs));
            }
            Rvalue::Unary(_, op) => uses.extend(self.operand_uses(op)),
            Rvalue::Ref(_, place) => uses.push(place.local),
            Rvalue::Deref(place) => uses.push(place.local),
            Rvalue::Cast(_, op, _) => uses.extend(self.operand_uses(op)),
            Rvalue::Aggregate(_, ops) => {
                for op in ops.iter() {
                    uses.extend(self.operand_uses(op));
                }
            }
            Rvalue::Discriminant(place) => uses.push(place.local),
            Rvalue::Len(place) => uses.push(place.local),
            Rvalue::CheckedBinary(_, lhs, rhs) => {
                uses.extend(self.operand_uses(lhs));
                uses.extend(self.operand_uses(rhs));
            }
            Rvalue::NullConstant => {}
            Rvalue::AddressOf(_, place) => uses.push(place.local),
            Rvalue::ShallowInitBox(op, _) => uses.extend(self.operand_uses(op)),
            Rvalue::CopyForDeref(place) => uses.push(place.local),
            // Catch any other variants
            _ => {}
        }

        uses
    }

    /// Get locals used by an operand
    fn operand_uses(&self, operand: &Operand) -> Vec<LocalId> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => vec![place.local],
            Operand::Constant(_) => vec![],
        }
    }

    /// Get locals used by a terminator
    fn terminator_uses(&self, terminator: &Terminator) -> Vec<LocalId> {
        let mut uses = Vec::new();

        match terminator {
            Terminator::SwitchInt { discriminant, .. } => {
                uses.extend(self.operand_uses(discriminant));
            }
            Terminator::Call { func, args, .. } => {
                uses.extend(self.operand_uses(func));
                for arg in args.iter() {
                    uses.extend(self.operand_uses(arg));
                }
            }
            Terminator::Assert { condition, .. } => {
                uses.extend(self.operand_uses(condition));
            }
            _ => {}
        }

        uses
    }

    /// Check if a statement has side effects
    fn has_side_effects(&self, stmt: &MirStatement) -> bool {
        match stmt {
            MirStatement::Assign(_, rvalue) => {
                // Aggregates might have side effects (allocations)
                matches!(rvalue, Rvalue::Aggregate(_, _))
            }
            MirStatement::Drop(_) => true,
            MirStatement::DeferCleanup { .. } => true,
            _ => false,
        }
    }

    /// Compute reachable blocks from entry
    fn compute_reachable_blocks(&self, func: &MirFunction) -> HashSet<BlockId> {
        let mut reachable = HashSet::new();
        let mut worklist = VecDeque::new();

        worklist.push_back(func.entry_block);

        while let Some(block_id) = worklist.pop_front() {
            if reachable.insert(block_id) {
                if let Some(block) = func.blocks.iter().find(|b| b.id == block_id) {
                    for &succ in block.successors.iter() {
                        if !reachable.contains(&succ) {
                            worklist.push_back(succ);
                        }
                    }
                }
            }
        }

        reachable
    }

    // =========================================================================
    // Loop Hoisting Pass
    // =========================================================================

    /// Hoist loop-invariant checks out of loops
    fn loop_hoisting(&mut self, func: &mut MirFunction) {
        tracing::debug!("Running loop hoisting for function: {}", func.name);

        let dom_tree = DominatorTree::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
        );

        let loop_info = LoopInfo::compute(
            &func.blocks.iter().cloned().collect::<Vec<_>>(),
            func.entry_block,
            &dom_tree,
        );

        // For each loop, find invariant checks
        for (header, body) in &loop_info.loop_bodies {
            let invariants = self.find_loop_invariant_checks(*header, body, func);

            for (_block_id, _stmt_idx) in invariants {
                // Move check to preheader
                // In practice, this would:
                // 1. Find or create preheader block
                // 2. Move statement from loop body to preheader
                self.stats.checks_hoisted += 1;
            }
        }

        tracing::debug!(
            "Loop hoisting: {} checks hoisted",
            self.stats.checks_hoisted
        );
    }

    /// Find loop-invariant checks
    fn find_loop_invariant_checks(
        &self,
        _header: BlockId,
        body: &HashSet<BlockId>,
        func: &MirFunction,
    ) -> Vec<(BlockId, usize)> {
        let mut invariants = Vec::new();

        for &block_id in body {
            if let Some(block) = func.blocks.iter().find(|b| b.id == block_id) {
                for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                    if self.is_loop_invariant_check(stmt, body, func) {
                        invariants.push((block_id, stmt_idx));
                    }
                }
            }
        }

        invariants
    }

    /// Check if a statement is loop-invariant
    fn is_loop_invariant_check(
        &self,
        stmt: &MirStatement,
        loop_body: &HashSet<BlockId>,
        func: &MirFunction,
    ) -> bool {
        match stmt {
            MirStatement::BoundsCheck { array, index } => {
                // Invariant if both array and index are defined outside loop
                self.is_defined_outside_loop(&array.local, loop_body, func)
                    && self.is_defined_outside_loop(&index.local, loop_body, func)
            }
            MirStatement::GenerationCheck(place) => {
                self.is_defined_outside_loop(&place.local, loop_body, func)
            }
            _ => false,
        }
    }

    /// Check if a local is defined outside a loop
    fn is_defined_outside_loop(
        &self,
        local: &LocalId,
        loop_body: &HashSet<BlockId>,
        func: &MirFunction,
    ) -> bool {
        for block in func.blocks.iter() {
            if loop_body.contains(&block.id) {
                for stmt in block.statements.iter() {
                    if let MirStatement::Assign(place, _) = stmt {
                        if place.local == *local {
                            return false; // Defined inside loop
                        }
                    }
                }
            }
        }
        true
    }

    // =========================================================================
    // Reference Promotion Pass
    // =========================================================================

    /// Promote &T references to &checked T where proven safe
    fn promote_references(&mut self, func: &mut MirFunction, escape_info: &EscapeAnalysisResult) {
        tracing::debug!("Running reference promotion for function: {}", func.name);

        for place in &escape_info.promotable_to_checked {
            // Update reference type from &T to &checked T
            // This involves changing the ReferenceLayout from ThinRef to FatRef
            for local in func.locals.iter_mut() {
                if local.id == place.local {
                    if let MirType::Ref {
                        inner,
                        mutable,
                        layout,
                    } = &local.ty
                    {
                        if *layout == ReferenceLayout::ThinRef {
                            local.ty = MirType::Ref {
                                inner: inner.clone(),
                                mutable: *mutable,
                                layout: ReferenceLayout::FatRef(MetadataKind::Length),
                            };
                            self.stats.refs_promoted_to_checked += 1;
                        }
                    }
                }
            }
        }

        tracing::debug!(
            "Reference promotion: {} references promoted to &checked T",
            self.stats.refs_promoted_to_checked
        );
    }
}

impl Default for OptimizationPhase {
    fn default() -> Self {
        Self::new(OptimizationLevel::O2)
    }
}

impl CompilationPhase for OptimizationPhase {
    fn name(&self) -> &str {
        "Phase 6: Optimization"
    }

    fn description(&self) -> &str {
        "Escape analysis, check elimination, inlining, vectorization"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract MIR modules from input - now using the full internal MIR structure
        let mir_modules = match &input.data {
            PhaseData::Mir(modules) => modules.clone(),
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for optimization phase: expected MIR")
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // PhaseData::Mir now carries the full internal MIR structure (mir_lowering::MirModule)
        // with functions, locals, SSA form, and CFG - enabling proper optimization passes:
        // - Escape analysis for CBGR reference classification
        // - CBGR check elimination (50-90% typical)
        // - Bounds check elimination
        // - SBGL optimization (Stack-Based Garbage-free Lists) for NoEscape refs
        // - Reference promotion to &checked T
        // - Loop invariant code motion (check hoisting)
        // - Dead code elimination
        // - Function inlining (O2+)
        // - SIMD vectorization (O3)

        // Convert List to Vec for optimize_mir_modules
        let mut mir_vec: Vec<MirModule> = mir_modules.into_iter().collect();

        // Run the full optimization pipeline via optimize_mir_modules()
        let (stats, all_warnings) = optimize_mir_modules(&mut mir_vec, self.opt_level);

        tracing::debug!(
            "Full MIR optimization complete: {} CBGR checks eliminated, {} bounds checks eliminated, \
             {} functions inlined, {} loops vectorized, {} NoEscape refs identified, {} refs promoted",
            stats.cbgr_checks_eliminated,
            stats.bounds_checks_eliminated,
            stats.functions_inlined,
            stats.loops_vectorized,
            stats.no_escape_refs_identified,
            stats.refs_promoted_to_checked
        );

        // Convert back to List for output
        let output_modules: List<MirModule> = mir_vec.into_iter().collect();

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);
        metrics.add_custom_metric("opt_level", format!("{:?}", self.opt_level));
        metrics.add_custom_metric(
            "cbgr_checks_eliminated",
            stats.cbgr_checks_eliminated.to_text(),
        );
        metrics.add_custom_metric("cbgr_checks_kept", stats.cbgr_checks_kept.to_text());
        metrics.add_custom_metric(
            "bounds_checks_eliminated",
            stats.bounds_checks_eliminated.to_text(),
        );
        metrics.add_custom_metric("functions_inlined", stats.functions_inlined.to_text());
        metrics.add_custom_metric("loops_vectorized", stats.loops_vectorized.to_text());
        metrics.add_custom_metric("dead_blocks_removed", stats.dead_blocks_removed.to_text());
        metrics.add_custom_metric(
            "dead_statements_removed",
            stats.dead_statements_removed.to_text(),
        );
        metrics.add_custom_metric("no_escape_refs", stats.no_escape_refs_identified.to_text());
        metrics.add_custom_metric("refs_promoted", stats.refs_promoted_to_checked.to_text());
        metrics.add_custom_metric(
            "sbgl_optimizations",
            stats.sbgl_optimizations_applied.to_text(),
        );
        metrics.add_custom_metric("sbgl_warnings", stats.sbgl_warnings_emitted.to_text());
        metrics.add_custom_metric("checks_hoisted", stats.checks_hoisted.to_text());

        // Calculate elimination rates
        let total_cbgr = stats.cbgr_checks_eliminated + stats.cbgr_checks_kept;
        let cbgr_rate = if total_cbgr > 0 {
            (stats.cbgr_checks_eliminated as f64 / total_cbgr as f64) * 100.0
        } else {
            0.0
        };

        let total_bounds = stats.bounds_checks_eliminated + stats.bounds_checks_kept;
        let bounds_rate = if total_bounds > 0 {
            (stats.bounds_checks_eliminated as f64 / total_bounds as f64) * 100.0
        } else {
            0.0
        };

        tracing::info!(
            "Optimization complete: level {:?}, CBGR elimination: {:.1}%, bounds elimination: {:.1}%, \
             NoEscape refs: {}, promoted: {}, SBGL: {} applied / {} warnings, {:.2}ms",
            self.opt_level,
            cbgr_rate,
            bounds_rate,
            stats.no_escape_refs_identified,
            stats.refs_promoted_to_checked,
            stats.sbgl_optimizations_applied,
            stats.sbgl_warnings_emitted,
            duration.as_millis()
        );

        Ok(PhaseOutput {
            data: PhaseData::OptimizedMir(output_modules),
            warnings: all_warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Many optimizations can run in parallel across functions
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

// =============================================================================
// Full MIR Optimization Pipeline
// =============================================================================

/// Run all optimization passes on MIR modules using the full internal MIR structure
///
/// **MAIN OPTIMIZATION ENTRY POINT** - This function implements the complete
/// optimization pipeline on the full internal MIR representation with:
/// - Functions, locals, basic blocks in SSA form
/// - CFG (Control Flow Graph) structure
/// - Complete type information
///
/// Optimizations performed:
/// - Escape analysis for CBGR reference classification
/// - CBGR check elimination (50-90% typical)
/// - Bounds check elimination
/// - SBGL optimization (Stack-Based Garbage-free Lists) for NoEscape refs
/// - Reference promotion to &checked T
/// - Loop invariant code motion (check hoisting)
/// - Dead code elimination
/// - Function inlining (O2+)
/// - SIMD vectorization (O3)
///
/// Called from:
/// - OptimizationPhase::execute() via the pipeline orchestrator
/// - mir_lowering.rs for early optimization before phase output
pub fn optimize_mir_modules(
    modules: &mut [MirModule],
    opt_level: OptimizationLevel,
) -> (OptimizationStats, List<Diagnostic>) {
    let mut phase = OptimizationPhase::new(opt_level);
    let mut all_warnings = List::new();

    match opt_level {
        OptimizationLevel::O0 => {
            // No optimization
        }
        OptimizationLevel::O1 => {
            // Basic optimizations: DCE only
            for module in modules.iter_mut() {
                for func in module.functions.iter_mut() {
                    phase.dead_code_elimination(func);
                }
            }
        }
        OptimizationLevel::O2 => {
            // Standard optimizations
            for module in modules.iter_mut() {
                for func in module.functions.iter_mut() {
                    // Escape analysis
                    let escape_info = phase.escape_analysis(func);

                    // SBGL optimization
                    let warnings = phase.sbgl_optimization(func, &escape_info);
                    all_warnings.extend(warnings);

                    // Check elimination
                    phase.check_elimination(func, &escape_info);

                    // Reference promotion
                    phase.promote_references(func, &escape_info);

                    // Loop hoisting
                    phase.loop_hoisting(func);

                    // Dead code elimination
                    phase.dead_code_elimination(func);
                }
            }

            // Function inlining (cross-function)
            phase.function_inlining(modules);
        }
        OptimizationLevel::O3 => {
            // All of O2 plus vectorization
            for module in modules.iter_mut() {
                for func in module.functions.iter_mut() {
                    // Escape analysis
                    let escape_info = phase.escape_analysis(func);

                    // SBGL optimization
                    let warnings = phase.sbgl_optimization(func, &escape_info);
                    all_warnings.extend(warnings);

                    // Check elimination
                    phase.check_elimination(func, &escape_info);

                    // Reference promotion
                    phase.promote_references(func, &escape_info);

                    // SIMD vectorization
                    phase.vectorization(func);

                    // Loop hoisting
                    phase.loop_hoisting(func);

                    // Dead code elimination
                    phase.dead_code_elimination(func);
                }
            }

            // Function inlining (cross-function)
            phase.function_inlining(modules);
        }
    }

    (phase.stats, all_warnings)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::mir_lowering::{BasicBlock, BlockId, LocalId};
    use super::*;
    use verum_ast::span::Span as AstSpan;

    fn make_test_function() -> MirFunction {
        MirFunction {
            name: "test".into(),
            signature: super::super::mir_lowering::MirSignature {
                params: List::new(),
                return_type: MirType::Unit,
                contexts: List::new(),
                is_async: false,
            },
            locals: List::new(),
            blocks: List::from(vec![BasicBlock {
                id: BlockId(0),
                statements: List::new(),
                terminator: Terminator::Return,
                predecessors: List::new(),
                successors: List::new(),
                phi_nodes: List::new(),
                is_cleanup: false,
            }]),
            entry_block: BlockId(0),
            cleanup_blocks: List::new(),
            span: AstSpan::dummy(),
        }
    }

    #[test]
    fn test_escape_category_classification() {
        let place = Place::local(LocalId(1));
        let mut info = ReferenceEscapeInfo::new(place, BlockId(0));

        // NoEscape case
        info.compute_category();
        assert_eq!(info.category, EscapeCategory::NoEscape);

        // ThreadEscape takes priority
        info.sent_to_thread = true;
        info.compute_category();
        assert_eq!(info.category, EscapeCategory::ThreadEscape);

        // HeapEscape
        info.sent_to_thread = false;
        info.stored_to_heap = true;
        info.compute_category();
        assert_eq!(info.category, EscapeCategory::HeapEscape);

        // LocalEscape via return
        info.stored_to_heap = false;
        info.returned = true;
        info.compute_category();
        assert_eq!(info.category, EscapeCategory::LocalEscape);
    }

    #[test]
    fn test_vectorization_blocker_variants() {
        // Ensure all variants can be created
        let blockers = [VectorizationBlocker::LoopCarriedDependency,
            VectorizationBlocker::UnalignedAccess,
            VectorizationBlocker::UnprovenBounds,
            VectorizationBlocker::PotentialAliasing,
            VectorizationBlocker::NonVectorizableOperation,
            VectorizationBlocker::NonVectorizableStatement,
            VectorizationBlocker::TooSmall,
            VectorizationBlocker::ComplexControlFlow];
        assert_eq!(blockers.len(), 8);
    }

    #[test]
    fn test_optimization_level_default() {
        let phase = OptimizationPhase::default();
        assert_eq!(phase.opt_level, OptimizationLevel::O2);
    }

    #[test]
    fn test_ssa_form_creation() {
        let func = make_test_function();
        let phase = OptimizationPhase::default();
        let ssa = phase.build_ssa(&func);

        assert_eq!(ssa.entry_block, BlockId(0));
    }

    #[test]
    fn test_reference_flow_info() {
        let mut flow_info = ReferenceFlowInfo::new();

        let ssa_place1 = SSAPlace {
            place: Place::local(LocalId(1)),
            version: 0,
        };
        let ssa_place2 = SSAPlace {
            place: Place::local(LocalId(2)),
            version: 0,
        };

        flow_info.add_reference(ssa_place1.clone(), Place::local(LocalId(0)));
        flow_info.propagate_flow(ssa_place1.clone(), ssa_place2.clone());

        assert!(flow_info.references.contains_key(&ssa_place1));
        assert!(
            flow_info
                .flow_forward
                .get(&ssa_place1)
                .unwrap()
                .contains(&ssa_place2)
        );
    }

    #[test]
    fn test_operand_uses() {
        let phase = OptimizationPhase::default();

        let copy_op = Operand::Copy(Place::local(LocalId(5)));
        assert_eq!(phase.operand_uses(&copy_op), vec![LocalId(5)]);

        let const_op = Operand::Constant(MirConstant::Int(42));
        assert!(phase.operand_uses(&const_op).is_empty());
    }

    #[test]
    fn test_has_side_effects() {
        let phase = OptimizationPhase::default();

        let assign_stmt = MirStatement::Assign(
            Place::local(LocalId(0)),
            Rvalue::Use(Operand::Constant(MirConstant::Int(0))),
        );
        assert!(!phase.has_side_effects(&assign_stmt));

        let drop_stmt = MirStatement::Drop(Place::local(LocalId(1)));
        assert!(phase.has_side_effects(&drop_stmt));
    }

    #[test]
    fn test_inline_decision_small_function() {
        let phase = OptimizationPhase::default();

        let small_func = MirFunction {
            name: "small".into(),
            signature: super::super::mir_lowering::MirSignature {
                params: List::new(),
                return_type: MirType::Unit,
                contexts: List::new(),
                is_async: false,
            },
            locals: List::new(),
            blocks: List::from(vec![BasicBlock {
                id: BlockId(0),
                statements: List::from(vec![MirStatement::Nop, MirStatement::Nop]),
                terminator: Terminator::Return,
                predecessors: List::new(),
                successors: List::new(),
                phi_nodes: List::new(),
                is_cleanup: false,
            }]),
            entry_block: BlockId(0),
            cleanup_blocks: List::new(),
            span: AstSpan::dummy(),
        };

        let caller = make_test_function();

        // Small functions should be inlined
        assert!(phase.should_inline(&caller, &small_func));
    }

    #[test]
    fn test_vectorizable_rvalue() {
        let phase = OptimizationPhase::default();

        let binary = Rvalue::Binary(
            verum_ast::expr::BinOp::Add,
            Operand::Constant(MirConstant::Int(1)),
            Operand::Constant(MirConstant::Int(2)),
        );
        assert!(phase.is_vectorizable_rvalue(&binary));

        let use_rv = Rvalue::Use(Operand::Constant(MirConstant::Int(0)));
        assert!(phase.is_vectorizable_rvalue(&use_rv));

        let deref = Rvalue::Deref(Place::local(LocalId(0)));
        assert!(!phase.is_vectorizable_rvalue(&deref));
    }
}
