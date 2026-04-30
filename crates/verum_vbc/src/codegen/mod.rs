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
//!       │
//!       ▼
//! ┌─────────────────────────────────────────┐
//! │            VbcCodegen                    │
//! │  ┌───────────────────────────────────┐  │
//! │  │      CodegenContext               │  │
//! │  │  - RegisterAllocator              │  │
//! │  │  - Label management               │  │
//! │  │  - Loop/defer stacks              │  │
//! │  │  - Constant pool                  │  │
//! │  └───────────────────────────────────┘  │
//! │                                         │
//! │  compile_module() → VbcModule          │
//! │    ├─ compile_function()               │
//! │    │   ├─ compile_block()              │
//! │    │   │   ├─ compile_stmt()           │
//! │    │   │   │   └─ compile_expr()       │
//! │    │   │   │       ├─ literals         │
//! │    │   │   │       ├─ binary ops       │
//! │    │   │   │       ├─ calls            │
//! │    │   │   │       └─ control flow     │
//! │    │   │   └─ ...                      │
//! │    │   └─ ...                          │
//! │    └─ ...                              │
//! └─────────────────────────────────────────┘
//!       │
//!       ▼
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
pub use error::{CodegenError, CodegenErrorKind, CodegenResult, SkipClass};
pub use registers::{RegisterAllocator, RegisterInfo, RegisterKind, RegisterSnapshot};

use crate::types::CbgrTier;
use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_common::Map;
use verum_common::well_known_types::WellKnownType as WKT;

use crate::instruction::{Instruction, Reg};
use crate::module::{
    CallingConvention as FfiCallingConvention, CType, ErrorProtocol, FfiLibrary, FfiLibraryId,
    FfiOwnership, FfiPlatform, FfiSignature, FfiStructField, FfiStructLayout, FfiSymbol, FfiSymbolId,
    FunctionDescriptor, FunctionId, MemoryEffects, ParamDescriptor, VbcFunction, VbcModule,
};
use crate::types::{StringId, TypeDescriptor, TypeId, TypeRef};
use crate::validate;

use verum_ast::decl::{ExternBlockDecl, MountDecl, MountTree, MountTreeKind, TypeDeclBody, VariantData};
use verum_ast::ffi::{FFIBoundary, CallingConvention as AstCallingConvention};
use verum_ast::ty::PathSegment;
use verum_ast::{Block, FunctionBody, FunctionDecl, Item, ItemKind, Module, StmtKind};
use verum_ast::bitfield::ByteOrder;

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

    /// Pending constants that need bytecode compilation.
    /// These are constants whose values couldn't be inlined (e.g., struct literals).
    /// Stored as (function_name, expression_clone) for compilation in compile_function_bodies.
    pending_constants: Vec<(String, verum_ast::Expr)>,

    /// Map from type name to TypeId for user-defined types.
    /// Used to emit correct type_id in New instructions for proper Drop dispatch.
    type_name_to_id: std::collections::HashMap<String, crate::types::TypeId>,

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
    /// emitted bytecode.  Runtime calls panic with `FunctionNotFound`.
    /// This is the dev-loop default — it lets partial / forward-referenced
    /// stdlib state still build.
    ///
    /// `true` (opt-in via `with_strict_codegen`): bug-class failures are
    /// converted into a hard `CodegenError` returned from
    /// `compile_module_items_lenient`, halting the build at the first
    /// such failure.  `Irreducible` failures (FFI prototype, unimplemented
    /// language feature) continue to skip silently — these represent the
    /// documented Tier-0 contract, not bugs.
    ///
    /// Intended for CI and release builds where any bug-class skip is a
    /// regression that must block the merge.  Tracked under #166
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
    ///     .with_target(TargetConfig::linux_x86_64());
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
    /// codegen resource exhaustion, parser/lowering bug).  `Irreducible`
    /// errors (interpreter limitation — FFI prototype, unimplemented
    /// feature) continue to skip silently because they represent the
    /// documented Tier-0 contract.
    ///
    /// Intended for CI / release builds.  See the `strict_codegen` field
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
                if (WKT::Heap.matches(&base_name) || WKT::Shared.matches(&base_name)) && args.len() == 1 {
                    // Transparent wrapper — return inner type for method dispatch
                    if let verum_ast::ty::GenericArg::Type(inner_ty) = &args[0] {
                        return self.type_to_simple_name(inner_ty);
                    }
                }
                base_name
            }
            verum_ast::ty::TypeKind::Path(path) => {
                // Get the last segment name (simple type name)
                path.segments.iter().rev().find_map(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    }
                }).unwrap_or_else(|| "Unknown".to_string())
            }
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string()).unwrap_or_else(|| "Unknown".to_string())
            }
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
            verum_ast::ty::TypeKind::Generic { base, .. } => {
                self.extract_base_type_name(base)
            }
            // Path type: extract the last segment name
            verum_ast::ty::TypeKind::Path(path) => {
                path.segments.iter().rev().find_map(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    }
                })
            }
            // Primitive types
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string())
            }
            _ => None,
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
    pub fn is_builtin_ctor_collection(&self, name: &str) -> bool {
        self.builtin_ctor_collections.contains(name)
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

    /// Resolves a generic return type name (e.g., "V", "K", "T") to a concrete type
    /// by extracting the corresponding type arg from a parameterized receiver type.
    /// E.g., receiver="Map<Int, Node>", base="Map", ret="V" → Some("Node")
    /// because Map has params [K, V] and V is at index 1.
    pub fn resolve_generic_return_type(
        &self, receiver_type: &str, base_type: &str, generic_name: &str,
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
            let is_bare_generic = first_arg.len() <= 2
                && first_arg.chars().all(|c| c.is_ascii_uppercase());
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
            tracing::warn!(
                "CodegenConfig surface: debug_info=true (this field lands on \
                 the config but VBC codegen does not currently emit DWARF-\
                 style debug info — only the narrower `source_map` flag, \
                 which controls line/col tracking via debug_vars, is wired)",
            );
        }

        Self {
            ctx: CodegenContext::new(),
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
            // Track variant name collisions
            variant_collisions: std::collections::HashSet::new(),
            // Bitfield type layouts for @bitfield types
            bitfield_types: std::collections::HashMap::new(),
            // Field name indices for record field access
            field_name_indices: std::collections::HashMap::new(),
            next_field_id: 0,
            type_field_layouts: std::collections::HashMap::new(),
            type_field_type_names: std::collections::HashMap::new(),
            // Pending constants for deferred compilation
            pending_constants: Vec::new(),
            // Type name to TypeId mapping for Drop dispatch.
            // Pre-populated with all well-known type names and their aliases so that
            // ast_type_to_type_ref and type_ref_for_type_kind can do a single lookup
            // instead of hardcoded match arms.
            type_name_to_id: {
                let mut m = std::collections::HashMap::new();
                use crate::types::TypeId;
                // Primitives and their aliases
                m.insert("Int".to_string(), TypeId::INT);
                m.insert("Int64".to_string(), TypeId::INT);
                m.insert("i64".to_string(), TypeId::INT);
                m.insert("Int32".to_string(), TypeId::I32);
                m.insert("i32".to_string(), TypeId::I32);
                m.insert("Int16".to_string(), TypeId::I16);
                m.insert("i16".to_string(), TypeId::I16);
                m.insert("Int8".to_string(), TypeId::I8);
                m.insert("i8".to_string(), TypeId::I8);
                m.insert("UInt64".to_string(), TypeId::U64);
                m.insert("u64".to_string(), TypeId::U64);
                m.insert("UInt32".to_string(), TypeId::U32);
                m.insert("u32".to_string(), TypeId::U32);
                m.insert("UInt16".to_string(), TypeId::U16);
                m.insert("u16".to_string(), TypeId::U16);
                m.insert("UInt8".to_string(), TypeId::U8);
                m.insert("u8".to_string(), TypeId::U8);
                m.insert("Byte".to_string(), TypeId::U8);
                m.insert("Float".to_string(), TypeId::FLOAT);
                m.insert("Float64".to_string(), TypeId::FLOAT);
                m.insert("f64".to_string(), TypeId::FLOAT);
                m.insert("Float32".to_string(), TypeId::F32);
                m.insert("f32".to_string(), TypeId::F32);
                m.insert("Bool".to_string(), TypeId::BOOL);
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
                // Pointer/wrapper types
                m.insert("Heap".to_string(), TypeId::PTR);
                m.insert("Shared".to_string(), TypeId::PTR);
                m
            },
            // Collection type generic parameter name templates.
            // Used by resolve_generic_return_type to map generic names to positions.
            collection_type_params: {
                let mut m = std::collections::HashMap::new();
                m.insert("Map".to_string(), vec!["K".to_string(), "V".to_string()]);
                m.insert("BTreeMap".to_string(), vec!["K".to_string(), "V".to_string()]);
                m.insert("List".to_string(), vec!["T".to_string()]);
                m.insert("Set".to_string(), vec!["T".to_string()]);
                m.insert("BTreeSet".to_string(), vec!["T".to_string()]);
                m.insert("Deque".to_string(), vec!["T".to_string()]);
                m.insert("Channel".to_string(), vec!["T".to_string()]);
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
    pub fn import_functions(&mut self, functions: &std::collections::HashMap<String, FunctionInfo>) {
        self.ctx.import_functions(functions);
        // Update next_func_id to avoid ID conflicts
        if let Some(max_id) = functions.values().map(|f| f.id.0).max()
            && max_id >= self.next_func_id {
                self.next_func_id = max_id.saturating_add(1);
            }
    }

    /// Imports protocols from previously compiled modules.
    ///
    /// This is used during stdlib compilation to make protocol default
    /// methods from earlier modules available for impl blocks in later modules.
    /// Iteration is sorted by name so that downstream codegen sees
    /// protocols in a deterministic order — this matters because some
    /// later passes assign function IDs in iteration order.
    pub fn import_protocols(&mut self, protocols: &std::collections::HashMap<String, ProtocolInfo>) {
        let mut sorted: Vec<&String> = protocols.keys().collect();
        sorted.sort();
        for name in sorted {
            let info = &protocols[name];
            self.protocol_registry.entry(name.clone()).or_insert_with(|| info.clone());
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
                && let TypeDeclBody::Protocol(protocol_body) = &type_decl.body {
                    let protocol_name = type_decl.name.name.to_string();
                    let mut default_methods = std::collections::HashMap::new();

                    // Extract superprotocol names from extends clause
                    let super_protocols: Vec<String> = protocol_body.extends.iter()
                        .filter_map(|ty| {
                            // Extract protocol name from type (e.g., Named { path: "PartialEq", .. })
                            if let verum_ast::ty::Type {
                                kind: verum_ast::ty::TypeKind::Path(path),
                                ..
                            } = ty {
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
                        if let ProtocolItemKind::Function { decl, default_impl: verum_common::Maybe::Some(body) } = &protocol_item.kind {
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

                    // Register protocol info even if no default methods - needed for inheritance tracking
                    if !default_methods.is_empty() || !super_protocols.is_empty() {
                        self.protocol_registry.insert(protocol_name.clone(), ProtocolInfo {
                            name: protocol_name,
                            default_methods,
                            super_protocols,
                        });
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
        if let Some(name) = Self::extract_source_module_name(module) {
            self.ctx.current_source_module = Some(name);
        }
        let module_name = self.ctx.current_source_module.clone().unwrap_or_default();
        let funcs_before = self.ctx.functions.len();
        let result = self.collect_all_declarations(module);
        let funcs_after = self.ctx.functions.len();
        // #200 diagnostic: surface decl-collection per-module net change so
        // a silent decl-drop (returning Err that drops items mid-walk) is
        // visible at trace level.  Triggered via `RUST_LOG=trace`.
        match &result {
            Ok(()) => {
                tracing::trace!(
                    "[decl-collect] {} ok: +{} funcs (total {})",
                    module_name, funcs_after - funcs_before, funcs_after
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[decl-collect] {} ERR: +{} funcs registered before fail: {}",
                    module_name, funcs_after - funcs_before, e
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

    /// Marks all type names in a module as user-defined.
    /// This allows bare variant disambiguation to prefer user types over stdlib types.
    pub fn mark_user_defined_types(&mut self, module: &Module) {
        for item in module.items.iter() {
            if let ItemKind::Type(type_decl) = &item.kind {
                self.ctx.user_defined_types.insert(type_decl.name.name.to_string());
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
        let mut per_proto_overrides: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
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
                per_proto_overrides.entry(derived.clone()).or_insert(overrides);
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

                // Iterate default methods in a deterministic order.  Without
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
                    self.pending_default_methods.push((default_func.clone(), type_name.to_string()));
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
                if let TypeKind::Path(p) = &ty.kind { p } else { return None; }
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
                let already_compiled = self.functions.iter().any(|f| f.descriptor.id == func_info.id);
                if already_compiled {
                    continue;
                }
            }

            // Compile the function body
            self.ctx.generic_type_params.clear();
            self.ctx.const_generic_params.clear();
            if let Err(_e) = self.compile_function(&default_func, Some(&type_name)) {
                // Skip - some default methods may have unresolvable dependencies
                // (e.g., FFI functions, external symbols not available in VBC)
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
            // Atomic ordering constants (core/intrinsics/atomic.vr)
            ("ORDERING_RELAXED", 0),
            ("ORDERING_ACQUIRE", 1),
            ("ORDERING_RELEASE", 2),
            ("ORDERING_ACQ_REL", 3),
            ("ORDERING_SEQ_CST", 4),
            // POSIX errno constants (core/sys/darwin/errno.vr, core/sys/linux/errno.vr)
            ("EPERM", 1),
            ("ENOENT", 2),
            ("ESRCH", 3),
            ("EINTR", 4),
            ("EIO", 5),
            ("ENXIO", 6),
            ("E2BIG", 7),
            ("ENOEXEC", 8),
            ("EBADF", 9),
            ("ECHILD", 10),
            ("EAGAIN", 11),
            ("ENOMEM", 12),
            ("EACCES", 13),
            ("EFAULT", 14),
            ("EBUSY", 16),
            ("EEXIST", 17),
            ("ENODEV", 19),
            ("ENOTDIR", 20),
            ("EISDIR", 21),
            ("EINVAL", 22),
            ("EMFILE", 24),
            ("ENOSPC", 28),
            ("EPIPE", 32),
            ("ERANGE", 34),
            ("ENOSYS", 78),
            ("ENOTEMPTY", 66),
            ("ECONNREFUSED", 61),
            ("ECONNRESET", 54),
            ("ECONNABORTED", 53),
            ("ETIMEDOUT", 60),
            ("EADDRINUSE", 48),
            ("EADDRNOTAVAIL", 49),
            ("ENETUNREACH", 51),
            ("EALREADY", 37),
            ("EINPROGRESS", 36),
            ("ENOTCONN", 57),
            ("EWOULDBLOCK", 35),
            // kqueue filter constants (core/sys/darwin/libsystem.vr)
            ("EVFILT_READ", -1),
            ("EVFILT_WRITE", -2),
            ("EVFILT_TIMER", -7),
            ("EVFILT_USER", -10),
            // kqueue flags
            ("EV_ADD", 0x0001),
            ("EV_DELETE", 0x0002),
            ("EV_ENABLE", 0x0004),
            ("EV_DISABLE", 0x0008),
            ("EV_CLEAR", 0x0020),
            ("EV_ONESHOT", 0x0010),
            ("EV_EOF", 0x8000_i64),
            ("EV_ERROR", 0x4000),
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
            ("SEEK_SET", 0),
            ("SEEK_CUR", 1),
            ("SEEK_END", 2),
            // CBGR generation constants (core/mem/allocator.vr, core/mem/header.vr)
            ("GEN_INITIAL", 1),
            ("GEN_DEAD", 0),
            ("HEADER_SIZE", 16),
            ("FLAG_ARENA", 4),
            // Capability constants (core/mem/capability.vr)
            ("CAP_READ", 1),
            ("CAP_WRITE", 2),
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
            ("PROT_READ", 1),
            ("PROT_WRITE", 2),
            ("PROT_NONE", 0),
            ("MAP_PRIVATE", 2),
            ("MAP_ANONYMOUS", 0x20),
            ("MAP_ANON", 0x1000),
            ("MAP_HUGETLB", 0x40000),
            ("MADV_HUGEPAGE", 14),
            ("MEM_COMMIT", 0x1000),
            ("MEM_RESERVE", 0x2000),
            ("MEM_RELEASE", 0x8000_i64),
            ("MEM_LARGE_PAGES", 0x20000000),
            ("PAGE_READWRITE", 4),
            ("PAGE_NOACCESS", 1),
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
            ("MADV_FREE", 8),
            // Network/socket constants (core/net/, core/sys/)
            ("AF_INET", 2),
            ("AF_INET6", 30),
            ("SOCK_STREAM", 1),
            ("SOCK_DGRAM", 2),
            ("SOL_SOCKET", 0xFFFF_i64),
            ("SO_REUSEADDR", 2),
            ("SO_KEEPALIVE", 8),
            ("IPPROTO_TCP", 6),
            ("TCP_NODELAY", 1),
            // File open flags
            ("O_RDONLY", 0),
            ("O_WRONLY", 1),
            ("O_RDWR", 2),
            ("O_CREAT", 0x200),
            ("O_TRUNC", 0x400),
            ("O_APPEND", 8),
            ("O_CLOEXEC", 0x1000000),
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
            ("O_DIRECTORY", 0x100000),
            ("O_NOFOLLOW", 0x100),
            ("O_NONBLOCK", 0x4),
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
            // Network message flags
            ("MSG_PEEK", 2),
            ("MSG_DONTWAIT", 0x40),
            ("MSG_WAITALL", 0x40),
            // Shutdown constants (core/net/)
            ("SHUT_RD", 0),
            ("SHUT_WR", 1),
            ("SHUT_RDWR", 2),
            // Socket type flags (linux)
            ("SOCK_NONBLOCK", 0x800),
            ("SOCK_CLOEXEC", 0x80000),
            // Socket option levels
            ("SOL_TCP", 6),
            ("SO_ERROR", 4),
        ];

        // Variant constructors that need tags for pattern matching and ? operator
        // Declaration-order variant tags for built-in sum types; must
        // agree with register_type_constructors and the register_builtin
        // table near compile_program.
        let variant_tags: &[(&str, u32)] = &[
            ("Ok", 0), ("Err", 1),
            ("None", 0), ("Some", 1),
            ("Less", 0), ("Equal", 1), ("Greater", 2),
            ("Continue", 0), ("Break", 1),
            ("True", 1), ("False", 0),
        ];
        let tag_map: std::collections::HashMap<&str, u32> = variant_tags.iter().copied().collect();

        for &(name, _value) in constants {
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
                parent_type_name: None,
                variant_payload_types: None,
                is_partial_pattern: false, takes_self_mut_ref: false,
                return_type_name: None,
                return_type_inner: None,
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
            ("PerfCounter.elapsed_since", 2, "time_perf_counter_elapsed_since"),
            ("PerfCounter.as_nanos", 1, "time_perf_counter_as_nanos"),
            // DeadlineTimer impl methods
            ("DeadlineTimer.from_duration", 1, "time_deadline_timer_from_duration"),
            ("DeadlineTimer.is_expired", 1, "time_deadline_timer_is_expired"),
            ("DeadlineTimer.remaining", 1, "time_deadline_timer_remaining"),
            // Arithmetic intrinsics (core/intrinsics/arithmetic.vr)
            ("add", 2, "add"), ("sub", 2, "sub"), ("mul", 2, "mul"),
            ("div", 2, "div"), ("rem", 2, "rem"), ("neg", 1, "neg"),
            ("abs_signed", 1, "abs_signed"), ("signum", 1, "signum"),
            ("min", 2, "min"), ("max", 2, "max"), ("clamp", 3, "clamp"),
            ("checked_add", 2, "checked_add"), ("checked_sub", 2, "checked_sub"),
            ("checked_mul", 2, "checked_mul"), ("checked_div", 2, "checked_div"),
            ("checked_add_u64", 2, "checked_add_u64"), ("checked_sub_u64", 2, "checked_sub_u64"),
            ("checked_mul_u64", 2, "checked_mul_u64"),
            ("overflowing_add", 2, "overflowing_add"), ("overflowing_sub", 2, "overflowing_sub"),
            ("overflowing_mul", 2, "overflowing_mul"),
            ("wrapping_add", 2, "wrapping_add"), ("wrapping_sub", 2, "wrapping_sub"),
            ("wrapping_mul", 2, "wrapping_mul"), ("wrapping_neg", 1, "wrapping_neg"),
            ("wrapping_shl", 2, "wrapping_shl"), ("wrapping_shr", 2, "wrapping_shr"),
            ("saturating_add", 2, "saturating_add"), ("saturating_sub", 2, "saturating_sub"),
            ("saturating_mul", 2, "saturating_mul"),
            // Comparison intrinsics
            ("eq", 2, "eq"), ("ne", 2, "ne"), ("lt", 2, "lt"),
            ("le", 2, "le"), ("gt", 2, "gt"), ("ge", 2, "ge"),
            // Bitwise intrinsics (core/intrinsics/bitwise.vr)
            ("clz", 1, "clz"), ("ctz", 1, "ctz"), ("bswap", 1, "bswap"),
            ("bitreverse", 1, "bitreverse"), ("rotl", 2, "rotl"), ("rotr", 2, "rotr"),
            ("bitand", 2, "bitand"), ("bitor", 2, "bitor"), ("bitxor", 2, "bitxor"),
            ("bitnot", 1, "bitnot"), ("shl", 2, "shl"), ("shr", 2, "shr"),
            // Conversion intrinsics (core/intrinsics/conversion.vr)
            ("int_to_float", 1, "int_to_float"), ("float_to_int", 1, "float_to_int"),
            ("f32_to_bits", 1, "f32_to_bits"), ("f32_from_bits", 1, "f32_from_bits"),
            ("f64_to_bits", 1, "f64_to_bits"), ("f64_from_bits", 1, "f64_from_bits"),
            ("to_le_bytes", 1, "to_le_bytes"), ("to_be_bytes", 1, "to_be_bytes"),
            ("from_le_bytes", 1, "from_le_bytes"), ("from_be_bytes", 1, "from_be_bytes"),
            ("to_le_bytes_2", 1, "to_le_bytes_2"), ("to_le_bytes_4", 1, "to_le_bytes_4"),
            ("to_le_bytes_8", 1, "to_le_bytes_8"),
            ("to_be_bytes_2", 1, "to_be_bytes_2"), ("to_be_bytes_4", 1, "to_be_bytes_4"),
            ("to_be_bytes_8", 1, "to_be_bytes_8"),
            ("from_le_bytes_2", 1, "from_le_bytes_2"), ("from_le_bytes_4", 1, "from_le_bytes_4"),
            ("from_le_bytes_8", 1, "from_le_bytes_8"),
            ("from_be_bytes_2", 1, "from_be_bytes_2"), ("from_be_bytes_4", 1, "from_be_bytes_4"),
            ("from_be_bytes_8", 1, "from_be_bytes_8"),
            // Float intrinsics (core/intrinsics/float.vr)
            ("f32_infinity", 0, "f32_infinity"), ("f32_neg_infinity", 0, "f32_neg_infinity"),
            ("f32_nan", 0, "f32_nan"),
            ("sqrt", 1, "sqrt"), ("cbrt", 1, "cbrt"),
            ("exp", 1, "exp"), ("expm1", 1, "expm1"), ("exp2", 1, "exp2"),
            ("log", 1, "log"), ("log1p", 1, "log1p"), ("log10", 1, "log10"),
            ("log2", 1, "log2"), ("pow", 2, "pow"), ("powi", 2, "powi"),
            ("floor", 1, "floor"), ("ceil", 1, "ceil"), ("round", 1, "round"),
            ("trunc", 1, "trunc"), ("fabs", 1, "fabs"),
            ("minnum", 2, "minnum"), ("maxnum", 2, "maxnum"),
            ("fma", 3, "fma"), ("copysign", 2, "copysign"), ("hypot", 2, "hypot"),
            ("sin", 1, "sin"), ("cos", 1, "cos"), ("tan", 1, "tan"),
            ("asin", 1, "asin"), ("acos", 1, "acos"), ("atan", 1, "atan"),
            ("atan2", 2, "atan2"),
            ("sinh", 1, "sinh"), ("cosh", 1, "cosh"), ("tanh", 1, "tanh"),
            ("asinh", 1, "asinh"), ("acosh", 1, "acosh"), ("atanh", 1, "atanh"),
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
            ("set_multicast_loop_v4", 2, "net_set_multicast_loop_v4_linux"),
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
                is_partial_pattern: false, takes_self_mut_ref: false,
                return_type_name: None,
                return_type_inner: None,
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
        self.ctx.set_tier_context(TierContext::with_decisions(decisions));
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
        // `should_compile_item`).  Statements don't carry a stable
        // name, so the warn site identifies the function context
        // via `current_function`.
        let (include, failures) = self
            .cfg_evaluator
            .should_include_with_failures(&stmt.attributes);
        if !failures.is_empty() {
            let fn_name = self
                .ctx
                .current_function
                .as_deref()
                .unwrap_or("<unknown>");
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
        // `Function.attributes`), leaving `Item.attributes` empty.  So
        // `Item.attributes`-only checking silently bypasses @cfg gates
        // for every type declaration in the stdlib — `@cfg(target_arch
        // = "x86_64") public type ExceptionFrame is { … };` reaches
        // codegen even on aarch64 hosts, surfacing as duplicate-id
        // findings in #170's global type-table consistency check.
        //
        // Walk the inner decl's attributes when present.
        match &item.kind {
            ItemKind::Type(type_decl)
                if !type_decl.attributes.is_empty() => {
                    let (include, failures) = self
                        .cfg_evaluator
                        .should_include_with_failures(&type_decl.attributes);
                    self.warn_cfg_parse_failures(&failures, item, "TypeDecl");
                    if !include {
                        return false;
                    }
                }
            ItemKind::Function(func)
                if !func.attributes.is_empty() => {
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
    /// predicate failed to parse cleanly.  These attributes are
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
                            && let verum_ast::LiteralKind::Text(s) = &lit.kind {
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
                                && let verum_ast::LiteralKind::Text(s) = &lit.kind {
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
    fn extract_type_layout_hints(attrs: &verum_common::List<verum_ast::attr::Attribute>) -> (u32, bool, bool) {
        let mut alignment = 8u32;
        let mut is_packed = false;
        let mut is_repr_c = false;

        for attr in attrs.iter() {
            match attr.name.as_str() {
                "align" => {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first) = args.first()
                            && let verum_ast::ExprKind::Literal(lit) = &first.kind
                                && let verum_ast::LiteralKind::Int(int_lit) = &lit.kind {
                                    let val = int_lit.value as u32;
                                    if val > 0 && val.is_power_of_two() {
                                        alignment = val;
                                    }
                                }
                }
                "repr" => {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first) = args.first()
                            && let Some(repr_name) = Self::attr_arg_as_ident(first) {
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
                        && let Some(first) = args.first() {
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
                                && let verum_ast::LiteralKind::Int(int_lit) = &lit.kind {
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
                                && let verum_ast::LiteralKind::Text(s) = &lit.kind {
                                    hints.target_features = Some(s.as_str().to_string());
                                }
                }
                "target_cpu" => {
                    if let verum_common::Maybe::Some(args) = &attr.args
                        && let Some(first) = args.first()
                            && let verum_ast::ExprKind::Literal(lit) = &first.kind
                                && let verum_ast::LiteralKind::Text(s) = &lit.kind {
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
            verum_ast::ExprKind::Path(path) => {
                path.segments.last().and_then(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.to_string())
                    } else {
                        None
                    }
                })
            }
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
        if let Some(name) = Self::extract_source_module_name(module) {
            self.ctx.current_source_module = Some(name);
        }
        let mut result = Ok(());
        for item in module.items.iter() {
            if self.should_compile_item(item)
                && let Err(e) = self.compile_item(item) {
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
        if let Some(name) = Self::extract_source_module_name(module) {
            self.ctx.current_source_module = Some(name);
        }
        let mut first_strict_err: Option<CodegenError> = None;
        for item in module.items.iter() {
            if self.should_compile_item(item) {
                // Use lenient item compilation that skips individual functions
                // that fail.  In strict_codegen mode, the helper returns the
                // first `BugClass` error encountered so we can halt the build
                // at the call site instead of papering over a real defect.
                if let Err(e) = self.compile_item_lenient(item)
                    && first_strict_err.is_none() {
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
                    // Symmetric with the impl-item branch below.  Promoted to
                    // warn-level (was debug) so silent skips of user-callable
                    // top-level functions surface as "method/function not
                    // found on value" only AFTER the warning fires, instead
                    // of being completely silent.
                    let fname = func.name.name.as_str();
                    let class = e.skip_class();
                    tracing::warn!(
                        "[lenient] SKIP top-level fn {} ({}): {} — runtime calls \
                         will panic with `FunctionNotFound`",
                        fname, class.label(), e
                    );
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
                                undef_owned, near
                            );
                        } else {
                            tracing::warn!(
                                "[lenient]   no near-matches for '{}' in ctx.functions ({} entries total)",
                                undef_owned, self.ctx.functions.len()
                            );
                        }
                    }
                    tracing::debug!("[lenient] SKIP top-level fn {}: {}", fname, e);
                    if self.config.strict_codegen && class == SkipClass::BugClass && first_strict_err.is_none() {
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

                let impl_type_generics: Vec<String> = impl_decl.generics.iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                let impl_const_generics: Vec<String> = impl_decl.generics.iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Const { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                for impl_item in impl_decl.items.iter() {
                    // Honour `@cfg` gates on impl items.  ImplItem and
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
                        // platform stubs).  But silent skips of *user-callable*
                        // impl-block methods are insidious: they show up at
                        // runtime as `method 'X.Y' not found on value` with no
                        // hint that compilation dropped the body.  Emit a
                        // warn-level trace so the underlying cause (typically
                        // an unresolved cross-module function reference) is
                        // visible in normal CI / dev runs without RUST_LOG
                        // tweaking.
                        if let Err(e) = self.compile_function(func, type_name.as_ref()) {
                            let fname = func.name.name.as_str();
                            let ty = type_name.as_deref().unwrap_or("?");
                            let class = e.skip_class();
                            tracing::warn!(
                                "[lenient] SKIP {}.{} ({}): {} — runtime calls to \
                                 this method will panic 'method '{}.{}' not found \
                                 on value'.  Add the missing dependency to the \
                                 caller's mount list or fix the cross-module \
                                 reference in {} stdlib.",
                                ty, fname, class.label(), e, ty, fname, ty
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
                                        undef_owned, near
                                    );
                                } else {
                                    tracing::warn!(
                                        "[lenient]   no near-matches for '{}' in ctx.functions ({} entries total)",
                                        undef_owned, self.ctx.functions.len()
                                    );
                                }
                            }
                            tracing::debug!("[lenient] SKIP {}.{}: {}", ty, fname, e);
                            if self.config.strict_codegen && class == SkipClass::BugClass && first_strict_err.is_none() {
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
        // Verify type-descriptor self-consistency before emitting bytecode.
        // Catches the class of bugs where codegen produces a TypeDescriptor
        // whose variants disagree with their declared `kind`/`arity`/
        // `fields` shape — historically these surfaced at runtime as
        // `field index N (offset M) exceeds object data size K` /
        // `Null pointer dereference` panics, far from the codegen site.
        self.verify_type_layout_invariants()?;
        // Cross-module type-table consistency check (#170).  In strict
        // mode (`config.strict_codegen = true`), any duplicate-id /
        // same-name-different-id / variant-tag-anomaly finding fails
        // the build with the bundled error.  In lenient mode (default),
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
                        d.type_id, d.descriptor_names,
                    );
                }
                for d in &report.duplicate_names_with_different_ids {
                    tracing::warn!(
                        "[type-table]   name `{}` declared with conflicting \
                         TypeIds: {:?}",
                        d.name, d.type_ids,
                    );
                }
                for a in &report.variant_tag_anomalies {
                    tracing::warn!(
                        "[type-table]   variant tags non-dense in `{}` \
                         (TypeId({})): expected {} variants, max tag {}, \
                         duplicates {:?}, missing {:?}",
                        a.type_name, a.type_id, a.expected_count,
                        a.max_tag_seen, a.duplicate_tags, a.missing_tags,
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
            && let Err(e) = validate::validate_module(&module) {
                return Err(CodegenError::internal(format!(
                    "VBC structural validation failed for module `{}`: {}",
                    module.name,
                    e,
                )));
            }

        Ok(module)
    }

    /// Verify that every `TypeDescriptor` in `self.types` satisfies the
    /// per-variant shape invariants implied by its `VariantKind`.  Runs
    /// at module-finalization time so misshapen descriptors fail loudly
    /// here rather than producing bytecode that crashes at runtime.
    ///
    /// Per-variant invariants:
    ///   * `Unit`   → `arity == 0` and `fields` is empty.
    ///   * `Tuple`  → `arity > 0` and `fields` is empty (the arity
    ///                counts payload elements; tuple variants don't
    ///                use `fields`).
    ///   * `Record` → `arity == 0` and `fields` is non-empty (records
    ///                track their layout in `fields`, not `arity`).
    ///
    /// Cross-variant invariants:
    ///   * Tags within a sum type are dense: `0..variants.len()` with
    ///     no duplicates and no gaps.  The runtime resolves variant
    ///     dispatch by indexing into the variants array by tag, so any
    ///     gap or duplicate yields wrong-variant dispatch later.
    ///
    /// Spec hooks: `verum_vbc::types::VariantKind`,
    /// `verum_vbc::types::VariantDescriptor`.
    pub fn verify_type_layout_invariants(&self) -> CodegenResult<()> {
        Self::check_type_layout_invariants_inner(&self.types, &self.ctx.strings)
    }

    /// Phase 2 of #146 — scan emitted bytecode and report when a
    /// `MakeVariant { tag, field_count }` instruction has no matching
    /// (tag, payload-width) pair in any declared type's variant
    /// table.  Reports as a `tracing::warn!` rather than failing the
    /// compile because variant constructors registered for types
    /// declared in other loaded modules (e.g. `Result.Ok` from
    /// `core.base.result` referenced from a downstream module) live
    /// in those modules' TypeDescriptor arrays, not this one's.
    /// A hard fail would be a false positive for every cross-module
    /// variant emission.
    ///
    /// The whole-program version of this check belongs at the level
    /// where the unified type table is materialized — out of scope
    /// for the per-module finalize.  Keeping the warning here at
    /// least surfaces module-internal layout drift early.
    ///
    /// Returns the number of instructions reported (zero in clean
    /// builds), useful for tests that want a structural assertion.
    pub fn report_make_variant_inconsistencies(&self) -> usize {
        use crate::instruction::Instruction;
        use crate::types::VariantKind;
        // Build the set of valid (tag, field_count) combos.  Stored as
        // a HashSet<(u32, u32)> for O(1) membership check.
        let mut valid: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::new();
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
        // Empty type table = nothing to compare against.
        if valid.is_empty() {
            return 0;
        }
        let mut reported: usize = 0;
        for f in &self.functions {
            for ins in &f.instructions {
                if let Instruction::MakeVariant {
                    dst: _,
                    tag,
                    field_count,
                } = ins
                    && !valid.contains(&(*tag, *field_count)) {
                        let fname = self
                            .ctx
                            .strings
                            .get(f.descriptor.name.0 as usize)
                            .cloned()
                            .unwrap_or_else(|| {
                                format!("<FunctionId({})>", f.descriptor.id.0)
                            });
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
                        reported += 1;
                    }
            }
        }
        reported
    }

    /// Push a `TypeDescriptor` into `self.types`, skipping when an
    /// existing descriptor already claims the same `TypeId`.
    ///
    /// Used at type-registration sites where the well-known TypeId
    /// map (e.g. `Heap`/`Shared` both bound to `TypeId::PTR = 14`)
    /// produces multiple `TypeDescriptor` instances at the same id.
    /// First-wins semantics: keep the first registration, drop the
    /// rest.  Function-table registrations are independent and not
    /// affected by this dedupe.
    ///
    /// Safe because the runtime dispatches by `TypeId`, not by
    /// descriptor identity — two descriptors at the same id are
    /// observationally indistinguishable from one descriptor at
    /// that id (modulo whichever variants/fields the first one
    /// happened to register, which is the existing well-known
    /// alias semantic).
    fn push_type_dedupe(&mut self, ty: crate::types::TypeDescriptor) {
        if self.types.iter().any(|t| t.id == ty.id) {
            return;
        }
        self.types.push(ty);
    }

    /// Allocate a fresh user-defined `TypeId` that doesn't collide
    /// with the reserved well-known ranges.
    ///
    /// Reserved ranges (see `crate::types::TypeId` constants):
    ///   * 0..16        primitives + aliases
    ///   * 256..260     meta system (TokenStream / Token / Kind / Span)
    ///   * 512..1024    semantic collections + dependent-type packaging
    ///                  (LIST, MAP, …, PI, SIGMA, WITNESS)
    ///
    /// Without this guard, a stdlib build whose user-type count
    /// exceeds 240 wraps `next_type_id` into the meta range, then
    /// past 252 wraps into the semantic range — and stdlib types
    /// silently collide with reserved IDs.  #170's global
    /// consistency check surfaced this on `result.vr` where
    /// `OneshotInner` and `Channel` both ended up at TypeId(523).
    ///
    /// The function bumps `next_type_id` past every reserved range
    /// it encounters before returning.  Idempotent in the sense
    /// that calling it `n` times produces `n` distinct IDs.
    fn alloc_user_type_id(&mut self) -> crate::types::TypeId {
        use crate::types::TypeId;
        loop {
            let candidate = TypeId::FIRST_USER + self.next_type_id;
            // Meta-system range: 256..260 (TOKEN_STREAM..SPAN).  If we
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
    /// type table.  Reports the structural-hygiene classes that the
    /// per-module verifier deliberately can't catch:
    ///
    ///   1. **Duplicate `TypeId`** — two `TypeDescriptor`s with the
    ///      same numeric id but different declaration sites.  Caused
    ///      by a name collision where the `type_name_to_id` insert
    ///      guard (`if !contains_key`) silently merges the second
    ///      type's declaration into the first's slot.
    ///
    ///   2. **Same name, different `TypeId`** — two descriptors
    ///      sharing a name with distinct ids.  Indicates the codegen
    ///      ran multiple type-allocation passes and the second pass
    ///      didn't see the first pass's registration.
    ///
    ///   3. **Variant-tag gaps / duplicates within a sum** — already
    ///      checked per-module by `verify_type_layout_invariants`,
    ///      lifted here so a global pass catches the case where two
    ///      modules separately declare overlapping subsets of the
    ///      same logical sum's variants.
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
    /// unified type table.  At the per-module level this is a warn
    /// (cross-module variants live in other modules' descriptors); at
    /// the global level it's a real bug — every `MakeVariant` should
    /// resolve once all modules have been registered.
    pub fn find_orphan_make_variants(&self) -> Vec<OrphanMakeVariant> {
        use crate::instruction::Instruction;
        use crate::types::VariantKind;
        // Build the set of valid (tag, field_count) combos across all
        // declared types.  HashSet for O(1) membership.
        let mut valid: std::collections::HashSet<(u32, u32)> =
            std::collections::HashSet::new();
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
        // Empty type table → can't compare; bail.  The per-module
        // verifier handles the empty case identically.
        if valid.is_empty() {
            return Vec::new();
        }
        let mut orphans = Vec::new();
        for f in &self.functions {
            for ins in &f.instructions {
                if let Instruction::MakeVariant { dst: _, tag, field_count } = ins
                    && !valid.contains(&(*tag, *field_count)) {
                        let fname = self
                            .ctx
                            .strings
                            .get(f.descriptor.name.0 as usize)
                            .cloned()
                            .unwrap_or_else(|| {
                                format!("<FunctionId({})>", f.descriptor.id.0)
                            });
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
    /// `TypeDescriptor`s and the matching string table.  Pulled out
    /// so unit tests can construct synthetic tables without going
    /// through the full codegen lifecycle.
    fn compute_type_table_health(
        types: &[crate::types::TypeDescriptor],
        strings: &[String],
    ) -> TypeTableHealthReport {
        use std::collections::HashMap;
        let resolve_name = |idx: u32| -> String {
            strings.get(idx as usize).cloned().unwrap_or_else(|| format!("<id {}>", idx))
        };

        // Pass 1: bucket by TypeId. >1 in a bucket means duplicate ids.
        let mut by_id: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, ty) in types.iter().enumerate() {
            by_id.entry(ty.id.0).or_default().push(i);
        }
        let mut duplicate_ids: Vec<DuplicateTypeId> = Vec::new();
        for (id, idxs) in &by_id {
            if idxs.len() > 1 {
                let names: Vec<String> = idxs.iter()
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
            by_name.entry(resolve_name(ty.name.0)).or_default().push((ty.id.0, i));
        }
        let mut duplicate_names_with_different_ids: Vec<DuplicateNameDifferentId> = Vec::new();
        for (name, slots) in &by_name {
            if slots.len() > 1 {
                let ids: std::collections::HashSet<u32> =
                    slots.iter().map(|(id, _)| *id).collect();
                if ids.len() > 1 {
                    let mut sorted_ids: Vec<u32> = ids.into_iter().collect();
                    sorted_ids.sort_unstable();
                    duplicate_names_with_different_ids
                        .push(DuplicateNameDifferentId {
                            name: name.clone(),
                            type_ids: sorted_ids,
                        });
                }
            }
        }

        // Pass 3: variant-tag density.  Tags within a sum must be
        // 0..variants.len() with no holes and no duplicates.  The
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
            let mut seen: std::collections::HashSet<u32> =
                std::collections::HashSet::new();
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
            // max_tag == n-1.  Anything else has gaps or out-of-range
            // tags.
            let dense = seen.len() as u32 == n
                && (n == 0 || max_tag == n - 1);
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

    /// Test-only: push a synthetic `TypeDescriptor` into the codegen's
    /// type table.  Used by integration tests for the layout verifier
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
            strings.get(idx as usize).cloned().unwrap_or_else(|| format!("<id {}>", idx))
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
                                type_name, v_name, v.arity, v.fields.len(),
                            )));
                        }
                    }
                    VariantKind::Tuple => {
                        if !v.fields.is_empty() {
                            return Err(CodegenError::internal(format!(
                                "type-layout invariant: variant `{}.{}` is `Tuple` \
                                 (arity={}) but also has {} record-field(s); \
                                 tuple variants store payload count in `arity`, \
                                 not `fields`",
                                type_name, v_name, v.arity, v.fields.len(),
                            )));
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
                                type_name, v_name, v.fields.len(), v.arity,
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
        if !module.attributes.is_empty()
            && !self.cfg_evaluator.should_include(&module.attributes)
        {
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
            if !self.should_compile_item(item) { continue; }
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
            if !self.should_compile_item(item) { continue; }
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
            if !self.should_compile_item(item) { continue; }
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
            if !self.should_compile_item(item) { continue; }
            if let ItemKind::Type(type_decl) = &item.kind {
                let type_name = type_decl.name.name.to_string();
                if !self.type_name_to_id.contains_key(&type_name) {
                    let type_id = self.alloc_user_type_id();
                    self.type_name_to_id.insert(type_name, type_id);
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
    pub fn resolve_mounts(
        &mut self,
        module: &Module,
        source_path: &str,
        core_root: &str,
    ) {
        let mut resolved_files: std::collections::HashSet<String> = std::collections::HashSet::new();
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
            // Honour the per-item @cfg gate.  A `mount` whose attribute
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
                    let file_candidates = Self::module_path_to_file_candidates(&module_path, source_path, core_root);
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
            }
        }

        // Parse and register each imported module
        for file_path in to_parse {
            #[cfg(feature = "codegen")]
            if let Ok(source) = std::fs::read_to_string(&file_path) {
                let mut parser = verum_parser::Parser::new(&source);
                if let Ok(imported_module) = parser.parse_module() {
                    // Recursively resolve mounts from this imported file first
                    self.resolve_mounts_recursive(
                        &imported_module, &file_path, core_root, resolved_files, depth + 1,
                    );
                    // Then register its declarations
                    self.collect_protocol_definitions(&imported_module);
                    let _ = self.collect_all_declarations(&imported_module);
                }
            }
        }
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
            MountTreeKind::Nested { prefix: nested_prefix, trees } => {
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
                    "." => { start_idx = i + 1; }
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

            let remaining: Vec<&str> = module_path[start_idx..].iter().map(|s| s.as_str()).collect();
            if !remaining.is_empty() {
                // Try as file: base/remaining.vr
                let file_path = base.join(remaining.join("/")).with_extension("vr");
                candidates.push(file_path.to_string_lossy().to_string());

                // Try parent as file (last segment might be item name)
                if remaining.len() > 1 {
                    let parent_path = base.join(remaining[..remaining.len()-1].join("/")).with_extension("vr");
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
            (std::path::Path::new(core_root).to_path_buf(), &module_path[1..])
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
                let parent = root.join(path_segments[..path_segments.len()-1].join("/")).with_extension("vr");
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
    ///   * User-phase (`prefer_existing_functions = false`): any existing
    ///     variants for the redeclared type are purged via
    ///     `clear_variants_for_type` before the new set is registered —
    ///     this wipes these sentinels.
    ///
    ///   * Stdlib-phase (`prefer_existing_functions = true`): if a prior
    ///     registration has already populated variants for this nominal
    ///     type, `has_variants_for_type` short-circuits the re-registration
    ///     so stdlib variants do not leak into a user type of the same
    ///     name.
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
    fn register_builtin_variants(&mut self) {
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
        // The tags here match the declaration order in `core/base/maybe.vr`,
        // `core/base/result.vr`, `core/base/ordering.vr`. When a user program
        // *does* define its own `type Maybe is None | Some(T)` (or similar),
        // register_type_constructors overwrites these entries with the
        // user-level tags (which also happen to match), so both paths agree.
        use crate::codegen::context::FunctionInfo;
        use crate::module::FunctionId;
        let builtins: &[(&str, &str, u32, usize, Vec<String>)] = &[
            // (type_name, variant_name, tag, arity, param_names)
            ("Maybe", "None",    0, 0, vec![]),
            ("Maybe", "Some",    1, 1, vec!["_0".into()]),
            ("Result", "Ok",     0, 1, vec!["_0".into()]),
            ("Result", "Err",    1, 1, vec!["_0".into()]),
            ("Ordering", "Less",    0, 0, vec![]),
            ("Ordering", "Equal",   1, 0, vec![]),
            ("Ordering", "Greater", 2, 0, vec![]),
        ];
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
            };
            // Always register qualified name.
            self.ctx.register_function(qualified, info.clone());
            // Also register simple name unless it would collide with a prior
            // registration (follows the same "simple-on-no-collision" rule as
            // user variant registration).
            if self.ctx.lookup_function(variant_name).is_none() {
                self.ctx.register_function((*variant_name).to_string(), info);
            }
        }
    }

    /// Registers runtime I/O and networking functions as builtins.
    /// These emit Call instructions that the LLVM lowering intercepts.
    fn register_runtime_io_functions(&mut self) {
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
            self.functions.push(VbcFunction::new(stub_descriptor, vec![]));

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
                if let verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } = &impl_decl.kind {
                    let derived_name = protocol.segments.last()
                        .and_then(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                            _ => None,
                        });
                    let for_type_name = Self::for_type_generic_param_name(for_type);
                    if let (Some(derived_name), Some(param_name)) = (derived_name, for_type_name) {
                        for g in impl_decl.generics.iter() {
                            if let verum_ast::ty::GenericParamKind::Type { name, bounds, .. } = &g.kind
                                && name.name.as_str() == param_name
                            {
                                for b in bounds.iter() {
                                    if let Some(base_name) = Self::type_bound_protocol_name(b) {
                                        let explicit_methods: std::collections::HashSet<String> =
                                            impl_decl.items.iter().filter_map(|item| {
                                                if let verum_ast::decl::ImplItemKind::Function(f) = &item.kind {
                                                    Some(f.name.name.to_string())
                                                } else {
                                                    None
                                                }
                                            }).collect();
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
                let is_protocol_impl = matches!(&impl_decl.kind, verum_ast::decl::ImplKind::Protocol { .. });
                let prev_prefer_existing = self.ctx.prefer_existing_functions;
                if is_protocol_impl {
                    self.ctx.prefer_existing_functions = true;
                }

                // Check if this is a Drop implementation
                let is_drop_impl = if let verum_ast::decl::ImplKind::Protocol { protocol, .. } = &impl_decl.kind {
                    protocol.segments.first()
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
                                    if let Some(func_info) = self.ctx.lookup_function(&qualified_name) {
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
                                self.register_constant_with_value(&qualified, Some(value), Some(ty))?;
                            }
                        }
                        _ => {}
                    }
                }

                // Generate default protocol methods for protocol impls
                if let verum_ast::decl::ImplKind::Protocol { protocol, .. } = &impl_decl.kind
                    && let Some(ref ty_name) = type_name {
                        // Get the last segment of the protocol path (e.g., "Hasher" from "core.protocols.Hasher")
                        let protocol_name = protocol.segments.last()
                            .and_then(|s| match s {
                                verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                                _ => None,
                            });
                        if let Some(pn) = protocol_name {
                            // Get methods explicitly implemented
                            let implemented_methods: std::collections::HashSet<String> = impl_decl.items.iter()
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
                            if let Some(&proto_type_id) = self.type_name_to_id.get(&pn) {
                                // Get protocol method names in vtable order (from protocol's variants)
                                let method_names: Vec<String> = self.types.iter()
                                    .find(|td| td.id == proto_type_id)
                                    .map(|td| td.variants.iter()
                                        .map(|v| {
                                            let idx = v.name.0 as usize;
                                            if idx < self.ctx.strings.len() {
                                                self.ctx.strings[idx].clone()
                                            } else {
                                                String::new()
                                            }
                                        })
                                        .collect())
                                    .unwrap_or_default();

                                // Look up concrete FunctionIds for each method
                                let method_fn_ids: Vec<u32> = method_names.iter()
                                    .map(|method_name| {
                                        let qualified = format!("{}.{}", ty_name, method_name);
                                        self.ctx.lookup_function(&qualified)
                                            .map(|fi| fi.id.0)
                                            .unwrap_or(u32::MAX) // sentinel for missing method
                                    })
                                    .collect();

                                // Push protocol impl onto the concrete type's descriptor
                                if let Some(&concrete_type_id) = self.type_name_to_id.get(ty_name.as_str()) {
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
                    let field_names: Vec<String> = fields.iter()
                        .map(|f| f.name.name.to_string())
                        .collect();
                    let field_types: Vec<String> = fields.iter()
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
                self.register_constant_with_value(&const_decl.name.name, Some(&const_decl.value), Some(&const_decl.ty))?;
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

                    // Queue the init expression for compilation as a TLS initializer
                    self.pending_tls_inits.push((name, static_decl.value.clone(), slot));
                } else {
                    self.register_constant_with_value(&static_decl.name.name, Some(&static_decl.value), Some(&static_decl.ty))?;
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
            // Module declarations don't produce bytecode
            ItemKind::Module(_) => {
                // No bytecode for module structure
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
                let members: Vec<String> = group_decl.contexts.iter()
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
                        let entries: Vec<(String, verum_ast::expr::Expr)> = provides.iter()
                            .map(|(name, expr)| (name.name.to_string(), expr.clone()))
                            .collect();
                        self.context_layers.insert(layer_name, ContextLayer::Inline(entries));
                    }
                    verum_ast::decl::LayerKind::Composite { layers } => {
                        let names: Vec<String> = layers.iter()
                            .map(|id| id.name.to_string())
                            .collect();
                        self.context_layers.insert(layer_name, ContextLayer::Composite(names));
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
                    if !name.is_empty() && name != "()" { Some(name) } else { None }
                } else { None }
            })
            .collect();

        let contexts: Vec<String> = func
            .contexts
            .iter()
            .filter(|c| {
                // Skip negative contexts and false conditional contexts
                if c.is_negative { return false; }
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
        let return_type = func.return_type.as_ref().map(|ret_ty| self.ast_type_to_type_ref(ret_ty));

        // Extract intrinsic name from @intrinsic("name") attribute if present.
        // This enables industrial-grade intrinsic resolution at declaration time.
        // If the function doesn't have @intrinsic but was previously registered
        // as an intrinsic (via register_stdlib_intrinsics), preserve that name — but
        // ONLY if this function is a forward declaration (no body). A user-defined
        // function with a body should override any previously registered intrinsic
        // stub of the same name, so it calls the user's implementation instead.
        let intrinsic_name = self.extract_intrinsic_name(func)
            .or_else(|| {
                if matches!(func.body, verum_common::Maybe::None) {
                    self.ctx.lookup_function(&name)
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
            yield_type: None,  // Will be inferred from yield expressions
            intrinsic_name,
            variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
        };

        // #201 diagnostic — env-var-gated trace of every register_function
        // call. Set `VERUM_TRACE_REGISTER=1` (or `=try_alloc` for a
        // substring-filtered trace) to surface registration attempts on the
        // run-interpreter path without flooding normal runs.
        //
        // The substring filter is helpful for the original #201 reproduction
        // ("ZERO entries match try_alloc") — running with
        //   VERUM_TRACE_REGISTER=try_alloc verum run --interp file.vr
        // shows whether `try_alloc` reaches register_function at all, and
        // under what `effective_module`.
        if let Ok(filter) = std::env::var("VERUM_TRACE_REGISTER") {
            let pass = filter == "1" || filter.is_empty()
                || base_name.contains(filter.as_str())
                || name.contains(filter.as_str());
            if pass {
                let eff_mod = self.ctx.current_source_module
                    .as_deref()
                    .unwrap_or(&self.config.module_name);
                eprintln!(
                    "[register-fn] module={} base={} mangled={} arity={} prefer_existing={}",
                    eff_mod, base_name, name, info.param_count,
                    self.ctx.prefer_existing_functions
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
        //   - mangled nested-function names (already module-local)
        //   - anonymous / empty module names
        //   - names that look like already-qualified type-method registrations
        //     ("Foo.bar") — those get their own qualified registration path
        //     via `register_impl_function`.
        // Prefer the *source module* (from the `module X.Y.Z;` declaration at
        // the top of the current .vr file) over `config.module_name`. The
        // config's module_name is fixed per-codegen-session (`"main"` for a
        // single-file user run) but a single session processes many imported
        // stdlib modules, each with its own path. `current_source_module` is
        // scoped to the file currently being collected/compiled.
        let effective_module = self.ctx.current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        if self.nested_function_scope.is_empty()
            && !effective_module.is_empty()
            && effective_module != "main"
            && !base_name.contains('.')
            && !base_name.contains("::")
        {
            let dot_qualified = format!("{}.{}", effective_module, base_name);
            let colon_qualified = effective_module
                .replace('.', "::")
                + "::"
                + &base_name;
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
    fn register_pattern_as_function(&mut self, pat: &verum_ast::decl::PatternDecl) -> CodegenResult<()> {
        let name = pat.name.name.to_string();
        let id = FunctionId(self.next_func_id);
        self.next_func_id = self.next_func_id.saturating_add(1);

        // Combine type_params (parameterized patterns) + params (match params)
        let mut param_names: Vec<String> = pat.type_params.iter()
            .enumerate()
            .map(|(i, p)| self.extract_param_name(p).unwrap_or_else(|| format!("_tp{}", i)))
            .collect();
        for (i, p) in pat.params.iter().enumerate() {
            param_names.push(self.extract_param_name(p).unwrap_or_else(|| format!("_arg{}", i)));
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
            return_type_name: if is_partial { Some("Maybe".to_string()) } else { None },
            return_type_inner: None,
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
        use verum_ast::ty::{TypeKind, PathSegment};
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
    fn compile_pattern_as_function(&mut self, pat: &verum_ast::decl::PatternDecl) -> CodegenResult<()> {
        let name = pat.name.name.to_string();

        let func_info = self.ctx.lookup_function(&name)
            .ok_or_else(|| CodegenError::internal(format!("pattern not registered: {}", name)))?
            .clone();

        // Build params with mutability (patterns are immutable)
        let mut params: Vec<(String, bool)> = pat.type_params.iter()
            .enumerate()
            .map(|(i, p)| (self.extract_param_name(p).unwrap_or_else(|| format!("_tp{}", i)), false))
            .collect();
        for (i, p) in pat.params.iter().enumerate() {
            params.push((self.extract_param_name(p).unwrap_or_else(|| format!("_arg{}", i)), false));
        }

        self.ctx.begin_function(&name, &params, func_info.return_type.clone());

        // Register parameter types for correct operation selection
        for (param_name, param) in params.iter().zip(pat.type_params.iter().chain(pat.params.iter())) {
            if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                let var_type = self.type_kind_to_var_type(&ty.kind);
                self.ctx.register_variable_type(&param_name.0, var_type);
            }
        }

        // Compile the body expression
        let result = self.compile_expr(&pat.body)
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

        let name_id = StringId(self.intern_string(&name));
        let mut descriptor = FunctionDescriptor::new(name_id);
        descriptor.id = func_info.id;
        descriptor.register_count = register_count;
        descriptor.locals_count = params.len() as u16;
        if let Some(ref rt) = ret_type {
            descriptor.return_type = rt.clone();
        }

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
        self.functions.push(vbc_func);

        Ok(())
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
            intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
        };

        self.ctx.register_function(name, info);
        Ok(())
    }

    /// Registers import aliases so that aliased function names can be resolved.
    ///
    /// This processes imports like `import sys.linux.syscall.{write as sys_write}` and
    /// registers `sys_write` as pointing to `sys.linux.syscall.write`.
    fn register_import_aliases(&mut self, import: &MountDecl) -> CodegenResult<()> {
        self.process_import_tree(&import.tree, &[])
    }

    /// Recursively processes an import tree to register function aliases.
    fn process_import_tree(&mut self, tree: &MountTree, prefix: &[String]) -> CodegenResult<()> {
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

                // The alias is either explicit or defaults to the function name
                let alias_name = match &tree.alias {
                    verum_common::Maybe::Some(alias) => alias.name.to_string(),
                    verum_common::Maybe::None => func_name.clone(),
                };

                // Try to look up the function in the registry with various qualified names
                let qualified_verum = full_path.join(".");
                let qualified_rust = full_path.join("::");

                // Helper to check if we should register the alias
                // Don't overwrite existing registrations with MORE params (e.g., FFI functions)
                // This prevents safe wrappers from overwriting raw FFI functions
                let should_register = |alias: &str, new_info: &FunctionInfo| -> bool {
                    match self.ctx.lookup_function(alias) {
                        Some(existing) => {
                            // Only overwrite if new registration has same or more params
                            // This preserves FFI functions (more params) over safe wrappers (fewer params)
                            new_info.param_count >= existing.param_count
                        }
                        None => true, // No existing registration, always register
                    }
                };

                // First try Verum-style qualified name
                if let Some(func_info) = self.ctx.lookup_function(&qualified_verum).cloned() {
                    if should_register(&alias_name, &func_info) {
                        self.ctx.register_function(alias_name.clone(), func_info);
                    }
                    return Ok(());
                }

                // Try Rust-style qualified name
                if let Some(func_info) = self.ctx.lookup_function(&qualified_rust).cloned() {
                    if should_register(&alias_name, &func_info) {
                        self.ctx.register_function(alias_name.clone(), func_info);
                    }
                    return Ok(());
                }

                // Try module.name without the file component (e.g., sys.ORDERING_ACQUIRE)
                // This handles the case where imports reference files (sys.intrinsics.X)
                // but constants are registered at module level (sys.X)
                if full_path.len() >= 3 {
                    let module_name = &full_path[0];
                    let simplified_qualified = format!("{}.{}", module_name, func_name);
                    if let Some(func_info) = self.ctx.lookup_function(&simplified_qualified).cloned() {
                        if should_register(&alias_name, &func_info) {
                            self.ctx.register_function(alias_name.clone(), func_info);
                        }
                        return Ok(());
                    }
                }

                // Try with "core." prefix (modules are registered as core.sys.* but imported as sys.*)
                if full_path.first().map(|s| s.as_str()) == Some("sys")
                    || full_path.first().map(|s| s.as_str()) == Some(".")
                {
                    // Try core.sys.linux.futex_wait, core.sys.darwin.futex_wait, etc.
                    let mut core_path = vec!["core".to_string()];
                    for p in &full_path {
                        if p != "." { core_path.push(p.clone()); }
                    }
                    let core_qualified = core_path.join(".");
                    if let Some(func_info) = self.ctx.lookup_function(&core_qualified).cloned() {
                        if should_register(&alias_name, &func_info) {
                            self.ctx.register_function(alias_name.clone(), func_info);
                        }
                        return Ok(());
                    }
                    // Also try without the file component: core.sys.linux.futex_wait → core.sys.futex_wait
                    if core_path.len() >= 3 {
                        let simplified = format!("core.{}.{}", core_path[1], func_name);
                        if let Some(func_info) = self.ctx.lookup_function(&simplified).cloned() {
                            if should_register(&alias_name, &func_info) {
                                self.ctx.register_function(alias_name.clone(), func_info);
                            }
                            return Ok(());
                        }
                    }
                }

                // Try just the function name (it might be already registered without qualification)
                if let Some(func_info) = self.ctx.lookup_function(&func_name).cloned() {
                    if should_register(&alias_name, &func_info) {
                        self.ctx.register_function(alias_name, func_info);
                    }
                    return Ok(());
                }

                // Check if this is a TYPE name import (e.g., `mount sys.io_engine.{IoError}`).
                // Type names aren't functions themselves, but their variant constructors
                // are registered as `TypeName.Variant`. Import all qualified constructors.
                // Iterate sorted for deterministic registration order (HashMap iter
                // would otherwise leak per-process random hasher seed into bytecode).
                let type_prefix = format!("{}.", func_name);
                let mut sorted_keys: Vec<&String> = self.ctx.functions.keys()
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
                        || core_prefix_dot.as_ref().is_some_and(|cp| name.starts_with(cp));
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
            MountTreeKind::Nested { prefix: nested_prefix, trees } => {
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

                // Process each nested tree with the accumulated prefix
                for sub_tree in trees.iter() {
                    self.process_import_tree(sub_tree, &new_prefix)?;
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

            // Helper to check if we should register the alias
            // Don't overwrite existing registrations with FEWER params (e.g., FFI functions)
            // This prevents safe wrappers (fewer params) from overwriting raw FFI functions (more params)
            let should_register = |ctx: &CodegenContext, alias: &str, new_info: &FunctionInfo| -> bool {
                match ctx.lookup_function(alias) {
                    Some(existing) => {
                        // Only overwrite if new registration has same or more params
                        new_info.param_count >= existing.param_count
                    }
                    None => true, // No existing registration, always register
                }
            };

            // Try to look up the function in the registry with various qualified names
            let qualified_verum = full_path.join(".");
            let qualified_rust = full_path.join("::");

            // First try Verum-style qualified name (e.g., sys.intrinsics.ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&qualified_verum).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    self.ctx.register_function(alias_name, func_info);
                }
                continue;
            }

            // Try Rust-style qualified name (e.g., sys::intrinsics::ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&qualified_rust).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    self.ctx.register_function(alias_name, func_info);
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
                        self.ctx.register_function(alias_name, func_info);
                    }
                    continue;
                }
            }

            // Try just the function name (e.g., ORDERING_ACQUIRE)
            if let Some(func_info) = self.ctx.lookup_function(&func_name).cloned() {
                if should_register(&self.ctx, &alias_name, &func_info) {
                    self.ctx.register_function(alias_name, func_info);
                }
                continue;
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
        use verum_ast::ty::{TypeKind, PathSegment};

        match &ty.kind {
            TypeKind::Path(path) => {
                if let Some(segment) = path.segments.first()
                    && let PathSegment::Name(ident) = segment {
                        return Some(ident.name.to_string());
                    }
                None
            }
            TypeKind::Generic { base, .. } => {
                self.extract_impl_type_name_from_type(base)
            }
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
            TypeKind::CheckedReference { inner, .. } => self.extract_impl_type_name_from_type(inner),
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
    fn register_impl_function(&mut self, func: &FunctionDecl, type_name: &str) -> CodegenResult<()> {
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
                    if !name.is_empty() && name != "()" { Some(name) } else { None }
                } else { None }
            })
            .collect();

        let contexts: Vec<String> = func
            .contexts
            .iter()
            .filter(|c| {
                // Skip negative contexts and false conditional contexts
                if c.is_negative { return false; }
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
        let intrinsic_name = self.extract_intrinsic_name(func)
            .or_else(|| {
                self.ctx.lookup_function(&qualified_name)
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

        // Extract return type name for method dispatch tracking
        let return_type_name = if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
            self.extract_type_name(ret_ty)
        } else {
            None
        };

        // Convert return type for method dispatch and list/string register tracking
        let return_type = func.return_type.as_ref().map(|ret_ty| self.ast_type_to_type_ref(ret_ty));

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
            variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref,
            return_type_name,
            return_type_inner: None,
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
    fn collect_nested_declarations(&mut self, body: &FunctionBody, parent_name: &str) -> CodegenResult<()> {
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
                intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                return_type_name,
                return_type_inner: None,
            };

            self.ctx.register_function(name.clone(), info);

            // Also create FfiSymbol entry with error protocol from the AST
            let signature = self.create_ffi_signature_from_boundary(ffi_func);
            let (error_protocol, error_sentinel) = Self::map_ast_error_protocol(&ffi_func.error_protocol);
            let memory_effects = Self::map_ast_memory_effects(&ffi_func.memory_effects);
            let ownership = Self::map_ast_ownership(&ffi_func.ownership);
            let convention = Self::map_ast_calling_convention(&ffi_func.signature.calling_convention);
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
                    requires: ffi_func.requires.iter()
                        .map(|expr| format!("{:?}", expr))
                        .collect(),
                    ensures: ffi_func.ensures.iter()
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
            AstCallingConvention::Naked => FfiCallingConvention::C,    // No direct VBC equivalent
            AstCallingConvention::System => {
                // System = stdcall on Windows, C elsewhere
                #[cfg(target_os = "windows")]
                { FfiCallingConvention::Stdcall }
                #[cfg(not(target_os = "windows"))]
                { FfiCallingConvention::C }
            }
        }
    }

    /// Derive calling convention from a FunctionDecl's `extern_abi` field.
    ///
    /// `extern_abi` is a freeform string like `"C"`, `"stdcall"`, `"system"`.
    /// Absent means C ABI (the default for extern blocks).
    fn extern_abi_to_convention(abi: &verum_common::Maybe<verum_common::Text>) -> FfiCallingConvention {
        match abi {
            verum_common::Maybe::Some(s) => match s.as_str() {
                "C" | "c" | "cdecl" => FfiCallingConvention::C,
                "stdcall" | "Stdcall" | "StdCall" => FfiCallingConvention::Stdcall,
                "fastcall" | "FastCall" => FfiCallingConvention::Fastcall,
                "sysv64" | "SysV64" => FfiCallingConvention::SysV64,
                "system" | "System" => {
                    #[cfg(target_os = "windows")]
                    { FfiCallingConvention::Stdcall }
                    #[cfg(not(target_os = "windows"))]
                    { FfiCallingConvention::C }
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
            .map(|(_name, ty)| {
                self.verum_type_to_ctype(&verum_common::Maybe::Some(ty.clone()))
            })
            .collect();
        let return_type = self.verum_type_to_ctype(
            &verum_common::Maybe::Some(func.signature.return_type.clone()),
        );
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
    ///     fn getpid() -> Int;
    ///     fn malloc(size: Int) -> &unsafe Byte;
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
                    && let Some(first_arg) = args.first() {
                        // The first argument should be a string literal
                        if let ExprKind::Literal(lit) = &first_arg.kind
                            && let LiteralKind::Text(
                                StringLit::Regular(s)
                                | StringLit::MultiLine(s)
                            ) = &lit.kind {
                                return Some(s.to_string());
                            }
                    }
            }
        }
        None
    }

    /// Checks if a function has an @ffi attribute.
    fn has_ffi_attribute(&self, attributes: &verum_common::List<verum_ast::attr::Attribute>) -> bool {
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
                    && let Some(first_arg) = args.first() {
                        // The argument is a Path expression for an identifier like "C"
                        if let ExprKind::Path(path) = &first_arg.kind
                            && let Some(PathSegment::Name(ident)) = path.segments.first() {
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
    fn has_bitfield_attr(&self, attributes: &verum_common::List<verum_ast::attr::Attribute>) -> bool {
        for attr in attributes.iter() {
            if attr.name.as_str() == "bitfield" {
                return true;
            }
        }
        false
    }

    /// Extracts the byte order from @endian attribute, defaulting to little.
    fn get_byte_order(&self, attributes: &verum_common::List<verum_ast::attr::Attribute>) -> ByteOrder {
        use verum_ast::expr::ExprKind;

        for attr in attributes.iter() {
            if attr.name.as_str() == "endian"
                && let verum_common::Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                        && let ExprKind::Path(path) = &first_arg.kind
                            && let Some(PathSegment::Name(ident)) = path.segments.first() {
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
                let bit_offset = bit_spec.offset
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
                    intrinsic_name: Some(format!("bitfield_get:{}:{}:{}", type_name, field_name, bit_offset)), variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: None, // Bitfield getters return primitive types
                    return_type_inner: None,
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
                    intrinsic_name: Some(format!("bitfield_set:{}:{}:{}", type_name, field_name, bit_offset)), variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: None, // Setters return unit
                    return_type_inner: None,
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
            if let TypeKind::Function { params, return_type, .. } = &param_type.kind {
                // Create the callback signature
                let callback_return_type = self.verum_type_to_ctype(&verum_common::Maybe::Some((**return_type).clone()));
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
                    param_layout_indices: smallvec::SmallVec::from_elem(None, callback_param_types.len()),
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
                self.ffi_callback_signatures.insert(
                    (symbol_id, param_idx as u8),
                    callback_symbol_id,
                );
            }
        }
    }

    /// Gets the callback signature symbol ID for an FFI function parameter.
    ///
    /// Returns the synthetic FfiSymbol ID that contains the callback signature
    /// for the given FFI function and parameter index. Returns None if the
    /// parameter is not a function pointer type.
    pub fn get_callback_signature_id(&self, ffi_symbol_id: FfiSymbolId, param_idx: u8) -> Option<FfiSymbolId> {
        self.ffi_callback_signatures.get(&(ffi_symbol_id, param_idx)).copied()
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
                            // Signed integers: both Verum semantic names and compat names
                            "Int" | "Int64" | "i64" => CType::I64,
                            "Int32" | "i32" => CType::I32,
                            "Int16" | "i16" => CType::I16,
                            "Int8" | "i8" => CType::I8,
                            // Unsigned integers: both Verum semantic names and compat names
                            "UInt64" | "u64" => CType::U64,
                            "UInt32" | "u32" => CType::U32,
                            "UInt16" | "u16" => CType::U16,
                            "UInt8" | "u8" | "Byte" => CType::U8,
                            // Pointer-sized integers
                            "ISize" | "isize" => CType::Ssize,
                            "USize" | "usize" => CType::Size,
                            // Floating point: both Verum semantic names and compat names
                            "Float" | "Float64" | "f64" => CType::F64,
                            "Float32" | "f32" => CType::F32,
                            // Boolean
                            "Bool" | "bool" => CType::Bool,
                            // Unit type
                            "()" => CType::Void,
                            _ => CType::Ptr, // Unknown types become pointers
                        }
                    }
                    // Primitive type variants
                    TypeKind::Int => CType::I64,
                    TypeKind::Float => CType::F64,
                    TypeKind::Bool => CType::Bool,
                    TypeKind::Char => CType::U32, // Unicode codepoint
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
    fn get_struct_layout_index(&self, ty: &verum_common::Maybe<verum_ast::ty::Type>) -> Option<u16> {
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
                && !Self::evaluate_context_condition(cond) {
                    continue;
                }

            // Get context name
            let ctx_name = ctx.path.segments.last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_name.is_empty() { continue; }

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
                        self.ctx.emit(Instruction::Mov { dst: first, src: val });
                        if val != first { self.ctx.free_temp(val); }
                    }
                    for arg in transform.args.iter().skip(1) {
                        let arg_reg = self.ctx.alloc_temp();
                        if let Ok(Some(val)) = self.compile_expr(arg) {
                            self.ctx.emit(Instruction::Mov { dst: arg_reg, src: val });
                            if val != arg_reg { self.ctx.free_temp(val); }
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
            ExprKind::Field { expr: object, field } => {
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
                    } else { false }
                } else { false }
            }
            ExprKind::Literal(lit) => {
                matches!(lit.kind, verum_ast::literal::LiteralKind::Bool(true))
            }
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    matches!(ident.name.as_str(), "true" | "debug")
                } else { false }
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
    ///    and remove the simple name (code must use qualified names)
    ///
    /// This allows convenient unqualified usage like `Some(x)` when there's
    /// no ambiguity, while still supporting qualified names like `Maybe.Some(x)`
    /// when disambiguation is needed.
    fn register_type_constructors(
        &mut self,
        type_decl: &verum_ast::decl::TypeDecl,
    ) -> CodegenResult<()> {
        let type_name = type_decl.name.name.to_string();

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
                tracing::debug!("[variant] register_type_constructors entering for {}", type_name);

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
                            let names: Vec<String> = fields
                                .iter()
                                .map(|f| f.name.name.to_string())
                                .collect();
                            // Extract type names from each field
                            let type_names: Vec<String> = fields
                                .iter()
                                .map(|f| self.type_to_simple_name(&f.ty))
                                .collect();
                            // Register field layout for the variant name so
                            // match destructuring with `..` resolves correct field indices
                            self.register_record_fields(&variant_name, names.clone(), type_names.clone());
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
                        variant_payload_types: if payload_types.is_empty() { None } else { Some(payload_types) },
                        is_partial_pattern: false, takes_self_mut_ref: false,
                        // Variant constructors return the parent type
                        return_type_name: Some(type_name.clone()),
                        return_type_inner: None,
                    };

                    // 1. Always register with qualified name (TypeName::VariantName)
                    tracing::debug!(
                        "[variant] registering qualified {}.{} tag={} params={}",
                        type_name, variant_name, tag, param_count
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
                let (type_align, is_packed, _is_repr_c) = Self::extract_type_layout_hints(&type_decl.attributes);
                let field_size = if is_packed { 1u32 } else { 8u32 }; // packed: minimum size per field
                type_desc.size = (fields.len() as u32) * field_size;
                type_desc.alignment = type_align;

                // Build generic type param name → index mapping for field type resolution.
                // E.g., for `type Pair<A, B>`, maps {"A"→0, "B"→1}.
                let mut generic_param_map: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
                for (idx, generic) in type_decl.generics.iter().enumerate() {
                    if let verum_ast::ty::GenericParamKind::Type { name, .. } = &generic.kind {
                        generic_param_map.insert(name.name.to_string(), idx as u16);
                    }
                }

                // Also populate type_params on the TypeDescriptor (was only done for protocols)
                for generic in &type_decl.generics {
                    if let verum_ast::ty::GenericParamKind::Type { name, bounds, default } = &generic.kind {
                        let param_name_id = StringId(self.ctx.intern_string_raw(name.name.as_str()));
                        let bound_ids: smallvec::SmallVec<[crate::types::ProtocolId; 2]> = bounds.iter()
                            .filter_map(|bound| {
                                if let verum_ast::ty::TypeBoundKind::Protocol(path) = &bound.kind
                                    && let Some(seg) = path.segments.last()
                                        && let verum_ast::ty::PathSegment::Name(ident) = seg {
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

                let param_names: Vec<String> = fields
                    .iter()
                    .map(|f| f.name.name.to_string())
                    .collect();

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
                    intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
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

                // Encode method signatures as variants (VBC convention for protocol methods)
                for protocol_item in &protocol_body.items {
                    if let verum_ast::decl::ProtocolItemKind::Function { decl, .. } = &protocol_item.kind {
                        let method_name = decl.name.name.to_string();
                        let method_name_id = StringId(self.ctx.intern_string_raw(&method_name));

                        // Build function TypeRef for the method
                        let param_refs: Vec<crate::types::TypeRef> = decl.params.iter().filter_map(|p| {
                            if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                                Some(self.ast_type_to_type_ref(ty))
                            } else {
                                None // Skip self params
                            }
                        }).collect();

                        let ret_ref = match &decl.return_type {
                            verum_common::Maybe::Some(ty) => self.ast_type_to_type_ref(ty),
                            verum_common::Maybe::None => TypeRef::Concrete(crate::types::TypeId::UNIT),
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
                    if let verum_ast::ty::GenericParamKind::Type { name, bounds, default } = &generic.kind {
                        let param_name_id = StringId(self.ctx.intern_string_raw(name.name.as_str()));
                        let bound_ids: smallvec::SmallVec<[crate::types::ProtocolId; 2]> = bounds.iter()
                            .filter_map(|bound| {
                                if let verum_ast::ty::TypeBoundKind::Protocol(path) = &bound.kind
                                    && let Some(seg) = path.segments.last()
                                        && let verum_ast::ty::PathSegment::Name(ident) = seg {
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
                    self.type_aliases.insert(type_name, base_name);
                }
            }

            // Newtype and tuple types: register the type name as constructor
            TypeDeclBody::Newtype(_inner_type) => {
                // Track newtype names for GetF optimization (field .0 = identity)
                self.ctx.newtype_names.insert(type_name.clone());
                let inner_name = self.type_to_simple_name(_inner_type);
                self.ctx.newtype_inner_type.insert(type_name.clone(), inner_name);

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
                    intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                };

                self.ctx.register_function(type_name.clone(), info);
            }

            TypeDeclBody::Tuple(types) => {
                // Single-element tuple types like `type FileDesc is (Int)` are newtypes.
                // The value IS the single wrapped field — no heap allocation.
                if types.len() == 1 {
                    self.ctx.newtype_names.insert(type_name.clone());
                    let inner_name = self.type_to_simple_name(&types[0]);
                    self.ctx.newtype_inner_type.insert(type_name.clone(), inner_name);
                }

                let id = FunctionId(u32::MAX / 2);

                let param_names: Vec<String> = (0..types.len())
                    .map(|i| format!("_{}", i))
                    .collect();

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
                    intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
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
                    intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
                };

                self.ctx.register_function(type_name, info);
            }

            // SigmaTuple types: similar to tuple types
            TypeDeclBody::SigmaTuple(types) => {
                let id = FunctionId(u32::MAX / 2);

                let param_names: Vec<String> = (0..types.len())
                    .map(|i| format!("_{}", i))
                    .collect();

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
                    intrinsic_name: None, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
                    return_type_name: Some(type_name.clone()),
                    return_type_inner: None,
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
        if needs_compilation
            && let Some(expr) = value_expr {
                self.pending_constants.push((name.to_string(), expr.clone()));
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
            intrinsic_name, variant_tag: None, parent_type_name: None, variant_payload_types: None, is_partial_pattern: false, takes_self_mut_ref: false,
            return_type_name,
            return_type_inner: None,
        };

        // Register with simple name for local access
        self.ctx.register_function(name.to_string(), info.clone());

        // Also register with qualified name for cross-module imports
        // e.g., "sys.intrinsics.ORDERING_ACQUIRE" for import sys.intrinsics.{ORDERING_ACQUIRE}
        let module_name = &self.config.module_name;
        if !module_name.is_empty() && module_name != "main" {
            let qualified_name = format!("{}.{}", module_name, name);
            self.ctx.register_function(qualified_name, info);
        }

        // Register the constant's type for correct instruction selection.
        // This is critical for generating float vs integer operations on constants.
        // e.g., `const PI: Float = 3.14;` then `-PI` should use NegF, not NegI.
        // Uses register_constant_type which persists across function compilations.
        if let Some(ty) = const_type {
            let var_type = self.type_kind_to_var_type(&ty.kind);
            self.ctx.register_constant_type(name, var_type);
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

        for (name, expr) in constants {
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

            // Create function descriptor
            let name_id = StringId(self.intern_string(&name));
            let mut descriptor = FunctionDescriptor::new(name_id);
            descriptor.id = func_info.id;
            descriptor.register_count = register_count;
            descriptor.locals_count = 0;

            // Create VbcFunction and add it
            let vbc_func = VbcFunction::new(descriptor, instructions);
            self.functions.push(vbc_func);
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
                self.ctx.emit(Instruction::LoadI { dst: slot_reg, value: slot as i64 });
                self.ctx.emit(Instruction::TlsSet { slot: slot_reg, val: result_reg });
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
            self.functions.push(vbc_func);

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
            ExprKind::Literal(lit) => {
                match &lit.kind {
                    LiteralKind::Int(int_lit) => Some(int_lit.value as i64),
                    LiteralKind::Bool(b) => Some(if *b { 1 } else { 0 }),
                    _ => None,
                }
            }
            ExprKind::Paren(inner) => Self::extract_const_literal_value(inner),
            ExprKind::Unary { op: verum_ast::UnOp::Neg, expr: operand } => {
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
    fn extract_param_name(&self, param: &verum_ast::FunctionParam) -> Option<String> {
        use verum_ast::FunctionParamKind;
        match &param.kind {
            FunctionParamKind::Regular { pattern, .. } => {
                self.extract_pattern_name(pattern)
            }
            FunctionParamKind::SelfValue | FunctionParamKind::SelfValueMut |
            FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut |
            FunctionParamKind::SelfOwn | FunctionParamKind::SelfOwnMut |
            FunctionParamKind::SelfRefChecked | FunctionParamKind::SelfRefCheckedMut |
            FunctionParamKind::SelfRefUnsafe | FunctionParamKind::SelfRefUnsafeMut => {
                Some("self".to_string())
            }
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
    fn extract_pattern_name_and_mutable(&self, pattern: &verum_ast::Pattern) -> Option<(String, bool)> {
        use verum_ast::PatternKind;
        match &pattern.kind {
            PatternKind::Ident { name, mutable, .. } => Some((name.name.to_string(), *mutable)),
            PatternKind::Paren(inner) => self.extract_pattern_name_and_mutable(inner),
            _ => None,
        }
    }

    /// Extracts the parameter name and mutability from a function parameter.
    fn extract_param_name_and_mutable(&self, param: &verum_ast::FunctionParam) -> Option<(String, bool)> {
        use verum_ast::FunctionParamKind;
        match &param.kind {
            FunctionParamKind::Regular { pattern, .. } => {
                self.extract_pattern_name_and_mutable(pattern)
            }
            // Self parameters: SelfValueMut, SelfRefCheckedMut, SelfRefUnsafeMut are mutable
            FunctionParamKind::SelfValueMut | FunctionParamKind::SelfRefCheckedMut |
            FunctionParamKind::SelfRefUnsafeMut => Some(("self".to_string(), true)),
            FunctionParamKind::SelfValue |
            FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut |
            FunctionParamKind::SelfOwn | FunctionParamKind::SelfOwnMut |
            FunctionParamKind::SelfRefChecked | FunctionParamKind::SelfRefUnsafe => {
                Some(("self".to_string(), false))
            }
        }
    }

    /// Converts a TypeKind to VarTypeKind for instruction selection.
    ///
    /// This is crucial for generating correct float vs integer operations.
    fn type_kind_to_var_type(&self, type_kind: &verum_ast::ty::TypeKind) -> context::VarTypeKind {
        use verum_ast::ty::TypeKind;
        match type_kind {
            TypeKind::Int => context::VarTypeKind::Int,
            TypeKind::Float => context::VarTypeKind::Float,
            TypeKind::Bool => context::VarTypeKind::Bool,
            TypeKind::Char => context::VarTypeKind::Char,
            TypeKind::Text => context::VarTypeKind::Text,
            TypeKind::Unit => context::VarTypeKind::Unit,
            // Handle type aliases represented as Path types
            TypeKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    match ident.name.as_str() {
                        "Float" | "Float64" | "f64" | "Float32" | "f32" => context::VarTypeKind::Float,
                        "Int" | "Int64" | "i64" | "Int32" | "i32"
                        | "UInt8" | "u8" | "Byte" | "UInt16" | "u16"
                        | "UInt32" | "u32" | "UInt64" | "u64"
                        | "Int8" | "i8" | "Int16" | "i16"
                        | "UInt128" | "u128" | "Int128" | "i128"
                        | "UIntSize" | "usize" | "IntSize" | "isize" => context::VarTypeKind::Int,
                        "Bool" => context::VarTypeKind::Bool,
                        "Char" => context::VarTypeKind::Char,
                        "Text" => context::VarTypeKind::Text,
                        _ => context::VarTypeKind::Unknown,
                    }
                } else {
                    context::VarTypeKind::Unknown
                }
            }
            // For complex types, we return Unknown and let runtime handle them
            _ => context::VarTypeKind::Unknown,
        }
    }

    /// Converts a type name string to VarTypeKind for instruction selection.
    ///
    /// Used when we have type information as a string (e.g., from variant payload types).
    fn type_name_to_var_type(&self, type_name: &str) -> context::VarTypeKind {
        match type_name {
            "Float" | "Float64" | "f64" | "Float32" | "f32" => context::VarTypeKind::Float,
            "Int" | "Int64" | "i64" | "Int32" | "i32"
            | "UInt8" | "u8" | "Byte" | "UInt16" | "u16"
            | "UInt32" | "u32" | "UInt64" | "u64"
            | "Int8" | "i8" | "Int16" | "i16"
            | "UInt128" | "u128" | "Int128" | "i128"
            | "UIntSize" | "usize" | "IntSize" | "isize" => context::VarTypeKind::Int,
            "Bool" => context::VarTypeKind::Bool,
            "Char" => context::VarTypeKind::Char,
            "Text" => context::VarTypeKind::Text,
            "()" => context::VarTypeKind::Unit,
            _ => context::VarTypeKind::Unknown,
        }
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
    fn resolve_field_type_ref(&self, ty: &verum_ast::ty::Type, generic_param_map: &std::collections::HashMap<String, u16>) -> TypeRef {
        use verum_ast::ty::{TypeKind, PathSegment};
        // Check if the type is a simple path that matches a generic param
        if let TypeKind::Path(path) = &ty.kind {
            let type_name = path.segments.iter().find_map(|seg| {
                if let PathSegment::Name(ident) = seg {
                    Some(ident.name.to_string())
                } else {
                    None
                }
            }).unwrap_or_default();
            if let Some(&param_idx) = generic_param_map.get(&type_name) {
                return TypeRef::Generic(crate::types::TypeParamId(param_idx));
            }
        }
        // Fall back to standard resolution
        self.ast_type_to_type_ref(ty)
    }

    /// This enables storing return type information for method dispatch prefixing.
    fn ast_type_to_type_ref(&self, ty: &verum_ast::ty::Type) -> TypeRef {
        use verum_ast::ty::{TypeKind, PathSegment};

        match &ty.kind {
            TypeKind::Int => TypeRef::Concrete(TypeId::INT),
            TypeKind::Float => TypeRef::Concrete(TypeId::FLOAT),
            TypeKind::Bool => TypeRef::Concrete(TypeId::BOOL),
            TypeKind::Text => TypeRef::Concrete(TypeId::TEXT),
            TypeKind::Unit => TypeRef::Concrete(TypeId::UNIT),
            TypeKind::Never => TypeRef::Concrete(TypeId::NEVER),
            TypeKind::Path(path) => {
                // Extract the first segment name for primitive type lookup
                let type_name = path.segments.iter().find_map(|seg| {
                    if let PathSegment::Name(ident) = seg {
                        Some(ident.name.to_string())
                    } else {
                        None
                    }
                }).unwrap_or_default();

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
                    let arg_refs: Vec<TypeRef> = args.iter()
                        .filter_map(|arg| {
                            // Extract the inner Type from GenericArg::Type
                            if let verum_ast::ty::GenericArg::Type(inner_ty) = arg {
                                Some(self.ast_type_to_type_ref(inner_ty))
                            } else {
                                None
                            }
                        })
                        .collect();
                    TypeRef::Instantiated { base: base_id, args: arg_refs }
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
                    let elem_refs: Vec<TypeRef> = elements.iter()
                        .map(|e| self.ast_type_to_type_ref(e))
                        .collect();
                    TypeRef::Tuple(elem_refs)
                }
            }
            TypeKind::Function { params, return_type, contexts, .. } => {
                let param_refs: Vec<TypeRef> = params.iter()
                    .map(|p| self.ast_type_to_type_ref(p))
                    .collect();
                let ret_ref = self.ast_type_to_type_ref(return_type);
                let ctx_refs: smallvec::SmallVec<[crate::types::ContextRef; 2]> = contexts.requirements.iter()
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
            TypeKind::Char => TypeRef::Concrete(TypeId::INT), // Char is stored as Int
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
        use verum_ast::ty::{TypeKind, PathSegment};

        match &ty.kind {
            // Unit and Never have no extractable type name for dispatch
            TypeKind::Unit | TypeKind::Never => None,
            // Other primitives use the canonical display name
            _ if ty.kind.primitive_name().is_some() => {
                ty.kind.primitive_name().map(|n| n.to_string())
            }
            TypeKind::Path(path) => {
                // Extract the first type name from the path
                path.segments.iter().find_map(|seg| {
                    if let PathSegment::Name(ident) = seg {
                        Some(ident.name.to_string())
                    } else {
                        None
                    }
                })
            }
            TypeKind::Generic { base, args } => {
                // For generic types like Result<T, E>, extract the full type including args
                let base_name = self.extract_type_name(base)?;
                if args.is_empty() {
                    Some(base_name)
                } else {
                    // Build the full type string with generic arguments
                    let arg_strs: Vec<String> = args.iter().filter_map(|arg| {
                        match arg {
                            verum_ast::ty::GenericArg::Type(ty) => self.extract_type_name(ty),
                            verum_ast::ty::GenericArg::Const(_) => None,
                            verum_ast::ty::GenericArg::Lifetime(_) => None,
                            verum_ast::ty::GenericArg::Binding(_) => None,
                        }
                    }).collect();
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
                let inner_name = self.extract_type_name(inner)
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
                    let arg_names: Vec<String> = args.iter().map(|a| self.type_ref_to_name(a)).collect();
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
                let impl_type_generics: Vec<String> = impl_decl.generics.iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                // Pre-populate impl block const generics - enables recognizing SIZE, N etc.
                // in impl<const SIZE: Int> StackAllocator<SIZE> { ... }
                let impl_const_generics: Vec<String> = impl_decl.generics.iter()
                    .filter_map(|g| {
                        if let verum_ast::ty::GenericParamKind::Const { name, .. } = &g.kind {
                            Some(name.name.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                for impl_item in impl_decl.items.iter() {
                    // Honour `@cfg` gates on impl items.  Same pattern
                    // as `compile_item_lenient`'s impl loop — walk both
                    // ImplItem.attributes and FunctionDecl.attributes
                    // because the parser places attrs on the inner
                    // decl when present.  Without this, an
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
                        self.compile_function(func, type_name.as_ref())?;
                        // Compile any nested functions in this function's body
                        if let verum_common::Maybe::Some(ref body) = func.body {
                            self.compile_nested_functions(body)?;
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
    fn compile_function(&mut self, func: &FunctionDecl, impl_type_name: Option<&String>) -> CodegenResult<()> {
        let base_name = func.name.name.to_string();

        // Build the lookup name - use qualified name for impl functions
        let lookup_name = if let Some(type_name) = impl_type_name {
            format!("{}.{}", type_name, base_name)
        } else {
            base_name.clone()
        };

        // Get the pre-registered function info (for ID and properties).
        // Use arity-based lookup to resolve collisions between user functions
        // and stdlib methods with the same name but different arities.
        let param_count = func.params.len();
        let func_info = self.ctx.lookup_function_with_arity(&lookup_name, param_count)
            .ok_or_else(|| CodegenError::internal(format!("function not registered: {}", lookup_name)))?
            .clone();

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
        self.ctx.begin_function(&lookup_name, &params_with_mutability, func_info.return_type.clone());

        // Set current function's return type name for variant disambiguation.
        // When a variant name collides (e.g., "Lt" in both user's "Ordering" and
        // stdlib's "GeneralCategory"), this allows preferring the correct parent type.
        self.ctx.current_return_type_name = func_info.return_type_name.clone();

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
            self.ctx.variable_type_names.insert("self".to_string(), type_name.clone());
        }

        // Set required contexts from the function's using clause.
        // This enables compile_method_call to emit CtxGet for context receivers.
        self.ctx.set_required_contexts(&func_info.contexts);

        // Register named/aliased context bindings from AST.
        // Grammar: named_context = identifier ':' context_path | context_path 'as' identifier
        for ctx in &func.contexts {
            if ctx.is_negative { continue; }
            let ctx_type_name = ctx.path.segments.last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_type_name.is_empty() { continue; }
            if let verum_common::Maybe::Some(ref name_ident) = ctx.name {
                self.ctx.context_aliases.insert(name_ident.name.to_string(), ctx_type_name.clone());
            }
            if let verum_common::Maybe::Some(ref alias_ident) = ctx.alias {
                self.ctx.context_aliases.insert(alias_ident.name.to_string(), ctx_type_name.clone());
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
        // AND for resolving field indices in type-specific record access
        for ((param_name, _), param) in params_with_mutability.iter().zip(func.params.iter()) {
            if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                let var_type = self.type_kind_to_var_type(&ty.kind);
                self.ctx.register_variable_type(param_name, var_type);
                // Track type name for field index resolution
                let type_name = Self::extract_type_name_from_ast(ty);
                if type_name != "()" && !type_name.is_empty() {
                    self.ctx.variable_type_names.insert(param_name.clone(), type_name);
                }
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
        //   Rule 1  `T{pred}`            — predicate uses `it`.
        //   Rule 2  `T where |x| pred`   — predicate uses `x`.
        //   Rule 3  `x: T where pred`    — predicate uses `x`.
        //
        // The binding name is aliased to the parameter's register via
        // a `Mov` into a freshly-named local so `compile_expr` on the
        // predicate resolves the reference normally. When the binding
        // happens to coincide with the parameter name (common case
        // for pattern `fn f(x: Int { x > 0 })` with implicit `it`
        // collision guarded against), no alias is introduced.
        for param in func.params.iter() {
            let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind else { continue };
            let Some((param_name, _)) = self.extract_param_name_and_mutable(param) else { continue };

            // Extract (predicate_expr, binding_name) from the canonical
            // `Refined` node (post — the sigma surface form parses
            // to `Refined` with `predicate.binding = Some(name)`).
            let (pred_expr, binding_name) = match &ty.kind {
                verum_ast::ty::TypeKind::Refined { predicate, .. } => {
                    let bname = match &predicate.binding {
                        verum_common::Maybe::Some(id) => id.name.to_string(),
                        verum_common::Maybe::None    => "it".to_string(),
                    };
                    (predicate.expr.clone(), bname)
                }
                _ => continue,
            };

            // Resolve the parameter register. If resolution fails we
            // skip this obligation — a missing param register means
            // the function never reached body compilation (extern /
            // placeholder) and there is nothing to assert against.
            let Ok(param_reg) = self.ctx.get_var_reg(&param_name) else { continue };

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
                    self.ctx.emit(Instruction::Assert { cond: cond_reg, message_id });
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
                self.ctx.emit(Instruction::Mov { dst: alias_reg, src: param_reg });

                let vt = self.ctx.get_variable_type(&param_name);
                self.ctx.register_variable_type(&binding_name, vt);
                if let Some(type_name) = self.ctx.variable_type_names.get(&param_name).cloned() {
                    self.ctx.variable_type_names.insert(binding_name.clone(), type_name);
                }

                if let Ok(Some(cond_reg)) = self.compile_expr(&pred_expr) {
                    self.ctx.emit(Instruction::Assert { cond: cond_reg, message_id });
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
            if !ctx_decl.is_negative { continue; }
            let ctx_type_name = ctx_decl.path.segments.last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if ctx_type_name.is_empty() { continue; }
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
        //   - `@intrinsic("tcp_listen") pub fn __tcp_listen_raw(port: Int) -> Int;`
        //   - `@intrinsic("tcp_recv")   pub fn __tcp_recv_raw(fd: Int, max: Int) -> Text;`
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

        // Compile the body
        if let Some(ref body) = func.body {
            match body {
                verum_ast::FunctionBody::Block(block) => {
                    let result = self.compile_block(block)
                        .map_err(|e| e.with_context(format!("in function {}", lookup_name)))?;
                    // Return the block result if present (implicit return)
                    if let Some(reg) = result {
                        self.emit_return_refinement_assert(reg, func.return_type.as_ref(), &lookup_name);
                        self.ctx.emit(Instruction::Ret { value: reg });
                    }
                }
                verum_ast::FunctionBody::Expr(expr) => {
                    let result = self.compile_expr(expr)
                        .map_err(|e| e.with_context(format!("in function {}", lookup_name)))?;
                    // Return the expression result
                    if let Some(reg) = result {
                        self.emit_return_refinement_assert(reg, func.return_type.as_ref(), &lookup_name);
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

        // End function compilation
        let (instructions, register_count) = self.ctx.end_function();

        // Create VBC function
        let name_id = StringId(self.intern_string(&lookup_name));
        let mut descriptor = FunctionDescriptor::new(name_id);
        descriptor.id = func_info.id;
        descriptor.register_count = register_count;
        descriptor.locals_count = params_with_mutability.len() as u16;
        descriptor.optimization_hints.is_pure = func.is_pure;
        // Set return type from function info (default is UNIT)
        if let Some(ref ret_type) = func_info.return_type {
            descriptor.return_type = ret_type.clone();
        }

        // Populate parameter descriptors for proper method dispatch matching.
        // This enables the interpreter to match methods by parameter count.
        for ((param_name, is_mut), param) in params_with_mutability.iter().zip(func.params.iter()) {
            let type_ref = if let verum_ast::FunctionParamKind::Regular { ty, .. } = &param.kind {
                self.ast_type_to_type_ref(ty)
            } else {
                TypeRef::Concrete(TypeId::UNIT)
            };
            let param_name_id = StringId(self.intern_string(param_name));
            descriptor.params.push(ParamDescriptor {
                name: param_name_id,
                type_ref,
                is_mut: *is_mut,
                default: None,
            });
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

        // Set test flag if function has @test attribute (only for user code)
        if self.propagate_test_attr && func.attributes.iter().any(|a| a.is_named("test")) {
            descriptor.is_test = true;
        }

        // Map context names to ContextRef IDs and register in context_names table
        for ctx_name in &func_info.contexts {
            let ctx_id = self.intern_context_name(ctx_name);
            descriptor.contexts.push(crate::types::ContextRef(ctx_id));
        }

        let vbc_func = VbcFunction::new(descriptor, instructions);

        self.functions.push(vbc_func);
        Ok(())
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
            self.ctx.emit(Instruction::Mov { dst: safe_reg, src: result_reg });
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
        for (_name, var_reg) in vars.iter().rev() {
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
                    verum_common::Maybe::None    => "it".to_string(),
                };
                (predicate.expr.clone(), bname)
            }
            _ => return,
        };

        // Derive VarType from the underlying base (peel the refinement
        // layer) so predicate compilation picks the right comparisons.
        let base_vt = match &ret_ty.kind {
            verum_ast::ty::TypeKind::Refined { base, .. } => {
                self.type_kind_to_var_type(&base.kind)
            }
            _ => context::VarTypeKind::Unknown,
        };
        let base_type_name = match &ret_ty.kind {
            verum_ast::ty::TypeKind::Refined { base, .. } => {
                Self::extract_type_name_from_ast(base)
            }
            _ => String::new(),
        };

        let message_id = {
            let msg = format!("refinement violation: return value of `{}`", fn_name);
            self.intern_string(&msg)
        };

        self.ctx.enter_scope();
        let alias_reg = self.ctx.define_var(&binding_name, false);
        self.ctx.emit(Instruction::Mov { dst: alias_reg, src: result_reg });

        self.ctx.register_variable_type(&binding_name, base_vt);
        if !base_type_name.is_empty() && base_type_name != "()" {
            self.ctx.variable_type_names.insert(binding_name.clone(), base_type_name);
        }

        if let Ok(Some(cond_reg)) = self.compile_expr(&pred_expr) {
            self.ctx.emit(Instruction::Assert { cond: cond_reg, message_id });
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
                s = &s[prefix.len()..s.len()-1];
            }
        }
        s
    }

    fn resolve_field_index(&mut self, type_name: Option<&str>, field_name: &str) -> u32 {
        if let Some(tn) = type_name {
            // Try exact match first
            if let Some(fields) = self.type_field_layouts.get(tn)
                && let Some(pos) = fields.iter().position(|f| f == field_name) {
                    if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                        tracing::debug!("[FIELD] {}.{} → per-type idx {} (fields: {:?})", tn, field_name, pos, fields);
                    }
                    return pos as u32;
                }
            // Try with generic params stripped (e.g., "Slot<K, V>" → "Slot")
            if let Some(angle) = tn.find('<') {
                let base = &tn[..angle];
                if let Some(fields) = self.type_field_layouts.get(base)
                    && let Some(pos) = fields.iter().position(|f| f == field_name) {
                        if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                            tracing::debug!("[FIELD] {}.{} → per-type idx {} (stripped from '{}', fields: {:?})", base, field_name, pos, tn, fields);
                        }
                        return pos as u32;
                    }
            }
            // Try stripping transparent wrappers (Heap<X> → X, Shared<X> → X, &X → X)
            let unwrapped = Self::strip_wrapper_type(tn);
            if unwrapped != tn {
                // Try exact match on unwrapped
                if let Some(fields) = self.type_field_layouts.get(unwrapped)
                    && let Some(pos) = fields.iter().position(|f| f == field_name) {
                        if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                            tracing::debug!("[FIELD] {}.{} → per-type idx {} (unwrapped from '{}', fields: {:?})", unwrapped, field_name, pos, tn, fields);
                        }
                        return pos as u32;
                    }
                // Try with generic params stripped on the unwrapped type
                if let Some(angle) = unwrapped.find('<') {
                    let base = &unwrapped[..angle];
                    if let Some(fields) = self.type_field_layouts.get(base)
                        && let Some(pos) = fields.iter().position(|f| f == field_name) {
                            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                                tracing::debug!("[FIELD] {}.{} → per-type idx {} (unwrapped+stripped from '{}', fields: {:?})", base, field_name, pos, tn, fields);
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
                    if registered_simple == simple_name && type_n != tn
                        && let Some(pos) = fields.iter().position(|f| f == field_name) {
                            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                                tracing::debug!("[FIELD] {}.{} → cross-module match '{}' idx {} (fields: {:?})", tn, field_name, type_n, pos, fields);
                            }
                            return pos as u32;
                        }
                }
            }

            // Type known but field not found — fall through to type=None scan below.
            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                tracing::debug!("[FIELD] type '{}' NOT in type_field_layouts, field '{}' → scanning all types (fn={})",
                    tn, field_name, self.ctx.current_function.as_deref().unwrap_or("?"));
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
                    tracing::debug!("[FIELD] scan: field '{}' → unique {}.{} idx {}",
                        field_name, type_n, field_name, pos);
                }
                return pos as u32;
            }
            if candidates.len() > 1 {
                // All at same position? Use that.
                let first_pos = candidates[0].1;
                if candidates.iter().all(|(_, p)| *p == first_pos) {
                    if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                        tracing::debug!("[FIELD] scan: field '{}' → all at idx {} ({} types)",
                            field_name, first_pos, candidates.len());
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
                    tracing::debug!("[FIELD] scan: field '{}' ambiguous, picked {}.{} idx {} (most fields)",
                        field_name, best.0, field_name, best.1);
                }
                return best.1 as u32;
            }
            if std::env::var("VERUM_DEBUG_FIELDS").is_ok() {
                tracing::debug!("[FIELD] scan: field '{}' → not found in any type", field_name);
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
        self.type_field_layouts.get(type_name).map(|f| f.len() as u32)
    }

    /// Returns the type name of a field within a record type.
    fn field_type_name(&self, type_name: &str, field_name: &str) -> Option<&str> {
        self.type_field_type_names
            .get(&(type_name.to_string(), field_name.to_string()))
            .map(|s| s.as_str())
    }

    /// Generic parameters are preserved so that element types can be extracted
    /// later via `extract_element_type` (e.g., "List<Token>" → "Token").
    fn extract_type_name_from_ast(ty: &verum_ast::ty::Type) -> String {
        use verum_ast::ty::{TypeKind, PathSegment, GenericArg};
        if let Some(name) = ty.kind.primitive_name() {
            return name.to_string();
        }
        match &ty.kind {
            TypeKind::Path(path) => {
                // Get the last segment name (handles qualified paths like core.collections.List)
                path.segments.last()
                    .and_then(|seg| match seg {
                        PathSegment::Name(ident) => Some(ident.name.to_string()),
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
                    let arg_strs: Vec<String> = args.iter().map(|arg| {
                        match arg {
                            GenericArg::Type(ty) => Self::extract_type_name_from_ast(ty),
                            GenericArg::Const(_) => "_".to_string(),
                            _ => "_".to_string(),
                        }
                    }).collect();
                    format!("{}<{}>", base_name, arg_strs.join(", "))
                }
            }
            TypeKind::Reference { inner, .. }
            | TypeKind::CheckedReference { inner, .. }
            | TypeKind::UnsafeReference { inner, .. }
            | TypeKind::Pointer { inner, .. } => {
                Self::extract_type_name_from_ast(inner)
            }
            TypeKind::Slice(inner) => {
                format!("[{}]", Self::extract_type_name_from_ast(inner))
            }
            TypeKind::DynProtocol { bounds, .. } => {
                // dyn Protocol → "dyn:Protocol" for dispatch tracking
                let bound_names: Vec<String> = bounds.iter()
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
    fn register_record_fields(&mut self, type_name: &str, field_names: Vec<String>, field_types: Vec<String>) {
        // Intern all field names to ensure they have assigned indices
        for name in &field_names {
            self.intern_field_name(name);
        }
        // Store field type names for type inference through field access chains.
        // Only insert if not already present — user-defined types are registered
        // first (Pass 1b) and must not be overwritten by stdlib types (Pass 1c).
        if !self.type_field_layouts.contains_key(type_name) {
            for (name, ty) in field_names.iter().zip(field_types.iter()) {
                self.type_field_type_names.insert(
                    (type_name.to_string(), name.clone()),
                    ty.clone(),
                );
            }
            self.type_field_layouts.insert(type_name.to_string(), field_names.clone());

            // Cross-module field access support: also register under the simple name
            // (without module path) so imports using unqualified names can find fields.
            // e.g., "module_a.Point" → also register as "Point"
            if type_name.contains('.') {
                let simple = type_name.rsplit('.').next().unwrap_or(type_name);
                if !self.type_field_layouts.contains_key(simple) {
                    for (name, ty) in field_names.iter().zip(field_types.iter()) {
                        self.type_field_type_names.insert(
                            (simple.to_string(), name.clone()),
                            ty.clone(),
                        );
                    }
                    self.type_field_layouts.insert(simple.to_string(), field_names);
                }
            }
        } else {
            // Type already registered (user type takes priority over stdlib).
            // Still intern the field names for global lookup fallback.
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

    /// Builds the final VBC module.
    fn build_module(&mut self) -> CodegenResult<VbcModule> {
        let mut module = VbcModule::new(self.config.module_name.clone());

        // IMPORTANT: Intern strings FIRST and build mapping from codegen index to module StringId.
        // Codegen uses simple indices (0, 1, 2...) while VbcModule uses byte offsets.
        // This mapping is needed for function names, constants, and any other StringIds.
        let string_id_map: Vec<StringId> = self.ctx.strings
            .iter()
            .map(|s| module.intern_string(s))
            .collect();

        // Sort functions by ID so array index matches function ID
        // (closures compiled during parent functions may be pushed out of order)
        self.functions.sort_by_key(|f| f.descriptor.id.0);

        // Build func_id remapping: old sparse IDs → new contiguous 0-based IDs.
        // When stdlib functions are imported, next_func_id starts high, so the
        // module's own functions have non-zero-based IDs. The module stores functions
        // in a Vec indexed by FunctionId, so IDs must be contiguous from 0.
        let func_id_remap: std::collections::HashMap<u32, u32> = self.functions
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
            // Remap drop_fn from sparse ID to contiguous 0-based ID
            if let Some(drop_fn) = remapped_ty.drop_fn
                && let Some(&new_id) = func_id_remap.get(&drop_fn) {
                    remapped_ty.drop_fn = Some(new_id);
                }
            // Remap clone_fn from sparse ID to contiguous 0-based ID
            if let Some(clone_fn) = remapped_ty.clone_fn
                && let Some(&new_id) = func_id_remap.get(&clone_fn) {
                    remapped_ty.clone_fn = Some(new_id);
                }
            // Remap protocol method FunctionIds from sparse to contiguous 0-based IDs
            for proto_impl in remapped_ty.protocols.iter_mut() {
                for fn_id in proto_impl.methods.iter_mut() {
                    if *fn_id != u32::MAX
                        && let Some(&new_id) = func_id_remap.get(fn_id) {
                            *fn_id = new_id;
                        }
                }
            }
            module.add_type(remapped_ty);
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

        Ok(module)
    }

    /// Returns codegen statistics.
    pub fn stats(&self) -> &CodegenStats {
        &self.ctx.stats
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &CodegenConfig {
        &self.config
    }
}

/// Cross-module type-table health (#170).  Returned by
/// [`VbcCodegen::verify_global_type_table_consistency`].  See that
/// method's docstring for the bug classes each field tracks.
///
/// Note: `MakeVariant`-level orphan detection is intentionally NOT
/// part of this report.  At a single-module-with-mounts granularity
/// the cross-module-variant case dominates — most "orphans" are
/// legitimate references to variants whose declaring module wasn't
/// fully loaded.  Use [`VbcCodegen::find_orphan_make_variants`] for
/// the diagnostic; treat its output as informational unless you
/// know every transitively-referenced module is in the table.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TypeTableHealthReport {
    /// Multiple `TypeDescriptor`s share a single `TypeId.0`.  A real
    /// program never has this — TypeIds are supposed to be unique.
    /// Caused by name-collision merge in `type_name_to_id`.
    pub duplicate_ids: Vec<DuplicateTypeId>,
    /// Multiple `TypeDescriptor`s share a name but report different
    /// `TypeId.0` values.  Indicates the codegen ran multiple
    /// type-allocation passes that didn't reuse the prior pass's
    /// registration.
    pub duplicate_names_with_different_ids: Vec<DuplicateNameDifferentId>,
    /// A sum type's variant tags are not dense `0..variants.len()`
    /// or contain duplicates.  Runtime variant dispatch indexes by
    /// tag, so any gap or duplicate yields wrong-variant dispatch.
    pub variant_tag_anomalies: Vec<VariantTagAnomaly>,
}

impl TypeTableHealthReport {
    /// `true` when every category is empty.  Use this in a CI gate:
    /// `assert!(codegen.verify_global_type_table_consistency().is_clean())`.
    pub fn is_clean(&self) -> bool {
        self.duplicate_ids.is_empty()
            && self.duplicate_names_with_different_ids.is_empty()
            && self.variant_tag_anomalies.is_empty()
    }

    /// Total number of issues across all categories.  Useful for a
    /// "ratchet" baseline test that lets the count fall but never
    /// rise.
    pub fn issue_count(&self) -> usize {
        self.duplicate_ids.len()
            + self.duplicate_names_with_different_ids.len()
            + self.variant_tag_anomalies.len()
    }

    /// Convert to a `CodegenError` when issues exist.  Bundles every
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
                d.type_id, d.descriptor_names.len(), d.descriptor_names,
            ));
        }
        for d in &self.duplicate_names_with_different_ids {
            msg.push_str(&format!(
                "  - name `{}` declared with {} different TypeIds: {:?}\n",
                d.name, d.type_ids.len(), d.type_ids,
            ));
        }
        for a in &self.variant_tag_anomalies {
            msg.push_str(&format!(
                "  - variant tags non-dense in `{}` (TypeId({})): expected {} \
                 variants, max tag seen {}, duplicates {:?}, missing {:?}\n",
                a.type_name, a.type_id, a.expected_count, a.max_tag_seen,
                a.duplicate_tags, a.missing_tags,
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
/// declared TypeDescriptor carries".  Global pass equivalent of
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
        let config = CodegenConfig::new("test")
            .with_optimization_level(10); // Should be clamped to 3

        assert_eq!(config.optimization_level, 3);
    }

    /// Default config is lenient — partial / forward-referenced stdlib
    /// state still builds.  `with_strict_codegen()` opts in to promoting
    /// bug-class skips to hard errors.  Tracked under #166.
    #[test]
    fn validate_default_off_until_stdlib_clean() {
        // Pin: until pre-existing stdlib emit bugs are cleaned up
        // (TypeId(515) dangling refs, function-end-vs-instruction-stream
        // length divergence, archive-header counts disagreeing with
        // section bodies), the structural validator is OFF by default.
        // CI opts in via `with_validation()`.  This pin breaks together
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
        // even while the default is off.  When stdlib emit cleans up,
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
        use crate::types::{TypeDescriptor, TypeId, StringId, TypeKind};
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
    /// surfaces as `DuplicateNameDifferentId`.  Distinct from the
    /// duplicate-id case: here the *name* collides while the ids
    /// disagree, indicating the codegen ran multiple type-allocation
    /// passes that didn't share state.
    #[test]
    fn test_global_type_table_detects_same_name_different_ids() {
        use crate::types::{TypeDescriptor, TypeId, StringId, TypeKind};
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
    /// not by two descriptors sharing an id.  The
    /// duplicate-name-with-different-ids check stays silent because
    /// neither name is itself ambiguous.
    #[test]
    fn test_global_type_table_two_descriptors_same_id_different_names_flagged() {
        use crate::types::{TypeDescriptor, TypeId, StringId, TypeKind};
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
        // report is NOT clean.  This is the intended behaviour: aliases
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
                "allocated id {} below FIRST_USER ({})", id.0, TypeId::FIRST_USER,
            );
            assert!(
                !(256..260).contains(&id.0),
                "allocated id {} inside meta-system range 256..260", id.0,
            );
            assert!(
                !(TypeId::FIRST_SEMANTIC..=TypeId::LAST_SEMANTIC).contains(&id.0),
                "allocated id {} inside semantic-collection range {}..={}",
                id.0, TypeId::FIRST_SEMANTIC, TypeId::LAST_SEMANTIC,
            );
            assert!(
                seen.insert(id.0),
                "duplicate id {} returned from alloc_user_type_id", id.0,
            );
        }
        // After 1100 allocations starting from id 16, we should have
        // walked past the 4-id meta range (256..260) and the
        // 512-id semantic range (512..=1023).  Last allocated id
        // should therefore be FIRST_USER + 1100 + 4 + 512 - 1 =
        // 16 + 1100 + 516 - 1 = 1631 (off-by-one tolerated; the
        // strict check above is enough).
    }

    /// Synthetic-table smoke (#170): a sum type with a tag gap
    /// surfaces as a `VariantTagAnomaly`.  Runtime variant dispatch
    /// indexes by tag, so any gap = wrong-variant dispatch.
    #[test]
    fn test_global_type_table_detects_variant_tag_gap() {
        use crate::types::{
            TypeDescriptor, TypeId, StringId, TypeKind,
            VariantDescriptor, VariantKind,
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
