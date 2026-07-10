//! ARCH-P5 BYTE_SLICE (528) representation-tagged byte view — Tier-0 tests.
//!

//! `Text.as_bytes()` (TextExtended::AsBytes) produces a BYTE_SLICE heap
//! object `[ObjectHeader(528)][ptr: i64][len: i64]` with RAW payload
//! slots — bit-identical to the Tier-1 AsBytes slice Pack stamped by
//! `lower_pack_typed`.  These tests pin the producer's object form and
//! every Tier-0 consumer arm that replaced the retired
//! `len <= 1_000_000` FatRef-as-Text heuristic:
//!
//!   * producer stamp + payload contents (heap-string, small-string,
//!     empty text — never-null contract)
//!   * `Len` opcode + `GetE` indexing (bounds-checked byte reads)
//!   * `IterNew` / `IterNext` byte iteration (ITER_TYPE_BYTE_SLICE)
//!   * CbgrExtended `SliceLen` / `SliceGet` / `SliceSubslice` /
//!     `SliceSplitAt` / `Unslice` — re-slicing produces NEW BYTE_SLICE
//!     objects (subslice-of-subslice chains, the HttpParser.feed
//!     `&buf[pos..]` pattern)
//!   * equality (`CmpI Eq` → deep_value_eq byte-range comparison)
//!   * `InterpreterState::read_text` on a byte view

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{CompareOp, Instruction, Reg, TextSubOpcode};
use verum_vbc::interpreter::{Interpreter, value_as_byte_slice};
use verum_vbc::module::{Constant, FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::{StringId, TypeId};
use verum_vbc::value::Value;

// =============================================================================
// Helpers
// =============================================================================

/// Build a single-function module whose constant pool holds the given
/// strings (ConstId(i) == strings[i]).
fn create_module(bytecode_data: Vec<u8>, strings: &[&str]) -> Arc<VbcModule> {
    let mut module = VbcModule::new("test".to_string());
    for s in strings {
        let sid = module.strings.intern(s);
        module.constants.push(Constant::String(sid));
    }
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_data.len() as u32;
    func.register_count = 32;
    module.functions.push(func);
    module.bytecode = bytecode_data;
    Arc::new(module)
}

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

/// Run a program; returns (result, interpreter) so heap-backed results
/// stay live for inspection.
fn run_with_strings(instructions: &[Instruction], strings: &[&str]) -> (Value, Interpreter) {
    let module = create_module(encode(instructions), strings);
    let mut interp = Interpreter::new(module);
    let result = interp
        .execute_function(FunctionId(0))
        .expect("Execution failed");
    (result, interp)
}

/// `TextExtended AsBytes dst, text` — operands are raw reg bytes
/// (regs < 128 encode as one byte).
fn as_bytes_instr(dst: u8, text: u8) -> Instruction {
    Instruction::TextExtended {
        sub_op: TextSubOpcode::AsBytes as u8,
        operands: vec![dst, text],
    }
}

/// CbgrExtended carrier with raw single-byte reg operands.
fn cbgr_instr(sub_op: u8, operands: Vec<u8>) -> Instruction {
    Instruction::CbgrExtended { sub_op, operands }
}

const CBGR_SLICE_LEN: u8 = 0x05;
const CBGR_SLICE_GET: u8 = 0x06;
const CBGR_SLICE_SUBSLICE: u8 = 0x08;
const CBGR_SLICE_SPLIT_AT: u8 = 0x09;

// A >6-byte string exercises the heap-string (`[len:u64][bytes…]`)
// Text representation; <=6 bytes exercises the NaN-boxed small string.
const HEAP_STR: &str = "hello world, byte view";

// =============================================================================
// Producer: object form
// =============================================================================

#[test]
fn as_bytes_returns_byte_slice_stamped_object() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::Ret { value: Reg(1) },
        ],
        &[HEAP_STR],
    );
    // NOT a bare FatRef anymore (the retired producer shape) …
    assert!(!result.is_fat_ref(), "as_bytes must not produce a bare FatRef");
    // … but a heap object stamped BYTE_SLICE with a raw {ptr, len} payload.
    let (ptr, len) = value_as_byte_slice(&result)
        .expect("as_bytes result must classify as a BYTE_SLICE object");
    assert_eq!(len as usize, HEAP_STR.len());
    assert!(!ptr.is_null());
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    assert_eq!(bytes, HEAP_STR.as_bytes());
    // Header stamp is exactly TypeId::BYTE_SLICE (cross-tier pin: the
    // AOT lowering stamps the same constant via lower_pack_typed).
    let header_tid = unsafe {
        verum_vbc::interpreter::ObjectHeader::try_type_id(result.as_ptr::<u8>())
    };
    assert_eq!(header_tid, Some(TypeId::BYTE_SLICE));
}

#[test]
fn as_bytes_of_small_string_copies_inline_bytes() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::Ret { value: Reg(1) },
        ],
        &["abc"],
    );
    let (ptr, len) = value_as_byte_slice(&result).expect("BYTE_SLICE expected");
    assert_eq!(len, 3);
    let bytes = unsafe { std::slice::from_raw_parts(ptr, 3) };
    assert_eq!(bytes, b"abc");
}

#[test]
fn as_bytes_of_empty_text_is_never_null() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::Ret { value: Reg(1) },
        ],
        &[""],
    );
    let (ptr, len) = value_as_byte_slice(&result).expect("BYTE_SLICE expected");
    assert_eq!(len, 0);
    assert!(
        !ptr.is_null(),
        "empty byte view must point at the static empty buffer (never-null contract)"
    );
}

// =============================================================================
// Len + GetE
// =============================================================================

#[test]
fn len_opcode_reads_byte_slice_length() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::Len {
                dst: Reg(2),
                arr: Reg(1),
                type_hint: 0,
            },
            Instruction::Ret { value: Reg(2) },
        ],
        &[HEAP_STR],
    );
    assert_eq!(result.as_i64(), HEAP_STR.len() as i64);
}

#[test]
fn gete_indexes_byte_slice() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 4,
            },
            Instruction::GetE {
                dst: Reg(3),
                arr: Reg(1),
                idx: Reg(2),
            },
            Instruction::Ret { value: Reg(3) },
        ],
        &[HEAP_STR],
    );
    assert_eq!(result.as_i64(), HEAP_STR.as_bytes()[4] as i64);
}

#[test]
fn gete_out_of_bounds_panics_cleanly() {
    let module = create_module(
        encode(&[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 3, // len("abc") == 3 → OOB
            },
            Instruction::GetE {
                dst: Reg(3),
                arr: Reg(1),
                idx: Reg(2),
            },
            Instruction::Ret { value: Reg(3) },
        ]),
        &["abc"],
    );
    let mut interp = Interpreter::new(module);
    let err = interp.execute_function(FunctionId(0));
    assert!(err.is_err(), "OOB byte-slice index must error, not read past the view");
}

// =============================================================================
// Iteration (ITER_TYPE_BYTE_SLICE)
// =============================================================================

#[test]
fn iter_new_next_walks_byte_slice() {
    // "abc" → iterate 3 bytes, then exhaustion. Pre-ARCH-P5 this
    // SIGSEGV'd: the FatRef iterable fell into ITER_TYPE_LIST and its
    // marker payload was read as a List header.
    let (sum, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::IterNew {
                dst: Reg(2),
                iterable: Reg(1),
            },
            // Three IterNext calls: r3=byte, r4=has_next.
            Instruction::IterNext {
                dst: Reg(3),
                has_next: Reg(4),
                iter: Reg(2),
            },
            Instruction::IterNext {
                dst: Reg(5),
                has_next: Reg(6),
                iter: Reg(2),
            },
            Instruction::IterNext {
                dst: Reg(7),
                has_next: Reg(8),
                iter: Reg(2),
            },
            // sum = b0 + b1 + b2
            Instruction::BinaryI {
                op: verum_vbc::instruction::BinaryIntOp::Add,
                dst: Reg(9),
                a: Reg(3),
                b: Reg(5),
            },
            Instruction::BinaryI {
                op: verum_vbc::instruction::BinaryIntOp::Add,
                dst: Reg(10),
                a: Reg(9),
                b: Reg(7),
            },
            Instruction::Ret { value: Reg(10) },
        ],
        &["abc"],
    );
    assert_eq!(sum.as_i64(), b'a' as i64 + b'b' as i64 + b'c' as i64);
}

#[test]
fn iter_next_reports_exhaustion() {
    let (has_next, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::IterNew {
                dst: Reg(2),
                iterable: Reg(1),
            },
            Instruction::IterNext {
                dst: Reg(3),
                has_next: Reg(4),
                iter: Reg(2),
            },
            // Second next on a 1-byte view → exhausted.
            Instruction::IterNext {
                dst: Reg(5),
                has_next: Reg(6),
                iter: Reg(2),
            },
            Instruction::Ret { value: Reg(6) },
        ],
        &["x"],
    );
    assert!(has_next.is_bool());
    assert!(!has_next.as_bool(), "1-byte view must exhaust after one IterNext");
}

// =============================================================================
// CbgrExtended slice surface: SliceLen / SliceGet / Subslice / SplitAt
// =============================================================================

#[test]
fn slice_len_and_get_read_byte_slice() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            // r2 = SliceLen(r1)
            cbgr_instr(CBGR_SLICE_LEN, vec![2, 1]),
            // r3 = 6 (index); r4 = SliceGet(r1, r3)
            Instruction::LoadI {
                dst: Reg(3),
                value: 6,
            },
            cbgr_instr(CBGR_SLICE_GET, vec![4, 1, 3]),
            // ret = len * 1000 + byte
            Instruction::LoadI {
                dst: Reg(5),
                value: 1000,
            },
            Instruction::BinaryI {
                op: verum_vbc::instruction::BinaryIntOp::Mul,
                dst: Reg(6),
                a: Reg(2),
                b: Reg(5),
            },
            Instruction::BinaryI {
                op: verum_vbc::instruction::BinaryIntOp::Add,
                dst: Reg(7),
                a: Reg(6),
                b: Reg(4),
            },
            Instruction::Ret { value: Reg(7) },
        ],
        &[HEAP_STR],
    );
    let expected = (HEAP_STR.len() as i64) * 1000 + HEAP_STR.as_bytes()[6] as i64;
    assert_eq!(result.as_i64(), expected);
}

#[test]
fn subslice_produces_new_byte_slice() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 6,
            },
            Instruction::LoadI {
                dst: Reg(3),
                value: 11,
            },
            // r4 = r1[6..11]
            cbgr_instr(CBGR_SLICE_SUBSLICE, vec![4, 1, 2, 3]),
            Instruction::Ret { value: Reg(4) },
        ],
        &[HEAP_STR],
    );
    let (ptr, len) = value_as_byte_slice(&result)
        .expect("subslice of a BYTE_SLICE must be a BYTE_SLICE");
    assert_eq!(len, 5);
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    assert_eq!(bytes, &HEAP_STR.as_bytes()[6..11]);
}

#[test]
fn subslice_of_subslice_chains() {
    // The HttpParser.feed pattern: re-slice a re-slice.
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 6,
            },
            Instruction::LoadI {
                dst: Reg(3),
                value: 17,
            },
            // r4 = bytes[6..17]  (11 bytes)
            cbgr_instr(CBGR_SLICE_SUBSLICE, vec![4, 1, 2, 3]),
            Instruction::LoadI {
                dst: Reg(5),
                value: 2,
            },
            Instruction::LoadI {
                dst: Reg(6),
                value: 7,
            },
            // r7 = r4[2..7]  (== bytes[8..13])
            cbgr_instr(CBGR_SLICE_SUBSLICE, vec![7, 4, 5, 6]),
            Instruction::Ret { value: Reg(7) },
        ],
        &[HEAP_STR],
    );
    let (ptr, len) = value_as_byte_slice(&result)
        .expect("subslice-of-subslice must stay a BYTE_SLICE");
    assert_eq!(len, 5);
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    assert_eq!(bytes, &HEAP_STR.as_bytes()[8..13]);
}

#[test]
fn subslice_out_of_bounds_errors() {
    let module = create_module(
        encode(&[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 0,
            },
            Instruction::LoadI {
                dst: Reg(3),
                value: 4, // len("abc") == 3 → end OOB
            },
            cbgr_instr(CBGR_SLICE_SUBSLICE, vec![4, 1, 2, 3]),
            Instruction::Ret { value: Reg(4) },
        ]),
        &["abc"],
    );
    let mut interp = Interpreter::new(module);
    assert!(interp.execute_function(FunctionId(0)).is_err());
}

#[test]
fn split_at_produces_two_byte_slices() {
    // Split "abcdef…" at 6; return the RIGHT half and inspect it.
    let (right, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::LoadI {
                dst: Reg(2),
                value: 6,
            },
            // dst1=r3, dst2=r4, src=r1, mid=r2
            cbgr_instr(CBGR_SLICE_SPLIT_AT, vec![3, 4, 1, 2]),
            Instruction::Ret { value: Reg(4) },
        ],
        &[HEAP_STR],
    );
    let (ptr, len) = value_as_byte_slice(&right).expect("split_at right must be BYTE_SLICE");
    assert_eq!(len as usize, HEAP_STR.len() - 6);
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    assert_eq!(bytes, &HEAP_STR.as_bytes()[6..]);
}

// =============================================================================
// Equality + read_text
// =============================================================================

#[test]
fn byte_slices_of_equal_texts_compare_equal() {
    let instrs = [
        Instruction::LoadK {
            dst: Reg(0),
            const_id: 0,
        },
        Instruction::LoadK {
            dst: Reg(1),
            const_id: 1,
        },
        as_bytes_instr(2, 0),
        as_bytes_instr(3, 1),
        Instruction::CmpI {
            op: CompareOp::Eq,
            dst: Reg(4),
            a: Reg(2),
            b: Reg(3),
        },
        Instruction::Ret { value: Reg(4) },
    ];
    let (eq, _interp) = run_with_strings(&instrs, &[HEAP_STR, HEAP_STR]);
    assert!(eq.as_bool(), "byte views of identical text must compare equal");

    let (ne, _interp2) = run_with_strings(&instrs, &[HEAP_STR, "different payload bytes"]);
    assert!(!ne.as_bool(), "byte views of different text must compare unequal");
}

#[test]
fn read_text_reads_byte_slice_as_text() {
    let (result, interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            as_bytes_instr(1, 0),
            Instruction::Ret { value: Reg(1) },
        ],
        &[HEAP_STR],
    );
    assert_eq!(interp.state.read_text(result).as_deref(), Some(HEAP_STR));
}

// =============================================================================
// Heap helper round-trip (producer/classifier agreement)
// =============================================================================

#[test]
fn value_as_byte_slice_rejects_non_byte_slice_values() {
    assert!(value_as_byte_slice(&Value::from_i64(42)).is_none());
    assert!(value_as_byte_slice(&Value::nil()).is_none());
    assert!(value_as_byte_slice(&Value::from_bool(true)).is_none());
    let small = Value::from_small_string("abc").unwrap();
    assert!(value_as_byte_slice(&small).is_none());
}
