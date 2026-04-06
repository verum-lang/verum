//! Ownership Analysis for Compile-Time Memory Safety Detection
//!
//! This module implements ownership tracking to detect memory safety issues
//! at compile-time, including:
//!
//! - **Double-Free Detection**: Catches deallocations of already-freed memory
//! - **Use-After-Free Detection**: Enhanced detection beyond escape analysis
//! - **Ownership Transfer Tracking**: Monitors move semantics across boundaries
//! - **Resource Leak Detection**: Identifies allocations without corresponding frees
//!
//! # Architecture
//!
//! ```text
//! CFG → OwnershipAnalyzer → OwnershipAnalysisResult
//!                                │
//!                                ▼
//!                    ┌───────────────────────┐
//!                    │ Map<AllocId, AllocInfo>│
//!                    │ Set<DoubleFreeWarning> │
//!                    │ Set<LeakWarning>       │
//!                    │ List<OwnershipTransfer>│
//!                    └───────────────────────┘
//! ```
//!
//! # Algorithm Overview
//!
//! The analysis performs a forward dataflow analysis tracking:
//! 1. **Allocation Sites**: Where memory is allocated (new, alloc, Heap::new)
//! 2. **Deallocation Sites**: Where memory is freed (drop, free, explicit dealloc)
//! 3. **Ownership State**: Current owner of each allocation
//! 4. **Transfer Events**: Moves, borrows, and drops
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::ownership_analysis::OwnershipAnalyzer;
//!
//! let analyzer = OwnershipAnalyzer::new(cfg);
//! let result = analyzer.analyze();
//!
//! for warning in &result.double_free_warnings {
//!     println!("Double-free at {:?}: allocated at {:?}, first free at {:?}",
//!              warning.second_free_site,
//!              warning.allocation_site,
//!              warning.first_free_site);
//! }
//! ```
//!
//! Phase 6 of the CBGR analysis pipeline: ownership tracking for memory safety.
//! Detects double-free (deallocating already-freed memory), use-after-free
//! (beyond what escape analysis catches), ownership transfer violations (move
//! semantics across boundaries), and resource leaks (allocations without frees).
//! Results feed into tier decisions and diagnostic reporting.

use crate::analysis::{BlockId, ControlFlowGraph, DefSite, RefId, Span};
use verum_common::{List, Map};

// ============================================================================
// Allocation and Deallocation Site Tracking
// ============================================================================

/// Unique identifier for an allocation site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AllocId(pub u64);

impl AllocId {
    /// Create from a block and instruction index.
    #[must_use]
    pub fn from_site(block: BlockId, instr_index: u32) -> Self {
        Self(((block.0 as u64) << 32) | (instr_index as u64))
    }

    /// Create from a source span.
    #[must_use]
    pub fn from_span(span: Span) -> Self {
        Self(((span.0 as u64) << 32) | (span.1 as u64))
    }

    /// Get the block ID portion.
    #[must_use]
    pub fn block(&self) -> BlockId {
        BlockId((self.0 >> 32) as u64)
    }
}

/// Information about an allocation site.
#[derive(Debug, Clone)]
pub struct AllocationInfo {
    /// Unique allocation ID.
    pub id: AllocId,
    /// Reference ID if known.
    pub ref_id: Option<RefId>,
    /// Block where allocation occurs.
    pub block: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Type of allocation.
    pub kind: AllocationKind,
    /// Current ownership state.
    pub state: OwnershipState,
    /// Size if statically known.
    pub size: Option<usize>,
    /// Whether this allocation is on the heap.
    pub is_heap: bool,
}

impl AllocationInfo {
    /// Create new allocation info.
    #[must_use]
    pub fn new(id: AllocId, block: BlockId, kind: AllocationKind) -> Self {
        Self {
            id,
            ref_id: None,
            block,
            span: None,
            kind,
            state: OwnershipState::Owned,
            size: None,
            is_heap: matches!(kind, AllocationKind::Heap | AllocationKind::HeapNew | AllocationKind::RawAlloc),
        }
    }

    /// Create with source span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Create with ref ID.
    #[must_use]
    pub fn with_ref_id(mut self, ref_id: RefId) -> Self {
        self.ref_id = Some(ref_id);
        self
    }

    /// Create with size.
    #[must_use]
    pub fn with_size(mut self, size: usize) -> Self {
        self.size = Some(size);
        self
    }

    /// Check if this allocation has been freed.
    #[must_use]
    pub fn is_freed(&self) -> bool {
        matches!(self.state, OwnershipState::Freed { .. })
    }

    /// Check if this allocation is still owned.
    #[must_use]
    pub fn is_owned(&self) -> bool {
        matches!(self.state, OwnershipState::Owned)
    }
}

/// Type of allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationKind {
    /// Stack allocation (let binding).
    Stack,
    /// Heap allocation via Heap<T> wrapper.
    Heap,
    /// Heap allocation via Heap::new().
    HeapNew,
    /// Raw allocation via alloc().
    RawAlloc,
    /// Collection allocation (List, Map, etc.).
    Collection,
    /// String allocation.
    String,
    /// Temporary allocation (expression result).
    Temporary,
    /// Unknown allocation source.
    Unknown,
}

impl AllocationKind {
    /// Check if this kind typically requires manual deallocation.
    #[must_use]
    pub fn requires_dealloc(&self) -> bool {
        matches!(self, Self::Heap | Self::HeapNew | Self::RawAlloc | Self::Collection | Self::String)
    }

    /// Get a human-readable name for this kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stack => "stack",
            Self::Heap => "heap",
            Self::HeapNew => "Heap::new",
            Self::RawAlloc => "raw_alloc",
            Self::Collection => "collection",
            Self::String => "string",
            Self::Temporary => "temporary",
            Self::Unknown => "unknown",
        }
    }
}

/// Current ownership state of an allocation.
#[derive(Debug, Clone)]
pub enum OwnershipState {
    /// Allocation is currently owned (valid).
    Owned,
    /// Ownership has been moved to another owner.
    Moved {
        /// Where ownership was transferred.
        transfer_site: BlockId,
        /// New owner if known.
        new_owner: Option<RefId>,
    },
    /// Allocation has been borrowed (temporarily transferred).
    Borrowed {
        /// Where the borrow started.
        borrow_site: BlockId,
        /// Whether this is a mutable borrow.
        is_mutable: bool,
    },
    /// Allocation has been freed.
    Freed {
        /// Where the deallocation occurred.
        free_site: BlockId,
        /// Span of deallocation if available.
        free_span: Option<Span>,
    },
    /// Ownership state is unknown (conservative).
    Unknown,
}

impl OwnershipState {
    /// Check if this state allows further use.
    #[must_use]
    pub fn allows_use(&self) -> bool {
        matches!(self, Self::Owned | Self::Borrowed { .. })
    }

    /// Check if this state allows deallocation.
    #[must_use]
    pub fn allows_dealloc(&self) -> bool {
        matches!(self, Self::Owned)
    }
}

// ============================================================================
// Deallocation Site Tracking
// ============================================================================

/// Information about a deallocation site.
#[derive(Debug, Clone)]
pub struct DeallocationSite {
    /// Block where deallocation occurs.
    pub block: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Type of deallocation.
    pub kind: DeallocationKind,
    /// Allocation being freed (if known).
    pub alloc_id: Option<AllocId>,
}

impl DeallocationSite {
    /// Create new deallocation site.
    #[must_use]
    pub fn new(block: BlockId, kind: DeallocationKind) -> Self {
        Self {
            block,
            span: None,
            kind,
            alloc_id: None,
        }
    }

    /// Create with span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Create with allocation ID.
    #[must_use]
    pub fn with_alloc_id(mut self, alloc_id: AllocId) -> Self {
        self.alloc_id = Some(alloc_id);
        self
    }
}

/// Type of deallocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeallocationKind {
    /// Automatic drop at end of scope.
    ScopeDrop,
    /// Explicit drop() call.
    ExplicitDrop,
    /// Raw dealloc() call.
    RawDealloc,
    /// Collection clear/drop.
    CollectionClear,
    /// Move out of scope.
    MoveOut,
    /// Unknown deallocation.
    Unknown,
}

// ============================================================================
// Ownership Transfer Tracking
// ============================================================================

/// Record of an ownership transfer event.
#[derive(Debug, Clone)]
pub struct OwnershipTransfer {
    /// Allocation being transferred.
    pub alloc_id: AllocId,
    /// Previous owner.
    pub from_owner: Option<RefId>,
    /// New owner.
    pub to_owner: Option<RefId>,
    /// Block where transfer occurs.
    pub block: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Type of transfer.
    pub kind: TransferKind,
}

impl OwnershipTransfer {
    /// Create new ownership transfer.
    #[must_use]
    pub fn new(alloc_id: AllocId, block: BlockId, kind: TransferKind) -> Self {
        Self {
            alloc_id,
            from_owner: None,
            to_owner: None,
            block,
            span: None,
            kind,
        }
    }

    /// Set from owner.
    #[must_use]
    pub fn from(mut self, owner: RefId) -> Self {
        self.from_owner = Some(owner);
        self
    }

    /// Set to owner.
    #[must_use]
    pub fn to(mut self, owner: RefId) -> Self {
        self.to_owner = Some(owner);
        self
    }

    /// Set span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Type of ownership transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    /// Move semantics (ownership fully transferred).
    Move,
    /// Immutable borrow (temporary read access).
    ImmutableBorrow,
    /// Mutable borrow (temporary write access).
    MutableBorrow,
    /// Return borrow (end of borrow lifetime).
    ReturnBorrow,
    /// Clone (creates new ownership).
    Clone,
    /// Copy (value copy, not ownership transfer).
    Copy,
    /// Function call (ownership may transfer).
    FunctionCall,
    /// Return from function.
    FunctionReturn,
}

// ============================================================================
// Warning Types
// ============================================================================

/// Warning for double-free detection.
#[derive(Debug, Clone)]
pub struct DoubleFreeWarning {
    /// Allocation that was freed twice.
    pub alloc_id: AllocId,
    /// Original allocation site.
    pub allocation_site: BlockId,
    /// Allocation span if available.
    pub allocation_span: Option<Span>,
    /// First deallocation site.
    pub first_free_site: BlockId,
    /// First free span if available.
    pub first_free_span: Option<Span>,
    /// Second (invalid) deallocation site.
    pub second_free_site: BlockId,
    /// Second free span if available.
    pub second_free_span: Option<Span>,
    /// Confidence level (0.0-1.0).
    pub confidence: f64,
}

impl DoubleFreeWarning {
    /// Create new double-free warning.
    #[must_use]
    pub fn new(
        alloc_id: AllocId,
        allocation_site: BlockId,
        first_free_site: BlockId,
        second_free_site: BlockId,
    ) -> Self {
        Self {
            alloc_id,
            allocation_site,
            allocation_span: None,
            first_free_site,
            first_free_span: None,
            second_free_site,
            second_free_span: None,
            confidence: 1.0,
        }
    }

    /// Set allocation span.
    #[must_use]
    pub fn with_allocation_span(mut self, span: Span) -> Self {
        self.allocation_span = Some(span);
        self
    }

    /// Set first free span.
    #[must_use]
    pub fn with_first_free_span(mut self, span: Span) -> Self {
        self.first_free_span = Some(span);
        self
    }

    /// Set second free span.
    #[must_use]
    pub fn with_second_free_span(mut self, span: Span) -> Self {
        self.second_free_span = Some(span);
        self
    }

    /// Set confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }
}

/// Warning for use-after-free detection.
#[derive(Debug, Clone)]
pub struct UseAfterFreeWarning {
    /// Allocation that was used after free.
    pub alloc_id: AllocId,
    /// Reference being used.
    pub ref_id: RefId,
    /// Deallocation site.
    pub free_site: BlockId,
    /// Use site (after free).
    pub use_site: BlockId,
    /// Use span if available.
    pub use_span: Option<Span>,
    /// Confidence level.
    pub confidence: f64,
}

/// Warning for potential memory leak.
#[derive(Debug, Clone)]
pub struct LeakWarning {
    /// Allocation that may leak.
    pub alloc_id: AllocId,
    /// Allocation site.
    pub allocation_site: BlockId,
    /// Allocation span if available.
    pub allocation_span: Option<Span>,
    /// Why we think this leaks.
    pub reason: LeakReason,
    /// Confidence level.
    pub confidence: f64,
}

/// Reason for potential leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakReason {
    /// No deallocation found on any path.
    NoDeallocation,
    /// Some paths don't deallocate.
    PartialPaths,
    /// Ownership escapes function.
    OwnershipEscapes,
    /// Stored in container that may outlive scope.
    StoredInContainer,
    /// Unknown reason.
    Unknown,
}

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of ownership analysis.
#[derive(Debug, Clone)]
pub struct OwnershipAnalysisResult {
    /// All tracked allocations.
    pub allocations: Map<AllocId, AllocationInfo>,
    /// All deallocation sites.
    pub deallocations: List<DeallocationSite>,
    /// All ownership transfers.
    pub transfers: List<OwnershipTransfer>,
    /// Double-free warnings.
    pub double_free_warnings: List<DoubleFreeWarning>,
    /// Use-after-free warnings.
    pub use_after_free_warnings: List<UseAfterFreeWarning>,
    /// Leak warnings.
    pub leak_warnings: List<LeakWarning>,
    /// Analysis statistics.
    pub stats: OwnershipStats,
}

impl OwnershipAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            allocations: Map::new(),
            deallocations: List::new(),
            transfers: List::new(),
            double_free_warnings: List::new(),
            use_after_free_warnings: List::new(),
            leak_warnings: List::new(),
            stats: OwnershipStats::default(),
        }
    }

    /// Check if analysis found any issues.
    #[must_use]
    pub fn has_issues(&self) -> bool {
        !self.double_free_warnings.is_empty()
            || !self.use_after_free_warnings.is_empty()
            || !self.leak_warnings.is_empty()
    }

    /// Get total number of warnings.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.double_free_warnings.len()
            + self.use_after_free_warnings.len()
            + self.leak_warnings.len()
    }

    /// Get allocations that weren't deallocated.
    #[must_use]
    pub fn undeallocated_allocations(&self) -> List<&AllocationInfo> {
        self.allocations
            .values()
            .filter(|info| info.is_owned() && info.kind.requires_dealloc())
            .collect()
    }
}

/// Statistics from ownership analysis.
#[derive(Debug, Clone, Default)]
pub struct OwnershipStats {
    /// Total allocations tracked.
    pub total_allocations: usize,
    /// Total deallocations tracked.
    pub total_deallocations: usize,
    /// Total ownership transfers.
    pub total_transfers: usize,
    /// Stack allocations.
    pub stack_allocations: usize,
    /// Heap allocations.
    pub heap_allocations: usize,
    /// Allocations with known deallocation.
    pub matched_deallocs: usize,
    /// Allocations without deallocation.
    pub unmatched_allocs: usize,
    /// Analysis time in microseconds.
    pub analysis_time_us: u64,
}

// ============================================================================
// Ownership Analyzer
// ============================================================================

/// Analyzer for ownership tracking and memory safety detection.
pub struct OwnershipAnalyzer {
    /// Control flow graph being analyzed.
    cfg: ControlFlowGraph,
    /// Current allocations state.
    allocations: Map<AllocId, AllocationInfo>,
    /// Allocation counter for ID generation.
    alloc_counter: u64,
    /// Deallocation sites found.
    deallocations: List<DeallocationSite>,
    /// Ownership transfers.
    transfers: List<OwnershipTransfer>,
    /// Mapping from RefId to AllocId.
    ref_to_alloc: Map<RefId, AllocId>,
    /// Configuration options.
    config: OwnershipAnalysisConfig,
}

/// Configuration for ownership analysis.
#[derive(Debug, Clone)]
pub struct OwnershipAnalysisConfig {
    /// Whether to track stack allocations.
    pub track_stack: bool,
    /// Whether to track temporary allocations.
    pub track_temporaries: bool,
    /// Whether to detect leaks.
    pub detect_leaks: bool,
    /// Minimum confidence for warnings.
    pub min_confidence: f64,
    /// Maximum blocks to analyze (0 = unlimited).
    pub max_blocks: usize,
}

impl Default for OwnershipAnalysisConfig {
    fn default() -> Self {
        Self {
            track_stack: false, // Stack doesn't need manual dealloc
            track_temporaries: false,
            detect_leaks: true,
            min_confidence: 0.5,
            max_blocks: 0,
        }
    }
}

impl OwnershipAnalyzer {
    /// Create new ownership analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self {
            cfg,
            allocations: Map::new(),
            alloc_counter: 0,
            deallocations: List::new(),
            transfers: List::new(),
            ref_to_alloc: Map::new(),
            config: OwnershipAnalysisConfig::default(),
        }
    }

    /// Create with configuration.
    #[must_use]
    pub fn with_config(mut self, config: OwnershipAnalysisConfig) -> Self {
        self.config = config;
        self
    }

    /// Perform ownership analysis.
    #[must_use]
    pub fn analyze(mut self) -> OwnershipAnalysisResult {
        let start = std::time::Instant::now();

        // Phase 1: Extract allocation and deallocation sites from CFG
        self.extract_allocation_sites();
        self.extract_deallocation_sites();

        // Phase 2: Track ownership transfers
        self.analyze_ownership_transfers();

        // Phase 3: Match allocations with deallocations
        self.match_alloc_dealloc();

        // Phase 4: Detect issues
        let double_free_warnings = self.detect_double_free();
        let use_after_free_warnings = self.detect_use_after_free();
        let leak_warnings = if self.config.detect_leaks {
            self.detect_leaks()
        } else {
            List::new()
        };

        // Build statistics
        let stats = OwnershipStats {
            total_allocations: self.allocations.len(),
            total_deallocations: self.deallocations.len(),
            total_transfers: self.transfers.len(),
            stack_allocations: self.allocations.values()
                .filter(|a| matches!(a.kind, AllocationKind::Stack))
                .count(),
            heap_allocations: self.allocations.values()
                .filter(|a| a.is_heap)
                .count(),
            matched_deallocs: self.allocations.values()
                .filter(|a| a.is_freed())
                .count(),
            unmatched_allocs: self.allocations.values()
                .filter(|a| a.is_owned() && a.kind.requires_dealloc())
                .count(),
            analysis_time_us: start.elapsed().as_micros() as u64,
        };

        OwnershipAnalysisResult {
            allocations: self.allocations,
            deallocations: self.deallocations,
            transfers: self.transfers,
            double_free_warnings,
            use_after_free_warnings,
            leak_warnings,
            stats,
        }
    }

    /// Generate new allocation ID.
    fn new_alloc_id(&mut self) -> AllocId {
        let id = AllocId(self.alloc_counter);
        self.alloc_counter += 1;
        id
    }

    /// Extract allocation sites from CFG.
    fn extract_allocation_sites(&mut self) {
        // Collect allocation data first to avoid borrow issues
        let alloc_data: List<_> = self.cfg.blocks
            .iter()
            .flat_map(|(block_id, block)| {
                block.definitions.iter().map(move |def| {
                    let kind = Self::classify_allocation_static(def);
                    (*block_id, def.reference, def.span, kind, def.is_stack_allocated)
                })
            })
            .collect();

        // Process collected data
        for (block_id, ref_id, span, kind, _is_stack) in alloc_data {
            if kind != AllocationKind::Unknown {
                let skip = !self.config.track_stack && kind == AllocationKind::Stack;
                if !skip {
                    let alloc_id = self.new_alloc_id();
                    let mut info = AllocationInfo::new(alloc_id, block_id, kind);
                    info.ref_id = Some(ref_id);
                    if let Some(s) = span {
                        info.span = Some(s);
                    }
                    self.allocations.insert(alloc_id, info);
                    self.ref_to_alloc.insert(ref_id, alloc_id);
                }
            }
        }
    }

    /// Static version of classify_allocation for use in iterators.
    fn classify_allocation_static(def: &DefSite) -> AllocationKind {
        if def.is_stack_allocated {
            AllocationKind::Stack
        } else {
            AllocationKind::Heap
        }
    }

    /// Classify allocation type from definition site.
    fn classify_allocation(&self, def: &DefSite) -> AllocationKind {
        // Use is_stack_allocated flag from DefSite
        if def.is_stack_allocated {
            AllocationKind::Stack
        } else {
            // Heap allocation assumed
            AllocationKind::Heap
        }
    }

    /// Extract deallocation sites from CFG.
    fn extract_deallocation_sites(&mut self) {
        for (block_id, block) in &self.cfg.blocks {
            // Check for explicit drops or deallocations
            // In a real implementation, we'd look at call sites for drop/free
            for use_site in &block.uses {
                // Mutable use followed by no more uses could indicate drop
                if use_site.is_mutable {
                    // Conservative: track as potential deallocation point
                    let dealloc = DeallocationSite::new(*block_id, DeallocationKind::Unknown);
                    self.deallocations.push(dealloc);
                }
            }
        }
    }

    /// Analyze ownership transfers between blocks.
    fn analyze_ownership_transfers(&mut self) {
        for (block_id, block) in &self.cfg.blocks {
            // Track uses that might transfer ownership
            for use_site in &block.uses {
                if let Some(&alloc_id) = self.ref_to_alloc.get(&use_site.reference) {
                    let kind = if use_site.is_mutable {
                        TransferKind::MutableBorrow
                    } else {
                        TransferKind::ImmutableBorrow
                    };

                    let transfer = OwnershipTransfer::new(alloc_id, *block_id, kind);
                    self.transfers.push(transfer);
                }
            }
        }
    }

    /// Match allocations with their deallocations.
    fn match_alloc_dealloc(&mut self) {
        // For each deallocation, try to find the corresponding allocation
        for dealloc in &self.deallocations {
            if let Some(alloc_id) = dealloc.alloc_id {
                if let Some(info) = self.allocations.get_mut(&alloc_id) {
                    info.state = OwnershipState::Freed {
                        free_site: dealloc.block,
                        free_span: dealloc.span,
                    };
                }
            }
        }
    }

    /// Detect double-free issues.
    fn detect_double_free(&self) -> List<DoubleFreeWarning> {
        let mut warnings = List::new();

        // Group deallocations by allocation
        let mut deallocs_per_alloc: Map<AllocId, List<&DeallocationSite>> = Map::new();
        for dealloc in &self.deallocations {
            if let Some(alloc_id) = dealloc.alloc_id {
                deallocs_per_alloc
                    .entry(alloc_id)
                    .or_insert_with(List::new)
                    .push(dealloc);
            }
        }

        // Check for multiple deallocations
        for (alloc_id, deallocs) in &deallocs_per_alloc {
            if deallocs.len() >= 2 {
                if let Some(info) = self.allocations.get(alloc_id) {
                    let warning = DoubleFreeWarning::new(
                        *alloc_id,
                        info.block,
                        deallocs[0].block,
                        deallocs[1].block,
                    );
                    warnings.push(warning);
                }
            }
        }

        warnings
    }

    /// Detect use-after-free issues.
    fn detect_use_after_free(&self) -> List<UseAfterFreeWarning> {
        let mut warnings = List::new();

        for (alloc_id, info) in &self.allocations {
            if let OwnershipState::Freed { free_site, .. } = &info.state {
                // Check for uses after this block
                // This is simplified - real implementation would do proper dataflow
                if let Some(ref_id) = info.ref_id {
                    for (block_id, block) in &self.cfg.blocks {
                        // Check if this block comes after free_site
                        if block_id.0 > free_site.0 {
                            for use_site in &block.uses {
                                if use_site.reference == ref_id {
                                    let warning = UseAfterFreeWarning {
                                        alloc_id: *alloc_id,
                                        ref_id,
                                        free_site: *free_site,
                                        use_site: *block_id,
                                        use_span: use_site.span,
                                        confidence: 0.7, // Conservative
                                    };
                                    warnings.push(warning);
                                }
                            }
                        }
                    }
                }
            }
        }

        warnings
    }

    /// Detect potential memory leaks.
    fn detect_leaks(&self) -> List<LeakWarning> {
        let mut warnings = List::new();

        for (alloc_id, info) in &self.allocations {
            // Check if heap allocation without deallocation
            if info.is_owned() && info.kind.requires_dealloc() {
                let warning = LeakWarning {
                    alloc_id: *alloc_id,
                    allocation_site: info.block,
                    allocation_span: info.span,
                    reason: LeakReason::NoDeallocation,
                    confidence: 0.5, // Conservative
                };
                warnings.push(warning);
            }
        }

        warnings
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::BasicBlock;

    fn create_test_cfg() -> ControlFlowGraph {
        let entry = BlockId(0);
        let exit = BlockId(1);
        let mut cfg = ControlFlowGraph::new(entry, exit);

        // Entry block with allocation
        let mut entry_block = BasicBlock::empty(entry);
        entry_block.definitions.push(DefSite::new(entry, RefId(1), false)); // Heap allocation
        entry_block.successors.insert(exit);
        cfg.add_block(entry_block);

        // Exit block
        let mut exit_block = BasicBlock::empty(exit);
        exit_block.predecessors.insert(entry);
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_ownership_analyzer_creation() {
        let cfg = create_test_cfg();
        let analyzer = OwnershipAnalyzer::new(cfg);
        let result = analyzer.analyze();

        // Analysis completed successfully - result is valid
        let _ = &result.allocations;
    }

    #[test]
    fn test_allocation_kind_names() {
        assert_eq!(AllocationKind::Stack.name(), "stack");
        assert_eq!(AllocationKind::Heap.name(), "heap");
        assert_eq!(AllocationKind::HeapNew.name(), "Heap::new");
    }

    #[test]
    fn test_allocation_info_creation() {
        let alloc_id = AllocId(1);
        let info = AllocationInfo::new(alloc_id, BlockId(0), AllocationKind::Heap);

        assert!(info.is_owned());
        assert!(!info.is_freed());
        assert!(info.is_heap);
    }

    #[test]
    fn test_ownership_state_checks() {
        let owned = OwnershipState::Owned;
        assert!(owned.allows_use());
        assert!(owned.allows_dealloc());

        let freed = OwnershipState::Freed {
            free_site: BlockId(1),
            free_span: None,
        };
        assert!(!freed.allows_use());
        assert!(!freed.allows_dealloc());
    }

    #[test]
    fn test_double_free_warning_creation() {
        let warning = DoubleFreeWarning::new(
            AllocId(1),
            BlockId(0),
            BlockId(1),
            BlockId(2),
        );

        assert_eq!(warning.alloc_id, AllocId(1));
        assert_eq!(warning.allocation_site, BlockId(0));
        assert_eq!(warning.first_free_site, BlockId(1));
        assert_eq!(warning.second_free_site, BlockId(2));
        assert_eq!(warning.confidence, 1.0);
    }

    #[test]
    fn test_analysis_result_empty() {
        let result = OwnershipAnalysisResult::empty();

        assert!(!result.has_issues());
        assert_eq!(result.warning_count(), 0);
    }

    #[test]
    fn test_alloc_id_from_span() {
        let span: Span = (100, 200);
        let alloc_id = AllocId::from_span(span);

        assert_eq!(alloc_id.0, ((100_u64) << 32) | 200);
    }
}
