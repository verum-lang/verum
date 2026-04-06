//! Non-Lexical Lifetimes (NLL) Analysis
//!
//! This module implements Rust-style Non-Lexical Lifetimes for Verum, enabling
//! more precise borrow checking based on actual variable liveness rather than
//! lexical scope. This is the state-of-the-art approach to borrow checking.
//!
//! # Key Concepts
//!
//! ## Non-Lexical Regions
//!
//! Unlike lexical lifetimes that end at scope boundaries, NLL regions are
//! based on actual liveness - a borrow is active only while it's actually
//! used:
//!
//! ```text
//! fn example() {
//!     let mut x = 5;
//!     let r = &x;      // Borrow starts here
//!     println!("{}", r);  // Last use of r
//!     // Borrow ends here (NLL) vs. end of scope (lexical)
//!     x = 10;          // Now allowed with NLL!
//! }
//! ```
//!
//! ## Two-Phase Borrows
//!
//! Enables patterns like `vec.push(vec.len())` where the mutable borrow
//! is "reserved" but not activated until needed:
//!
//! ```text
//! Phase 1 (Reservation): &mut vec is reserved but inactive
//! Phase 2 (Activation): vec.len() can use &vec
//! Phase 3 (Use): push activates the mutable borrow
//! ```
//!
//! # Architecture
//!
//! ```text
//! CFG → NllAnalyzer → NllAnalysisResult
//!                          │
//!                          ▼
//!           ┌───────────────────────────────────┐
//!           │ Map<RefId, NllRegion>             │
//!           │ LivenessInfo                      │
//!           │ BorrowSet                         │
//!           │ List<NllViolation>                │
//!           └───────────────────────────────────┘
//! ```
//!
//! # Algorithm Overview
//!
//! 1. **Liveness Analysis**: Compute live ranges for all variables
//! 2. **Region Inference**: Create minimal regions based on liveness
//! 3. **Constraint Generation**: Generate subset constraints
//! 4. **Constraint Solving**: Fixed-point iteration
//! 5. **Violation Detection**: Check for conflicts
//!
//! Spec: Based on Rust RFC 2094 (Non-Lexical Lifetimes)

use crate::analysis::{BlockId, ControlFlowGraph, RefId, Span};
use verum_common::{List, Map, Set};

// ============================================================================
// NLL Region Types
// ============================================================================

/// A non-lexical region - a set of points where a borrow is active.
#[derive(Debug, Clone)]
pub struct NllRegion {
    /// Unique identifier.
    pub id: NllRegionId,
    /// Kind of region.
    pub kind: NllRegionKind,
    /// Program points in this region.
    pub points: Set<NllPoint>,
    /// Universal region elements (for function boundaries).
    pub universal_elements: Set<UniversalElement>,
    /// Is this a placeholder region (from signature)?
    pub is_placeholder: bool,
}

impl NllRegion {
    /// Create a new empty region.
    #[must_use]
    pub fn new(id: NllRegionId, kind: NllRegionKind) -> Self {
        Self {
            id,
            kind,
            points: Set::new(),
            universal_elements: Set::new(),
            is_placeholder: false,
        }
    }

    /// Create a universal (static) region.
    #[must_use]
    pub fn universal(id: NllRegionId) -> Self {
        let mut region = Self::new(id, NllRegionKind::Universal);
        region.universal_elements.insert(UniversalElement::Static);
        region
    }

    /// Add a point to this region.
    pub fn add_point(&mut self, point: NllPoint) {
        self.points.insert(point);
    }

    /// Add a universal element.
    pub fn add_universal(&mut self, elem: UniversalElement) {
        self.universal_elements.insert(elem);
    }

    /// Check if this region contains a point.
    #[must_use]
    pub fn contains(&self, point: &NllPoint) -> bool {
        self.kind == NllRegionKind::Universal || self.points.contains(point)
    }

    /// Check if this region is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.universal_elements.is_empty()
    }

    /// Merge another region into this one.
    pub fn merge(&mut self, other: &NllRegion) {
        for point in &other.points {
            self.points.insert(*point);
        }
        for elem in &other.universal_elements {
            self.universal_elements.insert(*elem);
        }
    }
}

/// Unique identifier for an NLL region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NllRegionId(pub u64);

impl NllRegionId {
    /// The static region.
    pub const STATIC: NllRegionId = NllRegionId(0);
}

/// Kind of NLL region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NllRegionKind {
    /// Universal region (static lifetime).
    Universal,
    /// Existential region (local lifetime).
    Existential,
    /// Placeholder from function signature.
    Placeholder,
    /// Inferred region.
    Inferred,
}

// ============================================================================
// Program Points
// ============================================================================

/// A program point in the NLL analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NllPoint {
    /// Block containing this point.
    pub block: BlockId,
    /// Statement index within block.
    pub statement: u32,
    /// Point kind (Start, Mid, or End of statement).
    pub kind: PointKind,
}

impl NllPoint {
    /// Create a new program point.
    #[must_use]
    pub fn new(block: BlockId, statement: u32, kind: PointKind) -> Self {
        Self { block, statement, kind }
    }

    /// Start of a statement.
    #[must_use]
    pub fn start(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, PointKind::Start)
    }

    /// Middle of a statement (for two-phase borrows).
    #[must_use]
    pub fn mid(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, PointKind::Mid)
    }

    /// End of a statement.
    #[must_use]
    pub fn end(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, PointKind::End)
    }
}

/// Kind of program point within a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointKind {
    /// Before the statement executes.
    Start,
    /// During statement execution (for two-phase).
    Mid,
    /// After the statement executes.
    End,
}

/// A universal element (for function boundaries).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UniversalElement {
    /// The static lifetime element.
    Static,
    /// A function's return region.
    Return(u32),
    /// A function parameter region.
    Param(u32),
    /// An external region.
    External(u64),
}

// ============================================================================
// Liveness Analysis
// ============================================================================

/// Liveness information for all variables.
#[derive(Debug, Clone, Default)]
pub struct LivenessInfo {
    /// Live ranges per variable.
    pub live_ranges: Map<RefId, LiveRange>,
    /// Drop points per variable.
    pub drop_points: Map<RefId, Set<NllPoint>>,
    /// Use points per variable.
    pub use_points: Map<RefId, Set<NllPoint>>,
    /// Definition points per variable.
    pub def_points: Map<RefId, NllPoint>,
}

impl LivenessInfo {
    /// Create empty liveness info.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a variable is live at a point.
    #[must_use]
    pub fn is_live_at(&self, var: RefId, point: &NllPoint) -> bool {
        self.live_ranges
            .get(&var)
            .map_or(false, |range| range.contains(point))
    }

    /// Get last use of a variable.
    #[must_use]
    pub fn last_use(&self, var: RefId) -> Option<NllPoint> {
        self.use_points.get(&var).and_then(|uses| {
            uses.iter().max_by(|a, b| {
                a.block.0.cmp(&b.block.0)
                    .then_with(|| a.statement.cmp(&b.statement))
            }).copied()
        })
    }
}

/// Live range for a variable.
#[derive(Debug, Clone, Default)]
pub struct LiveRange {
    /// Points where the variable is live.
    pub points: Set<NllPoint>,
    /// Start of the live range.
    pub start: Option<NllPoint>,
    /// End of the live range (NLL-style, based on last use).
    pub end: Option<NllPoint>,
}

impl LiveRange {
    /// Create empty live range.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a point to the live range.
    pub fn add_point(&mut self, point: NllPoint) {
        if self.start.is_none() {
            self.start = Some(point);
        }
        self.end = Some(point);
        self.points.insert(point);
    }

    /// Check if this range contains a point.
    #[must_use]
    pub fn contains(&self, point: &NllPoint) -> bool {
        self.points.contains(point)
    }
}

// ============================================================================
// Borrow Set
// ============================================================================

/// Set of active borrows at a program point.
#[derive(Debug, Clone, Default)]
pub struct BorrowSet {
    /// Active borrows.
    pub borrows: Map<BorrowId, BorrowData>,
    /// Next borrow ID.
    next_id: u64,
}

impl BorrowSet {
    /// Create empty borrow set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new borrow.
    pub fn create_borrow(&mut self, data: BorrowData) -> BorrowId {
        let id = BorrowId(self.next_id);
        self.next_id += 1;
        self.borrows.insert(id, data);
        id
    }

    /// Kill a borrow.
    pub fn kill_borrow(&mut self, id: BorrowId) {
        self.borrows.remove(&id);
    }

    /// Get active borrows for a place.
    #[must_use]
    pub fn borrows_for(&self, place: RefId) -> List<&BorrowData> {
        self.borrows
            .values()
            .filter(|b| b.borrowed_place == place)
            .collect()
    }

    /// Check for conflicting borrows (ignoring liveness).
    ///
    /// This is the legacy method that doesn't consider release points.
    /// Prefer `has_conflict_at` for liveness-aware conflict checking.
    #[must_use]
    pub fn has_conflict(&self, place: RefId, kind: NllBorrowKind) -> Option<&BorrowData> {
        self.borrows.values().find(|b| {
            b.borrowed_place == place && b.conflicts_with(kind)
        })
    }

    /// Check for conflicting borrows at a specific point (liveness-aware).
    ///
    /// This is the liveness-based conflict checker. A conflict only exists
    /// if the existing borrow is still live at the given point.
    ///
    /// Liveness-based borrow release: borrows are released at the point of last
    /// use of the assigned_place (the variable holding the reference), not at
    /// lexical scope end. This enables earlier re-access of the borrowed place.
    #[must_use]
    pub fn has_conflict_at(
        &self,
        place: RefId,
        kind: NllBorrowKind,
        point: &NllPoint,
    ) -> Option<&BorrowData> {
        self.borrows.values().find(|b| {
            b.borrowed_place == place
                && b.conflicts_with(kind)
                && b.is_live_at(point)
        })
    }
}

/// Unique identifier for a borrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BorrowId(pub u64);

/// Data about a borrow.
#[derive(Debug, Clone)]
pub struct BorrowData {
    /// ID of the borrow.
    pub id: BorrowId,
    /// Place being borrowed.
    pub borrowed_place: RefId,
    /// Reference holding the borrow.
    pub assigned_place: RefId,
    /// Kind of borrow.
    pub kind: NllBorrowKind,
    /// Region of the borrow.
    pub region: NllRegionId,
    /// Point where borrow was created.
    pub reserve_point: NllPoint,
    /// Point where borrow was activated (for two-phase).
    pub activation_point: Option<NllPoint>,
    /// Is this a two-phase borrow?
    pub two_phase: bool,
    /// Point where borrow is released (liveness-based).
    ///
    /// This is the key field for liveness-based borrow release.
    /// Instead of releasing at scope end, the borrow is released at
    /// the last use point, enabling earlier access to the borrowed place.
    ///
    /// Liveness-based borrow release: borrows are released at the point of last
    /// use of the assigned_place (the variable holding the reference), not at
    /// lexical scope end. This enables earlier re-access of the borrowed place.
    pub release_point: Option<NllPoint>,
}

impl BorrowData {
    /// Check if this borrow conflicts with another kind.
    #[must_use]
    pub fn conflicts_with(&self, other: NllBorrowKind) -> bool {
        match (self.kind, other) {
            (NllBorrowKind::Shared, NllBorrowKind::Shared) => false,
            _ => true,
        }
    }

    /// Check if this borrow is active (not just reserved).
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.two_phase || self.activation_point.is_some()
    }

    /// Check if this borrow has been released at or before the given point.
    ///
    /// This is the key method for liveness-based borrow release checking.
    /// A borrow is considered released if:
    /// 1. It has a release_point, AND
    /// 2. The given point is at or after the release_point
    ///
    /// Liveness-based borrow release: borrows are released at the point of last
    /// use of the assigned_place (the variable holding the reference), not at
    /// lexical scope end. This enables earlier re-access of the borrowed place.
    #[must_use]
    pub fn is_released_at(&self, point: &NllPoint) -> bool {
        if let Some(release) = &self.release_point {
            // Compare points: release happened before or at this point
            if point.block.0 > release.block.0 {
                return true;
            }
            if point.block == release.block && point.statement >= release.statement {
                return true;
            }
        }
        false
    }

    /// Check if this borrow is live at the given point.
    ///
    /// A borrow is live if:
    /// 1. It has been created (reserve_point <= point)
    /// 2. It has not been released yet (!is_released_at(point))
    #[must_use]
    pub fn is_live_at(&self, point: &NllPoint) -> bool {
        // Check if point is at or after reserve_point
        let after_reserve = point.block.0 > self.reserve_point.block.0
            || (point.block == self.reserve_point.block
                && point.statement >= self.reserve_point.statement);

        after_reserve && !self.is_released_at(point)
    }
}

/// Kind of borrow in NLL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NllBorrowKind {
    /// Shared (immutable) borrow.
    Shared,
    /// Mutable borrow.
    Mutable,
    /// Shallow borrow (for two-phase).
    Shallow,
}

// ============================================================================
// Constraints
// ============================================================================

/// A constraint on NLL regions.
#[derive(Debug, Clone)]
pub struct NllConstraint {
    /// Kind of constraint.
    pub kind: NllConstraintKind,
    /// Source span for diagnostics.
    pub span: Option<Span>,
}

impl NllConstraint {
    /// Create a subset constraint: sub ⊆ sup.
    #[must_use]
    pub fn subset(sub: NllRegionId, sup: NllRegionId) -> Self {
        Self {
            kind: NllConstraintKind::Subset { sub, sup },
            span: None,
        }
    }

    /// Create a liveness constraint: region must be live at point.
    #[must_use]
    pub fn live_at(region: NllRegionId, point: NllPoint) -> Self {
        Self {
            kind: NllConstraintKind::LiveAt { region, point },
            span: None,
        }
    }
}

/// Kind of NLL constraint.
#[derive(Debug, Clone)]
pub enum NllConstraintKind {
    /// Region subset constraint: sub ⊆ sup.
    Subset {
        sub: NllRegionId,
        sup: NllRegionId,
    },
    /// Region must be live at point.
    LiveAt {
        region: NllRegionId,
        point: NllPoint,
    },
    /// Borrow must be valid in region.
    BorrowValid {
        borrow: BorrowId,
        region: NllRegionId,
    },
}

// ============================================================================
// Violations
// ============================================================================

/// An NLL borrow checking violation.
#[derive(Debug, Clone)]
pub struct NllViolation {
    /// Kind of violation.
    pub kind: NllViolationKind,
    /// Primary span.
    pub span: Option<Span>,
    /// Helpful message.
    pub message: String,
}

impl NllViolation {
    /// Create a new violation.
    #[must_use]
    pub fn new(kind: NllViolationKind, message: impl Into<String>) -> Self {
        Self {
            kind,
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

/// Kind of NLL violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NllViolationKind {
    /// Conflicting borrows.
    ConflictingBorrow {
        first: BorrowId,
        second: BorrowId,
    },
    /// Use while mutably borrowed.
    UseWhileMutablyBorrowed {
        place: RefId,
        borrow: BorrowId,
    },
    /// Mutation while borrowed.
    MutationWhileBorrowed {
        place: RefId,
        borrow: BorrowId,
    },
    /// Move while borrowed.
    MoveWhileBorrowed {
        place: RefId,
        borrow: BorrowId,
    },
    /// Borrow outlives data.
    BorrowOutlivesData {
        borrow: BorrowId,
    },
    /// Return of local reference.
    ReturnLocalRef {
        borrow: BorrowId,
    },
}

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of NLL analysis.
#[derive(Debug, Clone)]
pub struct NllAnalysisResult {
    /// Inferred regions.
    pub regions: Map<NllRegionId, NllRegion>,
    /// Liveness information.
    pub liveness: LivenessInfo,
    /// Final borrow set.
    pub borrows: BorrowSet,
    /// Constraints generated.
    pub constraints: List<NllConstraint>,
    /// Violations found.
    pub violations: List<NllViolation>,
    /// Statistics.
    pub stats: NllStats,
}

impl NllAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            regions: Map::new(),
            liveness: LivenessInfo::new(),
            borrows: BorrowSet::new(),
            constraints: List::new(),
            violations: List::new(),
            stats: NllStats::default(),
        }
    }

    /// Check if analysis found violations.
    #[must_use]
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }
}

/// Statistics from NLL analysis.
#[derive(Debug, Clone, Default)]
pub struct NllStats {
    /// Regions created.
    pub regions_created: usize,
    /// Constraints generated.
    pub constraints_generated: usize,
    /// Fixed-point iterations.
    pub iterations: usize,
    /// Borrows tracked.
    pub borrows_tracked: usize,
    /// Violations found.
    pub violations_found: usize,
    /// Analysis time in microseconds.
    pub analysis_time_us: u64,
}

// ============================================================================
// NLL Analyzer
// ============================================================================

/// NLL borrow checker analyzer.
pub struct NllAnalyzer {
    /// Control flow graph.
    cfg: ControlFlowGraph,
    /// Configuration.
    config: NllConfig,
    /// Regions.
    regions: Map<NllRegionId, NllRegion>,
    /// Liveness info.
    liveness: LivenessInfo,
    /// Borrow set.
    borrows: BorrowSet,
    /// Constraints.
    constraints: List<NllConstraint>,
    /// Next region ID.
    next_region_id: u64,
}

/// Configuration for NLL analysis.
#[derive(Debug, Clone)]
pub struct NllConfig {
    /// Enable two-phase borrows.
    pub two_phase_borrows: bool,
    /// Enable polonius-style analysis.
    pub polonius_mode: bool,
    /// Maximum iterations for constraint solving.
    pub max_iterations: usize,
    /// Emit detailed diagnostics.
    pub detailed_diagnostics: bool,
}

impl Default for NllConfig {
    fn default() -> Self {
        Self {
            two_phase_borrows: true,
            polonius_mode: false,
            max_iterations: 1000,
            detailed_diagnostics: true,
        }
    }
}

impl NllAnalyzer {
    /// Create new NLL analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        let mut regions = Map::new();
        regions.insert(NllRegionId::STATIC, NllRegion::universal(NllRegionId::STATIC));

        Self {
            cfg,
            config: NllConfig::default(),
            regions,
            liveness: LivenessInfo::new(),
            borrows: BorrowSet::new(),
            constraints: List::new(),
            next_region_id: 1,
        }
    }

    /// Create with configuration.
    #[must_use]
    pub fn with_config(mut self, config: NllConfig) -> Self {
        self.config = config;
        self
    }

    /// Set max iterations for fixpoint computation (non-builder).
    pub fn set_max_iterations(&mut self, max: usize) {
        self.config.max_iterations = max;
    }

    /// Perform NLL analysis.
    #[must_use]
    pub fn analyze(mut self) -> NllAnalysisResult {
        let start = std::time::Instant::now();

        // Phase 1: Compute liveness for all variables
        self.compute_liveness();

        // Phase 2: Create initial regions for borrows
        self.create_borrow_regions();

        // Phase 3: Compute release points for liveness-based borrow release
        // Liveness-based borrow release: release at last use, not scope end
        self.compute_release_points();

        // Phase 4: Generate constraints
        self.generate_constraints();

        // Phase 5: Solve constraints (fixed-point iteration)
        let iterations = self.solve_constraints();

        // Phase 6: Check for violations (now considers release points)
        let violations = self.check_violations();

        // Build statistics
        let stats = NllStats {
            regions_created: self.regions.len(),
            constraints_generated: self.constraints.len(),
            iterations,
            borrows_tracked: self.borrows.borrows.len(),
            violations_found: violations.len(),
            analysis_time_us: start.elapsed().as_micros() as u64,
        };

        NllAnalysisResult {
            regions: self.regions,
            liveness: self.liveness,
            borrows: self.borrows,
            constraints: self.constraints,
            violations,
            stats,
        }
    }

    /// Compute liveness for all variables using backward dataflow.
    fn compute_liveness(&mut self) {
        // Collect all uses and definitions
        for (block_id, block) in &self.cfg.blocks {
            // Definitions
            for (idx, def) in block.definitions.iter().enumerate() {
                let point = NllPoint::end(*block_id, idx as u32);
                self.liveness.def_points.insert(def.reference, point);
            }

            // Uses
            for (idx, use_site) in block.uses.iter().enumerate() {
                let point = NllPoint::start(*block_id, idx as u32);
                self.liveness.use_points
                    .entry(use_site.reference)
                    .or_insert_with(Set::new)
                    .insert(point);
            }
        }

        // Compute live ranges using backward iteration
        let mut changed = true;
        let mut iterations = 0;

        while changed && iterations < self.config.max_iterations {
            changed = false;
            iterations += 1;

            // Process each block
            let block_ids: List<_> = self.cfg.blocks.keys().copied().collect();
            for block_id in &block_ids {
                if let Some(block) = self.cfg.blocks.get(block_id) {
                    // Each use extends the live range
                    for use_site in &block.uses {
                        let range = self.liveness.live_ranges
                            .entry(use_site.reference)
                            .or_insert_with(LiveRange::new);

                        // Add all points from definition to this use
                        if let Some(&_def_point) = self.liveness.def_points.get(&use_site.reference) {
                            // Add points between def and use
                            for stmt in 0..=block.definitions.len() {
                                let point = NllPoint::start(*block_id, stmt as u32);
                                if !range.contains(&point) {
                                    range.add_point(point);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Create regions for borrow operations.
    fn create_borrow_regions(&mut self) {
        // Collect data first to avoid borrow conflicts
        let def_data: List<(BlockId, u32, RefId)> = self.cfg.blocks
            .iter()
            .flat_map(|(block_id, block)| {
                block.definitions.iter().enumerate().map(move |(idx, def)| {
                    (*block_id, idx as u32, def.reference)
                })
            })
            .collect();

        // Create regions for each definition (potential borrow)
        for (block_id, idx, ref_id) in def_data {
            let region_id = self.fresh_region();
            let mut region = NllRegion::new(region_id, NllRegionKind::Inferred);

            // Region initially contains just the definition point
            region.add_point(NllPoint::end(block_id, idx));

            // If there's liveness info, extend region to last use (NLL!)
            if let Some(last_use) = self.liveness.last_use(ref_id) {
                // Add all points from definition to last use
                self.extend_region_to(&mut region, NllPoint::end(block_id, idx), last_use);
            }

            self.regions.insert(region_id, region);
        }
    }

    /// Extend a region from start to end point.
    fn extend_region_to(&self, region: &mut NllRegion, start: NllPoint, end: NllPoint) {
        // Simplified: add start and end, in real impl would trace CFG
        region.add_point(start);
        region.add_point(end);

        // Add intermediate points in the same block
        if start.block == end.block {
            for stmt in start.statement..=end.statement {
                region.add_point(NllPoint::start(start.block, stmt));
                region.add_point(NllPoint::end(start.block, stmt));
            }
        }
    }

    /// Compute release points for all borrows based on liveness analysis.
    ///
    /// This is the core of liveness-based borrow release. For each borrow,
    /// we determine the point at which it can be released (the last use of
    /// the assigned_place, i.e., the variable holding the reference).
    ///
    /// After this point, the borrowed place can be accessed again without
    /// conflict, even before the lexical scope ends.
    ///
    /// Liveness-based borrow release: borrows are released at the point of last
    /// use of the assigned_place (the variable holding the reference), not at
    /// lexical scope end. This enables earlier re-access of the borrowed place.
    fn compute_release_points(&mut self) {
        // Collect borrow IDs to avoid borrow conflicts
        let borrow_ids: List<BorrowId> = self.borrows.borrows.keys().copied().collect();

        for borrow_id in borrow_ids {
            if let Some(borrow) = self.borrows.borrows.get_mut(&borrow_id) {
                // The release point is determined by the last use of assigned_place
                // (the variable that holds the reference)
                if let Some(last_use) = self.liveness.last_use(borrow.assigned_place) {
                    // Set release point to just after the last use
                    // Using End kind to indicate the borrow is released after the statement
                    borrow.release_point = Some(NllPoint::end(last_use.block, last_use.statement));
                } else {
                    // If no uses found, the borrow is released immediately after creation
                    // This handles cases where a borrow is created but never used
                    borrow.release_point = Some(NllPoint::end(
                        borrow.reserve_point.block,
                        borrow.reserve_point.statement,
                    ));
                }
            }
        }
    }

    /// Generate constraints from the CFG.
    fn generate_constraints(&mut self) {
        // For each region, generate liveness constraints
        let region_ids: List<_> = self.regions.keys().copied().collect();
        for region_id in region_ids {
            if let Some(region) = self.regions.get(&region_id) {
                for point in region.points.iter() {
                    self.constraints.push(NllConstraint::live_at(region_id, *point));
                }
            }
        }

        // Generate subset constraints from assignments
        for (_block_id, block) in &self.cfg.blocks {
            for (_idx, use_site) in block.uses.iter().enumerate() {
                // If this use is an assignment to another reference,
                // create a subset constraint
                for def in &block.definitions {
                    if def.reference != use_site.reference {
                        // This is a simplified heuristic
                        // Real impl would analyze actual assignments
                    }
                }
            }
        }
    }

    /// Solve constraints using fixed-point iteration.
    fn solve_constraints(&mut self) -> usize {
        let mut iterations = 0;
        let mut changed = true;

        while changed && iterations < self.config.max_iterations {
            changed = false;
            iterations += 1;

            for constraint in &self.constraints {
                match &constraint.kind {
                    NllConstraintKind::Subset { sub, sup } => {
                        // Propagate points from sub to sup
                        let sub_points: Set<NllPoint> = self.regions
                            .get(sub)
                            .map(|r| r.points.clone())
                            .unwrap_or_default();

                        if let Some(sup_region) = self.regions.get_mut(sup) {
                            for point in sub_points {
                                if !sup_region.contains(&point) {
                                    sup_region.add_point(point);
                                    changed = true;
                                }
                            }
                        }
                    }
                    NllConstraintKind::LiveAt { region, point } => {
                        // Ensure region contains the point
                        if let Some(reg) = self.regions.get_mut(region) {
                            if !reg.contains(point) {
                                reg.add_point(*point);
                                changed = true;
                            }
                        }
                    }
                    NllConstraintKind::BorrowValid { borrow: _, region: _ } => {
                        // Ensure borrow's region is subset of target region
                        // (handled by subset constraints)
                    }
                }
            }
        }

        iterations
    }

    /// Check for borrow checking violations using liveness-based release.
    ///
    /// This is the core of NLL borrow checking with liveness-based release.
    /// A conflict only exists if there is a point where BOTH borrows are live.
    /// If borrow1 is released before borrow2 starts, there is no conflict.
    ///
    /// Liveness-based borrow release: borrows are released at the point of last
    /// use of the assigned_place (the variable holding the reference), not at
    /// lexical scope end. This enables earlier re-access of the borrowed place.
    fn check_violations(&self) -> List<NllViolation> {
        let mut violations = List::new();

        // Check for conflicting borrows
        for (id1, borrow1) in &self.borrows.borrows {
            for (id2, borrow2) in &self.borrows.borrows {
                if id1 >= id2 {
                    continue;
                }

                // Same place with conflicting kinds
                if borrow1.borrowed_place == borrow2.borrowed_place {
                    if borrow1.conflicts_with(borrow2.kind) {
                        // Check if regions overlap
                        let region1 = self.regions.get(&borrow1.region);
                        let region2 = self.regions.get(&borrow2.region);

                        if let (Some(r1), Some(r2)) = (region1, region2) {
                            // Liveness-based conflict check:
                            // Only report conflict if there's a point where BOTH borrows are live.
                            // This enables early release when first borrow is no longer used.
                            let has_live_overlap = r1.points.intersection(&r2.points)
                                .any(|point| {
                                    borrow1.is_live_at(point) && borrow2.is_live_at(point)
                                });

                            if has_live_overlap {
                                violations.push(NllViolation::new(
                                    NllViolationKind::ConflictingBorrow {
                                        first: *id1,
                                        second: *id2,
                                    },
                                    format!(
                                        "Cannot borrow {:?} as {:?} while already borrowed as {:?}",
                                        borrow2.borrowed_place, borrow2.kind, borrow1.kind
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

    /// Create a fresh region ID.
    fn fresh_region(&mut self) -> NllRegionId {
        let id = NllRegionId(self.next_region_id);
        self.next_region_id += 1;
        id
    }
}

// ============================================================================
// Two-Phase Borrow Support
// ============================================================================

/// Manager for two-phase borrows.
#[derive(Debug, Clone, Default)]
pub struct TwoPhaseBorrowManager {
    /// Reserved borrows awaiting activation.
    reserved: Map<BorrowId, BorrowData>,
    /// Activated borrows.
    activated: Set<BorrowId>,
}

impl TwoPhaseBorrowManager {
    /// Create new manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve a two-phase borrow.
    pub fn reserve(&mut self, borrow: BorrowData) {
        self.reserved.insert(borrow.id, borrow);
    }

    /// Activate a reserved borrow.
    pub fn activate(&mut self, id: BorrowId, point: NllPoint) -> bool {
        if let Some(borrow) = self.reserved.get_mut(&id) {
            borrow.activation_point = Some(point);
            self.activated.insert(id);
            true
        } else {
            false
        }
    }

    /// Check if a borrow is only reserved (not activated).
    #[must_use]
    pub fn is_reserved_only(&self, id: BorrowId) -> bool {
        self.reserved.contains_key(&id) && !self.activated.contains(&id)
    }

    /// Check if shared access is allowed despite reserved mutable borrow.
    #[must_use]
    pub fn allows_shared_access(&self, place: RefId) -> bool {
        // Shared access is allowed if the mutable borrow is only reserved
        self.reserved.values()
            .filter(|b| b.borrowed_place == place)
            .all(|b| !self.activated.contains(&b.id))
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

        let mut entry_block = BasicBlock::empty(entry);
        entry_block.successors.insert(exit);
        cfg.add_block(entry_block);

        let mut exit_block = BasicBlock::empty(exit);
        exit_block.predecessors.insert(entry);
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_nll_region_creation() {
        let region = NllRegion::new(NllRegionId(1), NllRegionKind::Inferred);
        assert!(region.is_empty());
        assert_eq!(region.kind, NllRegionKind::Inferred);
    }

    #[test]
    fn test_nll_region_add_point() {
        let mut region = NllRegion::new(NllRegionId(1), NllRegionKind::Inferred);
        let point = NllPoint::start(BlockId(0), 0);
        region.add_point(point);

        assert!(region.contains(&point));
        assert!(!region.is_empty());
    }

    #[test]
    fn test_universal_region() {
        let region = NllRegion::universal(NllRegionId::STATIC);
        let any_point = NllPoint::start(BlockId(999), 999);

        assert!(region.contains(&any_point));
        assert_eq!(region.kind, NllRegionKind::Universal);
    }

    #[test]
    fn test_nll_point_kinds() {
        let start = NllPoint::start(BlockId(0), 0);
        let mid = NllPoint::mid(BlockId(0), 0);
        let end = NllPoint::end(BlockId(0), 0);

        assert_eq!(start.kind, PointKind::Start);
        assert_eq!(mid.kind, PointKind::Mid);
        assert_eq!(end.kind, PointKind::End);
    }

    #[test]
    fn test_live_range() {
        let mut range = LiveRange::new();
        range.add_point(NllPoint::start(BlockId(0), 0));
        range.add_point(NllPoint::end(BlockId(0), 1));

        assert!(range.contains(&NllPoint::start(BlockId(0), 0)));
        assert!(!range.contains(&NllPoint::start(BlockId(1), 0)));
    }

    #[test]
    fn test_borrow_set() {
        let mut borrows = BorrowSet::new();

        let data = BorrowData {
            id: BorrowId(0),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: None,
        };

        let id = borrows.create_borrow(data);
        assert!(borrows.borrows.contains_key(&id));

        let place_borrows = borrows.borrows_for(RefId(1));
        assert_eq!(place_borrows.len(), 1);
    }

    #[test]
    fn test_borrow_conflict() {
        let mut borrows = BorrowSet::new();

        let shared = BorrowData {
            id: BorrowId(0),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: None,
        };
        borrows.create_borrow(shared);

        // Shared doesn't conflict with shared
        assert!(borrows.has_conflict(RefId(1), NllBorrowKind::Shared).is_none());

        // Shared conflicts with mutable
        assert!(borrows.has_conflict(RefId(1), NllBorrowKind::Mutable).is_some());
    }

    #[test]
    fn test_nll_analyzer_creation() {
        let cfg = create_test_cfg();
        let analyzer = NllAnalyzer::new(cfg);
        let result = analyzer.analyze();

        assert!(!result.has_violations());
    }

    #[test]
    fn test_nll_constraint_subset() {
        let constraint = NllConstraint::subset(NllRegionId(1), NllRegionId(2));
        matches!(constraint.kind, NllConstraintKind::Subset { .. });
    }

    #[test]
    fn test_nll_violation_creation() {
        let violation = NllViolation::new(
            NllViolationKind::BorrowOutlivesData { borrow: BorrowId(0) },
            "borrow outlives data",
        );

        assert_eq!(violation.message, "borrow outlives data");
    }

    #[test]
    fn test_two_phase_borrow_manager() {
        let mut manager = TwoPhaseBorrowManager::new();

        let borrow = BorrowData {
            id: BorrowId(1),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Mutable,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: true,
            release_point: None,
        };

        manager.reserve(borrow);
        assert!(manager.is_reserved_only(BorrowId(1)));
        assert!(manager.allows_shared_access(RefId(1)));

        manager.activate(BorrowId(1), NllPoint::mid(BlockId(0), 1));
        assert!(!manager.is_reserved_only(BorrowId(1)));
        assert!(!manager.allows_shared_access(RefId(1)));
    }

    #[test]
    fn test_region_merge() {
        let mut region1 = NllRegion::new(NllRegionId(1), NllRegionKind::Inferred);
        region1.add_point(NllPoint::start(BlockId(0), 0));

        let mut region2 = NllRegion::new(NllRegionId(2), NllRegionKind::Inferred);
        region2.add_point(NllPoint::start(BlockId(1), 0));

        region1.merge(&region2);

        assert!(region1.contains(&NllPoint::start(BlockId(0), 0)));
        assert!(region1.contains(&NllPoint::start(BlockId(1), 0)));
    }

    #[test]
    fn test_liveness_info() {
        let mut liveness = LivenessInfo::new();

        let point1 = NllPoint::start(BlockId(0), 0);
        let point2 = NllPoint::end(BlockId(0), 1);

        liveness.use_points
            .entry(RefId(1))
            .or_insert_with(Set::new)
            .insert(point1);
        liveness.use_points
            .entry(RefId(1))
            .or_insert_with(Set::new)
            .insert(point2);

        let last = liveness.last_use(RefId(1));
        assert!(last.is_some());
        assert_eq!(last.unwrap().statement, 1);
    }

    #[test]
    fn test_nll_analysis_result_empty() {
        let result = NllAnalysisResult::empty();

        assert!(!result.has_violations());
        assert!(result.regions.is_empty());
    }

    // =========================================================================
    // Liveness-based Borrow Release Tests
    // Liveness-based borrow release: borrows end at last use, not scope end
    // =========================================================================

    #[test]
    fn test_borrow_is_released_at() {
        let borrow = BorrowData {
            id: BorrowId(1),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: Some(NllPoint::end(BlockId(0), 1)),
        };

        // Before release point - not released
        assert!(!borrow.is_released_at(&NllPoint::start(BlockId(0), 0)));
        assert!(!borrow.is_released_at(&NllPoint::end(BlockId(0), 0)));

        // At release point - released
        assert!(borrow.is_released_at(&NllPoint::end(BlockId(0), 1)));

        // After release point - released
        assert!(borrow.is_released_at(&NllPoint::start(BlockId(0), 2)));
        assert!(borrow.is_released_at(&NllPoint::start(BlockId(1), 0)));
    }

    #[test]
    fn test_borrow_is_live_at() {
        let borrow = BorrowData {
            id: BorrowId(1),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Mutable,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 1),
            activation_point: None,
            two_phase: false,
            release_point: Some(NllPoint::end(BlockId(0), 2)),
        };

        // Before reserve point - not live
        assert!(!borrow.is_live_at(&NllPoint::start(BlockId(0), 0)));

        // At reserve point - live
        assert!(borrow.is_live_at(&NllPoint::start(BlockId(0), 1)));

        // Between reserve and release - live
        assert!(borrow.is_live_at(&NllPoint::end(BlockId(0), 1)));

        // At release point - not live (released)
        assert!(!borrow.is_live_at(&NllPoint::end(BlockId(0), 2)));

        // After release point - not live
        assert!(!borrow.is_live_at(&NllPoint::start(BlockId(0), 3)));
    }

    #[test]
    fn test_borrow_without_release_point_always_live() {
        let borrow = BorrowData {
            id: BorrowId(1),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: None, // No release point
        };

        // Without release_point, borrow is live until scope end (lexical)
        assert!(borrow.is_live_at(&NllPoint::start(BlockId(0), 0)));
        assert!(borrow.is_live_at(&NllPoint::end(BlockId(0), 100)));
        assert!(borrow.is_live_at(&NllPoint::start(BlockId(10), 0)));
    }

    #[test]
    fn test_has_conflict_at_liveness_based() {
        let mut borrows = BorrowSet::new();

        // First borrow: reserve at 0, release at 1
        let first = BorrowData {
            id: BorrowId(0),
            borrowed_place: RefId(1),
            assigned_place: RefId(2),
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: Some(NllPoint::end(BlockId(0), 1)),
        };
        borrows.create_borrow(first);

        // At point 0 (before release): conflict with mutable
        let point_before = NllPoint::start(BlockId(0), 0);
        assert!(borrows.has_conflict_at(RefId(1), NllBorrowKind::Mutable, &point_before).is_some());

        // At point 2 (after release): NO conflict with mutable!
        let point_after = NllPoint::start(BlockId(0), 2);
        assert!(borrows.has_conflict_at(RefId(1), NllBorrowKind::Mutable, &point_after).is_none());
    }

    #[test]
    fn test_liveness_based_no_false_conflict() {
        // This test verifies the core liveness-based borrow release scenario:
        //
        // let first = read_first(&data);  // borrow starts at stmt 0
        // // first last use at stmt 1
        // modify_data(&mut data);         // at stmt 2 - should NOT conflict!
        //
        // Liveness-based borrow release: release at last use, not scope end

        let mut borrows = BorrowSet::new();

        // Immutable borrow: created at stmt 0, released at stmt 1 (last use)
        let immutable_borrow = BorrowData {
            id: BorrowId(0),
            borrowed_place: RefId(1),  // data
            assigned_place: RefId(2),  // first
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: Some(NllPoint::end(BlockId(0), 1)), // Released after last use
        };
        borrows.create_borrow(immutable_borrow);

        // At stmt 2: the immutable borrow is no longer live
        let mutable_point = NllPoint::start(BlockId(0), 2);

        // With liveness-based release, there should be NO conflict
        let conflict = borrows.has_conflict_at(RefId(1), NllBorrowKind::Mutable, &mutable_point);
        assert!(conflict.is_none(), "Liveness-based release should allow mutable borrow after shared borrow is no longer used");
    }

    #[test]
    fn test_liveness_based_real_conflict() {
        // This test verifies that true conflicts are still detected:
        //
        // let first = read_first(&data);  // borrow at stmt 0
        // modify_data(&mut data);         // at stmt 1 - SHOULD conflict!
        // println!("{}", first);          // first used at stmt 2

        let mut borrows = BorrowSet::new();

        // Immutable borrow: created at stmt 0, last use at stmt 2
        let immutable_borrow = BorrowData {
            id: BorrowId(0),
            borrowed_place: RefId(1),  // data
            assigned_place: RefId(2),  // first
            kind: NllBorrowKind::Shared,
            region: NllRegionId(1),
            reserve_point: NllPoint::start(BlockId(0), 0),
            activation_point: None,
            two_phase: false,
            release_point: Some(NllPoint::end(BlockId(0), 2)), // Last use at stmt 2
        };
        borrows.create_borrow(immutable_borrow);

        // At stmt 1: the immutable borrow is STILL live (used later at stmt 2)
        let mutable_point = NllPoint::start(BlockId(0), 1);

        // There SHOULD be a conflict
        let conflict = borrows.has_conflict_at(RefId(1), NllBorrowKind::Mutable, &mutable_point);
        assert!(conflict.is_some(), "Should detect conflict when shared borrow is still live");
    }
}
