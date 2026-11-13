//! In-progress source headers are local proof context, never published protocols.
use super::TypeChecker;
use crate::{
    Result, Type, TypeError,
    protocol::{AssociatedType, ProtocolBound, ProtocolConstraints, ProtocolMethod, WhereClause},
};
use verum_ast::{
    GenericParam, Span,
    decl::{ProtocolItem, ProtocolItemKind},
};
use verum_common::{List, Map, Text};

pub(super) type DynObligations = List<(Span, verum_common::Set<Text>)>;

#[derive(Debug)]
pub(crate) struct SourceProtocolHeader {
    pub identity: Text,
    pub parameters: std::sync::Arc<List<GenericParam>>,
    pub owner: Option<List<Type>>,
    pub constraints: ProtocolConstraints,
    pub associated: Map<Text, AssociatedType>,
    pub supers: List<ProtocolBound>,
    pub(super) dyn_required: DynObligations,
}

impl SourceProtocolHeader {
    pub(super) fn self_bound(&self, span: Span) -> Option<ProtocolBound> {
        let args = self.owner.clone()?;
        let protocol = verum_ast::Path::new(
            self.identity.as_str().split('.')
                .map(|part| verum_ast::PathSegment::Name(verum_ast::Ident::new(part, span)))
                .collect(),
            span,
        );
        Some(ProtocolBound { protocol, args, associated_types: Map::new(), is_negative: false })
    }
}

fn invalid(span: Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    /// Every source registration route uses the same two phases. A later
    /// associated declaration is available to an earlier method, but a failed
    /// method cannot publish an incomplete protocol or leak its header outward.
    pub(super) fn elaborate_protocol_members(
        &mut self,
        identity: Text,
        parameters: &[GenericParam],
        constraints: &ProtocolConstraints,
        supers: &[ProtocolBound],
        items: &[ProtocolItem],
        declared_default_methods: &[Text],
    ) -> Result<(Map<Text, ProtocolMethod>, Map<Text, AssociatedType>, DynObligations)> {
        // A recursive source load introduces a different Self. Its method
        // obligations must not inherit the caller's equally-spelled receiver.
        let previous = std::mem::take(&mut self.source_type_registration.projection_assumptions);
        let callables = std::mem::take(&mut self.source_type_registration.callable_assumptions);
        let result = self.elaborate_protocol_members_in_scope(
            identity,
            parameters,
            constraints,
            supers,
            items,
            declared_default_methods,
        );
        self.source_type_registration.projection_assumptions = previous;
        self.source_type_registration.callable_assumptions = callables;
        result
    }

    fn elaborate_protocol_members_in_scope(
        &mut self,
        identity: Text,
        parameters: &[GenericParam],
        constraints: &ProtocolConstraints,
        supers: &[ProtocolBound],
        items: &[ProtocolItem],
        declared_default_methods: &[Text],
    ) -> Result<(Map<Text, ProtocolMethod>, Map<Text, AssociatedType>, DynObligations)> {
        let mut associated = Map::new();
        for item in items {
            if let ProtocolItemKind::Type { name, .. } = &item.kind {
                let declaration =
                    self.elaborate_associated_type(parameters, constraints, &item.kind)?;
                if associated.insert(name.name.clone(), declaration).is_some() {
                    return Err(invalid(item.span, "duplicate associated declaration"));
                }
            }
        }
        let owner = match constraints {
            ProtocolConstraints::Declared { parameters, .. } => Some(
                parameters
                    .iter()
                    .map(|variable| Type::Var(*variable))
                    .collect(),
            ),
            _ if parameters.is_empty() => Some(List::new()),
            _ => None,
        };
        let header = SourceProtocolHeader {
            identity,
            parameters: std::sync::Arc::new(parameters.iter().cloned().collect()),
            owner,
            constraints: constraints.clone(),
            associated: associated.clone(),
            supers: supers.iter().cloned().collect(),
            dyn_required: List::new(),
        };
        let span = items.first().map(|item| item.span).unwrap_or_default();
        let self_bound = header.self_bound(span);
        self.source_type_registration.headers.push(header);
        let result = (|| {
            if let Some(bound) = self_bound {
                let receiver = Type::Named {
                    path: verum_ast::Path::single(verum_ast::Ident::new("Self", span)),
                    args: List::new(),
                };
                let mut bounds = List::from_iter([bound]);
                // This protocol is not published until its signatures succeed.
                // Its declared edges already constrain the local Self, including
                // when Self appears as an argument of a superprotocol.
                if let ProtocolConstraints::Declared { self_type, parameter_bounds, .. } = constraints {
                    let substitution = [(*self_type, receiver.clone())].into_iter().collect();
                    for inherited in supers {
                        bounds.push(inherited.try_apply_subst(&substitution)
                            .map_err(|message| invalid(inherited.protocol.span, message))?);
                    }
                    for clause in parameter_bounds {
                        self.source_type_registration.projection_assumptions.push(
                            clause.try_apply_subst(&substitution)
                                .map_err(|message| invalid(span, message))?
                        );
                    }
                }
                self.source_type_registration.projection_assumptions.push(WhereClause {
                    ty: receiver, bounds,
                });
            }
            let mut methods = Map::new();
            for item in items {
                if let ProtocolItemKind::Function { decl, default_impl } = &item.kind {
                    let method = self.elaborate_protocol_method(
                        parameters,
                        constraints,
                        decl,
                        decl.body.is_some() || default_impl.is_some() || declared_default_methods.contains(&decl.name.name),
                    )?;
                    if methods.insert(decl.name.name.clone(), method).is_some() {
                        return Err(invalid(item.span, "duplicate protocol method"));
                    }
                }
            }
            Ok((methods, associated))
        })();
        let header = self.source_type_registration.headers.pop()
            .expect("source protocol member scope remains active during elaboration");
        result.map(|(methods, associated)| (methods, associated, header.dyn_required))
    }

    /// A projection is a type only after its receiver proves an exact declared
    /// member. The spelling `::Name` alone carries no such authority.
    pub(super) fn validate_source_projection(
        &self,
        receiver: &Type,
        name: &Text,
        arguments: &[Type],
        span: Span,
    ) -> Result<()> {
        if matches!(receiver, Type::Meta { .. } | Type::Unknown) {
            return Err(invalid(span, "associated projection has no type receiver"));
        }
        let mut assumptions: List<_> = self
            .type_var_bounds
            .iter()
            .map(|(variable, bounds)| WhereClause {
                ty: self.unifier.apply(&Type::Var(*variable)),
                bounds: bounds.clone(),
            })
            .collect();
        assumptions.extend(
            self.source_type_registration
                .projection_assumptions
                .iter()
                .cloned(),
        );
        self.validate_projection_with_header(
            self.source_type_registration.headers.last(),
            receiver,
            name,
            arguments,
            &assumptions,
            span,
        )
    }

    pub(super) fn validate_projection_with_header(
        &self,
        header: Option<&SourceProtocolHeader>,
        receiver: &Type,
        name: &Text,
        arguments: &[Type],
        assumptions: &[WhereClause],
        span: Span,
    ) -> Result<()> {
        let mut assumptions: List<_> = assumptions.iter().cloned().collect();
        let symbolic_self = matches!(receiver, Type::Named { path, args }
            if args.is_empty() && path.as_ident().is_some_and(|ident| ident.name.as_str() == "Self"));
        if symbolic_self {
            let header = header.ok_or_else(|| {
                invalid(
                    span,
                    "associated Self projection has no declaring protocol header",
                )
            })?;
            let bound = header.self_bound(span).ok_or_else(|| {
                invalid(
                    span,
                    "associated projection owner has no represented declaration telescope",
                )
            })?;
            assumptions.push(WhereClause {
                ty: receiver.clone(),
                bounds: List::from_iter([bound]),
            });
        }
        let application = self
            .protocol_checker
            .read()
            .resolve_associated_family_with_header(
                receiver,
                name,
                arguments,
                &assumptions,
                header.map(|header| crate::protocol::AssociatedDeclarationHeader {
                    identity: &header.identity,
                    constraints: &header.constraints,
                    associated: &header.associated,
                    supers: &header.supers,
                }),
            )
            .map_err(|message| invalid(span, message))?;
        if application.is_none() {
            return Err(invalid(
                span,
                format!("associated projection `{name}` has no unique declared receiver bound"),
            ));
        }
        Ok(())
    }
}
