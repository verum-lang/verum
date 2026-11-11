//! Immediate type children, in declaration order. Binder and declaration-owned
//! metadata stays with the node; substitution handles its scope separately.
use super::Type;

impl Type {
    /// Visit the same slots and order as `map_children`, without cloning a node.
    /// This lets the substitution machine rebuild results and reprocess owned
    /// type applications while retaining declaration-owned metadata.
    pub(super) fn for_each_child_mut(&mut self, mut visit: impl FnMut(&mut Type)) {
        match self {
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    visit(param);
                }
                visit(return_type);
            }
            Type::Named { args, .. } | Type::Generic { args, .. } | Type::Tuple(args) => {
                for arg in args {
                    visit(arg);
                }
            }
            Type::Record(fields)
            | Type::Variant(fields)
            | Type::ExtensibleRecord { fields, .. } => {
                for ty in fields.values_mut() {
                    visit(ty);
                }
            }
            Type::DynProtocol { bindings, .. } => {
                for ty in bindings.values_mut() {
                    visit(ty);
                }
            }
            Type::Array { element, .. }
            | Type::Slice { element }
            | Type::Tensor { element, .. } => visit(element),
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Ownership { inner, .. }
            | Type::Pointer { inner, .. }
            | Type::VolatilePointer { inner, .. }
            | Type::GenRef { inner }
            | Type::Quantified { inner, .. } => visit(inner),
            Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => visit(base),
            Type::Exists { body, .. } | Type::Forall { body, .. } => visit(body),
            Type::Meta { ty, .. } | Type::Eq { ty, .. } => visit(ty),
            Type::Future { output } => visit(output),
            Type::Generator {
                yield_ty,
                return_ty,
            } => {
                visit(yield_ty);
                visit(return_ty);
            }
            Type::TypeApp { constructor, args } => {
                visit(constructor);
                for arg in args {
                    visit(arg);
                }
            }
            Type::Pi {
                param_type,
                return_type,
                ..
            } => {
                visit(param_type);
                visit(return_type);
            }
            Type::Sigma {
                fst_type, snd_type, ..
            } => {
                visit(fst_type);
                visit(snd_type);
            }
            Type::PathType { space, .. } => visit(space),
            Type::Partial { element_type, .. } => visit(element_type),
            Type::Inductive {
                params, indices, ..
            } => {
                for (_, ty) in params {
                    visit(ty);
                }
                for (_, ty) in indices {
                    visit(ty);
                }
            }
            Type::Coinductive { params, .. } | Type::HigherInductive { params, .. } => {
                for (_, ty) in params {
                    visit(ty);
                }
            }
            Type::Unit
            | Type::Never
            | Type::Unknown
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Var(_)
            | Type::RecordField { .. }
            | Type::Lifetime { .. }
            | Type::TypeConstructor { .. }
            | Type::Interval
            | Type::Universe { .. }
            | Type::Prop
            | Type::Placeholder { .. } => {}
        }
    }

    pub(super) fn map_children<'a>(&'a self, mut map: impl FnMut(&'a Type) -> Type) -> Type {
        match self {
            Type::Var(v) => Type::Var(*v),
            Type::Function {
                property_requirement,
                params,
                return_type,
                type_params,
                contexts,
                properties,
            } => Type::Function {
                property_requirement: property_requirement.clone(),
                params: params.iter().map(|p| map(p)).collect(),
                return_type: Box::new(map(return_type)),
                type_params: type_params.clone(),
                contexts: contexts.clone(),
                properties: properties.clone(),
            },
            Type::Tuple(types) => Type::Tuple(types.iter().map(|t| map(t)).collect()),
            Type::Array { element, size } => Type::Array {
                element: Box::new(map(element)),
                size: *size,
            },
            Type::Record(fields) => {
                Type::Record(fields.iter().map(|(k, v)| (k.clone(), map(v))).collect())
            }
            Type::Variant(variants) => {
                Type::Variant(variants.iter().map(|(k, v)| (k.clone(), map(v))).collect())
            }
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },
            Type::CheckedReference { mutable, inner } => Type::CheckedReference {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },
            Type::UnsafeReference { mutable, inner } => Type::UnsafeReference {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },
            Type::Ownership { mutable, inner } => Type::Ownership {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },
            Type::Pointer { mutable, inner } => Type::Pointer {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },

            Type::VolatilePointer { mutable, inner } => Type::VolatilePointer {
                mutable: *mutable,
                inner: Box::new(map(inner)),
            },
            Type::Refined { base, predicate } => Type::Refined {
                base: Box::new(map(base)),
                predicate: predicate.clone(),
            },
            Type::Named { path, args } => Type::Named {
                path: path.clone(),
                args: args.iter().map(|a| map(a)).collect(),
            },
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args.iter().map(|a| map(a)).collect(),
            },
            Type::Exists { var, body } => Type::Exists {
                var: *var,
                body: Box::new(map(body)),
            },
            Type::Forall { vars, body } => Type::Forall {
                vars: vars.clone(),
                body: Box::new(map(body)),
            },
            Type::Meta {
                name,
                ty,
                refinement,
                value,
            } => Type::Meta {
                name: name.clone(),
                ty: Box::new(map(ty)),
                refinement: refinement.clone(),
                value: value.clone(),
            },
            Type::Future { output } => Type::Future {
                output: Box::new(map(output)),
            },
            Type::Generator {
                yield_ty,
                return_ty,
            } => Type::Generator {
                yield_ty: Box::new(map(yield_ty)),
                return_ty: Box::new(map(return_ty)),
            },
            Type::Tensor {
                element,
                shape,
                strides,
                span,
            } => Type::Tensor {
                element: Box::new(map(element)),
                shape: shape.clone(), // Shape is compile-time constant
                strides: strides.clone(),
                span: *span,
            },
            Type::GenRef { inner } => Type::GenRef {
                inner: Box::new(map(inner)),
            },
            Type::TypeConstructor { name, arity, kind } => Type::TypeConstructor {
                name: name.clone(),
                arity: *arity,
                kind: kind.clone(),
            },
            Type::TypeApp { constructor, args } => Type::TypeApp {
                constructor: Box::new(map(constructor)),
                args: args.iter().map(&mut map).collect(),
            },
            Type::Pi {
                param_name,
                param_type,
                return_type,
            } => Type::Pi {
                param_name: param_name.clone(),
                param_type: Box::new(map(param_type)),
                return_type: Box::new(map(return_type)),
            },
            Type::Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Type::Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(map(fst_type)),
                snd_type: Box::new(map(snd_type)),
            },
            Type::Eq { ty, lhs, rhs } => Type::Eq {
                ty: Box::new(map(ty)),
                lhs: lhs.clone(), // EqTerms are value-level, not type-level
                rhs: rhs.clone(),
            },
            Type::PathType { space, left, right } => Type::PathType {
                space: Box::new(map(space)),
                left: left.clone(), // CubicalTerms are value-level, not type-level
                right: right.clone(),
            },
            Type::Partial { element_type, face } => Type::Partial {
                element_type: Box::new(map(element_type)),
                face: face.clone(), // CubicalTerms are value-level
            },
            Type::Interval => Type::Interval,
            Type::RecordField {
                slot,
                receiver,
                level,
            } => Type::RecordField {
                slot: slot.clone(),
                receiver: receiver.clone(),
                level: level.clone(),
            },
            Type::Universe { level } => Type::Universe {
                level: level.clone(),
            },
            Type::Prop => Type::Prop,
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
                    .map(|(n, t)| (n.clone(), Box::new(map(t))))
                    .collect(),
                indices: indices
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(map(t))))
                    .collect(),
                universe: universe.clone(),
                constructors: constructors.clone(), // Constructors have their own type context
            },
            Type::Coinductive {
                name,
                params,
                destructors,
            } => Type::Coinductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(map(t))))
                    .collect(),
                destructors: destructors.clone(),
            },
            Type::HigherInductive {
                name,
                params,
                point_constructors,
                path_constructors,
            } => Type::HigherInductive {
                name: name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| (n.clone(), Box::new(map(t))))
                    .collect(),
                point_constructors: point_constructors.clone(),
                path_constructors: path_constructors.clone(),
            },
            Type::Quantified { inner, quantity } => Type::Quantified {
                inner: Box::new(map(inner)),
                quantity: *quantity,
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(map(element)),
            },
            Type::Unit
            | Type::Never
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Char
            | Type::Text
            | Type::Lifetime { .. }
            | Type::Placeholder { .. } => self.clone(),
            Type::DynProtocol { bounds, bindings } => {
                // Map uses a randomized hash table. Schedule children in the
                // destination's order, which is also its reconstruction order.
                let mut mapped: verum_common::Map<_, _> = bindings
                    .keys()
                    .map(|name| (name.clone(), Type::Unit))
                    .collect();
                for (name, value) in mapped.iter_mut() {
                    *value = map(&bindings[name]);
                }
                Type::DynProtocol {
                    bounds: bounds.clone(),
                    bindings: mapped,
                }
            }
            Type::ExtensibleRecord { fields, row_var } => {
                let new_fields = fields.iter().map(|(k, v)| (k.clone(), map(v))).collect();
                Type::ExtensibleRecord {
                    fields: new_fields,
                    row_var: *row_var,
                }
            }
            Type::CapabilityRestricted { base, capabilities } => Type::CapabilityRestricted {
                base: Box::new(map(base)),
                capabilities: capabilities.clone(),
            },
            Type::Unknown => Type::Unknown,
        }
    }
}
