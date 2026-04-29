//! CBGR Check Elimination via Escape Analysis
//!
//! This module implements CBGR check elimination for the Verum verification system.
//! CBGR checks are eliminated in AOT code only when escape analysis can prove
//! references don't outlive their allocations.
//!
//! # Core Algorithm
//!
//! For `&T` -> `&checked T` promotion, ALL of these must be proven:
//! 1. **Reference doesn't escape function scope**
//!    - Not returned from the function
//!    - Not stored in heap-allocated structures
//!    - Not captured by closures that outlive the scope
//! 2. **No concurrent access possible**
//!    - Reference is not shared across thread boundaries
//!    - No data races can occur
//! 3. **Allocation dominates all uses**
//!    - Every path that uses the reference goes through the allocation
//! 4. **Lifetime is stack-bounded**
//!    - Reference lifetime bounded by stack frame
//!    - Deallocation occurs before function return
//!
//! # Safety Requirements
//!
//! - **NEVER** eliminate a check if escape status is Unknown
//! - Conservative by default - only eliminate when proven safe
//! - Must maintain 100% memory safety (zero false negatives)
//!
//! # Performance Impact
//!
//! - Automatic optimization: ~15ns -> 0ns per dereference
//! - Zero developer effort (completely automatic)
//! - Falls back to CBGR if cannot prove safety
//!
//! # Example
//!
//! ```rust
//! use verum_verification::cbgr_elimination::{
//!     EscapeStatus, EscapeAnalysisResult, CBGROptimizer, OptimizationConfig,
//! };
//!
//! // Create optimizer with conservative settings
//! let optimizer = CBGROptimizer::new(OptimizationConfig::conservative());
//! ```

use std::collections::HashSet;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use verum_common::{List, Map, Text};

// =============================================================================
// Core Types
// =============================================================================

/// Reference variable identifier for tracking in escape analysis
///
/// Tracked by escape analysis to determine CBGR check eligibility.
///
/// Note: Named `RefVariable` to avoid conflict with vcgen::Variable
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefVariable {
    /// Unique identifier
    pub id: u64,
    /// Whether this variable represents a reference type
    pub is_reference: bool,
}

impl RefVariable {
    /// Create a new variable
    pub fn new(id: u64, is_reference: bool) -> Self {
        Self { id, is_reference }
    }

    /// Create a reference variable
    pub fn reference(id: u64) -> Self {
        Self::new(id, true)
    }

    /// Create a non-reference variable
    pub fn value(id: u64) -> Self {
        Self::new(id, false)
    }
}

impl fmt::Display for RefVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_reference {
            write!(f, "&var_{}", self.id)
        } else {
            write!(f, "var_{}", self.id)
        }
    }
}

/// Scope identifier for tracking reference lifetimes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopeId(pub u64);

impl ScopeId {
    /// Create a new scope ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Block identifier for CFG analysis
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub u64);

impl BlockId {
    /// Create a new block ID
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

// =============================================================================
// Escape Status
// =============================================================================

/// Result of escape analysis for a reference
///
/// The escape status of a reference after analysis. Only `NoEscape` allows
/// CBGR check elimination (promoting &T to &checked T with 0ns overhead).
/// All other statuses require keeping the ~15ns CBGR check for memory safety.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EscapeStatus {
    /// Reference doesn't escape - CBGR check can be eliminated
    ///
    /// This status indicates that the reference:
    /// - Doesn't escape function scope
    /// - Has no concurrent access
    /// - Allocation dominates all uses
    /// - Lifetime is stack-bounded
    NoEscape,

    /// Reference escapes to heap - check required
    ///
    /// The reference is stored in a heap-allocated structure (Box, Heap, etc.)
    /// and may outlive the current function scope.
    EscapesToHeap,

    /// Reference escapes through closure - check required
    ///
    /// The reference is captured by a closure that may outlive the current scope.
    EscapesToClosure,

    /// Reference escapes through return - check required
    ///
    /// The reference is returned from the function and must remain valid
    /// for the caller's use.
    EscapesToReturn,

    /// Reference escapes through struct field - check required
    ///
    /// The reference is stored in a struct field that may outlive the current scope.
    EscapesToField,

    /// Reference crosses thread boundaries - check required
    ///
    /// The reference is shared with another thread, requiring CBGR for
    /// concurrent safety.
    EscapesToThread,

    /// Unknown escape status - conservatively keep check
    ///
    /// SAFETY: When escape status cannot be determined, we MUST keep
    /// the CBGR check to ensure memory safety. This is the conservative
    /// default that ensures zero false negatives.
    #[default]
    Unknown,
}

impl EscapeStatus {
    /// Check if CBGR check can be safely eliminated
    ///
    /// # Safety
    ///
    /// Returns `true` ONLY when it has been proven safe to eliminate the check.
    /// Returns `false` for all uncertain or escaping cases.
    pub fn can_eliminate_check(&self) -> bool {
        matches!(self, EscapeStatus::NoEscape)
    }

    /// Get human-readable reason for the escape status
    pub fn reason(&self) -> &'static str {
        match self {
            EscapeStatus::NoEscape => "Reference does not escape (safe to eliminate CBGR check)",
            EscapeStatus::EscapesToHeap => "Reference escapes to heap allocation",
            EscapeStatus::EscapesToClosure => "Reference captured by escaping closure",
            EscapeStatus::EscapesToReturn => "Reference escapes via function return",
            EscapeStatus::EscapesToField => "Reference escapes via struct field",
            EscapeStatus::EscapesToThread => "Reference crosses thread boundaries",
            EscapeStatus::Unknown => "Escape status unknown (conservative: keep CBGR check)",
        }
    }

    /// Get CBGR overhead in nanoseconds
    pub fn cbgr_overhead_ns(&self) -> u32 {
        match self {
            EscapeStatus::NoEscape => 0,
            _ => 15, // ~15ns per CBGR check
        }
    }

    /// Check if this is a definitive escape (not unknown)
    pub fn is_definitive(&self) -> bool {
        !matches!(self, EscapeStatus::Unknown)
    }
}

impl fmt::Display for EscapeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EscapeStatus::NoEscape => write!(f, "NoEscape (0ns)"),
            EscapeStatus::EscapesToHeap => write!(f, "EscapesToHeap (~15ns)"),
            EscapeStatus::EscapesToClosure => write!(f, "EscapesToClosure (~15ns)"),
            EscapeStatus::EscapesToReturn => write!(f, "EscapesToReturn (~15ns)"),
            EscapeStatus::EscapesToField => write!(f, "EscapesToField (~15ns)"),
            EscapeStatus::EscapesToThread => write!(f, "EscapesToThread (~15ns)"),
            EscapeStatus::Unknown => write!(f, "Unknown (~15ns, conservative)"),
        }
    }
}

// =============================================================================
// Scope Analysis
// =============================================================================

/// Scope information for reference lifetime tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    /// Unique scope identifier
    pub id: ScopeId,
    /// Parent scope (None for function root scope)
    pub parent: Option<ScopeId>,
    /// Children scopes
    pub children: List<ScopeId>,
    /// RefVariables defined in this scope
    pub defined_variables: HashSet<RefVariable>,
    /// Is this scope a loop body?
    pub is_loop: bool,
    /// Is this scope a closure body?
    pub is_closure: bool,
    /// Entry block for this scope
    pub entry_block: BlockId,
    /// Exit blocks for this scope
    pub exit_blocks: List<BlockId>,
}

impl Scope {
    /// Create a new scope
    pub fn new(id: ScopeId, entry_block: BlockId) -> Self {
        Self {
            id,
            parent: None,
            children: List::new(),
            defined_variables: HashSet::new(),
            is_loop: false,
            is_closure: false,
            entry_block,
            exit_blocks: List::new(),
        }
    }

    /// Create a child scope
    pub fn with_parent(id: ScopeId, parent: ScopeId, entry_block: BlockId) -> Self {
        Self {
            id,
            parent: Some(parent),
            children: List::new(),
            defined_variables: HashSet::new(),
            is_loop: false,
            is_closure: false,
            entry_block,
            exit_blocks: List::new(),
        }
    }

    /// Mark this scope as a loop body
    pub fn set_loop(&mut self, is_loop: bool) {
        self.is_loop = is_loop;
    }

    /// Mark this scope as a closure body
    pub fn set_closure(&mut self, is_closure: bool) {
        self.is_closure = is_closure;
    }

    /// Add a variable defined in this scope
    pub fn add_variable(&mut self, var: RefVariable) {
        self.defined_variables.insert(var);
    }

    /// Add an exit block
    pub fn add_exit_block(&mut self, block: BlockId) {
        self.exit_blocks.push(block);
    }

    /// Check if a variable is defined in this scope
    pub fn contains_variable(&self, var: &RefVariable) -> bool {
        self.defined_variables.contains(var)
    }
}

// =============================================================================
// Function Representation (Simplified)
// =============================================================================

/// Definition site for a variable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefSite {
    /// RefVariable being defined
    pub variable: RefVariable,
    /// Block where definition occurs
    pub block: BlockId,
    /// Scope where definition occurs
    pub scope: ScopeId,
    /// Is this a stack allocation?
    pub is_stack_allocated: bool,
    /// Is this a heap allocation?
    pub is_heap_allocated: bool,
}

/// Use site for a variable
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseSite {
    /// RefVariable being used
    pub variable: RefVariable,
    /// Block where use occurs
    pub block: BlockId,
    /// Is this a mutable use?
    pub is_mutable: bool,
    /// Is this use in a return statement?
    pub is_return: bool,
    /// Is this use storing to a field?
    pub is_field_store: bool,
    /// Is this use in a thread spawn?
    pub is_thread_spawn: bool,
    /// Is this use capturing in a closure?
    pub is_closure_capture: bool,
}

/// Basic block in control flow graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicBlock {
    /// Block identifier
    pub id: BlockId,
    /// Predecessor blocks
    pub predecessors: HashSet<BlockId>,
    /// Successor blocks
    pub successors: HashSet<BlockId>,
    /// Definitions in this block
    pub definitions: List<DefSite>,
    /// Uses in this block
    pub uses: List<UseSite>,
    /// Scope this block belongs to
    pub scope: ScopeId,
}

impl BasicBlock {
    /// Create a new basic block
    pub fn new(id: BlockId, scope: ScopeId) -> Self {
        Self {
            id,
            predecessors: HashSet::new(),
            successors: HashSet::new(),
            definitions: List::new(),
            uses: List::new(),
            scope,
        }
    }

    /// Add a predecessor
    pub fn add_predecessor(&mut self, pred: BlockId) {
        self.predecessors.insert(pred);
    }

    /// Add a successor
    pub fn add_successor(&mut self, succ: BlockId) {
        self.successors.insert(succ);
    }

    /// Add a definition
    pub fn add_definition(&mut self, def: DefSite) {
        self.definitions.push(def);
    }

    /// Add a use
    pub fn add_use(&mut self, use_site: UseSite) {
        self.uses.push(use_site);
    }
}

/// Control flow graph for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFlowGraph {
    /// Basic blocks indexed by ID
    pub blocks: Map<BlockId, BasicBlock>,
    /// Entry block
    pub entry: BlockId,
    /// Exit blocks
    pub exits: List<BlockId>,
    /// Scopes in the function
    pub scopes: Map<ScopeId, Scope>,
    /// Root scope
    pub root_scope: ScopeId,
}

impl ControlFlowGraph {
    /// Create a new CFG
    pub fn new(entry: BlockId, root_scope: ScopeId) -> Self {
        Self {
            blocks: Map::new(),
            entry,
            exits: List::new(),
            scopes: Map::new(),
            root_scope,
        }
    }

    /// Add a basic block
    pub fn add_block(&mut self, block: BasicBlock) {
        self.blocks.insert(block.id, block);
    }

    /// Add a scope
    pub fn add_scope(&mut self, scope: Scope) {
        self.scopes.insert(scope.id, scope);
    }

    /// Add an exit block
    pub fn add_exit(&mut self, exit: BlockId) {
        self.exits.push(exit);
    }

    /// Check if block A dominates block B
    ///
    /// A dominates B if every path from entry to B goes through A.
    ///
    /// A dominates B if every path from entry to B goes through A.
    /// Used to verify allocation dominates all reference uses.
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if a == b {
            return true;
        }

        // Compute dominators using iterative algorithm
        let dom = self.compute_dominators();
        dom.get(&b).map(|doms| doms.contains(&a)).unwrap_or(false)
    }

    /// Compute dominator sets for all blocks
    fn compute_dominators(&self) -> Map<BlockId, HashSet<BlockId>> {
        let mut dominators: Map<BlockId, HashSet<BlockId>> = Map::new();

        // Initialize: entry dominates only itself
        let mut entry_set = HashSet::new();
        entry_set.insert(self.entry);
        dominators.insert(self.entry, entry_set);

        // Initialize: all other blocks dominated by all blocks
        let all_blocks: HashSet<BlockId> = self.blocks.keys().copied().collect();
        for &block_id in all_blocks.iter() {
            if block_id != self.entry {
                dominators.insert(block_id, all_blocks.clone());
            }
        }

        // Iterate until fixed point
        let mut changed = true;
        while changed {
            changed = false;

            for (&block_id, block) in &self.blocks {
                if block_id == self.entry {
                    continue;
                }

                // Compute intersection of dominators of all predecessors
                let mut new_dom = all_blocks.clone();
                for &pred_id in block.predecessors.iter() {
                    if let Some(pred_dom) = dominators.get(&pred_id) {
                        new_dom = new_dom.intersection(pred_dom).copied().collect();
                    }
                }

                // Add block itself
                new_dom.insert(block_id);

                // Check if changed
                if let Some(old_dom) = dominators.get(&block_id)
                    && &new_dom != old_dom
                {
                    dominators.insert(block_id, new_dom);
                    changed = true;
                }
            }
        }

        dominators
    }

    /// Get all use sites for a variable
    fn find_use_sites(&self, var: &RefVariable) -> List<UseSite> {
        let mut uses = List::new();
        for block in self.blocks.values() {
            for use_site in block.uses.iter() {
                if &use_site.variable == var {
                    uses.push(use_site.clone());
                }
            }
        }
        uses
    }

    /// Get definition site for a variable
    fn find_def_site(&self, var: &RefVariable) -> Option<DefSite> {
        for block in self.blocks.values() {
            for def in block.definitions.iter() {
                if &def.variable == var {
                    return Some(def.clone());
                }
            }
        }
        None
    }
}

/// Simplified function representation for escape analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// Function name
    pub name: Text,
    /// Control flow graph
    pub cfg: ControlFlowGraph,
    /// Reference variables in this function
    pub reference_vars: List<RefVariable>,
    /// Does this function spawn threads?
    pub spawns_threads: bool,
    /// Return type contains references?
    pub returns_reference: bool,
}

impl Function {
    /// Create a new function
    pub fn new(name: Text, cfg: ControlFlowGraph) -> Self {
        Self {
            name,
            cfg,
            reference_vars: List::new(),
            spawns_threads: false,
            returns_reference: false,
        }
    }

    /// Add a reference variable
    pub fn add_reference_var(&mut self, var: RefVariable) {
        self.reference_vars.push(var);
    }

    /// Set whether function spawns threads
    pub fn set_spawns_threads(&mut self, spawns: bool) {
        self.spawns_threads = spawns;
    }

    /// Set whether function returns a reference
    pub fn set_returns_reference(&mut self, returns_ref: bool) {
        self.returns_reference = returns_ref;
    }
}

// =============================================================================
// Escape Analysis Result
// =============================================================================

/// Analysis result for a function
///
/// Complete analysis result for a function: maps each reference to its escape status,
/// records which CBGR checks can be eliminated, and provides timing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscapeAnalysisResult {
    /// Name of the analyzed function
    pub function_name: Text,
    /// Escape status for each reference variable
    pub reference_status: Map<RefVariable, EscapeStatus>,
    /// Number of CBGR checks that can be eliminated
    pub eliminated_checks: usize,
    /// Total number of CBGR checks in the function
    pub total_checks: usize,
    /// Time spent on analysis
    pub analysis_duration: Duration,
    /// RefVariables that were promoted from &T to &checked T
    pub promoted_variables: List<RefVariable>,
    /// RefVariables that could not be promoted with reasons
    pub unpromoted_reasons: Map<RefVariable, Text>,
}

impl EscapeAnalysisResult {
    /// Create a new analysis result
    pub fn new(function_name: Text) -> Self {
        Self {
            function_name,
            reference_status: Map::new(),
            eliminated_checks: 0,
            total_checks: 0,
            analysis_duration: Duration::ZERO,
            promoted_variables: List::new(),
            unpromoted_reasons: Map::new(),
        }
    }

    /// Record escape status for a variable
    pub fn record_status(&mut self, var: RefVariable, status: EscapeStatus) {
        self.total_checks += 1;
        if status.can_eliminate_check() {
            self.eliminated_checks += 1;
            self.promoted_variables.push(var);
        } else {
            self.unpromoted_reasons
                .insert(var, Text::from(status.reason()));
        }
        self.reference_status.insert(var, status);
    }

    /// Get elimination rate as a percentage
    pub fn elimination_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.eliminated_checks as f64 / self.total_checks as f64) * 100.0
        }
    }

    /// Estimated time saved in nanoseconds
    pub fn estimated_time_saved_ns(&self) -> u64 {
        // ~15ns saved per eliminated check per dereference
        // Assume average 10 dereferences per reference
        (self.eliminated_checks as u64) * 15 * 10
    }
}

impl fmt::Display for EscapeAnalysisResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Escape Analysis Result for '{}':", self.function_name)?;
        writeln!(f, "  Total references: {}", self.total_checks)?;
        writeln!(
            f,
            "  Eliminated checks: {} ({:.1}%)",
            self.eliminated_checks,
            self.elimination_rate()
        )?;
        writeln!(f, "  Analysis time: {:?}", self.analysis_duration)?;
        writeln!(
            f,
            "  Estimated time saved: ~{}ns",
            self.estimated_time_saved_ns()
        )?;

        if !self.promoted_variables.is_empty() {
            writeln!(f, "\n  Promoted variables (&T -> &checked T):")?;
            for var in self.promoted_variables.iter() {
                writeln!(f, "    - {}", var)?;
            }
        }

        if !self.unpromoted_reasons.is_empty() {
            writeln!(f, "\n  Non-promoted variables:")?;
            for (var, reason) in self.unpromoted_reasons.iter() {
                writeln!(f, "    - {}: {}", var, reason)?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Optimization Configuration
// =============================================================================

/// Configuration for CBGR optimization
///
/// Controls aggressiveness of CBGR elimination: conservative mode only
/// eliminates trivially safe cases, aggressive mode enables interprocedural
/// analysis at the cost of longer compile times.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationConfig {
    /// Enable aggressive optimization (may increase compile time)
    pub aggressive: bool,
    /// Maximum analysis depth for interprocedural analysis
    pub max_analysis_depth: usize,
    /// Trust user annotations (@verify(static), etc.)
    pub trust_annotations: bool,
    /// Enable interprocedural escape analysis
    pub interprocedural: bool,
    /// Maximum time to spend on analysis per function (ms)
    pub timeout_ms: u64,
}

impl OptimizationConfig {
    /// Create conservative configuration (default)
    pub fn conservative() -> Self {
        Self {
            aggressive: false,
            max_analysis_depth: 2,
            trust_annotations: true,
            interprocedural: false,
            timeout_ms: 1000,
        }
    }

    /// Create balanced configuration
    pub fn balanced() -> Self {
        Self {
            aggressive: false,
            max_analysis_depth: 5,
            trust_annotations: true,
            interprocedural: true,
            timeout_ms: 5000,
        }
    }

    /// Create aggressive configuration (more optimization, more compile time)
    pub fn aggressive() -> Self {
        Self {
            aggressive: true,
            max_analysis_depth: 10,
            trust_annotations: true,
            interprocedural: true,
            timeout_ms: 30000,
        }
    }
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self::conservative()
    }
}

// =============================================================================
// CBGR Optimizer
// =============================================================================

/// CBGR optimization pass
///
/// Performs escape analysis to determine which CBGR checks can be safely
/// eliminated (promoting &T to &checked T). Guarantees zero false negatives --
/// any check eliminated is proven safe. The promotion rule is:
///   no_escape(&T) /\ allocation_dominates_uses(&T) => promote &T to &checked T
#[derive(Debug)]
pub struct CBGROptimizer {
    /// Configuration for optimization
    pub config: OptimizationConfig,
    /// Statistics from optimization runs
    stats: OptimizationStats,
}

/// Statistics from optimization runs
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OptimizationStats {
    /// Total functions analyzed
    pub functions_analyzed: usize,
    /// Total references analyzed
    pub references_analyzed: usize,
    /// Total checks eliminated
    pub checks_eliminated: usize,
    /// Total checks preserved
    pub checks_preserved: usize,
    /// Total analysis time
    pub total_analysis_time: Duration,
}

impl CBGROptimizer {
    /// Create a new optimizer with the given configuration
    pub fn new(config: OptimizationConfig) -> Self {
        Self {
            config,
            stats: OptimizationStats::default(),
        }
    }

    /// Create optimizer with conservative settings
    pub fn conservative() -> Self {
        Self::new(OptimizationConfig::conservative())
    }

    /// Create optimizer with balanced settings
    pub fn balanced() -> Self {
        Self::new(OptimizationConfig::balanced())
    }

    /// Create optimizer with aggressive settings
    pub fn aggressive_mode() -> Self {
        Self::new(OptimizationConfig::aggressive())
    }

    /// Get optimization statistics
    pub fn stats(&self) -> &OptimizationStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = OptimizationStats::default();
    }

    /// Read mirror of `OptimizationConfig.aggressive`. Surfaced
    /// for orchestrators that want to know which preset stance
    /// the optimizer is running under without re-reading the
    /// config struct.
    #[must_use]
    pub fn aggressive_enabled(&self) -> bool {
        self.config.aggressive
    }

    /// Read mirror of `OptimizationConfig.interprocedural`.
    /// Embedders that compose the optimizer with their own
    /// call-graph walker consult this to decide whether to
    /// pre-resolve callees before invoking `analyze_escape`.
    #[must_use]
    pub fn interprocedural_enabled(&self) -> bool {
        self.config.interprocedural
    }

    /// Read mirror of `OptimizationConfig.trust_annotations`.
    /// When `true`, callers should pre-filter variables that
    /// carry `@verify(static)` / equivalent escape-proven
    /// annotations and skip them — the optimizer's analysis
    /// would just confirm what the annotation already states.
    #[must_use]
    pub fn trust_annotations_enabled(&self) -> bool {
        self.config.trust_annotations
    }

    /// Read mirror of `OptimizationConfig.max_analysis_depth`.
    /// Embedders that do their own callee recursion consult
    /// this to bound their own walk symmetrically with the
    /// optimizer's documented depth limit.
    #[must_use]
    pub fn max_analysis_depth(&self) -> usize {
        self.config.max_analysis_depth
    }

    /// Read mirror of `OptimizationConfig.timeout_ms`. The
    /// `analyze_escape` method honours this as a per-function
    /// wall-clock budget — analysis bails when the budget is
    /// exhausted, returning a partial result whose preserved
    /// (non-eliminated) entries reflect the conservative
    /// fallback behaviour for un-analysed variables.
    #[must_use]
    pub fn timeout_ms(&self) -> u64 {
        self.config.timeout_ms
    }

    /// Analyze escape status of all references in a function
    ///
    /// Main entry point for escape analysis. Analyzes each reference variable
    /// in the function and determines its escape status by checking:
    /// (1) reference doesn't escape scope, (2) no concurrent access,
    /// (3) allocation dominates all uses, (4) lifetime is stack-bounded.
    ///
    /// Honours `config.timeout_ms` as a wall-clock per-function
    /// budget. When the budget is exhausted mid-analysis, the
    /// remaining variables are recorded as `EscapeStatus::Unknown`
    /// (conservative fallback — checks are preserved, not
    /// eliminated). Before this wire-up the field was inert —
    /// pathological inputs could run unbounded regardless of
    /// configured budget.
    pub fn analyze_escape(&mut self, func: &Function) -> EscapeAnalysisResult {
        let start = Instant::now();
        let mut result = EscapeAnalysisResult::new(func.name.clone());

        let budget = if self.config.timeout_ms == 0 {
            // 0 = unlimited.
            None
        } else {
            Some(Duration::from_millis(self.config.timeout_ms))
        };

        for var in func.reference_vars.iter() {
            let status = if let Some(budget) = budget {
                if start.elapsed() >= budget {
                    EscapeStatus::Unknown
                } else {
                    self.analyze_variable_escape(func, var)
                }
            } else {
                self.analyze_variable_escape(func, var)
            };
            result.record_status(*var, status);
        }

        result.analysis_duration = start.elapsed();

        // Update statistics
        self.stats.functions_analyzed += 1;
        self.stats.references_analyzed += func.reference_vars.len();
        self.stats.checks_eliminated += result.eliminated_checks;
        self.stats.checks_preserved += result.total_checks - result.eliminated_checks;
        self.stats.total_analysis_time += result.analysis_duration;

        result
    }

    /// Analyze escape status of a single variable
    fn analyze_variable_escape(&self, func: &Function, var: &RefVariable) -> EscapeStatus {
        // Only analyze reference variables
        if !var.is_reference {
            return EscapeStatus::NoEscape;
        }

        // Criterion 1: Check for return escape
        if self.escapes_via_return(func, var) {
            return EscapeStatus::EscapesToReturn;
        }

        // Criterion 2: Check for heap escape
        if self.escapes_via_heap(func, var) {
            return EscapeStatus::EscapesToHeap;
        }

        // Criterion 3: Check for closure escape
        if self.escapes_via_closure(func, var) {
            return EscapeStatus::EscapesToClosure;
        }

        // Criterion 4: Check for field escape
        if self.escapes_via_field(func, var) {
            return EscapeStatus::EscapesToField;
        }

        // Criterion 5: Check for thread escape
        if self.escapes_via_thread(func, var) {
            return EscapeStatus::EscapesToThread;
        }

        // Criterion 6: Check allocation dominance
        if !self.allocation_dominates_uses(func, var) {
            return EscapeStatus::Unknown;
        }

        // Criterion 7: Check stack-boundedness
        if !self.is_stack_bounded(func, var) {
            return EscapeStatus::Unknown;
        }

        // All criteria passed - safe to eliminate check
        EscapeStatus::NoEscape
    }

    /// Check if variable escapes via function return
    fn escapes_via_return(&self, func: &Function, var: &RefVariable) -> bool {
        if !func.returns_reference {
            return false;
        }

        let use_sites = func.cfg.find_use_sites(var);
        use_sites.iter().any(|use_site| use_site.is_return)
    }

    /// Check if variable escapes via heap allocation
    fn escapes_via_heap(&self, func: &Function, var: &RefVariable) -> bool {
        // Check if defined via heap allocation
        if let Some(def) = func.cfg.find_def_site(var)
            && def.is_heap_allocated
        {
            return true;
        }

        // No heap escape found
        false
    }

    /// Check if variable escapes via closure capture
    fn escapes_via_closure(&self, func: &Function, var: &RefVariable) -> bool {
        let use_sites = func.cfg.find_use_sites(var);
        use_sites.iter().any(|use_site| use_site.is_closure_capture)
    }

    /// Check if variable escapes via struct field store
    fn escapes_via_field(&self, func: &Function, var: &RefVariable) -> bool {
        let use_sites = func.cfg.find_use_sites(var);
        use_sites.iter().any(|use_site| use_site.is_field_store)
    }

    /// Check if variable escapes via thread spawn
    fn escapes_via_thread(&self, func: &Function, var: &RefVariable) -> bool {
        if !func.spawns_threads {
            return false;
        }

        let use_sites = func.cfg.find_use_sites(var);
        use_sites.iter().any(|use_site| use_site.is_thread_spawn)
    }

    /// Check if allocation dominates all uses
    ///
    /// Verify that allocation dominates all uses: every path from entry to
    /// each use site goes through the allocation. This ensures the reference
    /// is valid (allocated) at all points where it is dereferenced.
    fn allocation_dominates_uses(&self, func: &Function, var: &RefVariable) -> bool {
        // Find definition site
        let def_site = match func.cfg.find_def_site(var) {
            Some(def) => def,
            None => return false, // No definition found - conservative
        };

        // Check if definition dominates all uses
        let use_sites = func.cfg.find_use_sites(var);
        for use_site in use_sites.iter() {
            if !func.cfg.dominates(def_site.block, use_site.block) {
                return false;
            }
        }

        true
    }

    /// Check if variable lifetime is stack-bounded
    ///
    /// Check if variable lifetime is bounded by the stack frame (not heap-allocated).
    fn is_stack_bounded(&self, func: &Function, var: &RefVariable) -> bool {
        // Check if defined via stack allocation
        if let Some(def) = func.cfg.find_def_site(var) {
            return def.is_stack_allocated;
        }

        false
    }
}

// =============================================================================
// Public API Functions
// =============================================================================

/// Analyze escape status for all references in a function
///
/// Convenience function that creates an optimizer with default settings
/// and analyzes escape status for all references in a function.
///
/// # Example
///
/// ```ignore
/// let result = analyze_escape(&function);
/// println!("Eliminated {} checks", result.eliminated_checks);
/// ```
pub fn analyze_escape(func: &Function) -> EscapeAnalysisResult {
    let mut optimizer = CBGROptimizer::conservative();
    optimizer.analyze_escape(func)
}

/// Check if a CBGR check can be eliminated for a variable
///
/// Returns `true` only if it can be proven that the reference doesn't
/// escape and all four safety criteria are met (no escape, no concurrent
/// access, allocation dominates uses, stack-bounded lifetime).
///
/// # Safety
///
/// This function guarantees zero false negatives. If it returns `true`,
/// the check can be safely eliminated.
pub fn can_eliminate_check(var: &RefVariable, analysis: &EscapeAnalysisResult) -> bool {
    analysis
        .reference_status
        .get(var)
        .map(|status| status.can_eliminate_check())
        .unwrap_or(false) // Conservative: if not analyzed, don't eliminate
}

/// Optimize a function by recording which checks can be eliminated
///
/// Records which CBGR checks can be eliminated during code generation.
/// The actual elimination happens in the codegen phase.
///
/// Note: The actual Function type doesn't support in-place modification
/// of optimization flags in this simplified representation, so this
/// function just performs analysis and returns the result.
pub fn optimize_function(func: &Function, result: &EscapeAnalysisResult) -> OptimizedFunction {
    OptimizedFunction {
        function_name: func.name.clone(),
        eliminated_checks: result.promoted_variables.clone(),
        preserved_checks: result.unpromoted_reasons.keys().copied().collect(),
        total_savings_ns: result.estimated_time_saved_ns(),
    }
}

/// Result of function optimization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedFunction {
    /// Function name
    pub function_name: Text,
    /// RefVariables with eliminated CBGR checks
    pub eliminated_checks: List<RefVariable>,
    /// RefVariables with preserved CBGR checks
    pub preserved_checks: List<RefVariable>,
    /// Total estimated savings in nanoseconds
    pub total_savings_ns: u64,
}

/// Prove that a variable's scope validity makes CBGR elimination safe
///
/// Proves that a variable's scope validity (lifetime) is contained within
/// the given scope, making CBGR check elimination safe.
pub fn prove_scope_validity(var: &RefVariable, scope: &Scope, cfg: &ControlFlowGraph) -> bool {
    // Check if variable is defined in this scope
    if !scope.contains_variable(var) {
        return false;
    }

    // Check that all uses are within the scope
    let use_sites = cfg.find_use_sites(var);

    for use_site in use_sites.iter() {
        // Get the block's scope
        if let Some(block) = cfg.blocks.get(&use_site.block) {
            // Check if use is in the same scope or a child scope
            if !is_scope_contained(block.scope, scope.id, cfg) {
                return false;
            }
        } else {
            // Block not found - conservative
            return false;
        }
    }

    // All uses are within scope - validity proven
    true
}

/// Check if scope_a is contained within scope_b (same or child)
fn is_scope_contained(scope_a: ScopeId, scope_b: ScopeId, cfg: &ControlFlowGraph) -> bool {
    if scope_a == scope_b {
        return true;
    }

    // Walk up the parent chain
    let mut current = scope_a;
    while let Some(scope) = cfg.scopes.get(&current) {
        if let Some(parent) = scope.parent {
            if parent == scope_b {
                return true;
            }
            current = parent;
        } else {
            break;
        }
    }

    false
}

// =============================================================================
// Errors
// =============================================================================

/// Errors that can occur during CBGR elimination analysis
#[derive(Debug, Error)]
pub enum CBGRAnalysisError {
    /// Analysis timeout
    #[error("analysis timeout after {timeout_ms}ms")]
    Timeout {
        /// Timeout in milliseconds
        timeout_ms: u64,
    },

    /// Invalid CFG
    #[error("invalid control flow graph: {reason}")]
    InvalidCFG {
        /// Reason for invalidity
        reason: String,
    },

    /// Internal error
    #[error("internal error: {0}")]
    Internal(Text),
}

// =============================================================================
// Tests Module Notice
// =============================================================================

// Tests are in tests/cbgr_elimination_tests.rs per CLAUDE.md standards
