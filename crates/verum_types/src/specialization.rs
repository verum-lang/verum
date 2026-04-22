//! Specialization Validation and Enforcement
//!
//! Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — Protocol Specialization
//!
//! This module implements the validation layer for @specialize attribute, ensuring:
//! 1. Specialized implementations are actually more specific than base implementations
//! 2. No overlapping implementations exist without a specialization relationship
//! 3. Specialization lattice is well-formed (coherence)
//!
//! # Architecture
//!
//! The specialization system works in two phases:
//!
//! ## Compile-Time Validation (this module)
//! - Detect overlapping implementations
//! - Verify specialization relationships
//! - Build specialization lattice
//! - Check coherence constraints
//!
//! ## Runtime Selection (specialization_selection.rs)
//! - Select most specific implementation
//! - Cache selection decisions
//! - Handle method dispatch
//!
//! # Example
//!
//! ```verum
//! // General implementation
//! implement<T> Clone for Maybe<T> where T: Clone {
//!     default fn clone(self: &Self) -> Self {
//!         match self {
//!             Some(x) => Some(x.clone()),
//!             None => None,
//!         }
//!     }
//! }
//!
//! // Specialized implementation (more specific)
//! @specialize
//! implement<T> Clone for Maybe<T> where T: Copy {
//!     fn clone(self: &Self) -> Self {
//!         *self  // More efficient for Copy types
//!     }
//! }
//! ```

use thiserror::Error;
use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_common::{List, Map, Maybe, Set, Text};

use crate::TypeError;
use crate::advanced_protocols::{SpecializationInfo, SpecializationLattice};
use crate::protocol::{Protocol, ProtocolBound, ProtocolChecker, ProtocolImpl, WhereClause};
use crate::ty::Type;
use crate::ty::TypeVar;

// ==================== Error Types ====================

/// Specialization validation error
#[derive(Debug, Clone, Error)]
pub enum SpecializationValidationError {
    /// Implementation marked @specialize but not more specific than any base impl
    #[error("implementation marked @specialize but not more specific than base")]
    NotMoreSpecific {
        specialized_impl: Path,
        base_impl: Maybe<Path>,
        reason: Text,
        span: Span,
    },

    /// Overlapping implementations without specialization relationship
    #[error("overlapping implementations detected")]
    OverlappingImpls {
        impl1: Path,
        impl2: Path,
        overlap_type: Type,
        span: Span,
    },

    /// Cyclic specialization dependency
    #[error("cyclic specialization detected")]
    CyclicSpecialization { cycle: List<Path>, span: Span },

    /// Invalid lattice structure
    #[error("specialization lattice is malformed")]
    MalformedLattice { reason: Text, span: Span },

    /// Default method in non-base implementation
    #[error("default methods only allowed in base implementations")]
    DefaultInSpecialization { method: Text, span: Span },
}

// ==================== Overlap Detection ====================

/// Detects overlapping protocol implementations
pub struct OverlapDetector {
    /// Detected overlaps
    overlaps: List<(usize, usize, Type)>,
}

impl OverlapDetector {
    /// Create a new overlap detector
    pub fn new() -> Self {
        Self {
            overlaps: List::new(),
        }
    }

    /// Detect all overlapping implementations for a protocol
    ///
    /// Two implementations overlap if there exists a type that could match both.
    /// This is conservative - we may report false positives to be safe.
    ///
    /// # Algorithm
    ///
    /// For each pair of implementations (I1, I2):
    /// 1. Try to unify their for_types
    /// 2. If unification succeeds, they overlap
    /// 3. Check if a specialization relationship exists
    /// 4. If no relationship, report error
    pub fn detect_overlaps(
        &mut self,
        protocol: &Protocol,
        impls: &[ProtocolImpl],
    ) -> Result<(), List<SpecializationValidationError>> {
        self.overlaps.clear();
        let mut errors = List::new();

        // Check all pairs of implementations
        for (i, impl1) in impls.iter().enumerate() {
            for (j, impl2) in impls.iter().enumerate() {
                if i >= j {
                    continue; // Only check each pair once
                }

                // Check if implementations overlap
                if let Some(overlap_ty) = self.check_overlap(&impl1.for_type, &impl2.for_type) {
                    self.overlaps.push((i, j, overlap_ty.clone()));

                    // Check if a specialization relationship exists
                    if !self.has_specialization_relationship(impl1, impl2) {
                        errors.push(SpecializationValidationError::OverlappingImpls {
                            impl1: impl1.protocol.clone(),
                            impl2: impl2.protocol.clone(),
                            overlap_type: overlap_ty,
                            span: impl1.span,
                        });
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check if two types overlap (could be unified)
    ///
    /// Returns the overlapping type if they overlap, None otherwise.
    ///
    /// Two types overlap if there exists a substitution that makes them equal.
    /// This is a conservative check - it may report overlap where none exists,
    /// but will never miss an actual overlap.
    ///
    /// # Algorithm
    ///
    /// We use a simplified unification-based approach:
    /// 1. Type variables overlap with anything
    /// 2. Named types with same constructor overlap if all args overlap
    /// 3. Structural types (tuples, functions, etc.) overlap if components overlap
    /// 4. Generic types with same base overlap if type arguments can unify
    pub fn check_overlap(&self, ty1: &Type, ty2: &Type) -> Option<Type> {
        // Use a mutable substitution to track unification
        let mut subst: Map<Text, Type> = Map::new();
        if self.check_overlap_with_subst(ty1, ty2, &mut subst) {
            // Return the unified type (apply substitution to ty1)
            Some(self.apply_subst(ty1, &subst))
        } else {
            None
        }
    }

    /// Internal overlap check with substitution tracking
    fn check_overlap_with_subst(
        &self,
        ty1: &Type,
        ty2: &Type,
        subst: &mut Map<Text, Type>,
    ) -> bool {
        match (ty1, ty2) {
            // Type variables - check or extend substitution
            (Type::Var(v1), Type::Var(v2)) if v1 == v2 => true,

            (Type::Var(var), other) | (other, Type::Var(var)) => {
                // Use type variable ID as a unique key
                let var_key = Text::from(format!("_tv{}", var.id()));
                // Check if already bound
                if let Some(existing) = subst.get(&var_key).cloned() {
                    // Must be compatible with existing binding
                    self.check_overlap_with_subst(&existing, other, subst)
                } else {
                    // Occurs check: variable cannot appear in the type it's being bound to
                    if self.occurs_in_var(var, other) {
                        false
                    } else {
                        // Bind the variable
                        subst.insert(var_key, other.clone());
                        true
                    }
                }
            }

            // Identical primitive types
            (Type::Unit, Type::Unit)
            | (Type::Bool, Type::Bool)
            | (Type::Int, Type::Int)
            | (Type::Float, Type::Float)
            | (Type::Char, Type::Char)
            | (Type::Text, Type::Text)
            | (Type::Never, Type::Never) => true,

            // Named types - same path required
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) => {
                // Paths must match
                if !self.paths_equal(p1, p2) {
                    return false;
                }
                // Arity must match
                if args1.len() != args2.len() {
                    return false;
                }
                // All arguments must overlap
                args1
                    .iter()
                    .zip(args2.iter())
                    .all(|(a1, a2)| self.check_overlap_with_subst(a1, a2, subst))
            }

            // Generic types
            (
                Type::Generic {
                    name: n1,
                    args: args1,
                },
                Type::Generic {
                    name: n2,
                    args: args2,
                },
            ) => {
                if n1 != n2 || args1.len() != args2.len() {
                    return false;
                }
                args1
                    .iter()
                    .zip(args2.iter())
                    .all(|(a1, a2)| self.check_overlap_with_subst(a1, a2, subst))
            }

            // Function types
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    ..
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    ..
                },
            ) => {
                // Arity must match
                if p1.len() != p2.len() {
                    return false;
                }
                // All parameters must overlap
                let params_ok = p1
                    .iter()
                    .zip(p2.iter())
                    .all(|(param1, param2)| self.check_overlap_with_subst(param1, param2, subst));
                // Return type must overlap
                params_ok && self.check_overlap_with_subst(r1.as_ref(), r2.as_ref(), subst)
            }

            // Tuple types
            (Type::Tuple(t1), Type::Tuple(t2)) => {
                if t1.len() != t2.len() {
                    return false;
                }
                t1.iter()
                    .zip(t2.iter())
                    .all(|(a, b)| self.check_overlap_with_subst(a, b, subst))
            }

            // Array types
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => {
                // Sizes must match (if known)
                if let (Some(size1), Some(size2)) = (s1, s2) {
                    if size1 != size2 {
                        return false;
                    }
                }
                self.check_overlap_with_subst(e1.as_ref(), e2.as_ref(), subst)
            }

            // Reference types
            (
                Type::Reference {
                    inner: i1,
                    mutable: m1,
                    ..
                },
                Type::Reference {
                    inner: i2,
                    mutable: m2,
                    ..
                },
            ) => {
                // Mutability must match or one must be immutable
                if *m1 && !*m2 {
                    return false; // Can't overlap mut ref with immut ref
                }
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            // Checked and Unsafe references
            (
                Type::CheckedReference {
                    inner: i1,
                    mutable: m1,
                    ..
                },
                Type::CheckedReference {
                    inner: i2,
                    mutable: m2,
                    ..
                },
            ) => {
                if m1 != m2 {
                    return false;
                }
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            (
                Type::UnsafeReference {
                    inner: i1,
                    mutable: m1,
                    ..
                },
                Type::UnsafeReference {
                    inner: i2,
                    mutable: m2,
                    ..
                },
            ) => {
                if m1 != m2 {
                    return false;
                }
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            // Refined types - overlap if base types overlap
            (Type::Refined { base: b1, .. }, Type::Refined { base: b2, .. }) => {
                self.check_overlap_with_subst(b1.as_ref(), b2.as_ref(), subst)
            }

            // One refined, one not - check base
            (Type::Refined { base, .. }, other) | (other, Type::Refined { base, .. }) => {
                self.check_overlap_with_subst(base.as_ref(), other, subst)
            }

            // Slice types
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.check_overlap_with_subst(e1.as_ref(), e2.as_ref(), subst)
            }

            // Pointer types
            (
                Type::Pointer {
                    inner: i1,
                    mutable: m1,
                },
                Type::Pointer {
                    inner: i2,
                    mutable: m2,
                },
            ) => {
                if m1 != m2 {
                    return false;
                }
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            // Ownership types
            (
                Type::Ownership {
                    inner: i1,
                    mutable: m1,
                },
                Type::Ownership {
                    inner: i2,
                    mutable: m2,
                },
            ) => {
                if m1 != m2 {
                    return false;
                }
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            // GenRef types
            (Type::GenRef { inner: i1 }, Type::GenRef { inner: i2 }) => {
                // GenRef overlap is determined by inner type overlap
                self.check_overlap_with_subst(i1.as_ref(), i2.as_ref(), subst)
            }

            // Dependent types (Pi, Sigma, Eq)
            (
                Type::Pi {
                    param_type: p1,
                    return_type: r1,
                    ..
                },
                Type::Pi {
                    param_type: p2,
                    return_type: r2,
                    ..
                },
            ) => {
                self.check_overlap_with_subst(p1.as_ref(), p2.as_ref(), subst)
                    && self.check_overlap_with_subst(r1.as_ref(), r2.as_ref(), subst)
            }

            (
                Type::Sigma {
                    fst_type: f1,
                    snd_type: s1,
                    ..
                },
                Type::Sigma {
                    fst_type: f2,
                    snd_type: s2,
                    ..
                },
            ) => {
                self.check_overlap_with_subst(f1.as_ref(), f2.as_ref(), subst)
                    && self.check_overlap_with_subst(s1.as_ref(), s2.as_ref(), subst)
            }

            // No overlap for different type constructors
            _ => false,
        }
    }

    /// Check if a type variable occurs in a type (occurs check for unification)
    fn occurs_in_var(&self, var: &TypeVar, ty: &Type) -> bool {
        match ty {
            Type::Var(v) => v.id() == var.id(),

            Type::Named { args, .. } | Type::Generic { args, .. } => {
                args.iter().any(|a| self.occurs_in_var(var, a))
            }

            Type::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.occurs_in_var(var, p))
                    || self.occurs_in_var(var, return_type.as_ref())
            }

            Type::Tuple(types) => types.iter().any(|t| self.occurs_in_var(var, t)),

            Type::Array { element, .. }
            | Type::Slice { element }
            | Type::Reference { inner: element, .. }
            | Type::CheckedReference { inner: element, .. }
            | Type::UnsafeReference { inner: element, .. }
            | Type::Pointer { inner: element, .. }
            | Type::Ownership { inner: element, .. }
            | Type::GenRef { inner: element }
            | Type::Refined { base: element, .. } => self.occurs_in_var(var, element.as_ref()),

            Type::Pi {
                param_type,
                return_type,
                ..
            } => {
                self.occurs_in_var(var, param_type.as_ref())
                    || self.occurs_in_var(var, return_type.as_ref())
            }

            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                self.occurs_in_var(var, fst_type.as_ref())
                    || self.occurs_in_var(var, snd_type.as_ref())
            }

            _ => false,
        }
    }

    /// Check if two paths are equal
    fn paths_equal(&self, p1: &Path, p2: &Path) -> bool {
        use verum_ast::ty::PathSegment;

        if p1.segments.len() != p2.segments.len() {
            return false;
        }

        p1.segments
            .iter()
            .zip(p2.segments.iter())
            .all(|(s1, s2)| match (s1, s2) {
                (PathSegment::Name(n1), PathSegment::Name(n2)) => n1.name == n2.name,
                (PathSegment::SelfValue, PathSegment::SelfValue)
                | (PathSegment::Super, PathSegment::Super)
                | (PathSegment::Cog, PathSegment::Cog)
                | (PathSegment::Relative, PathSegment::Relative) => true,
                _ => false,
            })
    }

    /// Apply a substitution to a type
    fn apply_subst(&self, ty: &Type, subst: &Map<Text, Type>) -> Type {
        match ty {
            Type::Var(var) => {
                // TypeVar uses ID-based lookup
                let var_key = Text::from(format!("_tv{}", var.id()));
                if let Some(replacement) = subst.get(&var_key) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }

            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args.iter().map(|a| self.apply_subst(a, subst)).collect(),
            },

            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args.iter().map(|a| self.apply_subst(a, subst)).collect(),
            },

            Type::Function {
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                params: params.iter().map(|p| self.apply_subst(p, subst)).collect(),
                return_type: Box::new(self.apply_subst(return_type.as_ref(), subst)),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },

            Type::Tuple(types) => {
                Type::Tuple(types.iter().map(|t| self.apply_subst(t, subst)).collect())
            }

            Type::Array { element, size } => Type::Array {
                element: Box::new(self.apply_subst(element.as_ref(), subst)),
                size: *size,
            },

            Type::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(self.apply_subst(inner.as_ref(), subst)),
                mutable: *mutable,
            },

            // For other types, return as-is (they don't contain type variables in their structure)
            _ => ty.clone(),
        }
    }

    /// Check if two implementations have a specialization relationship.
    ///
    /// Two implementations have a specialization relationship if:
    /// 1. One declares `@specialize` and its for_type is more specific, OR
    /// 2. One has strictly more restrictive where clauses covering the same type space
    ///
    /// This prevents false-positive overlap errors when a valid specialization exists.
    fn has_specialization_relationship(&self, impl1: &ProtocolImpl, impl2: &ProtocolImpl) -> bool {
        // Check if either impl explicitly declares specialization
        let impl1_specializes = matches!(&impl1.specialization, Maybe::Some(spec) if spec.is_specialized);
        let impl2_specializes = matches!(&impl2.specialization, Maybe::Some(spec) if spec.is_specialized);

        if impl1_specializes || impl2_specializes {
            // At least one declares specialization - verify the relationship is valid
            // by checking that the specializing impl's type is at least as specific
            if impl1_specializes {
                return self.type_specificity_score(&impl1.for_type)
                    >= self.type_specificity_score(&impl2.for_type);
            }
            if impl2_specializes {
                return self.type_specificity_score(&impl2.for_type)
                    >= self.type_specificity_score(&impl1.for_type);
            }
        }
        false
    }

    /// Compute a specificity score for a type.
    ///
    /// Higher scores indicate more specific types (fewer type variables, more concrete).
    /// Used to validate specialization relationships.
    fn type_specificity_score(&self, ty: &Type) -> usize {
        match ty {
            // Concrete types are maximally specific
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text
            | Type::Never => 10,

            // Named types: specific + score of args
            Type::Named { args, .. } => 5 + args.iter().map(|a| self.type_specificity_score(a)).sum::<usize>(),

            // Generic types: somewhat specific
            Type::Generic { args, .. } => 3 + args.iter().map(|a| self.type_specificity_score(a)).sum::<usize>(),

            // Type variables are least specific
            Type::Var(_) => 0,

            // Compound types: sum of components
            Type::Tuple(elems) => 2 + elems.iter().map(|e| self.type_specificity_score(e)).sum::<usize>(),
            Type::Function { params, return_type, .. } => {
                2 + params.iter().map(|p| self.type_specificity_score(p)).sum::<usize>()
                    + self.type_specificity_score(return_type)
            }
            Type::Array { element, .. } => 2 + self.type_specificity_score(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Slice { element: inner }
            | Type::GenRef { inner } => 1 + self.type_specificity_score(inner),
            Type::Refined { base, .. } => 1 + self.type_specificity_score(base),

            _ => 1,
        }
    }

    /// Get detected overlaps
    pub fn overlaps(&self) -> &[(usize, usize, Type)] {
        &self.overlaps
    }
}

impl Default for OverlapDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Specialization Validator ====================

/// Validates specialization relationships
pub struct SpecializationValidator {
    /// Overlap detector
    overlap_detector: OverlapDetector,
}

impl SpecializationValidator {
    /// Create a new specialization validator
    pub fn new() -> Self {
        Self {
            overlap_detector: OverlapDetector::new(),
        }
    }

    /// Validate all specializations for a protocol
    ///
    /// This is the main entry point for specialization validation.
    ///
    /// # Checks Performed
    ///
    /// 1. **Overlap Detection**: Ensure no overlapping impls without specialization
    /// 2. **Specificity Validation**: @specialize impls must be more specific
    /// 3. **Lattice Well-Formedness**: Check for cycles and consistency
    /// 4. **Default Method Placement**: Defaults only in base impls
    pub fn validate_specializations(
        &mut self,
        protocol: &Protocol,
        impls: &[ProtocolImpl],
    ) -> Result<(), List<SpecializationValidationError>> {
        let mut errors = List::new();

        // 1. Detect overlaps
        if let Err(overlap_errors) = self.overlap_detector.detect_overlaps(protocol, impls) {
            errors.extend(overlap_errors);
        }

        // 2. Validate each specialized implementation
        for (i, impl_info) in impls.iter().enumerate() {
            if let Maybe::Some(spec_info) = &impl_info.specialization
                && spec_info.is_specialized
            {
                // Find base implementation
                if let Err(e) = self.validate_specialization(impl_info, impls, i) {
                    errors.push(e);
                }
            }
        }

        // 3. Check lattice well-formedness
        if let Err(lattice_errors) = self.validate_lattice(protocol, impls) {
            errors.extend(lattice_errors);
        }

        // 4. Validate default method placement
        if let Err(default_errors) = self.validate_default_methods(impls) {
            errors.extend(default_errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate a single specialization relationship
    ///
    /// Ensures that the specialized impl is actually more specific than base impls
    fn validate_specialization(
        &self,
        specialized: &ProtocolImpl,
        all_impls: &[ProtocolImpl],
        specialized_idx: usize,
    ) -> Result<(), SpecializationValidationError> {
        let spec_info = match &specialized.specialization {
            Maybe::Some(info) => info,
            Maybe::None => {
                return Err(SpecializationValidationError::NotMoreSpecific {
                    specialized_impl: specialized.protocol.clone(),
                    base_impl: Maybe::None,
                    reason: "missing specialization metadata".into(),
                    span: specialized.span,
                });
            }
        };

        // Find base implementations that this specializes
        let mut found_base = false;

        for (i, base) in all_impls.iter().enumerate() {
            if i == specialized_idx {
                continue;
            }

            // Check if this could be a base for our specialization
            if self.could_be_base(specialized, base) {
                found_base = true;

                // Verify that specialized is actually more specific
                if !self.is_more_specific(&specialized.for_type, &base.for_type) {
                    return Err(SpecializationValidationError::NotMoreSpecific {
                        specialized_impl: specialized.protocol.clone(),
                        base_impl: Maybe::Some(base.protocol.clone()),
                        reason: "specialized impl is not more specific than base".into(),
                        span: specialized.span,
                    });
                }

                // Verify where clauses are more restrictive
                if !self
                    .where_clauses_more_restrictive(&specialized.where_clauses, &base.where_clauses)
                {
                    return Err(SpecializationValidationError::NotMoreSpecific {
                        specialized_impl: specialized.protocol.clone(),
                        base_impl: Maybe::Some(base.protocol.clone()),
                        reason: "where clauses are not more restrictive".into(),
                        span: specialized.span,
                    });
                }
            }
        }

        if !found_base {
            return Err(SpecializationValidationError::NotMoreSpecific {
                specialized_impl: specialized.protocol.clone(),
                base_impl: Maybe::None,
                reason: "no base implementation found to specialize".into(),
                span: specialized.span,
            });
        }

        Ok(())
    }

    /// Check if base could be a base implementation for specialized
    fn could_be_base(&self, specialized: &ProtocolImpl, base: &ProtocolImpl) -> bool {
        // Same protocol
        if specialized.protocol != base.protocol {
            return false;
        }

        // Base should not itself be a specialization
        if let Maybe::Some(base_spec) = &base.specialization
            && base_spec.is_specialized
        {
            return false;
        }

        // Types should overlap
        self.overlap_detector
            .check_overlap(&specialized.for_type, &base.for_type)
            .is_some()
    }

    /// Check if ty1 is more specific than ty2
    ///
    /// A type is more specific if it has fewer type variables and more concrete types.
    /// Any concrete type (primitive, named, tuple, etc.) is more specific than a type variable.
    fn is_more_specific(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            // Any concrete type is more specific than a type variable
            (_, Type::Var(_)) if !matches!(ty1, Type::Var(_)) => true,
            (Type::Var(_), _) if !matches!(ty2, Type::Var(_)) => false,

            // Compare compound types with same constructor
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) if p1 == p2 => {
                // More specific if any argument is more specific
                args1
                    .iter()
                    .zip(args2.iter())
                    .any(|(a1, a2)| self.is_more_specific(a1, a2))
            }

            (
                Type::Generic {
                    name: n1,
                    args: args1,
                },
                Type::Generic {
                    name: n2,
                    args: args2,
                },
            ) if n1 == n2 => {
                args1
                    .iter()
                    .zip(args2.iter())
                    .any(|(a1, a2)| self.is_more_specific(a1, a2))
            }

            // Tuple types
            (Type::Tuple(t1), Type::Tuple(t2)) if t1.len() == t2.len() => {
                t1.iter()
                    .zip(t2.iter())
                    .any(|(a, b)| self.is_more_specific(a, b))
            }

            // Equal types are not more specific
            _ => false,
        }
    }

    /// Check if where clauses are more restrictive
    ///
    /// Specialized impl should have all constraints of base plus potentially more
    fn where_clauses_more_restrictive(
        &self,
        specialized: &[WhereClause],
        base: &[WhereClause],
    ) -> bool {
        // For each base constraint, specialized must have it or something stricter
        for base_clause in base {
            let mut found = false;

            for spec_clause in specialized {
                if self.types_compatible(&spec_clause.ty, &base_clause.ty) {
                    // Check if specialized has all bounds of base (or more)
                    if self.bounds_more_restrictive(&spec_clause.bounds, &base_clause.bounds) {
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                return false;
            }
        }

        true
    }

    /// Check if types are compatible for where clause comparison.
    ///
    /// Two types are compatible if they can potentially unify, meaning there exists
    /// a substitution that makes them equal. This is used when checking if where clause
    /// constraints match between a base and specialized implementation.
    ///
    /// # Compatibility Rules
    ///
    /// 1. Type variables are compatible with any type (they can be instantiated)
    /// 2. Primitive types must match exactly
    /// 3. Named/Generic types must have matching paths/names and compatible arguments
    /// 4. Compound types (Function, Tuple, Array, etc.) must be structurally compatible
    /// 5. Reference types must have matching mutability and compatible inner types
    fn types_compatible(&self, ty1: &Type, ty2: &Type) -> bool {
        match (ty1, ty2) {
            // Type variables are compatible with anything (unification will bind them)
            (Type::Var(_), _) | (_, Type::Var(_)) => true,

            // Identical primitive types are compatible
            (Type::Unit, Type::Unit)
            | (Type::Bool, Type::Bool)
            | (Type::Int, Type::Int)
            | (Type::Float, Type::Float)
            | (Type::Char, Type::Char)
            | (Type::Text, Type::Text)
            | (Type::Never, Type::Never) => true,

            // Never type is compatible with any type (bottom type)
            (Type::Never, _) | (_, Type::Never) => true,

            // Named types - path and args must be compatible
            (
                Type::Named {
                    path: p1,
                    args: args1,
                },
                Type::Named {
                    path: p2,
                    args: args2,
                },
            ) => {
                self.overlap_detector.paths_equal(p1, p2)
                    && args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2.iter())
                        .all(|(a1, a2)| self.types_compatible(a1, a2))
            }

            // Generic types - name and args must be compatible.
            // Numeric aliases (`u64` ↔ `UInt64`, etc.) normalize via
            // `Type::canonical_primitive` so literal-synthesized types
            // match user-declared parameter types.
            (
                Type::Generic {
                    name: n1,
                    args: args1,
                },
                Type::Generic {
                    name: n2,
                    args: args2,
                },
            ) => {
                Type::canonical_primitive(n1.as_str())
                    == Type::canonical_primitive(n2.as_str())
                    && args1.len() == args2.len()
                    && args1
                        .iter()
                        .zip(args2.iter())
                        .all(|(a1, a2)| self.types_compatible(a1, a2))
            }

            // Function types - params and return type must be compatible
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                    contexts: c1,
                    ..
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                    contexts: c2,
                    ..
                },
            ) => {
                p1.len() == p2.len()
                    && c1 == c2
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|(a, b)| self.types_compatible(a, b))
                    && self.types_compatible(r1, r2)
            }

            // Tuple types - element-wise compatible
            (Type::Tuple(t1), Type::Tuple(t2)) => {
                t1.len() == t2.len()
                    && t1
                        .iter()
                        .zip(t2.iter())
                        .all(|(a, b)| self.types_compatible(a, b))
            }

            // Array types - element compatible and sizes compatible
            (
                Type::Array {
                    element: e1,
                    size: s1,
                },
                Type::Array {
                    element: e2,
                    size: s2,
                },
            ) => {
                self.types_compatible(e1, e2)
                    && match (s1, s2) {
                        (Some(n1), Some(n2)) => n1 == n2,
                        _ => true, // Unknown sizes are compatible
                    }
            }

            // Slice types
            (Type::Slice { element: e1 }, Type::Slice { element: e2 }) => {
                self.types_compatible(e1, e2)
            }

            // Reference types - mutability and inner must match
            (
                Type::Reference {
                    mutable: m1,
                    inner: i1,
                },
                Type::Reference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            (
                Type::CheckedReference {
                    mutable: m1,
                    inner: i1,
                },
                Type::CheckedReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            (
                Type::UnsafeReference {
                    mutable: m1,
                    inner: i1,
                },
                Type::UnsafeReference {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            // Ownership types
            (
                Type::Ownership {
                    mutable: m1,
                    inner: i1,
                },
                Type::Ownership {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            // Pointer types
            (
                Type::Pointer {
                    mutable: m1,
                    inner: i1,
                },
                Type::Pointer {
                    mutable: m2,
                    inner: i2,
                },
            ) => m1 == m2 && self.types_compatible(i1, i2),

            // Refinement types - base types must be compatible
            (Type::Refined { base: b1, .. }, Type::Refined { base: b2, .. }) => {
                self.types_compatible(b1, b2)
            }
            (Type::Refined { base, .. }, other) | (other, Type::Refined { base, .. }) => {
                self.types_compatible(base, other)
            }

            // Record types - all fields must be compatible
            (Type::Record(f1), Type::Record(f2)) => {
                f1.len() == f2.len()
                    && f1.iter().all(|(name, ty1)| {
                        f2.get(name)
                            .map(|ty2| self.types_compatible(ty1, ty2))
                            .unwrap_or(false)
                    })
            }

            // Variant types - all variants must be compatible
            (Type::Variant(v1), Type::Variant(v2)) => {
                v1.len() == v2.len()
                    && v1.iter().all(|(tag, ty1)| {
                        v2.get(tag)
                            .map(|ty2| self.types_compatible(ty1, ty2))
                            .unwrap_or(false)
                    })
            }

            // Pi types (dependent functions) - param and return must be compatible
            (
                Type::Pi {
                    param_type: p1,
                    return_type: r1,
                    ..
                },
                Type::Pi {
                    param_type: p2,
                    return_type: r2,
                    ..
                },
            ) => self.types_compatible(p1, p2) && self.types_compatible(r1, r2),

            // Sigma types (dependent pairs)
            (
                Type::Sigma {
                    fst_type: f1,
                    snd_type: s1,
                    ..
                },
                Type::Sigma {
                    fst_type: f2,
                    snd_type: s2,
                    ..
                },
            ) => self.types_compatible(f1, f2) && self.types_compatible(s1, s2),

            // Universe types - levels must be compatible
            (Type::Universe { level: l1 }, Type::Universe { level: l2 }) => {
                self.universe_levels_compatible(l1, l2)
            }

            // Prop types
            (Type::Prop, Type::Prop) => true,

            // Inductive types
            (
                Type::Inductive {
                    name: n1,
                    params: p1,
                    ..
                },
                Type::Inductive {
                    name: n2,
                    params: p2,
                    ..
                },
            ) => {
                n1 == n2
                    && p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|((_, t1), (_, t2))| self.types_compatible(t1, t2))
            }

            // Coinductive types
            (
                Type::Coinductive {
                    name: n1,
                    params: p1,
                    ..
                },
                Type::Coinductive {
                    name: n2,
                    params: p2,
                    ..
                },
            ) => {
                n1 == n2
                    && p1.len() == p2.len()
                    && p1
                        .iter()
                        .zip(p2.iter())
                        .all(|((_, t1), (_, t2))| self.types_compatible(t1, t2))
            }

            // Future types
            (Type::Future { output: o1 }, Type::Future { output: o2 }) => {
                self.types_compatible(o1, o2)
            }

            // Generator types
            (
                Type::Generator {
                    yield_ty: y1,
                    return_ty: r1,
                },
                Type::Generator {
                    yield_ty: y2,
                    return_ty: r2,
                },
            ) => self.types_compatible(y1, y2) && self.types_compatible(r1, r2),

            // Meta types
            (
                Type::Meta {
                    name: n1, ty: t1, ..
                },
                Type::Meta {
                    name: n2, ty: t2, ..
                },
            ) => n1 == n2 && self.types_compatible(t1, t2),

            // Quantified types
            (
                Type::Quantified {
                    inner: i1,
                    quantity: q1,
                },
                Type::Quantified {
                    inner: i2,
                    quantity: q2,
                },
            ) => q1 == q2 && self.types_compatible(i1, i2),

            // TypeApp types
            (
                Type::TypeApp {
                    constructor: c1,
                    args: a1,
                },
                Type::TypeApp {
                    constructor: c2,
                    args: a2,
                },
            ) => {
                self.types_compatible(c1, c2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(t1, t2)| self.types_compatible(t1, t2))
            }

            // Different type constructors are not compatible
            _ => false,
        }
    }

    /// Check if two universe levels are compatible.
    ///
    /// Levels are compatible if:
    /// 1. Both are concrete and equal
    /// 2. At least one is a variable (can be unified)
    /// 3. Both have the same structure (Max, Succ)
    fn universe_levels_compatible(
        &self,
        l1: &crate::ty::UniverseLevel,
        l2: &crate::ty::UniverseLevel,
    ) -> bool {
        use crate::ty::UniverseLevel;

        match (l1, l2) {
            (UniverseLevel::Concrete(n1), UniverseLevel::Concrete(n2)) => n1 == n2,
            (UniverseLevel::Variable(_), _) | (_, UniverseLevel::Variable(_)) => true,
            (UniverseLevel::Max(a1, b1), UniverseLevel::Max(a2, b2)) => a1 == a2 && b1 == b2,
            (UniverseLevel::Succ(n1), UniverseLevel::Succ(n2)) => n1 == n2,
            _ => false,
        }
    }

    /// Check if bounds are more restrictive
    fn bounds_more_restrictive(&self, spec: &[ProtocolBound], base: &[ProtocolBound]) -> bool {
        // Specialized must have all base bounds
        for base_bound in base {
            if !spec.iter().any(|s| s.protocol == base_bound.protocol) {
                return false;
            }
        }
        true
    }

    /// Validate specialization lattice structure
    ///
    /// Ensures no cycles and lattice is well-formed
    fn validate_lattice(
        &self,
        protocol: &Protocol,
        impls: &[ProtocolImpl],
    ) -> Result<(), List<SpecializationValidationError>> {
        let mut errors = List::new();

        // Build dependency graph
        let mut deps: Map<usize, Set<usize>> = Map::new();

        for (i, impl1) in impls.iter().enumerate() {
            for (j, impl2) in impls.iter().enumerate() {
                if i != j && self.is_more_specific(&impl1.for_type, &impl2.for_type) {
                    deps.entry(i).or_default().insert(j);
                }
            }
        }

        // Check for cycles using DFS
        for i in 0..impls.len() {
            let mut visited = Set::new();
            let mut stack = List::new();

            if let Some(cycle) = self.detect_cycle(i, &deps, &mut visited, &mut stack, impls) {
                errors.push(SpecializationValidationError::CyclicSpecialization {
                    cycle,
                    span: impls[i].span,
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Detect cycles in specialization graph using DFS
    fn detect_cycle(
        &self,
        node: usize,
        deps: &Map<usize, Set<usize>>,
        visited: &mut Set<usize>,
        stack: &mut List<usize>,
        impls: &[ProtocolImpl],
    ) -> Option<List<Path>> {
        if stack.contains(&node) {
            // Found a cycle
            let start = stack.iter().position(|&n| n == node).unwrap_or(0);
            let cycle_paths: List<Path> = stack
                .iter()
                .skip(start)
                .map(|&i| impls[i].protocol.clone())
                .collect();
            return Some(cycle_paths);
        }

        if visited.contains(&node) {
            return None;
        }

        visited.insert(node);
        stack.push(node);

        if let Some(successors) = deps.get(&node) {
            for &succ in successors.iter() {
                if let Some(cycle) = self.detect_cycle(succ, deps, visited, stack, impls) {
                    return Some(cycle);
                }
            }
        }

        stack.pop();
        None
    }

    /// Validate default method placement
    ///
    /// Default methods should only appear in base (non-specialized) implementations
    fn validate_default_methods(
        &self,
        impls: &[ProtocolImpl],
    ) -> Result<(), List<SpecializationValidationError>> {
        let mut errors = List::new();

        for impl_info in impls {
            if let Maybe::Some(spec_info) = &impl_info.specialization
                && spec_info.is_specialized
                && spec_info.is_default
            {
                // This is a specialized impl with default methods - error
                for method_name in impl_info.methods.keys() {
                    errors.push(SpecializationValidationError::DefaultInSpecialization {
                        method: method_name.clone(),
                        span: impl_info.span,
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Get the overlap detector
    pub fn overlap_detector(&self) -> &OverlapDetector {
        &self.overlap_detector
    }
}

impl Default for SpecializationValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Integration with TypeError ====================

impl SpecializationValidationError {
    /// Convert to TypeError for reporting
    pub fn to_type_error(self) -> TypeError {
        match self {
            SpecializationValidationError::NotMoreSpecific {
                specialized_impl,
                base_impl,
                reason,
                span,
            } => TypeError::Other(
                format!(
                    "invalid specialization: implementation marked @specialize is not more specific\n  \
                     specialized impl: {}\n  \
                     base impl: {}\n  \
                     reason: {}\n  \
                     help: ensure specialized impl has more concrete types or more restrictive where clauses",
                    specialized_impl,
                    base_impl.as_ref().map(|p| p.to_string()).unwrap_or_else(|| "none".to_string()),
                    reason
                )
                .into(),
            ),

            SpecializationValidationError::OverlappingImpls {
                impl1,
                impl2,
                overlap_type,
                span,
            } => {
                // Helper to format type for user-friendly display
                let format_type = |ty: &Type| match ty {
                    Type::Named { path, .. } => path.to_string(),
                    Type::Var(v) => format!("type variable {}", v.id()),
                    _ => "unknown type".to_string(),
                };

                TypeError::Other(
                    format!(
                        "overlapping protocol implementations detected\n  \
                         impl 1: {}\n  \
                         impl 2: {}\n  \
                         overlap on type: {}\n  \
                         help: add @specialize to the more specific implementation\n  \
                         help: or use where clauses to make implementations non-overlapping",
                        impl1, impl2, format_type(&overlap_type)
                    )
                    .into(),
                )
            }

            SpecializationValidationError::CyclicSpecialization { cycle, span } => {
                let cycle_str = cycle.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" -> ");
                TypeError::Other(
                    format!(
                        "cyclic specialization detected\n  \
                         cycle: {}\n  \
                         help: remove circular specialization dependencies",
                        cycle_str
                    )
                    .into(),
                )
            }

            SpecializationValidationError::MalformedLattice { reason, span } => TypeError::Other(
                format!(
                    "specialization lattice is malformed\n  \
                     reason: {}\n  \
                     help: check specialization relationships for consistency",
                    reason
                )
                .into(),
            ),

            SpecializationValidationError::DefaultInSpecialization { method, span } => {
                TypeError::Other(
                    format!(
                        "default method '{}' in specialized implementation\n  \
                         help: default methods are only allowed in base (non-specialized) implementations\n  \
                         help: remove 'default' keyword from specialized impl",
                        method
                    )
                    .into(),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::TypeVar;
    use verum_ast::ty::Ident;

    fn make_simple_type(name: &str) -> Type {
        Type::Named {
            path: Path::single(Ident::new(name, Span::default())),
            args: List::new(),
        }
    }

    fn make_generic_type(name: &str, arg: Type) -> Type {
        Type::Named {
            path: Path::single(Ident::new(name, Span::default())),
            args: vec![arg].into(),
        }
    }

    #[test]
    fn test_overlap_detection_identical_types() {
        let detector = OverlapDetector::new();
        let ty1 = make_simple_type("Int");
        let ty2 = make_simple_type("Int");

        assert!(detector.check_overlap(&ty1, &ty2).is_some());
    }

    #[test]
    fn test_overlap_detection_different_types() {
        let detector = OverlapDetector::new();
        let ty1 = make_simple_type("Int");
        let ty2 = make_simple_type("Text");

        assert!(detector.check_overlap(&ty1, &ty2).is_none());
    }

    #[test]
    fn test_overlap_detection_generic_types() {
        let detector = OverlapDetector::new();
        let ty1 = make_generic_type("List", make_simple_type("Int"));
        let ty2 = make_generic_type("List", make_simple_type("Int"));

        assert!(detector.check_overlap(&ty1, &ty2).is_some());
    }

    #[test]
    fn test_specificity_concrete_vs_variable() {
        let validator = SpecializationValidator::new();
        let concrete = make_simple_type("Int");
        let var = Type::Var(TypeVar::with_id(0));

        assert!(validator.is_more_specific(&concrete, &var));
        assert!(!validator.is_more_specific(&var, &concrete));
    }

    #[test]
    fn test_specificity_partial_specialization() {
        let validator = SpecializationValidator::new();
        let specific = make_generic_type("List", make_simple_type("Int"));
        let generic = make_generic_type("List", Type::Var(TypeVar::with_id(0)));

        assert!(validator.is_more_specific(&specific, &generic));
    }
}
