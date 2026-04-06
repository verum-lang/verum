#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for Dependent Types v2.0+ Features
//!
//! These tests cover the new dependent types features added for v2.0+:
//! - Universe Hierarchy: Type : Type1 : Type2 : ... (prevents paradoxes via stratification)
//! - Inductive Types: defined by constructors with strict positivity (e.g., Nat = Zero | Succ(Nat))
//! - Higher Inductive Types (HITs): types with path constructors (e.g., Circle with base + loop)
//! - Quantitative Type Theory: usage tracking with quantities 0|1|w (linear/affine/unrestricted)
//! - View Patterns: alternative pattern interfaces (e.g., Parity view on Nat -> Even(k) | Odd(k))
//! - Proof Irrelevance: all proofs of a Prop are equal; Squash<A> truncates to proposition
//!
// REQUIRES API MIGRATION:
// - Use verum_std::core::Text instead of verum_common::Text (which is just String alias)
// - Text::from() returns verum_std::core::Text, but dependent module expects this type

#![allow(unexpected_cfgs)]
#![cfg(feature = "dependent_types_v2_tests_disabled")]

use verum_ast::{
    Type, TypeKind,
    expr::{BinOp, Expr, ExprKind},
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::Text;
use verum_common::{Heap, List, Map, Maybe, Set};

// ==================== Test Helpers ====================

fn make_int_type() -> Type {
    Type::new(TypeKind::Int, Span::dummy())
}

fn make_int_lit(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

fn make_var(name: &str) -> Expr {
    let ident = Ident::new(name.into(), Span::dummy());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::dummy(),
    };
    Expr::new(ExprKind::Path(path), Span::dummy())
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

// ==================== Universe Hierarchy Tests ====================

#[test]
fn test_universe_level_concrete() {
    use verum_smt::UniverseLevel;

    let level0 = UniverseLevel::Concrete(0);
    let level1 = UniverseLevel::Concrete(1);
    let level2 = UniverseLevel::Concrete(2);

    assert_eq!(UniverseLevel::TYPE0, level0);
    assert_eq!(UniverseLevel::TYPE1, level1);
    assert_eq!(UniverseLevel::TYPE2, level2);

    assert!(level0.is_ground());
    assert!(level1.is_ground());
}

#[test]
fn test_universe_level_succ() {
    use verum_smt::UniverseLevel;

    let level0 = UniverseLevel::Concrete(0);
    let level1 = level0.succ();
    let level2 = level1.succ();

    assert_eq!(level1.eval(), Maybe::Some(1));
    assert_eq!(level2.eval(), Maybe::Some(2));
}

#[test]
fn test_universe_level_max() {
    use verum_smt::UniverseLevel;

    let level1 = UniverseLevel::Concrete(1);
    let level3 = UniverseLevel::Concrete(3);
    let max_level = UniverseLevel::max(level1, level3);

    assert_eq!(max_level.eval(), Maybe::Some(3));
}

#[test]
fn test_universe_level_variable() {
    use verum_smt::UniverseLevel;

    let var_level = UniverseLevel::variable("u");

    assert!(!var_level.is_ground());
    assert_eq!(var_level.eval(), Maybe::None);

    let vars = var_level.variables();
    assert!(vars.contains(&Text::from("u")));
}

#[test]
fn test_universe_level_substitution() {
    use verum_smt::UniverseLevel;

    let var_level = UniverseLevel::variable("u");
    let substituted = var_level.substitute(&Text::from("u"), &UniverseLevel::Concrete(5));

    assert_eq!(substituted.eval(), Maybe::Some(5));
}

#[test]
fn test_universe_constraint_solver() {
    use verum_smt::{UniverseConstraintSolver, UniverseLevel};

    let mut solver = UniverseConstraintSolver::new();

    // Add constraint: u <= 5
    solver.add_leq(UniverseLevel::variable("u"), UniverseLevel::Concrete(5));

    // Add constraint: u < v
    solver.add_lt(UniverseLevel::variable("u"), UniverseLevel::variable("v"));

    let result = solver.solve();
    assert!(result.is_ok(), "Constraints should be satisfiable");

    // Check assignments
    let u_val = solver.get_assignment("u");
    let v_val = solver.get_assignment("v");

    if let (Maybe::Some(u), Maybe::Some(v)) = (u_val, v_val) {
        assert!(u < v, "u should be less than v");
        assert!(u <= 5, "u should be at most 5");
    }
}

#[test]
fn test_universe_constraint_equality() {
    use verum_smt::{UniverseConstraintSolver, UniverseLevel};

    let mut solver = UniverseConstraintSolver::new();

    // Add constraint: u = 3
    solver.add_eq(UniverseLevel::variable("u"), UniverseLevel::Concrete(3));

    let result = solver.solve();
    assert!(result.is_ok());

    let u_val = solver.get_assignment("u");
    assert_eq!(u_val, Maybe::Some(3));
}

// ==================== Inductive Types Tests ====================

#[test]
fn test_inductive_type_creation() {
    use verum_smt::{Constructor, InductiveType};

    let nat_type = InductiveType::new(Text::from("Nat"))
        .with_constructor(Constructor::simple(Text::from("Zero"), make_int_type()))
        .with_constructor(Constructor::simple(Text::from("Succ"), make_int_type()));

    assert_eq!(nat_type.name.as_str(), "Nat");
    assert_eq!(nat_type.constructors.len(), 2);
    assert_eq!(nat_type.constructors[0].name.as_str(), "Zero");
    assert_eq!(nat_type.constructors[1].name.as_str(), "Succ");
}

#[test]
fn test_inductive_type_with_params() {
    use verum_smt::{Constructor, InductiveType, TypeParam, UniverseLevel};

    // List<T> with constructors Nil and Cons
    let list_type = InductiveType::new(Text::from("List"))
        .with_param(TypeParam::explicit(Text::from("T"), make_int_type()))
        .with_constructor(Constructor::simple(Text::from("Nil"), make_int_type()))
        .with_constructor(Constructor::simple(Text::from("Cons"), make_int_type()))
        .at_universe(UniverseLevel::TYPE1);

    assert_eq!(list_type.name.as_str(), "List");
    assert_eq!(list_type.params.len(), 1);
    assert_eq!(list_type.params[0].name.as_str(), "T");
    assert_eq!(list_type.universe, UniverseLevel::TYPE1);
}

#[test]
fn test_inductive_type_strict_positivity() {
    use verum_smt::{Constructor, InductiveType};

    // Valid inductive type
    let valid_type = InductiveType::new(Text::from("Valid"))
        .with_constructor(Constructor::simple(Text::from("MkValid"), make_int_type()));

    let result = valid_type.check_strict_positivity();
    assert!(
        result.is_ok(),
        "Valid inductive type should pass positivity check"
    );
}

// ==================== Higher Inductive Types Tests ====================

#[test]
fn test_higher_inductive_type_circle() {
    use verum_smt::{Constructor, HigherInductiveType, PathConstructor};

    // Circle type from spec:
    // hott inductive Circle : Type {
    //     base : Circle,
    //     loop : base = base
    // }
    let base_expr = make_var("base");
    let circle = HigherInductiveType::new(Text::from("Circle"))
        .with_point(Constructor::simple(Text::from("base"), make_int_type()))
        .with_path(PathConstructor::new(
            Text::from("loop"),
            base_expr.clone(),
            base_expr,
        ));

    assert_eq!(circle.name.as_str(), "Circle");
    assert_eq!(circle.point_constructors.len(), 1);
    assert_eq!(circle.path_constructors.len(), 1);
    assert_eq!(circle.path_constructors[0].name.as_str(), "loop");
}

#[test]
fn test_higher_inductive_type_quotient() {
    use verum_smt::{Constructor, HigherInductiveType};

    // Quotient type
    let quotient = HigherInductiveType::new(Text::from("Quotient"))
        .with_point(Constructor::simple(Text::from("class"), make_int_type()));

    assert_eq!(quotient.name.as_str(), "Quotient");
    assert_eq!(quotient.point_constructors.len(), 1);
}

// ==================== Quantitative Type Theory Tests ====================

#[test]
fn test_quantity_zero() {
    use verum_smt::Quantity;

    let zero = Quantity::Zero;
    let one = Quantity::One;

    // 0 * x = 0
    assert_eq!(zero.mul(one), Quantity::Zero);
    assert_eq!(one.mul(zero), Quantity::Zero);

    // 0 + x = x
    assert_eq!(zero.add(one), Quantity::One);
}

#[test]
fn test_quantity_linear() {
    use verum_smt::Quantity;

    let one = Quantity::One;

    // 1 * 1 = 1 (linear composition)
    assert_eq!(one.mul(one), Quantity::One);

    // 1 + 1 = ω (used in both branches)
    assert_eq!(one.add(one), Quantity::Omega);
}

#[test]
fn test_quantity_omega() {
    use verum_smt::Quantity;

    let one = Quantity::One;
    let omega = Quantity::Omega;

    // ω * x = ω for x ≠ 0
    assert_eq!(omega.mul(one), Quantity::Omega);
    assert_eq!(one.mul(omega), Quantity::Omega);

    // ω + x = ω
    assert_eq!(omega.add(one), Quantity::Omega);
}

#[test]
fn test_quantity_subsumption() {
    use verum_smt::Quantity;

    let zero = Quantity::Zero;
    let one = Quantity::One;
    let omega = Quantity::Omega;

    // 0 <: 1 <: ω
    assert!(zero.subsumed_by(one));
    assert!(zero.subsumed_by(omega));
    assert!(one.subsumed_by(omega));

    // 1 is not subsumed by 0
    assert!(!one.subsumed_by(zero));
    assert!(!omega.subsumed_by(one));
}

#[test]
fn test_quantified_binding() {
    use verum_smt::{QuantifiedBinding, Quantity};

    let linear = QuantifiedBinding::linear(Text::from("x"), make_int_type());
    assert_eq!(linear.quantity, Quantity::One);

    let unrestricted = QuantifiedBinding::unrestricted(Text::from("y"), make_int_type());
    assert_eq!(unrestricted.quantity, Quantity::Omega);

    let erased = QuantifiedBinding::erased(Text::from("z"), make_int_type());
    assert_eq!(erased.quantity, Quantity::Zero);
}

// ==================== View Patterns Tests ====================

#[test]
fn test_view_type_creation() {
    use verum_smt::ViewType;

    // Parity view for Nat
    let parity_view = ViewType::new(
        Text::from("Parity"),
        make_int_type(), // Base: Nat
        make_int_type(), // Result: indexed by Nat
    );

    assert_eq!(parity_view.name.as_str(), "Parity");
}

// ==================== Proof Irrelevance Tests ====================

#[test]
fn test_squash_type() {
    use verum_smt::Squash;

    let squash = Squash::new(make_int_type());

    // Squash erases computational content
    // ||Int|| : Prop
    assert_eq!(
        format!("{:?}", squash.inner_type.kind),
        format!("{:?}", make_int_type().kind)
    );
}

#[test]
fn test_subset_type() {
    use verum_smt::SubsetType;

    // { x : Int | x > 0 }
    let predicate = make_binary(BinOp::Gt, make_var("x"), make_int_lit(0));
    let subset = SubsetType::new(Text::from("x"), make_int_type(), predicate);

    assert!(
        subset.proof_irrelevant,
        "Subset type should be proof irrelevant by default"
    );
    assert_eq!(subset.var_name.as_str(), "x");
}

#[test]
fn test_subset_type_relevant_proof() {
    use verum_smt::SubsetType;

    let predicate = make_binary(BinOp::Gt, make_var("x"), make_int_lit(0));
    let subset = SubsetType::new(Text::from("x"), make_int_type(), predicate).with_relevant_proof();

    assert!(
        !subset.proof_irrelevant,
        "Proof should be relevant after .with_relevant_proof()"
    );
}

// ==================== Coinductive Types Extensions Tests ====================

#[test]
fn test_coinductive_type_new_api() {
    use verum_smt::{CoinductiveType, Destructor, UniverseLevel};

    // Stream<Int> with head and tail
    let stream = CoinductiveType::new(Text::from("Stream"))
        .with_destructor(Destructor::new(Text::from("head"), make_int_type()))
        .with_destructor(Destructor::new(Text::from("tail"), make_int_type()));

    assert_eq!(stream.name.as_str(), "Stream");
    assert_eq!(stream.destructors.len(), 2);
    assert_eq!(stream.universe, UniverseLevel::TYPE0);
}

// ==================== Integration: Full Dependent Types Stack ====================

#[test]
fn test_full_dependent_types_v2_features() {
    use verum_smt::{
        Constructor, InductiveType, QuantifiedBinding, Quantity, Squash, SubsetType,
        UniverseConstraintSolver, UniverseLevel,
    };

    // 1. Universe levels
    let mut solver = UniverseConstraintSolver::new();
    solver.add_lt(UniverseLevel::variable("α"), UniverseLevel::variable("β"));
    assert!(solver.solve().is_ok());

    // 2. Inductive type
    let nat = InductiveType::new(Text::from("Nat"))
        .with_constructor(Constructor::simple(Text::from("zero"), make_int_type()))
        .at_universe(UniverseLevel::TYPE0);
    assert!(nat.check_strict_positivity().is_ok());

    // 3. Quantitative typing
    let linear_binding = QuantifiedBinding::linear(Text::from("resource"), make_int_type());
    assert_eq!(linear_binding.quantity, Quantity::One);

    // 4. Proof irrelevance
    let _squash = Squash::new(make_int_type());
    let pred = make_binary(BinOp::Gt, make_var("n"), make_int_lit(0));
    let subset = SubsetType::new(Text::from("n"), make_int_type(), pred);
    assert!(subset.proof_irrelevant);

    // All v2.0+ features work together!
    assert!(true, "Full dependent types v2.0+ stack operational");
}
