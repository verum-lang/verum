//! Dominance Analysis for CBGR Escape Analysis (Phase 3)
//!
//! CBGR requires that allocation dominates all uses for safe promotion of &T to
//! &checked T. This module determines the dominator tree so the promotion decision
//! engine can verify criterion 3 ("allocation dominates all uses in the CFG") and
//! works alongside atomic thread-safe generation tracking (acquire-release ordering)
//! and explicit revocation support (CAP_REVOKE capability, atomic generation increment).
//!
//! This module implements dominance analysis using the Cooper-Harvey-Kennedy algorithm,
//! which is the third phase of the 4-phase escape analysis pipeline:
//!
//! - Phase 1: Build SSA representation (`ssa.rs`)
//! - Phase 2: Track reference flow (`escape_analysis.rs`)
//! - **Phase 3: Dominance analysis (this module)**
//! - Phase 4: Promotion decision (`promotion_decision.rs`)
//!
//! # Algorithm Overview
//!
//! Dominance analysis determines the dominator tree for a control flow graph.
//! A block A dominates block B if every path from the entry to B must pass through A.
//!
//! The Cooper-Harvey-Kennedy algorithm computes dominators efficiently:
//! - Time complexity: O(n * m) where n = blocks, m = edges (linear in practice)
//! - Space complexity: O(n) for immediate dominators
//!
//! # Purpose in Escape Analysis
//!
//! Dominance is critical for safe reference promotion:
//! - For `&T` -> `&checked T` promotion, the allocation must dominate all uses
//! - If a use can occur on a path that doesn't include the allocation, promotion is unsafe
//! - Dominance frontiers indicate where phi nodes are needed (merge points)
//!
//! # Example
//!
//! ```text
//!     Entry (0)
//!       |
//!       v
//!     Alloc (1)    <- allocation site
//!       |
//!     +---+---+
//!     |       |
//!     v       v
//!   Use1(2)  Use2(3)  <- both dominated by Alloc
//!     |       |
//!     +---+---+
//!         |
//!         v
//!      Exit (4)
//! ```
//!
//! In this example, block 1 dominates blocks 2, 3, and 4. Since the allocation
//! in block 1 dominates all use sites, promotion is safe.

use crate::analysis::{BlockId, ControlFlowGraph, DefSite, RefId};
use std::collections::VecDeque;
use std::fmt;
use verum_common::{List, Map, Set};

// ==================================================================================
// Dominance Information (Section 2.4.1)
// ==================================================================================

/// Dominance information computed for a control flow graph
///
/// Contains three key data structures:
/// 1. `dominators`: For each block, the set of blocks that dominate it
/// 2. `immediate_dom`: For each block, its immediate dominator (closest dominator)
/// 3. `dominance_frontier`: For each block, blocks at its dominance frontier
///
/// # Dominance Frontier
///
/// The dominance frontier DF(X) of a block X contains blocks Y where:
/// - X dominates a predecessor of Y
/// - X does not strictly dominate Y
///
/// Dominance frontiers are used for phi node placement in SSA construction
/// and for determining merge points where different reference versions meet.
#[derive(Debug, Clone)]
pub struct DominanceInfo {
    /// For each block, the set of blocks that dominate it
    /// A block always dominates itself
    pub dominators: Map<BlockId, Set<BlockId>>,

    /// Immediate dominator for each block (closest dominator)
    /// Entry block has no immediate dominator
    pub immediate_dom: Map<BlockId, BlockId>,

    /// Dominance frontier for each block
    /// DF(X) = { Y : X dominates a pred of Y but X doesn't strictly dominate Y }
    pub dominance_frontier: Map<BlockId, Set<BlockId>>,

    /// Dominator tree children (reverse of `immediate_dom`)
    /// `dom_tree_children`[X] = { Y : idom(Y) = X }
    pub dom_tree_children: Map<BlockId, Set<BlockId>>,

    /// Entry block ID
    entry: BlockId,

    /// Reverse postorder numbering for efficient intersection
    postorder_numbers: Map<BlockId, usize>,
}

impl DominanceInfo {
    /// Compute dominance information using the Cooper-Harvey-Kennedy algorithm
    ///
    /// This is a simple, fast algorithm that works well in practice:
    /// 1. Initialize: entry dominates only itself, others dominated by all
    /// 2. Iterate in reverse postorder until fixed point
    /// 3. Compute immediate dominators from dominator sets
    /// 4. Build dominator tree and dominance frontiers
    ///
    /// # Algorithm Reference
    ///
    /// Cooper, Harvey, Kennedy. "A Simple, Fast Dominance Algorithm"
    /// Software Practice and Experience, 2001
    ///
    /// # Parameters
    ///
    /// - `cfg`: The control flow graph to analyze
    ///
    /// # Returns
    ///
    /// Complete dominance information for the CFG
    #[must_use]
    pub fn compute(cfg: &ControlFlowGraph) -> Self {
        let mut info = Self {
            dominators: Map::new(),
            immediate_dom: Map::new(),
            dominance_frontier: Map::new(),
            dom_tree_children: Map::new(),
            entry: cfg.entry,
            postorder_numbers: Map::new(),
        };

        // Handle empty CFG
        if cfg.blocks.is_empty() {
            return info;
        }

        // Step 1: Compute reverse postorder and postorder numbers
        let rpo = info.compute_reverse_postorder(cfg);
        info.compute_postorder_numbers(cfg);

        // Step 2: Compute dominators using iterative algorithm
        info.compute_dominators_iterative(cfg, &rpo);

        // Step 3: Compute immediate dominators
        info.compute_immediate_dominators(cfg);

        // Step 4: Build dominator tree
        info.build_dominator_tree(cfg);

        // Step 5: Compute dominance frontiers
        info.compute_dominance_frontiers(cfg);

        info
    }

    /// Compute reverse postorder traversal of CFG
    fn compute_reverse_postorder(&self, cfg: &ControlFlowGraph) -> List<BlockId> {
        let mut result = List::new();
        let mut visited = Set::new();

        self.rpo_visit(cfg.entry, cfg, &mut visited, &mut result);

        result.reverse();
        result
    }

    /// Recursive helper for reverse postorder
    fn rpo_visit(
        &self,
        block_id: BlockId,
        cfg: &ControlFlowGraph,
        visited: &mut Set<BlockId>,
        result: &mut List<BlockId>,
    ) {
        if visited.contains(&block_id) {
            return;
        }
        visited.insert(block_id);

        if let Some(block) = cfg.blocks.get(&block_id) {
            for &succ in &block.successors {
                self.rpo_visit(succ, cfg, visited, result);
            }
        }

        result.push(block_id);
    }

    /// Compute postorder numbers for efficient intersection
    fn compute_postorder_numbers(&mut self, cfg: &ControlFlowGraph) {
        let mut visited = Set::new();
        let mut counter = 0;

        self.postorder_visit(cfg.entry, cfg, &mut visited, &mut counter);
    }

    /// Recursive helper for postorder numbering
    fn postorder_visit(
        &mut self,
        block_id: BlockId,
        cfg: &ControlFlowGraph,
        visited: &mut Set<BlockId>,
        counter: &mut usize,
    ) {
        if visited.contains(&block_id) {
            return;
        }
        visited.insert(block_id);

        if let Some(block) = cfg.blocks.get(&block_id) {
            for &succ in &block.successors {
                self.postorder_visit(succ, cfg, visited, counter);
            }
        }

        self.postorder_numbers.insert(block_id, *counter);
        *counter += 1;
    }

    /// Compute dominators using the Cooper-Harvey-Kennedy iterative algorithm
    fn compute_dominators_iterative(&mut self, cfg: &ControlFlowGraph, rpo: &[BlockId]) {
        // Initialize: entry dominates only itself
        let mut entry_dom = Set::new();
        entry_dom.insert(cfg.entry);
        self.dominators.insert(cfg.entry, entry_dom);

        // Initialize: all other blocks dominated by all blocks (will be refined)
        let all_blocks: Set<BlockId> = cfg.blocks.keys().copied().collect();
        for &block_id in rpo {
            if block_id != cfg.entry {
                self.dominators.insert(block_id, all_blocks.clone());
            }
        }

        // Iterate until fixed point
        let max_iterations = cfg.blocks.len() * cfg.blocks.len() + 10;
        let mut iteration = 0;
        let mut changed = true;

        while changed && iteration < max_iterations {
            changed = false;
            iteration += 1;

            for &block_id in rpo {
                if block_id == cfg.entry {
                    continue;
                }

                if let Some(block) = cfg.blocks.get(&block_id) {
                    // Compute intersection of all predecessors' dominator sets
                    let mut new_dom: Option<Set<BlockId>> = None;

                    for &pred_id in &block.predecessors {
                        if let Some(pred_dom) = self.dominators.get(&pred_id) {
                            new_dom = match new_dom {
                                None => Some(pred_dom.clone()),
                                Some(current) => {
                                    Some(current.intersection(pred_dom).copied().collect())
                                }
                            };
                        }
                    }

                    // Add block itself (every block dominates itself)
                    let mut new_dom = new_dom.unwrap_or_default();
                    new_dom.insert(block_id);

                    // Check if changed
                    if let Some(old_dom) = self.dominators.get(&block_id) {
                        if &new_dom != old_dom {
                            self.dominators.insert(block_id, new_dom);
                            changed = true;
                        }
                    } else {
                        self.dominators.insert(block_id, new_dom);
                        changed = true;
                    }
                }
            }
        }
    }

    /// Compute immediate dominators from dominator sets
    ///
    /// The immediate dominator of block B is the unique block D such that:
    /// - D strictly dominates B (D dominates B and D != B)
    /// - D does not strictly dominate any other strict dominator of B
    ///   (i.e., D is the closest dominator)
    ///
    /// Algorithm: For each block B, find the dominator D such that D has the
    /// most dominators among all dominators of B (closest in dominator tree).
    fn compute_immediate_dominators(&mut self, cfg: &ControlFlowGraph) {
        for (&block_id, dom_set) in &self.dominators {
            if block_id == cfg.entry {
                // Entry has no immediate dominator
                continue;
            }

            // Find immediate dominator: the dominator with the most dominators
            // (i.e., deepest in the dominator tree, closest to block_id)
            let mut idom: Option<BlockId> = None;
            let mut max_dom_count = 0;

            for &dom_id in dom_set {
                if dom_id == block_id {
                    continue;
                }

                // Count how many blocks dominate this candidate
                let dom_count = self
                    .dominators
                    .get(&dom_id)
                    .map_or(0, |s| s.len());

                // The immediate dominator has the largest dominator set
                // (excluding the block itself)
                if dom_count > max_dom_count {
                    max_dom_count = dom_count;
                    idom = Some(dom_id);
                }
            }

            if let Some(idom_id) = idom {
                self.immediate_dom.insert(block_id, idom_id);
            }
        }
    }

    /// Build the dominator tree from immediate dominators
    fn build_dominator_tree(&mut self, cfg: &ControlFlowGraph) {
        // Initialize children sets
        for &block_id in cfg.blocks.keys() {
            self.dom_tree_children.insert(block_id, Set::new());
        }

        // Add children based on immediate dominator relationship
        for (&block_id, &idom_id) in &self.immediate_dom {
            self.dom_tree_children
                .entry(idom_id)
                .or_default()
                .insert(block_id);
        }
    }

    /// Compute dominance frontiers using the algorithm from Cytron et al.
    ///
    /// DF(X) = { Y : X dominates a predecessor of Y but X doesn't strictly dominate Y }
    ///
    /// This uses the formula:
    /// DF(X) = `DF_local(X)` ∪ ∪_{Z ∈ children(X)} `DF_up(Z)`
    ///
    /// where:
    /// - `DF_local(X)` = { Y ∈ succ(X) : idom(Y) != X }
    /// - `DF_up(Z)` = { Y ∈ DF(Z) : idom(Y) != X }
    fn compute_dominance_frontiers(&mut self, cfg: &ControlFlowGraph) {
        // Initialize empty frontiers
        for &block_id in cfg.blocks.keys() {
            self.dominance_frontier.insert(block_id, Set::new());
        }

        // Compute using the Cytron algorithm
        for (&block_id, block) in &cfg.blocks {
            // Only process join points (blocks with multiple predecessors)
            if block.predecessors.len() >= 2 {
                for &pred_id in &block.predecessors {
                    let mut runner = pred_id;

                    // Walk up the dominator tree until we reach block_id's dominator
                    while let Some(&idom) = self.immediate_dom.get(&block_id) {
                        if runner == idom {
                            break;
                        }

                        // Add block_id to runner's dominance frontier
                        self.dominance_frontier
                            .entry(runner)
                            .or_default()
                            .insert(block_id);

                        // Move up to runner's immediate dominator
                        match self.immediate_dom.get(&runner) {
                            Some(&runner_idom) if runner_idom != runner => {
                                runner = runner_idom;
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
    }

    /// Check if block A dominates block B
    ///
    /// A dominates B if every path from entry to B must pass through A.
    /// A block always dominates itself.
    ///
    /// # Parameters
    ///
    /// - `a`: Potential dominator block
    /// - `b`: Block to check for domination
    ///
    /// # Returns
    ///
    /// `true` if A dominates B, `false` otherwise
    #[must_use]
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if a == b {
            return true;
        }

        match self.dominators.get(&b) {
            Some(dom_set) => dom_set.contains(&a),
            None => false,
        }
    }

    /// Check if block A strictly dominates block B
    ///
    /// A strictly dominates B if A dominates B and A != B.
    #[must_use]
    pub fn strictly_dominates(&self, a: BlockId, b: BlockId) -> bool {
        a != b && self.dominates(a, b)
    }

    /// Get the immediate dominator of a block
    ///
    /// Returns `None` for the entry block (which has no immediate dominator).
    #[must_use]
    pub fn get_immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.immediate_dom.get(&block).copied()
    }

    /// Get the dominance frontier of a block
    ///
    /// Returns the set of blocks at the dominance frontier, where phi nodes
    /// may be needed for values defined in this block.
    #[must_use]
    pub fn frontier(&self, block: BlockId) -> &Set<BlockId> {
        use std::sync::LazyLock;
        static EMPTY: LazyLock<Set<BlockId>> = LazyLock::new(Set::new);
        self.dominance_frontier.get(&block).unwrap_or(&EMPTY)
    }

    /// Get all dominators of a block
    #[must_use]
    pub fn get_dominators(&self, block: BlockId) -> Option<&Set<BlockId>> {
        self.dominators.get(&block)
    }

    /// Get children of a block in the dominator tree
    #[must_use]
    pub fn get_dom_tree_children(&self, block: BlockId) -> Option<&Set<BlockId>> {
        self.dom_tree_children.get(&block)
    }

    /// Check if an allocation site dominates all use sites
    ///
    /// This is a key criterion for reference promotion:
    /// - The allocation must be executed before any use
    /// - All uses must be on paths that pass through the allocation
    ///
    /// # Parameters
    ///
    /// - `allocation_block`: Block where reference is allocated
    /// - `use_blocks`: Blocks where reference is used
    ///
    /// # Returns
    ///
    /// `true` if allocation dominates all uses, `false` otherwise
    #[must_use]
    pub fn allocation_dominates_uses(
        &self,
        allocation_block: BlockId,
        use_blocks: &Set<BlockId>,
    ) -> bool {
        for &use_block in use_blocks {
            if !self.dominates(allocation_block, use_block) {
                return false;
            }
        }
        true
    }

    /// Check dominance for a reference with its definition and uses
    ///
    /// Verifies that the definition site dominates all use sites.
    /// This is the dominance criterion from Spec Section 0.12.1.
    ///
    /// # Parameters
    ///
    /// - `def_site`: Definition site of the reference
    /// - `use_sites`: All use sites of the reference
    ///
    /// # Returns
    ///
    /// `true` if definition dominates all uses
    #[must_use]
    pub fn check_reference_dominance(
        &self,
        def_site: &DefSite,
        use_sites: &[crate::analysis::UseeSite],
    ) -> bool {
        let use_blocks: Set<BlockId> = use_sites.iter().map(|u| u.block).collect();
        self.allocation_dominates_uses(def_site.block, &use_blocks)
    }

    /// Compute the iterated dominance frontier for a set of blocks
    ///
    /// This is used for phi node placement in SSA construction.
    /// IDF(S) = limit of DF^n(S) as n -> infinity
    ///
    /// # Parameters
    ///
    /// - `blocks`: Initial set of blocks
    ///
    /// # Returns
    ///
    /// The iterated dominance frontier
    #[must_use]
    pub fn iterated_dominance_frontier(&self, blocks: &Set<BlockId>) -> Set<BlockId> {
        let mut work_list: VecDeque<BlockId> = blocks.iter().copied().collect();
        let mut result = Set::new();
        let mut processed = Set::new();

        while let Some(block_id) = work_list.pop_front() {
            if processed.contains(&block_id) {
                continue;
            }
            processed.insert(block_id);

            if let Some(frontier) = self.dominance_frontier.get(&block_id) {
                for &df_block in frontier {
                    if result.insert(df_block) {
                        work_list.push_back(df_block);
                    }
                }
            }
        }

        result
    }
}

impl fmt::Display for DominanceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Dominance Analysis ===")?;
        writeln!(f)?;

        writeln!(f, "Immediate Dominators:")?;
        let mut idom_entries: Vec<_> = self.immediate_dom.iter().collect();
        idom_entries.sort_by_key(|(k, _)| k.0);
        for (block, idom) in idom_entries {
            writeln!(f, "  idom(block_{}) = block_{}", block.0, idom.0)?;
        }
        writeln!(f)?;

        writeln!(f, "Dominance Frontiers:")?;
        let mut df_entries: Vec<_> = self.dominance_frontier.iter().collect();
        df_entries.sort_by_key(|(k, _)| k.0);
        for (block, frontier) in df_entries {
            if !frontier.is_empty() {
                let frontier_str: Vec<_> =
                    frontier.iter().map(|b| format!("block_{}", b.0)).collect();
                writeln!(
                    f,
                    "  DF(block_{}) = {{ {} }}",
                    block.0,
                    frontier_str.join(", ")
                )?;
            }
        }

        Ok(())
    }
}

// ==================================================================================
// Reference Info for Promotion Decision (Section 2.5)
// ==================================================================================

/// Information about a reference for promotion decision
///
/// Aggregates all information needed to decide if a reference can be promoted
/// from `&T` (managed, ~15ns) to `&checked T` (0ns).
#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    /// Reference identifier
    pub ref_id: RefId,

    /// Allocation site (definition)
    pub allocation_site: BlockId,

    /// All use sites
    pub use_sites: Set<BlockId>,

    /// Whether the reference is stack-allocated
    pub is_stack_allocated: bool,

    /// Escape category from Phase 2
    pub escape_category: EscapeCategory,
}

impl ReferenceInfo {
    /// Create new reference info
    #[must_use]
    pub fn new(
        ref_id: RefId,
        allocation_site: BlockId,
        is_stack_allocated: bool,
        escape_category: EscapeCategory,
    ) -> Self {
        Self {
            ref_id,
            allocation_site,
            use_sites: Set::new(),
            is_stack_allocated,
            escape_category,
        }
    }

    /// Add a use site
    pub fn add_use_site(&mut self, block: BlockId) {
        self.use_sites.insert(block);
    }

    /// Check if all uses are in a single block
    #[must_use]
    pub fn single_block_usage(&self) -> bool {
        self.use_sites.len() <= 1 && self.use_sites.iter().all(|&b| b == self.allocation_site)
    }
}

/// Escape category for promotion decision
///
/// This is the output from Phase 2 escape analysis, categorizing
/// how a reference might escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EscapeCategory {
    /// Reference proven not to escape - candidate for promotion
    NoEscape,

    /// Reference may escape via some path
    MayEscape,

    /// Reference definitely escapes
    Escapes,

    /// Unknown escape status
    Unknown,
}

impl EscapeCategory {
    /// Check if promotion is potentially allowed (subject to dominance check)
    #[must_use]
    pub fn allows_promotion(&self) -> bool {
        matches!(self, EscapeCategory::NoEscape)
    }
}

// ==================================================================================
// Dominance-Based Promotion Decision (Phase 4 Core)
// ==================================================================================

/// Promotion decision integrating escape analysis and dominance
///
/// This represents the final decision about whether to promote a reference,
/// combining information from all phases:
/// - Phase 1 (SSA): Use-def chains
/// - Phase 2 (Escape): Escape category
/// - Phase 3 (Dominance): Allocation dominates uses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionDecision {
    /// Safe to promote to &checked T (0ns)
    PromoteToChecked,

    /// Keep as managed &T (~15ns) - escape category prevents promotion
    KeepManagedEscape,

    /// Keep as managed &T (~15ns) - dominance check failed
    KeepManagedDominance,

    /// Keep as managed &T (~15ns) - conservative decision
    KeepManagedConservative,
}

impl PromotionDecision {
    /// Check if reference should be promoted
    #[must_use]
    pub fn should_promote(&self) -> bool {
        matches!(self, PromotionDecision::PromoteToChecked)
    }

    /// Get expected overhead in nanoseconds
    #[must_use]
    pub fn overhead_ns(&self) -> u64 {
        match self {
            PromotionDecision::PromoteToChecked => 0,
            _ => 15,
        }
    }

    /// Get reason string
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            PromotionDecision::PromoteToChecked => {
                "NoEscape + dominance verified -> promote to &checked T"
            }
            PromotionDecision::KeepManagedEscape => "May escape -> keep &T (CBGR protection)",
            PromotionDecision::KeepManagedDominance => {
                "Allocation doesn't dominate all uses -> keep &T"
            }
            PromotionDecision::KeepManagedConservative => {
                "Conservative decision -> keep &T (safer)"
            }
        }
    }
}

impl fmt::Display for PromotionDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.reason())
    }
}

/// Make a promotion decision based on escape category and dominance
///
/// This is the core Phase 4 decision logic that combines results from
/// Phase 2 (escape analysis) and Phase 3 (dominance analysis).
///
/// # Algorithm
///
/// ```text
/// if escape_category != NoEscape:
///     return KeepManagedEscape  // May escape, need CBGR
/// if !allocation_dominates_uses:
///     return KeepManagedDominance  // May use before allocation
/// return PromoteToChecked  // Safe to promote
/// ```
///
/// # Parameters
///
/// - `ref_info`: Reference information with escape category
/// - `dominance`: Dominance information
///
/// # Returns
///
/// Promotion decision for the reference
#[must_use]
pub fn decide_promotion(ref_info: &ReferenceInfo, dominance: &DominanceInfo) -> PromotionDecision {
    // Step 1: Check escape category from Phase 2
    if !ref_info.escape_category.allows_promotion() {
        return PromotionDecision::KeepManagedEscape;
    }

    // Step 2: Check dominance from Phase 3
    let alloc_dominates_uses =
        dominance.allocation_dominates_uses(ref_info.allocation_site, &ref_info.use_sites);

    if !alloc_dominates_uses {
        // Even if NoEscape, if allocation doesn't dominate uses,
        // the reference might be used before it's allocated on some path
        return PromotionDecision::KeepManagedDominance;
    }

    // Step 3: Additional safety checks (conservative)
    if !ref_info.is_stack_allocated {
        // Heap-allocated references need more care
        // Could be refined with more analysis
        return PromotionDecision::KeepManagedConservative;
    }

    // All checks passed - safe to promote
    PromotionDecision::PromoteToChecked
}

// ==================================================================================
// Statistics and Reporting
// ==================================================================================

/// Statistics from dominance analysis
#[derive(Debug, Clone, Default)]
pub struct DominanceStats {
    /// Number of blocks analyzed
    pub blocks_analyzed: usize,

    /// Number of dominance edges
    pub dominance_edges: usize,

    /// Size of largest dominance frontier
    pub max_frontier_size: usize,

    /// Number of references checked for dominance
    pub references_checked: usize,

    /// Number of references where allocation dominates all uses
    pub allocation_dominates_count: usize,

    /// Number of promotion decisions made
    pub promotions_decided: usize,

    /// Number of references promoted
    pub references_promoted: usize,
}

impl DominanceStats {
    /// Calculate promotion rate
    #[must_use]
    pub fn promotion_rate(&self) -> f64 {
        if self.promotions_decided == 0 {
            0.0
        } else {
            (self.references_promoted as f64 / self.promotions_decided as f64) * 100.0
        }
    }
}

impl fmt::Display for DominanceStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Dominance Analysis Statistics:")?;
        writeln!(f, "  Blocks analyzed:      {}", self.blocks_analyzed)?;
        writeln!(f, "  Dominance edges:      {}", self.dominance_edges)?;
        writeln!(f, "  Max frontier size:    {}", self.max_frontier_size)?;
        writeln!(f, "  References checked:   {}", self.references_checked)?;
        writeln!(
            f,
            "  Allocation dominates: {}",
            self.allocation_dominates_count
        )?;
        writeln!(f, "  Promotions decided:   {}", self.promotions_decided)?;
        writeln!(
            f,
            "  References promoted:  {} ({:.1}%)",
            self.references_promoted,
            self.promotion_rate()
        )?;
        Ok(())
    }
}

// ==================================================================================
// Tests
// ==================================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::BasicBlock;

    fn create_simple_cfg() -> ControlFlowGraph {
        // Create a simple diamond CFG:
        //     Entry (0)
        //       |
        //     Block1 (1)
        //      / \
        //   B2(2) B3(3)
        //      \ /
        //     Exit (4)

        let entry = BlockId(0);
        let block1 = BlockId(1);
        let block2 = BlockId(2);
        let block3 = BlockId(3);
        let exit = BlockId(4);

        let mut cfg = ControlFlowGraph::new(entry, exit);

        // Entry block
        let mut entry_block = BasicBlock {
            id: entry,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        entry_block.successors.insert(block1);

        // Block 1 (branch point)
        let mut b1 = BasicBlock {
            id: block1,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        b1.predecessors.insert(entry);
        b1.successors.insert(block2);
        b1.successors.insert(block3);

        // Block 2 (left branch)
        let mut b2 = BasicBlock {
            id: block2,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        b2.predecessors.insert(block1);
        b2.successors.insert(exit);

        // Block 3 (right branch)
        let mut b3 = BasicBlock {
            id: block3,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        b3.predecessors.insert(block1);
        b3.successors.insert(exit);

        // Exit block
        let mut exit_block = BasicBlock {
            id: exit,
            predecessors: Set::new(),
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };
        exit_block.predecessors.insert(block2);
        exit_block.predecessors.insert(block3);

        cfg.add_block(entry_block);
        cfg.add_block(b1);
        cfg.add_block(b2);
        cfg.add_block(b3);
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_dominance_computation() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Entry dominates everything
        assert!(dom_info.dominates(BlockId(0), BlockId(0)));
        assert!(dom_info.dominates(BlockId(0), BlockId(1)));
        assert!(dom_info.dominates(BlockId(0), BlockId(2)));
        assert!(dom_info.dominates(BlockId(0), BlockId(3)));
        assert!(dom_info.dominates(BlockId(0), BlockId(4)));

        // Block 1 dominates blocks 2, 3, 4
        assert!(dom_info.dominates(BlockId(1), BlockId(2)));
        assert!(dom_info.dominates(BlockId(1), BlockId(3)));
        assert!(dom_info.dominates(BlockId(1), BlockId(4)));

        // Block 2 doesn't dominate block 3 (different branches)
        assert!(!dom_info.dominates(BlockId(2), BlockId(3)));

        // Block 3 doesn't dominate block 2
        assert!(!dom_info.dominates(BlockId(3), BlockId(2)));
    }

    #[test]
    fn test_immediate_dominators() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // idom(block_1) = entry
        assert_eq!(
            dom_info.get_immediate_dominator(BlockId(1)),
            Some(BlockId(0))
        );

        // idom(block_2) = block_1
        assert_eq!(
            dom_info.get_immediate_dominator(BlockId(2)),
            Some(BlockId(1))
        );

        // idom(block_3) = block_1
        assert_eq!(
            dom_info.get_immediate_dominator(BlockId(3)),
            Some(BlockId(1))
        );

        // Entry has no immediate dominator
        assert_eq!(dom_info.get_immediate_dominator(BlockId(0)), None);
    }

    #[test]
    fn test_dominance_frontier() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Block 2's frontier should include exit (join point)
        let df2 = dom_info.frontier(BlockId(2));
        assert!(df2.contains(&BlockId(4)));

        // Block 3's frontier should include exit
        let df3 = dom_info.frontier(BlockId(3));
        assert!(df3.contains(&BlockId(4)));
    }

    #[test]
    fn test_allocation_dominates_uses() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Allocation in block 1, uses in blocks 2 and 3
        let mut use_sites = Set::new();
        use_sites.insert(BlockId(2));
        use_sites.insert(BlockId(3));

        assert!(dom_info.allocation_dominates_uses(BlockId(1), &use_sites));

        // Allocation in block 2, use in block 3 - should fail
        let mut use_sites2 = Set::new();
        use_sites2.insert(BlockId(3));

        assert!(!dom_info.allocation_dominates_uses(BlockId(2), &use_sites2));
    }

    #[test]
    fn test_promotion_decision_no_escape_dominates() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Reference allocated in block 1, used in blocks 2 and 3, NoEscape
        let mut ref_info = ReferenceInfo::new(
            RefId(1),
            BlockId(1),
            true, // stack allocated
            EscapeCategory::NoEscape,
        );
        ref_info.add_use_site(BlockId(2));
        ref_info.add_use_site(BlockId(3));

        let decision = decide_promotion(&ref_info, &dom_info);
        assert_eq!(decision, PromotionDecision::PromoteToChecked);
    }

    #[test]
    fn test_promotion_decision_escapes() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Reference that escapes - should not promote regardless of dominance
        let mut ref_info = ReferenceInfo::new(
            RefId(2),
            BlockId(1),
            true,
            EscapeCategory::MayEscape, // escapes!
        );
        ref_info.add_use_site(BlockId(2));

        let decision = decide_promotion(&ref_info, &dom_info);
        assert_eq!(decision, PromotionDecision::KeepManagedEscape);
    }

    #[test]
    fn test_promotion_decision_not_dominates() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        // Reference allocated in block 2, used in block 3 - doesn't dominate
        let mut ref_info = ReferenceInfo::new(RefId(3), BlockId(2), true, EscapeCategory::NoEscape);
        ref_info.add_use_site(BlockId(3));

        let decision = decide_promotion(&ref_info, &dom_info);
        assert_eq!(decision, PromotionDecision::KeepManagedDominance);
    }

    #[test]
    fn test_iterated_dominance_frontier() {
        let cfg = create_simple_cfg();
        let dom_info = DominanceInfo::compute(&cfg);

        let mut initial = Set::new();
        initial.insert(BlockId(2));
        initial.insert(BlockId(3));

        let idf = dom_info.iterated_dominance_frontier(&initial);

        // IDF should include the exit block (join point)
        assert!(idf.contains(&BlockId(4)));
    }
}
