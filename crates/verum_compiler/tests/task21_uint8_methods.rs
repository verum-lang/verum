//! Task #21 — drift-pin: `metadata.types["UInt8"].methods` must
//! contain the canonical inherent-method names declared in
//! `core/base/primitives.vr`'s `implement UInt8 { ... }` block.

use verum_types::core_metadata::CoreMetadata;

fn load_metadata() -> CoreMetadata {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/stdlib_runtime.core_metadata"));
    bincode::deserialize::<CoreMetadata>(bytes).expect("deserialise embedded metadata")
}

#[test]
fn uint8_inherent_methods_populated() {
    let m = load_metadata();
    let td = m
        .types
        .get(&verum_common::Text::from("UInt8"))
        .expect("metadata.types[UInt8] must exist after Pass 2.5 synthesis");
    let methods: Vec<String> = td.methods.iter().map(|t| t.as_str().to_string()).collect();
    eprintln!("UInt8 has {} methods: {:?}", methods.len(), methods);

    for expected in &[
        "wrapping_add",
        "wrapping_sub",
        "wrapping_mul",
        "checked_add",
        "saturating_add",
        "to_int",
    ] {
        assert!(
            methods.contains(&expected.to_string()),
            "UInt8 should have `{expected}` after Task #21 \
             recover_primitive_parent_from_name fix — methods were {:?}",
            methods
        );
    }
}

#[test]
fn uint8_wrapping_add_in_metadata_functions() {
    let m = load_metadata();
    let key = verum_common::Text::from("UInt8.wrapping_add");
    let fd = m
        .functions
        .get(&key)
        .expect("metadata.functions[UInt8.wrapping_add] must exist after Task #21 fix");
    eprintln!(
        "UInt8.wrapping_add: params={:?}, return={}",
        fd.params.iter().map(|p| p.ty.as_str()).collect::<Vec<_>>(),
        fd.return_type.as_str()
    );
    assert_eq!(fd.return_type.as_str(), "UInt8");
}
