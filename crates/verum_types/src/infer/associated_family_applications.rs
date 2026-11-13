//! An associated family application is indivisible: resolve its complete
//! receiver and argument telescope before normalizing the result constructor.
use super::TypeChecker;
use crate::{
    Result, Type, TypeError,
    protocol::{Protocol, ProtocolConstraints, WhereClause},
};
use verum_ast::Span;
use verum_common::{List, Text};

pub(super) fn projection_parts(ty: &Type) -> Option<(&Type, Text, &[Type])> {
    match ty {
        Type::TypeApp { constructor, args } => match constructor.as_ref() {
            Type::Generic {
                name,
                args: receiver,
            } if name.starts_with("::") && receiver.len() == 1 => {
                Some((&receiver[0], name[2..].into(), args))
            }
            _ => None,
        },
        Type::Generic { name, args } if name.starts_with("::") && args.len() > 1 => {
            Some((&args[0], name[2..].into(), &args.as_slice()[1..]))
        }
        _ => None,
    }
}

/// Source admission also proves ordinary zero-argument associated members.
/// Keep their existing normalization representation unchanged.
pub(super) fn checked_projection_parts(ty: &Type) -> Option<(&Type, Text, &[Type])> {
    if let Some((receiver, name)) = ty.ordinary_projection() {
        Some((receiver, name.into(), &[]))
    } else {
        projection_parts(ty)
    }
}

impl TypeChecker {
    pub(super) fn resolve_checked_family_application(&self, ty: &Type, span: Span) -> Result<Type> {
        let Some((receiver, name, arguments)) = checked_projection_parts(ty) else {
            return Ok(ty.clone());
        };
        let assumptions: List<_> = self
            .type_var_bounds
            .iter()
            .map(|(variable, bounds)| WhereClause {
                ty: self.unifier.apply(&Type::Var(*variable)),
                bounds: bounds.clone(),
            })
            .collect();
        let receiver = self.normalize_type(receiver);
        self.protocol_checker
            .read()
            .resolve_associated_family(&receiver, &name, arguments, &assumptions)
            .map(|application| {
                application
                    .map(|value| value.result)
                    .unwrap_or_else(|| ty.clone())
            })
            .map_err(|message| TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: message,
                span,
            })
    }

    /// Normalization never authorizes an unproved application. The fallible
    /// source boundary reports its domain error; this consumer retains identity.
    pub(super) fn normalize_family_application(&self, ty: &Type) -> Type {
        self.resolve_checked_family_application(ty, Span::dummy())
            .unwrap_or_else(|_| ty.clone())
    }

    /// A protocol's own methods are elaborated before it is published. Validate
    /// their family domains against the complete declaration and exact local
    /// assumptions, rather than searching an incomplete ambient registry.
    pub(super) fn check_protocol_family_applications(&self, protocol: &Protocol) -> Result<()> {
        let ProtocolConstraints::Declared {
            parameters,
            parameter_bounds,
            ..
        } = &protocol.constraints
        else {
            return Ok(());
        };
        let owner: List<_> = parameters
            .iter()
            .map(|binder| binder.declaration_type().map_err(|message| TypeError::OtherWithCodeSpanned {
                code: "E405".into(), msg: message, span: protocol.span,
            }))
            .collect::<Result<_>>()?;
        let header = super::source_protocol_header::SourceProtocolHeader {
            dyn_required: List::new(),
            identity: protocol.name.clone(), parameters: protocol.source_parameters.clone().unwrap_or_default(),
            owner: Some(owner.clone()), constraints: protocol.constraints.clone(), associated: protocol.associated_types.clone(), supers: protocol.super_protocols.clone(),
        };
        for method in protocol.methods.values() {
            let Some(scope) = &method.scope else {
                continue;
            };
            let mut assumptions = parameter_bounds.clone();
            assumptions.extend(scope.where_clauses.iter().cloned());
            assumptions.extend(scope.protocol_bounds.iter().map(|(variable, bounds)| {
                WhereClause {
                    ty: Type::Var(*variable),
                    bounds: bounds.clone(),
                }
            }));
            let mut pending = vec![&method.ty];
            pending.extend(method.type_param_bounds.values().flatten());
            for clause in &assumptions {
                pending.push(&clause.ty);
                for bound in &clause.bounds {
                    pending.extend(bound.args.iter());
                    pending.extend(bound.associated_types.values());
                }
            }
            while let Some(ty) = pending.pop() {
                if let Some((receiver, name, arguments)) = checked_projection_parts(ty) {
                    if matches!(receiver, Type::Named { path, args } if args.is_empty() && path.as_ident().is_some_and(|name| name.name.as_str() == "Self"))
                    {
                        self.validate_projection_with_header(Some(&header), receiver, &name,
                            arguments, &assumptions, protocol.span)?;
                    }
                    // An applied family is one declaration-owned application.
                    // Its bare constructor is not a second zero-argument use;
                    // recursively validate only its receiver and actual inputs.
                    pending.push(receiver);
                    pending.extend(arguments.iter());
                    continue;
                }
                match ty {
                    Type::Function {
                        params,
                        return_type,
                        type_params,
                        ..
                    } => {
                        pending.extend(params.iter());
                        pending.push(return_type);
                        for parameter in type_params {
                            if let Some(default) = &parameter.default {
                                pending.push(default);
                            }
                            for bound in &parameter.bounds {
                                pending.extend(bound.args.iter());
                                pending.extend(bound.associated_types.values());
                            }
                        }
                    }
                    Type::Named { args, .. } | Type::Generic { args, .. } | Type::Tuple(args) => {
                        pending.extend(args.iter())
                    }
                    Type::TypeApp { constructor, args } => {
                        pending.push(constructor);
                        pending.extend(args.iter());
                    }
                    Type::Reference { inner, .. }
                    | Type::CheckedReference { inner, .. }
                    | Type::UnsafeReference { inner, .. }
                    | Type::Pointer { inner, .. }
                    | Type::VolatilePointer { inner, .. }
                    | Type::Ownership { inner, .. }
                    | Type::GenRef { inner }
                    | Type::Quantified { inner, .. } => pending.push(inner),
                    Type::Array { element, .. }
                    | Type::Slice { element }
                    | Type::Tensor { element, .. } => pending.push(element),
                    Type::Record(fields) | Type::Variant(fields) => pending.extend(fields.values()),
                    Type::ExtensibleRecord { fields, .. } => pending.extend(fields.values()),
                    Type::DynProtocol { bindings, .. } => pending.extend(bindings.values()),
                    Type::Forall { body, .. } | Type::Exists { body, .. } => pending.push(body),
                    Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                        pending.push(base)
                    }
                    Type::Meta { ty, .. } | Type::Eq { ty, .. } => pending.push(ty),
                    Type::Future { output } => pending.push(output),
                    Type::Generator {
                        yield_ty,
                        return_ty,
                    } => {
                        pending.push(yield_ty);
                        pending.push(return_ty);
                    }
                    Type::Pi {
                        param_type,
                        return_type,
                        ..
                    } => {
                        pending.push(param_type);
                        pending.push(return_type);
                    }
                    Type::Sigma {
                        fst_type, snd_type, ..
                    } => {
                        pending.push(fst_type);
                        pending.push(snd_type);
                    }
                    Type::PathType { space, .. } => pending.push(space),
                    Type::Partial { element_type, .. } => pending.push(element_type),
                    Type::Inductive {
                        params,
                        indices,
                        constructors,
                        ..
                    } => {
                        pending.extend(params.iter().chain(indices).map(|(_, ty)| ty.as_ref()));
                        for constructor in constructors {
                            pending
                                .extend(constructor.type_params.iter().map(|(_, ty)| ty.as_ref()));
                            pending.extend(constructor.args.iter().map(|ty| ty.as_ref()));
                            pending.push(&constructor.return_type);
                        }
                    }
                    Type::Coinductive {
                        params,
                        destructors,
                        ..
                    } => {
                        pending.extend(params.iter().map(|(_, ty)| ty.as_ref()));
                        pending.extend(
                            destructors
                                .iter()
                                .map(|destructor| destructor.result_type.as_ref()),
                        );
                    }
                    Type::HigherInductive {
                        params,
                        point_constructors,
                        path_constructors,
                        ..
                    } => {
                        pending.extend(params.iter().map(|(_, ty)| ty.as_ref()));
                        for constructor in point_constructors {
                            pending
                                .extend(constructor.type_params.iter().map(|(_, ty)| ty.as_ref()));
                            pending.extend(constructor.args.iter().map(|ty| ty.as_ref()));
                            pending.push(&constructor.return_type);
                        }
                        for constructor in path_constructors {
                            pending
                                .extend(constructor.type_params.iter().map(|(_, ty)| ty.as_ref()));
                            pending.extend(constructor.args.iter().map(|ty| ty.as_ref()));
                            pending.push(&constructor.path_type.ty);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl TypeChecker {
    pub(super) fn resolve_checked_omitted_family(
        &self,
        receiver: &Type,
        name: &Text,
        span: Span,
    ) -> Result<Option<Type>> {
        let assumptions: List<_> = self
            .type_var_bounds
            .iter()
            .map(|(variable, bounds)| WhereClause {
                ty: self.unifier.apply(&Type::Var(*variable)),
                bounds: bounds.clone(),
            })
            .collect();
        let receiver = self.normalize_type(receiver);
        self.protocol_checker
            .read()
            .resolve_omitted_family_arguments(&receiver, name, &assumptions)
            .map(|application| application.map(|application| application.result))
            .map_err(|msg| TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg,
                span,
            })
    }
}
