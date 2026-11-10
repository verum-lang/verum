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
        let selected_source = descriptor.source_signature.as_ref().ok_or_else(|| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
                "imported callable {}.{} owner slots lack the selected source signature",
                descriptor.origin_module.and_then(|id| module.strings.get(id)).unwrap_or(&module.name),
                module.strings.get(descriptor.name).unwrap_or("<missing name>"))))
        })?;
        let mut roles = Map::new();
        if descriptor.parent_type.is_some()
            && !selected_source.is_method()
        {
            roles = self.imported_implementation_owner_roles(module, descriptor, remap, &owner)?;
        } else if descriptor.parent_type.is_some() {
            let receiver = super::formal_parameters::remapped_parameters(descriptor, remap)
                .and_then(|parameters| parameters.first().cloned().flatten());
            if let Some(receiver) = receiver {
                let parent = descriptor.parent_type.and_then(|id| remap.get(&id.0))
                    .copied().map(crate::types::TypeId).ok_or_else(|| CodegenError::new(
                        CodegenErrorKind::InvalidSourceSignature("receiver has no remapped declaring nominal".into())))?;
                let mut nominal = &receiver;
                while let TypeRef::Reference { inner, .. } = nominal { nominal = inner; }
                let selected = match nominal {
                    TypeRef::Instantiated { base, .. } => Some(*base),
                    TypeRef::Concrete(id) if self.declaration_generic_roster(*id)
                        .is_some_and(|parameters| parameters.is_empty()) => Some(*id),
                    _ => None,
                };
                if selected != Some(parent) {
                    return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                        "semantic receiver lacks its declaration-owned nominal application".into())));
                }
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
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    if known_value { "imported value-generic method lacks its declaration-owned semantic receiver" }
                    else { "imported method lacks its declaration-owned semantic receiver" }.into())));
            }
            // An implementation can also bind parameters used only by its
            // bounds (for example B in F: fn(I.Item) -> B). They belong to
            // the exact implementation even though Self<I, F> omits them.
            if roles.len() != owner.len() {
                let declared = self.imported_implementation_owner_roles(module, descriptor, remap, &owner)?;
                for (id, receiver_role) in &roles {
                    let corresponding = declared.get(id).ok_or_else(|| CodegenError::new(
                        CodegenErrorKind::InvalidSourceSignature("receiver slot is absent from its selected implementation".into())))?;
                    if receiver_role.value_parameter().is_some() != corresponding.value_parameter().is_some()
                        || receiver_role.value_carrier != corresponding.value_carrier
                    { return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                        "receiver role conflicts with its selected implementation".into()))); }
                }
                roles = declared;
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
                    format!("imported callable {}.{} owner slot {} lacks its declaration-owned receiver role",
                        descriptor.origin_module.and_then(|id| module.strings.get(id)).unwrap_or(&module.name),
                        module.strings.get(descriptor.name).unwrap_or("<missing name>"), parameter.id.0),
                ))
            })?;
            declared.name = name.to_owned();
            roster.push(declared);
        }
        Ok(())
    }

    /// The exact selected implementation owns static and receiver-independent
    /// parameters. An ordinary argument supplies no receiver evidence.
    fn imported_implementation_owner_roles(
        &mut self, module: &VbcModule, descriptor: &FunctionDescriptor,
        remap: &std::collections::HashMap<u32, u32>, owned: &Set<TypeParamId>,
    ) -> CodegenResult<Map<TypeParamId, DeclaredGenericParameter>> {
        let identity = format!("{}.{}",
            descriptor.origin_module.and_then(|id| module.strings.get(id)).unwrap_or("<missing owner>"),
            module.strings.get(descriptor.name).unwrap_or("<missing name>"));
        let invalid = |message: &str| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
            format!("imported callable {identity}: {message}")));
        let source = descriptor.source_signature.as_ref()
            .ok_or_else(|| invalid("missing selected source signature"))?;
        let owner = descriptor.origin_module.and_then(|id| module.strings.get(id))
            .ok_or_else(|| invalid("missing declaration owner"))?;
        let parent_id = descriptor.parent_type.ok_or_else(|| invalid("missing declaring nominal"))?;
        let parent = module.types.iter().find(|ty| ty.id == parent_id)
            .ok_or_else(|| invalid("missing declaring nominal descriptor"))?;
        let mut matches = Vec::new();
        // The declaration interface and the completed module may both carry
        // this implementation with different local dispatch slots. Generic
        // parameters belong to its exact source declaration.
        let mut seen = std::collections::HashSet::new();
        for carrier in parent.source_inherent_implementations.iter().chain(
            parent.protocols.iter().filter_map(|implementation| implementation.source_implementation.as_ref()))
        {
            if carrier.origin_module() != owner || !seen.insert(carrier) { continue; }
            let declaration = carrier.declaration().map_err(|_| invalid("malformed implementation authority"))?;
            if declaration.items.iter().any(|item| matches!(&item.kind,
                verum_ast::decl::ImplItemKind::Function(method) if method == source))
            { matches.push(declaration); }
        }
        if matches.len() != 1 { return Err(invalid("selected method requires one exact retained implementation")); }
        let implementation = matches.pop().unwrap();
        let target = remap.get(&parent_id.0).copied().map(crate::types::TypeId)
            .ok_or_else(|| invalid("declaring nominal was not remapped"))?;
        if self.implementation_nominal_id(&implementation.kind) != Some(target) {
            return Err(invalid("retained implementation has a different nominal target"));
        }
        let parameters = self.implementation_source_parameters(&implementation)?;
        let declared = Self::generic_roster(&parameters)
            .ok_or_else(|| invalid("unsupported implementation parameter roles"))?;
        let names = declared.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>();
        let (dense, shadow) = self.declared_impl_generic_param_maps(source, &implementation.kind, &names);
        let expected = dense.iter().chain(&shadow).map(|(name, id)| (name.as_str(), *id))
            .collect::<std::collections::BTreeSet<_>>();
        let actual = descriptor.type_params.iter().map(|slot| {
            module.strings.get(slot.name).map(|name| (name, slot.id.0))
                .ok_or_else(|| invalid("generic slot lacks its declared name"))
        }).collect::<CodegenResult<std::collections::BTreeSet<_>>>()?;
        if expected != actual || actual.len() != descriptor.type_params.len() {
            return Err(invalid("generic slots differ from the selected implementation and method"));
        }
        let scope = self.declared_impl_generic_scope(&implementation.kind, &names);
        let mut roles = Map::new();
        for mut parameter in declared {
            let id = scope.get(&parameter.name).copied().map(TypeParamId)
                .ok_or_else(|| invalid("implementation parameter has no declared witness slot"))?;
            if !owned.contains(&id) { return Err(invalid("implementation slot was erased or claimed by a method-local binder")); }
            parameter.witness_slot = Some(id);
            parameter.explicit_slot = None;
            if let Some(source) = parameter.value_parameter() {
                let ty = match &source.kind { GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. } => ty, _ => unreachable!() };
                let domain = self.resolve_signature_type_ref(ty, &scope)?;
                parameter.value_carrier = self.runtime_value_carrier(&domain);
            }
            roles.insert(id, parameter);
        }
        if roles.len() != owned.len() { return Err(invalid("owner slots do not match the selected implementation")); }
        Ok(roles)
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
