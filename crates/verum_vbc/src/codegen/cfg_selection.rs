//! Target selection shared by implementation registration and materialization.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use verum_ast::{
    Attribute,
    decl::{ImplDecl, ImplItem, ImplItemKind},
};

impl VbcCodegen {
    /// Preserve the original declaration while projecting the items available
    /// to this compilation target. Validate every cfg, including those after
    /// an already-false condition, before publishing any selected evidence.
    pub(super) fn selected_impl_items<'a>(
        &self,
        declaration: &'a ImplDecl,
    ) -> CodegenResult<Vec<&'a ImplItem>> {
        let mut selected = Vec::with_capacity(declaration.items.len());
        for item in &declaration.items {
            let function_attributes: &[Attribute] = match &item.kind {
                ImplItemKind::Function(function) => &function.attributes,
                _ => &[],
            };
            let mut include = true;
            for attribute in item.attributes.iter().chain(function_attributes) {
                let (active, failures) = self
                    .cfg_evaluator
                    .should_include_with_failures(std::slice::from_ref(attribute));
                if !failures.is_empty() {
                    return Err(CodegenError::with_span(
                        CodegenErrorKind::InvalidSourceSignature(
                            "invalid @cfg on implementation item".into(),
                        ),
                        attribute.span,
                    ));
                }
                include &= active;
            }
            if include {
                selected.push(item);
            }
        }
        Ok(selected)
    }
}
