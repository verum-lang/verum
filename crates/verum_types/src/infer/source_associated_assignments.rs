//! Implementation family bodies own their local telescope independently of
//! protocol re-elaboration. Source names may shadow owner binders.
use super::TypeChecker;
use crate::{Result, Type, TypeError, protocol::AssociatedTypeAssignment};
use verum_ast::decl::{ImplDecl, ImplItemKind};
use verum_common::{Map, Text};

fn invalid(span: verum_ast::Span, message: impl Into<Text>) -> TypeError {
    TypeError::OtherWithCodeSpanned {
        code: "E405".into(),
        msg: message.into(),
        span,
    }
}

impl TypeChecker {
    pub(super) fn elaborate_impl_associated_assignments(
        &mut self,
        protocol: &Text,
        implementation: &ImplDecl,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<Map<Text, AssociatedTypeAssignment>> {
        if !implementation
            .items
            .iter()
            .any(|item| matches!(item.kind, ImplItemKind::Type { .. }))
        {
            return Ok(Map::new());
        }
        self.check_associated_type_obligations(implementation, protocol)?;
        let declarations = self
            .protocol_checker
            .read()
            .get_protocol_definition(protocol)
            .ok_or_else(|| {
                invalid(
                    implementation.span,
                    "associated assignment has no selected protocol declaration",
                )
            })?
            .associated_types
            .clone();
        let previous_self = self.current_self_type.clone();
        self.push_decl_param_frame(owner.clone());
        let lowered = (|| -> Result<_> {
            let verum_ast::decl::ImplKind::Protocol { for_type, .. } = &implementation.kind else {
                return Err(invalid(
                    implementation.span,
                    "associated assignment requires a protocol implementation",
                ));
            };
            let receiver = self.ast_to_type(for_type)?;
            self.set_current_self_type(Some(receiver));
            let mut assignments = Map::new();
            for item in &implementation.items {
                let ImplItemKind::Type {
                    name,
                    type_params,
                    ty,
                    ..
                } = &item.kind
                else {
                    continue;
                };
                let declaration = declarations.get(&name.name).ok_or_else(|| {
                    invalid(
                        item.span,
                        format!(
                            "associated assignment `{}` is absent from its protocol",
                            name.name
                        ),
                    )
                })?;
                if declaration.refinement.is_some()
                    || matches!(
                        declaration.kind,
                        crate::advanced_protocols::AssociatedTypeKind::HigherKinded { .. }
                    )
                {
                    return Err(invalid(
                        item.span,
                        "associated assignment requires checked refinement or declared kind evidence",
                    ));
                }
                if declaration.binders.local.len() != type_params.len() {
                    return Err(invalid(
                        item.span,
                        "associated assignment does not match its declared local telescope",
                    ));
                }
                let (frame, local_parameters) = self.associated_parameter_frame(type_params)?;
                self.push_decl_param_frame(frame);
                let lowered = self.ast_to_type(ty);
                self.pop_decl_param_frame();
                let lowered = self.unifier.normalize_aliases(&lowered?);
                let assignment = AssociatedTypeAssignment::mixed_family(local_parameters, lowered)
                    .map_err(|message| invalid(item.span, message))?;
                if assignments.insert(name.name.clone(), assignment).is_some() {
                    return Err(invalid(item.span, "duplicate associated assignment"));
                }
            }
            Ok(assignments)
        })();
        self.pop_decl_param_frame();
        self.set_current_self_type(previous_self);
        lowered
    }
}
