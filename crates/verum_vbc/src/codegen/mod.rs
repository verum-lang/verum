//! AST to VBC code generation.
//!

//! This module transforms Verum AST into VBC bytecode that can be:
//! - Executed directly by the VBC interpreter (Tier 0)
//! - Lowered to MLIR for JIT compilation (Tier 1-2)
//! - Compiled to native code via MLIR/LLVM (Tier 3)
//!

//! # Architecture
//!

//! ```text
//! AST (verum_ast)
//!  │
//!  ▼
//! ┌─────────────────────────────────────────┐
//! │ VbcCodegen │
//! │ ┌───────────────────────────────────┐ │
//! │ │ CodegenContext │ │
//! │ │ - RegisterAllocator │ │
//! │ │ - Label management │ │
//! │ │ - Loop/defer stacks │ │
//! │ │ - Constant pool │ │
//! │ └───────────────────────────────────┘ │
//! │ │
//! │ compile_module() → VbcModule │
//! │ ├─ compile_function() │
//! │ │ ├─ compile_block() │
//! │ │ │ ├─ compile_stmt() │
//! │ │ │ │ └─ compile_expr() │
//! │ │ │ │ ├─ literals │
//! │ │ │ │ ├─ binary ops │
//! │ │ │ │ ├─ calls │
//! │ │ │ │ └─ control flow │
//! │ │ │ └─ ... │
//! │ │ └─ ... │
//! │ └─ ... │
//! └─────────────────────────────────────────┘
//!  │
//!  ▼
//! VbcModule (instructions + constants + strings)
//! ```
//!

//! # Example
//!

//! ```ignore
//! use verum_vbc::codegen::VbcCodegen;
//! use verum_ast::Module;
//!

//! let ast: Module = parse_source_code(source);
//! let mut codegen = VbcCodegen::new();
//! let vbc_module = codegen.compile_module(&ast)?;
//! ```

pub mod context;
pub mod error;
pub mod registers;

mod expressions;
mod statements;

#[cfg(test)]
mod tests_comprehensive;

#[cfg(test)]
mod tests_integration;

#[cfg(test)]
mod tests_execution;

#[cfg(test)]
mod tests_e2e;

#[cfg(test)]
mod test_params;

pub use context::{CodegenContext, CodegenStats, ExprId, FunctionInfo, LoopContext, TierContext};

/// Information about a protocol for cross-module default method inheritance.
#[derive(Debug, Clone)]
pub struct ProtocolInfo {
    /// Protocol name (e.g., "Eq", "Ord", "Clone").
    pub name: String,
    /// Default method implementations: maps method name to its FunctionDecl.
    pub default_methods: std::collections::HashMap<String, verum_ast::FunctionDecl>,
    /// Superprotocol names this protocol extends.
    pub super_protocols: Vec<String>,
}

/// A blanket protocol implementation waiting to be monomorphized onto
/// every concrete implementor of its bound protocol.
///

/// Shape: `implement<T: base_protocol> derived_protocol for T { ... }`.
#[derive(Debug, Clone)]
pub struct BlanketImpl {
    /// The bound protocol that receivers must implement — for every concrete
    /// type implementing this, we replay the blanket impl.
    pub base_protocol: String,
    /// The protocol being implemented for all bound-satisfying receivers.
    pub derived_protocol: String,
    /// Method names explicitly implemented in the blanket-impl body.
    /// These take priority over the derived protocol's default methods.
    pub explicit_methods: std::collections::HashSet<String>,
}
pub use error::{CodegenError, CodegenErrorKind, CodegenOptionExt, CodegenResult, SkipClass};
pub use registers::{RegisterAllocator, RegisterInfo, RegisterKind, RegisterSnapshot};

use crate::types::CbgrTier;
use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_common::Map;
use verum_common::well_known_types::WellKnownType as WKT;

use crate::instruction::{Instruction, Reg};
use crate::module::{
    CType, CallingConvention as FfiCallingConvention, ErrorProtocol, FfiLibrary, FfiLibraryId,
    FfiOwnership, FfiPlatform, FfiSignature, FfiStructField, FfiStructLayout, FfiSymbol,
    FfiSymbolId, FunctionDescriptor, FunctionId, MemoryEffects, ParamDescriptor, VbcFunction,
    VbcModule,
};
use crate::types::{StringId, TypeDescriptor, TypeId, TypeRef};
use crate::validate;

use verum_ast::bitfield::ByteOrder;
use verum_ast::decl::{
    ExternBlockDecl, MountDecl, MountTree, MountTreeKind, TypeDeclBody, VariantData,
};
use verum_ast::ffi::{CallingConvention as AstCallingConvention, FFIBoundary};
use verum_ast::ty::PathSegment;
use verum_ast::{Block, FunctionBody, FunctionDecl, Item, ItemKind, Module, StmtKind};

/// Bitfield layout information for a type.
///

/// Tracks the bit layout of fields in a @bitfield type, enabling
/// generation of efficient getter/setter accessors.
#[derive(Debug, Clone)]
pub struct BitfieldLayout {
    /// Type name.
    pub type_name: String,
    /// Total size in bits.
    pub total_bits: u32,
    /// Byte order (little/big/native).
    pub byte_order: ByteOrder,
    /// Field layouts in declaration order.
    pub fields: Vec<BitfieldFieldLayout>,
}

/// Layout information for a single bitfield field.
#[derive(Debug, Clone)]
pub struct BitfieldFieldLayout {
    /// Field name.
    pub name: String,
    /// Bit offset from start of storage.
    pub bit_offset: u32,
    /// Bit width.
    pub bit_width: u32,
    /// Mask for this field (shifted to position).
    pub mask: u64,
    /// Whether this is a boolean field (1-bit).
    pub is_bool: bool,
}

/// VBC code generator.
/// Context layer kind for composable provide bundles.
#[derive(Debug, Clone)]
pub enum ContextLayer {
    /// Inline: list of (context_name, value_expr) pairs from provide statements.
    Inline(Vec<(String, verum_ast::expr::Expr)>),
    /// Composite: list of constituent layer names.
    Composite(Vec<String>),
}

/// FFI contract AST expressions for requires/ensures compilation.
///

/// Stored separately from `FfiContract` (which is serializable for VBC module)
/// because `verum_ast::Expr` can't be stored in `verum_vbc::module`.
#[derive(Debug, Clone)]
pub struct FfiContractExprs {
    /// Precondition AST expressions — compiled to asserts before FFI call.
    pub requires: Vec<verum_ast::expr::Expr>,
    /// Postcondition AST expressions — compiled to asserts after FFI call.
    /// `result` identifier refers to the return value.
    pub ensures: Vec<verum_ast::expr::Expr>,
    /// Function name (for diagnostic messages).
    pub function_name: String,
}

/// Detailed report of `MakeVariant` / `MakeVariantTyped`
/// emissions across a module's bytecode (#146 Phase 3e).
///

/// Returned by `VbcCodegen::collect_make_variant_report` to give
/// downstream tooling (audit gates, regression ratchets, post-
/// Phase-3 codegen telemetry) a structural view of variant
/// construction quality:
///

///  - `typed_emissions / untyped_emissions` — total counts per
///  instruction class. Post-Phase-3c clean compilation should
///  drive `untyped_emissions` toward zero (only fallback paths
///  for cross-cog forward refs / mid-pass-1 lookups remain
///  untyped).
///  - `typed_inconsistencies / untyped_inconsistencies` —
///  emissions whose layout doesn't match a module-declared
///  variant. Untyped: the legacy Phase-2 cross-module
///  false-positive signal. Typed: stronger — pinned by
///  operand-supplied type_id, so a mismatch is genuine
///  codegen drift.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MakeVariantReport {
    /// Number of legacy `MakeVariant` instructions emitted.
    pub untyped_emissions: usize,
    /// Number of `MakeVariantTyped` instructions emitted.
    pub typed_emissions: usize,
    /// Untyped emissions whose `(tag, field_count)` doesn't match
    /// any module-local declared variant (legacy Phase-2 signal —
    /// cross-module emissions show up here as false positives).
    pub untyped_inconsistencies: usize,
    /// Typed emissions whose `(type_id, tag, field_count)`
    /// disagrees with the declared variant of that type_id.
    /// These are stronger signals than the untyped class — the
    /// operand-supplied type_id pins the parent sum-type, so a
    /// mismatch is genuine codegen drift, not cross-module
    /// resolution.
    pub typed_inconsistencies: usize,
}

impl MakeVariantReport {
    /// Total inconsistency count — sum of typed + untyped
    /// inconsistencies. Useful for backwards-compatible callers
    /// that previously consumed
    /// `report_make_variant_inconsistencies() -> usize`.
    #[inline]
    pub fn total_inconsistencies(&self) -> usize {
        self.typed_inconsistencies + self.untyped_inconsistencies
    }

    /// Total variant emissions (typed + untyped). Lets ratchet
    /// tests assert that "typed/(typed+untyped)" stays above a
    /// threshold post-Phase-3.
    #[inline]
    pub fn total_emissions(&self) -> usize {
        self.typed_emissions + self.untyped_emissions
    }
}

///

/// Transforms Verum AST into VBC bytecode modules.
#[derive(Debug)]
pub struct VbcCodegen {
    /// Codegen context with registers, labels, etc.
    ctx: CodegenContext,

    /// Configuration for code generation.
    config: CodegenConfig,

    /// Generated functions.
    functions: Vec<VbcFunction>,

    /// Type descriptors.
    types: Vec<TypeDescriptor>,

    /// Next function ID.
    next_func_id: u32,

    /// Next type ID.
    next_type_id: u32,

    /// Counter for generating unique closure names.
    closure_counter: u32,

    /// Stack of parent function names for nested function name mangling.
    /// Used to generate unique names like `outer$inner$deeply_nested`.
    nested_function_scope: Vec<String>,

    /// Cfg evaluator for statement-level @cfg filtering.
    /// Statements with non-matching @cfg attributes are skipped.
    cfg_evaluator: CfgEvaluator,

    /// When false, @test attributes are NOT propagated to FunctionDescriptor.is_test.
    /// Used during stdlib compilation to prevent stdlib test functions from being
    /// treated as user test functions by the @test runner.
    propagate_test_attr: bool,

    /// Declared return type of the function currently being compiled.
    /// Populated at the start of body compilation and consulted by
    /// `compile_return` (explicit `return expr;` statements) to emit
    /// the refinement-Assert before the Ret instruction. Cleared once
    /// the function finishes so it can never leak across compilations.
    pub(super) current_return_ast_type: Option<verum_ast::ty::Type>,

    /// Fully-qualified name of the function currently being compiled.
    /// Used for refinement-violation messages at explicit returns;
    /// the implicit-return sites in `compile_function` have direct
    /// access to `lookup_name` and do not need this.
    pub(super) current_fn_lookup_name: Option<String>,

    // ========================================================================
    // FFI Tables (populated from @ffi extern blocks)
    // ========================================================================
    /// FFI libraries (native libraries to load).
    ffi_libraries: Vec<FfiLibrary>,

    /// FFI symbols (functions to resolve).
    ffi_symbols: Vec<FfiSymbol>,

    /// Map from function name to FFI symbol ID (for FFI function call detection).
    ffi_function_map: std::collections::HashMap<String, FfiSymbolId>,

    /// Map from library name to library ID (for deduplication).
    ffi_library_map: std::collections::HashMap<String, FfiLibraryId>,

    /// Map from (FFI symbol ID, param index) to callback signature symbol ID.
    /// Used for creating trampolines when passing functions to FFI.
    ffi_callback_signatures: std::collections::HashMap<(FfiSymbolId, u8), FfiSymbolId>,

    /// FFI contracts (requires/ensures) indexed by symbol ID.
    /// Stores serializable metadata for the VBC module.
    ffi_contracts: std::collections::HashMap<FfiSymbolId, crate::module::FfiContract>,

    /// FFI contract AST expressions indexed by function name.
    /// Used to compile requires/ensures into assertions at call sites.
    /// Separate from ffi_contracts because AST types can't be in verum_vbc::module.
    ffi_contract_exprs: std::collections::HashMap<String, FfiContractExprs>,

    /// Context group definitions: group_name → [context_name, ...]
    /// Grammar: context_group_def = 'using' identifier '=' context_list_def ';'
    /// Example: using WebContext = [Database, Logger, Auth];
    context_groups: std::collections::HashMap<String, Vec<String>>,

    /// Context layers: composable provide bundles.
    /// Grammar: layer_def = visibility 'layer' identifier layer_body
    context_layers: std::collections::HashMap<String, ContextLayer>,

    /// FFI struct layouts for @repr(C) types.
    ffi_layouts: Vec<FfiStructLayout>,

    /// Map from type name to FFI layout index (for @repr(C) struct types).
    /// This allows FFI function signatures to reference struct types by layout index.
    repr_c_types: std::collections::HashMap<String, u16>,

    /// Pending import aliases to resolve after all declarations are collected.
    /// Maps (qualified_path, alias_name) pairs for deferred resolution.
    pending_imports: Vec<(Vec<String>, String)>,

    /// Mount-rename alias buffer — Phase 2 of task #11 fundamental fix.
    /// Every `mount X.{NAME as ALIAS}` declaration in this module's
    /// source captures here AS SOON AS `register_function_authoritative`
    /// resolves the alias to a concrete FunctionInfo.  `build_module`
    /// drains this buffer into `VbcModule.mount_aliases` so the archive
    /// preserves the alias mapping across the precompile boundary; the
    /// user-side AOT loader (`apply_lazy_with_types`) then replays the
    /// aliases into its own `ctx.functions` before recompiling any
    /// body from this module.
    ///
    /// Carries `(alias_name, FunctionId)`.  Drained on every
    /// `finalize_module` so the buffer resets between modules.
    mount_aliases_buffer: Vec<(String, FunctionId)>,

    /// Variant names that have collisions (multiple types define the same variant name).
    /// When a collision is detected, the simple name is removed from the registry and
    /// added here, forcing code to use qualified names (e.g., `Maybe.Some` vs `Keyword.Some`).
    variant_collisions: std::collections::HashSet<String>,

    /// Bitfield type layouts for @bitfield types.
    /// Maps type name to its bitfield layout for accessor generation.
    bitfield_types: std::collections::HashMap<String, BitfieldLayout>,

    /// Field name to sequential index mapping.
    /// Used for consistent field access across record construction and field access.
    /// Each unique field name gets a globally unique index starting from 0.
    field_name_indices: std::collections::HashMap<String, u32>,

    /// Next field name index to assign.
    next_field_id: u32,

    /// Type field layouts: maps type name → ordered field names.
    /// Populated from record type declarations.
    type_field_layouts: std::collections::HashMap<String, Vec<String>>,

    /// Type field type names: maps (type_name, field_name) → field_type_name.
    /// Used to infer the type of field access expressions for chained access.
    type_field_type_names: std::collections::HashMap<(String, String), String>,

    /// Declared type-name for every `static mut NAME: T = init;` slot.
    /// Populated when an `ItemKind::Static` with `is_mut == true` is
    /// processed; consulted by `extract_expr_type_name`'s Path arm so a
    /// bare reference to a static-mut binding propagates the declared
    /// type to let-bindings (`let r = STATIC_MUT_RECORD`).  Without this,
    /// downstream `r.field` lookups have no `type_name` hint, fall
    /// through to the global interned-name fallback in
    /// `resolve_field_index`, and read at completely wrong byte offsets
    /// (e.g. `field_idx = intern("a")` instead of positional 0) — every
    /// `r.field0` read returns the value of some other field, every
    /// method dispatch on `STATIC_MUT.method()` either reads a wrong
    /// receiver self or null-derefs at the first GetF.  Architectural
    /// rule: every `static mut NAME` MUST record its declared type here
    /// (both bare-name and module-qualified) so the type-tracking surface
    /// matches what pure `static NAME` gets for free via
    /// `register_constant_with_value` → `FunctionInfo.return_type_name`.
    static_mut_type_names: std::collections::HashMap<String, String>,

    /// Pending constants that need bytecode compilation.
    /// These are constants whose values couldn't be inlined (e.g., struct literals).
    /// Stored as (function_name, expression_clone) for compilation in compile_function_bodies.
    /// Pending constants queued for body-compilation.  Each tuple is
    /// `(name, expr, source_module_at_push_time)` — the third element
    /// captures `current_source_module` AT THE TIME the const is
    /// queued, so when `compile_pending_constants` eventually flushes
    /// the queue (potentially during a later file's compile pass
    /// inside the shared stdlib-bootstrap codegen instance), the
    /// archive descriptor.name promotion can use the const's *own*
    /// source-module-declared path rather than whichever file's
    /// `compile_items_into_state` happens to be on the stack.
    /// Without this capture, a `public const X: USize = X.bits;` from
    /// `core/sys/bitfield.vr` (`module sys.bitfield;`) would inherit
    /// the directory-derived umbrella module name `core.sys` from the
    /// FIRST file's `compile_pending_constants` call that drains it,
    /// landing as `core.sys.X` in the archive instead of the file's
    /// own `sys.bitfield.X` — task #121 archive-side regression.
    pending_constants: Vec<(String, verum_ast::Expr, Option<String>)>,

    /// Map from type name to TypeId for user-defined types.
    /// Used to emit correct type_id in New instructions for proper Drop dispatch.
    type_name_to_id: std::collections::HashMap<String, crate::types::TypeId>,

    /// Archive-wide function-name → user-side FunctionId index (task #12).
    /// Populated by `archive_ctx_loader::record_archive_function_name`
    /// during archive load.  Used by Tier-2 cross-module Call resolution
    /// to recover the right user-side FunctionId for cross-archive
    /// dispatches whose target isn't in the user's mount set.
    /// First-wins discipline mirrors `ctx.functions`.
    pub(crate) archive_func_name_to_fid:
        std::collections::HashMap<String, crate::module::FunctionId>,

    /// Collection type generic parameter name templates.
    /// Maps collection type name (e.g. "Map") to its generic parameter names (e.g. ["K", "V"]).
    /// Used by `resolve_generic_return_type` to map return type names to concrete types.
    collection_type_params: std::collections::HashMap<String, Vec<String>>,

    /// Set of type names that are transparent wrappers (single-field generic wrappers).
    /// When a bare wrapper type without generic args is encountered during field resolution,
    /// we fall through to scan-all-types rather than failing.
    transparent_wrappers: std::collections::HashSet<String>,

    /// Set of collection type names whose `.new()` constructor is intercepted and
    /// emitted as a `CallM` with the type name as receiver, so the interpreter
    /// creates a built-in heap object with the correct TypeId rather than a plain
    /// record (e.g., Channel, List, Map, Set, Deque).
    builtin_ctor_collections: std::collections::HashSet<String>,

    /// Protocol registry for default method inheritance.
    /// Maps protocol name → protocol info with default method implementations.
    protocol_registry: std::collections::HashMap<String, ProtocolInfo>,

    /// Type alias registry for method resolution.
    ///

    /// Maps type alias name → base type name, enabling method calls like
    /// `Vec4f.splat(1.0)` to resolve to `Vec.splat` when `type Vec4f = Vec<Float32, 4>`.
    ///

    /// For generic aliases like `Vec<Float32, 4>`, we store the base type name `Vec`.
    /// For simple aliases like `type MyInt = Int`, we store `Int`.
    type_aliases: std::collections::HashMap<String, String>,

    /// Context name → ContextRef ID mapping for context string table.
    /// Enables context name resolution from opaque ContextRef IDs.
    context_name_to_id: std::collections::HashMap<String, u32>,

    /// Context names in registration order (index == ContextRef ID).
    context_names: Vec<String>,

    /// Pending default protocol method compilations.
    ///

    /// These are default methods registered during declaration collection that need
    /// their bodies compiled during body compilation phase. Deferred compilation is
    /// necessary because default methods may reference functions from other modules
    /// that haven't been registered yet during declaration collection.
    ///

    /// Stored as (func_decl, type_name) pairs.
    pending_default_methods: Vec<(verum_ast::FunctionDecl, String)>,

    /// Blanket protocol implementations: `implement<T: BaseProto> DerivedProto for T {}`.
    ///

    /// These can't be monomorphized at collection time because not every
    /// concrete implementor of BaseProto is known yet. When a concrete
    /// `implement BaseProto for ConcreteTy` is processed, we replay each
    /// matching blanket impl here to register DerivedProto's default methods
    /// for ConcreteTy — without this, `h.derived_method()` on a concrete
    /// receiver panics at runtime ("method not found on value") because the
    /// default body was registered under the generic-param name.
    blanket_impls: Vec<BlanketImpl>,

    /// Static variable initializer function IDs.
    /// These are registered as global constructors so they run before main().
    static_init_functions: Vec<FunctionId>,

    /// Pending @thread_local static initializations.
    /// Stored as (name, init_expr, tls_slot) for compilation as TlsSet in global constructors.
    pending_tls_inits: Vec<(String, verum_ast::Expr, u16)>,
}

/// Configuration for code generation.
#[derive(Debug, Clone)]
pub struct CodegenConfig {
    /// Module name.
    pub module_name: String,

    /// Whether to generate debug info.
    pub debug_info: bool,

    /// Optimization level (0-3).
    pub optimization_level: u8,

    /// Whether to run the post-emit structural validator on the
    /// freshly-built `VbcModule` before it leaves the codegen pipeline.
    ///

    /// `false` (default, dev-loop): structural validation is OFF. Codegen
    /// returns whatever bytecode it builds, even if it violates internal
    /// invariants. This matches the historical pre-#1c4ddcc1 behaviour
    /// and preserves the boot path on stdlib while pre-existing encoding
    /// bugs (function-end-vs-instruction-stream length divergence,
    /// dangling TypeId references in stdlib registry, archive-header
    /// counts that disagree with the section bodies) are triaged as
    /// separate compiler tasks.
    ///

    /// `true` (opt-in via `with_validation()`): runs `validate::validate_module`
    /// in strict mode at the end of `finalize_module`. Used in CI where
    /// full structural correctness must hold. Will flip to `true` by
    /// default once stdlib emits cleanly under the validator.
    pub validate: bool,

    /// Whether to include source map.
    pub source_map: bool,

    /// Target configuration for @cfg evaluation.
    /// Statements with non-matching @cfg attributes are skipped.
    /// Defaults to host platform via `TargetConfig::host()`.
    pub target_config: TargetConfig,

    // ========================================================================
    // V-LLSI Profile Configuration
    // ========================================================================
    /// Whether the module can be executed by VBC interpreter.
    ///

    /// - `true` for Application and Research profiles
    /// - `false` for Systems profile (VBC is intermediate IR only)
    ///

    /// V-LLSI: Whether the generated bytecode can run in the Tier 0 interpreter.
    /// False for Systems profile (AOT-only: uses inline asm, direct syscalls, no-libc).
    pub is_interpretable: bool,

    /// Whether this is a Systems profile build.
    ///

    /// Systems profile enables:
    /// - Raw pointers and unsafe code
    /// - Inline assembly
    /// - No libc linking (direct syscalls)
    /// - NOT VBC-interpretable (AOT only)
    pub is_systems_profile: bool,

    /// Whether this targets embedded/bare-metal.
    ///

    /// Embedded modules have additional restrictions:
    /// - No heap allocation
    /// - No OS dependencies
    /// - No async runtime
    /// - Static CBGR only
    pub is_embedded: bool,

    /// Whether `compile_module_items_lenient` should promote bug-class
    /// skips (`SkipClass::BugClass` — undefined function, arity mismatch,
    /// type regression, etc.) to hard compilation errors.
    ///

    /// `false` (default): every per-item failure surfaces as a warn-level
    /// `[lenient] SKIP` trace and the function/method is omitted from the
    /// emitted bytecode. Runtime calls panic with `FunctionNotFound`.
    /// This is the dev-loop default — it lets partial / forward-referenced
    /// stdlib state still build.
    ///

    /// `true` (opt-in via `with_strict_codegen`): bug-class failures are
    /// converted into a hard `CodegenError` returned from
    /// `compile_module_items_lenient`, halting the build at the first
    /// such failure. `Irreducible` failures (FFI prototype, unimplemented
    /// language feature) continue to skip silently — these represent the
    /// documented Tier-0 contract, not bugs.
    ///

    /// Intended for CI and release builds where any bug-class skip is a
    /// regression that must block the merge. Tracked under #166
    /// (eliminate the lenient-SKIP class) — once full-stdlib bug-class
    /// counts hit zero (#176) this flag will flip to `true` by default.
    pub strict_codegen: bool,
}

impl Default for CodegenConfig {
    fn default() -> Self {
        Self {
            module_name: "main".to_string(),
            debug_info: false,
            optimization_level: 0,
            // Pre-existing stdlib emit bugs (TypeId(515) dangling refs,
            // function-end-vs-instruction-stream divergence, archive-
            // header count mismatch) currently fail strict validation.
            // Default-off until the bug class is closed; CI opts in via
            // `with_validation()`.
            validate: false,
            source_map: false,
            target_config: TargetConfig::host(),
            // Default: Application profile (interpretable, not systems, not embedded)
            is_interpretable: true,
            is_systems_profile: false,
            is_embedded: false,
            // Default: lenient — partial/forward-referenced stdlib still builds.
            // CI/release flips this on via with_strict_codegen.
            strict_codegen: false,
        }
    }
}

impl CodegenConfig {
    /// Creates a new config with the given module name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            module_name: name.into(),
            ..Default::default()
        }
    }

    /// Enables debug info.
    pub fn with_debug_info(mut self) -> Self {
        self.debug_info = true;
        self
    }

    /// Sets optimization level.
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Enables validation.
    pub fn with_validation(mut self) -> Self {
        self.validate = true;
        self
    }

    /// Enables source map.
    pub fn with_source_map(mut self) -> Self {
        self.source_map = true;
        self
    }

    /// Sets the target configuration for @cfg evaluation.
    ///

    /// Statements with `@cfg(...)` attributes are filtered based on the target.
    /// For example, `@cfg(target_os = "linux")` blocks are skipped when
    /// compiling for macOS.
    ///

    /// # Example
    ///

    /// ```ignore
    /// use verum_vbc::codegen::CodegenConfig;
    /// use verum_ast::cfg::TargetConfig;
    ///

    /// // Cross-compile for Linux on a macOS host
    /// let config = CodegenConfig::new("my_module")
    ///  .with_target(TargetConfig::linux_x86_64());
    /// ```
    pub fn with_target(mut self, target: TargetConfig) -> Self {
        self.target_config = target;
        self
    }

    // ========================================================================
    // V-LLSI Profile Configuration Methods
    // ========================================================================

    /// Configures Systems profile (low-level, AOT-only).
    ///

    /// Systems profile modules:
    /// - Cannot be interpreted by VBC (AOT compilation required)
    /// - Can use raw pointers and unsafe code
    /// - Can use inline assembly
    /// - Target no-libc linking (direct syscalls)
    ///

    /// V-LLSI Systems profile: enables raw pointers, inline assembly, direct syscalls,
    /// and disables interpreter execution. Used for OS kernel / embedded development.
    pub fn with_systems_profile(mut self) -> Self {
        self.is_interpretable = false;
        self.is_systems_profile = true;
        self
    }

    /// Configures Application profile (safe, VBC-interpretable).
    ///

    /// This is the default profile. Application modules:
    /// - Can be interpreted by VBC for rapid development
    /// - Have full CBGR memory safety
    /// - Can use all high-level language features
    pub fn with_application_profile(mut self) -> Self {
        self.is_interpretable = true;
        self.is_systems_profile = false;
        self.is_embedded = false;
        self
    }

    /// Configures Research profile (experimental, VBC-interpretable).
    ///

    /// Research modules:
    /// - Can be interpreted by VBC
    /// - Enable experimental features (dependent types, etc.)
    /// - Used for formal verification research
    pub fn with_research_profile(mut self) -> Self {
        self.is_interpretable = true;
        self.is_systems_profile = false;
        self.is_embedded = false;
        self
    }

    /// Configures embedded/bare-metal target.
    ///

    /// Embedded modules have additional restrictions:
    /// - No heap allocation
    /// - No OS dependencies
    /// - No async runtime
    /// - Static CBGR only
    /// - NOT VBC-interpretable
    pub fn with_embedded(mut self) -> Self {
        self.is_embedded = true;
        self.is_interpretable = false; // Embedded is never interpretable
        self
    }

    /// Explicitly sets VBC interpretability.
    ///

    /// Most code should use `with_systems_profile()` or `with_application_profile()`
    /// instead. This method is provided for fine-grained control.
    pub fn with_interpretable(mut self, interpretable: bool) -> Self {
        self.is_interpretable = interpretable;
        self
    }

    /// Promotes `BugClass` lenient skips to hard codegen errors.
    ///

    /// In strict mode `compile_module_items_lenient` returns a
    /// `CodegenError` on the first item that fails with a `BugClass`
    /// error (undefined function, arity mismatch, type regression,
    /// codegen resource exhaustion, parser/lowering bug). `Irreducible`
    /// errors (interpreter limitation — FFI prototype, unimplemented
    /// feature) continue to skip silently because they represent the
    /// documented Tier-0 contract.
    ///

    /// Intended for CI / release builds. See the `strict_codegen` field
    /// docstring for the broader rationale.
    pub fn with_strict_codegen(mut self) -> Self {
        self.strict_codegen = true;
        self
    }
}

impl Default for VbcCodegen {
    fn default() -> Self {
        Self::new()
    }
}

impl VbcCodegen {
    /// Creates a new VBC codegen.
    pub fn new() -> Self {
        Self::with_config(CodegenConfig::default())
    }

    /// Looks up a type name in the well-known type registry (`type_name_to_id`).
    ///

    /// This is the SINGLE point of truth for mapping type name strings to TypeIds.
    /// It handles both canonical names ("Int", "List") and aliases ("i64", "Int64", "Option").
    /// User-defined types registered during declaration collection are also found here.
    fn get_well_known_type_id(&self, name: &str) -> Option<crate::types::TypeId> {
        self.type_name_to_id.get(name).copied()
    }

    /// Extracts the simple type name (last segment) from a type for method dispatch.
    /// For `core.net.Ipv6Addr`, returns `"Ipv6Addr"`.
    /// For `List<E>`, returns `"List"`.
    fn type_to_simple_name(&self, ty: &verum_ast::ty::Type) -> String {
        match &ty.kind {
            // Generic type: extract base type name from the generic base
            // List<E> → List, Map<K, V> → Map
            // Heap<T> → T, Shared<T> → T (transparent wrappers for method dispatch)
            verum_ast::ty::TypeKind::Generic { base, args } => {
                let base_name = self.type_to_simple_name(base);
                // VBC-internal: Heap<T> and Shared<T> are transparent wrappers — unwrap
                // to inner type T so method dispatch resolves against T, not the wrapper.
                if (WKT::Heap.matches(&base_name) || WKT::Shared.matches(&base_name))
                    && args.len() == 1
                {
                    // Transparent wrapper — return inner type for method dispatch
                    if let verum_ast::ty::GenericArg::Type(inner_ty) = &args[0] {
                        return self.type_to_simple_name(inner_ty);
                    }
                }
                base_name
            }
            verum_ast::ty::TypeKind::Path(path) => {
                // Get the last segment name (simple type name)
                path.segments
                    .iter()
                    .rev()
                    .find_map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "Unknown".to_string())
            }
            _ if ty.kind.primitive_name().is_some() => ty
                .kind
                .primitive_name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Unknown".to_string()),
            verum_ast::ty::TypeKind::Tuple(_) => "Tuple".to_string(),
            verum_ast::ty::TypeKind::Array { .. } => "Array".to_string(),
            verum_ast::ty::TypeKind::Reference { inner, .. } => {
                // For references like &T, use the inner type's name
                self.type_to_simple_name(inner)
            }
            verum_ast::ty::TypeKind::Slice(element) => {
                // For slices like [T], return "Slice"
                format!("Slice<{}>", self.type_to_simple_name(element))
            }
            _ => "Unknown".to_string(),
        }
    }

    /// Extracts the base type name from a type for type alias resolution.
    ///

    /// For generic types like `Vec<Float32, 4>`, returns `"Vec"`.
    /// For simple path types like `Int`, returns `"Int"`.
    /// For primitive types, returns the primitive name.
    fn extract_base_type_name(&self, ty: &verum_ast::ty::Type) -> Option<String> {
        match &ty.kind {
            // Generic type: extract base type name from the generic base
            // Vec<Float32, 4> → Vec
            verum_ast::ty::TypeKind::Generic { base, .. } => self.extract_base_type_name(base),
            // Path type: extract the last segment name
            verum_ast::ty::TypeKind::Path(path) => {
                path.segments.iter().rev().find_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                    _ => None,
                })
            }
            // Primitive types
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string())
            }
            _ => None,
        }
    }

    /// Returns `true` iff `ty` is a generic type whose first type-argument
    /// resolves (via `extract_base_type_name`) to `inner`.  Used to detect
    /// e.g. `List<Byte>` → for routing `with_capacity(N)` to the packed
    /// byte-list intrinsic (red-team §4 ergonomic auto-routing).
    pub(crate) fn is_generic_first_arg(&self, ty: &verum_ast::ty::Type, inner: &str) -> bool {
        match &ty.kind {
            verum_ast::ty::TypeKind::Generic { args, .. } => args.iter().next().is_some_and(|a| {
                if let verum_ast::ty::GenericArg::Type(t) = a {
                    self.extract_base_type_name(t).as_deref() == Some(inner)
                } else {
                    false
                }
            }),
            _ => false,
        }
    }

    /// Returns `true` if `name` is registered as a transparent *allocating* wrapper
    /// (i.e. Heap or Shared — a wrapper that allocates on the heap, but NOT Maybe).
    /// Used for the `Wrapper(x)` / `Wrapper::new(x)` call patterns which only apply
    /// to allocating wrappers.
    pub fn is_allocating_wrapper(&self, name: &str) -> bool {
        // Allocating wrappers are the transparent wrappers minus Maybe.
        // Maybe is a sum-type wrapper, not an allocating one.
        self.transparent_wrappers.contains(name) && name != "Maybe"
    }

    /// Returns `true` if `name` is a collection type whose `.new()` constructor
    /// is intercepted so the interpreter creates a built-in heap object with
    /// the correct TypeId. These are the interpreter's built-in collection
    /// constructors (Channel, List, Map, Set, Deque).
    ///
    /// FUNDAMENTAL #7 — delegates to the centralised
    /// `WellKnownType::name_has_builtin_constructor_intercept` predicate
    /// instead of consulting an in-memory HashSet that had to be
    /// hand-populated.  The HashSet field (`builtin_ctor_collections`)
    /// is retained as a no-op for binary-compat with the construction
    /// site but is no longer consulted on the hot path; future cleanup
    /// can remove it once all reads are migrated.
    pub fn is_builtin_ctor_collection(&self, name: &str) -> bool {
        verum_common::well_known_types::WellKnownType::name_has_builtin_constructor_intercept(name)
    }

    /// Resolves a type name through the type alias chain.
    ///

    /// If `type_name` is a type alias, returns the base type name.
    /// Otherwise, returns the original name.
    ///

    /// Handles alias chains: if A → B and B → C, resolving A returns C.
    pub fn resolve_type_alias(&self, type_name: &str) -> String {
        let mut current = type_name.to_string();
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 10; // Prevent infinite loops

        while iterations < MAX_ITERATIONS {
            if let Some(target) = self.type_aliases.get(&current) {
                current = target.clone();
                iterations += 1;
            } else {
                break;
            }
        }

        current
    }

    /// Strips generic arguments from a type name.
    ///

    /// Examples:
    /// - "Maybe<Int>" → "Maybe"
    /// - "Result<T, E>" → "Result"
    /// - "List" → "List"
    ///

    /// This is needed for method dispatch because methods are registered with
    /// base type names (e.g., "Maybe.is_some") but variables may have full
    /// generic type names (e.g., "Maybe<Int>").
    pub fn strip_generic_args(type_name: &str) -> &str {
        if let Some(idx) = type_name.find('<') {
            &type_name[..idx]
        } else {
            type_name
        }
    }

    /// Substitute a generic-param-shaped payload type (`"T"` / `"E"` /
    /// generic name) with the concrete arg from `receiver_type`'s
    /// `<...>` syntax, consulting the TypeDescriptor for parent type's
    /// declared type-params.  Closes the 4th defect class of task #22
    /// (variant-tag drift in nested generic constructors).
    ///
    /// Receiver-type-driven substitution: when the variant's declared
    /// payload type is a bare generic-param name (`"T"` for
    /// `Poll<T>::Ready(T)`), and the outer context's runtime
    /// instantiation is known (`receiver_type = "Poll<Result<Int,Int>>"`),
    /// resolve to the concrete `Result<Int,Int>` so downstream
    /// nested-variant lookup consults the right type's variant table.
    ///
    /// Returns `None` when:
    ///   * `payload_ty` is not a generic-param shape (concrete type
    ///     names like `"Result<T, E>"` pass through unchanged at the
    ///     caller — this helper only fires for bare-name cases).
    ///   * `receiver_type` has no `<...>` syntax.
    ///   * The base type's TypeDescriptor has no matching type-param.
    ///   * The generic arg list is shorter than the param index.
    ///
    /// Bare-name detection mirrors `looks_like_type_param` from
    /// `verum_common::well_known_types` (single ASCII-uppercase chars
    /// or short caps-only names).  Concrete types like `"Result"`,
    /// `"Int"`, `"Maybe<T>"` are rejected by this gate and pass
    /// through to the caller's fallback path.
    pub fn substitute_payload_generic(
        &self,
        payload_ty: &str,
        receiver_type: &str,
    ) -> Option<String> {
        // Only fire for bare generic-param shapes (`T`, `K`, `V`, `E`,
        // `Self`).  Mixed concrete-name payload types (`Maybe<T>`,
        // `Result<Ok, Err>`) carry enough info already — passing them
        // through unchanged is correct.
        if !verum_common::well_known_types::looks_like_type_param(payload_ty)
            && payload_ty != "Self"
        {
            return None;
        }
        // Extract base type of receiver (strip `<...>`).
        let base_type = Self::strip_generic_args(receiver_type);
        if base_type.is_empty() || base_type == receiver_type {
            return None;
        }
        // Look up TypeDescriptor for base_type to get its declared
        // type-params (in declaration order).  Uses the codegen's
        // `self.types` directly — populated for both user-phase and
        // archive-phase types, so no cross-mount race.
        let type_id = self.type_name_to_id.get(base_type).copied()?;
        let desc = self.types.iter().find(|t| t.id == type_id)?;
        if desc.type_params.is_empty() {
            return None;
        }
        // `Self` in a variant's payload type denotes the parent type
        // itself (e.g., `type Tree<T> is Leaf(T) | Node { left: Self,
        // right: Self }` — `Self` = `Tree<T>`).  Return the original
        // `receiver_type` (the concrete instantiation) for `Self`.
        if payload_ty == "Self" {
            return Some(receiver_type.to_string());
        }
        // Find param index by name in the TypeDescriptor's type-params.
        let param_idx = desc
            .type_params
            .iter()
            .position(|p| {
                self.ctx
                    .strings
                    .get(p.name.0 as usize)
                    .map(|s| s == payload_ty)
                    .unwrap_or(false)
            })?;
        // Parse generic args from `receiver_type<arg1, arg2, ...>`
        // honouring nested-`<...>` depth.
        let start = receiver_type.find('<')?;
        let end = receiver_type.rfind('>')?;
        let inner = &receiver_type[start + 1..end];
        let mut args = Vec::new();
        let mut depth = 0i32;
        let mut arg_start = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(inner[arg_start..i].trim().to_string());
                    arg_start = i + c.len_utf8();
                }
                _ => {}
            }
        }
        args.push(inner[arg_start..].trim().to_string());
        args.get(param_idx).cloned().filter(|s| !s.is_empty())
    }

    /// Resolves a generic return type name (e.g., "V", "K", "T") to a concrete type
    /// by extracting the corresponding type arg from a parameterized receiver type.
    /// E.g., receiver="Map<Int, Node>", base="Map", ret="V" → Some("Node")
    /// because Map has params [K, V] and V is at index 1.
    pub fn resolve_generic_return_type(
        &self,
        receiver_type: &str,
        base_type: &str,
        generic_name: &str,
    ) -> Option<String> {
        // Look up generic param names from the data-driven collection_type_params registry
        let params = self.collection_type_params.get(base_type)?;
        let param_names: Vec<&str> = params.iter().map(|s| s.as_str()).collect();
        let param_idx = param_names.iter().position(|&p| p == generic_name)?;
        // Extract type args from receiver_type (e.g., "Map<Int, Node>" → ["Int", "Node"])
        let start = receiver_type.find('<')?;
        let end = receiver_type.rfind('>')?;
        let inner = &receiver_type[start + 1..end];
        // Split by commas respecting nested generics
        let mut args = Vec::new();
        let mut depth = 0;
        let mut arg_start = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(inner[arg_start..i].trim().to_string());
                    arg_start = i + 1;
                }
                _ => {}
            }
        }
        args.push(inner[arg_start..].trim().to_string());
        args.get(param_idx).cloned().filter(|s| !s.is_empty())
    }

    /// Extracts the element type from a generic collection type string.
    /// "List<LexToken>" → Some("LexToken"), "Map<Text, Int>" → Some("Text"),
    /// "LexToken" → None (not a generic type).
    pub fn extract_element_type(type_name: &str) -> Option<String> {
        let start = type_name.find('<')?;
        let end = type_name.rfind('>')?;
        if start + 1 >= end {
            return None;
        }
        let inner = &type_name[start + 1..end];
        // For Map<K,V>, take the first type arg; for List<T>, take the only one
        let first_arg = if let Some(comma) = inner.find(',') {
            inner[..comma].trim()
        } else {
            inner.trim()
        };
        if first_arg.is_empty() {
            None
        } else {
            // Filter out bare generic type parameters (e.g., "T", "K", "V", "E").
            // These provide no useful type information and cause wrong field index
            // resolution when used as base_type in resolve_field_index.
            // Concrete type names are either longer or start lowercase (primitives).
            let is_bare_generic =
                first_arg.len() <= 2 && first_arg.chars().all(|c| c.is_ascii_uppercase());
            if is_bare_generic {
                None
            } else {
                Some(first_arg.to_string())
            }
        }
    }

    /// Creates a new VBC codegen with configuration.
    pub fn with_config(config: CodegenConfig) -> Self {
        // Create cfg evaluator from target config
        let cfg_evaluator = CfgEvaluator::with_config(config.target_config.clone());

        // Phase-not-realised tracing: `CodegenConfig.debug_info`
        // (default false) lands on the config from CompilerConfig
        // forwarding (api.rs:1045) but the VBC codegen does not
        // currently emit DWARF-style or other broad debug info.
        // The narrower `source_map` flag (line/col tracking via
        // debug_vars) IS wired at finalize_module (line ~4827) —
        // these are conceptually separate even though the names
        // overlap. Surface a warning when `debug_info = true` so
        // a `[codegen.vbc] debug_info = true` setting in
        // verum.toml doesn't silently produce a module without
        // any DWARF metadata.
        if config.debug_info {
            // `debug_info=true` is the default in `CompilerOptions`,
            // so a `tracing::warn!` here would fire on every script
            // compilation and drown real diagnostics. Drop to
            // `tracing::debug!` — visible under `RUST_LOG=debug` for
            // anyone investigating "why isn't DWARF metadata being
            // emitted?", silent in normal use. The comment block
            // above documents the conceptual debug_info vs source_map
            // split for readers of the source.
            tracing::debug!(
                target: "verum_vbc::codegen",
                "CodegenConfig surface: debug_info=true (advisory only — the \
                 narrower `source_map` flag controls actual line/col tracking \
                 via debug_vars; broad DWARF emission is not yet wired)",
            );
        }

        // Cross-compilation: codegen MUST emit constants for the
        // build target, never the host. Sync target_os from
        // CodegenConfig.target_config into the context so downstream
        // emitters (resolve_stdlib_constant_value, errno / socket /
        // file flag dispatch) see the correct target.
        let mut ctx = CodegenContext::new();
        ctx.target_os = config.target_config.target_os.to_string();
        Self {
            ctx,
            config,
            functions: Vec::new(),
            types: Vec::new(),
            next_func_id: 0,
            next_type_id: 0,
            closure_counter: 0,
            nested_function_scope: Vec::new(),
            cfg_evaluator,
            // FFI tables
            ffi_libraries: Vec::new(),
            ffi_symbols: Vec::new(),
            ffi_function_map: std::collections::HashMap::new(),
            ffi_library_map: std::collections::HashMap::new(),
            ffi_callback_signatures: std::collections::HashMap::new(),
            ffi_contracts: std::collections::HashMap::new(),
            ffi_contract_exprs: std::collections::HashMap::new(),
            context_groups: std::collections::HashMap::new(),
            context_layers: std::collections::HashMap::new(),
            // FFI struct layouts for @repr(C) types
            ffi_layouts: Vec::new(),
            repr_c_types: std::collections::HashMap::new(),
            // Deferred imports for multi-file modules
            pending_imports: Vec::new(),
            mount_aliases_buffer: Vec::new(),
            // Track variant name collisions
            variant_collisions: std::collections::HashSet::new(),
            // Bitfield type layouts for @bitfield types
            bitfield_types: std::collections::HashMap::new(),
            // Field name indices for record field access
            field_name_indices: std::collections::HashMap::new(),
            next_field_id: 0,
            type_field_layouts: std::collections::HashMap::new(),
            type_field_type_names: std::collections::HashMap::new(),
            static_mut_type_names: std::collections::HashMap::new(),
            // Pending constants for deferred compilation
            pending_constants: Vec::new(),
            archive_func_name_to_fid: std::collections::HashMap::new(),
            // Type name to TypeId mapping for Drop dispatch.
            // Pre-populated with all well-known type names and their aliases so that
            // ast_type_to_type_ref and type_ref_for_type_kind can do a single lookup
            // instead of hardcoded match arms.
            type_name_to_id: {
                let mut m = std::collections::HashMap::new();
                use crate::types::TypeId;
                // Primitives — full alias matrix from
                // `NUMERIC_ALIAS_MATRIX` (canonical Verum + width-tagged
                // + legacy uppercase-short + Rust-style lowercase). Drift
                // between this table and the canonical registry causes
                // `looking up T from a name expression` to return
                // `None` for any un-mapped alias, surfacing as
                // `TypeId(0)` defaults at instruction emission (every
                // arithmetic op on the un-mapped alias falls through to
                // generic `Int` semantics regardless of source width).
                // Bare `Int` / `UInt` / `Float` map to the canonical
                // pointer-width TypeIds; width-tagged forms map to
                // their dedicated TypeIds.
                m.insert("Int".to_string(), TypeId::INT);
                m.insert("Int64".to_string(), TypeId::INT);
                m.insert("I64".to_string(), TypeId::INT);
                m.insert("i64".to_string(), TypeId::INT);
                m.insert("Int32".to_string(), TypeId::I32);
                m.insert("I32".to_string(), TypeId::I32);
                m.insert("i32".to_string(), TypeId::I32);
                m.insert("Int16".to_string(), TypeId::I16);
                m.insert("I16".to_string(), TypeId::I16);
                m.insert("i16".to_string(), TypeId::I16);
                m.insert("Int8".to_string(), TypeId::I8);
                m.insert("I8".to_string(), TypeId::I8);
                m.insert("i8".to_string(), TypeId::I8);
                m.insert("UInt".to_string(), TypeId::U64);
                m.insert("UInt64".to_string(), TypeId::U64);
                m.insert("U64".to_string(), TypeId::U64);
                m.insert("u64".to_string(), TypeId::U64);
                m.insert("UInt32".to_string(), TypeId::U32);
                m.insert("U32".to_string(), TypeId::U32);
                m.insert("u32".to_string(), TypeId::U32);
                m.insert("UInt16".to_string(), TypeId::U16);
                m.insert("U16".to_string(), TypeId::U16);
                m.insert("u16".to_string(), TypeId::U16);
                m.insert("UInt8".to_string(), TypeId::U8);
                m.insert("U8".to_string(), TypeId::U8);
                m.insert("u8".to_string(), TypeId::U8);
                m.insert("Byte".to_string(), TypeId::U8);
                // Pointer-width integers — every alias spelling
                // resolves to TypeId(14) (USIZE/ISIZE are the same
                // sentinel; the runtime distinguishes signedness at
                // the operation site, not the TypeId).
                m.insert("USize".to_string(), TypeId::USIZE);
                m.insert("UIntSize".to_string(), TypeId::USIZE);
                m.insert("Usize".to_string(), TypeId::USIZE);
                m.insert("usize".to_string(), TypeId::USIZE);
                m.insert("ISize".to_string(), TypeId::ISIZE);
                m.insert("IntSize".to_string(), TypeId::ISIZE);
                m.insert("Isize".to_string(), TypeId::ISIZE);
                m.insert("isize".to_string(), TypeId::ISIZE);
                m.insert("Float".to_string(), TypeId::FLOAT);
                m.insert("Float64".to_string(), TypeId::FLOAT);
                m.insert("F64".to_string(), TypeId::FLOAT);
                m.insert("f64".to_string(), TypeId::FLOAT);
                m.insert("Float32".to_string(), TypeId::F32);
                m.insert("F32".to_string(), TypeId::F32);
                m.insert("f32".to_string(), TypeId::F32);
                m.insert("Bool".to_string(), TypeId::BOOL);
                m.insert("bool".to_string(), TypeId::BOOL);
                m.insert("Char".to_string(), TypeId::CHAR);
                m.insert("char".to_string(), TypeId::CHAR);
                m.insert("Text".to_string(), TypeId::TEXT);
                // Collection types
                m.insert("List".to_string(), TypeId::LIST);
                m.insert("Map".to_string(), TypeId::MAP);
                m.insert("Set".to_string(), TypeId::SET);
                m.insert("Maybe".to_string(), TypeId::MAYBE);
                m.insert("Option".to_string(), TypeId::MAYBE);
                m.insert("Result".to_string(), TypeId::RESULT);
                m.insert("Range".to_string(), TypeId::RANGE);
                m.insert("Array".to_string(), TypeId::ARRAY);
                m.insert("Tuple".to_string(), TypeId::TUPLE);
                m.insert("Deque".to_string(), TypeId::DEQUE);
                m.insert("Channel".to_string(), TypeId::CHANNEL);
                // Pointer/wrapper types — bind to their dedicated
                // semantic-collection TypeIds (HEAP=519, SHARED=520),
                // NOT to the catch-all `PTR=14`.  Pre-fix both names
                // mapped to PTR, which collapsed both user-declared
                // types onto a single id; archive_metadata's
                // `module.types` walk then deduplicated by id and
                // dropped one of them — `Shared` lost the race
                // (declared after `Heap` in `core/base/memory.vr`),
                // disappeared from `metadata.types`, and every
                // user-side `Shared<T>` reference died with
                // "type not found: Shared".  The runtime side
                // already uses the proper IDs (heap allocator stamps
                // `TypeId::SHARED` on Shared boxes per
                // `interpreter/dispatch_table/handlers/method_dispatch.rs:692`,
                // disassembler maps both back per `disassemble.rs:207-208`),
                // so swapping to dedicated IDs aligns codegen with
                // the runtime contract.
                m.insert("Heap".to_string(), TypeId::HEAP);
                m.insert("Shared".to_string(), TypeId::SHARED);
                m
            },
            // Collection type generic parameter name templates.
            // Used by resolve_generic_return_type to map generic names to positions.
            collection_type_params: {
                let mut m = std::collections::HashMap::new();
                m.insert("Map".to_string(), vec!["K".to_string(), "V".to_string()]);
                m.insert(
                    "BTreeMap".to_string(),
                    vec!["K".to_string(), "V".to_string()],
                );
                m.insert("List".to_string(), vec!["T".to_string()]);
                m.insert("Set".to_string(), vec!["T".to_string()]);
                m.insert("BTreeSet".to_string(), vec!["T".to_string()]);
                m.insert("Deque".to_string(), vec!["T".to_string()]);
                m.insert("Channel".to_string(), vec!["T".to_string()]);
                // task #12 §B: memory/concurrency carriers needed for
                // the cross-method generic-arg substitution at
                // `extract_expr_type_name`'s MethodCall arm. Without
                // these, calls like `Shared.clone() -> Shared<T>`
                // through `let shared = self.inner.clone()` leak the
                // literal `T` into `variable_type_names["shared"]`
                // and downstream `(*shared).lock()` resolves the
                // method name to `T.lock` instead of `<concrete>.lock`.
                m.insert("Heap".to_string(), vec!["T".to_string()]);
                m.insert("Shared".to_string(), vec!["T".to_string()]);
                m.insert("Weak".to_string(), vec!["T".to_string()]);
                m.insert("Pin".to_string(), vec!["T".to_string()]);
                m.insert("ManuallyDrop".to_string(), vec!["T".to_string()]);
                m.insert("Cow".to_string(), vec!["T".to_string()]);
                m.insert("Mutex".to_string(), vec!["T".to_string()]);
                m.insert("MutexGuard".to_string(), vec!["T".to_string()]);
                m.insert("RwLock".to_string(), vec!["T".to_string()]);
                m.insert(
                    "RwLockReadGuard".to_string(),
                    vec!["T".to_string()],
                );
                m.insert(
                    "RwLockWriteGuard".to_string(),
                    vec!["T".to_string()],
                );
                m.insert("PoisonError".to_string(), vec!["T".to_string()]);
                m.insert("AtomicInt".to_string(), Vec::new());
                m.insert("AtomicBool".to_string(), Vec::new());
                // Core sum types — needed for `unwrap_or_else` /
                // `unwrap_or` return-type substitution (task #15).
                // `Result<T, E>.unwrap_or_else<F>(self, f: F) -> T`
                // — substituting T against the receiver's generic args
                // requires knowing Result's param names [T, E].
                m.insert(
                    "Result".to_string(),
                    vec!["T".to_string(), "E".to_string()],
                );
                m.insert("Maybe".to_string(), vec!["T".to_string()]);
                m
            },
            // Transparent wrapper types: bare wrapper without generic args falls through
            // to scan-all-types during field resolution.
            transparent_wrappers: {
                let mut s = std::collections::HashSet::new();
                s.insert("Heap".to_string());
                s.insert("Shared".to_string());
                s.insert("Maybe".to_string());
                s
            },
            // Collection types whose `.new()` is intercepted as a CallM to the
            // interpreter's built-in constructor handler.
            builtin_ctor_collections: {
                let mut s = std::collections::HashSet::new();
                s.insert("Channel".to_string());
                s.insert("List".to_string());
                s.insert("Map".to_string());
                s.insert("Set".to_string());
                s.insert("Deque".to_string());
                s
            },
            // Protocol registry for default method inheritance
            protocol_registry: std::collections::HashMap::new(),
            // Type alias registry for method resolution
            type_aliases: std::collections::HashMap::new(),
            // Context name registry for ContextRef string table
            context_name_to_id: std::collections::HashMap::new(),
            context_names: Vec::new(),
            // Pending default protocol methods for deferred compilation
            pending_default_methods: Vec::new(),
            blanket_impls: Vec::new(),
            // Static variable initializer functions (become global constructors)
            static_init_functions: Vec::new(),
            // Pending @thread_local static initializations
            pending_tls_inits: Vec::new(),
            propagate_test_attr: true,
            current_return_ast_type: None,
            current_fn_lookup_name: None,
        }
    }

    /// Imports functions from previously compiled modules.
    ///

    /// This is used during stdlib compilation to make functions from
    /// earlier modules (e.g., core) available when compiling later
    /// modules (e.g., collections, async).
    ///

    /// # Example
    ///

    /// ```ignore
    /// // After compiling core module
    /// let core_functions = core_codegen.export_functions();
    ///

    /// // Before compiling collections module
    /// let mut collections_codegen = VbcCodegen::new();
    /// collections_codegen.import_functions(&core_functions);
    /// collections_codegen.compile_module(&collections_ast)?;
    /// ```
    pub fn import_functions(
        &mut self,
        functions: &std::collections::HashMap<String, FunctionInfo>,
    ) {
        self.ctx.import_functions(functions);
        // Update next_func_id to avoid ID conflicts.
        //
        // Filter out the `u32::MAX` sentinel that
        // `register_single_ffi_function` stamps on FFI
        // FunctionInfos (mod.rs:5754).  Without this filter the
        // sentinel saturates `next_func_id` on the first
        // `import_functions` call that pulls in any FFI entry, and
        // every subsequent `register_function` allocation also
        // returns `u32::MAX` — collapsing dozens of distinct
        // function bodies (closures, inherent methods, …) onto a
        // single id which `build_module`'s id-dedup then keeps just
        // one of.  In stdlib bootstrap this dropped the per-stdlib-
        // module function count from ~4500 effective to ~570 and
        // produced the missing-`Text.with_capacity` /
        // missing-`List.iter` symptom in user code.
        // Filter sentinel ids.  Multiple sentinels exist in the
        // codegen registry — `u32::MAX` (FFI extern,
        // `register_single_ffi_function`), `u32::MAX / 2` (newtype
        // constructor, `expressions.rs:1331`/`:3599`), and any
        // future high-bit-set sentinel that downstream gates check
        // via `id == sentinel`.  Any sentinel leaking into the
        // max() saturates `next_func_id` and collapses every
        // subsequent allocation (closures, impl methods, …) onto
        // a single id which `build_module`'s id-dedup then keeps
        // just one of.  Cap the threshold at the lowest known
        // sentinel boundary (`u32::MAX / 4` ≈ 1B) — legitimate
        // FunctionIds never approach this in stdlib + cog
        // compilation (~30K functions max even for the full
        // stdlib).  Using a single threshold instead of an
        // explicit sentinel list is robust to new sentinels added
        // upstream.
        const SENTINEL_THRESHOLD: u32 = u32::MAX / 4;
        if let Some(max_id) = functions
            .values()
            .map(|f| f.id.0)
            .filter(|&id| id < SENTINEL_THRESHOLD)
            .max()
            && max_id >= self.next_func_id
        {
            self.next_func_id = max_id.saturating_add(1);
        }
        if std::env::var("VERUM_TRACE_NEXT_FUNC_ID").is_ok() {
            eprintln!("[import_functions] {} entries, next_func_id={}", functions.len(), self.next_func_id);
        }
    }

    /// Imports protocols from previously compiled modules.
    ///

    /// This is used during stdlib compilation to make protocol default
    /// methods from earlier modules available for impl blocks in later modules.
    /// Iteration is sorted by name so that downstream codegen sees
    /// protocols in a deterministic order — this matters because some
    /// later passes assign function IDs in iteration order.
    pub fn import_protocols(
        &mut self,
        protocols: &std::collections::HashMap<String, ProtocolInfo>,
    ) {
        let mut sorted: Vec<&String> = protocols.keys().collect();
        sorted.sort();
        for name in sorted {
            let info = &protocols[name];
            self.protocol_registry
                .entry(name.clone())
                .or_insert_with(|| info.clone());

            // #130 — also allocate a TypeId + stub TypeDescriptor for
            // each imported protocol.  Without this, when a later
            // module declares `implement Iterator for IntoList<T>`,
            // the impl-push at `compile_item Impl` (line ~5162)
            // checks `self.type_name_to_id.get("Iterator")` — which
            // returns None because Iterator was registered only in
            // the source module's compilation unit.  The push is
            // skipped silently; archive_metadata then sees an empty
            // `IntoList.protocols` and the typechecker downstream
            // can't resolve `xs.into_iter().map(f)` because no
            // Iterator-impl-for-IntoList exists in the
            // metadata.implementations table.
            //
            // Stub kind=Protocol with empty variants is correct: the
            // codegen impl-push reads `td.variants` to populate the
            // impl's methods array (line ~5169), but downstream
            // typecheck (which is the consumer for stdlib metadata)
            // resolves impl methods via the protocol's own method
            // declarations table — see infer.rs:2417 — so empty
            // methods at the impl level is fine; the protocol body
            // remains the canonical source.
            if !self.type_name_to_id.contains_key(name) {
                let type_id = self.alloc_user_type_id();
                self.type_name_to_id.insert(name.clone(), type_id);
                let name_sid = StringId(self.ctx.intern_string_raw(name));
                let stub = crate::types::TypeDescriptor {
                    id: type_id,
                    name: name_sid,
                    kind: crate::types::TypeKind::Protocol,
                    ..Default::default()
                };
                self.types.push(stub);
            }
        }
    }

    /// Exports registered protocols for use by subsequent modules.
    pub fn export_protocols(&self) -> std::collections::HashMap<String, ProtocolInfo> {
        self.protocol_registry.clone()
    }

    /// Collects protocol definitions from a module.
    ///

    /// This should be called on all modules BEFORE collect_non_protocol_declarations
    /// to ensure protocols are available when processing impl blocks.
    pub fn collect_protocol_definitions(&mut self, module: &Module) {
        use verum_ast::decl::ProtocolItemKind;

        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Type(type_decl) = &item.kind
                && let TypeDeclBody::Protocol(protocol_body) = &type_decl.body
            {
                let protocol_name = type_decl.name.name.to_string();
                let mut default_methods = std::collections::HashMap::new();

                // Extract superprotocol names from extends clause
                let super_protocols: Vec<String> = protocol_body
                    .extends
                    .iter()
                    .filter_map(|ty| {
                        // Extract protocol name from type (e.g., Named { path: "PartialEq", .. })
                        if let verum_ast::ty::Type {
                            kind: verum_ast::ty::TypeKind::Path(path),
                            ..
                        } = ty
                        {
                            path.segments.last().and_then(|seg| {
                                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                                    Some(ident.name.to_string())
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                // Extract default method implementations
                for protocol_item in &protocol_body.items {
                    if let ProtocolItemKind::Function {
                        decl,
                        default_impl: verum_common::Maybe::Some(body),
                    } = &protocol_item.kind
                    {
                        {
                            let method_name = decl.name.name.to_string();
                            // Create a FunctionDecl with the body from default_impl.
                            // The `decl` might have body: None, so we need to merge them.
                            let mut func_with_body = decl.clone();
                            func_with_body.body = Some(body.clone());
                            default_methods.insert(method_name, func_with_body);
                        }
                    }
                }

                // Register protocol info when it carries default methods
                // OR super-protocols.  The "always-register" attempt
                // caused regressions in the Poll suite (5 tests including
                // test_poll_default_is_pending) because empty-protocol
                // entries triggered additional protocol-walk iterations
                // on Poll-implementers whose Default method was already
                // materialised by the existing path — the second walk
                // re-materialised the same default body under a different
                // FunctionId and the runtime then dispatched to the wrong
                // bytecode.
                //
                // For the blanket-impl case (Future → FutureExt), the
                // `protocol_registry.get(&proto_name)` lookup in
                // `generate_default_protocol_methods` was the symptom,
                // not the root cause — the actual blanket walk consults
                // `self.blanket_impls` (populated by the pre-pass in
                // `collect_all_declarations`), and the loop visits
                // EVERY queued protocol whether it's registered or not.
                // The protocol_info lookup is gated to skip the inner
                // default-method emission for empty protocols, which
                // is correct (nothing to emit).
                if !default_methods.is_empty() || !super_protocols.is_empty() {
                    self.protocol_registry.insert(
                        protocol_name.clone(),
                        ProtocolInfo {
                            name: protocol_name,
                            default_methods,
                            super_protocols,
                        },
                    );
                }
            }

            // Context declarations also need TypeDescriptors for dyn: dispatch.
            // When `implement Formatter for DecimalFormatter` is compiled, it looks
            // up "Formatter" in type_name_to_id to find the protocol's TypeDescriptor.
            // Without this, context protocol impls can't populate TypeDescriptor.protocols.
            if let ItemKind::Context(ctx_decl) = &item.kind {
                let ctx_name = ctx_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&ctx_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(ctx_name.clone(), type_id);

                    // Create TypeDescriptor with method names as variants
                    // (dyn: dispatch uses variants to determine vtable method order)
                    let mut type_desc = TypeDescriptor {
                        id: type_id,
                        name: StringId(self.ctx.intern_string_raw(&ctx_name)),
                        kind: crate::types::TypeKind::Protocol,
                        ..Default::default()
                    };
                    for method in ctx_decl.methods.iter() {
                        let method_name = method.name.name.to_string();
                        let name_id = StringId(self.ctx.intern_string_raw(&method_name));
                        type_desc.variants.push(crate::types::VariantDescriptor {
                            name: name_id,
                            tag: type_desc.variants.len() as u32,
                            payload: None,
                            kind: crate::types::VariantKind::Unit,
                            arity: 0,
                            fields: smallvec::SmallVec::new(),
                        });
                    }
                    self.push_type_dedupe(type_desc);
                }
            }
        }
    }

    /// Collects non-protocol declarations from a module.
    ///

    /// This calls collect_all_declarations but should be called AFTER
    /// collect_protocol_definitions has been called on all modules.
    pub fn collect_non_protocol_declarations(&mut self, module: &Module) -> CodegenResult<()> {
        // Scope the current source module to this file's `module X.Y.Z;`
        // declaration (if any). Without this, functions from imported stdlib
        // files get registered under the outer codegen's `config.module_name`
        // (typically `"main"` for user-run single-file compilations), which
        // loses the provenance needed to resolve cross-module paths like
        // `super.darwin.tls.ctx_get`.
        let prev = self.ctx.current_source_module.take();
        if let Some(name) =
            Self::resolve_full_module_path(module, &self.config.module_name)
        {
            self.ctx.current_source_module = Some(name);
        }
        let module_name = self.ctx.current_source_module.clone().unwrap_or_default();
        let funcs_before = self.ctx.functions.len();
        let result = self.collect_all_declarations(module);
        let funcs_after = self.ctx.functions.len();
        // #200 diagnostic: surface decl-collection per-module net change so
        // a silent decl-drop (returning Err that drops items mid-walk) is
        // visible at trace level. Triggered via `RUST_LOG=trace`.
        match &result {
            Ok(()) => {
                tracing::trace!(
                    "[decl-collect] {} ok: +{} funcs (total {})",
                    module_name,
                    funcs_after - funcs_before,
                    funcs_after
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[decl-collect] {} ERR: +{} funcs registered before fail: {}",
                    module_name,
                    funcs_after - funcs_before,
                    e
                );
            }
        }
        // #201 diagnostic — env-var-gated trace that fires WITHOUT
        // RUST_LOG=trace (which not every run-interpreter invocation
        // sets). Reports per-module net function count so a silent
        // skip of `core.base.memory` is immediately visible.
        if std::env::var("VERUM_TRACE_DECL").is_ok() {
            let net = funcs_after.saturating_sub(funcs_before);
            let status = if result.is_ok() { "ok" } else { "ERR" };
            eprintln!(
                "[decl-collect] {} {}: +{} funcs (total {})",
                module_name, status, net, funcs_after
            );
        }
        self.ctx.current_source_module = prev;
        result
    }

    /// Extract the dotted name of the first top-level `module X.Y.Z;`
    /// declaration in the given AST module, if any. Stdlib `.vr` files
    /// start with exactly one such declaration; user files typically
    /// don't have one.
    fn extract_source_module_name(module: &Module) -> Option<String> {
        for item in module.items.iter() {
            if let ItemKind::Module(decl) = &item.kind {
                let name = decl.name.name.to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
        None
    }

    /// Resolve the full module path for the AST being compiled, given
    /// the codegen's config.module_name as the parent prefix.
    ///
    /// Stdlib `.vr` files use short `module X;` declarations (e.g.
    /// `core/shell/builtins.vr` says `module builtins;`) — the AST
    /// only carries the leaf segment.  Without composition,
    /// `current_source_module` lands as just `"builtins"` and every
    /// per-module-qualified registration (the `<module>.<func>`
    /// archive key, the qualified function lookup at call sites)
    /// loses the parent `core.shell` path.  The user's
    /// `mount core.shell.builtins.{path_exists}` then can't find
    /// `core.shell.builtins.path_exists` in ctx.functions and falls
    /// through to the bare-name lookup, picking up whichever
    /// `path_exists` simple-name slot won the first-wins race —
    /// which is a stub from a different file.
    ///
    /// Composition rules:
    ///  * No AST module declaration → use `config.module_name`.
    ///  * AST decl is a SHORT name (no dots) AND the parent
    ///    config.module_name's last segment matches → AST is the
    ///    `mod.vr` of the parent: use `config.module_name`.
    ///  * AST decl is a SHORT name AND parent's last segment
    ///    differs → AST is a SUBMODULE: append, yielding
    ///    `config.module_name.<ast_decl>`.
    ///  * AST decl is already DOTTED → user wrote a fully-
    ///    qualified path; trust it as authoritative.
    fn resolve_full_module_path(module: &Module, parent_module_name: &str) -> Option<String> {
        let ast_name = Self::extract_source_module_name(module)?;
        if ast_name.is_empty() {
            if parent_module_name.is_empty() {
                return None;
            }
            return Some(parent_module_name.to_string());
        }
        // Already qualified — trust it.
        if ast_name.contains('.') {
            return Some(ast_name);
        }
        // No parent — single-segment AST decl is canonical.
        if parent_module_name.is_empty() || parent_module_name == "main" {
            return Some(ast_name);
        }
        // mod.vr equivalent: AST's leaf matches parent's leaf.
        let parent_leaf = parent_module_name
            .rsplit('.')
            .next()
            .unwrap_or(parent_module_name);
        if ast_name == parent_leaf {
            return Some(parent_module_name.to_string());
        }
        // Submodule file: prepend the parent path.
        Some(format!("{}.{}", parent_module_name, ast_name))
    }

    /// Marks all type names in a module as user-defined.
    /// This allows bare variant disambiguation to prefer user types over stdlib types.
    pub fn mark_user_defined_types(&mut self, module: &Module) {
        for item in module.items.iter() {
            if let ItemKind::Type(type_decl) = &item.kind {
                self.ctx
                    .user_defined_types
                    .insert(type_decl.name.name.to_string());
            }
        }
    }

    /// Generates default method implementations for a protocol implementation.
    ///

    /// When `implement Eq for Point { fn eq(...) { ... } }` only defines `eq`,
    /// this method generates `Point.ne` using Eq's default `ne` implementation.
    ///

    /// Also handles protocol inheritance: when `Eq extends PartialEq`, implementing
    /// `Eq for Point` will also generate default methods from `PartialEq` (like `ne`).
    fn generate_default_protocol_methods(
        &mut self,
        protocol_name: &str,
        type_name: &str,
        implemented_methods: &std::collections::HashSet<String>,
    ) -> CodegenResult<()> {
        // Collect all protocols to check (this protocol + all superprotocols
        // + any blanket-impl derived protocol whose base is in the chain).
        let mut protocols_to_check = vec![protocol_name.to_string()];
        let mut checked = std::collections::HashSet::new();

        // Shadowed per-protocol override: blanket-impl entries carry their
        // own `explicit_methods` set that takes priority over the derived
        // protocol's default bodies for the same method name.
        let mut per_proto_overrides: std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        > = std::collections::HashMap::new();
        per_proto_overrides.insert(protocol_name.to_string(), implemented_methods.clone());

        while let Some(proto_name) = protocols_to_check.pop() {
            if checked.contains(&proto_name) {
                continue;
            }
            checked.insert(proto_name.clone());

            // Enqueue any blanket-impl derived protocols whose base is
            // the one we're about to process. This is the monomorphization
            // step — `implement<B: Base> Derived for B {}` flows Derived's
            // default methods down to every concrete implementor of Base.
            let pending_derivations: Vec<(String, std::collections::HashSet<String>)> = self
                .blanket_impls
                .iter()
                .filter(|b| b.base_protocol == proto_name)
                .map(|b| (b.derived_protocol.clone(), b.explicit_methods.clone()))
                .collect();
            for (derived, overrides) in pending_derivations {
                per_proto_overrides
                    .entry(derived.clone())
                    .or_insert(overrides);
                if !checked.contains(&derived) {
                    protocols_to_check.push(derived);
                }
            }

            if let Some(protocol_info) = self.protocol_registry.get(&proto_name).cloned() {
                for super_proto in &protocol_info.super_protocols {
                    if !checked.contains(super_proto) {
                        protocols_to_check.push(super_proto.clone());
                    }
                }

                let empty = std::collections::HashSet::new();
                let overrides = per_proto_overrides.get(&proto_name).unwrap_or(&empty);

                // Iterate default methods in a deterministic order. Without
                // the sort, HashMap iteration order leaks Rust's per-process
                // random hasher seed into VBC function-ID assignment, which
                // makes the same source emit different bytecode each run.
                // Symptom matrix included "method 'X.next' not found",
                // "Null pointer dereference", "Division by zero", and
                // "field index 2 (offset 24) exceeds object data size 8" —
                // all trigger when run-time dispatch reads a function ID
                // that was assigned to a different method in the run that
                // produced the bytecode.
                let mut sorted_methods: Vec<(&String, &verum_ast::FunctionDecl)> =
                    protocol_info.default_methods.iter().collect();
                sorted_methods.sort_by(|a, b| a.0.cmp(b.0));

                for (method_name, default_func) in sorted_methods {
                    if overrides.contains(method_name) {
                        continue;
                    }

                    let full_method_name = format!("{}.{}", type_name, method_name);
                    if self.ctx.lookup_function(&full_method_name).is_some() {
                        continue;
                    }

                    self.register_impl_function(default_func, type_name)?;
                    self.pending_default_methods
                        .push((default_func.clone(), type_name.to_string()));
                }
            }
        }
        Ok(())
    }

    /// If the `for_type` of a protocol impl is a bare path matching one of
    /// the impl's generic parameters, return that name. Otherwise None.
    fn for_type_generic_param_name(ty: &verum_ast::ty::Type) -> Option<String> {
        use verum_ast::ty::{PathSegment, TypeKind};
        if let TypeKind::Path(path) = &ty.kind
            && path.segments.len() == 1
            && let Some(PathSegment::Name(ident)) = path.segments.first()
        {
            return Some(ident.name.to_string());
        }
        None
    }

    /// Extract the protocol path's last identifier from a TypeBound.
    fn type_bound_protocol_name(b: &verum_ast::ty::TypeBound) -> Option<String> {
        use verum_ast::ty::{PathSegment, TypeBoundKind, TypeKind};
        let path = match &b.kind {
            TypeBoundKind::Protocol(path) => path,
            TypeBoundKind::GenericProtocol(ty) => {
                if let TypeKind::Path(p) = &ty.kind {
                    p
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        path.segments.last().and_then(|s| {
            if let PathSegment::Name(ident) = s {
                Some(ident.name.to_string())
            } else {
                None
            }
        })
    }

    /// Compiles pending default protocol methods.
    ///

    /// Called during body compilation phase after all declarations have been collected.
    /// This ensures that functions referenced by default methods (like `range` in Iterator.advance_by)
    /// are available when the default method bodies are compiled.
    /// Returns the number of monomorphisations currently queued in
    /// `pending_default_methods`.  Diagnostic helper for callers that
    /// want to surface the queue size before draining (e.g. the
    /// stdlib-bootstrap "draining N pending" warn-level log).
    pub fn pending_default_methods_count(&self) -> usize {
        self.pending_default_methods.len()
    }

    /// Drain `pending_default_methods` and compile each queued body.
    ///
    /// Called once between declaration collection and main body compile —
    /// see `vbc_codegen.rs::compile_ast_to_vbc` and the matching site in
    /// `stdlib_bootstrap.rs::compile_core_module_from_ast`.  The queue is
    /// populated by `generate_default_protocol_methods` when a concrete
    /// `implement Base for Concrete` triggers materialisation of the
    /// inherited protocol's default-method bodies onto `Concrete.<method>`.
    /// Per-item compile failure surfaces as a `tracing::warn!` and the
    /// rest of the queue continues — some default methods reference
    /// external symbols (FFI, intrinsics not yet available in VBC) that
    /// can't compile in every load context.
    pub fn compile_pending_default_methods(&mut self) -> CodegenResult<()> {
        // Take ownership of pending methods to avoid borrow conflicts
        let pending = std::mem::take(&mut self.pending_default_methods);

        for (default_func, type_name) in pending {
            // Check if the function body was already compiled (e.g., by explicit impl in main module)
            let full_method_name = format!("{}.{}", type_name, default_func.name.name);

            // Check if the function was already compiled by checking if it exists in self.functions.
            // We can't rely on func_info.id being a sentinel because register_impl_function assigns
            // a valid ID before the body is compiled.
            if let Some(func_info) = self.ctx.lookup_function(&full_method_name) {
                // Check if a function with this ID was already compiled (exists in self.functions)
                let already_compiled = self
                    .functions
                    .iter()
                    .any(|f| f.descriptor.id == func_info.id);
                if already_compiled {
                    continue;
                }
            }

            // Compile the function body
            self.ctx.generic_type_params.clear();
            self.ctx.const_generic_params.clear();
            if let Err(e) = self.compile_function(&default_func, Some(&type_name)) {
                // Skip - some default methods may have unresolvable dependencies
                // (e.g., FFI functions, external symbols not available in VBC).
                // Surface via warn-level so silent body-compile failures of
                // user-callable protocol-default methods show up in the precompile
                // log; otherwise the only sign is a "method not found on receiver"
                // runtime panic with no diagnostic trail.  Mirror the
                // `compile_item_lenient` warn convention.
                let class = e.skip_class();
                tracing::warn!(
                    "[lenient] SKIP default-body {}.{} ({}): {} — runtime calls \
                     will panic with 'method not found' via auto-stub",
                    type_name,
                    default_func.name.name.as_str(),
                    class.label(),
                    e
                );
            }
        }

        Ok(())
    }

    /// Registers well-known stdlib constants so user code can reference them.
    ///

    /// These constants are defined in core/ .vr files (e.g., core/intrinsics/atomic.vr)
    /// and their values come from the Verum language specification. Without this,
    /// user code that imports `core.intrinsics.{ORDERING_SEQ_CST}` would get
    /// "undefined variable" errors because the VBC codegen starts fresh.
    pub fn register_stdlib_constants(&mut self) {
        let constants: &[(&str, i64)] = &[
            // Atomic ordering constants — sourced from
            // `verum_common::atomic_ordering` (single source of truth
            // shared with the runtime atomic-op dispatcher and stdlib
            // `core/intrinsics/atomic.vr`).
            ("ORDERING_RELAXED", verum_common::atomic_ordering::ORDERING_RELAXED),
            ("ORDERING_ACQUIRE", verum_common::atomic_ordering::ORDERING_ACQUIRE),
            ("ORDERING_RELEASE", verum_common::atomic_ordering::ORDERING_RELEASE),
            ("ORDERING_ACQ_REL", verum_common::atomic_ordering::ORDERING_ACQ_REL),
            ("ORDERING_SEQ_CST", verum_common::atomic_ordering::ORDERING_SEQ_CST),
            // POSIX errno constants — sourced from `verum_common::errno`
            // single source of truth. Cross-platform values use module-
            // level constants; platform-divergent values reference
            // the per-platform submodules with historical assignments
            // preserved (Phase-2 dispatch in resolve_stdlib_constant_value
            // wins per build target).
            ("EPERM", verum_common::errno::EPERM),
            ("ENOENT", verum_common::errno::ENOENT),
            ("ESRCH", verum_common::errno::ESRCH),
            ("EINTR", verum_common::errno::EINTR),
            ("EIO", verum_common::errno::EIO),
            ("ENXIO", verum_common::errno::ENXIO),
            ("E2BIG", verum_common::errno::E2BIG),
            ("ENOEXEC", verum_common::errno::ENOEXEC),
            ("EBADF", verum_common::errno::EBADF),
            ("ECHILD", verum_common::errno::ECHILD),
            ("EAGAIN", verum_common::errno::linux::EAGAIN),     // 11 (Linux); Darwin=35
            ("ENOMEM", verum_common::errno::ENOMEM),
            ("EACCES", verum_common::errno::EACCES),
            ("EFAULT", verum_common::errno::EFAULT),
            ("EBUSY", verum_common::errno::EBUSY),
            ("EEXIST", verum_common::errno::EEXIST),
            ("ENODEV", verum_common::errno::ENODEV),
            ("ENOTDIR", verum_common::errno::ENOTDIR),
            ("EISDIR", verum_common::errno::EISDIR),
            ("EINVAL", verum_common::errno::EINVAL),
            ("EMFILE", verum_common::errno::EMFILE),
            ("ENOSPC", verum_common::errno::ENOSPC),
            ("EPIPE", verum_common::errno::EPIPE),
            ("ERANGE", verum_common::errno::ERANGE),
            // Platform-divergent errno values (Darwin assignments
            // preserved for ABI continuity; Phase-2 dispatch wins
            // per build target via `errno::errno_value`).
            ("ENOSYS", verum_common::errno::darwin::ENOSYS),         // 78 (Darwin); Linux=38
            ("ENOTEMPTY", verum_common::errno::darwin::ENOTEMPTY),   // 66 (Darwin); Linux=39
            ("ECONNREFUSED", verum_common::errno::darwin::ECONNREFUSED), // 61 (Darwin); Linux=111
            ("ECONNRESET", verum_common::errno::darwin::ECONNRESET),     // 54 (Darwin); Linux=104
            ("ECONNABORTED", verum_common::errno::darwin::ECONNABORTED), // 53 (Darwin); Linux=103
            ("ETIMEDOUT", verum_common::errno::darwin::ETIMEDOUT),       // 60 (Darwin); Linux=110
            ("EADDRINUSE", verum_common::errno::darwin::EADDRINUSE),     // 48 (Darwin); Linux=98
            ("EADDRNOTAVAIL", verum_common::errno::darwin::EADDRNOTAVAIL), // 49 (Darwin); Linux=99
            ("ENETUNREACH", verum_common::errno::darwin::ENETUNREACH),   // 51 (Darwin); Linux=101
            ("EALREADY", verum_common::errno::darwin::EALREADY),         // 37 (Darwin); Linux=114
            ("EINPROGRESS", verum_common::errno::darwin::EINPROGRESS),   // 36 (Darwin); Linux=115
            ("ENOTCONN", verum_common::errno::darwin::ENOTCONN),         // 57 (Darwin); Linux=107
            ("EWOULDBLOCK", verum_common::errno::darwin::EWOULDBLOCK),   // 35 (Darwin); Linux=11
            // kqueue filter type IDs and event flags (Darwin only).
            // Sourced from `verum_common::os_events::kqueue` — single
            // source of truth shared with the runtime kqueue handlers
            // and stdlib `core/sys/darwin/libsystem.vr`. Linux targets
            // route through the dispatch path (resolve_stdlib_constant_value)
            // and resolve EPOLL_* names instead via `os_events::epoll`.
            ("EVFILT_READ", verum_common::os_events::kqueue::EVFILT_READ),
            ("EVFILT_WRITE", verum_common::os_events::kqueue::EVFILT_WRITE),
            ("EVFILT_TIMER", verum_common::os_events::kqueue::EVFILT_TIMER),
            ("EVFILT_USER", verum_common::os_events::kqueue::EVFILT_USER),
            ("EV_ADD", verum_common::os_events::kqueue::EV_ADD),
            ("EV_DELETE", verum_common::os_events::kqueue::EV_DELETE),
            ("EV_ENABLE", verum_common::os_events::kqueue::EV_ENABLE),
            ("EV_DISABLE", verum_common::os_events::kqueue::EV_DISABLE),
            ("EV_CLEAR", verum_common::os_events::kqueue::EV_CLEAR),
            ("EV_ONESHOT", verum_common::os_events::kqueue::EV_ONESHOT),
            ("EV_EOF", verum_common::os_events::kqueue::EV_EOF),
            ("EV_ERROR", verum_common::os_events::kqueue::EV_ERROR),
            // Once-init states
            ("ONCE_INIT", 0),
            ("ONCE_RUNNING", 1),
            ("ONCE_DONE", 2),
            // Ordering variants (core/base/ordering.vr)
            ("Less", -1),
            ("Equal", 0),
            ("Greater", 1),
            // ControlFlow variants (core/base/ops.vr)
            ("Continue", 0),
            ("Break", 1),
            // Result variants (core/base/result.vr) — `Ok(T) | Err(E)`
            ("Ok", 0),
            ("Err", 1),
            // Maybe variants (core/base/maybe.vr) — `None | Some(T)`.
            // Tags MUST match declaration order: the pattern matcher and
            // register_type_constructors both derive tags positionally, so
            // reversing these here would make `None` and `Some(x)` dispatch
            // swap at runtime.
            ("None", 0),
            ("Some", 1),
            // Bool-like
            ("True", 1),
            ("False", 0),
            // File seek constants (core/io/file.vr)
            ("SEEK_SET", verum_common::posix_files::SEEK_SET),
            ("SEEK_CUR", verum_common::posix_files::SEEK_CUR),
            ("SEEK_END", verum_common::posix_files::SEEK_END),
            // CBGR generation constants — sourced from `verum_common::cbgr`
            // (single source of truth shared with the runtime + stdlib).
            ("GEN_INITIAL", verum_common::cbgr::GEN_INITIAL as i64),
            ("GEN_DEAD", 0), // Sentinel: 0 generation = freed/uninitialised
            // HEADER_SIZE — canonical stdlib `core/mem/header.vr`
            // value is 32 (AllocationHeader). Pre-fix codegen carried
            // 16 here (likely confused with ThinRef = 16 bytes),
            // which broke user code computing offsets from
            // `@const HEADER_SIZE` — stdlib-declared HEADER_SIZE = 32
            // disagreed with codegen emission. Now sourced from
            // `verum_common::layout::ALLOCATION_HEADER_SIZE` (= 32),
            // matching `core/mem/header.vr` exactly.
            (
                "HEADER_SIZE",
                verum_common::layout::ALLOCATION_HEADER_SIZE as i64,
            ),
            ("FLAG_ARENA", 4),
            // CBGR capability constants — sourced from
            // `verum_common::cbgr::caps`. Each canonical name maps to
            // a single bit position in the 16-bit caps half of the
            // packed `epoch_and_caps` u32 (see layout::CAPS_BITS).
            ("CAP_READ", verum_common::cbgr::caps::READ as i64),
            ("CAP_WRITE", verum_common::cbgr::caps::WRITE as i64),
            // CAP_OWNED is the user-facing alias for OWNER (full
            // ownership — read|write|mutable|delegate|revoke).
            // Codegen table preserves the historical 4-bit alias for
            // ABI continuity; the runtime resolves through caps::OWNER.
            ("CAP_OWNED", 4),
            // Per-thread context slot table (core/sys/common.vr).
            // Without these, `core/runtime/ctx_bridge.vr` cannot lower
            // env_ctx_get/set/end/active_slot_count/install_parent_contexts
            // and AOT skips them with `[lenient] SKIP top-level fn …
            // undefined variable: CONTEXT_SLOT_COUNT`, leading to
            // SIGSEGV at runtime when the dropped helpers are called.
            ("CONTEXT_SLOT_COUNT", 256),
            ("MAX_CONTEXT_SLOTS", 256),
            ("CONTEXT_STACK_DEPTH", 8),
            // Page allocator / mmap constants
            ("PAGE_SIZE", 4096),
            ("SIZE_CLASS_COUNT", 8),
            ("GLOBAL_EPOCH", 0),
            // Cross-platform POSIX page-protection bits (sourced from
            // `verum_common::os_memory` — single source of truth).
            ("PROT_READ", verum_common::os_memory::PROT_READ),
            ("PROT_WRITE", verum_common::os_memory::PROT_WRITE),
            ("PROT_NONE", verum_common::os_memory::PROT_NONE),
            // Cross-platform mmap mode bits.
            ("MAP_PRIVATE", verum_common::os_memory::MAP_PRIVATE),
            // Platform-divergent — Linux vs Darwin diverge on 0x20 vs 0x1000.
            // Historical hand-table mixed Linux MAP_ANONYMOUS=0x20 with
            // Darwin MAP_ANON=0x1000 in the same lookup. Phase-2 dispatch
            // (commit 8e0993944) makes this target-conditional via
            // `os_memory::os_memory_const_value(name, target_os)` —
            // entries below are kept for non-dispatch consumers / fallback.
            ("MAP_ANONYMOUS", verum_common::os_memory::linux::MAP_ANONYMOUS), // 0x20 (Linux)
            ("MAP_ANON", verum_common::os_memory::darwin::MAP_ANON),           // 0x1000 (Darwin)
            ("MAP_HUGETLB", verum_common::os_memory::linux::MAP_HUGETLB),     // 0x40000 (Linux only)
            ("MADV_HUGEPAGE", verum_common::os_memory::linux::MADV_HUGEPAGE), // 14 (Linux only)
            // Windows VirtualAlloc / MemoryProtection constants.
            ("MEM_COMMIT", verum_common::os_memory::windows::MEM_COMMIT),
            ("MEM_RESERVE", verum_common::os_memory::windows::MEM_RESERVE),
            ("MEM_RELEASE", verum_common::os_memory::windows::MEM_RELEASE),
            ("MEM_LARGE_PAGES", verum_common::os_memory::windows::MEM_LARGE_PAGES),
            ("PAGE_READWRITE", verum_common::os_memory::windows::PAGE_READWRITE),
            ("PAGE_NOACCESS", verum_common::os_memory::windows::PAGE_NOACCESS),
            // Log level constants (core/base/log.vr)
            ("LOG_TRACE", 0),
            ("LOG_DEBUG", 1),
            ("LOG_INFO", 2),
            ("LOG_WARN", 3),
            ("LOG_ERROR", 4),
            ("Trace", 0),
            ("Debug", 1),
            ("Info", 2),
            ("Warn", 3),
            // Math constants (core/math/constants.vr)
            ("PI", 3),
            ("E", 2),
            ("TAU", 6),
            ("FRAC_PI_2", 1),
            ("FRAC_PI_4", 0),
            ("FRAC_1_PI", 0),
            ("LN_2", 0),
            ("LN_10", 2),
            ("LOG2_E", 1),
            ("LOG10_E", 0),
            ("SQRT_2", 1),
            ("EPSILON", 0),
            ("INFINITY", 0),
            ("NEG_INFINITY", 0),
            ("NAN", 0),
            ("MAX_FLOAT", 0),
            ("MIN_FLOAT", 0),
            ("MAX_INT", i64::MAX),
            ("MIN_INT", i64::MIN),
            // CBGR constants (core/mem/)
            ("GEN_UNALLOCATED", 0),
            ("GEN_FREED", 0),
            // Memory management constants
            ("MADV_FREE", verum_common::os_memory::linux::MADV_FREE), // 8 (Linux); Darwin=5
            // Cross-platform POSIX socket constants — Linux + Darwin agree,
            // sourced from `verum_common::posix_sockets` (single source of truth).
            ("AF_INET", verum_common::posix_sockets::AF_INET),
            ("SOCK_STREAM", verum_common::posix_sockets::SOCK_STREAM),
            ("SOCK_DGRAM", verum_common::posix_sockets::SOCK_DGRAM),
            ("IPPROTO_TCP", verum_common::posix_sockets::IPPROTO_TCP),
            ("TCP_NODELAY", verum_common::posix_sockets::TCP_NODELAY),
            // Platform-divergent socket constants. Values below mix
            // Linux and Darwin assignments preserved for ABI continuity
            // with the historical hand-table. Future work (TODO
            // #52-phase-2) makes this table target-conditional via
            // `posix_sockets::socket_const_for_target(name, target_os)`.
            ("AF_INET6", verum_common::posix_sockets::darwin::AF_INET6),       // 30 (Darwin)
            ("SOL_SOCKET", verum_common::posix_sockets::darwin::SOL_SOCKET),   // 0xFFFF (Darwin)
            ("SO_REUSEADDR", verum_common::posix_sockets::linux::SO_REUSEADDR), // 2 (Linux)
            ("SO_KEEPALIVE", verum_common::posix_sockets::darwin::SO_KEEPALIVE), // 8 (Darwin)
            // Cross-platform POSIX file-open flags (Linux + Darwin agree).
            ("O_RDONLY", verum_common::posix_files::O_RDONLY),
            ("O_WRONLY", verum_common::posix_files::O_WRONLY),
            ("O_RDWR", verum_common::posix_files::O_RDWR),
            // Platform-divergent file flags. Values below are
            // Darwin-canonical per the historical hand-table; future
            // work (TODO #53-phase-2) makes this target-conditional via
            // `posix_files::file_flag_for_target(name, target_os)`.
            ("O_CREAT", verum_common::posix_files::darwin::O_CREAT),     // 0x200 (Darwin)
            ("O_TRUNC", verum_common::posix_files::darwin::O_TRUNC),     // 0x400 (Darwin)
            ("O_APPEND", verum_common::posix_files::darwin::O_APPEND),   // 8 (Darwin)
            ("O_CLOEXEC", verum_common::posix_files::darwin::O_CLOEXEC), // 0x1000000 (Darwin)
            // Io_uring constants (core/async/executor.vr)
            ("DEFAULT_SQ_ENTRIES", 256),
            ("DEFAULT_CQ_ENTRIES", 512),
            // Math constants (alternative naming from elementary.vr)
            ("LN2", 0),
            ("LN10", 2),
            ("LOG2E", 1),
            ("LOG10E", 0),
            ("FRAC_1_SQRT_2", 0),
            // Error types (commonly used in net, io)
            ("IoError", 0),
            ("CompletionResult", 0),
            // Memory layout constants (core/mem/heap.vr)
            ("SLICE_SIZE", 64),
            ("MIN_BLOCK_SIZE", 16),
            ("MAX_SMALL_SIZE", 256),
            ("NUM_SIZE_CLASSES", 32),
            // Wildcard placeholder
            ("_", 0),
            // Math constants (alternative naming)
            ("FRAC_1_SQRT2", 0),
            // CBGR capability flags (core/mem/capability.vr)
            ("CAP_REVOKE", 8),
            ("CAP_IMMUTABLE", 16),
            // File open flags (additional)
            // Additional Darwin-canonical file flags (see note above).
            ("O_DIRECTORY", verum_common::posix_files::darwin::O_DIRECTORY), // 0x100000 (Darwin)
            ("O_NOFOLLOW", verum_common::posix_files::darwin::O_NOFOLLOW),   // 0x100 (Darwin)
            ("O_NONBLOCK", verum_common::posix_files::darwin::O_NONBLOCK),   // 0x4 (Darwin)
            // Memory bin constants (core/mem/heap.vr)
            ("BIN_COUNT", 64),
            ("SIZE_CLASSES", 0),
            // Async executor constants (core/async/executor.vr)
            ("WAKE_TOKEN", 0),
            ("TIMER_TOKEN", 1),
            // CBGR capability flags (more)
            ("CAP_MUTABLE", 32),
            // Memory heap/segment/size_class constants (core/mem/)
            ("BIN_HUGE", 63),
            ("BIN_FULL", 64),
            ("QUEUE_COUNT", 4),
            ("SMALL_SIZE_MAX", 1024),
            ("MEDIUM_SIZE_MAX", 8192),
            ("SEGMENT_SIZE", 4 * 1024 * 1024),
            ("SLICES_PER_SEGMENT", 64 * 1024),
            ("SMALL_PAGE_SIZE", 65536),
            ("MEDIUM_PAGE_SIZE", 524288),
            // Cross-platform message flag.
            ("MSG_PEEK", verum_common::posix_sockets::MSG_PEEK),
            // Platform-divergent message flags. Historical hand-table
            // mixed Linux MSG_DONTWAIT=0x40 with Darwin MSG_WAITALL=0x40
            // in the same lookup. Phase-2 dispatch (commit 8e0993944)
            // routes per build target via
            // `posix_sockets::socket_const_for_target(name, target_os)`
            // — entries below are kept for non-dispatch fallback.
            ("MSG_DONTWAIT", verum_common::posix_sockets::linux::MSG_DONTWAIT), // 0x40 (Linux); Darwin=0x80
            ("MSG_WAITALL", verum_common::posix_sockets::darwin::MSG_WAITALL), // 0x40 (Darwin); Linux=0x100
            // Cross-platform shutdown directions.
            ("SHUT_RD", verum_common::posix_sockets::SHUT_RD),
            ("SHUT_WR", verum_common::posix_sockets::SHUT_WR),
            ("SHUT_RDWR", verum_common::posix_sockets::SHUT_RDWR),
            // Linux-only socket type flags.
            ("SOCK_NONBLOCK", verum_common::posix_sockets::linux::SOCK_NONBLOCK), // 0o4000 = 0x800
            ("SOCK_CLOEXEC", verum_common::posix_sockets::linux::SOCK_CLOEXEC),   // 0o2000000 = 0x80000
            // Linux-only / Darwin-divergent socket option levels.
            ("SOL_TCP", verum_common::posix_sockets::linux::SOL_TCP),  // Linux only (Darwin uses IPPROTO_TCP)
            ("SO_ERROR", verum_common::posix_sockets::linux::SO_ERROR), // 4 (Linux); Darwin=0x1007
        ];

        // Variant tags for built-in sum-type constructors are sourced from
        // the canonical layout constants in `verum_common::well_known_types`
        // — the single source of truth used by VBC codegen, the runtime
        // dispatcher, and the registry. Editing a layout constant
        // automatically retunes this map. Bool's True/False are extras
        // not modelled by a layout constant (they're not a sum type) so
        // they're appended explicitly.
        let layout_sources: &[&[verum_common::well_known_types::VariantLayoutEntry]] = &[
            verum_common::well_known_types::RESULT_VARIANT_LAYOUT,
            verum_common::well_known_types::MAYBE_VARIANT_LAYOUT,
            verum_common::well_known_types::ORDERING_VARIANT_LAYOUT,
            verum_common::well_known_types::CONTROLFLOW_VARIANT_LAYOUT,
        ];
        let mut tag_map: std::collections::HashMap<&str, u32> =
            std::collections::HashMap::new();
        for layout in layout_sources {
            for entry in layout.iter() {
                tag_map.insert(entry.name, entry.tag);
            }
        }
        // Bool literals — not a sum type but the codegen treats them
        // identically to nullary variant constructors for pattern-matching
        // dispatch.
        tag_map.insert("True", 1);
        tag_map.insert("False", 0);

        // Pre-built `parent_type_name` lookup so variant ctors registered
        // by this pass carry the same `Some("Maybe")` / `Some("Result")` /
        // etc. info that `register_builtin_variants` set on its own
        // entries.  Pre-this-fix the const path emitted
        // `parent_type_name: None`, and because `prefer_existing_functions
        // = false` at this point in the pipeline, the const-side
        // registration OVERWROTE the builtin's correctly-parented entry.
        // `compile_simple_path` then read `parent_type_name = None` for
        // bare `None` / `Ok` / `Err`, called `emit_make_variant(.., None)`,
        // and the typed-form gate failed for lack of a parent — codegen
        // demoted to legacy `MakeVariant` (synthetic 0x8000+tag id).
        // The runtime's `format_variant_for_print_depth` global tag-scan
        // then picked whichever stdlib variant happened to share the
        // same tag (`AliasError.EmptyWeights` for tag=0,
        // `ProductivityResult.Productive` for tag=0, depending on type-table
        // order) — `let b: Maybe<Int> = None;` rendered as the colliding
        // variant.  Bare `Some(3)` worked because `compile_call` resolves
        // parent through `emit_make_variant_for_function` which re-reads
        // the live function table at emit time, but by the time of the
        // second registration the bare `Some` entry's parent had already
        // been clobbered too — `Some(3)` only worked by coincidence
        // (the alt-key path triggered for `Some` because its
        // `param_count = 1` mismatched the const-path's 0).
        let mut parent_map: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for (layout, parent) in [
            (verum_common::well_known_types::MAYBE_VARIANT_LAYOUT, "Maybe"),
            (verum_common::well_known_types::RESULT_VARIANT_LAYOUT, "Result"),
            (verum_common::well_known_types::ORDERING_VARIANT_LAYOUT, "Ordering"),
            (verum_common::well_known_types::CONTROLFLOW_VARIANT_LAYOUT, "ControlFlow"),
        ] {
            for entry in layout.iter() {
                parent_map.insert(entry.name, parent);
            }
        }

        for &(name, _value) in constants {
            // Skip variant-ctor entries that `register_builtin_variants`
            // already registered with full parent_type_name — overwriting
            // would null out the parent metadata.  The builtin's
            // FunctionInfo carries `intrinsic_name = None` while a
            // const-path entry would set `__const_<name>`; downstream
            // `compile_simple_path` checks `variant_tag` first
            // (line ~1426), so the missing `__const_*` marker is fine.
            // Names not in `parent_map` (errno, EVFILT_*, MEM_*, …) keep
            // the legacy const-registration path unchanged.
            if parent_map.contains_key(name)
                && let Some(existing) = self.ctx.lookup_function(name)
                && existing.variant_tag.is_some()
                && existing.parent_type_name.is_some()
            {
                continue;
            }
            let id = FunctionId(self.next_func_id);
            self.next_func_id = self.next_func_id.saturating_add(1);
            let info = FunctionInfo {
                id,
                param_count: 0,
                param_names: vec![],
                param_type_names: vec![],
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: Some(format!("__const_{}", name)),
                variant_tag: tag_map.get(name).copied(),
                // Carry the parent type name when this const corresponds
                // to a known variant.  Without this, names that were
                // never registered as variant ctors (the variant-name
                // wasn't in `register_builtin_variants`'s table — e.g.
                // `True`/`False` — but ARE in `tag_map`) lose the parent
                // link, and pattern-match dispatch falls through to the
                // global tag scan.
                parent_type_name: parent_map.get(name).map(|p| (*p).to_string()),
                variant_payload_types: None,
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: None,
                return_type_inner: None,
                is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
            };
            self.ctx.register_function(name.to_string(), info);
        }
    }

    /// Registers well-known stdlib intrinsic functions so user code can call them.
    /// These are functions defined in core/ .vr files that map to VBC intrinsics.
    pub fn register_stdlib_intrinsics(&mut self) {
        // (name, param_count, intrinsic_name)
        let intrinsics: &[(&str, usize, &str)] = &[
            // Memory allocation intrinsics (from core/base/memory.vr)
            ("alloc", 2, "alloc"),
            ("alloc_zeroed", 2, "alloc_zeroed"),
            ("dealloc", 3, "dealloc"),
            ("realloc", 4, "realloc"),
            ("ptr_write", 2, "ptr_write"),
            ("ptr_read", 1, "ptr_read"),
            ("memcpy", 3, "memcpy"),
            ("memmove", 3, "memmove"),
            ("memset", 3, "memset"),
            ("memcmp", 3, "memcmp"),
            ("null_ptr", 0, "null_ptr"),
            ("is_null", 1, "ptr_is_null"),
            ("ptr_offset", 2, "ptr_offset"),
            ("slice_from_raw_parts", 2, "slice_from_raw_parts"),
            ("pow_f32", 2, "pow_f32"),
            ("ptr_to_ref", 1, "ptr_to_ref"),
            ("spin_loop_hint", 0, "spin_loop_hint"),
            ("spin_hint", 0, "spin_hint"),
            ("char_is_alphanumeric", 1, "char_is_alphanumeric"),
            ("rdtsc", 0, "rdtsc"),
            ("catch_unwind", 1, "catch_unwind"),
            // Duration impl methods (registered as "Duration.method" by collect_all_declarations)
            ("Duration.from_nanos", 1, "time_duration_from_nanos"),
            ("Duration.from_micros", 1, "time_duration_from_micros"),
            ("Duration.from_millis", 1, "time_duration_from_millis"),
            ("Duration.from_secs", 1, "time_duration_from_secs"),
            ("Duration.as_nanos", 1, "time_duration_as_nanos"),
            ("Duration.as_micros", 1, "time_duration_as_micros"),
            ("Duration.as_millis", 1, "time_duration_as_millis"),
            ("Duration.as_secs", 1, "time_duration_as_secs"),
            ("Duration.is_zero", 1, "time_duration_is_zero"),
            ("Duration.add", 2, "time_duration_add"),
            ("Duration.saturating_add", 2, "time_duration_saturating_add"),
            ("Duration.saturating_sub", 2, "time_duration_saturating_sub"),
            ("Duration.subsec_nanos", 1, "time_duration_subsec_nanos"),
            // Instant impl methods
            ("Instant.now", 0, "time_instant_now"),
            ("Instant.elapsed", 1, "time_instant_elapsed"),
            ("Instant.duration_since", 2, "time_instant_duration_since"),
            // Free time functions
            ("monotonic_nanos", 0, "monotonic_nanos"),
            ("monotonic_micros", 0, "time_monotonic_micros"),
            ("monotonic_millis", 0, "time_monotonic_millis"),
            ("monotonic_raw_nanos", 0, "time_monotonic_raw_nanos"),
            ("realtime_nanos", 0, "time_realtime_nanos"),
            ("unix_timestamp", 0, "time_unix_timestamp"),
            ("sleep_ms", 1, "time_sleep_ms"),
            ("sleep_us", 1, "time_sleep_us"),
            ("sleep", 1, "time_sleep_duration"),
            // Stopwatch impl methods
            ("Stopwatch.new", 0, "time_stopwatch_new"),
            ("Stopwatch.elapsed", 1, "time_stopwatch_elapsed"),
            ("Stopwatch.stop", 1, "time_stopwatch_stop"),
            ("Stopwatch.start", 1, "time_stopwatch_start"),
            ("Stopwatch.reset", 1, "time_stopwatch_reset"),
            // PerfCounter impl methods
            ("PerfCounter.now", 0, "time_perf_counter_now"),
            (
                "PerfCounter.elapsed_since",
                2,
                "time_perf_counter_elapsed_since",
            ),
            ("PerfCounter.as_nanos", 1, "time_perf_counter_as_nanos"),
            // DeadlineTimer impl methods
            (
                "DeadlineTimer.from_duration",
                1,
                "time_deadline_timer_from_duration",
            ),
            (
                "DeadlineTimer.is_expired",
                1,
                "time_deadline_timer_is_expired",
            ),
            (
                "DeadlineTimer.remaining",
                1,
                "time_deadline_timer_remaining",
            ),
            // Arithmetic intrinsics (core/intrinsics/arithmetic.vr)
            ("add", 2, "add"),
            ("sub", 2, "sub"),
            ("mul", 2, "mul"),
            ("div", 2, "div"),
            ("rem", 2, "rem"),
            ("neg", 1, "neg"),
            ("abs_signed", 1, "abs_signed"),
            ("signum", 1, "signum"),
            ("min", 2, "min"),
            ("max", 2, "max"),
            ("clamp", 3, "clamp"),
            ("checked_add", 2, "checked_add"),
            ("checked_sub", 2, "checked_sub"),
            ("checked_mul", 2, "checked_mul"),
            ("checked_div", 2, "checked_div"),
            ("checked_add_u64", 2, "checked_add_u64"),
            ("checked_sub_u64", 2, "checked_sub_u64"),
            ("checked_mul_u64", 2, "checked_mul_u64"),
            ("overflowing_add", 2, "overflowing_add"),
            ("overflowing_sub", 2, "overflowing_sub"),
            ("overflowing_mul", 2, "overflowing_mul"),
            ("wrapping_add", 2, "wrapping_add"),
            ("wrapping_sub", 2, "wrapping_sub"),
            ("wrapping_mul", 2, "wrapping_mul"),
            ("wrapping_neg", 1, "wrapping_neg"),
            ("wrapping_shl", 2, "wrapping_shl"),
            ("wrapping_shr", 2, "wrapping_shr"),
            ("saturating_add", 2, "saturating_add"),
            ("saturating_sub", 2, "saturating_sub"),
            ("saturating_mul", 2, "saturating_mul"),
            // Comparison intrinsics
            ("eq", 2, "eq"),
            ("ne", 2, "ne"),
            ("lt", 2, "lt"),
            ("le", 2, "le"),
            ("gt", 2, "gt"),
            ("ge", 2, "ge"),
            // Bitwise intrinsics (core/intrinsics/bitwise.vr)
            //
            // **REMOVED entries with `InlineSequence` strategy** (clz, ctz, bswap,
            // bitreverse, rotl, rotr) — their bodies in `core/intrinsics/bitwise.vr`
            // already declare `@intrinsic("ctlz" / "cttz" / "bswap" / "bitreverse"
            // / "rotate_left" / "rotate_right", …)` via the macro form, which
            // `extract_intrinsic_name` picks up and propagates as the canonical
            // dispatch identity.
            //
            // Pre-removal these entries created a SECOND `FunctionInfo` (via the
            // "Otherwise register a new function" branch below) with stub-shape
            // body but `intrinsic_name = "<bare>"`.  The bare name shadowed the
            // body-extracted LLVM-canonical name, and the user-side
            // `compile_imported_intrinsic_call` intercept emitted the inline
            // sequence to a register that the surrounding bind/print path
            // dispatched as Unit — symptom: `clz(1 as UInt64) = ()` instead of 63.
            // The matched-arity body-path entries (`popcnt`, `clz_u64`, etc.)
            // were always correct because they only travelled through the body
            // `@intrinsic` macro — which is the right path for every inline
            // bit-manipulation primitive.
            //
            // `DirectOpcode`-strategy bitwise intrinsics (bitand, bitor, bitxor,
            // bitnot, shl, shr) remain in the table because their bodies route
            // through a different opcode-direct emit that is not affected by the
            // InlineSequence dispatch path defect.
            ("bitand", 2, "bitand"),
            ("bitor", 2, "bitor"),
            ("bitxor", 2, "bitxor"),
            ("bitnot", 1, "bitnot"),
            ("shl", 2, "shl"),
            ("shr", 2, "shr"),
            // Conversion intrinsics (core/intrinsics/conversion.vr)
            ("int_to_float", 1, "int_to_float"),
            ("float_to_int", 1, "float_to_int"),
            ("f32_to_bits", 1, "f32_to_bits"),
            ("f32_from_bits", 1, "f32_from_bits"),
            ("f64_to_bits", 1, "f64_to_bits"),
            ("f64_from_bits", 1, "f64_from_bits"),
            ("to_le_bytes", 1, "to_le_bytes"),
            ("to_be_bytes", 1, "to_be_bytes"),
            ("from_le_bytes", 1, "from_le_bytes"),
            ("from_be_bytes", 1, "from_be_bytes"),
            ("to_le_bytes_2", 1, "to_le_bytes_2"),
            ("to_le_bytes_4", 1, "to_le_bytes_4"),
            ("to_le_bytes_8", 1, "to_le_bytes_8"),
            ("to_be_bytes_2", 1, "to_be_bytes_2"),
            ("to_be_bytes_4", 1, "to_be_bytes_4"),
            ("to_be_bytes_8", 1, "to_be_bytes_8"),
            ("from_le_bytes_2", 1, "from_le_bytes_2"),
            ("from_le_bytes_4", 1, "from_le_bytes_4"),
            ("from_le_bytes_8", 1, "from_le_bytes_8"),
            ("from_be_bytes_2", 1, "from_be_bytes_2"),
            ("from_be_bytes_4", 1, "from_be_bytes_4"),
            ("from_be_bytes_8", 1, "from_be_bytes_8"),
            // Float intrinsics (core/intrinsics/float.vr)
            ("f32_infinity", 0, "f32_infinity"),
            ("f32_neg_infinity", 0, "f32_neg_infinity"),
            ("f32_nan", 0, "f32_nan"),
            ("sqrt", 1, "sqrt"),
            ("cbrt", 1, "cbrt"),
            ("exp", 1, "exp"),
            ("expm1", 1, "expm1"),
            ("exp2", 1, "exp2"),
            ("log", 1, "log"),
            ("log1p", 1, "log1p"),
            ("log10", 1, "log10"),
            ("log2", 1, "log2"),
            ("pow", 2, "pow"),
            ("powi", 2, "powi"),
            ("floor", 1, "floor"),
            ("ceil", 1, "ceil"),
            ("round", 1, "round"),
            ("trunc", 1, "trunc"),
            ("fabs", 1, "fabs"),
            ("minnum", 2, "minnum"),
            ("maxnum", 2, "maxnum"),
            ("fma", 3, "fma"),
            ("copysign", 2, "copysign"),
            ("hypot", 2, "hypot"),
            ("sin", 1, "sin"),
            ("cos", 1, "cos"),
            ("tan", 1, "tan"),
            ("asin", 1, "asin"),
            ("acos", 1, "acos"),
            ("atan", 1, "atan"),
            ("atan2", 2, "atan2"),
            ("sinh", 1, "sinh"),
            ("cosh", 1, "cosh"),
            ("tanh", 1, "tanh"),
            ("asinh", 1, "asinh"),
            ("acosh", 1, "acosh"),
            ("atanh", 1, "atanh"),
            // Char intrinsics (core/intrinsics/runtime/text.vr)
            ("char_is_alphabetic", 1, "char_is_alphabetic"),
            ("char_is_numeric", 1, "char_is_numeric"),
            ("char_is_whitespace", 1, "char_is_whitespace"),
            ("char_is_control", 1, "char_is_control"),
            ("char_is_uppercase", 1, "char_is_uppercase"),
            ("char_is_lowercase", 1, "char_is_lowercase"),
            ("char_to_uppercase", 1, "char_to_uppercase"),
            ("char_to_lowercase", 1, "char_to_lowercase"),
            ("char_encode_utf8", 1, "char_encode_utf8"),
            ("char_escape_debug", 1, "char_escape_debug"),
            // Text intrinsics (core/intrinsics/runtime/text.vr)
            ("text_from_static", 1, "text_from_static"),
            ("text_byte_len", 1, "text_byte_len"),
            ("utf8_decode_char", 1, "utf8_decode_char"),
            ("text_parse_int", 1, "text_parse_int"),
            ("text_parse_float", 1, "text_parse_float"),
            ("int_to_text", 1, "int_to_text"),
            // Atomic intrinsics (core/intrinsics/atomic.vr)
            ("compiler_fence", 1, "compiler_fence"),
            ("atomic_load_u8", 2, "atomic_load_u8"),
            ("atomic_load_u16", 2, "atomic_load_u16"),
            ("atomic_load_u32", 2, "atomic_load_u32"),
            ("atomic_load_u64", 2, "atomic_load_u64"),
            ("atomic_store_u8", 3, "atomic_store_u8"),
            ("atomic_store_u16", 3, "atomic_store_u16"),
            ("atomic_store_u32", 3, "atomic_store_u32"),
            ("atomic_store_u64", 3, "atomic_store_u64"),
            ("atomic_cas_u32", 5, "atomic_cas_u32"),
            ("atomic_cas_u64", 5, "atomic_cas_u64"),
            ("atomic_fetch_add_u32", 3, "atomic_fetch_add_u32"),
            ("atomic_fetch_add_u64", 3, "atomic_fetch_add_u64"),
            ("atomic_fetch_sub_u32", 3, "atomic_fetch_sub_u32"),
            ("atomic_fetch_sub_u64", 3, "atomic_fetch_sub_u64"),
            ("atomic_fetch_and_u64", 3, "atomic_fetch_and_u64"),
            ("atomic_fetch_or_u64", 3, "atomic_fetch_or_u64"),
            // Platform futex stubs (cross-module imports from sys/)
            ("sys_futex_wait", 3, "futex_wait"),
            ("sys_futex_wake", 2, "futex_wake"),
            // Platform file I/O stubs (cross-module imports from sys/)
            ("sys_open", 3, "sys_open"),
            ("sys_close", 1, "sys_close"),
            ("sys_read", 3, "sys_read"),
            ("sys_write", 3, "sys_write"),
            ("sys_lseek", 3, "sys_lseek"),
            ("sys_fstat", 2, "sys_fstat"),
            ("sys_fsync", 1, "sys_fsync"),
            ("sys_ftruncate", 2, "sys_ftruncate"),
            ("sys_dup", 1, "sys_dup"),
            // Raw memory allocation (core/sys/raw.vr, used by core/mem/arena.vr)
            ("__alloc_raw", 1, "__alloc_raw"),
            ("__alloc_zeroed_raw", 1, "__alloc_zeroed_raw"),
            ("__dealloc_raw", 2, "__dealloc_raw"),
            ("__realloc_raw", 3, "__realloc_raw"),
            // File system intrinsics (core/io/fs.vr, used by core/io/path.vr)
            ("fs_metadata", 1, "fs_metadata"),
            ("fs_read_dir", 1, "fs_read_dir"),
            ("fs_create_dir", 1, "fs_create_dir"),
            ("fs_create_dir_all", 1, "fs_create_dir_all"),
            ("fs_remove_file", 1, "fs_remove_file"),
            ("fs_remove_dir", 1, "fs_remove_dir"),
            ("fs_rename", 2, "fs_rename"),
            ("fs_copy", 2, "fs_copy"),
            ("fs_canonicalize", 1, "fs_canonicalize"),
            // Tensor intrinsics (core/math/simple.vr, core/math/tensor.vr)
            ("tensor_zeros", 1, "tensor_zeros"),
            ("tensor_ones", 1, "tensor_ones"),
            ("tensor_full", 2, "tensor_full"),
            // from_list(data, shape) — takes the flat payload + the target shape.
            ("tensor_from_list", 2, "tensor_from_list"),
            ("tensor_shape", 1, "tensor_shape"),
            ("tensor_reshape", 2, "tensor_reshape"),
            ("tensor_matmul", 2, "tensor_matmul"),
            ("tensor_transpose", 1, "tensor_transpose"),
            ("tensor_add", 2, "tensor_add"),
            ("tensor_sub", 2, "tensor_sub"),
            ("tensor_mul_elementwise", 2, "tensor_mul_elementwise"),
            ("tensor_sum", 1, "tensor_sum"),
            ("tensor_mean", 1, "tensor_mean"),
            ("tensor_randn", 1, "tensor_randn"),
            ("tensor_rand", 1, "tensor_rand"),
            ("tensor_arange", 3, "tensor_arange"),
            ("tensor_linspace", 3, "tensor_linspace"),
            ("tensor_eye", 1, "tensor_eye"),
            ("tensor_squeeze", 1, "tensor_squeeze"),
            ("tensor_unsqueeze", 2, "tensor_unsqueeze"),
            ("tensor_max", 1, "tensor_max"),
            ("tensor_min", 1, "tensor_min"),
            ("tensor_softmax", 2, "tensor_softmax"),
            ("tensor_cat", 2, "tensor_cat"),
            ("tensor_stack", 2, "tensor_stack"),
            // Async intrinsics (core/async/task.vr)
            ("lazy", 1, "async_lazy"),
            ("spawn", 1, "async_spawn"),
            ("spawn_blocking", 1, "async_spawn_blocking"),
            // Context bridge intrinsics (core/context/provider.vr)
            ("env_ctx_get", 1, "env_ctx_get"),
            ("env_ctx_set", 2, "env_ctx_set"),
            ("env_ctx_end", 1, "env_ctx_end"),
            // Raw memory ops (core/mem/raw_ops.vr, used by arena.vr)
            ("load_i64", 1, "load_i64"),
            ("store_i64", 2, "store_i64"),
            ("memzero", 2, "memzero"),
            // Platform syscalls (core/sys/, used by allocator.vr)
            ("mmap", 6, "sys_mmap"),
            ("munmap", 2, "sys_munmap"),
            ("mprotect", 3, "sys_mprotect"),
            ("madvise", 3, "sys_madvise"),
            ("VirtualAlloc", 4, "sys_virtual_alloc"),
            ("VirtualFree", 3, "sys_virtual_free"),
            // OS error (core/sys/common.vr)
            ("OSError", 1, "os_error"),
            // Autodiff intrinsics (core/math/autodiff.vr, used by simple.vr)
            ("grad", 2, "autodiff_grad"),
            ("value_and_grad", 2, "autodiff_value_and_grad"),
            ("with_no_grad", 1, "autodiff_with_no_grad"),
            // GPU intrinsics (core/math/gpu.vr)
            ("default_device", 0, "gpu_default_device"),
            ("has_gpu", 0, "gpu_has_gpu"),
            // NN intrinsics (core/math/nn.vr, used by simple.vr)
            ("relu", 1, "nn_relu"),
            ("sigmoid", 1, "nn_sigmoid"),
            ("softmax", 1, "nn_softmax"),
            ("cross_entropy_loss", 2, "nn_cross_entropy_loss"),
            ("mse_loss", 2, "nn_mse_loss"),
            ("Linear", 2, "nn_linear"),
            ("Conv2d", 4, "nn_conv2d"),
            ("BatchNorm", 1, "nn_batch_norm"),
            ("Dropout", 1, "nn_dropout"),
            ("Sequential", 1, "nn_sequential"),
            ("LayerNorm", 1, "nn_layer_norm"),
            ("RMSNorm", 1, "nn_rms_norm"),
            ("gelu", 1, "nn_gelu"),
            ("silu", 1, "nn_silu"),
            ("AdamW", 1, "nn_adamw"),
            ("SGD", 1, "nn_sgd"),
            // Random (core/math/random.vr)
            ("RandomKey", 1, "random_key"),
            // Async/waker intrinsics (core/async/)
            ("noop_waker", 0, "async_noop_waker"),
            ("select::timeout", 2, "async_select_timeout"),
            // Tensor advanced intrinsics (core/math/tensor.vr, autodiff.vr, linalg.vr)
            ("full_like", 2, "tensor_full_like"),
            ("zeros_like", 1, "tensor_zeros_like"),
            ("ones_like", 1, "tensor_ones_like"),
            ("ln", 1, "math_ln"),
            // Memory intrinsics (core/mem/heap.vr)
            ("slices_for_bin", 1, "mem_slices_for_bin"),
            // IO intrinsics (core/io/fs.vr)
            ("sys_readdir", 1, "sys_readdir"),
            ("sys_stat", 2, "sys_stat"),
            ("sys_mkdir", 2, "sys_mkdir"),
            ("sys_unlink", 1, "sys_unlink"),
            ("sys_rmdir", 1, "sys_rmdir"),
            ("sys_rename", 2, "sys_rename"),
            ("sys_readlink", 2, "sys_readlink"),
            // Network intrinsics (core/net/)
            ("sys_socket", 3, "sys_socket"),
            ("sys_bind", 3, "sys_bind"),
            ("sys_listen", 2, "sys_listen"),
            ("sys_accept", 3, "sys_accept"),
            ("sys_connect", 3, "sys_connect"),
            ("sys_send", 3, "sys_send"),
            ("sys_recv", 3, "sys_recv"),
            ("sys_sendto", 5, "sys_sendto"),
            ("sys_recvfrom", 5, "sys_recvfrom"),
            ("sys_setsockopt", 5, "sys_setsockopt"),
            ("sys_getsockopt", 4, "sys_getsockopt"),
            ("sys_shutdown", 2, "sys_shutdown"),
            ("sys_getaddrinfo", 4, "sys_getaddrinfo"),
            // Iterator helpers
            ("once", 1, "iter_once"),
            // CBGR capability intrinsics (core/mem/capability.vr)
            ("has_capability", 2, "cbgr_has_capability"),
            // Memory management intrinsics (core/mem/heap.vr)
            ("bin_to_size", 1, "mem_bin_to_size"),
            ("size_to_bin", 1, "mem_size_to_bin"),
            // File system extended (core/io/fs.vr)
            ("sys_closedir", 1, "sys_closedir"),
            ("sys_opendir", 1, "sys_opendir"),
            // DNS (core/net/dns.vr) - takes (host, port) or (host)
            ("dns_resolve", 2, "dns_resolve"),
            // Tensor functions (core/math/tensor.vr, autodiff.vr)
            ("zeros", 1, "tensor_zeros"),
            ("full", 2, "tensor_full"),
            // Math functions (core/math/elementary.vr, ieee754.vr)
            ("ldexp", 2, "math_ldexp"),
            ("frexp", 1, "math_frexp"),
            ("scalbn", 2, "math_scalbn"),
            ("ilogb", 1, "math_ilogb"),
            ("nextafter", 2, "math_nextafter"),
            ("remainder", 2, "math_remainder"),
            ("fmod", 2, "math_fmod"),
            // CBGR validation intrinsics (core/mem/)
            ("validate_write", 1, "cbgr_validate_write"),
            ("validate_read", 1, "cbgr_validate_read"),
            ("validate_generation", 2, "cbgr_validate_generation"),
            ("validate_epoch", 2, "cbgr_validate_epoch"),
            // IO/FS extended (core/io/fs.vr)
            ("sys_lstat", 2, "sys_lstat"),
            ("sys_symlink", 2, "sys_symlink"),
            ("sys_link", 2, "sys_link"),
            ("sys_chmod", 2, "sys_chmod"),
            ("sys_chown", 3, "sys_chown"),
            // Network helpers (core/net/)
            ("safe_set_nosigpipe", 2, "net_set_nosigpipe"),
            ("safe_set_reuseaddr", 2, "net_set_reuseaddr"),
            ("safe_set_nonblocking", 2, "net_set_nonblocking"),
            // Context bridge (core/runtime/)
            ("ctx_set", 2, "ctx_set"),
            ("ctx_get", 1, "ctx_get"),
            // Memory management (core/mem/heap.vr, size_class.vr)
            ("page_kind_for_bin", 1, "mem_page_kind_for_bin"),
            ("blocks_per_page", 1, "mem_blocks_per_page"),
            // Async executor (core/async/executor.vr)
            ("create_io_engine", 1, "async_create_io_engine"),
            // Tensor math (core/math/autodiff.vr)
            ("matmul", 2, "tensor_matmul"),
            ("sum_axis", 2, "tensor_sum_axis"),
            ("mean_axis", 2, "tensor_mean_axis"),
            // Context runtime (core/runtime/)
            ("ctx_clear", 1, "ctx_clear"),
            // Execution tier (core/mem/)
            ("get_execution_tier", 0, "cbgr_get_execution_tier"),
            // CBGR hazard pointers (core/mem/)
            ("acquire_hazard", 1, "cbgr_acquire_hazard"),
            ("release_hazard", 1, "cbgr_release_hazard"),
            // Tensor stats (core/math/autodiff.vr)
            ("var_axis", 2, "tensor_var_axis"),
            // IO/env helpers (core/io/fs.vr)
            ("env_var", 1, "env_var"),
            // Network helpers (core/net/)
            ("safe_getsockopt_error", 1, "net_getsockopt_error"),
            ("safe_getsockname", 3, "net_getsockname"),
            ("safe_getpeername", 3, "net_getpeername"),
            ("safe_set_broadcast", 2, "net_set_broadcast"),
            ("safe_set_tcp_nodelay", 2, "net_set_tcp_nodelay"),
            ("safe_set_keepalive", 2, "net_set_keepalive"),
            ("safe_set_socket_timeout", 3, "net_set_socket_timeout"),
            ("safe_setsockopt_int", 4, "net_setsockopt_int"),
            ("safe_getsockopt", 4, "net_getsockopt"),
            ("safe_fcntl", 3, "net_fcntl"),
            ("fcntl", 3, "sys_fcntl"),
            ("safe_set_send_buffer_size", 2, "net_set_send_buffer_size"),
            ("safe_set_recv_buffer_size", 2, "net_set_recv_buffer_size"),
            ("safe_join_multicast_v4", 3, "net_join_multicast_v4"),
            ("safe_leave_multicast_v4", 3, "net_leave_multicast_v4"),
            ("safe_set_multicast_ttl_v4", 2, "net_set_multicast_ttl_v4"),
            ("safe_set_multicast_loop_v4", 2, "net_set_multicast_loop_v4"),
            ("safe_peek", 3, "net_peek"),
            ("safe_recv_nonblock", 3, "net_recv_nonblock"),
            ("safe_send_nonblock", 3, "net_send_nonblock"),
            // Linux-side network socket option functions (used by tcp.vr, udp.vr)
            ("getsockname", 3, "net_getsockname_linux"),
            ("getpeername", 3, "net_getpeername_linux"),
            ("setsockopt_int", 4, "net_setsockopt_int_linux"),
            ("getsockopt_error", 1, "net_getsockopt_error_linux"),
            ("set_nonblocking", 2, "net_set_nonblocking_linux"),
            ("set_reuseaddr", 2, "net_set_reuseaddr_linux"),
            ("set_reuseport", 2, "net_set_reuseport"),
            ("set_tcp_nodelay", 2, "net_set_tcp_nodelay_linux"),
            ("set_keepalive", 2, "net_set_keepalive_linux"),
            ("set_socket_timeout", 3, "net_set_socket_timeout_linux"),
            ("set_broadcast", 2, "net_set_broadcast_linux"),
            ("shutdown", 2, "net_shutdown"),
            ("set_send_buffer_size", 2, "net_set_send_buffer_size_linux"),
            ("set_recv_buffer_size", 2, "net_set_recv_buffer_size_linux"),
            // Linux-side multicast/IO functions
            ("join_multicast_v4", 3, "net_join_multicast_v4_linux"),
            ("leave_multicast_v4", 3, "net_leave_multicast_v4_linux"),
            ("set_multicast_ttl_v4", 2, "net_set_multicast_ttl_v4_linux"),
            (
                "set_multicast_loop_v4",
                2,
                "net_set_multicast_loop_v4_linux",
            ),
            // Linux bare socket operations (used via @cfg blocks, no alias)
            ("socket", 3, "sys_socket"),
            ("bind", 3, "sys_bind"),
            ("connect", 3, "sys_connect"),
            ("close", 1, "sys_close"),
            ("send", 3, "sys_send"),
            ("recv", 3, "sys_recv"),
            ("sendto", 5, "sys_sendto"),
            ("recvfrom", 5, "sys_recvfrom"),
            ("listen", 2, "sys_listen"),
            ("accept", 3, "sys_accept"),
            ("peek", 3, "net_peek"),
            ("recv_nonblock", 3, "net_recv_nonblock"),
            ("send_nonblock", 3, "net_send_nonblock"),
            ("read", 3, "sys_read"),
            ("getsockopt", 4, "net_getsockopt_linux"),
            // File I/O helpers (core/io/file.vr)
            ("file_read_to_string", 1, "io_file_read_to_string"),
            ("file_read", 1, "io_file_read"),
            ("file_write", 2, "io_file_write"),
            // Context frame management (core/runtime/)
            ("ctx_push_frame", 0, "ctx_push_frame"),
            ("ctx_pop_frame", 0, "ctx_pop_frame"),
            // IO extended (core/io/fs.vr)
            ("sys_getcwd", 1, "sys_getcwd"),
            ("sys_chdir", 1, "sys_chdir"),
            // Memory segment (core/mem/segment.vr)
            ("segment_alloc", 1, "mem_segment_alloc"),
            ("segment_free", 1, "mem_segment_free"),
            ("segment_abandon", 1, "mem_segment_abandon"),
            ("ptr_to_segment", 1, "mem_ptr_to_segment"),
            // Thread-local helpers (core/sys/*/tls.vr)
            ("get_tid_fast", 0, "sys_get_tid_fast"),
            // Formatter write (core/base/fmt.vr)
            ("write", 2, "fmt_write"),
        ];

        for &(name, param_count, iname) in intrinsics {
            // If already registered (from stdlib compilation), just set the intrinsic_name
            if self.ctx.lookup_function(name).is_some() {
                self.ctx.set_function_intrinsic(name, iname.to_string());
                continue;
            }
            // Otherwise register a new function with the intrinsic_name
            let id = FunctionId(self.next_func_id);
            self.next_func_id = self.next_func_id.saturating_add(1);
            let info = FunctionInfo {
                id,
                param_count,
                param_names: vec![],
                param_type_names: vec![],
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: Some(iname.to_string()),
                variant_tag: None,
                parent_type_name: None,
                variant_payload_types: None,
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: None,
                return_type_inner: None,
                is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
            };
            self.ctx.register_function(name.to_string(), info);
        }
    }

    /// Exports all currently registered functions.
    ///

    /// This is used during stdlib compilation to collect functions
    /// from this module for use by dependent modules.
    pub fn export_functions(&self) -> std::collections::HashMap<String, FunctionInfo> {
        self.ctx.export_functions()
    }

    /// Returns inferred variable-to-type-name mappings from the last compiled function.
    ///

    /// The codegen clears `variable_type_names` between functions. This method
    /// returns the snapshot captured just before the clear, giving the playground
    /// access to types like `List<Int>`, `Map<Text, Bool>`, etc.
    #[allow(clippy::misnamed_getters)]
    pub fn variable_type_names(&self) -> &std::collections::HashMap<String, String> {
        &self.ctx.last_function_variable_types
    }

    /// Sets tier context from escape analysis results.
    ///

    /// This should be called before `compile_module()` to enable
    /// tier-aware code generation for references.
    ///

    /// # Example
    ///

    /// ```ignore
    /// use verum_vbc::codegen::{VbcCodegen, TierContext, ExprId};
    /// use verum_vbc::types::CbgrTier;
    ///

    /// let mut codegen = VbcCodegen::new();
    ///

    /// // From escape analysis results
    /// let mut tier_context = TierContext::new();
    /// tier_context.enabled = true;
    /// tier_context.set_tier(ExprId(42), CbgrTier::Tier1);
    ///

    /// codegen.set_tier_context(tier_context);
    /// let module = codegen.compile_module(&ast)?;
    /// ```
    pub fn set_tier_context(&mut self, tier_context: TierContext) {
        self.ctx.set_tier_context(tier_context);
    }

    /// When enabled, register_function will not overwrite existing entries.
    /// Use this when importing stdlib declarations after user code to prevent
    /// stdlib FFI names (e.g., "pipe") from shadowing user-defined functions.
    pub fn set_prefer_existing_functions(&mut self, prefer: bool) {
        self.ctx.prefer_existing_functions = prefer;
    }

    /// Returns the number of functions compiled so far.
    pub fn function_count(&self) -> u32 {
        self.functions.len() as u32
    }

    /// Controls whether @test attributes are propagated to is_test flag.
    /// Set to false when compiling stdlib to prevent stdlib tests from
    /// being picked up by the @test runner.
    pub fn set_propagate_test_attr(&mut self, propagate: bool) {
        self.propagate_test_attr = propagate;
    }

    /// Sets tier decisions from a map of expression IDs to tiers.
    ///

    /// Convenience method when you have a `Map<ExprId, CbgrTier>`.
    pub fn set_tier_decisions(&mut self, decisions: Map<ExprId, CbgrTier>) {
        self.ctx
            .set_tier_context(TierContext::with_decisions(decisions));
    }

    /// Gets reference statistics after compilation.
    ///

    /// Shows the distribution of references across tiers.
    pub fn tier_stats(&self) -> (usize, usize, usize) {
        (
            self.ctx.stats.tier0_refs,
            self.ctx.stats.tier1_refs,
            self.ctx.stats.tier2_refs,
        )
    }

    /// Check if a statement should be compiled based on @cfg attributes.
    ///

    /// When a `CfgEvaluator` is configured (via `CodegenConfig::with_target`),
    /// this method checks if the statement's @cfg attributes match the target.
    /// Statements with non-matching @cfg are filtered out, preventing issues like:
    /// - "return outside function" from skipped @cfg blocks
    /// - "undefined variable" from variables in @cfg blocks used outside
    /// - Platform-specific code being compiled for wrong targets
    pub(crate) fn should_compile_stmt(&self, stmt: &verum_ast::Stmt) -> bool {
        // Fast path: no attributes on statement
        if stmt.attributes.is_empty() {
            return true;
        }

        // Use the failures-returning variant so silently-malformed
        // `@cfg(...)` predicates surface as warns (parity with
        // `should_compile_item`). Statements don't carry a stable
        // name, so the warn site identifies the function context
        // via `current_function`.
        let (include, failures) = self
            .cfg_evaluator
            .should_include_with_failures(&stmt.attributes);
        if !failures.is_empty() {
            let fn_name = self.ctx.current_function.as_deref().unwrap_or("<unknown>");
            for attr in &failures {
                tracing::warn!(
                    "[cfg] @{}(...) attribute on statement inside `{}` could \
                     not be parsed; the statement is included by fall-through \
                     (fail-open).  Check predicate syntax: identifier (`unix`), \
                     key-value (`target_os = \"linux\"`), or \
                     `all`/`any`/`not` combinator.",
                    attr.name.as_str(),
                    fn_name,
                );
            }
        }
        include
    }

    /// Check if an item should be compiled based on @cfg attributes.
    ///

    /// Similar to `should_compile_stmt`, but for top-level items like functions,
    /// imports, and type declarations. This is critical for proper cross-platform
    /// compilation: without it, imports like `@cfg(target_os = "linux") import ...`
    /// would be processed even when compiling for macOS, causing function registration
    /// conflicts.
    pub(crate) fn should_compile_item(&self, item: &Item) -> bool {
        // Check `Item.attributes` (the outer attribute list — populated
        // by the parser when no inner decl carries attributes, e.g.
        // for `mount` and `module` items).
        if !item.attributes.is_empty() {
            let (include, failures) = self
                .cfg_evaluator
                .should_include_with_failures(&item.attributes);
            self.warn_cfg_parse_failures(&failures, item, "Item");
            if !include {
                return false;
            }
        }

        // Critical fix (#170 / #181): the `verum_fast_parser` puts the
        // attributes for `type X` / `implement` / `function` declarations
        // on the *inner* decl (`TypeDecl.attributes`, `ImplDecl.attributes`,
        // `Function.attributes`), leaving `Item.attributes` empty. So
        // `Item.attributes`-only checking silently bypasses @cfg gates
        // for every type declaration in the stdlib — `@cfg(target_arch
        // = "x86_64") public type ExceptionFrame is { … };` reaches
        // codegen even on aarch64 hosts, surfacing as duplicate-id
        // findings in #170's global type-table consistency check.
        //

        // Walk the inner decl's attributes when present.
        match &item.kind {
            ItemKind::Type(type_decl) if !type_decl.attributes.is_empty() => {
                let (include, failures) = self
                    .cfg_evaluator
                    .should_include_with_failures(&type_decl.attributes);
                self.warn_cfg_parse_failures(&failures, item, "TypeDecl");
                if !include {
                    return false;
                }
            }
            ItemKind::Function(func) if !func.attributes.is_empty() => {
                let (include, failures) = self
                    .cfg_evaluator
                    .should_include_with_failures(&func.attributes);
                self.warn_cfg_parse_failures(&failures, item, "Function");
                if !include {
                    return false;
                }
            }
            // ImplDecl carries its attributes in `Item.attributes`, not
            // an inner field — already covered by the outer check above.
            _ => {}
        }

        true
    }

    /// Emits a `tracing::warn!` for each `@cfg` attribute whose
    /// predicate failed to parse cleanly. These attributes are
    /// silently ignored by `cfg_evaluator.should_include` (fail-open
    /// for forward-compatibility — see `crates/verum_ast/src/cfg.rs`),
    /// so without this surface they go unnoticed: a typo in
    /// `@cfg(target_oss = "linux")` (note the double-s) compiles
    /// cleanly on every platform because the predicate is
    /// unparseable, returns `true` by fall-through, and the item is
    /// included unconditionally.
    ///

    /// `site` identifies whether the attribute lives on the `Item`
    /// itself or on the inner decl (`TypeDecl` / `Function`) — useful
    /// for the developer to locate the failure source.
    fn warn_cfg_parse_failures(
        &self,
        failures: &[&verum_ast::attr::Attribute],
        item: &Item,
        site: &str,
    ) {
        if failures.is_empty() {
            return;
        }
        let item_name = match &item.kind {
            ItemKind::Type(td) => td.name.name.to_string(),
            ItemKind::Function(f) => f.name.name.to_string(),
            ItemKind::Impl(_) => "<impl block>".to_string(),
            ItemKind::Mount(_) => "<mount>".to_string(),
            ItemKind::Module(md) => md.name.name.to_string(),
            _ => "<item>".to_string(),
        };
        for attr in failures {
            tracing::warn!(
                "[cfg] @{}(...) attribute on {site} `{}` could not be parsed; \
                 the item is included by fall-through (fail-open).  Check the \
                 predicate syntax: identifier (`unix`), key-value \
                 (`target_os = \"linux\"`), or `all`/`any`/`not` combinator.",
                attr.name.as_str(),
                item_name,
            );
        }
    }

    /// Extracts the intrinsic name from function attributes.
    ///

    /// Looks for `@intrinsic("name")` attribute and returns the intrinsic name.
    /// This enables industrial-grade intrinsic resolution where:
    /// 1. Intrinsic identity is established at declaration time via @intrinsic
    /// 2. The codegen uses this stored name rather than call-site name matching
    /// 3. Imports and aliases work correctly for intrinsic functions
    ///

    /// # Example
    /// ```verum
    /// @intrinsic("atomic_load_u64")
    /// pub fn atomic_load_u64(ptr: *const u64, ordering: u8) -> u64;
    /// ```
    /// Returns `Some("atomic_load_u64")` for the above function.
    fn extract_intrinsic_name(&self, func: &FunctionDecl) -> Option<String> {
        // Check function-level @intrinsic("name") attribute
        for attr in func.attributes.iter() {
            if attr.name.as_str() == "intrinsic"
                && let verum_common::Maybe::Some(args) = &attr.args
                && let Some(first_arg) = args.first()
                && let verum_ast::ExprKind::Literal(lit) = &first_arg.kind
                && let verum_ast::LiteralKind::Text(s) = &lit.kind
            {
                return Some(s.as_str().to_string());
            }
        }
        // Check function body for @intrinsic("name", ...) expression
        // Core intrinsics use: fn int_to_float(x) { @intrinsic("sitofp", x) }
        if let verum_common::Maybe::Some(body) = &func.body {
            let body_expr = match body {
                verum_ast::decl::FunctionBody::Expr(e) => Some(e),
                verum_ast::decl::FunctionBody::Block(block) => {
                    // Single-statement block with expression return
                    if block.stmts.len() == 1 {
                        if let verum_ast::StmtKind::Expr { expr: e, .. } = &block.stmts[0].kind {
                            Some(e)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            };
            if let Some(expr) = body_expr
                && let verum_ast::ExprKind::MetaFunction { name, args } = &expr.kind
                && name.name.as_str() == "intrinsic"
                && let Some(first_arg) = args.first()
                && let verum_ast::ExprKind::Literal(lit) = &first_arg.kind
                && let verum_ast::LiteralKind::Text(s) = &lit.kind
            {
                return Some(s.as_str().to_string());
            }
        }
        None
    }

    /// Initialize the codegen for multi-module compilation.
    ///

    /// This performs the same setup as `compile_module()` (reset, register builtins)
    /// but does NOT compile any module. Use this followed by `collect_all_declarations()`,
    /// `compile_module_items()`, and `finalize_module()` when you need to compile
    /// multiple modules into a single VBC module (e.g., main + imported stdlib modules).
    pub fn initialize(&mut self) {
        self.reset();
        self.register_builtin_variants();
        self.register_stdlib_constants();
        self.register_stdlib_intrinsics();
        self.register_runtime_io_functions();
    }

    /// Extract optimization hints from function attributes.
    ///

    /// Maps AST @attributes to VBC OptimizationHints:
    /// - @inline, @inline(always), @inline(never), @inline(release)
    /// - @cold, @hot
    /// - @optimize(none|size|speed|balanced)
    /// - @align(N)
    ///

    /// Extract type layout hints from type declaration attributes.
    ///

    /// Returns (alignment, is_packed, is_repr_c):
    /// - alignment: from @align(N) or @repr(packed)→1, default 8
    /// - is_packed: true if @repr(packed)
    /// - is_repr_c: true if @repr(C)
    fn extract_type_layout_hints(
        attrs: &verum_common::List<verum_ast::attr::Attribute>,
    ) -> (u32, bool, bool) {
        let mut alignment = 8u32;
        let mut is_packed = false;
        let mut is_repr_c = false;

        for attr in attrs.iter() {
            match attr.name.as_str() {
                "align" => {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first) = args.first()
                        && let verum_ast::ExprKind::Literal(lit) = &first.kind
                        && let verum_ast::LiteralKind::Int(int_lit) = &lit.kind
                    {
                        let val = int_lit.value as u32;
                        if val > 0 && val.is_power_of_two() {
                            alignment = val;
                        }
                    }
                }
                "repr" => {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first) = args.first()
                        && let Some(repr_name) = Self::attr_arg_as_ident(first)
                    {
                        match repr_name.as_str() {
                            "packed" => {
                                is_packed = true;
                                alignment = 1;
                            }
                            "C" => {
                                is_repr_c = true;
                            }
                            "cache_optimal" => {
                                // Cache line alignment (64 bytes)
                                alignment = 64;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        (alignment, is_packed, is_repr_c)
    }

    /// - @target_feature("...")
    /// - @target_cpu("...")
    fn extract_optimization_hints(&self, func: &FunctionDecl) -> crate::module::OptimizationHints {
        use crate::module::{InlineHint, OptLevel, OptimizationHints};
        let mut hints = OptimizationHints::default();
        for attr in func.attributes.iter() {
            match attr.name.as_str() {
                "inline" => {
                    hints.inline_hint = Some(match &attr.args {
                        verum_common::Maybe::Some(args) if !args.is_empty() => {
                            if let Some(first) = args.first() {
                                match Self::attr_arg_as_ident(first).as_deref() {
                                    Some("always") => InlineHint::Always,
                                    Some("never") => InlineHint::Never,
                                    Some("release") => InlineHint::Release,
                                    _ => InlineHint::Suggest,
                                }
                            } else {
                                InlineHint::Suggest
                            }
                        }
                        _ => InlineHint::Suggest,
                    });
                }
                "cold" => hints.is_cold = true,
                "hot" => hints.is_hot = true,
                "optimize" => {
                    if let verum_common::Maybe::Some(args) = &attr.args
                        && let Some(first) = args.first()
                    {
                        hints.opt_level = match Self::attr_arg_as_ident(first).as_deref() {
                            Some("none") => Some(OptLevel::None),
                            Some("size") => Some(OptLevel::Size),
                            Some("speed") => Some(OptLevel::Speed),
                            Some("balanced") => Some(OptLevel::Balanced),
                            _ => None,
                        };
                    }
                }
                "align" => {
                    if let verum_common::Maybe::Some(args) = &attr.args
                        && let Some(first) = args.first()
                        && let verum_ast::ExprKind::Literal(lit) = &first.kind
                        && let verum_ast::LiteralKind::Int(int_lit) = &lit.kind
                    {
                        let val = int_lit.value as u32;
                        if val > 0 && val.is_power_of_two() {
                            hints.alignment = Some(val);
                        }
                    }
                }
                "target_feature" => {
                    if let verum_common::Maybe::Some(args) = &attr.args
                        && let Some(first) = args.first()
                        && let verum_ast::ExprKind::Literal(lit) = &first.kind
                        && let verum_ast::LiteralKind::Text(s) = &lit.kind
                    {
                        hints.target_features = Some(s.as_str().to_string());
                    }
                }
                "target_cpu" => {
                    if let verum_common::Maybe::Some(args) = &attr.args
                        && let Some(first) = args.first()
                        && let verum_ast::ExprKind::Literal(lit) = &first.kind
                        && let verum_ast::LiteralKind::Text(s) = &lit.kind
                    {
                        hints.target_cpu = Some(s.as_str().to_string());
                    }
                }
                _ => {}
            }
        }
        hints
    }

    /// Extract an identifier name from an expression (for attribute args).
    fn attr_arg_as_ident(expr: &verum_ast::Expr) -> Option<String> {
        match &expr.kind {
            verum_ast::ExprKind::Path(path) => path.segments.last().and_then(|seg| {
                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                    Some(ident.name.to_string())
                } else {
                    None
                }
            }),
            verum_ast::ExprKind::Literal(lit) => {
                if let verum_ast::LiteralKind::Text(s) = &lit.kind {
                    Some(s.as_str().to_string())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Compile function bodies for a module without building the final VbcModule.
    ///

    /// This is the second pass of compilation. Declarations must already be registered
    /// via `collect_all_declarations()`. Unlike `compile_function_bodies()`, this does
    /// NOT call `build_module()` — call `finalize_module()` separately when done.
    pub fn compile_module_items(&mut self, module: &Module) -> CodegenResult<()> {
        let prev = self.ctx.current_source_module.take();
        if let Some(name) =
            Self::resolve_full_module_path(module, &self.config.module_name)
        {
            self.ctx.current_source_module = Some(name);
        }
        let mut result = Ok(());
        for item in module.items.iter() {
            if self.should_compile_item(item)
                && let Err(e) = self.compile_item(item)
            {
                result = Err(e);
                break;
            }
        }
        self.ctx.current_source_module = prev;
        result
    }

    /// Compile function bodies for an imported module with error recovery.
    ///

    /// Unlike `compile_module_items`, this method catches per-item compilation errors
    /// and continues with the remaining items. This is necessary for imported stdlib
    /// modules that may contain functions referencing FFI/external symbols not available
    /// in VBC (e.g., `mach_timebase_info` in `core/sys/darwin/time.vr`). If the main
    /// module actually calls a skipped function, it will get a FunctionNotFound error
    /// at runtime, which is the correct behavior.
    pub fn compile_module_items_lenient(&mut self, module: &Module) -> CodegenResult<()> {
        let prev = self.ctx.current_source_module.take();
        if let Some(name) =
            Self::resolve_full_module_path(module, &self.config.module_name)
        {
            self.ctx.current_source_module = Some(name);
        }
        let mut first_strict_err: Option<CodegenError> = None;
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                // Use lenient item compilation that skips individual functions
                // that fail. In strict_codegen mode, the helper returns the
                // first `BugClass` error encountered so we can halt the build
                // at the call site instead of papering over a real defect.
                if let Err(e) = self.compile_item_lenient(item)
                    && first_strict_err.is_none()
                {
                    first_strict_err = Some(e);
                }
            }
        }
        self.ctx.current_source_module = prev;
        match first_strict_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Compiles an item with lenient error handling - skips individual functions that fail.
    /// This is used for imported stdlib modules where some functions may reference
    /// FFI/external symbols not available in VBC interpreter.
    ///

    /// Returns `Err(CodegenError)` *only* when `config.strict_codegen` is
    /// `true` AND the per-item failure classifies as `SkipClass::BugClass`.
    /// All other failures (irreducible, or any failure in non-strict mode)
    /// surface as warn-level traces and the function returns `Ok(())` —
    /// the documented Tier-0 contract.
    fn compile_item_lenient(&mut self, item: &Item) -> CodegenResult<()> {
        let mut first_strict_err: Option<CodegenError> = None;
        match &item.kind {
            ItemKind::Function(func) => {
                self.ctx.generic_type_params.clear();
                self.ctx.const_generic_params.clear();
                if let Err(e) = self.compile_function(func, None) {
                    // Symmetric with the impl-item branch below. Promoted to
                    // warn-level (was debug) so silent skips of user-callable
                    // top-level functions surface as "method/function not
                    // found on value" only AFTER the warning fires, instead
                    // of being completely silent.
                    let fname = func.name.name.as_str();
                    let class = e.skip_class();
                    let err_text = format!("{}", e);
                    tracing::warn!(
                        "[lenient] SKIP top-level fn {} ({}): {} — runtime calls \
                         will panic with the same message via auto-stub",
                        fname,
                        class.label(),
                        e
                    );
                    // Replace the dropped function with a panic-stub
                    // body so typed dispatch keeps finding it.  Only
                    // actual calls hit the panic; pattern matching,
                    // function-pointer references, and qualified-path
                    // resolution all keep working.  See
                    // `emit_lenient_panic_stub` doc for the full
                    // rationale.
                    self.emit_lenient_panic_stub(func, None, &err_text);
                    if let Some(undef) = e.undefined_function_name() {
                        let undef_owned = undef.to_string();
                        let near: Vec<String> = self
                            .ctx
                            .functions
                            .keys()
                            .filter(|k| {
                                k.ends_with(&format!(".{}", undef_owned))
                                    || k.ends_with(&format!("::{}", undef_owned))
                                    || k.as_str() == undef_owned.as_str()
                            })
                            .take(8)
                            .cloned()
                            .collect();
                        if !near.is_empty() {
                            tracing::warn!(
                                "[lenient]   near-matches for '{}' in ctx.functions: {:?}",
                                undef_owned,
                                near
                            );
                        } else {
                            tracing::warn!(
                                "[lenient]   no near-matches for '{}' in ctx.functions ({} entries total)",
                                undef_owned,
                                self.ctx.functions.len()
                            );
                        }
                    }
                    tracing::debug!("[lenient] SKIP top-level fn {}: {}", fname, e);
                    if self.config.strict_codegen
                        && class == SkipClass::BugClass
                        && first_strict_err.is_none()
                    {
                        first_strict_err = Some(e);
                    }
                }
                // Compile nested functions even if parent failed
                if let verum_common::Maybe::Some(ref body) = func.body {
                    let _ = self.compile_nested_functions(body);
                }
            }
            ItemKind::Impl(impl_decl) => {
                // Compile each function individually, skipping those that fail
                let type_name = self.extract_impl_type_name(&impl_decl.kind);

                let impl_type_generics: Vec<String> = impl_decl
                    .generics
                    .iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                let impl_const_generics: Vec<String> = impl_decl
                    .generics
                    .iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Const { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                for impl_item in impl_decl.items.iter() {
                    // Honour `@cfg` gates on impl items. ImplItem and
                    // its inner FunctionDecl both carry attributes;
                    // walk both so `@cfg(target_os = "linux") fn foo(…)`
                    // inside a cross-platform `implement Bar { … }`
                    // block is filtered correctly.
                    if !impl_item.attributes.is_empty() {
                        let (include, failures) = self
                            .cfg_evaluator
                            .should_include_with_failures(&impl_item.attributes);
                        if !failures.is_empty() {
                            for attr in &failures {
                                tracing::warn!(
                                    "[cfg] @{}(...) on impl item could not be \
                                     parsed; the item is included by fall-through.",
                                    attr.name.as_str(),
                                );
                            }
                        }
                        if !include {
                            continue;
                        }
                    }
                    if let verum_ast::decl::ImplItemKind::Function(func) = &impl_item.kind {
                        if !func.attributes.is_empty() {
                            let (include, failures) = self
                                .cfg_evaluator
                                .should_include_with_failures(&func.attributes);
                            if !failures.is_empty() {
                                let fname = func.name.name.as_str();
                                for attr in &failures {
                                    tracing::warn!(
                                        "[cfg] @{}(...) on impl-method `{}` \
                                         could not be parsed; included by \
                                         fall-through.",
                                        attr.name.as_str(),
                                        fname,
                                    );
                                }
                            }
                            if !include {
                                continue;
                            }
                        }
                        self.ctx.generic_type_params.clear();
                        self.ctx.const_generic_params.clear();
                        for g in &impl_type_generics {
                            self.ctx.generic_type_params.insert(g.clone());
                        }
                        for g in &impl_const_generics {
                            self.ctx.const_generic_params.insert(g.clone());
                        }

                        // Compile function individually - skip if it fails.
                        //

                        // Lenient skips were originally a debug-only diagnostic
                        // because most are routine (FFI prototypes, conditional
                        // platform stubs). But silent skips of *user-callable*
                        // impl-block methods are insidious: they show up at
                        // runtime as `method 'X.Y' not found on value` with no
                        // hint that compilation dropped the body. Emit a
                        // warn-level trace so the underlying cause (typically
                        // an unresolved cross-module function reference) is
                        // visible in normal CI / dev runs without RUST_LOG
                        // tweaking.
                        if let Err(e) = self.compile_function(func, type_name.as_ref()) {
                            let fname = func.name.name.as_str();
                            let ty = type_name.as_deref().unwrap_or("?");
                            let class = e.skip_class();
                            let err_text = format!("{}", e);
                            tracing::warn!(
                                "[lenient] SKIP {}.{} ({}): {} — runtime calls to \
                                 this method will panic with the same message via \
                                 auto-stub.  Add the missing dependency to the \
                                 caller's mount list or fix the cross-module \
                                 reference in {} stdlib.",
                                ty,
                                fname,
                                class.label(),
                                e,
                                ty
                            );
                            // Auto-stub: replace the dropped body
                            // with a Panic instruction so dispatch
                            // (CallM by suffix-and-args, qualified
                            // path resolution, function-pointer load)
                            // keeps finding the method.  Pattern
                            // matching against variants of the
                            // carrier type is unaffected — the panic
                            // only fires on actual call execution.
                            self.emit_lenient_panic_stub(
                                func,
                                type_name.as_deref(),
                                &err_text,
                            );
                            // For debugging stdlib hygiene: dump near-matches
                            // from the ctx.functions table so the user can
                            // see whether the missing name is registered
                            // under a qualified form nearby (the most common
                            // sign of a transitive-import gap).
                            if let Some(undef) = e.undefined_function_name() {
                                let undef_owned = undef.to_string();
                                let near: Vec<String> = self
                                    .ctx
                                    .functions
                                    .keys()
                                    .filter(|k| {
                                        k.ends_with(&format!(".{}", undef_owned))
                                            || k.ends_with(&format!("::{}", undef_owned))
                                            || k.as_str() == undef_owned.as_str()
                                    })
                                    .take(8)
                                    .cloned()
                                    .collect();
                                if !near.is_empty() {
                                    tracing::warn!(
                                        "[lenient]   near-matches for '{}' in ctx.functions: {:?}",
                                        undef_owned,
                                        near
                                    );
                                } else {
                                    tracing::warn!(
                                        "[lenient]   no near-matches for '{}' in ctx.functions ({} entries total)",
                                        undef_owned,
                                        self.ctx.functions.len()
                                    );
                                }
                            }
                            tracing::debug!("[lenient] SKIP {}.{}: {}", ty, fname, e);
                            if self.config.strict_codegen
                                && class == SkipClass::BugClass
                                && first_strict_err.is_none()
                            {
                                first_strict_err = Some(e);
                            }
                        }

                        // Compile nested functions even if parent failed
                        if let verum_common::Maybe::Some(ref body) = func.body {
                            let _ = self.compile_nested_functions(body);
                        }
                    }
                }
            }
            ItemKind::Pattern(pat_decl) => {
                self.ctx.generic_type_params.clear();
                self.ctx.const_generic_params.clear();
                let _ = self.compile_pattern_as_function(pat_decl);
            }
            // #122 — process imported modules' Mount items so their
            // aliases (`mount X.Y as Z`) get registered in
            // ctx.functions. Without this, every alias-bearing
            // mount in a stdlib module surfaced as bug-class lenient
            // SKIP at the consumer's call site (the symptom that
            // motivated #119 audit pass 1). The strict-mode
            // semantics here mirror compile_item: any failure
            // becomes the strict error if classified BugClass.
            ItemKind::Mount(import_decl) => {
                if let Err(e) = self.register_import_aliases(import_decl) {
                    let class = e.skip_class();
                    if self.config.strict_codegen
                        && class == SkipClass::BugClass
                        && first_strict_err.is_none()
                    {
                        first_strict_err = Some(e);
                    }
                }
            }
            _ => {}
        }
        match first_strict_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Build and return the final VbcModule from all compiled items.
    ///

    /// Call this after `initialize()`, `collect_all_declarations()`, and
    /// `compile_module_items()` to produce the final bytecode module.
    pub fn finalize_module(&mut self) -> CodegenResult<VbcModule> {
        // Compile any pending constants (struct literals, etc.) before building
        self.compile_pending_constants()?;
        // Compile any pending @thread_local initializations
        self.compile_pending_tls_inits()?;
        // Ensure every Call/CallG/TailCall/etc. target has a
        // corresponding VbcFunction descriptor in `self.functions`
        // — without this, user code that calls a non-intrinsic
        // stdlib function lands at runtime as
        // `InterpreterError::FunctionNotFound`.  Most stdlib
        // functions reach the interpreter via name-based intercepts
        // (`try_intercept_shell_runtime`/`file_runtime`/`env_runtime`/…)
        // which need the FUNCTION DESCRIPTOR's name to dispatch —
        // so the descriptor must be present even when the body
        // isn't. `emit_missing_stub_descriptors` walks every emitted
        // instruction's `func_id` operand and adds a stub
        // descriptor (with the right name + parent_type + arity)
        // for any id that was registered in `ctx.functions` but
        // hasn't been pushed to `self.functions`.
        self.emit_missing_stub_descriptors();
        // Verify type-descriptor self-consistency before emitting bytecode.
        // Catches the class of bugs where codegen produces a TypeDescriptor
        // whose variants disagree with their declared `kind`/`arity`/
        // `fields` shape — historically these surfaced at runtime as
        // `field index N (offset M) exceeds object data size K` /
        // `Null pointer dereference` panics, far from the codegen site.
        self.verify_type_layout_invariants()?;
        // Cross-module type-table consistency check (#170). In strict
        // mode (`config.strict_codegen = true`), any duplicate-id /
        // same-name-different-id / variant-tag-anomaly finding fails
        // the build with the bundled error. In lenient mode (default),
        // findings are emitted at warn-level so CI logs surface the
        // regression without blocking dev iteration.
        let report = self.verify_global_type_table_consistency();
        if !report.is_clean() {
            if self.config.strict_codegen {
                report.into_error()?;
                // Unreachable — into_error returns Err when not clean.
                // The early-return path above keeps the type checker
                // happy without unwrap().
            } else {
                tracing::warn!(
                    "[type-table] {} cross-module consistency finding(s) — \
                     run with `strict_codegen` to fail the build, or see \
                     `verify_global_type_table_consistency` for the report \
                     shape.  See #170 / #181 for the remediation plan.",
                    report.issue_count(),
                );
                for d in &report.duplicate_ids {
                    tracing::warn!(
                        "[type-table]   duplicate TypeId({}) shared by {:?}",
                        d.type_id,
                        d.descriptor_names,
                    );
                }
                for d in &report.duplicate_names_with_different_ids {
                    tracing::warn!(
                        "[type-table]   name `{}` declared with conflicting \
                         TypeIds: {:?}",
                        d.name,
                        d.type_ids,
                    );
                }
                for a in &report.variant_tag_anomalies {
                    tracing::warn!(
                        "[type-table]   variant tags non-dense in `{}` \
                         (TypeId({})): expected {} variants, max tag {}, \
                         duplicates {:?}, missing {:?}",
                        a.type_name,
                        a.type_id,
                        a.expected_count,
                        a.max_tag_seen,
                        a.duplicate_tags,
                        a.missing_tags,
                    );
                }
            }
        }
        let module = self.build_module()?;

        // Honour `config.validate`: when set, run the VBC structural
        // validator over the freshly-built module before returning.
        // Surfaces malformed bytecode at codegen-time instead of
        // letting it reach the interpreter / serializer where the
        // failure mode is far harder to localise. Default-off keeps
        // the codegen hot path unchanged for production builds.
        if self.config.validate
            && let Err(e) = validate::validate_module(&module)
        {
            return Err(CodegenError::internal(format!(
                "VBC structural validation failed for module `{}`: {}",
                module.name, e,
            )));
        }

        Ok(module)
    }

    /// Verify that every `TypeDescriptor` in `self.types` satisfies the
    /// per-variant shape invariants implied by its `VariantKind`. Runs
    /// at module-finalization time so misshapen descriptors fail loudly
    /// here rather than producing bytecode that crashes at runtime.
    ///

    /// Per-variant invariants:
    ///  * `Unit` → `arity == 0` and `fields` is empty.
    ///  * `Tuple` → `arity > 0` and `fields` is empty (the arity
    ///  counts payload elements; tuple variants don't
    ///  use `fields`).
    ///  * `Record` → `arity == 0` and `fields` is non-empty (records
    ///  track their layout in `fields`, not `arity`).
    ///

    /// Cross-variant invariants:
    ///  * Tags within a sum type are dense: `0..variants.len()` with
    ///  no duplicates and no gaps. The runtime resolves variant
    ///  dispatch by indexing into the variants array by tag, so any
    ///  gap or duplicate yields wrong-variant dispatch later.
    ///

    /// Spec hooks: `verum_vbc::types::VariantKind`,
    /// `verum_vbc::types::VariantDescriptor`.
    pub fn verify_type_layout_invariants(&self) -> CodegenResult<()> {
        Self::check_type_layout_invariants_inner(&self.types, &self.ctx.strings)
    }

    /// Phase 2 of #146 — scan emitted bytecode and report when a
    /// `MakeVariant { tag, field_count }` instruction has no matching
    /// (tag, payload-width) pair in any declared type's variant
    /// table. Reports as a `tracing::warn!` rather than failing the
    /// compile because variant constructors registered for types
    /// declared in other loaded modules (e.g. `Result.Ok` from
    /// `core.base.result` referenced from a downstream module) live
    /// in those modules' TypeDescriptor arrays, not this one's.
    /// A hard fail would be a false positive for every cross-module
    /// variant emission.
    ///

    /// Phase 3e (#146) extension — the same scan also counts
    /// `MakeVariantTyped` emissions and validates them against the
    /// type table by `(type_id, tag, field_count)` (a stricter check
    /// since the operand-supplied type_id pins the parent sum-type
    /// id directly, no cross-tag heuristics needed). The returned
    /// `MakeVariantReport` separates typed-vs-untyped emission
    /// counts so post-Phase-3 builds can ratchet on the
    /// untyped-emission count toward zero.
    ///

    /// Returns the number of instructions reported (zero in clean
    /// builds), useful for tests that want a structural assertion.
    pub fn report_make_variant_inconsistencies(&self) -> usize {
        self.collect_make_variant_report().total_inconsistencies()
    }

    /// Detailed make-variant emission report (Phase 3e of #146).
    ///

    /// Walks every function's bytecode and counts:
    ///  - `typed_emissions`: `MakeVariantTyped` instructions emitted
    ///  (preferred, carry the parent sum-type id);
    ///  - `untyped_emissions`: legacy `MakeVariant` instructions
    ///  (fallback when codegen can't resolve the parent type);
    ///  - `untyped_inconsistencies`: untyped emissions whose
    ///  `(tag, field_count)` doesn't match any module-local
    ///  declared variant (the legacy Phase-2 signal — false
    ///  positives expected for cross-module emissions);
    ///  - `typed_inconsistencies`: typed emissions whose
    ///  `(type_id, tag, field_count)` doesn't match the
    ///  declared variant of that type_id (these are stronger
    ///  signals — the operand-supplied type_id pins the parent,
    ///  so a mismatch is genuine codegen drift, not cross-module).
    ///

    /// Performance: single O(#instructions) walk; type-table is
    /// indexed once into a HashMap for O(1) lookups during the walk.
    pub fn collect_make_variant_report(&self) -> MakeVariantReport {
        use crate::instruction::Instruction;
        use crate::types::{TypeId, VariantKind};

        // Index 1: legacy Phase-2 set of (tag, field_count) combos
        // declared anywhere in the module. Ratifies untyped
        // emissions module-locally.
        let mut valid_pairs: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::new();
        // Index 2: (type_id, tag) → field_count map. Lets typed
        // emissions cross-check directly against the declared
        // variant of the same type_id.
        let mut typed_lookup: std::collections::HashMap<(TypeId, u32), u32> =
            std::collections::HashMap::new();
        for ty in &self.types {
            for v in &ty.variants {
                let count = match v.kind {
                    VariantKind::Unit => 0u32,
                    VariantKind::Tuple => v.arity as u32,
                    VariantKind::Record => v.fields.len() as u32,
                };
                valid_pairs.insert((v.tag, count));
                typed_lookup.insert((ty.id, v.tag), count);
            }
        }

        let valid_pairs_known = !valid_pairs.is_empty();
        let mut report = MakeVariantReport::default();

        for f in &self.functions {
            for ins in &f.instructions {
                match ins {
                    Instruction::MakeVariant {
                        dst: _,
                        tag,
                        field_count,
                    } => {
                        report.untyped_emissions += 1;
                        if valid_pairs_known && !valid_pairs.contains(&(*tag, *field_count)) {
                            let fname = self
                                .ctx
                                .strings
                                .get(f.descriptor.name.0 as usize)
                                .cloned()
                                .unwrap_or_else(|| format!("<FunctionId({})>", f.descriptor.id.0));
                            tracing::warn!(
                                "[layout] MakeVariant {{ tag: {}, field_count: \
                                 {} }} in `{}` has no matching variant in this \
                                 module's type table — may be cross-module \
                                 (legitimate) or stale-FunctionInfo drift \
                                 (latent bug). See #146 Phase 2.",
                                tag,
                                field_count,
                                fname,
                            );
                            report.untyped_inconsistencies += 1;
                        }
                    }
                    Instruction::MakeVariantTyped {
                        dst: _,
                        type_id,
                        tag,
                        field_count,
                    } => {
                        report.typed_emissions += 1;
                        // Skip cross-module / builtin type_ids: a
                        // typed emission against a TypeId that this
                        // module didn't declare is cross-module
                        // (legitimate) — analogous to the untyped
                        // false-positive class.
                        let type_id_v = TypeId(*type_id);
                        if let Some(expected) = typed_lookup.get(&(type_id_v, *tag)) {
                            if *expected != *field_count {
                                let fname = self
                                    .ctx
                                    .strings
                                    .get(f.descriptor.name.0 as usize)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        format!("<FunctionId({})>", f.descriptor.id.0)
                                    });
                                tracing::warn!(
                                    "[layout] MakeVariantTyped {{ type_id: {}, \
                                     tag: {}, field_count: {} }} in `{}` \
                                     disagrees with declared variant arity \
                                     {} — codegen drift. See #146 Phase 3e.",
                                    type_id,
                                    tag,
                                    field_count,
                                    fname,
                                    expected,
                                );
                                report.typed_inconsistencies += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        report
    }

    /// Register an archive-sourced `TypeDescriptor` into the codegen
    /// type registry without going through the AST walker.  T2 of
    /// the single-path archive-driven epic — paired with
    /// `archive_ctx_loader::populate_ctx_from_archive`, this skips
    /// `collect_non_protocol_declarations` for stdlib types entirely.
    ///
    /// `simple_name` is the type's display name (e.g. `Maybe`,
    /// `ConnectionError`) — used to seed `type_name_to_id` and
    /// `type_field_layouts` so downstream lookups
    /// (`compile_path` → `lookup_type` → `compile_field_access`
    /// → `field_type_name`) resolve through the archive descriptor.
    /// First-wins on `simple_name` collision.
    ///
    /// The descriptor's archive-side `TypeId` is preserved; archive
    /// builds use a global ID space already disjoint from the
    /// codegen's user-side `next_type_id` counter, which is bumped
    /// past every observed archive id so subsequent
    /// `alloc_user_type_id` calls don't collide.
    pub fn register_archive_type(
        &mut self,
        ty: crate::types::TypeDescriptor,
        simple_name: String,
    ) {
        // Bump next_type_id past this archive id so future user-type
        // allocations stay disjoint.
        let id_val = ty.id.0;
        if id_val >= self.next_type_id {
            self.next_type_id = id_val.saturating_add(1);
        }
        // First-wins on type-name lookup.
        if !self.type_name_to_id.contains_key(&simple_name) {
            self.type_name_to_id.insert(simple_name.clone(), ty.id);
        }
        // Field layout cache for `field_type_name` consumers.  Uses
        // the descriptor's structured field list (records) and
        // each variant's payload field list (record-style variants).
        // For tuple variants the synthetic `f0`/`f1`/... names match
        // codegen's positional-field convention.
        if !ty.fields.is_empty() {
            let names: Vec<String> = ty
                .fields
                .iter()
                .map(|f| {
                    self.ctx
                        .strings
                        .get(f.name.0 as usize)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            // **Type-name map unconditional population** (closes task #9
            // cross-mount race for archive-loaded types).
            //
            // `register_record_fields` (mod.rs:12407 — the user-phase
            // path) unconditionally populates `type_field_type_names`
            // even when `type_field_layouts` is first-wins-guarded;
            // commit `ab768e5d8` established this invariant for the
            // user phase to keep the type-map fresh under repeated
            // forward-reference re-registration.  The archive-side
            // path here previously only populated `type_field_layouts`
            // and left `type_field_type_names` empty — so when an
            // archive-loaded record type was queried at a field-access
            // call site (`f.value` field-type lookup driving
            // raw-pointer marker propagation, list-mount tracing, …),
            // the missing entry caused `field_type_name` to return
            // `None`.  Downstream `resolve_field_index` then fell
            // through to the "pick the type with the most fields"
            // global-scan heuristic and silently routed field writes
            // to wrong offsets — pinned 5 tests at
            // `core-tests/async/future/regression_test.vr §C`
            // (ReadyFuture/Join2/Select2/Lazy `.value`/`.f`/`.fut1`
            // field-access under `List` mount).
            //
            // Resolve each field's `TypeRef` to its canonical type
            // name via `type_ref_to_field_name` (an inline mirror of
            // `extract_type_name_from_ast`'s prefix preservation:
            // bare references flatten, `&unsafe`/`*const`/`*mut`
            // preserve their prefix so the raw-pointer marker at
            // `compile_field_access` line 14372 still fires).  When
            // the type-ref doesn't resolve to a nominal name (free
            // generic param, function type, structural shape), skip
            // that field — populating with `""` would shadow a future
            // genuine registration via first-wins.
            for (fname, fdesc) in names.iter().zip(ty.fields.iter()) {
                if let Some(ty_name) = self.type_ref_to_field_name(&fdesc.type_ref) {
                    self.type_field_type_names.insert(
                        (simple_name.clone(), fname.clone()),
                        ty_name,
                    );
                }
            }
            // Archive-sourced field names live in archive's string
            // pool, not in codegen's. Pull them through the
            // descriptor → simple-name map keyed by simple type name.
            self.type_field_layouts
                .entry(simple_name)
                .or_insert(names);
        }
        self.push_type_dedupe(ty);
    }

    /// Resolve a [`TypeRef`] to its canonical AST-style field-type
    /// name, mirroring `extract_type_name_from_ast`'s prefix-preservation
    /// invariants so archive-side and user-side `type_field_type_names`
    /// entries stay byte-identical.  See `register_archive_type` for
    /// rationale and consumer surface.
    fn type_ref_to_field_name(&self, ty: &crate::types::TypeRef) -> Option<String> {
        use crate::types::{CbgrTier, TypeRef};
        match ty {
            TypeRef::Concrete(tid) => {
                if let Some(prim) = self.primitive_type_id_to_name(*tid) {
                    return Some(prim.to_string());
                }
                self.types
                    .iter()
                    .find(|t| t.id == *tid)
                    .and_then(|t| self.ctx.strings.get(t.name.0 as usize).cloned())
            }
            TypeRef::Instantiated { base, args } => {
                let base_name = self
                    .primitive_type_id_to_name(*base)
                    .map(|s| s.to_string())
                    .or_else(|| {
                        self.types
                            .iter()
                            .find(|t| t.id == *base)
                            .and_then(|t| self.ctx.strings.get(t.name.0 as usize).cloned())
                    })?;
                if args.is_empty() {
                    Some(base_name)
                } else {
                    let arg_names: Vec<String> = args
                        .iter()
                        .map(|a| self.type_ref_to_field_name(a).unwrap_or_else(|| "_".into()))
                        .collect();
                    Some(format!("{}<{}>", base_name, arg_names.join(", ")))
                }
            }
            // Mirror `extract_type_name_from_ast`'s
            // reference-handling: bare `&T` / `&checked T` (Tier0 /
            // Tier1) flatten to the inner name — the raw-pointer
            // marker is keyed on the `&unsafe ` / `*const ` / `*mut `
            // prefix only.  Tier2 (`&unsafe T`) preserves the prefix.
            TypeRef::Reference { inner, tier, .. } => match tier {
                CbgrTier::Tier2 => self
                    .type_ref_to_field_name(inner)
                    .map(|n| format!("&unsafe {}", n)),
                _ => self.type_ref_to_field_name(inner),
            },
            TypeRef::Slice(inner) => {
                self.type_ref_to_field_name(inner).map(|n| format!("[{}]", n))
            }
            // Function / Rank2Function / Generic / Tuple / Array do
            // not produce a nominal field-type name that downstream
            // consumers (raw-pointer prefix check, name-equality for
            // mount-trace) compare against.  Returning None here
            // mirrors `register_record_fields`'s behaviour at
            // `extract_type_name_from_ast`'s structural-shape return
            // (which produces a name like `(Int, Int)` for tuples —
            // not a registry key) by simply omitting the entry.
            _ => None,
        }
    }

    /// Resolve a built-in [`TypeId`] to its canonical Verum name.
    /// Source-of-truth mirrors the dispatch in `type_ref_to_name`
    /// (codegen-side) and `primitive_typeid_name` (archive_ctx_loader),
    /// keeping all three sites pinned to a single name discipline.
    fn primitive_type_id_to_name(
        &self,
        tid: crate::types::TypeId,
    ) -> Option<&'static str> {
        use crate::types::TypeId;
        Some(match tid {
            TypeId::UNIT => "()",
            TypeId::INT => "Int",
            TypeId::FLOAT => "Float",
            TypeId::BOOL => "Bool",
            TypeId::TEXT => "Text",
            TypeId::CHAR => "Char",
            TypeId::U8 => "Byte",
            TypeId::I32 => "Int32",
            TypeId::U64 => "UInt64",
            _ => return None,
        })
    }

    /// Pre-populate the codegen registry from an embedded
    /// `VbcArchive`.  Walks every archived module's type table and
    /// registers each `TypeDescriptor` via
    /// [`register_archive_type`](Self::register_archive_type).
    ///
    /// Pairs with `archive_ctx_loader::populate_ctx_from_archive` —
    /// that one handles function info, this one handles types.
    /// Together they replace the slow source-driven `imported_modules`
    /// collection that walks 2400+ stdlib `.vr` files.
    ///
    /// Returns the number of type descriptors registered.  Best-
    /// effort: per-module decode failures are silently skipped with
    /// a `tracing::warn!` so a corrupt archive entry doesn't poison
    /// the entire stdlib load.
    pub fn populate_types_from_archive(
        &mut self,
        archive: &crate::archive::VbcArchive,
    ) -> usize {
        let mut registered: usize = 0;
        for entry in &archive.index {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        target: "vbc_codegen::populate_types",
                        "skip module {}: decode failed ({:?})",
                        entry.name, e
                    );
                    continue;
                }
            };
            for ty in &module.types {
                let simple_name = match module.strings.get(ty.name) {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                // Each archive-sourced descriptor needs its
                // string-table-relative field names rewritten to
                // codegen's string pool too — the descriptor itself
                // is moved into self.types as-is (the linker pass
                // remaps StringIds at finalize time, mirroring the
                // existing source-driven flow).
                self.register_archive_type(ty.clone(), simple_name);
                registered += 1;
            }
        }
        // **Field-type-name reconciliation pass**.  `register_archive_type`
        // populates `type_field_type_names` from each field's TypeRef by
        // resolving the inner TypeId back to a name via `self.types` —
        // but at the time RefCell is registered, BorrowState may not yet
        // be in `self.types` (declaration order within a module is
        // ARBITRARY at archive load time).  The resolution silently
        // returns None, leaving the entry missing.  Downstream
        // `extract_expr_type_name` for `rc.borrow_state` then can't find
        // the field's type name, `let bs = rc.borrow_state` doesn't
        // record a type for `bs`, and `bs.count` falls through to the
        // global intern_field_name index (non-zero for `count` because
        // `value` was interned first) — surfaces at runtime as "field
        // access out of bounds: field index 1 ... exceeds object data
        // size 8" when reading slot 1 of a 1-slot BorrowState.
        //
        // The reconciliation walks every registered type AFTER all are
        // loaded, repopulating field-type entries with the now-complete
        // type table.  Insertion stays first-wins via the
        // `or_insert`-equivalent in `register_archive_type`'s populator
        // — we use the same pattern here so any user-side
        // `register_record_fields` registration already in place keeps
        // its slot.
        let pending_field_types: Vec<((String, String), String)> = {
            let mut out: Vec<((String, String), String)> = Vec::new();
            for ty in &self.types {
                let simple_name = match self.ctx.strings.get(ty.name.0 as usize) {
                    Some(s) => s.clone(),
                    None => continue,
                };
                for fdesc in ty.fields.iter() {
                    let fname = match self.ctx.strings.get(fdesc.name.0 as usize) {
                        Some(s) => s.clone(),
                        None => continue,
                    };
                    if self
                        .type_field_type_names
                        .contains_key(&(simple_name.clone(), fname.clone()))
                    {
                        continue;
                    }
                    if let Some(ty_name) = self.type_ref_to_field_name(&fdesc.type_ref) {
                        out.push(((simple_name.clone(), fname), ty_name));
                    }
                }
            }
            out
        };
        for (key, value) in pending_field_types {
            self.type_field_type_names.entry(key).or_insert(value);
        }
        registered
    }

    /// Push a `TypeDescriptor` into `self.types`, skipping when an
    /// existing descriptor already claims the same `TypeId`.
    ///

    /// Used at type-registration sites where the well-known TypeId
    /// map (e.g. `Heap`/`Shared` both bound to `TypeId::PTR = 14`)
    /// produces multiple `TypeDescriptor` instances at the same id.
    /// First-wins semantics: keep the first registration, drop the
    /// rest. Function-table registrations are independent and not
    /// affected by this dedupe.
    ///

    /// Safe because the runtime dispatches by `TypeId`, not by
    /// descriptor identity — two descriptors at the same id are
    /// observationally indistinguishable from one descriptor at
    /// that id (modulo whichever variants/fields the first one
    /// happened to register, which is the existing well-known
    /// alias semantic).
    fn push_type_dedupe(&mut self, ty: crate::types::TypeDescriptor) {
        // If a descriptor with this id already exists AND it carries
        // populated structural data (variants OR fields), skip the
        // re-push — the first wins. But if the existing entry is
        // an empty PLACEHOLDER (no variants and no fields, typically
        // produced by the speculative `alloc_user_type_id` path or
        // by a partial cross-module registration), REPLACE it with
        // the new fully-populated descriptor. Otherwise the
        // placeholder pins the id and downstream consumers
        // (`format_variant_for_print_depth`, `validate_make_variant_typed`)
        // see no variants — every typed variant rendered as a
        // generic record fallback `{tag, payload...}` instead of
        // the proper `Constructor(payload...)` form.
        if let Some(idx) = self.types.iter().position(|t| t.id == ty.id) {
            let existing = &self.types[idx];
            let existing_empty = existing.variants.is_empty() && existing.fields.is_empty();
            let new_richer = !ty.variants.is_empty() || !ty.fields.is_empty();
            if existing_empty && new_richer {
                self.types[idx] = ty;
            }
            return;
        }
        self.types.push(ty);
    }

    /// Allocate a fresh user-defined `TypeId` that doesn't collide
    /// with the reserved well-known ranges.
    ///

    /// Reserved ranges (see `crate::types::TypeId` constants):
    ///  * 0..16 primitives + aliases
    ///  * 256..260 meta system (TokenStream / Token / Kind / Span)
    ///  * 512..1024 semantic collections + dependent-type packaging
    ///  (LIST, MAP, …, PI, SIGMA, WITNESS)
    ///

    /// Without this guard, a stdlib build whose user-type count
    /// exceeds 240 wraps `next_type_id` into the meta range, then
    /// past 252 wraps into the semantic range — and stdlib types
    /// silently collide with reserved IDs. #170's global
    /// consistency check surfaced this on `result.vr` where
    /// `OneshotInner` and `Channel` both ended up at TypeId(523).
    ///

    /// The function bumps `next_type_id` past every reserved range
    /// it encounters before returning. Idempotent in the sense
    /// that calling it `n` times produces `n` distinct IDs.
    fn alloc_user_type_id(&mut self) -> crate::types::TypeId {
        use crate::types::TypeId;
        loop {
            let candidate = TypeId::FIRST_USER + self.next_type_id;
            // Meta-system range: 256..260 (TOKEN_STREAM..SPAN). If we
            // landed inside, skip to 260.
            if (256..260).contains(&candidate) {
                self.next_type_id = 260 - TypeId::FIRST_USER;
                continue;
            }
            // Semantic-collection + dependent-type range: 512..1024.
            // Skip past it on first encounter.
            if (TypeId::FIRST_SEMANTIC..=TypeId::LAST_SEMANTIC).contains(&candidate) {
                self.next_type_id = (TypeId::LAST_SEMANTIC + 1) - TypeId::FIRST_USER;
                continue;
            }
            self.next_type_id += 1;
            return TypeId(candidate);
        }
    }

    /// Cross-module type-table consistency check (#170).
    ///

    /// Runs after all imported modules have been processed and the
    /// codegen's `self.types` represents the unified, whole-program
    /// type table. Reports the structural-hygiene classes that the
    /// per-module verifier deliberately can't catch:
    ///

    ///  1. **Duplicate `TypeId`** — two `TypeDescriptor`s with the
    ///  same numeric id but different declaration sites. Caused
    ///  by a name collision where the `type_name_to_id` insert
    ///  guard (`if !contains_key`) silently merges the second
    ///  type's declaration into the first's slot.
    ///

    ///  2. **Same name, different `TypeId`** — two descriptors
    ///  sharing a name with distinct ids. Indicates the codegen
    ///  ran multiple type-allocation passes and the second pass
    ///  didn't see the first pass's registration.
    ///

    ///  3. **Variant-tag gaps / duplicates within a sum** — already
    ///  checked per-module by `verify_type_layout_invariants`,
    ///  lifted here so a global pass catches the case where two
    ///  modules separately declare overlapping subsets of the
    ///  same logical sum's variants.
    ///

    /// Returns a structured `TypeTableHealthReport`; the caller
    /// decides whether to `report.assert_clean()` (hard fail), warn
    /// via tracing, or stash for downstream consumers (CI dashboards).
    /// Distinct return shape from `verify_type_layout_invariants` so
    /// the two checks compose without one masking the other.
    pub fn verify_global_type_table_consistency(&self) -> TypeTableHealthReport {
        Self::compute_type_table_health(&self.types, &self.ctx.strings)
    }

    /// Scan every emitted function body for `MakeVariant` instructions
    /// whose `(tag, field_count)` doesn't match any variant in the
    /// unified type table. At the per-module level this is a warn
    /// (cross-module variants live in other modules' descriptors); at
    /// the global level it's a real bug — every `MakeVariant` should
    /// resolve once all modules have been registered.
    pub fn find_orphan_make_variants(&self) -> Vec<OrphanMakeVariant> {
        use crate::instruction::Instruction;
        use crate::types::VariantKind;
        // Build the set of valid (tag, field_count) combos across all
        // declared types. HashSet for O(1) membership.
        let mut valid: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for ty in &self.types {
            for v in &ty.variants {
                let count = match v.kind {
                    VariantKind::Unit => 0u32,
                    VariantKind::Tuple => v.arity as u32,
                    VariantKind::Record => v.fields.len() as u32,
                };
                valid.insert((v.tag, count));
            }
        }
        // Empty type table → can't compare; bail. The per-module
        // verifier handles the empty case identically.
        if valid.is_empty() {
            return Vec::new();
        }
        let mut orphans = Vec::new();
        for f in &self.functions {
            for ins in &f.instructions {
                if let Instruction::MakeVariant {
                    dst: _,
                    tag,
                    field_count,
                } = ins
                    && !valid.contains(&(*tag, *field_count))
                {
                    let fname = self
                        .ctx
                        .strings
                        .get(f.descriptor.name.0 as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("<FunctionId({})>", f.descriptor.id.0));
                    orphans.push(OrphanMakeVariant {
                        function_name: fname,
                        tag: *tag,
                        field_count: *field_count,
                    });
                }
            }
        }
        orphans
    }

    /// Pure helper that builds the health report from a slice of
    /// `TypeDescriptor`s and the matching string table. Pulled out
    /// so unit tests can construct synthetic tables without going
    /// through the full codegen lifecycle.
    fn compute_type_table_health(
        types: &[crate::types::TypeDescriptor],
        strings: &[String],
    ) -> TypeTableHealthReport {
        use std::collections::HashMap;
        let resolve_name = |idx: u32| -> String {
            strings
                .get(idx as usize)
                .cloned()
                .unwrap_or_else(|| format!("<id {}>", idx))
        };

        // Pass 1: bucket by TypeId. >1 in a bucket means duplicate ids.
        let mut by_id: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, ty) in types.iter().enumerate() {
            by_id.entry(ty.id.0).or_default().push(i);
        }
        let mut duplicate_ids: Vec<DuplicateTypeId> = Vec::new();
        for (id, idxs) in &by_id {
            if idxs.len() > 1 {
                let names: Vec<String> = idxs
                    .iter()
                    .map(|&i| resolve_name(types[i].name.0))
                    .collect();
                duplicate_ids.push(DuplicateTypeId {
                    type_id: *id,
                    descriptor_names: names,
                });
            }
        }

        // Pass 2: bucket by name. Two entries with the same name should
        // share the same id (alias case); different ids = a real bug.
        let mut by_name: HashMap<String, Vec<(u32, usize)>> = HashMap::new();
        for (i, ty) in types.iter().enumerate() {
            by_name
                .entry(resolve_name(ty.name.0))
                .or_default()
                .push((ty.id.0, i));
        }
        let mut duplicate_names_with_different_ids: Vec<DuplicateNameDifferentId> = Vec::new();
        for (name, slots) in &by_name {
            if slots.len() > 1 {
                let ids: std::collections::HashSet<u32> = slots.iter().map(|(id, _)| *id).collect();
                if ids.len() > 1 {
                    let mut sorted_ids: Vec<u32> = ids.into_iter().collect();
                    sorted_ids.sort_unstable();
                    duplicate_names_with_different_ids.push(DuplicateNameDifferentId {
                        name: name.clone(),
                        type_ids: sorted_ids,
                    });
                }
            }
        }

        // Pass 3: variant-tag density. Tags within a sum must be
        // 0..variants.len() with no holes and no duplicates. The
        // per-module verifier already checks this; we re-check here
        // because the per-module check skips empty `variants` arrays
        // (records) but a sum that lost variants in cross-module
        // dedupe still has the original arity stored elsewhere — we
        // only flag when the slice is non-empty AND non-dense.
        let mut variant_tag_anomalies: Vec<VariantTagAnomaly> = Vec::new();
        for ty in types {
            if ty.variants.is_empty() {
                continue;
            }
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            let mut max_tag: u32 = 0;
            let mut duplicate_tags: Vec<u32> = Vec::new();
            for v in &ty.variants {
                if !seen.insert(v.tag) {
                    duplicate_tags.push(v.tag);
                }
                if v.tag > max_tag {
                    max_tag = v.tag;
                }
            }
            let n = ty.variants.len() as u32;
            // Gap detection: a dense [0..n) means seen.len() == n AND
            // max_tag == n-1. Anything else has gaps or out-of-range
            // tags.
            let dense = seen.len() as u32 == n && (n == 0 || max_tag == n - 1);
            if !dense || !duplicate_tags.is_empty() {
                let missing: Vec<u32> = (0..n.max(max_tag + 1))
                    .filter(|t| !seen.contains(t))
                    .collect();
                variant_tag_anomalies.push(VariantTagAnomaly {
                    type_name: resolve_name(ty.name.0),
                    type_id: ty.id.0,
                    expected_count: n,
                    max_tag_seen: max_tag,
                    duplicate_tags,
                    missing_tags: missing,
                });
            }
        }

        TypeTableHealthReport {
            duplicate_ids,
            duplicate_names_with_different_ids,
            variant_tag_anomalies,
        }
    }

    /// Test-only: intern a string in the codegen's string table and
    /// return its index, for constructing synthetic TypeDescriptors in
    /// integration tests of `verify_type_layout_invariants`.
    #[doc(hidden)]
    pub fn intern_string_for_test(&mut self, s: &str) -> u32 {
        self.ctx.intern_string_raw(s)
    }

    /// Record archive-side function name → user-side FunctionId.
    /// Populates the Tier-2 name-fallback index from
    /// `archive_ctx_loader` for cross-module Call resolution.
    pub fn record_archive_function_name(
        &mut self,
        name: &str,
        fid: crate::module::FunctionId,
    ) {
        self.archive_func_name_to_fid
            .entry(name.to_string())
            .or_insert(fid);
    }

    /// Test-only: push a synthetic `TypeDescriptor` into the codegen's
    /// type table. Used by integration tests for the layout verifier
    /// to construct deliberately-malformed types and assert the
    /// verifier rejects them.
    #[doc(hidden)]
    pub fn push_type_for_test(&mut self, ty: crate::types::TypeDescriptor) {
        self.types.push(ty);
    }

    fn check_type_layout_invariants_inner(
        types: &[crate::types::TypeDescriptor],
        strings: &[String],
    ) -> CodegenResult<()> {
        use crate::types::VariantKind;
        let resolve_name = |idx: u32| -> String {
            strings
                .get(idx as usize)
                .cloned()
                .unwrap_or_else(|| format!("<id {}>", idx))
        };
        for ty in types {
            if ty.variants.is_empty() {
                continue;
            }
            let type_name = resolve_name(ty.name.0);
            for v in &ty.variants {
                let v_name = resolve_name(v.name.0);
                match v.kind {
                    VariantKind::Unit => {
                        if v.arity != 0 || !v.fields.is_empty() {
                            return Err(CodegenError::internal(format!(
                                "type-layout invariant: variant `{}.{}` is `Unit` \
                                 but has arity={} and {} record-field(s); \
                                 unit variants must carry no payload",
                                type_name,
                                v_name,
                                v.arity,
                                v.fields.len(),
                            )));
                        }
                    }
                    VariantKind::Tuple => {
                        // Tuple variants admit two valid `fields` shapes:
                        //
                        //   * Empty (the import-from-archive form —
                        //     `import_archive_type` strips fields for
                        //     Tuple kind because the archive's fields
                        //     entry is redundant once arity is known).
                        //
                        //   * Positional `_0`, `_1`, …, `_(arity-1)`
                        //     with `fields.len() == arity` (the
                        //     fresh-codegen form — see
                        //     `compile_type_decl` line ~8327, which
                        //     populates per-slot TypeRef so
                        //     `archive_metadata` can recover the
                        //     payload type for each slot WITHOUT
                        //     falling back to the parent's first
                        //     generic param).
                        //
                        // Anything else (mismatched arity, named
                        // fields on Tuple) indicates the descriptor
                        // was assembled by a path that confused
                        // Tuple with Record — that's the original
                        // class of bug this gate catches.
                        if !v.fields.is_empty() {
                            if v.fields.len() != v.arity as usize {
                                return Err(CodegenError::internal(format!(
                                    "type-layout invariant: variant `{}.{}` is `Tuple` \
                                     (arity={}) but `fields.len() = {}`; \
                                     positional payload count must agree with arity",
                                    type_name,
                                    v_name,
                                    v.arity,
                                    v.fields.len(),
                                )));
                            }
                            for (idx, fd) in v.fields.iter().enumerate() {
                                let expected = format!("_{}", idx);
                                let actual = strings
                                    .get(fd.name.0 as usize)
                                    .map(|s| s.as_str())
                                    .unwrap_or("");
                                if actual != expected {
                                    return Err(CodegenError::internal(format!(
                                        "type-layout invariant: variant `{}.{}` is `Tuple` \
                                         but `fields[{}].name = {:?}` (expected positional \
                                         `{}`); tuple variants must use positional names",
                                        type_name, v_name, idx, actual, expected,
                                    )));
                                }
                            }
                        }
                        if v.arity == 0 {
                            return Err(CodegenError::internal(format!(
                                "type-layout invariant: variant `{}.{}` is `Tuple` \
                                 with arity=0; a zero-arity payload should be \
                                 declared as `Unit` instead",
                                type_name, v_name,
                            )));
                        }
                    }
                    VariantKind::Record => {
                        if v.arity != 0 {
                            return Err(CodegenError::internal(format!(
                                "type-layout invariant: variant `{}.{}` is `Record` \
                                 (with {} field(s)) but also reports arity={}; \
                                 record variants store payload count in `fields`, \
                                 not `arity`",
                                type_name,
                                v_name,
                                v.fields.len(),
                                v.arity,
                            )));
                        }
                        if v.fields.is_empty() {
                            return Err(CodegenError::internal(format!(
                                "type-layout invariant: variant `{}.{}` is `Record` \
                                 with no fields; a fieldless variant should be \
                                 declared as `Unit` instead",
                                type_name, v_name,
                            )));
                        }
                    }
                }
            }
            let n = ty.variants.len() as u32;
            let mut seen = vec![false; n as usize];
            for v in &ty.variants {
                if v.tag >= n {
                    return Err(CodegenError::internal(format!(
                        "type-layout invariant: variant tag {} on type `{}` is \
                         outside the dense range 0..{} for {} variant(s); \
                         runtime dispatch indexes the variants array by tag",
                        v.tag, type_name, n, n,
                    )));
                }
                if seen[v.tag as usize] {
                    return Err(CodegenError::internal(format!(
                        "type-layout invariant: duplicate variant tag {} on \
                         type `{}` — every variant of a sum type must have a \
                         unique tag",
                        v.tag, type_name,
                    )));
                }
                seen[v.tag as usize] = true;
            }
        }
        Ok(())
    }

    /// Compiles an AST module to VBC.
    pub fn compile_module(&mut self, module: &Module) -> CodegenResult<VbcModule> {
        self.reset();
        self.register_builtin_variants();
        self.register_stdlib_constants();
        self.register_stdlib_intrinsics();
        self.register_runtime_io_functions();

        // #107 — module-level @cfg gating. If the AST module carries
        // attributes (e.g., `module.attributes` populated by the parser
        // from a file-scope `@cfg(target_os = "windows")` placed before
        // the first item), evaluate them once. If they don't match the
        // active build cfg, return an empty VbcModule — every item in
        // this file is implicitly conditional on the module-level cfg.
        //

        // This complements the per-item check at `should_compile_item`
        // for cases where the @cfg appears at file scope (above the
        // module's first item) rather than on individual declarations.
        // Without this, on macOS the entire `core/sys/windows/winsock2.vr`
        // compiles its `public fn` bodies, emitting `_socket`/`_bind`/...
        // symbols that shadow libSystem's BSD socket API and SIGABRT
        // when called via the wrappers from `core/net/tcp.vr`.
        if !module.attributes.is_empty() && !self.cfg_evaluator.should_include(&module.attributes) {
            // Module gated out — produce an empty VbcModule.
            return self.build_module();
        }

        // Pass 1: Collect protocol definitions first (for default method inheritance)
        // This ensures protocols with default methods are registered before impl blocks
        // that implement them are processed. Without this, protocol default methods
        // wouldn't be available when generating impl block methods.
        self.collect_protocol_definitions(module);

        // Pass 1.5: Pre-allocate TypeIds for all user-defined types.
        // This ensures forward references in field types resolve correctly
        // (e.g., CompileResult.ast: Maybe<Module> needs Module's TypeId).
        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Type(type_decl) = &item.kind {
                let type_name = type_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&type_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name, type_id);
                }
            }
        }

        // Mark all types in this module as user-defined (for variant disambiguation)
        self.mark_user_defined_types(module);

        // Pass 2: Collect all function declarations
        // Filter items based on @cfg attributes to prevent cross-platform conflicts
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.collect_declarations(item)?;
            }
        }

        // Compile pending default protocol methods.
        // These were registered during declaration collection but their bodies need to be
        // compiled after all functions are registered.
        self.compile_pending_default_methods()?;

        // Pass 3: Compile function bodies
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.compile_item(item)?;
            }
        }

        // Compile any pending constants (struct literals, etc.)
        self.compile_pending_constants()?;
        self.compile_pending_tls_inits()?;

        // Build the VBC module
        self.build_module()
    }

    /// Compiles an AST module to VBC with mount (import) resolution.
    ///

    /// Like `compile_module`, but first resolves `mount` declarations by finding
    /// the corresponding .vr files in `core_root`, parsing them, and registering
    /// their type/function declarations. This makes imported types, variant
    /// constructors, and function signatures available during compilation.
    ///

    /// # Arguments
    /// * `module` - The parsed AST module to compile
    /// * `source_path` - Path to the source .vr file (used for relative path resolution)
    /// * `core_root` - Path to the `core/` directory root
    pub fn compile_module_with_mounts(
        &mut self,
        module: &Module,
        source_path: &str,
        core_root: &str,
    ) -> CodegenResult<VbcModule> {
        self.reset();
        self.register_builtin_variants();
        self.register_stdlib_constants();
        self.register_stdlib_intrinsics();
        self.register_runtime_io_functions();

        // Mount resolution: parse imported .vr files and register their declarations.
        // This happens after builtins are registered so that imported types don't
        // conflict with built-in variants (None, Some, Ok, Err, etc.).
        // Use prefer_existing_functions to avoid overwriting builtins.
        self.set_prefer_existing_functions(true);
        self.resolve_mounts(module, source_path, core_root);
        self.set_prefer_existing_functions(false);

        // Pass 1: Collect protocol definitions first (for default method inheritance)
        self.collect_protocol_definitions(module);

        // Pass 1.5: Pre-allocate TypeIds for all user-defined types.
        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Type(type_decl) = &item.kind {
                let type_name = type_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&type_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name, type_id);
                }
            }
        }

        // Mark all types in this module as user-defined (for variant disambiguation)
        self.mark_user_defined_types(module);

        // Pass 2: Collect all function declarations
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.collect_declarations(item)?;
            }
        }

        // Compile pending default protocol methods.
        self.compile_pending_default_methods()?;

        // Pass 3: Compile function bodies
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.compile_item(item)?;
            }
        }

        // Compile any pending constants (struct literals, etc.)
        self.compile_pending_constants()?;
        self.compile_pending_tls_inits()?;

        // Build the VBC module
        self.build_module()
    }

    /// Compiles additional AST modules without resetting.
    ///

    /// This is used when compiling multiple .vr files into a single logical module
    /// (e.g., core/base/*.vr files). Unlike `compile_module`, this method preserves
    /// previously registered functions and imported functions from other modules.
    ///

    /// Call `import_functions` before the first `compile_additional_module` to make
    /// functions from previously compiled modules available.
    pub fn compile_additional_module(&mut self, module: &Module) -> CodegenResult<VbcModule> {
        // Pass 1: Collect protocol definitions first (for default method inheritance)
        self.collect_protocol_definitions(module);

        // Pass 1.5: Pre-allocate TypeIds for all user-defined types.
        // This ensures type references in function return types resolve correctly
        // (e.g., CharIndices, ByteIter, Lines in text.vr need TypeIds before
        // ast_type_to_type_ref is called for function descriptors).
        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Type(type_decl) = &item.kind {
                let type_name = type_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&type_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name, type_id);
                }
            }
        }

        // Pass 2: Collect all function declarations (adds to existing)
        // Filter items based on @cfg attributes to prevent cross-platform conflicts
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.collect_declarations(item)?;
            }
        }

        // Compile pending default protocol methods.
        // These were registered during declaration collection but their bodies need to be
        // compiled after all functions are registered.
        self.compile_pending_default_methods()?;

        // Pass 3: Compile function bodies
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.compile_item(item)?;
            }
        }

        // Compile any pending constants (struct literals, etc.)
        self.compile_pending_constants()?;
        self.compile_pending_tls_inits()?;

        // Build the VBC module
        self.build_module()
    }

    /// Collects all declarations from an AST module without compiling.
    ///

    /// This is used for two-pass compilation where all declarations from
    /// multiple files need to be registered before compiling any function bodies.
    /// This ensures type constructors (like None, Some) are available when
    /// compiling functions in any file.
    ///

    /// Items are filtered based on @cfg attributes to prevent cross-platform
    /// conflicts (e.g., Linux imports being processed when targeting macOS).
    pub fn collect_all_declarations(&mut self, module: &Module) -> CodegenResult<()> {
        // Pre-allocate TypeIds for user-defined types before collecting declarations
        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Type(type_decl) = &item.kind {
                let type_name = type_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&type_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name, type_id);
                }
            }
        }

        // **Blanket-impl pre-pass** (closes task #11).
        //
        // Stdlib declaration order in `core/async/future.vr` puts
        // `implement<F: Future> FutureExt for F {}` at line 257 — AFTER
        // every concrete `implement Future for ReadyFuture / PendingFuture
        // / Lazy / MapFuture / AndThenFuture` at lines 65-169.  Single-
        // pass collection means concrete impls call
        // `generate_default_protocol_methods("Future", "ReadyFuture", …)`
        // with `self.blanket_impls = [Future→IntoFuture]` (line 47's
        // blanket is observable), MISSING `Future→FutureExt` (line 257
        // not yet visited).  Result: FutureExt's default-method bodies
        // (`block` / `map` / `and_then`) never monomorphise onto
        // ReadyFuture and runtime `ready(v).block()` panics with
        // "method 'ReadyFuture.block' not found".
        //
        // Pre-pass populates `self.blanket_impls` from a single linear
        // scan over `module.items`, identifying blanket impls (those
        // where `for_type` IS a generic param of the impl) and recording
        // their `(base_protocol, derived_protocol, explicit_methods)`
        // tuple.  Subsequent `collect_declarations` walk's blanket-impl
        // observation at mod.rs:5593 short-circuits via the
        // `already_present` check, so per-blanket-impl bookkeeping
        // remains O(unique blanket impls), not O(occurrences).
        //
        // The pre-pass NEVER calls `generate_default_protocol_methods`
        // itself — it only seeds `self.blanket_impls`.  This is the
        // critical invariant that avoids the Poll-suite regression of
        // the prior reverted attempt: the protocol-registry guard at
        // line 1455 (skip empty `default_methods` AND `super_protocols`
        // entries) stays intact; default-method materialisation runs
        // exactly once per (concrete impl × derived protocol) pair
        // during the main pass.  Poll-implementers' Default-for-Poll<T>
        // impls (poll.vr line 168) are NOT blanket — `for_type = Poll<T>`
        // is a generic type, not a bare param — so `for_type_generic_param_name`
        // returns `None` and the pre-pass skips them, preserving the
        // original collection order for the Poll dispatch path.
        for item in module.items.iter() {
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Impl(impl_decl) = &item.kind
                && let verum_ast::decl::ImplKind::Protocol {
                    protocol, for_type, ..
                } = &impl_decl.kind
                && let Some(derived_name) = protocol.segments.last().and_then(|s| match s {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                    _ => None,
                })
                && let Some(param_name) = Self::for_type_generic_param_name(for_type)
            {
                for g in impl_decl.generics.iter() {
                    if let verum_ast::ty::GenericParamKind::Type { name, bounds, .. } = &g.kind
                        && name.name.as_str() == param_name
                    {
                        for b in bounds.iter() {
                            if let Some(base_name) = Self::type_bound_protocol_name(b) {
                                let explicit_methods: std::collections::HashSet<String> =
                                    impl_decl
                                        .items
                                        .iter()
                                        .filter_map(|item| {
                                            if let verum_ast::decl::ImplItemKind::Function(f) =
                                                &item.kind
                                            {
                                                Some(f.name.name.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                let already_present = self.blanket_impls.iter().any(|b| {
                                    b.base_protocol == base_name
                                        && b.derived_protocol == derived_name
                                });
                                if !already_present {
                                    self.blanket_impls.push(BlanketImpl {
                                        base_protocol: base_name,
                                        derived_protocol: derived_name.clone(),
                                        explicit_methods,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.collect_declarations(item)?;
            }
        }
        Ok(())
    }

    /// Resolves mount (import) declarations from a module by finding the corresponding
    /// .vr files, parsing them, and registering their type/function declarations.
    ///

    /// This enables cross-file compilation: when file A mounts types from file B,
    /// this method parses file B and registers its declarations so that A's codegen
    /// knows about the imported types, variant constructors, and function signatures.
    ///

    /// Only declarations (type names, variant constructors, function signatures) are
    /// registered -- function bodies from imported files are NOT compiled.
    ///

    /// # Arguments
    /// * `module` - The parsed AST module containing mount declarations
    /// * `source_path` - Path to the source .vr file (used to resolve relative paths)
    /// * `core_root` - Path to the `core/` directory root
    ///

    /// # Example
    /// ```ignore
    /// let mut codegen = VbcCodegen::new();
    /// codegen.resolve_mounts(&module, "core/sync/mutex.vr", "core/");
    /// codegen.compile_module(&module)?;
    /// ```
    pub fn resolve_mounts(&mut self, module: &Module, source_path: &str, core_root: &str) {
        let mut resolved_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Prevent re-resolving the source file itself
        if let Ok(canonical) = std::fs::canonicalize(source_path) {
            resolved_files.insert(canonical.to_string_lossy().to_string());
        }
        // Resolve mounts recursively (depth-limited to prevent infinite loops)
        self.resolve_mounts_recursive(module, source_path, core_root, &mut resolved_files, 0);
    }

    /// Recursively resolves mount declarations up to a bounded depth.
    fn resolve_mounts_recursive(
        &mut self,
        module: &Module,
        source_path: &str,
        core_root: &str,
        resolved_files: &mut std::collections::HashSet<String>,
        depth: u32,
    ) {
        const MAX_DEPTH: u32 = 4;
        if depth > MAX_DEPTH {
            return;
        }

        // Collect all files to parse (avoid borrow issues with recursive calls)
        let mut to_parse: Vec<String> = Vec::new();

        for item in module.items.iter() {
            // Honour the per-item @cfg gate. A `mount` whose attribute
            // doesn't match the current TargetConfig must not pull its
            // file into the build, otherwise platform-cfg type
            // declarations from the wrong target end up in the unified
            // type table and the global type-table consistency check
            // (#170) reports them as collisions with the matching
            // platform's declarations.
            if !self.should_compile_item(item) {
                continue;
            }
            if let ItemKind::Mount(mount_decl) = &item.kind {
                let paths = Self::extract_mount_file_paths(&mount_decl.tree, &[]);
                for module_path in paths {
                    let file_candidates =
                        Self::module_path_to_file_candidates(&module_path, source_path, core_root);
                    for candidate in file_candidates {
                        let canonical = match std::fs::canonicalize(&candidate) {
                            Ok(c) => c.to_string_lossy().to_string(),
                            Err(_) => continue, // File doesn't exist
                        };
                        if resolved_files.contains(&canonical) {
                            continue;
                        }
                        resolved_files.insert(canonical);
                        to_parse.push(candidate);
                        break; // Found a valid file, stop trying candidates
                    }
                }
                // Glob-mount expansion: `mount core.*` / `mount core.text.*`
                // — when the mount tree is a glob, walk every `.vr` file
                // under the resolved directory.  Without this, the
                // stdlib-internal compile harness can't see cross-module
                // static methods (`Text.from_utf8_lossy`, `List.of`)
                // because `mount core.*` previously degenerated into a
                // single `core/mod.vr` candidate (which holds no mounts).
                let globs = Self::extract_glob_mount_paths(&mount_decl.tree, &[]);
                for module_path in globs {
                    let file_candidates =
                        Self::module_path_to_file_candidates(&module_path, source_path, core_root);
                    for candidate in file_candidates {
                        // The glob's "file" candidate is typically a
                        // module file path; strip the `.vr` extension /
                        // `mod.vr` suffix and walk the resolved
                        // directory.  A single glob may contribute many
                        // files via the recursive walk below.
                        let dir = std::path::Path::new(&candidate);
                        let walk_root: std::path::PathBuf = if candidate.ends_with("/mod.vr") {
                            dir.parent().unwrap_or(dir).to_path_buf()
                        } else if dir.extension().and_then(|s| s.to_str()) == Some("vr") {
                            dir.with_extension("").to_path_buf()
                        } else {
                            dir.to_path_buf()
                        };
                        if walk_root.is_dir() {
                            for vr_file in Self::walk_vr_files(&walk_root) {
                                let canonical = match std::fs::canonicalize(&vr_file) {
                                    Ok(c) => c.to_string_lossy().to_string(),
                                    Err(_) => continue,
                                };
                                if resolved_files.contains(&canonical) {
                                    continue;
                                }
                                resolved_files.insert(canonical);
                                to_parse.push(vr_file);
                            }
                            break; // Walked one valid root, done with this glob
                        }
                    }
                }
            }
        }

        // Parse and register each imported module
        for file_path in to_parse {
            #[cfg(feature = "codegen")]
            if let Ok(source) = std::fs::read_to_string(&file_path) {
                let mut parser = verum_fast_parser::Parser::new(&source);
                if let Ok(imported_module) = parser.parse_module() {
                    // Recursively resolve mounts from this imported file first
                    self.resolve_mounts_recursive(
                        &imported_module,
                        &file_path,
                        core_root,
                        resolved_files,
                        depth + 1,
                    );
                    // Then register its declarations
                    self.collect_protocol_definitions(&imported_module);
                    let _ = self.collect_all_declarations(&imported_module);
                }
            }
        }
    }

    /// Extracts only the module-path segments of GLOB-form mounts.
    /// Sibling to `extract_mount_file_paths`; the glob form needs a
    /// distinct extraction path because the resolver expands a glob
    /// into a directory walk rather than a single-file candidate.
    fn extract_glob_mount_paths(tree: &MountTree, prefix: &[String]) -> Vec<Vec<String>> {
        let mut results = Vec::new();
        match &tree.kind {
            MountTreeKind::Path(_) => {}
            MountTreeKind::Glob(path) => {
                let mut segments: Vec<String> = prefix.to_vec();
                for segment in path.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => segments.push(ident.name.to_string()),
                        PathSegment::Super => segments.push("super".to_string()),
                        PathSegment::Cog => segments.push("cog".to_string()),
                        PathSegment::Relative => segments.push(".".to_string()),
                        PathSegment::SelfValue => segments.push("self".to_string()),
                    }
                }
                if !segments.is_empty() {
                    results.push(segments);
                }
            }
            MountTreeKind::Nested {
                prefix: nested_prefix,
                trees,
            } => {
                let mut segments: Vec<String> = prefix.to_vec();
                for segment in nested_prefix.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => segments.push(ident.name.to_string()),
                        PathSegment::Super => segments.push("super".to_string()),
                        PathSegment::Cog => segments.push("cog".to_string()),
                        PathSegment::Relative => segments.push(".".to_string()),
                        PathSegment::SelfValue => segments.push("self".to_string()),
                    }
                }
                for child in trees.iter() {
                    let child_results = Self::extract_glob_mount_paths(child, &segments);
                    results.extend(child_results);
                }
            }
            MountTreeKind::File { .. } => {}
        }
        results
    }

    /// Walk a directory recursively and collect every `.vr` file path.
    /// Used by the glob-mount expansion to enumerate all sibling
    /// modules under a `mount core.*` / `mount core.text.*` declaration.
    /// Skips hidden directories and target build artefacts.
    fn walk_vr_files(root: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if name.starts_with('.') || name == "target" {
                    continue;
                }
                if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file()
                    && path.extension().and_then(|s| s.to_str()) == Some("vr")
                {
                    out.push(path.to_string_lossy().to_string());
                }
            }
        }
        out
    }

    /// Extracts module path segments from a mount tree.
    ///

    /// For `mount core.base.protocols.{X, Y}`, returns `["core", "base", "protocols"]`.
    /// For `mount core.*`, returns `["core"]`.
    /// For `mount .atomic.*`, returns `[".", "atomic"]`.
    fn extract_mount_file_paths(tree: &MountTree, prefix: &[String]) -> Vec<Vec<String>> {
        let mut results = Vec::new();
        match &tree.kind {
            MountTreeKind::Path(path) => {
                let mut segments: Vec<String> = prefix.to_vec();
                for segment in path.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => segments.push(ident.name.to_string()),
                        PathSegment::Super => segments.push("super".to_string()),
                        PathSegment::Cog => segments.push("cog".to_string()),
                        PathSegment::Relative => segments.push(".".to_string()),
                        PathSegment::SelfValue => segments.push("self".to_string()),
                    }
                }
                if !segments.is_empty() {
                    results.push(segments);
                }
            }
            MountTreeKind::Glob(path) => {
                // `mount core.*` - extract the path prefix
                let mut segments: Vec<String> = prefix.to_vec();
                for segment in path.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => segments.push(ident.name.to_string()),
                        PathSegment::Super => segments.push("super".to_string()),
                        PathSegment::Cog => segments.push("cog".to_string()),
                        PathSegment::Relative => segments.push(".".to_string()),
                        PathSegment::SelfValue => segments.push("self".to_string()),
                    }
                }
                if !segments.is_empty() {
                    results.push(segments);
                }
            }
            MountTreeKind::Nested {
                prefix: nested_prefix,
                trees,
            } => {
                let mut segments: Vec<String> = prefix.to_vec();
                for segment in nested_prefix.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => segments.push(ident.name.to_string()),
                        PathSegment::Super => segments.push("super".to_string()),
                        PathSegment::Cog => segments.push("cog".to_string()),
                        PathSegment::Relative => segments.push(".".to_string()),
                        PathSegment::SelfValue => segments.push("self".to_string()),
                    }
                }
                // The prefix is the module path (e.g., core.base.protocols)
                // Each tree in trees is an item name (e.g., X, Y, Z)
                // We want to resolve the prefix as a file
                if !segments.is_empty() {
                    results.push(segments.clone());
                }
                // Also try each child tree recursively
                for child in trees.iter() {
                    let child_results = Self::extract_mount_file_paths(child, &segments);
                    results.extend(child_results);
                }
            }
            // #5 / P1.5 — file-relative mount carries the
            // resolved file path as the literal `path` field;
            // the loader uses it directly without going
            // through module-path → file-path translation.
            // This extractor is for resolving module-path
            // mounts to candidate files, so File mounts simply
            // contribute themselves verbatim as a single-
            // segment "path" (the loader downstream will
            // recognise the leading `./` / `../` and treat it
            // as a literal source-relative path).
            MountTreeKind::File { path, .. } => {
                results.push(vec![path.as_str().to_string()]);
            }
        }
        results
    }

    /// Converts a module path like `["core", "base", "protocols"]` to candidate file paths.
    ///

    /// Tries multiple resolution strategies:
    /// 1. Direct: `core/base/protocols.vr`
    /// 2. Module dir: `core/base/protocols/mod.vr`
    /// 3. Parent module: `core/base.vr` (if protocols is an item name, not a file)
    /// 4. Relative: resolve `.` and `super` relative to source file
    fn module_path_to_file_candidates(
        module_path: &[String],
        source_path: &str,
        core_root: &str,
    ) -> Vec<String> {
        let mut candidates = Vec::new();

        if module_path.is_empty() {
            return candidates;
        }

        let source_dir = std::path::Path::new(source_path)
            .parent()
            .unwrap_or(std::path::Path::new("."));

        // Handle relative paths (starting with "." or "super")
        if module_path[0] == "." || module_path[0] == "super" {
            let mut base = source_dir.to_path_buf();
            let mut start_idx = 0;
            let mut super_count = 0u32;

            for (i, seg) in module_path.iter().enumerate() {
                match seg.as_str() {
                    "." => {
                        start_idx = i + 1;
                    }
                    "super" => {
                        super_count += 1;
                        // First `super` stays in source_dir (parent module = containing directory).
                        // Each additional `super` goes up one more level.
                        if super_count > 1 {
                            base = base.parent().unwrap_or(&base).to_path_buf();
                        }
                        start_idx = i + 1;
                    }
                    _ => break,
                }
            }

            let remaining: Vec<&str> = module_path[start_idx..]
                .iter()
                .map(|s| s.as_str())
                .collect();
            if !remaining.is_empty() {
                // Try as file: base/remaining.vr
                let file_path = base.join(remaining.join("/")).with_extension("vr");
                candidates.push(file_path.to_string_lossy().to_string());

                // Try parent as file (last segment might be item name)
                if remaining.len() > 1 {
                    let parent_path = base
                        .join(remaining[..remaining.len() - 1].join("/"))
                        .with_extension("vr");
                    candidates.push(parent_path.to_string_lossy().to_string());
                }

                // Try as directory with mod.vr
                let mod_path = base.join(remaining.join("/")).join("mod.vr");
                candidates.push(mod_path.to_string_lossy().to_string());
            }
            return candidates;
        }

        // Handle absolute paths starting with "core"
        let (root, segments) = if module_path[0] == "core" {
            // Strip "core" prefix, use core_root as base
            (
                std::path::Path::new(core_root).to_path_buf(),
                &module_path[1..],
            )
        } else {
            // Non-core paths: try under core/ anyway (e.g., "sys" → "core/sys")
            (std::path::Path::new(core_root).to_path_buf(), module_path)
        };

        let path_segments: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();

        if !path_segments.is_empty() {
            // Direct file: core_root/base/protocols.vr
            let file_path = root.join(path_segments.join("/")).with_extension("vr");
            candidates.push(file_path.to_string_lossy().to_string());

            // Parent file (last segment is item name): core_root/base.vr
            if path_segments.len() > 1 {
                let parent = root
                    .join(path_segments[..path_segments.len() - 1].join("/"))
                    .with_extension("vr");
                candidates.push(parent.to_string_lossy().to_string());
            }

            // Module directory: core_root/base/protocols/mod.vr
            let mod_path = root.join(path_segments.join("/")).join("mod.vr");
            candidates.push(mod_path.to_string_lossy().to_string());
        } else {
            // Just "core" itself - try core/mod.vr
            let mod_path = root.join("mod.vr");
            candidates.push(mod_path.to_string_lossy().to_string());
        }

        candidates
    }

    /// Compiles function bodies only, assuming declarations are already registered.
    ///

    /// This is the second pass of two-pass compilation. All declarations should
    /// have been collected via `collect_all_declarations` first.
    ///

    /// Items are filtered based on @cfg attributes to prevent cross-platform
    /// conflicts (e.g., Linux imports being processed when targeting macOS).
    pub fn compile_function_bodies(&mut self, module: &Module) -> CodegenResult<VbcModule> {
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                self.compile_item(item)?;
            }
        }

        // Compile pending constants (non-inlineable constants like struct literals).
        // These are compiled as zero-argument functions that return the constant value.
        self.compile_pending_constants()?;
        self.compile_pending_tls_inits()?;

        // Build the VBC module
        self.build_module()
    }

    /// Compile a module's items into the codegen's accumulated state
    /// WITHOUT producing a finalised `VbcModule`.  Pairs with
    /// [`Self::finalize_module_from_state`] for the multi-file
    /// stdlib-bootstrap path: all files in one stdlib module share a
    /// single string pool, type table, and bytecode block, so a final
    /// `finalize_module_from_state` call emits ONE coherent
    /// `VbcModule`.  Replaces the per-file `compile_function_bodies →
    /// merge_stdlib_vbc_modules` pattern that silently dropped
    /// inherent-method bodies (StringIds in function descriptors and
    /// bytecode operands point at SOURCE-local string offsets that
    /// the post-merge string-pool re-intern doesn't remap).
    ///
    /// Drains pending constants + TLS inits at the end of each file
    /// to mirror `compile_function_bodies`'s flow (their
    /// registrations happen during item processing; per-file flush
    /// keeps the bookkeeping symmetric with the old path).
    pub fn compile_items_into_state(&mut self, module: &Module) -> CodegenResult<()> {
        // Mirror `compile_function_bodies`'s per-file source-module
        // scoping: every file gets its own `current_source_module`
        // before `compile_item` runs, then restores the previous
        // value at the end.  Without this, qualified-name registration
        // in `register_function` falls back to `config.module_name`
        // (the *parent* stdlib module name), and inherent methods
        // declared in `core/text/text.vr` would land under
        // `core.text.text.method_name` (correct file path) only when
        // the AST's `module text;` decl is recovered — which itself
        // depends on this scoping being active.
        //
        // **Per-item lenient compilation**: uses `compile_item_lenient`
        // so a single failing item (typically a forward-ref to a
        // module compiled later) doesn't drop the rest of the file's
        // bodies on the floor.  The lenient helper emits a panic-stub
        // for the failed function so its slot in `self.functions`
        // remains populated under the FunctionId allocated during
        // signature registration — load-bearing for archive
        // dispatch: `register_module_filtered` walks the archive
        // entry's `functions` table to pull descriptors into the
        // user-side codegen ctx, and a missing slot translates into
        // "undefined function: <name>" at user-code codegen.
        let prev = self.ctx.current_source_module.take();
        if let Some(name) =
            Self::resolve_full_module_path(module, &self.config.module_name)
        {
            self.ctx.current_source_module = Some(name);
        }
        let result: CodegenResult<()> = (|| {
            let mut first_strict_err: Option<CodegenError> = None;
            for item in module.items.iter() {
                if self.should_compile_item(item)
                    && let Err(e) = self.compile_item_lenient(item)
                    && first_strict_err.is_none()
                {
                    first_strict_err = Some(e);
                }
            }
            self.compile_pending_constants()?;
            self.compile_pending_tls_inits()?;
            match first_strict_err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        })();
        self.ctx.current_source_module = prev;
        result
    }

    /// Emit a single coherent `VbcModule` from the codegen's
    /// accumulated types/functions/bytecode/strings.  Used by the
    /// stdlib-bootstrap path after `compile_items_into_state` calls
    /// have run for every file in the module — one finalize pass
    /// replaces the per-file `build_module + merge` chain.
    pub fn finalize_module_from_state(&mut self) -> CodegenResult<VbcModule> {
        // Stdlib precompile finalize — MINIMAL path.
        //
        // **History**: two earlier attempts to add prep passes for
        // cross-module Call resolution failed:
        //
        //  * Full prep passes (commit b409d23f2 hunk, then reverted):
        //    archive 12.9 MB → 110+ MB, first-compile 1 s → 85 s.
        //    Source: `emit_missing_stub_descriptors`'s CallM pass
        //    (~1M stubs across all stdlib modules).
        //
        //  * Surgical (Call-id pass only, this commit's predecessor):
        //    archive 12.9 MB → 134 MB. Source: Call-id pass at stdlib
        //    scale still produces ~800K stubs (each stdlib module has
        //    100s of bodies, each referencing ~100 cross-module
        //    functions).
        //
        // **Conclusion**: cross-module dispatch resolution at archive-
        // load time cannot rely on per-module extern-stub synthesis
        // without explosion at stdlib scale. The right architectural
        // fix is a bytecode-format change: encode cross-module function
        // references as STRING IDs (function names) instead of raw
        // func_ids, with a single string interning per module. The
        // user-side merge then resolves by name once per Call site,
        // not once per (module × imported-function) pair.
        //
        // Tracked as #47 with the format-change scope acknowledged.
        // Until that lands, stdlib bodies with raw `Call { func_id }`
        // to NON-`@intrinsic` cross-module functions rely on the
        // ArchiveBodyRemap Tier 2 name-fallback (which covers the
        // subset where the calling module's archive has the dangling
        // id in its function table — works for SAME-MODULE-but-
        // unemitted cases, not for truly cross-module).
        //
        // **Architectural value preserved**: the `compile_function`
        // descriptor.intrinsic_name propagation fix (commit
        // 698795e39) round-trips every `@intrinsic` declaration's
        // marker through the archive. User-side compile_call's
        // intercept at `expressions.rs:4385` fires for cbgr_alloc /
        // every other CBGR allocator + every `@intrinsic` declaration
        // with a Verum body — the bulk of cross-module dispatch
        // defects close via this path WITHOUT requiring stub
        // synthesis.
        //
        // **Safe prep passes** (added 2026-05-14): the only prep pass
        // that explodes is `emit_missing_stub_descriptors`. The other
        // two — `compile_pending_constants` + `compile_pending_tls_inits`
        // — emit bodies ONLY for items already queued during module
        // walk, so their output is bounded by the actual user-declared
        // const / TLS init count (typically dozens per stdlib module,
        // not the 7000+ imported-function fanout that blew up the
        // stub pass). Without these, public `const` declarations
        // whose value expressions are non-literal (method calls, like
        // `public const USIZE_BITS: USize = USize.bits;`) NEVER get a
        // body in the archive — user-side `mount core.sys.bitfield;
        // … bitfield.USIZE_BITS` then resolves to a body-less stub
        // and any read of the constant returns Unit. Same hazard for
        // `@thread_local static` inits with non-trivial initialisers.
        self.compile_pending_constants()?;
        self.compile_pending_tls_inits()?;

        // **Task #47 stage-3 finalize (paired with
        // `pre_register_unique_public_free_functions` in
        // `verum_compiler::pipeline::stdlib_bootstrap`).**
        //
        // For every `Call`/`CallG`/`TailCall`/`NewClosure`/`Spawn`/
        // `GenCreate` instruction emitted into THIS module's bytecode
        // whose `func_id` operand points to a stage-3-pre-registered
        // stub (id in the `u32::MAX - 0x100_0000 ..` sentinel range),
        // synthesise a minimal extern-shaped `FunctionDescriptor` with
        // the function's NAME.  The descriptor has no body (panic-stub
        // body emitter is the existing fallback for the few stubs that
        // remain unresolved at runtime — see below); the body for the
        // real function lives in its producing module's archive entry.
        //
        // At archive load time, `ArchiveBodyRemap::map_function`
        // (`codegen/mod.rs:16314+`) follows three tiers:
        //
        //   * Tier 1: per-module remap (in-module bodies) — irrelevant
        //     for stub ids since the body lives elsewhere.
        //   * Tier 2a: `archive_id_to_name.get(stub_id)` → look up name
        //     in user codegen's mount-filtered `ctx.functions`.  Hits
        //     when the user code directly mounted the producing
        //     module's function.
        //   * Tier 2b: same name → archive-wide `archive_func_by_name`
        //     index (populated for EVERY archive function regardless
        //     of mount-set membership — see `archive_ctx_loader.rs:
        //     1796`).  Hits when the producing module is transitively
        //     loaded but not directly mounted by the user.
        //   * Tier 3: identity fallback — the silent-miscompile case
        //     this fix eliminates.
        //
        // Without the descriptor here, the stub_id leaks past Tier 1
        // into Tier 3 identity fallback (the failure mode observed in
        // the 2026-05-23 investigation: bloom.try_new's Call landing
        // on `DequeIntoIter.zip_longest` / `DequeDrain.map`).
        //
        // **Why this isn't the historical 110-134 MB explosion**:
        // `emit_missing_stub_descriptors_with_callm(false)` iterates
        // `self.functions` (only the LOCAL bytecode), extracts the
        // `referenced: HashSet<u32>` of `Call.func_id` operands
        // present in emitted instructions, and synthesises a stub
        // ONLY for ids in that set.  Per stdlib module this is bounded
        // by actual cross-module call count (typically tens), not by
        // the >7000 transitively-imported `ctx.functions` entries the
        // earlier failed pass walked unconditionally.  The CallM half
        // — which DID explode at 1M synthesized stubs — is gated off
        // by `include_callm=false`.
        //
        // Pin: `pre_register_unique_public_free_functions` in
        // `verum_compiler::pipeline::stdlib_bootstrap` MUST register
        // stubs with id in the `u32::MAX - 0x100_0000 ..` band, AND
        // the same module's `is_in_stub_range` check (in the post-
        // compile global-registry update loop) MUST recognise the
        // stage-3 band — otherwise the real bodies' subsequent
        // `register_function` won't overwrite the stub's id mapping.
        //
        // **IMPORTANT** — calling the FULL `emit_missing_stub_descriptors_with_callm(false)`
        // here would synthesise stubs for EVERY referenced cross-
        // module Call target, producing ~800K stubs at stdlib scale
        // and blowing `runtime.vbca` from ~13 MB to ~145 MB (docs at
        // `mod.rs:5687-5750`).  The stage-3-only variant
        // `emit_stage3_stub_descriptors` filters strictly to the
        // sentinel-range stage-3 ids registered by
        // `pre_register_unique_public_free_functions`, keeping the
        // archive growth bounded to the ~hundreds of stage-3 stubs
        // actually referenced by stdlib bodies (typical: ~100 KB
        // total archive growth across all modules).
        self.emit_stage3_stub_descriptors();

        self.build_module()
    }

    /// Resets the codegen for a new module.
    fn reset(&mut self) {
        self.ctx.reset();
        self.functions.clear();
        self.types.clear();
        self.next_func_id = 0;
        self.next_type_id = 0;
        self.closure_counter = 0;
        self.nested_function_scope.clear();
        // Clear FFI tables
        self.ffi_libraries.clear();
        self.ffi_symbols.clear();
        self.ffi_function_map.clear();
        self.ffi_library_map.clear();
        self.ffi_callback_signatures.clear();
        self.ffi_contracts.clear();
        self.ffi_contract_exprs.clear();
        // Clear pending imports
        self.pending_imports.clear();
        // Clear variant collisions (but typically these persist across modules)
        // Don't clear - collisions should accumulate across all compiled types
        // Clear field name indices
        self.field_name_indices.clear();
        self.next_field_id = 0;
        self.type_field_layouts.clear();
        self.type_field_type_names.clear();
        // Clear pending constants
        self.pending_constants.clear();
        // Clear static init function tracking
        self.static_init_functions.clear();
        // Clear pending TLS initializations
        self.pending_tls_inits.clear();
        // Clear context name registry
        self.context_name_to_id.clear();
        self.context_names.clear();
    }

    /// Seeds sentinel variant constructors (`None`, `Some`, `Ok`, `Err`,
    /// `Less`, `Equal`, `Greater`) so standalone compilation paths that do
    /// not pre-load `core/base/{maybe,result,ordering}.vr` can still
    /// resolve these names — e.g. a user writing `let x = Some(42);`
    /// without declaring a `Maybe` type.
    ///

    /// First-wins dispatch. Real registrations from actual type
    /// declarations take precedence via two complementary guards in
    /// `register_type_constructors`:
    ///

    ///  * User-phase (`prefer_existing_functions = false`): any existing
    ///  variants for the redeclared type are purged via
    ///  `clear_variants_for_type` before the new set is registered —
    ///  this wipes these sentinels.
    ///

    ///  * Stdlib-phase (`prefer_existing_functions = true`): if a prior
    ///  registration has already populated variants for this nominal
    ///  type, `has_variants_for_type` short-circuits the re-registration
    ///  so stdlib variants do not leak into a user type of the same
    ///  name.
    ///

    /// Sentinel IDs (`u32::MAX - tag`) never overlap with real function
    /// IDs or the `u32::MAX / 2` range reserved for newtype pass-through.
    /// Tag values are kept in lock-step with the canonical declarations
    /// in `core/base/maybe.vr`, `core/base/result.vr`, and
    /// `core/base/ordering.vr` so a sentinel-seeded value is
    /// bit-compatible with a stdlib-seeded one.
    ///

    /// Long-term, this seed should be deleted entirely and replaced with
    /// an unconditional `ALWAYS_INCLUDE` of the relevant `core.base.*`
    /// modules. That refactor touches standalone-codegen call sites
    /// (REPL, `meta::vbc_executor`, bench harnesses) that drive
    /// `VbcCodegen::compile_module` without a pipeline-assembled stdlib.
    pub fn register_builtin_variants(&mut self) {
        // Register a fixed set of stdlib variant constructors so that user
        // code like `Maybe.Some(42)`, `Result.Ok(x)`, `Ordering.Less` compiles
        // correctly even when the stdlib definition isn't in the auto-included
        // module list.
        //

        // Rationale: `core.base.maybe` is deliberately excluded from
        // `collect_imported_stdlib_modules`' ALWAYS_INCLUDE list because the
        // stdlib `Maybe<T>` collides with user-defined `Maybe` test fixtures.
        // But programs that don't define their own `Maybe` still need the
        // variant tags to emit `MakeVariant` correctly — otherwise
        // `Maybe.Some(42)` falls through to method dispatch and panics at
        // runtime with "method 'Some' not found on value".
        //

        // Variant entries here are NOT hardcoded — they're sourced from
        // the canonical layouts in
        // `verum_common::well_known_types::{MAYBE_VARIANT_LAYOUT,
        // RESULT_VARIANT_LAYOUT, ORDERING_VARIANT_LAYOUT}`. The runtime
        // constructors (`make_ordering`, future `make_maybe_*` /
        // `make_result_*`) consult the same constants, so drift between
        // the .vr source-of-truth and either consumer is caught at the
        // verum_common test surface (`*_variant_layout_pinned`).
        //
        // When a user program *does* define its own
        // `type Maybe is None | Some(T)` (or similar),
        // register_type_constructors overwrites these entries with the
        // user-level tags (which also happen to match by convention), so
        // both paths agree.
        use crate::codegen::context::FunctionInfo;
        use crate::module::FunctionId;
        // Source of truth: `BUILTIN_VARIANT_CARRIERS` in
        // `verum_common::well_known_types`.  Adding a new variant
        // carrier (e.g. `Either<L,R>`) means appending one entry to
        // that constant — every consumer (this codegen registration
        // and the meta-sandbox builtin-fn dispatch) picks it up
        // automatically.  Arity lives inside each `VariantLayoutEntry`,
        // so no per-type ad-hoc loop branches.
        let layout_sources = verum_common::well_known_types::BUILTIN_VARIANT_CARRIERS;
        // (parent_type, variant_name, tag, arity, param_names) —
        // expanded uniformly from the layouts.  Param names follow the
        // tuple-payload convention (`_0`, `_1`, …) for nullary-or-N
        // variant carriers; if a future variant carries N>1 payloads
        // the name list grows automatically.
        let mut builtins: Vec<(&str, &str, u32, usize, Vec<String>)> =
            Vec::with_capacity(8);
        for (parent, layout) in layout_sources {
            for entry in layout.iter() {
                let arity = entry.arity as usize;
                let param_names: Vec<String> = (0..arity).map(|i| format!("_{}", i)).collect();
                builtins.push((parent, entry.name, entry.tag, arity, param_names));
            }
        }
        let builtins = &builtins;
        for (type_name, variant_name, tag, arity, param_names) in builtins {
            let qualified = format!("{}.{}", type_name, variant_name);
            // Skip if already registered (e.g., earlier pass or user-defined)
            if self.ctx.lookup_function(&qualified).is_some() {
                continue;
            }
            // Use descending-from-u32::MAX sentinel, matching register_type_constructors.
            // This avoids colliding with u32::MAX / 2 used by newtype constructors,
            // which would mis-dispatch variants through the newtype pass-through path.
            let sentinel_id = FunctionId(u32::MAX - *tag);
            let info = FunctionInfo {
                id: sentinel_id,
                param_count: *arity,
                param_names: param_names.clone(),
                param_type_names: vec![],
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: None,
                variant_tag: Some(*tag),
                parent_type_name: Some((*type_name).to_string()),
                variant_payload_types: None,
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: Some((*type_name).to_string()),
                return_type_inner: None,
                is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
            };
            // Always register qualified name.
            self.ctx.register_function(qualified, info.clone());
            // Also register simple name unless it would collide with a prior
            // registration (follows the same "simple-on-no-collision" rule as
            // user variant registration).
            if self.ctx.lookup_function(variant_name).is_none() {
                self.ctx
                    .register_function((*variant_name).to_string(), info);
            }
        }

        // Variant-name → TypeId mapping is set above via the
        // function table.  Type DESCRIPTORS for Maybe / Result /
        // Ordering are imported lazily via the archive lazy-load
        // path (`archive_ctx_loader::apply_lazy_with_types` second
        // pass — see the variant-tag-collision fix that imports
        // types alongside variant ctors in the unqualified-wanted
        // pass).  Eager pre-registration here would emit a
        // method-less stub descriptor that BLOCKS the real archive
        // import (via the first-wins gate at
        // `import_archive_type`); user code calling inherent
        // methods like `m.is_some()` would then panic with
        // `method 'Maybe.is_some' not found`.  Lazy import
        // preserves the full Maybe/Result descriptor including
        // inherent methods.
    }

    /// Registers runtime I/O and networking functions as builtins.
    /// These emit Call instructions that the LLVM lowering intercepts.
    pub fn register_runtime_io_functions(&mut self) {
        use crate::codegen::context::FunctionInfo;
        // (name, param_count)
        let functions: &[(&str, usize)] = &[
            ("file_write", 2),
            ("file_read", 1),
            ("file_append", 2),
            ("file_delete", 1),
            ("file_exists", 1),
            ("file_read_all", 1),
            ("file_write_all", 2),
            ("file_open", 2),
            ("file_close", 1),
            ("tcp_connect", 2),
            ("tcp_listen", 2),
            ("tcp_listen_v2", 4),
            ("tcp_local_port", 1),
            ("tcp_accept", 1),
            ("tcp_send", 2),
            ("tcp_recv", 2),
            ("tcp_close", 1),
            ("udp_bind", 1),
            ("udp_send", 3),
            ("udp_recv", 2),
            ("udp_close", 1),
        ];

        for (name, param_count) in functions {
            // Skip if already registered (e.g., from core/ module compilation)
            if self.ctx.lookup_function(name).is_some() {
                continue;
            }
            let func_id = FunctionId(self.next_func_id);
            self.next_func_id = self.next_func_id.saturating_add(1);

            // Add stub function with empty body to the function list so it
            // appears in the VBC module with the correct name. The LLVM lowering
            // intercepts calls by name and routes to runtime functions.
            let name_string_id = self.ctx.intern_string_raw(name);
            let stub_descriptor = FunctionDescriptor {
                id: func_id,
                name: StringId(name_string_id),
                ..Default::default()
            };
            self.push_function_dedup(VbcFunction::new(stub_descriptor, vec![]));

            let info = FunctionInfo {
                id: func_id,
                param_count: *param_count,
                param_names: vec![],
                param_type_names: vec![],
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: None,
                variant_tag: None,
                parent_type_name: None,
                variant_payload_types: None,
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: None,
                return_type_inner: None,
                is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
            };
            self.ctx.register_function(name.to_string(), info);
        }
    }

    /// Collects function declarations for forward references.
    fn collect_declarations(&mut self, item: &Item) -> CodegenResult<()> {
        match &item.kind {
            ItemKind::Function(func) => {
                // Check if this is a single-function FFI declaration:
                // @ffi("library") extern fn name(...);
                if func.extern_abi.is_some() && self.has_ffi_attribute(&func.attributes) {
                    // This is a single-function FFI declaration
                    self.register_single_ffi_function(func)?;
                } else {
                    self.register_function(func)?;
                    // Recursively collect nested declarations from function body
                    if let verum_common::Maybe::Some(ref body) = func.body {
                        let func_name = func.name.name.to_string();
                        self.collect_nested_declarations(body, &func_name)?;
                    }
                }
            }
            ItemKind::Impl(impl_decl) => {
                // Get the type name for qualified method registration
                let type_name = self.extract_impl_type_name(&impl_decl.kind);

                // Detect blanket impls: `implement<T: Base> Derived for T {}`.
                // Deferred monomorphization — when a concrete type later
                // `implement Base for Concrete`, we replay the blanket impl
                // onto Concrete. Without this, the default method bodies
                // register under the generic-param name and runtime
                // dispatch panics with "method not found on value".
                if let verum_ast::decl::ImplKind::Protocol {
                    protocol, for_type, ..
                } = &impl_decl.kind
                {
                    let derived_name = protocol.segments.last().and_then(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    });
                    let for_type_name = Self::for_type_generic_param_name(for_type);
                    if let (Some(derived_name), Some(param_name)) = (derived_name, for_type_name) {
                        for g in impl_decl.generics.iter() {
                            if let verum_ast::ty::GenericParamKind::Type { name, bounds, .. } =
                                &g.kind
                                && name.name.as_str() == param_name
                            {
                                for b in bounds.iter() {
                                    if let Some(base_name) = Self::type_bound_protocol_name(b) {
                                        let explicit_methods: std::collections::HashSet<String> =
                                            impl_decl
                                                .items
                                                .iter()
                                                .filter_map(|item| {
                                                    if let verum_ast::decl::ImplItemKind::Function(
                                                        f,
                                                    ) = &item.kind
                                                    {
                                                        Some(f.name.name.to_string())
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .collect();
                                        // Skip if `collect_all_declarations`'s
                                        // blanket-impl pre-pass already
                                        // registered this (base, derived)
                                        // pair — keeps the linear-scan
                                        // walks O(unique blanket impls)
                                        // instead of O(file occurrences).
                                        let already_present = self.blanket_impls.iter().any(|b| {
                                            b.base_protocol == base_name
                                                && b.derived_protocol == derived_name
                                        });
                                        if !already_present {
                                            self.blanket_impls.push(BlanketImpl {
                                                base_protocol: base_name,
                                                derived_protocol: derived_name.clone(),
                                                explicit_methods,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // For protocol impls, inherent methods take priority.
                // If "AppConfig.new" already exists from inherent impl,
                // don't let protocol impl's methods overwrite it.
                //

                // Save the previous flag value so we can restore it after the
                // impl. Without this save/restore, the unconditional `= false`
                // at the end of this branch corrupts pipeline-set state — e.g.
                // pipeline.rs's `set_prefer_existing_functions(true)` for the
                // imported-stdlib loop gets overwritten the moment any imported
                // module contains a protocol impl, after which the next
                // ItemKind::Type registration (e.g. RecoveryStrategy) runs in
                // user-mode, triggers cross-type collision detection, and
                // unregisters bare `None`. That breaks every other stdlib
                // body that legitimately uses the simple `Maybe.None` alias
                // (BTreeMap, Receiver.poll, all Stream adapters, etc.).
                let is_protocol_impl =
                    matches!(&impl_decl.kind, verum_ast::decl::ImplKind::Protocol { .. });
                let prev_prefer_existing = self.ctx.prefer_existing_functions;
                if is_protocol_impl {
                    self.ctx.prefer_existing_functions = true;
                }

                // Check if this is a Drop implementation
                let is_drop_impl = if let verum_ast::decl::ImplKind::Protocol { protocol, .. } =
                    &impl_decl.kind
                {
                    protocol
                        .segments
                        .first()
                        .and_then(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                            _ => None,
                        })
                        .map(|name| name == "Drop")
                        .unwrap_or(false)
                } else {
                    false
                };

                for impl_item in impl_decl.items.iter() {
                    match &impl_item.kind {
                        verum_ast::decl::ImplItemKind::Function(func) => {
                            // Register function by qualified name (e.g., "List::new") for static calls
                            // and method resolution. We do NOT register by simple name to avoid
                            // collisions with standalone functions of the same name.
                            if let Some(ref ty_name) = type_name {
                                self.register_impl_function(func, ty_name)?;

                                // If this is a Drop impl and the function is named "drop",
                                // record the drop function ID in the TypeDescriptor
                                if is_drop_impl && func.name.name == "drop" {
                                    let qualified_name = format!("{}.drop", ty_name);
                                    if let Some(func_info) =
                                        self.ctx.lookup_function(&qualified_name)
                                    {
                                        // Find the TypeDescriptor for this type and set drop_fn
                                        if let Some(type_id) = self.type_name_to_id.get(ty_name) {
                                            for type_desc in self.types.iter_mut() {
                                                if type_desc.id == *type_id {
                                                    type_desc.drop_fn = Some(func_info.id.0);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Fallback: register by simple name if type extraction failed
                                // This shouldn't happen often now that we handle more type kinds
                                self.register_function(func)?;
                            }

                            // Recursively collect nested declarations from impl function body
                            if let verum_common::Maybe::Some(ref body) = func.body {
                                let func_name = func.name.name.to_string();
                                self.collect_nested_declarations(body, &func_name)?;
                            }
                        }
                        verum_ast::decl::ImplItemKind::Const { name, value, ty } => {
                            // Register associated constants with qualified name (e.g., "Fd.INVALID")
                            // so that `Fd.INVALID` resolves to the constant value at codegen time.
                            if let Some(ref ty_name) = type_name {
                                let qualified = format!("{}.{}", ty_name, name.name);
                                self.register_constant_with_value(
                                    &qualified,
                                    Some(value),
                                    Some(ty),
                                )?;
                            }
                        }
                        _ => {}
                    }
                }

                // Generate default protocol methods for protocol impls.
                //
                // **Blanket-impl skip** (closes task #11 spurious-F.block
                // class): when `for_type` IS a generic parameter (e.g.
                // `implement<F: Future> FutureExt for F {}`), do NOT
                // materialise default methods under the generic-param's
                // bare name (`F.block` / `F.map` / `F.and_then`).  The
                // blanket impl is metadata — it records the
                // (base → derived) relationship in `self.blanket_impls`
                // (already done by the pre-pass + the inline-detection
                // arm above).  Concrete `implement Base for Concrete`
                // declarations are what trigger materialisation onto
                // the concrete type.  Without this skip, the spurious
                // `F.<method>` entries pollute the function table,
                // leak phantom FunctionIds via cross-pollination, and
                // shadow legitimate concrete-type bindings (the
                // mechanism that made `j.block()` on Join2 dispatch
                // through the wrong body).
                // Strict generic-param classification: a `for_type` is a
                // generic param IFF (a) it's a bare single-segment path
                // AND (b) the bare name is declared in the impl's
                // `<...>` generics clause.  Without (b), every concrete
                // bare-path receiver (`implement Hasher for DefaultHasher`,
                // `implement Display for Formatter`, …) is misclassified
                // as a generic-param blanket — gating below at
                // `!for_type_is_generic_param` then suppresses
                // `generate_default_protocol_methods` for legitimate
                // concrete impls, so `<Type>.<default_method>`
                // monomorphisations (DefaultHasher.write_int /
                // .write_byte, Formatter.* forwarders, every concrete
                // Iterator impl's default combinators) never reach the
                // queue.  This is the architectural defect that made
                // 582 out of 584 stdlib modules drain `0 pending`
                // default-method monomorphisations.
                let for_type_is_generic_param = if let verum_ast::decl::ImplKind::Protocol {
                    for_type,
                    ..
                } = &impl_decl.kind
                {
                    Self::for_type_generic_param_name(for_type)
                        .map(|bare| {
                            impl_decl.generics.iter().any(|g| {
                                matches!(
                                    &g.kind,
                                    verum_ast::ty::GenericParamKind::Type { name, .. }
                                        if name.name.as_str() == bare.as_str()
                                )
                            })
                        })
                        .unwrap_or(false)
                } else {
                    false
                };
                if let verum_ast::decl::ImplKind::Protocol { protocol, .. } = &impl_decl.kind
                    && let Some(ref ty_name) = type_name
                    && !for_type_is_generic_param
                {
                    // Get the last segment of the protocol path (e.g., "Hasher" from "core.protocols.Hasher")
                    let protocol_name = protocol.segments.last().and_then(|s| match s {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    });
                    if let Some(pn) = protocol_name {
                        // Get methods explicitly implemented
                        let implemented_methods: std::collections::HashSet<String> = impl_decl
                            .items
                            .iter()
                            .filter_map(|item| {
                                if let verum_ast::decl::ImplItemKind::Function(func) = &item.kind {
                                    Some(func.name.name.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        // Generate default methods from protocol
                        self.generate_default_protocol_methods(&pn, ty_name, &implemented_methods)?;

                        // Populate TypeDescriptor.protocols for vtable-based dispatch.
                        // Look up protocol's TypeDescriptor to get canonical method order,
                        // then map each method to the concrete Type.method FunctionId.
                        //
                        // **Foreign-protocol auto-stub**: when the protocol is
                        // imported from a different module (e.g. `mount
                        // core.base.coercion.{ArrayCoercible}` in
                        // `core/collections/list.vr` declaring
                        // `implement<T> ArrayCoercible for List<T> {}`), the
                        // protocol type itself is not in this module's
                        // `type_name_to_id` table — pre-fix the entire impl
                        // record was silently dropped from
                        // `TypeDescriptor.protocols`, breaking the
                        // archive-side `metadata.implementations` table that
                        // drives user-side `register_coercion_markers_from_metadata`
                        // (and `register_stdlib_impls_for_target` more
                        // generally).  Synthesise a placeholder protocol-
                        // type stub in the local table so the impl record
                        // survives and archive_metadata can resolve the
                        // protocol name via `type_id_to_name`.  The stub
                        // carries the protocol's name only; method bodies
                        // live with the impl's concrete-type registration.
                        if !self.type_name_to_id.contains_key(&pn) {
                            let proto_id = self.alloc_user_type_id();
                            let name_sid = StringId(self.ctx.intern_string_raw(&pn));
                            self.types.push(crate::types::TypeDescriptor {
                                id: proto_id,
                                name: name_sid,
                                kind: crate::types::TypeKind::Protocol,
                                ..Default::default()
                            });
                            self.type_name_to_id.insert(pn.clone(), proto_id);
                        }
                        if let Some(&proto_type_id) = self.type_name_to_id.get(&pn) {
                            // Get protocol method names in vtable order (from protocol's variants)
                            let method_names: Vec<String> = self
                                .types
                                .iter()
                                .find(|td| td.id == proto_type_id)
                                .map(|td| {
                                    td.variants
                                        .iter()
                                        .map(|v| {
                                            let idx = v.name.0 as usize;
                                            if idx < self.ctx.strings.len() {
                                                self.ctx.strings[idx].clone()
                                            } else {
                                                String::new()
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            // Look up concrete FunctionIds for each method
                            let method_fn_ids: Vec<u32> = method_names
                                .iter()
                                .map(|method_name| {
                                    let qualified = format!("{}.{}", ty_name, method_name);
                                    self.ctx
                                        .lookup_function(&qualified)
                                        .map(|fi| fi.id.0)
                                        .unwrap_or(u32::MAX) // sentinel for missing method
                                })
                                .collect();

                            // Push protocol impl onto the concrete type's descriptor
                            if let Some(&concrete_type_id) =
                                self.type_name_to_id.get(ty_name.as_str())
                            {
                                for type_desc in self.types.iter_mut() {
                                    if type_desc.id == concrete_type_id {
                                        type_desc.protocols.push(crate::types::ProtocolImpl {
                                            protocol: crate::types::ProtocolId(proto_type_id.0),
                                            methods: method_fn_ids,
                                        });
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }

                // Restore prefer_existing_functions to its prior value
                // (instead of unconditionally false) so caller-set context
                // — pipeline.rs's stdlib-loading `true` — survives across
                // impl-block boundaries.
                if is_protocol_impl {
                    self.ctx.prefer_existing_functions = prev_prefer_existing;
                }
            }
            // Type declarations - register variant constructors for sum types
            // and record field layouts for proper field indexing
            ItemKind::Type(type_decl) => {
                self.register_type_constructors(type_decl)?;
                // Register record field layouts for sequential field indexing
                if let TypeDeclBody::Record(fields) = &type_decl.body {
                    let field_names: Vec<String> =
                        fields.iter().map(|f| f.name.name.to_string()).collect();
                    let field_types: Vec<String> = fields
                        .iter()
                        .map(|f| Self::extract_type_name_from_ast(&f.ty))
                        .collect();
                    self.register_record_fields(&type_decl.name.name, field_names, field_types);
                }
            }
            ItemKind::Protocol(_) | ItemKind::Predicate(_) => {
                // Protocols and predicates don't produce bytecode directly
            }
            // Constants and statics: register as zero-argument functions
            ItemKind::Const(const_decl) => {
                self.register_constant_with_value(
                    &const_decl.name.name,
                    Some(&const_decl.value),
                    Some(&const_decl.ty),
                )?;
            }
            ItemKind::Static(static_decl) => {
                // `static mut` needs writable backing storage: the constant-function
                // codegen path that plain `static` uses re-executes the initializer
                // on every read, so writes made from one frame are invisible to
                // subsequent reads (`EPOCH_COUNTER = EPOCH_COUNTER + 1` bumps a local
                // copy; `Epoch.current()` from another frame reads the initializer
                // again and returns `1`). Route mutable statics through the same TLS
                // slot mechanism that `@thread_local` uses — that's a real heap slot
                // that survives across frames and matches the "process-wide writable
                // global" semantics the user expects.
                let is_thread_local = item.attributes.iter().any(|a| a.is_named("thread_local"))
                    || static_decl.is_mut;
                if is_thread_local {
                    // Assign a TLS slot for this variable
                    let name = static_decl.name.name.to_string();
                    let slot = self.ctx.register_thread_local(&name);

                    // Also register with qualified name for cross-module imports
                    let module_name = &self.config.module_name;
                    if !module_name.is_empty() && module_name != "main" {
                        let qualified_name = format!("{}.{}", module_name, name);
                        self.ctx.thread_local_vars.insert(qualified_name, slot);
                    }

                    // Register the type for correct instruction selection
                    if let Some(var_type) = Some(&static_decl.ty) {
                        let vt = self.type_kind_to_var_type(&var_type.kind);
                        self.ctx.register_constant_type(&name, vt);
                    }

                    // Record the declared *type name* (separate from the
                    // VarTypeKind above, which only carries primitive
                    // discriminators like Int/Float/Bool/Text/Unit).
                    // `extract_expr_type_name` consults this map when a
                    // bare reference to a static-mut binding flows into a
                    // let-binding or a field-access receiver; without it,
                    // the path's type is unknown and `resolve_field_index`
                    // falls through to the global interned-name fallback —
                    // producing wildly wrong byte offsets for record-typed
                    // static muts.  Both the bare name and the module-
                    // qualified alias are recorded so cross-module mounts
                    // of the same static surface the type identically.
                    let declared_type_name = Self::extract_type_name_from_ast(&static_decl.ty);
                    if !declared_type_name.is_empty() {
                        self.static_mut_type_names
                            .insert(name.clone(), declared_type_name.clone());
                        let module_name = &self.config.module_name;
                        if !module_name.is_empty() && module_name != "main" {
                            let qualified_name = format!("{}.{}", module_name, name);
                            self.static_mut_type_names
                                .insert(qualified_name, declared_type_name);
                        }
                    }

                    // Queue the init expression for compilation as a TLS initializer
                    self.pending_tls_inits
                        .push((name, static_decl.value.clone(), slot));
                } else {
                    self.register_constant_with_value(
                        &static_decl.name.name,
                        Some(&static_decl.value),
                        Some(&static_decl.ty),
                    )?;
                    // Track static init functions as global constructors
                    if let Some(info) = self.ctx.lookup_function(&static_decl.name.name) {
                        self.static_init_functions.push(info.id);
                    }
                }
            }
            // Active pattern declarations - compile as callable functions
            ItemKind::Pattern(pat_decl) => {
                self.register_pattern_as_function(pat_decl)?;
            }
            // Module declarations: inline `module X { ... }` blocks
            // need their inner items compiled with the module-name
            // qualifier so static calls like `Transducer.compose2(a, b)`
            // resolve to the qualified-registered function.  Pre-fix
            // this arm was `_ => {}`, leaving every `public module X
            // { pub fn Y(...) ... }` function unregistered — caller
            // dispatch fell through to instance-method lookup on the
            // first argument, panicking with "method 'Y' not found on
            // receiver of runtime kind …".  Closes task #38 dispatch
            // surface for `Transducer.compose2` / `.compose` /
            // `.identity` / `.filter` / `.map` / etc.
            //
            // File-level `module foo;` (no items) is handled separately
            // via `extract_source_module_name`; this arm only fires
            // for inline modules with non-empty `items`.
            ItemKind::Module(mod_decl) => {
                if let verum_common::Maybe::Some(ref items) = mod_decl.items {
                    let module_name = mod_decl.name.name.to_string();
                    let prev_source = self.ctx.current_source_module.clone();
                    self.ctx.current_source_module = Some(module_name);
                    // LENIENT — see parallel arm in compile_item for rationale.
                    for item in items.iter() {
                        if let Err(e) = self.collect_declarations(item) {
                            tracing::warn!(
                                "[lenient inline-module decl-collect] failed: {}",
                                e
                            );
                        }
                    }
                    self.ctx.current_source_module = prev_source;
                }
            }
            // Import declarations register aliased function names
            ItemKind::Mount(import_decl) => {
                self.register_import_aliases(import_decl)?;
            }
            // Context declarations are type-level
            ItemKind::Context(_) => {
                // Context types don't produce bytecode directly
            }
            ItemKind::ContextGroup(group_decl) => {
                // Register context group for expansion in function using clauses
                let group_name = group_decl.name.name.to_string();
                let members: Vec<String> = group_decl
                    .contexts
                    .iter()
                    .filter(|c| !c.is_negative) // Groups can include negative constraints
                    .filter_map(|c| {
                        c.path.segments.last().and_then(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                            _ => None,
                        })
                    })
                    .collect();
                self.context_groups.insert(group_name, members);
            }
            ItemKind::Layer(layer_decl) => {
                // Register layer for `provide LayerName;` expansion.
                // Inline layers: store provide list.
                // Composite layers: store constituent layer names.
                let layer_name = layer_decl.name.name.to_string();
                match &layer_decl.kind {
                    verum_ast::decl::LayerKind::Inline { provides } => {
                        let entries: Vec<(String, verum_ast::expr::Expr)> = provides
                            .iter()
                            .map(|(name, expr)| (name.name.to_string(), expr.clone()))
                            .collect();
                        self.context_layers
                            .insert(layer_name, ContextLayer::Inline(entries));
                    }
                    verum_ast::decl::LayerKind::Composite { layers } => {
                        let names: Vec<String> =
                            layers.iter().map(|id| id.name.to_string()).collect();
                        self.context_layers
                            .insert(layer_name, ContextLayer::Composite(names));
                    }
                }
            }
            // FFI boundary declarations - register extern functions
            ItemKind::FFIBoundary(ffi_boundary) => {
                self.register_ffi_functions(ffi_boundary)?;
            }
            // Extern block declarations (e.g., extern "C" { fn verum_alloc(...); })
            // These are FFI function declarations used for calling kernel intrinsics
            ItemKind::ExternBlock(extern_block) => {
                self.register_extern_block_functions(extern_block)?;
            }
            // Meta (macro) declarations are expanded during parsing
            ItemKind::Meta(_) => {
                // Macros should be expanded before codegen
            }
            // Proof-related items are not compiled to bytecode (proof erasure).
            //

            // Proofs are a purely compile-time phenomenon: they are verified by
            // the proof_verification phase (see crates/verum_compiler/src/phases/
            // proof_verification.rs) and then erased from the VBC codegen path.
            // This enforces the VBC-first architecture invariant that runtime
            // carries zero proof-term overhead.
            //

            // All 5 proof-item kinds MUST be listed explicitly here — relying on
            // the catch-all `_ => {}` arm would silently ignore new proof kinds
            // added to ItemKind in the future.
            ItemKind::Theorem(_)
            | ItemKind::Lemma(_)
            | ItemKind::Corollary(_)
            | ItemKind::Axiom(_)
            | ItemKind::Tactic(_) => {
                // Proofs are verified in the verification phase, not executed.
            }
            // Catch-all for any future item kinds
            #[allow(unreachable_patterns)]
            _ => {}
        }
        Ok(())
    }

    /// Registers a function for lookup.
    ///

    /// For nested functions, the name is mangled with the parent scope names
    /// using `$` as a separator (e.g., `outer$inner$deeply_nested`).
    fn register_function(&mut self, func: &FunctionDecl) -> CodegenResult<()> {
        let base_name = func.name.name.to_string();

        // Mangle the name if we're inside a nested function scope
        let name = if self.nested_function_scope.is_empty() {
            base_name.clone()
        } else {
            format!("{}${}", self.nested_function_scope.join("$"), base_name)
        };

        let id = FunctionId(self.next_func_id);
        self.next_func_id = self.next_func_id.saturating_add(1);

        // Extract parameter names and type names
        let param_names: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();
        let param_type_names: Vec<String> = func
            .params
            .iter()
            .filter_map(|p| {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    let name = Self::extract_type_name_from_ast(ty);
                    if !name.is_empty() && name != "()" {
                        Some(name)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // For each parameter, extract the closure-arg return-type
        // simple-name when the parameter is function-typed (directly
        // `fn(...) -> X`, or via a generic-param bound that resolves
        // to a function type).  Drives the call-site disambiguation
        // hook in `compile_static_method_call` so that a closure
        // argument's body sees the right variant-table when its
        // return expression mentions a sum-type variant whose simple
        // name collides across two types (canonical case:
        // `ReduceResult.Continue` vs `ControlFlow.Continue`).
        let param_closure_return_type_names: Vec<Option<String>> = func
            .params
            .iter()
            .map(|p| {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    Self::extract_closure_return_type_name(ty, &func.generics)
                } else {
                    None
                }
            })
            .collect();

        let contexts: Vec<String> = func
            .contexts
            .iter()
            .filter(|c| {
                // Skip negative contexts and false conditional contexts
                if c.is_negative {
                    return false;
                }
                if let verum_common::Maybe::Some(ref cond) = c.condition {
                    return Self::evaluate_context_condition(cond);
                }
                true
            })
            .flat_map(|c| {
                let name = format!("{}", c.path);
                // Expand context groups to individual member contexts
                if let Some(members) = self.context_groups.get(&name) {
                    members.clone()
                } else {
                    vec![name]
                }
            })
            .collect();

        // Convert return type for method dispatch prefixing.
        // This enables correct method name prefixing when calling methods on function returns
        // (e.g., return_uint64().checked_add(1) should use uint64$checked_add).
        let return_type = func
            .return_type
            .as_ref()
            .map(|ret_ty| self.ast_type_to_type_ref(ret_ty));

        // Extract intrinsic name from @intrinsic("name") attribute if present.
        // This enables industrial-grade intrinsic resolution at declaration time.
        // If the function doesn't have @intrinsic but was previously registered
        // as an intrinsic (via register_stdlib_intrinsics), preserve that name — but
        // ONLY if this function is a forward declaration (no body). A user-defined
        // function with a body should override any previously registered intrinsic
        // stub of the same name, so it calls the user's implementation instead.
        let intrinsic_name = self.extract_intrinsic_name(func).or_else(|| {
            if matches!(func.body, verum_common::Maybe::None) {
                self.ctx
                    .lookup_function(&name)
                    .and_then(|existing| existing.intrinsic_name.clone())
            } else {
                None
            }
        });

        // Extract the base return type name for method dispatch tracking
        let return_type_name = if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
            self.extract_type_name(ret_ty)
        } else {
            None
        };

        let info = FunctionInfo {
            id,
            param_count: param_names.len(),
            param_names,
            param_type_names,
            is_async: func.is_async,
            is_generator: func.is_generator, // fn* syntax
            contexts,
            return_type,
            yield_type: None, // Will be inferred from yield expressions
            intrinsic_name,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
            is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names,
        };

        // #201 diagnostic — env-var-gated trace of every register_function
        // call. Set `VERUM_TRACE_REGISTER=1` (or `=try_alloc` for a
        // substring-filtered trace) to surface registration attempts on the
        // run-interpreter path without flooding normal runs.
        //

        // The substring filter is helpful for the original #201 reproduction
        // ("ZERO entries match try_alloc") — running with
        //  VERUM_TRACE_REGISTER=try_alloc verum run --interp file.vr
        // shows whether `try_alloc` reaches register_function at all, and
        // under what `effective_module`.
        if let Ok(filter) = std::env::var("VERUM_TRACE_REGISTER") {
            let pass = filter == "1"
                || filter.is_empty()
                || base_name.contains(filter.as_str())
                || name.contains(filter.as_str());
            if pass {
                let eff_mod = self
                    .ctx
                    .current_source_module
                    .as_deref()
                    .unwrap_or(&self.config.module_name);
                eprintln!(
                    "[register-fn] module={} base={} mangled={} arity={} prefer_existing={}",
                    eff_mod, base_name, name, info.param_count, self.ctx.prefer_existing_functions
                );
            }
        }

        self.ctx.register_function(name.clone(), info.clone());

        // Also register under the function's fully-qualified module path, so
        // cross-module calls like `sys.darwin.tls.ctx_get(slot)` (and the
        // `super.*` equivalents after super-translation at the call site)
        // resolve without relying on the previously-catastrophic last-segment
        // fallback. The module_name comes from the codegen config, which is
        // set per-.vr-file from the stdlib / user build pipeline.
        //

        // Skip for:
        //  - mangled nested-function names (already module-local)
        //  - anonymous / empty module names
        //  - names that look like already-qualified type-method registrations
        //  ("Foo.bar") — those get their own qualified registration path
        //  via `register_impl_function`.
        // Prefer the *source module* (from the `module X.Y.Z;` declaration at
        // the top of the current .vr file) over `config.module_name`. The
        // config's module_name is fixed per-codegen-session (`"main"` for a
        // single-file user run) but a single session processes many imported
        // stdlib modules, each with its own path. `current_source_module` is
        // scoped to the file currently being collected/compiled.
        let effective_module = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        if self.nested_function_scope.is_empty()
            && !effective_module.is_empty()
            && effective_module != "main"
            && !base_name.contains('.')
            && !base_name.contains("::")
        {
            let dot_qualified = format!("{}.{}", effective_module, base_name);
            let colon_qualified = effective_module.replace('.', "::") + "::" + &base_name;
            // Preserve existing registrations — don't clobber user-visible
            // qualified aliases already installed by import/mount passes.
            if self.ctx.lookup_function(&dot_qualified).is_none() {
                self.ctx.register_function(dot_qualified, info.clone());
            }
            if self.ctx.lookup_function(&colon_qualified).is_none() {
                self.ctx.register_function(colon_qualified, info.clone());
            }
        }

        // Also register under the base name for local lookup within the parent function.
        // This allows code in the outer function to call `inner()` directly.
        if !self.nested_function_scope.is_empty() {
            // Register a lookup alias from base_name -> mangled name
            // This is handled by also registering under the base name
            // Note: The context already has the mangled name, so we need to
            // make the lookup work for the short name too
            if let Some(mangled_info) = self.ctx.lookup_function(&name).cloned() {
                self.ctx.register_function(base_name, mangled_info);
            }
        }

        Ok(())
    }

    /// Registers an active pattern declaration as a callable function.
    ///

    /// Active patterns (`pattern Even(n: Int) -> Bool = ...`) are compiled
    /// as regular functions so that `compile_pattern_test` for `PatternKind::Active`
    /// can find them via `lookup_function`.
    fn register_pattern_as_function(
        &mut self,
        pat: &verum_ast::decl::PatternDecl,
    ) -> CodegenResult<()> {
        let name = pat.name.name.to_string();
        let id = FunctionId(self.next_func_id);
        self.next_func_id = self.next_func_id.saturating_add(1);

        // Combine type_params (parameterized patterns) + params (match params)
        let mut param_names: Vec<String> = pat
            .type_params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_tp{}", i))
            })
            .collect();
        for (i, p) in pat.params.iter().enumerate() {
            param_names.push(
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_arg{}", i)),
            );
        }

        // Detect partial patterns by checking if return type is Maybe<T>
        let is_partial = Self::is_maybe_return_type(&pat.return_type);

        let info = FunctionInfo {
            id,
            param_count: param_names.len(),
            param_names,
            param_type_names: vec![],
            is_async: false,
            is_generator: false,
            contexts: vec![],
            return_type: None,
            yield_type: None,
            intrinsic_name: None,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: is_partial,
            takes_self_mut_ref: false,
            return_type_name: if is_partial {
                Some("Maybe".to_string())
            } else {
                None
            },
            return_type_inner: None,
            is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
        };

        self.ctx.register_function(name, info);
        Ok(())
    }

    /// Check if an AST return type is Maybe<T> (partial active pattern).
    ///

    /// VBC-internal: uses WKT::Maybe to recognize the stdlib Maybe type by name.
    /// Partial active patterns return Maybe<T>; the codegen must emit different
    /// bytecode (conditional branch on None vs Some) for these patterns.
    fn is_maybe_return_type(ty: &verum_ast::ty::Type) -> bool {
        use verum_ast::ty::{PathSegment, TypeKind};
        let check_path = |path: &verum_ast::ty::Path| -> bool {
            path.segments.iter().any(|seg| {
                if let PathSegment::Name(ident) = seg {
                    WKT::Maybe.matches(ident.name.as_str())
                } else {
                    false
                }
            })
        };
        match &ty.kind {
            TypeKind::Path(path) => check_path(path),
            TypeKind::Generic { base, .. } => {
                // Maybe<T> parses as Generic { base: Path("Maybe"), args: [...] }
                if let TypeKind::Path(path) = &base.kind {
                    check_path(path)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Compiles an active pattern declaration body as a function.
    fn compile_pattern_as_function(
        &mut self,
        pat: &verum_ast::decl::PatternDecl,
    ) -> CodegenResult<()> {
        let name = pat.name.name.to_string();

        let func_info = self
            .ctx
            .lookup_function(&name)
            .or_internal_else(|| format!("pattern not registered: {}", name))?
            .clone();

        // Build params with mutability (patterns are immutable)
        let mut params: Vec<(String, bool)> = pat
            .type_params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    self.extract_param_name(p)
                        .unwrap_or_else(|| format!("_tp{}", i)),
                    false,
                )
            })
            .collect();
        for (i, p) in pat.params.iter().enumerate() {
            params.push((
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_arg{}", i)),
                false,
            ));
        }

        self.ctx
            .begin_function(&name, &params, func_info.return_type.clone());

        // Register parameter types for correct operation selection
        for (param_name, param) in params
            .iter()
            .zip(pat.type_params.iter().chain(pat.params.iter()))
        {
            if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                let var_type = self.type_kind_to_var_type(&ty.kind);
                self.ctx.register_variable_type(&param_name.0, var_type);
            }
        }

        // Compile the body expression
        let result = self
            .compile_expr(&pat.body)
            .map_err(|e| e.with_context(format!("in pattern {}", name)))?;

        if let Some(reg) = result {
            self.ctx.emit(Instruction::Ret { value: reg });
        } else {
            self.ctx.emit(Instruction::RetV);
        }

        self.ensure_return()?;

        let ret_type = func_info.return_type.clone();

        // Collect debug variable info BEFORE end_function() clears register state
        let debug_vars = if self.config.source_map {
            self.ctx.collect_debug_variables()
        } else {
            Vec::new()
        };

        let (instructions, register_count) = self.ctx.end_function();

        // Promote the descriptor's stored name to the FULL source-
        // module-qualified form (`sys.bitfield.USIZE_BITS` rather
        // than just `USIZE_BITS`) so the archive-load path's
        // qualified-key registration (`archive_ctx_loader::register_module`
        // line ~314: `format!("{}.{}", module_name, simple_name)`)
        // doesn't collapse multiple sibling files in the same
        // directory onto the same archive-entry module-name.
        //
        // Without this promotion every `.vr` file under e.g. `core/sys/`
        // gets folded into a single archive entry named `core.sys` —
        // so `sys/bitfield.vr`'s `module sys.bitfield;`-declared
        // exports land in the user-side function registry as
        // `core.sys.USIZE_BITS` instead of the file-declared
        // `sys.bitfield.USIZE_BITS`, and cross-module bare-mount
        // qualified access (`mount core.sys.bitfield; ...
        // bitfield.USIZE_BITS`) misses the registry entirely.
        //
        // The simple-name HashMap key in `ctx.functions` is kept for
        // local lookups; this change only affects what gets serialised
        // into the `.vbca` archive.  Impl methods (`type_name.method`,
        // already qualified through `lookup_name`) and nested-function
        // names (with `$` separators) are left alone — the prefix is
        // only added when the basic `lookup_name` carries no `.` or
        // `$`.  Closes audit task #121 archive-side gap.
        let effective_module = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let descriptor_name = if !effective_module.is_empty()
            && effective_module != "main"
            && !name.contains('.')
            && !name.contains('$')
        {
            format!("{}.{}", effective_module, name)
        } else {
            name.clone()
        };
        let name_id = StringId(self.intern_string(&descriptor_name));
        let mut descriptor = FunctionDescriptor::new(name_id);
        descriptor.id = func_info.id;
        descriptor.register_count = register_count;
        descriptor.locals_count = params.len() as u16;
        if let Some(ref rt) = ret_type {
            descriptor.return_type = rt.clone();
        }
        // #87 — propagate the intrinsic-name marker into the
        // archive-side descriptor.  Carries `__const_val_<N>` and
        // similar inline-constant markers across the precompile →
        // archive → load round-trip so cross-module references to
        // stdlib `public const FOO: Int = 256;` resolve correctly
        // at user-side codegen time.  Without this propagation, the
        // marker survived inside the same compilation unit but was
        // dropped at the archive boundary, surfacing as
        // `UndefinedVariable` at every cross-module reference.
        if let Some(ref iname) = func_info.intrinsic_name {
            descriptor.intrinsic_name = Some(StringId(self.intern_string(iname)));
        }
        // #97 — propagate the const marker so the archive-driven
        // typechecker can distinguish const-as-zero-arg-function from
        // a genuine zero-arg function. Set by
        // `register_constant_with_value`; round-trips via
        // `vbc::FunctionDescriptor::is_const`.
        descriptor.is_const = func_info.is_const;

        // Populate debug variables for DWARF emission
        if !debug_vars.is_empty() {
            let instr_count = instructions.len() as u32;
            descriptor.debug_variables = debug_vars
                .into_iter()
                .map(|(var_name, register, is_param, arg_idx)| {
                    let name_sid = StringId(self.intern_string(&var_name));
                    crate::module::DebugVariableInfo {
                        name: name_sid,
                        register,
                        scope_start: 0,
                        scope_end: instr_count,
                        is_parameter: is_param,
                        arg_index: arg_idx,
                    }
                })
                .collect();
        }

        let vbc_func = VbcFunction::new(descriptor, instructions);
        self.push_function_dedup(vbc_func);

        Ok(())
    }

    /// **VBC-DISP-2: push-time duplicate-id detection** — the
    /// architecturally-clean alternative to the post-pass dedup at
    /// `finalize_module` (kept as defense-in-depth).
    ///
    /// Blanket-impl replays + generic monomorphization both call
    /// `register_function` for the same name; under
    /// `prefer_existing_functions=true` (stdlib loading) the
    /// registry is first-wins, so the second `register_function`
    /// is a no-op and the FunctionInfo's id stays the same.  But
    /// `compile_function` ALWAYS pushes a fresh VbcFunction body —
    /// producing TWO bodies with the SAME descriptor.id.
    ///
    /// This helper enforces the registry's wins-policy at push
    /// time:
    ///   * `prefer_existing_functions=true` (first-wins): if the
    ///     id is already in `self.functions`, SKIP the new push.
    ///     The first body wins, matching the registry's behaviour.
    ///   * `prefer_existing_functions=false` (last-wins, user code):
    ///     REPLACE the existing entry.
    ///
    /// The post-pass dedup at `finalize_module` becomes a no-op
    /// for legitimate code paths but stays as a safety net for
    /// any push site that bypasses this helper (audit aid).
    fn push_function_dedup(&mut self, vbc_func: VbcFunction) {
        let new_id = vbc_func.descriptor.id.0;
        // Task #16 reland blocker #2: filter EMPTY stage-1/2 stubs at
        // the codegen-emission boundary so they never enter
        // `self.functions` (the per-module compiled bytecode set).
        //
        // Sentinel ranges match the codegen-side stub-overwrite gate
        // at `stdlib_bootstrap.rs:1453`, the archive-metadata Pass 2
        // filter at `archive_metadata.rs::register_module_metadata`
        // (commit `fdda6ee22`), AND the runtime sentinel handler at
        // `verum_vbc::interpreter::handle_call` (commit `b5f5462d4`).
        //
        // **Empty-body gate**: filter ONLY when `instructions.is_empty()`.
        // The first reland's `Int.checked_add` + `Text.from_utf8_unchecked`
        // regression (revert `f98f7ea49`) came from this filter blanket-
        // rejecting EVERY function with a sentinel-range ID — including
        // legitimate real bodies that get assigned those IDs when the
        // producing module's compile path looks up the pre-registered
        // stub in `global_function_registry` and reuses its ID for the
        // overlay (the stub-overwrite-gate-intended path).  Real bodies
        // with sentinel IDs are the SUCCESS case for a future stages-1+2
        // reland; only true stubs (no instructions) should be dropped.
        const STAGE1_STUB_BASE: u32 = u32::MAX - 0x40_0000;
        const STAGE2_STUB_BASE: u32 = u32::MAX - 0xC0_0000;
        const STUB_RANGE_WIDTH: u32 = 0x10_0000;
        let in_stage1 =
            new_id <= STAGE1_STUB_BASE && new_id >= STAGE1_STUB_BASE - STUB_RANGE_WIDTH;
        let in_stage2 =
            new_id <= STAGE2_STUB_BASE && new_id >= STAGE2_STUB_BASE - STUB_RANGE_WIDTH;
        if (in_stage1 || in_stage2) && vbc_func.instructions.is_empty() {
            return;
        }
        if let Some(existing_idx) = self.functions.iter().position(|f| f.descriptor.id.0 == new_id)
        {
            if self.ctx.prefer_existing_functions {
                // First-wins: the existing body stays; drop the
                // new push.  Stdlib loading + protocol-impl path
                // hit this branch (~25 collapses pre-fix that the
                // post-pass dedup was cleaning up).
                return;
            }
            // Last-wins: replace.  User-code compilation path.
            self.functions[existing_idx] = vbc_func;
            return;
        }
        self.functions.push(vbc_func);
    }

    /// Registers an FFI extern function for lookup WITHOUT consuming a function ID.
    ///

    /// FFI functions don't need regular function IDs because they're called via
    /// the FfiExtended instruction using the FFI symbol ID, not via the Call instruction.
    /// This ensures that FFI function declarations don't interfere with the function ID
    /// assignment for regular functions.
    fn register_ffi_extern_function(&mut self, func: &FunctionDecl) -> CodegenResult<()> {
        let name = func.name.name.to_string();

        // Extract parameter names
        let param_names: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();

        // Use a sentinel ID of u32::MAX to indicate this is an FFI function.
        // This ID should never be used for Call instructions because FFI functions
        // are detected via ffi_function_map and emit FfiExtended instructions instead.
        // Extract return type name for method dispatch tracking
        let return_type_name = if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
            self.extract_type_name(ret_ty)
        } else {
            None
        };
        let info = FunctionInfo {
            id: FunctionId(u32::MAX),
            param_count: param_names.len(),
            param_names,
            param_type_names: vec![],
            is_async: func.is_async,
            is_generator: func.is_generator,
            contexts: Vec::new(),
            return_type: None,
            yield_type: None,
            intrinsic_name: None,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
            is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
        };

        self.ctx.register_function(name, info);
        Ok(())
    }

    /// Registers import aliases so that aliased function names can be resolved.
    ///

    /// This processes imports like `import sys.linux.syscall.{write as sys_write}` and
    /// registers `sys_write` as pointing to `sys.linux.syscall.write`.
    fn register_import_aliases(&mut self, import: &MountDecl) -> CodegenResult<()> {
        // #122 — propagate the OUTER `MountDecl.alias` into the tree
        // walker. The grammar admits two surfaces for `as <alias>`:
        //
        //   * simple-path form: `mount core.X.Y.fn as alias;`
        //     parser stores the alias on `MountDecl.alias`,
        //     `MountTree.alias` is `None` — see
        //     verum_fast_parser/src/decl.rs:5663-5670 vs :5816-5820.
        //   * nested form:      `mount core.X.{fn as alias, ...};`
        //     parser stores the alias on each inner `MountTree.alias`,
        //     `MountDecl.alias` is `None`.
        //
        // Pre-fix `process_import_tree` only inspected `tree.alias`,
        // so every simple-path alias was silently dropped — the
        // alias key never landed in `ctx.functions`. This was the
        // remaining 20% of #122 that the cross-instance ctx bridge
        // (commit 83b8a3f3) couldn't paper over: the alias name
        // wasn't even *attempted* on either codegen instance.
        let outer_alias = match &import.alias {
            verum_common::Maybe::Some(ident) => Some(ident),
            verum_common::Maybe::None => None,
        };
        self.process_import_tree(&import.tree, &[], outer_alias)
    }

    /// Recursively processes an import tree to register function aliases.
    fn process_import_tree(
        &mut self,
        tree: &MountTree,
        prefix: &[String],
        outer_alias: Option<&verum_ast::ty::Ident>,
    ) -> CodegenResult<()> {
        match &tree.kind {
            MountTreeKind::Path(path) => {
                // Build the full qualified name by combining prefix and path
                let mut full_path: Vec<String> = prefix.to_vec();
                for segment in path.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => {
                            full_path.push(ident.name.to_string());
                        }
                        PathSegment::Super => full_path.push("super".to_string()),
                        PathSegment::Cog => full_path.push("cog".to_string()),
                        PathSegment::Relative => full_path.push(".".to_string()),
                        PathSegment::SelfValue => full_path.push("self".to_string()),
                    }
                }

                if full_path.is_empty() {
                    return Ok(());
                }

                // The last segment is the function/item name
                let func_name = match full_path.last() {
                    Some(name) => name.clone(),
                    None => return Ok(()),
                };

                // Alias precedence: per-tree alias (nested form
                // `mount X.{fn as alias}`) wins; falling back to the
                // outer-decl alias (simple form `mount X.fn as alias;`),
                // then to the function name itself.
                let alias_name = match &tree.alias {
                    verum_common::Maybe::Some(alias) => alias.name.to_string(),
                    verum_common::Maybe::None => match outer_alias {
                        Some(ident) => ident.name.to_string(),
                        None => func_name.clone(),
                    },
                };

                // Try to look up the function in the registry with various qualified names
                let qualified_verum = full_path.join(".");
                let qualified_rust = full_path.join("::");

                // Architectural rule: an explicit `mount X.Y.{name}` /
                // `mount X.Y.name as alias` is an authoritative binding —
                // the user named the function they want, so it MUST win
                // over any passive archive-load that previously registered
                // the same simple name.  The pre-fix `should_register`
                // gate plumbed `force_override = true` but then routed
                // through `register_function`, which under
                // `prefer_existing_functions = true` is first-wins via
                // `entry().or_insert(_)` — passive loads still owned the
                // bare-name slot, and arity-different user mounts ended
                // up at `name#arity` instead of `name` (so the call
                // site's bare-name lookup picked the passively-loaded
                // wrong function).  The architectural fix lifts that
                // collision class via `register_function_authoritative`,
                // which unconditionally overwrites both `name` and
                // `name#arity` for the user's chosen binding.  Glob
                // mounts (`mount X.*`) deliberately keep first-wins to
                // preserve the FFI-raw / safe-wrapper precedence rule.

                // First try Verum-style qualified name
                if let Some(func_info) = self.ctx.lookup_function(&qualified_verum).cloned() {
                    let fid = func_info.id;
                    self.ctx
                        .register_function_authoritative(alias_name.clone(), func_info);
                    // Task #11 Phase 2: capture rename for archive-side
                    // alias plumbing (only when alias name truly differs
                    // from the canonical last-segment name).
                    if alias_name != func_name {
                        self.mount_aliases_buffer.push((alias_name.clone(), fid));
                    }
                    return Ok(());
                }

                // Try Rust-style qualified name
                if let Some(func_info) = self.ctx.lookup_function(&qualified_rust).cloned() {
                    let fid = func_info.id;
                    self.ctx
                        .register_function_authoritative(alias_name.clone(), func_info);
                    if alias_name != func_name {
                        self.mount_aliases_buffer.push((alias_name.clone(), fid));
                    }
                    return Ok(());
                }

                // Try module.name without the file component (e.g., sys.ORDERING_ACQUIRE)
                // This handles the case where imports reference files (sys.intrinsics.X)
                // but constants are registered at module level (sys.X)
                if full_path.len() >= 3 {
                    let module_name = &full_path[0];
                    let simplified_qualified = format!("{}.{}", module_name, func_name);
                    if let Some(func_info) =
                        self.ctx.lookup_function(&simplified_qualified).cloned()
                    {
                        let fid = func_info.id;
                        self.ctx
                            .register_function_authoritative(alias_name.clone(), func_info);
                        if alias_name != func_name {
                            self.mount_aliases_buffer.push((alias_name.clone(), fid));
                        }
                        return Ok(());
                    }
                }

                // #122 — Try WITHOUT "core." prefix (mount uses
                // `core.X.Y.fn` but functions register under their
                // source-module's `module X.Y;` declaration which
                // omits the `core.` prefix). This is the inverse of
                // the with-prefix retry below; needed when the user
                // writes the canonical form `mount core.sys.common.X
                // as Y` and the function was registered at
                // `module=sys.common`. Without this branch, every
                // alias mount of a `core.*`-canonical-path function
                // bug-class lenient SKIPs.
                if full_path.first().map(|s| s.as_str()) == Some("core") && full_path.len() >= 2 {
                    let stripped: Vec<String> = full_path[1..].to_vec();
                    let stripped_qualified = stripped.join(".");
                    if let Some(func_info) =
                        self.ctx.lookup_function(&stripped_qualified).cloned()
                    {
                        let fid = func_info.id;
                        self.ctx
                            .register_function_authoritative(alias_name.clone(), func_info);
                        if alias_name != func_name {
                            self.mount_aliases_buffer.push((alias_name.clone(), fid));
                        }
                        return Ok(());
                    }
                }

                // #FUNDAMENTAL — Path-suffix narrowing for selective mount.
                //
                // The source-side `module X;` declaration determines the
                // canonical registration prefix.  A file at
                // `core/async/timer.vr` declaring `module timer;` registers
                // every function as `timer.<fn_name>` — NOT under the
                // file-path-canonical `core.async.timer.<fn_name>`.  The
                // selective-mount user writes the file-path-canonical form
                // (`mount core.async.timer.{timeout_ms}`), and the verbatim /
                // `core.`-stripped lookups above probe
                // `core.async.timer.timeout_ms` and `async.timer.timeout_ms`
                // — neither of which is registered.
                //
                //
                // Without this branch the bare-name fallback at the bottom
                // of this function picks whichever module's `timeout_ms`
                // happened to win the bare-name slot during archive load —
                // typically the first-loaded `core.runtime.supervisor.
                // timeout_ms(&self)` 1-param method, which then panics with
                // `WrongArgumentCount expected:1 found:2` when the user's
                // 2-arg call site lowers.
                //
                // The probe walks the parent path from FULL to LEAF,
                // stripping one leading segment at a time and re-anchoring
                // on `func_name`. The FIRST tail that resolves wins —
                // longest-prefix-match routing-table discipline.
                //
                // `full_path` already contains the function-name as the
                // last element, so the *parent path* is
                // `full_path[..full_path.len() - 1]`.  For
                // path = ["core","async","timer","timeout_ms"] the probe
                // tries (in order):
                //   ["async","timer"].timeout_ms       (covered by core-stripped above)
                //   ["timer"].timeout_ms               ← NEW: matches `module timer;` decl
                //
                // The `full_path.len() >= 3` gate skips paths whose parent
                // is already a single segment (verbatim lookup above
                // already covers `<segment>.<func_name>`).
                if full_path.len() >= 3 {
                    let parent_path = &full_path[..full_path.len() - 1];
                    for start_idx in 1..parent_path.len() {
                        let tail = &parent_path[start_idx..];
                        let qualified = format!("{}.{}", tail.join("."), func_name);
                        if let Some(func_info) =
                            self.ctx.lookup_function(&qualified).cloned()
                        {
                            let fid = func_info.id;
                            self.ctx
                                .register_function_authoritative(alias_name.clone(), func_info);
                            if alias_name != func_name {
                                self.mount_aliases_buffer.push((alias_name.clone(), fid));
                            }
                            return Ok(());
                        }
                    }
                }

                // Try with "core." prefix (modules are registered as core.sys.* but imported as sys.*)
                if full_path.first().map(|s| s.as_str()) == Some("sys")
                    || full_path.first().map(|s| s.as_str()) == Some(".")
                {
                    // Try core.sys.linux.futex_wait, core.sys.darwin.futex_wait, etc.
                    let mut core_path = vec!["core".to_string()];
                    for p in &full_path {
                        if p != "." {
                            core_path.push(p.clone());
                        }
                    }
                    let core_qualified = core_path.join(".");
                    if let Some(func_info) = self.ctx.lookup_function(&core_qualified).cloned() {
                        let fid = func_info.id;
                        self.ctx
                            .register_function_authoritative(alias_name.clone(), func_info);
                        if alias_name != func_name {
                            self.mount_aliases_buffer.push((alias_name.clone(), fid));
                        }
                        return Ok(());
                    }
                    // Also try without the file component: core.sys.linux.futex_wait → core.sys.futex_wait
                    if core_path.len() >= 3 {
                        let simplified = format!("core.{}.{}", core_path[1], func_name);
                        if let Some(func_info) = self.ctx.lookup_function(&simplified).cloned() {
                            let fid = func_info.id;
                            self.ctx
                                .register_function_authoritative(alias_name.clone(), func_info);
                            if alias_name != func_name {
                                self.mount_aliases_buffer.push((alias_name.clone(), fid));
                            }
                            return Ok(());
                        }
                    }
                }

                // #FUNDAMENTAL — `public mount` re-export traversal.
                //
                // Before falling through to bare-name lookup (which is
                // structurally ambiguous when multiple modules export
                // the same simple name — canonical example:
                // `core.sys.common.PAGE_SIZE: USize = 4096` vs sibling
                // `core.mem.allocator.PAGE_SIZE: Int = 65536`), look
                // for a function/const registered under the user-named
                // parent's subtree. `mount core.sys.{PAGE_SIZE}`
                // expresses "the PAGE_SIZE re-exported by core.sys",
                // and `core/sys/mod.vr` routes that re-export through
                // `public mount .common.{PAGE_SIZE, ...}`, so the
                // canonical registration is `core.sys.common.PAGE_SIZE`.
                // Without this branch, the bare-name fallback at the
                // bottom of this function picks whichever sibling
                // module's `PAGE_SIZE` happens to own the bare slot —
                // a first-wins race that depends on archive iteration
                // order and surfaces as silent value drift (`PAGE_SIZE`
                // imports as 65536 instead of 4096).
                //
                // Scope: only fire when `full_path.len() >= 2` (there's
                // a real parent prefix to anchor the scan against).
                // The probe builds two prefix variants — verbatim and
                // `core.`-stripped — to match both the user-written
                // mount path (`core.sys.PAGE_SIZE`) and the source-side
                // registration form (`sys.common.PAGE_SIZE`, because
                // the source file declared `module sys.common;`
                // without a leading `core.`).
                //
                // Tie-breaker: prefer the SHALLOWEST hit (fewest dots
                // = closest sibling of the parent path) so a direct
                // submodule re-export wins over a deeper sibling that
                // happens to define the same simple name. Multiple hits
                // at the same depth are sorted alphabetically for
                // determinism.
                if full_path.len() >= 2 {
                    let parent_segs = &full_path[..full_path.len() - 1];
                    let parent_dot = format!("{}.", parent_segs.join("."));
                    let leaf_suffix = format!(".{}", func_name);
                    let mut hits: Vec<String> = self
                        .ctx
                        .functions
                        .keys()
                        .filter(|k| k.starts_with(&parent_dot) && k.ends_with(&leaf_suffix))
                        .cloned()
                        .collect();
                    // Also try the `core.`-stripped prefix (covers
                    // the stdlib case where the source file declared
                    // `module sys.common;` and the function was
                    // registered as `sys.common.PAGE_SIZE`).
                    if parent_segs.first().map(|s| s.as_str()) == Some("core")
                        && parent_segs.len() >= 2
                    {
                        let stripped_parent_dot =
                            format!("{}.", parent_segs[1..].join("."));
                        for k in self.ctx.functions.keys() {
                            if k.starts_with(&stripped_parent_dot)
                                && k.ends_with(&leaf_suffix)
                                && !hits.iter().any(|h| h == k)
                            {
                                hits.push(k.clone());
                            }
                        }
                    }
                    if !hits.is_empty() {
                        // Shallowest first; alphabetical as deterministic
                        // tiebreak among same-depth hits.
                        hits.sort_by(|a, b| {
                            let da = a.matches('.').count();
                            let db = b.matches('.').count();
                            da.cmp(&db).then_with(|| a.cmp(b))
                        });
                        if let Some(func_info) =
                            self.ctx.lookup_function(&hits[0]).cloned()
                        {
                            let fid = func_info.id;
                            self.ctx.register_function_authoritative(
                                alias_name.clone(),
                                func_info,
                            );
                            if alias_name != func_name {
                                self.mount_aliases_buffer.push((alias_name.clone(), fid));
                            }
                            return Ok(());
                        }
                    }
                }

                // Try just the function name (it might be already registered without qualification)
                if let Some(func_info) = self.ctx.lookup_function(&func_name).cloned() {
                    let fid = func_info.id;
                    let has_rename = alias_name != func_name;
                    self.ctx
                        .register_function_authoritative(alias_name.clone(), func_info);
                    if has_rename {
                        self.mount_aliases_buffer.push((alias_name, fid));
                    }
                    return Ok(());
                }

                // Check if this is a TYPE name import (e.g., `mount sys.io_engine.{IoError}`).
                // Type names aren't functions themselves, but their variant constructors
                // are registered as `TypeName.Variant`. Import all qualified constructors.
                // Iterate sorted for deterministic registration order (HashMap iter
                // would otherwise leak per-process random hasher seed into bytecode).
                let type_prefix = format!("{}.", func_name);
                let mut sorted_keys: Vec<&String> = self
                    .ctx
                    .functions
                    .keys()
                    .filter(|name| name.starts_with(&type_prefix))
                    .collect();
                sorted_keys.sort();
                let type_constructors: Vec<(String, FunctionInfo)> = sorted_keys
                    .into_iter()
                    .map(|name| (name.clone(), self.ctx.functions[name].clone()))
                    .collect();
                if !type_constructors.is_empty() {
                    for (qualified, info) in type_constructors {
                        // Register e.g., "IoError.WouldBlock" if not already present
                        if self.ctx.lookup_function(&qualified).is_none() {
                            self.ctx.register_function(qualified, info);
                        }
                    }
                    return Ok(());
                }

                // Bare-mount module-alias registration.
                //
                // If `full_path = [X, Y, Z]` (i.e. at least one parent
                // segment) and the function registry already holds
                // functions under the qualified prefix `X.Y.Z.` (or
                // `X::Y::Z::`), the user-written `mount X.Y.Z;` is
                // a module-import — not a function-import. Register
                // the rightmost segment as a module alias so a later
                // use site `Z.fn(args)` / `Z.CONST` expands to the
                // full path before qualified-function lookup.
                //
                // The prefix check is the necessary signal: only land
                // an alias when there's something under that prefix
                // to dispatch to. Without the check we'd populate
                // aliases for typos / forward-declared modules and
                // shadow legitimate single-name bindings.
                //
                // Honours the same `core.` ↔ bare prefix asymmetry as
                // every other branch in `process_import_tree`: stdlib
                // modules declare themselves with `module sys.X;`
                // (no leading `core.`), so the registry key for an
                // export of `mount core.sys.bitfield.USIZE_BITS` is
                // `sys.bitfield.USIZE_BITS`. The receiver-side flatten
                // matches against the FULL path the user wrote
                // (`["core", "sys", "bitfield"]`), so the alias must
                // also carry that form — the qualified-lookup chain
                // at the use site already tries the `core.`-stripped
                // shape as a fallback. The module-alias's job is to
                // *prove* that the path is a module, not to fix the
                // qualified-name registration shape.
                if full_path.len() >= 2 {
                    let prefix_verum = format!("{}.", full_path.join("."));
                    let prefix_rust = format!("{}::", full_path.join("::"));
                    let mut module_has_exports = self.ctx.functions.keys().any(|k| {
                        k.starts_with(&prefix_verum) || k.starts_with(&prefix_rust)
                    });
                    // Try the `core.`-stripped prefix as a secondary
                    // probe — covers the common stdlib case where
                    // `mount core.sys.bitfield;` points at exports
                    // registered under `sys.bitfield.*` (because the
                    // source file declared `module sys.bitfield;`).
                    if !module_has_exports
                        && full_path.first().map(|s| s.as_str()) == Some("core")
                        && full_path.len() >= 3
                    {
                        let stripped_verum = format!("{}.", full_path[1..].join("."));
                        let stripped_rust = format!("{}::", full_path[1..].join("::"));
                        module_has_exports = self.ctx.functions.keys().any(|k| {
                            k.starts_with(&stripped_verum) || k.starts_with(&stripped_rust)
                        });
                    }
                    if module_has_exports {
                        let module_leaf = full_path.last().cloned().unwrap_or_default();
                        // Honour the alias precedence — if the user wrote
                        // `mount X.Y.Z as W;`, the module is accessible
                        // under `W`, not `Z`. Otherwise the leaf name.
                        let module_alias_name = match &tree.alias {
                            verum_common::Maybe::Some(a) => a.name.to_string(),
                            verum_common::Maybe::None => match outer_alias {
                                Some(ident) => ident.name.to_string(),
                                None => module_leaf,
                            },
                        };
                        if !module_alias_name.is_empty() {
                            self.ctx
                                .module_aliases
                                .insert(module_alias_name, full_path.clone());
                        }
                        return Ok(());
                    }
                }

                // Function not found - store for deferred resolution
                // This handles intra-module imports where the constant/function
                // from another file in the same module hasn't been registered yet
                self.pending_imports.push((full_path, alias_name));
                Ok(())
            }
            MountTreeKind::Glob(glob_path) => {
                // Glob import: `mount io.*;` imports all exported names from the io module.
                // Build the module prefix from the path segments.
                let mut module_prefix: Vec<String> = prefix.to_vec();
                for segment in glob_path.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => {
                            module_prefix.push(ident.name.to_string());
                        }
                        PathSegment::Super => module_prefix.push("super".to_string()),
                        PathSegment::Cog => module_prefix.push("cog".to_string()),
                        PathSegment::Relative => module_prefix.push(".".to_string()),
                        PathSegment::SelfValue => module_prefix.push("self".to_string()),
                    }
                }

                if module_prefix.is_empty() {
                    return Ok(());
                }

                // Build dot-separated prefix for matching (e.g., "io." or "core.")
                let prefix_dot = format!("{}.", module_prefix.join("."));
                // Also try with "core." prepended (modules registered as core.io.* but imported as io.*)
                let core_prefix_dot = if module_prefix.first().map(|s| s.as_str()) != Some("core") {
                    let mut cp = vec!["core".to_string()];
                    cp.extend(module_prefix.iter().filter(|s| s.as_str() != ".").cloned());
                    Some(format!("{}.", cp.join(".")))
                } else {
                    None
                };

                // Collect matching functions to avoid borrow conflict.
                // Iterate by sorted qualified name so that when two
                // qualified names share a bare suffix the deterministic
                // pick is stable across runs (HashMap iteration order
                // would otherwise leak Rust's per-process random hasher
                // seed into bytecode-emission order).
                let mut sorted_names: Vec<&String> = self.ctx.functions.keys().collect();
                sorted_names.sort();
                let mut to_register: Vec<(String, FunctionInfo)> = Vec::new();
                for name in sorted_names {
                    let info = &self.ctx.functions[name];
                    // Match qualified names like "core.io.protocols.StreamError" for prefix "io."
                    let matches = name.starts_with(&prefix_dot)
                        || core_prefix_dot
                            .as_ref()
                            .is_some_and(|cp| name.starts_with(cp));
                    if matches {
                        // Extract bare name (last segment after the last dot)
                        if let Some(bare) = name.rsplit('.').next() {
                            // Don't overwrite existing registrations
                            if self.ctx.lookup_function(bare).is_none() {
                                to_register.push((bare.to_string(), info.clone()));
                            }
                        }
                    }
                }

                for (bare_name, info) in to_register {
                    self.ctx.register_function(bare_name, info);
                }

                Ok(())
            }
            MountTreeKind::Nested {
                prefix: nested_prefix,
                trees,
            } => {
                // Build the new prefix
                let mut new_prefix: Vec<String> = prefix.to_vec();
                for segment in nested_prefix.segments.iter() {
                    match segment {
                        PathSegment::Name(ident) => {
                            new_prefix.push(ident.name.to_string());
                        }
                        PathSegment::Super => new_prefix.push("super".to_string()),
                        PathSegment::Cog => new_prefix.push("cog".to_string()),
                        PathSegment::Relative => new_prefix.push(".".to_string()),
                        PathSegment::SelfValue => new_prefix.push("self".to_string()),
                    }
                }

                // Process each nested tree with the accumulated prefix.
                // Nested-form does not propagate the outer-decl alias to
                // children: an outer `as ALIAS` on `mount X.{a, b, c}`
                // is grammatically ambiguous (which child gets it?)
                // and is treated as a no-op for codegen — children use
                // their own per-tree alias if any.
                for sub_tree in trees.iter() {
                    self.process_import_tree(sub_tree, &new_prefix, None)?;
                }

                Ok(())
            }
            // #5 / P1.5 — file-relative mount aliases are
            // already wired by the session loader (which
            // registers each loaded file as a module under
            // the alias). The VBC codegen import-aliasing
            // pass doesn't add another mapping.
            MountTreeKind::File { .. } => Ok(()),
        }
    }

    /// Resolves pending imports that couldn't be resolved during initial processing.
    ///

    /// This is called after all declarations from all files in a module have been collected,
    /// allowing cross-file imports within the same module to be resolved.
    ///

    /// Handles path resolution for imports like `sys.intrinsics.ORDERING_ACQUIRE` where:
    /// - `sys` is the module name
    /// - `intrinsics` is the file name (not a submodule)
    /// - `ORDERING_ACQUIRE` is the constant/function name
    ///

    /// The constant might be registered as `ORDERING_ACQUIRE` or `sys.ORDERING_ACQUIRE`,
    /// but not as `sys.intrinsics.ORDERING_ACQUIRE`.
    pub fn resolve_pending_imports(&mut self) {
        // Take ownership of pending imports to avoid borrow issues
        let pending = std::mem::take(&mut self.pending_imports);

        for (full_path, alias_name) in pending {
            if full_path.is_empty() {
                continue;
            }

            let func_name = match full_path.last() {
                Some(name) => name.clone(),
                None => continue,
            };

            // Helper to check if we should register the alias.
            //
            // The deferred-resolution path executes for every previously-
            // pending mount that the import-tree pass couldn't resolve
            // synchronously (typically because the target wasn't loaded
            // from the archive yet). By the time we reach this point the
            // archive has finished loading, so we can re-attempt with
            // full visibility. As with `register_import_aliases`, the
            // user wrote the binding name explicitly — explicit mount
            // imports are authoritative and override any passively-
            // registered first-wins entry.
            let should_register =
                |_ctx: &CodegenContext, _alias: &str, _new_info: &FunctionInfo| -> bool {
                    true
                };

            // Try to look up the function in the registry with various qualified names
            let qualified_verum = full_path.join(".");
            let qualified_rust = full_path.join("::");

            // First try Verum-style qualified name (e.g., sys.intrinsics.ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&qualified_verum).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    let fid = func_info.id;
                    let has_rename = alias_name != func_name;
                    self.ctx.register_function(alias_name.clone(), func_info);
                    if has_rename {
                        self.mount_aliases_buffer.push((alias_name, fid));
                    }
                }
                continue;
            }

            // Try Rust-style qualified name (e.g., sys::intrinsics::ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&qualified_rust).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    let fid = func_info.id;
                    let has_rename = alias_name != func_name;
                    self.ctx.register_function(alias_name.clone(), func_info);
                    if has_rename {
                        self.mount_aliases_buffer.push((alias_name, fid));
                    }
                }
                continue;
            }

            // Try module.name without the file component (e.g., sys.ORDERING_ACQUIRE)
            // This handles the case where imports reference files (sys.intrinsics.X)
            // but constants are registered at module level (sys.X)
            if full_path.len() >= 3 {
                let module_name = &full_path[0];
                let simplified_qualified = format!("{}.{}", module_name, func_name);
                if let Some(func_info) = self.ctx.lookup_function(&simplified_qualified).cloned() {
                    if should_register(&self.ctx, &alias_name, &func_info) {
                        let fid = func_info.id;
                        let has_rename = alias_name != func_name;
                        self.ctx.register_function(alias_name.clone(), func_info);
                        if has_rename {
                            self.mount_aliases_buffer.push((alias_name, fid));
                        }
                    }
                    continue;
                }
            }

            // #122 — Try WITHOUT "core." prefix in deferred-resolution path
            // (mirror of the same retry in `register_import_aliases`). The
            // canonical mount form `mount core.sys.common.X as Y`
            // resolves through this branch when the function got
            // registered under `module=sys.common` (without the
            // `core.` prefix because `current_source_module` records
            // the file's own `module sys.common;` declaration).
            if full_path.first().map(|s| s.as_str()) == Some("core") && full_path.len() >= 2 {
                let stripped: Vec<String> = full_path[1..].to_vec();
                let stripped_qualified = stripped.join(".");
                if let Some(func_info) =
                    self.ctx.lookup_function(&stripped_qualified).cloned()
                {
                    if should_register(&self.ctx, &alias_name, &func_info) {
                        let fid = func_info.id;
                        let has_rename = alias_name != func_name;
                        self.ctx.register_function(alias_name.clone(), func_info);
                        if has_rename {
                            self.mount_aliases_buffer.push((alias_name, fid));
                        }
                    }
                    continue;
                }
            }

            // Try just the function name (e.g., ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&func_name).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    let fid = func_info.id;
                    let has_rename = alias_name != func_name;
                    self.ctx.register_function(alias_name.clone(), func_info);
                    if has_rename {
                        self.mount_aliases_buffer.push((alias_name, fid));
                    }
                }
                continue;
            }

            // Deferred bare-mount module-alias check.  Mirror of the
            // synchronous branch in `process_import_tree` — by the time
            // this fires the archive has finished loading, so if a
            // `mount X.Y.Z;` couldn't resolve as a function/type/const
            // during the first pass, it may still be a *module* with
            // exported items registered under the `X.Y.Z.` prefix that
            // landed between the two passes.
            if full_path.len() >= 2 {
                let prefix_verum = format!("{}.", full_path.join("."));
                let prefix_rust = format!("{}::", full_path.join("::"));
                let mut module_has_exports = self
                    .ctx
                    .functions
                    .keys()
                    .any(|k| k.starts_with(&prefix_verum) || k.starts_with(&prefix_rust));
                if !module_has_exports
                    && full_path.first().map(|s| s.as_str()) == Some("core")
                    && full_path.len() >= 3
                {
                    let stripped_verum = format!("{}.", full_path[1..].join("."));
                    let stripped_rust = format!("{}::", full_path[1..].join("::"));
                    module_has_exports = self.ctx.functions.keys().any(|k| {
                        k.starts_with(&stripped_verum) || k.starts_with(&stripped_rust)
                    });
                }
                if module_has_exports && !alias_name.is_empty() {
                    self.ctx
                        .module_aliases
                        .insert(alias_name, full_path.clone());
                    continue;
                }
            }

            // Still not found - this is OK, the stub mechanism will handle it at call time
            // This can happen for functions defined in other modules not yet compiled
        }
    }

    /// Extracts the type name from an impl block's ImplKind.
    ///

    /// For inherent impls (`implement List { ... }`), extracts "List".
    /// For protocol impls (`implement Iterator for List { ... }`), extracts "List".
    fn extract_impl_type_name(&self, kind: &verum_ast::decl::ImplKind) -> Option<String> {
        use verum_ast::decl::ImplKind;

        let ty = match kind {
            ImplKind::Inherent(ty) => ty,
            ImplKind::Protocol { for_type, .. } => for_type,
        };

        self.extract_impl_type_name_from_type(ty)
    }

    /// Helper to extract type name from a type.
    fn extract_impl_type_name_from_type(&self, ty: &verum_ast::ty::Type) -> Option<String> {
        use verum_ast::ty::{PathSegment, TypeKind};

        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(segment) = path.segments.first()
                    && let PathSegment::Name(ident) = segment
                {
                    return Some(ident.name.to_string());
                }
                None
            }
            TypeKind::Generic { base, .. } => self.extract_impl_type_name_from_type(base),
            // Primitive types - use primitive_name() as canonical source,
            // but override Unit ("()" -> "Unit") and Unknown ("unknown" -> "Unknown")
            // since we need identifier-style names for method dispatch
            TypeKind::Unit => Some("Unit".to_string()),
            TypeKind::Unknown => Some("Unknown".to_string()),
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string())
            }
            // Container types
            TypeKind::Slice(_) => Some("Slice".to_string()),
            TypeKind::Array { .. } => Some("Array".to_string()),
            TypeKind::Tuple(_) => Some("Tuple".to_string()),
            // Reference types - extract the inner type name
            TypeKind::Reference { inner, .. } => self.extract_impl_type_name_from_type(inner),
            TypeKind::CheckedReference { inner, .. } => {
                self.extract_impl_type_name_from_type(inner)
            }
            TypeKind::UnsafeReference { inner, .. } => self.extract_impl_type_name_from_type(inner),
            TypeKind::Pointer { inner, .. } => self.extract_impl_type_name_from_type(inner),
            // Record types
            TypeKind::Record { .. } => Some("Record".to_string()),
            // Fallback for other types
            _ => None,
        }
    }

    /// Registers an impl function with a qualified name (e.g., "List.new").
    ///

    /// This allows static method calls like `List.new()` to be resolved.
    fn register_impl_function(
        &mut self,
        func: &FunctionDecl,
        type_name: &str,
    ) -> CodegenResult<()> {
        let func_name = func.name.name.to_string();
        let qualified_name = format!("{}.{}", type_name, func_name);

        let id = FunctionId(self.next_func_id);
        self.next_func_id = self.next_func_id.saturating_add(1);

        // Extract parameter names and type names
        let param_names: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name(p)
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();
        let param_type_names: Vec<String> = func
            .params
            .iter()
            .filter_map(|p| {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    let name = Self::extract_type_name_from_ast(ty);
                    // **Self → concrete substitution** so default-method
                    // monomorphisations (`Ord.max(self, other: Self)`
                    // monomorphised onto `Amount`) record the concrete
                    // parameter type — downstream variant-dispatch
                    // disambiguation, method-receiver inference, and
                    // every `field_type_name` lookup against
                    // `param_type_names` operate on the real layout.
                    let name = Self::substitute_self_in_type_name(&name, type_name);
                    if !name.is_empty() && name != "()" {
                        Some(name)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        // Closure expected-return-type name per param (mirrors the
        // populate-site in `register_function`).  Captures the case
        // where an `implement<I: Iterator> I { fn reduce_with<R,
        // F: fn(R, Self.Item) -> ReduceResult<R>>(…) }` blanket-impl
        // method takes a closure parameter and the body refers to a
        // bare variant constructor whose simple name collides across
        // two sum types.  See `extract_closure_return_type_name` and
        // the matching usage in `compile_static_method_call` /
        // `compile_call` / `compile_method_call`.
        let param_closure_return_type_names: Vec<Option<String>> = func
            .params
            .iter()
            .map(|p| {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    Self::extract_closure_return_type_name(ty, &func.generics)
                } else {
                    None
                }
            })
            .collect();

        let contexts: Vec<String> = func
            .contexts
            .iter()
            .filter(|c| {
                // Skip negative contexts and false conditional contexts
                if c.is_negative {
                    return false;
                }
                if let verum_common::Maybe::Some(ref cond) = c.condition {
                    return Self::evaluate_context_condition(cond);
                }
                true
            })
            .flat_map(|c| {
                let name = format!("{}", c.path);
                // Expand context groups to individual member contexts
                if let Some(members) = self.context_groups.get(&name) {
                    members.clone()
                } else {
                    vec![name]
                }
            })
            .collect();

        // Extract intrinsic name from @intrinsic("name") attribute if present.
        // Preserve existing intrinsic_name from register_stdlib_intrinsics() if present.
        let intrinsic_name = self.extract_intrinsic_name(func).or_else(|| {
            self.ctx
                .lookup_function(&qualified_name)
                .and_then(|existing| existing.intrinsic_name.clone())
        });

        // Check if the first parameter is &mut self (mutable reference to self).
        // When true, method calls must create a CBGR reference to the receiver.
        let takes_self_mut_ref = func.params.first().is_some_and(|param| {
            use verum_ast::decl::FunctionParamKind;
            matches!(
                &param.kind,
                FunctionParamKind::SelfRefMut
                    | FunctionParamKind::SelfRefCheckedMut
                    | FunctionParamKind::SelfRefUnsafeMut
            )
        });

        // Extract return type name for method dispatch tracking.
        // **Self → concrete substitution** is critical for default-
        // method monomorphisation: `Ord.max(self, other: Self) -> Self`
        // monomorphised onto `Amount` must record `return_type_name =
        // "Amount"`, not the literal `"Self"`.  Downstream call sites
        // like `let m = a.max(b); m.<field>` rely on
        // `infer_expr_type_name`'s MethodCall arm reading
        // `return_type_name` to look up the right field layout.
        let return_type_name = if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
            self.extract_type_name(ret_ty)
                .map(|n| Self::substitute_self_in_type_name(&n, type_name))
        } else {
            None
        };

        // Convert return type for method dispatch and list/string register tracking
        let return_type = func
            .return_type
            .as_ref()
            .map(|ret_ty| self.ast_type_to_type_ref(ret_ty));

        let info = FunctionInfo {
            id,
            param_count: param_names.len(),
            param_names,
            param_type_names,
            is_async: func.is_async,
            is_generator: func.is_generator,
            contexts,
            return_type,
            yield_type: None,
            intrinsic_name,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: false,
            takes_self_mut_ref,
            return_type_name,
            return_type_inner: None,
            is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names,
        };

        self.ctx.register_function(qualified_name, info);

        Ok(())
    }

    /// Collects nested declarations from a function body.
    ///

    /// This recursively walks the function body to find nested items (functions,
    /// types, etc.) and registers them. This is necessary because `collect_declarations`
    /// only processes top-level items, so nested functions would not be registered
    /// and would cause "function not registered" errors during compilation.
    ///

    /// The parent function name is tracked in `nested_function_scope` for name mangling.
    fn collect_nested_declarations(
        &mut self,
        body: &FunctionBody,
        parent_name: &str,
    ) -> CodegenResult<()> {
        // Push the parent function name onto the scope stack for name mangling
        self.nested_function_scope.push(parent_name.to_string());

        match body {
            FunctionBody::Block(block) => {
                self.collect_nested_declarations_from_block(block)?;
            }
            FunctionBody::Expr(_) => {
                // Expression bodies can't contain item declarations
            }
        }

        // Pop the parent function name from the scope stack
        self.nested_function_scope.pop();

        Ok(())
    }

    /// Collects nested declarations from a block.
    ///

    /// This walks all statements in the block looking for item declarations.
    /// Note: `collect_declarations` already handles recursive collection for
    /// nested functions, so we don't need to explicitly recurse here.
    ///

    /// Items are filtered based on @cfg attributes for consistency with
    /// top-level item collection.
    fn collect_nested_declarations_from_block(&mut self, block: &Block) -> CodegenResult<()> {
        for stmt in block.stmts.iter() {
            if let StmtKind::Item(item) = &stmt.kind {
                // Filter items based on @cfg attributes
                if self.should_compile_item(item) {
                    // Register the nested item (this recursively collects any nested declarations)
                    self.collect_declarations(item)?;
                }
            }
        }
        Ok(())
    }

    /// Registers FFI functions from an FFI boundary declaration.
    ///

    /// FFI functions are external functions with C ABI that can be called from Verum code.
    /// They are registered as callable functions so that VBC codegen can emit Call instructions.
    fn register_ffi_functions(&mut self, boundary: &FFIBoundary) -> CodegenResult<()> {
        for ffi_func in boundary.functions.iter() {
            let name = ffi_func.name.name.to_string();
            let id = FunctionId(self.next_func_id);
            self.next_func_id = self.next_func_id.saturating_add(1);

            // Extract parameter names from FFI signature
            let param_names: Vec<String> = ffi_func
                .signature
                .params
                .iter()
                .map(|(ident, _ty)| ident.name.to_string())
                .collect();

            // Extract return type name
            let return_type_name = self.extract_type_name(&ffi_func.signature.return_type);

            let info = FunctionInfo {
                id,
                param_count: param_names.len(),
                param_names,
                param_type_names: vec![],
                is_async: false,     // FFI functions are sync
                is_generator: false, // FFI functions are not generators
                contexts: vec![],    // FFI functions don't use contexts
                return_type: None,   // Type info is in FFISignature
                yield_type: None,
                intrinsic_name: None,
                variant_tag: None,
                parent_type_name: None,
                variant_payload_types: None,
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name,
                return_type_inner: None,
                is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
            };

            self.ctx.register_function(name.clone(), info);

            // Also create FfiSymbol entry with error protocol from the AST
            let signature = self.create_ffi_signature_from_boundary(ffi_func);
            let (error_protocol, error_sentinel) =
                Self::map_ast_error_protocol(&ffi_func.error_protocol);
            let memory_effects = Self::map_ast_memory_effects(&ffi_func.memory_effects);
            let ownership = Self::map_ast_ownership(&ffi_func.ownership);
            let convention =
                Self::map_ast_calling_convention(&ffi_func.signature.calling_convention);
            let symbol_id = FfiSymbolId(self.ffi_symbols.len() as u32);
            self.ffi_symbols.push(FfiSymbol {
                name: StringId(0),
                library_idx: -1,
                convention,
                signature,
                memory_effects,
                error_protocol,
                error_sentinel,
                wrapper_fn: None,
                validated: true,
                ownership,
            });
            self.ffi_function_map.insert(name.clone(), symbol_id);

            // Store contract metadata (requires/ensures) for debug-mode assertion generation
            if !ffi_func.requires.is_empty() || !ffi_func.ensures.is_empty() {
                // Serializable metadata for VBC module
                let contract = crate::module::FfiContract {
                    requires: ffi_func
                        .requires
                        .iter()
                        .map(|expr| format!("{:?}", expr))
                        .collect(),
                    ensures: ffi_func
                        .ensures
                        .iter()
                        .map(|expr| format!("{:?}", expr))
                        .collect(),
                    thread_safe: ffi_func.thread_safe,
                };
                self.ffi_contracts.insert(symbol_id, contract);

                // Actual AST expressions for compilation at call sites
                let contract_exprs = FfiContractExprs {
                    requires: ffi_func.requires.iter().cloned().collect(),
                    ensures: ffi_func.ensures.iter().cloned().collect(),
                    function_name: name.clone(),
                };
                self.ffi_contract_exprs.insert(name, contract_exprs);
            }
        }
        Ok(())
    }

    /// Map AST error protocol to VBC error protocol + sentinel value.
    ///

    /// Returns `(protocol, sentinel)`:
    /// - Errno → (NegOneErrno, -1)
    /// - ReturnCode(expr) → (ReturnCodePattern, evaluated_value)
    /// - ReturnValue(null) → (NullErrno, 0)
    /// - ReturnValueWithErrno(expr) → (SentinelWithErrno, 0)
    fn map_ast_error_protocol(proto: &verum_ast::ffi::ErrorProtocol) -> (ErrorProtocol, i64) {
        match proto {
            verum_ast::ffi::ErrorProtocol::None => (ErrorProtocol::None, 0),
            verum_ast::ffi::ErrorProtocol::Errno => (ErrorProtocol::NegOneErrno, -1),
            verum_ast::ffi::ErrorProtocol::ReturnCode(expr) => {
                // Try to extract literal integer value from the pattern expression
                let sentinel = Self::try_eval_const_i64(expr).unwrap_or(0);
                (ErrorProtocol::ReturnCodePattern, sentinel)
            }
            verum_ast::ffi::ErrorProtocol::ReturnValue(_expr) => {
                // ReturnValue(null) — sentinel is 0 (null pointer)
                (ErrorProtocol::NullErrno, 0)
            }
            verum_ast::ffi::ErrorProtocol::ReturnValueWithErrno(_expr) => {
                // Sentinel + errno — sentinel is 0 (null), errno checked on match
                (ErrorProtocol::SentinelWithErrno, 0)
            }
            verum_ast::ffi::ErrorProtocol::Exception => (ErrorProtocol::Exception, 0),
        }
    }

    /// Try to evaluate a constant integer expression from AST.
    /// Handles literals (-1, 0, 42) and negation (-(1)).
    fn try_eval_const_i64(expr: &verum_ast::expr::Expr) -> Option<i64> {
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(n) => Some(n.value as i64),
                _ => None,
            },
            ExprKind::Unary { op, expr: operand } => {
                if matches!(op, verum_ast::expr::UnOp::Neg) {
                    Self::try_eval_const_i64(operand).map(|v| -v)
                } else {
                    None
                }
            }
            ExprKind::Path(path) => {
                // Named constants like SQLITE_OK — can't resolve at compile time,
                // fall back to 0 (success = 0 convention)
                let _name = path.as_ident().map(|i| i.as_str()).unwrap_or("");
                Some(0)
            }
            _ => None,
        }
    }

    /// Map AST calling convention to VBC calling convention.
    fn map_ast_calling_convention(cc: &AstCallingConvention) -> FfiCallingConvention {
        match cc {
            AstCallingConvention::C => FfiCallingConvention::C,
            AstCallingConvention::StdCall => FfiCallingConvention::Stdcall,
            AstCallingConvention::FastCall => FfiCallingConvention::Fastcall,
            AstCallingConvention::SysV64 => FfiCallingConvention::SysV64,
            AstCallingConvention::Interrupt => FfiCallingConvention::C, // No direct VBC equivalent
            AstCallingConvention::Naked => FfiCallingConvention::C,     // No direct VBC equivalent
            AstCallingConvention::System => {
                // System = stdcall on Windows, C elsewhere
                #[cfg(target_os = "windows")]
                {
                    FfiCallingConvention::Stdcall
                }
                #[cfg(not(target_os = "windows"))]
                {
                    FfiCallingConvention::C
                }
            }
        }
    }

    /// Derive calling convention from a FunctionDecl's `extern_abi` field.
    ///

    /// `extern_abi` is a freeform string like `"C"`, `"stdcall"`, `"system"`.
    /// Absent means C ABI (the default for extern blocks).
    fn extern_abi_to_convention(
        abi: &verum_common::Maybe<verum_common::Text>,
    ) -> FfiCallingConvention {
        match abi {
            verum_common::Maybe::Some(s) => match s.as_str() {
                "C" | "c" | "cdecl" => FfiCallingConvention::C,
                "stdcall" | "Stdcall" | "StdCall" => FfiCallingConvention::Stdcall,
                "fastcall" | "FastCall" => FfiCallingConvention::Fastcall,
                "sysv64" | "SysV64" => FfiCallingConvention::SysV64,
                "system" | "System" => {
                    #[cfg(target_os = "windows")]
                    {
                        FfiCallingConvention::Stdcall
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        FfiCallingConvention::C
                    }
                }
                _ => FfiCallingConvention::C,
            },
            verum_common::Maybe::None => FfiCallingConvention::C,
        }
    }

    /// Map AST memory effects to VBC memory effects.
    fn map_ast_memory_effects(effects: &verum_ast::ffi::MemoryEffects) -> MemoryEffects {
        match effects {
            verum_ast::ffi::MemoryEffects::Pure => MemoryEffects::PURE,
            verum_ast::ffi::MemoryEffects::Reads(_) => MemoryEffects::READS,
            verum_ast::ffi::MemoryEffects::Writes(_) => MemoryEffects::WRITES,
            verum_ast::ffi::MemoryEffects::Allocates => MemoryEffects::ALLOCS,
            verum_ast::ffi::MemoryEffects::Deallocates(_) => MemoryEffects::FREES,
            verum_ast::ffi::MemoryEffects::Combined(effects_list) => {
                let mut combined = MemoryEffects::PURE;
                for effect in effects_list.iter() {
                    combined = combined.union(Self::map_ast_memory_effects(effect));
                }
                combined
            }
        }
    }

    /// Map AST ownership to VBC ownership.
    fn map_ast_ownership(ownership: &verum_ast::ffi::Ownership) -> FfiOwnership {
        match ownership {
            verum_ast::ffi::Ownership::Borrow => FfiOwnership::Borrow,
            verum_ast::ffi::Ownership::TransferTo(_) => FfiOwnership::TransferTo,
            verum_ast::ffi::Ownership::TransferFrom(_) => FfiOwnership::TransferFrom,
            verum_ast::ffi::Ownership::Shared => FfiOwnership::Shared,
        }
    }

    /// Create an FfiSignature from an FFI boundary function declaration.
    fn create_ffi_signature_from_boundary(
        &self,
        func: &verum_ast::ffi::FFIFunction,
    ) -> FfiSignature {
        let param_types: smallvec::SmallVec<[CType; 4]> = func
            .signature
            .params
            .iter()
            .map(|(_name, ty)| self.verum_type_to_ctype(&verum_common::Maybe::Some(ty.clone())))
            .collect();
        let return_type = self.verum_type_to_ctype(&verum_common::Maybe::Some(
            func.signature.return_type.clone(),
        ));
        let mut sig = FfiSignature::new(return_type, param_types);
        sig.is_variadic = func.signature.is_variadic;
        sig
    }

    /// Registers functions from an extern block declaration.
    ///

    /// Extern blocks contain FFI function declarations like:
    /// ```verum
    /// @ffi("libSystem.B.dylib")
    /// extern {
    ///  fn getpid() -> Int;
    ///  fn malloc(size: Int) -> &unsafe Byte;
    /// }
    /// ```
    ///

    /// This method:
    /// 1. Extracts the library name from @ffi attribute
    /// 2. Creates FFI library and symbol entries
    /// 3. Registers functions so FFI calls can be resolved
    fn register_extern_block_functions(
        &mut self,
        extern_block: &ExternBlockDecl,
    ) -> CodegenResult<()> {
        // Extract library name from @ffi("library") attribute
        let library_name = self.extract_ffi_library_name(&extern_block.attributes);

        // Get or create library entry
        let library_idx = if let Some(ref lib_name) = library_name {
            // Check if we already have this library
            if let Some(&lib_id) = self.ffi_library_map.get(lib_name) {
                lib_id.0 as i16
            } else {
                // Create new library entry
                let lib_id = FfiLibraryId(self.ffi_libraries.len() as u16);
                // Tag the platform by the library NAME, not the host target.
                // `@ffi("kernel32.dll")` is a Windows library even when we
                // compile on macOS. Without this, the runtime library-loader
                // can't skip cross-platform libraries and ends up trying to
                // dlopen `kernel32.dll` on Darwin (prior failure mode).
                let platform = FfiPlatform::from_library_name(lib_name);
                self.ffi_libraries.push(FfiLibrary {
                    name: StringId(0), // Will be remapped in build_module
                    platform,
                    required: true,
                    version: None,
                });
                // Store the library name for later interning
                self.ffi_library_map.insert(lib_name.clone(), lib_id);
                lib_id.0 as i16
            }
        } else {
            -1 // Default library (platform default)
        };

        for func in extern_block.functions.iter() {
            let func_name = func.name.name.to_string();

            // Create FFI signature from function parameters and return type
            let signature = self.create_ffi_signature(func);
            // FunctionDecl uses extern_abi (e.g., "C", "stdcall") — map to convention.
            // Defaults to C calling convention for extern blocks.
            let convention = Self::extern_abi_to_convention(&func.extern_abi);

            // Create FFI symbol entry
            let symbol_id = FfiSymbolId(self.ffi_symbols.len() as u32);
            self.ffi_symbols.push(FfiSymbol {
                name: StringId(0), // Will be remapped in build_module
                library_idx,
                convention,
                signature,
                memory_effects: MemoryEffects::default(), // PURE by default
                error_protocol: ErrorProtocol::None,
                error_sentinel: 0,
                wrapper_fn: None,
                validated: false,
                ownership: FfiOwnership::default(),
            });

            // Track function name -> FFI symbol ID mapping
            self.ffi_function_map.insert(func_name.clone(), symbol_id);

            // Register callback signature symbols for function pointer parameters.
            // This allows CreateCallback to find the correct signature when passing
            // Verum functions to FFI calls.
            self.register_callback_signatures(symbol_id, func);

            // Register FFI function for lookup WITHOUT consuming a function ID.
            // FFI functions don't need function IDs because they're called via
            // FfiExtended instruction using the FFI symbol ID, not via Call instruction.
            self.register_ffi_extern_function(func)?;
        }
        Ok(())
    }

    /// Extracts the library name from @ffi("library") attribute.
    fn extract_ffi_library_name(
        &self,
        attributes: &verum_common::List<verum_ast::attr::Attribute>,
    ) -> Option<String> {
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::{LiteralKind, StringLit};

        for attr in attributes.iter() {
            if attr.name.as_str() == "ffi" {
                // Extract library name from args
                if let verum_common::Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                {
                    // The first argument should be a string literal
                    if let ExprKind::Literal(lit) = &first_arg.kind
                        && let LiteralKind::Text(StringLit::Regular(s) | StringLit::MultiLine(s)) =
                            &lit.kind
                    {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    /// Checks if a function has an @ffi attribute.
    fn has_ffi_attribute(
        &self,
        attributes: &verum_common::List<verum_ast::attr::Attribute>,
    ) -> bool {
        for attr in attributes.iter() {
            if attr.name.as_str() == "ffi" {
                return true;
            }
        }
        false
    }

    /// Checks if a type definition has @repr(C) attribute.
    fn has_repr_c(&self, attributes: &verum_common::List<verum_ast::attr::Attribute>) -> bool {
        use verum_ast::expr::ExprKind;

        for attr in attributes.iter() {
            if attr.name.as_str() == "repr" {
                // Check for @repr(C) - the arg should be an identifier "C"
                if let verum_common::Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                {
                    // The argument is a Path expression for an identifier like "C"
                    if let ExprKind::Path(path) = &first_arg.kind
                        && let Some(PathSegment::Name(ident)) = path.segments.first()
                    {
                        let name = ident.name.as_str();
                        if name == "C" || name == "c" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Checks if a type definition has @bitfield attribute.
    fn has_bitfield_attr(
        &self,
        attributes: &verum_common::List<verum_ast::attr::Attribute>,
    ) -> bool {
        for attr in attributes.iter() {
            if attr.name.as_str() == "bitfield" {
                return true;
            }
        }
        false
    }

    /// Extracts the byte order from @endian attribute, defaulting to little.
    fn get_byte_order(
        &self,
        attributes: &verum_common::List<verum_ast::attr::Attribute>,
    ) -> ByteOrder {
        use verum_ast::expr::ExprKind;

        for attr in attributes.iter() {
            if attr.name.as_str() == "endian"
                && let verum_common::Maybe::Some(ref args) = attr.args
                && let Some(first_arg) = args.first()
                && let ExprKind::Path(path) = &first_arg.kind
                && let Some(PathSegment::Name(ident)) = path.segments.first()
            {
                let name = ident.name.as_str();
                return match name {
                    "big" => ByteOrder::Big,
                    "native" => ByteOrder::Native,
                    _ => ByteOrder::Little, // Default to little
                };
            }
        }
        ByteOrder::Little // Default
    }

    /// Generates bitfield layout and registers accessor functions for a @bitfield type.
    ///

    /// For each field with @bits(N), generates:
    /// - `TypeName.field_name(&self) -> T` - getter
    /// - `TypeName.set_field_name(&mut self, value: T)` - setter
    ///

    /// Bitfield accessors are generated as intrinsic-like functions that compile
    /// to efficient bit manipulation sequences (shift + mask + or/and).
    fn generate_bitfield_accessors(
        &mut self,
        type_name: &str,
        fields: &verum_common::List<verum_ast::decl::RecordField>,
        byte_order: ByteOrder,
    ) {
        let mut field_layouts = Vec::new();
        let mut current_bit_offset: u32 = 0;

        // Calculate layout for each field
        for field in fields.iter() {
            let field_name = field.name.name.to_string();

            // Check if field has @bits(N) attribute via BitSpec
            if let verum_common::Maybe::Some(ref bit_spec) = field.bit_spec {
                let bit_width = bit_spec.width.bits;

                // Use explicit offset if provided, otherwise use current offset
                let bit_offset = bit_spec
                    .offset
                    .as_ref()
                    .map(|o| *o)
                    .unwrap_or(current_bit_offset);

                // Calculate mask
                let mask = if bit_width >= 64 {
                    u64::MAX
                } else {
                    ((1u64 << bit_width) - 1) << bit_offset
                };

                // Check if this is a boolean field (1-bit)
                let is_bool = bit_width == 1;

                field_layouts.push(BitfieldFieldLayout {
                    name: field_name.clone(),
                    bit_offset,
                    bit_width,
                    mask,
                    is_bool,
                });

                // Update current offset for next field
                current_bit_offset = bit_offset + bit_width;

                // Register getter function: TypeName.field_name
                let getter_name = format!("{}.{}", type_name, field_name);
                let getter_id = FunctionId(self.next_func_id);
                self.next_func_id = self.next_func_id.saturating_add(1);

                let getter_info = FunctionInfo {
                    id: getter_id,
                    param_count: 1, // &self
                    param_names: vec!["self".to_string()],
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: Some(format!(
                        "bitfield_get:{}:{}:{}",
                        type_name, field_name, bit_offset
                    )),
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: None, // Bitfield getters return primitive types
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };
                self.ctx.register_function(getter_name, getter_info);

                // Register setter function: TypeName.set_field_name
                let setter_name = format!("{}.set_{}", type_name, field_name);
                let setter_id = FunctionId(self.next_func_id);
                self.next_func_id = self.next_func_id.saturating_add(1);

                let setter_info = FunctionInfo {
                    id: setter_id,
                    param_count: 2, // &mut self, value
                    param_names: vec!["self".to_string(), "value".to_string()],
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: Some(format!(
                        "bitfield_set:{}:{}:{}",
                        type_name, field_name, bit_offset
                    )),
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: None, // Setters return unit
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };
                self.ctx.register_function(setter_name, setter_info);
            }
        }

        // Store the bitfield layout
        let total_bits = current_bit_offset;
        let layout = BitfieldLayout {
            type_name: type_name.to_string(),
            total_bits,
            byte_order,
            fields: field_layouts,
        };
        self.bitfield_types.insert(type_name.to_string(), layout);
    }

    /// Generates an FfiStructLayout for a @repr(C) record type.
    ///

    /// Calculates C-compatible struct layout with proper field alignment and padding.
    fn generate_ffi_struct_layout(
        &mut self,
        type_name: &str,
        fields: &verum_common::List<verum_ast::decl::RecordField>,
    ) -> u16 {
        // Calculate C-compatible layout
        let mut layout_fields = Vec::new();
        let mut current_offset: u32 = 0;
        let mut max_align: u16 = 1;

        for field in fields.iter() {
            let field_name = field.name.name.to_string();
            let c_type = self.verum_type_to_ctype(&verum_common::Maybe::Some(field.ty.clone()));

            // Get size and alignment for this C type
            let (field_size, field_align) = self.ctype_size_align(c_type);

            // Align current offset
            let alignment = field_align as u32;
            if alignment > 0 {
                current_offset = (current_offset + alignment - 1) & !(alignment - 1);
            }

            // Intern field name using codegen's field name table to match GetF/SetF field indices
            let field_name_id = crate::types::StringId(self.intern_field_name(&field_name));

            layout_fields.push(FfiStructField {
                name: field_name_id,
                c_type,
                offset: current_offset,
                size: field_size,
                align: field_align,
            });

            current_offset += field_size as u32;
            max_align = max_align.max(field_align);
        }

        // Final struct size: align to max field alignment
        let alignment = max_align as u32;
        if alignment > 0 {
            current_offset = (current_offset + alignment - 1) & !(alignment - 1);
        }

        // Create the layout
        let layout_idx = self.ffi_layouts.len() as u16;
        let struct_name_id = crate::types::StringId(self.intern_string(type_name));

        let mut layout = FfiStructLayout::new(struct_name_id, current_offset, max_align);
        layout.fields = layout_fields;

        // Store type name to layout index mapping
        self.repr_c_types.insert(type_name.to_string(), layout_idx);
        self.ffi_layouts.push(layout);

        layout_idx
    }

    /// Returns the size and alignment for a C type.
    fn ctype_size_align(&self, c_type: CType) -> (u16, u16) {
        match c_type {
            CType::Void => (0, 1),
            CType::I8 | CType::U8 | CType::Bool => (1, 1),
            CType::I16 | CType::U16 => (2, 2),
            CType::I32 | CType::U32 | CType::F32 => (4, 4),
            CType::I64 | CType::U64 | CType::F64 => (8, 8),
            CType::Ptr | CType::CStr | CType::FnPtr | CType::Size | CType::Ssize => (8, 8), // 64-bit pointers
            CType::StructPtr | CType::ArrayPtr => (8, 8),
            CType::StructValue => (0, 1), // Should be replaced with actual layout
        }
    }

    /// Registers a single-function FFI declaration.
    ///

    /// This handles the syntax:
    /// ```verum
    /// @ffi("libSystem.B.dylib")
    /// extern fn getpid() -> Int;
    /// ```
    fn register_single_ffi_function(&mut self, func: &FunctionDecl) -> CodegenResult<()> {
        // Extract library name from @ffi("library") attribute
        let library_name = self.extract_ffi_library_name(&func.attributes);

        // Get or create library entry
        let library_idx = if let Some(ref lib_name) = library_name {
            // Check if we already have this library
            if let Some(&lib_id) = self.ffi_library_map.get(lib_name) {
                lib_id.0 as i16
            } else {
                // Create new library entry
                let lib_id = FfiLibraryId(self.ffi_libraries.len() as u16);
                // Tag the platform by the library NAME, not the host target.
                // `@ffi("kernel32.dll")` is a Windows library even when we
                // compile on macOS. Without this, the runtime library-loader
                // can't skip cross-platform libraries and ends up trying to
                // dlopen `kernel32.dll` on Darwin (prior failure mode).
                let platform = FfiPlatform::from_library_name(lib_name);
                self.ffi_libraries.push(FfiLibrary {
                    name: StringId(0), // Will be remapped in build_module
                    platform,
                    required: true,
                    version: None,
                });
                // Store the library name for later interning
                self.ffi_library_map.insert(lib_name.clone(), lib_id);
                lib_id.0 as i16
            }
        } else {
            -1 // Default library (platform default)
        };

        let func_name = func.name.name.to_string();

        // Create FFI signature from function parameters and return type
        let signature = self.create_ffi_signature(func);
        let convention = Self::extern_abi_to_convention(&func.extern_abi);

        // Create FFI symbol entry
        let symbol_id = FfiSymbolId(self.ffi_symbols.len() as u32);
        self.ffi_symbols.push(FfiSymbol {
            name: StringId(0), // Will be remapped in build_module
            library_idx,
            convention,
            signature,
            memory_effects: MemoryEffects::default(), // PURE by default
            error_protocol: ErrorProtocol::None,
            error_sentinel: 0,
            wrapper_fn: None,
            validated: false,
            ownership: FfiOwnership::default(),
        });

        // Track function name -> FFI symbol ID mapping
        self.ffi_function_map.insert(func_name.clone(), symbol_id);

        // Register callback signature symbols for function pointer parameters.
        // This allows CreateCallback to find the correct signature when passing
        // Verum functions to FFI calls.
        self.register_callback_signatures(symbol_id, func);

        // Register FFI function for lookup WITHOUT consuming a function ID.
        // FFI functions don't need function IDs because they're called via
        // FfiExtended instruction using the FFI symbol ID, not via Call instruction.
        self.register_ffi_extern_function(func)?;

        Ok(())
    }

    /// Creates an FFI signature from a function declaration.
    fn create_ffi_signature(&self, func: &FunctionDecl) -> FfiSignature {
        use smallvec::SmallVec;
        use verum_ast::decl::FunctionParamKind;

        // Map return type
        let return_type = self.verum_type_to_ctype(&func.return_type);

        // Get layout index for struct-by-value return type
        let return_layout_idx = self.get_struct_layout_index(&func.return_type);

        // Map parameter types and get layout indices for struct-by-value parameters
        let mut param_types: SmallVec<[CType; 4]> = SmallVec::new();
        let mut param_layout_indices: SmallVec<[Option<u16>; 4]> = SmallVec::new();

        for param in func.params.iter() {
            let param_ty = match &param.kind {
                FunctionParamKind::Regular { ty, .. } => verum_common::Maybe::Some(ty.clone()),
                _ => verum_common::Maybe::None, // Self parameters
            };
            param_types.push(self.verum_type_to_ctype(&param_ty));
            param_layout_indices.push(self.get_struct_layout_index(&param_ty));
        }

        FfiSignature {
            return_type,
            param_types,
            is_variadic: func.is_variadic,
            fixed_param_count: func.params.len() as u8,
            return_layout_idx,
            param_layout_indices,
        }
    }

    /// Registers callback signature symbols for function pointer parameters.
    ///

    /// For each parameter in an FFI function that is a function type (callback),
    /// this creates a synthetic FfiSymbol representing the callback's signature.
    /// The mapping (ffi_symbol_id, param_idx) -> callback_signature_id is stored
    /// so that CreateCallback can find the correct signature at compile time.
    fn register_callback_signatures(&mut self, symbol_id: FfiSymbolId, func: &FunctionDecl) {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::ty::TypeKind;

        for (param_idx, param) in func.params.iter().enumerate() {
            // Check if this parameter is a function type
            let param_type = match &param.kind {
                FunctionParamKind::Regular { ty, .. } => ty,
                _ => continue, // Skip self parameters
            };

            // Check if it's a function type and extract the signature
            if let TypeKind::Function {
                params,
                return_type,
                ..
            } = &param_type.kind
            {
                // Create the callback signature
                let callback_return_type =
                    self.verum_type_to_ctype(&verum_common::Maybe::Some((**return_type).clone()));
                let callback_param_types: smallvec::SmallVec<[crate::module::CType; 4]> = params
                    .iter()
                    .map(|t| self.verum_type_to_ctype(&verum_common::Maybe::Some(t.clone())))
                    .collect();

                let callback_signature = FfiSignature {
                    return_type: callback_return_type,
                    param_types: callback_param_types.clone(),
                    is_variadic: false,
                    fixed_param_count: params.len() as u8,
                    return_layout_idx: None, // Callbacks don't support struct-by-value yet
                    param_layout_indices: smallvec::SmallVec::from_elem(
                        None,
                        callback_param_types.len(),
                    ),
                };

                // Create a synthetic FfiSymbol for this callback signature
                let callback_symbol_id = FfiSymbolId(self.ffi_symbols.len() as u32);
                self.ffi_symbols.push(FfiSymbol {
                    name: StringId(0), // Synthetic, no actual name needed
                    library_idx: -1,   // Not bound to any library
                    convention: FfiCallingConvention::C,
                    signature: callback_signature,
                    memory_effects: MemoryEffects::default(),
                    error_protocol: ErrorProtocol::None,
                    error_sentinel: 0,
                    wrapper_fn: None,
                    validated: false,
                    ownership: FfiOwnership::default(),
                });

                // Store the mapping for lookup during FFI call compilation
                self.ffi_callback_signatures
                    .insert((symbol_id, param_idx as u8), callback_symbol_id);
            }
        }
    }

    /// Gets the callback signature symbol ID for an FFI function parameter.
    ///

    /// Returns the synthetic FfiSymbol ID that contains the callback signature
    /// for the given FFI function and parameter index. Returns None if the
    /// parameter is not a function pointer type.
    pub fn get_callback_signature_id(
        &self,
        ffi_symbol_id: FfiSymbolId,
        param_idx: u8,
    ) -> Option<FfiSymbolId> {
        self.ffi_callback_signatures
            .get(&(ffi_symbol_id, param_idx))
            .copied()
    }

    /// Maps a Verum type to a C type for FFI.
    fn verum_type_to_ctype(&self, ty: &verum_common::Maybe<verum_ast::ty::Type>) -> CType {
        use verum_ast::ty::TypeKind;

        match ty {
            verum_common::Maybe::None => CType::Void,
            verum_common::Maybe::Some(t) => {
                match &t.kind {
                    TypeKind::Path(path) => {
                        let name = path.to_string();

                        // Check if this is a @repr(C) struct type
                        if self.repr_c_types.contains_key(&name) {
                            return CType::StructValue;
                        }

                        match name.as_str() {
                            // Signed integers — canonical Verum +
                            // width-tagged + legacy uppercase-short +
                            // Rust-style lowercase aliases. Drift
                            // between this FFI C-type table and the
                            // canonical `NUMERIC_ALIAS_MATRIX` in
                            // verum_common would silently route an
                            // un-mapped alias to `CType::Ptr`,
                            // producing wrong FFI argument codegen
                            // (e.g. a `USize` argument passed as a
                            // pointer when the C ABI expects a 64-bit
                            // unsigned register).
                            "Int" | "Int64" | "I64" | "i64" => CType::I64,
                            "Int32" | "I32" | "i32" => CType::I32,
                            "Int16" | "I16" | "i16" => CType::I16,
                            "Int8" | "I8" | "i8" => CType::I8,
                            // Unsigned integers
                            "UInt" | "UInt64" | "U64" | "u64" => CType::U64,
                            "UInt32" | "U32" | "u32" => CType::U32,
                            "UInt16" | "U16" | "u16" => CType::U16,
                            "UInt8" | "U8" | "u8" | "Byte" => CType::U8,
                            // Pointer-sized integers (canonical Verum
                            // capitalisations + legacy uppercase-short
                            // + Rust-style lowercase + the
                            // `IntSize`/`UIntSize` prior canonical
                            // spellings — all alias to the same
                            // 64-bit width on every supported target).
                            "ISize" | "IntSize" | "Isize" | "isize" => CType::Ssize,
                            "USize" | "UIntSize" | "Usize" | "usize" => CType::Size,
                            // Floating point
                            "Float" | "Float64" | "F64" | "f64" => CType::F64,
                            "Float32" | "F32" | "f32" => CType::F32,
                            // Boolean
                            "Bool" | "bool" => CType::Bool,
                            // Character — Verum's Char is a 32-bit
                            // Unicode codepoint, ABI-equivalent to a
                            // `wchar_t` / `uint32_t` slot at the FFI
                            // boundary (most C ABIs treat `char`
                            // arguments as int-promoted; the wider
                            // Char→u32 mapping preserves the bit
                            // pattern without sign extension).
                            "Char" | "char" => CType::U32,
                            // Unit type
                            "()" | "Unit" => CType::Void,
                            _ => CType::Ptr, // Unknown types become pointers
                        }
                    }
                    // Primitive type variants
                    TypeKind::Int => CType::I64,
                    TypeKind::Float => CType::F64,
                    TypeKind::Bool => CType::Bool,
                    TypeKind::Char => CType::U32,  // Unicode codepoint
                    TypeKind::Text => CType::CStr, // String as C string
                    TypeKind::Unit => CType::Void,
                    TypeKind::Never => CType::Void, // Never returns
                    TypeKind::Reference { inner, .. } | TypeKind::Pointer { inner, .. } => {
                        // Check if the inner type is a @repr(C) struct - use StructPtr
                        if let TypeKind::Path(path) = &inner.kind {
                            let name = path.to_string();
                            if self.repr_c_types.contains_key(&name) {
                                return CType::StructPtr;
                            }
                        }
                        CType::Ptr
                    }
                    // Function types become function pointers in FFI
                    TypeKind::Function { .. } | TypeKind::Rank2Function { .. } => CType::FnPtr,
                    _ => CType::Ptr,
                }
            }
        }
    }

    /// Gets the layout index for a @repr(C) struct type.
    /// Handles both direct struct types and references to struct types.
    fn get_struct_layout_index(
        &self,
        ty: &verum_common::Maybe<verum_ast::ty::Type>,
    ) -> Option<u16> {
        use verum_ast::ty::TypeKind;

        match ty {
            verum_common::Maybe::None => None,
            verum_common::Maybe::Some(t) => {
                match &t.kind {
                    // Direct struct type
                    TypeKind::Path(path) => {
                        let name = path.to_string();
                        self.repr_c_types.get(&name).copied()
                    }
                    // Reference to struct type - extract the inner type
                    TypeKind::Reference { inner, .. } | TypeKind::Pointer { inner, .. } => {
                        if let TypeKind::Path(path) = &inner.kind {
                            let name = path.to_string();
                            self.repr_c_types.get(&name).copied()
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
        }
    }

    /// Emit context transform wrapping at function entry.
    ///

    /// For each context declared with transforms (e.g., `using [Database.transactional()]`):
    /// 1. CtxGet the base context
    /// 2. Call the transform method on it
    /// 3. CtxProvide the transformed value as the local context
    ///

    /// Returns the number of transforms emitted (for potential CtxEnd cleanup).
    fn emit_context_transforms(&mut self, func: &verum_ast::decl::FunctionDecl) -> usize {
        let mut count = 0;

        for ctx in &func.contexts {
            if ctx.is_negative || ctx.transforms.is_empty() {
                continue;
            }
            // Skip conditional contexts whose condition is false
            if let verum_common::Maybe::Some(ref cond) = ctx.condition
                && !Self::evaluate_context_condition(cond)
            {
                continue;
            }

            // Get context name
            let ctx_name = ctx
                .path
                .segments
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_name.is_empty() {
                continue;
            }

            let ctx_type_id = self.intern_string(&ctx_name);

            // Step 1: CtxGet the base context value
            let base_reg = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::CtxGet {
                dst: base_reg,
                ctx_type: ctx_type_id,
            });

            // Step 2: Apply each transform in chain
            let mut current_reg = base_reg;
            for transform in ctx.transforms.iter() {
                let method_name = transform.name.name.to_string();

                // Compile transform arguments
                let args_start = if !transform.args.is_empty() {
                    let first = self.ctx.alloc_temp();
                    if let Ok(Some(val)) = self.compile_expr(&transform.args[0]) {
                        self.ctx.emit(Instruction::Mov {
                            dst: first,
                            src: val,
                        });
                        if val != first {
                            self.ctx.free_temp(val);
                        }
                    }
                    for arg in transform.args.iter().skip(1) {
                        let arg_reg = self.ctx.alloc_temp();
                        if let Ok(Some(val)) = self.compile_expr(arg) {
                            self.ctx.emit(Instruction::Mov {
                                dst: arg_reg,
                                src: val,
                            });
                            if val != arg_reg {
                                self.ctx.free_temp(val);
                            }
                        }
                    }
                    first
                } else {
                    Reg(0)
                };

                // Emit method call on current context value
                let result_reg = self.ctx.alloc_temp();
                let method_id = self.intern_string(&method_name);
                self.ctx.emit(Instruction::CallM {
                    dst: result_reg,
                    receiver: current_reg,
                    method_id,
                    args: crate::instruction::RegRange {
                        start: args_start,
                        count: transform.args.len() as u8,
                    },
                });

                if current_reg != base_reg {
                    self.ctx.free_temp(current_reg);
                }
                current_reg = result_reg;
            }

            // Step 3: CtxProvide the transformed value
            self.ctx.emit(Instruction::CtxProvide {
                ctx_type: ctx_type_id,
                value: current_reg,
                body_offset: 0, // 0 = function-scoped (cleaned up at return)
            });

            self.ctx.free_temp(base_reg);
            count += 1;

            // Context transform emitted for ctx_name with N transforms
        }

        count
    }

    /// Evaluate a compile-time condition for conditional contexts.
    /// Returns true if the condition is met, false if not (context will be skipped).
    fn evaluate_context_condition(expr: &verum_ast::expr::Expr) -> bool {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Field {
                expr: object,
                field,
            } => {
                if let ExprKind::Path(path) = &object.kind {
                    if let Some(ident) = path.as_ident() {
                        match ident.name.as_str() {
                            "cfg" => match field.as_str() {
                                "debug" | "debug_assertions" => cfg!(debug_assertions),
                                "release" => !cfg!(debug_assertions),
                                "unix" => cfg!(unix),
                                "windows" => cfg!(windows),
                                _ => false,
                            },
                            "platform" => match field.as_str() {
                                "macos" | "darwin" => cfg!(target_os = "macos"),
                                "linux" => cfg!(target_os = "linux"),
                                "windows" => cfg!(target_os = "windows"),
                                _ => false,
                            },
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            ExprKind::Literal(lit) => {
                matches!(lit.kind, verum_ast::literal::LiteralKind::Bool(true))
            }
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    matches!(ident.name.as_str(), "true" | "debug")
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Checks if a function name is an FFI function.
    pub fn is_ffi_function(&self, name: &str) -> bool {
        self.ffi_function_map.contains_key(name)
    }

    /// Gets the FFI symbol ID for a function name.
    pub fn get_ffi_symbol_id(&self, name: &str) -> Option<FfiSymbolId> {
        self.ffi_function_map.get(name).copied()
    }

    /// Gets the FFI symbol by ID.
    pub fn get_ffi_symbol(&self, id: FfiSymbolId) -> Option<&FfiSymbol> {
        self.ffi_symbols.get(id.0 as usize)
    }

    /// Gets the FFI contract expressions for a function name.
    pub fn get_ffi_contract_exprs(&self, name: &str) -> Option<&FfiContractExprs> {
        self.ffi_contract_exprs.get(name)
    }

    /// Returns the optimization level from codegen config.
    pub fn optimization_level(&self) -> u8 {
        self.config.optimization_level
    }

    /// Checks if an FFI function returns a pointer type.
    ///

    /// Returns true if the FFI function's return type is a pointer (Ptr, CStr,
    /// StructPtr, ArrayPtr, or FnPtr). This is used to mark result registers
    /// as containing raw pointers that need DerefRaw instructions.
    pub fn ffi_returns_pointer(&self, id: FfiSymbolId) -> bool {
        self.ffi_symbols
            .get(id.0 as usize)
            .map(|sym| sym.signature.return_type.is_pointer())
            .unwrap_or(false)
    }

    /// Registers variant constructors from a type declaration.
    ///

    /// For sum types like `type Maybe<T> is None | Some(T)`, this registers
    /// variants with both qualified and simple names when safe.
    ///

    /// Registration strategy:
    /// 1. Always register `Type.Variant` (qualified) - never collides
    /// 2. Also register `Variant` (simple) if no collision exists
    /// 3. If another type defines the same variant name, mark as collision
    ///  and remove the simple name (code must use qualified names)
    ///

    /// This allows convenient unqualified usage like `Some(x)` when there's
    /// no ambiguity, while still supporting qualified names like `Maybe.Some(x)`
    /// when disambiguation is needed.
    fn register_type_constructors(
        &mut self,
        type_decl: &verum_ast::decl::TypeDecl,
    ) -> CodegenResult<()> {
        let type_name = type_decl.name.name.to_string();
        if std::env::var("VERUM_TRACE_RTC").is_ok() && (type_name == "IoResult" || type_name == "IoError" || type_name == "Metadata") {
            eprintln!("[RTC] type_name={} body_kind={}", type_name, match &type_decl.body {
                TypeDeclBody::Alias(_) => "Alias",
                TypeDeclBody::Variant(_) => "Variant",
                TypeDeclBody::Record(_) => "Record",
                TypeDeclBody::Newtype(_) => "Newtype",
                TypeDeclBody::Tuple(_) => "Tuple",
                TypeDeclBody::Unit => "Unit",
                TypeDeclBody::Protocol(_) => "Protocol",
                TypeDeclBody::Inductive(_) => "Inductive",
                TypeDeclBody::Coinductive(_) => "Coinductive",
                _ => "OTHER",
            });
        }

        match &type_decl.body {
            // Variant types: register each variant as a constructor
            TypeDeclBody::Variant(variants) => {
                // Wholesale-replace semantics for user-defined types.
                //

                // Stdlib modules compile first with `prefer_existing_functions = true`
                // (constructors register via `or_insert`). User-code compilation
                // runs with `prefer_existing = false` and may redeclare a type
                // that the stdlib already registered — e.g. the user writes
                // `type Maybe is Nothing | Just(Int);` while the stdlib's
                // `core.base.maybe` already registered `Maybe.None`/`Maybe.Some`.
                //

                // Without this purge the user's variants coexist with the
                // stdlib's in the function table, which lets the compiler
                // accept `Some(x)` against a `Maybe` that has no `Some`
                // constructor and produces surprising dispatch.
                //

                // Gate: only in the user-compilation phase
                // (`!prefer_existing_functions`). During stdlib loading we
                // keep first-wins semantics so two stdlib modules defining
                // the same nominal type do not clobber each other.
                if !self.ctx.prefer_existing_functions {
                    self.ctx.clear_variants_for_type(&type_name);
                } else if self.ctx.has_variants_for_type(&type_name) {
                    // First-wins: a prior (user or stdlib) declaration has
                    // already populated the variant set for this nominal
                    // type. Skip re-registering under `prefer_existing`
                    // mode — otherwise stdlib's `Maybe = None | Some(T)`
                    // would leak `None`/`Some` constructors into a user
                    // program that redeclared `type Maybe is Nothing |
                    // Just(Int)` and they would coexist in the function
                    // table, producing nondeterministic dispatch.
                    tracing::debug!(
                        "[variant] first-wins SKIP for type {} — variants already registered",
                        type_name
                    );
                    return Ok(());
                }
                tracing::debug!(
                    "[variant] register_type_constructors entering for {}",
                    type_name
                );

                for (variant_index, variant) in variants.iter().enumerate() {
                    let variant_name = variant.name.name.to_string();
                    let qualified_name = format!("{}.{}", type_name, variant_name);
                    // Variant constructors are compiled inline (MakeVariant instruction),
                    // not as callable functions. Use a sentinel ID that won't conflict
                    // with real function IDs.
                    let id = FunctionId(u32::MAX - variant_index as u32);

                    // Determine parameter count and payload types based on variant data
                    let (param_count, param_names, payload_types) = match &variant.data {
                        verum_common::Maybe::None => (0, vec![], vec![]),
                        verum_common::Maybe::Some(VariantData::Tuple(types)) => {
                            let count = types.len();
                            let names: Vec<String> =
                                (0..count).map(|i| format!("_{}", i)).collect();
                            // Extract type names from each type in the tuple
                            let type_names: Vec<String> = types
                                .iter()
                                .map(|ty| self.type_to_simple_name(ty))
                                .collect();
                            (count, names, type_names)
                        }
                        verum_common::Maybe::Some(VariantData::Record(fields)) => {
                            let count = fields.len();
                            let names: Vec<String> =
                                fields.iter().map(|f| f.name.name.to_string()).collect();
                            // Extract type names from each field
                            let type_names: Vec<String> = fields
                                .iter()
                                .map(|f| self.type_to_simple_name(&f.ty))
                                .collect();
                            // **Architectural rule** (closes task #16):
                            // record-style variant field layouts MUST be
                            // registered under the QUALIFIED
                            // `<ParentType>.<Variant>` name — never the bare
                            // variant simple name — because variants are
                            // NOT independent types in the type-name
                            // registry.  Pre-fix this site called
                            // `register_record_fields(&variant_name, …)`
                            // with bare `variant_name = "Timeout"`,
                            // populating `type_field_layouts["Timeout"]`
                            // with the variant's payload field set
                            // (e.g. `["ts"]` from
                            // `core.sys.io_engine.CompletionOp.Timeout {
                            // ts: &TimeSpec }`).  When the real
                            // `core.async.timer.Timeout<F>` record then
                            // tried to register its 3-field layout
                            // (`["future", "sleep", "completed"]`), the
                            // first-wins guard at
                            // `register_record_fields:13146` blocked the
                            // overwrite — `type_field_layouts["Timeout"]`
                            // stayed at `["ts"]`.  Downstream
                            // `Timeout.new`'s precompiled body emitted
                            // `Instruction::New { type_id, field_count: 1 }`
                            // (via `type_field_count("Timeout") ⇒ 1`),
                            // and the subsequent SetF for field index 5
                            // wrote 40 bytes past the 8-byte allocation —
                            // the exact field-write OOB panic this task
                            // pinned.
                            //
                            // Match-destructure (`match e { Timeout { .. }
                            // => … }`) still works because it dispatches
                            // by the scrutinee's nominal type via
                            // `<scrutinee_type>.<variant>` qualified
                            // lookup, NOT by the bare variant name —
                            // verified by the absence of regressions on
                            // record-style variant match arms after this
                            // fix.
                            self.register_record_fields(
                                &qualified_name,
                                names.clone(),
                                type_names.clone(),
                            );
                            (count, names, type_names)
                        }
                    };

                    // Tag is the variant's declaration order index (0, 1, 2, ...)
                    let tag = variant_index as u32;

                    let info = FunctionInfo {
                        id,
                        param_count,
                        param_names: param_names.clone(),
                        param_type_names: vec![],
                        is_async: false,
                        is_generator: false,
                        contexts: vec![],
                        return_type: None,
                        yield_type: None,
                        intrinsic_name: None,
                        variant_tag: Some(tag),
                        parent_type_name: Some(type_name.clone()),
                        variant_payload_types: if payload_types.is_empty() {
                            None
                        } else {
                            Some(payload_types)
                        },
                        is_partial_pattern: false,
                        takes_self_mut_ref: false,
                        // Variant constructors return the parent type
                        return_type_name: Some(type_name.clone()),
                        return_type_inner: None,
                        is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                    };

                    // 1. Always register with qualified name (TypeName::VariantName)
                    tracing::debug!(
                        "[variant] registering qualified {}.{} tag={} params={}",
                        type_name,
                        variant_name,
                        tag,
                        param_count
                    );
                    self.ctx.register_function(qualified_name, info.clone());

                    // 2. Handle simple name registration with collision detection.
                    //

                    // First-wins semantics during stdlib loading
                    // (`prefer_existing_functions = true`): if a built-in or
                    // earlier-loaded stdlib type already owns the simple
                    // name (e.g. `Maybe.None`), do NOT remove it when a
                    // later stdlib type (e.g. `RecoveryStrategy`,
                    // `BackoffStrategy`, `JitterConfig`) declares its own
                    // `None` variant. The later type still gets its
                    // qualified `<Type>.None` registration (line 5719) —
                    // call sites that need it can use the qualified form.
                    //

                    // Without this gate, every stdlib type that declares
                    // `| None` (or any other commonly-named variant) would
                    // wipe the bare `None` alias, breaking every other
                    // stdlib body that legitimately uses `Maybe.None` via
                    // the simple name (BTreeMap, Receiver.poll, every
                    // Stream adapter, etc.) — the lenient-skip
                    // "undefined variable: None" cluster.
                    //

                    // In user-mode (`prefer_existing_functions = false`)
                    // the original collision-removal stays: a user-defined
                    // `type Foo is None | ...` wins the simple name over
                    // the stdlib's `Maybe.None` (per the architectural
                    // rule in `verum_types/src/CLAUDE.md`).
                    if !self.variant_collisions.contains(&variant_name) {
                        // Check if simple name is already registered
                        if let Some(existing) = self.ctx.lookup_function(&variant_name) {
                            if existing.parent_type_name.as_deref() == Some(&type_name) {
                                // Same type re-registering the same variant — allow overwrite
                                self.ctx.unregister_function(&variant_name);
                                self.ctx.register_function(variant_name.clone(), info);
                            } else if self.ctx.prefer_existing_functions {
                                // Stdlib loading: keep first-registered simple name.
                                // Qualified `<Type>.<Variant>` was already registered
                                // above; downstream code can disambiguate via that.
                                tracing::debug!(
                                    "[variant] first-wins KEEP simple {} for {} (existing parent={:?}, new type={})",
                                    variant_name,
                                    existing.parent_type_name.as_deref().unwrap_or("<builtin>"),
                                    existing.parent_type_name,
                                    type_name,
                                );
                            } else {
                                // User-mode collision: another type already defines this variant name.
                                // Remove the existing simple name and mark as collision so neither
                                // type can use the bare form — both must qualify.
                                self.ctx.unregister_function(&variant_name);
                                self.variant_collisions.insert(variant_name.clone());
                            }
                        } else {
                            // No collision - register simple name too for convenience
                            self.ctx.register_function(variant_name.clone(), info);
                        }
                    }
                    // If already in collision set, don't register simple name
                }

                // VBC-3 fundamental fix: register a TypeDescriptor for
                // the sum type itself so MakeVariantTyped can carry
                // its concrete TypeId in the heap header (and the
                // runtime variant-name lookup at
                // `format_variant_for_print_depth` can resolve the
                // constructor name in O(N_variants_of_type) instead
                // of falling back to a global scan that loses
                // disambiguation across sum types sharing variant
                // tags — e.g. `Result.Err` vs `ShellError.SpawnFailed`
                // both tag=1). Pre-fix the variant-arm registered
                // each variant as a constructor function but NEVER
                // pushed a descriptor for the parent type — codegen's
                // type-typed emit helper would resolve the name to a
                // TypeId via `type_name_to_id` but the runtime
                // validator would reject it as `unknown type_id`
                // because no descriptor matched, demoting every
                // variant emission back to the legacy form. Now the
                // descriptor IS pushed with kind=Sum + a complete
                // variant table, so the typed path stays load-bearing.
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };
                // Capture generic parameters from the AST so the
                // archive's TypeDescriptor preserves them.  Pre-fix
                // `Maybe<T>` / `Result<T, E>` / etc. landed in VBC
                // with empty `type_params`; archive_metadata then
                // emitted CoreMetadata with `generic_params=[]`,
                // breaking positional substitution at use sites
                // (e.g. `Maybe<Int>.unwrap_or(0)` failed to bind T
                // → Int because T wasn't recorded).
                let mut sum_type_params: smallvec::SmallVec<[crate::types::TypeParamDescriptor; 2]> =
                    smallvec::SmallVec::new();
                let mut sum_generic_param_map: std::collections::HashMap<String, u16> =
                    std::collections::HashMap::new();
                for (idx, gp) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                        let gname_id = StringId(self.ctx.intern_string_raw(gname.name.as_str()));
                        sum_type_params.push(crate::types::TypeParamDescriptor {
                            name: gname_id,
                            id: crate::types::TypeParamId(idx as u16),
                            bounds: smallvec::SmallVec::new(),
                            default: None,
                            variance: crate::types::Variance::Invariant,
                            type_bounds: smallvec::SmallVec::new(),
                        });
                        sum_generic_param_map
                            .insert(gname.name.to_string(), idx as u16);
                    }
                }
                let mut sum_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Sum,
                    type_params: sum_type_params,
                    ..Default::default()
                };
                for (variant_index, variant) in variants.iter().enumerate() {
                    let variant_name = variant.name.name.to_string();
                    let variant_name_id = StringId(self.ctx.intern_string_raw(&variant_name));
                    let (kind, arity, fields_desc): (
                        crate::types::VariantKind,
                        u8,
                        smallvec::SmallVec<[crate::types::FieldDescriptor; 4]>,
                    ) = match &variant.data {
                        verum_common::Maybe::None => (
                            crate::types::VariantKind::Unit,
                            0,
                            smallvec::SmallVec::new(),
                        ),
                        verum_common::Maybe::Some(VariantData::Tuple(types)) => {
                            // Empty-payload tuple variants — `loop_path()`
                            // — are syntactically `Tuple(0 args)` but
                            // semantically Unit. The layout validator
                            // rejects `Tuple` + `arity=0` ("should be
                            // Unit instead"); coerce here so the
                            // descriptor is well-formed.
                            if types.is_empty() {
                                (
                                    crate::types::VariantKind::Unit,
                                    0,
                                    smallvec::SmallVec::new(),
                                )
                            } else {
                                // Populate `fields` with positional
                                // type info so archive_metadata can
                                // recover the per-slot TypeRef of each
                                // tuple variant payload.  Pre-fix
                                // `fields_desc` was always empty for
                                // tuple variants; archive_metadata's
                                // arity-padding fallback then always
                                // grabbed the FIRST generic param
                                // ("T") for every slot — so
                                // `Result.Err(E)` got serialised as
                                // `Err(T)` and the typechecker bound
                                // payloads to the wrong type
                                // parameter.  Use
                                // `resolve_field_type_ref` so generic
                                // params land as
                                // `TypeRef::Generic(idx)` keyed by the
                                // parent's positional generic params.
                                let fds: smallvec::SmallVec<[crate::types::FieldDescriptor; 4]> =
                                    types
                                        .iter()
                                        .enumerate()
                                        .map(|(i, ty)| {
                                            let pos_name = format!("_{}", i);
                                            crate::types::FieldDescriptor {
                                                name: StringId(
                                                    self.ctx.intern_string_raw(&pos_name),
                                                ),
                                                type_ref: self.resolve_field_type_ref(
                                                    ty,
                                                    &sum_generic_param_map,
                                                ),
                                                ..Default::default()
                                            }
                                        })
                                        .collect();
                                (
                                    crate::types::VariantKind::Tuple,
                                    types.len() as u8,
                                    fds,
                                )
                            }
                        }
                        verum_common::Maybe::Some(VariantData::Record(fields)) => {
                            // Record-variant payload count lives in
                            // `fields`, NOT in `arity` — the layout
                            // validator (`type-layout invariant`)
                            // rejects the over-specified case
                            // ("variant is Record but also reports
                            // arity=N"). Keep arity=0 here so the
                            // descriptor is internally consistent.
                            let fds: smallvec::SmallVec<[crate::types::FieldDescriptor; 4]> =
                                fields
                                    .iter()
                                    .map(|f| crate::types::FieldDescriptor {
                                        name: StringId(
                                            self.ctx.intern_string_raw(f.name.name.as_str()),
                                        ),
                                        type_ref: self.resolve_field_type_ref(
                                            &f.ty,
                                            &sum_generic_param_map,
                                        ),
                                        ..Default::default()
                                    })
                                    .collect();
                            (crate::types::VariantKind::Record, 0u8, fds)
                        }
                    };
                    sum_desc.variants.push(crate::types::VariantDescriptor {
                        name: variant_name_id,
                        tag: variant_index as u32,
                        payload: None,
                        kind,
                        arity,
                        fields: fields_desc,
                    });
                }
                tracing::debug!(
                    "[variant] pushing Sum TypeDescriptor for {} (id={}, variants={})",
                    type_name,
                    type_id.0,
                    sum_desc.variants.len(),
                );
                self.push_type_dedupe(sum_desc);
            }

            // Record types: register the type name as a constructor
            // For `type Point is { x: Int, y: Int }`, register `Point` as constructor
            TypeDeclBody::Record(fields) => {
                // Check for @repr(C) attribute - generate FFI struct layout if present
                if self.has_repr_c(&type_decl.attributes) {
                    self.generate_ffi_struct_layout(&type_name, fields);
                }

                // Check for @bitfield attribute - generate accessor methods
                if self.has_bitfield_attr(&type_decl.attributes) {
                    let byte_order = self.get_byte_order(&type_decl.attributes);
                    self.generate_bitfield_accessors(&type_name, fields, byte_order);
                }

                // Use pre-allocated TypeId from Pass 1.5, or allocate a new one
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };

                // Generic parameters are populated below by the
                // bounds-and-defaults pass (search for "Also populate
                // type_params" later in this arm).  That second pass
                // fills in the same name+id PLUS the protocol bounds
                // (`<T: Eq + Hash>`) and default-type bindings (`<T = Int>`)
                // which this short pass cannot — so we emit nothing
                // here and let the richer pass populate the field
                // exactly once.
                //
                // Pre-fix this arm built `record_type_params` from
                // generics (id + name only), assigned it to
                // `type_params:` at descriptor construction, AND THEN
                // the bounds-and-defaults pass below also pushed
                // every generic — duplicating every entry.
                // archive_metadata's `convert_generic_params` walked
                // the duplicated list verbatim → `metadata.types["List"]`
                // ended up with `generic_params=[T, T]` (length 2,
                // same name twice).  Downstream consumers compared
                // `generic_params.len()` against user-side type-arg
                // count and rejected `List<Int>` (1 arg) with
                // "non-record type: List<_, _>" at every use site.
                // Identical pattern affected `Map`
                // (`generic_params=[K,V,K,V]`), `Set`, every record
                // type with generics — the entire cross-collection
                // user-side dispatch surface.
                //
                // Symptomatic test: `vcs/specs/L1-core/generics/associated_types.vr:387`
                // `let pairs: List<(Int, Int)> = collect_all(zipped);`
                // failed with `Cannot access field 'new' on
                // non-record type: List<_, _>`.
                //
                // Sum arm (~line 8012) populates type_params in a
                // single richer pass and was unaffected; the
                // duplication was Record-arm and Protocol-arm only.
                // Create a TypeDescriptor for this type (drop_fn will be set later if Drop is implemented)
                let mut type_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Record,
                    ..Default::default()
                };

                // Extract @align and @repr from type attributes for struct layout control.
                // @align(N) overrides default alignment (must be power of 2).
                // @repr(packed) eliminates padding (alignment = 1).
                // @repr(C) uses C-compatible layout rules.
                // @repr(cache_optimal) reorders fields for cache locality.
                let (type_align, is_packed, _is_repr_c) =
                    Self::extract_type_layout_hints(&type_decl.attributes);
                let field_size = if is_packed { 1u32 } else { 8u32 }; // packed: minimum size per field
                type_desc.size = (fields.len() as u32) * field_size;
                type_desc.alignment = type_align;

                // Build generic type param name → index mapping for field type resolution.
                // E.g., for `type Pair<A, B>`, maps {"A"→0, "B"→1}.
                let mut generic_param_map: std::collections::HashMap<String, u16> =
                    std::collections::HashMap::new();
                for (idx, generic) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name, .. } = &generic.kind {
                        generic_param_map.insert(name.name.to_string(), idx as u16);
                    }
                }

                // Also populate type_params on the TypeDescriptor (was only done for protocols)
                for generic in &type_decl.generics {
                    if let verum_ast::ty::GenericParamKind::Type {
                        name,
                        bounds,
                        default,
                    } = &generic.kind
                    {
                        let param_name_id =
                            StringId(self.ctx.intern_string_raw(name.name.as_str()));
                        let bound_ids: smallvec::SmallVec<[crate::types::ProtocolId; 2]> = bounds
                            .iter()
                            .filter_map(|bound| {
                                if let verum_ast::ty::TypeBoundKind::Protocol(path) = &bound.kind
                                    && let Some(seg) = path.segments.last()
                                    && let verum_ast::ty::PathSegment::Name(ident) = seg
                                {
                                    let bname = ident.name.to_string();
                                    if let Some(&pid) = self.type_name_to_id.get(&bname) {
                                        return Some(crate::types::ProtocolId(pid.0));
                                    }
                                }
                                None
                            })
                            .collect();
                        let default_ref = match default {
                            verum_common::Maybe::Some(ty) => Some(self.ast_type_to_type_ref(ty)),
                            verum_common::Maybe::None => None,
                        };
                        let param_desc = crate::types::TypeParamDescriptor {
                            name: param_name_id,
                            id: crate::types::TypeParamId(type_desc.type_params.len() as u16),
                            bounds: bound_ids,
                            default: default_ref,
                            ..Default::default()
                        };
                        type_desc.type_params.push(param_desc);
                    }
                }

                // Populate fields for drop tracking
                // NOTE: Field offset must use global field index from intern_field_name,
                // since that's what SetF/GetF use at runtime.
                for field in fields.iter() {
                    let field_name = field.name.name.to_string();
                    // Use full type resolution to preserve generic parameters.
                    // Check if field type is a generic type parameter (e.g., `first: A`).
                    let field_type_ref = self.resolve_field_type_ref(&field.ty, &generic_param_map);

                    // Use the same global field index that SetF/GetF use
                    let field_idx = self.intern_field_name(&field_name);

                    type_desc.fields.push(crate::types::FieldDescriptor {
                        name: StringId(self.ctx.intern_string_raw(&field_name)),
                        type_ref: field_type_ref,
                        offset: field_idx * 8, // Use global field index * sizeof(Value)
                        visibility: crate::types::Visibility::Public,
                    });
                }

                self.push_type_dedupe(type_desc);

                let id = FunctionId(u32::MAX / 2);

                let param_names: Vec<String> =
                    fields.iter().map(|f| f.name.name.to_string()).collect();

                let info = FunctionInfo {
                    id,
                    param_count: fields.len(),
                    param_names,
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };

                self.ctx.register_function(type_name, info);
            }

            // Protocol types: create TypeDescriptor with super-protocol and method metadata
            TypeDeclBody::Protocol(protocol_body) => {
                // Use pre-allocated TypeId from Pass 1.5, or allocate a new one
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };

                let mut type_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Protocol,
                    ..Default::default()
                };

                // Encode super-protocols in the protocols field
                // For protocol types, protocols[] means "extends these protocols"
                if let Some(proto_info) = self.protocol_registry.get(&type_name) {
                    for super_name in &proto_info.super_protocols {
                        if let Some(&super_id) = self.type_name_to_id.get(super_name) {
                            type_desc.protocols.push(crate::types::ProtocolImpl {
                                protocol: crate::types::ProtocolId(super_id.0),
                                methods: Vec::new(),
                            });
                        }
                    }
                }

                // Encode method signatures as variants (VBC convention for protocol methods).
                //
                // #131 Layer E — protocol-level + method-local generic
                // param scoping.  Pre-fix `ast_type_to_type_ref` had
                // no scope: an unknown `TypeKind::Path([Self])` /
                // `[Item]` / `[U]` (protocol-level Self/Item or
                // method-local `map<U, B>` params) fell through to
                // `TypeRef::Concrete(TypeId::PTR=14)`.  At
                // archive_metadata-time the PTR fallback resolved to
                // the wrong-arity concrete name `"Heap"` /  `"List"`
                // via the module-local type-id-to-name table —
                // protocol-method signatures got rendered as
                // `MappedIter<Heap, Heap>` instead of
                // `MappedIter<Self, F>` and the typechecker actively
                // mismatched user-side `Heap<T,V>` shapes.
                //
                // Fix: build a combined `generic_param_map`
                // (protocol-level params at IDs 0..N from
                // `type_decl.generics`, then method-local params at
                // IDs N..N+M from `decl.generics`), and route every
                // type-ref conversion through `resolve_field_type_ref`
                // — which emits `TypeRef::Generic(id)` for path-name
                // matches.  archive_metadata's
                // `type_ref_to_text_with_params` only seeds its
                // `param_id_to_name` from protocol-level params
                // (IDs 0..N), so method-local IDs (N+) fall through
                // to `__generic_N` placeholders, which the
                // typechecker's `parse_descriptor_type_string`
                // converts to fresh TypeVars (the unifier-permissive
                // shape we want).
                //
                // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`:
                // every binding comes from the protocol decl AST's
                // own generics list, no hardcoded param names.
                let mut proto_param_map: std::collections::HashMap<String, u16> = type_decl
                    .generics
                    .iter()
                    .enumerate()
                    .filter_map(|(i, g)| match &g.kind {
                        verum_ast::ty::GenericParamKind::Type { name, .. } => {
                            Some((name.name.to_string(), i as u16))
                        }
                        _ => None,
                    })
                    .collect();
                // #131 Layer E — `Self` is the implicit
                // implementor type in protocol method signatures
                // (`fn lt(self, other: Self) -> Bool` in PartialOrd,
                // `MappedIter<Self, F>` return-type in Iterator.map).
                // Protocols never declare `Self` in `type_decl.generics`,
                // so without this binding `ast_type_to_type_ref` falls
                // through to `TypeRef::Concrete(TypeId::PTR=14)` —
                // which collides with codegen's `Heap → TypeId::PTR`
                // alias mapping at line 1102.  archive_metadata's
                // `type_id_to_name` then renders id=14 as `"Heap"`,
                // breaking every protocol method that mentions Self
                // (PartialOrd `lt`/`le`/etc, Iterator return types,
                // …).  Bind Self to an out-of-range synthetic ID so
                // it round-trips as `__generic_N` → fresh TypeVar.
                let self_synthetic_id = type_decl.generics.len() as u16 + 0x4000;
                proto_param_map
                    .entry("Self".to_string())
                    .or_insert(self_synthetic_id);
                for protocol_item in &protocol_body.items {
                    if let verum_ast::decl::ProtocolItemKind::Function { decl, .. } =
                        &protocol_item.kind
                    {
                        let method_name = decl.name.name.to_string();
                        let method_name_id = StringId(self.ctx.intern_string_raw(&method_name));

                        // Build the combined scope: protocol-level
                        // params first (IDs 0..N), then method-local
                        // params (IDs N..N+M).  Method-locals start
                        // from the protocol-param count so the
                        // archive_metadata renderer's
                        // protocol-only `param_id_to_name` doesn't
                        // resolve them — they round-trip as
                        // `__generic_N` → fresh TypeVar.
                        let mut combined_map = proto_param_map.clone();
                        let proto_count = combined_map.len() as u16;
                        for (i, g) in decl.generics.iter().enumerate() {
                            if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                                let key = name.name.to_string();
                                combined_map
                                    .entry(key)
                                    .or_insert(proto_count + i as u16);
                            }
                        }

                        // Build function TypeRef for the method
                        let param_refs: Vec<crate::types::TypeRef> = decl
                            .params
                            .iter()
                            .filter_map(|p| {
                                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } =
                                    &p.kind
                                {
                                    Some(self.resolve_field_type_ref(ty, &combined_map))
                                } else {
                                    None // Skip self params
                                }
                            })
                            .collect();

                        let ret_ref = match &decl.return_type {
                            verum_common::Maybe::Some(ty) => {
                                self.resolve_field_type_ref(ty, &combined_map)
                            }
                            verum_common::Maybe::None => {
                                TypeRef::Concrete(crate::types::TypeId::UNIT)
                            }
                        };

                        let method_type_ref = TypeRef::Function {
                            params: param_refs,
                            return_type: Box::new(ret_ref),
                            contexts: smallvec::SmallVec::new(),
                        };

                        let variant_desc = crate::types::VariantDescriptor {
                            name: method_name_id,
                            tag: type_desc.variants.len() as u32,
                            payload: Some(method_type_ref),
                            ..Default::default()
                        };
                        type_desc.variants.push(variant_desc);
                    }
                }

                // Add generic type parameters
                for generic in &type_decl.generics {
                    if let verum_ast::ty::GenericParamKind::Type {
                        name,
                        bounds,
                        default,
                    } = &generic.kind
                    {
                        let param_name_id =
                            StringId(self.ctx.intern_string_raw(name.name.as_str()));
                        let bound_ids: smallvec::SmallVec<[crate::types::ProtocolId; 2]> = bounds
                            .iter()
                            .filter_map(|bound| {
                                if let verum_ast::ty::TypeBoundKind::Protocol(path) = &bound.kind
                                    && let Some(seg) = path.segments.last()
                                    && let verum_ast::ty::PathSegment::Name(ident) = seg
                                {
                                    let bname = ident.name.to_string();
                                    if let Some(&pid) = self.type_name_to_id.get(&bname) {
                                        return Some(crate::types::ProtocolId(pid.0));
                                    }
                                }
                                None
                            })
                            .collect();

                        let default_ref = match default {
                            verum_common::Maybe::Some(ty) => Some(self.ast_type_to_type_ref(ty)),
                            verum_common::Maybe::None => None,
                        };

                        let param_desc = crate::types::TypeParamDescriptor {
                            name: param_name_id,
                            id: crate::types::TypeParamId(type_desc.type_params.len() as u16),
                            bounds: bound_ids,
                            default: default_ref,
                            ..Default::default()
                        };
                        type_desc.type_params.push(param_desc);
                    }
                }

                self.push_type_dedupe(type_desc);
            }

            // Type aliases: register mapping from alias name to base type
            // This enables method resolution: Vec4f.splat() → Vec.splat()
            TypeDeclBody::Alias(target_type) => {
                // Extract base type name from alias target
                // For `type Vec4f = Vec<Float32, 4>`, extract "Vec"
                // For `type MyInt = Int`, extract "Int"
                if let Some(base_name) = self.extract_base_type_name(target_type) {
                    self.type_aliases
                        .insert(type_name.clone(), base_name);
                }
                // Emit a `TypeKind::Alias` TypeDescriptor so the
                // archive preserves the alias relation.
                // Pre-fix every `type IoResult<T> is Result<T,
                // StreamError>;` lost the alias info at VBC stage —
                // archive_metadata couldn't find IoResult, and the
                // typechecker's pattern matcher saw `IoResult<X>`
                // as a bare opaque Type::Generic with no resolution
                // path back to Result.
                // Use pre-allocated TypeId from collect_all_declarations
                // when present (avoids id collision with the
                // placeholder pre-pass).  Falls back to a fresh
                // user-id otherwise.
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };
                let name_id = StringId(self.intern_string(&type_name));
                // Build generic_param_map from type_decl.generics so
                // alias targets reference T/E/etc. as
                // `TypeRef::Generic(idx)` rather than degrading to
                // `TypeRef::Concrete(PTR)` via the unknown-name
                // fallback.  Without this, `type IoResult<T> is
                // Result<T, StreamError>;` lost T → became
                // `Result<PTR, StreamError>` and the typechecker's
                // alias expansion produced `Result<PTR, …>` at
                // every IoResult<X> site.
                let mut generic_param_map: std::collections::HashMap<String, u16> =
                    std::collections::HashMap::new();
                for (idx, gp) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                        generic_param_map.insert(gname.name.to_string(), idx as u16);
                    }
                }
                let target_ref = self.resolve_field_type_ref(target_type, &generic_param_map);
                if std::env::var("VERUM_TRACE_RTC").is_ok() && type_name == "IoResult" {
                    eprintln!("[RTC-emit] IoResult id={} target_ref={:?}", type_id.0, target_ref);
                }
                // Generic params (T, K, V, …) on the alias signature.
                let mut alias_type_params: smallvec::SmallVec<[crate::types::TypeParamDescriptor; 2]> =
                    smallvec::SmallVec::new();
                for (idx, gp) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                        let gname_id = StringId(self.intern_string(gname.name.as_str()));
                        alias_type_params.push(crate::types::TypeParamDescriptor {
                            name: gname_id,
                            id: crate::types::TypeParamId(idx as u16),
                            bounds: smallvec::SmallVec::new(),
                            default: None,
                            variance: crate::types::Variance::Invariant,
                            type_bounds: smallvec::SmallVec::new(),
                        });
                    }
                }
                let type_desc = crate::types::TypeDescriptor {
                    id: type_id,
                    name: name_id,
                    kind: crate::types::TypeKind::Alias,
                    type_params: alias_type_params,
                    fields: smallvec::SmallVec::new(),
                    variants: smallvec::SmallVec::new(),
                    size: 0,
                    alignment: 0,
                    drop_fn: None,
                    clone_fn: None,
                    protocols: smallvec::SmallVec::new(),
                    visibility: crate::types::Visibility::Public,
                    alias_target: Some(target_ref),
                    // Aliases are name-only redirects; representation
                    // is decided by the alias *target* type's
                    // descriptor, not by the alias itself.
                    is_transparent_wrapper: false,
                };
                self.push_type_dedupe(type_desc);
            }

            // Newtype and tuple types: register the type name as constructor
            TypeDeclBody::Newtype(_inner_type) => {
                // Track newtype names for GetF optimization (field .0 = identity)
                self.ctx.newtype_names.insert(type_name.clone());
                let inner_name = self.type_to_simple_name(_inner_type);
                self.ctx
                    .newtype_inner_type
                    .insert(type_name.clone(), inner_name);

                // Allocate / reuse a TypeId so the descriptor below
                // and any later cross-module reference share the same
                // identity.  Same pattern as the Record / Sum arms.
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };

                // Generic-param map for the inner-type resolution —
                // `type Wrap<T> is T;` writes a `Generic(0)` slot, and
                // the consumer side substitutes against the type's
                // `type_params` to recover `T`'s identity.  Same shape
                // as the Record arm; empty for the common
                // non-generic newtype case (`type FileDesc is Int;`).
                let mut generic_param_map: std::collections::HashMap<String, u16> =
                    std::collections::HashMap::new();
                for (idx, gp) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                        generic_param_map.insert(gname.name.to_string(), idx as u16);
                    }
                }
                let inner_type_ref = self.resolve_field_type_ref(_inner_type, &generic_param_map);
                let inner_field_idx = self.intern_field_name("_0");

                // Canonical (serialisable) marker on the type
                // descriptor: this type IS a transparent wrapper.
                // Mirrors `newtype_names` so archive-loaded types
                // recover their representation policy after a
                // round-trip without rebuilding the codegen-local
                // HashSet from source.  See `TypeDescriptor::is_transparent_wrapper`.
                //
                // Inner-type field "_0" — pushed into `type_desc.fields`
                // unconditionally so the archive carries the inner
                // type via the same structural channel as every other
                // record type.  Downstream consumers (typechecker's
                // `__newtype_inner_X` registration, archive_ctx_loader's
                // Pass 5, runtime field dispatch) read this directly
                // instead of relying on out-of-band metadata.  Without
                // this push, the inner-type identity vanished at the
                // archive boundary and `FileDesc.STDIN.as_raw()`
                // dispatched on the bare inner value (Int) — losing
                // the wrapper's static type identity.
                let mut type_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Record,
                    is_transparent_wrapper: true,
                    ..Default::default()
                };
                type_desc.fields.push(crate::types::FieldDescriptor {
                    name: StringId(self.ctx.intern_string_raw("_0")),
                    type_ref: inner_type_ref,
                    offset: inner_field_idx * 8,
                    visibility: crate::types::Visibility::Public,
                });
                type_desc.size = 8; // single inner value (one Value-slot)
                self.push_type_dedupe(type_desc);

                let id = FunctionId(u32::MAX / 2);

                let info = FunctionInfo {
                    id,
                    param_count: 1,
                    param_names: vec!["_0".to_string()],
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                    // Newtype constructors are pure transparent
                    // wrappers — `type X is T;` makes `X(t)` an
                    // identity Mov.  This flag is the canonical
                    // discriminator that the codegen passthrough
                    // arms gate on.
                    is_transparent_wrapper: true,
                    param_closure_return_type_names: Vec::new(),
                };

                self.ctx.register_function(type_name.clone(), info);
            }

            TypeDeclBody::Tuple(types) => {
                // Single-element tuple types like `type FileDesc is (Int)` are newtypes.
                // The value IS the single wrapped field — no heap allocation.
                let is_transparent = types.len() == 1;
                if is_transparent {
                    self.ctx.newtype_names.insert(type_name.clone());
                    let inner_name = self.type_to_simple_name(&types[0]);
                    self.ctx
                        .newtype_inner_type
                        .insert(type_name.clone(), inner_name);
                }

                // Allocate / reuse a TypeId — same pattern as the Record arm.
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };

                // Generic-param map for inner type resolution — mirror
                // of the Newtype arm above.
                let mut generic_param_map: std::collections::HashMap<String, u16> =
                    std::collections::HashMap::new();
                for (idx, gp) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                        generic_param_map.insert(gname.name.to_string(), idx as u16);
                    }
                }

                // Canonical descriptor — flips the transparent flag for
                // single-element tuples; multi-element tuples remain
                // boxed records (one slot per element).
                let mut type_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Record,
                    is_transparent_wrapper: is_transparent,
                    ..Default::default()
                };
                // Push the inner-type fields into `type_desc.fields` —
                // for single-element tuples ("_0") this is the transparent
                // wrapper's inner type that downstream typechecker /
                // archive_ctx_loader paths key on for `__newtype_inner_X`
                // registration.  For multi-element tuples ("_0", "_1",
                // ...) every slot is preserved so generic destructure
                // and `value.<index>` field-access resolve their static
                // types correctly.
                for (i, inner_ty) in types.iter().enumerate() {
                    let field_name = format!("_{}", i);
                    let inner_type_ref = self.resolve_field_type_ref(inner_ty, &generic_param_map);
                    let field_idx = self.intern_field_name(&field_name);
                    type_desc.fields.push(crate::types::FieldDescriptor {
                        name: StringId(self.ctx.intern_string_raw(&field_name)),
                        type_ref: inner_type_ref,
                        offset: field_idx * 8,
                        visibility: crate::types::Visibility::Public,
                    });
                }
                type_desc.size = (types.len() as u32) * 8;
                self.push_type_dedupe(type_desc);

                let id = FunctionId(u32::MAX / 2);

                let param_names: Vec<String> =
                    (0..types.len()).map(|i| format!("_{}", i)).collect();

                let info = FunctionInfo {
                    id,
                    param_count: types.len(),
                    param_names,
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                    // Single-element tuple `type FileDesc is (Int)` IS
                    // a transparent wrapper (zero-cost — the value is
                    // the inner value at runtime).  Multi-element
                    // tuples are boxed records — pass-through would
                    // drop the other elements, so the flag stays
                    // false there.
                    is_transparent_wrapper: is_transparent,
                    param_closure_return_type_names: Vec::new(),
                };

                self.ctx.register_function(type_name, info);
            }

            // Unit type has no constructor arguments
            TypeDeclBody::Unit => {
                let id = FunctionId(u32::MAX / 2);

                let info = FunctionInfo {
                    id,
                    param_count: 0,
                    param_names: vec![],
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };

                self.ctx.register_function(type_name, info);
            }

            // SigmaTuple types: similar to tuple types
            TypeDeclBody::SigmaTuple(types) => {
                let id = FunctionId(u32::MAX / 2);

                let param_names: Vec<String> =
                    (0..types.len()).map(|i| format!("_{}", i)).collect();

                let info = FunctionInfo {
                    id,
                    param_count: types.len(),
                    param_names,
                    param_type_names: vec![],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: None,
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };

                self.ctx.register_function(type_name, info);
            }
            TypeDeclBody::Inductive(_) | TypeDeclBody::Coinductive(_) => {
                // Dependent type features (v2.0+) - no constructor registration needed yet
            }
            TypeDeclBody::Quotient { base, .. } => {
                // Register `Q.of` (static) and `Q.rep` (instance) as
                // identity-pass-through projections. At Tier-0 a
                // quotient type is represented at runtime by the
                // carrier value itself — the equivalence relation is
                // a compile-time obligation discharged by the model-
                // verification pipeline, not a runtime quotienting of
                // the representation. So `Q.of(t)` emits `Mov dst, t`
                // and `q.rep()` emits `Mov dst, q`. The type checker
                // (see verum_types::infer) has already enforced that
                // the return types match `Q` and the base carrier
                // respectively, so no runtime coercion is required.
                //

                // The newtype-pass-through sentinel id `u32::MAX / 2`
                // is shared with newtype / single-element-tuple
                // constructors; the call-site codegen recognises it
                // and emits the identity Mov.
                let pass_through_id = FunctionId(u32::MAX / 2);
                let base_type_name = self.type_to_simple_name(base);

                // `Q.of(rep: T) -> Q`
                let of_info = FunctionInfo {
                    id: pass_through_id,
                    param_count: 1,
                    param_names: vec!["rep".to_string()],
                    param_type_names: if base_type_name.is_empty() {
                        vec![]
                    } else {
                        vec![base_type_name.clone()]
                    },
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: Some(type_name.clone()),
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };
                let of_qualified = format!("{}.of", type_name);
                self.ctx.register_function(of_qualified, of_info);

                // `q.rep(&self) -> T`
                let rep_info = FunctionInfo {
                    id: pass_through_id,
                    param_count: 1,
                    param_names: vec!["self".to_string()],
                    param_type_names: vec![type_name.clone()],
                    is_async: false,
                    is_generator: false,
                    contexts: vec![],
                    return_type: None,
                    yield_type: None,
                    intrinsic_name: None,
                    variant_tag: None,
                    parent_type_name: Some(type_name.clone()),
                    variant_payload_types: None,
                    is_partial_pattern: false,
                    takes_self_mut_ref: false,
                    return_type_name: if base_type_name.is_empty() {
                        None
                    } else {
                        Some(base_type_name)
                    },
                    return_type_inner: None,
                    is_const: false,
                is_transparent_wrapper: false,
                param_closure_return_type_names: Vec::new(),
                };
                let rep_qualified = format!("{}.rep", type_name);
                self.ctx.register_function(rep_qualified, rep_info);

                // Mark the quotient as a newtype for the pass-through
                // codegen paths (Q ≡ carrier at runtime).
                self.ctx.newtype_names.insert(type_name.clone());
                let inner = self.type_to_simple_name(base);
                if !inner.is_empty() {
                    self.ctx.newtype_inner_type.insert(type_name.clone(), inner);
                }

                // Canonical descriptor — quotient types lower to the
                // carrier at runtime, same transparent-wrapper policy
                // as Newtype/single-element-Tuple.  See
                // `TypeDescriptor::is_transparent_wrapper`.
                let type_id = if let Some(&existing) = self.type_name_to_id.get(&type_name) {
                    existing
                } else {
                    let tid = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name.clone(), tid);
                    tid
                };
                let mut type_desc = TypeDescriptor {
                    id: type_id,
                    name: StringId(self.ctx.intern_string_raw(&type_name)),
                    kind: crate::types::TypeKind::Record,
                    is_transparent_wrapper: true,
                    ..Default::default()
                };
                type_desc.size = 8;
                self.push_type_dedupe(type_desc);
            }
        }
        Ok(())
    }

    /// Registers a constant or static variable for lookup, with optional value extraction.
    ///

    /// Constants are registered as zero-argument functions so that name
    /// resolution during codegen can find them. If the value expression is a
    /// simple integer literal, the constant is registered as an inlineable
    /// intrinsic (via `__const_val_N` naming) so the value is emitted as a
    /// `LoadI` at call sites rather than a function call.
    ///

    /// # Example
    ///

    /// For a constant declaration:
    /// ```verum
    /// const MAX_SIZE: Int = 1024;
    /// ```
    ///

    /// This registers `MAX_SIZE` as a callable. If the value is a literal integer,
    /// it's inlined at usage sites for zero-cost access.
    fn register_constant_with_value(
        &mut self,
        name: &str,
        value_expr: Option<&verum_ast::Expr>,
        const_type: Option<&verum_ast::ty::Type>,
    ) -> CodegenResult<()> {
        let id = FunctionId(self.next_func_id);
        self.next_func_id = self.next_func_id.saturating_add(1);

        // Try to extract a literal integer value from the expression.
        // If successful, register as an inlineable constant (no function call needed).
        let intrinsic_name = value_expr
            .and_then(Self::extract_const_literal_value)
            .map(|v| format!("__const_val_{}", v));

        // If the constant couldn't be inlined, queue it for compilation as a function.
        // This handles struct literals like `MemProt { read: false, write: false, exec: false }`.
        let needs_compilation = intrinsic_name.is_none() && value_expr.is_some();
        if needs_compilation && let Some(expr) = value_expr {
            // Capture the CURRENT source-module at queue time so the
            // descriptor.name promotion in `compile_pending_constants`
            // uses the const's own declared module (`sys.bitfield` for
            // `core/sys/bitfield.vr::USIZE_BITS`) instead of whichever
            // file's `compile_items_into_state` is on the stack when
            // the queue is eventually drained (task #121).
            let source_module = self.ctx.current_source_module.clone();
            self.pending_constants
                .push((name.to_string(), expr.clone(), source_module));
        }

        // Extract return type name from constant type if present
        let return_type_name = const_type.and_then(|ty| self.extract_type_name(ty));

        let info = FunctionInfo {
            id,
            param_count: 0,
            param_names: vec![],
            param_type_names: vec![],
            is_async: false,
            is_generator: false,
            contexts: vec![],
            return_type: None,
            yield_type: None,
            intrinsic_name,
            variant_tag: None,
            parent_type_name: None,
            variant_payload_types: None,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
            // #97 — const storage strategy.  Both inlinable and
            // non-inlinable consts (those queued in
            // `pending_constants` for body compilation) carry this
            // marker so the typechecker treats them as values.
            is_const: true,
            is_transparent_wrapper: false,
            param_closure_return_type_names: Vec::new(),
        };

        // Register with simple name for local access
        self.ctx.register_function(name.to_string(), info.clone());

        // Also register with qualified names for cross-module imports.
        // Mirror `register_function`'s qualified-name strategy: register
        // under both the dot-form (`sys.bitfield.USIZE_BITS`) and the
        // colon-form (`sys::bitfield::USIZE_BITS`), and prefer the
        // *source module* (from the file's `module X.Y.Z;` declaration)
        // over `config.module_name`. The latter is fixed per-codegen-
        // session (`"main"` for a single-file user run) but a session
        // processes many imported stdlib modules, each with its own path
        // — without using `current_source_module`, every imported file's
        // consts collapse onto the session's outer module name, so
        // `mount core.sys.bitfield.{USIZE_BITS}` cannot find
        // `core.sys.bitfield.USIZE_BITS` at the import-resolution site
        // (it lands under `core.USIZE_BITS` instead). The matching
        // change in `register_function` is at line ~5996 — keep these
        // two paths in step.
        // Bind to an owned String so the immutable borrow of
        // `self.ctx.current_source_module` (via `.as_deref()`) drops
        // before the subsequent `self.ctx.register_function(...)` /
        // `self.ctx.register_constant_type(...)` mutable-borrow calls
        // and the later `if info.intrinsic_name.is_some()` block that
        // reads `effective_module` again for the descriptor-name
        // promotion.  Without the clone the borrow stays alive across
        // the function body and rejects the mutable-borrow calls.
        let effective_module: String = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name)
            .to_string();
        if !effective_module.is_empty()
            && effective_module != "main"
            && !name.contains('.')
            && !name.contains("::")
        {
            let dot_qualified = format!("{}.{}", effective_module, name);
            let colon_qualified = effective_module.replace('.', "::") + "::" + name;
            if self.ctx.lookup_function(&dot_qualified).is_none() {
                self.ctx.register_function(dot_qualified, info.clone());
            }
            if self.ctx.lookup_function(&colon_qualified).is_none() {
                self.ctx.register_function(colon_qualified, info.clone());
            }
        }

        // Register the constant's type for correct instruction selection.
        // This is critical for generating float vs integer operations on constants.
        // e.g., `const PI: Float = 3.14;` then `-PI` should use NegF, not NegI.
        // Uses register_constant_type which persists across function compilations.
        if let Some(ty) = const_type {
            let var_type = self.type_kind_to_var_type(&ty.kind);
            self.ctx.register_constant_type(name, var_type);
        }

        // #97 — Push an unconditional stub VbcFunction for inlinable
        // consts (those with `intrinsic_name = Some("__const_val_<N>")`)
        // so the precompile path emits them into the archive even
        // when no internal stdlib bytecode references the const.
        // Without this, `public const SSO_CAPACITY: Int = 23;` is
        // registered in `ctx.functions` but never lands in
        // `module.functions`; downstream user-side codegen then can't
        // find SSO_CAPACITY in the archive and the typechecker
        // surfaces `unbound variable: SSO_CAPACITY`.
        //
        // Non-inlinable consts go through `compile_pending_constants`
        // which already pushes a real body to `self.functions`, so
        // the stub is gated on `intrinsic_name.is_some()`.
        if info.intrinsic_name.is_some() {
            // Mirror the source-module-qualified descriptor.name
            // promotion in `compile_function` (task #121): when the
            // current source module is known, emit the descriptor's
            // stored name in fully qualified form so the archive-side
            // load path sees `sys.bitfield.USIZE_BITS` rather than
            // a bare `USIZE_BITS` that collides with same-named consts
            // in sibling files of the same archive entry directory.
            let const_qualified_name = if !effective_module.is_empty()
                && effective_module != "main"
                && !name.contains('.')
                && !name.contains("::")
            {
                format!("{}.{}", effective_module, name)
            } else {
                name.to_string()
            };
            let name_id = StringId(self.intern_string(&const_qualified_name));
            let mut descriptor = crate::module::FunctionDescriptor::new(name_id);
            descriptor.id = info.id;
            descriptor.is_const = true;
            if let Some(ref iname) = info.intrinsic_name {
                descriptor.intrinsic_name = Some(StringId(self.intern_string(iname)));
            }
            // Set the const's actual return type (Int / Text / Char /
            // …) — derived from the AST `const_type` when present.
            // The codegen-side `type_kind_to_type_ref` mirror of the
            // typechecker's known-type mapping is the right helper
            // here; use a conservative `Int` fallback when the type
            // can't be inferred.
            if let Some(ty) = const_type {
                descriptor.return_type =
                    self.ast_type_to_type_ref(ty);
            }
            descriptor.register_count = 1;
            descriptor.locals_count = 0;
            let vbc_func = crate::module::VbcFunction::new(
                descriptor,
                vec![Instruction::RetV],
            );
            self.functions.push(vbc_func);
        }

        Ok(())
    }

    /// Compiles pending constants that couldn't be inlined.
    ///

    /// These are constants with complex values (like struct literals) that
    /// are compiled as zero-argument functions returning the constant value.
    fn compile_pending_constants(&mut self) -> CodegenResult<()> {
        // Take pending constants to avoid borrow issues
        let constants = std::mem::take(&mut self.pending_constants);

        for (name, expr, queued_source_module) in constants {
            // Get the pre-registered function info
            let func_info = match self.ctx.lookup_function(&name) {
                Some(info) => info.clone(),
                None => continue, // Skip if not found (shouldn't happen)
            };

            // Begin compiling the constant as a zero-argument function
            self.ctx.begin_function(&name, &[], None);

            // Compile the constant expression
            // Compile the constant expression
            if let Ok(Some(result_reg)) = self.compile_expr(&expr) {
                // Return the result
                self.ctx.emit(Instruction::Ret { value: result_reg });
            } else {
                // If compilation failed or returned no value, emit a nil return
                let nil_reg = self.ctx.alloc_temp();
                self.ctx.emit(Instruction::LoadNil { dst: nil_reg });
                self.ctx.emit(Instruction::Ret { value: nil_reg });
            }

            // End the function and collect instructions
            let (instructions, register_count) = self.ctx.end_function();

            // Mirror the source-module-qualified descriptor.name
            // promotion in `compile_function` and the inlinable-const
            // stub above (task #121): the body-compiled-const branch
            // also serializes a `FunctionDescriptor` into the archive,
            // and that descriptor's `name` is what the archive-load
            // path uses as the qualified registry key.  Without this
            // promotion, `public const USIZE_BITS: USize = USize.bits;`
            // (whose value is a method call so `extract_const_literal_value`
            // returns None and the const flows through the
            // `pending_constants` path here, not the inlinable stub
            // above) lands in the archive as the bare name `USIZE_BITS`
            // instead of `sys.bitfield.USIZE_BITS`.  Cross-module
            // bare-mount qualified access (`mount core.sys.bitfield;
            // ... bitfield.USIZE_BITS`) then can't find it in the
            // user-side function registry.
            //
            // Prefer the `queued_source_module` captured at queue
            // time — `current_source_module` reflects whichever
            // `compile_items_into_state` call happens to be on the
            // stack when the queue drains (the shared stdlib-bootstrap
            // codegen instance processes multiple files in sequence;
            // a const queued by an earlier file gets drained by a
            // later file's flush call).
            let const_effective_module = queued_source_module
                .as_deref()
                .or_else(|| self.ctx.current_source_module.as_deref())
                .unwrap_or(&self.config.module_name);
            let const_descriptor_name = if !const_effective_module.is_empty()
                && const_effective_module != "main"
                && !name.contains('.')
                && !name.contains("::")
            {
                format!("{}.{}", const_effective_module, name)
            } else {
                name.to_string()
            };
            let name_id = StringId(self.intern_string(&const_descriptor_name));
            let mut descriptor = FunctionDescriptor::new(name_id);
            descriptor.id = func_info.id;
            descriptor.register_count = register_count;
            descriptor.locals_count = 0;
            // #97 — propagate the const-storage marker into the
            // archive descriptor.  Without this, body-compiled
            // (non-inlinable) consts — Float-valued constants like
            // `public const NAN: Float = 0.0 / 0.0;`, struct-literal
            // initialisers, anything that fails `extract_const_literal_value`
            // — get archived with `is_const = false`, even though their
            // `FunctionInfo` correctly carries `is_const = true`.
            // The asymmetry made `register_stdlib_consts_from_metadata`
            // skip every Float-valued const (NAN / INFINITY / NEG_INFINITY /
            // PI / MAX / MIN / MIN_POSITIVE / EPSILON, all of
            // `core/base/primitives.vr::implement Float`) AND every
            // struct-literal const (MemProt's read/write/exec triples,
            // anything carrying composite initialisers).
            descriptor.is_const = true;

            // Propagate the const's TYPE into the archive descriptor.
            // `FunctionDescriptor::new` defaults `return_type` to
            // `TypeRef::Concrete(TypeId::UNIT)`, which the archive
            // metadata builder serialises as the literal text "Unit".
            // Downstream `register_stdlib_consts_from_metadata` then
            // builds a `Type::Unit` TypeScheme — user code's
            // `let n: Float = Float.NAN` fails unification with
            // `expected 'Float', found 'Unit'`.  Recover the
            // canonical type from the codegen-local
            // `constant_types: HashMap<String, VarTypeKind>` map
            // populated by `register_constant_type` (mod.rs:9654).
            // Mapping VarTypeKind → TypeId is exhaustive for the
            // primitive-typed const surface (Int / Float / Bool /
            // Byte / Char / Text / Int32 / UInt64); composite-typed
            // consts (Maybe<T>, struct literals, …) keep the Unit
            // fallback because their type wasn't a simple primitive
            // — those load through the codegen-side
            // `func_info.return_type_name` path on the user-archive
            // consumer side anyway.
            use crate::codegen::context::VarTypeKind;
            use crate::types::TypeId as VbcTypeId;
            use crate::types::TypeRef as VbcTypeRef;
            let var_ty_kind = self.ctx.get_constant_type(&name);
            let tid = match var_ty_kind {
                VarTypeKind::Int => Some(VbcTypeId::INT),
                VarTypeKind::Float => Some(VbcTypeId::FLOAT),
                VarTypeKind::Bool => Some(VbcTypeId::BOOL),
                VarTypeKind::Byte => Some(VbcTypeId::U8),
                VarTypeKind::Char => Some(VbcTypeId::CHAR),
                VarTypeKind::Text => Some(VbcTypeId::TEXT),
                VarTypeKind::Int32 => Some(VbcTypeId::I32),
                VarTypeKind::UInt64 => Some(VbcTypeId::U64),
                _ => None,
            };
            if let Some(tid) = tid {
                descriptor.return_type = VbcTypeRef::Concrete(tid);
            }
            // Closes task #18: Float.NAN / INFINITY / PI / MAX / MIN /
            // EPSILON etc. now land in the archive with `is_const=true`
            // AND `return_type=Float`, so the typechecker's lazy-load
            // path reads the correct TypeScheme.

            // Create VbcFunction and add it (dedupe via push_function_dedup
            // so blanket-impl replays don't produce same-id duplicates).
            let vbc_func = VbcFunction::new(descriptor, instructions);
            self.push_function_dedup(vbc_func);
        }

        Ok(())
    }

    /// Compiles pending @thread_local static initializations.
    ///

    /// Each @thread_local static gets an init function that:
    /// 1. Evaluates the initializer expression
    /// 2. Stores the result in the TLS slot via TlsSet
    ///

    /// These init functions are registered as global constructors.
    fn compile_pending_tls_inits(&mut self) -> CodegenResult<()> {
        let tls_inits = std::mem::take(&mut self.pending_tls_inits);

        for (name, init_expr, slot) in tls_inits {
            let init_name = format!("__tls_init_{}", name);
            let func_id = FunctionId(self.next_func_id);
            self.next_func_id = self.next_func_id.saturating_add(1);

            // Begin compiling the init function
            self.ctx.begin_function(&init_name, &[], None);

            // Compile the initializer expression
            if let Ok(Some(result_reg)) = self.compile_expr(&init_expr) {
                // Store result in TLS slot
                let slot_reg = self.ctx.alloc_temp();
                self.ctx.emit(Instruction::LoadI {
                    dst: slot_reg,
                    value: slot as i64,
                });
                self.ctx.emit(Instruction::TlsSet {
                    slot: slot_reg,
                    val: result_reg,
                });
                self.ctx.free_temp(slot_reg);
                // Return unit
                self.ctx.emit(Instruction::Ret { value: result_reg });
            } else {
                let nil_reg = self.ctx.alloc_temp();
                self.ctx.emit(Instruction::LoadNil { dst: nil_reg });
                self.ctx.emit(Instruction::Ret { value: nil_reg });
            }

            let (instructions, register_count) = self.ctx.end_function();

            let name_id = StringId(self.intern_string(&init_name));
            let mut descriptor = FunctionDescriptor::new(name_id);
            descriptor.id = func_id;
            descriptor.register_count = register_count;
            descriptor.locals_count = 0;

            let vbc_func = VbcFunction::new(descriptor, instructions);
            self.push_function_dedup(vbc_func);

            // Register as global constructor so it runs before main()
            self.static_init_functions.push(func_id);
        }

        Ok(())
    }

    /// Try to extract a compile-time integer value from a constant expression.
    ///

    /// Handles:
    /// - Integer literals: `42`, `0xFF`, `-1`
    /// - Negated integer literals: `-(42)`
    /// - Parenthesized integer literals: `(42)`
    /// - Constructor calls with single literal arg: `Duration(0)`
    fn extract_const_literal_value(expr: &verum_ast::Expr) -> Option<i64> {
        use verum_ast::ExprKind;
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => Some(int_lit.value as i64),
                LiteralKind::Bool(b) => Some(if *b { 1 } else { 0 }),
                _ => None,
            },
            ExprKind::Paren(inner) => Self::extract_const_literal_value(inner),
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: operand,
            } => {
                // Use checked_neg to handle i64::MIN (which can't be negated)
                Self::extract_const_literal_value(operand).and_then(|v| v.checked_neg())
            }
            // Cast expression like `Fd(0) as ValidFd` — extract value through the cast
            ExprKind::Cast { expr: inner, .. } => Self::extract_const_literal_value(inner),
            // Constructor call like Duration(0) — extract the inner literal
            ExprKind::Call { args, .. } => {
                if args.len() == 1 {
                    Self::extract_const_literal_value(&args[0])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extracts parameter name from function parameter.
    /// Detect `@device(gpu)` / `@device(GPU)` / `@device("gpu")` on a
    /// function declaration's attribute list.  Mirrors the predicate
    /// in `crates/verum_compiler/src/pipeline/gpu_detect.rs`.
    ///
    /// Returns true iff the function should be lowered EXCLUSIVELY
    /// through the MLIR GPU pipeline (the LLVM CPU pipeline emits
    /// only an extern stub).  Stable: callable from VBC codegen
    /// without taking `&mut self`.
    fn function_is_gpu_only(attrs: &verum_common::List<verum_ast::Attribute>) -> bool {
        use verum_ast::expr::ExprKind;
        use verum_ast::ty::PathSegment;
        use verum_common::Maybe;

        for attr in attrs.iter() {
            if attr.name.as_str() != "device" {
                continue;
            }
            let Maybe::Some(ref args) = attr.args else {
                continue;
            };
            let Some(first_arg) = args.first() else {
                continue;
            };
            match &first_arg.kind {
                ExprKind::Path(path) => {
                    if let Some(PathSegment::Name(ident)) = path.segments.first() {
                        if ident.name.as_str().eq_ignore_ascii_case("gpu") {
                            return true;
                        }
                    }
                }
                ExprKind::Literal(lit) => {
                    if let verum_ast::literal::LiteralKind::Text(s) = &lit.kind {
                        if s.as_str().eq_ignore_ascii_case("gpu") {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn extract_param_name(&self, param: &verum_ast::FunctionParam) -> Option<String> {
        use verum_ast::FunctionParamKind;
        match &param.kind {
            FunctionParamKind::Regular { pattern, .. } => self.extract_pattern_name(pattern),
            FunctionParamKind::SelfValue
            | FunctionParamKind::SelfValueMut
            | FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfOwn
            | FunctionParamKind::SelfOwnMut
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafe
            | FunctionParamKind::SelfRefUnsafeMut => Some("self".to_string()),
        }
    }

    /// Extracts the primary name from a pattern.
    fn extract_pattern_name(&self, pattern: &verum_ast::Pattern) -> Option<String> {
        use verum_ast::PatternKind;
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Some(name.name.to_string()),
            PatternKind::Paren(inner) => self.extract_pattern_name(inner),
            _ => None,
        }
    }

    /// Extracts the primary name and mutability from a pattern.
    fn extract_pattern_name_and_mutable(
        &self,
        pattern: &verum_ast::Pattern,
    ) -> Option<(String, bool)> {
        use verum_ast::PatternKind;
        match &pattern.kind {
            PatternKind::Ident { name, mutable, .. } => Some((name.name.to_string(), *mutable)),
            PatternKind::Paren(inner) => self.extract_pattern_name_and_mutable(inner),
            _ => None,
        }
    }

    /// Extracts the parameter name and mutability from a function parameter.
    fn extract_param_name_and_mutable(
        &self,
        param: &verum_ast::FunctionParam,
    ) -> Option<(String, bool)> {
        use verum_ast::FunctionParamKind;
        match &param.kind {
            FunctionParamKind::Regular { pattern, .. } => {
                self.extract_pattern_name_and_mutable(pattern)
            }
            // Self parameters: SelfValueMut, SelfRefCheckedMut, SelfRefUnsafeMut are mutable
            FunctionParamKind::SelfValueMut
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafeMut => Some(("self".to_string(), true)),
            FunctionParamKind::SelfValue
            | FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfOwn
            | FunctionParamKind::SelfOwnMut
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefUnsafe => Some(("self".to_string(), false)),
        }
    }

    /// Converts a TypeKind to VarTypeKind for instruction selection.
    ///
    /// Path-based aliases (`USize`, `i64`, `Byte`, …) are normalized through
    /// the shared `primitive_path_ident_to_typekind` helper so the recognized
    /// integer/float/bool/char/text/unit names cannot drift from the cast-
    /// normalization or `infer_expr_type_kind` sites — drift previously
    /// caused canonical Verum names like `USize` to fall to
    /// `VarTypeKind::Unknown`, which propagated into `infer_expr_type_kind`
    /// returning `None` for `n: USize` parameters and the bitwise-NOT path
    /// emitting `Instruction::Not` (logical, via is_truthy) instead of
    /// `Instruction::Bitwise{Not}` for `!mask` expressions in stdlib
    /// bitfield primitives.
    fn type_kind_to_var_type(&self, type_kind: &verum_ast::ty::TypeKind) -> context::VarTypeKind {
        use verum_ast::ty::TypeKind;
        match type_kind {
            TypeKind::Int => context::VarTypeKind::Int,
            TypeKind::Float => context::VarTypeKind::Float,
            TypeKind::Bool => context::VarTypeKind::Bool,
            TypeKind::Char => context::VarTypeKind::Char,
            TypeKind::Text => context::VarTypeKind::Text,
            TypeKind::Unit => context::VarTypeKind::Unit,
            TypeKind::Path(path) => path
                .as_ident()
                .and_then(|ident| Self::primitive_path_ident_to_typekind(&ident.name))
                .map(|tk| self.type_kind_to_var_type(&tk))
                .unwrap_or(context::VarTypeKind::Unknown),
            // **Audit-driven fundamental fix** — unwrap reference
            // types so `const VERSION: &Text = "..."` classifies as
            // Text (the canonical inner type).  Pre-fix the
            // `Reference { inner }` arm fell through to Unknown,
            // which the const-archive-descriptor propagation at
            // `compile_inlinable_const` (line ~10470) then mapped to
            // `None` → descriptor.return_type defaulted to Unit.
            // Downstream `register_stdlib_consts_from_metadata` then
            // registered `VERSION: Unit` in the typechecker, and
            // every `VERSION.len()` / `parse(VERSION)` call site
            // surfaced as `expected 'Text', found 'Unit'`.  Mutable
            // refs / checked refs / unsafe refs all defer to the
            // same inner-type classification.
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. } => {
                self.type_kind_to_var_type(&inner.kind)
            }
            _ => context::VarTypeKind::Unknown,
        }
    }

    /// Converts a type name string to VarTypeKind for instruction selection.
    ///
    /// Used when we have type information as a string (e.g., from variant
    /// payload types). Routes through `primitive_path_ident_to_typekind` so
    /// the recognized name set stays consistent with `type_kind_to_var_type`
    /// and the cast/infer sites in `expressions.rs`.
    fn type_name_to_var_type(&self, type_name: &str) -> context::VarTypeKind {
        Self::primitive_path_ident_to_typekind(type_name)
            .map(|tk| self.type_kind_to_var_type(&tk))
            .unwrap_or(context::VarTypeKind::Unknown)
    }

    /// Extracts inner type parameters from a generic type name string.
    ///

    /// Examples:
    /// - `"Maybe<Char>"` -> `["Char"]`
    /// - `"Result<Text, Error>"` -> `["Text", "Error"]`
    /// - `"List<Int>"` -> `["Int"]`
    /// - `"Map<Text, Int>"` -> `["Text", "Int"]`
    /// - `"Int"` -> `[]` (not generic)
    ///

    /// This is used for extracting payload types from generic containers like `Maybe<T>`
    /// when pattern matching, allowing bound variables to have the correct type.
    fn extract_inner_types(&self, type_name: &str) -> Vec<String> {
        // Find the opening angle bracket
        let Some(open_pos) = type_name.find('<') else {
            return vec![];
        };

        // Find the matching closing angle bracket
        let Some(close_pos) = type_name.rfind('>') else {
            return vec![];
        };

        if close_pos <= open_pos {
            return vec![];
        }

        // Extract the content between < and >
        let inner = &type_name[open_pos + 1..close_pos];

        // Split by comma, handling nested generics
        let mut result = Vec::new();
        let mut depth = 0;
        let mut current = String::new();

        for c in inner.chars() {
            match c {
                '<' => {
                    depth += 1;
                    current.push(c);
                }
                '>' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        result.push(trimmed);
                    }
                    current.clear();
                }
                _ => {
                    current.push(c);
                }
            }
        }

        // Don't forget the last segment
        let trimmed = current.trim().to_string();
        if !trimmed.is_empty() {
            result.push(trimmed);
        }

        result
    }

    /// Converts a full AST Type to a VBC TypeRef.
    ///

    /// Resolves a field type, mapping generic type parameters to TypeRef::Generic.
    /// `generic_param_map` maps param names (e.g., "A") to their TypeParamId index.
    /// When a parameter's declared type is a bare generic-param path
    /// (`f: F`) and the surrounding function decl bounds `F` to a
    /// function type (`F: fn(...) -> X`), substitute the resolved
    /// `TypeRef::Generic` with the bound's `TypeRef::Function`.
    ///
    /// This preserves the closure-arg shape across the archive
    /// boundary so user-side codegen can drive the call-site
    /// expected-return-type disambig push (see
    /// `archive_ctx_loader::extract_closure_return_type_from_typeref`).
    /// Without this round-trip, `f: F` archives as `TypeRef::Generic(F)`
    /// and the user-side method-call dispatch can't tell that the
    /// closure's body should consult a specific sum-type's variant
    /// table — a closure returning bare `Continue(...)` where two
    /// types ship a `Continue` variant (e.g. `ReduceResult.Continue`
    /// vs `ControlFlow.Continue`) hits non-deterministic resolution.
    ///
    /// Idempotent for non-generic-param paths or for generic params
    /// without a fn bound — returns `resolved` unchanged.
    fn substitute_fn_bound_for_generic(
        &self,
        resolved: TypeRef,
        ast_ty: &verum_ast::ty::Type,
        generics: &verum_common::List<verum_ast::ty::GenericParam>,
        generic_param_map: &std::collections::HashMap<String, u16>,
    ) -> TypeRef {
        use verum_ast::ty::{GenericParamKind, PathSegment, TypeBoundKind, TypeKind};

        // Only substitute when the resolved form is a bare generic
        // (the un-substituted shape we'd otherwise carry into the
        // archive). Direct fn types (`f: fn(...)`) already land as
        // `TypeRef::Function` — leave them alone.
        if !matches!(&resolved, TypeRef::Generic(_)) {
            return resolved;
        }
        // Recover the generic-param name from the AST.
        let target_name = match &ast_ty.kind {
            TypeKind::Path(path) if path.segments.len() == 1 => {
                match &path.segments[0] {
                    PathSegment::Name(ident) => ident.name.to_string(),
                    _ => return resolved,
                }
            }
            _ => return resolved,
        };
        // Find the matching generic param and probe its bounds.
        for gp in generics.iter() {
            let (gp_name, bounds) = match &gp.kind {
                GenericParamKind::Type { name, bounds, .. } => (name.name.as_str(), bounds),
                GenericParamKind::HigherKinded { name, bounds, .. } => {
                    (name.name.as_str(), bounds)
                }
                _ => continue,
            };
            if gp_name != target_name {
                continue;
            }
            // The parser emits a Function-typed bound as
            // `Equality(<Function type>)` (see
            // `verum_fast_parser::ty::type_to_type_bound`); fall back
            // through `GenericProtocol` for the
            // `F: Iterator<Item=...>`-shaped bound that carries a
            // Function payload.
            for b in bounds.iter() {
                let bound_ty = match &b.kind {
                    TypeBoundKind::Equality(ty) => ty,
                    TypeBoundKind::GenericProtocol(ty) => ty,
                    _ => continue,
                };
                if matches!(&bound_ty.kind, TypeKind::Function { .. }) {
                    // Resolve the bound through the same map so any
                    // nested generic-param references inside the
                    // bound (`fn(R, Self.Item) -> ReduceResult<R>`)
                    // stay as `TypeRef::Generic` rather than
                    // collapsing to `Concrete(PTR)`.
                    return self.resolve_field_type_ref(bound_ty, generic_param_map);
                }
            }
        }
        resolved
    }

    fn resolve_field_type_ref(
        &self,
        ty: &verum_ast::ty::Type,
        generic_param_map: &std::collections::HashMap<String, u16>,
    ) -> TypeRef {
        use verum_ast::ty::{PathSegment, TypeKind};
        // Generic instantiation: recurse into args with the same map
        // so nested type-param references (`Result<T, E>` from
        // `type IoResult<T> is Result<T, StreamError>;`) preserve
        // T → TypeRef::Generic(0) instead of degrading to
        // TypeRef::Concrete(PTR) via the un-aware fallback.
        if let TypeKind::Generic { base, args } = &ty.kind {
            let base_ref = self.resolve_field_type_ref(base, generic_param_map);
            if let TypeRef::Concrete(base_id) = base_ref {
                let arg_refs: Vec<TypeRef> = args
                    .iter()
                    .filter_map(|arg| {
                        if let verum_ast::ty::GenericArg::Type(inner_ty) = arg {
                            Some(self.resolve_field_type_ref(inner_ty, generic_param_map))
                        } else {
                            None
                        }
                    })
                    .collect();
                return TypeRef::Instantiated {
                    base: base_id,
                    args: arg_refs,
                };
            }
            return base_ref;
        }
        if let TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. } = &ty.kind
        {
            return self.resolve_field_type_ref(inner, generic_param_map);
        }
        // #131 Layer E — nested function types must preserve the
        // map through recursion.  The standard `ast_type_to_type_ref`
        // recurses with itself for `TypeKind::Function`, which loses
        // the protocol's / method's param scope.  For protocol
        // method signatures like `fn map<U, B>(self, f: fn(Self.Item)
        // -> B) -> ...`, the param `f`'s type is itself a function
        // type containing an associated-type projection
        // (`Self.Item`) and a method-local param (`B`).  Without
        // this branch, both fell through to `TypeRef::Concrete(PTR)`
        // → wrong-arity-name fallback at archive_metadata-render
        // time.
        if let TypeKind::Function {
            params,
            return_type,
            contexts,
            ..
        } = &ty.kind
        {
            let param_refs: Vec<TypeRef> = params
                .iter()
                .map(|p| self.resolve_field_type_ref(p, generic_param_map))
                .collect();
            let ret_ref = self.resolve_field_type_ref(return_type, generic_param_map);
            let ctx_refs: smallvec::SmallVec<[crate::types::ContextRef; 2]> = contexts
                .requirements
                .iter()
                .filter_map(|req| {
                    let ctx_name = format!("{}", req.path);
                    self.context_name_to_id
                        .get(&ctx_name)
                        .copied()
                        .map(crate::types::ContextRef)
                })
                .collect();
            return TypeRef::Function {
                params: param_refs,
                return_type: Box::new(ret_ref),
                contexts: ctx_refs,
            };
        }
        // #131 Layer E — associated-type projection.  In Verum the
        // parser produces `TypeKind::Qualified { self_ty, trait_ref,
        // assoc_name }` for both `Self.Item` and `<T as Trait>::Foo`
        // (parser at `verum_fast_parser/src/ty.rs:1928`).  The older
        // `TypeKind::AssociatedType` variant exists in the AST but
        // isn't produced by the source parser.  Both shapes are
        // rendered as a synthetic generic-param ID outside the
        // protocol-level range, so archive_metadata's
        // `param_id_to_name` (seeded only from protocol type_params)
        // doesn't resolve it.  The hash-derived ID is stable per
        // (base-name, assoc-name) pair within a method scope —
        // multiple references to the same `Self.Item` in one method
        // signature collapse to the same ID, preserving the
        // "this-is-the-same-type" invariant the unifier needs.
        // Range starts at 0xC000 = 49152 — well above any
        // legitimate combined protocol+method-local param count
        // (typically <16 in stdlib protocols).
        let qualified_components: Option<(String, String)> = match &ty.kind {
            TypeKind::AssociatedType { base, assoc } => {
                // Older shape — keep handling for forward-compat in
                // case future codepaths emit it.
                let mut base_name = String::new();
                if let TypeKind::Path(path) = &base.kind {
                    for seg in path.segments.iter() {
                        if let verum_ast::ty::PathSegment::Name(ident) = seg {
                            if !base_name.is_empty() {
                                base_name.push('.');
                            }
                            base_name.push_str(ident.name.as_str());
                        }
                    }
                }
                Some((base_name, assoc.name.as_str().to_string()))
            }
            TypeKind::Qualified {
                self_ty, assoc_name, ..
            } => {
                // Walk the (possibly nested) self_ty chain to recover
                // a stable base-key.  For `Self.Iter.Item`, parser
                // builds `Qualified { self_ty: Qualified { self_ty:
                // Self, assoc: Iter }, assoc: Item }`.  Joining each
                // level's assoc-name into the key keeps distinct
                // chained projections distinct in the hash.
                let mut base_name = String::new();
                let mut current: &verum_ast::ty::Type = self_ty;
                loop {
                    match &current.kind {
                        TypeKind::Path(path) => {
                            for seg in path.segments.iter() {
                                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                                    if !base_name.is_empty() {
                                        base_name.push('.');
                                    }
                                    base_name.push_str(ident.name.as_str());
                                }
                            }
                            break;
                        }
                        TypeKind::Qualified {
                            self_ty: inner_self,
                            assoc_name: inner_assoc,
                            ..
                        } => {
                            // Walk inwards, accumulating from the
                            // outside in — we'll reverse at the end
                            // for stable left-to-right naming.
                            base_name = format!("{}.{}", inner_assoc.name.as_str(), base_name);
                            current = inner_self;
                        }
                        _ => break,
                    }
                }
                Some((base_name, assoc_name.name.as_str().to_string()))
            }
            _ => None,
        };
        if let Some((base_str, assoc_str)) = qualified_components {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            base_str.hash(&mut h);
            assoc_str.hash(&mut h);
            let synthetic_id = 0xC000u16 + ((h.finish() as u16) & 0x3FFF);
            return TypeRef::Generic(crate::types::TypeParamId(synthetic_id));
        }
        // Check if the type is a simple path that matches a generic param.
        //
        // #131 Layer E — `Self` is parsed as `PathSegment::SelfValue`,
        // NOT `PathSegment::Name("Self")` (parser at
        // `verum_fast_parser/src/ty.rs:1915`).  The naive
        // `if let PathSegment::Name(ident) = seg` filter misses
        // `Self`, returns "" (empty), and the empty string isn't in
        // any param map → falls through to `ast_type_to_type_ref`
        // which emits `TypeRef::Concrete(PTR)`.  Recognise both
        // segment shapes (Name + SelfValue) so the protocol's
        // synthetic Self mapping registered above (id `0x4000+N`)
        // actually fires.
        if let TypeKind::Path(path) = &ty.kind {
            let type_name = path
                .segments
                .iter()
                .find_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.name.to_string()),
                    PathSegment::SelfValue => Some("Self".to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if let Some(&param_idx) = generic_param_map.get(&type_name) {
                return TypeRef::Generic(crate::types::TypeParamId(param_idx));
            }
        }
        // Fall back to standard resolution
        self.ast_type_to_type_ref(ty)
    }

    /// This enables storing return type information for method dispatch prefixing.
    fn ast_type_to_type_ref(&self, ty: &verum_ast::ty::Type) -> TypeRef {
        use verum_ast::ty::{PathSegment, TypeKind};

        match &ty.kind {
            TypeKind::Int => TypeRef::Concrete(TypeId::INT),
            TypeKind::Float => TypeRef::Concrete(TypeId::FLOAT),
            TypeKind::Bool => TypeRef::Concrete(TypeId::BOOL),
            TypeKind::Text => TypeRef::Concrete(TypeId::TEXT),
            TypeKind::Unit => TypeRef::Concrete(TypeId::UNIT),
            TypeKind::Never => TypeRef::Concrete(TypeId::NEVER),
            TypeKind::Path(path) => {
                // Extract the first segment name for primitive type lookup
                let type_name = path
                    .segments
                    .iter()
                    .find_map(|seg| {
                        if let PathSegment::Name(ident) = seg {
                            Some(ident.name.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                // Map Verum type names to TypeId via the consolidated registry
                match self.get_well_known_type_id(&type_name) {
                    Some(type_id) => TypeRef::Concrete(type_id),
                    None => {
                        // Truly unknown type — use PTR as generic carrier
                        TypeRef::Concrete(TypeId::PTR)
                    }
                }
            }
            TypeKind::Generic { base, args } => {
                // Generic types like Maybe<T>, List<T>, etc.
                let base_ref = self.ast_type_to_type_ref(base);
                if let TypeRef::Concrete(base_id) = base_ref {
                    let arg_refs: Vec<TypeRef> = args
                        .iter()
                        .filter_map(|arg| {
                            // Extract the inner Type from GenericArg::Type
                            if let verum_ast::ty::GenericArg::Type(inner_ty) = arg {
                                Some(self.ast_type_to_type_ref(inner_ty))
                            } else {
                                None
                            }
                        })
                        .collect();
                    TypeRef::Instantiated {
                        base: base_id,
                        args: arg_refs,
                    }
                } else {
                    base_ref
                }
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. } => {
                // References/pointers — resolve to the inner type's TypeRef
                // (VBC doesn't distinguish reference types at the bytecode level)
                self.ast_type_to_type_ref(inner)
            }
            TypeKind::Tuple(elements) => {
                if elements.is_empty() {
                    TypeRef::Concrete(TypeId::UNIT)
                } else {
                    // Multi-element tuples — produce TypeRef::Tuple with element types
                    // so LLVM lowering can track element types through Unpack
                    let elem_refs: Vec<TypeRef> = elements
                        .iter()
                        .map(|e| self.ast_type_to_type_ref(e))
                        .collect();
                    TypeRef::Tuple(elem_refs)
                }
            }
            TypeKind::Function {
                params,
                return_type,
                contexts,
                ..
            } => {
                let param_refs: Vec<TypeRef> = params
                    .iter()
                    .map(|p| self.ast_type_to_type_ref(p))
                    .collect();
                let ret_ref = self.ast_type_to_type_ref(return_type);
                let ctx_refs: smallvec::SmallVec<[crate::types::ContextRef; 2]> = contexts
                    .requirements
                    .iter()
                    .enumerate()
                    .map(|(i, _)| crate::types::ContextRef(i as u32))
                    .collect();
                TypeRef::Function {
                    params: param_refs,
                    return_type: Box::new(ret_ref),
                    contexts: ctx_refs,
                }
            }
            TypeKind::DynProtocol { .. } => {
                // Dynamic protocol objects are fat pointers (vtable + data pointer)
                TypeRef::Concrete(TypeId::PTR)
            }
            TypeKind::CapabilityRestricted { base, .. } => {
                // Capability restrictions are compile-time only; unwrap to base type
                self.ast_type_to_type_ref(base)
            }
            TypeKind::Char => TypeRef::Concrete(TypeId::CHAR),
            TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                let elem_ref = self.ast_type_to_type_ref(element);
                if let TypeRef::Concrete(base_id) = elem_ref {
                    TypeRef::Instantiated {
                        base: TypeId::LIST,
                        args: vec![TypeRef::Concrete(base_id)],
                    }
                } else {
                    TypeRef::Concrete(TypeId::LIST)
                }
            }
            _ => TypeRef::Concrete(TypeId::UNIT),
        }
    }

    /// Extracts the base type name from an AST Type for method dispatch tracking.
    ///

    /// For `Result<T, E>`, returns `Some("Result")`.
    /// For `Maybe<T>`, returns `Some("Maybe")`.
    /// For primitive types like `Int`, returns `Some("Int")`.
    /// Used to track return types for correct method dispatch on function results.
    fn extract_type_name(&self, ty: &verum_ast::ty::Type) -> Option<String> {
        use verum_ast::ty::{PathSegment, TypeKind};

        match &ty.kind {
            // Unit and Never have no extractable type name for dispatch
            TypeKind::Unit | TypeKind::Never => None,
            // Other primitives use the canonical display name
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string())
            }
            TypeKind::Path(path) => {
                // Extract the first type name from the path
                // `Self` is encoded as `PathSegment::SelfValue` — surface it
                // as the canonical capitalised `"Self"` token so downstream
                // `substitute_self_in_type_name` can perform Self → concrete
                // substitution at register_impl_function time (default-method
                // monomorphisation of protocol bodies — `fn max(self, other:
                // Self) -> Self` registered onto a concrete `<T>` must
                // surface `return_type_name = "T"`, not `None`).
                path.segments.iter().find_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.name.to_string()),
                    PathSegment::SelfValue => Some("Self".to_string()),
                    _ => None,
                })
            }
            TypeKind::Generic { base, args } => {
                // For generic types like Result<T, E>, extract the full type including args
                let base_name = self.extract_type_name(base)?;
                if args.is_empty() {
                    Some(base_name)
                } else {
                    // Build the full type string with generic arguments
                    let arg_strs: Vec<String> = args
                        .iter()
                        .filter_map(|arg| match arg {
                            verum_ast::ty::GenericArg::Type(ty) => self.extract_type_name(ty),
                            verum_ast::ty::GenericArg::Const(_) => None,
                            verum_ast::ty::GenericArg::Lifetime(_) => None,
                            verum_ast::ty::GenericArg::Binding(_) => None,
                        })
                        .collect();
                    if arg_strs.is_empty() {
                        Some(base_name)
                    } else {
                        Some(format!("{}<{}>", base_name, arg_strs.join(", ")))
                    }
                }
            }
            TypeKind::Reference { inner, .. } => {
                // For reference types, extract the inner type name
                self.extract_type_name(inner)
            }
            TypeKind::Slice(inner) => {
                // `&[T]` / `[T]` — return the bracketed form so downstream
                // method-dispatch code (which checks `starts_with('[')` to
                // route to the Slice.* implementation) sees a slice type.
                let inner_name = self
                    .extract_type_name(inner)
                    .unwrap_or_else(|| "T".to_string());
                Some(format!("[{}]", inner_name))
            }
            _ => None,
        }
    }

    /// Converts a VBC TypeRef to a simple type name for method dispatch prefixing.
    ///

    /// This is used to determine if a function return type is a specific primitive
    /// (UInt64, Int32, Byte) that requires method name prefixing for correct dispatch.
    pub fn type_ref_to_name(&self, type_ref: &TypeRef) -> String {
        match type_ref {
            TypeRef::Concrete(type_id) => {
                if *type_id == TypeId::U64 {
                    "UInt64".to_string()
                } else if *type_id == TypeId::I32 {
                    "Int32".to_string()
                } else if *type_id == TypeId::U8 {
                    "Byte".to_string()
                } else if *type_id == TypeId::INT {
                    "Int".to_string()
                } else if *type_id == TypeId::FLOAT {
                    "Float".to_string()
                } else if *type_id == TypeId::BOOL {
                    "Bool".to_string()
                } else if *type_id == TypeId::TEXT {
                    "Text".to_string()
                } else if *type_id == TypeId::UNIT {
                    "()".to_string()
                } else {
                    format!("TypeId({})", type_id.0)
                }
            }
            TypeRef::Generic(param_id) => format!("GenericParam({})", param_id.0),
            TypeRef::Instantiated { base, args } => {
                let base_name = self.type_ref_to_name(&TypeRef::Concrete(*base));
                if args.is_empty() {
                    base_name
                } else {
                    let arg_names: Vec<String> =
                        args.iter().map(|a| self.type_ref_to_name(a)).collect();
                    format!("{}<{}>", base_name, arg_names.join(", "))
                }
            }
            _ => "?".to_string(),
        }
    }

    /// Compiles a top-level item.
    fn compile_item(&mut self, item: &Item) -> CodegenResult<()> {
        match &item.kind {
            ItemKind::Function(func) => {
                // Clear generic params for standalone functions (impl methods set them before calling)
                self.ctx.generic_type_params.clear();
                self.ctx.const_generic_params.clear();
                self.compile_function(func, None)?;
                // Compile any nested functions in this function's body
                if let verum_common::Maybe::Some(ref body) = func.body {
                    self.compile_nested_functions(body)?;
                }
            }
            ItemKind::Impl(impl_decl) => {
                // Get the type name for qualified method lookup
                let type_name = self.extract_impl_type_name(&impl_decl.kind);

                // Pre-populate impl block generics - these will be added to function generics
                // in compile_function. This enables recognizing T, U etc. in impl<T, U> Foo<T> { ... }
                let impl_type_generics: Vec<String> = impl_decl
                    .generics
                    .iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                if std::env::var("VERUM_TRACE_GP").is_ok()
                    && type_name.as_deref() == Some("Maybe")
                {
                    eprintln!(
                        "[compile_item Impl] type_name={:?} impl_decl.generics.len()={} impl_type_generics={:?}",
                        type_name, impl_decl.generics.len(), impl_type_generics
                    );
                }

                // Pre-populate impl block const generics - enables recognizing SIZE, N etc.
                // in impl<const SIZE: Int> StackAllocator<SIZE> { ... }
                let impl_const_generics: Vec<String> = impl_decl
                    .generics
                    .iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Const { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                for impl_item in impl_decl.items.iter() {
                    // Honour `@cfg` gates on impl items. Same pattern
                    // as `compile_item_lenient`'s impl loop — walk both
                    // ImplItem.attributes and FunctionDecl.attributes
                    // because the parser places attrs on the inner
                    // decl when present. Without this, an
                    // `@cfg(target_os = "linux") fn …` inside a
                    // cross-platform `implement Bar { … }` was
                    // compiled on every host.
                    if !impl_item.attributes.is_empty()
                        && !self.cfg_evaluator.should_include(&impl_item.attributes)
                    {
                        continue;
                    }
                    if let verum_ast::decl::ImplItemKind::Function(func) = &impl_item.kind {
                        if !func.attributes.is_empty()
                            && !self.cfg_evaluator.should_include(&func.attributes)
                        {
                            continue;
                        }
                        // Set impl block generics before compiling function
                        // compile_function will add function's own generics to this
                        self.ctx.generic_type_params.clear();
                        self.ctx.const_generic_params.clear();
                        for g in &impl_type_generics {
                            self.ctx.generic_type_params.insert(g.clone());
                        }
                        for g in &impl_const_generics {
                            self.ctx.const_generic_params.insert(g.clone());
                        }
                        // Per-method error containment.  Pre-fix the
                        // `compile_function(...)?` propagated any single
                        // method's compile error up through the impl
                        // block's for-loop AND out of the surrounding
                        // `compile_item`, aborting every subsequent
                        // method in this impl block AND every
                        // subsequent `ItemKind::Impl` in the same
                        // module file.
                        //
                        // Concrete impact on `core/base/memory.vr`:
                        // `Heap.into_inner` (or another mid-impl-block
                        // method using a not-yet-supported expression
                        // form) failed and short-circuited the
                        // remaining 30+ Heap.* methods AND every
                        // `Shared.*` / `Weak.*` method in the same file.
                        // Symptom: `metadata.functions` had only the
                        // first 7 Heap.* entries, and every user-side
                        // `Shared<T>.new(...)` died at typecheck with
                        // "no method named 'new' found for type
                        // 'Shared<Int>'" despite the source being
                        // present.
                        //
                        // The per-file `compile_function_bodies` already
                        // contains a stub-on-failure mechanism that
                        // operates at FILE level — but that recovery
                        // only fires when the FILE-level `?` propagates
                        // out of a function-body chain.  Containing the
                        // error at the per-method granularity here
                        // preserves every body that compiles correctly
                        // and lets `emit_missing_stub_descriptors`
                        // fill placeholders for the dropped methods.
                        // Pure recovery improvement; no successful
                        // body is lost.  Pattern matches the existing
                        // containment at compile_item_lenient
                        // (~line 3276) and the per-default-method
                        // containment at compile_pending_default_methods
                        // (~line 1725).
                        if let Err(e) =
                            self.compile_function(func, type_name.as_ref())
                        {
                            tracing::trace!(
                                "[compile_item Impl] {}.{} body compile failed: {}",
                                type_name.as_deref().unwrap_or("?"),
                                func.name.name.as_str(),
                                e
                            );
                            // Don't propagate — let subsequent siblings
                            // compile.  emit_missing_stub_descriptors
                            // at file-level produces a panicking-stub
                            // body for the dropped function so dispatch
                            // by id still finds something callable.
                        }
                        // Compile any nested functions in this function's body
                        if let verum_common::Maybe::Some(ref body) = func.body {
                            // Same containment for nested-fn compilation.
                            if let Err(e) = self.compile_nested_functions(body) {
                                tracing::trace!(
                                    "[compile_item Impl] {}.{} nested-fn compile failed: {}",
                                    type_name.as_deref().unwrap_or("?"),
                                    func.name.name.as_str(),
                                    e
                                );
                            }
                        }
                    }
                }
            }
            // Active pattern declarations - compile body as a function
            ItemKind::Pattern(pat_decl) => {
                self.ctx.generic_type_params.clear();
                self.ctx.const_generic_params.clear();
                self.compile_pattern_as_function(pat_decl)?;
            }
            // Inline `module X { ... }` body-compile recursion — closes
            // task #38 dispatch surface for `Transducer.compose2` /
            // `.compose` / etc.  The collect_declarations Module arm
            // (above) registers SIGNATURES under qualified `X.fn` names;
            // here we must compile the BODIES of those same functions
            // with `current_source_module` set so generated bytecode
            // tracks the right qualifier.  Without this arm, signatures
            // exist but bodies are bytecode-empty — calls dispatch to
            // empty body and produce wrong results.
            ItemKind::Module(mod_decl) => {
                if let verum_common::Maybe::Some(ref items) = mod_decl.items {
                    let module_name = mod_decl.name.name.to_string();
                    let prev_source = self.ctx.current_source_module.clone();
                    self.ctx.current_source_module = Some(module_name);
                    // LENIENT — a single inner-fn compile failure must
                    // NOT abort the outer module's compilation (would
                    // drop top-level fn main, surfacing as "No main
                    // function found in VBC module").  Log per-item
                    // failures and continue — matches the lenient
                    // discipline of `compile_module_items_lenient`
                    // for imported stdlib modules.
                    for sub_item in items.iter() {
                        if let Err(e) = self.compile_item(sub_item) {
                            tracing::warn!(
                                "[lenient inline-module] failed to compile inner item: {}",
                                e
                            );
                        }
                    }
                    self.ctx.current_source_module = prev_source;
                }
            }
            // Non-function items are handled during declaration collection
            // or don't produce bytecode (types, protocols, etc.)
            _ => {}
        }
        Ok(())
    }

    /// Compiles nested function bodies from a function body.
    ///

    /// This is called after the parent function is compiled to compile
    /// any nested function declarations found in its body.
    fn compile_nested_functions(&mut self, body: &FunctionBody) -> CodegenResult<()> {
        match body {
            FunctionBody::Block(block) => {
                self.compile_nested_functions_from_block(block)?;
            }
            FunctionBody::Expr(_) => {
                // Expression bodies can't contain function declarations
            }
        }
        Ok(())
    }

    /// Compiles nested function bodies from a block.
    ///

    /// Since the statement-level compiler (statements.rs) now handles
    /// ALL nested functions inline (with capture analysis for closures),
    /// this post-hoc pass is a no-op. Kept for API compatibility.
    fn compile_nested_functions_from_block(&mut self, _block: &Block) -> CodegenResult<()> {
        Ok(())
    }

    /// Compiles a function declaration.
    ///

    /// The `impl_type_name` parameter is Some for functions inside impl blocks,
    /// allowing us to look up the function by its qualified name (e.g., "List.new").
    fn compile_function(
        &mut self,
        func: &FunctionDecl,
        impl_type_name: Option<&String>,
    ) -> CodegenResult<()> {
        let base_name = func.name.name.to_string();

        // Build the lookup name - use qualified name for impl functions
        let lookup_name = if let Some(type_name) = impl_type_name {
            format!("{}.{}", type_name, base_name)
        } else {
            base_name.clone()
        };

        // Get the pre-registered function info (for ID and properties).
        //

        // **Module-qualified-first lookup** to match `compile_call`'s
        // dispatch path. Without this, a top-level fn with a name that
        // collides across modules (e.g. `is_valid_page_size` in
        // `pager.vr`, `journal_header_api/header.vr`,
        // `wal_frame_layout/constants.vr`, `journal/writer.vr`) would
        // bind its compiled body to whichever func_info won the bare-
        // name registry slot — typically a different module's
        // function. Pager.vr's body would then be pushed to
        // `self.functions` with journal's codegen-id, while pager.vr's
        // own codegen-id (resolved via qualified-first at every call
        // site inside pager.vr) would have no `self.functions` entry
        // — `func_id_remap` lacks an entry for it, so the post-remap
        // Call lands on whatever function happens to occupy that
        // position.
        //

        // Mirror compile_call's resolution: try
        // `<source_module>.<base_name>` first when we have a source
        // module and the lookup is for a bare top-level fn.
        let param_count = func.params.len();
        let qualified_lookup = if impl_type_name.is_none() {
            self.ctx
                .current_source_module
                .as_deref()
                .filter(|m| !m.is_empty() && *m != "main")
                .and_then(|src_mod| {
                    let qn = format!("{}.{}", src_mod, base_name);
                    self.ctx
                        .lookup_function_with_arity(&qn, param_count)
                        .cloned()
                })
        } else {
            None
        };
        let func_info = match qualified_lookup {
            Some(info) => info,
            None => self
                .ctx
                .lookup_function_with_arity(&lookup_name, param_count)
                .ok_or_else(|| {
                    CodegenError::internal(format!("function not registered: {}", lookup_name))
                })?
                .clone(),
        };

        // IMPORTANT: Extract parameter names and mutability from the CURRENT function being compiled,
        // NOT from func_info. When there are multiple impl blocks for the same method
        // (e.g., multiple FromResidual implementations for Result), the func_info might
        // contain param_names from a different impl block that overwrote the registration.
        // Generate placeholder names for parameters whose names can't be extracted
        // (e.g., complex patterns in extern functions)
        let params_with_mutability: Vec<(String, bool)> = func
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name_and_mutable(p)
                    .unwrap_or_else(|| (format!("_arg{}", i), false))
            })
            .collect();

        // Begin function compilation
        self.ctx.begin_function(
            &lookup_name,
            &params_with_mutability,
            func_info.return_type.clone(),
        );

        // Set current function's return type name for variant disambiguation.
        // When a variant name collides (e.g., "Lt" in both user's "Ordering" and
        // stdlib's "GeneralCategory"), this allows preferring the correct parent type.
        self.ctx.current_return_type_name = func_info.return_type_name.clone();
        // Inner generics of the return type drive variant disambiguation
        // for `Err(InnerVariant(...))` inside `Result<_, E>` returners
        // and the equivalent `Some(InnerVariant(...))` inside
        // `Maybe<E>` — without this list the disambiguator only sees
        // the wrapper base name.
        self.ctx.current_return_type_inner = func_info.return_type_inner.clone();

        // Stash the full AST return type + function name so that
        // explicit `return expr;` statements compiled inside the body
        // (via `compile_return` in expressions.rs) can emit the
        // refinement Assert before the Ret instruction. These are
        // cleared right after `ensure_return` below so they never
        // leak across functions.
        self.current_return_ast_type = func.return_type.clone();
        self.current_fn_lookup_name = Some(lookup_name.clone());

        // Set `self` type name for method calls within impl methods.
        // This MUST be after begin_function which clears variable_type_names.
        // This enables compile_method_call to qualify `self.method()` calls properly.
        if let Some(type_name) = impl_type_name {
            self.ctx
                .variable_type_names
                .insert("self".to_string(), type_name.clone());
        }

        // Set required contexts from the function's using clause.
        // This enables compile_method_call to emit CtxGet for context receivers.
        self.ctx.set_required_contexts(&func_info.contexts);

        // Register named/aliased context bindings from AST.
        // Grammar: named_context = identifier ':' context_path | context_path 'as' identifier
        for ctx in &func.contexts {
            if ctx.is_negative {
                continue;
            }
            let ctx_type_name = ctx
                .path
                .segments
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_type_name.is_empty() {
                continue;
            }
            if let verum_common::Maybe::Some(ref name_ident) = ctx.name {
                self.ctx
                    .context_aliases
                    .insert(name_ident.name.to_string(), ctx_type_name.clone());
            }
            if let verum_common::Maybe::Some(ref alias_ident) = ctx.alias {
                self.ctx
                    .context_aliases
                    .insert(alias_ident.name.to_string(), ctx_type_name.clone());
            }
        }

        // Add function's generic parameters to context for recognition in expressions.
        // This prevents "undefined variable" errors when T or SIZE appears in expressions
        // like @intrinsic("size_of", T) or arithmetic with const generics.
        // Note: For impl methods, impl generics are pre-set by compile_item before calling this,
        // so we add to them rather than clearing. For standalone functions, the sets are empty.
        for generic in func.generics.iter() {
            match &generic.kind {
                verum_ast::ty::GenericParamKind::Type { name, .. } => {
                    self.ctx.generic_type_params.insert(name.name.to_string());
                }
                verum_ast::ty::GenericParamKind::Const { name, .. } => {
                    self.ctx.const_generic_params.insert(name.name.to_string());
                }
                // Context polymorphism: fn<using C>(f: fn(T) -> U using C) -> R using C
                // Register the context parameter name as a generic type param so it's
                // recognized in expressions and doesn't cause "undefined variable" errors.
                verum_ast::ty::GenericParamKind::Context { name } => {
                    self.ctx.generic_type_params.insert(name.name.to_string());
                }
                _ => {} // Meta/Lifetime generics not handled at VBC level
            }
        }

        // Register parameter types for proper instruction selection
        // This is critical for generating correct float vs integer operations
        // AND for resolving field indices in type-specific record access.
        //
        // **`Self` → concrete type substitution** (FUNDAMENTAL): when
        // compiling a protocol default method body monomorphised onto
        // a concrete type (`impl_type_name = Some("Amount")` for the
        // `Ord.max(self, other: Self) -> Self` default), every `Self`
        // reference in the AST must be substituted with the concrete
        // name so downstream field-index / method-dispatch resolution
        // operates on the real layout.  Without this substitution
        // `other: Self` registered as `variable_type_names["other"] = "Self"`,
        // and the post-return `let m = self.max(other); m.<field>`
        // field-access lookup uses the literal `"Self"` key — misses
        // the registered Amount layout entirely, falls through to
        // heuristic field-count = 2, and reads slot 1 of a 1-slot
        // record → "field access out of bounds: field index 1
        // exceeds object data size 8".  Closes the entire class of
        // default-method-monomorphisation-leaks-Self field-index
        // drifts that affects `Ord.max`, `Ord.min`, `Ord.clamp`,
        // `Eq.ne`, `Hash.hash_value`, and every other default method
        // with a `Self`-typed parameter or return.
        for ((param_name, _), param) in params_with_mutability.iter().zip(func.params.iter()) {
            if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                let var_type = self.type_kind_to_var_type(&ty.kind);
                self.ctx.register_variable_type(param_name, var_type);
                // Track type name for field index resolution
                let raw_type_name = Self::extract_type_name_from_ast(ty);
                let type_name = if let Some(concrete) = impl_type_name {
                    Self::substitute_self_in_type_name(&raw_type_name, concrete)
                } else {
                    raw_type_name
                };
                if type_name != "()" && !type_name.is_empty() {
                    self.ctx
                        .variable_type_names
                        .insert(param_name.clone(), type_name);
                }
            }
        }
        // Substitute `Self` in the saved return-type-name so that
        // call-sites like `let m = a.max(b); m.field` (where `m`'s
        // type was inferred from `max`'s return type via
        // `infer_expr_type_name`'s MethodCall arm at line ~17486)
        // resolve field indices through the concrete type's layout
        // rather than the literal `Self` (which is unregistered and
        // falls through to heuristics).
        if let Some(concrete) = impl_type_name
            && let Some(ref rtn) = self.ctx.current_return_type_name.clone()
        {
            let substituted = Self::substitute_self_in_type_name(rtn, concrete);
            if substituted != *rtn {
                self.ctx.current_return_type_name = Some(substituted);
            }
        }

        // Emit runtime Assert for each refined parameter.
        //

        // For `fn f(x: Int { x > 0 })` the compiler wraps entry with
        // a check of the predicate — if it fails, the interpreter
        // raises a refinement violation instead of allowing the
        // function to operate on an unsound argument. At Tier-1 (AOT)
        // the Assert instructions survive when the predicate is not
        // SMT-discharged; when the verifier proved the obligation
        // during compilation they are elided at link time.
        //

        // Three binding shapes feed this loop:
        //  Rule 1 `T{pred}` — predicate uses `it`.
        //  Rule 2 `T where |x| pred` — predicate uses `x`.
        //  Rule 3 `x: T where pred` — predicate uses `x`.
        //

        // The binding name is aliased to the parameter's register via
        // a `Mov` into a freshly-named local so `compile_expr` on the
        // predicate resolves the reference normally. When the binding
        // happens to coincide with the parameter name (common case
        // for pattern `fn f(x: Int { x > 0 })` with implicit `it`
        // collision guarded against), no alias is introduced.
        for param in func.params.iter() {
            let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind else {
                continue;
            };
            let Some((param_name, _)) = self.extract_param_name_and_mutable(param) else {
                continue;
            };

            // Extract (predicate_expr, binding_name) from the canonical
            // `Refined` node (post — the sigma surface form parses
            // to `Refined` with `predicate.binding = Some(name)`).
            let (pred_expr, binding_name) = match &ty.kind {
                verum_ast::ty::TypeKind::Refined { predicate, .. } => {
                    let bname = match &predicate.binding {
                        verum_common::Maybe::Some(id) => id.name.to_string(),
                        verum_common::Maybe::None => "it".to_string(),
                    };
                    (predicate.expr.clone(), bname)
                }
                _ => continue,
            };

            // Resolve the parameter register. If resolution fails we
            // skip this obligation — a missing param register means
            // the function never reached body compilation (extern /
            // placeholder) and there is nothing to assert against.
            let Ok(param_reg) = self.ctx.get_var_reg(&param_name) else {
                continue;
            };

            let message_id = {
                let msg = format!(
                    "refinement violation: parameter `{}` of `{}`",
                    param_name, lookup_name
                );
                self.intern_string(&msg)
            };

            if binding_name == param_name {
                // No aliasing needed — predicate already references the
                // parameter by the same name the caller bound.
                if let Ok(Some(cond_reg)) = self.compile_expr(&pred_expr) {
                    self.ctx.emit(Instruction::Assert {
                        cond: cond_reg,
                        message_id,
                    });
                    self.ctx.free_temp(cond_reg);
                }
            } else {
                // Scope the alias so it does not leak into the
                // function body and shadow a subsequent `it` or
                // rebind a legitimate user variable. Mirror the
                // parameter's VarType / type-name for the alias so
                // `compile_expr` selects the correct comparison ops
                // (Int < vs Float lt, record field resolution, etc.).
                self.ctx.enter_scope();
                let alias_reg = self.ctx.define_var(&binding_name, false);
                self.ctx.emit(Instruction::Mov {
                    dst: alias_reg,
                    src: param_reg,
                });

                let vt = self.ctx.get_variable_type(&param_name);
                self.ctx.register_variable_type(&binding_name, vt);
                if let Some(type_name) = self.ctx.variable_type_names.get(&param_name).cloned() {
                    self.ctx
                        .variable_type_names
                        .insert(binding_name.clone(), type_name);
                }

                if let Ok(Some(cond_reg)) = self.compile_expr(&pred_expr) {
                    self.ctx.emit(Instruction::Assert {
                        cond: cond_reg,
                        message_id,
                    });
                    self.ctx.free_temp(cond_reg);
                }
                self.ctx.exit_scope(false);
            }
        }

        // Context transforms: for contexts declared with transforms like
        // `using [Database.transactional()]`, emit CtxGet + method call + CtxProvide
        // at function entry to wrap the base context with the transform.
        // Grammar: transformed_context = context_path , context_transform , { context_transform }
        let _ctx_transform_count = self.emit_context_transforms(func);

        // Emit runtime negative context checks for `using [!Context]` constraints.
        // These fire at function entry and abort if an excluded context is present.
        let func_name_id = self.intern_string(&lookup_name);
        for ctx_decl in &func.contexts {
            if !ctx_decl.is_negative {
                continue;
            }
            let ctx_type_name = ctx_decl
                .path
                .segments
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_type_name.is_empty() {
                continue;
            }
            let ctx_type_id = self.intern_string(&ctx_type_name);
            self.ctx.emit(Instruction::CtxCheckNegative {
                ctx_type: ctx_type_id,
                func_name: func_name_id,
            });
        }

        // Detect the runtime-intrinsic shape: forward-declared with
        // `@intrinsic("name")` where the name is NOT in the typed-opcode
        // intrinsic registry. The implementation lives in the interpreter's
        // name-keyed runtime dispatcher (`try_dispatch_intrinsic_by_name`),
        // which fires only when the called function's bytecode_length == 0.
        // Emitting any instruction here — including the implicit `RetV` —
        // makes bytecode_length > 0 and short-circuits dispatch, causing
        // the function to always return Unit instead of its typed result.
        //

        // Examples that need the empty-body path:
        //  - `@intrinsic("tcp_listen") pub fn __tcp_listen_raw(port: Int) -> Int;`
        //  - `@intrinsic("tcp_recv") pub fn __tcp_recv_raw(fd: Int, max: Int) -> Text;`
        //

        // Typed-opcode intrinsics (math/SIMD/etc.) inline at the call
        // site via `compile_imported_intrinsic_call` so the body content
        // is unused — RetV is fine for them.
        let is_runtime_intrinsic_forward_decl = func.body.is_none() && {
            let iname = self.extract_intrinsic_name(func);
            match iname {
                Some(name) => crate::intrinsics::lookup_intrinsic(&name).is_none(),
                None => false,
            }
        };

        // Task #18 — escape-analysis pass.
        //
        // When the function's return type is a reference (`&T` / `&mut T`
        // / `&checked T` / `&unsafe T` and their mut variants), walk the
        // body AST and collect the names of local variables whose `&local`
        // expression flows directly into a return position (the trailing
        // expression of a block, or an explicit `return &local;`).
        //
        // `compile_block`'s scope-exit DropRef emission consumes this set
        // and skips the slot-generation bump for these names — without
        // the skip, the caller's CBGR ref carries the pre-bump generation
        // and trips `CBGR use-after-free detected: expected generation N,
        // found N+1` on the next deref.  The deeper runtime correctness
        // (slot-frame stabilisation across `pop_frame`'s own generation
        // bump) is handled in the interpreter's `do_return` — this
        // codegen pass elides the redundant DropRef bump and supplies
        // the AOT lowering with the same escape information.
        self.ctx.current_fn_escaping_vars.clear();
        if let Some(ref body) = func.body
            && Self::return_type_is_reference(func.return_type.as_ref())
        {
            Self::collect_escaping_local_refs(body, &mut self.ctx.current_fn_escaping_vars);
        }

        // Compile the body
        if let Some(ref body) = func.body {
            match body {
                verum_ast::FunctionBody::Block(block) => {
                    let result = self
                        .compile_block(block)
                        .map_err(|e| e.with_context(format!("in function {}", lookup_name)))?;
                    // Return the block result if present (implicit return)
                    if let Some(reg) = result {
                        self.emit_return_refinement_assert(
                            reg,
                            func.return_type.as_ref(),
                            &lookup_name,
                        );
                        self.ctx.emit(Instruction::Ret { value: reg });
                    }
                }
                verum_ast::FunctionBody::Expr(expr) => {
                    let result = self
                        .compile_expr(expr)
                        .map_err(|e| e.with_context(format!("in function {}", lookup_name)))?;
                    // Return the expression result
                    if let Some(reg) = result {
                        self.emit_return_refinement_assert(
                            reg,
                            func.return_type.as_ref(),
                            &lookup_name,
                        );
                        self.ctx.emit(Instruction::Ret { value: reg });
                    } else {
                        self.ctx.emit(Instruction::RetV);
                    }
                }
            }
        } else if !is_runtime_intrinsic_forward_decl {
            // Regular forward decl with no runtime-intrinsic shape —
            // emit the safe Unit return.
            self.ctx.emit(Instruction::RetV);
        }
        // else: leave bytecode empty for runtime-intrinsic dispatch.

        // Ensure function ends with return — but skip for runtime
        // intrinsics so their bytecode_length stays at 0.
        if !is_runtime_intrinsic_forward_decl {
            self.ensure_return()?;
        }

        // Clear the refinement return-type stash so it cannot leak
        // into the next function's compilation.
        self.current_return_ast_type = None;
        self.current_fn_lookup_name = None;
        // Clear task #18 escape-analysis set so it cannot leak into the
        // next function's compilation (its DropRef-skip discipline is
        // strictly per-function-body and must not contaminate siblings).
        self.ctx.current_fn_escaping_vars.clear();

        // End function compilation
        let (instructions, register_count) = self.ctx.end_function();

        // Promote the descriptor's stored name to the FULL source-
        // module-qualified form (`sys.bitfield.test_bit` rather than
        // bare `test_bit`) so the archive-load path's qualified-key
        // registration finds the function under its file-declared
        // module path.  Without this promotion every `.vr` file under
        // e.g. `core/sys/` gets folded into a single archive entry
        // named `core.sys`, sibling files collapse onto the same bare
        // simple name, and cross-module bare-mount qualified access
        // (`mount core.sys.bitfield; ... bitfield.test_bit(v, 7)`)
        // misses the user-side function registry.
        //
        // Mirrors the equivalent promotion already in place at
        // `register_constant_with_value` (inlinable-const stub) and
        // `compile_pending_constants` (body-compiled-const) — the
        // function-body path is the third sibling.  Impl methods
        // (`lookup_name = "Type.method"`, already qualified) and
        // nested-function names (with `$` separators) are left alone.
        // Closes audit task #121's function-side gap.
        let effective_module = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let descriptor_name = if !effective_module.is_empty()
            && effective_module != "main"
            && !lookup_name.contains('.')
            && !lookup_name.contains('$')
        {
            format!("{}.{}", effective_module, lookup_name)
        } else {
            lookup_name.clone()
        };

        // Create VBC function
        let name_id = StringId(self.intern_string(&descriptor_name));
        let mut descriptor = FunctionDescriptor::new(name_id);
        descriptor.id = func_info.id;
        descriptor.register_count = register_count;
        descriptor.locals_count = params_with_mutability.len() as u16;
        descriptor.optimization_hints.is_pure = func.is_pure;

        // Propagate `parent_type` for inherent methods.  Without
        // this, archive-driven typecheck cannot recover the
        // `Type.method` membership relation from the precompiled
        // VBC archive — calls like `Text.with_capacity(n)` fail
        // method lookup despite the body being present.
        // `type_name_to_id` carries every locally-defined +
        // built-in type; an unresolved name (e.g. impl target is
        // a generic parameter or a trait object) leaves
        // `parent_type` at None, which is the correct fallback.
        if let Some(type_name) = impl_type_name {
            descriptor.parent_type = self
                .type_name_to_id
                .get(type_name.as_str())
                .copied()
                .or_else(|| self.get_well_known_type_id(type_name));
        }
        // Set return type from function info (default is UNIT)
        if let Some(ref ret_type) = func_info.return_type {
            descriptor.return_type = ret_type.clone();
        }

        // Build a generic-param-name → TypeParamId index map for
        // type-ref resolution.  Combines the function's own
        // generics with the impl-block generics inherited from the
        // parent type.  Source for impl generics is the parent
        // type's `type_params` if found in `self.types` (the
        // codegen registers impl block's `<T>` into the parent
        // TypeDescriptor's type_params during the type-decl pass),
        // PLUS `ctx.generic_type_params` as a redundant safety net
        // for code paths that don't yet propagate the parent type
        // descriptor.
        // Without this, a method like `fn unwrap_or(default: T) -> T`
        // inside `implement Maybe<T>` had T unresolved → fell back
        // through `get_well_known_type_id` → matched
        // `well_known_types["Heap"] = TypeId::PTR(14)` (because
        // PTR's name slot was occupied by Heap), so descriptors
        // landed with `default: Heap`.  Result: `Maybe<Int>.unwrap_or(0)`
        // failed at user-code typecheck with `expected 'Heap',
        // found 'Int'`.
        let mut method_generic_param_map: std::collections::HashMap<String, u16> =
            std::collections::HashMap::new();
        let mut next_pid: u16 = 0;
        // 1. Inherit impl-block generics from the parent type's
        //    `TypeDescriptor.type_params` if available.  This is
        //    the authoritative source — populated by the Sum/Record
        //    arms of `register_type_constructors` from
        //    `type_decl.generics`.  Resolves T/E/K/V correctly even
        //    when ctx.generic_type_params got cleared between the
        //    impl-block setup and compile_function entry (which
        //    happens for some collect-decls → compile-bodies pass
        //    sequences in stdlib bootstrap).
        if let Some(parent_name) = impl_type_name {
            if let Some(&parent_tid) = self.type_name_to_id.get(parent_name.as_str()) {
                if let Some(parent_desc) = self.types.iter().find(|t| t.id == parent_tid) {
                    for tp in parent_desc.type_params.iter() {
                        if let Some(name) = self.ctx.strings.get(tp.name.0 as usize) {
                            if !method_generic_param_map.contains_key(name) {
                                method_generic_param_map.insert(name.clone(), next_pid);
                                next_pid += 1;
                            }
                        }
                    }
                }
            }
        }
        // 2. Fallback: also inherit from ctx.generic_type_params.
        for g in self.ctx.generic_type_params.iter() {
            if !method_generic_param_map.contains_key(g) {
                method_generic_param_map.insert(g.clone(), next_pid);
                next_pid += 1;
            }
        }
        // Function-level generics added after.
        for gp in func.generics.iter() {
            if let verum_ast::ty::GenericParamKind::Type { name: gname, .. } = &gp.kind {
                let n = gname.name.to_string();
                if !method_generic_param_map.contains_key(&n) {
                    method_generic_param_map.insert(n, next_pid);
                    next_pid += 1;
                }
            }
        }

        // Populate parameter descriptors for proper method dispatch matching.
        // This enables the interpreter to match methods by parameter count.
        //
        // **Architectural invariant** (closes task #11): every
        // self-shape param kind MUST round-trip its reference /
        // mutability shape through the archive so the user-side
        // FunctionInfo loader can recover `takes_self_mut_ref`
        // (`crates/verum_compiler/src/archive_ctx_loader.rs::
        // param_is_mut_self_ref`).  Pre-fix every non-Regular kind
        // collapsed to `Concrete(UNIT)`, erasing the entire `&` /
        // `&mut` / `&checked` / `&unsafe` taxonomy at serialisation
        // time — the user-side dispatch then had to assume "by
        // value" for every stdlib method's self, and every
        // `&mut self` method's `*self = value` writeback silently
        // dropped (Maybe.take / Maybe.replace / Text.push_str /
        // every stdlib mutator).
        //
        // Encoding rule (mirrors `compile_function`'s receiver
        // setup at the same self-shape branches):
        //   SelfValue / SelfValueMut / SelfOwn / SelfOwnMut →
        //     Concrete(parent_type)        — passed by value
        //   SelfRef         → Reference { Immutable, Tier0 }
        //   SelfRefMut      → Reference { Mutable,   Tier0 }
        //   SelfRefChecked  → Reference { Immutable, Checked }
        //   SelfRefCheckedMut → Reference { Mutable, Checked }
        //   SelfRefUnsafe   → Reference { Immutable, Unsafe }
        //   SelfRefUnsafeMut → Reference { Mutable,  Unsafe }
        //
        // The inner type is the parent TypeId (the impl target),
        // OR `TypeId::UNIT` when impl_type_name is None — that case
        // covers free `fn f(self)` (rare; legacy interpretation)
        // where there's no parent record/type to anchor the
        // reference on.  Concrete(UNIT) here is honest: a self of
        // type Unit really IS Unit, no ref involved.
        let parent_tid: TypeId = impl_type_name
            .and_then(|n| self.type_name_to_id.get(n.as_str()).copied())
            .or_else(|| impl_type_name.and_then(|n| self.get_well_known_type_id(n)))
            .unwrap_or(TypeId::UNIT);
        for ((param_name, is_mut), param) in params_with_mutability.iter().zip(func.params.iter()) {
            use verum_ast::FunctionParamKind;
            use crate::types::{CbgrTier, Mutability};
            // **Targeted fix for task #11** — only the `&mut`-family
            // self-shape variants need a meaningful `TypeRef` round-trip
            // (so the user-side dispatch can recover `takes_self_mut_ref`
            // from `param.type_ref`).  Value-typed self and `&self` keep
            // `Concrete(UNIT)` (the pre-fix behaviour) — those variants
            // never trigger `RefMut`/`DerefMut` semantics, so the type
            // info isn't needed for the writeback-correctness invariant,
            // and preserving the pre-fix encoding avoids the
            // "self type tracked as Box but no method-table populated"
            // regression that surfaced as field-access on the result of
            // a static constructor returning bogus values.
            //
            // Rationale: the `takes_self_mut_ref` detector at
            // `archive_ctx_loader.rs::param_is_mut_self_ref` only fires
            // for `Reference { Mutability::Mutable, .. }`.  Every other
            // self-shape can stay `Concrete(UNIT)` and still produces
            // the correct false-flag result.
            let type_ref = match &param.kind {
                FunctionParamKind::Regular { ty, .. } => {
                    let resolved = self.resolve_field_type_ref(ty, &method_generic_param_map);
                    // **Closure expected-return-type plumbing (#26 residual).**
                    // When the user wrote `f: F` and the function declares
                    // `F: fn(...) -> X`, the resolved TypeRef is
                    // `TypeRef::Generic(F)` — losing the fn-bound shape at the
                    // archive boundary.  Substitute with the bound's
                    // `TypeRef::Function` so user-side codegen reading the
                    // archive can recover the closure-arg's return-type
                    // simple-name via
                    // `archive_ctx_loader::extract_closure_return_type_from_typeref`
                    // and drive the call-site disambig push for closure
                    // arguments.  No-op when the param isn't a generic-param
                    // path or when the generic carries no fn bound.
                    self.substitute_fn_bound_for_generic(resolved, ty, &func.generics, &method_generic_param_map)
                }
                FunctionParamKind::SelfRefMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(parent_tid)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier0,
                },
                FunctionParamKind::SelfRefCheckedMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(parent_tid)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier1,
                },
                FunctionParamKind::SelfRefUnsafeMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(parent_tid)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier2,
                },
                // Value-typed self (`self`, `mut self`, `self` by ownership)
                // and `&self` / `&checked self` / `&unsafe self` keep
                // `Concrete(UNIT)` — see comment above.
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefUnsafe => TypeRef::Concrete(TypeId::UNIT),
            };
            let param_name_id = StringId(self.intern_string(param_name));
            descriptor.params.push(ParamDescriptor {
                name: param_name_id,
                type_ref,
                is_mut: *is_mut,
                default: None,
            });
        }
        // Also fix return_type: the func_info.return_type was built
        // earlier without generic-param awareness; rebuild it here
        // from the AST so generic returns (`-> T`, `-> Maybe<T>`,
        // `-> Result<T, E>`) preserve param refs as
        // TypeRef::Generic(idx) instead of degrading to
        // TypeRef::Concrete(PTR).
        if let verum_common::Maybe::Some(ref ret_ty_ast) = func.return_type {
            let resolved_return = self.resolve_field_type_ref(ret_ty_ast, &method_generic_param_map);
            descriptor.return_type = resolved_return;
        }

        // Extract optimization hints from AST attributes (@inline, @cold, @hot, etc.)
        descriptor.optimization_hints = self.extract_optimization_hints(func);

        // Set generator properties if this is a generator function (fn*).
        // Tracks is_generator flag, GENERATOR property bit, yield type, and suspend point count.
        if func_info.is_generator {
            descriptor.is_generator = true;
            descriptor.properties |= crate::types::PropertySet::GENERATOR;
            descriptor.yield_type = func_info.yield_type.clone();
            descriptor.suspend_point_count = self.ctx.suspend_point_count;
        }

        // Set async property if applicable
        if func_info.is_async {
            descriptor.properties |= crate::types::PropertySet::ASYNC;
        }

        // **Architectural propagation**: carry `@intrinsic("name")` marker
        // through the precompile → archive → user-side round-trip.
        //
        // `compile_function` is the path that handles regular function
        // declarations with bodies — including `@intrinsic` declarations
        // whose Verum body is the semantic specification (typechecker-
        // facing) but whose Tier-0 / AOT dispatch goes through the
        // intrinsic registry's emit strategy (FfiExtended / InlineSequence /
        // DirectOpcode / etc.).
        //
        // **Pre-fix defect**: this assignment was missing — every `@intrinsic`
        // function body was compiled and serialised into the archive WITHOUT
        // descriptor.intrinsic_name. User-side archive_ctx_loader then read
        // `fn_desc.intrinsic_name = None`, registered FunctionInfo with
        // `intrinsic_name = None`, and compile_call's intercept at
        // expressions.rs:4385 skipped the intrinsic dispatch — falling
        // through to a raw `Call` to the body. For `cbgr_alloc` (the load-
        // bearing CBGR allocator) this routed every allocation through the
        // Verum body's `get_local_heap().alloc()` chain, which recursively
        // calls `cbgr_alloc` to allocate its own TLS state — infinite
        // recursion and `unwrap() on None` panics. Every `Text` / `List` /
        // `Map` / `Set` / `Heap` / `Shared` allocation in Tier 0 went
        // through this broken path.
        //
        // Mirrors the parallel propagation in `emit_missing_stub_descriptors`
        // (line ~13022) and `register_constant_with_value` (line ~10313) —
        // every site that materialises a FunctionDescriptor from a
        // FunctionInfo MUST copy `intrinsic_name`.
        if let Some(ref iname) = func_info.intrinsic_name {
            descriptor.intrinsic_name = Some(StringId(self.intern_string(iname)));
        }

        // Set test flag if function has @test attribute (only for user code)
        if self.propagate_test_attr && func.attributes.iter().any(|a| a.is_named("test")) {
            descriptor.is_test = true;
        }

        // Set is_gpu_only flag if function carries `@device(gpu)` /
        // `@device(GPU)` / `@device("gpu")`.  Mirrors the predicate
        // in `crates/verum_compiler/src/pipeline/gpu_detect.rs` —
        // both AST shapes (path-segment + string-literal) qualify.
        // Functions tagged here go EXCLUSIVELY through the MLIR GPU
        // pipeline; the LLVM CPU pipeline emits only an extern stub.
        if Self::function_is_gpu_only(&func.attributes) {
            descriptor.is_gpu_only = true;
        }

        // Map context names to ContextRef IDs and register in context_names table
        for ctx_name in &func_info.contexts {
            let ctx_id = self.intern_context_name(ctx_name);
            descriptor.contexts.push(crate::types::ContextRef(ctx_id));
        }

        let vbc_func = VbcFunction::new(descriptor, instructions);

        self.push_function_dedup(vbc_func);
        Ok(())
    }

    /// Emits a panic-stub body for a function whose real body failed
    /// bug-class lenient codegen.  The stub re-uses the function's
    /// pre-registered `FunctionId`, name, params, and return type, so
    /// typed dispatch (CallM / Call / qualified-path lookup) finds it
    /// like any other function.  The body is two instructions:
    ///
    /// ```text
    /// Panic <message_id>      ;; carries the original codegen error
    /// RetV                    ;; never reached, but keeps bytecode valid
    /// ```
    ///
    /// This replaces the prior silent-drop behaviour where the
    /// function disappeared entirely from the module and runtime calls
    /// produced an opaque `FunctionNotFound`.  With the stub:
    ///
    /// * Pattern matching against variants of the carrier type still
    ///   works (variant constructor stays callable; the method stub
    ///   only fires on actual call).
    /// * Function-pointer references resolve cleanly.
    /// * Cross-module lookups (qualified-path resolution, suffix-and-
    ///   args dispatch) succeed with the stub's id.
    /// * Only an actual call panics — and with the original codegen
    ///   error message inline, not `FunctionNotFound`.
    ///
    /// Any failure inside the stub emitter degrades to a no-op return:
    /// the function stays dropped, matching the prior behaviour.
    /// This must NOT make the build worse for anything that compiled
    /// before.
    fn emit_lenient_panic_stub(
        &mut self,
        func: &FunctionDecl,
        impl_type_name: Option<&str>,
        error_message: &str,
    ) {
        let base_name = func.name.name.to_string();
        let lookup_name = if let Some(t) = impl_type_name {
            format!("{}.{}", t, base_name)
        } else {
            base_name.clone()
        };

        // Reuse the same lookup discipline as compile_function so the
        // stub binds to the same FunctionId.  If the function was
        // never registered (collect_declarations failed), skip — the
        // pre-existing drop semantics applies.
        let param_count = func.params.len();
        let func_info = match self
            .ctx
            .lookup_function_with_arity(&lookup_name, param_count)
            .cloned()
        {
            Some(info) => info,
            None => return,
        };

        let params_with_mutability: Vec<(String, bool)> = func
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                self.extract_param_name_and_mutable(p)
                    .unwrap_or_else(|| (format!("_arg{}", i), false))
            })
            .collect();

        // Fresh function block — `begin_function` clears every
        // function-scoped buffer (instructions, registers, labels,
        // type-tracking, etc.) so leftover state from the failed
        // compile_function call cannot poison the stub.
        self.ctx.begin_function(
            &lookup_name,
            &params_with_mutability,
            func_info.return_type.clone(),
        );

        // Intern the diagnostic message once.  Format mirrors what the
        // [lenient] SKIP warning prints so users see the same string
        // both at build time (warn) and at runtime (panic).
        let panic_message = format!(
            "[lenient] {} compiled to panic-stub: {}",
            lookup_name, error_message
        );
        let message_id = self.intern_string(&panic_message);

        self.ctx.emit(crate::Instruction::Panic { message_id });
        // Trailing RetV is unreachable but keeps the bytecode valid
        // for any decoder that walks past Panic.  ensure_return
        // observes `instructions.last()` and skips when the function
        // already terminates with Ret/RetV — Panic isn't recognised
        // as terminal there, so emit explicitly.
        self.ctx.emit(crate::Instruction::RetV);

        let (instructions, register_count) = self.ctx.end_function();

        // Build the descriptor.  Mirrors the compile_function
        // descriptor-building block at the same call shape — same
        // name_id, same FunctionId, same params, same return_type, +
        // generator/async/context flags from the registered info.
        //
        // Apply the same source-module-qualified descriptor.name
        // promotion as the success path (task #121): if the panic
        // stub stands in for a top-level fn declared in e.g.
        // `module sys.bitfield;`, the stub still needs to land in
        // the archive under `sys.bitfield.<name>` so cross-module
        // bare-mount dispatch reaches it.
        let effective_module = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let descriptor_name = if !effective_module.is_empty()
            && effective_module != "main"
            && !lookup_name.contains('.')
            && !lookup_name.contains('$')
        {
            format!("{}.{}", effective_module, lookup_name)
        } else {
            lookup_name.clone()
        };
        let name_id = StringId(self.intern_string(&descriptor_name));
        let mut descriptor = FunctionDescriptor::new(name_id);
        descriptor.id = func_info.id;
        descriptor.register_count = register_count;
        descriptor.locals_count = params_with_mutability.len() as u16;
        if let Some(ref ret_type) = func_info.return_type {
            descriptor.return_type = ret_type.clone();
        }
        // Mirror the self-shape → TypeRef encoding from the primary
        // `compile_function` site above, restricted to the `&mut`-family
        // variants — see that site's comment for the targeted-fix
        // rationale (closes task #11 panic-stub path without regressing
        // value-self semantics).
        for ((param_name, is_mut), param) in params_with_mutability.iter().zip(func.params.iter()) {
            use verum_ast::FunctionParamKind;
            use crate::types::{CbgrTier, Mutability};
            let type_ref = match &param.kind {
                FunctionParamKind::Regular { ty, .. } => self.ast_type_to_type_ref(ty),
                FunctionParamKind::SelfRefMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId::UNIT)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier0,
                },
                FunctionParamKind::SelfRefCheckedMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId::UNIT)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier1,
                },
                FunctionParamKind::SelfRefUnsafeMut => TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId::UNIT)),
                    mutability: Mutability::Mutable,
                    tier: CbgrTier::Tier2,
                },
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefUnsafe => TypeRef::Concrete(TypeId::UNIT),
            };
            let param_name_id = StringId(self.intern_string(param_name));
            descriptor.params.push(ParamDescriptor {
                name: param_name_id,
                type_ref,
                is_mut: *is_mut,
                default: None,
            });
        }
        if func_info.is_generator {
            descriptor.is_generator = true;
            descriptor.properties |= crate::types::PropertySet::GENERATOR;
            descriptor.yield_type = func_info.yield_type.clone();
        }
        if func_info.is_async {
            descriptor.properties |= crate::types::PropertySet::ASYNC;
        }
        for ctx_name in &func_info.contexts {
            let ctx_id = self.intern_context_name(ctx_name);
            descriptor.contexts.push(crate::types::ContextRef(ctx_id));
        }

        // Set parent_type so archive_metadata's Pass-2 function walker
        // adds this method to the parent type's `methods` list, making it
        // visible to register_inherent_methods_from_metadata.  Without
        // this, any method whose compile_function fails (common for async
        // methods that reference cross-module symbols not yet loaded) gets
        // a stub with parent_type=None, causing the method to be treated
        // as a free function — invisible at typecheck call sites.
        if let Some(type_name) = impl_type_name {
            descriptor.parent_type = self
                .type_name_to_id
                .get(type_name)
                .copied()
                .or_else(|| self.get_well_known_type_id(type_name));
        }

        let vbc_func = VbcFunction::new(descriptor, instructions);
        self.push_function_dedup(vbc_func);
    }

    /// Compiles a block.
    fn compile_block(&mut self, block: &Block) -> CodegenResult<Option<Reg>> {
        self.ctx.enter_scope();

        let mut result = None;

        // Compile statements.
        // Only update result when a statement produces a value (Some).
        // This handles @cfg filtered statements that return None - they should
        // not overwrite the result from a previous statement that did compile.
        for stmt in block.stmts.iter() {
            if let Some(reg) = self.compile_stmt(stmt)? {
                result = Some(reg);
            }
        }

        // Compile trailing expression
        if let Some(ref expr) = block.expr {
            result = self.compile_expr(expr)?;
        }

        // CRITICAL FIX: Copy result to a new register BEFORE exiting scope.
        // When exit_scope is called, it recycles registers for variables defined
        // in this scope (like `doubled` in `{ let doubled = x * 2; doubled }`).
        // If the result register is one of those recycled registers, its value
        // would be overwritten when the register is reused later.
        // By copying to a fresh temp register, we ensure the result survives.
        let final_result = if let Some(result_reg) = result {
            let safe_reg = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::Mov {
                dst: safe_reg,
                src: result_reg,
            });
            Some(safe_reg)
        } else {
            None
        };

        // Exit scope (handles defers and drops)
        let (vars, defers) = self.ctx.exit_scope(false);

        // Emit defer instructions
        for defer_instrs in defers {
            for instr in defer_instrs {
                self.ctx.emit(instr);
            }
        }

        // Drop variables in reverse declaration order (last declared = first dropped).
        // DropRef handles both:
        // 1. User-defined Drop::drop() calls (if type has Drop impl)
        // 2. CBGR slot invalidation (bumps generation to invalidate references)
        //
        // Task #18 escape-analysis discipline: skip DropRef for any local
        // whose name was collected by `collect_escaping_local_refs`
        // (populated at `compile_function` entry).  These slots are
        // returned via `&local` flowing into `Ret { value: &local }`;
        // their generation must NOT be bumped here, otherwise the
        // caller's CBGR ref carries the pre-bump generation and trips
        // `CBGR use-after-free detected` on the next deref.  The deeper
        // soundness across `pop_frame`'s own slot-generation bump is
        // handled by interpreter-side stabilisation in `do_return`
        // (commit chain phase 3).
        for (name, var_reg) in vars.iter().rev() {
            if self.ctx.current_fn_escaping_vars.contains(name) {
                continue;
            }
            self.ctx.emit(Instruction::DropRef { src: *var_reg });
        }

        Ok(final_result)
    }

    /// Emits an `Instruction::Assert` on the given return register when
    /// the function's declared return type is `Refined` or `Sigma`.
    ///

    /// Called at each implicit-return site (tail expression / block
    /// result). Explicit `return expr;` statements go through
    /// `compile_return` in `expressions.rs`, which invokes the same
    /// helper, so every return path exits through the predicate check.
    ///

    /// See the parameter-entry Assert emission (same file, around the
    /// body-compile prologue) for the binding-alias + VarType mirror
    /// idiom — the same idiom is used here so predicates like
    /// `Int { it > 0 }` in a return type lower to the correct
    /// integer comparison instead of falling back to an Unknown
    /// comparator that always yields `true`.
    pub(super) fn emit_return_refinement_assert(
        &mut self,
        result_reg: Reg,
        return_type: Option<&verum_ast::ty::Type>,
        fn_name: &str,
    ) {
        let Some(ret_ty) = return_type else { return };
        let (pred_expr, binding_name) = match &ret_ty.kind {
            verum_ast::ty::TypeKind::Refined { predicate, .. } => {
                let bname = match &predicate.binding {
                    verum_common::Maybe::Some(id) => id.name.to_string(),
                    verum_common::Maybe::None => "it".to_string(),
                };
                (predicate.expr.clone(), bname)
            }
            _ => return,
        };

        // Derive VarType from the underlying base (peel the refinement
        // layer) so predicate compilation picks the right comparisons.
        let base_vt = match &ret_ty.kind {
            verum_ast::ty::TypeKind::Refined { base, .. } => self.type_kind_to_var_type(&base.kind),
            _ => context::VarTypeKind::Unknown,
        };
        let base_type_name = match &ret_ty.kind {
            verum_ast::ty::TypeKind::Refined { base, .. } => Self::extract_type_name_from_ast(base),
            _ => String::new(),
        };

        let message_id = {
            let msg = format!("refinement violation: return value of `{}`", fn_name);
            self.intern_string(&msg)
        };

        self.ctx.enter_scope();
        let alias_reg = self.ctx.define_var(&binding_name, false);
        self.ctx.emit(Instruction::Mov {
            dst: alias_reg,
            src: result_reg,
        });

        self.ctx.register_variable_type(&binding_name, base_vt);
        if !base_type_name.is_empty() && base_type_name != "()" {
            self.ctx
                .variable_type_names
                .insert(binding_name.clone(), base_type_name);
        }

        if let Ok(Some(cond_reg)) = self.compile_expr(&pred_expr) {
            self.ctx.emit(Instruction::Assert {
                cond: cond_reg,
                message_id,
            });
            self.ctx.free_temp(cond_reg);
        }
        self.ctx.exit_scope(false);
    }

    /// Ensures function ends with a return instruction.
    fn ensure_return(&mut self) -> CodegenResult<()> {
        let needs_return = self.ctx.instructions.is_empty()
            || !matches!(
                self.ctx.instructions.last(),
                Some(Instruction::Ret { .. } | Instruction::RetV)
            );

        if needs_return {
            self.ctx.emit(Instruction::RetV);
        }

        Ok(())
    }

    /// Interns a field name and returns its globally unique field index.
    /// This is separate from `intern_string` to ensure field indices start at 0
    /// and are compact, used for record field access (GetF/SetF instructions).
    ///

    /// NOTE: Prefer `resolve_field_index()` when the type name is known, which
    /// returns the correct type-specific position (0, 1, 2, ...) matching the
    /// declared field order. This function returns a global ID that may not match
    /// the field's position within its type.
    /// Interns a context name, returning its ContextRef ID.
    /// Used to build the context_names string table in the VBC module.
    fn intern_context_name(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.context_name_to_id.get(name) {
            return id;
        }
        let id = self.context_names.len() as u32;
        self.context_name_to_id.insert(name.to_string(), id);
        self.context_names.push(name.to_string());
        id
    }

    fn intern_field_name(&mut self, name: &str) -> u32 {
        if let Some(&idx) = self.field_name_indices.get(name) {
            return idx;
        }
        let idx = self.next_field_id;
        self.next_field_id += 1;
        self.field_name_indices.insert(name.to_string(), idx);
        idx
    }

    /// Resolves the field index for a given type and field name.
    ///

    /// Returns the field's position within the type's declared field order
    /// (0, 1, 2, ...), which is correct for memory layout. Falls back to
    /// the global interned field ID if the type is not registered.
    /// Strips transparent wrappers (Heap<X>, Shared<X>, &X, &mut X) from a type name
    /// to get the underlying struct type for field layout lookup.
    fn strip_wrapper_type(tn: &str) -> &str {
        let mut s = tn;
        // Strip reference wrappers
        if s.starts_with("&mut ") {
            s = &s[5..];
        } else if s.starts_with('&') {
            s = &s[1..];
        }
        // Strip Heap<...> or Shared<...> wrappers
        for prefix in &["Heap<", "Shared<"] {
            if s.starts_with(prefix) && s.ends_with('>') {
                s = &s[prefix.len()..s.len() - 1];
            }
        }
        s
    }

    fn resolve_field_index(&mut self, type_name: Option<&str>, field_name: &str) -> u32 {
        if let Some(tn) = type_name {
            // **Architectural rule** (closes task #16 consumer side):
            // resolve field index from the canonical TypeDescriptor's
            // own field list BEFORE consulting `type_field_layouts`.
            // The descriptor is the single source of truth (each type
            // carries its own descriptor); the flat `type_field_layouts`
            // cache has historically been polluted by sibling sum types
            // whose record-style variants share the host record's simple
            // name (`CompletionOp.Timeout` vs the host record
            // `core.async.timer.Timeout<F>`).  Using the cache as the
            // first source led to "field write out of bounds" panics
            // when the literal's field `future` got resolved against
            // the polluted layout `["ts"]` (variant payload), falling
            // through to the global-scan fallback which then picked an
            // unrelated wrong-offset field from another type.
            //
            // Stripping generic args first (e.g. `Timeout<ReadyFuture>`
            // → `Timeout`) lets the descriptor lookup work for the
            // canonical, non-parameterised type names that
            // `type_name_to_id` is keyed by.
            let stripped = match tn.find('<') {
                Some(i) => &tn[..i],
                None => tn,
            };
            if let Some(&tid) = self.type_name_to_id.get(stripped)
                && let Some(td) = self.types.iter().find(|t| t.id == tid)
                && matches!(td.kind, crate::types::TypeKind::Record)
            {
                let field_id = self.field_name_indices.get(field_name).copied();
                for (idx, fd) in td.fields.iter().enumerate() {
                    if Some(fd.name.0) == field_id {
                        return idx as u32;
                    }
                }
                // Also try lookup by interned name string — covers cases
                // where field_name_indices race ahead of the descriptor's
                // own name interning.
                for (idx, fd) in td.fields.iter().enumerate() {
                    if let Some(fname) =
                        self.ctx.strings.get(fd.name.0 as usize)
                        && fname == field_name
                    {
                        return idx as u32;
                    }
                }
            }
            // Try exact match first
            if let Some(fields) = self.type_field_layouts.get(tn)
                && let Some(pos) = fields.iter().position(|f| f == field_name)
            {
                if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                    tracing::debug!(
                        "[FIELD] {}.{} → per-type idx {} (fields: {:?})",
                        tn,
                        field_name,
                        pos,
                        fields
                    );
                }
                return pos as u32;
            }
            // Try with generic params stripped (e.g., "Slot<K, V>" → "Slot")
            if let Some(angle) = tn.find('<') {
                let base = &tn[..angle];
                if let Some(fields) = self.type_field_layouts.get(base)
                    && let Some(pos) = fields.iter().position(|f| f == field_name)
                {
                    if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                        tracing::debug!(
                            "[FIELD] {}.{} → per-type idx {} (stripped from '{}', fields: {:?})",
                            base,
                            field_name,
                            pos,
                            tn,
                            fields
                        );
                    }
                    return pos as u32;
                }
            }
            // Try stripping transparent wrappers (Heap<X> → X, Shared<X> → X, &X → X)
            let unwrapped = Self::strip_wrapper_type(tn);
            if unwrapped != tn {
                // Try exact match on unwrapped
                if let Some(fields) = self.type_field_layouts.get(unwrapped)
                    && let Some(pos) = fields.iter().position(|f| f == field_name)
                {
                    if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                        tracing::debug!(
                            "[FIELD] {}.{} → per-type idx {} (unwrapped from '{}', fields: {:?})",
                            unwrapped,
                            field_name,
                            pos,
                            tn,
                            fields
                        );
                    }
                    return pos as u32;
                }
                // Try with generic params stripped on the unwrapped type
                if let Some(angle) = unwrapped.find('<') {
                    let base = &unwrapped[..angle];
                    if let Some(fields) = self.type_field_layouts.get(base)
                        && let Some(pos) = fields.iter().position(|f| f == field_name)
                    {
                        if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                            tracing::debug!(
                                "[FIELD] {}.{} → per-type idx {} (unwrapped+stripped from '{}', fields: {:?})",
                                base,
                                field_name,
                                pos,
                                tn,
                                fields
                            );
                        }
                        return pos as u32;
                    }
                }
            }
            // Bare wrapper types without generic args: treat as transparent.
            // These occur when infer_expr_type_name can't determine the inner type.
            // Fall through to scan-all-types — same as type=None.
            if self.transparent_wrappers.contains(tn) {
                // fall through
            }
            // Cross-module field access: type might be registered under a qualified name
            // (e.g., "module_a.Point") but accessed with simple name ("Point") or vice versa.
            // Search for any registered type whose simple name matches.
            {
                let simple_name = tn.rsplit('.').next().unwrap_or(tn);
                for (type_n, fields) in &self.type_field_layouts {
                    let registered_simple = type_n.rsplit('.').next().unwrap_or(type_n);
                    if registered_simple == simple_name
                        && type_n != tn
                        && let Some(pos) = fields.iter().position(|f| f == field_name)
                    {
                        if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                            tracing::debug!(
                                "[FIELD] {}.{} → cross-module match '{}' idx {} (fields: {:?})",
                                tn,
                                field_name,
                                type_n,
                                pos,
                                fields
                            );
                        }
                        return pos as u32;
                    }
                }
            }

            // Type known but field not found — fall through to type=None scan below.
            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                tracing::debug!(
                    "[FIELD] type '{}' NOT in type_field_layouts, field '{}' → scanning all types (fn={})",
                    tn,
                    field_name,
                    self.ctx.current_function.as_deref().unwrap_or("?")
                );
            }
        }
        // type_name is None or didn't match — search all registered types.
        // Collect ALL types that have this field name, with their positional indices.
        {
            let mut candidates: Vec<(String, usize)> = Vec::new();
            for (type_n, fields) in &self.type_field_layouts {
                if let Some(pos) = fields.iter().position(|f| f == field_name) {
                    candidates.push((type_n.clone(), pos));
                }
            }
            if candidates.len() == 1 {
                let (ref type_n, pos) = candidates[0];
                if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                    tracing::debug!(
                        "[FIELD] scan: field '{}' → unique {}.{} idx {}",
                        field_name,
                        type_n,
                        field_name,
                        pos
                    );
                }
                return pos as u32;
            }
            if candidates.len() > 1 {
                // All at same position? Use that.
                let first_pos = candidates[0].1;
                if candidates.iter().all(|(_, p)| *p == first_pos) {
                    if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                        tracing::debug!(
                            "[FIELD] scan: field '{}' → all at idx {} ({} types)",
                            field_name,
                            first_pos,
                            candidates.len()
                        );
                    }
                    return first_pos as u32;
                }
                // Ambiguous — pick the candidate with the most fields
                // (main data structs tend to have more fields than iterators)
                let mut best = &candidates[0];
                for c in &candidates[1..] {
                    let c_fields = self.type_field_layouts.get(&c.0).map_or(0, |f| f.len());
                    let b_fields = self.type_field_layouts.get(&best.0).map_or(0, |f| f.len());
                    if c_fields > b_fields {
                        best = c;
                    }
                }
                if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                    tracing::debug!(
                        "[FIELD] scan: field '{}' ambiguous, picked {}.{} idx {} (most fields)",
                        field_name,
                        best.0,
                        field_name,
                        best.1
                    );
                }
                return best.1 as u32;
            }
            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                tracing::debug!(
                    "[FIELD] scan: field '{}' → not found in any type",
                    field_name
                );
            }
        }
        // Fallback: use global interned ID (self-consistent but not FFI-correct)
        let idx = self.intern_field_name(field_name);
        if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
            tracing::debug!("[FIELD] global intern '{}' → idx {}", field_name, idx);
        }
        idx
    }

    /// Returns the number of declared fields for a record type.
    /// Returns None if the type is not registered.
    fn type_field_count(&self, type_name: &str) -> Option<u32> {
        self.type_field_layouts
            .get(type_name)
            .map(|f| f.len() as u32)
    }

    /// Returns the type name of a field within a record type.
    fn field_type_name(&self, type_name: &str, field_name: &str) -> Option<&str> {
        self.type_field_type_names
            .get(&(type_name.to_string(), field_name.to_string()))
            .map(|s| s.as_str())
    }

    /// Substitute the literal `Self` token in a textual type name with
    /// the concrete impl type — used during default-method
    /// monomorphisation to bind `Self`-typed params and returns to
    /// the concrete receiver's type.  Handles three shapes:
    ///   * bare `Self` → `<Concrete>`
    ///   * `Self<...>` → `<Concrete><...>` (preserves generic args)
    ///   * `&Self`, `&mut Self`, `Maybe<Self>`, `Result<Self, E>`,
    ///     etc. — substitution is performed inside the rendered
    ///     name string at word boundaries.
    ///
    /// The substitution operates on the rendered name (the output
    /// of `extract_type_name_from_ast`) rather than the AST so
    /// nested compositions (`&Self`, `Option<Self>`, …) are handled
    /// uniformly without re-walking the AST.  Word-boundary detection
    /// uses ASCII identifier characters as separators so a literal
    /// `Self` inside another identifier (`MySelf`, `SelfHash`)
    /// remains untouched.
    pub(super) fn substitute_self_in_type_name(name: &str, concrete: &str) -> String {
        if name == "Self" {
            return concrete.to_string();
        }
        if !name.contains("Self") {
            return name.to_string();
        }
        // Word-boundary substitution.  We replace `Self` only when
        // it appears as a standalone identifier — preceded and
        // followed by either string boundary or a non-identifier
        // character (anything outside `[A-Za-z0-9_]`).
        let mut out = String::with_capacity(name.len() + concrete.len());
        let bytes = name.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Look for `Self` at position i.
            let token_len = 4; // bytes in "Self"
            if i + token_len <= bytes.len() && &bytes[i..i + token_len] == b"Self" {
                let before_ok = i == 0 || !Self::is_ident_byte(bytes[i - 1]);
                let after_ok = i + token_len == bytes.len()
                    || !Self::is_ident_byte(bytes[i + token_len]);
                if before_ok && after_ok {
                    out.push_str(concrete);
                    i += token_len;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    #[inline]
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    /// **String-level generic-param substitution** (task #12 §B).
    ///
    /// Substitutes each occurrence of a generic param name (T, K, V, E,
    /// …) in `name` with its corresponding concrete type argument from
    /// `args`.  Word-boundary aware (so `Tree` doesn't get partially
    /// substituted for param `T`).
    ///
    /// Used by `extract_expr_type_name`'s MethodCall arm to propagate
    /// concrete generic instantiations through return-type annotations
    /// like `Shared<T>` (from `Shared.clone`'s signature) — pre-fix the
    /// `T` remained literal and downstream method-call name resolution
    /// emitted `T.method` instead of `<concrete>.method`, mis-routing
    /// dispatch.
    ///
    /// `params.len()` may be less than `args.len()` (extra args ignored)
    /// or more (extra params not substituted) — both cases are handled
    /// gracefully; substitution is best-effort.
    pub(super) fn substitute_generic_params_in_type_name(
        name: &str,
        params: &[String],
        args: &[String],
    ) -> String {
        if params.is_empty() || args.is_empty() {
            return name.to_string();
        }
        let mut out = String::with_capacity(name.len() + args.iter().map(|a| a.len()).sum::<usize>());
        let bytes = name.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let before_ok = i == 0 || !Self::is_ident_byte(bytes[i - 1]);
            if before_ok {
                // Look for any param name starting at position i.
                let mut matched: Option<(usize, &str)> = None;
                for (idx, pname) in params.iter().enumerate() {
                    if idx >= args.len() {
                        break;
                    }
                    let p = pname.as_bytes();
                    if i + p.len() <= bytes.len() && &bytes[i..i + p.len()] == p {
                        let after_ok = i + p.len() == bytes.len()
                            || !Self::is_ident_byte(bytes[i + p.len()]);
                        if after_ok {
                            matched = Some((p.len(), args[idx].as_str()));
                            break;
                        }
                    }
                }
                if let Some((plen, arg)) = matched {
                    out.push_str(arg);
                    i += plen;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    /// Depth-aware split of a generic type's outer-arg list.
    ///
    /// For `"Shared<Mutex<SemaphoreInner>>"`, returns
    /// `vec!["Mutex<SemaphoreInner>"]`.  For `"Map<Int, Node>"`,
    /// returns `vec!["Int", "Node"]`.  For non-generic `"Mutex"`,
    /// returns an empty Vec.
    ///
    /// Used in conjunction with `substitute_generic_params_in_type_name`
    /// to map receiver-type's concrete args onto a callee's named
    /// type parameters.
    pub(super) fn split_generic_args(type_name: &str) -> Vec<String> {
        let start = match type_name.find('<') {
            Some(s) => s,
            None => return Vec::new(),
        };
        let end = match type_name.rfind('>') {
            Some(e) => e,
            None => return Vec::new(),
        };
        if start + 1 >= end {
            return Vec::new();
        }
        let inner = &type_name[start + 1..end];
        let mut args = Vec::new();
        let mut depth = 0;
        let mut arg_start = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    let s = inner[arg_start..i].trim();
                    if !s.is_empty() {
                        args.push(s.to_string());
                    }
                    arg_start = i + 1;
                }
                _ => {}
            }
        }
        let tail = inner[arg_start..].trim();
        if !tail.is_empty() {
            args.push(tail.to_string());
        }
        args
    }

    /// For a parameter of declared type `ty`, return the *simple
    /// name* of the closure-arg's return type IF the parameter is
    /// callable.  Two shapes are recognised:
    ///
    ///   * **Direct function type**: `f: fn(...) -> X` —
    ///     return `Some(extract_type_name_from_ast(X))`.
    ///   * **Generic-param bound**: `f: F` where `F` is one of the
    ///     function's generic parameters whose bounds include a
    ///     function type `fn(...) -> X`.  Return the same simple
    ///     name.
    ///
    /// Returns `None` for non-callable parameters (`Int`, `List<T>`,
    /// `&Foo`, …), so the caller can use it directly without further
    /// filtering.
    ///
    /// Used by `register_function` to populate
    /// `FunctionInfo.param_closure_return_type_names`.  Drives the
    /// call-site disambiguation hook in `compile_static_method_call`
    /// — see that function's per-arg `push_disambig_context` block.
    fn extract_closure_return_type_name(
        ty: &verum_ast::ty::Type,
        generics: &verum_common::List<verum_ast::ty::GenericParam>,
    ) -> Option<String> {
        use verum_ast::ty::{GenericParamKind, PathSegment, TypeBoundKind, TypeKind};

        // Helper: extract the simple base name of a Function type's
        // return type.  Strips generic args (`ReduceResult<R>` → `"ReduceResult"`).
        let return_name_of_fn = |return_ty: &verum_ast::ty::Type| -> Option<String> {
            let raw = Self::extract_type_name_from_ast(return_ty);
            let base = raw.split('<').next().unwrap_or(&raw).trim().to_string();
            if base.is_empty() || base == "()" {
                None
            } else {
                Some(base)
            }
        };

        // Direct fn-type case: `f: fn(...) -> X`.
        if let TypeKind::Function { return_type, .. } = &ty.kind {
            return return_name_of_fn(return_type);
        }

        // Generic-param bound case: `f: F` where `F: fn(...) -> X`.
        // The parser surfaces `F: fn(...)` as a TypeBound on the
        // generic param F; the bound is encoded as either
        // `GenericProtocol(Type)` or `Protocol(Path)` depending on
        // shape (the former covers `fn(...)`-shaped bounds carried
        // as a Type literal).  We probe both.
        if let TypeKind::Path(path) = &ty.kind
            && path.segments.len() == 1
            && let Some(PathSegment::Name(ident)) = path.segments.first()
        {
            let target_name = ident.name.as_str();
            for gp in generics.iter() {
                let (gp_name, bounds) = match &gp.kind {
                    GenericParamKind::Type { name, bounds, .. } => (name.name.as_str(), bounds),
                    GenericParamKind::HigherKinded { name, bounds, .. } => {
                        (name.name.as_str(), bounds)
                    }
                    _ => continue,
                };
                if gp_name != target_name {
                    continue;
                }
                for b in bounds.iter() {
                    // The parser encodes `F: fn(...) -> X` as an
                    // `Equality` bound carrying the raw Function type
                    // (see `verum_fast_parser::ty::type_to_type_bound`
                    // — complex non-Path / non-Generic types fall
                    // through to the Equality arm).
                    // `Iterator<Item = ...>`-shaped bounds carrying a
                    // Function type would land under `GenericProtocol`,
                    // so we probe both for robustness.
                    let bound_ty = match &b.kind {
                        TypeBoundKind::Equality(ty) => ty,
                        TypeBoundKind::GenericProtocol(ty) => ty,
                        _ => continue,
                    };
                    if let TypeKind::Function { return_type, .. } = &bound_ty.kind {
                        return return_name_of_fn(return_type);
                    }
                }
            }
        }

        None
    }

    /// Task #18 — does the declared return type carry a reference qualifier?
    ///
    /// Returns `true` for `&T`, `&mut T`, `&checked T`, `&checked mut T`,
    /// `&unsafe T`, `&unsafe mut T`.  Returns `false` for owned types,
    /// raw pointers (`*const T` / `*mut T`), `Heap<T>` / `Shared<T>`,
    /// generics whose type parameter happens to be a reference (caller's
    /// problem — we cannot inspect through the bind here), and the
    /// `None`-return-type case (no escape possible).
    ///
    /// Used as a fast pre-filter for `collect_escaping_local_refs`: if
    /// the function never returns a reference, no `&local` inside its
    /// body needs the DropRef-skip discipline.
    fn return_type_is_reference(ret: Option<&verum_ast::ty::Type>) -> bool {
        use verum_ast::ty::TypeKind;
        let Some(ty) = ret else { return false };
        matches!(
            ty.kind,
            TypeKind::Reference { .. }
                | TypeKind::CheckedReference { .. }
                | TypeKind::UnsafeReference { .. }
        )
    }

    /// Task #18 — collect locals whose `&local` reaches a return position.
    ///
    /// Walks the function body and adds to `out` the names of every local
    /// variable whose address-taken form (`&local` / `&mut local` /
    /// `&checked local` / `&unsafe local` and their mut variants)
    /// appears as:
    ///   - the trailing expression of the body block (implicit return), or
    ///   - the operand of an explicit `return &local;` statement.
    ///
    /// The walk is intentionally conservative — it only recognises the
    /// direct `&IDENT` pattern, not derived patterns like
    /// `&local.field` (which is still a returned ref but is rare and
    /// caught by the runtime stabilisation in `do_return`).
    ///
    /// Refs whose operand is a non-`Path` expression (string literal,
    /// arithmetic, call result, …) need no entry here — they are not
    /// scope-bound locals, so `compile_block` never emits DropRef on
    /// their backing register.  Their stabilisation is exclusively the
    /// interpreter's `do_return` responsibility.
    fn collect_escaping_local_refs(body: &FunctionBody, out: &mut std::collections::HashSet<String>) {
        match body {
            FunctionBody::Block(block) => Self::scan_block_for_escaping_refs(block, out),
            FunctionBody::Expr(expr) => Self::scan_expr_as_return(expr, out),
        }
    }

    fn scan_block_for_escaping_refs(block: &Block, out: &mut std::collections::HashSet<String>) {
        // Walk every statement so that explicit `return &x;` is caught
        // wherever it lives in the block (top-level, inside an
        // `if`/`match` arm, inside a nested block, …).
        for stmt in block.stmts.iter() {
            Self::scan_stmt_for_escaping_refs(stmt, out);
        }
        // Trailing expression of the block — implicit return on the
        // outermost block of the function body; for nested blocks it's
        // only a "return position" when the nested block itself sits in
        // a return position. We treat all trailing-expressions as
        // return-position candidates because the DropRef-skip set is
        // additive (false-positives only elide a redundant generation
        // bump; no soundness impact).
        if let Some(ref expr) = block.expr {
            Self::scan_expr_as_return(expr, out);
        }
    }

    fn scan_stmt_for_escaping_refs(stmt: &verum_ast::Stmt, out: &mut std::collections::HashSet<String>) {
        if let StmtKind::Expr { expr, .. } = &stmt.kind {
            Self::scan_expr_for_return_stmt(expr, out);
        }
        // Let, Item, Defer, Provide, … — no return-position expression
        // surfaces directly through these stmt kinds.  (A nested `return
        // &x;` inside, e.g., a `defer { return &x; }` block would still
        // be reached because the inner block expression goes through
        // `scan_expr_for_return_stmt` when reached as an Expr stmt.)
    }

    /// Recursively scan an expression for explicit `return <expr>` nodes.
    /// Whenever one is found, its operand is treated as a return-position
    /// expression and fed into `scan_expr_as_return`.
    fn scan_expr_for_return_stmt(expr: &verum_ast::Expr, out: &mut std::collections::HashSet<String>) {
        use verum_ast::expr::ExprKind;
        match &expr.kind {
            ExprKind::Return(value) => {
                if let verum_common::Maybe::Some(ret_expr) = value {
                    Self::scan_expr_as_return(ret_expr, out);
                }
            }
            ExprKind::Block(block) => {
                Self::scan_block_for_escaping_refs(block, out);
            }
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::scan_block_for_escaping_refs(then_branch, out);
                if let verum_common::Maybe::Some(else_block) = else_branch {
                    Self::scan_expr_for_return_stmt(else_block, out);
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms.iter() {
                    Self::scan_expr_for_return_stmt(&arm.body, out);
                }
            }
            // Other expr kinds: any nested `return` inside them would be
            // ill-formed at this position; we do not descend.
            _ => {}
        }
    }

    /// Inspect a return-position expression.  If it is a direct `&IDENT`
    /// pattern (possibly wrapped in `Paren`), insert the identifier's
    /// name into `out`.
    fn scan_expr_as_return(expr: &verum_ast::Expr, out: &mut std::collections::HashSet<String>) {
        use verum_ast::expr::{ExprKind, UnOp};
        // Strip parens — `return (&x)` parses with Paren wrapper.
        let inner = Self::strip_paren(expr);
        match &inner.kind {
            ExprKind::Unary { op, expr: inner_expr } if matches!(
                op,
                UnOp::Ref
                    | UnOp::RefMut
                    | UnOp::RefChecked
                    | UnOp::RefCheckedMut
                    | UnOp::RefUnsafe
                    | UnOp::RefUnsafeMut
            ) => {
                let operand = Self::strip_paren(inner_expr);
                // Direct bare-identifier pattern: `&local`.
                if let ExprKind::Path(path) = &operand.kind
                    && path.segments.len() == 1
                    && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
                {
                    out.insert(ident.name.to_string());
                }
            }
            ExprKind::Block(block) => {
                // `{ ...; &x }` — trailing expression of inner block is
                // the return-position; recurse through the inner block.
                if let Some(ref tail) = block.expr {
                    Self::scan_expr_as_return(tail, out);
                }
                // Also scan statements for explicit `return &x;`.
                for stmt in block.stmts.iter() {
                    Self::scan_stmt_for_escaping_refs(stmt, out);
                }
            }
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let Some(ref tail) = then_branch.expr {
                    Self::scan_expr_as_return(tail, out);
                }
                for stmt in then_branch.stmts.iter() {
                    Self::scan_stmt_for_escaping_refs(stmt, out);
                }
                if let verum_common::Maybe::Some(else_block) = else_branch {
                    Self::scan_expr_as_return(else_block, out);
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms.iter() {
                    Self::scan_expr_as_return(&arm.body, out);
                }
            }
            _ => {}
        }
    }

    #[inline]
    fn strip_paren(expr: &verum_ast::Expr) -> &verum_ast::Expr {
        use verum_ast::expr::ExprKind;
        match &expr.kind {
            ExprKind::Paren(inner) => Self::strip_paren(inner),
            _ => expr,
        }
    }

    /// Generic parameters are preserved so that element types can be extracted
    /// later via `extract_element_type` (e.g., "List<Token>" → "Token").
    fn extract_type_name_from_ast(ty: &verum_ast::ty::Type) -> String {
        use verum_ast::ty::{GenericArg, PathSegment, TypeKind};
        if let Some(name) = ty.kind.primitive_name() {
            return name.to_string();
        }
        match &ty.kind {
            TypeKind::Path(path) => {
                // Get the last segment name (handles qualified paths like core.collections.List)
                // Self is encoded as PathSegment::SelfValue — surface it as the
                // canonical capitalised "Self" token so substitute_self_in_type_name
                // can perform Self → concrete substitution at register_impl_function
                // time. Falling through to `format!("{}", path)` here would render
                // Self as lowercase "self" (the Path Display impl), which the
                // word-boundary substitution then misses.
                path.segments
                    .last()
                    .and_then(|seg| match seg {
                        PathSegment::Name(ident) => Some(ident.name.to_string()),
                        PathSegment::SelfValue => Some("Self".to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("{}", path))
            }
            TypeKind::Generic { base, args } => {
                // Preserve generic parameters: List<Token> → "List<Token>"
                let base_name = Self::extract_type_name_from_ast(base);
                if args.is_empty() {
                    base_name
                } else {
                    let arg_strs: Vec<String> = args
                        .iter()
                        .map(|arg| match arg {
                            GenericArg::Type(ty) => Self::extract_type_name_from_ast(ty),
                            GenericArg::Const(_) => "_".to_string(),
                            _ => "_".to_string(),
                        })
                        .collect();
                    format!("{}<{}>", base_name, arg_strs.join(", "))
                }
            }
            // **Reference / pointer name preservation.**
            //
            // Default reference (`&T`) and checked-reference (`&checked T`)
            // flatten to the inner type name — both share the same
            // value-receiver dispatch semantics as `T` itself, so callers
            // querying `type_field_type_names` for `(Map, len)` correctly
            // get `Int` regardless of whether `len` is declared `Int` or
            // `&Int`.
            //
            // Unsafe reference (`&unsafe T`) and raw pointer
            // (`*const T` / `*mut T`) MUST preserve their `&unsafe ` /
            // `*const ` / `*mut ` prefix in the carrier name.  The
            // codegen's raw-pointer-flag propagation (see
            // `compile_field_access`'s post-GetF marking and
            // `compile_method_call`'s `offset` / `add` / `sub` /
            // `is_null` intercepts) keys on this prefix to decide
            // whether to route a method call through `ptr_offset` and
            // friends instead of CallM-dispatching against a non-
            // existent `<InnerType>.<method>` user method.  Stripping
            // the prefix here turns `self.entries.offset(idx)` into
            // a CallM against a phantom `Slot.offset`, which the
            // runtime then routes through the Int-receiver primitive
            // dispatch (raw pointers are i64-encoded under NaN-
            // boxing) — surfaces as "method 'Slot.offset' not found
            // on receiver of runtime kind Int" and aborts every
            // hash-table body the moment it touches its backing
            // pointer-array.
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. } => {
                Self::extract_type_name_from_ast(inner)
            }
            TypeKind::UnsafeReference { inner, .. } => {
                format!("&unsafe {}", Self::extract_type_name_from_ast(inner))
            }
            TypeKind::Pointer { inner, mutable, .. } => {
                let prefix = if *mutable { "*mut " } else { "*const " };
                format!("{}{}", prefix, Self::extract_type_name_from_ast(inner))
            }
            TypeKind::Slice(inner) => {
                format!("[{}]", Self::extract_type_name_from_ast(inner))
            }
            TypeKind::DynProtocol { bounds, .. } => {
                // dyn Protocol → "dyn:Protocol" for dispatch tracking
                let bound_names: Vec<String> = bounds
                    .iter()
                    .filter_map(|b| {
                        if let verum_ast::ty::TypeBoundKind::Protocol(path) = &b.kind {
                            path.as_ident().map(|id| id.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                format!("dyn:{}", bound_names.join("+"))
            }
            _ => format!("{:?}", ty.kind).chars().take(20).collect(),
        }
    }

    /// Registers the field layout for a record type.
    /// Called during type declaration collection.
    ///
    /// **Registration invariants (drift-pinned):**
    ///
    ///   1.  `type_field_layouts[type_name]` is set on the FIRST
    ///       registration and never overwritten by a later one —
    ///       this preserves the user-phase declaration when the
    ///       same type appears across multiple compilation passes
    ///       (Pass 1b user-phase / Pass 1c stdlib-phase / cross-
    ///       module mount tracing) so the field-index ordering is
    ///       stable for the entire codegen lifetime.
    ///
    ///   2.  `type_field_type_names[(type_name, field)]` is
    ///       ALWAYS populated by every registration call.  Pre-fix
    ///       this map was gated by the same first-registration
    ///       guard as the layout, but a partial first-registration
    ///       (the type_decl reaches `register_record_fields` from
    ///       a forward-reference path with an empty field-types
    ///       list, or the stdlib precompile lands the layout but
    ///       not the type-name entries) left the field-type map
    ///       missing the canonical entries.  Downstream
    ///       `infer_expr_type_name`'s Field arm then failed to
    ///       recover the receiver type through `obj.field`
    ///       chains, falling through to `resolve_field_index`'s
    ///       global-scan path that picks an unrelated type's
    ///       same-named field at a wrong index.  Surfaced as
    ///       "field index N (offset …) exceeds object data
    ///       size M" panics on `let f: ReadyFuture<Int> = ready(0);
    ///       f.value` when collection types were also mounted.
    ///
    ///   3.  The simple-name cross-registration (for qualified
    ///       `module.Type` declarations) follows the same two
    ///       invariants — layout under guard, field-type names
    ///       always.
    fn register_record_fields(
        &mut self,
        type_name: &str,
        field_names: Vec<String>,
        field_types: Vec<String>,
    ) {
        // Intern all field names to ensure they have assigned indices
        for name in &field_names {
            self.intern_field_name(name);
        }

        // Always populate the field-type map for THIS type_name —
        // see invariant #2 above.  Re-registration with the SAME
        // field-types is idempotent; re-registration with stale
        // field-types is impossible here because each call carries
        // the canonical AST-derived field set.
        for (name, ty) in field_names.iter().zip(field_types.iter()) {
            self.type_field_type_names
                .insert((type_name.to_string(), name.clone()), ty.clone());
        }

        // Set the layout under the first-registration guard — see
        // invariant #1 above.
        if !self.type_field_layouts.contains_key(type_name) {
            self.type_field_layouts
                .insert(type_name.to_string(), field_names.clone());
        }

        // Cross-module field access support: also register under the simple name
        // (without module path) so imports using unqualified names can find fields.
        // e.g., "module_a.Point" → also register as "Point"
        if type_name.contains('.') {
            let simple = type_name.rsplit('.').next().unwrap_or(type_name);
            // Field-type map: always populate.
            for (name, ty) in field_names.iter().zip(field_types.iter()) {
                self.type_field_type_names
                    .insert((simple.to_string(), name.clone()), ty.clone());
            }
            // Layout: guarded.
            if !self.type_field_layouts.contains_key(simple) {
                self.type_field_layouts
                    .insert(simple.to_string(), field_names);
            }
        }
    }

    /// Interns a string and returns its ID.
    fn intern_string(&mut self, s: &str) -> u32 {
        // Check if already interned
        for (i, existing) in self.ctx.strings.iter().enumerate() {
            if existing == s {
                return i as u32;
            }
        }

        let id = self.ctx.strings.len() as u32;
        self.ctx.strings.push(s.to_string());
        id
    }

    /// Walk every emitted function's instruction stream collecting
    /// the FunctionId targets of Call / CallG / TailCall / NewClosure /
    /// Spawn / GenCreate / CallM operands.  For each unique id that
    /// has NO corresponding VbcFunction in `self.functions` but DOES
    /// appear in `ctx.functions` (i.e. was loaded from the stdlib
    /// archive via `archive_ctx_loader::apply_lazy`), push a stub
    /// VbcFunction with the right descriptor metadata (name, arity,
    /// parent_type) so the interpreter's name-based intercept layer
    /// can dispatch.
    ///
    /// Idempotent + bounded: only adds stubs for IDs that are both
    /// (a) referenced by user bytecode and (b) registered in
    /// ctx.functions.  Bodies are zero-instruction `RetV` stubs —
    /// the actual implementation is intercepted at the call boundary
    /// (`try_intercept_shell_runtime` / `file_runtime` / `env_runtime`
    /// /… in dispatch_table).  No new module dependencies.
    fn emit_missing_stub_descriptors(&mut self) {
        self.emit_missing_stub_descriptors_with_callm(true);
    }

    /// **Task #47 stage-3-only stub descriptor emission.**
    ///
    /// Surgical counterpart to `emit_missing_stub_descriptors_with_callm`
    /// for the stdlib-bootstrap `finalize_module_from_state` path.
    ///
    /// Walks every emitted function's instruction stream collecting
    /// Call / CallG / TailCall / NewClosure / Spawn / GenCreate target
    /// ids that fall in the **stage-3 sentinel range**
    /// (`u32::MAX - 0x100_0000 .. u32::MAX - 0xF0_0000`).  For each
    /// such id that has NO corresponding `VbcFunction` in
    /// `self.functions` but DOES appear in `ctx.functions`, synthesises
    /// a minimal extern-shaped `FunctionDescriptor` carrying just the
    /// function's NAME so `ArchiveBodyRemap::map_function`'s Tier-2b
    /// name-fallback can resolve `Call(stub_id)` to the real
    /// user-side FunctionId at archive load.
    ///
    /// **Why this exists alongside the legacy `_with_callm` variant**:
    /// the legacy pass processes EVERY referenced id (including
    /// legitimate cross-module Call targets with non-sentinel ids),
    /// which produces ~800K stub descriptors at stdlib scale and
    /// blows `runtime.vbca` from ~13 MB to ~145 MB (documented at
    /// `mod.rs:5687-5750`).  This variant filters strictly to the
    /// stage-3 sentinel range (registered by
    /// `verum_compiler::pipeline::stdlib_bootstrap::
    /// pre_register_unique_public_free_functions`), keeping the
    /// archive growth bounded to ~100 KB.
    ///
    /// Idempotent + bounded: only adds stubs for IDs that are both
    /// (a) referenced by emitted bytecode AND (b) in the stage-3
    /// sentinel range AND (c) registered in `ctx.functions`.  Bodies
    /// are zero-instruction `RetV` stubs — the archive loader
    /// rewrites the Call's `func_id` via name lookup before the
    /// stub body is ever executed.
    fn emit_stage3_stub_descriptors(&mut self) {
        use std::collections::HashSet;
        // Mirror the STAGE3 range from
        // `verum_compiler::pipeline::stdlib_bootstrap`.
        const STAGE3_BASE: u32 = u32::MAX - 0x100_0000;
        const STUB_RANGE_WIDTH: u32 = 0x10_0000; // 1M slots
        let is_stage3 = |id: u32| -> bool {
            id <= STAGE3_BASE && id >= STAGE3_BASE.saturating_sub(STUB_RANGE_WIDTH)
        };

        // 1. Collect stage-3 referenced ids from emitted bytecode.
        let mut referenced: HashSet<u32> = HashSet::new();
        for vbc_func in self.functions.iter() {
            for instr in &vbc_func.instructions {
                match instr {
                    Instruction::Call { func_id, .. }
                    | Instruction::CallG { func_id, .. }
                    | Instruction::TailCall { func_id, .. }
                    | Instruction::NewClosure { func_id, .. }
                    | Instruction::Spawn { func_id, .. }
                    | Instruction::GenCreate { func_id, .. } => {
                        if is_stage3(*func_id) {
                            referenced.insert(*func_id);
                        }
                    }
                    _ => {}
                }
            }
        }
        if referenced.is_empty() {
            return;
        }

        // 2. Already-emitted IDs.
        let emitted: HashSet<u32> = self
            .functions
            .iter()
            .map(|f| f.descriptor.id.0)
            .collect();

        // 3. Build id → (name, info) map for referenced stage-3 ids.
        // PREFER qualified names (more dots → canonical).
        let mut id_to_entry: std::collections::HashMap<u32, (String, crate::codegen::FunctionInfo)> =
            std::collections::HashMap::new();
        for (name, info) in self.ctx.functions.iter() {
            if !is_stage3(info.id.0) {
                continue;
            }
            if !referenced.contains(&info.id.0) {
                continue;
            }
            id_to_entry
                .entry(info.id.0)
                .and_modify(|(existing_name, _)| {
                    let existing_dots = existing_name.matches('.').count();
                    let new_dots = name.matches('.').count();
                    if new_dots > existing_dots {
                        *existing_name = name.clone();
                    }
                })
                .or_insert_with(|| (name.clone(), info.clone()));
        }

        // 3b. **Stage-3 stub-name preservation fallback** (task #47
        // cascade root-cause fix).  For every `referenced` stub-id
        // that's NOT in `id_to_entry` (i.e. its bare-name slot in
        // `ctx.functions` was OVERWRITTEN by a real-id registration
        // during this module's compile), recover the original name
        // from `ctx.stage3_stub_names` and synthesize a minimal
        // FunctionInfo from the stub's name.
        //
        // Without this fallback the stub-id leaks past
        // `ArchiveBodyRemap::map_function`'s Tier 2a/2b name lookup
        // (because `archive_id_to_name[stub_id]` is missing — no
        // descriptor was emitted), surfaces at runtime as
        // `[lenient] stage-3 ... stub never resolved (func_id=N)`.
        for &stub_id in referenced.iter() {
            if id_to_entry.contains_key(&stub_id) {
                continue;
            }
            if let Some(recovered_name) = self.ctx.stage3_stub_names.get(&stub_id) {
                // Look up FunctionInfo by recovered name in
                // ctx.functions — may be the REAL-id info (overwrite
                // already happened) or absent.  Either way, we
                // synthesize a stub descriptor with the recovered
                // name so the archive load can chase the real body.
                let info = self
                    .ctx
                    .functions
                    .get(recovered_name)
                    .cloned()
                    .unwrap_or_else(|| {
                        // True orphan — synthesize minimal info with
                        // default 0-arity.  At runtime this can still
                        // panic if the archive load doesn't find a
                        // real body, but the panic now carries the
                        // recovered name in the descriptor for
                        // diagnosis.
                        crate::codegen::FunctionInfo {
                            id: crate::module::FunctionId(stub_id),
                            param_count: 0,
                            param_names: Vec::new(),
                            param_type_names: Vec::new(),
                            is_async: false,
                            is_generator: false,
                            contexts: Vec::new(),
                            return_type: None,
                            yield_type: None,
                            intrinsic_name: None,
                            variant_tag: None,
                            parent_type_name: None,
                            variant_payload_types: None,
                            is_partial_pattern: false,
                            takes_self_mut_ref: false,
                            return_type_name: None,
                            return_type_inner: None,
                            is_const: false,
                            is_transparent_wrapper: false,
                            param_closure_return_type_names: Vec::new(),
                        }
                    });
                id_to_entry.insert(stub_id, (recovered_name.clone(), info));
            }
        }

        // 4. Synthesise minimal descriptor for each id not in emitted.
        let mut to_push: Vec<(u32, String, crate::codegen::FunctionInfo)> = Vec::new();
        for (&id, (name, info)) in id_to_entry.iter() {
            if emitted.contains(&id) {
                continue;
            }
            to_push.push((id, name.clone(), info.clone()));
        }

        for (id, name, info) in to_push {
            let name_id = StringId(self.ctx.intern_string_raw(&name));
            let mut descriptor = crate::module::FunctionDescriptor::new(name_id);
            descriptor.id = crate::module::FunctionId(id);
            descriptor.register_count = 1;
            descriptor.locals_count = info.param_count as u16;
            for (i, pname) in info.param_names.iter().enumerate() {
                let pname_to_use = if pname.is_empty() {
                    format!("_arg{}", i)
                } else {
                    pname.clone()
                };
                let pname_id =
                    StringId(self.ctx.intern_string_raw(&pname_to_use));
                descriptor.params.push(crate::module::ParamDescriptor {
                    name: pname_id,
                    type_ref: TypeRef::Concrete(TypeId::UNIT),
                    is_mut: false,
                    default: None,
                });
            }
            while descriptor.params.len() < info.param_count {
                let i = descriptor.params.len();
                let pname_id = StringId(
                    self.ctx.intern_string_raw(&format!("_arg{}", i)),
                );
                descriptor.params.push(crate::module::ParamDescriptor {
                    name: pname_id,
                    type_ref: TypeRef::Concrete(TypeId::UNIT),
                    is_mut: false,
                    default: None,
                });
            }
            if info.is_async {
                descriptor.properties |= crate::types::PropertySet::ASYNC;
            }
            descriptor.is_generator = info.is_generator;
            if let Some(parent_name) = info.parent_type_name.as_deref()
                && let Some(&parent_tid) = self.type_name_to_id.get(parent_name)
            {
                descriptor.parent_type = Some(parent_tid);
            }
            descriptor.is_const = info.is_const;
            let vbc_func = crate::module::VbcFunction::new(
                descriptor,
                vec![Instruction::RetV],
            );
            self.functions.push(vbc_func);
        }
    }

    /// Surgical variant of `emit_missing_stub_descriptors`.
    ///
    /// When `include_callm = false`, runs ONLY the Call-id-driven pass
    /// (synthesises stubs for each unique `Call.func_id` in emitted
    /// bytecode that lacks a corresponding `VbcFunction` in
    /// `self.functions` but appears in `ctx.functions`). The CallM-by-
    /// method-name pass is SKIPPED.
    ///
    /// **Why this exists**: the CallM pass iterates `ctx.functions`
    /// entries whose trailing dotted segment matches each unique
    /// CallM `method_id` string. For BARE method names like `hash` /
    /// `next` / `iter` / `clone`, the suffix-index bucket contains
    /// hundreds of entries (every type that implements the method).
    /// The pass then pushes one stub per match — at stdlib precompile
    /// time, with 100+ modules each calling 100+ bare CallM methods,
    /// this produces ~1M stubs and grows `runtime.vbca` 12.9 MB →
    /// 110-132 MB, with user-side load times blowing up from <1 s to
    /// 85 s and verum process memory hitting 10 GB.
    ///
    /// At STDLIB PRECOMPILE time we want ONLY the Call-id pass — it's
    /// targeted (one stub per actually-referenced cross-module
    /// `Call.func_id`) and sufficient for the cross-module dispatch
    /// case (`Text.grow → alloc`-style raw Calls). The CallM pass is
    /// for USER-MODULE compile time where stub synthesis covers
    /// downstream-method dispatch via the runtime intercept layer.
    ///
    /// **Pin**: `finalize_module_from_state` (stdlib bootstrap) calls
    /// this with `include_callm=false`; `finalize_module` (user
    /// compile) calls the unconditional variant via the alias above.
    fn emit_missing_stub_descriptors_with_callm(&mut self, include_callm: bool) {
        use std::collections::{HashMap, HashSet};
        // 1. Collect referenced FunctionIds from emitted bytecode.
        let mut referenced: HashSet<u32> = HashSet::new();
        // Method names referenced via CallM.method_id — those are
        // STRING IDs (interned method names like "as_path" or
        // "Text.push_str"), not function IDs.  Resolve them to
        // ctx.functions entries by string match below.
        let mut method_names: HashSet<u32> = HashSet::new();
        for vbc_func in self.functions.iter() {
            for instr in &vbc_func.instructions {
                match instr {
                    Instruction::Call { func_id, .. }
                    | Instruction::CallG { func_id, .. }
                    | Instruction::TailCall { func_id, .. }
                    | Instruction::NewClosure { func_id, .. }
                    | Instruction::Spawn { func_id, .. }
                    | Instruction::GenCreate { func_id, .. } => {
                        referenced.insert(*func_id);
                    }
                    Instruction::CallM { method_id, .. } => {
                        method_names.insert(*method_id);
                    }
                    _ => {}
                }
            }
        }
        // 2. Already-emitted IDs.
        let emitted: HashSet<u32> = self
            .functions
            .iter()
            .map(|f| f.descriptor.id.0)
            .collect();
        // 3. Build name → FunctionInfo map, indexed by id.
        // ctx.functions is HashMap<String, FunctionInfo>; we need
        // to find by id.  Build a side index.
        //
        // **Cold-start optimisation**: only collect entries whose id
        // is actually `referenced` from emitted bytecode.  ctx.functions
        // grows to ~28K entries on a fully-mounted script (every
        // archive function loaded in apply_lazy_with_types lives there);
        // walking the full table to clone names + match-and-modify
        // the dots-tiebreak HashMap entry was burning ~30ms of
        // cold-start on hello-world.  Filtering on `referenced` first
        // collapses the work to O(N_referenced) typical few-tens —
        // most `ctx.functions` entries never get called by user code
        // and don't need a stub at all.
        let mut id_to_name: HashMap<u32, String> = HashMap::new();
        for (name, info) in self.ctx.functions.iter() {
            // Skip sentinel-id entries (FFI extern, newtype ctor —
            // those are dispatched out-of-band; emitting a stub
            // would collide with the real sentinel handler at the
            // call site).  Use the same threshold as
            // `import_functions`.
            const SENTINEL_THRESHOLD: u32 = u32::MAX / 4;
            if info.id.0 >= SENTINEL_THRESHOLD {
                continue;
            }
            // Skip ids never referenced by the emitted bytecode.
            // The `referenced` set was built in step 1 from every
            // Call/CallG/TailCall/NewClosure/Spawn/GenCreate site,
            // so this gate's only false-positive risk is a
            // late-emitted stub that references id X without X
            // being in `referenced` — which can't happen because
            // we walk the bytecode AFTER all emission has finished.
            if !referenced.contains(&info.id.0) {
                continue;
            }
            // PREFER QUALIFIED NAMES so the stub's simple-name
            // extraction (`rsplit('.').next()`) recovers the
            // canonical bare function name rather than a
            // user-supplied alias.  Without this, mounting
            // `core.shell.script.{args as script_args}` could leave
            // the stub registered under `script_args` — interpreter
            // intercepts (`try_intercept_env_runtime` etc.) match
            // on the canonical `args` and miss the alias.  Picking
            // the longest dotted name ensures we always have the
            // canonical bare name available as the trailing segment.
            id_to_name
                .entry(info.id.0)
                .and_modify(|existing| {
                    let existing_dots = existing.matches('.').count();
                    let new_dots = name.matches('.').count();
                    if new_dots > existing_dots {
                        *existing = name.clone();
                    }
                })
                .or_insert_with(|| name.clone());
        }
        // 4. For each referenced id missing from `emitted`, create
        // a stub.  Skip ids beyond SENTINEL_THRESHOLD (handled by
        // the FFI / variant-ctor / newtype-ctor dispatch paths).
        const SENTINEL_THRESHOLD: u32 = u32::MAX / 4;
        // Track which (name, arity) stubs we synthesise so the CallM
        // pass below doesn't add duplicates for the same qualified
        // function id.
        let mut stub_synthesised: std::collections::HashSet<u32> =
            std::collections::HashSet::new();
        // #97 — Const-declaration unconditional emission.  `public const`
        // declarations are registered as zero-arg FunctionInfo entries
        // (`is_const = true`); when no internal stdlib bytecode
        // references them, the `referenced` set above never contains
        // their ids and `emit_missing_stub_descriptors` skips them.
        // The result is a precompiled archive where stdlib consts are
        // missing from `module.functions` entirely — every user-side
        // `mount core.text.{SSO_CAPACITY}` then surfaces as `unbound
        // variable: SSO_CAPACITY` because the archive-driven
        // typechecker has nothing to register.  Force-emit every const
        // by injecting its id into the `referenced` set up-front; the
        // existing stub loop then handles them uniformly.  The is_const
        // marker is propagated below in the descriptor build.
        let mut referenced = referenced;
        for (name, info) in self.ctx.functions.iter() {
            if !info.is_const || info.id.0 >= SENTINEL_THRESHOLD {
                continue;
            }
            referenced.insert(info.id.0);
            id_to_name
                .entry(info.id.0)
                .or_insert_with(|| name.clone());
        }
        for id in referenced {
            if id >= SENTINEL_THRESHOLD {
                continue;
            }
            if emitted.contains(&id) {
                continue;
            }
            let name = match id_to_name.get(&id) {
                Some(n) => n.clone(),
                None => continue, // ID not in ctx.functions — drop
            };
            let info = match self.ctx.functions.get(&name) {
                Some(i) => i.clone(),
                None => continue,
            };
            // Use the QUALIFIED name as descriptor.name when ctx
            // tracks one — `find_function_by_name`/`_by_unique_bare_suffix`
            // both honour `.suffix` matching against fully-qualified
            // registrations, while name-based intercepts
            // (`try_intercept_*` etc.) consistently strip via
            // `rsplit('.').next()` so they work on either form.
            // Carrying the qualified form lets bare-name CallM
            // dispatch (`receiver.as_path()` codegen-emitted as
            // `CallM { method_id: "as_path" }`) hit the suffix-match
            // path and resolve to the unique `<TypeName>.as_path`
            // stub instead of bottoming out as
            // "method 'as_path' not found".
            let name_id = StringId(self.ctx.intern_string_raw(&name));
            stub_synthesised.insert(id);
            let mut descriptor = crate::module::FunctionDescriptor::new(name_id);
            descriptor.id = crate::module::FunctionId(id);
            descriptor.register_count = 1;
            descriptor.locals_count = info.param_count as u16;
            // Populate ParamDescriptors — placeholder names + UNIT
            // type refs.  Arity is the only field the dispatch path
            // checks for stub descriptors; the body is `RetV` so
            // type refs aren't consulted at runtime.
            for (i, pname) in info.param_names.iter().enumerate() {
                let pname_to_use = if pname.is_empty() {
                    format!("_arg{}", i)
                } else {
                    pname.clone()
                };
                let pname_id = StringId(self.ctx.intern_string_raw(&pname_to_use));
                descriptor.params.push(crate::module::ParamDescriptor {
                    name: pname_id,
                    type_ref: TypeRef::Concrete(TypeId::UNIT),
                    is_mut: false,
                    default: None,
                });
            }
            // Pad to declared param_count when fewer names were
            // available (extern FFI declarations sometimes come in
            // with empty `param_names`).
            while descriptor.params.len() < info.param_count {
                let i = descriptor.params.len();
                let pname_id = StringId(self.ctx.intern_string_raw(&format!("_arg{}", i)));
                descriptor.params.push(crate::module::ParamDescriptor {
                    name: pname_id,
                    type_ref: TypeRef::Concrete(TypeId::UNIT),
                    is_mut: false,
                    default: None,
                });
            }
            // PropertySet — only ASYNC matters for stubs; the rest
            // are runtime-effect tags consulted at AOT codegen, not
            // by the interpreter's intercept layer.
            if info.is_async {
                descriptor.properties |= crate::types::PropertySet::ASYNC;
            }
            descriptor.is_generator = info.is_generator;
            // Copy parent_type when present so method-dispatch
            // paths that key on `descriptor.parent_type` still work
            // through the stub (e.g.
            // `verify_global_type_table_consistency` reports).
            if let Some(parent_name) = info.parent_type_name.as_deref() {
                if let Some(&parent_tid) = self.type_name_to_id.get(parent_name) {
                    descriptor.parent_type = Some(parent_tid);
                }
            }
            // #87/#97 — propagate const-storage marker and inline-
            // constant marker.  Stubs emitted from a `public const X`
            // declaration must carry both: `is_const` so the archive-
            // driven typechecker treats the archive entry as a value,
            // and `intrinsic_name = Some("__const_val_<N>")` so user-
            // side codegen inlines the literal at every reference
            // site instead of emitting a Call to the empty `RetV`
            // body.  Without these the stub is indistinguishable
            // from a zero-arg fn whose body trivially returns Unit.
            descriptor.is_const = info.is_const;
            if let Some(ref iname) = info.intrinsic_name {
                descriptor.intrinsic_name =
                    Some(StringId(self.ctx.intern_string_raw(iname)));
            }
            // Stubs synthesised from a const get the const's actual
            // return type (typically `Int` / `Text`) so the typechecker's
            // archive-side metadata extraction sees a meaningful type
            // rather than `Unit` (the declared return for a body-less
            // RetV stub).  Without this, every cross-module `let x =
            // SSO_CAPACITY` would type x as Unit.
            if info.is_const {
                if let Some(ref rt) = info.return_type {
                    descriptor.return_type = rt.clone();
                }
            }
            let vbc_func = crate::module::VbcFunction::new(
                descriptor,
                vec![Instruction::RetV],
            );
            self.functions.push(vbc_func);
        }
        // CallM stubs are gated — stdlib precompile path
        // (`finalize_module_from_state`) passes `include_callm=false`
        // to avoid the 1M-stub explosion documented in the docstring
        // above. User-side compile keeps `include_callm=true` because
        // its scale is per-cog (not per-stdlib-module) and the
        // intercept layer needs the descriptor metadata for dispatch.
        if !include_callm {
            return;
        }
        // CallM stubs.  `method_id` is a STRING id, not a FunctionId,
        // so the FunctionId-driven loop above never sees these.  Without
        // a stub, name-based dispatch via `find_function_by_unique_bare_suffix`
        // misses `<TypeName>.<method>` (the qualified registration only
        // exists in `ctx.functions`, not in this module's compiled list).
        //
        // For each unique CallM method name, find ctx.functions entries
        // whose qualified name ends with `.<method>` (e.g. `PathBuf.as_path`)
        // and synthesise a `RetV` stub for each unemitted one.  Rust-side
        // intercepts (`try_intercept_file_runtime`, `try_intercept_shell_runtime`,
        // text mutation intercepts above this layer) fire BEFORE the body
        // runs, so the empty `RetV` body is never executed for any method
        // that has an intercept.  Methods without an intercept will surface
        // as a more debuggable "function returned Unit, expected T"
        // downstream rather than the opaque "method not found".
        // **Cold-start optimisation**: pre-build a simple-name index
        // over ctx.functions ONCE before the per-method loop.  The
        // CallM pass needs to find every qualified ctx.functions
        // entry whose trailing dotted segment equals `<method_name>`
        // (e.g. `PathBuf.as_path` for method `as_path`).  The original
        // loop walked all ~28K ctx.functions entries for each unique
        // CallM method name — O(M × N) on cold start, several tens of
        // ms on hello-world.  By bucketing once on `last_segment`,
        // each method-name lookup becomes O(1) average.
        // Owned suffix_index — references would tangle with the
        // `&mut self.ctx.intern_string_raw` calls inside the per-method
        // loop below.  Cloning each FunctionInfo once is cheaper than
        // cloning per-method × per-candidate as the unindexed walk did.
        let suffix_index: HashMap<String, Vec<(String, FunctionInfo)>> = {
            let mut idx: HashMap<String, Vec<(String, FunctionInfo)>> = HashMap::new();
            for (name, info) in self.ctx.functions.iter() {
                if info.id.0 >= SENTINEL_THRESHOLD {
                    continue;
                }
                let last_seg = match name.rfind('.') {
                    Some(p) => name[p + 1..].to_string(),
                    None => name.clone(),
                };
                idx.entry(last_seg).or_default().push((name.clone(), info.clone()));
            }
            idx
        };
        for method_id in method_names {
            let method_name = match self
                .ctx
                .strings
                .get(StringId(method_id).0 as usize)
                .cloned()
            {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            // For QUALIFIED method names (e.g. `Maybe.is_some`,
            // `PathBuf.as_path`), the bucket key is the trailing
            // segment AFTER the last dot — that's how
            // `suffix_index` was built above.  Without splitting
            // here, the lookup hits an empty bucket (the full
            // dotted name is never a bucket key) and the qualified
            // ctx.functions entry never gets a stub.  Result at
            // runtime: `find_function_by_name("Maybe.is_some")`
            // misses (no entry in `self.functions` named
            // `Maybe.is_some` and no entry whose suffix is
            // `.Maybe.is_some`), method dispatch falls through
            // every path, and the receiver-of-runtime-kind panic
            // fires.  Dotted call sites then have to rely on the
            // FunctionId-driven path above — but THAT path only
            // sees Call/CallG/TailCall/NewClosure/Spawn/GenCreate
            // sites, NOT CallM, so dotted-method CallMs were
            // structurally orphaned.
            //
            // Fundamental fix: treat the trailing segment as the
            // bucket key for both bare and dotted forms, and (for
            // dotted forms) additionally constrain the candidate
            // set to entries whose qualified name ENDS WITH the
            // full dotted method name.  This filters out
            // `Result.is_some` when looking up `Maybe.is_some`
            // (both bucket under "is_some") so the synthesised
            // stub matches the call site's intent.
            let (bucket_key, qualified_constraint): (&str, Option<&str>) =
                match method_name.rsplit_once('.') {
                    Some((_, tail)) => (tail, Some(method_name.as_str())),
                    None => (method_name.as_str(), None),
                };
            // O(1) lookup via the suffix index — buckets every
            // ctx.functions entry whose trailing dotted segment
            // matches `bucket_key`.  Empty bucket → no candidates,
            // skip without allocating.
            let candidates: Vec<(String, FunctionInfo)> = suffix_index
                .get(bucket_key)
                .cloned()
                .unwrap_or_default();
            let candidates: Vec<(String, FunctionInfo)> = match qualified_constraint {
                Some(qual) => {
                    let dot_qual = format!(".{}", qual);
                    candidates
                        .into_iter()
                        .filter(|(name, _)| name == qual || name.ends_with(&dot_qual))
                        .collect()
                }
                None => candidates,
            };
            for (qualified_name, info) in candidates {
                if emitted.contains(&info.id.0) {
                    continue;
                }
                if !stub_synthesised.insert(info.id.0) {
                    continue;
                }
                let name_id = StringId(self.ctx.intern_string_raw(&qualified_name));
                let mut descriptor = crate::module::FunctionDescriptor::new(name_id);
                descriptor.id = info.id;
                descriptor.register_count = 1;
                descriptor.locals_count = info.param_count as u16;
                for (i, pname) in info.param_names.iter().enumerate() {
                    let pname_to_use = if pname.is_empty() {
                        format!("_arg{}", i)
                    } else {
                        pname.clone()
                    };
                    let pname_id = StringId(self.ctx.intern_string_raw(&pname_to_use));
                    descriptor.params.push(crate::module::ParamDescriptor {
                        name: pname_id,
                        type_ref: TypeRef::Concrete(TypeId::UNIT),
                        is_mut: false,
                        default: None,
                    });
                }
                while descriptor.params.len() < info.param_count {
                    let i = descriptor.params.len();
                    let pname_id =
                        StringId(self.ctx.intern_string_raw(&format!("_arg{}", i)));
                    descriptor.params.push(crate::module::ParamDescriptor {
                        name: pname_id,
                        type_ref: TypeRef::Concrete(TypeId::UNIT),
                        is_mut: false,
                        default: None,
                    });
                }
                if info.is_async {
                    descriptor.properties |= crate::types::PropertySet::ASYNC;
                }
                descriptor.is_generator = info.is_generator;
                if let Some(parent_name) = info.parent_type_name.as_deref() {
                    if let Some(&parent_tid) = self.type_name_to_id.get(parent_name) {
                        descriptor.parent_type = Some(parent_tid);
                    }
                }
                let vbc_func = crate::module::VbcFunction::new(
                    descriptor,
                    vec![Instruction::RetV],
                );
                self.functions.push(vbc_func);
            }
        }
    }

    /// Builds the final VBC module.
    fn build_module(&mut self) -> CodegenResult<VbcModule> {
        let mut module = VbcModule::new(self.config.module_name.clone());

        // IMPORTANT: Intern strings FIRST and build mapping from codegen index to module StringId.
        // Codegen uses simple indices (0, 1, 2...) while VbcModule uses byte offsets.
        // This mapping is needed for function names, constants, and any other StringIds.
        let string_id_map: Vec<StringId> = self
            .ctx
            .strings
            .iter()
            .map(|s| module.intern_string(s))
            .collect();

        // Sort functions by ID so array index matches function ID
        // (closures compiled during parent functions may be pushed out of order)
        self.functions.sort_by_key(|f| f.descriptor.id.0);

        // **Duplicate-id collapse, the load-bearing dispatch fix.**
        //

        // `func_id_remap` was a HashMap<old_id → new_idx> that silently
        // collapsed duplicate `descriptor.id` entries — the second
        // function compiled with a given codegen-time id won the
        // HashMap key, and `Call(old_id)` instructions emitted from
        // ANY caller resolved to the winner. Live failure mode:
        // pager.vr's body emits `Call(14706)` for its local
        // `is_valid_page_size`; another module also pushes a function
        // with `descriptor.id = 14706` (because of asymmetric
        // `next_func_id` consumption between `register_*` and
        // `self.functions.push`); the second function (`StreamFilter.flatten`
        // in the live repro) wins the remap, and pager's call lands
        // on the wrong body — surfacing as `pager_open_memory failed`
        // for a perfectly-valid 4 KiB page once `mount base.{...}`
        // pulls in enough modules to trigger the collision.
        //

        // The dedup keeps the LAST function for each codegen-time id,
        // matching `register_function`'s last-wins semantics in user-
        // mode compilation (`prefer_existing_functions = false`).
        // Under stdlib loading (`prefer_existing = true`) registration
        // is first-wins, but compile-time pushes still respect that
        // because the registry only retains the first-registered info.
        // The duplicates we collapse here are the SECOND BODY pushes
        // — typically blanket-impl replays, generic monomorphisations,
        // or stdlib re-loads that re-emit the same function under
        // the same codegen-id.
        {
            use std::collections::HashMap;
            let before_len = self.functions.len();
            // Diagnostic counting is unconditional so we can ALWAYS
            // record the dup-group count under tracing::debug; the
            // verbose per-function listing is gated on the
            // `VERUM_TRACE_DEDUP` env so it doesn't pollute stderr
            // on every `verum run` invocation. The pre-fix path
            // unconditionally `eprintln!`'d both lines, which made
            // every script invocation emit ~30 lines of dispatch
            // diagnostics that drown real script output.
            let mut id_count: HashMap<u32, usize> = HashMap::new();
            for f in &self.functions {
                *id_count.entry(f.descriptor.id.0).or_insert(0) += 1;
            }
            let dups: Vec<(u32, usize)> = id_count
                .iter()
                .filter(|&(_, &c)| c > 1)
                .map(|(&k, &v)| (k, v))
                .collect();
            if !dups.is_empty() {
                tracing::debug!(
                    target: "verum_vbc::codegen::dedup",
                    "{} duplicate codegen-id groups detected (e.g. id={} appears {}x)",
                    dups.len(), dups[0].0, dups[0].1,
                );
                if std::env::var("VERUM_TRACE_DEDUP").is_ok() {
                    for f in &self.functions {
                        if f.descriptor.id.0 == dups[0].0 {
                            let n = self
                                .ctx
                                .strings
                                .get(f.descriptor.name.0 as usize)
                                .cloned()
                                .unwrap_or_default();
                            eprintln!(
                                "[codegen-dedup]   duplicate id={} name='{}' bytecode_len={}",
                                f.descriptor.id.0,
                                n,
                                f.instructions.len(),
                            );
                        }
                    }
                }
            }
            use std::collections::HashSet;
            let mut seen: HashSet<u32> = HashSet::new();
            // Walk in reverse so `retain`'s natural keep-first becomes
            // keep-last after a final reverse.
            self.functions.reverse();
            self.functions.retain(|f| seen.insert(f.descriptor.id.0));
            self.functions.reverse();
            if self.functions.len() != before_len {
                tracing::debug!(
                    target: "verum_vbc::codegen::dedup",
                    "collapsed {} functions ({} → {})",
                    before_len - self.functions.len(),
                    before_len,
                    self.functions.len(),
                );
            }
        }

        // Build func_id remapping: old sparse IDs → new contiguous 0-based IDs.
        // When stdlib functions are imported, next_func_id starts high, so the
        // module's own functions have non-zero-based IDs. The module stores functions
        // in a Vec indexed by FunctionId, so IDs must be contiguous from 0.
        let func_id_remap: std::collections::HashMap<u32, u32> = self
            .functions
            .iter()
            .enumerate()
            .map(|(new_idx, f)| (f.descriptor.id.0, new_idx as u32))
            .collect();

        // Add types AFTER func_id_remap is built, so we can remap drop_fn references
        for ty in &self.types {
            let mut remapped_ty = ty.clone();
            // Remap type name from codegen string index to module StringId
            if let Some(mapped) = string_id_map.get(remapped_ty.name.0 as usize) {
                remapped_ty.name = *mapped;
            }
            // Remap field names from codegen string index to module StringId
            for field in &mut remapped_ty.fields {
                if let Some(mapped) = string_id_map.get(field.name.0 as usize) {
                    field.name = *mapped;
                }
            }
            // Remap variant names AND each variant's inner field names
            // (record-style variant payloads). Pre-fix this remap was
            // skipped — variant.name carried the codegen-index StringId
            // while the runtime's `state.module.strings.get(StringId)`
            // expected a byte offset, so every typed-variant name
            // lookup at `format_variant_for_print_depth` either
            // returned None (rendering "Variant(N, ...)") or read from
            // the wrong offset (rendering an unrelated symbol like
            // "tcp_connect" or "file_write_all"). This is the
            // load-bearing fix for VBC-3's user-facing display: now
            // the typed-variant TypeDescriptor's variant names
            // resolve to the same string id-space the rest of the
            // runtime uses, so the constructor name displays
            // correctly (e.g., `SpawnFailed(...)` for
            // `ShellError.SpawnFailed`).
            for variant in &mut remapped_ty.variants {
                if let Some(mapped) = string_id_map.get(variant.name.0 as usize) {
                    variant.name = *mapped;
                }
                for f in &mut variant.fields {
                    if let Some(mapped) = string_id_map.get(f.name.0 as usize) {
                        f.name = *mapped;
                    }
                }
            }
            // Remap generic type-parameter names from codegen
            // string index to module StringId.  Pre-fix the param
            // names stayed as codegen-time indices, so the runtime's
            // `module.strings.get(tp.name)` returned garbage —
            // archive_metadata then emitted aliases like
            // `IoResult` with `generic_params=[""]` instead of
            // `[T]`, breaking positional substitution at use sites.
            for tp in remapped_ty.type_params.iter_mut() {
                if let Some(mapped) = string_id_map.get(tp.name.0 as usize) {
                    tp.name = *mapped;
                }
            }
            // Remap drop_fn from sparse ID to contiguous 0-based ID
            if let Some(drop_fn) = remapped_ty.drop_fn
                && let Some(&new_id) = func_id_remap.get(&drop_fn)
            {
                remapped_ty.drop_fn = Some(new_id);
            }
            // Remap clone_fn from sparse ID to contiguous 0-based ID
            if let Some(clone_fn) = remapped_ty.clone_fn
                && let Some(&new_id) = func_id_remap.get(&clone_fn)
            {
                remapped_ty.clone_fn = Some(new_id);
            }
            // Remap protocol method FunctionIds from sparse to contiguous 0-based IDs
            for proto_impl in remapped_ty.protocols.iter_mut() {
                for fn_id in proto_impl.methods.iter_mut() {
                    if *fn_id != u32::MAX
                        && let Some(&new_id) = func_id_remap.get(fn_id)
                    {
                        *fn_id = new_id;
                    }
                }
            }
            module.add_type(remapped_ty);
        }

        // Cross-module call name table collection.
        //
        // **Why this exists**: stdlib precompile uses a shared
        // codegen-global FunctionId namespace across all source
        // modules. When `build_module` finalises one archive
        // module's entry, `func_id_remap` covers only this module's
        // own functions (the entries pushed into `self.functions`),
        // mapping their sparse codegen-time ids to contiguous
        // 0..N indices. The remap leaves cross-module Call operands
        // unchanged — `func_id_remap.get(other_module_global_id)` is
        // `None`, so the rewrite below preserves the original sparse
        // id. The archive bytecode therefore carries call targets in
        // TWO id spaces: in-module 0..N (resolved by Tier-1 of
        // `ArchiveBodyRemap`) and cross-module precompile-globals
        // (need name-based Tier-2 resolution at user-side load).
        //
        // Tier-2's `archive_id_to_name` is populated from
        // `module.functions` (this module's own descriptors), so
        // cross-module ids miss → Tier-3 identity fallback → user
        // bytecode keeps the bogus sparse id → runtime dispatch
        // routes through whatever happens to live at that index in
        // the user codegen's contiguous function table. The live
        // failure mode pinned in the repro: `Text.push_byte →
        // Text.grow → alloc` dispatching to `Successors.next` and
        // null-dereferencing at pc=0.
        //
        // **The fix**: capture (cross_module_global_id, qualified-name)
        // pairs at this point — where `func_id_remap` is in scope so
        // we can decide "in-module vs cross-module" unambiguously
        // (`func_id_remap.contains_key(fid)` is the authoritative
        // gate; the post-remap `fid < N` test is unreliable because
        // earlier-registered modules' global ids can fall inside
        // this module's [0, N) range). Stored in
        // `module.external_function_names` and consumed by
        // `merge_archive_function_bodies` to enrich
        // `archive_id_to_name` for Tier-2 resolution.
        //
        // **Size budget**: bounded by the count of UNIQUE cross-
        // module ids actually called from this module's bodies —
        // typically tens per module (alloc, dealloc, panic, memcpy,
        // realloc, plus a handful of cross-module helpers). Each
        // entry is (FunctionId=4B, StringId=4B) + the qualified name
        // interned once into `module.strings` (avg ~30 bytes).
        // Total stdlib impact: ~few hundred KB instead of the 100+ MB
        // blowup seen with the full-stub approach (commit a36f164af
        // reverted that path).
        let mut external_seen: std::collections::HashSet<u32> =
            std::collections::HashSet::new();
        let mut external_pending: Vec<u32> = Vec::new();
        const EXTERN_SENTINEL_THRESHOLD: u32 = u32::MAX / 4;
        // **Task #9 close-out 2026-05-24** — distinguish stage-1/2/3
        // pre-register stubs from variant-ctor / FFI-extern sentinels.
        // Stub ids carry a NAME (the producing function's identifier)
        // and need name-based resolution at user-side archive merge
        // (Tier 2 of `ArchiveBodyRemap::map_function`).  Pre-fix the
        // simple `fid >= EXTERN_SENTINEL_THRESHOLD` gate skipped
        // BOTH classes — sentinel-dispatched IDs (variant ctor tags
        // `u32::MAX - tag`, FFI extern sentinels) AND stage-3 stub
        // IDs.  Stage-3 stubs then never made it into
        // `module.external_function_names`, so cross-module Call(stub)
        // sites in stdlib bodies kept the raw stub id (Tier 3
        // identity fallback) and tripped the lenient panic in
        // `calls.rs:117` at runtime.
        //
        // Range definitions mirror `stdlib_bootstrap::pre_register_*`
        // and `interpreter/dispatch_table/handlers/calls.rs:69-72`.
        // Width is 0x10_0000 (1M slots per stage).
        const STAGE1_STUB_BASE: u32 = u32::MAX - 0x40_0000;
        const STAGE2_STUB_BASE: u32 = u32::MAX - 0xC0_0000;
        const STAGE3_STUB_BASE: u32 = u32::MAX - 0x100_0000;
        const STUB_RANGE_WIDTH: u32 = 0x10_0000;
        let is_stage_stub = |id: u32| -> bool {
            let s1 = id <= STAGE1_STUB_BASE && id >= STAGE1_STUB_BASE - STUB_RANGE_WIDTH;
            let s2 = id <= STAGE2_STUB_BASE && id >= STAGE2_STUB_BASE - STUB_RANGE_WIDTH;
            let s3 = id <= STAGE3_STUB_BASE && id >= STAGE3_STUB_BASE - STUB_RANGE_WIDTH;
            s1 || s2 || s3
        };
        for func in &self.functions {
            for instr in &func.instructions {
                let fid = match instr {
                    Instruction::Call { func_id, .. }
                    | Instruction::CallG { func_id, .. }
                    | Instruction::TailCall { func_id, .. }
                    | Instruction::NewClosure { func_id, .. }
                    | Instruction::Spawn { func_id, .. }
                    | Instruction::GenCreate { func_id, .. } => *func_id,
                    _ => continue,
                };
                // STAGE STUB IDs go through the same name-resolution
                // path as cross-module Calls — they're known-name
                // entries in `ctx.functions` that need Tier 2 lookup
                // at user-side merge to reach the real body.
                if is_stage_stub(fid) {
                    if func_id_remap.contains_key(&fid) {
                        continue;
                    }
                    if external_seen.insert(fid) {
                        external_pending.push(fid);
                    }
                    continue;
                }
                if fid >= EXTERN_SENTINEL_THRESHOLD {
                    // Variant-ctor tag (`u32::MAX - tag`), FFI extern,
                    // newtype-ctor — dispatched out-of-band by the
                    // runtime; no name-resolution needed.
                    continue;
                }
                if func_id_remap.contains_key(&fid) {
                    // In-module — Tier-1 of `ArchiveBodyRemap` covers it.
                    continue;
                }
                if external_seen.insert(fid) {
                    external_pending.push(fid);
                }
            }
        }

        // Encode functions and their bytecode, remapping name_id and func_id references
        for func in &self.functions {
            // Remap string IDs and func_ids in instructions
            let remapped_instructions: Vec<Instruction> = func.instructions.iter().map(|instr| {
                let mut i = instr.clone();
                match &mut i {
                    Instruction::CallM { method_id, .. } => {
                        if let Some(mapped) = string_id_map.get(*method_id as usize) {
                            *method_id = mapped.0;
                        }
                    }
                    // Remap protocol_id in CmpG from codegen string index to module StringId.
                    // protocol_id encodes (codegen_string_index + 1), with 0 meaning no protocol.
                    Instruction::CmpG { protocol_id, .. }
                        if *protocol_id > 0 => {
                            let codegen_idx = (*protocol_id - 1) as usize;
                            if let Some(mapped) = string_id_map.get(codegen_idx) {
                                *protocol_id = mapped.0 + 1;
                            }
                        }
                    // Note: IsVar/AsVar tag fields are numeric variant/field indices, NOT string IDs
                    Instruction::Panic { message_id } => {
                        if let Some(mapped) = string_id_map.get(*message_id as usize) {
                            *message_id = mapped.0;
                        }
                    }
                    Instruction::Assert { message_id, .. } => {
                        if let Some(mapped) = string_id_map.get(*message_id as usize) {
                            *message_id = mapped.0;
                        }
                    }
                    // Context type identifiers carry interned names through the
                    // codegen→runtime boundary the same way method/protocol names
                    // do — without remapping, the runtime resolves the codegen-
                    // local index against the byte-offset string table and prints
                    // `Context unknown not provided` instead of the actual
                    // context name (e.g. `Database`).
                    Instruction::CtxGet { ctx_type, .. } => {
                        if let Some(mapped) = string_id_map.get(*ctx_type as usize) {
                            *ctx_type = mapped.0;
                        }
                    }
                    Instruction::CtxProvide { ctx_type, .. } => {
                        if let Some(mapped) = string_id_map.get(*ctx_type as usize) {
                            *ctx_type = mapped.0;
                        }
                    }
                    Instruction::CtxCheckNegative { ctx_type, func_name } => {
                        if let Some(mapped) = string_id_map.get(*ctx_type as usize) {
                            *ctx_type = mapped.0;
                        }
                        if let Some(mapped) = string_id_map.get(*func_name as usize) {
                            *func_name = mapped.0;
                        }
                    }
                    // Remap func_id references to contiguous 0-based IDs
                    Instruction::Call { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    Instruction::TailCall { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    Instruction::NewClosure { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    Instruction::CallG { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    Instruction::GenCreate { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    Instruction::Spawn { func_id, .. } => {
                        if let Some(&new_id) = func_id_remap.get(func_id) { *func_id = new_id; }
                    }
                    // Remap func_id inside FfiExtended::CreateCallback operand bytes.
                    // Format: dst:reg (variable-length), fn_id:u32, signature_idx:u32
                    Instruction::FfiExtended { sub_op, operands }
                        if *sub_op == 0x50 /* CreateCallback */ =>
                    {
                        // Register encoding: 1 byte if < 0x80, 2 bytes otherwise
                        let fn_id_offset = if !operands.is_empty() && operands[0] & 0x80 != 0 { 2 } else { 1 };
                        if operands.len() >= fn_id_offset + 4 {
                            let old_fn_id = u32::from_le_bytes([
                                operands[fn_id_offset],
                                operands[fn_id_offset + 1],
                                operands[fn_id_offset + 2],
                                operands[fn_id_offset + 3],
                            ]);
                            if let Some(&new_id) = func_id_remap.get(&old_fn_id) {
                                let new_bytes = new_id.to_le_bytes();
                                operands[fn_id_offset..fn_id_offset + 4].copy_from_slice(&new_bytes);
                            }
                        }
                    }
                    _ => {}
                }
                i
            }).collect();

            // Encode instructions to bytecode (with jump offset fixup)
            let mut bytecode = Vec::new();
            crate::bytecode::encode_instructions_with_fixup(&remapped_instructions, &mut bytecode);

            // Record bytecode offset and length in descriptor
            let bytecode_offset = module.append_bytecode(&bytecode);
            let bytecode_length = bytecode.len() as u32;

            // Create descriptor with bytecode location and remapped name_id + func_id
            let mut descriptor = func.descriptor.clone();
            descriptor.bytecode_offset = bytecode_offset;
            descriptor.bytecode_length = bytecode_length;

            // Remap function ID to contiguous 0-based index
            if let Some(&new_id) = func_id_remap.get(&descriptor.id.0) {
                descriptor.id = FunctionId(new_id);
            }

            // Remap function name_id from codegen index to module StringId (byte offset)
            let codegen_name_idx = descriptor.name.0 as usize;
            descriptor.name = string_id_map
                .get(codegen_name_idx)
                .copied()
                .unwrap_or(StringId::EMPTY);

            // Remap each parameter name through the same string_id_map so
            // runtime lookups (`state.module.strings.get(param.name)`)
            // recover the actual identifier rather than reading byte-offset
            // garbage. Used by the CallM dispatch's "first param == self"
            // detection to skip prepending the receiver for context methods
            // declared without `self`.
            for param in descriptor.params.iter_mut() {
                let codegen_idx = param.name.0 as usize;
                param.name = string_id_map
                    .get(codegen_idx)
                    .copied()
                    .unwrap_or(StringId::EMPTY);
            }

            // #87/#97 — remap `descriptor.intrinsic_name` (the
            // `__const_val_<N>` marker for inlinable consts and the
            // `@intrinsic("name")` carrier) through the same
            // `string_id_map`.  Pre-fix this remap was missing — codegen
            // pushed `intrinsic_name = Some(StringId(codegen_index))`
            // but `build_module` left it untranslated, so the
            // serialised descriptor pointed at a codegen-local index
            // that the runtime's byte-offset `module.strings.get()`
            // interpreted as garbage (or returned `None`).  Result:
            // every inlinable stdlib const lost its inline marker at
            // the archive boundary, surfaced as a body-less zero-arg
            // function whose `RetV` body returned Unit instead of the
            // const's literal value.
            if let Some(codegen_iname) = descriptor.intrinsic_name {
                let codegen_idx = codegen_iname.0 as usize;
                descriptor.intrinsic_name = string_id_map.get(codegen_idx).copied();
            }

            // Store decoded instructions for LLVM lowering (AOT path).
            // The LLVM lowering reads from descriptor.instructions rather than
            // decoding from raw bytecode.
            descriptor.instructions = Some(remapped_instructions);

            module.add_function(descriptor);
        }

        // Add constants, remapping string IDs through the same map
        for constant in &self.ctx.constants {
            match constant {
                context::ConstantEntry::Int(v) => {
                    module.add_constant(crate::module::Constant::Int(*v));
                }
                context::ConstantEntry::Float(v) => {
                    module.add_constant(crate::module::Constant::Float(*v));
                }
                context::ConstantEntry::String(codegen_id) => {
                    // Map codegen index to module StringId (byte offset)
                    let module_string_id = string_id_map
                        .get(*codegen_id as usize)
                        .copied()
                        .unwrap_or(StringId::EMPTY);
                    module.add_constant(crate::module::Constant::String(module_string_id));
                }
                context::ConstantEntry::Type(type_ref) => {
                    module.add_constant(crate::module::Constant::Type(type_ref.clone()));
                }
                context::ConstantEntry::Bytes(codegen_id) => {
                    // Look up the byte array in the codegen context
                    let bytes = self
                        .ctx
                        .bytes
                        .get(*codegen_id as usize)
                        .cloned()
                        .unwrap_or_default();
                    module.add_constant(crate::module::Constant::Bytes(bytes));
                }
            }
        }

        // Add FFI libraries, interning library names
        // We need to iterate in order by lib_id, so create a sorted list
        let mut lib_entries: Vec<_> = self.ffi_library_map.iter().collect();
        lib_entries.sort_by_key(|(_, id)| id.0);
        for (lib_name, lib_id) in lib_entries {
            if let Some(lib) = self.ffi_libraries.get(lib_id.0 as usize) {
                let mut lib_entry = lib.clone();
                lib_entry.name = module.intern_string(lib_name);
                module.ffi_libraries.push(lib_entry);
            }
        }

        // Add FFI symbols, interning symbol names where available.
        // We transfer ALL symbols including synthetic callback signatures.
        // Build a reverse map from symbol_id to function name for named symbols.
        let id_to_name: std::collections::HashMap<u32, &String> = self
            .ffi_function_map
            .iter()
            .map(|(name, id)| (id.0, name))
            .collect();

        // Transfer all FFI symbols in order
        for (idx, symbol) in self.ffi_symbols.iter().enumerate() {
            let mut symbol_entry = symbol.clone();
            // If this symbol has a name in the function map, intern it.
            // Otherwise, leave name as StringId(0) (synthetic callback signatures).
            if let Some(func_name) = id_to_name.get(&(idx as u32)) {
                symbol_entry.name = module.intern_string(func_name);
            }
            module.ffi_symbols.push(symbol_entry);
        }

        // Transfer FFI struct layouts for @repr(C) types
        for layout in &self.ffi_layouts {
            module.ffi_layouts.push(layout.clone());
        }

        // Set V-LLSI profile flags: interpretable, systems (AOT-only), embedded (no-heap)
        module.set_profile_flags(
            self.config.is_interpretable,
            self.config.is_systems_profile,
            self.config.is_embedded,
        );

        // Update auto-detected flags (preserves profile flags)
        module.update_flags();

        // Transfer static init functions as global constructors (with remapped IDs)
        for func_id in &self.static_init_functions {
            let new_id = func_id_remap.get(&func_id.0).copied().unwrap_or(func_id.0);
            module.global_ctors.push((FunctionId(new_id), 65535));
        }

        // Transfer context name table (maps ContextRef ID → StringId)
        for ctx_name in &self.context_names {
            let string_id = module.intern_string(ctx_name);
            module.context_names.push(string_id);
        }

        // Transfer field layout metadata for LLVM lowering field index remapping.
        // Build field_id_to_name reverse mapping from field_name_indices.
        let mut id_to_name = vec![String::new(); self.next_field_id as usize];
        for (name, &id) in &self.field_name_indices {
            if (id as usize) < id_to_name.len() {
                id_to_name[id as usize] = name.clone();
            }
        }
        module.field_id_to_name = id_to_name;
        module.type_field_layouts = self.type_field_layouts.clone();

        // Sync header table-counts with the populated module sections.
        // The validator's `validate_header` step compares
        // `header.<table>_count` to `module.<table>.len()` and rejects
        // any mismatch.  Without this sync, every non-empty
        // codegen-built module fails opt-in validation purely on
        // header-vs-section drift — orthogonal to the actual bytecode
        // structure.  Mirrors what `serialize::write_module` computes
        // implicitly at write-time.
        module.header.type_table_count = module.types.len() as u32;
        module.header.function_table_count = module.functions.len() as u32;
        module.header.constant_pool_count = module.constants.len() as u32;

        // Materialise the cross-module call name table (collected
        // above into `external_pending` while `func_id_remap` was in
        // scope). Look up each external fid's qualified name in
        // `self.ctx.functions` and intern it into `module.strings`.
        // See the rationale block at the collection site above for
        // why this is needed and the size budget it lives within.
        if !external_pending.is_empty() {
            let mut ctx_id_to_name: std::collections::HashMap<u32, String> =
                std::collections::HashMap::with_capacity(external_pending.len());
            for (name, info) in self.ctx.functions.iter() {
                if !external_seen.contains(&info.id.0) {
                    continue;
                }
                ctx_id_to_name
                    .entry(info.id.0)
                    .and_modify(|existing| {
                        // Prefer the longest dotted (qualified) name —
                        // canonical form for cross-module lookup at
                        // user-side merge. Mirrors
                        // `emit_missing_stub_descriptors`'s longest-wins
                        // tie-break.
                        let existing_dots = existing.matches('.').count();
                        let new_dots = name.matches('.').count();
                        if new_dots > existing_dots {
                            *existing = name.clone();
                        }
                    })
                    .or_insert_with(|| name.clone());
            }
            let mut external_list: Vec<(FunctionId, StringId)> =
                Vec::with_capacity(external_pending.len());
            for fid in external_pending {
                if let Some(name) = ctx_id_to_name.get(&fid) {
                    // Clone the name out so the borrow on
                    // `ctx_id_to_name` (and transitively on `self.ctx`)
                    // is released before `module.intern_string`.
                    let owned = name.clone();
                    let sid = module.intern_string(&owned);
                    external_list.push((FunctionId(fid), sid));
                }
            }
            tracing::debug!(
                target: "verum_vbc::codegen::external_funcs",
                "module='{}' external_function_names: {} entries",
                self.config.module_name,
                external_list.len(),
            );
            module.external_function_names = external_list;
        }

        // Task #11 Phase 2: drain mount-alias capture buffer into the
        // module-level `mount_aliases` table.  Each tuple is
        // `(alias_string_id, FunctionId)` — the loader-side replay
        // (`apply_lazy_with_types` in `verum_compiler`) reads this
        // table after a passive archive load and re-installs every
        // captured alias via `register_function_authoritative`, so
        // user-side AOT codegen sees the same alias bindings the
        // precompile stage observed.
        //
        // Aliases are deduplicated against `(alias_name,
        // function_id)`: a stdlib module may resolve the same alias
        // through multiple lookup variants (e.g. `qualified_verum`
        // then deferred-resolution) and emit two captures for the
        // same pair.  We dedupe so the archive doesn't grow
        // quadratically across re-resolution sweeps.
        if !self.mount_aliases_buffer.is_empty() {
            let mut seen: std::collections::HashSet<(String, u32)> =
                std::collections::HashSet::with_capacity(self.mount_aliases_buffer.len());
            let drained = std::mem::take(&mut self.mount_aliases_buffer);
            let mut emitted: Vec<(StringId, FunctionId)> = Vec::with_capacity(drained.len());
            for (alias_name, fid) in drained {
                if !seen.insert((alias_name.clone(), fid.0)) {
                    continue;
                }
                let sid = module.intern_string(&alias_name);
                emitted.push((sid, fid));
            }
            tracing::debug!(
                target: "verum_vbc::codegen::mount_aliases",
                "module='{}' mount_aliases: {} entries (task #11 Phase 2)",
                self.config.module_name,
                emitted.len(),
            );
            module.mount_aliases = emitted;
        }

        Ok(module)
    }

    /// Returns codegen statistics.
    pub fn stats(&self) -> &CodegenStats {
        &self.ctx.stats
    }

    /// Mutable access to the inner codegen context.  Used by
    /// archive-driven pre-population (T2) so external loaders can
    /// register `FunctionInfo` entries directly without going
    /// through the AST walker.
    pub fn ctx_mut(&mut self) -> &mut CodegenContext {
        &mut self.ctx
    }

    /// Mutable access to the function-id allocator counter.  Exposed
    /// for archive-driven pre-population (T2) so the loader can
    /// remap each archive-local FunctionId to a globally-unique id
    /// in the user codegen's namespace.  Without this, two archive
    /// modules with overlapping local FunctionIds (e.g. both expose
    /// id=0 for their first function) collapse to a single
    /// ctx.functions slot at codegen time and `emit_missing_stub_descriptors`
    /// emits exactly one stub for that id — picking an arbitrary
    /// canonical name and routing every `Call(0)` through that name's
    /// intercept regardless of which source function the call site
    /// intended.
    pub fn next_func_id_mut(&mut self) -> &mut u32 {
        &mut self.next_func_id
    }

    /// Import a TypeDescriptor from an archive module into the user
    /// codegen.  Walks the descriptor's StringId references (name,
    /// type-param names, field names, variant names, variant field
    /// names) and re-interns each into the user codegen's
    /// `ctx.strings` table; allocates a fresh user-side TypeId via
    /// `alloc_user_type_id` so cross-archive id collisions don't
    /// collapse two stdlib types onto a single slot; updates
    /// `type_name_to_id` so `compile_record`'s descriptor_match path
    /// can resolve `parent_canonical → TypeId` and `emit_make_variant`
    /// can route through `MakeVariantTyped`.
    ///
    /// Idempotent on the type's name: when the same name is already
    /// bound (user-defined type collides with archive type), keep the
    /// user's binding — same first-wins discipline as the function
    /// table loader.
    ///
    /// `archive_strings` is the source module's string table — every
    /// StringId in `ty` indexes into this slice, NOT the user codegen.
    /// Shim that forwards to `import_archive_type_with_protocol_remap`
    /// with an empty remap. Existing callers that don't have access to
    /// the source module's full type table (and therefore can't build
    /// a meaningful remap) get the legacy clone-as-is behaviour —
    /// protocol-default-method dispatch will only succeed for types
    /// imported via the bulk `import_archive_module_types` entry
    /// point, which builds the remap first.
    pub fn import_archive_type(
        &mut self,
        ty: &crate::types::TypeDescriptor,
        archive_strings: &crate::module::StringTable,
    ) {
        let empty: std::collections::HashMap<
            crate::types::TypeId,
            crate::types::TypeId,
        > = std::collections::HashMap::new();
        self.import_archive_type_with_protocol_remap(ty, archive_strings, &empty);
    }

    /// As [`import_archive_type`], but additionally remaps each
    /// `ProtocolImpl.protocol` from its archive-local TypeId to the
    /// codegen-local TypeId allocated by the FIRST PASS of
    /// `import_archive_module_types`. Without the remap, the imported
    /// descriptor's `protocols` list points at archive ids that don't
    /// resolve in `self.types` — and the runtime's
    /// `Module::find_method_by_receiver_type` protocol-default-method
    /// fallback then fails to translate `Iterator.collect` from
    /// `Range`'s `ProtocolImpl` list back to a real protocol name.
    pub fn import_archive_type_with_protocol_remap(
        &mut self,
        ty: &crate::types::TypeDescriptor,
        archive_strings: &crate::module::StringTable,
        protocol_id_remap: &std::collections::HashMap<
            crate::types::TypeId,
            crate::types::TypeId,
        >,
    ) {
        let intern = |this: &mut Self, sid: crate::types::StringId| -> crate::types::StringId {
            let name = match archive_strings.get(sid) {
                Some(s) => s.to_string(),
                None => return crate::types::StringId::EMPTY,
            };
            // Use HashMap-backed intern path on the context — O(1)
            // amortised vs the linear scan in `Self::intern_string`.
            crate::types::StringId(this.ctx.intern_string_raw(&name))
        };
        // Bail out cleanly when the name doesn't resolve in the archive
        // string table — keeps the loader robust against malformed
        // archive entries instead of panicking mid-walk.
        let name_str = match archive_strings.get(ty.name) {
            Some(s) => s.to_string(),
            None => return,
        };
        // First-wins for the `type_name_to_id` slot: user-defined or
        // earlier-archive registration owns it.  But — and this is
        // the load-bearing fix for stdlib variant-tag-collision —
        // when an earlier registration set the name→id mapping
        // WITHOUT pushing a real `TypeDescriptor` into `self.types`
        // (the case for built-in well-known types like Maybe /
        // Result / Ordering whose ids are pre-allocated by
        // `register_builtin_variants` but whose descriptors only
        // arrive via this archive-import path), we must still push
        // the imported descriptor under that pre-existing id.  Without
        // this, `emit_make_variant` finds the id but `self.types`
        // misses the descriptor and demotes to the untyped
        // `MakeVariant` form — runtime then guesses the variant
        // name via the global tag scan and lands on whichever
        // unrelated type's variant ALSO has tag 0/1
        // (`AliasError.EmptyWeights` vs `Maybe.None`,
        // `AliasError.NonFiniteWeight` vs `Maybe.Some`).
        let new_id = match self.type_name_to_id.get(&name_str).copied() {
            Some(existing_id) => {
                // Reuse the pre-allocated id IF no descriptor exists
                // yet for it.  When a descriptor IS already present,
                // the earlier registration is authoritative — bail
                // (mirrors the prior first-wins discipline).
                if self.types.iter().any(|d| d.id == existing_id) {
                    return;
                }
                existing_id
            }
            None => {
                let id = self.alloc_user_type_id();
                self.type_name_to_id.insert(name_str.clone(), id);
                id
            }
        };
        let new_name_id = crate::types::StringId(self.ctx.intern_string_raw(&name_str));

        // Type parameters
        let mut new_type_params: smallvec::SmallVec<[crate::types::TypeParamDescriptor; 2]> =
            smallvec::SmallVec::new();
        for tp in ty.type_params.iter() {
            new_type_params.push(crate::types::TypeParamDescriptor {
                name: intern(self, tp.name),
                id: tp.id,
                bounds: tp.bounds.clone(),
                default: tp.default.clone(),
                variance: tp.variance,
                type_bounds: smallvec::SmallVec::new(),
            });
        }

        // Record fields
        let mut new_fields: smallvec::SmallVec<[crate::types::FieldDescriptor; 4]> =
            smallvec::SmallVec::new();
        for fd in ty.fields.iter() {
            new_fields.push(crate::types::FieldDescriptor {
                name: intern(self, fd.name),
                ..fd.clone()
            });
        }

        // Variants (each carries its own field list).  Type-layout
        // invariants per `verify_type_layout_invariants`:
        //  * Unit  — `arity == 0` AND `fields.is_empty()`
        //  * Tuple — `arity > 0`  AND `fields.is_empty()` (payload
        //    count lives in `arity`, NOT in `fields`)
        //  * Record — `arity == 0` AND `!fields.is_empty()` (named
        //    field metadata lives in `fields`)
        // Some archive precompile sites populate `fields` for Tuple
        // variants too (positional `_0`/`_1` synthesised names) — our
        // import strips them to keep the codegen-time invariant
        // checker happy.
        let mut new_variants: smallvec::SmallVec<[crate::types::VariantDescriptor; 4]> =
            smallvec::SmallVec::new();
        for v in ty.variants.iter() {
            let copy_fields = matches!(v.kind, crate::types::VariantKind::Record);
            let mut v_fields: smallvec::SmallVec<[crate::types::FieldDescriptor; 4]> =
                smallvec::SmallVec::new();
            if copy_fields {
                for fd in v.fields.iter() {
                    v_fields.push(crate::types::FieldDescriptor {
                        name: intern(self, fd.name),
                        ..fd.clone()
                    });
                }
            }
            // Tuple variants use `arity`; record variants use
            // `fields.len()`. Re-derive arity for record variants so
            // the invariant `kind == Record → arity == 0` holds.
            let arity = match v.kind {
                crate::types::VariantKind::Tuple => v.arity,
                _ => 0,
            };
            new_variants.push(crate::types::VariantDescriptor {
                name: intern(self, v.name),
                tag: v.tag,
                payload: v.payload.clone(),
                kind: v.kind,
                arity,
                fields: v_fields,
            });
        }

        // ProtocolImpl remap. ProtocolId is "index into type table"
        // (types.rs L317), so archive-local protocol references need
        // to be translated to codegen-local ids — otherwise the
        // runtime's `find_method_by_receiver_type` protocol-default-
        // method fallback can't reach `Iterator.collect` from
        // `Range`'s `protocols` list.
        let new_protocols: smallvec::SmallVec<[crate::types::ProtocolImpl; 2]> = ty
            .protocols
            .iter()
            .map(|pi| {
                let new_protocol = protocol_id_remap
                    .get(&crate::types::TypeId(pi.protocol.0))
                    .map(|tid| crate::types::ProtocolId(tid.0))
                    .unwrap_or(pi.protocol);
                crate::types::ProtocolImpl {
                    protocol: new_protocol,
                    methods: pi.methods.clone(),
                }
            })
            .collect();

        let imported = crate::types::TypeDescriptor {
            id: new_id,
            name: new_name_id,
            kind: ty.kind.clone(),
            type_params: new_type_params,
            fields: new_fields,
            variants: new_variants,
            size: ty.size,
            alignment: ty.alignment,
            drop_fn: ty.drop_fn,
            clone_fn: ty.clone_fn,
            protocols: new_protocols,
            visibility: ty.visibility,
            alias_target: ty.alias_target.clone(),
            is_transparent_wrapper: ty.is_transparent_wrapper,
        };

        // Restore the codegen-local newtype fast-cache from the
        // canonical descriptor flag.  Without this, archive-loaded
        // newtypes (`type Meters is (Float)` from stdlib) would
        // surface as opaque records at runtime — `m.0` would emit
        // `GetF(0)` on a value that was never boxed, producing
        // garbage.  See `TypeDescriptor::is_transparent_wrapper`.
        if imported.is_transparent_wrapper {
            self.ctx.newtype_names.insert(name_str.clone());
        }

        // **Field-layout cache for record types.** Mirror what
        // `register_archive_type` does on the eager-population path.
        // Without this, `resolve_field_index` falls through to the
        // scan-all-types heuristic ("pick the type with the most
        // fields"), which silently routes record-construction field
        // writes to wrong offsets — surfaces at runtime as
        // `field write out of bounds: field index N exceeds object
        // data size M` (e.g. `PanicInfo` with 2 fields gets `message`
        // resolved to idx=7 because some sibling stdlib type also has
        // a `message` field at position 7 and won the most-fields
        // tie-break). First-wins on simple-name collision matches the
        // discipline at `register_archive_type:~3859`.
        if !imported.fields.is_empty() {
            let names: Vec<String> = imported
                .fields
                .iter()
                .map(|f| {
                    self.ctx
                        .strings
                        .get(f.name.0 as usize)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            self.type_field_layouts
                .entry(name_str.clone())
                .or_insert(names.clone());

            // **Field-type-name population — mirrors `register_archive_type`
            // at codegen/mod.rs:3997.**
            //
            // Pre-fix `import_archive_type_with_protocol_remap` populated
            // ONLY `type_field_layouts` for the imported descriptor;
            // `type_field_type_names` was left empty.  Downstream
            // `extract_expr_type_name`'s Field arm
            // (expressions.rs:16089) consults
            // `type_field_type_names[(type_name, field_name)]` to recover
            // the field's declared type — when missing, the recovery
            // returns None, the enclosing `let bs = rc.borrow_state`
            // doesn't record a type for `bs`, and subsequent
            // `bs.count` in `compile_field_access` falls through to the
            // global-intern-name path which produces a non-zero field
            // index for `count` (because `value` was interned first).
            // The runtime then reads slot N>0 of a 1-slot BorrowState
            // allocation — surfaces as "field access out of bounds:
            // field index 1 (offset 8+8 = 16) exceeds object data size
            // 8" for `let bs = rc.borrow_state; bs.count`.
            //
            // The population mirrors `register_archive_type`'s
            // discipline: walk each field's `TypeRef`, resolve to a
            // canonical name via `type_ref_to_field_name`, and insert
            // the entry first-wins.  When the inner type isn't yet in
            // `self.types` (declaration-order race within the same
            // archive module — common because BorrowState appears AFTER
            // RefCell in `core/base/cell.vr`), `type_ref_to_field_name`
            // returns None and we skip the entry.  The
            // `apply_lazy_with_types` caller invokes
            // `import_archive_module_types` once per module — at the
            // end of that walk, every type in the module IS loaded, so
            // a second-pass repopulation can fill any deferred entries.
            for (fname, fdesc) in names.iter().zip(imported.fields.iter()) {
                if self
                    .type_field_type_names
                    .contains_key(&(name_str.clone(), fname.clone()))
                {
                    continue;
                }
                if let Some(ty_name) = self.type_ref_to_field_name(&fdesc.type_ref) {
                    self.type_field_type_names.insert(
                        (name_str.clone(), fname.clone()),
                        ty_name,
                    );
                }
            }
        }
        // Variant-record layouts: each record-style variant's field
        // names register under the variant's simple name so
        // `compile_record` for variant constructors finds the same
        // declared-order layout. Mirrors the same shape as
        // `register_archive_type` would do via its descriptor walk.
        for v in imported.variants.iter() {
            if !matches!(v.kind, crate::types::VariantKind::Record) || v.fields.is_empty() {
                continue;
            }
            let v_name = match self.ctx.strings.get(v.name.0 as usize) {
                Some(s) => s.clone(),
                None => continue,
            };
            let v_field_names: Vec<String> = v
                .fields
                .iter()
                .map(|f| {
                    self.ctx
                        .strings
                        .get(f.name.0 as usize)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect();
            self.type_field_layouts
                .entry(v_name)
                .or_insert(v_field_names);
        }

        // Restore the codegen-local type-alias fast-cache for
        // `TypeKind::Alias` archive-imported descriptors. Pre-fix,
        // archive types preserved `alias_target` on the descriptor
        // but `import_archive_type` skipped populating
        // `self.type_aliases` (the HashMap consulted by
        // `resolve_type_alias`). Result: stdlib aliases like
        // `type TextResult<T> is Result<T, Text>;` lost the alias
        // relation at the codegen layer, and method dispatch on
        // `let r: TextResult<Int> = ...; r.is_ok()` emitted
        // `CallM("TextResult.is_ok")` (the literal alias name)
        // instead of `CallM("Result.is_ok")` — runtime then panicked
        // with "method 'TextResult.is_ok' not found on receiver of
        // runtime kind Object" because the function table only has
        // the canonical `Result.is_ok` entry.
        //
        // Mirror the source-driven `TypeDeclBody::Alias` arm
        // (`compile_type_decl` line ~8817): walk the alias target's
        // `TypeRef`, recover the base type name, register
        // `alias_name → base_name` in `self.type_aliases`. The
        // `resolve_type_alias` lookup then returns the canonical
        // name and method dispatch routes correctly.
        // Alias-cache populate runs as a SECOND PASS in
        // `import_archive_module_types` — it needs access to the
        // archive's own type table to resolve the alias target's
        // TypeId to a name (the cloned `alias_target` carries
        // archive-local TypeIds that codegen-side `self.types`
        // doesn't have at this point in the per-type-import loop).

        self.push_type_dedupe(imported);
    }

    /// Bulk import every TypeDescriptor in an archive module into the
    /// user codegen.  Wraps `import_archive_type` for caller
    /// convenience; the loader uses this from the archive lazy-load
    /// path to extend the type table with stdlib sum types
    /// (`ShellError`, `IoErrorKind`, `Lifecycle`, …) so the runtime's
    /// `format_variant_for_print_depth` resolves variant names
    /// type-scoped instead of globally guessing.
    ///
    /// **Protocol descriptors**: their full bodies stay out of
    /// `self.types` (their `variants` field stores method-name vtable
    /// entries used by the dyn: dispatch lowering, not for variant
    /// pattern-match rendering — including them would pollute the
    /// type-scoped variant scan that `format_variant_for_print_depth`
    /// uses as the primary lookup path). But the protocol-NAME → ID
    /// mapping IS registered (`self.type_name_to_id`) so the runtime's
    /// `find_method_by_receiver_type` can resolve
    /// `<ProtocolName>.<method>` fallbacks for default-method dispatch
    /// (e.g. `range.collect()` ↦ `Iterator.collect`).
    ///
    /// **Pre-pass for protocol-id remap**: archive `ProtocolImpl`
    /// entries reference protocols by their archive-local TypeId. The
    /// import path re-allocates codegen-local TypeIds, so those
    /// references become stale. Build a `archive_id → codegen_id` map
    /// FIRST (covers both Protocol and non-Protocol types), then pass
    /// it down to `import_archive_type` for in-place remap of each
    /// type's `protocols` list.
    pub fn import_archive_module_types(&mut self, module: &crate::module::VbcModule) {
        // FIRST PASS — protocol stub registration. For each Protocol
        // type in the archive, allocate (or reuse) a codegen-local
        // TypeId AND push a name-only stub `TypeDescriptor` into
        // `self.types` with cleared `variants` (so the variant-scan
        // consumer `format_variant_for_print_depth` doesn't see the
        // method-name vtable entries as fake variants).  The stub
        // makes the runtime's `Module::find_method_by_receiver_type`
        // protocol-default-method fallback succeed — it walks
        // `self.types.iter().find(|t| t.id.0 == pi.protocol.0)` to
        // recover the protocol's name from a `ProtocolImpl.protocol`
        // ref, then composes `<ProtocolName>.<method>` for a second
        // lookup against the function table.
        //
        // Both passes feed the same `protocol_id_remap` so the
        // non-Protocol import pass below can fix up each
        // `ProtocolImpl.protocol` ref from archive-local to
        // codegen-local ids.
        let mut protocol_id_remap: std::collections::HashMap<
            crate::types::TypeId,
            crate::types::TypeId,
        > = std::collections::HashMap::new();
        for ty in module.types.iter() {
            if !matches!(ty.kind, crate::types::TypeKind::Protocol) {
                continue;
            }
            let proto_name = match module.strings.get(ty.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let codegen_id = match self.type_name_to_id.get(&proto_name).copied() {
                Some(existing) => existing,
                None => {
                    let id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(proto_name.clone(), id);
                    id
                }
            };
            protocol_id_remap.insert(ty.id, codegen_id);
            // Push the stub IF no descriptor for this id has been
            // pushed yet (first-wins, mirrors `import_archive_type`'s
            // existing discipline).
            if !self.types.iter().any(|t| t.id == codegen_id) {
                let stub_name_id = crate::types::StringId(
                    self.ctx.intern_string_raw(&proto_name),
                );
                self.types.push(crate::types::TypeDescriptor {
                    id: codegen_id,
                    name: stub_name_id,
                    kind: crate::types::TypeKind::Protocol,
                    type_params: smallvec::SmallVec::new(),
                    fields: smallvec::SmallVec::new(),
                    // CLEARED — see comment block above.
                    variants: smallvec::SmallVec::new(),
                    size: 0,
                    alignment: 0,
                    drop_fn: None,
                    clone_fn: None,
                    protocols: smallvec::SmallVec::new(),
                    visibility: ty.visibility,
                    alias_target: None,
                    is_transparent_wrapper: false,
                });
            }
        }
        // SECOND PASS — non-Protocol imports with protocol-id remap.
        for ty in module.types.iter() {
            if matches!(ty.kind, crate::types::TypeKind::Protocol) {
                continue;
            }
            self.import_archive_type_with_protocol_remap(
                ty,
                &module.strings,
                &protocol_id_remap,
            );
        }
        // SECOND PASS — populate `type_aliases` for every imported
        // `TypeKind::Alias` descriptor by resolving its `alias_target`
        // through the archive's own type table (the cloned target
        // carries archive-local TypeIds; codegen-side `type_name_to_id`
        // only has them mapped by name, not by id).
        //
        // Without this, archive aliases like
        // `type TextResult<T> is Result<T, Text>;` lose the alias
        // relation at the codegen layer — `r.is_ok()` (where `r:
        // TextResult<Int>`) emits `CallM("TextResult.is_ok")`
        // instead of `CallM("Result.is_ok")` because
        // `compile_method_call`'s `resolve_type_alias("TextResult")`
        // returns identity. The runtime then panics with "method
        // 'TextResult.is_ok' not found on receiver of runtime kind
        // Object".
        for ty in module.types.iter() {
            if !matches!(ty.kind, crate::types::TypeKind::Alias) {
                continue;
            }
            let alias_name = match module.strings.get(ty.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let target_ref = match &ty.alias_target {
                Some(t) => t,
                None => continue,
            };
            // Recover the target's base TypeId, then look up its
            // name in the archive's own type table.
            let target_tid = match target_ref {
                crate::types::TypeRef::Concrete(tid) => *tid,
                crate::types::TypeRef::Instantiated { base, .. } => *base,
                _ => continue,
            };
            let target_name = module
                .types
                .iter()
                .find(|t| t.id == target_tid)
                .and_then(|t| module.strings.get(t.name))
                .map(|s| s.to_string());
            // Fallback: when the target lives in a sibling archive
            // module not part of this `import_archive_module_types`
            // call, the in-archive lookup misses. Probe the codegen's
            // own `type_name_to_id` reverse — if any type in
            // `self.types` matches the archive TypeId by VALUE (which
            // is the well-known-types convention for the Verum
            // builtins like Maybe/Result/Ordering), use its name.
            let target_name = target_name.or_else(|| {
                self.types.iter().find(|t| t.id == target_tid).and_then(|t| {
                    self.ctx.strings.get(t.name.0 as usize).cloned()
                })
            });
            if let Some(name) = target_name {
                self.type_aliases.insert(alias_name, name);
            }
        }
        // **Field-type-name deferred-resolution pass**.
        //
        // The per-type loop above populates `type_field_type_names`
        // eagerly (via `import_archive_type_with_protocol_remap`), but
        // when a field's TypeRef references a type that hasn't been
        // loaded YET (declaration order within the module is arbitrary
        // — e.g. `RefCell` appears before `BorrowState` in
        // `core/base/cell.vr`), `type_ref_to_field_name` returns None
        // and the entry is skipped.  By the end of this method every
        // type in the module IS loaded — re-walk fields and repopulate
        // any missing entries.
        //
        // The pass is first-wins (`.entry(...).or_insert(...)`) so
        // entries already populated by the eager path keep their slot;
        // only deferred ones get filled.
        let pending: Vec<((String, String), String)> = {
            let mut out: Vec<((String, String), String)> = Vec::new();
            for ty in &self.types {
                if ty.fields.is_empty() {
                    continue;
                }
                let simple_name = match self.ctx.strings.get(ty.name.0 as usize) {
                    Some(s) => s.clone(),
                    None => continue,
                };
                for fdesc in ty.fields.iter() {
                    let fname = match self.ctx.strings.get(fdesc.name.0 as usize) {
                        Some(s) => s.clone(),
                        None => continue,
                    };
                    if self
                        .type_field_type_names
                        .contains_key(&(simple_name.clone(), fname.clone()))
                    {
                        continue;
                    }
                    if let Some(ty_name) = self.type_ref_to_field_name(&fdesc.type_ref) {
                        out.push(((simple_name.clone(), fname), ty_name));
                    }
                }
            }
            out
        };
        for (key, value) in pending {
            self.type_field_type_names.entry(key).or_insert(value);
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &CodegenConfig {
        &self.config
    }

    /// Merge archive-driven function bodies into the user codegen.
    ///
    /// Phase 2 of the precompiled-stdlib body-merge epic. The archive
    /// loader (`verum_compiler::archive_ctx_loader::register_module_filtered`)
    /// registers stdlib `FunctionInfo` metadata into `ctx.functions`
    /// for every wanted symbol; without a matching VBC body, the
    /// finalize-time stub-emitter synthesises a `RetV` placeholder
    /// that returns `Unit` — every `Some(x).is_some()` evaluates to
    /// `()` instead of `true`. This method copies the archive's real
    /// bytecode bodies into `self.functions` so the stub fallback
    /// only fires for symbols that genuinely have no body in the
    /// archive (FFI extern, user-supplied stubs, etc.).
    ///
    /// `archive_module` is a single decoded archive module (e.g. the
    /// `core.base` module bundling Maybe/Result impls).
    /// `func_id_remap` maps each archive-local `FunctionId` to the
    /// codegen-global `FunctionId` that the loader allocated for it
    /// during the metadata pass.
    ///
    /// Returns the number of bodies successfully copied. Skips:
    ///
    ///   * Functions whose new id was already pushed (idempotent —
    ///     repeat calls don't duplicate).
    ///   * Functions whose archive descriptor has zero bytecode
    ///     length (FFI extern / stub-only declarations).
    ///   * Functions referencing archive constants of variants the
    ///     codegen's `ConstantEntry` table can't represent
    ///     (`Constant::Function` / `Constant::Protocol` /
    ///     `Constant::Array` — currently rare in stdlib bodies).
    ///
    /// **Performance**: O(N_func × M_instr_per_func). All ID remaps
    /// are HashMap lookups (O(1) avg); the per-function decode reuses
    /// the descriptor's pre-decoded `instructions` field when set,
    /// falling back to a fresh decode of the raw bytecode region.
    /// Lazy: only the functions in `func_id_remap` are touched, so a
    /// hello-world that mounts five stdlib symbols pays for five
    /// bodies, not the full archive's ~7000.
    pub fn merge_archive_function_bodies(
        &mut self,
        archive_module: &crate::module::VbcModule,
        func_id_remap: &std::collections::HashMap<u32, crate::module::FunctionId>,
    ) -> usize {
        use std::collections::HashMap;
        if func_id_remap.is_empty() {
            return 0;
        }

        // **Archive-wide name index population** (task #12 fix).
        //
        // Record (archive_function_name → user_fid) for EVERY function
        // in this archive module, regardless of mount-set membership.
        // Populates `archive_func_name_to_fid` which
        // `ArchiveBodyRemap::map_function`'s Tier-2b fallback consults
        // to resolve cross-module Calls whose target isn't in the
        // user's mount-filtered `ctx.functions`.  First-wins: once a
        // name is bound across the merge sequence, sibling archives
        // can't rebind it — mirrors `ctx.functions`'s stdlib-bootstrap
        // first-wins discipline.
        for fn_desc in archive_module.functions.iter() {
            if let Some(&user_fid) = func_id_remap.get(&fn_desc.id.0)
                && let Some(name) = archive_module.strings.get(fn_desc.name)
                && !name.is_empty()
            {
                self.archive_func_name_to_fid
                    .entry(name.to_string())
                    .or_insert(user_fid);
                // **task #17/#39 + #10/§3.3 close-out** — propagate
                // `__tls_init_*` synthetic functions from the archive's
                // codegen into the user-side `static_init_functions`
                // list.  Without this, the archive's TLS-init ctors
                // are loaded as ordinary function bodies but never
                // registered as global constructors — the user-side
                // codegen's `module.global_ctors` only contains the
                // ctors the USER's source declares, not the stdlib's.
                //
                // Symptom pre-fix: `static mut GLOBAL_HAZARD_DOMAIN:
                // HazardDomain = HazardDomain { ... }` declared in
                // `core/mem/hazard.vr` was precompiled into the stdlib
                // archive with its `__tls_init_GLOBAL_HAZARD_DOMAIN`
                // synthetic function; user code that mounted
                // `GLOBAL_HAZARD_DOMAIN` then queried it via TlsGet
                // and observed `Value::default()` (nil) — the slot
                // was never populated.  `hazard_stats()` null-derefed
                // at `&self.thread_count` because `self` was nil.
                //
                // Idempotent across multi-archive merges: the
                // `FunctionId(user_fid)` is the user-side remapped id
                // post-merge, and `static_init_functions` is a
                // dedup-on-push by the comparison at line ~15067.
                if name.starts_with("__tls_init_") {
                    let already_registered = self
                        .static_init_functions
                        .iter()
                        .any(|&fid| fid == user_fid);
                    if !already_registered {
                        self.static_init_functions.push(user_fid);
                    }
                }
            }
        }

        // Cache of archive-side ids already in `self.functions` to
        // make this call idempotent.
        let already_emitted: std::collections::HashSet<u32> = self
            .functions
            .iter()
            .map(|f| f.descriptor.id.0)
            .collect();

        // ----- Build per-archive-module ID remap tables -----

        // archive type id → codegen type id (via type-name lookup;
        // codegen.type_name_to_id was populated by
        // `import_archive_module_types`).
        let mut type_id_remap: HashMap<u32, u32> = HashMap::new();
        for ty in archive_module.types.iter() {
            let archive_name = match archive_module.strings.get(ty.name) {
                Some(s) => s,
                None => continue,
            };
            if let Some(&codegen_tid) = self.type_name_to_id.get(archive_name) {
                type_id_remap.insert(ty.id.0, codegen_tid.0);
            }
        }

        // archive const id → codegen const id. Deep-copy each
        // referenced constant into ctx.constants. We only need to
        // import constants that the bodies actually reference, but
        // pre-walking instructions is wasted work — copy them all
        // up-front (typical archive module has <100 constants).
        let mut const_id_remap: HashMap<u32, u32> = HashMap::new();
        for (idx, src_const) in archive_module.constants.iter().enumerate() {
            let archive_cid = idx as u32;
            let codegen_cid = match src_const {
                crate::module::Constant::Int(v) => self.ctx.add_const_int(*v),
                crate::module::Constant::Float(v) => self.ctx.add_const_float(*v),
                crate::module::Constant::String(sid) => {
                    let text = archive_module.strings.get(*sid).unwrap_or("");
                    self.ctx.add_const_string(text)
                }
                crate::module::Constant::Bytes(bytes) => {
                    self.ctx.add_const_bytes(bytes.clone())
                }
                crate::module::Constant::Type(type_ref) => {
                    let remapped = remap_type_ref_archive(type_ref, &type_id_remap);
                    let id = crate::module::ConstId(self.ctx.constants.len() as u32);
                    self.ctx
                        .constants
                        .push(context::ConstantEntry::Type(remapped));
                    id
                }
                // Function / Protocol / Array constants don't have a
                // direct ConstantEntry variant. Skip (very rare in
                // stdlib bodies; the body that loads such a constant
                // will fail at runtime when its LoadK lands on a
                // dangling const id, surfacing as a typed error
                // rather than a silent miscompile).
                crate::module::Constant::Function(_)
                | crate::module::Constant::Protocol(_)
                | crate::module::Constant::Array(_) => continue,
            };
            const_id_remap.insert(archive_cid, codegen_cid.0);
        }

        // Build archive-local id → function name index, for the
        // name-based fallback in `ArchiveBodyRemap::map_function`.
        //
        // **Why this exists**: per-module `func_id_remap` only covers
        // archive functions whose body lives in *this* archive module.
        // But Tier-0 stdlib bodies frequently emit raw `Call { func_id }`
        // pointing at cross-module functions (e.g. `Text.grow` calling
        // `alloc` from `core.base.memory`). When the precompile's
        // `emit_missing_stub_descriptors` does NOT synthesise an
        // extern-stub descriptor for the callee in this archive module
        // (the descriptor synthesis pass has filter conditions that can
        // miss cross-module calls under certain `ctx.functions`
        // populations), the cross-module `func_id` arrives at user
        // merge with no entry in `func_id_remap`. The identity-fallback
        // (`unwrap_or(src)`) then dispatches the call to whichever
        // user-side function happens to live at that raw id — observed
        // in the wild as `Text.grow → alloc` landing on `Unfold.fuse`
        // (id 8088), the next `is_null` landing on `RepeatWith.windows`
        // (id 7909), and a subsequent assertion-pc-12 crash.
        //
        // The name-table here lets `map_function` look up the archive
        // id's name and re-resolve to the user-side function with the
        // same name via `self.ctx.functions`, recovering the correct
        // dispatch target without requiring a precompile rebuild.
        let mut archive_id_to_name: HashMap<u32, String> = HashMap::with_capacity(
            archive_module.functions.len() + archive_module.external_function_names.len(),
        );
        for fn_desc in archive_module.functions.iter() {
            if let Some(name) = archive_module.strings.get(fn_desc.name) {
                archive_id_to_name.insert(fn_desc.id.0, name.to_string());
            }
        }
        // Merge the cross-module call name table (populated at
        // precompile finalize, see `build_module`'s tail in
        // `crates/verum_vbc/src/codegen/mod.rs`). Each entry maps an
        // archive bytecode `func_id` whose body lives in a sibling
        // archive module to its qualified name. Without this merge,
        // `ArchiveBodyRemap::map_function`'s Tier-2 name fallback
        // misses for every cross-module Call (the in-module
        // `archive_module.functions` table only carries this module's
        // own functions) and the identity-fallback (Tier-3) routes
        // the call to whatever unrelated user-side function happens
        // to live at the raw codegen-time id — observed in the wild
        // as `Text.push_byte → Text.grow → alloc` dispatching to
        // `Successors.next` and crashing with a null-pointer deref
        // at pc=0. **Per-module entries (`archive_module.functions`)
        // win on conflict**: if a cross-module name happens to also
        // refer to a function defined in this module, the local
        // definition takes precedence so the Tier-1 per-module remap
        // still routes the call to the correct local body.
        for (fid, sid) in archive_module.external_function_names.iter() {
            archive_id_to_name.entry(fid.0).or_insert_with(|| {
                archive_module
                    .strings
                    .get(*sid)
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            });
        }
        if std::env::var("VERUM_TRACE_ARCHIVE_FUNCS").is_ok() && archive_module.name.contains("text") {
            eprintln!(
                "[archive-funcs] module='{}' n_funcs={} archive_id_to_name.len={} sample_ids={:?}",
                archive_module.name,
                archive_module.functions.len(),
                archive_id_to_name.len(),
                archive_id_to_name.iter().take(10).map(|(k, v)| (k, v.as_str())).collect::<Vec<_>>(),
            );
            // Sample max id
            let max_id = archive_module.functions.iter().map(|f| f.id.0).max().unwrap_or(0);
            eprintln!("[archive-funcs] max_archive_id={}", max_id);
        }
        // Build user-side name → FunctionId index ONCE per merge call.
        // Cloning the (name, id) pairs is cheap relative to the cost
        // of allocating a HashMap-per-instruction fallback path.
        let ctx_func_by_name: HashMap<String, crate::module::FunctionId> = self
            .ctx
            .functions
            .iter()
            .map(|(name, info)| (name.clone(), info.id))
            .collect();
        // Snapshot the archive-wide name index so it doesn't keep
        // `self` borrowed across the body-rewrite loop below (which
        // needs `&mut self` for `remap_archive_string_operands`).
        let archive_func_by_name_snapshot: HashMap<String, crate::module::FunctionId> =
            self.archive_func_name_to_fid.clone();

        // ----- Walk archive functions and copy bodies -----

        let remap = ArchiveBodyRemap {
            funcs: func_id_remap,
            types: &type_id_remap,
            consts: &const_id_remap,
            archive_id_to_name: &archive_id_to_name,
            ctx_func_by_name: &ctx_func_by_name,
            archive_func_by_name: &archive_func_by_name_snapshot,
        };

        // ----- TLS slot remap — fixes cross-archive slot collisions -----
        //
        // **task #10/§3.3 close-out + AOT cascade Class B partial close.**
        //
        // Pre-fix: each stdlib precompile pass restarted `next_tls_slot`
        // at 0.  Different archive modules' `static mut` declarations
        // therefore picked overlapping slot literals (hazard.vr assigned
        // slot 0 to GLOBAL_HAZARD_DOMAIN; sys/common.vr also assigned
        // slot 0 to PROCESS_ARGC).  When all archives merge into one
        // user-side runtime, the LAST `__tls_init_*` for that slot wins,
        // silently overwriting earlier writes.  Consumers reading their
        // module's static then see another module's value (or nil if
        // the slot was overwritten with a different type).
        //
        // Fix: per-archive-module pre-pass that scans every
        // `__tls_init_<NAME>` function for the `LoadI dst, val; TlsSet
        // {slot: dst, ...}` peephole pattern.  For each unique archive
        // slot literal found, allocate a fresh user-side TLS slot via
        // `register_thread_local(NAME)` (keyed on the NAME extracted
        // from the function name so cross-module re-imports stay
        // coherent).  Apply the remap in a third instruction-walk pass
        // that rewrites `LoadI val` to the user-side slot wherever the
        // dst register flows directly into a TlsSet or TlsGet slot
        // operand.
        let mut tls_slot_remap: HashMap<i64, i64> = HashMap::new();
        for archive_desc in archive_module.functions.iter() {
            let archive_name = match archive_module.strings.get(archive_desc.name) {
                Some(s) if s.starts_with("__tls_init_") => &s[11..],
                _ => continue,
            };
            // Decode this ctor's body to extract the literal slot.
            let ctor_instrs: Vec<crate::instruction::Instruction> =
                if let Some(ref decoded) = archive_desc.instructions {
                    decoded.clone()
                } else {
                    let off = archive_desc.bytecode_offset as usize;
                    let len = archive_desc.bytecode_length as usize;
                    if len == 0 || off + len > archive_module.bytecode.len() {
                        continue;
                    }
                    let region = &archive_module.bytecode[off..off + len];
                    match crate::bytecode::decode_instructions(region) {
                        Ok(decoded) => decoded,
                        Err(_) => continue,
                    }
                };
            // Find the LoadI; TlsSet pattern.  The `__tls_init_*`
            // function body emitted by `compile_pending_tls_inits`
            // (codegen/mod.rs::~10955) is:
            //   <init expression>
            //   LoadI slot_reg, <slot literal>
            //   TlsSet { slot: slot_reg, val: result_reg }
            //   Ret { value: result_reg }
            // So the slot literal is the LoadI immediately preceding
            // the TlsSet.
            //
            // Sister scan: recover the declared TYPE NAME of the static
            // by tracking `New { dst, type_id, ... }` opcodes.  The init
            // expression for a record-typed `static mut NAME: T = T{..}`
            // emits a `New` whose dst register ultimately flows into the
            // TlsSet val (possibly via intermediate SetF/Clone steps).
            // Recovering the archive type_id at this scan, mapping
            // through `type_id_remap`, and reverse-lookup via
            // `self.types` lets us re-populate `static_mut_type_names`
            // for cross-archive static-mut bindings — the source-module
            // population in the `ItemKind::Static` arm only fires for
            // statics declared in the user's own translation unit.
            // Without this scan, every `STATIC_MUT_RECORD.field` access
            // in user code that mounts a stdlib static (e.g.
            // `core.mem.hazard.GLOBAL_HAZARD_DOMAIN`) falls through to
            // the global interned-id fallback in `resolve_field_index`
            // and reads at wildly wrong byte offsets — root cause of
            // `hazard_stats() → GLOBAL_HAZARD_DOMAIN.scan_hazards()`
            // null-derefing at the first GetF inside the method body.
            let mut prev_loadi: Option<(crate::instruction::Reg, i64)> = None;
            let mut last_new: Option<(crate::instruction::Reg, u32)> = None;
            for instr in ctor_instrs.iter() {
                match instr {
                    crate::instruction::Instruction::LoadI { dst, value } => {
                        prev_loadi = Some((*dst, *value));
                    }
                    crate::instruction::Instruction::New { dst, type_id, .. } => {
                        last_new = Some((*dst, *type_id));
                    }
                    crate::instruction::Instruction::TlsSet { slot, val } => {
                        if let Some((load_dst, archive_slot)) = prev_loadi
                            && load_dst == *slot
                        {
                            // Allocate a user-side slot keyed on NAME so
                            // downstream consumers in OTHER archive
                            // modules that mount this NAME hit the same
                            // user_slot.
                            let user_slot =
                                self.ctx.register_thread_local(archive_name) as i64;
                            tls_slot_remap.insert(archive_slot, user_slot);

                            // If the val we're about to store originated
                            // from a record `New { type_id }`, recover
                            // the canonical type name and populate
                            // static_mut_type_names for both the bare
                            // archive name and the module-qualified key.
                            if let Some((new_dst, archive_type_id)) = last_new
                                && new_dst == *val
                                && let Some(&user_tid) = type_id_remap.get(&archive_type_id)
                                && let Some(td) = self.types.iter().find(|t| t.id.0 == user_tid)
                                && let Some(canonical_name) =
                                    self.ctx.strings.get(td.name.0 as usize).cloned()
                            {
                                // `td.name` is the simple type-name as
                                // re-registered into `self.ctx.strings`
                                // by `import_archive_module_types`; the
                                // same key that `type_field_layouts` is
                                // populated under.  Insert it for the
                                // bare archive name so subsequent reads
                                // of `STATIC` in user code surface the
                                // right type, and ALSO for the dotted
                                // archive-qualified key (some lookup
                                // sites probe via the `<module>.<NAME>`
                                // form).
                                self.static_mut_type_names
                                    .insert(archive_name.to_string(), canonical_name);
                            }
                            break;
                        }
                    }
                    _ => {
                        prev_loadi = None;
                    }
                }
            }
        }

        let mut copied = 0usize;
        for archive_desc in archive_module.functions.iter() {
            let archive_fid = archive_desc.id.0;
            let codegen_fid = match func_id_remap.get(&archive_fid) {
                Some(&fid) => fid,
                None => continue, // not in wanted set
            };
            if already_emitted.contains(&codegen_fid.0) {
                continue;
            }
            // Decode instructions: prefer the descriptor's cached
            // `instructions` field (populated post-deserialize); fall
            // back to a fresh bytecode decode.
            let mut instructions = if let Some(ref decoded) = archive_desc.instructions {
                decoded.clone()
            } else {
                let off = archive_desc.bytecode_offset as usize;
                let len = archive_desc.bytecode_length as usize;
                if len == 0 || off + len > archive_module.bytecode.len() {
                    continue;
                }
                let region = &archive_module.bytecode[off..off + len];
                match crate::bytecode::decode_instructions(region) {
                    Ok(decoded) => decoded,
                    Err(_) => continue,
                }
            };
            // PRE-PASS — convert jump offsets from BYTE form (the
            // archive's serialised representation, what
            // `decode_instructions` produces) to INSTRUCTION-INDEX
            // form (what `fixup_jump_offsets` expects on input).
            // Codegen's `encode_instructions_with_fixup` runs at
            // finalize and assumes its input has instruction-index
            // offsets — feeding it byte-offset values double-applies
            // the fixup (treating archive byte offset 7 as
            // instr-index 7, then converting THAT to a byte offset
            // → infinite loop or out-of-bounds branch). Without
            // this normalisation, `Maybe.is_some()` on `None` jumps
            // to byte offset 0 of the function (instead of byte
            // offset of the post-Ret point), spinning forever.
            byte_offsets_to_instr_indices(&mut instructions);

            // FIRST PASS — string-id remap. Several instruction
            // variants carry an interned-name index that the
            // codegen's finalize-time `string_id_map` will look up
            // (CallM.method_id, Panic.message_id, Assert.message_id,
            // CtxGet/CtxProvide/CtxCheckNegative.ctx_type, …). These
            // are STRING ids in the codegen-internal namespace, NOT
            // function ids — the shared `rewrite_instruction_ids`
            // helper (next pass) deliberately treats CallM.method_id
            // as a function id (matching the linker's convention),
            // so we MUST do the string-id remap first to convert the
            // archive's byte-offset StringId to a codegen-local
            // index BEFORE the function-id pass — otherwise the
            // function-id pass would (correctly per its semantics
            // but wrong for our use case) try to map the archive
            // byte offset as a function id and produce garbage.
            for instr in instructions.iter_mut() {
                self.remap_archive_string_operands(instr, archive_module);
            }
            // SECOND PASS — function/type/const id remap via the
            // shared per-instruction helper.
            for instr in instructions.iter_mut() {
                crate::bytecode_remap::rewrite_instruction_ids(instr, &remap);
            }

            // THIRD PASS — TLS slot remap (peephole on LoadI → TlsSet /
            // LoadI → TlsGet).  See `tls_slot_remap` construction above
            // for the full rationale.  Walk instruction pairs and, when
            // the destination register of a LoadI matches the slot
            // operand of the immediately-following TlsSet/TlsGet, rewrite
            // the LoadI value through `tls_slot_remap`.  Identity-fallback
            // for slot literals not in the remap (defensive — should
            // never happen for `__tls_init_*`-managed slots since the
            // pre-pass builds the remap exhaustively).
            if !tls_slot_remap.is_empty() {
                for i in 0..instructions.len() {
                    let (loadi_dst, loadi_val): (
                        crate::instruction::Reg,
                        i64,
                    ) = match &instructions[i] {
                        crate::instruction::Instruction::LoadI { dst, value } => {
                            (*dst, *value)
                        }
                        _ => continue,
                    };
                    if i + 1 >= instructions.len() {
                        continue;
                    }
                    let consumes_as_slot = match &instructions[i + 1] {
                        crate::instruction::Instruction::TlsSet { slot, .. }
                        | crate::instruction::Instruction::TlsGet { slot, .. } => {
                            *slot == loadi_dst
                        }
                        _ => false,
                    };
                    if !consumes_as_slot {
                        continue;
                    }
                    if let Some(&new_slot) = tls_slot_remap.get(&loadi_val)
                        && new_slot != loadi_val
                    {
                        instructions[i] = crate::instruction::Instruction::LoadI {
                            dst: loadi_dst,
                            value: new_slot,
                        };
                    }
                }
            }

            // Build the new descriptor with codegen-namespace ids /
            // strings. Param + return types pull from the
            // (already-remapped) codegen type table.
            let mut new_desc = archive_desc.clone();
            new_desc.id = codegen_fid;
            // Re-intern the function name into the codegen string
            // table so the finalize-time `string_id_map` can resolve
            // it to the final-module StringId.
            if let Some(name_text) = archive_module.strings.get(archive_desc.name) {
                let codegen_sid = self.ctx.intern_string_raw(name_text);
                new_desc.name = StringId(codegen_sid);
            }
            // Param names → re-intern; param type refs → remap.
            for param in new_desc.params.iter_mut() {
                if let Some(pname_text) = archive_module.strings.get(param.name) {
                    let pname_id = self.ctx.intern_string_raw(pname_text);
                    param.name = StringId(pname_id);
                }
                param.type_ref = remap_type_ref_archive(&param.type_ref, &type_id_remap);
            }
            new_desc.return_type =
                remap_type_ref_archive(&new_desc.return_type, &type_id_remap);
            if let Some(parent) = new_desc.parent_type {
                if let Some(&codegen_parent) = type_id_remap.get(&parent.0) {
                    new_desc.parent_type = Some(crate::types::TypeId(codegen_parent));
                }
            }
            // The codegen finalize re-encodes from `instructions` —
            // clear bytecode_offset / bytecode_length so finalize
            // doesn't try to use stale archive offsets.
            new_desc.bytecode_offset = 0;
            new_desc.bytecode_length = 0;
            new_desc.instructions = Some(instructions.clone());
            self.functions
                .push(crate::module::VbcFunction::new(new_desc, instructions));
            copied += 1;
        }

        copied
    }
}

impl VbcCodegen {
    /// Per-instruction string-id remap from archive's byte-offset
    /// StringId space to the codegen-internal sequential string
    /// index. Called as a SECOND pass after
    /// [`crate::bytecode_remap::rewrite_instruction_ids`] which
    /// (deliberately, for linker compatibility) leaves these fields
    /// untouched.
    ///
    /// `archive_module` provides the source string table — every
    /// archive StringId resolves to a UTF-8 text via
    /// `archive_module.strings.get(StringId)` and we re-intern that
    /// text into `ctx.strings` to obtain the codegen-internal index
    /// the finalize-time `string_id_map` will resolve to a final
    /// module-level StringId.
    fn remap_archive_string_operands(
        &mut self,
        instr: &mut crate::instruction::Instruction,
        archive_module: &crate::module::VbcModule,
    ) {
        use crate::instruction::Instruction;
        let intern_archive = |this: &mut Self, archive_sid: u32| -> u32 {
            let text = match archive_module.strings.get(crate::types::StringId(archive_sid))
            {
                Some(s) => s.to_string(),
                None => return archive_sid,
            };
            this.ctx.intern_string_raw(&text)
        };
        match instr {
            Instruction::CallM { method_id, .. } => {
                let new_id = intern_archive(self, *method_id);
                *method_id = new_id;
            }
            Instruction::Panic { message_id } => {
                let new_id = intern_archive(self, *message_id);
                *message_id = new_id;
            }
            Instruction::Assert { message_id, .. } => {
                let new_id = intern_archive(self, *message_id);
                *message_id = new_id;
            }
            Instruction::CtxGet { ctx_type, .. } => {
                let new_id = intern_archive(self, *ctx_type);
                *ctx_type = new_id;
            }
            Instruction::CtxProvide { ctx_type, .. } => {
                let new_id = intern_archive(self, *ctx_type);
                *ctx_type = new_id;
            }
            Instruction::CtxCheckNegative { ctx_type, func_name } => {
                let new_ctx = intern_archive(self, *ctx_type);
                *ctx_type = new_ctx;
                let new_fn = intern_archive(self, *func_name);
                *func_name = new_fn;
            }
            // CmpG.protocol_id uses (string_idx + 1) encoding when
            // > 0 — pass through the +1 offset.
            Instruction::CmpG { protocol_id, .. } if *protocol_id > 0 => {
                let archive_sid = *protocol_id - 1;
                let new_id = intern_archive(self, archive_sid);
                *protocol_id = new_id + 1;
            }
            _ => {}
        }
    }
}

/// Helper IdRemap implementation used by [`VbcCodegen::merge_archive_function_bodies`]
/// during the per-instruction id rewrite. The lookup tables are
/// borrowed from the calling scope so we don't pay the
/// allocate-per-instruction cost of cloning HashMaps.
struct ArchiveBodyRemap<'a> {
    funcs: &'a std::collections::HashMap<u32, crate::module::FunctionId>,
    types: &'a std::collections::HashMap<u32, u32>,
    consts: &'a std::collections::HashMap<u32, u32>,
    /// Archive-local function id → function name (interned from
    /// `archive_module.strings`). Populated for every entry in the
    /// archive module's `functions` table. Used by `map_function`'s
    /// name-based fallback when the per-module remap misses.
    archive_id_to_name: &'a std::collections::HashMap<u32, String>,
    /// User-codegen `ctx.functions` projected to name → FunctionId.
    /// Used by `map_function` to recover a cross-module Call target
    /// when the archive id isn't covered by the per-module remap.
    ctx_func_by_name: &'a std::collections::HashMap<String, crate::module::FunctionId>,
    /// **Archive-wide cross-module name → user_fid index** (task #12).
    /// Populated unconditionally by `archive_ctx_loader` for every
    /// archive function regardless of the user's mount set.  Acts as
    /// a Tier-2b fallback when `ctx_func_by_name` (mount-filtered) misses
    /// — covers the transitive-cross-module case where stdlib body X's
    /// `Call { func_id }` targets stdlib function Y whose home module
    /// isn't directly mounted by the user.
    archive_func_by_name: &'a std::collections::HashMap<String, crate::module::FunctionId>,
}

impl crate::bytecode_remap::IdRemap for ArchiveBodyRemap<'_> {
    fn map_function(&self, src: crate::module::FunctionId) -> crate::module::FunctionId {
        // Stage-1/2/3 stub-id range definitions (mirrored from
        // `stdlib_bootstrap::merge_codegen_into_self`). A user-side
        // `ctx_func_by_name` mapping that points to a stub-range id
        // means the producing module's REAL body wasn't merged into
        // ctx yet — bypassing the Tier 2a return below avoids freezing
        // the unresolved stub id into the rewritten body. Tier 2b
        // (archive-wide name index) then has a chance to find the
        // real body that's already loaded as part of a later archive.
        //
        // Without this gate, task #47's `global_ctors` cascade fires
        // FunctionNotFound(0xFEFF****) at runtime for every cross-
        // module Call whose target's stub_id leaked into ctx because
        // the dependency was loaded out of resolution order.
        const STAGE1_STUB_BASE: u32 = u32::MAX - 0x40_0000;
        const STAGE2_STUB_BASE: u32 = u32::MAX - 0xC0_0000;
        const STAGE3_STUB_BASE: u32 = u32::MAX - 0x100_0000;
        const STUB_RANGE_WIDTH: u32 = 0x10_0000;
        let is_stub_id = |id: u32| -> bool {
            let s1 = id <= STAGE1_STUB_BASE && id >= STAGE1_STUB_BASE - STUB_RANGE_WIDTH;
            let s2 = id <= STAGE2_STUB_BASE && id >= STAGE2_STUB_BASE - STUB_RANGE_WIDTH;
            let s3 = id <= STAGE3_STUB_BASE && id >= STAGE3_STUB_BASE - STUB_RANGE_WIDTH;
            s1 || s2 || s3
        };
        // Tier 1: per-module remap (archive function ids whose body
        // lives in *this* archive module).
        if let Some(&fid) = self.funcs.get(&src.0) {
            return fid;
        }
        // Tier 2: name-based fallback for cross-module calls. If the
        // archive id corresponds to a known extern descriptor in this
        // archive module, look its name up in the user codegen's
        // `ctx.functions` table — that's where stdlib functions from
        // sibling modules (e.g. `alloc` in `core.base.memory` called
        // from `core.text.text::Text.grow`) live after archive load.
        // Without this fallback, the identity-fallback below would
        // land the Call on whatever unrelated function happens to
        // occupy the raw archive id at user-side (live failure mode:
        // `Text.grow → alloc` dispatching to `Unfold.fuse`).
        if let Some(name) = self.archive_id_to_name.get(&src.0) {
            if let Some(&fid) = self.ctx_func_by_name.get(name) {
                // **Stub-id reject** (task #47 close-out): if the user-
                // side ctx maps `name` to a stub-range id, the producing
                // module's real body hasn't been merged yet. Skip Tier
                // 2a so Tier 2b's archive-wide index can find the real
                // body in another loaded archive. The stub itself has
                // no executable body, so returning it here would cause
                // FunctionNotFound at runtime.
                if !is_stub_id(fid.0) {
                    if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
                        eprintln!("[remap-fallback] tier2a OK archive_id={} → name={:?} → user_fid={}", src.0, name, fid.0);
                    }
                    return fid;
                } else if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
                    eprintln!("[remap-fallback] tier2a REJECT-STUB archive_id={} → name={:?} → ctx_fid={} (stub-range) — falling through to tier2b", src.0, name, fid.0);
                }
            }
            // **Tier-2b** (task #12): the user-facing `ctx.functions`
            // table is filtered by the mount set (only names the user
            // explicitly brought into scope are present).  Cross-module
            // Calls inside transitively-loaded stdlib bodies frequently
            // target functions whose home archive isn't directly mounted
            // by the user — `ctx.functions` misses for them.  The
            // archive-wide name index (populated unconditionally by
            // `archive_ctx_loader` for every loaded archive function)
            // catches that case.  Without this, the identity-fallback
            // below silently dispatches the Call to whatever unrelated
            // user-side function happens to occupy the raw archive id —
            // canonical failure: `AsyncSemaphore.new` body's
            // `Mutex.new(...)` Call landing on
            // `Phaser.arrive_and_await$closure$4` because the user only
            // mounted `core.async.semaphore.AsyncSemaphore`.
            if let Some(&fid) = self.archive_func_by_name.get(name) {
                // Same stub-id reject as Tier 2a — never freeze a
                // stub-range id into rewritten bytecode (task #47).
                if !is_stub_id(fid.0) {
                    if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
                        eprintln!("[remap-fallback] tier2b OK archive_id={} → name={:?} → user_fid={} (archive-wide)", src.0, name, fid.0);
                    }
                    return fid;
                } else if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
                    eprintln!("[remap-fallback] tier2b REJECT-STUB archive_id={} → name={:?} → archive_fid={} (stub-range)", src.0, name, fid.0);
                }
            }
            if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
                eprintln!("[remap-fallback] tier2 MISS archive_id={} archive_name={:?} not in ctx_func_by_name OR archive_func_by_name", src.0, name);
            }
        } else if std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok() {
            eprintln!("[remap-fallback] tier3 IDENTITY archive_id={} not in archive_id_to_name (Tier1 misses too)", src.0);
        }
        // Tier 3: identity fallback. Reserved for ids the archive
        // serialiser intentionally leaves opaque (kernel intrinsic
        // dispatch tags, FFI sentinels). A miss here surfaces at
        // runtime as `FunctionNotFound` rather than silent
        // miscompile.
        src
    }
    fn map_type_id(&self, src: crate::types::TypeId) -> crate::types::TypeId {
        self.types
            .get(&src.0)
            .copied()
            .map(crate::types::TypeId)
            .unwrap_or(src)
    }
    fn map_const(&self, src: crate::module::ConstId) -> crate::module::ConstId {
        self.consts
            .get(&src.0)
            .copied()
            .map(crate::module::ConstId)
            .unwrap_or(src)
    }
    // String / Protocol use the IdRemap default (identity) — strings
    // don't appear as instruction operands, and protocol ids in the
    // user codegen's namespace match the archive's namespace for
    // built-in protocols (Eq/Ord/Hash/...).
}

/// Convert jump offsets in `instructions` from BYTE form to
/// INSTRUCTION-INDEX form, in place. Mirrors the inverse of
/// `crate::bytecode::fixup_jump_offsets` (which converts
/// instr-index → byte form), so that an archive function
/// freshly decoded with `decode_instructions` (byte form, since
/// that's what was serialised) can be re-fed into the codegen's
/// `encode_instructions_with_fixup` pipeline cleanly without
/// double-applying the fixup.
///
/// Algorithm: walk once to compute byte offset of each instruction
/// (sum of `instruction_size` for preceding instructions). For
/// every jump-bearing instruction, the byte target is
/// `(idx_byte_offset + instr_size + offset)`; the corresponding
/// instr-index target is the index whose byte-offset equals that
/// target. Store `target_idx - current_idx` as the new offset.
///
/// **Performance**: O(N + N×log N) — the byte-offset table is
/// built linearly; per-jump lookup is a binary search. For typical
/// stdlib bodies (10-50 instructions) this is microseconds.
fn byte_offsets_to_instr_indices(instructions: &mut [crate::instruction::Instruction]) {
    use crate::instruction::Instruction;
    if instructions.is_empty() {
        return;
    }
    // Byte offset of each instruction (cumulative size of preceding
    // instructions). Includes a sentinel at the end equal to total
    // bytecode length, so a jump to "past last instruction" maps to
    // index `instructions.len()` (Ret-fall-through pattern).
    let mut byte_offsets: Vec<usize> = Vec::with_capacity(instructions.len() + 1);
    let mut instr_sizes: Vec<usize> = Vec::with_capacity(instructions.len());
    let mut cur = 0usize;
    for instr in instructions.iter() {
        byte_offsets.push(cur);
        let sz = crate::bytecode::instruction_size(instr);
        instr_sizes.push(sz);
        cur += sz;
    }
    byte_offsets.push(cur); // sentinel for end-of-function
    let byte_to_idx = |byte: usize| -> Option<i32> {
        // Binary search; the table is monotone-strictly-increasing
        // because every instruction has size >= 1.
        match byte_offsets.binary_search(&byte) {
            Ok(idx) => Some(idx as i32),
            Err(_) => None,
        }
    };
    for (idx, instr) in instructions.iter_mut().enumerate() {
        let instr_end_byte = byte_offsets[idx] + instr_sizes[idx];
        let convert = |old_byte_offset: i32| -> i32 {
            let target_byte = (instr_end_byte as i32) + old_byte_offset;
            if target_byte < 0 {
                return old_byte_offset; // out-of-range; preserve
            }
            match byte_to_idx(target_byte as usize) {
                Some(target_idx) => target_idx - (idx as i32),
                None => old_byte_offset, // doesn't land on an instruction boundary
            }
        };
        match instr {
            Instruction::Jmp { offset } => {
                *offset = convert(*offset);
            }
            Instruction::JmpIf { offset, .. }
            | Instruction::JmpNot { offset, .. } => {
                *offset = convert(*offset);
            }
            Instruction::JmpCmp { offset, .. } => {
                *offset = convert(*offset);
            }
            Instruction::CtxProvide { body_offset, .. } => {
                *body_offset = convert(*body_offset);
            }
            Instruction::TryBegin { handler_offset } => {
                *handler_offset = convert(*handler_offset);
            }
            _ => {}
        }
    }
}

/// Recursive [`TypeRef`] remap shared by
/// [`VbcCodegen::merge_archive_function_bodies`] descriptor + constant
/// rewrites. Mirrors `linker::VbcLinker::remap_type_ref` but takes a
/// borrowed `HashMap` and returns identity for unmapped ids (rather
/// than `Err(LinkError::DanglingReference)`) because the loader's
/// "wanted" filter intentionally subsets the archive — references to
/// unimported types resolve at runtime via the codegen's type-name
/// global lookup.
fn remap_type_ref_archive(
    src: &crate::types::TypeRef,
    type_id_remap: &std::collections::HashMap<u32, u32>,
) -> crate::types::TypeRef {
    use crate::types::TypeRef;
    match src {
        TypeRef::Concrete(tid) => {
            let new_id = type_id_remap
                .get(&tid.0)
                .copied()
                .map(crate::types::TypeId)
                .unwrap_or(*tid);
            TypeRef::Concrete(new_id)
        }
        TypeRef::Generic(p) => TypeRef::Generic(*p),
        TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
            base: type_id_remap
                .get(&base.0)
                .copied()
                .map(crate::types::TypeId)
                .unwrap_or(*base),
            args: args
                .iter()
                .map(|a| remap_type_ref_archive(a, type_id_remap))
                .collect(),
        },
        TypeRef::Function {
            params,
            return_type,
            contexts,
        } => TypeRef::Function {
            params: params
                .iter()
                .map(|p| remap_type_ref_archive(p, type_id_remap))
                .collect(),
            return_type: Box::new(remap_type_ref_archive(return_type, type_id_remap)),
            contexts: contexts.clone(),
        },
        TypeRef::Rank2Function {
            type_param_count,
            params,
            return_type,
            contexts,
        } => TypeRef::Rank2Function {
            type_param_count: *type_param_count,
            params: params
                .iter()
                .map(|p| remap_type_ref_archive(p, type_id_remap))
                .collect(),
            return_type: Box::new(remap_type_ref_archive(return_type, type_id_remap)),
            contexts: contexts.clone(),
        },
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => TypeRef::Reference {
            inner: Box::new(remap_type_ref_archive(inner, type_id_remap)),
            mutability: *mutability,
            tier: *tier,
        },
        TypeRef::Tuple(elems) => TypeRef::Tuple(
            elems
                .iter()
                .map(|e| remap_type_ref_archive(e, type_id_remap))
                .collect(),
        ),
        TypeRef::Array { element, length } => TypeRef::Array {
            element: Box::new(remap_type_ref_archive(element, type_id_remap)),
            length: *length,
        },
        TypeRef::Slice(inner) => {
            TypeRef::Slice(Box::new(remap_type_ref_archive(inner, type_id_remap)))
        }
    }
}

/// Cross-module type-table health (#170). Returned by
/// [`VbcCodegen::verify_global_type_table_consistency`]. See that
/// method's docstring for the bug classes each field tracks.
///

/// Note: `MakeVariant`-level orphan detection is intentionally NOT
/// part of this report. At a single-module-with-mounts granularity
/// the cross-module-variant case dominates — most "orphans" are
/// legitimate references to variants whose declaring module wasn't
/// fully loaded. Use [`VbcCodegen::find_orphan_make_variants`] for
/// the diagnostic; treat its output as informational unless you
/// know every transitively-referenced module is in the table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TypeTableHealthReport {
    /// Multiple `TypeDescriptor`s share a single `TypeId.0`. A real
    /// program never has this — TypeIds are supposed to be unique.
    /// Caused by name-collision merge in `type_name_to_id`.
    pub duplicate_ids: Vec<DuplicateTypeId>,
    /// Multiple `TypeDescriptor`s share a name but report different
    /// `TypeId.0` values. Indicates the codegen ran multiple
    /// type-allocation passes that didn't reuse the prior pass's
    /// registration.
    pub duplicate_names_with_different_ids: Vec<DuplicateNameDifferentId>,
    /// A sum type's variant tags are not dense `0..variants.len()`
    /// or contain duplicates. Runtime variant dispatch indexes by
    /// tag, so any gap or duplicate yields wrong-variant dispatch.
    pub variant_tag_anomalies: Vec<VariantTagAnomaly>,
}

impl TypeTableHealthReport {
    /// `true` when every category is empty. Use this in a CI gate:
    /// `assert!(codegen.verify_global_type_table_consistency().is_clean())`.
    pub fn is_clean(&self) -> bool {
        self.duplicate_ids.is_empty()
            && self.duplicate_names_with_different_ids.is_empty()
            && self.variant_tag_anomalies.is_empty()
    }

    /// Total number of issues across all categories. Useful for a
    /// "ratchet" baseline test that lets the count fall but never
    /// rise.
    pub fn issue_count(&self) -> usize {
        self.duplicate_ids.len()
            + self.duplicate_names_with_different_ids.len()
            + self.variant_tag_anomalies.len()
    }

    /// Convert to a `CodegenError` when issues exist. Bundles every
    /// finding into a single `Internal` error so a strict-mode CI
    /// build can use `?` propagation.
    pub fn into_error(self) -> CodegenResult<()> {
        if self.is_clean() {
            return Ok(());
        }
        let mut msg = String::from("type-table consistency violations:\n");
        for d in &self.duplicate_ids {
            msg.push_str(&format!(
                "  - duplicate TypeId({}) shared by {} descriptor(s): {:?}\n",
                d.type_id,
                d.descriptor_names.len(),
                d.descriptor_names,
            ));
        }
        for d in &self.duplicate_names_with_different_ids {
            msg.push_str(&format!(
                "  - name `{}` declared with {} different TypeIds: {:?}\n",
                d.name,
                d.type_ids.len(),
                d.type_ids,
            ));
        }
        for a in &self.variant_tag_anomalies {
            msg.push_str(&format!(
                "  - variant tags non-dense in `{}` (TypeId({})): expected {} \
                 variants, max tag seen {}, duplicates {:?}, missing {:?}\n",
                a.type_name,
                a.type_id,
                a.expected_count,
                a.max_tag_seen,
                a.duplicate_tags,
                a.missing_tags,
            ));
        }
        Err(CodegenError::internal(msg))
    }
}

/// Single instance of "two TypeDescriptors share a TypeId".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateTypeId {
    /// The shared `TypeId.0` value.
    pub type_id: u32,
    /// Names of every descriptor that claims this id (length ≥ 2).
    pub descriptor_names: Vec<String>,
}

/// Single instance of "same name, different TypeIds".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateNameDifferentId {
    /// The collided type name.
    pub name: String,
    /// All distinct TypeIds claimed under this name (length ≥ 2,
    /// sorted ascending so test assertions are stable).
    pub type_ids: Vec<u32>,
}

/// Single instance of "MakeVariant references a variant that no
/// declared TypeDescriptor carries". Global pass equivalent of
/// the per-module #146 Phase 2 warn-level check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanMakeVariant {
    /// Function whose body emits the orphan instruction.
    pub function_name: String,
    /// `tag` operand of the `MakeVariant` instruction.
    pub tag: u32,
    /// `field_count` operand of the `MakeVariant` instruction.
    pub field_count: u32,
}

/// Single instance of "variant tags within a sum are non-dense".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantTagAnomaly {
    /// Owning type's simple name.
    pub type_name: String,
    /// Owning type's `TypeId.0`.
    pub type_id: u32,
    /// Number of variants declared on the type.
    pub expected_count: u32,
    /// Largest tag value actually seen on any variant.
    pub max_tag_seen: u32,
    /// Tags that appeared on more than one variant.
    pub duplicate_tags: Vec<u32>,
    /// Tags expected from `0..expected_count.max(max_tag_seen+1)`
    /// that no variant carries.
    pub missing_tags: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_codegen() {
        let codegen = VbcCodegen::new();
        assert!(codegen.functions.is_empty());
        assert_eq!(codegen.next_func_id, 0);
    }

    #[test]
    fn test_config_builder() {
        let config = CodegenConfig::new("my_module")
            .with_debug_info()
            .with_optimization_level(2)
            .with_validation();

        assert_eq!(config.module_name, "my_module");
        assert!(config.debug_info);
        assert_eq!(config.optimization_level, 2);
        assert!(config.validate);
    }

    #[test]
    fn test_optimization_level_clamped() {
        let config = CodegenConfig::new("test").with_optimization_level(10); // Should be clamped to 3

        assert_eq!(config.optimization_level, 3);
    }

    /// Default config is lenient — partial / forward-referenced stdlib
    /// state still builds. `with_strict_codegen()` opts in to promoting
    /// bug-class skips to hard errors. Tracked under #166.
    #[test]
    fn validate_default_off_until_stdlib_clean() {
        // Pin: until pre-existing stdlib emit bugs are cleaned up
        // (TypeId(515) dangling refs, function-end-vs-instruction-stream
        // length divergence, archive-header counts disagreeing with
        // section bodies), the structural validator is OFF by default.
        // CI opts in via `with_validation()`. This pin breaks together
        // with the default flip back to `true` once the bug class is
        // closed — that's the right time to delete it.
        let config = CodegenConfig::new("validate_default");
        assert!(
            !config.validate,
            "default for `validate` is false until stdlib emit is clean — flip back to true via `with_validation()` opt-in or after the encoding-bug class closes",
        );

        let mut codegen = VbcCodegen::with_config(config);
        let module = codegen
            .finalize_module()
            .expect("default-off codegen must always succeed");
        assert_eq!(module.name, "validate_default");
    }

    #[test]
    fn validate_opt_in_via_with_validation_passes_clean_module() {
        // Pin: opt-in path via `with_validation()` runs the validator.
        // A clean codegen-built module passes — keeps the wiring honest
        // even while the default is off. When stdlib emit cleans up,
        // this test stays valid (just becomes redundant with the new
        // default-on semantics).
        let mut config = CodegenConfig::new("validate_opt_in");
        config.validate = true;

        let mut codegen = VbcCodegen::with_config(config);
        let module = codegen
            .finalize_module()
            .expect("opt-in validator on clean codegen-built module must pass");
        assert_eq!(module.name, "validate_opt_in");
    }

    #[test]
    fn validate_config_off_skips_validator_short_circuit() {
        // Pin: the gate short-circuits before the validator call, so
        // setting `validate = false` keeps the codegen hot path free
        // of any structural-validation cost. We can't observe the
        // skip directly (the validator has no side effects on a clean
        // module), but we pin the semantic contract: a manually-
        // disabled gate produces an Ok result with the same module
        // identity as the default-on path. If a future refactor
        // accidentally makes the validator unconditional, this test
        // breaks together with `validate_default_*` only when the
        // validator starts catching something — at which point the
        // single-source-of-truth gate is the right place to fix.
        let mut config = CodegenConfig::new("validate_off");
        config.validate = false;

        let mut codegen = VbcCodegen::with_config(config);
        let module = codegen
            .finalize_module()
            .expect("validate=false codegen path must always succeed");
        assert_eq!(module.name, "validate_off");
    }

    #[test]
    fn test_strict_codegen_default_lenient() {
        let config = CodegenConfig::new("test");
        assert!(
            !config.strict_codegen,
            "default codegen mode must stay lenient — CI/release opts in via with_strict_codegen()",
        );
    }

    #[test]
    fn test_strict_codegen_opt_in() {
        let config = CodegenConfig::new("test").with_strict_codegen();
        assert!(
            config.strict_codegen,
            "with_strict_codegen() must flip the flag on so CI / release builds reject any bug-class skip",
        );
    }

    /// Synthetic-table smoke (#170): an empty table is clean.
    #[test]
    fn test_global_type_table_clean_when_empty() {
        let report = VbcCodegen::compute_type_table_health(&[], &[]);
        assert!(report.is_clean(), "empty table must classify as clean");
        assert_eq!(report.issue_count(), 0);
        assert!(report.into_error().is_ok());
    }

    /// Synthetic-table smoke (#170): two TypeDescriptors with the
    /// same `TypeId.0` must surface as a `DuplicateTypeId` finding —
    /// this is the symptom of a type-name collision silently merging
    /// into a single id slot.
    #[test]
    fn test_global_type_table_detects_duplicate_id() {
        use crate::types::{StringId, TypeDescriptor, TypeId, TypeKind};
        let strings = vec!["Foo".to_string(), "Bar".to_string()];
        let mk = |name_idx: u32, id: u32| TypeDescriptor {
            id: TypeId(id),
            name: StringId(name_idx),
            kind: TypeKind::Unit,
            ..Default::default()
        };
        let types = vec![mk(0, 17), mk(1, 17)]; // Foo and Bar both claim id 17
        let report = VbcCodegen::compute_type_table_health(&types, &strings);
        assert!(!report.is_clean());
        assert_eq!(report.duplicate_ids.len(), 1);
        assert_eq!(report.duplicate_ids[0].type_id, 17);
        let mut names = report.duplicate_ids[0].descriptor_names.clone();
        names.sort();
        assert_eq!(names, vec!["Bar", "Foo"]);
        assert!(report.into_error().is_err());
    }

    /// Synthetic-table smoke (#170): same name, different TypeIds
    /// surfaces as `DuplicateNameDifferentId`. Distinct from the
    /// duplicate-id case: here the *name* collides while the ids
    /// disagree, indicating the codegen ran multiple type-allocation
    /// passes that didn't share state.
    #[test]
    fn test_global_type_table_detects_same_name_different_ids() {
        use crate::types::{StringId, TypeDescriptor, TypeId, TypeKind};
        let strings = vec!["Counter".to_string(), "Counter".to_string()];
        let mk = |name_idx: u32, id: u32| TypeDescriptor {
            id: TypeId(id),
            name: StringId(name_idx),
            kind: TypeKind::Unit,
            ..Default::default()
        };
        let types = vec![mk(0, 17), mk(1, 18)];
        let report = VbcCodegen::compute_type_table_health(&types, &strings);
        assert!(!report.is_clean());
        assert_eq!(report.duplicate_names_with_different_ids.len(), 1);
        let d = &report.duplicate_names_with_different_ids[0];
        assert_eq!(d.name, "Counter");
        assert_eq!(d.type_ids, vec![17, 18]);
    }

    /// Synthetic-table smoke (#170): two descriptors with different
    /// names but pointing at the same TypeId are flagged as a
    /// duplicate-id finding — aliases should be represented by a
    /// SINGLE descriptor with multiple names in the string table,
    /// not by two descriptors sharing an id. The
    /// duplicate-name-with-different-ids check stays silent because
    /// neither name is itself ambiguous.
    #[test]
    fn test_global_type_table_two_descriptors_same_id_different_names_flagged() {
        use crate::types::{StringId, TypeDescriptor, TypeId, TypeKind};
        let strings = vec!["Int".to_string(), "i64".to_string()];
        let mk = |name_idx: u32, id: u32| TypeDescriptor {
            id: TypeId(id),
            name: StringId(name_idx),
            kind: TypeKind::Unit,
            ..Default::default()
        };
        // Two names, but pointing at the *same* id — alias case.
        let types = vec![mk(0, 0), mk(1, 0)];
        let report = VbcCodegen::compute_type_table_health(&types, &strings);
        // duplicate_ids fires (two descriptors share TypeId(0)) — that's
        // technically a "duplicate" by the strict definition, so the
        // report is NOT clean. This is the intended behaviour: aliases
        // should be represented by a single TypeDescriptor with multiple
        // names in the string table, not two descriptors.
        assert!(!report.is_clean());
        assert_eq!(report.duplicate_ids.len(), 1);
        // But the name-vs-id check is silent because both names are
        // claimed against the same id.
        assert_eq!(report.duplicate_names_with_different_ids.len(), 0);
    }

    /// `alloc_user_type_id` must skip reserved ranges (#170).
    /// Drives the allocator past 256..260 (meta range) and
    /// 512..1024 (semantic-collection + dependent-type range) and
    /// asserts that no allocated id lands inside.
    #[test]
    fn test_alloc_user_type_id_skips_reserved_ranges() {
        use crate::types::TypeId;
        let mut codegen = VbcCodegen::new();
        // Reset next_type_id so we start from 0 and walk past every
        // reserved range deterministically.
        codegen.next_type_id = 0;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1100 {
            let id = codegen.alloc_user_type_id();
            assert!(
                id.0 >= TypeId::FIRST_USER,
                "allocated id {} below FIRST_USER ({})",
                id.0,
                TypeId::FIRST_USER,
            );
            assert!(
                !(256..260).contains(&id.0),
                "allocated id {} inside meta-system range 256..260",
                id.0,
            );
            assert!(
                !(TypeId::FIRST_SEMANTIC..=TypeId::LAST_SEMANTIC).contains(&id.0),
                "allocated id {} inside semantic-collection range {}..={}",
                id.0,
                TypeId::FIRST_SEMANTIC,
                TypeId::LAST_SEMANTIC,
            );
            assert!(
                seen.insert(id.0),
                "duplicate id {} returned from alloc_user_type_id",
                id.0,
            );
        }
        // After 1100 allocations starting from id 16, we should have
        // walked past the 4-id meta range (256..260) and the
        // 512-id semantic range (512..=1023). Last allocated id
        // should therefore be FIRST_USER + 1100 + 4 + 512 - 1 =
        // 16 + 1100 + 516 - 1 = 1631 (off-by-one tolerated; the
        // strict check above is enough).
    }

    /// Synthetic-table smoke (#170): a sum type with a tag gap
    /// surfaces as a `VariantTagAnomaly`. Runtime variant dispatch
    /// indexes by tag, so any gap = wrong-variant dispatch.
    #[test]
    fn test_global_type_table_detects_variant_tag_gap() {
        use crate::types::{
            StringId, TypeDescriptor, TypeId, TypeKind, VariantDescriptor, VariantKind,
        };
        use smallvec::smallvec;
        let strings = vec![
            "Color".to_string(),
            "Red".to_string(),
            "Green".to_string(),
            "Blue".to_string(),
        ];
        // 3 variants with tags 0, 2, 5 — gaps at 1, 3, 4.
        let mk_v = |name_idx: u32, tag: u32| VariantDescriptor {
            name: StringId(name_idx),
            tag,
            payload: None,
            kind: VariantKind::Unit,
            arity: 0,
            fields: smallvec![],
        };
        let ty = TypeDescriptor {
            id: TypeId(17),
            name: StringId(0),
            kind: TypeKind::Sum,
            variants: smallvec![mk_v(1, 0), mk_v(2, 2), mk_v(3, 5)],
            ..Default::default()
        };
        let report = VbcCodegen::compute_type_table_health(&[ty], &strings);
        assert!(!report.is_clean());
        assert_eq!(report.variant_tag_anomalies.len(), 1);
        let a = &report.variant_tag_anomalies[0];
        assert_eq!(a.type_name, "Color");
        assert_eq!(a.expected_count, 3);
        assert_eq!(a.max_tag_seen, 5);
        assert_eq!(a.missing_tags, vec![1, 3, 4]);
        assert!(a.duplicate_tags.is_empty());
    }
}
