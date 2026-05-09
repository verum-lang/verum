//! Bidirectional type inference engine.
//!
//! This is the core of Verum's type system. It implements bidirectional
//! type checking which is 3-5x faster than traditional Algorithm W.
//!
//! # Bidirectional Type Checking
//!
//! The algorithm operates in two modes:
//! - **Synthesis (⇒)**: Infer type from expression
//! - **Checking (⇐)**: Check expression against expected type
//!
//! Key insight: Type annotations switch from synthesis to checking mode,
//! allowing the algorithm to prune the search space early.

// Submodules containing impl TypeChecker method groups
pub(crate) mod decls;           // Type declaration registration (register_type_declaration*, …)
pub(crate) mod env;             // Environment management (stdlib bootstrap, pre-register, constraints)
pub(crate) mod expr;            // Expression inference (synth_expr, check_expr, infer_expr*)
pub(crate) mod modules;         // Module/import/export methods (check_item, check_module, …)
pub(crate) mod patterns;        // Pattern binding and destructuring
pub(crate) mod path_resolution; // Path resolution (ExprKind::Path)
pub(crate) mod types;           // Type resolution (ast_to_type*, normalize_type*, check_cast)

use crate::const_eval::ConstEvaluator;
use crate::context::{TypeContext, TypeScheme};
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};

/// A verification goal deferred from the type-checker to the
/// pipeline's DependentVerifier. Accumulated during inference
/// when a check can't be resolved locally.
#[derive(Debug, Clone)]
pub enum DeferredVerificationGoal {
    /// Two `EqTerm`s that the unifier couldn't prove definitionally
    /// equal, even after the cubical bridge. The orchestrator may
    /// try the full SMT dependent-equality backend.
    CubicalEquality {
        lhs: crate::ty::EqTerm,
        rhs: crate::ty::EqTerm,
        span: verum_ast::span::Span,
    },
    /// Universe constraints that the local solver left undecided
    /// (e.g., involving variables from other modules). The
    /// orchestrator's universe solver may resolve them with a
    /// wider constraint set.
    UniverseConstraints {
        constraints: List<crate::universe_solver::UniverseConstraint>,
    },
}
use crate::integer_hierarchy::IntegerHierarchy;
use crate::meta_context::{MetaContextValidation, MetaContextValidator};
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
use crate::protocol::ProtocolChecker;
use crate::refinement::RefinementChecker;
use crate::stage_checker::{StageChecker, StageConfig};
use crate::subtype::Subtyping;
use crate::ty::{Type, TypeVar};
use crate::unify::Unifier;
use crate::{Result, TypeCheckMetrics, TypeError};
use std::time::Instant;
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::well_known_types::type_names as wkt_names;

// Const aliases for use in `matches!()` patterns (which require string literals or consts)
const WKT_HEAP: &str = wkt_names::HEAP;
const WKT_SHARED: &str = wkt_names::SHARED;
const WKT_RESULT: &str = wkt_names::RESULT;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashSet;

// Debug counter for tracking recursive call depth
thread_local! {
    static AST_TO_TYPE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static NORMALIZE_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Set of type names currently being resolved, for cycle detection.
    static TYPE_RESOLUTION_STACK: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Set of type names currently being normalized, for indirect cycle detection.
    /// Prevents stack overflow from circular struct types like A -> B -> C -> A.
    static NORMALIZE_TYPE_STACK: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Global call depth counter — catches infinite recursion that per-TypeChecker
    /// guards miss (e.g., recursion through protocol dispatch or unification).
    static GLOBAL_CALL_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Per-call depth counter for the Deref-coercion retry in method
    /// dispatch (#112). Caps at 4 hops — matches the existing
    /// `Heap<Shared<Mutex<T>>>` hint-walk depth and prevents infinite
    /// recursion through pathological `Deref::Target` cycles. Each
    /// retry increments on entry and decrements via RAII guard on exit
    /// so partial failures don't poison subsequent dispatches.
    static DEREF_COERCION_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// RAII guard for global call depth tracking.
pub(crate) struct GlobalDepthGuard;

impl GlobalDepthGuard {
    #[inline]
    pub(crate) fn enter() -> Result<Self> {
        GLOBAL_CALL_DEPTH.with(|d| {
            let depth = d.get() + 1;
            d.set(depth);
            if depth > 100 {
                d.set(0); // Reset to prevent cascading errors
                Err(TypeError::Other(Text::from(format!(
                    "global type inference depth exceeded ({})",
                    depth
                ))))
            } else {
                Ok(GlobalDepthGuard)
            }
        })
    }
}

impl Drop for GlobalDepthGuard {
    fn drop(&mut self) {
        GLOBAL_CALL_DEPTH.with(|d| {
            let v = d.get();
            if v > 0 {
                d.set(v - 1);
            }
        });
    }
}

/// RAII guard for thread-local depth counters.
/// Automatically decrements the counter on drop, preventing stuck counters
/// after panics or early returns.
struct ThreadLocalDepthGuard {
    counter: &'static std::thread::LocalKey<Cell<usize>>,
}

impl ThreadLocalDepthGuard {
    /// Increment the counter and return a guard that decrements on drop.
    /// Returns None if the depth exceeds the limit.
    fn new(counter: &'static std::thread::LocalKey<Cell<usize>>, max_depth: usize) -> Option<Self> {
        let depth = counter.with(|d| {
            let current = d.get();
            d.set(current + 1);
            current + 1
        });
        if depth > max_depth {
            // Exceeded limit — decrement and signal failure
            counter.with(|d| d.set(d.get().saturating_sub(1)));
            return None;
        }
        Some(ThreadLocalDepthGuard { counter })
    }
}

impl Drop for ThreadLocalDepthGuard {
    fn drop(&mut self) {
        self.counter.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// RAII guard for cycle detection during type resolution.
/// Inserts a type name into the resolution stack and removes it on drop.
struct TypeResolutionCycleGuard {
    name: String,
}

impl TypeResolutionCycleGuard {
    /// Try to begin resolving a type. Returns None if the type is already
    /// being resolved (cycle detected).
    fn try_enter(name: String) -> Option<Self> {
        TYPE_RESOLUTION_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&name) {
                None // Cycle detected
            } else {
                stack.insert(name.clone());
                Some(TypeResolutionCycleGuard { name })
            }
        })
    }
}

impl Drop for TypeResolutionCycleGuard {
    fn drop(&mut self) {
        TYPE_RESOLUTION_STACK.with(|stack| {
            stack.borrow_mut().remove(&self.name);
        });
    }
}

/// RAII guard for cycle detection during type normalization.
/// Prevents stack overflow from indirect circular struct types like A -> B -> C -> A.
struct NormalizeTypeCycleGuard {
    name: String,
}

impl NormalizeTypeCycleGuard {
    /// Try to begin normalizing a type. Returns None if the type is already
    /// being normalized (cycle detected).
    fn try_enter(name: String) -> Option<Self> {
        NORMALIZE_TYPE_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&name) {
                None // Cycle detected
            } else {
                stack.insert(name.clone());
                Some(NormalizeTypeCycleGuard { name })
            }
        })
    }
}

impl Drop for NormalizeTypeCycleGuard {
    fn drop(&mut self) {
        NORMALIZE_TYPE_STACK.with(|stack| {
            stack.borrow_mut().remove(&self.name);
        });
    }
}

// =====================================================================
// Stack Safety Constants
// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
// =====================================================================

/// Maximum allowed stack allocation in bytes (1MB default)
/// Larger allocations should use Heap<T> or List<T>
const MAX_STACK_ALLOCATION_BYTES: u64 = 1024 * 1024;

/// Size of primitive types in bytes
const SIZE_OF_INT: u64 = 8;
const SIZE_OF_FLOAT: u64 = 8;
const SIZE_OF_BOOL: u64 = 1;
const SIZE_OF_CHAR: u64 = 4;
const SIZE_OF_POINTER: u64 = 8;

/// Read the highest `@classification(<level>)` annotation from a
/// list of AST attributes (#291 Phase 2b-Integration). Mirrors
/// the same logic in `verum_compiler::phases::safety_gate::
/// read_classification` — kept in sync; both consumers duplicate
/// the small AST walk so neither crate depends on the other's
/// implementation.
///

/// Returns `MlsLevel::Public` (the safe default) when no
/// `@classification` attribute is present. Multiple attributes on
/// the same item produce the highest declared level — matching
/// the lattice's join semantics.
pub(crate) fn read_param_classification(
    attrs: &List<verum_ast::attr::Attribute>,
) -> verum_common::mls::MlsLevel {
    use verum_common::mls::MlsLevel;
    let mut found = MlsLevel::Public;
    for attr in attrs.iter() {
        if !attr.is_named("classification") {
            continue;
        }
        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                if let verum_ast::expr::ExprKind::Path(path) = &arg.kind {
                    if let Some(ident) = path.as_ident() {
                        let parsed = MlsLevel::from_manifest_str(ident.as_str());
                        if parsed > found {
                            found = parsed;
                        }
                    }
                }
            }
        }
    }
    found
}

/// Detect `@declassify` attribute on a FunctionDecl (#295).
///

/// Returns `true` when the function declares itself a
/// declassification boundary — the call-classification walker
/// skips its body entirely. `@declassify` accepts no arguments
/// in this Phase 2b version (the function as a whole is the
/// declassification boundary). A future Phase 2b-Cap variant
/// could accept a destination level
/// (`@declassify(public)`) to cap the escape-hatch level.
pub(crate) fn has_declassify_attr_on_function(func: &verum_ast::decl::FunctionDecl) -> bool {
    func.attributes.iter().any(|a| a.is_named("declassify"))
}

/// Stdlib top-level prefix discriminator.  When a user writes
/// `mount foo.bar.X` and `foo` is a known stdlib top-level (an
/// immediate subdirectory of `core/`), the resolver MUST normalize
/// the path to `core.foo.bar.X` so the cross-file `ModuleRegistry`
/// finds the right module.  Without this, `mount database.postgres.AsyncPgPool`
/// resolves against `database.postgres` which doesn't exist (the
/// registered path is `core.database.postgres`), and every type
/// imported via that route surfaces as `E101: type not found`.
///
/// **Architectural rule.**  This list is name-list shorthand for
/// the registry's top-level structure.  Per the
/// `verum_types/CLAUDE.md` rule (no hardcoded stdlib knowledge in
/// the compiler), it could in principle be discovered at
/// session-init by enumerating the registry's top-level keys;
/// keeping it explicit here is a clarity choice — every name on
/// this list IS the name of a `core/` subdirectory or a `.vr` file
/// at `core/`'s root, and every cross-file resolver in this crate
/// converges on the same set of recognised prefixes.
pub(crate) fn is_stdlib_toplevel_path(path_str: &str) -> bool {
    // Every `core/` subdirectory plus the bare-dotted-prefix form.
    // Drop the trailing dot for prefix matching; bare `==` for the
    // unsegmented form (e.g. `mount sys`).
    const STDLIB_TOPS: &[&str] = &[
        "sys",
        "io",
        "net",
        "runtime",
        "sync",
        "mem",
        "text",
        "time",
        "collections",
        "base",
        "intrinsics",
        "simd",
        "math",
        "async",
        "async_",
        "meta",
        "action",
        "architecture",
        "archive",
        "cache",
        "cli",
        "cog",
        "compress",
        "concurrency",
        "configuration",
        "context",
        "control",
        "database",
        "diagnostics",
        "encoding",
        "eval",
        "logic",
        "mesh",
        "metrics",
        "money",
        "proof",
        "protobuf",
        "redis",
        "search",
        "security",
        "shell",
        "signal",
        "storage",
        "target",
        "term",
        "theory_interop",
        "tracing",
        "types",
        "verify",
    ];
    for &top in STDLIB_TOPS {
        if path_str == top {
            return true;
        }
        if path_str.len() > top.len()
            && path_str.starts_with(top)
            && path_str.as_bytes()[top.len()] == b'.'
        {
            return true;
        }
    }
    false
}

/// Recursively collect every public Mount re-export from an inline-module
/// item list AND from every nested public submodule.
///
/// **Why recursive.**  The canonical "prelude" pattern lives at
/// `core/mod.vr`:
///
/// ```verum
/// public module prelude {
///     public mount super.collections.List;
///     public mount super.base.Maybe;
///     // …
/// }
/// ```
///
/// `mount core.*` walks `core`'s top-level items.  Without recursion, the
/// `prelude` submodule is seen only as `ItemKind::Module(...)` and its
/// inner `public mount ...` re-exports are invisible to the outer walk —
/// even though the user writing `mount core.*` semantically expects the
/// prelude to fold in.
///
/// **Generality.**  Per the type-system architectural rule (no hardcoded
/// stdlib knowledge in the compiler), this walk recurses into ANY public
/// submodule, not just the canonical `prelude` name.  Any inline module
/// that contains public submodules with public Mount re-exports will
/// expose those re-exports at every outer mount-glob site.
///
/// Path resolution honours `super` / `self` / `cog` segments relative to
/// the SUBMODULE's path during recursion, mirroring the language's
/// natural scoping discipline (a `super` inside `core.prelude` resolves
/// to `core`, not back to whatever was the top-level `module_name`).
///
/// `current_module_path` is the dotted path at which `items` lives (e.g.
/// `"core"` for the top-level call, `"core.prelude"` once recursed into
/// the prelude submodule).
pub(crate) fn collect_inline_mount_reexports_recursive(
    items: &[verum_ast::Item],
    current_module_path: &str,
    out: &mut Vec<(Text, Option<Text>)>,
) {
    use verum_ast::decl::MountTreeKind;
    use verum_ast::ty::PathSegment;

    // Resolution helper closure — lives inside the function body
    // because it depends on the per-call `current_module_path`.
    let resolve = |path: &verum_ast::ty::Path| -> Text {
        let mut parts: Vec<String> = Vec::new();
        for seg in &path.segments {
            match seg {
                PathSegment::Super => {
                    if parts.is_empty() {
                        // Strip last segment from current_module_path —
                        // `super` from a submodule lifts one level.
                        let segs: Vec<&str> = current_module_path.split('.').collect();
                        if segs.len() > 1 {
                            for s in &segs[..segs.len() - 1] {
                                parts.push(s.to_string());
                            }
                        }
                    } else {
                        parts.pop();
                    }
                }
                PathSegment::SelfValue => {
                    if parts.is_empty() {
                        for s in current_module_path.split('.') {
                            parts.push(s.to_string());
                        }
                    }
                }
                PathSegment::Cog => {
                    parts.clear();
                }
                PathSegment::Relative => {
                    if parts.is_empty() {
                        for s in current_module_path.split('.') {
                            parts.push(s.to_string());
                        }
                    }
                }
                PathSegment::Name(ident) => {
                    parts.push(ident.name.as_str().to_string());
                }
            }
        }
        Text::from(parts.join("."))
    };

    for item in items {
        match &item.kind {
            verum_ast::ItemKind::Mount(mount_decl) => {
                if !matches!(mount_decl.visibility, verum_ast::decl::Visibility::Public) {
                    continue;
                }
                match &mount_decl.tree.kind {
                    MountTreeKind::Glob(path) => {
                        let p = resolve(path);
                        out.push((p, None));
                    }
                    MountTreeKind::Path(path) => {
                        // Single item: parent is everything except last segment.
                        let mut prefix = path.clone();
                        let item_name_text =
                            if let Some(PathSegment::Name(id)) = prefix.segments.last() {
                                Some(Text::from(id.name.as_str()))
                            } else {
                                None
                            };
                        if !prefix.segments.is_empty() {
                            let mut new_segs = prefix.segments.clone();
                            new_segs.pop();
                            prefix.segments = new_segs;
                        }
                        let parent = resolve(&prefix);
                        out.push((parent, item_name_text));
                    }
                    MountTreeKind::Nested { prefix, trees } => {
                        let parent = resolve(prefix);
                        for tree in trees {
                            if let MountTreeKind::Path(p) = &tree.kind {
                                if let Some(PathSegment::Name(id)) = p.segments.first() {
                                    out.push((
                                        parent.clone(),
                                        Some(Text::from(id.name.as_str())),
                                    ));
                                }
                            }
                        }
                    }
                    MountTreeKind::File { .. } => {}
                }
            }
            verum_ast::ItemKind::Module(submod) => {
                // Recurse only into public submodules.  A private nested
                // module's items are NOT visible to the outer mount site
                // by design (`pub` discipline).
                if !matches!(submod.visibility, verum_ast::decl::Visibility::Public) {
                    continue;
                }
                if let Maybe::Some(sub_items) = &submod.items {
                    let nested_path = if current_module_path.is_empty() {
                        submod.name.name.as_str().to_string()
                    } else {
                        format!("{}.{}", current_module_path, submod.name.name.as_str())
                    };
                    collect_inline_mount_reexports_recursive(
                        sub_items.as_slice(),
                        nested_path.as_str(),
                        out,
                    );
                }
            }
            _ => {}
        }
    }
}

use smallvec::SmallVec;
use verum_ast::decl::{FunctionBody, FunctionParamKind};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind, TypeProperty, UnOp};
use verum_ast::literal::Literal;
use verum_ast::pattern::Pattern;
use verum_ast::span::{Span, Spanned};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{Ident, Path};
use verum_common::ToText;
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
// Cross-module type resolution
// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports
use verum_modules::{ModulePath, ModuleRegistry, NameResolver, resolve_import, resolver::NameKind};

/// Mode for bidirectional type checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferMode {
    /// Synthesis mode (⇒): infer type from expression
    Synth,
    /// Checking mode (⇐): check expression against expected type
    Check(TypeVar), // Points to expected type in context
}

/// Result of type inference.
#[derive(Debug, Clone)]
pub struct InferResult {
    /// Inferred or checked type
    pub ty: Type,
    /// Any verification conditions generated
    pub vcs: List<crate::refinement::VerificationCondition>,
}

impl InferResult {
    pub fn new(ty: Type) -> Self {
        Self {
            ty,
            vcs: List::new(),
        }
    }

    pub fn with_vcs(ty: Type, vcs: List<crate::refinement::VerificationCondition>) -> Self {
        Self { ty, vcs }
    }
}

/// Work items for iterative type inference.
///

/// This enum represents the different stages of type inference work to avoid
/// stack overflow in deeply nested expressions.
#[derive(Debug)]
enum InferWork<'a> {
    /// Synthesize type for an expression (push result to value stack)
    SynthExpr(&'a Expr),

    /// Check expression against expected type (no result pushed)
    CheckExpr(&'a Expr, Type),

    /// Compute binary operation result after left operand is ready
    /// (left_ty is on value stack, need to process right operand)
    BinaryOpRight {
        op: BinOp,
        right: &'a Expr,
        span: Span,
    },

    /// Compute binary operation final result after both operands are ready
    /// (both left_ty and right_ty are on value stack)
    BinaryOpResult { op: BinOp, span: Span },

    /// Compute unary operation result after operand is ready
    /// (inner_ty is on value stack)
    /// inner_expr is needed for NLL tracking in Deref operations
    UnaryOpResult {
        op: UnOp,
        inner_expr: &'a Expr,
        span: Span,
    },

    /// Process function call arguments
    /// (func_ty is on value stack, need to process args)
    CallArgs {
        func_expr: &'a Expr,
        args: &'a [Expr],
        span: Span,
    },

    /// Compute function call result after all args are checked
    /// (func_ty is on value stack, all args have been checked)
    CallResult { func_ty: Type, arg_count: usize },

    /// Process field access after receiver is ready
    /// (receiver_ty is on value stack)
    FieldResult { field: Ident, span: Span },

    /// Process method call after receiver is ready
    /// (receiver_ty is on value stack, need to process args)
    MethodCall {
        method: Ident,
        args: &'a [Expr],
        span: Span,
    },

    /// Process if-expression after condition is checked
    /// (need to process then and else branches)
    IfBranches {
        then_branch: &'a Expr,
        else_branch: &'a Expr,
        span: Span,
    },

    /// Compute if-expression result after both branches are ready
    /// (then_ty and else_ty are on value stack)
    IfResult { span: Span },
}

/// Stored contract clauses for a function (requires/ensures).
/// Used for checking preconditions at call sites and postconditions at returns.
#[derive(Debug, Clone)]
pub struct FunctionContract {
    /// Parameter names in order (for substitution into requires predicates)
    pub param_names: List<Text>,
    /// Precondition expressions (requires clauses)
    pub requires: List<Expr>,
    /// Postcondition expressions (ensures clauses)
    pub ensures: List<Expr>,
}

/// The main type checker.
pub struct TypeChecker {
    /// Type context
    pub(crate) ctx: TypeContext,
    /// Unifier
    pub(crate) unifier: Unifier,
    /// Refinement checker
    refinement: RefinementChecker,
    /// Subtyping checker
    pub(crate) subtyping: Subtyping,
    /// Const evaluator for meta parameters
    const_eval: ConstEvaluator,
    /// Protocol checker for method resolution (shared across all modules)
    /// Using Shared<RwLock<...>> enables stdlib impls to be pre-registered and
    /// shared across all TypeChecker instances for industrial-grade type resolution.
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    pub protocol_checker: Shared<parking_lot::RwLock<ProtocolChecker>>,
    /// Integer type hierarchy for numeric cast checking
    integer_hierarchy: IntegerHierarchy,
    /// Kind inference engine for higher-kinded types
    /// Higher-kinded type (HKT) inference and specialization selection: kind inference for type constructors (Type -> Type), automatic selection of most specific specialization
    kind_inferer: crate::kind_inference::KindInferer,
    /// Performance metrics
    pub metrics: TypeCheckMetrics,
    /// Current generator context (for tracking yield types)
    /// Generator functions: fn* syntax yields values lazily, producing Iterator<Item=T> types
    generator_context: Maybe<GeneratorContext>,
    /// Collected diagnostics (warnings, notes, etc.)
    pub(crate) diagnostics: List<Diagnostic>,
    /// Errors of `is_soundness_critical()` kind that surfaced inside
    /// helpers whose Rust signature is `()` (e.g. cross-module type
    /// pre-passes). Drained by `phase_type_check` before declaring
    /// success so a Berardi-shaped declaration tucked inside an
    /// imported module still aborts the build.
    pub(crate) deferred_soundness_errors: Vec<TypeError>,
    /// MOD-MED-2 — provenance side-table for glob-imported
    /// names. Maps each name registered by a glob mount to its
    /// `ImportProvenance { origin, module_path }`. Consulted at every
    /// glob-registration site to decide whether the incoming glob
    /// should overwrite an existing entry: the rule is
    /// `Project > External > Stdlib`, ties go to the first registrant.
    /// Explicit imports bypass this map entirely.
    pub(crate) glob_import_provenance:
        std::collections::HashMap<verum_common::Text, crate::import_origin::ImportProvenance>,
    /// MOD-MED-2 — name of the user's current cog used by
    /// `ImportOrigin::classify` to distinguish project paths from
    /// stdlib/external. Empty string means classification falls back
    /// to "External" for non-stdlib paths.
    pub(crate) current_cog_name: verum_common::Text,
    /// Whether dependent-type features (Pi, Sigma, dependent match)
    /// are enabled. Controlled by `[types] dependent` in verum.toml.
    dependent_enabled: bool,
    /// Whether HKT kind inference is active (`[types] higher_kinded`).
    higher_kinded_enabled: bool,
    /// Whether universe polymorphism is tracked (`[types] universe_polymorphism`).
    universe_poly_enabled: bool,
    /// Whether coinductive types (codata/cofix) are allowed (`[types] coinductive`).
    coinductive_enabled: bool,
    /// Whether quotient types are allowed (`[types] quotient`).
    quotient_enabled: bool,
    /// Whether automatic instance search is active (`[types] instance_search`).
    instance_search_enabled: bool,
    /// Maximum coherence-check depth (`[types] coherence_check_depth`).
    coherence_check_depth: u32,
    /// Whether protocols may declare HKT generic parameters
    /// (e.g. `protocol Functor<F<_>> { ... }`). Controlled by
    /// `[protocols].higher_kinded_protocols` in Verum.toml.
    /// Default `false` — must be explicitly enabled in the
    /// manifest. Pre-condition: `[types].higher_kinded` must also
    /// be true (enforced at manifest validation time, see
    /// `LanguageFeatures::validate` at language_features.rs:412).
    /// Closes the inert-defense pattern at session.rs:590.
    higher_kinded_protocols_enabled: bool,
    /// Whether protocols may declare generic associated types
    /// (GATs, e.g. `type Item<T>` inside a protocol body).
    /// Controlled by `[protocols].generic_associated_types` in
    /// Verum.toml. Default `false` — must be explicitly enabled
    /// in the manifest. Pre-condition: `[protocols].
    /// associated_types` must also be true (enforced at manifest
    /// validation time, see `LanguageFeatures::validate` at
    /// language_features.rs:419).
    /// Closes the inert-defense pattern at session.rs (#265).
    generic_associated_types_enabled: bool,
    /// MLS classification sidecar (#289 Phase 2b foundation).
    ///

    /// Maps binding identity (variable name in the current scope's
    /// flat namespace) to its `MlsLevel`. When a function
    /// parameter carries `@classification(secret)`, the binding
    /// lands here at parameter-introduction time. Subsequent
    /// `let` bindings derived from that variable inherit the
    /// classification (taint propagation) — Phase 2b-full pipes
    /// the propagation through unify / synth / check sites in
    /// `infer.rs::synth_*`.
    ///

    /// The map is keyed by `Text` (variable name, scoped) rather
    /// than a TypeVar id because classification is a property of
    /// BINDINGS (not types) — two different variables can carry
    /// the same `Type::Int` but distinct classifications.
    ///

    /// Architecture phases (all CLOSED):
    ///  * Phase 2b-Foundation (#289): this map (storage).
    ///  * Phase 2b-Integration (#291): seeded from
    ///  `@classification` attributes at
    ///  `register_function_signature` time.
    ///  * Phase 2b-Followup (#292): expression classification
    ///  propagated through let-bindings via
    ///  `expr_classification` + StmtKind::Let arm.
    ///  * Phase 2b-Helper (#293): `check_classification_downflow`
    ///  enforces lattice contract `param.subsumes(arg)`.
    ///  * Phase 2b-Integration (#294):
    ///  `check_module_call_classifications` walker invokes
    ///  the helper at every call site.
    ///  * Phase 2b-@declassify (#295): functions carrying
    ///  `@declassify` are skipped by the walker.
    pub(crate) classification_map:
        std::collections::HashMap<verum_common::Text, verum_common::mls::MlsLevel>,
    /// Name resolver for cross-module resolution
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Name resolution across modules
    pub(crate) module_resolver: NameResolver,
    /// Module registry for type lookup
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Module-qualified type access
    /// THE authoritative module registry — same Shared<RwLock<...>>
    /// handle the compiler Session owns. Prior design had a
    /// `Shared<ModuleRegistry>` here (no RwLock) and a separate
    /// `session_registry: Shared<RwLock<ModuleRegistry>>` alongside,
    /// populated by clone-on-set. That bifurcation meant the two
    /// copies drifted in state — lazy-loaded modules landed in
    /// session_registry but module_registry kept its stale snapshot.
    /// This field now unifies both roles: one handle, one state.
    pub(crate) module_registry: Shared<parking_lot::RwLock<ModuleRegistry>>,
    /// Current module path for import resolution
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Path resolution in imports
    pub(crate) current_module_path: Text,
    /// Registry of inline module declarations for qualified path resolution
    /// Maps module path (e.g., "cog.api.v1") to its declaration
    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
    pub(crate) inline_modules: Map<Text, verum_ast::decl::ModuleDecl>,
    /// User-visible module aliases from `mount X as A;` declarations.
    /// Maps the alias identifier (`A`) to the fully-qualified module
    /// path (`X`, possibly with a `cog.` prefix). Populated by
    /// `process_import_aliases` whenever the aliased path resolves to
    /// a known module. Consulted by method-call dispatch so that
    /// `A.method(...)` is treated as a module-path lookup
    /// (`<path>.method`) rather than a value-lookup on the identifier.
    ///

    /// Needed because stdlib symbols (e.g. `core.sys.linux.syscall.stat`)
    /// can be resolved into the flat name environment via cross-module
    /// imports and shadow a locally-declared mount alias like
    /// `mount core.net.h3.qpack.static_table as stat;`. Without this
    /// registry the method-call receiver was synth_expr'd as the
    /// stdlib function and `.get(0)` attempted method dispatch on a
    /// function value.
    pub(crate) module_aliases: Map<Text, Text>,
    /// Tracks which modules have had their function signatures pre-registered
    /// to avoid redundant pre-registration when importing multiple items from the same module
    preregistered_modules: std::collections::HashSet<String>,
    /// Tracks which modules have had their blanket protocol impls registered
    /// Blanket impls like `implement<T, U: From<T>> Into<U> for T` must be registered
    /// globally when ANY item from the module is imported, not just when a specific type is imported.
    blanket_impls_registered_modules: std::collections::HashSet<String>,
    /// Tracks which modules have had their primitive type impls registered
    /// Primitive impls like `implement Int { fn abs(self) -> Int { ... } }` must be
    /// registered globally when ANY item from the module is imported.
    primitive_impls_registered_modules: std::collections::HashSet<String>,
    /// Module-level type inference context (COMPLETE implementation)
    /// Enables cross-function inference, mutual recursion, and polymorphic inference
    module_context: Maybe<crate::module_context::ModuleContext>,
    /// Current function's return type (for ? operator checking)
    /// Try operator type checking: ? operator desugars to match with From conversion, requires Result/Maybe return type — Try operator type checking
    current_function_return_type: Maybe<Type>,
    /// Current function's name (for better error messages)
    current_function_name: Maybe<Text>,
    /// Current function's return type span (for diagnostic source locations)
    current_function_return_span: Maybe<Span>,
    /// Current function's parameter names (for return lifetime validation)
    /// Used to distinguish local variables from parameters when checking return values
    /// Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Return lifetime validation
    current_function_params: Set<Text>,
    /// Type registry mapping AST nodes to inferred types
    /// Enables type information flow from TypeChecker to Codegen
    pub type_registry: crate::type_registry::TypeRegistry,
    /// Context resolver for expanding context groups
    /// Context group expansion: resolving context group names to their constituent contexts recursively — Context group expansion
    context_resolver: crate::context_resolution::ContextResolver,
    /// Current self type when checking methods in implement blocks
    /// Used to bind `self` parameters in method contexts
    pub(crate) current_self_type: Maybe<Type>,
    /// Capability checker for context attenuation
    /// Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0 - Capability Attenuation
    capability_checker: crate::capability::CapabilityChecker,
    /// Context declarations for method-level capability extraction
    /// Maps context name to its AST declaration
    context_declarations: Map<Text, verum_ast::decl::ContextDecl>,
    /// Method capability mapper for extracting required capabilities from method calls
    method_capability_mapper: crate::capability::MethodCapabilityMapper,
    /// Inherent instance methods from implement blocks
    /// Maps type_name -> (method_name -> method_type)
    /// Used for resolving obj.method() calls where method has self parameter
    ///

    /// NOTE: This is wrapped in Shared<RwLock<...>> to enable order-independent
    /// method resolution across modules. Methods registered in implement blocks
    /// become immediately visible to all TypeChecker instances sharing this map.
    /// Order-independent declarations: types and functions can be referenced before their definition within a module
    inherent_methods: Shared<parking_lot::RwLock<Map<Text, Map<Text, crate::context::TypeScheme>>>>,
    /// For each (type_name, method_name), stores the list of allowed impl self-type arg patterns.
    /// Used to filter method availability based on receiver type specialization.
    /// E.g., write() on Register has patterns [[Var, WriteOnly], [Var, ReadWrite]]
    /// meaning it's only available when the 2nd type arg is WriteOnly or ReadWrite.
    method_impl_patterns: Shared<parking_lot::RwLock<Map<Text, Map<Text, List<List<Type>>>>>>,
    /// Variance checker for generic type parameters.
    /// Validates that declared variance (+T, -T) matches actual usage in type bodies.
    /// Ensures type system soundness for covariant/contravariant generic parameters.
    variance_checker: crate::variance::VarianceChecker,
    /// Maps variant type signature to its declared name for instance method lookup
    /// This is necessary because variant types lose their name when resolved
    /// from a Named type to the underlying Variant structure.
    /// Key is a stable string signature of the variant structure (sorted variant names).
    variant_type_names: Map<Text, Text>,
    /// Audit-A1: every collision detected during
    /// `register_variant_type_name_first_wins` (existing entry maps
    /// signature `S` to type `A`, second registration tries to map `S`
    /// to a different type `B`). Stored as `(signature, kept, dropped)`
    /// so downstream diagnostics can report both type names. Empty in
    /// the well-formed case; non-empty surfaces a coherence violation
    /// the prior `or_insert()` silently swallowed.
    variant_collision_log: List<(Text, Text, Text)>,
    /// Record variant fields: maps variant name (e.g., "Rect") to its field types.
    /// Used for `Rect { w: 4, h: 6 }` construction resolution in check_expr.
    /// Stored separately from type_defs to avoid infinite recursion during type resolution.
    variant_record_fields: Map<Text, indexmap::IndexMap<Text, Type>>,
    /// Maps variant constructor short names (e.g., "Ok") to all parent type names
    /// that define them. This enables scope-aware resolution: when the expected
    /// type context indicates a specific parent type, we can pick the right constructor.
    /// For example, "Ok" -> ["Result", "CheckedResult"] allows resolving Ok(x) to
    /// CheckedResult.Ok(x) when the expected type is CheckedResult<T>.
    variant_constructor_parents: Map<Text, List<Text>>,
    /// Type variable bounds tracking for improved inference
    /// Maps type variables to their protocol bounds (e.g., T: Ord + Clone)
    /// This enables method resolution on bounded type variables and
    /// better error messages when bounds are violated.
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    type_var_bounds: Map<TypeVar, List<crate::protocol::ProtocolBound>>,
    /// Lookup from a Higher-Kinded parameter name to its fresh TypeVar id.
    ///

    /// HKT parameters like `F<_>: Functor` are registered both as a
    /// `Type::Var(fresh_tvar)` in `ctx.env`/`ctx.types` AND as a
    /// `Type::TypeConstructor { name, arity, kind }` in kind-inferred contexts.
    /// Later lookups through `ctx.env` typically see the `TypeConstructor`
    /// form, which does not carry the TypeVar id needed to query the
    /// protocol bounds registered in `type_var_bounds`. This side table
    /// preserves the connection so method dispatch through an HKT
    /// parameter's protocol bound can still be resolved via the bound-first
    /// path in `infer_method_call_inner_impl`.
    hkt_type_var_by_name: Map<Text, TypeVar>,
    /// Direct type bounds for type variables (e.g., F: fn() -> T).
    /// Unlike protocol bounds which reference protocols by path, type bounds
    /// store the actual type constraint. This is essential for:
    /// - Function type bounds: F: fn(A) -> B
    /// - Higher-kinded type bounds: C: Container<T>
    /// - Equality bounds from generics
    /// Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
    type_var_type_bounds: Map<TypeVar, List<Type>>,
    /// Deferred constraint queue for improved constraint solving
    /// Contains constraints that couldn't be solved immediately and need
    /// to be revisited after more type information is available.
    /// This enables better handling of mutually recursive definitions
    /// and complex generic instantiations.
    deferred_constraints: List<DeferredConstraint>,
    /// Property inferrer for computational property inference
    /// Computational properties: compile-time tracking of Pure, IO, Async, Fallible, Mutates effects inferred from function bodies — (Pure, IO, Async, Fallible, Mutates)
    /// Analyzes function bodies to infer their computational properties
    property_inferrer: crate::computational_properties::PropertyInferrer,
    /// Tracks whether we are currently inside an async function or async block
    /// This is used to validate that async-only constructs (select, await) are
    /// only used within async contexts.
    /// Select expressions require async context: "select { ... }" only valid in async functions — Select expressions require async context
    in_async_context: bool,
    /// Depth counter for try/recover blocks. When > 0, throw is allowed
    /// even if the enclosing function doesn't return Result<T, E>.
    try_recover_depth: usize,
    /// Current function's throws clause error types (resolved from AST).
    /// When Some, the enclosing function has a `throws(E1 | E2)` clause and
    /// `throw expr` must produce a value matching one of these types.
    /// When None, `throw` is only allowed inside a try/recover block.
    current_function_throws: Maybe<List<Type>>,
    /// Context checker for validating context requirements during type inference.
    /// Integrated during type inference to catch context errors early.
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    context_checker: ContextChecker,
    /// Stage checker for N-level staged metaprogramming validation.
    /// Validates cross-stage calls and quote stage coherence.
    /// Stage coherence: runtime code cannot depend on meta-only values, meta code cannot observe runtime state — Stage Coherence Rule
    stage_checker: StageChecker,
    /// Current function's stage level (0 = runtime, 1+ = meta).
    /// Used for cross-stage call validation during type inference.
    current_function_stage: u32,
    /// Current function's transparency flag (from @transparent attribute).
    /// When true, the macro disables hygienic expansion and M402 checks are enabled.
    /// Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
    current_function_is_transparent: bool,
    /// Current function's context requirements (from `using [...]` clause).
    /// Used to validate that function calls satisfy context requirements.
    /// Set at function boundary, None when outside function scope.
    current_function_contexts: Maybe<ContextSet>,
    /// Current function's call sites for call graph building.
    /// Maps callee name to call site information.
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    current_function_call_sites: Map<Text, crate::context_check::CallSiteInfo>,
    /// Current function being checked (for call graph registration)
    current_function_being_checked: Maybe<Text>,
    /// Initialization tracker for definite assignment analysis.
    /// Tracks partial initialization of compound types (tuples, arrays, structs).
    /// Spec: L0-critical/memory-safety/uninitialized - Compile-time partial init detection
    init_tracker: crate::context::InitTracker,
    /// Affine type tracker for move semantics enforcement.
    /// Ensures affine values are used at most once, detecting use-after-move.
    /// Spec: L0-critical/reference_system/value_transfer - Affine type safety
    pub(crate) affine_tracker: crate::affine::AffineTracker,
    /// Borrow tracker for reference aliasing detection.
    /// Ensures borrowing rules are followed: at most one &mut OR multiple &T.
    /// Spec: L0-critical/reference_system/access_rules - Reference aliasing safety
    borrow_tracker: crate::aliasing::BorrowTracker,
    /// NLL context flag: true when processing function call arguments.
    /// When true, mutable borrows use NLL behavior (release field borrows).
    in_call_arg_context: bool,
    /// Set of (type_name, method_name) pairs where the method takes `self` by value
    /// (SelfValue, SelfValueMut, SelfOwn, SelfOwnMut), meaning the receiver is consumed.
    /// Populated during impl block registration. Used by affine tracking.
    self_by_value_methods: std::collections::HashSet<(Text, Text)>,
    /// Unsafe context flag: true when inside an unsafe block.
    /// Required for creating Tier 2 (unsafe) references.
    /// Spec: L0-critical/reference_system/reference_tiers/unsafe_without_block
    in_unsafe_context: bool,
    /// When true, functions being type-checked are from implement blocks
    /// and should NOT be registered as standalone functions in the environment.
    in_impl_block: bool,
    /// Set of type names currently being registered.
    /// Prevents infinite recursion when registering mutually recursive types.
    types_being_registered: std::collections::HashSet<Text>,
    /// Set of (module_path, item_name) pairs currently being imported.
    /// Prevents infinite recursion when importing types with circular dependencies.
    /// When a circular import is detected, a warning is issued and the import is skipped.
    /// Circular import handling: detection and error reporting for cyclic module dependencies — Circular Import Handling
    imports_in_progress: std::collections::HashSet<(Text, Text)>,
    /// Set of module paths currently being glob-expanded (`mount foo.*`).
    /// Prevents unbounded recursion when a module's glob expansion transitively
    /// re-enters the same module via `public mount .sibling;` re-exports.
    /// Visit order is preserved so the error path reads "A → B → A".
    /// Guarded by: `import_all_from_module`, `import_all_from_inline_module`.
    /// Emits: `TypeError::ImportCycle` (E0811).
    glob_imports_in_progress: std::collections::HashSet<Text>,
    /// Visit-order trace of glob expansions currently on the stack — used to
    /// render the cycle path in the `ImportCycle` diagnostic. Invariant:
    /// the HashSet above contains exactly the entries of this Vec.
    glob_imports_stack: Vec<Text>,
    /// Set of (module_path, item_name) pairs currently being resolved via
    /// re-export chains (`resolve_export_kind_with_reexports` /
    /// `find_function_with_source_module`). Prevents unbounded recursion when
    /// A re-exports via B and B re-exports via A for the same item name.
    reexport_resolution_in_progress: std::collections::HashSet<(Text, Text)>,
    /// Tracks imported names and their source modules for ambiguity detection.
    /// Maps name -> list of source module paths.
    /// If a name has more than one source, it's ambiguous.
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Ambiguity
    pub(crate) imported_names: Map<Text, List<verum_common::Text>>,
    /// Tracks constants currently being evaluated for cycle detection.
    /// When evaluating a constant, if it depends on another constant that is
    /// also being evaluated, we have a circular dependency (E600 error).
    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Constant Initialization Order
    constants_being_evaluated: std::collections::HashSet<Text>,
    /// Maps constant full path to the set of constants it depends on.
    /// Used for cycle detection after all constants are processed.
    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Constant Initialization Order
    constant_dependencies: Map<Text, std::collections::HashSet<Text>>,
    /// The full path of the constant currently being checked (if any).
    /// Used to record dependencies when referencing other constants.
    current_constant_path: Maybe<Text>,
    /// Maps imported names to their full source paths for dependency tracking.
    /// E.g., "VALUE_B" -> "cog.const_b.VALUE_B"
    imported_constant_paths: Map<Text, Text>,
    /// Termination checker for unbounded recursion detection (E321)
    /// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
    termination_checker: crate::termination::TerminationChecker,
    /// Maps function names to their minimum required argument count.
    /// This supports default parameter values - functions can be called with
    /// fewer arguments than total params if remaining params have defaults.
    /// Spec: Grammar default_value in function_param
    function_required_params: Map<Text, usize>,
    /// Maps function names to their requires/ensures contract clauses.
    /// Used to check preconditions at call sites and postconditions at returns.
    /// Function contracts: preconditions (requires) and postconditions (ensures)
    function_contracts: Map<Text, FunctionContract>,
    /// Maps function names to the ordered list of their parameter names.
    ///

    /// Populated by `register_function_signature` for every function the
    /// type checker sees. This is separate from `function_contracts` which
    /// only stores entries for functions with explicit `requires`/`ensures`
    /// clauses; here we keep parameter names for *every* function so that
    /// dependent refinement enforcement (see the call-site loop around
    /// line 10558) can substitute earlier argument values into subsequent
    /// parameters' refinement predicates.
    ///

    /// Example: for `fn safe_get(len: Int, i: Int{< len}) -> Int`, this
    /// stores `safe_get → [len, i]` so that at a call `safe_get(5, 10)`
    /// the refinement checker can substitute `len → 5` into `i < len`
    /// before checking the second argument against the predicate.
    ///

    /// Empty entries (functions where names couldn't be extracted —
    /// e.g. destructuring patterns, or closures) are acceptable — the
    /// enforcement path falls back to the pre-existing non-dependent
    /// behaviour for those calls.
    function_param_names: Map<Text, List<Text>>,
    /// MLS classification per parameter for every registered
    /// function (#293 Phase 2b-Final). Parallel structure to
    /// `function_param_names` — index `i` of the `List<MlsLevel>`
    /// corresponds to index `i` of `function_param_names[fn]`.
    /// Public is the no-classification baseline so unclassified
    /// parameters share the same default as untracked bindings
    /// (lattice JOIN identity element).
    ///

    /// Populated by `register_function_signature` from each
    /// parameter's `@classification(<level>)` attribute. Read at
    /// call sites by `synth_call` / `check_app` to enforce the
    /// down-flow contract: the argument expression's classification
    /// must subsume the parameter's required level (Public is the
    /// most-permissive sink — accepts anything).
    function_param_classifications: Map<Text, List<verum_common::mls::MlsLevel>>,
    /// Stdlib metadata loaded from stdlib.vbc.
    /// Contains type definitions, protocols, and methods for stdlib types.
    /// Used for type checking user code that uses stdlib types.
    ///
    /// Held as `Arc<CoreMetadata>` so production callers (which receive
    /// `Arc<CoreMetadata>` from the pipeline's lazy embedded sidecar)
    /// can hand it off without paying the 15ms cost of a 3MB deep
    /// clone.  Reads dereference the `Arc` transparently.
    core_metadata: Maybe<std::sync::Arc<crate::core_metadata::CoreMetadata>>,
    /// Lazy module resolver for on-demand module loading.
    /// When a module import fails because the module isn't in the registry,
    /// this resolver is called to load the module on-demand.
    /// This enables lazy loading of any module - stdlib, dependencies, or local.
    /// File system to module mapping: lib.vr/main.vr is cog root, foo.vr defines module foo, foo/bar.vr defines foo.bar
    lazy_resolver: Option<verum_modules::SharedModuleResolver>,
    /// Session registry for lazy module loading.
    /// This is the shared RwLock-wrapped registry from the session/pipeline.
    /// When lazy loading is triggered, modules are registered here, and then
    /// the local module_registry is refreshed from this shared registry.
    session_registry: Option<Shared<parking_lot::RwLock<ModuleRegistry>>>,
    /// Operator-to-protocol mapping for stdlib-agnostic operator resolution.
    /// Maps operators (Add, Sub, Eq, etc.) to their corresponding protocols,
    /// enabling protocol-based type checking instead of hardcoded type names.
    /// Stdlib-agnostic type system: type checker operates without hardcoded knowledge of stdlib types, stdlib types registered from parsed .vr files
    operator_protocols: OperatorProtocols,
    /// Tracks number of generic parameters for each registered type.
    /// Used to fill in fresh type variables when a generic type is used
    /// without explicit type arguments (e.g., `PendingFuture` instead of `PendingFuture<T>`).
    type_generics_count: Map<Text, usize>,
    /// Refinement evidence tracker for flow-sensitive refinement propagation.
    /// Tracks learned predicates through control flow (e.g., after `if x.is_empty() { return }`,
    /// we know `!x.is_empty()` holds in the continuation).
    /// Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Types Enhancement
    refinement_evidence: crate::refinement_evidence::RefinementEvidence,
    /// Prototype mode flag: when true, certain type errors become warnings.
    /// This is activated by the @prototype attribute on functions or modules.
    /// @prototype mode: relaxed type checking for rapid prototyping, deferred refinement verification — @prototype Mode
    ///

    /// Behavior changes in prototype mode:
    /// - Unknown field access → WARNING + infer type
    /// - Missing type annotations → WARNING + infer type
    /// - Ambiguous types → WARNING + pick default
    /// - Explicit type mismatches → ERROR (unchanged for safety)
    prototype_mode: bool,
    /// Implicit argument resolution context.
    /// Tracks implicit parameters across nested scopes for dependent type inference.
    /// Implicit arguments: compiler-inferred function arguments resolved by unification or type class search — Implicit Arguments
    implicit_context: crate::implicit::ImplicitContext,
    /// Tracks variable names declared with `let mut`.
    /// Used to skip static upper-bound array index checks for mutable arrays
    /// that may have been resized via push/pop at runtime.
    /// Spec: L0-critical/memory-safety/array_bounds
    mutable_bindings: std::collections::HashSet<String>,
    /// Active pattern declarations registered during item collection.
    /// Maps pattern name to (param types, return type) for type checking
    /// active pattern invocations in match arms.
    /// Spec: grammar/verum.ebnf line 1817 - pattern_def
    pub(crate) pattern_declarations: Map<Text, (List<Type>, Type)>,
    /// HIT path-constructor metadata. Keyed by the higher-inductive
    /// type name (e.g., `"Circle"`, `"Interval"`); each entry holds
    /// the parsed path-constructors with their endpoint expressions.
    /// Populated during type registration whenever a variant carries
    /// `Variant::path_endpoints`. Consumed by HIT-aware tactics
    /// (cubical, descent) that need to know the topology of the type.
    pub hit_path_constructors: Map<Text, List<crate::ty::PathConstructor>>,
    /// When true, the type checker is in stdlib single-file mode.
    /// Field/method resolution failures are accepted with
    /// Type::Unknown rather than erroring — sibling module types
    /// may not be fully loaded. Set by pipeline for core/*.vr.
    pub stdlib_single_file_mode: bool,
    /// Deferred verification goals accumulated during type checking.
    /// When the unifier encounters a `Type::Eq` that can't be
    /// resolved even after the cubical bridge, it pushes a
    /// `CubicalEquality` goal here. When the universe solver
    /// encounters undecided constraints, it pushes
    /// `UniverseConstraints`. The pipeline drains this after
    /// type checking and feeds the goals into the
    /// `DependentVerifier` orchestrator.
    pub deferred_verification_goals: List<DeferredVerificationGoal>,
    /// When true, variant short-name protection uses relaxed rules that allow
    /// user-defined monomorphic unit variants to shadow polymorphic stdlib unit
    /// variants (e.g., user's `Status.Pending` overrides `Poll.Pending`).
    /// During stdlib loading, stricter rules protect `Maybe.None` etc. from
    /// being overridden by other stdlib types' unit variants.
    user_code_phase: bool,
    /// Tracks names that were imported via explicit braced imports (`mount foo.{Bar}`).
    /// When a name is in this set, glob imports (`mount foo.*`) and internal/transitive
    /// imports will NOT overwrite it. This prevents name collisions like the Ordering
    /// conflict where `core.base.ordering.Ordering` (Less|Equal|Greater) gets overwritten
    /// by `core.sync.atomic.Ordering` (Relaxed|Acquire|Release|AcqRel|SeqCst).
    explicit_imports: std::collections::HashSet<String>,
    /// Flag indicating we are currently inside a `register_type_declaration` call
    /// triggered by an explicit import (`mount foo.{Bar}`). When true, the provenance
    /// check in register_type_declaration_body allows the registration to proceed
    /// (overwriting whatever was previously registered for that name).
    /// When false, the check blocks registrations that would overwrite an explicitly
    /// imported type with a different variant structure.
    in_explicit_import_registration: bool,
    /// Skolem tracker for existential type scope management.
    skolem_tracker: crate::existential::SkolemTracker,
    /// Cfg evaluator for conditional compilation.
    /// Used to skip type-checking of @cfg-gated items that don't match the current platform.
    cfg_evaluator: verum_ast::cfg::CfgEvaluator,
    /// Unified recursion depth counter for type inference.
    /// Tracks combined depth across check_expr, infer_expr, and synth_expr
    /// to prevent stack overflow from mutual recursion between these functions.
    /// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
    inference_depth: Cell<u32>,
}

/// Generator context tracking for yield expressions
/// Generator functions: fn* syntax yields values lazily, producing Iterator<Item=T> types
#[derive(Debug, Clone)]
struct GeneratorContext {
    /// Type of values yielded by the generator
    yield_ty: Type,
    /// Final return type of the generator
    return_ty: Type,
}

/// Deferred constraint for improved constraint solving
///

/// These constraints are collected during type inference when they cannot
/// be solved immediately (e.g., because type variables are not yet resolved).
/// They are revisited after more type information becomes available.
///

/// Constraint-based type inference: collect type constraints from expressions and solve via unification
#[derive(Debug, Clone, PartialEq)]
pub enum DeferredConstraint {
    /// Type equality constraint: t1 = t2
    /// Deferred when both sides contain unresolved type variables
    Equality { left: Type, right: Type, span: Span },
    /// Protocol bound constraint: T: Protocol
    /// Deferred when T is an unresolved type variable
    ProtocolBound {
        ty: Type,
        protocol: Text,
        span: Span,
    },
    /// Subtype constraint: t1 <: t2
    /// Deferred when either side contains unresolved type variables
    Subtype { sub: Type, super_: Type, span: Span },
    /// Method constraint: T has method M with signature S
    /// Used for protocol method resolution on type variables
    HasMethod {
        receiver_ty: Type,
        method_name: Text,
        expected_signature: Type,
        span: Span,
    },
    /// Associated type projection constraint: T.Item = R
    /// Deferred when the base type T is an unresolved type variable
    ///

    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    ///

    /// This constraint arises when we encounter a projection like `T.Item`
    /// but T is not yet known. Once T is resolved, we can look up the
    /// protocol implementation and resolve the associated type.
    Projection {
        /// The deferred projection containing base type and associated type name
        deferred: crate::projection::DeferredProjection,
        /// The type variable representing the projection result
        result_var: TypeVar,
        /// Source span for error messages
        span: Span,
    },
    /// Associated type bound constraint: T.Item: Protocol
    /// Deferred when the projection cannot be resolved yet
    ///

    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    ProjectionBound {
        /// The projection (e.g., T.Item)
        projection: crate::projection::Projection,
        /// The protocol that the projection result must implement
        protocol: Text,
        /// Source span for error messages
        span: Span,
    },
}

/// A single step in a type conversion path.
///

/// E0204 Multiple conversion paths: when try (?) operator finds multiple From implementations for error conversion, requiring explicit disambiguation — E0204 Multiple conversion paths
///

/// Represents one From<source> for target implementation in a conversion chain.
///

/// # Visibility
///

/// This struct is `pub` to enable external testing but is not part of the stable API.
#[derive(Debug, Clone)]
pub struct ConversionStep {
    /// Source type being converted from
    pub from_type: Type,
    /// Target type being converted to
    pub to_type: Type,
    /// Span of the From implementation for diagnostic purposes
    pub impl_span: Span,
}

/// A complete conversion path from source to target type.
///

/// E0204 Multiple conversion paths: when try (?) operator finds multiple From implementations for error conversion, requiring explicit disambiguation — E0204 Multiple conversion paths
///

/// Represents a sequence of From implementations that convert from one type to another.
/// Used for detecting ambiguous conversion paths in the ? operator.
///

/// # Visibility
///

/// This struct is `pub` to enable external testing but is not part of the stable API.
#[derive(Debug, Clone)]
pub struct ConversionPath {
    /// Sequence of conversion steps in this path
    pub steps: List<ConversionStep>,
    /// Current type at the end of this path (for BFS traversal)
    pub current_type: Type,
    /// Set of visited types to detect cycles
    pub visited: std::collections::HashSet<Text>,
}

/// Maximum unified inference depth across check_expr, infer_expr, and synth_expr.
/// This prevents stack overflow from mutual recursion between these functions.
/// 100 allows complex but finite type checking (large record types with 30-40 fields,
/// deeply nested if-else chains, async chains, contexts, generics)
/// while preventing stack overflow (each level ~8-16KB of stack).
const MAX_INFERENCE_DEPTH: u32 = 100;

/// RAII guard for the TypeChecker inference depth counter.
/// Decrements the counter on drop, ensuring correct behavior even on panic/early-return.
/// Uses a raw pointer to the Cell to avoid holding a borrow on `self`, which would
/// conflict with the mutable borrows needed by the inner type-checking functions.
struct InferenceDepthGuard {
    depth_cell: *const Cell<u32>,
}

// SAFETY: The guard is only created from a reference to TypeChecker.inference_depth
// and is always dropped within the same function scope, so the Cell outlives the guard.
unsafe impl Send for InferenceDepthGuard {}

impl Drop for InferenceDepthGuard {
    fn drop(&mut self) {
        // SAFETY: The pointer is valid because the guard's lifetime is bounded
        // by the function that created it, and inference_depth lives in TypeChecker.
        let cell = unsafe { &*self.depth_cell };
        let d = cell.get();
        if d > 0 {
            cell.set(d - 1);
        }
    }
}

impl TypeChecker {
    /// QTT validation: walk a function body and confirm every
    /// declared binding's runtime usage matches its declared
    /// quantity. Returns `Ok(usage_map)` on success or the first
    /// `QttViolation` encountered (alphabetically first, for
    /// deterministic diagnostics).
    ///

    /// This is the integration entry point: callers (codegen,
    /// LSP, `@verify(formal)` boundary) supply the per-binding
    /// declared quantities (most often derived from explicit
    /// `meta` / `&checked` annotations or function-signature
    /// `Quantity` fields) plus the body, and the checker validates
    /// them against the QTT calculus.
    pub fn check_function_qtt(
        &self,
        declarations: &std::collections::HashMap<Text, crate::ty::Quantity>,
        body: &verum_ast::expr::Expr,
    ) -> std::result::Result<crate::qtt_usage::UsageMap, crate::qtt_usage::QttViolation> {
        let tracked: std::collections::HashSet<Text> = declarations.keys().cloned().collect();
        let usage = crate::qtt_walker::walk_expr(&tracked, body);
        crate::qtt_usage::check_usage(declarations, &usage)?;
        Ok(usage)
    }

    /// Increment the unified inference depth counter and return an RAII guard.
    /// The guard decrements the counter on drop, preventing stuck counters
    /// after panics or early returns.
    #[inline]
    fn inc_inference_depth(&self, context: &str) -> Result<InferenceDepthGuard> {
        let depth = self.inference_depth.get() + 1;
        self.inference_depth.set(depth);
        if depth > MAX_INFERENCE_DEPTH {
            self.inference_depth.set(depth - 1);
            return Err(TypeError::Other(Text::from(format!(
                "type inference recursion limit exceeded in {} (depth {})",
                context, depth
            ))));
        }
        Ok(InferenceDepthGuard {
            depth_cell: &self.inference_depth as *const Cell<u32>,
        })
    }

    /// Decrement the unified inference depth counter.
    /// Kept for any remaining manual usage, but prefer the RAII guard from inc_inference_depth.
    #[inline]
    fn dec_inference_depth(&self) {
        let d = self.inference_depth.get();
        if d > 0 {
            self.inference_depth.set(d - 1);
        }
    }

    /// Create a TypeChecker with language primitives only.
    ///

    /// STDLIB-AGNOSTIC: This constructor does NOT include hardcoded stdlib types.
    /// Stdlib types (Maybe, Result, List, etc.) are loaded dynamically from:
    /// - core/*.vr source files (via pipeline.load_stdlib_modules())
    /// - Pre-compiled VBC archives (via CoreMetadata)
    ///

    /// Only language primitives (Bool, Unit) and compiler intrinsics are included.
    pub fn new() -> Self {
        Self {
            ctx: TypeContext::new(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            // STDLIB-AGNOSTIC: Use new_minimal() instead of new() to avoid hardcoded types.
            // Type constructors are registered when stdlib is loaded from source files.
            kind_inferer: crate::kind_inference::KindInferer::new_minimal(),
            metrics: TypeCheckMetrics::new(),
            generator_context: Maybe::None,
            diagnostics: List::new(),
            deferred_soundness_errors: Vec::new(),
            glob_import_provenance: std::collections::HashMap::new(),
            current_cog_name: verum_common::Text::from(""),
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            higher_kinded_protocols_enabled: false,
            generic_associated_types_enabled: false,
            classification_map: std::collections::HashMap::new(),
            module_resolver: NameResolver::new(),
            module_registry: Shared::new(parking_lot::RwLock::new(ModuleRegistry::new())),
            current_module_path: verum_common::Text::from("cog"),
            inline_modules: Map::new(),
            module_aliases: Map::new(),
            preregistered_modules: std::collections::HashSet::new(),
            blanket_impls_registered_modules: std::collections::HashSet::new(),
            primitive_impls_registered_modules: std::collections::HashSet::new(),
            module_context: Maybe::None,
            current_function_return_type: Maybe::None,
            current_function_name: Maybe::None,
            current_function_return_span: Maybe::None,
            current_function_params: Set::new(),
            type_registry: crate::type_registry::TypeRegistry::new(),
            context_resolver: crate::context_resolution::ContextResolver::new(),
            current_self_type: Maybe::None,
            capability_checker: crate::capability::CapabilityChecker::new(),
            context_declarations: Map::new(),
            method_capability_mapper: crate::capability::MethodCapabilityMapper::new(),
            inherent_methods: Shared::new(parking_lot::RwLock::new(Map::new())),
            method_impl_patterns: Shared::new(parking_lot::RwLock::new(Map::new())),
            variance_checker: crate::variance::VarianceChecker::new(),
            variant_type_names: Map::new(),
            variant_collision_log: List::new(),
            variant_record_fields: Map::new(),
            variant_constructor_parents: Map::new(),
            type_var_bounds: Map::new(),
            hkt_type_var_by_name: Map::new(),
            type_var_type_bounds: Map::new(),
            deferred_constraints: List::new(),
            property_inferrer: crate::computational_properties::PropertyInferrer::new(),
            in_async_context: false,
            try_recover_depth: 0,
            current_function_throws: Maybe::None,
            context_checker: ContextChecker::new(),
            stage_checker: StageChecker::with_defaults(),
            current_function_stage: 0,
            current_function_is_transparent: false,
            current_function_contexts: Maybe::None,
            current_function_call_sites: Map::new(),
            current_function_being_checked: Maybe::None,
            init_tracker: crate::context::InitTracker::new(),
            affine_tracker: crate::affine::AffineTracker::with_core(),
            borrow_tracker: crate::aliasing::BorrowTracker::new(),
            in_call_arg_context: false,
            self_by_value_methods: std::collections::HashSet::new(),
            in_unsafe_context: false,
            in_impl_block: false,
            types_being_registered: std::collections::HashSet::new(),
            imports_in_progress: std::collections::HashSet::new(),
            glob_imports_in_progress: std::collections::HashSet::new(),
            glob_imports_stack: Vec::new(),
            reexport_resolution_in_progress: std::collections::HashSet::new(),
            imported_names: Map::new(),
            constants_being_evaluated: std::collections::HashSet::new(),
            constant_dependencies: Map::new(),
            current_constant_path: Maybe::None,
            imported_constant_paths: Map::new(),
            termination_checker: crate::termination::TerminationChecker::new(),
            function_required_params: Map::new(),
            function_contracts: Map::new(),
            function_param_names: Map::new(),
            function_param_classifications: Map::new(),
            core_metadata: Maybe::None,
            lazy_resolver: None,
            session_registry: None,
            operator_protocols: OperatorProtocols::standard(),
            type_generics_count: Map::new(),
            refinement_evidence: crate::refinement_evidence::RefinementEvidence::new(),
            prototype_mode: false,
            implicit_context: crate::implicit::ImplicitContext::new(),
            mutable_bindings: std::collections::HashSet::new(),
            pattern_declarations: Map::new(),
            hit_path_constructors: Map::new(),
            deferred_verification_goals: List::new(),
            stdlib_single_file_mode: false,
            user_code_phase: false,
            explicit_imports: std::collections::HashSet::new(),
            in_explicit_import_registration: false,
            skolem_tracker: crate::existential::SkolemTracker::new(),
            cfg_evaluator: verum_ast::cfg::CfgEvaluator::new(),
            inference_depth: Cell::new(0),
        }
    }

    /// Create a new type checker with stdlib metadata loaded from stdlib.vbc.
    ///

    /// This is the PREFERRED constructor for compiling user code.
    /// Types and methods are loaded from stdlib.vbca metadata, enabling
    /// type checking of stdlib types without parsing .vr source files.
    pub fn new_with_core(
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) -> Self {
        // T2-extended-perf: defer eager `load_stdlib_from_metadata`
        // (~3.8s on release cold start, ~5.3s debug).  The
        // pipeline's `phase_type_check` calls
        // `register_stdlib_types_for_module(user_module)` which
        // pre-loads ONLY the stdlib types the user code actually
        // references (~10s of types vs 1000+).  Drops cold-start
        // typecheck from 3.8s → ~50ms for a hello.vr-style script.
        //
        // Production callers MUST drive `register_stdlib_types_for_module`
        // before the main typecheck pass.  Audit/corpus tooling
        // that needs the entire stdlib pre-registered upfront uses
        // [`new_with_core_eager`](Self::new_with_core_eager) instead.
        //
        // Takes `Arc<CoreMetadata>` so production callers (which
        // hold an `Arc` from the pipeline's lazy embedded sidecar)
        // hand off the metadata in O(1) — no 15ms 3MB deep clone.
        let mut checker = Self::with_minimal_context();
        checker.core_metadata = Maybe::Some(metadata);
        checker
    }

    /// Hand stdlib metadata to a TypeChecker constructed via a
    /// non-`new_with_core` path (e.g.
    /// [`with_shared_methods`](Self::with_shared_methods) or
    /// [`with_minimal_context`](Self::with_minimal_context)).
    ///
    /// Required so the receiver-driven lazy stdlib-type loader
    /// (`infer_method_call_inner_impl`'s `ensure_stdlib_type_loaded`
    /// call on the receiver's type-name) actually has a metadata
    /// table to pull from.  Without it, every method call on a
    /// stdlib type that wasn't named explicitly by user code (e.g.
    /// `pool.acquire().await?` returning `AsyncPgPoolGuard`
    /// inferred indirectly through `Result.Ok` arm) fails the
    /// inherent-method bucket lookup and surfaces as
    /// `MethodNotFound` despite the bodies being in the
    /// precompiled archive.
    pub fn set_core_metadata(
        &mut self,
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) {
        self.core_metadata = Maybe::Some(metadata);
    }

    /// Eager construction — registers every type/protocol/function
    /// from the supplied metadata upfront.  ~3.8s on release cold
    /// start.  Used by audit / corpus tooling that needs the
    /// entire stdlib pre-registered regardless of what user code
    /// references.  Production `verum run` / `verum build` use the
    /// lazy [`new_with_core`](Self::new_with_core) path instead.
    pub fn new_with_core_eager(
        metadata: std::sync::Arc<crate::core_metadata::CoreMetadata>,
    ) -> Self {
        let mut checker = Self::with_minimal_context();
        checker.load_stdlib_from_metadata(&metadata);
        checker.core_metadata = Maybe::Some(metadata);
        checker
    }

    /// Pre-scan a user module AST for every named type / protocol /
    /// generic-type-arg, and register each from `core_metadata` if
    /// not yet loaded.  Pairs with [`Self::new_with_core_lazy`] so a
    /// single pre-pass covers the entire typecheck of user code.
    ///
    /// O(names_in_module) — typically tens to low hundreds for a
    /// real script, vs O(stdlib_total) ≈ 1000+ for the eager path.
    pub fn register_stdlib_types_for_module(&mut self, module: &verum_ast::Module) {
        if matches!(self.core_metadata, Maybe::None) {
            return;
        }
        let mut needed: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        for item in module.items.iter() {
            collect_named_types_from_item(item, &mut needed);
        }
        // Drain the set into the registration loop.  Each `ensure`
        // call may register additional dependencies (variant payload
        // types, field types, super-protocols) that were transitively
        // referenced — those are picked up by repeated passes until
        // the set stabilises.
        let mut to_load: Vec<Text> = needed.into_iter().collect();
        let mut already: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        while let Some(name) = to_load.pop() {
            if !already.insert(name.clone()) {
                continue;
            }
            self.ensure_stdlib_type_loaded(&name, &mut to_load);
        }

        // Register inductive constructors for pattern matching.
        // The eager `load_stdlib_from_metadata` calls this at the end
        // of its single-pass walk; the lazy path needs the same hook
        // so `match res { Ok(p) => …, Err(e) => … }` patterns
        // resolve against Result's variant body, and `Ok(x)` /
        // `Err(e)` / `Some(x)` value-position uses bind to the
        // constructor.  Without this, every pattern match on
        // Result/Maybe/IoResult fails `Pattern expects a variant
        // type, but scrutinee has type Result/IoResult`.
        let metadata = match &self.core_metadata {
            Maybe::Some(m) => m.clone(),
            Maybe::None => return,
        };
        self.register_stdlib_constructors_from_metadata(&metadata);

        // Unconditionally register every type alias from metadata
        // into the unifier's alias registry.  Aliases are cheap
        // (single TypeRef payload + param-name list) and a user
        // script may reference an alias indirectly through a
        // function's return type without ever naming the alias
        // directly — `fs_metadata(p) -> IoResult<Metadata>` returns
        // IoResult but the user code never writes "IoResult".  The
        // alias needs to be in the registry by the time pattern
        // matching expands the type.
        for (alias_name, td) in metadata.types.iter() {
            if let crate::core_metadata::TypeDescriptorKind::Alias { target } = &td.kind {
                let target_ty = parse_descriptor_type_string(target.as_str());
                self.ctx
                    .define_alias(alias_name.clone(), target_ty.clone());
                self.unifier
                    .register_type_alias(alias_name.clone(), target_ty);
                let param_names: List<Text> = td
                    .generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect();
                if !param_names.is_empty() {
                    self.unifier
                        .register_type_alias_params(alias_name.clone(), param_names);
                }
            }
        }
    }

    /// Register a single stdlib type from `core_metadata` if it's
    /// declared there and not yet registered.  Pushes any
    /// transitively-referenced type names into `pending` so the
    /// caller's loop can continue until the closure stabilises.
    /// Idempotent under repeated calls (already-loaded types
    /// short-circuit on `ctx.lookup_type`).
    pub fn ensure_stdlib_type_loaded(
        &mut self,
        name: &Text,
        pending: &mut Vec<Text>,
    ) {
        let metadata = match &self.core_metadata {
            Maybe::Some(m) => m.clone(),
            Maybe::None => return,
        };
        let type_desc = match metadata.types.get(name) {
            Some(td) => td.clone(),
            None => return,
        };
        // For built-in primitive types (Text, Int, Float, Bool, …)
        // already registered by `register_builtins`, the type
        // definition does NOT need re-registration — but its
        // inherent-method bucket DOES need population from
        // `core_metadata`, otherwise method-call typecheck
        // (`text.push_str(...)`, `text.trim()`, …) fails despite
        // the bodies being present in the precompiled archive.
        // Run the type-definition registration only when the type
        // isn't yet in ctx, but ALWAYS run inherent-method
        // registration (which is idempotent — skips already-
        // populated method names).
        if self.ctx.lookup_type(name.as_str()).is_none() {
            // Convert + register this single type.  Mirror of the body
            // of `load_stdlib_from_metadata` reduced to one entry.
            let ty = self.type_descriptor_to_type(&type_desc);
            self.ctx.define_type(name.clone(), ty.clone());
            self.ctx.env.insert(name.clone(), TypeScheme::mono(ty.clone()));

            // Mirror the eager loader's `type_generics_count` write.
            // Without this, `try_resolve_variant_constructor` falls
            // into its `generics_count == 0` branch and synthesises
            // bare `Type::Named { path: parent, args: [] }` for the
            // constructor return — every `Ok(x)` / `Err(e)` /
            // `Some(x)` value-position use against an expected
            // `Result<T, E>` then fails unification with "found
            // 'Result'" because the synthesised type carries no
            // type arguments.  The eager loader ran this at
            // `load_stdlib_from_metadata` (line 1981-1984); the
            // lazy `ensure_stdlib_type_loaded` path missed it.
            if !type_desc.generic_params.is_empty() {
                self.type_generics_count
                    .insert(name.clone(), type_desc.generic_params.len());
            }

            // Type alias: also register in BOTH the ctx's alias
            // registry AND the unifier's alias registry so
            // `try_expand_alias` and the pattern matcher can resolve
            // `IoResult<T>` → `Result<T, IoError>` etc.  Generic
            // alias param-name list goes through
            // `register_type_alias_params` so positional type
            // argument substitution works (`IoResult<Text>` →
            // `Result<Text, StreamError>`, not just
            // `Result<T, StreamError>`).
            //
            // Pre-fix every match pattern over a stdlib alias
            // (`match res { Ok(p) => …, Err(e) => … }` on
            // `IoResult<Text>`) failed `Pattern expects a variant
            // type, but scrutinee has type IoResult<...>`.
            if let crate::core_metadata::TypeDescriptorKind::Alias { target } =
                &type_desc.kind
            {
                let target_ty = Type::Named {
                    path: Self::text_to_path(target),
                    args: List::new(),
                };
                self.ctx.define_alias(name.clone(), target_ty.clone());
                self.unifier
                    .register_type_alias(name.clone(), target_ty);
                let param_names: List<Text> = type_desc
                    .generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect();
                if !param_names.is_empty() {
                    self.unifier
                        .register_type_alias_params(name.clone(), param_names);
                }
                pending.push(target.clone());
            }

            // Variant signatures — same logic as the eager loader, gated
            // to this single type.  Pushes payload type names into
            // `pending` so the loop registers them too.
            if let crate::core_metadata::TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                register_variant_signature_for_lazy(self, name, &type_desc, cases, pending);
            }
            // Record fields — push field type names into `pending`.
            if let crate::core_metadata::TypeDescriptorKind::Record { fields } = &type_desc.kind {
                for f in fields.iter() {
                    if !f.ty.is_empty() {
                        pending.push(f.ty.clone());
                    }
                }
            }
            // Generic param defaults — also dependencies.
            for gp in type_desc.generic_params.iter() {
                if let Maybe::Some(default_text) = &gp.default {
                    pending.push(default_text.clone());
                }
            }
        }

        // ALWAYS register inherent methods, even when the type
        // itself was already in ctx (e.g. primitives like Text,
        // List, Map registered via `register_builtins`).  Without
        // this unconditional pass, every `text.push_str(...)` /
        // `list.iter()` / `map.get()` call site fails `no method
        // named …` typecheck despite the bodies being in the
        // precompiled archive.  The pass is idempotent — skips
        // method names already populated in the
        // inherent_methods bucket.
        //
        // Method signatures push their referenced type names into
        // `pending` so dependent stdlib types (Path, Metadata,
        // IoResult, …) load transitively — needed for alias
        // resolution + variant pattern matching at user-code
        // call sites.
        let referenced = self.register_inherent_methods_from_metadata(name, &type_desc, &metadata);
        for r in referenced {
            pending.push(r);
        }

        // #130 — When the loaded type is itself a Protocol, register
        // its body (methods / super-protocols / type-params) into
        // `protocol_checker` so impl-registration consumers can look
        // up its method signatures.  The eager
        // `load_stdlib_from_metadata` registers every protocol
        // upfront via `metadata.protocols`; the lazy path needs the
        // same hook gated to this single name.
        if matches!(
            type_desc.kind,
            crate::core_metadata::TypeDescriptorKind::Protocol { .. }
        ) {
            self.register_stdlib_protocol_for_name(name, &metadata);
        }

        // #130 — register protocol implementations that target this
        // type.  Pre-fix the lazy loader registered the type
        // definition + inherent methods but NOT the protocol impls
        // recorded in `metadata.implementations`, so
        // `protocol_checker.get_implementations(IntoList<_>)` returned
        // empty — the canonical `xs.into_iter().map(f).collect()`
        // chain failed at type-check because the dispatcher had no
        // impl to walk for `IntoList<_>`.
        //
        // Pairs with the archive_metadata fix that populates
        // `ImplementationDescriptor.protocol` from the VBC type
        // table's `ProtocolId` field (was hardcoded `Text::default()`).
        //
        // Stdlib-agnostic per `crates/verum_types/src/CLAUDE.md`:
        // the impl list comes from `metadata.implementations`, not a
        // hardcoded mapping.  Adding `Foldable` / `Functor` / etc.
        // implementations to a stdlib type works identically.
        let proto_deps = self.register_stdlib_impls_for_target(name, &metadata);
        for proto_name in proto_deps {
            pending.push(proto_name);
        }
    }

    /// Register the body of a single stdlib protocol from
    /// `metadata.protocols[name]` into `protocol_checker`.  Idempotent
    /// (no-op if `protocol_checker.get_protocol(name)` already returns
    /// `Some`).  Mirror of the eager loop in
    /// `load_stdlib_from_metadata` lines ~2178-2275, gated to a
    /// single name for the lazy path.
    fn register_stdlib_protocol_for_name(
        &mut self,
        name: &Text,
        metadata: &crate::core_metadata::CoreMetadata,
    ) {
        use crate::protocol::{Protocol, ProtocolMethod};
        let protocol_desc = match metadata.protocols.get(name) {
            Some(d) => d,
            None => return,
        };

        // Conservative MERGE policy with `register_standard_protocols`
        // hardcoded baseline (in `crates/verum_types/src/protocol.rs:1973+`):
        //
        //  * If the existing protocol has ≥2 methods, it's a
        //    well-formed hardcoded entry (Eq/Ord/Show/PartialOrd/etc
        //    all seed multiple methods with hand-curated TypeVar
        //    shapes that round-trip correctly through the unifier).
        //    OVERWRITING those with VBCA-derived signatures breaks
        //    operator-protocol unification (Layer E gap: TypeRef::Generic
        //    method-local param ids render as concrete TypeIds via
        //    fallback to PTR / well-known-type-name — so e.g. PartialOrd's
        //    `lt(T, T) -> Bool` gets stomped by `lt(Heap<_,_>, Heap<_,_>)
        //    -> Bool`).  Skip in this case — keep the hardcoded entry.
        //  * If the existing protocol has 0 or 1 methods, it's a
        //    stub (e.g. Iterator hardcoded with only `next`).  Layer D
        //    populates the missing 73 default-method signatures.
        //    SUPPLEMENT in this case — preserve the hardcoded
        //    protocol's `associated_types` + `associated_consts`
        //    (the stub seeds `Iterator::Item`, etc.) but replace the
        //    methods table with the VBCA-derived 74 entries.
        //
        // This split is the cleanest co-existence with the legacy
        // hardcoded path until `register_standard_protocols` is
        // removed in favour of pure metadata-driven registration
        // (separate refactor; CLAUDE.md "no stdlib knowledge in
        // compiler" rule).
        let (existing_assoc_types, existing_assoc_consts, existing_method_count) = {
            let pc = self.protocol_checker.read();
            match pc.get_protocol(name) {
                Maybe::Some(p) => (
                    p.associated_types.clone(),
                    p.associated_consts.clone(),
                    p.methods.len(),
                ),
                Maybe::None => (verum_common::Map::new(), verum_common::Map::new(), 0),
            }
        };
        // Skip when the hardcoded baseline is well-formed (≥2 methods).
        if existing_method_count >= 2 {
            return;
        }

        // Convert each metadata-derived parameter / return type
        // string back into a structured `Type`.  The
        // descriptor-string parser handles primitives, bare names,
        // generic instantiations (`Maybe<T>`), references (`&T`),
        // function types, and the `__opaque_type_N` /
        // `__generic_N` placeholders (mapped to fresh TypeVars).
        // After #131 Layer E landed, codegen emits proper
        // `TypeRef::Generic` for protocol-level params, method-
        // local params, AND associated-type projections
        // (`Self.Item`) — archive_metadata renders the unresolved
        // ones as `__generic_N` placeholders that the parser
        // converts to fresh TypeVars.  No additional safety guard
        // needed at this site.
        let to_type = |s: &Text| -> Type { parse_descriptor_type_string(s.as_str()) };
        let mut methods = verum_common::Map::new();
        for m in protocol_desc.required_methods.iter() {
            let params: List<Type> = m.params.iter().map(|p| to_type(&p.ty)).collect();
            let return_type = to_type(&m.return_type);
            let method_type = Type::function(params, return_type);
            let protocol_method =
                ProtocolMethod::simple(m.name.clone(), method_type, false);
            methods.insert(m.name.clone(), protocol_method);
        }
        for m in protocol_desc.default_methods.iter() {
            let params: List<Type> = m.params.iter().map(|p| to_type(&p.ty)).collect();
            let return_type = to_type(&m.return_type);
            let method_type = Type::function(params, return_type);
            let protocol_method =
                ProtocolMethod::simple(m.name.clone(), method_type, true);
            methods.insert(m.name.clone(), protocol_method);
        }

        let protocol = Protocol {
            name: name.clone(),
            kind: crate::protocol::ProtocolKind::Constraint,
            type_params: protocol_desc
                .generic_params
                .iter()
                .map(|g| crate::protocol::TypeParam {
                    name: g.name.clone(),
                    bounds: g
                        .bounds
                        .iter()
                        .map(|b| crate::protocol::ProtocolBound {
                            protocol: Self::text_to_path(b),
                            args: List::new(),
                            is_negative: false,
                        })
                        .collect(),
                    default: g.default.as_ref().map(|d| Type::Named {
                        path: Self::text_to_path(d),
                        args: List::new(),
                    }),
                })
                .collect(),
            methods,
            associated_types: existing_assoc_types,
            associated_consts: existing_assoc_consts,
            super_protocols: protocol_desc
                .super_protocols
                .iter()
                .map(|sp| crate::protocol::ProtocolBound {
                    protocol: Self::text_to_path(sp),
                    args: List::new(),
                    is_negative: false,
                })
                .collect(),
            specialization_info: Maybe::None,
            defining_crate: Maybe::None,
            span: Span::default(),
        };
        let _ = self.protocol_checker.write().register_protocol(protocol);
    }

    /// Register every protocol implementation in
    /// `metadata.implementations` that targets `type_name`.  Returns
    /// the set of protocol names referenced by these impls so the
    /// caller can push them onto the pending queue (so the protocol's
    /// own type-definition + body load before any subsequent
    /// dispatch tries to resolve a default method on the impl).
    /// Mirror of the eager loop in `load_stdlib_from_metadata` lines
    /// ~2278-2320, gated to a single target.
    fn register_stdlib_impls_for_target(
        &mut self,
        type_name: &Text,
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<Text> {
        use crate::protocol::ProtocolImpl;
        let mut proto_deps: Vec<Text> = Vec::new();
        for impl_desc in metadata.implementations.iter() {
            if impl_desc.target_type.as_str() != type_name.as_str() {
                continue;
            }
            if impl_desc.protocol.as_str().is_empty() {
                continue;
            }
            // Make sure the protocol body is registered so we can
            // pull its method-signature map.  Idempotent — a no-op
            // when this protocol was already loaded earlier.
            self.register_stdlib_protocol_for_name(&impl_desc.protocol, metadata);

            // Idempotent guard: skip if THIS impl (target_type,
            // protocol) was already registered.  The protocol-checker
            // uses (type-key, protocol-key) as its impl-index key —
            // duplicate registrations would still overwrite cleanly,
            // but skipping spares the repeated allocation.
            let for_type = Type::Named {
                path: Self::text_to_path(&impl_desc.target_type),
                args: List::new(),
            };
            {
                let pc = self.protocol_checker.read();
                if pc.get_implementations(&for_type).iter().any(|i| {
                    i.protocol
                        .as_ident()
                        .map(|id| id.as_str() == impl_desc.protocol.as_str())
                        .unwrap_or(false)
                }) {
                    proto_deps.push(impl_desc.protocol.clone());
                    continue;
                }
            }

            let associated_types: verum_common::Map<Text, Type> = impl_desc
                .associated_types
                .iter()
                .map(|(name, type_name)| {
                    let ty = Type::Named {
                        path: Self::text_to_path(type_name),
                        args: List::new(),
                    };
                    (name.clone(), ty)
                })
                .collect();

            let methods: verum_common::Map<Text, Type> = {
                let pc = self.protocol_checker.read();
                if let Maybe::Some(protocol) = pc.get_protocol(&impl_desc.protocol) {
                    protocol
                        .methods
                        .iter()
                        .map(|(name, method)| (name.clone(), method.ty.clone()))
                        .collect()
                } else {
                    verum_common::Map::new()
                }
            };

            let protocol_impl = ProtocolImpl {
                protocol: Self::text_to_path(&impl_desc.protocol),
                protocol_args: List::new(),
                for_type,
                where_clauses: List::new(),
                methods,
                associated_types,
                associated_consts: verum_common::Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: verum_common::Map::new(),
            };
            let _ = self.protocol_checker.write().register_impl(protocol_impl);
            proto_deps.push(impl_desc.protocol.clone());
        }
        proto_deps
    }

    /// Walk a parsed Type and push every named-type reference
    /// (Type::Named, Type::Generic.name, Type::Reference.inner …)
    /// into the lazy-loader's pending set.  Ensures that methods
    /// whose signatures reference stdlib type aliases (e.g.
    /// `fs_metadata(p: &Path) -> IoResult<Metadata>`) trigger
    /// loading of IoResult, Metadata, and Path so their alias
    /// targets and inductive-constructor registrations are
    /// available when the user code matches against them.
    /// On-demand stdlib type loader for method-dispatch sites.
    ///
    /// The pre-pass (`register_stdlib_types_for_module`) doesn't
    /// recurse into function bodies — `collect_named_types_from_function_body`
    /// is intentionally a no-op for performance.  Body-local
    /// pattern matches that bind variables of stdlib types (e.g.
    /// `match fs_metadata(p) { Ok(m) => m.len() }`) therefore reach
    /// method dispatch with the receiver type's `inherent_methods`
    /// bucket empty.  This helper closes that gap by walking every
    /// type name reachable from `recv_ty` and lazy-loading each
    /// through the same machinery the pre-pass uses.
    ///
    /// Idempotent — short-circuits at every layer:
    ///  * `ensure_stdlib_type_loaded` skips if `ctx.type_defs`
    ///    already has the type.
    ///  * `register_inherent_methods_from_metadata` skips method
    ///    names already populated in the bucket.
    pub fn lazy_load_receiver_methods(&mut self, recv_ty: &Type) {
        if self.core_metadata.is_none() {
            return;
        }
        let mut names: Vec<Text> = Vec::new();
        Self::push_referenced_type_names(recv_ty, &mut names);
        if names.is_empty() {
            return;
        }
        let mut pending: Vec<Text> = names;
        let mut already: std::collections::HashSet<Text> =
            std::collections::HashSet::new();
        while let Some(name) = pending.pop() {
            if !already.insert(name.clone()) {
                continue;
            }
            self.ensure_stdlib_type_loaded(&name, &mut pending);
        }
    }

    /// CoreMetadata-driven re-export resolver for free functions.
    ///
    /// Walks `ast`'s public mount declarations looking for a tree
    /// that re-exports `func_name`.  When found, derives the source
    /// module path from the mount's prefix and constructs the
    /// `module_path.func_name` key for `metadata.functions`.  If
    /// metadata holds that key, parse the descriptor's parameter
    /// + return strings into a `Type::Function` and return.
    ///
    /// Stops at the first match — multiple re-export chains
    /// shouldn't produce duplicate keys (the first-wins discipline
    /// in archive_metadata gives at most one qualified key per
    /// concrete (module_path, simple_name) pair).
    pub fn resolve_metadata_reexport_function(
        &self,
        metadata: &crate::core_metadata::CoreMetadata,
        ast: &verum_ast::Module,
        func_name: &str,
        ast_module_path: &Text,
    ) -> Option<Type> {
        use verum_ast::ItemKind;
        use verum_ast::decl::{MountTreeKind, Visibility as AstVisibility};
        use verum_ast::ty::PathSegment;
        for item in ast.items.iter() {
            let mount = match &item.kind {
                ItemKind::Mount(m) if m.visibility == AstVisibility::Public => m,
                _ => continue,
            };
            // Two surfaces: `pub mount .X.{name}` (Nested) and
            // `pub mount .X.name` (Path).
            let (matched_prefix_segments, matched) = match &mount.tree.kind {
                MountTreeKind::Nested { prefix, trees } => {
                    let mut found = false;
                    for tree in trees.iter() {
                        if let MountTreeKind::Path(item_path) = &tree.kind {
                            if let Some(PathSegment::Name(id)) = item_path.segments.last() {
                                if id.name.as_str() == func_name {
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !found {
                        continue;
                    }
                    let segs: Vec<&str> = prefix
                        .segments
                        .iter()
                        .filter_map(|s| match s {
                            PathSegment::Name(id) => Some(id.name.as_str()),
                            _ => None,
                        })
                        .collect();
                    (segs, true)
                }
                MountTreeKind::Path(path) => {
                    if let Some(PathSegment::Name(id)) = path.segments.last() {
                        if id.name.as_str() != func_name {
                            continue;
                        }
                    } else {
                        continue;
                    }
                    let segs: Vec<&str> = path
                        .segments
                        .iter()
                        .take(path.segments.len() - 1)
                        .filter_map(|s| match s {
                            PathSegment::Name(id) => Some(id.name.as_str()),
                            _ => None,
                        })
                        .collect();
                    (segs, true)
                }
                _ => continue,
            };
            if !matched {
                continue;
            }
            // Build `current_module.prefix.func_name`.  We don't
            // have direct access to the source module's path here,
            // but `self.current_module_path` reflects the unit
            // currently being typechecked — the usual case is the
            // caller passes ast = module_info.ast where module_info
            // is the directly-mounted module, so its path lives in
            // the resolved_module_path the caller already knows.
            // Pass that through current_module_path so this helper
            // can compose the qualified key.
            let module_path = matched_prefix_segments.join(".");
            // Compose candidate qualified keys for the
            // `metadata.functions` lookup.  The function's actual
            // recorded module_path is the SOURCE module — but the
            // function may have been re-exported through any number
            // of intermediate modules, so we have to try multiple
            // assumptions about which module owns the descriptor.
            //
            // Precedence:
            //   1. `<ast_module_path>.<func_name>` — the function
            //      was registered against the re-exporting module
            //      (most common for stdlib's `public module X;` +
            //      `public mount .X.{fn}` pattern, where the VBC
            //      module_path strips the file segment).
            //   2. `<ast_module_path>.<prefix>.<func_name>` — the
            //      function lives in the prefix submodule.
            //   3. `<prefix>.<func_name>` — absolute prefix.
            //   4. `core.<prefix>.<func_name>` — root prefix
            //      where the prefix already starts at `core`.
            let cur = ast_module_path.as_str();
            let mut candidates: Vec<String> = Vec::new();
            if !cur.is_empty() {
                candidates.push(format!("{}.{}", cur, func_name));
                if !module_path.is_empty() {
                    candidates.push(format!("{}.{}.{}", cur, module_path, func_name));
                }
            }
            if !module_path.is_empty() {
                candidates.push(format!("core.{}.{}", module_path, func_name));
                candidates.push(format!("{}.{}", module_path, func_name));
            }
            for key in candidates {
                let key_text: Text = key.into();
                if let Some(fd) = metadata.functions.get(&key_text) {
                    let params: List<Type> = fd
                        .params
                        .iter()
                        .map(|p| parse_descriptor_type_string(p.ty.as_str()))
                        .collect();
                    let return_ty = parse_descriptor_type_string(fd.return_type.as_str());
                    return Some(Type::function(params, return_ty));
                }
            }
        }
        None
    }

    fn push_referenced_type_names(ty: &Type, out: &mut Vec<Text>) {
        match ty {
            Type::Named { path, args } => {
                if let Some(seg) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(id) = seg {
                        out.push(Text::from(id.name.as_str()));
                    }
                }
                for a in args.iter() {
                    Self::push_referenced_type_names(a, out);
                }
            }
            Type::Generic { name, args } => {
                out.push(name.clone());
                for a in args.iter() {
                    Self::push_referenced_type_names(a, out);
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => {
                Self::push_referenced_type_names(inner, out)
            }
            Type::Tuple(elems) => {
                for e in elems.iter() {
                    Self::push_referenced_type_names(e, out);
                }
            }
            Type::Function { params, return_type, .. } => {
                for p in params.iter() {
                    Self::push_referenced_type_names(p, out);
                }
                Self::push_referenced_type_names(return_type, out);
            }
            _ => {}
        }
    }

    /// Lazy counterpart of the AST-driven `import_impl_blocks` pass.
    ///
    /// Walks `type_desc.methods` (populated by
    /// `archive_metadata::register_module_metadata` from VBC
    /// `FunctionDescriptor.parent_type`) and registers each
    /// `(method_name → method_type_scheme)` pair into the checker's
    /// `inherent_methods` table — same structure the AST path
    /// populates.  Without this step calls like
    /// `Text.with_capacity(n)` would fail typecheck even though
    /// the body is in the precompiled VBC archive.
    ///
    /// Idempotent: skips method names already present in
    /// `inherent_methods[name]` so repeated
    /// `ensure_stdlib_type_loaded` calls don't accumulate
    /// duplicates.
    fn register_inherent_methods_from_metadata(
        &self,
        type_name: &Text,
        type_desc: &crate::core_metadata::TypeDescriptor,
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<Text> {
        let mut referenced: Vec<Text> = Vec::new();
        if type_desc.methods.is_empty() {
            return referenced;
        }
        let mut methods_guard = self.inherent_methods.write();
        let bucket = methods_guard.entry(type_name.clone()).or_default();
        for method_name in type_desc.methods.iter() {
            if bucket.get(method_name).is_some() {
                continue;
            }
            // Prefer qualified `Type.method` lookup so a simple name
            // shared across multiple types (e.g. `with_capacity` on
            // Text + TextBuilder + List) resolves to the descriptor
            // belonging to THIS type.  Fall back to the bare simple
            // name for free functions / single-type methods.
            let qualified: Text = format!("{}.{}", type_name, method_name).into();
            let fn_desc = match metadata.functions.get(&qualified) {
                Some(d) => d,
                None => match metadata.functions.get(method_name) {
                    Some(d) => d,
                    None => continue,
                },
            };
            // Build the function type from the descriptor.
            // `parse_descriptor_type_string` handles primitives,
            // bare names, generic instantiations
            // (`IoResult<Metadata>`), and references (`&Text`)
            // — required because `archive_metadata::type_ref_to_text`
            // serialises VBC TypeRefs as joined strings, and a
            // naive `Type::Named { path: "IoResult<Metadata>" }`
            // would never unify with the call-site
            // `Type::Generic { name: "IoResult", args: [Metadata] }`.
            let to_type = |s: &Text| -> Type { parse_descriptor_type_string(s.as_str()) };
            // Skip the `self` receiver parameter when present.  VBC
            // stores method descriptors with the receiver as the
            // first parameter (named `"self"`) but its `type_ref`
            // is a `TypeId::UNIT` sentinel — `type_ref_to_text`
            // renders this as `"Unit"`, which would yield a method
            // signature `fn(Unit, …) -> R`.  Verum dispatches the
            // receiver separately during method resolution; the
            // inherent_methods bucket should hold ONLY the
            // call-site arg types.  Skipping `self` here aligns
            // with how `import_impl_blocks` (the AST-driven path)
            // populates the bucket.
            let params: List<Type> = fn_desc
                .params
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    if i == 0 && p.name.as_str() == "self" {
                        None
                    } else {
                        Some(to_type(&p.ty))
                    }
                })
                .collect();
            let return_ty = to_type(&fn_desc.return_type);
            // Recursively collect referenced type names so the
            // lazy loader registers any types this method's
            // signature touches (Path, Metadata, IoResult, …).
            for p in params.iter() {
                Self::push_referenced_type_names(p, &mut referenced);
            }
            Self::push_referenced_type_names(&return_ty, &mut referenced);
            let fn_ty = Type::function(params, return_ty);
            // Determine whether the method is static (no `self`
            // receiver).  Static-method dispatch sites
            // (`Text.with_capacity(64)`, `Heap.alloc(layout)` …)
            // read from the `$static$<method>` bucket key;
            // instance-method dispatch reads the bare key.  The
            // AST-driven registration path elsewhere in this file
            // follows the same convention.  Pre-fix every
            // metadata-loaded static method was registered ONLY
            // under the bare key, so every `Type.static_method(...)`
            // call site failed typecheck despite the body being in
            // the precompiled archive.
            let is_static = fn_desc
                .params
                .first()
                .map(|p| p.name.as_str() != "self")
                .unwrap_or(true);
            // Wrap in a polymorphic TypeScheme when the method's
            // signature contains free TypeVars (introduced by
            // `parse_descriptor_type_string`'s
            // `__generic_N`/`__opaque_type_N` → fresh-TypeVar
            // conversion).  These represent the method's generic
            // parameters: e.g. `Shared.new(value: T) -> Shared<T>`
            // gets `fn(_TyVar_a) -> Shared<_TyVar_a>` after
            // structural parsing.
            //
            // Pre-fix the method was registered as `mono(fn_ty)`,
            // which means the SAME TypeVars are reused on every
            // lookup.  First call site `Shared<Int>.new(42)` binds
            // `_TyVar_a := Int` in the unifier's substitution
            // table; second call site `Shared<Bool>.new(true)`
            // looks up the SAME scheme, sees `_TyVar_a` already
            // bound to `Int`, and rejects `true: Bool` with
            // `expected 'Int', found 'Bool'`.
            //
            // Wrapping with `TypeScheme::poly(bound_vars, fn_ty)`
            // forces fresh instantiation of every bound TypeVar
            // on each `instantiate()` call — the canonical
            // Hindley-Milner discipline already used by the AST-
            // driven impl-block path (`register_impl_method` and
            // `compile_pending_default_methods` produce poly
            // schemes; the metadata-driven path was just missing
            // the wrapper).
            //
            // Stdlib-agnostic — the bound-vars list is harvested
            // from the parsed signature itself via
            // `collect_type_vars` (already used by the
            // dependent-types subsystem), no hardcoded type-name
            // list.
            let scheme = {
                use crate::dependent_helpers::collect_type_vars;
                let vars = collect_type_vars(&fn_ty);
                if vars.is_empty() {
                    TypeScheme::mono(fn_ty.clone())
                } else {
                    let var_list: List<crate::ty::TypeVar> =
                        vars.iter().copied().collect();
                    TypeScheme::poly(var_list, fn_ty.clone())
                }
            };
            bucket.insert(method_name.clone(), scheme.clone());
            if is_static {
                let static_key: Text =
                    format!("$static${}", method_name.as_str()).into();
                bucket.entry(static_key).or_insert(scheme);
            }
        }
        referenced
    }

    /// Convert a StageError from the stage checker to a TypeError.
    /// Used for integrating stage validation into type checking.
    /// Stage coherence: runtime code cannot depend on meta-only values, meta code cannot observe runtime state — Stage Coherence Rule
    fn stage_error_to_type_error(err: crate::stage_checker::StageError) -> TypeError {
        use crate::stage_checker::StageError;
        match err {
            StageError::StageMismatch {
                current_stage,
                target_stage,
                expected_stage,
                hint,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1001: stage mismatch in quote expression: current stage is {}, target stage is {} (expected {}). {}",
                current_stage, target_stage, expected_stage, hint
            ))),
            StageError::CrossStageCall {
                caller_stage,
                callee_stage,
                callee_name,
                hint,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1002: cross-stage call: stage {} function cannot directly call stage {} function '{}'. {}",
                caller_stage, callee_stage, callee_name, hint
            ))),
            StageError::StageOverflow {
                used_stage,
                max_stage,
                function_name,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1003: stage overflow: meta({}) exceeds maximum allowed stage {} for function '{}'",
                used_stage, max_stage, function_name
            ))),
            StageError::CyclicStage { cycle, start, .. } => TypeError::Other(Text::from(format!(
                "E1004: cyclic stage dependency starting from '{}': {}",
                start,
                cycle
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            ))),
            StageError::InvalidStageEscape {
                escape_stage,
                current_stage,
                valid_range,
                ..
            } => TypeError::Other(Text::from(format!(
                "E1005: invalid stage escape: $(stage {}) is invalid in stage {} context (valid: {})",
                escape_stage, current_stage, valid_range
            ))),
        }
    }

    /// Create a TypeChecker with minimal context (no stdlib types at all).
    ///

    /// STDLIB-AGNOSTIC: This is the most minimal constructor, used when:
    /// - Compiling stdlib itself - types are registered as .vr files are parsed
    /// - Testing type system in isolation
    ///

    /// Contains NO types - not even primitives. Caller must register
    /// all types and methods via the type checker APIs.
    ///

    /// For compiling user code, use `new_with_core()` instead.
    pub fn with_minimal_context() -> Self {
        Self {
            ctx: TypeContext::new_minimal(),
            unifier: Unifier::new(),
            refinement: RefinementChecker::new(Default::default()),
            subtyping: Subtyping::new(),
            const_eval: ConstEvaluator::new(),
            protocol_checker: Shared::new(parking_lot::RwLock::new(ProtocolChecker::new())),
            integer_hierarchy: IntegerHierarchy::new(),
            // STDLIB-AGNOSTIC: Minimal context uses minimal kind inferer
            kind_inferer: crate::kind_inference::KindInferer::new_minimal(),
            metrics: TypeCheckMetrics::new(),
            generator_context: Maybe::None,
            diagnostics: List::new(),
            deferred_soundness_errors: Vec::new(),
            glob_import_provenance: std::collections::HashMap::new(),
            current_cog_name: verum_common::Text::from(""),
            dependent_enabled: true,
            higher_kinded_enabled: true,
            universe_poly_enabled: false,
            coinductive_enabled: true,
            quotient_enabled: true,
            instance_search_enabled: true,
            coherence_check_depth: 16,
            higher_kinded_protocols_enabled: false,
            generic_associated_types_enabled: false,
            classification_map: std::collections::HashMap::new(),
            module_resolver: NameResolver::new(),
            module_registry: Shared::new(parking_lot::RwLock::new(ModuleRegistry::new())),
            current_module_path: verum_common::Text::from("cog"),
            inline_modules: Map::new(),
            module_aliases: Map::new(),
            preregistered_modules: std::collections::HashSet::new(),
            blanket_impls_registered_modules: std::collections::HashSet::new(),
            primitive_impls_registered_modules: std::collections::HashSet::new(),
            module_context: Maybe::None,
            current_function_return_type: Maybe::None,
            current_function_name: Maybe::None,
            current_function_return_span: Maybe::None,
            current_function_params: Set::new(),
            type_registry: crate::type_registry::TypeRegistry::new(),
            context_resolver: crate::context_resolution::ContextResolver::new(),
            current_self_type: Maybe::None,
            capability_checker: crate::capability::CapabilityChecker::new(),
            context_declarations: Map::new(),
            method_capability_mapper: crate::capability::MethodCapabilityMapper::new(),
            inherent_methods: Shared::new(parking_lot::RwLock::new(Map::new())),
            method_impl_patterns: Shared::new(parking_lot::RwLock::new(Map::new())),
            variance_checker: crate::variance::VarianceChecker::new(),
            variant_type_names: Map::new(),
            variant_collision_log: List::new(),
            variant_record_fields: Map::new(),
            variant_constructor_parents: Map::new(),
            type_var_bounds: Map::new(),
            hkt_type_var_by_name: Map::new(),
            type_var_type_bounds: Map::new(),
            deferred_constraints: List::new(),
            property_inferrer: crate::computational_properties::PropertyInferrer::new(),
            in_async_context: false,
            try_recover_depth: 0,
            current_function_throws: Maybe::None,
            context_checker: ContextChecker::new(),
            stage_checker: StageChecker::with_defaults(),
            current_function_stage: 0,
            current_function_is_transparent: false,
            current_function_contexts: Maybe::None,
            current_function_call_sites: Map::new(),
            current_function_being_checked: Maybe::None,
            init_tracker: crate::context::InitTracker::new(),
            affine_tracker: crate::affine::AffineTracker::with_core(),
            borrow_tracker: crate::aliasing::BorrowTracker::new(),
            in_call_arg_context: false,
            self_by_value_methods: std::collections::HashSet::new(),
            in_unsafe_context: false,
            in_impl_block: false,
            types_being_registered: std::collections::HashSet::new(),
            imports_in_progress: std::collections::HashSet::new(),
            glob_imports_in_progress: std::collections::HashSet::new(),
            glob_imports_stack: Vec::new(),
            reexport_resolution_in_progress: std::collections::HashSet::new(),
            imported_names: Map::new(),
            constants_being_evaluated: std::collections::HashSet::new(),
            constant_dependencies: Map::new(),
            current_constant_path: Maybe::None,
            imported_constant_paths: Map::new(),
            termination_checker: crate::termination::TerminationChecker::new(),
            function_required_params: Map::new(),
            function_contracts: Map::new(),
            function_param_names: Map::new(),
            function_param_classifications: Map::new(),
            core_metadata: Maybe::None,
            lazy_resolver: None,
            session_registry: None,
            operator_protocols: OperatorProtocols::standard(),
            type_generics_count: Map::new(),
            refinement_evidence: crate::refinement_evidence::RefinementEvidence::new(),
            prototype_mode: false,
            implicit_context: crate::implicit::ImplicitContext::new(),
            mutable_bindings: std::collections::HashSet::new(),
            pattern_declarations: Map::new(),
            hit_path_constructors: Map::new(),
            deferred_verification_goals: List::new(),
            stdlib_single_file_mode: false,
            user_code_phase: false,
            explicit_imports: std::collections::HashSet::new(),
            in_explicit_import_registration: false,
            skolem_tracker: crate::existential::SkolemTracker::new(),
            cfg_evaluator: verum_ast::cfg::CfgEvaluator::new(),
            inference_depth: Cell::new(0),
        }
    }

    /// Load stdlib types from metadata into the type context.
    ///

    /// This registers all types, protocols, and implementations from
    /// the pre-loaded stdlib.vbca metadata.
    ///

    /// Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files
    pub fn load_stdlib_from_metadata(&mut self, metadata: &crate::core_metadata::CoreMetadata) {
        use crate::core_metadata::TypeDescriptorKind;
        use crate::protocol::{Protocol, ProtocolImpl, ProtocolMethod};
        use crate::ty::Type;
        use verum_common::span::Span;

        // Register types from metadata in source declaration order.
        //

        // The variant_type_names registry uses first-registered-wins semantics,
        // so the order types are registered determines which type owns each
        // variant signature when names overlap (e.g., Result.Ok vs CheckedResult.Ok).
        //

        // Iteration walks `type_declaration_order`, which records insertion order
        // through the metadata pipeline:
        //  archive layer order (Core → Text → Collections → …)
        //  → per-module .vr file declaration order
        //

        // This means stdlib's `Maybe`/`Result`/`Ordering` register before any
        // sibling cog's variant aliases naturally — no hardcoded priority list,
        // no compiler-side stdlib type knowledge.
        //

        // Trailing tail: any type present in `metadata.types` but missing from
        // the order list (defensive — should never happen in practice) is
        // appended in alphabetical order so we still register every type.
        let ordered_types = Self::stdlib_iteration_order(metadata);
        for (name, type_desc) in ordered_types {
            // Convert core_metadata::TypeDescriptor to Type
            let ty = self.type_descriptor_to_type(type_desc);
            self.ctx.define_type(name.clone(), ty.clone());
            // Also register in the type environment so type names can be resolved
            // (e.g., `List<Int>` needs "List" in env, not just in type_defs)
            self.ctx
                .env
                .insert(name.clone(), TypeScheme::mono(ty.clone()));

            // CRITICAL FIX: Register variant type signatures for method lookup
            // This enables methods defined on Maybe<T> to be found when the type
            // has been normalized to its variant form (None | Some(T))
            if let TypeDescriptorKind::Variant { cases } = &type_desc.kind {
                // Build the variant type structure to compute its signature
                let mut variant_map: indexmap::IndexMap<verum_common::Text, Type> =
                    indexmap::IndexMap::new();
                for case in cases.iter() {
                    let payload_ty = match &case.payload {
                        verum_common::Maybe::None => Type::Unit,
                        verum_common::Maybe::Some(crate::core_metadata::VariantPayload::Tuple(
                            types,
                        )) => {
                            if types.len() == 1 {
                                // Single field tuple - unwrap for signature
                                Type::Named {
                                    path: Self::text_to_path(&types[0]),
                                    args: verum_common::List::new(),
                                }
                            } else {
                                // Multiple fields - create tuple
                                Type::Tuple(
                                    types
                                        .iter()
                                        .map(|t| Type::Named {
                                            path: Self::text_to_path(t),
                                            args: verum_common::List::new(),
                                        })
                                        .collect(),
                                )
                            }
                        }
                        verum_common::Maybe::Some(
                            crate::core_metadata::VariantPayload::Record(fields),
                        ) => {
                            let field_map: indexmap::IndexMap<verum_common::Text, Type> = fields
                                .iter()
                                .map(|f| {
                                    (
                                        f.name.clone(),
                                        Type::Named {
                                            path: Self::text_to_path(&f.ty),
                                            args: verum_common::List::new(),
                                        },
                                    )
                                })
                                .collect();
                            Type::Record(field_map)
                        }
                    };
                    variant_map.insert(case.name.clone(), payload_ty);
                }

                let variant_type = Type::Variant(variant_map.clone());
                if let Some(sig) = Self::variant_type_signature(&variant_type) {
                    self.register_variant_type_name_first_wins(sig.clone(), name.clone());
                    if let Some(relaxed_sig) = Self::variant_type_signature_relaxed(&variant_type) {
                        if relaxed_sig != sig {
                            self.register_variant_type_name_first_wins(relaxed_sig, name.clone());
                        }
                    }
                }

                // Register variant constructor parent mappings for scope-aware resolution.
                for (vname, _payload_ty) in &variant_map {
                    let parents = self
                        .variant_constructor_parents
                        .entry(vname.clone())
                        .or_default();
                    if !parents.iter().any(|p| p == name) {
                        parents.push(name.clone());
                    }
                }

                // Register unit variant constructors as values in the env.
                // This enables pattern matching and expression usage of stdlib variant
                // constructors (e.g., `Less`, `Equal`, `Greater` from Ordering).
                for (vname, payload_ty) in &variant_map {
                    if *payload_ty == Type::Unit {
                        // Only register short name if not already taken
                        if self.ctx.env.lookup(vname.as_str()).is_none() {
                            self.ctx
                                .env
                                .insert_mono(vname.clone(), variant_type.clone());
                        }
                    }
                    // Always register qualified name (Type.Variant)
                    let qualified_name: verum_common::Text = format!("{}.{}", name, vname).into();
                    self.ctx
                        .env
                        .insert_mono(qualified_name, variant_type.clone());
                }
            }

            // Register generic parameters count for later instantiation
            if !type_desc.generic_params.is_empty() {
                self.type_generics_count
                    .insert(name.clone(), type_desc.generic_params.len());
            }
        }

        // Register protocols via protocol_checker (public API)
        // Sort for deterministic registration order (HashMap iteration is non-deterministic).
        let mut sorted_protocols: Vec<_> = metadata.protocols.iter().collect();
        sorted_protocols.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (name, protocol_desc) in sorted_protocols {
            // Convert core_metadata methods to protocol ProtocolMethods
            let mut methods = verum_common::Map::new();
            for m in protocol_desc.required_methods.iter() {
                // Build function type from method signature
                let params: verum_common::List<Type> = m
                    .params
                    .iter()
                    .map(|p| {
                        // Parse type name - simplified for now
                        Type::Named {
                            path: Self::text_to_path(&p.ty),
                            args: verum_common::List::new(),
                        }
                    })
                    .collect();
                let return_type = Type::Named {
                    path: Self::text_to_path(&m.return_type.clone()),
                    args: verum_common::List::new(),
                };
                let method_type = Type::function(params, return_type);

                let protocol_method = ProtocolMethod::simple(
                    m.name.clone(),
                    method_type,
                    false, // required methods don't have defaults
                );
                methods.insert(m.name.clone(), protocol_method);
            }

            // Add default methods
            for m in protocol_desc.default_methods.iter() {
                let params: verum_common::List<Type> = m
                    .params
                    .iter()
                    .map(|p| Type::Named {
                        path: Self::text_to_path(&p.ty),
                        args: verum_common::List::new(),
                    })
                    .collect();
                let return_type = Type::Named {
                    path: Self::text_to_path(&m.return_type.clone()),
                    args: verum_common::List::new(),
                };
                let method_type = Type::function(params, return_type);

                let protocol_method = ProtocolMethod::simple(
                    m.name.clone(),
                    method_type,
                    true, // default methods have implementations
                );
                methods.insert(m.name.clone(), protocol_method);
            }

            // Create Protocol struct
            let protocol = Protocol {
                name: name.clone(),
                kind: crate::protocol::ProtocolKind::Constraint,
                type_params: protocol_desc
                    .generic_params
                    .iter()
                    .map(|g| crate::protocol::TypeParam {
                        name: g.name.clone(),
                        bounds: g
                            .bounds
                            .iter()
                            .map(|b| crate::protocol::ProtocolBound {
                                protocol: Self::text_to_path(b),
                                args: verum_common::List::new(),
                                is_negative: false,
                            })
                            .collect(),
                        default: g.default.as_ref().map(|d| Type::Named {
                            path: Self::text_to_path(d),
                            args: verum_common::List::new(),
                        }),
                    })
                    .collect(),
                methods,
                associated_types: verum_common::Map::new(), // Simplified
                associated_consts: verum_common::Map::new(),
                super_protocols: protocol_desc
                    .super_protocols
                    .iter()
                    .map(|sp| crate::protocol::ProtocolBound {
                        protocol: Self::text_to_path(sp),
                        args: verum_common::List::new(),
                        is_negative: false,
                    })
                    .collect(),
                specialization_info: verum_common::Maybe::None,
                defining_crate: verum_common::Maybe::None,
                span: Span::default(),
            };

            let _ = self.protocol_checker.write().register_protocol(protocol);
        }

        // Register protocol implementations via protocol_checker
        for impl_desc in metadata.implementations.iter() {
            // Convert associated types from impl descriptor
            let associated_types: verum_common::Map<verum_common::Text, Type> = impl_desc
                .associated_types
                .iter()
                .map(|(name, type_name)| {
                    let ty = Type::Named {
                        path: Self::text_to_path(type_name),
                        args: verum_common::List::new(),
                    };
                    (name.clone(), ty)
                })
                .collect();

            // Look up the protocol definition to get method types
            // The protocol was registered in the previous loop, so we can look it up
            let methods: verum_common::Map<verum_common::Text, Type> = {
                let protocol_checker_guard = self.protocol_checker.read();
                #[cfg(debug_assertions)]
                if impl_desc.protocol.as_str() == "Ord" {
                    // eprintln!("[DEBUG load_stdlib_impl] Looking up protocol 'Ord' for impl on '{}'", impl_desc.target_type);
                    if let verum_common::Maybe::Some(protocol) =
                        protocol_checker_guard.get_protocol(&impl_desc.protocol)
                    {
                        // eprintln!("[DEBUG load_stdlib_impl] Found protocol 'Ord' with {} methods", protocol.methods.len());
                        for (name, method) in protocol.methods.iter() {
                            // eprintln!("[DEBUG load_stdlib_impl] method '{}': {:?}", name, method.ty);
                        }
                    } else {
                        // eprintln!("[DEBUG load_stdlib_impl] Protocol 'Ord' NOT FOUND!");
                    }
                }
                if let verum_common::Maybe::Some(protocol) =
                    protocol_checker_guard.get_protocol(&impl_desc.protocol)
                {
                    protocol
                        .methods
                        .iter()
                        .map(|(name, method)| (name.clone(), method.ty.clone()))
                        .collect()
                } else {
                    verum_common::Map::new()
                }
            };

            let protocol_impl = ProtocolImpl {
                protocol: Self::text_to_path(&impl_desc.protocol),
                protocol_args: verum_common::List::new(),
                for_type: Type::Named {
                    path: Self::text_to_path(&impl_desc.target_type),
                    args: verum_common::List::new(),
                },
                where_clauses: verum_common::List::new(),
                methods,
                associated_types,
                associated_consts: verum_common::Map::new(),
                specialization: verum_common::Maybe::None,
                impl_crate: verum_common::Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: verum_common::Map::new(),
            };
            // Ignore coherence errors during metadata loading
            let _ = self.protocol_checker.write().register_impl(protocol_impl);
        }

        // Register inductive constructors for pattern matching
        self.register_stdlib_constructors_from_metadata(metadata);
    }

    /// Helper function to create a Path from a type name string.
    fn text_to_path(name: &verum_common::Text) -> verum_ast::ty::Path {
        use verum_ast::ty::{Ident, Path};
        use verum_common::span::Span;
        let ident = Ident::new(name.as_str(), Span::default());
        Path::single(ident)
    }

    /// Walk metadata types in stdlib source declaration order.
    ///

    /// Returns `(name, descriptor)` pairs ordered first by `type_declaration_order`
    /// (which records insertion order: archive layer → .vr declaration order),
    /// then any orphan types not present in that list appended in alphabetical
    /// (BTreeMap) order. The orphan tail is defensive — every type inserted via
    /// `core_loader::extract_module_metadata` or `pipeline::cached → metadata`
    /// is already pushed to `type_declaration_order`.
    ///

    /// First-registered-wins iteration is the architectural alternative to
    /// hardcoded priority lists like `["Result", "Maybe", "Ordering", "Bool"]`.
    /// Compiler stays stdlib-agnostic; correctness comes from source order.
    fn stdlib_iteration_order(
        metadata: &crate::core_metadata::CoreMetadata,
    ) -> Vec<(&Text, &crate::core_metadata::TypeDescriptor)> {
        let mut seen: std::collections::HashSet<&Text> =
            std::collections::HashSet::with_capacity(metadata.types.len());
        let mut out: Vec<(&Text, &crate::core_metadata::TypeDescriptor)> =
            Vec::with_capacity(metadata.types.len());

        for name in metadata.type_declaration_order.iter() {
            if let Some(desc) = metadata.types.get(name) {
                if seen.insert(name) {
                    out.push((name, desc));
                }
            }
        }

        for (name, desc) in metadata.types.iter() {
            if !seen.contains(name) {
                seen.insert(name);
                out.push((name, desc));
            }
        }

        out
    }

    /// Convert a core_metadata::TypeDescriptor to a Type.
    fn type_descriptor_to_type(&self, desc: &crate::core_metadata::TypeDescriptor) -> Type {
        use crate::core_metadata::TypeDescriptorKind;
        use crate::ty::Type;

        if desc.generic_params.is_empty() {
            // Concrete type
            match &desc.kind {
                TypeDescriptorKind::Record { .. } => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Variant { .. } => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Protocol { .. } => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Alias { target } => Type::Named {
                    path: Self::text_to_path(target),
                    args: verum_common::List::new(),
                },
                TypeDescriptorKind::Opaque => Type::Named {
                    path: Self::text_to_path(&desc.name),
                    args: verum_common::List::new(),
                },
            }
        } else {
            // Generic type - create a type constructor
            Type::Generic {
                name: desc.name.clone(),
                args: desc
                    .generic_params
                    .iter()
                    .map(|_| {
                        // Use fresh type variables for generic parameters
                        Type::Var(crate::ty::TypeVar::fresh())
                    })
                    .collect(),
            }
        }
    }

    // Environment management methods (register_stdlib*, pre_register_module*, defer_constraint*, …)
    // → see infer/env.rs in this module
    // Type resolution and normalization methods (ast_to_type*, normalize_type*, check_cast, …)
    // → see infer/types.rs in this module
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

// Module import/export and item-checking methods (check_item, check_module, check_import, …)
// → see infer/modules.rs in this module


// GAT inference infrastructure (GATInferenceError, GATConstraint, OptimizedGATInference,
// GAT-specific TypeChecker methods) → see infer/gat.rs in this module
pub(crate) mod gat;
pub(crate) use gat::{GATConstraint, GATInferenceError, GATInferenceStats, OptimizedGATInference,
              ConflictingRequirement};

// Standalone helpers (ConditionExt, levenshtein_distance, span_to_line_col,
// expr_kind_description, HKT methods, QTT helpers, type-size, meta_value_to_literal, …)
// → see infer/helpers.rs in this module
pub(crate) mod helpers;
// Re-export helpers so sibling submodules can access them via `super::`
pub(crate) use helpers::{
    mount_tree_exports_name, resolve_builtin_meta_type, levenshtein_distance,
    expr_kind_description, type_kind_description, extract_quantity_from_attrs,
    walk_stmt_for_qtt_usage, resolve_primitive_method, meta_value_to_literal,
    make_maybe_type, span_to_line_col,
    collect_named_types_from_item, parse_descriptor_type_string,
    register_variant_signature_for_lazy,
};

#[cfg(test)]
pub(crate) mod infer_tests;

