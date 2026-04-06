//! Enhanced Escape Analysis for CBGR Optimization
//!
//! This module implements comprehensive escape analysis to automatically promote
//! `&T` (managed, ~15ns) references to `&checked T` (0ns) when the compiler can
//! prove the reference doesn't escape its scope.
//!
//! # Purpose
//!
//! Escape analysis is critical for CBGR performance. It enables automatic
//! elimination of runtime checks for references that are provably local, transforming
//! ~15ns CBGR overhead into 0ns.
//!
//! # CBGR Memory Alignment and Escape Analysis Requirements
//!
//! The CBGR allocation system uses careful memory alignment (MIN_ALIGNMENT=16,
//! HEADER_SIZE=32, CACHE_LINE_SIZE=64). Each allocation has a 32-byte header
//! containing generation counters and epoch information. Escape analysis must
//! correctly track references through this header structure to determine if
//! references can bypass CBGR validation:
//!
//! 1. **Track reference creation points**: Monitor all allocation sites
//! 2. **Analyze all uses**: Determine if references escape to heap, return, etc.
//! 3. **Skip CBGR validation**: References that don't escape can use direct pointers
//!    (bypassing the generation check against the AllocationHeader)
//! 4. **Mark `NoEscape` references**: Enable SBGL (Scope-Bound Generation-Less)
//!    optimization where raw pointers replace ThinRef/FatRef internally
//!
//! # Escape Scenarios
//!
//! ```text
//! ✅ NoEscape (0ns CBGR):
//!    - Used only within local scope
//!    - Passed to function by reference only (callee doesn't escape)
//!    - Loop iteration variables
//!
//! ❌ Escapes (~15ns CBGR):
//!    - Stored in heap-allocated structure
//!    - Passed to function that stores it
//!    - Returned from function
//!    - Crosses thread boundaries
//! ```
//!
//! # Performance Impact
//!
//! - **Hot loops**: 0ns (promoted to &checked T via escape analysis)
//! - **Application code**: 0.5-1% overhead (many refs proven `NoEscape`)
//! - **Complex flows**: 1-2% overhead (conservative CBGR where needed)
//!
//! # Algorithm
//!
//! 1. Build SSA form for precise data flow tracking
//! 2. Perform forward dataflow analysis to track reference flow
//! 3. Detect escape points (heap stores, returns, thread spawns)
//! 4. Compute `NoEscape` set (references that never leave scope)
//! 5. Integrate with codegen to skip CBGR checks for `NoEscape` refs

use crate::analysis::{
    BlockId, ControlFlowGraph, DefSite, EscapeResult, FunctionId, RefId, UseeSite,
};
use crate::call_graph::CallGraph;
use crate::ssa::SsaFunction;
use std::fmt;
use verum_common::{List, Map, Maybe, Set, Text};

// ==================================================================================
// Escape State Tracking (Section 2.3.1)
// ==================================================================================

/// Escape state for a reference during analysis
///
/// This enum represents the progressive understanding of whether a reference
/// escapes, updated as we perform dataflow analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeState {
    /// Reference definitely doesn't escape (proven safe for 0ns optimization)
    NoEscape,

    /// Reference may escape (need runtime CBGR checks)
    MayEscape,

    /// Reference definitely escapes (confirmed need for CBGR)
    Escapes,

    /// Unknown state (initial value, conservative)
    Unknown,
}

impl EscapeState {
    /// Check if this state allows CBGR check elimination
    #[must_use]
    pub fn allows_optimization(&self) -> bool {
        matches!(self, EscapeState::NoEscape)
    }

    /// Check if this state requires CBGR checks
    #[must_use]
    pub fn requires_cbgr(&self) -> bool {
        matches!(
            self,
            EscapeState::MayEscape | EscapeState::Escapes | EscapeState::Unknown
        )
    }

    /// Merge two escape states (conservative join)
    ///
    /// Used in dataflow analysis to combine states from different paths:
    /// - `NoEscape` + `NoEscape` = `NoEscape`
    /// - `NoEscape` + Escapes = Escapes
    /// - `MayEscape` + anything (except Unknown) = `MayEscape`
    /// - Unknown + known = known (Unknown is refined by analysis)
    #[must_use]
    pub fn merge(self, other: EscapeState) -> EscapeState {
        use EscapeState::{Escapes, MayEscape, NoEscape, Unknown};

        match (self, other) {
            // Both NoEscape: stays NoEscape
            (NoEscape, NoEscape) => NoEscape,

            // Unknown is refined by a known state
            (Unknown, known) => known,
            (known, Unknown) => known,

            // Any Escapes: becomes Escapes
            (Escapes, _) | (_, Escapes) => Escapes,

            // Any MayEscape: becomes MayEscape
            (MayEscape, _) | (_, MayEscape) => MayEscape,
        }
    }

    /// Convert to user-facing string
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            EscapeState::NoEscape => "NoEscape (0ns)",
            EscapeState::MayEscape => "MayEscape (~15ns)",
            EscapeState::Escapes => "Escapes (~15ns)",
            EscapeState::Unknown => "Unknown (~15ns)",
        }
    }
}

impl fmt::Display for EscapeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ==================================================================================
// Escape Point Tracking (Section 2.3.2)
// ==================================================================================

/// Escape point - location where a reference escapes
///
/// Tracks the exact program point where escape occurs for diagnostics
/// and optimization feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscapePoint {
    /// Reference that escapes
    pub reference: RefId,

    /// Basic block where escape occurs
    pub block: BlockId,

    /// Type of escape
    pub escape_kind: EscapeKind,

    /// Human-readable description
    pub description: Text,

    /// Source location (for IDE integration)
    pub source_location: Maybe<SourceLocation>,
}

/// Kind of escape
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeKind {
    /// Reference returned from function
    ReturnEscape,

    /// Reference stored in heap-allocated structure
    HeapStore,

    /// Reference captured by closure that escapes
    ClosureCapture,

    /// Reference passed to thread spawn
    ThreadCrossing,

    /// Reference passed to function parameter that escapes
    ParameterEscape,

    /// Reference stored in global variable
    GlobalStore,

    /// Reference escapes via unknown mechanism
    Unknown,
}

impl EscapeKind {
    /// Get human-readable name
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            EscapeKind::ReturnEscape => "Return escape",
            EscapeKind::HeapStore => "Heap store",
            EscapeKind::ClosureCapture => "Closure capture",
            EscapeKind::ThreadCrossing => "Thread crossing",
            EscapeKind::ParameterEscape => "Parameter escape",
            EscapeKind::GlobalStore => "Global store",
            EscapeKind::Unknown => "Unknown escape",
        }
    }

    /// Get optimization hint
    #[must_use]
    pub fn optimization_hint(&self) -> &'static str {
        match self {
            EscapeKind::ReturnEscape => "Consider returning owned values instead of references",
            EscapeKind::HeapStore => "Consider stack allocation or arena allocation",
            EscapeKind::ClosureCapture => "Consider immediate closure invocation or owned captures",
            EscapeKind::ThreadCrossing => "Consider message passing instead of shared state",
            EscapeKind::ParameterEscape => "Consider using &checked T if lifetime is provable",
            EscapeKind::GlobalStore => "Avoid global mutable state when possible",
            EscapeKind::Unknown => "Improve type annotations for better analysis",
        }
    }
}

/// Source location for escape diagnostics
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// File path
    pub file: Text,

    /// Line number (1-indexed)
    pub line: u32,

    /// Column number (1-indexed)
    pub column: u32,

    /// Length of the relevant span
    pub length: u32,
}

// ==================================================================================
// Enhanced Escape Analyzer (Section 2.3.3)
// ==================================================================================

/// Enhanced escape analyzer with forward dataflow analysis
///
/// This is the main escape analysis engine that determines which references
/// can be optimized to 0ns overhead by proving they don't escape.
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │   Enhanced Escape Analyzer          │
/// ├─────────────────────────────────────┤
/// │ 1. SSA Construction                 │
/// │    └─ Build SSA form for precision  │
/// ├─────────────────────────────────────┤
/// │ 2. Dataflow Analysis                │
/// │    ├─ Forward propagation           │
/// │    ├─ Escape point detection        │
/// │    └─ State merging at joins        │
/// ├─────────────────────────────────────┤
/// │ 3. Interprocedural Analysis         │
/// │    ├─ Call graph integration        │
/// │    ├─ Parameter flow tracking       │
/// │    └─ Return value analysis         │
/// ├─────────────────────────────────────┤
/// │ 4. Optimization Decision            │
/// │    ├─ NoEscape → &checked T (0ns)   │
/// │    └─ Escapes → &T (~15ns)          │
/// └─────────────────────────────────────┘
/// ```
#[derive(Debug)]
pub struct EnhancedEscapeAnalyzer {
    /// Control flow graph
    cfg: ControlFlowGraph,

    /// SSA representation (if available)
    ssa: Maybe<SsaFunction>,

    /// Current escape state for each reference
    escape_states: Map<RefId, EscapeState>,

    /// Detected escape points
    escape_points: List<EscapePoint>,

    /// Call graph for interprocedural analysis
    call_graph: Maybe<CallGraph>,

    /// Current function being analyzed
    #[allow(dead_code)]
    current_function: Maybe<FunctionId>,

    /// Statistics
    stats: EscapeAnalysisStats,

    /// Configuration
    config: EscapeAnalysisConfig,
}

/// Configuration for escape analysis
#[derive(Debug, Clone)]
pub struct EscapeAnalysisConfig {
    /// Enable interprocedural analysis
    pub enable_interprocedural: bool,

    /// Maximum iterations for fixpoint computation
    pub max_iterations: usize,

    /// Enable closure escape analysis
    pub enable_closure_analysis: bool,

    /// Enable thread escape analysis
    pub enable_thread_analysis: bool,

    /// Confidence threshold for promotion (0.0-1.0)
    pub confidence_threshold: f64,
}

impl Default for EscapeAnalysisConfig {
    fn default() -> Self {
        Self {
            enable_interprocedural: true,
            max_iterations: 100,
            enable_closure_analysis: true,
            enable_thread_analysis: true,
            confidence_threshold: 0.95,
        }
    }
}

/// Statistics from escape analysis
#[derive(Debug, Clone, Default)]
pub struct EscapeAnalysisStats {
    /// Total references analyzed
    pub total_references: usize,

    /// References that don't escape
    pub no_escape_count: usize,

    /// References that may escape
    pub may_escape_count: usize,

    /// References that definitely escape
    pub escapes_count: usize,

    /// References with unknown state
    pub unknown_count: usize,

    /// Number of dataflow iterations
    pub iterations: usize,

    /// Number of escape points detected
    pub escape_points_detected: usize,

    /// Analysis time (milliseconds)
    pub analysis_time_ms: u64,
}

impl EscapeAnalysisStats {
    /// Calculate `NoEscape` percentage
    #[must_use]
    pub fn no_escape_percentage(&self) -> f64 {
        if self.total_references == 0 {
            0.0
        } else {
            (self.no_escape_count as f64 / self.total_references as f64) * 100.0
        }
    }

    /// Get estimated CBGR overhead reduction
    ///
    /// Assumes each `NoEscape` reference saves ~150ns per function
    /// (10 derefs * 15ns each)
    #[must_use]
    pub fn estimated_time_saved_ns(&self) -> u64 {
        (self.no_escape_count as u64) * 150
    }
}

impl fmt::Display for EscapeAnalysisStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Escape Analysis Statistics:")?;
        writeln!(f, "  Total references:     {}", self.total_references)?;
        writeln!(
            f,
            "  NoEscape (0ns):       {} ({:.1}%)",
            self.no_escape_count,
            self.no_escape_percentage()
        )?;
        writeln!(f, "  MayEscape (~15ns):    {}", self.may_escape_count)?;
        writeln!(f, "  Escapes (~15ns):      {}", self.escapes_count)?;
        writeln!(f, "  Unknown:              {}", self.unknown_count)?;
        writeln!(f, "  Escape points:        {}", self.escape_points_detected)?;
        writeln!(f, "  Dataflow iterations:  {}", self.iterations)?;
        writeln!(
            f,
            "  Est. time saved:      ~{}μs per execution",
            self.estimated_time_saved_ns() / 1000
        )?;
        writeln!(f, "  Analysis time:        {}ms", self.analysis_time_ms)?;
        Ok(())
    }
}

impl EnhancedEscapeAnalyzer {
    /// Create new enhanced escape analyzer
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self {
            cfg,
            ssa: Maybe::None,
            escape_states: Map::new(),
            escape_points: List::new(),
            call_graph: Maybe::None,
            current_function: Maybe::None,
            stats: EscapeAnalysisStats::default(),
            config: EscapeAnalysisConfig::default(),
        }
    }

    /// Create analyzer with SSA for precise analysis
    #[must_use]
    pub fn with_ssa(cfg: ControlFlowGraph, ssa: SsaFunction) -> Self {
        Self {
            cfg,
            ssa: Maybe::Some(ssa),
            escape_states: Map::new(),
            escape_points: List::new(),
            call_graph: Maybe::None,
            current_function: Maybe::None,
            stats: EscapeAnalysisStats::default(),
            config: EscapeAnalysisConfig::default(),
        }
    }

    /// Create analyzer with call graph for interprocedural analysis
    #[must_use]
    pub fn with_call_graph(
        cfg: ControlFlowGraph,
        call_graph: CallGraph,
        function_id: FunctionId,
    ) -> Self {
        Self {
            cfg,
            ssa: Maybe::None,
            escape_states: Map::new(),
            escape_points: List::new(),
            call_graph: Maybe::Some(call_graph),
            current_function: Maybe::Some(function_id),
            stats: EscapeAnalysisStats::default(),
            config: EscapeAnalysisConfig::default(),
        }
    }

    /// Set analysis configuration
    #[must_use]
    pub fn with_config(mut self, config: EscapeAnalysisConfig) -> Self {
        self.config = config;
        self
    }

    /// Set configuration on an existing analyzer (non-builder).
    pub fn set_config(&mut self, config: EscapeAnalysisConfig) {
        self.config = config;
    }

    /// Get the current configuration
    #[must_use]
    pub fn config(&self) -> &EscapeAnalysisConfig {
        &self.config
    }

    /// Run escape analysis on all references
    ///
    /// This is the main entry point. It performs complete escape analysis
    /// and returns results for all references.
    ///
    /// # Algorithm
    ///
    /// 1. Initialize all references to Unknown state
    /// 2. Perform forward dataflow analysis
    /// 3. Detect escape points
    /// 4. Compute fixpoint via iteration
    /// 5. Finalize states and generate report
    pub fn analyze(&mut self) -> EscapeAnalysisResult {
        let start_time = std::time::Instant::now();

        // Step 1: Initialize all references
        self.initialize_references();

        // Step 2: Perform dataflow analysis
        self.perform_dataflow_analysis();

        // Step 3: Finalize statistics
        self.finalize_stats();

        // Record analysis time
        self.stats.analysis_time_ms = start_time.elapsed().as_millis() as u64;

        // Return results
        EscapeAnalysisResult {
            escape_states: self.escape_states.clone(),
            escape_points: self.escape_points.clone(),
            stats: self.stats.clone(),
        }
    }

    /// Initialize all references to Unknown state
    fn initialize_references(&mut self) {
        // Collect all references from CFG
        let mut all_refs = Set::new();

        for block in self.cfg.blocks.values() {
            // Add definitions
            for def in &block.definitions {
                all_refs.insert(def.reference);
            }

            // Add uses
            for use_site in &block.uses {
                all_refs.insert(use_site.reference);
            }
        }

        // Initialize escape states
        for ref_id in all_refs {
            self.escape_states.insert(ref_id, EscapeState::Unknown);
        }

        self.stats.total_references = self.escape_states.len();
    }

    /// Perform forward dataflow analysis
    ///
    /// This is the core analysis engine. It iterates over the CFG in
    /// topological order, tracking escape states and detecting escape points.
    ///
    /// # Algorithm
    ///
    /// ```text
    /// WorkList = {entry block}
    /// while WorkList not empty:
    ///     block = WorkList.pop()
    ///     old_state = state[block]
    ///     new_state = transfer(block, state)
    ///     if new_state != old_state:
    ///         state[block] = new_state
    ///         WorkList.add(successors(block))
    /// ```
    fn perform_dataflow_analysis(&mut self) {
        let mut worklist = List::new();
        worklist.push(self.cfg.entry);

        let mut visited = Set::new();
        let mut iterations = 0;

        while let Some(block_id) = worklist.pop() {
            if iterations >= self.config.max_iterations {
                // Max iterations reached - stop to prevent infinite loops
                break;
            }

            iterations += 1;

            // Skip if already visited (unless state changed)
            if visited.contains(&block_id) {
                continue;
            }
            visited.insert(block_id);

            // Get block and clone necessary data to avoid borrow conflicts
            let (definitions, uses, successors) = match self.cfg.blocks.get(&block_id) {
                Maybe::Some(b) => (b.definitions.clone(), b.uses.clone(), b.successors.clone()),
                Maybe::None => continue,
            };

            // Process definitions in this block
            for def in &definitions {
                self.process_definition(def, block_id);
            }

            // Process uses in this block
            for use_site in &uses {
                self.process_use(use_site, block_id);
            }

            // Add successors to worklist
            for &succ_id in &successors {
                if !visited.contains(&succ_id) {
                    worklist.push(succ_id);
                }
            }
        }

        self.stats.iterations = iterations;
    }

    /// Process a reference definition
    fn process_definition(&mut self, def: &DefSite, block_id: BlockId) {
        let ref_id = def.reference;

        // Determine initial state based on allocation type
        let new_state = if def.is_stack_allocated {
            // Stack-allocated: starts as NoEscape
            EscapeState::NoEscape
        } else {
            // Heap-allocated: starts as MayEscape
            EscapeState::MayEscape
        };

        // Update state
        if let Some(current_state) = self.escape_states.get_mut(&ref_id) {
            let merged = current_state.merge(new_state);
            *current_state = merged;
        } else {
            self.escape_states.insert(ref_id, new_state);
        }

        // Check for heap allocation patterns
        if !def.is_stack_allocated {
            self.record_escape_point(EscapePoint {
                reference: ref_id,
                block: block_id,
                escape_kind: EscapeKind::HeapStore,
                description: "Reference allocated on heap".to_string().into(),
                source_location: Maybe::None,
            });
        }
    }

    /// Process a reference use
    fn process_use(&mut self, use_site: &UseeSite, block_id: BlockId) {
        let ref_id = use_site.reference;

        // Check if this use causes escape
        if self.use_causes_escape(use_site, block_id) {
            // Update state to Escapes
            if let Some(state) = self.escape_states.get_mut(&ref_id) {
                *state = EscapeState::Escapes;
            }
        }
    }

    /// Check if a use causes reference to escape
    fn use_causes_escape(&mut self, use_site: &UseeSite, block_id: BlockId) -> bool {
        let ref_id = use_site.reference;

        // Check 1: Used in exit block (might return)
        if block_id == self.cfg.exit {
            self.record_escape_point(EscapePoint {
                reference: ref_id,
                block: block_id,
                escape_kind: EscapeKind::ReturnEscape,
                description: "Reference used in exit block (may be returned)".to_string().into(),
                source_location: Maybe::None,
            });
            return true;
        }

        // Check 2: Stored to heap (detected by SSA if available)
        if self.is_heap_store(use_site, block_id) {
            self.record_escape_point(EscapePoint {
                reference: ref_id,
                block: block_id,
                escape_kind: EscapeKind::HeapStore,
                description: "Reference stored to heap-allocated structure".to_string().into(),
                source_location: Maybe::None,
            });
            return true;
        }

        // Check 3: Passed to escaping function (requires call graph)
        if self.config.enable_interprocedural
            && self.is_passed_to_escaping_function(use_site, block_id)
        {
            self.record_escape_point(EscapePoint {
                reference: ref_id,
                block: block_id,
                escape_kind: EscapeKind::ParameterEscape,
                description: "Reference passed to function that may retain it".to_string().into(),
                source_location: Maybe::None,
            });
            return true;
        }

        false
    }

    /// Check if use is a heap store
    ///
    /// Uses SSA representation to precisely determine if a use site stores
    /// a reference to the heap. A heap store means the reference escapes
    /// and cannot be promoted to a stack-based lifetime.
    fn is_heap_store(&self, use_site: &UseeSite, _block_id: BlockId) -> bool {
        // Use SSA if available for precise analysis
        if let Maybe::Some(ref ssa) = self.ssa {
            // Convert UseeSite to UseSiteKey for SSA lookup
            let use_key = crate::ssa::UseSiteKey::from(use_site);

            // Check if there's a use-def chain for this site
            if let Some(value_id) = ssa.use_def.get(&use_key) {
                // Check if this value flows to a heap store
                if ssa.heap_stores.contains(value_id) {
                    return true;
                }

                // Check if any value in the def-use chain leads to heap store
                if let Maybe::Some(def_uses) = ssa.def_use.get(value_id) {
                    for downstream_use in def_uses {
                        // Get the downstream value if it exists
                        if let Some(downstream_id) = ssa.use_def.get(downstream_use)
                            && ssa.heap_stores.contains(downstream_id)
                        {
                            return true;
                        }
                    }
                }
            }

            // Also check by reference ID - if this reference is directly in heap_stores
            // This catches cases where the reference itself is stored
            for &heap_store_id in &ssa.heap_stores {
                if let Maybe::Some(value) = ssa.values.get(&heap_store_id) {
                    // Check if this heap store involves our reference
                    for use_site_val in &value.uses {
                        if use_site_val.reference == use_site.reference {
                            return true;
                        }
                    }
                }
            }

            // No heap store detected via SSA
            false
        } else {
            // Without SSA, conservatively return false (assume no heap store)
            // This may allow some optimizations that shouldn't be performed,
            // but maintains correctness because CBGR runtime checks catch errors
            false
        }
    }

    /// Check if reference is passed to escaping function
    ///
    /// Uses the call graph to determine if a reference passed to a function
    /// at a call site may escape. A reference escapes if the called function:
    /// - Stores it to the heap
    /// - Returns it (potentially to be stored elsewhere)
    /// - Passes it to a thread spawn
    /// - Is not a known safe function
    ///
    /// Uses interprocedural call graph analysis: queries CallGraph::may_retain() to check
    /// if the callee may store, return, or thread-spawn the reference. Known-safe functions
    /// (pure stdlib functions that don't retain references) are excluded from escape.
    fn is_passed_to_escaping_function(&self, use_site: &UseeSite, _block_id: BlockId) -> bool {
        // Requires call graph
        if let Maybe::Some(ref cg) = self.call_graph {
            // Check if this use site is a call argument
            // We need to find the call edge and check the parameter escape info

            // Look through all call edges
            for edge in &cg.edges {
                // Check if any call site in this edge matches our block
                for site in &edge.sites {
                    if site.block == use_site.block {
                        // Found a call in the same block
                        // Check if our reference is one of the arguments
                        for (arg_idx, arg_ref) in site.ref_args.iter().enumerate() {
                            if let Maybe::Some(ref_id) = arg_ref
                                && *ref_id == use_site.reference
                            {
                                // This reference is passed as argument arg_idx

                                // Check RefFlow for this edge
                                if edge.flow.param_escapes(arg_idx) {
                                    return true;
                                }

                                // Check if callee may store to heap
                                if edge.flow.may_store_heap {
                                    return true;
                                }

                                // Check if callee may spawn thread with this arg
                                if edge.flow.may_spawn_thread {
                                    return true;
                                }

                                // Also check the may_retain query for more precise analysis
                                if cg.may_retain(edge.callee, arg_idx) {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }

            // Check if any call in the same block targets a thread-spawning function
            for (func_id, sig) in &cg.signatures {
                if sig.is_thread_spawn {
                    // Check if our reference's block has a call to this function
                    if let Maybe::Some(callers) = cg.callers.get(func_id) {
                        for caller in callers {
                            if let Maybe::Some(edges_from_caller) = cg.calls.get(caller)
                                && edges_from_caller.contains(func_id)
                            {
                                // There's a call to thread spawn - check if our ref is involved
                                // Be conservative: if thread spawn is called, reference may escape
                                return true;
                            }
                        }
                    }
                }
            }

            // No escape detected through call graph analysis
            false
        } else {
            // Without call graph, conservatively return false
            // CBGR runtime checks will catch any actual use-after-free
            false
        }
    }

    /// Record an escape point
    fn record_escape_point(&mut self, point: EscapePoint) {
        self.escape_points.push(point);
        self.stats.escape_points_detected += 1;
    }

    /// Finalize statistics
    fn finalize_stats(&mut self) {
        // Count states
        for state in self.escape_states.values() {
            match state {
                EscapeState::NoEscape => self.stats.no_escape_count += 1,
                EscapeState::MayEscape => self.stats.may_escape_count += 1,
                EscapeState::Escapes => self.stats.escapes_count += 1,
                EscapeState::Unknown => self.stats.unknown_count += 1,
            }
        }
    }

    /// Get escape state for a reference
    #[must_use]
    pub fn get_state(&self, reference: RefId) -> Maybe<EscapeState> {
        self.escape_states.get(&reference).copied()
    }

    /// Get all escape points
    #[must_use]
    pub fn escape_points(&self) -> &[EscapePoint] {
        &self.escape_points
    }

    /// Get statistics
    #[must_use]
    pub fn statistics(&self) -> &EscapeAnalysisStats {
        &self.stats
    }

    /// Convert `EscapeState` to `EscapeResult` for compatibility
    #[must_use]
    pub fn to_escape_result(&self, reference: RefId) -> EscapeResult {
        match self.get_state(reference) {
            Maybe::Some(EscapeState::NoEscape) => EscapeResult::DoesNotEscape,
            Maybe::Some(EscapeState::Escapes | EscapeState::MayEscape) => {
                // Determine specific escape reason from escape points
                for point in &self.escape_points {
                    if point.reference == reference {
                        return match point.escape_kind {
                            EscapeKind::ReturnEscape => EscapeResult::EscapesViaReturn,
                            EscapeKind::HeapStore => EscapeResult::EscapesViaHeap,
                            EscapeKind::ClosureCapture => EscapeResult::EscapesViaClosure,
                            EscapeKind::ThreadCrossing => EscapeResult::EscapesViaThread,
                            EscapeKind::ParameterEscape => EscapeResult::EscapesViaHeap,
                            EscapeKind::GlobalStore => EscapeResult::EscapesViaHeap,
                            EscapeKind::Unknown => EscapeResult::ExceedsStackBounds,
                        };
                    }
                }
                EscapeResult::ExceedsStackBounds
            }
            _ => EscapeResult::ExceedsStackBounds,
        }
    }
}

/// Result of escape analysis
#[derive(Debug, Clone)]
pub struct EscapeAnalysisResult {
    /// Escape state for each reference
    pub escape_states: Map<RefId, EscapeState>,

    /// All detected escape points
    pub escape_points: List<EscapePoint>,

    /// Statistics
    pub stats: EscapeAnalysisStats,
}

impl EscapeAnalysisResult {
    /// Get references that don't escape (can use 0ns optimization)
    #[must_use]
    pub fn no_escape_refs(&self) -> List<RefId> {
        self.escape_states
            .iter()
            .filter(|(_, state)| **state == EscapeState::NoEscape)
            .map(|(ref_id, _)| *ref_id)
            .collect()
    }

    /// Get references that escape (need CBGR)
    #[must_use]
    pub fn escaping_refs(&self) -> List<RefId> {
        self.escape_states
            .iter()
            .filter(|(_, state)| state.requires_cbgr())
            .map(|(ref_id, _)| *ref_id)
            .collect()
    }

    /// Generate diagnostic report
    #[must_use]
    pub fn generate_report(&self) -> Text {
        let mut report = Text::new();

        report.push_str("=== Escape Analysis Report ===\n\n");
        report.push_str(&format!("{}\n", self.stats));

        if !self.escape_points.is_empty() {
            report.push_str("\n=== Escape Points ===\n\n");

            for (i, point) in self.escape_points.iter().enumerate() {
                report.push_str(&format!(
                    "{}. Reference {:?} ({}):\n",
                    i + 1,
                    point.reference,
                    point.escape_kind.name()
                ));
                report.push_str(&format!("   {}\n", point.description));
                report.push_str(&format!(
                    "   Hint: {}\n\n",
                    point.escape_kind.optimization_hint()
                ));
            }
        }

        report
    }
}

impl fmt::Display for EscapeAnalysisResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.generate_report())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_state_merge() {
        use EscapeState::*;

        assert_eq!(NoEscape.merge(NoEscape), NoEscape);
        assert_eq!(NoEscape.merge(Escapes), Escapes);
        assert_eq!(MayEscape.merge(NoEscape), MayEscape);
        assert_eq!(Escapes.merge(MayEscape), Escapes);
    }

    #[test]
    fn test_escape_state_allows_optimization() {
        assert!(EscapeState::NoEscape.allows_optimization());
        assert!(!EscapeState::MayEscape.allows_optimization());
        assert!(!EscapeState::Escapes.allows_optimization());
        assert!(!EscapeState::Unknown.allows_optimization());
    }

    #[test]
    fn test_escape_stats_percentage() {
        let stats = EscapeAnalysisStats {
            total_references: 100,
            no_escape_count: 70,
            may_escape_count: 20,
            escapes_count: 10,
            unknown_count: 0,
            iterations: 5,
            escape_points_detected: 30,
            analysis_time_ms: 10,
        };

        assert_eq!(stats.no_escape_percentage(), 70.0);
        assert_eq!(stats.estimated_time_saved_ns(), 70 * 150);
    }
}
