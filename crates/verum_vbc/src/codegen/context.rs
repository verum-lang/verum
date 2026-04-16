//! Lowering context for VBC codegen.
//!
//! Tracks compilation state including:
//! - Current function being compiled
//! - Register allocator
//! - Label generation for jumps
//! - Loop context for break/continue
//! - Defer stack for cleanup
//! - CBGR tier decisions for reference operations

use super::error::{CodegenError, CodegenResult};
use super::registers::{RegisterAllocator, RegisterInfo};
use crate::cbgr::DereferenceCodegen;
use crate::instruction::{Instruction, Reg};
use crate::module::{ConstId, FunctionId};
use crate::types::{CbgrTier, TypeRef};
use std::collections::{HashMap, HashSet};
use verum_cbgr::tier_types::Tier0Reason;
use verum_common::Map;

/// Context for VBC code generation.
///
/// Tracks all state needed during function compilation.
#[derive(Debug)]
pub struct CodegenContext {
    /// Register allocator for current function.
    pub registers: RegisterAllocator,

    /// Generated instructions for current function.
    pub instructions: Vec<Instruction>,

    /// Source spans for each instruction (parallel to instructions vec).
    /// Used to build SourceMap for DWARF debug info.
    /// Entry i corresponds to instructions[i]'s source location.
    pub instruction_spans: Vec<verum_common::Span>,

    /// Current source span (set before emitting instructions).
    current_span: verum_common::Span,

    /// Label counter for generating unique labels.
    label_counter: u32,

    /// Map from label names to instruction indices.
    labels: HashMap<String, usize>,

    /// Pending forward jumps (label name → instruction indices to patch).
    forward_jumps: HashMap<String, Vec<usize>>,

    /// Stack of loop contexts for break/continue.
    loop_stack: Vec<LoopContext>,

    /// Stack of defer expressions per scope.
    defer_stack: Vec<Vec<DeferInfo>>,

    /// Current function name (for error messages).
    pub current_function: Option<String>,

    /// Whether we're inside a function body.
    pub in_function: bool,

    /// Return type of current function (for type checking).
    pub return_type: Option<TypeRef>,

    /// Return type name of current function (for variant disambiguation).
    /// When a variant name collides with a stdlib variant (e.g., "Lt" exists in both
    /// user-defined "Ordering" and stdlib "GeneralCategory"), this is used to prefer
    /// the variant whose parent type matches the function's return type.
    pub current_return_type_name: Option<String>,

    /// Constant pool for current module.
    pub constants: Vec<ConstantEntry>,

    /// String table for current module.
    pub strings: Vec<String>,

    /// String interning map.
    string_intern: HashMap<String, u32>,

    /// Byte array table for current module.
    pub bytes: Vec<Vec<u8>>,

    /// Byte array interning map.
    bytes_intern: HashMap<Vec<u8>, u32>,

    /// Function registry for lookups.
    pub functions: HashMap<String, FunctionInfo>,

    /// When true, register_function will not overwrite existing entries.
    /// Used when importing stdlib modules after user code has been registered.
    pub prefer_existing_functions: bool,

    /// Statistics for codegen.
    pub stats: CodegenStats,

    /// CBGR tier context from escape analysis.
    ///
    /// Contains tier decisions that determine which instruction
    /// variant to emit for reference operations.
    pub tier_context: TierContext,

    /// Number of yield points (suspend points) in the current generator function.
    /// Each `yield` expression in a `fn*` function increments this counter.
    /// Used for state machine validation and resume-point indexing.
    pub suspend_point_count: u16,

    /// Variable type tracking for correct instruction selection.
    ///
    /// Maps variable names to their inferred basic types.
    /// Used to select int vs float instructions for operations on variables.
    pub variable_types: HashMap<String, VarTypeKind>,

    /// Constant type tracking for correct instruction selection.
    ///
    /// Maps constant names to their declared types.
    /// Unlike `variable_types`, this persists across function compilations.
    /// Used to select int vs float instructions for operations on constants.
    /// e.g., `const PI: Float = 3.14;` → `-PI` should use NegF, not NegI.
    pub constant_types: HashMap<String, VarTypeKind>,

    /// Variable type name tracking for custom Eq protocol dispatch.
    ///
    /// Maps variable names to their custom type names (e.g., "err" → "OSError").
    /// Used to determine if `==` should dispatch to a custom `implement Eq` method.
    pub variable_type_names: HashMap<String, String>,

    /// Snapshot of `variable_type_names` from the last compiled function body.
    /// Preserved across function boundaries so the playground can read bindings
    /// after compilation (the main map gets cleared between functions).
    pub last_function_variable_types: HashMap<String, String>,

    /// Current match scrutinee type name for resolving variant patterns.
    ///
    /// When compiling `match expr { V6(x) => ... }`, we need to know if V6 refers to
    /// `IpAddr.V6` or `SocketAddr.V6`. If the scrutinee `expr` has a known type,
    /// we store it here so pattern binding can use the qualified variant name.
    pub match_scrutinee_type: Option<String>,

    /// Registers that contain raw FFI pointers (not CBGR references).
    ///
    /// When dereferencing values in these registers, we emit DerefRaw/DerefMutRaw
    /// instructions which bypass CBGR validation. This is necessary because FFI
    /// functions return raw C pointers that don't have CBGR headers.
    ///
    /// FFI raw pointer handling: registers containing pointers returned from FFI (extern)
    /// functions are tracked here. Dereferences emit DerefRaw/DerefMutRaw which bypass
    /// CBGR validation since FFI pointers lack CBGR headers (no generation/epoch metadata).
    pub raw_pointer_regs: HashSet<Reg>,

    /// Generic type parameters in scope for the current function.
    ///
    /// When compiling generic functions like `fn foo<T, U>()`, the type parameters
    /// T and U are added here. This allows `compile_simple_path()` to recognize
    /// type parameters as valid identifiers (not "undefined variables") when they
    /// appear in expressions like `@intrinsic("size_of", T)`.
    pub generic_type_params: HashSet<String>,

    /// Const generic parameters in scope for the current function.
    ///
    /// When compiling generic functions/impls like `fn foo<const N: Int>()` or
    /// `implement<const SIZE: Int> StackAllocator<SIZE>`, the const parameters
    /// like N and SIZE are added here. This allows `compile_simple_path()` to recognize
    /// const generic parameters as valid identifiers when they appear in expressions.
    ///
    /// Note: At VBC level, const generics are compile-time known values, but we emit
    /// them as runtime values via GetConst since they're resolved during monomorphization.
    pub const_generic_params: HashSet<String>,

    /// Newtype type names (single-field wrapper types like `type FileDesc is (Int)`).
    ///
    /// Used to optimize field access: `fd.0` on a newtype emits `Mov` instead of
    /// `GetF`, since the value IS the single field (no heap indirection).
    pub newtype_names: HashSet<String>,

    /// Maps newtype name to its inner type name (e.g., "Meters" -> "Float").
    /// Used to propagate float tracking through newtype `.0` access.
    pub newtype_inner_type: HashMap<String, String>,

    /// Type names defined in user code (not stdlib).
    /// Used to disambiguate bare variant constructors when stdlib has a variant
    /// with the same name (e.g., user's `Disconnected` vs stdlib's `TryRecvError.Disconnected`).
    pub user_defined_types: HashSet<String>,

    /// Variables that hold byte arrays (contiguous byte buffers).
    ///
    /// When a variable is declared as `let buf: [Byte; N] = uninit()` or similar,
    /// it's marked as a byte array variable. This affects how `&mut buf[idx] as *mut Byte`
    /// is compiled - we emit `ByteArrayElementAddr` instead of `GetE + Ref` to get
    /// the actual memory address of the element rather than its value.
    pub byte_array_vars: HashSet<String>,

    /// Variables that hold typed arrays with their element sizes.
    ///
    /// Maps variable name to element size in bytes. For example:
    /// - `let arr: [UInt64; 4]` -> ("arr", 8)
    /// - `let arr: [UInt32; 10]` -> ("arr", 4)
    /// - `let arr: [UInt16; 100]` -> ("arr", 2)
    ///
    /// This is used for `TypedArrayElementAddr` to compute correct element offsets.
    /// Byte arrays (element size 1) are tracked separately in `byte_array_vars`.
    pub typed_array_vars: std::collections::HashMap<String, usize>,

    /// Depth counter for nested try/recover blocks.
    ///
    /// When > 0, the `?` operator emits `Throw` instead of `Ret` so that
    /// the error is caught by the enclosing try/recover handler rather than
    /// returning from the function.
    pub try_recover_depth: u32,

    /// Required contexts from the current function's `using [...]` clause.
    ///
    /// When a function declares `using [Logger, Database]`, these context names
    /// are stored here. When compiling method calls like `Logger.log(msg)`, we
    /// check if the receiver name is in this set. If so, we emit a `CtxGet`
    /// instruction to retrieve the context value from the context stack before
    /// calling the method.
    ///
    /// This enables the context system to work correctly: functions that require
    /// contexts can access them via method calls on the context type name.
    pub required_contexts: HashSet<String>,
    /// Context alias map: alias → context type name (e.g., "db" → "Database").
    /// Populated from `using [db: Database]` or `using [Database as db]`.
    pub context_aliases: HashMap<String, String>,

    /// Cache for Active pattern results to avoid double-calling pattern functions.
    ///
    /// When a partial Active pattern (returning `Maybe<T>`) is used in a match arm,
    /// the pattern function is called during `compile_pattern_test()` to check if
    /// it matches. The result (`Maybe<T>` value) is cached here so that during
    /// `compile_pattern_bind()`, we can extract the value without calling the
    /// pattern function again.
    ///
    /// Key: (scrutinee_register, pattern_name)
    /// Value: register containing the Maybe<T> result
    ///
    /// This cache is cleared at the end of each match arm to prevent stale entries.
    pub active_pattern_cache: HashMap<(Reg, String), Reg>,

    /// Thread-local static variables.
    ///
    /// Maps variable names declared with `@thread_local static mut VAR: T = init;`
    /// to their TLS slot index. When reading, emits `TlsGet { slot }`.
    /// When writing, emits `TlsSet { slot, val }`.
    pub thread_local_vars: HashMap<String, u16>,

    /// Next available TLS slot for `@thread_local` statics.
    pub next_tls_slot: u16,
}

/// Basic type kind for variable type tracking.
///
/// Used during codegen to select appropriate int/float/bool instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarTypeKind {
    /// Integer type (i64 at runtime).
    Int,
    /// Float type (f64 at runtime).
    Float,
    /// Boolean type.
    Bool,
    /// Byte type (u8 at runtime, stored as i64 0-255).
    Byte,
    /// Character type.
    Char,
    /// String/text type.
    Text,
    /// Unit type.
    Unit,
    /// Signed 32-bit integer type (stored as i64, methods use i32 semantics).
    Int32,
    /// Unsigned 64-bit integer type (stored as i64, methods use u64 semantics).
    UInt64,
    /// Unknown or untracked type.
    Unknown,
}

/// Information about a loop for break/continue.
#[derive(Debug, Clone)]
pub struct LoopContext {
    /// Label for loop start (for continue).
    pub continue_label: String,

    /// Label for loop end (for break).
    pub break_label: String,

    /// Optional loop label from source (for labeled break/continue).
    pub source_label: Option<String>,

    /// Register for break value (if any).
    pub break_value_reg: Option<Reg>,

    /// Scope level at loop entry.
    pub scope_level: usize,
}

/// Information about a deferred expression.
#[derive(Debug, Clone)]
pub struct DeferInfo {
    /// The instructions to execute on scope exit.
    pub instructions: Vec<Instruction>,

    /// Whether this is errdefer (only on error path).
    pub is_errdefer: bool,
}

/// Saved context state for nested function compilation (closures, generators).
///
/// When compiling a closure or generator inside a function, we call `begin_function()`
/// which clears labels, forward_jumps, loop_stack, defer_stack, and variable_type_names.
/// This struct saves these values so they can be restored after the nested function
/// is compiled, allowing the outer function's loops, jumps, and type tracking to
/// continue working.
#[derive(Debug, Clone)]
pub struct ClosureCompilationContext {
    /// Saved label counter.
    pub label_counter: u32,
    /// Saved labels map.
    pub labels: HashMap<String, usize>,
    /// Saved forward jumps.
    pub forward_jumps: HashMap<String, Vec<usize>>,
    /// Saved loop stack.
    pub loop_stack: Vec<LoopContext>,
    /// Saved defer stack.
    pub defer_stack: Vec<Vec<DeferInfo>>,
    /// Saved variable type names (critical for method resolution).
    pub variable_type_names: HashMap<String, String>,
}

/// Entry in the constant pool.
#[derive(Debug, Clone)]
pub enum ConstantEntry {
    /// Integer constant.
    Int(i64),
    /// Float constant.
    Float(f64),
    /// String constant (index into string table).
    String(u32),
    /// Byte array constant (index into bytes table).
    Bytes(u32),
    /// Type constant.
    Type(TypeRef),
}

/// Information about a function.
#[derive(Debug, Clone, Default)]
pub struct FunctionInfo {
    /// Function ID in module.
    pub id: FunctionId,
    /// Parameter count.
    pub param_count: usize,
    /// Parameter names.
    pub param_names: Vec<String>,
    /// Parameter type names (for DI auto-resolution in inject expressions).
    /// Parallel to param_names: param_type_names[i] is the type of param_names[i].
    pub param_type_names: Vec<String>,
    /// Whether function is async.
    pub is_async: bool,
    /// Whether function is a generator (fn*). Generator functions emit Yield opcodes
    /// to suspend execution and are invoked via GenCreate/GenNext/GenHasNext opcodes.
    pub is_generator: bool,
    /// Required contexts.
    pub contexts: Vec<String>,
    /// Return type.
    pub return_type: Option<TypeRef>,
    /// Yield type (for generators).
    pub yield_type: Option<TypeRef>,
    /// Intrinsic name if this function is declared with @intrinsic("name").
    ///
    /// When set, calls to this function will be compiled using the intrinsic
    /// codegen path instead of emitting a regular Call instruction. The name
    /// is looked up in `INTRINSIC_REGISTRY` to get the `CodegenStrategy`.
    ///
    /// This enables industrial-grade intrinsic resolution where:
    /// 1. Intrinsic identity is established at declaration time via @intrinsic
    /// 2. The codegen uses this stored name rather than call-site name matching
    /// 3. Imports and aliases work correctly for intrinsic functions
    pub intrinsic_name: Option<String>,
    /// If this function is a variant constructor, stores its tag.
    /// The tag is the variant's index in the type declaration order (0, 1, 2, ...).
    /// When present, calls emit MakeVariant instead of Call.
    pub variant_tag: Option<u32>,
    /// If this function is a variant constructor, stores the parent type name.
    /// For a variant `Som` of type `Opt<T>`, this would be `Some("Opt")`.
    /// Used to correctly determine the receiver type for method calls on variant values.
    pub parent_type_name: Option<String>,
    /// If this function is a variant constructor, stores the type names of its payload fields.
    /// For a variant like `V6(Ipv6Addr)`, this would be `Some(vec!["Ipv6Addr"])`.
    /// Used by pattern matching to track types of extracted variables.
    pub variant_payload_types: Option<Vec<String>>,
    /// Whether this function is an active pattern that returns Maybe<T>.
    /// Partial patterns require unwrapping Some(v) to get bindings.
    pub is_partial_pattern: bool,
    /// Whether the first parameter takes self by mutable reference (&mut self).
    /// When true, method calls must create a CBGR reference to the receiver
    /// and pass that reference (not the value) as the first argument.
    /// This enables `*self = value` inside the method to write back to the caller's variable.
    pub takes_self_mut_ref: bool,
    /// Base type name of the return type (e.g., "Result", "Maybe", "List").
    ///
    /// Used for type tracking when calling functions that return wrapper types.
    /// For `fn foo() -> Result<T, E>`, this would be `Some("Result")`.
    /// Enables correct method dispatch on the return value.
    pub return_type_name: Option<String>,
    /// Inner type parameters of the return type.
    ///
    /// For `fn foo() -> Maybe<Char>`, this would be `Some(vec!["Char"])`.
    /// For `fn bar() -> Result<Int, Text>`, this would be `Some(vec!["Int", "Text"])`.
    /// Used for pattern matching to infer types of extracted values (e.g., `c` in `Some(c)`).
    pub return_type_inner: Option<Vec<String>>,
}

/// Statistics collected during codegen.
#[derive(Debug, Clone, Default)]
pub struct CodegenStats {
    /// Number of functions compiled.
    pub functions_compiled: usize,
    /// Number of instructions generated.
    pub instructions_generated: usize,
    /// Number of expressions compiled.
    pub expressions_compiled: usize,
    /// Number of statements compiled.
    pub statements_compiled: usize,
    /// Number of constants created.
    pub constants_created: usize,
    /// Number of labels generated.
    pub labels_generated: usize,
    /// Number of Tier 0 references (runtime checked).
    pub tier0_refs: usize,
    /// Number of Tier 1 references (compiler proven safe).
    pub tier1_refs: usize,
    /// Number of Tier 2 references (unsafe).
    pub tier2_refs: usize,
    /// Number of tier fallbacks (Tier 1/2 -> Tier 0 for safety).
    ///
    /// Tracks cases where a higher tier was requested but couldn't be
    /// verified safe, requiring fallback to runtime-checked Tier 0.
    /// High values here may indicate escape analysis gaps or unsafe
    /// code patterns that need attention.
    pub tier_fallbacks: usize,
    /// Number of capability checks emitted.
    pub capability_checks: usize,
    /// Number of statements filtered out by @cfg.
    ///
    /// Tracks statements that were skipped due to non-matching @cfg
    /// attributes. This helps verify that platform-specific code is
    /// being correctly filtered for the target platform.
    pub cfg_filtered_stmts: usize,
}

// ==================== CBGR Tier Context ====================

/// Identifier for expressions (used as key for tier decisions).
///
/// In the full integration, this would come from the typed AST.
/// For now, we use a simple u64 that can be derived from span or expression ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(pub u64);

/// CBGR tier context for code generation.
///
/// Holds tier decisions from escape analysis that determine how
/// references should be compiled (Tier 0 with checks, Tier 1 direct, etc.).
///
/// Also preserves the Tier0Reason for expressions that remain at Tier 0,
/// enabling better diagnostics and error messages.
#[derive(Debug, Clone, Default)]
pub struct TierContext {
    /// Tier decisions from escape analysis.
    /// Maps expression IDs to their determined tiers.
    decisions: Map<ExprId, CbgrTier>,

    /// Tier0 reasons for expressions that couldn't be promoted.
    /// Only populated for expressions at Tier 0.
    tier0_reasons: Map<ExprId, Tier0Reason>,

    /// Default tier when no decision is available.
    /// Tier0 is conservative (always safe).
    pub default_tier: CbgrTier,

    /// Whether tier context is enabled.
    /// When disabled, all references use Tier0.
    pub enabled: bool,

    /// Whether we're currently in an unsafe block.
    /// When true, Tier 2 references can be used.
    pub in_unsafe: bool,
}

impl TierContext {
    /// Create a new tier context with defaults.
    pub fn new() -> Self {
        Self {
            decisions: Map::new(),
            tier0_reasons: Map::new(),
            default_tier: CbgrTier::Tier0,
            enabled: false,
            in_unsafe: false,
        }
    }

    /// Create an enabled tier context with decisions.
    pub fn with_decisions(decisions: Map<ExprId, CbgrTier>) -> Self {
        Self {
            decisions,
            tier0_reasons: Map::new(),
            default_tier: CbgrTier::Tier0,
            enabled: true,
            in_unsafe: false,
        }
    }

    /// Create an enabled tier context with decisions and reasons.
    pub fn with_decisions_and_reasons(
        decisions: Map<ExprId, CbgrTier>,
        tier0_reasons: Map<ExprId, Tier0Reason>,
    ) -> Self {
        Self {
            decisions,
            tier0_reasons,
            default_tier: CbgrTier::Tier0,
            enabled: true,
            in_unsafe: false,
        }
    }

    /// Get tier for an expression.
    ///
    /// Returns the tier from escape analysis if available,
    /// otherwise returns the default tier.
    pub fn get_tier(&self, expr_id: ExprId) -> CbgrTier {
        if !self.enabled {
            return CbgrTier::Tier0;
        }
        self.decisions.get(&expr_id).copied().unwrap_or(self.default_tier)
    }

    /// Get tier for an expression with span information.
    ///
    /// Converts span (start, end) to ExprId for lookup.
    pub fn get_tier_for_span(&self, start: u32, end: u32) -> CbgrTier {
        let expr_id = ExprId(((start as u64) << 32) | (end as u64));
        self.get_tier(expr_id)
    }

    /// Set tier decision for an expression.
    pub fn set_tier(&mut self, expr_id: ExprId, tier: CbgrTier) {
        self.decisions.insert(expr_id, tier);
    }

    /// Set tier decision with reason for an expression.
    ///
    /// The reason is stored only for Tier 0 expressions to enable better
    /// diagnostics when explaining why a reference couldn't be promoted.
    pub fn set_tier_with_reason(&mut self, expr_id: ExprId, tier: CbgrTier, reason: Option<Tier0Reason>) {
        self.decisions.insert(expr_id, tier);
        if tier == CbgrTier::Tier0
            && let Some(r) = reason {
                self.tier0_reasons.insert(expr_id, r);
            }
    }

    /// Get the Tier0Reason for an expression, if available.
    ///
    /// Returns `Some(reason)` if the expression is at Tier 0 and a reason
    /// was recorded, `None` otherwise.
    pub fn get_tier0_reason(&self, expr_id: ExprId) -> Option<Tier0Reason> {
        self.tier0_reasons.get(&expr_id).copied()
    }

    /// Get a diagnostic message explaining why an expression is at Tier 0.
    ///
    /// Returns a human-readable string suitable for error messages and
    /// compiler diagnostics.
    pub fn get_tier0_diagnostic(&self, expr_id: ExprId) -> String {
        if let Some(reason) = self.get_tier0_reason(expr_id) {
            format!("Reference requires runtime validation: {}", reason.description())
        } else if self.get_tier(expr_id) == CbgrTier::Tier0 {
            "Reference requires runtime validation (reason not analyzed)".to_string()
        } else {
            "Reference has been promoted to zero-overhead tier".to_string()
        }
    }

    /// Get dereference codegen strategy for a tier.
    pub fn get_deref_strategy(&self, expr_id: ExprId) -> DereferenceCodegen {
        DereferenceCodegen::for_tier(self.get_tier(expr_id))
    }

    /// Check if any tier decisions are available.
    pub fn has_decisions(&self) -> bool {
        !self.decisions.is_empty()
    }

    /// Get number of decisions.
    pub fn decision_count(&self) -> usize {
        self.decisions.len()
    }

    // ==================== Unsafe Block Management (Phase 5.4) ====================

    /// Enter an unsafe block.
    ///
    /// When inside an unsafe block, Tier 2 references are allowed without
    /// requiring explicit `&unsafe` syntax. This matches Verum's unsafe semantics
    /// where unsafe blocks allow raw memory operations.
    ///
    /// Returns the previous unsafe state for restoration.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // In codegen for unsafe block:
    /// let prev_unsafe = ctx.tier_context.enter_unsafe();
    /// // ... compile unsafe block contents ...
    /// ctx.tier_context.exit_unsafe(prev_unsafe);
    /// ```
    #[must_use]
    pub fn enter_unsafe(&mut self) -> bool {
        let was_unsafe = self.in_unsafe;
        self.in_unsafe = true;
        was_unsafe
    }

    /// Exit an unsafe block, restoring previous state.
    ///
    /// Pass the value returned from `enter_unsafe()` to properly
    /// handle nested unsafe blocks.
    pub fn exit_unsafe(&mut self, prev_state: bool) {
        self.in_unsafe = prev_state;
    }

    /// Check if currently inside an unsafe block.
    pub fn is_unsafe(&self) -> bool {
        self.in_unsafe
    }

    /// Get the effective tier for a reference, considering unsafe context.
    ///
    /// When inside an unsafe block, this can promote references to Tier 2
    /// if the caller requests it. Outside unsafe, Tier 2 is only allowed
    /// with explicit `&unsafe` syntax.
    ///
    /// # Arguments
    ///
    /// * `expr_id` - The expression ID to look up
    /// * `want_tier2` - Whether Tier 2 is explicitly requested (e.g., `&unsafe T`)
    ///
    /// # Returns
    ///
    /// The effective tier, potentially promoted to Tier 2 if in unsafe context.
    pub fn get_effective_tier(&self, expr_id: ExprId, want_tier2: bool) -> CbgrTier {
        let base_tier = self.get_tier(expr_id);

        // Tier 2 is allowed if:
        // 1. Explicitly requested via &unsafe syntax, OR
        // 2. We're inside an unsafe block and the reference allows it
        if want_tier2 {
            // Explicit &unsafe always results in Tier 2
            CbgrTier::Tier2
        } else if self.in_unsafe {
            // Inside unsafe, allow promotion to Tier 2 if analysis doesn't require Tier 0
            match base_tier {
                CbgrTier::Tier0 => CbgrTier::Tier0, // Safety-critical, keep Tier 0
                CbgrTier::Tier1 => CbgrTier::Tier2, // Promote to Tier 2 in unsafe
                CbgrTier::Tier2 => CbgrTier::Tier2, // Already Tier 2
            }
        } else {
            // Outside unsafe, use the analyzed tier
            base_tier
        }
    }

    /// Check if a tier is allowed in the current context.
    ///
    /// Returns true if the requested tier is safe to use.
    /// Tier 2 requires either unsafe context or explicit `&unsafe`.
    pub fn is_tier_allowed(&self, tier: CbgrTier, is_explicit: bool) -> bool {
        match tier {
            CbgrTier::Tier0 | CbgrTier::Tier1 => true,
            CbgrTier::Tier2 => self.in_unsafe || is_explicit,
        }
    }

    /// Create from TierAnalysisResult (bridge to verum_cbgr).
    ///
    /// Converts RefId-keyed tier decisions from escape analysis
    /// to ExprId-keyed decisions for codegen. The RefId is used
    /// directly as the ExprId since they're both u64 identifiers.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use verum_cbgr::tier_analysis::{TierAnalyzer, analyze_tiers};
    /// use verum_vbc::codegen::TierContext;
    ///
    /// let result = analyze_tiers(&cfg);
    /// let tier_context = TierContext::from_analysis_result(&result);
    /// codegen.set_tier_context(tier_context);
    /// ```
    ///
    /// # ExprId/RefId Unification
    ///
    /// This method handles the ExprId/RefId mismatch between the CBGR tier analyzer
    /// (which uses RefId) and VBC codegen (which uses span-based ExprId):
    /// - If span information is available in TierAnalysisResult, we use span-based ExprId
    ///   (encoded as `(start << 32) | end`) to match VBC codegen's span-based lookup.
    /// - If no span info is available, we fall back to using RefId directly as ExprId.
    ///
    /// The span-based approach is preferred because VBC codegen creates ExprId from
    /// expression spans, and this ensures tier decisions are correctly looked up.
    pub fn from_analysis_result(result: &verum_cbgr::tier_analysis::TierAnalysisResult) -> Self {
        use verum_cbgr::tier_types::CbgrTier as AnalysisTier;

        let mut decisions = Map::new();

        for (ref_id, tier) in &result.decisions {
            // Convert from verum_cbgr::tier_types::CbgrTier to verum_vbc::types::CbgrTier
            let vbc_tier = match tier.to_vbc_tier() {
                AnalysisTier::Tier0 => CbgrTier::Tier0,
                AnalysisTier::Tier1 => CbgrTier::Tier1,
                AnalysisTier::Tier2 => CbgrTier::Tier2,
            };

            // Prefer span-based ExprId when available (matches VBC codegen's lookup)
            // Span-based ExprId preferred: matches VBC codegen's span-based lookup scheme
            let expr_id = if let Some((start, end)) = result.get_span(*ref_id) {
                // Create span-based ExprId: (start << 32) | end
                // This matches how VBC codegen creates ExprId in expressions.rs
                ExprId(((start as u64) << 32) | (end as u64))
            } else {
                // Fallback: use RefId directly when no span available
                ExprId(ref_id.0)
            };

            decisions.insert(expr_id, vbc_tier);
        }

        // Extract Tier0 reasons from the analysis result
        let mut tier0_reasons = Map::new();
        for (ref_id, tier) in &result.decisions {
            if let Some(reason) = tier.reason() {
                let expr_id = if let Some((start, end)) = result.get_span(*ref_id) {
                    ExprId(((start as u64) << 32) | (end as u64))
                } else {
                    ExprId(ref_id.0)
                };
                tier0_reasons.insert(expr_id, *reason);
            }
        }

        Self {
            decisions,
            tier0_reasons,
            default_tier: CbgrTier::Tier0,
            enabled: true,
            in_unsafe: false, // Set by codegen when entering unsafe blocks
        }
    }
}

impl Default for CodegenContext {
    fn default() -> Self {
        Self::new()
    }
}

impl CodegenContext {
    /// Creates a new codegen context.
    pub fn new() -> Self {
        Self {
            registers: RegisterAllocator::new(),
            instructions: Vec::new(),
            instruction_spans: Vec::new(),
            current_span: verum_common::Span::default(),
            label_counter: 0,
            labels: HashMap::new(),
            forward_jumps: HashMap::new(),
            loop_stack: Vec::new(),
            defer_stack: vec![Vec::new()], // Root scope
            current_function: None,
            in_function: false,
            return_type: None,
            current_return_type_name: None,
            constants: Vec::new(),
            strings: Vec::new(),
            string_intern: HashMap::new(),
            bytes: Vec::new(),
            bytes_intern: HashMap::new(),
            functions: HashMap::new(),
            prefer_existing_functions: false,
            stats: CodegenStats::default(),
            tier_context: TierContext::new(),
            suspend_point_count: 0,
            variable_types: HashMap::new(),
            constant_types: HashMap::new(),
            variable_type_names: HashMap::new(),
            last_function_variable_types: HashMap::new(),
            match_scrutinee_type: None,
            raw_pointer_regs: HashSet::new(),
            generic_type_params: HashSet::new(),
            const_generic_params: HashSet::new(),
            newtype_names: HashSet::new(),
            newtype_inner_type: HashMap::new(),
            user_defined_types: HashSet::new(),
            byte_array_vars: HashSet::new(),
            typed_array_vars: HashMap::new(),
            try_recover_depth: 0,
            required_contexts: HashSet::new(),
            context_aliases: HashMap::new(),
            active_pattern_cache: HashMap::new(),
            thread_local_vars: HashMap::new(),
            next_tls_slot: 0,
        }
    }

    /// Creates a new codegen context with tier analysis results.
    pub fn with_tier_context(tier_context: TierContext) -> Self {
        Self {
            suspend_point_count: 0,
            tier_context,
            raw_pointer_regs: HashSet::new(),
            generic_type_params: HashSet::new(),
            const_generic_params: HashSet::new(),
            typed_array_vars: HashMap::new(),
            byte_array_vars: HashSet::new(),
            required_contexts: HashSet::new(),
            context_aliases: HashMap::new(),
            active_pattern_cache: HashMap::new(),
            thread_local_vars: HashMap::new(),
            next_tls_slot: 0,
            ..Self::new()
        }
    }

    /// Allocate a TLS slot for a `@thread_local` static variable.
    /// Returns the slot index assigned to this variable.
    pub fn register_thread_local(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.thread_local_vars.get(name) {
            return slot;
        }
        let slot = self.next_tls_slot;
        self.next_tls_slot += 1;
        self.thread_local_vars.insert(name.to_string(), slot);
        slot
    }

    /// Check if a name refers to a `@thread_local` static variable.
    pub fn is_thread_local(&self, name: &str) -> Option<u16> {
        self.thread_local_vars.get(name).copied()
    }

    /// Sets the tier context from escape analysis results.
    pub fn set_tier_context(&mut self, tier_context: TierContext) {
        self.tier_context = tier_context;
    }

    /// Gets the tier for an expression.
    ///
    /// Returns the tier from escape analysis if available,
    /// otherwise returns the default (Tier0 - managed).
    pub fn get_tier_for_expr(&self, expr_id: ExprId) -> CbgrTier {
        self.tier_context.get_tier(expr_id)
    }

    /// Gets the tier for an expression identified by span.
    pub fn get_tier_for_span(&self, start: u32, end: u32) -> CbgrTier {
        self.tier_context.get_tier_for_span(start, end)
    }

    /// Records a reference operation in stats.
    pub fn record_ref_tier(&mut self, tier: CbgrTier) {
        match tier {
            CbgrTier::Tier0 => self.stats.tier0_refs += 1,
            CbgrTier::Tier1 => self.stats.tier1_refs += 1,
            CbgrTier::Tier2 => self.stats.tier2_refs += 1,
        }
    }

    // ==================== Raw Pointer Register Tracking ====================

    /// Marks a register as containing a raw FFI pointer.
    ///
    /// Registers containing raw pointers must use DerefRaw/DerefMutRaw
    /// instructions which bypass CBGR validation.
    pub fn mark_raw_pointer(&mut self, reg: Reg) {
        self.raw_pointer_regs.insert(reg);
    }

    /// Checks if a register contains a raw FFI pointer.
    ///
    /// If true, dereference operations should use DerefRaw/DerefMutRaw
    /// instead of the standard Deref/DerefMut which expect CBGR headers.
    pub fn is_raw_pointer(&self, reg: Reg) -> bool {
        self.raw_pointer_regs.contains(&reg)
    }

    /// Clears raw pointer tracking for a register.
    ///
    /// Called when a register is reused for a non-pointer value.
    pub fn clear_raw_pointer(&mut self, reg: Reg) {
        self.raw_pointer_regs.remove(&reg);
    }

    /// Clears all raw pointer register tracking.
    ///
    /// Called when starting a new function to reset state.
    pub fn clear_all_raw_pointers(&mut self) {
        self.raw_pointer_regs.clear();
    }

    // ==================== Active Pattern Result Cache ====================

    /// Caches an Active pattern result for later use in pattern binding.
    ///
    /// When a partial Active pattern (returning `Maybe<T>`) is tested, we cache
    /// the result so that `compile_pattern_bind()` can extract the value without
    /// calling the pattern function again.
    ///
    /// The key is (scrutinee_register, pattern_name) to handle multiple patterns
    /// in the same match expression.
    pub fn cache_active_pattern_result(&mut self, scrutinee: Reg, pattern_name: &str, result_reg: Reg) {
        self.active_pattern_cache.insert((scrutinee, pattern_name.to_string()), result_reg);
    }

    /// Retrieves a cached Active pattern result.
    ///
    /// Returns the register containing the `Maybe<T>` result from the pattern
    /// function call made during `compile_pattern_test()`.
    ///
    /// Returns `None` if no cached result exists (shouldn't happen in normal flow).
    pub fn get_cached_active_pattern_result(&self, scrutinee: Reg, pattern_name: &str) -> Option<Reg> {
        self.active_pattern_cache.get(&(scrutinee, pattern_name.to_string())).copied()
    }

    /// Clears the active pattern cache for a specific scrutinee.
    ///
    /// Called at the end of each match arm to prevent stale entries from
    /// being used in subsequent arms.
    pub fn clear_active_pattern_cache_for(&mut self, scrutinee: Reg) {
        self.active_pattern_cache.retain(|(s, _), _| *s != scrutinee);
    }

    /// Clears all active pattern cache entries.
    ///
    /// Called when starting a new match expression or function.
    pub fn clear_active_pattern_cache(&mut self) {
        self.active_pattern_cache.clear();
    }

    // ==================== Byte Array Variable Tracking ====================

    /// Marks a variable as holding a byte array.
    ///
    /// Variables marked as byte arrays need special handling when their elements
    /// are referenced with `&mut arr[idx] as *mut Byte` - we emit `ByteArrayElementAddr`
    /// instead of `GetE + Ref` to get the actual memory address.
    pub fn mark_byte_array_var(&mut self, name: &str) {
        self.byte_array_vars.insert(name.to_string());
    }

    /// Checks if a variable is a byte array.
    ///
    /// If true, `&mut var[idx] as *mut T` patterns should use `ByteArrayElementAddr`
    /// to compute the element address instead of fetching its value with `GetE`.
    pub fn is_byte_array_var(&self, name: &str) -> bool {
        self.byte_array_vars.contains(name)
    }

    /// Clears byte array variable tracking.
    ///
    /// Called when starting a new function to reset state.
    pub fn clear_byte_array_vars(&mut self) {
        self.byte_array_vars.clear();
    }

    /// Marks a variable as holding a typed array with the specified element size.
    ///
    /// Variables marked as typed arrays need special handling when their elements
    /// are referenced with `&mut arr[idx] as *mut T` - we emit `TypedArrayElementAddr`
    /// with the element size to compute the correct memory address.
    pub fn mark_typed_array_var(&mut self, name: &str, elem_size: usize) {
        self.typed_array_vars.insert(name.to_string(), elem_size);
    }

    /// Gets the element size of a typed array variable.
    ///
    /// Returns `Some(size)` if the variable is a typed array, `None` otherwise.
    /// For byte arrays (tracked separately), returns `Some(1)`.
    pub fn get_typed_array_elem_size(&self, name: &str) -> Option<usize> {
        if self.byte_array_vars.contains(name) {
            Some(1)
        } else {
            self.typed_array_vars.get(name).copied()
        }
    }

    /// Clears typed array variable tracking.
    ///
    /// Called when starting a new function to reset state.
    pub fn clear_typed_array_vars(&mut self) {
        self.typed_array_vars.clear();
    }

    // ==================== Label Management ====================

    /// Generates a unique label name.
    pub fn new_label(&mut self, prefix: &str) -> String {
        let label = format!("{}_{}", prefix, self.label_counter);
        self.label_counter += 1;
        self.stats.labels_generated += 1;
        label
    }

    /// Defines a label at the current instruction position.
    pub fn define_label(&mut self, name: &str) {
        let pos = self.instructions.len();
        self.labels.insert(name.to_string(), pos);

        // Patch any forward jumps to this label
        if let Some(indices) = self.forward_jumps.remove(name) {
            for idx in indices {
                self.patch_jump(idx, pos as i32);
            }
        }
    }

    /// Records a forward jump to be patched later.
    pub fn record_forward_jump(&mut self, label: &str) {
        let idx = self.instructions.len();
        self.forward_jumps
            .entry(label.to_string())
            .or_default()
            .push(idx);
    }

    /// Calculates relative offset from current position to a label.
    pub fn label_offset(&self, label: &str) -> Option<i32> {
        self.labels.get(label).map(|&target| {
            let current = self.instructions.len() as i32;
            let target = target as i32;
            target - current
        })
    }

    /// Patches a jump instruction at the given index.
    fn patch_jump(&mut self, idx: usize, target: i32) {
        if idx >= self.instructions.len() {
            return;
        }

        let offset = target - idx as i32;

        // Replace the instruction with corrected offset
        match &mut self.instructions[idx] {
            Instruction::Jmp { offset: o } => *o = offset,
            Instruction::JmpIf { cond: _, offset: o } => *o = offset,
            Instruction::JmpNot { cond: _, offset: o } => *o = offset,
            Instruction::JmpCmp { op: _, a: _, b: _, offset: o } => *o = offset,
            Instruction::CtxProvide { body_offset, .. } => *body_offset = offset,
            Instruction::TryBegin { handler_offset } => *handler_offset = offset,
            _ => {}
        }
    }

    // ==================== Instruction Emission ====================

    /// Emits an instruction, recording the current source span for debug info.
    pub fn emit(&mut self, instr: Instruction) {
        self.instructions.push(instr);
        self.instruction_spans.push(self.current_span);
        self.stats.instructions_generated += 1;
    }

    /// Set the current source span (called before emitting instructions from an expression).
    pub fn set_current_span(&mut self, span: verum_common::Span) {
        self.current_span = span;
    }

    /// Emits a jump instruction with a placeholder offset.
    ///
    /// The offset will be patched when the target label is defined.
    pub fn emit_forward_jump(&mut self, label: &str, make_instr: impl FnOnce(i32) -> Instruction) {
        self.record_forward_jump(label);
        self.emit(make_instr(0)); // Placeholder
    }

    /// Emits a jump to a known label (backward jump).
    pub fn emit_backward_jump(&mut self, label: &str, make_instr: impl FnOnce(i32) -> Instruction) -> CodegenResult<()> {
        let offset = self.label_offset(label)
            .ok_or_else(|| CodegenError::internal(format!("undefined label: {}", label)))?;
        // Note: offset is instruction-level, fixup_jump_offsets converts to bytes
        self.emit(make_instr(offset));
        Ok(())
    }

    /// Emits a forward CtxProvide instruction with body offset to be patched.
    pub fn emit_forward_context_provide(&mut self, end_label: &str, ctx_type: u32, value: Reg) {
        self.record_forward_jump(end_label);
        self.emit(Instruction::CtxProvide {
            ctx_type,
            value,
            body_offset: 0, // Placeholder - will be patched
        });
    }

    /// Returns the current instruction index.
    pub fn current_pc(&self) -> usize {
        self.instructions.len()
    }

    // ==================== Loop Management ====================

    /// Enters a loop.
    pub fn enter_loop(&mut self, source_label: Option<String>, break_value_reg: Option<Reg>) -> LoopContext {
        let continue_label = self.new_label("loop_continue");
        let break_label = self.new_label("loop_break");
        let scope_level = self.registers.scope_level();

        let ctx = LoopContext {
            continue_label: continue_label.clone(),
            break_label: break_label.clone(),
            source_label,
            break_value_reg,
            scope_level,
        };

        self.loop_stack.push(ctx.clone());
        ctx
    }

    /// Exits the current loop.
    pub fn exit_loop(&mut self) -> Option<LoopContext> {
        self.loop_stack.pop()
    }

    /// Gets the current loop context.
    pub fn current_loop(&self) -> Option<&LoopContext> {
        self.loop_stack.last()
    }

    /// Finds a loop by label.
    pub fn find_loop(&self, label: Option<&str>) -> Option<&LoopContext> {
        match label {
            Some(lbl) => self.loop_stack.iter().rev().find(|ctx| {
                ctx.source_label.as_deref() == Some(lbl)
            }),
            None => self.loop_stack.last(),
        }
    }

    /// Checks if we're inside a loop.
    pub fn in_loop(&self) -> bool {
        !self.loop_stack.is_empty()
    }

    /// Pushes a new loop context (simplified API).
    pub fn push_loop(&mut self, source_label: String, break_label: String) {
        let continue_label = self.new_label("loop_continue");
        let scope_level = self.registers.scope_level();

        let ctx = LoopContext {
            continue_label,
            break_label,
            source_label: Some(source_label),
            break_value_reg: None,
            scope_level,
        };

        self.loop_stack.push(ctx);
    }

    /// Pops the current loop context.
    pub fn pop_loop(&mut self) -> Option<LoopContext> {
        self.loop_stack.pop()
    }

    /// Calculates backward offset from current position to a label.
    pub fn calculate_backward_offset(&self, label: &str) -> i32 {
        self.label_offset(label).unwrap_or(0)
    }

    // ==================== Defer Management ====================

    /// Pushes a new defer scope.
    pub fn push_defer_scope(&mut self) {
        self.defer_stack.push(Vec::new());
    }

    /// Pops a defer scope and returns deferred instructions.
    pub fn pop_defer_scope(&mut self, is_error_path: bool) -> Vec<Vec<Instruction>> {
        let mut result = Vec::new();

        if let Some(defers) = self.defer_stack.pop() {
            // Execute in LIFO order
            for defer in defers.into_iter().rev() {
                if !defer.is_errdefer || is_error_path {
                    result.push(defer.instructions);
                }
            }
        }

        result
    }

    /// Adds a defer to current scope.
    pub fn add_defer(&mut self, instructions: Vec<Instruction>, is_errdefer: bool) {
        if let Some(scope) = self.defer_stack.last_mut() {
            scope.push(DeferInfo {
                instructions,
                is_errdefer,
            });
        }
    }

    /// Gets all defers that need to run for scope exit.
    pub fn pending_defers(&self, is_error_path: bool) -> Vec<&Vec<Instruction>> {
        let mut result = Vec::new();

        if let Some(defers) = self.defer_stack.last() {
            for defer in defers.iter().rev() {
                if !defer.is_errdefer || is_error_path {
                    result.push(&defer.instructions);
                }
            }
        }

        result
    }

    // ==================== Scope Management ====================

    /// Enters a new scope.
    pub fn enter_scope(&mut self) {
        self.registers.enter_scope();
        self.push_defer_scope();
    }

    /// Exits the current scope.
    ///
    /// Returns variables that went out of scope (for drop calls).
    pub fn exit_scope(&mut self, is_error_path: bool) -> (Vec<(String, Reg)>, Vec<Vec<Instruction>>) {
        let defers = self.pop_defer_scope(is_error_path);
        let vars = self.registers.exit_scope();
        (vars, defers)
    }

    // ==================== Unsafe Context (Phase 5.4) ====================

    /// Enters unsafe context for Tier 2 reference promotion.
    ///
    /// Returns the previous unsafe state to support nested unsafe blocks.
    /// The returned value should be passed to `exit_unsafe()` to properly
    /// restore state after the unsafe block.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Compiling: unsafe { ... }
    /// let prev = ctx.enter_unsafe();
    /// compile_block_contents(block);
    /// ctx.exit_unsafe(prev);
    /// ```
    ///
    /// # Tier Promotion Behavior
    ///
    /// Inside an unsafe block:
    /// - Tier 0 stays Tier 0 (safety-critical references)
    /// - Tier 1 can be promoted to Tier 2 (skip CBGR validation)
    /// - Explicit `&unsafe` always uses Tier 2
    #[must_use]
    pub fn enter_unsafe(&mut self) -> bool {
        self.tier_context.enter_unsafe()
    }

    /// Exits unsafe context, restoring previous state.
    ///
    /// Pass the value returned from `enter_unsafe()` to correctly
    /// handle nested unsafe blocks.
    pub fn exit_unsafe(&mut self, prev_state: bool) {
        self.tier_context.exit_unsafe(prev_state);
    }

    /// Check if currently inside an unsafe block.
    pub fn is_unsafe(&self) -> bool {
        self.tier_context.is_unsafe()
    }

    /// Get the effective tier for a reference, considering unsafe context.
    ///
    /// Delegates to TierContext's get_effective_tier for full tier promotion
    /// logic including unsafe block handling.
    pub fn get_effective_tier(&self, expr_id: super::context::ExprId, want_tier2: bool) -> CbgrTier {
        self.tier_context.get_effective_tier(expr_id, want_tier2)
    }

    // ==================== Source Location ====================

    /// Returns the current source file name.
    pub fn current_file(&self) -> String {
        self.current_function
            .as_ref()
            .map(|f| f.split("::").next().unwrap_or("unknown"))
            .unwrap_or("unknown")
            .to_string()
    }

    /// Returns the current source line number.
    pub fn current_line(&self) -> u32 {
        // Placeholder - would be tracked during compilation
        0
    }

    /// Returns the current source column number.
    pub fn current_column(&self) -> u32 {
        // Placeholder - would be tracked during compilation
        0
    }

    // ==================== Variable Allocation ====================

    /// Allocates a register for a new named variable.
    pub fn alloc_var(&mut self, name: &str) -> CodegenResult<Reg> {
        self.registers.alloc_named(name)
    }

    // ==================== Constant Pool ====================

    /// Adds an integer constant and returns its ID.
    pub fn add_const_int(&mut self, value: i64) -> ConstId {
        // Check for existing
        for (i, c) in self.constants.iter().enumerate() {
            if let ConstantEntry::Int(v) = c
                && *v == value {
                    return ConstId(i as u32);
                }
        }

        let id = ConstId(self.constants.len() as u32);
        self.constants.push(ConstantEntry::Int(value));
        self.stats.constants_created += 1;
        id
    }

    /// Adds a float constant and returns its ID.
    pub fn add_const_float(&mut self, value: f64) -> ConstId {
        // Check for existing (careful with NaN)
        for (i, c) in self.constants.iter().enumerate() {
            if let ConstantEntry::Float(v) = c
                && v.to_bits() == value.to_bits() {
                    return ConstId(i as u32);
                }
        }

        let id = ConstId(self.constants.len() as u32);
        self.constants.push(ConstantEntry::Float(value));
        self.stats.constants_created += 1;
        id
    }

    /// Interns a string into the string table and returns its raw index.
    ///
    /// Used by the Eq protocol dispatch to encode type names in CmpG's protocol_id field.
    /// The returned index can be used with `StringId` in the interpreter to resolve the name.
    pub fn intern_string_raw(&mut self, value: &str) -> u32 {
        if let Some(&id) = self.string_intern.get(value) {
            id
        } else {
            let id = self.strings.len() as u32;
            self.strings.push(value.to_string());
            self.string_intern.insert(value.to_string(), id);
            id
        }
    }

    /// Adds a string constant and returns its ID.
    pub fn add_const_string(&mut self, value: &str) -> ConstId {
        // Intern the string
        let string_id = if let Some(&id) = self.string_intern.get(value) {
            id
        } else {
            let id = self.strings.len() as u32;
            self.strings.push(value.to_string());
            self.string_intern.insert(value.to_string(), id);
            id
        };

        // Check for existing string constant
        for (i, c) in self.constants.iter().enumerate() {
            if let ConstantEntry::String(sid) = c
                && *sid == string_id {
                    return ConstId(i as u32);
                }
        }

        let id = ConstId(self.constants.len() as u32);
        self.constants.push(ConstantEntry::String(string_id));
        self.stats.constants_created += 1;
        id
    }

    /// Adds a byte array constant and returns its ID.
    pub fn add_const_bytes(&mut self, value: Vec<u8>) -> ConstId {
        // Intern the byte array
        let bytes_id = if let Some(&id) = self.bytes_intern.get(&value) {
            id
        } else {
            let id = self.bytes.len() as u32;
            self.bytes.push(value.clone());
            self.bytes_intern.insert(value, id);
            id
        };

        // Check for existing bytes constant
        for (i, c) in self.constants.iter().enumerate() {
            if let ConstantEntry::Bytes(bid) = c
                && *bid == bytes_id
            {
                return ConstId(i as u32);
            }
        }

        let id = ConstId(self.constants.len() as u32);
        self.constants.push(ConstantEntry::Bytes(bytes_id));
        self.stats.constants_created += 1;
        id
    }

    // ==================== Variable Access ====================

    /// Looks up a variable's register info.
    pub fn lookup_var(&self, name: &str) -> Option<&RegisterInfo> {
        self.registers.lookup(name)
    }

    /// Looks up a variable's register info (mutable).
    pub fn lookup_var_mut(&mut self, name: &str) -> Option<&mut RegisterInfo> {
        self.registers.lookup_mut(name)
    }

    /// Gets the register for a variable.
    pub fn get_var_reg(&self, name: &str) -> CodegenResult<Reg> {
        self.registers.get_reg(name)
            .ok_or_else(|| CodegenError::undefined_variable(name))
    }

    /// Defines a new variable.
    pub fn define_var(&mut self, name: &str, is_mutable: bool) -> Reg {
        self.registers.alloc_local(name, is_mutable)
    }

    /// Allocates a temporary register.
    pub fn alloc_temp(&mut self) -> Reg {
        self.registers.alloc_temp()
    }

    /// Frees a temporary register.
    ///
    /// Also clears raw pointer tracking for the register to prevent stale
    /// FFI pointer marks from leaking to the next allocation of the same register.
    pub fn free_temp(&mut self, reg: Reg) {
        self.raw_pointer_regs.remove(&reg);
        self.registers.free_temp(reg);
    }

    // ==================== Function Management ====================

    /// Starts compiling a new function.
    ///
    /// Each parameter is a tuple of (name, is_mutable).
    pub fn begin_function(&mut self, name: &str, params: &[(String, bool)], return_type: Option<TypeRef>) {
        self.registers.reset();
        self.instructions.clear();
        self.labels.clear();
        self.forward_jumps.clear();
        self.loop_stack.clear();
        self.defer_stack.clear();
        self.defer_stack.push(Vec::new()); // Root scope

        // Clear function-scoped type/variable tracking to prevent cross-function leakage.
        // Without this, variable type annotations (e.g. UInt64 from a Duration method)
        // leak into subsequent functions, causing wrong method dispatch prefixes.
        self.variable_types.clear();
        // Snapshot variable types before clearing — playground uses this to display
        // accurate types (List<Int>, Map<Text, Bool>) in the sidebar.
        if !self.variable_type_names.is_empty() {
            self.last_function_variable_types = self.variable_type_names.clone();
        }
        self.variable_type_names.clear();
        self.generic_type_params.clear();
        self.const_generic_params.clear();
        self.byte_array_vars.clear();
        self.typed_array_vars.clear();
        self.active_pattern_cache.clear();
        self.raw_pointer_regs.clear();

        self.current_function = Some(name.to_string());
        self.in_function = true;
        self.return_type = return_type;
        self.suspend_point_count = 0; // Reset for generators

        // Allocate parameter registers
        self.registers.alloc_parameters(params);

        self.stats.functions_compiled += 1;
    }

    /// Finishes compiling the current function.
    ///
    /// Returns the generated instructions and register count.
    pub fn end_function(&mut self) -> (Vec<Instruction>, u16) {
        self.current_function = None;
        self.in_function = false;
        self.return_type = None;
        self.current_return_type_name = None;

        (
            std::mem::take(&mut self.instructions),
            self.registers.register_count(),
        )
    }

    /// Collects debug variable info from the register allocator.
    ///
    /// Returns (variable_name, register, is_parameter, arg_index) tuples
    /// for all named variables (locals + parameters).
    pub fn collect_debug_variables(&self) -> Vec<(String, u16, bool, u16)> {
        self.registers.collect_debug_variables()
    }

    /// Registers a function for lookup.
    pub fn register_function(&mut self, name: String, info: FunctionInfo) {
        if self.prefer_existing_functions {
            self.functions.entry(name).or_insert(info);
        } else {
            // Store alternative arities for arity-based disambiguation.
            // When a user function collides with a stdlib method of different arity,
            // we keep both so the call site can pick the right one.
            if let Some(existing) = self.functions.get(&name) {
                if existing.param_count != info.param_count {
                    let alt_key = format!("{}#{}", name, info.param_count);
                    self.functions.insert(alt_key, info);
                    return;
                }
            }
            self.functions.insert(name, info);
        }
    }

    /// Sets the intrinsic_name for an existing function.
    /// Returns true if the function was found and updated, false otherwise.
    pub fn set_function_intrinsic(&mut self, name: &str, intrinsic_name: String) -> bool {
        if let Some(info) = self.functions.get_mut(name) {
            info.intrinsic_name = Some(intrinsic_name);
            true
        } else {
            false
        }
    }

    /// Unregisters a function by name.
    ///
    /// Used when a name collision is detected during variant registration.
    /// Returns true if the function was found and removed.
    pub fn unregister_function(&mut self, name: &str) -> bool {
        self.functions.remove(name).is_some()
    }

    /// Looks up a function by name.
    pub fn lookup_function(&self, name: &str) -> Option<&FunctionInfo> {
        self.functions.get(name)
    }

    /// Looks up a function by name with arity disambiguation.
    /// When the primary lookup returns a function with wrong arity,
    /// checks for an arity-qualified alternative (name#arity).
    pub fn lookup_function_with_arity(&self, name: &str, arity: usize) -> Option<&FunctionInfo> {
        if let Some(info) = self.functions.get(name) {
            if info.param_count == arity {
                return Some(info);
            }
            // Primary has wrong arity — check for arity-qualified alternative
            let alt_key = format!("{}#{}", name, arity);
            if let Some(alt_info) = self.functions.get(&alt_key) {
                return Some(alt_info);
            }
            // Still return primary (caller will report arity error)
            return Some(info);
        }
        // Check arity-qualified key directly
        let alt_key = format!("{}#{}", name, arity);
        self.functions.get(&alt_key)
    }

    /// Search for a function whose name ends with the given suffix.
    /// Used to find qualified variant names (e.g., "Option.None") when
    /// the simple name is not registered due to collision.
    ///
    /// When multiple matches exist (e.g., "Ordering.Lt" and "GeneralCategory.Lt"),
    /// prefers the one whose parent_type_name matches the current function's return
    /// type. If no return type context is available, returns the match only if unique.
    pub fn find_function_by_suffix(&self, suffix: &str) -> Option<&FunctionInfo> {
        let mut matches: Vec<&FunctionInfo> = Vec::new();
        for (_name, info) in &self.functions {
            if _name.ends_with(suffix) {
                matches.push(info);
            }
        }

        if matches.len() == 1 {
            return Some(matches[0]);
        }

        if matches.len() > 1 {
            // Multiple matches — try to disambiguate using the current function's return type.
            // E.g., if we're in `fn cmp_val(...) -> Ordering` and looking for ".Lt",
            // prefer "Ordering.Lt" over "GeneralCategory.Lt".
            // Strip generic args (e.g., "Maybe<Int>" -> "Maybe") since variants are
            // registered under the base type name.
            if let Some(ref ret_type) = self.current_return_type_name {
                let base = ret_type.split('<').next().unwrap_or(ret_type.as_str());
                for info in &matches {
                    if info.parent_type_name.as_deref() == Some(base) {
                        return Some(info);
                    }
                }
            }
            // Also try the match_scrutinee_type for pattern matching context
            if let Some(ref scrutinee_type) = self.match_scrutinee_type {
                let base = scrutinee_type.split('<').next().unwrap_or(scrutinee_type.as_str());
                for info in &matches {
                    if info.parent_type_name.as_deref() == Some(base) {
                        return Some(info);
                    }
                }
            }
            // Prefer variants from user-defined types over stdlib types
            let user_matches: Vec<_> = matches.iter()
                .filter(|info| info.parent_type_name.as_ref()
                    .map(|p| self.user_defined_types.contains(p))
                    .unwrap_or(false))
                .collect();
            if user_matches.len() == 1 {
                return Some(user_matches[0]);
            }
            // Ambiguous with no context — return None to avoid nondeterminism
            return None;
        }

        None
    }

    /// Find a variant constructor by simple name and argument count.
    ///
    /// When a simple variant name (e.g., "Done") is in the collision set,
    /// this tries all qualified forms ("TypeName.Done") and picks the one
    /// whose param_count matches. Returns the variant tag if exactly one match.
    pub fn find_variant_by_suffix_and_args(&self, name: &str, arg_count: usize) -> Option<u32> {
        let suffix = format!(".{}", name);
        // Collect all matches with their parent type names for disambiguation.
        let mut matches: Vec<(u32, Option<String>)> = Vec::new();
        for (fn_name, fn_info) in &self.functions {
            if fn_name.ends_with(&suffix) && fn_info.param_count == arg_count
                && let Some(tag) = fn_info.variant_tag {
                    matches.push((tag, fn_info.parent_type_name.clone()));
                }
        }
        if matches.len() == 1 {
            return Some(matches[0].0);
        }
        if matches.len() > 1 {
            // Ambiguous: prefer user-defined types over stdlib types.
            let user_matches: Vec<u32> = matches.iter()
                .filter(|(_, parent)| parent.as_ref()
                    .map(|p| self.user_defined_types.contains(p))
                    .unwrap_or(false))
                .map(|(tag, _)| *tag)
                .collect();
            if user_matches.len() == 1 {
                return Some(user_matches[0]);
            }
        }
        None // Ambiguous or not found — fall through to hash
    }

    /// Find a variant tag by simple name and parent type.
    ///
    /// When a simple variant name (e.g., "Done") is in the collision set and
    /// we know the expected parent type (from match scrutinee), try the
    /// qualified form "TypeName.VariantName" directly.
    pub fn find_variant_by_type_and_name(&self, type_name: &str, variant_name: &str) -> Option<u32> {
        let qualified = format!("{}.{}", type_name, variant_name);
        self.functions.get(&qualified)
            .and_then(|info| info.variant_tag)
    }

    /// Find the parent type of a variant by searching "*.variant_name" entries
    /// filtered by param_count. Returns the parent_type_name if exactly one match.
    /// Used to resolve variant → parent type when simple name is collided.
    pub fn find_variant_parent_type_by_args(&self, name: &str, arg_count: usize) -> Option<String> {
        let suffix = format!(".{}", name);
        let mut parents = Vec::new();
        for (fn_name, fn_info) in &self.functions {
            if fn_name.ends_with(&suffix)
                && fn_info.variant_tag.is_some()
                && fn_info.param_count == arg_count
                && let Some(ref parent) = fn_info.parent_type_name
                    && !parents.contains(parent) {
                        parents.push(parent.clone());
                    }
        }
        if parents.len() == 1 {
            parents.into_iter().next()
        } else {
            None
        }
    }

    /// Find the parent type of a variant by looking up "*.variant_name" entries.
    ///
    /// Returns the parent_type_name if exactly one qualified variant with this
    /// name exists. Used to resolve type context when variable_type_names
    /// stores a variant name instead of the parent type name.
    pub fn find_variant_parent_type(&self, variant_name: &str) -> Option<String> {
        let suffix = format!(".{}", variant_name);
        let mut parents = Vec::new();
        for (fn_name, fn_info) in &self.functions {
            if fn_name.ends_with(&suffix) && fn_info.variant_tag.is_some()
                && let Some(ref parent) = fn_info.parent_type_name
                    && !parents.contains(parent) {
                        parents.push(parent.clone());
                    }
        }
        // Also check if variant_name itself is a registered variant
        if parents.is_empty() {
            for fn_info in self.functions.values() {
                if fn_info.variant_tag.is_some()
                    && let Some(ref parent) = fn_info.parent_type_name
                        && parent == variant_name {
                            return Some(variant_name.to_string());
                        }
            }
        }
        if parents.len() == 1 {
            parents.into_iter().next()
        } else {
            None // Ambiguous
        }
    }

    /// Check if any registered function has a name starting with the given prefix.
    /// Used to detect type namespaces (e.g., "IoError." has IoError.WouldBlock, etc.)
    pub fn has_functions_with_prefix(&self, prefix: &str) -> bool {
        self.functions.keys().any(|name| name.starts_with(prefix))
    }

    /// Search for a variant constructor whose name ends with the given suffix
    /// and has the expected parameter count.
    ///
    /// This is used when matching patterns to find the correct variant when
    /// there are name collisions (e.g., IpAddr.V6 vs SocketAddr.V6).
    /// Returns the first matching variant with payload type information.
    pub fn find_variant_with_suffix(&self, suffix: &str, expected_param_count: usize) -> Option<&FunctionInfo> {
        for (name, info) in &self.functions {
            // Only consider variant constructors (must have variant_tag set)
            if info.variant_tag.is_some()
                && name.ends_with(suffix)
                && info.param_count == expected_param_count
                && info.variant_payload_types.is_some()
            {
                return Some(info);
            }
        }
        None
    }

    // ==================== Closure Compilation Context ====================

    /// Saves the current label/loop context for closure compilation.
    ///
    /// When compiling a closure or generator, `begin_function()` clears labels,
    /// forward_jumps, loop_stack, and defer_stack. This method saves these values
    /// BEFORE calling `begin_function()` so they can be restored after.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let saved = ctx.save_closure_context();
    /// ctx.begin_function("closure", &[], None);
    /// // ... compile closure body ...
    /// let (instrs, reg_count) = ctx.end_function();
    /// ctx.restore_closure_context(saved);
    /// // outer function's loops/labels are now restored
    /// ```
    pub fn save_closure_context(&self) -> ClosureCompilationContext {
        ClosureCompilationContext {
            label_counter: self.label_counter,
            labels: self.labels.clone(),
            forward_jumps: self.forward_jumps.clone(),
            loop_stack: self.loop_stack.clone(),
            defer_stack: self.defer_stack.clone(),
            variable_type_names: self.variable_type_names.clone(),
        }
    }

    /// Restores label/loop context after closure compilation.
    ///
    /// Call this AFTER `end_function()` to restore the outer function's
    /// labels, forward_jumps, loop_stack, and defer_stack.
    pub fn restore_closure_context(&mut self, saved: ClosureCompilationContext) {
        self.label_counter = saved.label_counter;
        self.labels = saved.labels;
        self.forward_jumps = saved.forward_jumps;
        self.loop_stack = saved.loop_stack;
        self.defer_stack = saved.defer_stack;
        self.variable_type_names = saved.variable_type_names;
    }

    /// Looks up a function by qualified name (e.g., "module::function" or "Type::method").
    ///
    /// This is used for cross-module path resolution.
    pub fn lookup_qualified_function(&self, qualified_name: &str) -> Option<&FunctionInfo> {
        // First, try exact match
        if let Some(info) = self.functions.get(qualified_name) {
            return Some(info);
        }

        // Try without module prefix (simple resolution for single-module compilation)
        // e.g., "module::func" -> "func"
        if let Some(simple_name) = qualified_name.rsplit("::").next()
            && let Some(info) = self.functions.get(simple_name) {
                return Some(info);
            }

        None
    }

    /// Imports all functions from a pre-compiled module's registry.
    ///
    /// This is used during stdlib compilation to make functions from
    /// previously compiled modules (e.g., core) available when compiling
    /// dependent modules (e.g., collections, async).
    pub fn import_functions(&mut self, functions: &std::collections::HashMap<String, FunctionInfo>) {
        for (name, info) in functions {
            // Don't overwrite if already registered (local definitions take precedence)
            if !self.functions.contains_key(name) {
                self.functions.insert(name.clone(), info.clone());
            }
        }
    }

    /// Exports all currently registered functions.
    ///
    /// This is used during stdlib compilation to collect functions
    /// registered in this module for use by later modules.
    pub fn export_functions(&self) -> std::collections::HashMap<String, FunctionInfo> {
        self.functions.clone()
    }

    // ==================== Utilities ====================

    /// Checks if the context is valid for generating code.
    pub fn validate(&self) -> CodegenResult<()> {
        // Check for unresolved forward jumps
        if !self.forward_jumps.is_empty() {
            let labels: Vec<_> = self.forward_jumps.keys().collect();
            return Err(CodegenError::internal(format!(
                "unresolved forward jumps: {:?}",
                labels
            )));
        }

        Ok(())
    }

    /// Resets the context for a new module.
    pub fn reset(&mut self) {
        self.registers.reset();
        self.instructions.clear();
        self.label_counter = 0;
        self.labels.clear();
        self.forward_jumps.clear();
        self.loop_stack.clear();
        self.defer_stack.clear();
        self.defer_stack.push(Vec::new());
        self.current_function = None;
        self.in_function = false;
        self.return_type = None;
        self.constants.clear();
        self.strings.clear();
        self.string_intern.clear();
        self.functions.clear();
        self.stats = CodegenStats::default();
        self.variable_types.clear();
        if !self.variable_type_names.is_empty() {
            self.last_function_variable_types = self.variable_type_names.clone();
        }
        self.variable_type_names.clear();
        self.generic_type_params.clear();
        self.const_generic_params.clear();
        self.required_contexts.clear();
    }

    // ==================== Context System (using/provide) ====================

    /// Sets the required contexts for the current function.
    ///
    /// Called at the start of function compilation to track which context
    /// names from `using [...]` are available. When method calls are compiled,
    /// the codegen checks if the receiver is a required context and emits
    /// `CtxGet` accordingly.
    pub fn set_required_contexts(&mut self, contexts: &[String]) {
        self.required_contexts.clear();
        for ctx in contexts {
            self.required_contexts.insert(ctx.clone());
        }
    }

    /// Checks if a name is a required context for the current function.
    ///
    /// Returns true if the name was declared in the function's `using [...]` clause.
    pub fn is_required_context(&self, name: &str) -> bool {
        self.required_contexts.contains(name) || self.context_aliases.contains_key(name)
    }

    /// Resolve alias → context type name. Returns name itself if not an alias.
    pub fn resolve_context_alias(&self, name: &str) -> String {
        self.context_aliases.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// Clears the required contexts and aliases.
    pub fn clear_required_contexts(&mut self) {
        self.required_contexts.clear();
        self.context_aliases.clear();
    }

    /// Registers a variable's type for correct instruction selection.
    pub fn register_variable_type(&mut self, name: &str, type_kind: VarTypeKind) {
        self.variable_types.insert(name.to_string(), type_kind);
    }

    /// Gets a variable's type for instruction selection.
    pub fn get_variable_type(&self, name: &str) -> VarTypeKind {
        self.variable_types.get(name).copied().unwrap_or(VarTypeKind::Unknown)
    }

    /// Registers a constant's type for correct instruction selection.
    ///
    /// Unlike variable types, constant types persist across function compilations.
    /// This is necessary because constants are declared at module scope and used
    /// in multiple functions.
    pub fn register_constant_type(&mut self, name: &str, type_kind: VarTypeKind) {
        self.constant_types.insert(name.to_string(), type_kind);
    }

    /// Gets a constant's type for instruction selection.
    ///
    /// Returns Unknown if the constant type is not registered.
    pub fn get_constant_type(&self, name: &str) -> VarTypeKind {
        self.constant_types.get(name).copied().unwrap_or(VarTypeKind::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_context() {
        let ctx = CodegenContext::new();
        assert!(ctx.instructions.is_empty());
        assert!(!ctx.in_function);
        assert!(ctx.current_function.is_none());
    }

    #[test]
    fn test_label_management() {
        let mut ctx = CodegenContext::new();

        let l1 = ctx.new_label("test");
        let l2 = ctx.new_label("test");

        assert_ne!(l1, l2);
        assert!(l1.starts_with("test_"));
        assert!(l2.starts_with("test_"));
    }

    #[test]
    fn test_define_label() {
        let mut ctx = CodegenContext::new();

        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.define_label("target");
        ctx.emit(Instruction::Nop);

        assert_eq!(ctx.labels.get("target"), Some(&2));
    }

    #[test]
    fn test_forward_jump_patching() {
        let mut ctx = CodegenContext::new();

        // Emit forward jump
        ctx.emit_forward_jump("target", |offset| Instruction::Jmp { offset });
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.define_label("target");

        // Check that jump was patched
        match &ctx.instructions[0] {
            Instruction::Jmp { offset } => assert_eq!(*offset, 3),
            _ => panic!("expected Jmp instruction"),
        }
    }

    #[test]
    fn test_loop_context() {
        let mut ctx = CodegenContext::new();

        assert!(!ctx.in_loop());

        let loop1 = ctx.enter_loop(Some("outer".to_string()), None);
        assert!(ctx.in_loop());
        assert!(loop1.source_label.as_deref() == Some("outer"));

        let loop2 = ctx.enter_loop(None, Some(Reg(0)));
        assert!(loop2.break_value_reg == Some(Reg(0)));

        // Find by label
        let found = ctx.find_loop(Some("outer"));
        assert!(found.is_some());
        assert!(found.unwrap().source_label.as_deref() == Some("outer"));

        ctx.exit_loop();
        ctx.exit_loop();
        assert!(!ctx.in_loop());
    }

    #[test]
    fn test_defer_stack() {
        let mut ctx = CodegenContext::new();

        ctx.add_defer(vec![Instruction::Nop], false);
        ctx.add_defer(vec![Instruction::Nop, Instruction::Nop], true); // errdefer

        // Non-error path: only normal defers
        let defers = ctx.pending_defers(false);
        assert_eq!(defers.len(), 1);

        // Error path: both
        ctx = CodegenContext::new();
        ctx.add_defer(vec![Instruction::Nop], false);
        ctx.add_defer(vec![Instruction::Nop, Instruction::Nop], true);
        let defers = ctx.pending_defers(true);
        assert_eq!(defers.len(), 2);
    }

    #[test]
    fn test_constant_pool() {
        let mut ctx = CodegenContext::new();

        let c1 = ctx.add_const_int(42);
        let c2 = ctx.add_const_int(42); // Should reuse
        let c3 = ctx.add_const_int(100);

        assert_eq!(c1, c2);
        assert_ne!(c1, c3);

        let f1 = ctx.add_const_float(3.14);
        let f2 = ctx.add_const_float(3.14);
        assert_eq!(f1, f2);

        let s1 = ctx.add_const_string("hello");
        let s2 = ctx.add_const_string("hello");
        let s3 = ctx.add_const_string("world");
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_function_lifecycle() {
        let mut ctx = CodegenContext::new();

        ctx.begin_function("test_fn", &[("a".to_string(), false), ("b".to_string(), false)], None);

        assert!(ctx.in_function);
        assert_eq!(ctx.current_function.as_deref(), Some("test_fn"));

        // Parameters are allocated
        assert!(ctx.registers.get_reg("a").is_some());
        assert!(ctx.registers.get_reg("b").is_some());

        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::RetV);

        let (instrs, reg_count) = ctx.end_function();
        assert_eq!(instrs.len(), 2);
        assert!(reg_count >= 2);
        assert!(!ctx.in_function);
    }

    #[test]
    fn test_scope_management() {
        let mut ctx = CodegenContext::new();

        ctx.begin_function("test", &[], None);

        let r1 = ctx.define_var("x", false);
        ctx.enter_scope();
        let r2 = ctx.define_var("y", true);

        assert!(ctx.lookup_var("x").is_some());
        assert!(ctx.lookup_var("y").is_some());

        let (vars, _defers) = ctx.exit_scope(false);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].0, "y");
        assert_eq!(vars[0].1, r2);

        assert!(ctx.lookup_var("x").is_some());
        assert!(ctx.lookup_var("y").is_none());

        let _ = r1; // Suppress warning
    }

    #[test]
    fn test_validate() {
        let mut ctx = CodegenContext::new();

        // Valid context
        assert!(ctx.validate().is_ok());

        // Add unresolved forward jump
        ctx.record_forward_jump("undefined_label");
        assert!(ctx.validate().is_err());
    }
}
