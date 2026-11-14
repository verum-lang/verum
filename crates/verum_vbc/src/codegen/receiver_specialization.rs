use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::receiver_methods::{pointee, select};
use crate::types::TypeRef;

impl VbcCodegen {
    /// A receiver is a source parameter role, not a display name in FunctionInfo.
    fn has_declared_method_receiver(&self, function: &FunctionInfo) -> CodegenResult<bool> {
        self.declared_callable_descriptors
            .get(&function.id)
            .and_then(|descriptor| descriptor.source_signature.as_ref())
            .map(|signature| signature.is_method())
            .ok_or_else(|| {
                CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
                    "missing declared receiver role for {:?}",
                    function.id
                )))
            })
    }
    /// Structural values retain their element and extent, while a descriptor
    /// catalogues their methods. Resolve that catalogue by kind and match its
    /// declared receiver; rendered array/slice names are not type evidence.
    pub(super) fn structural_receiver_method_key(
        &self,
        receiver: &TypeRef,
        method: &str,
        arity: usize,
    ) -> CodegenResult<Option<String>> {
        let actual = pointee(receiver);
        let kind = match actual {
            TypeRef::Array { .. } => crate::types::TypeKind::Array,
            TypeRef::Slice(_) => crate::types::TypeKind::Slice,
            _ => return Ok(None),
        };
        let mut selected: Option<(crate::module::FunctionId, String)> = None;
        let mut declared = false;
        for catalogue in self.types.iter().filter(|catalogue| catalogue.kind == kind) {
            let Some(owner) = self.type_declaration_identity(catalogue) else {
                continue;
            };
            let Some(key) = self.registered_receiver_method(&owner, method) else {
                continue;
            };
            let specialized = self.receiver_specialization(&key, Some(actual), arity)?;
            let Some(function) = specialized.or_else(|| self.ctx.lookup_qualified_function(&key))
            else {
                continue;
            };
            declared = true;
            if function.param_count != arity || !self.has_declared_method_receiver(function)? {
                continue;
            }
            let Some(pattern) = self
                .ctx
                .semantic_fn_params
                .get(&function.id)
                .and_then(|parameters| parameters.first()?.as_ref())
            else {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    format!("missing structural receiver declaration for {key}"),
                )));
            };
            if !crate::mono::match_declared_types(
                pointee(pattern),
                actual,
                &mut crate::mono::TypeSubstitution::empty(),
            ) {
                continue;
            }
            if selected.as_ref().is_some_and(|(id, _)| *id != function.id) {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    format!(
                        "receiver {actual:?} does not select a unique implementation of {method}"
                    ),
                )));
            }
            selected = Some((function.id, key));
        }
        if declared && selected.is_none() {
            return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                format!("no declared structural receiver of {method} matches {actual:?}"),
            )));
        }
        Ok(selected.map(|(_, key)| key))
    }

    /// Sibling linkage slots identify bodies, but their ordinal says nothing
    /// about applicability. Match the declaration-owned self type in the
    /// caller's type space, including remapped archive parameter declarations.
    pub(super) fn receiver_specialization(
        &self,
        family: &str,
        receiver: Option<&TypeRef>,
        arity: usize,
    ) -> CodegenResult<Option<&FunctionInfo>> {
        let Some(
            actual @ (TypeRef::Instantiated { .. } | TypeRef::Array { .. } | TypeRef::Slice(_)),
        ) = receiver.map(pointee)
        else {
            return Ok(None);
        };
        let functions = &self.ctx.functions;
        let Some(second) = functions.get(&format!("{family}#impl2")) else {
            return Ok(None);
        };
        let rest = (3..).map_while(|ordinal| functions.get(&format!("{family}#impl{ordinal}")));
        let siblings = functions
            .get(family)
            .into_iter()
            .chain([second])
            .chain(rest);
        let mut candidates = verum_common::List::new();
        for candidate in siblings.filter(|candidate| candidate.param_count == arity) {
            if !self.has_declared_method_receiver(candidate)? {
                continue;
            }
            let pattern = self
                .ctx
                .semantic_fn_params
                .get(&candidate.id)
                .and_then(|params| params.first()?.as_ref());
            candidates.push((candidate.id, candidate, pattern));
        }
        select(family, actual, candidates.into_iter())
            .map_err(|message| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message)))
    }
}
