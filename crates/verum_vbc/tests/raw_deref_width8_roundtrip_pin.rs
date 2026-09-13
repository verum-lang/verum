//! Width-8 round-trip pins for the raw-pointer deref opcodes (T1479 / A177).
//!
//! Verum stores every struct field as an 8-byte `Value` slot whatever
//! the field's declared type, so `&self.value as *const Int` addresses
//! the low byte of a NaN box. `DerefMutRaw` at width 8 has always
//! written the FULL box (`val_value.bits()`) "so … writes survive
//! round-trip through the raw-pointer storage" — but `DerefRaw` at the
//! same width handed those bits to `Value::from_i64`, so a read of a
//! field holding 42 answered `0x7FF9_0000_0000_002A` =
//! 9221401712017801258. Silently: a large number looks like an answer.
//!
//! Four properties, and the first two must hold TOGETHER — a change
//! that fixed the read by moving the address would break the write,
//! and a change that unboxed unconditionally would break FFI.
//!
//!   * a slot holding a boxed Int reads back as that Int;
//!   * a slot holding a genuine C `int64_t` still reads back as it is;
//!   * write-then-read through the same address round-trips, and the
//!     bytes left in memory are still the box the write chose;
//!   * widths 1/2/4 keep reaching past the tag to the payload bytes,
//!     which is what makes `AtomicU8`/`U16`/`U32` field reads correct.

use std::sync::Arc;

use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, MemSubOpcode, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::nanbox::{NAN_INTEGER_HEADER, PAYLOAD_MASK};

/// Register byte encoding for extended-opcode operand vectors —
/// mirrors `VbcCodegen::write_reg` (short `< 128`, long `0x80 |` hi).
fn write_reg(operands: &mut Vec<u8>, reg: u16) {
    if reg < 128 {
        operands.push(reg as u8);
    } else {
        operands.push(0x80 | ((reg >> 8) as u8));
        operands.push((reg & 0xFF) as u8);
    }
}

fn box_i64(v: i64) -> u64 {
    NAN_INTEGER_HEADER | ((v as u64) & PAYLOAD_MASK)
}

fn mem_extended(sub_op: MemSubOpcode, regs: &[Reg], size: u8) -> Instruction {
    let mut operands = Vec::with_capacity(regs.len() + 1);
    for r in regs {
        write_reg(&mut operands, r.0);
    }
    operands.push(size);
    Instruction::MemExtended {
        sub_op: sub_op as u8,
        operands,
    }
}

fn run(instrs: &[Instruction]) -> i64 {
    let mut bc = Vec::new();
    for instr in instrs {
        bytecode::encode_instruction(instr, &mut bc);
    }
    let mut module = VbcModule::new("raw_deref_width8_pin".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bc.len() as u32;
    func.register_count = 8;
    module.functions.push(func);
    module.bytecode = bc;

    let mut interp = Interpreter::new(Arc::new(module));
    interp
        .execute_function(FunctionId(0))
        .expect("raw deref execution failed")
        .as_i64()
}

/// `*p` where `p` addresses a Verum field slot. The eight bytes ARE the
/// NaN box, and the answer owed is the field, not the box.
#[test]
fn width8_read_of_a_boxed_int_slot_answers_the_field() {
    let slot: &'static mut u64 = Box::leak(Box::new(box_i64(42)));
    let addr = slot as *mut u64 as i64;

    let got = run(&[
        Instruction::LoadI {
            dst: Reg(0),
            value: addr,
        },
        mem_extended(MemSubOpcode::DerefRaw, &[Reg(1), Reg(0)], 8),
        Instruction::Ret { value: Reg(1) },
    ]);

    assert_eq!(
        got, 42,
        "a width-8 raw read of a field slot must answer the field; \
         9221401712017801258 is the NaN box handed back verbatim (A177)"
    );
}

/// The other half of the same width: a genuine C `int64_t` is NOT a
/// boxed value, and must keep the historical reading. Unboxing width 8
/// unconditionally would turn every FFI `long` read into a payload
/// truncation.
#[test]
fn width8_read_of_a_plain_c_int64_is_unchanged() {
    for raw in [0i64, 42, -1, 0x1234_5678_9ABC_DEF0u64 as i64, i64::MIN] {
        let slot: &'static mut i64 = Box::leak(Box::new(raw));
        let addr = slot as *mut i64 as i64;

        let got = run(&[
            Instruction::LoadI {
                dst: Reg(0),
                value: addr,
            },
            mem_extended(MemSubOpcode::DerefRaw, &[Reg(1), Reg(0)], 8),
            Instruction::Ret { value: Reg(1) },
        ]);

        // `Value` carries 48-bit inline integers; the wide constants
        // above round-trip through the boxed-int side table, so compare
        // against what the interpreter's own `from_i64` would produce.
        let want = verum_vbc::value::Value::from_i64(raw).as_i64();
        assert_eq!(
            got, want,
            "a width-8 read of a plain C int64 ({raw}) must be untouched by the \
             field-slot decode"
        );
    }
}

/// The two-sided property. The write already stored the full box; the
/// read must be its inverse, and the bytes left behind must still be
/// what the write chose — a "fix" that relocated the address would pass
/// the read half alone.
#[test]
fn width8_write_then_read_round_trips_and_leaves_the_box() {
    let slot: &'static mut u64 = Box::leak(Box::new(box_i64(42)));
    let addr = slot as *mut u64 as i64;

    let got = run(&[
        Instruction::LoadI {
            dst: Reg(0),
            value: addr,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 99,
        },
        mem_extended(MemSubOpcode::DerefMutRaw, &[Reg(0), Reg(1)], 8),
        mem_extended(MemSubOpcode::DerefRaw, &[Reg(2), Reg(0)], 8),
        Instruction::Ret { value: Reg(2) },
    ]);

    assert_eq!(got, 99, "write-then-read through one raw address must round-trip");
    assert_eq!(
        *slot,
        box_i64(99),
        "the write must still lay down the NaN box — an ordinary field read \
         of the same slot goes on reading it as a Value"
    );
}

/// Widths 1/2/4 exist to reach past the tag to the payload bytes. That
/// is what makes `AtomicU8` / `AtomicU16` / `AtomicU32` correct against
/// a Verum field, and the width-8 decode must not have disturbed it.
#[test]
fn sub_word_widths_still_read_the_payload_bytes() {
    for (width, want) in [(1u8, 0xC8i64), (2, 0x9AC8), (4, 0x1234_9AC8)] {
        let slot: &'static mut u64 = Box::leak(Box::new(box_i64(0x1234_9AC8)));
        let addr = slot as *mut u64 as i64;

        let got = run(&[
            Instruction::LoadI {
                dst: Reg(0),
                value: addr,
            },
            mem_extended(MemSubOpcode::DerefRaw, &[Reg(1), Reg(0)], width),
            Instruction::Ret { value: Reg(1) },
        ]);

        assert_eq!(
            got, want,
            "width {width} must keep zero-extending the payload's low bytes"
        );
    }
}
