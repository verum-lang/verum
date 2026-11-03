//! Implementation metadata follows the exact target declaration even when an
//! implementation precedes its layout. Pending facts never create descriptors.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{ProtocolImpl, TypeDescriptor, TypeId};
use verum_ast::decl::ImplKind;

impl VbcCodegen {
    pub(super) fn attach_declared_protocol_impl(
        &mut self,
        implementation: &ImplKind,
        name: &str,
        fact: ProtocolImpl,
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
                self.types[slot].register_implementation(fact);
                return Ok(());
            }
            if self.source_generic_rosters.contains_key(&id) {
                // The source header declares this ID. Preserve the complete
                // record unchanged until its real descriptor is registered.
                let pending = self.pending_protocol_attachments.entry(id).or_default();
                if !pending.contains(&fact) {
                    pending.push(fact);
                }
                return Ok(());
            }
        }
        self.attach_impl_to_undeclared_target(name, fact)
    }

    pub(super) fn take_pending_protocol_attachments(&mut self, target: &mut TypeDescriptor) {
        if let Some(facts) = self.pending_protocol_attachments.remove(&target.id) {
            for fact in facts {
                target.register_implementation(fact);
            }
        }
    }

    pub(super) fn check_pending_protocol_attachments(&self) -> CodegenResult<()> {
        let Some(id) = self
            .pending_protocol_attachments
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
