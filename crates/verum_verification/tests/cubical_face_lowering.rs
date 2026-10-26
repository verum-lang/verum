use std::collections::BTreeSet;
use verum_common::Text;
use verum_kernel::FaceFormula as CanonicalFace;
use verum_verification::cubical::FaceFormula;

fn scope() -> BTreeSet<Text> {
    [Text::from("i"), Text::from("j")].into_iter().collect()
}

#[test]
fn parsed_faces_lower_to_the_shared_canonical_algebra() {
    for source in [
        "i = 0 ∨ (i = 0 ∧ j = 1)",
        "i = 0 ∨ i = 0",
        "(i = 0 ∨ j = 1) ∧ i = 0",
    ] {
        let face = FaceFormula::parse(source).unwrap();
        assert_eq!(
            face.lower_scoped(&scope()).unwrap(),
            CanonicalFace::at_zero("i")
        );
    }
}

#[test]
fn undeclared_dimensions_are_rejected_before_simplification() {
    for source in [
        "missing = 0",
        "0 ∧ missing = 0",
        "1 ∨ missing = 0",
        "missing = 0 ∧ missing = 1",
    ] {
        let error = FaceFormula::parse(source)
            .unwrap()
            .lower_scoped(&scope())
            .unwrap_err();
        assert!(
            error
                .as_str()
                .contains("undeclared interval dimension: missing")
        );
    }
}

#[test]
fn declared_endpoints_remain_distinct_from_an_exhaustive_boolean_split() {
    let boundary = FaceFormula::parse("i = 0 ∨ i = 1")
        .unwrap()
        .lower_scoped(&scope())
        .unwrap();
    assert!(!boundary.is_top());
    assert!(boundary.restrict("i", false).is_top());
    assert!(boundary.restrict("i", true).is_top());
    assert!(
        FaceFormula::parse("i = 0 ∧ i = 1")
            .unwrap()
            .lower_scoped(&scope())
            .unwrap()
            .is_bottom()
    );
}
