//! T1359 layer 5 — a record-typed FIELD must travel to C and back.
//!
//! The layout half of T1359 (`t1359_ffi_nested_struct_layout.rs`) makes
//! the OFFSETS right. This is the other half: `marshal_field_to_c` and
//! `marshal_field_from_c` answered `{}` and `None` for
//! `CType::StructValue`, so a nested record was silently skipped in
//! BOTH directions — a `DarwinStat.st_mtime` reached `fstat` as sixteen
//! zero bytes and never came back, and `core/io/fs.vr`'s `metadata()`
//! reported every file as modified at the epoch. A plausible wrong
//! value with no diagnostic, which is the expensive kind.
//!
//! The layouts here come from the CODEGEN, not from a hand-built
//! fixture. That is deliberate and is the T1360 lesson applied: the
//! test that should have caught T1360 built its `FfiStructLayout` by
//! hand and so asserted a property of a path the value never travelled.
//! A gate on the marshaller must be fed by the producer.
//!
//! The Verum-side objects ARE hand-built — a byte buffer shaped
//! `[ObjectHeader | slot0 | slot1 | ...]` — because these functions read
//! and write slots at `HEADER + i*8` and never inspect the header. The
//! test states that rather than implying a real heap allocation.
//!
//! WHERE THIS RUNS, checked and not assumed (T1349's lesson). The
//! marshaller lives behind the `ffi` feature — `ffi/mod.rs` gates
//! `pub mod runtime` on it — and `ffi` is NOT in the default set and was
//! in NO CI job's feature list. It reaches a CI compiler only
//! transitively, because `verum_compiler` declares
//! `verum_vbc = { features = ["codegen", "ffi"] }`, so feature
//! unification switches it on inside THAT build graph. Compiled there,
//! gated nowhere.
//!
//! So this file is gated on BOTH features and the guardrails job's step
//! now passes `--features codegen,ffi`. Without that word the file
//! compiles to an empty test binary and reports success having run
//! nothing — the exact shape T1349 measured on five other files.
#![cfg(all(feature = "codegen", feature = "ffi"))]

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::ffi::runtime::{marshal_c_to_verum_struct_at, marshal_verum_struct_to_c_at};
use verum_vbc::interpreter::{OBJECT_HEADER_SIZE, ObjectHeader};
use verum_vbc::types::TypeId;
use verum_vbc::module::{CType, FfiStructField, FfiStructLayout, VbcModule};
use verum_vbc::value::Value;

/// `Outer` mirrors the shape that matters: a scalar, a nested record,
/// then another scalar AFTER it — the third field is the one whose
/// offset the nested field's size decides.
const SOURCE: &str = "module t;\n\
    \n\
    @ffi(\"libSystem.B.dylib\")\n\
    extern {\n\
    \x20   fn probe(p: &unsafe Outer) -> Int32;\n\
    }\n\
    \n\
    type Inner is { lo: Int, hi: Int };\n\
    type Outer is { head: Int64, mid: Inner, tail: Int64 };\n";

// C layout of `Outer` under natural alignment:
//   head  off 0   (8)
//   mid   off 8   (16)  = Inner { lo off 0, hi off 8 }
//   tail  off 24  (8)
//   size  32
const OFF_HEAD: usize = 0;
const OFF_MID_LO: usize = 8;
const OFF_MID_HI: usize = 16;
const OFF_TAIL: usize = 24;

fn compile() -> VbcModule {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(SOURCE, file_id);
    let parser = VerumParser::new();
    let ast = parser.parse_module(lexer, file_id).unwrap_or_else(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| format!("{}", e)).collect();
        panic!("parse failed:\n{}", msgs.join("\n"))
    });
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("t1359m"));
    codegen
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile failed: {}", e))
}

fn layout<'a>(m: &'a VbcModule, name: &str) -> &'a FfiStructLayout {
    m.ffi_layouts
        .iter()
        .find(|l| m.strings.get(l.name) == Some(name))
        .unwrap_or_else(|| {
            let have: Vec<Option<&str>> = m.ffi_layouts.iter().map(|l| m.strings.get(l.name)).collect();
            panic!(
                "no layout named `{name}`; {} present, names resolve to {have:?}",
                m.ffi_layouts.len()
            )
        })
}

/// A Verum heap object as these functions see it: a header they never
/// read, followed by `slots` NaN-boxed `Value`s.
///
/// Backed by `Vec<u64>`, not `Vec<u8>`, and that is not a style choice:
/// a `Vec<u8>` has alignment 1, and both the `Value` slots and the
/// pointer that a parent object's slot holds must be 8-aligned for the
/// NaN box to survive the round trip. `OBJECT_HEADER_SIZE` is 24, three
/// words, so the slot area stays word-aligned too.
struct FakeObject {
    words: Vec<u64>,
}

const HEADER_WORDS: usize = OBJECT_HEADER_SIZE / std::mem::size_of::<u64>();

/// Words written past the object's declared data area and checked for
/// change. They stand in for a fault: this fixture cannot put the object
/// at the end of a page, so an overrun is detected by SENTINEL rather
/// than by SIGSEGV. Said plainly because the difference matters — a
/// sentinel proves a write happened, a fault proves it left the
/// allocation.
const GUARD_WORDS: usize = 4;

/// The sentinel must be a VALID NaN-boxed value, not a memorable bit
/// pattern, and that is not decoration.
///
/// Measured while taking this test's polarity: with `0xDEADBEEF…` in the
/// guard words, reverting the bound made the to-C leg read one of them
/// and call `as_i64()` on it, which `debug_assert`s in `value.rs` — the
/// test went red on the READ and never reached the write-back leg, so
/// the sentinel it exists for was never exercised. A readable sentinel
/// lets the out-of-bounds READ pass through harmlessly (a read changes
/// nothing a sentinel can see anyway) and leaves the out-of-bounds
/// WRITE to trip it, which is the half a sentinel can actually prove.
fn guard_value() -> Value {
    Value::from_i64(0x5E5E_5E5E)
}

impl FakeObject {
    fn new(slots: usize) -> Self {
        assert_eq!(
            OBJECT_HEADER_SIZE % std::mem::size_of::<u64>(),
            0,
            "the header must be a whole number of words for this fixture to be aligned"
        );
        let mut words = vec![0u64; HEADER_WORDS + slots + GUARD_WORDS];
        for w in words.iter_mut().skip(HEADER_WORDS + slots) {
            *w = guard_value().to_bits();
        }
        let mut me = Self { words };
        // A REAL header, not zeros. `size` is what the marshaller now
        // bounds the slot walk against (T1361); a zeroed header declares
        // an object with no data and every field is correctly skipped —
        // which would make every test in this file pass for the wrong
        // reason.
        let header = ObjectHeader::new(
            TypeId(1),
            0,
            (slots * std::mem::size_of::<Value>()) as u32,
        );
        // SAFETY: `words` is 8-aligned (Vec<u64>) and at least
        // HEADER_WORDS long, which is `OBJECT_HEADER_SIZE` bytes.
        unsafe {
            *(me.words.as_mut_ptr() as *mut ObjectHeader) = header;
        }
        me
    }

    /// True while nothing has been written past the declared data area.
    fn guards_intact(&self) -> bool {
        self.words
            .iter()
            .skip(HEADER_WORDS + self.slots())
            .all(|w| *w == guard_value().to_bits())
    }

    fn slots(&self) -> usize {
        self.words.len() - HEADER_WORDS - GUARD_WORDS
    }
    fn ptr(&mut self) -> *mut u8 {
        self.words.as_mut_ptr() as *mut u8
    }
    fn set(&mut self, slot: usize, v: Value) {
        // SAFETY: `words` was sized for the slot count the caller asked
        // for; callers below stay inside it, and the base is 8-aligned
        // because it came from a `Vec<u64>`.
        unsafe {
            let p = self.words.as_mut_ptr().add(HEADER_WORDS) as *mut Value;
            *p.add(slot) = v;
        }
    }
    fn get(&self, slot: usize) -> Value {
        // SAFETY: as above.
        unsafe {
            let p = self.words.as_ptr().add(HEADER_WORDS) as *const Value;
            *p.add(slot)
        }
    }
}

fn read_i64(buf: &[u8; 256], off: usize) -> i64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&buf[off..off + 8]);
    i64::from_le_bytes(w)
}

fn write_i64(buf: &mut [u8; 256], off: usize, v: i64) {
    buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

/// The layout half, restated here so a failure in THIS file is not read
/// as a marshalling defect when it is a layout one.
#[test]
fn the_layout_this_gate_stands_on_is_the_c_one() {
    let m = compile();
    let outer = layout(&m, "Outer");
    assert_eq!(outer.fields.len(), 3);
    assert_eq!(outer.fields[0].offset, OFF_HEAD as u32);
    assert_eq!(outer.fields[1].offset, OFF_MID_LO as u32);
    assert_eq!(outer.fields[1].size, 16, "the nested record must be 16 bytes");
    assert_eq!(outer.fields[2].offset, OFF_TAIL as u32);
    assert_eq!(outer.size, 32);
    assert!(outer.fields[1].nested_layout.is_some());
}

/// Verum -> C. Pre-fix the nested record contributed nothing, so the C
/// side saw zeros where the caller had put values.
#[test]
fn a_nested_record_reaches_c() {
    let m = compile();
    let outer_layout = layout(&m, "Outer");

    let mut inner = FakeObject::new(2);
    inner.set(0, Value::from_i64(0x1111));
    inner.set(1, Value::from_i64(0x2222));

    let mut outer = FakeObject::new(3);
    outer.set(0, Value::from_i64(0x0BAD));
    outer.set(1, Value::from_ptr(inner.ptr()));
    outer.set(2, Value::from_i64(0x0F00));

    let mut buf = [0u8; 256];
    // SAFETY: both objects are sized for their slot counts above; `buf`
    // is 256 bytes and the layout is 32.
    unsafe {
        marshal_verum_struct_to_c_at(
            outer_layout,
            &m.ffi_layouts,
            outer.ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            0,
            0,
        );
    }

    assert_eq!(read_i64(&buf, OFF_HEAD), 0x0BAD, "scalar before the nested field");
    assert_eq!(
        read_i64(&buf, OFF_MID_LO),
        0x1111,
        "nested field never reached C — this is the T1359 layer-5 defect verbatim"
    );
    assert_eq!(read_i64(&buf, OFF_MID_HI), 0x2222, "nested field, second word");
    assert_eq!(read_i64(&buf, OFF_TAIL), 0x0F00, "scalar after the nested field");
}

/// C -> Verum, which is the leg `metadata()` depends on: the kernel
/// filled `struct stat`, and the timespecs must come back.
#[test]
fn a_nested_record_comes_back_from_c() {
    let m = compile();
    let outer_layout = layout(&m, "Outer");

    let mut inner = FakeObject::new(2);
    let mut outer = FakeObject::new(3);
    outer.set(1, Value::from_ptr(inner.ptr()));

    let mut buf = [0u8; 256];
    write_i64(&mut buf, OFF_HEAD, 7);
    write_i64(&mut buf, OFF_MID_LO, 1_757_000_000);
    write_i64(&mut buf, OFF_MID_HI, 675_893_106);
    write_i64(&mut buf, OFF_TAIL, 9);

    // SAFETY: as in the sibling test.
    unsafe {
        marshal_c_to_verum_struct_at(
            outer_layout,
            &m.ffi_layouts,
            buf.as_ptr(),
            buf.len(),
            outer.ptr(),
            0,
            0,
        );
    }

    assert_eq!(outer.get(0).as_i64(), 7);
    assert_eq!(outer.get(2).as_i64(), 9);
    assert_eq!(
        inner.get(0).as_i64(),
        1_757_000_000,
        "the nested record was not written back — `metadata()` reports the epoch"
    );
    assert_eq!(inner.get(1).as_i64(), 675_893_106);

    // The nested object must be the SAME object the caller passed in,
    // not a replacement: `&mut record` means the caller keeps their
    // object on both sides of the boundary.
    assert!(outer.get(1).is_ptr());
    assert_eq!(outer.get(1).as_ptr::<u8>(), inner.ptr());
}

/// A layout whose offsets run past the marshalling buffer must be
/// skipped, not written through. There was no bound check here at all
/// before T1359, and `field.offset` is a `u32` read from a module —
/// which `tests/red_team_bytecode_trust_boundary.rs` treats as
/// untrusted input.
#[test]
fn a_field_offset_past_the_buffer_is_refused() {
    let m = compile();
    let mut forged = layout(&m, "Outer").clone();
    forged.fields[0].offset = 4096;

    let mut inner = FakeObject::new(2);
    inner.set(0, Value::from_i64(1));
    inner.set(1, Value::from_i64(2));
    let mut outer = FakeObject::new(3);
    outer.set(0, Value::from_i64(0x0BAD));
    outer.set(1, Value::from_ptr(inner.ptr()));
    // EVERY slot, including the one this test does not care about. A
    // zeroed slot is not a NaN-boxed value — it reads back with tag
    // `None`, and `marshal_field_to_c` calls `as_i64()` on whatever the
    // slot holds, which debug-asserts. The marshaller assumes a
    // constructor-built record where every slot is initialised; that
    // assumption is pre-existing and true in production, and leaving it
    // implicit here cost one red run.
    outer.set(2, Value::from_i64(0x0F00));

    let mut buf = [0u8; 256];
    // SAFETY: the point of the test is that the out-of-range field is
    // skipped; if it is not, this is a heap write past `buf` and the
    // test is the thing that catches it.
    unsafe {
        marshal_verum_struct_to_c_at(
            &forged,
            &m.ffi_layouts,
            outer.ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            0,
            0,
        );
    }

    // The skip must be SURGICAL: the forged field alone is refused, and
    // its in-range neighbours still travel. A blanket bail-out would
    // also make both assertions below pass on the first one alone, which
    // is why the second is stated positively rather than as another zero.
    assert!(
        buf.iter().take(8).all(|b| *b == 0),
        "the forged field wrote at offset 0 — the bound was not applied"
    );
    assert_eq!(
        read_i64(&buf, OFF_MID_LO),
        1,
        "the nested field must still travel; refusing one field must not \
         abandon the rest of the struct"
    );
    assert_eq!(read_i64(&buf, OFF_MID_HI), 2);
    assert_eq!(read_i64(&buf, OFF_TAIL), 0x0F00);
}

/// T1361 — the walk is bounded by the RECEIVER, not only by the buffer.
///
/// `layout.fields.len()` is compiled into the module; the object comes
/// from the caller's register. A `.vbc` module is untrusted input, so a
/// layout claiming more fields than the receiver has slots must not read
/// past the object on the way out or WRITE past it coming back.
///
/// The overrun is caught by sentinel words after the data area, not by a
/// fault — this fixture cannot page-align the tail. A sentinel proves a
/// write happened; only a guard page would prove it left the allocation.
#[test]
fn a_layout_with_more_fields_than_the_receiver_has_slots_is_bounded() {
    let m = compile();
    let mut forged = layout(&m, "Outer").clone();
    // Three real fields become seven, the extra four naming slots the
    // receiver does not have.
    for i in 0..4u32 {
        forged.fields.push(FfiStructField {
            name: forged.fields[0].name,
            c_type: CType::I64,
            offset: 32 + i * 8,
            size: 8,
            align: 8,
            nested_layout: None,
        });
    }

    let mut inner = FakeObject::new(2);
    inner.set(0, Value::from_i64(1));
    inner.set(1, Value::from_i64(2));
    let mut outer = FakeObject::new(3);
    outer.set(0, Value::from_i64(7));
    outer.set(1, Value::from_ptr(inner.ptr()));
    outer.set(2, Value::from_i64(9));

    let mut buf = [0u8; 256];
    // SAFETY: the object is sized for three slots; the point of the test
    // is that the four forged fields do not reach past it.
    unsafe {
        marshal_verum_struct_to_c_at(
            &forged,
            &m.ffi_layouts,
            outer.ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            0,
            0,
        );
    }
    assert!(
        outer.guards_intact(),
        "the to-C leg read past the receiver's data area"
    );
    // The real fields still travelled.
    assert_eq!(read_i64(&buf, OFF_HEAD), 7);
    assert_eq!(read_i64(&buf, OFF_TAIL), 9);

    // And the write-back leg, where an unbounded walk is corruption
    // rather than a stray read.
    write_i64(&mut buf, 32, 0x1111_1111);
    write_i64(&mut buf, 40, 0x2222_2222);
    // SAFETY: as above.
    unsafe {
        marshal_c_to_verum_struct_at(
            &forged,
            &m.ffi_layouts,
            buf.as_ptr(),
            buf.len(),
            outer.ptr(),
            0,
            0,
        );
    }
    assert!(
        outer.guards_intact(),
        "the from-C leg WROTE past the receiver's data area — this is heap corruption"
    );
    assert_eq!(outer.get(0).as_i64(), 7, "in-range slots still travel");
}
