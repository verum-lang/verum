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
    /// #91 Phase 3 — pre-resolved-static-call side-table.
    ///
    /// During `infer_method_call_inner_impl`, when the dispatch
    /// resolves unambiguously to a single `FunctionId` (or variant
    /// constructor), the resolution is written here keyed by the
    /// MethodCall expression's `Span`.  After type checking
    /// completes, `commit_resolved_call_targets` walks the AST
    /// mutably and stamps each `Expr::resolved_call_target` from
    /// this table.  At codegen time, `compile_method_call` reads
    /// the field and emits a direct `Call` / `MakeVariant` in O(1)
    /// instead of replaying the legacy 7-step name-resolution
    /// cascade in `try_resolve_static_method`.
    ///
    /// Side-table architecture chosen over `&mut Expr` walk:
    /// inference is `&Expr`-driven across hundreds of call sites
    /// (synth_expr, check_expr, infer_method_call_inner, …);
    /// converting them all to `&mut Expr` is mechanically large and
    /// risks subtle borrow conflicts.  The side-table keeps
    /// inference immutable and concentrates the mut walk in a
    /// single finalisation pass.  Span uniqueness is sufficient for
    /// non-macro source — every method-call expression in the
    /// parsed AST has a distinct span.
    pub(crate) resolved_call_targets:
        std::collections::HashMap<verum_ast::span::Span, verum_ast::expr::ResolvedCallTarget>,
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

// TypeChecker constructor, stdlib bootstrap, and factory methods
// → see infer/core.rs in this module
pub(crate) mod core;

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

