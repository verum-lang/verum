//! Type unification algorithm.
//!
//! This module implements Robinson's unification algorithm with extensions for:
//! - Refinement types (structural unification)
//! - Occurs check (prevent infinite types)
//! - Substitution composition
//! - Dependent types (Pi, Sigma, Eq, Universe)
//! - Inductive and coinductive types
//! - Higher inductive types (HITs)
//! - Quantitative type theory
//!
//! Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking

use crate::ty::{EqTerm, Quantity, Substitution, SubstitutionExt, Type, TypeVar, UniverseLevel};
use crate::{Result, TypeError};
use indexmap::IndexMap;
use std::sync::Arc;
use verum_ast::span::Span;
#[allow(unused_imports)]
use verum_common::Text;
use verum_common::ToText;
use verum_common::{List, Map, Maybe};

#[path = "unify_rules.rs"]
mod rules;

#[derive(Default)]
struct VariantRegistrationJournal {
    names: Map<Text, Option<Text>>,
    originals: Map<Text, Option<Type>>,
    orders: Map<Text, Option<List<Type>>>,
}

/// Unifier performs type unification.
///
/// CRITICAL: The Unifier now maintains an accumulated substitution that
/// grows with each unification. This is essential for proper Hindley-Milner
/// type inference - type variables resolved in earlier unifications must
/// be reflected in later type comparisons.
///
/// Hindley-Milner type inference: algorithm W with let-polymorphism, constraint collection, and unification
pub struct Unifier {
    /// Count of unifications performed (for metrics)
    pub unify_count: usize,
    /// Current recursion depth for unify_inner (prevents stack overflow)
    unify_depth: u32,
    /// Maximum permitted unify_inner recursion depth (#304).
    /// Pre-fix this was a hardcoded `MAX_UNIFY_DEPTH = 50`
    /// constant; embedders running heavy generic-type inference
    /// could not raise the cap. Default 50 stays for backward
    /// compatibility (50 frames × ~2500 lines of match arms ≈
    /// the safe stack-frame budget on 64MB-stack threads); use
    /// `with_max_unify_depth` to override.
    max_unify_depth: u32,
    /// Accumulated substitution from all unifications
    /// This is composed with each new unification result
    substitution: Substitution,
    /// Registry mapping variant signatures to type names for stdlib-agnostic unification.
    /// Key is a stable string signature of the variant structure (sorted variant names),
    /// Value is the nominal type name (e.g., "Maybe", "Result").
    /// This enables Generic<->Variant unification without hardcoding stdlib types.
    variant_type_names: Map<Text, Text>,
    /// Original (unsubstituted) variant type definitions.
    /// Key is the type name, value is the original Type::Variant with TypeVars.
    /// Used for stdlib-agnostic Generic<->Variant unification to properly map
    /// type arguments to their corresponding positions in the variant structure.
    original_variant_types: Map<Text, Type>,
    /// Protocol declaration keys retain nominal identity during unification.
    /// Only the type checker has the evidence needed to erase a carrier.
    protocol_names: std::collections::HashSet<Text>,
    /// Complete declaration-owned parameter telescopes for sum types.
    variant_parameters: Map<Text, List<Type>>,
    variant_registration_journal: Option<VariantRegistrationJournal>,
    /// Type alias registry: maps alias names to their target types.
    /// E.g., "Byte" -> Named("UInt8"), enabling transparent unification
    /// through aliases like `type Byte is UInt8;`.
    type_aliases: Arc<Map<Text, Type>>,
    /// Type parameter names for generic type aliases, in declaration order.
    /// E.g., "IoResult" -> ["T"] for `type IoResult<T> is Result<T, StreamError>`.
    /// Used by try_expand_alias to substitute type arguments into generic aliases.
    type_alias_params: Map<Text, List<Text>>,
    /// Data-driven set of collection type names that support array coercion.
    /// Array literals `[1, 2, 3]` can unify with any `C<T>` where `C` is in this set.
    /// Populated via `register_array_coercible_type` from
    /// `implement<T> ArrayCoercible for X<T> {}` blocks discovered
    /// by `verum_compiler::stdlib_coercion_registry::scan_protocol_implementations`
    /// during stdlib bootstrap.  Default is empty so a stdlib that
    /// fails to declare any `ArrayCoercible` impl produces a
    /// type-mismatch error at array-literal sites rather than
    /// silently falling back to a hardcoded "List" assumption.
    array_coercible_types: std::collections::HashSet<Text>,
    /// Data-driven set of type names in the "tensor family" that coerce freely with each
    /// other and with scalar primitives (Float, Int, Bool).
    /// Defaults to `{"DynTensor", "Tensor", "Vector", "Cotangent", "Tangent"}`.
    /// Will be superseded by a `TensorLike` or `Numeric` protocol check once protocols
    /// are available in unification.
    tensor_family_types: std::collections::HashSet<Text>,
    /// Data-driven set of Named type names that coerce bidirectionally with `Int`.
    /// Includes sized integer aliases, FFI integer newtypes, tensor types, and
    /// collection types that participate in index/length coercions.
    /// Will be superseded by structural checks (e.g., `IntCoercible` protocol or
    /// querying type declarations for integer newtype wrappers).
    int_coercible_named_types: std::collections::HashSet<Text>,
    /// Data-driven set of range-like type names whose Generic form coerces with `Tuple`
    /// (e.g., `Range<Int>` vs `(Maybe<USize>, Maybe<USize>)` from slicing).
    /// Will be superseded by a `RangeLike` protocol check.
    range_like_types: std::collections::HashSet<Text>,
    /// Data-driven set of sized numeric type names that coerce with each other
    /// in Named<->Named cross-coercion (e.g., USize ↔ UInt64, Int32 ↔ I32).
    /// These are arguably part of the language definition (not stdlib), but are
    /// still centralized here to avoid scattered string literals.
    sized_numeric_types: std::collections::HashSet<Text>,
    /// Types whose in-memory layout is a packed-C-struct byte mirror
    /// of a kernel/libsystem ABI (sockaddr family, etc.) and that
    /// the unifier should accept against `[Byte]` / `[UInt8]` byte
    /// buffers at FFI boundaries.  Populated by stdlib loading via
    /// `register_bytewise_ffi_type` (driven by
    /// `implement BytewiseFfi for X {}` blocks).  Replaces the
    /// hardcoded `matches!(name, "Sockaddr" | "SocketAddr" |
    /// "SockaddrIn")` short-circuit that previously lived inline in
    /// the Named↔Named unification arm.
    bytewise_ffi_types: std::collections::HashSet<Text>,
    /// Current Self type for implement blocks.
    /// When set, Type::Named("Self") is treated as equivalent to this type during unification.
    /// This prevents "expected 'Foo', found 'Self'" errors when Self leaks into type comparisons.
    self_type: Option<Type>,
    /// Context variable bindings from context unification.
    /// Maps context type variables to their resolved ContextExpr values.
    context_bindings: IndexMap<TypeVar, crate::di::requirement::ContextExpr>,

    /// Whether cubical-type normalization is enabled (Path/Partial/Eq
    /// definitional equality uses the cubical `whnf` normalizer).
    ///
    /// When `false` (i.e. `[types] cubical = false` in `verum.toml`),
    /// the unifier falls back to strict syntactic equality on cubical
    /// terms. Users disabling cubical lose identities like
    /// `transport (refl A) x ≡ x`, which is the point — they pay zero
    /// normalization cost in return.
    cubical_enabled: bool,
    /// T0701: associated-type projection resolver hook.
    ///
    /// The projection arms of `unify_inner` promised three cases in
    /// their comment; case 3 ("projection has concrete base → try to
    /// resolve, then unify result") was never implemented, so
    /// `::Item[Range<Int>]` vs `Int` fell through to the plain
    /// Generic-vs-other mismatch (rendered `expected 'Item<Range<Int>>',
    /// found 'Int'`) at every call site that used bare `unify` instead
    /// of `unify_with_projections` — i.e. most of them.  The unifier
    /// has no ProtocolChecker of its own, so InferenceContext installs
    /// this closure at construction (over its shared checker); the
    /// checker's `try_find_associated_type` carries its own cycle
    /// detection.
    projection_resolver:
        Option<std::sync::Arc<dyn Fn(&Type, &str) -> Option<Type> + Send + Sync>>,

    /// Declaration-backed equivalence for differently spelled nominal heads.
    /// The metadata loader installs an exact-key snapshot; the unifier must
    /// never infer ownership from a shared final path segment.
    nominal_identity_resolver:
        Maybe<std::sync::Arc<dyn Fn(&str, &str) -> bool + Send + Sync>>,

    /// Type variables that are RIGID: universally quantified parameters of
    /// the function body currently being checked.
    ///
    /// `fn f<T>(t: T) -> T` promises to work for every `T` the caller
    /// picks, so inside the body `T` stands for a type that is not yet
    /// known and cannot be discovered. Represented as an ordinary
    /// `TypeVar` that this set marks as unbindable, rather than as a new
    /// `Type` variant — every existing match arm keeps working, and the
    /// one place that could break the promise is [`Unifier::bind_var`].
    ///
    /// Empty everywhere except inside a generic body. The function's
    /// published scheme still quantifies over exactly these vars
    /// (`TypeScheme::poly`), so a call site instantiates fresh flexible
    /// ones and is unaffected.
    ///
    /// Carries the SOURCE NAME of each parameter, because the diagnostic is
    /// half the fix. A rigid var renders through the ordinary var path as
    /// `_`, and "expected `_`, found `Int`" tells the author nothing about
    /// which promise they broke; "expected `T`, found `Int`" names it.
    rigid_vars: Map<TypeVar, RigidVar>,
}

/// What is known about one rigid type parameter for the duration of a body.
///
/// `bounded` used to be the whole story, and it is the reason a call
/// through a bounded parameter had no result type. Measured 2026-09-03,
/// two files differing in one thing:
///
/// ```text
///     fn via_bound<U, F: fn(Int) -> List<U>>(f: F) -> Int {
///         let inner = f(1);           // error<E404>: `inner` is `_`
///         inner.len()
///     }
///     fn via_direct<U>(f: fn(Int) -> List<U>) -> Int {
///         let inner = f(1);           // clean
///         inner.len()
///     }
/// ```
///
/// `bind_var` permits the call — a bounded parameter may take a function
/// shape, because the bound promised one — but the table recorded only
/// THAT a bound exists, never BY WHAT. So the caller of `bind_var` had
/// invented a shape out of fresh variables and unification had nothing to
/// narrow it with, and the return type stayed unsolved.
///
/// `fn_bound` restores the discarded half. It is the parameter's declared
/// function type, already resolved in the scope that declared it, so a
/// call site can ask for the signature instead of guessing at it.
#[derive(Debug, Clone, PartialEq)]
pub struct RigidVar {
    /// The name the parameter was written under — the diagnostic is half
    /// the fix, see the field comment above.
    pub name: Text,
    /// Whether the parameter carries any bound at all.
    pub bounded: bool,
    /// The parameter's bound when that bound is a function type
    /// (`F: fn(A) -> B`), which is the only bound shape that answers
    /// "what does calling this parameter produce".
    pub fn_bound: Maybe<Type>,
}

const MAX_ALIAS_EXPANSION_DEPTH: usize = 30;

/// Default `unify_inner` recursion budget (#304). 50 frames ×
/// ~2500-line match arms ≈ the safe stack-frame budget on
/// 64 MiB-stack threads. Embedders with deeper generic-type
/// inference can raise via `Unifier::with_max_unify_depth`.
pub const DEFAULT_MAX_UNIFY_DEPTH: u32 = 50;

impl Unifier {
    pub fn new() -> Self {
        Self::with_type_aliases(Arc::default())
    }

    /// Start a fresh inference state with an already validated alias table.
    pub(crate) fn with_type_aliases(type_aliases: Arc<Map<Text, Type>>) -> Self {
        // Array-coercible types — populated dynamically via
        // `register_array_coercible_type()` from stdlib's
        // `implement<T> ArrayCoercible for X<T> {}` blocks.
        // Default is empty; List opts in via its own implement
        // block in `core/collections/list.vr`.
        let array_coercible_types: std::collections::HashSet<Text> =
            std::collections::HashSet::new();

        // Tensor family: populated via register_tensor_family_type() from
        // stdlib declarations (types implementing TensorLike protocol).
        let tensor_family_types: std::collections::HashSet<Text> = std::collections::HashSet::new();

        // Named types that coerce bidirectionally with Int.
        // Only language-level primitive integer types are hardcoded here.
        // Stdlib types (Duration, Path, etc.) are registered dynamically
        // via register_int_coercible_type() during type checking.
        // Primitive integer aliases are NOT seeded here — `is_int_coercible_named`
        // delegates the primitive-classification arm to
        // `verum_common::well_known_types::type_names::is_integer_type`, the
        // canonical drift-pinned registry. Only stdlib `implement` blocks
        // (e.g. `implement IntCoercible for Duration {}` in
        // `core/time/duration.vr`) feed names into this HashSet via
        // `register_int_coercible_type`; the seed is intentionally empty so
        // a stdlib that opts no type in still gets correct primitive
        // classification through the predicate.
        let int_coercible_named_types: std::collections::HashSet<Text> =
            std::collections::HashSet::new();

        // Range-like types — populated via register_range_like_type() from stdlib.
        let range_like_types: std::collections::HashSet<Text> = std::collections::HashSet::new();

        // Sized numeric types that cross-coerce with each other (Named<->Named).
        // These are arguably language-level (not stdlib), but centralized here.
        // Sized numeric types — language primitives only, no stdlib types.
        // Additional types can be registered via register_sized_numeric_type().
        // Same delegation pattern as `int_coercible_named_types` above:
        // the primitive sized-numeric alias set lives in the canonical
        // `verum_common::well_known_types::type_names::is_numeric_type`
        // registry. Only stdlib types that opt in via
        // `register_sized_numeric_type` (typically `implement
        // SizedNumeric for Duration {}` style) populate this HashSet.
        let sized_numeric_types: std::collections::HashSet<Text> =
            std::collections::HashSet::new();

        // BytewiseFfi types — packed C-struct byte mirrors of
        // libsystem ABI types.  Populated dynamically via
        // `register_bytewise_ffi_type()` from stdlib `implement
        // BytewiseFfi for X {}` blocks; default is empty so a
        // type that hasn't opted in correctly produces a
        // type-mismatch error rather than a silent FFI-shape
        // coercion.
        let bytewise_ffi_types: std::collections::HashSet<Text> = std::collections::HashSet::new();

        Self {
            unify_count: 0,
            unify_depth: 0,
            max_unify_depth: DEFAULT_MAX_UNIFY_DEPTH,
            substitution: Substitution::new(),
            variant_type_names: Map::new(),
            original_variant_types: Map::new(),
            protocol_names: std::collections::HashSet::new(),
            variant_parameters: Map::new(),
            variant_registration_journal: None,
            type_aliases,
            type_alias_params: Map::new(),
            array_coercible_types,
            tensor_family_types,
            int_coercible_named_types,
            range_like_types,
            sized_numeric_types,
            bytewise_ffi_types,
            self_type: None,
            context_bindings: IndexMap::new(),
            cubical_enabled: true,
            projection_resolver: None,
            nominal_identity_resolver: None,
            rigid_vars: Map::new(),
        }
    }

    /// Make `vars` rigid for the duration of a function body, each paired
    /// with the name it was written under.
    ///
    /// Adds rather than replaces: a generic function declared inside a
    /// generic function must keep the OUTER parameters rigid too, or the
    /// inner body could discover the outer `T`.
    pub fn enter_rigid_scope(
        &mut self,
        vars: impl IntoIterator<Item = (TypeVar, Text, bool, Maybe<Type>)>,
    ) {
        for (var, name, bounded, fn_bound) in vars {
            self.rigid_vars.insert(
                var,
                RigidVar {
                    name,
                    bounded,
                    fn_bound,
                },
            );
        }
    }

    /// Record that a parameter carries a bound after all.
    ///
    /// A `where` clause is processed after the body's rigid scope is
    /// installed, so a parameter written `fn f<T>(…) where T: P` enters the
    /// scope looking unbounded and has to be corrected here.
    pub fn mark_rigid_bounded(&mut self, var: TypeVar) {
        if let Some(entry) = self.rigid_vars.get_mut(&var) {
            entry.bounded = true;
        }
    }

    /// Record the function type a parameter is bounded by, for a bound that
    /// arrives after the rigid scope was installed (a `where` clause).
    pub fn set_rigid_fn_bound(&mut self, var: TypeVar, fn_bound: Type) {
        if let Some(entry) = self.rigid_vars.get_mut(&var) {
            entry.bounded = true;
            entry.fn_bound = Maybe::Some(fn_bound);
        }
    }

    /// Restore the rigid set saved by [`Unifier::rigid_snapshot`].
    pub fn exit_rigid_scope(&mut self, previous: Map<TypeVar, RigidVar>) {
        self.rigid_vars = previous;
    }

    /// The current rigid set, for a caller that must restore it on EVERY
    /// exit path including the early ones.
    ///
    /// A body check leaves through a dozen `?` operators, so the restore
    /// cannot sit at the bottom of the function that installs the scope: an
    /// error would leak the parameters of a half-checked body into the next
    /// function, where they belong to nothing and would refuse to bind.
    pub fn rigid_snapshot(&self) -> Map<TypeVar, RigidVar> {
        self.rigid_vars.clone()
    }

    /// Whether `var` is a rigid parameter of the body being checked.
    pub fn is_rigid(&self, var: TypeVar) -> bool {
        self.rigid_vars.contains_key(&var)
    }

    /// Callable contracts declared by universally quantified callers in scope.
    pub(crate) fn rigid_callable_bounds(&self) -> impl Iterator<Item = (TypeVar, &Type)> {
        self.rigid_vars.iter().filter_map(|(var, entry)| {
            entry.fn_bound.as_ref().map(|contract| (*var, contract))
        })
    }

    /// The function type a rigid parameter is bounded by, if it has one.
    ///
    /// A call site asks this instead of inventing a shape out of fresh
    /// variables: the parameter's own declaration already says what calling
    /// it produces, and inventing the shape is what left the result type
    /// unsolved. See [`RigidVar`] for the measurement.
    pub fn rigid_fn_bound(&self, var: TypeVar) -> Maybe<Type> {
        self.rigid_vars
            .get(&var)
            .map(|entry| entry.fn_bound.clone())
            .unwrap_or(Maybe::None)
    }

    /// How a rigid var should read in a diagnostic: the name the author
    /// wrote, not the `_` an unsolved variable renders as.
    fn rigid_name(&self, var: TypeVar) -> Text {
        self.rigid_vars
            .get(&var)
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| Type::Var(var).to_text())
    }

    /// Whether a rigid parameter carries a protocol or structural bound.
    fn rigid_is_bounded(&self, var: TypeVar) -> bool {
        self.rigid_vars
            .get(&var)
            .map(|entry| entry.bounded)
            .unwrap_or(false)
    }

    /// Enable or disable cubical normalization during unification.
    ///
    /// Populated by the compilation session from `[types] cubical` in
    /// `verum.toml`. Disabling causes PathType/Partial/Eq arms to use
    /// strict syntactic equality instead of `CubicalTerm::whnf`-based
    /// definitional equality.
    pub fn set_cubical_enabled(&mut self, enabled: bool) {
        self.cubical_enabled = enabled;
    }

    /// Query the current cubical-normalization setting.
    pub fn cubical_enabled(&self) -> bool {
        self.cubical_enabled
    }

    /// Set the current Self type for implement block unification.
    /// When set, Type::Named("Self") or Type::Generic("Self") will be resolved
    /// to this concrete type during unification.
    pub fn set_self_type(&mut self, self_type: Option<Type>) {
        self.self_type = self_type;
    }

    /// If the given type is the symbolic "Self" type, resolve it to the concrete self_type.
    /// Returns the type unchanged if it's not Self or no self_type is set.
    fn resolve_self(&self, ty: &Type) -> Option<Type> {
        self.self_type.as_ref()?;
        match ty {
            Type::Named { path, args } if args.is_empty() => {
                if let Some(ident) = path.as_ident() {
                    if ident.as_str() == "Self" {
                        return self.self_type.clone();
                    }
                }
                None
            }
            Type::Generic { name, args } if args.is_empty() && name.as_str() == "Self" => {
                self.self_type.clone()
            }
            _ => None,
        }
    }

    /// Register a collection type name that supports array literal coercion.
    /// Array literals `[1, 2, 3]` will unify with `TypeName<T>` for any registered name.
    /// "List" is registered by default.
    pub fn register_array_coercible_type(&mut self, type_name: Text) {
        self.array_coercible_types.insert(type_name);
    }

    /// Walk `name` through any registered type aliases to its
    /// terminal head name — the head of the deepest non-alias
    /// `Named` target.  Returns `None` when `name` has no alias
    /// chain (caller falls back to the literal-name lookup).
    ///
    /// **Bounded by the alias-cycle invariant** enforced at
    /// `register_type_alias` (chains are guaranteed acyclic), so
    /// the loop terminates without an explicit step counter.
    /// **Type-only follow** — only walks `Named { path, .. }`
    /// targets; an alias whose target is some non-named shape
    /// (Tuple, Function, …) returns the previous step's head.
    ///
    /// Common cases:
    ///   * `Tensor<T> is DynTensor<T>` → `"Tensor"` resolves to
    ///     `"DynTensor"` so `is_tensor_family("Tensor")` returns
    ///     `true` via DynTensor's `implement TensorLike`.
    ///   * Darwin `SockaddrIn = DarwinSockaddrIn` → `"SockaddrIn"`
    ///     resolves to `"DarwinSockaddrIn"` so
    ///     `is_bytewise_ffi("SockaddrIn")` returns `true` via
    ///     DarwinSockaddrIn's `implement BytewiseFfi`.
    ///
    /// Both seams previously required a 1-entry hardcoded
    /// fallback (`TENSOR_FAMILY_STDLIB_NAMES = ["Tensor"]`,
    /// `BYTEWISE_FFI_STDLIB_NAMES = ["SockaddrIn"]`); the
    /// alias-aware lookup eliminates them entirely — the literal
    /// alias name is checked first, and only falls through to
    /// the resolved head when the direct lookup misses.
    /// Register a protocol's resolved declaration key.
    pub fn register_protocol_name(&mut self, name: &str) {
        self.protocol_names.insert(Text::from(name));
    }

    pub(crate) fn protocol_name_is_registered(&self, name: &Text) -> bool {
        self.protocol_names.contains(name)
    }

    pub(crate) fn restore_protocol_name(&mut self, name: Text, was_registered: bool) {
        if was_registered { self.protocol_names.insert(name); }
        else { self.protocol_names.remove(&name); }
    }

    pub fn resolve_aliased_head_text(&self, name: &str) -> Option<String> {
        self.resolve_aliased_head_name(name)
    }

    fn resolve_aliased_head_name(&self, name: &str) -> Option<String> {
        let mut current = Text::from(name);
        let mut step = false;
        loop {
            let target = match self.type_aliases.get(&current) {
                Some(t) => t,
                None => return if step { Some(current.to_string()) } else { None },
            };
            let head = match target {
                Type::Named { path, .. } => path.segments.last().and_then(|seg| {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                        _ => None,
                    }
                }),
                // `parse_descriptor_type_string` produces `Type::Generic`
                // for any alias target with type arguments (`Bytes = List<Byte>`
                // → `Generic { name: "List", args: [Byte] }`). Without this
                // arm, every alias to an instantiated collection type
                // failed `is_array_coercible` / sibling family-check lookups
                // because the head walk fell through `_ => None`.
                // Concrete impact: `let bs: Bytes = [1, 2, 3]` failed to
                // coerce the `[Int; 3]` array literal to `List<Byte>`
                // (whose `ArrayCoercible` impl is registered) — surfaced
                // as `expected 'List<Byte>', found '[Byte; 3]'` across
                // base/mod + base/primitives + base/protocols clusters.
                Type::Generic { name, .. } => Some(name.as_str()),
                _ => None,
            };
            match head {
                Some(h) if h != current.as_str() => {
                    current = Text::from(h);
                    step = true;
                }
                _ => return if step { Some(current.to_string()) } else { None },
            }
        }
    }

    /// Helper: check `name` against `set`, falling back to the
    /// alias-resolved head name. Centralises the alias-aware
    /// pattern across the six family-check methods so the alias
    /// rule is structurally identical for every coercion marker.
    #[inline]
    fn family_contains(
        &self,
        set: &std::collections::HashSet<Text>,
        name: &str,
    ) -> bool {
        if set.contains(name) {
            return true;
        }
        match self.resolve_aliased_head_name(name) {
            Some(resolved) => set.contains(resolved.as_str()),
            None => false,
        }
    }

    /// Check whether a type name supports array coercion (data-driven).
    fn is_array_coercible(&self, name: &str) -> bool {
        self.family_contains(&self.array_coercible_types, name)
    }

    /// Return the element of an already registered array-coercible collection.
    /// This uses the same head predicates as array/slice unification so checker
    /// authority checks do not infer coercibility from a name or record shape.
    pub(crate) fn array_coercible_element<'a>(&self, ty: &'a Type) -> Maybe<&'a Type> {
        match ty {
            Type::Generic { name, args }
                if args.len() == 1 && self.is_array_coercible(name.as_str()) =>
            {
                Some(&args[0])
            }
            Type::Named { path, args }
                if args.len() == 1 && self.path_is_array_coercible(path) =>
            {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Register a type name as part of the tensor family (coerces with other tensor
    /// types and with scalar primitives). Default set includes DynTensor, Tensor, etc.
    pub fn register_tensor_family_type(&mut self, type_name: Text) {
        self.tensor_family_types.insert(type_name);
    }

    /// Check whether a type name is in the tensor family
    /// (alias-aware): the literal name is checked first, with
    /// fallback to the alias-resolved head — so `Tensor<T>` (alias
    /// for `DynTensor<T>`) inherits TensorLike via DynTensor's
    /// `implement` block without a parallel hardcoded entry.
    fn is_tensor_family(&self, name: &str) -> bool {
        self.family_contains(&self.tensor_family_types, name)
    }

    /// Register a Named type name that coerces bidirectionally with `Int`.
    pub fn register_int_coercible_type(&mut self, type_name: Text) {
        self.int_coercible_named_types.insert(type_name);
    }

    /// Check whether a Named type name coerces bidirectionally
    /// with `Int` (alias-aware).
    ///
    /// Two-arm classification:
    /// 1. **Primitive integer aliases** (`Int8`..`Int128`, `UInt`..`UInt128`,
    ///    `USize`/`ISize`/`IntSize`/`UIntSize`, `Byte`, plus all
    ///    legacy uppercase-short and Rust-style lowercase aliases) are
    ///    classified through the canonical
    ///    `verum_common::well_known_types::type_names::is_integer_type`
    ///    registry — drift-pinned by `NUMERIC_ALIAS_MATRIX`. The seed
    ///    HashSet doesn't carry them.
    /// 2. **Stdlib types with `implement IntCoercible for X {}` blocks**
    ///    (Duration, Instant, etc.) feed names into the HashSet via
    ///    `register_int_coercible_type`. Those are queried through
    ///    `family_contains` (alias-aware via the family normalisation
    ///    layer).
    fn is_int_coercible_named(&self, name: &str) -> bool {
        verum_common::well_known_types::type_names::is_integer_type(name)
            || self.family_contains(&self.int_coercible_named_types, name)
    }

    /// Compact one-line rendering of a type's SHAPE, for `VERUM_TRACE_UNIFY`.
    ///
    /// Deliberately lossy: head name plus type arguments, recursively, and a
    /// bare variant tag for everything else. Spans, paths and payload
    /// details are dropped — what a unification trace has to answer is
    /// "which two shapes were compared", and the `{:?}` form buries that
    /// under source offsets.
    fn shape_of(t: &Type) -> String {
        use Type::*;
        let args = |a: &List<Type>| -> String {
            if a.is_empty() {
                String::new()
            } else {
                let inner: Vec<String> = a.iter().map(Self::shape_of).collect();
                format!("<{}>", inner.join(", "))
            }
        };
        match t {
            Unit => "Unit".into(),
            Never => "Never".into(),
            Unknown => "Unknown".into(),
            Bool => "Bool".into(),
            Int => "Int".into(),
            Float => "Float".into(),
            Char => "Char".into(),
            Text => "Text".into(),
            Var(v) => format!("?{v:?}"),
            Named { path, args: a } => {
                let head = path
                    .segments
                    .last()
                    .and_then(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "?path".to_string());
                format!("{head}{}", args(a))
            }
            Generic { name, args: a } => format!("{name}{}", args(a)),
            Tuple(items) => {
                let inner: Vec<String> = items.iter().map(Self::shape_of).collect();
                format!("({})", inner.join(", "))
            }
            other => {
                // Variant tag only. Keeps the probe total without enumerating
                // a 40-arm enum that grows independently of this instrument.
                let d = format!("{other:?}");
                d.split(['(', ' ', '{']).next().unwrap_or("?").to_string()
            }
        }
    }

    /// Register a type as range-like (supports tuple/slice coercion).
    /// Called from stdlib type registration for types implementing RangeLike.
    pub fn register_range_like_type(&mut self, type_name: Text) {
        self.range_like_types.insert(type_name);
    }

    /// Check whether a Generic type coerces with `Tuple`
    /// (range/slice pattern — alias-aware).
    fn is_range_like(&self, name: &str) -> bool {
        self.family_contains(&self.range_like_types, name)
    }

    /// Register a type as sized numeric (cross-coerces with other sized numerics).
    /// Called from stdlib type registration for additional numeric types
    /// (Duration, Instant, etc.) beyond language-level primitives.
    pub fn register_sized_numeric_type(&mut self, type_name: Text) {
        self.sized_numeric_types.insert(type_name);
    }

    /// Check whether a Named type is a sized numeric that
    /// cross-coerces with other sized numerics (alias-aware).
    ///
    /// Primitive numerics (every Int / UInt / Float / Byte width)
    /// flow through `verum_common::well_known_types::type_names::is_numeric_type`
    /// — the canonical registry — and stdlib opt-ins
    /// (Duration, Instant, etc.) flow through the dynamically-populated
    /// HashSet. Drift between the two layers is structurally impossible
    /// since each name is classified by exactly one mechanism (predicate
    /// or registered set), and the predicate is alias-matrix-pinned.
    fn is_sized_numeric(&self, name: &str) -> bool {
        verum_common::well_known_types::type_names::is_numeric_type(name)
            || self.family_contains(&self.sized_numeric_types, name)
    }

    /// Register a type as bytewise-FFI compatible — its in-memory
    /// layout is a packed-C-struct byte mirror of a kernel /
    /// libsystem ABI (sockaddr family, etc.) and the unifier
    /// should accept it against `[Byte]` / `[UInt8]` byte buffers
    /// at FFI boundaries.  Called from stdlib type registration
    /// (`scan_protocol_implementations`) when a type carries
    /// `implement BytewiseFfi for X {}`.
    pub fn register_bytewise_ffi_type(&mut self, type_name: Text) {
        self.bytewise_ffi_types.insert(type_name);
    }

    /// True iff `name` (or the head of its alias chain) is
    /// registered as a packed-C-struct byte mirror compatible
    /// with `[Byte]` / `[UInt8]` at FFI boundaries (alias-aware
    /// — `SockaddrIn` on Darwin resolves to `DarwinSockaddrIn`).
    /// Replaces a hardcoded `matches!(name, "Sockaddr" |
    /// "SocketAddr" | "SockaddrIn")` short-circuit previously
    /// inline in the Named↔Named unification arm — drift between
    /// the unifier's accept set and the stdlib's declared
    /// `implement BytewiseFfi` blocks is now structurally
    /// impossible.
    fn is_bytewise_ffi(&self, name: &str) -> bool {
        self.family_contains(&self.bytewise_ffi_types, name)
    }

    /// Set the variant type names registry for stdlib-agnostic unification.
    /// This should be called when initializing the unifier with type context data.
    pub fn set_variant_type_names(&mut self, registry: Map<Text, Text>) {
        if let Some(journal) = &mut self.variant_registration_journal {
            for signature in self.variant_type_names.keys().chain(registry.keys()) {
                journal.names.entry(signature.clone())
                    .or_insert_with(|| self.variant_type_names.get(signature).cloned());
            }
        }
        self.variant_type_names = registry;
    }

    /// Register a variant type name mapping.
    pub fn register_variant_type_name(&mut self, signature: Text, type_name: Text) {
        if let Some(journal) = &mut self.variant_registration_journal {
            journal.names.entry(signature.clone())
                .or_insert_with(|| self.variant_type_names.get(&signature).cloned());
        }
        // Use entry().or_insert to keep FIRST registration (stdlib types registered first
        // should take precedence). This ensures deterministic variant resolution
        // regardless of module registration order.
        self.variant_type_names
            .entry(signature)
            .or_insert(type_name);
    }

    /// Register the original (unsubstituted) variant type definition.
    /// This is needed for Generic<->Variant unification to properly map type arguments.
    pub fn register_original_variant_type(&mut self, type_name: Text, original_type: Type) {
        if let Some(journal) = &mut self.variant_registration_journal {
            journal.originals.entry(type_name.clone())
                .or_insert_with(|| self.original_variant_types.get(&type_name).cloned());
        }
        self.original_variant_types.insert(type_name, original_type);
    }

    /// Legacy type-only metadata delegates to the same positional telescope.
    pub fn register_type_var_order(&mut self, type_name: Text, order: List<TypeVar>) {
        self.register_variant_parameters(type_name, order.into_iter().map(Type::Var).collect());
    }

    pub(crate) fn register_variant_parameters(&mut self, type_name: Text, parameters: List<Type>) {
        if let Some(journal) = &mut self.variant_registration_journal {
            journal.orders.entry(type_name.clone())
                .or_insert_with(|| self.variant_parameters.get(&type_name).cloned());
        }
        self.variant_parameters.insert(type_name, parameters);
    }

    /// The declaration owns both the sum body and its parameter telescope.
    /// Consumers must instantiate them together, without reconstructing slots
    /// from payloads or depending on a temporary value scope.
    pub(crate) fn declared_variant_template(&self, name: &Text) -> Option<(&List<Type>, &Type)> {
        Some((self.variant_parameters.get(name)?, self.original_variant_types.get(name)?))
    }

    /// Match an instance against its declared body. Instantiating the telescope
    /// handles unused, reordered and value slots without guessing from payloads.
    fn unify_declared_variant(
        &mut self, name: &Text, args: &[Type], variants: &indexmap::IndexMap<Text, Type>, span: Span,
    ) -> Option<Result<Substitution>> {
        let parameters = self.variant_parameters.get(name)?.clone();
        let body = self.original_variant_types.get(name)?.clone();
        Some((|| {
            let invalid = |message| TypeError::UniverseBinding { message, span };
            if parameters.len() != args.len() {
                return Err(invalid("sum application differs from its declared arity".into()));
            }
            let binders = parameters.iter().map(crate::context::GenericBinder::declared)
                .collect::<std::result::Result<List<_>, _>>().map_err(invalid)?;
            let fresh = crate::context::instantiate_binders(&binders, &[]).map_err(invalid)?;
            let mut scope = Substitution::new();
            for (parameter, argument) in parameters.iter().zip(args) {
                if parameter == argument { continue; }
                match (parameter, argument) {
                    (Type::Var(variable), argument) if !matches!(argument, Type::Meta { .. }) => {
                        scope.insert(*variable, argument.clone());
                    }
                    (Type::Meta { value: crate::ty::ValueTerm::Parameter(id), .. }, Type::Meta { .. }) => {
                        scope.value_bindings.insert(*id, argument.clone());
                    }
                    _ => return Err(invalid("sum argument has the wrong parameter kind".into())),
                }
            }
            let expected = body.try_apply_subst(&scope).map_err(invalid)?;
            // A constructor's template must not bind the shared declaration when
            // matched against an instance. Only this declaration's slots freshen.
            let actual = Type::Variant(variants.clone()).try_apply_subst(&fresh).map_err(invalid)?;
            self.unify_inner(&expected, &actual, span)
        })())
    }

    pub(crate) fn begin_variant_registration_journal(&mut self) {
        debug_assert!(self.variant_registration_journal.is_none());
        self.variant_registration_journal = Some(VariantRegistrationJournal::default());
    }

    pub(crate) fn finish_variant_registration_journal(&mut self, commit: bool) {
        let Some(journal) = self.variant_registration_journal.take() else { return; };
        if commit { return; }
        for (name, previous) in journal.names {
            match previous {
                Some(value) => { self.variant_type_names.insert(name, value); }
                None => { self.variant_type_names.remove(&name); }
            }
        }
        for (name, previous) in journal.originals {
            match previous {
                Some(value) => { self.original_variant_types.insert(name, value); }
                None => { self.original_variant_types.remove(&name); }
            }
        }
        for (name, previous) in journal.orders {
            match previous {
                Some(value) => { self.variant_parameters.insert(name, value); }
                None => { self.variant_parameters.remove(&name); }
            }
        }
    }

    /// Register a type alias mapping (e.g., "Byte" -> UInt8 Named type).
    ///
    /// Performs cycle detection: registering `type A is B` when a chain
    /// B -> ... -> A already exists is rejected so that `try_expand_alias`
    /// cannot produce infinite recursion or unreachable types.
    pub fn register_type_alias(&mut self, alias_name: Text, target: Type) {
        if self.alias_creates_cycle(&alias_name, &target) {
            // Silently drop cyclic registrations. Caller receives no new
            // alias, preserving the already-registered target. Cycle
            // detection at type-check time will report the user-visible
            // diagnostic via a dedicated pass.
            return;
        }
        Arc::make_mut(&mut self.type_aliases).insert(alias_name, target);
    }

    pub(crate) fn into_type_aliases(self) -> Arc<Map<Text, Type>> {
        self.type_aliases
    }

    /// Returns true if registering `alias -> target` would create a cycle
    /// with existing aliases (e.g., `type A is B; type B is A`).
    fn alias_creates_cycle(&self, alias: &Text, target: &Type) -> bool {
        let mut visited: std::collections::HashSet<Text> = std::collections::HashSet::new();
        visited.insert(alias.clone());
        self.type_mentions_alias(target, &mut visited)
    }

    fn type_mentions_alias(
        &self,
        ty: &Type,
        visited: &mut std::collections::HashSet<Text>,
    ) -> bool {
        match ty {
            Type::Named { path, args } => {
                let name = path.segments.last().and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => {
                        Some(Text::from(ident.name.as_str()))
                    }
                    _ => None,
                });
                if let Some(n) = name {
                    if visited.contains(&n) {
                        return true;
                    }
                    if let Some(next) = self.type_aliases.get(&n) {
                        visited.insert(n);
                        if self.type_mentions_alias(next, visited) {
                            return true;
                        }
                    }
                }
                for arg in args {
                    if self.type_mentions_alias(arg, visited) {
                        return true;
                    }
                }
                false
            }
            Type::Reference { inner, .. } => self.type_mentions_alias(inner, visited),
            Type::Generic { args, .. } => {
                for arg in args {
                    if self.type_mentions_alias(arg, visited) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Register type parameter names for a generic type alias.
    /// E.g., for `type IoResult<T> is Result<T, StreamError>`, register ["T"].
    /// This enables try_expand_alias to substitute type arguments for generic aliases.
    /// The declared parameter names of a registered generic alias
    /// (`IoResult` → `["T"]`), for positional substitution at sites
    /// that never see a `__type_params_` registration (T0862).
    pub fn alias_param_names(&self, alias_name: &str) -> Option<&List<Text>> {
        self.type_alias_params.get(&Text::from(alias_name))
    }

    pub fn register_type_alias_params(&mut self, alias_name: Text, params: List<Text>) {
        self.type_alias_params.insert(alias_name, params);
    }

    /// Preserve one lexical alias while a foreign declaration is registered.
    /// Qualified alias keys are independent and remain available to its values.
    pub(crate) fn alias_binding(&self, name: &str) -> (Option<Type>, Option<List<Text>>) {
        let key = Text::from(name);
        (self.type_aliases.get(&key).cloned(), self.type_alias_params.get(&key).cloned())
    }

    pub(crate) fn restore_alias_binding(
        &mut self,
        name: Text,
        binding: (Option<Type>, Option<List<Text>>),
    ) {
        match binding.0 {
            Some(target) => { Arc::make_mut(&mut self.type_aliases).insert(name.clone(), target); }
            None => { Arc::make_mut(&mut self.type_aliases).remove(&name); }
        }
        match binding.1 {
            Some(parameters) => { self.type_alias_params.insert(name, parameters); }
            None => { self.type_alias_params.remove(&name); }
        }
    }

    /// Remove a type alias (used when user types override stdlib aliases)
    pub fn remove_type_alias(&mut self, alias_name: &str) {
        let key: Text = alias_name.into();
        Arc::make_mut(&mut self.type_aliases).remove(&key);
    }

    /// Recursively expand type aliases in a type.
    /// Handles direct aliases (Byte -> UInt8), generic types (List<Byte> -> List<UInt8>),
    /// and reference types (&mut List<Byte> -> &mut List<UInt8>).
    /// Normalize only registered transparent aliases. Nominal records retain
    /// their declaration identity; nested arguments use the same alias walker.
    pub(crate) fn normalize_aliases(&self, ty: &Type) -> Type {
        let mut current = ty.clone();
        for _ in 0..MAX_ALIAS_EXPANSION_DEPTH {
            let Some(next) = self.try_expand_alias(&current) else {
                break;
            };
            if next == current {
                break;
            }
            current = next;
        }
        current
    }

    pub fn try_expand_alias(&self, ty: &Type) -> Option<Type> {
        self.try_expand_alias_impl(ty, 0)
    }

    fn try_expand_alias_impl(&self, ty: &Type, depth: usize) -> Option<Type> {
        if depth > MAX_ALIAS_EXPANSION_DEPTH {
            return None;
        }
        match ty {
            // Direct Named type alias (e.g., Byte -> UInt8)
            Type::Named { path, args } => {
                // Qualified aliases belong to their declaration; a sibling's
                // bare alias is not evidence for an unresolved full path.
                let parts: Option<List<&str>> = path
                    .segments
                    .iter()
                    .map(|segment| match segment {
                        verum_ast::ty::PathSegment::Name(name) => Some(name.name.as_str()),
                        _ => None,
                    })
                    .collect();
                if let Some(parts) = parts {
                    let key: Text = parts.join(".").into();
                    if let Some(target) = self.type_aliases.get(&key) {
                        if args.is_empty() {
                            // Simple alias: Byte -> UInt8
                            return Some(target.clone());
                        }
                        // Generic alias: IoResult<Text> -> Result<Text, StreamError>
                        // Substitute positional type arguments into the alias target.
                        if let Some(param_names) = self.type_alias_params.get(&key) {
                            if !param_names.is_empty() && args.len() <= param_names.len() {
                                let substituted =
                                    self.substitute_alias_params(target, param_names, args);
                                return Some(substituted);
                            }
                        }
                    }
                }
                // Try expanding aliases in type arguments (e.g., List<Byte> -> List<UInt8>)
                let mut changed = false;
                let d = depth + 1;
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| {
                        if let Some(expanded) = self.try_expand_alias_impl(arg, d) {
                            changed = true;
                            expanded
                        } else {
                            arg.clone()
                        }
                    })
                    .collect();
                if changed {
                    Some(Type::Named {
                        path: path.clone(),
                        args: new_args,
                    })
                } else {
                    None
                }
            }
            // Expand through references: &T, &mut T, &checked T, etc.
            Type::Reference { inner, mutable } => {
                self.try_expand_alias_impl(inner, depth + 1)
                    .map(|expanded| Type::Reference {
                        inner: Box::new(expanded),
                        mutable: *mutable,
                    })
            }
            Type::CheckedReference { inner, mutable } => self
                .try_expand_alias_impl(inner, depth + 1)
                .map(|expanded| Type::CheckedReference {
                    inner: Box::new(expanded),
                    mutable: *mutable,
                }),
            Type::Ownership { inner, mutable } => {
                self.try_expand_alias_impl(inner, depth + 1)
                    .map(|expanded| Type::Ownership {
                        inner: Box::new(expanded),
                        mutable: *mutable,
                    })
            }
            // Handle Generic type aliases (e.g., IoResult<Text> as Generic { name: "IoResult", args: [Text] })
            Type::Generic { name, args } => {
                let key = name.clone();
                if let Some(target) = self.type_aliases.get(&key) {
                    if args.is_empty() {
                        return Some(target.clone());
                    }
                    if let Some(param_names) = self.type_alias_params.get(&key) {
                        if !param_names.is_empty() && args.len() <= param_names.len() {
                            let substituted =
                                self.substitute_alias_params(target, param_names, args);
                            return Some(substituted);
                        }
                    }
                }
                // Try expanding aliases in type arguments
                let mut changed = false;
                let d = depth + 1;
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| {
                        if let Some(expanded) = self.try_expand_alias_impl(arg, d) {
                            changed = true;
                            expanded
                        } else {
                            arg.clone()
                        }
                    })
                    .collect();
                if changed {
                    Some(Type::Generic {
                        name: name.clone(),
                        args: new_args,
                    })
                } else {
                    None
                }
            }
            Type::Function {
                property_requirement,
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let mut changed = false;
                let params = params
                    .iter()
                    .map(|ty| {
                        if let Some(expanded) = self.try_expand_alias_impl(ty, depth + 1) {
                            changed = true;
                            expanded
                        } else {
                            ty.clone()
                        }
                    })
                    .collect();
                let return_type =
                    if let Some(expanded) = self.try_expand_alias_impl(return_type, depth + 1) {
                        changed = true;
                        Box::new(expanded)
                    } else {
                        return_type.clone()
                    };
                changed.then(|| Type::Function {
                    property_requirement: property_requirement.clone(),
                    params,
                    return_type,
                    type_params: type_params.clone(),
                    contexts: contexts.clone(),
                    properties: properties.clone(),
                })
            }
            Type::Array { element, size } => {
                self.try_expand_alias_impl(element, depth + 1)
                    .map(|element| Type::Array {
                        element: Box::new(element),
                        size: *size,
                    })
            }
            Type::Slice { element } => {
                self.try_expand_alias_impl(element, depth + 1)
                    .map(|element| Type::Slice {
                        element: Box::new(element),
                    })
            }
            // Expand through tuples
            Type::Tuple(elements) => {
                let mut changed = false;
                let d = depth + 1;
                let new_elements: List<Type> = elements
                    .iter()
                    .map(|el| {
                        if let Some(expanded) = self.try_expand_alias_impl(el, d) {
                            changed = true;
                            expanded
                        } else {
                            el.clone()
                        }
                    })
                    .collect();
                if changed {
                    Some(Type::Tuple(new_elements))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Substitute type parameter names in an alias target with concrete type arguments.
    /// For `type IoResult<T> is Result<T, StreamError>`, given args=[Text],
    /// replaces all occurrences of type variable "T" with Text in the target type.
    fn substitute_alias_params(&self, target: &Type, param_names: &[Text], args: &[Type]) -> Type {
        // Build a name->type substitution map
        let subst_map: Map<Text, Type> = param_names
            .iter()
            .zip(args.iter())
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect();

        self.apply_alias_subst(target, &subst_map)
    }

    /// Recursively apply a name-based substitution to a type.
    ///
    /// Exposed to the crate because binding a generic impl's parameters
    /// from a receiver (`implement<T> Deref for G<T>` seen as `G<Inner>`)
    /// is the same operation as substituting an alias's parameters, and a
    /// second implementation of it is how the `Result<Unit>` class was
    /// born one layer up.
    pub(crate) fn apply_alias_subst(&self, ty: &Type, subst: &Map<Text, Type>) -> Type {
        // Substituting a parameter map into a type is a property of the
        // TYPE, not of unification state (the body never read `self`),
        // and three subsystems need it.  It lives on `Type` next to
        // `apply_arguments`; this stays as the unifier-facing spelling.
        ty.substitute_params(subst)
    }


    /// Generate a stable signature for a variant type (sorted variant names with payload types).
    ///
    /// IMPORTANT: Must produce identical signatures to `variant_type_signature()` in infer.rs
    /// and `variant_type_signature_static()` in protocol.rs.
    fn variant_type_signature(variants: &IndexMap<Text, Type>) -> Text {
        let mut entries: Vec<String> = variants
            .iter()
            .map(|(name, payload)| {
                let payload_name = match payload {
                    Type::Named { path, .. } => path
                        .as_ident()
                        .map(|id| id.name.as_str().to_string())
                        .unwrap_or_default(),
                    Type::Generic { name: n, .. } => n.as_str().to_string(),
                    // Unit, primitives, and TypeVars are not distinctive for
                    // disambiguation — only Named/Generic payload types matter.
                    _ => String::new(),
                };
                if payload_name.is_empty() {
                    name.as_str().to_string()
                } else {
                    format!("{}({})", name.as_str(), payload_name)
                }
            })
            .collect();
        entries.sort();
        format!("Variant({})", entries.join("|")).into()
    }

    /// Generate a relaxed variant type signature using only variant names (ignoring payload types).
    /// Used as fallback when the full signature doesn't match due to concrete type arguments.
    fn variant_type_signature_relaxed(variants: &IndexMap<Text, Type>) -> Text {
        let mut names: Vec<&str> = variants.keys().map(|k| k.as_str()).collect();
        names.sort();
        // NAMESPACE — see the doc on
        // `TypeChecker::variant_type_signature_relaxed` in
        // infer/modules.rs.  All three copies of this pair must
        // agree on the spelling: registration writes through one
        // and lookup reads through another (T1069).
        format!("VariantRelaxed({})", names.join("|")).into()
    }

    /// Get the accumulated substitution established by successful unifications.
    pub fn get_substitution(&self) -> &Substitution {
        &self.substitution
    }

    /// Get context variable bindings accumulated during unification.
    pub fn get_context_bindings(&self) -> &IndexMap<TypeVar, crate::di::requirement::ContextExpr> {
        &self.context_bindings
    }

    /// Apply the current accumulated substitution to a type,
    /// also resolving context variable bindings.
    pub fn apply(&self, ty: &Type) -> Type {
        self.try_apply(ty, Span::default())
            .expect("internal unifier application must have been validated")
    }

    /// Apply on a source-checking path, preserving substitution diagnostics.
    pub fn try_apply(&self, ty: &Type, span: Span) -> Result<Type> {

        let resolved = ty.try_apply_subst(&self.substitution).map_err(|message| TypeError::UniverseBinding { message, span })?;
        // Also resolve context variables in function types
        Ok(if !self.context_bindings.is_empty() {
            self.resolve_context_vars(&resolved)
        } else {
            resolved
        })
    }

    /// Save the current substitution state for tentative unification.
    /// Returns a snapshot that can be restored via `restore_substitution`.
    pub fn save_substitution(&self) -> Substitution {
        self.substitution.clone()
    }

    /// Restore a previously saved substitution state.
    /// Used to roll back tentative unification attempts.
    pub fn restore_substitution(&mut self, saved: Substitution) {
        self.substitution = saved;
    }

    /// Resolve context variables in a type using context_bindings
    fn resolve_context_vars(&self, ty: &Type) -> Type {
        match ty {
            Type::Function {
                property_requirement,
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => {
                let resolved_contexts = contexts
                    .as_ref()
                    .map(|ctx| ctx.apply_context_subst(&self.context_bindings));
                Type::Function {
                    property_requirement: property_requirement.clone(),
                    params: params
                        .iter()
                        .map(|p| self.resolve_context_vars(p))
                        .collect(),
                    return_type: Box::new(self.resolve_context_vars(return_type)),
                    type_params: type_params.clone(),
                    contexts: resolved_contexts,
                    properties: properties.clone(),
                }
            }
            _ => ty.clone(),
        }
    }

    /// Reset the accumulated substitution (e.g., for new function scope)
    pub fn reset_substitution(&mut self) {
        self.substitution = Substitution::new();
    }

    /// Helper to check if a path represents an array-coercible collection type.
    /// Uses the data-driven `array_coercible_types` set instead of hardcoding "List".
    /// Will be replaced with a `FromArray` protocol check once protocols are available.
    fn path_is_array_coercible(&self, path: &verum_ast::ty::Path) -> bool {
        if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
            self.is_array_coercible(ident.name.as_str())
        } else {
            false
        }
    }

    /// Create a constructor kind for N parameters: * -> * -> ... -> *
    /// For arity 0: Type (*)
    /// For arity 1: * -> * (unary constructor like List)
    /// For arity 2: * -> * -> * (binary constructor like Map)
    fn kind_for_arity(arity: usize) -> crate::advanced_protocols::Kind {
        use crate::advanced_protocols::Kind;
        let mut kind = Kind::Type;
        for _ in 0..arity {
            kind = Kind::arrow(Kind::Type, kind);
        }
        kind
    }

    /// Helper to collect all type variables from a type.
    /// Used for building substitutions when unifying Named types with Variant types.
    fn collect_type_vars_from_type(ty: &Type, vars: &mut Vec<TypeVar>) {
        use Type::*;
        match ty {
            Var(tv) => {
                if !vars.contains(tv) {
                    vars.push(*tv);
                }
            }
            Named { args, .. } | Generic { args, .. } => {
                for arg in args {
                    Self::collect_type_vars_from_type(arg, vars);
                }
            }
            Tuple(elems) => {
                for elem in elems {
                    Self::collect_type_vars_from_type(elem, vars);
                }
            }
            Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    Self::collect_type_vars_from_type(param, vars);
                }
                Self::collect_type_vars_from_type(return_type, vars);
            }
            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. } => {
                Self::collect_type_vars_from_type(inner, vars);
            }
            Array { element, .. } | Slice { element } => {
                Self::collect_type_vars_from_type(element, vars);
            }
            Variant(variant_map) | Record(variant_map) => {
                for (_, payload) in variant_map {
                    Self::collect_type_vars_from_type(payload, vars);
                }
            }
            _ => {}
        }
    }

    /// Check if a type contains any unresolved type variables.
    ///
    /// This is used during associated type projection unification to determine
    /// whether a projection can be resolved or must be deferred.
    ///
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — Associated type resolution
    fn has_type_vars(ty: &Type) -> bool {
        use Type::*;
        match ty {
            Var(_) => true,
            Named { args, .. } | Generic { args, .. } => args.iter().any(Self::has_type_vars),
            Tuple(elems) => elems.iter().any(Self::has_type_vars),
            Function {
                params,
                return_type,
                ..
            } => params.iter().any(Self::has_type_vars) || Self::has_type_vars(return_type),
            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. } => Self::has_type_vars(inner),
            Array { element, .. } | Slice { element } => Self::has_type_vars(element),
            Variant(map) | Record(map) => map.values().any(Self::has_type_vars),
            Future { output } => Self::has_type_vars(output),
            Generator {
                yield_ty,
                return_ty,
            } => Self::has_type_vars(yield_ty) || Self::has_type_vars(return_ty),
            Refined { base, .. } => Self::has_type_vars(base),
            Forall { body, .. } | Exists { body, .. } => Self::has_type_vars(body),
            // Primitive types have no type variables
            Unit | Never | Bool | Int | Float | Char | Text => false,
            // Others - conservative: assume no vars
            _ => false,
        }
    }

    /// Unify two context expressions directly (public API for context polymorphism).
    ///
    /// This is the entry point for callers (e.g., type inference) that need to unify
    /// context expressions from callback types without going through full function type
    /// unification. For example, when inferring `C` in:
    ///
    /// ```verum
    /// fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter<T,U> using C
    /// ```
    ///
    /// The inference engine can call `unify_context_exprs(C_var, concrete_ctx)` to bind
    /// the context variable `C` to the concrete context requirement of the callback `f`.
    ///
    /// Context polymorphism rules:
    /// - Variable vs Concrete: binds variable to the concrete requirement
    /// - Variable vs Variable: binds one to the other (if distinct)
    /// - Concrete vs Concrete: must be equal
    pub fn unify_context_exprs(
        &mut self,
        a: &crate::di::requirement::ContextExpr,
        b: &crate::di::requirement::ContextExpr,
        span: Span,
    ) -> crate::Result<()> {
        use crate::di::requirement::ContextExpr;

        match (a, b) {
            // Variable binds to concrete
            (ContextExpr::Variable(var), concrete @ ContextExpr::Concrete(_))
            | (concrete @ ContextExpr::Concrete(_), ContextExpr::Variable(var)) => {
                // Check for conflicting binding
                if let Some(existing) = self.context_bindings.get(var) {
                    if existing != concrete {
                        return Err(TypeError::ContextMismatch {
                            expected: Text::from(format!("{}", existing)),
                            actual: Text::from(format!("{}", concrete)),
                            span,
                        });
                    }
                }
                self.context_bindings.insert(*var, concrete.clone());
                Ok(())
            }

            // Both variables: bind first to second (if distinct)
            (ContextExpr::Variable(v1), ContextExpr::Variable(v2)) => {
                if v1 != v2 {
                    self.context_bindings
                        .insert(*v1, ContextExpr::Variable(*v2));
                }
                Ok(())
            }

            // Both concrete: must be equal
            (ContextExpr::Concrete(req1), ContextExpr::Concrete(req2)) => {
                if req1 == req2 {
                    Ok(())
                } else {
                    Err(TypeError::ContextMismatch {
                        expected: Text::from(format!("using {}", req2)),
                        actual: Text::from(format!("using {}", req1)),
                        span,
                    })
                }
            }
        }
    }

    /// Unify optional context expressions for context polymorphism (internal).
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism
    ///
    /// Rules:
    /// - None/None: compatible (both pure)
    /// - Concrete/Concrete: must be equal
    /// - Variable/Concrete: bind variable to the concrete requirement
    /// - Concrete/Variable: bind variable to the concrete requirement
    /// - Variable/Variable: unify the type variables
    fn unify_contexts(
        &mut self,
        ctx1: &Option<crate::di::requirement::ContextExpr>,
        ctx2: &Option<crate::di::requirement::ContextExpr>,
        span: verum_ast::span::Span,
    ) -> crate::Result<Substitution> {
        use crate::di::requirement::ContextExpr;

        match (ctx1, ctx2) {
            // Both None (pure): compatible
            (None, None) => Ok(Substitution::new()),

            // One None, one non-empty concrete: not compatible
            (None, Some(ContextExpr::Concrete(req))) if !req.is_empty() => {
                Err(TypeError::ContextMismatch {
                    expected: Text::from(format!("using {}", req)),
                    actual: Text::from("pure (no contexts)"),
                    span,
                })
            }
            (Some(ContextExpr::Concrete(req)), None) if !req.is_empty() => {
                Err(TypeError::ContextMismatch {
                    expected: Text::from("pure (no contexts)"),
                    actual: Text::from(format!("using {}", req)),
                    span,
                })
            }

            // None with empty concrete: compatible
            (None, Some(ContextExpr::Concrete(req))) if req.is_empty() => Ok(Substitution::new()),
            (Some(ContextExpr::Concrete(req)), None) if req.is_empty() => Ok(Substitution::new()),

            // None with variable: variable binds to empty (pure)
            (None, Some(ContextExpr::Variable(var))) => {
                use crate::di::requirement::ContextRequirement;
                self.context_bindings
                    .insert(*var, ContextExpr::Concrete(ContextRequirement::empty()));
                Ok(Substitution::new())
            }
            (Some(ContextExpr::Variable(var)), None) => {
                use crate::di::requirement::ContextRequirement;
                self.context_bindings
                    .insert(*var, ContextExpr::Concrete(ContextRequirement::empty()));
                Ok(Substitution::new())
            }

            // Both Some: delegate to unify_context_exprs
            (Some(a), Some(b)) => {
                self.unify_context_exprs(a, b, span)?;
                Ok(Substitution::new())
            }

            // Catch-all for any remaining cases
            _ => Ok(Substitution::new()),
        }
    }

    /// Unify two types, returning a substitution that makes them equal.
    ///
    /// This is the core of Hindley-Milner type inference.
    /// The algorithm finds the most general unifier (MGU) if one exists.
    ///
    /// CRITICAL FIX: The unifier now:
    /// 1. Applies the accumulated substitution to both types before unifying
    /// 2. Composes the new substitution with the accumulated one
    /// 3. Stores the composed substitution for future unifications
    ///
    /// This ensures that type variables resolved in earlier unifications are
    /// properly reflected in later type comparisons.
    /// T0701: install the associated-type projection resolver (see the
    /// field doc).  Called once by InferenceContext construction.
    pub fn set_projection_resolver(
        &mut self,
        resolver: std::sync::Arc<dyn Fn(&Type, &str) -> Option<Type> + Send + Sync>,
    ) {
        self.projection_resolver = Some(resolver);
    }

    pub(crate) fn set_nominal_identity_resolver(
        &mut self,
        resolver: std::sync::Arc<dyn Fn(&str, &str) -> bool + Send + Sync>,
    ) {
        self.nominal_identity_resolver = Some(resolver);
    }

    pub fn unify(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        self.unify_count += 1;

        // Guard against stack overflow from recursive unification
        self.unify_depth += 1;
        if self.unify_depth > 200 {
            self.unify_depth -= 1;
            return Err(TypeError::Other(verum_common::Text::from(format!(
                "unification depth exceeded ({})",
                self.unify_depth
            ))));
        }
        let result = self.unify_impl(t1, t2, span);
        // VERUM_TRACE_CTOR: pinpoint the raise path for the #31 class — a
        // bare constructor name unified as a nominal type ("found 'Some'").
        // One choke point instead of instrumenting every Mismatch site.
        if self.unify_depth == 1
            && let Err(crate::TypeError::Mismatch { ref actual, .. }) = result
            && crate::ctor_trace_enabled()
            && (actual.as_str() == "Some" || actual.as_str() == "Ok" || actual.as_str() == "Err")
        {
            eprintln!(
                "[ctor-trace] unify MISMATCH actual={:?} t1={:?} t2={:?}\n{}",
                actual,
                t1,
                t2,
                std::backtrace::Backtrace::force_capture()
            );
        }
        // LITERAL-SIZED-ALIAS-COERCE-1 oracle: same choke point, no
        // value filter — every top-level unify mismatch dumps its two
        // types + the RAISE stack under VERUM_TRACE_E400.
        if self.unify_depth == 1
            && let Err(crate::TypeError::Mismatch { .. }) = result
            && std::env::var("VERUM_TRACE_E400").is_ok()
        {
            eprintln!(
                "[e400-unify] t1={:?} t2={:?}\n{}",
                t1,
                t2,
                std::backtrace::Backtrace::force_capture()
            );
        }
        self.unify_depth -= 1;
        result
    }

    fn unify_impl(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        // Guard against infinite unification loops
        const MAX_UNIFY_CALLS: usize = 50_000;
        if self.unify_count > MAX_UNIFY_CALLS {
            return Err(TypeError::Other(verum_common::Text::from(
                "type inference iteration limit exceeded (possible infinite loop)",
            )));
        }

        // Apply current substitution to both types before unifying
        let t1_resolved = t1.try_apply_subst(&self.substitution).map_err(|message| TypeError::UniverseBinding { message, span })?;
        let t2_resolved = t2.try_apply_subst(&self.substitution).map_err(|message| TypeError::UniverseBinding { message, span })?;

        // Unify the resolved types, with type alias fallback.
        // If direct unification fails, try expanding type aliases on either
        // side and retry. This handles cases like `Byte` (alias for `UInt8`)
        // being used interchangeably with `UInt8`.
        let result = self.unify_inner(&t1_resolved, &t2_resolved, span);

        let new_subst = match result {
            Ok(s) => s,
            Err(_) if !self.type_aliases.is_empty() => {
                let t1_expanded = self.try_expand_alias(&t1_resolved);
                let t2_expanded = self.try_expand_alias(&t2_resolved);
                let t1_use = t1_expanded.as_ref().unwrap_or(&t1_resolved);
                let t2_use = t2_expanded.as_ref().unwrap_or(&t2_resolved);
                // Only retry if at least one side was expanded
                if t1_expanded.is_some() || t2_expanded.is_some() {
                    self.unify_inner(t1_use, t2_use, span)?
                } else {
                    return result;
                }
            }
            Err(_) => return result,
        };

        // Compose with accumulated substitution
        self.substitution = self.substitution.compose(&new_subst).map_err(|message| TypeError::UniverseBinding { message, span })?;

        Ok(new_subst)
    }

    fn unify_inner(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        use Type::*;

        // Bound recursive rule expansion, including alias/projection cycles.
        // Rule temporaries live in individual handlers rather than one giant
        // recursive frame. Type substitution uses its own explicit work stack.
        self.unify_depth += 1;
        if self.unify_depth > self.max_unify_depth {
            self.unify_depth -= 1;
            return Err(TypeError::Other(verum_common::Text::from(
                "type inference recursion limit exceeded",
            )));
        }
        let result = self.unify_inner_impl(t1, t2, span);
        self.unify_depth = self.unify_depth.saturating_sub(1);
        // T0722 hunt, verdict edition. `VERUM_TRACE_UNIFY=<substring>` prints
        // the pair AND what the unifier decided about it.
        //
        // The pair alone is not enough, and reading it as if it were cost this
        // investigation two false mechanism stories. A rejected pair whose
        // caller discards the error looks identical, in a pair-only trace, to
        // an accepted one — both appear as "compared, program compiled". Two
        // T0722 sites were exactly that shape: the unifier said Err and
        // something downstream shrugged. `-> Err` next to a program that
        // compiles anyway is the signature to look for.
        if let Ok(want) = std::env::var("VERUM_TRACE_UNIFY") {
            let (a, b) = (Self::shape_of(t1), Self::shape_of(t2));
            if want.is_empty() || a.contains(&want) || b.contains(&want) {
                let verdict = match &result {
                    Ok(_) => "Ok",
                    Err(_) => "Err",
                };
                eprintln!("[unify] t1={a} | t2={b} -> {verdict}");
            }
        }
        result
    }

    /// Override the unify_inner recursion budget (#304). Useful
    /// when embedders running heavy generic-type inference need
    /// to raise the default 50-frame cap, or when fuzzing /
    /// security-sensitive contexts want to lower it. Setting `0`
    /// rejects every unification (the gate fires on the first
    /// frame). Returns `self` for fluent construction.
    pub fn with_max_unify_depth(mut self, max_depth: u32) -> Self {
        self.max_unify_depth = max_depth;
        self
    }

    /// Read mirror of the configured maximum unify recursion
    /// depth. Surfaced as a getter so embedders can confirm
    /// the value the manifest set without exercising the full
    /// unify path.
    pub fn configured_max_unify_depth(&self) -> u32 {
        self.max_unify_depth
    }

    fn unify_inner_impl(&mut self, t1: &Type, t2: &Type, span: Span) -> Result<Substitution> {
        use Type::*;
        let (unification_left, unification_right) = (t1, t2);

        // The pair-only leg of the trace lives in `unify_inner`, which sees the
        // VERDICT too. This one stays for the case that wrapper cannot report:
        // a pair that ENTERS and never returns — a panic, a recursion-limit
        // bail, or a `return` from deep inside the 3000-line match. Enable it
        // with `VERUM_TRACE_UNIFY_ENTRY`; a pair printed here with no matching
        // `[unify] ... ->` line below it did not come back.
        if let Ok(want) = std::env::var("VERUM_TRACE_UNIFY_ENTRY") {
            // Render the SHAPE, not `{:?}`. Every Named/Generic carries a
            // `Span` per segment, and the debug dump spends its first two
            // hundred characters on source offsets — the earlier version of
            // this probe truncated at 120 and printed `Named { path: Path {
            // segments: [Name(Ident { name: Text { inner: "Maybe" }, span:
            // Span { start: 0, end`, cutting off exactly the type ARGUMENTS
            // the probe exists to show. Reading that output, `Maybe<Int>`
            // and `Maybe<Maybe<Int>>` are indistinguishable, which is the
            // one distinction T0722 turns on.
            let (a, b) = (Self::shape_of(t1), Self::shape_of(t2));
            if want.is_empty() || a.contains(&want) || b.contains(&want) {
                eprintln!("[unify-enter] t1={a} | t2={b}");
            }
        }

        // CRITICAL FIX: Resolve symbolic "Self" types before unification.
        // In implement blocks, Self may leak into types that reach the unifier.
        // Replace Self with the concrete implementing type to prevent
        // "expected 'Foo', found 'Self'" mismatches.
        // Route through unify_inner to maintain depth tracking.
        if let Some(resolved) = self.resolve_self(t1) {
            return self.unify_inner(&resolved, t2, span);
        }
        if let Some(resolved) = self.resolve_self(t2) {
            return self.unify_inner(t1, &resolved, span);
        }

        // PROOF IRRELEVANCE (Inductive types: recursive type definitions with structural recursion, termination checking — .1)
        // If both types have type Prop, they unify regardless of their structure.
        // All proofs of a proposition are equal.
        if self.is_type_in_prop(t1) && self.is_type_in_prop(t2)
            && !t1.contains_record_field() && !t2.contains_record_field()
        {
            return Ok(Substitution::new());
        }

        match (t1, t2) {
            // A type read from a value has rigid identity. In particular the
            // permissive bare-name and Unknown arms below cannot identify it
            // with some other receiver's field.
            (RecordField { slot: s1, receiver: r1, level: l1 },
             RecordField { slot: s2, receiver: r2, level: l2 }) if s1 == s2 && r1 == r2 => {
                let mut result = Substitution::new();
                result.universes.unify(l1, l2).map_err(|message| TypeError::UniverseBinding { message, span })?;
                Ok(result)
            }
            (Var(v), field @ RecordField { .. }) | (field @ RecordField { .. }, Var(v)) => {
                self.bind_var(*v, field, span)
            }
            (RecordField { .. }, _) | (_, RecordField { .. }) => Err(TypeError::Mismatch {
                expected: t2.to_text(), actual: t1.to_text(), span,
            }),

            // Equality is not bottom elimination. Diverging expressions may be
            // checked against any expected type, but no inhabited type equals
            // Never (including inside an invariant type argument).
            (Never, Never) => Ok(Substitution::new()),
            (Var(v), Never) | (Never, Var(v)) => self.bind_var(*v, &Never, span),
            (Never, _) | (_, Never) => Err(TypeError::Mismatch {
                expected: t2.to_text(), actual: t1.to_text(), span,
            }),

            // Unknown type (top type) - any value can be assigned to unknown
            // Unknown is a supertype of all types, so T -> Unknown is valid
            // But Unknown -> T requires explicit narrowing (is-check)
            // In unification context, we allow it to avoid false positives
            // since the type checker handles narrowing elsewhere
            (Unknown, other) | (other, Unknown)
                if !other.contains_capability_restriction()
                    && !self.normalize_aliases(other).is_never() => Ok(Substitution::new()),

            // Same types
            (Unit, Unit)
            | (Bool, Bool)
            | (Int, Int)
            | (Float, Float)
            | (Char, Char)
            | (Text, Text) => Ok(Substitution::new()),

            // VBC-MIXED-ARITH-SILENT-NAN-1: the blanket Int~Float
            // "widening" arm was REMOVED — the runtime implements NO
            // numeric coercion (Tier-0 float-extract asserts in debug
            // and reads NaN-boxed Int bits as quiet NaN in release;
            // Tier-1 emits the raw float op), so every unification
            // this arm permitted was a latent silent-NaN. Literal
            // convenience (`f(42)` with `f(x: Float)`) belongs to the
            // LITERAL adaptation paths (check_expr literal-coercion),
            // which type the literal AT the expectation instead of
            // pretending two distinct runtime representations unify.

            // Char → Text widening: a single character can be used where text is expected
            // This allows `let s: Text = 'a';` without explicit conversion.
            (Char, Text) | (Text, Char) => Ok(Substitution::new()),

            // Text and Int are distinct types in Verum. No implicit coercion.
            // Use explicit conversion: `text.parse_int()` or `int.to_text()`.
            // (Text, Int) | (Int, Text) => intentionally NOT unified

            // Sized integer coercion: Int ↔ UInt32/UInt16/Int32/Int16/Int8/UInt8/UInt64
            // Verum's `Int` is the universal integer type (64-bit signed).
            // Sized integers (UInt32, UInt16, etc.) coerce bidirectionally with Int
            // to allow natural usage: `let epoch: UInt32 = 42;` or `assert(epoch == 0);`
            //

            // The lookup is structural via the `IntCoercible` protocol from
            // `core/base/coercion.vr` — populated by
            // `verum_compiler::stdlib_coercion_registry::scan_protocol_implementations`
            // which walks loaded AST modules for `implement IntCoercible for X`
            // blocks. Language-level primitives (UInt8, Int32, U64, ...) are
            // pre-seeded in `Unifier::new`. The HashSet here acts as a fast
            // O(1) cache of the protocol-implementation discovery result —
            // the structural check happens at pipeline-load time.
            (Int, Named { path, .. }) | (Named { path, .. }, Int) => { self.unify_integer_nominal(unification_left, unification_right, span) }

            // Unit and empty tuple are equivalent
            // () is parsed as empty Tuple but semantically equals Unit
            (Unit, Tuple(elements)) | (Tuple(elements), Unit) if elements.is_empty() => {
                Ok(Substitution::new())
            }

            // Unit newtype coercion: `type Foo is ()` makes Foo structurally Unit.
            // When a function returns `()` but the declared return type is a named
            // unit type (like `Database` which is `type Database is ()`), allow the
            // coercion. This is safe because unit newtypes carry no data.
            (Unit, Named { path, args, .. }) | (Named { path, args, .. }, Unit)
                if args.is_empty() =>
            { self.unify_unit_nominal(unification_left, unification_right, span) }

            // Unresolved generic type parameters: Named types with single-letter
            // uppercase names like "T", "U", "E", "K", "V" that haven't been
            // substituted with type variables. Allow them to unify with any
            // concrete type.
            //

            // Heuristic: strictly single-letter uppercase only. Dropping the
            // two-letter case eliminates the need for a hardcoded exclusion
            // list (previously "Ok" | "No" | "Eq" | "Fn" | "IO") while leaving
            // the common-case generic parameters (T, U, A, B, E, F, K, V)
            // identifiable. Two-letter user types (`Io`, `Ex`, etc.) are
            // treated as concrete — the rare user-level generic parameter
            // with a two-letter name must be explicitly bound via
            // `<Xy: Trait>` declarations, which create proper TypeVars that
            // never reach this fallback path. This is the stdlib-agnostic
            // replacement for the previous hardcoded name list per AGENTS.md.
            (Named { path, args, .. }, other) | (other, Named { path, args, .. })
                if args.is_empty() && {
                    let name = path
                        .segments
                        .last()
                        .map(|s| match s {
                            verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                            _ => "",
                        })
                        .unwrap_or("");
                    name.len() == 1
                        && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                        && !matches!(other, Named { .. } if {
                            let oname = match other {
                                Named { path: op, .. } => op.segments.last().map(|s| match s {
                                    verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                                    _ => "",
                                }).unwrap_or(""),
                                _ => "",
                            };
                            oname.len() == 1
                                && oname.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                        })
                } =>
            { self.unify_unbound_nominal_parameter(unification_left, unification_right, span) }

            // Type variables
            (Var(v1), Var(v2)) if v1 == v2 => Ok(Substitution::new()),

            // Two unification variables: the OLDER one survives as the
            // representative. `TypeVar::fresh()` hands out ids from a monotone
            // counter, so a lower id means a wider scope — a declared type
            // parameter is minted when its signature (or `implement` header) is
            // read, and every var created while checking a body that mentions it
            // is minted later.
            //
            // Direction here is not cosmetic. The arm below binds whichever var
            // the caller happened to write on the LEFT, so `unify(T, fresh)`
            // pinned the DECLARED PARAMETER to a body-local variable that
            // nothing goes on to solve. One `implement<T> …` block mints a
            // single var for `T` and shares it across every method
            // (`check_impl_block_inner` in decls.rs), and the accumulated
            // substitution is never reset between bodies — so that inverted bind
            // leaked forward: after `List.from_slice` bound it, `T` read as `_`
            // for every later method of `core/collections/list.vr` (T0742).
            (Var(v1), Var(v2)) => {
                let (younger, older) = if v1.id() >= v2.id() {
                    (v1, v2)
                } else {
                    (v2, v1)
                };
                self.bind_var(*younger, &Type::Var(*older), span)
            }

            (Var(v), ty) | (ty, Var(v)) => self.bind_var(*v, ty, span),

            // Function types
            (
                Function {
                    property_requirement: requirement1,
                    params: p1,
                    return_type: r1,
                    type_params: tp1,
                    contexts: c1,
                    properties: _,
                },
                Function {
                    property_requirement: requirement2,
                    params: p2,
                    return_type: r2,
                    type_params: tp2,
                    contexts: c2,
                    properties: _,
                },
            ) => { self.unify_functions(unification_left, unification_right, span) }

            // Tuple types
            (Tuple(t1s), Tuple(t2s)) => { self.unify_tuples(unification_left, unification_right, span) }

            // Array types
            (
                Array {
                    element: e1,
                    size: s1,
                },
                Array {
                    element: e2,
                    size: s2,
                },
            ) => { self.unify_arrays(unification_left, unification_right, span) }

            // Array -> collection coercion (unification direction)
            // [T; N] can unify with C<T> for any array-coercible collection type C
            // Array literal to collection coercion: [1, 2, 3] infers as List<Int>
            // Uses data-driven array_coercible_types set (defaults to {"List"})
            (
                Array {
                    element: e1,
                    size: _,
                },
                Generic { name, args },
            ) if self.is_array_coercible(name.as_str()) && args.len() == 1 => {
                self.unify_inner(e1, &args[0], span)
            }
            (
                Generic { name, args },
                Array {
                    element: e1,
                    size: _,
                },
            ) if self.is_array_coercible(name.as_str()) && args.len() == 1 => {
                self.unify_inner(e1, &args[0], span)
            }
            // Array -> Named collection coercion (when collection type is parsed as Named)
            (
                Array {
                    element: e1,
                    size: _,
                },
                Named { path, args },
            ) if self.path_is_array_coercible(path) && args.len() == 1 => {
                self.unify_inner(e1, &args[0], span)
            }
            (
                Named { path, args },
                Array {
                    element: e1,
                    size: _,
                },
            ) if self.path_is_array_coercible(path) && args.len() == 1 => {
                self.unify_inner(e1, &args[0], span)
            }

            // Array -> TypeApp coercion (for HKT inference)
            // [T; N] can coerce to F<T> where F is inferred as List
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (
                Array {
                    element: e1,
                    size: _,
                },
                TypeApp {
                    constructor,
                    args: app_args,
                },
            )
            | (
                TypeApp {
                    constructor,
                    args: app_args,
                },
                Array {
                    element: e1,
                    size: _,
                },
            ) => { self.unify_array_application(unification_left, unification_right, span) }

            // Slice types
            (Slice { element: e1 }, Slice { element: e2 }) => self.unify_inner(e1, e2, span),

            // Array -> Slice coercion: [T; N] can coerce to [T]
            // This enables &[T; N] -> &[T] through reference unification
            // Unsized coercion: converting sized types to unsized (e.g., List<T> to &[T], concrete to &dyn Protocol)
            (Array { element: e1, .. }, Slice { element: e2 }) => self.unify_inner(e1, e2, span),
            (Slice { element: e1 }, Array { element: e2, .. }) => self.unify_inner(e1, e2, span),

            // Collection -> Slice coercion: C<T> can coerce to [T] for array-coercible C
            // This enables patterns like passing &List<T> where &[T] is expected
            (Generic { name, args }, Slice { element: e2 })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(&args[0], e2, span)
            }
            (Slice { element: e1 }, Generic { name, args })
                if self.is_array_coercible(name.as_str()) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }
            // Collection (Named) -> Slice coercion
            (Named { path, args }, Slice { element: e2 })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(&args[0], e2, span)
            }
            (Slice { element: e1 }, Named { path, args })
                if self.path_is_array_coercible(path) && args.len() == 1 =>
            {
                self.unify_inner(e1, &args[0], span)
            }

            // Record types
            (Record(f1), Record(f2)) => { self.unify_records(unification_left, unification_right, span) }

            // Extensible record with row polymorphism
            // Pattern matching: exhaustiveness checking, type narrowing in match arms, irrefutable patterns — .1 - Row Polymorphism
            (
                ExtensibleRecord {
                    fields: f1,
                    row_var: r1,
                },
                ExtensibleRecord {
                    fields: f2,
                    row_var: r2,
                },
            ) => { self.unify_open_records(unification_left, unification_right, span) }

            // Extensible record unifying with closed record
            (
                ExtensibleRecord {
                    fields: ext_fields,
                    row_var,
                },
                Record(rec_fields),
            )
            | (
                Record(rec_fields),
                ExtensibleRecord {
                    fields: ext_fields,
                    row_var,
                },
            ) => { self.unify_open_closed_records(unification_left, unification_right, span) }

            // Variant types
            (Variant(v1), Variant(v2)) => { self.unify_variants(unification_left, unification_right, span) }

            // Dynamic protocol objects (dyn Display, dyn Error, dyn Display + Debug, etc.)
            (
                DynProtocol {
                    bounds: b1,
                    bindings: bi1,
                },
                DynProtocol {
                    bounds: b2,
                    bindings: bi2,
                },
            ) => { self.unify_dynamic_protocols(unification_left, unification_right, span) }

            // Erasing a concrete carrier requires implementation evidence.
            // Equality cannot establish that evidence or recover an erased carrier.
            (_, DynProtocol { .. }) | (DynProtocol { .. }, _) => Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            }),

            // Reference types with coercion rules
            // Unified reference model: &T (managed CBGR ~15ns), &checked T (statically verified 0ns), &unsafe T (unchecked 0ns) — .3.3 - Three-Tier Reference Coercion
            // Hierarchy: &unsafe T <: &checked T <: &T (Liskov Substitution Principle)
            //

            // ALLOWED (implicit upcast - "forgetful"):
            //  &unsafe T → &checked T ✓
            //  &unsafe T → &T ✓
            //  &checked T → &T ✓
            //

            // FORBIDDEN (downcast):
            //  &T → &checked T ✗
            //  &T → &unsafe T ✗
            //  &checked T → &unsafe T ✗

            // Same-tier references: direct unification with mutability coercion
            // &mut T can be used where &T is expected (mutable → immutable is safe)
            // Reference coercion: &mut T coerces to &T, &T coerces to &dyn Protocol when Protocol is implemented
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_references(unification_left, unification_right, span) }

            (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_checked_references(unification_left, unification_right, span) }

            (
                UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_unsafe_references(unification_left, unification_right, span) }

            // UPCAST: &unsafe T → &checked T
            //

            // Unsafe references coerce to checked references, but mutability
            // still has to match (or downgrade mut→immut). Silently allowing
            // `&unsafe mut T → &checked T` would break the guarantee that a
            // shared `&checked T` never aliases mutable access. The managed
            // and checked-only branches above already enforce the
            // `m2 && !m1` rule; cross-tier unification gets the same
            // treatment here.
            (
                UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_unsafe_checked_reference(unification_left, unification_right, span) }

            // UPCAST: &unsafe T → &T or &mut T (allowed)
            // Unsafe references have no compile-time safety guarantees, so they
            // can coerce to any managed reference tier. The mutability of the
            // unsafe ref is irrelevant since safety is manual.
            (
                UnsafeReference {
                    mutable: _m1,
                    inner: i1,
                },
                Reference {
                    mutable: _m2,
                    inner: i2,
                },
            ) => self.unify_inner(i1, i2, span),

            // UPCAST: &checked T → &T (allowed)
            (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_checked_reference(unification_left, unification_right, span) }

            // FFI COERCION: &mut T → &unsafe T (allowed for FFI interop)
            // This is technically a downcast to unsafe, but is commonly needed
            // when passing mutable buffers to FFI functions.
            // The caller takes responsibility for safety when using unsafe.
            (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_reference_unsafe(unification_left, unification_right, span) }

            // POINTER ↔ REFERENCE COERCION
            // *const T / *mut T can coerce to &T / &mut T (upcast from raw to managed)
            // and vice versa for FFI interop
            (
                Pointer {
                    mutable: m1,
                    inner: i1,
                },
                Reference {
                    mutable: m2,
                    inner: i2,
                },
            )
            | (
                Reference {
                    mutable: m1,
                    inner: i1,
                },
                Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_pointer_reference(unification_left, unification_right, span) }

            // POINTER ↔ CHECKED REFERENCE COERCION
            (
                Pointer {
                    mutable: m1,
                    inner: i1,
                },
                CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            )
            | (
                CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_pointer_checked_reference(unification_left, unification_right, span) }

            // FORBIDDEN DOWNCASTS: These will fall through to type mismatch

            // Ownership references
            (
                Ownership {
                    mutable: m1,
                    inner: i1,
                },
                Ownership {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_ownership(unification_left, unification_right, span) }

            // Pointers — allow *const T ↔ *mut T coercion for raw pointer interop
            // Raw pointers are inherently unsafe; mutability is advisory, not enforced.
            (
                Pointer {
                    mutable: _m1,
                    inner: i1,
                },
                Pointer {
                    mutable: _m2,
                    inner: i2,
                },
            ) => self.unify_inner(i1, i2, span),

            // POINTER ↔ UNSAFE REFERENCE COERCION
            // In Verum, *const T ≡ &unsafe T and *mut T ≡ &unsafe mut T
            // These are semantically equivalent types. Since both represent
            // raw/unmanaged access, mutability coercion is unrestricted.
            (
                Pointer {
                    mutable: _m1,
                    inner: i1,
                },
                UnsafeReference {
                    mutable: _m2,
                    inner: i2,
                },
            )
            | (
                UnsafeReference {
                    mutable: _m1,
                    inner: i1,
                },
                Pointer {
                    mutable: _m2,
                    inner: i2,
                },
            ) => self.unify_inner(i1, i2, span),

            // VOLATILE POINTER unification
            (
                VolatilePointer {
                    mutable: m1,
                    inner: i1,
                },
                VolatilePointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => { self.unify_volatile_pointers(unification_left, unification_right, span) }

            // POINTER ↔ INTEGER COERCION (FFI)
            // Raw pointers and pointer-width integer types are
            // interchangeable in FFI contexts. This enables
            // `ptr as usize` and `usize as *mut T` patterns. The
            // recognised alias set (`USize`, `ISize`, `IntSize`,
            // `UIntSize`, `usize`, `isize`, `Usize`, `Isize`) lives in
            // `verum_common::well_known_types::type_names::is_pointer_width_integer_type`
            // — the canonical drift-pinned registry. Previously this
            // matches!() listed only `UIntSize`/`IntSize`/`usize`/`isize`,
            // silently rejecting the canonical Verum capitalisations
            // `USize` and `ISize`.
            (Pointer { .. }, Named { path, .. }) | (Named { path, .. }, Pointer { .. })
                if verum_common::well_known_types::type_names::is_pointer_width_integer_type(
                    path.last_segment_name(),
                ) =>
            {
                Ok(Substitution::new())
            }

            // CString ↔ *const c_char coercion (FFI string interop)
            (Named { path, .. }, Pointer { .. })
                if matches!(path.last_segment_name(), "CString" | "CStr") =>
            {
                Ok(Substitution::new())
            }
            (Pointer { .. }, Named { path, .. })
                if matches!(path.last_segment_name(), "CString" | "CStr") =>
            {
                Ok(Substitution::new())
            }

            // A named protocol has declaration identity even when its spelling
            // is also used as the type of an erased receiver. The type checker
            // establishes the directional coercion from the source implementation.
            (Named { path: actual, .. }, Named { path: expected, .. })
                if !crate::ty::same_type_path(actual, expected)
                    && crate::ty::nominal_path_name(expected)
                        .is_some_and(|name| self.protocol_names.contains(name.as_ref())) =>
            {
                Err(TypeError::Mismatch { expected: t2.to_text(), actual: t1.to_text(), span })
            }

            // META TYPE COERCION: TokenStream ↔ @Expr/@Ident/@Type etc.
            // Quote blocks produce TokenStream, but meta functions may return @Expr
            (Named { path: p1, .. }, Named { path: p2, .. })
                if (matches!(p1.last_segment_name(), "TokenStream")
                    && matches!(
                        p2.last_segment_name(),
                        "@Expr" | "@Ident" | "@Type" | "@Pattern" | "@Stmt" | "@Item" | "@Block"
                    ))
                    || (matches!(p2.last_segment_name(), "TokenStream")
                        && matches!(
                            p1.last_segment_name(),
                            "@Expr"
                                | "@Ident"
                                | "@Type"
                                | "@Pattern"
                                | "@Stmt"
                                | "@Item"
                                | "@Block"
                        )) =>
            {
                Ok(Substitution::new())
            }

            // AUTO-DEREF COERCION: &T → T (implicit dereference)
            // When a reference type appears where a value type is expected,
            // automatically dereference. This follows the principle that
            // reading through a reference is always safe.
            // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — Auto-dereference semantics
            (
                Reference { inner, .. }
                | CheckedReference { inner, .. }
                | UnsafeReference { inner, .. },
                other,
            ) if !matches!(
                other,
                Reference { .. }
                    | CheckedReference { .. }
                    | UnsafeReference { .. }
                    | Ownership { .. }
                    | Pointer { .. }
            ) =>
            {
                self.unify_inner(inner, other, span)
            }

            // AUTO-DEREF COERCION (reversed): T expected, &T found
            (
                other,
                Reference { inner, .. }
                | CheckedReference { inner, .. }
                | UnsafeReference { inner, .. },
            ) if !matches!(
                other,
                Reference { .. }
                    | CheckedReference { .. }
                    | UnsafeReference { .. }
                    | Ownership { .. }
                    | Pointer { .. }
            ) =>
            {
                self.unify_inner(other, inner, span)
            }

            // Refinement types (structural unification - ignore predicates)
            (Refined { base: b1, .. }, Refined { base: b2, .. }) => self.unify_inner(b1, b2, span),

            (Refined { base, .. }, ty) | (ty, Refined { base, .. }) => {
                self.unify_inner(base, ty, span)
            }

            // Named types
            (Named { path: p1, args: a1 }, Named { path: p2, args: a2 }) => { self.unify_nominal_types(unification_left, unification_right, span) }

            // Associated projections retain their receiver and member identity.
            // Equal projections unify structurally; concrete receivers may be
            // normalized by the installed declaration-backed resolver. An
            // unresolved projection is not evidence of an arbitrary equality.

            // Case 1: Both are projections (same associated type name)
            (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 })
                if n1.as_str().starts_with("::") && n2.as_str().starts_with("::") && n1 == n2 =>
            { self.unify_equal_projections(unification_left, unification_right, span) }

            // Case 3 (T0701): projection with a CONCRETE base — the case the
            // comment above always promised and no arm implemented.  Resolve
            // `::Item[Range<Int>]` → `Int` through the installed resolver and
            // unify the result in place of the projection.  Two arms (not an
            // or-pattern) so expected/found positions in a downstream
            // mismatch stay honest.  No resolver installed, or the resolver
            // answering None (no impl provides the associated type), falls
            // through to the ordinary arms — the same honest mismatch as
            // before.
            (Generic { name, args }, other)
                if name.as_str().starts_with("::")
                    && args.len() == 1
                    && !Self::has_type_vars(&args[0])
                    && self.projection_resolver.is_some() =>
            { self.unify_left_projection(unification_left, unification_right, span) }
            (other, Generic { name, args })
                if name.as_str().starts_with("::")
                    && args.len() == 1
                    && !Self::has_type_vars(&args[0])
                    && self.projection_resolver.is_some() =>
            { self.unify_right_projection(unification_left, unification_right, span) }

            // Generic types (stdlib types like List<T>, Map<K,V>, Box<T>, etc.)
            (Generic { name: n1, args: a1 }, Generic { name: n2, args: a2 }) => { self.unify_generic_types(unification_left, unification_right, span) }

            // Cross-unification: Generic <-> Named
            // This handles cases where the same type is represented differently:
            // - Generic { name: "Map", args } from Type::map()
            // - Named { path: Map, args } from parsing Map<K, V>
            (Generic { name, args: a1 }, Named { path, args: a2 })
            | (Named { path, args: a2 }, Generic { name, args: a1 }) => { self.unify_generic_nominal(unification_left, unification_right, span) }

            // Structural sum payloads can be checked only against the exact
            // declaration's template. Missing ownership is not equality evidence.
            (Generic { name, args }, Variant(variants))
            | (Variant(variants), Generic { name, args }) => {
                self.unify_declared_variant(name, args, variants, span)
                    .unwrap_or_else(|| Err(TypeError::Mismatch {
                        expected: t2.to_text(), actual: t1.to_text(), span,
                    }))
            }
            (Named { path, args }, Variant(variants))
            | (Variant(variants), Named { path, args }) => {
                crate::ty::nominal_path_name(path)
                    .and_then(|name| self.unify_declared_variant(&name.as_ref().into(), args, variants, span))
                    .unwrap_or_else(|| Err(TypeError::Mismatch {
                        expected: t2.to_text(), actual: t1.to_text(), span,
                    }))
            }

            // TypeApp (with Var constructor) vs Variant - HKT inference
            // When M<A> is unified with Maybe<Int> variant, infer M = Maybe
            // This is stdlib-agnostic: uses the variant_type_names registry.
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (
                TypeApp {
                    constructor,
                    args: app_args,
                },
                Variant(variants),
            )
            | (
                Variant(variants),
                TypeApp {
                    constructor,
                    args: app_args,
                },
            ) => { self.unify_application_variant(unification_left, unification_right, span) }

            // Meta parameter types
            // Meta system: unified compile-time computation via "meta fn", "meta" parameters, @derive macros, tagged literals, all under single "meta" concept — Meta parameters are compile-time values
            // Meta parameters unify if:
            // 1. Names match (same compile-time parameter)
            // 2. Base types unify
            // 3. Refinements are compatible
            // 4. Concrete compile-time values (when both sides carry one) match.
            //  This is how `StaticMatrix<T, 2, 3>` vs `StaticMatrix<T, 3, 2>`
            //  produces a dimension-mismatch diagnostic.
            (
                Meta {
                    name: n1,
                    ty: t1,
                    refinement: r1,
                    value: v1,
                },
                Meta {
                    name: n2,
                    ty: t2,
                    refinement: r2,
                    value: v2,
                },
            ) => { self.unify_meta_values(unification_left, unification_right, span) }

            // Meta parameter unification with non-Meta types:
            // A meta parameter like `N: meta usize` should be treated as its base type
            // when used in expression contexts (array sizes, ranges, arithmetic).
            (Meta { ty, .. }, other) | (other, Meta { ty, .. }) => {
                self.unify_inner(ty, other, span)
            }

            // ============================================
            // DEPENDENT TYPES (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
            // ============================================

            // Pi Types (Dependent Functions)
            // Pi types (dependent functions): (x: A) -> B(x) where return type depends on input value, non-dependent functions are special case
            // (x: A) -> B(x) unifies with (y: A') -> B'(y) if:
            // 1. A unifies with A'
            // 2. B[x := fresh] unifies with B'[y := fresh] under the substitution
            (
                Pi {
                    param_name: n1,
                    param_type: p1,
                    return_type: r1,
                },
                Pi {
                    param_name: n2,
                    param_type: p2,
                    return_type: r2,
                },
            ) => { self.unify_dependent_functions(unification_left, unification_right, span) }

            // Sigma Types (Dependent Pairs)
            // Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma
            // (x: A, B(x)) unifies with (y: A', B'(y)) if:
            // 1. A unifies with A'
            // 2. B[x := fresh] unifies with B'[y := fresh]
            (
                Sigma {
                    fst_name: n1,
                    fst_type: f1,
                    snd_type: s1,
                },
                Sigma {
                    fst_name: n2,
                    fst_type: f2,
                    snd_type: s2,
                },
            ) => { self.unify_dependent_pairs(unification_left, unification_right, span) }

            // Equality Types
            // Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution
            // Eq<A, x, y> unifies with Eq<A', x', y'> if:
            // 1. A unifies with A'
            // 2. x equals x' (definitional equality)
            // 3. y equals y' (definitional equality)
            (
                Eq {
                    ty: t1,
                    lhs: l1,
                    rhs: r1,
                },
                Eq {
                    ty: t2,
                    lhs: l2,
                    rhs: r2,
                },
            ) => { self.unify_equalities(unification_left, unification_right, span) }

            // Cubical Path Types
            //

            //  Γ ⊢ A : Type Γ ⊢ a, b : A
            //  ─────────────────────────── (Path-Form)
            //  Γ ⊢ Path<A>(a, b) : Type
            //

            // Two path types unify when:
            //  1. Their carrier spaces unify as regular types,
            //  2. Their left endpoints are definitionally equal as cubical
            //  terms (normalized via `whnf` — so `transp (λ i. A) x ≡ x`
            //  and `refl x @ i ≡ x` are accepted),
            //  3. Their right endpoints are likewise definitionally equal.
            //

            // Without this arm, path types silently fall into the fallback
            // mismatch path — which made the 492-line cubical normalizer in
            // `crate::cubical` dead code on the unification hot path.
            (
                Type::PathType {
                    space: s1,
                    left: l1,
                    right: r1,
                },
                Type::PathType {
                    space: s2,
                    left: l2,
                    right: r2,
                },
            ) => { self.unify_paths(unification_left, unification_right, span) }

            // Partial Element Types — `Partial[φ] A`: a partial element
            // of `A` defined on the face constraint `φ`. Used in Kan
            // composition to describe the compositional boundary.
            //

            // Two partial types unify when:
            //  1. Their element types unify,
            //  2. Their face constraints are definitionally equal
            //  (cubical normalization handles `(i ∨ ~i) ≡ 1`, etc.).
            (
                Type::Partial {
                    element_type: e1,
                    face: f1,
                },
                Type::Partial {
                    element_type: e2,
                    face: f2,
                },
            ) => { self.unify_partial_elements(unification_left, unification_right, span) }

            // Interval type `I` is a singleton — trivial unification.
            (Type::Interval, Type::Interval) => Ok(Substitution::new()),

            // Universe Types
            // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter
            // Type_n unifies with Type_m if n == m
            // Universe polymorphism is handled by level variables
            (Universe { level: l1 }, Universe { level: l2 }) => {
                let mut result = Substitution::new();
                result.universes.unify(l1, l2).map_err(|message| TypeError::UniverseBinding { message, span })?;
                Ok(result)
            }

            // Prop (Proof-Irrelevant Propositions)
            // Inductive types: recursive type definitions with structural recursion, termination checking — .1
            // Prop unifies with Prop
            (Prop, Prop) => Ok(Substitution::new()),

            // Inductive Types
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .1
            // Two inductive types unify if:
            // 1. Names match
            // 2. All parameters unify
            // 3. All indices unify (for indexed families)
            (
                Inductive {
                    name: n1,
                    params: p1,
                    indices: i1,
                    universe: u1,
                    constructors: c1,
                },
                Inductive {
                    name: n2,
                    params: p2,
                    indices: i2,
                    universe: u2,
                    constructors: c2,
                },
            ) => { self.unify_inductive_types(unification_left, unification_right, span) }

            // Coinductive Types
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .2
            (
                Coinductive {
                    name: n1,
                    params: p1,
                    destructors: d1,
                },
                Coinductive {
                    name: n2,
                    params: p2,
                    destructors: d2,
                },
            ) => { self.unify_coinductive_types(unification_left, unification_right, span) }

            // Higher Inductive Types (HITs)
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .3
            (
                HigherInductive {
                    name: n1,
                    params: p1,
                    point_constructors: pc1,
                    path_constructors: pa1,
                },
                HigherInductive {
                    name: n2,
                    params: p2,
                    point_constructors: pc2,
                    path_constructors: pa2,
                },
            ) => { self.unify_higher_inductive_types(unification_left, unification_right, span) }

            // Quantified Types (Quantitative Type Theory)
            // Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4
            // Track usage: linear (1), affine (0..1), unrestricted (ω)
            (
                Quantified {
                    inner: i1,
                    quantity: q1,
                },
                Quantified {
                    inner: i2,
                    quantity: q2,
                },
            ) => { self.unify_quantities(unification_left, unification_right, span) }

            // Quantified type can unify with unquantified if quantity is unrestricted
            (Quantified { inner, quantity }, ty) | (ty, Quantified { inner, quantity }) => { self.unify_quantified_carrier(unification_left, unification_right, span) }

            // Existential Types
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5 - Existential Unification
            //

            // Two existential types unify if their bodies unify after alpha-renaming
            // to use the same bound variable.
            (Exists { var: v1, body: b1 }, Exists { var: v2, body: b2 }) => { self.unify_existentials(unification_left, unification_right, span) }

            // Existential with concrete type: the concrete type becomes the witness
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — .5
            (Exists { var, body }, ty) | (ty, Exists { var, body }) => { self.unify_existential_carrier(unification_left, unification_right, span) }

            // Universal Types (Forall)
            // Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Universal Types
            //

            // Two universal types unify if their bodies unify after alpha-renaming
            (
                Forall {
                    vars: vs1,
                    body: b1,
                },
                Forall {
                    vars: vs2,
                    body: b2,
                },
            ) => { self.unify_universal_types(unification_left, unification_right, span) }

            // RANK2-SUBSUMPTION (#41): Forall vs concrete. Pre-fix there was
            // NO arm here (only Exists↔concrete and Forall↔Forall), so every
            // rank-2 annotation check fell to the catch-all Mismatch:
            // `let _: fn<B: P>(B) -> B.Assoc = generic_fn;` failed
            // "expected '∀_. fn(_) -> _', found 'fn(_) -> _'" — the
            // async_iterator integration family. Standard predicative
            // subsumption, two directions:
            //
            // * ACTUAL is ∀, a mono type is required — using a polymorphic
            //   value at a concrete type: INSTANTIATE the binders with fresh
            //   unification vars and unify the body.
            (Forall { vars, body }, ty) => {
                let mut inst = Substitution::new();
                for v in vars.iter() {
                    inst.insert(*v, Type::Var(TypeVar::fresh()));
                }
                let body_inst = body.try_apply_subst(&inst).map_err(|message| TypeError::UniverseBinding { message, span })?;
                self.unify_inner(&body_inst, ty, span)
            }
            // * A ∀ is REQUIRED, actual is a (freshly instantiated) mono
            //   type: SKOLEMIZE the expected binders into rigid constants —
            //   the actual's fresh unification vars may bind to them, i.e.
            //   the actual must be GENUINELY polymorphic; an attempt to pin
            //   a concrete type fails against the rigid skolem.
            (ty, Forall { vars, body }) => { self.unify_right_universal(unification_left, unification_right, span) }

            // TypeApp - higher-kinded type applications (e.g., F<Int> where F is a type constructor)
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
            (
                Type::TypeApp {
                    constructor: c1,
                    args: a1,
                },
                Type::TypeApp {
                    constructor: c2,
                    args: a2,
                },
            ) => { self.unify_applications(unification_left, unification_right, span) }

            // TypeConstructor unification - two type constructors unify if they have the same name and arity
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
            (
                Type::TypeConstructor {
                    name: n1,
                    arity: a1,
                    ..
                },
                Type::TypeConstructor {
                    name: n2,
                    arity: a2,
                    ..
                },
            ) => { self.unify_constructors(unification_left, unification_right, span) }

            // TypeApp with type variable constructor vs Generic - HKT inference
            // When F<A> (TypeApp with Var constructor) is unified with List<Int> (Generic),
            // we infer F = List (TypeConstructor) and A = Int
            // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types inference
            (
                Type::TypeApp {
                    constructor,
                    args: app_args,
                },
                Type::Generic {
                    name,
                    args: gen_args,
                },
            )
            | (
                Type::Generic {
                    name,
                    args: gen_args,
                },
                Type::TypeApp {
                    constructor,
                    args: app_args,
                },
            ) => { self.unify_application_generic(unification_left, unification_right, span) }

            // TypeApp with type variable constructor vs Named - HKT inference
            // Similar to Generic case but for Named types
            (
                Type::TypeApp {
                    constructor,
                    args: app_args,
                },
                Type::Named {
                    path,
                    args: named_args,
                },
            )
            | (
                Type::Named {
                    path,
                    args: named_args,
                },
                Type::TypeApp {
                    constructor,
                    args: app_args,
                },
            ) => { self.unify_application_nominal(unification_left, unification_right, span) }

            // ================================================================
            // Bool ↔ Int and Char ↔ Int coercions — REMOVED (semantic honesty)
            // ================================================================
            //

            // Previously this unifier silently accepted `Bool` and `Char` as
            // subtypes of `Int`, meaning `let x: Int = true`, `fn f() -> Int {
            // true }`, and `a: Int + b: Bool` all type-checked cleanly — an
            // enormous blind spot directly contradicting AGENTS.md's "semantic
            // honesty" principle ("types describe meaning, not implementation")
            // and causing `integration_test::test_type_error_detection` to
            // fail because `fn bad_add(a: Int, b: Bool) -> Int { a + b }` was
            // accepted.
            //

            // The original justification was to enable patterns like
            // `assert_eq(x > 0, 1)` and arithmetic on characters. Those
            // patterns are themselves code smells — they paper over missing
            // explicit conversions. The correct forms are:
            //

            //  - `assert_eq(x > 0, true)` (compare booleans with booleans)
            //  - `(ch as Int) == 65` (explicit cast for char)
            //

            // Removing the coercions:
            //  - makes `test_type_error_detection` pass correctly
            //  - restores type safety for Bool and Char at assignment,
            //  return, argument passing, and binary operator sites
            //  - aligns with AGENTS.md Rule: "Semantic Honesty"
            //  - keeps the Verum refinement system sound (Int refinements
            //  like `Int{>= 0}` can no longer be "satisfied" by a Bool)
            //

            // If a user really needs Bool-as-Int semantics they must write
            // an explicit cast `as Int` or use `if cond { 1 } else { 0 }`.
            //

            // Related: AGENTS.md section "Semantic Types", the test
            // `crates/verum_compiler/tests/type_error_detection_debug.rs`
            // that documents the full matrix of cases (Int+Bool, Bool+Bool,
            // let x: Int = true, fn f() -> Int { true }, ...), and
            // `crates/verum_compiler/tests/integration_test.rs:113-138`.
            //

            // Note: `(Char, Char)` still unifies (same-type reflexivity) via
            // the general primitive case below; only the cross-type
            // coercions are removed here.

            // Permission equality is symmetric. Giving away a permission is a
            // directed value conversion, checked by Subtyping at the use site.
            (CapabilityRestricted { base: left, capabilities: held },
             CapabilityRestricted { base: right, capabilities: required })
                if held.is_satisfied_by(required) && required.is_satisfied_by(held) =>
            {
                self.unify_inner(left, right, span)
            }

            // Placeholder types: forward-declared recursive types have placeholder inner
            // types that must unify while the declaration resolves — but ONLY with their
            // own named type (or a still-undetermined type). A leaked placeholder that
            // unified with *everything* was unsound (tech-debt register A27, pool T0119).
            // Example: SegmentError { inner: <placeholder:SegmentError> } unifies with
            //  SegmentError { inner: MmapFailed(...) | MunmapFailed(...) | ... }
            (Placeholder { name, .. }, other) | (other, Placeholder { name, .. }) => { self.unify_placeholders(unification_left, unification_right, span) }

            // Scalar ↔ Tensor coercion for math interoperability
            // In tensor libraries, scalar types (Float, Int, Bool) and their tensor
            // wrappers (DynTensor<Float>, etc.) are used interchangeably.
            // This bidirectional coercion allows natural math code without explicit wraps.
            //

            // The lookup is structural via the `TensorLike` protocol from
            // `core/base/coercion.vr` — populated by
            // `scan_protocol_implementations` (see #101 step 2). The HashSet
            // here is a fast O(1) cache of the protocol-implementation
            // discovery result.
            (Float, Generic { name, args }) | (Generic { name, args }, Float)
                if args.len() == 1
                    && self.is_tensor_family(name.as_str())
                    && matches!(&args[0], Type::Float | Type::Var(_) | Type::Unknown) =>
            {
                Ok(Substitution::new())
            }
            (Float, Named { path, args }) | (Named { path, args }, Float) if args.len() <= 1 => { self.unify_float_nominal(unification_left, unification_right, span) }
            // Bool ↔ tensor coercion (for relu_vjp etc.)
            // Uses data-driven tensor_family_types set.
            (Bool, Generic { name, args }) | (Generic { name, args }, Bool)
                if args.len() == 1
                    && self.is_tensor_family(name.as_str())
                    && matches!(&args[0], Type::Bool | Type::Var(_)) =>
            {
                Ok(Substitution::new())
            }
            // Int ↔ tensor coercion (for tensor indexing/shapes)
            // Uses data-driven tensor_family_types set.
            (Int, Generic { name, args }) | (Generic { name, args }, Int)
                if args.len() == 1 && self.is_tensor_family(name.as_str()) =>
            {
                Ok(Substitution::new())
            }
            // NOTE: Named ↔ Named coercion for tensor types is handled in the main
            // Named match arm above (at the segment/args length mismatch check).

            // Slice ↔ element coercion: [Byte] vs UInt8/Byte
            // FFI functions often use raw bytes where Verum uses byte slices
            (Slice { element }, Named { path, args, .. })
            | (Named { path, args, .. }, Slice { element })
                if args.is_empty() =>
            { self.unify_slice_nominal(unification_left, unification_right, span) }

            // There is deliberately NO `Int ↔ Variant` coercion here.
            //
            // One existed until T0722, justified as serving "FFI wrappers":
            // C functions return `Int` error codes while Verum wrappers return
            // a `Result`- or `Maybe`-shaped sum. The unifier has no notion of
            // an FFI boundary, so the arm fired wherever those two shapes met.
            // `let z: Int = maybe_value;` type-checked and `z + 1` read a raw
            // pointer — 39765685617, then 54182278177, no panic, exit 0.
            //
            // Recognition was structural (`classify_variants`), so it was never
            // limited to the stdlib's own types: a user's
            // `type Own<T> is Nothing | Just(T)` satisfied a declared `Int` too.
            // That was measured, not assumed.
            //
            // Whether any FFI wrapper actually needed it was also measured: a
            // full 2558-file stdlib bake — the corpus that IS the FFI wrappers,
            // core/sys, core/net, core/database — reached this arm ZERO times
            // and produced its 21.9 MB archive without it.
            //
            // An extern whose declared Verum return type is `Result`/`Maybe`
            // while the C signature returns `int` is a mis-declared extern, and
            // the honest fix is at that declaration, not a global coercion that
            // makes every sum type in the language interchangeable with `Int`.

            // Named type ↔ record structural coercion
            // When a Named type (like SocketAddrV4) is expected but a structural record
            // is provided, allow coercion. Named types are often aliases for records
            // and strict matching causes false positives in cross-module code.
            (
                Named {
                    path: _,
                    args: named_args,
                    ..
                },
                Record(_),
            )
            | (
                Record(_),
                Named {
                    path: _,
                    args: named_args,
                    ..
                },
            ) if named_args.is_empty() => Ok(Substitution::new()),

            // Tuple ↔ Named type coercion for tuple-like newtypes
            (Tuple(_), Named { .. }) | (Named { .. }, Tuple(_)) => {
                // Allow tuple ↔ named type when it looks like a newtype pattern
                // e.g., VisibleRow = (List<Int>, Int, Line, Bool, Bool)
                Ok(Substitution::new())
            }

            // Generic/Named collection ↔ scalar coercion
            // List<USize> vs Int, Range<Int> vs (Maybe<USize>, Maybe<USize>), etc.
            // These arise from type inference through indexing and slicing operations.
            //

            // The lookup is structural via the `Indexable` and `RangeLike`
            // protocols from `core/base/coercion.vr` — populated by
            // `scan_protocol_implementations` (see #101 step 2). The HashSets
            // here are fast O(1) caches of the protocol-implementation
            // discovery result.
            // There is deliberately NO `(Generic { indexable }, Int)` arm here.
            //
            // One existed until T0722. It accepted a COLLECTION where an `Int`
            // was declared, so `let y: Int = xs;` type-checked and `y + 1`
            // added one to a raw pointer — printing a different large number
            // on every run, with no panic, no warning and exit 0.
            //
            // Its comment justified it as smoothing "type inference through
            // indexing": the container allegedly reached the unifier where the
            // element was meant. That was measured before removal, with a
            // positive control inside each run:
            //
            //   * `xs[0]` / `xs.len()` into `Int`: 16 unifications, 2 of them
            //     naming `List`, the arm fired ZERO times, correct output.
            //   * a full 2558-file stdlib bake: the arm fired ZERO times and
            //     produced a 21.9 MB archive.
            //   * `let y: Int = xs;`: the arm fired, and only there.
            //
            // Element-vs-`Int` widening — the coercion the comment actually
            // described (`slice[i] + 1` where the element is `USize`) — is
            // handled by `is_int_coercible_named` / `sized_numeric_types`,
            // which compare the ELEMENT. This arm compared the CONTAINER, so
            // it never served the case it was written for.
            (Generic { name, .. }, Tuple(_)) | (Tuple(_), Generic { name, .. })
                if self.is_range_like(name.as_str()) =>
            {
                Ok(Substitution::new())
            }

            // Future type unification: Future<A> ↔ Future<B>
            // Both sides are the built-in Future type variant
            (Future { output: o1 }, Future { output: o2 }) => self.unify_inner(o1, o2, span),

            // Future cross-representation: Type::Future ↔ Named/Generic { "Future" }
            // async blocks produce Type::Future { output }, while parsed types produce
            // Named { path: "Future", args: [T] } or Generic { name: "Future", args: [T] }
            (Future { output }, Named { path, args })
            | (Named { path, args }, Future { output })
                if args.len() == 1 =>
            { self.unify_future_nominal(unification_left, unification_right, span) }
            (Future { output }, Generic { name, args })
            | (Generic { name, args }, Future { output })
                if args.len() == 1 && name.as_str() == "Future" =>
            {
                self.unify_inner(output, &args[0], span)
            }

            // #123 — `(TypeConstructor, Variant)` HKT recovery.
            //
            // When a HKT parameter `F<_>` reaches a position where the
            // unifier sees the bare type constructor (no `TypeApp`
            // wrapper around it), and the actual side is a structural
            // `Variant` shape, the existing `(TypeApp, Variant)`
            // branch (~line 3047) doesn't fire. The missing case shows
            // up for the canonical Monad default impl
            //
            //   fn flatten<A>(ffa: F<F<A>>) -> F<A> { Self.flat_map(ffa, |fa| fa) }
            //
            // dispatched as `Maybe.flatten(nested)` where
            // `nested: Maybe<Maybe<Int>>`. The dispatch unfolds the
            // parameter type but at one level the unifier ends up
            // matching the bare `F` (TypeConstructor) against the
            // outer Variant — and there is no rule to recover the
            // nominal "Maybe" name from the structural form.
            //
            // The fix mirrors the constructor-matching half of the
            // `(TypeApp, Variant)` branch: look the variant signature
            // up in `variant_type_names`. If the registered name
            // equals the constructor name (concrete case), accept.
            // If the constructor's name is a single uppercase
            // identifier with arity > 0 — the canonical shape of an
            // HKT parameter — accept *as-if-bound*: the dispatch's
            // outer substitution context will have already bound F
            // and the Variant just provides the structural confirmation.
            //
            // No new AST nodes, no new error variants — pure unifier
            // extension, fully consistent with the existing HKT
            // recovery path.
            (Variant(variants), TypeConstructor { name, arity, .. })
            | (TypeConstructor { name, arity, .. }, Variant(variants))
                if *arity > 0 =>
            { self.unify_variant_constructor(unification_left, unification_right, span) }

            // Type mismatch
            _ => Err(TypeError::Mismatch {
                expected: t2.to_text(),
                actual: t1.to_text(),
                span,
            }),
        }
    }

    /// Heuristic: does `name` look like a higher-kinded type parameter
    /// (`F`, `M`, `Self.F`, etc.) rather than a stdlib-or-user nominal
    /// type constructor (`Maybe`, `List`, `Result`, ...)?
    ///
    /// Used by the `(TypeConstructor, Variant)` HKT recovery branch to
    /// distinguish "this is an unresolved protocol parameter that the
    /// outer dispatch already pinned" from "this is a genuinely-named
    /// constructor and the variant doesn't match".
    ///
    /// Stdlib-agnostic: relies purely on identifier shape, never on a
    /// hardcoded list of HKT-parameter names.
    fn looks_like_hkt_param(name: &str) -> bool {
        // Trim any `Self.` prefix used for protocol-associated kinds.
        let bare = name.strip_prefix("Self.").unwrap_or(name);
        if bare.is_empty() {
            return false;
        }
        // Single-letter uppercase: F, M, T, A, B — the conventional
        // HKT-param shape. Verum stdlib types are at least 4 chars
        // (Maybe, List, etc.) so single-letter never collides.
        if bare.len() == 1 {
            return bare.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        }
        false
    }

    /// Bind a type variable to a type.
    ///
    /// Performs the occurs check to prevent infinite types.
    fn bind_var(&self, var: TypeVar, ty: &Type, span: Span) -> Result<Substitution> {
        // INSTRUMENT (VERUM_TRACE_BINDTO=<substring>): who binds a type
        // variable to the NAMED type. R3's `ISize` is a constant filler
        // that appears in programmes never mentioning `Range`, and all
        // five `Self`-substitution sites in `protocol.rs` are measured
        // SILENT for an ordinary method call — so the question left is
        // which unification puts `ISize` into an unbound var.
        //
        // Needs RUST_BACKTRACE=1 alongside, like any `force_capture`.
        if let Ok(want) = std::env::var("VERUM_TRACE_BINDTO")
            && format!("{}", ty) == want
        {
            eprintln!(
                "[bindto] {:?} := {}\n{}",
                var,
                ty,
                std::backtrace::Backtrace::force_capture(),
            );
        }
        // Don't bind to itself
        if let Type::Var(v) = ty
            && *v == var
        {
            return Ok(Substitution::new());
        }

        // A RIGID var is a type parameter of the body being checked. The
        // signature promised to work for every type the caller picks, so
        // the body may not decide what it is.
        //
        // Two cases, and the asymmetry is the whole point. Against another
        // FLEXIBLE var the binding is legal but must go the other way — the
        // flexible one learns it is this parameter, not the reverse — and
        // `(Var, Var)` above picks its direction by variable id, which has
        // nothing to do with rigidity. Against anything else the promise is
        // broken and this is the type error.
        //
        // Without this, `fn f<T>(t: T) -> T { 42 }` was ACCEPTED with no
        // call site: T was an ordinary inference variable shared by the
        // body and every caller, so the body simply solved it to Int. The
        // mistake then surfaced at a CALLER, or as an ambiguity elsewhere
        // in the body, never at the line that made it.
        if self.rigid_vars.contains_key(&var) {
            // "The other side carries no information yet" is not always a
            // bare `Type::Var`. A refinement over an unsolved variable —
            // `_{a: T where true}`, which `core/math/hott.vr` produces by
            // the dozen — is just as unknown, and a top-level-only test
            // refused it as though it were a concrete type. Peel the
            // wrappers that add a constraint without naming a type.
            let mut carrier = ty;
            loop {
                match carrier {
                    Type::Refined { base, .. } => carrier = base,
                    _ => break,
                }
            }
            // A BOUNDED parameter may take a function shape. The only way
            // a body can demand one is if the bound promised it —
            // `fn retain<F: fn(&T) -> Bool>(f: F)` calling `f`, or passing
            // it on — and whether the caller's chosen type really satisfies
            // the bound is checked at the call site.
            //
            // Narrower than the alternative, and deliberately so: dropping
            // rigidity for every bounded parameter also lets these through
            // and costs real detection. Measured, it took
            // `vcs/specs/L1-core/types/protocols/protocol_impl.vr` from 0
            // diagnostics back to 3 — rigidity is what stops a bounded
            // parameter being solved to junk during inference.
            let callable_by_bound =
                matches!(carrier, Type::Function { .. }) && self.rigid_is_bounded(var);
            match carrier {
                // Nothing is known about the other side: let it learn that
                // it is this parameter, rather than the reverse.
                Type::Var(other) if !self.rigid_vars.contains_key(other) => {
                    return self.bind_var(*other, &Type::Var(var), span);
                }
                Type::Unknown => return Ok(Substitution::new()),
                _ => {}
            }
            if !callable_by_bound {
            // A refusal here is only as good as what the caller does with
            // it, and `synth_and_check`'s error arm has some thirty
            // recovery paths that answer `Ok(expected)`. VERUM_TRACE_RIGID
            // says the refusal HAPPENED, which separates "the guard never
            // saw this case" from "the guard fired and someone recovered".
            if std::env::var_os("VERUM_TRACE_RIGID").is_some() {
                eprintln!(
                    "[rigid] refuse bind {} := {}",
                    self.rigid_name(var),
                    ty.to_text()
                );
            }
            return Err(TypeError::Mismatch {
                expected: self.rigid_name(var),
                // The found side is often the OTHER parameter, and it would
                // render as `_` through the ordinary path — "expected `A`,
                // found `_`" hides exactly the fact that makes the code
                // wrong.
                actual: match ty {
                    Type::Var(other) => self.rigid_name(*other),
                    other => other.to_text(),
                },
                span,
            });
            }
        }

        // FULL BIND LOG (#41 door-3): under the ctor trace, log every var
        // binding compactly so redefine-instance var ids (from the
        // TYPE-REDEFINE trap) can be joined against their binding sites in a
        // single run.
        if crate::ctor_trace_enabled() {
            eprintln!("[ctor-trace] bind v{} := {:?}", var.id(), ty);
        }

        // TEMPLATE-PIN TRAP (#41): binding a var that BELONGS to a registered
        // type's declaration (variant_parameters) pins the TEMPLATE globally —
        // always a bug upstream (a registry template leaked into unification
        // unfreshened). Gated diagnostic to locate the leaking door.
        if crate::ctor_trace_enabled()
            && self
                .variant_parameters
                .iter()
                .any(|(_, order)| order.contains(&Type::Var(var)))
        {
            let owner: Vec<_> = self
                .variant_parameters
                .iter()
                .filter(|(_, o)| o.contains(&Type::Var(var)))
                .map(|(n, _)| n.as_str().to_string())
                .collect();
            eprintln!(
                "[ctor-trace] TEMPLATE-PIN bind_var {:?} (owner {:?}) := {:?}\n{}",
                var,
                owner,
                ty,
                std::backtrace::Backtrace::force_capture()
            );
        }

        // Occurs check: var must not appear in ty
        if ty.free_vars().contains(&var) {
            // When the cycle is through a reference or container type (e.g., T = &T,
            // T = List<T>), break the cycle by replacing the recursive occurrence
            // with a fresh type variable. This allows type inference to proceed for
            // common patterns like `let x = &x` or method chains returning &Self.
            match ty {
                Type::Reference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::Reference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                Type::CheckedReference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::CheckedReference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                Type::UnsafeReference { inner, mutable } if inner.free_vars().contains(&var) => {
                    let fresh = Type::Var(TypeVar::fresh());
                    let broken = Type::UnsafeReference {
                        inner: Box::new(fresh),
                        mutable: *mutable,
                    };
                    let mut subst = Substitution::new();
                    subst.insert(var, broken);
                    return Ok(subst);
                }
                // A var that occurs in a FUNCTION type it is being bound to is a
                // GENUINE infinite type (`v1 = v1 -> Int`) and MUST fail the
                // occurs-check. (The former #115 band-aid cycle-broke it into a
                // fresh-holed function and returned Ok — unsound: it silently
                // accepts an unrepresentable type. The pattern-binding case that
                // band-aid worked around is fixed at its source instead.)
                _ => {
                    return Err(TypeError::InfiniteType {
                        var: var.to_text(),
                        ty: ty.to_text(),
                        span,
                    });
                }
            }
        }

        let mut subst = Substitution::new();
        subst.insert(var, ty.clone());
        Ok(subst)
    }

    // ============================================
    // DEPENDENT TYPE HELPER FUNCTIONS
    // ============================================

    /// Rename bound variable in a type (for alpha-equivalence).
    ///
    /// Used when unifying Pi/Sigma types with different parameter names.
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Alpha-equivalence for dependent types
    fn rename_bound_var(ty: &Type, from: &Text, to: &Text) -> Type {
        match ty {
            // Pi type: rename in return type if the bound variable matches
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => {
                if param_name == from {
                    // Shadow - don't rename inside
                    ty.clone()
                } else {
                    Type::Pi {
                        param_name: param_name.clone(),
                        param_type: Box::new(Self::rename_bound_var(param_type, from, to)),
                        return_type: Box::new(Self::rename_bound_var(return_type, from, to)),
                    }
                }
            }

            // Sigma type: rename in second type if the bound variable matches
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => {
                if fst_name == from {
                    // Shadow - don't rename inside
                    ty.clone()
                } else {
                    Type::Sigma {
                        fst_name: fst_name.clone(),
                        fst_type: Box::new(Self::rename_bound_var(fst_type, from, to)),
                        snd_type: Box::new(Self::rename_bound_var(snd_type, from, to)),
                    }
                }
            }

            // Meta types may reference the bound variable
            Type::Meta {
                name,
                ty: inner,
                refinement,
                value,
            } => {
                let new_name = if name == from {
                    to.clone()
                } else {
                    name.clone()
                };
                Type::Meta {
                    name: new_name,
                    ty: Box::new(Self::rename_bound_var(inner, from, to)),
                    refinement: refinement.clone(),
                    value: value.clone(),
                }
            }

            // Function types
            Type::Function {
                property_requirement,
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                property_requirement: property_requirement.clone(),
                params: params
                    .iter()
                    .map(|p| Self::rename_bound_var(p, from, to))
                    .collect(),
                return_type: Box::new(Self::rename_bound_var(return_type, from, to)),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },

            // Tuple types
            Type::Tuple(ts) => Type::Tuple(
                ts.iter()
                    .map(|t| Self::rename_bound_var(t, from, to))
                    .collect(),
            ),

            // Array types
            Type::Array { element, size } => Type::Array {
                element: Box::new(Self::rename_bound_var(element, from, to)),
                size: *size,
            },

            // Slice types
            Type::Slice { element } => Type::Slice {
                element: Box::new(Self::rename_bound_var(element, from, to)),
            },

            // Reference types
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::Ownership { mutable, inner } => Type::Ownership {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            Type::VolatilePointer { mutable, inner } => Type::VolatilePointer {
                mutable: *mutable,
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            // Generic types
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Named types
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Record types
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            ),

            // Variant types
            Type::Variant(variants) => Type::Variant(
                variants
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            ),

            // Refined types
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(Self::rename_bound_var(base, from, to)),
                predicate: predicate.clone(),
            },

            // Equality types
            Type::Eq {
                ty: inner,
                lhs,
                rhs,
            } => Type::Eq {
                ty: Box::new(Self::rename_bound_var(inner, from, to)),
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },

            // Cubical path types - rename in space type; left/right are cubical terms, not types
            Type::PathType { space, left, right } => Type::PathType {
                space: Box::new(Self::rename_bound_var(space, from, to)),
                left: left.clone(),
                right: right.clone(),
            },

            // Partial element types - rename in element_type; face is a cubical term, not a type
            Type::Partial { element_type, face } => Type::Partial {
                element_type: Box::new(Self::rename_bound_var(element_type, from, to)),
                face: face.clone(),
            },

            // Interval type I - no inner types to rename
            Type::Interval => ty.clone(),

            // Inductive types
            Type::Inductive {
                name,
                params,
                indices,
                universe,
                constructors,
            } => Type::Inductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                indices: indices
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                universe: universe.clone(),
                constructors: constructors.clone(),
            },

            // Coinductive types
            Type::Coinductive {
                name,
                params,
                destructors,
            } => Type::Coinductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                destructors: destructors.clone(),
            },

            // Higher inductive types
            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => Type::HigherInductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(Self::rename_bound_var(t, from, to))))
                    .collect(),
                point_constructors: point_constructors.clone(),
                path_constructors: path_constructors.clone(),
            },

            // Quantified types
            Type::Quantified { inner, quantity } => Type::Quantified {
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
                quantity: *quantity,
            },

            // Exists type
            Type::Exists { var, body } => Type::Exists {
                var: *var,
                body: Box::new(Self::rename_bound_var(body, from, to)),
            },

            // Forall type
            Type::Forall { vars, body } => Type::Forall {
                vars: vars.clone(),
                body: Box::new(Self::rename_bound_var(body, from, to)),
            },

            // Future type
            Type::Future { output } => Type::Future {
                output: Box::new(Self::rename_bound_var(output, from, to)),
            },

            // Generator type
            Type::Generator {
                yield_ty,
                return_ty,
            } => Type::Generator {
                yield_ty: Box::new(Self::rename_bound_var(yield_ty, from, to)),
                return_ty: Box::new(Self::rename_bound_var(return_ty, from, to)),
            },

            // Tensor type
            Type::Tensor {
                element,
                shape,
                strides,
                span,
            } => Type::Tensor {
                element: Box::new(Self::rename_bound_var(element, from, to)),
                shape: shape.clone(),
                strides: strides.clone(),
                span: *span,
            },

            // GenRef type
            Type::GenRef { inner } => Type::GenRef {
                inner: Box::new(Self::rename_bound_var(inner, from, to)),
            },

            // TypeApp type
            Type::TypeApp { constructor, args } => Type::TypeApp {
                constructor: Box::new(Self::rename_bound_var(constructor, from, to)),
                args: args
                    .iter()
                    .map(|a| Self::rename_bound_var(a, from, to))
                    .collect(),
            },

            // Primitive and other types - no renaming needed
            Type::Unit
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Never
            | Type::Var(_)
            | Type::Universe { .. }
            | Type::RecordField { .. }
            | Type::Prop
            | Type::Lifetime { .. }
            | Type::TypeConstructor { .. }
            | Type::Placeholder { .. } => ty.clone(),

            // ExtensibleRecord - rename in fields
            Type::ExtensibleRecord { fields, row_var } => Type::ExtensibleRecord {
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
                row_var: *row_var,
            },

            // CapabilityRestricted - rename in base type
            Type::CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(Self::rename_bound_var(base, from, to)),
                capabilities: capabilities.clone(),
            },

            // Unknown type - no inner types to rename
            Type::Unknown => ty.clone(),

            // DynProtocol - rename in associated type bindings
            Type::DynProtocol { bounds, bindings } => Type::DynProtocol {
                bounds: bounds.clone(),
                bindings: bindings
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::rename_bound_var(v, from, to)))
                    .collect(),
            },
        }
    }

    /// Check if two equality type terms are definitionally equal.
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — Equality types
    fn eq_terms_equal(t1: &EqTerm, t2: &EqTerm) -> bool {
        match (t1, t2) {
            (EqTerm::Var(v1), EqTerm::Var(v2)) => v1 == v2,
            (EqTerm::Const(c1), EqTerm::Const(c2)) => c1 == c2,
            (EqTerm::App { func: f1, args: a1 }, EqTerm::App { func: f2, args: a2 }) => {
                Self::eq_terms_equal(f1, f2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(x, y)| Self::eq_terms_equal(x, y))
            }
            (
                EqTerm::Lambda {
                    param: p1,
                    body: b1,
                },
                EqTerm::Lambda {
                    param: p2,
                    body: b2,
                },
            ) => {
                // Alpha-equivalence for lambda terms
                if p1 == p2 {
                    Self::eq_terms_equal(b1, b2)
                } else {
                    // Rename p2 to p1 in b2 and compare
                    let b2_renamed = Self::rename_eq_term(b2, p2, p1);
                    Self::eq_terms_equal(b1, &b2_renamed)
                }
            }
            (
                EqTerm::Proj {
                    pair: pair1,
                    component: c1,
                },
                EqTerm::Proj {
                    pair: pair2,
                    component: c2,
                },
            ) => c1 == c2 && Self::eq_terms_equal(pair1, pair2),
            (EqTerm::Refl(inner1), EqTerm::Refl(inner2)) => Self::eq_terms_equal(inner1, inner2),
            (
                EqTerm::J {
                    proof: p1,
                    motive: m1,
                    base: b1,
                },
                EqTerm::J {
                    proof: p2,
                    motive: m2,
                    base: b2,
                },
            ) => {
                Self::eq_terms_equal(p1, p2)
                    && Self::eq_terms_equal(m1, m2)
                    && Self::eq_terms_equal(b1, b2)
            }
            _ => false,
        }
    }

    /// Rename a variable in an equality term (for alpha-equivalence).
    fn rename_eq_term(term: &EqTerm, from: &Text, to: &Text) -> EqTerm {
        match term {
            EqTerm::Var(v) => {
                if v == from {
                    EqTerm::Var(to.clone())
                } else {
                    term.clone()
                }
            }
            EqTerm::Const(_) => term.clone(),
            EqTerm::App { func, args } => EqTerm::App {
                func: Box::new(Self::rename_eq_term(func, from, to)),
                args: args
                    .iter()
                    .map(|a| Self::rename_eq_term(a, from, to))
                    .collect(),
            },
            EqTerm::Lambda { param, body } => {
                if param == from {
                    // Shadow - don't rename inside
                    term.clone()
                } else {
                    EqTerm::Lambda {
                        param: param.clone(),
                        body: Box::new(Self::rename_eq_term(body, from, to)),
                    }
                }
            }
            EqTerm::Proj { pair, component } => EqTerm::Proj {
                pair: Box::new(Self::rename_eq_term(pair, from, to)),
                component: *component,
            },
            EqTerm::Refl(inner) => EqTerm::Refl(Box::new(Self::rename_eq_term(inner, from, to))),
            EqTerm::J {
                proof,
                motive,
                base,
            } => EqTerm::J {
                proof: Box::new(Self::rename_eq_term(proof, from, to)),
                motive: Box::new(Self::rename_eq_term(motive, from, to)),
                base: Box::new(Self::rename_eq_term(base, from, to)),
            },
        }
    }

    /// Check if two universe levels can unify.
    ///
    /// Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe hierarchy
    fn universe_levels_unify(l1: &UniverseLevel, l2: &UniverseLevel) -> bool {
        l1.equivalent(l2)
    }

    /// Check if two quantities are compatible for unification.
    ///
    /// Dependent type checking: bidirectional type checking with dependent types, elaboration to core calculus — .4 - Quantitative Type Theory
    fn quantities_compatible(q1: &Quantity, q2: &Quantity) -> bool {
        match (q1, q2) {
            // Same quantities always compatible
            (Quantity::Zero, Quantity::Zero)
            | (Quantity::One, Quantity::One)
            | (Quantity::Omega, Quantity::Omega) => true,

            // Erased (0) is subquantity of everything
            (Quantity::Zero, _) => true,

            // Linear (1) is subquantity of unrestricted (ω)
            (Quantity::One, Quantity::Omega) => true,

            // AtMost(n) is compatible if n1 <= n2
            (Quantity::AtMost(n1), Quantity::AtMost(n2)) => n1 <= n2,
            (Quantity::AtMost(_), Quantity::Omega) => true,
            (Quantity::One, Quantity::AtMost(n)) => *n >= 1,

            // Graded quantities must have same parameter
            (Quantity::Graded(n1), Quantity::Graded(n2)) => n1 == n2,

            // Other combinations don't unify directly
            _ => false,
        }
    }

    // ============================================
    // PROJECTION NORMALIZATION SUPPORT
    // ============================================

    /// Check if a type contains a projection (associated type access)
    ///
    /// Projections are represented as `Type::Generic { name: "T.Item", ... }`
    /// or `Type::Generic { name: "T::Item", ... }`.
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    pub fn contains_projection(ty: &Type) -> bool {
        match ty {
            Type::Generic { name, args } => {
                // Check if this type name looks like a projection
                let is_projection = name.contains(".") || name.contains("::");

                // Also check args for nested projections
                is_projection || args.iter().any(Self::contains_projection)
            }

            // Recursively check compound types
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(Self::contains_projection)
                    || Self::contains_projection(return_type)
            }

            Type::Named { args, .. } => args.iter().any(Self::contains_projection),

            Type::Tuple(types) => types.iter().any(Self::contains_projection),

            Type::Array { element, .. } => Self::contains_projection(element),

            Type::Slice { element } => Self::contains_projection(element),

            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => Self::contains_projection(inner),

            Type::Refined { base, .. } => Self::contains_projection(base),

            Type::Future { output } => Self::contains_projection(output),

            Type::GenRef { inner } => Self::contains_projection(inner),

            Type::TypeApp { constructor, args } => {
                Self::contains_projection(constructor) || args.iter().any(Self::contains_projection)
            }

            _ => false,
        }
    }

    /// Unify with projection normalization
    ///
    /// This method attempts to normalize any projections in the types before
    /// unifying. If normalization fails (e.g., because a type variable is not
    /// yet resolved), it defers the projection and continues with regular unification.
    ///
    /// # Arguments
    ///
    /// * `t1` - First type to unify
    /// * `t2` - Second type to unify
    /// * `protocol_checker` - Protocol checker for resolving projections
    /// * `span` - Source span for error messages
    ///
    /// # Returns
    ///
    /// * `Ok((subst, deferred))` - Substitution and any deferred projections
    /// * `Err(TypeError)` - Unification failed
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    pub fn unify_with_projections(
        &mut self,
        t1: &Type,
        t2: &Type,
        protocol_checker: &crate::protocol::ProtocolChecker,
        span: Span,
    ) -> Result<(Substitution, Vec<crate::projection::DeferredProjection>)> {
        use crate::projection::{ProjectionResolver, ProjectionResult};

        let mut deferred = Vec::new();
        let resolver = ProjectionResolver::new(protocol_checker, span);

        // Try to normalize t1
        let t1_normalized = if Self::contains_projection(t1) {
            match resolver.normalize(t1) {
                Ok(normalized) => normalized,
                Err(_) => t1.clone(), // Keep original on error
            }
        } else {
            t1.clone()
        };

        // Try to normalize t2
        let t2_normalized = if Self::contains_projection(t2) {
            match resolver.normalize(t2) {
                Ok(normalized) => normalized,
                Err(_) => t2.clone(), // Keep original on error
            }
        } else {
            t2.clone()
        };

        // Now unify the (possibly normalized) types
        let subst = self.unify(&t1_normalized, &t2_normalized, span)?;

        // Check for remaining projections that couldn't be normalized
        // These become deferred constraints
        if Self::contains_projection(&t1_normalized) {
            if let Some(proj) = crate::projection::parse_projection(&t1_normalized, span) {
                match resolver.resolve_projection(&proj) {
                    Ok(ProjectionResult::Deferred(d)) => deferred.push(d),
                    _ => {} // Already resolved or error
                }
            }
        }

        if Self::contains_projection(&t2_normalized) {
            if let Some(proj) = crate::projection::parse_projection(&t2_normalized, span) {
                match resolver.resolve_projection(&proj) {
                    Ok(ProjectionResult::Deferred(d)) => deferred.push(d),
                    _ => {} // Already resolved or error
                }
            }
        }

        Ok((subst, deferred))
    }

    /// Normalize a type by resolving all projections
    ///
    /// This is a convenience method that wraps the ProjectionResolver.
    ///
    /// # Arguments
    ///
    /// * `ty` - Type to normalize
    /// * `protocol_checker` - Protocol checker for resolving projections
    /// * `span` - Source span for error messages
    ///
    /// # Returns
    ///
    /// The normalized type, or the original type if normalization fails.
    pub fn normalize_projections(
        ty: &Type,
        protocol_checker: &crate::protocol::ProtocolChecker,
        span: Span,
    ) -> Type {
        if !Self::contains_projection(ty) {
            return ty.clone();
        }

        let resolver = crate::projection::ProjectionResolver::new(protocol_checker, span);
        match resolver.normalize(ty) {
            Ok(normalized) => {
                // The unifier's own projection reduction, with BOTH sides.
                // Two earlier candidates (`normalize_projection_type` in the
                // protocol registry, `reduce_projection_shape` in infer) are
                // silent on the T0997 repro; the reduction that hands back
                // the first call site's element type has to be reached from
                // somewhere, and this is the one the raise stack passes
                // through.
                if std::env::var("VERUM_TRACE_ASSOC").is_ok() {
                    eprintln!(
                        "[unify-norm] {} -> {}",
                        ty.to_text(),
                        normalized.to_text()
                    );
                }
                normalized
            }
            Err(e) => {
                // #306-followup: surface projection-resolution
                // failures so a `<X as Iterator>::Item` that can't
                // resolve via the active impl candidates produces
                // an audible error trail instead of silently
                // returning the unresolved Type::Projection.
                // Downstream type errors will then be traceable to
                // the actual cause (missing impl, ambiguous bound,
                // etc.) instead of "type mismatch on unresolved
                // projection".
                tracing::warn!(
                    "normalize_projections: projection resolution \
                     failed ({}) — returning unresolved Type::\
                     Projection.  Downstream type-mismatch errors \
                     may stem from this gap; verify the protocol \
                     bound has a concrete impl for the receiver.",
                    e
                );
                ty.clone()
            }
        }
    }

    /// Check if a type lives in the Prop universe (proof-irrelevant).
    /// Inductive types: recursive type definitions with structural recursion, termination checking — .1
    ///
    /// Returns true if the type's universe is Prop, meaning all inhabitants
    /// are definitionally equal (proof irrelevance).
    ///
    /// # Examples
    /// - Prop itself is in Prop (but we check the type-of)
    /// - Equality types Eq<A, x, y> can be in Prop
    /// - Inductive types declared in Prop universe
    /// - Sigma types with Prop in the second component
    fn is_type_in_prop(&self, ty: &Type) -> bool {
        match ty {
            // Prop itself is proof-irrelevant
            Type::Prop => true,

            // Equality types are often in Prop
            // (they live in the same universe as their carrier type)
            Type::Eq { ty: carrier_ty, .. } => self.is_type_in_prop(carrier_ty),

            // Inductive types: check if they're declared in a Prop-level universe
            // For simplicity, we check if universe level is 0 and named with Prop convention
            Type::Inductive { universe, .. } => {
                matches!(universe, UniverseLevel::Concrete(0))
            }

            // Sigma types with Prop in second component
            // (x: A, P(x)) where P(x) : Prop
            Type::Sigma { snd_type, .. } => self.is_type_in_prop(snd_type),

            // Types explicitly marked as proof-irrelevant via quantification
            Type::Quantified {
                quantity: Quantity::Zero,
                ..
            } => true,

            // Refinement types: check the base type
            Type::Refined { base, .. } => self.is_type_in_prop(base),

            // Other types are not proof-irrelevant
            _ => false,
        }
    }
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod alias_aware_family_pins {
    //! Pin tests for the alias-aware coercion-marker family
    //! lookups (`is_tensor_family` / `is_bytewise_ffi` / etc.).
    //! Each family-check method now consults the literal name
    //! first then falls through to `resolve_aliased_head_name`,
    //! letting alias names inherit their target's marker without
    //! a parallel hardcoded fallback.
    use super::*;

    fn named(name: &str) -> Type {
        Type::Named {
            path: verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::Ident::new(Text::from(name), Default::default()),
                )],
                span: Default::default(),
            },
            args: List::new(),
        }
    }

    /// `Tensor<T> is DynTensor<T>` — `is_tensor_family("Tensor")`
    /// must return true via alias resolution to `"DynTensor"`,
    /// even though the literal `"Tensor"` is *not* registered.
    /// Closes the architectural seam that previously kept
    /// `TENSOR_FAMILY_STDLIB_NAMES = ["Tensor"]` as a hardcoded
    /// alias-coverage fallback.
    #[test]
    fn tensor_family_resolves_through_alias() {
        let mut u = Unifier::new();
        u.register_tensor_family_type(Text::from("DynTensor"));
        u.register_type_alias(Text::from("Tensor"), named("DynTensor"));

        // Direct lookup on the registered head still works.
        assert!(u.is_tensor_family("DynTensor"));
        // Literal alias name not registered.
        assert!(!u.tensor_family_types.contains("Tensor"));
        // Alias-aware lookup resolves `Tensor → DynTensor`.
        assert!(u.is_tensor_family("Tensor"));
        // Types with no alias chain still produce the
        // direct-lookup result.
        assert!(!u.is_tensor_family("List"));
    }

    /// Darwin `type SockaddrIn = DarwinSockaddrIn;` —
    /// `is_bytewise_ffi("SockaddrIn")` must follow the alias to
    /// `"DarwinSockaddrIn"`, which carries
    /// `implement BytewiseFfi`.  Closes the architectural seam
    /// that previously kept `BYTEWISE_FFI_STDLIB_NAMES =
    /// ["SockaddrIn"]` as hardcoded alias coverage.
    #[test]
    fn bytewise_ffi_resolves_through_alias() {
        let mut u = Unifier::new();
        u.register_bytewise_ffi_type(Text::from("DarwinSockaddrIn"));
        u.register_type_alias(Text::from("SockaddrIn"), named("DarwinSockaddrIn"));
        assert!(u.is_bytewise_ffi("DarwinSockaddrIn"));
        assert!(u.is_bytewise_ffi("SockaddrIn"));
        // Sum-type / arbitrary names still rejected.
        assert!(!u.is_bytewise_ffi("SocketAddr"));
    }

    /// Multi-step alias chains resolve to their terminal head.
    /// `A is B; B is C; register_tensor_family(C)` —
    /// `is_tensor_family("A")` must walk both hops.
    #[test]
    fn alias_chain_walks_to_terminal_head() {
        let mut u = Unifier::new();
        u.register_tensor_family_type(Text::from("C"));
        u.register_type_alias(Text::from("B"), named("C"));
        u.register_type_alias(Text::from("A"), named("B"));

        assert_eq!(
            u.resolve_aliased_head_name("A"),
            Some("C".to_string()),
            "two-hop alias chain must resolve to its terminal head",
        );
        assert!(u.is_tensor_family("A"));
        assert!(u.is_tensor_family("B"));
        assert!(u.is_tensor_family("C"));
    }

    /// `resolve_aliased_head_name` returns `None` for names with
    /// no alias chain — the family-check methods then fall
    /// through to the direct-lookup result.
    #[test]
    fn resolve_aliased_head_returns_none_for_non_alias() {
        let u = Unifier::new();
        assert_eq!(u.resolve_aliased_head_name("UnknownType"), None);
        assert_eq!(u.resolve_aliased_head_name("List"), None);
    }

    /// Cross-marker cross-cutting pin: every family-check
    /// method (`is_tensor_family` / `is_bytewise_ffi` /
    /// `is_int_coercible_named` / `is_range_like` /
    /// `is_sized_numeric`) is alias-aware via the shared
    /// `family_contains` helper, so the alias rule applies
    /// uniformly across all five markers — adding a new
    /// family-check method must route through `family_contains`
    /// or this pin's pattern of cross-checks fails.
    #[test]
    fn all_family_checks_alias_aware_uniformly() {
        let mut u = Unifier::new();
        u.register_type_alias(Text::from("AliasName"), named("HeadName"));
        // Register HeadName in each family in turn and verify
        // AliasName follows.
        u.register_tensor_family_type(Text::from("HeadName"));
        assert!(u.is_tensor_family("AliasName"));
        u.register_bytewise_ffi_type(Text::from("HeadName"));
        assert!(u.is_bytewise_ffi("AliasName"));
        u.register_int_coercible_type(Text::from("HeadName"));
        assert!(u.is_int_coercible_named("AliasName"));
        u.register_range_like_type(Text::from("HeadName"));
        assert!(u.is_range_like("AliasName"));
        u.register_sized_numeric_type(Text::from("HeadName"));
        assert!(u.is_sized_numeric("AliasName"));
    }
}

#[cfg(test)]
mod var_var_age_discipline {
    //! When two unification variables meet, the OLDER one must survive as
    //! the representative — a lower id means a wider scope.
    //!
    //! Before this rule the direction came from argument POSITION, so
    //! `unify(T_declared, fresh)` pinned a declared type parameter to a
    //! body-local variable nothing solves. Because an `implement<T> …` block
    //! mints one var for `T` and shares it across every method, and the
    //! accumulated substitution is never reset between bodies, that binding
    //! leaked forward through the whole block (T0742).
    use super::*;

    #[test]
    fn older_var_survives_regardless_of_argument_order() {
        let older = TypeVar::fresh();
        let younger = TypeVar::fresh();
        assert!(older.id() < younger.id(), "fresh() must be monotone");

        // Both argument orders must reach the SAME representative — that
        // symmetry is the whole point; a positional rule passes one and
        // fails the other.
        for (first, second) in [(older, younger), (younger, older)] {
            let mut u = Unifier::new();
            u.unify(&Type::Var(first), &Type::Var(second), Default::default())
                .expect("two type variables always unify");
            assert_eq!(
                u.apply(&Type::Var(younger)),
                Type::Var(older),
                "younger must resolve to older (unify order {}, {})",
                first.id(),
                second.id(),
            );
            assert_eq!(
                u.apply(&Type::Var(older)),
                Type::Var(older),
                "the older var must stay free (unify order {}, {})",
                first.id(),
                second.id(),
            );
        }
    }

    /// A declared parameter unified with any number of body-local vars stays
    /// the representative, so a later scope still sees it unpinned. This is
    /// the shape the defect actually took: method after method in one
    /// `implement` block, each contributing its own fresh vars.
    #[test]
    fn declared_param_survives_a_run_of_body_locals() {
        let param = TypeVar::fresh();
        let mut u = Unifier::new();
        for _ in 0..8 {
            let body_local = TypeVar::fresh();
            u.unify(&Type::Var(param), &Type::Var(body_local), Default::default())
                .expect("two type variables always unify");
            assert_eq!(u.apply(&Type::Var(body_local)), Type::Var(param));
        }
        assert_eq!(
            u.apply(&Type::Var(param)),
            Type::Var(param),
            "the declared parameter must never be pinned by a body-local var"
        );
    }

    /// var→var edges now strictly decrease in id, so the substitution graph
    /// is acyclic by construction — `a := b` followed by `b := a` can no
    /// longer be built, whichever order the callers use.
    #[test]
    fn var_edges_cannot_form_a_cycle() {
        let a = TypeVar::fresh();
        let b = TypeVar::fresh();
        let mut u = Unifier::new();
        u.unify(&Type::Var(a), &Type::Var(b), Default::default())
            .expect("two type variables always unify");
        u.unify(&Type::Var(b), &Type::Var(a), Default::default())
            .expect("re-unifying the same pair is a no-op");
        // `apply` terminating with a bare var is the observable consequence:
        // a cycle would either loop or resolve to the younger var.
        assert_eq!(u.apply(&Type::Var(b)), Type::Var(a));
        assert_eq!(u.apply(&Type::Var(a)), Type::Var(a));
    }
}

#[cfg(test)]
mod variant_registration_rollback {
    use super::*;

    #[test]
    fn restores_existing_variant_bindings_and_removes_provisional_bindings() {
        let mut unifier = Unifier::new();
        let original = TypeVar::fresh();
        let provisional = TypeVar::fresh();
        unifier.register_variant_type_name("signature".into(), "Stable".into());
        unifier.register_original_variant_type("Stable".into(), Type::Var(original));
        unifier.register_type_var_order("Stable".into(), List::from_single(original));
        unifier.begin_variant_registration_journal();
        unifier.set_variant_type_names(Map::new());
        for name in ["Stable", "Child"] {
            unifier.register_variant_type_name(name.into(), name.into());
            unifier.register_original_variant_type(name.into(), Type::Var(provisional));
            unifier.register_type_var_order(name.into(), List::from_single(provisional));
        }
        unifier.finish_variant_registration_journal(false);
        assert_eq!(unifier.variant_type_names.get(&Text::from("signature")), Some(&Text::from("Stable")));
        assert_eq!(unifier.original_variant_types.get(&Text::from("Stable")), Some(&Type::Var(original)));
        assert_eq!(unifier.variant_parameters.get(&Text::from("Stable")), Some(&List::from_single(Type::Var(original))));
        assert!(!unifier.variant_type_names.contains_key(&Text::from("Child")));
        assert!(!unifier.original_variant_types.contains_key(&Text::from("Child")));
        assert!(!unifier.variant_parameters.contains_key(&Text::from("Child")));
        unifier.begin_variant_registration_journal();
        unifier.register_original_variant_type("Child".into(), Type::Var(provisional));
        unifier.finish_variant_registration_journal(true);
        assert_eq!(unifier.original_variant_types.get(&Text::from("Child")), Some(&Type::Var(provisional)));
    }
}

// Tests moved to tests/unify_tests.rs
