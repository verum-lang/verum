#![cfg(feature = "codegen")]
//! Execute the stdlib release paths with real atomic and futex instructions.
//! The test sets the initial holder count; the public fixtures exercise guards.

use std::sync::Arc;
use verum_ast::{
    ItemKind, Module,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const PROBE: &str = r#"
fn check_releases() -> Int {
    let lock: RwLock<Int> = RwLock.new(17);
    lock.state.store(2, MemoryOrdering.Release);
    lock.release_read();
    if lock.state.load(MemoryOrdering.Acquire) != 1 { return 1; }
    if lock.writer_sequence.load(MemoryOrdering.Acquire) != 0 { return 2; }
    if lock.reader_sequence.load(MemoryOrdering.Acquire) != 0 { return 3; }
    lock.release_read();
    if lock.state.load(MemoryOrdering.Acquire) != 0 { return 4; }
    if lock.writer_sequence.load(MemoryOrdering.Acquire) != 1 { return 5; }
    if lock.reader_sequence.load(MemoryOrdering.Acquire) != 0 { return 6; }
    lock.state.store(-1, MemoryOrdering.Release);
    lock.release_write();
    if lock.state.load(MemoryOrdering.Acquire) != 0 { return 7; }
    if lock.writer_sequence.load(MemoryOrdering.Acquire) != 2 { return 8; }
    if lock.reader_sequence.load(MemoryOrdering.Acquire) != 1 { return 9; }
    0
}
"#;

fn release_unit(owner: &str, source: &str) -> Module {
    let mut module = Parser::new(source).parse_module().expect(owner);
    module.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Mount(_) => true,
        ItemKind::Const(decl) => decl.name.name.starts_with("ORDERING_"),
        ItemKind::Type(decl) => matches!(
            decl.name.name.as_str(),
            "MemoryOrder" | "MemoryOrdering" | "AtomicInt" | "AtomicBool" | "AtomicU32" | "RwLock"
        ),
        ItemKind::Function(decl) => match owner {
            "core.intrinsics.atomic" | "core.intrinsics.runtime.sync" => {
                decl.body.is_none() || decl.name.name == "compiler_fence"
            }
            _ => matches!(
                decl.name.name.as_str(),
                "ordering_to_u8" | "wake_on_release"
            ),
        },
        ItemKind::Impl(decl) => {
            let ImplKind::Inherent(target) = &decl.kind else {
                return false;
            };
            let base = match &target.kind {
                TypeKind::Generic { base, .. } => base.as_ref(),
                _ => target,
            };
            let TypeKind::Path(path) = &base.kind else {
                return false;
            };
            let methods: &[&str] = match path.last_segment_name() {
                "AtomicInt" => &["new", "load", "store", "fetch_sub"],
                "AtomicU32" => &["new", "load", "fetch_add", "wake_waiters"],
                "AtomicBool" => &["new"],
                "RwLock" => &["new", "release_read", "release_write"],
                _ => return false,
            };
            decl.items.retain(|item| {
                matches!(&item.kind,
                ImplItemKind::Function(method) if methods.contains(&method.name.name.as_str()))
            });
            !decl.items.is_empty()
        }
        _ => false,
    });
    if owner == "core.sync.rwlock" {
        module
            .items
            .extend(Parser::new(PROBE).parse_module().unwrap().items);
    }
    module
}

#[test]
fn last_reader_advances_only_the_writer_queue_and_write_release_advances_both() {
    let sources = [
        (
            "core.intrinsics.atomic",
            include_str!("../../../core/intrinsics/atomic.vr"),
        ),
        (
            "core.intrinsics.runtime.sync",
            include_str!("../../../core/intrinsics/runtime/sync.vr"),
        ),
        (
            "core.sync.atomic",
            include_str!("../../../core/sync/atomic.vr"),
        ),
        (
            "core.sync.rwlock",
            include_str!("../../../core/sync/rwlock.vr"),
        ),
    ];
    let modules = sources.map(|(owner, source)| (owner, release_unit(owner, source)));
    let entries = modules.each_ref().map(|(owner, module)| (*owner, module));
    let mut generator = VbcCodegen::new();
    generator.register_builtin_variants();
    generator.register_stdlib_constants();
    generator.register_stdlib_intrinsics();
    generator
        .collect_owned_unit_declarations(&entries)
        .expect("release declarations");
    generator
        .compile_owned_unit_items(&entries, ItemFailurePolicy::Strict)
        .expect("release bodies");
    let module = generator.finalize_module().expect("release module");
    for name in [
        "core.sync.rwlock.RwLock.release_read",
        "core.sync.rwlock.RwLock.release_write",
        "core.sync.rwlock.wake_on_release",
        "core.sync.atomic.AtomicU32.wake_waiters",
    ] {
        let id = module.find_function_by_name(name).expect(name);
        let descriptor = module.get_function(id).expect(name);
        assert!(
            descriptor.has_source_body && descriptor.bytecode_length > 0,
            "{name}"
        );
    }
    let entry = module
        .find_function_by_name("core.sync.rwlock.check_releases")
        .unwrap();
    let mut vm = Interpreter::new(Arc::new(module));
    vm.run_global_ctors().unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(0));
}
