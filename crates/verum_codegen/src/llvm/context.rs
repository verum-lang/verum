//! Per-function lowering context for VBC → LLVM IR.
//!
//! This module manages the state needed during the lowering of a single
//! VBC function to LLVM IR.

use std::collections::HashMap;

use verum_common::Text;
use verum_llvm::basic_block::BasicBlock;
use verum_llvm::builder::Builder;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::BasicTypeEnum;
use verum_llvm::values::{BasicValueEnum, FunctionValue, PointerValue};
use verum_vbc::module::VbcModule;

/// Exception handler info for structured exception handling.
#[derive(Debug, Clone)]
pub struct ExceptionHandler<'ctx> {
    /// Basic block for the exception handler code.
    pub handler_block: BasicBlock<'ctx>,
    /// Basic block to continue after catch (after try-end).
    pub continuation_block: Option<BasicBlock<'ctx>>,
}

use super::cbgr::CbgrLowering;
use super::error::{LlvmLoweringError, Result};
use super::register_types::{RegisterType, RegisterTypeMap, MethodDispatchTable};
use super::types::{RefTier, TypeLowering};
use std::sync::Arc;

/// Pre-built index for O(1) function name lookups in VBC modules.
///
/// Replaces O(n) linear scans of `vbc_mod.functions` in instruction.rs
/// Strategy 2/3 with efficient HashMap lookups.
#[derive(Debug, Clone, Default)]
pub struct FuncNameIndex {
    /// Method suffix → list of (function_index, full_name, param_count).
    /// E.g., ".push" → [(42, "List.push", 2), (99, "Deque.push", 2)]
    pub by_suffix: HashMap<String, Vec<FuncIndexEntry>>,
    /// Full function name → (function_index, param_count).
    /// E.g., "List.push" → (42, 2)
    pub by_name: HashMap<String, FuncIndexEntry>,
}

#[derive(Debug, Clone)]
pub struct FuncIndexEntry {
    pub index: usize,
    pub name: String,
    pub param_count: u16,
}

impl FuncNameIndex {
    /// Build the index from a VBC module.
    pub fn build(vbc_module: &VbcModule) -> Self {
        let mut by_suffix: HashMap<String, Vec<FuncIndexEntry>> = HashMap::new();
        let mut by_name: HashMap<String, FuncIndexEntry> = HashMap::new();

        for (idx, func_desc) in vbc_module.functions.iter().enumerate() {
            let fname = vbc_module.strings.get(func_desc.name).unwrap_or("");
            if fname.is_empty() {
                continue;
            }
            let entry = FuncIndexEntry {
                index: idx,
                name: fname.to_string(),
                param_count: func_desc.params.len() as u16,
            };
            by_name.insert(fname.to_string(), entry.clone());
            // Index by method suffix (part after last '.')
            if let Some(dot_pos) = fname.rfind('.') {
                let suffix = &fname[dot_pos..]; // e.g., ".push"
                by_suffix.entry(suffix.to_string()).or_default().push(entry.clone());
            }
            // Also index by bare name for exact matches
            by_suffix.entry(fname.to_string()).or_default().push(entry);
        }

        FuncNameIndex { by_suffix, by_name }
    }

    /// Find functions matching a method suffix (e.g., ".push").
    pub fn find_by_suffix(&self, suffix: &str) -> &[FuncIndexEntry] {
        self.by_suffix.get(suffix).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Find a function by exact name.
    pub fn find_by_name(&self, name: &str) -> Option<&FuncIndexEntry> {
        self.by_name.get(name)
    }
}

/// Per-function lowering context.
///
/// Manages register values, basic blocks, and statistics for a single
/// function being lowered.
pub struct FunctionContext<'a, 'ctx> {
    /// LLVM function being built.
    function: FunctionValue<'ctx>,

    /// LLVM module containing the function.
    module: &'a Module<'ctx>,

    /// VBC module for FFI symbol table and metadata access.
    /// Required for zero-cost FFI lowering.
    vbc_module: Option<&'a VbcModule>,

    /// Pre-built function name index for O(1) lookups (shared across all functions).
    func_name_index: Option<Arc<FuncNameIndex>>,

    /// VBC func_id → LLVM function value mapping (shared from lowering context).
    /// Enables Call instructions to resolve by func_id instead of name, avoiding
    /// name collision issues where multiple VBC functions share a name.
    func_id_map: Option<Arc<HashMap<u32, FunctionValue<'ctx>>>>,

    /// LLVM builder for instruction generation.
    builder: Builder<'ctx>,

    /// Type lowering helper.
    types: TypeLowering<'ctx>,

    /// CBGR lowering helper.
    cbgr: CbgrLowering<'ctx>,

    /// Register values (register index → LLVM value).
    registers: HashMap<u16, BasicValueEnum<'ctx>>,

    /// Register stack slots for mutable registers.
    register_slots: HashMap<u16, PointerValue<'ctx>>,

    /// Basic block map (block index → LLVM BasicBlock).
    blocks: HashMap<u32, BasicBlock<'ctx>>,

    /// Current CBGR tier for optimizations.
    current_tier: RefTier,

    /// Per-register CBGR tiers for escape-analysis-based optimization.
    /// Registers marked as Tier1 have been proven safe by escape analysis.
    register_tiers: HashMap<u16, RefTier>,

    /// Registers that contain references (for escape tracking).
    reference_registers: HashMap<u16, ReferenceInfo>,

    /// Instruction counter.
    instruction_count: usize,

    /// Function name for diagnostics.
    function_name: Text,

    /// CBGR elimination statistics.
    cbgr_elimination_stats: CbgrEliminationStats,

    /// Stack of exception handlers for structured exception handling.
    exception_handlers: Vec<ExceptionHandler<'ctx>>,

    /// Pointer to the current exception value (allocated on first use).
    exception_value_slot: Option<PointerValue<'ctx>>,

    /// When true, registers use alloca-based storage (for multi-block functions).
    /// This ensures values persist across basic block boundaries.
    alloca_mode: bool,

    /// Alloca slots for registers in alloca mode (register index → alloca ptr).
    alloca_registers: HashMap<u16, PointerValue<'ctx>>,

    /// Tracks the LLVM type of each alloca register (for typed loads in alloca mode).
    alloca_register_types: HashMap<u16, BasicTypeEnum<'ctx>>,

    /// Registers that hold string pointers (for correct DebugPrint dispatch).
    /// NOTE: Being replaced by text_registers. Kept during migration.
    string_registers: std::collections::HashSet<u16>,

    /// Registers that hold Text* pointers (new flat {ptr, len, cap} layout).
    /// Text objects are 24-byte heap structs. Register holds i64 = pointer to struct.
    text_registers: std::collections::HashSet<u16>,

    /// Registers that hold boolean values (for correct Not dispatch: logical vs bitwise).
    bool_registers: std::collections::HashSet<u16>,

    /// Registers that hold float values (for correct ToString dispatch: float vs int).
    float_registers: std::collections::HashSet<u16>,

    /// Pre-scanned float registers: registers that VBC BinaryF/UnaryF/CmpF/LoadK(Float)
    /// will use. Unlike float_registers, these are NOT cleared by set_register().
    /// (Migrated to reg_types — prescan_float flag)

    /// Pre-scanned text registers: registers that the pre-pass determined hold Text values
    /// (e.g., AsVar extracting from Result<Text, E>). Unlike text_registers, these are
    /// NOT cleared by set_register(), so instructions like AsVar can restore text marking.
    /// (Migrated to reg_types — prescan_text flag)

    /// Variant registers with float fields: (variant_reg, field_idx) → true.
    /// Used by GetVariantData to mark extracted float fields as float_registers.
    variant_float_fields: std::collections::HashSet<(u16, u32)>,

    /// Registers that hold list header pointers (for correct GetE/SetE: header indirection).
    list_registers: std::collections::HashSet<u16>,

    /// Registers that hold lists whose elements are strings (List<Text>).
    /// Used by GetE to propagate string_register tracking through list access.
    /// (Migrated to reg_types — RegisterType::List { element: Text })

    /// Registers that hold channel pointers (for channel method dispatch in CallM).
    chan_registers: std::collections::HashSet<u16>,

    /// Registers that hold range objects (for correct IterNew/IterNext dispatch).
    /// These are header-based ranges from New{type_id:517}.
    range_registers: std::collections::HashSet<u16>,

    /// Registers that hold flat range objects from NewRange/verum_range_new.
    /// Layout: {start, end, step, current} at offsets 0, 8, 16, 24 — NO object header.
    flat_range_registers: std::collections::HashSet<u16>,

    /// Registers that hold map header pointers (for Map method dispatch in CallM).
    map_registers: std::collections::HashSet<u16>,

    /// Map registers whose values are lists (propagated from insert to get).
    map_list_value_registers: std::collections::HashSet<u16>,

    /// Map registers whose values are strings/text (propagated from insert to get).
    map_string_value_registers: std::collections::HashSet<u16>,

    /// Source register for map copies (r_copy → r_original). Used to backpropagate
    /// map value types from Call argument registers to original variable registers.
    map_copy_source: HashMap<u16, u16>,

    /// Source register for RefMut (dst → src). Used to backpropagate type info
    /// (e.g., generic_type_args) from mutable reference back to original variable.
    refmut_source: HashMap<u16, u16>,

    /// Registers that hold set header pointers (for Set method dispatch in CallM).
    set_registers: std::collections::HashSet<u16>,

    /// Registers that hold deque pointers (for Deque method dispatch in CallM).
    deque_registers: std::collections::HashSet<u16>,

    /// Registers that hold BTreeMap pointers (for BTreeMap method dispatch in CallM).
    btreemap_registers: std::collections::HashSet<u16>,

    /// Registers that hold BTreeSet pointers (for BTreeSet method dispatch in CallM).
    btreeset_registers: std::collections::HashSet<u16>,

    /// Registers that hold BinaryHeap pointers (for BinaryHeap method dispatch in CallM).
    binaryheap_registers: std::collections::HashSet<u16>,

    /// Registers that hold AtomicInt/AtomicBool pointers (for atomic method intercepts).
    atomic_int_registers: std::collections::HashSet<u16>,

    /// Registers that hold generator handles (for gen.next()/gen.has_next() dispatch).
    gen_registers: std::collections::HashSet<u16>,

    /// Registers that hold slice pointers ({ptr: i64, len: i64} structs from Pack).
    /// Used by GetE to dereference the embedded pointer and Len to read offset 8.
    slice_registers: std::collections::HashSet<u16>,

    /// Registers that hold text iterators (for `for ch in text_string` dispatch).
    text_iter_registers: std::collections::HashSet<u16>,

    /// Registers that hold map iterators (for `for k in map` dispatch).
    map_iter_registers: std::collections::HashSet<u16>,

    /// Registers that hold custom iterators (user-defined types with has_next/next methods).
    /// For `for v in CustomType { ... }` where CustomType is not List/Range/Map/Text/Generator.
    /// (Migrated to reg_types — RegisterType::CustomIterator)
    /// Type name for custom iterator registers (e.g., "Counter" for `Counter.has_next`/`Counter.next`).
    custom_iter_type_names: HashMap<u16, String>,

    /// Registers holding Maybe<Text> variant objects (from strip_prefix/strip_suffix etc.).
    /// Used by AsVar to propagate string_register marking to extracted payload.
    /// (Migrated to reg_types — RegisterType::Maybe { inner_is_text: true })

    /// Registers holding Maybe<&T> variants from compiled modules (e.g., List.get()).
    /// The payload is a raw pointer that needs auto-deref in unwrap/match.
    /// (Migrated to reg_types — RegisterType::Maybe { inner_is_ref: true })

    /// Registers that hold "owned" text objects — allocated by Concat/ToString/CharToStr.
    /// These must be freed when overwritten or at function exit. NOT set for string literals
    /// (LoadK Constant::String) or function parameters, which are borrowed.
    /// (Migrated to reg_types — RegisterType::Text { owned: true })

    /// Registers holding FFI-allocated pointers (TransferFrom ownership).
    /// (Migrated to reg_types — owned_ffi flag)

    /// Registers consumed by TransferTo FFI calls (use-after-transfer detection).
    /// Any subsequent use of these registers is a potential error.
    /// (Migrated to reg_types — consumed_ffi flag)

    /// Registers that hold struct pointers (stack-spilled structs in alloca mode).
    /// Used by ListPush to heap-copy structs before storing in list backing array.
    /// (Migrated to reg_types — RegisterType::Struct)
    /// Size in bytes of each struct register (field_count * 8).
    struct_register_sizes: HashMap<u16, u32>,

    /// Registers currently holding f64 values (stored directly, no bitcast).
    /// (Migrated to alloca_register_types — FloatType check)

    /// Registers currently holding pointer values (stored directly, no ptrtoint).
    /// (Migrated to alloca_register_types — PointerType check)

    /// Registers that hold pointers to inline structs within arrays (no object header).
    /// Set by offset() when the element type is a struct (e.g., Slot<K,V> in Map).
    /// Used by Deref (skip load-through) and GetF (skip header offset).
    /// (Migrated to reg_types — RegisterType::InlineStruct)

    /// Element stride override for pointer registers (in bytes).
    /// Set when GetF loads a backing pointer whose elements are larger than 8 bytes.
    /// Used by offset() to compute correct byte offset (stride * index).
    /// Default (absent) = 8 bytes.
    element_stride_registers: HashMap<u16, u64>,

    /// Registers that hold Heap<T> allocations requiring deallocation on return.
    /// Drop protocol: these registers are freed via verum_dealloc before Ret/RetV.
    /// The return value register is excluded (ownership transfers to caller).
    heap_alloc_registers: std::collections::HashSet<u16>,

    /// Current context provide nesting depth.
    /// Incremented on CtxProvide, decremented on CtxEnd.
    /// Used to pass accurate stack depth to the context runtime.
    ctx_provide_depth: u64,

    /// Tracks which struct fields contain list values.
    /// Key: (obj_register, field_idx), Value: true if field holds a list.
    /// Populated by SetF when storing a list_register into a field.
    /// Used by GetF to propagate list_register tracking through field access.
    struct_list_fields: HashMap<(u16, u32), bool>,

    /// Tracks which struct fields contain string values.
    /// Same pattern as struct_list_fields but for string_registers.
    struct_string_fields: HashMap<(u16, u32), bool>,

    /// Tracks the object type name for registers that hold struct/object pointers.
    /// Used by GetF to look up the correct type's field metadata instead of
    /// blindly using the function name prefix (which is only correct for `self`).
    obj_register_types: HashMap<u16, String>,

    /// Tracks the inner struct type name for registers holding Maybe<Heap<T>> variants.
    /// When GetF loads a field of type Maybe<Heap<Foo>>, we record the innermost struct
    /// type name ("Foo") so that AsVar/unwrap can propagate it to obj_register_type,
    /// enabling downstream GetF to look up field metadata correctly.
    maybe_inner_types: HashMap<u16, String>,

    /// Registers holding Maybe/variant values (heap-allocated variant objects or null).
    /// Used by Ref/RefMut to pass through variant pointers instead of taking alloca addresses.
    variant_registers: std::collections::HashSet<u16>,

    /// Registers where Ref passed through the value instead of creating a pointer.
    /// In VBC semantics, `&x` for primitives (Int, Float, Bool) is just `x` (value copy).
    /// When Deref encounters a register in this set, it passes through instead of loading.
    /// (Migrated to reg_types — pass_through_ref flag)

    /// Lists whose elements are pass-through refs (e.g., List<Heap<T>>).
    /// When GetE reads from such a list, the result is marked as pass_through_ref
    /// so that Deref passes through instead of loading through the pointer.
    /// (Migrated to reg_types — pass_through_ref_list flag)

    /// Registers holding generic type parameters compiled as `ptr` (value-as-pointer).
    /// In compiled module functions, generic params (K, V, T) are represented as `ptr`
    /// where the pointer IS the value (via inttoptr), not a real memory address.
    /// Deref on these registers should do ptrtoint (extract value) instead of load
    /// (which would crash on fake addresses like 0x1 or 0x2).
    /// (Migrated to reg_types — RegisterType::GenericParam)

    /// Tracks the return type of closure registers (from NewClosure or fn-type parameters).
    /// Used by CallClosure to mark the dst register with the correct type (List, Text, etc.)
    /// so downstream operations dispatch correctly.
    closure_return_types: HashMap<u16, verum_vbc::types::TypeRef>,

    /// Unified register type map (Phase 1: coexists with legacy HashSets).
    ///
    /// This is the new architecture that will replace all 40+ HashSet<u16> tracking
    /// fields. During Phase 1, both systems are maintained in parallel. Once all
    /// call sites are migrated, the legacy fields will be removed.
    reg_types: RegisterTypeMap,

    /// Method dispatch table for declarative method routing.
    ///
    /// Maps (type_name, method_name) → DispatchTarget, replacing the Strategy
    /// 0/1/2/3/4 cascade in instruction.rs.
    dispatch_table: MethodDispatchTable,

    /// Tracks tuple element types for registers holding tuple values.
    /// When a function returns a tuple (e.g., `-> (List<Int>, Text)`), the Call handler
    /// stores element types here. Unpack then uses this to mark extracted element registers
    /// with the correct types (list_registers, text_registers, etc.).
    tuple_element_types: HashMap<u16, Vec<verum_vbc::types::TypeRef>>,

    /// Tracks generic type args for registers holding generic struct instances.
    /// When a function returns `Pair<List<Int>, Text>`, the Call handler stores
    /// [List<Int>, Text] here. GetF uses this to resolve `TypeRef::Generic(n)` fields.
    generic_type_args: HashMap<u16, Vec<verum_vbc::types::TypeRef>>,

    /// Element addresses from GetE on list registers. Maps register → pointer into
    /// the backing array. Used by Ref to create references that point directly into
    /// heap-allocated backing storage (surviving function returns), instead of
    /// creating dangling stack alloca pointers.
    gete_element_ptrs: std::collections::HashMap<u16, PointerValue<'ctx>>,

    /// Per-instruction overrides for Len dispatch.
    /// When a register is reused for both List and Text at different instruction points,
    /// the global list_register mark may be wrong. This set records VBC instruction indices
    /// where Len's arr register is known to be a list (from flow-sensitive analysis).
    len_list_overrides: std::collections::HashSet<usize>,

    /// Current VBC instruction index being lowered (set by vbc_lowering before each instruction).
    current_vbc_instr_idx: usize,

    /// Base function ID offset for the current function's source module.
    /// Call func_ids in merged stdlib bytecode are relative to the source module.
    /// Add this offset to get the correct function ID in the merged module.
    func_id_base: u32,

    /// Collected diagnostics emitted during lowering.
    diagnostics: Vec<super::error::LoweringDiagnostic>,

    /// Pending loop optimization hints from a LoopHint instruction.
    /// Applied to the next backward Jmp (loop back-edge) as !llvm.loop metadata.
    pub pending_loop_hints: Option<verum_vbc::module::LoopHints>,

    /// Pending branch likelihood hint from a BranchHint instruction.
    /// Applied to the next JmpIf/JmpNot as llvm.expect.i1 wrapping.
    pub pending_branch_hint: Option<bool>,

    /// VBC escape analysis tier decisions for this function.
    /// Maps VBC instruction offset to decided RefTier.
    /// Populated from `EscapeAnalysisResult` before instruction lowering.
    /// When present, overrides the LLVM-side local/unknown heuristic.
    vbc_escape_tiers: HashMap<usize, RefTier>,
}

/// Information about a reference stored in a register.
#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    /// The source of the reference (e.g., local alloca, function parameter).
    pub source: ReferenceSource,
    /// Whether the reference has been proven to not escape.
    pub is_no_escape: bool,
    /// Whether the reference has been stored to heap.
    pub escapes_to_heap: bool,
    /// Whether the reference has been returned from the function.
    pub returned: bool,
}

/// Source of a reference value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceSource {
    /// Reference to a local stack allocation.
    LocalAlloca,
    /// Reference from a function parameter.
    Parameter,
    /// Reference loaded from heap.
    HeapLoad,
    /// Reference from other source (conservative).
    Unknown,
}

impl ReferenceInfo {
    /// Create info for a local reference (can be optimized).
    pub fn local() -> Self {
        Self {
            source: ReferenceSource::LocalAlloca,
            is_no_escape: true,
            escapes_to_heap: false,
            returned: false,
        }
    }

    /// Create info for a parameter reference (may escape via caller).
    pub fn parameter() -> Self {
        Self {
            source: ReferenceSource::Parameter,
            is_no_escape: false,
            escapes_to_heap: false,
            returned: false,
        }
    }

    /// Create info for an unknown reference (conservative).
    pub fn unknown() -> Self {
        Self {
            source: ReferenceSource::Unknown,
            is_no_escape: false,
            escapes_to_heap: false,
            returned: false,
        }
    }

    /// Mark the reference as escaping to heap.
    pub fn mark_heap_escape(&mut self) {
        self.escapes_to_heap = true;
        self.is_no_escape = false;
    }

    /// Mark the reference as returned.
    pub fn mark_returned(&mut self) {
        self.returned = true;
        self.is_no_escape = false;
    }

    /// Check if this reference can use Tier 1 (zero overhead).
    pub fn can_use_tier1(&self) -> bool {
        self.is_no_escape && !self.escapes_to_heap && !self.returned
    }
}

/// Statistics for CBGR check elimination.
#[derive(Debug, Default, Clone)]
pub struct CbgrEliminationStats {
    /// Total reference operations encountered.
    pub total_ref_ops: usize,
    /// References proven safe (Tier 1).
    pub proven_safe: usize,
    /// References requiring runtime checks (Tier 0).
    pub runtime_checks: usize,
    /// Estimated time saved in nanoseconds.
    pub estimated_savings_ns: u64,
}

impl CbgrEliminationStats {
    /// Calculate the elimination rate.
    pub fn elimination_rate(&self) -> f64 {
        if self.total_ref_ops == 0 {
            0.0
        } else {
            self.proven_safe as f64 / self.total_ref_ops as f64
        }
    }

    /// Add statistics from a reference operation.
    pub fn record_ref_op(&mut self, is_proven_safe: bool) {
        self.total_ref_ops += 1;
        if is_proven_safe {
            self.proven_safe += 1;
            self.estimated_savings_ns += 15; // ~15ns per check eliminated
        } else {
            self.runtime_checks += 1;
        }
    }
}

impl<'a, 'ctx> FunctionContext<'a, 'ctx> {
    /// Create a new function context.
    pub fn new(
        context: &'ctx Context,
        module: &'a Module<'ctx>,
        function: FunctionValue<'ctx>,
        function_name: impl Into<Text>,
    ) -> Self {
        let builder = context.create_builder();
        let types = TypeLowering::new(context);
        let cbgr = CbgrLowering::new(context);

        Self {
            function,
            module,
            vbc_module: None,
            func_name_index: None,
            func_id_map: None,
            builder,
            types,
            cbgr,
            registers: HashMap::new(),
            register_slots: HashMap::new(),
            blocks: HashMap::new(),
            current_tier: RefTier::Tier0,
            register_tiers: HashMap::new(),
            reference_registers: HashMap::new(),
            instruction_count: 0,
            function_name: function_name.into(),
            cbgr_elimination_stats: CbgrEliminationStats::default(),
            exception_handlers: Vec::new(),
            exception_value_slot: None,
            alloca_mode: false,
            alloca_registers: HashMap::new(),
            alloca_register_types: HashMap::new(),
            string_registers: std::collections::HashSet::new(),
            text_registers: std::collections::HashSet::new(),
            bool_registers: std::collections::HashSet::new(),
            float_registers: std::collections::HashSet::new(),


            variant_float_fields: std::collections::HashSet::new(),
            list_registers: std::collections::HashSet::new(),

            chan_registers: std::collections::HashSet::new(),
            range_registers: std::collections::HashSet::new(),
            flat_range_registers: std::collections::HashSet::new(),
            map_registers: std::collections::HashSet::new(),
            map_list_value_registers: std::collections::HashSet::new(),
            map_string_value_registers: std::collections::HashSet::new(),
            map_copy_source: HashMap::new(),
            refmut_source: HashMap::new(),
            set_registers: std::collections::HashSet::new(),
            deque_registers: std::collections::HashSet::new(),
            btreemap_registers: std::collections::HashSet::new(),
            btreeset_registers: std::collections::HashSet::new(),
            binaryheap_registers: std::collections::HashSet::new(),
            atomic_int_registers: std::collections::HashSet::new(),
            gen_registers: std::collections::HashSet::new(),
            slice_registers: std::collections::HashSet::new(),
            text_iter_registers: std::collections::HashSet::new(),
            map_iter_registers: std::collections::HashSet::new(),

            custom_iter_type_names: HashMap::new(),





            heap_alloc_registers: std::collections::HashSet::new(),

            struct_register_sizes: HashMap::new(),



            element_stride_registers: HashMap::new(),
            ctx_provide_depth: 0,
            struct_list_fields: HashMap::new(),
            struct_string_fields: HashMap::new(),
            obj_register_types: HashMap::new(),
            maybe_inner_types: HashMap::new(),
            variant_registers: std::collections::HashSet::new(),



            closure_return_types: HashMap::new(),
            reg_types: RegisterTypeMap::new(),
            dispatch_table: MethodDispatchTable::new(),
            tuple_element_types: HashMap::new(),
            generic_type_args: HashMap::new(),
            gete_element_ptrs: std::collections::HashMap::new(),
            len_list_overrides: std::collections::HashSet::new(),
            current_vbc_instr_idx: 0,
            func_id_base: 0,
            diagnostics: Vec::new(),
            pending_loop_hints: None,
            pending_branch_hint: None,
            vbc_escape_tiers: HashMap::new(),
        }
    }

    /// Create a new function context with VBC module for FFI support.
    ///
    /// The VBC module provides access to FFI symbol tables, struct layouts,
    /// and other metadata required for zero-cost FFI lowering.
    pub fn with_vbc_module(
        context: &'ctx Context,
        module: &'a Module<'ctx>,
        vbc_module: &'a VbcModule,
        function: FunctionValue<'ctx>,
        function_name: impl Into<Text>,
    ) -> Self {
        let builder = context.create_builder();
        let types = TypeLowering::new(context);
        let cbgr = CbgrLowering::new(context);

        Self {
            function,
            module,
            vbc_module: Some(vbc_module),
            func_name_index: None,
            func_id_map: None,
            builder,
            types,
            cbgr,
            registers: HashMap::new(),
            register_slots: HashMap::new(),
            blocks: HashMap::new(),
            current_tier: RefTier::Tier0,
            register_tiers: HashMap::new(),
            reference_registers: HashMap::new(),
            instruction_count: 0,
            function_name: function_name.into(),
            cbgr_elimination_stats: CbgrEliminationStats::default(),
            exception_handlers: Vec::new(),
            exception_value_slot: None,
            alloca_mode: false,
            alloca_registers: HashMap::new(),
            alloca_register_types: HashMap::new(),
            string_registers: std::collections::HashSet::new(),
            text_registers: std::collections::HashSet::new(),
            bool_registers: std::collections::HashSet::new(),
            float_registers: std::collections::HashSet::new(),


            variant_float_fields: std::collections::HashSet::new(),
            list_registers: std::collections::HashSet::new(),

            chan_registers: std::collections::HashSet::new(),
            range_registers: std::collections::HashSet::new(),
            flat_range_registers: std::collections::HashSet::new(),
            map_registers: std::collections::HashSet::new(),
            map_list_value_registers: std::collections::HashSet::new(),
            map_string_value_registers: std::collections::HashSet::new(),
            map_copy_source: HashMap::new(),
            refmut_source: HashMap::new(),
            set_registers: std::collections::HashSet::new(),
            deque_registers: std::collections::HashSet::new(),
            btreemap_registers: std::collections::HashSet::new(),
            btreeset_registers: std::collections::HashSet::new(),
            binaryheap_registers: std::collections::HashSet::new(),
            atomic_int_registers: std::collections::HashSet::new(),
            gen_registers: std::collections::HashSet::new(),
            slice_registers: std::collections::HashSet::new(),
            text_iter_registers: std::collections::HashSet::new(),
            map_iter_registers: std::collections::HashSet::new(),

            custom_iter_type_names: HashMap::new(),





            heap_alloc_registers: std::collections::HashSet::new(),

            struct_register_sizes: HashMap::new(),



            element_stride_registers: HashMap::new(),
            ctx_provide_depth: 0,
            struct_list_fields: HashMap::new(),
            struct_string_fields: HashMap::new(),
            obj_register_types: HashMap::new(),
            maybe_inner_types: HashMap::new(),
            variant_registers: std::collections::HashSet::new(),



            closure_return_types: HashMap::new(),
            reg_types: RegisterTypeMap::new(),
            dispatch_table: MethodDispatchTable::new(),
            tuple_element_types: HashMap::new(),
            generic_type_args: HashMap::new(),
            gete_element_ptrs: std::collections::HashMap::new(),
            len_list_overrides: std::collections::HashSet::new(),
            current_vbc_instr_idx: 0,
            func_id_base: 0,
            diagnostics: Vec::new(),
            pending_loop_hints: None,
            pending_branch_hint: None,
            vbc_escape_tiers: HashMap::new(),
        }
    }

    /// Emit a structured warning diagnostic.
    pub fn emit_warning(&mut self, category: impl Into<Text>, message: impl Into<Text>) {
        self.diagnostics.push(super::error::LoweringDiagnostic::warning(
            category,
            message,
            self.function_name.clone(),
        ));
    }

    /// Emit a warning for an unimplemented sub-opcode.
    pub fn emit_unimplemented_sub_op(&mut self, category: impl Into<Text>, sub_op: u8) {
        self.diagnostics.push(super::error::LoweringDiagnostic::unimplemented_sub_op(
            category,
            sub_op,
            self.function_name.clone(),
        ));
    }

    /// Take all collected diagnostics, draining the internal list.
    pub fn take_diagnostics(&mut self) -> Vec<super::error::LoweringDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Get the number of diagnostics emitted.
    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    /// Get the VBC module if available.
    ///
    /// Returns None if the context was created without a VBC module.
    /// FFI operations that require module-level metadata will fail
    /// with an appropriate error if no VBC module is set.
    pub fn vbc_module(&self) -> Option<&'a VbcModule> {
        self.vbc_module
    }

    /// Get the function name index for O(1) lookups.
    pub fn func_name_index(&self) -> Option<&FuncNameIndex> {
        self.func_name_index.as_deref()
    }

    /// Set the function name index (shared across all functions in a module).
    pub fn set_func_name_index(&mut self, index: Arc<FuncNameIndex>) {
        self.func_name_index = Some(index);
    }

    /// Set the func_id → LLVM function map for name-collision-safe resolution.
    pub fn set_func_id_map(&mut self, map: Arc<HashMap<u32, FunctionValue<'ctx>>>) {
        self.func_id_map = Some(map);
    }

    /// Resolve a VBC function ID to its LLVM function value.
    /// This is the authoritative lookup that handles name collisions correctly.
    pub fn resolve_func_id(&self, func_id: u32) -> Option<FunctionValue<'ctx>> {
        self.func_id_map.as_ref().and_then(|m| m.get(&func_id).copied())
    }

    /// Get the LLVM function.
    pub fn function(&self) -> FunctionValue<'ctx> {
        self.function
    }

    /// Get the builder.
    pub fn builder(&self) -> &Builder<'ctx> {
        &self.builder
    }

    /// Get mutable builder.
    pub fn builder_mut(&mut self) -> &mut Builder<'ctx> {
        &mut self.builder
    }

    /// Get the type lowering helper.
    pub fn types(&self) -> &TypeLowering<'ctx> {
        &self.types
    }

    /// Get the unified register type map.
    pub fn reg_types(&self) -> &RegisterTypeMap {
        &self.reg_types
    }

    /// Get mutable register type map.
    pub fn reg_types_mut(&mut self) -> &mut RegisterTypeMap {
        &mut self.reg_types
    }

    /// Get the method dispatch table.
    pub fn dispatch_table(&self) -> &MethodDispatchTable {
        &self.dispatch_table
    }

    /// Get the CBGR lowering helper.
    pub fn cbgr(&self) -> &CbgrLowering<'ctx> {
        &self.cbgr
    }

    /// Get mutable CBGR lowering helper.
    pub fn cbgr_mut(&mut self) -> &mut CbgrLowering<'ctx> {
        &mut self.cbgr
    }

    /// Get builder and mutable CBGR helper together.
    ///
    /// This method allows simultaneous access to both the builder (for generating
    /// instructions) and the CBGR helper (for reference operations) by splitting
    /// the borrow across the two fields.
    pub fn builder_and_cbgr(&mut self) -> (&Builder<'ctx>, &mut CbgrLowering<'ctx>) {
        (&self.builder, &mut self.cbgr)
    }

    /// Set the current CBGR tier.
    pub fn set_tier(&mut self, tier: RefTier) {
        self.current_tier = tier;
    }

    /// Get the current CBGR tier.
    pub fn current_tier(&self) -> RefTier {
        self.current_tier
    }

    /// Get the LLVM context.
    ///
    /// This is needed for FFI lowering and other operations that require
    /// creating new types or values.
    pub fn llvm_context(&self) -> &'ctx Context {
        self.types.context()
    }

    /// Get the LLVM module that contains this function.
    ///
    /// Returns the parent module of the function being lowered.
    /// This is needed for declaring external functions for FFI calls.
    pub fn get_module(&self) -> &'a Module<'ctx> {
        self.module
    }

    // ========================================================================
    // Per-Register Tier Tracking (CBGR Elimination)
    // ========================================================================

    /// Set the CBGR tier for a specific register.
    ///
    /// This is used for escape-analysis-based optimization where individual
    /// registers can have different tiers based on their escape behavior.
    pub fn set_register_tier(&mut self, reg: u16, tier: RefTier) {
        self.register_tiers.insert(reg, tier);
    }

    /// Get the CBGR tier for a specific register.
    ///
    /// Returns the per-register tier if set, otherwise falls back to the
    /// function's default tier.
    pub fn get_register_tier(&self, reg: u16) -> RefTier {
        self.register_tiers
            .get(&reg)
            .copied()
            .unwrap_or(self.current_tier)
    }

    /// Set VBC escape analysis tier decisions for this function.
    ///
    /// Maps VBC instruction offset to RefTier. Called before instruction lowering
    /// to provide escape analysis results from `VbcEscapeAnalyzer`.
    pub fn set_vbc_escape_tiers(&mut self, tiers: HashMap<usize, RefTier>) {
        self.vbc_escape_tiers = tiers;
    }

    /// Look up the escape-analysis-decided tier for a VBC instruction offset.
    ///
    /// Returns `Some(RefTier)` if the escape analysis produced a decision for
    /// this instruction, `None` otherwise.
    pub fn get_vbc_escape_tier(&self, offset: usize) -> Option<RefTier> {
        self.vbc_escape_tiers.get(&offset).copied()
    }

    /// Mark a register as holding a C runtime string pointer (verum_text_alloc layout).
    /// Mutually exclusive with text_register (compiled text.vr flat layout).
    pub fn mark_string_register(&mut self, reg: u16) {
        self.string_registers.insert(reg);
        self.text_registers.remove(&reg);
        self.reg_types.set(reg, RegisterType::Text { owned: false, compiled_layout: false });
    }

    /// Check if a register holds a C runtime string pointer.
    pub fn is_string_register(&self, reg: u16) -> bool {
        self.string_registers.contains(&reg)
    }

    /// Mark a register as holding a compiled text.vr Text* pointer (flat {ptr, len, cap}).
    /// Mutually exclusive with string_register (C runtime layout).
    pub fn mark_text_register(&mut self, reg: u16) {
        self.text_registers.insert(reg);
        self.string_registers.remove(&reg);
        self.reg_types.set(reg, RegisterType::Text { owned: false, compiled_layout: true });
    }

    /// Check if a register holds a compiled text.vr Text* pointer.
    pub fn is_text_register(&self, reg: u16) -> bool {
        self.text_registers.contains(&reg)
    }

    /// Mark a register as holding an "owned" text object (allocated by Concat/ToString/etc.).
    /// Owned text must be freed when the register is overwritten or at function exit.
    pub fn mark_owned_text_register(&mut self, reg: u16) {
        self.reg_types.mark_owned_text(reg);
    }

    /// Check if a register holds an owned text object that needs freeing.
    pub fn is_owned_text_register(&self, reg: u16) -> bool {
        self.reg_types.is_owned_text(reg)
    }

    /// Unmark a register as owned (e.g., when transferring ownership to a return value).
    pub fn unmark_owned_text_register(&mut self, reg: u16) {
        self.reg_types.unmark_owned_text(reg);
    }

    /// Mark a register as holding a Heap<T> allocation (needs dealloc on return).
    pub fn mark_heap_alloc_register(&mut self, reg: u16) {
        self.heap_alloc_registers.insert(reg);
    }

    /// Get all heap allocation registers (for drop cleanup at function exit).
    pub fn heap_alloc_registers(&self) -> Vec<u16> {
        self.heap_alloc_registers.iter().copied().collect()
    }

    /// Get all owned text registers (for cleanup at function exit).
    pub fn owned_text_registers(&self) -> Vec<u16> {
        self.reg_types.owned_text_registers()
    }

    /// Mark a register as holding a Maybe<Text> variant object (from strip_prefix etc.).
    /// Used by AsVar to propagate string_register marking to extracted payload.
    pub fn mark_maybe_string_register(&mut self, reg: u16) {
        self.reg_types.mark_maybe_text_inner(reg);
    }

    /// Check if a register holds a Maybe<Text> variant object.
    pub fn is_maybe_string_register(&self, reg: u16) -> bool {
        self.reg_types.is_maybe_text(reg)
    }

    /// Set the inner struct type name for a register holding Maybe<Heap<T>>.
    /// After AsVar/unwrap, this propagates to obj_register_type.
    pub fn set_maybe_inner_type(&mut self, reg: u16, type_name: String) {
        self.maybe_inner_types.insert(reg, type_name);
    }

    /// Get the inner struct type name for a Maybe<Heap<T>> register.
    pub fn get_maybe_inner_type(&self, reg: u16) -> Option<&str> {
        self.maybe_inner_types.get(&reg).map(|s| s.as_str())
    }

    /// Mark a register as holding a variant value (heap-allocated variant or null).
    /// Used by Ref/RefMut to pass through variant pointers instead of taking alloca addresses.
    pub fn mark_variant_register(&mut self, reg: u16) {
        self.variant_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Variant { type_name: None });
    }

    /// Check if a register holds a variant value.
    pub fn is_variant_register(&self, reg: u16) -> bool {
        self.variant_registers.contains(&reg)
    }

    /// Mark a register as a pass-through reference (value, not pointer).
    /// Used when Ref on a primitive just copies the value instead of creating a pointer.
    pub fn mark_pass_through_ref(&mut self, reg: u16) {
        self.reg_types.mark_pass_through_ref(reg);
    }

    /// Check if a register is a pass-through reference.
    pub fn is_pass_through_ref(&self, reg: u16) -> bool {
        self.reg_types.is_pass_through_ref(reg)
    }

    /// Mark a list register as containing pass-through ref elements (e.g., List<Heap<T>>).
    pub fn mark_pass_through_ref_list(&mut self, reg: u16) {
        self.reg_types.mark_pass_through_ref_list(reg);
    }

    /// Check if a list register contains pass-through ref elements.
    pub fn is_pass_through_ref_list(&self, reg: u16) -> bool {
        self.reg_types.is_pass_through_ref_list(reg)
    }

    /// Mark a register as holding a generic type parameter value-as-pointer.
    /// In compiled module functions, generic params (K, V, T) are `ptr` where the
    /// pointer IS the value (via inttoptr). Deref should ptrtoint, not load.
    pub fn mark_generic_ptr_register(&mut self, reg: u16) {
        self.reg_types.set(reg, RegisterType::GenericParam);
    }

    /// Check if a register holds a generic type parameter value-as-pointer.
    pub fn is_generic_ptr_register(&self, reg: u16) -> bool {
        self.reg_types.is_generic_param(reg)
    }

    /// Store the return type for a closure register (from NewClosure or fn-type parameter).
    pub fn set_closure_return_type(&mut self, reg: u16, ret_type: verum_vbc::types::TypeRef) {
        self.closure_return_types.insert(reg, ret_type);
    }

    /// Get the return type for a closure register, if tracked.
    pub fn get_closure_return_type(&self, reg: u16) -> Option<&verum_vbc::types::TypeRef> {
        self.closure_return_types.get(&reg)
    }

    /// Store the element types for a tuple register (from function return or Pack).
    pub fn set_tuple_element_types(&mut self, reg: u16, types: Vec<verum_vbc::types::TypeRef>) {
        self.tuple_element_types.insert(reg, types);
    }

    /// Get the element types for a tuple register, if tracked.
    pub fn get_tuple_element_types(&self, reg: u16) -> Option<&Vec<verum_vbc::types::TypeRef>> {
        self.tuple_element_types.get(&reg)
    }

    /// Store generic type args for a register holding a generic struct instance.
    /// E.g., for `Pair<List<Int>, Text>`, stores [Instantiated{LIST,[INT]}, Concrete(TEXT)].
    pub fn set_generic_type_args(&mut self, reg: u16, args: Vec<verum_vbc::types::TypeRef>) {
        use verum_vbc::types::{TypeRef, TypeId};
        // Skip type args containing unresolved generic params or type-erased PTR.
        // Compiled stdlib functions use Generic(K/V) or Concrete(PTR) for all params,
        // which carry no useful type info and would shadow the legacy register tracking
        // (map_list_value, string_registers, etc.) that correctly identifies types.
        let is_useless = |a: &TypeRef| match a {
            TypeRef::Generic(_) => true,
            TypeRef::Concrete(tid) if *tid == TypeId::PTR => true,
            _ => false,
        };
        if args.iter().any(is_useless) {
            return;
        }
        self.generic_type_args.insert(reg, args);
    }

    /// Get the generic type args for a register, if tracked.
    pub fn get_generic_type_args(&self, reg: u16) -> Option<&Vec<verum_vbc::types::TypeRef>> {
        self.generic_type_args.get(&reg)
    }

    /// Save the element pointer from GetE (pointer into list backing array).
    /// Used by Ref to create heap-stable references instead of dangling stack allocas.
    pub fn set_gete_element_ptr(&mut self, reg: u16, ptr: PointerValue<'ctx>) {
        self.gete_element_ptrs.insert(reg, ptr);
    }

    /// Get the element pointer from GetE, if this register was set by a list GetE.
    pub fn get_gete_element_ptr(&self, reg: u16) -> Option<PointerValue<'ctx>> {
        self.gete_element_ptrs.get(&reg).copied()
    }

    /// Mark a register as holding a Maybe<&T> variant from compiled module (payload is a pointer).
    pub fn mark_ref_payload_register(&mut self, reg: u16) {
        self.reg_types.mark_maybe_ref_inner(reg);
    }

    /// Check if a register holds a Maybe<&T> variant with pointer payload.
    pub fn is_ref_payload_register(&self, reg: u16) -> bool {
        self.reg_types.is_maybe_ref(reg)
    }

    /// Mark a register as holding a boolean value.
    pub fn mark_bool_register(&mut self, reg: u16) {
        self.bool_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Bool);
    }

    /// Check if a register holds a boolean value.
    pub fn is_bool_register(&self, reg: u16) -> bool {
        self.bool_registers.contains(&reg)
    }

    /// Mark a register as holding a float value.
    pub fn mark_float_register(&mut self, reg: u16) {
        self.float_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Float);
    }

    /// Set pre-scanned float registers (survives set_register clearing).
    pub fn set_prescan_float_registers(&mut self, regs: std::collections::HashSet<u16>) {
        self.reg_types.set_prescan_float(regs);
    }

    /// Check if a register was identified as float by VBC prescan (BinaryF/UnaryF operand).
    pub fn is_prescan_float_register(&self, reg: u16) -> bool {
        self.reg_types.is_prescan_float(reg)
    }

    /// Set all prescan text registers from a batch.
    pub fn set_prescan_text_registers(&mut self, regs: std::collections::HashSet<u16>) {
        self.reg_types.set_prescan_text(regs);
    }

    /// Mark a register as holding a Text value based on type pre-pass analysis.
    /// Unlike text_registers, these survive set_register() clearing.
    pub fn mark_prescan_text_register(&mut self, reg: u16) {
        self.reg_types.mark_prescan_text(reg);
    }

    /// Check if a register was identified as Text by the type pre-pass.
    pub fn is_prescan_text_register(&self, reg: u16) -> bool {
        self.reg_types.is_prescan_text(reg)
    }

    /// Check if a register holds a float value.
    pub fn is_float_register(&self, reg: u16) -> bool {
        self.float_registers.contains(&reg)
    }

    /// Mark a variant register field as containing a float value.
    pub fn mark_variant_float_field(&mut self, variant_reg: u16, field_idx: u32) {
        self.variant_float_fields.insert((variant_reg, field_idx));
    }

    /// Check if a variant register field contains a float value.
    pub fn is_variant_float_field(&self, variant_reg: u16, field_idx: u32) -> bool {
        self.variant_float_fields.contains(&(variant_reg, field_idx))
    }

    /// Copy variant float field tracking from one register to another.
    pub fn copy_variant_float_fields(&mut self, from: u16, to: u16) {
        let fields: Vec<u32> = self.variant_float_fields.iter()
            .filter(|(r, _)| *r == from)
            .map(|(_, f)| *f)
            .collect();
        for f in fields {
            self.variant_float_fields.insert((to, f));
        }
    }

    /// Mark a register as holding a list header pointer.
    pub fn mark_list_register(&mut self, reg: u16) {
        self.list_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::List { element: None });
    }

    /// Check if a register holds a list header pointer.
    pub fn is_list_register(&self, reg: u16) -> bool {
        self.list_registers.contains(&reg)
    }

    /// Mark a VBC instruction index where Len's arr register is known to be a list.
    /// Used for flow-sensitive dispatch when registers are reused for different types.
    pub fn mark_len_list_override(&mut self, instr_idx: usize) {
        self.len_list_overrides.insert(instr_idx);
    }

    /// Check if a VBC instruction index has a flow-sensitive list override for Len.
    pub fn is_len_list_override(&self, instr_idx: usize) -> bool {
        self.len_list_overrides.contains(&instr_idx)
    }

    /// Set the current VBC instruction index being lowered.
    pub fn set_current_vbc_instr_idx(&mut self, idx: usize) {
        self.current_vbc_instr_idx = idx;
    }

    /// Get the current VBC instruction index being lowered.
    pub fn current_vbc_instr_idx(&self) -> usize {
        self.current_vbc_instr_idx
    }

    /// Set the function ID base offset for resolving Call targets in merged stdlib modules.
    pub fn set_func_id_base(&mut self, base: u32) {
        self.func_id_base = base;
    }

    /// Get the function ID base offset.
    pub fn func_id_base(&self) -> u32 {
        self.func_id_base
    }

    /// Mark a list register as containing string elements (List<Text>).
    pub fn mark_string_list_register(&mut self, reg: u16) {
        self.reg_types.mark_list_text_elements(reg);
    }

    /// Check if a list register contains string elements.
    pub fn is_string_list_register(&self, reg: u16) -> bool {
        self.reg_types.is_string_list(reg)
    }

    /// Mark a register as holding a channel pointer.
    pub fn mark_chan_register(&mut self, reg: u16) {
        self.chan_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Channel { element: None });
    }

    /// Check if a register holds a channel pointer.
    pub fn is_chan_register(&self, reg: u16) -> bool {
        self.chan_registers.contains(&reg)
    }

    /// Mark a register as holding a generator handle.
    pub fn mark_gen_register(&mut self, reg: u16) {
        self.gen_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Generator);
    }

    /// Check if a register holds a generator handle.
    pub fn is_gen_register(&self, reg: u16) -> bool {
        self.gen_registers.contains(&reg)
    }

    /// Mark a register as holding a range object (type_id 517).
    pub fn mark_range_register(&mut self, reg: u16) {
        self.range_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Range { flat: false });
    }

    /// Check if a register holds a range object.
    pub fn is_range_register(&self, reg: u16) -> bool {
        self.range_registers.contains(&reg)
    }

    /// Mark a register as holding a flat range (from NewRange/verum_range_new).
    pub fn mark_flat_range_register(&mut self, reg: u16) {
        self.flat_range_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Range { flat: true });
    }

    /// Check if a register holds a flat range.
    pub fn is_flat_range_register(&self, reg: u16) -> bool {
        self.flat_range_registers.contains(&reg)
    }

    /// Mark a register as holding a map header pointer.
    pub fn mark_map_register(&mut self, reg: u16) {
        self.map_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Map { key: None, value: None });
    }

    /// Check if a register holds a map header pointer.
    pub fn is_map_register(&self, reg: u16) -> bool {
        self.map_registers.contains(&reg)
    }

    /// Mark a map register as having list values (from insert with list_register).
    pub fn mark_map_list_value(&mut self, reg: u16) {
        self.map_list_value_registers.insert(reg);
        self.reg_types.mark_map_list_values(reg);
    }

    /// Check if a map register has list values.
    pub fn is_map_list_value(&self, reg: u16) -> bool {
        self.map_list_value_registers.contains(&reg)
    }

    /// Check if a map register (or any register in its Mov chain) has list values.
    pub fn is_map_list_value_chain(&self, reg: u16) -> bool {
        if self.map_list_value_registers.contains(&reg) { return true; }
        let mut current = reg;
        while let Some(src) = self.map_copy_source.get(&current).copied() {
            if self.map_list_value_registers.contains(&src) { return true; }
            current = src;
        }
        false
    }

    /// Mark a map register as having string/text values.
    pub fn mark_map_string_value(&mut self, reg: u16) {
        self.map_string_value_registers.insert(reg);
        self.reg_types.mark_map_text_values(reg);
    }

    /// Check if a map register has string/text values.
    pub fn is_map_string_value(&self, reg: u16) -> bool {
        self.map_string_value_registers.contains(&reg)
    }

    /// Check if a map register (or any register in its Mov chain) has string values.
    pub fn is_map_string_value_chain(&self, reg: u16) -> bool {
        if self.map_string_value_registers.contains(&reg) { return true; }
        let mut current = reg;
        while let Some(src) = self.map_copy_source.get(&current).copied() {
            if self.map_string_value_registers.contains(&src) { return true; }
            current = src;
        }
        false
    }

    /// Record that `dst` is a Mov copy of `src` for map registers.
    pub fn set_map_copy_source(&mut self, dst: u16, src: u16) {
        self.map_copy_source.insert(dst, src);
    }

    /// Record that `dst` is a RefMut of `src`.
    pub fn set_refmut_source(&mut self, dst: u16, src: u16) {
        self.refmut_source.insert(dst, src);
    }

    /// Get the original source register for a RefMut destination.
    pub fn get_refmut_source(&self, dst: u16) -> Option<u16> {
        self.refmut_source.get(&dst).copied()
    }

    /// Mark map_list_value on a register AND all its copy sources up the chain.
    pub fn mark_map_list_value_chain(&mut self, reg: u16) {
        self.map_list_value_registers.insert(reg);
        self.reg_types.mark_map_list_values(reg);
        let mut current = reg;
        while let Some(src) = self.map_copy_source.get(&current).copied() {
            self.map_list_value_registers.insert(src);
            self.reg_types.mark_map_list_values(src);
            current = src;
        }
    }

    /// Mark map_string_value on a register AND all its copy sources up the chain.
    pub fn mark_map_string_value_chain(&mut self, reg: u16) {
        self.map_string_value_registers.insert(reg);
        self.reg_types.mark_map_text_values(reg);
        let mut current = reg;
        while let Some(src) = self.map_copy_source.get(&current).copied() {
            self.map_string_value_registers.insert(src);
            self.reg_types.mark_map_text_values(src);
            current = src;
        }
    }

    /// Mark a register as holding a set header pointer.
    pub fn mark_set_register(&mut self, reg: u16) {
        self.set_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Set { element: None });
    }

    /// Check if a register holds a set header pointer.
    pub fn is_set_register(&self, reg: u16) -> bool {
        self.set_registers.contains(&reg)
    }

    /// Mark a register as holding a deque pointer.
    pub fn mark_deque_register(&mut self, reg: u16) {
        self.deque_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Deque { element: None });
    }

    /// Check if a register holds a deque pointer.
    pub fn is_deque_register(&self, reg: u16) -> bool {
        self.deque_registers.contains(&reg)
    }

    /// Mark a register as holding a BTreeMap pointer.
    pub fn mark_btreemap_register(&mut self, reg: u16) {
        self.btreemap_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::BTreeMap { key: None, value: None });
    }

    /// Check if a register holds a BTreeMap pointer.
    pub fn is_btreemap_register(&self, reg: u16) -> bool {
        self.btreemap_registers.contains(&reg)
    }

    /// Mark a register as holding a BTreeSet pointer.
    pub fn mark_btreeset_register(&mut self, reg: u16) {
        self.btreeset_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::BTreeSet { element: None });
    }

    /// Check if a register holds a BTreeSet pointer.
    pub fn is_btreeset_register(&self, reg: u16) -> bool {
        self.btreeset_registers.contains(&reg)
    }

    /// Mark a register as holding a BinaryHeap pointer.
    pub fn mark_binaryheap_register(&mut self, reg: u16) {
        self.binaryheap_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::BinaryHeap { element: None });
    }

    /// Check if a register holds a BinaryHeap pointer.
    pub fn is_binaryheap_register(&self, reg: u16) -> bool {
        self.binaryheap_registers.contains(&reg)
    }

    /// Mark a register as holding an AtomicInt/AtomicBool pointer.
    pub fn mark_atomic_int_register(&mut self, reg: u16) {
        self.atomic_int_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Atomic);
    }

    /// Check if a register holds an AtomicInt/AtomicBool pointer.
    pub fn is_atomic_int_register(&self, reg: u16) -> bool {
        self.atomic_int_registers.contains(&reg)
    }

    /// Mark a register as holding a slice pointer ({ptr, len} from Pack).
    pub fn mark_slice_register(&mut self, reg: u16) {
        self.slice_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::Slice { element: None });
    }

    /// Check if a register holds a slice pointer.
    pub fn is_slice_register(&self, reg: u16) -> bool {
        self.slice_registers.contains(&reg)
    }

    /// Mark a register as holding a text iterator.
    pub fn mark_text_iter_register(&mut self, reg: u16) {
        self.text_iter_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::TextIterator);
    }

    /// Check if a register holds a text iterator.
    pub fn is_text_iter_register(&self, reg: u16) -> bool {
        self.text_iter_registers.contains(&reg)
    }

    /// Mark a register as holding a map iterator.
    pub fn mark_map_iter_register(&mut self, reg: u16) {
        self.map_iter_registers.insert(reg);
        self.reg_types.set(reg, RegisterType::MapIterator);
    }

    /// Check if a register holds a map iterator.
    pub fn is_map_iter_register(&self, reg: u16) -> bool {
        self.map_iter_registers.contains(&reg)
    }

    /// Mark a register as holding a custom iterator (user-defined type with has_next/next).
    pub fn mark_custom_iter_register(&mut self, reg: u16, type_name: String) {
        self.reg_types.set(reg, RegisterType::CustomIterator { type_name: type_name.clone() });
        self.custom_iter_type_names.insert(reg, type_name);
    }

    /// Check if a register holds a custom iterator.
    pub fn is_custom_iter_register(&self, reg: u16) -> bool {
        self.reg_types.is_custom_iterator(reg)
    }

    /// Get the type name for a custom iterator register.
    pub fn get_custom_iter_type(&self, reg: u16) -> Option<&str> {
        self.custom_iter_type_names.get(&reg).map(|s| s.as_str())
    }

    /// Mark a register as holding a struct pointer (stack-spilled).
    pub fn mark_struct_register(&mut self, reg: u16, field_count: u32) {
        self.struct_register_sizes.insert(reg, field_count);
        self.reg_types.set(reg, RegisterType::Struct { type_name: "unknown".to_string(), size: field_count });
    }

    /// Check if a register holds a struct pointer.
    pub fn is_struct_register(&self, reg: u16) -> bool {
        self.reg_types.is_struct(reg)
    }

    /// Get field count for a struct register.
    pub fn struct_register_field_count(&self, reg: u16) -> u32 {
        self.struct_register_sizes.get(&reg).copied().unwrap_or(0)
    }

    /// Mark a struct field as containing a list value.
    /// Called from SetF when storing a list_register into a struct field.
    pub fn mark_struct_list_field(&mut self, obj_reg: u16, field_idx: u32) {
        self.struct_list_fields.insert((obj_reg, field_idx), true);
    }

    /// Check if a struct field is known to contain a list value.
    /// Called from GetF to propagate list_register tracking.
    pub fn is_struct_list_field(&self, obj_reg: u16, field_idx: u32) -> bool {
        self.struct_list_fields.get(&(obj_reg, field_idx)).copied().unwrap_or(false)
    }

    /// Mark a struct field as containing a string value.
    pub fn mark_struct_string_field(&mut self, obj_reg: u16, field_idx: u32) {
        self.struct_string_fields.insert((obj_reg, field_idx), true);
    }

    /// Check if a struct field is known to contain a string value.
    pub fn is_struct_string_field(&self, obj_reg: u16, field_idx: u32) -> bool {
        self.struct_string_fields.get(&(obj_reg, field_idx)).copied().unwrap_or(false)
    }

    /// Mark a register as holding a pointer to an inline struct (no object header).
    /// Set by offset() when the element type is a struct within an array.
    pub fn mark_inline_struct_register(&mut self, reg: u16) {
        self.reg_types.set(reg, RegisterType::InlineStruct { type_name: "unknown".to_string(), size: 0 });
    }

    /// Check if a register holds an inline struct pointer (no header).
    pub fn is_inline_struct_register(&self, reg: u16) -> bool {
        self.reg_types.is_inline_struct(reg)
    }

    /// Set the element stride (in bytes) for a pointer register.
    /// Used by offset() to compute correct byte offsets for struct arrays.
    pub fn set_element_stride(&mut self, reg: u16, stride: u64) {
        self.element_stride_registers.insert(reg, stride);
    }

    /// Get the element stride for a pointer register (default: 8 bytes).
    pub fn get_element_stride(&self, reg: u16) -> u64 {
        self.element_stride_registers.get(&reg).copied().unwrap_or(8)
    }

    /// Set the object type name for a register.
    /// Used by GetF to look up field metadata from the correct type.
    pub fn set_obj_register_type(&mut self, reg: u16, type_name: String) {
        self.obj_register_types.insert(reg, type_name);
    }

    /// Get the object type name for a register, if tracked.
    pub fn get_obj_register_type(&self, reg: u16) -> Option<&str> {
        self.obj_register_types.get(&reg).map(|s| s.as_str())
    }

    /// Get the current context provide nesting depth.
    pub fn ctx_provide_depth(&self) -> u64 {
        self.ctx_provide_depth
    }

    /// Increment the context provide nesting depth (called on CtxProvide).
    pub fn increment_ctx_provide_depth(&mut self) {
        self.ctx_provide_depth += 1;
    }

    /// Decrement the context provide nesting depth (called on CtxEnd).
    pub fn decrement_ctx_provide_depth(&mut self) {
        self.ctx_provide_depth = self.ctx_provide_depth.saturating_sub(1);
    }

    /// Register a reference in a register for escape tracking.
    ///
    /// Call this when creating a reference (Ref/RefMut instructions).
    pub fn register_reference(&mut self, reg: u16, info: ReferenceInfo) {
        // Set initial tier based on escape analysis (before moving info)
        let tier = if info.can_use_tier1() {
            RefTier::Tier1
        } else {
            RefTier::Tier0
        };
        self.register_tiers.insert(reg, tier);
        self.reference_registers.insert(reg, info);
    }

    /// Mark a register's reference as escaping to heap.
    ///
    /// Call this when storing a reference to heap memory.
    pub fn mark_heap_escape(&mut self, reg: u16) {
        if let Some(info) = self.reference_registers.get_mut(&reg) {
            info.mark_heap_escape();
            // Downgrade to Tier0 since it now escapes
            self.register_tiers.insert(reg, RefTier::Tier0);
        }
    }

    /// Mark a register's reference as returned from function.
    ///
    /// Call this when a reference is returned.
    pub fn mark_returned(&mut self, reg: u16) {
        if let Some(info) = self.reference_registers.get_mut(&reg) {
            info.mark_returned();
            // Downgrade to Tier0 since it now escapes via return
            self.register_tiers.insert(reg, RefTier::Tier0);
        }
    }

    /// Check if a register contains a reference that can use Tier 1.
    pub fn can_use_tier1(&self, reg: u16) -> bool {
        self.reference_registers
            .get(&reg)
            .map(|info| info.can_use_tier1())
            .unwrap_or(false)
    }

    /// Get the effective tier for a reference operation.
    ///
    /// This considers both the per-register tier and the function's default tier,
    /// and records statistics for CBGR elimination.
    pub fn get_effective_ref_tier(&mut self, reg: u16) -> RefTier {
        let tier = self.get_register_tier(reg);
        let is_proven_safe = matches!(tier, RefTier::Tier1 | RefTier::Tier2);
        self.cbgr_elimination_stats.record_ref_op(is_proven_safe);
        tier
    }

    /// Get the CBGR elimination statistics.
    pub fn cbgr_elimination_stats(&self) -> &CbgrEliminationStats {
        &self.cbgr_elimination_stats
    }

    /// Increment the instruction counter.
    pub fn increment_instruction_count(&mut self) {
        self.instruction_count += 1;
    }

    /// Get the instruction count.
    pub fn instruction_count(&self) -> usize {
        self.instruction_count
    }

    /// Get the function name.
    pub fn function_name(&self) -> &Text {
        &self.function_name
    }

    // ========================================================================
    // Register Management
    // ========================================================================

    /// Enable alloca-based register storage for multi-block functions.
    /// When enabled, all register set/get operations use stack allocas
    /// instead of SSA values, which ensures values persist across basic blocks.
    /// LLVM's mem2reg pass will optimize these to SSA later.
    pub fn enable_alloca_mode(&mut self) {
        self.alloca_mode = true;
    }

    pub fn is_alloca_mode(&self) -> bool {
        self.alloca_mode
    }

    /// Get a register value.
    /// In alloca mode, loads from the alloca slot; otherwise reads from the SSA value map.
    /// Float registers use f64 allocas; all others use i64.
    pub fn get_register(&self, reg: u16) -> Result<BasicValueEnum<'ctx>> {
        // Phase 5: Warn on use-after-transfer (FFI ownership)
        if self.reg_types.is_consumed_ffi(reg) {
            tracing::warn!(
                "FFI: use-after-transfer — register r{} was consumed by TransferTo FFI call",
                reg
            );
        }
        if self.alloca_mode {
            if let Some(&alloca_ptr) = self.alloca_registers.get(&reg) {
                // Load with the correct type based on the alloca type.
                // Typed allocas (alloca double, alloca ptr) preserve LLVM type info.
                let load_type = self.alloca_register_types.get(&reg)
                    .copied()
                    .unwrap_or(BasicTypeEnum::IntType(self.types.i64_type()));
                self.builder
                    .build_load(load_type, alloca_ptr, &format!("r{}", reg))
                    .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
            } else {
                // Register not yet allocated — return zero
                Ok(self.types.i64_type().const_zero().into())
            }
        } else {
            self.registers
                .get(&reg)
                .copied()
                .ok_or_else(|| LlvmLoweringError::InvalidRegister(reg))
        }
    }

    /// Set a register value.
    /// In alloca mode, stores to the alloca slot; otherwise stores the SSA value directly.
    /// All alloca slots uniformly use i64 type. This ensures that VBC registers reused
    /// across branches with different types (ptr, i1, i64) share a single alloca.
    /// Values are coerced to i64 on store (zext for small ints, ptrtoint for pointers).
    pub fn set_register(&mut self, reg: u16, value: BasicValueEnum<'ctx>) {
        // Clear stale type marks — callers that need string/bool/float marks
        // will re-add them after this call.
        // Clear unified type map (covers struct, inline_struct, custom_iter, generic_param, etc.)
        self.reg_types.clear(reg);
        self.string_registers.remove(&reg);
        self.text_registers.remove(&reg);
        self.bool_registers.remove(&reg);
        self.float_registers.remove(&reg);
        self.list_registers.remove(&reg);
        self.chan_registers.remove(&reg);
        self.range_registers.remove(&reg);
        self.flat_range_registers.remove(&reg);
        self.map_registers.remove(&reg);
        self.set_registers.remove(&reg);
        self.deque_registers.remove(&reg);
        self.btreemap_registers.remove(&reg);
        self.btreeset_registers.remove(&reg);
        self.binaryheap_registers.remove(&reg);
        self.atomic_int_registers.remove(&reg);
        self.slice_registers.remove(&reg);
        self.text_iter_registers.remove(&reg);
        self.map_iter_registers.remove(&reg);
        self.struct_register_sizes.remove(&reg);
        self.element_stride_registers.remove(&reg);
        self.obj_register_types.remove(&reg);
        self.gete_element_ptrs.remove(&reg);
        // owned_text cleared by reg_types.clear(reg) above — prevents Ret handler from
        // freeing a register that now holds a non-text value after VBC temp register reuse.

        if self.alloca_mode {
            let i64_ty = self.types.i64_type();

            // alloca_register_types updated per-branch below

            // Coerce value to i64 for uniform storage (or f64 directly for floats)
            let i64_value: BasicValueEnum<'ctx> = match value {
                BasicValueEnum::IntValue(v) => {
                    let bw = v.get_type().get_bit_width();
                    if bw < 64 {
                        self.builder
                            .build_int_z_extend(v, i64_ty, &format!("r{}_widen", reg))
                            .expect("zext should not fail")
                            .into()
                    } else {
                        value
                    }
                }
                BasicValueEnum::PointerValue(v) => {
                    // Store pointer through typed ptr alloca.
                    // SROA can track pointer provenance through `alloca ptr`.
                    let ptr_type = self.types.ptr_type();
                    if let Some(&existing_ptr) = self.alloca_registers.get(&reg) {
                        let _ = self.builder.build_store(existing_ptr, v);
                    } else {
                        let entry = self.function.get_first_basic_block()
                            .expect("function must have entry block");
                        let saved_block = self.builder.get_insert_block();
                        if let Some(first_instr) = entry.get_first_instruction() {
                            self.builder.position_before(&first_instr);
                        } else {
                            self.builder.position_at_end(entry);
                        }
                        let alloca = self.builder
                            .build_alloca(BasicTypeEnum::PointerType(ptr_type), &format!("r{}_ptr", reg))
                            .expect("alloca should not fail");
                        if let Some(block) = saved_block {
                            self.builder.position_at_end(block);
                        }
                        self.alloca_registers.insert(reg, alloca);
                        self.alloca_register_types.insert(reg, ptr_type.into());
                        let _ = self.builder.build_store(alloca, v);
                    }
                    self.alloca_register_types.insert(reg, BasicTypeEnum::PointerType(ptr_type));
                    return;
                }
                BasicValueEnum::FloatValue(v) => {
                    // Store f64 through typed double alloca — enables SROA scalar promotion.
                    // `alloca double` + `store double` + `load double` is a clean pattern
                    // that SROA/mem2reg promotes to SSA f64 registers.
                    let f64_type = self.types.f64_type();
                    if let Some(&existing_ptr) = self.alloca_registers.get(&reg) {
                        let _ = self.builder.build_store(existing_ptr, v);
                    } else {
                        // First use — create typed f64 alloca
                        let entry = self.function.get_first_basic_block()
                            .expect("function must have entry block");
                        let saved_block = self.builder.get_insert_block();
                        if let Some(first_instr) = entry.get_first_instruction() {
                            self.builder.position_before(&first_instr);
                        } else {
                            self.builder.position_at_end(entry);
                        }
                        let alloca = self.builder
                            .build_alloca(BasicTypeEnum::FloatType(f64_type), &format!("r{}_f64", reg))
                            .expect("alloca should not fail");
                        if let Some(block) = saved_block {
                            self.builder.position_at_end(block);
                        }
                        self.alloca_registers.insert(reg, alloca);
                        self.alloca_register_types.insert(reg, f64_type.into());
                        let _ = self.builder.build_store(alloca, v);
                    }
                    // Mark this register as currently holding a float value
                    self.alloca_register_types.insert(reg, BasicTypeEnum::FloatType(f64_type));
                    return;
                }
                BasicValueEnum::StructValue(sv) => {
                    // In alloca mode all registers are i64. Structs (e.g., CBGR refs,
                    // unit type) must be coerced:
                    //  - Empty struct (unit): store 0
                    //  - Non-empty struct: spill to a stack alloca and store the
                    //    alloca pointer as i64 so downstream code can recover it
                    //    via inttoptr.
                    let field_count = sv.get_type().count_fields();
                    if field_count == 0 {
                        i64_ty.const_zero().into()
                    } else {
                        // Spill struct to stack, store pointer as i64.
                        // Mark as struct register so ListPush can heap-copy.
                        self.reg_types.set(reg, RegisterType::Struct { type_name: "unknown".to_string(), size: field_count });
                        self.struct_register_sizes.insert(reg, field_count);
                        let struct_ty: BasicTypeEnum = sv.get_type().into();
                        let struct_alloca = self.builder
                            .build_alloca(struct_ty, &format!("r{}_struct_spill", reg))
                            .expect("struct alloca should not fail");
                        let _ = self.builder.build_store(struct_alloca, sv);
                        self.builder
                            .build_ptr_to_int(struct_alloca, i64_ty, &format!("r{}_s2i", reg))
                            .expect("ptrtoint should not fail")
                            .into()
                    }
                }
                // Array, Vector — store zero as fallback (these should be rare in
                // alloca mode; concrete lowerings handle them via stack slots)
                _ => i64_ty.const_zero().into(),
            };

            if let Some(&existing_ptr) = self.alloca_registers.get(&reg) {
                let _ = self.builder.build_store(existing_ptr, i64_value);
                // CRITICAL: Update the register type to i64. If this alloca was
                // previously used for a pointer (alloca ptr), get_register must now
                // load as i64, not ptr. Without this, an integer value (e.g., loop
                // accumulator) gets loaded as ptr and passed to puts() → SIGSEGV.
                self.alloca_register_types.insert(reg, i64_ty.into());
            } else {
                // First use of this register — create i64 alloca in entry block
                let entry = self.function.get_first_basic_block()
                    .expect("function must have entry block");
                let saved_block = self.builder.get_insert_block();

                if let Some(first_instr) = entry.get_first_instruction() {
                    self.builder.position_before(&first_instr);
                } else {
                    self.builder.position_at_end(entry);
                }

                let alloca = self.builder
                    .build_alloca(BasicTypeEnum::IntType(i64_ty), &format!("r{}_slot", reg))
                    .expect("alloca should not fail");

                if let Some(block) = saved_block {
                    self.builder.position_at_end(block);
                }

                self.alloca_registers.insert(reg, alloca);
                self.alloca_register_types.insert(reg, i64_ty.into());
                let _ = self.builder.build_store(alloca, i64_value);
            }
        } else {
            self.registers.insert(reg, value);
        }
    }

    /// Allocate a stack slot for a mutable register.
    pub fn alloc_register_slot(
        &mut self,
        reg: u16,
        ty: verum_llvm::types::BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>> {
        let slot = self
            .builder
            .build_alloca(ty, name)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        self.register_slots.insert(reg, slot);
        Ok(slot)
    }

    /// Get a register's stack slot.
    pub fn get_register_slot(&self, reg: u16) -> Option<PointerValue<'ctx>> {
        self.register_slots.get(&reg).copied()
    }

    /// Get the alloca pointer for a register in alloca mode.
    /// Returns the alloca address where the register's value is stored.
    pub fn get_alloca_ptr(&self, reg: u16) -> Option<PointerValue<'ctx>> {
        self.alloca_registers.get(&reg).copied()
    }

    /// Load a value from a register's stack slot.
    pub fn load_register(
        &self,
        reg: u16,
        ty: verum_llvm::types::BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>> {
        let slot = self
            .register_slots
            .get(&reg)
            .ok_or_else(|| LlvmLoweringError::InvalidRegister(reg))?;

        self.builder
            .build_load(ty, *slot, &format!("r{}", reg))
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
    }

    /// Store a value to a register's stack slot.
    pub fn store_register(&self, reg: u16, value: BasicValueEnum<'ctx>) -> Result<()> {
        let slot = self
            .register_slots
            .get(&reg)
            .ok_or_else(|| LlvmLoweringError::InvalidRegister(reg))?;

        self.builder
            .build_store(*slot, value)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        Ok(())
    }

    // ========================================================================
    // Basic Block Management
    // ========================================================================

    /// Create a basic block.
    pub fn create_block(&mut self, index: u32, name: &str) -> BasicBlock<'ctx> {
        let context = self.types.context();
        let block = context.append_basic_block(self.function, name);
        self.blocks.insert(index, block);
        block
    }

    /// Get a basic block by index.
    pub fn get_block(&self, index: u32) -> Result<BasicBlock<'ctx>> {
        self.blocks
            .get(&index)
            .copied()
            .ok_or_else(|| LlvmLoweringError::MissingBlock(format!("block_{}", index).into()))
    }

    /// Position the builder at the end of a block.
    pub fn position_at_end(&self, block: BasicBlock<'ctx>) {
        self.builder.position_at_end(block);
    }

    /// Get the entry block of the function.
    pub fn entry_block(&self) -> Option<BasicBlock<'ctx>> {
        self.function.get_first_basic_block()
    }

    /// Check if the current block has a terminator.
    pub fn current_block_has_terminator(&self) -> bool {
        self.builder
            .get_insert_block()
            .map(|b| b.get_terminator().is_some())
            .unwrap_or(false)
    }

    // ========================================================================
    // Exception Handling
    // ========================================================================

    /// Push an exception handler onto the stack.
    ///
    /// Called by TryBegin to set up structured exception handling.
    pub fn push_exception_handler(&mut self, handler: ExceptionHandler<'ctx>) {
        self.exception_handlers.push(handler);
    }

    /// Pop an exception handler from the stack.
    ///
    /// Called by TryEnd when leaving a try block normally.
    pub fn pop_exception_handler(&mut self) -> Option<ExceptionHandler<'ctx>> {
        self.exception_handlers.pop()
    }

    /// Get the current exception handler (if any).
    pub fn current_exception_handler(&self) -> Option<&ExceptionHandler<'ctx>> {
        self.exception_handlers.last()
    }

    /// Check if there's an active exception handler.
    pub fn has_exception_handler(&self) -> bool {
        !self.exception_handlers.is_empty()
    }

    /// Get or create the exception value slot.
    ///
    /// This allocates a stack slot to store the current exception value.
    /// The slot is reused across all exception handlers in the function.
    pub fn get_or_create_exception_slot(&mut self) -> Result<PointerValue<'ctx>> {
        if let Some(slot) = self.exception_value_slot {
            return Ok(slot);
        }

        // Allocate in entry block for proper dominance
        let entry = self.entry_block()
            .ok_or_else(|| LlvmLoweringError::internal("No entry block for exception slot"))?;

        // Save current position
        let current_block = self.builder.get_insert_block();

        // Position at end of entry block (before terminator if any)
        if let Some(terminator) = entry.get_terminator() {
            self.builder.position_before(&terminator);
        } else {
            self.builder.position_at_end(entry);
        }

        // Allocate i64 slot for exception value (alloca mode uses i64 for all values)
        let i64_type = self.types.i64_type();
        let slot = self.builder
            .build_alloca(i64_type, "exception_value")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Initialize to zero
        let zero = i64_type.const_zero();
        self.builder
            .build_store(slot, zero)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;

        // Restore position
        if let Some(block) = current_block {
            self.builder.position_at_end(block);
        }

        self.exception_value_slot = Some(slot);
        Ok(slot)
    }

    /// Store the exception value.
    pub fn store_exception_value(&mut self, value: BasicValueEnum<'ctx>) -> Result<()> {
        let slot = self.get_or_create_exception_slot()?;
        // Coerce value to i64 if needed (exception slot is always i64)
        let i64_type = self.types.i64_type();
        let store_val = if value.is_int_value() {
            let iv = value.into_int_value();
            if iv.get_type().get_bit_width() != 64 {
                self.builder
                    .build_int_z_extend(iv, i64_type, "exc_ext")
                    .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
                    .into()
            } else {
                value
            }
        } else if value.is_pointer_value() {
            self.builder
                .build_ptr_to_int(value.into_pointer_value(), i64_type, "exc_ptr2int")
                .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?
                .into()
        } else {
            value
        };
        self.builder
            .build_store(slot, store_val)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        Ok(())
    }

    /// Load the current exception value.
    pub fn load_exception_value(&mut self) -> Result<BasicValueEnum<'ctx>> {
        let slot = self.get_or_create_exception_slot()?;
        let i64_type = self.types.i64_type();
        self.builder
            .build_load(i64_type, slot, "exception_load")
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))
    }

    /// Clear the current exception value (set to zero).
    pub fn clear_exception_value(&mut self) -> Result<()> {
        let slot = self.get_or_create_exception_slot()?;
        let zero = self.types.i64_type().const_zero();
        self.builder
            .build_store(slot, zero)
            .map_err(|e| LlvmLoweringError::llvm_error(e.to_string()))?;
        Ok(())
    }
}

/// Statistics for function lowering.
#[derive(Debug, Default, Clone)]
pub struct FunctionStats {
    /// Number of instructions lowered.
    pub instructions: usize,
    /// Number of registers used.
    pub registers_used: usize,
    /// Number of basic blocks.
    pub basic_blocks: usize,
    /// Number of CBGR operations.
    pub cbgr_ops: usize,
    /// CBGR tier distribution.
    pub tier_distribution: TierDistribution,
}

/// CBGR tier distribution.
#[derive(Debug, Default, Clone)]
pub struct TierDistribution {
    /// Tier 0 references.
    pub tier0: usize,
    /// Tier 1 references.
    pub tier1: usize,
    /// Tier 2 references.
    pub tier2: usize,
}

impl TierDistribution {
    /// Get the total number of references.
    pub fn total(&self) -> usize {
        self.tier0 + self.tier1 + self.tier2
    }

    /// Calculate the elimination rate (Tier 1 + Tier 2 / Total).
    pub fn elimination_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            (self.tier1 + self.tier2) as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_info_local() {
        let info = ReferenceInfo::local();
        assert!(info.can_use_tier1());
        assert!(info.is_no_escape);
        assert!(!info.escapes_to_heap);
        assert!(!info.returned);
        assert_eq!(info.source, ReferenceSource::LocalAlloca);
    }

    #[test]
    fn test_reference_info_parameter() {
        let info = ReferenceInfo::parameter();
        assert!(!info.can_use_tier1());
        assert!(!info.is_no_escape);
        assert_eq!(info.source, ReferenceSource::Parameter);
    }

    #[test]
    fn test_reference_info_unknown() {
        let info = ReferenceInfo::unknown();
        assert!(!info.can_use_tier1());
        assert!(!info.is_no_escape);
        assert_eq!(info.source, ReferenceSource::Unknown);
    }

    #[test]
    fn test_reference_info_heap_escape() {
        let mut info = ReferenceInfo::local();
        assert!(info.can_use_tier1());

        info.mark_heap_escape();
        assert!(!info.can_use_tier1());
        assert!(info.escapes_to_heap);
        assert!(!info.is_no_escape);
    }

    #[test]
    fn test_reference_info_returned() {
        let mut info = ReferenceInfo::local();
        assert!(info.can_use_tier1());

        info.mark_returned();
        assert!(!info.can_use_tier1());
        assert!(info.returned);
        assert!(!info.is_no_escape);
    }

    #[test]
    fn test_cbgr_elimination_stats() {
        let mut stats = CbgrEliminationStats::default();
        assert_eq!(stats.total_ref_ops, 0);
        assert_eq!(stats.elimination_rate(), 0.0);

        // Record safe reference
        stats.record_ref_op(true);
        assert_eq!(stats.total_ref_ops, 1);
        assert_eq!(stats.proven_safe, 1);
        assert_eq!(stats.runtime_checks, 0);
        assert_eq!(stats.estimated_savings_ns, 15);
        assert_eq!(stats.elimination_rate(), 1.0);

        // Record unsafe reference
        stats.record_ref_op(false);
        assert_eq!(stats.total_ref_ops, 2);
        assert_eq!(stats.proven_safe, 1);
        assert_eq!(stats.runtime_checks, 1);
        assert_eq!(stats.elimination_rate(), 0.5);
    }

    #[test]
    fn test_tier_distribution() {
        let mut dist = TierDistribution::default();
        assert_eq!(dist.total(), 0);
        assert_eq!(dist.elimination_rate(), 0.0);

        dist.tier0 = 5;
        dist.tier1 = 3;
        dist.tier2 = 2;
        assert_eq!(dist.total(), 10);
        assert_eq!(dist.elimination_rate(), 0.5); // (3+2) / 10 = 0.5
    }
}
