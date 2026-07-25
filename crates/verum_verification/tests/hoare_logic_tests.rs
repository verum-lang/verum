//! Comprehensive tests for Hoare Logic verification
//!
//! Tests for weakest precondition calculus, verification condition generation,
//! and SMT integration.

use verum_verification::hoare_logic::{Command, HoareTriple, WPCalculator};
use verum_verification::{Formula, SmtExpr, VCVariable as Variable, VarType, hoare_generate_vc};
// Use verum_common types to match Command enum
use verum_common::{Heap, Maybe};

// ==================== Basic WP Tests ====================

#[test]
fn test_wp_skip() {
    // wp(skip, Q) = Q
    let calc = WPCalculator::new();
    let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));

    let wp = calc.wp(&Command::Skip, &post).unwrap();

    // WP of skip is the postcondition itself
    assert!(matches!(wp, Formula::Gt(_, _)));
}

#[test]
fn test_wp_assignment() {
    // wp(x := e, Q) = Q[x := e]
    // wp(x := y + 1, x > 0) = y + 1 > 0
    let calc = WPCalculator::new();

    let cmd = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::add(SmtExpr::var("y"), SmtExpr::int(1)),
    };
    let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));

    let wp = calc.wp(&cmd, &post).unwrap();

    // After substitution, x should be replaced by y + 1
    // So x > 0 becomes y + 1 > 0
    match &wp {
        Formula::Gt(_, right) => {
            // right should be 0
            assert!(matches!(right.as_ref(), SmtExpr::IntConst(0)));
        }
        _ => panic!("Expected Gt formula, got {:?}", wp),
    }
}

#[test]
fn test_wp_conditional() {
    // wp(if b then c1 else c2, Q) = (b => wp(c1, Q)) /\ (!b => wp(c2, Q))
    let calc = WPCalculator::new();

    let cmd = Command::If {
        condition: Formula::gt(SmtExpr::var("x"), SmtExpr::int(0)),
        then_branch: Heap::new(Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::int(1),
        }),
        else_branch: Maybe::Some(Heap::new(Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::int(0),
        })),
    };

    let post = Formula::ge(SmtExpr::var("y"), SmtExpr::int(0));

    let wp = calc.wp(&cmd, &post).unwrap();

    // Both branches should result in y >= 0
    assert!(matches!(wp, Formula::And(_)));
}

// ==================== Hoare Triple Tests ====================

#[test]
fn test_valid_hoare_triple() {
    // {x >= 0} x := x + 1 {x > 0} is valid
    let pre = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
    let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let cmd = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
    };

    let triple = HoareTriple::new(pre, cmd, post);
    let vc = hoare_generate_vc(&triple).unwrap();

    // VC should be: x >= 0 => x + 1 > 0, which is valid
    assert!(matches!(vc.formula, Formula::Implies(_, _)));
}

#[test]
fn test_invalid_hoare_triple() {
    // {x >= 0} x := x - 1 {x > 0} is NOT valid (counterexample: x = 0)
    let pre = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
    let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let cmd = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::sub(SmtExpr::var("x"), SmtExpr::int(1)),
    };

    let triple = HoareTriple::new(pre, cmd, post);
    let vc = hoare_generate_vc(&triple).unwrap();

    // VC should be: x >= 0 => x - 1 > 0, which is NOT valid
    // x = 0 is a counterexample
    assert!(matches!(vc.formula, Formula::Implies(_, _)));
}

// ==================== Loop Verification Tests ====================

#[test]
fn test_while_loop_invariant() {
    // while x > 0 inv x >= 0 { x := x - 1 }
    // Postcondition: x = 0
    let calc = WPCalculator::new();

    let invariant = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
    let condition = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));

    let body = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::sub(SmtExpr::var("x"), SmtExpr::int(1)),
    };

    let cmd = Command::While {
        condition,
        invariant,
        body: Heap::new(body),
        decreases: Maybe::Some(SmtExpr::var("x")),
        lexicographic_decreases: Maybe::None,
    };

    let post = Formula::eq(SmtExpr::var("x"), SmtExpr::int(0));

    let wp = calc.wp(&cmd, &post).unwrap();

    // WP should include invariant preservation and termination conditions
    assert!(matches!(wp, Formula::And(_)));
}

// ==================== Variable Substitution Tests ====================

#[test]
fn test_substitution_in_nested_expression() {
    let calc = WPCalculator::new();

    // x := y * 2; postcondition: x + 1 > y
    let cmd = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::mul(SmtExpr::var("y"), SmtExpr::int(2)),
    };

    let post = Formula::gt(
        SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
        SmtExpr::var("y"),
    );

    let wp = calc.wp(&cmd, &post).unwrap();

    // After substitution: y * 2 + 1 > y
    match &wp {
        Formula::Gt(_, right) => {
            // right should be y
            assert!(matches!(right.as_ref(), SmtExpr::Var(_)));
        }
        _ => panic!("Expected Gt formula"),
    }
}

// ==================== Edge Cases ====================

#[test]
fn test_assert_command() {
    let calc = WPCalculator::new();

    // assert x > 0
    let assertion = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let cmd = Command::Assert(assertion.clone());

    let post = Formula::True;
    let wp = calc.wp(&cmd, &post).unwrap();

    // WP of assert should be: P ∧ Q = (x > 0) ∧ true = x > 0
    match &wp {
        Formula::And(_) => {}
        Formula::Gt(_, _) => {} // Or it might just be the assertion itself
        _ => panic!("Unexpected WP formula: {:?}", wp),
    }
}

#[test]
fn test_assume_command() {
    let calc = WPCalculator::new();

    // assume x > 0
    let assumption = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let cmd = Command::Assume(assumption.clone());

    let post = Formula::gt(SmtExpr::var("x"), SmtExpr::int(-1));
    let wp = calc.wp(&cmd, &post).unwrap();

    // WP of assume should be: assumption => postcondition
    assert!(matches!(wp, Formula::Implies(_, _)));
}

// ==================== Formula Construction Tests ====================

#[test]
fn test_formula_and() {
    let f1 = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let f2 = Formula::lt(SmtExpr::var("x"), SmtExpr::int(10));

    let and = Formula::and(vec![f1, f2]);

    match and {
        Formula::And(conjuncts) => {
            assert_eq!(conjuncts.len(), 2);
        }
        _ => panic!("Expected And formula"),
    }
}

#[test]
fn test_formula_or() {
    let f1 = Formula::eq(SmtExpr::var("x"), SmtExpr::int(0));
    let f2 = Formula::eq(SmtExpr::var("x"), SmtExpr::int(1));

    let or = Formula::or(vec![f1, f2]);

    match or {
        Formula::Or(disjuncts) => {
            assert_eq!(disjuncts.len(), 2);
        }
        _ => panic!("Expected Or formula"),
    }
}

#[test]
fn test_formula_implies() {
    let antecedent = Formula::ge(SmtExpr::var("x"), SmtExpr::int(0));
    let consequent = Formula::ge(SmtExpr::var("x"), SmtExpr::int(-1));

    let impl_formula = Formula::implies(antecedent, consequent);

    assert!(matches!(impl_formula, Formula::Implies(_, _)));
}

// ==================== SmtExpr Tests ====================

#[test]
fn test_smt_expr_arithmetic() {
    let x = SmtExpr::var("x");
    let one = SmtExpr::int(1);

    let add = SmtExpr::add(x.clone(), one.clone());
    let sub = SmtExpr::sub(x.clone(), one.clone());
    let mul = SmtExpr::mul(x.clone(), one.clone());

    // These should not panic
    assert!(!matches!(add, SmtExpr::IntConst(_)));
    assert!(!matches!(sub, SmtExpr::IntConst(_)));
    assert!(!matches!(mul, SmtExpr::IntConst(_)));
}

#[test]
fn test_variable_creation() {
    let v1 = Variable::new("x");
    assert_eq!(v1.name.as_str(), "x");

    let v2 = Variable::typed("y", VarType::Int);
    assert_eq!(v2.name.as_str(), "y");
    assert!(matches!(v2.ty, Maybe::Some(VarType::Int)));

    let v3 = Variable::versioned("z", 42);
    assert_eq!(v3.name.as_str(), "z");
    assert!(matches!(v3.version, Maybe::Some(42)));
}

// ==================== Sequence Tests ====================

#[test]
fn test_sequence_wp() {
    let calc = WPCalculator::new();

    // x := 1; y := x + 1 with postcondition y > 1
    let c1 = Command::Assign {
        var: Variable::new("x"),
        expr: SmtExpr::int(1),
    };
    let c2 = Command::Assign {
        var: Variable::new("y"),
        expr: SmtExpr::add(SmtExpr::var("x"), SmtExpr::int(1)),
    };

    let cmd = Command::Seq {
        first: Heap::new(c1),
        second: Heap::new(c2),
    };

    let post = Formula::gt(SmtExpr::var("y"), SmtExpr::int(1));

    let wp = calc.wp(&cmd, &post).unwrap();

    // wp(x := 1; y := x + 1, y > 1) should be valid
    assert!(!matches!(wp, Formula::False));
}

// ==================== Triple Verification Tests ====================

#[test]
fn test_simple_triple_vc_generation() {
    // {true} x := 1 {x > 0}
    let triple = HoareTriple::new(
        Formula::True,
        Command::Assign {
            var: Variable::new("x"),
            expr: SmtExpr::int(1),
        },
        Formula::gt(SmtExpr::var("x"), SmtExpr::int(0)),
    );

    let vc = hoare_generate_vc(&triple).unwrap();

    // VC should be: true => 1 > 0, which simplifies to 1 > 0
    // This should be valid
    assert!(!matches!(vc.formula, Formula::False));
}

#[test]
fn test_triple_with_precondition() {
    // {x > 0} y := x {y > 0}
    let triple = HoareTriple::new(
        Formula::gt(SmtExpr::var("x"), SmtExpr::int(0)),
        Command::Assign {
            var: Variable::new("y"),
            expr: SmtExpr::var("x"),
        },
        Formula::gt(SmtExpr::var("y"), SmtExpr::int(0)),
    );

    let vc = hoare_generate_vc(&triple).unwrap();

    // VC should be: x > 0 => x > 0, which is valid (after substitution y := x)
    assert!(!matches!(vc.formula, Formula::False));
}

// ==================== Havoc Tests ====================

#[test]
fn test_havoc_command() {
    let calc = WPCalculator::new();

    // havoc x
    let cmd = Command::Havoc(Variable::new("x"));
    let post = Formula::True;

    let wp = calc.wp(&cmd, &post).unwrap();

    // wp(havoc x, true) = forall x. true = true
    assert!(!matches!(wp, Formula::False));
}

// ==================== Bitvector Tests ====================

#[test]
fn test_bitvector_constant_creation() {
    // Test that bitvector constants can be created
    let bv32 = SmtExpr::BitVecConst(42, 32);
    let bv64 = SmtExpr::BitVecConst(0xDEADBEEF, 64);

    // Should create valid SmtExpr variants
    assert!(matches!(bv32, SmtExpr::BitVecConst(42, 32)));
    assert!(matches!(bv64, SmtExpr::BitVecConst(0xDEADBEEF, 64)));
}

#[test]
fn test_bitvector_variable_type() {
    // Test typed bitvector variable creation
    let v = Variable::typed("bv_x", VarType::BitVec(32));
    assert_eq!(v.name.as_str(), "bv_x");
    match v.ty {
        Maybe::Some(VarType::BitVec(w)) => assert_eq!(w, 32),
        _ => panic!("Expected BitVec(32) type"),
    }
}

#[test]
fn test_bitvector_smtlib_sort() {
    // Test SMT-LIB sort generation for bitvectors
    let bv_type = VarType::BitVec(64);
    assert_eq!(bv_type.smtlib_sort().as_str(), "(_ BitVec 64)");
}

// ==================== Array Operation Tests ====================

#[test]
fn test_array_select_expression() {
    // Test array select: arr[idx]
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::int(5);
    let select = SmtExpr::Select(Box::new(arr), Box::new(idx));

    // Should create a valid Select variant
    assert!(matches!(select, SmtExpr::Select(_, _)));
}

#[test]
fn test_array_store_expression() {
    // Test array store: arr[idx := val]
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::int(3);
    let val = SmtExpr::int(100);
    let store = SmtExpr::Store(Box::new(arr), Box::new(idx), Box::new(val));

    // Should create a valid Store variant
    assert!(matches!(store, SmtExpr::Store(_, _, _)));
}

#[test]
fn test_array_type_creation() {
    // Test array type for variables
    let arr_type = VarType::Array(Box::new(VarType::Int), Box::new(VarType::Int));
    assert_eq!(arr_type.smtlib_sort().as_str(), "(Array Int Int)");
}

#[test]
fn test_nested_array_select() {
    // Test nested array access: arr[i][j]
    let arr = SmtExpr::var("matrix");
    let i = SmtExpr::int(0);
    let j = SmtExpr::int(1);

    // matrix[0] then [1]
    let row = SmtExpr::Select(Box::new(arr), Box::new(i));
    let element = SmtExpr::Select(Box::new(row), Box::new(j));

    // Should create nested Select
    match element {
        SmtExpr::Select(inner, _) => {
            assert!(matches!(*inner, SmtExpr::Select(_, _)));
        }
        _ => panic!("Expected nested Select"),
    }
}

#[test]
fn test_select_as_binop() {
    // Test Select as binary operation for tuple/array access
    use verum_verification::SmtBinOp;

    let tuple = SmtExpr::var("tuple");
    let idx = SmtExpr::int(0);
    let select = SmtExpr::BinOp(SmtBinOp::Select, Box::new(tuple), Box::new(idx));

    // Should create a BinOp with Select operation
    assert!(matches!(select, SmtExpr::BinOp(SmtBinOp::Select, _, _)));
}

// ==================== Complex Pattern Tests ====================

#[test]
fn test_formula_with_array_access() {
    // Test formula involving array access: arr[0] > 0
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::int(0);
    let access = SmtExpr::Select(Box::new(arr), Box::new(idx));
    let zero = SmtExpr::int(0);

    let formula = Formula::gt(access, zero);

    // Should create a valid Gt formula
    assert!(matches!(formula, Formula::Gt(_, _)));
}

#[test]
fn test_wp_with_array_assignment() {
    let calc = WPCalculator::new();

    // Model array assignment as: arr_0 := arr[0]
    // This is a simplified model of array element assignment
    let cmd = Command::Assign {
        var: Variable::new("arr_0"),
        expr: SmtExpr::int(42),
    };

    let post = Formula::eq(SmtExpr::var("arr_0"), SmtExpr::int(42));

    let wp = calc.wp(&cmd, &post).unwrap();

    // wp(arr_0 := 42, arr_0 == 42) should be 42 == 42, which is true
    assert!(!matches!(wp, Formula::False));
}

// ==================== Variable Substitution with Complex Types ====================

#[test]
fn test_substitution_preserves_bitvector() {
    // Test that substitution works with bitvector constants
    let bv = SmtExpr::BitVecConst(255, 8);
    let var = Variable::new("x");
    let replacement = SmtExpr::int(1);

    // Substituting in a bitvector constant should leave it unchanged
    let result = bv.substitute(&var, &replacement);
    assert!(matches!(result, SmtExpr::BitVecConst(255, 8)));
}

#[test]
fn test_substitution_in_select() {
    // Test substitution inside array select
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::var("i");
    let select = SmtExpr::Select(Box::new(arr), Box::new(idx));

    let var_i = Variable::new("i");
    let replacement = SmtExpr::int(5);

    let result = select.substitute(&var_i, &replacement);

    // Index should be substituted
    match result {
        SmtExpr::Select(_, idx_box) => {
            assert!(matches!(*idx_box, SmtExpr::IntConst(5)));
        }
        _ => panic!("Expected Select after substitution"),
    }
}

#[test]
fn test_substitution_in_store() {
    // Test substitution inside array store
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::var("i");
    let val = SmtExpr::var("v");
    let store = SmtExpr::Store(Box::new(arr), Box::new(idx), Box::new(val));

    let var_v = Variable::new("v");
    let replacement = SmtExpr::int(100);

    let result = store.substitute(&var_v, &replacement);

    // Value should be substituted
    match result {
        SmtExpr::Store(_, _, val_box) => {
            assert!(matches!(*val_box, SmtExpr::IntConst(100)));
        }
        _ => panic!("Expected Store after substitution"),
    }
}

// ==================== For Loop Tests ====================

#[test]
fn test_for_loop_command() {
    let calc = WPCalculator::new();

    // for i in 0..10 { body }
    let cmd = Command::For {
        var: Variable::new("i"),
        start: SmtExpr::int(0),
        end: SmtExpr::int(10),
        invariant: Formula::True,
        body: Heap::new(Command::Skip),
    };

    let post = Formula::True;

    let wp = calc.wp(&cmd, &post).unwrap();

    // Should compute a valid WP for for loop
    assert!(!matches!(wp, Formula::False));
}

#[test]
fn test_for_loop_with_invariant() {
    let calc = WPCalculator::new();

    // for i in 0..n inv i >= 0 { body }
    let cmd = Command::For {
        var: Variable::new("i"),
        start: SmtExpr::int(0),
        end: SmtExpr::var("n"),
        invariant: Formula::ge(SmtExpr::var("i"), SmtExpr::int(0)),
        body: Heap::new(Command::Skip),
    };

    let post = Formula::True;

    let wp = calc.wp(&cmd, &post).unwrap();

    // Should include invariant in WP
    assert!(!matches!(wp, Formula::False));
}

// ==================== SMT-LIB Output Tests ====================

#[test]
fn test_bitvector_smtlib_output() {
    // Test SMT-LIB representation of bitvector constant
    let bv = SmtExpr::BitVecConst(42, 32);
    let smtlib = bv.to_smtlib();

    // Should produce (_ bv42 32) format
    assert_eq!(smtlib.as_str(), "(_ bv42 32)");
}

#[test]
fn test_array_smtlib_type_output() {
    // Test SMT-LIB sort output for array types
    let arr_type = VarType::Array(Box::new(VarType::Int), Box::new(VarType::BitVec(32)));

    let sort = arr_type.smtlib_sort();
    assert_eq!(sort.as_str(), "(Array Int (_ BitVec 32))");
}
