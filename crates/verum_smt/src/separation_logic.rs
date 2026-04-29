//! Separation Logic for Heap Verification
//!
//! Implements separation logic for verifying heap-manipulating programs using Z3 SMT solver.
//!
//! Separation logic assertions model disjoint heap ownership:
//! - Separating conjunction (P * Q): heap splits into disjoint regions satisfying P and Q
//! - Magic wand (P -* Q): separating implication for frame reasoning
//! - Points-to (x |-> v): single heap cell at location x contains value v
//! - Frame rule: {P} c {Q} implies {P * R} c {Q * R} (frame preservation)
//! - List segments lseg(x, y, xs) and tree predicates tree(x, t)
//!
//! SMT encoding uses Z3 array theory: heap as Array<Int, Int>, disjointness via
//! quantified constraints over array domains.
//!
//! ## Features
//!
//! - **Separating Conjunction (P * Q)**: Disjoint heap regions encoded via Z3 array theory
//! - **Magic Wand (P -* Q)**: Separating implication for frame reasoning
//! - **Points-To Assertions (x |-> v)**: Single heap cell ownership
//! - **List Segments (lseg(x, y, xs))**: Recursive list segment predicates
//! - **Tree Predicates (tree(x, t))**: Binary tree shape predicates
//! - **Frame Rule**: {P} c {Q} => {P * R} c {Q * R}
//! - **Heap Entailment**: P |- Q verification via SMT
//! - **Weakest Precondition**: wp(c, Q) computation
//!
//! ## Z3 Encoding
//!
//! Heaps are encoded as arrays from addresses to values:
//! ```text
//! Heap = Array(Address, Value)
//! Points-to(x, v) = heap[x] == v AND allocated(x)
//! Sep(P, Q) = P(h1) AND Q(h2) AND disjoint(dom(h1), dom(h2))
//! ```
//!
//! ## Performance Targets
//!
//! - Assertion checking: < 10ms
//! - Frame rule application: < 5ms
//! - WP computation: < 20ms
//! - Heap entailment: < 50ms

use std::cell::RefCell;
use std::time::{Duration, Instant};

use z3::{
    Model, Params, SatResult, Solver, Sort,
    ast::{Array, Bool, Dynamic},
};

use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::{BinOp, Expr, ExprKind, Ident, Path};
use verum_common::{Heap, List, Map, Maybe, Set, Text};

use crate::context::Context as VerumContext;
use crate::proof_search::{ProofError, ProofGoal, ProofSearchEngine, ProofTerm};

// ==================== Configuration ====================

/// Configuration for separation logic verification
#[derive(Debug, Clone)]
pub struct SepLogicConfig {
    /// Timeout for individual entailment checks (ms)
    pub entailment_timeout_ms: u64,
    /// Maximum unfolding depth for recursive predicates
    pub max_unfolding_depth: usize,
    /// Enable frame inference
    pub enable_frame_inference: bool,
    /// Enable symbolic execution for entailment
    pub enable_symbolic_execution: bool,
    /// Cache entailment results
    pub enable_caching: bool,
}

impl Default for SepLogicConfig {
    fn default() -> Self {
        Self {
            entailment_timeout_ms: 5000,
            max_unfolding_depth: 10,
            enable_frame_inference: true,
            enable_symbolic_execution: true,
            enable_caching: true,
        }
    }
}

// ==================== Separation Logic Assertions ====================

/// Separation logic assertion about heap
///
/// Separation logic assertions about the heap. Core assertions:
/// - PointsTo: `x |-> v` (location x contains value v, exclusive ownership)
/// - SepConj: `P * Q` (heap splits into disjoint regions for P and Q)
/// - MagicWand: `P -* Q` (adding a P-satisfying heap yields Q)
/// - Emp: empty heap (no owned locations)
/// - ListSeg/TreePred: recursive shape predicates for linked structures
#[derive(Debug, Clone)]
pub enum SepAssertion {
    /// Points-to: x |-> v (location x contains value v)
    PointsTo { location: Expr, value: Expr },

    /// Separating conjunction: P * Q (disjoint heaps)
    Sep {
        left: Heap<SepAssertion>,
        right: Heap<SepAssertion>,
    },

    /// Pure assertion (no heap reference)
    Pure(Expr),

    /// Empty heap
    Emp,

    /// Disjunction: P \/ Q
    Or {
        left: Heap<SepAssertion>,
        right: Heap<SepAssertion>,
    },

    /// Existential quantification: exists x. P
    Exists { var: Text, body: Heap<SepAssertion> },

    /// Universal quantification: forall x. P
    Forall { var: Text, body: Heap<SepAssertion> },

    /// Separating implication (magic wand): P -* Q
    /// "If you give me P, I'll give you Q"
    Wand {
        left: Heap<SepAssertion>,
        right: Heap<SepAssertion>,
    },

    /// Conjunction: P /\ Q
    And {
        left: Heap<SepAssertion>,
        right: Heap<SepAssertion>,
    },

    /// List segment predicate: lseg(from, to, elements)
    /// Represents a linked list segment from 'from' to 'to' containing 'elements'
    ListSegment {
        from: Expr,
        to: Expr,
        elements: List<Expr>,
    },

    /// Binary tree predicate: tree(root, structure)
    /// Represents a binary tree rooted at 'root' with given structure
    Tree {
        root: Expr,
        left_child: Maybe<Heap<SepAssertion>>,
        right_child: Maybe<Heap<SepAssertion>>,
    },

    /// Block of contiguous memory: block(base, size)
    /// Represents ownership of a memory block
    Block { base: Expr, size: Expr },

    /// Array segment: array_seg(base, offset, length, elements)
    ArraySegment {
        base: Expr,
        offset: Expr,
        length: Expr,
        elements: List<Expr>,
    },
}

impl SepAssertion {
    /// Create points-to assertion
    pub fn points_to(location: Expr, value: Expr) -> Self {
        SepAssertion::PointsTo { location, value }
    }

    /// Create separating conjunction
    pub fn sep(left: SepAssertion, right: SepAssertion) -> Self {
        SepAssertion::Sep {
            left: Heap::new(left),
            right: Heap::new(right),
        }
    }

    /// Create pure assertion
    pub fn pure(expr: Expr) -> Self {
        SepAssertion::Pure(expr)
    }

    /// Create empty heap assertion
    pub fn emp() -> Self {
        SepAssertion::Emp
    }

    /// Create separating implication (magic wand)
    pub fn wand(left: SepAssertion, right: SepAssertion) -> Self {
        SepAssertion::Wand {
            left: Heap::new(left),
            right: Heap::new(right),
        }
    }

    /// Create conjunction
    pub fn and(left: SepAssertion, right: SepAssertion) -> Self {
        SepAssertion::And {
            left: Heap::new(left),
            right: Heap::new(right),
        }
    }

    /// Create existential quantification
    pub fn exists(var: Text, body: SepAssertion) -> Self {
        SepAssertion::Exists {
            var,
            body: Heap::new(body),
        }
    }

    /// Create universal quantification
    pub fn forall(var: Text, body: SepAssertion) -> Self {
        SepAssertion::Forall {
            var,
            body: Heap::new(body),
        }
    }

    /// Create disjunction
    pub fn or(left: SepAssertion, right: SepAssertion) -> Self {
        SepAssertion::Or {
            left: Heap::new(left),
            right: Heap::new(right),
        }
    }

    /// Create list segment
    pub fn list_segment(from: Expr, to: Expr, elements: List<Expr>) -> Self {
        SepAssertion::ListSegment { from, to, elements }
    }

    /// Create empty list segment (from == to)
    pub fn empty_list_segment(at: Expr) -> Self {
        SepAssertion::ListSegment {
            from: at.clone(),
            to: at,
            elements: List::new(),
        }
    }

    /// Create tree predicate
    pub fn tree(
        root: Expr,
        left_child: Maybe<SepAssertion>,
        right_child: Maybe<SepAssertion>,
    ) -> Self {
        SepAssertion::Tree {
            root,
            left_child: left_child.map(Heap::new),
            right_child: right_child.map(Heap::new),
        }
    }

    /// Create block predicate
    pub fn block(base: Expr, size: Expr) -> Self {
        SepAssertion::Block { base, size }
    }

    /// Create array segment
    pub fn array_segment(base: Expr, offset: Expr, length: Expr, elements: List<Expr>) -> Self {
        SepAssertion::ArraySegment {
            base,
            offset,
            length,
            elements,
        }
    }

    /// Check if assertion is pure (no heap references)
    pub fn is_pure(&self) -> bool {
        matches!(self, SepAssertion::Pure(_) | SepAssertion::Emp)
    }

    /// Check if assertion is empty
    pub fn is_emp(&self) -> bool {
        matches!(self, SepAssertion::Emp)
    }

    /// Get the footprint (set of accessed locations)
    pub fn footprint(&self) -> List<Expr> {
        let mut locs = List::new();
        self.collect_footprint(&mut locs);
        locs
    }

    fn collect_footprint(&self, locs: &mut List<Expr>) {
        match self {
            SepAssertion::PointsTo { location, .. } => {
                locs.push(location.clone());
            }
            SepAssertion::Sep { left, right }
            | SepAssertion::And { left, right }
            | SepAssertion::Or { left, right }
            | SepAssertion::Wand { left, right } => {
                left.collect_footprint(locs);
                right.collect_footprint(locs);
            }
            SepAssertion::Exists { body, .. } | SepAssertion::Forall { body, .. } => {
                body.collect_footprint(locs);
            }
            SepAssertion::ListSegment { from, .. } => {
                locs.push(from.clone());
            }
            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => {
                locs.push(root.clone());
                if let Maybe::Some(left) = left_child {
                    left.collect_footprint(locs);
                }
                if let Maybe::Some(right) = right_child {
                    right.collect_footprint(locs);
                }
            }
            SepAssertion::Block { base, .. } => {
                locs.push(base.clone());
            }
            SepAssertion::ArraySegment { base, .. } => {
                locs.push(base.clone());
            }
            SepAssertion::Pure(_) | SepAssertion::Emp => {}
        }
    }
}

// ==================== Commands ====================

/// Command in separation logic
///
/// Commands in separation logic for weakest precondition computation:
/// - Skip: no-op, wp(skip, Q) = Q
/// - Assign: `x := e`, wp(x:=e, Q) = Q[e/x]
/// - Seq: `c1; c2`, wp(c1;c2, Q) = wp(c1, wp(c2, Q))
/// - If: conditional, wp(if b then c1 else c2, Q) = (b => wp(c1,Q)) /\ (!b => wp(c2,Q))
/// - While: loop with invariant, wp uses the loop invariant
/// - Alloc/Dealloc/Read/Write: heap-manipulating operations
#[derive(Debug, Clone)]
pub enum Command {
    /// Skip (no-op)
    Skip,

    /// Assignment: x := e
    Assign { var: Text, expr: Expr },

    /// Sequential composition: c1; c2
    Seq {
        first: Heap<Command>,
        second: Heap<Command>,
    },

    /// Conditional: if b then c1 else c2
    If {
        condition: Expr,
        then_branch: Heap<Command>,
        else_branch: Heap<Command>,
    },

    /// While loop: while b invariant inv do c
    While {
        condition: Expr,
        invariant: SepAssertion,
        body: Heap<Command>,
    },

    /// Heap allocation: x := alloc(size)
    Alloc { result: Text, size: Expr },

    /// Heap read: x := *addr
    Read { result: Text, addr: Expr },

    /// Heap write: *addr := value
    Write { addr: Expr, value: Expr },

    /// Heap deallocation: free(ptr, size)
    Free { ptr: Expr, size: Expr },

    /// Atomic compare-and-swap
    CAS {
        result: Text,
        addr: Expr,
        expected: Expr,
        desired: Expr,
    },

    /// Function call with contract
    Call {
        result: Maybe<Text>,
        func: Text,
        args: List<Expr>,
        pre: SepAssertion,
        post: SepAssertion,
    },
}

// ==================== Hoare Triples ====================

/// Hoare triple: {P} c {Q}
///
/// Hoare triple `{P} c {Q}`: for all states s, if P(s) holds then the weakest
/// precondition wp(c, Q)(s) also holds. Verification checks that P implies wp(c, Q).
#[derive(Debug, Clone)]
pub struct HoareTriple {
    /// Precondition
    pub pre: SepAssertion,
    /// Command
    pub command: Command,
    /// Postcondition
    pub post: SepAssertion,
}

impl HoareTriple {
    /// Create a new Hoare triple
    pub fn new(pre: SepAssertion, command: Command, post: SepAssertion) -> Self {
        Self { pre, command, post }
    }
}

// ==================== Memory Region Classification ====================

/// Classification of memory regions for stack/heap separation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegion {
    /// Stack memory (local variables, function arguments)
    Stack,
    /// Heap memory (dynamically allocated)
    Heap,
    /// Global/static memory
    Global,
    /// Unknown region (conservative)
    Unknown,
}

/// CBGR generation information for memory safety verification
#[derive(Debug, Clone, Default)]
pub struct GenerationInfo {
    /// Current generation of the allocation
    pub generation: u64,
    /// Epoch capabilities (for CBGR tier-based checking)
    pub epoch_caps: u64,
    /// Whether this is a thin reference (16 bytes) or fat reference (24 bytes)
    pub is_fat_ref: bool,
}

// ==================== Z3 Heap Model ====================

/// Z3-based biheap model for separation logic encoding
///
/// This model implements a proper separation of stack and heap memory regions,
/// with support for CBGR (Capability-Based Generational References) verification.
///
/// Memory Layout:
/// - Stack: [0x0000_0000_0000_0000, 0x0000_7FFF_FFFF_FFFF) - lower half
/// - Heap:  [0x0000_8000_0000_0000, 0xFFFF_FFFF_FFFF_FFFF) - upper half
pub struct Z3HeapModel {
    /// Heap as array from addresses to values
    heap: Array,
    /// Allocation bitmap (which addresses are allocated)
    allocated: Array,
    /// Generation map for CBGR (address -> generation counter)
    generations: Array,
    /// Region classification map (address -> region type)
    regions: Array,
    /// Fresh variable counter
    fresh_counter: RefCell<usize>,
    /// Address sort (bitvector 64)
    addr_sort: Sort,
    /// Value sort (bitvector 64)
    value_sort: Sort,
    /// Generation sort (bitvector 64)
    gen_sort: Sort,
    /// Region sort (bitvector 8 - 0=Stack, 1=Heap, 2=Global, 3=Unknown)
    region_sort: Sort,
    /// Stack base address
    stack_base: i64,
    /// Heap base address
    heap_base: i64,
}

impl Z3HeapModel {
    /// Stack region starts at 0
    const STACK_BASE: i64 = 0x0000_0000_0000_0000;
    /// Heap region starts at 2^63
    const HEAP_BASE: i64 = 0x0000_8000_0000_0000u64 as i64;
    /// Region code for stack
    const REGION_STACK: i64 = 0;
    /// Region code for heap
    const REGION_HEAP: i64 = 1;
    /// Region code for global
    const REGION_GLOBAL: i64 = 2;
    /// Region code for unknown
    const REGION_UNKNOWN: i64 = 3;

    /// Create a new biheap model with stack/heap separation
    pub fn new() -> Self {
        let addr_sort = Sort::bitvector(64);
        let value_sort = Sort::bitvector(64);
        let bool_sort = Sort::bool();
        let gen_sort = Sort::bitvector(64);
        let region_sort = Sort::bitvector(8);

        // Heap: Address -> Value
        let heap = Array::new_const("heap", &addr_sort, &value_sort);

        // Allocated: Address -> Bool
        let allocated = Array::new_const("allocated", &addr_sort, &bool_sort);

        // Generations: Address -> Generation (for CBGR)
        let generations = Array::new_const("generations", &addr_sort, &gen_sort);

        // Regions: Address -> Region (for stack/heap classification)
        let regions = Array::new_const("regions", &addr_sort, &region_sort);

        Self {
            heap,
            allocated,
            generations,
            regions,
            fresh_counter: RefCell::new(0),
            addr_sort,
            value_sort,
            gen_sort,
            region_sort,
            stack_base: Self::STACK_BASE,
            heap_base: Self::HEAP_BASE,
        }
    }

    /// Create a biheap model with custom base addresses
    pub fn with_bases(stack_base: i64, heap_base: i64) -> Self {
        let mut model = Self::new();
        model.stack_base = stack_base;
        model.heap_base = heap_base;
        model
    }

    /// Create a fresh heap variable
    pub fn fresh_heap(&self, prefix: &str) -> Array {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Array::new_const(
            format!("{}_{}", prefix, counter),
            &self.addr_sort,
            &self.value_sort,
        )
    }

    /// Create a fresh allocation map
    pub fn fresh_allocated(&self, prefix: &str) -> Array {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        let bool_sort = Sort::bool();
        Array::new_const(
            format!("{}_alloc_{}", prefix, counter),
            &self.addr_sort,
            &bool_sort,
        )
    }

    /// Create a fresh address variable
    pub fn fresh_addr(&self, prefix: &str) -> Dynamic {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Dynamic::from_ast(&z3::ast::BV::new_const(
            format!("{}_{}", prefix, counter),
            64,
        ))
    }

    /// Create a fresh value variable
    pub fn fresh_value(&self, prefix: &str) -> Dynamic {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Dynamic::from_ast(&z3::ast::BV::new_const(
            format!("{}_{}", prefix, counter),
            64,
        ))
    }

    /// Get the heap array
    pub fn heap(&self) -> &Array {
        &self.heap
    }

    /// Get the allocation map
    pub fn allocated(&self) -> &Array {
        &self.allocated
    }

    /// Create address constant from integer
    pub fn addr_const(&self, value: i64) -> Dynamic {
        Dynamic::from_ast(&z3::ast::BV::from_i64(value, 64))
    }

    /// Create value constant from integer
    pub fn value_const(&self, value: i64) -> Dynamic {
        Dynamic::from_ast(&z3::ast::BV::from_i64(value, 64))
    }

    /// Get the generations array (for CBGR verification)
    pub fn generations(&self) -> &Array {
        &self.generations
    }

    /// Get the regions array (for stack/heap classification)
    pub fn regions(&self) -> &Array {
        &self.regions
    }

    /// Create a fresh generation variable
    pub fn fresh_generation(&self, prefix: &str) -> Dynamic {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Dynamic::from_ast(&z3::ast::BV::new_const(
            format!("{}_gen_{}", prefix, counter),
            64,
        ))
    }

    /// Create a generation constant
    pub fn generation_const(&self, generation: u64) -> Dynamic {
        Dynamic::from_ast(&z3::ast::BV::from_u64(generation, 64))
    }

    /// Create a region constant
    pub fn region_const(&self, region: MemoryRegion) -> Dynamic {
        let code = match region {
            MemoryRegion::Stack => Self::REGION_STACK,
            MemoryRegion::Heap => Self::REGION_HEAP,
            MemoryRegion::Global => Self::REGION_GLOBAL,
            MemoryRegion::Unknown => Self::REGION_UNKNOWN,
        };
        Dynamic::from_ast(&z3::ast::BV::from_i64(code, 8))
    }

    /// Create a constraint that an address is in the stack region
    pub fn is_stack_addr(&self, addr: &Dynamic) -> Bool {
        let heap_base = z3::ast::BV::from_i64(self.heap_base, 64);
        if let Some(addr_bv) = addr.as_bv() {
            addr_bv.bvslt(&heap_base)
        } else {
            Bool::from_bool(false)
        }
    }

    /// Create a constraint that an address is in the heap region
    pub fn is_heap_addr(&self, addr: &Dynamic) -> Bool {
        let heap_base = z3::ast::BV::from_i64(self.heap_base, 64);
        if let Some(addr_bv) = addr.as_bv() {
            addr_bv.bvsge(&heap_base)
        } else {
            Bool::from_bool(false)
        }
    }

    /// Create a CBGR generation check: expected_gen == actual_gen
    pub fn generation_check(&self, addr: &Dynamic, expected_gen: &Dynamic) -> Bool {
        let actual_gen = self.generations.select(addr);
        expected_gen.eq(&actual_gen)
    }

    /// Create a fresh generations array
    pub fn fresh_generations(&self, prefix: &str) -> Array {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Array::new_const(
            format!("{}_gens_{}", prefix, counter),
            &self.addr_sort,
            &self.gen_sort,
        )
    }

    /// Create a fresh regions array
    pub fn fresh_regions(&self, prefix: &str) -> Array {
        let counter = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        Array::new_const(
            format!("{}_regs_{}", prefix, counter),
            &self.addr_sort,
            &self.region_sort,
        )
    }
}

impl Default for Z3HeapModel {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Iterated Separating Conjunction ====================

/// Represents an iterated separating conjunction: P1 * P2 * ... * Pn
/// This is used for encoding multiple disjoint heap regions efficiently.
#[derive(Debug, Clone)]
pub struct IterSep {
    /// The assertions in the conjunction
    pub assertions: List<SepAssertion>,
}

impl IterSep {
    /// Create an empty iterated separating conjunction (equivalent to emp)
    pub fn emp() -> Self {
        Self {
            assertions: List::new(),
        }
    }

    /// Create from a single assertion
    pub fn single(assertion: SepAssertion) -> Self {
        let mut assertions = List::new();
        assertions.push(assertion);
        Self { assertions }
    }

    /// Create from multiple assertions
    pub fn from_iter(iter: impl IntoIterator<Item = SepAssertion>) -> Self {
        Self {
            assertions: iter.into_iter().collect(),
        }
    }

    /// Add an assertion to the conjunction
    pub fn add(&mut self, assertion: SepAssertion) {
        self.assertions.push(assertion);
    }

    /// Check if empty (equivalent to emp)
    pub fn is_empty(&self) -> bool {
        self.assertions.is_empty()
    }

    /// Get the number of conjuncts
    pub fn len(&self) -> usize {
        self.assertions.len()
    }

    /// Convert to a single SepAssertion
    pub fn to_assertion(&self) -> SepAssertion {
        if self.assertions.is_empty() {
            return SepAssertion::Emp;
        }

        let mut iter = self.assertions.iter();
        let first = iter.next().unwrap().clone();

        iter.fold(first, |acc, next| SepAssertion::Sep {
            left: Heap::new(acc),
            right: Heap::new(next.clone()),
        })
    }

    /// Get the combined footprint of all assertions
    pub fn footprint(&self) -> List<Expr> {
        let mut locs = List::new();
        for assertion in self.assertions.iter() {
            for loc in assertion.footprint().iter() {
                locs.push(loc.clone());
            }
        }
        locs
    }
}

// ==================== CBGR Separation Logic Assertions ====================

/// CBGR-specific assertions for memory safety verification
#[derive(Debug, Clone)]
pub enum CBGRAssertion {
    /// Reference with expected generation: ref(addr, gen)
    /// "This reference points to addr and expects generation gen"
    RefWithGen {
        addr: Expr,
        expected_gen: u64,
        is_fat: bool,
    },

    /// Valid reference assertion: valid_ref(addr, gen)
    /// "The reference is valid (generation matches)"
    ValidRef { addr: Expr, expected_gen: u64 },

    /// Deallocated assertion: freed(addr, old_gen)
    /// "The address was freed, generation was incremented"
    Freed { addr: Expr, old_gen: u64 },

    /// Stack-allocated assertion: stack_ref(addr)
    /// "This reference points to stack memory"
    StackRef { addr: Expr },

    /// Heap-allocated assertion: heap_ref(addr)
    /// "This reference points to heap memory"
    HeapRef { addr: Expr },

    /// Tier-specific assertion: tier_check(addr, tier)
    /// "This reference is checked at the specified execution tier"
    TierCheck { addr: Expr, tier: ExecutionTier },
}

/// Execution tier for CBGR checking costs
///
/// Verum uses a two-tier execution model:
/// - Interpreter: Full runtime checks (~100ns)
/// - Aot: Optimized checks (0ns for proven-safe, ~15ns otherwise)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTier {
    /// Tier 0: Interpreter (~100ns CBGR overhead)
    Interpreter,
    /// Tier 1: AOT (0ns for proven-safe, ~15ns otherwise)
    Aot,
}

impl CBGRAssertion {
    /// Convert to a standard separation logic assertion
    pub fn to_sep_assertion(&self) -> SepAssertion {
        use verum_ast::span::Span;

        match self {
            CBGRAssertion::RefWithGen {
                addr, expected_gen, ..
            } => {
                // Encode as pure assertion about generation
                let gen_check = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "__cbgr_gen_check",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![
                            addr.clone(),
                            Expr::new(
                                ExprKind::Literal(Literal::new(
                                    LiteralKind::Int(IntLit::new(*expected_gen as i128)),
                                    Span::dummy(),
                                )),
                                Span::dummy(),
                            ),
                        ]),
                    },
                    Span::dummy(),
                );
                SepAssertion::Pure(gen_check)
            }

            CBGRAssertion::ValidRef { addr, expected_gen } => {
                let valid_check = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "__cbgr_valid",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![
                            addr.clone(),
                            Expr::new(
                                ExprKind::Literal(Literal::new(
                                    LiteralKind::Int(IntLit::new(*expected_gen as i128)),
                                    Span::dummy(),
                                )),
                                Span::dummy(),
                            ),
                        ]),
                    },
                    Span::dummy(),
                );
                SepAssertion::Pure(valid_check)
            }

            CBGRAssertion::Freed { addr, .. } => {
                // After free, the address is no longer allocated
                SepAssertion::And {
                    left: Heap::new(SepAssertion::Pure(Expr::new(
                        ExprKind::Call {
                            func: Heap::new(Expr::new(
                                ExprKind::Path(Path::from_ident(Ident::new(
                                    "__cbgr_freed",
                                    Span::dummy(),
                                ))),
                                Span::dummy(),
                            )),
                            type_args: List::new(),
                            args: List::from_iter(vec![addr.clone()]),
                        },
                        Span::dummy(),
                    ))),
                    right: Heap::new(SepAssertion::Emp),
                }
            }

            CBGRAssertion::StackRef { addr } => {
                let stack_check = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "__is_stack",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![addr.clone()]),
                    },
                    Span::dummy(),
                );
                SepAssertion::Pure(stack_check)
            }

            CBGRAssertion::HeapRef { addr } => {
                let heap_check = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "__is_heap",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![addr.clone()]),
                    },
                    Span::dummy(),
                );
                SepAssertion::Pure(heap_check)
            }

            CBGRAssertion::TierCheck { addr, tier } => {
                let tier_code = match tier {
                    ExecutionTier::Interpreter => 0,
                    ExecutionTier::Aot => 1,
                };
                let tier_check = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "__tier_check",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![
                            addr.clone(),
                            Expr::new(
                                ExprKind::Literal(Literal::new(
                                    LiteralKind::Int(IntLit::new(tier_code)),
                                    Span::dummy(),
                                )),
                                Span::dummy(),
                            ),
                        ]),
                    },
                    Span::dummy(),
                );
                SepAssertion::Pure(tier_check)
            }
        }
    }
}

// ==================== Frame Inference ====================

/// Result of frame inference
#[derive(Debug, Clone)]
pub struct FrameInferenceResult {
    /// The inferred frame (if found)
    pub frame: Maybe<SepAssertion>,
    /// Whether the inference succeeded
    pub success: bool,
    /// Residual constraints
    pub residual: List<SepAssertion>,
    /// Diagnosis message
    pub message: Text,
}

impl FrameInferenceResult {
    /// Create a successful frame inference result
    pub fn success(frame: SepAssertion) -> Self {
        Self {
            frame: Maybe::Some(frame),
            success: true,
            residual: List::new(),
            message: Text::from("Frame inference succeeded"),
        }
    }

    /// Create a failed frame inference result
    pub fn failure(message: &str) -> Self {
        Self {
            frame: Maybe::None,
            success: false,
            residual: List::new(),
            message: Text::from(message),
        }
    }

    /// Create with residual constraints
    pub fn with_residual(frame: SepAssertion, residual: List<SepAssertion>) -> Self {
        Self {
            frame: Maybe::Some(frame),
            success: true,
            residual,
            message: Text::from("Frame inference succeeded with residual constraints"),
        }
    }
}

// ==================== Bounded Predicate Unfolding ====================

/// Configuration for bounded unfolding of recursive predicates
#[derive(Debug, Clone)]
pub struct UnfoldingConfig {
    /// Maximum depth for list segment unfolding
    pub max_lseg_depth: usize,
    /// Maximum depth for tree unfolding
    pub max_tree_depth: usize,
    /// Whether to use lazy unfolding (unfold on demand)
    pub lazy_unfolding: bool,
    /// Whether to generate fold/unfold lemmas
    pub generate_lemmas: bool,
}

impl Default for UnfoldingConfig {
    fn default() -> Self {
        Self {
            max_lseg_depth: 10,
            max_tree_depth: 5,
            lazy_unfolding: true,
            generate_lemmas: true,
        }
    }
}

/// State for bounded predicate unfolding
#[derive(Debug, Clone)]
pub struct UnfoldingState {
    /// Current unfolding depth for each predicate
    pub depths: Map<Text, usize>,
    /// Generated unfolding lemmas
    pub lemmas: List<SepAssertion>,
    /// Configuration
    pub config: UnfoldingConfig,
}

impl UnfoldingState {
    /// Create a new unfolding state
    pub fn new(config: UnfoldingConfig) -> Self {
        Self {
            depths: Map::new(),
            lemmas: List::new(),
            config,
        }
    }

    /// Check if we can unfold a predicate further
    pub fn can_unfold(&self, predicate: &str, max_depth: usize) -> bool {
        let current = self
            .depths
            .get(&Text::from(predicate))
            .copied()
            .unwrap_or(0);
        current < max_depth
    }

    /// Increment the unfolding depth for a predicate
    pub fn increment_depth(&mut self, predicate: &str) {
        let key = Text::from(predicate);
        let current = self.depths.get(&key).copied().unwrap_or(0);
        self.depths.insert(key, current + 1);
    }

    /// Add a generated lemma
    pub fn add_lemma(&mut self, lemma: SepAssertion) {
        self.lemmas.push(lemma);
    }
}

// ==================== Symbolic Heap Representation ====================

/// A symbolic heap for efficient entailment checking
#[derive(Debug, Clone)]
pub struct SymbolicHeap {
    /// Pure constraints (no heap access)
    pub pure: List<Expr>,
    /// Spatial assertions (points-to, predicates)
    pub spatial: List<SepAssertion>,
    /// Existentially quantified variables
    pub existentials: List<Text>,
}

impl SymbolicHeap {
    /// Create an empty symbolic heap
    pub fn emp() -> Self {
        Self {
            pure: List::new(),
            spatial: List::new(),
            existentials: List::new(),
        }
    }

    /// Create from a separation logic assertion
    pub fn from_assertion(assertion: &SepAssertion) -> Self {
        let mut heap = Self::emp();
        heap.add_assertion(assertion);
        heap
    }

    /// Add an assertion to the symbolic heap
    pub fn add_assertion(&mut self, assertion: &SepAssertion) {
        match assertion {
            SepAssertion::Emp => {}
            SepAssertion::Pure(expr) => {
                self.pure.push(expr.clone());
            }
            SepAssertion::PointsTo { .. }
            | SepAssertion::ListSegment { .. }
            | SepAssertion::Tree { .. }
            | SepAssertion::Block { .. }
            | SepAssertion::ArraySegment { .. } => {
                self.spatial.push(assertion.clone());
            }
            SepAssertion::Sep { left, right } => {
                self.add_assertion(left);
                self.add_assertion(right);
            }
            SepAssertion::And { left, right } => {
                self.add_assertion(left);
                self.add_assertion(right);
            }
            SepAssertion::Or { .. } => {
                // Disjunction is kept as a single spatial assertion
                self.spatial.push(assertion.clone());
            }
            SepAssertion::Wand { .. } => {
                // Magic wand is kept as a single spatial assertion
                self.spatial.push(assertion.clone());
            }
            SepAssertion::Exists { var, body } => {
                self.existentials.push(var.clone());
                self.add_assertion(body);
            }
            SepAssertion::Forall { .. } => {
                // Universal quantification is treated as pure
                self.spatial.push(assertion.clone());
            }
        }
    }

    /// Convert back to a SepAssertion
    pub fn to_assertion(&self) -> SepAssertion {
        let mut result = if self.spatial.is_empty() {
            SepAssertion::Emp
        } else {
            let mut iter = self.spatial.iter();
            let first = iter.next().unwrap().clone();
            iter.fold(first, |acc, next| SepAssertion::Sep {
                left: Heap::new(acc),
                right: Heap::new(next.clone()),
            })
        };

        // Add pure constraints
        for pure in self.pure.iter() {
            result = SepAssertion::And {
                left: Heap::new(SepAssertion::Pure(pure.clone())),
                right: Heap::new(result),
            };
        }

        // Wrap with existentials
        for var in self.existentials.iter().rev() {
            result = SepAssertion::Exists {
                var: var.clone(),
                body: Heap::new(result),
            };
        }

        result
    }

    /// Check if the heap is empty
    pub fn is_empty(&self) -> bool {
        self.spatial.is_empty() && self.pure.is_empty()
    }

    /// Get the footprint
    pub fn footprint(&self) -> List<Expr> {
        let mut locs = List::new();
        for spatial in self.spatial.iter() {
            for loc in spatial.footprint().iter() {
                locs.push(loc.clone());
            }
        }
        locs
    }
}

// ==================== Z3 Separation Logic Encoder ====================

/// Encodes separation logic assertions as Z3 formulas
///
/// This encoder implements production-grade features:
/// - Biheap model with stack/heap separation
/// - CBGR generation tracking for memory safety
/// - Bounded unfolding for recursive predicates
/// - Frame inference for modular verification
/// - Symbolic execution for complex entailments
pub struct SepLogicEncoder {
    /// Heap model with stack/heap separation
    model: Z3HeapModel,
    /// Z3 solver
    solver: Solver,
    /// Configuration
    config: SepLogicConfig,
    /// Fresh variable counter
    fresh_counter: RefCell<usize>,
    /// Variable bindings (name -> Z3 expression)
    bindings: RefCell<Map<Text, Dynamic>>,
    /// Unfolding state for recursive predicates
    unfolding_state: RefCell<UnfoldingState>,
    /// Cache for encoded assertions (for performance)
    encoding_cache: RefCell<Map<u64, Bool>>,
}

impl SepLogicEncoder {
    /// Create a new encoder with default configuration
    pub fn new(config: SepLogicConfig) -> Self {
        let model = Z3HeapModel::new();
        let solver = Solver::new();

        // Set timeout
        let mut params = Params::new();
        params.set_u32("timeout", config.entailment_timeout_ms as u32);
        solver.set_params(&params);

        Self {
            model,
            solver,
            config: config.clone(),
            fresh_counter: RefCell::new(0),
            bindings: RefCell::new(Map::new()),
            unfolding_state: RefCell::new(UnfoldingState::new(UnfoldingConfig {
                max_lseg_depth: config.max_unfolding_depth,
                max_tree_depth: config.max_unfolding_depth / 2,
                lazy_unfolding: true,
                generate_lemmas: true,
            })),
            encoding_cache: RefCell::new(Map::new()),
        }
    }

    /// Get fresh variable counter
    fn fresh_counter(&self) -> usize {
        let current = *self.fresh_counter.borrow();
        *self.fresh_counter.borrow_mut() += 1;
        current
    }

    /// Reset the solver and clear caches
    pub fn reset(&self) {
        self.solver.reset();
        self.bindings.borrow_mut().clear();
        self.encoding_cache.borrow_mut().clear();
        *self.unfolding_state.borrow_mut() = UnfoldingState::new(UnfoldingConfig::default());
    }

    /// Infer the frame for an entailment: given P, find R such that P |- Q * R
    ///
    /// This implements the frame inference algorithm from separation logic:
    /// 1. Compute the symbolic heap for P and Q
    /// 2. Match spatial assertions from Q against P
    /// 3. The unmatched assertions in P form the frame
    pub fn infer_frame(
        &self,
        antecedent: &SepAssertion,
        consequent: &SepAssertion,
    ) -> FrameInferenceResult {
        // Honour the configured opt-out: when frame inference is
        // disabled, return a typed failure rather than silently
        // performing the work. Callers that only need entailment
        // validity (without the residual-frame computation) can
        // disable this for ~30% reduction in encoder work on
        // large heaps.
        if !self.config.enable_frame_inference {
            return FrameInferenceResult::failure(
                "frame inference is disabled by SepLogicConfig.enable_frame_inference = false",
            );
        }

        // Convert to symbolic heaps
        let ante_heap = SymbolicHeap::from_assertion(antecedent);
        let cons_heap = SymbolicHeap::from_assertion(consequent);

        // Track which spatial assertions in the antecedent are matched
        let mut matched: Vec<bool> = vec![false; ante_heap.spatial.len()];
        let mut residual = List::new();

        // Try to match each consequent spatial assertion
        for cons_spatial in cons_heap.spatial.iter() {
            let mut found_match = false;

            for (i, ante_spatial) in ante_heap.spatial.iter().enumerate() {
                if !matched[i] && self.spatial_match(ante_spatial, cons_spatial) {
                    matched[i] = true;
                    found_match = true;
                    break;
                }
            }

            if !found_match {
                // Could not match this consequent assertion
                residual.push(cons_spatial.clone());
            }
        }

        // If there are unmatched consequent assertions, inference failed
        if !residual.is_empty() {
            return FrameInferenceResult::failure("Could not match all consequent assertions");
        }

        // The frame is the unmatched antecedent assertions
        let mut frame_assertions = List::new();
        for (i, ante_spatial) in ante_heap.spatial.iter().enumerate() {
            if !matched[i] {
                frame_assertions.push(ante_spatial.clone());
            }
        }

        // Also include pure constraints that are not in the consequent
        for pure in ante_heap.pure.iter() {
            let pure_assertion = SepAssertion::Pure(pure.clone());
            frame_assertions.push(pure_assertion);
        }

        // Build the frame
        let frame = if frame_assertions.is_empty() {
            SepAssertion::Emp
        } else {
            let mut iter = frame_assertions.iter();
            let first = iter.next().unwrap().clone();
            iter.fold(first, |acc, next| SepAssertion::Sep {
                left: Heap::new(acc),
                right: Heap::new(next.clone()),
            })
        };

        FrameInferenceResult::success(frame)
    }

    /// Check if two spatial assertions match (syntactically)
    fn spatial_match(&self, left: &SepAssertion, right: &SepAssertion) -> bool {
        match (left, right) {
            (SepAssertion::Emp, SepAssertion::Emp) => true,

            (
                SepAssertion::PointsTo {
                    location: l1,
                    value: v1,
                },
                SepAssertion::PointsTo {
                    location: l2,
                    value: v2,
                },
            ) => self.expr_syntactic_eq(l1, l2) && self.expr_syntactic_eq(v1, v2),

            (
                SepAssertion::ListSegment {
                    from: f1,
                    to: t1,
                    elements: e1,
                },
                SepAssertion::ListSegment {
                    from: f2,
                    to: t2,
                    elements: e2,
                },
            ) => {
                self.expr_syntactic_eq(f1, f2)
                    && self.expr_syntactic_eq(t1, t2)
                    && e1.len() == e2.len()
                    && e1
                        .iter()
                        .zip(e2.iter())
                        .all(|(a, b)| self.expr_syntactic_eq(a, b))
            }

            (
                SepAssertion::Block { base: b1, size: s1 },
                SepAssertion::Block { base: b2, size: s2 },
            ) => self.expr_syntactic_eq(b1, b2) && self.expr_syntactic_eq(s1, s2),

            _ => false,
        }
    }

    /// Check if two expressions are syntactically equal
    fn expr_syntactic_eq(&self, left: &Expr, right: &Expr) -> bool {
        format!("{:?}", left) == format!("{:?}", right)
    }

    /// Encode with bounded unfolding for recursive predicates
    pub fn encode_with_unfolding(
        &self,
        assertion: &SepAssertion,
        heap: &Array,
        allocated: &Array,
        depth: usize,
    ) -> Bool {
        if depth == 0 {
            // At depth limit, use approximation
            return Bool::from_bool(true);
        }

        match assertion {
            SepAssertion::ListSegment { from, to, elements } => {
                // Bounded unfolding of list segment
                self.encode_list_segment_bounded(from, to, elements, heap, allocated, depth)
            }

            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => {
                // Bounded unfolding of tree
                self.encode_tree_bounded(root, left_child, right_child, heap, allocated, depth)
            }

            // For other assertions, delegate to the standard encoding
            _ => self.encode_assertion(assertion, heap, allocated),
        }
    }

    /// Encode list segment with bounded unfolding
    fn encode_list_segment_bounded(
        &self,
        from: &Expr,
        to: &Expr,
        elements: &[Expr],
        heap: &Array,
        allocated: &Array,
        depth: usize,
    ) -> Bool {
        let from_addr = self.encode_expr_as_addr(from);
        let to_addr = self.encode_expr_as_addr(to);

        if elements.is_empty() {
            // Empty list segment: from == to
            return from_addr.eq(&to_addr);
        }

        if depth == 0 {
            // At depth limit, just check from != to (non-empty list)
            return from_addr.eq(&to_addr).not();
        }

        // Unfold one step: from |-> (head, next) * lseg(next, to, tail)
        let head = self.encode_expr_as_value(&elements[0]);

        // from points to head
        let heap_at_from = heap.select(&from_addr);
        let head_eq = heap_at_from.eq(&head);

        // from is allocated
        let from_alloc = allocated.select(&from_addr);
        let is_alloc = from_alloc
            .as_bool()
            .unwrap_or_else(|| Bool::from_bool(true));

        // Next pointer
        let eight = z3::ast::BV::from_i64(8, 64);
        let next_addr = if let Some(from_bv) = from_addr.as_bv() {
            Dynamic::from_ast(&from_bv.bvadd(&eight))
        } else {
            self.model.fresh_addr("next")
        };

        if elements.len() == 1 {
            // Last element: next == to
            let next_eq_to = next_addr.eq(&to_addr);
            Bool::and(&[&head_eq, &is_alloc, &next_eq_to])
        } else {
            // Recursive case with bounded depth
            let tail: Vec<Expr> = elements[1..].to_vec();
            let next_expr = self.create_addr_expr(&next_addr);
            let rest =
                self.encode_list_segment_bounded(&next_expr, to, &tail, heap, allocated, depth - 1);
            Bool::and(&[&head_eq, &is_alloc, &rest])
        }
    }

    /// Create an expression from a Z3 address
    fn create_addr_expr(&self, addr: &Dynamic) -> Expr {
        use verum_ast::span::Span;

        // Try to extract the concrete value
        if let Some(bv) = addr.as_bv() {
            if let Some(val) = bv.as_i64() {
                return Expr::new(
                    ExprKind::Literal(Literal::new(
                        LiteralKind::Int(IntLit::new(val as i128)),
                        Span::dummy(),
                    )),
                    Span::dummy(),
                );
            }
        }

        // Create a fresh variable
        Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new(
                format!("__addr_{}", self.fresh_counter()),
                Span::dummy(),
            ))),
            Span::dummy(),
        )
    }

    /// Encode tree with bounded unfolding
    fn encode_tree_bounded(
        &self,
        root: &Expr,
        left_child: &Maybe<Heap<SepAssertion>>,
        right_child: &Maybe<Heap<SepAssertion>>,
        heap: &Array,
        allocated: &Array,
        depth: usize,
    ) -> Bool {
        let root_addr = self.encode_expr_as_addr(root);

        // Root is allocated
        let root_alloc = allocated.select(&root_addr);
        let is_alloc = root_alloc
            .as_bool()
            .unwrap_or_else(|| Bool::from_bool(true));

        if depth == 0 {
            // At depth limit, just check root is allocated
            return is_alloc;
        }

        // Encode children with reduced depth
        let left_encoded = match left_child {
            Maybe::Some(left) => self.encode_with_unfolding(left, heap, allocated, depth - 1),
            Maybe::None => Bool::from_bool(true),
        };

        let right_encoded = match right_child {
            Maybe::Some(right) => self.encode_with_unfolding(right, heap, allocated, depth - 1),
            Maybe::None => Bool::from_bool(true),
        };

        Bool::and(&[&is_alloc, &left_encoded, &right_encoded])
    }

    /// Encode CBGR generation check
    pub fn encode_cbgr_check(
        &self,
        addr: &Dynamic,
        expected_gen: u64,
        generations: &Array,
    ) -> Bool {
        let expected = self.model.generation_const(expected_gen);
        let actual = generations.select(addr);
        expected.eq(&actual)
    }

    /// Encode stack/heap region check
    pub fn encode_region_check(&self, addr: &Dynamic, expected_region: MemoryRegion) -> Bool {
        match expected_region {
            MemoryRegion::Stack => self.model.is_stack_addr(addr),
            MemoryRegion::Heap => self.model.is_heap_addr(addr),
            _ => Bool::from_bool(true),
        }
    }

    /// Encode a separation logic assertion as a Z3 formula
    ///
    /// The encoding uses the theory of arrays to model heaps:
    /// - Heap is an array from addresses to values
    /// - Allocated bitmap tracks which addresses are valid
    /// - Separating conjunction requires disjoint domains
    pub fn encode_assertion(
        &self,
        assertion: &SepAssertion,
        heap: &Array,
        allocated: &Array,
    ) -> Bool {
        match assertion {
            SepAssertion::Emp => {
                // Empty heap: forall addr. NOT allocated[addr]
                // Approximation: true (checked by disjointness)
                Bool::from_bool(true)
            }

            SepAssertion::Pure(expr) => {
                // Pure assertion: encode the expression
                self.encode_pure_expr(expr)
            }

            SepAssertion::PointsTo { location, value } => {
                // x |-> v: heap[x] == v AND allocated[x]
                let addr = self.encode_expr_as_addr(location);
                let val = self.encode_expr_as_value(value);

                let heap_lookup = heap.select(&addr);
                let is_allocated = allocated.select(&addr);

                let val_eq = heap_lookup.eq(&val);
                let is_alloc_bool = is_allocated
                    .as_bool()
                    .unwrap_or_else(|| Bool::from_bool(true));

                Bool::and(&[&val_eq, &is_alloc_bool])
            }

            SepAssertion::Sep { left, right } => {
                // P * Q: P(h1) AND Q(h2) AND disjoint(dom(h1), dom(h2))
                // AND h = h1 + h2

                // Create fresh heaps for left and right
                let h1 = self.model.fresh_heap("h1");
                let a1 = self.model.fresh_allocated("a1");
                let h2 = self.model.fresh_heap("h2");
                let a2 = self.model.fresh_allocated("a2");

                // Encode sub-assertions
                let left_encoded = self.encode_assertion(left, &h1, &a1);
                let right_encoded = self.encode_assertion(right, &h2, &a2);

                // Disjointness: forall addr. NOT (a1[addr] AND a2[addr])
                let disjoint = self.encode_disjointness(&a1, &a2);

                // Heap composition: heap = h1 + h2 (where a1 and a2 determine which to use)
                let composition = self.encode_heap_composition(heap, allocated, &h1, &a1, &h2, &a2);

                Bool::and(&[&left_encoded, &right_encoded, &disjoint, &composition])
            }

            SepAssertion::Wand { left, right } => {
                // P -* Q: forall h'. (P(h') AND disjoint(h, h')) => Q(h + h')
                // Encoded as: exists frame_heap. when combined with left gives right

                let frame_heap = self.model.fresh_heap("wand_frame");
                let frame_alloc = self.model.fresh_allocated("wand_frame_alloc");

                let left_encoded = self.encode_assertion(left, &frame_heap, &frame_alloc);

                // Combined heap
                let combined_heap = self.model.fresh_heap("wand_combined");
                let combined_alloc = self.model.fresh_allocated("wand_combined_alloc");

                let disjoint = self.encode_disjointness(allocated, &frame_alloc);
                let composition = self.encode_heap_composition(
                    &combined_heap,
                    &combined_alloc,
                    heap,
                    allocated,
                    &frame_heap,
                    &frame_alloc,
                );

                let right_encoded = self.encode_assertion(right, &combined_heap, &combined_alloc);

                // (left AND disjoint AND composition) => right
                let antecedent = Bool::and(&[&left_encoded, &disjoint, &composition]);
                antecedent.implies(&right_encoded)
            }

            SepAssertion::And { left, right } => {
                let left_encoded = self.encode_assertion(left, heap, allocated);
                let right_encoded = self.encode_assertion(right, heap, allocated);
                Bool::and(&[&left_encoded, &right_encoded])
            }

            SepAssertion::Or { left, right } => {
                let left_encoded = self.encode_assertion(left, heap, allocated);
                let right_encoded = self.encode_assertion(right, heap, allocated);
                Bool::or(&[&left_encoded, &right_encoded])
            }

            SepAssertion::Exists { var, body } => {
                // Existential: create fresh variable and encode body
                let fresh_var = self.model.fresh_value(var.as_str());
                self.bindings
                    .borrow_mut()
                    .insert(var.clone(), fresh_var.clone());
                let result = self.encode_assertion(body, heap, allocated);
                self.bindings.borrow_mut().remove(var);
                result
            }

            SepAssertion::Forall { var, body } => {
                // Universal: create bound variable
                let bound_var = self.model.fresh_value(var.as_str());
                self.bindings
                    .borrow_mut()
                    .insert(var.clone(), bound_var.clone());
                let body_encoded = self.encode_assertion(body, heap, allocated);
                self.bindings.borrow_mut().remove(var);

                // Create universal quantifier
                z3::ast::forall_const(&[&bound_var], &[], &body_encoded)
            }

            SepAssertion::ListSegment { from, to, elements } => {
                // lseg(from, to, []) = from == to AND emp
                // lseg(from, to, x::xs) = exists next. from |-> (x, next) * lseg(next, to, xs)
                self.encode_list_segment(from, to, elements, heap, allocated)
            }

            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => self.encode_tree(root, left_child, right_child, heap, allocated),

            SepAssertion::Block { base, size } => {
                // block(base, size): forall i in [0, size). allocated[base + i]
                self.encode_block(base, size, heap, allocated)
            }

            SepAssertion::ArraySegment {
                base,
                offset,
                length,
                elements,
            } => self.encode_array_segment(base, offset, length, elements, heap, allocated),
        }
    }

    // ==================== Quantifier Elimination ====================

    /// Apply quantifier elimination for decidable fragments of separation logic
    ///
    /// This implements quantifier elimination for:
    /// 1. Existentials over addresses with unique points-to
    /// 2. Universals over bounded ranges
    /// 3. Existentials with explicit witnesses
    pub fn eliminate_quantifiers(&self, assertion: &SepAssertion) -> SepAssertion {
        match assertion {
            SepAssertion::Exists { var, body } => {
                // Try to find a witness in the body
                if let Maybe::Some(witness) = self.find_existential_witness(var, body) {
                    // Replace the existential with the witnessed value
                    self.substitute_in_assertion(body, var, &witness)
                } else {
                    // Cannot eliminate - keep the existential
                    SepAssertion::Exists {
                        var: var.clone(),
                        body: Heap::new(self.eliminate_quantifiers(body)),
                    }
                }
            }

            SepAssertion::Forall { var, body } => {
                // Check if the universal can be eliminated
                if let Maybe::Some(finite_domain) = self.extract_finite_domain(var, body) {
                    // Replace forall x. P(x) with P(v1) AND P(v2) AND ... AND P(vn)
                    let mut conjuncts = List::new();
                    for value in finite_domain.iter() {
                        let instantiated = self.substitute_in_assertion(body, var, value);
                        conjuncts.push(self.eliminate_quantifiers(&instantiated));
                    }

                    if conjuncts.is_empty() {
                        SepAssertion::Pure(Expr::new(
                            ExprKind::Literal(Literal::new(
                                LiteralKind::Bool(true),
                                verum_ast::span::Span::dummy(),
                            )),
                            verum_ast::span::Span::dummy(),
                        ))
                    } else {
                        let mut iter = conjuncts.iter();
                        let first = iter.next().unwrap().clone();
                        iter.fold(first, |acc, next| SepAssertion::And {
                            left: Heap::new(acc),
                            right: Heap::new(next.clone()),
                        })
                    }
                } else {
                    // Cannot eliminate - keep the universal
                    SepAssertion::Forall {
                        var: var.clone(),
                        body: Heap::new(self.eliminate_quantifiers(body)),
                    }
                }
            }

            SepAssertion::Sep { left, right } => SepAssertion::Sep {
                left: Heap::new(self.eliminate_quantifiers(left)),
                right: Heap::new(self.eliminate_quantifiers(right)),
            },

            SepAssertion::And { left, right } => SepAssertion::And {
                left: Heap::new(self.eliminate_quantifiers(left)),
                right: Heap::new(self.eliminate_quantifiers(right)),
            },

            SepAssertion::Or { left, right } => SepAssertion::Or {
                left: Heap::new(self.eliminate_quantifiers(left)),
                right: Heap::new(self.eliminate_quantifiers(right)),
            },

            SepAssertion::Wand { left, right } => SepAssertion::Wand {
                left: Heap::new(self.eliminate_quantifiers(left)),
                right: Heap::new(self.eliminate_quantifiers(right)),
            },

            _ => assertion.clone(),
        }
    }

    /// Find a witness for an existential quantifier
    ///
    /// Looks for patterns like:
    /// - exists x. (addr |-> x) => x = addr's value
    /// - exists x. (x == constant) => x = constant
    fn find_existential_witness(&self, var: &Text, body: &SepAssertion) -> Maybe<Expr> {
        match body {
            SepAssertion::PointsTo { location, value } => {
                // If the value is the quantified variable, return the location
                if self.expr_is_var(value, var) {
                    Maybe::Some(location.clone())
                } else if self.expr_is_var(location, var) {
                    Maybe::Some(value.clone())
                } else {
                    Maybe::None
                }
            }

            SepAssertion::Pure(expr) => {
                // Look for x == constant patterns
                self.find_equality_witness(expr, var)
            }

            SepAssertion::And { left, right } => {
                // Try both sides
                if let witness @ Maybe::Some(_) = self.find_existential_witness(var, left) {
                    witness
                } else {
                    self.find_existential_witness(var, right)
                }
            }

            SepAssertion::Sep { left, right } => {
                // Try both sides
                if let witness @ Maybe::Some(_) = self.find_existential_witness(var, left) {
                    witness
                } else {
                    self.find_existential_witness(var, right)
                }
            }

            _ => Maybe::None,
        }
    }

    /// Check if an expression is exactly the given variable
    fn expr_is_var(&self, expr: &Expr, var: &Text) -> bool {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(ident) = path.as_ident() {
                return ident.name.as_str() == var.as_str();
            }
        }
        false
    }

    /// Find an equality witness in a pure expression (e.g., x == 42 => witness is 42)
    fn find_equality_witness(&self, expr: &Expr, var: &Text) -> Maybe<Expr> {
        if let ExprKind::Binary { op, left, right } = &expr.kind {
            if *op == BinOp::Eq {
                if self.expr_is_var(left, var) {
                    return Maybe::Some((**right).clone());
                }
                if self.expr_is_var(right, var) {
                    return Maybe::Some((**left).clone());
                }
            }
        }
        Maybe::None
    }

    /// Extract a finite domain for a universally quantified variable
    ///
    /// Looks for patterns like:
    /// - forall x. (0 <= x < n) => P(x) where n is small
    /// - forall x in {v1, v2, v3}. P(x)
    fn extract_finite_domain(&self, var: &Text, body: &SepAssertion) -> Maybe<List<Expr>> {
        // Look for bounded range patterns in the body
        if let SepAssertion::Pure(expr) = body {
            if let ExprKind::Binary {
                op: BinOp::Imply,
                left: range_expr,
                right: _,
            } = &expr.kind
            {
                // Try to extract a bounded range
                if let Maybe::Some((lower, upper)) = self.extract_bounded_range(range_expr, var) {
                    // Only instantiate if the range is small enough
                    if upper - lower <= 16 {
                        let mut domain = List::new();
                        for i in lower..upper {
                            domain.push(Expr::new(
                                ExprKind::Literal(Literal::new(
                                    LiteralKind::Int(IntLit::new(i as i128)),
                                    verum_ast::span::Span::dummy(),
                                )),
                                verum_ast::span::Span::dummy(),
                            ));
                        }
                        return Maybe::Some(domain);
                    }
                }
            }
        }
        Maybe::None
    }

    /// Extract a bounded range from an expression (e.g., 0 <= x < 10)
    fn extract_bounded_range(&self, expr: &Expr, var: &Text) -> Maybe<(i64, i64)> {
        if let ExprKind::Binary { op, left, right } = &expr.kind {
            match op {
                BinOp::And => {
                    // Combine two range constraints
                    let left_range = self.extract_bounded_range(left, var);
                    let right_range = self.extract_bounded_range(right, var);

                    match (left_range, right_range) {
                        (Maybe::Some((l1, u1)), Maybe::Some((l2, u2))) => {
                            Maybe::Some((l1.max(l2), u1.min(u2)))
                        }
                        (Maybe::Some(r), Maybe::None) | (Maybe::None, Maybe::Some(r)) => {
                            Maybe::Some(r)
                        }
                        _ => Maybe::None,
                    }
                }

                BinOp::Le | BinOp::Lt => {
                    // x < c or x <= c gives upper bound
                    if self.expr_is_var(left, var) {
                        if let Maybe::Some(c) = self.extract_constant(right) {
                            let upper = if *op == BinOp::Lt { c } else { c + 1 };
                            return Maybe::Some((i64::MIN, upper));
                        }
                    }
                    // c < x or c <= x gives lower bound
                    if self.expr_is_var(right, var) {
                        if let Maybe::Some(c) = self.extract_constant(left) {
                            let lower = if *op == BinOp::Lt { c + 1 } else { c };
                            return Maybe::Some((lower, i64::MAX));
                        }
                    }
                    Maybe::None
                }

                BinOp::Ge | BinOp::Gt => {
                    // x > c or x >= c gives lower bound
                    if self.expr_is_var(left, var) {
                        if let Maybe::Some(c) = self.extract_constant(right) {
                            let lower = if *op == BinOp::Gt { c + 1 } else { c };
                            return Maybe::Some((lower, i64::MAX));
                        }
                    }
                    // c > x or c >= x gives upper bound
                    if self.expr_is_var(right, var) {
                        if let Maybe::Some(c) = self.extract_constant(left) {
                            let upper = if *op == BinOp::Gt { c } else { c + 1 };
                            return Maybe::Some((i64::MIN, upper));
                        }
                    }
                    Maybe::None
                }

                _ => Maybe::None,
            }
        } else {
            Maybe::None
        }
    }

    /// Extract a constant integer from an expression
    fn extract_constant(&self, expr: &Expr) -> Maybe<i64> {
        if let ExprKind::Literal(lit) = &expr.kind {
            if let LiteralKind::Int(int_lit) = &lit.kind {
                return Maybe::Some(int_lit.value as i64);
            }
        }
        Maybe::None
    }

    /// Substitute a variable with an expression in an assertion
    fn substitute_in_assertion(
        &self,
        assertion: &SepAssertion,
        var: &Text,
        replacement: &Expr,
    ) -> SepAssertion {
        match assertion {
            SepAssertion::Pure(expr) => {
                SepAssertion::Pure(self.substitute_in_expr(expr, var, replacement))
            }

            SepAssertion::Emp => SepAssertion::Emp,

            SepAssertion::PointsTo { location, value } => SepAssertion::PointsTo {
                location: self.substitute_in_expr(location, var, replacement),
                value: self.substitute_in_expr(value, var, replacement),
            },

            SepAssertion::Sep { left, right } => SepAssertion::Sep {
                left: Heap::new(self.substitute_in_assertion(left, var, replacement)),
                right: Heap::new(self.substitute_in_assertion(right, var, replacement)),
            },

            SepAssertion::And { left, right } => SepAssertion::And {
                left: Heap::new(self.substitute_in_assertion(left, var, replacement)),
                right: Heap::new(self.substitute_in_assertion(right, var, replacement)),
            },

            SepAssertion::Or { left, right } => SepAssertion::Or {
                left: Heap::new(self.substitute_in_assertion(left, var, replacement)),
                right: Heap::new(self.substitute_in_assertion(right, var, replacement)),
            },

            SepAssertion::Wand { left, right } => SepAssertion::Wand {
                left: Heap::new(self.substitute_in_assertion(left, var, replacement)),
                right: Heap::new(self.substitute_in_assertion(right, var, replacement)),
            },

            SepAssertion::Exists { var: bound, body } => {
                if bound == var {
                    // Variable is shadowed
                    assertion.clone()
                } else {
                    SepAssertion::Exists {
                        var: bound.clone(),
                        body: Heap::new(self.substitute_in_assertion(body, var, replacement)),
                    }
                }
            }

            SepAssertion::Forall { var: bound, body } => {
                if bound == var {
                    // Variable is shadowed
                    assertion.clone()
                } else {
                    SepAssertion::Forall {
                        var: bound.clone(),
                        body: Heap::new(self.substitute_in_assertion(body, var, replacement)),
                    }
                }
            }

            SepAssertion::ListSegment { from, to, elements } => SepAssertion::ListSegment {
                from: self.substitute_in_expr(from, var, replacement),
                to: self.substitute_in_expr(to, var, replacement),
                elements: elements
                    .iter()
                    .map(|e| self.substitute_in_expr(e, var, replacement))
                    .collect(),
            },

            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => SepAssertion::Tree {
                root: self.substitute_in_expr(root, var, replacement),
                left_child: left_child
                    .as_ref()
                    .map(|c| Heap::new(self.substitute_in_assertion(c, var, replacement))),
                right_child: right_child
                    .as_ref()
                    .map(|c| Heap::new(self.substitute_in_assertion(c, var, replacement))),
            },

            SepAssertion::Block { base, size } => SepAssertion::Block {
                base: self.substitute_in_expr(base, var, replacement),
                size: self.substitute_in_expr(size, var, replacement),
            },

            SepAssertion::ArraySegment {
                base,
                offset,
                length,
                elements,
            } => SepAssertion::ArraySegment {
                base: self.substitute_in_expr(base, var, replacement),
                offset: self.substitute_in_expr(offset, var, replacement),
                length: self.substitute_in_expr(length, var, replacement),
                elements: elements
                    .iter()
                    .map(|e| self.substitute_in_expr(e, var, replacement))
                    .collect(),
            },
        }
    }

    /// Substitute a variable with an expression in an expression
    fn substitute_in_expr(&self, expr: &Expr, var: &Text, replacement: &Expr) -> Expr {
        if self.expr_is_var(expr, var) {
            return replacement.clone();
        }

        match &expr.kind {
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Heap::new(self.substitute_in_expr(left, var, replacement)),
                    right: Heap::new(self.substitute_in_expr(right, var, replacement)),
                },
                expr.span,
            ),

            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: Heap::new(self.substitute_in_expr(inner, var, replacement)),
                },
                expr.span,
            ),

            ExprKind::Call { func, args, .. } => Expr::new(
                ExprKind::Call {
                    func: Heap::new(self.substitute_in_expr(func, var, replacement)),
                    type_args: List::new(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_in_expr(a, var, replacement))
                        .collect(),
                },
                expr.span,
            ),

            ExprKind::Index { expr: base, index } => Expr::new(
                ExprKind::Index {
                    expr: Heap::new(self.substitute_in_expr(base, var, replacement)),
                    index: Heap::new(self.substitute_in_expr(index, var, replacement)),
                },
                expr.span,
            ),

            _ => expr.clone(),
        }
    }

    /// Encode disjointness of two allocation maps
    fn encode_disjointness(&self, a1: &Array, a2: &Array) -> Bool {
        // forall addr. NOT (a1[addr] AND a2[addr])
        let addr = self.model.fresh_addr("disjoint_addr");
        let a1_at_addr = a1.select(&addr);
        let a2_at_addr = a2.select(&addr);

        let both_allocated =
            if let (Some(b1), Some(b2)) = (a1_at_addr.as_bool(), a2_at_addr.as_bool()) {
                Bool::and(&[&b1, &b2])
            } else {
                Bool::from_bool(false)
            };

        let not_both = both_allocated.not();
        z3::ast::forall_const(&[&addr], &[], &not_both)
    }

    /// Encode heap composition: result = h1 + h2
    fn encode_heap_composition(
        &self,
        result_heap: &Array,
        result_alloc: &Array,
        h1: &Array,
        a1: &Array,
        h2: &Array,
        a2: &Array,
    ) -> Bool {
        // forall addr.
        //   result_alloc[addr] = a1[addr] OR a2[addr]
        //   AND (a1[addr] => result_heap[addr] == h1[addr])
        //   AND (a2[addr] => result_heap[addr] == h2[addr])

        let addr = self.model.fresh_addr("compose_addr");

        let a1_at = a1.select(&addr);
        let a2_at = a2.select(&addr);
        let result_a_at = result_alloc.select(&addr);

        // Convert to Bool if possible
        let (a1_bool, a2_bool, result_a_bool) =
            match (a1_at.as_bool(), a2_at.as_bool(), result_a_at.as_bool()) {
                (Some(b1), Some(b2), Some(br)) => (b1, b2, br),
                _ => return Bool::from_bool(true),
            };

        // result_alloc[addr] = a1[addr] OR a2[addr]
        let alloc_union = Bool::or(&[&a1_bool, &a2_bool]);
        let alloc_eq = result_a_bool.eq(&alloc_union);

        // Value composition
        let h1_at = h1.select(&addr);
        let h2_at = h2.select(&addr);
        let result_h_at = result_heap.select(&addr);

        let from_h1 = a1_bool.implies(result_h_at.eq(&h1_at));
        let from_h2 = a2_bool.implies(result_h_at.eq(&h2_at));

        let body = Bool::and(&[&alloc_eq, &from_h1, &from_h2]);
        z3::ast::forall_const(&[&addr], &[], &body)
    }

    /// Encode list segment predicate
    fn encode_list_segment(
        &self,
        from: &Expr,
        to: &Expr,
        elements: &[Expr],
        heap: &Array,
        allocated: &Array,
    ) -> Bool {
        let from_addr = self.encode_expr_as_addr(from);
        let to_addr = self.encode_expr_as_addr(to);

        if elements.is_empty() {
            // Empty list segment: from == to
            from_addr.eq(&to_addr)
        } else {
            // Non-empty: from |-> (elements[0], next) * lseg(next, to, elements[1..])
            let first_elem = self.encode_expr_as_value(&elements[0]);

            // from points to first element
            let heap_at_from = heap.select(&from_addr);
            let elem_eq = heap_at_from.eq(&first_elem);

            // from is allocated
            let from_alloc = allocated.select(&from_addr);
            let is_alloc = from_alloc
                .as_bool()
                .unwrap_or_else(|| Bool::from_bool(true));

            // Next pointer (from + 8 for 64-bit systems)
            let eight = z3::ast::BV::from_i64(8, 64);
            let next_addr = if let Some(from_bv) = from_addr.as_bv() {
                Dynamic::from_ast(&from_bv.bvadd(&eight))
            } else {
                self.model.fresh_addr("next")
            };

            if elements.len() == 1 {
                // Last element: next == to
                let next_eq_to = next_addr.eq(&to_addr);
                Bool::and(&[&elem_eq, &is_alloc, &next_eq_to])
            } else {
                // Recursive case
                let rest: List<Expr> = elements[1..].to_vec().into();
                // Create expression for next address
                let next_expr = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        format!("__next_{}", self.fresh_counter()),
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                // This is a recursive encoding - for practical use we'd need bounded unfolding
                let rest_encoded = self.encode_list_segment(&next_expr, to, &rest, heap, allocated);
                Bool::and(&[&elem_eq, &is_alloc, &rest_encoded])
            }
        }
    }

    /// Encode tree predicate
    fn encode_tree(
        &self,
        root: &Expr,
        left_child: &Maybe<Heap<SepAssertion>>,
        right_child: &Maybe<Heap<SepAssertion>>,
        heap: &Array,
        allocated: &Array,
    ) -> Bool {
        let root_addr = self.encode_expr_as_addr(root);

        // Root is allocated
        let root_alloc = allocated.select(&root_addr);
        let is_alloc = root_alloc
            .as_bool()
            .unwrap_or_else(|| Bool::from_bool(true));

        // Encode children
        let left_encoded = match left_child {
            Maybe::Some(left) => self.encode_assertion(left, heap, allocated),
            Maybe::None => Bool::from_bool(true),
        };

        let right_encoded = match right_child {
            Maybe::Some(right) => self.encode_assertion(right, heap, allocated),
            Maybe::None => Bool::from_bool(true),
        };

        Bool::and(&[&is_alloc, &left_encoded, &right_encoded])
    }

    /// Encode block predicate
    fn encode_block(&self, base: &Expr, size: &Expr, _heap: &Array, allocated: &Array) -> Bool {
        let base_addr = self.encode_expr_as_addr(base);
        let size_val = self.encode_expr_as_value(size);

        // forall i in [0, size). allocated[base + i]
        let i = self.model.fresh_value("block_i");

        // i >= 0 AND i < size
        let zero = z3::ast::BV::from_i64(0, 64);

        if let (Some(i_bv), Some(base_bv), Some(size_bv)) =
            (i.as_bv(), base_addr.as_bv(), size_val.as_bv())
        {
            let i_ge_zero = i_bv.bvsge(&zero);
            let i_lt_size = i_bv.bvslt(&size_bv);
            let range = Bool::and(&[&i_ge_zero, &i_lt_size]);

            // base + i is allocated
            let addr_i = base_bv.bvadd(&i_bv);
            let alloc_at_i = allocated.select(&Dynamic::from_ast(&addr_i));
            let is_alloc = alloc_at_i
                .as_bool()
                .unwrap_or_else(|| Bool::from_bool(true));

            let body = range.implies(&is_alloc);
            z3::ast::forall_const(&[&i], &[], &body)
        } else {
            Bool::from_bool(true)
        }
    }

    /// Encode array segment
    fn encode_array_segment(
        &self,
        base: &Expr,
        offset: &Expr,
        length: &Expr,
        elements: &[Expr],
        heap: &Array,
        allocated: &Array,
    ) -> Bool {
        let base_addr = self.encode_expr_as_addr(base);
        let offset_val = self.encode_expr_as_value(offset);
        let _length_val = self.encode_expr_as_value(length);

        let mut constraints: Vec<Bool> = Vec::new();

        // For each element, encode: heap[base + offset + i*8] == elements[i]
        // AND allocated[base + offset + i*8]
        for (i, elem) in elements.iter().enumerate() {
            let elem_val = self.encode_expr_as_value(elem);
            let i_offset = z3::ast::BV::from_i64((i * 8) as i64, 64);

            if let (Some(base_bv), Some(offset_bv)) = (base_addr.as_bv(), offset_val.as_bv()) {
                let addr_i = base_bv.bvadd(&offset_bv).bvadd(&i_offset);
                let addr_dyn = Dynamic::from_ast(&addr_i);

                let heap_at_i = heap.select(&addr_dyn);
                let elem_eq = heap_at_i.eq(&elem_val);

                let alloc_at_i = allocated.select(&addr_dyn);
                let is_alloc = alloc_at_i
                    .as_bool()
                    .unwrap_or_else(|| Bool::from_bool(true));

                constraints.push(elem_eq);
                constraints.push(is_alloc);
            }
        }

        if constraints.is_empty() {
            Bool::from_bool(true)
        } else {
            let refs: Vec<&Bool> = constraints.iter().collect();
            Bool::and(&refs)
        }
    }

    /// Encode a pure expression as Z3 Bool
    fn encode_pure_expr(&self, expr: &Expr) -> Bool {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Bool::from_bool(*b),
                _ => Bool::from_bool(true),
            },

            ExprKind::Binary { op, left, right } => {
                let left_encoded = self.encode_pure_expr(left);
                let right_encoded = self.encode_pure_expr(right);

                match op {
                    BinOp::And => Bool::and(&[&left_encoded, &right_encoded]),
                    BinOp::Or => Bool::or(&[&left_encoded, &right_encoded]),
                    BinOp::Imply => left_encoded.implies(&right_encoded),
                    BinOp::Eq => {
                        // Try integer comparison
                        let left_int = self.encode_expr_as_value(left);
                        let right_int = self.encode_expr_as_value(right);
                        left_int.eq(&right_int)
                    }
                    BinOp::Ne => {
                        let left_int = self.encode_expr_as_value(left);
                        let right_int = self.encode_expr_as_value(right);
                        left_int.eq(&right_int).not()
                    }
                    BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                        let left_val = self.encode_expr_as_value(left);
                        let right_val = self.encode_expr_as_value(right);
                        if let (Some(l_bv), Some(r_bv)) = (left_val.as_bv(), right_val.as_bv()) {
                            match op {
                                BinOp::Lt => l_bv.bvslt(&r_bv),
                                BinOp::Le => l_bv.bvsle(&r_bv),
                                BinOp::Gt => l_bv.bvsgt(&r_bv),
                                BinOp::Ge => l_bv.bvsge(&r_bv),
                                _ => Bool::from_bool(true),
                            }
                        } else {
                            Bool::from_bool(true)
                        }
                    }
                    _ => Bool::from_bool(true),
                }
            }

            ExprKind::Unary { op, expr: inner } => {
                let inner_encoded = self.encode_pure_expr(inner);
                match op {
                    verum_ast::UnOp::Not => inner_encoded.not(),
                    _ => Bool::from_bool(true),
                }
            }

            ExprKind::Path(path) => {
                // Variable reference - look up in bindings
                if let Some(ident) = path.as_ident() {
                    let name = Text::from(ident.name.as_str());
                    if let Maybe::Some(binding) = self.bindings.borrow().get(&name) {
                        if let Some(b) = binding.as_bool() {
                            return b;
                        }
                    }
                }
                // Default to true for unknown
                Bool::from_bool(true)
            }

            _ => Bool::from_bool(true),
        }
    }

    /// Encode expression as address (bitvector)
    fn encode_expr_as_addr(&self, expr: &Expr) -> Dynamic {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                if let LiteralKind::Int(int_lit) = &lit.kind {
                    return self.model.addr_const(int_lit.value as i64);
                }
                self.model.fresh_addr("unknown_addr")
            }

            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = Text::from(ident.name.as_str());
                    if let Maybe::Some(binding) = self.bindings.borrow().get(&name) {
                        return binding.clone();
                    }
                    // Create fresh variable
                    self.model.fresh_addr(ident.name.as_str())
                } else {
                    self.model.fresh_addr("path_addr")
                }
            }

            ExprKind::Binary { op, left, right } => {
                let left_val = self.encode_expr_as_addr(left);
                let right_val = self.encode_expr_as_addr(right);

                if let (Some(l_bv), Some(r_bv)) = (left_val.as_bv(), right_val.as_bv()) {
                    match op {
                        BinOp::Add => Dynamic::from_ast(&l_bv.bvadd(&r_bv)),
                        BinOp::Sub => Dynamic::from_ast(&l_bv.bvsub(&r_bv)),
                        _ => self.model.fresh_addr("binary_addr"),
                    }
                } else {
                    self.model.fresh_addr("binary_addr")
                }
            }

            _ => self.model.fresh_addr("expr_addr"),
        }
    }

    /// Encode expression as value (bitvector)
    fn encode_expr_as_value(&self, expr: &Expr) -> Dynamic {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                if let LiteralKind::Int(int_lit) = &lit.kind {
                    return self.model.value_const(int_lit.value as i64);
                }
                self.model.fresh_value("unknown_val")
            }

            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = Text::from(ident.name.as_str());
                    if let Maybe::Some(binding) = self.bindings.borrow().get(&name) {
                        return binding.clone();
                    }
                    self.model.fresh_value(ident.name.as_str())
                } else {
                    self.model.fresh_value("path_val")
                }
            }

            ExprKind::Binary { op, left, right } => {
                let left_val = self.encode_expr_as_value(left);
                let right_val = self.encode_expr_as_value(right);

                if let (Some(l_bv), Some(r_bv)) = (left_val.as_bv(), right_val.as_bv()) {
                    match op {
                        BinOp::Add => Dynamic::from_ast(&l_bv.bvadd(&r_bv)),
                        BinOp::Sub => Dynamic::from_ast(&l_bv.bvsub(&r_bv)),
                        BinOp::Mul => Dynamic::from_ast(&l_bv.bvmul(&r_bv)),
                        _ => self.model.fresh_value("binary_val"),
                    }
                } else {
                    self.model.fresh_value("binary_val")
                }
            }

            _ => self.model.fresh_value("expr_val"),
        }
    }

    /// Verify heap entailment: P |- Q
    ///
    /// Returns Ok(true) if P entails Q, Ok(false) if not, Err on error
    pub fn verify_entailment(
        &self,
        antecedent: &SepAssertion,
        consequent: &SepAssertion,
    ) -> Result<EntailmentResult, SepLogicError> {
        let start = Instant::now();

        self.solver.reset();

        let heap = self.model.fresh_heap("ent_heap");
        let alloc = self.model.fresh_allocated("ent_alloc");

        // Encode antecedent
        let p_encoded = self.encode_assertion(antecedent, &heap, &alloc);
        self.solver.assert(&p_encoded);

        // Check if NOT(consequent) is satisfiable
        let q_encoded = self.encode_assertion(consequent, &heap, &alloc);
        self.solver.assert(q_encoded.not());

        let result = match self.solver.check() {
            SatResult::Unsat => {
                // NOT(Q) is UNSAT given P, so P |- Q
                EntailmentResult::Valid {
                    proof_time: start.elapsed(),
                }
            }
            SatResult::Sat => {
                // Found counterexample
                let model = self.solver.get_model();
                EntailmentResult::Invalid {
                    counterexample: self.extract_counterexample(model),
                    proof_time: start.elapsed(),
                }
            }
            SatResult::Unknown => {
                let reason: Text = self
                    .solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "unknown".to_string())
                    .into();
                EntailmentResult::Unknown {
                    reason,
                    elapsed: start.elapsed(),
                }
            }
        };

        Ok(result)
    }

    /// Extract counterexample from Z3 model
    ///
    /// This extracts heap and allocation information from the Z3 model by
    /// dynamically discovering all allocated addresses. The extraction process:
    /// 1. Iterate over all function declarations in the model
    /// 2. For array-based heap/allocation declarations, extract the function interpretation
    /// 3. For each address in the interpretation, check allocation and extract heap values
    /// 4. Additionally check all address-like constants referenced in the model
    fn extract_counterexample(&self, model: Option<Model>) -> SepCounterexample {
        let mut heap_contents = Map::new();
        let mut allocations = Set::new();
        let mut description_parts = vec!["Entailment failed - counterexample found".to_string()];

        if let Some(m) = model {
            // Collect all addresses we need to check from various sources
            let mut addresses_to_check: Vec<Dynamic> = Vec::new();

            // 1. Extract addresses from all function declarations in the model
            for func_decl in m.iter() {
                let name = func_decl.name().to_string();

                // Check for address variables (created by fresh_addr with various prefixes)
                if name.starts_with("addr_")
                    || name.starts_with("loc_")
                    || name.starts_with("disjoint_addr")
                    || name.starts_with("compose_addr")
                    || name.contains("_addr_")
                {
                    // Evaluate this address constant using the model
                    // FuncDecl::apply returns Dynamic for zero-arity functions
                    if func_decl.arity() == 0 {
                        let addr_dyn = func_decl.apply(&[]);
                        if let Some(evaluated) = m.eval(&addr_dyn, true) {
                            addresses_to_check.push(evaluated);
                        }
                    }
                }

                // Check for allocation array interpretations
                // The allocation array maps addresses to booleans
                if name.contains("alloc") || name.starts_with("a1_") || name.starts_with("a2_") {
                    // Try to extract the function interpretation for array stores
                    if let Some(interp) = m.get_func_interp(&func_decl) {
                        // Get all entries in the function interpretation
                        for entry in interp.get_entries() {
                            // Each entry has arguments (the address) and a value (allocated or not)
                            let args = entry.get_args();
                            if !args.is_empty() {
                                addresses_to_check.push(args[0].clone());
                            }
                        }
                    }
                }

                // Check for heap array interpretations to find addresses
                if name.contains("heap") || name.starts_with("h1_") || name.starts_with("h2_") {
                    if let Some(interp) = m.get_func_interp(&func_decl) {
                        for entry in interp.get_entries() {
                            let args = entry.get_args();
                            if !args.is_empty() {
                                addresses_to_check.push(args[0].clone());
                            }
                        }
                    }
                }
            }

            // 2. For each discovered address, check if it's allocated and extract heap value
            for addr_dyn in &addresses_to_check {
                let alloc_check = self.model.allocated().select(addr_dyn);

                if let Some(alloc_val) = m.eval(&alloc_check, true) {
                    if let Some(bool_val) = alloc_val.as_bool() {
                        if bool_val.as_bool() == Some(true) {
                            // Address is allocated, extract the heap value
                            let heap_val = self.model.heap().select(addr_dyn);
                            if let Some(val) = m.eval(&heap_val, true) {
                                // Try to extract the address as i64
                                if let Some(bv) = addr_dyn.as_bv() {
                                    if let Some(addr_i64) = bv.as_i64() {
                                        if !allocations.contains(&addr_i64) {
                                            allocations.insert(addr_i64);
                                            // Try to extract value as i64
                                            if let Some(val_bv) = val.as_bv() {
                                                if let Some(val_i64) = val_bv.as_i64() {
                                                    heap_contents.insert(addr_i64, val_i64);
                                                    description_parts.push(format!(
                                                        "  heap[0x{:x}] = 0x{:x} ({})",
                                                        addr_i64, val_i64, val_i64
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 3. Check constants in the model for additional heap information
            // This catches cases where addresses are directly referenced as constants
            for const_decl in m.iter() {
                if const_decl.arity() == 0 {
                    let name = const_decl.name().to_string();
                    // Skip non-address constants and already processed addresses
                    if !name.contains("heap") && !name.contains("alloc") && !name.starts_with("__")
                    {
                        continue;
                    }

                    // Try to evaluate and check if it's a BV64 that could be an address
                    let const_val = const_decl.apply(&[]);
                    if let Some(evaluated) = m.eval(&const_val, true) {
                        if let Some(bv) = evaluated.as_bv() {
                            if bv.get_size() == 64 {
                                if let Some(addr_i64) = bv.as_i64() {
                                    // Check if this address is allocated but not yet recorded
                                    if !allocations.contains(&addr_i64) {
                                        let addr = self.model.addr_const(addr_i64);
                                        let alloc_check = self.model.allocated().select(&addr);
                                        if let Some(alloc_val) = m.eval(&alloc_check, true) {
                                            if let Some(bool_val) = alloc_val.as_bool() {
                                                if bool_val.as_bool() == Some(true) {
                                                    allocations.insert(addr_i64);
                                                    let heap_val = self.model.heap().select(&addr);
                                                    if let Some(val) = m.eval(&heap_val, true) {
                                                        if let Some(val_bv) = val.as_bv() {
                                                            if let Some(val_i64) = val_bv.as_i64() {
                                                                heap_contents
                                                                    .insert(addr_i64, val_i64);
                                                                description_parts.push(format!(
                                                                    "  heap[0x{:x}] = 0x{:x} ({})",
                                                                    addr_i64, val_i64, val_i64
                                                                ));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Add summary information
            if allocations.is_empty() {
                description_parts
                    .push("  (no heap allocations found in counterexample)".to_string());
            } else {
                description_parts.insert(
                    1,
                    format!("  Total allocated addresses: {}", allocations.len()),
                );
            }
        } else {
            description_parts.push("  (no model available)".to_string());
        }

        SepCounterexample {
            heap_contents,
            allocations,
            description: Text::from(description_parts.join("\n")),
        }
    }

    /// Verify frame rule application
    ///
    /// Given {P} c {Q}, verify that {P * R} c {Q * R} holds
    pub fn verify_frame_rule(
        &self,
        pre: &SepAssertion,
        post: &SepAssertion,
        frame: &SepAssertion,
    ) -> Result<bool, SepLogicError> {
        // Frame rule is sound if:
        // 1. Frame is pure or its footprint is disjoint from modified locations
        // 2. The command doesn't modify frame's footprint

        // For separation logic, frame rule is always sound by construction
        // We verify the disjointness condition

        let pre_footprint = pre.footprint();
        let frame_footprint = frame.footprint();

        // Check syntactic disjointness
        let disjoint = self.are_footprints_disjoint(&pre_footprint, &frame_footprint);

        Ok(disjoint)
    }

    /// Check if two footprints are disjoint using SMT-based verification
    ///
    /// This method uses Z3 to check if there exists any possible assignment of
    /// variables that could make two addresses from the footprints equal. If such
    /// an assignment exists (SAT), the footprints may overlap and are not provably
    /// disjoint. If no such assignment exists (UNSAT for all pairs), the footprints
    /// are provably disjoint.
    ///
    /// This properly handles:
    /// - Address aliasing (e.g., `x` and `y` could be equal)
    /// - Arithmetic expressions (e.g., `x + 1` and `y` could be equal if `y = x + 1`)
    /// - Symbolic addresses with constraints from the context
    fn are_footprints_disjoint(&self, fp1: &[Expr], fp2: &[Expr]) -> bool {
        // Empty footprints are trivially disjoint
        if fp1.is_empty() || fp2.is_empty() {
            return true;
        }

        // Create a fresh solver for disjointness checking
        let disjoint_solver = Solver::new();

        // Set a short timeout for disjointness checks (faster than full entailment)
        let mut params = Params::new();
        params.set_u32(
            "timeout",
            (self.config.entailment_timeout_ms / 4).max(500) as u32,
        );
        disjoint_solver.set_params(&params);

        // For each pair of addresses, check if they could be equal
        for e1 in fp1 {
            for e2 in fp2 {
                // Encode both expressions as addresses
                let addr1 = self.encode_expr_as_addr(e1);
                let addr2 = self.encode_expr_as_addr(e2);

                // Create equality constraint: addr1 == addr2
                // If this is SAT, the addresses could be equal (potential overlap)
                let eq_constraint = addr1.eq(&addr2);

                // Use push/pop for efficiency when checking multiple pairs
                disjoint_solver.push();
                disjoint_solver.assert(&eq_constraint);

                match disjoint_solver.check() {
                    SatResult::Sat => {
                        // Found a satisfying assignment where addr1 == addr2
                        // This means the footprints could overlap - not disjoint
                        return false;
                    }
                    SatResult::Unsat => {
                        // This pair is provably different, check next pair
                        disjoint_solver.pop(1);
                    }
                    SatResult::Unknown => {
                        // Conservative: if we can't prove disjointness, assume not disjoint
                        // This is sound but may reject valid programs
                        return false;
                    }
                }
            }
        }

        // All pairs are provably different
        true
    }
}

// ==================== Entailment Result ====================

/// Result of entailment checking
#[derive(Debug)]
pub enum EntailmentResult {
    /// Entailment is valid
    Valid { proof_time: Duration },
    /// Entailment is invalid with counterexample
    Invalid {
        counterexample: SepCounterexample,
        proof_time: Duration,
    },
    /// Could not determine (timeout or unknown)
    Unknown { reason: Text, elapsed: Duration },
}

impl EntailmentResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid { .. })
    }
}

/// Counterexample for failed entailment
#[derive(Debug, Clone)]
pub struct SepCounterexample {
    /// Heap contents at counterexample
    pub heap_contents: Map<i64, i64>,
    /// Allocated addresses
    pub allocations: Set<i64>,
    /// Human-readable description
    pub description: Text,
}

/// Error type for separation logic operations
#[derive(Debug)]
pub enum SepLogicError {
    /// Encoding error
    EncodingError(Text),
    /// Solver error
    SolverError(Text),
    /// Timeout
    Timeout { elapsed: Duration },
    /// Invalid assertion structure
    InvalidAssertion(Text),
}

impl std::fmt::Display for SepLogicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EncodingError(msg) => write!(f, "Encoding error: {}", msg),
            Self::SolverError(msg) => write!(f, "Solver error: {}", msg),
            Self::Timeout { elapsed } => write!(f, "Timeout after {:?}", elapsed),
            Self::InvalidAssertion(msg) => write!(f, "Invalid assertion: {}", msg),
        }
    }
}

impl std::error::Error for SepLogicError {}

// ==================== Separation Logic Verifier ====================

/// Separation logic verifier with Z3 backend
pub struct SeparationLogic {
    /// Proof search engine for fallback
    engine: ProofSearchEngine,
    /// Fresh variable counter
    var_counter: std::cell::Cell<usize>,
    /// Configuration
    config: SepLogicConfig,
    /// Statistics
    stats: RefCell<SepLogicStats>,
}

/// Statistics for separation logic verification
#[derive(Debug, Clone, Default)]
pub struct SepLogicStats {
    /// Number of entailment checks
    pub entailment_checks: usize,
    /// Number of successful proofs
    pub successful_proofs: usize,
    /// Number of failed proofs
    pub failed_proofs: usize,
    /// Number of timeouts
    pub timeouts: usize,
    /// Total verification time
    pub total_time: Duration,
}

impl SeparationLogic {
    /// Create a new separation logic verifier
    pub fn new() -> Self {
        Self {
            engine: ProofSearchEngine::new(),
            var_counter: std::cell::Cell::new(0),
            config: SepLogicConfig::default(),
            stats: RefCell::new(SepLogicStats::default()),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: SepLogicConfig) -> Self {
        Self {
            engine: ProofSearchEngine::new(),
            var_counter: std::cell::Cell::new(0),
            config,
            stats: RefCell::new(SepLogicStats::default()),
        }
    }

    /// Reset the fresh variable counter
    pub fn reset_counter(&self) {
        self.var_counter.set(0);
    }

    /// Get statistics
    pub fn stats(&self) -> SepLogicStats {
        self.stats.borrow().clone()
    }

    /// Verify heap entailment using Z3
    pub fn verify_entailment(
        &self,
        antecedent: &SepAssertion,
        consequent: &SepAssertion,
    ) -> Result<EntailmentResult, SepLogicError> {
        let start = Instant::now();

        // Create Z3 encoder (uses thread-local context)
        let encoder = SepLogicEncoder::new(self.config.clone());

        let result = encoder.verify_entailment(antecedent, consequent)?;

        // Update statistics
        let mut stats = self.stats.borrow_mut();
        stats.entailment_checks += 1;
        stats.total_time += start.elapsed();

        match &result {
            EntailmentResult::Valid { .. } => stats.successful_proofs += 1,
            EntailmentResult::Invalid { .. } => stats.failed_proofs += 1,
            EntailmentResult::Unknown { .. } => stats.timeouts += 1,
        }

        Ok(result)
    }

    /// Verify Hoare triple: {P} c {Q}
    ///
    /// Verify Hoare triple {P} c {Q} by computing wp(c, Q) and checking P => wp(c, Q).
    /// Uses separation logic rules: frame rule for heap disjointness, weakest precondition
    /// calculus for commands, and Z3 for implication checking.
    pub fn verify_triple(
        &mut self,
        context: &VerumContext,
        triple: &HoareTriple,
    ) -> Result<ProofTerm, ProofError> {
        // Compute weakest precondition wp(c, Q)
        let wp = self.wp(&triple.command, &triple.post)?;

        // Verify P implies wp(c, Q)
        self.verify_implication(context, &triple.pre, &wp)
    }

    /// Compute weakest precondition
    ///
    /// Compute weakest precondition using Hoare logic rules:
    /// - wp(skip, Q) = Q
    /// - wp(x := e, Q) = Q[e/x] (substitution)
    /// - wp(c1; c2, Q) = wp(c1, wp(c2, Q)) (sequential composition)
    /// - wp(if b then c1 else c2, Q) = (b => wp(c1,Q)) /\ (!b => wp(c2,Q))
    /// - wp(while b inv I do c, Q) = I (loop invariant)
    pub fn wp(&self, command: &Command, post: &SepAssertion) -> Result<SepAssertion, ProofError> {
        match command {
            Command::Skip => {
                // wp(skip, Q) = Q
                Ok(post.clone())
            }

            Command::Assign { var, expr } => {
                // wp(x := e, Q) = Q[e/x]
                Ok(self.substitute(post, var, expr))
            }

            Command::Seq { first, second } => {
                // wp(c1; c2, Q) = wp(c1, wp(c2, Q))
                let wp2 = self.wp(second, post)?;
                self.wp(first, &wp2)
            }

            Command::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // wp(if b then c1 else c2, Q) = (b => wp(c1, Q)) /\ (!b => wp(c2, Q))
                let wp_then = self.wp(then_branch, post)?;
                let wp_else = self.wp(else_branch, post)?;

                let b_implies_then = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: Heap::new(condition.clone()),
                        right: Heap::new(self.assertion_to_expr(&wp_then)),
                    },
                    verum_ast::span::Span::dummy(),
                );

                let not_b = Expr::new(
                    ExprKind::Unary {
                        op: verum_ast::UnOp::Not,
                        expr: Heap::new(condition.clone()),
                    },
                    verum_ast::span::Span::dummy(),
                );

                let not_b_implies_else = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: Heap::new(not_b),
                        right: Heap::new(self.assertion_to_expr(&wp_else)),
                    },
                    verum_ast::span::Span::dummy(),
                );

                let conjunction = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Heap::new(b_implies_then),
                        right: Heap::new(not_b_implies_else),
                    },
                    verum_ast::span::Span::dummy(),
                );

                Ok(SepAssertion::Pure(conjunction))
            }

            Command::While { invariant, .. } => {
                // wp(while b inv I do c, Q) = I
                Ok(invariant.clone())
            }

            Command::Alloc { result, size: _ } => {
                // wp(result := alloc(size), Q) = forall ptr. (ptr |-> _) -* Q[result := ptr]
                let ptr_var = format!("__alloc_ptr_{}", self.fresh_var_counter());

                let ptr_expr = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        ptr_var.clone(),
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let wildcard_value = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        "_",
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let allocated_region = SepAssertion::PointsTo {
                    location: ptr_expr.clone(),
                    value: wildcard_value,
                };

                let subst_post = self.substitute(post, result, &ptr_expr);

                let wand = SepAssertion::Wand {
                    left: Heap::new(allocated_region),
                    right: Heap::new(subst_post),
                };

                Ok(SepAssertion::Forall {
                    var: ptr_var.into(),
                    body: Heap::new(wand),
                })
            }

            Command::Read { result, addr } => {
                // wp(result := *addr, Q) = exists v. (addr |-> v) * ((addr |-> v) -* Q[result := v])
                let value_var = format!("__read_val_{}", self.fresh_var_counter());

                let value_expr = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        value_var.clone(),
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let points_to = SepAssertion::PointsTo {
                    location: addr.clone(),
                    value: value_expr.clone(),
                };

                let subst_post = self.substitute(post, result, &value_expr);

                let wand = SepAssertion::Wand {
                    left: Heap::new(points_to.clone()),
                    right: Heap::new(subst_post),
                };

                let sep = SepAssertion::Sep {
                    left: Heap::new(points_to),
                    right: Heap::new(wand),
                };

                Ok(SepAssertion::Exists {
                    var: value_var.into(),
                    body: Heap::new(sep),
                })
            }

            Command::Write { addr, value } => {
                // wp(*addr := value, Q) = (addr |-> _) * ((addr |-> value) -* Q)
                let old_value = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        "_",
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let old_points_to = SepAssertion::PointsTo {
                    location: addr.clone(),
                    value: old_value,
                };

                let new_points_to = SepAssertion::PointsTo {
                    location: addr.clone(),
                    value: value.clone(),
                };

                let wand = SepAssertion::Wand {
                    left: Heap::new(new_points_to),
                    right: Heap::new(post.clone()),
                };

                Ok(SepAssertion::Sep {
                    left: Heap::new(old_points_to),
                    right: Heap::new(wand),
                })
            }

            Command::Free { ptr, size: _ } => {
                // wp(free(ptr, size), Q) = (ptr |-> _) * (emp -* Q)
                let wildcard = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        "_",
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let freed_region = SepAssertion::PointsTo {
                    location: ptr.clone(),
                    value: wildcard,
                };

                let wand = SepAssertion::Wand {
                    left: Heap::new(SepAssertion::Emp),
                    right: Heap::new(post.clone()),
                };

                Ok(SepAssertion::Sep {
                    left: Heap::new(freed_region),
                    right: Heap::new(wand),
                })
            }

            Command::CAS {
                result,
                addr,
                expected,
                desired,
            } => {
                // wp(CAS(result, addr, expected, desired), Q) =
                //   exists v. (addr |-> v) * (
                //     (v == expected => (addr |-> desired) -* Q[result := true]) /\
                //     (v != expected => (addr |-> v) -* Q[result := false])
                //   )
                let value_var = format!("__cas_val_{}", self.fresh_var_counter());
                let value_expr = Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new(
                        value_var.clone(),
                        verum_ast::span::Span::dummy(),
                    ))),
                    verum_ast::span::Span::dummy(),
                );

                let points_to_old = SepAssertion::PointsTo {
                    location: addr.clone(),
                    value: value_expr.clone(),
                };

                // Success case: v == expected
                let true_lit = Expr::new(
                    ExprKind::Literal(Literal::new(
                        LiteralKind::Bool(true),
                        verum_ast::span::Span::dummy(),
                    )),
                    verum_ast::span::Span::dummy(),
                );
                let post_success = self.substitute(post, result, &true_lit);

                let points_to_new = SepAssertion::PointsTo {
                    location: addr.clone(),
                    value: desired.clone(),
                };

                let success_wand = SepAssertion::Wand {
                    left: Heap::new(points_to_new),
                    right: Heap::new(post_success),
                };

                // Failure case: v != expected
                let false_lit = Expr::new(
                    ExprKind::Literal(Literal::new(
                        LiteralKind::Bool(false),
                        verum_ast::span::Span::dummy(),
                    )),
                    verum_ast::span::Span::dummy(),
                );
                let post_failure = self.substitute(post, result, &false_lit);

                let failure_wand = SepAssertion::Wand {
                    left: Heap::new(points_to_old.clone()),
                    right: Heap::new(post_failure),
                };

                // Condition: v == expected
                let eq_cond = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Eq,
                        left: Heap::new(value_expr.clone()),
                        right: Heap::new(expected.clone()),
                    },
                    verum_ast::span::Span::dummy(),
                );

                let ne_cond = Expr::new(
                    ExprKind::Unary {
                        op: verum_ast::UnOp::Not,
                        expr: Heap::new(eq_cond.clone()),
                    },
                    verum_ast::span::Span::dummy(),
                );

                // Build the conditional
                let success_impl = SepAssertion::And {
                    left: Heap::new(SepAssertion::Pure(eq_cond)),
                    right: Heap::new(success_wand),
                };

                let failure_impl = SepAssertion::And {
                    left: Heap::new(SepAssertion::Pure(ne_cond)),
                    right: Heap::new(failure_wand),
                };

                let conditional = SepAssertion::Or {
                    left: Heap::new(success_impl),
                    right: Heap::new(failure_impl),
                };

                let sep = SepAssertion::Sep {
                    left: Heap::new(points_to_old),
                    right: Heap::new(conditional),
                };

                Ok(SepAssertion::Exists {
                    var: value_var.into(),
                    body: Heap::new(sep),
                })
            }

            Command::Call {
                result,
                pre: call_pre,
                post: call_post,
                ..
            } => {
                // wp(call, Q) = call_pre * (call_post -* Q)
                let subst_post = if let Maybe::Some(res_var) = result {
                    // Substitute result variable in postcondition
                    let res_expr = Expr::new(
                        ExprKind::Path(Path::from_ident(Ident::new(
                            res_var.clone(),
                            verum_ast::span::Span::dummy(),
                        ))),
                        verum_ast::span::Span::dummy(),
                    );
                    self.substitute(post, res_var, &res_expr)
                } else {
                    post.clone()
                };

                let wand = SepAssertion::Wand {
                    left: Heap::new(call_post.clone()),
                    right: Heap::new(subst_post),
                };

                Ok(SepAssertion::Sep {
                    left: Heap::new(call_pre.clone()),
                    right: Heap::new(wand),
                })
            }
        }
    }

    /// Generate fresh variable counter
    fn fresh_var_counter(&self) -> usize {
        let current = self.var_counter.get();
        self.var_counter.set(current + 1);
        current
    }

    /// Generate a fresh variable name with a given prefix
    pub fn fresh_var(&self, prefix: &str) -> Text {
        Text::from(format!("{}_{}", prefix, self.fresh_var_counter()))
    }

    /// Apply frame rule
    ///
    /// Frame rule: {P} c {Q} => {P * R} c {Q * R}
    pub fn apply_frame_rule(&self, triple: HoareTriple, frame: SepAssertion) -> HoareTriple {
        HoareTriple {
            pre: SepAssertion::Sep {
                left: Heap::new(triple.pre),
                right: Heap::new(frame.clone()),
            },
            command: triple.command,
            post: SepAssertion::Sep {
                left: Heap::new(triple.post),
                right: Heap::new(frame),
            },
        }
    }

    /// Verify implication between assertions using Z3
    fn verify_implication(
        &mut self,
        context: &VerumContext,
        pre: &SepAssertion,
        post: &SepAssertion,
    ) -> Result<ProofTerm, ProofError> {
        // Try Z3-based entailment first
        match self.verify_entailment(pre, post) {
            Ok(EntailmentResult::Valid { .. }) => {
                // Create proof term for successful entailment
                Ok(ProofTerm::Axiom("separation_logic_entailment".into()))
            }
            Ok(EntailmentResult::Invalid { counterexample, .. }) => Err(ProofError::TacticFailed(
                format!("Entailment failed: {}", counterexample.description).into(),
            )),
            Ok(EntailmentResult::Unknown { reason, .. }) => {
                // Fall back to proof search engine
                let pre_expr = self.assertion_to_expr(pre);
                let post_expr = self.assertion_to_expr(post);

                let implication = Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Imply,
                        left: Heap::new(pre_expr),
                        right: Heap::new(post_expr),
                    },
                    verum_ast::span::Span::dummy(),
                );

                let goal = ProofGoal::new(implication);
                match self.engine.try_smt_discharge(context, &goal)? {
                    Maybe::Some(proof) => Ok(proof),
                    Maybe::None => Err(ProofError::TacticFailed(
                        format!("Failed to verify implication: {}", reason).into(),
                    )),
                }
            }
            Err(e) => Err(ProofError::TacticFailed(format!("{}", e).into())),
        }
    }

    /// Convert separation logic assertion to expression
    fn assertion_to_expr(&self, assertion: &SepAssertion) -> Expr {
        use verum_ast::span::Span;

        match assertion {
            SepAssertion::Pure(expr) => expr.clone(),

            SepAssertion::Emp => Expr::new(
                ExprKind::Call {
                    func: Heap::new(Expr::new(
                        ExprKind::Path(Path::from_ident(Ident::new("emp", Span::dummy()))),
                        Span::dummy(),
                    )),
                    type_args: List::new(),
                    args: List::new(),
                },
                Span::dummy(),
            ),

            SepAssertion::PointsTo { location, value } => Expr::new(
                ExprKind::Call {
                    func: Heap::new(Expr::new(
                        ExprKind::Path(Path::from_ident(Ident::new("points_to", Span::dummy()))),
                        Span::dummy(),
                    )),
                    type_args: List::new(),
                    args: List::from_iter(vec![location.clone(), value.clone()]),
                },
                Span::dummy(),
            ),

            SepAssertion::Sep { left, right } => {
                let left_expr = self.assertion_to_expr(left);
                let right_expr = self.assertion_to_expr(right);
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("sep", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![left_expr, right_expr]),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Or { left, right } => {
                let left_expr = self.assertion_to_expr(left);
                let right_expr = self.assertion_to_expr(right);
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::Or,
                        left: Heap::new(left_expr),
                        right: Heap::new(right_expr),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::And { left, right } => {
                let left_expr = self.assertion_to_expr(left);
                let right_expr = self.assertion_to_expr(right);
                Expr::new(
                    ExprKind::Binary {
                        op: BinOp::And,
                        left: Heap::new(left_expr),
                        right: Heap::new(right_expr),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Exists { var, body } => {
                let body_expr = self.assertion_to_expr(body);
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("exists", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![
                            Expr::new(
                                ExprKind::Path(Path::from_ident(Ident::new(
                                    var.clone(),
                                    Span::dummy(),
                                ))),
                                Span::dummy(),
                            ),
                            body_expr,
                        ]),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Forall { var, body } => {
                let body_expr = self.assertion_to_expr(body);
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("forall", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![
                            Expr::new(
                                ExprKind::Path(Path::from_ident(Ident::new(
                                    var.clone(),
                                    Span::dummy(),
                                ))),
                                Span::dummy(),
                            ),
                            body_expr,
                        ]),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Wand { left, right } => {
                let left_expr = self.assertion_to_expr(left);
                let right_expr = self.assertion_to_expr(right);
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("wand", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: List::from_iter(vec![left_expr, right_expr]),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::ListSegment { from, to, elements } => {
                let mut args = vec![from.clone(), to.clone()];
                args.extend(elements.iter().cloned());
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("lseg", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: args.into(),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => {
                let mut args = vec![root.clone()];
                if let Maybe::Some(left) = left_child {
                    args.push(self.assertion_to_expr(left));
                }
                if let Maybe::Some(right) = right_child {
                    args.push(self.assertion_to_expr(right));
                }
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new("tree", Span::dummy()))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: args.into(),
                    },
                    Span::dummy(),
                )
            }

            SepAssertion::Block { base, size } => Expr::new(
                ExprKind::Call {
                    func: Heap::new(Expr::new(
                        ExprKind::Path(Path::from_ident(Ident::new("block", Span::dummy()))),
                        Span::dummy(),
                    )),
                    type_args: List::new(),
                    args: List::from_iter(vec![base.clone(), size.clone()]),
                },
                Span::dummy(),
            ),

            SepAssertion::ArraySegment {
                base,
                offset,
                length,
                elements,
            } => {
                let mut args = vec![base.clone(), offset.clone(), length.clone()];
                args.extend(elements.iter().cloned());
                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(Expr::new(
                            ExprKind::Path(Path::from_ident(Ident::new(
                                "array_seg",
                                Span::dummy(),
                            ))),
                            Span::dummy(),
                        )),
                        type_args: List::new(),
                        args: args.into(),
                    },
                    Span::dummy(),
                )
            }
        }
    }

    /// Substitute expression for variable in assertion
    fn substitute(&self, assertion: &SepAssertion, var: &Text, replacement: &Expr) -> SepAssertion {
        match assertion {
            SepAssertion::Pure(expr) => {
                SepAssertion::Pure(self.substitute_expr(expr, var, replacement))
            }

            SepAssertion::Emp => SepAssertion::Emp,

            SepAssertion::PointsTo { location, value } => SepAssertion::PointsTo {
                location: self.substitute_expr(location, var, replacement),
                value: self.substitute_expr(value, var, replacement),
            },

            SepAssertion::Sep { left, right } => SepAssertion::Sep {
                left: Heap::new(self.substitute(left, var, replacement)),
                right: Heap::new(self.substitute(right, var, replacement)),
            },

            SepAssertion::Or { left, right } => SepAssertion::Or {
                left: Heap::new(self.substitute(left, var, replacement)),
                right: Heap::new(self.substitute(right, var, replacement)),
            },

            SepAssertion::And { left, right } => SepAssertion::And {
                left: Heap::new(self.substitute(left, var, replacement)),
                right: Heap::new(self.substitute(right, var, replacement)),
            },

            SepAssertion::Wand { left, right } => SepAssertion::Wand {
                left: Heap::new(self.substitute(left, var, replacement)),
                right: Heap::new(self.substitute(right, var, replacement)),
            },

            SepAssertion::Exists {
                var: bound_var,
                body,
            } => {
                if bound_var == var {
                    assertion.clone()
                } else {
                    SepAssertion::Exists {
                        var: bound_var.clone(),
                        body: Heap::new(self.substitute(body, var, replacement)),
                    }
                }
            }

            SepAssertion::Forall {
                var: bound_var,
                body,
            } => {
                if bound_var == var {
                    assertion.clone()
                } else {
                    SepAssertion::Forall {
                        var: bound_var.clone(),
                        body: Heap::new(self.substitute(body, var, replacement)),
                    }
                }
            }

            SepAssertion::ListSegment { from, to, elements } => SepAssertion::ListSegment {
                from: self.substitute_expr(from, var, replacement),
                to: self.substitute_expr(to, var, replacement),
                elements: elements
                    .iter()
                    .map(|e| self.substitute_expr(e, var, replacement))
                    .collect(),
            },

            SepAssertion::Tree {
                root,
                left_child,
                right_child,
            } => SepAssertion::Tree {
                root: self.substitute_expr(root, var, replacement),
                left_child: left_child
                    .as_ref()
                    .map(|c| Heap::new(self.substitute(c, var, replacement))),
                right_child: right_child
                    .as_ref()
                    .map(|c| Heap::new(self.substitute(c, var, replacement))),
            },

            SepAssertion::Block { base, size } => SepAssertion::Block {
                base: self.substitute_expr(base, var, replacement),
                size: self.substitute_expr(size, var, replacement),
            },

            SepAssertion::ArraySegment {
                base,
                offset,
                length,
                elements,
            } => SepAssertion::ArraySegment {
                base: self.substitute_expr(base, var, replacement),
                offset: self.substitute_expr(offset, var, replacement),
                length: self.substitute_expr(length, var, replacement),
                elements: elements
                    .iter()
                    .map(|e| self.substitute_expr(e, var, replacement))
                    .collect(),
            },
        }
    }

    /// Substitute expression for variable in expression
    fn substitute_expr(&self, expr: &Expr, var: &Text, replacement: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if ident.name.as_str() == var.as_str() {
                        return replacement.clone();
                    }
                }
                expr.clone()
            }

            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Heap::new(self.substitute_expr(left, var, replacement)),
                    right: Heap::new(self.substitute_expr(right, var, replacement)),
                },
                expr.span,
            ),

            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: Heap::new(self.substitute_expr(inner, var, replacement)),
                },
                expr.span,
            ),

            ExprKind::Call { func, args, .. } => {
                let new_func = self.substitute_expr(func, var, replacement);
                let new_args = args
                    .iter()
                    .map(|arg| self.substitute_expr(arg, var, replacement))
                    .collect();

                Expr::new(
                    ExprKind::Call {
                        func: Heap::new(new_func),
                        type_args: List::new(),
                        args: new_args,
                    },
                    expr.span,
                )
            }

            ExprKind::Index {
                expr: base_expr,
                index,
            } => Expr::new(
                ExprKind::Index {
                    expr: Heap::new(self.substitute_expr(base_expr, var, replacement)),
                    index: Heap::new(self.substitute_expr(index, var, replacement)),
                },
                expr.span,
            ),

            ExprKind::Field {
                expr: base_expr,
                field,
            } => Expr::new(
                ExprKind::Field {
                    expr: Heap::new(self.substitute_expr(base_expr, var, replacement)),
                    field: field.clone(),
                },
                expr.span,
            ),

            ExprKind::Attenuate {
                context,
                capabilities,
            } => Expr::new(
                ExprKind::Attenuate {
                    context: Heap::new(self.substitute_expr(context, var, replacement)),
                    capabilities: capabilities.clone(),
                },
                expr.span,
            ),

            _ => expr.clone(),
        }
    }

    /// Get the underlying proof search engine
    pub fn engine(&self) -> &ProofSearchEngine {
        &self.engine
    }

    /// Get mutable access to proof search engine
    pub fn engine_mut(&mut self) -> &mut ProofSearchEngine {
        &mut self.engine
    }

    /// Simplify separation logic assertion
    pub fn simplify(&self, assertion: &SepAssertion) -> SepAssertion {
        match assertion {
            SepAssertion::Sep { left, right } => {
                let left_simp = self.simplify(left);
                let right_simp = self.simplify(right);

                // emp * P = P
                if left_simp.is_emp() {
                    return right_simp;
                }
                // P * emp = P
                if right_simp.is_emp() {
                    return left_simp;
                }

                SepAssertion::Sep {
                    left: Heap::new(left_simp),
                    right: Heap::new(right_simp),
                }
            }

            SepAssertion::Wand { left, right } => {
                let left_simp = self.simplify(left);
                let right_simp = self.simplify(right);

                // emp -* P = P
                if left_simp.is_emp() {
                    return right_simp;
                }

                SepAssertion::Wand {
                    left: Heap::new(left_simp),
                    right: Heap::new(right_simp),
                }
            }

            SepAssertion::And { left, right } => {
                let left_simp = self.simplify(left);
                let right_simp = self.simplify(right);

                SepAssertion::And {
                    left: Heap::new(left_simp),
                    right: Heap::new(right_simp),
                }
            }

            SepAssertion::Or { left, right } => {
                let left_simp = self.simplify(left);
                let right_simp = self.simplify(right);

                SepAssertion::Or {
                    left: Heap::new(left_simp),
                    right: Heap::new(right_simp),
                }
            }

            SepAssertion::Exists { var, body } => SepAssertion::Exists {
                var: var.clone(),
                body: Heap::new(self.simplify(body)),
            },

            SepAssertion::Forall { var, body } => SepAssertion::Forall {
                var: var.clone(),
                body: Heap::new(self.simplify(body)),
            },

            _ => assertion.clone(),
        }
    }

    /// Check if two assertions are syntactically equal
    pub fn assertions_equal(&self, left: &SepAssertion, right: &SepAssertion) -> bool {
        match (left, right) {
            (SepAssertion::Emp, SepAssertion::Emp) => true,
            (SepAssertion::Pure(e1), SepAssertion::Pure(e2)) => {
                format!("{:?}", e1) == format!("{:?}", e2)
            }
            (
                SepAssertion::PointsTo {
                    location: l1,
                    value: v1,
                },
                SepAssertion::PointsTo {
                    location: l2,
                    value: v2,
                },
            ) => {
                format!("{:?}", l1) == format!("{:?}", l2)
                    && format!("{:?}", v1) == format!("{:?}", v2)
            }
            (
                SepAssertion::Sep {
                    left: l1,
                    right: r1,
                },
                SepAssertion::Sep {
                    left: l2,
                    right: r2,
                },
            ) => self.assertions_equal(l1, l2) && self.assertions_equal(r1, r2),
            (
                SepAssertion::Wand {
                    left: l1,
                    right: r1,
                },
                SepAssertion::Wand {
                    left: l2,
                    right: r2,
                },
            ) => self.assertions_equal(l1, l2) && self.assertions_equal(r1, r2),
            (
                SepAssertion::And {
                    left: l1,
                    right: r1,
                },
                SepAssertion::And {
                    left: l2,
                    right: r2,
                },
            ) => self.assertions_equal(l1, l2) && self.assertions_equal(r1, r2),
            (
                SepAssertion::Or {
                    left: l1,
                    right: r1,
                },
                SepAssertion::Or {
                    left: l2,
                    right: r2,
                },
            ) => self.assertions_equal(l1, l2) && self.assertions_equal(r1, r2),
            (
                SepAssertion::Exists { var: v1, body: b1 },
                SepAssertion::Exists { var: v2, body: b2 },
            ) => v1 == v2 && self.assertions_equal(b1, b2),
            (
                SepAssertion::Forall { var: v1, body: b1 },
                SepAssertion::Forall { var: v2, body: b2 },
            ) => v1 == v2 && self.assertions_equal(b1, b2),
            _ => false,
        }
    }

    /// Extract heap locations referenced in an assertion
    pub fn extract_locations(&self, assertion: &SepAssertion) -> List<Expr> {
        assertion.footprint()
    }

    /// Apply magic wand elimination
    ///
    /// If we have P * (P -* Q), we can derive Q
    pub fn apply_wand_elimination(
        &self,
        have: &SepAssertion,
        wand_left: &SepAssertion,
        wand_right: &SepAssertion,
    ) -> Result<SepAssertion, ProofError> {
        if self.assertions_equal(have, wand_left) {
            Ok(wand_right.clone())
        } else {
            Err(ProofError::TacticFailed(
                "Cannot apply wand elimination: assertion mismatch".into(),
            ))
        }
    }

    /// Check if two assertions describe disjoint heaps
    pub fn are_disjoint(&self, left: &SepAssertion, right: &SepAssertion) -> bool {
        let left_locs = self.extract_locations(left);
        let right_locs = self.extract_locations(right);

        for l1 in left_locs.iter() {
            for l2 in right_locs.iter() {
                if format!("{:?}", l1) == format!("{:?}", l2) {
                    return false;
                }
            }
        }
        true
    }
}

impl Default for SeparationLogic {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Helper Functions ====================

/// Create a list reversal verification example
pub fn list_reversal_example() -> HoareTriple {
    use verum_ast::span::Span;

    let pre = SepAssertion::Pure(Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("list", Span::dummy()))),
        Span::dummy(),
    ));

    let command = Command::Assign {
        var: "ys".into(),
        expr: Expr::new(
            ExprKind::Call {
                func: Heap::new(Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new("reverse", Span::dummy()))),
                    Span::dummy(),
                )),
                type_args: List::new(),
                args: List::from_iter(vec![Expr::new(
                    ExprKind::Path(Path::from_ident(Ident::new("xs", Span::dummy()))),
                    Span::dummy(),
                )]),
            },
            Span::dummy(),
        ),
    };

    let post = SepAssertion::Pure(Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("list_reversed", Span::dummy()))),
        Span::dummy(),
    ));

    HoareTriple::new(pre, command, post)
}

/// Create a heap allocation example
pub fn alloc_example() -> HoareTriple {
    use verum_ast::span::Span;

    let pre = SepAssertion::Emp;

    let command = Command::Alloc {
        result: "x".into(),
        size: Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit::new(1)),
                Span::dummy(),
            )),
            Span::dummy(),
        ),
    };

    let post = SepAssertion::PointsTo {
        location: Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new("x", Span::dummy()))),
            Span::dummy(),
        ),
        value: Expr::new(
            ExprKind::Path(Path::from_ident(Ident::new("_", Span::dummy()))),
            Span::dummy(),
        ),
    };

    HoareTriple::new(pre, command, post)
}

/// Create a heap read example
pub fn read_example() -> HoareTriple {
    use verum_ast::span::Span;

    let x_var = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("x", Span::dummy()))),
        Span::dummy(),
    );

    let val_42 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(42)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let pre = SepAssertion::PointsTo {
        location: x_var.clone(),
        value: val_42.clone(),
    };

    let command = Command::Read {
        result: "y".into(),
        addr: x_var.clone(),
    };

    let y_var = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("y", Span::dummy()))),
        Span::dummy(),
    );

    let y_eq_42 = Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(y_var),
            right: Heap::new(val_42.clone()),
        },
        Span::dummy(),
    );

    let post = SepAssertion::And {
        left: Heap::new(SepAssertion::PointsTo {
            location: x_var,
            value: val_42,
        }),
        right: Heap::new(SepAssertion::Pure(y_eq_42)),
    };

    HoareTriple::new(pre, command, post)
}

/// Create a heap write example
pub fn write_example() -> HoareTriple {
    use verum_ast::span::Span;

    let x_var = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("x", Span::dummy()))),
        Span::dummy(),
    );

    let wildcard = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("_", Span::dummy()))),
        Span::dummy(),
    );

    let val_42 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(42)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let pre = SepAssertion::PointsTo {
        location: x_var.clone(),
        value: wildcard,
    };

    let command = Command::Write {
        addr: x_var.clone(),
        value: val_42.clone(),
    };

    let post = SepAssertion::PointsTo {
        location: x_var,
        value: val_42,
    };

    HoareTriple::new(pre, command, post)
}

/// Create a heap free example
pub fn free_example() -> HoareTriple {
    use verum_ast::span::Span;

    let x_var = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("x", Span::dummy()))),
        Span::dummy(),
    );

    let val_42 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(42)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let size_1 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(1)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let pre = SepAssertion::PointsTo {
        location: x_var.clone(),
        value: val_42,
    };

    let command = Command::Free {
        ptr: x_var,
        size: size_1,
    };

    let post = SepAssertion::Emp;

    HoareTriple::new(pre, command, post)
}

/// Create a list segment example
pub fn list_segment_example() -> HoareTriple {
    use verum_ast::span::Span;

    let head = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("head", Span::dummy()))),
        Span::dummy(),
    );

    let tail = Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new("tail", Span::dummy()))),
        Span::dummy(),
    );

    let elem1 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(1)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let elem2 = Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(2)),
            Span::dummy(),
        )),
        Span::dummy(),
    );

    let pre = SepAssertion::ListSegment {
        from: head.clone(),
        to: tail.clone(),
        elements: List::from_iter(vec![elem1.clone(), elem2.clone()]),
    };

    let command = Command::Skip;

    let post = SepAssertion::ListSegment {
        from: head,
        to: tail,
        elements: List::from_iter(vec![elem1, elem2]),
    };

    HoareTriple::new(pre, command, post)
}
