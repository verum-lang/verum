//! Variance Inference and Checking
//!

//! Variance inference: determining covariant/contravariant/invariant usage of type parameters from their positions
//!

//! Variance determines how generic type parameters behave under subtyping.
//! This module implements variance inference and checking to ensure type soundness.
//!

//! # Variance Rules
//!

//! - **Covariant** (+T): If S <: T, then Container<S> <: Container<T>
//! - **Contravariant** (-T): If S <: T, then Container<T> <: Container<S>
//! - **Invariant** (T): Requires exact type match
//!

//! # Examples
//!

//! ```rust,ignore
//! use verum_types::variance::*;
//! use verum_types::ty::Type;
//! use verum_common::{List, Text};
//!

//! // Container<+T> with only covariant uses of T
//! // type Container<+T> is { value: T, get: Unit -> T }
//! // ✅ Covariant (safe)
//!

//! // Sink<-T> with only contravariant uses of T
//! // type Sink<-T> is { put: T -> Unit }
//! // ✅ Contravariant (safe)
//!

//! // Cell<T> with mutable reference
//! // type Cell<T> is { value: &mut T }
//! // ✅ Invariant (required for soundness)
//! ```

use crate::context::TypeParam;
use crate::ty::Type;
use verum_ast::span::Span;
use verum_common::well_known_types::type_names as wkt_names;
use verum_common::{List, Map, Maybe, Text};

const WKT_LIST: &str = wkt_names::LIST;
const WKT_SET: &str = wkt_names::SET;
const WKT_MAYBE: &str = wkt_names::MAYBE;
const WKT_RESULT: &str = wkt_names::RESULT;
const WKT_MAP: &str = wkt_names::MAP;

/// Variance of a type parameter
///

/// Determines how subtyping works for generic types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variance {
    /// Covariant: Container<S> <: Container<T> when S <: T
    Covariant,

    /// Contravariant: Container<T> <: Container<S> when S <: T
    Contravariant,

    /// Invariant: Requires exact type match
    Invariant,
}

/// Error when declared variance doesn't match inferred variance
#[derive(Debug, Clone)]
pub struct VarianceError {
    /// Name of the type parameter
    pub param_name: Text,

    /// Variance declared by the user
    pub declared: Variance,

    /// Variance inferred from type structure
    pub inferred: Variance,

    /// Location of the variance annotation
    pub span: Span,

    /// Explanation of why variance was inferred
    pub reason: Text,
}

impl std::fmt::Display for VarianceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let declared_str = match self.declared {
            Variance::Covariant => "covariant",
            Variance::Contravariant => "contravariant",
            Variance::Invariant => "invariant",
        };
        let inferred_str = match self.inferred {
            Variance::Covariant => "covariant",
            Variance::Contravariant => "contravariant",
            Variance::Invariant => "invariant",
        };
        write!(
            f,
            "variance mismatch for type parameter `{}`: declared {} but inferred {}\n  {}\n  help: {}",
            self.param_name,
            declared_str,
            inferred_str,
            self.reason,
            self.suggestion()
        )
    }
}

impl VarianceError {
    fn suggestion(&self) -> Text {
        match (self.declared, self.inferred) {
            (Variance::Covariant, Variance::Contravariant) => {
                format!("change `+{}` to `-{}`", self.param_name, self.param_name).into()
            }
            (Variance::Covariant, Variance::Invariant) => format!(
                "change `+{}` to `{}` (invariant)",
                self.param_name, self.param_name
            )
            .into(),
            (Variance::Contravariant, Variance::Covariant) => {
                format!("change `-{}` to `+{}`", self.param_name, self.param_name).into()
            }
            (Variance::Contravariant, Variance::Invariant) => format!(
                "change `-{}` to `{}` (invariant)",
                self.param_name, self.param_name
            )
            .into(),
            _ => "declared variance is more permissive than inferred variance".into(),
        }
    }
}

/// Variance inference engine
pub struct VarianceChecker {
    /// Cache of inferred variances for type definitions
    /// Key: (type_name, param_index) -> Variance
    cache: Map<(Text, usize), Variance>,
    /// Per-constructor variance overrides.  Stdlib loading or an
    /// embedder calls [`VarianceChecker::register_constructor_variances`]
    /// to feed the per-parameter variance recovered from a type's
    /// `@variance(...)` annotation or from structural inference; the
    /// override shadows [`DEFAULT_CONSTRUCTOR_VARIANCES`] for that
    /// type name.
    ///
    /// Pre-fix this lived inside `infer_constructor_variances` as a
    /// hardcoded match-arm with ~25 type names — the comment said
    /// "temporary implementation until we have a proper type
    /// definition registry". Replacing the match with
    /// `[overrides → defaults → empty]` lookup is the seam the
    /// type-registry plumbing was always missing: dynamic
    /// registration now slots in without touching this file.
    constructor_variances: std::collections::HashMap<Text, Vec<Variance>>,
}

/// Default per-parameter variance for the small set of well-known
/// constructor types whose variance is fixed by the language
/// definition or by Verum stdlib semantics.
///
/// **Why a default table at all?** Variance is properly a property
/// of the type DEFINITION, recovered from `@variance(...)`
/// annotations or inferred from the type body when stdlib is
/// loaded. Until that pass plumbs results into the
/// `VarianceChecker`, this table provides the minimal set of
/// defaults that variance-using sites (currently mostly tests + GAT
/// verification) expect to see — registering a new entry via
/// [`VarianceChecker::register_constructor_variances`] shadows the
/// table at runtime, so embedders / stdlib-loading can extend
/// without touching this constant.
///
/// **What's in here?** Verum stdlib semantic types (`List` /
/// `Maybe` / `Result` / `Map` / `Set` — pulled from
/// [`verum_common::well_known_types::type_names`] so a future
/// rename in the canonical name table flows through automatically),
/// plus a per-stdlib-mount-audit-accepted set of migration-ergonomic
/// Rust aliases (`Vec` / `Option` / `HashMap` / `Box` / `Rc` /
/// `Arc` / …). Function-type variance (`Fn` / `FnOnce` / `FnMut`,
/// contravariant-in-arg + covariant-in-return) is computed by the
/// `Type::Function` arm of `variance_at_position` and is
/// intentionally absent from this Named-type table.
///
/// Pinned by `default_constructor_variances_table_pinned` in this
/// crate's test suite — adding / removing / reordering entries
/// requires the pin to follow.
pub const DEFAULT_CONSTRUCTOR_VARIANCES: &[(&str, &[Variance])] = &[
    // ---- Verum stdlib semantic types (canonical names) ----
    (WKT_LIST, &[Variance::Covariant]),
    (WKT_SET, &[Variance::Covariant]),
    (WKT_MAYBE, &[Variance::Covariant]),
    (WKT_RESULT, &[Variance::Covariant, Variance::Covariant]),
    // Map<K, +V> — keys invariant (used in lookups), values covariant.
    (WKT_MAP, &[Variance::Invariant, Variance::Covariant]),

    // ---- Migration-ergonomic Rust aliases ----
    // (the same set `stdlib_mount_audit` admits as accepted-by-name)
    ("Vec", &[Variance::Covariant]),
    ("Array", &[Variance::Covariant]),
    ("Seq", &[Variance::Covariant]),
    ("HashSet", &[Variance::Covariant]),
    ("TreeSet", &[Variance::Covariant]),
    ("Option", &[Variance::Covariant]),
    ("HashMap", &[Variance::Invariant, Variance::Covariant]),
    ("TreeMap", &[Variance::Invariant, Variance::Covariant]),

    // ---- Tree / graph containers (covariant in element) ----
    ("Tree", &[Variance::Covariant]),
    ("Graph", &[Variance::Covariant]),

    // ---- Smart pointers (covariant) ----
    ("Heap", &[Variance::Covariant]),
    ("Shared", &[Variance::Covariant]),
    ("Box", &[Variance::Covariant]),
    ("Rc", &[Variance::Covariant]),
    ("Arc", &[Variance::Covariant]),

    // ---- Cell-like (invariant — &mut-style aliasing) ----
    ("Cell", &[Variance::Invariant]),
    ("RefCell", &[Variance::Invariant]),
    ("Atomic", &[Variance::Invariant]),
    ("Ref", &[Variance::Invariant]),
    ("RefMut", &[Variance::Invariant]),

    // ---- Lock-mediated interior mutability (invariant) ----
    // `Mutex<T>` and `RwLock<T>` lend `&mut T` through their
    // lock guards, so they must be invariant in T — same
    // architectural reason as `Cell` / `RefCell`.  Source-of-truth:
    // `core/sync/mutex.vr` and `core/sync/rwlock.vr`.
    ("Mutex", &[Variance::Invariant]),
    ("RwLock", &[Variance::Invariant]),

    // ---- Tensor family (covariant in element) ----
    ("Tensor", &[Variance::Covariant]),
    ("Matrix", &[Variance::Covariant]),
    ("Vector", &[Variance::Covariant]),
];

impl VarianceChecker {
    /// Create a new variance checker
    pub fn new() -> Self {
        Self {
            cache: Map::new(),
            constructor_variances: std::collections::HashMap::new(),
        }
    }

    /// Register a per-parameter variance list for a named
    /// constructor — overrides any default in
    /// [`DEFAULT_CONSTRUCTOR_VARIANCES`]. Call this from stdlib
    /// loading (after parsing each type's `@variance(...)`
    /// annotations) or from an embedder that knows variance for a
    /// type the default table doesn't carry.
    ///
    /// Idempotent — re-registering the same name with the same
    /// list is a no-op.
    pub fn register_constructor_variances(&mut self, name: &str, variances: Vec<Variance>) {
        self.constructor_variances
            .insert(Text::from(name), variances);
    }

    /// True iff `name` has a registered (non-default) variance
    /// override.  Inspection-only helper — exposed so callers can
    /// distinguish "variance came from stdlib's `@variance`
    /// annotation" from "variance fell back to the default
    /// table".
    pub fn has_registered_variance(&self, name: &str) -> bool {
        self.constructor_variances.contains_key(name)
    }

    /// Infer the variance of a type parameter in a type body
    ///

    /// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant///
    /// # Algorithm
    ///

    /// 1. Find all occurrences of the parameter in the type body
    /// 2. Compute variance at each occurrence position
    /// 3. Combine all variances (must be compatible)
    pub fn infer_variance(&mut self, param: &TypeParam, body: &Type) -> Variance {
        // Check cache first
        if let Maybe::Some(cached) = self.lookup_cache(&param.name, 0) {
            return cached;
        }

        // Collect all occurrences and their variances
        let variance = self.variance_at_position(&param.name, body, Variance::Covariant);

        // Cache the result
        self.cache_variance(&param.name, 0, variance);

        variance
    }

    /// Check that declared variance matches inferred variance
    ///

    /// Variance compatibility: checking that type parameter usage is consistent with declared variance
    pub fn check_variance(
        &mut self,
        param: &TypeParam,
        body: &Type,
        span: Span,
    ) -> Result<(), VarianceError> {
        let declared = param.variance;
        let inferred = self.infer_variance(param, body);

        if self.compatible_variance(declared, inferred) {
            Ok(())
        } else {
            Err(VarianceError {
                param_name: param.name.clone(),
                declared,
                inferred,
                span,
                reason: self.explain_variance(&param.name, body, inferred),
            })
        }
    }

    /// Compute variance at a specific position in the type
    ///

    /// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant
    fn variance_at_position(
        &mut self,
        param_name: &Text,
        ty: &Type,
        position_variance: Variance,
    ) -> Variance {
        match ty {
            // Function type: parameters are contravariant, return is covariant
            Type::Function {
                params,
                return_type,
                ..
            } => {
                let mut variances = List::new();

                // Parameters are in contravariant position
                for param_ty in params.iter() {
                    if self.occurs_in(param_name, param_ty) {
                        let param_var = self.variance_at_position(
                            param_name,
                            param_ty,
                            flip_variance(position_variance),
                        );
                        variances.push(param_var);
                    }
                }

                // Return type is in covariant position
                if self.occurs_in(param_name, return_type) {
                    let ret_var =
                        self.variance_at_position(param_name, return_type, position_variance);
                    variances.push(ret_var);
                }

                combine_variances(&variances)
            }

            // References: shared are covariant, mutable are invariant
            Type::Reference { mutable, inner } => {
                if *mutable {
                    // Mutable references force invariance
                    if self.occurs_in(param_name, inner) {
                        Variance::Invariant
                    } else {
                        position_variance
                    }
                } else {
                    // Shared references are covariant
                    self.variance_at_position(param_name, inner, position_variance)
                }
            }

            // Checked references: shared are covariant, mutable are invariant
            Type::CheckedReference { mutable, inner } => {
                if *mutable {
                    // Mutable references force invariance
                    if self.occurs_in(param_name, inner) {
                        Variance::Invariant
                    } else {
                        position_variance
                    }
                } else {
                    // Shared references are covariant
                    self.variance_at_position(param_name, inner, position_variance)
                }
            }

            // Unsafe references: shared are covariant, mutable are invariant
            Type::UnsafeReference { mutable, inner } => {
                if *mutable {
                    // Mutable references force invariance
                    if self.occurs_in(param_name, inner) {
                        Variance::Invariant
                    } else {
                        position_variance
                    }
                } else {
                    // Shared references are covariant
                    self.variance_at_position(param_name, inner, position_variance)
                }
            }

            // Named types that might be the parameter itself, or a generic type
            Type::Named { path, args } => {
                // Check if this Named type IS the parameter we're looking for
                // (when type parameters are represented as Named types)
                let type_name = path.segments.last().and_then(|seg| {
                    use verum_ast::ty::PathSegment;
                    match seg {
                        PathSegment::Name(id) => Some(&id.name),
                        _ => None,
                    }
                });

                if let Some(name) = type_name
                    && name == param_name
                {
                    // This IS the type parameter - return the position variance
                    return position_variance;
                }

                // Generic types: apply variance composition
                // Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant//
                // For a generic type C<T>, if T appears in position with variance v1,
                // and C's parameter has declared variance v2, then the combined variance
                // is compose_variance(v2, v1).
                //

                // Example: If we have Container<+T> (covariant) and we're checking
                // variance of S in Container<Foo<S>>, and S is covariant in Foo,
                // then S is covariant in Container<Foo<S>> (compose Covariant ∘ Covariant).
                let mut variances = List::new();

                // For each type argument, compose with the parameter's declared variance
                // Since we don't have a type definition registry yet, we use a conservative
                // approach: assume covariance for standard library types and invariance for others.
                //

                // Standard library covariant types: List, Set, Maybe, Tree, etc.
                // Invariant types: Map (keys), Cell, Ref, etc.
                let param_variances = self.infer_constructor_variances(path);

                for (i, arg) in args.iter().enumerate() {
                    if self.occurs_in(param_name, arg) {
                        // Get the declared variance of this parameter (default to invariant if unknown)
                        let param_var = param_variances
                            .get(i)
                            .copied()
                            .unwrap_or(Variance::Invariant);

                        // Compute variance at this argument position
                        let arg_var = self.variance_at_position(param_name, arg, position_variance);

                        // Compose: outer parameter variance ∘ inner argument variance
                        let composed = compose_variance(param_var, arg_var);
                        variances.push(composed);
                    }
                }

                combine_variances(&variances)
            }

            // Type variables - these don't have names, so we can't match on them directly
            // Variance checking works at a higher level with TypeParam names
            Type::Var(_) => Variance::Covariant,

            // Tuple: all elements in same variance position
            Type::Tuple(elements) => {
                let mut variances = List::new();
                for elem in elements.iter() {
                    if self.occurs_in(param_name, elem) {
                        let elem_var =
                            self.variance_at_position(param_name, elem, position_variance);
                        variances.push(elem_var);
                    }
                }
                combine_variances(&variances)
            }

            // Record: all fields in same variance position
            Type::Record(fields) => {
                let mut variances = List::new();
                for (_name, field_ty) in fields.iter() {
                    if self.occurs_in(param_name, field_ty) {
                        let field_var =
                            self.variance_at_position(param_name, field_ty, position_variance);
                        variances.push(field_var);
                    }
                }
                combine_variances(&variances)
            }

            // Variant: all variants in same variance position
            Type::Variant(variants) => {
                let mut variances = List::new();
                for (_name, variant_ty) in variants.iter() {
                    if self.occurs_in(param_name, variant_ty) {
                        let variant_var =
                            self.variance_at_position(param_name, variant_ty, position_variance);
                        variances.push(variant_var);
                    }
                }
                combine_variances(&variances)
            }

            // Array: elements in same variance position
            Type::Array { element, .. } => {
                self.variance_at_position(param_name, element, position_variance)
            }

            // Tensor: elements in same variance position
            Type::Tensor { element, .. } => {
                self.variance_at_position(param_name, element, position_variance)
            }

            // Refined types: check the base type
            Type::Refined { base, .. } => {
                self.variance_at_position(param_name, base, position_variance)
            }

            // Meta parameters: check the base type (ty field, not base)
            Type::Meta { ty, .. } => self.variance_at_position(param_name, ty, position_variance),

            // Primitives and other types don't contain the parameter
            _ => Variance::Covariant, // Doesn't affect variance
        }
    }

    /// Check if a type parameter occurs in a type
    fn occurs_in(&self, param_name: &Text, ty: &Type) -> bool {
        match ty {
            Type::Named { path, args } => {
                // Check if this Named type IS the parameter
                let is_param = path
                    .segments
                    .last()
                    .and_then(|seg| {
                        use verum_ast::ty::PathSegment;
                        match seg {
                            PathSegment::Name(id) => Some(&id.name),
                            _ => None,
                        }
                    })
                    .map(|name| name == param_name)
                    .unwrap_or(false);

                is_param || args.iter().any(|a| self.occurs_in(param_name, a))
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.occurs_in(param_name, p))
                    || self.occurs_in(param_name, return_type)
            }
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => self.occurs_in(param_name, inner),
            Type::Tuple(elements) => elements.iter().any(|e| self.occurs_in(param_name, e)),
            Type::Record(fields) => fields
                .iter()
                .any(|(_name, ty)| self.occurs_in(param_name, ty)),
            Type::Variant(variants) => variants
                .iter()
                .any(|(_name, ty)| self.occurs_in(param_name, ty)),
            Type::Array { element, .. } => self.occurs_in(param_name, element),
            Type::Tensor { element, .. } => self.occurs_in(param_name, element),
            Type::Refined { base, .. } => self.occurs_in(param_name, base),
            Type::Meta { ty, .. } => self.occurs_in(param_name, ty),
            _ => false,
        }
    }

    /// Check if declared variance is compatible with inferred variance
    ///

    /// Variance compatibility: checking that type parameter usage is consistent with declared variance
    fn compatible_variance(&self, declared: Variance, inferred: Variance) -> bool {
        match (declared, inferred) {
            // Exact match is always OK
            (d, i) if d == i => true,

            // Declared invariant is always safe (most restrictive)
            (Variance::Invariant, _) => true,

            // Otherwise, must match exactly
            _ => false,
        }
    }

    /// Generate an explanation for why a variance was inferred
    fn explain_variance(&mut self, param_name: &Text, ty: &Type, variance: Variance) -> Text {
        match variance {
            Variance::Covariant => format!(
                "type parameter `{}` appears only in covariant positions",
                param_name
            )
            .into(),
            Variance::Contravariant => format!(
                "type parameter `{}` appears only in contravariant positions (function arguments)",
                param_name
            )
            .into(),
            Variance::Invariant => {
                if self.has_mutable_ref(param_name, ty) {
                    format!(
                        "type parameter `{}` appears in mutable reference (forces invariance)",
                        param_name
                    )
                    .into()
                } else {
                    format!(
                        "type parameter `{}` appears in both covariant and contravariant positions",
                        param_name
                    )
                    .into()
                }
            }
        }
    }

    /// Check if a parameter appears in a mutable reference
    fn has_mutable_ref(&self, param_name: &Text, ty: &Type) -> bool {
        match ty {
            Type::Reference { mutable, inner }
            | Type::CheckedReference { mutable, inner }
            | Type::UnsafeReference { mutable, inner } => {
                *mutable && self.occurs_in(param_name, inner)
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.has_mutable_ref(param_name, p))
                    || self.has_mutable_ref(param_name, return_type)
            }
            Type::Named { args, .. } => args.iter().any(|a| self.has_mutable_ref(param_name, a)),
            Type::Tuple(elements) => elements.iter().any(|e| self.has_mutable_ref(param_name, e)),
            Type::Record(fields) => fields
                .iter()
                .any(|(_name, ty)| self.has_mutable_ref(param_name, ty)),
            Type::Variant(variants) => variants
                .iter()
                .any(|(_name, ty)| self.has_mutable_ref(param_name, ty)),
            Type::Array { element, .. } => self.has_mutable_ref(param_name, element),
            Type::Tensor { element, .. } => self.has_mutable_ref(param_name, element),
            Type::Refined { base, .. } => self.has_mutable_ref(param_name, base),
            Type::Meta { ty, .. } => self.has_mutable_ref(param_name, ty),
            _ => false,
        }
    }

    /// Look up cached variance
    fn lookup_cache(&self, param_name: &Text, index: usize) -> Maybe<Variance> {
        self.cache.get(&(param_name.clone(), index)).copied()
    }

    /// Cache a variance result
    fn cache_variance(&mut self, param_name: &Text, index: usize, variance: Variance) {
        self.cache.insert((param_name.clone(), index), variance);
    }

    /// Infer variance for constructor parameters by name.
    ///
    /// Lookup chain (first hit wins):
    ///   1. **Instance overrides** in `self.constructor_variances`
    ///      — registered via
    ///      [`Self::register_constructor_variances`] from stdlib
    ///      loading or by an embedder.
    ///   2. **Static defaults** in
    ///      [`DEFAULT_CONSTRUCTOR_VARIANCES`] — the curated
    ///      well-known-types + migration-ergonomic-aliases table.
    ///   3. **Empty list** — caller defaults to `Variance::Invariant`
    ///      (the conservative-but-safe choice).
    ///
    /// This replaces the prior hardcoded `match type_name { … }`
    /// that mixed canonical Verum names (`List` / `Maybe` /
    /// `Result` / `Map` / `Set`) with Rust-style migration aliases
    /// (`Vec` / `Option` / `HashMap` / …) inside one large arm
    /// table — adding a new constructor required editing this file,
    /// even when stdlib already encoded the variance via
    /// `@variance(...)` declarations. The two-tier lookup gives
    /// dynamic registration (tier 1) AND a static fallback for the
    /// well-known set (tier 2) without parallel rule sites.
    ///
    /// Variance composition: covariant ∘ covariant = covariant;
    /// covariant ∘ contravariant = contravariant; any ∘ invariant
    /// = invariant. Flip reverses covariant ↔ contravariant.
    fn infer_constructor_variances(&self, path: &verum_ast::ty::Path) -> List<Variance> {
        use verum_ast::ty::PathSegment;
        let type_name = path
            .segments
            .last()
            .and_then(|seg| match seg {
                PathSegment::Name(id) => Some(id.name.as_ref()),
                _ => None,
            })
            .unwrap_or("");

        // Tier 1: instance-level overrides.
        if let Some(variances) = self.constructor_variances.get(type_name) {
            return variances.iter().copied().collect::<Vec<_>>().into();
        }

        // Tier 2: static defaults table — single source of truth
        // for the well-known Verum + migration-alias set.
        for (default_name, defaults) in DEFAULT_CONSTRUCTOR_VARIANCES {
            if *default_name == type_name {
                return defaults.iter().copied().collect::<Vec<_>>().into();
            }
        }

        // Tier 3: no data → caller falls back to Invariant.
        List::new()
    }
}

impl Default for VarianceChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// Flip variance: Covariant ↔ Contravariant, Invariant stays Invariant
///

/// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant
pub fn flip_variance(v: Variance) -> Variance {
    match v {
        Variance::Covariant => Variance::Contravariant,
        Variance::Contravariant => Variance::Covariant,
        Variance::Invariant => Variance::Invariant,
    }
}

/// Compose variances: outer variance ∘ inner variance
///

/// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant
///

/// # Examples
///

/// ```text
/// Covariant ∘ Covariant = Covariant
/// Covariant ∘ Contravariant = Contravariant
/// Contravariant ∘ Covariant = Contravariant
/// Contravariant ∘ Contravariant = Covariant
/// Invariant ∘ _ = Invariant
/// _ ∘ Invariant = Invariant
/// ```
pub fn compose_variance(outer: Variance, inner: Variance) -> Variance {
    match (outer, inner) {
        (Variance::Covariant, v) => v,
        (Variance::Contravariant, Variance::Covariant) => Variance::Contravariant,
        (Variance::Contravariant, Variance::Contravariant) => Variance::Covariant,
        (Variance::Contravariant, Variance::Invariant) => Variance::Invariant,
        (Variance::Invariant, _) => Variance::Invariant,
    }
}

/// Combine multiple variances into a single variance
///

/// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant
///

/// # Rules
///

/// - If any is Invariant, result is Invariant
/// - If both Covariant and Contravariant present, result is Invariant
/// - If all Covariant, result is Covariant
/// - If all Contravariant, result is Contravariant
/// - Default: Invariant
pub fn combine_variances(variances: &List<Variance>) -> Variance {
    if variances.is_empty() {
        return Variance::Covariant; // Vacuously covariant
    }

    // If any is invariant, result is invariant
    if variances.iter().any(|v| *v == Variance::Invariant) {
        return Variance::Invariant;
    }

    // Check if we have both covariant and contravariant
    let has_covariant = variances.iter().any(|v| *v == Variance::Covariant);
    let has_contravariant = variances.iter().any(|v| *v == Variance::Contravariant);

    if has_covariant && has_contravariant {
        return Variance::Invariant;
    }

    // All covariant
    if has_covariant {
        return Variance::Covariant;
    }

    // All contravariant
    if has_contravariant {
        return Variance::Contravariant;
    }

    // Default: invariant
    Variance::Invariant
}

#[cfg(test)]
mod default_constructor_variances_pins {
    use super::*;

    /// Pin the entries of [`DEFAULT_CONSTRUCTOR_VARIANCES`] so a
    /// future edit can't silently change Verum's variance posture
    /// for a stdlib type — adding / removing / re-ordering an entry
    /// must be a deliberate test edit. The table is *the* canonical
    /// source for these defaults; before this pin a typo in the
    /// list (e.g. `List → Invariant`) would leak into every
    /// variance-using site.
    #[test]
    fn default_constructor_variances_table_pinned() {
        // Build a name → variances lookup so the assertion order
        // doesn't depend on the table's internal ordering.
        let table: std::collections::HashMap<&str, &[Variance]> =
            DEFAULT_CONSTRUCTOR_VARIANCES.iter().copied().collect();

        // Total entry count — drift-pinned so a future addition
        // forces this test to follow.
        assert_eq!(
            DEFAULT_CONSTRUCTOR_VARIANCES.len(),
            table.len(),
            "DEFAULT_CONSTRUCTOR_VARIANCES has duplicate names",
        );
        // 5 Verum semantic + 8 Rust aliases + 2 tree/graph
        // + 5 smart pointers + 5 cell-likes + 2 lock-mediated
        // + 3 tensor-family = 30.
        assert_eq!(
            DEFAULT_CONSTRUCTOR_VARIANCES.len(),
            30,
            "DEFAULT_CONSTRUCTOR_VARIANCES entry count drifted — \
             update both the table and this pin together",
        );

        // Verum stdlib semantic types (canonical names from
        // verum_common::well_known_types::type_names).
        assert_eq!(table.get(WKT_LIST).copied(), Some(&[Variance::Covariant][..]));
        assert_eq!(table.get(WKT_SET).copied(), Some(&[Variance::Covariant][..]));
        assert_eq!(table.get(WKT_MAYBE).copied(), Some(&[Variance::Covariant][..]));
        assert_eq!(
            table.get(WKT_RESULT).copied(),
            Some(&[Variance::Covariant, Variance::Covariant][..]),
        );
        assert_eq!(
            table.get(WKT_MAP).copied(),
            Some(&[Variance::Invariant, Variance::Covariant][..]),
        );

        // Migration-ergonomic Rust aliases — same shape as the
        // canonical-name siblings.
        assert_eq!(table.get("Vec").copied(), Some(&[Variance::Covariant][..]));
        assert_eq!(table.get("Option").copied(), Some(&[Variance::Covariant][..]));
        assert_eq!(
            table.get("HashMap").copied(),
            Some(&[Variance::Invariant, Variance::Covariant][..]),
        );

        // Smart pointers — all covariant.
        for ptr in ["Heap", "Shared", "Box", "Rc", "Arc"] {
            assert_eq!(
                table.get(ptr).copied(),
                Some(&[Variance::Covariant][..]),
                "smart pointer `{}` must be covariant",
                ptr,
            );
        }

        // Cell-likes — all invariant.
        for cell in ["Cell", "RefCell", "Atomic", "Ref", "RefMut"] {
            assert_eq!(
                table.get(cell).copied(),
                Some(&[Variance::Invariant][..]),
                "cell-like `{}` must be invariant",
                cell,
            );
        }

        // Lock-mediated containers — invariant because their lock
        // guards expose `&mut T`.
        for lock in ["Mutex", "RwLock"] {
            assert_eq!(
                table.get(lock).copied(),
                Some(&[Variance::Invariant][..]),
                "lock-mediated `{}` must be invariant",
                lock,
            );
        }

        // Tensor family — all covariant in element.
        for tensor in ["Tensor", "Matrix", "Vector"] {
            assert_eq!(
                table.get(tensor).copied(),
                Some(&[Variance::Covariant][..]),
                "tensor-family `{}` must be covariant",
                tensor,
            );
        }

        // Function types are NOT in this Named-type table — their
        // variance is computed by the `Type::Function` arm of
        // `variance_at_position`. A future regression that adds
        // them here would silently shadow the structural rule.
        assert!(
            table.get("Fn").is_none(),
            "Function types must not appear in the Named-type table",
        );
        assert!(table.get("FnOnce").is_none());
        assert!(table.get("FnMut").is_none());
    }

    /// `register_constructor_variances` overrides
    /// [`DEFAULT_CONSTRUCTOR_VARIANCES`] for the registered name —
    /// a stdlib that declares
    /// `type Container<+T>` with a non-default variance can flow
    /// the result into the checker without editing the source
    /// file's defaults table.
    #[test]
    fn register_constructor_variances_overrides_defaults() {
        let mut checker = VarianceChecker::new();

        // Helper to construct a one-segment Path from a name.
        fn name_path(name: &str) -> verum_ast::ty::Path {
            verum_ast::ty::Path {
                segments: smallvec::smallvec![verum_ast::ty::PathSegment::Name(
                    verum_ast::Ident::new(verum_common::Text::from(name), Default::default()),
                )],
                span: Default::default(),
            }
        }

        // Default for Vec is [Covariant] — the override changes it.
        let pre_override =
            checker.infer_constructor_variances(&name_path("Vec"));
        assert_eq!(
            pre_override.as_slice(),
            &[Variance::Covariant],
            "default Vec variance must be Covariant",
        );
        assert!(!checker.has_registered_variance("Vec"));

        checker.register_constructor_variances("Vec", vec![Variance::Invariant]);
        assert!(checker.has_registered_variance("Vec"));

        let post_override =
            checker.infer_constructor_variances(&name_path("Vec"));
        assert_eq!(
            post_override.as_slice(),
            &[Variance::Invariant],
            "registered Vec variance must shadow the default",
        );

        // Unregistered names still go through the defaults table.
        let list = checker.infer_constructor_variances(&name_path(WKT_LIST));
        assert_eq!(list.as_slice(), &[Variance::Covariant]);

        // Names not in either tier produce an empty list — caller
        // falls back to Invariant.
        let unknown = checker.infer_constructor_variances(&name_path("__no_such_type__"));
        assert!(unknown.as_slice().is_empty());
    }

    /// Cross-consistency pin: every name in
    /// [`DEFAULT_CONSTRUCTOR_VARIANCES`] produces an identical
    /// variance list whether queried via `VarianceChecker`
    /// (through the `infer_constructor_variances` path) or via
    /// the sibling `crate::subtype::Subtyping` checker (through
    /// `variances_for_type_name`).
    ///
    /// Pre-fix Subtyping carried its own hardcoded variance match
    /// that contradicted this table on `Shared` (Subtyping said
    /// Invariant; canonical says Covariant — `Shared<T>` is a
    /// reference-counted shared pointer with no interior
    /// mutability) and was missing entirely for `Mutex` /
    /// `RwLock`.  Both gaps are now closed: Subtyping delegates
    /// to `DEFAULT_CONSTRUCTOR_VARIANCES`, so the table is the
    /// single source of truth for both consumers — drift between
    /// the two surfaces is structurally impossible.
    #[test]
    fn subtyping_and_variance_checker_agree_on_canonical_table() {
        for (name, expected) in DEFAULT_CONSTRUCTOR_VARIANCES {
            let arg_count = expected.len();
            let from_subtyping = crate::subtype::Subtyping::variances_for_type_name_for_test(
                name, arg_count,
            );
            assert_eq!(
                from_subtyping.as_slice(),
                *expected,
                "Subtyping disagrees with DEFAULT_CONSTRUCTOR_VARIANCES on `{}`",
                name,
            );
        }
    }
}
