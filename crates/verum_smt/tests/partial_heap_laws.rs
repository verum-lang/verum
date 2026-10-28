//! The shared spatial algebra works independently of scalar representation.
use verum_smt::partial_heap::PartialHeap;
use z3::{
    SatResult, Solver, Sort,
    ast::{Array, BV, Bool, Dynamic, Int},
};

fn prove(property: Bool) {
    let solver = Solver::new();
    solver.assert(property.not());
    assert_eq!(solver.check(), SatResult::Unsat);
}

#[test]
fn read_after_write_overwrite_and_frame_laws_hold_for_both_frontends() {
    for word in [false, true] {
        let sort = if word {
            Sort::bitvector(64)
        } else {
            Sort::int()
        };
        let constant = |n: i64| -> Dynamic {
            if word {
                BV::from_i64(n, 64).into()
            } else {
                Int::from_i64(n).into()
            }
        };
        let heap = PartialHeap::new(
            Array::fresh_const("values", &sort, &sort),
            Array::fresh_const("owned", &sort, &Sort::bool()),
            constant(0),
        )
        .unwrap();
        let first = heap.store(&constant(8), &constant(42)).unwrap();
        let second = first.store(&constant(8), &constant(9)).unwrap();
        prove(first.load(&constant(8)).unwrap().eq(constant(42)));
        prove(second.load(&constant(8)).unwrap().eq(constant(9)));
        prove(
            second
                .load(&constant(16))
                .unwrap()
                .eq(heap.load(&constant(16)).unwrap()),
        );
        prove(second.owned().eq(heap.owned()));
        let singleton = PartialHeap::new(
            heap.values().clone(),
            Array::const_array(&sort, &Bool::from_bool(false))
                .store(&constant(8), &Bool::from_bool(true)),
            constant(0),
        )
        .unwrap();
        // Opaque spatial predicates must ignore a write outside their domain.
        let other = singleton.store(&constant(16), &constant(123)).unwrap();
        prove(
            singleton
                .relation("predicate", &[])
                .eq(other.relation("predicate", &[])),
        );
    }
}

#[test]
fn wrong_sorts_are_errors_instead_of_solver_panics() {
    let heap = PartialHeap::new(
        Array::fresh_const("values", &Sort::int(), &Sort::int()),
        Array::fresh_const("owned", &Sort::int(), &Sort::bool()),
        Int::from_i64(0).into(),
    )
    .unwrap();
    let wrong: Dynamic = Bool::from_bool(true).into();
    let address: Dynamic = Int::from_i64(8).into();
    assert!(heap.load(&wrong).is_err());
    assert!(heap.store(&address, &wrong).is_err());
    assert!(heap.points_to(&address, &wrong).is_err());
    assert!(PartialHeap::new(heap.values().clone(), heap.values().clone(), address).is_err());
}
