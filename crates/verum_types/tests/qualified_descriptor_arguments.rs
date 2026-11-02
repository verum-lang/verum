//! T1505: a preserved module path must remain a nominal constraint when a
//! function signature is reconstructed from archive metadata.

use verum_ast::Span;
use verum_types::infer::parse_descriptor_type_string;
use verum_types::unify::Unifier;

fn compatible(declared: &str, actual: &str) -> bool {
    Unifier::new()
        .unify(
            &parse_descriptor_type_string(declared).unwrap(),
            &parse_descriptor_type_string(actual).unwrap(),
            Span::dummy(),
        )
        .is_ok()
}

#[test]
fn qualified_nominal_argument_accepts_its_imported_bare_name() {
    assert!(compatible("Carrier<domain.atomic.Flag>", "Carrier<Flag>"));
}

#[test]
fn unrelated_argument_does_not_satisfy_the_qualified_signature() {
    assert!(!compatible("Carrier<domain.atomic.Flag>", "Carrier<Token>"));
    assert!(!compatible(
        "Carrier<domain.atomic.Flag>",
        "Carrier<domain>"
    ));
}

#[test]
fn two_qualified_nominals_with_the_same_leaf_remain_distinct() {
    assert!(!compatible(
        "Carrier<domain.atomic.Flag>",
        "Carrier<other.atomic.Flag>",
    ));
}

fn named(parts: &[&str], arguments: &[verum_types::Type]) -> verum_types::Type {
    use verum_ast::ty::{Ident, Path, PathSegment};
    verum_types::Type::Named {
        path: Path::new(
            parts
                .iter()
                .map(|name| PathSegment::Name(Ident::new(*name, Span::dummy())))
                .collect(),
            Span::dummy(),
        ),
        args: arguments.iter().cloned().collect(),
    }
}

#[test]
fn archived_and_source_nominals_agree_on_the_entire_path_and_arguments() {
    use verum_types::{Type, subtype::Subtyping};

    let archived = parse_descriptor_type_string("domain.atomic.Flag<Int>").unwrap();
    let source = named(&["domain", "atomic", "Flag"], &[Type::Int]);
    let partly_qualified = named(&["domain.atomic", "Flag"], &[Type::Int]);
    let foreign = named(&["other", "atomic", "Flag"], &[Type::Int]);
    let different_argument = named(&["domain", "atomic", "Flag"], &[Type::Bool]);
    let missing_argument = named(&["domain", "atomic", "Flag"], &[]);

    for (other, expected) in [
        (&source, true),
        (&partly_qualified, true),
        (&foreign, false),
        (&different_argument, false),
        (&missing_argument, false),
    ] {
        for (left, right) in [(&archived, other), (other, &archived)] {
            assert_eq!(
                Unifier::new().unify(left, right, Span::dummy()).is_ok(),
                expected
            );
            assert_eq!(Subtyping::new().is_subtype(left, right), expected);
        }
    }
}

#[test]
fn a_packed_nested_owner_does_not_become_a_bare_leaf_inside_a_qualified_path() {
    let left = named(&["domain", "nested.Flag"], &[]);
    let right = named(&["domain", "Flag"], &[]);
    for (a, b) in [(&left, &right), (&right, &left)] {
        assert!(Unifier::new().unify(a, b, Span::dummy()).is_err());
        assert!(!verum_types::subtype::Subtyping::new().is_subtype(a, b));
    }
}

#[test]
fn archive_array_extents_are_preserved_at_their_own_nesting_depth() {
    use verum_types::Type;
    let array = |element, size| Type::array(element, Some(size));
    let slice = |element| Type::Slice { element: Box::new(element) };
    for (text, expected) in [
        ("[Int; 0]", array(Type::Int, 0)),
        ("[[Int; 2]; 3]", array(array(Type::Int, 2), 3)),
        ("[[Int; 2]]", slice(array(Type::Int, 2))),
        ("[[Int]; 4]", array(slice(Type::Int), 4)),
    ] {
        assert_eq!(parse_descriptor_type_string(text).unwrap(), expected, "{text}");
    }
}

#[test]
fn invalid_descriptor_extents_do_not_produce_types() {
    for invalid in ["[Int; -1]", "[Int; ]", "&[[Int; 9223372036854775808]; 2]", "[Int; N]"] {
        assert!(parse_descriptor_type_string(invalid).is_err(), "{invalid}");
    }
}
