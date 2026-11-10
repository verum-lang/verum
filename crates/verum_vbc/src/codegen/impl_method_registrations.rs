use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionId, FunctionInfo, VbcCodegen};
use crate::mono::{TypeSubstitution, match_declared_types};
use crate::types::TypeRef;
use verum_ast::Span;

pub(super) struct ImplMethodRegistration {
    pub id: FunctionId,
    pub linkage: String,
    pub receiver: Option<TypeRef>,
    pub declaration: Span,
}

impl VbcCodegen {
    /// Sibling methods may share both name and operand types while implementing
    /// different protocols. Their source declaration chooses the operator body.
    pub(super) fn declared_operator_linkage(
        &self, family: &str, receiver: &TypeRef, operand: Option<&TypeRef>, protocol: &str, member: &str,
    ) -> CodegenResult<Option<String>> {
        let Some(methods) = self.impl_method_registrations.get(family).filter(|rows| rows.len() > 1) else { return Ok(None) };
        let Some(owner) = receiver.base_type_id().and_then(|id| self.type_by_id(id)) else { return Ok(None) };
        let Some(protocol_id) = self.nominal_type_id(protocol) else { return Ok(None) };
        let mut selected = None;
        for row in owner.protocols.iter().filter(|row| row.protocol.0 == protocol_id.0) {
            let Some(source) = &row.source_implementation else { continue };
            let source = source.declaration().map_err(|error| CodegenError::internal(error.to_string()))?;
            for item in &source.items {
                let verum_ast::decl::ImplItemKind::Function(function) = &item.kind else { continue };
                if function.name.as_str() != member { continue; }
                for method in methods.iter().filter(|method| method.declaration == function.span) {
                    let Some(pattern) = &method.receiver else { continue };
                    let mut bindings = TypeSubstitution::empty();
                    if !match_declared_types(pattern, receiver, &mut bindings) { continue; }
                    if let Some(actual) = operand
                        && let Some(input) = self.ctx.semantic_fn_params.get(&method.id)
                            .and_then(|params| params.get(1)).and_then(Option::as_ref)
                        && !match_declared_types(input, actual, &mut bindings)
                    { continue; }
                    if selected.is_some_and(|previous: &ImplMethodRegistration| previous.id != method.id) {
                        return Err(CodegenError::with_span(CodegenErrorKind::InvalidSourceSignature(
                            format!("overloaded operator `{member}` has competing applicable implementations")
                        ), function.span));
                    }
                    selected = Some(method);
                }
            }
        }
        Ok(selected.map(|method| method.linkage.clone()))
    }

    /// Slot order supplies the linkage spelling. Source location and the
    /// declared receiver identify the body, including a deferred default.
    pub(super) fn registered_impl_body(
        &self,
        family: &str,
        source: &verum_ast::FunctionDecl,
    ) -> CodegenResult<Option<(String, FunctionInfo)>> {
        let Some(methods) = self.impl_method_registrations.get(family) else {
            return Ok(None);
        };
        if methods.len() < 2 {
            return Ok(None);
        }
        let receiver = self
            .current_impl_receiver
            .as_ref()
            .map(|witness| &witness.ty);
        let mut candidates = methods.iter().filter(|method| {
            method.declaration == source.span && method.receiver.as_ref() == receiver
        });
        let invalid = || {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(format!(
                    "method body has no unique declaration for {family}"
                )),
                source.span,
            )
        };
        let first = candidates.next().ok_or_else(invalid)?;
        let method = if candidates.next().is_some() {
            let signature = verum_ast::source_signature::capture_source_signature(source, true)
                .map_err(|error| {
                    CodegenError::with_span(
                        CodegenErrorKind::InvalidSourceSignature(error.into_string()),
                        source.span,
                    )
                })?;
            let mut exact = methods.iter().filter(|method| {
                method.receiver.as_ref() == receiver
                    && self
                        .declared_callable_descriptors
                        .get(&method.id)
                        .and_then(|f| f.source_signature.as_ref())
                        == Some(&signature)
            });
            let method = exact.next().ok_or_else(invalid)?;
            if exact.next().is_some() {
                return Err(invalid());
            }
            method
        } else {
            first
        };
        let info = self
            .ctx
            .lookup_function(&method.linkage)
            .filter(|info| info.id == method.id)
            .or_else(|| self.ctx.lookup_function_by_id(method.id))
            .ok_or_else(invalid)?;
        Ok(Some((method.linkage.clone(), info.clone())))
    }

    /// An override for another specialization does not supply this receiver's
    /// default. Imported methods retain their declared self formal in the
    /// receiving module's type space.
    pub(super) fn impl_method_for_receiver(
        &self,
        family: &str,
        receiver: &TypeRef,
    ) -> Option<FunctionId> {
        if let Some(methods) = self.impl_method_registrations.get(family) {
            return methods
                .iter()
                .find(|method| {
                    method.receiver.as_ref().is_some_and(|pattern| {
                        match_declared_types(pattern, receiver, &mut TypeSubstitution::empty())
                    })
                })
                .map(|method| method.id);
        }
        let imported = self.ctx.lookup_function(family)?;
        if !matches!(receiver, TypeRef::Instantiated { .. }) {
            return Some(imported.id);
        }
        if imported.param_names.first().map(String::as_str) != Some("self") {
            return None;
        }
        let mut pattern = self
            .ctx
            .semantic_fn_params
            .get(&imported.id)?
            .first()?
            .as_ref()?;
        while let TypeRef::Reference { inner, .. } = pattern {
            pattern = inner;
        }
        match_declared_types(pattern, receiver, &mut TypeSubstitution::empty())
            .then_some(imported.id)
    }

    pub(super) fn bind_deferred_protocol_slots(&mut self) {
        let pending = std::mem::take(&mut self.pending_protocol_method_slots);
        if pending.is_empty() {
            return;
        }
        let resolved: std::collections::HashMap<_, Vec<_>> = pending
            .into_iter()
            .map(|(source, slots)| {
                let ids = slots
                    .into_iter()
                    .filter_map(|(slot, family, receiver)| {
                        self.impl_method_for_receiver(&family, &receiver)
                            .map(|id| (slot, id.0))
                    })
                    .collect();
                (source, ids)
            })
            .collect();
        for ty in &mut self.types {
            for implementation in &mut ty.protocols {
                let Some(slots) = implementation
                    .source_implementation
                    .as_ref()
                    .and_then(|source| resolved.get(source))
                else {
                    continue;
                };
                for &(slot, id) in slots {
                    if let Some(method) = implementation.methods.get_mut(slot)
                        && *method == u32::MAX
                    {
                        *method = id;
                    }
                }
            }
        }
    }
}
