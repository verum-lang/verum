//! ARCH-P5 cross-tier drift pin — BYTE_SLICE (528) byte-view stamp.
//!

//! The Tier-1 `TextExtended::AsBytes` lowering must stamp its slice
//! Pack with `verum_vbc::types::TypeId::BYTE_SLICE` via
//! `lower_pack_typed` — the SAME TypeId the Tier-0 interpreter stamps
//! in `Heap::alloc_byte_slice` — so both tiers produce ONE
//! representation-tagged byte-view object form and every consumer can
//! dispatch on the header instead of the retired `len <= 1_000_000`
//! FatRef-as-Text heuristic.
//!

//! Two guards, in the same text-grep style as
//! `aot_lowering_coverage.rs` (no LLVM context setup needed):
//!
//!  1. The raw TypeId value is pinned through the IMPORTED constant
//!     (never a magic number) — drift in verum_vbc breaks this test
//!     here too, not just in verum_vbc's own `byte_slice_typeid_pinned`.
//!  2. The AsBytes lowering site must reference `TypeId::BYTE_SLICE`
//!     and route through `lower_pack_typed` — a regression back to
//!     plain `lower_pack` (generic TUPLE stamp) would silently revive
//!     the untyped-slice class this migration retired.

use std::fs;
use std::path::PathBuf;

use verum_vbc::types::TypeId;

fn codegen_src(file: &str) -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("src/llvm").join(file);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e))
}

#[test]
fn byte_slice_typeid_pinned_via_imported_constant() {
    // The cross-tier contract value.  Uses the imported verum_vbc
    // constant — the single source of truth both tiers stamp.
    assert_eq!(TypeId::BYTE_SLICE.0, 528, "BYTE_SLICE TypeId drifted");
    assert_ne!(
        TypeId::BYTE_SLICE.0,
        TypeId::TUPLE.0,
        "byte views must be distinguishable from generic TUPLE Packs"
    );
}

#[test]
fn as_bytes_lowering_stamps_byte_slice_via_lower_pack_typed() {
    let instruction_rs = codegen_src("instruction.rs");

    // Locate the AsBytes lowering arm (TextExtended sub-op 0x34).
    let as_bytes_idx = instruction_rs
        .find("// AsBytes — borrow a Text as a byte slice")
        .expect("AsBytes lowering arm not found in instruction.rs");
    // Scan a bounded window after the marker — the arm is short; a
    // window keeps the check anchored to THIS site rather than any
    // later lower_pack_typed caller.
    let window = &instruction_rs[as_bytes_idx..(as_bytes_idx + 4000).min(instruction_rs.len())];

    assert!(
        window.contains("lower_pack_typed"),
        "AsBytes lowering must stamp its Pack via lower_pack_typed (found plain lower_pack?)"
    );
    assert!(
        window.contains("TypeId::BYTE_SLICE"),
        "AsBytes lowering must use the imported TypeId::BYTE_SLICE constant, not a magic number"
    );

    // The generic-length runtime helper must accept the BYTE_SLICE
    // stamp exactly like the TUPLE slice shape.
    let runtime_rs = codegen_src("runtime.rs");
    let generic_len_idx = runtime_rs
        .find("fn emit_verum_generic_len")
        .expect("emit_verum_generic_len not found in runtime.rs");
    let len_window = &runtime_rs[generic_len_idx..(generic_len_idx + 6000).min(runtime_rs.len())];
    assert!(
        len_window.contains("TypeId::BYTE_SLICE"),
        "verum_generic_len must treat the BYTE_SLICE stamp as the Pack slice shape"
    );
}
