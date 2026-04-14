//! Associated Type Projection Resolution
//!
//! This module implements resolution and normalization of associated type projections
//! like `T.Item` where `T` implements `Iterator`.
//!
//! # Overview
//!
//! Associated type projections occur when accessing an associated type through a
//! type parameter or concrete type. For example:
//!
//! ```verum
//! fn process<I>(iter: I) -> List<I.Item>
//!     where type I: Iterator,
//!           type I.Item: Display
//! {
//!     iter.map(|x| x.to_string()).collect()
//! }
//! ```
//!
//! In this example, `I.Item` is a projection that must be resolved:
//! 1. When `I = List<Int>`, resolve `List<Int>.Item` to `Int`
//! 2. When `I` is still a type variable, create a deferred projection constraint
//!
//! # Spec References
//!
//! - Associated type bounds: constraining associated types in where clauses (where T.Item: Display)
//! - GATs: associated types with their own type parameters for lending iterators and monadic patterns
//! - Protocol system: method resolution, default impls, associated types, protocol objects

use crate::protocol::{ProtocolChecker, ProtocolError, ProtocolImpl};
use crate::ty::{Substitution, SubstitutionExt, Type, TypeVar};
use crate::TypeError;
use thiserror::Error;
use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_common::{List, Map, Text};
use verum_common::ToText;

/// Errors that can occur during projection resolution
#[derive(Debug, Error, Clone)]
pub enum ProjectionError {
    /// Type does not implement the required protocol
    #[error("Cannot resolve associated type: type `{ty}` does not implement protocol `{protocol}`")]
    TypeDoesNotImplementProtocol { ty: Text, protocol: Text, span: Span },

    /// Associated type not found in protocol
    #[error("Protocol `{protocol}` has no associated type named `{assoc_name}`")]
    AssociatedTypeNotFound {
        protocol: Text,
        assoc_name: Text,
        span: Span,
    },

    /// Associated type not specified in implementation
    #[error("Implementation of `{protocol}` for `{ty}` does not specify associated type `{assoc_name}`")]
    AssociatedTypeNotSpecified {
        protocol: Text,
        ty: Text,
        assoc_name: Text,
        span: Span,
    },

    /// Ambiguous associated type: multiple implementations could apply
    #[error("Ambiguous associated type: type `{ty}` has multiple implementations that could provide `{assoc_name}`")]
    AmbiguousAssociatedType {
        ty: Text,
        assoc_name: Text,
        candidates: List<Text>,
        span: Span,
    },

    /// Cannot resolve projection on unresolved type variable
    #[error("Cannot resolve projection `{projection}`: type is not yet fully known")]
    UnresolvedTypeVariable { projection: Text, span: Span },

    /// Protocol not found
    #[error("Protocol `{protocol}` not found")]
    ProtocolNotFound { protocol: Text, span: Span },

    /// Nested projection failed
    #[error("Failed to resolve nested projection `{outer}` -> `{inner}`: {reason}")]
    NestedProjectionFailed {
        outer: Text,
        inner: Text,
        reason: Text,
        span: Span,
    },

    /// Associated type bound not satisfied
    #[error("Associated type `{assoc_name}` does not satisfy bound `{bound}` in projection `{projection}`")]
    AssociatedTypeBoundNotSatisfied {
        projection: Text,
        assoc_name: Text,
        bound: Text,
        span: Span,
    },
}

impl From<ProjectionError> for TypeError {
    fn from(err: ProjectionError) -> Self {
        match err {
            ProjectionError::TypeDoesNotImplementProtocol { ty, protocol, span } => {
                TypeError::ProtocolNotSatisfied {
                    ty,
                    protocol,
                    span,
                }
            }
            ProjectionError::AssociatedTypeNotFound {
                protocol,
                assoc_name,
                span,
            } => TypeError::Other(
                format!(
                    "Protocol '{}' has no associated type named '{}'",
                    protocol, assoc_name
                )
                .into(),
            ),
            ProjectionError::AssociatedTypeNotSpecified {
                protocol,
                ty,
                assoc_name,
                span,
            } => TypeError::Other(
                format!(
                    "Implementation of '{}' for '{}' does not specify associated type '{}'",
                    protocol, ty, assoc_name
                )
                .into(),
            ),
            ProjectionError::AmbiguousAssociatedType {
                ty,
                assoc_name,
                candidates,
                span,
            } => TypeError::AmbiguousMethod {
                method: assoc_name,
                candidates,
                span,
            },
            ProjectionError::UnresolvedTypeVariable { projection, span } => {
                TypeError::AmbiguousType { span }
            }
            ProjectionError::ProtocolNotFound { protocol, span } => TypeError::TypeNotFound {
                name: protocol,
                span,
            },
            ProjectionError::NestedProjectionFailed {
                outer,
                inner,
                reason,
                span,
            } => TypeError::Other(
                format!(
                    "Failed to resolve nested projection '{}.{}': {}",
                    outer, inner, reason
                )
                .into(),
            ),
            ProjectionError::AssociatedTypeBoundNotSatisfied {
                projection,
                assoc_name,
                bound,
                span,
            } => TypeError::Other(
                format!(
                    "Associated type '{}' in projection '{}' does not satisfy bound '{}'",
                    assoc_name, projection, bound
                )
                .into(),
            ),
        }
    }
}

/// A projection represents an associated type access like `T.Item`
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    /// The base type being projected from (e.g., `T` in `T.Item`)
    pub base: Type,
    /// The protocol that provides the associated type (inferred or explicit)
    pub protocol: Option<Path>,
    /// The name of the associated type (e.g., `Item` in `T.Item`)
    pub assoc_name: Text,
    /// Source span for error messages
    pub span: Span,
}

impl Projection {
    /// Create a new projection
    pub fn new(base: Type, assoc_name: Text, span: Span) -> Self {
        Self {
            base,
            protocol: None,
            assoc_name,
            span,
        }
    }

    /// Create a projection with explicit protocol
    pub fn with_protocol(base: Type, protocol: Path, assoc_name: Text, span: Span) -> Self {
        Self {
            base,
            protocol: Some(protocol),
            assoc_name,
            span,
        }
    }

    /// Format projection for display
    pub fn display(&self) -> Text {
        if let Some(ref proto) = self.protocol {
            format!(
                "<{} as {}>.{}",
                self.base.to_text(),
                proto.as_ident().map(|i| i.as_str()).unwrap_or("?"),
                self.assoc_name
            )
            .into()
        } else {
            format!("{}.{}", self.base.to_text(), self.assoc_name).into()
        }
    }
}

/// Result of projection resolution
#[derive(Debug, Clone)]
pub enum ProjectionResult {
    /// Projection was fully resolved to a concrete type
    Resolved(Type),

    /// Projection is deferred because the base type is not yet known
    Deferred(DeferredProjection),
}

/// A deferred projection that needs to be resolved later
#[derive(Debug, Clone, PartialEq)]
pub struct DeferredProjection {
    /// The original projection
    pub projection: Projection,
    /// Type variable that represents the result
    pub result_var: TypeVar,
    /// Any constraints on the result type
    pub result_bounds: List<crate::protocol::ProtocolBound>,
}

/// Projection resolver for associated type projections
///
/// This struct handles the resolution of associated type projections like `T.Item`,
/// working with the ProtocolChecker to look up implementations and resolve types.
pub struct ProjectionResolver<'a> {
    /// Protocol checker for looking up implementations
    protocol_checker: &'a ProtocolChecker,
    /// Current type substitution
    substitution: Substitution,
    /// Span for error messages
    span: Span,
}

impl<'a> ProjectionResolver<'a> {
    /// Create a new projection resolver
    pub fn new(protocol_checker: &'a ProtocolChecker, span: Span) -> Self {
        Self {
            protocol_checker,
            substitution: Substitution::new(),
            span,
        }
    }

    /// Create a projection resolver with a substitution
    pub fn with_substitution(
        protocol_checker: &'a ProtocolChecker,
        substitution: Substitution,
        span: Span,
    ) -> Self {
        Self {
            protocol_checker,
            substitution,
            span,
        }
    }

    /// Resolve a projection to a concrete type
    ///
    /// This is the main entry point for resolving projections like `T.Item`.
    ///
    /// # Algorithm
    ///
    /// 1. Apply current substitution to the base type
    /// 2. If base is a type variable, return deferred projection
    /// 3. Find implementation that provides the associated type
    /// 4. Look up the associated type in the implementation
    /// 5. Apply substitution from implementation matching
    ///
    /// # Returns
    ///
    /// * `Ok(ProjectionResult::Resolved(ty))` - Successfully resolved to concrete type
    /// * `Ok(ProjectionResult::Deferred(proj))` - Base type not yet resolved
    /// * `Err(ProjectionError)` - Resolution failed
    pub fn resolve_projection(&self, projection: &Projection) -> Result<ProjectionResult, ProjectionError> {
        // Apply current substitution to base type
        let base = projection.base.apply_subst(&self.substitution);

        // Handle type variables - defer resolution
        if let Type::Var(var) = &base {
            // Check if the variable has a known bound that might help
            // For now, we defer until the variable is resolved
            let result_var = TypeVar::fresh();
            return Ok(ProjectionResult::Deferred(DeferredProjection {
                projection: Projection {
                    base: base.clone(),
                    protocol: projection.protocol.clone(),
                    assoc_name: projection.assoc_name.clone(),
                    span: projection.span,
                },
                result_var,
                result_bounds: List::new(),
            }));
        }

        // Try to resolve the projection
        if let Some(ref protocol) = projection.protocol {
            // Protocol is explicit - resolve directly
            self.resolve_with_protocol(&base, protocol, &projection.assoc_name)
        } else {
            // Protocol is implicit - search for it
            self.resolve_without_protocol(&base, &projection.assoc_name)
        }
    }

    /// Resolve a projection when the protocol is explicitly specified
    fn resolve_with_protocol(
        &self,
        base: &Type,
        protocol: &Path,
        assoc_name: &Text,
    ) -> Result<ProjectionResult, ProjectionError> {
        // Look up the associated type through the protocol checker
        match self
            .protocol_checker
            .infer_associated_type(base, protocol, assoc_name)
        {
            Ok(resolved_ty) => Ok(ProjectionResult::Resolved(resolved_ty)),
            Err(ProtocolError::NotImplemented { ty, protocol }) => {
                Err(ProjectionError::TypeDoesNotImplementProtocol {
                    ty: ty.to_text(),
                    protocol: protocol
                        .as_ident()
                        .map(|i| -> Text { i.name.clone() })
                        .unwrap_or_else(|| "?".into()),
                    span: self.span,
                })
            }
            Err(ProtocolError::AssociatedTypeNotSpecified {
                protocol,
                assoc_name,
                for_type,
            }) => Err(ProjectionError::AssociatedTypeNotSpecified {
                protocol: protocol
                    .as_ident()
                    .map(|i| -> Text { i.name.clone() })
                    .unwrap_or_else(|| "?".into()),
                ty: for_type.to_text(),
                assoc_name,
                span: self.span,
            }),
            Err(ProtocolError::ProtocolNotFound { name }) => {
                Err(ProjectionError::ProtocolNotFound {
                    protocol: name,
                    span: self.span,
                })
            }
            Err(_e) => Err(ProjectionError::TypeDoesNotImplementProtocol {
                ty: base.to_text(),
                protocol: protocol
                    .as_ident()
                    .map(|i| -> Text { i.name.clone() })
                    .unwrap_or_else(|| "?".into()),
                span: self.span,
            }),
        }
    }

    /// Resolve a projection when the protocol needs to be inferred
    ///
    /// This searches for protocols that the base type implements and that
    /// have the requested associated type.
    fn resolve_without_protocol(
        &self,
        base: &Type,
        assoc_name: &Text,
    ) -> Result<ProjectionResult, ProjectionError> {
        // Get all implementations for this type
        let impls = self.protocol_checker.get_implementations(base);

        // Filter to those that have the requested associated type
        let mut candidates: List<(&ProtocolImpl, Type)> = List::new();

        for impl_ in impls {
            // Check if this implementation has the associated type
            if let Some(assoc_ty) = impl_.associated_types.get(assoc_name) {
                candidates.push((impl_, assoc_ty.clone()));
            } else {
                // Check protocol definition for default
                let protocol_name: Text = impl_
                    .protocol
                    .as_ident()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "?".into());
                if let Option::Some(protocol_def) = self.protocol_checker.get_protocol(&protocol_name) {
                    if let Some(assoc_type_def) = protocol_def.associated_types.get(assoc_name) {
                        if let Option::Some(ref default_ty) = assoc_type_def.default {
                            candidates.push((impl_, default_ty.clone()));
                        }
                    }
                }
            }
        }

        match candidates.len() {
            0 => {
                // No implementation provides this associated type
                // Try to find which protocol might have it for a better error message
                Err(ProjectionError::AssociatedTypeNotFound {
                    protocol: "<inferred>".into(),
                    assoc_name: assoc_name.clone(),
                    span: self.span,
                })
            }
            1 => {
                let (_, assoc_ty) = &candidates[0];
                Ok(ProjectionResult::Resolved(assoc_ty.clone()))
            }
            _ => {
                // Multiple candidates - this is ambiguous
                let candidate_names: List<Text> = candidates
                    .iter()
                    .map(|(impl_, _)| {
                        impl_
                            .protocol
                            .as_ident()
                            .map(|i| -> Text { i.name.clone() })
                            .unwrap_or_else(|| "?".into())
                    })
                    .collect();

                Err(ProjectionError::AmbiguousAssociatedType {
                    ty: base.to_text(),
                    assoc_name: assoc_name.clone(),
                    candidates: candidate_names,
                    span: self.span,
                })
            }
        }
    }

    /// Resolve a nested projection like `T.Item.SubItem`
    ///
    /// This resolves each projection step by step, applying the result
    /// of each step as the base for the next.
    pub fn resolve_nested_projection(
        &self,
        base: &Type,
        path: &[Text],
    ) -> Result<ProjectionResult, ProjectionError> {
        if path.is_empty() {
            return Ok(ProjectionResult::Resolved(base.clone()));
        }

        let mut current = base.clone();

        for (idx, assoc_name) in path.iter().enumerate() {
            let projection = Projection::new(current.clone(), assoc_name.clone(), self.span);

            match self.resolve_projection(&projection)? {
                ProjectionResult::Resolved(ty) => {
                    current = ty;
                }
                ProjectionResult::Deferred(deferred) => {
                    // Create a nested deferred projection for the remaining path
                    if idx + 1 < path.len() {
                        return Err(ProjectionError::NestedProjectionFailed {
                            outer: current.to_text(),
                            inner: path[idx + 1..].join(".").into(),
                            reason: "base type not yet resolved".into(),
                            span: self.span,
                        });
                    }
                    return Ok(ProjectionResult::Deferred(deferred));
                }
            }
        }

        Ok(ProjectionResult::Resolved(current))
    }

    /// Normalize a type by resolving all projections within it
    ///
    /// This walks through a type and resolves any projection types
    /// (represented as `Type::Generic { name: "T::Item", ... }`) to their
    /// concrete types.
    pub fn normalize(&self, ty: &Type) -> Result<Type, ProjectionError> {
        match ty {
            // Check for projection syntax: "T.Item" or "T::Item"
            Type::Generic { name, args } if name.contains(".") || name.contains("::") => {
                // Parse the projection path
                let parts: Vec<&str> = if name.contains(".") {
                    name.as_str().split(".").collect::<Vec<_>>()
                } else {
                    name.as_str().split("::").collect::<Vec<_>>()
                };

                if parts.len() >= 2 {
                    // First part is the base type name
                    let base_name = parts[0];
                    let base = Type::Generic {
                        name: base_name.into(),
                        args: args.clone(),
                    };

                    // Rest are projection steps
                    let path: List<Text> = parts[1..].iter().map(|s| Text::from(*s)).collect();

                    match self.resolve_nested_projection(&base, &path)? {
                        ProjectionResult::Resolved(resolved) => Ok(resolved),
                        ProjectionResult::Deferred(_) => {
                            // Return original type if deferred
                            Ok(ty.clone())
                        }
                    }
                } else {
                    Ok(ty.clone())
                }
            }

            // Recursively normalize compound types
            Type::Function {
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                let normalized_params = params
                    .iter()
                    .map(|p| self.normalize(p))
                    .collect::<Result<List<_>, _>>()?;
                let normalized_return = self.normalize(return_type)?;

                Ok(Type::Function {
                    params: normalized_params,
                    return_type: Box::new(normalized_return),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                })
            }

            Type::Generic { name, args } => {
                let normalized_args = args
                    .iter()
                    .map(|a| self.normalize(a))
                    .collect::<Result<List<_>, _>>()?;

                Ok(Type::Generic {
                    name: name.clone(),
                    args: normalized_args,
                })
            }

            Type::Named { path, args } => {
                let normalized_args = args
                    .iter()
                    .map(|a| self.normalize(a))
                    .collect::<Result<List<_>, _>>()?;

                Ok(Type::Named {
                    path: path.clone(),
                    args: normalized_args,
                })
            }

            Type::Tuple(types) => {
                let normalized = types
                    .iter()
                    .map(|t| self.normalize(t))
                    .collect::<Result<List<_>, _>>()?;

                Ok(Type::Tuple(normalized))
            }

            Type::Array { element, size } => {
                let normalized_elem = self.normalize(element)?;
                Ok(Type::Array {
                    element: Box::new(normalized_elem),
                    size: *size,
                })
            }

            Type::Reference { mutable, inner } => {
                let normalized = self.normalize(inner)?;
                Ok(Type::Reference {
                    mutable: *mutable,
                    inner: Box::new(normalized),
                })
            }

            Type::CheckedReference { mutable, inner } => {
                let normalized = self.normalize(inner)?;
                Ok(Type::CheckedReference {
                    mutable: *mutable,
                    inner: Box::new(normalized),
                })
            }

            Type::UnsafeReference { mutable, inner } => {
                let normalized = self.normalize(inner)?;
                Ok(Type::UnsafeReference {
                    mutable: *mutable,
                    inner: Box::new(normalized),
                })
            }

            Type::Refined { base, predicate } => {
                let normalized_base = self.normalize(base)?;
                Ok(Type::Refined {
                    base: Box::new(normalized_base),
                    predicate: predicate.clone(),
                })
            }

            Type::Future { output } => {
                let normalized = self.normalize(output)?;
                Ok(Type::Future {
                    output: Box::new(normalized),
                })
            }

            Type::GenRef { inner } => {
                let normalized = self.normalize(inner)?;
                Ok(Type::GenRef {
                    inner: Box::new(normalized),
                })
            }

            Type::TypeApp { constructor, args } => {
                let normalized_constructor = self.normalize(constructor)?;
                let normalized_args = args
                    .iter()
                    .map(|a| self.normalize(a))
                    .collect::<Result<List<_>, _>>()?;

                Ok(Type::TypeApp {
                    constructor: Box::new(normalized_constructor),
                    args: normalized_args,
                })
            }

            // Base types that don't need normalization
            _ => Ok(ty.clone()),
        }
    }
}

/// Check if a projection's result satisfies a bound
///
/// This is used to verify constraints like `I.Item: Display`.
///
/// # Arguments
///
/// * `projection` - The projection to check
/// * `bound` - The protocol bound to verify
/// * `protocol_checker` - Protocol checker for implementation lookup
/// * `span` - Source span for error messages
///
/// # Returns
///
/// * `Ok(())` - The bound is satisfied
/// * `Err(ProjectionError)` - The bound is not satisfied
pub fn check_associated_type_bound(
    projection: &Projection,
    bound: &crate::protocol::ProtocolBound,
    protocol_checker: &ProtocolChecker,
    span: Span,
) -> Result<(), ProjectionError> {
    let resolver = ProjectionResolver::new(protocol_checker, span);

    // First resolve the projection
    match resolver.resolve_projection(projection)? {
        ProjectionResult::Resolved(resolved_ty) => {
            // Check if the resolved type implements the bound
            if protocol_checker.implements(&resolved_ty, &bound.protocol) {
                Ok(())
            } else {
                Err(ProjectionError::AssociatedTypeBoundNotSatisfied {
                    projection: projection.display(),
                    assoc_name: projection.assoc_name.clone(),
                    bound: bound
                        .protocol
                        .as_ident()
                        .map(|i| -> Text { i.name.clone() })
                        .unwrap_or_else(|| "?".into()),
                    span,
                })
            }
        }
        ProjectionResult::Deferred(_) => {
            // Cannot check bound on deferred projection
            // This will be checked later when the type is resolved
            Ok(())
        }
    }
}

/// Try to parse a type as a projection
///
/// This converts types like `Type::Generic { name: "T.Item", ... }`
/// into a `Projection` structure for resolution.
pub fn parse_projection(ty: &Type, span: Span) -> Option<Projection> {
    match ty {
        Type::Generic { name, args } => {
            // Check for "T.Item" or "T::Item" syntax
            if name.contains(".") {
                let parts: Vec<&str> = name.as_str().split(".").collect();
                if parts.len() >= 2 {
                    let base = Type::Generic {
                        name: parts[0].into(),
                        args: args.clone(),
                    };
                    // INVARIANT: parts.len() >= 2 checked above, so last() always succeeds
                    let assoc_name = parts.last().expect("parts verified non-empty").to_string().into();
                    return Some(Projection::new(base, assoc_name, span));
                }
            } else if name.contains("::") {
                let parts: Vec<&str> = name.as_str().split("::").collect();
                if parts.len() >= 2 {
                    let base = Type::Generic {
                        name: parts[0].into(),
                        args: args.clone(),
                    };
                    // INVARIANT: parts.len() >= 2 checked above, so last() always succeeds
                    let assoc_name = parts.last().expect("parts verified non-empty").to_string().into();
                    return Some(Projection::new(base, assoc_name, span));
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AssociatedType, Protocol, ProtocolImpl, ProtocolMethod};
    use verum_ast::ty::{Ident, Path};

    fn create_test_protocol_checker() -> ProtocolChecker {
        let mut checker = ProtocolChecker::new_empty();

        // Register Iterator protocol with Item associated type
        let iterator_proto = Protocol {
            name: "Iterator".into(),
            kind: crate::protocol::ProtocolKind::Constraint,
            type_params: List::new(),
            defining_crate: Option::None,
            methods: {
                let mut methods = Map::new();
                methods.insert(
                    "next".into(),
                    ProtocolMethod::simple(
                        "next".into(),
                        Type::function(List::new(), Type::Unit),
                        false,
                    ),
                );
                methods
            },
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert(
                    "Item".into(),
                    AssociatedType::simple("Item".into(), List::new()),
                );
                assoc
            },
            associated_consts: Map::new(),
            super_protocols: List::new(),
            specialization_info: Option::None,
            span: Span::dummy(),
        };

        let _ = checker.register_protocol(iterator_proto);

        // Register List<T> as implementing Iterator with Item = T
        let list_impl = ProtocolImpl {
            protocol: Path::single(Ident::new("Iterator", Span::dummy())),
            protocol_args: List::new(),
            for_type: Type::Generic {
                name: "List".into(),
                args: List::from(vec![Type::Var(TypeVar::with_id(0))]),
            },
            where_clauses: List::new(),
            methods: Map::new(),
            associated_types: {
                let mut assoc = Map::new();
                assoc.insert("Item".into(), Type::Var(TypeVar::with_id(0)));
                assoc
            },
            associated_consts: Map::new(),
            specialization: Option::None,
            impl_crate: Option::None,
            span: Span::dummy(),
            type_param_fn_bounds: Map::new(),
        };

        let _ = checker.register_impl(list_impl);

        checker
    }

    #[test]
    fn test_resolve_projection_concrete_type() {
        let checker = create_test_protocol_checker();
        let resolver = ProjectionResolver::new(&checker, Span::dummy());

        // Create projection: List<Int>.Item
        let list_int = Type::Generic {
            name: "List".into(),
            args: List::from(vec![Type::Int]),
        };
        let projection = Projection::with_protocol(
            list_int,
            Path::single(Ident::new("Iterator", Span::dummy())),
            "Item".into(),
            Span::dummy(),
        );

        let result = resolver.resolve_projection(&projection);
        assert!(result.is_ok());

        match result.unwrap() {
            ProjectionResult::Resolved(ty) => {
                assert_eq!(ty, Type::Int);
            }
            ProjectionResult::Deferred(_) => {
                panic!("Expected resolved type, got deferred");
            }
        }
    }

    #[test]
    fn test_resolve_projection_type_variable() {
        let checker = create_test_protocol_checker();
        let resolver = ProjectionResolver::new(&checker, Span::dummy());

        // Create projection: T.Item where T is a type variable
        let t_var = Type::Var(TypeVar::with_id(99));
        let projection = Projection::new(t_var, "Item".into(), Span::dummy());

        let result = resolver.resolve_projection(&projection);
        assert!(result.is_ok());

        match result.unwrap() {
            ProjectionResult::Deferred(deferred) => {
                assert_eq!(deferred.projection.assoc_name, Text::from("Item"));
            }
            ProjectionResult::Resolved(_) => {
                panic!("Expected deferred projection, got resolved");
            }
        }
    }

    #[test]
    fn test_parse_projection() {
        // Test "T.Item" syntax
        let ty = Type::Generic {
            name: "T.Item".into(),
            args: List::new(),
        };
        let proj = parse_projection(&ty, Span::dummy());
        assert!(proj.is_some());
        let proj = proj.unwrap();
        assert_eq!(proj.assoc_name, Text::from("Item"));

        // Test "T::Item" syntax
        let ty2 = Type::Generic {
            name: "T::Item".into(),
            args: List::new(),
        };
        let proj2 = parse_projection(&ty2, Span::dummy());
        assert!(proj2.is_some());
        let proj2 = proj2.unwrap();
        assert_eq!(proj2.assoc_name, Text::from("Item"));
    }
}
