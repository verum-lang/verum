#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module, decl::ImplItemKind};
use verum_fast_parser::FastParser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn source(text: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(text, verum_ast::FileId::new(file))
        .unwrap()
}

fn declarations(ast: &mut Module, types: &[&str], functions: &[&str], methods: &[&str]) {
    ast.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(decl) => decl.items.is_none(),
        ItemKind::Mount(_) => true,
        ItemKind::Type(decl) => types.contains(&decl.name.as_str()),
        ItemKind::Function(decl) => functions.contains(&decl.name.as_str()),
        ItemKind::Const(decl) => functions.contains(&decl.name.as_str()),
        ItemKind::Impl(decl) => {
            let owner = match &decl.kind {
                verum_ast::decl::ImplKind::Inherent(owner) => owner,
                _ => return false,
            };
            let owner = if let verum_ast::TypeKind::Generic { base, .. } = &owner.kind {
                base.as_ref()
            } else {
                owner
            };
            let verum_ast::TypeKind::Path(path) = &owner.kind else {
                return false;
            };
            if !types.contains(&path.last_segment_name()) {
                return false;
            }
            decl.items.retain(|member| {
                matches!(&member.kind, ImplItemKind::Function(f)
                if methods.contains(&f.name.as_str()))
            });
            !decl.items.is_empty()
        }
        _ => false,
    });
}

#[test]
fn map_nested_causes_in_a_linked_source_component() {
    let client = source(
        include_str!("../../../vcs/specs/L2-standard/errors/advanced/cause_map_error_tree.vr"),
        4,
    );
    let mut maybe = source(include_str!("../../../core/base/maybe.vr"), 1);
    declarations(&mut maybe, &["Maybe"], &[], &[]);
    let mut memory = source(include_str!("../../../core/base/memory.vr"), 2);
    declarations(
        &mut memory,
        &["Heap"],
        &[
            "replace",
            "GEN_OFFSET",
            "EPOCH_CAPS_OFFSET",
            "ORDERING_ACQUIRE",
        ],
        &["new", "as_ref", "as_mut", "validate_cbgr"],
    );
    let mut result = source(include_str!("../../../core/base/result.vr"), 3);
    declarations(
        &mut result,
        &["Cause", "DefectInfo", "Result"],
        &[],
        &["take_child", "map_error"],
    );
    let mut storage = source(include_str!("../../../core/intrinsics/memory.vr"), 5);
    declarations(&mut storage, &[], &["ptr_write"], &[]);
    let mut atomic = source(include_str!("../../../core/intrinsics/atomic.vr"), 6);
    declarations(
        &mut atomic,
        &[],
        &["atomic_load_u32", "ORDERING_ACQUIRE"],
        &[],
    );
    let mut header = source(include_str!("../../../core/mem/header.vr"), 7);
    declarations(&mut header, &[], &["HEADER_SIZE"], &[]);
    let mut allocator = source(include_str!("../../../core/mem/allocator.vr"), 8);
    declarations(
        &mut allocator,
        &["AllocError"],
        &["cbgr_alloc"],
        &["message"],
    );

    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("cause_map_error_tree"));
    let units = [
        ("core.base.maybe", &maybe),
        ("core.intrinsics.memory", &storage),
        ("core.intrinsics.atomic", &atomic),
        ("core.mem.header", &header),
        ("core.mem.allocator", &allocator),
        ("core.base.memory", &memory),
        ("core.base.result", &result),
        ("cause_map_error_tree", &client),
    ];
    compiler.collect_owned_unit_declarations(&units).unwrap();
    compiler.resolve_pending_imports();
    compiler
        .compile_owned_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let module = compiler.finalize_module().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module_validated(&wire).unwrap();
    let entry = module
        .find_function_by_name("cause_map_error_tree.main")
        .unwrap();
    let returned = Interpreter::try_new_validated(module.into())
        .unwrap()
        .execute_function(entry)
        .unwrap();
    assert_eq!(returned.try_as_i64(), Some(0));
}
