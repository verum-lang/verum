//! ARCH-P5 final leg — ONE canonical heap Text layout, Tier-0 tests.
//!

//! Every interpreter-side heap Text producer now allocates a SINGLE
//! self-contained TEXT (4) record
//! `[ObjectHeader(TEXT, size = 24 + len)]{ptr, len, cap}[bytes…]`
//! (`Heap::alloc_text` / `Heap::alloc_text_with_capacity`), retiring
//! the legacy `TypeId(0x0001)` `[len:u64][bytes…]` form — the last
//! dual representation in the Text ABI.  These tests pin:
//!
//!   * the producer stamp + record payload contents (`alloc_string`,
//!     string constants via LoadK, Concat / ToString opcodes)
//!   * `cap == 0` — text.vr's static/immutable marker (text.vr:25) —
//!     on every immutable producer, `cap > 0` + `cap + 1` byte storage
//!     on the capacity-carrying producer
//!   * `read_text` round-trips (including > SSO and multi-byte UTF-8)
//!   * equality between an `alloc_string` Text, a constant-pool Text,
//!     and a concat result (deep_value_eq record arm)
//!   * `Len` opcode + `len()`/`is_empty()` method dispatch on records
//!   * `as_bytes` of a canonical record (BYTE_SLICE composition)
//!   * `push_str` / `push` mutator intercepts on record receivers
//!   * the record reader's field-0 encoding tolerance (raw pointer
//!     bits — the `Text.from_utf8_unchecked` struct-literal path)

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{CompareOp, Instruction, Reg, RegRange, TextSubOpcode};
use verum_vbc::interpreter::{
    Interpreter, OBJECT_HEADER_SIZE, ObjectHeader, TEXT_RECORD_SIZE, text_record_cap,
    text_record_payload, value_as_byte_slice, value_as_text_record,
};
use verum_vbc::module::{Constant, FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::{StringId, TypeId};
use verum_vbc::value::Value;

// =============================================================================
// Helpers
// =============================================================================

/// Build a single-function module whose constant pool holds the given
/// strings (ConstId(i) == strings[i]).  Returns the module plus the
/// interned StringIds (usable as CallM method ids).
fn create_module(bytecode_data: Vec<u8>, strings: &[&str]) -> (Arc<VbcModule>, Vec<StringId>) {
    let mut module = VbcModule::new("test".to_string());
    let mut ids = Vec::new();
    for s in strings {
        let sid = module.strings.intern(s);
        module.constants.push(Constant::String(sid));
        ids.push(sid);
    }
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bytecode_data.len() as u32;
    func.register_count = 32;
    module.functions.push(func);
    module.bytecode = bytecode_data;
    (Arc::new(module), ids)
}

/// Two-phase builder for programs that need interned method-name ids
/// (CallM): interns `strings` as constants first, then lets `build`
/// see the module (to intern method names) before the bytecode is
/// attached.
fn create_module_two_phase(
    strings: &[&str],
    build: impl FnOnce(&mut VbcModule) -> Vec<Instruction>,
) -> Arc<VbcModule> {
    let mut module = VbcModule::new("test".to_string());
    for s in strings {
        let sid = module.strings.intern(s);
        module.constants.push(Constant::String(sid));
    }
    let instructions = build(&mut module);
    let bytecode_data = encode(&instructions);
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
    let (module, _ids) = create_module(encode(instructions), strings);
    let mut interp = Interpreter::new(module);
    let result = interp
        .execute_function(FunctionId(0))
        .expect("Execution failed");
    (result, interp)
}

/// Assert that `v` is a canonical TEXT record carrying exactly `expected`.
fn assert_canonical_record(v: &Value, expected: &str) {
    assert!(v.is_ptr(), "expected a heap Text record, got {:?}", v);
    let base = v.as_ptr::<u8>();
    let header = unsafe { &*(base as *const ObjectHeader) };
    assert_eq!(
        header.type_id,
        TypeId::TEXT,
        "heap Text must be stamped TypeId::TEXT (4)"
    );
    let (p, len) =
        value_as_text_record(v).expect("TEXT-stamped object must classify as a text record");
    assert_eq!(len, expected.len(), "record len field");
    let bytes = unsafe { std::slice::from_raw_parts(p, len) };
    assert_eq!(bytes, expected.as_bytes(), "record byte content");
}

// A >6-byte string exercises the heap record; <=6 bytes stays SSO.
const HEAP_STR: &str = "hello world, canonical record";

// =============================================================================
// Producer: alloc_string
// =============================================================================

#[test]
fn alloc_string_returns_text_stamped_record() {
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let v = interp.alloc_string(HEAP_STR).expect("alloc_string");
    let base = v.as_ptr::<u8>();
    let header = unsafe { &*(base as *const ObjectHeader) };
    assert_eq!(header.type_id, TypeId::TEXT);
    // Single-allocation contract: size = 24 (record head) + byte_len.
    assert_eq!(header.size as usize, TEXT_RECORD_SIZE + HEAP_STR.len());
    // Record fields: ptr points INTO the same allocation at payload+24.
    let (p, len) = unsafe { text_record_payload(base) }.expect("record payload");
    let expected_bytes_addr =
        base as usize + OBJECT_HEADER_SIZE + TEXT_RECORD_SIZE;
    assert_eq!(p as usize, expected_bytes_addr, "self-contained ptr");
    assert_eq!(len, HEAP_STR.len());
    // cap == 0: the static/immutable marker (text.vr:25) — the object
    // owns inline storage that must never reach the .vr allocator's
    // dealloc, and mutation COW-promotes.
    assert_eq!(unsafe { text_record_cap(base) }, 0);
    let bytes = unsafe { std::slice::from_raw_parts(p, len) };
    assert_eq!(bytes, HEAP_STR.as_bytes());
}

#[test]
fn alloc_string_small_stays_sso() {
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let v = interp.alloc_string("abc").expect("alloc_string");
    assert!(v.is_small_string(), "<= 6 bytes must remain SSO");
}

#[test]
fn read_text_roundtrips_alloc_string() {
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    for s in [
        "",
        "abc",
        HEAP_STR,
        "unicode Ω≈ç√∫ string, multi-byte and heap-sized",
    ] {
        let v = interp.alloc_string(s).expect("alloc_string");
        assert_eq!(interp.read_text(v).as_deref(), Some(s), "round-trip {s:?}");
    }
}

#[test]
fn read_text_roundtrips_large_string_beyond_former_heuristic_cap() {
    // The retired readers refused > 1_000_000-byte strings (a
    // heuristic-era guard); the typed record is trusted at any length.
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let big = "x".repeat(1_500_000);
    let v = interp.alloc_string(&big).expect("alloc_string");
    assert_canonical_record(&v, &big);
    assert_eq!(interp.read_text(v).as_deref(), Some(big.as_str()));
}

// =============================================================================
// Producer: string constants (LoadK → load_constant)
// =============================================================================

#[test]
fn string_constant_realizes_as_canonical_record() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::Ret { value: Reg(0) },
        ],
        &[HEAP_STR],
    );
    assert_canonical_record(&result, HEAP_STR);
    // Constants are immutable literals — cap == 0 mirrors the AOT
    // rodata Text globals' `{ptr, len, cap=0}` marker.
    let base = result.as_ptr::<u8>();
    assert_eq!(unsafe { text_record_cap(base) }, 0);
}

// =============================================================================
// Producer: Concat / ToString opcodes
// =============================================================================

#[test]
fn concat_produces_canonical_record_and_content() {
    let (result, interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::LoadK {
                dst: Reg(1),
                const_id: 1,
            },
            Instruction::Concat {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ],
        &["hello world, ", "concat tail"],
    );
    assert_canonical_record(&result, "hello world, concat tail");
    assert_eq!(
        interp.read_text(result).as_deref(),
        Some("hello world, concat tail")
    );
}

#[test]
fn concat_eq_literal_cross_producer() {
    // deep_value_eq between a Concat-produced record and a
    // constant-pool record (both canonical, different producers).
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::LoadK {
                dst: Reg(1),
                const_id: 1,
            },
            Instruction::Concat {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::LoadK {
                dst: Reg(3),
                const_id: 2,
            },
            Instruction::CmpI {
                dst: Reg(4),
                a: Reg(2),
                b: Reg(3),
                op: CompareOp::Eq,
            },
            Instruction::Ret { value: Reg(4) },
        ],
        &["hello world, ", "concat tail", "hello world, concat tail"],
    );
    assert!(result.is_bool() && result.as_bool(), "concat == literal");
}

#[test]
fn alloc_string_eq_constant_pool_text() {
    // Equality between an `alloc_string` record (host-side producer)
    // and a load_constant record, via the interpreter's deep equality
    // on registers.
    let (module, _) = create_module(
        encode(&[
            Instruction::LoadK {
                dst: Reg(1),
                const_id: 0,
            },
            Instruction::CmpI {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
                op: CompareOp::Eq,
            },
            Instruction::Ret { value: Reg(2) },
        ]),
        &[HEAP_STR],
    );
    let mut interp = Interpreter::new(module);
    let v = interp.alloc_string(HEAP_STR).expect("alloc_string");
    let result = interp
        .execute_function_with_args(FunctionId(0), &[v])
        .expect("execute");
    assert!(result.is_bool() && result.as_bool(), "alloc_string == constant");
}

// =============================================================================
// Len opcode + len()/is_empty() dispatch on records
// =============================================================================

#[test]
fn len_opcode_reads_record_field() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::Len {
                dst: Reg(1),
                arr: Reg(0),
                type_hint: 0,
            },
            Instruction::Ret { value: Reg(1) },
        ],
        &[HEAP_STR],
    );
    assert!(result.is_int());
    assert_eq!(result.as_i64() as usize, HEAP_STR.len());
}

// =============================================================================
// as_bytes composition (BYTE_SLICE over the record's inline bytes)
// =============================================================================

#[test]
fn as_bytes_of_canonical_record_composes() {
    let (result, _interp) = run_with_strings(
        &[
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::TextExtended {
                sub_op: TextSubOpcode::AsBytes as u8,
                operands: vec![1, 0],
            },
            Instruction::Ret { value: Reg(1) },
        ],
        &[HEAP_STR],
    );
    let (p, len) = value_as_byte_slice(&result)
        .expect("as_bytes of a record must produce a BYTE_SLICE view");
    assert_eq!(len as usize, HEAP_STR.len());
    let bytes = unsafe { std::slice::from_raw_parts(p, len as usize) };
    assert_eq!(bytes, HEAP_STR.as_bytes());
}

// =============================================================================
// Mutator intercepts on record receivers (push_str / push)
// =============================================================================

#[test]
fn push_str_on_record_receiver_appends() {
    // r0 = heap record, r1 = arg text; CallM push_str writes the new
    // record back into the receiver register.  The method_id is the
    // interned StringId of "push_str" (the string table pre-interns
    // standard names, so the id is discovered, not assumed).
    let module = create_module_two_phase(&["hello world, mutable ", "tail bytes"], |m| {
        let push_str_id = m.strings.intern("push_str").0;
        vec![
            Instruction::LoadK {
                dst: Reg(0),
                const_id: 0,
            },
            Instruction::LoadK {
                dst: Reg(1),
                const_id: 1,
            },
            Instruction::CallM {
                dst: Reg(2),
                receiver: Reg(0),
                method_id: push_str_id,
                args: RegRange::new(Reg(1), 1),
            },
            Instruction::Ret { value: Reg(0) },
        ]
    });
    let mut interp = Interpreter::new(module);
    let result = interp
        .execute_function(FunctionId(0))
        .expect("Execution failed");
    assert_eq!(
        interp.read_text(result).as_deref(),
        Some("hello world, mutable tail bytes")
    );
    assert_canonical_record(&result, "hello world, mutable tail bytes");
}

#[test]
fn push_on_fresh_empty_text() {
    // Empty record (cap == 0, ptr = static empty buffer): push must
    // COW-promote, never write through the empty pointer.
    let module = create_module_two_phase(&["appended-after-empty"], |m| {
        let push_str_id = m.strings.intern("push_str").0;
        vec![
            Instruction::LoadK {
                dst: Reg(1),
                const_id: 0,
            },
            Instruction::CallM {
                dst: Reg(2),
                receiver: Reg(0),
                method_id: push_str_id,
                args: RegRange::new(Reg(1), 1),
            },
            Instruction::Ret { value: Reg(0) },
        ]
    });
    let mut interp = Interpreter::new(module);
    // Build the canonical EMPTY record host-side and pass it as r0.
    let empty = {
        let obj = interp.state.heap.alloc_text(&[]).expect("alloc_text");
        Value::from_ptr(obj.as_ptr() as *mut u8)
    };
    assert_eq!(interp.read_text(empty).as_deref(), Some(""));
    let base = empty.as_ptr::<u8>();
    let (p, len) = unsafe { text_record_payload(base) }.expect("record");
    assert!(!p.is_null(), "empty record ptr is never null");
    assert_eq!(len, 0);
    assert_eq!(unsafe { text_record_cap(base) }, 0);
    let result = interp
        .execute_function_with_args(FunctionId(0), &[empty])
        .expect("execute");
    assert_eq!(
        interp.read_text(result).as_deref(),
        Some("appended-after-empty")
    );
}

// =============================================================================
// Capacity-carrying producer (with_capacity / reserve surface)
// =============================================================================

#[test]
fn alloc_text_with_capacity_layout_contract() {
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let obj = interp
        .state
        .heap
        .alloc_text_with_capacity(b"seed", 64)
        .expect("alloc_text_with_capacity");
    let base = obj.as_ptr() as *mut u8;
    let header = unsafe { &*(base as *const ObjectHeader) };
    assert_eq!(header.type_id, TypeId::TEXT);
    // Owned-buffer convention: storage reserves cap + 1 bytes (the
    // text.vr NUL slot), so size = 24 + cap + 1.
    assert_eq!(header.size as usize, TEXT_RECORD_SIZE + 64 + 1);
    let (p, len) = unsafe { text_record_payload(base) }.expect("record");
    assert_eq!(len, 4);
    assert_eq!(unsafe { text_record_cap(base) }, 64);
    assert_eq!(
        p as usize,
        base as usize + OBJECT_HEADER_SIZE + TEXT_RECORD_SIZE,
        "self-contained storage"
    );
    let v = Value::from_ptr(base);
    assert_eq!(interp.state.read_text(v).as_deref(), Some("seed"));
}

// =============================================================================
// Record-reader field-0 tolerance (raw pointer bits)
// =============================================================================

#[test]
fn record_reader_tolerates_raw_pointer_field0() {
    // The `Text.from_utf8_unchecked` struct-literal path stores the
    // `&unsafe Byte` field as RAW pointer bits (no NaN box).  The
    // canonical reader must recover the address.
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let backing: &'static [u8] = b"raw-field0 external bytes";
    let obj = interp
        .state
        .heap
        .alloc(TypeId::TEXT, TEXT_RECORD_SIZE)
        .expect("alloc");
    let base = obj.as_ptr() as *mut u8;
    unsafe {
        let data = base.add(OBJECT_HEADER_SIZE);
        // Raw pointer bits in slot 0, NaN-boxed Int len in slot 1,
        // Int cap in slot 2.
        *(data as *mut u64) = backing.as_ptr() as u64;
        *((data as *mut Value).add(1)) = Value::from_i64(backing.len() as i64);
        *((data as *mut Value).add(2)) = Value::from_i64(0);
    }
    let v = Value::from_ptr(base);
    let (p, len) = value_as_text_record(&v).expect("record with raw field0");
    assert_eq!(p as usize, backing.as_ptr() as usize);
    assert_eq!(len, backing.len());
    assert_eq!(
        interp.state.read_text(v).as_deref(),
        Some("raw-field0 external bytes")
    );
}

// =============================================================================
// Cross-tier stamp pin
// =============================================================================

#[test]
fn text_record_stamp_pinned() {
    // TypeId::TEXT is 4 on both tiers; the record head is 24 bytes
    // ({ptr@0, len@8, cap@16} — verum_common::layout::TEXT_SIZE), and
    // the immutable producer appends the bytes in the SAME allocation.
    assert_eq!(TypeId::TEXT.0, 4);
    assert_eq!(TEXT_RECORD_SIZE, 24);
    let (module, _) = create_module(encode(&[Instruction::Ret { value: Reg(0) }]), &[]);
    let mut interp = Interpreter::new(module);
    let v = interp.alloc_string(HEAP_STR).expect("alloc_string");
    let base = v.as_ptr::<u8>();
    let header = unsafe { &*(base as *const ObjectHeader) };
    assert_eq!(header.size as usize, TEXT_RECORD_SIZE + HEAP_STR.len());
}
