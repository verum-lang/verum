//! Pillar-3 increment 1 (ARRAY-ITER-CONCRETIZE-1) — the metadata
//! CARRY of impl-block generic names on method descriptors.
//!
//! Locks two contracts:
//!
//! 1. **Wire compat**: `FunctionDescriptor::impl_generic_names` is
//!    `#[serde(default)]` — descriptors serialised BEFORE the field
//!    existed still deserialize (empty carry → the scheme-birth site
//!    keeps the conservative `impl_var_count = 0` behaviour).
//!
//! 2. **Round-trip**: a carried `["T"]` survives
//!    serialize → deserialize unchanged, in declaration order.
//!
//! The end-to-end semantics (receiver args binding the impl-level
//! var so slice-iterator closures see concrete element types) are
//! covered by `repro/closx.vr` + `repro/collx.vr` and the meta/
//! conformance suite; this file pins the FORMAT so a field rename /
//! default removal shows up as a unit failure instead of a bake-time
//! decode error.

use verum_common::{List, Maybe, Text};
use verum_types::core_metadata::FunctionDescriptor;

fn carried_descriptor() -> FunctionDescriptor {
    FunctionDescriptor {
        name: Text::from("Slice.iter"),
        module_path: Text::from("core.collections.slice"),
        generic_params: List::new(),
        params: List::new(),
        return_type: Text::from("SliceIter<__generic_0>"),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: Maybe::None,
        parent_type: Maybe::Some(Text::from("Slice")),
        impl_generic_names: List::from_iter([Text::from("T")]),
        is_const: false,
        decl_span: Maybe::None,
    }
}

#[test]
fn impl_generic_names_round_trips() {
    let fd = carried_descriptor();
    let json = serde_json::to_string(&fd).expect("serialize");
    let back: FunctionDescriptor = serde_json::from_str(&json).expect("deserialize");
    let names: Vec<&str> = back
        .impl_generic_names
        .iter()
        .map(|t| t.as_str())
        .collect();
    assert_eq!(names, vec!["T"]);
    assert_eq!(back.return_type.as_str(), "SliceIter<__generic_0>");
}

#[test]
fn missing_impl_generic_names_defaults_empty() {
    // Serialise, then strip the field to simulate a pre-carry
    // descriptor produced by an older writer.
    let fd = carried_descriptor();
    let mut value = serde_json::to_value(&fd).expect("to_value");
    let obj = value.as_object_mut().expect("object");
    obj.remove("impl_generic_names")
        .expect("field present in new writer output");
    let back: FunctionDescriptor =
        serde_json::from_value(value).expect("old-shape descriptor must still decode");
    assert!(
        back.impl_generic_names.is_empty(),
        "absent carry must default to empty (conservative impl_var_count=0)"
    );
}

#[test]
fn multi_param_carry_preserves_declaration_order() {
    // `implement<K, V> Map<K, V>` — order is load-bearing: the
    // scheme-birth site zips `__generic_i` placeholders against
    // this list positionally.
    let mut fd = carried_descriptor();
    fd.name = Text::from("Map.get");
    fd.parent_type = Maybe::Some(Text::from("Map"));
    fd.impl_generic_names = List::from_iter([Text::from("K"), Text::from("V")]);
    let json = serde_json::to_string(&fd).expect("serialize");
    let back: FunctionDescriptor = serde_json::from_str(&json).expect("deserialize");
    let names: Vec<&str> = back
        .impl_generic_names
        .iter()
        .map(|t| t.as_str())
        .collect();
    assert_eq!(names, vec!["K", "V"]);
}
