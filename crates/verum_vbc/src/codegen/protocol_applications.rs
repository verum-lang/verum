//! Retain protocol applications in the implementing declaration's binder space.
//! The protocol owns names in default signatures/bodies; the implementation
//! owns every substituted TypeRef and runtime witness slot.
use super::{
    CodegenError, CodegenErrorKind, CodegenResult, ImplReceiverWitness, ProtocolInfo, VbcCodegen,
    source_substitution::SourceTypeSubstitution,
};
use crate::types::{TypeId, TypeParamId, TypeRef};
use std::collections::HashMap;
use verum_ast::{
    Expr, ExprKind, Path, Type,
    ty::{GenericArg, GenericParam, GenericParamKind, TypeKind},
};

#[derive(Debug, Clone)]
pub(super) struct ProtocolApplication {
    pub(super) key: String,
    pub(super) name: String,
    pub(super) protocol: Option<crate::types::TypeId>,
    pub(super) substitution: SourceTypeSubstitution,
}

impl VbcCodegen {
    pub(super) fn protocol_application_parts(ty: &Type) -> CodegenResult<(&Path, &[GenericArg])> {
        match &ty.kind {
            TypeKind::Path(path) => Ok((path, &[])),
            TypeKind::Generic { base, args } => match &base.kind {
                TypeKind::Path(path) => Ok((path, args)),
                _ => Err(invalid(
                    "protocol application requires a declared protocol path",
                    ty.span,
                )),
            },
            _ => Err(invalid(
                "protocol application requires a declared protocol path",
                ty.span,
            )),
        }
    }

    /// Resolve the selected source identity, including mounts, before reading
    /// generic binders. A same-spelled protocol from another owner cannot supply
    /// a parameter roster or a default body.
    pub(super) fn resolve_protocol_application(
        &mut self,
        path: &Path,
        args: &[GenericArg],
        scope: &HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
        caller_parameters: &[GenericParam],
    ) -> CodegenResult<ProtocolApplication> {
        let written = Self::format_type_path(path);
        let selected = self.nominal_type_id(&written);
        let key = selected
            .and_then(|id| {
                self.type_by_id(id)
                    .and_then(|descriptor| self.type_declaration_identity(descriptor))
                    .map(|key| key.to_string())
                    .filter(|key| self.protocol_registry.contains_key(key))
                    .or_else(|| {
                        self.protocol_registry.iter().find_map(|(key, info)| {
                            let owner =
                                crate::module::qualify_module_name(&info.source_module, &info.name);
                            (key == &owner && self.type_name_to_id.get(key) == Some(&id))
                                .then(|| key.clone())
                        })
                    })
            })
            .unwrap_or_else(|| self.declared_dyn_path(path));
        let protocol = self.protocol_registry.get(&key).cloned().filter(|info| {
            key == crate::module::qualify_module_name(&info.source_module, &info.name)
        });
        let name = protocol
            .as_ref()
            .map(|info| info.name.clone())
            .unwrap_or_else(|| written.rsplit('.').next().unwrap_or(&written).to_owned());
        let substitution = match protocol {
            Some(info) => {
                if selected.is_none() {
                    return Err(invalid(
                        "inherited protocol application has no selected nominal identity",
                        path.span,
                    ));
                }
                self.resolve_protocol_arguments(
                    &info,
                    args,
                    scope,
                    declared_ids,
                    caller_parameters,
                    path.span,
                )?
            }
            None => {
                let has_arguments = args.iter().any(|argument| {
                    !matches!(argument, GenericArg::Lifetime(_) | GenericArg::Binding(_))
                });
                let declares_parameters =
                    selected
                        .and_then(|id| self.type_by_id(id))
                        .is_some_and(|descriptor| {
                            !descriptor.type_params.is_empty()
                                || descriptor.source_declaration.as_ref().is_some_and(
                                    |declaration| {
                                        declaration.generics.iter().any(|parameter| {
                                            !matches!(
                                                parameter.kind,
                                                GenericParamKind::Lifetime { .. }
                                            )
                                        })
                                    },
                                )
                        });
                if has_arguments || declares_parameters {
                    return Err(invalid(
                        &format!(
                            "protocol application '{key}' lacks its declaration-owned parameter header"
                        ),
                        path.span,
                    ));
                }
                SourceTypeSubstitution::default()
            }
        };
        Ok(ProtocolApplication {
            key,
            name,
            protocol: selected,
            substitution,
        })
    }

    fn resolve_protocol_arguments(
        &mut self,
        protocol: &ProtocolInfo,
        args: &[GenericArg],
        scope: &HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
        caller_parameters: &[GenericParam],
        span: verum_ast::Span,
    ) -> CodegenResult<SourceTypeSubstitution> {
        // Associated equalities are not positional type parameters. Lifetimes
        // have no runtime TypeRef slot and are erased consistently with the
        // common signature elaborator.
        let mut arguments = args
            .iter()
            .filter(|arg| !matches!(arg, GenericArg::Lifetime(_) | GenericArg::Binding(_)));
        let mut result = SourceTypeSubstitution::default();
        for parameter in protocol.generics.iter() {
            if matches!(parameter.kind, GenericParamKind::Lifetime { .. }) {
                continue;
            }
            let argument = arguments.next();
            let resolved = (|| {
                match &parameter.kind {
                    GenericParamKind::Type { name, default, .. } => {
                        let witness = if let Some(argument) = argument {
                            let ty = argument.type_for_declared_slot().ok_or_else(|| {
                                invalid(
                                    "protocol application argument does not denote a type",
                                    span,
                                )
                            })?;
                            self.reject_protocol_value_as_type(&ty, caller_parameters, scope)?;
                            self.resolve_protocol_argument(&ty, scope, declared_ids)?
                        } else if let Some(default) = default {
                            self.with_protocol_source_scope(
                                Some(protocol.source_module.clone()),
                                result.clone(),
                                None,
                                |cg| {
                                    cg.reject_protocol_value_as_type(
                                        default,
                                        &protocol.generics,
                                        &HashMap::new(),
                                    )?;
                                    cg.resolve_protocol_argument(
                                        default,
                                        &HashMap::new(),
                                        declared_ids,
                                    )
                                },
                            )?
                        } else {
                            return Err(invalid(
                                "protocol application is missing a type argument",
                                span,
                            ));
                        };
                        result.insert(name.name.to_string(), witness);
                    }
                    GenericParamKind::Meta { name, ty, .. }
                    | GenericParamKind::Const { name, ty } => {
                        let value_type = self.with_protocol_source_scope(
                            Some(protocol.source_module.clone()),
                            result.clone(),
                            None,
                            |cg| {
                                cg.protocol_value_carrier(
                                    parameter,
                                    ty,
                                    &HashMap::new(),
                                    declared_ids,
                                )
                            },
                        )?;
                        let argument = argument.ok_or_else(|| {
                            invalid("protocol application is missing a value argument", span)
                        })?;
                        let witness = self.resolve_protocol_value_argument(
                            argument,
                            scope,
                            declared_ids,
                            caller_parameters,
                            &[],
                            &value_type,
                            span,
                        )?;
                        result.insert_value(name.name.to_string(), witness, value_type);
                    }
                    _ => {
                        return Err(invalid(
                            "protocol application parameter has no source substitution representation",
                            parameter.span,
                        ));
                    }
                }
                Ok(())
            })();
            resolved.map_err(|mut error: CodegenError| {
                let name = parameter_name(parameter).unwrap_or("<unsupported>");
                error.context = Some(format!(
                    "protocol application '{}.{}', parameter '{}', declaration {:?}; {}",
                    protocol.source_module,
                    protocol.name,
                    name,
                    parameter.span,
                    error
                        .context
                        .as_deref()
                        .unwrap_or("argument checked at application span")
                ));
                error
            })?;
        }
        if arguments.next().is_some() {
            return Err(invalid(
                &format!(
                    "protocol application '{}' has too many positional arguments",
                    crate::module::qualify_module_name(&protocol.source_module, &protocol.name)
                ),
                span,
            ));
        }
        Ok(result)
    }

    /// Walk type positions, preserving the kind of nominal argument slots and
    /// locally quantified shadowing. A value is legal inside Sized<N>, but not
    /// inside (N, Int), &N, or fn() -> N.
    pub(super) fn reject_protocol_value_as_type(
        &mut self,
        ty: &Type,
        parameters: &[GenericParam],
        scope: &HashMap<String, u16>,
    ) -> CodegenResult<()> {
        use verum_ast::visitor::{Visitor, walk_type};
        struct Positions<'a> {
            cg: &'a mut VbcCodegen,
            parameters: &'a [GenericParam],
            scope: &'a HashMap<String, u16>,
            locals: Vec<String>,
            error: Option<CodegenError>,
            depth: usize,
            budget: usize,
        }
        impl Positions<'_> {
            fn value_name(&self, name: &str) -> bool {
                !self.locals.iter().any(|local| local == name)
                    && (self
                        .cg
                        .current_source_substitution
                        .get_value(name)
                        .is_some()
                        || self.parameters.iter().any(|parameter| {
                            parameter_name(parameter) == Some(name)
                                && matches!(
                                    parameter.kind,
                                    GenericParamKind::Meta { .. } | GenericParamKind::Const { .. }
                                )
                        }))
            }
        }
        impl Visitor for Positions<'_> {
            fn visit_expr(&mut self, _: &Expr) {} // Array extents/refinement predicates are value positions.
            fn visit_type(&mut self, ty: &Type) {
                if self.error.is_some() {
                    return;
                }
                if self.depth >= 128 || self.budget == 0 {
                    self.error = Some(invalid(
                        "protocol type argument exceeds its structural limit",
                        ty.span,
                    ));
                    return;
                }
                self.budget -= 1;
                self.depth += 1;
                match &ty.kind {
                    TypeKind::Path(path)
                        if path
                            .as_ident()
                            .is_some_and(|name| self.value_name(name.name.as_str())) =>
                    {
                        self.error = Some(invalid(
                            "protocol type argument names a value parameter",
                            ty.span,
                        ));
                    }
                    TypeKind::Generic { base, args } => {
                        self.visit_type(base);
                        let roster = self
                            .cg
                            .resolve_signature_type_ref(base, self.scope)
                            .ok()
                            .and_then(|base| {
                                if let TypeRef::Concrete(id) = base {
                                    self.cg
                                        .declaration_generic_roster(id)
                                        .map(|roster| (id, roster.into_owned()))
                                } else {
                                    None
                                }
                            });
                        let Some((nominal, roster)) = roster else {
                            self.error = Some(invalid(
                                "protocol type argument has no declaration-owned parameter roster",
                                base.span,
                            ));
                            self.depth -= 1;
                            return;
                        };
                        for (index, argument) in args
                            .iter()
                            .filter(|arg| !matches!(arg, GenericArg::Lifetime(_)))
                            .enumerate()
                        {
                            let Some(parameter) = roster.get(index) else {
                                self.error = Some(invalid(
                                    "protocol type argument exceeds the nominal parameter roster",
                                    ty.span,
                                ));
                                break;
                            };
                            if let Some(declaration) = parameter.value_parameter() {
                                if let Err(error) = self.cg.validate_nested_protocol_value(
                                    nominal,
                                    declaration,
                                    argument,
                                    self.scope,
                                    self.parameters,
                                    &self.locals,
                                    ty.span,
                                ) {
                                    self.error = Some(error);
                                }
                                continue;
                            }
                            if let Some(argument) = argument.type_for_declared_slot() {
                                self.visit_type(&argument);
                            } else {
                                self.error = Some(invalid(
                                    "protocol type argument supplies a value to a type slot",
                                    ty.span,
                                ));
                            }
                        }
                    }
                    TypeKind::Rank2Function {
                        type_params,
                        params,
                        return_type,
                        ..
                    } => {
                        let outer = self.locals.len();
                        self.locals.extend(
                            type_params
                                .iter()
                                .filter_map(parameter_name)
                                .map(str::to_owned),
                        );
                        for parameter in params {
                            self.visit_type(parameter);
                        }
                        self.visit_type(return_type);
                        self.locals.truncate(outer);
                    }
                    _ => walk_type(self, ty),
                }
                self.depth -= 1;
            }
        }
        let mut positions = Positions {
            cg: self,
            parameters,
            scope,
            locals: vec![],
            error: None,
            depth: 0,
            budget: 4096,
        };
        positions.visit_type(ty);
        positions.error.map_or(Ok(()), Err)
    }

    fn validate_nested_protocol_value(
        &mut self,
        nominal: TypeId,
        parameter: &GenericParam,
        argument: &GenericArg,
        scope: &HashMap<String, u16>,
        parameters: &[GenericParam],
        local_types: &[String],
        span: verum_ast::Span,
    ) -> CodegenResult<()> {
        let descriptor = self
            .type_by_id(nominal)
            .ok_or_else(|| invalid("protocol value slot lacks its nominal declaration", span))?;
        let owner = self
            .type_declaration_identity(descriptor)
            .and_then(|name| name.rsplit_once('.').map(|(owner, _)| owner.to_string()));
        let (GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. }) =
            &parameter.kind
        else {
            return Err(invalid(
                "protocol nominal argument kind disagrees with its source declaration",
                span,
            ));
        };
        let declared_ids = Self::ordered_generic_param_ids(scope, &Default::default());
        let carrier = self.with_protocol_source_scope(
            owner,
            SourceTypeSubstitution::default(),
            None,
            |cg| cg.protocol_value_carrier(parameter, ty, &HashMap::new(), &[]),
        )?;
        self.resolve_protocol_value_argument(
            argument,
            scope,
            &declared_ids,
            parameters,
            local_types,
            &carrier,
            span,
        )?;
        Ok(())
    }

    /// ConstValue has one wire interpretation: a checked signed 64-bit integer.
    /// Other scalar carriers and refinements need their own typed evidence; an
    /// integer payload or an erased ABI type cannot establish that evidence.
    fn protocol_value_carrier(
        &self,
        parameter: &GenericParam,
        ty: &Type,
        scope: &HashMap<String, u16>,
        declared_ids: &[TypeParamId],
    ) -> CodegenResult<TypeRef> {
        if matches!(ty.kind, TypeKind::Refined { .. })
            || matches!(&parameter.kind, GenericParamKind::Meta { refinement, .. } if refinement.is_some())
        {
            return Err(invalid(
                "refined protocol value parameters require a checked refinement witness",
                parameter.span,
            ));
        }
        let witness = self.resolve_protocol_argument(ty, scope, declared_ids)?;
        if !witness.semantic || witness.ty != TypeRef::Concrete(TypeId::INT) {
            return Err(invalid(
                "protocol value parameter carrier has no exact VBC scalar representation",
                ty.span,
            ));
        }
        Ok(witness.ty)
    }

    fn protocol_named_value(
        &self,
        name: &str,
        scope: &HashMap<String, u16>,
        declared_ids: &[TypeParamId],
        parameters: &[GenericParam],
        local_types: &[String],
        expected: &TypeRef,
        span: verum_ast::Span,
    ) -> CodegenResult<ImplReceiverWitness> {
        if local_types.iter().any(|local| local == name) {
            return Err(invalid(
                "protocol value argument names a locally quantified type",
                span,
            ));
        }
        if let Some((witness, actual)) = self.current_source_substitution.get_value(name) {
            if actual == expected && witness.semantic {
                return Ok(witness.clone());
            }
            return Err(invalid(
                "protocol value argument has an incompatible declared carrier",
                span,
            ));
        }
        // A dense slot is an address, not proof that a name denotes a value.
        // Only the caller's declaration can establish its kind and carrier.
        if self.current_source_substitution.get(name).is_none()
            && let Some(parameter) = parameters.iter().find(|p| parameter_name(p) == Some(name))
            && let GenericParamKind::Meta { ty, .. } | GenericParamKind::Const { ty, .. } =
                &parameter.kind
        {
            let actual = self.protocol_value_carrier(parameter, ty, scope, declared_ids)?;
            if &actual == expected
                && let Some(&id) = scope.get(name)
            {
                return Ok(ImplReceiverWitness {
                    ty: TypeRef::Generic(TypeParamId(id)),
                    semantic: true,
                });
            }
        }
        Err(invalid(
            "protocol value argument lacks a declaration-owned value witness",
            span,
        ))
    }

    fn resolve_protocol_value_argument(
        &self,
        argument: &GenericArg,
        scope: &HashMap<String, u16>,
        declared_ids: &[TypeParamId],
        parameters: &[GenericParam],
        local_types: &[String],
        expected: &TypeRef,
        span: verum_ast::Span,
    ) -> CodegenResult<ImplReceiverWitness> {
        let expression = match argument {
            GenericArg::Type(ty) => {
                if let TypeKind::Path(path) = &ty.kind
                    && let Some(name) = path.as_ident()
                {
                    return self.protocol_named_value(
                        name.name.as_str(),
                        scope,
                        declared_ids,
                        parameters,
                        local_types,
                        expected,
                        ty.span,
                    );
                }
                return Err(invalid(
                    "protocol value argument does not denote a value",
                    ty.span,
                ));
            }
            GenericArg::Const(expression) => expression,
            _ => {
                return Err(invalid(
                    "protocol value argument does not denote a value",
                    span,
                ));
            }
        };
        let expression = Self::normalized_place(expression);
        if let ExprKind::Path(path) = &expression.kind
            && let Some(name) = path.as_ident()
        {
            return self.protocol_named_value(
                name.name.as_str(),
                scope,
                declared_ids,
                parameters,
                local_types,
                expected,
                expression.span,
            );
        }
        let value =
            super::source_substitution::checked_scalar_expression(expression, &mut |leaf| {
                let ExprKind::Path(path) = &leaf.kind else {
                    return Ok(None);
                };
                let Some(name) = path.as_ident() else {
                    return Ok(None);
                };
                let witness = self.protocol_named_value(
                    name.name.as_str(),
                    scope,
                    declared_ids,
                    parameters,
                    local_types,
                    expected,
                    leaf.span,
                )?;
                Ok(match witness.ty {
                    TypeRef::ConstValue(value) => Some(value),
                    _ => None,
                })
            })?;
        Ok(ImplReceiverWitness {
            ty: TypeRef::ConstValue(value),
            semantic: true,
        })
    }

    fn resolve_protocol_argument(
        &self,
        ty: &Type,
        scope: &HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
    ) -> CodegenResult<ImplReceiverWitness> {
        let resolved = self.resolve_signature_type_ref(ty, scope)?;
        let semantic =
            self.semantic_formal_type(ty, scope, declared_ids).as_ref() == Some(&resolved);
        Ok(ImplReceiverWitness {
            ty: resolved,
            semantic,
        })
    }

    /// Application arguments are resolved in their declaration's lexical scope,
    /// under the already resolved parent application or blanket receiver. All
    /// substitutions are simultaneous and restored before any error propagates.
    pub(super) fn with_protocol_source_scope<T>(
        &mut self,
        owner: Option<String>,
        substitution: SourceTypeSubstitution,
        receiver: Option<ImplReceiverWitness>,
        lower: impl FnOnce(&mut Self) -> CodegenResult<T>,
    ) -> CodegenResult<T> {
        let previous_owner = std::mem::replace(&mut self.ctx.current_source_module, owner.clone());
        let previous_lexical = std::mem::replace(&mut self.default_body_type_scope, owner);
        let previous_substitution =
            std::mem::replace(&mut self.current_source_substitution, substitution);
        let previous_receiver =
            receiver.map(|receiver| self.current_impl_receiver.replace(receiver));
        let result = lower(self);
        self.ctx.current_source_module = previous_owner;
        self.default_body_type_scope = previous_lexical;
        self.current_source_substitution = previous_substitution;
        if let Some(previous) = previous_receiver {
            self.current_impl_receiver = previous;
        }
        result
    }
}

fn parameter_name(parameter: &GenericParam) -> Option<&str> {
    match &parameter.kind {
        GenericParamKind::Type { name, .. }
        | GenericParamKind::Const { name, .. }
        | GenericParamKind::Meta { name, .. }
        | GenericParamKind::Context { name }
        | GenericParamKind::HigherKinded { name, .. }
        | GenericParamKind::Level { name }
        | GenericParamKind::KindAnnotated { name, .. } => Some(name.name.as_str()),
        GenericParamKind::Lifetime { .. } => None,
    }
}

fn invalid(message: &str, span: verum_ast::Span) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into()),
        span,
    )
}
