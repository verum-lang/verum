//! Lowering context for VBC codegen.
//!

//! Tracks compilation state including:
//! - Current function being compiled
//! - Register allocator
//! - Label generation for jumps
//! - Loop context for break/continue
//! - Defer stack for cleanup
//! - CBGR tier decisions for reference operations

use super::error::{CodegenError, CodegenOptionExt, CodegenResult};
use super::registers::{RegisterAllocator, RegisterInfo};
use crate::cbgr::DereferenceCodegen;
use crate::instruction::{Instruction, Reg};
use crate::module::{ConstId, FunctionId};
use crate::types::{CbgrTier, TypeRef};
use std::collections::{HashMap, HashSet};
use verum_cbgr::tier_types::Tier0Reason;
use verum_common::Map;

/// ARCH-P2 canonical id→name choice — ONE total order over a
/// FunctionId's registration-key set, shared by every site that picks
/// a serialized/recorded name (stub descriptors ×3 incl. stage-3,
/// `external_function_names`, const force-emission, stage-3 stub-name
/// recording). HashMap walk / arrival order is per-process; any
/// first-seen or tie-kept choice is a bake dice.
///   1. more dots wins (most-qualified, identity-bearing; the trailing
///      segment recovers the canonical bare name);
///   2. equal dots: a non-`#` spelling beats a `name#arity` mirror
///      (dice-9: `#` is kept ONLY when it is the id's sole spelling);
///   3. still tied: lexicographically smallest.
pub(crate) fn canonical_name_better(new: &str, cur: &str) -> bool {
    let nd = new.matches('.').count();
    let cd = cur.matches('.').count();
    if nd != cd {
        return nd > cd;
    }
    let nh = new.contains('#');
    let ch = cur.contains('#');
    if nh != ch {
        return !nh;
    }
    new < cur
}

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

    /// Build target operating system (`"linux"` / `"macos"` / `"darwin"` /
    /// `"ios"` / `"windows"` / `"freebsd"` / `"none"` for embedded /
    /// cross-compilation targets).
    ///
    /// Mirrors `CodegenConfig::target_config::target_os` and is the
    /// canonical input for **cross-compilation-correct** code emission
    /// — every codegen path that materialises a platform-divergent
    /// constant (POSIX errno / socket / file flag, syscall number, etc.)
    /// MUST consult this field rather than the host platform.
    ///
    /// Defaults to the host target via `TargetConfig::host()` when
    /// constructed via `CodegenContext::new()`; cross-compilation
    /// callers (CLI `--target` flag, build scripts) overwrite this
    /// before any codegen begins.
    pub target_os: String,

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

    /// Inner generic args of the current function's return type, when
    /// known.  For `fn f() -> Result<T, ConnectionError>` this carries
    /// `vec!["T", "ConnectionError"]`; the variant disambiguator
    /// consults each entry as a candidate parent type when the simple
    /// name collides across modules.
    ///
    /// Without this list, `current_return_type_name` strips to
    /// `"Result"` and the disambiguator can't see that
    /// `ConnectionError.IoError(Text)` is the right pick over a
    /// same-named unit variant elsewhere — closing the
    /// "IoError arity expected 0 found 1" class of stdlib lenient SKIPs.
    pub current_return_type_inner: Option<Vec<String>>,

    /// Concrete type name of the impl block whose method body is
    /// currently being compiled (e.g. `"RecoveryRetryPolicy"` for a
    /// method registered under `RecoveryRetryPolicy.new`).  `None` for
    /// free functions.  Set by `compile_function` from its
    /// `impl_type_name` argument, cleared by `begin_function` /
    /// `end_function`.
    ///
    /// Unlike `current_return_type_name` (which is temporarily
    /// overwritten with let-annotation hints around `let x: T = …`
    /// initializer expressions — see statements.rs), this field stays
    /// stable for the whole method body, so it is the reliable source
    /// for resolving a bare `Self { … }` record literal to the concrete
    /// impl type during `compile_record`.
    pub current_impl_type_name: Option<String>,

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
    /// SAME-NAME-PARENT-TIEBREAK-1 (task #50): qualified names that
    /// resolved to MORE THAN ONE distinct real body during
    /// registration (the stdlib carries same-named types — two
    /// `Rational`s → two bodied `Rational.mul`s). Devirtualization is
    /// FORBIDDEN for these: a compile-time pick has no receiver
    /// TypeId, so it guesses; the CallM runtime resolver tiebreaks by
    /// the receiver's header TypeId instead.
    pub ambiguous_function_names: std::collections::HashSet<String>,

    /// **Scope-aware function index** (#17/#39 foundation, additive).
    ///
    /// Maps `(module_scope, simple_name)` -> `FunctionInfo`.  Populated
    /// in parallel with `functions` by `register_function` when
    /// `current_source_module` is known.  Consumers prefer this index
    /// over the bare `functions` table when a current compile-scope is
    /// available — the per-module entry shields name resolution from
    /// the cross-module first-wins collision class that surfaced 7+
    /// times this session (SIZE_CLASSES allocator vs size_class.vr,
    /// Char.from_digit canonical vs primitives.vr, etc).  Fallback to
    /// bare `functions` lookup when scope is absent or scope-specific
    /// entry is missing.  Architectural rule: every call site that
    /// previously did `lookup_function(name)` and risked first-wins
    /// shadow should migrate to `lookup_function_in_scope(scope, name)`.
    pub scoped_functions: HashMap<(String, String), FunctionInfo>,
    /// NAMES of free fns DECLARED by the unit being compiled (user-phase
    /// AST declarations only; stdlib bake — `prefer_existing_functions` —
    /// never writes here). Bare-name call resolution consults this
    /// FIRST: the unit's own `fn take` must beat `core.base.memory.take`
    /// in the type-aware overload scan (task #20). Stores names only —
    /// FunctionInfo snapshots go stale when ids are renumbered after
    /// declaration (a cached clone produced `Call 536870934` → "Function
    /// not found"); the lookup reads the LIVE registration instead.
    pub unit_declared_fns: std::collections::HashSet<String>,

    /// **ARCH-P2 stage 1 — content-addressed canonical function index**
    /// (dual keying, warn-on-divergence).  See
    /// `docs/architecture/tier-coherence-pillars.md`, Pillar 2,
    /// Migration stage (1).
    ///
    /// Parallel to `functions` / `scoped_functions`: keyed by the
    /// decl's FULLY-QUALIFIED path — derived from
    /// `current_source_module` + simple name by EXACTLY the
    /// descriptor-name promotion rule in `codegen/mod.rs`
    /// `compile_function` (bare names gain the module prefix;
    /// `$`-nested and already-dotted names stay as-is) — each path
    /// holding every DISTINCT content fingerprint ever registered
    /// under it.  `Vec` len > 1 == two different contents claiming
    /// one canonical path: the stage-1 divergence corpus that gates
    /// stage 2 (flip readers to canonical).
    ///
    /// Stage-1 contract: NOTHING consults this index for
    /// dispatch/lookup — zero behavior change.  Writers are
    /// `register_function` and `register_function_authoritative`
    /// (primary registrations only; the `name#arity` alt-key mirrors
    /// those methods maintain are arity-disambiguation shadows, not
    /// decls, and never land here).  The emit path enriches entries
    /// with a body fingerprint via
    /// [`Self::enrich_canonical_body_fingerprint`].
    ///
    /// Tracing: `VERUM_TRACE_CANON=1` (or `=<substring>` to filter
    /// by canonical path) logs `[canon-diverge]` the moment a path
    /// gains a second distinct fingerprint; module finalization
    /// additionally emits the `[canon-report]` sweep
    /// ([`Self::canonical_vs_bare_report`]).
    pub canonical_index: HashMap<String, Vec<CanonicalFnEntry>>,

    /// When true, register_function will not overwrite existing entries.
    /// Used when importing stdlib modules after user code has been registered.
    pub prefer_existing_functions: bool,

    /// **Task #47 stage-3 stub-id preservation** (architectural fix
    /// for the cascade where stub-ids leak into runtime as orphan
    /// `[lenient] stage-3 ... stub never resolved` panics).
    ///
    /// Records the original `(stub_id -> name)` mapping for every
    /// stage-3 stub-id (range `[u32::MAX - 0x100_0000 - 0x10_0000,
    /// u32::MAX - 0x100_0000]`) ever registered, regardless of
    /// whether a subsequent `register_function` call OVERWRITES
    /// the bare-name slot in `functions` with a real-id `FunctionInfo`.
    ///
    /// The cascade root cause: when module B emits `Call(stub_id)`
    /// referencing module A's `foo`, the consumer-side bytecode
    /// holds the stub_id.  When module A later compiles and
    /// overwrites `functions["foo"]` with the real_id, the
    /// stub_id -> "foo" mapping is lost from `functions`.
    /// `emit_stage3_stub_descriptors` then iterates `functions` and
    /// CANNOT synthesize a descriptor for stub_id (no matching name
    /// in `functions`), so the archive lacks the
    /// `external_function_names[stub_id]` -> "foo" mapping that
    /// `ArchiveBodyRemap::map_function` needs to chase the real body.
    /// The stub_id leaks to runtime as orphan -> panic.
    ///
    /// This side table preserves the original stub_id -> name
    /// regardless of subsequent overwrites, so the stage-3 emit
    /// pass can always synthesize the descriptor.
    pub stage3_stub_names: HashMap<u32, String>,

    /// **Stage-5 pending mounts** — explicit braced-mount items whose
    /// target couldn't be resolved at mount time (producing module not
    /// compiled yet AND simple name not globally unique, so stages
    /// 1-4 have no stub). Maps the local alias to the mount's FULL
    /// qualified path. `compile_call`'s miss path consults this to
    /// synthesize a stage-5 stub with the call site's arity, bound to
    /// the qualified spelling the archive name-remap can chase
    /// unambiguously.
    pub pending_mount_aliases: HashMap<String, String>,

    /// Descending allocation counter for stage-5 stub ids
    /// (`stub_ranges::STAGE5_BASE - counter`).
    pub stage5_stub_counter: u32,

    /// Dotted module path for functions currently being collected/compiled.
    ///

    /// The codegen's `config.module_name` is fixed per-codegen-session and
    /// is `"main"` for a user-run of a single `.vr` file. But when that
    /// session subsequently pulls in stdlib `imported_modules`, each of
    /// those has its own `module X.Y.Z;` declaration at the top of the
    /// file, and their top-level functions should be registered under
    /// `X.Y.Z.fn_name` — not under `main.fn_name`.
    ///

    /// `register_function` (the method wrapper, not the HashMap setter)
    /// reads this field to produce qualified aliases for cross-module
    /// paths like `super.darwin.tls.ctx_get` to resolve. `None` means
    /// "use `config.module_name` as-is".
    pub current_source_module: Option<String>,

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

    /// Names of bindings (params) whose DECLARED type is a REFERENCE
    /// (`&T` / `&mut T` / `&checked T` / `&unsafe T`, plus `&self`-shape
    /// receivers). Consulted by the `*x` (Deref) lowering: `*reference` is a
    /// built-in dereference (→ pointee), NOT an invocation of the pointee
    /// type's `Deref` impl — the latter applies only to `*value`. Without
    /// this, `let old = *self` in `Maybe.replace`/`take` (self: &mut Maybe,
    /// and Maybe `implement Deref { fn deref(&self)->&T }`) wrongly called
    /// `Maybe::deref` and yielded the payload T instead of the Maybe.
    /// Populated per-function at compile_function entry; saved/restored across
    /// closure bodies alongside `variable_type_names`.
    pub reference_bindings: std::collections::HashSet<String>,

    /// Parameter REGISTERS whose declared param type is a REFERENCE to a
    /// statically-known HEAP OBJECT (`f: &mut Formatter`, `&self` on a
    /// record impl, …) — Pillar 1 carried fact
    /// (docs/architecture/tier-coherence-pillars.md).  A re-ref of such a
    /// param (the `&mut self` receiver wrapping for `f.write_str(..)`, or
    /// an explicit `&mut f`) emits the typed `RefObj` opcode instead of
    /// untyped `RefMut`, so Tier-1 lowers a pointer passthrough BY OPCODE
    /// CONTRACT instead of re-deriving object-ness from heuristic mark
    /// sets.  Tier-0 executes RefObj on the RefMut path unchanged.
    ///
    /// Populated per-function at compile_function entry (param i ↔ register
    /// i via `alloc_parameters`); cleared by `begin_function`; saved and
    /// restored across closure bodies — closures re-use low register
    /// indices for their own params, so inheriting the outer set would
    /// mis-tag them (unlike the name-keyed `reference_bindings`, which IS
    /// deliberately inherited for captured-variable deref semantics).
    pub object_ref_param_regs: std::collections::HashSet<u16>,

    /// Snapshot of `variable_type_names` from the last compiled function body.
    /// Preserved across function boundaries so the playground can read bindings
    /// after compilation (the main map gets cleared between functions).
    pub last_function_variable_types: HashMap<String, String>,

    /// REFL-CLOSURE-XREC-1 (runtime leg): element-type hints for the user
    /// params of a closure compiled as an iterator-adapter argument
    /// (`xs.iter().map(|p| p.x + p.y)` — `p` carries the receiver's element
    /// type). Shape: `(span_key, hints)` where `span_key` identifies the
    /// exact `ExprKind::Closure` node the hints were derived for
    /// (`(span.start << 32) | span.end`) and `hints[i]` types user param
    /// `i` (captures excluded).
    ///
    /// Set once at `compile_method_call` entry when the method is a known
    /// adapter, an arg is a closure, and the receiver's element type
    /// resolves to a registered record; consumed by `compile_closure` IFF
    /// the dispatching closure's span key matches — unmatched entries are
    /// left in place (a nested adapter in receiver position must not
    /// steal the outer call's hints) and are inert by construction (the
    /// key can only ever match the closure it was derived for). The
    /// consumer registers hinted params in `variable_type_names` — the
    /// same channel the for-loop binder uses (`compile_for`) — so field
    /// accesses inside the closure body resolve LOCAL field indices
    /// instead of falling through `resolve_field_index` to the global
    /// intern table (wrong offsets → runtime "field access out of
    /// bounds"). `None` entries / absent hints fail open to today's
    /// behavior.
    pub closure_param_type_hints: Option<(u64, Vec<Option<String>>)>,

    /// Current match scrutinee type name for resolving variant patterns.
    ///

    /// When compiling `match expr { V6(x) => ... }`, we need to know if V6 refers to
    /// `IpAddr.V6` or `SocketAddr.V6`. If the scrutinee `expr` has a known type,
    /// we store it here so pattern binding can use the qualified variant name.
    pub match_scrutinee_type: Option<String>,

    /// When the match scrutinee is a tuple expression `(a, b, c)`, this holds
    /// the per-element type names so that the tuple-pattern destructure path
    /// in `compile_pattern_bind` can set [`Self::match_scrutinee_type`]
    /// per-element before recursing into each sub-pattern.  Without this,
    /// `match (self, other) { (Some(a), Some(b)) => a == b }` inside a
    /// protocol impl would lose `Maybe<T>` context for the inner element
    /// pattern; payload-type inference in `Some(a)` would then fail to
    /// register `a`'s type, falling through to a primitive `CmpI` on the
    /// Maybe wrapper instead of dispatching `a == b` correctly.  Set in
    /// `compile_match` when the scrutinee is `ExprKind::Tuple(_)`; cleared
    /// to `None` for non-tuple scrutinees.
    pub match_tuple_element_types: Option<Vec<Option<String>>>,

    /// Per-element type names for a `let (a, b, …) = <tuple-expr>`
    /// destructure, set by `compile_let` and CONSUMED (`.take()`) by the
    /// Tuple arm of `compile_pattern_bind`. Kept separate from
    /// `match_tuple_element_types` (which `compile_match` populates and
    /// must survive across ALL arms of a multi-arm match) so consuming it
    /// for one let-destructure cannot starve a later match arm — the
    /// RECORD-LET-REF-TYPE-LOSS fix must not regress
    /// `match (self, other) { … }` element typing.
    pub pending_let_tuple_types: Option<Vec<Option<String>>>,

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
    /// Parent-type name → its ORDERED generic-parameter names
    /// (`ControlFlow` → ["B", "C"]). Carried fact for pattern-bind
    /// payload typing: a variant's payload template (`Continue(C)`)
    /// maps to the parent's param POSITION (`C` = #1), which indexes
    /// the scrutinee's instantiated args. The previous
    /// tag-as-arg-index heuristic silently swapped payload types for
    /// any sum type whose variant order doesn't mirror its param
    /// order (ControlFlow<B, C> is Continue(C) | Break(B)) — #47
    /// runtime leg. Filled by BOTH type-registration paths (local
    /// `register_type_constructors`, archive ctx loader pass 4).
    pub type_generic_params: HashMap<String, Vec<String>>,

    /// The subset of `type_generic_params` that are CONST params
    /// (`type MyAlloc<const SIZE: Int>` ⇒ {"MyAlloc": {"SIZE"}}).
    /// `type_generic_params` does not record the const-vs-type kind, but a
    /// bare-impl form (`implement MyAlloc<SIZE>` with no `<const SIZE: Int>`
    /// clause) must still classify the inherited `SIZE` as const so the body
    /// emits the value-witness `LoadT{Generic}` rather than the type-param
    /// `LoadNil`.  Filled alongside `type_generic_params`.
    pub type_const_param_names: HashMap<String, std::collections::HashSet<String>>,

    /// Same params in DECLARATION ORDER (#44-B): `TypeRef::Generic(idx)`
    /// witnesses are positional, so `T.default()`-class emission needs
    /// T's index among the enclosing function's generics. Maintained in
    /// lockstep with `generic_type_params`; cleared per function.
    pub generic_type_params_ordered: Vec<String>,

    /// Registers with a LIVE register-reference taken on them (#48
    /// self-referential-slot corruption class). `emit_ref_instruction`
    /// pins its `src` here; `free_temp` refuses to recycle a pinned
    /// slot. Without the pin, the referent's temp could be freed and
    /// the very next alloc reuse it — codegen then emitted
    /// `Ref r13<-r10; Mov r10<-r13` (List.fmt_debug element loop),
    /// storing a ref INTO its own referent slot: the interpreter's
    /// ref-unwrap recursed to a stack-guard SIGBUS (pre-guard) or a
    /// soft method-miss (post-guard). Cleared per function by
    /// `begin_function`.
    pub ref_pinned_regs: std::collections::HashSet<u16>,

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

    /// CONST-GENERIC-VALUE-CARRY-1 (task #19): staged type/const witness
    /// args for the IMMEDIATELY-FOLLOWING `compile_static_method_call`.
    ///
    /// `StackAllocator<256>.new()` reaches the static-call emitter through
    /// a `FunctionInfo` that carries no callsite instantiation, and for a
    /// const-only impl the descriptor is not even "generic" (const params
    /// have no `TypeRef::Generic` occurrence in params/return), so
    /// `record_generic_instantiation` derives nothing.  The TypeExpr
    /// receiver branch in `compile_method_call` parses the receiver's
    /// generic args (`Const(256)` → `ConstValue(256)`) and stages them
    /// here; `compile_static_method_call` TAKES the value at entry (every
    /// early-return path drops it — no leakage into unrelated calls by
    /// construction) and emits `CallG` carrying the witnesses.
    pub pending_static_call_type_args: Option<Vec<crate::types::TypeRef>>,

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

    /// Mounted TYPE-name bindings: alias/simple name → full dotted mount
    /// path (`mount core.meta.token.{Group}` ⇒ `"Group" →
    /// "core.meta.token.Group"`; `Group as TokGroup` ⇒ `"TokGroup" → …`).
    ///
    /// META-GROUP-XMODULE-1: type registries (`type_name_to_id`,
    /// `type_field_layouts`) are keyed by SIMPLE name with first-wins
    /// across every loaded archive module, so two stdlib types sharing a
    /// simple name (`meta.token.Group` record vs `math.algebra.Group`
    /// protocol) collide — the loser is silently dropped and its record
    /// literals fall through to the global-intern field-index fallback
    /// (out-of-bounds `SetF`). This table carries the user's explicit
    /// mount intent so `resolve_record_type_key` can re-key the lookup
    /// to the module-qualified registration (`"core.meta.Group"`).
    /// Populated by `process_import_tree` for uppercase-leaf mounts.
    pub mounted_types: HashMap<String, String>,

    /// Module aliases populated from bare-path `mount X.Y.Z;` declarations.
    ///
    /// Each entry maps the rightmost path segment (`Z`) to the **full
    /// module-path segments** (`["X", "Y", "Z"]`). When the user later
    /// writes `Z.fn(args)` or `Z.CONST`, the codegen path-resolution
    /// (`try_flatten_module_path`) consults this table and expands the
    /// single-segment receiver to the full module path, so the
    /// downstream qualified-function lookup finds
    /// `X.Y.Z.fn` / `X.Y.Z.CONST` in the registry.
    ///
    /// Distinct from the function-alias registration that
    /// `process_import_tree` already does for `mount X.Y.fn` — that
    /// path imports a *function* under its simple name. Module aliases
    /// import a *module* under its rightmost segment, so callers reach
    /// every exported function/const via qualified access.
    ///
    /// Without this table, `mount core.sys.bitfield;` followed by
    /// `bitfield.test_bit(v, 7)` fails at codegen with `unbound
    /// variable: bitfield`, blocking every cross-module conformance
    /// suite that relies on the canonical module-qualified call form
    /// (`core-tests/sys/bitfield`, `core-tests/collections/union_find`,
    /// `core-tests/collections/toposort`).
    pub module_aliases: HashMap<String, Vec<String>>,

    /// Variables that hold byte arrays (contiguous byte buffers).
    ///

    /// When a variable is declared as `let buf: [Byte; N] = uninit()` or similar,
    /// it's marked as a byte array variable. This affects how `&mut buf[idx] as *mut Byte`
    /// is compiled - we emit `ByteArrayElementAddr` instead of `GetE + Ref` to get
    /// the actual memory address of the element rather than its value.
    pub byte_array_vars: HashSet<String>,

    /// Task #18 — escape-analysis result for the current function.
    ///
    /// Names of local variables whose `&local` ref flows out of the function
    /// via `Ret { value: &local }` (implicit trailing-expression return or
    /// explicit `return &local;`).  Populated by
    /// `analyze_escaping_local_refs` at `compile_function` entry; consumed
    /// by `compile_block`'s scope-exit DropRef emission so the slot's
    /// generation is NOT bumped — without this skip the caller's CBGR ref
    /// would carry the pre-bump generation and trip use-after-free
    /// detection on the next deref.
    ///
    /// Cleared at `compile_function` exit so the set never leaks across
    /// function compilations.  When the function's return type is not a
    /// reference type the set is always empty (no escape possible).
    pub current_fn_escaping_vars: HashSet<String>,

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

    /// Simple names brought into scope by an EXPLICIT `mount X.Y.{name}` /
    /// `mount X.Y.fn as alias` (every site that calls
    /// `register_function_authoritative`).  Used by bare-name call
    /// resolution to prefer the user's explicitly-mounted target over an
    /// unmounted same-simple-name function from another module that an
    /// arg-type overload match would otherwise select (INTRINSIC-MOUNT-
    /// COLLISION: `mount core.intrinsics.arithmetic.{saturating_add}` was
    /// losing to `core.math.checked.saturating_add(Int64,Int64)` when the
    /// argument was a typed `Int`).
    pub explicit_mount_names: HashSet<String>,
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
    /// Saved reference-binding names (for the `*reference` Deref lowering).
    pub reference_bindings: std::collections::HashSet<String>,
    /// Saved object-ref param registers (Pillar 1 typed-ref emission) —
    /// register-keyed, so the closure body must NOT inherit them.
    pub object_ref_param_regs: std::collections::HashSet<u16>,
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

/// One canonical-identity record in
/// [`CodegenContext::canonical_index`] (**ARCH-P2 stage 1** — see
/// `docs/architecture/tier-coherence-pillars.md`, Pillar 2).
///
/// `fingerprint` is blake3 (truncated to u64) over the
/// registration-visible **signature surface** of the decl:
/// `param_count`, `param_names`, `return_type_name`, `is_async`,
/// `is_generator`, `is_const`, `parent_type_name`, `variant_tag`.
/// The function id is deliberately NOT part of the fingerprint —
/// ids are module-local and re-homed on collision, which is exactly
/// the disease this identity replaces.
///
/// Function BODY content does not exist at registration time, so
/// body-content addressing arrives when descriptors themselves are
/// keyed canonically (stage-2 prep).  Until then the emit path
/// (`compile_function`'s descriptor build) back-fills
/// `body_fingerprint` from the finalized descriptor surface:
/// promoted name + decoded instruction count + register count.  The
/// encoded bytecode byte-length only materialises at serialization
/// time (`FunctionDescriptor::bytecode_length` is 0 until then) and
/// joins the fingerprint when descriptors are keyed in stage 2.
#[derive(Debug, Clone)]
pub struct CanonicalFnEntry {
    /// Signature-surface content fingerprint (registration time).
    pub fingerprint: u64,
    /// Emit-time descriptor/body fingerprint; `None` until
    /// `compile_function` finalizes this decl's descriptor (variant
    /// constructors, consts, and pattern-functions keep `None` in
    /// stage 1).
    pub body_fingerprint: Option<u64>,
    /// Snapshot of the FIRST `FunctionInfo` registered under this
    /// (path, fingerprint).  Same-fingerprint re-registrations are
    /// idempotent and keep this snapshot — a later id claiming the
    /// bare slot surfaces in `canonical_vs_bare_report` (deliberate
    /// stage-1 evidence, not a lookup input).
    pub info: FunctionInfo,
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

    /// `true` when this entry was created from a `const` (or `static`)
    /// declaration via [`register_constant_with_value`].  The codegen
    /// represents consts as zero-arg functions for storage uniformity,
    /// but the typechecker treats them as values — without this flag
    /// the archive-driven typechecker can't distinguish `mount X.CONST`
    /// from `mount X.zero_arg_fn`.  Round-trips through
    /// `vbc::FunctionDescriptor::is_const`.
    pub is_const: bool,

    /// `true` when this function is the synthetic constructor of a
    /// **transparent wrapper** type — newtype (`type X is T;`),
    /// single-element tuple (`type X is (T,)`), or quotient
    /// (`quotient X is T`).  Such a constructor has no compiled body;
    /// calls compile to an identity Mov (the wrapper is zero-cost at
    /// runtime — the value IS the inner value).
    ///
    /// Architecturally this discriminator REPLACES the prior crutch of
    /// inspecting `id.0 == u32::MAX / 2 && args.len() == 1` in codegen
    /// to decide whether to pass-through inner args. The sentinel ID
    /// is shared by every body-less synthetic constructor (newtype,
    /// record, unit, quotient), so the bare ID can't distinguish a
    /// transparent wrapper (correct: pass-through) from a record
    /// constructor whose `param_count` happens to be 1 — or worse,
    /// from a same-id collision with a real protocol-impl method
    /// (registration bug: previously routed `Text.from(s)` through
    /// the passthrough path, returning the raw `s` literal as a
    /// detagged Value and breaking every downstream method dispatch
    /// on the result).
    ///
    /// Set ONLY by `compile_type_decl` arms in
    /// `verum_vbc/src/codegen/mod.rs` when the type is genuinely a
    /// transparent wrapper.  Every other sentinel-id constructor
    /// (records / units / variants) leaves this flag at its default
    /// `false`, so the disambiguation is structural, not heuristic.
    pub is_transparent_wrapper: bool,

    /// For each parameter, the *return-type simple-name* of that
    /// parameter's function-type signature, when the parameter is
    /// callable (i.e. its declared type is `fn(...) -> X`, or a
    /// generic-param whose bound resolves to `fn(...) -> X`).
    ///
    /// Parallel to `param_names` / `param_type_names`; entries are
    /// `None` for parameters that aren't function-typed.  At a call
    /// site, when arg `i` is a closure expression, codegen pushes
    /// `param_closure_return_type_names[i]` as the
    /// `current_return_type_name` disambig context before compiling
    /// the closure body — so bare variant constructors in the body
    /// (`Continue(...)`, `Some(...)`, …) consult the right type's
    /// variant table even when the simple name collides across two
    /// sum types (e.g. `ReduceResult.Continue` vs `ControlFlow.Continue`).
    ///
    /// Populated by `register_function` (see `mod.rs::register_function`).
    /// Empty by default — callers that don't populate it (test stubs,
    /// synthetic constructors, etc.) get the no-op path.
    pub param_closure_return_type_names: Vec<Option<String>>,
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
        self.decisions
            .get(&expr_id)
            .copied()
            .unwrap_or(self.default_tier)
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
    pub fn set_tier_with_reason(
        &mut self,
        expr_id: ExprId,
        tier: CbgrTier,
        reason: Option<Tier0Reason>,
    ) {
        self.decisions.insert(expr_id, tier);
        if tier == CbgrTier::Tier0
            && let Some(r) = reason
        {
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
            format!(
                "Reference requires runtime validation: {}",
                reason.description()
            )
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

    /// Iterate over all `(ExprId, CbgrTier)` decisions.
    ///

    /// Used by per-function tier-analysis aggregators in the
    /// compiler pipeline that need to merge multiple per-function
    /// `TierContext`s into a module-level one. Pre-#118 the
    /// pipeline iterated `0..decision_count()` and constructed
    /// `ExprId(i)` for `i in 0..N` — but `from_analysis_result`
    /// populates decisions with span-encoded `ExprId(start<<32|end)`
    /// values, so the `0..N` lookup always missed and the merge
    /// silently inserted only `default_tier` (Tier0). CBGR tier
    /// promotion (~15ns → 0ns) was never applied to user code.
    /// Exposing the canonical iterator makes the correct merge a
    /// one-liner: `for (id, t) in src.iter_decisions() { dst.set_tier(id, t); }`.
    pub fn iter_decisions(&self) -> impl Iterator<Item = (ExprId, CbgrTier)> + '_ {
        self.decisions.iter().map(|(k, v)| (*k, *v))
    }

    /// Iterate over all `(ExprId, Tier0Reason)` entries — the
    /// companion of `iter_decisions` for the per-function aggregator
    /// when it wants to preserve diagnostic provenance for refs that
    /// stayed at Tier 0 (e.g., escape-via-return).
    pub fn iter_tier0_reasons(&self) -> impl Iterator<Item = (ExprId, Tier0Reason)> + '_ {
        self.tier0_reasons.iter().map(|(k, v)| (*k, *v))
    }

    /// Merge another tier context's decisions into this one.
    ///

    /// Conflict policy: later wins (last-write semantics inside the
    /// `Map::insert`). In practice the per-function aggregator
    /// produces disjoint key sets — every function's expressions
    /// have distinct (start,end) spans within a module — so this
    /// behaves like a disjoint union for the canonical caller.
    pub fn merge_from(&mut self, other: &TierContext) {
        for (id, t) in other.iter_decisions() {
            self.decisions.insert(id, t);
        }
        for (id, r) in other.iter_tier0_reasons() {
            self.tier0_reasons.insert(id, r);
        }
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
    ///  (encoded as `(start << 32) | end`) to match VBC codegen's span-based lookup.
    /// - If no span info is available, we fall back to using RefId directly as ExprId.
    ///

    /// The span-based approach is preferred because VBC codegen creates ExprId from
    /// expression spans, and this ensures tier decisions are correctly looked up.
    pub fn from_analysis_result(result: &verum_cbgr::tier_analysis::TierAnalysisResult) -> Self {
        use verum_cbgr::tier_types::CbgrTier as AnalysisTier;

        let mut decisions = Map::new();

        // ARCH-P2: result.decisions is a HashMap; two RefIds collapsing
        // to one span-derived ExprId made the surviving tier last-wins in
        // hash order. Sorted RefId walk → deterministic collision winner
        // (highest RefId, stable across bakes).
        let mut ordered: Vec<_> = result.decisions.iter().collect();
        ordered.sort_by_key(|(rid, _)| rid.0);

        for (ref_id, tier) in ordered {
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

        // Extract Tier0 reasons from the analysis result (same sorted
        // walk as `decisions` above — one collision discipline).
        let mut tier0_reasons = Map::new();
        let mut ordered_reasons: Vec<_> = result.decisions.iter().collect();
        ordered_reasons.sort_by_key(|(rid, _)| rid.0);
        for (ref_id, tier) in ordered_reasons {
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
            target_os: verum_ast::cfg::TargetConfig::host().target_os.to_string(),
            current_function: None,
            in_function: false,
            return_type: None,
            current_return_type_name: None,
            current_impl_type_name: None,
            current_return_type_inner: None,
            constants: Vec::new(),
            strings: Vec::new(),
            string_intern: HashMap::new(),
            bytes: Vec::new(),
            bytes_intern: HashMap::new(),
            functions: HashMap::new(),
            ambiguous_function_names: std::collections::HashSet::new(),
            scoped_functions: HashMap::new(),
            unit_declared_fns: std::collections::HashSet::new(),
            canonical_index: HashMap::new(),
            prefer_existing_functions: false,
            stage3_stub_names: HashMap::new(),
            pending_mount_aliases: HashMap::new(),
            stage5_stub_counter: 0,
            current_source_module: None,
            stats: CodegenStats::default(),
            tier_context: TierContext::new(),
            suspend_point_count: 0,
            variable_types: HashMap::new(),
            constant_types: HashMap::new(),
            variable_type_names: HashMap::new(),
            reference_bindings: std::collections::HashSet::new(),
            object_ref_param_regs: std::collections::HashSet::new(),
            last_function_variable_types: HashMap::new(),
            closure_param_type_hints: None,
            match_scrutinee_type: None,
            match_tuple_element_types: None,
            pending_let_tuple_types: None,
            raw_pointer_regs: HashSet::new(),
            generic_type_params: HashSet::new(),
            generic_type_params_ordered: Vec::new(),
            type_generic_params: HashMap::new(),
            type_const_param_names: HashMap::new(),
            ref_pinned_regs: std::collections::HashSet::new(),
            const_generic_params: HashSet::new(),
            pending_static_call_type_args: None,
            newtype_names: HashSet::new(),
            newtype_inner_type: HashMap::new(),
            user_defined_types: HashSet::new(),
            mounted_types: HashMap::new(),
            module_aliases: HashMap::new(),
            byte_array_vars: HashSet::new(),
            current_fn_escaping_vars: HashSet::new(),
            typed_array_vars: HashMap::new(),
            try_recover_depth: 0,
            required_contexts: HashSet::new(),
            explicit_mount_names: HashSet::new(),
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
            generic_type_params_ordered: Vec::new(),
            type_generic_params: HashMap::new(),
            type_const_param_names: HashMap::new(),
            ref_pinned_regs: std::collections::HashSet::new(),
            const_generic_params: HashSet::new(),
            pending_static_call_type_args: None,
            typed_array_vars: HashMap::new(),
            byte_array_vars: HashSet::new(),
            required_contexts: HashSet::new(),
            explicit_mount_names: HashSet::new(),
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
    ///
    /// FN-LOCAL-STATIC-ONCE-1 (task #16): a `static` declared inside a
    /// fn body is HOISTED to a module-level synthetic cell registered
    /// under the mangled key `<fn>$static$<name>` (see the
    /// `ItemKind::Static` arm in `collect_declarations`).  Resolution
    /// here is scope-first: while compiling the enclosing fn's body
    /// (`current_function` set), the mangled key is probed before the
    /// bare name so the body reads/writes its own hoisted once-init
    /// cell — and a fn-local static correctly shadows any module-level
    /// static of the same name.  This is the ONE authority every
    /// by-name TLS consult goes through (identifier read, assignment
    /// TlsSet, compound-assign, `&STATIC` address-of, path-head
    /// classification).
    pub fn is_thread_local(&self, name: &str) -> Option<u16> {
        if self.thread_local_vars.is_empty() {
            return None;
        }
        if !name.contains('$')
            && let Some(cf) = self.current_function.as_deref()
        {
            // Full current-function key first (covers nested-fn
            // mangles like `outer$inner`), then the bare last dot
            // segment (covers `Type.method` / `module.fn` forms —
            // collection pushed only the undotted fn name).
            let mangled = format!("{}$static${}", cf, name);
            if let Some(&slot) = self.thread_local_vars.get(&mangled) {
                return Some(slot);
            }
            let bare = cf.rsplit('.').next().unwrap_or(cf);
            if bare != cf {
                let mangled = format!("{}$static${}", bare, name);
                if let Some(&slot) = self.thread_local_vars.get(&mangled) {
                    return Some(slot);
                }
            }
        }
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
    pub fn cache_active_pattern_result(
        &mut self,
        scrutinee: Reg,
        pattern_name: &str,
        result_reg: Reg,
    ) {
        self.active_pattern_cache
            .insert((scrutinee, pattern_name.to_string()), result_reg);
    }

    /// Retrieves a cached Active pattern result.
    ///

    /// Returns the register containing the `Maybe<T>` result from the pattern
    /// function call made during `compile_pattern_test()`.
    ///

    /// Returns `None` if no cached result exists (shouldn't happen in normal flow).
    pub fn get_cached_active_pattern_result(
        &self,
        scrutinee: Reg,
        pattern_name: &str,
    ) -> Option<Reg> {
        self.active_pattern_cache
            .get(&(scrutinee, pattern_name.to_string()))
            .copied()
    }

    /// Clears the active pattern cache for a specific scrutinee.
    ///

    /// Called at the end of each match arm to prevent stale entries from
    /// being used in subsequent arms.
    pub fn clear_active_pattern_cache_for(&mut self, scrutinee: Reg) {
        self.active_pattern_cache
            .retain(|(s, _), _| *s != scrutinee);
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
            Instruction::JmpCmp {
                op: _,
                a: _,
                b: _,
                offset: o,
            } => *o = offset,
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
    pub fn emit_backward_jump(
        &mut self,
        label: &str,
        make_instr: impl FnOnce(i32) -> Instruction,
    ) -> CodegenResult<()> {
        let offset = self
            .label_offset(label)
            .or_internal_else(|| format!("undefined label: {}", label))?;
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
    pub fn enter_loop(
        &mut self,
        source_label: Option<String>,
        break_value_reg: Option<Reg>,
    ) -> LoopContext {
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
            Some(lbl) => self
                .loop_stack
                .iter()
                .rev()
                .find(|ctx| ctx.source_label.as_deref() == Some(lbl)),
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
    pub fn exit_scope(
        &mut self,
        is_error_path: bool,
    ) -> (Vec<(String, Reg)>, Vec<Vec<Instruction>>) {
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
    pub fn get_effective_tier(
        &self,
        expr_id: super::context::ExprId,
        want_tier2: bool,
    ) -> CbgrTier {
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
                && *v == value
            {
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
                && v.to_bits() == value.to_bits()
            {
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
            // ARCH-P2 dice tracing: VERUM_TRACE_STRDICE=<substr> prints
            // provenance for the FIRST intern of any matching string —
            // byte-diff localization of nondeterministic bakes ends at
            // a string-table delta; this names the interning code path.
            {
                static MARKER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
                if let Some(m) = MARKER.get_or_init(|| std::env::var("VERUM_TRACE_STRDICE").ok())
                    && !m.is_empty() && value.contains(m.as_str())
                {
                    let bt = std::backtrace::Backtrace::force_capture().to_string();
                    // Release builds: symbol names are stripped/inlined, the
                    // `verum_` filter often matches NOTHING and the trace
                    // degrades to a bare header.  Keep raw frames (addresses
                    // symbolicate offline via atos/addr2line against the
                    // binary) when the filtered view is empty.
                    let filtered: Vec<&str> = bt
                        .lines()
                        .filter(|l| l.contains("verum_"))
                        .take(12)
                        .collect();
                    let frames: Vec<&str> = if filtered.is_empty() {
                        bt.lines().take(30).collect()
                    } else {
                        filtered
                    };
                    eprintln!(
                        "[strdice] intern {:?} module={:?} strings_len={}\n{}",
                        value,
                        self.current_source_module.as_deref(),
                        self.strings.len(),
                        frames.join("\n")
                    );
                }
            }
            let id = self.strings.len() as u32;
            self.strings.push(value.to_string());
            self.string_intern.insert(value.to_string(), id);
            id
        }
    }

    /// Adds a string constant and returns its ID.
    pub fn add_const_string(&mut self, value: &str) -> ConstId {
        // Single interning authority — was a duplicated inline copy of
        // intern_string_raw (drift risk + bypassed the dice tracing).
        let string_id = self.intern_string_raw(value);

        // Check for existing string constant
        for (i, c) in self.constants.iter().enumerate() {
            if let ConstantEntry::String(sid) = c
                && *sid == string_id
            {
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
        self.registers
            .get_reg(name)
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
        // #48: a slot with a LIVE register-ref taken on it must never
        // be recycled — see `ref_pinned_regs`.
        if self.ref_pinned_regs.contains(&reg.0) {
            return;
        }
        self.raw_pointer_regs.remove(&reg);
        self.registers.free_temp(reg);
    }

    // ==================== Function Management ====================

    /// Starts compiling a new function.
    ///

    /// Each parameter is a tuple of (name, is_mutable).
    pub fn begin_function(
        &mut self,
        name: &str,
        params: &[(String, bool)],
        return_type: Option<TypeRef>,
    ) {
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
        // Pillar 1: register-keyed — must not leak across functions (and
        // closures re-use low register indices for their own params).
        self.object_ref_param_regs.clear();
        self.ref_pinned_regs.clear();
        // GENERIC PARAMS ARE NOT CLEARED HERE (#44-B contract repair).
        // compile_function documents (mod.rs ~14078): "For impl methods,
        // impl generics are PRE-SET by compile_item before calling this,
        // so we add to them rather than clearing." The item-level walks
        // (compile_item / compile_item_lenient / pending-default drain)
        // own the clear+populate; this mid-sequence clear ERASED the
        // impl-level generics for EVERY impl method body — `T.default()`
        // inside `implement<T> Maybe<T>` saw an empty param set, missed
        // the witness intercept, and collapsed into the type-namespace
        // LoadNil stub (Maybe.flatten's None arm returned nil). Closures
        // and generators (their compiles also route through here) must
        // ALSO see the enclosing function's generics, so not clearing is
        // correct for them too.
        self.byte_array_vars.clear();
        self.typed_array_vars.clear();
        self.active_pattern_cache.clear();
        self.raw_pointer_regs.clear();

        self.current_function = Some(name.to_string());
        self.in_function = true;
        self.return_type = return_type;
        self.current_impl_type_name = None;
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
        self.current_return_type_inner = None;
        self.current_impl_type_name = None;

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

    /// Records a register→owner-type hint for the current function
    /// (FUNC-REGISTRY-QUALIFICATION-1). Used where VBC codegen statically
    /// knows a register's owner type but the bytecode alone can't recover it
    /// (the for-loop `__for_iter` temp). Flushed into the FunctionDescriptor
    /// at build time and consumed only by the AOT reg_types pass.
    pub fn add_register_type_hint(&mut self, register: u16, type_name: String) {
        self.registers.push_type_hint(register, type_name);
    }

    /// Returns the register→owner-type hints collected for the current
    /// function. Collected before `end_function` clears register state,
    /// mirroring `collect_debug_variables`.
    pub fn collect_register_type_hints(&self) -> Vec<(u16, String)> {
        self.registers.collect_type_hints()
    }

    /// Marks a parameter register whose declared type is a reference to a
    /// statically-known heap object (Pillar 1 typed-ref emission).
    pub fn mark_object_ref_param_reg(&mut self, reg: u16) {
        self.object_ref_param_regs.insert(reg);
    }

    /// True when `reg` is a parameter register whose declared type is a
    /// reference to a statically-known heap object — a re-ref of it emits
    /// the typed `RefObj` opcode (Pillar 1).
    pub fn is_object_ref_param_reg(&self, reg: u16) -> bool {
        self.object_ref_param_regs.contains(&reg)
    }

    /// Atomically save+override the variant-disambiguation context
    /// `(current_return_type_name, current_return_type_inner)`,
    /// returning the old pair so a caller can restore it.
    ///
    /// Single-source-of-truth helper for the half-dozen
    /// expression-compile sites that temporarily override the
    /// disambiguation context (let-binding annotations, call-arg type
    /// hints, assert_eq's first-arg-drives-second pattern, closure
    /// returns, etc.).  Without it, sites that update only one field
    /// leave the other stale, which can mis-resolve a same-name
    /// variant when the surrounding return type's inner generics
    /// happen to point at the same family.
    pub fn push_disambig_context(
        &mut self,
        new_name: Option<String>,
    ) -> (Option<String>, Option<Vec<String>>) {
        let prev_name = self.current_return_type_name.take();
        let prev_inner = self.current_return_type_inner.take();
        self.current_return_type_name = new_name;
        // The new override is unrelated to the surrounding function's
        // return type, so its inner generics don't apply here.
        self.current_return_type_inner = None;
        (prev_name, prev_inner)
    }

    /// Restore the variant-disambiguation context saved by
    /// [`push_disambig_context`](Self::push_disambig_context).
    pub fn pop_disambig_context(&mut self, saved: (Option<String>, Option<Vec<String>>)) {
        let (name, inner) = saved;
        self.current_return_type_name = name;
        self.current_return_type_inner = inner;
    }

    /// Registers a function for lookup.
    ///

    /// Two collision strategies:
    ///  * different arity → stored under `name#arity` so
    ///  `lookup_function_with_arity` can pick the right one at the
    ///  call site (e.g. FFI `write(fd, buf, n)` and high-level
    ///  `write(path, contents)`).
    ///  * same arity → in `prefer_existing_functions` mode keep the
    ///  existing entry; otherwise overwrite (user-mode wins).
    ///

    /// The arity-suffix branch must run in BOTH modes — without it, in
    /// stdlib-loading mode the second registration is dropped on the
    /// floor, so the caller-site `lookup_function_with_arity` can't find
    /// the alternative arity and resolves the wrong function. That was
    /// the root of the `wrong number of arguments for write: expected 2,
    /// found 3` cluster, where the FFI 3-arity `write` was silently
    /// dropped in favour of the 2-arity `core/io/fs.vr` shim, breaking
    /// every `safe_write` / `safe_pread` / `safe_send` wrapper that
    /// genuinely needs the FFI.
    pub fn register_function(&mut self, name: String, info: FunctionInfo) {
        // Diagnostic (VERUM_TRACE_FNREG=<substring>): logs EVERY
        // registration whose key contains the filter — id, arity,
        // return type, and whether the key was already bound. The
        // registration graph has many writers (compile_function,
        // archive Pass 3/4, mounts, ctor fanouts); this is the one
        // choke point they all funnel through.
        if let Ok(filter) = std::env::var("VERUM_TRACE_FNREG")
            && name.contains(&filter)
        {
            eprintln!(
                "[fnreg] '{}' id={} arity={} rt={:?} variant_tag={:?} prev={:?}",
                name,
                info.id.0,
                info.param_count,
                info.return_type_name.as_deref(),
                info.variant_tag,
                self.functions.get(&name).map(|e| (e.id.0, e.param_count)),
            );
        }
        // CTOR-BINDING-SHIELD-1 (#51): a bare (dotless) key whose
        // existing binding is a VARIANT CONSTRUCTOR (variant_tag=Some)
        // is a language-level name — `Continue(42)` compiles to a
        // MakeVariant through it.  A tag-less FUNCTION alias arriving
        // later (archive simple-alias sweeps, ghost stage-3 stubs —
        // the postgres archive carries a phantom
        // 'core.net.weft.dst.Continue' RetV descriptor whose bare
        // alias clobbered ControlFlow's ctor and sent `Continue(42)`
        // into KeyType.eq) must NOT replace it: the ctor stays, the
        // late alias remains reachable through its qualified key.
        // Deliberate rebinds go through
        // `register_function_authoritative`, which this gate does not
        // guard.  Shape-based (tag presence), not a name list — per
        // the no-hardcoded-stdlib-knowledge rule.
        if !name.contains('.')
            && info.variant_tag.is_none()
            && self
                .functions
                .get(&name)
                .is_some_and(|existing| existing.variant_tag.is_some())
        {
            if std::env::var("VERUM_TRACE_FNREG").is_ok() {
                eprintln!(
                    "[fnreg] SHIELD kept ctor binding for bare '{}' (incoming id={} has no variant_tag)",
                    name, info.id.0
                );
            }
            return;
        }
        // ARCH-P2 stage 1 — content-addressed canonical identity,
        // DUAL keying.  Purely additive parallel index: the bare /
        // `name#arity` / scoped tables below keep sole lookup
        // authority (stage-1 contract: zero behavior change).  Runs
        // FIRST so every early `return` in the collision branches
        // below still lands the primary registration canonically;
        // the `name#arity` mirrors those branches create are direct
        // `functions` inserts that never route back through here.
        self.register_canonical(&name, &info);
        // FUNC-REGISTRY-QUALIFICATION-1 (phase 2) — module-qualified
        // mirror key.  Every bare simple-name registration that happens
        // while a source-module scope is active ALSO lands under its
        // qualified `<module>.<name>` key, so collision-prone bare
        // slots (the documented THREE bare `range` registrations; bare
        // `Text.new` mis-binding inside stdlib-merged suite contexts)
        // stay recoverable: qualified consumers (`resolve_function_key`'s
        // suffix scan, the archive loader's canonical forms, the
        // stub-descriptor emitters' longest-dotted name preference)
        // can always reach the per-module entry even after the bare
        // slot was claimed by another module's same-name function.
        //
        // Discipline mirrors the metadata layer (METADATA-DETERMINISM-1,
        // commit 54d0ae1d4): the qualified slot is strictly additive
        // and FIRST-WINS (`or_insert`) — it never replaces an existing
        // entry, and the bare slot's collision policy below is
        // untouched.  Runs BEFORE the arity-collision branch so a
        // registration demoted to `<name>#<arity>` still gets its
        // qualified key (the FFI `write`-3-arity class loses only the
        // bare slot, not its module identity).
        //
        // Gates:
        //  * bare names only — dotted names (`Type.method`, module
        //    paths), `::`-qualified forms and `name#arity` alt-keys
        //    already carry their own qualification;
        //  * variant constructors excluded — their canonical qualified
        //    form is `<ParentType>.<Variant>` (registered separately);
        //    module-qualifying the bare alias would double-count them
        //    in the `.{variant}` suffix-scan disambiguators
        //    (`find_variant_by_suffix_and_args` returns None on
        //    ambiguity, so a duplicate key can flip a unique match
        //    into a miss);
        //  * `main` (single-file user compiles) excluded — mirrors the
        //    `compile_function` dot-qualified registration discipline
        //    in `codegen/mod.rs`.
        if let Some(scope) = &self.current_source_module
            && !scope.is_empty()
            && scope.as_str() != "main"
            && info.variant_tag.is_none()
            && !name.contains('.')
            && !name.contains("::")
            && !name.contains('#')
            // MIRROR-OWNERSHIP-1 (#51 ghost root): the mirror asserts
            // "<scope> DECLARES <name>" — that is only true for ids this
            // compile allocated for real local bodies.  Sentinel-band
            // ids (FFI externs, newtype/variant ctors, stage-1..5
            // stubs — everything ≥ u32::MAX/4) are dispatch-band
            // registrations passing through while SOME module is in
            // scope; gluing the scope onto them FABRICATES provenance.
            // Live failure: dst.vr's stage-5 stub for bare `Continue`
            // (a `?`-desugar ControlFlow ctor reference) got mirrored
            // as 'core.net.weft.dst.Continue'; the accumulated ctx
            // carried that key into the POSTGRES module's stub-emit,
            // whose canonical name pick (most dots wins) stamped the
            // phantom onto the archive descriptor — and its bare alias
            // clobbered ControlFlow's ctor at every user-side load
            // (pre-shield: `Continue(42)` dispatched to KeyType.eq).
            && info.id.0 < u32::MAX / 4
        {
            let qualified = format!("{}.{}", scope, name);
            // Same tracing contract as the funnel head: the mirror is a
            // DIRECT insert (never routes back through
            // register_function), so without this line a mirror-created
            // key is invisible to VERUM_TRACE_FNREG — the exact blind
            // spot that hid the 'core.net.weft.dst.Continue' ghost key
            // (#51).
            if let Ok(filter) = std::env::var("VERUM_TRACE_FNREG")
                && qualified.contains(&filter)
            {
                eprintln!(
                    "[fnreg-MIRROR] '{}' (scope='{}' bare='{}') id={} arity={}",
                    qualified, scope, name, info.id.0, info.param_count
                );
            }
            self.functions
                .entry(qualified)
                .or_insert_with(|| info.clone());
        }
        if let Some(existing) = self.functions.get(&name).cloned()
            && existing.param_count != info.param_count
        {
            let existing = &existing;
            // FUNDAMENTAL #6 — arity-collision richness promotion.
            //
            // The bare-name slot in `self.functions` was, pre-fix,
            // monopolised by whichever registration ran first.  When
            // an early stub-shaped registration (no `return_type_name`,
            // no body) arrived before the real fn, the real fn — with
            // proper signature info — got shoved to the `<name>#<arity>`
            // alt-key and the bare slot kept the stub.  Every
            // bare-keyed `lookup_function(name)` then returned the
            // info-less stub, breaking type inference at call sites
            // (FUNDAMENTAL #5 patched one symptom in
            // `extract_expr_type_name`; this fix closes the root).
            //
            // Discipline: when the incoming `info` is strictly RICHER
            // than the existing entry — has a concrete `return_type_name`
            // where existing has None — promote the new entry to the
            // bare slot and demote the existing entry to its OWN
            // `<name>#<existing_arity>` alt-key.  Both arities remain
            // arity-aware-lookup-addressable; the bare slot now holds
            // the registration with usable type info.
            //
            // Preserves the FFI-raw / safe-wrapper precedence the
            // original first-wins discipline guarded: when both new
            // AND existing have `return_type_name` (or both lack it),
            // first-wins still applies.  Only the stub-vs-real
            // asymmetric case promotes.
            // **Stub-vs-real promotion heuristic (#23 fundamental fix)**.
            //
            // Pre-fix the rule was strictly `new.rt.is_some() &&
            // existing.rt.is_none()` — i.e. "richer return-type info
            // wins". That heuristic FAILS in the following defect
            // pattern (observed for `file_exists`, `read_file`, every
            // common simple-name across modules):
            //
            //   1. Real `core.sys.file_ops.file_exists(path: Text) -> Bool`
            //      registers under bare `file_exists` with
            //      `return_type_name = None` (the early decl-collection
            //      pass hadn't yet extracted the return type) and
            //      `param_count = 1`.
            //
            //   2. A panic-stub for `core.database.…value_api.file_exists`
            //      (or any other module's failed compile) registers
            //      with `return_type_name = Some("()")` (Unit) and
            //      `param_count = 0` — synthesised by
            //      `emit_lenient_panic_stub` with the canonical
            //      `Panic; RetV;` 1-byte body.
            //
            //   3. The arity-collision branch saw `new.rt.is_some()`
            //      (Unit) vs `existing.rt.is_none()` and PROMOTED the
            //      stub to the bare slot — demoting the real function
            //      to `file_exists#1`.  Call sites resolving bare
            //      `file_exists` then dispatched to the 0-param stub
            //      and the real function's bytecode body was
            //      unreachable.
            //
            // Sharpened heuristic: the stub-vs-real signature is
            // distinctive — stubs have `param_count == 0` AND return
            // Unit (`rt == "()"`).  Don't promote in that shape even
            // when the existing entry has no return_type_name.
            let new_is_stub_shape =
                info.param_count == 0 && info.return_type_name.as_deref() == Some("()");
            let existing_is_stub_shape =
                existing.param_count == 0 && existing.return_type_name.as_deref() == Some("()");
            let new_is_richer = info.return_type_name.is_some()
                && existing.return_type_name.is_none()
                && !new_is_stub_shape;
            // The inverse: if the EXISTING entry is a stub-shape and
            // the new entry is a real function (any non-zero
            // param_count OR non-Unit return), promote the new entry
            // regardless of `rt.is_some()` parity.  This closes the
            // case where the stub got there first and the real fn
            // arrives second.
            let new_is_real_over_stub = existing_is_stub_shape
                && !new_is_stub_shape
                && info.param_count > 0;
            let alt_key = format!("{}#{}", name, info.param_count);
            // ARCHIVE-SERIALIZE-DETERMINISM-1 wave 3: the bare-slot
            // winner among same-name DIFFERENT-ARITY functions was
            // ARRIVAL-ORDER-dependent (parallel bake feeds
            // registrations in completion order) — byte-diff of two
            // bakes showed `on_signal#2`+`broadcast_channel` vs
            // `on_signal`+`broadcast_channel#1`, flipping the string
            // table and every bare-suffix dispatch winner (the §40
            // canary's dice). Canonical order-independent rule: among
            // non-stub candidates the LOWEST ARITY owns the bare slot
            // (ties keep the incumbent); everyone else lives at
            // `name#arity`, which the arity-aware lookup probes first
            // anyway. The richer/real-over-stub promotions below stay
            // (they only fire on equal-arity or stub shapes).
            // SAME-NAME-PARENT-TIEBREAK-1: two DISTINCT real bodies
            // claiming one name (same arity — a genuine same-signature
            // duplicate, not an overload) poison compile-time
            // devirtualization for that name.
            if !new_is_stub_shape
                && !existing_is_stub_shape
                && info.id != existing.id
                && info.param_count == existing.param_count
            {
                if std::env::var("VERUM_TRACE_TIEBREAK").is_ok() {
                    eprintln!(
                        "[ambig-mark] '{}' ids {}/{} arity {}",
                        name, existing.id.0, info.id.0, info.param_count
                    );
                }
                self.ambiguous_function_names.insert(name.clone());
            }
            if !new_is_stub_shape
                && !existing_is_stub_shape
                && info.param_count != existing.param_count
            {
                // BOTH parties always get their `name#arity` mirror so
                // the KEY SET (and thus the interned-string set feeding
                // the bake's string table) is arrival-order-independent
                // — wave 3 made the bare WINNER deterministic but the
                // alt-mirror existed only on displacement, which still
                // flipped `on_signal` vs `on_signal#2` presence between
                // bakes.
                let existing_alt =
                    format!("{}#{}", name, existing.param_count);
                let existing_clone = existing.clone();
                self.functions
                    .entry(existing_alt)
                    .or_insert(existing_clone);
                if info.param_count < existing.param_count {
                    self.functions.insert(alt_key, info.clone());
                    self.functions.insert(name, info);
                } else {
                    self.functions.entry(alt_key).or_insert(info);
                }
                return;
            }
            if new_is_richer || new_is_real_over_stub {
                let existing_alt = format!("{}#{}", name, existing.param_count);
                if let Some(existing_info) = self.functions.remove(&name) {
                    self.functions
                        .entry(existing_alt)
                        .or_insert(existing_info);
                }
                // Wave 5 (ARCHIVE-SERIALIZE-DETERMINISM-1): mirror the
                // NEW entry too — the promotion branch created only the
                // displaced party's alt, so the KEY SET still depended
                // on arrival order (spawn_detached vs
                // spawn_detached#1 flipped per bake).
                self.functions.insert(alt_key.clone(), info.clone());
                self.functions.insert(name, info);
                return;
            }
            // Same alternative-arity precedence as the simple name:
            // first-wins under prefer_existing, last-wins otherwise.
            if self.prefer_existing_functions {
                let existing_alt =
                    format!("{}#{}", name, existing.param_count);
                let existing_clone = existing.clone();
                self.functions
                    .entry(existing_alt)
                    .or_insert(existing_clone);
                self.functions.entry(alt_key).or_insert(info);
            } else {
                self.functions.insert(alt_key, info);
            }
            return;
        }
        // ARCHIVE-SERIALIZE-DETERMINISM-1 dice-8: the name#arity
        // mirror was created only INSIDE collision branches, so the
        // KEY SET (→ interned-string set → bake bytes) depended on
        // whether a same-name collision HAPPENED — which is
        // arrival-order-dependent (byte-diff: broadcast_channel#1
        // present in one bake, spawn_detached#1 in the other).
        // Create the mirror UNCONDITIONALLY at first registration of
        // any bare name: the key set becomes
        // {bare, bare#arity, qualified…} regardless of collision
        // history. Lookups already probe name#arity as a fallback,
        // so extra mirrors are semantically inert.
        if !name.contains('.') && !name.contains("::") {
            let alt_key = format!("{}#{}", name, info.param_count);
            self.functions.entry(alt_key).or_insert_with(|| info.clone());
        }
        // **#17/#39 scope-aware mirror**: also record under the
        // (current_source_module, name) key so collision-prone lookups
        // can prefer the per-scope entry.  Only fires when scope is
        // known AND the simple name is bare (no qualifier dots) — fully-
        // qualified registrations are already collision-safe.
        if let Some(scope) = &self.current_source_module
            && !name.contains('.') && !name.contains("::")
        {
            let key = (scope.clone(), name.clone());
            if self.prefer_existing_functions {
                self.scoped_functions.entry(key).or_insert(info.clone());
            } else {
                self.scoped_functions.insert(key, info.clone());
            }
        }
        // **Stub-name preservation (task #47 stage-3 + stage-4
        // consts)** — record every name-resolved stub-id we observe
        // BEFORE the bare-name slot in `functions` potentially gets
        // overwritten by a real-id registration.  See
        // `stage3_stub_names` field doc.  Ranges are canonical in
        // `crate::stub_ranges`; we check BOTH the incoming info AND
        // any existing entry so we catch the stub-id at either side
        // of the overwrite.
        //
        // ARCH-P2: the recorded spelling is CANONICAL over arrivals —
        // first-seen let bare vs `name#arity` flip with registration
        // order, which flipped the recovered stage-3 stub descriptor
        // name per bake (the on_signal byte dice).
        if crate::stub_ranges::is_name_resolved_stub_id(info.id.0) {
            self.stage3_stub_names
                .entry(info.id.0)
                .and_modify(|existing| {
                    if canonical_name_better(&name, existing) {
                        *existing = name.clone();
                    }
                })
                .or_insert_with(|| name.clone());
        }
        if let Some(existing) = self.functions.get(&name)
            && crate::stub_ranges::is_name_resolved_stub_id(existing.id.0)
        {
            self.stage3_stub_names
                .entry(existing.id.0)
                .and_modify(|existing_name| {
                    if canonical_name_better(&name, existing_name) {
                        *existing_name = name.clone();
                    }
                })
                .or_insert_with(|| name.clone());
        }
        if std::env::var("VERUM_TRACE_TIEBREAK").is_ok() && name.ends_with("Rational.mul") {
            eprintln!(
                "[reg-tail] '{}' id={} prefer_existing={} (fresh-name path)",
                name, info.id.0, self.prefer_existing_functions
            );
        }
        // SAME-NAME-PARENT-TIEBREAK-1 (task #50), bare-suffix leg: two
        // module-qualified registrations with DIFFERENT full names can
        // still collide on the `Type.method` suffix (core.math.Rational
        // vs core.text.numeric.Rational). Only the FIRST gets the bare
        // mirror, so the direct-collision detector above never sees the
        // second — detect the suffix collision here and poison
        // devirtualization for the bare spelling.
        if name.contains('.') {
            let mut it = name.rsplitn(3, '.');
            if let (Some(m), Some(t)) = (it.next(), it.next()) {
                let bare2 = format!("{}.{}", t, m);
                if bare2 != name
                    && let Some(prev) = self.functions.get(&bare2)
                    && prev.id != info.id
                {
                    if std::env::var("VERUM_TRACE_TIEBREAK").is_ok() {
                        eprintln!(
                            "[ambig-mark] '{}' via suffix of '{}' (ids {}/{})",
                            bare2, name, prev.id.0, info.id.0
                        );
                    }
                    self.ambiguous_function_names.insert(bare2);
                }
            }
        }
        if self.prefer_existing_functions {
            self.functions.entry(name).or_insert(info);
        } else {
            self.functions.insert(name, info);
        }
    }

    /// Authoritatively register `info` under `name`, replacing any prior
    /// binding regardless of `prefer_existing_functions` and regardless of
    /// arity collision.
    ///
    /// Use this for explicit user bindings — `mount X.Y.{name}` and
    /// `mount X.Y.fn as alias` — where the user has named the function
    /// they want and that binding MUST win over any passive archive-load
    /// or stdlib-bootstrap registration of the same simple name.  Passive
    /// loads (intrinsic registration, archive ingestion, stdlib bootstrap)
    /// keep using `register_function` — its first-wins protects the
    /// FFI-raw / safe-wrapper precedence rule that callers rely on.
    ///
    /// Architectural rationale: an explicit mount establishes a per-file
    /// authoritative binding for the simple name in scope.  When the
    /// stdlib has 7+ free-function definitions sharing the same simple
    /// name (`select`, `join`, `len`, `from`, `read`, …) — the user's
    /// mount path is the disambiguating signal.  Routing arity collisions
    /// to `name#arity` for an explicit mount silently dropped the user's
    /// chosen function into a shadow slot while the bare name still
    /// resolved to whichever passive load happened to be first — the
    /// runtime then dispatched to a different module's function with the
    /// matching simple name (e.g. `core.shell.interactive.select`
    /// instead of `core.async.future.select`).  Authoritative
    /// registration breaks that collision class by making the user's
    /// binding the unconditional simple-name owner.
    ///
    /// Returns the previously-registered `FunctionInfo`, if any.
    pub fn register_function_authoritative(
        &mut self,
        name: String,
        info: FunctionInfo,
    ) -> Option<FunctionInfo> {
        if let Ok(filter) = std::env::var("VERUM_TRACE_FNREG")
            && name.contains(&filter)
        {
            eprintln!(
                "[fnreg-AUTH] '{}' id={} arity={} rt={:?} prev={:?}",
                name,
                info.id.0,
                info.param_count,
                info.return_type_name.as_deref(),
                self.functions.get(&name).map(|e| (e.id.0, e.param_count)),
            );
        }
        // ARCH-P2 stage 1 — canonical DUAL keying, same additive
        // discipline as `register_function` (an authoritative mount
        // is still a primary registration of the decl's content).
        self.register_canonical(&name, &info);
        // Mirror the `name#arity` shadow slot so callers that resolve via
        // `lookup_function_with_arity` still find the chosen variant —
        // the authoritative binding owns BOTH `name` AND `name#arity`
        // (the only key the arity-aware lookup probes).
        let alt_key = format!("{}#{}", name, info.param_count);
        self.functions.insert(alt_key, info.clone());
        // Record that this simple name was explicitly mounted, so bare-name
        // call resolution can prefer this user-chosen target over an
        // unmounted same-name function selected by arg-type overload.
        self.explicit_mount_names.insert(name.clone());
        self.functions.insert(name, info)
    }

    // ------------------------------------------------------------------
    // ARCH-P2 stage 1 — content-addressed canonical function identity.
    //
    // Contract (docs/architecture/tier-coherence-pillars.md, Pillar 2):
    // every decl's canonical identity = fully-qualified path +
    // blake3(content).  Stage 1 computes it and keys a PARALLEL index,
    // WITHOUT changing any existing winner/lookup semantics; the
    // divergence corpus this gathers gates stage 2 (flip readers).
    // ------------------------------------------------------------------

    /// `VERUM_TRACE_CANON` gate.  `"1"` / `"true"` / `"*"` / `"all"`
    /// (or an empty value) match every path; any other value is a
    /// substring filter on the canonical path — the
    /// `VERUM_TRACE_FNREG` convention.
    fn canon_trace_enabled_for(path: &str) -> bool {
        match std::env::var("VERUM_TRACE_CANON") {
            Ok(v) => {
                v.is_empty()
                    || v == "1"
                    || v == "true"
                    || v == "*"
                    || v == "all"
                    || path.contains(&v)
            }
            Err(_) => false,
        }
    }

    /// Fully-qualified canonical path for a registration key —
    /// EXACTLY the `compile_function` descriptor-name promotion rule
    /// (`codegen/mod.rs`): prefix `current_source_module` when the
    /// name is bare (no `.` / `$`) and the scope is a real module;
    /// keep dotted (`Type.method`, module paths) and `$`-nested
    /// names as-is.  The context has no `config.module_name`
    /// fallback — with no source-module scope active the bare name
    /// IS the canonical path (single-file `main` compiles).
    pub fn canonical_qualified_path(&self, name: &str) -> String {
        match self.current_source_module.as_deref() {
            Some(scope)
                if !scope.is_empty()
                    && scope != "main"
                    && !name.contains('.')
                    && !name.contains('$') =>
            {
                format!("{}.{}", scope, name)
            }
            _ => name.to_string(),
        }
    }

    /// blake3 (truncated to u64) over the registration-visible
    /// signature surface of a decl.  Field order and separators are
    /// FIXED — this is a stable identity input, not a display format.
    /// See [`CanonicalFnEntry`] for what is (and is not) covered.
    pub fn signature_fingerprint(info: &FunctionInfo) -> u64 {
        let mut h = blake3::Hasher::new();
        h.update(&(info.param_count as u64).to_le_bytes());
        for p in &info.param_names {
            h.update(p.as_bytes());
            // NUL separator: ["ab","c"] must not collide with ["a","bc"].
            h.update(&[0u8]);
        }
        h.update(&[0xFFu8]); // end-of-params sentinel
        match info.return_type_name.as_deref() {
            Some(rt) => {
                h.update(&[1u8]);
                h.update(rt.as_bytes());
            }
            None => {
                h.update(&[0u8]);
            }
        }
        h.update(&[
            info.is_async as u8,
            info.is_generator as u8,
            info.is_const as u8,
        ]);
        match info.parent_type_name.as_deref() {
            Some(p) => {
                h.update(&[1u8]);
                h.update(p.as_bytes());
            }
            None => {
                h.update(&[0u8]);
            }
        }
        match info.variant_tag {
            Some(t) => {
                h.update(&[1u8]);
                h.update(&t.to_le_bytes());
            }
            None => {
                h.update(&[0u8]);
            }
        }
        u64::from_le_bytes(
            h.finalize().as_bytes()[..8]
                .try_into()
                .expect("blake3 output is 32 bytes; [..8] always fits"),
        )
    }

    /// Insert a PRIMARY registration into the canonical index.
    ///
    /// Additive-only (never consulted by lookups in stage 1).
    /// `name#arity` alt-keys are excluded — canonical identity
    /// belongs to the decl, not its arity-disambiguation shadow.
    /// Re-registering the same (path, fingerprint) is idempotent;
    /// a second DISTINCT fingerprint on one path appends (that IS
    /// the divergence corpus) and warns under `VERUM_TRACE_CANON`.
    ///
    /// Hot-path budget (28K registrations on mounted scripts): one
    /// hash lookup + one small blake3 + at most one `String` alloc —
    /// O(1) amortized; the env read happens only on divergence.
    fn register_canonical(&mut self, name: &str, info: &FunctionInfo) {
        if name.contains('#') {
            return;
        }
        let path = self.canonical_qualified_path(name);
        let fp = Self::signature_fingerprint(info);
        match self.canonical_index.get_mut(&path) {
            Some(entries) => {
                if entries.iter().any(|e| e.fingerprint == fp) {
                    return; // identical content re-registered
                }
                entries.push(CanonicalFnEntry {
                    fingerprint: fp,
                    body_fingerprint: None,
                    info: info.clone(),
                });
                if Self::canon_trace_enabled_for(&path) {
                    let fps: Vec<String> = entries
                        .iter()
                        .map(|e| format!("{:016x}", e.fingerprint))
                        .collect();
                    let arities: Vec<usize> =
                        entries.iter().map(|e| e.info.param_count).collect();
                    let ids: Vec<u32> =
                        entries.iter().map(|e| e.info.id.0).collect();
                    eprintln!(
                        "[canon-diverge] path={} fps=[{}] arities={:?} ids={:?}",
                        path,
                        fps.join(","),
                        arities,
                        ids
                    );
                }
            }
            None => {
                self.canonical_index.insert(
                    path,
                    vec![CanonicalFnEntry {
                        fingerprint: fp,
                        body_fingerprint: None,
                        info: info.clone(),
                    }],
                );
            }
        }
    }

    /// Emit-path enrichment: back-fill the canonical entry's
    /// `body_fingerprint` when `compile_function` finalizes the
    /// decl's descriptor.  `path` is the promoted descriptor name
    /// (same derivation as [`Self::canonical_qualified_path`]); the
    /// entry is matched by function id, falling back to the bare
    /// simple name for registrations that happened without the
    /// module scope the promotion saw.  A miss is traced under
    /// `VERUM_TRACE_CANON` — stage-1 evidence, never an error.
    ///
    /// The fingerprint input is the descriptor surface available at
    /// emit time: promoted name + decoded instruction count +
    /// register count.  Encoded bytecode byte-length exists only at
    /// serialization time and joins in stage 2 when descriptors are
    /// keyed canonically.
    pub fn enrich_canonical_body_fingerprint(
        &mut self,
        path: &str,
        id: FunctionId,
        instruction_count: usize,
        register_count: u16,
    ) {
        let mut h = blake3::Hasher::new();
        h.update(path.as_bytes());
        h.update(&(instruction_count as u64).to_le_bytes());
        h.update(&(register_count as u64).to_le_bytes());
        let fp = u64::from_le_bytes(
            h.finalize().as_bytes()[..8]
                .try_into()
                .expect("blake3 output is 32 bytes; [..8] always fits"),
        );
        let bare = path.rsplit('.').next().unwrap_or(path);
        for key in [path, bare] {
            if let Some(entries) = self.canonical_index.get_mut(key)
                && let Some(e) =
                    entries.iter_mut().find(|e| e.info.id.0 == id.0)
            {
                e.body_fingerprint = Some(fp);
                return;
            }
            if key == bare {
                break; // don't probe the same key twice for dotless paths
            }
        }
        if Self::canon_trace_enabled_for(path) {
            eprintln!(
                "[canon-body-miss] path={} id={} (no canonical entry)",
                path, id.0
            );
        }
    }

    /// Canonical paths currently claimed by MORE THAN ONE distinct
    /// content fingerprint — the stage-1 divergence corpus.  Sorted
    /// by path for deterministic assertions / report output.
    pub fn canonical_divergences(&self) -> Vec<(&str, usize)> {
        let mut v: Vec<(&str, usize)> = self
            .canonical_index
            .iter()
            .filter(|(_, entries)| entries.len() > 1)
            .map(|(path, entries)| (path.as_str(), entries.len()))
            .collect();
        v.sort_unstable();
        v
    }

    /// POST-REGISTRATION consistency sweep: compares the BARE-KEY
    /// winner in `functions` against the canonical-path claimants of
    /// the same simple name.  Reports two shapes:
    ///
    ///  * `ghost-bare-winner` — the bare slot's function id is not
    ///    among ANY canonical entry for the simple name (a writer
    ///    bypassed primary registration, or a same-signature
    ///    re-registration displaced the winner after canonical
    ///    snapshot);
    ///  * `cross-module` — more than one canonical path claims the
    ///    simple name (the first-wins collision class Pillar 2
    ///    exists to retire).
    ///
    /// Skips `name#arity` mirrors, dotted / `$`-nested keys, and
    /// bare variant-constructor aliases (a variant's canonical form
    /// is `Parent.Variant`; its module-qualified bare alias would
    /// report a path-spelling split for every variant in the stdlib
    /// and drown the signal).  Pure query — no lookup semantics
    /// change; O(bare names + canonical paths); callers run it only
    /// under `VERUM_TRACE_CANON`.  Output sorted for determinism.
    pub fn canonical_vs_bare_report(&self) -> Vec<String> {
        // simple-name -> canonical paths, one pass over the index.
        let mut by_simple: HashMap<&str, Vec<&str>> = HashMap::new();
        for path in self.canonical_index.keys() {
            let simple = path.rsplit('.').next().unwrap_or(path.as_str());
            by_simple.entry(simple).or_default().push(path.as_str());
        }
        let mut report = Vec::new();
        for (name, bare) in &self.functions {
            if name.contains('.')
                || name.contains('#')
                || name.contains("::")
                || name.contains('$')
                || bare.variant_tag.is_some()
            {
                continue;
            }
            let Some(paths) = by_simple.get(name.as_str()) else {
                // Name never canonically registered — nothing to
                // compare (stage 1 only indexes the two register_*
                // entry points).
                continue;
            };
            let mut canonical_ids: Vec<u32> = Vec::new();
            for path in paths {
                if let Some(entries) = self.canonical_index.get(*path) {
                    canonical_ids.extend(entries.iter().map(|e| e.info.id.0));
                }
            }
            canonical_ids.sort_unstable();
            let mut sorted_paths: Vec<&str> = paths.clone();
            sorted_paths.sort_unstable();
            if !canonical_ids.contains(&bare.id.0) {
                report.push(format!(
                    "ghost-bare-winner name={} bare_id={} canonical_ids={:?} paths={:?}",
                    name, bare.id.0, canonical_ids, sorted_paths
                ));
            }
            if sorted_paths.len() > 1 {
                report.push(format!(
                    "cross-module name={} claimed_by={:?}",
                    name, sorted_paths
                ));
            }
        }
        report.sort_unstable();
        report
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

    /// Returns true iff at least one variant constructor is currently
    /// registered whose `parent_type_name` matches the given type name.
    ///

    /// Used by the variant-registration fast-path to short-circuit
    /// stdlib loading when the user has already declared a type of the
    /// same name — see `register_type_constructors`.
    pub fn has_variants_for_type(&self, type_name: &str) -> bool {
        self.functions.iter().any(|(_, info)| {
            info.variant_tag.is_some() && info.parent_type_name.as_deref() == Some(type_name)
        })
    }

    /// Removes every variant constructor whose `parent_type_name` matches
    /// the given type. A variant entry is one where `variant_tag.is_some()`.
    ///

    /// This is used when a user-defined type redeclares a name that the
    /// stdlib also defines: before the user's variants are registered,
    /// the stdlib's leftover constructor entries (qualified and simple)
    /// must be wiped so that the user's layout is the only one visible.
    ///

    /// Returns the number of entries removed.
    pub fn clear_variants_for_type(&mut self, type_name: &str) -> usize {
        let keys: Vec<String> = self
            .functions
            .iter()
            .filter(|(_, info)| {
                info.variant_tag.is_some() && info.parent_type_name.as_deref() == Some(type_name)
            })
            .map(|(k, _)| k.clone())
            .collect();
        let removed = keys.len();
        for k in keys {
            self.functions.remove(&k);
        }
        removed
    }

    /// Looks up a function by name.
    pub fn lookup_function(&self, name: &str) -> Option<&FunctionInfo> {
        self.functions.get(name)
    }

    /// **Scope-aware function lookup** (#17/#39 foundation).
    ///
    /// Probes the per-module scope index first using
    /// `(current_source_module, name)`; falls back to the bare
    /// `functions` table when no scope-specific entry is registered.
    /// Call sites that have a current compile-scope (which is most
    /// codegen sites — `current_source_module` is the dotted path of
    /// the module currently being collected/compiled) should prefer
    /// this over plain `lookup_function` to dodge cross-module
    /// first-wins shadowing.
    pub fn lookup_function_in_scope(&self, name: &str) -> Option<&FunctionInfo> {
        if let Some(scope) = &self.current_source_module
            && let Some(info) = self.scoped_functions.get(&(scope.clone(), name.to_string()))
        {
            return Some(info);
        }
        self.functions.get(name)
    }

    /// Look up a function by its globally-unique `FunctionId`.
    ///
    /// Used by the #91 pre-resolved-static-call fast path: the
    /// typechecker stamps the resolved id onto the `MethodCall` AST
    /// node, codegen retrieves the FunctionInfo here without
    /// re-deriving it through string-based name resolution.
    ///
    /// Implementation: linear scan over the function table; the
    /// fast path runs once per resolved method-call site so the
    /// O(N) cost is bounded by the call's argument-evaluation work
    /// already performed.  An id→name reverse-index is a future
    /// micro-optimisation if profiling shows this on the hot path.
    pub fn lookup_function_by_id(
        &self,
        id: crate::module::FunctionId,
    ) -> Option<&FunctionInfo> {
        self.functions.values().find(|f| f.id == id)
    }

    /// **Scope-aware arity-disambiguated lookup** (#17/#39).
    /// Probes the per-module index first when scope is known and the
    /// simple name has no qualifier, then falls back to the standard
    /// `lookup_function_with_arity` chain.  Same arity-matching
    /// discipline (primary by-name + `name#arity` alternative) applies
    /// to both the scoped and the bare lookup.
    pub fn lookup_function_with_arity_in_scope(
        &self,
        name: &str,
        arity: usize,
    ) -> Option<&FunctionInfo> {
        if let Some(scope) = &self.current_source_module
            && !name.contains('.')
            && !name.contains("::")
        {
            let key = (scope.clone(), name.to_string());
            if let Some(info) = self.scoped_functions.get(&key)
                && info.param_count == arity
            {
                return Some(info);
            }
        }
        self.lookup_function_with_arity(name, arity)
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

    /// FUNC-REGISTRY-QUALIFICATION-1 (phase 2) — qualified-aware
    /// function-key resolution.  Returns the winning `(key, info)`
    /// pair so callers (and the trace) can see WHICH registration
    /// form resolved.
    ///
    /// Resolution order:
    ///  1. exact key `name`;
    ///  2. canonical `<parent_type>.<leaf>` key (when `parent_type`
    ///     is given and `name` isn't already rooted at it);
    ///  3. arity alt-key `<name>#<arity>` (when `arity` is given);
    ///  4. qualified-suffix scan: any key ending in `.<name>` — or,
    ///     with `parent_type`, `.<parent>.<leaf>` / a
    ///     `parent_type_name`-pinned `.<leaf>` — with the
    ///     lexicographically-smallest key winning (deterministic
    ///     across rebakes, unlike a raw HashMap-order `find`).
    ///
    /// Filters applied at every stage: `arity` (when `Some`) must
    /// equal `param_count`, and the `u32::MAX` placeholder id is
    /// never a valid resolution target.  `parent_type` additionally
    /// pins stage-1 hits to entries that are actually the parent's
    /// (keyed under `<parent>.` or carrying a matching
    /// `parent_type_name`) — a bare first-wins squatter from another
    /// module falls through to the qualified stages instead of
    /// mis-binding (the bare-`Text.new`-inside-stdlib-merged-suites
    /// class).
    ///
    /// This is the shared replacement for the per-site hand-rolled
    /// "bare lookup, then walk the table for `.<Type>.<method>`"
    /// pattern; first consumer is `try_emit_display_dispatch`'s
    /// `Formatter.new` resolution in `expressions.rs`.
    ///
    /// Diagnostics: `VERUM_TRACE_FN_RESOLVE=<substr>` prints
    /// `key -> resolution` (or MISS) for every call whose `name`
    /// contains `<substr>`; empty or `1` traces all calls.
    pub fn resolve_function_key(
        &self,
        name: &str,
        arity: Option<usize>,
        parent_type: Option<&str>,
    ) -> Option<(&str, &FunctionInfo)> {
        let trace = match std::env::var("VERUM_TRACE_FN_RESOLVE") {
            Ok(f) => f.is_empty() || f == "1" || name.contains(&f),
            Err(_) => false,
        };
        let emit_hit = |stage: &str, key: &str, info: &FunctionInfo| {
            if trace {
                eprintln!(
                    "[fn-resolve] '{}' (arity={:?}, parent={:?}) -> '{}' via {} (id={}, arity={})",
                    name, arity, parent_type, key, stage, info.id.0, info.param_count,
                );
            }
        };
        let leaf = name.rsplit('.').next().unwrap_or(name);
        let accepts = |info: &FunctionInfo| -> bool {
            info.id.0 != u32::MAX && arity.map(|a| info.param_count == a).unwrap_or(true)
        };
        // Stage 1: exact key.
        if let Some((key, info)) = self.functions.get_key_value(name) {
            let parent_ok = match parent_type {
                None => true,
                Some(p) => {
                    name.starts_with(&format!("{}.", p))
                        || info.parent_type_name.as_deref() == Some(p)
                }
            };
            if parent_ok && accepts(info) {
                emit_hit("exact", key, info);
                return Some((key.as_str(), info));
            }
        }
        // Stage 2: canonical `<parent>.<leaf>` form.
        if let Some(p) = parent_type {
            let canonical = format!("{}.{}", p, leaf);
            if canonical != name
                && let Some((key, info)) = self.functions.get_key_value(&canonical)
                && accepts(info)
            {
                emit_hit("canonical", key, info);
                return Some((key.as_str(), info));
            }
        }
        // Stage 3: arity alt-key (the `name#arity` demotion slot).
        if let Some(a) = arity {
            let alt = format!("{}#{}", name, a);
            if let Some((key, info)) = self.functions.get_key_value(&alt)
                && accepts(info)
            {
                emit_hit("arity-alt", key, info);
                return Some((key.as_str(), info));
            }
        }
        // Stage 4: deterministic qualified-suffix scan.
        let dotted_suffix = format!(".{}", name);
        let parent_suffix = parent_type.map(|p| format!(".{}.{}", p, leaf));
        let leaf_suffix = format!(".{}", leaf);
        let mut best: Option<(&String, &FunctionInfo)> = None;
        for (key, info) in self.functions.iter() {
            if !accepts(info) {
                continue;
            }
            let matched = match (parent_type, &parent_suffix) {
                (Some(p), Some(ps)) => {
                    key.ends_with(ps.as_str())
                        || (info.parent_type_name.as_deref() == Some(p)
                            && key.ends_with(leaf_suffix.as_str()))
                }
                _ => key.ends_with(dotted_suffix.as_str()),
            };
            if !matched {
                continue;
            }
            let replace = match best {
                Some((best_key, _)) => key.as_str() < best_key.as_str(),
                None => true,
            };
            if replace {
                best = Some((key, info));
            }
        }
        if let Some((key, info)) = best {
            emit_hit("suffix-scan", key, info);
            return Some((key.as_str(), info));
        }
        if trace {
            eprintln!(
                "[fn-resolve] '{}' (arity={:?}, parent={:?}) -> MISS",
                name, arity, parent_type,
            );
        }
        None
    }

    /// QUALIFIED-CALL-FIRST-MATCH-1 — module-scope-authoritative
    /// resolution for a DOTTED (≥2-segment) call path.
    ///
    /// A call site that spells `darwin.time.monotonic_nanos()` inside
    /// `module sys;` names an EXPLICIT cross-module target.  The legacy
    /// resolution ladders bottomed out in a bare-LEAF lookup
    /// (`lookup_function_in_scope("monotonic_nanos")`), which prefers
    /// the CURRENT module's same-named function — the most-wrong
    /// candidate (live failure: `core.sys.monotonic_time_ns` ↔
    /// `core.sys.monotonic_nanos` mutual recursion → StackOverflow in
    /// every Bencher/sleep test).
    ///
    /// Resolution order (all stages arity-filtered, deterministic):
    ///  1. the literal spelling, both separators (`a.b.c` / `a::b::c`),
    ///     with the `#arity` alt-key probed on arity mismatch;
    ///  2. `module_aliases` head expansion (`mount X.Y.Z;` makes
    ///     `Z.f()` reach `X.Y.Z.f`), plus its `core.`-stripped twin;
    ///  3. `current_source_module`-anchored candidates, deepest anchor
    ///     first (relative-path shadowing semantics), each probed
    ///     verbatim AND `core.`-stripped (source files declare
    ///     `module sys.darwin.time;` without the `core.` root while
    ///     mod.vr scopes may carry it), plus a `core.`-PREFIXED probe
    ///     of the literal spelling;
    ///  4. ranked qualified-suffix scan: any key ending in
    ///     `.<dotted>` — ranked by the `process_import_tree` re-export
    ///     tie-break (fewest dots → module-parent before type-parent →
    ///     lexicographic) so the pick is stable across bakes.
    ///
    /// The user-written segment count is the ambiguity FLOOR: stage 4
    /// never matches on fewer segments than the call spelled, so this
    /// can never degrade into the bare-leaf first-match disease it
    /// replaces.
    ///
    /// Diagnostics: `VERUM_TRACE_QCALL=<substr>` (empty/`1` = all).
    pub fn resolve_qualified_dotted_call(
        &self,
        parts: &[String],
        arity: usize,
    ) -> Option<(String, FunctionInfo)> {
        if parts.len() < 2 {
            return None;
        }
        let dotted = parts.join(".");
        let trace = match std::env::var("VERUM_TRACE_QCALL") {
            Ok(f) => f.is_empty() || f == "1" || dotted.contains(&f),
            Err(_) => false,
        };
        let accepts = |info: &FunctionInfo| -> bool {
            info.id.0 != u32::MAX && info.param_count == arity
        };
        // Exact-key probe with the `name#arity` alt-key fallback
        // (mirrors `lookup_function_with_arity`'s discipline).
        let probe = |key: &str, stage: &str| -> Option<(String, FunctionInfo)> {
            if let Some(info) = self.functions.get(key)
                && accepts(info)
            {
                if trace {
                    eprintln!(
                        "[qcall] '{}' (arity={}) -> '{}' via {} (id={})",
                        dotted, arity, key, stage, info.id.0
                    );
                }
                return Some((key.to_string(), info.clone()));
            }
            let alt = format!("{}#{}", key, arity);
            if let Some(info) = self.functions.get(&alt)
                && accepts(info)
            {
                if trace {
                    eprintln!(
                        "[qcall] '{}' (arity={}) -> '{}' via {}-alt (id={})",
                        dotted, arity, key, stage, info.id.0
                    );
                }
                return Some((key.to_string(), info.clone()));
            }
            None
        };
        // Stage 1: literal spelling — plus its `core.`-stripped twin
        // (source files declare `module intrinsics.x;` etc. without
        // the `core.` root, so an absolute `core.`-rooted call
        // spelling maps onto a stripped registration key — the
        // `core.intrinsics.num_cpus()` sys-delegator class).
        if let Some(hit) = probe(&dotted, "exact") {
            return Some(hit);
        }
        if let Some(hit) = probe(&parts.join("::"), "exact-colon") {
            return Some(hit);
        }
        // Progressive head-strip (mirrors `resolve_core_rooted_suffix`,
        // #122): leading MODULE-shaped (lower-case) segments may be
        // spelled at the call site but absent from the registration
        // key — `core.time.Time.sleep_ms()` registers as
        // `Time.sleep_ms`, `core.intrinsics.num_cpus()` as
        // `intrinsics.num_cpus`.  Exact-key probes only (a stripped
        // spelling is less specific, so it may never enter the ranked
        // suffix scan — the floor guarantee); at least 2 segments
        // remain.
        for k in 1..parts.len().saturating_sub(1) {
            if !parts[k - 1]
                .chars()
                .next()
                .map(|c| c.is_ascii_lowercase())
                .unwrap_or(false)
            {
                break;
            }
            if let Some(hit) = probe(&parts[k..].join("."), "head-strip") {
                return Some(hit);
            }
        }
        // Stage 2: module-alias head expansion.
        if let Some(full) = self.module_aliases.get(parts[0].as_str()) {
            let mut expanded = full.clone();
            expanded.extend(parts[1..].iter().cloned());
            let exp = expanded.join(".");
            if let Some(hit) = probe(&exp, "alias") {
                return Some(hit);
            }
            if let Some(stripped) = exp.strip_prefix("core.")
                && let Some(hit) = probe(stripped, "alias-core-strip")
            {
                return Some(hit);
            }
        }
        // Stage 3: current-module-anchored candidates, deepest first.
        if let Some(cur) = self.current_source_module.as_deref()
            && !cur.is_empty()
            && cur != "main"
        {
            let segs: Vec<&str> = cur.split('.').collect();
            for depth in (1..=segs.len()).rev() {
                let anchor = segs[..depth].join(".");
                let candidate = format!("{}.{}", anchor, dotted);
                if let Some(hit) = probe(&candidate, "anchored") {
                    return Some(hit);
                }
                if let Some(stripped) = candidate.strip_prefix("core.")
                    && let Some(hit) = probe(stripped, "anchored-core-strip")
                {
                    return Some(hit);
                }
            }
        }
        if let Some(hit) = probe(&format!("core.{}", dotted), "core-prefixed") {
            return Some(hit);
        }
        // Stage 4: ranked qualified-suffix scan (floor = the user's
        // written segment count; for `core.`-rooted spellings the
        // stripped form keeps the floor at the meaningful segments).
        // Tie-break mirrors the process_import_tree re-export ranking
        // exactly.
        let scan_target: &str = match dotted.strip_prefix("core.") {
            Some(stripped) if stripped.contains('.') => stripped,
            _ => dotted.as_str(),
        };
        let suffix = format!(".{}", scan_target);
        let parent_is_module = |k: &str| -> bool {
            let segs: Vec<&str> = k.split('.').collect();
            segs.len() >= 2
                && segs[segs.len() - 2]
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_lowercase())
                    .unwrap_or(false)
        };
        let mut best: Option<(&String, &FunctionInfo)> = None;
        for (key, info) in self.functions.iter() {
            if !key.ends_with(suffix.as_str()) || !accepts(info) {
                continue;
            }
            let better = match best {
                None => true,
                Some((bk, _)) => {
                    let (da, db) = (key.matches('.').count(), bk.matches('.').count());
                    da < db
                        || (da == db
                            && parent_is_module(key)
                            && !parent_is_module(bk))
                        || (da == db
                            && parent_is_module(key) == parent_is_module(bk)
                            && key.as_str() < bk.as_str())
                }
            };
            if better {
                best = Some((key, info));
            }
        }
        if let Some((key, info)) = best {
            if trace {
                eprintln!(
                    "[qcall] '{}' (arity={}) -> '{}' via suffix-rank (id={})",
                    dotted, arity, key, info.id.0
                );
            }
            return Some((key.clone(), info.clone()));
        }
        if trace {
            eprintln!("[qcall] '{}' (arity={}) -> MISS", dotted, arity);
        }
        None
    }

    /// Search for a function whose name ends with the given suffix.
    /// Used to find qualified variant names (e.g., "Option.None") when
    /// the simple name is not registered due to collision.
    ///

    /// When multiple matches exist (e.g., "Ordering.Lt" and "GeneralCategory.Lt"),
    /// prefers the one whose parent_type_name matches the current function's return
    /// type. If no return type context is available, returns the match only if unique.
    pub fn find_function_by_suffix(&self, suffix: &str) -> Option<&FunctionInfo> {
        let mut keyed: Vec<(&String, &FunctionInfo)> = Vec::new();
        for (name, info) in &self.functions {
            if name.ends_with(suffix) {
                keyed.push((name, info));
            }
        }
        // ARCH-P2: the disambiguation loops below return the FIRST
        // parent==base entry — hash order made the pick a per-bake dice
        // when a parent has both a bare and a module-rooted registration
        // with distinct ids. Canonical order: key ascending.
        keyed.sort_by(|a, b| a.0.cmp(b.0));
        let matches: Vec<&FunctionInfo> = keyed.into_iter().map(|(_, i)| i).collect();

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
            // Inner-generic check: when we're in `fn f() -> Result<X, E>`,
            // a variant whose parent matches `E` is a valid pick — the
            // variant is being constructed as the Err payload.  Without
            // this, `Err(IoError(...))` inside a `Result<_, ConnectionError>`
            // returner can't disambiguate between
            // `ConnectionError.IoError(Text)` and a same-named unit variant
            // elsewhere.  Closes the IoError-arity stdlib bug class.
            if let Some(ref inner) = self.current_return_type_inner {
                for inner_name in inner {
                    let base = inner_name.split('<').next().unwrap_or(inner_name.as_str());
                    for info in &matches {
                        if info.parent_type_name.as_deref() == Some(base) {
                            return Some(info);
                        }
                    }
                }
            }
            // Also try the match_scrutinee_type for pattern matching context
            if let Some(ref scrutinee_type) = self.match_scrutinee_type {
                let base = scrutinee_type
                    .split('<')
                    .next()
                    .unwrap_or(scrutinee_type.as_str());
                for info in &matches {
                    if info.parent_type_name.as_deref() == Some(base) {
                        return Some(info);
                    }
                }
            }
            // Prefer variants from user-defined types over stdlib types
            let user_matches: Vec<_> = matches
                .iter()
                .filter(|info| {
                    info.parent_type_name
                        .as_ref()
                        .map(|p| self.user_defined_types.contains(p))
                        .unwrap_or(false)
                })
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
        // ARCH-P2: keyed + sorted — the first-parent-match loops below
        // inherited hash order (per-bake MakeVariant tag dice when one
        // parent carries duplicate registrations). Canonical: key order.
        let mut keyed: Vec<(&String, u32, Option<String>)> = Vec::new();
        for (fn_name, fn_info) in &self.functions {
            if fn_name.ends_with(&suffix)
                && fn_info.param_count == arg_count
                && let Some(tag) = fn_info.variant_tag
            {
                keyed.push((fn_name, tag, fn_info.parent_type_name.clone()));
            }
        }
        keyed.sort_by(|a, b| a.0.cmp(b.0));
        let matches: Vec<(u32, Option<String>)> =
            keyed.into_iter().map(|(_, t, p)| (t, p)).collect();
        if matches.len() == 1 {
            return Some(matches[0].0);
        }
        if matches.len() > 1 {
            // Disambiguate using the enclosing function's declared return type.
            // Constructing `Ok(x)` inside `fn f() -> Result<T, E>` must resolve
            // to `Result.Ok`, not some unrelated `Foo.Ok` that happens to share
            // the simple name (which the collision logic stripped from the
            // direct lookup table). Mirrors find_function_by_suffix's logic so
            // both call sites converge on the same disambiguation discipline.
            if let Some(ref ret_type) = self.current_return_type_name {
                let base = ret_type.split('<').next().unwrap_or(ret_type.as_str());
                for (tag, parent) in &matches {
                    if parent.as_deref() == Some(base) {
                        return Some(*tag);
                    }
                }
            }
            // Inner-generic check: cover `Err(IoError(msg))` inside
            // `fn f() -> Result<X, ConnectionError>` — the variant we want
            // (`ConnectionError.IoError(Text)`) has its parent appear as
            // an inner generic of the return type, not as the return type
            // itself.  Tries each inner name in declaration order; first
            // match wins.  Closes the IoError-arity stdlib bug class.
            if let Some(ref inner) = self.current_return_type_inner {
                for inner_name in inner {
                    let base = inner_name.split('<').next().unwrap_or(inner_name.as_str());
                    for (tag, parent) in &matches {
                        if parent.as_deref() == Some(base) {
                            return Some(*tag);
                        }
                    }
                }
            }
            // Then the match scrutinee type (covers `match x { Ok(_) => ... }`
            // when the constructor pattern itself produces a value, e.g. via
            // a guard expression that re-constructs).
            if let Some(ref scrutinee_type) = self.match_scrutinee_type {
                let base = scrutinee_type
                    .split('<')
                    .next()
                    .unwrap_or(scrutinee_type.as_str());
                for (tag, parent) in &matches {
                    if parent.as_deref() == Some(base) {
                        return Some(*tag);
                    }
                }
            }
            // Last: prefer user-defined types over stdlib types — kept as a
            // weak final fallback so user code still wins when both contexts
            // above are absent.
            let user_matches: Vec<u32> = matches
                .iter()
                .filter(|(_, parent)| {
                    parent
                        .as_ref()
                        .map(|p| self.user_defined_types.contains(p))
                        .unwrap_or(false)
                })
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
    /// Position of `param` in `type_name`'s declared generic-parameter
    /// list (`("ControlFlow", "C")` → `Some(1)`). See
    /// [`Self::type_generic_params`].
    pub fn generic_param_position(&self, type_name: &str, param: &str) -> Option<usize> {
        self.type_generic_params
            .get(type_name)
            .and_then(|params| params.iter().position(|p| p == param))
    }

    pub fn find_variant_by_type_and_name(
        &self,
        type_name: &str,
        variant_name: &str,
    ) -> Option<u32> {
        let qualified = format!("{}.{}", type_name, variant_name);
        self.functions
            .get(&qualified)
            .and_then(|info| info.variant_tag)
    }

    /// Find the declared payload types for a variant looked up by
    /// `(type_name, variant_name)` — used by the construction-side
    /// payload-type propagation at
    /// `compile_variant_constructor_hinted` to push the per-arg
    /// `current_return_type_name` context into nested
    /// `compile_expr` calls (closes nested case of task #22).
    ///
    /// Returns `None` when the qualified `<type>.<variant>` entry
    /// doesn't exist or has no payload-types vector — the caller
    /// then proceeds without per-arg context, identical to pre-fix
    /// behaviour.
    pub fn find_variant_payload_types_by_type_and_name(
        &self,
        type_name: &str,
        variant_name: &str,
    ) -> Option<Vec<String>> {
        let qualified = format!("{}.{}", type_name, variant_name);
        self.functions
            .get(&qualified)
            .and_then(|info| info.variant_payload_types.clone())
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
                && !parents.contains(parent)
            {
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
            if fn_name.ends_with(&suffix)
                && fn_info.variant_tag.is_some()
                && let Some(ref parent) = fn_info.parent_type_name
                && !parents.contains(parent)
            {
                parents.push(parent.clone());
            }
        }
        // Also check if variant_name itself is a registered variant
        if parents.is_empty() {
            for fn_info in self.functions.values() {
                if fn_info.variant_tag.is_some()
                    && let Some(ref parent) = fn_info.parent_type_name
                    && parent == variant_name
                {
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
    pub fn find_variant_with_suffix(
        &self,
        suffix: &str,
        expected_param_count: usize,
    ) -> Option<&FunctionInfo> {
        // ARCH-P2: first-match over the HashMap walk was a per-bake dice
        // — with colliding suffixes (the doc's IpAddr.V6 vs SocketAddr.V6
        // case) the picked constructor flipped MakeVariant tag + payload
        // layout across bakes. Deterministic rule: lexicographically-
        // smallest matching key wins (mirrors the min-by-key scans at
        // resolve_function_key stage 4 and the field-index scans).
        let mut best: Option<(&String, &FunctionInfo)> = None;
        for (name, info) in &self.functions {
            // Only consider variant constructors (must have variant_tag set)
            if info.variant_tag.is_some()
                && name.ends_with(suffix)
                && info.param_count == expected_param_count
                && info.variant_payload_types.is_some()
                && best.map(|(b, _)| name < b).unwrap_or(true)
            {
                best = Some((name, info));
            }
        }
        best.map(|(_, info)| info)
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
            reference_bindings: self.reference_bindings.clone(),
            object_ref_param_regs: self.object_ref_param_regs.clone(),
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
        self.reference_bindings = saved.reference_bindings;
        self.object_ref_param_regs = saved.object_ref_param_regs;
    }

    /// Looks up a function by qualified name (e.g., "module::function" or "Type::method").
    ///

    /// Returns a match for the *exact* qualified name only. Callers that also
    /// want a simple-name fallback (for module-style imports where only the
    /// last segment is registered) must request it explicitly via
    /// `lookup_qualified_function_with_fallback`. The strict default prevents
    /// a whole class of silent rebinding bugs, for example:
    ///

    /// ```text
    /// // core/mem/epoch.vr — free function
    /// public fn current_epoch() -> UInt64 { … }
    ///

    /// // core/runtime/mod.vr — static method on a unit type
    /// implement Runtime { public fn current_epoch() -> UInt32 { … } }
    /// ```
    ///

    /// Here `Runtime::current_epoch` is not registered (the method is stored
    /// as `Runtime.current_epoch`); if the qualified lookup silently returned
    /// the bare `current_epoch`, every caller of `Runtime.current_epoch()`
    /// would compile to a self-recursive call on the free function, blowing
    /// the stack. Same shape as the earlier super/cog/relative regression.
    pub fn lookup_qualified_function(&self, qualified_name: &str) -> Option<&FunctionInfo> {
        self.functions.get(qualified_name)
    }

    /// Looks up a qualified name, falling back to the last segment if the
    /// exact name is not registered. Intended only for explicit module-style
    /// imports (e.g. `mount io.print` brings "print" into scope and later
    /// `io.print("hello")` should resolve to it). Never use this for method
    /// dispatch — see the doc on `lookup_qualified_function` for the
    /// regression this avoids.
    pub fn lookup_qualified_function_with_fallback(
        &self,
        qualified_name: &str,
    ) -> Option<&FunctionInfo> {
        if let Some(info) = self.functions.get(qualified_name) {
            return Some(info);
        }

        // Refuse the fallback when the qualified path is rooted at a module-
        // path keyword (`super`, `cog`, `.`): those are explicit cross-module
        // references, not aliases.
        let is_rooted_module_path = qualified_name.starts_with("super::")
            || qualified_name.starts_with("super.")
            || qualified_name.starts_with("cog::")
            || qualified_name.starts_with("cog.")
            || qualified_name.starts_with(".::")
            || qualified_name.starts_with("..");
        if is_rooted_module_path {
            return None;
        }

        if let Some(simple_name) = qualified_name.rsplit("::").next()
            && let Some(info) = self.functions.get(simple_name)
        {
            return Some(info);
        }

        None
    }

    /// Imports all functions from a pre-compiled module's registry.
    ///

    /// This is used during stdlib compilation to make functions from
    /// previously compiled modules (e.g., core) available when compiling
    /// dependent modules (e.g., collections, async).
    pub fn import_functions(
        &mut self,
        functions: &std::collections::HashMap<String, FunctionInfo>,
    ) {
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
        // ARCH-P2 stage 1 — the canonical index is DUAL-keyed with
        // `functions` and shares its lifetime: clear it at the same
        // per-module reset boundary so a fresh module never inherits
        // the previous module's divergence corpus.
        self.canonical_index.clear();
        // Stage-3 stub-name preservation map — cleared at the per-
        // module reset boundary along with `functions`.  Accumulates
        // stub_id -> name mappings during a single module's compile
        // walk; consulted by `emit_stage3_stub_descriptors` at
        // build_module time to recover stub-ids whose bare-name slot
        // got overwritten by a real-id `FunctionInfo` (the cascade
        // root cause for orphan `[lenient] stage-3 ... stub never
        // resolved` panics — see field doc on `stage3_stub_names`).
        self.stage3_stub_names.clear();
        self.stats = CodegenStats::default();
        self.variable_types.clear();
        if !self.variable_type_names.is_empty() {
            self.last_function_variable_types = self.variable_type_names.clone();
        }
        self.variable_type_names.clear();
        self.generic_type_params.clear();
        self.generic_type_params_ordered.clear();
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
        self.context_aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Resolve `type Foo is Bar;` alias to its canonical target.
    ///
    /// The codegen-side type-alias registry isn't populated yet;
    /// returning the input verbatim is a no-op fallback that keeps
    /// callers like `expressions.rs::is_type_ns` correct (`resolved
    /// != *first` evaluates false → alias-arm of the OR doesn't
    /// fire, primary `has_functions_with_prefix` still works).
    /// When the codegen alias map is wired in a future commit this
    /// method becomes the canonical resolver.
    pub fn resolve_type_alias(&self, name: &str) -> String {
        name.to_string()
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
        self.variable_types
            .get(name)
            .copied()
            .unwrap_or(VarTypeKind::Unknown)
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
    ///
    /// FN-LOCAL-STATIC-ONCE-1: mirrors `is_thread_local`'s scope-first
    /// probe — a fn-local static's primitive discriminator is
    /// registered under its hoisted `<fn>$static$<name>` key, so the
    /// enclosing body's instruction selection must resolve through the
    /// same mangling before falling back to the bare name.
    pub fn get_constant_type(&self, name: &str) -> VarTypeKind {
        if !name.contains('$')
            && let Some(cf) = self.current_function.as_deref()
        {
            let mangled = format!("{}$static${}", cf, name);
            if let Some(&vt) = self.constant_types.get(&mangled) {
                return vt;
            }
            let bare = cf.rsplit('.').next().unwrap_or(cf);
            if bare != cf {
                let mangled = format!("{}$static${}", bare, name);
                if let Some(&vt) = self.constant_types.get(&mangled) {
                    return vt;
                }
            }
        }
        self.constant_types
            .get(name)
            .copied()
            .unwrap_or(VarTypeKind::Unknown)
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

        ctx.begin_function(
            "test_fn",
            &[("a".to_string(), false), ("b".to_string(), false)],
            None,
        );

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

    // ───────────────────────────────────────────────────────────────
    // #118 — TierContext merge regression tests.
    //

    // Pre-#118 the compiler pipeline merged per-function tier
    // analyses with `for i in 0..decision_count() { dst.set_tier(
    // ExprId(i), src.get_tier(ExprId(i))) }`, which always missed
    // the span-encoded keys produced by `from_analysis_result` and
    // silently inserted only `default_tier` (Tier0). These tests
    // pin the canonical `iter_decisions()` / `merge_from()` API so
    // a future regression in the per-function aggregator surfaces
    // here instead of as silent runtime CBGR overhead in user code.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn iter_decisions_returns_canonical_expr_id_keys() {
        let mut tc = TierContext::new();
        tc.enabled = true;
        let span_id_a = ExprId(((42u64) << 32) | 47u64);
        let span_id_b = ExprId(((100u64) << 32) | 110u64);
        tc.set_tier(span_id_a, CbgrTier::Tier1);
        tc.set_tier(span_id_b, CbgrTier::Tier0);

        let collected: Vec<_> = tc.iter_decisions().collect();
        assert_eq!(collected.len(), 2);
        // Keys must be the original span-encoded ExprIds, not 0..N.
        let ids: Vec<u64> = collected.iter().map(|(id, _)| id.0).collect();
        assert!(ids.contains(&span_id_a.0));
        assert!(ids.contains(&span_id_b.0));
    }

    #[test]
    fn merge_from_preserves_span_keys_and_tiers() {
        let mut src = TierContext::new();
        src.enabled = true;
        let span_a = ExprId(((42u64) << 32) | 47u64);
        let span_b = ExprId(((100u64) << 32) | 110u64);
        src.set_tier(span_a, CbgrTier::Tier1);
        src.set_tier(span_b, CbgrTier::Tier2);

        let mut dst = TierContext::new();
        dst.enabled = true;
        dst.merge_from(&src);

        // Both decisions land at their original span-encoded ExprIds —
        // the bug was inserting at ExprId(0..N) instead.
        assert_eq!(dst.get_tier(span_a), CbgrTier::Tier1);
        assert_eq!(dst.get_tier(span_b), CbgrTier::Tier2);
        // Querying via the buggy ExprId(0) / ExprId(1) should NOT
        // yield Tier1/Tier2 — those slots only carry the default.
        assert_eq!(dst.get_tier(ExprId(0)), CbgrTier::Tier0);
        assert_eq!(dst.get_tier(ExprId(1)), CbgrTier::Tier0);
    }

    #[test]
    fn merge_from_disjoint_function_unions_decisions() {
        // Each "function" produces a TierContext whose ExprIds are
        // span-disjoint from every other function — the realistic
        // pipeline pattern. Aggregator must contain the union.
        let mut fn_a = TierContext::new();
        fn_a.enabled = true;
        fn_a.set_tier(ExprId(((10u64) << 32) | 12), CbgrTier::Tier1);
        fn_a.set_tier(ExprId(((14u64) << 32) | 16), CbgrTier::Tier1);

        let mut fn_b = TierContext::new();
        fn_b.enabled = true;
        fn_b.set_tier(ExprId(((30u64) << 32) | 32), CbgrTier::Tier2);

        let mut agg = TierContext::new();
        agg.enabled = true;
        agg.merge_from(&fn_a);
        agg.merge_from(&fn_b);

        assert_eq!(agg.decision_count(), 3);
        assert_eq!(agg.get_tier(ExprId(((10u64) << 32) | 12)), CbgrTier::Tier1);
        assert_eq!(agg.get_tier(ExprId(((14u64) << 32) | 16)), CbgrTier::Tier1);
        assert_eq!(agg.get_tier(ExprId(((30u64) << 32) | 32)), CbgrTier::Tier2);
    }

    #[test]
    fn merge_from_preserves_tier0_reasons() {
        let mut src = TierContext::new();
        src.enabled = true;
        let span = ExprId(((42u64) << 32) | 47u64);
        src.set_tier_with_reason(span, CbgrTier::Tier0, Some(Tier0Reason::Escapes));

        let mut dst = TierContext::new();
        dst.enabled = true;
        dst.merge_from(&src);

        assert_eq!(dst.get_tier(span), CbgrTier::Tier0);
        assert_eq!(dst.get_tier0_reason(span), Some(Tier0Reason::Escapes));
    }

    /// Construct a minimal variant-constructor `FunctionInfo` for
    /// disambiguator tests.  Only the fields the disambiguator
    /// actually consults (param_count, variant_tag, parent_type_name)
    /// need to be meaningful; everything else uses `Default`.
    fn variant_info(parent: &str, tag: u32, arity: usize) -> FunctionInfo {
        FunctionInfo {
            param_count: arity,
            variant_tag: Some(tag),
            parent_type_name: Some(parent.to_string()),
            ..Default::default()
        }
    }

    /// Construct a minimal plain-function `FunctionInfo` for the
    /// FUNC-REGISTRY-QUALIFICATION-1 tests.
    fn plain_info(id: u32, arity: usize) -> FunctionInfo {
        FunctionInfo {
            id: FunctionId(id),
            param_count: arity,
            ..Default::default()
        }
    }

    /// FUNC-REGISTRY-QUALIFICATION-1 (phase 2): a bare registration
    /// under an active source-module scope mirrors into the qualified
    /// `<module>.<name>` key; the qualified slot is first-wins and a
    /// later same-name registration from another scope gets its OWN
    /// qualified key without disturbing the first.
    #[test]
    fn register_function_mirrors_module_qualified_key() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.iter.range".to_string());
        ctx.register_function("range".to_string(), plain_info(7, 1));

        assert!(ctx.lookup_function("range").is_some());
        let q = ctx
            .lookup_function("core.iter.range.range")
            .expect("bare registration must mirror the qualified key");
        assert_eq!(q.id.0, 7);

        // Second same-name registration from a different scope: its
        // qualified key lands under ITS module; the first module's
        // qualified slot is untouched (first-wins, never replaced).
        ctx.current_source_module = Some("core.rand".to_string());
        ctx.register_function("range".to_string(), plain_info(9, 1));
        assert_eq!(ctx.lookup_function("core.iter.range.range").unwrap().id.0, 7);
        assert_eq!(ctx.lookup_function("core.rand.range").unwrap().id.0, 9);

        // No scope / "main" scope: no mirror.
        let mut ctx2 = CodegenContext::new();
        ctx2.register_function("free_fn".to_string(), plain_info(1, 0));
        assert!(ctx2.functions.keys().all(|k| !k.contains('.')));
        ctx2.current_source_module = Some("main".to_string());
        ctx2.register_function("other_fn".to_string(), plain_info(2, 0));
        assert!(ctx2.lookup_function("main.other_fn").is_none());
    }

    /// The qualified mirror must skip variant constructors (their
    /// canonical qualified form is `<ParentType>.<Variant>`; a
    /// module-qualified alias would double-count them in the
    /// `.{variant}` suffix-scan disambiguators) and dotted /
    /// already-qualified names.
    #[test]
    fn qualified_mirror_skips_variants_and_dotted_names() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.base.ordering".to_string());

        ctx.register_function("Lt".to_string(), variant_info("Ordering", 0, 0));
        assert!(ctx.lookup_function("core.base.ordering.Lt").is_none());

        ctx.register_function("Ordering.cmp".to_string(), plain_info(4, 2));
        assert!(ctx
            .lookup_function("core.base.ordering.Ordering.cmp")
            .is_none());

        // The suffix-scan disambiguator that motivated the variant
        // exclusion still sees exactly one `.Lt` entry after a
        // qualified `Ordering.Lt` registration.
        ctx.register_function("Ordering.Lt".to_string(), variant_info("Ordering", 0, 0));
        assert_eq!(ctx.find_variant_by_suffix_and_args("Lt", 0), Some(0));
    }

    // ----------------------------------------------------------------
    // ARCH-P2 stage 1 — content-addressed canonical function index
    // (dual keying, warn-on-divergence).
    // ----------------------------------------------------------------

    /// Identical content registered twice under one qualified path
    /// collapses to ONE canonical entry (idempotent re-registration:
    /// the fingerprint covers the signature surface, not the id).
    #[test]
    fn canonical_index_idempotent_for_identical_content() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.io.file".to_string());
        ctx.register_function("open".to_string(), plain_info(3, 2));
        ctx.register_function("open".to_string(), plain_info(3, 2));

        let entries = ctx
            .canonical_index
            .get("core.io.file.open")
            .expect("canonical entry under the qualified path");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].info.id.0, 3);
        assert!(entries[0].body_fingerprint.is_none());
        assert!(ctx.canonical_divergences().is_empty());
    }

    /// Two DIFFERENT contents (arity 1 vs arity 2) claiming one
    /// canonical path = a divergence: both fingerprints retained,
    /// `canonical_divergences` reports the path.
    #[test]
    fn canonical_index_divergence_on_different_content() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.net.addr".to_string());
        ctx.register_function("parse".to_string(), plain_info(1, 1));
        ctx.register_function("parse".to_string(), plain_info(2, 2));

        let entries = ctx.canonical_index.get("core.net.addr.parse").unwrap();
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].fingerprint, entries[1].fingerprint);
        assert_eq!(
            ctx.canonical_divergences(),
            vec![("core.net.addr.parse", 2)]
        );
    }

    /// `name#arity` alt-key mirrors — whether created internally by
    /// the collision branches or registered directly — never land in
    /// the canonical index.
    #[test]
    fn canonical_index_excludes_arity_mirrors() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.sys.fs".to_string());
        ctx.register_function("write".to_string(), plain_info(1, 2));
        ctx.register_function("write".to_string(), plain_info(2, 3));

        // The bare table grew its arity mirrors (dice-8 discipline)…
        assert!(ctx.functions.contains_key("write#2"));
        assert!(ctx.functions.contains_key("write#3"));
        // …the canonical index did not.
        assert!(ctx.canonical_index.keys().all(|k| !k.contains('#')));
        // Defensive: a direct registration OF a #-key is excluded too.
        ctx.register_function("direct#4".to_string(), plain_info(9, 4));
        assert!(ctx.canonical_index.keys().all(|k| !k.contains('#')));
    }

    /// The canonical path derivation mirrors the `compile_function`
    /// descriptor-name promotion: no scope / `main` scope keep the
    /// bare name; dotted and `$`-nested names are never re-prefixed.
    #[test]
    fn canonical_path_matches_descriptor_promotion_rules() {
        let mut ctx = CodegenContext::new();
        // No scope: the bare name IS the canonical path.
        ctx.register_function("standalone".to_string(), plain_info(1, 0));
        assert!(ctx.canonical_index.contains_key("standalone"));
        // "main" scope (single-file user compile): no promotion.
        ctx.current_source_module = Some("main".to_string());
        ctx.register_function("user_fn".to_string(), plain_info(2, 0));
        assert!(ctx.canonical_index.contains_key("user_fn"));
        assert!(!ctx.canonical_index.contains_key("main.user_fn"));
        // Dotted + $-nested names stay as-is even under a real scope.
        ctx.current_source_module = Some("core.iter".to_string());
        ctx.register_function("Iterator.next".to_string(), plain_info(3, 1));
        assert!(ctx.canonical_index.contains_key("Iterator.next"));
        ctx.register_function("outer$inner".to_string(), plain_info(4, 0));
        assert!(ctx.canonical_index.contains_key("outer$inner"));
        assert!(!ctx.canonical_index.contains_key("core.iter.outer$inner"));
    }

    /// An authoritative (explicit-mount) registration is a primary
    /// registration: it lands canonically like any other.
    #[test]
    fn canonical_index_covers_authoritative_registration() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.async.future".to_string());
        ctx.register_function_authoritative(
            "select".to_string(),
            plain_info(21, 2),
        );

        let entries = ctx
            .canonical_index
            .get("core.async.future.select")
            .expect("authoritative registration must land canonically");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].info.id.0, 21);
        // Its #arity mirror stays out of the canonical index.
        assert!(ctx.functions.contains_key("select#2"));
        assert!(ctx.canonical_index.keys().all(|k| !k.contains('#')));
    }

    /// The consistency sweep finds (a) cross-module claims — two
    /// canonical paths for one simple name — and (b) ghost bare
    /// winners — a bare-slot id no canonical entry knows.
    #[test]
    fn canonical_vs_bare_report_finds_ghost_and_cross_module() {
        let mut ctx = CodegenContext::new();
        // Module A registers foo (id 1); module B registers foo
        // (id 2, same arity) and last-wins takes the bare slot.
        ctx.current_source_module = Some("mod.a".to_string());
        ctx.register_function("foo".to_string(), plain_info(1, 1));
        ctx.current_source_module = Some("mod.b".to_string());
        ctx.register_function("foo".to_string(), plain_info(2, 1));

        let report = ctx.canonical_vs_bare_report();
        assert!(
            report
                .iter()
                .any(|l| l.starts_with("cross-module name=foo")),
            "expected a cross-module finding, got: {report:?}"
        );
        // The bare winner (id 2) IS canonically known — no ghost yet.
        assert!(
            !report.iter().any(|l| l.starts_with("ghost-bare-winner")),
            "no ghost expected while the bare id is canonical: {report:?}"
        );

        // A writer bypasses primary registration and claims the bare
        // slot with an id no canonical entry has seen.
        ctx.functions.insert("foo".to_string(), plain_info(99, 1));
        let report = ctx.canonical_vs_bare_report();
        assert!(
            report.iter().any(|l| l
                .starts_with("ghost-bare-winner name=foo")
                && l.contains("bare_id=99")),
            "expected a ghost-bare-winner finding, got: {report:?}"
        );
    }

    /// Emit-path enrichment back-fills `body_fingerprint` on the
    /// canonical entry (matched by id); unknown paths/ids are a
    /// traced no-op, never a spurious entry.
    #[test]
    fn enrich_canonical_body_fingerprint_backfills_entry() {
        let mut ctx = CodegenContext::new();
        ctx.current_source_module = Some("core.text.text".to_string());
        ctx.register_function("trim".to_string(), plain_info(5, 1));
        assert!(
            ctx.canonical_index["core.text.text.trim"][0]
                .body_fingerprint
                .is_none()
        );

        ctx.enrich_canonical_body_fingerprint(
            "core.text.text.trim",
            FunctionId(5),
            12,
            4,
        );
        let entry = &ctx.canonical_index["core.text.text.trim"][0];
        assert!(entry.body_fingerprint.is_some());

        // Bare-name fallback: registration happened without the scope
        // the descriptor promotion later saw.
        let mut ctx2 = CodegenContext::new();
        ctx2.register_function("helper".to_string(), plain_info(7, 0));
        ctx2.enrich_canonical_body_fingerprint(
            "core.late.scope.helper",
            FunctionId(7),
            3,
            1,
        );
        assert!(ctx2.canonical_index["helper"][0].body_fingerprint.is_some());

        // Unknown path+id: no panic, no spurious entry.
        ctx2.enrich_canonical_body_fingerprint(
            "core.late.scope.nope",
            FunctionId(77),
            1,
            1,
        );
        assert!(!ctx2.canonical_index.contains_key("core.late.scope.nope"));
        assert!(!ctx2.canonical_index.contains_key("nope"));
    }

    /// `resolve_function_key`: exact key wins when present and
    /// arity-compatible; otherwise the deterministic qualified-suffix
    /// scan recovers the module-qualified registration.
    #[test]
    fn resolve_function_key_exact_then_qualified_scan() {
        let mut ctx = CodegenContext::new();
        // Archive shape: only the module-qualified key exists.
        ctx.register_function(
            "core.base.protocols.Formatter.new".to_string(),
            plain_info(11, 1),
        );

        let (key, info) = ctx
            .resolve_function_key("Formatter.new", Some(1), Some("Formatter"))
            .expect("suffix scan must resolve the qualified Formatter.new");
        assert_eq!(key, "core.base.protocols.Formatter.new");
        assert_eq!(info.id.0, 11);

        // Arity filter: no 3-arg Formatter.new anywhere.
        assert!(ctx
            .resolve_function_key("Formatter.new", Some(3), Some("Formatter"))
            .is_none());

        // Deterministic pick: with TWO qualified matches the
        // lexicographically-smallest key wins.
        ctx.register_function("zzz.fmt.Formatter.new".to_string(), plain_info(13, 1));
        let (key, info) = ctx
            .resolve_function_key("Formatter.new", Some(1), Some("Formatter"))
            .unwrap();
        assert_eq!(key, "core.base.protocols.Formatter.new");
        assert_eq!(info.id.0, 11);

        // Exact key wins once the bare form exists.
        ctx.register_function("Formatter.new".to_string(), plain_info(12, 1));
        let (key, info) = ctx
            .resolve_function_key("Formatter.new", Some(1), Some("Formatter"))
            .unwrap();
        assert_eq!(key, "Formatter.new");
        assert_eq!(info.id.0, 12);
    }

    /// `resolve_function_key` with a parent pin must NOT resolve to a
    /// bare same-name squatter from an unrelated module (the bare
    /// `Text.new`-mis-bind class) — it falls through to the qualified
    /// scan and finds the parent's real entry.
    #[test]
    fn resolve_function_key_parent_pin_skips_bare_squatter() {
        let mut ctx = CodegenContext::new();
        // Bare `new` squatter (0-ary, unrelated module won first-wins).
        ctx.register_function("new".to_string(), plain_info(3, 0));
        // The real ctor, qualified-only.
        ctx.register_function(
            "core.base.protocols.Formatter.new".to_string(),
            plain_info(11, 1),
        );

        let (key, info) = ctx
            .resolve_function_key("new", Some(1), Some("Formatter"))
            .expect("parent-pinned scan must recover the qualified ctor");
        assert_eq!(key, "core.base.protocols.Formatter.new");
        assert_eq!(info.id.0, 11);

        // Placeholder-id entries are never resolution targets.
        let mut ctx2 = CodegenContext::new();
        ctx2.register_function("Widget.make".to_string(), plain_info(u32::MAX, 1));
        assert!(ctx2
            .resolve_function_key("Widget.make", Some(1), Some("Widget"))
            .is_none());
    }

    /// QUALIFIED-CALL-FIRST-MATCH-1 — the monotonic mutual-recursion
    /// repro: `darwin.time.monotonic_nanos()` inside `module sys;`
    /// (scoped `core.sys`) must resolve to the qualified
    /// `sys.darwin.time.monotonic_nanos`, NEVER to the current
    /// module's own bare `monotonic_nanos`.
    #[test]
    fn resolve_qualified_dotted_call_never_binds_bare_leaf() {
        let mut ctx = CodegenContext::new();
        // Current module's own fn owns the bare + scoped slots (the
        // pre-fix mis-bind target).
        ctx.current_source_module = Some("core.sys".to_string());
        ctx.register_function("monotonic_nanos".to_string(), plain_info(1363, 0));
        // The real cross-module target, registered under its
        // module-decl-rooted path (no `core.` prefix — source files
        // declare `module sys.darwin.time;`).
        ctx.register_function(
            "sys.darwin.time.monotonic_nanos".to_string(),
            plain_info(3306, 0),
        );

        let parts: Vec<String> = ["darwin", "time", "monotonic_nanos"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (key, info) = ctx
            .resolve_qualified_dotted_call(&parts, 0)
            .expect("anchored resolution must find the qualified target");
        assert_eq!(key, "sys.darwin.time.monotonic_nanos");
        assert_eq!(info.id.0, 3306);

        // A dotted path that matches nothing must MISS (the call site
        // then synthesizes a stage-5 stub) — never fall to the bare
        // leaf.
        let missing: Vec<String> = ["darwin", "time", "no_such_fn"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(ctx.resolve_qualified_dotted_call(&missing, 0).is_none());

        // Arity floor: a 2-arg call cannot bind the 0-arg target.
        assert!(ctx.resolve_qualified_dotted_call(&parts, 2).is_none());
    }

    /// QUALIFIED-CALL-FIRST-MATCH-1 — module-alias head expansion and
    /// the deterministic suffix ranking (fewest dots → module-parent
    /// before type-parent → lexicographic).
    #[test]
    fn resolve_qualified_dotted_call_alias_and_ranked_suffix() {
        let mut ctx = CodegenContext::new();
        // `mount core.sys.bitfield;` installs the module alias.
        ctx.module_aliases.insert(
            "bitfield".to_string(),
            vec!["core".to_string(), "sys".to_string(), "bitfield".to_string()],
        );
        // Registration is `core.`-stripped (module decl `sys.bitfield;`).
        ctx.register_function(
            "sys.bitfield.test_bit".to_string(),
            plain_info(42, 2),
        );
        let parts: Vec<String> = ["bitfield", "test_bit"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (key, info) = ctx
            .resolve_qualified_dotted_call(&parts, 2)
            .expect("alias expansion + core-strip must resolve");
        assert_eq!(key, "sys.bitfield.test_bit");
        assert_eq!(info.id.0, 42);

        // Ranked suffix: two candidates end with `.time.nanos` — the
        // shallower key wins deterministically.
        let mut ctx2 = CodegenContext::new();
        ctx2.register_function("zzz.deep.mod.time.nanos".to_string(), plain_info(7, 0));
        ctx2.register_function("sys.time.nanos".to_string(), plain_info(8, 0));
        let parts2: Vec<String> = ["time", "nanos"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (key2, info2) = ctx2.resolve_qualified_dotted_call(&parts2, 0).unwrap();
        assert_eq!(key2, "sys.time.nanos");
        assert_eq!(info2.id.0, 8);

        // Progressive head-strip: `core.time.Time.sleep_ms(ms)` maps
        // onto the canonical `Time.sleep_ms` impl-method key (leading
        // lower-case module segments absent from the registration).
        let mut ctx3 = CodegenContext::new();
        ctx3.register_function("Time.sleep_ms".to_string(), plain_info(9, 1));
        let parts3: Vec<String> = ["core", "time", "Time", "sleep_ms"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (key3, info3) = ctx3.resolve_qualified_dotted_call(&parts3, 1).unwrap();
        assert_eq!(key3, "Time.sleep_ms");
        assert_eq!(info3.id.0, 9);
        // The strip stops at the first non-module (uppercase) segment:
        // a bare `sleep_ms` free fn must NOT be reachable through it.
        let mut ctx4 = CodegenContext::new();
        ctx4.register_function("sleep_ms".to_string(), plain_info(10, 1));
        assert!(ctx4.resolve_qualified_dotted_call(&parts3, 1).is_none());
    }

    /// Reproduces the `IoError` collision documented in the bug class
    /// for `Connection.commit_tx`: two `IoError` variants exist —
    /// `VfsErrorKind.IoError` (unit) and `ConnectionError.IoError(Text)`.
    /// Without inner-generic context, the disambiguator can only see
    /// `current_return_type_name = "Result"` and gives up.  With the
    /// fix, `current_return_type_inner = ["X", "ConnectionError"]`
    /// drives the right pick.
    #[test]
    fn variant_disambig_uses_inner_generic_args() {
        let mut ctx = CodegenContext::new();

        // Two same-name variants, different parents and arities.
        ctx.register_function(
            "VfsErrorKind.IoError".to_string(),
            variant_info("VfsErrorKind", 5, 0),
        );
        ctx.register_function(
            "ConnectionError.IoError".to_string(),
            variant_info("ConnectionError", 4, 1),
        );

        // Outer return type is `Result` with `ConnectionError` as the
        // second generic. The disambiguator's inner-generic check
        // must steer to ConnectionError.IoError(Text) for arity 1.
        ctx.current_return_type_name = Some("Result".to_string());
        ctx.current_return_type_inner =
            Some(vec!["X".to_string(), "ConnectionError".to_string()]);

        let tag = ctx.find_variant_by_suffix_and_args("IoError", 1);
        assert_eq!(
            tag,
            Some(4),
            "disambiguator must pick ConnectionError.IoError(Text), not VfsErrorKind.IoError"
        );
    }

    /// Mirror test for `find_function_by_suffix` — the same
    /// inner-generic resolution must apply to the function-suffix
    /// path so callers like `compile_variant_constructor` and
    /// `compile_call_function` agree.
    #[test]
    fn find_function_by_suffix_uses_inner_generic_args() {
        let mut ctx = CodegenContext::new();

        ctx.register_function(
            "VfsErrorKind.IoError".to_string(),
            variant_info("VfsErrorKind", 5, 0),
        );
        ctx.register_function(
            "ConnectionError.IoError".to_string(),
            variant_info("ConnectionError", 4, 1),
        );

        ctx.current_return_type_name = Some("Result".to_string());
        ctx.current_return_type_inner =
            Some(vec!["Unit".to_string(), "ConnectionError".to_string()]);

        let info = ctx.find_function_by_suffix(".IoError");
        assert!(info.is_some(), "must resolve via inner-generic path");
        assert_eq!(
            info.unwrap().parent_type_name.as_deref(),
            Some("ConnectionError"),
            "must pick the variant whose parent appears as an inner generic"
        );
    }

    /// Without the inner-generic list (as before this fix), the
    /// disambiguator falls back to None for ambiguous matches with
    /// no other context — confirming the test above is exercising
    /// the new code path, not an unrelated tiebreaker.
    #[test]
    fn variant_disambig_returns_none_without_inner_when_ambiguous() {
        let mut ctx = CodegenContext::new();
        ctx.register_function(
            "FooError.IoError".to_string(),
            variant_info("FooError", 1, 1),
        );
        ctx.register_function(
            "BarError.IoError".to_string(),
            variant_info("BarError", 2, 1),
        );

        // Outer name is "Result" — neither parent matches.
        // No inner-generic context — disambiguator must fall back
        // to None (ambiguous, no signal).
        ctx.current_return_type_name = Some("Result".to_string());
        ctx.current_return_type_inner = None;

        let tag = ctx.find_variant_by_suffix_and_args("IoError", 1);
        assert!(
            tag.is_none(),
            "ambiguous matches with no resolution signal must return None"
        );
    }

    /// `push_disambig_context` / `pop_disambig_context` must carry
    /// both fields atomically — sites that previously saved only the
    /// name field could leak inner-generic state from an outer
    /// context into an inner override.
    #[test]
    fn push_pop_disambig_context_round_trips_both_fields() {
        let mut ctx = CodegenContext::new();
        ctx.current_return_type_name = Some("Result".to_string());
        ctx.current_return_type_inner =
            Some(vec!["Int".to_string(), "ConnectionError".to_string()]);

        let saved = ctx.push_disambig_context(Some("Maybe".to_string()));
        assert_eq!(ctx.current_return_type_name.as_deref(), Some("Maybe"));
        assert!(
            ctx.current_return_type_inner.is_none(),
            "push must clear inner — a different override doesn't inherit the outer's generics"
        );

        ctx.pop_disambig_context(saved);
        assert_eq!(ctx.current_return_type_name.as_deref(), Some("Result"));
        assert_eq!(
            ctx.current_return_type_inner.as_deref(),
            Some(vec!["Int".to_string(), "ConnectionError".to_string()].as_slice())
        );
    }
}
