//! Execute the executor's timer queue with recorded wakes and owned waker releases.
#![cfg(feature = "codegen")]

#[path = "support/selected_stdlib.rs"]
mod selected_stdlib;

use selected_stdlib::source_unit;
use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const WAKER_PROBE: &str = r#"
// Test waker records invocation and ownership release for each registration.
fn fired(id: Int) -> Int { static mut COUNT: Int = 0; if id != 0 { COUNT = COUNT * 10 + id; } COUNT }
fn released(increment: Int) -> Int { static mut COUNT: Int = 0; COUNT += increment; COUNT }
fn remember(data: &unsafe Byte) { let _ = fired(data as Int); }
fn release(_data: &unsafe Byte) { let _ = released(1); }
fn owned_wake(data: &unsafe Byte) { remember(data); release(data); }
public fn recording(id: Int) -> Waker {
    Waker { raw: RawWaker { data: id as &unsafe Byte, vtable: RawWakerVTable {
        clone: noop_clone, wake: owned_wake, wake_by_ref: remember, drop: release,
    } } }
}
public fn observed_wakes() -> Int { fired(0) }
public fn observed_releases() -> Int { released(0) }
 "#;

const QUEUE_PROBE: &str = r#"
mount core.async.waker.{recording, observed_wakes, observed_releases};
fn notify_due(queue: &mut TimerQueue, now: UInt64) -> Bool {
    let due = queue.pop_due(now);
    match &due {
        Maybe.Some(entry) => { entry.waker.wake_by_ref(); true },
        Maybe.None => false,
    }
}
fn probe_timer_order() -> Int {
    let mut queue = TimerQueue.new();
    if queue.next_deadline().is_some() { return -1; }
    queue.register(20, recording(3));
    queue.register(10, recording(1));
    queue.register(10, recording(2));
    if queue.next_deadline().unwrap() != 10 || notify_due(&mut queue, 9) { return -2; }
    if !notify_due(&mut queue, 10) || observed_wakes() != 1 { return -3; }
    if !notify_due(&mut queue, 10) || observed_wakes() != 12 { return -4; }
    if queue.next_deadline().unwrap() != 20 || notify_due(&mut queue, 19) { return -5; }
    if !notify_due(&mut queue, 20) || observed_wakes() != 123 { return -6; }
    if queue.next_deadline().is_some() || notify_due(&mut queue, 100) { return -7; }
    if observed_releases() == 3 { 1 } else { -8 }
}
"#;

#[test]
fn timers_fire_in_deadline_and_registration_order_and_release_their_wakers() {
    let sources = [
        ("mod.vr", ""),
        ("base/mod.vr", "Maybe Result"),
        ("base/result.vr", "Result"),
        (
            "base/maybe.vr",
            "Maybe Maybe<T>.is_none Maybe<T>.is_some Maybe<T>.unwrap",
        ),
        ("base/protocols.vr", "Drop Clone Deref DerefMut"),
        ("base/memory.vr", "replace forget alloc is_null"),
        (
            "intrinsics/memory.vr",
            "null_ptr ptr_offset ptr_is_null ptr_write list_storage_write slice_from_raw_parts list_storage_read list_storage_move forget",
        ),
        (
            "intrinsics/runtime/mem_raw.vr",
            "memset_addr memcpy_addr __load_byte __store_byte load_byte store_byte",
        ),
        (
            "mem/allocator.vr",
            "AllocError AllocError.message cbgr_alloc",
        ),
        (
            "collections/list.vr",
            "List List<T>.new List<T>.push List<T>.len List<T>.as_slice List<T>.with_capacity List<T>.insert List<T>.pop List<T>.remove List<T>.truncate",
        ),
        (
            "async/executor.vr",
            "TimerEntry TimerQueue TimerQueue.new TimerQueue.register TimerQueue.next_deadline TimerQueue.pop_due",
        ),
        (
            "async/waker.vr",
            "noop_waker noop_vtable noop_clone noop_drop noop_wake noop_wake_by_ref Waker Waker.from_raw Waker.wake Waker.wake_by_ref Drop@Waker.drop RawWaker RawWaker.new RawWakerVTable RawWakerVTable.new",
        ),
    ];
    let units: verum_common::List<_> = sources
        .iter()
        .enumerate()
        .map(|(index, (path, symbols))| {
            let mut unit = source_unit(path, symbols, index);
            let probe = match *path {
                "async/waker.vr" => Some(WAKER_PROBE),
                "async/executor.vr" => Some(QUEUE_PROBE),
                _ => None,
            };
            if let Some(probe) = probe {
                let extra = FastParser::new()
                    .parse_module_str(probe, FileId::new(index as u32 + 100))
                    .unwrap();
                unit.items.extend(extra.items);
            }
            unit
        })
        .collect();
    let sources: verum_common::List<_> = units.iter().collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(""));
    codegen.initialize();
    codegen.collect_unit_declarations(&sources).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&sources, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = codegen.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
    ] {
        let entry = module
            .find_function_by_name("core.async.executor.probe_timer_order")
            .unwrap();
        let mut vm = Interpreter::try_new_validated(module.into()).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(1));
    }
}
