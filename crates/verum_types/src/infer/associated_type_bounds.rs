//! Associated families retain their declaration telescope and complete bounds.

use super::TypeChecker;
use crate::advanced_protocols::{AssociatedTypeKind, GATTypeParam, GATWhereClause, Variance};
use crate::{
    Result, TypeError,
    context::GenericBinder,
    protocol::{
        AssociatedFamilyBinders, AssociatedType, ProtocolBound, ProtocolConstraints, WhereClause,
    },
    ty::{Type, TypeVar},
};
use verum_ast::{
    decl::ProtocolItemKind,
    ty::{GenericParam, GenericParamKind, TypeBound, TypeBoundKind, WherePredicateKind},
};
use verum_common::{List, Maybe, Text};

impl TypeChecker {
    pub(super) fn source_super_protocol_bounds(
        supers: &[verum_ast::Type],
    ) -> Result<List<TypeBound>> {
        supers
            .iter()
            .map(|ty| {
                Ok(TypeBound {
                    span: ty.span,
                    kind: match &ty.kind {
                        verum_ast::TypeKind::Path(path) => TypeBoundKind::Protocol(path.clone()),
                        verum_ast::TypeKind::Generic { .. } => {
                            TypeBoundKind::GenericProtocol(ty.clone())
                        }
                        _ => {
                            return Err(unsupported(
                                ty.span,
                                "a super-protocol must be a declared protocol type",
                            ));
                        }
                    },
                })
            })
            .collect()
    }

    pub(super) fn protocol_constraint_scope(
        &mut self,
        parameters: &[GenericParam],
        supers: &[verum_ast::Type],
        where_clauses: &[Option<&verum_ast::WhereClause>],
    ) -> (ProtocolConstraints, Option<List<ProtocolBound>>) {
        let saved_self = self.current_self_type.clone();
        let self_type = TypeVar::fresh();
        let mut frame = indexmap::IndexMap::new();
        let mut variables = List::new();
        for parameter in parameters {
            let GenericParamKind::Type { name, .. } = &parameter.kind else {
                // An unavailable universal telescope is not an elaboration error.
                // Keep ordinary unconstrained value/kind protocols usable; any
                // actual constraint remains an explicit unsupported obligation.
                let has_constraints = !supers.is_empty()
                    || where_clauses
                        .iter()
                        .flatten()
                        .any(|clause| !clause.predicates.is_empty())
                    || parameters.iter().any(|parameter| match &parameter.kind {
                        GenericParamKind::Type {
                            bounds, default, ..
                        } => !bounds.is_empty() || default.is_some(),
                        GenericParamKind::HigherKinded { bounds, .. }
                        | GenericParamKind::KindAnnotated { bounds, .. } => !bounds.is_empty(),
                        GenericParamKind::Meta { ty, refinement, .. } => {
                            refinement.is_some() || value_type_has_obligations(ty)
                        }
                        GenericParamKind::Const { ty, .. } => value_type_has_obligations(ty),
                        _ => true,
                    });
                return (
                    if has_constraints {
                        ProtocolConstraints::Unsupported(
                        "universal protocol constraints currently require ordinary type parameters"
                            .into(),
                    )
                    } else {
                        ProtocolConstraints::Unknown
                    },
                    None,
                );
            };
            let variable = TypeVar::fresh();
            variables.push(variable);
            frame.insert(name.name.clone(), Type::Var(variable));
        }
        self.push_decl_param_frame(frame);
        self.set_current_self_type(Some(Type::Var(self_type)));
        let result = (|| -> Result<_> {
            let mut defaults = List::new();
            let mut parameter_bounds = List::new();
            for (parameter, variable) in parameters.iter().zip(&variables) {
                let GenericParamKind::Type {
                    bounds, default, ..
                } = &parameter.kind
                else {
                    unreachable!()
                };
                defaults.push(
                    default
                        .as_ref()
                        .map(|ty| self.ast_to_type(ty))
                        .transpose()?,
                );
                parameter_bounds.push(WhereClause {
                    ty: Type::Var(*variable),
                    bounds: self.elaborate_family_bounds(bounds)?,
                });
            }
            for clause in where_clauses.iter().flatten() {
                for predicate in &clause.predicates {
                    let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                        return Err(unsupported(
                            predicate.span,
                            "protocol constraint scope requires type obligations",
                        ));
                    };
                    parameter_bounds.push(WhereClause {
                        ty: self.ast_to_type(ty)?,
                        bounds: self.elaborate_family_bounds(bounds)?,
                    });
                }
            }
            let bounds = Self::source_super_protocol_bounds(supers)?;
            let supers = self.elaborate_family_bounds(&bounds)?;
            Ok((
                ProtocolConstraints::Declared {
                    self_type,
                    parameters: variables,
                    defaults,
                    parameter_bounds,
                },
                supers,
            ))
        })();
        self.pop_decl_param_frame();
        self.set_current_self_type(saved_self);
        match result {
            Ok((scope, supers)) => (scope, Some(supers)),
            Err(error) => (
                ProtocolConstraints::Unsupported(error.to_string().into()),
                None,
            ),
        }
    }

    pub(super) fn elaborate_associated_type(
        &mut self,
        owner: &[GenericParam],
        scope: &ProtocolConstraints,
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
        let mut binders = AssociatedFamilyBinders::default();
        for (position, parameter) in owner.iter().enumerate() {
            let Some(name) = type_parameter_name(parameter) else {
                return Err(unsupported(
                    parameter.span,
                    "associated family owner requires a checked protocol telescope",
                ));
            };
            let var = match scope {
                ProtocolConstraints::Declared { parameters, .. } => parameters[position],
                _ => TypeVar::fresh(),
            };
            frame.insert(name.clone(), Type::Var(var));
            binders.owner.push((position, GenericBinder::Type(var)));
        }
        if let ProtocolConstraints::Declared { self_type, .. } = scope {
            binders.self_type = Some(*self_type);
        }
        self.push_decl_param_frame(frame);
        let old_self = self.current_self_type.clone();
        if let Some(var) = binders.self_type {
            self.set_current_self_type(Some(Type::Var(var)));
        }
        let result = (|| {
            let (frame, local) = self.associated_parameter_frame(type_params)?;
            binders.local = local;
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
                        GenericParamKind::Meta { name, .. }
                        | GenericParamKind::Const { name, .. } => {
                            parameters.push(GATTypeParam {
                                name: name.name.clone(),
                                bounds: List::new(),
                                default: Maybe::None,
                                variance: Variance::Invariant,
                            });
                            continue;
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
        })();
        self.pop_decl_param_frame();
        self.set_current_self_type(old_self);
        result
    }

    /// Elaborate each local in lexical declaration order. The existing value
    /// binder constructor preserves its normalized sort and source refinement.
    pub(super) fn associated_parameter_frame(
        &mut self,
        parameters: &[GenericParam],
    ) -> Result<(indexmap::IndexMap<Text, Type>, List<GenericBinder>)> {
        // Declaration lookup deliberately reads only the innermost frame.
        // Carry this family's lexical owner explicitly, then shadow in order.
        let owner = self.decl_param_frames.last().cloned().unwrap_or_default();
        self.push_decl_param_frame(owner);
        let result = (|| {
            let mut binders = List::new();
            let mut local_names = verum_common::Set::new();
            for parameter in parameters {
                let (name, value) = match &parameter.kind {
                    GenericParamKind::Type { name, .. }
                    | GenericParamKind::HigherKinded { name, .. }
                    | GenericParamKind::KindAnnotated { name, .. } => {
                        (&name.name, Type::Var(TypeVar::fresh()))
                    }
                    GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } => {
                        (&name.name, self.source_value_parameter(parameter)?)
                    }
                    _ => {
                        return Err(unsupported(
                            parameter.span,
                            "associated family level, lifetime and context parameters require represented kind evidence",
                        ));
                    }
                };
                let binder = GenericBinder::declared(&value)
                    .map_err(|message| unsupported(parameter.span, &message))?;
                let frame = self
                    .decl_param_frames
                    .last_mut()
                    .expect("family lexical scope");
                if !local_names.insert(name.clone()) {
                    return Err(unsupported(
                        parameter.span,
                        "duplicate associated family parameter",
                    ));
                }
                frame.insert(name.clone(), value);
                binders.push(binder);
            }
            Ok((
                self.decl_param_frames
                    .last()
                    .expect("family lexical scope")
                    .clone(),
                binders,
            ))
        })();
        self.pop_decl_param_frame();
        result
    }

    pub(super) fn elaborate_family_bounds(
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

/// This only classifies explicit source obligations. Ordinary value-argument
/// kinds still belong to the argument elaborator; an unavailable universal
/// telescope neither validates them nor turns them into type parameters.
fn value_type_has_obligations(ty: &verum_ast::Type) -> bool {
    use verum_ast::{TypeKind, Visitor};
    struct Scan {
        found: bool,
        remaining: usize,
        depth: usize,
    }
    impl Visitor for Scan {
        // Value expressions (for example an array length) are validated by the
        // argument elaborator, not interpreted as universal protocol premises.
        fn visit_expr(&mut self, _: &verum_ast::Expr) {}

        fn visit_type(&mut self, ty: &verum_ast::Type) {
            if self.found {
                return;
            }
            if self.remaining == 0 || self.depth >= 64 {
                self.found = true;
                return;
            }
            self.remaining -= 1;
            if matches!(
                ty.kind,
                TypeKind::Refined { .. }
                    | TypeKind::Bounded { .. }
                    | TypeKind::Rank2Function { .. }
                    | TypeKind::Unknown
                    | TypeKind::Inferred
            ) {
                self.found = true;
                return;
            }
            self.depth += 1;
            verum_ast::visitor::walk_type(self, ty);
            self.depth -= 1;
        }
    }
    let mut scan = Scan {
        found: false,
        remaining: 4096,
        depth: 0,
    };
    scan.visit_type(ty);
    scan.found
}

fn unsupported(span: verum_ast::span::Span, message: &str) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}
