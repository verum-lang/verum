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

    /// Register inductive constructors from metadata for pattern matching.
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


/* ============================================================================
 * GAT INFERENCE ENHANCEMENTS
 * ============================================================================
 */

/// Enhanced error reporting for GAT inference failures
///

/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4
///

/// Provides rich, actionable diagnostics when GAT type inference fails,
/// including:
/// - Detailed constraint analysis
/// - Conflicting requirement identification
/// - Actionable suggestions (add annotations, simplify bounds, etc.)
/// - Example code snippets
#[derive(Debug, Clone)]
pub struct GATInferenceError {
    /// Name of the GAT that failed inference
    pub gat_name: Text,

    /// Name of the protocol containing the GAT
    pub trait_name: Text,

    /// Type bindings that were attempted
    pub attempted_bindings: Map<Text, Type>,

    /// Constraints that failed to be satisfied
    pub failed_constraints: List<GATConstraint>,

    /// Conflicting requirements from different sources
    pub conflicting_requirements: List<ConflictingRequirement>,

    /// Suggested fix for the user
    pub suggestion: GATInferenceSuggestion,

    /// Source location
    pub span: Span,
}

/// A single constraint in GAT inference
#[derive(Debug, Clone)]
pub struct GATConstraint {
    /// The type being constrained
    pub ty: Type,

    /// The protocol bound that must be satisfied
    pub bound: crate::protocol::ProtocolBound,

    /// Source of this constraint (where clause, parameter bound, etc.)
    pub source: Text,

    /// Whether this constraint was satisfied
    pub satisfied: bool,

    /// Reason for failure (if not satisfied)
    pub failure_reason: Maybe<Text>,
}

/// A requirement that conflicts with another
#[derive(Debug, Clone)]
pub struct ConflictingRequirement {
    /// Source of this requirement (e.g., "impl bound", "where clause")
    pub source: Text,

    /// The required type
    pub requirement: Type,

    /// Source location
    pub location: Span,

    /// Explanation of why this conflicts
    pub conflict_explanation: Text,
}

/// Suggested fix for GAT inference failure
#[derive(Debug, Clone)]
pub enum GATInferenceSuggestion {
    /// Add a type annotation
    AddTypeAnnotation {
        /// Where to add the annotation
        location: Text,
        /// The annotation to add
        annotation: Text,
        /// Example usage
        example: Text,
    },

    /// Simplify bounds on the GAT
    SimplifyBounds {
        /// Current complex bounds
        current_bounds: Text,
        /// Suggested simpler bounds
        suggested_bounds: Text,
    },

    /// Split implementation into multiple impls
    SplitImplementation {
        /// Reason for splitting
        reason: Text,
        /// Suggested split
        suggestion: Text,
    },

    /// Use a concrete type instead of GAT
    UseConcreteType {
        /// GAT name
        gat_name: Text,
        /// Suggested concrete type
        suggested_type: Type,
    },

    /// Add where clause to disambiguate
    AddWhereClause {
        /// The where clause to add
        clause: Text,
        /// Explanation
        explanation: Text,
    },
}

impl TypeChecker {
    /// Create detailed error for GAT inference failure
    ///

    /// Analyzes failed constraints to provide actionable diagnostics.
    fn create_gat_error(
        &self,
        gat_name: &Text,
        trait_name: &Text,
        attempted_bindings: &Map<Text, Type>,
        constraints: &List<GATConstraint>,
        span: Span,
    ) -> GATInferenceError {
        // Analyze constraints to find conflicts
        let mut conflicting = List::new();

        for (i, c1) in constraints.iter().enumerate() {
            if !c1.satisfied {
                for c2 in constraints.iter().skip(i + 1) {
                    if !c2.satisfied && self.constraints_conflict(c1, c2) {
                        conflicting.push(ConflictingRequirement {
                            source: c1.source.clone(),
                            requirement: c1.ty.clone(),
                            location: span,
                            conflict_explanation: verum_common::Text::from(format!(
                                "Conflicts with {} requiring {}",
                                c2.source, c2.ty
                            )),
                        });
                    }
                }
            }
        }

        // Generate actionable suggestion
        let suggestion = self.suggest_gat_fix(
            gat_name,
            trait_name,
            attempted_bindings,
            constraints,
            &conflicting,
        );

        GATInferenceError {
            gat_name: gat_name.clone(),
            trait_name: trait_name.clone(),
            attempted_bindings: attempted_bindings.clone(),
            failed_constraints: constraints.clone(),
            conflicting_requirements: conflicting,
            suggestion,
            span,
        }
    }

    /// Check if two constraints conflict
    fn constraints_conflict(&self, c1: &GATConstraint, c2: &GATConstraint) -> bool {
        // Simple check: same type variable with incompatible bounds
        if let (Type::Var(v1), Type::Var(v2)) = (&c1.ty, &c2.ty)
            && v1 == v2
        {
            // Check if bounds are incompatible
            return !self.bounds_compatible(&c1.bound, &c2.bound);
        }
        false
    }

    /// Check if two protocol bounds are compatible
    ///

    /// Bounds are compatible if:
    /// 1. They require the same protocol (fast path - exact equality)
    /// 2. One bound subsumes the other (e.g., Copy subsumes Clone because Copy: Clone)
    /// 3. Transitively related through the protocol hierarchy
    ///

    /// Protocol coherence: ensuring unique implementations across the program, orphan rules, overlap detection — Coherence Rules
    ///

    /// For GAT constraint checking, two bounds are "compatible" if they can both be
    /// satisfied by the same type. This means checking if one protocol is a
    /// subprotocol of the other (either direction works for compatibility).
    fn bounds_compatible(
        &self,
        b1: &crate::protocol::ProtocolBound,
        b2: &crate::protocol::ProtocolBound,
    ) -> bool {
        // Fast path: exact protocol equality
        if b1.protocol == b2.protocol {
            // Same protocol - compatible if arguments are compatible
            return self.protocol_args_compatible(&b1.args, &b2.args);
        }

        // Extract protocol names for subsumption checking
        let b1_name = self.extract_protocol_name_from_path(&b1.protocol);
        let b2_name = self.extract_protocol_name_from_path(&b2.protocol);

        // Handle negative bounds: a positive and negative bound for the same protocol conflict
        if b1.is_negative != b2.is_negative {
            // If same protocol but opposite polarity, they conflict (incompatible)
            if b1_name == b2_name {
                return false;
            }
            // Different protocols with opposite polarity - need hierarchy check
            // e.g., T: Clone and T: !Copy might be compatible if Clone doesn't require Copy
            // For now, treat as compatible if they're not the same protocol
            return true;
        }

        // Both positive or both negative - check subsumption
        // For positive bounds: compatible if one is subprotocol of the other
        // For negative bounds: compatible if they're for different protocols
        if b1.is_negative {
            // Both negative: !A and !B are compatible (type must not implement either)
            return true;
        }

        // Both positive: check if one protocol inherits from the other
        // This makes them compatible because any type implementing the subprotocol
        // automatically implements the superprotocol
        self.check_protocol_subsumption(&b1_name, &b2_name)
    }

    /// Extract protocol name from a Path for bound subsumption checking
    fn extract_protocol_name_from_path(&self, path: &verum_ast::ty::Path) -> Text {
        // For simple paths (e.g., "Clone"), use the first segment
        // For qualified paths (e.g., "std.iter.Iterator"), use the last segment
        path.segments
            .last()
            .and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => {
                    Some(verum_common::Text::from(ident.name.as_str()))
                }
                _ => None,
            })
            .unwrap_or_else(|| verum_common::Text::from(""))
    }

    /// Check if protocol arguments are compatible
    ///

    /// Type arguments must be compatible for the bounds to be compatible.
    fn protocol_args_compatible(&self, args1: &List<Type>, args2: &List<Type>) -> bool {
        // If different number of arguments, not compatible
        if args1.len() != args2.len() {
            return false;
        }

        // All corresponding arguments must be compatible
        for (a1, a2) in args1.iter().zip(args2.iter()) {
            if !self.types_compatible_for_bounds(a1, a2) {
                return false;
            }
        }

        true
    }

    /// Check if two types are compatible for bound checking
    ///

    /// Types are compatible if they unify or one is a subtype of the other.
    fn types_compatible_for_bounds(&self, t1: &Type, t2: &Type) -> bool {
        // Exact equality
        if t1 == t2 {
            return true;
        }

        // Type variables are compatible with anything (they can be unified)
        if matches!(t1, Type::Var(_)) || matches!(t2, Type::Var(_)) {
            return true;
        }

        // Check structural compatibility for common cases
        match (t1, t2) {
            // References with same mutability and compatible targets
            (
                Type::Reference {
                    inner: ty1,
                    mutable: m1,
                },
                Type::Reference {
                    inner: ty2,
                    mutable: m2,
                },
            ) => m1 == m2 && self.types_compatible_for_bounds(ty1, ty2),
            // Other cases - conservative false for now
            _ => false,
        }
    }

    /// Check if one protocol subsumes another (transitive inheritance check)
    ///

    /// Returns true if:
    /// - p1 == p2 (reflexive)
    /// - p1 inherits from p2 (p1 is subprotocol of p2)
    /// - p2 inherits from p1 (p2 is subprotocol of p1)
    ///

    /// Both directions are checked because for bound compatibility, we care
    /// whether there exists a type that can satisfy both bounds, which is
    /// possible if either protocol inherits from the other.
    fn check_protocol_subsumption(&self, p1: &Text, p2: &Text) -> bool {
        // Reflexive case
        if p1 == p2 {
            return true;
        }

        // Check if p1 inherits from p2 (p1 <: p2)
        // This means p1 is more specific, so any type implementing p1 also implements p2
        if self.protocol_checker.read().inherits_from(p1, p2) {
            return true;
        }

        // Check if p2 inherits from p1 (p2 <: p1)
        // This means p2 is more specific, so any type implementing p2 also implements p1
        if self.protocol_checker.read().inherits_from(p2, p1) {
            return true;
        }

        // No inheritance relationship - bounds may still be compatible
        // if both are superprotocols of some common subprotocol
        // For now, we're conservative and say they're compatible
        // (could lead to false positives in conflict detection, but not false negatives)
        //

        // A more precise check would require finding if there exists a common subprotocol,
        // but that's expensive and rarely needed in practice.
        //

        // Examples:
        // - Clone and Debug are compatible (many types implement both)
        // - Send and Sync are compatible (many types implement both)
        //

        // We return true here to avoid spurious conflict errors.
        true
    }

    /// Suggest fix for GAT inference failure
    fn suggest_gat_fix(
        &self,
        gat_name: &Text,
        trait_name: &Text,
        attempted_bindings: &Map<Text, Type>,
        constraints: &List<GATConstraint>,
        conflicts: &List<ConflictingRequirement>,
    ) -> GATInferenceSuggestion {
        // Strategy 1: If multiple conflicts, suggest type annotation
        if conflicts.len() > 1 {
            let annotation = self.infer_best_annotation(gat_name, attempted_bindings);
            let example = verum_common::Text::from(format!(
                "let value: {}.{}<{}> = ...",
                trait_name, gat_name, annotation
            ));

            return GATInferenceSuggestion::AddTypeAnnotation {
                location: verum_common::Text::from(format!(
                    "for GAT '{}.{}'",
                    trait_name, gat_name
                )),
                annotation,
                example,
            };
        }

        // Strategy 2: If single unsatisfied constraint, suggest where clause
        let unsatisfied: Vec<_> = constraints.iter().filter(|c| !c.satisfied).collect();
        if unsatisfied.len() == 1 {
            let constraint = unsatisfied[0];
            let clause = verum_common::Text::from(format!(
                "where {}: {}",
                self.ty_to_string(&constraint.ty),
                self.bound_to_string(&constraint.bound)
            ));

            return GATInferenceSuggestion::AddWhereClause {
                clause: clause.clone(),
                explanation: verum_common::Text::from(format!(
                    "Add this constraint to satisfy the {} requirement",
                    constraint.source
                )),
            };
        }

        // Strategy 3: Check if we can suggest a concrete type
        if let Maybe::Some(concrete) = self.find_concrete_candidate(attempted_bindings) {
            return GATInferenceSuggestion::UseConcreteType {
                gat_name: gat_name.clone(),
                suggested_type: concrete,
            };
        }

        // Strategy 4: Suggest simplifying bounds
        if constraints.len() > 3 {
            let simplified = self.try_simplify_constraints(constraints);
            return GATInferenceSuggestion::SimplifyBounds {
                current_bounds: verum_common::Text::from(format!(
                    "{} constraints",
                    constraints.len()
                )),
                suggested_bounds: simplified,
            };
        }

        // Fallback: Split implementation
        GATInferenceSuggestion::SplitImplementation {
            reason: verum_common::Text::from("GAT constraints are too complex for single impl"),
            suggestion: verum_common::Text::from(
                "Consider splitting into multiple impl blocks with different bounds",
            ),
        }
    }

    /// Infer best type annotation from attempted bindings
    fn infer_best_annotation(&self, _gat_name: &Text, bindings: &Map<Text, Type>) -> Text {
        let types: Vec<_> = bindings
            .iter()
            .map(|(name, ty)| format!("{}: {}", name, ty))
            .collect();

        verum_common::Text::from(types.join(", "))
    }

    /// Find a concrete type candidate from bindings
    fn find_concrete_candidate(&self, bindings: &Map<Text, Type>) -> Maybe<Type> {
        for (_name, ty) in bindings {
            if !matches!(ty, Type::Var(_)) {
                return Maybe::Some(ty.clone());
            }
        }
        Maybe::None
    }

    /// Try to simplify constraints
    fn try_simplify_constraints(&self, constraints: &List<GATConstraint>) -> Text {
        // Group by bound
        let mut bound_counts = Map::new();
        for constraint in constraints {
            let bound_str = self.bound_to_string(&constraint.bound);
            *bound_counts.entry(bound_str).or_insert(0) += 1;
        }

        // Show most common bounds
        let mut counts: Vec<_> = bound_counts.iter().collect();
        counts.sort_by_key(|(_, count)| std::cmp::Reverse(**count));

        let top_bounds: Vec<_> = counts
            .iter()
            .take(3)
            .map(|(bound, _)| bound.as_str())
            .collect();

        verum_common::Text::from(top_bounds.join(" + "))
    }

    /// Convert type to string for error messages
    fn ty_to_string(&self, ty: &Type) -> Text {
        match ty {
            Type::Var(v) => verum_common::Text::from(format!("T{}", v.id())),
            Type::Named { path, args } => {
                let name = self.path_to_string(path);
                if args.is_empty() {
                    name
                } else {
                    let arg_strs: Vec<_> = args.iter().map(|a| self.ty_to_string(a)).collect();
                    verum_common::Text::from(format!("{}<{}>", name, arg_strs.join(", ")))
                }
            }
            _ => verum_common::Text::from(format!("{}", ty)),
        }
    }

    /// Convert protocol bound to string
    fn bound_to_string(&self, bound: &crate::protocol::ProtocolBound) -> Text {
        self.path_to_string(&bound.protocol)
    }

    /// Format GAT error as user-friendly diagnostic
    pub fn format_gat_error(&self, error: &GATInferenceError) -> Text {
        let mut msg = verum_common::Text::from(format!(
            "Cannot infer type for GAT '{}.{}'",
            error.trait_name, error.gat_name
        ));

        if !error.attempted_bindings.is_empty() {
            msg.push_str("\n\nAttempted bindings:");
            for (param, ty) in &error.attempted_bindings {
                msg.push_str(&format!("\n  {} = {}", param, self.ty_to_string(ty)));
            }
        }

        if !error.failed_constraints.is_empty() {
            msg.push_str("\n\nFailed constraints:");
            for constraint in &error.failed_constraints {
                if !constraint.satisfied {
                    msg.push_str(&format!(
                        "\n  {} must satisfy {} (from {})",
                        self.ty_to_string(&constraint.ty),
                        self.bound_to_string(&constraint.bound),
                        constraint.source
                    ));
                    if let Maybe::Some(reason) = &constraint.failure_reason {
                        msg.push_str(&format!("\n    Reason: {}", reason));
                    }
                }
            }
        }

        if !error.conflicting_requirements.is_empty() {
            msg.push_str("\n\nConflicting requirements:");
            for conflict in &error.conflicting_requirements {
                msg.push_str(&format!(
                    "\n  From {}: {}",
                    conflict.source,
                    self.ty_to_string(&conflict.requirement)
                ));
                msg.push_str(&format!("\n    {}", conflict.conflict_explanation));
            }
        }

        msg.push_str("\n\nSuggestion:");
        match &error.suggestion {
            GATInferenceSuggestion::AddTypeAnnotation {
                location,
                annotation,
                example,
            } => {
                msg.push_str(&format!(
                    "\n  Add type annotation {} with:\n    {}",
                    location, annotation
                ));
                msg.push_str(&format!("\n  Example:\n    {}", example));
            }
            GATInferenceSuggestion::SimplifyBounds {
                current_bounds,
                suggested_bounds,
            } => {
                msg.push_str(&format!(
                    "\n  Simplify bounds from:\n    {}\n  to:\n    {}",
                    current_bounds, suggested_bounds
                ));
            }
            GATInferenceSuggestion::SplitImplementation { reason, suggestion } => {
                msg.push_str(&format!("\n  {}\n  {}", reason, suggestion));
            }
            GATInferenceSuggestion::UseConcreteType {
                gat_name,
                suggested_type,
            } => {
                msg.push_str(&format!(
                    "\n  Use concrete type for {}: {}",
                    gat_name,
                    self.ty_to_string(suggested_type)
                ));
            }
            GATInferenceSuggestion::AddWhereClause {
                clause,
                explanation,
            } => {
                msg.push_str(&format!("\n  {}\n    {}", clause, explanation));
            }
        }

        msg
    }
}

/* ============================================================================
 * GAT INFERENCE PERFORMANCE OPTIMIZATIONS
 * ============================================================================
 */

/// Performance-optimized GAT inference engine
///

/// Generic Associated Types (GATs): associated types with their own type parameters, enabling lending iterators and monadic abstractions — .4
///

/// Implements advanced optimizations for GAT type inference:
/// - Constraint caching (memoization)
/// - Incremental solving (dependency tracking)
/// - Early pruning (quick feasibility checks)
/// - Constraint simplification
///

/// Performance characteristics:
/// - Cache hit: O(1) ~1ms
/// - Incremental: O(changed) instead of O(total)
/// - Early prune: 50-70% reduction in search space
/// - Overall: O(n²) instead of O(n³) for deep hierarchies
pub struct OptimizedGATInference {
    /// Cache of solved GAT constraints
    /// Key: (GAT path, type parameter bindings) -> Result<Type>
    constraint_cache: Map<ConstraintKey, CachedResult>,

    /// Dependency graph for incremental solving
    dependency_graph: DependencyGraph,

    /// Performance statistics
    stats: GATInferenceStats,

    /// Maximum cache size before eviction (LRU)
    max_cache_size: usize,

    /// Access timestamps for LRU eviction
    cache_timestamps: Map<ConstraintKey, u64>,

    /// Current timestamp counter
    current_timestamp: u64,

    /// Protocol checker for bound verification
    protocol_checker: ProtocolChecker,
}

/// Key for constraint cache lookup
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct ConstraintKey {
    /// GAT identifier (path + name)
    gat_id: Text,

    /// Type parameter bindings (sorted for consistency)
    param_bindings: Vec<(Text, TypeFingerprint)>,
}

/// Fingerprint of a type for fast comparison
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum TypeFingerprint {
    Var(u32),
    Named { path: Text, arity: usize },
    Function { arity: usize },
    Other,
}

impl TypeFingerprint {
    fn from_type(ty: &Type) -> Self {
        match ty {
            Type::Var(v) => TypeFingerprint::Var(v.id() as u32),
            Type::Named { path, args } => TypeFingerprint::Named {
                path: path
                    .segments
                    .iter()
                    .map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => id.name.as_str().to_owned(),
                        _ => "_".to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join(".")
                    .into(),
                arity: args.len(),
            },
            Type::Function { params, .. } => TypeFingerprint::Function {
                arity: params.len(),
            },
            _ => TypeFingerprint::Other,
        }
    }
}

/// Cached result of GAT inference
#[derive(Debug, Clone)]
struct CachedResult {
    /// Inferred type (None if inference failed)
    result: Maybe<Type>,

    /// Constraints that were checked
    constraints: List<GATConstraint>,

    /// Timestamp of last access (for LRU)
    last_accessed: u64,
}

/// Dependency graph tracking GAT relationships
#[derive(Debug, Clone, Default)]
struct DependencyGraph {
    /// Nodes in the graph (GAT definitions)
    nodes: Map<Text, GATNode>,

    /// Edges (dependencies between GATs)
    edges: Map<Text, Set<Text>>,

    /// Reverse edges (dependents)
    reverse_edges: Map<Text, Set<Text>>,
}

/// Node in dependency graph
#[derive(Debug, Clone)]
struct GATNode {
    /// GAT identifier
    gat_id: Text,

    /// Depth in hierarchy (0 = leaf)
    depth: usize,

    /// Whether this GAT has been solved in current iteration
    is_solved: bool,

    /// Cached solution (if solved)
    solution: Maybe<Type>,
}

/// Performance statistics for profiling
#[derive(Debug, Clone, Default)]
pub struct GATInferenceStats {
    /// Number of cache hits
    pub cache_hits: usize,

    /// Number of cache misses
    pub cache_misses: usize,

    /// Number of constraints simplified
    pub constraints_simplified: usize,

    /// Number of early prunes
    pub early_prunes: usize,

    /// Total inference time (milliseconds)
    pub total_time_ms: f64,

    /// Number of incremental updates
    pub incremental_updates: usize,

    /// Average cache lookup time (microseconds)
    pub avg_cache_lookup_us: f64,
}

impl OptimizedGATInference {
    /// Create new optimized GAT inference engine
    pub fn new() -> Self {
        Self {
            constraint_cache: Map::new(),
            dependency_graph: DependencyGraph::default(),
            stats: GATInferenceStats::default(),
            max_cache_size: 10000, // Configurable
            cache_timestamps: Map::new(),
            current_timestamp: 0,
            protocol_checker: ProtocolChecker::new(),
        }
    }

    /// Create with custom protocol checker
    pub fn with_protocol_checker(protocol_checker: ProtocolChecker) -> Self {
        Self {
            constraint_cache: Map::new(),
            dependency_graph: DependencyGraph::default(),
            stats: GATInferenceStats::default(),
            max_cache_size: 10000,
            cache_timestamps: Map::new(),
            current_timestamp: 0,
            protocol_checker,
        }
    }

    /// Solve GAT constraints with optimizations
    pub fn solve_with_optimizations(
        &mut self,
        gat_name: &Text,
        trait_name: &Text,
        param_bindings: &Map<Text, Type>,
        constraints: &List<GATConstraint>,
    ) -> Result<Type> {
        let start = Instant::now();

        // 1. Check cache
        let key = self.make_constraint_key(gat_name, param_bindings);
        if let Maybe::Some(cached) = self.get_from_cache(&key) {
            self.stats.cache_hits += 1;
            self.stats.avg_cache_lookup_us = (self.stats.avg_cache_lookup_us
                * (self.stats.cache_hits - 1) as f64
                + start.elapsed().as_micros() as f64)
                / self.stats.cache_hits as f64;

            return match cached.result {
                Maybe::Some(ty) => Ok(ty),
                Maybe::None => Err(TypeError::AmbiguousType {
                    span: Span::default(),
                }),
            };
        }
        self.stats.cache_misses += 1;

        // 2. Simplify constraints before solving
        let simplified = self.simplify_constraints(constraints);
        self.stats.constraints_simplified += constraints.len() - simplified.len();

        // 3. Early feasibility check
        if !self.quick_feasibility_check(&simplified) {
            self.stats.early_prunes += 1;

            // Cache negative result
            self.insert_into_cache(
                key,
                CachedResult {
                    result: Maybe::None,
                    constraints: simplified,
                    last_accessed: self.current_timestamp,
                },
            );

            return Err(TypeError::AmbiguousType {
                span: Span::default(),
            });
        }

        // 4. Build dependency graph
        let gat_id = verum_common::Text::from(format!("{}.{}", trait_name, gat_name));
        self.build_dependency_graph_for(&gat_id, param_bindings);

        // 5. Solve in dependency order (topological sort)
        let solve_order = self.topological_sort(&gat_id);

        let mut result = Maybe::None;
        for dep_gat_id in solve_order {
            if let Some(node) = self.dependency_graph.nodes.get(&dep_gat_id) {
                if node.is_solved {
                    continue;
                }

                // Solve this GAT
                if dep_gat_id == gat_id {
                    // This is our target GAT - solve with full constraints
                    result = self.solve_gat_constraints(param_bindings, &simplified);
                } else {
                    // Dependency - solve with minimal constraints
                    result = Maybe::None; // Would solve dependency here
                }
            }
        }

        let solution = match result {
            Maybe::Some(ty) => ty,
            Maybe::None => {
                return Err(TypeError::AmbiguousType {
                    span: Span::default(),
                });
            }
        };

        // 6. Cache result
        self.insert_into_cache(
            key,
            CachedResult {
                result: Maybe::Some(solution.clone()),
                constraints: simplified,
                last_accessed: self.current_timestamp,
            },
        );

        // 7. Update stats
        self.stats.total_time_ms += start.elapsed().as_secs_f64() * 1000.0;

        Ok(solution)
    }

    /// Make cache key from GAT and bindings
    fn make_constraint_key(&self, gat_name: &Text, bindings: &Map<Text, Type>) -> ConstraintKey {
        let mut param_bindings: Vec<_> = bindings
            .iter()
            .map(|(name, ty)| (name.clone(), TypeFingerprint::from_type(ty)))
            .collect();

        // Sort for consistency
        param_bindings.sort_by(|a, b| a.0.cmp(&b.0));

        ConstraintKey {
            gat_id: gat_name.clone(),
            param_bindings,
        }
    }

    /// Get from cache with LRU update
    fn get_from_cache(&mut self, key: &ConstraintKey) -> Maybe<CachedResult> {
        if let Some(cached) = self.constraint_cache.get_mut(key) {
            // Update access time
            self.current_timestamp += 1;
            cached.last_accessed = self.current_timestamp;
            self.cache_timestamps
                .insert(key.clone(), self.current_timestamp);

            Maybe::Some(cached.clone())
        } else {
            Maybe::None
        }
    }

    /// Insert into cache with LRU eviction
    fn insert_into_cache(&mut self, key: ConstraintKey, result: CachedResult) {
        // Check if we need to evict
        if self.constraint_cache.len() >= self.max_cache_size {
            self.evict_lru();
        }

        self.current_timestamp += 1;
        self.constraint_cache.insert(key.clone(), result);
        self.cache_timestamps.insert(key, self.current_timestamp);
    }

    /// Evict least recently used cache entry
    fn evict_lru(&mut self) {
        if let Some((oldest_key, _)) = self
            .cache_timestamps
            .iter()
            .min_by_key(|(_, timestamp)| *timestamp)
        {
            let oldest_key = oldest_key.clone();
            self.constraint_cache.remove(&oldest_key);
            self.cache_timestamps.remove(&oldest_key);
        }
    }

    /// Simplify constraints by removing redundancies using logical implication
    ///

    /// This performs three simplification passes:
    /// 1. Deduplication: Remove exact duplicates
    /// 2. Subsumption: Remove weaker constraints implied by stronger ones
    /// 3. Protocol hierarchy: Use inheritance to eliminate redundant bounds
    fn simplify_constraints(&self, constraints: &List<GATConstraint>) -> List<GATConstraint> {
        if constraints.is_empty() {
            return List::new();
        }

        // Pass 1: Deduplication by fingerprint
        let mut seen_bounds = Set::new();
        let mut deduped = List::new();

        for constraint in constraints {
            let bound_str = format!("{}::{}", constraint.ty, constraint.bound.protocol);
            if !seen_bounds.contains(&bound_str) {
                seen_bounds.insert(bound_str.clone());
                deduped.push(constraint.clone());
            }
        }

        // Pass 2: Group constraints by type
        let mut by_type: Map<Text, List<GATConstraint>> = Map::new();
        for constraint in &deduped {
            let type_key = format!("{}", constraint.ty);
            by_type
                .entry(type_key.into())
                .or_default()
                .push(constraint.clone());
        }

        // Pass 3: For each type, remove subsumed bounds using protocol hierarchy
        let mut simplified = List::new();
        for (_ty_key, type_constraints) in by_type {
            let kept = self.remove_subsumed_bounds(&type_constraints);
            for c in kept {
                simplified.push(c);
            }
        }

        simplified
    }

    /// Remove bounds that are subsumed by more specific bounds
    ///

    /// If T: Ord and T: Eq, and Ord extends Eq, we only need T: Ord
    fn remove_subsumed_bounds(&self, constraints: &List<GATConstraint>) -> List<GATConstraint> {
        if constraints.len() <= 1 {
            return constraints.clone();
        }

        let mut kept = List::new();

        for (i, c1) in constraints.iter().enumerate() {
            let mut is_subsumed = false;
            let p1_name = self.extract_protocol_name(&c1.bound.protocol);

            for (j, c2) in constraints.iter().enumerate() {
                if i == j {
                    continue;
                }
                let p2_name = self.extract_protocol_name(&c2.bound.protocol);

                // Check if c2's bound implies c1's bound (c2 is more specific)
                // If p2 inherits from p1, then p2 implies p1
                if self.protocol_checker.inherits_from(&p2_name, &p1_name) {
                    is_subsumed = true;
                    break;
                }
            }

            if !is_subsumed {
                kept.push(c1.clone());
            }
        }

        kept
    }

    /// Extract protocol name from Path for comparison
    fn extract_protocol_name(&self, path: &verum_ast::ty::Path) -> Text {
        path.segments
            .last()
            .and_then(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => {
                    Some(verum_common::Text::from(ident.name.as_str()))
                }
                _ => None,
            })
            .unwrap_or_else(|| verum_common::Text::from(""))
    }

    /// Quick feasibility check using protocol hierarchy
    ///

    /// Checks for obvious contradictions in constraints:
    /// 1. Positive/negative bound conflicts (T: Clone vs T: !Clone)
    /// 2. Incompatible protocol requirements
    /// 3. Exceeded bound count heuristic
    fn quick_feasibility_check(&self, constraints: &List<GATConstraint>) -> bool {
        if constraints.is_empty() {
            return true;
        }

        // Group bounds by type variable
        let mut var_bounds: Map<u32, List<&crate::protocol::ProtocolBound>> = Map::new();

        for constraint in constraints {
            if let Type::Var(v) = &constraint.ty {
                var_bounds
                    .entry(v.id() as u32)
                    .or_default()
                    .push(&constraint.bound);
            }
        }

        // Check each variable's bounds for feasibility
        for (_var, bounds) in &var_bounds {
            if !self.check_bounds_compatible(bounds) {
                return false;
            }
        }

        true
    }

    /// Check if a set of bounds for a single type variable are compatible
    fn check_bounds_compatible(&self, bounds: &List<&crate::protocol::ProtocolBound>) -> bool {
        if bounds.len() <= 1 {
            return true;
        }

        // Separate positive and negative bounds
        let mut positive_bounds = List::new();
        let mut negative_bounds = List::new();

        for bound in bounds {
            if bound.is_negative {
                negative_bounds.push(*bound);
            } else {
                positive_bounds.push(*bound);
            }
        }

        // Check for direct conflicts: T: P and T: !P
        for pos in &positive_bounds {
            let pos_name = self.extract_protocol_name(&pos.protocol);
            for neg in &negative_bounds {
                let neg_name = self.extract_protocol_name(&neg.protocol);

                // Direct conflict
                if pos_name == neg_name {
                    return false;
                }

                // Inheritance conflict: if pos requires neg (e.g., Ord requires Eq, but !Eq)
                if self.protocol_checker.inherits_from(&pos_name, &neg_name) {
                    return false;
                }
            }
        }

        // Check positive bounds for compatibility using protocol hierarchy
        // Multiple bounds are compatible if they can all be satisfied by some type
        // Most common protocols are compatible (Clone, Debug, Eq, etc.)
        for i in 0..positive_bounds.len() {
            for j in (i + 1)..positive_bounds.len() {
                let p1 = &positive_bounds[i];
                let p2 = &positive_bounds[j];
                let p1_name = self.extract_protocol_name(&p1.protocol);
                let p2_name = self.extract_protocol_name(&p2.protocol);

                // Check if bounds are inherently incompatible
                if self.are_protocols_incompatible(&p1_name, &p2_name) {
                    return false;
                }
            }
        }

        // Heuristic: too many unrelated bounds is suspicious
        let unique_protocols: Set<_> = positive_bounds
            .iter()
            .map(|b| self.extract_protocol_name(&b.protocol))
            .collect();

        // Allow up to 8 different bounds (increased from 5 for complex GATs)
        if unique_protocols.len() > 8 {
            return false;
        }

        true
    }

    /// Check if two protocols are inherently incompatible
    ///

    /// Some protocol combinations are known to be impossible to satisfy together.
    fn are_protocols_incompatible(&self, p1: &Text, p2: &Text) -> bool {
        // Known incompatible pairs (can be extended)
        let incompatible_pairs = [
            ("Copy", "Drop"), // Copy types cannot have custom Drop
        ];

        for (a, b) in &incompatible_pairs {
            if (p1.as_str() == *a && p2.as_str() == *b) || (p1.as_str() == *b && p2.as_str() == *a)
            {
                return true;
            }
        }

        false
    }

    /// Build dependency graph for a GAT by traversing type bindings
    ///

    /// Finds dependent GATs by examining:
    /// 1. Associated types in bindings that reference other GATs
    /// 2. Type parameters that contain GAT applications
    /// 3. Protocol bounds that involve GAT constraints
    fn build_dependency_graph_for(&mut self, gat_id: &Text, bindings: &Map<Text, Type>) {
        // Insert root node if not exists
        if !self.dependency_graph.nodes.contains_key(gat_id) {
            self.dependency_graph.nodes.insert(
                gat_id.clone(),
                GATNode {
                    gat_id: gat_id.clone(),
                    depth: 0,
                    is_solved: false,
                    solution: Maybe::None,
                },
            );
        }

        // Ensure edges entry exists
        if !self.dependency_graph.edges.contains_key(gat_id) {
            self.dependency_graph
                .edges
                .insert(gat_id.clone(), Set::new());
        }

        // Traverse bindings to find dependent GATs
        let mut max_depth = 0;
        for (_name, ty) in bindings {
            let deps = self.find_gat_dependencies(ty);
            for dep_id in deps {
                if dep_id != *gat_id {
                    // Add forward edge
                    if let Some(edges) = self.dependency_graph.edges.get_mut(gat_id) {
                        edges.insert(dep_id.clone());
                    }

                    // Add reverse edge
                    self.dependency_graph
                        .reverse_edges
                        .entry(dep_id.clone())
                        .or_default()
                        .insert(gat_id.clone());

                    // Recursively build for dependencies
                    // Clone bindings to avoid borrowing issues
                    let empty_bindings = Map::new();
                    self.build_dependency_graph_for(&dep_id, &empty_bindings);

                    // Track maximum depth
                    if let Some(dep_node) = self.dependency_graph.nodes.get(&dep_id) {
                        max_depth = max_depth.max(dep_node.depth + 1);
                    }
                }
            }
        }

        // Update depth for this node
        if let Some(node) = self.dependency_graph.nodes.get_mut(gat_id) {
            node.depth = max_depth;
        }
    }

    /// Find GAT dependencies within a type
    fn find_gat_dependencies(&self, ty: &Type) -> List<verum_common::Text> {
        let mut deps = List::new();
        self.collect_gat_deps(ty, &mut deps);
        deps
    }

    /// Recursively collect GAT identifiers from a type
    fn collect_gat_deps(&self, ty: &Type, deps: &mut List<verum_common::Text>) {
        match ty {
            Type::Named { path, args } => {
                // Check if this is a GAT application (Protocol.AssocType<Args>)
                if path.segments.len() >= 2 {
                    // Extract potential GAT identifier
                    let gat_id = self.path_to_gat_id(path);
                    if !gat_id.is_empty() {
                        deps.push(gat_id);
                    }
                }
                // Recurse into type arguments
                for arg in args {
                    self.collect_gat_deps(arg, deps);
                }
            }
            Type::Generic { args, .. } => {
                // Recurse into type arguments
                for arg in args {
                    self.collect_gat_deps(arg, deps);
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.collect_gat_deps(param, deps);
                }
                self.collect_gat_deps(return_type, deps);
            }
            Type::Tuple(elements) => {
                for elem in elements {
                    self.collect_gat_deps(elem, deps);
                }
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. } => {
                self.collect_gat_deps(inner, deps);
            }
            Type::Slice { element } | Type::Array { element, .. } => {
                self.collect_gat_deps(element, deps);
            }
            _ => {} // Primitives, Var, etc. have no GAT deps
        }
    }

    /// Convert a Path to a GAT identifier string
    fn path_to_gat_id(&self, path: &verum_ast::ty::Path) -> Text {
        if path.segments.len() < 2 {
            return verum_common::Text::from("");
        }

        let parts: Vec<Text> = path
            .segments
            .iter()
            .filter_map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => {
                    Some(verum_common::Text::from(ident.name.as_str()))
                }
                _ => None,
            })
            .collect();

        if parts.len() >= 2 {
            verum_common::Text::from(format!(
                "{}.{}",
                parts[parts.len() - 2],
                parts[parts.len() - 1]
            ))
        } else {
            verum_common::Text::from("")
        }
    }

    /// Topological sort of dependency graph
    fn topological_sort(&self, root: &Text) -> List<verum_common::Text> {
        let mut result = List::new();
        let mut visited = Set::new();

        self.dfs_toposort(root, &mut visited, &mut result);

        // Reverse for bottom-up solving
        result.reverse();
        result
    }

    /// DFS for topological sort
    fn dfs_toposort(
        &self,
        gat_id: &Text,
        visited: &mut Set<Text>,
        stack: &mut List<verum_common::Text>,
    ) {
        if visited.contains(gat_id) {
            return;
        }
        visited.insert(gat_id.clone());

        // Visit dependencies first
        if let Some(deps) = self.dependency_graph.edges.get(gat_id) {
            for dep_id in deps.iter() {
                self.dfs_toposort(dep_id, visited, stack);
            }
        }

        stack.push(gat_id.clone());
    }

    /// Solve GAT constraints using proper constraint unification
    ///

    /// # Algorithm
    ///

    /// 1. Group constraints by type variable
    /// 2. For each variable, find candidate types from bindings
    /// 3. Filter candidates by checking all bounds are satisfied
    /// 4. Find intersection of valid candidates
    /// 5. Return the most specific type that satisfies all constraints
    ///

    /// # Returns
    ///

    /// - `Some(type)` if a solution exists
    /// - `None` if constraints are unsatisfiable
    fn solve_gat_constraints(
        &self,
        bindings: &Map<Text, Type>,
        constraints: &List<GATConstraint>,
    ) -> Maybe<Type> {
        if constraints.is_empty() {
            // No constraints - return first concrete type from bindings
            return self.find_first_concrete_type(bindings);
        }

        // Group constraints by the type they constrain
        let grouped = self.group_constraints_by_type(constraints);

        // Collect all candidate concrete types from bindings
        let candidates: List<&Type> = bindings
            .values()
            .filter(|ty| !matches!(ty, Type::Var(_)))
            .collect();

        if candidates.is_empty() {
            // No concrete types available - try to synthesize from constraints
            return self.synthesize_from_constraints(constraints);
        }

        // For each candidate, check if it satisfies all constraints
        let mut valid_candidates = List::new();

        for candidate in &candidates {
            if self.satisfies_all_constraints(candidate, constraints) {
                valid_candidates.push((*candidate).clone());
            }
        }

        if valid_candidates.is_empty() {
            // No candidate satisfies all constraints
            // Try to find a type that partially satisfies (for error recovery)
            return Maybe::None;
        }

        if valid_candidates.len() == 1 {
            return Maybe::Some(valid_candidates[0].clone());
        }

        // Multiple valid candidates - find the most specific one
        self.find_most_specific_type(&valid_candidates, &grouped)
    }

    /// Find the first concrete (non-variable) type in bindings
    fn find_first_concrete_type(&self, bindings: &Map<Text, Type>) -> Maybe<Type> {
        for (_name, ty) in bindings {
            if !matches!(ty, Type::Var(_)) {
                return Maybe::Some(ty.clone());
            }
        }
        Maybe::None
    }

    /// Group constraints by the type they constrain
    fn group_constraints_by_type(
        &self,
        constraints: &List<GATConstraint>,
    ) -> Map<Text, List<GATConstraint>> {
        let mut grouped: Map<Text, List<GATConstraint>> = Map::new();

        for constraint in constraints {
            let type_key = format!("{}", constraint.ty);
            grouped
                .entry(type_key.into())
                .or_default()
                .push(constraint.clone());
        }

        grouped
    }

    /// Check if a type satisfies all given constraints
    fn satisfies_all_constraints(&self, ty: &Type, constraints: &List<GATConstraint>) -> bool {
        for constraint in constraints {
            if !self.type_satisfies_constraint(ty, constraint) {
                return false;
            }
        }
        true
    }

    /// Check if a type satisfies a single constraint
    fn type_satisfies_constraint(&self, ty: &Type, constraint: &GATConstraint) -> bool {
        // Check if the constrained type matches or unifies with ty
        if !self.types_unify_for_constraint(ty, &constraint.ty) {
            // Constraint is for a different type - doesn't apply
            return true;
        }

        // Check the protocol bound
        let protocol_name = self.extract_protocol_name(&constraint.bound.protocol);

        if constraint.bound.is_negative {
            // Negative bound: type must NOT implement protocol
            !self
                .protocol_checker
                .implements_protocol(ty, protocol_name.as_str())
        } else {
            // Positive bound: type must implement protocol
            self.protocol_checker
                .implements_protocol(ty, protocol_name.as_str())
        }
    }

    /// Check if two types unify for constraint purposes
    fn types_unify_for_constraint(&self, t1: &Type, t2: &Type) -> bool {
        match (t1, t2) {
            // Type variables unify with anything
            (Type::Var(_), _) | (_, Type::Var(_)) => true,

            // Same concrete types unify
            (Type::Unit, Type::Unit) => true,
            (Type::Bool, Type::Bool) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Char, Type::Char) => true,
            (Type::Text, Type::Text) => true,

            // Named types unify if paths match (ignoring type args for now)
            (Type::Named { path: p1, .. }, Type::Named { path: p2, .. }) => {
                self.paths_equal(p1, p2)
            }

            // References unify if mutability and inner types match
            (
                Type::Reference {
                    inner: i1,
                    mutable: m1,
                },
                Type::Reference {
                    inner: i2,
                    mutable: m2,
                },
            ) => m1 == m2 && self.types_unify_for_constraint(i1, i2),

            // Different concrete types don't unify
            _ => false,
        }
    }

    /// Check if two paths are equal
    fn paths_equal(&self, p1: &verum_ast::ty::Path, p2: &verum_ast::ty::Path) -> bool {
        if p1.segments.len() != p2.segments.len() {
            return false;
        }

        for (s1, s2) in p1.segments.iter().zip(p2.segments.iter()) {
            match (s1, s2) {
                (verum_ast::ty::PathSegment::Name(id1), verum_ast::ty::PathSegment::Name(id2)) => {
                    if id1.name != id2.name {
                        return false;
                    }
                }
                _ => return false,
            }
        }

        true
    }

    /// Synthesize a type from constraints when no concrete candidates exist
    fn synthesize_from_constraints(&self, constraints: &List<GATConstraint>) -> Maybe<Type> {
        // Try to find a common type that satisfies all bounds
        // This is useful for type inference when we have bounds but no concrete type

        if constraints.is_empty() {
            return Maybe::None;
        }

        // Collect all positive bounds
        let positive_bounds: List<_> = constraints
            .iter()
            .filter(|c| !c.bound.is_negative)
            .collect();

        if positive_bounds.is_empty() {
            return Maybe::None;
        }

        // Try common built-in types that might satisfy the bounds
        let candidates = [Type::Int, Type::Float, Type::Bool, Type::Text, Type::Char];

        for candidate in &candidates {
            if self.satisfies_all_constraints(candidate, constraints) {
                return Maybe::Some(candidate.clone());
            }
        }

        Maybe::None
    }

    /// Find the most specific type among valid candidates
    ///

    /// Uses protocol hierarchy to determine specificity:
    /// - A type implementing Ord is more specific than one only implementing Eq
    fn find_most_specific_type(
        &self,
        candidates: &List<Type>,
        grouped_constraints: &Map<Text, List<GATConstraint>>,
    ) -> Maybe<Type> {
        if candidates.is_empty() {
            return Maybe::None;
        }

        if candidates.len() == 1 {
            return Maybe::Some(candidates[0].clone());
        }

        // Score each candidate by the number of protocols it implements
        // More protocols = more specific
        let mut best_candidate = &candidates[0];
        let mut best_score = 0usize;

        for candidate in candidates {
            let score = self.compute_specificity_score(candidate, grouped_constraints);
            if score > best_score {
                best_score = score;
                best_candidate = candidate;
            }
        }

        Maybe::Some(best_candidate.clone())
    }

    /// Compute a specificity score for a type based on protocol implementations
    fn compute_specificity_score(
        &self,
        ty: &Type,
        grouped_constraints: &Map<Text, List<GATConstraint>>,
    ) -> usize {
        let mut score = 0;

        // Base score for concrete types
        match ty {
            Type::Var(_) => score += 0,
            Type::Unit => score += 1,
            Type::Bool | Type::Char => score += 2,
            Type::Int | Type::Float => score += 3,
            Type::Text => score += 4,
            Type::Named { .. } => score += 5,
            _ => score += 1,
        }

        // Bonus for each constraint satisfied
        for (_ty_key, constraints) in grouped_constraints {
            for constraint in constraints {
                if self.type_satisfies_constraint(ty, constraint) {
                    score += 1;
                }
            }
        }

        // Bonus for types that implement more specific protocols
        let specific_protocols = ["Ord", "Hash", "Clone", "Copy"];
        for protocol in &specific_protocols {
            if self.protocol_checker.implements_protocol(ty, protocol) {
                score += 2;
            }
        }

        score
    }

    /// Incremental invalidation when constraints change
    pub fn invalidate_dependents(&mut self, changed_gat: &Text) {
        // Find all GATs that depend on changed GAT
        let mut to_invalidate = Set::new();
        to_invalidate.insert(changed_gat.clone());

        // BFS to find transitive dependents
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(changed_gat.clone());

        while let Some(current) = queue.pop_front() {
            // Find all GATs that depend on current
            if let Some(dependents) = self.dependency_graph.reverse_edges.get(&current) {
                for dependent in dependents.iter() {
                    if !to_invalidate.contains(dependent) {
                        to_invalidate.insert(dependent.clone());
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }

        // Remove from cache
        self.constraint_cache
            .retain(|key, _| !to_invalidate.contains(&key.gat_id));

        // Mark as unsolved in graph
        for gat_id in to_invalidate {
            if let Some(node) = self.dependency_graph.nodes.get_mut(&gat_id) {
                node.is_solved = false;
                node.solution = Maybe::None;
            }
        }

        self.stats.incremental_updates += 1;
    }

    /// Get performance statistics
    pub fn get_stats(&self) -> &GATInferenceStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = GATInferenceStats::default();
    }

    /// Clear cache (for testing or memory management)
    pub fn clear_cache(&mut self) {
        self.constraint_cache.clear();
        self.cache_timestamps.clear();
        self.dependency_graph = DependencyGraph::default();
        self.current_timestamp = 0;
    }
}

// Helper trait for conditions
trait ConditionExt {
    /// Get as a simple expression condition (for backward compatibility)
    fn as_expr(&self) -> Result<&Expr>;

    /// Check if this is a let condition
    fn is_let(&self) -> bool;

    /// Get as a let condition (pattern and value)
    fn as_let(&self) -> Option<(&Pattern, &Expr)>;
}

impl ConditionExt for verum_ast::expr::ConditionKind {
    fn as_expr(&self) -> Result<&Expr> {
        match self {
            verum_ast::expr::ConditionKind::Expr(e) => Ok(e),
            verum_ast::expr::ConditionKind::Let { .. } => {
                Err(TypeError::Other(verum_common::Text::from(
                    "Expected boolean expression, found let pattern. Use as_let() for let conditions.",
                )))
            }
        }
    }

    fn is_let(&self) -> bool {
        matches!(self, verum_ast::expr::ConditionKind::Let { .. })
    }

    fn as_let(&self) -> Option<(&Pattern, &Expr)> {
        match self {
            verum_ast::expr::ConditionKind::Let { pattern, value } => Some((pattern, value)),
            _ => None,
        }
    }
}

/// Helper to check a condition and optionally bind patterns to scope
///

/// Map `@builtin_*` meta-type markers that appear on the RHS of stdlib
/// type aliases (e.g. `type I is @builtin_interval;`) to the internal
/// `Type` primitive they stand for. Returns `None` for names that are
/// not cubical / HoTT language primitives — those fall through to the
/// regular qualified-path resolution so that user-defined `@` meta
/// types continue to work.
/// Returns true if the given mount tree re-exports an item named `name`.
/// Used by import resolution to recognise that a name is re-exported
/// through a `public mount .path.{item}` chain even when it isn't a
/// direct item in the module's items list.
fn mount_tree_exports_name(tree: &verum_ast::decl::MountTree, name: &str) -> bool {
    use verum_ast::decl::MountTreeKind;
    use verum_ast::ty::PathSegment;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Last segment is the imported item name (or rename target).
            if let Some(PathSegment::Name(ident)) = path.segments.last() {
                return ident.name.as_str() == name;
            }
            false
        }
        MountTreeKind::Glob(_) => {
            // Glob re-exports everything; conservatively treat as exporting
            // any name. Callers fall through to real resolution.
            true
        }
        MountTreeKind::Nested { trees, .. } => {
            trees.iter().any(|t| mount_tree_exports_name(t, name))
        }
        // #5 / P1.5 — file-relative mounts contribute exports
        // through the session loader's per-file module
        // registration, not through this AST-level export
        // probe. The alias (if any) is the only name
        // observable from the importing scope.
        MountTreeKind::File { .. } => tree
            .alias
            .as_ref()
            .map(|a| a.name.as_str() == name)
            .unwrap_or(false),
    }
}

fn resolve_builtin_meta_type(name: &str) -> Option<Type> {
    match name {
        "@builtin_interval" => Some(Type::Interval),
        // Path / Glue are type *constructors* that already have their own
        // AST sugaring (`TypeKind::PathType`, `TypeKind::DependentApp`) at
        // use sites. The bare marker on the alias RHS is an opaque primitive
        // stand-in — the declared carrier (`Path`, `Glue`) becomes a named
        // type pointing at this opaque marker, and real uses `Path<A>(a, b)`
        // take the sugared AST path and lower to `Type::Eq` / dependent app.
        "@builtin_path" => Some(Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                verum_common::Text::from("Path"),
                verum_ast::span::Span::dummy(),
            )),
            args: List::new(),
        }),
        "@builtin_glue" => Some(Type::Named {
            path: verum_ast::ty::Path::single(verum_ast::Ident::new(
                verum_common::Text::from("Glue"),
                verum_ast::span::Span::dummy(),
            )),
            args: List::new(),
        }),
        _ => None,
    }
}

/// This handles both expression conditions and let conditions:
/// - `if x > 0` - Expression condition (must evaluate to Bool)
/// - `if let Some(v) = opt` - Let condition (pattern must match value)
///

/// Returns the type of bindings introduced (empty for expression conditions)
fn check_condition(
    checker: &mut TypeChecker,
    condition: &verum_ast::expr::ConditionKind,
) -> Result<List<(Text, Type)>> {
    match condition {
        verum_ast::expr::ConditionKind::Expr(expr) => {
            // Expression condition - must be Bool
            checker.check_expr(expr, &Type::bool())?;
            Ok(List::new())
        }
        verum_ast::expr::ConditionKind::Let { pattern, value } => {
            // Let condition - bind pattern to value
            // If-let expressions: "if let Pattern = expr { ... }" for refutable pattern matching with type narrowing
            //

            // The pattern is checked against the type of the value,
            // and any bound variables become available in the then-branch.
            let value_result = checker.synth_expr(value)?;
            let value_ty = value_result.ty;

            // Bind pattern to value type
            checker.bind_pattern(pattern, &value_ty)?;

            // Collect bindings introduced by the pattern
            let bindings = checker.collect_pattern_bindings(pattern, &value_ty)?;

            Ok(bindings)
        }
    }
}

/// Check all conditions in an if-condition chain
///

/// Handles both simple conditions and let-chains like:
/// `if let Some(x) = opt && x > 0`
fn check_all_conditions(
    checker: &mut TypeChecker,
    conditions: &verum_ast::expr::IfCondition,
) -> Result<List<(Text, Type)>> {
    let mut all_bindings = List::new();

    for cond in &conditions.conditions {
        let mut bindings = check_condition(checker, cond)?;
        all_bindings.append(&mut bindings);
    }

    Ok(all_bindings)
}

/// Compute Levenshtein distance between two strings for suggestions
///

/// Used to provide "did you mean?" suggestions in error messages.
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.len();
    let len2 = s2.len();

    // Fast paths
    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }
    if s1 == s2 {
        return 0;
    }

    // Standard dynamic programming approach
    let mut prev_row: List<usize> = (0..=len2).collect();
    let mut curr_row: List<usize> = vec![0; len2 + 1].into();

    for (i, c1) in s1.chars().enumerate() {
        curr_row[0] = i + 1;

        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[len2]
}

// ==================== Helper Functions ====================

/// Helper function to convert Span to LineColSpan using the global source file registry
pub(crate) fn span_to_line_col(span: Span) -> verum_common::span::LineColSpan {
    // Use verum_common's global registry as that's where compiler registers files
    verum_common::global_span_to_line_col(span)
}

/// Helper function to get a human-readable description of an ExprKind
fn expr_kind_description(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Literal(_) => "literal",
        ExprKind::Path(_) => "path",
        ExprKind::Binary { .. } => "binary operation",
        ExprKind::Unary { .. } => "unary operation",
        ExprKind::Call { .. } => "function call",
        ExprKind::MethodCall { .. } => "method call",
        ExprKind::Field { .. } => "field access",
        ExprKind::OptionalChain { .. } => "optional chain",
        ExprKind::TupleIndex { .. } => "tuple index",
        ExprKind::Index { .. } => "index operation",
        ExprKind::Pipeline { .. } => "pipeline",
        ExprKind::NullCoalesce { .. } => "null coalesce",
        ExprKind::Cast { .. } => "type cast",
        ExprKind::Try(_) => "try expression",
        ExprKind::TryBlock(_) => "try block",
        ExprKind::TryRecover { .. } => "try-recover expression",
        ExprKind::TryFinally { .. } => "try-finally expression",
        ExprKind::TryRecoverFinally { .. } => "try-recover-finally expression",
        ExprKind::Block(_) => "block",
        ExprKind::If { .. } => "if expression",
        ExprKind::Match { .. } => "match expression",
        ExprKind::While { .. } => "while loop",
        ExprKind::For { .. } => "for loop",
        ExprKind::ForAwait { .. } => "for await loop",
        ExprKind::Loop { .. } => "loop",
        ExprKind::Break { .. } => "break",
        ExprKind::Continue { .. } => "continue",
        ExprKind::Return(_) => "return",
        ExprKind::Yield(_) => "yield",
        ExprKind::Closure { .. } => "closure",
        ExprKind::Tuple(_) => "tuple",
        ExprKind::Array(_) => "array",
        ExprKind::Record { .. } => "record",
        ExprKind::InterpolatedString { .. } => "interpolated string",
        ExprKind::TensorLiteral { .. } => "tensor literal",
        ExprKind::MapLiteral { .. } => "map literal",
        ExprKind::SetLiteral { .. } => "set literal",
        ExprKind::Range { .. } => "range",
        ExprKind::Comprehension { .. } => "comprehension",
        ExprKind::StreamComprehension { .. } => "stream comprehension",
        ExprKind::MapComprehension { .. } => "map comprehension",
        ExprKind::SetComprehension { .. } => "set comprehension",
        ExprKind::GeneratorComprehension { .. } => "generator expression",
        ExprKind::StreamLiteral(_) => "stream literal",
        ExprKind::Forall { .. } => "forall expression",
        ExprKind::Exists { .. } => "exists expression",
        ExprKind::Attenuate { .. } => "attenuate expression",
        ExprKind::Async(_) => "async expression",
        ExprKind::Await(_) => "await expression",
        ExprKind::Spawn { .. } => "spawn expression",
        ExprKind::Inject { .. } => "inject expression",
        ExprKind::Unsafe(_) => "unsafe block",
        ExprKind::Meta(_) => "meta block",
        ExprKind::Quote { .. } => "quote expression",
        ExprKind::StageEscape { .. } => "stage escape expression",
        ExprKind::Lift { .. } => "lift expression",
        ExprKind::MacroCall { .. } => "macro call",
        ExprKind::UseContext { .. } => "context handler binding",
        ExprKind::Paren(_) => "parenthesized expression",
        ExprKind::TypeProperty { .. } => "type property expression",
        ExprKind::TypeExpr(_) => "type expression",
        ExprKind::Select { .. } => "select expression",
        ExprKind::Throw(_) => "throw expression",
        ExprKind::Typeof(_) => "typeof expression",
        ExprKind::Is { .. } => "is pattern test",
        ExprKind::TypeBound { .. } => "type bound expression",
        ExprKind::MetaFunction { .. } => "meta function",
        ExprKind::Nursery { .. } => "nursery expression",
        ExprKind::InlineAsm { .. } => "inline assembly",
        ExprKind::DestructuringAssign { .. } => "destructuring assignment",
        ExprKind::CalcBlock(_) => "calc block",
        ExprKind::NamedArg { .. } => "named argument",
        ExprKind::CopatternBody { .. } => "copattern body",
    }
}

/// Helper function to get a human-readable description of a TypeKind
fn type_kind_description(kind: &verum_ast::ty::TypeKind) -> String {
    use verum_ast::ty::TypeKind;
    match kind {
        TypeKind::Unit => "unit type ()".to_string(),
        TypeKind::Bool => WKT::Bool.as_str().to_string(),
        TypeKind::Int => WKT::Int.as_str().to_string(),
        TypeKind::Float => WKT::Float.as_str().to_string(),
        TypeKind::Char => WKT::Char.as_str().to_string(),
        TypeKind::Text => WKT::Text.as_str().to_string(),
        TypeKind::Never => "never type !".to_string(),
        TypeKind::Path(path) => format!("path '{}'", path),
        TypeKind::PathType { .. } => "path type Path<A>(a, b)".to_string(),
        TypeKind::DependentApp { .. } => "dependent type application T<..>(v..)".to_string(),
        TypeKind::Tuple(_) => "tuple type".to_string(),
        TypeKind::Array { .. } => "array type".to_string(),
        TypeKind::Slice(_) => "slice type".to_string(),
        TypeKind::Function { .. } => "function type".to_string(),
        TypeKind::Rank2Function { .. } => "rank-2 function type".to_string(),
        TypeKind::Reference { .. } => "reference type".to_string(),
        TypeKind::CheckedReference { .. } => "checked reference type".to_string(),
        TypeKind::UnsafeReference { .. } => "unsafe reference type".to_string(),
        TypeKind::Pointer { .. } => "pointer type".to_string(),
        TypeKind::VolatilePointer { .. } => "volatile pointer type".to_string(),
        TypeKind::Generic { .. } => "generic type".to_string(),
        TypeKind::Qualified { .. } => "qualified type".to_string(),
        TypeKind::Refined { .. } => "refinement type".to_string(),
        TypeKind::Inferred => "inferred type".to_string(),
        TypeKind::Bounded { .. } => "bounded type".to_string(),
        TypeKind::DynProtocol { .. } => "dyn protocol type".to_string(),
        TypeKind::Ownership { .. } => "ownership type".to_string(),
        TypeKind::GenRef { .. } => "GenRef type".to_string(),
        TypeKind::TypeConstructor { .. } => "type constructor".to_string(),
        TypeKind::Tensor { .. } => "tensor type".to_string(),
        TypeKind::Existential { .. } => "existential type".to_string(),
        TypeKind::AssociatedType { .. } => "associated type".to_string(),
        TypeKind::CapabilityRestricted { .. } => "capability-restricted type".to_string(),
        TypeKind::Unknown => "Unknown type".to_string(),
        TypeKind::Record { .. } => "record type".to_string(),
        TypeKind::Universe { .. } => "universe type".to_string(),
        TypeKind::Meta { .. } => "meta type".to_string(),
        TypeKind::TypeLambda { .. } => "type lambda".to_string(),
    }
}

// ==================== Type Registration for Pipeline ====================

// Type declaration registration methods (register_type_declaration*, register_variant*, …)
// → see infer/decls.rs in this module


// ==================== HKT Instantiation ====================

impl TypeChecker {
    /// Check kind compatibility when applying a type constructor to arguments.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
    ///

    /// When applying `F<Int>` where `F: * -> *`, this verifies:
    /// 1. F has the expected constructor kind (* -> *)
    /// 2. Int has kind * (the expected argument kind)
    /// 3. The resulting application F<Int> has kind *
    ///

    /// # Arguments
    ///

    /// * `constructor` - The type constructor being applied (e.g., F, List, Map)
    /// * `args` - The type arguments being applied
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(Kind)` - The resulting kind after application
    /// * `Err(TypeError)` - If kind mismatch or arity error
    ///

    /// # Examples
    ///

    /// ```ignore
    /// // F<Int> where F: * -> *
    /// let result_kind = checker.check_type_application_kind(
    ///  &Type::TypeConstructor { name: "F".into(), arity: 1, kind: Kind::unary_constructor() },
    ///  &[Type::Int],
    ///  Span::default()
    /// )?;
    /// assert_eq!(result_kind, Kind::Type);
    /// ```
    pub fn check_type_application_kind(
        &mut self,
        constructor: &Type,
        args: &[Type],
        span: Span,
    ) -> Result<crate::kind_inference::Kind> {
        if !self.higher_kinded_enabled {
            // HKT disabled — skip kind checking, assume kind Type for all.
            return Ok(crate::kind_inference::Kind::Type);
        }
        self.kind_inferer
            .check_type_application_kind(constructor, args, span)
    }

    /// Instantiate an HKT parameter with a concrete type constructor.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT parameter instantiation
    ///

    /// When calling `fn foo<F<_>: Functor>(x: F<Int>)` with `foo::<List>(...)`,
    /// this verifies:
    /// 1. `List` has kind `* -> *` (matches F's expected kind)
    /// 2. `List` implements `Functor` (satisfies protocol bound)
    ///

    /// # Arguments
    ///

    /// * `hkt_param_name` - Name of the HKT parameter (e.g., "F")
    /// * `expected_kind` - The expected kind for the parameter (e.g., * -> *)
    /// * `concrete_constructor` - The concrete type constructor being substituted (e.g., List)
    /// * `protocol_bounds` - Protocol bounds that must be satisfied (e.g., Functor)
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(HKTInstantiationResult)` - Successful instantiation with result info
    /// * `Err(TypeError)` - If kind mismatch or protocol not implemented
    ///

    /// # Examples
    ///

    /// ```ignore
    /// // Instantiate F<_> with List where F<_>: Functor
    /// let result = checker.instantiate_hkt_param(
    ///  "F",
    ///  &Kind::unary_constructor(),
    ///  &Type::TypeConstructor { name: "List".into(), arity: 1, kind: Kind::unary_constructor() },
    ///  &[ProtocolBound::simple("Functor".into())],
    ///  Span::default(),
    /// )?;
    /// ```
    pub fn instantiate_hkt_param(
        &mut self,
        hkt_param_name: &str,
        expected_kind: &crate::kind_inference::Kind,
        concrete_constructor: &Type,
        protocol_bounds: &[crate::protocol::ProtocolBound],
        span: Span,
    ) -> Result<crate::kind_inference::HKTInstantiationResult> {
        // Extract protocol checker reference to avoid self borrow conflict
        // We use a reference to the protocol_checker directly in the closure
        let protocol_checker = &self.protocol_checker;

        // Create a closure that checks protocol implementation using the protocol_checker
        let check_protocol = |ty: &Type, bound: &crate::protocol::ProtocolBound| -> bool {
            // Extract the constructor name
            let constructor_name: Text = match ty {
                Type::TypeConstructor { name, .. } => name.clone(),
                Type::Named { path, .. } => path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                        _ => "unknown".into(),
                    })
                    .unwrap_or_else(|| "unknown".into()),
                Type::Generic { name, .. } => name.clone(),
                _ => return false,
            };

            // Extract protocol name from path
            let protocol_name: Text = bound
                .protocol
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                    _ => verum_common::Text::from("unknown"),
                })
                .unwrap_or_else(|| verum_common::Text::from("unknown"));

            // Check if the protocol implementation is registered
            protocol_checker
                .read()
                .type_constructor_implements_protocol(&constructor_name, &protocol_name)
        };

        self.kind_inferer.instantiate_hkt_param(
            hkt_param_name,
            expected_kind,
            concrete_constructor,
            protocol_bounds,
            span,
            check_protocol,
        )
    }

    /// Check if a type constructor implements a protocol.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Protocol checking for type constructors
    ///

    /// For HKT bounds like `F<_>: Functor + Monad`, this checks if the type
    /// constructor (e.g., List, Maybe) implements the required protocol.
    ///

    /// # Arguments
    ///

    /// * `constructor` - The type constructor to check
    /// * `bound` - The protocol bound that must be satisfied
    ///

    /// # Returns
    ///

    /// * `true` if the constructor implements the protocol
    /// * `false` otherwise
    ///

    /// # Examples
    ///

    /// ```ignore
    /// let list_ctor = Type::TypeConstructor { name: "List".into(), arity: 1, kind: Kind::unary_constructor() };
    /// let functor_bound = ProtocolBound::simple("Functor".into());
    /// let implements = checker.check_type_constructor_implements_protocol(&list_ctor, &functor_bound);
    /// ```
    pub fn check_type_constructor_implements_protocol(
        &self,
        constructor: &Type,
        bound: &crate::protocol::ProtocolBound,
    ) -> bool {
        // Extract the constructor name
        let constructor_name = match constructor {
            Type::TypeConstructor { name, .. } => name.clone(),
            Type::Named { path, .. } => path
                .segments
                .last()
                .map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                    _ => "unknown".into(),
                })
                .unwrap_or_else(|| "unknown".into()),
            Type::Generic { name, .. } => name.clone(),
            _ => return false,
        };

        // Extract the protocol name from the Path
        let protocol_name: Text = bound
            .protocol
            .segments
            .last()
            .map(|seg| match seg {
                verum_ast::ty::PathSegment::Name(ident) => ident.name.clone(),
                _ => verum_common::Text::from("unknown"),
            })
            .unwrap_or_else(|| verum_common::Text::from("unknown"));

        // Check if the protocol implementation is registered
        // The protocol checker tracks implementations by (type_name, protocol_name)
        self.protocol_checker
            .read()
            .type_constructor_implements_protocol(&constructor_name, &protocol_name)
    }

    /// Verify HKT bounds for a function call with type constructor arguments.
    ///

    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — HKT verification during type checking
    ///

    /// When calling a function like `fn traverse<F<_>: Applicative, A, B>(...)`
    /// with concrete type constructor arguments, this method verifies all HKT
    /// constraints are satisfied.
    ///

    /// # Arguments
    ///

    /// * `hkt_params` - List of (param_name, expected_kind, protocol_bounds)
    /// * `concrete_args` - The concrete type constructors being substituted
    /// * `span` - Source location for error reporting
    ///

    /// # Returns
    ///

    /// * `Ok(List<HKTInstantiationResult>)` - All instantiations succeeded
    /// * `Err(TypeError)` - First failing constraint
    pub fn verify_hkt_bounds(
        &mut self,
        hkt_params: &[(
            Text,
            crate::kind_inference::Kind,
            List<crate::protocol::ProtocolBound>,
        )],
        concrete_args: &[Type],
        span: Span,
    ) -> Result<List<crate::kind_inference::HKTInstantiationResult>> {
        if hkt_params.len() != concrete_args.len() {
            return Err(TypeError::Other(
                format!(
                    "Expected {} HKT arguments but got {}",
                    hkt_params.len(),
                    concrete_args.len()
                )
                .into(),
            ));
        }

        let mut results = List::new();
        let mut protocol_errors = List::new();

        for (i, (param_name, expected_kind, bounds)) in hkt_params.iter().enumerate() {
            let concrete = &concrete_args[i];

            let result = self.instantiate_hkt_param(
                param_name.as_str(),
                expected_kind,
                concrete,
                bounds.as_slice(),
                span,
            )?;

            // Collect protocol bound failures for better error messages
            if !result.protocol_bounds_satisfied {
                for bound in bounds {
                    if !self.check_type_constructor_implements_protocol(concrete, bound) {
                        protocol_errors.push((
                            param_name.clone(),
                            bound.protocol.clone(),
                            concrete.clone(),
                        ));
                    }
                }
            }

            results.push(result);
        }

        // Report all protocol violations at once for better error messages
        if !protocol_errors.is_empty() {
            let error_msg = protocol_errors
                .iter()
                .map(|(param, protocol, ty)| {
                    format!(
                        "HKT parameter '{}' requires '{}' to implement '{}'",
                        param,
                        self.type_display(ty),
                        protocol
                    )
                })
                .collect::<List<String>>()
                .join("; ");

            return Err(TypeError::ProtocolNotSatisfied {
                ty: format!("{:?}", concrete_args).into(),
                protocol: error_msg,
                span,
            });
        }

        Ok(results)
    }

    /// Helper to display a type for error messages
    fn type_display(&self, ty: &Type) -> String {
        match ty {
            Type::TypeConstructor { name, arity, .. } => {
                if *arity > 0 {
                    format!("{}<{}>", name, "_,".repeat(*arity).trim_end_matches(','))
                } else {
                    name.to_string()
                }
            }
            Type::Named { path, args } => {
                let name = path
                    .segments
                    .last()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.to_string(),
                        _ => "?".to_string(),
                    })
                    .unwrap_or_else(|| "?".to_string());
                if args.is_empty() {
                    name
                } else {
                    format!("{}<{}>", name, args.len())
                }
            }
            Type::Generic { name, args } => {
                if args.is_empty() {
                    name.to_string()
                } else {
                    format!("{}<{}>", name, args.len())
                }
            }
            Type::TypeApp { constructor, args } => {
                format!("{}<{} args>", self.type_display(constructor), args.len())
            }
            _ => format!("{:?}", ty),
        }
    }
}

// ==================== Kind Annotation Conversion ====================

impl TypeChecker {
    /// Convert an AST `KindAnnotation` (from `verum_ast`) to the type-checker's
    /// `kind_inference::Kind`, which is used internally for kind constraint solving.
    ///

    /// Both types represent the same algebra (`Type | K1 -> K2`) but live in
    /// different crates to avoid a circular dependency.
    pub(crate) fn ast_kind_to_infer_kind(
        ann: &verum_ast::ty::KindAnnotation,
    ) -> crate::kind_inference::Kind {
        match ann {
            verum_ast::ty::KindAnnotation::Type => crate::kind_inference::Kind::Type,
            verum_ast::ty::KindAnnotation::Arrow(lhs, rhs) => crate::kind_inference::Kind::Arrow(
                Box::new(Self::ast_kind_to_infer_kind(lhs)),
                Box::new(Self::ast_kind_to_infer_kind(rhs)),
            ),
        }
    }
}

// ==================== Kind Inference Integration ====================

impl crate::kind_inference::KindInference for TypeChecker {
    fn kind_inferer(&mut self) -> &mut crate::kind_inference::KindInferer {
        &mut self.kind_inferer
    }

    fn check_kind(&mut self, ty: &Type, expected_kind: &crate::kind_inference::Kind) -> Result<()> {
        self.kind_inferer.check_kind(ty, expected_kind)
    }

    fn infer_kind(&mut self, ty: &Type) -> Result<crate::kind_inference::Kind> {
        self.kind_inferer.infer_kind(ty)
    }

    fn check_protocol_kinds(&mut self, protocol: &crate::protocol::Protocol) -> Result<()> {
        self.kind_inferer.check_protocol_kinds(protocol)
    }
}

// ==================== QTT V2 helpers (#235, A.Z.5 §7.6) ====================

/// extract the declared QTT [`crate::ty::Quantity`]
/// from a parameter's attribute list.
///

/// Reads the first `@quantity(...)` attribute via
/// [`verum_ast::attr::QuantityAttr::from_attribute`] and maps the
/// AST-side enum (`Zero / One / Many`) to the verum_types-side
/// [`crate::ty::Quantity`] (`Zero / One / Omega / AtMost / Graded`).
/// Returns `Quantity::Omega` (unrestricted) when no `@quantity`
/// attribute is present — matches default.
fn extract_quantity_from_attrs(
    attrs: &verum_common::List<verum_ast::attr::Attribute>,
) -> crate::ty::Quantity {
    use verum_ast::attr::{Quantity as AstQty, QuantityAttr};
    use verum_common::Maybe;
    for a in attrs.iter() {
        if let Maybe::Some(parsed) = QuantityAttr::from_attribute(a) {
            return match parsed.quantity {
                AstQty::Zero => crate::ty::Quantity::Zero,
                AstQty::One => crate::ty::Quantity::One,
                AstQty::Many => crate::ty::Quantity::Omega,
            };
        }
    }
    crate::ty::Quantity::Omega
}

/// walk a single statement node, accumulating QTT
/// usage for tracked bindings into `usage`. Per QTT calculus,
/// statements compose sequentially — each contributes
/// `merge_sequential` to the running tally.
///

/// Recognised statement shapes:
///  * `Stmt::Expr { expr, .. }` — recurse into expr.
///  * `Stmt::Let { value, .. }` — recurse into the initialiser.
///  * `Stmt::LetElse { value, else_block, .. }` — initialiser is
///  sequential; else_block is taken as a branch (worst-case
///  accumulated via merge_sequential since the LetElse else
///  path runs only on pattern-mismatch — pessimistic).
///  * `Stmt::Defer(expr)` / `Errdefer(expr)` — recurse.
///  * Other Stmt variants (Item, etc.) — no value-usage, skip.
fn walk_stmt_for_qtt_usage(
    tracked: &std::collections::HashSet<verum_common::Text>,
    stmt: &verum_ast::stmt::Stmt,
    usage: &mut crate::qtt_usage::UsageMap,
) {
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Let { value, .. } => {
            if let verum_common::Maybe::Some(v) = value {
                let d = crate::qtt_walker::walk_expr(tracked, v);
                let merged = std::mem::take(usage).merge_sequential(d);
                *usage = merged;
            }
        }
        StmtKind::LetElse {
            value, else_block, ..
        } => {
            let value_usage = crate::qtt_walker::walk_expr(tracked, value);
            let merged = std::mem::take(usage).merge_sequential(value_usage);
            *usage = merged;
            // else_block is a Block; walk its statements recursively.
            for s in else_block.stmts.iter() {
                walk_stmt_for_qtt_usage(tracked, s, usage);
            }
            if let verum_common::Maybe::Some(tail) = &else_block.expr {
                let tail_usage = crate::qtt_walker::walk_expr(tracked, tail);
                let merged = std::mem::take(usage).merge_sequential(tail_usage);
                *usage = merged;
            }
        }
        StmtKind::Expr { expr, .. } => {
            let d = crate::qtt_walker::walk_expr(tracked, expr);
            let merged = std::mem::take(usage).merge_sequential(d);
            *usage = merged;
        }
        StmtKind::Defer(e) | StmtKind::Errdefer(e) => {
            let d = crate::qtt_walker::walk_expr(tracked, e);
            let merged = std::mem::take(usage).merge_sequential(d);
            *usage = merged;
        }
        // Other statement kinds (Item declarations, etc.) don't
        // produce variable references at this scope.
        _ => {}
    }
}

// ==================== Stack Safety Checks ====================
// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow

impl TypeChecker {
    /// Calculate the size of a type in bytes for stack allocation checking.
    ///

    /// Returns None if the size cannot be determined at compile time
    /// (e.g., for dynamically-sized types or circular types).
    pub fn calculate_type_size(&self, ty: &Type) -> Option<u64> {
        // Use depth-tracked version to prevent stack overflow on circular types
        self.calculate_type_size_impl(ty, &mut HashSet::new())
    }

    /// Internal implementation with cycle detection via visited set.
    /// Prevents stack overflow from circular struct types (A -> B -> C -> A).
    fn calculate_type_size_impl(&self, ty: &Type, visited: &mut HashSet<String>) -> Option<u64> {
        match ty {
            // Primitive types
            Type::Int => Some(SIZE_OF_INT),
            Type::Float => Some(SIZE_OF_FLOAT),
            Type::Bool => Some(SIZE_OF_BOOL),
            Type::Char => Some(SIZE_OF_CHAR),
            Type::Unit => Some(0),
            Type::Never => Some(0),
            Type::Text => Some(SIZE_OF_POINTER * 3), // ptr + len + cap

            // References and pointers are pointer-sized
            Type::Reference { .. }
            | Type::CheckedReference { .. }
            | Type::UnsafeReference { .. }
            | Type::Pointer { .. }
            | Type::VolatilePointer { .. } => Some(SIZE_OF_POINTER),

            // Array with known size: element_size * count
            Type::Array {
                element,
                size: Some(count),
            } => {
                let elem_size = self.calculate_type_size_impl(element, visited)?;
                Some(elem_size * (*count as u64))
            }

            // Array without known size - dynamic, can't determine
            Type::Array { size: None, .. } => None,

            // Slice is fat pointer (ptr + len)
            Type::Slice { .. } => Some(SIZE_OF_POINTER * 2),

            // Tuple: sum of all element sizes (simplified, ignoring alignment)
            Type::Tuple(elements) => {
                let mut total = 0u64;
                for elem in elements.iter() {
                    total += self.calculate_type_size_impl(elem, visited)?;
                }
                Some(total)
            }

            // Named types - look up struct fields
            Type::Named { path, args } => {
                let type_name = self.path_to_string(path);

                // CYCLE GUARD: Detect circular struct types (A -> B -> C -> A).
                // If we're already computing the size of this type, we have an
                // infinite-size type. Return None (unknown size) to prevent stack overflow.
                if !visited.insert(type_name.to_string()) {
                    return None; // Circular type detected - size is infinite/unknown
                }

                let struct_key = format!("__struct_fields_{}", type_name);

                let result =
                    if let Maybe::Some(Type::Record(fields)) = self.ctx.lookup_type(&struct_key) {
                        let mut total = 0u64;
                        for field_ty in fields.values() {
                            // Substitute type parameters if present
                            let resolved_ty = if !args.is_empty() {
                                // For simplicity, use the field type as-is for size calculation
                                // A full implementation would substitute type params
                                field_ty.clone()
                            } else {
                                field_ty.clone()
                            };
                            match self.calculate_type_size_impl(&resolved_ty, visited) {
                                Some(size) => total += size,
                                None => {
                                    visited.remove(type_name.as_str());
                                    return None;
                                }
                            }
                        }
                        Some(total)
                    } else {
                        // Assume pointer-sized for unknown named types (conservative)
                        Some(SIZE_OF_POINTER)
                    };

                visited.remove(type_name.as_str());
                result
            }

            // Record types: sum of field sizes
            Type::Record(fields) => {
                let mut total = 0u64;
                for field_ty in fields.values() {
                    total += self.calculate_type_size_impl(field_ty, visited)?;
                }
                Some(total)
            }

            // Generic types - unknown size without resolving the full type definition
            // This is stdlib-agnostic: no hardcoded type names
            Type::Generic { .. } => None,

            // Function pointers
            Type::Function { .. } => Some(SIZE_OF_POINTER),

            // Type variables - can't determine size
            Type::Var(_) => None,

            // Variants - size of largest variant
            Type::Variant(variants) => {
                let mut max_size = 0u64;
                for variant_ty in variants.values() {
                    if let Some(size) = self.calculate_type_size_impl(variant_ty, visited) {
                        max_size = max_size.max(size);
                    }
                }
                // Add discriminant size
                Some(max_size + SIZE_OF_INT)
            }

            // Other types - conservatively assume unknown
            _ => None,
        }
    }

    /// Check if a stack allocation exceeds the safe limit.
    ///

    /// Returns an error if the type's size exceeds MAX_STACK_ALLOCATION_BYTES.
    /// Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
    pub fn check_stack_allocation_size(&self, ty: &Type, span: Span) -> Result<()> {
        if let Some(size) = self.calculate_type_size(ty) {
            if size > MAX_STACK_ALLOCATION_BYTES {
                return Err(TypeError::StackAllocationExceedsLimit {
                    size,
                    limit: MAX_STACK_ALLOCATION_BYTES,
                    span,
                });
            }
        }
        Ok(())
    }
}

/// Create a Maybe<T> type for use in return types.
fn make_maybe_type(inner: Type) -> Type {
    use smallvec::smallvec;
    use verum_ast::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    let ident = Ident::new("Maybe", Span::dummy());
    Type::Named {
        path: Path {
            segments: smallvec![PathSegment::Name(ident)],
            span: Span::dummy(),
        },
        args: List::from(vec![inner]),
    }
}

/// Resolve built-in methods on primitive types (Int, Float, Bool, Char, Byte).
/// These are language built-in types with inherent methods, not stdlib types.
/// HARDCODED FALLBACK for primitive type method return types.
///

/// This function maps (primitive_type, method_name, arg_count) -> return_type for
/// Int, Float, Bool, Char, and Byte methods. It serves as a safety net when the
/// stdlib .vr implement blocks are not loaded into inherent_methods.
///

/// In normal compilation (stdlib loaded via pipeline Pass 5), all these methods
/// should be resolved from inherent_methods BEFORE reaching this fallback.
/// The checked/saturating/wrapping arithmetic methods intentionally return None
/// here to force resolution through stdlib (for correct unsigned type handling).
///

/// HARDCODE(#7): Once confirmed that inherent_methods always has these
/// signatures, this function can be removed entirely.
fn resolve_primitive_method(recv_ty: &Type, method: &str, arg_count: usize) -> Option<Type> {
    // Peel references to get the underlying type
    let base_ty = match recv_ty {
        Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::UnsafeReference { inner, .. } => inner.as_ref(),
        _ => recv_ty,
    };

    // Classify the primitive type
    let prim = match base_ty {
        Type::Int => "int",
        Type::Float => "float",
        Type::Bool => "bool",
        Type::Char => "char",
        Type::Named { path, .. } => {
            let id = path.as_ident()?;
            let tn = id.name.as_str();
            match tn {
                _ if verum_common::well_known_types::type_names::is_integer_type(tn)
                    && tn != "Byte" =>
                {
                    "int"
                }
                _ if verum_common::well_known_types::type_names::is_float_type(tn) => "float",
                "Char" => "char",
                "Byte" => "byte",
                "Bool" => "bool",
                _ => return None,
            }
        }
        _ => return None,
    };

    match prim {
        "int" => match (method, arg_count) {
            ("abs", 0) | ("signum", 0) => Some(Type::Int),
            ("is_positive", 0) | ("is_negative", 0) | ("is_zero", 0) => Some(Type::Bool),
            ("min", 1) | ("max", 1) | ("clamp", 2) | ("pow", 1) => Some(Type::Int),
            // CRITICAL: Do NOT handle checked/saturating/wrapping arithmetic here!
            // These must fall through to inherent method lookup so that UInt64.checked_add
            // resolves to the correct unsigned intrinsic (checked_add_u64) instead of
            // using signed Int arithmetic. The stdlib defines type-specific methods.
            ("checked_add", 1) | ("checked_sub", 1) | ("checked_mul", 1) | ("checked_div", 1) => {
                None
            }
            ("saturating_add", 1) | ("saturating_sub", 1) => None,
            ("wrapping_add", 1) | ("wrapping_sub", 1) => None,
            ("to_float", 0) | ("to_f64", 0) => Some(Type::Float),
            ("to_int", 0) => Some(Type::Int),
            ("count_ones", 0) | ("count_zeros", 0) => Some(Type::Int),
            ("leading_zeros", 0) | ("trailing_zeros", 0) => Some(Type::Int),
            ("reverse_bits", 0) | ("swap_bytes", 0) => Some(Type::Int),
            ("rotate_left", 1) | ("rotate_right", 1) => Some(Type::Int),
            ("in_range", 2) => Some(Type::Bool),
            // CBGR epoch_caps bit inspection methods (packed capability integer)
            ("can_read", 0) | ("can_write", 0) | ("can_extend", 0) | ("is_unique", 0) => {
                Some(Type::Bool)
            }
            ("epoch", 0) | ("raw", 0) => Some(Type::Int), // Extract epoch / identity for capabilities
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            ("to_hex_string", 0) | ("to_binary_string", 0) | ("to_octal_string", 0) => {
                Some(Type::Text)
            }
            ("max_value", 0) | ("min_value", 0) | ("MIN", 0) | ("MAX", 0) | ("BITS", 0) => {
                Some(Type::Int)
            }
            // NOTE: to_le_bytes/to_be_bytes/from_le_bytes/from_be_bytes must fall through
            // to proper method resolution so type-specific byte sizes are used.
            // Int uses 8 bytes, Int32 uses 4 bytes, Int16 uses 2 bytes, etc.
            _ => None,
        },
        "float" => match (method, arg_count) {
            ("abs", 0)
            | ("ceil", 0)
            | ("floor", 0)
            | ("round", 0)
            | ("trunc", 0)
            | ("fract", 0) => Some(Type::Float),
            ("sqrt", 0) | ("sin", 0) | ("cos", 0) | ("tan", 0) | ("ln", 0) | ("signum", 0) => {
                Some(Type::Float)
            }
            ("log2", 0) | ("log10", 0) | ("exp", 0) | ("exp2", 0) | ("cbrt", 0) => {
                Some(Type::Float)
            }
            ("asin", 0) | ("acos", 0) | ("atan", 0) => Some(Type::Float),
            ("atan2", 1) | ("log", 1) => Some(Type::Float),
            ("is_nan", 0) | ("is_infinite", 0) | ("is_finite", 0) => Some(Type::Bool),
            ("is_positive", 0) | ("is_negative", 0) | ("is_zero", 0) => Some(Type::Bool),
            ("to_int", 0) | ("to_i64", 0) => Some(Type::Int),
            ("to_degrees", 0) | ("to_radians", 0) => Some(Type::Float),
            ("min", 1) | ("max", 1) | ("clamp", 2) => Some(Type::Float),
            ("pow", 1) | ("powi", 1) | ("hypot", 1) => Some(Type::Float),
            ("pi", 0) | ("e", 0) | ("epsilon", 0) => Some(Type::Float),
            ("infinity", 0) | ("neg_infinity", 0) | ("nan", 0) => Some(Type::Float),
            ("max_value", 0) | ("min_value", 0) => Some(Type::Float),
            ("MIN", 0)
            | ("MAX", 0)
            | ("EPSILON", 0)
            | ("INFINITY", 0)
            | ("NEG_INFINITY", 0)
            | ("NAN", 0) => Some(Type::Float),
            ("BITS", 0) | ("MIN_POSITIVE", 0) => Some(Type::Int),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "bool" => match (method, arg_count) {
            ("and_then", 1) | ("or_else", 1) => Some(Type::Bool),
            // NOTE: select<T> is a generic method - must fall through to proper method resolution
            // so the type variable T is correctly inferred from arguments
            ("xor", 1) => Some(Type::Bool),
            ("to_int", 0) => Some(Type::Int),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "char" => match (method, arg_count) {
            ("is_alphabetic", 0) | ("is_alphanumeric", 0) | ("is_numeric", 0) => Some(Type::Bool),
            ("is_uppercase", 0) | ("is_lowercase", 0) | ("is_whitespace", 0) => Some(Type::Bool),
            ("is_ascii", 0) | ("is_ascii_alphabetic", 0) | ("is_ascii_alphanumeric", 0) => {
                Some(Type::Bool)
            }
            ("is_ascii_digit", 0) | ("is_ascii_whitespace", 0) => Some(Type::Bool),
            ("to_uppercase", 0) | ("to_lowercase", 0) => Some(Type::Char),
            ("to_ascii_uppercase", 0) | ("to_ascii_lowercase", 0) => Some(Type::Char),
            ("to_digit", 1) => Some(make_maybe_type(Type::Int)),
            ("from_digit", 1) | ("from_digit", 2) => Some(make_maybe_type(Type::Char)),
            ("len_utf8", 0) | ("len_utf16", 0) => Some(Type::Int),
            ("is_control", 0) | ("is_digit", 1) => Some(Type::Bool),
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        "byte" => match (method, arg_count) {
            ("to_int", 0) => Some(Type::Int),
            ("is_ascii", 0) | ("is_ascii_alphabetic", 0) | ("is_ascii_digit", 0) => {
                Some(Type::Bool)
            }
            ("min_value", 0) | ("max_value", 0) | ("MIN", 0) | ("MAX", 0) | ("BITS", 0) => {
                Some(Type::Int)
            }
            ("to_text", 0) | ("to_string", 0) => Some(Type::Text),
            _ => None,
        },
        _ => None,
    }
}

// Tests moved to tests/infer_tests.rs

// ---------------------------------------------------------------------------
// Mount-cycle-detection regression tests (SIGBUS fix, 2026-04-24).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod qtt_v2_enforcement_tests {
    //! QTT V2 enforcement pass tests. Validates the
    //! integration: `@quantity(0|1|omega)` attribute on a parameter
    //! produces a `Quantity` declaration that drives `qtt_walker`-
    //! based usage counting + `qtt_usage::check_usage` validation.
    use super::extract_quantity_from_attrs;
    use verum_ast::Ident;
    use verum_ast::attr::{Attribute, Quantity as AstQty, QuantityAttr};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::span::Span;
    use verum_common::{List, Maybe, Text};

    fn span() -> Span {
        Span::default()
    }

    fn quantity_attr(q: AstQty) -> Attribute {
        let raw = QuantityAttr::new(q, span());
        // Surface form: @quantity(<glyph>) — encoded as Path arg.
        let mut segs: List<verum_ast::ty::PathSegment> = List::new();
        segs.push(verum_ast::ty::PathSegment::Name(Ident {
            name: Text::from(raw.quantity.surface_glyph()),
            span: span(),
        }));
        let path = verum_ast::ty::Path::new(segs, span());
        let mut args: List<Expr> = List::new();
        args.push(Expr::new(ExprKind::Path(path), span()));
        Attribute {
            name: Text::from("quantity"),
            args: Maybe::Some(args),
            span: span(),
        }
    }

    fn attr_list(qs: Vec<AstQty>) -> List<Attribute> {
        let mut l: List<Attribute> = List::new();
        for q in qs {
            l.push(quantity_attr(q));
        }
        l
    }

    #[test]
    fn empty_attrs_default_to_omega() {
        let attrs: List<Attribute> = List::new();
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Omega,
        );
    }

    #[test]
    fn quantity_zero_attr_extracts_zero() {
        let attrs = attr_list(vec![AstQty::Zero]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Zero,
        );
    }

    #[test]
    fn quantity_one_attr_extracts_linear() {
        let attrs = attr_list(vec![AstQty::One]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::One,
        );
    }

    #[test]
    fn quantity_many_attr_extracts_omega() {
        let attrs = attr_list(vec![AstQty::Many]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Omega,
        );
    }

    #[test]
    fn first_quantity_attr_wins_over_extras() {
        // Multiple @quantity attributes on the same param: the
        // first one wins (deterministic ordering, no collision
        // diagnostic — the parser tolerates duplicates because
        // they're discoverable via the AST round-trip).
        let attrs = attr_list(vec![AstQty::One, AstQty::Zero]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::One,
        );
    }

    #[test]
    fn unrelated_attr_does_not_affect_extraction() {
        let mut l: List<Attribute> = List::new();
        l.push(Attribute {
            name: Text::from("inline"),
            args: Maybe::None,
            span: span(),
        });
        l.push(quantity_attr(AstQty::One));
        assert_eq!(extract_quantity_from_attrs(&l), crate::ty::Quantity::One,);
    }
}

#[cfg(test)]
mod mount_cycle_tests {
    //! Regression: when a stdlib module's glob expansion re-enters itself via
    //! `public mount` re-exports the interpreter used to SIGBUS with ~900k
    //! `__mh_execute_header` frames. The compiler now guards every glob-
    //! expansion entry point with a `HashSet<Text>` visited-set and emits
    //! `TypeError::ImportCycle` (E0811) when the set re-enters.

    use super::TypeChecker;
    use crate::TypeError;
    use verum_ast::decl::{ModuleDecl, MountDecl, MountTree, MountTreeKind, Visibility};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{List, Maybe, Text};

    fn mount_glob_decl(path_str: &str) -> MountDecl {
        let span = Span::dummy();
        let segments: List<PathSegment> = path_str
            .split('.')
            .map(|seg| PathSegment::Name(Ident::new(seg, span)))
            .collect();
        MountDecl {
            visibility: Visibility::Private,
            tree: MountTree {
                kind: MountTreeKind::Glob(Path::new(segments, span)),
                alias: Maybe::None,
                span,
            },
            alias: Maybe::None,
            span,
        }
    }

    fn make_module(name: &str) -> ModuleDecl {
        let span = Span::dummy();
        ModuleDecl {
            name: Ident::new(name, span),
            visibility: Visibility::Public,
            items: Maybe::Some(List::new()),
            profile: Maybe::None,
            features: Maybe::None,
            contexts: List::new(),
            span,
        }
    }

    /// Direct test: calling `import_all_from_inline_module` for a module
    /// whose path is already on the glob-in-progress stack must return
    /// `TypeError::ImportCycle`, not stack-overflow.
    #[test]
    fn inline_module_cycle_returns_import_cycle_error() {
        let mut checker = TypeChecker::new();
        let key: Text = "cog.loopy".into();

        // Register an empty inline module so the code path doesn't bail early
        // with "module not found".
        checker
            .inline_modules
            .insert(key.clone(), make_module("loopy"));

        // Seed the glob-in-progress set to simulate being mid-expansion of
        // this module (as would happen if the caller is one stack frame up).
        checker.glob_imports_in_progress.insert(key.clone());
        checker.glob_imports_stack.push(key.clone());

        // Recursively entering the same module must produce E0811, not SIGBUS.
        let err = checker
            .import_all_from_inline_module(key.as_str())
            .expect_err("expected ImportCycle error on re-entry");

        match err {
            TypeError::ImportCycle {
                cycle_path,
                modules_in_cycle,
                ..
            } => {
                assert!(
                    cycle_path.as_str().contains("loopy"),
                    "cycle_path should mention the looping module, got: {}",
                    cycle_path
                );
                assert!(
                    modules_in_cycle.iter().any(|m| m.as_str() == "cog.loopy"),
                    "modules_in_cycle should include cog.loopy, got: {:?}",
                    modules_in_cycle
                );
            }
            other => panic!("expected ImportCycle, got: {:?}", other),
        }
    }

    /// Direct test: `import_all_from_module` (registry-backed path) is
    /// symmetrically guarded.
    #[test]
    fn registry_module_cycle_returns_import_cycle_error() {
        let mut checker = TypeChecker::new();
        let key: Text = "core.loopy".into();

        // Simulate being mid-expansion.
        checker.glob_imports_in_progress.insert(key.clone());
        checker.glob_imports_stack.push(key.clone());

        let registry = verum_modules::ModuleRegistry::new();
        let err = checker
            .import_all_from_module(&key, &registry)
            .expect_err("expected ImportCycle error on re-entry");

        assert!(matches!(err, TypeError::ImportCycle { .. }));
    }

    /// Positive control: a fresh checker (no in-progress cycle) must NOT
    /// produce ImportCycle — the guard triggers only on actual re-entry.
    #[test]
    fn non_cyclic_inline_mount_does_not_trigger_guard() {
        let mut checker = TypeChecker::new();
        let key: Text = "cog.fine".into();
        checker
            .inline_modules
            .insert(key.clone(), make_module("fine"));

        // No seeding — this is a clean call.
        let result = checker.import_all_from_inline_module(key.as_str());
        assert!(
            result.is_ok(),
            "clean inline-module glob should not be flagged as a cycle, got {:?}",
            result
        );

        // After the call the guard must have cleaned up after itself.
        assert!(
            !checker.glob_imports_in_progress.contains(&key),
            "glob_imports_in_progress must drop key on exit"
        );
        assert!(
            checker.glob_imports_stack.is_empty(),
            "glob_imports_stack must be empty after clean exit"
        );
    }

    /// Compile-time regression: ensure the MountDecl helper builds a glob
    /// that actually lowers to MountTreeKind::Glob (guards against silent
    /// grammar drift inside the test harness).
    #[test]
    fn mount_glob_decl_helper_produces_glob_kind() {
        let decl = mount_glob_decl("core.action");
        assert!(matches!(decl.tree.kind, MountTreeKind::Glob(_)));
    }

    /// Regression: `find_type_declaration_with_source_module` used to recurse
    /// indefinitely when a module re-exported a sibling whose last segment
    /// matched the target type name, e.g.
    ///

    /// ```ignore
    /// // core/tmp_repro/mod.vr (module path "core.tmp_repro")
    /// public mount core.tmp_repro.sub;
    /// ```
    ///

    /// Looking up type `sub` in module `core.tmp_repro` would match the mount,
    /// strip the last segment back to `core.tmp_repro`, and re-enter the same
    /// AST — SIGBUSing after ~32k recursive frames in release builds.
    ///

    /// The fix threads a visited-set through
    /// `find_type_declaration_with_source_module_inner`; re-entry now returns
    /// `None` instead of blowing the stack.
    #[test]
    fn self_referential_mount_terminates_with_none() {
        use verum_ast::decl::{Item, ItemKind, MountDecl, MountTree, MountTreeKind, Visibility};
        use verum_common::FileId;

        let checker = TypeChecker::new();
        let span = Span::dummy();

        // Build MountDecl equivalent to `public mount core.tmp_repro.sub;`
        // (a Path mount, not a Glob, so it hits the
        // `find_type_declaration_with_source_module` re-export code path).
        let segments: List<PathSegment> = ["core", "tmp_repro", "sub"]
            .iter()
            .map(|seg| PathSegment::Name(Ident::new(*seg, span)))
            .collect();
        let mount_item = Item::new(
            ItemKind::Mount(MountDecl {
                visibility: Visibility::Public,
                tree: MountTree {
                    kind: MountTreeKind::Path(Path::new(segments, span)),
                    alias: Maybe::None,
                    span,
                },
                alias: Maybe::None,
                span,
            }),
            span,
        );

        let items: List<Item> = List::from(vec![mount_item]);
        let ast = verum_ast::Module::new(items, FileId::new(0), span);

        let registry = verum_modules::ModuleRegistry::new();
        // The key property: this call MUST return (rather than blow the
        // stack). The answer itself is `None` — `sub` is not actually
        // resolvable through the self-referential mount — and that is the
        // correct fallback signal for upstream callers.
        let result = checker.find_type_declaration_with_source_module(
            &ast,
            "sub",
            &Text::from("core.tmp_repro"),
            &registry,
        );
        assert!(
            result.is_none(),
            "self-referential mount should resolve to None (was: {:?})",
            result
        );
    }

    // ============================================================
    // [protocols].higher_kinded_protocols wire-up pins (task #264).
    // ============================================================

    #[test]
    fn hkt_protocols_default_is_disabled() {
        // Pin: documented Verum.toml default — HKT-bearing protocol
        // declarations are rejected unless the user explicitly opts
        // in via `[protocols].higher_kinded_protocols = true`.
        let checker = TypeChecker::new();
        assert!(
            !checker.higher_kinded_protocols_enabled(),
            "default must be false"
        );
    }

    #[test]
    fn hkt_protocols_setter_round_trips() {
        let mut checker = TypeChecker::new();
        checker.set_higher_kinded_protocols_enabled(true);
        assert!(checker.higher_kinded_protocols_enabled());
        checker.set_higher_kinded_protocols_enabled(false);
        assert!(!checker.higher_kinded_protocols_enabled());
        // Idempotent.
        checker.set_higher_kinded_protocols_enabled(false);
        assert!(!checker.higher_kinded_protocols_enabled());
    }

    #[test]
    fn hkt_protocols_disabled_rejects_higher_kinded_param() {
        // Pin: when [protocols].higher_kinded_protocols is false, a
        // protocol declaring an HKT generic parameter is rejected at
        // registration time with TypeError::Other citing the manifest.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false → reject.
        assert!(!checker.higher_kinded_protocols_enabled());

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Functor", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::HigherKinded {
                    name: Ident::new("F", Span::default()),
                    arity: 1,
                    bounds: verum_common::List::new(),
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        match result {
            Err(TypeError::Other(msg)) => {
                assert!(
                    msg.as_str().contains("higher_kinded_protocols"),
                    "rejection must cite the manifest field; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Functor"),
                    "rejection must name the protocol; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("F<"),
                    "rejection must show the HKT param syntax; got: {}",
                    msg
                );
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn hkt_protocols_enabled_accepts_higher_kinded_param() {
        // Pin: with [protocols].higher_kinded_protocols = true (and
        // [types].higher_kinded already implicit at the manifest
        // validation layer), HKT-bearing protocol declarations
        // register successfully.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        checker.set_higher_kinded_protocols_enabled(true);

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Functor", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::HigherKinded {
                    name: Ident::new("F", Span::default()),
                    arity: 1,
                    bounds: verum_common::List::new(),
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "with hkt protocols enabled, registration must succeed; got {:?}",
            result
        );
    }

    #[test]
    fn hkt_protocols_disabled_accepts_regular_protocol() {
        // Pin: the gate ONLY rejects HigherKinded params. Regular
        // type params (`protocol Eq<T>`) register fine even when
        // the HKT flag is false. No false positives.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false.
        assert!(!checker.higher_kinded_protocols_enabled());

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Eq", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::Type {
                    name: Ident::new("T", Span::default()),
                    bounds: verum_common::List::new(),
                    default: VMaybe::None,
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "regular type-param protocol must register even with hkt disabled; got {:?}",
            result
        );
    }

    // ============================================================
    // [protocols].generic_associated_types wire-up pins (task #265).
    // ============================================================

    #[test]
    fn gat_default_is_disabled() {
        let checker = TypeChecker::new();
        assert!(
            !checker.generic_associated_types_enabled(),
            "default must be false"
        );
    }

    #[test]
    fn gat_setter_round_trips() {
        let mut checker = TypeChecker::new();
        checker.set_generic_associated_types_enabled(true);
        assert!(checker.generic_associated_types_enabled());
        checker.set_generic_associated_types_enabled(false);
        assert!(!checker.generic_associated_types_enabled());
    }

    #[test]
    fn gat_disabled_rejects_generic_associated_type() {
        // Pin: when [protocols].generic_associated_types is false,
        // a protocol body containing a `type Item<T>` declaration
        // (non-empty type_params on the associated type) is rejected
        // at registration time with TypeError::Other citing the
        // manifest field.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false — gate active.
        assert!(!checker.generic_associated_types_enabled());

        let gat_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Item", Span::default()),
                type_params: verum_common::List::from(vec![GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new("T", Span::default()),
                        bounds: verum_common::List::new(),
                        default: VMaybe::None,
                    },
                    is_implicit: false,
                    span: Span::default(),
                }]),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Stream", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![gat_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        match result {
            Err(TypeError::Other(msg)) => {
                assert!(
                    msg.as_str().contains("generic_associated_types"),
                    "rejection must cite the manifest field; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Stream"),
                    "rejection must name the protocol; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Item"),
                    "rejection must name the GAT; got: {}",
                    msg
                );
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn gat_enabled_accepts_generic_associated_type() {
        // Pin: with [protocols].generic_associated_types = true,
        // GAT-bearing protocol declarations register successfully.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        checker.set_generic_associated_types_enabled(true);

        let gat_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Item", Span::default()),
                type_params: verum_common::List::from(vec![GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new("T", Span::default()),
                        bounds: verum_common::List::new(),
                        default: VMaybe::None,
                    },
                    is_implicit: false,
                    span: Span::default(),
                }]),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Stream", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![gat_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "with GAT enabled, registration must succeed; got {:?}",
            result
        );
    }

    #[test]
    fn gat_disabled_accepts_regular_associated_type() {
        // Pin: the gate ONLY rejects associated types with non-empty
        // type_params. Regular `type Output;` (zero type_params)
        // registers fine even with the GAT flag off.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::Ident;
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false.
        assert!(!checker.generic_associated_types_enabled());

        let regular_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Output", Span::default()),
                type_params: verum_common::List::new(),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Iterator", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![regular_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "regular zero-param associated type must register even with GAT disabled; got {:?}",
            result
        );
    }

    // ============================================================
    // MLS classification sidecar pin tests (#289 Phase 2b-Foundation).
    // ============================================================

    #[test]
    fn classification_sidecar_default_is_public() {
        // Pin: looking up an unknown binding returns Public — the
        // safe default. Lattice's join() identity element so taint
        // propagation through unclassified contexts is a no-op.
        let checker = TypeChecker::new();
        let level = checker.binding_classification(&Text::from("x"));
        assert_eq!(level, verum_common::mls::MlsLevel::Public);
    }

    #[test]
    fn classification_sidecar_explicit_returns_none_for_unknown() {
        // Pin: distinguishes "not in map" from "explicitly Public"
        // for sink-detection use cases.
        let checker = TypeChecker::new();
        let level = checker.binding_classification_explicit(&Text::from("x"));
        assert!(level.is_none());
    }

    #[test]
    fn classification_sidecar_set_round_trips() {
        // Pin: setter stores the classification; getter retrieves
        // it. Foundation primitive — Phase 2b-Integration uses
        // this pair at parameter-introduction sites.
        let mut checker = TypeChecker::new();
        let var = Text::from("secret_data");
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::Secret);
        assert_eq!(
            checker.binding_classification(&var),
            verum_common::mls::MlsLevel::Secret
        );
        assert_eq!(
            checker.binding_classification_explicit(&var),
            Some(verum_common::mls::MlsLevel::Secret)
        );
    }

    #[test]
    fn classification_sidecar_overwrite_uses_latest() {
        // Pin: re-setting overwrites — useful for shadowing scopes
        // where a binding is rebound at higher / lower
        // classification (Phase 2b-Full handles scoping; the
        // sidecar primitive is the underlying storage).
        let mut checker = TypeChecker::new();
        let var = Text::from("v");
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::Public);
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::TopSecret);
        assert_eq!(
            checker.binding_classification(&var),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    #[test]
    fn classification_sidecar_drain_clears_map() {
        // Pin: drain returns the full map and empties the
        // checker's storage. Used by audit reports + scope-exit
        // cleanup.
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(Text::from("a"), verum_common::mls::MlsLevel::Secret);
        checker.set_binding_classification(Text::from("b"), verum_common::mls::MlsLevel::TopSecret);
        let drained = checker.drain_classification_map();
        assert_eq!(drained.len(), 2);
        // After drain, lookups return Public again.
        assert_eq!(
            checker.binding_classification(&Text::from("a")),
            verum_common::mls::MlsLevel::Public
        );
        assert!(
            checker
                .binding_classification_explicit(&Text::from("b"))
                .is_none()
        );
    }

    #[test]
    fn classification_sidecar_uses_lattice_join_when_combining() {
        // Pin: callers use the lattice's `join` to combine
        // classifications across multiple sources — this test
        // verifies the sidecar interoperates with the lattice
        // primitive from #282 Phase 2a.
        let mut checker = TypeChecker::new();
        checker
            .set_binding_classification(Text::from("source"), verum_common::mls::MlsLevel::Secret);
        let other = verum_common::mls::MlsLevel::TopSecret;
        let combined = checker
            .binding_classification(&Text::from("source"))
            .join(other);
        assert_eq!(combined, verum_common::mls::MlsLevel::TopSecret);
    }

    // ============================================================
    // MLS Phase 2b-Integration pin tests (#291) — sidecar seeding
    // from parameter @classification attributes at function-
    // signature registration time.
    // ============================================================

    /// Build a `@classification(<level>)` attribute for tests.
    fn mk_classification_attr_2b(level: &str) -> verum_ast::attr::Attribute {
        use verum_ast::expr::{Expr, ExprKind};
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(level, Span::default()));
        let arg = Expr::new(ExprKind::Path(path), Span::default());
        let mut args = List::new();
        args.push(arg);
        verum_ast::attr::Attribute::new(
            Text::from("classification"),
            Maybe::Some(args),
            Span::default(),
        )
    }

    /// Build a Regular FunctionParam with a single Ident pattern
    /// and an optional `@classification` attribute.
    fn mk_param(name: &str, classification: Option<&str>) -> verum_ast::decl::FunctionParam {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::pattern::{Pattern, PatternKind};
        let mut attrs = List::new();
        if let Some(level) = classification {
            attrs.push(mk_classification_attr_2b(level));
        }
        verum_ast::decl::FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: verum_ast::ty::Ident::new(name, Span::default()),
                        subpattern: Maybe::None,
                    },
                    span: Span::default(),
                },
                ty: verum_ast::ty::Type {
                    kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(
                        verum_ast::ty::Ident::new("Int", Span::default()),
                    )),
                    span: Span::default(),
                },
                default_value: Maybe::None,
            },
            attributes: attrs,
            span: Span::default(),
        }
    }

    /// Build a FunctionDecl with the given parameters for sidecar
    /// seeding tests.
    fn mk_function_decl_2b(
        params: List<verum_ast::decl::FunctionParam>,
    ) -> verum_ast::FunctionDecl {
        verum_ast::FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("test_fn", Span::default()),
            generics: List::new(),
            params,
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: None,
            attributes: List::new(),
            is_async: false,
            is_meta: false,
            is_unsafe: false,
            span: Span::default(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            std_attr: Maybe::None,
            contexts: List::new(),
        }
    }

    #[test]
    fn read_param_classification_returns_public_for_no_attr() {
        // Pin: helper returns Public when no @classification is
        // present — matches the safe-default semantic.
        let attrs: List<verum_ast::attr::Attribute> = List::new();
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::Public);
    }

    #[test]
    fn read_param_classification_extracts_secret() {
        let mut attrs = List::new();
        attrs.push(mk_classification_attr_2b("secret"));
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::Secret);
    }

    #[test]
    fn read_param_classification_takes_max_when_multiple() {
        // Pin: multiple @classification attributes take the highest
        // (lattice join). Pathological but legal AST.
        let mut attrs = List::new();
        attrs.push(mk_classification_attr_2b("secret"));
        attrs.push(mk_classification_attr_2b("top_secret"));
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::TopSecret);
    }

    #[test]
    fn register_function_signature_seeds_sidecar_for_classified_param() {
        // Pin: after register_function_signature, the sidecar
        // contains an entry for each Regular Ident parameter whose
        // attributes carry a non-Public classification.
        let mut params = List::new();
        params.push(mk_param("data", Some("secret")));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        assert_eq!(
            checker.binding_classification(&Text::from("data")),
            verum_common::mls::MlsLevel::Secret,
            "register_function_signature must seed sidecar for classified params"
        );
    }

    #[test]
    fn register_function_signature_does_not_seed_unclassified_params() {
        // Pin: parameters without @classification do NOT seed the
        // sidecar — keeps the map sparse (only classified
        // bindings are tracked).
        let mut params = List::new();
        params.push(mk_param("plain", None));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        // Unclassified binding returns Public via the default path
        // but should NOT have an explicit entry.
        assert!(
            checker
                .binding_classification_explicit(&Text::from("plain"))
                .is_none(),
            "unclassified params must not produce a sidecar entry"
        );
    }

    #[test]
    fn register_function_signature_seeds_multiple_classified_params() {
        // Pin: every classified parameter gets its own sidecar
        // entry. Multi-parameter functions track each binding
        // independently.
        let mut params = List::new();
        params.push(mk_param("low", None));
        params.push(mk_param("med", Some("secret")));
        params.push(mk_param("high", Some("top_secret")));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        assert!(
            checker
                .binding_classification_explicit(&Text::from("low"))
                .is_none()
        );
        assert_eq!(
            checker.binding_classification(&Text::from("med")),
            verum_common::mls::MlsLevel::Secret
        );
        assert_eq!(
            checker.binding_classification(&Text::from("high")),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    // ============================================================
    // MLS Phase 2b-Followup pin tests (#292) — expression
    // classification + let-binding propagation.
    // ============================================================

    fn mk_path_expr(name: &str) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(name, Span::default()));
        Expr::new(ExprKind::Path(path), Span::default())
    }

    fn mk_int_lit(n: i64) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{IntLit, Literal, LiteralKind};
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit {
                    value: n as i128,
                    suffix: Maybe::None,
                }),
                Span::default(),
            )),
            Span::default(),
        )
    }

    #[test]
    fn expr_classification_path_resolves_classified_binding() {
        // Pin: a Path expression referring to a classified binding
        // returns that binding's classification — the load-bearing
        // read site for let-binding propagation.
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_data"),
            verum_common::mls::MlsLevel::Secret,
        );
        let expr = mk_path_expr("secret_data");
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Secret
        );
    }

    #[test]
    fn expr_classification_path_unknown_returns_public() {
        // Pin: unknown Path expressions return Public (sparse-by-
        // design). No false positives from typos.
        let checker = TypeChecker::new();
        let expr = mk_path_expr("nonexistent");
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Public
        );
    }

    #[test]
    fn expr_classification_literal_returns_public() {
        // Pin: literal expressions are unclassified. Constants are
        // not derived from any classified source.
        let checker = TypeChecker::new();
        let expr = mk_int_lit(42);
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Public
        );
    }

    #[test]
    fn expr_classification_binary_joins_operand_classifications() {
        // Pin: `a + b` where a is Secret and b is Public produces
        // Secret. Lattice JOIN semantics — both operands taint the
        // result.
        use verum_ast::expr::{BinOp, Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(Text::from("a"), verum_common::mls::MlsLevel::Secret);
        let left = mk_path_expr("a");
        let right = mk_int_lit(5);
        let binop = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&binop),
            verum_common::mls::MlsLevel::Secret
        );
    }

    #[test]
    fn expr_classification_binary_max_when_both_classified() {
        // Pin: when both operands are classified at different
        // levels, the lattice JOIN produces the maximum.
        use verum_ast::expr::{BinOp, Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_v"),
            verum_common::mls::MlsLevel::Secret,
        );
        checker
            .set_binding_classification(Text::from("ts_v"), verum_common::mls::MlsLevel::TopSecret);
        let left = mk_path_expr("secret_v");
        let right = mk_path_expr("ts_v");
        let binop = Expr::new(
            ExprKind::Binary {
                op: BinOp::Mul,
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&binop),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    // ============================================================
    // MLS Phase 2b-Final pin tests (#293) — call-site down-flow
    // helper + parameter classification metadata registration.
    // ============================================================

    #[test]
    fn register_function_signature_stores_param_classifications() {
        // Pin: parameter classification metadata is stored at
        // signature-registration time so call sites can look it up
        // by function name. Sparse map: every function gets an
        // entry (even if all-Public) so the lookup contract is
        // uniform.
        let mut params = List::new();
        params.push(mk_param("low", None));
        params.push(mk_param("med", Some("secret")));
        params.push(mk_param("high", Some("top_secret")));
        let func = mk_function_decl_2b(params);
        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        let levels = checker
            .function_param_classifications(&Text::from("test_fn"))
            .expect("registration must populate param classifications");
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], verum_common::mls::MlsLevel::Public);
        assert_eq!(levels[1], verum_common::mls::MlsLevel::Secret);
        assert_eq!(levels[2], verum_common::mls::MlsLevel::TopSecret);
    }

    #[test]
    fn function_param_classifications_returns_none_for_unknown() {
        let checker = TypeChecker::new();
        assert!(
            checker
                .function_param_classifications(&Text::from("never_registered"))
                .is_none()
        );
    }

    #[test]
    fn check_classification_downflow_accepts_higher_param() {
        // Pin: lattice subsumption — Public arg flowing into
        // Secret param is ACCEPTED (param provides MORE protection
        // than the unclassified data requires).
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            "foo",
            0,
            "x",
        );
        assert!(
            result.is_ok(),
            "arg=Public into param=Secret must accept (over-protection)"
        );
    }

    #[test]
    fn check_classification_downflow_accepts_equal() {
        let checker = TypeChecker::new();
        for level in [
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            verum_common::mls::MlsLevel::TopSecret,
        ] {
            assert!(
                checker
                    .check_classification_downflow(level, level, "f", 0, "p")
                    .is_ok()
            );
        }
    }

    #[test]
    fn check_classification_downflow_rejects_secret_to_public() {
        // Pin: the load-bearing reject — Secret arg into Public
        // param is the leak we're catching.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Secret,
            verum_common::mls::MlsLevel::Public,
            "log_visible",
            0,
            "msg",
        );
        match result {
            Err(TypeError::Other(msg)) => {
                let s = msg.as_str();
                assert!(s.contains("MLS down-flow"), "got: {}", s);
                assert!(s.contains("secret"), "got: {}", s);
                assert!(s.contains("public"), "got: {}", s);
                assert!(s.contains("log_visible"), "got: {}", s);
                assert!(s.contains("@declassify"), "got: {}", s);
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn check_classification_downflow_rejects_top_secret_to_secret() {
        // Pin: TopSecret arg into Secret param is rejected — the
        // param provides only Secret-level protection, but the
        // argument requires TopSecret-level protection. Without
        // this rejection, downstream operations on the param
        // would handle TopSecret data under Secret-grade rules.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::TopSecret,
            verum_common::mls::MlsLevel::Secret,
            "f",
            1,
            "data",
        );
        assert!(
            result.is_err(),
            "TopSecret arg into Secret param must reject (under-protection)"
        );
    }

    // ============================================================
    // MLS Phase 2b-Final-Integration pin tests (#294) — module
    // walker that calls check_classification_downflow at every
    // detected call site.
    // ============================================================

    /// Build a Module with a single function whose body is a
    /// statement-expression call.
    fn mk_module_with_call(
        callee_name: &str,
        callee_param: (&str, Option<&str>),
        caller_arg_path: &str,
        caller_classified_locals: Vec<(&str, &str)>,
    ) -> verum_ast::Module {
        use verum_ast::expr::{Expr, ExprKind};

        // The callee declaration:
        let mut callee_params = List::new();
        callee_params.push(mk_param(callee_param.0, callee_param.1));
        let callee = {
            let mut decl = mk_function_decl_2b(callee_params);
            decl.name = verum_ast::ty::Ident::new(callee_name, Span::default());
            decl
        };

        // The caller body: just one call expression `callee(arg)`.
        let func_path =
            verum_ast::ty::Path::single(verum_ast::ty::Ident::new(callee_name, Span::default()));
        let func_expr = Expr::new(ExprKind::Path(func_path), Span::default());
        let arg_path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            caller_arg_path,
            Span::default(),
        ));
        let arg_expr = Expr::new(ExprKind::Path(arg_path), Span::default());
        let mut args = List::new();
        args.push(arg_expr);
        let call_expr = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(func_expr),
                args,
                type_args: List::new(),
            },
            Span::default(),
        );
        let call_stmt = verum_ast::stmt::Stmt {
            kind: verum_ast::stmt::StmtKind::Expr {
                expr: call_expr,
                has_semi: false,
            },
            attributes: Vec::new(),
            span: Span::default(),
        };
        let mut stmts = List::new();
        stmts.push(call_stmt);
        let body = verum_ast::expr::Block {
            stmts,
            expr: Maybe::None,
            span: Span::default(),
        };

        // The caller declaration with classified locals as
        // parameters.
        let mut caller_params = List::new();
        for (name, level) in caller_classified_locals {
            caller_params.push(mk_param(name, Some(level)));
        }
        let caller = {
            let mut decl = mk_function_decl_2b(caller_params);
            decl.name = verum_ast::ty::Ident::new("caller", Span::default());
            decl.body = Some(verum_ast::decl::FunctionBody::Block(body));
            decl
        };

        let mut items = List::new();
        items.push(verum_ast::decl::Item::new(
            verum_ast::ItemKind::Function(callee),
            Span::default(),
        ));
        items.push(verum_ast::decl::Item::new(
            verum_ast::ItemKind::Function(caller),
            Span::default(),
        ));
        verum_ast::Module {
            items,
            attributes: List::new(),
            file_id: verum_ast::FileId::new(0),
            span: Span::default(),
        }
    }

    #[test]
    fn module_walker_detects_secret_to_public_call_site_leak() {
        // Pin: caller passes a Secret-classified local to a
        // function whose parameter is unclassified (Public). The
        // walker emits one TypeError::Other diagnostic per leak.
        let module = mk_module_with_call(
            "log_visible", // callee
            ("msg", None), // callee param: unclassified
            "secret_data", // caller arg: a name in caller's params
            vec![("secret_data", "secret")],
        );

        let mut checker = TypeChecker::new();
        // Register both functions so their param classifications
        // are visible to the walker.
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(
            errors.len(),
            1,
            "secret arg → public param must produce one error"
        );
        match &errors[0] {
            TypeError::Other(msg) => {
                let s = msg.as_str();
                assert!(s.contains("MLS down-flow"), "got: {}", s);
                assert!(s.contains("log_visible"), "got: {}", s);
                assert!(s.contains("secret"), "got: {}", s);
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn module_walker_accepts_classified_param_chain() {
        // Pin: when caller's classified local flows into a
        // matching-classification parameter, no leak.
        let module = mk_module_with_call(
            "encrypt",
            ("data", Some("secret")), // callee param: secret
            "secret_data",
            vec![("secret_data", "secret")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "secret arg → secret param must accept; got {} errors",
            errors.len()
        );
    }

    #[test]
    fn module_walker_accepts_unclassified_program() {
        // Pin: a program with no classifications anywhere
        // produces zero diagnostics. Phase 2b is dormant in
        // public-floor builds — zero overhead.
        let module = mk_module_with_call("plain_fn", ("arg", None), "x", vec![("x", "public")]);

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "fully-public program must produce no diagnostics"
        );
    }

    #[test]
    fn module_walker_accepts_over_protection() {
        // Pin: passing a public arg to a secret-classified param
        // is fine — parameter provides MORE protection than the
        // unclassified data requires.
        let module = mk_module_with_call(
            "encrypt",
            ("data", Some("secret")),
            "x",
            vec![("x", "public")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "public arg → secret param (over-protection) must accept"
        );
    }

    // ============================================================
    // MLS Phase 2b @declassify escape hatch pin tests (#295).
    //

    // When a function carries `@declassify`, its body is the
    // boundary where classified data is explicitly allowed to
    // flow into lower-classification sinks. The walker skips
    // such functions entirely.
    // ============================================================

    /// Build a `@declassify` attribute (no args needed).
    fn mk_declassify_attr_simple() -> verum_ast::attr::Attribute {
        verum_ast::attr::Attribute::simple(verum_common::Text::from("declassify"), Span::default())
    }

    /// Build a Module with a `@declassify`-marked caller passing a
    /// classified arg into a public param.
    fn mk_module_with_declassify_caller(caller_has_declassify: bool) -> verum_ast::Module {
        let mut module = mk_module_with_call(
            "log_visible", // unclassified callee
            ("msg", None),
            "secret_data",
            vec![("secret_data", "secret")],
        );
        if caller_has_declassify {
            // The second item is the caller — promote its
            // attributes to include @declassify.
            if let verum_ast::ItemKind::Function(ref mut f) = module.items[1].kind {
                f.attributes.push(mk_declassify_attr_simple());
            }
        }
        module
    }

    #[test]
    fn declassify_caller_skips_walker() {
        // Pin: when the caller carries @declassify, the walker
        // skips its body entirely — no leak diagnostic even though
        // a Secret arg flows into a Public param.
        let module = mk_module_with_declassify_caller(true);
        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }
        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "@declassify caller must skip down-flow walker; got {} errors",
            errors.len()
        );
    }

    #[test]
    fn no_declassify_still_fires_walker() {
        // Pin: same module WITHOUT @declassify still fires the
        // leak diagnostic — regression-control for the escape
        // hatch (it's not silently always-on).
        let module = mk_module_with_declassify_caller(false);
        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }
        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(errors.len(), 1, "without @declassify, the leak still fires");
    }

    #[test]
    fn has_declassify_attr_on_function_returns_true_with_attr() {
        let mut params = List::new();
        params.push(mk_param("x", None));
        let mut func = mk_function_decl_2b(params);
        func.attributes.push(mk_declassify_attr_simple());
        assert!(super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn has_declassify_attr_on_function_returns_false_without_attr() {
        let func = mk_function_decl_2b(List::new());
        assert!(!super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn has_declassify_ignores_other_attrs() {
        // Pin: only @declassify produces true. Sibling attributes
        // (@inline, @classification, etc.) don't accidentally
        // trip the escape hatch.
        let mut params = List::new();
        params.push(mk_param("x", None));
        let mut func = mk_function_decl_2b(params);
        func.attributes.push(mk_classification_attr_2b("secret"));
        // @classification but no @declassify → walker still fires.
        assert!(!super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn module_walker_detects_top_secret_to_secret_underflow() {
        // Pin: TopSecret arg into Secret param is rejected — the
        // parameter provides only Secret-grade protection.
        let module = mk_module_with_call(
            "secret_only_handler",
            ("data", Some("secret")),
            "ts_data",
            vec![("ts_data", "top_secret")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(
            errors.len(),
            1,
            "top_secret → secret must reject (under-protection)"
        );
    }

    #[test]
    fn check_classification_downflow_accepts_public_to_secret() {
        // Pin: Public arg flowing into Secret param is ACCEPTED.
        // The parameter provides MORE protection than the
        // unclassified argument requires — over-protection is
        // fine, only under-protection is a leak.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            "f",
            0,
            "p",
        );
        assert!(
            result.is_ok(),
            "Public arg into Secret param must accept (over-protection)"
        );
    }

    #[test]
    fn expr_classification_call_propagates_through_args() {
        // Pin: function calls propagate classification from
        // arguments to result. `foo(secret_arg)` taints the result
        // at Secret. The function's own classification is the
        // join with arg classifications — Phase 2b-Final will
        // refine this with parameter-classification matching.
        use verum_ast::expr::{Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_arg"),
            verum_common::mls::MlsLevel::Secret,
        );
        let func = mk_path_expr("foo");
        let mut args = List::new();
        args.push(mk_path_expr("secret_arg"));
        let call = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(func),
                args,
                type_args: List::new(),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&call),
            verum_common::mls::MlsLevel::Secret
        );
    }
}

// =============================================================================
// Audit-A4: meta-value → AST literal conversion (file-scope free function)
// =============================================================================

/// Convert a `MetaValue` (the const-generic environment's binding type) to
/// an AST `Literal` so a refinement predicate's `Path(N)` can be rewritten
/// to a literal at substitution time.
///

/// Returns `None` for `MetaValue` shapes that have no direct literal
/// representation (compound types, AST values). The caller leaves the path
/// unchanged in that case so SMT continues to see a symbolic reference.
fn meta_value_to_literal(value: &verum_ast::MetaValue) -> Option<verum_ast::literal::Literal> {
    use verum_ast::literal::{FloatLit, IntLit, Literal, LiteralKind, StringLit};
    use verum_ast::span::Span;
    let span = Span::dummy();
    match value {
        verum_ast::MetaValue::Bool(b) => Some(Literal::new(LiteralKind::Bool(*b), span)),
        verum_ast::MetaValue::Int(i) => Some(Literal::new(
            LiteralKind::Int(IntLit {
                value: *i,
                suffix: None,
            }),
            span,
        )),
        // UInt is folded into Int (i128 covers practical const-generic ranges).
        verum_ast::MetaValue::UInt(u) => Some(Literal::new(
            LiteralKind::Int(IntLit {
                value: (*u) as i128,
                suffix: None,
            }),
            span,
        )),
        verum_ast::MetaValue::Float(f) => Some(Literal::new(
            LiteralKind::Float(FloatLit {
                value: *f,
                suffix: None,
            }),
            span,
        )),
        verum_ast::MetaValue::Char(c) => Some(Literal::new(LiteralKind::Char(*c), span)),
        verum_ast::MetaValue::Text(t) => Some(Literal::new(
            LiteralKind::Text(StringLit::Regular(t.clone())),
            span,
        )),
        _ => None,
    }
}

// ============================================================================
// T2-extended-perf: lazy stdlib type registration helpers
// ============================================================================

/// Scan a top-level [`verum_ast::Item`] for every named type
/// reference (in field types, function signatures, type
/// declarations, etc.) and accumulate the bare names into `out`.
///
/// Used by [`TypeChecker::register_stdlib_types_for_module`] to
/// build the closure of stdlib types the user module references,
/// so only those are pulled out of `core_metadata` (skipping the
/// other 99% of stdlib types every cold start used to register
/// upfront).
fn collect_named_types_from_item(
    item: &verum_ast::Item,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::ItemKind;
    match &item.kind {
        ItemKind::Function(f) => {
            for p in f.params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    collect_named_types_from_ty(ty, out);
                }
            }
            if let Some(rt) = f.return_type.as_ref() {
                collect_named_types_from_ty(rt, out);
            }
            if let verum_common::Maybe::Some(body) = &f.body {
                collect_named_types_from_function_body(body, out);
            }
        }
        ItemKind::Type(td) => {
            // Field / variant payload types pull in their referenced
            // names so the user-defined type's transitive closure
            // through stdlib ends up loaded.
            collect_named_types_from_type_decl_body(&td.body, out);
        }
        ItemKind::Const(c) => {
            collect_named_types_from_ty(&c.ty, out);
        }
        ItemKind::Static(s) => {
            collect_named_types_from_ty(&s.ty, out);
        }
        ItemKind::Mount(_) => {
            // mount declarations carry symbol names, not type names —
            // the symbols themselves get resolved through other
            // registration paths.  No type-name harvest here.
        }
        ItemKind::Impl(impl_decl) => {
            collect_named_types_from_impl_kind(&impl_decl.kind, out);
            for it in impl_decl.items.iter() {
                collect_named_types_from_impl_item(it, out);
            }
        }
        ItemKind::Protocol(_)
        | ItemKind::Module(_)
        | ItemKind::Theorem(_)
        | ItemKind::Lemma(_)
        | ItemKind::Corollary(_)
        | ItemKind::Axiom(_) => {
            // Less common in user scripts; the lazy loader will
            // catch them via direct lookup-on-miss when needed.
        }
        _ => {}
    }
}

fn collect_named_types_from_ty(
    ty: &verum_ast::ty::Type,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                out.insert(ident.name.clone());
            }
            // Multi-segment paths: also harvest the LAST segment as
            // a likely type name.  `core.io.fs.Path` brings in
            // `Path`.  The first-segment names tend to be modules,
            // not types, so we don't harvest them.
            if path.segments.len() > 1 {
                if let Some(verum_ast::ty::PathSegment::Name(last)) = path.segments.last() {
                    out.insert(last.name.clone());
                }
            }
        }
        TypeKind::Generic { base, args } => {
            collect_named_types_from_ty(base, out);
            for a in args {
                if let verum_ast::ty::GenericArg::Type(t) = a {
                    collect_named_types_from_ty(t, out);
                }
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. }
        | TypeKind::Pointer { inner, .. } => {
            collect_named_types_from_ty(inner, out);
        }
        TypeKind::Slice(inner) => {
            collect_named_types_from_ty(inner, out);
        }
        TypeKind::Array { element, .. } => {
            collect_named_types_from_ty(element, out);
        }
        TypeKind::Tuple(elems) => {
            for e in elems {
                collect_named_types_from_ty(e, out);
            }
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            for p in params {
                collect_named_types_from_ty(p, out);
            }
            collect_named_types_from_ty(return_type, out);
        }
        _ => {}
    }
}

fn collect_named_types_from_type_decl_body(
    body: &verum_ast::decl::TypeDeclBody,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::TypeDeclBody;
    match body {
        TypeDeclBody::Alias(t) | TypeDeclBody::Newtype(t) => {
            collect_named_types_from_ty(t, out);
        }
        TypeDeclBody::Record(fields) => {
            for f in fields.iter() {
                collect_named_types_from_ty(&f.ty, out);
            }
        }
        TypeDeclBody::Variant(variants) => {
            for v in variants.iter() {
                if let verum_common::Maybe::Some(data) = &v.data {
                    use verum_ast::decl::VariantData;
                    match data {
                        VariantData::Tuple(tys) => {
                            for t in tys.iter() {
                                collect_named_types_from_ty(t, out);
                            }
                        }
                        VariantData::Record(fields) => {
                            for f in fields.iter() {
                                collect_named_types_from_ty(&f.ty, out);
                            }
                        }
                    }
                }
            }
        }
        TypeDeclBody::Tuple(tys) | TypeDeclBody::SigmaTuple(tys) => {
            for t in tys.iter() {
                collect_named_types_from_ty(t, out);
            }
        }
        TypeDeclBody::Protocol(_) | TypeDeclBody::Unit => {}
        // Less common forms — pull names conservatively where shape is known.
        _ => {}
    }
}

/// Walk a `verum_ast::decl::ImplItem` (a function / type / const
/// inside an impl block) and harvest its named type references.
fn collect_named_types_from_impl_item(
    item: &verum_ast::decl::ImplItem,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::ImplItemKind;
    match &item.kind {
        ImplItemKind::Function(f) => {
            for p in f.params.iter() {
                if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                    collect_named_types_from_ty(ty, out);
                }
            }
            if let Some(rt) = f.return_type.as_ref() {
                collect_named_types_from_ty(rt, out);
            }
        }
        _ => {}
    }
}

fn collect_named_types_from_impl_kind(
    kind: &verum_ast::decl::ImplKind,
    out: &mut std::collections::HashSet<Text>,
) {
    use verum_ast::decl::ImplKind;
    match kind {
        ImplKind::Inherent(ty) => {
            collect_named_types_from_ty(ty, out);
        }
        ImplKind::Protocol {
            protocol,
            for_type,
            ..
        } => {
            if let Some(ident) = protocol.as_ident() {
                out.insert(ident.name.clone());
            }
            collect_named_types_from_ty(for_type, out);
        }
    }
}

fn collect_named_types_from_function_body(
    _body: &verum_ast::FunctionBody,
    _out: &mut std::collections::HashSet<Text>,
) {
    // Function body harvest is intentionally NOT recursed into for
    // the V0 lazy pre-pass — function-body type references go
    // through the bare `ast_to_type` path which falls back to
    // `Type::Named` opaque on miss, and then through the path
    // resolution in expression typechecking.  Adding deep body
    // walking here would re-introduce most of the eager-load cost.
    //
    // Real-world scripts: function signatures + record/variant
    // fields cover ~95% of stdlib type references; body-only
    // references (e.g., a transient `let x: Maybe<Int> = ...`
    // inside a function) are picked up by the lookup-on-miss
    // fallback at typecheck time.
}

/// Per-variant signature registration extracted from
/// `load_stdlib_from_metadata` so the lazy loader can register one
/// type's variants without walking the entire stdlib.  Mirrors the
/// eager loader's behaviour for that single type.
/// Parse a `archive_metadata::type_ref_to_text` output string
/// back into a structured `Type`.
///
/// **Stdlib-agnostic**: no hardcoded type names.  Built-in
/// primitive type names (`Int`, `Float`, `Bool`, `Char`, `Text`,
/// `Unit`, …) are registered as `Type::*` variants in
/// `ctx.type_defs` by `register_primitives`; user-side resolution
/// via `Type::Named` lookup recovers them at unify time.  This
/// parser is a pure structural decoder for compound type strings:
///
/// * empty / `"()"` → `Type::Unit` (the single language-level
///   special case — `()` is a sigil, not a type name);
/// * `"&T"` / `"&mut T"` → `Type::Reference` over the parsed
///   inner type;
/// * `"Base<arg1, arg2, …>"` → `Type::Generic` with parsed args
///   (top-level commas only, nested generics handled via
///   depth counter);
/// * bare identifiers → `Type::Named` — the unifier's
///   `try_expand_alias` and `ctx.lookup_type` resolve these
///   against the user's type registry.
///
/// Without this parser, signatures stored as compound strings
/// degrade to opaque `Type::Named { path: "IoResult<Metadata>" }`
/// blobs that never unify with `Type::Generic { name: "IoResult",
/// args: [Type::Named { Metadata }] }` at call sites.
fn parse_descriptor_type_string(raw: &str) -> Type {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "()" {
        return Type::Unit;
    }
    // VBC opaque-type fallbacks → fresh type variable.  The
    // string `"__opaque_type_N"` (from `archive_metadata::type_ref_to_text`'s
    // fallback for unmapped concrete TypeIds) and `"__generic_N"`
    // (TypeRef::Generic(N) without a param-name map) both
    // represent "VBC didn't resolve this further" — the unifier
    // should treat them as fresh type variables so they unify
    // with any concrete type at use sites.  Without this, every
    // method signature carrying an unresolved cross-module
    // TypeId fails downstream unification (`expected
    // '__opaque_type_14', found 'Text'`).
    if trimmed.starts_with("__opaque_type_")
        || trimmed.starts_with("__generic_")
        || trimmed == "__opaque_typeref"
    {
        return Type::Var(crate::ty::TypeVar::fresh());
    }
    // References: "&mut T" / "&T".
    if let Some(rest) = trimmed.strip_prefix("&mut ") {
        return Type::Reference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: true,
        };
    }
    if let Some(rest) = trimmed.strip_prefix('&') {
        return Type::Reference {
            inner: Box::new(parse_descriptor_type_string(rest.trim_start())),
            mutable: false,
        };
    }
    // Generic instantiation: "Base<arg1, arg2, ...>".
    if let Some(open) = trimmed.find('<') {
        if trimmed.ends_with('>') {
            let base = &trimmed[..open];
            let inside = &trimmed[open + 1..trimmed.len() - 1];
            let args = split_top_level_commas(inside)
                .into_iter()
                .map(|s| parse_descriptor_type_string(s.trim()))
                .collect();
            return Type::Generic {
                name: Text::from(base),
                args,
            };
        }
    }
    // Language primitives — these have dedicated `TypeKind`
    // variants in the AST (`TypeKind::Bool`, `TypeKind::Int`, …)
    // and dedicated `Type::Bool` / `Type::Int` / … sentinels in
    // the type system.  They are NOT stdlib types — the grammar
    // parses them as built-ins distinct from `TypeKind::Path`.
    // Without this normalisation, archive_metadata's textual
    // payloads ("Bool" / "Int" / …) round-trip through the
    // structural parser as `Type::Named { path: "Bool" }` and
    // every downstream check that branches on `Type::Bool`
    // (logical NOT, integer arithmetic, etc.) misses them.
    match trimmed {
        "Bool" => return Type::Bool,
        "Int" => return Type::Int,
        "Float" => return Type::Float,
        "Char" => return Type::Char,
        "Text" => return Type::Text,
        "Never" => return Type::Never,
        "Unit" => return Type::Unit,
        _ => {}
    }
    // Other named types → Type::Named.  Resolution flows
    // through the unifier's lookup against `ctx.type_defs`
    // populated by `register_primitives`.
    Type::Named {
        path: TypeChecker::text_to_path(&Text::from(trimmed)),
        args: List::new(),
    }
}

/// Split a string on commas at depth=0 (top-level), respecting
/// nesting in `<>`/`()`/`[]`.  Used by
/// `parse_descriptor_type_string` for generic-arg lists.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut depth: i32 = 0;
    let mut start = 0;
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn register_variant_signature_for_lazy(
    checker: &mut TypeChecker,
    name: &Text,
    type_desc: &crate::core_metadata::TypeDescriptor,
    cases: &List<crate::core_metadata::VariantCase>,
    pending: &mut Vec<Text>,
) {
    // Push payload type names → pending so the lazy loader
    // closure picks them up.
    for case in cases.iter() {
        if let Maybe::Some(payload) = &case.payload {
            match payload {
                crate::core_metadata::VariantPayload::Tuple(types) => {
                    for t in types.iter() {
                        if !t.is_empty() {
                            pending.push(t.clone());
                        }
                    }
                }
                crate::core_metadata::VariantPayload::Record(fields) => {
                    for f in fields.iter() {
                        if !f.ty.is_empty() {
                            pending.push(f.ty.clone());
                        }
                    }
                }
            }
        }
    }

    // #126 — generic-parameter substitution at variant construction.
    //
    // Map every parent generic-param NAME (e.g. `"T"`, `"A"`, `"E"`)
    // to a freshly-allocated `TypeVar`.  The variant payload types
    // are then built using these vars instead of literal
    // `Type::Named { path: "T" }` placeholders.  When the unit-variant
    // env entry is inserted as a `TypeScheme::poly` quantified over
    // the same vars, every lookup yields a freshly-instantiated
    // `Type::Variant` whose generic positions are fresh `Type::Var`s
    // — exactly what the unifier expects at `mapped == None` sites.
    //
    // Pre-fix this function inserted the variant_type as a `mono`
    // scheme whose payload positions held rigid `Type::Named "T"`,
    // so `Maybe<Int> == None` failed with `expected 'T', found 'Int'`
    // because the unifier compared `Int` (concrete) against `Named "T"`
    // (rigid name) with no rule to unify.
    use indexmap::IndexMap;
    use crate::ty::TypeVar;
    let param_to_var: IndexMap<Text, TypeVar> = type_desc
        .generic_params
        .iter()
        .map(|gp| (gp.name.clone(), TypeVar::fresh()))
        .collect();
    let resolve_payload_name = |t: &Text| -> Type {
        if let Some(tv) = param_to_var.get(t) {
            Type::Var(*tv)
        } else {
            Type::Named {
                path: TypeChecker::text_to_path(t),
                args: List::new(),
            }
        }
    };

    let mut variant_map: IndexMap<Text, Type> = IndexMap::new();
    for case in cases.iter() {
        let payload_ty = match &case.payload {
            Maybe::None => Type::Unit,
            Maybe::Some(crate::core_metadata::VariantPayload::Tuple(types)) => {
                if types.len() == 1 {
                    resolve_payload_name(&types[0])
                } else {
                    Type::Tuple(types.iter().map(&resolve_payload_name).collect())
                }
            }
            Maybe::Some(crate::core_metadata::VariantPayload::Record(fields)) => {
                let field_map: IndexMap<Text, Type> = fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_payload_name(&f.ty)))
                    .collect();
                Type::Record(field_map)
            }
        };
        variant_map.insert(case.name.clone(), payload_ty);
    }

    let variant_type = Type::Variant(variant_map.clone());
    if let Some(sig) = TypeChecker::variant_type_signature(&variant_type) {
        checker.register_variant_type_name_first_wins(sig.clone(), name.clone());
        if let Some(relaxed) = TypeChecker::variant_type_signature_relaxed(&variant_type) {
            if relaxed != sig {
                checker.register_variant_type_name_first_wins(relaxed, name.clone());
            }
        }
    }

    // Variant constructor parent mappings.
    for (vname, _payload_ty) in &variant_map {
        let parents = checker
            .variant_constructor_parents
            .entry(vname.clone())
            .or_default();
        if !parents.iter().any(|p| p == name) {
            parents.push(name.clone());
        }
    }

    // Register unit-variant constructors as env values (so `None`,
    // `Less`, `Greater`, … resolve as expressions) and ALWAYS the
    // qualified `Type.Variant` form.
    //
    // #126 — when the parent has generic params, the env entry must
    // be a *polymorphic* `TypeScheme` quantified over the same fresh
    // TypeVars that we substituted into the payload positions. This
    // way every `lookup → instantiate` yields a fresh per-call-site
    // copy with independent unification slots.
    use crate::context::TypeScheme;
    let scheme_vars: List<TypeVar> = param_to_var.values().copied().collect();
    let make_scheme = || -> TypeScheme {
        if scheme_vars.is_empty() {
            TypeScheme::mono(variant_type.clone())
        } else {
            TypeScheme::poly(scheme_vars.clone(), variant_type.clone())
        }
    };

    for (vname, payload_ty) in &variant_map {
        if *payload_ty == Type::Unit {
            if checker.ctx.env.lookup(vname.as_str()).is_none() {
                checker.ctx.env.insert(vname.clone(), make_scheme());
            }
        }
        let qualified_name: Text = format!("{}.{}", name, vname).into();
        checker.ctx.env.insert(qualified_name, make_scheme());
    }

    // Payload-bearing variant constructors are NOT registered as
    // env functions here.  The eager `load_stdlib_from_metadata`
    // path (lines 1717-1731) doesn't register them either —
    // dispatch goes through `variant_constructor_parents` (set
    // above) and the typechecker's own variant-resolution path.
    // Pre-fix attempt to register them as `fn(T) -> Variant`
    // typed env entries broke method dispatch on generics
    // (`list.len()`, `maybe.unwrap_or(0)`, …) because the
    // constructor's typed shape interfered with the type-method
    // resolution path.
}
