use verum_common::List;
use verum_kernel::{Clause, FaceFormula, FaceLit};

#[test]
fn disjunction_is_structurally_idempotent() {
    let p = FaceFormula::at_zero("i");
    assert_eq!(p.or(&p), p);
}

#[test]
fn disjunction_order_does_not_change_the_normal_form() {
    let p = FaceFormula::at_zero("i");
    let q = FaceFormula::at_one("j");
    assert_eq!(p.or(&q), q.or(&p));
}

#[test]
fn conjunction_removes_absorbed_products() {
    let p = FaceFormula::at_zero("i");
    let q = FaceFormula::at_one("j");
    let r = FaceFormula::at_zero("k");
    assert_eq!(p.or(&q).and(&p.or(&r)), p.or(&q.and(&r)));
}

#[test]
fn clause_implication_runs_from_stronger_to_weaker_constraints() {
    let p = Clause::singleton(FaceLit::zero("i"));
    let q = Clause::singleton(FaceLit::one("j"));
    let both = p.and(&q).unwrap();
    assert!(both.implies(&p));
    assert!(!p.implies(&both));
    assert!(p.implies(&Clause::empty()));
    assert!(!Clause::empty().implies(&p));
}

#[test]
fn repeated_joins_do_not_accumulate_clauses() {
    let original = FaceFormula::at_zero("i").or(&FaceFormula::at_one("j"));
    let mut face = original.clone();
    for _ in 0..256 {
        face = face.or(&original).and(&original);
        assert_eq!(face.clauses().len(), 2);
    }
    assert_eq!(face, original);
}

#[test]
fn interval_endpoints_do_not_exhaust_the_interval() {
    let boundary = FaceFormula::at_zero("i").or(&FaceFormula::at_one("i"));
    assert!(!boundary.is_top());
    assert!(!FaceFormula::top().implies(&boundary));
    assert!(boundary.restrict("i", false).is_top());
    assert!(boundary.restrict("i", true).is_top());
    assert!(FaceFormula::at_zero("i").restrict("i", true).is_bottom());
}

fn holds(clause: &Clause, point: [Option<bool>; 2]) -> bool {
    clause.literals().iter().all(|lit| {
        let coordinate = match lit.var.as_str() {
            "i" => 0,
            "j" => 1,
            _ => panic!("unexpected dimension"),
        };
        point[coordinate] == Some(lit.end)
    })
}

fn points() -> impl Iterator<Item = [Option<bool>; 2]> {
    [None, Some(false), Some(true)].into_iter().flat_map(|i| {
        [None, Some(false), Some(true)]
            .into_iter()
            .map(move |j| [i, j])
    })
}

fn truth(face: &FaceFormula) -> u16 {
    points().enumerate().fold(0, |mask, (index, point)| {
        mask | (u16::from(face.clauses().iter().any(|clause| holds(clause, point))) << index)
    })
}

#[test]
fn exhaustive_two_dimension_semantics_including_interior_points() {
    // Every consistent conjunction on two dimensions, then every subset of
    // those conjunctions. The oracle evaluates assignments directly; it does
    // not use the implication or normalization implementation.
    let clauses: List<Clause> = points()
        .map(|point| {
            let mut clause = Clause::empty();
            for (variable, endpoint) in ["i", "j"].into_iter().zip(point) {
                if let Some(end) = endpoint {
                    clause = clause
                        .and(&Clause::singleton(FaceLit {
                            var: variable.into(),
                            end,
                        }))
                        .unwrap();
                }
            }
            clause
        })
        .collect();
    let mut distinct: List<(FaceFormula, u16)> = List::new();
    for selection in 0usize..(1 << clauses.len()) {
        let chosen: List<Clause> = clauses
            .iter()
            .enumerate()
            .filter(|(index, _)| selection & (1 << index) != 0)
            .map(|(_, clause)| clause.clone())
            .collect();
        let expected = points().enumerate().fold(0u16, |mask, (index, point)| {
            mask | (u16::from(chosen.iter().any(|clause| holds(clause, point))) << index)
        });
        let canonical = FaceFormula::from_clauses(chosen);
        assert_eq!(truth(&canonical), expected);
        if let Some((previous, _)) = distinct.iter().find(|(_, mask)| *mask == expected) {
            assert_eq!(&canonical, previous);
        } else {
            distinct.push((canonical, expected));
        }
    }
    for (left, left_truth) in &distinct {
        for (right, right_truth) in &distinct {
            assert_eq!(truth(&left.and(right)), left_truth & right_truth);
            assert_eq!(truth(&left.or(right)), left_truth | right_truth);
            assert_eq!(left.implies(right), left_truth & !right_truth == 0);
            assert_eq!(left.equivalent_to(right), left_truth == right_truth);
        }
        for (coordinate, variable) in ["i", "j"].into_iter().enumerate() {
            for endpoint in [false, true] {
                let expected = points().enumerate().fold(0, |mask, (index, mut point)| {
                    point[coordinate] = Some(endpoint);
                    mask | (u16::from(left.clauses().iter().any(|clause| holds(clause, point)))
                        << index)
                });
                assert_eq!(truth(&left.restrict(variable, endpoint)), expected);
            }
        }
    }
}
