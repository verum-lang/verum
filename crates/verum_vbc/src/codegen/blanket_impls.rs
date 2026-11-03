//! Source declarations retained for blanket implementation specialization.
use super::{BlanketImpl, VbcCodegen};
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
    ) {
        let ImplKind::Protocol {
            protocol, for_type, ..
        } = &declaration.kind
        else {
            return;
        };
        let Some(PathSegment::Name(derived)) = protocol.segments.last() else {
            return;
        };
        let Some(receiver_parameter) = Self::for_type_generic_param_name(for_type) else {
            return;
        };
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
                let body_methods: Vec<_> = declaration
                    .items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        ImplItemKind::Function(function) => Some(function.clone()),
                        _ => None,
                    })
                    .collect();
                self.blanket_impls.push(BlanketImpl {
                    base_protocol,
                    derived_protocol,
                    explicit_methods: body_methods
                        .iter()
                        .map(|function| function.name.name.to_string())
                        .collect(),
                    body_methods,
                    receiver_parameter: receiver_parameter.clone(),
                    source_module: source_module.clone(),
                    declaration: std::sync::Arc::new(declaration.clone()),
                });
            }
        }
    }
}
