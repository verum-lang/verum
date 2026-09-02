//! T1068 — the scalar name→id inverse must agree with the pinned
//! forward map, and must decline everything outside the scalar arm.
//!
//! The pair is the load-bearing part: codegen attaches
//! `implement P for Float` through the inverse, and if the two drift
//! the attach silently targets the wrong identity.

use verum_vbc::types::TypeId;

#[test]
fn every_forward_scalar_name_round_trips() {
    // Drive from the FORWARD authority so a new scalar added there
    // cannot quietly miss the inverse.
    let scalars = [
        TypeId::UNIT,
        TypeId::BOOL,
        TypeId::INT,
        TypeId::FLOAT,
        TypeId::TEXT,
        TypeId::NEVER,
        TypeId::U8,
        TypeId::U16,
        TypeId::U32,
        TypeId::U64,
        TypeId::I8,
        TypeId::I16,
        TypeId::I32,
        TypeId::F32,
        TypeId::CHAR,
        TypeId::I128,
        TypeId::U128,
    ];
    for id in scalars {
        let name = id
            .well_known_name()
            .unwrap_or_else(|| panic!("{id:?} must have a well-known name"));
        assert_eq!(
            TypeId::from_well_known_scalar_name(name),
            Some(id),
            "`{name}` must invert back to {id:?}"
        );
    }
}

#[test]
fn shared_id_spellings_resolve_by_name() {
    // These three are absent from the forward map because their ids
    // are shared; the inverse still has to answer for them.
    assert_eq!(TypeId::from_well_known_scalar_name("Byte"), Some(TypeId::BYTE));
    assert_eq!(TypeId::from_well_known_scalar_name("USize"), Some(TypeId::USIZE));
    assert_eq!(TypeId::from_well_known_scalar_name("ISize"), Some(TypeId::ISIZE));
}

#[test]
fn non_scalars_and_unknowns_are_declined() {
    // A probe that can only say "yes" measures nothing: these must
    // come back None, including the semantic band the doc-comment
    // deliberately excludes.
    for name in ["List", "Map", "Heap", "Shared", "Ordering", "Widget", ""] {
        assert_eq!(
            TypeId::from_well_known_scalar_name(name),
            None,
            "`{name}` is not a scalar and must be declined"
        );
    }
}
