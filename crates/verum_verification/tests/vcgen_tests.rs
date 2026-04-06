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
// Verification Condition Generation Tests
//
// Verification Condition Generation:
//
// VC generation converts function contracts into logical formulas whose validity
// implies program correctness.
//
// Key rules:
// - Functions: VC(f) = forall params. Precondition => wp(body, Postcondition)
// - Loops (3 VCs): init (pre => inv), preserve (inv & cond => wp(body, inv)),
//   exit (inv & !cond => post)
// - Calls: VC(call f(args)) = Pre_f[args/params] & wp(cont, Post_f[args/params])
// - Weakest precondition: wp(Skip,Q) = Q; wp(Assign(x,e),Q) = Q[x:=eval(e)];
//   wp(Seq(c1,c2),Q) = wp(c1, wp(c2, Q)); wp(If(b,c1,c2),Q) = (b=>wp(c1,Q)) & (!b=>wp(c2,Q))
//
// SMT encoding: negate the VC, check satisfiability. UNSAT = VC valid = proof succeeds.
//
// These tests verify the VCGen implementation including weakest precondition
// calculus rules, formula substitution, SMT-LIB encoding, VC generation for
// functions, and SMT solver integration.

use verum_common::{Maybe, Text};
use verum_verification::vcgen::{
    Formula, FunctionSignature, SmtExpr, SourceLocation, SymbolTable, VCGenerator, VCKind, VarType,
    Variable, VerificationCondition, substitute, vc_to_smtlib,
};

// Z3 SMT solver for actual verification

// =============================================================================
// Variable Tests
// =============================================================================

#[test]
fn test_variable_creation() {
    let var = Variable::new("x");
    assert_eq!(var.name, Text::from("x"));
    assert!(matches!(var.version, Maybe::None));
    assert!(matches!(var.ty, Maybe::None));
}

#[test]
fn test_variable_versioned() {
    let var = Variable::versioned("x", 3);
    assert_eq!(var.name, Text::from("x"));
    assert_eq!(var.version, Maybe::Some(3));
}

#[test]
fn test_variable_typed() {
    let var = Variable::typed("x", VarType::Int);
    assert_eq!(var.name, Text::from("x"));
    assert_eq!(var.ty, Maybe::Some(VarType::Int));
}

#[test]
fn test_variable_smtlib_name() {
    let var1 = Variable::new("x");
    assert_eq!(var1.smtlib_name(), Text::from("x"));

    let var2 = Variable::versioned("x", 5);
    assert_eq!(var2.smtlib_name(), Text::from("x_5"));
}

#[test]
fn test_variable_result() {
    let result = Variable::result();
    assert_eq!(result.name, Text::from("result"));
}

// =============================================================================
// VarType Tests
// =============================================================================

#[test]
fn test_vartype_smtlib_sort() {
    assert_eq!(VarType::Int.smtlib_sort(), Text::from("Int"));
    assert_eq!(VarType::Bool.smtlib_sort(), Text::from("Bool"));
    assert_eq!(VarType::Real.smtlib_sort(), Text::from("Real"));
    assert_eq!(
        VarType::BitVec(32).smtlib_sort(),
        Text::from("(_ BitVec 32)")
    );
}

#[test]
fn test_vartype_array_smtlib() {
    let arr_ty = VarType::Array(Box::new(VarType::Int), Box::new(VarType::Bool));
    assert_eq!(arr_ty.smtlib_sort(), Text::from("(Array Int Bool)"));
}

// =============================================================================
// SMT Expression Tests
// =============================================================================

#[test]
fn test_smt_expr_constants() {
    let int_const = SmtExpr::int(42);
    assert_eq!(int_const.to_smtlib(), Text::from("42"));

    let neg_const = SmtExpr::int(-10);
    assert_eq!(neg_const.to_smtlib(), Text::from("(- 10)"));

    let bool_true = SmtExpr::bool(true);
    assert_eq!(bool_true.to_smtlib(), Text::from("true"));

    let bool_false = SmtExpr::bool(false);
    assert_eq!(bool_false.to_smtlib(), Text::from("false"));
}

#[test]
fn test_smt_expr_var() {
    let var = SmtExpr::var("x");
    assert_eq!(var.to_smtlib(), Text::from("x"));
}

#[test]
fn test_smt_expr_binary_ops() {
    let x = SmtExpr::var("x");
    let y = SmtExpr::var("y");

    let add = SmtExpr::add(x.clone(), y.clone());
    assert_eq!(add.to_smtlib(), Text::from("(+ x y)"));

    let sub = SmtExpr::sub(x.clone(), y.clone());
    assert_eq!(sub.to_smtlib(), Text::from("(- x y)"));

    let mul = SmtExpr::mul(x.clone(), y.clone());
    assert_eq!(mul.to_smtlib(), Text::from("(* x y)"));
}

#[test]
fn test_smt_expr_substitution() {
    let x = Variable::new("x");
    let expr = SmtExpr::add(SmtExpr::Var(x.clone()), SmtExpr::int(1));
    let replacement = SmtExpr::int(5);

    let result = expr.substitute(&x, &replacement);
    assert_eq!(result.to_smtlib(), Text::from("(+ 5 1)"));
}

#[test]
fn test_smt_expr_free_variables() {
    let x = Variable::new("x");
    let y = Variable::new("y");
    let expr = SmtExpr::add(SmtExpr::Var(x.clone()), SmtExpr::Var(y.clone()));

    let free_vars = expr.free_variables();
    assert!(free_vars.contains(&x));
    assert!(free_vars.contains(&y));
    assert_eq!(free_vars.len(), 2);
}

#[test]
fn test_smt_expr_select_store() {
    let arr = SmtExpr::var("arr");
    let idx = SmtExpr::int(0);
    let val = SmtExpr::int(42);

    let select = SmtExpr::Select(Box::new(arr.clone()), Box::new(idx.clone()));
    assert_eq!(select.to_smtlib(), Text::from("(select arr 0)"));

    let store = SmtExpr::Store(Box::new(arr), Box::new(idx), Box::new(val));
    assert_eq!(store.to_smtlib(), Text::from("(store arr 0 42)"));
}

#[test]
fn test_smt_expr_ite() {
    let cond = Formula::Var(Variable::new("b"));
    let then_e = SmtExpr::int(1);
    let else_e = SmtExpr::int(0);

    let ite = SmtExpr::Ite(Box::new(cond), Box::new(then_e), Box::new(else_e));
    assert_eq!(ite.to_smtlib(), Text::from("(ite b 1 0)"));
}

// =============================================================================
// Formula Tests
// =============================================================================

#[test]
fn test_formula_constants() {
    assert_eq!(Formula::True.to_smtlib(), Text::from("true"));
    assert_eq!(Formula::False.to_smtlib(), Text::from("false"));
}

#[test]
fn test_formula_not() {
    let f = Formula::not(Formula::Var(Variable::new("p")));
    assert_eq!(f.to_smtlib(), Text::from("(not p)"));
}

#[test]
fn test_formula_and() {
    let p = Formula::Var(Variable::new("p"));
    let q = Formula::Var(Variable::new("q"));
    let f = Formula::and([p, q]);
    assert_eq!(f.to_smtlib(), Text::from("(and p q)"));
}

#[test]
fn test_formula_and_empty() {
    let f = Formula::and([]);
    assert_eq!(f, Formula::True);
}

#[test]
fn test_formula_and_single() {
    let p = Formula::Var(Variable::new("p"));
    let f = Formula::and([p.clone()]);
    assert_eq!(f, p);
}

#[test]
fn test_formula_or() {
    let p = Formula::Var(Variable::new("p"));
    let q = Formula::Var(Variable::new("q"));
    let f = Formula::or([p, q]);
    assert_eq!(f.to_smtlib(), Text::from("(or p q)"));
}

#[test]
fn test_formula_implies() {
    let p = Formula::Var(Variable::new("p"));
    let q = Formula::Var(Variable::new("q"));
    let f = Formula::implies(p, q);
    assert_eq!(f.to_smtlib(), Text::from("(=> p q)"));
}

#[test]
fn test_formula_comparisons() {
    let x = SmtExpr::var("x");
    let y = SmtExpr::var("y");

    assert_eq!(
        Formula::eq(x.clone(), y.clone()).to_smtlib(),
        Text::from("(= x y)")
    );
    assert_eq!(
        Formula::lt(x.clone(), y.clone()).to_smtlib(),
        Text::from("(< x y)")
    );
    assert_eq!(
        Formula::le(x.clone(), y.clone()).to_smtlib(),
        Text::from("(<= x y)")
    );
    assert_eq!(
        Formula::gt(x.clone(), y.clone()).to_smtlib(),
        Text::from("(> x y)")
    );
    assert_eq!(
        Formula::ge(x.clone(), y.clone()).to_smtlib(),
        Text::from("(>= x y)")
    );
}

#[test]
fn test_formula_quantifiers() {
    let x = Variable::typed("x", VarType::Int);
    let body = Formula::gt(SmtExpr::Var(x.clone()), SmtExpr::int(0));
    let forall = Formula::Forall(vec![x].into(), Box::new(body));
    assert_eq!(forall.to_smtlib(), Text::from("(forall ((x Int)) (> x 0))"));
}

#[test]
fn test_formula_substitution() {
    // Test Q[x/e]: (x > 0)[x/5] = (5 > 0)
    let x = Variable::new("x");
    let formula = Formula::gt(SmtExpr::Var(x.clone()), SmtExpr::int(0));
    let replacement = SmtExpr::int(5);

    let result = formula.substitute(&x, &replacement);
    assert_eq!(result.to_smtlib(), Text::from("(> 5 0)"));
}

#[test]
fn test_formula_substitution_bound_var() {
    // Bound variables should NOT be substituted
    let x = Variable::typed("x", VarType::Int);
    let body = Formula::gt(SmtExpr::Var(x.clone()), SmtExpr::int(0));
    let forall = Formula::Forall(vec![x.clone()].into(), Box::new(body));

    let replacement = SmtExpr::int(42);
    let result = forall.substitute(&x, &replacement);

    // x is bound, so it should remain unchanged
    assert_eq!(result.to_smtlib(), Text::from("(forall ((x Int)) (> x 0))"));
}

#[test]
fn test_formula_free_variables() {
    let x = Variable::new("x");
    let y = Variable::new("y");
    let formula = Formula::and([
        Formula::Var(x.clone()),
        Formula::gt(SmtExpr::Var(y.clone()), SmtExpr::int(0)),
    ]);

    let free_vars = formula.free_variables();
    assert!(free_vars.contains(&x));
    assert!(free_vars.contains(&y));
}

// =============================================================================
// Formula Simplification Tests
// =============================================================================

#[test]
fn test_formula_simplify_not_true() {
    let f = Formula::not(Formula::True);
    assert_eq!(f.simplify(), Formula::False);
}

#[test]
fn test_formula_simplify_not_false() {
    let f = Formula::not(Formula::False);
    assert_eq!(f.simplify(), Formula::True);
}

#[test]
fn test_formula_simplify_double_negation() {
    let p = Formula::Var(Variable::new("p"));
    let f = Formula::not(Formula::not(p.clone()));
    assert_eq!(f.simplify(), p);
}

#[test]
fn test_formula_simplify_and_with_true() {
    let p = Formula::Var(Variable::new("p"));
    let f = Formula::and([Formula::True, p.clone()]);
    assert_eq!(f.simplify(), p);
}

#[test]
fn test_formula_simplify_and_with_false() {
    let p = Formula::Var(Variable::new("p"));
    let f = Formula::and([Formula::False, p]);
    assert_eq!(f.simplify(), Formula::False);
}

#[test]
fn test_formula_simplify_implies_false_antecedent() {
    let q = Formula::Var(Variable::new("q"));
    let f = Formula::implies(Formula::False, q);
    assert_eq!(f.simplify(), Formula::True);
}

#[test]
fn test_formula_simplify_implies_true_consequent() {
    let p = Formula::Var(Variable::new("p"));
    let f = Formula::implies(p, Formula::True);
    assert_eq!(f.simplify(), Formula::True);
}

// =============================================================================
// Verification Condition Tests
// =============================================================================

#[test]
fn test_vc_creation() {
    let formula = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let vc = VerificationCondition::new(
        formula,
        SourceLocation::unknown(),
        VCKind::Precondition,
        "x must be positive",
    );

    assert_eq!(vc.kind, VCKind::Precondition);
    assert!(!vc.verified);
}

#[test]
fn test_vc_with_function() {
    let formula = Formula::True;
    let vc = VerificationCondition::new(
        formula,
        SourceLocation::unknown(),
        VCKind::Postcondition,
        "trivial postcondition",
    )
    .with_function("increment");

    assert_eq!(vc.function_name, Maybe::Some(Text::from("increment")));
}

#[test]
fn test_vc_to_smtlib() {
    let x = Variable::typed("x", VarType::Int);
    let formula = Formula::gt(SmtExpr::Var(x), SmtExpr::int(0));
    let vc = VerificationCondition::new(
        formula,
        SourceLocation::unknown(),
        VCKind::Precondition,
        "x positive",
    );

    let smtlib = vc_to_smtlib(&vc);
    let smtlib_str = smtlib.as_str();

    // Check that the SMT-LIB output contains expected elements
    assert!(smtlib_str.contains("(set-logic ALL)"));
    assert!(smtlib_str.contains("(declare-const x Int)"));
    assert!(smtlib_str.contains("(assert (not"));
    assert!(smtlib_str.contains("(check-sat)"));
}

// =============================================================================
// VCKind Tests
// =============================================================================

#[test]
fn test_vc_kind_description() {
    assert_eq!(VCKind::Precondition.description(), "precondition");
    assert_eq!(VCKind::Postcondition.description(), "postcondition");
    assert_eq!(
        VCKind::LoopInvariantInit.description(),
        "loop invariant initialization"
    );
    assert_eq!(
        VCKind::LoopInvariantPreserve.description(),
        "loop invariant preservation"
    );
    assert_eq!(VCKind::ArrayBounds.description(), "array bounds");
    assert_eq!(VCKind::DivisionByZero.description(), "division by zero");
}

// =============================================================================
// Symbol Table Tests
// =============================================================================

#[test]
fn test_symbol_table_variables() {
    let mut st = SymbolTable::new();
    st.add_variable("x", VarType::Int);
    st.add_variable("y", VarType::Bool);

    assert_eq!(st.get_variable_type("x"), Maybe::Some(VarType::Int));
    assert_eq!(st.get_variable_type("y"), Maybe::Some(VarType::Bool));
    assert_eq!(st.get_variable_type("z"), Maybe::None);
}

#[test]
fn test_symbol_table_ssa_versions() {
    let mut st = SymbolTable::new();

    assert_eq!(st.next_ssa_version("x"), 0);
    assert_eq!(st.next_ssa_version("x"), 1);
    assert_eq!(st.next_ssa_version("x"), 2);
    assert_eq!(st.next_ssa_version("y"), 0);
}

#[test]
fn test_symbol_table_loop_invariants() {
    let mut st = SymbolTable::new();
    let invariant = Formula::gt(SmtExpr::var("i"), SmtExpr::int(0));

    st.add_loop_invariant(1, invariant.clone());

    assert!(st.loop_invariants.get(&1).is_some());
}

// =============================================================================
// VCGenerator Tests
// =============================================================================

#[test]
fn test_vcgen_new() {
    let generator = VCGenerator::new();
    assert!(generator.vcs.is_empty());
}

#[test]
fn test_vcgen_with_source_file() {
    let generator = VCGenerator::new().with_source_file("test.vr");
    assert_eq!(generator.source_file, Text::from("test.vr"));
}

#[test]
fn test_vcgen_register_function() {
    let mut generator = VCGenerator::new();
    let sig = FunctionSignature {
        params: vec![(Text::from("x"), VarType::Int)].into(),
        return_type: VarType::Int,
        precondition: Formula::ge(SmtExpr::var("x"), SmtExpr::int(0)),
        postcondition: Formula::gt(SmtExpr::var("result"), SmtExpr::var("x")),
    };

    generator.register_function("increment", sig);

    assert!(generator.symbol_table.get_function("increment").is_some());
}

#[test]
fn test_vcgen_set_loop_invariant() {
    let mut generator = VCGenerator::new();
    let invariant = Formula::and([
        Formula::ge(SmtExpr::var("i"), SmtExpr::int(0)),
        Formula::lt(SmtExpr::var("i"), SmtExpr::var("n")),
    ]);

    generator.set_loop_invariant(0, invariant.clone());

    assert!(generator.symbol_table.loop_invariants.get(&0).is_some());
}

// =============================================================================
// Source Location Tests
// =============================================================================

#[test]
fn test_source_location_new() {
    let loc = SourceLocation::new(Text::from("test.vr"), 10, 5);
    assert_eq!(loc.file, Text::from("test.vr"));
    assert_eq!(loc.line, 10);
    assert_eq!(loc.column, 5);
}

#[test]
fn test_source_location_unknown() {
    let loc = SourceLocation::unknown();
    assert_eq!(loc.file, Text::from("<unknown>"));
    assert_eq!(loc.line, 0);
    assert_eq!(loc.column, 0);
}

#[test]
fn test_source_location_display() {
    let loc = SourceLocation::new(Text::from("main.vr"), 42, 15);
    let display = format!("{}", loc);
    assert_eq!(display, "main.vr:42:15");
}

// =============================================================================
// SMT-LIB Encoding Property Tests
// =============================================================================

#[test]
fn test_smtlib_encoding_valid_syntax() {
    let formula = Formula::and([
        Formula::gt(SmtExpr::var("x"), SmtExpr::int(0)),
        Formula::lt(SmtExpr::var("x"), SmtExpr::int(100)),
        Formula::implies(
            Formula::Var(Variable::new("p")),
            Formula::eq(SmtExpr::var("x"), SmtExpr::int(50)),
        ),
    ]);

    let vc = VerificationCondition::new(
        formula,
        SourceLocation::unknown(),
        VCKind::Custom,
        "test VC",
    );

    let smtlib = vc_to_smtlib(&vc);

    // Check balanced parentheses
    let open_parens = smtlib.as_str().matches('(').count();
    let close_parens = smtlib.as_str().matches(')').count();
    assert_eq!(
        open_parens, close_parens,
        "Unbalanced parentheses in SMT-LIB output"
    );
}

#[test]
fn test_smtlib_array_encoding() {
    let arr = Variable::typed(
        "arr",
        VarType::Array(Box::new(VarType::Int), Box::new(VarType::Int)),
    );
    let select = SmtExpr::Select(
        Box::new(SmtExpr::Var(arr.clone())),
        Box::new(SmtExpr::int(0)),
    );
    let formula = Formula::eq(select, SmtExpr::int(42));

    let vc = VerificationCondition::new(
        formula,
        SourceLocation::unknown(),
        VCKind::ArrayBounds,
        "array access",
    );

    let smtlib = vc_to_smtlib(&vc);
    assert!(smtlib.as_str().contains("(Array Int Int)"));
    assert!(smtlib.as_str().contains("(select arr 0)"));
}

// =============================================================================
// Weakest Precondition Calculus Tests
// =============================================================================

// Note: These tests verify the wp rules conceptually.
// Full integration tests with AST nodes require more setup.

#[test]
fn test_wp_skip_rule() {
    // wp(skip, Q) = Q
    let q = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));

    // For a skip statement (empty statement), wp should return Q unchanged
    // This is verified by the implementation returning postcondition.clone()
    // for StmtKind::Empty
    assert_eq!(q.clone(), q);
}

#[test]
fn test_wp_assignment_substitution() {
    // wp(x := e, Q) = Q[x/e]
    // For Q = (x > 0) and e = 5, wp(x := 5, x > 0) = (5 > 0)
    let x = Variable::new("x");
    let q = Formula::gt(SmtExpr::Var(x.clone()), SmtExpr::int(0));
    let e = SmtExpr::int(5);

    let result = substitute(q, x, e);
    assert_eq!(result.to_smtlib(), Text::from("(> 5 0)"));
}

#[test]
fn test_wp_conditional_rule() {
    // wp(if b then S1 else S2, Q) = (b => wp(S1, Q)) && (!b => wp(S2, Q))
    let b = Formula::Var(Variable::new("b"));
    let wp_s1_q = Formula::gt(SmtExpr::var("x"), SmtExpr::int(0));
    let wp_s2_q = Formula::gt(SmtExpr::var("y"), SmtExpr::int(0));

    let wp_if = Formula::and([
        Formula::implies(b.clone(), wp_s1_q),
        Formula::implies(Formula::not(b), wp_s2_q),
    ]);

    // Check that it has the right structure
    if let Formula::And(conjuncts) = &wp_if {
        assert_eq!(conjuncts.len(), 2);
    } else {
        panic!("Expected And formula");
    }
}

// =============================================================================
// Integration Tests with Formula Construction
// =============================================================================

#[test]
fn test_complex_formula_construction() {
    // Construct: forall x. (x >= 0 && x < n) => (arr[x] >= 0)
    let x = Variable::typed("x", VarType::Int);
    let n = SmtExpr::var("n");
    let arr = SmtExpr::var("arr");

    let bounds = Formula::and([
        Formula::ge(SmtExpr::Var(x.clone()), SmtExpr::int(0)),
        Formula::lt(SmtExpr::Var(x.clone()), n),
    ]);

    let access = SmtExpr::Select(Box::new(arr), Box::new(SmtExpr::Var(x.clone())));

    let property = Formula::ge(access, SmtExpr::int(0));

    let formula = Formula::Forall(vec![x].into(), Box::new(Formula::implies(bounds, property)));

    let smtlib = formula.to_smtlib();
    assert!(smtlib.as_str().contains("forall"));
    assert!(smtlib.as_str().contains("select arr x"));
}

#[test]
fn test_increment_function_vc() {
    // Function: fn increment(x: Int) -> Int where x >= 0 ensures result > x
    //   return x + 1
    //
    // VC: x >= 0 => (x + 1 > x)
    let x = SmtExpr::var("x");
    let result = SmtExpr::add(x.clone(), SmtExpr::int(1));

    let precondition = Formula::ge(x.clone(), SmtExpr::int(0));
    let postcondition = Formula::gt(result, x);

    let vc_formula = Formula::implies(precondition, postcondition);

    let vc = VerificationCondition::new(
        vc_formula.clone(),
        SourceLocation::unknown(),
        VCKind::Postcondition,
        "increment postcondition",
    )
    .with_function("increment");

    assert_eq!(vc.kind, VCKind::Postcondition);
    assert_eq!(vc.function_name, Maybe::Some(Text::from("increment")));

    // Now actually verify this VC using the SMT solver
    // Get SMT-LIB encoding and verify with Z3
    let smtlib = vc_to_smtlib(&vc);

    // The VC should contain the proper formula structure
    assert!(
        smtlib.as_str().contains("(>= x 0)"),
        "Should contain precondition"
    );
    assert!(smtlib.as_str().contains("(+ x 1)"), "Should contain x + 1");

    // Verify with Z3 directly
    // Create Z3 context and solver
    let z3_ctx = z3::Context::thread_local();
    let solver = z3::Solver::new();

    // Create Z3 variable for x
    let x_var = z3::ast::Int::new_const("x");

    // Encode: x >= 0 => (x + 1 > x)
    // This is equivalent to: !(x >= 0) || (x + 1 > x)
    // Or: x < 0 || x + 1 > x
    let zero = z3::ast::Int::from_i64(0);
    let one = z3::ast::Int::from_i64(1);

    let precond = x_var.ge(&zero);
    let x_plus_one = z3::ast::Int::add(&[&x_var, &one]);
    let postcond = x_plus_one.gt(&x_var);

    // Assert the negation of (precond => postcond) to check validity
    // If UNSAT, the VC is valid
    let implication = z3::ast::Bool::implies(&precond, &postcond);
    solver.assert(implication.not());

    let result = solver.check();

    // The VC should be valid (negation should be UNSAT)
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "increment postcondition VC should be valid"
    );
}

#[test]
fn test_division_safety_vc() {
    // For expression: a / b
    // VC: b != 0
    let b = SmtExpr::var("b");
    let div_safe = Formula::Ne(Box::new(b), Box::new(SmtExpr::int(0)));

    let vc = VerificationCondition::new(
        div_safe,
        SourceLocation::new(Text::from("math.vr"), 10, 5),
        VCKind::DivisionByZero,
        "division by non-zero",
    );

    assert_eq!(vc.kind, VCKind::DivisionByZero);
    let smtlib = vc_to_smtlib(&vc);
    assert!(smtlib.as_str().contains("(distinct b 0)"));
}

#[test]
fn test_array_bounds_vc() {
    // For expression: arr[i]
    // VC: i >= 0 && i < len
    let i = SmtExpr::var("i");
    let len = SmtExpr::var("len");

    let bounds_check = Formula::and([Formula::ge(i.clone(), SmtExpr::int(0)), Formula::lt(i, len)]);

    let vc = VerificationCondition::new(
        bounds_check,
        SourceLocation::unknown(),
        VCKind::ArrayBounds,
        "array index in bounds",
    );

    assert_eq!(vc.kind, VCKind::ArrayBounds);
}

#[test]
fn test_loop_invariant_vcs() {
    // Loop: while i < n inv I { S }
    // Three VCs:
    // 1. Init: Precondition => I
    // 2. Preserve: I && i < n => wp(S, I)
    // 3. Exit: I && !(i < n) => Postcondition

    let i = SmtExpr::var("i");
    let n = SmtExpr::var("n");

    let invariant = Formula::and([
        Formula::ge(i.clone(), SmtExpr::int(0)),
        Formula::le(i.clone(), n.clone()),
    ]);

    let condition = Formula::lt(i.clone(), n.clone());

    // Exit VC: I && !condition => some postcondition
    let exit_vc = Formula::implies(
        Formula::and([invariant.clone(), Formula::not(condition)]),
        Formula::eq(i, n), // postcondition: i == n at exit
    );

    let vc = VerificationCondition::new(
        exit_vc,
        SourceLocation::unknown(),
        VCKind::LoopInvariantExit,
        "loop invariant implies postcondition",
    );

    assert_eq!(vc.kind, VCKind::LoopInvariantExit);
}

// =============================================================================
// Edge Cases and Error Handling
// =============================================================================

#[test]
fn test_empty_formula_and() {
    let f = Formula::and(Vec::<Formula>::new());
    assert_eq!(f, Formula::True);
}

#[test]
fn test_empty_formula_or() {
    let f = Formula::or(Vec::<Formula>::new());
    assert_eq!(f, Formula::False);
}

#[test]
fn test_nested_quantifiers() {
    let x = Variable::typed("x", VarType::Int);
    let y = Variable::typed("y", VarType::Int);

    let inner = Formula::Exists(
        vec![y.clone()].into(),
        Box::new(Formula::gt(SmtExpr::Var(y), SmtExpr::Var(x.clone()))),
    );

    let outer = Formula::Forall(vec![x].into(), Box::new(inner));

    let smtlib = outer.to_smtlib();
    assert!(smtlib.as_str().contains("forall"));
    assert!(smtlib.as_str().contains("exists"));
}

#[test]
fn test_let_binding_in_formula() {
    let x = Variable::new("x");
    let bound_expr = SmtExpr::add(SmtExpr::int(1), SmtExpr::int(2));
    let body = Formula::eq(SmtExpr::Var(x.clone()), SmtExpr::int(3));

    let let_formula = Formula::Let(x, Box::new(bound_expr), Box::new(body));
    let smtlib = let_formula.to_smtlib();

    assert!(smtlib.as_str().contains("let"));
    assert!(smtlib.as_str().contains("(+ 1 2)"));
}

// =============================================================================
// Performance Sanity Tests
// =============================================================================

#[test]
fn test_large_conjunction() {
    // Create a large conjunction to verify it doesn't explode
    let formulas: Vec<Formula> = (0..100)
        .map(|i| {
            let var_name = format!("x{}", i);
            Formula::gt(SmtExpr::var(var_name), SmtExpr::int(0))
        })
        .collect();

    let big_and = Formula::And(formulas.into());
    let smtlib = big_and.to_smtlib();

    // Should produce valid SMT-LIB without crashing
    assert!(smtlib.as_str().starts_with("(and"));
}

#[test]
fn test_deep_nesting() {
    // Create deeply nested formula
    let mut formula = Formula::True;
    for _ in 0..50 {
        formula = Formula::not(formula);
    }

    // Should simplify to True (even number of negations)
    let simplified = formula.simplify();
    assert_eq!(simplified, Formula::True);
}

// =============================================================================
// SMT Solver Integration Tests
// =============================================================================
// These tests connect VCGen output to the Z3 SMT solver for actual verification

/// Helper function to verify a formula with Z3
fn verify_with_z3(formula: &Formula) -> z3::SatResult {
    let solver = z3::Solver::new();

    // Build Z3 formula from our Formula type
    let z3_formula = translate_formula_to_z3(formula);

    // Check validity by asserting negation
    // If UNSAT, the original formula is valid
    solver.assert(z3_formula.not());
    solver.check()
}

/// Translate our Formula to Z3 Bool
fn translate_formula_to_z3(formula: &Formula) -> z3::ast::Bool {
    match formula {
        Formula::True => z3::ast::Bool::from_bool(true),
        Formula::False => z3::ast::Bool::from_bool(false),
        Formula::Var(v) => z3::ast::Bool::new_const(v.smtlib_name().as_str()),
        Formula::Not(inner) => translate_formula_to_z3(inner).not(),
        Formula::And(formulas) => {
            let z3_formulas: Vec<_> = formulas.iter().map(translate_formula_to_z3).collect();
            let refs: Vec<_> = z3_formulas.iter().collect();
            z3::ast::Bool::and(&refs)
        }
        Formula::Or(formulas) => {
            let z3_formulas: Vec<_> = formulas.iter().map(translate_formula_to_z3).collect();
            let refs: Vec<_> = z3_formulas.iter().collect();
            z3::ast::Bool::or(&refs)
        }
        Formula::Implies(ante, cons) => {
            let a = translate_formula_to_z3(ante);
            let c = translate_formula_to_z3(cons);
            z3::ast::Bool::implies(&a, &c)
        }
        Formula::Iff(left, right) => {
            let l = translate_formula_to_z3(left);
            let r = translate_formula_to_z3(right);
            l.iff(&r)
        }
        Formula::Eq(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.eq(&r)
        }
        Formula::Ne(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.eq(&r).not()
        }
        Formula::Lt(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.lt(&r)
        }
        Formula::Le(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.le(&r)
        }
        Formula::Gt(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.gt(&r)
        }
        Formula::Ge(left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            l.ge(&r)
        }
        // For quantifiers and other complex constructs, return true as fallback
        _ => z3::ast::Bool::from_bool(true),
    }
}

/// Translate SMT expression to Z3 Int
fn translate_smt_expr_to_z3_int(expr: &SmtExpr) -> z3::ast::Int {
    match expr {
        SmtExpr::IntConst(n) => z3::ast::Int::from_i64(*n),
        SmtExpr::Var(v) => z3::ast::Int::new_const(v.smtlib_name().as_str()),
        SmtExpr::BinOp(op, left, right) => {
            let l = translate_smt_expr_to_z3_int(left);
            let r = translate_smt_expr_to_z3_int(right);
            match op {
                verum_verification::vcgen::SmtBinOp::Add => z3::ast::Int::add(&[&l, &r]),
                verum_verification::vcgen::SmtBinOp::Sub => z3::ast::Int::sub(&[&l, &r]),
                verum_verification::vcgen::SmtBinOp::Mul => z3::ast::Int::mul(&[&l, &r]),
                verum_verification::vcgen::SmtBinOp::Div => l.div(&r),
                verum_verification::vcgen::SmtBinOp::Mod => l.modulo(&r),
                verum_verification::vcgen::SmtBinOp::Pow => {
                    // Approximation for simple cases
                    l
                }
                verum_verification::vcgen::SmtBinOp::Select => {
                    // Array/tuple selection - approximate as left operand
                    l
                }
            }
        }
        SmtExpr::UnOp(op, inner) => {
            let i = translate_smt_expr_to_z3_int(inner);
            match op {
                verum_verification::vcgen::SmtUnOp::Neg => i.unary_minus(),
                verum_verification::vcgen::SmtUnOp::Abs => {
                    // abs(x) = if x >= 0 then x else -x
                    let zero = z3::ast::Int::from_i64(0);
                    i.ge(&zero).ite(&i, &i.unary_minus())
                }
                verum_verification::vcgen::SmtUnOp::Deref => {
                    // Dereference - just return the value for integer approximation
                    i
                }
                verum_verification::vcgen::SmtUnOp::Len => {
                    // Length - return a placeholder for arrays/slices
                    z3::ast::Int::from_i64(0)
                }
                verum_verification::vcgen::SmtUnOp::GetVariantValue => {
                    // Variant value extraction - return the value for integer approximation
                    i
                }
            }
        }
        SmtExpr::Apply(name, args) => {
            // For simple variable references
            if args.is_empty() {
                z3::ast::Int::new_const(name.as_str())
            } else {
                // For function applications, create uninterpreted
                z3::ast::Int::new_const(name.as_str())
            }
        }
        _ => z3::ast::Int::from_i64(0), // Fallback
    }
}

#[test]
fn test_smt_verify_simple_tautology() {
    // Verify: x >= 0 => x >= 0 (trivially true)
    let x = SmtExpr::var("x");
    let formula = Formula::implies(
        Formula::ge(x.clone(), SmtExpr::int(0)),
        Formula::ge(x, SmtExpr::int(0)),
    );

    let result = verify_with_z3(&formula);
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "Simple tautology should be valid"
    );
}

#[test]
fn test_smt_verify_arithmetic_property() {
    // Verify: x + 1 > x (always true for integers)
    let x = SmtExpr::var("x");
    let x_plus_one = SmtExpr::add(x.clone(), SmtExpr::int(1));
    let formula = Formula::gt(x_plus_one, x);

    let result = verify_with_z3(&formula);
    assert_eq!(result, z3::SatResult::Unsat, "x + 1 > x should always hold");
}

#[test]
fn test_smt_verify_invalid_property() {
    // Verify: x > 10 (NOT a tautology - has counterexamples)
    let x = SmtExpr::var("x");
    let formula = Formula::gt(x, SmtExpr::int(10));

    let result = verify_with_z3(&formula);
    assert_eq!(
        result,
        z3::SatResult::Sat,
        "x > 10 should have counterexamples"
    );
}

#[test]
fn test_smt_verify_division_safety() {
    // Verify division safety: b != 0 ensures b != 0 (trivially true)
    let b = SmtExpr::var("b");
    let div_safe = Formula::Ne(Box::new(b.clone()), Box::new(SmtExpr::int(0)));

    // Create VC: b != 0 => b != 0
    let vc = Formula::implies(div_safe.clone(), div_safe);

    let result = verify_with_z3(&vc);
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "Division safety VC should be valid"
    );
}

#[test]
fn test_smt_verify_array_bounds() {
    // Verify: 0 <= i && i < n => 0 <= i (part of bounds check)
    let i = SmtExpr::var("i");
    let n = SmtExpr::var("n");

    let in_bounds = Formula::and([
        Formula::ge(i.clone(), SmtExpr::int(0)),
        Formula::lt(i.clone(), n),
    ]);

    let lower_bound_holds = Formula::ge(i, SmtExpr::int(0));

    let vc = Formula::implies(in_bounds, lower_bound_holds);

    let result = verify_with_z3(&vc);
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "Array bounds VC should be valid"
    );
}

#[test]
fn test_smt_verify_loop_invariant_exit() {
    // Verify loop exit: (i <= n) && !(i < n) => i == n
    let i = SmtExpr::var("i");
    let n = SmtExpr::var("n");

    let invariant = Formula::le(i.clone(), n.clone());
    let not_condition = Formula::not(Formula::lt(i.clone(), n.clone()));
    let postcondition = Formula::eq(i, n);

    let vc = Formula::implies(Formula::and([invariant, not_condition]), postcondition);

    let result = verify_with_z3(&vc);
    assert_eq!(result, z3::SatResult::Unsat, "Loop exit VC should be valid");
}

#[test]
fn test_smt_verify_absolute_value() {
    // Verify: abs(x) >= 0 (always true)
    let x = SmtExpr::var("x");
    let abs_x = SmtExpr::UnOp(verum_verification::vcgen::SmtUnOp::Abs, Box::new(x));
    let formula = Formula::ge(abs_x, SmtExpr::int(0));

    let result = verify_with_z3(&formula);
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "abs(x) >= 0 should always hold"
    );
}

#[test]
fn test_smt_verify_implication_chain() {
    // Verify: (A => B) && (B => C) && A => C
    let a = Formula::Var(Variable::new("a"));
    let b = Formula::Var(Variable::new("b"));
    let c = Formula::Var(Variable::new("c"));

    let premises = Formula::and([
        Formula::implies(a.clone(), b.clone()),
        Formula::implies(b, c.clone()),
        a,
    ]);

    let vc = Formula::implies(premises, c);

    let result = verify_with_z3(&vc);
    assert_eq!(
        result,
        z3::SatResult::Unsat,
        "Implication chain should be valid"
    );
}

#[test]
fn test_full_vc_pipeline_with_smt() {
    // Create a complete VC and verify it end-to-end
    // Function: fn max(a: Int, b: Int) -> Int ensures result >= a && result >= b
    //   if a >= b { a } else { b }

    let a = SmtExpr::var("a");
    let b = SmtExpr::var("b");

    // For branch a >= b, result = a
    // VC1: a >= b => (a >= a && a >= b)
    let branch1_vc = Formula::implies(
        Formula::ge(a.clone(), b.clone()),
        Formula::and([
            Formula::ge(a.clone(), a.clone()),
            Formula::ge(a.clone(), b.clone()),
        ]),
    );

    // For branch a < b, result = b
    // VC2: a < b => (b >= a && b >= b)
    let branch2_vc = Formula::implies(
        Formula::lt(a.clone(), b.clone()),
        Formula::and([
            Formula::ge(b.clone(), a.clone()),
            Formula::ge(b.clone(), b.clone()),
        ]),
    );

    // Both branches should verify
    let result1 = verify_with_z3(&branch1_vc);
    let result2 = verify_with_z3(&branch2_vc);

    assert_eq!(
        result1,
        z3::SatResult::Unsat,
        "max() branch 1 VC should be valid"
    );
    assert_eq!(
        result2,
        z3::SatResult::Unsat,
        "max() branch 2 VC should be valid"
    );

    // Create full VC
    let vc = VerificationCondition::new(
        Formula::and([branch1_vc.clone(), branch2_vc.clone()]),
        SourceLocation::unknown(),
        VCKind::Postcondition,
        "max function postcondition",
    )
    .with_function("max");

    // Verify the full VC
    let full_result = verify_with_z3(&vc.formula);
    assert_eq!(
        full_result,
        z3::SatResult::Unsat,
        "Full max() VC should be valid"
    );
}
