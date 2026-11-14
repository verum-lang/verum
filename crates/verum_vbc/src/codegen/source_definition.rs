//! One exact source-input witness for declaration collection and body emission.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::module::{FunctionId, SourceDefinitionIdentity};

impl VbcCodegen {
    /// Recollection keeps an ID only when the complete source input agrees.
    /// The implementation selector is an index, never body evidence. A stale
    /// or body-free archive cannot replace the exact definition witness.
    pub(super) fn reusable_impl_definition(
        &self,
        function: &verum_ast::FunctionDecl,
        family: &str,
    ) -> CodegenResult<Option<FunctionId>> {
        if function.body.is_none() {
            return Ok(None);
        }
        let Some(scope) = self
            .current_impl_definition
            .and_then(|definition| definition.implementation_scope)
        else {
            return Ok(None);
        };
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let key = (owner.into(), function.name.name.clone(), scope);
        let mut candidates = self
            .declared_impl_definitions
            .get(&key)
            .cloned()
            .unwrap_or_default();
        // A legacy archive has no definition index. Its exact declared linkage
        // is only a candidate for refusal, never evidence permitting reuse.
        if let Some(info) = self.ctx.lookup_function(family)
            && !candidates.contains(&info.id)
        {
            candidates.push(info.id);
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let Some(definition) =
            self.callable_source_definition(function, self.current_source_callable_parent)?
        else {
            return Ok(None);
        };
        let invalid = || {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(format!(
                    "conflicting or unproved source definition for {family}"
                )),
                function.span,
            )
        };
        let mut selected = None;
        for id in candidates {
            let Some(descriptor) = self.declared_callable_descriptors.get(&id) else {
                continue;
            };
            let actual_owner = descriptor
                .origin_module
                .and_then(|id| self.ctx.strings.get(id.0 as usize));
            let actual_name = descriptor
                .declaration_identity
                .as_ref()
                .and_then(|identity| self.ctx.strings.get(identity.source_name.0 as usize));
            if actual_owner.map(|owner| owner.as_str()) != Some(owner)
                || actual_name.map(|name| name.as_str()) != Some(function.name.name.as_str())
            {
                continue;
            }
            if let Some(previous) = descriptor.source_definition
                && previous.implementation_scope != Some(scope)
            {
                continue;
            }
            let Some(receiver) = self
                .current_impl_receiver
                .as_ref()
                .filter(|receiver| receiver.semantic)
                .map(|receiver| &receiver.ty)
            else {
                return Err(invalid());
            };
            if descriptor.source_receiver.as_ref() != Some(receiver) {
                // This candidate has the same exact implementation selector.
                // A changed resolved receiver is rebinding, not a new source
                // specialization. Missing evidence cannot authorize reuse.
                return Err(invalid());
            }
            if descriptor.source_definition != Some(definition) {
                return Err(invalid());
            }
            if selected.is_some_and(|previous| previous != id) {
                return Err(invalid());
            }
            selected = Some(id);
        }
        Ok(selected)
    }

    pub(super) fn implementation_source_definition(
        &self,
        item: &verum_ast::Item,
    ) -> CodegenResult<SourceDefinitionIdentity> {
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let scopes = self.definition_lexical_scopes(owner)?;
        let mut digest = blake3::Hasher::new();
        verum_ast::source_definition::encode_implementation(
            item,
            owner,
            &scopes,
            verum_ast::source_definition::Limits::default(),
            &mut |bytes| {
                digest.update(bytes);
            },
        )
        .map_err(|error| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(error.to_string()),
                item.span,
            )
        })?;
        let mut scope = blake3::Hasher::new();
        verum_ast::source_definition::encode_implementation_scope(
            item,
            owner,
            verum_ast::source_definition::Limits::default(),
            &mut |bytes| {
                scope.update(bytes);
            },
        )
        .map_err(|error| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(error.to_string()),
                item.span,
            )
        })?;
        Ok(SourceDefinitionIdentity {
            version: verum_ast::source_definition::VERSION,
            digest: *digest.finalize().as_bytes(),
            implementation_scope: Some(*scope.finalize().as_bytes()),
        })
    }

    pub(super) fn callable_source_definition(
        &self,
        function: &verum_ast::FunctionDecl,
        lexical_parent: Option<FunctionId>,
    ) -> CodegenResult<Option<SourceDefinitionIdentity>> {
        if function.body.is_none() {
            return Ok(None);
        }
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(&self.config.module_name);
        let mut scopes = self.definition_lexical_scopes(owner)?;
        if let Some(body_owner) = self
            .default_body_type_scope
            .as_deref()
            .filter(|body_owner| *body_owner != owner)
        {
            scopes.extend(self.definition_lexical_scopes(body_owner)?);
        }
        let parent = match lexical_parent {
            None => None,
            Some(parent) => {
                let Some(identity) = self
                    .declared_callable_descriptors
                    .get(&parent)
                    .and_then(|descriptor| descriptor.source_definition.as_ref())
                else {
                    return Ok(None);
                };
                Some(identity.digest.as_slice())
            }
        };
        let mut digest = blake3::Hasher::new();
        verum_ast::source_definition::encode(
            function,
            owner,
            parent,
            self.current_impl_definition
                .as_ref()
                .map(|identity| identity.digest.as_slice()),
            &scopes,
            verum_ast::source_definition::Limits::default(),
            &mut |bytes| {
                digest.update(bytes);
            },
        )
        .map_err(|error| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(error.to_string()),
                function.span,
            )
        })?;
        Ok(Some(SourceDefinitionIdentity {
            version: verum_ast::source_definition::VERSION,
            digest: *digest.finalize().as_bytes(),
            implementation_scope: self
                .current_impl_definition
                .and_then(|identity| identity.implementation_scope),
        }))
    }
}
