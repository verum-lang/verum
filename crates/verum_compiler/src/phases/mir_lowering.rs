//! MIR (Mid-level IR) Infrastructure for Verification and Analysis
//!
//! **IMPORTANT: This module is NOT part of the main compilation pipeline.**
//!
//! The actual Verum compilation uses a VBC-first architecture (v2.1):
//! ```text
//! Source → TypedAST → VBC Bytecode → { Interpreter (Tier 0) | AOT (Tier 1) }
//! ```
//!
//! MIR exists for specialized analysis and verification purposes only:
//! - SMT-based contract verification (`phases::verification_phase`)
//! - Advanced optimization analysis (`phases::optimization`)
//! - CBGR bounds elimination (`passes::cbgr_integration`)
//!
//! For the main compilation path, see `phases::vbc_codegen`.
//!
//! ## MIR Properties
//!
//! - Control Flow Graph (CFG) representation
//! - Static Single Assignment (SSA) form
//! - Explicit CBGR checks inserted
//! - Bounds checks inserted
//! - Move semantics explicit
//! - Temporaries made explicit
//! - Pattern matching lowered to decisions
//! - **ThinRef vs FatRef selection** (two-tier reference system)
//!
//! ## Two-Tier Reference System
//!
//! Memory model and CBGR: three-tier references (&T ~15ns, &checked T 0ns, &unsafe T 0ns).
//!
//! **ThinRef (`&T`)**: 16-byte pointer (8-byte ptr + CBGR metadata)
//! - Used for: Most references, default choice
//! - Overhead: ~15ns per access (CBGR check)
//! - Size: 16 bytes (pointer + generation + epoch/caps)
//!
//! **FatRef (`&checked T`)**: 24-byte fat pointer
//! - Used for: Array slices, dynamic bounds checking, trait objects
//! - Overhead: ~2-3ns per access (inline bounds check)
//! - Size: 24 bytes (pointer + metadata + length/vtable)
//!
//! Selection happens during MIR lowering based on:
//! 1. Type of reference (array slice -> FatRef, other -> ThinRef)
//! 2. Escape analysis results (NoEscape -> can eliminate checks)
//! 3. Optimization level
//!
//! ## Future Integration
//!
//! MIR may be integrated into the verification pipeline as an optional
//! analysis pass for functions with contracts (`requires`/`ensures`).
//!
//! Phase 5: HIR to MIR lowering. Creates control flow graph (CFG), inserts
//! safety checks (CBGR, bounds, overflow), tracks unsafe regions.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use verum_ast::{
    Expr, Module, Pattern, Stmt, Type,
    decl::{FunctionBody, FunctionDecl, FunctionParamKind, ItemKind},
    expr::{ArrayExpr, BinOp, Block, ConditionKind, ExprKind, RecoverBody, UnOp},
    literal::LiteralKind,
    pattern::PatternKind,
    span::Span as AstSpan,
    stmt::StmtKind,
    ty::TypeKind,
};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_common::{ConstValue, List, Text};
use verum_types::const_eval::ConstEvaluator;

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

// =============================================================================
// MIR Core Data Structures
// =============================================================================

/// A complete MIR module containing lowered functions
#[derive(Debug, Clone)]
pub struct MirModule {
    pub name: Text,
    pub functions: List<MirFunction>,
    pub types: List<MirTypeDef>,
    pub globals: List<MirGlobal>,
}

/// A MIR function with basic blocks in CFG form
#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: Text,
    pub signature: MirSignature,
    pub locals: List<MirLocal>,
    pub blocks: List<BasicBlock>,
    pub entry_block: BlockId,
    pub cleanup_blocks: List<BlockId>,
    pub span: AstSpan,
}

/// Function signature in MIR
#[derive(Debug, Clone)]
pub struct MirSignature {
    pub params: List<MirType>,
    pub return_type: MirType,
    pub contexts: List<Text>,
    pub is_async: bool,
}

/// A local variable or temporary in MIR (SSA form)
#[derive(Debug, Clone)]
pub struct MirLocal {
    pub id: LocalId,
    pub name: Text,
    pub ty: MirType,
    pub kind: LocalKind,
    /// SSA version number
    pub ssa_version: u32,
    /// Needs drop on scope exit
    pub needs_drop: bool,
    pub span: AstSpan,
}

/// Kind of local variable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// Function parameter
    Arg,
    /// User-declared variable
    Var,
    /// Compiler-generated temporary
    Temp,
    /// Return place
    ReturnPlace,
    /// Phi node destination
    Phi,
}

/// A basic block in the CFG
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub statements: List<MirStatement>,
    pub terminator: Terminator,
    pub predecessors: List<BlockId>,
    pub successors: List<BlockId>,
    /// Phi nodes for SSA form (at block entry)
    pub phi_nodes: List<PhiNode>,
    /// Is this a cleanup/unwind block?
    pub is_cleanup: bool,
}

/// Block identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

/// Local variable identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);

/// A statement in a basic block
#[derive(Debug, Clone)]
pub enum MirStatement {
    /// Assignment: place = rvalue
    Assign(Place, Rvalue),
    /// Storage live marker
    StorageLive(LocalId),
    /// Storage dead marker
    StorageDead(LocalId),
    /// CBGR bounds check: bounds_check(array, index)
    BoundsCheck { array: Place, index: Place },
    /// CBGR generation check for reference validation
    GenerationCheck(Place),
    /// CBGR epoch check for cross-epoch safety
    EpochCheck(Place),
    /// CBGR capability check for write permissions
    CapabilityCheck { place: Place, required: Capability },
    /// Drop a value
    Drop(Place),
    /// Drop in place (destructor only, no deallocation)
    DropInPlace(Place),
    /// Defer cleanup registration
    DeferCleanup { cleanup_block: BlockId },
    /// Execute deferred cleanups
    RunDeferredCleanups,
    /// Set discriminant (for enum variants)
    SetDiscriminant { place: Place, variant_idx: usize },
    /// Retag for Stacked Borrows / CBGR
    Retag { place: Place, kind: RetagKind },
    /// No operation
    Nop,
    /// Debug variable info
    DebugVar { local: LocalId, name: Text },
    /// Context provision: provide context_name = value
    /// Level 2 dynamic contexts: runtime dependency injection via task-local storage.
    ContextProvide {
        /// Context name (e.g., "Database", "Logger")
        context_name: Text,
        /// Place holding the context value
        value: Place,
    },
    /// Context unprovide (cleanup at scope exit)
    ContextUnprovide {
        /// Context name to remove from scope
        context_name: Text,
    },
    /// Context resolve: get context value by name
    /// Used for `using [ContextName]` resolution
    ContextResolve {
        /// Target place to store resolved context
        target: Place,
        /// Context name to resolve
        context_name: Text,
    },
}

/// Retag kind for borrow tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetagKind {
    /// Function entry retag
    FnEntry,
    /// Two-phase borrow
    TwoPhase,
    /// Raw pointer
    Raw,
    /// Default retag
    Default,
}

/// CBGR capability flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Write,
    Execute,
    ReadWrite,
}

/// Block terminator (control flow)
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Unconditional jump
    Goto(BlockId),
    /// Conditional branch based on integer value
    SwitchInt {
        discriminant: Operand,
        targets: List<(i64, BlockId)>,
        otherwise: BlockId,
    },
    /// Boolean branch (if/else)
    Branch {
        condition: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Return from function
    Return,
    /// Unreachable code
    Unreachable,
    /// Function call with continuation blocks
    Call {
        destination: Place,
        func: Operand,
        args: List<Operand>,
        success_block: BlockId,
        unwind_block: BlockId,
    },
    /// Async function call
    AsyncCall {
        destination: Place,
        func: Operand,
        args: List<Operand>,
        success_block: BlockId,
        unwind_block: BlockId,
    },
    /// Await point (suspend and resume)
    Await {
        future: Place,
        destination: Place,
        resume_block: BlockId,
        unwind_block: BlockId,
    },
    /// Assert condition with message
    Assert {
        condition: Operand,
        expected: bool,
        msg: Text,
        target: BlockId,
        unwind: BlockId,
    },
    /// Cleanup block terminator
    Cleanup { place: Place, target: BlockId },
    /// Drop with unwinding
    DropAndReplace {
        place: Place,
        value: Operand,
        target: BlockId,
        unwind: BlockId,
    },
    /// Resume unwinding
    Resume,
    /// Abort execution
    Abort,
    /// Yield from generator
    Yield {
        value: Operand,
        resume: BlockId,
        drop: BlockId,
    },
    /// Inline assembly terminator
    InlineAsm {
        /// Assembly template string
        template: Text,
        /// Input operands: (constraint, value)
        inputs: List<(Text, Operand)>,
        /// Output operands: (constraint, place)
        outputs: List<(Text, Place)>,
        /// Clobbered registers
        clobbers: List<Text>,
        /// Assembly options
        options: MirAsmOptions,
        /// Continuation block after asm (None for diverging asm)
        destination: Option<BlockId>,
        /// Unwind block if asm can unwind
        unwind: Option<BlockId>,
    },
}

/// MIR-level inline assembly options
#[derive(Debug, Clone, Default)]
pub struct MirAsmOptions {
    pub volatile: bool,
    pub pure_: bool,
    pub nomem: bool,
    pub readonly: bool,
    pub preserves_flags: bool,
    pub nostack: bool,
    pub att_syntax: bool,
}

impl Default for Terminator {
    fn default() -> Self {
        Terminator::Unreachable
    }
}

/// Right-hand side value
#[derive(Debug, Clone)]
pub enum Rvalue {
    /// Use an operand as-is
    Use(Operand),
    /// Binary operation
    Binary(BinOp, Operand, Operand),
    /// Unary operation
    Unary(UnOp, Operand),
    /// Create a reference (with CBGR metadata)
    Ref(BorrowKind, Place),
    /// Dereference a pointer
    Deref(Place),
    /// Type cast
    Cast(CastKind, Operand, MirType),
    /// Aggregate (tuple, struct, array)
    Aggregate(AggregateKind, List<Operand>),
    /// Discriminant read (for enums)
    Discriminant(Place),
    /// Array/slice length
    Len(Place),
    /// Null/None constant
    NullConstant,
    /// Checked binary operation (overflow checking)
    CheckedBinary(BinOp, Operand, Operand),
    /// Address of (creates raw pointer)
    AddressOf(Mutability, Place),
    /// Shallow init check (for uninitialized memory)
    ShallowInitBox(Operand, MirType),
    /// Copy for [T; N] where T: Copy
    CopyForDeref(Place),
    /// Thread-local reference
    ThreadLocalRef(Text),
    /// Repeat expression [value; count]
    Repeat(Operand, usize),
}

/// Cast kinds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    /// Pointer to pointer
    Pointer,
    /// Int to int (possibly with sign extension)
    IntToInt,
    /// Float to int
    FloatToInt,
    /// Int to float
    IntToFloat,
    /// Float to float
    FloatToFloat,
    /// Pointer to int
    PointerToInt,
    /// Int to pointer
    IntToPointer,
    /// Enum to int
    EnumToInt,
    /// Function to pointer
    FnToPointer,
    /// Unsizing coercion
    Unsize,
    /// Transmute (bit cast)
    Transmute,
}

/// Mutability marker
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {
    Immutable,
    Mutable,
}

/// Kind of aggregate
#[derive(Debug, Clone)]
pub enum AggregateKind {
    Tuple,
    Array(MirType),
    Struct(Text),
    Variant(Text, usize), // enum name, variant index
    Closure(Text, List<Place>),
    Generator(Text),
    /// Map comprehension aggregate
    Map,
    /// Set comprehension aggregate
    Set,
}

/// Borrow kind for references
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    /// Shared reference &T
    Shared,
    /// Mutable reference &mut T
    Mutable,
    /// Unique/move reference
    Unique,
    /// Shallow borrow (for closures)
    Shallow,
}

/// An operand in MIR
#[derive(Debug, Clone)]
pub enum Operand {
    /// Copy a place
    Copy(Place),
    /// Move a place
    Move(Place),
    /// Constant value
    Constant(MirConstant),
}

/// A place (memory location)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projections: List<PlaceProjection>,
}

impl Place {
    pub fn local(id: LocalId) -> Self {
        Place {
            local: id,
            projections: List::new(),
        }
    }

    pub fn return_place() -> Self {
        Place {
            local: LocalId(0), // By convention, _0 is return place
            projections: List::new(),
        }
    }

    pub fn with_field(mut self, field: usize) -> Self {
        self.projections.push(PlaceProjection::Field(field));
        self
    }

    pub fn with_index(mut self, index: LocalId) -> Self {
        self.projections.push(PlaceProjection::Index(index));
        self
    }

    pub fn with_deref(mut self) -> Self {
        self.projections.push(PlaceProjection::Deref);
        self
    }

    pub fn with_downcast(mut self, variant: usize) -> Self {
        self.projections.push(PlaceProjection::Downcast(variant));
        self
    }
}

/// Projection from a place
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlaceProjection {
    /// Field access .field
    Field(usize),
    /// Array index [i]
    Index(LocalId),
    /// Constant index [n]
    ConstantIndex { offset: usize, from_end: bool },
    /// Subslice [a..b]
    Subslice { from: usize, to: usize },
    /// Dereference *ptr
    Deref,
    /// Downcast to variant
    Downcast(usize),
    /// Opaque cast (for layout optimization)
    OpaqueCast(MirType),
}

/// A constant value in MIR
#[derive(Debug, Clone)]
pub enum MirConstant {
    /// Unit value
    Unit,
    /// Boolean
    Bool(bool),
    /// Integer
    Int(i64),
    /// Unsigned integer
    UInt(u64),
    /// Float
    Float(f64),
    /// Character
    Char(char),
    /// String
    String(Text),
    /// Function pointer
    Function(Text),
    /// Static reference
    Static(Text),
    /// Undef/uninitialized
    Undef,
    /// Zero-sized type value
    Zst,
    /// Array of constants (for compile-time arrays)
    Array(List<MirConstant>),
    /// Tuple of constants (for compile-time tuples)
    Tuple(List<MirConstant>),
    /// Byte string literal (b"...")
    Bytes(Vec<u8>),
}

/// MIR type representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirType {
    Unit,
    Bool,
    Int,
    UInt,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    Float,
    F32,
    F64,
    Char,
    Text,
    /// Named type
    Named(Text),
    /// Tuple type
    Tuple(List<MirType>),
    /// Array type with size
    Array(Box<MirType>, usize),
    /// Slice type
    Slice(Box<MirType>),
    /// Reference with layout info
    Ref {
        inner: Box<MirType>,
        mutable: bool,
        layout: ReferenceLayout,
    },
    /// Raw pointer
    Pointer {
        inner: Box<MirType>,
        mutable: bool,
    },
    /// Function type
    Function {
        params: List<MirType>,
        ret: Box<MirType>,
    },
    /// Never type (diverges)
    Never,
    /// Inferred/unknown type
    Infer,
    /// Generator state type
    Generator {
        name: Text,
        state_types: List<MirType>,
    },
}

/// Reference layout (ThinRef vs FatRef)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceLayout {
    /// ThinRef: 16 bytes (8-byte ptr + 4-byte gen + 4-byte epoch/caps)
    ThinRef,
    /// FatRef: 24 bytes (16-byte ThinRef + 8-byte metadata)
    FatRef(MetadataKind),
}

/// Kind of metadata for fat references
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataKind {
    /// Slice length
    Length,
    /// Vtable pointer for trait objects
    VTable,
}

/// Type definition in MIR
#[derive(Debug, Clone)]
pub struct MirTypeDef {
    pub name: Text,
    pub kind: MirTypeDefKind,
}

/// Kind of type definition
#[derive(Debug, Clone)]
pub enum MirTypeDefKind {
    Struct(List<(Text, MirType)>),
    Enum(List<(Text, List<MirType>)>),
    Alias(MirType),
}

/// Global variable in MIR
#[derive(Debug, Clone)]
pub struct MirGlobal {
    pub name: Text,
    pub ty: MirType,
    pub mutable: bool,
    pub initializer: Option<MirConstant>,
}

// =============================================================================
// SSA Support
// =============================================================================

/// Phi node for SSA form
#[derive(Debug, Clone)]
pub struct PhiNode {
    pub dest: LocalId,
    pub operands: List<(BlockId, Operand)>,
    pub ty: MirType,
}

/// SSA name for tracking versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SsaName {
    pub local: LocalId,
    pub version: u32,
}

// =============================================================================
// Control Flow Graph Analysis
// =============================================================================

/// Dominator tree for CFG analysis
#[derive(Debug, Clone)]
pub struct DominatorTree {
    /// Immediate dominator for each block
    idom: HashMap<BlockId, BlockId>,
    /// Dominance frontier for each block
    frontier: HashMap<BlockId, HashSet<BlockId>>,
}

impl DominatorTree {
    /// Compute dominator tree using Cooper-Harvey-Kennedy algorithm
    pub fn compute(blocks: &[BasicBlock], entry: BlockId) -> Self {
        let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
        let mut changed = true;

        // Initialize: entry dominates itself
        idom.insert(entry, entry);

        // Get reverse postorder
        let rpo = Self::reverse_postorder(blocks, entry);
        let rpo_index: HashMap<BlockId, usize> =
            rpo.iter().enumerate().map(|(i, &b)| (b, i)).collect();

        // Iterate until fixed point
        // SAFETY: Max iterations prevent infinite loop on malformed CFG
        const MAX_FIXPOINT_ITERATIONS: usize = 10000;
        let mut iterations = 0;

        while changed && iterations < MAX_FIXPOINT_ITERATIONS {
            iterations += 1;
            changed = false;
            for &block_id in &rpo {
                if block_id == entry {
                    continue;
                }

                let block = &blocks[block_id.0];
                let mut new_idom: Option<BlockId> = None;

                for &pred in block.predecessors.iter() {
                    if idom.contains_key(&pred) {
                        new_idom = Some(match new_idom {
                            None => pred,
                            Some(current) => Self::intersect(&idom, &rpo_index, pred, current),
                        });
                    }
                }

                if let Some(new_idom) = new_idom {
                    if idom.get(&block_id) != Some(&new_idom) {
                        idom.insert(block_id, new_idom);
                        changed = true;
                    }
                }
            }
        }

        if iterations >= MAX_FIXPOINT_ITERATIONS {
            eprintln!(
                "Warning: Dominator computation did not converge after {} iterations",
                MAX_FIXPOINT_ITERATIONS
            );
        }

        // Compute dominance frontiers
        let frontier = Self::compute_frontiers(blocks, &idom);

        DominatorTree { idom, frontier }
    }

    fn intersect(
        idom: &HashMap<BlockId, BlockId>,
        rpo_index: &HashMap<BlockId, usize>,
        mut b1: BlockId,
        mut b2: BlockId,
    ) -> BlockId {
        while b1 != b2 {
            while rpo_index.get(&b1) > rpo_index.get(&b2) {
                b1 = idom[&b1];
            }
            while rpo_index.get(&b2) > rpo_index.get(&b1) {
                b2 = idom[&b2];
            }
        }
        b1
    }

    fn reverse_postorder(blocks: &[BasicBlock], entry: BlockId) -> Vec<BlockId> {
        let mut visited = HashSet::new();
        let mut postorder = Vec::new();
        Self::dfs_postorder(blocks, entry, &mut visited, &mut postorder);
        postorder.reverse();
        postorder
    }

    fn dfs_postorder(
        blocks: &[BasicBlock],
        block: BlockId,
        visited: &mut HashSet<BlockId>,
        postorder: &mut Vec<BlockId>,
    ) {
        if !visited.insert(block) {
            return;
        }
        if block.0 < blocks.len() {
            for &succ in blocks[block.0].successors.iter() {
                Self::dfs_postorder(blocks, succ, visited, postorder);
            }
        }
        postorder.push(block);
    }

    fn compute_frontiers(
        blocks: &[BasicBlock],
        idom: &HashMap<BlockId, BlockId>,
    ) -> HashMap<BlockId, HashSet<BlockId>> {
        let mut frontier: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();

        for block in blocks {
            if block.predecessors.len() >= 2 {
                for &pred in block.predecessors.iter() {
                    let mut runner = pred;
                    while runner != *idom.get(&block.id).unwrap_or(&block.id) {
                        frontier.entry(runner).or_default().insert(block.id);
                        runner = *idom.get(&runner).unwrap_or(&runner);
                        if runner == *idom.get(&runner).unwrap_or(&runner) {
                            break;
                        }
                    }
                }
            }
        }

        frontier
    }

    /// Check if a dominates b
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if a == b {
            return true;
        }
        let mut current = b;
        while let Some(&dom) = self.idom.get(&current) {
            if dom == a {
                return true;
            }
            if dom == current {
                break;
            }
            current = dom;
        }
        false
    }

    /// Get dominance frontier for a block
    pub fn dominance_frontier(&self, block: BlockId) -> Option<&HashSet<BlockId>> {
        self.frontier.get(&block)
    }
}

/// Loop information for CFG
#[derive(Debug, Clone)]
pub struct LoopInfo {
    /// Loop headers (blocks that are targets of back edges)
    pub headers: HashSet<BlockId>,
    /// For each loop header, the blocks in the loop body
    pub loop_bodies: HashMap<BlockId, HashSet<BlockId>>,
    /// Loop nesting depth for each block
    pub depth: HashMap<BlockId, usize>,
    /// Back edges (source -> header)
    pub back_edges: Vec<(BlockId, BlockId)>,
}

impl LoopInfo {
    /// Detect loops in CFG
    pub fn compute(blocks: &[BasicBlock], _entry: BlockId, dominator: &DominatorTree) -> Self {
        let mut headers = HashSet::new();
        let mut loop_bodies: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
        let mut back_edges = Vec::new();

        // Find back edges (edges where target dominates source)
        for block in blocks {
            for &succ in block.successors.iter() {
                if dominator.dominates(succ, block.id) {
                    // This is a back edge, succ is a loop header
                    headers.insert(succ);
                    back_edges.push((block.id, succ));

                    // Compute loop body using reverse DFS from block to header
                    let body = Self::compute_loop_body(blocks, succ, block.id);
                    loop_bodies.insert(succ, body);
                }
            }
        }

        // Compute nesting depth
        let depth = Self::compute_depth(blocks, &loop_bodies);

        LoopInfo {
            headers,
            loop_bodies,
            depth,
            back_edges,
        }
    }

    fn compute_loop_body(
        blocks: &[BasicBlock],
        header: BlockId,
        back_edge_source: BlockId,
    ) -> HashSet<BlockId> {
        let mut body = HashSet::new();
        body.insert(header);

        let mut worklist = vec![back_edge_source];

        while let Some(block_id) = worklist.pop() {
            if body.insert(block_id) {
                if block_id.0 < blocks.len() {
                    for &pred in blocks[block_id.0].predecessors.iter() {
                        worklist.push(pred);
                    }
                }
            }
        }

        body
    }

    fn compute_depth(
        blocks: &[BasicBlock],
        loop_bodies: &HashMap<BlockId, HashSet<BlockId>>,
    ) -> HashMap<BlockId, usize> {
        let mut depth = HashMap::new();

        for block in blocks {
            let mut d = 0;
            for (_, body) in loop_bodies {
                if body.contains(&block.id) {
                    d += 1;
                }
            }
            depth.insert(block.id, d);
        }

        depth
    }

    /// Check if a block is a loop header
    pub fn is_loop_header(&self, block: BlockId) -> bool {
        self.headers.contains(&block)
    }

    /// Get the loop header for a block (if in a loop)
    pub fn containing_loop(&self, block: BlockId) -> Option<BlockId> {
        for (&header, body) in &self.loop_bodies {
            if body.contains(&block) {
                return Some(header);
            }
        }
        None
    }
}

// =============================================================================
// Type Registry for Field and Variant Resolution
// =============================================================================

/// Registry for struct and enum type definitions
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Struct definitions: name -> list of (field_name, field_type, field_index)
    structs: HashMap<Text, Vec<(Text, MirType, usize)>>,
    /// Enum definitions: name -> list of (variant_name, variant_fields, discriminant)
    enums: HashMap<Text, Vec<(Text, Vec<MirType>, i64)>>,
    /// Type name to qualified name mapping (for handling module paths)
    type_aliases: HashMap<Text, Text>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a struct type definition
    pub fn register_struct(&mut self, name: Text, fields: Vec<(Text, MirType)>) {
        let indexed_fields: Vec<(Text, MirType, usize)> = fields
            .into_iter()
            .enumerate()
            .map(|(idx, (name, ty))| (name, ty, idx))
            .collect();
        self.structs.insert(name, indexed_fields);
    }

    /// Register an enum type definition
    pub fn register_enum(&mut self, name: Text, variants: Vec<(Text, Vec<MirType>)>) {
        let indexed_variants: Vec<(Text, Vec<MirType>, i64)> = variants
            .into_iter()
            .enumerate()
            .map(|(idx, (name, fields))| (name, fields, idx as i64))
            .collect();
        self.enums.insert(name, indexed_variants);
    }

    /// Look up a struct field by name, returning its index
    pub fn get_field_index(&self, struct_name: &str, field_name: &str) -> Option<usize> {
        // Try direct lookup first
        if let Some(fields) = self.structs.get(struct_name) {
            for (fname, _, idx) in fields {
                if fname.as_str() == field_name {
                    return Some(*idx);
                }
            }
        }

        // Try with alias resolution
        if let Some(resolved_name) = self.type_aliases.get(struct_name) {
            return self.get_field_index(resolved_name.as_str(), field_name);
        }

        None
    }

    /// Look up an enum variant discriminant by path
    pub fn get_variant_discriminant(&self, enum_name: &str, variant_name: &str) -> Option<i64> {
        // Try direct lookup first
        if let Some(variants) = self.enums.get(enum_name) {
            for (vname, _, disc) in variants {
                if vname.as_str() == variant_name {
                    return Some(*disc);
                }
            }
        }

        // Try with alias resolution
        if let Some(resolved_name) = self.type_aliases.get(enum_name) {
            return self.get_variant_discriminant(resolved_name.as_str(), variant_name);
        }

        None
    }

    /// Get struct definition for debugging
    pub fn get_struct(&self, name: &str) -> Option<&Vec<(Text, MirType, usize)>> {
        self.structs.get(name)
    }

    /// Get enum definition for debugging
    pub fn get_enum(&self, name: &str) -> Option<&Vec<(Text, Vec<MirType>, i64)>> {
        self.enums.get(name)
    }
}

// =============================================================================
// MIR Lowering Context
// =============================================================================

/// Context for lowering HIR/AST to MIR
pub struct LoweringContext {
    /// Current function being lowered
    current_func: Option<MirFunction>,
    /// Next block ID
    next_block: usize,
    /// Next local ID
    next_local: usize,
    /// Variable name to local ID mapping (current scope)
    var_map: HashMap<Text, LocalId>,
    /// SSA version tracking for each variable
    ssa_versions: HashMap<LocalId, u32>,
    /// Scope stack for variable shadowing
    scope_stack: Vec<HashMap<Text, LocalId>>,
    /// Loop context stack (header, exit, continue_target)
    loop_stack: Vec<LoopContext>,
    /// Defer stack for cleanup
    defer_stack: Vec<BlockId>,
    /// Drop flags for conditional drop
    drop_flags: HashMap<LocalId, LocalId>,
    /// Type information cache
    type_cache: HashMap<LocalId, MirType>,
    /// Type registry for struct and enum definitions
    type_registry: TypeRegistry,
    /// Const evaluator for compile-time expressions
    const_evaluator: ConstEvaluator,
    /// Statistics
    stats: LoweringStats,
    /// Diagnostics
    pub diagnostics: List<Diagnostic>,
    /// Recursion depth tracking to prevent stack overflow
    recursion_depth: usize,
}

/// Loop context for break/continue
#[derive(Clone, Copy)]
pub struct LoopContext {
    pub header: BlockId,
    pub exit: BlockId,
    pub continue_target: BlockId,
    /// Optional result place for loop expressions
    pub result: Option<LocalId>,
}

/// Statistics for MIR lowering
#[derive(Debug, Clone, Default)]
pub struct LoweringStats {
    /// Number of functions lowered
    pub functions_lowered: usize,
    /// Number of basic blocks created
    pub blocks_created: usize,
    /// Number of instructions generated
    pub instructions_generated: usize,
    /// Number of CBGR checks inserted
    pub cbgr_checks_inserted: usize,
    /// Number of bounds checks inserted
    pub bounds_checks_inserted: usize,
    /// Number of ThinRef references selected
    pub thin_refs_selected: usize,
    /// Number of FatRef references selected
    pub fat_refs_selected: usize,
    /// Number of temporaries created
    pub temps_created: usize,
    /// Number of phi nodes created
    pub phi_nodes_created: usize,
    /// Number of drop statements inserted
    pub drops_inserted: usize,
    /// Number of SSA versions created
    pub ssa_versions_created: usize,
}

impl LoweringContext {
    pub fn new() -> Self {
        LoweringContext {
            current_func: None,
            next_block: 0,
            next_local: 0,
            var_map: HashMap::new(),
            ssa_versions: HashMap::new(),
            scope_stack: Vec::new(),
            loop_stack: Vec::new(),
            defer_stack: Vec::new(),
            drop_flags: HashMap::new(),
            type_cache: HashMap::new(),
            type_registry: TypeRegistry::new(),
            const_evaluator: ConstEvaluator::new(),
            stats: LoweringStats::default(),
            diagnostics: List::new(),
            recursion_depth: 0,
        }
    }

    /// Convert AST Span to diagnostic LineColSpan
    ///
    /// Note: Full line/column conversion requires access to the source file
    /// which is managed at the Session level. This method provides a best-effort
    /// conversion using available information from the span.
    ///
    /// For accurate line/column information, diagnostics should be created at
    /// a higher level where Session is available, or the span should be stored
    /// and converted later via Session::convert_span().
    fn span_to_diag(&self, span: AstSpan) -> verum_diagnostics::Span {
        // Handle dummy/synthetic spans
        if span.is_dummy() {
            return verum_diagnostics::Span::new("<generated>", 1, 1, 1);
        }

        // Provide file ID and byte offset information for debugging
        // Real line/column conversion happens at Session level with source file access
        let file_info = if span.file_id.is_dummy() {
            "<generated>".to_string()
        } else {
            format!("<file:{}>", span.file_id.raw())
        };

        // Estimate line number from byte offset (very rough - 80 chars per line average)
        // This is just for debugging; accurate conversion needs source file
        let estimated_line = if span.start > 0 {
            (span.start / 80) as usize + 1
        } else {
            1
        };

        let estimated_col = if span.start > 0 {
            (span.start % 80) as usize + 1
        } else {
            1
        };

        let estimated_end_col = if span.end > span.start {
            estimated_col + (span.end - span.start) as usize
        } else {
            estimated_col + 1
        };

        verum_diagnostics::Span::new(&file_info, estimated_line, estimated_col, estimated_end_col)
    }

    /// Create a new basic block
    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.stats.blocks_created += 1;

        if let Some(ref mut func) = self.current_func {
            func.blocks.push(BasicBlock {
                id,
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::new(),
                successors: List::new(),
                phi_nodes: List::new(),
                is_cleanup: false,
            });
        }

        id
    }

    /// Create a new cleanup block
    fn new_cleanup_block(&mut self) -> BlockId {
        let id = self.new_block();

        if let Some(ref mut func) = self.current_func {
            if let Some(block) = func.blocks.iter_mut().find(|b| b.id == id) {
                block.is_cleanup = true;
            }
            func.cleanup_blocks.push(id);
        }

        id
    }

    /// Create a new local variable with SSA versioning
    fn new_local(&mut self, name: impl Into<Text>, ty: MirType, kind: LocalKind) -> LocalId {
        let id = LocalId(self.next_local);
        self.next_local += 1;

        let name = name.into();
        let needs_drop = self.type_needs_drop(&ty);

        // Initialize SSA version
        self.ssa_versions.insert(id, 0);
        self.stats.ssa_versions_created += 1;

        if let Some(ref mut func) = self.current_func {
            func.locals.push(MirLocal {
                id,
                name: name.clone(),
                ty: ty.clone(),
                kind,
                ssa_version: 0,
                needs_drop,
                span: AstSpan::dummy(),
            });
        }

        // Cache type
        self.type_cache.insert(id, ty);

        // Track in var_map if it's a named variable
        if kind == LocalKind::Var || kind == LocalKind::Arg {
            self.var_map.insert(name, id);
        }

        id
    }

    /// Create a new temporary
    fn new_temp(&mut self, ty: MirType) -> LocalId {
        self.stats.temps_created += 1;
        self.new_local(format!("_tmp{}", self.next_local), ty, LocalKind::Temp)
    }

    /// Get next SSA version for a local
    fn next_ssa_version(&mut self, local: LocalId) -> u32 {
        let version = self.ssa_versions.entry(local).or_insert(0);
        *version += 1;
        self.stats.ssa_versions_created += 1;
        *version
    }

    /// Check if type needs drop
    fn type_needs_drop(&self, ty: &MirType) -> bool {
        match ty {
            MirType::Unit
            | MirType::Bool
            | MirType::Int
            | MirType::UInt
            | MirType::I8
            | MirType::I16
            | MirType::I32
            | MirType::I64
            | MirType::I128
            | MirType::U8
            | MirType::U16
            | MirType::U32
            | MirType::U64
            | MirType::U128
            | MirType::Float
            | MirType::F32
            | MirType::F64
            | MirType::Char
            | MirType::Never
            | MirType::Infer => false,
            MirType::Text
            | MirType::Named(_)
            | MirType::Slice(_)
            | MirType::Array(_, _)
            | MirType::Generator { .. } => true,
            MirType::Tuple(types) => types.iter().any(|t| self.type_needs_drop(t)),
            MirType::Ref { .. } | MirType::Pointer { .. } => false, // References don't own data
            MirType::Function { .. } => false,
        }
    }

    /// Push a new scope
    fn push_scope(&mut self) {
        self.scope_stack.push(self.var_map.clone());
    }

    /// Pop a scope and emit drops for local variables
    fn pop_scope(&mut self, block: BlockId) {
        if let Some(prev) = self.scope_stack.pop() {
            // Find variables that were added in this scope
            let new_vars: Vec<_> = self
                .var_map
                .iter()
                .filter(|(k, _)| !prev.contains_key(*k))
                .map(|(_, &v)| v)
                .collect();

            // Emit drops for new variables that need dropping
            for local in new_vars.into_iter().rev() {
                if let Some(ty) = self.type_cache.get(&local) {
                    if self.type_needs_drop(ty) {
                        self.push_statement(block, MirStatement::Drop(Place::local(local)));
                        self.stats.drops_inserted += 1;
                    }
                }
                self.push_statement(block, MirStatement::StorageDead(local));
            }

            self.var_map = prev;
        }
    }

    /// Pop scope without emitting drops (for early returns)
    fn pop_scope_no_drops(&mut self) {
        if let Some(prev) = self.scope_stack.pop() {
            self.var_map = prev;
        }
    }

    /// Look up a variable
    fn lookup_var(&self, name: &str) -> Option<LocalId> {
        self.var_map.get(name).copied()
    }

    /// Bind a variable name to a local ID
    fn bind_var(&mut self, name: &str, local: LocalId) {
        self.var_map.insert(name.to_string().into(), local);
    }

    /// Convert a const evaluation result to a MIR constant
    ///
    /// Used by meta block evaluation to embed compile-time computed values
    /// directly into MIR as constants.
    fn const_value_to_mir_constant(&self, value: &ConstValue) -> MirConstant {
        match value {
            ConstValue::Int(n) => MirConstant::Int(*n as i64),
            ConstValue::UInt(n) => MirConstant::UInt(*n as u64),
            ConstValue::Float(f) => MirConstant::Float(*f),
            ConstValue::Bool(b) => MirConstant::Bool(*b),
            ConstValue::Text(s) => MirConstant::String(s.clone()),
            ConstValue::Char(c) => MirConstant::Char(*c),
            ConstValue::Array(elements) => {
                let mir_elements: List<MirConstant> = elements
                    .iter()
                    .map(|e| self.const_value_to_mir_constant(e))
                    .collect();
                MirConstant::Array(mir_elements)
            }
            ConstValue::Tuple(elements) => {
                let mir_elements: List<MirConstant> = elements
                    .iter()
                    .map(|e| self.const_value_to_mir_constant(e))
                    .collect();
                MirConstant::Tuple(mir_elements)
            }
            ConstValue::Bytes(bytes) => MirConstant::Bytes(bytes.clone()),
            ConstValue::Unit => MirConstant::Unit,
            ConstValue::Maybe(maybe) => match maybe {
                verum_common::Maybe::None => MirConstant::Unit,
                verum_common::Maybe::Some(v) => {
                    MirConstant::Tuple(verum_common::List::from(vec![
                        self.const_value_to_mir_constant(v),
                    ]))
                }
            },
            // Convert Map to array of (key, value) tuples for MIR
            ConstValue::Map(map) => {
                let mir_elements: List<MirConstant> = map
                    .iter()
                    .map(|(k, v)| {
                        MirConstant::Tuple(verum_common::List::from(vec![
                            MirConstant::String(k.clone()),
                            self.const_value_to_mir_constant(v),
                        ]))
                    })
                    .collect();
                MirConstant::Array(mir_elements)
            }
            // Convert Set to array of strings for MIR
            ConstValue::Set(set) => {
                let mir_elements: List<MirConstant> = set
                    .iter()
                    .map(|s| MirConstant::String(s.clone()))
                    .collect();
                MirConstant::Array(mir_elements)
            }
        }
    }

    /// Convert an AST literal to a MIR constant
    ///
    /// Handles all literal kinds defined in verum_ast::literal::LiteralKind,
    /// producing appropriate MIR constants for code generation.
    fn lower_literal_to_constant(&self, lit: &verum_ast::literal::Literal) -> MirConstant {
        use verum_ast::literal::LiteralKind;

        match &lit.kind {
            LiteralKind::Bool(b) => MirConstant::Bool(*b),
            LiteralKind::Int(i) => MirConstant::Int(i.value as i64),
            LiteralKind::Float(f) => MirConstant::Float(f.value),
            LiteralKind::Char(c) => MirConstant::Char(*c),
            LiteralKind::ByteChar(b) => MirConstant::Int(*b as i64),
            LiteralKind::ByteString(bytes) => MirConstant::Bytes(bytes.clone()),
            LiteralKind::Text(s) => MirConstant::String(s.as_str().into()),

            // Tagged literals: d#"2025-11-05", sql#"SELECT...", rx#"pattern"
            // These are lowered to string constants with the tag preserved for runtime handling
            LiteralKind::Tagged { tag, content } => {
                // Create a tagged string representation that can be processed at runtime
                // Format: "tag:content" - the runtime or interpreter will handle the parsing
                MirConstant::String(format!("{}:{}", tag, content).into())
            }

            // Interpolated string literals are handled separately via ExprKind::InterpolatedString
            // If we encounter one here, it's a string literal with interpolation markers
            LiteralKind::InterpolatedString(interp) => {
                // For now, emit the raw content - full interpolation is handled elsewhere
                MirConstant::String(interp.content.clone().into())
            }

            // Contract literals: contract#"it > 0"
            // These are specification constructs, lowered as unit at runtime
            // The contract checking is done during verification, not runtime
            LiteralKind::Contract(_) => MirConstant::Unit,

            // Composite literals: mat#"[[1, 2], [3, 4]]", vec#"<1, 2, 3>"
            // These need runtime parsing based on their tag
            LiteralKind::Composite(composite) => {
                // Store as tagged string for runtime processing
                MirConstant::String(format!("{}:{}", composite.tag, composite.content).into())
            }

            // Context-adaptive literals: #FF5733
            // The interpretation depends on type context - handled during type inference
            // At MIR level, we emit the raw representation
            LiteralKind::ContextAdaptive(adaptive) => {
                MirConstant::String(adaptive.raw.clone().into())
            }
        }
    }

    /// Add a statement to a block
    fn push_statement(&mut self, block: BlockId, stmt: MirStatement) {
        if let Some(ref mut func) = self.current_func {
            if let Some(b) = func.blocks.iter_mut().find(|b| b.id == block) {
                b.statements.push(stmt);
                self.stats.instructions_generated += 1;
            }
        }
    }

    /// Add a phi node to a block
    fn add_phi_node(&mut self, block: BlockId, dest: LocalId, ty: MirType) {
        if let Some(ref mut func) = self.current_func {
            if let Some(b) = func.blocks.iter_mut().find(|b| b.id == block) {
                b.phi_nodes.push(PhiNode {
                    dest,
                    operands: List::new(),
                    ty,
                });
                self.stats.phi_nodes_created += 1;
            }
        }
    }

    /// Add phi operand
    fn add_phi_operand(
        &mut self,
        block: BlockId,
        dest: LocalId,
        from_block: BlockId,
        operand: Operand,
    ) {
        if let Some(ref mut func) = self.current_func {
            if let Some(b) = func.blocks.iter_mut().find(|b| b.id == block) {
                if let Some(phi) = b.phi_nodes.iter_mut().find(|p| p.dest == dest) {
                    phi.operands.push((from_block, operand));
                }
            }
        }
    }

    /// Set block terminator
    fn set_terminator(&mut self, block: BlockId, term: Terminator) {
        if let Some(ref mut func) = self.current_func {
            if let Some(b) = func.blocks.iter_mut().find(|b| b.id == block) {
                // Update successor information
                let successors = match &term {
                    Terminator::Goto(target) => vec![*target],
                    Terminator::SwitchInt {
                        targets, otherwise, ..
                    } => {
                        let mut succs: Vec<_> = targets.iter().map(|(_, b)| *b).collect();
                        succs.push(*otherwise);
                        succs
                    }
                    Terminator::Branch {
                        then_block,
                        else_block,
                        ..
                    } => {
                        vec![*then_block, *else_block]
                    }
                    Terminator::Call {
                        success_block,
                        unwind_block,
                        ..
                    }
                    | Terminator::AsyncCall {
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
                    _ => vec![],
                };

                b.successors = successors.into_iter().collect();
                b.terminator = term;
            }
        }

        // Update predecessor information
        if let Some(ref mut func) = self.current_func {
            let successors: Vec<BlockId> = func
                .blocks
                .iter()
                .find(|b| b.id == block)
                .map(|b| b.successors.iter().copied().collect())
                .unwrap_or_default();

            for succ in successors {
                if let Some(succ_block) = func.blocks.iter_mut().find(|b| b.id == succ) {
                    if !succ_block.predecessors.iter().any(|&p| p == block) {
                        succ_block.predecessors.push(block);
                    }
                }
            }
        }
    }

    /// Select reference layout based on type
    fn select_reference_layout(&mut self, inner_type: &MirType) -> ReferenceLayout {
        match inner_type {
            MirType::Slice(_) => {
                self.stats.fat_refs_selected += 1;
                ReferenceLayout::FatRef(MetadataKind::Length)
            }
            MirType::Named(name) if name.starts_with("dyn ") => {
                self.stats.fat_refs_selected += 1;
                ReferenceLayout::FatRef(MetadataKind::VTable)
            }
            _ => {
                self.stats.thin_refs_selected += 1;
                ReferenceLayout::ThinRef
            }
        }
    }

    /// Insert CBGR generation check
    fn insert_cbgr_check(&mut self, block: BlockId, place: Place) {
        self.stats.cbgr_checks_inserted += 1;
        self.push_statement(block, MirStatement::GenerationCheck(place));
    }

    /// Insert CBGR epoch check
    fn insert_epoch_check(&mut self, block: BlockId, place: Place) {
        self.stats.cbgr_checks_inserted += 1;
        self.push_statement(block, MirStatement::EpochCheck(place));
    }

    /// Insert CBGR capability check
    fn insert_capability_check(&mut self, block: BlockId, place: Place, cap: Capability) {
        self.stats.cbgr_checks_inserted += 1;
        self.push_statement(
            block,
            MirStatement::CapabilityCheck {
                place,
                required: cap,
            },
        );
    }

    /// Insert bounds check
    fn insert_bounds_check(&mut self, block: BlockId, array: Place, index: Place) {
        self.stats.bounds_checks_inserted += 1;
        self.push_statement(block, MirStatement::BoundsCheck { array, index });
    }

    /// Emit drops for all deferred cleanups
    fn emit_deferred_cleanups(&mut self, block: BlockId) {
        // Clone the defer stack to avoid borrow conflict with push_statement
        let deferred: Vec<BlockId> = self.defer_stack.iter().rev().copied().collect();
        for cleanup_block in deferred {
            self.push_statement(block, MirStatement::DeferCleanup { cleanup_block });
        }
    }

    /// Resolve field index from type information
    ///
    /// Queries the type of the local variable from type_cache, then looks up
    /// the field name in the struct definition to get its index.
    ///
    /// # Arguments
    /// * `local` - The local variable whose type contains the field
    /// * `field_name` - The name of the field to resolve
    ///
    /// # Returns
    /// The zero-based index of the field in the struct layout, or 0 if not found
    /// (with a diagnostic error logged).
    fn resolve_field_index(&self, local: LocalId, field_name: &str) -> usize {
        // Get the type of the local from cache
        let ty = match self.type_cache.get(&local) {
            Some(ty) => ty,
            None => {
                // Type not in cache - this shouldn't happen in well-formed code
                tracing::warn!(
                    "resolve_field_index: Local {:?} not found in type cache, field: {}",
                    local,
                    field_name
                );
                return 0;
            }
        };

        // Extract struct name from type
        let struct_name = match ty {
            MirType::Named(name) => name.as_str(),
            MirType::Ref { inner, .. } => {
                // Dereference to get inner type
                if let MirType::Named(name) = &**inner {
                    name.as_str()
                } else {
                    tracing::warn!(
                        "resolve_field_index: Cannot access field '{}' on non-struct type: {:?}",
                        field_name,
                        inner
                    );
                    return 0;
                }
            }
            _ => {
                tracing::warn!(
                    "resolve_field_index: Cannot access field '{}' on non-struct type: {:?}",
                    field_name,
                    ty
                );
                return 0;
            }
        };

        // Look up field in type registry
        match self.type_registry.get_field_index(struct_name, field_name) {
            Some(idx) => {
                tracing::debug!(
                    "resolve_field_index: Resolved {}.{} -> index {}",
                    struct_name,
                    field_name,
                    idx
                );
                idx
            }
            None => {
                tracing::warn!(
                    "resolve_field_index: Field '{}' not found in struct '{}'. Available fields: {:?}",
                    field_name,
                    struct_name,
                    self.type_registry.get_struct(struct_name)
                );
                // Return 0 as fallback to prevent crashes, but this indicates an error
                0
            }
        }
    }

    /// Resolve variant discriminant from type information
    ///
    /// Parses a variant path (e.g., `Result::Ok` or `Option::Some`) and looks up
    /// the discriminant value for the variant in the enum definition.
    ///
    /// # Arguments
    /// * `variant_path` - The AST path to the enum variant (e.g., `Result::Ok`)
    ///
    /// # Returns
    /// The discriminant value for the variant (usually the zero-based index), or 0
    /// if not found (with a diagnostic error logged).
    fn resolve_variant_discriminant(&self, variant_path: &verum_ast::ty::Path) -> i64 {
        // Parse the path to extract enum name and variant name
        // Path format: EnumName::VariantName or module::EnumName::VariantName

        if variant_path.segments.is_empty() {
            tracing::warn!("resolve_variant_discriminant: Empty variant path");
            return 0;
        }

        // Last segment is the variant name
        let variant_name = match variant_path.segments.last() {
            Some(verum_ast::ty::PathSegment::Name(ident)) => ident.name.as_str(),
            _ => {
                tracing::warn!(
                    "resolve_variant_discriminant: Invalid variant path segment: {:?}",
                    variant_path.segments.last()
                );
                return 0;
            }
        };

        // Second-to-last segment is typically the enum name
        // (or we try each segment as the enum name)
        let enum_name = if variant_path.segments.len() >= 2 {
            match &variant_path.segments[variant_path.segments.len() - 2] {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                _ => {
                    tracing::warn!(
                        "resolve_variant_discriminant: Invalid enum path segment: {:?}",
                        variant_path.segments[variant_path.segments.len() - 2]
                    );
                    return 0;
                }
            }
        } else {
            // Single-segment path - unusual for variants, but try it
            tracing::warn!(
                "resolve_variant_discriminant: Single-segment variant path: {}",
                variant_name
            );
            variant_name
        };

        // Look up variant discriminant in type registry
        match self
            .type_registry
            .get_variant_discriminant(enum_name, variant_name)
        {
            Some(disc) => {
                tracing::debug!(
                    "resolve_variant_discriminant: Resolved {}::{} -> discriminant {}",
                    enum_name,
                    variant_name,
                    disc
                );
                disc
            }
            None => {
                tracing::warn!(
                    "resolve_variant_discriminant: Variant '{}' not found in enum '{}'. Available variants: {:?}",
                    variant_name,
                    enum_name,
                    self.type_registry.get_enum(enum_name)
                );
                // Return 0 as fallback to prevent crashes, but this indicates an error
                0
            }
        }
    }
}

impl Default for LoweringContext {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Type Lowering
// =============================================================================

impl LoweringContext {
    /// Lower AST type to MIR type
    pub fn lower_type(&mut self, ty: &Type) -> MirType {
        match &ty.kind {
            TypeKind::Unit => MirType::Unit,
            TypeKind::Bool => MirType::Bool,
            TypeKind::Int => MirType::Int,
            TypeKind::Float => MirType::Float,
            TypeKind::Char => MirType::Char,
            TypeKind::Text => MirType::Text,
            TypeKind::Path(path) => {
                let name: Text = path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        verum_ast::ty::PathSegment::SelfValue => "self",
                        verum_ast::ty::PathSegment::Super => "super",
                        verum_ast::ty::PathSegment::Cog => "cog",
                        verum_ast::ty::PathSegment::Relative => ".",
                    })
                    .collect::<Vec<&str>>()
                    .join(".")
                    .into();
                MirType::Named(name)
            }
            TypeKind::Tuple(types) => {
                MirType::Tuple(types.iter().map(|t| self.lower_type(t)).collect())
            }
            TypeKind::Array { element, size } => {
                let elem = Box::new(self.lower_type(element));
                // Evaluate array size at compile time
                let size = size
                    .as_ref()
                    .and_then(|size_expr| {
                        match self.const_evaluator.eval(size_expr) {
                            Ok(ConstValue::UInt(n)) => Some(n as usize),
                            Ok(ConstValue::Int(n)) if n >= 0 => Some(n as usize),
                            Ok(val) => {
                                // Report diagnostic for non-integer size
                                self.diagnostics.push(
                                    DiagnosticBuilder::error()
                                        .message(format!(
                                            "array size must be an integer, found: {}",
                                            val
                                        ))
                                        .build(),
                                );
                                Some(0)
                            }
                            Err(e) => {
                                // Report diagnostic for failed evaluation
                                self.diagnostics.push(
                                    DiagnosticBuilder::error()
                                        .message(format!(
                                            "failed to evaluate array size at compile time: {}",
                                            e
                                        ))
                                        .build(),
                                );
                                Some(0)
                            }
                        }
                    })
                    .unwrap_or(0);
                MirType::Array(elem, size)
            }
            TypeKind::Slice(inner) => MirType::Slice(Box::new(self.lower_type(inner))),
            TypeKind::Reference { mutable, inner } => {
                let inner_ty = self.lower_type(inner);
                let layout = self.select_reference_layout(&inner_ty);
                MirType::Ref {
                    inner: Box::new(inner_ty),
                    mutable: *mutable,
                    layout,
                }
            }
            TypeKind::CheckedReference { mutable, inner } => {
                let inner_ty = self.lower_type(inner);
                // Checked refs use FatRef layout
                MirType::Ref {
                    inner: Box::new(inner_ty),
                    mutable: *mutable,
                    layout: ReferenceLayout::FatRef(MetadataKind::Length),
                }
            }
            TypeKind::UnsafeReference { mutable, inner } => {
                // Unsafe refs are raw pointers
                MirType::Pointer {
                    inner: Box::new(self.lower_type(inner)),
                    mutable: *mutable,
                }
            }
            TypeKind::Pointer { mutable, inner } => MirType::Pointer {
                inner: Box::new(self.lower_type(inner)),
                mutable: *mutable,
            },
            TypeKind::Function {
                params,
                return_type,
                ..
            } => MirType::Function {
                params: params.iter().map(|t| self.lower_type(t)).collect(),
                ret: Box::new(self.lower_type(return_type)),
            },
            TypeKind::Refined { base, .. } => {
                // Refinements are erased at MIR level
                self.lower_type(base)
            }
            TypeKind::Inferred => MirType::Infer,
            TypeKind::DynProtocol { .. } => {
                // Dynamic protocol objects use fat pointers
                MirType::Named("dyn".into())
            }
            _ => MirType::Infer, // Handle other cases
        }
    }
}

// =============================================================================
// Expression Lowering
// =============================================================================

impl LoweringContext {
    /// Lower an expression - main entry point for expression lowering
    ///
    /// Uses iterative approaches for deeply nested structures to prevent stack overflow.
    /// Relies on RUST_MIN_STACK environment variable for stack size requirements.
    fn lower_expr(
        &mut self,
        expr: &Expr,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Check recursion depth as a safety measure
        const MAX_RECURSION_DEPTH: usize = 10000;
        if self.recursion_depth > MAX_RECURSION_DEPTH {
            return Err(DiagnosticBuilder::error()
                .message(format!(
                    "Expression nesting too deep (>{} levels). Consider simplifying the code.",
                    MAX_RECURSION_DEPTH
                ))
                .build());
        }

        self.recursion_depth += 1;
        let result = match &expr.kind {
            // Simple, non-recursive cases - handle directly
            ExprKind::Literal(_) | ExprKind::Path(_) => {
                self.lower_expr_direct(expr, current_block, dest)
            }
            // Complex cases that can recurse deeply - use iterative approach
            ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Tuple(_)
            | ExprKind::Array(_)
            | ExprKind::Block(_)
            | ExprKind::If { .. } => self.lower_expr_iterative(expr, current_block, dest),
            // All other cases use direct lowering (they handle their own recursion carefully)
            _ => self.lower_expr_direct(expr, current_block, dest),
        };
        self.recursion_depth -= 1;
        result
    }

    /// Iterative expression lowering using explicit work stack
    /// Prevents stack overflow for deeply nested expressions
    fn lower_expr_iterative(
        &mut self,
        root_expr: &Expr,
        initial_block: BlockId,
        final_dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Work stack: we process items from the end (LIFO)
        let mut work_stack: Vec<(&Expr, BlockId, Place)> =
            vec![(root_expr, initial_block, final_dest.clone())];
        let mut result_block = initial_block;

        while let Some((expr, current_block, dest)) = work_stack.pop() {
            match &expr.kind {
                ExprKind::Literal(lit) => {
                    let constant = self.lower_literal_to_constant(lit);
                    self.push_statement(
                        current_block,
                        MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(constant))),
                    );
                    result_block = current_block;
                }

                ExprKind::Binary { op, left, right }
                    if !op.is_assignment() && *op != BinOp::And && *op != BinOp::Or =>
                {
                    // Lower left operand
                    let left_temp = self.new_temp(MirType::Infer);
                    let block1 = self.lower_expr(left, current_block, Place::local(left_temp))?;

                    // Lower right operand
                    let right_temp = self.new_temp(MirType::Infer);
                    let block2 = self.lower_expr(right, block1, Place::local(right_temp))?;

                    // Generate binary operation
                    let rvalue = if self.should_check_overflow(op) {
                        Rvalue::CheckedBinary(
                            *op,
                            Operand::Copy(Place::local(left_temp)),
                            Operand::Copy(Place::local(right_temp)),
                        )
                    } else {
                        Rvalue::Binary(
                            *op,
                            Operand::Copy(Place::local(left_temp)),
                            Operand::Copy(Place::local(right_temp)),
                        )
                    };

                    self.push_statement(block2, MirStatement::Assign(dest, rvalue));
                    result_block = block2;
                }

                ExprKind::Unary { op, expr: inner } => {
                    let inner_temp = self.new_temp(MirType::Infer);
                    let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                    match op {
                        UnOp::Ref | UnOp::RefMut => {
                            let borrow_kind = if *op == UnOp::RefMut {
                                BorrowKind::Mutable
                            } else {
                                BorrowKind::Shared
                            };

                            self.insert_cbgr_check(block, Place::local(inner_temp));
                            self.push_statement(
                                block,
                                MirStatement::Retag {
                                    place: Place::local(inner_temp),
                                    kind: RetagKind::Default,
                                },
                            );

                            self.push_statement(
                                block,
                                MirStatement::Assign(
                                    dest,
                                    Rvalue::Ref(borrow_kind, Place::local(inner_temp)),
                                ),
                            );
                        }
                        UnOp::Deref => {
                            self.insert_cbgr_check(block, Place::local(inner_temp));
                            self.insert_epoch_check(block, Place::local(inner_temp));

                            self.push_statement(
                                block,
                                MirStatement::Assign(dest, Rvalue::Deref(Place::local(inner_temp))),
                            );
                        }
                        _ => {
                            self.push_statement(
                                block,
                                MirStatement::Assign(
                                    dest,
                                    Rvalue::Unary(*op, Operand::Copy(Place::local(inner_temp))),
                                ),
                            );
                        }
                    }

                    result_block = block;
                }

                ExprKind::Tuple(exprs) => {
                    let mut temps = List::new();
                    let mut block = current_block;

                    for expr in exprs.iter() {
                        let temp = self.new_temp(MirType::Infer);
                        block = self.lower_expr(expr, block, Place::local(temp))?;
                        temps.push(Operand::Move(Place::local(temp)));
                    }

                    self.push_statement(
                        block,
                        MirStatement::Assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, temps)),
                    );

                    result_block = block;
                }

                ExprKind::Array(ArrayExpr::List(exprs)) => {
                    let mut temps = List::new();
                    let mut block = current_block;

                    for expr in exprs.iter() {
                        let temp = self.new_temp(MirType::Infer);
                        block = self.lower_expr(expr, block, Place::local(temp))?;
                        temps.push(Operand::Move(Place::local(temp)));
                    }

                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Aggregate(AggregateKind::Array(MirType::Infer), temps),
                        ),
                    );

                    result_block = block;
                }

                ExprKind::Block(block_expr) => {
                    result_block = self.lower_block_iterative(block_expr, current_block, dest)?;
                }

                ExprKind::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    // Lower if expression iteratively to handle deeply nested if/else chains
                    result_block = self.lower_if_iterative(
                        condition,
                        then_branch,
                        else_branch,
                        current_block,
                        dest,
                    )?;
                }

                // For all other cases, delegate to direct lowering
                _ => {
                    result_block = self.lower_expr_direct(expr, current_block, dest)?;
                }
            }
        }

        Ok(result_block)
    }

    /// Fully iterative if expression lowering to handle deeply nested control flow
    /// Uses explicit continuation-passing to avoid stack overflow on deeply nested if/else chains
    ///
    /// This function unrolls nested if/else chains into a flat loop, preventing stack overflow
    /// regardless of nesting depth.
    fn lower_if_iterative(
        &mut self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &Block,
        else_branch: &Option<Box<Expr>>,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Continuation stack: (condition, then_branch, else_branch, else_entry_block)
        // We use a stack to handle nested if/else chains without recursion
        struct IfContinuation<'a> {
            condition: &'a verum_ast::expr::IfCondition,
            then_branch: &'a Block,
            else_branch: &'a Option<Box<Expr>>,
            entry_block: BlockId,
        }

        let mut continuations: Vec<IfContinuation<'_>> = vec![IfContinuation {
            condition,
            then_branch,
            else_branch,
            entry_block: current_block,
        }];

        // All branches will eventually jump to this single join block
        let final_join_block = self.new_block();

        // Track then branch exit blocks that need to jump to join
        let mut then_exits: Vec<BlockId> = Vec::new();

        while let Some(cont) = continuations.pop() {
            // Lower condition
            let mut cond_block = cont.entry_block;
            let mut cond_temp = LocalId(0);

            for cond in cont.condition.conditions.iter() {
                match cond {
                    ConditionKind::Expr(expr) => {
                        cond_temp = self.new_temp(MirType::Bool);
                        // Lower condition expression
                        cond_block = self.lower_expr(expr, cond_block, Place::local(cond_temp))?;
                    }
                    ConditionKind::Let { pattern, value } => {
                        let val_temp = self.new_temp(MirType::Infer);
                        cond_block = self.lower_expr(value, cond_block, Place::local(val_temp))?;
                        cond_temp = self.lower_pattern_binding(
                            pattern,
                            Place::local(val_temp),
                            cond_block,
                        )?;
                    }
                }
            }

            // Create then and else blocks
            let then_block = self.new_block();
            let else_block = self.new_block();

            self.set_terminator(
                cond_block,
                Terminator::Branch {
                    condition: Operand::Copy(Place::local(cond_temp)),
                    then_block,
                    else_block,
                },
            );

            // Lower then branch (non-recursive: blocks typically don't contain deeply nested if chains)
            self.push_scope();
            let then_exit =
                self.lower_block_iterative(cont.then_branch, then_block, dest.clone())?;
            self.pop_scope(then_exit);
            then_exits.push(then_exit);

            // Handle else branch
            match cont.else_branch.as_ref() {
                Some(else_expr) => {
                    // Check if else branch is another if expression (if-else-if chain)
                    if let ExprKind::If {
                        condition: nested_cond,
                        then_branch: nested_then,
                        else_branch: nested_else,
                    } = &else_expr.kind
                    {
                        // Push the nested if as a continuation instead of recursing
                        continuations.push(IfContinuation {
                            condition: nested_cond,
                            then_branch: nested_then,
                            else_branch: nested_else,
                            entry_block: else_block,
                        });
                    } else {
                        // Else branch is not an if - lower it directly
                        self.push_scope();
                        let else_exit =
                            self.lower_expr(else_expr.as_ref(), else_block, dest.clone())?;
                        self.pop_scope(else_exit);
                        then_exits.push(else_exit); // Else exit also jumps to join
                    }
                }
                None => {
                    // No else branch: assign unit
                    self.push_statement(
                        else_block,
                        MirStatement::Assign(
                            dest.clone(),
                            Rvalue::Use(Operand::Constant(MirConstant::Unit)),
                        ),
                    );
                    then_exits.push(else_block);
                }
            }
        }

        // Connect all exit blocks to the final join block
        for exit_block in then_exits {
            self.set_terminator(exit_block, Terminator::Goto(final_join_block));
        }

        Ok(final_join_block)
    }

    /// Iterative block lowering using explicit statement queue
    /// Prevents stack overflow for blocks with many statements
    fn lower_block_iterative(
        &mut self,
        block: &Block,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        self.push_scope();

        let mut curr = current_block;

        // Lower each statement iteratively
        for stmt in block.stmts.iter() {
            curr = self.lower_stmt_iterative(stmt, curr)?;
        }

        // Lower trailing expression if present
        if let Some(ref expr) = block.expr.as_ref() {
            curr = self.lower_expr(expr.as_ref(), curr, dest)?;
        } else {
            // No trailing expr: result is unit
            self.push_statement(
                curr,
                MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
            );
        }

        self.pop_scope(curr);

        Ok(curr)
    }

    /// Iterative statement lowering
    /// Processes statements without deep recursion
    fn lower_stmt_iterative(
        &mut self,
        stmt: &Stmt,
        current_block: BlockId,
    ) -> Result<BlockId, Diagnostic> {
        // Statements are processed sequentially, so we don't need a work stack here
        // The main issue is expressions within statements, which we handle via lower_expr
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                let local_ty = match ty.as_ref() {
                    Some(t) => self.lower_type(t),
                    None => MirType::Infer,
                };

                if let PatternKind::Ident {
                    name, mutable: _, ..
                } = &pattern.kind
                {
                    let local = self.new_local(name.name.clone(), local_ty, LocalKind::Var);

                    self.push_statement(current_block, MirStatement::StorageLive(local));

                    self.push_statement(
                        current_block,
                        MirStatement::DebugVar {
                            local,
                            name: name.name.clone().into(),
                        },
                    );

                    if let Some(val_expr) = value {
                        return self.lower_expr(val_expr, current_block, Place::local(local));
                    }
                } else {
                    if let Some(val_expr) = value {
                        let temp = self.new_temp(local_ty);
                        self.push_statement(current_block, MirStatement::StorageLive(temp));
                        let block = self.lower_expr(val_expr, current_block, Place::local(temp))?;
                        let _ = self.lower_pattern_binding(pattern, Place::local(temp), block);
                        return Ok(block);
                    }
                }

                Ok(current_block)
            }

            StmtKind::Expr { expr, has_semi: _ } => {
                let temp = self.new_temp(MirType::Infer);
                self.lower_expr(expr, current_block, Place::local(temp))
            }

            // For other statement kinds, delegate to the original lower_stmt
            _ => self.lower_stmt(stmt, current_block),
        }
    }

    /// Direct (potentially recursive) expression lowering
    /// Only used for cases that don't risk deep recursion
    fn lower_expr_direct(
        &mut self,
        expr: &Expr,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                let constant = self.lower_literal_to_constant(lit);
                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(constant))),
                );
                Ok(current_block)
            }

            ExprKind::Path(path) => {
                // Look up variable
                if let Some(ident) = path.as_ident() {
                    if let Some(local_id) = self.lookup_var(ident.as_str()) {
                        self.push_statement(
                            current_block,
                            MirStatement::Assign(
                                dest,
                                Rvalue::Use(Operand::Copy(Place::local(local_id))),
                            ),
                        );
                        return Ok(current_block);
                    }
                }

                // Might be a function or constant reference
                let name = path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(".");

                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Constant(MirConstant::Function(name.into()))),
                    ),
                );
                Ok(current_block)
            }

            ExprKind::Binary { op, left, right } => {
                // Handle short-circuit operators specially
                if *op == BinOp::And || *op == BinOp::Or {
                    return self.lower_short_circuit(left, right, current_block, dest, *op);
                }

                // Handle assignment operators
                if op.is_assignment() {
                    return self.lower_assignment(op, left, right, current_block, dest);
                }

                // Lower operands
                let left_temp = self.new_temp(MirType::Infer);
                let right_temp = self.new_temp(MirType::Infer);

                let block1 = self.lower_expr(left, current_block, Place::local(left_temp))?;
                let block2 = self.lower_expr(right, block1, Place::local(right_temp))?;

                // Generate binary operation (with checked arithmetic for overflow)
                let rvalue = if self.should_check_overflow(op) {
                    Rvalue::CheckedBinary(
                        *op,
                        Operand::Copy(Place::local(left_temp)),
                        Operand::Copy(Place::local(right_temp)),
                    )
                } else {
                    Rvalue::Binary(
                        *op,
                        Operand::Copy(Place::local(left_temp)),
                        Operand::Copy(Place::local(right_temp)),
                    )
                };

                self.push_statement(block2, MirStatement::Assign(dest, rvalue));

                Ok(block2)
            }

            ExprKind::Unary { op, expr: inner } => {
                match op {
                    UnOp::Ref | UnOp::RefMut => {
                        // Create a reference with CBGR metadata
                        let inner_temp = self.new_temp(MirType::Infer);
                        let block =
                            self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                        let borrow_kind = if *op == UnOp::RefMut {
                            BorrowKind::Mutable
                        } else {
                            BorrowKind::Shared
                        };

                        // Insert CBGR generation check for reference creation
                        self.insert_cbgr_check(block, Place::local(inner_temp));

                        // Retag for borrow tracking
                        self.push_statement(
                            block,
                            MirStatement::Retag {
                                place: Place::local(inner_temp),
                                kind: RetagKind::Default,
                            },
                        );

                        self.push_statement(
                            block,
                            MirStatement::Assign(
                                dest,
                                Rvalue::Ref(borrow_kind, Place::local(inner_temp)),
                            ),
                        );

                        Ok(block)
                    }
                    UnOp::Deref => {
                        // Dereference with CBGR validation
                        let inner_temp = self.new_temp(MirType::Infer);
                        let block =
                            self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                        // Insert CBGR checks before dereference
                        self.insert_cbgr_check(block, Place::local(inner_temp));
                        self.insert_epoch_check(block, Place::local(inner_temp));

                        self.push_statement(
                            block,
                            MirStatement::Assign(dest, Rvalue::Deref(Place::local(inner_temp))),
                        );

                        Ok(block)
                    }
                    _ => {
                        // Other unary ops
                        let inner_temp = self.new_temp(MirType::Infer);
                        let block =
                            self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                        self.push_statement(
                            block,
                            MirStatement::Assign(
                                dest,
                                Rvalue::Unary(*op, Operand::Copy(Place::local(inner_temp))),
                            ),
                        );

                        Ok(block)
                    }
                }
            }

            ExprKind::Call { func, args, .. } => {
                self.lower_call(func.as_ref(), args, current_block, dest)
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.lower_method_call(receiver.as_ref(), method, args, current_block, dest),

            ExprKind::Field { expr: obj, field } => {
                let obj_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(obj, current_block, Place::local(obj_temp))?;

                // Resolve field index from type information
                // Note: This requires integration with type checker to get struct layout
                let field_idx = self.resolve_field_index(obj_temp, field.name.as_str());

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Copy(Place::local(obj_temp).with_field(field_idx))),
                    ),
                );

                Ok(block)
            }

            ExprKind::Index { expr: array, index } => {
                let array_temp = self.new_temp(MirType::Infer);
                let index_temp = self.new_temp(MirType::Int);

                let block1 = self.lower_expr(array, current_block, Place::local(array_temp))?;
                let block2 = self.lower_expr(index, block1, Place::local(index_temp))?;

                // Insert bounds check
                self.insert_bounds_check(
                    block2,
                    Place::local(array_temp),
                    Place::local(index_temp),
                );

                self.push_statement(
                    block2,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Copy(
                            Place::local(array_temp).with_index(index_temp),
                        )),
                    ),
                );

                Ok(block2)
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if(
                condition.as_ref(),
                then_branch,
                else_branch,
                current_block,
                dest,
            ),

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => self.lower_match(scrutinee.as_ref(), arms, current_block, dest),

            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            } => self.lower_loop(body, current_block, dest),

            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => self.lower_while(condition, body, current_block, dest),

            ExprKind::For {
                label: _,
                pattern,
                iter,
                body,
                invariants: _,
                decreases: _,
            } => self.lower_for(pattern, iter, body, current_block, dest),

            ExprKind::ForAwait {
                label: _,
                pattern,
                async_iterable,
                body,
                invariants: _,
                decreases: _,
            } => self.lower_for_await(pattern, async_iterable, body, current_block, dest),

            ExprKind::Block(block) => self.lower_block_iterative(block, current_block, dest),

            ExprKind::Break { label: _, value } => {
                if let Some(loop_ctx) = self.loop_stack.last().copied() {
                    if let Some(val) = value.as_ref() {
                        // Store break value in loop result place
                        if let Some(result_local) = loop_ctx.result {
                            self.lower_expr(
                                val.as_ref(),
                                current_block,
                                Place::local(result_local),
                            )?;
                        } else {
                            self.lower_expr(val.as_ref(), current_block, dest.clone())?;
                        }
                    }
                    // Emit cleanups before break
                    self.emit_deferred_cleanups(current_block);
                    self.set_terminator(current_block, Terminator::Goto(loop_ctx.exit));
                    // Return current block since control flow diverges
                    Ok(current_block)
                } else {
                    Err(DiagnosticBuilder::new(Severity::Error)
                        .message("break outside of loop")
                        .build())
                }
            }

            ExprKind::Continue { label: _ } => {
                if let Some(loop_ctx) = self.loop_stack.last().copied() {
                    // Emit cleanups before continue
                    self.emit_deferred_cleanups(current_block);
                    self.set_terminator(current_block, Terminator::Goto(loop_ctx.continue_target));
                    Ok(current_block)
                } else {
                    Err(DiagnosticBuilder::new(Severity::Error)
                        .message("continue outside of loop")
                        .build())
                }
            }

            ExprKind::Return(value) => {
                if let Some(val) = value.as_ref() {
                    self.lower_expr(val.as_ref(), current_block, Place::return_place())?;
                }
                // Emit all deferred cleanups before return
                let deferred: Vec<BlockId> = self.defer_stack.iter().rev().copied().collect();
                for cleanup_block in deferred {
                    self.push_statement(
                        current_block,
                        MirStatement::DeferCleanup { cleanup_block },
                    );
                }
                // Emit drops for all scope variables
                self.push_statement(current_block, MirStatement::RunDeferredCleanups);
                self.set_terminator(current_block, Terminator::Return);
                Ok(current_block)
            }

            ExprKind::Tuple(exprs) => {
                let mut temps = List::new();
                let mut block = current_block;

                for expr in exprs.iter() {
                    let temp = self.new_temp(MirType::Infer);
                    block = self.lower_expr(expr, block, Place::local(temp))?;
                    temps.push(Operand::Move(Place::local(temp)));
                }

                self.push_statement(
                    block,
                    MirStatement::Assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, temps)),
                );

                Ok(block)
            }

            ExprKind::Array(array_expr) => match array_expr {
                ArrayExpr::List(exprs) => {
                    let mut temps = List::new();
                    let mut block = current_block;

                    for expr in exprs.iter() {
                        let temp = self.new_temp(MirType::Infer);
                        block = self.lower_expr(expr, block, Place::local(temp))?;
                        temps.push(Operand::Move(Place::local(temp)));
                    }

                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Aggregate(AggregateKind::Array(MirType::Infer), temps),
                        ),
                    );

                    Ok(block)
                }
                ArrayExpr::Repeat { value, count } => {
                    // [value; count] syntax - evaluate count at compile time
                    let value_temp = self.new_temp(MirType::Infer);
                    let block1 = self.lower_expr(value, current_block, Place::local(value_temp))?;

                    // Evaluate repeat count as compile-time constant
                    let repeat_count = match self.const_evaluator.eval(count) {
                        Ok(ConstValue::UInt(n)) => n as usize,
                        Ok(ConstValue::Int(n)) if n >= 0 => n as usize,
                        Ok(val) => {
                            self.diagnostics.push(
                                DiagnosticBuilder::error()
                                    .message(format!(
                                        "array repeat count must be an integer, found: {}",
                                        val
                                    ))
                                    .span_label(self.span_to_diag(count.span), "not an integer")
                                    .build(),
                            );
                            0
                        }
                        Err(e) => {
                            self.diagnostics.push(
                                DiagnosticBuilder::error()
                                    .message(format!(
                                        "failed to evaluate array repeat count at compile time: {}",
                                        e
                                    ))
                                    .span_label(self.span_to_diag(count.span), "evaluation failed")
                                    .build(),
                            );
                            0
                        }
                    };

                    // Emit repeat rvalue with evaluated count
                    self.push_statement(
                        block1,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Repeat(Operand::Copy(Place::local(value_temp)), repeat_count),
                        ),
                    );

                    Ok(block1)
                }
            },

            ExprKind::Closure {
                async_: _,
                move_: _,
                params,
                contexts: _,
                return_type: _,
                body,
            } => {
                // Lower closure: capture free variables and create closure aggregate
                let mut captures = List::new();
                self.push_scope();

                // Create locals for parameters
                for param in params.iter() {
                    if let PatternKind::Ident { name, .. } = &param.pattern.kind {
                        let ty = param
                            .ty
                            .as_ref()
                            .map(|t| self.lower_type(t))
                            .unwrap_or(MirType::Infer);
                        let local = self.new_local(name.name.clone(), ty, LocalKind::Arg);
                        captures.push(Place::local(local));
                    }
                }

                // Note: Full closure lowering would create a separate function
                // For now, just emit a closure aggregate
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::Closure("closure".into(), captures),
                            List::new(),
                        ),
                    ),
                );

                self.pop_scope_no_drops();
                let _ = body; // Would be lowered to separate function
                Ok(current_block)
            }

            ExprKind::Async(body) => {
                // Async blocks are lowered to a generator state machine that implements Future.
                //
                // The async block body is wrapped in a generator closure that:
                // 1. Has suspension points at each await
                // 2. Stores local variables in the generator state
                // 3. Returns Poll::Ready(value) on completion
                //
                // State machine structure:
                // - State 0: Initial (before first suspension)
                // - State N: Resumed after await point N
                // - State COMPLETE: Finished (returns Poll::Ready)
                //
                // The generated MIR creates:
                // 1. A state storage local for tracking resume point
                // 2. Switch on state to resume at correct point
                // 3. Yield points that store state and return Poll::Pending

                // Create state tracking local
                let state_local = self.new_local("__async_state", MirType::U32, LocalKind::Var);
                self.push_statement(current_block, MirStatement::StorageLive(state_local));

                // Initialize state to 0 (initial state)
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(state_local),
                        Rvalue::Use(Operand::Constant(MirConstant::UInt(0))),
                    ),
                );

                // Create the generator type for this async block
                let generator_name = format!("__async_generator_{}", self.next_local);
                let generator_ty = MirType::Generator {
                    name: generator_name.clone().into(),
                    state_types: List::new(), // Will be populated by captured variables
                };

                // Create generator aggregate with state
                let generator_temp = self.new_temp(generator_ty.clone());
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(generator_temp),
                        Rvalue::Aggregate(
                            AggregateKind::Generator(generator_name.into()),
                            List::from(vec![Operand::Copy(Place::local(state_local))]),
                        ),
                    ),
                );

                // Lower the body to get the result type
                let body_temp = self.new_temp(MirType::Infer);
                let body_exit =
                    self.lower_block_iterative(body, current_block, Place::local(body_temp))?;

                // Store result in destination wrapped as Poll::Ready
                self.push_statement(
                    body_exit,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::Variant("Poll".into(), 0), // Poll::Ready
                            List::from(vec![Operand::Move(Place::local(body_temp))]),
                        ),
                    ),
                );

                Ok(body_exit)
            }

            ExprKind::Await(inner) => {
                // Await becomes a suspension point
                let inner_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                let resume_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                // Set await terminator
                self.set_terminator(
                    block,
                    Terminator::Await {
                        future: Place::local(inner_temp),
                        destination: dest,
                        resume_block,
                        unwind_block,
                    },
                );

                // Unwind block resumes unwinding
                self.set_terminator(unwind_block, Terminator::Resume);

                Ok(resume_block)
            }

            ExprKind::Try(inner) => {
                // Error propagation: check result and propagate error
                let inner_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                let ok_block = self.new_block();
                let err_block = self.new_block();

                // Check discriminant (Result::Ok = 0, Result::Err = 1)
                let discrim_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(discrim_temp),
                        Rvalue::Discriminant(Place::local(inner_temp)),
                    ),
                );

                self.set_terminator(
                    block,
                    Terminator::SwitchInt {
                        discriminant: Operand::Copy(Place::local(discrim_temp)),
                        targets: List::from(vec![(0, ok_block)]),
                        otherwise: err_block,
                    },
                );

                // OK path: extract value
                self.push_statement(
                    ok_block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Move(
                            Place::local(inner_temp).with_downcast(0).with_field(0), // Ok variant data
                        )),
                    ),
                );

                // Error path: return early with error
                self.push_statement(
                    err_block,
                    MirStatement::Assign(
                        Place::return_place(),
                        Rvalue::Aggregate(
                            AggregateKind::Variant(verum_common::well_known_types::type_names::RESULT.into(), 1), // Err variant
                            List::from(vec![Operand::Move(
                                Place::local(inner_temp).with_downcast(1).with_field(0),
                            )]),
                        ),
                    ),
                );
                // Emit cleanups before early return
                self.emit_deferred_cleanups(err_block);
                self.set_terminator(err_block, Terminator::Return);

                Ok(ok_block)
            }

            ExprKind::TryBlock(inner) => {
                // Plain try block: evaluate inner and wrap in Ok()
                // Try blocks: wrap fallible code, propagate errors via ? operator.
                let inner_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                // Wrap result in Ok variant
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::Variant(verum_common::well_known_types::type_names::RESULT.into(), 0), // Ok variant
                            List::from(vec![Operand::Move(Place::local(inner_temp))]),
                        ),
                    ),
                );

                Ok(block)
            }

            ExprKind::Cast { expr: inner, ty } => {
                let inner_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                let target_ty = self.lower_type(ty);
                let cast_kind = self.determine_cast_kind(&MirType::Infer, &target_ty);

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Cast(
                            cast_kind,
                            Operand::Copy(Place::local(inner_temp)),
                            target_ty,
                        ),
                    ),
                );

                Ok(block)
            }

            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                // Lower range to tuple (start, end, inclusive)
                let mut operands = List::new();

                let mut block = current_block;

                if let Some(start_expr) = start.as_ref() {
                    let temp = self.new_temp(MirType::Int);
                    block = self.lower_expr(start_expr.as_ref(), block, Place::local(temp))?;
                    operands.push(Operand::Move(Place::local(temp)));
                }

                if let Some(end_expr) = end.as_ref() {
                    let temp = self.new_temp(MirType::Int);
                    block = self.lower_expr(end_expr.as_ref(), block, Place::local(temp))?;
                    operands.push(Operand::Move(Place::local(temp)));
                }

                operands.push(Operand::Constant(MirConstant::Bool(*inclusive)));

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(AggregateKind::Struct("Range".into()), operands),
                    ),
                );

                Ok(block)
            }

            ExprKind::Record { path, fields, base } => {
                let mut operands = List::new();
                let mut block = current_block;

                // Lower each field
                for field in fields.iter() {
                    let temp = self.new_temp(MirType::Infer);
                    if let Some(ref value) = field.value.as_ref() {
                        block = self.lower_expr(value, block, Place::local(temp))?;
                    } else {
                        // Shorthand: { x } means { x: x }
                        if let Some(local_id) = self.lookup_var(field.name.as_str()) {
                            self.push_statement(
                                block,
                                MirStatement::Assign(
                                    Place::local(temp),
                                    Rvalue::Use(Operand::Copy(Place::local(local_id))),
                                ),
                            );
                        }
                    }
                    operands.push(Operand::Move(Place::local(temp)));
                }

                // Handle struct update syntax { ..base }
                if let Some(base_expr) = base.as_ref() {
                    let base_temp = self.new_temp(MirType::Infer);
                    block = self.lower_expr(base_expr.as_ref(), block, Place::local(base_temp))?;
                    // In full impl, would copy remaining fields from base
                }

                let struct_name = path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(".");

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(AggregateKind::Struct(struct_name.into()), operands),
                    ),
                );

                Ok(block)
            }

            ExprKind::Paren(inner) => self.lower_expr(inner, current_block, dest),

            ExprKind::Pipeline { left, right } => {
                // x |> f is equivalent to f(x)
                let left_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(left, current_block, Place::local(left_temp))?;

                // Lower right side as function call with left as argument
                let func_temp = self.new_temp(MirType::Infer);
                let block2 = self.lower_expr(right, block, Place::local(func_temp))?;

                let success_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                self.set_terminator(
                    block2,
                    Terminator::Call {
                        destination: dest,
                        func: Operand::Copy(Place::local(func_temp)),
                        args: List::from(vec![Operand::Move(Place::local(left_temp))]),
                        success_block,
                        unwind_block,
                    },
                );

                self.set_terminator(unwind_block, Terminator::Resume);

                Ok(success_block)
            }

            ExprKind::NullCoalesce { left, right } => {
                // a ?? b: if a is Some, use a; otherwise use b
                let left_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(left, current_block, Place::local(left_temp))?;

                let some_block = self.new_block();
                let none_block = self.new_block();
                let join_block = self.new_block();

                // Check discriminant
                let discrim_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(discrim_temp),
                        Rvalue::Discriminant(Place::local(left_temp)),
                    ),
                );

                self.set_terminator(
                    block,
                    Terminator::SwitchInt {
                        discriminant: Operand::Copy(Place::local(discrim_temp)),
                        targets: List::from(vec![(0, some_block)]), // Some = 0
                        otherwise: none_block,
                    },
                );

                // Some path: use left value
                self.push_statement(
                    some_block,
                    MirStatement::Assign(
                        dest.clone(),
                        Rvalue::Use(Operand::Move(
                            Place::local(left_temp).with_downcast(0).with_field(0),
                        )),
                    ),
                );
                self.set_terminator(some_block, Terminator::Goto(join_block));

                // None path: evaluate right
                let none_exit = self.lower_expr(right, none_block, dest)?;
                self.set_terminator(none_exit, Terminator::Goto(join_block));

                Ok(join_block)
            }

            ExprKind::TupleIndex { expr: tuple, index } => {
                // Access tuple element by numeric index: tuple.0, tuple.1, etc.
                let tuple_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(tuple, current_block, Place::local(tuple_temp))?;

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Copy(
                            Place::local(tuple_temp).with_field(*index as usize),
                        )),
                    ),
                );

                Ok(block)
            }

            ExprKind::OptionalChain { expr: obj, field } => {
                // Optional chaining: obj?.field
                // If obj is Some, access field; otherwise propagate None
                let obj_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(obj, current_block, Place::local(obj_temp))?;

                let some_block = self.new_block();
                let none_block = self.new_block();
                let join_block = self.new_block();

                // Check discriminant (Some = 0, None = 1)
                let discrim_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(discrim_temp),
                        Rvalue::Discriminant(Place::local(obj_temp)),
                    ),
                );

                self.set_terminator(
                    block,
                    Terminator::SwitchInt {
                        discriminant: Operand::Copy(Place::local(discrim_temp)),
                        targets: List::from(vec![(0, some_block)]),
                        otherwise: none_block,
                    },
                );

                // Some path: access field and wrap in Some
                let inner_place = Place::local(obj_temp).with_downcast(0).with_field(0);
                let field_idx = self.resolve_field_index(obj_temp, field.name.as_str());
                let field_value = self.new_temp(MirType::Infer);
                self.push_statement(
                    some_block,
                    MirStatement::Assign(
                        Place::local(field_value),
                        Rvalue::Use(Operand::Copy(inner_place.with_field(field_idx))),
                    ),
                );
                self.push_statement(
                    some_block,
                    MirStatement::Assign(
                        dest.clone(),
                        Rvalue::Aggregate(
                            AggregateKind::Variant("Option".into(), 0), // Some
                            List::from(vec![Operand::Move(Place::local(field_value))]),
                        ),
                    ),
                );
                self.set_terminator(some_block, Terminator::Goto(join_block));

                // None path: propagate None
                self.push_statement(
                    none_block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::Variant("Option".into(), 1), // None
                            List::new(),
                        ),
                    ),
                );
                self.set_terminator(none_block, Terminator::Goto(join_block));

                Ok(join_block)
            }

            ExprKind::TryRecover {
                try_block,
                recover,
            } => {
                // try { ... } recover { pattern => expr, ... }
                // Execute try_block, and if it returns Err, match against recover arms
                let try_temp = self.new_temp(MirType::Infer);
                let try_exit = self.lower_expr(try_block, current_block, Place::local(try_temp))?;

                let ok_block = self.new_block();
                let err_block = self.new_block();
                let join_block = self.new_block();

                // Check if try_block returned Ok or Err
                let discrim_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    try_exit,
                    MirStatement::Assign(
                        Place::local(discrim_temp),
                        Rvalue::Discriminant(Place::local(try_temp)),
                    ),
                );

                self.set_terminator(
                    try_exit,
                    Terminator::SwitchInt {
                        discriminant: Operand::Copy(Place::local(discrim_temp)),
                        targets: List::from(vec![(0, ok_block)]), // Ok = 0
                        otherwise: err_block,
                    },
                );

                // Ok path: extract and return the value
                self.push_statement(
                    ok_block,
                    MirStatement::Assign(
                        dest.clone(),
                        Rvalue::Use(Operand::Move(
                            Place::local(try_temp).with_downcast(0).with_field(0),
                        )),
                    ),
                );
                self.set_terminator(ok_block, Terminator::Goto(join_block));

                // Err path: match error against recover body
                let err_value = Place::local(try_temp).with_downcast(1).with_field(0);
                let err_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    err_block,
                    MirStatement::Assign(
                        Place::local(err_temp),
                        Rvalue::Use(Operand::Move(err_value)),
                    ),
                );

                // Lower recover body based on its kind
                let otherwise_block = self.new_block();
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        // Lower recover arms as a match expression
                        let mut arm_blocks: Vec<BlockId> = Vec::new();

                        for _arm in arms.iter() {
                            let arm_block = self.new_block();
                            arm_blocks.push(arm_block);
                        }

                        // Generate switch based on pattern discriminants
                        let mut targets = List::new();
                        for (i, arm) in arms.iter().enumerate() {
                            if let Some(disc) = self.pattern_to_discriminant(&arm.pattern) {
                                targets.push((disc, arm_blocks[i]));
                            }
                        }

                        self.set_terminator(
                            err_block,
                            Terminator::SwitchInt {
                                discriminant: Operand::Copy(Place::local(err_temp)),
                                targets,
                                otherwise: otherwise_block,
                            },
                        );

                        // Lower each arm body
                        for (i, arm) in arms.iter().enumerate() {
                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                &arm.pattern,
                                Place::local(err_temp),
                                arm_blocks[i],
                            )?;
                            let arm_exit = self.lower_expr(&arm.body, arm_blocks[i], dest.clone())?;
                            self.pop_scope(arm_exit);
                            self.set_terminator(arm_exit, Terminator::Goto(join_block));
                        }
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        // Closure syntax: recover |e| expr
                        // Bind the error to the parameter pattern and evaluate body
                        let closure_block = self.new_block();
                        self.set_terminator(err_block, Terminator::Goto(closure_block));

                        self.push_scope();
                        let _ = self.lower_pattern_binding(
                            &param.pattern,
                            Place::local(err_temp),
                            closure_block,
                        )?;
                        let body_exit = self.lower_expr(body, closure_block, dest.clone())?;
                        self.pop_scope(body_exit);
                        self.set_terminator(body_exit, Terminator::Goto(join_block));
                    }
                }

                // Otherwise block: re-raise the error
                self.push_statement(
                    otherwise_block,
                    MirStatement::Assign(
                        Place::return_place(),
                        Rvalue::Aggregate(
                            AggregateKind::Variant("Result".into(), 1),
                            List::from(vec![Operand::Move(Place::local(err_temp))]),
                        ),
                    ),
                );
                self.set_terminator(otherwise_block, Terminator::Return);

                Ok(join_block)
            }

            ExprKind::TryFinally {
                try_block,
                finally_block,
            } => {
                // try { ... } finally { ... }
                // Execute try_block, then always execute finally_block
                let try_temp = self.new_temp(MirType::Infer);
                let try_exit = self.lower_expr(try_block, current_block, Place::local(try_temp))?;

                // Execute finally block
                let finally_temp = self.new_temp(MirType::Unit);
                let finally_exit =
                    self.lower_expr(finally_block, try_exit, Place::local(finally_temp))?;

                // Copy result from try block
                self.push_statement(
                    finally_exit,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(try_temp)))),
                );

                Ok(finally_exit)
            }

            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                // try { ... } recover { ... } finally { ... }
                // Combines TryRecover and TryFinally semantics
                let try_temp = self.new_temp(MirType::Infer);
                let try_exit = self.lower_expr(try_block, current_block, Place::local(try_temp))?;

                let ok_block = self.new_block();
                let err_block = self.new_block();
                let after_recover = self.new_block();

                // Check if try_block returned Ok or Err
                let discrim_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    try_exit,
                    MirStatement::Assign(
                        Place::local(discrim_temp),
                        Rvalue::Discriminant(Place::local(try_temp)),
                    ),
                );

                self.set_terminator(
                    try_exit,
                    Terminator::SwitchInt {
                        discriminant: Operand::Copy(Place::local(discrim_temp)),
                        targets: List::from(vec![(0, ok_block)]),
                        otherwise: err_block,
                    },
                );

                // Ok path
                let result_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    ok_block,
                    MirStatement::Assign(
                        Place::local(result_temp),
                        Rvalue::Use(Operand::Move(
                            Place::local(try_temp).with_downcast(0).with_field(0),
                        )),
                    ),
                );
                self.set_terminator(ok_block, Terminator::Goto(after_recover));

                // Err path: handle recover body
                let err_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    err_block,
                    MirStatement::Assign(
                        Place::local(err_temp),
                        Rvalue::Use(Operand::Move(
                            Place::local(try_temp).with_downcast(1).with_field(0),
                        )),
                    ),
                );

                // Lower recover body based on its kind
                let otherwise_block = self.new_block();
                match recover {
                    RecoverBody::MatchArms { arms, .. } => {
                        // Lower recover arms
                        let mut arm_blocks: Vec<BlockId> = Vec::new();
                        for _ in arms.iter() {
                            arm_blocks.push(self.new_block());
                        }

                        let mut targets = List::new();
                        for (i, arm) in arms.iter().enumerate() {
                            if let Some(disc) = self.pattern_to_discriminant(&arm.pattern) {
                                targets.push((disc, arm_blocks[i]));
                            }
                        }

                        self.set_terminator(
                            err_block,
                            Terminator::SwitchInt {
                                discriminant: Operand::Copy(Place::local(err_temp)),
                                targets,
                                otherwise: otherwise_block,
                            },
                        );

                        for (i, arm) in arms.iter().enumerate() {
                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                &arm.pattern,
                                Place::local(err_temp),
                                arm_blocks[i],
                            )?;
                            let arm_exit =
                                self.lower_expr(&arm.body, arm_blocks[i], Place::local(result_temp))?;
                            self.pop_scope(arm_exit);
                            self.set_terminator(arm_exit, Terminator::Goto(after_recover));
                        }
                    }
                    RecoverBody::Closure { param, body, .. } => {
                        // Closure syntax: recover |e| expr
                        let closure_block = self.new_block();
                        self.set_terminator(err_block, Terminator::Goto(closure_block));

                        self.push_scope();
                        let _ = self.lower_pattern_binding(
                            &param.pattern,
                            Place::local(err_temp),
                            closure_block,
                        )?;
                        let body_exit = self.lower_expr(body, closure_block, Place::local(result_temp))?;
                        self.pop_scope(body_exit);
                        self.set_terminator(body_exit, Terminator::Goto(after_recover));
                    }
                }

                // Otherwise: re-raise
                self.push_statement(
                    otherwise_block,
                    MirStatement::Assign(
                        Place::return_place(),
                        Rvalue::Aggregate(
                            AggregateKind::Variant("Result".into(), 1),
                            List::from(vec![Operand::Move(Place::local(err_temp))]),
                        ),
                    ),
                );
                self.set_terminator(otherwise_block, Terminator::Return);

                // Finally block always executes
                let finally_temp = self.new_temp(MirType::Unit);
                let finally_exit =
                    self.lower_expr(finally_block, after_recover, Place::local(finally_temp))?;

                // Move result to destination
                self.push_statement(
                    finally_exit,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Move(Place::local(result_temp))),
                    ),
                );

                Ok(finally_exit)
            }

            ExprKind::Comprehension { expr, clauses } => {
                // List comprehension: [x * 2 for x in list if x > 0]
                // Lower to: create list, iterate, filter, map, collect
                let list_temp = self.new_temp(MirType::Named(verum_common::well_known_types::type_names::LIST.into()));
                self.push_statement(current_block, MirStatement::StorageLive(list_temp));

                // Initialize empty list
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(list_temp),
                        Rvalue::Aggregate(AggregateKind::Array(MirType::Infer), List::new()),
                    ),
                );

                let mut block = current_block;

                // Process each clause
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { pattern, iter } => {
                            // Create iterator
                            let iter_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(iter, block, Place::local(iter_temp))?;

                            // Create loop
                            let header = self.new_block();
                            let body = self.new_block();
                            let exit = self.new_block();

                            self.set_terminator(block, Terminator::Goto(header));

                            // Iterator next call
                            let next_temp = self.new_temp(MirType::Infer);
                            let success = self.new_block();
                            let unwind = self.new_cleanup_block();

                            self.set_terminator(
                                header,
                                Terminator::Call {
                                    destination: Place::local(next_temp),
                                    func: Operand::Constant(MirConstant::Function(
                                        "Iterator::next".into(),
                                    )),
                                    args: List::from(vec![Operand::Copy(Place::local(iter_temp))]),
                                    success_block: success,
                                    unwind_block: unwind,
                                },
                            );
                            self.set_terminator(unwind, Terminator::Resume);

                            // Check if Some or None
                            let disc_temp = self.new_temp(MirType::Int);
                            self.push_statement(
                                success,
                                MirStatement::Assign(
                                    Place::local(disc_temp),
                                    Rvalue::Discriminant(Place::local(next_temp)),
                                ),
                            );

                            self.set_terminator(
                                success,
                                Terminator::SwitchInt {
                                    discriminant: Operand::Copy(Place::local(disc_temp)),
                                    targets: List::from(vec![(0, body)]),
                                    otherwise: exit,
                                },
                            );

                            // Bind pattern in body
                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                pattern,
                                Place::local(next_temp).with_downcast(0).with_field(0),
                                body,
                            )?;

                            block = body;
                            self.loop_stack.push(LoopContext {
                                header,
                                exit,
                                continue_target: header,
                                result: None,
                            });
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                            // Filter clause
                            let cond_temp = self.new_temp(MirType::Bool);
                            block = self.lower_expr(cond, block, Place::local(cond_temp))?;

                            let then_block = self.new_block();
                            let skip_block = self.new_block();

                            self.set_terminator(
                                block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(cond_temp)),
                                    then_block,
                                    else_block: skip_block,
                                },
                            );

                            // Skip block goes back to header
                            if let Some(ctx) = self.loop_stack.last() {
                                self.set_terminator(skip_block, Terminator::Goto(ctx.header));
                            }

                            block = then_block;
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let {
                            pattern, value, ..
                        } => {
                            // Let binding
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(value, block, Place::local(temp))?;
                            let _ =
                                self.lower_pattern_binding(pattern, Place::local(temp), block)?;
                        }
                    }
                }

                // Evaluate expression and push to list
                let elem_temp = self.new_temp(MirType::Infer);
                let eval_exit = self.lower_expr(expr, block, Place::local(elem_temp))?;

                // Call list.push(elem)
                let push_result = self.new_temp(MirType::Unit);
                let push_success = self.new_block();
                let push_unwind = self.new_cleanup_block();

                self.set_terminator(
                    eval_exit,
                    Terminator::Call {
                        destination: Place::local(push_result),
                        func: Operand::Constant(MirConstant::Function("List::push".into())),
                        args: List::from(vec![
                            Operand::Copy(Place::local(list_temp)),
                            Operand::Move(Place::local(elem_temp)),
                        ]),
                        success_block: push_success,
                        unwind_block: push_unwind,
                    },
                );
                self.set_terminator(push_unwind, Terminator::Resume);

                // Continue to header
                if let Some(ctx) = self.loop_stack.last() {
                    self.set_terminator(push_success, Terminator::Goto(ctx.header));
                }

                // Pop scopes and loop contexts
                while !self.loop_stack.is_empty() {
                    self.pop_scope_no_drops();
                    self.loop_stack.pop();
                }

                // Get exit block from first loop
                let exit_block = self.new_block();
                self.push_statement(
                    exit_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(list_temp)))),
                );

                Ok(exit_block)
            }

            ExprKind::StreamComprehension { expr, clauses } => {
                // Stream comprehension: [| x * 2 for x in iter if x > 0 |]
                // Lower to: create generator that yields values lazily
                //
                // Unlike list comprehension which collects all values eagerly,
                // stream comprehension creates a generator that yields values on demand.
                // This enables lazy evaluation and infinite streams.
                //
                // The generator structure:
                // 1. State machine tracking current position in iteration
                // 2. Yield points for each element produced
                // 3. Completion when iteration exhausts

                let stream_temp = self.new_temp(MirType::Named("Stream".into()));
                self.push_statement(current_block, MirStatement::StorageLive(stream_temp));

                // Create generator for stream with captured environment
                let generator_name: Text = format!("__stream_generator_{}", self.next_local).into();

                // Collect captured variables from clauses for generator closure
                let mut captured_operands = List::new();

                // Initialize generator aggregate with captured state
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(stream_temp),
                        Rvalue::Aggregate(
                            AggregateKind::Generator(generator_name),
                            captured_operands.clone(),
                        ),
                    ),
                );

                let mut block = current_block;

                // Track exit blocks for proper cleanup
                let mut exit_blocks: List<BlockId> = List::new();

                // Process each clause with full pattern matching support
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { pattern, iter } => {
                            // Create iterator from source expression
                            let iter_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(iter, block, Place::local(iter_temp))?;

                            // Add iterator to captured operands for generator state
                            captured_operands.push(Operand::Copy(Place::local(iter_temp)));

                            // Create loop structure: header -> body -> yield -> resume -> header
                            let header = self.new_block();
                            let body = self.new_block();
                            let exit = self.new_block();

                            self.set_terminator(block, Terminator::Goto(header));

                            // Iterator next call in header
                            let next_temp = self.new_temp(MirType::Infer);
                            let success = self.new_block();
                            let unwind = self.new_cleanup_block();

                            self.set_terminator(
                                header,
                                Terminator::Call {
                                    destination: Place::local(next_temp),
                                    func: Operand::Constant(MirConstant::Function(
                                        "Iterator::next".into(),
                                    )),
                                    args: List::from(vec![Operand::Copy(Place::local(iter_temp))]),
                                    success_block: success,
                                    unwind_block: unwind,
                                },
                            );
                            self.set_terminator(unwind, Terminator::Resume);

                            // Check discriminant: Some(value) -> body, None -> exit
                            let disc_temp = self.new_temp(MirType::Int);
                            self.push_statement(
                                success,
                                MirStatement::Assign(
                                    Place::local(disc_temp),
                                    Rvalue::Discriminant(Place::local(next_temp)),
                                ),
                            );

                            self.set_terminator(
                                success,
                                Terminator::SwitchInt {
                                    discriminant: Operand::Copy(Place::local(disc_temp)),
                                    // 0 = Some variant
                                    targets: List::from(vec![(0, body)]),
                                    otherwise: exit,
                                },
                            );

                            // Bind pattern in body block
                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                pattern,
                                Place::local(next_temp).with_downcast(0).with_field(0),
                                body,
                            )?;

                            block = body;
                            exit_blocks.push(exit);
                            self.loop_stack.push(LoopContext {
                                header,
                                exit,
                                continue_target: header,
                                result: None,
                            });
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                            // Filter clause: if condition is false, continue to next iteration
                            let cond_temp = self.new_temp(MirType::Bool);
                            block = self.lower_expr(cond, block, Place::local(cond_temp))?;

                            let then_block = self.new_block();
                            let skip_block = self.new_block();

                            self.set_terminator(
                                block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(cond_temp)),
                                    then_block,
                                    else_block: skip_block,
                                },
                            );

                            // Skip block continues to loop header (skip this element)
                            if let Some(ctx) = self.loop_stack.last() {
                                self.set_terminator(skip_block, Terminator::Goto(ctx.header));
                            }

                            block = then_block;
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let {
                            pattern, value, ..
                        } => {
                            // Let binding in comprehension scope
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(value, block, Place::local(temp))?;
                            let _ =
                                self.lower_pattern_binding(pattern, Place::local(temp), block)?;
                        }
                    }
                }

                // Evaluate yield expression
                let elem_temp = self.new_temp(MirType::Infer);
                let eval_exit = self.lower_expr(expr, block, Place::local(elem_temp))?;

                // Yield the computed element value
                // After yield, control returns here when consumer requests next value
                let resume_block = self.new_block();
                let drop_block = self.new_cleanup_block();

                self.set_terminator(
                    eval_exit,
                    Terminator::Yield {
                        value: Operand::Move(Place::local(elem_temp)),
                        resume: resume_block,
                        drop: drop_block,
                    },
                );

                self.set_terminator(drop_block, Terminator::Resume);

                // After yield resumes, continue to loop header for next iteration
                if let Some(ctx) = self.loop_stack.last() {
                    self.set_terminator(resume_block, Terminator::Goto(ctx.header));
                }

                // Pop all loop scopes created by For clauses
                while !self.loop_stack.is_empty() {
                    self.pop_scope_no_drops();
                    self.loop_stack.pop();
                }

                // Create final exit block that chains all loop exits
                let final_exit = self.new_block();

                // Connect all exit blocks to final exit
                for exit_block in exit_blocks.iter() {
                    self.set_terminator(*exit_block, Terminator::Goto(final_exit));
                }

                // Move stream to destination
                self.push_statement(
                    final_exit,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Move(Place::local(stream_temp))),
                    ),
                );

                Ok(final_exit)
            }

            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                // Map comprehension: {k: v for (k, v) in pairs if condition}
                // Lower to: create Map, iterate source, insert key-value pairs
                let map_temp = self.new_temp(MirType::Named(verum_common::well_known_types::type_names::MAP.into()));
                self.push_statement(current_block, MirStatement::StorageLive(map_temp));

                // Initialize empty map
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(map_temp),
                        Rvalue::Aggregate(AggregateKind::Map, List::new()),
                    ),
                );

                let mut block = current_block;

                // Track exit blocks for proper cleanup
                let mut exit_blocks: List<BlockId> = List::new();

                // Process each clause with full pattern matching support
                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { pattern, iter } => {
                            // Create iterator from source expression
                            let iter_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(iter, block, Place::local(iter_temp))?;

                            // Create loop structure: header -> body -> header -> exit
                            let header = self.new_block();
                            let body = self.new_block();
                            let exit = self.new_block();

                            self.set_terminator(block, Terminator::Goto(header));

                            // Iterator next call in header
                            let next_temp = self.new_temp(MirType::Infer);
                            let success = self.new_block();
                            let unwind = self.new_cleanup_block();

                            self.set_terminator(
                                header,
                                Terminator::Call {
                                    destination: Place::local(next_temp),
                                    func: Operand::Constant(MirConstant::Function(
                                        "Iterator::next".into(),
                                    )),
                                    args: List::from(vec![Operand::Copy(Place::local(iter_temp))]),
                                    success_block: success,
                                    unwind_block: unwind,
                                },
                            );
                            self.set_terminator(unwind, Terminator::Resume);

                            // Check discriminant: Some(value) -> body, None -> exit
                            let disc_temp = self.new_temp(MirType::Int);
                            self.push_statement(
                                success,
                                MirStatement::Assign(
                                    Place::local(disc_temp),
                                    Rvalue::Discriminant(Place::local(next_temp)),
                                ),
                            );

                            self.set_terminator(
                                success,
                                Terminator::SwitchInt {
                                    discriminant: Operand::Copy(Place::local(disc_temp)),
                                    targets: List::from(vec![(0, body)]),
                                    otherwise: exit,
                                },
                            );

                            // Bind pattern in body block
                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                pattern,
                                Place::local(next_temp).with_downcast(0).with_field(0),
                                body,
                            )?;

                            block = body;
                            exit_blocks.push(exit);
                            self.loop_stack.push(LoopContext {
                                header,
                                exit,
                                continue_target: header,
                                result: None,
                            });
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                            let cond_temp = self.new_temp(MirType::Bool);
                            block = self.lower_expr(cond, block, Place::local(cond_temp))?;

                            let then_block = self.new_block();
                            let skip_block = self.new_block();

                            self.set_terminator(
                                block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(cond_temp)),
                                    then_block,
                                    else_block: skip_block,
                                },
                            );

                            if let Some(ctx) = self.loop_stack.last() {
                                self.set_terminator(skip_block, Terminator::Goto(ctx.header));
                            }

                            block = then_block;
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let {
                            pattern, value, ..
                        } => {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(value, block, Place::local(temp))?;
                            let _ =
                                self.lower_pattern_binding(pattern, Place::local(temp), block)?;
                        }
                    }
                }

                // Evaluate key and value expressions
                let key_temp = self.new_temp(MirType::Infer);
                let key_exit = self.lower_expr(key_expr, block, Place::local(key_temp))?;

                let value_temp = self.new_temp(MirType::Infer);
                let value_exit = self.lower_expr(value_expr, key_exit, Place::local(value_temp))?;

                // Insert key-value pair into map
                let insert_success = self.new_block();
                let insert_unwind = self.new_cleanup_block();
                let insert_dest = self.new_temp(MirType::Unit);

                self.set_terminator(
                    value_exit,
                    Terminator::Call {
                        destination: Place::local(insert_dest),
                        func: Operand::Constant(MirConstant::Function("Map::insert".into())),
                        args: List::from(vec![
                            Operand::Copy(Place::local(map_temp)),
                            Operand::Move(Place::local(key_temp)),
                            Operand::Move(Place::local(value_temp)),
                        ]),
                        success_block: insert_success,
                        unwind_block: insert_unwind,
                    },
                );
                self.set_terminator(insert_unwind, Terminator::Resume);

                // Continue to loop header for next iteration
                if let Some(ctx) = self.loop_stack.last() {
                    self.set_terminator(insert_success, Terminator::Goto(ctx.header));
                }

                // Pop all loop scopes
                while !self.loop_stack.is_empty() {
                    self.pop_scope_no_drops();
                    self.loop_stack.pop();
                }

                // Create final exit block
                let final_exit = self.new_block();

                for exit_block in exit_blocks.iter() {
                    self.set_terminator(*exit_block, Terminator::Goto(final_exit));
                }

                // Move map to destination
                self.push_statement(
                    final_exit,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(map_temp)))),
                );

                Ok(final_exit)
            }

            ExprKind::SetComprehension { expr, clauses } => {
                // Set comprehension: set{x for x in items if condition}
                // Lower to: create Set, iterate source, insert elements
                let set_temp = self.new_temp(MirType::Named(verum_common::well_known_types::type_names::SET.into()));
                self.push_statement(current_block, MirStatement::StorageLive(set_temp));

                // Initialize empty set
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(set_temp),
                        Rvalue::Aggregate(AggregateKind::Set, List::new()),
                    ),
                );

                let mut block = current_block;
                let mut exit_blocks: List<BlockId> = List::new();

                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { pattern, iter } => {
                            let iter_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(iter, block, Place::local(iter_temp))?;

                            let header = self.new_block();
                            let body = self.new_block();
                            let exit = self.new_block();

                            self.set_terminator(block, Terminator::Goto(header));

                            let next_temp = self.new_temp(MirType::Infer);
                            let success = self.new_block();
                            let unwind = self.new_cleanup_block();

                            self.set_terminator(
                                header,
                                Terminator::Call {
                                    destination: Place::local(next_temp),
                                    func: Operand::Constant(MirConstant::Function(
                                        "Iterator::next".into(),
                                    )),
                                    args: List::from(vec![Operand::Copy(Place::local(iter_temp))]),
                                    success_block: success,
                                    unwind_block: unwind,
                                },
                            );
                            self.set_terminator(unwind, Terminator::Resume);

                            let disc_temp = self.new_temp(MirType::Int);
                            self.push_statement(
                                success,
                                MirStatement::Assign(
                                    Place::local(disc_temp),
                                    Rvalue::Discriminant(Place::local(next_temp)),
                                ),
                            );

                            self.set_terminator(
                                success,
                                Terminator::SwitchInt {
                                    discriminant: Operand::Copy(Place::local(disc_temp)),
                                    targets: List::from(vec![(0, body)]),
                                    otherwise: exit,
                                },
                            );

                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                pattern,
                                Place::local(next_temp).with_downcast(0).with_field(0),
                                body,
                            )?;

                            block = body;
                            exit_blocks.push(exit);
                            self.loop_stack.push(LoopContext {
                                header,
                                exit,
                                continue_target: header,
                                result: None,
                            });
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                            let cond_temp = self.new_temp(MirType::Bool);
                            block = self.lower_expr(cond, block, Place::local(cond_temp))?;

                            let then_block = self.new_block();
                            let skip_block = self.new_block();

                            self.set_terminator(
                                block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(cond_temp)),
                                    then_block,
                                    else_block: skip_block,
                                },
                            );

                            if let Some(ctx) = self.loop_stack.last() {
                                self.set_terminator(skip_block, Terminator::Goto(ctx.header));
                            }

                            block = then_block;
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let {
                            pattern, value, ..
                        } => {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(value, block, Place::local(temp))?;
                            let _ =
                                self.lower_pattern_binding(pattern, Place::local(temp), block)?;
                        }
                    }
                }

                // Evaluate element expression
                let elem_temp = self.new_temp(MirType::Infer);
                let eval_exit = self.lower_expr(expr, block, Place::local(elem_temp))?;

                // Insert element into set
                let insert_success = self.new_block();
                let insert_unwind = self.new_cleanup_block();
                let insert_dest = self.new_temp(MirType::Unit);

                self.set_terminator(
                    eval_exit,
                    Terminator::Call {
                        destination: Place::local(insert_dest),
                        func: Operand::Constant(MirConstant::Function("Set::insert".into())),
                        args: List::from(vec![
                            Operand::Copy(Place::local(set_temp)),
                            Operand::Move(Place::local(elem_temp)),
                        ]),
                        success_block: insert_success,
                        unwind_block: insert_unwind,
                    },
                );
                self.set_terminator(insert_unwind, Terminator::Resume);

                if let Some(ctx) = self.loop_stack.last() {
                    self.set_terminator(insert_success, Terminator::Goto(ctx.header));
                }

                while !self.loop_stack.is_empty() {
                    self.pop_scope_no_drops();
                    self.loop_stack.pop();
                }

                let final_exit = self.new_block();

                for exit_block in exit_blocks.iter() {
                    self.set_terminator(*exit_block, Terminator::Goto(final_exit));
                }

                self.push_statement(
                    final_exit,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(set_temp)))),
                );

                Ok(final_exit)
            }

            ExprKind::GeneratorComprehension { expr, clauses } => {
                // Generator expression: gen{x for x in items if condition}
                // Lower to: create generator that yields values lazily (same as stream but Generator type)
                let gen_temp = self.new_temp(MirType::Named("Generator".into()));
                self.push_statement(current_block, MirStatement::StorageLive(gen_temp));

                let generator_name: Text = format!("__generator_{}", self.next_local).into();
                let mut captured_operands = List::new();

                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(gen_temp),
                        Rvalue::Aggregate(
                            AggregateKind::Generator(generator_name),
                            captured_operands.clone(),
                        ),
                    ),
                );

                let mut block = current_block;
                let mut exit_blocks: List<BlockId> = List::new();

                for clause in clauses.iter() {
                    match &clause.kind {
                        verum_ast::expr::ComprehensionClauseKind::For { pattern, iter } => {
                            let iter_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(iter, block, Place::local(iter_temp))?;

                            captured_operands.push(Operand::Copy(Place::local(iter_temp)));

                            let header = self.new_block();
                            let body = self.new_block();
                            let exit = self.new_block();

                            self.set_terminator(block, Terminator::Goto(header));

                            let next_temp = self.new_temp(MirType::Infer);
                            let success = self.new_block();
                            let unwind = self.new_cleanup_block();

                            self.set_terminator(
                                header,
                                Terminator::Call {
                                    destination: Place::local(next_temp),
                                    func: Operand::Constant(MirConstant::Function(
                                        "Iterator::next".into(),
                                    )),
                                    args: List::from(vec![Operand::Copy(Place::local(iter_temp))]),
                                    success_block: success,
                                    unwind_block: unwind,
                                },
                            );
                            self.set_terminator(unwind, Terminator::Resume);

                            let disc_temp = self.new_temp(MirType::Int);
                            self.push_statement(
                                success,
                                MirStatement::Assign(
                                    Place::local(disc_temp),
                                    Rvalue::Discriminant(Place::local(next_temp)),
                                ),
                            );

                            self.set_terminator(
                                success,
                                Terminator::SwitchInt {
                                    discriminant: Operand::Copy(Place::local(disc_temp)),
                                    targets: List::from(vec![(0, body)]),
                                    otherwise: exit,
                                },
                            );

                            self.push_scope();
                            let _ = self.lower_pattern_binding(
                                pattern,
                                Place::local(next_temp).with_downcast(0).with_field(0),
                                body,
                            )?;

                            block = body;
                            exit_blocks.push(exit);
                            self.loop_stack.push(LoopContext {
                                header,
                                exit,
                                continue_target: header,
                                result: None,
                            });
                        }
                        verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                            let cond_temp = self.new_temp(MirType::Bool);
                            block = self.lower_expr(cond, block, Place::local(cond_temp))?;

                            let then_block = self.new_block();
                            let skip_block = self.new_block();

                            self.set_terminator(
                                block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(cond_temp)),
                                    then_block,
                                    else_block: skip_block,
                                },
                            );

                            if let Some(ctx) = self.loop_stack.last() {
                                self.set_terminator(skip_block, Terminator::Goto(ctx.header));
                            }

                            block = then_block;
                        }
                        verum_ast::expr::ComprehensionClauseKind::Let {
                            pattern, value, ..
                        } => {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(value, block, Place::local(temp))?;
                            let _ =
                                self.lower_pattern_binding(pattern, Place::local(temp), block)?;
                        }
                    }
                }

                // Evaluate yield expression
                let elem_temp = self.new_temp(MirType::Infer);
                let eval_exit = self.lower_expr(expr, block, Place::local(elem_temp))?;

                // Yield the computed element value
                let resume_block = self.new_block();
                let drop_block = self.new_cleanup_block();

                self.set_terminator(
                    eval_exit,
                    Terminator::Yield {
                        value: Operand::Move(Place::local(elem_temp)),
                        resume: resume_block,
                        drop: drop_block,
                    },
                );

                self.set_terminator(drop_block, Terminator::Resume);

                if let Some(ctx) = self.loop_stack.last() {
                    self.set_terminator(resume_block, Terminator::Goto(ctx.header));
                }

                while !self.loop_stack.is_empty() {
                    self.pop_scope_no_drops();
                    self.loop_stack.pop();
                }

                let final_exit = self.new_block();

                for exit_block in exit_blocks.iter() {
                    self.set_terminator(*exit_block, Terminator::Goto(final_exit));
                }

                self.push_statement(
                    final_exit,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(gen_temp)))),
                );

                Ok(final_exit)
            }

            ExprKind::InterpolatedString {
                handler,
                parts,
                exprs,
            } => {
                // f"Hello {name}" or sql"SELECT * FROM {table}"
                // Lower to format function call
                let mut operands = List::new();
                let mut block = current_block;

                // Add string parts as constants
                for part in parts.iter() {
                    operands.push(Operand::Constant(MirConstant::String(part.clone().into())));
                }

                // Lower expression values
                for expr in exprs.iter() {
                    let temp = self.new_temp(MirType::Infer);
                    block = self.lower_expr(expr, block, Place::local(temp))?;
                    operands.push(Operand::Move(Place::local(temp)));
                }

                // Call handler function
                let handler_func = format!("{}::format", handler);
                let success_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                self.set_terminator(
                    block,
                    Terminator::Call {
                        destination: dest,
                        func: Operand::Constant(MirConstant::Function(handler_func.into())),
                        args: operands,
                        success_block,
                        unwind_block,
                    },
                );

                self.set_terminator(unwind_block, Terminator::Resume);

                Ok(success_block)
            }

            ExprKind::TensorLiteral {
                shape,
                elem_type,
                data,
            } => {
                // tensor<2, 3> Int { [[1, 2, 3], [4, 5, 6]] }
                let data_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(data, current_block, Place::local(data_temp))?;

                // Create tensor type with shape info
                let _tensor_ty = MirType::Named(
                    format!(
                        "Tensor<{}>",
                        shape
                            .iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .into(),
                );

                let elem_mir_ty = self.lower_type(elem_type);
                let _ = elem_mir_ty; // Used for validation

                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Aggregate(
                            AggregateKind::Struct("Tensor".into()),
                            List::from(vec![Operand::Move(Place::local(data_temp))]),
                        ),
                    ),
                );

                Ok(block)
            }

            ExprKind::MapLiteral { entries } => {
                // { "key": value, "key2": value2 }
                let map_temp = self.new_temp(MirType::Named(verum_common::well_known_types::type_names::MAP.into()));
                self.push_statement(current_block, MirStatement::StorageLive(map_temp));

                // Initialize empty map
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(map_temp),
                        Rvalue::Aggregate(AggregateKind::Struct(verum_common::well_known_types::type_names::MAP.into()), List::new()),
                    ),
                );

                let mut block = current_block;

                // Insert each entry
                for (key, value) in entries.iter() {
                    let key_temp = self.new_temp(MirType::Infer);
                    let val_temp = self.new_temp(MirType::Infer);

                    block = self.lower_expr(key, block, Place::local(key_temp))?;
                    block = self.lower_expr(value, block, Place::local(val_temp))?;

                    let insert_result = self.new_temp(MirType::Unit);
                    let success = self.new_block();
                    let unwind = self.new_cleanup_block();

                    self.set_terminator(
                        block,
                        Terminator::Call {
                            destination: Place::local(insert_result),
                            func: Operand::Constant(MirConstant::Function("Map::insert".into())),
                            args: List::from(vec![
                                Operand::Copy(Place::local(map_temp)),
                                Operand::Move(Place::local(key_temp)),
                                Operand::Move(Place::local(val_temp)),
                            ]),
                            success_block: success,
                            unwind_block: unwind,
                        },
                    );
                    self.set_terminator(unwind, Terminator::Resume);
                    block = success;
                }

                self.push_statement(
                    block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(map_temp)))),
                );

                Ok(block)
            }

            ExprKind::SetLiteral { elements } => {
                // { 1, 2, 3 }
                let set_temp = self.new_temp(MirType::Named(verum_common::well_known_types::type_names::SET.into()));
                self.push_statement(current_block, MirStatement::StorageLive(set_temp));

                // Initialize empty set
                self.push_statement(
                    current_block,
                    MirStatement::Assign(
                        Place::local(set_temp),
                        Rvalue::Aggregate(AggregateKind::Struct(verum_common::well_known_types::type_names::SET.into()), List::new()),
                    ),
                );

                let mut block = current_block;

                // Insert each element
                for elem in elements.iter() {
                    let elem_temp = self.new_temp(MirType::Infer);
                    block = self.lower_expr(elem, block, Place::local(elem_temp))?;

                    let insert_result = self.new_temp(MirType::Unit);
                    let success = self.new_block();
                    let unwind = self.new_cleanup_block();

                    self.set_terminator(
                        block,
                        Terminator::Call {
                            destination: Place::local(insert_result),
                            func: Operand::Constant(MirConstant::Function("Set::insert".into())),
                            args: List::from(vec![
                                Operand::Copy(Place::local(set_temp)),
                                Operand::Move(Place::local(elem_temp)),
                            ]),
                            success_block: success,
                            unwind_block: unwind,
                        },
                    );
                    self.set_terminator(unwind, Terminator::Resume);
                    block = success;
                }

                self.push_statement(
                    block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(set_temp)))),
                );

                Ok(block)
            }

            ExprKind::Yield(value) => {
                // yield value: suspend generator and return value
                let value_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(value, current_block, Place::local(value_temp))?;

                let resume_block = self.new_block();
                let drop_block = self.new_cleanup_block();

                self.set_terminator(
                    block,
                    Terminator::Yield {
                        value: Operand::Move(Place::local(value_temp)),
                        resume: resume_block,
                        drop: drop_block,
                    },
                );

                self.set_terminator(drop_block, Terminator::Resume);

                // Result of yield expression is the value passed in on resume
                self.push_statement(
                    resume_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
                );

                Ok(resume_block)
            }

            ExprKind::Inject { .. } => {
                // inject TypeName — Level 1 static DI
                // MIR lowering: treat as unit constant (actual resolution at VBC level)
                Ok(current_block)
            }

            ExprKind::Spawn { expr, contexts: _ } => {
                // spawn { compute_value() }
                // Creates a new task that runs concurrently
                let task_temp = self.new_temp(MirType::Named("TaskHandle".into()));

                // Lower the spawned expression as a closure
                let closure_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(expr, current_block, Place::local(closure_temp))?;

                // Call spawn function
                let success_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                self.set_terminator(
                    block,
                    Terminator::Call {
                        destination: Place::local(task_temp),
                        func: Operand::Constant(MirConstant::Function("runtime::spawn".into())),
                        args: List::from(vec![Operand::Move(Place::local(closure_temp))]),
                        success_block,
                        unwind_block,
                    },
                );

                self.set_terminator(unwind_block, Terminator::Resume);

                self.push_statement(
                    success_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(task_temp)))),
                );

                Ok(success_block)
            }

            ExprKind::Unsafe(body) => {
                // unsafe { ... }
                // Lower body with reduced safety checks
                self.lower_block_iterative(body, current_block, dest)
            }

            ExprKind::Meta(body) => {
                // meta { ... }
                // Compile-time execution block - evaluated during compilation
                //
                // Meta system: compile-time computation via meta fn / @derive / @tagged_literal.
                //
                // Meta blocks are evaluated at compile time, enabling:
                // 1. Compile-time arithmetic and type computation
                // 2. Generic parameter evaluation
                // 3. Const-if elimination (dead code removal)
                // 4. Build-time configuration
                //
                // Strategy:
                // 1. Try to evaluate the block's trailing expression at compile time
                // 2. If successful, embed result as constant in MIR
                // 3. If not evaluable (side effects, non-const operations), emit error

                // For blocks with only a trailing expression, try const evaluation
                if body.stmts.is_empty() {
                    if let Some(ref expr) = body.expr {
                        // Attempt compile-time evaluation
                        match self.const_evaluator.eval(expr.as_ref()) {
                            Ok(const_val) => {
                                // Successfully evaluated at compile time
                                let mir_const = self.const_value_to_mir_constant(&const_val);
                                self.push_statement(
                                    current_block,
                                    MirStatement::Assign(
                                        dest,
                                        Rvalue::Use(Operand::Constant(mir_const)),
                                    ),
                                );
                                return Ok(current_block);
                            }
                            Err(_) => {
                                // Cannot evaluate at compile time
                                // Fall through to try lowering with statement processing
                            }
                        }
                    }
                }

                // For blocks with statements, try to process each statement
                // and evaluate bindings for const evaluation context
                self.push_scope();
                let mut curr = current_block;
                let mut can_const_eval = true;

                for stmt in body.stmts.iter() {
                    match &stmt.kind {
                        StmtKind::Let {
                            pattern,
                            ty: _,
                            value: Some(value),
                            ..
                        } => {
                            // Try to const-evaluate the value and bind it
                            if let PatternKind::Ident { name, .. } = &pattern.kind {
                                match self.const_evaluator.eval(value) {
                                    Ok(const_val) => {
                                        // Bind the const value for subsequent evaluation
                                        self.const_evaluator
                                            .bind(name.name.as_str(), const_val.clone());

                                        // Also emit MIR for the binding
                                        let mir_const =
                                            self.const_value_to_mir_constant(&const_val);
                                        let local = self.new_local(
                                            name.name.clone(),
                                            MirType::Infer,
                                            LocalKind::Var,
                                        );
                                        self.push_statement(
                                            curr,
                                            MirStatement::Assign(
                                                Place::local(local),
                                                Rvalue::Use(Operand::Constant(mir_const)),
                                            ),
                                        );
                                        self.bind_var(name.name.as_str(), local);
                                    }
                                    Err(_) => {
                                        can_const_eval = false;
                                        // Fall back to runtime evaluation for this statement
                                        curr = self.lower_stmt_iterative(stmt, curr)?;
                                    }
                                }
                            } else {
                                // Complex patterns require runtime handling
                                can_const_eval = false;
                                curr = self.lower_stmt_iterative(stmt, curr)?;
                            }
                        }
                        _ => {
                            // Other statement types (expressions, etc.)
                            can_const_eval = false;
                            curr = self.lower_stmt_iterative(stmt, curr)?;
                        }
                    }
                }

                // Evaluate trailing expression
                if let Some(ref expr) = body.expr {
                    if can_const_eval {
                        // Try const evaluation for the final expression
                        match self.const_evaluator.eval(expr.as_ref()) {
                            Ok(const_val) => {
                                let mir_const = self.const_value_to_mir_constant(&const_val);
                                self.push_statement(
                                    curr,
                                    MirStatement::Assign(
                                        dest,
                                        Rvalue::Use(Operand::Constant(mir_const)),
                                    ),
                                );
                            }
                            Err(e) => {
                                // Emit warning that meta block couldn't be fully const-evaluated
                                self.diagnostics.push(
                                    DiagnosticBuilder::new(Severity::Warning)
                                        .message(format!(
                                            "meta block expression could not be evaluated at compile time: {}",
                                            e
                                        ))
                                        .add_note("falling back to runtime evaluation")
                                        .build(),
                                );
                                curr = self.lower_expr(expr.as_ref(), curr, dest)?;
                            }
                        }
                    } else {
                        // Not all statements were const-evaluable, lower expression normally
                        curr = self.lower_expr(expr.as_ref(), curr, dest)?;
                    }
                } else {
                    // No trailing expression, result is unit
                    self.push_statement(
                        curr,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Use(Operand::Constant(MirConstant::Unit)),
                        ),
                    );
                }

                self.pop_scope(curr);
                Ok(curr)
            }

            ExprKind::MacroCall { path, args } => {
                // Macro should be expanded before MIR lowering
                // If we reach here, there was a failure in macro expansion phase
                //
                // This is a compilation error - macros must be fully expanded
                // during the macro expansion phase before MIR lowering.
                //
                // Possible causes:
                // 1. Unknown/undefined macro
                // 2. Macro expansion phase was skipped
                // 3. Recursive macro expansion exceeded limits
                // 4. Macro returned invalid AST

                let macro_name = path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        verum_ast::ty::PathSegment::SelfValue => "self".to_string(),
                        verum_ast::ty::PathSegment::Super => "super".to_string(),
                        verum_ast::ty::PathSegment::Cog => "cog".to_string(),
                        verum_ast::ty::PathSegment::Relative => ".".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("::");

                let arg_count = args.tokens.len();
                let span = self.span_to_diag(expr.span);

                self.diagnostics.push(
                    DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "unexpanded macro `{}!` with {} argument(s)",
                            macro_name, arg_count
                        ))
                        .span(span)
                        .add_note("macros must be expanded before MIR lowering")
                        .help("ensure the macro is defined and the macro expansion phase completed successfully")
                        .build(),
                );

                // Emit unreachable terminator since this is an error condition
                // The code should not execute past this point
                let error_block = self.new_block();
                self.set_terminator(current_block, Terminator::Goto(error_block));
                self.set_terminator(error_block, Terminator::Unreachable);

                // Create a dummy exit block for control flow continuity
                let exit_block = self.new_block();
                self.push_statement(
                    exit_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Undef))),
                );
                Ok(exit_block)
            }

            ExprKind::UseContext {
                context,
                handler,
                body,
            } => {
                // use ContextName = handler in body
                // Bind context handler and evaluate body
                let handler_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(handler, current_block, Place::local(handler_temp))?;

                // Extract context name
                let context_name: Text = context
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(".")
                    .into();

                // Provide context
                self.push_statement(
                    block,
                    MirStatement::ContextProvide {
                        context_name: context_name.clone(),
                        value: Place::local(handler_temp),
                    },
                );

                // Lower body with context in scope
                let body_exit = self.lower_expr(body, block, dest)?;

                // Unprovide context
                self.push_statement(body_exit, MirStatement::ContextUnprovide { context_name });

                Ok(body_exit)
            }

            ExprKind::Forall { bindings, body } => {
                // forall x: T. predicate(x)
                // Used in dependent types and formal verification
                // Lower as a function that takes witnesses for all bindings
                self.push_scope();

                // Process each binding
                for binding in bindings {
                    let param_ty = if let verum_common::Maybe::Some(ty) = &binding.ty {
                        self.lower_type(ty)
                    } else {
                        MirType::Infer
                    };
                    let param = self.new_temp(param_ty);
                    let _ = self.lower_pattern_binding(&binding.pattern, Place::local(param), current_block)?;
                }

                let body_temp = self.new_temp(MirType::Bool);
                let body_exit = self.lower_expr(body, current_block, Place::local(body_temp))?;

                self.pop_scope(body_exit);

                // Result is always true (proof obligation)
                self.push_statement(
                    body_exit,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                Ok(body_exit)
            }

            ExprKind::Exists { bindings, body } => {
                // exists x: T. predicate(x)
                // Used in dependent types and formal verification
                self.push_scope();

                // Process each binding
                for binding in bindings {
                    let param_ty = if let verum_common::Maybe::Some(ty) = &binding.ty {
                        self.lower_type(ty)
                    } else {
                        MirType::Infer
                    };
                    let param = self.new_temp(param_ty);
                    let _ = self.lower_pattern_binding(&binding.pattern, Place::local(param), current_block)?;
                }

                let body_temp = self.new_temp(MirType::Bool);
                let body_exit = self.lower_expr(body, current_block, Place::local(body_temp))?;

                self.pop_scope(body_exit);

                // Result depends on whether witness exists
                self.push_statement(
                    body_exit,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Copy(Place::local(body_temp)))),
                );

                Ok(body_exit)
            }

            ExprKind::Attenuate {
                context,
                capabilities: _,
            } => {
                // context.attenuate(capabilities)
                // Creates a restricted sub-context
                let context_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(context, current_block, Place::local(context_temp))?;

                // Call attenuate method
                let success_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                self.set_terminator(
                    block,
                    Terminator::Call {
                        destination: dest,
                        func: Operand::Constant(MirConstant::Function("Context::attenuate".into())),
                        args: List::from(vec![Operand::Move(Place::local(context_temp))]),
                        success_block,
                        unwind_block,
                    },
                );

                self.set_terminator(unwind_block, Terminator::Resume);

                Ok(success_block)
            }

            ExprKind::TypeProperty { ty, property } => {
                // Type property expression (T.size, T.alignment, etc.)
                // These are compile-time constants that should be evaluated during constant folding
                // If we reach MIR lowering, we emit a constant based on the property
                use verum_ast::expr::TypeProperty;

                let constant = match property {
                    TypeProperty::Size => {
                        let size = self.compute_type_size(ty);
                        MirConstant::Int(size)
                    }
                    TypeProperty::Alignment => {
                        let align = self.compute_type_alignment(ty);
                        MirConstant::Int(align)
                    }
                    TypeProperty::Stride => {
                        let size = self.compute_type_size(ty);
                        let align = self.compute_type_alignment(ty);
                        // Round up size to next multiple of alignment
                        MirConstant::Int((size + align - 1) / align * align)
                    }
                    TypeProperty::Bits => {
                        let size = self.compute_type_size(ty);
                        MirConstant::Int(size * 8)
                    }
                    TypeProperty::Name => {
                        let name = self.compute_type_name(ty);
                        MirConstant::String(name)
                    }
                    TypeProperty::Min => {
                        // Return minimum value for numeric types
                        self.compute_type_min_constant(ty)
                    }
                    TypeProperty::Max => {
                        // Return maximum value for numeric types
                        self.compute_type_max_constant(ty)
                    }
                    TypeProperty::Id => {
                        // Return hash of canonical type name using Blake3
                        let name = self.compute_type_name(ty);
                        let mut hasher = crate::hash::ContentHash::new();
                        hasher.update_str(&name);
                        MirConstant::Int(hasher.finalize().to_u64() as i64)
                    }
                };

                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(constant))),
                );

                Ok(current_block)
            }

            // Throw expression - wraps error in Result::Err and returns
            // throw: wraps value in Err and returns from the current function.
            //
            // For functions with `throws(E)` clause, `throw e` is lowered to:
            //   1. Evaluate the thrown expression
            //   2. Run deferred cleanups (error path cleanup)
            //   3. Wrap in Result::Err variant
            //   4. Return from function
            //
            // This allows proper integration with the Result-based error handling
            // system while supporting typed throws declarations.
            ExprKind::Throw(inner) => {
                // Evaluate the thrown expression (the error value)
                let error_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(error_temp))?;

                // Create cleanup block for error path (errdefer semantics)
                // errdefer: deferred cleanup that runs only on error-path unwinding.
                let cleanup_block = self.new_cleanup_block();

                // Emit deferred cleanups before returning error
                // This ensures resources are properly cleaned up on error path
                self.emit_deferred_cleanups(block);

                // Wrap the error value in Result::Err variant
                // Result is represented as: Ok = variant 0, Err = variant 1
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::return_place(),
                        Rvalue::Aggregate(
                            AggregateKind::Variant(verum_common::well_known_types::type_names::RESULT.into(), 1), // Err variant
                            List::from(vec![Operand::Move(Place::local(error_temp))]),
                        ),
                    ),
                );

                // Set cleanup block terminator to resume unwinding
                self.set_terminator(cleanup_block, Terminator::Resume);

                // Return from function with the wrapped error
                self.set_terminator(block, Terminator::Return);

                // Create unreachable block for code after throw
                // (control flow never reaches here)
                let unreachable_block = self.new_block();
                self.set_terminator(unreachable_block, Terminator::Unreachable);

                Ok(unreachable_block)
            }

            ExprKind::Typeof(inner) => {
                // Typeof expressions return runtime type information
                // Lower the inner expression and call runtime typeof intrinsic
                let inner_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(inner, current_block, Place::local(inner_temp))?;

                // Call typeof intrinsic (placeholder - will be resolved at codegen)
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        dest,
                        Rvalue::Use(Operand::Constant(MirConstant::Unit)), // Placeholder
                    ),
                );

                Ok(block)
            }

            // Select expression - races multiple async operations
            // Syntax: generator/async generator expression lowering.
            //
            // Lowering strategy:
            //   1. Create futures for all arms (evaluate future expressions)
            //   2. Create a polling loop that checks each future
            //   3. When any future completes, bind pattern and execute body
            //   4. Cancel (drop) all other futures
            //   5. Handle default/else arm if no futures are ready
            //
            // The select compiles to a state machine with polling semantics.
            // For biased select, arms are checked in order (earlier = higher priority).
            // For fair select, implementation may randomize or round-robin.
            ExprKind::Select { biased, arms, span: _ } => {
                // Handle empty select (should be caught by type checker, but be defensive)
                if arms.is_empty() {
                    self.push_statement(
                        current_block,
                        MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
                    );
                    return Ok(current_block);
                }

                // Check if there's a default/else arm (no future expression)
                let has_default = arms.iter().any(|arm| arm.is_else());

                // Create temps to hold each future
                let mut future_temps: Vec<LocalId> = Vec::with_capacity(arms.len());
                let mut poll_blocks: Vec<BlockId> = Vec::with_capacity(arms.len());
                let mut body_blocks: Vec<BlockId> = Vec::with_capacity(arms.len());

                // Evaluate all future expressions first (creates the futures)
                let mut setup_block = current_block;
                for arm in arms.iter() {
                    if arm.is_else() {
                        // Default arm has no future to evaluate
                        future_temps.push(LocalId(0)); // Placeholder
                        continue;
                    }

                    if let Some(ref future_expr) = arm.future {
                        let future_temp = self.new_temp(MirType::Infer);
                        self.push_statement(setup_block, MirStatement::StorageLive(future_temp));
                        setup_block =
                            self.lower_expr(future_expr, setup_block, Place::local(future_temp))?;
                        future_temps.push(future_temp);
                    } else {
                        future_temps.push(LocalId(0)); // Placeholder for non-future arms
                    }
                }

                // Create polling loop structure
                let poll_header = self.new_block();
                let join_block = self.new_block();
                let default_block = if has_default {
                    self.new_block()
                } else {
                    // No default - if all futures pend, we suspend
                    self.new_block()
                };

                // Jump from setup to poll header
                self.set_terminator(setup_block, Terminator::Goto(poll_header));

                // Create poll and body blocks for each arm
                for _ in arms.iter() {
                    poll_blocks.push(self.new_block());
                    body_blocks.push(self.new_block());
                }

                // For biased select, check arms in order
                // For fair select, we could randomize, but for simplicity we also use order
                // (true fairness would require runtime support)
                let _ = biased; // Currently both use same ordering; could add shuffle for fair

                // Build the polling chain: poll_header -> poll_0 -> poll_1 -> ... -> default/suspend
                self.set_terminator(poll_header, Terminator::Goto(poll_blocks[0]));

                for (i, arm) in arms.iter().enumerate() {
                    let poll_block = poll_blocks[i];
                    let body_block = body_blocks[i];
                    let next_poll = if i + 1 < poll_blocks.len() {
                        poll_blocks[i + 1]
                    } else {
                        default_block
                    };

                    if arm.is_else() {
                        // Default/else arm - always taken when reached
                        self.set_terminator(poll_block, Terminator::Goto(body_block));
                    } else {
                        // Poll the future using Await terminator
                        // The runtime will check if the future is ready
                        let future_temp = future_temps[i];
                        let result_temp = self.new_temp(MirType::Infer);
                        let unwind_block = self.new_cleanup_block();

                        // Create an await point that resumes to result check
                        let ready_check_block = self.new_block();

                        self.set_terminator(
                            poll_block,
                            Terminator::Await {
                                future: Place::local(future_temp),
                                destination: Place::local(result_temp),
                                resume_block: ready_check_block,
                                unwind_block,
                            },
                        );

                        self.set_terminator(unwind_block, Terminator::Resume);

                        // After await, check if we got a value (Poll::Ready) or not (Poll::Pending)
                        // For select, we need to handle Poll::Pending by trying next arm
                        // We use discriminant check: 0 = Ready, 1 = Pending
                        let poll_discrim = self.new_temp(MirType::Int);
                        self.push_statement(
                            ready_check_block,
                            MirStatement::Assign(
                                Place::local(poll_discrim),
                                Rvalue::Discriminant(Place::local(result_temp)),
                            ),
                        );

                        // Branch based on poll result
                        self.set_terminator(
                            ready_check_block,
                            Terminator::SwitchInt {
                                discriminant: Operand::Copy(Place::local(poll_discrim)),
                                targets: List::from(vec![(0, body_block)]), // Ready -> body
                                otherwise: next_poll,                       // Pending -> try next
                            },
                        );

                        // In body block, bind pattern and execute
                        self.push_scope();

                        // Extract ready value from Poll::Ready(value)
                        let ready_value_place =
                            Place::local(result_temp).with_downcast(0).with_field(0);

                        // Bind pattern to the ready value
                        if let Some(ref pattern) = arm.pattern {
                            let _ =
                                self.lower_pattern_binding(pattern, ready_value_place, body_block)?;
                        }

                        // Check guard if present
                        let exec_block = if let Some(ref guard) = arm.guard {
                            let guard_temp = self.new_temp(MirType::Bool);
                            let guard_block =
                                self.lower_expr(guard, body_block, Place::local(guard_temp))?;

                            let guard_pass = self.new_block();
                            self.set_terminator(
                                guard_block,
                                Terminator::Branch {
                                    condition: Operand::Copy(Place::local(guard_temp)),
                                    then_block: guard_pass,
                                    else_block: next_poll, // Guard failed, try next arm
                                },
                            );

                            guard_pass
                        } else {
                            body_block
                        };

                        // Lower the body expression
                        let body_exit = self.lower_expr(&arm.body, exec_block, dest.clone())?;
                        self.pop_scope(body_exit);

                        // Drop all other futures (cancellation)
                        for (j, &other_temp) in future_temps.iter().enumerate() {
                            if j != i && other_temp.0 != 0 && !arms[j].is_else() {
                                self.push_statement(body_exit, MirStatement::Drop(Place::local(other_temp)));
                            }
                        }

                        // Jump to join block
                        self.set_terminator(body_exit, Terminator::Goto(join_block));
                    }
                }

                // Handle default/else arm body
                if has_default {
                    for (i, arm) in arms.iter().enumerate() {
                        if arm.is_else() {
                            let body_block = body_blocks[i];
                            self.push_scope();

                            // Lower the else body
                            let body_exit = self.lower_expr(&arm.body, body_block, dest.clone())?;
                            self.pop_scope(body_exit);

                            // Drop all futures
                            for (j, &other_temp) in future_temps.iter().enumerate() {
                                if other_temp.0 != 0 && !arms[j].is_else() {
                                    self.push_statement(body_exit, MirStatement::Drop(Place::local(other_temp)));
                                }
                            }

                            self.set_terminator(body_exit, Terminator::Goto(join_block));
                            break;
                        }
                    }
                } else {
                    // No default arm - if all futures are pending, we yield back to executor
                    // This creates a suspend point that will be resumed when any future is ready
                    // For now, we loop back to poll_header (busy-wait pattern)
                    // A proper implementation would use waker registration
                    self.set_terminator(default_block, Terminator::Goto(poll_header));
                }

                Ok(join_block)
            }

            // Is expression - pattern test (value is Pattern / value !is Pattern)
            // Spec: grammar/verum.ebnf - is_expr production
            //
            // The `is` expression tests if a value matches a pattern and returns Bool.
            // If `negated` is true, it's the `!is` operator which inverts the result.
            //
            // Example: `x is Some(y)` returns true if x matches Some variant
            // Example: `x !is None` returns true if x does NOT match None
            //
            // Lowering strategy:
            //   1. Evaluate the expression to test
            //   2. Use pattern matching logic to generate match check code
            //   3. The result is a boolean indicating match success
            //   4. If negated, invert the boolean result
            //
            // This uses the same pattern matching infrastructure as `match` but
            // doesn't bind any variables (we only care about match success).
            ExprKind::Is { expr: inner, pattern, negated } => {
                // Evaluate the expression to test
                let scrut_temp = self.new_temp(MirType::Infer);
                let eval_block = self.lower_expr(inner, current_block, Place::local(scrut_temp))?;

                // Use pattern matching to test if the value matches
                // lower_pattern_test returns a LocalId holding bool indicating match
                // Note: We create a temporary scope to prevent bindings from escaping
                self.push_scope();
                let match_result = self.lower_pattern_test(pattern, Place::local(scrut_temp), eval_block)?;
                self.pop_scope(eval_block);

                // If negated (!is), invert the result
                if *negated {
                    // dest = !match_result
                    self.push_statement(
                        eval_block,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Unary(UnOp::Not, Operand::Copy(Place::local(match_result))),
                        ),
                    );
                } else {
                    // dest = match_result
                    self.push_statement(
                        eval_block,
                        MirStatement::Assign(
                            dest,
                            Rvalue::Use(Operand::Copy(Place::local(match_result))),
                        ),
                    );
                }

                Ok(eval_block)
            }

            ExprKind::TypeExpr(_) => {
                // Type expressions (e.g., List<Int>) are used for static method calls
                // They should be desugared into direct function calls during HIR/AST lowering
                // If we reach here, it means the type expression wasn't properly handled
                Err(DiagnosticBuilder::new(Severity::Error)
                    .message("type expression should be resolved through method call desugaring")
                    .build())
            }

            ExprKind::TypeBound { .. } => {
                // Type bound expressions (T: Protocol) are compile-time conditions
                // They are evaluated during type checking and should not appear in MIR
                // If we reach here, assign true (the check passed during type checking)
                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Bool(true)))),
                );
                Ok(current_block)
            }

            ExprKind::MetaFunction { name, .. } => {
                // Meta-functions (@file, @line, @const, etc.) should be evaluated at compile-time
                // If we reach MIR lowering, it means the meta-function wasn't expanded
                Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!("Meta-function @{} should be expanded before MIR lowering", name.name))
                    .build())
            }

            ExprKind::Quote { .. } => {
                // Quote expressions are compile-time constructs for staged metaprogramming
                // They should be expanded during the staged compilation pipeline
                // If we reach MIR lowering, emit unit as placeholder
                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
                );
                Ok(current_block)
            }

            ExprKind::StageEscape { expr, .. } => {
                // Stage escape expressions are processed during staged compilation
                // During MIR lowering, we simply lower the inner expression
                // The stage context is handled by the meta pipeline
                self.lower_expr(expr, current_block, dest)
            }

            ExprKind::Lift { expr } => {
                // Lift expressions are syntactic sugar for stage escape at current stage
                // During MIR lowering, we simply lower the inner expression
                // The stage context is handled by the meta pipeline
                self.lower_expr(expr, current_block, dest)
            }

            ExprKind::Nursery {
                options,
                body,
                on_cancel,
                recover,
                ..
            } => {
                // Nursery creates a structured concurrency scope.
                // All spawned tasks must complete before the scope exits.
                //
                // MIR lowering maps nursery semantics to existing MIR constructs:
                // 1. Call runtime::nursery_create with options to get a handle
                // 2. Enter a new scope for the body
                // 3. Lower body statements (spawn expressions use the handle)
                // 4. Call runtime::nursery_await_all to wait for tasks
                // 5. Branch on error: if error, go to on_cancel/recover; else join
                // 6. Call runtime::nursery_destroy for cleanup

                // Step 1: Create nursery handle via runtime call
                let nursery_handle = self.new_temp(MirType::Named("NurseryHandle".into()));
                let create_success = self.new_block();
                let create_unwind = self.new_cleanup_block();

                // Build args for nursery_create: timeout and max_tasks (as i64, -1 for None)
                let mut create_args: Vec<Operand> = Vec::new();
                let mut current = current_block;

                // Timeout argument
                let timeout_temp = self.new_temp(MirType::Int);
                if let verum_common::Maybe::Some(timeout_expr) = &options.timeout {
                    current = self.lower_expr(timeout_expr, current, Place::local(timeout_temp))?;
                    create_args.push(Operand::Copy(Place::local(timeout_temp)));
                } else {
                    // No timeout: pass -1 sentinel
                    self.push_statement(
                        current,
                        MirStatement::Assign(
                            Place::local(timeout_temp),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(-1))),
                        ),
                    );
                    create_args.push(Operand::Copy(Place::local(timeout_temp)));
                }

                // Max tasks argument
                let max_tasks_temp = self.new_temp(MirType::Int);
                if let verum_common::Maybe::Some(max_expr) = &options.max_tasks {
                    current = self.lower_expr(max_expr, current, Place::local(max_tasks_temp))?;
                    create_args.push(Operand::Copy(Place::local(max_tasks_temp)));
                } else {
                    // No max: pass -1 sentinel (unlimited)
                    self.push_statement(
                        current,
                        MirStatement::Assign(
                            Place::local(max_tasks_temp),
                            Rvalue::Use(Operand::Constant(MirConstant::Int(-1))),
                        ),
                    );
                    create_args.push(Operand::Copy(Place::local(max_tasks_temp)));
                }

                self.set_terminator(
                    current,
                    Terminator::Call {
                        destination: Place::local(nursery_handle),
                        func: Operand::Constant(MirConstant::Function("runtime::nursery_create".into())),
                        args: List::from(create_args),
                        success_block: create_success,
                        unwind_block: create_unwind,
                    },
                );
                self.set_terminator(create_unwind, Terminator::Resume);

                // Step 2-3: Lower body in a new scope
                self.push_scope();
                let body_result = self.new_temp(MirType::Infer);
                let mut body_block = create_success;
                for stmt in body.stmts.iter() {
                    body_block = self.lower_stmt(stmt, body_block)?;
                }
                if let verum_common::Maybe::Some(expr) = &body.expr {
                    body_block = self.lower_expr(expr, body_block, Place::local(body_result))?;
                }
                self.pop_scope(body_block);

                // Step 4: Await all tasks — returns error status (0 = ok, nonzero = error)
                let await_result = self.new_temp(MirType::Int);
                let await_success = self.new_block();
                let await_unwind = self.new_cleanup_block();

                self.set_terminator(
                    body_block,
                    Terminator::Call {
                        destination: Place::local(await_result),
                        func: Operand::Constant(MirConstant::Function("runtime::nursery_await_all".into())),
                        args: List::from(vec![Operand::Copy(Place::local(nursery_handle))]),
                        success_block: await_success,
                        unwind_block: await_unwind,
                    },
                );
                self.set_terminator(await_unwind, Terminator::Resume);

                // Step 5: Branch on error status
                let join_block = self.new_block();
                let error_block = self.new_block();

                // Check if await_result != 0 (error occurred)
                let has_error = self.new_temp(MirType::Bool);
                self.push_statement(
                    await_success,
                    MirStatement::Assign(
                        Place::local(has_error),
                        Rvalue::Binary(
                            BinOp::Ne,
                            Operand::Copy(Place::local(await_result)),
                            Operand::Constant(MirConstant::Int(0)),
                        ),
                    ),
                );

                // If there is an on_cancel or recover handler, branch to error path;
                // otherwise go directly to join
                let has_error_handler = on_cancel.is_some() || recover.is_some();
                if has_error_handler {
                    self.set_terminator(
                        await_success,
                        Terminator::Branch {
                            condition: Operand::Copy(Place::local(has_error)),
                            then_block: error_block,
                            else_block: join_block,
                        },
                    );

                    // Lower on_cancel block if present
                    let mut error_exit = error_block;
                    if let verum_common::Maybe::Some(cancel_block) = on_cancel {
                        self.push_scope();
                        let cancel_dest = self.new_temp(MirType::Infer);
                        error_exit = self.lower_block_iterative(cancel_block, error_block, Place::local(cancel_dest))?;
                        self.pop_scope(error_exit);
                    }

                    // Lower recover block if present
                    if let verum_common::Maybe::Some(recover_body) = recover {
                        // Get the error value from the nursery for matching
                        let error_val = self.new_temp(MirType::Infer);
                        let get_err_success = self.new_block();
                        let get_err_unwind = self.new_cleanup_block();

                        self.set_terminator(
                            error_exit,
                            Terminator::Call {
                                destination: Place::local(error_val),
                                func: Operand::Constant(MirConstant::Function("runtime::nursery_get_error".into())),
                                args: List::from(vec![Operand::Copy(Place::local(nursery_handle))]),
                                success_block: get_err_success,
                                unwind_block: get_err_unwind,
                            },
                        );
                        self.set_terminator(get_err_unwind, Terminator::Resume);

                        match recover_body {
                            RecoverBody::MatchArms { arms, .. } => {
                                // Lower match arms inline against the error_val.
                                // Build a switch over arm discriminants, then lower
                                // each arm body.
                                let recover_join = self.new_block();
                                let mut arm_blocks: Vec<BlockId> = Vec::new();
                                for _ in arms.iter() {
                                    arm_blocks.push(self.new_block());
                                }
                                let recover_otherwise = self.new_block();
                                self.set_terminator(recover_otherwise, Terminator::Unreachable);

                                let mut targets = List::new();
                                for (i, arm) in arms.iter().enumerate() {
                                    let discriminant = self.pattern_to_discriminant(&arm.pattern);
                                    if let Some(disc) = discriminant {
                                        targets.push((disc, arm_blocks[i]));
                                    }
                                }

                                let err_discrim = self.new_temp(MirType::Int);
                                self.push_statement(
                                    get_err_success,
                                    MirStatement::Assign(
                                        Place::local(err_discrim),
                                        Rvalue::Discriminant(Place::local(error_val)),
                                    ),
                                );
                                self.set_terminator(
                                    get_err_success,
                                    Terminator::SwitchInt {
                                        discriminant: Operand::Copy(Place::local(err_discrim)),
                                        targets,
                                        otherwise: recover_otherwise,
                                    },
                                );

                                for (i, arm) in arms.iter().enumerate() {
                                    self.push_scope();
                                    let _bind = self.lower_pattern_binding(
                                        &arm.pattern,
                                        Place::local(error_val),
                                        arm_blocks[i],
                                    )?;
                                    let arm_exit = self.lower_expr(
                                        &arm.body,
                                        arm_blocks[i],
                                        dest.clone(),
                                    )?;
                                    self.pop_scope(arm_exit);
                                    self.set_terminator(arm_exit, Terminator::Goto(recover_join));
                                }

                                error_exit = recover_join;
                            }
                            RecoverBody::Closure { body: closure_body, .. } => {
                                // Lower closure body with error value as implicit argument
                                error_exit = self.lower_expr(closure_body, get_err_success, dest.clone())?;
                            }
                        }
                    }

                    self.set_terminator(error_exit, Terminator::Goto(join_block));
                } else {
                    // No error handlers — go directly to join
                    self.set_terminator(await_success, Terminator::Goto(join_block));
                }

                // Step 6: Destroy nursery handle (cleanup)
                let destroy_success = self.new_block();
                let destroy_unwind = self.new_cleanup_block();

                self.set_terminator(
                    join_block,
                    Terminator::Call {
                        destination: dest.clone(),
                        func: Operand::Constant(MirConstant::Function("runtime::nursery_destroy".into())),
                        args: List::from(vec![Operand::Move(Place::local(nursery_handle))]),
                        success_block: destroy_success,
                        unwind_block: destroy_unwind,
                    },
                );
                self.set_terminator(destroy_unwind, Terminator::Resume);

                // If body had no trailing expression, assign unit as final result
                if body.expr.is_none() && !has_error_handler {
                    self.push_statement(
                        destroy_success,
                        MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
                    );
                }

                Ok(destroy_success)
            }

            ExprKind::StreamLiteral(stream_lit) => {
                // Stream literals create lazy iterators/generators
                //
                // stream[1, 2, 3]     -> finite iterator over elements
                // stream[1, 2, 3, ...] -> cycling infinite iterator
                // stream[0..100]     -> lazy range iterator [0, 100)
                // stream[0..]        -> infinite range from 0
                //
                // Stream literals: syntax sugar for creating stream values.
                use verum_ast::expr::StreamLiteralKind;

                match &stream_lit.kind {
                    StreamLiteralKind::Elements { elements, cycles } => {
                        if elements.is_empty() {
                            // Empty stream: stream[] - create empty iterator
                            // MIR: create an empty array and wrap in iterator
                            let empty_array = self.new_temp(MirType::Infer);
                            self.push_statement(
                                current_block,
                                MirStatement::Assign(
                                    Place::local(empty_array),
                                    Rvalue::Aggregate(AggregateKind::Array(MirType::Infer), List::new()),
                                ),
                            );
                            // Wrap in iterator (runtime handles this)
                            self.push_statement(
                                current_block,
                                MirStatement::Assign(
                                    dest,
                                    Rvalue::Use(Operand::Move(Place::local(empty_array))),
                                ),
                            );
                            return Ok(current_block);
                        }

                        // Build array of elements first
                        let mut temps = List::new();
                        let mut block = current_block;

                        for elem in elements.iter() {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(elem, block, Place::local(temp))?;
                            temps.push(Operand::Move(Place::local(temp)));
                        }

                        // Create array aggregate
                        let array_temp = self.new_temp(MirType::Infer);
                        self.push_statement(
                            block,
                            MirStatement::Assign(
                                Place::local(array_temp),
                                Rvalue::Aggregate(AggregateKind::Array(MirType::Infer), temps),
                            ),
                        );

                        if *cycles {
                            // Cycling stream: mark as cycling for runtime
                            // For MIR, we represent this as a generator that cycles
                            self.push_statement(
                                block,
                                MirStatement::Assign(
                                    dest,
                                    Rvalue::Aggregate(
                                        AggregateKind::Generator("__stream_cycle".into()),
                                        List::from_iter([Operand::Move(Place::local(array_temp))]),
                                    ),
                                ),
                            );
                        } else {
                            // Finite stream: create iterator from array
                            self.push_statement(
                                block,
                                MirStatement::Assign(
                                    dest,
                                    Rvalue::Aggregate(
                                        AggregateKind::Generator("__stream_iter".into()),
                                        List::from_iter([Operand::Move(Place::local(array_temp))]),
                                    ),
                                ),
                            );
                        }

                        Ok(block)
                    }

                    StreamLiteralKind::Range { start, end, inclusive } => {
                        // Range-based stream: stream[0..100] or stream[0..]
                        let start_temp = self.new_temp(MirType::Int);
                        let block1 = self.lower_expr(start, current_block, Place::local(start_temp))?;

                        let (block2, end_operand) = if let verum_common::Maybe::Some(end_expr) = end {
                            let end_temp = self.new_temp(MirType::Int);
                            let b = self.lower_expr(end_expr, block1, Place::local(end_temp))?;
                            (b, verum_common::Maybe::Some(Operand::Move(Place::local(end_temp))))
                        } else {
                            // Infinite range: stream[0..]
                            (block1, verum_common::Maybe::None)
                        };

                        // Create range generator
                        // For finite range: Generator("__stream_range", [start, end, inclusive])
                        // For infinite range: Generator("__stream_range_infinite", [start])
                        let generator_name = if end_operand.is_some() {
                            if *inclusive {
                                "__stream_range_inclusive"
                            } else {
                                "__stream_range"
                            }
                        } else {
                            "__stream_range_infinite"
                        };

                        let mut operands = List::new();
                        operands.push(Operand::Move(Place::local(start_temp)));
                        if let verum_common::Maybe::Some(end_op) = end_operand {
                            operands.push(end_op);
                        }

                        self.push_statement(
                            block2,
                            MirStatement::Assign(
                                dest,
                                Rvalue::Aggregate(AggregateKind::Generator(generator_name.into()), operands),
                            ),
                        );

                        Ok(block2)
                    }
                }
            }

            // Phase 5: Inline Assembly
            // At MIR level, inline assembly is represented as a terminator
            // that will be handled by the LLVM backend (or error at VBC tier)
            ExprKind::InlineAsm { template, operands, options } => {
                // Lower each operand expression and categorize into inputs/outputs
                let mut inputs = List::new();
                let mut outputs = List::new();
                let mut clobbers = List::new();
                let mut block = current_block;

                for operand in operands.iter() {
                    match &operand.kind {
                        verum_ast::expr::AsmOperandKind::In { constraint, expr } => {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(expr, block, Place::local(temp))?;
                            inputs.push((constraint.constraint.clone(), Operand::Move(Place::local(temp))));
                        }
                        verum_ast::expr::AsmOperandKind::Out { constraint, place, .. } => {
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(place, block, Place::local(temp))?;
                            outputs.push((constraint.constraint.clone(), Place::local(temp)));
                        }
                        verum_ast::expr::AsmOperandKind::InOut { constraint, place } => {
                            // InOut is both input and output to the same place
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(place, block, Place::local(temp))?;
                            inputs.push((constraint.constraint.clone(), Operand::Copy(Place::local(temp))));
                            outputs.push((constraint.constraint.clone(), Place::local(temp)));
                        }
                        verum_ast::expr::AsmOperandKind::InLateOut { constraint, in_expr, out_place, .. } => {
                            // Input from one expr, output to different place
                            let in_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(in_expr, block, Place::local(in_temp))?;
                            inputs.push((constraint.constraint.clone(), Operand::Move(Place::local(in_temp))));
                            let out_temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(out_place, block, Place::local(out_temp))?;
                            outputs.push((constraint.constraint.clone(), Place::local(out_temp)));
                        }
                        verum_ast::expr::AsmOperandKind::Const { expr } => {
                            // Const operands are immediate values
                            let temp = self.new_temp(MirType::Infer);
                            block = self.lower_expr(expr, block, Place::local(temp))?;
                            inputs.push((Text::from("i"), Operand::Copy(Place::local(temp))));
                        }
                        verum_ast::expr::AsmOperandKind::Sym { path } => {
                            // Symbol operands are handled by codegen
                            let sym_name = path.segments.iter()
                                .map(|s| match s {
                                    verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                                    _ => "_".to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join("::");
                            inputs.push((Text::from("s"), Operand::Constant(MirConstant::Function(sym_name.into()))));
                        }
                        verum_ast::expr::AsmOperandKind::Clobber { reg } => {
                            clobbers.push(reg.clone());
                        }
                    }
                }

                // Convert AST options to MIR options
                let mir_options = MirAsmOptions {
                    volatile: options.volatile,
                    pure_: options.pure_asm,
                    nomem: options.nomem,
                    readonly: options.readonly,
                    preserves_flags: options.preserves_flags,
                    nostack: options.nostack,
                    att_syntax: !options.intel_syntax,
                };

                // Create continuation block for after the asm
                let success_block = self.new_block();
                let unwind_block = self.new_block();

                // Set terminator for inline assembly
                self.set_terminator(
                    block,
                    Terminator::InlineAsm {
                        template: template.clone(),
                        inputs,
                        outputs,
                        clobbers,
                        options: mir_options,
                        destination: Some(success_block),
                        unwind: Some(unwind_block),
                    },
                );

                // Unwind block resumes unwinding
                self.set_terminator(unwind_block, Terminator::Resume);

                // The result is typically unit for inline assembly
                self.push_statement(
                    success_block,
                    MirStatement::Assign(dest, Rvalue::NullConstant),
                );

                Ok(success_block)
            }

            ExprKind::DestructuringAssign { pattern, op, value } => {
                // Destructuring assignment: (a, b) = expr or (a, b) += (da, db)
                // 1. Evaluate the value expression
                let value_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(value, current_block, Place::local(value_temp))?;

                // 2. Handle simple vs compound assignment
                if *op == verum_ast::BinOp::Assign {
                    // Simple assignment: (a, b) = expr
                    // Lower pattern binding using the existing pattern matching infrastructure
                    let _binding_local = self.lower_pattern_binding(pattern, Place::local(value_temp), block)?;
                } else {
                    // Compound assignment: (a, b) += (da, db)
                    // Each pattern element must be an existing variable that gets updated
                    self.lower_compound_destructuring(pattern, Place::local(value_temp), op, block)?;
                }

                // Assignment doesn't produce a value, assign unit to dest
                self.push_statement(
                    block,
                    MirStatement::Assign(dest, Rvalue::NullConstant),
                );

                Ok(block)
            }

            // Calc blocks are proof constructs - they lower to unit at runtime
            ExprKind::CalcBlock(_) => {
                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest, Rvalue::NullConstant),
                );
                Ok(current_block)
            }

            // Named arguments - lower the value expression
            ExprKind::NamedArg { value, .. } => {
                self.lower_expr(value, current_block, dest)
            }

            // Copattern body: coinductive value definition via observations.
            // In MIR, lower the copattern body as an opaque aggregate —
            // each arm is sequentially lowered and stored as a field.
            // The resulting block holds the last assigned block id.
            ExprKind::CopatternBody { arms, .. } => {
                // Emit a null/unit as the coinductive object placeholder.
                // Full MIR lowering of coinductive types requires a dedicated
                // codata elimination pass; for now we lower it as a NullConstant
                // to allow the pipeline to proceed.
                self.push_statement(
                    current_block,
                    MirStatement::Assign(dest.clone(), Rvalue::NullConstant),
                );
                // Evaluate each arm body for its side-effects (e.g. recursive calls)
                // and store into a temporary to keep MIR well-formed.
                let mut block = current_block;
                for arm in arms.iter() {
                    let tmp = self.new_temp(MirType::Infer);
                    block = self.lower_expr(&arm.body, block, Place::local(tmp))?;
                }
                Ok(block)
            }
        }
    }

    /// Compute the size of a type in bytes (for TypeProperty lowering)
    fn compute_type_size(&self, ty: &verum_ast::ty::Type) -> i64 {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Unit => 0,
            TypeKind::Bool => 1,
            TypeKind::Char => 4,
            TypeKind::Int | TypeKind::Float => 8,
            TypeKind::Text => 24,
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "I8" | "U8" => 1,
                        "I16" | "U16" => 2,
                        "I32" | "U32" | "F32" => 4,
                        "I64" | "U64" | "F64" | "Int" | "Float" => 8,
                        "I128" | "U128" => 16,
                        _ => 8,
                    }
                } else {
                    8
                }
            }
            _ => 8,
        }
    }

    /// Compute the alignment of a type in bytes (for TypeProperty lowering)
    fn compute_type_alignment(&self, ty: &verum_ast::ty::Type) -> i64 {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Unit | TypeKind::Bool => 1,
            TypeKind::Char => 4,
            TypeKind::Int | TypeKind::Float | TypeKind::Text => 8,
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "Bool" | "I8" | "U8" => 1,
                        "I16" | "U16" => 2,
                        "I32" | "U32" | "F32" | "Char" => 4,
                        _ => 8,
                    }
                } else {
                    8
                }
            }
            _ => 8,
        }
    }

    /// Compute the name of a type (for TypeProperty lowering)
    fn compute_type_name(&self, ty: &verum_ast::ty::Type) -> Text {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    Text::from(ident.as_str())
                } else {
                    Text::from("<complex_path>")
                }
            }
            _ if ty.kind.primitive_name().is_some() => {
                Text::from(ty.kind.primitive_name().unwrap())
            }
            _ => Text::from("<unknown>"),
        }
    }

    /// Compute the minimum value constant for numeric types
    fn compute_type_min_constant(&self, ty: &verum_ast::ty::Type) -> MirConstant {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Int => MirConstant::Int(i64::MIN),
            TypeKind::Float => MirConstant::Float(f64::MIN),
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "I8" => MirConstant::Int(i8::MIN as i64),
                        "I16" => MirConstant::Int(i16::MIN as i64),
                        "I32" => MirConstant::Int(i32::MIN as i64),
                        "I64" | "Int" => MirConstant::Int(i64::MIN),
                        "U8" | "U16" | "U32" | "U64" | "UInt" => MirConstant::Int(0),
                        "F32" => MirConstant::Float(f32::MIN as f64),
                        "F64" | "Float" => MirConstant::Float(f64::MIN),
                        _ => MirConstant::Undef,
                    }
                } else {
                    MirConstant::Undef
                }
            }
            _ => MirConstant::Undef,
        }
    }

    /// Compute the maximum value constant for numeric types
    fn compute_type_max_constant(&self, ty: &verum_ast::ty::Type) -> MirConstant {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Int => MirConstant::Int(i64::MAX),
            TypeKind::Float => MirConstant::Float(f64::MAX),
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.as_str() {
                        "I8" => MirConstant::Int(i8::MAX as i64),
                        "I16" => MirConstant::Int(i16::MAX as i64),
                        "I32" => MirConstant::Int(i32::MAX as i64),
                        "I64" | "Int" => MirConstant::Int(i64::MAX),
                        "U8" => MirConstant::Int(u8::MAX as i64),
                        "U16" => MirConstant::Int(u16::MAX as i64),
                        "U32" => MirConstant::Int(u32::MAX as i64),
                        "U64" | "UInt" => MirConstant::Int(i64::MAX), // Capped to i64::MAX
                        "F32" => MirConstant::Float(f32::MAX as f64),
                        "F64" | "Float" => MirConstant::Float(f64::MAX),
                        _ => MirConstant::Undef,
                    }
                } else {
                    MirConstant::Undef
                }
            }
            _ => MirConstant::Undef,
        }
    }

    /// Check if overflow checking is needed for an operation
    fn should_check_overflow(&self, op: &BinOp) -> bool {
        matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul)
    }

    /// Determine cast kind based on source and target types
    fn determine_cast_kind(&self, _from: &MirType, to: &MirType) -> CastKind {
        match to {
            MirType::Int
            | MirType::I8
            | MirType::I16
            | MirType::I32
            | MirType::I64
            | MirType::I128
            | MirType::UInt
            | MirType::U8
            | MirType::U16
            | MirType::U32
            | MirType::U64
            | MirType::U128 => CastKind::IntToInt,
            MirType::Float | MirType::F32 | MirType::F64 => CastKind::FloatToFloat,
            MirType::Pointer { .. } => CastKind::Pointer,
            _ => CastKind::Transmute,
        }
    }

    /// Lower assignment operators
    ///
    /// Handles both simple assignment (=) and compound assignments (+=, -=, etc.).
    /// Uses expr_to_place_mut to properly evaluate complex index expressions in
    /// assignment targets like `arr[i + 1] = value`.
    fn lower_assignment(
        &mut self,
        op: &BinOp,
        left: &Expr,
        right: &Expr,
        current_block: BlockId,
        _dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Get the target place, potentially evaluating complex index expressions
        let (target_place, block_after_lhs) = self.expr_to_place_mut(left, current_block)?;

        if *op == BinOp::Assign {
            // Simple assignment: target = rhs
            let block = self.lower_expr(right, block_after_lhs, target_place)?;
            Ok(block)
        } else {
            // Compound assignment (+=, -=, etc.): target = target op rhs
            let compound_op = self.compound_to_binary(op);

            // Evaluate the right-hand side first to a temporary
            let right_temp = self.new_temp(MirType::Infer);
            let block1 = self.lower_expr(right, block_after_lhs, Place::local(right_temp))?;

            // Read the current value of the target
            let left_temp = self.new_temp(MirType::Infer);
            self.push_statement(
                block1,
                MirStatement::Assign(
                    Place::local(left_temp),
                    Rvalue::Use(Operand::Copy(target_place.clone())),
                ),
            );

            // Perform the binary operation
            let result_temp = self.new_temp(MirType::Infer);
            let rvalue = if self.should_check_overflow(&compound_op) {
                Rvalue::CheckedBinary(
                    compound_op,
                    Operand::Copy(Place::local(left_temp)),
                    Operand::Copy(Place::local(right_temp)),
                )
            } else {
                Rvalue::Binary(
                    compound_op,
                    Operand::Copy(Place::local(left_temp)),
                    Operand::Copy(Place::local(right_temp)),
                )
            };

            self.push_statement(
                block1,
                MirStatement::Assign(Place::local(result_temp), rvalue),
            );

            // Write the result back to the target
            self.push_statement(
                block1,
                MirStatement::Assign(
                    target_place,
                    Rvalue::Use(Operand::Move(Place::local(result_temp))),
                ),
            );

            Ok(block1)
        }
    }

    /// Convert compound assignment to binary operation
    fn compound_to_binary(&self, op: &BinOp) -> BinOp {
        match op {
            BinOp::AddAssign => BinOp::Add,
            BinOp::SubAssign => BinOp::Sub,
            BinOp::MulAssign => BinOp::Mul,
            BinOp::DivAssign => BinOp::Div,
            BinOp::RemAssign => BinOp::Rem,
            BinOp::BitAndAssign => BinOp::BitAnd,
            BinOp::BitOrAssign => BinOp::BitOr,
            BinOp::BitXorAssign => BinOp::BitXor,
            BinOp::ShlAssign => BinOp::Shl,
            BinOp::ShrAssign => BinOp::Shr,
            _ => *op,
        }
    }

    /// Convert expression to place with complex index evaluation support
    ///
    /// This version can evaluate complex index expressions that aren't simple variables.
    /// It modifies the MIR builder state by potentially creating temporaries and emitting
    /// statements to evaluate the index expression.
    ///
    /// # Arguments
    /// * `expr` - The expression to convert to a place
    /// * `current_block` - The current basic block being built
    ///
    /// # Returns
    /// A tuple of (Place, BlockId) where:
    /// - Place: The resulting place expression
    /// - BlockId: The potentially new current block after evaluation
    ///
    /// # Examples
    /// ```ignore
    /// // arr[i] where i is a variable -> simple lookup
    /// // arr[x + 1] where x+1 needs evaluation -> creates temp, evaluates, uses temp
    /// // arr[foo()] where foo() needs evaluation -> creates temp, evaluates call, uses temp
    /// ```
    fn expr_to_place_mut(
        &mut self,
        expr: &Expr,
        current_block: BlockId,
    ) -> Result<(Place, BlockId), Diagnostic> {
        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    if let Some(local_id) = self.lookup_var(ident.as_str()) {
                        return Ok((Place::local(local_id), current_block));
                    }
                }
                Err(DiagnosticBuilder::new(Severity::Error)
                    .message("invalid assignment target")
                    .span_label(self.span_to_diag(expr.span), "not a valid lvalue")
                    .build())
            }
            ExprKind::Field { expr: obj, field } => {
                let (base_place, block) = self.expr_to_place_mut(obj, current_block)?;
                // Resolve field index from type information
                let field_idx = match base_place.local {
                    local => self.resolve_field_index(local, field.name.as_str()),
                };
                Ok((base_place.with_field(field_idx), block))
            }
            ExprKind::Index { expr: array, index } => {
                // First, convert the base array to a place
                let (base_place, block1) = self.expr_to_place_mut(array, current_block)?;

                // Now handle the index expression
                // Strategy: Always evaluate the index to a temporary local
                // This handles both simple variables and complex expressions uniformly

                match &index.kind {
                    ExprKind::Path(path) => {
                        // Simple variable index - try direct lookup first for efficiency
                        if let Some(ident) = path.as_ident() {
                            if let Some(index_local) = self.lookup_var(ident.as_str()) {
                                return Ok((base_place.with_index(index_local), block1));
                            }
                        }
                        // If not found in var_map, evaluate it as an expression
                        // This handles module paths, associated constants, etc.
                        let index_temp = self.new_temp(MirType::Int);
                        let block2 = self.lower_expr(index, block1, Place::local(index_temp))?;
                        Ok((base_place.with_index(index_temp), block2))
                    }
                    ExprKind::Literal(lit) => {
                        // Constant index - could use ConstantIndex projection for optimization,
                        // but for consistency, evaluate to a temporary
                        match &lit.kind {
                            LiteralKind::Int(_int_lit) => {
                                // For integer literals, we could optimize by using ConstantIndex
                                // but for now, create a temp for consistency
                                let index_temp = self.new_temp(MirType::Int);
                                let block2 =
                                    self.lower_expr(index, block1, Place::local(index_temp))?;
                                Ok((base_place.with_index(index_temp), block2))
                            }
                            _ => Err(DiagnosticBuilder::new(Severity::Error)
                                .message("invalid index type")
                                .span_label(
                                    self.span_to_diag(index.span),
                                    "index must be an integer",
                                )
                                .build()),
                        }
                    }
                    _ => {
                        // Complex index expression (binary op, call, etc.)
                        // Evaluate it to a temporary and use that as the index
                        let index_temp = self.new_temp(MirType::Int);
                        let block2 = self.lower_expr(index, block1, Place::local(index_temp))?;

                        // Insert bounds check for safety
                        // Note: This is inserted during lowering for all array accesses
                        // The optimizer may remove redundant checks later
                        self.insert_bounds_check(
                            block2,
                            base_place.clone(),
                            Place::local(index_temp),
                        );

                        Ok((base_place.with_index(index_temp), block2))
                    }
                }
            }
            ExprKind::Unary {
                op: UnOp::Deref,
                expr: inner,
            } => {
                let (base_place, block) = self.expr_to_place_mut(inner, current_block)?;
                Ok((base_place.with_deref(), block))
            }
            _ => Err(DiagnosticBuilder::new(Severity::Error)
                .message("invalid assignment target")
                .span_label(
                    self.span_to_diag(expr.span),
                    "expression cannot be used as an lvalue",
                )
                .build()),
        }
    }

    /// Lower short-circuit boolean operations (&&, ||)
    fn lower_short_circuit(
        &mut self,
        left: &Expr,
        right: &Expr,
        current_block: BlockId,
        dest: Place,
        op: BinOp,
    ) -> Result<BlockId, Diagnostic> {
        let left_temp = self.new_temp(MirType::Bool);
        let block = self.lower_expr(left, current_block, Place::local(left_temp))?;

        let right_block = self.new_block();
        let short_circuit_block = self.new_block();
        let join_block = self.new_block();

        // Add phi node for the join block
        let phi_dest = self.new_temp(MirType::Bool);
        self.add_phi_node(join_block, phi_dest, MirType::Bool);

        match op {
            BinOp::And => {
                // if left is false, result is false; else evaluate right
                self.set_terminator(
                    block,
                    Terminator::Branch {
                        condition: Operand::Copy(Place::local(left_temp)),
                        then_block: right_block,
                        else_block: short_circuit_block,
                    },
                );
                // Short circuit: result is false
                self.push_statement(
                    short_circuit_block,
                    MirStatement::Assign(
                        dest.clone(),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(false))),
                    ),
                );
            }
            BinOp::Or => {
                // if left is true, result is true; else evaluate right
                self.set_terminator(
                    block,
                    Terminator::Branch {
                        condition: Operand::Copy(Place::local(left_temp)),
                        then_block: short_circuit_block,
                        else_block: right_block,
                    },
                );
                // Short circuit: result is true
                self.push_statement(
                    short_circuit_block,
                    MirStatement::Assign(
                        dest.clone(),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
            }
            _ => {
                // This branch should never be reached because lower_short_circuit
                // is only called for And/Or operators. Return an error for safety.
                return Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!(
                        "Internal compiler error: lower_short_circuit called with non-boolean operator {:?}",
                        op
                    ))
                    .build());
            }
        }

        self.set_terminator(short_circuit_block, Terminator::Goto(join_block));
        self.add_phi_operand(
            join_block,
            phi_dest,
            short_circuit_block,
            Operand::Copy(dest.clone()),
        );

        // Evaluate right side
        let right_exit = self.lower_expr(right, right_block, dest.clone())?;
        self.set_terminator(right_exit, Terminator::Goto(join_block));
        self.add_phi_operand(join_block, phi_dest, right_exit, Operand::Copy(dest));

        Ok(join_block)
    }

    /// Lower function call
    fn lower_call(
        &mut self,
        func: &Expr,
        args: &[Expr],
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Lower function expression
        let func_temp = self.new_temp(MirType::Infer);
        let mut block = self.lower_expr(func, current_block, Place::local(func_temp))?;

        // Lower arguments
        let mut arg_operands = List::new();
        for arg in args.iter() {
            let arg_temp = self.new_temp(MirType::Infer);
            block = self.lower_expr(arg, block, Place::local(arg_temp))?;
            arg_operands.push(Operand::Move(Place::local(arg_temp)));
        }

        // Create call terminator
        let success_block = self.new_block();
        let unwind_block = self.new_cleanup_block();

        self.set_terminator(
            block,
            Terminator::Call {
                destination: dest,
                func: Operand::Copy(Place::local(func_temp)),
                args: arg_operands,
                success_block,
                unwind_block,
            },
        );

        // Unwind block resumes unwinding
        self.set_terminator(unwind_block, Terminator::Resume);

        Ok(success_block)
    }

    /// Lower method call
    fn lower_method_call(
        &mut self,
        receiver: &Expr,
        method: &verum_ast::Ident,
        args: &[Expr],
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Lower receiver
        let recv_temp = self.new_temp(MirType::Infer);
        let mut block = self.lower_expr(receiver, current_block, Place::local(recv_temp))?;

        // Lower arguments (receiver is first argument)
        let mut arg_operands = List::new();
        arg_operands.push(Operand::Move(Place::local(recv_temp)));

        for arg in args.iter() {
            let arg_temp = self.new_temp(MirType::Infer);
            block = self.lower_expr(arg, block, Place::local(arg_temp))?;
            arg_operands.push(Operand::Move(Place::local(arg_temp)));
        }

        // Create call terminator
        let success_block = self.new_block();
        let unwind_block = self.new_cleanup_block();

        self.set_terminator(
            block,
            Terminator::Call {
                destination: dest,
                func: Operand::Constant(MirConstant::Function(method.name.clone().into())),
                args: arg_operands,
                success_block,
                unwind_block,
            },
        );

        self.set_terminator(unwind_block, Terminator::Resume);

        Ok(success_block)
    }

    /// Lower if expression
    fn lower_if(
        &mut self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &Block,
        else_branch: &Option<Box<Expr>>,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Lower condition
        let mut cond_block = current_block;
        let mut cond_temp = LocalId(0);

        for cond in condition.conditions.iter() {
            match cond {
                ConditionKind::Expr(expr) => {
                    cond_temp = self.new_temp(MirType::Bool);
                    cond_block = self.lower_expr(expr, cond_block, Place::local(cond_temp))?;
                }
                ConditionKind::Let { pattern, value } => {
                    // Let-binding condition (if let)
                    let val_temp = self.new_temp(MirType::Infer);
                    cond_block = self.lower_expr(value, cond_block, Place::local(val_temp))?;
                    cond_temp =
                        self.lower_pattern_binding(pattern, Place::local(val_temp), cond_block)?;
                }
            }
        }

        // Create branches
        let then_block = self.new_block();
        let else_block = self.new_block();
        let join_block = self.new_block();

        self.set_terminator(
            cond_block,
            Terminator::Branch {
                condition: Operand::Copy(Place::local(cond_temp)),
                then_block,
                else_block,
            },
        );

        // Lower then branch (use iterative to avoid stack overflow)
        self.push_scope();
        let then_exit = self.lower_block_iterative(then_branch, then_block, dest.clone())?;
        self.pop_scope(then_exit);
        self.set_terminator(then_exit, Terminator::Goto(join_block));

        // Lower else branch
        if let Some(else_expr) = else_branch.as_ref() {
            self.push_scope();
            let else_exit = self.lower_expr(else_expr.as_ref(), else_block, dest)?;
            self.pop_scope(else_exit);
            self.set_terminator(else_exit, Terminator::Goto(join_block));
        } else {
            // No else branch: assign unit
            self.push_statement(
                else_block,
                MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
            );
            self.set_terminator(else_block, Terminator::Goto(join_block));
        }

        Ok(join_block)
    }

    /// Lower match expression to switch and jump table
    fn lower_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[verum_ast::pattern::MatchArm],
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Lower scrutinee
        let scrut_temp = self.new_temp(MirType::Infer);
        let scrut_block = self.lower_expr(scrutinee, current_block, Place::local(scrut_temp))?;

        // Create blocks for each arm
        let join_block = self.new_block();
        let mut arm_blocks: Vec<BlockId> = Vec::new();

        for _ in arms.iter() {
            arm_blocks.push(self.new_block());
        }

        // Create otherwise block for exhaustiveness
        let otherwise_block = self.new_block();
        self.set_terminator(otherwise_block, Terminator::Unreachable);

        // Build jump table targets
        let mut targets = List::new();
        for (i, arm) in arms.iter().enumerate() {
            // Get discriminant value from pattern
            let discriminant = self.pattern_to_discriminant(&arm.pattern);
            if let Some(disc) = discriminant {
                targets.push((disc, arm_blocks[i]));
            }
        }

        // Get discriminant of scrutinee
        let discrim_temp = self.new_temp(MirType::Int);
        self.push_statement(
            scrut_block,
            MirStatement::Assign(
                Place::local(discrim_temp),
                Rvalue::Discriminant(Place::local(scrut_temp)),
            ),
        );

        // Emit switch
        self.set_terminator(
            scrut_block,
            Terminator::SwitchInt {
                discriminant: Operand::Copy(Place::local(discrim_temp)),
                targets,
                otherwise: otherwise_block,
            },
        );

        // Lower each arm
        for (i, arm) in arms.iter().enumerate() {
            self.push_scope();

            // Bind pattern variables
            let _ =
                self.lower_pattern_binding(&arm.pattern, Place::local(scrut_temp), arm_blocks[i]);

            // Check guard if present
            let arm_block = arm_blocks[i];
            let body_block = if let Some(guard) = arm.guard.as_ref() {
                // Lower guard
                let guard_temp = self.new_temp(MirType::Bool);
                let guard_block = self.lower_expr(
                    guard,
                    arm_block,
                    Place::local(guard_temp),
                )?;

                let guard_pass = self.new_block();
                let guard_fail = self.new_block();

                self.set_terminator(
                    guard_block,
                    Terminator::Branch {
                        condition: Operand::Copy(Place::local(guard_temp)),
                        then_block: guard_pass,
                        else_block: guard_fail,
                    },
                );

                // Failed guard goes to next arm or otherwise
                let next_arm = if i + 1 < arm_blocks.len() {
                    arm_blocks[i + 1]
                } else {
                    otherwise_block
                };
                self.set_terminator(guard_fail, Terminator::Goto(next_arm));

                guard_pass
            } else {
                arm_block
            };

            // Lower arm body
            let arm_exit = self.lower_expr(&arm.body, body_block, dest.clone())?;
            self.pop_scope(arm_exit);
            self.set_terminator(arm_exit, Terminator::Goto(join_block));
        }

        Ok(join_block)
    }

    /// Lower loop expression
    fn lower_loop(
        &mut self,
        body: &Block,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        let header_block = self.new_block();
        let body_block = self.new_block();
        let exit_block = self.new_block();

        // Create result local for break values
        let result_local = self.new_temp(MirType::Infer);

        // Jump to header
        self.set_terminator(current_block, Terminator::Goto(header_block));

        // Header jumps to body
        self.set_terminator(header_block, Terminator::Goto(body_block));

        // Push loop context
        self.loop_stack.push(LoopContext {
            header: header_block,
            exit: exit_block,
            continue_target: header_block,
            result: Some(result_local),
        });

        // Lower body (use iterative to avoid stack overflow)
        self.push_scope();
        let body_exit = self.lower_block_iterative(body, body_block, Place::local(result_local))?;
        self.pop_scope(body_exit);

        // Body loops back to header (back edge)
        self.set_terminator(body_exit, Terminator::Goto(header_block));

        // Pop loop context
        self.loop_stack.pop();

        // Copy result to destination at exit
        self.push_statement(
            exit_block,
            MirStatement::Assign(dest, Rvalue::Use(Operand::Move(Place::local(result_local)))),
        );

        Ok(exit_block)
    }

    /// Lower while loop
    fn lower_while(
        &mut self,
        condition: &Expr,
        body: &Block,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        let header_block = self.new_block();
        let body_block = self.new_block();
        let exit_block = self.new_block();

        // Jump to header
        self.set_terminator(current_block, Terminator::Goto(header_block));

        // Lower condition in header
        let cond_temp = self.new_temp(MirType::Bool);
        let cond_exit = self.lower_expr(condition, header_block, Place::local(cond_temp))?;

        self.set_terminator(
            cond_exit,
            Terminator::Branch {
                condition: Operand::Copy(Place::local(cond_temp)),
                then_block: body_block,
                else_block: exit_block,
            },
        );

        // Push loop context
        self.loop_stack.push(LoopContext {
            header: header_block,
            exit: exit_block,
            continue_target: header_block,
            result: None,
        });

        // Lower body (use iterative to avoid stack overflow)
        self.push_scope();
        let body_exit = self.lower_block_iterative(body, body_block, dest.clone())?;
        self.pop_scope(body_exit);
        self.set_terminator(body_exit, Terminator::Goto(header_block));

        // Pop loop context
        self.loop_stack.pop();

        // While loops return unit
        self.push_statement(
            exit_block,
            MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
        );

        Ok(exit_block)
    }

    /// Lower for loop (desugars to iterator pattern)
    fn lower_for(
        &mut self,
        pattern: &Pattern,
        iter_expr: &Expr,
        body: &Block,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // For loops are lowered to:
        // let mut iter = iter_expr.into_iter();
        // loop {
        //     match iter.next() {
        //         Some(pattern) => body,
        //         None => break,
        //     }
        // }

        // Create iterator
        let iter_temp = self.new_temp(MirType::Infer);
        let init_block = self.lower_expr(iter_expr, current_block, Place::local(iter_temp))?;

        let header_block = self.new_block();
        let body_block = self.new_block();
        let exit_block = self.new_block();

        self.set_terminator(init_block, Terminator::Goto(header_block));

        // Call iter.next()
        let next_result = self.new_temp(MirType::Infer);
        let next_block = self.new_block();
        let unwind_block = self.new_cleanup_block();

        self.set_terminator(
            header_block,
            Terminator::Call {
                destination: Place::local(next_result),
                func: Operand::Constant(MirConstant::Function("Iterator::next".into())),
                args: List::from(vec![Operand::Move(Place::local(iter_temp))]),
                success_block: next_block,
                unwind_block,
            },
        );
        self.set_terminator(unwind_block, Terminator::Resume);

        // Check if Some or None
        let discrim_temp = self.new_temp(MirType::Int);
        self.push_statement(
            next_block,
            MirStatement::Assign(
                Place::local(discrim_temp),
                Rvalue::Discriminant(Place::local(next_result)),
            ),
        );

        self.set_terminator(
            next_block,
            Terminator::SwitchInt {
                discriminant: Operand::Copy(Place::local(discrim_temp)),
                targets: List::from(vec![(0, body_block)]), // Some = 0
                otherwise: exit_block,                      // None = 1
            },
        );

        // Push loop context
        self.loop_stack.push(LoopContext {
            header: header_block,
            exit: exit_block,
            continue_target: header_block,
            result: None,
        });

        self.push_scope();

        // Bind pattern
        let _ = self.lower_pattern_binding(
            pattern,
            Place::local(next_result).with_downcast(0).with_field(0), // Some's inner value
            body_block,
        );

        // Lower body (use iterative to avoid stack overflow)
        let body_exit = self.lower_block_iterative(body, body_block, dest.clone())?;
        self.pop_scope(body_exit);
        self.set_terminator(body_exit, Terminator::Goto(header_block));

        self.loop_stack.pop();

        // For loops return unit
        self.push_statement(
            exit_block,
            MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
        );

        Ok(exit_block)
    }

    /// Lower `for await` loop (async iteration)
    ///
    /// Grammar: `for_await_loop = 'for' , 'await' , pattern , 'in' , expression , { loop_annotation } , block_expr`
    ///
    /// For await loops iterate over AsyncIterator types. They are lowered to:
    /// ```text
    /// let mut iter = async_iterable.into_async_iter();  // or assume already AsyncIterator
    /// loop {
    ///     let next_future = iter.next();
    ///     let next_result = next_future.await;         // suspension point
    ///     match next_result {
    ///         Some(value) => { bind pattern to value; body }
    ///         None => break
    ///     }
    /// }
    /// ```
    ///
    /// This generates a state machine that:
    /// 1. Creates the async iterator from the iterable expression
    /// 2. Enters a loop that calls next() and awaits the result
    /// 3. Pattern matches on Some/None to either execute body or exit
    /// 4. Properly handles break/continue with async cleanup
    fn lower_for_await(
        &mut self,
        pattern: &Pattern,
        async_iterable: &Expr,
        body: &Block,
        current_block: BlockId,
        dest: Place,
    ) -> Result<BlockId, Diagnostic> {
        // Step 1: Lower the async iterable expression and create the async iterator
        //
        // We assume the async_iterable is already an AsyncIterator or implements IntoAsyncIterator.
        // In a full implementation, we would call .into_async_iter(), but for MIR purposes
        // we treat the iterable as the iterator directly (type checking ensures correctness).
        let iter_temp = self.new_temp(MirType::Infer);
        let init_block = self.lower_expr(async_iterable, current_block, Place::local(iter_temp))?;

        // Step 2: Create CFG blocks for the async iteration state machine
        //
        // Block layout:
        //   init_block -> header_block: Start calling next()
        //   header_block -> call_next_block: Call iter.next()
        //   call_next_block -> await_block: Await the future
        //   await_block -> resume_block: Resume after await
        //   resume_block -> match_block: Check Some/None
        //   match_block -> body_block (Some) or exit_block (None)
        //   body_block -> header_block: Loop back
        //   exit_block: Loop exit, continue to next code
        let header_block = self.new_block();
        let call_next_block = self.new_block();
        let await_resume_block = self.new_block();
        let match_block = self.new_block();
        let body_block = self.new_block();
        let exit_block = self.new_block();
        let unwind_block = self.new_cleanup_block();

        // Transition from init to header
        self.set_terminator(init_block, Terminator::Goto(header_block));

        // Step 3: In header block, call iter.next() to get a Future<Maybe<Item>>
        //
        // The next() method returns a future that we need to await.
        let next_future_temp = self.new_temp(MirType::Infer);

        // header_block: goto call_next_block (simple transition)
        self.set_terminator(header_block, Terminator::Goto(call_next_block));

        // call_next_block: Call AsyncIterator::next(&mut iter)
        // This is a non-async call that returns a Future
        self.set_terminator(
            call_next_block,
            Terminator::Call {
                destination: Place::local(next_future_temp),
                func: Operand::Constant(MirConstant::Function("AsyncIterator::next".into())),
                args: List::from(vec![Operand::Move(Place::local(iter_temp))]),
                success_block: await_resume_block,
                unwind_block,
            },
        );

        // Step 4: Await the future to get the actual Maybe<Item> result
        //
        // This is the suspension point where the async function yields control.
        // When the future completes, execution resumes at await_resume_block with
        // the result written to next_result_temp.
        let next_result_temp = self.new_temp(MirType::Infer);

        // Create a dedicated block for the await operation
        let await_point_block = self.new_block();
        self.set_terminator(await_resume_block, Terminator::Goto(await_point_block));

        // Set up the await terminator - this is where the coroutine suspends
        self.set_terminator(
            await_point_block,
            Terminator::Await {
                future: Place::local(next_future_temp),
                destination: Place::local(next_result_temp),
                resume_block: match_block,
                unwind_block,
            },
        );

        // Step 5: Match on the result - Some(value) continues, None exits
        //
        // The Maybe<T> type has:
        //   - Variant 0: Some(T) - contains the next item
        //   - Variant 1: None - iterator exhausted
        let discrim_temp = self.new_temp(MirType::Int);
        self.push_statement(
            match_block,
            MirStatement::Assign(
                Place::local(discrim_temp),
                Rvalue::Discriminant(Place::local(next_result_temp)),
            ),
        );

        // SwitchInt on discriminant: Some(0) -> body, None(1) -> exit
        self.set_terminator(
            match_block,
            Terminator::SwitchInt {
                discriminant: Operand::Copy(Place::local(discrim_temp)),
                targets: List::from(vec![(0, body_block)]), // Some = discriminant 0
                otherwise: exit_block,                      // None = discriminant 1 (or any other)
            },
        );

        // Step 6: Set up loop context for break/continue handling
        //
        // For async iteration:
        // - break: jumps to exit_block (with optional value handling)
        // - continue: jumps to header_block to start next iteration
        self.loop_stack.push(LoopContext {
            header: header_block,
            exit: exit_block,
            continue_target: header_block,
            result: None,
        });

        // Step 7: Enter body scope and bind the pattern
        //
        // Extract the value from Some(value) and bind it to the pattern.
        // The value is at: next_result_temp.downcast(0).field(0)
        self.push_scope();

        let _ = self.lower_pattern_binding(
            pattern,
            Place::local(next_result_temp)
                .with_downcast(0)
                .with_field(0), // Some variant's inner value
            body_block,
        );

        // Step 8: Lower the loop body
        //
        // Use iterative lowering to avoid stack overflow on deeply nested bodies.
        let body_exit = self.lower_block_iterative(body, body_block, dest.clone())?;

        // Step 9: Clean up body scope and loop back to header
        self.pop_scope(body_exit);
        self.set_terminator(body_exit, Terminator::Goto(header_block));

        // Step 10: Pop loop context
        self.loop_stack.pop();

        // Step 11: Set up unwind block for async cleanup
        //
        // If an error occurs during iteration (including in the await),
        // we need to properly unwind.
        self.set_terminator(unwind_block, Terminator::Resume);

        // Step 12: For await loops return unit (like regular for loops)
        //
        // The loop itself doesn't produce a value; any values are produced
        // through side effects in the body.
        self.push_statement(
            exit_block,
            MirStatement::Assign(dest, Rvalue::Use(Operand::Constant(MirConstant::Unit))),
        );

        Ok(exit_block)
    }

    /// Lower statement
    fn lower_stmt(&mut self, stmt: &Stmt, current_block: BlockId) -> Result<BlockId, Diagnostic> {
        match &stmt.kind {
            StmtKind::Let { pattern, ty, value } => {
                // Create local for binding
                let local_ty = match ty.as_ref() {
                    Some(t) => self.lower_type(t),
                    None => MirType::Infer,
                };

                if let PatternKind::Ident {
                    name, mutable: _, ..
                } = &pattern.kind
                {
                    let local = self.new_local(name.name.clone(), local_ty, LocalKind::Var);

                    // Mark storage live
                    self.push_statement(current_block, MirStatement::StorageLive(local));

                    // Debug info
                    self.push_statement(
                        current_block,
                        MirStatement::DebugVar {
                            local,
                            name: name.name.clone().into(),
                        },
                    );

                    // Initialize if value provided
                    if let Some(val_expr) = value {
                        return self.lower_expr(val_expr, current_block, Place::local(local));
                    }
                } else {
                    // Complex pattern: lower value then destructure
                    if let Some(val_expr) = value {
                        let temp = self.new_temp(local_ty);
                        self.push_statement(current_block, MirStatement::StorageLive(temp));
                        let block = self.lower_expr(val_expr, current_block, Place::local(temp))?;
                        let _ = self.lower_pattern_binding(pattern, Place::local(temp), block);
                        return Ok(block);
                    }
                }

                Ok(current_block)
            }

            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                // let pattern = value else { diverging };
                let local_ty = match ty.as_ref() {
                    Some(t) => self.lower_type(t),
                    None => MirType::Infer,
                };

                let temp = self.new_temp(local_ty);
                self.push_statement(current_block, MirStatement::StorageLive(temp));
                let block = self.lower_expr(value, current_block, Place::local(temp))?;

                // Check if pattern matches
                let match_temp = self.lower_pattern_binding(pattern, Place::local(temp), block)?;

                let success_block = self.new_block();
                let fail_block = self.new_block();

                self.set_terminator(
                    block,
                    Terminator::Branch {
                        condition: Operand::Copy(Place::local(match_temp)),
                        then_block: success_block,
                        else_block: fail_block,
                    },
                );

                // Lower else block (must diverge) - use iterative to avoid stack overflow
                let else_temp = self.new_temp(MirType::Never);
                let _ = self.lower_block_iterative(else_block, fail_block, Place::local(else_temp));

                Ok(success_block)
            }

            StmtKind::Expr { expr, has_semi: _ } => {
                let temp = self.new_temp(MirType::Infer);
                self.lower_expr(expr, current_block, Place::local(temp))
            }

            StmtKind::Defer(expr) => {
                // Defer: register cleanup block
                let cleanup_block = self.new_cleanup_block();
                self.defer_stack.push(cleanup_block);

                let temp = self.new_temp(MirType::Infer);
                let _ = self.lower_expr(expr, cleanup_block, Place::local(temp));

                // The cleanup block will be executed later
                self.push_statement(current_block, MirStatement::DeferCleanup { cleanup_block });

                Ok(current_block)
            }

            StmtKind::Errdefer(expr) => {
                // Errdefer: register cleanup block that only executes on error path
                // Note: The error-path-only semantics are tracked separately during
                // lowering to the next IR level. At MIR level, we use DeferCleanup
                // but the errdefer_stack tracks which cleanups are errdefer vs defer.
                let cleanup_block = self.new_cleanup_block();
                self.defer_stack.push(cleanup_block);

                let temp = self.new_temp(MirType::Infer);
                let _ = self.lower_expr(expr, cleanup_block, Place::local(temp));

                // Register cleanup (errdefer semantics handled by scope cleanup logic)
                self.push_statement(current_block, MirStatement::DeferCleanup { cleanup_block });

                Ok(current_block)
            }

            StmtKind::Provide { context, value, .. } => {
                // Context provision: evaluate value and register with context system
                // Level 2 dynamic contexts: runtime dependency injection via task-local storage.

                // 1. Evaluate the context value expression
                let value_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(value, current_block, Place::local(value_temp))?;

                // 2. Emit ContextProvide statement to register context
                let context_name: Text = context.clone().into();
                self.push_statement(
                    block,
                    MirStatement::ContextProvide {
                        context_name: context_name.clone(),
                        value: Place::local(value_temp),
                    },
                );

                // 3. Create cleanup block to unprovide context at scope exit
                let cleanup_block = self.new_block();
                self.push_statement(
                    cleanup_block,
                    MirStatement::ContextUnprovide { context_name },
                );
                self.defer_stack.push(cleanup_block);

                // 4. Register deferred cleanup for proper scope management
                self.push_statement(block, MirStatement::DeferCleanup { cleanup_block });

                Ok(block)
            }

            StmtKind::ProvideScope {
                context,
                value,
                block: scope_block,
                ..
            } => {
                // Block-scoped context provision
                // Similar to Provide but with explicit scope handling
                // Block-scoped provide: context available only within the provide block.

                // 1. Evaluate the context value expression
                let value_temp = self.new_temp(MirType::Infer);
                let block = self.lower_expr(value, current_block, Place::local(value_temp))?;

                // 2. Emit ContextProvide statement to register context
                let context_name: Text = context.clone().into();
                self.push_statement(
                    block,
                    MirStatement::ContextProvide {
                        context_name: context_name.clone(),
                        value: Place::local(value_temp),
                    },
                );

                // 3. Lower the block expression (context is in scope)
                let block_result = self.new_temp(MirType::Infer);
                let after_block =
                    self.lower_expr(scope_block, block, Place::local(block_result))?;

                // 4. Emit ContextUnprovide after block completes
                self.push_statement(
                    after_block,
                    MirStatement::ContextUnprovide { context_name },
                );

                Ok(after_block)
            }

            StmtKind::Empty => Ok(current_block),

            StmtKind::Item(_) => {
                // Item declarations in blocks are handled separately
                Ok(current_block)
            }
        }
    }

    /// Lower pattern binding, returns a boolean local indicating match success
    fn lower_pattern_binding(
        &mut self,
        pattern: &Pattern,
        source: Place,
        block: BlockId,
    ) -> Result<LocalId, Diagnostic> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                // Wildcard matches everything
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::Ident {
                name, mutable: _, ..
            } => {
                // Bind name to source
                let local = self.new_local(name.name.clone(), MirType::Infer, LocalKind::Var);
                self.push_statement(block, MirStatement::StorageLive(local));
                self.push_statement(
                    block,
                    MirStatement::Assign(Place::local(local), Rvalue::Use(Operand::Copy(source))),
                );
                self.var_map.insert(name.name.clone().into(), local);

                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::Literal(lit) => {
                // Compare with literal
                let lit_temp = self.new_temp(MirType::Infer);
                let constant = self.lower_literal_to_constant(lit);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(lit_temp),
                        Rvalue::Use(Operand::Constant(constant)),
                    ),
                );

                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(source),
                            Operand::Copy(Place::local(lit_temp)),
                        ),
                    ),
                );
                Ok(result)
            }

            PatternKind::Tuple(patterns) => {
                // Destructure tuple
                let mut all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(all_match),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                for (i, pat) in patterns.iter().enumerate() {
                    let field_place = source.clone().with_field(i);
                    let match_result = self.lower_pattern_binding(pat, field_place, block)?;

                    // all_match = all_match && match_result
                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                Ok(all_match)
            }

            PatternKind::Variant { path, data } => {
                // Resolve expected discriminant from type information
                let expected_disc = self.resolve_variant_discriminant(path);

                // Get actual discriminant
                let disc_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(disc_temp),
                        Rvalue::Discriminant(source.clone()),
                    ),
                );

                // Compare discriminants
                let disc_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(disc_match),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(Place::local(disc_temp)),
                            Operand::Constant(MirConstant::Int(expected_disc)),
                        ),
                    ),
                );

                // If discriminant matches, bind inner patterns
                if let Some(data) = data {
                    match data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            let variant_place = source.with_downcast(expected_disc as usize);
                            for (i, pat) in patterns.iter().enumerate() {
                                let _ = self.lower_pattern_binding(
                                    pat,
                                    variant_place.clone().with_field(i),
                                    block,
                                )?;
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            let variant_place = source.with_downcast(expected_disc as usize);
                            for (i, field) in fields.iter().enumerate() {
                                if let Some(pat) = &field.pattern {
                                    let _ = self.lower_pattern_binding(
                                        pat,
                                        variant_place.clone().with_field(i),
                                        block,
                                    )?;
                                }
                            }
                        }
                    }
                }

                Ok(disc_match)
            }

            PatternKind::Or(patterns) => {
                // Try each alternative
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(false))),
                    ),
                );

                for pat in patterns.iter() {
                    let pat_match = self.lower_pattern_binding(pat, source.clone(), block)?;
                    let new_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_result),
                            Rvalue::Binary(
                                BinOp::Or,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(pat_match)),
                            ),
                        ),
                    );
                }

                Ok(result)
            }

            PatternKind::Rest => {
                // Rest pattern (..) - matches any remaining elements
                // In a slice context, this captures remaining elements
                // Always succeeds as a match
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::Array(patterns) => {
                // Array pattern: [a, b, c]
                // Similar to tuple but for arrays
                let mut all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(all_match),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                for (i, pat) in patterns.iter().enumerate() {
                    // For constant indices, use field projection
                    let elem_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(elem_temp),
                            Rvalue::Use(Operand::Copy(source.clone().with_field(i))),
                        ),
                    );

                    let match_result =
                        self.lower_pattern_binding(pat, Place::local(elem_temp), block)?;

                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                Ok(all_match)
            }

            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                // Slice pattern: [a, .., b] or [a, rest @ .., b]
                // Matches arrays/slices with patterns at beginning and end
                let mut all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(all_match),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                // Get length of source
                let len_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(Place::local(len_temp), Rvalue::Len(source.clone())),
                );

                // Check minimum length requirement
                let min_len = before.len() + after.len();
                let min_len_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(min_len_temp),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(min_len as i64))),
                    ),
                );

                let len_check = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(len_check),
                        Rvalue::Binary(
                            BinOp::Ge,
                            Operand::Copy(Place::local(len_temp)),
                            Operand::Copy(Place::local(min_len_temp)),
                        ),
                    ),
                );

                let new_all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(new_all_match),
                        Rvalue::Binary(
                            BinOp::And,
                            Operand::Copy(Place::local(all_match)),
                            Operand::Copy(Place::local(len_check)),
                        ),
                    ),
                );
                all_match = new_all_match;

                // Match 'before' patterns from the start
                for (i, pat) in before.iter().enumerate() {
                    let elem_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(elem_temp),
                            Rvalue::Use(Operand::Copy(source.clone().with_field(i))),
                        ),
                    );

                    let match_result =
                        self.lower_pattern_binding(pat, Place::local(elem_temp), block)?;

                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                // Match 'after' patterns from the end
                for (i, pat) in after.iter().enumerate() {
                    // Calculate index from end: len - after.len() + i
                    let offset = after.len() - i;
                    let idx_temp = self.new_temp(MirType::Int);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(idx_temp),
                            Rvalue::Binary(
                                BinOp::Sub,
                                Operand::Copy(Place::local(len_temp)),
                                Operand::Constant(MirConstant::Int(offset as i64)),
                            ),
                        ),
                    );

                    let elem_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(elem_temp),
                            Rvalue::Use(Operand::Copy(source.clone().with_index(idx_temp))),
                        ),
                    );

                    let match_result =
                        self.lower_pattern_binding(pat, Place::local(elem_temp), block)?;

                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                // Handle rest pattern if present (binds middle elements)
                if let Some(rest_pattern) = rest.as_ref() {
                    // Create subslice for middle elements
                    let subslice_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(subslice_temp),
                            Rvalue::Use(Operand::Copy(source.clone())),
                        ),
                    );

                    let _ = self.lower_pattern_binding(
                        rest_pattern,
                        Place::local(subslice_temp),
                        block,
                    )?;
                }

                Ok(all_match)
            }

            PatternKind::Record {
                path,
                fields,
                rest: _,
            } => {
                // Record pattern: Point { x, y } or Point { x: px, y: py }
                let mut all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(all_match),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                // Get struct name for field resolution
                let struct_name: Text = path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(".")
                    .into();

                // Match each field
                for field in fields.iter() {
                    let field_name = field.name.name.as_str();
                    let field_idx = self
                        .type_registry
                        .get_field_index(struct_name.as_str(), field_name)
                        .unwrap_or(0);

                    let field_place = source.clone().with_field(field_idx);

                    // If pattern is provided, match against it; otherwise use shorthand (bind to field name)
                    let match_result = if let Some(pat) = &field.pattern {
                        self.lower_pattern_binding(pat, field_place, block)?
                    } else {
                        // Shorthand: { x } means bind x to the field
                        let local =
                            self.new_local(field_name.to_string(), MirType::Infer, LocalKind::Var);
                        self.push_statement(block, MirStatement::StorageLive(local));
                        self.push_statement(
                            block,
                            MirStatement::Assign(
                                Place::local(local),
                                Rvalue::Use(Operand::Copy(field_place)),
                            ),
                        );
                        self.var_map.insert(field_name.into(), local);

                        let result = self.new_temp(MirType::Bool);
                        self.push_statement(
                            block,
                            MirStatement::Assign(
                                Place::local(result),
                                Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                            ),
                        );
                        result
                    };

                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                Ok(all_match)
            }

            PatternKind::Reference { mutable, inner } => {
                // Reference pattern: &x or &mut x
                // Dereference the source and match inner pattern
                let deref_place = source.with_deref();

                // Check mutability if required
                if *mutable {
                    // For &mut patterns, we need mutable access
                    self.insert_capability_check(block, deref_place.clone(), Capability::Write);
                }

                self.lower_pattern_binding(inner, deref_place, block)
            }

            PatternKind::Range {
                start,
                end,
                inclusive,
            } => {
                // Range pattern: 1..10 or 1..=10
                let result = self.new_temp(MirType::Bool);

                let mut conds = Vec::new();

                // Check start bound if present
                if let Some(start_lit) = start.as_ref() {
                    let start_temp = self.new_temp(MirType::Infer);
                    let constant = match &start_lit.kind {
                        LiteralKind::Int(i) => MirConstant::Int(i.value as i64),
                        LiteralKind::Char(c) => MirConstant::Char(*c),
                        _ => MirConstant::Undef,
                    };
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(start_temp),
                            Rvalue::Use(Operand::Constant(constant)),
                        ),
                    );

                    let ge_check = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(ge_check),
                            Rvalue::Binary(
                                BinOp::Ge,
                                Operand::Copy(source.clone()),
                                Operand::Copy(Place::local(start_temp)),
                            ),
                        ),
                    );
                    conds.push(ge_check);
                }

                // Check end bound if present
                if let Some(end_lit) = end.as_ref() {
                    let end_temp = self.new_temp(MirType::Infer);
                    let constant = match &end_lit.kind {
                        LiteralKind::Int(i) => MirConstant::Int(i.value as i64),
                        LiteralKind::Char(c) => MirConstant::Char(*c),
                        _ => MirConstant::Undef,
                    };
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(end_temp),
                            Rvalue::Use(Operand::Constant(constant)),
                        ),
                    );

                    let cmp_op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                    let le_check = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(le_check),
                            Rvalue::Binary(
                                cmp_op,
                                Operand::Copy(source.clone()),
                                Operand::Copy(Place::local(end_temp)),
                            ),
                        ),
                    );
                    conds.push(le_check);
                }

                // Combine conditions with AND
                if conds.is_empty() {
                    // No bounds - always matches
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                        ),
                    );
                } else if conds.len() == 1 {
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(conds[0]))),
                        ),
                    );
                } else {
                    // Combine with AND
                    let combined = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(combined),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(conds[0])),
                                Operand::Copy(Place::local(conds[1])),
                            ),
                        ),
                    );
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(combined))),
                        ),
                    );
                }

                Ok(result)
            }

            PatternKind::Paren(inner) => {
                // Parenthesized pattern: unwrap and match inner
                self.lower_pattern_binding(inner, source, block)
            }            PatternKind::View {
                view_function,
                pattern: inner,
            } => {
                // View pattern: match on result of applying view function
                // Example: parity(n) @ Even(_) => ...
                let view_temp = self.new_temp(MirType::Infer);

                // Lower the view function call
                let success_block = self.new_block();
                let unwind_block = self.new_cleanup_block();

                // Get source value for the view function argument
                let source_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(source_temp),
                        Rvalue::Use(Operand::Copy(source)),
                    ),
                );

                // Lower view function expression
                let _ = view_function;

                // Call view function
                self.set_terminator(
                    block,
                    Terminator::Call {
                        destination: Place::local(view_temp),
                        func: Operand::Constant(MirConstant::Function("view".into())),
                        args: List::from(vec![Operand::Move(Place::local(source_temp))]),
                        success_block,
                        unwind_block,
                    },
                );

                self.set_terminator(unwind_block, Terminator::Resume);

                // Match inner pattern on view result
                self.lower_pattern_binding(inner, Place::local(view_temp), success_block)
            }

            PatternKind::Active { name, params, bindings } => {
                // Active pattern: call the pattern function and branch on its result.
                //
                // Total patterns (e.g., Even()) return Bool:
                //   call active_pattern_fn(source, params...) -> Bool
                //   result = call_result
                //
                // Partial/extraction patterns (e.g., ParseInt(n)) return Maybe<T>:
                //   call active_pattern_fn(source, params...) -> Maybe<T>
                //   if Some(val): bind val to bindings, result = true
                //   if None: result = false

                let result = self.new_temp(MirType::Bool);

                // Build arguments: source value + any pattern parameters
                let source_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(source_temp),
                        Rvalue::Use(Operand::Copy(source.clone())),
                    ),
                );

                let mut call_args = vec![Operand::Copy(Place::local(source_temp))];

                // Lower each parameter expression and add to args
                let mut param_block = block;
                for param_expr in params.iter() {
                    let param_temp = self.new_temp(MirType::Infer);
                    param_block = self.lower_expr(param_expr, param_block, Place::local(param_temp))?;
                    call_args.push(Operand::Copy(Place::local(param_temp)));
                }

                // Call the active pattern function
                let call_result = self.new_temp(MirType::Infer);
                let call_success = self.new_block();
                let call_unwind = self.new_cleanup_block();

                // The pattern function name is the active pattern's name
                let func_name: Text = format!("active_pattern::{}", name.as_str()).into();

                self.set_terminator(
                    param_block,
                    Terminator::Call {
                        destination: Place::local(call_result),
                        func: Operand::Constant(MirConstant::Function(func_name)),
                        args: List::from(call_args),
                        success_block: call_success,
                        unwind_block: call_unwind,
                    },
                );
                self.set_terminator(call_unwind, Terminator::Resume);

                if bindings.is_empty() {
                    // Total pattern: result is a Bool directly from the call
                    self.push_statement(
                        call_success,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(call_result))),
                        ),
                    );
                } else {
                    // Partial/extraction pattern: result is Maybe<T>
                    // Check discriminant: Some = 1, None = 0
                    let discrim = self.new_temp(MirType::Int);
                    self.push_statement(
                        call_success,
                        MirStatement::Assign(
                            Place::local(discrim),
                            Rvalue::Discriminant(Place::local(call_result)),
                        ),
                    );

                    let some_block = self.new_block();
                    let none_block = self.new_block();
                    let pat_join = self.new_block();

                    // Branch: discriminant == 1 (Some) -> some_block, else -> none_block
                    self.set_terminator(
                        call_success,
                        Terminator::SwitchInt {
                            discriminant: Operand::Copy(Place::local(discrim)),
                            targets: List::from(vec![(1, some_block)]),
                            otherwise: none_block,
                        },
                    );

                    // Some branch: extract inner value and bind to patterns
                    // Downcast to Some variant (index 1), then access field 0 for the inner value
                    let inner_val = self.new_temp(MirType::Infer);
                    self.push_statement(
                        some_block,
                        MirStatement::Assign(
                            Place::local(inner_val),
                            Rvalue::Use(Operand::Copy(
                                Place::local(call_result).with_downcast(1).with_field(0),
                            )),
                        ),
                    );

                    // Bind inner value to each binding pattern
                    let bind_block = some_block;
                    for binding in bindings.iter() {
                        let _bind_result = self.lower_pattern_binding(
                            binding,
                            Place::local(inner_val),
                            bind_block,
                        )?;
                    }

                    // Some => result = true
                    self.push_statement(
                        bind_block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                        ),
                    );
                    self.set_terminator(bind_block, Terminator::Goto(pat_join));

                    // None => result = false
                    self.push_statement(
                        none_block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Constant(MirConstant::Bool(false))),
                        ),
                    );
                    self.set_terminator(none_block, Terminator::Goto(pat_join));

                    // Continue from join block — but we need to return the result local
                    // The join block is where subsequent code continues; however,
                    // lower_pattern_binding returns a LocalId (the bool result), and the
                    // caller handles control flow. We store result in pat_join too.
                    self.push_statement(
                        pat_join,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(result))),
                        ),
                    );
                }

                Ok(result)
            }

            PatternKind::And(patterns) => {
                // And pattern: bind variables from all sub-patterns
                // All patterns match the same source value
                // Each sub-pattern is checked, result is conjunction of all
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                for pat in patterns.iter() {
                    let pat_result = self.lower_pattern_binding(pat, source.clone(), block)?;
                    // result = result && pat_result
                    let and_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(and_result),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(pat_result)),
                            ),
                        ),
                    );
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(and_result))),
                        ),
                    );
                }

                Ok(result)
            }

            PatternKind::TypeTest { binding, test_type } => {
                // TypeTest pattern: `x is Type` - runtime type check with binding
                // Spec: Type test patterns narrow unknown/any types at runtime
                //
                // Strategy:
                // 1. Perform runtime type check against test_type
                // 2. If successful, cast source to test_type and bind to variable
                // 3. Return boolean indicating match success
                //
                // The MIR uses a discriminant check against the type tag, similar
                // to how variant patterns work. For concrete types, we generate
                // a type ID comparison based on the type name string.

                // Get the type name for discriminant lookup
                let type_name = format!("{:?}", test_type.kind);
                let target_type = MirType::Named(type_name.clone().into());

                // Create result boolean for the type test
                let result = self.new_temp(MirType::Bool);

                // Get the runtime type tag of the source value
                let type_tag = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(Place::local(type_tag), Rvalue::Discriminant(source.clone())),
                );

                // Get the expected type tag for the test type by treating type name as a path
                let type_path = verum_ast::ty::Path::single(verum_ast::Ident {
                    name: type_name.clone().into(),
                    span: verum_ast::Span::default(),
                });
                let expected_tag = self.resolve_variant_discriminant(&type_path);
                let expected_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(expected_temp),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(expected_tag))),
                    ),
                );

                // Compare type tags
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(Place::local(type_tag)),
                            Operand::Copy(Place::local(expected_temp)),
                        ),
                    ),
                );

                // Create the binding variable with the narrowed type
                let local = self.new_local(binding.name.clone(), target_type.clone(), LocalKind::Var);
                self.push_statement(block, MirStatement::StorageLive(local));

                // Cast the source to the target type and assign to binding
                // This cast is safe because we've already checked the type
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(local),
                        Rvalue::Cast(CastKind::Transmute, Operand::Copy(source), target_type),
                    ),
                );

                self.var_map.insert(binding.name.clone().into(), local);

                Ok(result)
            }

            PatternKind::Stream { head_patterns, rest } => {
                // Stream pattern: stream[first, second, ...rest]
                // Matches elements from an iterator/generator and optionally binds the rest.
                //
                // For pattern binding:
                // 1. Consume head elements from iterator and bind to pattern variables
                // 2. If rest is specified, bind remaining iterator to rest variable
                //
                // Stream pattern matching: destructuring stream values in match arms.

                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                // Consume and bind head elements from the iterator
                for pat in head_patterns.iter() {
                    // Get next element from iterator (we assume match test already verified it exists)
                    let elem_temp = self.new_temp(MirType::Infer);

                    // For MIR, we represent iterator next as a call-like operation
                    // The runtime will handle actual iterator protocol
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(elem_temp),
                            Rvalue::Use(Operand::Copy(source.clone())),
                        ),
                    );

                    // Recursively bind the element pattern
                    let pat_result = self.lower_pattern_binding(pat, Place::local(elem_temp), block)?;

                    // Combine with result: result = result && pat_result
                    let and_temp = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(and_temp),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(pat_result)),
                            ),
                        ),
                    );
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(result),
                            Rvalue::Use(Operand::Copy(Place::local(and_temp))),
                        ),
                    );
                }

                // Bind rest iterator if specified
                if let verum_common::Maybe::Some(rest_ident) = rest {
                    // The remaining iterator is bound to the rest variable
                    // After consuming head elements, the iterator position is advanced
                    let local = self.new_local(
                        rest_ident.name.clone(),
                        MirType::Infer,
                        LocalKind::Var,
                    );
                    self.push_statement(block, MirStatement::StorageLive(local));
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(local),
                            Rvalue::Use(Operand::Copy(source)),
                        ),
                    );
                    self.var_map.insert(rest_ident.name.clone().into(), local);
                }

                Ok(result)
            }

            PatternKind::Guard { pattern, guard } => {
                // Guard pattern: (pattern if guard_expr)
                // Spec: Rust RFC 3637 - Guard Patterns
                //
                // Strategy:
                // 1. Lower the inner pattern binding
                // 2. Evaluate the guard expression
                // 3. Result is: inner_matches AND guard_is_true

                // Lower inner pattern
                let inner_result = self.lower_pattern_binding(pattern, source.clone(), block)?;

                // Evaluate guard expression into a temporary
                let guard_temp = self.new_temp(MirType::Bool);
                let _ = self.lower_expr(guard, block, Place::local(guard_temp))?;

                // Combine with AND: inner_result && guard_result
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Binary(
                            BinOp::And,
                            Operand::Copy(Place::local(inner_result)),
                            Operand::Copy(Place::local(guard_temp)),
                        ),
                    ),
                );

                Ok(result)
            }

            PatternKind::Cons { head, tail } => {
                // Cons pattern: head :: tail
                let _ = self.lower_pattern_binding(head, source.clone(), block)?;
                let result = self.lower_pattern_binding(tail, source, block)?;
                Ok(result)
            }
        }
    }

    /// Lower compound destructuring assignment: `(a, b) += (da, db)`
    ///
    /// Unlike simple destructuring assignment which binds new variables,
    /// compound destructuring updates existing variables in place.
    ///
    /// For `(a, b) += (da, db)`:
    /// 1. Load current values of a and b
    /// 2. Get corresponding elements from the RHS
    /// 3. Perform the binary operation (a + da, b + db)
    /// 4. Store results back to a and b
    fn lower_compound_destructuring(
        &mut self,
        pattern: &Pattern,
        source: Place,
        op: &verum_ast::BinOp,
        block: BlockId,
    ) -> Result<(), Diagnostic> {
        let bin_op = self.compound_to_binary(op);

        match &pattern.kind {
            PatternKind::Tuple(patterns) => {
                // For each element in the tuple pattern, perform compound operation
                for (i, pat) in patterns.iter().enumerate() {
                    let field_source = source.clone().with_field(i);
                    self.lower_compound_destructuring_element(pat, field_source, &bin_op, block)?;
                }
                Ok(())
            }

            PatternKind::Array(patterns) => {
                // For each element in the array pattern, perform compound operation
                for (i, pat) in patterns.iter().enumerate() {
                    let elem_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(elem_temp),
                            Rvalue::Use(Operand::Copy(source.clone().with_field(i))),
                        ),
                    );
                    self.lower_compound_destructuring_element(pat, Place::local(elem_temp), &bin_op, block)?;
                }
                Ok(())
            }

            PatternKind::Paren(inner) => {
                // Unwrap parenthesized pattern
                self.lower_compound_destructuring(inner, source, op, block)
            }

            PatternKind::Ident { name: _name, .. } => {
                // Single identifier - treat as simple compound assignment
                self.lower_compound_destructuring_element(pattern, source, &bin_op, block)
            }

            _ => Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "compound destructuring assignment not supported for pattern kind: {:?}",
                    pattern.kind
                ))
                .build()),
        }
    }

    /// Lower a single element of compound destructuring
    ///
    /// For an identifier `a` with RHS value `da` and operation `+`:
    /// 1. Load current value of `a`
    /// 2. Compute `a + da`
    /// 3. Store result back to `a`
    fn lower_compound_destructuring_element(
        &mut self,
        pattern: &Pattern,
        rhs_value: Place,
        bin_op: &BinOp,
        block: BlockId,
    ) -> Result<(), Diagnostic> {
        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                // Look up the existing variable
                let var_local = self.lookup_var(name.name.as_str()).ok_or_else(|| {
                    DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "undefined variable '{}' in compound destructuring assignment",
                            name.name
                        ))
                        .build()
                })?;

                let var_place = Place::local(var_local);

                // Load current value
                let current_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(current_temp),
                        Rvalue::Use(Operand::Copy(var_place.clone())),
                    ),
                );

                // Load RHS value
                let rhs_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(rhs_temp),
                        Rvalue::Use(Operand::Copy(rhs_value)),
                    ),
                );

                // Perform binary operation
                let result_temp = self.new_temp(MirType::Infer);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result_temp),
                        Rvalue::Binary(
                            *bin_op,
                            Operand::Copy(Place::local(current_temp)),
                            Operand::Copy(Place::local(rhs_temp)),
                        ),
                    ),
                );

                // Store result back to variable
                self.push_statement(
                    block,
                    MirStatement::Assign(var_place, Rvalue::Use(Operand::Move(Place::local(result_temp)))),
                );

                Ok(())
            }

            PatternKind::Wildcard => {
                // Wildcard discards the value - no operation needed
                Ok(())
            }

            PatternKind::Paren(inner) => {
                // Unwrap parenthesized pattern
                self.lower_compound_destructuring_element(inner, rhs_value, bin_op, block)
            }

            PatternKind::Tuple(patterns) => {
                // Nested tuple: recursively handle each element
                for (i, pat) in patterns.iter().enumerate() {
                    let field_source = rhs_value.clone().with_field(i);
                    self.lower_compound_destructuring_element(pat, field_source, bin_op, block)?;
                }
                Ok(())
            }

            _ => Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "compound destructuring assignment requires identifier patterns, found: {:?}",
                    pattern.kind
                ))
                .build()),
        }
    }

    /// Lower pattern test for `is` expressions - tests match without creating bindings
    ///
    /// This is used by `x is Pattern` and `x !is Pattern` expressions to test
    /// whether a value matches a pattern, returning a boolean result without
    /// actually binding any pattern variables to scope.
    ///
    /// Unlike `lower_pattern_binding`, this function:
    /// - Does NOT create variable bindings for identifiers
    /// - Returns a boolean indicating match success
    /// - Is suitable for conditional tests where we don't need extracted values
    fn lower_pattern_test(
        &mut self,
        pattern: &Pattern,
        source: Place,
        block: BlockId,
    ) -> Result<LocalId, Diagnostic> {
        match &pattern.kind {
            PatternKind::Wildcard => {
                // Wildcard matches everything
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::Ident { .. } => {
                // Identifier patterns always match (for testing purposes, like wildcard)
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::Literal(lit) => {
                // Compare with literal
                let lit_temp = self.new_temp(MirType::Infer);
                let constant = self.lower_literal_to_constant(lit);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(lit_temp),
                        Rvalue::Use(Operand::Constant(constant)),
                    ),
                );

                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(source),
                            Operand::Copy(Place::local(lit_temp)),
                        ),
                    ),
                );
                Ok(result)
            }

            PatternKind::Tuple(patterns) => {
                // Test all tuple elements match
                let mut all_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(all_match),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                for (i, pat) in patterns.iter().enumerate() {
                    let field_place = source.clone().with_field(i);
                    let match_result = self.lower_pattern_test(pat, field_place, block)?;

                    let new_all_match = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_all_match),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(all_match)),
                                Operand::Copy(Place::local(match_result)),
                            ),
                        ),
                    );
                    all_match = new_all_match;
                }

                Ok(all_match)
            }

            PatternKind::Variant { path, data } => {
                // Check discriminant matches expected variant
                let expected_disc = self.resolve_variant_discriminant(path);

                let disc_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(disc_temp),
                        Rvalue::Discriminant(source.clone()),
                    ),
                );

                let disc_match = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(disc_match),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(Place::local(disc_temp)),
                            Operand::Constant(MirConstant::Int(expected_disc)),
                        ),
                    ),
                );

                // If no nested data patterns, discriminant match is sufficient
                if data.is_none() {
                    return Ok(disc_match);
                }

                // Test nested patterns
                let mut result = disc_match;

                if let Some(data) = data {
                    match data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            let variant_place = source.with_downcast(expected_disc as usize);
                            for (i, pat) in patterns.iter().enumerate() {
                                let nested_result = self.lower_pattern_test(
                                    pat,
                                    variant_place.clone().with_field(i),
                                    block,
                                )?;
                                let new_result = self.new_temp(MirType::Bool);
                                self.push_statement(
                                    block,
                                    MirStatement::Assign(
                                        Place::local(new_result),
                                        Rvalue::Binary(
                                            BinOp::And,
                                            Operand::Copy(Place::local(result)),
                                            Operand::Copy(Place::local(nested_result)),
                                        ),
                                    ),
                                );
                                result = new_result;
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            let variant_place = source.with_downcast(expected_disc as usize);
                            for (i, field) in fields.iter().enumerate() {
                                if let Some(pat) = &field.pattern {
                                    let nested_result = self.lower_pattern_test(
                                        pat,
                                        variant_place.clone().with_field(i),
                                        block,
                                    )?;
                                    let new_result = self.new_temp(MirType::Bool);
                                    self.push_statement(
                                        block,
                                        MirStatement::Assign(
                                            Place::local(new_result),
                                            Rvalue::Binary(
                                                BinOp::And,
                                                Operand::Copy(Place::local(result)),
                                                Operand::Copy(Place::local(nested_result)),
                                            ),
                                        ),
                                    );
                                    result = new_result;
                                }
                            }
                        }
                    }
                }

                Ok(result)
            }

            PatternKind::Or(patterns) => {
                // Any alternative can match - use OR
                let mut result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(false))),
                    ),
                );

                for pat in patterns.iter() {
                    let pat_match = self.lower_pattern_test(pat, source.clone(), block)?;
                    let new_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_result),
                            Rvalue::Binary(
                                BinOp::Or,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(pat_match)),
                            ),
                        ),
                    );
                    result = new_result;
                }

                Ok(result)
            }

            PatternKind::Range { start, end, inclusive } => {
                // Test if value is within range (Range uses Literals)
                let mut result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );

                if let Some(start_lit) = start {
                    let start_const = self.lower_literal_to_constant(start_lit);
                    let start_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(start_temp),
                            Rvalue::Use(Operand::Constant(start_const)),
                        ),
                    );

                    let ge_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(ge_result),
                            Rvalue::Binary(
                                BinOp::Ge,
                                Operand::Copy(source.clone()),
                                Operand::Copy(Place::local(start_temp)),
                            ),
                        ),
                    );

                    let new_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_result),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(ge_result)),
                            ),
                        ),
                    );
                    result = new_result;
                }

                if let Some(end_lit) = end {
                    let end_const = self.lower_literal_to_constant(end_lit);
                    let end_temp = self.new_temp(MirType::Infer);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(end_temp),
                            Rvalue::Use(Operand::Constant(end_const)),
                        ),
                    );

                    let cmp_op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                    let cmp_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(cmp_result),
                            Rvalue::Binary(
                                cmp_op,
                                Operand::Copy(source.clone()),
                                Operand::Copy(Place::local(end_temp)),
                            ),
                        ),
                    );

                    let new_result = self.new_temp(MirType::Bool);
                    self.push_statement(
                        block,
                        MirStatement::Assign(
                            Place::local(new_result),
                            Rvalue::Binary(
                                BinOp::And,
                                Operand::Copy(Place::local(result)),
                                Operand::Copy(Place::local(cmp_result)),
                            ),
                        ),
                    );
                    result = new_result;
                }

                Ok(result)
            }

            PatternKind::Paren(inner) => self.lower_pattern_test(inner, source, block),

            PatternKind::Reference { inner, mutable: _ } => {
                let deref_place = source.with_deref();
                self.lower_pattern_test(inner, deref_place, block)
            }

            PatternKind::Rest => {
                let result = self.new_temp(MirType::Bool);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Use(Operand::Constant(MirConstant::Bool(true))),
                    ),
                );
                Ok(result)
            }

            PatternKind::TypeTest { test_type, .. } => {
                // TypeTest pattern test: just check if the type matches, no binding
                // This is used for `x is Type` expressions where we only need the boolean result

                // Get the type name for discriminant lookup
                let type_name = format!("{:?}", test_type.kind);

                // Create result boolean for the type test
                let result = self.new_temp(MirType::Bool);

                // Get the runtime type tag of the source value
                let type_tag = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(Place::local(type_tag), Rvalue::Discriminant(source.clone())),
                );

                // Get the expected type tag for the test type by treating type name as a path
                let type_path = verum_ast::ty::Path::single(verum_ast::Ident {
                    name: type_name.into(),
                    span: verum_ast::Span::default(),
                });
                let expected_tag = self.resolve_variant_discriminant(&type_path);
                let expected_temp = self.new_temp(MirType::Int);
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(expected_temp),
                        Rvalue::Use(Operand::Constant(MirConstant::Int(expected_tag))),
                    ),
                );

                // Compare type tags
                self.push_statement(
                    block,
                    MirStatement::Assign(
                        Place::local(result),
                        Rvalue::Binary(
                            BinOp::Eq,
                            Operand::Copy(Place::local(type_tag)),
                            Operand::Copy(Place::local(expected_temp)),
                        ),
                    ),
                );

                Ok(result)
            }

            // For other patterns, fall back to lower_pattern_binding
            _ => self.lower_pattern_binding(pattern, source, block),
        }
    }

    /// Get discriminant value from pattern (for match lowering)
    ///
    /// Resolves the discriminant value for various pattern types:
    /// - Literals: Direct integer value or boolean (0/1)
    /// - Variants: Looks up discriminant from type registry
    /// - Other patterns: Returns None (handled elsewhere)
    fn pattern_to_discriminant(&self, pattern: &Pattern) -> Option<i64> {
        match &pattern.kind {
            PatternKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => Some(i.value as i64),
                LiteralKind::Bool(b) => Some(if *b { 1 } else { 0 }),
                LiteralKind::Char(c) => Some(*c as i64),
                _ => None,
            },
            PatternKind::Variant { path, data: _ } => {
                // Use resolve_variant_discriminant to look up actual discriminant
                // from type registry based on enum definition
                // This handles both unit variants (None) and data variants (Some(x))
                Some(self.resolve_variant_discriminant(path))
            }
            PatternKind::Ident { name, .. } => {
                // Ident patterns might be enum unit variants without qualification
                // (e.g., `None` when `use Option::None` is in scope)
                // Try to resolve as a simple identifier path
                let path = verum_ast::ty::Path::single(name.clone());
                // If this fails to resolve, it will return 0 (safe default for non-variant idents)
                Some(self.resolve_variant_discriminant(&path))
            }
            _ => None,
        }
    }
}

// =============================================================================
// Function Lowering
// =============================================================================

impl LoweringContext {
    /// Lower a function declaration
    fn lower_function(&mut self, func: &FunctionDecl) -> Result<MirFunction, Diagnostic> {
        self.stats.functions_lowered += 1;

        // Reset state for new function
        self.next_block = 0;
        self.next_local = 0;
        self.var_map.clear();
        self.ssa_versions.clear();
        self.scope_stack.clear();
        self.loop_stack.clear();
        self.defer_stack.clear();
        self.drop_flags.clear();
        self.type_cache.clear();

        // Create signature
        let params: List<MirType> = func
            .params
            .iter()
            .filter_map(|p| match &p.kind {
                FunctionParamKind::Regular { ty, .. } => Some(self.lower_type(ty)),
                _ => None,
            })
            .collect();

        let return_type = func
            .return_type
            .as_ref()
            .map(|t| self.lower_type(t))
            .unwrap_or(MirType::Unit);

        let contexts: List<Text> = func
            .contexts
            .iter()
            .map(|c| {
                c.path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join(".")
                    .into()
            })
            .collect();

        let signature = MirSignature {
            params: params.clone(),
            return_type: return_type.clone(),
            contexts,
            is_async: func.is_async,
        };

        // Initialize function
        self.current_func = Some(MirFunction {
            name: func.name.name.clone().into(),
            signature,
            locals: List::new(),
            blocks: List::new(),
            entry_block: BlockId(0),
            cleanup_blocks: List::new(),
            span: func.span,
        });

        // Create return place
        let _ = self.new_local("_0", return_type, LocalKind::ReturnPlace);

        // Create parameter locals with CBGR retags
        let entry_block = self.new_block();

        for param in func.params.iter() {
            match &param.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => {
                    let param_ty = self.lower_type(ty);
                    if let PatternKind::Ident { name, .. } = &pattern.kind {
                        let local = self.new_local(name.name.clone(), param_ty, LocalKind::Arg);
                        // Retag parameters at function entry
                        self.push_statement(
                            entry_block,
                            MirStatement::Retag {
                                place: Place::local(local),
                                kind: RetagKind::FnEntry,
                            },
                        );
                    }
                }
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefMut
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefCheckedMut
                | FunctionParamKind::SelfRefUnsafe
                | FunctionParamKind::SelfRefUnsafeMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut => {
                    let local = self.new_local("self", MirType::Infer, LocalKind::Arg);
                    self.push_statement(
                        entry_block,
                        MirStatement::Retag {
                            place: Place::local(local),
                            kind: RetagKind::FnEntry,
                        },
                    );
                }
            }
        }

        // Lower function body
        if let Some(ref body) = func.body {
            match body {
                FunctionBody::Block(block) => {
                    let exit_block =
                        self.lower_block_iterative(block, entry_block, Place::return_place())?;
                    self.set_terminator(exit_block, Terminator::Return);
                }
                FunctionBody::Expr(expr) => {
                    let exit_block = self.lower_expr(expr, entry_block, Place::return_place())?;
                    self.set_terminator(exit_block, Terminator::Return);
                }
            }
        } else {
            // No body: just return
            self.set_terminator(entry_block, Terminator::Return);
        }

        Ok(self.current_func.take().unwrap())
    }
}

// =============================================================================
// Module Lowering
// =============================================================================

impl LoweringContext {
    /// Lower a module to MIR
    pub fn lower_module(&mut self, module: &Module) -> Result<MirModule, Diagnostic> {
        let mut functions = List::new();
        let mut types = List::new();
        let globals = List::new();

        // First pass: Register all type definitions in the type registry
        // This must happen before lowering functions so field/variant resolution works
        for item in module.items.iter() {
            if let ItemKind::Type(type_decl) = &item.kind {
                match &type_decl.body {
                    verum_ast::decl::TypeDeclBody::Record(fields) => {
                        // Register struct in type registry
                        let struct_fields: Vec<(Text, MirType)> = fields
                            .iter()
                            .map(|f| (f.name.as_str().into(), self.lower_type(&f.ty)))
                            .collect();

                        self.type_registry
                            .register_struct(type_decl.name.as_str().into(), struct_fields);

                        tracing::debug!(
                            "Registered struct '{}' with {} fields",
                            type_decl.name.name,
                            fields.len()
                        );
                    }
                    verum_ast::decl::TypeDeclBody::Variant(variants) => {
                        // Register enum in type registry
                        let enum_variants: Vec<(Text, Vec<MirType>)> = variants
                            .iter()
                            .map(|v| {
                                let variant_types: Vec<MirType> = match &v.data {
                                    Some(verum_ast::decl::VariantData::Tuple(types)) => {
                                        types.iter().map(|t| self.lower_type(t)).collect()
                                    }
                                    Some(verum_ast::decl::VariantData::Record(fields)) => {
                                        fields.iter().map(|f| self.lower_type(&f.ty)).collect()
                                    }
                                    None => Vec::new(),
                                };
                                (v.name.as_str().into(), variant_types)
                            })
                            .collect();

                        self.type_registry
                            .register_enum(type_decl.name.as_str().into(), enum_variants);

                        tracing::debug!(
                            "Registered enum '{}' with {} variants",
                            type_decl.name.name,
                            variants.len()
                        );
                    }
                    _ => {
                        // Aliases don't need registry entries (they're resolved during type lowering)
                    }
                }
            }
        }

        // Second pass: Lower all items (functions and type definitions)
        for item in module.items.iter() {
            match &item.kind {
                ItemKind::Function(func) => {
                    let mir_func = self.lower_function(func)?;
                    functions.push(mir_func);
                }
                ItemKind::Type(type_decl) => {
                    // Lower type definitions for MIR output
                    let type_def = MirTypeDef {
                        name: type_decl.name.as_str().into(),
                        kind: match &type_decl.body {
                            verum_ast::decl::TypeDeclBody::Record(fields) => {
                                MirTypeDefKind::Struct(
                                    fields
                                        .iter()
                                        .map(|f| (f.name.as_str().into(), self.lower_type(&f.ty)))
                                        .collect(),
                                )
                            }
                            verum_ast::decl::TypeDeclBody::Variant(variants) => {
                                MirTypeDefKind::Enum(
                                    variants
                                        .iter()
                                        .map(|v| {
                                            let types = match &v.data {
                                                Some(verum_ast::decl::VariantData::Tuple(
                                                    types,
                                                )) => types
                                                    .iter()
                                                    .map(|t| self.lower_type(t))
                                                    .collect(),
                                                Some(verum_ast::decl::VariantData::Record(
                                                    fields,
                                                )) => fields
                                                    .iter()
                                                    .map(|f| self.lower_type(&f.ty))
                                                    .collect(),
                                                None => List::new(),
                                            };
                                            (v.name.as_str().into(), types)
                                        })
                                        .collect(),
                                )
                            }
                            verum_ast::decl::TypeDeclBody::Alias(ty) => {
                                MirTypeDefKind::Alias(self.lower_type(ty))
                            }
                            _ => MirTypeDefKind::Alias(MirType::Unit),
                        },
                    };
                    types.push(type_def);
                }
                _ => {}
            }
        }

        Ok(MirModule {
            name: format!("module_{}", module.file_id.raw()).into(),
            functions,
            types,
            globals,
        })
    }
}

// =============================================================================
// SSA Construction
// =============================================================================

impl LoweringContext {
    /// Convert MIR to SSA form using Cytron algorithm
    pub fn convert_to_ssa(&mut self, func: &mut MirFunction) {
        // Already in partial SSA form from lowering
        // This would complete the transformation with phi nodes

        // 1. Compute dominance frontiers
        let blocks: Vec<BasicBlock> = func.blocks.iter().cloned().collect();
        let dom_tree = DominatorTree::compute(&blocks, func.entry_block);

        // 2. Insert phi nodes at dominance frontiers
        let mut var_defs: HashMap<LocalId, HashSet<BlockId>> = HashMap::new();

        // Find all definitions
        for block in func.blocks.iter() {
            for stmt in block.statements.iter() {
                if let MirStatement::Assign(place, _) = stmt {
                    if place.projections.is_empty() {
                        var_defs.entry(place.local).or_default().insert(block.id);
                    }
                }
            }
        }

        // Insert phi nodes
        for (local, def_blocks) in var_defs.iter() {
            let mut worklist: Vec<BlockId> = def_blocks.iter().copied().collect();
            let mut processed = HashSet::new();

            while let Some(block_id) = worklist.pop() {
                if let Some(frontier_set) = dom_tree.dominance_frontier(block_id) {
                    for &frontier_block in frontier_set {
                        if !processed.contains(&frontier_block) {
                            processed.insert(frontier_block);

                            // Add phi node
                            let ty = self
                                .type_cache
                                .get(local)
                                .cloned()
                                .unwrap_or(MirType::Infer);
                            if let Some(block) =
                                func.blocks.iter_mut().find(|b| b.id == frontier_block)
                            {
                                block.phi_nodes.push(PhiNode {
                                    dest: *local,
                                    operands: List::new(),
                                    ty,
                                });
                                self.stats.phi_nodes_created += 1;
                            }

                            worklist.push(frontier_block);
                        }
                    }
                }
            }
        }

        // 3. Rename variables (assign SSA versions)
        self.rename_variables(func, &dom_tree);
    }

    /// Rename variables for SSA
    fn rename_variables(&mut self, func: &mut MirFunction, _dom_tree: &DominatorTree) {
        let mut var_stack: HashMap<LocalId, Vec<u32>> = HashMap::new();

        // Initialize stacks
        for local in func.locals.iter() {
            var_stack.insert(local.id, vec![0]);
        }

        // Rename in dominator tree order
        self.rename_block(func, func.entry_block, &mut var_stack);
    }

    fn rename_block(
        &mut self,
        func: &mut MirFunction,
        block_id: BlockId,
        var_stack: &mut HashMap<LocalId, Vec<u32>>,
    ) {
        // Process phi nodes
        if let Some(block) = func.blocks.iter_mut().find(|b| b.id == block_id) {
            for phi in block.phi_nodes.iter_mut() {
                let version = self.next_ssa_version(phi.dest);
                var_stack.entry(phi.dest).or_default().push(version);
            }
        }

        // Process statements
        // In full implementation, would rename all uses and definitions
    }
}

// =============================================================================
// Phase Implementation
// =============================================================================

/// MIR Lowering Phase
pub struct MirLoweringPhase {
    stats: LoweringStats,
}

impl MirLoweringPhase {
    pub fn new() -> Self {
        Self {
            stats: LoweringStats::default(),
        }
    }

    fn lower_modules(&mut self, modules: &[Module]) -> Result<List<MirModule>, List<Diagnostic>> {
        let mut ctx = LoweringContext::new();
        let mut mir_modules = List::new();

        for module in modules {
            match ctx.lower_module(module) {
                Ok(mir_module) => mir_modules.push(mir_module),
                Err(diag) => return Err(List::from(vec![diag])),
            }
        }

        self.stats = ctx.stats;
        Ok(mir_modules)
    }
}

impl Default for MirLoweringPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for MirLoweringPhase {
    fn name(&self) -> &str {
        "Phase 5: HIR -> MIR Lowering"
    }

    fn description(&self) -> &str {
        "Lower to control flow graph with SSA form and insert safety checks"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules from input
        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for MIR lowering phase")
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Create mutable phase for statistics
        let mut phase = Self::new();

        // Lower modules to MIR
        let mut mir_modules = phase.lower_modules(modules)?;

        // ============================================================================
        // OPTIMIZATION PIPELINE INTEGRATION (CRITICAL)
        // ============================================================================
        // Run FULL MIR optimizations before converting to wrapper format.
        // This is the MAIN optimization pass that implements:
        // - Escape analysis (NoEscape/LocalEscape/HeapEscape/ThreadEscape)
        // - CBGR check elimination (50-90% typical)
        // - Bounds check elimination
        // - SBGL optimization (Stack-Based Garbage-free Lists)
        // - Reference promotion to &checked T (zero-cost tier)
        // - Loop invariant code motion
        // - Dead code elimination
        // - Function inlining (O2+)
        // - SIMD vectorization (O3)
        //
        // This happens on the full internal MirModule structure with functions, locals,
        // and SSA form, enabling sophisticated interprocedural analysis.
        // ============================================================================

        // Convert OptimizationLevel from super:: to optimization::
        let opt_level = match input.context.opt_level {
            super::OptimizationLevel::O0 => super::optimization::OptimizationLevel::O0,
            super::OptimizationLevel::O1 => super::optimization::OptimizationLevel::O1,
            super::OptimizationLevel::O2 => super::optimization::OptimizationLevel::O2,
            super::OptimizationLevel::O3 => super::optimization::OptimizationLevel::O3,
        };

        let (_opt_stats, opt_warnings) =
            super::optimization::optimize_mir_modules(&mut mir_modules, opt_level);

        tracing::debug!(
            "MIR optimization complete: opt_level={:?}, warnings={}",
            input.context.opt_level,
            opt_warnings.len()
        );

        // Pass the full internal MIR structure directly to PhaseData::Mir
        // This enables the optimization phase to perform meaningful optimizations
        // on the complete MIR structure with functions, locals, and SSA form.
        // Previous implementation converted to a simplified wrapper format that
        // lost function/local/statement information needed for proper optimization.
        let output_modules: List<MirModule> = mir_modules.into_iter().collect();

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);
        metrics.add_custom_metric(
            "functions_lowered",
            phase.stats.functions_lowered.to_string(),
        );
        metrics.add_custom_metric("blocks_created", phase.stats.blocks_created.to_string());
        metrics.add_custom_metric(
            "instructions_generated",
            phase.stats.instructions_generated.to_string(),
        );
        metrics.add_custom_metric(
            "cbgr_checks_inserted",
            phase.stats.cbgr_checks_inserted.to_string(),
        );
        metrics.add_custom_metric(
            "bounds_checks_inserted",
            phase.stats.bounds_checks_inserted.to_string(),
        );
        metrics.add_custom_metric(
            "thin_refs_selected",
            phase.stats.thin_refs_selected.to_string(),
        );
        metrics.add_custom_metric(
            "fat_refs_selected",
            phase.stats.fat_refs_selected.to_string(),
        );
        metrics.add_custom_metric("temps_created", phase.stats.temps_created.to_string());
        metrics.add_custom_metric(
            "phi_nodes_created",
            phase.stats.phi_nodes_created.to_string(),
        );
        metrics.add_custom_metric("drops_inserted", phase.stats.drops_inserted.to_string());
        metrics.add_custom_metric(
            "ssa_versions_created",
            phase.stats.ssa_versions_created.to_string(),
        );

        tracing::info!(
            "MIR lowering complete: {} functions, {} blocks, {} instructions, {} phi nodes, ThinRef: {}, FatRef: {}, {:.2}ms",
            phase.stats.functions_lowered,
            phase.stats.blocks_created,
            phase.stats.instructions_generated,
            phase.stats.phi_nodes_created,
            phase.stats.thin_refs_selected,
            phase.stats.fat_refs_selected,
            duration.as_millis()
        );

        Ok(PhaseOutput {
            data: PhaseData::Mir(output_modules),
            warnings: opt_warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Functions can be lowered in parallel
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_creation() {
        let mut ctx = LoweringContext::new();
        ctx.current_func = Some(MirFunction {
            name: "test".into(),
            signature: MirSignature {
                params: List::new(),
                return_type: MirType::Unit,
                contexts: List::new(),
                is_async: false,
            },
            locals: List::new(),
            blocks: List::new(),
            entry_block: BlockId(0),
            cleanup_blocks: List::new(),
            span: AstSpan::dummy(),
        });

        let b1 = ctx.new_block();
        let b2 = ctx.new_block();

        assert_eq!(b1.0, 0);
        assert_eq!(b2.0, 1);
        assert_eq!(ctx.stats.blocks_created, 2);
    }

    #[test]
    fn test_local_creation() {
        let mut ctx = LoweringContext::new();
        ctx.current_func = Some(MirFunction {
            name: "test".into(),
            signature: MirSignature {
                params: List::new(),
                return_type: MirType::Unit,
                contexts: List::new(),
                is_async: false,
            },
            locals: List::new(),
            blocks: List::new(),
            entry_block: BlockId(0),
            cleanup_blocks: List::new(),
            span: AstSpan::dummy(),
        });

        let l1 = ctx.new_local("x", MirType::Int, LocalKind::Var);
        let l2 = ctx.new_temp(MirType::Float);

        assert_eq!(l1.0, 0);
        assert_eq!(l2.0, 1);
        assert_eq!(ctx.lookup_var("x"), Some(l1));
    }

    #[test]
    fn test_reference_layout_selection() {
        let mut ctx = LoweringContext::new();

        // Simple types get ThinRef
        let layout = ctx.select_reference_layout(&MirType::Int);
        assert_eq!(layout, ReferenceLayout::ThinRef);

        // Slices get FatRef with Length
        let layout = ctx.select_reference_layout(&MirType::Slice(Box::new(MirType::Int)));
        assert_eq!(layout, ReferenceLayout::FatRef(MetadataKind::Length));

        // Trait objects get FatRef with VTable
        let layout = ctx.select_reference_layout(&MirType::Named("dyn Display".into()));
        assert_eq!(layout, ReferenceLayout::FatRef(MetadataKind::VTable));
    }

    #[test]
    fn test_dominator_tree() {
        // Create a simple diamond CFG:
        // entry -> then, else
        // then -> join
        // else -> join
        let blocks = vec![
            BasicBlock {
                id: BlockId(0), // entry
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::new(),
                successors: List::from(vec![BlockId(1), BlockId(2)]),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(1), // then
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(0)]),
                successors: List::from(vec![BlockId(3)]),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(2), // else
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(0)]),
                successors: List::from(vec![BlockId(3)]),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(3), // join
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(1), BlockId(2)]),
                successors: List::new(),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
        ];

        let dom_tree = DominatorTree::compute(&blocks, BlockId(0));

        // Entry dominates everything
        assert!(dom_tree.dominates(BlockId(0), BlockId(0)));
        assert!(dom_tree.dominates(BlockId(0), BlockId(1)));
        assert!(dom_tree.dominates(BlockId(0), BlockId(2)));
        assert!(dom_tree.dominates(BlockId(0), BlockId(3)));

        // Then and else don't dominate join
        assert!(!dom_tree.dominates(BlockId(1), BlockId(3)));
        assert!(!dom_tree.dominates(BlockId(2), BlockId(3)));
    }

    #[test]
    fn test_loop_detection() {
        // Create a simple loop CFG:
        // entry -> header
        // header -> body, exit
        // body -> header (back edge)
        let blocks = vec![
            BasicBlock {
                id: BlockId(0), // entry
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::new(),
                successors: List::from(vec![BlockId(1)]),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(1), // header
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(0), BlockId(2)]),
                successors: List::from(vec![BlockId(2), BlockId(3)]),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(2), // body
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(1)]),
                successors: List::from(vec![BlockId(1)]), // back edge
                phi_nodes: List::new(),
                is_cleanup: false,
            },
            BasicBlock {
                id: BlockId(3), // exit
                statements: List::new(),
                terminator: Terminator::default(),
                predecessors: List::from(vec![BlockId(1)]),
                successors: List::new(),
                phi_nodes: List::new(),
                is_cleanup: false,
            },
        ];

        let dom_tree = DominatorTree::compute(&blocks, BlockId(0));
        let loop_info = LoopInfo::compute(&blocks, BlockId(0), &dom_tree);

        // Header should be detected as loop header
        assert!(loop_info.is_loop_header(BlockId(1)));
        assert!(!loop_info.is_loop_header(BlockId(0)));
        assert!(!loop_info.is_loop_header(BlockId(2)));

        // Body should be in loop
        assert_eq!(loop_info.containing_loop(BlockId(2)), Some(BlockId(1)));
    }

    #[test]
    fn test_ssa_versioning() {
        let mut ctx = LoweringContext::new();

        let local = LocalId(0);
        ctx.ssa_versions.insert(local, 0);

        let v1 = ctx.next_ssa_version(local);
        let v2 = ctx.next_ssa_version(local);
        let v3 = ctx.next_ssa_version(local);

        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(v3, 3);
    }

    #[test]
    fn test_type_needs_drop() {
        let ctx = LoweringContext::new();

        // Primitives don't need drop
        assert!(!ctx.type_needs_drop(&MirType::Int));
        assert!(!ctx.type_needs_drop(&MirType::Bool));
        assert!(!ctx.type_needs_drop(&MirType::Float));

        // Complex types need drop
        assert!(ctx.type_needs_drop(&MirType::Text));
        assert!(ctx.type_needs_drop(&MirType::Named("String".into())));
        assert!(ctx.type_needs_drop(&MirType::Slice(Box::new(MirType::Int))));
    }
}
