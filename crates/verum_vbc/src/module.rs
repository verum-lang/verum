//! VBC module and function structures.
//!
//! This module defines the high-level structures for VBC modules:
//! - [`VbcModule`]: Complete compiled module
//! - [`VbcFunction`]: Individual function with bytecode
//! - [`FunctionDescriptor`]: Function metadata
//! - [`Constant`]: Constant pool entries
//! - [`SpecializationEntry`]: Pre-computed specializations

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::format::{VbcFlags, VbcHeader};
use crate::instruction::Instruction;
use crate::metadata::{AutodiffGraph, DeviceHints, DistributionMetadata, MlirHints, ShapeMetadata};
use crate::types::{
    CbgrTier, ContextRef, Mutability, PropertySet, ProtocolId, StringId, TypeDescriptor, TypeId,
    TypeParamDescriptor, TypeRef, Visibility,
};

// ============================================================================
// Identifiers
// ============================================================================

/// Function identifier - index into function table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FunctionId(pub u32);

/// Constant identifier - index into constant pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ConstId(pub u32);

// ============================================================================
// VBC Module
// ============================================================================

/// Complete VBC module.
///
/// A VbcModule contains all compiled code and metadata for a Verum module,
/// ready for interpretation, JIT compilation, or AOT compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VbcModule {
    /// Module header.
    pub header: VbcHeader,

    /// Module name.
    pub name: String,

    /// String table (deduplicated strings).
    pub strings: StringTable,

    /// Type table.
    pub types: Vec<TypeDescriptor>,

    /// Function table.
    pub functions: Vec<FunctionDescriptor>,

    /// Constant pool.
    pub constants: Vec<Constant>,

    /// Raw bytecode (all functions concatenated).
    pub bytecode: Vec<u8>,

    /// Pre-computed specializations.
    pub specializations: Vec<SpecializationEntry>,

    /// Source map for debugging (optional).
    pub source_map: Option<SourceMap>,

    /// Module dependencies.
    pub dependencies: Vec<ModuleDependency>,

    // ========================================================================
    // FFI Support
    // ========================================================================
    /// FFI libraries (native libraries to load).
    pub ffi_libraries: Vec<FfiLibrary>,

    /// FFI symbols (functions/variables to resolve).
    pub ffi_symbols: Vec<FfiSymbol>,

    /// FFI struct layouts (for marshalling).
    pub ffi_layouts: Vec<FfiStructLayout>,

    /// Source directory for resolving relative paths (FFI libraries, etc.).
    /// This is the directory containing the main source file.
    #[serde(default)]
    pub source_dir: Option<String>,

    /// Index into `functions` where user-defined functions start.
    /// Functions before this index are from the stdlib. Used by the @test runner
    /// to only execute user-defined test functions, not stdlib tests.
    #[serde(default)]
    pub user_function_start: u32,

    // ========================================================================
    // Tensor Metadata: compile-time shape verification and GPU kernel dispatch
    // ========================================================================
    /// Shape annotations for compile-time tensor verification.
    /// Maps instruction IDs to static/symbolic shapes for shape checking.
    #[serde(default)]
    pub shape_metadata: ShapeMetadata,

    /// Device placement hints for CPU/GPU/TPU execution.
    /// Guides the runtime/compiler in device selection.
    #[serde(default)]
    pub device_hints: DeviceHints,

    /// Distribution topology for distributed training.
    /// Mesh topology, sharding specs, and collective operations.
    #[serde(default)]
    pub distribution: DistributionMetadata,

    /// Autodiff graph for gradient computation.
    /// Forward→backward mapping, checkpoints, tape structure.
    #[serde(default)]
    pub autodiff_graph: AutodiffGraph,

    /// MLIR lowering hints for optimization.
    /// Fusion groups, target-specific optimizations.
    #[serde(default)]
    pub mlir_hints: MlirHints,

    // ========================================================================
    // Global Constructors/Destructors
    // ========================================================================
    /// Global constructor entries: (function_id, priority).
    /// These functions run before main() in priority order (lower = first).
    /// Used for static variable initialization.
    #[serde(default)]
    pub global_ctors: Vec<(FunctionId, u32)>,

    /// Global destructor entries: (function_id, priority).
    /// These functions run after main() returns, in priority order.
    #[serde(default)]
    pub global_dtors: Vec<(FunctionId, u32)>,

    // ========================================================================
    // Context System
    // ========================================================================
    /// Context name table: maps ContextRef(id) → StringId for name resolution.
    /// Enables core_loader to recover context names from opaque ContextRef IDs.
    #[serde(default)]
    pub context_names: Vec<StringId>,

    // ========================================================================
    // Field Layout Metadata (for correct GetF/SetF field index resolution)
    // ========================================================================
    /// Maps global interned field ID → field name.
    /// Allows LLVM lowering to reverse-lookup field names from VBC GetF/SetF instructions.
    #[serde(default)]
    pub field_id_to_name: Vec<String>,

    /// Maps type name → ordered list of field names.
    /// Used by LLVM lowering to remap global field IDs to positional indices.
    #[serde(default)]
    pub type_field_layouts: std::collections::HashMap<String, Vec<String>>,
}

impl Default for VbcModule {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl VbcModule {
    /// Creates a new empty module.
    ///
    /// The module name is automatically interned in the string table.
    pub fn new(name: String) -> Self {
        let mut strings = StringTable::new();
        // Intern the module name as the first string (ID 0)
        strings.intern(&name);

        Self {
            header: VbcHeader::new(),
            name,
            strings,
            types: Vec::new(),
            functions: Vec::new(),
            constants: Vec::new(),
            bytecode: Vec::new(),
            specializations: Vec::new(),
            source_map: None,
            dependencies: Vec::new(),
            ffi_libraries: Vec::new(),
            ffi_symbols: Vec::new(),
            ffi_layouts: Vec::new(),
            source_dir: None,
            // Tensor metadata (defaults)
            shape_metadata: ShapeMetadata::default(),
            device_hints: DeviceHints::default(),
            distribution: DistributionMetadata::default(),
            autodiff_graph: AutodiffGraph::default(),
            mlir_hints: MlirHints::default(),
            // Global constructors/destructors
            global_ctors: Vec::new(),
            global_dtors: Vec::new(),
            // Context name table
            context_names: Vec::new(),
            // Field layout metadata
            field_id_to_name: Vec::new(),
            type_field_layouts: std::collections::HashMap::new(),
            // User function start (defaults to 0 = all functions are user code)
            user_function_start: 0,
        }
    }

    /// Adds a string to the string table, returning its ID.
    pub fn intern_string(&mut self, s: &str) -> StringId {
        self.strings.intern(s)
    }

    /// Gets a string by ID.
    pub fn get_string(&self, id: StringId) -> Option<&str> {
        self.strings.get(id)
    }

    /// Adds a type descriptor.
    pub fn add_type(&mut self, desc: TypeDescriptor) -> TypeId {
        let id = TypeId(self.types.len() as u32 + TypeId::FIRST_USER);
        self.types.push(desc);
        id
    }

    /// Gets a type descriptor by ID.
    pub fn get_type(&self, id: TypeId) -> Option<&TypeDescriptor> {
        // Search by TypeId. Note: do NOT filter by is_builtin() because VBC TypeIds
        // are assigned non-deterministically (HashMap iteration order in the compiler),
        // so user-defined types like MapKeys can get assigned builtin-range IDs (< 16).
        self.types.iter().find(|desc| desc.id == id)
    }

    /// Gets the name of a type by its TypeId.
    /// Returns None for builtin types or if the type is not found.
    pub fn get_type_name(&self, id: TypeId) -> Option<String> {
        if let Some(desc) = self.get_type(id) {
            self.get_string(desc.name).map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Renders a `TypeRef` as a human-readable string.
    ///
    /// Handles all Verum type constructs including generics, references,
    /// function types, tuples, and CBGR tiers.
    ///
    /// # Examples
    /// - `TypeRef::Concrete(TypeId::INT)` → `"Int"`
    /// - `Instantiated { base: List, args: [INT] }` → `"List<Int>"`
    /// - `Function { params: [INT, TEXT], return_type: BOOL }` → `"fn(Int, Text) -> Bool"`
    /// - `Reference { inner: TEXT, mutability: Mutable, tier: Tier1 }` → `"&mut checked Text"`
    pub fn display_type_ref(&self, tr: &TypeRef) -> String {
        match tr {
            TypeRef::Concrete(tid) => self.display_type_id(*tid),
            TypeRef::Generic(p) => format!("T{}", p.0),
            TypeRef::Instantiated { base, args } => {
                let base_name = self.display_type_id(*base);
                let args_str: Vec<String> = args.iter().map(|a| self.display_type_ref(a)).collect();
                format!("{}<{}>", base_name, args_str.join(", "))
            }
            TypeRef::Function { params, return_type, .. } => {
                let params_str: Vec<String> = params.iter().map(|p| self.display_type_ref(p)).collect();
                format!("fn({}) -> {}", params_str.join(", "), self.display_type_ref(return_type))
            }
            TypeRef::Rank2Function { type_param_count, params, return_type, .. } => {
                let params_str: Vec<String> = params.iter().map(|p| self.display_type_ref(p)).collect();
                format!("fn<{}>({}) -> {}", type_param_count, params_str.join(", "), self.display_type_ref(return_type))
            }
            TypeRef::Reference { inner, mutability, tier } => {
                let m = match mutability { Mutability::Immutable => "&", Mutability::Mutable => "&mut " };
                let t = match tier { CbgrTier::Tier0 => "", CbgrTier::Tier1 => "checked ", CbgrTier::Tier2 => "unsafe " };
                format!("{}{}{}", m, t, self.display_type_ref(inner))
            }
            TypeRef::Tuple(elems) => {
                let s: Vec<String> = elems.iter().map(|e| self.display_type_ref(e)).collect();
                format!("({})", s.join(", "))
            }
            TypeRef::Array { element, length } => {
                format!("[{}; {}]", self.display_type_ref(element), length)
            }
            TypeRef::Slice(inner) => {
                format!("[{}]", self.display_type_ref(inner))
            }
        }
    }

    /// Renders a `TypeId` as a human-readable string.
    pub fn display_type_id(&self, tid: TypeId) -> String {
        match tid {
            TypeId::UNIT => "()".into(),
            TypeId::BOOL => "Bool".into(),
            TypeId::INT => "Int".into(),
            TypeId::FLOAT => "Float".into(),
            TypeId::TEXT => "Text".into(),
            TypeId::NEVER => "Never".into(),
            TypeId::U8 => "U8".into(),
            TypeId::U16 => "U16".into(),
            TypeId::U32 => "U32".into(),
            TypeId::U64 => "U64".into(),
            TypeId::I8 => "I8".into(),
            TypeId::I16 => "I16".into(),
            TypeId::I32 => "I32".into(),
            TypeId::F32 => "F32".into(),
            TypeId::PTR => "Ptr".into(),
            TypeId::LIST => "List".into(),
            TypeId::MAP => "Map".into(),
            TypeId::SET => "Set".into(),
            TypeId::MAYBE => "Maybe".into(),
            TypeId::RESULT => "Result".into(),
            TypeId::DEQUE => "Deque".into(),
            TypeId::CHANNEL => "Channel".into(),
            _ => self.get_type_name(tid).unwrap_or_else(|| format!("type#{}", tid.0)),
        }
    }

    /// Gets the field count of a type by its TypeId.
    pub fn get_type_field_count(&self, id: TypeId) -> Option<u32> {
        self.get_type(id).map(|desc| desc.fields.len() as u32)
    }

    /// Adds a function descriptor.
    pub fn add_function(&mut self, desc: FunctionDescriptor) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        self.functions.push(desc);
        id
    }

    /// Gets a function descriptor by ID.
    pub fn get_function(&self, id: FunctionId) -> Option<&FunctionDescriptor> {
        self.functions.get(id.0 as usize)
    }

    /// Gets a mutable function descriptor by ID.
    pub fn get_function_mut(&mut self, id: FunctionId) -> Option<&mut FunctionDescriptor> {
        self.functions.get_mut(id.0 as usize)
    }

    /// Adds a constant to the pool.
    pub fn add_constant(&mut self, constant: Constant) -> ConstId {
        let id = ConstId(self.constants.len() as u32);
        self.constants.push(constant);
        id
    }

    /// Gets a constant by ID.
    pub fn get_constant(&self, id: ConstId) -> Option<&Constant> {
        self.constants.get(id.0 as usize)
    }

    /// Appends bytecode and returns the offset.
    pub fn append_bytecode(&mut self, code: &[u8]) -> u32 {
        let offset = self.bytecode.len() as u32;
        self.bytecode.extend_from_slice(code);
        offset
    }

    /// Sets profile-related flags based on the compilation profile.
    ///
    /// This method sets the following flags based on the profile:
    /// - `NOT_INTERPRETABLE`: Systems profile modules cannot be interpreted
    /// - `SYSTEMS_PROFILE`: Marks modules compiled with Systems profile
    /// - `EMBEDDED_TARGET`: Marks modules for embedded/bare-metal targets
    ///
    /// # Arguments
    ///
    /// * `is_interpretable` - Whether the module can be executed by VBC interpreter
    /// * `is_systems_profile` - Whether this is a Systems profile build (low-level code)
    /// * `is_embedded` - Whether this targets embedded/bare-metal
    ///
    /// V-LLSI (Verum Low-Level System Interface): Sets execution profile flags that control
    /// bytecode layout and feature availability. Systems profile enables raw pointers, inline
    /// assembly, and interrupt handlers. Embedded profile disables heap allocation and OS APIs.
    /// Interpretable flag ensures bytecode can run in Tier 0 interpreter (no AOT-only features).
    pub fn set_profile_flags(
        &mut self,
        is_interpretable: bool,
        is_systems_profile: bool,
        is_embedded: bool,
    ) {
        // Systems profile: NOT interpretable, VBC is intermediate IR only
        if !is_interpretable {
            self.header.flags |= VbcFlags::NOT_INTERPRETABLE;
        }

        // Mark systems profile modules
        if is_systems_profile {
            self.header.flags |= VbcFlags::SYSTEMS_PROFILE;
        }

        // Mark embedded targets
        if is_embedded {
            self.header.flags |= VbcFlags::EMBEDDED_TARGET;
            // Embedded targets are also not interpretable
            self.header.flags |= VbcFlags::NOT_INTERPRETABLE;
        }
    }

    /// Updates module flags based on content.
    pub fn update_flags(&mut self) {
        // Preserve profile flags that were set via set_profile_flags
        let profile_flags = self.header.flags & (
            VbcFlags::NOT_INTERPRETABLE |
            VbcFlags::SYSTEMS_PROFILE |
            VbcFlags::EMBEDDED_TARGET
        );

        let mut flags = profile_flags;

        // Check if any function is generic
        if self.functions.iter().any(|f| f.is_generic) {
            flags |= VbcFlags::HAS_GENERICS;
        }

        // Check if there are precompiled specializations
        if !self.specializations.is_empty() {
            flags |= VbcFlags::HAS_PRECOMPILED_SPECS;
        }

        // Check for async functions
        if self
            .functions
            .iter()
            .any(|f| f.properties.contains(PropertySet::ASYNC))
        {
            flags |= VbcFlags::HAS_ASYNC;
        }

        // Check for context usage
        if self.functions.iter().any(|f| !f.contexts.is_empty()) {
            flags |= VbcFlags::HAS_CONTEXTS;
        }

        // Check for tensor ops
        if self
            .functions
            .iter()
            .any(|f| f.properties.contains(PropertySet::GPU))
        {
            flags |= VbcFlags::HAS_GPU;
        }

        // Check if debug info present
        if self.source_map.is_some() {
            flags |= VbcFlags::DEBUG_INFO;
        }

        // Check for FFI usage
        if !self.ffi_symbols.is_empty() {
            flags |= VbcFlags::HAS_FFI;
        }

        self.header.flags = flags;
    }

    // ========================================================================
    // FFI Methods
    // ========================================================================

    /// Adds an FFI library.
    pub fn add_ffi_library(&mut self, library: FfiLibrary) -> FfiLibraryId {
        let id = FfiLibraryId(self.ffi_libraries.len() as u16);
        self.ffi_libraries.push(library);
        id
    }

    /// Gets an FFI library by ID.
    pub fn get_ffi_library(&self, id: FfiLibraryId) -> Option<&FfiLibrary> {
        self.ffi_libraries.get(id.0 as usize)
    }

    /// Adds an FFI symbol.
    pub fn add_ffi_symbol(&mut self, symbol: FfiSymbol) -> FfiSymbolId {
        let id = FfiSymbolId(self.ffi_symbols.len() as u32);
        self.ffi_symbols.push(symbol);
        id
    }

    /// Gets an FFI symbol by ID.
    pub fn get_ffi_symbol(&self, id: FfiSymbolId) -> Option<&FfiSymbol> {
        self.ffi_symbols.get(id.0 as usize)
    }

    /// Looks up an FFI symbol by name.
    pub fn find_ffi_symbol(&self, name: &str) -> Option<FfiSymbolId> {
        for (idx, sym) in self.ffi_symbols.iter().enumerate() {
            if let Some(sym_name) = self.strings.get(sym.name)
                && sym_name == name {
                    return Some(FfiSymbolId(idx as u32));
                }
        }
        None
    }

    /// Adds an FFI struct layout.
    pub fn add_ffi_layout(&mut self, layout: FfiStructLayout) -> u32 {
        let id = self.ffi_layouts.len() as u32;
        self.ffi_layouts.push(layout);
        id
    }

    /// Gets an FFI struct layout by index.
    pub fn get_ffi_layout(&self, idx: u32) -> Option<&FfiStructLayout> {
        self.ffi_layouts.get(idx as usize)
    }

    /// Returns true if this module uses FFI.
    pub fn has_ffi(&self) -> bool {
        !self.ffi_symbols.is_empty()
    }
}

// ============================================================================
// String Table
// ============================================================================

/// Deduplicated string table.
///
/// Strings are stored once and referenced by [`StringId`].
/// The ID is the byte offset in the serialized form.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StringTable {
    /// Map from string to ID for deduplication.
    index: IndexMap<String, StringId>,
    /// Next available offset.
    next_offset: u32,
}

impl StringTable {
    /// Creates a new empty string table.
    pub fn new() -> Self {
        Self {
            index: IndexMap::new(),
            next_offset: 0,
        }
    }

    /// Interns a string, returning its ID.
    ///
    /// If the string already exists, returns the existing ID.
    pub fn intern(&mut self, s: &str) -> StringId {
        if let Some(&id) = self.index.get(s) {
            return id;
        }

        let id = StringId(self.next_offset);
        // Offset includes 4-byte length prefix + string bytes
        self.next_offset += 4 + s.len() as u32;
        self.index.insert(s.to_string(), id);
        id
    }

    /// Gets a string by ID.
    pub fn get(&self, id: StringId) -> Option<&str> {
        // Find the string with matching ID
        self.index
            .iter()
            .find(|&(_, sid)| *sid == id)
            .map(|(s, _)| s.as_str())
    }

    /// Returns an iterator over all strings in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, StringId)> {
        self.index.iter().map(|(s, &id)| (s.as_str(), id))
    }

    /// Returns the number of strings.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Returns the total serialized size.
    pub fn serialized_size(&self) -> u32 {
        self.next_offset
    }
}

// ============================================================================
// Optimization Hints
// ============================================================================

/// Inline mode for function inlining control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InlineHint {
    /// @inline - suggest inlining (LLVM `inlinehint`)
    Suggest,
    /// @inline(always) - always inline (LLVM `alwaysinline`)
    Always,
    /// @inline(never) - never inline (LLVM `noinline`)
    Never,
    /// @inline(release) - inline only in release builds
    Release,
}

/// Per-function optimization level override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OptLevel {
    /// @optimize(none) - no optimization (LLVM `optnone`)
    None,
    /// @optimize(size) - optimize for size (LLVM `optsize`)
    Size,
    /// @optimize(speed) - optimize for speed
    Speed,
    /// @optimize(balanced) - balance size and speed (default)
    Balanced,
}

/// Loop unroll hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopUnrollHint {
    /// @unroll(N)
    Count(u32),
    /// @unroll(full)
    Full,
    /// @no_unroll
    Disable,
}

/// Vectorization hint for loops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VectorizeHint {
    /// @vectorize or @simd - enable
    Enable,
    /// @vectorize(force) or @simd(force)
    Force,
    /// @vectorize(width: N)
    Width(u32),
    /// @no_vectorize or @simd(never)
    Disable,
}

/// Combined loop optimization hints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopHints {
    /// Loop unroll hint.
    pub unroll: Option<LoopUnrollHint>,
    /// Vectorization hint.
    pub vectorize: Option<VectorizeHint>,
}

/// Function-level optimization hints extracted from AST attributes.
///
/// These flow through the pipeline: AST @attributes -> VBC OptimizationHints -> LLVM attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationHints {
    /// Function is pure (no side effects, deterministic).
    pub is_pure: bool,
    /// Function never panics or diverges.
    pub is_total: bool,
    /// Prefer inlining even if heuristics say no (legacy field, prefer inline_hint).
    pub force_inline: bool,
    /// @inline / @inline(always) / @inline(never) / @inline(release)
    pub inline_hint: Option<InlineHint>,
    /// @cold - rarely executed (LLVM: `cold` attr, `.text.cold` section)
    pub is_cold: bool,
    /// @hot - frequently executed (LLVM: `hot` attr)
    pub is_hot: bool,
    /// @optimize(none|size|speed|balanced)
    pub opt_level: Option<OptLevel>,
    /// @align(N) - function alignment in bytes
    pub alignment: Option<u32>,
    /// @target_feature("+avx2,+fma")
    pub target_features: Option<String>,
    /// @target_cpu("native"|"x86-64-v3")
    pub target_cpu: Option<String>,
}

// ============================================================================
// Function Descriptor
// ============================================================================

/// Function descriptor in the function table.
///
/// Contains all metadata about a function including signature,
/// bytecode location, and optimization hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDescriptor {
    /// Unique function ID.
    pub id: FunctionId,

    /// Function name (qualified: "List.push", "main").
    pub name: StringId,

    /// Containing type (for methods).
    pub parent_type: Option<TypeId>,

    /// Generic type parameters.
    pub type_params: SmallVec<[TypeParamDescriptor; 2]>,

    /// Function parameters.
    pub params: SmallVec<[ParamDescriptor; 4]>,

    /// Return type.
    pub return_type: TypeRef,

    /// Context requirements: `using [Database, Logger]`.
    pub contexts: SmallVec<[ContextRef; 2]>,

    /// Computational properties: `{Async, IO, Fallible}`.
    pub properties: PropertySet,

    /// Bytecode offset within bytecode section.
    pub bytecode_offset: u32,

    /// Bytecode length.
    pub bytecode_length: u32,

    /// Number of local variables.
    pub locals_count: u16,

    /// Number of registers needed.
    pub register_count: u16,

    /// Maximum stack depth (for VBC validation).
    pub max_stack: u16,

    /// Inline candidate flag.
    pub is_inline_candidate: bool,

    /// Is this a generic function?
    pub is_generic: bool,

    /// Visibility.
    pub visibility: Visibility,

    /// Is this a generator function (fn*)? Generator functions use the Yield opcode to
    /// suspend execution and produce values lazily. The interpreter maintains a Generator
    /// struct with saved PC, registers, and context stack per generator instance.
    pub is_generator: bool,

    /// Type of yielded values (for generators).
    /// For regular functions, this is None.
    pub yield_type: Option<TypeRef>,

    /// Number of yield points (suspend points) in the generator.
    /// Used for state machine validation.
    pub suspend_point_count: u16,

    /// Calling convention for this function.
    /// Default is C for regular functions.
    /// Calling convention for low-level code: C (default), Interrupt (auto save/restore
    /// registers, uses iret), Naked (no prologue/epilogue, inline asm only), etc.
    #[serde(default)]
    pub calling_convention: CallingConvention,

    /// Optimization hints for downstream passes.
    #[serde(default)]
    pub optimization_hints: OptimizationHints,

    /// Decoded instructions (populated after deserialization).
    #[serde(skip)]
    pub instructions: Option<Vec<Instruction>>,

    /// Base function ID offset for resolving Call targets within merged stdlib modules.
    /// When a stdlib module is merged into the main module, Call func_ids in its bytecode
    /// are relative to the source module. This offset converts them to the merged module's IDs.
    #[serde(default)]
    pub func_id_base: u32,

    /// Debug variable info for DWARF emission.
    ///
    /// Maps register indices to variable names and scopes for debugger variable
    /// inspection. Populated by VBC codegen when `source_map` is enabled.
    ///
    /// Each entry describes a local variable:
    /// - `name`: Variable name (index into string table)
    /// - `register`: VBC register holding this variable's value
    /// - `scope_start` / `scope_end`: Instruction index range where the variable is live
    /// - `is_parameter`: True if this is a function parameter (not a local)
    /// - `arg_index`: For parameters, the 1-based argument position
    #[serde(default)]
    pub debug_variables: Vec<DebugVariableInfo>,

    /// Is this a test function (annotated with `@test`)?
    /// Used by the test runner to discover and execute test functions.
    #[serde(default)]
    pub is_test: bool,
}

/// Debug information for a local variable or parameter.
///
/// Used to emit DWARF `DW_TAG_variable` / `DW_TAG_formal_parameter` entries
/// in the AOT compilation path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugVariableInfo {
    /// Variable name (index into string table).
    pub name: StringId,
    /// Register index holding this variable.
    pub register: u16,
    /// Instruction index where the variable becomes live.
    pub scope_start: u32,
    /// Instruction index where the variable goes out of scope.
    pub scope_end: u32,
    /// True if this is a function parameter.
    pub is_parameter: bool,
    /// For parameters: 1-based argument index. 0 for locals.
    pub arg_index: u16,
}

impl Default for FunctionDescriptor {
    fn default() -> Self {
        Self {
            id: FunctionId(0),
            name: StringId::EMPTY,
            parent_type: None,
            type_params: SmallVec::new(),
            params: SmallVec::new(),
            return_type: TypeRef::Concrete(TypeId::UNIT),
            contexts: SmallVec::new(),
            properties: PropertySet::empty(),
            bytecode_offset: 0,
            bytecode_length: 0,
            locals_count: 0,
            register_count: 0,
            max_stack: 0,
            is_inline_candidate: false,
            is_generic: false,
            visibility: Visibility::Public,
            is_generator: false,
            yield_type: None,
            suspend_point_count: 0,
            calling_convention: CallingConvention::C,
            optimization_hints: OptimizationHints::default(),
            instructions: None,
            func_id_base: 0,
            debug_variables: Vec::new(),
            is_test: false,
        }
    }
}

impl FunctionDescriptor {
    /// Creates a new function descriptor.
    pub fn new(name: StringId) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    /// Returns true if this is a method (has parent type).
    pub fn is_method(&self) -> bool {
        self.parent_type.is_some()
    }

    /// Returns the arity (number of parameters).
    pub fn arity(&self) -> usize {
        self.params.len()
    }
}

/// Function parameter descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDescriptor {
    /// Parameter name.
    pub name: StringId,
    /// Parameter type.
    pub type_ref: TypeRef,
    /// Is this parameter mutable?
    pub is_mut: bool,
    /// Default value (constant pool index, if any).
    pub default: Option<ConstId>,
}

impl Default for ParamDescriptor {
    fn default() -> Self {
        Self {
            name: StringId::EMPTY,
            type_ref: TypeRef::Concrete(TypeId::UNIT),
            is_mut: false,
            default: None,
        }
    }
}

// ============================================================================
// VBC Function (High-level)
// ============================================================================

/// High-level function representation with decoded instructions.
///
/// Used during codegen and interpretation. For serialization,
/// use [`FunctionDescriptor`] with raw bytecode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VbcFunction {
    /// Function descriptor.
    pub descriptor: FunctionDescriptor,

    /// Decoded instructions.
    pub instructions: Vec<Instruction>,

    /// Basic block boundaries (instruction indices).
    pub block_starts: Vec<u32>,
}

impl VbcFunction {
    /// Creates a new function from descriptor and instructions.
    pub fn new(descriptor: FunctionDescriptor, instructions: Vec<Instruction>) -> Self {
        let block_starts = Self::compute_block_starts(&instructions);
        Self {
            descriptor,
            instructions,
            block_starts,
        }
    }

    /// Computes basic block boundaries.
    fn compute_block_starts(instructions: &[Instruction]) -> Vec<u32> {
        let mut starts = vec![0];
        for (i, instr) in instructions.iter().enumerate() {
            match instr {
                // After branches/jumps, next instruction is block start
                Instruction::Jmp { .. }
                | Instruction::JmpIf { .. }
                | Instruction::JmpNot { .. }
                | Instruction::JmpCmp { .. }
                | Instruction::Ret { .. }
                | Instruction::RetV
                | Instruction::Switch { .. }
                    if i + 1 < instructions.len() => {
                        starts.push((i + 1) as u32);
                    }
                _ => {}
            }
        }
        starts.sort_unstable();
        starts.dedup();
        starts
    }
}

// ============================================================================
// Constant Pool
// ============================================================================

/// Constant pool entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    /// Integer constant.
    Int(i64),
    /// Float constant.
    Float(f64),
    /// String constant (index into string table).
    String(StringId),
    /// Type reference.
    Type(TypeRef),
    /// Function reference.
    Function(FunctionId),
    /// Protocol reference.
    Protocol(ProtocolId),
    /// Array of constants (for array literals).
    Array(Vec<ConstId>),
    /// Bytes literal.
    Bytes(Vec<u8>),
}

impl Constant {
    /// Returns the constant tag for serialization.
    pub fn tag(&self) -> u8 {
        match self {
            Constant::Int(_) => 0x01,
            Constant::Float(_) => 0x02,
            Constant::String(_) => 0x03,
            Constant::Type(_) => 0x04,
            Constant::Function(_) => 0x05,
            Constant::Protocol(_) => 0x06,
            Constant::Array(_) => 0x07,
            Constant::Bytes(_) => 0x08,
        }
    }
}

// ============================================================================
// Specialization Table
// ============================================================================

/// Pre-computed specialization entry.
///
/// Maps a generic function with specific type arguments to
/// specialized bytecode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecializationEntry {
    /// Generic function being specialized.
    pub generic_fn: FunctionId,
    /// Type arguments.
    pub type_args: Vec<TypeRef>,
    /// Hash of (generic_fn, type_args) for quick lookup.
    pub hash: u64,
    /// Offset to specialized bytecode.
    pub bytecode_offset: u32,
    /// Length of specialized bytecode.
    pub bytecode_length: u32,
    /// Specialized register count.
    pub register_count: u16,
}

// ============================================================================
// Source Map
// ============================================================================

/// Source map for debugging.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceMap {
    /// File names (indices into string table).
    pub files: Vec<StringId>,
    /// Mapping entries.
    pub entries: Vec<SourceMapEntry>,
}

/// Source map entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMapEntry {
    /// Bytecode offset.
    pub bytecode_offset: u32,
    /// File index.
    pub file_idx: u16,
    /// Source line (1-based).
    pub line: u32,
    /// Source column (1-based).
    pub column: u16,
}

// ============================================================================
// Module Dependencies
// ============================================================================

/// Module dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDependency {
    /// Module name.
    pub name: StringId,
    /// Content hash for cache invalidation.
    pub hash: u64,
}

// ============================================================================
// FFI Tables
// ============================================================================

/// FFI library identifier - index into ffi_libraries table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FfiLibraryId(pub u16);

/// FFI symbol identifier - index into ffi_symbols table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FfiSymbolId(pub u32);

/// Platform identifier for FFI libraries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum FfiPlatform {
    /// Any platform (cross-platform library).
    #[default]
    Any = 0,
    /// macOS / Darwin.
    Darwin = 1,
    /// Linux.
    Linux = 2,
    /// Windows.
    Windows = 3,
    /// FreeBSD.
    FreeBSD = 4,
    /// iOS.
    Ios = 5,
    /// Android.
    Android = 6,
}


impl FfiPlatform {
    /// Returns true if this platform matches the current compilation target.
    pub fn matches_current(&self) -> bool {
        match self {
            Self::Any => true,
            Self::Darwin => cfg!(target_os = "macos"),
            Self::Linux => cfg!(target_os = "linux"),
            Self::Windows => cfg!(target_os = "windows"),
            Self::FreeBSD => cfg!(target_os = "freebsd"),
            Self::Ios => cfg!(target_os = "ios"),
            Self::Android => cfg!(target_os = "android"),
        }
    }

    /// Infer the platform an FFI library targets from its name.
    ///
    /// Used when the codegen sees `@ffi("kernel32.dll")` or
    /// `@ffi("libSystem.B.dylib")` and needs to tag the library with the
    /// platform it actually belongs to — rather than blindly tagging every
    /// library with the current compilation target. Without this, every
    /// library ends up marked as the compilation host's platform, so the
    /// runtime `load_module_libraries` filter can't skip
    /// cross-platform libraries (e.g. `kernel32.dll` on macOS).
    ///
    /// Falls back to `Any` when the name has no obvious platform
    /// signature — correct for genuinely cross-platform libraries and
    /// user-provided `@ffi("mylib")` names.
    pub fn from_library_name(name: &str) -> Self {
        // Lowercase for matching while preserving the original for lookup.
        let lower = name.to_ascii_lowercase();

        // Windows: `.dll` extension, or well-known Win32 module names.
        if lower.ends_with(".dll")
            || lower == "kernel32"
            || lower == "ntdll"
            || lower == "user32"
            || lower == "ws2_32"
            || lower == "winsock2"
            || lower == "advapi32"
            || lower == "gdi32"
        {
            return Self::Windows;
        }

        // macOS / Darwin: `.dylib`, Mach-O frameworks, libSystem, etc.
        if lower.ends_with(".dylib")
            || lower.ends_with(".framework")
            || lower.starts_with("libsystem")
            || lower.contains("libsystem.b.dylib")
            || lower.starts_with("libc++")
            || lower == "corefoundation"
            || lower == "security"
            || lower == "systemconfiguration"
            || lower.starts_with("/system/library/")
        {
            return Self::Darwin;
        }

        // Linux / ELF: `.so` extension.
        if lower.ends_with(".so") || lower.contains(".so.") {
            return Self::Linux;
        }

        // Neutral / unknown — let it be loaded on any platform. This matches
        // user-written `@ffi("mylib")` where they expect the platform loader
        // to map the name appropriately (Linux: libmylib.so, Darwin:
        // libmylib.dylib, Windows: mylib.dll).
        Self::Any
    }
}

/// Calling convention for FFI calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum CallingConvention {
    /// C calling convention (cdecl).
    #[default]
    C = 0,
    /// Windows stdcall.
    Stdcall = 1,
    /// System V AMD64 ABI.
    SysV64 = 2,
    /// Windows fastcall.
    Fastcall = 3,
    /// Microsoft x64 (Windows).
    Win64 = 4,
    /// ARM AAPCS.
    ArmAapcs = 5,
    /// ARM64.
    Arm64 = 6,
    /// Interrupt handler calling convention.
    /// - All registers saved/restored automatically
    /// - Uses iret for return (x86/x86_64)
    /// - First parameter is InterruptStackFrame reference
    ///
    /// Interrupt handler calling convention: all registers are saved/restored automatically,
    /// uses iret for return on x86/x86_64, first parameter is InterruptStackFrame reference.
    /// Annotated with `@interrupt` attribute in Verum source.
    Interrupt = 7,
    /// Naked function - no prologue/epilogue.
    /// Must contain only inline assembly.
    Naked = 8,
}


/// Error handling protocol for FFI calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum ErrorProtocol {
    /// No error handling - function never fails.
    #[default]
    None = 0,
    /// Returns -1 on error, sets errno (POSIX convention).
    NegOneErrno = 1,
    /// Returns null on error, sets errno.
    NullErrno = 2,
    /// Returns 0 on success, error code on failure.
    ZeroSuccess = 3,
    /// Windows HRESULT: negative = failure (FAILED macro).
    HResult = 4,
    /// Reserved for internal use. Not mapped from grammar.
    /// Out-error pointer pattern (e.g., getaddrinfo) is not in Verum grammar.
    _ReservedOutError = 5,
    /// `errors_via = Exception` — C++ exception marker.
    /// Verum does not implement C++ unwinding; this maps to a compile-time
    /// diagnostic recommending extern "C" wrappers. At runtime, treated as None.
    Exception = 6,
    /// Returns sentinel value on error (pattern in error_sentinel).
    /// Unlike NegOneErrno, the sentinel is user-specified.
    ReturnCodePattern = 7,
    /// Returns sentinel on error AND sets errno.
    SentinelWithErrno = 8,
}


/// Memory effects of an FFI call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryEffects(pub u8);

impl MemoryEffects {
    /// No memory effects - pure function.
    pub const PURE: Self = Self(0);
    /// Reads global state.
    pub const READS: Self = Self(1);
    /// Writes global state.
    pub const WRITES: Self = Self(2);
    /// Allocates memory.
    pub const ALLOCS: Self = Self(4);
    /// Frees memory.
    pub const FREES: Self = Self(8);
    /// May perform I/O.
    pub const IO: Self = Self(16);
    /// May throw/longjmp.
    pub const THROWS: Self = Self(32);

    /// Returns true if the function is pure (no side effects).
    pub fn is_pure(&self) -> bool {
        self.0 == 0
    }

    /// Combines two memory effects.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns true if this effect includes the specified effect.
    pub fn contains(&self, effect: Self) -> bool {
        (self.0 & effect.0) == effect.0
    }
}

impl Default for MemoryEffects {
    fn default() -> Self {
        Self::PURE
    }
}

/// C type descriptor for FFI signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CType {
    /// void
    Void = 0,
    /// int8_t / char
    I8 = 1,
    /// int16_t / short
    I16 = 2,
    /// int32_t / int
    I32 = 3,
    /// int64_t / long long
    I64 = 4,
    /// uint8_t / unsigned char
    U8 = 5,
    /// uint16_t / unsigned short
    U16 = 6,
    /// uint32_t / unsigned int
    U32 = 7,
    /// uint64_t / unsigned long long
    U64 = 8,
    /// float
    F32 = 9,
    /// double
    F64 = 10,
    /// void* / generic pointer
    Ptr = 11,
    /// const char* / C string
    CStr = 12,
    /// bool (C99 _Bool)
    Bool = 13,
    /// size_t
    Size = 14,
    /// ssize_t / ptrdiff_t
    Ssize = 15,
    /// Pointer to struct (index into ffi_layouts)
    StructPtr = 16,
    /// Pointer to array
    ArrayPtr = 17,
    /// Function pointer
    FnPtr = 18,
    /// Struct passed/returned by value (layout index stored separately)
    StructValue = 19,
}

impl CType {
    /// Returns true if this C type is a pointer type.
    ///
    /// Pointer types include: Ptr, CStr, StructPtr, ArrayPtr, FnPtr.
    /// These types represent raw FFI pointers that bypass CBGR validation.
    pub fn is_pointer(&self) -> bool {
        matches!(
            self,
            CType::Ptr | CType::CStr | CType::StructPtr | CType::ArrayPtr | CType::FnPtr
        )
    }
}

/// FFI function signature descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiSignature {
    /// Return type.
    pub return_type: CType,
    /// Parameter types.
    pub param_types: SmallVec<[CType; 4]>,
    /// Is this a variadic function?
    pub is_variadic: bool,
    /// Number of fixed parameters (for variadic functions).
    pub fixed_param_count: u8,
    /// Layout index for return type if it's StructValue (index into ffi_layouts).
    #[serde(default)]
    pub return_layout_idx: Option<u16>,
    /// Layout indices for parameters that are StructValue (index into ffi_layouts).
    /// Parallel to param_types - None for non-struct types.
    #[serde(default)]
    pub param_layout_indices: SmallVec<[Option<u16>; 4]>,
}

impl Default for FfiSignature {
    fn default() -> Self {
        Self {
            return_type: CType::Void,
            param_types: SmallVec::new(),
            is_variadic: false,
            fixed_param_count: 0,
            return_layout_idx: None,
            param_layout_indices: SmallVec::new(),
        }
    }
}

impl FfiSignature {
    /// Creates a new signature with the given return type and parameter types.
    pub fn new(return_type: CType, param_types: SmallVec<[CType; 4]>) -> Self {
        let param_layout_indices = SmallVec::from_elem(None, param_types.len());
        Self {
            return_type,
            param_types,
            is_variadic: false,
            fixed_param_count: 0,
            return_layout_idx: None,
            param_layout_indices,
        }
    }

    /// Sets the layout index for struct-by-value return type.
    pub fn with_return_layout(mut self, layout_idx: u16) -> Self {
        self.return_layout_idx = Some(layout_idx);
        self
    }

    /// Sets the layout index for a struct-by-value parameter.
    pub fn with_param_layout(mut self, param_idx: usize, layout_idx: u16) -> Self {
        // Extend param_layout_indices if needed
        while self.param_layout_indices.len() <= param_idx {
            self.param_layout_indices.push(None);
        }
        self.param_layout_indices[param_idx] = Some(layout_idx);
        self
    }
}

/// FFI library descriptor.
///
/// Describes a native library that can be loaded for FFI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiLibrary {
    /// Library name (e.g., "libSystem.B.dylib", "libc.so.6").
    pub name: StringId,
    /// Platform this library is for.
    pub platform: FfiPlatform,
    /// Is this library required? If false, missing library is not an error.
    pub required: bool,
    /// Library version (optional, for documentation).
    pub version: Option<StringId>,
}

impl FfiLibrary {
    /// Creates a new FFI library descriptor.
    pub fn new(name: StringId, platform: FfiPlatform) -> Self {
        Self {
            name,
            platform,
            required: true,
            version: None,
        }
    }
}

/// FFI symbol descriptor.
///
/// Describes a single FFI symbol (function or variable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiSymbol {
    /// Symbol name (e.g., "getpid", "malloc").
    pub name: StringId,
    /// Library index (-1 for default/platform library).
    pub library_idx: i16,
    /// Calling convention.
    pub convention: CallingConvention,
    /// Function signature.
    pub signature: FfiSignature,
    /// Memory effects.
    pub memory_effects: MemoryEffects,
    /// Error handling protocol.
    pub error_protocol: ErrorProtocol,
    /// Sentinel value for ReturnCodePattern/SentinelWithErrno protocols.
    /// For ReturnCode(X): error when result == X.
    /// For ReturnValue(null): 0 (null pointer).
    /// For NegOneErrno: -1 (implicit, not used).
    #[serde(default)]
    pub error_sentinel: i64,
    /// Verum function ID that wraps this FFI symbol (optional).
    pub wrapper_fn: Option<FunctionId>,
    /// Whether this symbol has passed FFI type safety validation.
    #[serde(default)]
    pub validated: bool,
    /// Ownership semantics for pointer parameters.
    #[serde(default)]
    pub ownership: FfiOwnership,
}

/// FFI ownership semantics for pointer parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum FfiOwnership {
    /// Borrowed reference — caller retains ownership.
    #[default]
    Borrow = 0,
    /// Ownership transferred to callee — caller must not use after call.
    TransferTo = 1,
    /// Ownership transferred from callee — caller must free.
    TransferFrom = 2,
    /// Shared access — both sides may access concurrently.
    Shared = 3,
}

/// FFI contract: pre/postconditions for an FFI function.
///
/// `requires` expressions are checked before the call (debug mode only).
/// `ensures` expressions are checked after the call (debug mode only).
/// Stored as stringified expressions — compiled to asserts at call sites.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FfiContract {
    /// Precondition expressions (stringified from AST).
    /// Empty = no preconditions.
    pub requires: Vec<String>,
    /// Postcondition expressions (stringified from AST).
    /// `result` refers to the return value.
    /// Empty = no postconditions.
    pub ensures: Vec<String>,
    /// Whether this function is declared thread-safe.
    pub thread_safe: bool,
}

impl FfiSymbol {
    /// Creates a new FFI symbol descriptor.
    pub fn new(name: StringId, signature: FfiSignature) -> Self {
        Self {
            name,
            library_idx: -1, // Default library
            convention: CallingConvention::C,
            signature,
            memory_effects: MemoryEffects::default(),
            error_protocol: ErrorProtocol::None,
            error_sentinel: 0,
            wrapper_fn: None,
            validated: false,
            ownership: FfiOwnership::default(),
        }
    }
}

/// FFI struct field descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiStructField {
    /// Field name.
    pub name: StringId,
    /// Field type.
    pub c_type: CType,
    /// Byte offset within the struct.
    pub offset: u32,
    /// Field size in bytes.
    pub size: u16,
    /// Field alignment.
    pub align: u16,
}

/// FFI struct layout descriptor.
///
/// Describes the memory layout of a C struct for marshalling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfiStructLayout {
    /// Struct name.
    pub name: StringId,
    /// Total struct size in bytes.
    pub size: u32,
    /// Struct alignment.
    pub align: u16,
    /// Field descriptors.
    pub fields: Vec<FfiStructField>,
    /// Corresponding Verum type ID (if any).
    pub verum_type: Option<TypeId>,
}

impl FfiStructLayout {
    /// Creates a new FFI struct layout.
    pub fn new(name: StringId, size: u32, align: u16) -> Self {
        Self {
            name,
            size,
            align,
            fields: Vec::new(),
            verum_type: None,
        }
    }

    /// Adds a field to the struct layout.
    pub fn add_field(&mut self, field: FfiStructField) {
        self.fields.push(field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_table_intern() {
        let mut table = StringTable::new();

        let id1 = table.intern("hello");
        let id2 = table.intern("world");
        let id3 = table.intern("hello"); // duplicate

        assert_eq!(id1, id3); // Same string, same ID
        assert_ne!(id1, id2); // Different strings, different IDs

        assert_eq!(table.get(id1), Some("hello"));
        assert_eq!(table.get(id2), Some("world"));
    }

    #[test]
    fn test_module_creation() {
        let mut module = VbcModule::new("test_module".to_string());

        // Add a string
        let hello_id = module.intern_string("hello");
        assert_eq!(module.get_string(hello_id), Some("hello"));

        // Add a constant
        let const_id = module.add_constant(Constant::Int(42));
        assert_eq!(module.get_constant(const_id), Some(&Constant::Int(42)));

        // Add a function
        let func = FunctionDescriptor::new(hello_id);
        let func_id = module.add_function(func);
        assert!(module.get_function(func_id).is_some());
    }

    #[test]
    fn test_constant_tags() {
        assert_eq!(Constant::Int(0).tag(), 0x01);
        assert_eq!(Constant::Float(0.0).tag(), 0x02);
        assert_eq!(Constant::String(StringId(0)).tag(), 0x03);
    }
}
