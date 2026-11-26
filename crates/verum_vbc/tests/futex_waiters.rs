#![cfg(feature = "codegen")]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;
use verum_vbc::value::Value;

// Compile callers of the real stdlib intrinsic declarations, then run the same
// module on worker interpreters. Only the host thread scheduling and word
// allocation are supplied by this test.
fn vm() -> Interpreter {
    static MODULE: OnceLock<Arc<VbcModule>> = OnceLock::new();
    let module = MODULE.get_or_init(|| {
        let source = concat!(
            include_str!("../../../core/intrinsics/runtime/sync.vr"),
            r#"
            fn wait_word(addr: *const UInt32, expected: UInt32, ns: UInt64) -> Int32 {
                futex_wait(addr, expected, ns)
            }
            fn wake_word(addr: *const UInt32, count: UInt32) -> Int32 { futex_wake(addr, count) }
            fn wake_single(addr: *const UInt32) -> Int32 { futex_wake_one(addr) }
            fn broadcast(addr: *const UInt32) -> Int32 { futex_wake_all(addr) }
            type OwnedWord is { prefix: UInt32, value: UInt32 };
            implement OwnedWord {
                fn park(&self, expected: UInt32) -> Int32 {
                    futex_wait(&self.value as *const UInt32, expected, 2000000_u64)
                }
            }
            fn park_owned(expected: UInt32) -> Int32 {
                let cell = OwnedWord { prefix: 41_u32, value: 7_u32 };
                cell.park(expected)
            }
            fn publish(addr: *mut UInt32) -> Int32 {
                @intrinsic("atomic_store_u32", addr, 1_u32, 2_u8);
                futex_wake_one(addr as *const UInt32)
            }
            "#
        );
        let ast = Parser::new(source)
            .parse_module()
            .expect("parse futex callers");
        Arc::new(
            VbcCodegen::new()
                .compile_module(&ast)
                .expect("compile futex callers"),
        )
    });
    Interpreter::new(Arc::clone(module))
}

fn call(vm: &mut Interpreter, name: &str, args: &[Value]) -> i64 {
    let qualified = format!("core.intrinsics.runtime.sync.{name}");
    let function = vm
        .module()
        .find_function_by_name(&qualified)
        .unwrap_or_else(|| {
            let names: verum_common::List<_> = vm
                .module()
                .functions
                .iter()
                .filter_map(|function| vm.module().get_string(function.name))
                .collect();
            panic!("missing {qualified} in {names:?}");
        });
    vm.execute_function_with_args(function, args)
        .expect(name)
        .as_integer_compatible()
}

fn address(word: &AtomicU32) -> Value {
    Value::from_ptr(word.as_ptr())
}

fn wait(vm: &mut Interpreter, word: &AtomicU32, expected: i64, timeout_ns: i64) -> i64 {
    call(
        vm,
        "wait_word",
        &[
            address(word),
            Value::from_i64(expected),
            Value::from_i64(timeout_ns),
        ],
    )
}

#[test]
fn wakes_without_waiters_and_timeouts_leave_no_credit_on_reuse() {
    let word = AtomicU32::new(7);
    let mut worker = vm();
    assert_eq!(call(&mut worker, "wake_single", &[address(&word)]), 0);
    assert_eq!(wait(&mut worker, &word, 6, 0), -11);
    assert_eq!(wait(&mut worker, &word, 7, 2_000_000), -110);
    for _ in 0..2 {
        assert_eq!(call(&mut worker, "broadcast", &[address(&word)]), 0);
        assert_eq!(
            call(
                &mut worker,
                "wake_word",
                &[address(&word), Value::from_i64(3)]
            ),
            0
        );
    }
    word.store(9, Ordering::Release);
    assert_eq!(wait(&mut worker, &word, 9, 2_000_000), -110);
}

#[test]
fn direct_owned_word_projection_compares_the_selected_field() {
    let mut worker = vm();
    assert_eq!(call(&mut worker, "park_owned", &[Value::from_i64(41)]), -11);
    assert_eq!(call(&mut worker, "park_owned", &[Value::from_i64(7)]), -110);
}

#[test]
fn registered_indefinite_wait_accepts_one_wake_and_leaves_the_address_empty() {
    let word = Arc::new(AtomicU32::new(4));
    let (report, result) = mpsc::channel();
    let parked_word = Arc::clone(&word);
    let worker = thread::spawn(move || {
        report.send(wait(&mut vm(), &parked_word, 4, 0)).unwrap();
    });
    let mut sender = vm();
    let deadline = Instant::now() + Duration::from_secs(4);
    while call(&mut sender, "wake_single", &[address(&word)]) == 0 {
        assert!(Instant::now() < deadline, "the waiter failed to register");
        thread::yield_now();
    }
    assert_eq!(result.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
    worker.join().unwrap();
    assert_eq!(call(&mut sender, "wake_single", &[address(&word)]), 0);
}

#[test]
fn capped_wakes_and_broadcast_select_only_current_waiters_at_the_same_address() {
    const WORKERS: usize = 6;
    let word = AtomicU32::new(0);
    let other = AtomicU32::new(0);
    let started = Barrier::new(WORKERS + 1);
    let (finished, results) = mpsc::channel();
    thread::scope(|scope| {
        for _ in 0..WORKERS {
            let finished = finished.clone();
            let started = &started;
            let word = &word;
            scope.spawn(move || {
                let mut worker = vm();
                started.wait();
                finished
                    .send(wait(&mut worker, word, 0, 5_000_000_000))
                    .unwrap();
            });
        }
        let mut sender = vm();
        started.wait();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut selected = 0;
        while selected < WORKERS {
            assert!(Instant::now() < deadline, "waiters failed to register");
            assert_eq!(call(&mut sender, "broadcast", &[address(&other)]), 0);
            assert_eq!(
                call(
                    &mut sender,
                    "wake_word",
                    &[address(&word), Value::from_i64(0)]
                ),
                0
            );
            let cap = match selected {
                0 => 1,
                1 | 2 => 2,
                _ => WORKERS - selected,
            };
            let count = if selected < 3 {
                call(
                    &mut sender,
                    "wake_word",
                    &[address(&word), Value::from_i64(cap as i64)],
                )
            } else {
                call(&mut sender, "broadcast", &[address(&word)])
            };
            assert!((0..=cap as i64).contains(&count));
            if count == 0 {
                thread::yield_now();
            }
            selected += count as usize;
        }
        for _ in 0..WORKERS {
            assert_eq!(results.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
        }
        assert_eq!(call(&mut sender, "broadcast", &[address(&word)]), 0);
    });
}

#[test]
fn a_timeout_racing_a_wake_has_exactly_one_winner() {
    let word = AtomicU32::new(3);
    thread::scope(|scope| {
        let (begin, start) = mpsc::sync_channel(0);
        let (report, result) = mpsc::channel();
        let shared_word = &word;
        scope.spawn(move || {
            let mut worker = vm();
            while start.recv().is_ok() {
                if report
                    .send(wait(&mut worker, shared_word, 3, 100_000))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut sender = vm();
        for round in 0..64 {
            begin.send(()).unwrap();
            if round % 2 == 0 {
                thread::sleep(Duration::from_micros(100));
            }
            let woken = call(&mut sender, "wake_single", &[address(&word)]);
            let waited = result.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                matches!((woken, waited), (0, -110) | (1, 0)),
                "wake={woken}, wait={waited}"
            );
        }
        assert_eq!(call(&mut sender, "broadcast", &[address(&word)]), 0);
    });
}

#[test]
fn release_store_and_wake_racing_registration_either_delivers_or_observes_the_new_word() {
    let word = AtomicU32::new(0);
    thread::scope(|scope| {
        let (begin, start) = mpsc::sync_channel(0);
        let (report, result) = mpsc::channel();
        let shared_word = &word;
        scope.spawn(move || {
            let mut worker = vm();
            while start.recv().is_ok() {
                let waited = wait(&mut worker, shared_word, 0, 1_000_000_000);
                if report.send(waited).is_err() {
                    break;
                }
            }
        });
        let mut publisher = vm();
        for _ in 0..128 {
            word.store(0, Ordering::Release);
            begin.send(()).unwrap();
            let woken = call(&mut publisher, "publish", &[address(&word)]);
            let waited = result.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(
                matches!((woken, waited), (0, -11) | (1, 0)),
                "wake={woken}, wait={waited}"
            );
            assert_eq!(word.load(Ordering::Acquire), 1);
        }
    });
}
