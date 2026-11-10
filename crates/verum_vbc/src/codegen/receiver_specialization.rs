use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::receiver_methods::{pointee, select};
use crate::types::TypeRef;

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
        let candidates = siblings
            .filter(|candidate| {
                candidate.param_count == arity
                    && candidate.param_names.first().map(String::as_str) == Some("self")
            })
            .map(|candidate| {
                let pattern = self
                    .ctx
                    .semantic_fn_params
                    .get(&candidate.id)
                    .and_then(|params| params.first()?.as_ref());
                (candidate.id, candidate, pattern)
            });
        select(family, actual, candidates)
            .map_err(|message| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message)))
    }
}
