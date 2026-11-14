//! Implementation metadata follows the exact target declaration even when an
//! implementation precedes its layout. Pending facts never create descriptors.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{ProtocolImpl, TypeDescriptor, TypeId};
use verum_ast::decl::ImplKind;
use verum_common::{Maybe, well_known_types::WellKnownProtocol};

#[derive(Clone, PartialEq)]
pub(super) enum LifecycleMethod {
    Drop(crate::module::FunctionId),
    Clone(crate::module::FunctionId),
}

#[derive(Clone, PartialEq)]
pub(super) enum ImplementationAttachment {
    Protocol {
        implementation: ProtocolImpl,
        lifecycle: Maybe<LifecycleMethod>,
    },
    Inherent(verum_ast::source_implementation::AssociatedImplementationSource),
}
impl ImplementationAttachment {
    pub(super) fn attach(self, target: &mut TypeDescriptor) {
        match self {
            Self::Protocol { implementation, lifecycle } => {
                match lifecycle {
                    Some(LifecycleMethod::Drop(method)) => target.drop_fn = Some(method.0),
                    Some(LifecycleMethod::Clone(method)) => target.clone_fn = Some(method.0),
                    None => (),
                }
                target.register_implementation(implementation);
            }
            Self::Inherent(source) => {
                if !target.source_inherent_implementations.contains(&source) {
                    target.source_inherent_implementations.push(source);
                }
            }
        }
    }
}
/// A bootstrap interface can contribute the same source row with provisional
/// function IDs. Only a complete local definition of that very declaration can
/// replace it. Distinct local bodies and conflicting associated facts stay visible.
pub(super) fn finish_protocol_slots(
    target: &mut TypeDescriptor, functions: &std::collections::HashMap<u32, u32>,
) {
    let local = |id: &u32| *id != u32::MAX && functions.contains_key(id);
    let completed = |row: &ProtocolImpl| !row.methods.is_empty() && row.methods.iter().all(local);
    if target.protocols.len() > 1 && target.protocols.iter().any(|row| !completed(row)) {
        let mut definitions: std::collections::HashMap<_, Vec<_>> = std::collections::HashMap::new();
        for row in target.protocols.iter().filter(|row| completed(row)) {
            if let Some(source) = &row.source_implementation {
                definitions.entry((row.protocol, source)).or_default().push(row);
            }
        }
        let keep: Vec<_> = target.protocols.iter().map(|row| {
            if completed(row) || row.methods.is_empty() { return true; }
            let matches = row.source_implementation.as_ref()
                .and_then(|source| definitions.get(&(row.protocol, source)));
            !matches.is_some_and(|candidates| candidates.iter().any(|definition| {
                row.protocol_args_text == definition.protocol_args_text
                    && row.associated_types == definition.associated_types
                    && row.type_param_fn_bounds == definition.type_param_fn_bounds
                    && row.methods.len() == definition.methods.len()
                    && row.methods.iter().zip(&definition.methods)
                        .all(|(before, after)| !local(before) || before == after)
            }))
        }).collect();
        let mut keep = keep.into_iter();
        target.protocols.retain(|_| keep.next().expect("one decision per protocol row"));
    }
    for row in &mut target.protocols {
        for slot in &mut row.methods {
            if let Some(&id) = functions.get(slot) { *slot = id; }
        }
    }
}

impl VbcCodegen {
    /// A protocol's language role belongs to its defining declaration. Mount
    /// aliases and archive bundle parents cannot establish that identity.
    pub(super) fn declared_protocol_role(&self, id: TypeId) -> Maybe<WellKnownProtocol> {
        let declaration = self.type_by_id(id)?;
        if declaration.kind != crate::types::TypeKind::Protocol { return None; }
        let identity = self.type_declaration_identity(declaration)?;
        let (owner, name) = identity.rsplit_once('.')?;
        let role = WellKnownProtocol::from_name(name)?;
        (role.canonical_archive_modules().first().copied() == Some(owner)).then_some(role)
    }

    /// Lifecycle members belong to their canonical source protocol, even
    /// through renamed mounts. Recording one does not grant permission to
    /// execute a forward descriptor or invoke user Clone at a value-copy site.
    pub(super) fn declared_lifecycle_method(
        &self, protocol: TypeId, names: &[impl AsRef<str>], methods: &[u32],
    ) -> CodegenResult<Maybe<LifecycleMethod>> {
        let (member, constructor): (&str, fn(crate::module::FunctionId) -> LifecycleMethod) =
            match self.declared_protocol_role(protocol) {
                Some(WellKnownProtocol::Drop) => ("drop", LifecycleMethod::Drop),
                Some(WellKnownProtocol::Clone) => ("clone", LifecycleMethod::Clone),
                _ => return Ok(None),
            };
        let slot = names.iter().position(|name| name.as_ref() == member)
            .ok_or_else(|| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                format!("canonical lifecycle protocol has no declared {member} member"),
            )))?;
        let id = methods.get(slot).copied().filter(|id| *id != u32::MAX)
            .ok_or_else(|| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                format!("canonical lifecycle implementation has no selected {member} body"),
            )))?;
        Ok(Some(constructor(crate::module::FunctionId(id))))
    }

    pub(super) fn attach_declared_implementation(
        &mut self,
        implementation: &ImplKind,
        name: &str,
        fact: ImplementationAttachment,
    ) -> CodegenResult<()> {
        if let Some(id) = self.implementation_nominal_id(implementation) {
            // Scalar aliases may share a runtime ID; their source spelling
            // still owns their implementation metadata.
            let scalar = TypeId::from_well_known_scalar_name(name) == Some(id);
            let slot = self.types.iter().position(|ty| {
                ty.id == id
                    && (!scalar
                        || ((ty.canonical_scalar_type_id(name) == Some(id)
                            || self.canonical_runtime_record_id(ty) == Some(id))
                            && self.ctx.strings.get(ty.name.0 as usize).map(String::as_str)
                                == Some(name)))
            });
            if let Some(slot) = slot {
                fact.attach(&mut self.types[slot]);
                return Ok(());
            }
            if self.source_generic_rosters.contains_key(&id) {
                // The source header declares this ID. Preserve the complete
                // record unchanged until its real descriptor is registered.
                let pending = self.pending_implementation_attachments.entry(id).or_default();
                if !pending.contains(&fact) {
                    pending.push(fact);
                }
                return Ok(());
            }
        }
        self.attach_impl_to_undeclared_target(name, fact)
    }

    pub(super) fn take_pending_implementation_attachments(&mut self, target: &mut TypeDescriptor) {
        if let Some(facts) = self.pending_implementation_attachments.remove(&target.id) {
            for fact in facts {
                fact.attach(target);
            }
        }
    }

    pub(super) fn check_pending_implementation_attachments(&self) -> CodegenResult<()> {
        let Some(id) = self
            .pending_implementation_attachments
            .keys()
            .min_by_key(|id| id.0)
        else {
            return Ok(());
        };
        Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
            format!(
                "protocol implementation target {} was declared but has no completed descriptor",
                id.0,
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ProtocolId, StringId, TypeRef};
    use verum_ast::{ItemKind, source_implementation::AssociatedImplementationSource};
    use verum_fast_parser::Parser;

    #[test]
    fn a_local_definition_only_completes_its_own_provisional_slots() {
        let ast = Parser::new("implement Protocol for Receiver { type Item = Int; }")
            .parse_module().unwrap();
        let ItemKind::Impl(declaration) = &ast.items[0].kind else { panic!("implementation") };
        let source = |owner: &str| AssociatedImplementationSource::new(owner.into(), declaration).unwrap();
        let row = |owner: &str, methods: &[u32], item| ProtocolImpl {
            protocol: ProtocolId(91),
            source_implementation: Some(source(owner)),
            methods: methods.to_vec(),
            associated_types: vec![(StringId(9), TypeRef::Concrete(item))],
            protocol_args_text: vec![],
            type_param_fn_bounds: vec![],
        };
        let local = std::collections::HashMap::from([(20, 0), (21, 1), (22, 2)]);
        let mut receiver = TypeDescriptor::default();
        receiver.protocols.extend([
            row("source", &[20, 100], TypeId::INT),
            row("source", &[20, 21], TypeId::INT),
            row("source", &[22, 100], TypeId::INT),
            row("source", &[20, 100], TypeId::BOOL),
            row("other", &[20, 100], TypeId::INT),
        ]);
        finish_protocol_slots(&mut receiver, &local);
        let result: Vec<_> = receiver.protocols.iter().map(|p| p.methods.as_slice()).collect();
        assert_eq!(result, [&[0, 1][..], &[2, 100], &[0, 100], &[0, 100]]);
    }
}
