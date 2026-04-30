//! Locks the sort-aware variable extraction added so quantifier
//! elimination in `interpolation::quantifier_eliminate` can build
//! bound variables that match the formula's actual sorts.
//!
//! Pre-fix the QE path created `Bool::new_const(name)` for every
//! variable. Z3 distinguishes constants by name AND sort, so when the
//! formula contained `Int x` but the bound list had `Bool x`, the
//! existential bound nothing — QE then operated on a vacuous
//! quantifier and produced an unsound result.
//!
//! `collect_typed_variables_from_bool` walks the formula AST and
//! returns `(name, Sort)` pairs by reading each constant's actual
//! sort via `get_sort()` on the AST node.

use verum_common::Text;
use verum_smt::variable_extraction::collect_typed_variables_from_bool;
use z3::ast::{Bool, Int, Real};

#[test]
fn extracts_int_sort_for_int_variable() {
    let _ctx = z3::Context::thread_local();
    let x = Int::new_const("x");
    let zero = Int::from_i64(0);
    let formula = x.gt(&zero);

    let typed = collect_typed_variables_from_bool(&formula);
    let x_sort = typed
        .get(&Text::from("x"))
        .expect("x must be present in typed map");
    assert_eq!(
        format!("{}", x_sort),
        format!("{}", z3::Sort::int()),
        "Int variable must be tagged with Int sort"
    );
}

#[test]
fn extracts_bool_sort_for_bool_variable() {
    let _ctx = z3::Context::thread_local();
    let b = Bool::new_const("b");
    // formula = b
    let typed = collect_typed_variables_from_bool(&b);

    let b_sort = typed
        .get(&Text::from("b"))
        .expect("b must be present in typed map");
    assert_eq!(
        format!("{}", b_sort),
        format!("{}", z3::Sort::bool()),
        "Bool variable must be tagged with Bool sort"
    );
}

#[test]
fn extracts_real_sort_for_real_variable() {
    let _ctx = z3::Context::thread_local();
    let r = Real::new_const("r");
    let one = Real::from_rational(1, 1);
    let formula = r.gt(&one);

    let typed = collect_typed_variables_from_bool(&formula);

    let r_sort = typed
        .get(&Text::from("r"))
        .expect("r must be present in typed map");
    assert_eq!(
        format!("{}", r_sort),
        format!("{}", z3::Sort::real()),
        "Real variable must be tagged with Real sort"
    );
}

#[test]
fn mixed_sorts_in_one_formula_are_each_tagged_correctly() {
    // The architectural payoff: a formula mixing sorts must report
    // each variable with its own sort. Pre-fix the interpolation QE
    // path treated all of these as Bool, breaking soundness.
    let _ctx = z3::Context::thread_local();
    let x_int = Int::new_const("x_int");
    let y_bool = Bool::new_const("y_bool");
    let zero = Int::from_i64(0);
    // formula = (x_int > 0) AND y_bool
    let formula = Bool::and(&[&x_int.gt(&zero), &y_bool]);

    let typed = collect_typed_variables_from_bool(&formula);

    let x_sort = typed.get(&Text::from("x_int")).expect("x_int present");
    let y_sort = typed.get(&Text::from("y_bool")).expect("y_bool present");

    assert_eq!(format!("{}", x_sort), format!("{}", z3::Sort::int()));
    assert_eq!(format!("{}", y_sort), format!("{}", z3::Sort::bool()));
}

#[test]
fn no_variables_in_constant_formula() {
    let _ctx = z3::Context::thread_local();
    let formula = Bool::from_bool(true);
    let typed = collect_typed_variables_from_bool(&formula);
    assert_eq!(typed.len(), 0, "constant formula has no free variables");
}
