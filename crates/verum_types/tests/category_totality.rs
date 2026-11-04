use verum_types::category_algebra::{Category, Morphism, Object};

#[test]
fn object_without_identity_is_not_a_category() {
    let mut category = Category::new();
    category.add_object(Object::new("A"));
    assert!(
        category.check_laws().is_err(),
        "a nonempty object set requires an identity for every object"
    );
}

#[test]
fn missing_composable_pair_is_not_a_category() {
    let mut category = Category::new();
    let object = Object::new("A");
    category.add_morphism(Morphism::new("id", object.clone(), object.clone()));
    category.add_morphism(Morphism::new("f", object.clone(), object.clone()));
    category.set_identity(object, "id").unwrap();
    category.set_composition("id", "id", "id").unwrap();
    category.set_composition("id", "f", "f").unwrap();
    category.set_composition("f", "id", "f").unwrap();
    assert!(
        category.check_laws().is_err(),
        "f after f has no declared result, despite every unit equation being supplied"
    );
}

#[test]
fn complete_one_object_category_is_valid() {
    let mut category = Category::new();
    let object = Object::new("A");
    category.add_morphism(Morphism::new("id", object.clone(), object.clone()));
    category.add_morphism(Morphism::new("f", object.clone(), object.clone()));
    category.set_identity(object, "id").unwrap();
    category.set_composition("id", "id", "id").unwrap();
    category.set_composition("id", "f", "f").unwrap();
    category.set_composition("f", "id", "f").unwrap();
    category.set_composition("f", "f", "f").unwrap();
    assert!(category.check_laws().is_ok());
}

use verum_types::category_algebra::CategoryError;

fn two_object_discrete() -> Category {
    let mut category = Category::new();
    for name in ["A", "B"] {
        let object = Object::new(name);
        category.add_morphism(Morphism::new(name, object.clone(), object.clone()));
        category.set_identity(object, name).unwrap();
        category.set_composition(name, name, name).unwrap();
    }
    category
}

#[test]
fn disconnected_objects_do_not_require_noncomposable_entries() {
    assert_eq!(two_object_discrete().check_laws(), Ok(()));
}

#[test]
fn replacing_an_identity_cannot_reuse_its_old_endpoint_check() {
    let mut category = two_object_discrete();
    category.add_morphism(Morphism::new("A", Object::new("B"), Object::new("B")));
    assert_eq!(
        category.check_laws(),
        Err(CategoryError::IdentityShape { name: "A".into() })
    );
}

#[test]
fn replacing_an_arrow_cannot_reuse_old_composition_checks() {
    let mut category = two_object_discrete();
    category.add_morphism(Morphism::new("f", Object::new("A"), Object::new("A")));
    for (g, f, result) in [("A", "f", "f"), ("f", "A", "f"), ("f", "f", "f")] {
        category.set_composition(g, f, result).unwrap();
    }
    assert_eq!(category.check_laws(), Ok(()));
    category.add_morphism(Morphism::new("f", Object::new("B"), Object::new("A")));
    assert!(matches!(
        category.check_laws(),
        Err(CategoryError::CompositionTypeMismatch { .. })
    ));
}

#[test]
fn a_new_isolated_object_invalidates_previous_success() {
    let mut category = two_object_discrete();
    assert_eq!(category.check_laws(), Ok(()));
    category.add_object(Object::new("C"));
    assert_eq!(
        category.check_laws(),
        Err(CategoryError::MissingIdentity {
            object: Object::new("C")
        })
    );
}

#[test]
fn right_unit_failure_is_reported_on_a_total_table() {
    let mut category = Category::new();
    let object = Object::new("A");
    for name in ["id", "f"] {
        category.add_morphism(Morphism::new(name, object.clone(), object.clone()));
    }
    category.set_identity(object, "id").unwrap();
    for (g, f, result) in [
        ("id", "id", "id"),
        ("id", "f", "f"),
        ("f", "id", "id"),
        ("f", "f", "f"),
    ] {
        category.set_composition(g, f, result).unwrap();
    }
    assert_eq!(
        category.check_laws(),
        Err(CategoryError::RightIdentityFailure {
            morphism: "f".into()
        })
    );
}

#[test]
fn all_three_arrow_unital_tables_match_exhaustive_associativity() {
    let names = ["e", "a", "b"];
    let mut accepted = 0;
    let mut rejected = 0;
    // All 3^4 operations with a fixed two-sided identity. The reference
    // calculation works directly on integer tables, independently of the
    // production endpoint index and named composition lookup.
    for encoding in 0..81 {
        let mut digits = encoding;
        let mut table = [[0usize; 3]; 3];
        for i in 0..3 {
            table[0][i] = i;
            table[i][0] = i;
        }
        for row in table.iter_mut().skip(1) {
            for result in row.iter_mut().skip(1) {
                *result = digits % 3;
                digits /= 3;
            }
        }
        let associative = (0..3).all(|h| {
            (0..3).all(|g| (0..3).all(|f| table[h][table[g][f]] == table[table[h][g]][f]))
        });
        let mut category = Category::new();
        let object = Object::new("O");
        for name in names {
            category.add_morphism(Morphism::new(name, object.clone(), object.clone()));
        }
        category.set_identity(object, "e").unwrap();
        for g in 0..3 {
            for f in 0..3 {
                category
                    .set_composition(names[g], names[f], names[table[g][f]])
                    .unwrap();
            }
        }
        let result = category.check_laws();
        assert_eq!(
            result.is_ok(),
            associative,
            "operation {encoding}: {result:?}"
        );
        if associative {
            accepted += 1;
        } else {
            rejected += 1;
            assert!(matches!(
                result,
                Err(CategoryError::AssociativityFailure { .. })
            ));
        }
    }
    assert!(accepted > 0 && rejected > 0);
}
