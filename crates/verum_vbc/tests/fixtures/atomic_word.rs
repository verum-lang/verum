#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::FileId;
    use verum_common::List;
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};

    let selection = [
        ("mod.vr", ""),
        ("base/mod.vr", "Result"),
        ("base/result.vr", "Result"),
        (
            "intrinsics/atomic.vr",
            "ORDERING_RELAXED ORDERING_ACQUIRE ORDERING_RELEASE ORDERING_ACQ_REL ORDERING_SEQ_CST atomic_load_u64 atomic_store_u64 atomic_cas_u64 atomic_fetch_add_u64 atomic_fetch_or_u64",
        ),
        (
            "sync/atomic.vr",
            "AtomicInt AtomicInt.new AtomicInt.load AtomicInt.store AtomicInt.get_mut AtomicInt.compare_exchange AtomicInt.fetch_or AtomicU64 AtomicU64.new AtomicU64.load AtomicU64.store AtomicU64.get_mut AtomicU64.compare_exchange AtomicU64.fetch_add MemoryOrdering ordering_to_u8",
        ),
    ];
    let mut units: List<_> = selection
        .iter()
        .enumerate()
        .map(|(index, (path, symbols))| selected_stdlib::source_unit(path, symbols, index))
        .collect();
    let client = include_str!("../../../../core-tests/sync/atomic/word_test.vr");
    units.push(
        FastParser::new()
            .parse_module_str(client, FileId::new(100))
            .unwrap(),
    );
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
