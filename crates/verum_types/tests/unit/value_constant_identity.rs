use super::same_constant_identity as same;
use verum_common::{ConstValue as C, Heap, List, OrderedMap, OrderedSet};

#[test]
fn signed_zero_is_observably_distinct_without_changing_numeric_equality() {
    assert_eq!(C::Float(0.0), C::Float(-0.0));
    assert!(!same(&C::Float(0.0), &C::Float(-0.0)));
    assert!(same(&C::Float(-0.0), &C::Float(-0.0)));
}
#[test]
fn ieee_payload_identity_is_reflexive_and_exact() {
    for bits in [
        0u64,
        1,
        0x8000_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0x7ff8_0000_0000_0001,
        0x7ff0_0000_0000_0001,
    ] {
        assert!(same(
            &C::Float(f64::from_bits(bits)),
            &C::Float(f64::from_bits(bits))
        ));
    }
    assert!(!same(
        &C::Float(f64::from_bits(0x7ff8_0000_0000_0001)),
        &C::Float(f64::from_bits(0x7ff8_0000_0000_0002))
    ));
}
#[test]
fn sequence_identity_preserves_nested_values_shape_and_length() {
    let positive = C::Tuple(List::from_single(C::Array(List::from_single(C::Float(
        0.0,
    )))));
    let negative = C::Tuple(List::from_single(C::Array(List::from_single(C::Float(
        -0.0,
    )))));
    assert!(!same(&positive, &negative));
    assert!(same(&positive, &positive.clone()));
    assert!(!same(&C::Tuple(List::new()), &C::Array(List::new())));
    assert!(!same(
        &C::Tuple(List::new()),
        &C::Tuple(List::from_single(C::Unit))
    ));
}
#[test]
fn optional_identity_preserves_presence_and_nested_values() {
    assert!(same(&C::Maybe(None), &C::Maybe(None)));
    assert!(!same(&C::Maybe(None), &C::Maybe(Some(Heap::new(C::Unit)))));
    assert!(!same(
        &C::Maybe(Some(Heap::new(C::Float(0.0)))),
        &C::Maybe(Some(Heap::new(C::Float(-0.0))))
    ));
}
#[test]
fn ordered_map_identity_preserves_canonical_keys_and_values() {
    let mut a = OrderedMap::new();
    a.insert("a".into(), C::Float(0.0));
    a.insert("b".into(), C::Unit);
    let mut b = OrderedMap::new();
    b.insert("b".into(), C::Unit);
    b.insert("a".into(), C::Float(0.0));
    // OrderedMap uses canonical key order, not insertion order.
    assert!(same(&C::Map(a.clone()), &C::Map(b)));
    b = a.clone();
    b.insert("a".into(), C::Float(-0.0));
    assert!(!same(&C::Map(a.clone()), &C::Map(b)));
    assert!(same(&C::Map(a.clone()), &C::Map(a)));
}
#[test]
fn ordered_set_identity_uses_canonical_membership() {
    let mut a = OrderedSet::new();
    a.insert("a".into());
    a.insert("b".into());
    let mut b = OrderedSet::new();
    b.insert("b".into());
    b.insert("a".into());
    assert!(same(&C::Set(a.clone()), &C::Set(b.clone())));
    b.insert("c".into());
    assert!(!same(&C::Set(a.clone()), &C::Set(b)));
    assert!(same(&C::Set(a.clone()), &C::Set(a)));
}
#[test]
fn scalar_kinds_and_payloads_remain_distinct() {
    let values = List::from_iter([
        C::Unit,
        C::Bool(false),
        C::Int(1),
        C::UInt(1),
        C::Float(1.0),
        C::Char('x'),
        C::Text("x".into()),
        C::Bytes(b"x".to_vec()),
    ]);
    for (i, a) in values.iter().enumerate() {
        for (j, b) in values.iter().enumerate() {
            assert_eq!(same(a, b), i == j, "{a:?}/{b:?}");
        }
    }
}
#[test]
fn deeply_nested_constants_do_not_recurse() {
    let (mut a, mut b) = (C::Int(7), C::Int(7));
    for _ in 0..20_000 {
        a = C::Tuple(List::from_single(a));
        b = C::Tuple(List::from_single(b));
    }
    assert!(same(&a, &b));
    fn release(mut node: C) {
        while let C::Tuple(mut elements) = node {
            node = elements.pop().unwrap();
        }
    }
    release(a);
    release(b);
}
