//! Retain protocol applications in the implementing declaration's binder space.
//! The protocol owns names in default signatures/bodies; the implementation
//! owns every substituted TypeRef and runtime witness slot.
use super::{
    CodegenError, CodegenErrorKind, CodegenResult, ImplReceiverWitness, ProtocolInfo, VbcCodegen,
    source_substitution::SourceTypeSubstitution,
};
use std::collections::HashMap;
use verum_ast::{
    Path, Type,
    ty::{GenericArg, GenericParamKind, TypeKind},
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
                self.resolve_protocol_arguments(&info, args, scope, declared_ids, path.span)?
            }
            None => SourceTypeSubstitution::default(),
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
            let GenericParamKind::Type { name, default, .. } = &parameter.kind else {
                return Err(invalid(
                    "protocol application parameter has no source type substitution representation",
                    parameter.span,
                ));
            };
            let witness = if let Some(argument) = arguments.next() {
                let ty = argument.type_for_declared_slot().ok_or_else(|| {
                    invalid("protocol application argument does not denote a type", span)
                })?;
                self.resolve_protocol_argument(&ty, scope, declared_ids)?
            } else if let Some(default) = default {
                self.with_protocol_source_scope(
                    Some(protocol.source_module.clone()),
                    result.clone(),
                    None,
                    |cg| cg.resolve_protocol_argument(default, &HashMap::new(), declared_ids),
                )?
            } else {
                return Err(invalid(
                    &format!(
                        "protocol application '{}' is missing argument '{}'",
                        protocol.name, name.name
                    ),
                    span,
                ));
            };
            result.insert(name.name.to_string(), witness);
        }
        if arguments.next().is_some() {
            return Err(invalid(
                &format!(
                    "protocol application '{}' has too many positional arguments",
                    protocol.name
                ),
                span,
            ));
        }
        Ok(result)
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

fn invalid(message: &str, span: verum_ast::Span) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into()),
        span,
    )
}
