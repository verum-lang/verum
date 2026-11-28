//! Exercise the same source declarations and storage operations in the VM and native backend.
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    visitor::{Visitor, walk_expr},
};
use verum_fast_parser::FastParser;

pub const RETAIN_OWNERSHIP: &str =
    include_str!("../../../../core-tests/collections/list/retain_ownership_test.vr");
pub const RETAIN_PROBES: &[&str] = &[
    "retain_preserves_order_and_drops_each_rejected_owner_once",
    "retaining_borrowed_elements_leaves_their_owner_live",
    "retaining_raw_handles_leaves_their_owner_live",
];
pub const CLONE_ELEMENTS: &str = r#"
    mount core.base.protocols.Clone;
    static mut CLONES: Int = 0;
    type Ticket is { id: Int };
    implement Clone for Ticket {
        fn clone(&self) -> Ticket {
            unsafe { CLONES = CLONES + 1; }
            Ticket { id: self.id + 10 }
        }
    }
    fn probe() -> Int {
        let mut source: List<Ticket> = List<Ticket>.new();
        source.push(Ticket { id: 1 });
        source.push(Ticket { id: 7 });
        let copy = source.clone();
        assert(source[0].id == 1 && source[1].id == 7);
        assert(copy[0].id == 11 && copy[1].id == 17);
        unsafe { CLONES }
    }
"#;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
fn named_type(ty: &verum_ast::Type, name: &str) -> bool {
    match &ty.kind {
        verum_ast::ty::TypeKind::Path(p) => p.as_ident().is_some_and(|i| i.name == name),
        verum_ast::ty::TypeKind::Generic { base, .. } => named_type(base, name),
        _ => false,
    }
}

#[derive(Default)]
struct ClientMethods {
    clone: bool,
    retain: bool,
}

impl Visitor for ClientMethods {
    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        if let verum_ast::ExprKind::MethodCall { method, .. } = &expr.kind {
            match method.name.as_str() {
                "clone" => self.clone = true,
                "retain" | "retain_mut" => self.retain = true,
                _ => {}
            }
        }
        walk_expr(self, expr);
    }
}

fn is_header(item: &verum_ast::Item) -> bool {
    matches!(&item.kind, ItemKind::Module(decl) if decl.items.is_none())
        || matches!(&item.kind, ItemKind::Mount(_))
}

pub fn source_module(source: &str, rename_methods: bool) -> verum_vbc::VbcModule {
    let parser = FastParser::new();
    let mut files = 0..;
    let mut parse = |source: &str| {
        parser
            .parse_module_str(source, verum_ast::FileId::new(files.next().unwrap()))
            .expect("fixture must follow the source grammar")
    };
    let ast = if source.starts_with("module ") {
        parse(source)
    } else {
        parse(&format!(
            "mount core.collections.list.List; mount core.base.maybe.Maybe; mount core.base.result.Result; mount core.mem.allocator.AllocError;\n{source}"
        ))
    };
    let mut client = ClientMethods::default();
    for item in &ast.items {
        client.visit_item(item);
    }
    let mut core = parse(include_str!("../../../../core/collections/list.vr"));
    core.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Mount(_) => true,
        ItemKind::Type(t) => t.name.name == "List",
        ItemKind::Function(f) => f.name.name == "list_max_len",
        ItemKind::Impl(i) if matches!(&i.kind, ImplKind::Inherent(t) if named_type(t, "List")) => {
            let mut aliases = verum_common::List::new();
            i.items.retain_mut(|m| {
                let ImplItemKind::Function(f) = &m.kind else {
                    return false;
                };
                if [
                    "with_capacity",
                    "try_with_capacity",
                    "clear",
                    "truncate",
                    "get",
                    "set",
                    "push",
                    "pop",
                    "insert",
                    "remove",
                    "swap",
                    "swap_remove",
                ]
                .contains(&f.name.name.as_str())
                {
                    if rename_methods {
                        // An alias exercises the same source body independently of
                        // native method intercepts. Clients and other source methods
                        // still resolve the ordinary declaration under its real name.
                        let mut alias = m.clone();
                        if let ImplItemKind::Function(copy) = &mut alias.kind {
                            copy.name.name = format!("checked_{}", f.name.name).into();
                        }
                        aliases.push(alias);
                    }
                    true
                } else {
                    [
                        "new",
                        "len",
                        "capacity",
                        "reserve",
                        "shrink_to_fit",
                        "resize_buffer",
                        "try_resize_buffer",
                        "free_buffer",
                        "grow",
                        "next_cap",
                        "reverse",
                        "is_empty",
                        "as_slice",
                    ]
                    .contains(&f.name.name.as_str())
                        || (client.retain && matches!(f.name.name.as_str(), "retain" | "retain_mut"))
                }
            });
            i.items.extend(aliases);
            true
        }
        ItemKind::Impl(decl) if matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.as_ident().is_some_and(|name| match name.name.as_str() {
                "Drop" => named_type(for_type, "List"),
                "Clone" => client.clone && named_type(for_type, "List"),
                _ => false,
            })) => true,
        _ => false,
    });
    let mut protocols = parse(include_str!("../../../../core/base/protocols.vr"));
    protocols.items.retain(|item| {
        is_header(item)
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|name| name.as_str() == "Drop" || (client.clone && name.as_str() == "Clone"))
    });
    let mut memory = parse(include_str!("../../../../core/intrinsics/memory.vr"));
    memory.items.retain(|item| is_header(item) || matches!(&item.kind, ItemKind::Function(f) if ["slice_from_raw_parts", "list_storage_stride", "list_storage_read", "list_storage_write", "list_storage_move", "list_storage_resize"].contains(&f.name.name.as_str())));
    let mut primitives = parse(include_str!("../../../../core/base/primitives.vr"));
    primitives.items.retain_mut(|item| {
        if is_header(item) { return true; }
        let ItemKind::Impl(decl) = &mut item.kind else { return false; };
        if client.clone && matches!(&decl.kind, ImplKind::Protocol { protocol, for_type, .. }
            if protocol.as_ident().is_some_and(|name| name.name == "Clone")
            && (matches!(for_type.kind, verum_ast::ty::TypeKind::Int) || named_type(for_type, "Byte"))) {
            return true;
        }
        if !matches!(&decl.kind, ImplKind::Inherent(ty) if matches!(&ty.kind,
            verum_ast::ty::TypeKind::Int)) {
            return false;
        }
        decl.items.retain(|item| matches!(&item.kind, ImplItemKind::Function(function) if function.name.name == "checked_mul"));
        true
    });
    let mut arithmetic = parse(include_str!("../../../../core/intrinsics/arithmetic.vr"));
    arithmetic.items.retain(|item| is_header(item) || matches!(&item.kind, ItemKind::Function(function) if function.name.name == "checked_mul"));
    let mut allocation = parse(include_str!("../../../../core/mem/allocator.vr"));
    allocation.items.retain(|item| {
        is_header(item)
            || matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "AllocError")
    });
    let mut result_owner = parse(include_str!("../../../../core/base/result.vr"));
    result_owner.items.retain(|item| {
        is_header(item) || matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "Result")
    });
    let mut maybe_owner = parse(include_str!("../../../../core/base/maybe.vr"));
    maybe_owner.items.retain(|item| {
        is_header(item) || matches!(&item.kind, ItemKind::Type(decl) if decl.name.name == "Maybe")
    });
    let mut root = parse(include_str!("../../../../core/mod.vr"));
    root.items.retain_mut(|item| {
        if let ItemKind::Module(module) = &mut item.kind
            && module.name.name == "prelude"
            && let Some(items) = &mut module.items
        {
            items.retain(|item| matches!(item.kind, ItemKind::Mount(_)));
            return true;
        }
        is_header(item)
    });
    let mut collections = parse(include_str!("../../../../core/collections/mod.vr"));
    collections.items.retain(is_header);
    let units = [
        &root,
        &collections,
        &protocols,
        &result_owner,
        &maybe_owner,
        &allocation,
        &memory,
        &arithmetic,
        &primitives,
        &core,
        &ast,
    ];
    let mut codegen = VbcCodegen::new();
    codegen.register_builtin_variants();
    codegen.register_stdlib_constants();
    codegen.register_stdlib_intrinsics();
    codegen
        .collect_unit_declarations(&units)
        .expect("declarations");
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .expect("source bodies");
    let mut vbc = codegen.finalize_module().expect("source VBC");
    vbc.resolve_protocol_dispatch();
    for function in &mut vbc.functions {
        function.is_generic = !function.type_params.is_empty();
    }
    vbc
}

pub fn compile(source: &str, rename_methods: bool) -> verum_vbc::VbcModule {
    let mut vbc = source_module(source, rename_methods);
    for function in &mut vbc.functions {
        let body = &vbc.bytecode[function.bytecode_offset as usize
            ..(function.bytecode_offset + function.bytecode_length) as usize];
        let mut instructions = verum_vbc::bytecode::decode_instructions(body).expect("source body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    let graph = verum_vbc::mono::module_instantiations(&vbc).expect("source generic witnesses");
    let mut module = verum_vbc::mono::monomorphize_minimal(vbc, &graph)
        .expect("source mono")
        .module;
    module.resolve_external_bands();
    module.synthesize_intrinsic_band_wrappers();
    module.resolve_protocol_dispatch();
    module
}
