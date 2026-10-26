use std::collections::BTreeMap;
use verum_common::List;
use verum_kernel::{Context, CoreTerm, FaceFormula, IntervalExpr, KernelError, UniverseLevel};

// The four-element De Morgan algebra: componentwise lattice operations
// on two bits, with ~(a,b) = (!b,!a). Both interior elements are fixed by
// involution. Boolean-only valuations would miss invalid complement laws.
fn reverse(value: u8) -> u8 {
    ((!value & 1) << 1) | ((!value & 2) >> 1)
}

fn coordinate(name: &str) -> usize {
    match name {
        "i" => 0,
        "j" => 1,
        _ => panic!("unexpected dimension {name}"),
    }
}

fn evaluate(expr: &IntervalExpr, point: [u8; 2]) -> u8 {
    expr.clauses().iter().fold(0, |sum, clause| {
        sum | clause.iter().fold(3, |product, literal| {
            let value = point[coordinate(literal.dimension.as_str())];
            product
                & if literal.reversed {
                    reverse(value)
                } else {
                    value
                }
        })
    })
}

fn signature(expr: &IntervalExpr) -> u32 {
    (0..16).fold(0, |signature, index| {
        signature | (u32::from(evaluate(expr, [index & 3, index >> 2])) << (2 * index))
    })
}

// Enumerate every antichain of subsets of the four formal generators
// i, ~i, j, ~j directly as bitsets. This oracle does not use the kernel's
// canonicalizer or its clause-implication implementation.
fn all_expressions() -> BTreeMap<u32, IntervalExpr> {
    let generators = [
        IntervalExpr::variable("i"),
        IntervalExpr::variable("i").reverse(),
        IntervalExpr::variable("j"),
        IntervalExpr::variable("j").reverse(),
    ];
    let mut expressions = BTreeMap::new();
    for selection in 0u32..(1 << 16) {
        let clauses: List<u8> = (0..16)
            .filter(|clause| selection & (1 << clause) != 0)
            .collect();
        if clauses
            .iter()
            .any(|a| clauses.iter().any(|b| a != b && a & b == *a))
        {
            continue;
        }
        let expr = clauses.iter().fold(IntervalExpr::zero(), |sum, clause| {
            sum.join(&generators.iter().enumerate().fold(
                IntervalExpr::one(),
                |product, (index, generator)| {
                    if clause & (1 << index) != 0 {
                        product.meet(generator)
                    } else {
                        product
                    }
                },
            ))
        });
        let expected = (0..16u8).fold(0u32, |mask, index| {
            let i = index & 3;
            let j = index >> 2;
            let values = [i, reverse(i), j, reverse(j)];
            let value = clauses.iter().fold(0, |sum, clause| {
                sum | values.iter().enumerate().fold(3, |product, (bit, value)| {
                    if clause & (1 << bit) != 0 {
                        product & value
                    } else {
                        product
                    }
                })
            });
            mask | (u32::from(value) << (2 * index))
        });
        assert_eq!(signature(&expr), expected);
        assert!(expressions.insert(expected, expr).is_none());
    }
    assert_eq!(expressions.len(), 168);
    expressions
}

#[test]
fn interval_is_de_morgan_but_not_boolean() {
    let i = IntervalExpr::variable("i");
    let j = IntervalExpr::variable("j");
    assert_ne!(i.meet(&i.reverse()), IntervalExpr::zero());
    assert_ne!(i.join(&i.reverse()), IntervalExpr::one());
    assert_eq!(i.meet(&i.reverse()).clauses()[0].len(), 2);
    assert_eq!(i.join(&j).reverse(), i.reverse().meet(&j.reverse()));
    assert_eq!(i.meet(&j).reverse(), i.reverse().join(&j.reverse()));
    assert_eq!(i.join(&j).meet(&i), i);
    assert_eq!(IntervalExpr::zero().reverse(), IntervalExpr::one());
    assert_eq!(IntervalExpr::one().reverse(), IntervalExpr::zero());
}

#[test]
fn all_two_dimension_expressions_obey_the_four_element_algebra() {
    let expressions = all_expressions();
    for (left_signature, left) in &expressions {
        assert_eq!(left.reverse().reverse(), *left);
        let expected_reverse = (0..16).fold(0u32, |mask, index| {
            mask | (u32::from(reverse(((left_signature >> (2 * index)) & 3) as u8)) << (2 * index))
        });
        assert_eq!(&left.reverse(), &expressions[&expected_reverse]);
        for (right_signature, right) in &expressions {
            assert_eq!(
                &left.meet(right),
                &expressions[&(left_signature & right_signature)]
            );
            assert_eq!(
                &left.join(right),
                &expressions[&(left_signature | right_signature)]
            );
        }
    }
}

#[test]
fn substitution_preserves_values_and_does_not_recurse_into_its_replacement() {
    let i = IntervalExpr::variable("i");
    let j = IntervalExpr::variable("j");
    let replacements = [
        IntervalExpr::zero(),
        IntervalExpr::one(),
        i.clone(),
        i.reverse(),
        j.clone(),
        i.meet(&j.reverse()),
        i.join(&i.reverse()),
    ];
    assert_eq!(i.substitute("i", &i.reverse()), i.reverse());
    assert_eq!(i.reverse().substitute("i", &j), j.reverse());
    for expression in all_expressions().values() {
        assert_eq!(expression.substitute("absent", &j), *expression);
        for replacement in &replacements {
            let substituted = expression.substitute("i", replacement);
            for index in 0..16 {
                let point = [index & 3, index >> 2];
                assert_eq!(
                    evaluate(&substituted, point),
                    evaluate(expression, [evaluate(replacement, point), point[1]])
                );
            }
        }
    }
}

fn chain_value(expression: &IntervalExpr, point: [u8; 2]) -> u8 {
    expression
        .clauses()
        .iter()
        .map(|clause| {
            clause
                .iter()
                .map(|literal| {
                    let value = point[coordinate(literal.dimension.as_str())];
                    if literal.reversed { 2 - value } else { value }
                })
                .min()
                .unwrap_or(2)
        })
        .max()
        .unwrap_or(0)
}

fn face_holds(face: &FaceFormula, point: [u8; 2]) -> bool {
    face.clauses().iter().any(|clause| {
        clause.literals().iter().all(|literal| {
            point[coordinate(literal.var.as_str())] == if literal.end { 2 } else { 0 }
        })
    })
}

#[test]
fn endpoint_projection_agrees_with_interior_and_endpoint_valuations() {
    for expression in all_expressions().values() {
        for endpoint in [false, true] {
            let face = expression.endpoint_face(endpoint);
            for i in 0..3 {
                for j in 0..3 {
                    assert_eq!(
                        face_holds(&face, [i, j]),
                        chain_value(expression, [i, j]) == if endpoint { 2 } else { 0 }
                    );
                }
            }
        }
    }
    let i = IntervalExpr::variable("i");
    assert!(i.meet(&i.reverse()).endpoint_face(true).is_bottom());
    assert!(!i.join(&i.reverse()).endpoint_face(true).is_top());
}

#[test]
fn dimension_scope_is_explicit_and_separate_from_term_bindings() {
    let empty = Context::new();
    let terms = empty.extend("i".into(), CoreTerm::Universe(UniverseLevel::Concrete(0)));
    let i = IntervalExpr::variable("i");
    assert!(
        matches!(terms.check_interval(&i), Err(KernelError::UnboundDimension(name)) if name.as_str() == "i")
    );
    let dimensions = terms.extend_dimension("i".into()).unwrap();
    assert!(dimensions.check_interval(&i).is_ok());
    assert!(dimensions.lookup("i").is_some());
    assert!(matches!(
        dimensions.extend_dimension("i".into()),
        Err(KernelError::DuplicateDimension(_))
    ));
    assert!(
        dimensions
            .check_interval(&i.join(&IntervalExpr::variable("j")))
            .is_err()
    );
    assert!(empty.check_interval(&IntervalExpr::zero()).is_ok());
    assert!(empty.check_interval(&IntervalExpr::one()).is_ok());
    assert!(empty.check_interval(&i).is_err());
    assert!(empty.lookup("i").is_none());
}

#[test]
fn face_substitution_commutes_with_interval_endpoint_projection() {
    let i = IntervalExpr::variable("i");
    let j = IntervalExpr::variable("j");
    let replacements = [
        IntervalExpr::zero(),
        IntervalExpr::one(),
        i.reverse(),
        j.clone(),
        i.meet(&j.reverse()),
        i.join(&i.reverse()),
    ];
    for expression in all_expressions().values() {
        for endpoint in [false, true] {
            let face = expression.endpoint_face(endpoint);
            for replacement in &replacements {
                assert_eq!(
                    face.substitute("i", replacement),
                    expression
                        .substitute("i", replacement)
                        .endpoint_face(endpoint)
                );
                for x in 0..3 {
                    for y in 0..3 {
                        let point = [x, y];
                        assert_eq!(
                            face_holds(&face.substitute("i", replacement), point),
                            face_holds(&face, [chain_value(replacement, point), y])
                        );
                    }
                }
            }
            assert_eq!(
                face.substitute("i", &IntervalExpr::zero()),
                face.restrict("i", false)
            );
            assert_eq!(
                face.substitute("i", &IntervalExpr::one()),
                face.restrict("i", true)
            );
        }
    }
}

#[test]
fn scoped_face_assumptions_accumulate_without_leaking_into_parent_contexts() {
    let context = Context::default()
        .extend_dimension("i".into())
        .unwrap()
        .extend_dimension("j".into())
        .unwrap();
    let i0 = FaceFormula::at_zero("i");
    let j1 = FaceFormula::at_one("j");
    let restricted = context.assume_face(&i0).unwrap().assume_face(&j1).unwrap();
    assert!(context.face_assumption().is_top());
    assert!(!context.entails_face(&i0).unwrap());
    assert!(restricted.entails_face(&i0.and(&j1)).unwrap());
    assert!(!restricted.entails_face(&FaceFormula::at_one("i")).unwrap());
    assert_eq!(restricted.face_assumption().dimensions().len(), 2);
    assert!(
        restricted
            .intervals_equal(&IntervalExpr::variable("i"), &IntervalExpr::zero())
            .unwrap()
    );
    assert!(
        restricted
            .intervals_equal(&IntervalExpr::variable("j"), &IntervalExpr::one())
            .unwrap()
    );
    assert!(
        context
            .assume_face(&FaceFormula::at_zero("missing"))
            .is_err()
    );
    assert!(
        context
            .entails_face(&FaceFormula::at_zero("missing"))
            .is_err()
    );
}

#[test]
fn interval_equality_checks_each_face_and_keeps_free_interiors() {
    let i = IntervalExpr::variable("i");
    let j = IntervalExpr::variable("j");
    let boundary = FaceFormula::at_zero("i").or(&FaceFormula::at_one("i"));
    let context = Context::new()
        .extend_dimension("i".into())
        .unwrap()
        .extend_dimension("j".into())
        .unwrap();
    assert!(
        !context
            .intervals_equal(&i.join(&i.reverse()), &IntervalExpr::one())
            .unwrap()
    );
    let restricted = context.assume_face(&boundary).unwrap();
    assert!(
        restricted
            .intervals_equal(&i.join(&i.reverse()), &IntervalExpr::one())
            .unwrap()
    );
    assert!(
        !restricted
            .intervals_equal(&i, &IntervalExpr::zero())
            .unwrap()
    );
    assert!(
        !restricted
            .intervals_equal(&j.join(&j.reverse()), &IntervalExpr::one())
            .unwrap()
    );
    let impossible = restricted
        .assume_face(&FaceFormula::at_zero("i").and(&FaceFormula::at_one("i")))
        .unwrap();
    assert!(impossible.intervals_equal(&i, &j).unwrap());
    assert!(
        impossible
            .intervals_equal(&IntervalExpr::variable("missing"), &j)
            .is_err()
    );
    assert!(
        impossible
            .assume_face(&FaceFormula::at_zero("missing"))
            .is_err()
    );
    assert!(
        impossible
            .entails_face(&FaceFormula::at_zero("missing"))
            .is_err()
    );
}

#[test]
fn equality_under_faces_matches_all_four_element_valuations() {
    let expressions = all_expressions();
    let faces = [
        FaceFormula::top(),
        FaceFormula::bottom(),
        FaceFormula::at_zero("i"),
        FaceFormula::at_one("j"),
        FaceFormula::at_zero("i").or(&FaceFormula::at_one("i")),
        FaceFormula::at_zero("i").or(&FaceFormula::at_one("j")),
        FaceFormula::at_zero("i").and(&FaceFormula::at_one("j")),
    ];
    for face in &faces {
        let mask = (0..16u8).fold(0u32, |mask, index| {
            let point = [index & 3, index >> 2];
            let satisfies = face.clauses().iter().any(|clause| {
                clause
                    .literals()
                    .iter()
                    .all(|lit| point[coordinate(lit.var.as_str())] == if lit.end { 3 } else { 0 })
            });
            mask | (if satisfies { 3 } else { 0 } << (2 * index))
        });
        for (left_values, left) in &expressions {
            for (right_values, right) in &expressions {
                assert_eq!(
                    left.equivalent_on(right, face),
                    left_values & mask == right_values & mask
                );
            }
        }
    }
}
