//! Recover imported receiver-owned roles only from exact semantic receiver
//! applications and their nominal declaration headers. Method-local slots remain
//! separate, including same-spelled shadowing parameters.
use super::impl_generic_rosters::{DeclaredGenericParameter, GenericRoster};
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::{
    module::{FunctionDescriptor, VbcModule},
    types::{TypeParamId, TypeRef},
};
use verum_ast::ty::GenericParamKind;
use verum_common::{Map, Set};

impl VbcCodegen {
    pub(super) fn complete_imported_callable_owners(
        &mut self,
        module: &VbcModule,
        descriptor: &FunctionDescriptor,
        remap: &std::collections::HashMap<u32, u32>,
        roster: &mut GenericRoster,
    ) -> CodegenResult<()> {
        if descriptor.source_signature.is_none() {
            let slots = descriptor
                .explicit_type_param_ids
                .iter()
                .copied()
                .flatten()
                .collect();
            let mut declared = Map::new();
            if let Some(parameters) =
                super::formal_parameters::remapped_parameters(descriptor, remap)
            {
                for parameter in parameters.iter().flatten() {
                    self.collect_receiver_value_roles(
                        parameter,
                        &slots,
                        &mut declared,
                        0,
                        &mut 4096,
                    )?;
                }
            }
            if declared
                .values()
                .any(|parameter| parameter.value_parameter().is_some())
            {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "imported value-generic callable lacks its source-owned parameter roles".into(),
                )));
            }
        }
        let local: Set<_> = roster
            .iter()
            .filter_map(|parameter| parameter.witness_slot)
            .collect();
        let owner: Set<_> = descriptor
            .type_params
            .iter()
            .map(|parameter| parameter.id)
            .filter(|id| !local.contains(id))
            .collect();
        if owner.is_empty() {
            return Ok(());
        }
        let mut roles = Map::new();
        if descriptor.parent_type.is_some() {
            let receiver = super::formal_parameters::remapped_parameters(descriptor, remap)
                .and_then(|parameters| parameters.first().cloned().flatten());
            if let Some(receiver) = receiver {
                self.collect_receiver_value_roles(&receiver, &owner, &mut roles, 0, &mut 4096)?;
            } else {
                let parent = descriptor
                    .parent_type
                    .and_then(|id| remap.get(&id.0))
                    .map(|id| crate::types::TypeId(*id));
                let known_value = parent
                    .and_then(|id| self.declaration_generic_roster(id))
                    .is_some_and(|header| {
                        header
                            .iter()
                            .any(|parameter| parameter.value_parameter().is_some())
                    });
                if known_value {
                    return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                        "imported value-generic method lacks its declaration-owned semantic receiver".into())));
                }
            }
        }
        for parameter in &descriptor.type_params {
            if !owner.contains(&parameter.id) {
                continue;
            }
            let name = module.strings.get(parameter.name).ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "imported generic slot has no owned name".into(),
                ))
            })?;
            let mut declared = roles.remove(&parameter.id).ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "imported callable owner slot lacks its declaration-owned receiver role".into(),
                ))
            })?;
            declared.name = name.to_owned();
            roster.push(declared);
        }
        Ok(())
    }

    fn collect_receiver_value_roles(
        &mut self,
        receiver: &TypeRef,
        owned: &Set<TypeParamId>,
        roles: &mut Map<TypeParamId, DeclaredGenericParameter>,
        depth: usize,
        budget: &mut usize,
    ) -> CodegenResult<()> {
        let invalid = |message: &str| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into()))
        };
        if depth > 64 || *budget == 0 {
            return Err(invalid(
                "receiver generic declaration exceeds structural limit",
            ));
        }
        *budget -= 1;
        match receiver {
            TypeRef::Reference { inner, .. }
            | TypeRef::Slice(inner)
            | TypeRef::Array { element: inner, .. } => {
                self.collect_receiver_value_roles(inner, owned, roles, depth + 1, budget)?
            }
            TypeRef::Instantiated { base, args } => {
                let header = self
                    .declaration_generic_roster(*base)
                    .ok_or_else(|| invalid("receiver has no declaration-owned generic roles"))?
                    .into_owned();
                if header.len() != args.len() {
                    return Err(invalid(
                        "receiver argument count does not match its declared generic roles",
                    ));
                }
                let descriptor = self
                    .type_by_id(*base)
                    .ok_or_else(|| invalid("receiver generic roles have no nominal descriptor"))?;
                let owner = descriptor
                    .origin_module
                    .and_then(|id| self.ctx.strings.get(id.0 as usize))
                    .cloned();
                let scope = descriptor
                    .type_params
                    .iter()
                    .filter_map(|parameter| {
                        self.ctx
                            .strings
                            .get(parameter.name.0 as usize)
                            .map(|name| (name.clone(), parameter.id.0))
                    })
                    .collect();
                for (parameter, argument) in header.into_iter().zip(args) {
                    if let Some(source) = parameter.value_parameter() {
                        let TypeRef::Generic(id) = argument else {
                            if !matches!(argument, TypeRef::ConstValue(_)) {
                                return Err(invalid(
                                    "receiver value slot has no represented witness",
                                ));
                            }
                            continue;
                        };
                        if !owned.contains(id) {
                            continue;
                        }
                        let ty = match &source.kind {
                            GenericParamKind::Meta { ty, .. }
                            | GenericParamKind::Const { ty, .. } => ty,
                            _ => continue,
                        };
                        let owner = owner.as_ref().ok_or_else(|| {
                            invalid("receiver value domain has no declared lexical owner")
                        })?;
                        let previous = self.default_body_type_scope.replace(owner.clone());
                        let carrier = self
                            .resolve_signature_type_ref(ty, &scope)
                            .ok()
                            .and_then(|domain| self.runtime_value_carrier(&domain));
                        self.default_body_type_scope = previous;
                        let mut parameter = parameter.clone();
                        parameter.witness_slot = Some(*id);
                        parameter.explicit_slot = None;
                        parameter.value_carrier = carrier;
                        if let Some(prior) = roles.get(id) {
                            if prior.value_parameter().is_none()
                                || prior.value_carrier != parameter.value_carrier
                            {
                                return Err(invalid(
                                    "receiver gives a generic slot conflicting value carriers",
                                ));
                            }
                        } else {
                            roles.insert(*id, parameter);
                        }
                    } else if let TypeRef::Generic(id) = argument {
                        if owned.contains(id) {
                            let mut parameter = parameter.clone();
                            parameter.witness_slot = Some(*id);
                            parameter.explicit_slot = None;
                            if roles
                                .get(id)
                                .is_some_and(|prior| prior.value_parameter().is_some())
                            {
                                return Err(invalid(
                                    "receiver gives a generic slot conflicting type and value roles",
                                ));
                            }
                            roles.insert(*id, parameter);
                        }
                    } else {
                        self.collect_receiver_value_roles(
                            argument,
                            owned,
                            roles,
                            depth + 1,
                            budget,
                        )?;
                    }
                }
            }
            TypeRef::Tuple(elements) => {
                for element in elements {
                    self.collect_receiver_value_roles(element, owned, roles, depth + 1, budget)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}
