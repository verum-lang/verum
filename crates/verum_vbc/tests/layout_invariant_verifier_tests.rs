//! Tests for the type-layout invariant verifier (#146).
//!
//! Each test constructs a synthetic `TypeDescriptor` with a deliberate
//! shape violation, pushes it into a fresh `VbcCodegen`'s type table,
//! and asserts that `verify_type_layout_invariants` rejects it with
//! a focused error message.  Together they pin the invariants that
//! `finalize_module` enforces before bytecode is emitted.

use smallvec::smallvec;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::types::{
    FieldDescriptor, StringId, TypeDescriptor, TypeId, TypeKind, TypeRef,
    VariantDescriptor, VariantKind, Visibility,
};

fn make_type_with_variants(name_idx: u32, variants: Vec<VariantDescriptor>) -> TypeDescriptor {
    TypeDescriptor {
        id: TypeId(1000),
        name: StringId(name_idx),
        kind: TypeKind::Sum,
        type_params: smallvec![],
        fields: smallvec![],
        variants: variants.into(),
        size: 0,
        alignment: 1,
        drop_fn: None,
        clone_fn: None,
        protocols: smallvec![],
        visibility: Visibility::Public,
    }
}

fn unit_variant(tag: u32, name_idx: u32) -> VariantDescriptor {
    VariantDescriptor {
        name: StringId(name_idx),
        tag,
        payload: None,
        kind: VariantKind::Unit,
        arity: 0,
        fields: smallvec![],
    }
}

fn tuple_variant(tag: u32, name_idx: u32, arity: u8) -> VariantDescriptor {
    VariantDescriptor {
        name: StringId(name_idx),
        tag,
        payload: None,
        kind: VariantKind::Tuple,
        arity,
        fields: smallvec![],
    }
}

fn record_variant(tag: u32, name_idx: u32, fields: Vec<FieldDescriptor>) -> VariantDescriptor {
    VariantDescriptor {
        name: StringId(name_idx),
        tag,
        payload: None,
        kind: VariantKind::Record,
        arity: 0,
        fields: fields.into(),
    }
}

fn make_field(name_idx: u32) -> FieldDescriptor {
    FieldDescriptor {
        name: StringId(name_idx),
        type_ref: TypeRef::concrete(TypeId::INT),
        offset: 0,
        visibility: Visibility::Public,
    }
}

/// Prime a `VbcCodegen`'s string table with the given names and return
/// the per-name string indices for use in variant descriptors.
fn make_codegen_with_strings(names: &[&str]) -> (VbcCodegen, Vec<u32>) {
    let mut codegen = VbcCodegen::new();
    let idxs: Vec<u32> = names.iter().map(|n| codegen.intern_string_for_test(n)).collect();
    (codegen, idxs)
}

#[test]
fn accepts_well_formed_unit_only_sum() {
    // type Color is Red | Green | Blue;
    let (mut codegen, idxs) = make_codegen_with_strings(&["Color", "Red", "Green", "Blue"]);
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![
        unit_variant(0, idxs[1]),
        unit_variant(1, idxs[2]),
        unit_variant(2, idxs[3]),
    ]));
    codegen.verify_type_layout_invariants().expect("well-formed sum should pass");
}

#[test]
fn accepts_mixed_unit_tuple_record() {
    // type Shape is Point | Pair(Int, Int) | Box { w: Int, h: Int };
    let (mut codegen, idxs) =
        make_codegen_with_strings(&["Shape", "Point", "Pair", "Box", "w", "h"]);
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![
        unit_variant(0, idxs[1]),
        tuple_variant(1, idxs[2], 2),
        record_variant(2, idxs[3], vec![make_field(idxs[4]), make_field(idxs[5])]),
    ]));
    codegen.verify_type_layout_invariants().expect("mixed sum should pass");
}

#[test]
fn rejects_unit_with_payload() {
    // Synthetic: a Unit variant that wrongly carries an arity.
    let (mut codegen, idxs) = make_codegen_with_strings(&["Bad", "X"]);
    let mut bad = unit_variant(0, idxs[1]);
    bad.arity = 2;
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![bad]));
    let err = codegen.verify_type_layout_invariants().unwrap_err();
    let s = format!("{}", err);
    assert!(s.contains("Bad.X"), "error should name the offending variant: {}", s);
    assert!(s.contains("Unit"), "error should mention the variant kind: {}", s);
}

#[test]
fn rejects_tuple_with_zero_arity() {
    // Synthetic: a Tuple variant with arity=0 — should be Unit.
    let (mut codegen, idxs) = make_codegen_with_strings(&["Bad", "X"]);
    let bad = tuple_variant(0, idxs[1], 0); // arity=0 + Tuple kind = inconsistent
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![bad]));
    let err = codegen.verify_type_layout_invariants().unwrap_err();
    let s = format!("{}", err);
    assert!(
        s.contains("Tuple") && s.contains("arity=0"),
        "error should mention zero-arity tuple: {}",
        s
    );
}

#[test]
fn rejects_record_without_fields() {
    let (mut codegen, idxs) = make_codegen_with_strings(&["Bad", "X"]);
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![
        record_variant(0, idxs[1], vec![]),
    ]));
    let err = codegen.verify_type_layout_invariants().unwrap_err();
    let s = format!("{}", err);
    assert!(
        s.contains("Record") && s.contains("no fields"),
        "error should call out the empty record: {}",
        s
    );
}

#[test]
fn rejects_duplicate_tags() {
    let (mut codegen, idxs) = make_codegen_with_strings(&["Color", "Red", "Green"]);
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![
        unit_variant(0, idxs[1]),
        unit_variant(0, idxs[2]), // duplicate tag
    ]));
    let err = codegen.verify_type_layout_invariants().unwrap_err();
    let s = format!("{}", err);
    assert!(s.contains("duplicate variant tag"), "error should call out the duplicate: {}", s);
}

#[test]
fn rejects_out_of_range_tag() {
    // Two variants but one of them has tag=5 — beyond `0..2`.
    let (mut codegen, idxs) = make_codegen_with_strings(&["Color", "Red", "Green"]);
    codegen.push_type_for_test(make_type_with_variants(idxs[0], vec![
        unit_variant(0, idxs[1]),
        unit_variant(5, idxs[2]),
    ]));
    let err = codegen.verify_type_layout_invariants().unwrap_err();
    let s = format!("{}", err);
    assert!(s.contains("outside the dense range"), "error should explain the gap: {}", s);
}
