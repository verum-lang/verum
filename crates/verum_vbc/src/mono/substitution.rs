//! Type substitution for monomorphization.

use std::collections::HashMap;

use crate::module::FunctionDescriptor;
use crate::types::{TypeParamDescriptor, TypeParamId, TypeRef};

/// Type substitution environment for monomorphization.
///
/// Maps type parameters to concrete types.
pub struct TypeSubstitution {
    /// Type parameter bindings: TypeParamId -> concrete TypeRef.
    bindings: HashMap<TypeParamId, TypeRef>,
}

impl TypeSubstitution {
    /// Creates a new substitution from type parameters and arguments.
    pub fn new(params: &[TypeParamDescriptor], args: &[TypeRef]) -> Self {
        let mut bindings = HashMap::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            bindings.insert(param.id, arg.clone());
        }
        Self { bindings }
    }

    /// Creates a substitution from a function descriptor and type arguments.
    pub fn from_function(func: &FunctionDescriptor, args: &[TypeRef]) -> Self {
        Self::new(&func.type_params, args)
    }

    /// Creates an empty substitution.
    pub fn empty() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Adds a binding to the substitution.
    pub fn bind(&mut self, param: TypeParamId, ty: TypeRef) {
        self.bindings.insert(param, ty);
    }

    /// Gets the binding for a type parameter.
    pub fn get(&self, param: TypeParamId) -> Option<&TypeRef> {
        self.bindings.get(&param)
    }

    /// Applies the substitution to a type reference.
    ///
    /// Recursively substitutes type parameters with their bindings.
    pub fn apply(&self, type_ref: &TypeRef) -> TypeRef {
        match type_ref {
            TypeRef::Generic(param_id) => {
                self.bindings
                    .get(param_id)
                    .cloned()
                    .unwrap_or_else(|| type_ref.clone())
            }
            TypeRef::Concrete(_) => type_ref.clone(),
            TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
                base: *base,
                args: args.iter().map(|a| self.apply(a)).collect(),
            },
            TypeRef::Function {
                params,
                return_type,
                contexts,
            } => TypeRef::Function {
                params: params.iter().map(|p| self.apply(p)).collect(),
                return_type: Box::new(self.apply(return_type)),
                contexts: contexts.clone(),
            },
            TypeRef::Rank2Function {
                type_param_count,
                params,
                return_type,
                contexts,
            } => TypeRef::Rank2Function {
                type_param_count: *type_param_count,
                params: params.iter().map(|p| self.apply(p)).collect(),
                return_type: Box::new(self.apply(return_type)),
                contexts: contexts.clone(),
            },
            TypeRef::Reference {
                inner,
                mutability,
                tier,
            } => TypeRef::Reference {
                inner: Box::new(self.apply(inner)),
                mutability: *mutability,
                tier: *tier,
            },
            TypeRef::Tuple(elements) => {
                TypeRef::Tuple(elements.iter().map(|e| self.apply(e)).collect())
            }
            TypeRef::Array { element, length } => TypeRef::Array {
                element: Box::new(self.apply(element)),
                length: *length,
            },
            TypeRef::Slice(element) => TypeRef::Slice(Box::new(self.apply(element))),
        }
    }

    /// Returns the number of bindings.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns true if there are no bindings.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TypeId, Variance};

    #[test]
    fn test_type_substitution() {
        let params = vec![TypeParamDescriptor {
            id: TypeParamId(0),
            name: crate::types::StringId(0),
            bounds: Default::default(),
            variance: Variance::Invariant,
            default: None,
        }];
        let args = vec![TypeRef::Concrete(TypeId::INT)];

        let subst = TypeSubstitution::new(&params, &args);

        let generic = TypeRef::Generic(TypeParamId(0));
        let substituted = subst.apply(&generic);

        assert_eq!(substituted, TypeRef::Concrete(TypeId::INT));
    }

    #[test]
    fn test_type_substitution_nested() {
        let params = vec![TypeParamDescriptor {
            id: TypeParamId(0),
            name: crate::types::StringId(0),
            bounds: Default::default(),
            variance: Variance::Invariant,
            default: None,
        }];
        let args = vec![TypeRef::Concrete(TypeId::INT)];

        let subst = TypeSubstitution::new(&params, &args);

        // List<T> where T = Int
        let generic = TypeRef::Instantiated {
            base: TypeId(20), // Assume List is type 20
            args: vec![TypeRef::Generic(TypeParamId(0))],
        };
        let substituted = subst.apply(&generic);

        assert_eq!(
            substituted,
            TypeRef::Instantiated {
                base: TypeId(20),
                args: vec![TypeRef::Concrete(TypeId::INT)],
            }
        );
    }

    #[test]
    fn test_empty_substitution() {
        let subst = TypeSubstitution::empty();
        assert!(subst.is_empty());

        let generic = TypeRef::Generic(TypeParamId(0));
        let result = subst.apply(&generic);
        // Unbound generics remain unchanged
        assert_eq!(result, generic);
    }
}
