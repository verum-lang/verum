use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::receiver_methods::{pointee, select};
use crate::types::TypeRef;

impl VbcCodegen {
    /// A receiver is a source parameter role, not a display name in FunctionInfo.
    pub(super) fn has_declared_method_receiver(&self, function: &FunctionInfo) -> CodegenResult<bool> {
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
            // A synthesized structural catalogue can be registered under its
            // local name. It may supply a method only while that name still
            // denotes this exact descriptor in the caller's type table.
            let key = self.registered_receiver_method(&owner, method).or_else(|| {
                let name = self.ctx.strings.get(catalogue.name.0 as usize)?;
                if self.type_name_to_id.get(name.as_str()) != Some(&catalogue.id) {
                    return None;
                }
                self.registered_receiver_method(name, method)
            });
            let Some(key) = key else { continue };
            let specialized = self.receiver_specialization(&key, Some(actual), arity, None)?;
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
        source: Option<&(verum_common::Text, verum_ast::Span)>,
    ) -> CodegenResult<Option<&FunctionInfo>> {
        self.receiver_role_specialization(family, receiver, arity, true, source)
    }

    pub(super) fn static_receiver_specialization(
        &self, family: &str, receiver: &TypeRef, arity: usize,
    ) -> CodegenResult<Option<&FunctionInfo>> {
        self.receiver_role_specialization(family, Some(receiver), arity, false, None)
    }

    fn receiver_role_specialization(
        &self, family: &str, receiver: Option<&TypeRef>, arity: usize, takes_receiver: bool,
        source: Option<&(verum_common::Text, verum_ast::Span)>,
    ) -> CodegenResult<Option<&FunctionInfo>> {
        let actual = receiver.map(pointee);
        let has_receiver_family = match actual {
            Some(TypeRef::Concrete(_)) => !takes_receiver,
            Some(TypeRef::Instantiated { .. } | TypeRef::Array { .. } | TypeRef::Slice(_)) => true,
            _ => false,
        };
        if source.is_none() && !has_receiver_family { return Ok(None); }
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
        if let Some((owner, site)) = source {
            // The same receiver can implement several same-named methods.
            // The checked source location distinguishes their linkage slots.
            let member = family.rsplit('.').next().unwrap_or(family);
            let mut matches = siblings.filter(|candidate| {
                let Some(descriptor) = self.declared_callable_descriptors.get(&candidate.id) else {
                    return false;
                };
                let origin = descriptor.origin_module.and_then(|id| self.ctx.strings.get(id.0 as usize));
                candidate.param_count == arity && origin.is_some_and(|name| name.as_str() == owner.as_str())
                    && descriptor.source_signature.as_ref().is_some_and(|signature|
                        signature.is_method() && signature.name.as_str() == member
                        && signature.span.start == site.start && signature.span.end == site.end)
            });
            return match (matches.next(), matches.next()) {
                (Some(selected), None) => Ok(Some(selected)),
                _ => Err(CodegenError::with_span(CodegenErrorKind::InvalidSourceSignature(
                    format!("no unique implementation of {family} at the checked declaration in {owner}"),
                ), *site)),
            };
        }
        let Some(actual) = actual else { return Ok(None); };
        let mut candidates = verum_common::List::new();
        for candidate in siblings.filter(|candidate| candidate.param_count == arity) {
            if self.has_declared_method_receiver(candidate)? != takes_receiver {
                continue;
            }
            let pattern = if takes_receiver { self
                .ctx
                .semantic_fn_params
                .get(&candidate.id)
                .and_then(|params| params.first()?.as_ref())
            } else { self.declared_callable_descriptors.get(&candidate.id)
                .and_then(|descriptor| descriptor.source_receiver.as_ref()) };
            candidates.push((candidate.id, candidate, pattern));
        }
        select(family, actual, candidates.into_iter())
            .map_err(|message| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message)))
    }
}
