#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::{FileId, ItemKind, cfg::TargetConfig};
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::VbcCodegen;
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy};
    // The uncontended path takes and releases the source mutex twice.
    let selection = [
        ("base/result.vr", "Result Result<T,E>.is_ok"),
        ("base/memory.vr", "replace drop"),
        (
            "base/maybe.vr",
            "Maybe Maybe<T>.replace Maybe<T>.is_some Maybe<T>.take Maybe<T>.is_none Maybe<T>.unwrap",
        ),
        ("mod.vr", ""),
        ("base/mod.vr", "Maybe Result"),
        ("base/protocols.vr", "Drop Clone Deref DerefMut Send Sync"),
        (
            "sync/mutex.vr",
            "LockResult PoisonError<T>.get_mut Mutex MutexGuard PoisonError Mutex<T>.new Mutex<T>.lock PoisonError<T>.into_inner Drop@MutexGuard<T>.drop lock_contended Deref@MutexGuard<T>.deref DerefMut@MutexGuard<T>.deref_mut MutexGuard<T>.data MutexGuard<T>.data_mut PoisonError<T>.new",
        ),
        (
            "sync/atomic.vr",
            "AtomicInt AtomicInt.new AtomicInt.fetch_add MemoryOrdering ordering_to_u8 FutexLock AtomicBool AtomicBool.new FutexLock.lock AtomicBool.load FutexLock.new FutexLock.unlock AtomicU32 FUTEX_UNLOCKED FutexLock.try_lock AtomicInt.compare_exchange FUTEX_LOCKED FutexLock.lock_slow AtomicInt.load AtomicInt.swap FUTEX_LOCKED_CONTENDED",
        ),
        (
            "sync/mod.vr",
            "Mutex MutexGuard AtomicInt MemoryOrdering Send Sync",
        ),
        (
            "intrinsics/atomic.vr",
            "ORDERING_RELAXED ORDERING_ACQUIRE ORDERING_RELEASE ORDERING_ACQ_REL ORDERING_SEQ_CST atomic_fetch_add_u64 atomic_load_u8 atomic_cas_u64 atomic_load_u64",
        ),
        ("intrinsics/memory.vr", "forget ptr_read"),
        ("sys/common.vr", "OSError OSError.from_code"),
        (
            "intrinsics/runtime/sync.vr",
            "futex_wait futex_wake futex_wake_one futex_wake_all",
        ),
        (
            "sys/darwin/thread.vr",
            "futex_wait futex_wake __ulock_wait __ulock_wake",
        ),
        (
            "sys/error.vr",
            "native_error_kind OSErrorKind native_error_message",
        ),
        ("sys/darwin/errno.vr", "*const"),
        (
            "sys/darwin/libsystem.vr",
            "poll poll_descriptors PthreadMutex",
        ),
    ];
    let client = r#"
module core.sync.mutex;
type GuardPayload is { first: Int, second: Int };
type Holder is { mutex: Mutex<GuardPayload>, guard_cookie: Int };
fn probe_retained_guard() -> Int {
    let owner = Holder { mutex: Mutex.new(GuardPayload { first: 17, second: 41 }), guard_cookie: 99 };
    {
        let mut first_lock = owner.mutex.lock();
        match &mut first_lock {
            Result.Ok(guard) => {
                let data = guard.deref_mut();
                data.second = 73;
                if data.first != 17 { return -2; }
            }
            Result.Err(_) => return -3,
        }
    }
    let second_lock = owner.mutex.lock();
    match &second_lock {
        Result.Ok(guard) => if guard.deref().second == 73 && owner.guard_cookie == 99 { 1 } else { -1 },
        Result.Err(_) => -4,
    }
}
fn main() -> Int { if probe_retained_guard() == 1 { 73 } else { 1 } }
"#;
    let mut units: verum_common::List<_> = selection
        .iter()
        .enumerate()
        .map(|(i, (file, symbols))| selected_stdlib::source_unit(file, symbols, i))
        .collect();
    let client = FastParser::new()
        .parse_module_str(client, FileId::new(300))
        .unwrap();
    let index = selection
        .iter()
        .position(|(file, _)| *file == "sync/mutex.vr")
        .unwrap();
    units[index].items.extend(
        client
            .items
            .into_iter()
            .filter(|item| !matches!(item.kind, ItemKind::Module(_))),
    );
    let inputs: verum_common::List<_> = units.iter().collect();
    let mut generator =
        VbcCodegen::with_config(CodegenConfig::new("").with_target(TargetConfig::macos_aarch64()));
    generator.initialize();
    generator.collect_unit_declarations(&inputs).unwrap();
    generator.resolve_pending_imports();
    generator
        .compile_unit_items(&inputs, ItemFailurePolicy::Strict)
        .unwrap();
    let mut source = generator.finalize_module().unwrap();
    source.resolve_protocol_dispatch();
    source
}
