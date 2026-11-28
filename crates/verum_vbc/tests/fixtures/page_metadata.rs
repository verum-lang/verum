#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

use verum_ast::{FileId, ItemKind};
use verum_common::List;
use verum_fast_parser::FastParser;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

pub fn module() -> verum_vbc::VbcModule {
    let selection = [
        ("mod.vr", ""),
        ("base/mod.vr", "Result Ok Err"),
        ("base/result.vr", "Result"),
        ("mem/header.vr", "GEN_INITIAL"),
        (
            "intrinsics/memory.vr",
            "ptr_offset ptr_offset_mut ptr_read ptr_write slice_from_raw_parts slice_from_raw_parts_mut",
        ),
        ("intrinsics/runtime/cbgr.vr", "cbgr_deallocate"),
        ("intrinsics/runtime/mem_raw.vr", "load_byte store_byte"),
        ("intrinsics/runtime/sync.vr", "spin_hint"),
        (
            "encoding/varint.vr",
            "VarintError VarintErrorKind sqlite_decode",
        ),
        ("database/sqlite/native/l2_record/varint.vr", "decode"),
    ];
    let mut units: List<_> = selection
        .iter()
        .enumerate()
        .map(|(index, (path, names))| selected_stdlib::source_unit(path, names, index))
        .collect();
    let parse = |source, index| {
        FastParser::new()
            .parse_module_str(source, FileId::new(index))
            .unwrap()
    };
    // The allocator mounts its raw-pointer operations through this facade's
    // real wildcard and explicit re-exports.
    units.push(parse(
        include_str!("../../../../core/intrinsics/mod.vr"),
        40,
    ));
    let mut allocator = selected_stdlib::source_unit(
        "mem/allocator.vr",
        "AllocError AllocPageHeader AllocPageHeader.slot_metadata AllocPageHeader.slot_metadata_mut PAGE_HEADER_SIZE cbgr_alloc_zeroed",
        41,
    );
    let client = parse(
        concat!(
            include_str!("../../../../core-tests/mem/allocator/page_metadata_test.vr"),
            "\nfn main() -> Int { metadata_control() }",
        ),
        43,
    );
    allocator.items.extend(
        client
            .items
            .into_iter()
            .filter(|item| !matches!(item.kind, ItemKind::Module(_))),
    );
    units.push(allocator);
    let inputs: List<_> = units.iter().collect();
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
