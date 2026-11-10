//! Address bit 47 is legal on Linux AArch64. A managed handle with the
//! same payload must never escape into the header probe as an address.
#![cfg(target_pointer_width = "64")]
use verum_vbc::{
    interpreter::ObjectHeader,
    value::{FatRef, ThinRef, Value},
};

#[test]
fn high_addresses_and_handle_payloads_are_disjoint() {
    for handle in [
        Value::from_generator(0),
        Value::from_thin_ref(ThinRef::null()),
        Value::from_fat_ref(FatRef::null()),
    ] {
        let addr = (handle.bits() & verum_vbc::value::PAYLOAD_MASK) as usize;
        // No fabricated address is dereferenced: check only its encoding.
        let ptr = std::ptr::without_provenance_mut::<u8>(addr);
        let object = Value::from_ptr(ptr);
        assert_ne!(object, handle);
        assert!(object.is_regular_ptr() && object.is_tagged() && !object.is_nil());
        assert!(!object.is_cbgr_ref() && !object.is_int() && !object.is_generator());
        assert_eq!(Value::from_bits(object.bits()).as_ptr::<u8>(), ptr);
        assert!(!handle.is_regular_ptr());
        let absent = handle.as_ptr::<u8>();
        assert!(absent.is_null());
        // SAFETY: null is explicitly accepted by both probe APIs.
        assert!(unsafe { ObjectHeader::try_from_ptr(absent) }.is_none());
        let fallback = unsafe { ObjectHeader::ref_or_stub(absent) };
        assert_eq!((fallback.type_id.0, fallback.size), (0, 0));
    }
}

#[test]
fn every_address_bit_round_trips_without_aliasing_scalar_values() {
    for addr in [0, 1, (1 << 47) - 1, 1 << 47, (1 << 47) + 1, (1 << 48) - 1] {
        let pointer = std::ptr::without_provenance_mut::<u8>(addr);
        let value = Value::from_ptr(pointer);
        assert_eq!(value.is_nil(), addr == 0);
        assert_eq!(value.is_regular_ptr(), addr != 0);
        assert!(!value.is_float() && !value.is_boxed_i128());
        assert_eq!(value.as_ptr(), pointer);
        // A floating load with exactly those raw bits is a NaN, never a pointer.
        let nan = Value::from_f64(f64::from_bits(value.bits()));
        assert!(nan.is_nan_float() && !nan.is_ptr());
    }
}
