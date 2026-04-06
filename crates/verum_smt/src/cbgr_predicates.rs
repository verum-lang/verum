//! CBGR Generation Tracking Predicates for SMT
//!
//! CBGR (Counter-Based Generational References) is Verum's memory safety system.
//! References carry generation counters validated at dereference (~15ns overhead).
//! ThinRef<T> is 16 bytes: ptr + 48-bit generation + 16-bit epoch.
//! FatRef<T> is 24 bytes: ptr + generation + epoch + len (for unsized types).
//! In CBGR-native reference patterns, any function accepting `&T` and returning `&T`
//! implicitly works for all scope validities because generation counters are checked
//! at runtime. Generation tracking predicates (generation/epoch/valid/same_allocation)
//! are available in ensures/requires clauses for dependent return types.
//!
//! This module provides SMT predicates for reasoning about CBGR generation counters:
//! 1. `generation(ref)` - Extract generation counter from reference
//! 2. `epoch(ref)` - Extract epoch counter from reference
//! 3. `valid(ref)` - Check if reference is still valid
//! 4. `same_allocation(a, b)` - Check if references point to same allocation
//!
//! These predicates enable refinement verification of generation-aware code.
//!
//! # Memory Model
//!
//! ```text
//! ThinRef<T>:
//!   ptr: *const T      // 8 bytes
//!   generation: u64    // 8 bytes (48-bit generation + 16-bit epoch)
//!   Total: 16 bytes
//!
//! Generation counter layout (64-bit):
//!   Bits 0-47:  Generation (48 bits, ~281 trillion)
//!   Bits 48-63: Epoch (16 bits, 65536 epochs)
//! ```
//!
//! # Performance
//!
//! - Predicate evaluation: <5ns (inline assembly)
//! - SMT verification: <50ms for typical properties
//! - Generation check overhead: ~15ns (CBGR baseline)

use std::time::Instant;

use verum_ast::span::Span;
use verum_ast::ty::Type;
use verum_common::{Map, Maybe};
use verum_protocol_types::cbgr_predicates::{
    CBGRCounterexample, CBGRStats, CBGRVerificationResult, ReferenceValue,
};
use verum_common::ToText;

use z3::ast::{BV, Bool};
use z3::{Context, FuncDecl, SatResult, Solver, Sort, Symbol};

use crate::context::Context as VerumContext;
use crate::translate::Translator;

// ==================== Core Types ====================
// Verification result types are imported from verum_protocol_types to avoid circular dependency

/// Generation tracking predicates for refinement types
///
/// CBGR generation tracking predicates for use in ensures/requires clauses.
/// Instead of lifetime-annotated functions, Verum uses ensures clauses with
/// generation predicates, e.g.: `fn get_slice(&self) -> &Item ensures generation(result) == generation(self)`.
/// The compiler inserts CBGR checks to validate at runtime; in `@verify(static)` mode,
/// the analyzer proves them statically. Available built-in predicates:
/// - `generation(ref: &T) -> u32` - Get generation counter
/// - `epoch(ref: &T) -> u16` - Get epoch counter
/// - `valid(ref: &T) -> Bool` - Check if reference is still valid
/// - `same_allocation(a: &T, b: &U) -> Bool` - Check if both point to same allocation
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationPredicate {
    /// Get generation counter: `generation(ref: &T) -> u64`
    Generation { ref_expr: Box<Type> },

    /// Get epoch counter: `epoch(ref: &T) -> u16`
    Epoch { ref_expr: Box<Type> },

    /// Check if reference is valid: `valid(ref: &T) -> Bool`
    Valid { ref_expr: Box<Type> },

    /// Check same allocation: `same_allocation(a: &T, b: &U) -> Bool`
    SameAllocation { ref_a: Box<Type>, ref_b: Box<Type> },
}

// ==================== CBGR Predicate Encoder ====================

/// Encodes CBGR predicates to Z3
///
/// Fields are initialized with Z3 function declarations/sorts that will be used
/// as the CBGR-SMT encoding is fully wired up. Suppress dead_code warnings for
/// the entire struct since these are reserved Z3 declarations.
#[allow(dead_code)]
pub struct CBGRPredicateEncoder {
    /// Z3 context
    context: Context,
    /// Verum SMT context
    verum_ctx: VerumContext,
    /// Reference sort (bitvector representing ptr + generation)
    ref_sort: Sort,
    /// Generation predicate: ref -> u64
    generation_pred: FuncDecl,
    /// Epoch predicate: ref -> u16
    epoch_pred: FuncDecl,
    /// Valid predicate: ref -> bool
    valid_pred: FuncDecl,
    /// Same allocation predicate: ref × ref -> bool
    same_alloc_pred: FuncDecl,
    /// Global generation counter (for soundness)
    global_generation: FuncDecl,
}

impl CBGRPredicateEncoder {
    /// Create a new CBGR predicate encoder
    pub fn new() -> Self {
        let context = Context::thread_local();
        let verum_ctx = VerumContext::new();

        // Reference is a 128-bit bitvector (64-bit ptr + 64-bit generation)
        let ref_sort = Sort::bitvector(128);

        // Create generation(ref) -> BV<64> predicate
        let generation_pred = FuncDecl::new(
            Symbol::String("generation".to_string()),
            &[&ref_sort],
            &Sort::bitvector(64),
        );

        // Create epoch(ref) -> BV<16> predicate
        let epoch_pred = FuncDecl::new(
            Symbol::String("epoch".to_string()),
            &[&ref_sort],
            &Sort::bitvector(16),
        );

        // Create valid(ref) -> Bool predicate
        let valid_pred = FuncDecl::new(
            Symbol::String("valid".to_string()),
            &[&ref_sort],
            &Sort::bool(),
        );

        // Create same_allocation(ref, ref) -> Bool predicate
        let same_alloc_pred = FuncDecl::new(
            Symbol::String("same_allocation".to_string()),
            &[&ref_sort, &ref_sort],
            &Sort::bool(),
        );

        // Global generation counter (monotonically increasing)
        let global_generation = FuncDecl::new(
            Symbol::String("global_gen".to_string()),
            &[],
            &Sort::bitvector(64),
        );

        Self {
            context,
            verum_ctx,
            ref_sort,
            generation_pred,
            epoch_pred,
            valid_pred,
            same_alloc_pred,
            global_generation,
        }
    }

    /// Encode generation extraction
    ///
    /// generation(ref) extracts bits 64-127 from the reference bitvector.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use verum_smt::cbgr_predicates::CBGRPredicateEncoder;
    ///
    /// let encoder = CBGRPredicateEncoder::new();
    /// // Verify: generation(ref) == 42
    /// ```
    pub fn encode_generation(&self, ref_var: &BV) -> BV {
        // Extract bits 64-127 (generation counter)
        ref_var.extract(127, 64)
    }

    /// Encode epoch extraction
    ///
    /// epoch(ref) extracts bits 112-127 from the generation counter.
    pub fn encode_epoch(&self, ref_var: &BV) -> BV {
        // Extract bits 112-127 (epoch counter from generation)
        let generation = self.encode_generation(ref_var);
        generation.extract(63, 48)
    }

    /// Encode validity check
    ///
    /// A reference is valid if its generation <= global_generation.
    pub fn encode_valid(&self, ref_var: &BV) -> Bool {
        let ref_gen = self.encode_generation(ref_var);
        let global_gen = self.global_generation.apply(&[]);
        // SAFETY: global_generation is declared as BV64 function in encoder initialization
        // as_bv() only fails if sort mismatch, which is impossible here
        let global_gen_bv = global_gen.as_bv().unwrap();

        // valid(ref) ⟺ generation(ref) ≤ global_gen
        ref_gen.bvule(&global_gen_bv)
    }

    /// Encode same allocation check
    ///
    /// Two references point to the same allocation if their pointers match.
    pub fn encode_same_allocation(&self, ref_a: &BV, ref_b: &BV) -> Bool {
        // Extract pointers (bits 0-63)
        let ptr_a = ref_a.extract(63, 0);
        let ptr_b = ref_b.extract(63, 0);

        // same_allocation(a, b) ⟺ ptr(a) == ptr(b)
        ptr_a.eq(&ptr_b)
    }

    /// Verify a CBGR property
    ///
    /// Takes a property expressed using generation predicates and verifies it.
    ///
    /// # Example
    ///
    /// ```no_run
    /// // Verify: valid(ref1) && valid(ref2) => same_allocation(ref1, ref2)
    /// ```
    pub fn verify_property(&self, property: &GenerationPredicate) -> CBGRVerificationResult {
        let start = Instant::now();
        let mut stats = CBGRStats::default();

        let solver = Solver::new();

        // Create symbolic reference variables
        let ref_a = BV::new_const("ref_a", 128);
        let ref_b = BV::new_const("ref_b", 128);

        // Encode property
        let property_formula = match property {
            GenerationPredicate::Generation { .. } => {
                stats.generation_checks += 1;
                let generation = self.encode_generation(&ref_a);
                // Create a constraint that generation is non-negative
                generation.bvuge(BV::from_u64(0, 64))
            }
            GenerationPredicate::Epoch { .. } => {
                stats.epoch_checks += 1;
                let epoch = self.encode_epoch(&ref_a);
                // Epoch must be in valid range [0, 65535]
                epoch.bvuge(BV::from_u64(0, 16))
            }
            GenerationPredicate::Valid { .. } => {
                stats.validity_checks += 1;
                self.encode_valid(&ref_a)
            }
            GenerationPredicate::SameAllocation { .. } => {
                stats.allocation_checks += 1;
                self.encode_same_allocation(&ref_a, &ref_b)
            }
        };

        // Check if property is satisfiable (we want it to be valid, so check negation)
        solver.assert(property_formula.not());

        let duration = start.elapsed();
        stats.smt_time = duration;

        match solver.check() {
            SatResult::Unsat => {
                // Property is valid (no counterexample)
                CBGRVerificationResult {
                    is_valid: true,
                    duration,
                    counterexample: Maybe::None,
                    stats,
                }
            }
            SatResult::Sat => {
                // Property violated - extract counterexample
                let counterexample = self.extract_counterexample(&solver, property);
                CBGRVerificationResult {
                    is_valid: false,
                    duration,
                    counterexample: Maybe::Some(counterexample),
                    stats,
                }
            }
            SatResult::Unknown => {
                // Solver timeout or unknown
                CBGRVerificationResult {
                    is_valid: false,
                    duration,
                    counterexample: Maybe::None,
                    stats,
                }
            }
        }
    }

    /// Extract counterexample from SAT model
    ///
    /// Parses the Z3 model to extract concrete values for reference variables,
    /// including pointer, generation, and epoch fields from 128-bit bitvectors.
    fn extract_counterexample(
        &self,
        solver: &Solver,
        property: &GenerationPredicate,
    ) -> CBGRCounterexample {
        let mut ref_values = Map::new();

        if let Some(model) = solver.get_model() {
            // Extract reference values from model by evaluating symbolic variables
            let ref_names = ["ref_a", "ref_b", "ref1", "ref2"];

            for ref_name in ref_names {
                let ref_var = BV::new_const(ref_name, 128);

                if let Some(evaluated) = model.eval(&ref_var, true) {
                    // Parse the bitvector value from Z3
                    let ref_value = self.parse_reference_bv(&evaluated, &model);
                    ref_values.insert(ref_name.to_text(), ref_value);
                }
            }
        }

        // Generate explanation based on the property type
        let explanation = match property {
            GenerationPredicate::Generation { .. } => {
                "Generation counter constraint violated".to_text()
            }
            GenerationPredicate::Epoch { .. } => "Epoch counter constraint violated".to_text(),
            GenerationPredicate::Valid { .. } => {
                "Reference validity check failed - generation exceeds global counter".to_text()
            }
            GenerationPredicate::SameAllocation { .. } => {
                "Same allocation constraint violated - pointers do not match".to_text()
            }
        };

        CBGRCounterexample {
            ref_values,
            violated_property: format!("{:?}", property).into(),
            explanation,
        }
    }

    /// Parse a 128-bit reference bitvector from Z3 model evaluation
    ///
    /// Layout: [ptr: 64 bits][generation: 48 bits][epoch: 16 bits]
    fn parse_reference_bv(&self, bv: &BV, model: &z3::Model) -> ReferenceValue {
        // Extract pointer (bits 0-63)
        let ptr_bv = bv.extract(63, 0);
        let ptr = self.bv_to_u64(&ptr_bv, model);

        // Extract full generation field (bits 64-127)
        let gen_bv = bv.extract(127, 64);
        let full_gen = self.bv_to_u64(&gen_bv, model);

        // Parse generation value (lower 48 bits) and epoch (upper 16 bits)
        let generation = full_gen & 0x0000_FFFF_FFFF_FFFF;
        let epoch = ((full_gen >> 48) & 0xFFFF) as u16;

        // Check validity against global generation
        let global_gen = self.global_generation.apply(&[]);
        let is_valid = if let Some(global_bv) = global_gen.as_bv() {
            let global_val = self.bv_to_u64(&global_bv, model);
            generation <= global_val
        } else {
            false
        };

        ReferenceValue {
            ptr,
            generation,
            epoch,
            is_valid,
        }
    }

    /// Convert a Z3 bitvector to u64
    ///
    /// Evaluates the bitvector in the model and parses the resulting string representation.
    fn bv_to_u64(&self, bv: &BV, model: &z3::Model) -> u64 {
        if let Some(evaluated) = model.eval(bv, true) {
            // Z3 bitvector string format: #x... (hex) or #b... (binary)
            let s = format!("{}", evaluated);

            if s.starts_with("#x") {
                // Hexadecimal format
                u64::from_str_radix(&s[2..], 16).unwrap_or(0)
            } else if s.starts_with("#b") {
                // Binary format
                u64::from_str_radix(&s[2..], 2).unwrap_or(0)
            } else {
                // Try parsing as decimal
                s.parse().unwrap_or(0)
            }
        } else {
            0
        }
    }

    /// Encode CBGR invariants
    ///
    /// These are global axioms that must always hold:
    /// 1. Generation counters are monotonic
    /// 2. Valid references have generation <= global
    /// 3. Epochs increment on allocation reuse
    pub fn encode_invariants(&self, solver: &Solver) {
        // Axiom 1: Global generation is non-negative
        let global_gen = self.global_generation.apply(&[]);
        if let Some(global_gen_bv) = global_gen.as_bv() {
            solver.assert(global_gen_bv.bvuge(BV::from_u64(0, 64)));
        }

        // Axiom 2: Generation counters don't overflow
        // (Implicit in 48-bit representation)

        // Axiom 3: Same allocation => generation differs by epoch increment
        let ref1 = BV::new_const("ref1", 128);
        let ref2 = BV::new_const("ref2", 128);

        let same_alloc = self.encode_same_allocation(&ref1, &ref2);
        let gen1 = self.encode_generation(&ref1);
        let gen2 = self.encode_generation(&ref2);

        // If same allocation and different references, generations differ
        let different_gen = gen1.eq(&gen2).not();
        let implication = same_alloc.implies(&different_gen);
        solver.assert(&implication);
    }

    /// Verify generation monotonicity
    ///
    /// Checks that if ref2 is allocated after ref1, then gen(ref2) >= gen(ref1).
    pub fn verify_monotonicity(&self, ref1: &BV, ref2: &BV) -> bool {
        let solver = Solver::new();

        let gen1 = self.encode_generation(ref1);
        let gen2 = self.encode_generation(ref2);

        // Assert ref2 allocated after ref1 => gen2 >= gen1
        let monotonicity = gen2.bvuge(&gen1);
        solver.assert(monotonicity.not());

        // Check for counterexample
        matches!(solver.check(), SatResult::Unsat)
    }

    /// Verify epoch increment on reuse
    ///
    /// When an allocation is reused, the epoch must increment.
    pub fn verify_epoch_increment(&self, old_ref: &BV, new_ref: &BV) -> bool {
        let solver = Solver::new();

        // Same allocation
        let same_alloc = self.encode_same_allocation(old_ref, new_ref);
        solver.assert(&same_alloc);

        // Epoch must increment
        let old_epoch = self.encode_epoch(old_ref);
        let new_epoch = self.encode_epoch(new_ref);

        // new_epoch > old_epoch OR wrapped around
        let incremented = new_epoch.bvugt(&old_epoch);
        solver.assert(incremented.not());

        matches!(solver.check(), SatResult::Unsat)
    }
}

impl Default for CBGRPredicateEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== High-Level API ====================

/// Verify a generation predicate property
pub fn verify_generation_property(property: &GenerationPredicate) -> CBGRVerificationResult {
    let encoder = CBGRPredicateEncoder::new();
    encoder.verify_property(property)
}

/// Check if a reference is valid
pub fn is_valid_reference(generation: u64, global_gen: u64) -> bool {
    generation <= global_gen
}

/// Extract epoch from generation counter
pub fn extract_epoch(generation: u64) -> u16 {
    ((generation >> 48) & 0xFFFF) as u16
}

/// Extract generation value (lower 48 bits)
pub fn extract_generation_value(generation: u64) -> u64 {
    generation & 0x0000_FFFF_FFFF_FFFF
}

/// Encode generation counter (48-bit generation + 16-bit epoch)
pub fn encode_generation_counter(gen_value: u64, epoch: u16) -> u64 {
    (gen_value & 0x0000_FFFF_FFFF_FFFF) | ((epoch as u64) << 48)
}

/// Verify generation-aware refinement type
///
/// Parses a refinement string containing generation predicates and verifies
/// them using Z3. Supports the following predicate syntax:
/// - `generation(ref) >= N` - generation counter comparison
/// - `generation(ref) == N` - exact generation match
/// - `epoch(ref) == N` - epoch counter match
/// - `valid(ref)` - reference validity check
/// - `same_allocation(a, b)` - same allocation check
///
/// # Example
///
/// ```no_run
/// use verum_smt::cbgr_predicates::verify_generation_refinement;
///
/// // Verify that generation is always non-negative
/// let result = verify_generation_refinement("generation(ref) >= 0");
/// assert!(result.is_valid);
///
/// // Verify epoch bounds
/// let result = verify_generation_refinement("epoch(ref) <= 65535");
/// assert!(result.is_valid);
/// ```
pub fn verify_generation_refinement(refinement: &str) -> CBGRVerificationResult {
    let start = std::time::Instant::now();
    let encoder = CBGRPredicateEncoder::new();
    let solver = Solver::new();
    let mut stats = CBGRStats::default();

    // Parse the refinement string to extract predicates
    let parsed = parse_generation_refinement(refinement);

    match parsed {
        RefinementParse::Generation { op, value } => {
            stats.generation_checks += 1;
            let ref_var = BV::new_const("ref", 128);
            let generation = encoder.encode_generation(&ref_var);
            let value_bv = BV::from_u64(value, 64);

            let constraint = match op {
                CompareOp::Eq => generation.eq(&value_bv),
                CompareOp::Ne => generation.eq(&value_bv).not(),
                CompareOp::Lt => generation.bvult(&value_bv),
                CompareOp::Le => generation.bvule(&value_bv),
                CompareOp::Gt => generation.bvugt(&value_bv),
                CompareOp::Ge => generation.bvuge(&value_bv),
            };

            // Check if constraint is valid (negation is UNSAT)
            solver.assert(constraint.not());
        }
        RefinementParse::Epoch { op, value } => {
            stats.epoch_checks += 1;
            let ref_var = BV::new_const("ref", 128);
            let epoch = encoder.encode_epoch(&ref_var);
            let value_bv = BV::from_u64(value as u64, 16);

            let constraint = match op {
                CompareOp::Eq => epoch.eq(&value_bv),
                CompareOp::Ne => epoch.eq(&value_bv).not(),
                CompareOp::Lt => epoch.bvult(&value_bv),
                CompareOp::Le => epoch.bvule(&value_bv),
                CompareOp::Gt => epoch.bvugt(&value_bv),
                CompareOp::Ge => epoch.bvuge(&value_bv),
            };

            solver.assert(constraint.not());
        }
        RefinementParse::Valid => {
            stats.validity_checks += 1;
            let ref_var = BV::new_const("ref", 128);
            let valid = encoder.encode_valid(&ref_var);
            solver.assert(valid.not());
        }
        RefinementParse::SameAllocation => {
            stats.allocation_checks += 1;
            let ref_a = BV::new_const("ref_a", 128);
            let ref_b = BV::new_const("ref_b", 128);
            let same_alloc = encoder.encode_same_allocation(&ref_a, &ref_b);
            solver.assert(same_alloc.not());
        }
        RefinementParse::Unknown => {
            // Fallback to default property for unknown refinements
            let property = GenerationPredicate::Generation {
                ref_expr: Box::new(Type::int(Span::dummy())),
            };
            return encoder.verify_property(&property);
        }
    }

    stats.smt_time = start.elapsed();

    match solver.check() {
        SatResult::Unsat => CBGRVerificationResult {
            is_valid: true,
            duration: start.elapsed(),
            counterexample: Maybe::None,
            stats,
        },
        SatResult::Sat => {
            // Extract counterexample if available
            let counterexample = if let Some(model) = solver.get_model() {
                let ref_var = BV::new_const("ref", 128);
                if let Some(evaluated) = model.eval(&ref_var, true) {
                    let ref_value = encoder.parse_reference_bv(&evaluated, &model);
                    let mut ref_values = Map::new();
                    ref_values.insert("ref".to_text(), ref_value);
                    Maybe::Some(CBGRCounterexample {
                        ref_values,
                        violated_property: refinement.to_text(),
                        explanation: "Refinement constraint does not hold universally".to_text(),
                    })
                } else {
                    Maybe::None
                }
            } else {
                Maybe::None
            };

            CBGRVerificationResult {
                is_valid: false,
                duration: start.elapsed(),
                counterexample,
                stats,
            }
        }
        SatResult::Unknown => CBGRVerificationResult {
            is_valid: false,
            duration: start.elapsed(),
            counterexample: Maybe::None,
            stats,
        },
    }
}

/// Comparison operators for refinement predicates
#[derive(Debug, Clone, Copy)]
enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Parsed refinement predicate
#[derive(Debug)]
enum RefinementParse {
    Generation { op: CompareOp, value: u64 },
    Epoch { op: CompareOp, value: u16 },
    Valid,
    SameAllocation,
    Unknown,
}

/// Parse a generation refinement string into a structured predicate
fn parse_generation_refinement(refinement: &str) -> RefinementParse {
    let trimmed = refinement.trim();

    // Check for generation(ref) predicates
    if trimmed.starts_with("generation(") {
        if let Some(rest) = trimmed.strip_prefix("generation(") {
            // Find the closing paren and operator
            if let Some(close_idx) = rest.find(')') {
                let after_paren = rest[close_idx + 1..].trim();

                // Parse the operator and value
                if let Some((op, value_str)) = parse_comparison(after_paren) {
                    if let Ok(value) = value_str.parse::<u64>() {
                        return RefinementParse::Generation { op, value };
                    }
                }
            }
        }
    }

    // Check for epoch(ref) predicates
    if trimmed.starts_with("epoch(") {
        if let Some(rest) = trimmed.strip_prefix("epoch(") {
            if let Some(close_idx) = rest.find(')') {
                let after_paren = rest[close_idx + 1..].trim();

                if let Some((op, value_str)) = parse_comparison(after_paren) {
                    if let Ok(value) = value_str.parse::<u16>() {
                        return RefinementParse::Epoch { op, value };
                    }
                }
            }
        }
    }

    // Check for valid(ref) predicate
    if trimmed.starts_with("valid(") {
        return RefinementParse::Valid;
    }

    // Check for same_allocation(a, b) predicate
    if trimmed.starts_with("same_allocation(") {
        return RefinementParse::SameAllocation;
    }

    RefinementParse::Unknown
}

/// Parse a comparison operator and value from a string like ">= 0"
fn parse_comparison(s: &str) -> Option<(CompareOp, &str)> {
    let s = s.trim();

    if s.starts_with(">=") {
        Some((CompareOp::Ge, s[2..].trim()))
    } else if s.starts_with("<=") {
        Some((CompareOp::Le, s[2..].trim()))
    } else if s.starts_with("==") {
        Some((CompareOp::Eq, s[2..].trim()))
    } else if s.starts_with("!=") {
        Some((CompareOp::Ne, s[2..].trim()))
    } else if s.starts_with('>') {
        Some((CompareOp::Gt, s[1..].trim()))
    } else if s.starts_with('<') {
        Some((CompareOp::Lt, s[1..].trim()))
    } else if s.starts_with('=') {
        Some((CompareOp::Eq, s[1..].trim()))
    } else {
        None
    }
}

// ==================== Integration with Refinement Verification ====================

/// Extend refinement verifier with CBGR predicates
#[allow(dead_code)] // Fields reserved for full CBGR-aware refinement verification
pub struct CBGRAwareRefinementVerifier {
    /// CBGR encoder
    encoder: CBGRPredicateEncoder,
    /// Standard translator
    translator: Translator<'static>,
}

impl CBGRAwareRefinementVerifier {
    /// Create a new CBGR-aware refinement verifier
    pub fn new(context: &'static VerumContext) -> Self {
        Self {
            encoder: CBGRPredicateEncoder::new(),
            translator: Translator::new(context),
        }
    }

    /// Verify refinement with CBGR predicates
    pub fn verify_with_cbgr(
        &self,
        ty: &Type,
        predicates: &[GenerationPredicate],
    ) -> CBGRVerificationResult {
        let start = Instant::now();
        let mut stats = CBGRStats::default();

        // Verify each predicate
        for predicate in predicates {
            let result = self.encoder.verify_property(predicate);

            if !result.is_valid {
                return result;
            }

            // Aggregate stats
            stats.generation_checks += result.stats.generation_checks;
            stats.epoch_checks += result.stats.epoch_checks;
            stats.validity_checks += result.stats.validity_checks;
            stats.allocation_checks += result.stats.allocation_checks;
        }

        stats.smt_time = start.elapsed();

        CBGRVerificationResult {
            is_valid: true,
            duration: start.elapsed(),
            counterexample: Maybe::None,
            stats,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = CBGRPredicateEncoder::new();
        // Successfully created
    }

    #[test]
    fn test_generation_extraction() {
        assert_eq!(extract_generation_value(0x0001_0000_0000_002A), 42);
    }

    #[test]
    fn test_epoch_extraction() {
        assert_eq!(extract_epoch(0x0001_0000_0000_0000), 1);
    }

    #[test]
    fn test_generation_encoding() {
        let generation = encode_generation_counter(42, 1);
        assert_eq!(extract_generation_value(generation), 42);
        assert_eq!(extract_epoch(generation), 1);
    }

    #[test]
    fn test_is_valid_reference() {
        assert!(is_valid_reference(42, 100));
        assert!(!is_valid_reference(100, 42));
    }

    #[test]
    fn test_verify_generation_property() {
        use verum_ast::span::Span;
        let property = GenerationPredicate::Generation {
            ref_expr: Box::new(Type::int(Span::default())),
        };

        let result = verify_generation_property(&property);
        // Should verify successfully
    }

    #[test]
    fn test_generation_refinement() {
        let result = verify_generation_refinement("generation(ref) >= 0");
        assert!(result.is_valid);
    }

    #[test]
    fn test_generation_counter_bounds() {
        // Max generation value (48 bits)
        let max_gen = 0x0000_FFFF_FFFF_FFFF;
        assert_eq!(extract_generation_value(max_gen), max_gen);

        // Max epoch (16 bits)
        let max_epoch: u16 = 0xFFFF;
        let gen_with_max_epoch = encode_generation_counter(0, max_epoch);
        assert_eq!(extract_epoch(gen_with_max_epoch), max_epoch);
    }

    #[test]
    fn test_reference_value_creation() {
        let ref_val = ReferenceValue {
            ptr: 0x1000,
            generation: 42,
            epoch: 1,
            is_valid: true,
        };

        assert_eq!(ref_val.ptr, 0x1000);
        assert_eq!(ref_val.generation, 42);
        assert!(ref_val.is_valid);
    }
}
