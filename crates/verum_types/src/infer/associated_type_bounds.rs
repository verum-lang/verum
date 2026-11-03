//! Associated families retain their declaration telescope and complete bounds.

use super::TypeChecker;
use crate::advanced_protocols::{AssociatedTypeKind, GATTypeParam, GATWhereClause, Variance};
use crate::{
    Result, TypeError,
    protocol::{AssociatedType, AssociatedTypeBinders},
    ty::{Type, TypeVar},
};
use verum_ast::{
    decl::ProtocolItemKind,
    ty::{GenericParam, GenericParamKind, TypeBound, TypeBoundKind, WherePredicateKind},
};
use verum_common::{List, Maybe, Text};

impl TypeChecker {
    pub(super) fn elaborate_associated_type(
        &mut self,
        owner: &[GenericParam],
        item: &ProtocolItemKind,
    ) -> Result<AssociatedType> {
        let ProtocolItemKind::Type {
            name,
            type_params,
            bounds,
            where_clause,
            default_type,
        } = item
        else {
            return Err(TypeError::Other(
                "expected an associated type declaration".into(),
            ));
        };
        let mut frame = indexmap::IndexMap::new();
        let mut binders = AssociatedTypeBinders::default();
        for (position, parameter) in owner.iter().enumerate() {
            if let Some(name) = type_parameter_name(parameter) {
                let var = TypeVar::fresh();
                frame.insert(name.clone(), Type::Var(var));
                binders.owner.push((position, var));
            }
        }
        for parameter in type_params {
            let Some(name) = type_parameter_name(parameter) else {
                return Err(unsupported(
                    parameter.span,
                    "associated family value, level and context parameters are not supported",
                ));
            };
            let var = TypeVar::fresh();
            frame.insert(name.clone(), Type::Var(var));
            binders.local.push(var);
        }
        self.push_decl_param_frame(frame);
        let result = (|| {
            let mut parameters = List::new();
            let mut higher_kinded = false;
            for parameter in type_params {
                let (name, bounds, default) = match &parameter.kind {
                    GenericParamKind::Type {
                        name,
                        bounds,
                        default,
                    } => (name, bounds, default.as_ref()),
                    GenericParamKind::HigherKinded { name, bounds, .. }
                    | GenericParamKind::KindAnnotated { name, bounds, .. } => {
                        higher_kinded = true;
                        (name, bounds, None)
                    }
                    _ => unreachable!("family parameter kinds checked before entering scope"),
                };
                parameters.push(GATTypeParam {
                    name: name.name.clone(),
                    bounds: self.elaborate_family_bounds(bounds)?,
                    default: default.map(|ty| self.ast_to_type(ty)).transpose()?.into(),
                    variance: Variance::Invariant,
                });
            }
            let mut clauses = List::new();
            if let Some(clause) = where_clause {
                for predicate in &clause.predicates {
                    let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                        return Err(unsupported(
                            predicate.span,
                            "associated family where clauses require a type parameter bound",
                        ));
                    };
                    let verum_ast::ty::TypeKind::Path(path) = &ty.kind else {
                        return Err(unsupported(
                            predicate.span,
                            "associated family where clauses require a type parameter bound",
                        ));
                    };
                    let Some(ident) = path.as_ident() else {
                        return Err(unsupported(
                            predicate.span,
                            "associated family where clauses require a type parameter bound",
                        ));
                    };
                    if !parameters
                        .iter()
                        .any(|parameter: &GATTypeParam| parameter.name == ident.name)
                    {
                        return Err(unsupported(
                            predicate.span,
                            "associated family where clause must constrain a family parameter",
                        ));
                    }
                    clauses.push(GATWhereClause {
                        param: ident.name.clone(),
                        constraints: self.elaborate_family_bounds(bounds)?,
                        span: predicate.span,
                    });
                }
            }
            let mut associated = AssociatedType::generic(
                name.name.clone(),
                parameters,
                self.elaborate_family_bounds(bounds)?,
                clauses,
            );
            associated.binders = binders;
            associated.default = default_type
                .as_ref()
                .map(|ty| self.ast_to_type(ty))
                .transpose()?
                .into();
            if type_params.is_empty() {
                associated.kind = AssociatedTypeKind::Regular;
            } else if higher_kinded {
                associated.kind = AssociatedTypeKind::HigherKinded {
                    arity: type_params.len(),
                };
            }
            Ok(associated)
        })();
        self.pop_decl_param_frame();
        result
    }

    fn elaborate_family_bounds(
        &mut self,
        bounds: &[TypeBound],
    ) -> Result<List<crate::protocol::ProtocolBound>> {
        for bound in bounds {
            match &bound.kind {
                TypeBoundKind::Protocol(_) | TypeBoundKind::NegativeProtocol(_) => {}
                TypeBoundKind::GenericProtocol(ty) => {
                    let verum_ast::ty::TypeKind::Generic { base, args } = &ty.kind else {
                        return Err(unsupported(bound.span, "invalid generic protocol bound"));
                    };
                    if !matches!(base.kind, verum_ast::ty::TypeKind::Path(_))
                        || args.iter().any(|arg| {
                            !matches!(
                                arg,
                                verum_ast::ty::GenericArg::Type(_)
                                    | verum_ast::ty::GenericArg::Binding(_)
                            )
                        })
                    {
                        return Err(unsupported(
                            bound.span,
                            "associated family protocol bounds require type arguments or associated type equalities",
                        ));
                    }
                }
                _ => {
                    return Err(unsupported(
                        bound.span,
                        "this associated family constraint cannot yet be checked; use a protocol bound with associated type equalities",
                    ));
                }
            }
        }
        self.convert_type_bounds_to_protocol_bounds(bounds)
    }
}

fn type_parameter_name(parameter: &GenericParam) -> Option<&Text> {
    match &parameter.kind {
        GenericParamKind::Type { name, .. }
        | GenericParamKind::HigherKinded { name, .. }
        | GenericParamKind::KindAnnotated { name, .. } => Some(&name.name),
        _ => None,
    }
}

fn unsupported(span: verum_ast::span::Span, message: &str) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}
