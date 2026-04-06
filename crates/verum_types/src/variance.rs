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
}

impl VarianceChecker {
    /// Create a new variance checker
    pub fn new() -> Self {
        Self { cache: Map::new() }
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

    /// Infer variance for constructor parameters based on well-known types
    ///
    /// This is a temporary implementation until we have a proper type definition registry.
    /// It provides correct variance for standard library types based on their documented behavior.
    ///
    /// Variance composition: covariant*covariant=covariant, covariant*contravariant=contravariant, any*invariant=invariant. Flip reverses covariant<->contravariant
    fn infer_constructor_variances(&self, path: &verum_ast::ty::Path) -> List<Variance> {
        // Get the last component of the path (the type name)
        use verum_ast::ty::PathSegment;
        let type_name = path
            .segments
            .last()
            .and_then(|seg| match seg {
                PathSegment::Name(id) => Some(id.name.as_ref()),
                _ => None,
            })
            .unwrap_or("");

        match type_name {
            // Covariant container types (read-only, immutable)
            WKT_LIST | "Vec" | "Array" | "Seq" => vec![Variance::Covariant].into(),
            WKT_SET | "HashSet" | "TreeSet" => vec![Variance::Covariant].into(),
            WKT_MAYBE | "Option" | WKT_RESULT => {
                // Maybe<+T>, Result<+T, +E>
                vec![Variance::Covariant, Variance::Covariant].into()
            }
            "Tree" | "Graph" => vec![Variance::Covariant].into(),

            // Invariant types (mutable, or bidirectional)
            "Cell" | "RefCell" | "Atomic" => {
                // Cell<T> has &mut-like semantics, must be invariant
                vec![Variance::Invariant].into()
            }
            "Ref" | "RefMut" => {
                // References to mutable containers
                vec![Variance::Invariant].into()
            }
            WKT_MAP | "HashMap" | "TreeMap" => {
                // Map<K, +V> - keys invariant (used in lookups), values covariant
                vec![Variance::Invariant, Variance::Covariant].into()
            }

            // Function types are handled separately
            "Fn" | "FnOnce" | "FnMut" => {
                // Fn(A) -> B is contravariant in A, covariant in B
                // But this is handled in the Function arm of variance_at_position
                vec![Variance::Contravariant, Variance::Covariant].into()
            }

            // Smart pointers (covariant)
            "Box" | "Rc" | "Arc" => vec![Variance::Covariant].into(),

            // Tensor types (covariant in element type)
            "Tensor" | "Matrix" | "Vector" => vec![Variance::Covariant].into(),

            // Conservative default: invariant for unknown types
            // This is safe but may reject valid variance annotations.
            // When we have a type registry, we'll look up the actual declared variance.
            _ => {
                // Return empty list - caller will use Invariant default
                List::new()
            }
        }
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
