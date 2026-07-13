//! #54 IS-REGULAR-PTR-CONTRACT pin — the ONE structural guard that makes
//! every `is_ptr() → as_ptr() → header-probe` site safe against
//! NaN-box special-value markers (FatRef / ThinRef / …):
//! `ObjectHeader::try_from_ptr` MUST reject any address with the
//! special-value marker bit (bit 47) set, and `ref_or_stub` MUST return
//! the TypeId(0) stub for it, so every `header.type_id == X` dispatch
//! deterministically takes its else branch.
//!
//! History: char_extended's EncodeUtf8 wrote an `&mut [Byte]` scratch
//! through `is_ptr` — the FatRef marker passed the gate and the store
//! went to marker+offset (deterministic SIGSEGV in the text-mix interp
//! run, fixed 4eb1a4c6b). The probe layer was already safe because of
//! THIS invariant; the raw-else arm was not. If this pin ever fails,
//! ~200 `is_ptr` call sites become latently unsafe at once.

use verum_vbc::interpreter::ObjectHeader;

const SPECIAL_VALUE_MARKER: u64 = 1u64 << 47;
const FAT_REF_MARKER_PAYLOAD: u64 = 0xE000_0000_0000;

#[test]
fn try_from_ptr_rejects_marker_bit_addresses() {
    // The FatRef marker payload itself (8-aligned, non-null, unmapped).
    let marker = FAT_REF_MARKER_PAYLOAD as *const u8;
    assert!(
        (marker as u64) & SPECIAL_VALUE_MARKER != 0,
        "test premise: the FatRef marker payload carries bit 47"
    );
    // SAFETY: try_from_ptr's contract is exactly that this call is safe
    // for ANY bit pattern — it must classify, never dereference, a
    // marker-tagged address.
    let probed = unsafe { ObjectHeader::try_from_ptr(marker) };
    assert!(
        probed.is_none(),
        "try_from_ptr must reject special-value marker addresses"
    );

    // A marker-bit address with arbitrary low bits (ThinRef-style).
    let thin = (SPECIAL_VALUE_MARKER | 0x1234_5678) as *const u8;
    let probed = unsafe { ObjectHeader::try_from_ptr(thin) };
    assert!(probed.is_none(), "any bit-47 address must be rejected");
}

#[test]
fn ref_or_stub_returns_type_zero_stub_for_markers() {
    let marker = FAT_REF_MARKER_PAYLOAD as *const u8;
    // SAFETY: same classification-only contract as above.
    let header = unsafe { ObjectHeader::ref_or_stub(marker) };
    assert_eq!(
        header.type_id.0, 0,
        "marker addresses must yield the TypeId(0) stub so every \
         `header.type_id == X` dispatch falls through benignly"
    );
    assert_eq!(header.size, 0, "stub must carry size 0 (size-gated arms)");
}
