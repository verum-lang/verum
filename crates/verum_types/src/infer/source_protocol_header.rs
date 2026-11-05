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

#[derive(Debug)]
pub(crate) struct SourceProtocolHeader {
    pub identity: Text,
    pub parameters: std::sync::Arc<List<GenericParam>>,
    pub owner: Option<List<Type>>,
    pub constraints: ProtocolConstraints,
    pub associated: Map<Text, AssociatedType>,
    pub supers: List<ProtocolBound>,
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
    ) -> Result<(Map<Text, ProtocolMethod>, Map<Text, AssociatedType>)> {
        // A recursive source load introduces a different Self. Its method
        // obligations must not inherit the caller's equally-spelled receiver.
        let previous = std::mem::take(&mut self.source_type_registration.projection_assumptions);
        let result = self.elaborate_protocol_members_in_scope(
            identity,
            parameters,
            constraints,
            supers,
            items,
        );
        self.source_type_registration.projection_assumptions = previous;
        result
    }

    fn elaborate_protocol_members_in_scope(
        &mut self,
        identity: Text,
        parameters: &[GenericParam],
        constraints: &ProtocolConstraints,
        supers: &[ProtocolBound],
        items: &[ProtocolItem],
    ) -> Result<(Map<Text, ProtocolMethod>, Map<Text, AssociatedType>)> {
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
        self.source_type_registration
            .headers
            .push(SourceProtocolHeader {
                identity,
                parameters: std::sync::Arc::new(parameters.iter().cloned().collect()),
                owner,
                constraints: constraints.clone(),
                associated: associated.clone(),
                supers: supers.iter().cloned().collect(),
            });
        let result = (|| {
            let mut methods = Map::new();
            for item in items {
                if let ProtocolItemKind::Function { decl, default_impl } = &item.kind {
                    let method = self.elaborate_protocol_method(
                        parameters,
                        constraints,
                        decl,
                        decl.body.is_some() || default_impl.is_some(),
                    )?;
                    if methods.insert(decl.name.name.clone(), method).is_some() {
                        return Err(invalid(item.span, "duplicate protocol method"));
                    }
                }
            }
            Ok((methods, associated))
        })();
        self.source_type_registration.headers.pop();
        result
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
            let owner = header.owner.as_ref().ok_or_else(|| {
                invalid(
                    span,
                    "associated projection owner has no represented declaration telescope",
                )
            })?;
            let protocol = verum_ast::Path::new(
                header
                    .identity
                    .as_str()
                    .split('.')
                    .map(|part| verum_ast::PathSegment::Name(verum_ast::Ident::new(part, span)))
                    .collect(),
                span,
            );
            assumptions.push(WhereClause {
                ty: receiver.clone(),
                bounds: List::from_iter([ProtocolBound {
                    protocol,
                    args: owner.clone(),
                    associated_types: Map::new(),
                    is_negative: false,
                }]),
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
