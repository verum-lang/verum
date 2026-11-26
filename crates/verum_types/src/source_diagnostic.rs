//! Source ownership for semantic diagnostics shared by batch and editor analysis.
use verum_ast::{ItemKind, Module};
use verum_common::List;
use verum_diagnostics::Diagnostic;

impl crate::TypeError {
    /// Attach the function owning the original span before a client projects
    /// it onto its own source map. Generated and foreign spans have no owner here.
    pub fn source_diagnostic(&self, module: &Module, render: impl FnOnce(&Self) -> Diagnostic) -> Diagnostic {
        let diagnostic = render(self);
        match enclosing_function(module, self.span()) {
            Some(name) => diagnostic.add_note(format!("while checking function `{name}`")),
            None => diagnostic,
        }
    }
}

fn enclosing_function(module: &Module, span: verum_ast::span::Span) -> Option<&str> {
    use verum_ast::decl::{ImplItemKind, ProtocolItemKind, TypeDeclBody};

    if span.is_empty() {
        return None;
    }
    let mut pending: List<_> = module.items.iter().collect();
    while let Some(item) = pending.pop() {
        if !item.span.contains(span) {
            continue;
        }
        let protocol_items = match &item.kind {
            ItemKind::Function(function) => return Some(function.name.as_str()),
            ItemKind::Module(module) => {
                if let Some(items) = &module.items {
                    pending.extend(items.iter());
                }
                None
            }
            ItemKind::Impl(implementation) => {
                for item in &implementation.items {
                    if item.span.contains(span)
                        && let ImplItemKind::Function(function) = &item.kind
                    {
                        return Some(function.name.as_str());
                    }
                }
                None
            }
            ItemKind::Protocol(protocol) => Some(&protocol.items),
            ItemKind::Type(declaration) => match &declaration.body {
                TypeDeclBody::Protocol(protocol) => Some(&protocol.items),
                _ => None,
            },
            _ => None,
        };
        if let Some(items) = protocol_items {
            for item in items {
                if item.span.contains(span)
                    && let ProtocolItemKind::Function { decl, .. } = &item.kind
                {
                    return Some(decl.name.as_str());
                }
            }
        }
    }
    None
}
