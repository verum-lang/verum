//! Protocol methods retain the exact variables of their declaration telescope.

use super::TypeChecker;
use crate::{
    Result, TypeError,
    context::TypeScheme,
    protocol::{ProtocolConstraints, ProtocolMethod, ProtocolMethodScope},
    ty::{Type, TypeVar},
};
use verum_ast::{
    FunctionDecl, GenericParam, Span,
    decl::FunctionParamKind,
    ty::{GenericParamKind, TypeBoundKind},
};
use verum_common::{List, Map, Text};

pub(super) struct AppliedProtocolMethod {
    pub owner: Text,
    pub arguments: List<Type>,
    pub method: ProtocolMethod,
}

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    pub(super) fn elaborate_protocol_method(
        &mut self,
        owner: &[GenericParam],
        constraints: &ProtocolConstraints,
        method: &FunctionDecl,
        has_default: bool,
    ) -> Result<ProtocolMethod> {
        let mut frame = indexmap::IndexMap::new();
        let mut scope = ProtocolMethodScope::default();
        for (position, parameter) in owner.iter().enumerate() {
            match &parameter.kind {
                GenericParamKind::Type { name, .. } => {
                    let variable = match constraints {
                        ProtocolConstraints::Declared { parameters, .. } => parameters[position],
                        _ => TypeVar::fresh(),
                    };
                    frame.insert(name.name.clone(), Type::Var(variable));
                    scope.binders.owner.push((position, variable));
                }
                GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } => {
                    frame.insert(name.name.clone(), self.source_value_parameter(parameter)?);
                    scope.owner_values.push((position, name.name.clone()));
                }
                _ => {
                    return Err(invalid(
                        parameter.span,
                        "protocol method owner parameter has no supported signature representation",
                    ));
                }
            }
        }
        let mut names = List::new();
        for parameter in &method.generics {
            match &parameter.kind {
                GenericParamKind::Type { name, .. } => {
                    let variable = TypeVar::fresh();
                    frame.insert(name.name.clone(), Type::Var(variable));
                    names.push(name.name.clone());
                    scope.binders.local.push(variable);
                }
                _ => {
                    return Err(invalid(
                        parameter.span,
                        "protocol method value, higher-kinded, level and context parameters need a quantified signature carrier",
                    ));
                }
            }
        }
        // Source declaration frames take precedence over flat context aliases.
        // A single frame also ensures that local parameters shadow owner names.
        self.push_decl_param_frame(frame);
        let saved_source_scope = std::mem::replace(&mut self.resolving_source_signature, true);
        let saved_self = self.current_self_type.clone();
        let receiver = Type::Named {
            path: verum_ast::Path::single(verum_ast::Ident::new("Self", method.span)),
            args: List::new(),
        };
        self.set_current_self_type(Some(receiver.clone()));
        let assumption_base = self.source_type_registration.projection_assumptions.len();
        let callable_base = self.source_type_registration.callable_assumptions.len();
        if let Some(bound) = self.source_type_registration.headers.last()
            .and_then(|header| header.self_bound(method.span))
        {
            self.source_type_registration.projection_assumptions.push(crate::protocol::WhereClause {
                ty: receiver, bounds: List::from_iter([bound]),
            });
        }
        if let ProtocolConstraints::Declared { parameter_bounds, .. } = constraints {
            self.source_type_registration.projection_assumptions.extend(parameter_bounds.iter().cloned());
        }
        let result = (|| -> Result<_> {
            let mut type_bounds: Map<Text, List<Type>> = Map::new();
            for (parameter, variable) in method.generics.iter().zip(&scope.binders.local) {
                let (name, bounds) = match &parameter.kind {
                    GenericParamKind::Type { name, bounds, .. }
                    | GenericParamKind::HigherKinded { name, bounds, .. }
                    | GenericParamKind::KindAnnotated { name, bounds, .. } => (name, bounds),
                    _ => unreachable!(),
                };
                let mut protocol_bounds = List::new();
                for bound in bounds {
                    match &bound.kind {
                        TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty)
                            if matches!(
                                ty.kind,
                                verum_ast::TypeKind::Function { .. }
                                    | verum_ast::TypeKind::Rank2Function { .. }
                            ) =>
                        {
                            let contract = self.ast_to_type(ty)?;
                            self.source_type_registration.callable_assumptions
                                .push((Type::Var(*variable), contract.clone()));
                            type_bounds
                                .entry(name.name.clone())
                                .or_insert_with(List::new)
                                .push(contract);
                        }
                        _ => protocol_bounds.extend(self.convert_type_bounds_for_receiver(
                            std::slice::from_ref(bound),
                            Some(&Type::Var(*variable)),
                        )?),
                    }
                }
                if !protocol_bounds.is_empty() {
                    self.source_type_registration.projection_assumptions.push(crate::protocol::WhereClause {
                        ty: Type::Var(*variable), bounds: protocol_bounds.clone(),
                    });
                    scope.protocol_bounds.insert(*variable, protocol_bounds);
                }
            }
            for clause in [
                method.generic_where_clause.as_ref(),
                method.meta_where_clause.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                for predicate in &clause.predicates {
                    let verum_ast::ty::WherePredicateKind::Type { ty, bounds } = &predicate.kind
                    else {
                        return Err(invalid(
                            predicate.span,
                            "method value predicates need a checked dependent call obligation",
                        ));
                    };
                    let subject = self.ast_to_type(ty)?;
                    let bounds = self.convert_type_bounds_for_receiver(bounds, Some(&subject))?;
                    let clause = crate::protocol::WhereClause { ty: subject, bounds };
                    self.source_type_registration.projection_assumptions.push(clause.clone());
                    scope.where_clauses.push(clause);
                }
            }
            let params = method
                .params
                .iter()
                .filter_map(|param| match &param.kind {
                    FunctionParamKind::Regular { ty, .. } => Some(self.ast_to_type(ty)),
                    _ => None,
                })
                .collect::<Result<List<_>>>()?;
            let output = method
                .return_type
                .as_ref()
                .map(|ty| self.ast_to_type(ty))
                .transpose()?
                .unwrap_or(Type::Unit);
            let output = self.wrap_return_type_for_sig_full(
                output,
                &method.throws_clause,
                method.is_async,
                method.is_generator,
            );
            let contexts = if method.contexts.is_empty() {
                None
            } else {
                self.ensure_using_types_registered(&method.contexts);
                let requirement = self
                    .context_resolver
                    .resolve_requirement(&method.contexts, method.span)
                    .map_err(|error| invalid(method.span, error.to_string()))?;
                Some(crate::di::requirement::ContextExpr::Concrete(requirement))
            };
            let ty = Type::Function {
                params,
                return_type: Box::new(output),
                contexts,
                type_params: List::new(),
                properties: None,
                property_requirement:
                    crate::computational_properties::PropertyRequirement::declared_function(
                        method.is_pure,
                        method.is_meta,
                    ),
            };
            let mut signature = ProtocolMethod::with_type_bounds(
                method.name.name.clone(),
                ty,
                has_default,
                names,
                type_bounds,
            );
            signature
                .set_dispatch(verum_ast::method_dispatch::MethodDispatch::from_declaration(method));
            signature.scope = Some(scope);
            Ok(signature)
        })();
        self.source_type_registration.projection_assumptions.truncate(assumption_base);
        self.source_type_registration.callable_assumptions.truncate(callable_base);
        self.set_current_self_type(saved_self);
        self.resolving_source_signature = saved_source_scope;
        self.pop_decl_param_frame();
        result
    }

    /// Instantiate owner arguments while preserving method-local quantifiers.
    pub(super) fn applied_protocol_method(
        &self,
        method: &ProtocolMethod,
        arguments: &[Type],
        receiver: &Type,
    ) -> Result<ProtocolMethod> {
        method
            .apply_owner(arguments, receiver)
            .map_err(TypeError::Other)
    }

    pub(super) fn inherited_protocol_method_scheme(
        &self,
        method: &ProtocolMethod,
        arguments: &[Type],
        receiver: &Type,
        impl_variables: &List<TypeVar>,
    ) -> Result<TypeScheme> {
        let applied = self.applied_protocol_method(method, arguments, receiver)?;
        let locals = applied
            .scope
            .as_ref()
            .map(|scope| scope.binders.local.clone())
            .unwrap_or_default();
        let mut variables = impl_variables.clone();
        variables.extend(locals.iter().copied());
        let mut scheme = TypeScheme::poly(variables, applied.ty);
        scheme.impl_var_count = impl_variables.len();
        scheme.impl_self_type = Some(receiver.clone());
        scheme.receiver_kind = method.receiver_kind();
        scheme.explicit_method_vars = Some(locals.iter()
            .map(|variable| Some(crate::context::GenericBinder::Type(*variable))).collect());
        for (name, variable) in applied.type_param_names.iter().zip(&locals) {
            if let Some(bound) = applied.type_param_bounds.get(name) {
                scheme.var_type_bounds.insert(*variable, bound.clone());
            }
        }
        if let Some(scope) = applied.scope {
            scheme.var_protocol_bounds = scope.protocol_bounds;
            scheme.where_clauses = scope.where_clauses;
        }
        Ok(scheme)
    }
    /// An explicit override still implements the declaration's conditional
    /// method contract. Rename local binders by its recorded source slots.
    pub(super) fn attach_protocol_method_requirements(
        &self,
        scheme: &mut TypeScheme,
        declaration: &ProtocolMethod,
        arguments: &[Type],
        receiver: &Type,
        span: Span,
    ) -> Result<()> {
        let applied = declaration
            .apply_owner(arguments, receiver)
            .map_err(|message| invalid(span, message))?;
        let Some(scope) = applied.scope else {
            return Ok(());
        };
        let slots = scheme.explicit_method_vars.as_ref().ok_or_else(|| {
            invalid(
                span,
                "method override lacks its declared local parameter slots",
            )
        })?;
        if slots.len() != scope.binders.local.len() {
            return Err(invalid(
                span,
                "method override changes the declared generic parameter roster",
            ));
        }
        let mut substitution = crate::ty::Substitution::new();
        for (original, slot) in scope.binders.local.iter().zip(slots) {
            let Some(crate::context::GenericBinder::Type(variable)) = slot.as_ref() else {
                return Err(invalid(
                    span,
                    "method override changes a type parameter's declared kind",
                ));
            };
            substitution.insert(*original, Type::Var(*variable));
        }
        for clause in &scope.where_clauses {
            scheme.where_clauses.push(
                clause
                    .try_apply_subst(&substitution)
                    .map_err(|message| invalid(span, message))?,
            );
        }
        for (original, bounds) in &scope.protocol_bounds {
            let Some(Type::Var(variable)) = substitution.get(original) else {
                return Err(invalid(
                    span,
                    "method bound lacks its declared local binder",
                ));
            };
            scheme
                .var_protocol_bounds
                .entry(*variable)
                .or_default()
                .extend(
                    bounds
                        .iter()
                        .map(|bound| bound.try_apply_subst(&substitution))
                        .collect::<std::result::Result<List<_>, _>>()
                        .map_err(|message| invalid(span, message))?,
                );
        }
        for (name, original) in applied.type_param_names.iter().zip(&scope.binders.local) {
            if let Some(bounds) = applied.type_param_bounds.get(name) {
                let Some(Type::Var(variable)) = substitution.get(original) else {
                    return Err(invalid(
                        span,
                        "method type bound lacks its declared local binder",
                    ));
                };
                scheme.var_type_bounds.entry(*variable).or_default().extend(
                    bounds
                        .iter()
                        .map(|bound| bound.try_apply_subst(&substitution))
                        .collect::<std::result::Result<List<_>, _>>()
                        .map_err(|message| invalid(span, message))?,
                );
            }
        }
        Ok(())
    }

    /// Resolve a method along declared super-protocol applications. Every edge
    /// transports the owner's argument roster before the next lookup.
    pub(super) fn applied_protocol_methods_in_hierarchy(
        &self,
        protocol_name: &Text,
        arguments: &[Type],
        receiver: &Type,
        owner_receiver: &Type,
        method_name: &Text,
        span: Span,
    ) -> Result<List<AppliedProtocolMethod>> {
        fn visit(
            checker: &TypeChecker,
            name: &Text,
            arguments: &[Type],
            receiver: &Type,
            owner_receiver: &Type,
            method: &Text,
            span: Span,
            active: &mut List<Text>,
        ) -> Result<List<AppliedProtocolMethod>> {
            if active.len() >= 128 || active.contains(name) {
                return Err(invalid(
                    span,
                    "cyclic or excessively deep protocol method hierarchy",
                ));
            }
            let protocol = checker
                .protocol_checker
                .read()
                .get_protocol(name)
                .cloned()
                .ok_or_else(|| {
                    invalid(
                        span,
                        format!("protocol method owner `{name}` is not registered"),
                    )
                })?;
            if let Some(method) = protocol.methods.get(method) {
                return Ok(List::from_iter([AppliedProtocolMethod {
                    owner: name.clone(),
                    arguments: arguments.iter().cloned().collect(),
                    method: method
                        .apply_owner(arguments, receiver)
                        .map_err(|message| invalid(span, message))?,
                }]));
            }
            if protocol.super_protocols.is_empty() {
                return Ok(List::new());
            }
            let mut substitution = crate::ty::Substitution::new();
            match &protocol.constraints {
                ProtocolConstraints::Declared {
                    self_type,
                    parameters,
                    ..
                } => {
                    if parameters.len() != arguments.len() {
                        return Err(invalid(
                            span,
                            "super-protocol application lacks its complete owner arguments",
                        ));
                    }
                    substitution.insert(*self_type, owner_receiver.clone());
                    for (variable, argument) in parameters.iter().zip(arguments) {
                        substitution.insert(*variable, argument.clone());
                    }
                }
                _ if !protocol.type_params.is_empty() => {
                    return Err(invalid(
                        span,
                        "super-protocol application lacks its declaration-owned telescope",
                    ));
                }
                _ => {}
            }
            active.push(name.clone());
            let mut methods: List<AppliedProtocolMethod> = List::new();
            for super_bound in &protocol.super_protocols {
                if super_bound.is_negative {
                    continue;
                }
                let bound = super_bound
                    .try_apply_subst(&substitution)
                    .map_err(|message| invalid(span, message))?;
                let owner = checker.path_to_string(&bound.protocol);
                for candidate in visit(
                    checker,
                    &owner,
                    &bound.args,
                    receiver,
                    owner_receiver,
                    method,
                    span,
                    active,
                )? {
                    // A diamond reaching the same declaration/application adds
                    // one obligation. Distinct declarations are never first-wins.
                    if !methods.iter().any(|prior| {
                        prior.owner == candidate.owner
                            && prior.arguments.len() == candidate.arguments.len()
                            && prior
                                .arguments
                                .iter()
                                .zip(&candidate.arguments)
                                .all(|(a, b)| {
                                    checker
                                        .protocol_checker
                                        .read()
                                        .types_structurally_equal(a, b)
                                })
                    }) {
                        methods.push(candidate);
                    }
                }
            }
            active.pop();
            Ok(methods)
        }
        visit(
            self,
            protocol_name,
            arguments,
            receiver,
            owner_receiver,
            method_name,
            span,
            &mut List::new(),
        )
    }

    pub(super) fn applied_protocol_method_in_hierarchy(
        &self,
        protocol_name: &Text,
        arguments: &[Type],
        receiver: &Type,
        method_name: &Text,
        span: Span,
    ) -> Result<Option<ProtocolMethod>> {
        let methods = self.applied_protocol_methods_in_hierarchy(
            protocol_name,
            arguments,
            receiver,
            receiver,
            method_name,
            span,
        )?;
        if methods.len() > 1 {
            return Err(invalid(
                span,
                "protocol method has competing inherited declaration owners",
            ));
        }
        Ok(methods.into_iter().next().map(|selected| selected.method))
    }

    /// Prepare a protocol candidate without granting its requirements. The
    /// selected candidate alone contributes obligations to the calling scope.
    pub(super) fn prepare_protocol_method_candidate(
        &self,
        protocol_name: &Text,
        owner_arguments: &[Type],
        receiver: &Type,
        method_name: &Text,
        span: Span,
    ) -> Result<Option<crate::context::InstantiatedScheme>> {
        let Some(method) = self.applied_protocol_method_in_hierarchy(
            protocol_name,
            owner_arguments,
            receiver,
            method_name,
            span,
        )?
        else {
            return Ok(None);
        };
        let scheme = self.inherited_protocol_method_scheme(&method, &[], receiver, &List::new())?;
        let mut instance = scheme
            .try_instantiate()
            .map_err(|message| invalid(span, message))?;
        instance.ty = self.instantiate_method_for_receiver(&instance.ty, receiver);
        Ok(Some(instance))
    }

    /// Freshen method-local quantifiers only. The owner's arguments may be
    /// rigid parameters of the caller and must retain their identity.
    pub(super) fn instantiate_applied_protocol_method(
        &mut self,
        method: &ProtocolMethod,
        receiver: &Type,
        arguments: &List<verum_ast::GenericArg>,
        span: Span,
    ) -> Result<Type> {
        let scheme = self.inherited_protocol_method_scheme(method, &[], receiver, &List::new())?;
        let instance = self.instantiate_value_instance(&scheme, span)?;
        for (variable, bounds) in &instance.type_bounds {
            for bound in bounds {
                self.register_type_var_type_bound(*variable, bound.clone());
            }
        }
        self.bind_explicit_method_arguments(
            instance.explicit_method_vars.as_ref(),
            arguments,
            &verum_ast::Ident::new(method.name.clone(), span),
            span,
        )?;
        Ok(self.instantiate_method_for_receiver(&instance.ty, receiver))
    }
}
