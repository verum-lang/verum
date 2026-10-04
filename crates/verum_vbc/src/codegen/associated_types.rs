//! Resolve carried associated bindings in the consumer's nominal pool.

use super::VbcCodegen;
use crate::{mono::TypeSubstitution, types::TypeRef};
use verum_common::{List, Set};

impl VbcCodegen {
    /// Instantiate declaration-owned associated bindings. A projection does
    /// not carry a protocol ID, so incompatible same-named bindings remain
    /// unknown; neither protocol iteration order nor a method spelling wins.
    pub(super) fn resolve_declared_associated_types(&self, ty: &TypeRef) -> Option<TypeRef> {
        fn resolve(
            codegen: &VbcCodegen,
            ty: &TypeRef,
            active: &mut Set<TypeRef>,
            depth: usize,
        ) -> Option<TypeRef> {
            // The depth bound also stops expanding cycles such as
            // Grow<T>.Argument -> Grow<Grow<T>>.Argument.
            if depth >= 64 {
                return None;
            }
            let nested = |ty, active: &mut Set<TypeRef>| resolve(codegen, ty, active, depth + 1);
            match ty {
                TypeRef::AssociatedProjection { base, assoc } => {
                    if !active.insert(ty.clone()) {
                        return None;
                    }
                    let result = (|| {
                        let base = nested(base, active)?;
                        let (id, args) = match &base {
                            TypeRef::Concrete(id) => (*id, &[][..]),
                            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                            _ => return None,
                        };
                        let descriptor = codegen.type_by_id(id)?;
                        if descriptor.type_params.len() != args.len() {
                            return None;
                        }
                        let substitution = TypeSubstitution::new(&descriptor.type_params, args);
                        let mut binding = None;
                        for implementation in &descriptor.protocols {
                            for (name, target) in &implementation.associated_types {
                                if codegen.ctx.strings.get(name.0 as usize).map(String::as_str)
                                    != Some(assoc.as_str())
                                {
                                    continue;
                                }
                                let target = substitution.apply(target);
                                if binding.as_ref().is_some_and(|previous| previous != &target) {
                                    return None;
                                }
                                binding = Some(target);
                            }
                        }
                        resolve(codegen, &binding?, active, depth + 1)
                    })();
                    active.remove(ty);
                    result
                }
                TypeRef::Instantiated { base, args } => Some(TypeRef::Instantiated {
                    base: *base,
                    args: args
                        .iter()
                        .map(|arg| nested(arg, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                }),
                TypeRef::Function {
                    params,
                    return_type,
                    contexts,
                } => Some(TypeRef::Function {
                    params: params
                        .iter()
                        .map(|param| nested(param, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                    return_type: Box::new(nested(return_type, active)?),
                    contexts: contexts.clone(),
                }),
                TypeRef::Reference {
                    inner,
                    mutability,
                    tier,
                } => Some(TypeRef::Reference {
                    inner: Box::new(nested(inner, active)?),
                    mutability: *mutability,
                    tier: *tier,
                }),
                TypeRef::Tuple(items) => Some(TypeRef::Tuple(
                    items
                        .iter()
                        .map(|item| nested(item, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                )),
                TypeRef::Array { element, length } => Some(TypeRef::Array {
                    element: Box::new(nested(element, active)?),
                    length: *length,
                }),
                TypeRef::Slice(inner) => Some(TypeRef::Slice(Box::new(nested(inner, active)?))),
                // Rank-2 signatures are not certified as ordinary callables.
                TypeRef::Rank2Function { .. } => None,
                TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) => {
                    Some(ty.clone())
                }
            }
        }
        resolve(self, ty, &mut Set::new(), 0)
    }
}
