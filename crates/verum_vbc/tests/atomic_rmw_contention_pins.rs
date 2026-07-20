//! Contention pins for the atomic read-modify-write opcode (T0340).
//!
//! `atomic_fetch_add` and its five siblings used to be lowered as an
//! `AtomicLoad`, an arithmetic instruction and one `AtomicCas` whose
//! result was discarded.  Every step was atomic; the sequence was not.
//! Two threads that interleave both read the same old value, both write
//! `old + 1`, and one increment disappears — with no error raised
//! anywhere and no way for a single-threaded test to notice.
//!
//! These pins therefore have to CONTEND.  A test that increments a
//! counter on one thread passes just as happily against the broken
//! lowering, which is exactly how it survived.  Each pin below runs
//! many threads against one shared cell and checks two properties:
//!
//!   * the final total is EXACTLY `threads * iterations` — no update
//!     was lost;
//!   * the old values returned across all threads are a permutation of
//!     `0..total` — every operation observed a distinct predecessor,
//!     which is the stronger statement that no two operations were
//!     serialised at the same point.
//!
//! Each is repeated over several rounds, because a race that shows up
//! one run in twenty passes once and lies.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use verum_vbc::bytecode;
use verum_vbc::instruction::{AtomicRmwOp, Instruction, Reg, SystemSubOpcode};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::nanbox::{NAN_INTEGER_HEADER, PAYLOAD_MASK};

/// How many worker threads contend for the shared cell.
const THREADS: usize = 8;
/// How many operations each worker performs.
const ITERS: usize = 500;
/// How many times the whole contended scenario is repeated.
const ROUNDS: usize = 12;

/// Build a single-function module whose body performs one atomic RMW
/// against the address baked into the bytecode and returns the value
/// read before the update.
fn rmw_module(addr: usize, op: AtomicRmwOp, operand: i64, size: u8) -> Arc<VbcModule> {
    let instrs = [
        Instruction::LoadI {
            dst: Reg(0),
            value: addr as i64,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: operand,
        },
        op.encode(Reg(2), Reg(0), Reg(1), size),
        Instruction::Ret { value: Reg(2) },
    ];

    let mut bc = Vec::new();
    for instr in &instrs {
        bytecode::encode_instruction(instr, &mut bc);
    }

    let mut module = VbcModule::new("atomic_rmw_pin".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bc.len() as u32;
    func.register_count = 8;
    module.functions.push(func);
    module.bytecode = bc;
    Arc::new(module)
}

/// An 8-byte atomic slot holds a NaN-boxed `Value`, so reading one back
/// from Rust means unwrapping the tag the same way the interpreter does.
fn unbox(raw: u64) -> i64 {
    let payload = (raw & PAYLOAD_MASK) as i64;
    if payload & (1 << 47) != 0 {
        payload | !(PAYLOAD_MASK as i64)
    } else {
        payload
    }
}

fn box_i64(v: i64) -> u64 {
    NAN_INTEGER_HEADER | ((v as u64) & PAYLOAD_MASK)
}

/// Run `THREADS` workers, each performing `ITERS` copies of the module's
/// RMW, and collect every returned old value.
fn contend(module: Arc<VbcModule>) -> Vec<i64> {
    let mut handles = Vec::with_capacity(THREADS);
    for _ in 0..THREADS {
        let module = Arc::clone(&module);
        handles.push(thread::spawn(move || {
            let mut seen = Vec::with_capacity(ITERS);
            let mut interp = Interpreter::new(module);
            for _ in 0..ITERS {
                let old = interp
                    .execute_function(FunctionId(0))
                    .expect("atomic RMW execution failed");
                seen.push(old.as_i64());
            }
            seen
        }));
    }

    let mut all = Vec::with_capacity(THREADS * ITERS);
    for h in handles {
        all.extend(h.join().expect("worker thread panicked"));
    }
    all
}

#[test]
fn fetch_add_under_contention_loses_no_updates() {
    let total = (THREADS * ITERS) as i64;

    for round in 0..ROUNDS {
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let module = rmw_module(cell as *const _ as usize, AtomicRmwOp::Add, 1, 8);

        let mut olds = contend(module);

        assert_eq!(
            unbox(cell.load(Ordering::SeqCst)),
            total,
            "round {round}: {THREADS} threads x {ITERS} increments must total {total} — \
             a smaller number means increments were read-modify-written on top of each other"
        );

        olds.sort_unstable();
        let expected: Vec<i64> = (0..total).collect();
        assert_eq!(
            olds, expected,
            "round {round}: every fetch_add must observe a distinct predecessor value"
        );
    }
}

#[test]
fn fetch_sub_under_contention_loses_no_updates() {
    let total = (THREADS * ITERS) as i64;

    for round in 0..ROUNDS {
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(box_i64(total))));
        let module = rmw_module(cell as *const _ as usize, AtomicRmwOp::Sub, 1, 8);

        let mut olds = contend(module);

        assert_eq!(
            unbox(cell.load(Ordering::SeqCst)),
            0,
            "round {round}: {total} decrements from {total} must reach exactly 0"
        );

        olds.sort_unstable();
        let expected: Vec<i64> = (1..=total).collect();
        assert_eq!(
            olds, expected,
            "round {round}: every fetch_sub must observe a distinct predecessor value"
        );
    }
}

#[test]
fn fetch_or_under_contention_sets_every_bit() {
    // Each worker owns one bit and ORs it in repeatedly. A dropped
    // update leaves that worker's bit clear in the final word.
    for round in 0..ROUNDS {
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let addr = cell as *const _ as usize;

        let mut handles = Vec::with_capacity(THREADS);
        for tid in 0..THREADS {
            let module = rmw_module(addr, AtomicRmwOp::Or, 1i64 << tid, 8);
            handles.push(thread::spawn(move || {
                let mut interp = Interpreter::new(module);
                for _ in 0..ITERS {
                    interp
                        .execute_function(FunctionId(0))
                        .expect("atomic OR execution failed");
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }

        let expected = (1i64 << THREADS) - 1;
        assert_eq!(
            unbox(cell.load(Ordering::SeqCst)),
            expected,
            "round {round}: every worker's bit must survive; a missing bit is a lost OR"
        );
    }
}

#[test]
fn fetch_and_clears_exactly_the_masked_bits() {
    // Start with all THREADS bits set; each worker clears its own.
    for round in 0..ROUNDS {
        let start = (1i64 << THREADS) - 1;
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(box_i64(start))));
        let addr = cell as *const _ as usize;

        let mut handles = Vec::with_capacity(THREADS);
        for tid in 0..THREADS {
            let module = rmw_module(addr, AtomicRmwOp::And, !(1i64 << tid), 8);
            handles.push(thread::spawn(move || {
                let mut interp = Interpreter::new(module);
                for _ in 0..ITERS {
                    interp
                        .execute_function(FunctionId(0))
                        .expect("atomic AND execution failed");
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }

        assert_eq!(
            unbox(cell.load(Ordering::SeqCst)),
            0,
            "round {round}: every worker must have cleared its bit"
        );
    }
}

#[test]
fn fetch_xor_toggles_an_even_number_of_times() {
    // Each worker XORs bit 0 an even number of times, so the bit must
    // end clear. A lost update flips the parity.
    for round in 0..ROUNDS {
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
        let addr = cell as *const _ as usize;
        let module = rmw_module(addr, AtomicRmwOp::Xor, 1, 8);

        let olds = contend(module);
        assert_eq!(olds.len(), THREADS * ITERS);

        let expected = if (THREADS * ITERS) % 2 == 0 { 0 } else { 1 };
        assert_eq!(
            unbox(cell.load(Ordering::SeqCst)),
            expected,
            "round {round}: {} XORs of bit 0 must leave parity {expected}",
            THREADS * ITERS
        );
    }
}

#[test]
fn exchange_under_contention_hands_off_every_value() {
    // Each worker swaps in its own id and records what it displaced.
    // With a correct exchange the multiset {final value} ∪ {displaced}
    // minus {initial} is exactly the multiset of values written.
    for round in 0..ROUNDS {
        let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(box_i64(-1))));
        let addr = cell as *const _ as usize;

        let mut handles = Vec::with_capacity(THREADS);
        for tid in 0..THREADS {
            let module = rmw_module(addr, AtomicRmwOp::Xchg, tid as i64, 8);
            handles.push(thread::spawn(move || {
                let mut interp = Interpreter::new(module);
                let mut seen = Vec::with_capacity(ITERS);
                for _ in 0..ITERS {
                    let old = interp
                        .execute_function(FunctionId(0))
                        .expect("atomic exchange execution failed");
                    seen.push(old.as_i64());
                }
                seen
            }));
        }

        let mut displaced = Vec::with_capacity(THREADS * ITERS);
        for h in handles {
            displaced.extend(h.join().expect("worker thread panicked"));
        }

        let mut observed = displaced;
        observed.push(unbox(cell.load(Ordering::SeqCst)));
        observed.sort_unstable();

        let mut written: Vec<i64> = (0..THREADS)
            .flat_map(|tid| std::iter::repeat_n(tid as i64, ITERS))
            .collect();
        written.push(-1); // the initial value, displaced by the first swap
        written.sort_unstable();

        assert_eq!(
            observed, written,
            "round {round}: every exchanged value must be observed exactly once"
        );
    }
}

#[test]
fn sub_word_widths_contend_correctly() {
    // 1/2/4-byte cells hold raw integers rather than NaN-boxed values
    // and take the hardware RMW path. Contend on each width.
    for (size, bits) in [(1u8, 8u32), (2, 16), (4, 32)] {
        for round in 0..ROUNDS {
            // Keep the total inside the width so the check is exact.
            let iters = 20usize;
            let total = (THREADS * iters) as u64;
            assert!(total < (1u64 << bits), "pin would overflow a {bits}-bit cell");

            let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
            let addr = cell as *const _ as usize;
            let module = rmw_module(addr, AtomicRmwOp::Add, 1, size);

            let mut handles = Vec::with_capacity(THREADS);
            for _ in 0..THREADS {
                let module = Arc::clone(&module);
                handles.push(thread::spawn(move || {
                    let mut interp = Interpreter::new(module);
                    for _ in 0..iters {
                        interp
                            .execute_function(FunctionId(0))
                            .expect("sub-word atomic RMW failed");
                    }
                }));
            }
            for h in handles {
                h.join().expect("worker thread panicked");
            }

            let mask = (1u64 << bits) - 1;
            assert_eq!(
                cell.load(Ordering::SeqCst) & mask,
                total,
                "size {size} round {round}: sub-word fetch_add lost an update"
            );
        }
    }
}

#[test]
fn rmw_on_an_untouched_cell_starts_from_zero() {
    // A cell that was never stored through holds raw zero, which is
    // bit-distinct from a NaN-boxed zero. Reading it as anything other
    // than 0 is what made a `static mut` counter's first increment
    // impossible; the RMW compares against the bits it observed, so the
    // distinction never arises.
    let cell: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
    let module = rmw_module(cell as *const _ as usize, AtomicRmwOp::Add, 7, 8);
    let mut interp = Interpreter::new(module);

    let old = interp
        .execute_function(FunctionId(0))
        .expect("atomic RMW execution failed");
    assert_eq!(old.as_i64(), 0, "an untouched cell must read as 0");
    assert_eq!(unbox(cell.load(Ordering::SeqCst)), 7);
}

#[test]
fn atomic_fetch_intrinsics_lower_to_one_indivisible_opcode() {
    // Shape pin: the registry expansion for every atomic RMW intrinsic
    // must be the single `AtomicRmw` opcode. If a load / modify /
    // compare-and-swap sequence ever comes back, the contention pins
    // above would start failing intermittently — this fails
    // deterministically instead, and names the reason.
    let names = [
        "atomic_fetch_add_u64",
        "atomic_fetch_sub_u64",
        "atomic_fetch_and_u64",
        "atomic_fetch_or_u64",
        "atomic_fetch_xor_u64",
        "atomic_fetch_add_u32",
        "atomic_fetch_sub_u32",
        "atomic_fetch_add_u16",
        "atomic_fetch_add_u8",
    ];

    for name in names {
        let intr = verum_vbc::intrinsics::INTRINSIC_REGISTRY
            .lookup(name)
            .unwrap_or_else(|| panic!("{name} missing from the intrinsic registry"));
        let body = verum_vbc::intrinsics::expand::expand_intrinsic_wrapper(intr)
            .unwrap_or_else(|| panic!("{name} has no synthesizable wrapper body"));

        let rmw_count = body
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::FfiExtended { sub_op, .. }
                        if *sub_op == SystemSubOpcode::AtomicRmw as u8
                )
            })
            .count();
        assert_eq!(rmw_count, 1, "{name} must lower to exactly one AtomicRmw");

        assert!(
            !body
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::AtomicCas { .. })),
            "{name} must not lower to a compare-and-swap sequence — that shape \
             drops updates under contention"
        );
    }
}
