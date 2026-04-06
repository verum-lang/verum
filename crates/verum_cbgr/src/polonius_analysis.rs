//! Polonius-Style Origin Analysis
//!
//! This module implements a Polonius-inspired borrow checking algorithm that uses
//! a Datalog-style approach for more precise and permissive analysis. Polonius is
//! the next generation of Rust's borrow checker, offering:
//!
//! - **Better error messages**: More precise location of where borrows conflict
//! - **More permissive analysis**: Accepts more programs that NLL rejects
//! - **Location-sensitive analysis**: Borrows tracked at each program point
//! - **Datalog semantics**: Clear, declarative specification
//!
//! # Key Concepts
//!
//! ## Origins (Lifetimes)
//!
//! In Polonius, "origins" replace traditional lifetimes. An origin represents
//! the set of loans (borrows) that a reference might contain:
//!
//! ```text
//! let x = 5;
//! let r: &'a i32 = &x;  // Origin 'a contains the loan for x
//! ```
//!
//! ## Loans
//!
//! A loan represents a borrow of a specific place at a specific point:
//!
//! ```text
//! Loan { place: x, point: P1, kind: Shared }
//! ```
//!
//! ## Facts
//!
//! The analysis is expressed as Datalog-style facts and rules:
//!
//! - `loan_issued_at(origin, loan, point)`: A loan was created
//! - `origin_live_on_entry(origin, point)`: Origin is live at point
//! - `loan_invalidated_at(loan, point)`: A loan becomes invalid
//! - `errors(loan, point)`: Detected borrow error
//!
//! # Algorithm
//!
//! 1. Generate input facts from CFG
//! 2. Apply Datalog rules to compute derived facts
//! 3. Check for `errors` facts
//!
//! # Example Rules
//!
//! ```datalog
//! // A loan is live if its origin is live
//! loan_live_at(Loan, Point) :-
//!     origin_live_on_entry(Origin, Point),
//!     loan_issued_at(Origin, Loan, _).
//!
//! // An error occurs if a live loan is invalidated
//! errors(Loan, Point) :-
//!     loan_live_at(Loan, Point),
//!     loan_invalidated_at(Loan, Point).
//! ```
//!
//! Spec: Based on Polonius (https://github.com/rust-lang/polonius)

use crate::analysis::{BlockId, ControlFlowGraph, RefId};
use verum_common::{List, Map, Set};

// ============================================================================
// Core Types
// ============================================================================

/// Unique identifier for an origin (lifetime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OriginId(pub u64);

impl OriginId {
    /// The static origin.
    pub const STATIC: OriginId = OriginId(0);

    /// Create from an index.
    #[must_use]
    pub fn from_index(idx: u64) -> Self {
        Self(idx + 1)
    }
}

/// Unique identifier for a loan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoanId(pub u64);

/// A program point for Polonius analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PoloniusPoint {
    /// Block ID.
    pub block: BlockId,
    /// Statement index.
    pub statement: u32,
    /// Sub-point (Start = 0, Mid = 1, End = 2).
    pub sub: u8,
}

impl PoloniusPoint {
    /// Create a new point.
    #[must_use]
    pub fn new(block: BlockId, statement: u32, sub: u8) -> Self {
        Self { block, statement, sub }
    }

    /// Start of statement.
    #[must_use]
    pub fn start(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, 0)
    }

    /// Mid-point.
    #[must_use]
    pub fn mid(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, 1)
    }

    /// End of statement.
    #[must_use]
    pub fn end(block: BlockId, statement: u32) -> Self {
        Self::new(block, statement, 2)
    }
}

/// A loan representing a borrow.
#[derive(Debug, Clone)]
pub struct Loan {
    /// Unique identifier.
    pub id: LoanId,
    /// Place being borrowed.
    pub place: RefId,
    /// Point where loan was created.
    pub issued_at: PoloniusPoint,
    /// Kind of loan.
    pub kind: LoanKind,
    /// Origin containing this loan.
    pub origin: OriginId,
}

/// Kind of loan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanKind {
    /// Shared (immutable) loan.
    Shared,
    /// Mutable loan.
    Mutable,
}

// ============================================================================
// Input Facts
// ============================================================================

/// Input facts for Polonius analysis.
#[derive(Debug, Clone, Default)]
pub struct InputFacts {
    /// loan_issued_at(origin, loan, point): A loan was issued.
    pub loan_issued_at: Set<(OriginId, LoanId, PoloniusPoint)>,

    /// origin_live_on_entry(origin, point): An origin is live at a point.
    pub origin_live_on_entry: Set<(OriginId, PoloniusPoint)>,

    /// loan_invalidated_at(loan, point): A loan is invalidated.
    pub loan_invalidated_at: Set<(LoanId, PoloniusPoint)>,

    /// loan_killed_at(loan, point): A loan is killed (no longer in scope).
    pub loan_killed_at: Set<(LoanId, PoloniusPoint)>,

    /// origin_contains_loan_on_entry(origin, loan, point): Origin contains loan.
    pub origin_contains_loan_on_entry: Set<(OriginId, LoanId, PoloniusPoint)>,

    /// cfg_edge(from_point, to_point): Control flow edge.
    pub cfg_edge: Set<(PoloniusPoint, PoloniusPoint)>,

    /// subset(origin1, origin2, point): origin1 ⊆ origin2 at point.
    pub subset: Set<(OriginId, OriginId, PoloniusPoint)>,

    /// placeholder(origin, loan): Origin is a placeholder for loan.
    pub placeholder: Set<(OriginId, LoanId)>,
}

impl InputFacts {
    /// Create empty input facts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a loan issued fact.
    pub fn add_loan_issued(&mut self, origin: OriginId, loan: LoanId, point: PoloniusPoint) {
        self.loan_issued_at.insert((origin, loan, point));
    }

    /// Add an origin live fact.
    pub fn add_origin_live(&mut self, origin: OriginId, point: PoloniusPoint) {
        self.origin_live_on_entry.insert((origin, point));
    }

    /// Add a loan invalidated fact.
    pub fn add_loan_invalidated(&mut self, loan: LoanId, point: PoloniusPoint) {
        self.loan_invalidated_at.insert((loan, point));
    }

    /// Add a CFG edge.
    pub fn add_cfg_edge(&mut self, from: PoloniusPoint, to: PoloniusPoint) {
        self.cfg_edge.insert((from, to));
    }

    /// Add a subset constraint.
    pub fn add_subset(&mut self, sub: OriginId, sup: OriginId, point: PoloniusPoint) {
        self.subset.insert((sub, sup, point));
    }
}

// ============================================================================
// Output Facts
// ============================================================================

/// Output facts computed by Polonius analysis.
#[derive(Debug, Clone, Default)]
pub struct OutputFacts {
    /// loan_live_at(loan, point): A loan is live at a point.
    pub loan_live_at: Set<(LoanId, PoloniusPoint)>,

    /// origin_contains_loan_at(origin, loan, point): Origin contains loan at point.
    pub origin_contains_loan_at: Set<(OriginId, LoanId, PoloniusPoint)>,

    /// errors(loan, point): Borrow error detected.
    pub errors: Set<(LoanId, PoloniusPoint)>,

    /// subset_anywhere(origin1, origin2): origin1 ⊆ origin2 somewhere.
    pub subset_anywhere: Set<(OriginId, OriginId)>,
}

impl OutputFacts {
    /// Create empty output facts.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

// ============================================================================
// Polonius Error
// ============================================================================

/// A Polonius borrow checking error.
#[derive(Debug, Clone)]
pub struct PoloniusError {
    /// The loan involved.
    pub loan: LoanId,
    /// Where the error occurred.
    pub point: PoloniusPoint,
    /// Kind of error.
    pub kind: PoloniusErrorKind,
    /// Message.
    pub message: String,
}

impl PoloniusError {
    /// Create a new error.
    #[must_use]
    pub fn new(loan: LoanId, point: PoloniusPoint, kind: PoloniusErrorKind, message: impl Into<String>) -> Self {
        Self {
            loan,
            point,
            kind,
            message: message.into(),
        }
    }
}

/// Kind of Polonius error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoloniusErrorKind {
    /// Use of invalidated loan.
    InvalidatedLoan,
    /// Conflicting loans.
    ConflictingLoans,
    /// Loan escapes.
    LoanEscapes,
    /// Move while borrowed.
    MoveWhileBorrowed,
}

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of Polonius analysis.
#[derive(Debug, Clone)]
pub struct PoloniusAnalysisResult {
    /// Input facts.
    pub input: InputFacts,
    /// Output facts.
    pub output: OutputFacts,
    /// All loans.
    pub loans: Map<LoanId, Loan>,
    /// Detected errors.
    pub errors: List<PoloniusError>,
    /// Statistics.
    pub stats: PoloniusStats,
}

impl PoloniusAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            input: InputFacts::new(),
            output: OutputFacts::new(),
            loans: Map::new(),
            errors: List::new(),
            stats: PoloniusStats::default(),
        }
    }

    /// Check if analysis found errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || self.output.has_errors()
    }
}

/// Statistics from Polonius analysis.
#[derive(Debug, Clone, Default)]
pub struct PoloniusStats {
    /// Number of loans.
    pub loans: usize,
    /// Number of origins.
    pub origins: usize,
    /// Number of input facts.
    pub input_facts: usize,
    /// Number of output facts.
    pub output_facts: usize,
    /// Number of iterations.
    pub iterations: usize,
    /// Analysis time in microseconds.
    pub analysis_time_us: u64,
}

// ============================================================================
// Polonius Analyzer
// ============================================================================

/// Polonius-style borrow checker analyzer.
pub struct PoloniusAnalyzer {
    /// Control flow graph.
    cfg: ControlFlowGraph,
    /// Configuration.
    config: PoloniusConfig,
    /// Input facts.
    input: InputFacts,
    /// Output facts.
    output: OutputFacts,
    /// All loans.
    loans: Map<LoanId, Loan>,
    /// Next loan ID.
    next_loan_id: u64,
    /// Next origin ID.
    next_origin_id: u64,
}

/// Configuration for Polonius analysis.
#[derive(Debug, Clone)]
pub struct PoloniusConfig {
    /// Maximum iterations for fixed-point.
    pub max_iterations: usize,
    /// Enable location-sensitive analysis.
    pub location_sensitive: bool,
    /// Enable move checking.
    pub check_moves: bool,
}

impl Default for PoloniusConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1000,
            location_sensitive: true,
            check_moves: true,
        }
    }
}

impl PoloniusAnalyzer {
    /// Create new Polonius analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        Self {
            cfg,
            config: PoloniusConfig::default(),
            input: InputFacts::new(),
            output: OutputFacts::new(),
            loans: Map::new(),
            next_loan_id: 0,
            next_origin_id: 1, // 0 is reserved for static
        }
    }

    /// Create with configuration.
    #[must_use]
    pub fn with_config(mut self, config: PoloniusConfig) -> Self {
        self.config = config;
        self
    }

    /// Perform Polonius analysis.
    #[must_use]
    pub fn analyze(mut self) -> PoloniusAnalysisResult {
        let start = std::time::Instant::now();

        // Phase 1: Generate input facts from CFG
        self.generate_input_facts();

        // Phase 2: Compute output facts (Datalog-style fixed-point)
        let iterations = self.compute_output_facts();

        // Phase 3: Extract errors
        let errors = self.extract_errors();

        // Build statistics
        let stats = PoloniusStats {
            loans: self.loans.len(),
            origins: self.next_origin_id as usize,
            input_facts: self.count_input_facts(),
            output_facts: self.count_output_facts(),
            iterations,
            analysis_time_us: start.elapsed().as_micros() as u64,
        };

        PoloniusAnalysisResult {
            input: self.input,
            output: self.output,
            loans: self.loans,
            errors,
            stats,
        }
    }

    /// Generate input facts from the CFG.
    fn generate_input_facts(&mut self) {
        // Generate CFG edges
        for (block_id, block) in &self.cfg.blocks {
            // Intra-block edges
            let num_statements = block.definitions.len().max(block.uses.len());
            for i in 0..num_statements {
                let from = PoloniusPoint::end(*block_id, i as u32);
                let to = PoloniusPoint::start(*block_id, (i + 1) as u32);
                self.input.add_cfg_edge(from, to);
            }

            // Inter-block edges
            for &succ in &block.successors {
                let from = PoloniusPoint::end(*block_id, num_statements as u32);
                let to = PoloniusPoint::start(succ, 0);
                self.input.add_cfg_edge(from, to);
            }
        }

        // Collect data first to avoid borrow conflicts
        let def_data: List<(BlockId, u32, RefId)> = self.cfg.blocks
            .iter()
            .flat_map(|(block_id, block)| {
                block.definitions.iter().enumerate().map(move |(idx, def)| {
                    (*block_id, idx as u32, def.reference)
                })
            })
            .collect();

        // Generate loan facts for definitions (potential borrows)
        for (block_id, idx, ref_id) in def_data {
            let loan_id = self.fresh_loan_id();
            let origin_id = self.fresh_origin_id();
            let point = PoloniusPoint::end(block_id, idx);

            // Create the loan
            let loan = Loan {
                id: loan_id,
                place: ref_id,
                issued_at: point,
                kind: LoanKind::Shared, // Default, would be determined by context
                origin: origin_id,
            };
            self.loans.insert(loan_id, loan);

            // Add input facts
            self.input.add_loan_issued(origin_id, loan_id, point);
            self.input.add_origin_live(origin_id, point);
        }
    }

    /// Compute output facts using Datalog-style fixed-point iteration.
    fn compute_output_facts(&mut self) -> usize {
        let mut iterations = 0;
        let mut changed = true;

        while changed && iterations < self.config.max_iterations {
            changed = false;
            iterations += 1;

            // Rule 1: loan_live_at(Loan, Point) :-
            //           origin_live_on_entry(Origin, Point),
            //           loan_issued_at(Origin, Loan, _).
            for (origin, point) in self.input.origin_live_on_entry.iter() {
                for (o, loan, _) in self.input.loan_issued_at.iter() {
                    if o == origin {
                        if self.output.loan_live_at.insert((*loan, *point)) {
                            changed = true;
                        }
                    }
                }
            }

            // Rule 2: origin_contains_loan_at(Origin, Loan, Point) :-
            //           loan_issued_at(Origin, Loan, Point).
            for (origin, loan, point) in self.input.loan_issued_at.iter() {
                if self.output.origin_contains_loan_at.insert((*origin, *loan, *point)) {
                    changed = true;
                }
            }

            // Rule 3: origin_contains_loan_at propagation through CFG
            let current: List<_> = self.output.origin_contains_loan_at.iter().cloned().collect();
            for (origin, loan, point) in current {
                for (from, to) in self.input.cfg_edge.iter() {
                    if *from == point {
                        // Check if loan is killed
                        if !self.input.loan_killed_at.contains(&(loan, *to)) {
                            if self.output.origin_contains_loan_at.insert((origin, loan, *to)) {
                                changed = true;
                            }
                        }
                    }
                }
            }

            // Rule 4: errors(Loan, Point) :-
            //           loan_live_at(Loan, Point),
            //           loan_invalidated_at(Loan, Point).
            let live_loans: List<_> = self.output.loan_live_at.iter().cloned().collect();
            for (loan, point) in live_loans {
                if self.input.loan_invalidated_at.contains(&(loan, point)) {
                    if self.output.errors.insert((loan, point)) {
                        changed = true;
                    }
                }
            }

            // Rule 5: subset propagation
            for (sub, sup, point) in self.input.subset.iter() {
                if self.output.subset_anywhere.insert((*sub, *sup)) {
                    changed = true;
                }

                // Propagate loans through subset
                let sub_loans: List<_> = self.output.origin_contains_loan_at
                    .iter()
                    .filter(|(o, _, p)| o == sub && p == point)
                    .map(|(_, l, _)| *l)
                    .collect();

                for loan in sub_loans {
                    if self.output.origin_contains_loan_at.insert((*sup, loan, *point)) {
                        changed = true;
                    }
                }
            }
        }

        iterations
    }

    /// Extract errors from output facts.
    fn extract_errors(&self) -> List<PoloniusError> {
        let mut errors = List::new();

        for (loan, point) in &self.output.errors {
            let error = PoloniusError::new(
                *loan,
                *point,
                PoloniusErrorKind::InvalidatedLoan,
                format!("Loan {:?} used after invalidation at {:?}", loan, point),
            );
            errors.push(error);
        }

        errors
    }

    /// Create a fresh loan ID.
    fn fresh_loan_id(&mut self) -> LoanId {
        let id = LoanId(self.next_loan_id);
        self.next_loan_id += 1;
        id
    }

    /// Create a fresh origin ID.
    fn fresh_origin_id(&mut self) -> OriginId {
        let id = OriginId(self.next_origin_id);
        self.next_origin_id += 1;
        id
    }

    /// Count input facts.
    fn count_input_facts(&self) -> usize {
        self.input.loan_issued_at.len()
            + self.input.origin_live_on_entry.len()
            + self.input.loan_invalidated_at.len()
            + self.input.loan_killed_at.len()
            + self.input.origin_contains_loan_on_entry.len()
            + self.input.cfg_edge.len()
            + self.input.subset.len()
            + self.input.placeholder.len()
    }

    /// Count output facts.
    fn count_output_facts(&self) -> usize {
        self.output.loan_live_at.len()
            + self.output.origin_contains_loan_at.len()
            + self.output.errors.len()
            + self.output.subset_anywhere.len()
    }
}

// ============================================================================
// Move Analysis Integration
// ============================================================================

/// Move tracking for Polonius.
#[derive(Debug, Clone, Default)]
pub struct MoveTracker {
    /// Moved places.
    moved: Map<RefId, PoloniusPoint>,
    /// Partially moved places.
    partially_moved: Map<RefId, Set<PoloniusPoint>>,
}

impl MoveTracker {
    /// Create new move tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a move.
    pub fn record_move(&mut self, place: RefId, point: PoloniusPoint) {
        self.moved.insert(place, point);
    }

    /// Record a partial move.
    pub fn record_partial_move(&mut self, place: RefId, point: PoloniusPoint) {
        self.partially_moved
            .entry(place)
            .or_insert_with(Set::new)
            .insert(point);
    }

    /// Check if a place is moved at a point.
    #[must_use]
    pub fn is_moved_at(&self, place: RefId, point: PoloniusPoint) -> bool {
        self.moved.get(&place).map_or(false, |&p| p <= point)
    }

    /// Check if a place is usable at a point.
    #[must_use]
    pub fn is_usable(&self, place: RefId, point: PoloniusPoint) -> bool {
        !self.is_moved_at(place, point)
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
    fn test_origin_id() {
        assert_eq!(OriginId::STATIC, OriginId(0));
        assert_eq!(OriginId::from_index(0), OriginId(1));
    }

    #[test]
    fn test_polonius_point() {
        let p1 = PoloniusPoint::start(BlockId(0), 0);
        let p2 = PoloniusPoint::mid(BlockId(0), 0);
        let p3 = PoloniusPoint::end(BlockId(0), 0);

        assert_eq!(p1.sub, 0);
        assert_eq!(p2.sub, 1);
        assert_eq!(p3.sub, 2);
    }

    #[test]
    fn test_input_facts() {
        let mut facts = InputFacts::new();

        let origin = OriginId(1);
        let loan = LoanId(1);
        let point = PoloniusPoint::start(BlockId(0), 0);

        facts.add_loan_issued(origin, loan, point);
        facts.add_origin_live(origin, point);

        assert!(facts.loan_issued_at.contains(&(origin, loan, point)));
        assert!(facts.origin_live_on_entry.contains(&(origin, point)));
    }

    #[test]
    fn test_output_facts() {
        let mut facts = OutputFacts::new();

        assert!(!facts.has_errors());

        facts.errors.insert((LoanId(0), PoloniusPoint::start(BlockId(0), 0)));
        assert!(facts.has_errors());
    }

    #[test]
    fn test_polonius_analyzer_creation() {
        let cfg = create_test_cfg();
        let analyzer = PoloniusAnalyzer::new(cfg);
        let result = analyzer.analyze();

        assert!(!result.has_errors());
    }

    #[test]
    fn test_loan_creation() {
        let loan = Loan {
            id: LoanId(0),
            place: RefId(1),
            issued_at: PoloniusPoint::start(BlockId(0), 0),
            kind: LoanKind::Shared,
            origin: OriginId(1),
        };

        assert_eq!(loan.kind, LoanKind::Shared);
    }

    #[test]
    fn test_polonius_error_creation() {
        let error = PoloniusError::new(
            LoanId(0),
            PoloniusPoint::start(BlockId(0), 0),
            PoloniusErrorKind::InvalidatedLoan,
            "loan invalidated",
        );

        assert_eq!(error.kind, PoloniusErrorKind::InvalidatedLoan);
    }

    #[test]
    fn test_move_tracker() {
        let mut tracker = MoveTracker::new();

        let place = RefId(1);
        let point1 = PoloniusPoint::start(BlockId(0), 0);
        let point2 = PoloniusPoint::start(BlockId(0), 1);

        tracker.record_move(place, point1);

        assert!(tracker.is_moved_at(place, point2));
        assert!(!tracker.is_usable(place, point2));
    }

    #[test]
    fn test_cfg_edge_generation() {
        let cfg = create_test_cfg();
        let analyzer = PoloniusAnalyzer::new(cfg);
        let result = analyzer.analyze();

        // Should have at least one CFG edge between entry and exit
        assert!(!result.input.cfg_edge.is_empty());
    }

    #[test]
    fn test_polonius_result_empty() {
        let result = PoloniusAnalysisResult::empty();

        assert!(!result.has_errors());
        assert!(result.loans.is_empty());
    }

    #[test]
    fn test_subset_fact() {
        let mut facts = InputFacts::new();

        let origin1 = OriginId(1);
        let origin2 = OriginId(2);
        let point = PoloniusPoint::start(BlockId(0), 0);

        facts.add_subset(origin1, origin2, point);
        assert!(facts.subset.contains(&(origin1, origin2, point)));
    }

    #[test]
    fn test_loan_kind_variants() {
        assert_eq!(LoanKind::Shared, LoanKind::Shared);
        assert_ne!(LoanKind::Shared, LoanKind::Mutable);
    }
}
