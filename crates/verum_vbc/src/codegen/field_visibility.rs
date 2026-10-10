//! Preserve source field policy independently of its runtime representation.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{DeclaredFieldVisibility, StringId, Visibility};
use verum_ast::{PathSegment, Visibility as AstVisibility};
use verum_common::{List, Text};

impl VbcCodegen {
    pub(super) fn lower_field_visibility(
        &mut self,
        visibility: &AstVisibility,
    ) -> CodegenResult<(Visibility, DeclaredFieldVisibility)> {
        // This is the declaration's source context, also used for origin_module;
        // archive import never runs source normalization in the consumer scope.
        let owner = self
            .ctx
            .current_source_module
            .as_deref()
            .unwrap_or(self.config.module_name.as_str());
        let resolved = visibility
            .resolve_declared_scope(owner)
            .map_err(|message| {
                CodegenError::new(CodegenErrorKind::TypeInference(message.to_string()))
            })?;
        let coarse = match resolved {
            AstVisibility::Public => Visibility::Public,
            AstVisibility::PublicCrate => Visibility::Cog,
            _ => Visibility::Private,
        };
        let declared = match resolved {
            AstVisibility::Public => DeclaredFieldVisibility::Public,
            AstVisibility::Private => DeclaredFieldVisibility::Private,
            AstVisibility::PublicCrate => DeclaredFieldVisibility::Cog,
            AstVisibility::PublicSuper => DeclaredFieldVisibility::Super,
            AstVisibility::Internal => DeclaredFieldVisibility::Internal,
            AstVisibility::Protected => DeclaredFieldVisibility::Protected,
            AstVisibility::PublicIn(path) => {
                let names: List<Text> = path
                    .segments
                    .iter()
                    .map(|segment| {
                        let PathSegment::Name(ident) = segment else {
                            unreachable!("resolved scopes contain only names");
                        };
                        ident.name.clone()
                    })
                    .collect();
                DeclaredFieldVisibility::In(StringId(self.ctx.intern_string_raw(&names.join("."))))
            }
        };
        Ok((coarse, declared))
    }
}
