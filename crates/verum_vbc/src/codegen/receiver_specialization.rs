use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::mono::{TypeSubstitution, match_declared_types};
use crate::types::TypeRef;

fn pointee(mut ty: &TypeRef) -> &TypeRef {
    while let TypeRef::Reference { inner, .. } = ty {
        ty = inner;
    }
    ty
}

impl VbcCodegen {
    /// Sibling linkage slots identify bodies, but their ordinal says nothing
    /// about applicability. Match the declaration-owned self type in the
    /// caller's type space, including remapped archive parameter declarations.
    pub(super) fn receiver_specialization(
        &self,
        family: &str,
        receiver: Option<&TypeRef>,
        arity: usize,
    ) -> CodegenResult<Option<&FunctionInfo>> {
        let Some(actual @ TypeRef::Instantiated { .. }) = receiver.map(pointee) else {
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
        let mut first_pattern = None;
        let mut distinct = false;
        let mut selected = None;
        let mut ambiguous = false;
        for candidate in siblings {
            if candidate.param_count != arity
                || candidate.param_names.first().map(String::as_str) != Some("self")
            {
                continue;
            }
            let pattern = self
                .ctx
                .semantic_fn_params
                .get(&candidate.id)
                .and_then(|params| params.first()?.as_ref())
                .map(pointee);
            let Some(pattern) = pattern else {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    format!(
                        "missing receiver declaration for {family} implementation {}",
                        candidate.id.0
                    ),
                )));
            };
            distinct |= first_pattern.is_some_and(|first| first != pattern);
            first_pattern.get_or_insert(pattern);
            if match_declared_types(pattern, actual, &mut TypeSubstitution::empty()) {
                ambiguous |=
                    selected.is_some_and(|previous: &FunctionInfo| previous.id != candidate.id);
                selected = Some(candidate);
            }
        }
        if !distinct {
            // Receiver type alone does not distinguish other overload axes.
            return Ok(None);
        }
        if ambiguous || selected.is_none() {
            return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                format!("receiver {actual:?} does not select a unique implementation of {family}"),
            )));
        }
        Ok(selected)
    }
}
