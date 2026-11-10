use crate::module::{FunctionId, VbcModule};
use crate::mono::{TypeSubstitution, match_declared_types};
use crate::types::{StringId, TypeId, TypeRef};
use std::collections::HashMap;

pub(crate) type MethodFamilies =
    HashMap<(TypeId, StringId, usize), smallvec::SmallVec<[FunctionId; 1]>>;

pub(crate) fn pointee(mut ty: &TypeRef) -> &TypeRef {
    while let TypeRef::Reference { inner, .. } = ty {
        ty = inner;
    }
    ty
}

/// Receiver-directed selection only applies when the self patterns distinguish
/// the implementations. Equal patterns leave other overload axes to their owner.
pub(crate) fn select<'a, T>(
    name: &str,
    receiver: &TypeRef,
    candidates: impl Iterator<Item = (FunctionId, T, Option<&'a TypeRef>)>,
) -> Result<Option<T>, String> {
    let mut first_pattern = None;
    let mut distinct = false;
    let mut selected = None;
    let mut ambiguous = false;
    for (id, value, pattern) in candidates {
        let pattern = pattern.map(pointee).ok_or_else(|| {
            format!(
                "missing receiver declaration for {name} implementation {}",
                id.0
            )
        })?;
        distinct |= first_pattern.is_some_and(|first| first != pattern);
        first_pattern.get_or_insert(pattern);
        if match_declared_types(pattern, receiver, &mut TypeSubstitution::empty()) {
            ambiguous |= selected
                .as_ref()
                .is_some_and(|(previous, _)| *previous != id);
            selected = Some((id, value));
        }
    }
    if !distinct && (first_pattern.is_none() || selected.is_some()) {
        return Ok(None);
    }
    if ambiguous || selected.is_none() {
        return Err(format!(
            "receiver {receiver:?} does not select a unique implementation of {name}"
        ));
    }
    Ok(selected.map(|(_, value)| value))
}

impl VbcModule {
    pub(crate) fn receiver_specialization(
        &self,
        receiver: &TypeRef,
        method: StringId,
        argc: u8,
    ) -> Result<Option<FunctionId>, String> {
        let TypeRef::Instantiated { base, .. } = receiver else {
            return Ok(None);
        };
        let Some(name) = self
            .get_string(method)
            .and_then(|name| name.rsplit('.').next())
        else {
            return Ok(None);
        };
        let Some(source_name) = self.strings.lookup(name) else {
            return Ok(None);
        };
        let families = self.receiver_method_families.get_or_init(|| {
            let mut families = MethodFamilies::new();
            for (index, function) in self.functions.iter().enumerate() {
                let (Some(parent), Some(identity)) =
                    (function.parent_type, &function.declaration_identity)
                else {
                    continue;
                };
                if identity.lexical_parent.is_none()
                    && function.has_source_body
                    && function
                        .source_signature
                        .as_ref()
                        .is_some_and(|decl| decl.is_method())
                {
                    families
                        .entry((parent, identity.source_name, function.params.len()))
                        .or_default()
                        .push(FunctionId(index as u32));
                }
            }
            families.retain(|_, ids| ids.len() > 1);
            families
        });
        let Some(ids) = families.get(&(*base, source_name, usize::from(argc) + 1)) else {
            return Ok(None);
        };
        select(
            name,
            receiver,
            ids.iter().map(|id| {
                let pattern = self
                    .get_function(*id)
                    .and_then(|f| f.semantic_parameter_types())
                    .and_then(|parameters| parameters.first()?.as_ref());
                (*id, *id, pattern)
            }),
        )
    }
}
