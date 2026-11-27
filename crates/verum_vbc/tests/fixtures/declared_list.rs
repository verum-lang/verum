use verum_ast::{
    FileId, ItemKind, Module,
    decl::{ImplItemKind, ImplKind},
};
use verum_common::List;
use verum_fast_parser::FastParser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    serialize::serialize_module,
};

fn select(mut module: Module, declarations: &[&str], methods: Option<(&str, &[&str])>) -> Module {
    module.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(header) => header.items.is_none(),
        ItemKind::Mount(_) => true,
        ItemKind::Function(f) => declarations.contains(&f.name.name.as_str()),
        ItemKind::Impl(block) => {
            let Some((owner, names)) = methods else {
                return false;
            };
            let receiver = match &block.kind {
                ImplKind::Inherent(receiver) => receiver,
                ImplKind::Protocol {
                    protocol, for_type, ..
                } if protocol.to_string() == "Drop" => for_type,
                _ => return false,
            };
            if verum_ast::pretty::format_type(receiver) != owner {
                return false;
            }
            block.items.retain(|item| {
                matches!(&item.kind, ImplItemKind::Function(f)
                if names.contains(&f.name.name.as_str()))
            });
            !block.items.is_empty()
        }
        kind => kind
            .type_or_protocol_name()
            .is_some_and(|name| declarations.contains(&name.name.as_str())),
    });
    module
}

pub fn compile_consumer(source: &str) -> VbcModule {
    let parser = FastParser::new();
    let mut next_file = 0;
    let mut parse = |text: &str| {
        next_file += 1;
        parser
            .parse_module_str(text, FileId::new(next_file))
            .expect("source grammar")
    };
    // Standard signatures refer to List and Maybe through the public prelude.
    let mut core = parse(include_str!("../../../../core/mod.vr"));
    core.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Mount(_) => true,
        ItemKind::Module(header) if header.items.is_none() => true,
        ItemKind::Module(header) if header.name.name == "prelude" => {
            header
                .items
                .as_mut()
                .unwrap()
                .retain(|item| matches!(item.kind, ItemKind::Mount(_)));
            true
        }
        _ => false,
    });
    let list = select(
        parse(include_str!("../../../../core/collections/list.vr")),
        &["List", "list_max_len"],
        Some((
            "List<T>",
            &[
                "new",
                "len",
                "push",
                "grow",
                "next_cap",
                "resize_buffer",
                "drop",
                "clear",
                "truncate",
                "pop",
                "free_buffer",
            ],
        )),
    );
    let units = [
        core,
        select(
            parse(include_str!("../../../../core/base/protocols.vr")),
            &["Drop"],
            None,
        ),
        select(
            parse(include_str!("../../../../core/base/maybe.vr")),
            &["Maybe"],
            Some(("Maybe<T>", &["is_some"])),
        ),
        select(
            parse(include_str!("../../../../core/intrinsics/arithmetic.vr")),
            &["checked_mul"],
            None,
        ),
        select(
            parse(include_str!("../../../../core/base/primitives.vr")),
            &[],
            Some(("Int", &["checked_mul"])),
        ),
        select(
            parse(include_str!("../../../../core/intrinsics/memory.vr")),
            &[
                "list_storage_read",
                "list_storage_write",
                "list_storage_stride",
                "list_storage_resize",
            ],
            None,
        ),
        list,
        parse(source),
    ];
    let units: List<_> = units.iter().collect();
    // Both core and consumer are top-level modules in the source namespace.
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(""));
    codegen.initialize();
    codegen
        .collect_unit_declarations(&units)
        .expect("source declarations");
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .expect("compile source");
    let mut module = codegen.finalize_module().expect("completed source module");
    module.resolve_protocol_dispatch();
    module
}

pub fn source_and_wire(source: VbcModule) -> [VbcModule; 2] {
    let bytes = serialize_module(&source).expect("encode");
    let wire = deserialize_module(&bytes).expect("decode");
    [source, wire]
}
