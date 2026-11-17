//! Pattern payload facts come from the scrutinee's declaration telescope.
//! Rendering a hidden parameter is unnecessary for preserving its identity.
use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::types::{TypeKind, TypeRef};

impl VbcCodegen {
    pub(super) fn pattern_variant_payload_declaration(
        &self,
        declaration: Option<&TypeRef>,
        path: &str,
        field: u32,
    ) -> CodegenResult<Option<TypeRef>> {
        let Some(declaration) = declaration else {
            return Ok(None);
        };
        let Some(owner) = declaration
            .base_type_id()
            .and_then(|id| self.type_by_id(id))
        else {
            return Ok(None);
        };
        if owner.kind != TypeKind::Sum {
            return Ok(None);
        }
        let path = self.resolve_variant_path_alias(&path.replace("::", "."));
        let variant = self.scrutinee_variant(owner, &path)
            .ok_or_else(|| {
                CodegenError::unsupported_expr(
                    "variant pattern does not belong to the scrutinee declaration",
                )
            })?;
        self.projected_variant_field(declaration, variant.tag, field)
            .map(Some)
            .ok_or_else(|| {
                CodegenError::unsupported_expr(
                    "pattern payload has no resolved declaration in its source scope",
                )
            })
    }
}
