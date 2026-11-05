//! Check associated families in their declaring telescope, using rigid goals.
use super::TypeChecker;
use crate::{
    Result, TypeError,
    context::GenericBinder,
    protocol::{AssociatedFamilyBinders, ProtocolBound, ProtocolConstraints, WhereClause},
    ty::{Substitution, Type, TypeVar},
};
use verum_ast::{
    Span,
    decl::{ImplDecl, ImplItemKind, ImplKind},
    ty::{GenericArg, GenericParamKind, WherePredicateKind},
};
use verum_common::{List, Map, Text};

impl TypeChecker {
    pub(super) fn check_associated_type_obligations(
        &mut self,
        declaration: &ImplDecl,
        identity: &Text,
    ) -> Result<()> {
        let (constraints, associated_types, super_protocols, has_parameter_bounds) = {
            let registry = self.protocol_checker.read();
            let protocol = registry
                .get_protocol(identity)
                .ok_or_else(|| refusal(declaration.span, "protocol declaration is unavailable"))?;
            (
                protocol.constraints.clone(),
                protocol.associated_types.clone(),
                protocol.super_protocols.clone(),
                protocol
                    .type_params
                    .iter()
                    .any(|parameter| !parameter.bounds.is_empty()),
            )
        };
        let mut provided = Map::new();
        for item in &declaration.items {
            if let ImplItemKind::Type {
                name,
                type_params,
                ty,
            } = &item.kind
            {
                let Some(associated) = associated_types.get(&name.name) else {
                    return Err(refusal(
                        item.span,
                        format!(
                            "`{identity}` does not declare associated type `{}`",
                            name.name
                        ),
                    ));
                };
                if associated.type_params.len() != type_params.len() {
                    return Err(refusal(
                        item.span,
                        format!(
                            "associated type `{}` has {} parameters; `{identity}` declares {}",
                            name.name,
                            type_params.len(),
                            associated.type_params.len()
                        ),
                    ));
                }
                if provided
                    .insert(name.name.clone(), (type_params, ty, item.span))
                    .is_some()
                {
                    return Err(refusal(
                        item.span,
                        "associated type is implemented more than once",
                    ));
                }
            }
        }
        for (name, associated) in &associated_types {
            if !provided.contains_key(name) && associated.default.is_none() {
                return Err(refusal(
                    declaration.span,
                    format!(
                        "implementation of `{identity}` omits required associated type `{name}`"
                    ),
                ));
            }
        }
        let has_associated_bounds = associated_types
            .values()
            .any(|associated| !associated.bounds.is_empty());
        // A local requirement narrows the family's domain even if its result
        // has no bounds. Prove it from the declared domain using the same rigid
        // variables and implication checker as result obligations.
        let needs_domain_check = |name: &Text| {
            associated_types.get(name).is_some_and(|associated| {
                associated
                    .binders
                    .local
                    .iter()
                    .any(|binder| matches!(binder, GenericBinder::Value(_)))
            }) || provided.get(name).is_some_and(|(parameters, _, _)| {
                parameters.iter().any(|parameter| match &parameter.kind {
                    GenericParamKind::Type { bounds, .. } => !bounds.is_empty(),
                    _ => true,
                })
            })
        };
        let has_family_domains = associated_types.keys().any(needs_domain_check);
        if let ProtocolConstraints::Unsupported(reason) = &constraints {
            return Err(refusal(declaration.span, reason.clone()));
        }
        let ProtocolConstraints::Declared {
            self_type: self_variable,
            parameters: owner_variables,
            defaults,
            parameter_bounds,
        } = &constraints
        else {
            return if has_associated_bounds
                || has_family_domains
                || has_parameter_bounds
                || !super_protocols.is_empty()
            {
                Err(refusal(
                    declaration.span,
                    "protocol obligations lack their declaration-owned telescope",
                ))
            } else {
                Ok(())
            };
        };
        if !has_associated_bounds
            && !has_family_domains
            && super_protocols.is_empty()
            && parameter_bounds
                .iter()
                .all(|clause| clause.bounds.is_empty())
        {
            return Ok(());
        }
        let ImplKind::Protocol {
            protocol_args,
            for_type,
            ..
        } = &declaration.kind
        else {
            unreachable!()
        };
        let (frame, _) = self.associated_parameter_frame(&declaration.generics)?;
        self.push_decl_param_frame(frame.clone());
        let previous_self = self.current_self_type.clone();
        let result = (|| {
            let self_type = self.ast_to_type(for_type)?;
            self.set_current_self_type(Some(self_type.clone()));
            let mut substitution = Substitution::new();
            substitution.insert(*self_variable, self_type);
            for (position, variable) in owner_variables.iter().enumerate() {
                let argument = match protocol_args.get(position) {
                    Some(GenericArg::Type(ty)) => self.ast_to_type(ty)?,
                    Some(_) => {
                        return Err(refusal(
                            declaration.span,
                            "protocol type argument is not represented as a type",
                        ));
                    }
                    None => defaults
                        .get(position)
                        .and_then(|default| default.as_ref())
                        .ok_or_else(|| {
                            refusal(declaration.span, "protocol type argument is missing")
                        })?
                        .try_apply_subst(&substitution)
                        .map_err(|error| refusal(declaration.span, error))?,
                };
                substitution.insert(*variable, argument);
            }
            if protocol_args.len() > owner_variables.len() {
                return Err(refusal(
                    declaration.span,
                    "too many protocol type arguments",
                ));
            }
            let mut assumptions = List::new();
            for parameter in &declaration.generics {
                let GenericParamKind::Type { name, bounds, .. } = &parameter.kind else {
                    // Value domains travel in their Type::Meta declaration binder.
                    continue;
                };
                assumptions.push(WhereClause {
                    ty: frame[&name.name].clone(),
                    bounds: self.elaborate_family_bounds(bounds)?,
                });
            }
            if let Some(clause) = &declaration.generic_where_clause {
                for predicate in &clause.predicates {
                    let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                        return Err(refusal(
                            predicate.span,
                            "universal associated obligations require typed where constraints",
                        ));
                    };
                    assumptions.push(WhereClause {
                        ty: self.ast_to_type(ty)?,
                        bounds: self.elaborate_family_bounds(bounds)?,
                    });
                }
            }
            for clause in parameter_bounds {
                let ty = clause
                    .ty
                    .try_apply_subst(&substitution)
                    .map_err(|error| refusal(declaration.span, error))?;
                let bounds: List<_> = clause
                    .bounds
                    .iter()
                    .map(|bound| bound.try_apply_subst(&substitution))
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|error| refusal(declaration.span, error))?;
                self.prove_associated_bounds(&ty, &bounds, &assumptions, declaration.span)?;
            }
            let mut values = Map::new();
            for (name, (parameters, ty, _)) in &provided {
                if parameters.is_empty() {
                    values.insert(name.clone(), self.ast_to_type(ty)?);
                }
            }
            for (name, associated) in &associated_types {
                if associated.bounds.is_empty() && !needs_domain_check(name) {
                    continue;
                }
                let mut family_frame = frame.clone();
                let mut family_substitution = substitution.clone();
                let mut family_assumptions = assumptions.clone();
                let provided = provided.get(name);
                // Elaborate implementation domains before any local shadows its
                // own carrier annotation. The retained identities are then bound
                // to the declaration's rigid arguments in declaration order.
                let implementation_parameters = match provided {
                    Some((parameters, _, _)) => self.associated_parameter_frame(parameters)?.1,
                    None => List::new(),
                };
                let mut implementation_scope = Substitution::new();
                let mut family_arguments = List::new();
                for (index, parameter) in associated.type_params.iter().enumerate() {
                    let source_binder = associated.binders.local.get(index).ok_or_else(|| {
                        refusal(
                            declaration.span,
                            "family parameter lacks its declaration identity",
                        )
                    })?;
                    let argument = match source_binder {
                        GenericBinder::Type(_) => Type::Var(TypeVar::fresh()),
                        GenericBinder::Value(value) => {
                            let Type::Meta {
                                name,
                                ty,
                                refinement,
                                ..
                            } = value
                                .try_apply_subst(&family_substitution)
                                .map_err(|message| refusal(declaration.span, message))?
                            else {
                                return Err(refusal(
                                    declaration.span,
                                    "family value binder lost its domain",
                                ));
                            };
                            Type::meta(name, *ty, refinement)
                        }
                        _ => {
                            return Err(refusal(
                                declaration.span,
                                "universal family constraints require represented kind evidence",
                            ));
                        }
                    };
                    AssociatedFamilyBinders::bind_argument(
                        source_binder,
                        &argument,
                        &mut family_substitution,
                    )
                    .map_err(|message| refusal(declaration.span, message))?;
                    let local_name = if let Some((parameters, _, _)) = provided {
                        match (&parameters[index].kind, source_binder) {
                            (GenericParamKind::Type { name, .. }, GenericBinder::Type(_)) => {
                                &name.name
                            }
                            (
                                GenericParamKind::Meta { name, .. }
                                | GenericParamKind::Const { name, .. },
                                GenericBinder::Value(_),
                            ) => &name.name,
                            _ => {
                                return Err(refusal(
                                    parameters[index].span,
                                    "associated implementation parameter role differs from its declaration",
                                ));
                            }
                        }
                    } else {
                        &parameter.name
                    };
                    if let Some(parameter) = implementation_parameters.get(index) {
                        AssociatedFamilyBinders::bind_argument_normalized(
                            parameter,
                            &argument,
                            &mut implementation_scope,
                            |ty| self.normalize_type(ty),
                        )
                        .map_err(|message| refusal(declaration.span, message))?;
                    }
                    family_frame.insert(local_name.clone(), argument.clone());
                    family_arguments.push(argument);
                }
                if matches!(
                    associated.kind,
                    crate::advanced_protocols::AssociatedTypeKind::HigherKinded { .. }
                ) {
                    return Err(refusal(
                        declaration.span,
                        "universal family constraints require declared kind evidence",
                    ));
                }
                for (parameter, argument) in associated.type_params.iter().zip(&family_arguments) {
                    let bounds = parameter
                        .bounds
                        .iter()
                        .map(|bound| bound.try_apply_subst(&family_substitution))
                        .collect::<std::result::Result<List<_>, _>>()
                        .map_err(|error| refusal(declaration.span, error))?;
                    family_assumptions.push(WhereClause {
                        ty: argument.clone(),
                        bounds,
                    });
                }
                for clause in &associated.where_clauses {
                    let index = associated
                        .type_params
                        .iter()
                        .position(|parameter| parameter.name == clause.param)
                        .ok_or_else(|| {
                            refusal(
                                clause.span,
                                "family where clause has no declaring parameter",
                            )
                        })?;
                    let bounds = clause
                        .constraints
                        .iter()
                        .map(|bound| bound.try_apply_subst(&family_substitution))
                        .collect::<std::result::Result<List<_>, _>>()
                        .map_err(|error| refusal(clause.span, error))?;
                    family_assumptions.push(WhereClause {
                        ty: family_arguments[index].clone(),
                        bounds,
                    });
                }
                self.push_decl_param_frame(family_frame);
                let checked = (|| {
                    let (actual, span) = if let Some((parameters, ty, span)) = provided {
                        for (parameter, argument) in parameters.iter().zip(&family_arguments) {
                            match &parameter.kind {
                                GenericParamKind::Type { bounds, .. } => {
                                    let required = self.elaborate_family_bounds(bounds)?;
                                    self.prove_associated_bounds(
                                        argument,
                                        &required,
                                        &family_assumptions,
                                        *span,
                                    )?;
                                }
                                GenericParamKind::Meta { .. } | GenericParamKind::Const { .. } => {
                                    // The complete value domain was checked in its
                                    // prefix lexical scope before installing locals.
                                }
                                _ => {
                                    return Err(refusal(
                                        parameter.span,
                                        "associated implementation parameter lacks represented kind evidence",
                                    ));
                                }
                            }
                        }
                        (self.ast_to_type(ty)?, *span)
                    } else {
                        (
                            associated
                                .default
                                .as_ref()
                                .unwrap()
                                .try_apply_subst(&family_substitution)
                                .map_err(|error| refusal(declaration.span, error))?,
                            declaration.span,
                        )
                    };
                    let bounds: List<_> = associated
                        .bounds
                        .iter()
                        .map(|bound| {
                            instantiate_bound(
                                bound,
                                associated.binders.self_type,
                                &values,
                                &family_substitution,
                            )
                        })
                        .collect::<std::result::Result<_, _>>()
                        .map_err(|error| refusal(span, error))?;
                    self.prove_associated_bounds(&actual, &bounds, &family_assumptions, span)
                })();
                self.pop_decl_param_frame();
                checked?;
            }
            Ok(())
        })();
        self.pop_decl_param_frame();
        self.set_current_self_type(previous_self);
        result
    }

    fn prove_associated_bounds(
        &mut self,
        ty: &Type,
        bounds: &[ProtocolBound],
        assumptions: &[WhereClause],
        span: Span,
    ) -> Result<()> {
        let mut pending: Vec<_> = bounds.iter().map(|bound| (ty.clone(), bound.clone())).collect();
        let mut seen: Vec<(Type, ProtocolBound)> = Vec::new();
        while let Some((subject, bound)) = pending.pop() {
            if seen.iter().any(|(prior, requirement)| prior == &subject && requirement == &bound) {
                continue;
            }
            if seen.len() == 4096 {
                return Err(refusal(span, "source protocol evidence budget exhausted"));
            }
            seen.push((subject.clone(), bound.clone()));
            let name = self.path_to_string(&bound.protocol);
            self.ensure_stdlib_type_loaded_transitive(&name);
            self.load_source_bound_implementations(&subject, &bound, assumptions, span)?;
            let registry = self.protocol_checker.read();
            // A declaration assumption can already prove this goal. Otherwise
            // follow only the actual matched implementation's owned telescope,
            // never guessed protocols for a nominal argument's spelling.
            if registry.check_universal_bounds(&subject, std::slice::from_ref(&bound), assumptions).is_ok() {
                continue;
            }
            let requirements = registry.source_bound_requirements(&subject, &bound)
                .map_err(|error| refusal(span, error))?;
            for clause in requirements {
                for required in clause.bounds {
                    pending.push((clause.ty.clone(), required));
                }
            }
        }
        self.protocol_checker
            .read()
            .check_universal_bounds(ty, bounds, assumptions)
            .map_err(|error| refusal(span, error))
    }
}

fn refusal(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

fn instantiate_bound(
    bound: &ProtocolBound,
    self_var: Option<TypeVar>,
    values: &Map<Text, Type>,
    substitution: &Substitution,
) -> std::result::Result<ProtocolBound, Text> {
    Ok(ProtocolBound {
        protocol: bound.protocol.clone(),
        is_negative: bound.is_negative,
        args: bound
            .args
            .iter()
            .map(|ty| instantiate_type(ty, self_var, values, substitution, 0))
            .collect::<std::result::Result<_, _>>()?,
        associated_types: bound
            .associated_types
            .iter()
            .map(|(name, ty)| {
                Ok((
                    name.clone(),
                    instantiate_type(ty, self_var, values, substitution, 0)?,
                ))
            })
            .collect::<std::result::Result<_, Text>>()?,
    })
}

/// Only a projection from this declaration's Self binder may consult its items.
fn instantiate_type(
    ty: &Type,
    self_var: Option<TypeVar>,
    values: &Map<Text, Type>,
    substitution: &Substitution,
    depth: usize,
) -> std::result::Result<Type, Text> {
    if depth > 64 {
        return Err("associated projection substitution exceeds its depth budget".into());
    }
    let recurse = |ty: &Type| instantiate_type(ty, self_var, values, substitution, depth + 1);
    match ty {
        Type::Generic { name, args }
            if name.starts_with("::")
                && args.len() == 1
                && self_var.is_some_and(|var| args[0] == Type::Var(var)) =>
        {
            values.get(&Text::from(&name[2..])).cloned().ok_or_else(|| {
                format!(
                    "Self projection `{}` lacks a concrete declaration in this implementation",
                    &name[2..]
                )
                .into()
            })
        }
        Type::Generic { name, args } => Ok(Type::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(recurse)
                .collect::<std::result::Result<_, _>>()?,
        }),
        Type::Named { path, args } => Ok(Type::Named {
            path: path.clone(),
            args: args
                .iter()
                .map(recurse)
                .collect::<std::result::Result<_, _>>()?,
        }),
        Type::Tuple(types) => Ok(Type::Tuple(
            types
                .iter()
                .map(recurse)
                .collect::<std::result::Result<_, _>>()?,
        )),
        Type::Reference { mutable, inner } => Ok(Type::Reference {
            mutable: *mutable,
            inner: Box::new(recurse(inner)?),
        }),
        Type::Slice { element } => Ok(Type::Slice {
            element: Box::new(recurse(element)?),
        }),
        Type::Array { element, size } => Ok(Type::Array {
            element: Box::new(recurse(element)?),
            size: size.clone(),
        }),
        Type::TypeApp { constructor, args } => Ok(Type::TypeApp {
            constructor: Box::new(recurse(constructor)?),
            args: args
                .iter()
                .map(recurse)
                .collect::<std::result::Result<_, _>>()?,
        }),
        Type::Var(_) => ty.try_apply_subst(substitution),
        _ if self_var.is_some_and(|var| ty.free_vars().contains(&var)) => {
            Err("Self projection substitution in this type form is not yet supported".into())
        }
        _ => ty.try_apply_subst(substitution),
    }
}
