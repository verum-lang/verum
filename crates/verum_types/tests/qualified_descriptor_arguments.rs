//! T1505: a preserved module path must remain a nominal constraint when a
//! function signature is reconstructed from archive metadata.

use verum_ast::Span;
use verum_types::infer::parse_descriptor_type_string;
use verum_types::unify::Unifier;

fn compatible(declared: &str, actual: &str) -> bool {
    Unifier::new()
        .unify(
            &parse_descriptor_type_string(declared),
            &parse_descriptor_type_string(actual),
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
