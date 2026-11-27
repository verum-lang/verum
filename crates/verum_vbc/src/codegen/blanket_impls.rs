//! Source declarations retained for blanket implementation specialization.
use super::{BlanketImpl, CodegenResult, VbcCodegen};
use verum_ast::{
    decl::{ImplDecl, ImplItemKind, ImplKind},
    ty::{GenericParamKind, PathSegment},
};

impl VbcCodegen {
    /// A retained template is lowered only after selecting its receiver. Its
    /// parameter name is not a nominal owner for a second executable body.
    pub(super) fn retains_blanket_declaration(&self, declaration: &ImplDecl) -> bool {
        self.blanket_impls.iter().any(|blanket| {
            blanket.source_module == self.ctx.current_source_module
                && blanket.declaration.span == declaration.span
                && blanket.declaration.as_ref() == declaration
        })
    }

    /// Both collection paths retain the same declaration and receiver binder.
    pub(super) fn record_blanket_impl(
        &mut self,
        item: &verum_ast::Item,
        source_module: Option<String>,
    ) -> CodegenResult<()> {
        let verum_ast::ItemKind::Impl(declaration) = &item.kind else {
            return Ok(());
        };
        let ImplKind::Protocol {
            protocol, for_type, ..
        } = &declaration.kind
        else {
            return Ok(());
        };
        let Some(PathSegment::Name(derived)) = protocol.segments.last() else {
            return Ok(());
        };
        let Some(receiver_parameter) = Self::for_type_generic_param_name(for_type) else {
            return Ok(());
        };
        let selected_items = self.selected_impl_items(declaration)?;
        let mut retained = None;
        for parameter in &declaration.generics {
            let GenericParamKind::Type { name, bounds, .. } = &parameter.kind else {
                continue;
            };
            if name.name.as_str() != receiver_parameter {
                continue;
            }
            for bound in bounds {
                let Some(base_protocol) = Self::type_bound_protocol_name(bound) else {
                    continue;
                };
                let derived_protocol = derived.name.to_string();
                if self.blanket_impls.iter().any(|existing| {
                    existing.source_module == source_module
                        && *existing.prerequisite == *bound
                        && *existing.declaration == *declaration
                }) {
                    continue;
                }
                let explicit_methods = selected_items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        ImplItemKind::Function(function) => Some(function.name.name.to_string()),
                        _ => None,
                    })
                    .collect();
                let (source, definition) = if let Some(retained) = &retained {
                    retained
                } else {
                    let owner = source_module
                        .as_deref()
                        .unwrap_or_else(|| self.source_module_root());
                    let source = verum_ast::source_implementation::AssociatedImplementationSource::new_with_items(owner.into(), declaration, &selected_items)
                        .map_err(|error| super::CodegenError::with_span(super::CodegenErrorKind::InvalidSourceSignature(error.into_string()), declaration.span))?;
                    retained.insert((
                        std::sync::Arc::new(source),
                        self.implementation_definition_in_module(item, owner)?,
                    ))
                };
                self.blanket_impls.push(BlanketImpl {
                    base_protocol,
                    source: source.clone(),
                    definition: *definition,
                    prerequisite: std::sync::Arc::new(bound.clone()),
                    derived_protocol,
                    explicit_methods,
                    receiver_parameter: receiver_parameter.clone(),
                    source_module: source_module.clone(),
                    declaration: std::sync::Arc::new(declaration.clone()),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BlanketSelection {
    pub(super) index: usize,
    definition: crate::module::SourceDefinitionIdentity,
}

/// A proof schedules one row for this exact application and receiver. Slot IDs
/// are supplied by the body registration that owns the declared member.
pub(super) struct MaterializedBlanket {
    target: Option<crate::types::TypeId>,
    receiver: crate::types::TypeRef,
    name: String,
    arguments: Vec<crate::types::TypeRef>,
    pub(super) members: Vec<String>,
    pub(super) implementation: crate::types::ProtocolImpl,
    pub(super) overridden: bool,
}

impl VbcCodegen {
    pub(super) fn queue_blanket_row(
        &mut self,
        blanket: &BlanketImpl,
        application: &super::protocol_applications::ProtocolApplication,
        trigger: &ImplDecl,
        name: &str,
        receiver: &super::ImplReceiverWitness,
        items: &[&verum_ast::decl::ImplItem],
    ) -> CodegenResult<Option<BlanketSelection>> {
        use super::CodegenError;
        use crate::types::{ProtocolId, ProtocolImpl, StringId, TypeRef};
        let Some(protocol) = application.protocol else {
            return Ok(None);
        };
        let target = self.implementation_nominal_id(&trigger.kind);
        let source = blanket.source.as_ref();
        if let Some(index) = self.pending_blanket_rows.iter().position(|row| {
            row.target == target
                && row.name == name
                && row.receiver == receiver.ty
                && row.implementation.source_implementation.as_ref() == Some(source)
        }) {
            return Ok(Some(BlanketSelection {
                index,
                definition: blanket.definition,
            }));
        }
        let Some(declaration) = self.type_by_id(protocol) else {
            return Ok(None);
        };
        let mut members: Vec<_> = declaration
            .variants
            .iter()
            .map(|member| {
                self.ctx
                    .strings
                    .get(member.name.0 as usize)
                    .cloned()
                    .ok_or_else(|| CodegenError::internal("protocol member has no name"))
            })
            .collect::<CodegenResult<_>>()?;
        for item in items {
            if let ImplItemKind::Function(function) = &item.kind {
                let member = function.name.as_str();
                if !members.iter().any(|name| name == member) {
                    members.push(member.to_owned());
                }
            }
        }
        let arguments = self
            .protocol_registry
            .get(&application.key)
            .into_iter()
            .flat_map(|protocol| protocol.generics.iter())
            .filter_map(|parameter| {
                super::protocol_applications::parameter_name(parameter)
                    .and_then(|name| application.substitution.get(name))
                    .map(|argument| argument.ty.clone())
            })
            .collect();
        let (protocol_args_text, type_param_fn_bounds) =
            self.implementation_text_bindings(&blanket.declaration);
        let substitution = super::source_substitution::SourceTypeSubstitution::receiver(
            blanket.receiver_parameter.clone(),
            receiver.clone(),
        );
        let associated_types = self.with_protocol_source_scope(
            blanket.source_module.clone(),
            substitution,
            Some(receiver.clone()),
            |cg| {
                // A compact binding is in the nominal owner's parameter space. The
                // source row carries general/conditional equations with their premises.
                let Some(owner) = target
                    .and_then(|id| cg.implementation_target_slot(id, name))
                    .map(|slot| &cg.types[slot])
                else {
                    return Ok(Vec::new());
                };
                let ids: Vec<_> = owner
                    .type_params
                    .iter()
                    .map(|parameter| parameter.id)
                    .collect();
                let pattern = if ids.is_empty() {
                    TypeRef::Concrete(owner.id)
                } else {
                    TypeRef::Instantiated {
                        base: owner.id,
                        args: ids.iter().copied().map(TypeRef::Generic).collect(),
                    }
                };
                let mut binding = crate::mono::TypeSubstitution::empty();
                if !crate::mono::match_declared_types(&receiver.ty, &pattern, &mut binding) {
                    return Ok(Vec::new());
                }
                let mut bindings = Vec::new();
                for item in items {
                    let ImplItemKind::Type {
                        name,
                        ty,
                        type_params,
                    } = &item.kind
                    else {
                        continue;
                    };
                    if !type_params.is_empty() {
                        continue;
                    }
                    let Some(ty) = cg.semantic_formal_type(ty, &Default::default(), &ids) else {
                        continue;
                    };
                    let Ok(ty) = binding.apply(&ty) else {
                        continue;
                    };
                    if super::formal_parameters::resolved(&ty, &ids, 0) {
                        bindings.push((StringId(cg.ctx.intern_string_raw(name.as_str())), ty));
                    }
                }
                Ok(bindings)
            },
        )?;
        let index = self.pending_blanket_rows.len();
        self.pending_blanket_rows.push(MaterializedBlanket {
            target,
            receiver: receiver.ty.clone(),
            name: name.to_owned(),
            arguments,
            implementation: ProtocolImpl {
                source_implementation: Some(source.clone()),
                protocol: ProtocolId(protocol.0),
                methods: vec![u32::MAX; members.len()],
                associated_types,
                protocol_args_text,
                type_param_fn_bounds,
            },
            members,
            overridden: false,
        });
        Ok(Some(BlanketSelection {
            index,
            definition: blanket.definition,
        }))
    }

    pub(super) fn select_blanket_rows(&mut self) -> CodegenResult<()> {
        for index in 0..self.pending_blanket_rows.len() {
            let row = &self.pending_blanket_rows[index];
            let protocol = crate::types::TypeId(row.implementation.protocol.0);
            let candidates: Vec<_> = row
                .target
                .and_then(|id| self.implementation_target_slot(id, &row.name))
                .map(|slot| &self.types[slot])
                .into_iter()
                .flat_map(|ty| ty.protocols.iter())
                .filter(|existing| {
                    existing.protocol == row.implementation.protocol
                        && existing.source_implementation
                            != row.implementation.source_implementation
                })
                .filter_map(|existing| existing.source_implementation.clone())
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let receiver = row.receiver.clone();
            let arguments = row.arguments.clone();
            for explicit in candidates {
                if self
                    .proven_member_bounds(&explicit, &receiver, protocol, &arguments)?
                    .is_some()
                {
                    self.pending_blanket_rows[index].overridden = true;
                    break;
                }
            }
        }
        Ok(())
    }

    pub(super) fn attach_blanket_rows(&mut self) -> CodegenResult<()> {
        use super::protocol_attachments::ImplementationAttachment;
        for row in std::mem::take(&mut self.pending_blanket_rows) {
            if row.overridden {
                continue;
            }
            let lifecycle = self.declared_lifecycle_method(
                crate::types::TypeId(row.implementation.protocol.0),
                &row.members,
                &row.implementation.methods,
            )?;
            self.attach_implementation_to_target(
                row.target,
                &row.name,
                ImplementationAttachment::Protocol {
                    implementation: row.implementation,
                    lifecycle,
                },
            )?;
        }
        Ok(())
    }

    /// Two protocol defaults for the same leaf and concrete receiver have
    /// distinct source definitions, even when the same proof triggered both.
    pub(super) fn materialized_method_definition(
        &self,
        method: &super::DeferredDefaultMethod,
    ) -> Option<crate::module::SourceDefinitionIdentity> {
        let mut definition = method.implementation.declaration.definition?;
        if let Some(blanket) = method.blanket_row {
            let qualify = |original: [u8; 32], blanket: [u8; 32]| {
                let mut hash =
                    blake3::Hasher::new_derive_key("Verum materialized blanket definition v1");
                hash.update(&original);
                hash.update(&blanket);
                *hash.finalize().as_bytes()
            };
            definition.digest = qualify(definition.digest, blanket.definition.digest);
            definition.implementation_scope = definition
                .implementation_scope
                .zip(blanket.definition.implementation_scope)
                .map(|(trigger, source)| qualify(trigger, source));
        }
        Some(definition)
    }
}
