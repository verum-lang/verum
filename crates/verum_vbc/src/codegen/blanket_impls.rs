//! Source declarations retained for blanket implementation specialization.
use super::{BlanketImpl, CodegenResult, VbcCodegen};
use verum_ast::{
    decl::{ImplDecl, ImplItemKind, ImplKind},
    ty::{GenericParamKind, PathSegment},
};

impl VbcCodegen {
    /// Both collection paths retain the same declaration and receiver binder.
    pub(super) fn record_blanket_impl(
        &mut self,
        declaration: &ImplDecl,
        source_module: Option<String>,
    ) -> CodegenResult<()> {
        let ImplKind::Protocol {
            protocol, for_type, ..
        } = &declaration.kind
        else {
            return Ok(());
        };
        let Some(PathSegment::Name(derived)) = protocol.segments.last() else {
            return Ok(());
        };
        let Some(receiver_parameter) = Self::for_type_generic_param_name(for_type) else {
            return Ok(());
        };
        let selected_items = self.selected_impl_items(declaration)?;
        for parameter in &declaration.generics {
            let GenericParamKind::Type { name, bounds, .. } = &parameter.kind else {
                continue;
            };
            if name.name.as_str() != receiver_parameter {
                continue;
            }
            for bound in bounds {
                let Some(base_protocol) = Self::type_bound_protocol_name(bound) else {
                    continue;
                };
                let derived_protocol = derived.name.to_string();
                if self.blanket_impls.iter().any(|existing| {
                    existing.base_protocol == base_protocol
                        && existing.derived_protocol == derived_protocol
                }) {
                    continue;
                }
                let explicit_methods = selected_items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        ImplItemKind::Function(function) => Some(function.name.name.to_string()),
                        _ => None,
                    })
                    .collect();
                self.blanket_impls.push(BlanketImpl {
                    base_protocol,
                    derived_protocol,
                    explicit_methods,
                    receiver_parameter: receiver_parameter.clone(),
                    source_module: source_module.clone(),
                    declaration: std::sync::Arc::new(declaration.clone()),
                });
            }
        }
        Ok(())
    }
}
