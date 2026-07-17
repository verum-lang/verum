//! VBC module and function structures.
//!

//! This module defines the high-level structures for VBC modules:
//! - [`VbcModule`]: Complete compiled module
//! - [`VbcFunction`]: Individual function with bytecode
//! - [`FunctionDescriptor`]: Function metadata
//! - [`Constant`]: Constant pool entries
//! - [`SpecializationEntry`]: Pre-computed specializations

use std::collections::HashMap;
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
///
/// **XMOD-CALL-ID-BAND-1** — `XMOD_CALL_ID_BAND_BASE` below is the base
/// of the reserved id band that cross-module `Call`-family references
/// occupy in ARCHIVE bytecode.
///
/// The precompiler remaps module-LOCAL call targets to contiguous
/// `[0, N)` ids and re-homes every plain cross-module target to
/// `XMOD_CALL_ID_BAND_BASE + seq` (see the emission pass in
/// `codegen/mod.rs`), recording `(band_id, qualified_name)` in
/// [`VbcModule::external_function_names`].  Keeping the two id spaces
/// DISJOINT is what makes `ArchiveBodyRemap::map_function`'s
/// name-based Tier-0 resolution sound: an id-keyed external map over
/// an OVERLAPPING id space is structurally ambiguous in either
/// priority order (local-first broke `Deque.reallocate → realloc`;
/// external-first broke `get_heap_stats → get_heap`).
///
/// Band: `[0x2000_0000, 0x4000_0000)` — far above any realistic
/// function count, below the extern-sentinel threshold (`u32::MAX/4`)
/// and the stage-1/2/3 stub ranges near `u32::MAX`.
pub const XMOD_CALL_ID_BAND_BASE: u32 = 0x2000_0000;

/// First dense context slot available for compiler-assigned context
/// types (CTX-STORE-AUTHORITY-1). Slots below this are the statically-
/// allocated stdlib region (`EXEC_ENV_SLOT=0`, `SLOT_DATABASE=10`,
/// `SLOT_LOGGER=11`, …). Mirrors `CONTEXT_DYNAMIC_SLOT_BASE` in
/// `core/sys/common.vr` and the `CTX_DYNAMIC_SLOT_BASE` codegen constant
/// (`verum_codegen/src/llvm/context.rs`) — the three MUST stay equal.
pub const CTX_DYNAMIC_SLOT_BASE: u32 = 32;

/// Number of context slots. Mirrors `CONTEXT_SLOT_COUNT` in
/// `core/sys/common.vr` and `CTX_SLOT_COUNT` in the interpreter's
/// `dispatch_table/handlers/ctx_runtime.rs`.
pub const CTX_SLOT_COUNT: u32 = 256;

/// Collect the raw `ctx_type` string-table id of a context instruction
/// (`CtxGet` / `CtxProvide` / `CtxCheckNegative`) into `ids`; ignores
/// every other opcode. Shared by both scan representations in
/// [`VbcModule::ctx_dense_slot_map`].
fn push_ctx_type_id(instr: &Instruction, ids: &mut Vec<u32>) {
    match instr {
        Instruction::CtxGet { ctx_type, .. }
        | Instruction::CtxProvide { ctx_type, .. }
        | Instruction::CtxCheckNegative { ctx_type, .. } => ids.push(*ctx_type),
        _ => {}
    }
}

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

    // ========================================================================
    // Precompiled-stdlib archive extensions (Phase 3 of #precompile-stdlib).
    //
    // Single universal archive holds every platform's variants of cfg-
    // conditional functions. The runtime / AOT loader builds a CfgKey
    // from the active target triple and selects per-function variants
    // through `function_variants`. Functions absent from
    // `function_variants` are universal — `bytecode_offset/length` on
    // their `FunctionDescriptor` is the only body, used regardless of
    // target.
    // ========================================================================
    /// Deduplicated cfg-key table — referenced by `VbcVariant::cfg_key_id`.
    /// Empty when the module has no target-conditional functions.
    #[serde(default)]
    pub cfg_keys: Vec<crate::cfg_key::CfgKey>,

    /// Per-function variant tables — sparse list, only target-conditional
    /// functions appear. The loader picks the first variant whose
    /// `cfg_key_id` matches the active triple's CfgKey and overrides the
    /// FunctionDescriptor's bytecode_offset/bytecode_length.
    #[serde(default)]
    pub function_variants: Vec<FunctionVariantSet>,

    /// Theorem / axiom / lemma / corollary / tactic table. Items here
    /// are the proof-layer-only artefacts the precompile-stdlib epic
    /// keeps separate from runtime function bodies — they're only
    /// loaded by `--verify formal`, audit, and replay tooling.
    #[serde(default)]
    pub theorems: Vec<TheoremEntry>,

    /// `@framework(name, citation)` provenance edges and
    /// `@framework_translate(src, tgt, citation)` bridge edges.
    /// Loaded eagerly because the table is small (~1 KB) and audit
    /// tooling consults it on every invocation.
    #[serde(default)]
    pub framework_provenance: FrameworkProvenance,

    /// Discharge receipts for refinement obligations and theorems.
    /// Each entry is a content-hash pointer into the per-binary
    /// cert-store (`~/.verum/cert-store/<hash>`); the body of the
    /// proof is never inlined into the archive — only its hash plus
    /// the kernel rule that produced it.
    #[serde(default)]
    pub discharge_receipts: Vec<DischargeReceipt>,
    /// Cross-module call name table: maps every `FunctionId` that this
    /// module's bytecode references via `Call` / `CallG` / `TailCall` /
    /// `NewClosure` / `Spawn` / `GenCreate` but whose body lives in
    /// *another* archive module, to its qualified function name. The
    /// archive loader (`crates/verum_compiler/src/archive_ctx_loader.rs`)
    /// merges this table into `archive_id_to_name` so the per-module
    /// remap's Tier-2 name-based fallback resolves cross-module callees
    /// without each external reference paying the cost of a full
    /// FunctionDescriptor stub.
    ///
    /// Carries `(XMOD-band FunctionId, qualified-name StringId)` for
    /// plain cross-module references (see [`XMOD_CALL_ID_BAND_BASE`])
    /// and `(stage-stub FunctionId, name)` for stage-1/2/3 stub
    /// references.  Empty when the module has no cross-module
    /// references (e.g. tiny user scripts that import nothing).
    #[serde(default)]
    pub external_function_names: Vec<(FunctionId, StringId)>,

    /// **CROSS-MODULE-CALL-STRINGID (T0144)** — carried-fact band
    /// resolution: `band/stub FunctionId → concrete merged-table
    /// FunctionId`, computed ONCE per assembled module by
    /// [`Self::resolve_external_bands`] and consulted FIRST by every
    /// consumer (Tier-0 Call dispatch, AOT band-id lowering) before
    /// any per-consumer name chase. NOT serialized: the map is a fact
    /// about the FINAL merged function table, not about the wire
    /// module (the same archive merges into different user tables).
    ///
    /// Bytecode is deliberately NOT rewritten — band-id varints are
    /// wider than concrete ids and re-encoding would shift every
    /// branch offset (the envelope-pc lesson); a lookup table keeps
    /// the id spaces disjoint and the resolution single-sourced.
    #[serde(skip)]
    pub resolved_band_map: std::collections::HashMap<u32, FunctionId>,

    /// Mount-rename alias table — Phase 1 of task #11 fundamental fix
    /// (see `task11-mount-alias-aot-fix spec`).
    ///
    /// Each entry records a `mount X.{NAME as ALIAS}` rename that the
    /// module declared via a top-level `MountDecl`.  At precompile time
    /// `register_import_aliases` resolves `X.NAME` to its canonical
    /// FunctionId; this table preserves the `ALIAS → FunctionId` pair
    /// across the archive boundary so the user-side AOT loader can
    /// re-establish the alias in `ctx.functions` before re-compiling
    /// any function body from this module.
    ///
    /// Carries `(alias-name StringId, canonical-FunctionId)`.  Empty
    /// when the module declares no mount-rename aliases.  Loader-side
    /// consumer: `crates/verum_compiler/src/archive_ctx_loader.rs::
    /// apply_lazy_with_types` (planned).
    #[serde(default)]
    /// (alias name, best-effort archive-local fid, RESOLVED TARGET
    /// canonical key).  The target key is the load-time authority
    /// (REEXPORT-QUALIFIED-KEY-1): archive fids are renumbered
    /// per-entry at serialization, so an alias whose target lives in a
    /// DIFFERENT entry (every cross-subtree re-export) is unmappable
    /// by fid alone.
    pub mount_aliases: Vec<(StringId, FunctionId, StringId)>,

    /// ARCHIVE-TYPE-GLUE-IDS-1: lazily-built `TypeId.0 → types-vec
    /// index` reverse cache for id-correct descriptor resolution on
    /// runtime hot paths (`DropRef` drop-glue dispatch).
    ///
    /// `TypeDescriptor.id` is NOT positional in `types` — well-known-id
    /// backfills (Maybe / Result descriptors pushed under pre-allocated
    /// ids) and codegen allocation gaps shift descriptors relative to
    /// their ids; see `get_type`'s non-positional contract.  Any
    /// `types[id - FIRST_USER]` indexing therefore resolves the WRONG
    /// descriptor in general — latent while imported-type glue ids were
    /// cleared (every descriptor's `drop_fn` was `None`, so a wrong hit
    /// was still a no-op), loud once real drop glue is live.
    ///
    /// First-wins on duplicate ids, mirroring `get_type`'s `.find`
    /// semantics.  `OnceLock` (not `OnceCell`) keeps `Arc<VbcModule>`
    /// `Send + Sync`; skipped from serialize to keep the wire format
    /// unchanged — mirrors `StringTable::id_to_idx`.
    #[serde(skip, default)]
    pub(crate) type_idx_by_id: std::sync::OnceLock<std::collections::HashMap<u32, usize>>,
}

impl Default for VbcModule {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl VbcModule {
    /// Canonical raw-`ctx_type` → dense-slot map (CTX-STORE-AUTHORITY-1).
    ///
    /// The SINGLE source of the context slot numbering shared by both
    /// tiers: the Tier-1 LLVM lowering (`FunctionContext::ctx_dense_slot`
    /// in `verum_codegen`) and the Tier-0 interpreter
    /// (`InterpreterState::ctx_dense_slot`) both derive the slot for a
    /// given context type from THIS map, so `provide` on one tier and
    /// `get` / the user-callable `ctx_get(slot)` surface on the other
    /// agree on the slot.
    ///
    /// VBC `ctx_type` operands are raw string-table ids (empirically
    /// ≥ 256 under the baked stdlib), while the TLS / bindings slot table
    /// only holds `0..CTX_SLOT_COUNT`. Distinct ids are numbered in
    /// ascending order from [`CTX_DYNAMIC_SLOT_BASE`], keeping
    /// compiler-assigned dynamic slots above the statically-allocated
    /// stdlib region.
    ///
    /// Derivation is representation-agnostic: it scans decoded
    /// [`FunctionDescriptor::instructions`] when present (the codegen
    /// path), else decodes the concatenated [`Self::bytecode`] (the
    /// interpreter path, whose loaded modules carry only raw bytecode).
    /// Because the numbering depends solely on the *set* of context-type
    /// ids, both representations of the same module yield an identical
    /// map. Ids beyond the dynamic slot capacity are dropped (the caller
    /// turns an out-of-range lookup into a loud error, never a silent
    /// wrap).
    pub fn ctx_dense_slot_map(&self) -> HashMap<u32, u32> {
        let mut ids: Vec<u32> = Vec::new();
        let mut saw_decoded = false;
        for func in &self.functions {
            if let Some(instrs) = &func.instructions {
                saw_decoded = true;
                for instr in instrs {
                    push_ctx_type_id(instr, &mut ids);
                }
            }
        }
        // Interpreter path: the loaded module carries no decoded
        // instruction lists — recover the context ops from the raw
        // concatenated bytecode stream instead.
        if !saw_decoded
            && !self.bytecode.is_empty()
            && let Ok(decoded) = crate::bytecode::decode_instructions(&self.bytecode)
        {
            for instr in &decoded {
                push_ctx_type_id(instr, &mut ids);
            }
        }

        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .take((CTX_SLOT_COUNT - CTX_DYNAMIC_SLOT_BASE) as usize)
            .enumerate()
            .map(|(rank, id)| (id, CTX_DYNAMIC_SLOT_BASE + rank as u32))
            .collect()
    }

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
            // Phase 3 (precompile-stdlib epic) — empty by default; only
            // populated when the module is built by precompile-stdlib /
            // precompile-cog or merged from a `.vbca` archive.
            cfg_keys: Vec::new(),
            function_variants: Vec::new(),
            theorems: Vec::new(),
            framework_provenance: FrameworkProvenance::default(),
            discharge_receipts: Vec::new(),
            external_function_names: Vec::new(),
            resolved_band_map: std::collections::HashMap::new(),
            mount_aliases: Vec::new(),
            type_idx_by_id: std::sync::OnceLock::new(),
        }
    }

    /// Resolve the bytecode region for `function_id` against the active
    /// target `cfg`. Universal functions (no entry in
    /// `function_variants`) return their descriptor's `bytecode_offset`
    /// + `bytecode_length` directly. Multi-variant functions consult
    /// the variant table; the first variant whose `cfg_key_id` matches
    /// `cfg` wins.
    ///
    /// Returns `None` when the function exists but no variant matches
    /// the active target — the loader treats this as "function elided
    /// for this target" and surfaces a typed `FunctionNotFound` if a
    /// caller tries to dispatch it.
    pub fn resolve_bytecode_region(
        &self,
        function_id: FunctionId,
        cfg: &crate::cfg_key::CfgKey,
    ) -> Option<(u32, u32)> {
        let desc = self.functions.get(function_id.0 as usize)?;
        // Fast path: not in variant table → universal.
        let Some(set) = self
            .function_variants
            .iter()
            .find(|s| s.function_id == function_id)
        else {
            return Some((desc.bytecode_offset, desc.bytecode_length));
        };
        // Multi-variant: pick first matching cfg.
        for variant in &set.variants {
            let key = self.cfg_keys.get(variant.cfg_key_id as usize)?;
            if key.matches(cfg) {
                return Some((variant.bytecode_offset, variant.bytecode_length));
            }
        }
        // No matching variant — function elided for this target.
        None
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
        // Keep the id-correct reverse cache coherent when it is
        // already materialised (first-wins, mirroring `get_type`).
        if let Some(m) = self.type_idx_by_id.get_mut() {
            m.entry(desc.id.0).or_insert(self.types.len());
        }
        self.types.push(desc);
        id
    }

    /// ARCHIVE-TYPE-GLUE-IDS-1: id-correct `types` index lookup —
    /// O(1) amortised after the first call.  Returns the index of the
    /// FIRST descriptor whose `id` field matches (identical semantics
    /// to `get_type`, without the O(N) scan per call).  Use this —
    /// never positional `types[id - FIRST_USER]` indexing — to resolve
    /// a runtime `ObjectHeader.type_id` to its descriptor: descriptor
    /// ids are not positional (see `type_idx_by_id` field docs).
    pub fn type_index_by_id(&self, id: TypeId) -> Option<usize> {
        let map = self.type_idx_by_id.get_or_init(|| {
            let mut m = std::collections::HashMap::with_capacity(self.types.len());
            for (i, t) in self.types.iter().enumerate() {
                m.entry(t.id.0).or_insert(i);
            }
            m
        });
        map.get(&id.0).copied()
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
            TypeRef::Function {
                params,
                return_type,
                ..
            } => {
                let params_str: Vec<String> =
                    params.iter().map(|p| self.display_type_ref(p)).collect();
                format!(
                    "fn({}) -> {}",
                    params_str.join(", "),
                    self.display_type_ref(return_type)
                )
            }
            TypeRef::Rank2Function {
                type_param_count,
                params,
                return_type,
                ..
            } => {
                let params_str: Vec<String> =
                    params.iter().map(|p| self.display_type_ref(p)).collect();
                format!(
                    "fn<{}>({}) -> {}",
                    type_param_count,
                    params_str.join(", "),
                    self.display_type_ref(return_type)
                )
            }
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => {
                let m = match mutability {
                    Mutability::Immutable => "&",
                    Mutability::Mutable => "&mut ",
                };
                let t = match tier {
                    CbgrTier::Tier0 => "",
                    CbgrTier::Tier1 => "checked ",
                    CbgrTier::Tier2 => "unsafe ",
                };
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
            TypeRef::AssociatedProjection { base, assoc } => {
                format!("{}.{}", self.display_type_ref(base), assoc)
            }
            // Const-generic VALUE argument renders as its literal —
            // `StackAllocator<256>` round-trips as written.
            TypeRef::ConstValue(v) => v.to_string(),
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
            _ => self
                .get_type_name(tid)
                .unwrap_or_else(|| format!("type#{}", tid.0)),
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

    /// Finds a function by qualified name (e.g., "Result.unwrap"). Used by
    /// the runtime dispatcher to defer to user-compiled methods when a
    /// qualified call site has a real implementation, rather than falling
    /// through to the primitive-variant fallback that can't distinguish
    /// `Result.Err.unwrap()` from `Maybe.Some.unwrap()`. Returns the
    /// FunctionId for chaining or None if no function with that name
    /// exists in this module.
    ///

    /// First tries exact match. If exact match fails AND the queried name
    /// looks qualified (contains `.`), falls back to suffix-match against
    /// the queried name preceded by `.` — so `find_function_by_name("Result.unwrap")`
    /// also matches a fully-namespaced registration like
    /// `core.base.result.Result.unwrap`. This is the canonical disambiguation
    /// discipline used elsewhere in codegen (`find_variant_by_suffix_and_args`,
    /// `find_function_by_suffix`) and keeps the dispatcher's `prefer_user_compiled`
    /// guard sound regardless of whether stdlib symbols are registered with
    /// short or fully-qualified module paths.
    /// SAME-NAME-PARENT-TIEBREAK-1 (task #50): resolve a qualified
    /// method name PREFERRING the candidate whose `parent_type` matches
    /// the receiver's runtime TypeId. The stdlib carries same-named
    /// types (115 duplicate public names — e.g. TWO `Rational`s), so
    /// "Rational.mul" is ambiguous by NAME: the plain resolver's
    /// body-over-stub + lowest-id rule picked the OTHER Rational's
    /// body (raw Int-field Mul over BigInt pointers). The receiver's
    /// header TypeId is the ground truth the name cannot express.
    /// Falls back to the canonical name-only rule when no candidate
    /// matches the receiver (or no receiver id is available).
    pub fn find_function_by_name_for_receiver(
        &self,
        name: &str,
        receiver_tid: Option<crate::types::TypeId>,
    ) -> Option<FunctionId> {
        if let Some(tid) = receiver_tid {
            if std::env::var("VERUM_TRACE_TIEBREAK").is_ok() {
                for (idx, d) in self.functions.iter().enumerate() {
                    if self.get_string(d.name) == Some(name) {
                        eprintln!(
                            "[tiebreak] '{}' cand idx={} parent={:?} bodied={} (recv_tid={})",
                            name, idx, d.parent_type,
                            d.bytecode_length > 0
                                || d.instructions.as_ref().map(|i| !i.is_empty()).unwrap_or(false),
                            tid.0,
                        );
                    }
                }
            }
            let has_body = |desc: &crate::module::FunctionDescriptor| -> bool {
                desc.bytecode_length > 0
                    || desc
                        .instructions
                        .as_ref()
                        .map(|i| !i.is_empty())
                        .unwrap_or(false)
            };
            let mut best: Option<(bool, u32)> = None;
            for (idx, desc) in self.functions.iter().enumerate() {
                if desc.parent_type != Some(tid) {
                    continue;
                }
                if let Some(fname) = self.get_string(desc.name)
                    && fname == name
                {
                    let bodied = has_body(desc);
                    let better = match best {
                        None => true,
                        Some((b_bodied, b_idx)) => {
                            (bodied && !b_bodied) || (bodied == b_bodied && (idx as u32) < b_idx)
                        }
                    };
                    if better {
                        best = Some((bodied, idx as u32));
                    }
                }
            }
            if let Some((bodied, idx)) = best {
                if bodied {
                    return Some(FunctionId(idx));
                }
            }
        }
        self.find_function_by_name(name)
    }

    /// **T0144** — compute the carried-fact band-resolution map for
    /// this ASSEMBLED module: every `(band/stub id, qualified name)`
    /// entry in [`Self::external_function_names`] is resolved against
    /// the final function table via
    /// [`Self::resolve_function_by_name_ranked`] (exact → head-strip
    /// → ranked qualified-suffix). Call ONCE after the module reaches
    /// its final shape (post archive/mono merge); consumers then use
    /// [`Self::resolve_band_id`]. Returns the entries that did NOT
    /// resolve — callers surface them loudly (they are calls whose
    /// target body is genuinely absent from the module).
    pub fn resolve_external_bands(&mut self) -> Vec<(u32, String)> {
        let mut unresolved: Vec<(u32, String)> = Vec::new();
        let mut resolved: std::collections::HashMap<u32, FunctionId> =
            std::collections::HashMap::with_capacity(self.external_function_names.len());
        let entries: Vec<(u32, String)> = self
            .external_function_names
            .iter()
            .filter_map(|(fid, sid)| {
                self.get_string(*sid).map(|n| (fid.0, n.to_string()))
            })
            .collect();
        for (band_id, name) in entries {
            match self.resolve_function_by_name_ranked(&name) {
                Some(target) if target.0 != band_id => {
                    resolved.insert(band_id, target);
                }
                _ => unresolved.push((band_id, name)),
            }
        }
        self.resolved_band_map = resolved;
        unresolved
    }

    /// **T0144** — consult the carried-fact band resolution. Returns
    /// the concrete merged-table id for a band/stub reference id, when
    /// [`Self::resolve_external_bands`] resolved it.
    #[inline]
    pub fn resolve_band_id(&self, id: u32) -> Option<FunctionId> {
        self.resolved_band_map.get(&id).copied()
    }

    /// **T0103 LEG-2b** — materialize registry-intrinsic wrapper bodies
    /// for band names that [`Self::resolve_external_bands`] could NOT
    /// resolve against the function table.
    ///
    /// Cross-module calls to intrinsic fn-forms are recorded under the
    /// MOUNTING module's spelling (`core.base.primitives.eq` for
    /// `core.intrinsics.arithmetic.eq`), which shares only the bare
    /// last segment with the defining registration — outside ranked
    /// suffix resolution by design (a bare-name rank would be the
    /// shadowing hazard of T0231). When such a name's bare segment IS a
    /// registered intrinsic, the registry is the semantic authority
    /// (intrinsic-dispatch-contract §1): synthesize a wrapper body via
    /// [`crate::intrinsics::expand::expand_intrinsic_wrapper`], append
    /// it to the function table, and bind the band id to it. Both
    /// tiers then execute/lower the same body — the AOT const-zero
    /// degrade and the Tier-0 `FunctionNotFound` panic for this class
    /// disappear together.
    ///
    /// Scope guards, each load-bearing:
    /// * only `core.`-rooted names — a user function that happens to
    ///   end in `.eq` can never be hijacked;
    /// * only names ranked resolution MISSED — a real body (user or
    ///   stdlib, template or specialization) always wins;
    /// * only registry-covered bare names with a synthesizable
    ///   strategy — everything else stays loudly unresolved for the
    ///   absent-bodies policy (T0103 LEG-3).
    ///
    /// Call after [`Self::resolve_external_bands`] on an EXECUTION
    /// assembly (interp module, AOT pre-lowering module). Deliberately
    /// NOT part of `resolve_external_bands` itself so archive
    /// *encoding* paths never serialize synthesized bodies (bake
    /// byte-identity, defect-class §40).
    ///
    /// Kill switch: `VERUM_DISABLE_INTRINSIC_WRAPPERS=1`.
    /// Trace: `VERUM_TRACE_BAND_WRAPPER=1`.
    pub fn synthesize_intrinsic_band_wrappers(&mut self) -> usize {
        if std::env::var_os("VERUM_DISABLE_INTRINSIC_WRAPPERS").is_some() {
            return 0;
        }
        // Snapshot unresolved (band_id, name) pairs; sort by (name, id)
        // so appended FunctionIds are deterministic across runs
        // (bake-nondeterminism discipline).
        let mut pending: Vec<(u32, String)> = self
            .external_function_names
            .iter()
            .filter(|(fid, _)| !self.resolved_band_map.contains_key(&fid.0))
            .filter_map(|(fid, sid)| self.get_string(*sid).map(|n| (fid.0, n.to_string())))
            .collect();
        pending.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

        let trace = std::env::var_os("VERUM_TRACE_BAND_WRAPPER").is_some();
        // Wrappers register under a DOTLESS technical name
        // (`__band_wrapper$core$base$primitives$eq`): the only route to
        // a wrapper is the band map. A dotted registration would join
        // the `find_function_by_name` suffix-scan universe (`…ends_with
        // (".primitives.eq")`) and could silently capture OTHER
        // modules' re-export spellings — the loud-to-wrong inversion
        // this leg must never introduce.
        let mangle = |name: &str| format!("__band_wrapper${}", name.replace('.', "$"));
        // Idempotence across map recomputes: the post-mono
        // `resolve_external_bands` wipes the map, and mangled names are
        // deliberately invisible to ranked resolution — reuse an
        // already-synthesized wrapper instead of appending a duplicate.
        let mut synthesized_by_name: std::collections::HashMap<String, FunctionId> =
            std::collections::HashMap::new();
        for (idx, desc) in self.functions.iter().enumerate() {
            if let Some(n) = self.get_string(desc.name)
                && n.starts_with("__band_wrapper$")
            {
                synthesized_by_name.insert(n.to_string(), FunctionId(idx as u32));
            }
        }
        let mut bound = 0usize;
        for (band_id, name) in pending {
            if !name.starts_with("core.") {
                continue;
            }
            let mangled = mangle(&name);
            let fid = if let Some(&fid) = synthesized_by_name.get(&mangled) {
                fid
            } else {
                let bare = name.rsplit('.').next().unwrap_or(name.as_str());
                let Some(info) = crate::intrinsics::lookup_intrinsic(bare) else {
                    continue;
                };
                let Some(body) =
                    crate::intrinsics::expand::expand_intrinsic_wrapper(info.intrinsic)
                else {
                    continue;
                };
                let bytecode_offset = self.bytecode.len() as u32;
                let bytecode_length =
                    crate::bytecode::encode_instructions(&body.instructions, &mut self.bytecode)
                        as u32;
                let name_id = self.intern_string(&mangled);
                let params: smallvec::SmallVec<[ParamDescriptor; 4]> = (0..info
                    .intrinsic
                    .param_count)
                    .map(|_| ParamDescriptor {
                        name: StringId::EMPTY,
                        type_ref: crate::types::TypeRef::Concrete(crate::types::TypeId::INT),
                        is_mut: false,
                        default: None,
                    })
                    .collect();
                let desc = FunctionDescriptor {
                    id: FunctionId(self.functions.len() as u32),
                    name: name_id,
                    parent_type: None,
                    type_params: smallvec::SmallVec::new(),
                    params,
                    return_type: crate::types::TypeRef::Concrete(crate::types::TypeId::INT),
                    contexts: smallvec::SmallVec::new(),
                    properties: crate::types::PropertySet::default(),
                    bytecode_offset,
                    bytecode_length,
                    locals_count: 0,
                    register_count: body.register_count,
                    max_stack: 0,
                    is_inline_candidate: true,
                    is_generic: false,
                    visibility: crate::types::Visibility::Public,
                    is_generator: false,
                    yield_type: None,
                    suspend_point_count: 0,
                    calling_convention: Default::default(),
                    optimization_hints: Default::default(),
                    instructions: Some(body.instructions),
                    func_id_base: 0,
                    debug_variables: Vec::new(),
                    is_test: false,
                    is_gpu_only: false,
                    intrinsic_name: None,
                    is_const: false,
                    register_type_hints: Vec::new(),
                    return_type_name: None,
                };
                let fid = self.add_function(desc);
                if trace {
                    eprintln!(
                        "[band-wrapper] synthesized `{}` (bare `{}`, registered `{}`) as fn#{} ({} instrs, {} regs)",
                        name,
                        bare,
                        mangled,
                        fid.0,
                        self.functions[fid.0 as usize]
                            .instructions
                            .as_ref()
                            .map(|i| i.len())
                            .unwrap_or(0),
                        self.functions[fid.0 as usize].register_count,
                    );
                }
                synthesized_by_name.insert(mangled, fid);
                fid
            };
            self.resolved_band_map.insert(band_id, fid);
            bound += 1;
            if trace {
                eprintln!("[band-wrapper] band id {:#x} -> fn#{}", band_id, fid.0);
            }
        }
        bound
    }

    /// Ranked qualified-name resolution over THIS module's function
    /// table (T0103 LEG-2a) — the module-level twin of the loader's
    /// `ArchiveBodyRemap::qualified_suffix_chase` (codegen/mod.rs), so
    /// the AOT lowering's band-id name chase resolves with the same
    /// power the interpreter's merge path has. Order:
    ///
    ///  1. exact name (`find_function_by_name` — bodied beats stub);
    ///  2. progressive head-strip: leading lower-case module segments
    ///     may be spelled in the recorded name but absent from the
    ///     registration key (`core.time.Instant.now` recorded,
    ///     `Instant.now` registered) — exact probe per stripped level,
    ///     at least two segments kept;
    ///  3. ranked `.suffix` scan for the inverse shape (`Mutex.new`
    ///     recorded, `core.sync.mutex.Mutex.new` registered): bodied
    ///     beats stub, then fewest dots, then module-parent before
    ///     type-parent, then lexicographic — the stable tie-break
    ///     every ranked resolver in the toolchain uses.
    ///
    /// NOTE: the loader chase ranks over PRE-merge ctx/archive
    /// indexes; this one ranks over the merged table. Unifying those
    /// universes is T0144's resolution-map work — keep the ranking
    /// DISCIPLINE identical when touching either.
    pub fn resolve_function_by_name_ranked(&self, name: &str) -> Option<FunctionId> {
        if let Some(fid) = self.find_function_by_name(name) {
            return Some(fid);
        }
        if !name.contains('.') {
            return None;
        }
        // Step 2: progressive head-strip of module-shaped segments.
        let segs: Vec<&str> = name.split('.').collect();
        for k in 1..segs.len().saturating_sub(1) {
            if !segs[k - 1]
                .chars()
                .next()
                .map(|c| c.is_ascii_lowercase())
                .unwrap_or(false)
            {
                break;
            }
            let stripped = segs[k..].join(".");
            if let Some(fid) = self.find_function_by_name(&stripped) {
                return Some(fid);
            }
        }
        // Step 3: ranked suffix scan (also try the `core.`-stripped
        // spelling as the suffix floor, mirroring the loader).
        let target: &str = match name.strip_prefix("core.") {
            Some(stripped) if stripped.contains('.') => stripped,
            _ => name,
        };
        let suffix = format!(".{}", target);
        let has_body = |desc: &crate::module::FunctionDescriptor| -> bool {
            desc.bytecode_length > 0
                || desc
                    .instructions
                    .as_ref()
                    .map(|i| !i.is_empty())
                    .unwrap_or(false)
        };
        let parent_is_module = |k: &str| -> bool {
            let ks: Vec<&str> = k.split('.').collect();
            ks.len() >= 2
                && ks[ks.len() - 2]
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_lowercase())
                    .unwrap_or(false)
        };
        let mut best: Option<(bool, usize, bool, &str, FunctionId)> = None;
        for desc in self.functions.iter() {
            if crate::stub_ranges::is_stub_id(desc.id.0) {
                continue;
            }
            let Some(key) = self.get_string(desc.name) else {
                continue;
            };
            if !key.ends_with(&suffix) {
                continue;
            }
            let cand = (
                !has_body(desc),          // bodied first (false < true)
                key.matches('.').count(), // fewest dots
                !parent_is_module(key),   // module-parent first
                key,                      // lexicographic
                desc.id,
            );
            let better = match &best {
                None => true,
                Some(b) => (cand.0, cand.1, cand.2, cand.3) < (b.0, b.1, b.2, b.3),
            };
            if better {
                best = Some(cand);
            }
        }
        best.map(|(_, _, _, _, fid)| fid)
    }

    pub fn find_function_by_name(&self, name: &str) -> Option<FunctionId> {
        // ARCH-P2 step 0b (dispatch tie-break determinism): among
        // SAME-NAMED entries the winner used to be "lowest id" — an
        // interning-order artifact that made runtime dispatch depend
        // on bake layout (re-idding flipped stub-vs-real winners; the
        // 39-test signature that gated id canonicalization). Canonical
        // rule: a REAL BODY (bytecode/instructions present) beats a
        // synthesized stub regardless of position; ids only break
        // exact ties (same name, same bodiedness — semantically
        // interchangeable).
        let has_body = |desc: &crate::module::FunctionDescriptor| -> bool {
            desc.bytecode_length > 0
                || desc
                    .instructions
                    .as_ref()
                    .map(|i| !i.is_empty())
                    .unwrap_or(false)
        };
        // Exact match first
        let mut exact: Option<(bool, u32)> = None;
        for (idx, desc) in self.functions.iter().enumerate() {
            if let Some(fname) = self.get_string(desc.name)
                && fname == name
            {
                let bodied = has_body(desc);
                match exact {
                    None => exact = Some((bodied, idx as u32)),
                    Some((false, _)) if bodied => {
                        exact = Some((true, idx as u32))
                    }
                    _ => {}
                }
                if bodied {
                    // First bodied match is canonical — no later entry
                    // can outrank it.
                    break;
                }
            }
        }
        if let Some((_, idx)) = exact {
            return Some(FunctionId(idx));
        }
        // Suffix match: ".name" against fully-qualified registrations
        if name.contains('.') {
            let suffix = format!(".{}", name);
            let mut sfx: Option<(bool, u32)> = None;
            for (idx, desc) in self.functions.iter().enumerate() {
                if let Some(fname) = self.get_string(desc.name)
                    && fname.ends_with(&suffix)
                {
                    let bodied = has_body(desc);
                    match sfx {
                        None => sfx = Some((bodied, idx as u32)),
                        Some((false, _)) if bodied => {
                            sfx = Some((true, idx as u32))
                        }
                        _ => {}
                    }
                    if bodied {
                        break;
                    }
                }
            }
            if let Some((_, idx)) = sfx {
                return Some(FunctionId(idx));
            }
        }
        None
    }

    /// Like [`find_function_by_name`] but always tries a `.name`
    /// suffix match against fully-qualified registrations even when
    /// the input doesn't contain a dot.  Returns `Some` only when
    /// EXACTLY ONE function ends with `.name` (otherwise the lookup
    /// would be ambiguous — multiple stdlib types might define a
    /// method with the same simple name and silently picking one
    /// would mask a bug class as a wrong-dispatch).
    ///
    /// Used by the interpreter's method-dispatch fallback when
    /// codegen emitted a CallM with a BARE method name (because
    /// `infer_expr_type_name` couldn't statically determine the
    /// receiver's type at codegen time).
    pub fn find_function_by_unique_bare_suffix(&self, bare_name: &str) -> Option<FunctionId> {
        // SOUNDNESS (task #io-1 dispatcher leg): namespace-shaped free
        // fns (sys.linux.syscall.read, sys.darwin.libsystem.safe_write,
        // core.io.fs.write, core.io.file.write, core.shell.builtins.write,
        // etc.) CANNOT be invoked as instance methods — even when their
        // qualified name ends with a common method suffix (`.read` /
        // `.write` / `.close` / `.seek` / `.flush`).  When a user
        // method-call site emits a bare CallM (because the codegen
        // couldn't extract the receiver's static type), the
        // bare-suffix scan was previously accepting these free fns as
        // targets, mis-dispatching `s.write(buf)` for `s: Sink` to
        // `sys.linux.syscall.write`.  Reject them here so the scan
        // falls through to the legitimate user-typed method.
        // Architectural rule pinned in source: namespace-qualified
        // free fns (sys / core.sys / core.io.fs / core.io.file /
        // core.shell / core.net / core.database) participate ONLY in
        // `Call(fn_id)` dispatch, never in `CallM(receiver, method_id)`
        // bare-suffix scan.
        let is_namespace_shadow = |fname: &str| -> bool {
            fname.starts_with("sys.")
                || fname.starts_with("core.sys.")
                || fname.starts_with("core.io.fs.")
                || fname.starts_with("core.io.file.")
                || fname.starts_with("core.shell.")
                || fname.starts_with("core.net.")
                || fname.starts_with("core.database.")
        };
        if let Some(id) = self.find_function_by_name(bare_name) {
            // Don't accept a bare-name exact match that is itself a
            // namespace shadow.
            if let Some(desc) = self.functions.get(id.0 as usize)
                && let Some(fname) = self.get_string(desc.name)
                && is_namespace_shadow(fname)
            {
                // Fall through to suffix-scan in the namespace-shadow case.
            } else {
                return Some(id);
            }
        }
        let suffix = format!(".{}", bare_name);
        let mut found: Option<FunctionId> = None;
        for (idx, desc) in self.functions.iter().enumerate() {
            if let Some(fname) = self.get_string(desc.name) {
                if is_namespace_shadow(fname) {
                    continue;
                }
                if fname.ends_with(&suffix) {
                    if found.is_some() {
                        // Ambiguous — multiple types own a method
                        // with this simple name.  Refuse the lookup;
                        // the caller's qualified-form path or a
                        // type-aware dispatch should disambiguate.
                        return None;
                    }
                    found = Some(FunctionId(idx as u32));
                }
            }
        }
        found
    }

    /// TypeId-aware bare-name lookup for the runtime method-dispatch
    /// fallback.  When codegen emits a CallM with a BARE method name
    /// (because `infer_expr_type_name` couldn't statically determine
    /// the receiver's type), the user-compiled body lives under
    /// `<TypeName>.<bare>` in the function table — `find_function_by_name`
    /// misses it because the exact match fails and suffix-match is
    /// gated on `.` in the input.
    ///
    /// This helper takes the receiver's runtime TypeId, walks the
    /// type table to recover the type name, then looks up
    /// `<type_name>.<bare>` exactly.  Returns None when the TypeId
    /// has no descriptor in this module (stdlib types are NOT
    /// imported into user modules — only the methods they declare).
    pub fn find_method_by_receiver_type(
        &self,
        receiver_type: TypeId,
        bare_method: &str,
    ) -> Option<FunctionId> {
        // Built-in TypeIds (Range/List/Map/Set/Maybe/Result/...) are
        // allocated as constants in `crate::types::TypeId::*` — they
        // don't have a matching `TypeDescriptor` in `self.types` unless
        // the archive explicitly registered one. Resolve their canonical
        // name first so the qualified-lookup fast-path can succeed
        // before the descriptor walk falls through. Pairs with task
        // #134: stdlib types whose method-table lives outside the
        // Tier-0 inline dispatcher (e.g. `Range.collect` → Iterator's
        // default `collect` body in core.base.iterator) need this
        // name-resolved entrypoint.
        let builtin_name: Option<&'static str> = match receiver_type {
            TypeId::RANGE => Some("Range"),
            TypeId::LIST => Some("List"),
            TypeId::MAP => Some("Map"),
            TypeId::SET => Some("Set"),
            TypeId::DEQUE => Some("Deque"),
            TypeId::MAYBE => Some("Maybe"),
            TypeId::RESULT => Some("Result"),
            TypeId::HEAP => Some("Heap"),
            TypeId::SHARED => Some("Shared"),
            TypeId::TEXT => Some("Text"),
            TypeId::CHANNEL => Some("Channel"),
            _ => None,
        };
        if let Some(name) = builtin_name {
            let qualified = format!("{}.{}", name, bare_method);
            if let Some(id) = self.find_function_by_name(&qualified) {
                return Some(id);
            }
        }
        let descriptor = match self.types.iter().find(|t| t.id == receiver_type) {
            Some(d) => d,
            None => {
                // No descriptor for the receiver — built-in or otherwise.
                // For built-in receivers we already attempted the name
                // path above; for non-builtin unknown TypeIds there's
                // nothing more to do.
                return None;
            }
        };
        if let Some(type_name) = self.get_string(descriptor.name) {
            let qualified = format!("{}.{}", type_name, bare_method);
            if let Some(id) = self.find_function_by_name(&qualified) {
                return Some(id);
            }
        }
        // Protocol-default-method fallback. When `<Type>.<method>`
        // doesn't exist (e.g. `Range.collect` — `collect` is a
        // default method on `Iterator`, not overridden by Range), walk
        // the protocols this type implements and try
        // `<ProtocolName>.<method>` for each. ProtocolId is an index
        // into the same type table (protocols are types), so the name
        // lookup uses the same shape as the receiver-type path.
        let bare_suffix = format!(".{}", bare_method);
        for protocol_impl in descriptor.protocols.iter() {
            // Prefer the impl-block override: scan its method-id list
            // for a function whose name ends with `.<bare_method>`.
            for raw in protocol_impl.methods.iter().copied() {
                if let Some(desc) = self.functions.get(raw as usize)
                    && let Some(fname) = self.get_string(desc.name)
                    && (fname == bare_method || fname.ends_with(&bare_suffix))
                {
                    return Some(FunctionId(raw));
                }
            }
            // Fall back to the protocol's default method (declared on
            // the protocol type itself, not the impl block).
            let proto_name = self
                .types
                .iter()
                .find(|t| t.id.0 == protocol_impl.protocol.0)
                .and_then(|t| self.get_string(t.name));
            if let Some(name) = proto_name {
                let qualified = format!("{}.{}", name, bare_method);
                if let Some(id) = self.find_function_by_name(&qualified) {
                    return Some(id);
                }
            }
        }
        None
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
        let profile_flags = self.header.flags
            & (VbcFlags::NOT_INTERPRETABLE | VbcFlags::SYSTEMS_PROFILE | VbcFlags::EMBEDDED_TARGET);

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
                && sym_name == name
            {
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
    /// Reverse-lookup cache: `StringId → IndexMap position`. Lazily
    /// populated on first `get` call and invalidated on `intern`.
    /// Skipped from serialise to keep the wire format unchanged.
    /// Uses `OnceLock` (not `OnceCell`) so `Arc<VbcModule>` stays
    /// `Send + Sync` for the cross-thread async runtime path.
    #[serde(skip, default)]
    id_to_idx: std::sync::OnceLock<HashMap<StringId, usize>>,
}

impl StringTable {
    /// Creates a new empty string table.
    pub fn new() -> Self {
        Self {
            index: IndexMap::new(),
            next_offset: 0,
            id_to_idx: std::sync::OnceLock::new(),
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
        let new_idx = self.index.len() - 1;
        // Incremental cache update.  `IndexMap::insert` for a fresh
        // key APPENDS without shifting existing positions, so the
        // cached `(id → idx)` entries remain correct — we only need
        // to record the freshly-inserted (id, new_idx) pair.  Cold
        // start was paying for full rebuild on every intern (28K+
        // strings during finalize_module) before this fix.
        if let Some(idx) = self.id_to_idx.get_mut() {
            idx.insert(id, new_idx);
        }
        id
    }

    /// Gets a string by ID — O(1) amortised after first call.
    ///
    /// Pre-fix this was an O(N) linear scan of the IndexMap entries
    /// (the table is keyed by string, not by id). The cost compounded
    /// across the codegen + runtime hot paths (every variant-name
    /// resolution, every method-dispatch retry, every parameter-name
    /// extraction) and inflated cold-start by hundreds of milliseconds
    /// on archive loads. The reverse index is lazily populated and
    /// invalidated whenever `intern` shifts the IndexMap.
    pub fn get(&self, id: StringId) -> Option<&str> {
        let idx = *self.ensure_reverse_index().get(&id)?;
        self.index.get_index(idx).map(|(s, _)| s.as_str())
    }

    /// Lazy populator for the `StringId → IndexMap-position` reverse
    /// cache.  Built on first reverse lookup and kept fresh by
    /// `intern` (which clears the cache because IndexMap insertion
    /// shifts existing positions).
    fn ensure_reverse_index(&self) -> &HashMap<StringId, usize> {
        self.id_to_idx.get_or_init(|| {
            self.index
                .iter()
                .enumerate()
                .map(|(i, (_, &sid))| (sid, i))
                .collect()
        })
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

    /// #87 — Intrinsic-name marker for inlinable stdlib constants and
    /// compiler-recognised special functions.  The most-loaded carrier
    /// in production is `__const_val_<N>`, used by
    /// `register_constant_with_value` to inline integer literal
    /// constants at every reference site instead of emitting a
    /// `Call`.  Without this archive-side field, stdlib `public const
    /// MAX_FOO: Int = 256;` declarations were registered as plain
    /// zero-arg functions in the precompile pass, and the inline
    /// marker was dropped when archive_ctx_loader reconstructed the
    /// `FunctionInfo` — every cross-module reference then resolved
    /// to a body-less zero-arg function and surfaced as
    /// `UndefinedVariable` (the Path-resolution path can't recover
    /// the literal value without the marker).
    ///
    /// `None` for ordinary functions; `Some(stringid)` resolves to
    /// the marker text via the module's string table.
    #[serde(default)]
    pub intrinsic_name: Option<StringId>,

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

    /// Is this function annotated with `@device(gpu)` and therefore
    /// belongs to the GPU-only compute partition?
    ///
    /// Architectural invariant: a `is_gpu_only = true` function is
    /// lowered EXCLUSIVELY through the MLIR pipeline (`linalg`/`tensor`/
    /// `gpu` dialects) — the LLVM CPU pipeline emits only an `extern`
    /// stub for it.  Conversely, a `is_gpu_only = false` function is
    /// lowered EXCLUSIVELY through the LLVM CPU pipeline.  The two
    /// partitions are mutually exclusive: no compute function is
    /// lowered through both pipelines (closes the dual-lowering
    /// architectural violation surfaced by the codegen audit).
    ///
    /// Populated by VBC codegen by reading `@device(gpu)` attributes
    /// from the source AST.  See `crates/verum_compiler/src/pipeline/
    /// gpu_detect.rs` for the corresponding module-level scan.
    #[serde(default)]
    pub is_gpu_only: bool,

    /// `true` when this function descriptor represents a `const` (or
    /// `static`) declaration converted to a zero-arg function during
    /// codegen.  Const-as-function is purely a storage strategy — at
    /// the typechecker level the user sees a *value* of type
    /// `return_type`, not a callable.  Without this flag the
    /// archive-driven typechecker path can't tell a const from a
    /// genuine zero-arg fn (`fn random_seed() -> Int { ... }`) and
    /// either rejects `let x = SSO_CAPACITY` (registered as fn) or
    /// silently breaks `let f = random_seed` (registered as value).
    ///
    /// Inlinable consts (literal integer initialisers) ALSO carry
    /// `intrinsic_name = Some("__const_val_<N>")` for zero-cost use-
    /// site inlining; the two markers are independent — `is_const`
    /// covers ALL consts, `intrinsic_name` only those whose value
    /// fits the inline path.
    #[serde(default)]
    pub is_const: bool,

    /// AOT-only register→owner-type hints for method dispatch.
    ///
    /// Populated by VBC codegen ONLY where the receiver's static type is
    /// known but not recoverable from the bytecode alone (e.g. the for-loop
    /// `__for_iter` temp holding a custom iterator). The AOT `reg_types`
    /// pre-pass consumes these to type the register so owner-qualified
    /// method resolution succeeds; the interpreter ignores them entirely
    /// (it dispatches on the runtime object header). Empty for functions
    /// with no such hints. Serialized only at VBC format minor >= 2.
    #[serde(default)]
    pub register_type_hints: Vec<RegisterTypeHint>,

    /// Source-level rendered return-type name carried VERBATIM across the
    /// precompile → archive → load round-trip (`"Ordering"`,
    /// `"Maybe<Char>"`, `"Result<(BigInt, BigInt), BigIntError>"`).
    ///
    /// Carried-fact contract (RETNAME-CARRY-1): the loader-side
    /// re-derivation `type_ref_simple_name(&return_type)` is LOSSY twice
    /// over — (a) a source type that wasn't in `type_name_to_id` at its
    /// module's bake moment lowers to the `TypeId::PTR` carrier and
    /// re-derives as `"USize"` (integer-shaped!), so every downstream
    /// `let x = f();` records an integer binding and method dispatch
    /// emits `Int.<method>` for a record receiver; (b) `Instantiated`
    /// refs re-derive base-only (`"Maybe"`, args dropped), so match-arm
    /// payload typing can't recover `Char` from `Maybe<Char>`.  This
    /// field is written straight from the AST-rendered
    /// `FunctionInfo.return_type_name` at bake and preferred by
    /// `archive_ctx_loader` over any re-derivation.  `None` for
    /// functions without a declared return type (legacy archives
    /// decode as `None` via `serde(default)` and keep the old path).
    #[serde(default)]
    pub return_type_name: Option<StringId>,
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

/// AOT-only register→owner-type hint (FUNC-REGISTRY-QUALIFICATION-1 phase 1).
///
/// Records that VBC codegen statically knew `register` holds a value of type
/// `type_name`, for a site where the bytecode alone doesn't recover it (the
/// for-loop custom-iterator temp). Consumed only by the AOT `reg_types` pass
/// to type the register; the interpreter never reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterTypeHint {
    /// Register index whose owner type is being hinted.
    pub register: u16,
    /// Owner type name (index into string table).
    pub type_name: StringId,
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
            is_gpu_only: false,
            intrinsic_name: None,
            is_const: false,
            register_type_hints: Vec::new(),
            return_type_name: None,
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
                    if i + 1 < instructions.len() =>
                {
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

// ============================================================================
// Precompiled-stdlib archive extensions (Phase 3 of #precompile-stdlib).
//
// Multi-variant function bodies, theorem table, framework provenance,
// and discharge receipts. Every type below is `#[serde(default)]`
// optional on `VbcModule`; an unmodified VBC module emitted by today's
// codegen has empty vectors here, so backward-compat with on-disk
// caches and registry artefacts is automatic.
// ============================================================================

/// Per-function variant table — sparse, only target-conditional
/// functions appear. The loader picks `variants[i]` whose
/// `cfg_key_id`-indexed [`crate::cfg_key::CfgKey`] matches the active
/// target triple's resolved key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionVariantSet {
    /// Which function this set applies to. Sparse — most functions
    /// have no entry in `VbcModule.function_variants` and use their
    /// `FunctionDescriptor.bytecode_offset/length` directly.
    pub function_id: FunctionId,
    /// One entry per `#[cfg(...)]` arm in the source. The loader scans
    /// in order and returns the first match; the precompiler emits
    /// them sorted from most-specific to least-specific so a
    /// `cfg(all(target_os = "macos", target_arch = "aarch64"))` arm
    /// shadows a plain `cfg(target_os = "macos")` arm.
    pub variants: SmallVec<[VbcVariant; 4]>,
}

/// One target-conditional bytecode region.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VbcVariant {
    /// Index into `VbcModule.cfg_keys`. The cfg-key table is
    /// content-deduplicated — many variants of different functions
    /// can share the same `cfg(target_os = "macos")` key by ID.
    pub cfg_key_id: u16,
    /// 16-bit pad keeps the struct 12 bytes aligned. Reserved for a
    /// future `linkage / weak / inline` flag set without breaking
    /// the on-disk layout.
    #[serde(default)]
    pub flags: u16,
    /// Variant body offset within `VbcModule.bytecode` (or within the
    /// archive's body region when read from `.vbca`).
    pub bytecode_offset: u32,
    /// Variant body length in bytes.
    pub bytecode_length: u32,
}

/// Theorem / axiom / lemma / corollary / tactic entry — the
/// proof-layer artefact preserved in VBC archives. The optimised
/// processed representation lives here: typed predicate body
/// (PropositionRef → FunctionId), per-backend translations, lifecycle
/// status, ATS-V annotations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremEntry {
    /// Local index — stable within this VBC module.
    pub id: TheoremId,

    /// Theorem name (qualified: `core.math.giry.giry_left_identity`).
    pub name: StringId,

    /// Module path the theorem is declared in — used for downstream
    /// audit output.
    pub module_path: StringId,

    /// Discriminator (theorem / lemma / corollary / axiom / tactic).
    pub kind: TheoremKind,

    /// Best-effort propositional text. Used as fallback when no
    /// per-backend rendering is available; foreign tools that lack a
    /// translation receive the text in a comment plus `Prop`.
    pub propositional_text: StringId,

    /// Per-backend rendered propositions (`coq` / `lean` / `agda` /
    /// `isabelle` / `dedukti`). Stored as already-rendered text so
    /// the cross-format export path is a HashMap lookup, not a
    /// re-render.
    #[serde(default)]
    pub per_backend_propositions: SmallVec<[(StringId, StringId); 5]>,

    /// Theorem parameters — name + per-backend type-text per
    /// parameter (`Theorem foo (n : Z) : ...` etc.).
    #[serde(default)]
    pub params: SmallVec<[TheoremParamEntry; 4]>,

    /// Generic type parameters (`<S: RichS>`-style). Emitted as
    /// implicit arguments preceding the value parameters.
    #[serde(default)]
    pub generics: SmallVec<[TheoremGenericEntry; 2]>,

    /// True iff the source carried a proof body. Statements without
    /// a proof body are treated as axioms (postulates) by foreign
    /// tools regardless of `kind`.
    pub has_proof: bool,

    /// `@framework(name, citation)` framework attribution. Empty when
    /// the theorem is a native Verum statement (not citing an external
    /// formal system).
    #[serde(default)]
    pub framework: Option<StringId>,
    /// `@framework(name, "citation")` citation text.
    #[serde(default)]
    pub framework_citation: Option<StringId>,

    /// ATS-V `Lifecycle` status marker — `[T]` Theorem / `[H]`
    /// Hypothesis / `[D]` Discharged / `[C]` Conjecture / `[P]`
    /// Pending / `[I]` Inert / `[✗]` Refuted. Encoded as a small
    /// enum so the round-trip is stable.
    #[serde(default)]
    pub lifecycle: TheoremLifecycle,

    /// Pointer to the predicate's compiled body (the same VBC function
    /// that `requires`/`ensures` predicates compile to). `None` for
    /// axioms without a runtime witness.
    #[serde(default)]
    pub proposition_body: Option<FunctionId>,

    /// Pointers into `VbcModule.discharge_receipts` for every
    /// successful kernel re-check / SMT discharge / Lean replay.
    /// Empty for `[H]` / `[C]` / `[P]` lifecycle entries.
    #[serde(default)]
    pub discharged_by: SmallVec<[DischargeRef; 2]>,
}

/// Identifier for a [`TheoremEntry`] within a single [`VbcModule`].
/// Kept distinct from [`FunctionId`] so the linker can dedup these
/// independently when merging multiple archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TheoremId(pub u32);

/// Theorem-shape discriminator — round-trips with stable string tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TheoremKind {
    Theorem,
    Lemma,
    Corollary,
    Axiom,
    Tactic,
}

/// Lifecycle status marker — mirrors the seven-symbol CVE alphabet
/// (`[T]` / `[H]` / `[D]` / `[C]` / `[P]` / `[I]` / `[✗]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TheoremLifecycle {
    /// `[T]` — discharged theorem, certificate available.
    Theorem,
    /// `[H]` — hypothesis pending discharge.
    #[default]
    Hypothesis,
    /// `[D]` — defined / construction-only (no proof obligation).
    Defined,
    /// `[C]` — conjecture, no obligation set up.
    Conjecture,
    /// `[P]` — pending obligation, work in progress.
    Pending,
    /// `[I]` — inert / archived; carries history but is not load-
    /// bearing for current proofs.
    Inert,
    /// `[✗]` — explicitly refuted (counterexample known).
    Refuted,
}

/// One theorem parameter binding: `(name: T)` shape with per-backend
/// `T` translations stored as already-rendered text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremParamEntry {
    pub name: StringId,
    /// Per-backend type-text. `(backend_id, rendered_type_text)`.
    /// Backend IDs match `per_backend_propositions` keys.
    pub per_backend_type: SmallVec<[(StringId, StringId); 4]>,
}

/// One generic type-parameter binding: `<S: Bound>` shape with
/// per-backend `Bound` translations. Bound is optional when the
/// generic carries no protocol constraint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheoremGenericEntry {
    pub name: StringId,
    /// Per-backend bound text. Empty when `<S>` has no `: Bound`.
    pub per_backend_bound: SmallVec<[(StringId, StringId); 2]>,
}

/// Reference into [`VbcModule::discharge_receipts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DischargeRef(pub u32);

/// One discharge receipt — describes which kernel rule + which backend
/// emitted a certificate for a theorem / refinement obligation. The
/// certificate body itself is content-addressed (`cert_hash`) and
/// stored externally in `~/.verum/cert-store/`; this entry is the
/// *contract*, not the body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DischargeReceipt {
    /// Pointer to the theorem this receipt discharges.
    pub theorem_id: TheoremId,
    /// Backend that produced the certificate (`smt`, `smt_lemma`,
    /// `lean`, `coq`, `agda`, `isabelle`, `dedukti`, …).
    pub backend: StringId,
    /// Backend version pin at discharge time. The kernel-recheck
    /// pipeline refuses to trust a receipt whose backend version
    /// drifted from the in-binary expectation.
    pub backend_version: StringId,
    /// Content hash of the certificate body — the cert-store key.
    /// Blake3-encoded as 32 bytes for stability across compiler
    /// versions.
    pub cert_hash: [u8; 32],
    /// Wall-clock timestamp at discharge (seconds since UNIX epoch).
    /// Audit logs use this; the trust contract does not.
    pub discharged_at_seconds: i64,
    /// Kernel rule the discharge invoked (`K-Refine-omega` /
    /// `K-Universe-Ascent` / etc.). Empty when the discharge is a
    /// pure backend roundtrip without kernel involvement.
    #[serde(default)]
    pub kernel_rule: StringId,
}

/// `@framework(name, citation)` and
/// `@framework_translate(src, tgt, citation)` provenance edges
/// captured into the archive at precompile time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrameworkProvenance {
    /// `@framework(name, citation)` markers — one entry per attribute.
    pub citations: Vec<FrameworkCitation>,
    /// `@framework_translate(src, tgt, citation)` bridge edges.
    pub translations: Vec<FrameworkTranslation>,
}

/// One `@framework(name, citation)` attribute occurrence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkCitation {
    pub framework_name: StringId,
    pub citation: StringId,
    /// Theorem / function this citation decorates.
    pub item_kind: ProvenanceItemKind,
    /// Local item ID — interpretation depends on `item_kind`.
    pub item_id: u32,
}

/// One `@framework_translate(src, tgt, citation)` bridge edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkTranslation {
    pub source_framework: StringId,
    pub target_framework: StringId,
    pub citation: StringId,
    pub item_kind: ProvenanceItemKind,
    pub item_id: u32,
}

/// Discriminator for the local item ID stored in
/// [`FrameworkCitation`] / [`FrameworkTranslation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceItemKind {
    /// `item_id` is a [`FunctionId`]'s u32 representation.
    Function,
    /// `item_id` is a [`TheoremId`]'s u32 representation.
    Theorem,
}

#[cfg(test)]
mod precompile_extension_tests {
    use super::*;
    use crate::cfg_key::{CfgKey, TargetArch, TargetOs};

    fn make_module() -> VbcModule {
        VbcModule::new("test".to_string())
    }

    #[test]
    fn defaults_are_empty() {
        let m = make_module();
        assert!(m.cfg_keys.is_empty());
        assert!(m.function_variants.is_empty());
        assert!(m.theorems.is_empty());
        assert!(m.discharge_receipts.is_empty());
        assert!(m.framework_provenance.citations.is_empty());
        assert!(m.framework_provenance.translations.is_empty());
    }

    /// T0103 LEG-2b: an unresolved band reference whose recorded name
    /// is a re-export spelling of a registry intrinsic fn-form gets a
    /// synthesized wrapper body bound in the band map; a runtime-raw
    /// bodyless decl (no registry entry) and a non-core name stay
    /// unresolved.
    #[test]
    fn band_wrapper_synthesis_binds_registry_intrinsics_only() {
        let mut m = make_module();
        let eq_band = XMOD_CALL_ID_BAND_BASE;
        let raw_band = XMOD_CALL_ID_BAND_BASE + 1;
        let user_band = XMOD_CALL_ID_BAND_BASE + 2;
        let eq_sid = m.intern_string("core.base.primitives.eq");
        let raw_sid = m.intern_string("core.intrinsics.runtime.io.__async_read_raw");
        let user_sid = m.intern_string("myapp.util.eq");
        m.external_function_names.push((FunctionId(eq_band), eq_sid));
        m.external_function_names.push((FunctionId(raw_band), raw_sid));
        m.external_function_names.push((FunctionId(user_band), user_sid));

        let unresolved = m.resolve_external_bands();
        assert_eq!(unresolved.len(), 3, "empty module resolves nothing");

        let bound = m.synthesize_intrinsic_band_wrappers();
        assert_eq!(bound, 1, "exactly the registry-covered core name binds");

        let wrapper = m
            .resolve_band_id(eq_band)
            .expect("eq band id bound to a wrapper");
        let desc = m.get_function(wrapper).expect("wrapper descriptor exists");
        assert_eq!(
            m.get_string(desc.name),
            Some("__band_wrapper$core$base$primitives$eq"),
            "wrapper registered under the dotless technical name — the \
             band map is its ONLY route"
        );
        assert!(desc.bytecode_length > 0, "wrapper has a real encoded body");
        assert_eq!(desc.register_count, 3, "eq wrapper: r0, r1 params + r2 dest");
        let instrs = desc.instructions.as_ref().expect("decoded body attached");
        assert!(
            matches!(instrs.last(), Some(crate::instruction::Instruction::Ret { .. })),
            "wrapper body ends in Ret"
        );

        // The dotless registration must be invisible to every by-name
        // scan (exact, `.suffix`, ranked) — a dotted registration would
        // let OTHER re-export spellings silently capture the wrapper.
        assert!(m.find_function_by_name("core.base.primitives.eq").is_none());
        assert!(m.find_function_by_name("primitives.eq").is_none());
        assert!(m.find_function_by_name("eq").is_none());
        assert!(
            m.resolve_function_by_name_ranked("core.other.primitives.eq")
                .is_none(),
            "a sibling re-export spelling must NOT ranked-resolve onto the wrapper"
        );

        // The runtime-raw decl and the user-spelled name must stay out.
        assert!(m.resolve_band_id(raw_band).is_none());
        assert!(m.resolve_band_id(user_band).is_none());

        // Idempotence: a second pass binds nothing new and appends no
        // duplicate wrapper functions.
        let fn_count = m.functions.len();
        assert_eq!(m.synthesize_intrinsic_band_wrappers(), 0);
        assert_eq!(m.functions.len(), fn_count);

        // Post-mono shape: `resolve_external_bands` recomputes (wipes)
        // the map; the mangled wrapper is invisible to ranked
        // resolution, so the eq entry reads unresolved again — the
        // re-synthesis pass must REBIND to the existing wrapper, never
        // append a duplicate.
        let unresolved2 = m.resolve_external_bands();
        assert_eq!(unresolved2.len(), 3, "map recompute cannot see the wrapper");
        assert_eq!(m.synthesize_intrinsic_band_wrappers(), 1);
        assert_eq!(m.functions.len(), fn_count, "wrapper reused, not duplicated");
        assert_eq!(m.resolve_band_id(eq_band), Some(wrapper));
    }

    #[test]
    fn universal_function_resolves_via_descriptor() {
        let mut m = make_module();
        // Synthesise a function whose body is at offset 100, length 32.
        let mut desc = FunctionDescriptor {
            id: FunctionId(0),
            name: m.intern_string("hello"),
            parent_type: None,
            type_params: SmallVec::new(),
            params: SmallVec::new(),
            return_type: crate::types::TypeRef::Concrete(crate::types::TypeId::UNIT),
            contexts: SmallVec::new(),
            properties: crate::types::PropertySet::default(),
            bytecode_offset: 100,
            bytecode_length: 32,
            locals_count: 0,
            register_count: 0,
            max_stack: 0,
            is_inline_candidate: false,
            is_generic: false,
            visibility: crate::types::Visibility::Public,
            is_generator: false,
            yield_type: None,
            suspend_point_count: 0,
            calling_convention: Default::default(),
            optimization_hints: Default::default(),
            instructions: None,
            func_id_base: 0,
            debug_variables: Vec::new(),
            is_test: false,
            is_gpu_only: false,
            intrinsic_name: None,
            is_const: false,
            register_type_hints: Vec::new(),
            return_type_name: None,
        };
        // Backwards-compat field is filled with the existing layout.
        let _ = &mut desc;
        m.functions.push(desc);

        let mut intern = |s: &str| m.strings.intern(s);
        // Mutable borrow conflict — re-do via a counter so the test
        // doesn't entangle borrows of `m` and `intern`.
        let cfg = CfgKey {
            os: Some(TargetOs::Darwin),
            arch: Some(TargetArch::Aarch64),
            ..CfgKey::default()
        };
        let _ = intern;
        let region = m.resolve_bytecode_region(FunctionId(0), &cfg);
        assert_eq!(region, Some((100, 32)));
    }

    #[test]
    fn variant_table_overrides_descriptor() {
        let mut m = make_module();
        // One placeholder function so the variant lookup has a target.
        let desc = FunctionDescriptor {
            id: FunctionId(0),
            name: m.intern_string("syscall_x"),
            parent_type: None,
            type_params: SmallVec::new(),
            params: SmallVec::new(),
            return_type: crate::types::TypeRef::Concrete(crate::types::TypeId::UNIT),
            contexts: SmallVec::new(),
            properties: crate::types::PropertySet::default(),
            bytecode_offset: 0,
            bytecode_length: 0,
            locals_count: 0,
            register_count: 0,
            max_stack: 0,
            is_inline_candidate: false,
            is_generic: false,
            visibility: crate::types::Visibility::Public,
            is_generator: false,
            yield_type: None,
            suspend_point_count: 0,
            calling_convention: Default::default(),
            optimization_hints: Default::default(),
            instructions: None,
            func_id_base: 0,
            debug_variables: Vec::new(),
            is_test: false,
            is_gpu_only: false,
            intrinsic_name: None,
            is_const: false,
            register_type_hints: Vec::new(),
            return_type_name: None,
        };
        m.functions.push(desc);

        // Two cfg keys: darwin and linux.
        m.cfg_keys.push(CfgKey {
            os: Some(TargetOs::Darwin),
            ..CfgKey::default()
        });
        m.cfg_keys.push(CfgKey {
            os: Some(TargetOs::Linux),
            ..CfgKey::default()
        });

        m.function_variants.push(FunctionVariantSet {
            function_id: FunctionId(0),
            variants: SmallVec::from_vec(vec![
                VbcVariant {
                    cfg_key_id: 0,
                    flags: 0,
                    bytecode_offset: 100,
                    bytecode_length: 16,
                },
                VbcVariant {
                    cfg_key_id: 1,
                    flags: 0,
                    bytecode_offset: 200,
                    bytecode_length: 24,
                },
            ]),
        });

        let darwin = CfgKey {
            os: Some(TargetOs::Darwin),
            arch: Some(TargetArch::Aarch64),
            ..CfgKey::default()
        };
        let linux = CfgKey {
            os: Some(TargetOs::Linux),
            arch: Some(TargetArch::X86_64),
            ..CfgKey::default()
        };
        let windows_active = CfgKey {
            os: Some(TargetOs::Windows),
            arch: Some(TargetArch::X86_64),
            ..CfgKey::default()
        };

        assert_eq!(
            m.resolve_bytecode_region(FunctionId(0), &darwin),
            Some((100, 16))
        );
        assert_eq!(
            m.resolve_bytecode_region(FunctionId(0), &linux),
            Some((200, 24))
        );
        // No matching variant for Windows — function is elided for
        // this target.
        assert_eq!(
            m.resolve_bytecode_region(FunctionId(0), &windows_active),
            None
        );
    }
}
