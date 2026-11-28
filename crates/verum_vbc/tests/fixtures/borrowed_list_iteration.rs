#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    module_with_client(include_str!("borrowed_list_iteration.vr"))
}

pub fn module_with_client(source: &str) -> verum_vbc::VbcModule {
    use verum_ast::{
        FileId, ItemKind,
        decl::{ProtocolItemKind, TypeDeclBody},
    };
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

    let selection = [
        ("base/maybe.vr", "Maybe"),
        (
            "base/iterator.vr",
            "Iterator IntoIterator DoubleEndedIterator ExactSizeIterator",
        ),
        ("base/protocols.vr", "Drop"),
        ("base/mod.vr", "Maybe"),
        ("mod.vr", ""),
        ("intrinsics/memory.vr", "list_storage_read list_storage_resize list_storage_stride list_storage_write"),
        ("intrinsics/arithmetic.vr", "checked_mul"),
        ("base/primitives.vr", "Int.checked_mul"),
        (
            "collections/list.vr",
            "List list_max_len List<T>.push List<T>.grow List<T>.next_cap List<T>.resize_buffer List<T>.len ListIter Drop@List<T>.drop List<T>.clear List<T>.truncate List<T>.free_buffer List<T>.iter Iterator@ListIter<T>.next Iterator@ListIter<T>.size_hint IntoIterator@ListIter<T>.into_iter DoubleEndedIterator@ListIter<T>.next_back ExactSizeIterator@ListIter<T>.len",
        ),
    ];
    let mut units: verum_common::List<_> = selection
        .iter()
        .enumerate()
        .map(|(index, (path, symbols))| {
            let mut unit = selected_stdlib::source_unit(path, symbols, index);
            if *path == "base/iterator.vr" {
                for item in &mut unit.items {
                    let items = match &mut item.kind {
                        ItemKind::Type(decl) => match &mut decl.body {
                            TypeDeclBody::Protocol(body) => &mut body.items,
                            _ => continue,
                        },
                        ItemKind::Protocol(decl) => &mut decl.items,
                        _ => continue,
                    };
                    items.retain(|item| match &item.kind {
                        ProtocolItemKind::Function { decl, .. } => matches!(
                            decl.name.as_str(),
                            "next" | "size_hint" | "into_iter" | "next_back" | "len"
                        ),
                        _ => true,
                    });
                }
            }
            unit
        })
        .collect();
    let client = FastParser::new()
        .parse_module_str(source, FileId::new(100))
        .unwrap();
    units.last_mut().unwrap().items.extend(client.items);
    let inputs: verum_common::List<_> = units.iter().collect();
    let mut generator = VbcCodegen::new();
    generator.initialize();
    generator.collect_unit_declarations(&inputs).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&inputs, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = generator.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}
