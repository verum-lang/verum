use verum_ast::{
    FileId, ItemKind, Module,
    decl::{ImplItemKind, ImplKind, MountTreeKind, ProtocolItem, ProtocolItemKind, TypeDeclBody},
    pretty::format_type,
};
use verum_fast_parser::FastParser;
pub fn source_unit(path: &str, symbols: &str, index: usize) -> Module {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../core")
            .join(path),
    )
    .unwrap();
    let mut unit = FastParser::new()
        .parse_module_str(&source, FileId::new(index as u32 + 1))
        .unwrap();
    let selected = |name: &str| symbols.split_whitespace().any(|symbol| symbol == name);
    let keep_requirement = |owner: &str, item: &ProtocolItem| {
        let prefix = format!("{owner}#");
        if !symbols.split_whitespace().any(|symbol| symbol.starts_with(&prefix)) {
            return true;
        }
        match &item.kind {
            ProtocolItemKind::Function { decl, .. } => selected(&format!("{prefix}{}", decl.name)),
            _ => true,
        }
    };
    unit.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) if path == "mod.vr" && decl.name.as_str() == "prelude" && decl.items.is_some() => {
            decl.items.as_mut().unwrap().retain(|entry| matches!(entry.kind, ItemKind::Mount(_)));
            true
        }
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Mount(decl) if matches!(path, "base/mod.vr" | "intrinsics/mod.vr") => {
            matches!(&decl.tree.kind, MountTreeKind::Nested { trees, .. } if trees.iter().any(|tree|
                matches!(&tree.kind, MountTreeKind::Path(path) if selected(path.last_segment_name()))
            ))
        }
        ItemKind::Mount(_) => true,
        ItemKind::Const(decl) => selected(decl.name.as_str()) || selected("*const"),
        ItemKind::Static(decl) => selected(decl.name.as_str()),
        ItemKind::ExternBlock(block) => {
            block.functions.retain(|function| selected(function.name.as_str()));
            !block.functions.is_empty()
        }
        ItemKind::Type(decl) => {
            if let TypeDeclBody::Protocol(body) = &mut decl.body {
                body.items.retain(|item| keep_requirement(decl.name.as_str(), item));
            }
            selected(decl.name.as_str())
        }
        ItemKind::Protocol(decl) => {
            decl.items.retain(|item| keep_requirement(decl.name.as_str(), item));
            selected(decl.name.as_str())
        }
        ItemKind::Function(decl) => selected(decl.name.as_str()),
        ItemKind::Impl(decl) => {
            let owner = match &decl.kind {
                ImplKind::Inherent(ty) => format_type(ty).replace(" ", "").to_string(),
                ImplKind::Protocol { protocol, for_type, .. } => format!("{protocol}@{}", format_type(for_type).replace(" ", "")),
            };
            let keep_method = |f: &verum_ast::FunctionDecl| selected(&format!("{owner}.{}", f.name));
            let keep_impl = decl.items.iter().any(|item| matches!(&item.kind, ImplItemKind::Function(f) if keep_method(f)));
            if keep_impl {
                decl.items.retain(|item| match &item.kind {
                    ImplItemKind::Function(f) => keep_method(f),
                    _ => true,
                });
            }
            keep_impl
        }
        _ => false,
    });
    unit
}
