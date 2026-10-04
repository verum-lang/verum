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
    ///
    /// The descriptor maps compact argument positions to declaration-owned
    /// IDs, including shadowed method parameters. Descriptors without generic
    /// metadata retain the legacy positional argument convention.
    pub fn from_function(func: &FunctionDescriptor, args: &[TypeRef]) -> Self {
        // T1526/T1528: compact witnesses follow the declaration's exact ID
        // roster, including shadow parameters. The descriptor owns legacy
        // indexed compatibility as well; consumers never infer slot identity.
        if !func.type_params.is_empty() {
            let mut result = Self::empty();
            for param in &func.type_params {
                if let Some(arg) = func.generic_argument(args, param.id) {
                    result.bind(param.id, arg.clone());
                }
            }
            return result;
        }
        let mut bindings = HashMap::new();
        for (i, arg) in args.iter().enumerate() {
            bindings.insert(crate::types::TypeParamId(i as u16), arg.clone());
        }
        Self { bindings }
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
            TypeRef::Generic(param_id) => self
                .bindings
                .get(param_id)
                .cloned()
                .unwrap_or_else(|| type_ref.clone()),
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
            } => {
                // The locally quantified IDs belong to this function type,
                // not to the surrounding method/function witness frame.
                let scoped = Self {
                    bindings: self
                        .bindings
                        .iter()
                        .filter(|(id, _)| id.0 >= *type_param_count)
                        .map(|(id, ty)| (*id, ty.clone()))
                        .collect(),
                };
                TypeRef::Rank2Function {
                    type_param_count: *type_param_count,
                    params: params.iter().map(|p| scoped.apply(p)).collect(),
                    return_type: Box::new(scoped.apply(return_type)),
                    contexts: contexts.clone(),
                }
            }
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
            TypeRef::AssociatedProjection { base, assoc } => TypeRef::AssociatedProjection {
                base: Box::new(self.apply(base)),
                assoc: assoc.clone(),
            },
            // Const-generic VALUE argument — already fully concrete.
            TypeRef::ConstValue(_) => type_ref.clone(),
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
