//! Resolve generic type-method calls from carried receiver and argument facts.
//! No name search: the module's protocol dispatch map owns callee identity.

use super::{TypeSubstitution, discovery};
use crate::instruction::{Instruction as I, RegRange};
use crate::module::{FunctionDescriptor, VbcModule};
use crate::types::{StringId, TypeId, TypeRef};
use std::collections::HashMap;

fn unify(pattern: &TypeRef, actual: &TypeRef, bindings: &mut TypeSubstitution) -> bool {
    use TypeRef as T;
    match (pattern, actual) {
        (T::Generic(id), _) => match bindings.get(*id) {
            Some(bound) => bound == actual,
            None if discovery::concrete(actual) => {
                bindings.bind(*id, actual.clone());
                true
            }
            _ => false,
        },
        (T::Instantiated { base: p, args: pa }, T::Instantiated { base: a, args: aa }) => {
            p == a && pa.len() == aa.len() && pa.iter().zip(aa).all(|(p, a)| unify(p, a, bindings))
        }
        (
            T::Reference {
                inner: p,
                mutability: pm,
                tier: pt,
            },
            T::Reference {
                inner: a,
                mutability: am,
                tier: at,
            },
        ) => pm == am && pt == at && unify(p, a, bindings),
        (T::Tuple(p), T::Tuple(a)) => {
            p.len() == a.len() && p.iter().zip(a).all(|(p, a)| unify(p, a, bindings))
        }
        (
            T::Function {
                params: p,
                return_type: pr,
                contexts: pc,
            },
            T::Function {
                params: a,
                return_type: ar,
                contexts: ac,
            },
        ) => {
            pc == ac
                && p.len() == a.len()
                && p.iter().zip(a).all(|(p, a)| unify(p, a, bindings))
                && unify(pr, ar, bindings)
        }
        (
            T::Array {
                element: p,
                length: pl,
            },
            T::Array {
                element: a,
                length: al,
            },
        ) => pl == al && unify(p, a, bindings),
        (T::Slice(p), T::Slice(a)) => unify(p, a, bindings),
        _ => pattern == actual && discovery::concrete(actual),
    }
}

// Unit/PTR are also the old producer's unknown-signature sentinels. They
// cannot establish a receiver/argument fact for devirtualization by themselves.
fn usable(ty: &TypeRef) -> bool {
    discovery::concrete(ty) && !matches!(ty, TypeRef::Concrete(TypeId::UNIT | TypeId::PTR))
}

// GetF reads through references, but its field layout and generic parameters
// belong to the exact nominal owner. Caller/method parameter IDs are a different
// scope and cannot fill a missing owner argument.
fn field_type(module: &VbcModule, mut receiver: &TypeRef, index: u32) -> Option<TypeRef> {
    while let TypeRef::Reference { inner, .. } = receiver {
        receiver = inner;
    }
    let owner = module.get_type(receiver.base_type_id()?)?;
    let field = owner.fields.get(index as usize)?;
    let bindings = match receiver {
        TypeRef::Instantiated { args, .. } if args.len() == owner.type_params.len() => {
            TypeSubstitution::new(&owner.type_params, args)
        }
        TypeRef::Concrete(_) if owner.type_params.is_empty() => TypeSubstitution::empty(),
        _ => return None,
    };
    let result = bindings.apply(&field.type_ref);
    usable(&result).then_some(result)
}

fn static_call(
    module: &VbcModule,
    receiver: &TypeRef,
    method: u32,
    args: RegRange,
    values: &HashMap<u16, TypeRef>,
    witness: Option<&[TypeRef]>,
) -> Option<(u32, Vec<TypeRef>, TypeRef)> {
    let base = receiver.base_type_id()?;
    let name = module.get_string(StringId(method))?;
    let bare = name
        .strip_prefix("dyn:")
        .map(|n| n.rsplit('.').next().unwrap_or(n))
        .unwrap_or(name);
    // Qualified CallM tokens have their own authority; do not reinterpret
    // a foreign owner as a method on this receiver.
    if bare.contains('.') {
        return None;
    }
    let id = module.resolve_protocol_method_by_name(base.0, bare)?;
    let target = module.get_function(id)?;
    if target.params.len() != args.count as usize {
        return None;
    }
    let owner = module.get_type(base)?;
    let mut bindings = match receiver {
        TypeRef::Instantiated { args, .. } if args.len() == owner.type_params.len() => {
            TypeSubstitution::new(&owner.type_params, args)
        }
        TypeRef::Concrete(_) if owner.type_params.is_empty() => TypeSubstitution::empty(),
        _ => return None,
    };
    if let Some(witness) = witness {
        let carried = TypeSubstitution::from_function(target, witness);
        for parameter in &target.type_params {
            if let Some(actual) = carried.get(parameter.id) {
                if !unify(&TypeRef::Generic(parameter.id), actual, &mut bindings) {
                    return None;
                }
            }
        }
    }
    for (index, param) in target.params.iter().enumerate() {
        let actual = values.get(&args.start.0.checked_add(index as u16)?)?;
        if !usable(actual) || !unify(&param.type_ref, actual, &mut bindings) {
            return None;
        }
    }
    let type_args = target
        .type_params
        .iter()
        .map(|p| bindings.get(p.id).cloned())
        .collect::<Option<Vec<_>>>()?;
    if !type_args.iter().all(discovery::concrete) {
        return None;
    }
    Some((id.0, type_args, bindings.apply(&target.return_type)))
}

/// Straight-line facts are sufficient for `C.construct(self)` in a generic
/// protocol default. Unknown operations/control flow invalidate facts rather
/// than making a guessed callee permanent. This pass runs after substitution.
pub(super) fn resolve_type_methods(
    module: &VbcModule,
    function: &FunctionDescriptor,
    substitution: &TypeSubstitution,
    instructions: &mut [I],
) {
    // Each branch target starts a fresh straight-line proof. In particular,
    // facts from the physically preceding arm are not facts at a CFG join.
    let mut leaders = std::collections::HashSet::new();
    for (index, instruction) in instructions.iter().enumerate() {
        let offset = match instruction {
            I::Jmp { offset }
            | I::JmpIf { offset, .. }
            | I::JmpNot { offset, .. }
            | I::JmpCmp { offset, .. } => Some(*offset),
            I::CtxProvide { body_offset, .. } => Some(*body_offset),
            I::TryBegin { handler_offset } => Some(*handler_offset),
            // Switch tables are not currently relocated by the canonical
            // fixup API. Do not introduce width-changing rewrites here.
            I::Switch { .. } => return,
            _ => None,
        };
        if let Some(offset) = offset {
            leaders.insert(index as i64 + offset as i64);
        }
    }
    let mut values: HashMap<u16, TypeRef> = function
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| (i as u16, substitution.apply(&p.type_ref)))
        .filter(|(_, ty)| usable(ty))
        .collect();
    let mut tokens = HashMap::new();
    let mut pending: Option<Vec<TypeRef>> = None;
    let mut pending_index = None;
    let mut consumed_sidecars = Vec::new();
    for (index, instruction) in instructions.iter_mut().enumerate() {
        if leaders.contains(&(index as i64)) {
            values.clear();
            tokens.clear();
            pending = None;
            pending_index = None;
        }
        match instruction {
            I::Nop => {}
            I::LoadI { dst, .. } | I::TypeLayout { dst, .. } => {
                values.insert(dst.0, TypeRef::Concrete(TypeId::INT));
                tokens.remove(&dst.0);
            }
            I::LoadT { dst, type_ref } => {
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if discovery::concrete(type_ref) {
                    tokens.insert(dst.0, type_ref.clone());
                }
            }
            I::Mov { dst, src } => {
                let value = values.get(&src.0).cloned();
                let token = tokens.get(&src.0).cloned();
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if let Some(value) = value {
                    values.insert(dst.0, value);
                }
                if let Some(token) = token {
                    tokens.insert(dst.0, token);
                }
            }
            I::GetF {
                dst,
                obj,
                field_idx,
            } => {
                let value = values
                    .get(&obj.0)
                    .and_then(|receiver| field_type(module, receiver, *field_idx));
                // A field read changes only its destination. Keep independent
                // LoadT facts used by C.make(self.field), but never keep stale
                // destination facts when the field's type cannot be proved.
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if let Some(value) = value {
                    values.insert(dst.0, value);
                }
            }
            I::SetCallWitness { type_args } => {
                pending = Some(type_args.clone());
                pending_index = Some(index);
            }
            I::CallM {
                dst,
                receiver,
                method_id,
                args,
            } => {
                let witness = pending.take();
                let sidecar = pending_index.take();
                let target = tokens.get(&receiver.0).and_then(|receiver| {
                    static_call(
                        module,
                        receiver,
                        *method_id,
                        *args,
                        &values,
                        witness.as_deref(),
                    )
                });
                // Instance calls keep their existing receiver ABI. Qualify
                // only a receiver with a carried concrete type and an exact
                // dispatch-map entry; never substitute an arbitrary slot 0.
                if target.is_none() && !tokens.contains_key(&receiver.0) {
                    let instance = values
                        .get(&receiver.0)
                        .map(|ty| match ty {
                            TypeRef::Reference { inner, .. } => inner.as_ref(),
                            ty => ty,
                        })
                        .and_then(TypeRef::base_type_id);
                    if let Some(base) = instance
                        && let Some(name) = module.get_string(StringId(*method_id))
                        && !name.contains('.')
                        && let Some(id) = module.resolve_protocol_method_by_name(base.0, name)
                        && let Some(callee) = module.get_function(id)
                        && callee.params.len() == args.count as usize + 1
                    {
                        *method_id = callee.name.0;
                    }
                }
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if let Some((func_id, type_args, result)) = target {
                    if let Some(index) = sidecar {
                        consumed_sidecars.push(index);
                    }
                    if usable(&result) {
                        values.insert(dst.0, result);
                    }
                    *instruction = if type_args.is_empty() {
                        I::Call {
                            dst: *dst,
                            func_id,
                            args: *args,
                        }
                    } else {
                        I::CallG {
                            dst: *dst,
                            func_id,
                            type_args,
                            args: *args,
                        }
                    };
                }
            }
            I::CallG {
                dst,
                func_id,
                type_args,
                ..
            } => {
                pending = None;
                pending_index = None;
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if let Some(target) = module.get_function(module.resolved_function_id(*func_id)) {
                    let result = TypeSubstitution::from_function(target, type_args)
                        .apply(&target.return_type);
                    if usable(&result) {
                        values.insert(dst.0, result);
                    }
                }
            }
            I::Call { dst, func_id, .. } => {
                let witness = pending.take();
                pending_index = None;
                values.remove(&dst.0);
                tokens.remove(&dst.0);
                if let Some(target) = module.get_function(module.resolved_function_id(*func_id)) {
                    let result =
                        TypeSubstitution::from_function(target, witness.as_deref().unwrap_or(&[]))
                            .apply(&target.return_type);
                    if usable(&result) {
                        values.insert(dst.0, result);
                    }
                }
            }
            I::Ret { .. } => {
                values.clear();
                tokens.clear();
                pending = None;
                pending_index = None;
            }
            _ => {
                values.clear();
                tokens.clear();
                pending = None;
                pending_index = None;
            }
        }
    }
    for index in consumed_sidecars {
        instructions[index] = I::Nop;
    }
}
