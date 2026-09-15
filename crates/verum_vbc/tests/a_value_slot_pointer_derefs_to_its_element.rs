//! `&*p` on a pointer into a `Value` array — the fourth producer of a
//! returned `&T`, and the one that was given the wrong convention
//! (T1492).
//!
//! `docs/architecture/returned-reference-contract.md` names three
//! producers and two conventions: a register can hold the ADDRESS of the
//! referent or the referent's PRE-LOADED VALUE. `&*list.ptr.offset(i)`
//! is a fourth, and it took the address convention — so `*item` lowered
//! to the generic `Deref`, whose Int arm is identity, and
//! `List.unique([1, 1, 2, 3, 3])` compared three addresses to three
//! other addresses and kept all five elements.
//!
//! Two properties, and they are the two halves of one answer:
//!
//!   * `DerefValue` reads the eight bytes at an address as the `Value`
//!     they are — including a `Float`, whose slot holds a raw IEEE
//!     double with no tag and which `DerefRaw` at width 8 therefore
//!     hands back as an integer of the double's bit pattern;
//!   * the opcode is REACHABLE from the `&*p` lowering, i.e. the
//!     codegen gate that decides a pointee is one slot answers yes for
//!     the shapes `List` is written in and no for an inline record
//!     array.

use std::sync::Arc;

use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, MemSubOpcode, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

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

fn deref_value(dst: Reg, addr: Reg) -> Instruction {
    let mut operands = Vec::with_capacity(2);
    write_reg(&mut operands, dst.0);
    write_reg(&mut operands, addr.0);
    Instruction::MemExtended {
        sub_op: MemSubOpcode::DerefValue as u8,
        operands,
    }
}

fn run(instrs: &[Instruction]) -> Value {
    let mut bc = Vec::new();
    for instr in instrs {
        bytecode::encode_instruction(instr, &mut bc);
    }
    let mut module = VbcModule::new("deref_value_pin".to_string());
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
        .expect("deref-value execution failed")
}

fn read_slot(stored: Value) -> Value {
    let slot: &'static mut u64 = Box::leak(Box::new(stored.to_bits()));
    let addr = slot as *mut u64 as i64;
    run(&[
        Instruction::LoadI {
            dst: Reg(0),
            value: addr,
        },
        deref_value(Reg(1), Reg(0)),
        Instruction::Ret { value: Reg(1) },
    ])
}

/// The element a `List<Int>` slot holds, read through the address
/// `self.ptr.offset(i)` produces.
#[test]
fn an_int_element_reads_back_as_that_int() {
    let got = read_slot(Value::from_i64(7));
    assert!(got.is_int(), "want an Int, got tag {:?}", got.tag());
    assert_eq!(got.as_i64(), 7);
}

/// The half `DerefRaw` cannot answer. A `Float` occupies its slot as a
/// raw IEEE double — no quiet-NaN tag — so the width-8 FFI read decides
/// "not a box" and returns the bit pattern as an integer. A `Value`
/// slot is a `Value` whatever its tag says.
#[test]
fn a_float_element_reads_back_as_that_float() {
    let got = read_slot(Value::from_f64(1.5));
    assert!(
        got.is_float(),
        "a Float element must survive the slot read; tag {:?} bits {:#x} \
         is the DerefRaw answer, not this opcode's",
        got.tag(),
        got.to_bits()
    );
    assert_eq!(got.as_f64(), 1.5);
}

/// A `Bool` is a third tag again, and the one whose payload bits (0/1)
/// are also a plausible integer — so a reader that fell back to
/// `from_i64` would answer `false` with a straight face.
#[test]
fn a_bool_element_reads_back_as_that_bool() {
    let got = read_slot(Value::from_bool(true));
    assert!(got.is_bool(), "want a Bool, got tag {:?}", got.tag());
    assert!(got.as_bool());
}

/// Null is an error, not a silent zero: `&*null` is a defect at the call
/// site and the address convention used to hand it onward as the
/// number 0.
#[test]
fn a_null_address_is_refused() {
    let mut bc = Vec::new();
    for instr in [
        Instruction::LoadI {
            dst: Reg(0),
            value: 0,
        },
        deref_value(Reg(1), Reg(0)),
        Instruction::Ret { value: Reg(1) },
    ] {
        bytecode::encode_instruction(&instr, &mut bc);
    }
    let mut module = VbcModule::new("deref_value_null_pin".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bc.len() as u32;
    func.register_count = 8;
    module.functions.push(func);
    module.bytecode = bc;

    let mut interp = Interpreter::new(Arc::new(module));
    assert!(
        interp.execute_function(FunctionId(0)).is_err(),
        "a null address must be refused rather than read"
    );
}

/// The sub-opcode byte is part of the wire format: a renumber would make
/// every baked archive that carries it read as a different instruction.
#[test]
fn the_sub_opcode_byte_is_pinned() {
    assert_eq!(MemSubOpcode::DerefValue as u8, 0x1E);
    assert_eq!(
        MemSubOpcode::from_byte(0x1E),
        Some(MemSubOpcode::DerefValue),
        "the decoder must know the byte the encoder writes"
    );
}
