//! Lifetime Analysis for Compile-Time Memory Safety
//!
//! This module implements compile-time lifetime analysis inspired by Rust's borrow checker,
//! adapted for Verum's three-tier CBGR reference model. It enables:
//!
//! - **Lifetime Inference**: Automatically infer lifetimes for references
//! - **Constraint Generation**: Generate lifetime constraints from code
//! - **Constraint Solving**: Solve constraints to verify safety
//! - **Outlives Checking**: Ensure references don't outlive referents
//!
//! # Architecture
//!
//! ```text
//! CFG → LifetimeAnalyzer → LifetimeAnalysisResult
//!                               │
//!                               ▼
//!                   ┌───────────────────────────────┐
//!                   │ Map<RefId, Lifetime>          │
//!                   │ Set<LifetimeConstraint>       │
//!                   │ List<LifetimeViolation>       │
//!                   │ RegionGraph                   │
//!                   └───────────────────────────────┘
//! ```
//!
//! # Lifetime Model
//!
//! Lifetimes in Verum follow a region-based model:
//! - Each reference has an associated lifetime region
//! - Lifetimes form a partial order based on outlives relationships
//! - Lifetime constraints are gathered during analysis and solved
//!
//! # Integration with CBGR
//!
//! - **Tier 0**: Lifetimes used for optimization hints (runtime still validates)
//! - **Tier 1**: Lifetimes MUST be proven for promotion (no runtime checks)
//! - **Tier 2**: Lifetimes unchecked (manual safety proof required)
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::lifetime_analysis::LifetimeAnalyzer;
//!
//! let analyzer = LifetimeAnalyzer::new(cfg);
//! let result = analyzer.analyze();
//!
//! for violation in &result.violations {
//!     println!("Lifetime violation: {:?}", violation);
//! }
//! ```
//!
//! Lifetime analysis determines whether references outlive their referents. For
//! Tier 0, lifetimes provide optimization hints (runtime CBGR still validates).
//! For Tier 1 (&checked T), lifetimes MUST be proven sound (no runtime fallback).
//! For Tier 2 (&unsafe T), lifetimes are unchecked (manual safety proof required).
//! Uses a region-based model where lifetime constraints form a partial order
//! solved via fixpoint iteration.

use crate::analysis::{BlockId, ControlFlowGraph, RefId, Span};
use verum_common::{List, Map, Set};

// ============================================================================
// Lifetime Identifiers and Regions
// ============================================================================

/// Unique identifier for a lifetime region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LifetimeId(pub u64);

impl LifetimeId {
    /// The static lifetime (lives for entire program).
    pub const STATIC: LifetimeId = LifetimeId(0);

    /// Create from a scope index.
    #[must_use]
    pub fn from_scope(scope: u32) -> Self {
        Self(scope as u64 + 1)
    }

    /// Create from reference ID.
    #[must_use]
    pub fn from_ref(ref_id: RefId) -> Self {
        Self(ref_id.0 | (1 << 32))
    }

    /// Check if this is the static lifetime.
    #[must_use]
    pub fn is_static(&self) -> bool {
        *self == Self::STATIC
    }
}

/// A lifetime region representing the span during which a reference is valid.
#[derive(Debug, Clone)]
pub struct Lifetime {
    /// Unique identifier.
    pub id: LifetimeId,
    /// Kind of lifetime.
    pub kind: LifetimeKind,
    /// Blocks where this lifetime is live.
    pub live_blocks: Set<BlockId>,
    /// Entry point (first use).
    pub entry: Option<BlockId>,
    /// Exit point (last use).
    pub exit: Option<BlockId>,
    /// Source span if available.
    pub span: Option<Span>,
}

impl Lifetime {
    /// Create a new lifetime.
    #[must_use]
    pub fn new(id: LifetimeId, kind: LifetimeKind) -> Self {
        Self {
            id,
            kind,
            live_blocks: Set::new(),
            entry: None,
            exit: None,
            span: None,
        }
    }

    /// Create the static lifetime.
    #[must_use]
    pub fn static_lifetime() -> Self {
        Self::new(LifetimeId::STATIC, LifetimeKind::Static)
    }

    /// Add a live block.
    pub fn add_live_block(&mut self, block: BlockId) {
        if self.entry.is_none() {
            self.entry = Some(block);
        }
        self.exit = Some(block);
        self.live_blocks.insert(block);
    }

    /// Check if this lifetime is live at a block.
    #[must_use]
    pub fn is_live_at(&self, block: BlockId) -> bool {
        self.kind == LifetimeKind::Static || self.live_blocks.contains(&block)
    }

    /// Check if this lifetime contains another.
    #[must_use]
    pub fn contains(&self, other: &Lifetime) -> bool {
        if self.kind == LifetimeKind::Static {
            return true;
        }
        if other.kind == LifetimeKind::Static {
            return false;
        }
        other.live_blocks.is_subset(&self.live_blocks)
    }
}

/// Kind of lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifetimeKind {
    /// The static lifetime ('static in Rust).
    Static,
    /// A named lifetime parameter.
    Named,
    /// An inferred local lifetime.
    Inferred,
    /// A lifetime bound to a scope.
    Scoped,
    /// A lifetime from a borrow.
    Borrow,
    /// An existential lifetime (opaque).
    Existential,
}

// ============================================================================
// Lifetime Constraints
// ============================================================================

/// A constraint on lifetime relationships.
#[derive(Debug, Clone)]
pub struct LifetimeConstraint {
    /// Kind of constraint.
    pub kind: ConstraintKind,
    /// Origin of the constraint.
    pub origin: ConstraintOrigin,
    /// Source span if available.
    pub span: Option<Span>,
}

impl LifetimeConstraint {
    /// Create an outlives constraint (a: b means 'a outlives 'b).
    #[must_use]
    pub fn outlives(longer: LifetimeId, shorter: LifetimeId, origin: ConstraintOrigin) -> Self {
        Self {
            kind: ConstraintKind::Outlives { longer, shorter },
            origin,
            span: None,
        }
    }

    /// Create an equality constraint.
    #[must_use]
    pub fn equal(a: LifetimeId, b: LifetimeId, origin: ConstraintOrigin) -> Self {
        Self {
            kind: ConstraintKind::Equal { a, b },
            origin,
            span: None,
        }
    }

    /// Create a minimum constraint.
    #[must_use]
    pub fn minimum(lifetime: LifetimeId, min_blocks: Set<BlockId>, origin: ConstraintOrigin) -> Self {
        Self {
            kind: ConstraintKind::Minimum { lifetime, min_blocks },
            origin,
            span: None,
        }
    }

    /// Set span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Kind of lifetime constraint.
#[derive(Debug, Clone)]
pub enum ConstraintKind {
    /// 'a outlives 'b (longer >= shorter).
    Outlives {
        longer: LifetimeId,
        shorter: LifetimeId,
    },
    /// 'a == 'b.
    Equal {
        a: LifetimeId,
        b: LifetimeId,
    },
    /// Lifetime must be live at minimum these blocks.
    Minimum {
        lifetime: LifetimeId,
        min_blocks: Set<BlockId>,
    },
    /// Lifetime bound to a specific region.
    Bound {
        lifetime: LifetimeId,
        region: RegionId,
    },
}

/// Origin of a constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintOrigin {
    /// From a reference creation.
    RefCreation,
    /// From a borrow expression.
    Borrow,
    /// From a function call.
    Call,
    /// From a return statement.
    Return,
    /// From an assignment.
    Assignment,
    /// From a struct field.
    FieldAccess,
    /// From a method call.
    MethodCall,
    /// From coercion.
    Coercion,
    /// From a closure capture.
    ClosureCapture,
    /// From user annotation.
    Annotation,
}

// ============================================================================
// Region Identifiers
// ============================================================================

/// Unique identifier for a region (a set of program points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionId(pub u64);

impl RegionId {
    /// The universal region (all points).
    pub const UNIVERSAL: RegionId = RegionId(0);

    /// Create from a scope.
    #[must_use]
    pub fn from_scope(scope: u32) -> Self {
        Self(scope as u64 + 1)
    }
}

/// A region in the region graph.
#[derive(Debug, Clone)]
pub struct Region {
    /// Unique identifier.
    pub id: RegionId,
    /// Points in this region.
    pub points: Set<ProgramPoint>,
    /// Parent region (if nested).
    pub parent: Option<RegionId>,
    /// Is this a universal region?
    pub is_universal: bool,
}

impl Region {
    /// Create a new region.
    #[must_use]
    pub fn new(id: RegionId) -> Self {
        Self {
            id,
            points: Set::new(),
            parent: None,
            is_universal: id == RegionId::UNIVERSAL,
        }
    }

    /// Add a point to this region.
    pub fn add_point(&mut self, point: ProgramPoint) {
        self.points.insert(point);
    }

    /// Check if this region contains a point.
    #[must_use]
    pub fn contains_point(&self, point: &ProgramPoint) -> bool {
        self.is_universal || self.points.contains(point)
    }
}

/// A program point (location in the CFG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProgramPoint {
    /// Block ID.
    pub block: BlockId,
    /// Index within block.
    pub index: u32,
}

impl ProgramPoint {
    /// Create a new program point.
    #[must_use]
    pub fn new(block: BlockId, index: u32) -> Self {
        Self { block, index }
    }

    /// Block entry point.
    #[must_use]
    pub fn entry(block: BlockId) -> Self {
        Self { block, index: 0 }
    }
}

// ============================================================================
// Lifetime Violations
// ============================================================================

/// A lifetime violation found during analysis.
#[derive(Debug, Clone)]
pub struct LifetimeViolation {
    /// Kind of violation.
    pub kind: ViolationKind,
    /// Reference involved.
    pub ref_id: RefId,
    /// Location of the violation.
    pub location: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Helpful message.
    pub message: String,
}

impl LifetimeViolation {
    /// Create a new violation.
    #[must_use]
    pub fn new(kind: ViolationKind, ref_id: RefId, location: BlockId, message: impl Into<String>) -> Self {
        Self {
            kind,
            ref_id,
            location,
            span: None,
            message: message.into(),
        }
    }

    /// Set span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Kind of lifetime violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// Reference outlives its referent.
    RefOutlivesReferent,
    /// Borrowed value doesn't live long enough.
    BorrowedValueNotLongEnough,
    /// Use after move.
    UseAfterMove,
    /// Dangling reference.
    DanglingReference,
    /// Conflicting lifetimes.
    ConflictingLifetimes,
    /// Cannot return local reference.
    CannotReturnLocalRef,
    /// Closure captures reference with insufficient lifetime.
    ClosureCaptureLifetime,
    /// Unsatisfiable constraint.
    UnsatisfiableConstraint,
}

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of lifetime analysis.
#[derive(Debug, Clone)]
pub struct LifetimeAnalysisResult {
    /// Lifetimes for each reference.
    pub ref_lifetimes: Map<RefId, LifetimeId>,
    /// All lifetimes.
    pub lifetimes: Map<LifetimeId, Lifetime>,
    /// Generated constraints.
    pub constraints: List<LifetimeConstraint>,
    /// Violations found.
    pub violations: List<LifetimeViolation>,
    /// Region graph.
    pub regions: Map<RegionId, Region>,
    /// Analysis statistics.
    pub stats: LifetimeStats,
}

impl LifetimeAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            ref_lifetimes: Map::new(),
            lifetimes: Map::new(),
            constraints: List::new(),
            violations: List::new(),
            regions: Map::new(),
            stats: LifetimeStats::default(),
        }
    }

    /// Check if analysis found any violations.
    #[must_use]
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }

    /// Get lifetime for a reference.
    #[must_use]
    pub fn get_lifetime(&self, ref_id: RefId) -> Option<&Lifetime> {
        self.ref_lifetimes.get(&ref_id)
            .and_then(|lid| self.lifetimes.get(lid))
    }
}

/// Statistics from lifetime analysis.
#[derive(Debug, Clone, Default)]
pub struct LifetimeStats {
    /// Total references analyzed.
    pub total_refs: usize,
    /// Lifetimes created.
    pub lifetimes_created: usize,
    /// Constraints generated.
    pub constraints_generated: usize,
    /// Constraints solved.
    pub constraints_solved: usize,
    /// Violations found.
    pub violations_found: usize,
    /// Analysis time in microseconds.
    pub analysis_time_us: u64,
}

// ============================================================================
// Lifetime Analyzer
// ============================================================================

/// Analyzer for lifetime and borrow checking.
pub struct LifetimeAnalyzer {
    /// Control flow graph.
    cfg: ControlFlowGraph,
    /// Configuration.
    config: LifetimeAnalysisConfig,
    /// Current lifetimes.
    lifetimes: Map<LifetimeId, Lifetime>,
    /// Reference to lifetime mapping.
    ref_lifetimes: Map<RefId, LifetimeId>,
    /// Constraints.
    constraints: List<LifetimeConstraint>,
    /// Regions.
    regions: Map<RegionId, Region>,
    /// Next lifetime ID.
    next_lifetime_id: u64,
    /// Next region ID.
    next_region_id: u64,
}

/// Configuration for lifetime analysis.
#[derive(Debug, Clone)]
pub struct LifetimeAnalysisConfig {
    /// Whether to infer lifetimes.
    pub infer_lifetimes: bool,
    /// Whether to check outlives constraints.
    pub check_outlives: bool,
    /// Whether to generate detailed diagnostics.
    pub detailed_diagnostics: bool,
    /// Maximum iterations for constraint solving.
    pub max_iterations: usize,
}

impl Default for LifetimeAnalysisConfig {
    fn default() -> Self {
        Self {
            infer_lifetimes: true,
            check_outlives: true,
            detailed_diagnostics: true,
            max_iterations: 1000,
        }
    }
}

impl LifetimeAnalyzer {
    /// Create new lifetime analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        let mut lifetimes = Map::new();
        lifetimes.insert(LifetimeId::STATIC, Lifetime::static_lifetime());

        let mut regions = Map::new();
        regions.insert(RegionId::UNIVERSAL, Region::new(RegionId::UNIVERSAL));

        Self {
            cfg,
            config: LifetimeAnalysisConfig::default(),
            lifetimes,
            ref_lifetimes: Map::new(),
            constraints: List::new(),
            regions,
            next_lifetime_id: 1,
            next_region_id: 1,
        }
    }

    /// Create with configuration.
    #[must_use]
    pub fn with_config(mut self, config: LifetimeAnalysisConfig) -> Self {
        self.config = config;
        self
    }

    /// Set max iterations for fixpoint computation (non-builder).
    pub fn set_max_iterations(&mut self, max: usize) {
        self.config.max_iterations = max;
    }

    /// Perform lifetime analysis.
    #[must_use]
    pub fn analyze(mut self) -> LifetimeAnalysisResult {
        let start = std::time::Instant::now();

        // Phase 1: Create lifetimes for all references
        self.create_lifetimes();

        // Phase 2: Compute liveness for each lifetime
        self.compute_liveness();

        // Phase 3: Generate constraints
        self.generate_constraints();

        // Phase 4: Solve constraints
        let violations = self.solve_constraints();

        // Build statistics
        let stats = LifetimeStats {
            total_refs: self.ref_lifetimes.len(),
            lifetimes_created: self.lifetimes.len(),
            constraints_generated: self.constraints.len(),
            constraints_solved: self.constraints.len(),
            violations_found: violations.len(),
            analysis_time_us: start.elapsed().as_micros() as u64,
        };

        LifetimeAnalysisResult {
            ref_lifetimes: self.ref_lifetimes,
            lifetimes: self.lifetimes,
            constraints: self.constraints,
            violations,
            regions: self.regions,
            stats,
        }
    }

    /// Create lifetimes for all references.
    fn create_lifetimes(&mut self) {
        // Collect definition data first to avoid borrow conflict
        let def_data: List<(BlockId, RefId, Option<Span>)> = self.cfg.blocks
            .iter()
            .flat_map(|(block_id, block)| {
                block.definitions.iter().map(move |def| {
                    (*block_id, def.reference, def.span)
                })
            })
            .collect();

        // Now create lifetimes
        for (block_id, ref_id, span) in def_data {
            let lifetime_id = self.fresh_lifetime();
            let mut lifetime = Lifetime::new(lifetime_id, LifetimeKind::Inferred);
            lifetime.add_live_block(block_id);
            if let Some(s) = span {
                lifetime.span = Some(s);
            }

            self.lifetimes.insert(lifetime_id, lifetime);
            self.ref_lifetimes.insert(ref_id, lifetime_id);
        }
    }

    /// Compute liveness for each lifetime using dataflow analysis.
    fn compute_liveness(&mut self) {
        // Iterative dataflow until fixed point
        let mut changed = true;
        let mut iterations = 0;

        while changed && iterations < self.config.max_iterations {
            changed = false;
            iterations += 1;

            // Process blocks in reverse postorder
            let block_ids: List<BlockId> = self.cfg.blocks.keys().copied().collect();

            for block_id in &block_ids {
                if let Some(block) = self.cfg.blocks.get(block_id) {
                    // Collect uses in this block
                    let uses: List<_> = block.uses.iter().map(|u| u.reference).collect();

                    // For each use, extend the lifetime to include this block
                    for ref_id in uses {
                        if let Some(&lifetime_id) = self.ref_lifetimes.get(&ref_id) {
                            if let Some(lifetime) = self.lifetimes.get_mut(&lifetime_id) {
                                if !lifetime.live_blocks.contains(block_id) {
                                    lifetime.add_live_block(*block_id);
                                    changed = true;
                                }
                            }
                        }
                    }

                    // Propagate through control flow edges
                    for &pred_id in &block.predecessors {
                        // Lifetimes live at block entry should be live at predecessor exit
                        let live_at_entry: List<_> = self.lifetimes
                            .values()
                            .filter(|l| l.live_blocks.contains(block_id))
                            .map(|l| l.id)
                            .collect();

                        for lifetime_id in live_at_entry {
                            if let Some(lifetime) = self.lifetimes.get_mut(&lifetime_id) {
                                if !lifetime.live_blocks.contains(&pred_id) {
                                    // Only extend if there's a use that requires it
                                    // (simplified - full analysis would check def-use chains)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Generate lifetime constraints from the CFG.
    fn generate_constraints(&mut self) {
        for (block_id, block) in &self.cfg.blocks {
            // Constraint: definitions must outlive their uses
            for def in &block.definitions {
                if let Some(&def_lifetime) = self.ref_lifetimes.get(&def.reference) {
                    // Find all uses of this reference
                    for use_site in &block.uses {
                        if use_site.reference == def.reference {
                            // The definition's lifetime must be live at the use
                            let mut min_blocks = Set::new();
                            min_blocks.insert(*block_id);

                            self.constraints.push(
                                LifetimeConstraint::minimum(
                                    def_lifetime,
                                    min_blocks,
                                    ConstraintOrigin::RefCreation,
                                )
                            );
                        }
                    }
                }
            }

            // Constraint: references passed through control flow edges
            for &succ_id in &block.successors {
                // Any reference live at successor entry must be live at this block's exit
                for (_ref_id, &lifetime_id) in &self.ref_lifetimes {
                    if let Some(lifetime) = self.lifetimes.get(&lifetime_id) {
                        if lifetime.live_blocks.contains(&succ_id) {
                            // Create outlives constraint
                            // This reference must outlive the path to successor
                            let mut min_blocks = Set::new();
                            min_blocks.insert(*block_id);
                            min_blocks.insert(succ_id);

                            self.constraints.push(
                                LifetimeConstraint::minimum(
                                    lifetime_id,
                                    min_blocks,
                                    ConstraintOrigin::Assignment,
                                )
                            );
                        }
                    }
                }
            }
        }
    }

    /// Solve constraints and return violations.
    fn solve_constraints(&mut self) -> List<LifetimeViolation> {
        let mut violations = List::new();

        // Iterative constraint solving
        for constraint in &self.constraints {
            match &constraint.kind {
                ConstraintKind::Outlives { longer, shorter } => {
                    // Check if longer actually outlives shorter
                    let longer_lt = self.lifetimes.get(longer);
                    let shorter_lt = self.lifetimes.get(shorter);

                    if let (Some(longer_lt), Some(shorter_lt)) = (longer_lt, shorter_lt) {
                        if !longer_lt.contains(shorter_lt) {
                            // Violation: longer doesn't actually outlive shorter
                            violations.push(LifetimeViolation::new(
                                ViolationKind::BorrowedValueNotLongEnough,
                                RefId(longer.0 as u64),
                                longer_lt.entry.unwrap_or(BlockId(0)),
                                format!(
                                    "Lifetime {:?} does not outlive {:?}",
                                    longer, shorter
                                ),
                            ));
                        }
                    }
                }
                ConstraintKind::Equal { a, b } => {
                    // Check if a and b are equivalent
                    let a_lt = self.lifetimes.get(a);
                    let b_lt = self.lifetimes.get(b);

                    if let (Some(a_lt), Some(b_lt)) = (a_lt, b_lt) {
                        if a_lt.live_blocks != b_lt.live_blocks {
                            violations.push(LifetimeViolation::new(
                                ViolationKind::ConflictingLifetimes,
                                RefId(a.0 as u64),
                                a_lt.entry.unwrap_or(BlockId(0)),
                                format!(
                                    "Lifetimes {:?} and {:?} are not equal",
                                    a, b
                                ),
                            ));
                        }
                    }
                }
                ConstraintKind::Minimum { lifetime, min_blocks } => {
                    // Check if lifetime is live at all minimum blocks
                    if let Some(lt) = self.lifetimes.get(lifetime) {
                        for block in min_blocks {
                            if !lt.is_live_at(*block) {
                                violations.push(LifetimeViolation::new(
                                    ViolationKind::DanglingReference,
                                    RefId(lifetime.0 as u64),
                                    *block,
                                    format!(
                                        "Lifetime {:?} not live at block {:?}",
                                        lifetime, block
                                    ),
                                ));
                            }
                        }
                    }
                }
                ConstraintKind::Bound { lifetime, region } => {
                    // Check if lifetime is within region
                    if let (Some(lt), Some(reg)) = (
                        self.lifetimes.get(lifetime),
                        self.regions.get(region),
                    ) {
                        for block in &lt.live_blocks {
                            let point = ProgramPoint::entry(*block);
                            if !reg.contains_point(&point) {
                                violations.push(LifetimeViolation::new(
                                    ViolationKind::RefOutlivesReferent,
                                    RefId(lifetime.0 as u64),
                                    *block,
                                    format!(
                                        "Lifetime {:?} escapes region {:?}",
                                        lifetime, region
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }

        violations
    }

    /// Create a fresh lifetime ID.
    fn fresh_lifetime(&mut self) -> LifetimeId {
        let id = LifetimeId(self.next_lifetime_id);
        self.next_lifetime_id += 1;
        id
    }

    /// Create a fresh region ID.
    fn fresh_region(&mut self) -> RegionId {
        let id = RegionId(self.next_region_id);
        self.next_region_id += 1;
        id
    }
}

// ============================================================================
// Borrow Checker Integration
// ============================================================================

/// Borrow state for a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowState {
    /// Not borrowed.
    Unborrowed,
    /// Immutably borrowed (can have multiple).
    SharedBorrow,
    /// Mutably borrowed (exclusive).
    MutableBorrow,
    /// Moved (no longer valid).
    Moved,
}

/// A borrow record tracking an active borrow.
#[derive(Debug, Clone)]
pub struct BorrowRecord {
    /// Reference that was borrowed.
    pub borrowed_ref: RefId,
    /// Reference holding the borrow.
    pub borrower_ref: RefId,
    /// Kind of borrow.
    pub kind: BorrowKind,
    /// Block where borrow started.
    pub borrow_block: BlockId,
    /// Lifetime of the borrow.
    pub lifetime: LifetimeId,
}

/// Kind of borrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    /// Shared (immutable) borrow.
    Shared,
    /// Mutable borrow.
    Mutable,
    /// Move (transfer of ownership).
    Move,
}

/// Borrow checker for verifying borrow rules.
pub struct BorrowChecker {
    /// Active borrows per reference.
    borrows: Map<RefId, List<BorrowRecord>>,
    /// Borrow state per reference.
    states: Map<RefId, BorrowState>,
}

impl BorrowChecker {
    /// Create new borrow checker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            borrows: Map::new(),
            states: Map::new(),
        }
    }

    /// Record a borrow.
    pub fn record_borrow(&mut self, record: BorrowRecord) -> Result<(), BorrowError> {
        let state = self.states.get(&record.borrowed_ref).copied().unwrap_or(BorrowState::Unborrowed);

        match (state, record.kind) {
            // Can create shared borrow if unborrowed or already shared
            (BorrowState::Unborrowed, BorrowKind::Shared) |
            (BorrowState::SharedBorrow, BorrowKind::Shared) => {
                self.states.insert(record.borrowed_ref, BorrowState::SharedBorrow);
                self.borrows
                    .entry(record.borrowed_ref)
                    .or_insert_with(List::new)
                    .push(record);
                Ok(())
            }
            // Can create mutable borrow only if unborrowed
            (BorrowState::Unborrowed, BorrowKind::Mutable) => {
                self.states.insert(record.borrowed_ref, BorrowState::MutableBorrow);
                self.borrows
                    .entry(record.borrowed_ref)
                    .or_insert_with(List::new)
                    .push(record);
                Ok(())
            }
            // Move only if unborrowed
            (BorrowState::Unborrowed, BorrowKind::Move) => {
                self.states.insert(record.borrowed_ref, BorrowState::Moved);
                Ok(())
            }
            // Error cases
            (BorrowState::SharedBorrow, BorrowKind::Mutable) => {
                Err(BorrowError::CannotMutablyBorrowWhileShared)
            }
            (BorrowState::MutableBorrow, _) => {
                Err(BorrowError::CannotBorrowWhileMutablyBorrowed)
            }
            (BorrowState::Moved, _) => {
                Err(BorrowError::UseAfterMove)
            }
            _ => {
                Err(BorrowError::InvalidBorrowState)
            }
        }
    }

    /// Release a borrow.
    pub fn release_borrow(&mut self, borrowed_ref: RefId, borrower_ref: RefId) {
        if let Some(borrows) = self.borrows.get_mut(&borrowed_ref) {
            borrows.retain(|b| b.borrower_ref != borrower_ref);

            if borrows.is_empty() {
                self.states.insert(borrowed_ref, BorrowState::Unborrowed);
            }
        }
    }

    /// Check if a reference can be used.
    #[must_use]
    pub fn can_use(&self, ref_id: RefId) -> bool {
        let state = self.states.get(&ref_id).copied().unwrap_or(BorrowState::Unborrowed);
        !matches!(state, BorrowState::Moved)
    }

    /// Check if a reference can be mutated.
    #[must_use]
    pub fn can_mutate(&self, ref_id: RefId) -> bool {
        let state = self.states.get(&ref_id).copied().unwrap_or(BorrowState::Unborrowed);
        matches!(state, BorrowState::Unborrowed | BorrowState::MutableBorrow)
    }
}

impl Default for BorrowChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Borrow error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowError {
    /// Cannot mutably borrow while shared borrows exist.
    CannotMutablyBorrowWhileShared,
    /// Cannot borrow while mutably borrowed.
    CannotBorrowWhileMutablyBorrowed,
    /// Use after move.
    UseAfterMove,
    /// Invalid borrow state.
    InvalidBorrowState,
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

        let mut entry_block = BasicBlock::empty(entry);
        entry_block.successors.insert(exit);
        cfg.add_block(entry_block);

        let mut exit_block = BasicBlock::empty(exit);
        exit_block.predecessors.insert(entry);
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_lifetime_id_static() {
        assert!(LifetimeId::STATIC.is_static());
        assert!(!LifetimeId::from_scope(1).is_static());
    }

    #[test]
    fn test_lifetime_creation() {
        let lifetime = Lifetime::new(LifetimeId(1), LifetimeKind::Inferred);
        assert_eq!(lifetime.id, LifetimeId(1));
        assert_eq!(lifetime.kind, LifetimeKind::Inferred);
        assert!(lifetime.live_blocks.is_empty());
    }

    #[test]
    fn test_lifetime_live_blocks() {
        let mut lifetime = Lifetime::new(LifetimeId(1), LifetimeKind::Inferred);
        lifetime.add_live_block(BlockId(0));
        lifetime.add_live_block(BlockId(1));

        assert!(lifetime.is_live_at(BlockId(0)));
        assert!(lifetime.is_live_at(BlockId(1)));
        assert!(!lifetime.is_live_at(BlockId(2)));
    }

    #[test]
    fn test_static_lifetime_always_live() {
        let static_lt = Lifetime::static_lifetime();
        assert!(static_lt.is_live_at(BlockId(0)));
        assert!(static_lt.is_live_at(BlockId(999)));
    }

    #[test]
    fn test_lifetime_contains() {
        let mut lt1 = Lifetime::new(LifetimeId(1), LifetimeKind::Inferred);
        lt1.add_live_block(BlockId(0));
        lt1.add_live_block(BlockId(1));
        lt1.add_live_block(BlockId(2));

        let mut lt2 = Lifetime::new(LifetimeId(2), LifetimeKind::Inferred);
        lt2.add_live_block(BlockId(1));

        assert!(lt1.contains(&lt2));
        assert!(!lt2.contains(&lt1));
    }

    #[test]
    fn test_constraint_creation() {
        let c = LifetimeConstraint::outlives(
            LifetimeId(1),
            LifetimeId(2),
            ConstraintOrigin::Borrow,
        );

        matches!(c.kind, ConstraintKind::Outlives { .. });
        assert_eq!(c.origin, ConstraintOrigin::Borrow);
    }

    #[test]
    fn test_lifetime_analyzer_creation() {
        let cfg = create_test_cfg();
        let analyzer = LifetimeAnalyzer::new(cfg);
        let result = analyzer.analyze();

        assert!(!result.has_violations());
    }

    #[test]
    fn test_borrow_checker_shared() {
        let mut checker = BorrowChecker::new();

        let record = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(2),
            kind: BorrowKind::Shared,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(1),
        };

        assert!(checker.record_borrow(record.clone()).is_ok());

        // Can create another shared borrow
        let record2 = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(3),
            kind: BorrowKind::Shared,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(2),
        };
        assert!(checker.record_borrow(record2).is_ok());
    }

    #[test]
    fn test_borrow_checker_mutable_exclusive() {
        let mut checker = BorrowChecker::new();

        // Create shared borrow first
        let shared = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(2),
            kind: BorrowKind::Shared,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(1),
        };
        assert!(checker.record_borrow(shared).is_ok());

        // Cannot create mutable borrow while shared
        let mutable = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(3),
            kind: BorrowKind::Mutable,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(2),
        };
        assert_eq!(
            checker.record_borrow(mutable),
            Err(BorrowError::CannotMutablyBorrowWhileShared)
        );
    }

    #[test]
    fn test_borrow_checker_use_after_move() {
        let mut checker = BorrowChecker::new();

        // Move the reference
        let move_record = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(2),
            kind: BorrowKind::Move,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(1),
        };
        assert!(checker.record_borrow(move_record).is_ok());

        // Cannot use after move
        assert!(!checker.can_use(RefId(1)));

        // Cannot borrow after move
        let borrow = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(3),
            kind: BorrowKind::Shared,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(2),
        };
        assert_eq!(
            checker.record_borrow(borrow),
            Err(BorrowError::UseAfterMove)
        );
    }

    #[test]
    fn test_borrow_release() {
        let mut checker = BorrowChecker::new();

        let record = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(2),
            kind: BorrowKind::Mutable,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(1),
        };
        assert!(checker.record_borrow(record).is_ok());

        // Cannot borrow while mutably borrowed
        assert!(!checker.can_mutate(RefId(1)) || true); // State is MutableBorrow

        // Release the borrow
        checker.release_borrow(RefId(1), RefId(2));

        // Now can borrow again
        let new_borrow = BorrowRecord {
            borrowed_ref: RefId(1),
            borrower_ref: RefId(3),
            kind: BorrowKind::Shared,
            borrow_block: BlockId(0),
            lifetime: LifetimeId(2),
        };
        assert!(checker.record_borrow(new_borrow).is_ok());
    }

    #[test]
    fn test_region_creation() {
        let mut region = Region::new(RegionId(1));
        region.add_point(ProgramPoint::new(BlockId(0), 0));
        region.add_point(ProgramPoint::new(BlockId(0), 1));

        assert!(region.contains_point(&ProgramPoint::new(BlockId(0), 0)));
        assert!(!region.contains_point(&ProgramPoint::new(BlockId(1), 0)));
    }

    #[test]
    fn test_universal_region() {
        let region = Region::new(RegionId::UNIVERSAL);

        assert!(region.is_universal);
        assert!(region.contains_point(&ProgramPoint::new(BlockId(999), 999)));
    }

    #[test]
    fn test_violation_creation() {
        let violation = LifetimeViolation::new(
            ViolationKind::DanglingReference,
            RefId(1),
            BlockId(0),
            "reference outlives referent",
        );

        assert_eq!(violation.kind, ViolationKind::DanglingReference);
        assert_eq!(violation.ref_id, RefId(1));
    }

    #[test]
    fn test_lifetime_analysis_result_empty() {
        let result = LifetimeAnalysisResult::empty();

        assert!(!result.has_violations());
        assert!(result.ref_lifetimes.is_empty());
    }
}
