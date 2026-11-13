//! Call contracts belong to the lowering, not to an intrinsic's name or hints.
use verum_vbc::{
    instruction::ArithSubOpcode,
    intrinsics::{
        lookup_intrinsic,
        registry::{CodegenStrategy, IntrinsicCallBound},
    },
};

#[test]
fn checked_value_construction_is_certified_by_the_selected_operation() {
    for name in [
        "checked_add",
        "checked_sub",
        "checked_mul",
        "checked_div",
        "checked_add_u64",
        "checked_sub_u64",
        "checked_mul_u64",
    ] {
        let mut intrinsic = lookup_intrinsic(name).unwrap().intrinsic.clone();
        assert_eq!(
            intrinsic.call_bound(),
            IntrinsicCallBound::ValueConstruction,
            "{name}"
        );
        intrinsic.name = "unrelated_surface_name";
        assert_eq!(
            intrinsic.call_bound(),
            IntrinsicCallBound::ValueConstruction
        );
        // A neighboring arithmetic dispatch is not certified by this audit.
        intrinsic.strategy = CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyAdd);
        assert_eq!(intrinsic.call_bound(), IntrinsicCallBound::Unknown);
    }
}

#[test]
fn optimizer_purity_does_not_supply_a_missing_computational_contract() {
    let mut intrinsic = lookup_intrinsic("text_parse_int")
        .unwrap()
        .intrinsic
        .clone();
    intrinsic.name = "checked_add_u64";
    assert_eq!(intrinsic.call_bound(), IntrinsicCallBound::Unknown);
}

#[test]
fn stored_text_length_bounds_only_its_own_lowering() {
    let mut length = lookup_intrinsic("text_byte_len").unwrap().intrinsic.clone();
    assert_eq!(length.call_bound(), IntrinsicCallBound::ValueConstruction);
    length.strategy = lookup_intrinsic("text_as_bytes").unwrap().intrinsic.strategy.clone();
    assert_eq!(length.call_bound(), IntrinsicCallBound::Synchronous);
}
