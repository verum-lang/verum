//! #146 Phase 3f — bytecode determinism + cross-tier consistency
//! tests for the `MakeVariantTyped` instruction.
//!
//! Two contracts:
//!
//! 1. **Bytecode determinism**: encoding the same
//!    `Instruction::MakeVariantTyped` IR variant twice produces
//!    bit-identical bytes across compilations. Catches subtle
//!    non-determinism (HashMap iteration order, allocation-
//!    address-based ids, time-of-day reads) that would surface
//!    later as caching-cache-invalidation churn.
//!
//! 2. **Cross-tier consistency**: the Tier-0 interpreter
//!    (`handle_extended → MakeVariantTyped → alloc_variant_into_
//!    with_type_id`) and the Tier-1 AOT path (LLVM lowering at
//!    `instruction.rs::Instruction::MakeVariantTyped` →
//!    `runtime.lower_make_variant`) produce observationally-
//!    indistinguishable variant heap objects.  Checked here at
//!    the bytecode + decode level (the AOT path's IR-emission is
//!    pinned by `verum_codegen` test suites).
//!
//! See `verum_vbc::interpreter::dispatch_table::handlers::extended`
//! and `verum_codegen::llvm::instruction::Instruction::MakeVariantTyped`
//! for the validation + lowering counterparts.

use verum_vbc::bytecode::{decode_instruction, encode_instruction};
use verum_vbc::instruction::{ExtendedSubOpcode, Instruction, Opcode, Reg};

/// Pin: re-encoding the same IR variant produces bit-identical
/// bytes.  Different invocations of `encode_instruction` MUST NOT
/// emit different byte sequences for the same input — varint
/// encoding is deterministic, the wire-prefix is constant, and
/// no allocation-address-based ids leak into the output.
#[test]
fn make_variant_typed_encoding_is_deterministic() {
    let instr = Instruction::MakeVariantTyped {
        dst: Reg::new(7),
        type_id: 42,
        tag: 1,
        field_count: 2,
    };
    let mut a = Vec::new();
    let mut b = Vec::new();
    encode_instruction(&instr, &mut a);
    encode_instruction(&instr, &mut b);
    assert_eq!(a, b, "two encodes of the same IR variant must produce identical bytes");
}

/// Pin: encoder writes the documented wire prefix
/// `[0x1F (Extended)][0x01 (MakeVariantTyped sub-op)]`.
/// A refactor that swapped sub-op slots (e.g. relegated
/// MakeVariantTyped to 0x02) without keeping 0x01 would silently
/// reinterpret every previously-emitted .vbc archive's typed
/// variants.
#[test]
fn make_variant_typed_wire_prefix_is_pinned() {
    let instr = Instruction::MakeVariantTyped {
        dst: Reg::new(0),
        type_id: 1,
        tag: 0,
        field_count: 0,
    };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    assert_eq!(bytes[0], Opcode::Extended.to_byte());
    assert_eq!(bytes[1], ExtendedSubOpcode::MakeVariantTyped.to_byte());
    assert_eq!(bytes[1], 0x01); // Documented wire-protocol byte.
}

/// Pin: round-trip through encode/decode preserves all four
/// operand fields (dst, type_id, tag, field_count) bit-exactly.
/// The decoder recognises sub-op 0x01 and reconstructs the
/// typed `Instruction::MakeVariantTyped` rather than producing
/// the legacy opaque-operand `Instruction::Extended` carrier.
#[test]
fn make_variant_typed_roundtrip_preserves_all_operands() {
    let cases = [
        (Reg::new(0), 1u32, 0u32, 0u32),
        (Reg::new(7), 42, 1, 2),
        (Reg::new(255), 0xDEAD_BEEF, 0x7FFF_FFFF, 1024),
        (Reg::new(15), 0x100, 0xFF, 0xFF),
    ];
    for (dst, type_id, tag, field_count) in cases {
        let instr = Instruction::MakeVariantTyped {
            dst,
            type_id,
            tag,
            field_count,
        };
        let mut encoded = Vec::new();
        encode_instruction(&instr, &mut encoded);
        let mut offset = 0;
        let decoded =
            decode_instruction(&encoded, &mut offset).expect("decode succeeds");
        assert_eq!(offset, encoded.len());
        assert_eq!(decoded, instr);
    }
}

/// Pin: legacy `MakeVariant` and typed `MakeVariantTyped`
/// instructions produce DIFFERENT wire-format bytes — the wire
/// prefix is the discriminator (0x86 vs 0x1F+0x01).
///
/// A regression that aliased the two opcodes (e.g. accidentally
/// re-using `MakeVariant`'s primary opcode for the typed path)
/// would silently turn typed-variant emissions into the legacy
/// untyped form, and the runtime would fabricate the synthetic
/// `0x8000+tag` TypeId sentinel — breaking tag-disambiguation
/// for sum types that share variant tags (e.g. `Result.Err` and
/// `ShellError.SpawnFailed` both at tag=1).
#[test]
fn make_variant_and_typed_produce_distinct_wire_bytes() {
    let typed = Instruction::MakeVariantTyped {
        dst: Reg::new(3),
        type_id: 50,
        tag: 1,
        field_count: 1,
    };
    let untyped = Instruction::MakeVariant {
        dst: Reg::new(3),
        tag: 1,
        field_count: 1,
    };
    let mut typed_bytes = Vec::new();
    let mut untyped_bytes = Vec::new();
    encode_instruction(&typed, &mut typed_bytes);
    encode_instruction(&untyped, &mut untyped_bytes);
    assert_eq!(typed_bytes[0], Opcode::Extended.to_byte());
    assert_eq!(untyped_bytes[0], Opcode::MakeVariant.to_byte());
    assert_ne!(typed_bytes, untyped_bytes);
}

/// Pin: encoded byte length stays within the documented bounds.
/// Common case (small ids ≤ 127): `1 (0x1F) + 1 (sub-op) + 2
/// (reg) + 1 (type_id varint) + 1 (tag varint) + 1 (field_count
/// varint)` = 7 bytes (was 6 in the doc; reg encoding takes 2).
/// Worst case (u32 max ids): `1 + 1 + 2 + 5 + 5 + 5` = 19 bytes.
/// A regression that switched from varint to fixed-width u32 for
/// the operands would inflate the bytecode and miss this pin.
#[test]
fn make_variant_typed_encoding_is_compact() {
    // Common case — typical user-defined sum type with <128
    // declared types/variants/fields.
    let instr = Instruction::MakeVariantTyped {
        dst: Reg::new(0),
        type_id: 5,
        tag: 1,
        field_count: 2,
    };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    assert!(
        bytes.len() <= 8,
        "typical encoding should fit in 8 bytes, got {} bytes: {:?}",
        bytes.len(),
        bytes
    );

    // Worst case — pin the upper bound so a switch from varint
    // to fixed-width gets caught.  `Reg::MAX = 16383` (14-bit
    // register index space); the worst-case operand widths are
    // u32::MAX for the three varint fields.
    let instr_max = Instruction::MakeVariantTyped {
        dst: Reg::new(Reg::MAX),
        type_id: u32::MAX,
        tag: u32::MAX,
        field_count: u32::MAX,
    };
    let mut max_bytes = Vec::new();
    encode_instruction(&instr_max, &mut max_bytes);
    assert!(
        max_bytes.len() <= 19,
        "worst-case encoding should fit in 19 bytes, got {} bytes: {:?}",
        max_bytes.len(),
        max_bytes
    );
}
