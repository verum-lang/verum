//! A named predicate is evaluated in its declaration's lexical environment.
//! The value register remains in the enclosing function; local names do not.
use super::source_substitution::SourceTypeSubstitution;
use super::{
    CodegenError, CodegenErrorKind, CodegenResult, FieldRefinementInfo, ImplReceiverWitness,
    VbcCodegen,
};
use crate::{
    instruction::Reg,
    mono::TypeSubstitution,
    types::{TypeDescriptor, TypeRef},
};

#[derive(Debug, Clone)]
pub(super) struct RefinementScope {
    owner: verum_common::Text,
    substitution: SourceTypeSubstitution,
}

impl VbcCodegen {
    pub(super) fn nominal_refinement_scope(
        &self,
        declaration: &TypeDescriptor,
        arguments: &[TypeRef],
    ) -> CodegenResult<RefinementScope> {
        let invalid = |message: &str| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into()))
        };
        let owner = declaration
            .origin_module
            .map(|id| {
                self.ctx
                    .strings
                    .get(id.0 as usize)
                    .cloned()
                    .ok_or_else(|| invalid("refinement declaration has no lexical owner"))
            })
            .transpose()?
            .unwrap_or_else(|| self.config.module_name.clone());
        if declaration.type_params.len() != arguments.len() {
            return Err(invalid(
                "refinement application lacks its exact declaration arguments",
            ));
        }
        let mut substitution = SourceTypeSubstitution::default();
        if !arguments.is_empty() {
            let roster = self
                .declaration_generic_roster(declaration.id)
                .filter(|roster| roster.len() == arguments.len())
                .ok_or_else(|| {
                    invalid("refinement application lacks its declaration-owned parameter roles")
                })?;
            for ((parameter, descriptor), argument) in
                roster.iter().zip(&declaration.type_params).zip(arguments)
            {
                if self.ctx.strings.get(descriptor.name.0 as usize) != Some(&parameter.name) {
                    return Err(invalid(
                        "refinement application has mismatched declaration slots",
                    ));
                }
                let witness = ImplReceiverWitness {
                    ty: argument.clone(),
                    semantic: true,
                };
                if let Some(source) = parameter.value_parameter() {
                    let (verum_ast::ty::GenericParamKind::Meta { ty, .. }
                    | verum_ast::ty::GenericParamKind::Const { ty, .. }) = &source.kind
                    else {
                        return Err(invalid(
                            "refinement value parameter has no declared carrier",
                        ));
                    };
                    let mut active = verum_common::Set::new();
                    let mut cache = verum_common::Map::new();
                    let mut count_error = None;
                    let mut count = |expression: &verum_ast::Expr| match self
                        .const_eval_i64_in_scope(
                            expression,
                            Some(&owner),
                            None,
                            false,
                            &mut active,
                            &mut cache,
                            0,
                        ) {
                        Ok(value) => value.and_then(|value| u64::try_from(value).ok()),
                        Err(error) => {
                            count_error = Some(error);
                            None
                        }
                    };
                    let carrier =
                        substitution.elaborate(&Default::default(), |parameters, values| {
                            self.resolve_type_ref_in_scope(
                                ty,
                                parameters,
                                true,
                                Some(&owner),
                                &mut count,
                                None,
                                values,
                                &[],
                            )
                        });
                    if let Some(error) = count_error {
                        return Err(error);
                    }
                    substitution.insert_value(parameter.name.clone(), witness, carrier?);
                } else {
                    substitution.insert(parameter.name.clone(), witness);
                }
            }
        }
        Ok(RefinementScope {
            owner: owner.into(),
            substitution,
        })
    }

    /// Gather each declared obligation without replacing the refined type by
    /// its representation. Alias links use their exact semantic application.
    pub(super) fn refinements_of_annotation(
        &mut self,
        ty: &verum_ast::Type,
    ) -> CodegenResult<verum_common::List<FieldRefinementInfo>> {
        self.refinements_in_scope(ty, None)
    }

    pub(super) fn refinements_in_scope(
        &mut self,
        ty: &verum_ast::Type,
        scope: Option<RefinementScope>,
    ) -> CodegenResult<verum_common::List<FieldRefinementInfo>> {
        use verum_ast::ty::TypeKind;
        enum Cursor {
            Source(verum_ast::Type, Option<RefinementScope>),
            Witness(TypeRef),
        }
        let mut cursor = Cursor::Source(ty.clone(), scope);
        let mut obligations = verum_common::List::new();
        let mut visited = verum_common::Set::new();
        for _ in 0..crate::types::MAX_TYPE_REF_NODES {
            cursor = match cursor {
                Cursor::Source(annotation, scope) => match annotation.kind {
                    TypeKind::Refined { base, predicate } => {
                        let resolve = |cg: &mut Self| {
                            cg.resolve_signature_type_ref(&base, &cg.ctx.current_generic_param_ids)
                        };
                        let declared = match &scope {
                            Some(scope) => self.with_refinement_scope(scope, resolve)?,
                            None => resolve(self)?,
                        };
                        let representation = self
                            .expand_declared_alias(declared.clone())
                            .ok_or_else(|| {
                                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                                    "refinement base has no complete representation".into(),
                                ))
                            })?;
                        let representation_name = self.declaration_type_name(&representation, 0);
                        obligations.push(FieldRefinementInfo {
                            declaration_scope: scope.clone(),
                            base_declaration: Some(declared.clone()),
                            pred_expr: predicate.expr.clone(),
                            binding: predicate.binding.as_ref().map(|id| id.name.to_string()),
                            base_type_name: self
                                .declaration_type_name(&declared, 0)
                                .unwrap_or_else(|| Self::extract_type_name_from_ast(&base)),
                            base_var_type: representation_name
                                .as_deref()
                                .map(|name| self.type_name_to_var_type(name))
                                .unwrap_or(super::context::VarTypeKind::Unknown),
                        });
                        Cursor::Source(*base, scope)
                    }
                    TypeKind::Path(_) | TypeKind::Generic { .. } => {
                        let resolve = |cg: &mut Self| {
                            cg.resolve_signature_type_ref(
                                &annotation,
                                &cg.ctx.current_generic_param_ids,
                            )
                        };
                        let declared = match &scope {
                            Some(scope) => self.with_refinement_scope(scope, resolve)?,
                            None => resolve(self)?,
                        };
                        Cursor::Witness(declared)
                    }
                    _ => return Ok(obligations),
                },
                Cursor::Witness(witness) => {
                    let (id, arguments) = match &witness {
                        TypeRef::Concrete(id) => (*id, &[][..]),
                        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                        _ => return Ok(obligations),
                    };
                    let Some(declaration) = self.type_by_id(id) else {
                        return Ok(obligations);
                    };
                    if declaration.kind != crate::types::TypeKind::Alias {
                        return Ok(obligations);
                    }
                    if !visited.insert(witness.clone()) {
                        return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                            "cyclic refinement alias application".into(),
                        )));
                    }
                    let scope = self.nominal_refinement_scope(declaration, arguments)?;
                    if let Some(source) = &declaration.source_declaration {
                        let verum_ast::TypeDeclBody::Alias(target) = &source.body else {
                            return Err(CodegenError::new(
                                CodegenErrorKind::InvalidSourceSignature(
                                    "refinement alias source disagrees with its descriptor".into(),
                                ),
                            ));
                        };
                        Cursor::Source(target.clone(), Some(scope))
                    } else {
                        let target = declaration.alias_target.as_ref().ok_or_else(|| {
                            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                                "refinement alias has no declared target".into(),
                            ))
                        })?;
                        let target = TypeSubstitution::new(&declaration.type_params, arguments)
                            .apply(target)
                            .map_err(|error| {
                                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                                    error.to_string(),
                                ))
                            })?;
                        Cursor::Witness(target)
                    }
                }
            };
        }
        Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
            "refinement alias graph exceeds the semantic type resource limit".into(),
        )))
    }

    /// Select the field from the same applied owner used by the actual write.
    /// The retained declaration is authority for aliases, predicates and names;
    /// rendered type strings and reparsed predicate hints are not evidence.
    pub(super) fn emit_field_refinement_assert(
        &mut self,
        value: Reg,
        owner: &TypeRef,
        ordinal: u32,
    ) -> CodegenResult<()> {
        let invalid = |message: &str| {
            CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into()))
        };
        let (id, arguments) = match owner {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return Err(invalid("record field write has no nominal owner")),
        };
        let descriptor = self
            .type_by_id(id)
            .ok_or_else(|| invalid("record field write has no declared owner"))?;
        if descriptor.kind != crate::types::TypeKind::Record {
            return Err(invalid("record field write selected a non-record owner"));
        }
        let source = descriptor
            .source_declaration
            .as_ref()
            .ok_or_else(|| invalid("record field write requires its source declaration"))?;
        let slot = descriptor.fields.get(ordinal as usize)
            .ok_or_else(|| invalid("record field write has no storage slot"))?;
        let name = self.ctx.strings.get(slot.name.0 as usize)
            .ok_or_else(|| invalid("record field write has no declared name"))?;
        let mut scope = self.nominal_refinement_scope(descriptor, arguments)?;
        let annotation = match &source.body {
            verum_ast::TypeDeclBody::Record(fields) => {
                let field = fields.get(ordinal as usize)
                    .ok_or_else(|| invalid("record field write has no declared slot"))?;
                if name.as_str() != field.name.name.as_str() {
                    return Err(invalid("record field source disagrees with its storage slot"));
                }
                // Keep the same checked prefix as declaration lowering. A
                // previous Type-valued field is its exact owner projection,
                // never a free generic parameter from the current function.
                for (index, prior) in fields.iter().take(ordinal as usize).enumerate() {
                    if matches!(prior.ty.kind, verum_ast::ty::TypeKind::Universe {..}) {
                        scope.substitution.insert(prior.name.name.to_string(), ImplReceiverWitness {
                            ty: TypeRef::RecordFieldProjection { owner: id, ordinal: index as u32 },
                            semantic: true,
                        });
                    }
                }
                field.ty.clone()
            }
            verum_ast::TypeDeclBody::Tuple(fields) => fields.get(ordinal as usize)
                .ok_or_else(|| invalid("tuple constructor has no declared slot"))?.clone(),
            verum_ast::TypeDeclBody::Newtype(inner) if ordinal == 0 => inner.clone(),
            _ => return Err(invalid("record field source disagrees with its descriptor")),
        };
        let message = format!("refinement violation: field `{}.{name}`",
            self.type_declaration_identity(descriptor)
                .ok_or_else(|| invalid("record field owner has no declaration identity"))?);
        for obligation in self.refinements_in_scope(&annotation, Some(scope))? {
            self.emit_refinement_assert_on_reg(value, &obligation, &message)?;
        }
        Ok(())
    }

    /// Rebind only lexical names and type arguments. Do not reset allocation,
    /// emitted instructions, resource facts or the enclosing function identity.
    pub(super) fn with_refinement_scope<T>(
        &mut self,
        scope: &RefinementScope,
        lower: impl FnOnce(&mut Self) -> CodegenResult<T>,
    ) -> CodegenResult<T> {
        let restore_names = self.ctx.registers.suspend_named_bindings();
        let constant_scopes = std::mem::take(&mut self.lexical_constant_scopes);
        let active = std::mem::replace(&mut self.refinement_scope_active, true);
        let variables = std::mem::take(&mut self.ctx.variable_types);
        let constants = std::mem::take(&mut self.ctx.constant_types);
        let names = std::mem::take(&mut self.ctx.variable_type_names);
        let references = std::mem::take(&mut self.ctx.reference_bindings);
        let ids = std::mem::take(&mut self.ctx.current_generic_param_ids);
        let type_parameters = std::mem::take(&mut self.ctx.generic_type_params);
        let value_parameters = std::mem::take(&mut self.ctx.const_generic_params);
        let order = std::mem::take(&mut self.ctx.generic_type_params_ordered);
        let receiver = self.current_impl_receiver.take();
        let receiver_name = self.ctx.current_impl_type_name.take();
        let result = self.with_protocol_source_scope(
            Some(scope.owner.to_string()),
            scope.substitution.clone(),
            None,
            lower,
        );
        self.ctx.current_impl_type_name = receiver_name;
        self.current_impl_receiver = receiver;
        self.ctx.generic_type_params_ordered = order;
        self.ctx.const_generic_params = value_parameters;
        self.ctx.generic_type_params = type_parameters;
        self.ctx.current_generic_param_ids = ids;
        self.ctx.reference_bindings = references;
        self.ctx.variable_type_names = names;
        self.ctx.variable_types = variables;
        self.ctx.constant_types = constants;
        self.refinement_scope_active = active;
        self.lexical_constant_scopes = constant_scopes;
        restore_names(&mut self.ctx.registers);
        result
    }

    pub(super) fn emit_refinement_assert_on_reg(
        &mut self,
        value: Reg,
        info: &FieldRefinementInfo,
        message: &str,
    ) -> CodegenResult<()> {
        let lower = |codegen: &mut Self| {
            codegen.emit_refinement_assert_in_current_scope(
                value,
                &info.pred_expr,
                info.binding.as_deref(),
                info.base_var_type,
                &info.base_type_name,
                info.base_declaration.as_ref(),
                message,
            )
        };
        match &info.declaration_scope {
            Some(scope) => self.with_refinement_scope(scope, lower),
            None => lower(self),
        }
    }
}
