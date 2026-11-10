//! Callable generic arguments use the same declaration-owned roles as nominal
//! applications. Slot identity comes from the selected function, never a name.
use super::impl_generic_rosters::{DeclaredGenericParameter, GenericRoster};
use super::source_substitution::SourceTypeSubstitution;
use super::{
    CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, ImplReceiverWitness, VbcCodegen,
};
use crate::{
    module::{FunctionDescriptor, FunctionId, VbcModule},
    types::{TypeId, TypeParamId, TypeRef},
};
use std::borrow::Cow;
use verum_ast::{
    Expr, ExprKind, FunctionDecl,
    ty::{GenericArg, GenericParamKind, TypeKind},
};

impl VbcCodegen {
    fn callable_source_roster(source: &FunctionDecl) -> GenericRoster {
        let mut explicit = 0;
        source
            .generics
            .iter()
            .map(|parameter| {
                let name = match &parameter.kind {
                    GenericParamKind::Type { name, .. }
                    | GenericParamKind::Meta { name, .. }
                    | GenericParamKind::Const { name, .. }
                    | GenericParamKind::HigherKinded { name, .. }
                    | GenericParamKind::KindAnnotated { name, .. }
                    | GenericParamKind::Level { name }
                    | GenericParamKind::Context { name } => name.name.to_string(),
                    GenericParamKind::Lifetime { name } => name.name.to_string(),
                };
                let explicit_slot = (!parameter.is_implicit).then(|| {
                    let slot = explicit;
                    explicit += 1;
                    slot
                });
                DeclaredGenericParameter {
                    explicit_slot,
                    name,
                    source: Some(std::sync::Arc::new(parameter.clone())),
                    value_carrier: None,
                    witness_slot: None,
                }
            })
            .collect()
    }

    /// Retain the complete body-free declaration for every source callable.
    /// ABI type slots cannot preserve bounds, defaults, value kinds or receivers.
    pub(super) fn retain_callable_owner_signature(
        &self, source: &FunctionDecl, signature: Option<FunctionDecl>,
        parent: Option<&str>, _dense: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<Option<FunctionDecl>> {
        if let Some(signature) = signature { return Ok(Some(signature)); }
        verum_ast::source_signature::capture_source_signature(source, parent.is_some())
            .map(Some)
            .map_err(|message| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into_string())))
    }

    /// Elaborate an omitted implementation telescope using only the inherited
    /// binders selected from its actual nominal target. An empty concrete
    /// implementation remains empty, even when a method has same-named locals.
    pub(super) fn implementation_source_parameters(
        &self,
        implementation: &verum_ast::decl::ImplDecl,
    ) -> CodegenResult<verum_common::List<verum_ast::ty::GenericParam>> {
        if !implementation.generics.is_empty() {
            return Ok(implementation.generics.clone());
        }
        let inherited = self.resolve_impl_generics(implementation).2;
        if inherited.is_empty() {
            return Ok(verum_common::List::new());
        }
        let roster = self
            .implementation_nominal_id(&implementation.kind)
            .and_then(|id| self.declaration_generic_roster(id))
            .ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "implicit implementation parameters require the target declaration".into(),
                ))
            })?;
        inherited
            .iter()
            .map(|name| {
                roster
                    .iter()
                    .find(|parameter| &parameter.name == name)
                    .and_then(|parameter| parameter.source.as_deref())
                    .cloned()
                    .ok_or_else(|| {
                        CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                            "implicit implementation parameter lacks its source-owned role".into(),
                        ))
                    })
            })
            .collect()
    }

    pub(super) fn claim_callable_generic_roster(
        &mut self,
        id: FunctionId,
        source: &FunctionDecl,
        parent: Option<&str>,
        dense: &std::collections::HashMap<String, u16>,
        shadow: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<()> {
        let mut roster = self.source_callable_owner_roster(parent, dense)?;
        let mut locals = Self::callable_source_roster(source);
        for parameter in &mut locals {
            parameter.witness_slot = if matches!(parameter.source.as_deref().map(|source| &source.kind),
                Some(GenericParamKind::Level { .. } | GenericParamKind::Lifetime { .. } | GenericParamKind::Context { .. })) {
                None
            } else { shadow
                .get(&parameter.name)
                .or_else(|| dense.get(&parameter.name))
                .copied()
                .map(TypeParamId) };
            if parameter.value_parameter().is_some() {
                parameter.value_carrier = self
                    .current_source_substitution
                    .get_value(&parameter.name)
                    .and_then(|(_, domain)| self.runtime_value_carrier(domain));
            }
        }
        roster.extend(locals);
        self.source_callable_rosters.insert(id, Some(roster));
        Ok(())
    }

    fn source_callable_owner_roster(
        &self,
        parent: Option<&str>,
        dense: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<GenericRoster> {
        let Some(parent) = parent else {
            return Ok(Vec::new());
        };
        let source = self
            .current_deferred_impl
            .as_ref()
            .map(|scope| scope.declaration.ast_generics.as_slice())
            .or(self.current_impl_ast_generics.as_deref());
        let owner = if let Some(source) = source {
            Self::generic_roster(source).ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "implementation has an unsupported generic roster".into(),
                ))
            })?
        } else {
            self.nominal_type_id(parent)
                .and_then(|id| self.declaration_generic_roster(id))
                .map(|roster| roster.into_owned())
                .unwrap_or_default()
        };
        let mut result = Vec::new();
        for mut parameter in owner {
            let Some(id) = dense.get(&parameter.name).copied() else {
                if parameter.value_parameter().is_some() {
                    return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                        "implementation value parameter lacks its declaration-owned witness slot"
                            .into(),
                    )));
                }
                continue;
            };
            parameter.witness_slot = Some(TypeParamId(id));
            parameter.explicit_slot = None;
            if let Some(source) = parameter.value_parameter() {
                let ty = match &source.kind {
                    GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. } => ty,
                    _ => continue,
                };
                parameter.value_carrier = self
                    .resolve_signature_type_ref(ty, dense)
                    .ok()
                    .and_then(|domain| self.runtime_value_carrier(&domain));
            }
            result.push(parameter);
        }
        Ok(result)
    }

    pub(super) fn imported_callable_generic_roster(
        &mut self,
        module: &VbcModule,
        descriptor: &FunctionDescriptor,
        remap: &std::collections::HashMap<u32, u32>,
    ) -> Option<GenericRoster> {
        let mut roster =
            Self::descriptor_generic_roster(descriptor, |name| module.strings.get(name))?;
        let owner = descriptor
            .origin_module
            .and_then(|id| module.strings.get(id))
            .unwrap_or(&module.name);
        let previous_owner = self.default_body_type_scope.replace(owner.to_owned());
        let previous_substitution = std::mem::take(&mut self.current_source_substitution);
        let scope = descriptor
            .type_params
            .iter()
            .filter_map(|parameter| {
                module
                    .strings
                    .get(parameter.name)
                    .map(|name| (name.to_owned(), parameter.id.0))
            })
            .collect();
        for parameter in &mut roster {
            if let Some(source) = parameter.value_parameter() {
                let ty = match &source.kind {
                    GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. } => ty,
                    _ => continue,
                };
                parameter.value_carrier = self
                    .resolve_signature_type_ref(ty, &scope)
                    .ok()
                    .and_then(|domain| self.runtime_value_carrier(&domain));
            }
        }
        let completion =
            self.complete_imported_callable_owners(module, descriptor, remap, &mut roster);
        self.current_source_substitution = previous_substitution;
        self.default_body_type_scope = previous_owner;
        if let Err(error) = completion {
            let name = module.strings.get(descriptor.name).unwrap_or("<missing name>");
            self.record_source_declaration_error(error.with_context(format!("imported callable {owner}.{name}")));
            return None;
        }
        Some(roster)
    }

    pub(super) fn runtime_value_carrier(&self, domain: &TypeRef) -> Option<TypeRef> {
        fn resolve(cg: &VbcCodegen, domain: &TypeRef, active: &mut verum_common::Set<TypeId>, depth: usize) -> Option<TypeRef> {
            if depth > verum_common::const_value::MAX_SEQUENCE_DEPTH { return None; }
            match domain {
                TypeRef::Concrete(id) => {
                    let Some(target) = cg.type_by_id(*id).and_then(|descriptor| descriptor.alias_target.as_ref())
                    else { return Some(domain.clone()); };
                    if !active.insert(*id) { return None; }
                    resolve(cg, target, active, depth + 1)
                }
                TypeRef::Slice(element) => Some(TypeRef::Slice(Box::new(resolve(cg, element, active, depth + 1)?))),
                TypeRef::Array { element, length } => Some(TypeRef::Array {
                    element: Box::new(resolve(cg, element, active, depth + 1)?), length: *length
                }),
                _ => None,
            }
        }
        resolve(self, domain, &mut Default::default(), 0)
    }

    pub(super) fn descriptor_generic_roster<'a>(
        descriptor: &FunctionDescriptor,
        string: impl Fn(crate::types::StringId) -> Option<&'a str>,
    ) -> Option<GenericRoster> {
        if let Some(source) = &descriptor.source_signature {
            let mut roster = Self::callable_source_roster(source);
            let mut explicit = descriptor.explicit_type_param_ids.iter();
            for parameter in &mut roster {
                if parameter
                    .source
                    .as_ref()
                    .is_some_and(|source| !source.is_implicit)
                {
                    parameter.witness_slot = explicit.next().copied().flatten();
                } else {
                    let mut matches = descriptor
                        .type_params
                        .iter()
                        .filter(|slot| string(slot.name) == Some(parameter.name.as_str()));
                    let only = matches.next();
                    if matches.next().is_none() {
                        parameter.witness_slot = only.map(|slot| slot.id);
                    }
                }
            }
            return Some(roster);
        }
        descriptor
            .explicit_type_param_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let parameter = descriptor
                    .type_params
                    .iter()
                    .find(|parameter| Some(parameter.id) == *id)?;
                Some(DeclaredGenericParameter {
                    value_carrier: None,
                    witness_slot: *id,
                    explicit_slot: Some(index),
                    name: string(parameter.name)?.to_owned(),
                    source: None,
                })
            })
            .collect()
    }

    pub(super) fn callable_generic_roster(
        &self,
        function: &FunctionInfo,
    ) -> Option<Cow<'_, [DeclaredGenericParameter]>> {
        if let Some(roster) = self.source_callable_rosters.get(&function.id) {
            return roster
                .as_ref()
                .map(|roster| Cow::Borrowed(roster.as_slice()));
        }
        let descriptor = self
            .functions
            .iter()
            .find(|entry| entry.descriptor.id == function.id)
            .map(|entry| &entry.descriptor)?;
        Self::descriptor_generic_roster(descriptor, |id| {
            self.ctx.strings.get(id.0 as usize).map(String::as_str)
        })
        .map(Cow::Owned)
    }

    /// Both executable CallG and declaration inference consume this conversion.
    pub(super) fn callable_argument_witness(
        &self,
        function: &FunctionInfo,
        index: usize,
        argument: &GenericArg,
        expected: Option<&TypeRef>,
    ) -> CodegenResult<Option<TypeRef>> {
        let invalid =
            |message: &str| CodegenError::new(CodegenErrorKind::TypeInference(message.into()));
        let roster = self.callable_generic_roster(function).ok_or_else(|| {
            invalid("explicit generic argument requires declaration-owned slot metadata")
        })?;
        let parameter = roster
            .iter()
            .find(|parameter| parameter.explicit_slot == Some(index))
            .ok_or_else(|| {
                invalid("explicit generic argument exceeds the selected declaration's roster")
            })?;
        if parameter.witness_slot.is_none() {
            let erased = parameter
                .source
                .as_deref()
                .is_some_and(|source| match &source.kind {
                    GenericParamKind::Lifetime { .. } => {
                        matches!(argument, GenericArg::Lifetime(_))
                    }
                    GenericParamKind::Level { .. } => self.valid_erased_level_argument(argument),
                    GenericParamKind::Context { .. }
                    | GenericParamKind::HigherKinded { .. }
                    | GenericParamKind::KindAnnotated { .. } => {
                        argument.type_for_declared_slot().is_some()
                    }
                    _ => false,
                });
            return if erased {
                Ok(None)
            } else {
                Err(invalid(
                    "generic parameter has no declaration-owned runtime witness slot",
                ))
            };
        }
        if parameter.value_parameter().is_some() {
            let unsupported = || invalid("selected value parameter has no supported runtime scalar carrier");
            let carrier = parameter.value_carrier.as_ref().ok_or_else(unsupported)?;
            if *carrier != TypeRef::Concrete(TypeId::INT) {
                if crate::const_sequence::sequence_domain(carrier).is_none() { return Err(unsupported()); }
                if !crate::const_sequence::ConstSequence::supported_domain(carrier) {
                    return Err(invalid("unsupported element domain in constant sequence"));
                }
            }
            let expression = argument.value_for_declared_slot()
                .ok_or_else(|| invalid("value parameter received a type argument"))?;
            let expression = Self::normalized_place(&expression);
            if let ExprKind::Path(path) = &expression.kind
                && let Some(name) = path.as_ident()
            {
                if let Some((witness, domain)) = self
                    .current_source_substitution
                    .get_value(name.name.as_str())
                {
                    if self.runtime_value_carrier(domain).as_ref() != Some(carrier) {
                        return Err(invalid(
                            "runtime value argument has no supported scalar witness representation",
                        ));
                    }
                    return Ok(Some(witness.ty.clone()));
                }
                if self
                    .ctx
                    .current_generic_param_id(name.name.as_str())
                    .is_some()
                {
                    return Err(invalid("type parameter cannot occupy a value slot"));
                }
            }
            return self.closed_value_argument(expression, carrier).map(Some);
        }
        let projected = argument
            .type_for_declared_slot()
            .ok_or_else(|| invalid("type parameter received a value argument"))?;
        self.explicit_type_witness(&projected, expected)
            .map(Some)
            .ok_or_else(|| invalid("explicit structural type requires a declared type and checked nonnegative array count"))
    }

    fn valid_erased_level_argument(&self, argument: &GenericArg) -> bool {
        use verum_ast::UniverseLevelExpr;
        match argument.universe_for_declared_slot() {
            Ok(None | Some(UniverseLevelExpr::Concrete(_))) => true,
            Ok(Some(UniverseLevelExpr::Variable(name))) =>
                self.current_source_substitution.has_level(name.name.as_str()),
            _ => false,
        }
    }

    pub(super) fn callable_has_value_parameters(&self, function: &FunctionInfo) -> bool {
        self.callable_generic_roster(function)
            .is_some_and(|roster| {
                roster
                    .iter()
                    .any(|parameter| parameter.value_parameter().is_some())
            })
    }

    pub(super) fn validate_callable_value_bindings(
        &self,
        function: &FunctionInfo,
        bindings: &std::collections::BTreeMap<u16, TypeRef>,
    ) -> CodegenResult<()> {
        let Some(roster) = self.callable_generic_roster(function) else {
            return Ok(());
        };
        Self::check_value_bindings(&roster, bindings, |argument, carrier| {
            self.current_source_substitution
                .value_witness_domain(argument)
                .and_then(|domain| self.runtime_value_carrier(domain))
                .as_ref() == Some(carrier)
        })
    }

    fn check_value_bindings(
        roster: &[DeclaredGenericParameter],
        bindings: &std::collections::BTreeMap<u16, TypeRef>,
        caller_value: impl Fn(&TypeRef, &TypeRef) -> bool,
    ) -> CodegenResult<()> {
        for parameter in roster
            .iter()
            .filter(|parameter| parameter.value_parameter().is_some())
        {
            let invalid = || {
                CodegenError::new(CodegenErrorKind::TypeInference(
                    format!("value parameter requires an exact declaration-owned call witness: '{}' (slot {:?}, carrier {:?}); supplied {:?}",
                        parameter.name, parameter.witness_slot, parameter.value_carrier, bindings),
                ))
            };
            let carrier = parameter.value_carrier.as_ref().ok_or_else(invalid)?;
            let scalar = *carrier == TypeRef::Concrete(TypeId::INT);
            if !scalar && !crate::const_sequence::ConstSequence::supported_domain(carrier) { return Err(invalid()); }
            let slot = parameter.witness_slot.ok_or_else(invalid)?;
            match bindings.get(&slot.0).ok_or_else(invalid)? {
                TypeRef::ConstValue(_) if scalar => (),
                TypeRef::ConstSequence(sequence) if sequence.matches_domain(carrier) => (),
                argument @ TypeRef::Generic(_) if caller_value(argument, carrier) => (),
                _ => return Err(invalid()),
            }
        }
        Ok(())
    }

    /// All selected direct calls pass this final publication boundary, including
    /// synthesized formatting/residual calls. It never repairs missing evidence.
    pub(super) fn validate_emitted_value_calls(&self) -> CodegenResult<()> {
        use crate::instruction::Instruction;
        for caller in &self.functions {
            let caller_roster = self
                .source_callable_rosters
                .get(&caller.descriptor.id)
                .and_then(Option::as_ref);
            for instruction in &caller.instructions {
                let (target, arguments): (u32, &[TypeRef]) = match instruction {
                    Instruction::CallG {
                        func_id, type_args, ..
                    } => (*func_id, type_args),
                    Instruction::Call { func_id, .. } | Instruction::TailCall { func_id, .. } => {
                        (*func_id, &[])
                    }
                    _ => continue,
                };
                let target = FunctionId(target);
                let Some(roster) = self
                    .source_callable_rosters
                    .get(&target)
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                if !roster
                    .iter()
                    .any(|parameter| parameter.value_parameter().is_some())
                {
                    continue;
                }
                let declared = self.ctx.lookup_function_by_id(target).ok_or_else(|| {
                    CodegenError::new(CodegenErrorKind::TypeInference(
                        "value-generic call has no selected declaration".into(),
                    ))
                })?;
                let bindings = declared
                    .type_param_ids
                    .iter()
                    .zip(arguments)
                    .map(|(id, argument)| (id.0, argument.clone()))
                    .collect();
                Self::check_value_bindings(roster, &bindings, |argument, carrier| {
                    let TypeRef::Generic(id) = argument else {
                        return false;
                    };
                    caller_roster.is_some_and(|roster| {
                        roster.iter().any(|parameter| {
                            parameter.witness_slot == Some(*id)
                                && parameter.value_parameter().is_some()
                                && parameter.value_carrier.as_ref() == Some(carrier)
                        })
                    })
                })?;
            }
        }
        Ok(())
    }

    /// Reuse the existing typed substitution environment to keep symbolic value
    /// binders distinct from type binders while elaborating source signatures.
    pub(super) fn callable_value_scope(
        &self,
        function: &FunctionDecl,
        owner: Option<&str>,
        inherited: &[String],
    ) -> CodegenResult<SourceTypeSubstitution> {
        let (dense, shadow) = self.function_generic_param_maps(function, owner, inherited);
        let mut scope = dense.clone();
        scope.extend(shadow);
        let mut substitution = self.current_source_substitution.for_function(function);
        if let Some(owner) = owner {
            if let Some(declaration) = &self.current_deferred_impl {
                self.bind_source_value_parameters(
                    &declaration.declaration.ast_generics,
                    &dense,
                    &mut substitution,
                )?;
            } else if let Some(generics) = &self.current_impl_ast_generics {
                self.bind_source_value_parameters(generics, &dense, &mut substitution)?;
            } else if let Some(roster) = self
                .nominal_type_id(owner)
                .and_then(|id| self.declaration_generic_roster(id))
            {
                for parameter in roster
                    .iter()
                    .filter(|parameter| dense.contains_key(&parameter.name))
                {
                    if let Some(source) = &parameter.source {
                        self.bind_source_value_parameters(
                            std::slice::from_ref(source),
                            &dense,
                            &mut substitution,
                        )?;
                    }
                }
            }
        }
        // Function-local binders shadow the outer roles as well as their names.
        substitution = substitution.for_function(function);
        self.bind_source_value_parameters(&function.generics, &scope, &mut substitution)?;
        Ok(substitution)
    }

    /// Method-local binders take precedence over the protocol's binders in
    /// both the signature and the compile-time value environment.
    pub(super) fn protocol_member_signature(
        &mut self,
        function: &FunctionDecl,
        owner: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<TypeRef> {
        let (mut scope, local) = Self::extend_function_generic_scope(function, owner.clone());
        scope.extend(local);
        let mut substitution = self.current_source_substitution.for_function(function);
        self.bind_source_value_parameters(&function.generics, &scope, &mut substitution)?;
        self.with_protocol_source_scope(
            Some(self.lexical_type_scope().to_owned()),
            substitution,
            None,
            |cg| {
                let params = function
                    .params
                    .iter()
                    .filter_map(|parameter| match &parameter.kind {
                        verum_ast::FunctionParamKind::Regular { ty, .. } => {
                            Some(cg.resolve_signature_type_ref(ty, &scope))
                        }
                        _ => None,
                    })
                    .collect::<CodegenResult<Vec<_>>>()?;
                let result = function
                    .return_type
                    .as_ref()
                    .map(|ty| cg.resolve_signature_type_ref(ty, &scope))
                    .transpose()?
                    .unwrap_or(TypeRef::Concrete(TypeId::UNIT));
                Ok(TypeRef::Function {
                    is_async: function.is_async,
                    params,
                    return_type: Box::new(result),
                    contexts: Default::default(),
                })
            },
        )
    }

    pub(super) fn implementation_value_scope(
        &self,
        target: &verum_ast::Type,
        scope: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<SourceTypeSubstitution> {
        let mut substitution = self.current_source_substitution.clone();
        if let Some(parameters) = self.current_impl_ast_generics.as_deref() {
            substitution = substitution.for_parameters(parameters);
            self.bind_source_value_parameters(parameters, scope, &mut substitution)?;
        } else if let Some(roster) = self
            .implementation_type_nominal_id(target)
            .and_then(|id| self.declaration_generic_roster(id))
        {
            for parameter in roster
                .iter()
                .filter(|parameter| scope.contains_key(&parameter.name))
            {
                if let Some(source) = &parameter.source {
                    self.bind_source_value_parameters(
                        std::slice::from_ref(source),
                        scope,
                        &mut substitution,
                    )?;
                }
            }
        }
        Ok(substitution)
    }

    pub(super) fn nominal_value_scope(
        &self,
        declaration: &verum_ast::TypeDecl,
    ) -> CodegenResult<SourceTypeSubstitution> {
        if !declaration.generics.iter().any(|parameter| {
            matches!(
                parameter.kind,
                GenericParamKind::Meta { .. } | GenericParamKind::Const { .. }
            )
        }) {
            return Ok(self
                .current_source_substitution
                .for_parameters(&declaration.generics));
        }
        let roster = Self::generic_roster(&declaration.generics).ok_or_else(|| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                "nominal declaration has an unsupported parameter roster".into(),
            ))
        })?;
        let scope = roster
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                u16::try_from(index)
                    .map(|id| (parameter.name.clone(), id))
                    .map_err(|_| {
                        CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                            "nominal parameter roster exceeds witness index space".into(),
                        ))
                    })
            })
            .collect::<CodegenResult<std::collections::HashMap<_, _>>>()?;
        let mut substitution = self
            .current_source_substitution
            .for_parameters(&declaration.generics);
        self.bind_source_value_parameters(&declaration.generics, &scope, &mut substitution)?;
        Ok(substitution)
    }

    fn bind_source_value_parameters(
        &self,
        parameters: &[verum_ast::ty::GenericParam],
        scope: &std::collections::HashMap<String, u16>,
        substitution: &mut SourceTypeSubstitution,
    ) -> CodegenResult<()> {
        for parameter in parameters {
            let (name, ty) = match &parameter.kind {
                GenericParamKind::Meta { name, ty, .. } | GenericParamKind::Const { name, ty } => {
                    (name.name.as_str(), ty)
                }
                _ => continue,
            };
            let id = scope.get(name).copied().ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "value parameter has no declaration-owned witness slot".into(),
                ))
            })?;
            let value_type = self.resolve_signature_type_ref(ty, scope)?;
            substitution.insert_value(
                name.to_owned(),
                ImplReceiverWitness {
                    ty: TypeRef::Generic(TypeParamId(id)),
                    semantic: true,
                },
                value_type,
            );
        }
        Ok(())
    }
}
