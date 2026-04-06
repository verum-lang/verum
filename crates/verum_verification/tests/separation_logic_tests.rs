//! Comprehensive tests for Separation Logic implementation
//!
//! Separation Logic for heap reasoning:
//! - SepProp: Heap -> Prop (assertions over heap state)
//! - Separating conjunction (P * Q): heap splits into disjoint h1, h2 with P(h1) and Q(h2)
//! - Points-to predicate (x |-> v): singleton heap {x: v}
//! - Frame rule: {P} c {Q} implies {P * R} c {Q * R} (local reasoning)
//! - Used for verifying heap-manipulating programs (e.g., list reversal correctness)

use verum_common::List;
use verum_verification::separation_logic::*;
use verum_verification::vcgen::{Formula, SmtExpr, Variable};

// =============================================================================
// Basic Construction Tests
// =============================================================================

#[test]
fn test_address_construction() {
    // Concrete address
    let concrete = Address::concrete(0x1000);
    match concrete.0 {
        SmtExpr::IntConst(n) => assert_eq!(n, 0x1000),
        _ => panic!("Expected IntConst"),
    }

    // Symbolic address
    let symbolic = Address::symbolic("ptr");
    match symbolic.0 {
        SmtExpr::Var(ref v) => assert_eq!(v.name.as_str(), "ptr"),
        _ => panic!("Expected Var"),
    }

    // Null address
    let null = Address::null();
    match null.0 {
        SmtExpr::IntConst(0) => (),
        _ => panic!("Expected null address (0)"),
    }
}

#[test]
fn test_address_offset() {
    let base = Address::concrete(100);
    let offset = base.offset(10);

    match offset.0 {
        SmtExpr::BinOp(_, _, _) => (), // Should be an addition
        _ => panic!("Expected BinOp for offset"),
    }
}

#[test]
fn test_address_predicates() {
    let addr = Address::symbolic("x");

    // Test is_null predicate
    let is_null = addr.is_null();
    match is_null {
        Formula::Eq(_, _) => (),
        _ => panic!("Expected Eq formula"),
    }

    // Test is_nonnull predicate
    let is_nonnull = addr.is_nonnull();
    match is_nonnull {
        Formula::Ne(_, _) => (),
        _ => panic!("Expected Ne formula"),
    }
}

#[test]
fn test_value_construction() {
    // Integer value
    let int_val = Value::int(42);
    match int_val {
        Value::Int(SmtExpr::IntConst(42)) => (),
        _ => panic!("Expected Int(42)"),
    }

    // Boolean value
    let bool_val = Value::bool(true);
    match bool_val {
        Value::Bool(SmtExpr::BoolConst(true)) => (),
        _ => panic!("Expected Bool(true)"),
    }

    // Address value
    let addr = Address::concrete(0x1000);
    let addr_val = Value::addr(addr);
    match addr_val {
        Value::Addr(_) => (),
        _ => panic!("Expected Addr"),
    }
}

// =============================================================================
// Separation Logic Assertions Tests
// =============================================================================

#[test]
fn test_emp_assertion() {
    let emp = SepProp::emp();
    match emp {
        SepProp::Emp => (),
        _ => panic!("Expected Emp"),
    }
}

#[test]
fn test_points_to_assertion() {
    let addr = Address::concrete(100);
    let val = Value::int(42);
    let prop = SepProp::points_to(addr.clone(), val.clone());

    match prop {
        SepProp::PointsTo(a, v) => {
            assert_eq!(a, addr);
            match v {
                Value::Int(SmtExpr::IntConst(42)) => (),
                _ => panic!("Expected value 42"),
            }
        }
        _ => panic!("Expected PointsTo"),
    }
}

#[test]
fn test_field_points_to() {
    let addr = Address::symbolic("obj");
    let val = Value::int(10);
    let prop = SepProp::field_points_to(addr.clone(), "x", val);

    match prop {
        SepProp::FieldPointsTo(a, field, _) => {
            assert_eq!(a, addr);
            assert_eq!(field.as_str(), "x");
        }
        _ => panic!("Expected FieldPointsTo"),
    }
}

#[test]
fn test_separating_conjunction() {
    let p = SepProp::points_to(Address::concrete(100), Value::int(1));
    let q = SepProp::points_to(Address::concrete(200), Value::int(2));

    let star = SepProp::sep_conj(p.clone(), q.clone());

    match star {
        SepProp::SeparatingConj(left, right) => {
            assert_eq!(*left, p);
            assert_eq!(*right, q);
        }
        _ => panic!("Expected SeparatingConj"),
    }
}

#[test]
fn test_emp_identity_left() {
    let p = SepProp::points_to(Address::concrete(100), Value::int(1));
    let emp = SepProp::Emp;

    // emp * P = P
    let result = SepProp::sep_conj(emp, p.clone());
    assert_eq!(result, p);
}

#[test]
fn test_emp_identity_right() {
    let p = SepProp::points_to(Address::concrete(100), Value::int(1));
    let emp = SepProp::Emp;

    // P * emp = P
    let result = SepProp::sep_conj(p.clone(), emp);
    assert_eq!(result, p);
}

#[test]
fn test_magic_wand() {
    let p = SepProp::points_to(Address::concrete(100), Value::int(1));
    let q = SepProp::points_to(Address::concrete(200), Value::int(2));

    let wand = SepProp::magic_wand(p.clone(), q.clone());

    match wand {
        SepProp::MagicWand(left, right) => {
            assert_eq!(*left, p);
            assert_eq!(*right, q);
        }
        _ => panic!("Expected MagicWand"),
    }
}

#[test]
fn test_pure_assertion() {
    let formula = Formula::Eq(Box::new(SmtExpr::var("x")), Box::new(SmtExpr::IntConst(42)));
    let prop = SepProp::pure(formula.clone());

    match prop {
        SepProp::Pure(f) => assert_eq!(f, formula),
        _ => panic!("Expected Pure"),
    }
}

#[test]
fn test_star_multiple() {
    let props = vec![
        SepProp::points_to(Address::concrete(100), Value::int(1)),
        SepProp::points_to(Address::concrete(200), Value::int(2)),
        SepProp::points_to(Address::concrete(300), Value::int(3)),
    ];

    let result = SepProp::star(props);

    // Should be nested separating conjunctions
    match result {
        SepProp::SeparatingConj(_, _) => (),
        _ => panic!("Expected SeparatingConj"),
    }
}

// =============================================================================
// Standard Predicates Tests
// =============================================================================

#[test]
fn test_empty_list() {
    let head = Address::null();
    let contents: List<Value> = vec![].into();

    let list = StandardPredicates::list(head.clone(), contents);

    // Empty list should be emp with null constraint
    match list {
        SepProp::SeparatingConj(_, _) => (), // Pure constraint + emp
        SepProp::Pure(_) => (),
        _ => panic!("Empty list should have pure constraint"),
    }
}

#[test]
fn test_singleton_list() {
    let head = Address::symbolic("head");
    let contents: List<Value> = vec![Value::int(42)].into();

    let list = StandardPredicates::list(head, contents);

    // Should be existentially quantified
    match list {
        SepProp::Exists(_, _) => (),
        _ => panic!("Expected Exists for non-empty list"),
    }
}

#[test]
fn test_multi_element_list() {
    let head = Address::symbolic("head");
    let contents: List<Value> = vec![Value::int(1), Value::int(2), Value::int(3)].into();

    let list = StandardPredicates::list(head, contents);

    match list {
        SepProp::Exists(_, _) => (),
        _ => panic!("Expected Exists for multi-element list"),
    }
}

#[test]
fn test_array_empty() {
    let base = Address::concrete(0x1000);
    let data: List<Value> = vec![].into();

    let array = StandardPredicates::array(base, 0, data);

    // Empty array should be emp
    match array {
        SepProp::Emp => (),
        _ => panic!("Expected Emp for empty array"),
    }
}

#[test]
fn test_array_single_element() {
    let base = Address::concrete(0x1000);
    let data: List<Value> = vec![Value::int(42)].into();

    let array = StandardPredicates::array(base.clone(), 1, data);

    // Single element array should be points-to
    match array {
        SepProp::PointsTo(addr, val) => {
            assert_eq!(addr, base);
            match val {
                Value::Int(SmtExpr::IntConst(42)) => (),
                _ => panic!("Expected value 42"),
            }
        }
        _ => panic!("Expected PointsTo for single-element array"),
    }
}

#[test]
fn test_array_multiple_elements() {
    let base = Address::concrete(0x1000);
    let data: List<Value> = vec![Value::int(1), Value::int(2), Value::int(3)].into();

    let array = StandardPredicates::array(base, 3, data);

    // Multiple elements should create separating conjunction
    match array {
        SepProp::SeparatingConj(_, _) => (),
        _ => panic!("Expected SeparatingConj for multi-element array"),
    }
}

#[test]
fn test_cbgr_object_predicate() {
    let addr = Address::concrete(0x1000);
    let generation = 5;
    let epoch = 0;

    let obj = StandardPredicates::cbgr_object(addr, generation, epoch);

    match obj {
        SepProp::Predicate(name, args) => {
            assert_eq!(name.as_str(), "cbgr_object");
            assert_eq!(args.len(), 3);
        }
        _ => panic!("Expected Predicate"),
    }
}

// =============================================================================
// Substitution Tests
// =============================================================================

#[test]
fn test_substitute_in_points_to() {
    let var = Variable::new("x");
    let addr = Address::symbolic("x");
    let val = Value::int(42);
    let prop = SepProp::points_to(addr, val);

    let replacement = SmtExpr::IntConst(100);
    let result = prop.substitute(&var, &replacement);

    match result {
        SepProp::PointsTo(Address(SmtExpr::IntConst(100)), _) => (),
        _ => panic!("Substitution failed"),
    }
}

#[test]
fn test_substitute_in_value() {
    let var = Variable::new("y");
    let addr = Address::concrete(100);
    let val = Value::Int(SmtExpr::var("y"));
    let prop = SepProp::points_to(addr.clone(), val);

    let replacement = SmtExpr::IntConst(999);
    let result = prop.substitute(&var, &replacement);

    match result {
        SepProp::PointsTo(a, Value::Int(SmtExpr::IntConst(999))) => {
            assert_eq!(a, addr);
        }
        _ => panic!("Value substitution failed"),
    }
}

#[test]
fn test_substitute_in_separating_conj() {
    let var = Variable::new("x");
    let p = SepProp::points_to(Address::symbolic("x"), Value::int(1));
    let q = SepProp::points_to(Address::symbolic("x"), Value::int(2));
    let star = SepProp::sep_conj(p, q);

    let replacement = SmtExpr::IntConst(200);
    let result = star.substitute(&var, &replacement);

    match result {
        SepProp::SeparatingConj(left, right) => {
            // Both should have substituted addresses
            match (*left, *right) {
                (
                    SepProp::PointsTo(Address(SmtExpr::IntConst(200)), _),
                    SepProp::PointsTo(Address(SmtExpr::IntConst(200)), _),
                ) => (),
                _ => panic!("Substitution in sep conj failed"),
            }
        }
        _ => panic!("Expected SeparatingConj"),
    }
}

#[test]
fn test_substitute_respects_quantifier_binding() {
    let var = Variable::new("x");
    let inner = SepProp::points_to(Address::symbolic("x"), Value::int(1));
    let exists = SepProp::Exists(vec![var.clone()].into(), Box::new(inner.clone()));

    let replacement = SmtExpr::IntConst(999);
    let result = exists.substitute(&var, &replacement);

    // x is bound, so substitution shouldn't affect the inner formula
    match result {
        SepProp::Exists(_, inner_result) => {
            assert_eq!(*inner_result, inner);
        }
        _ => panic!("Expected Exists"),
    }
}

// =============================================================================
// Free Variables Tests
// =============================================================================

#[test]
fn test_free_vars_in_points_to() {
    let addr = Address::symbolic("x");
    let val = Value::Int(SmtExpr::var("y"));
    let prop = SepProp::points_to(addr, val);

    let free_vars = prop.free_variables();
    assert_eq!(free_vars.len(), 2);
    assert!(free_vars.contains(&Variable::new("x")));
    assert!(free_vars.contains(&Variable::new("y")));
}

#[test]
fn test_free_vars_in_separating_conj() {
    let p = SepProp::points_to(Address::symbolic("x"), Value::int(1));
    let q = SepProp::points_to(Address::symbolic("y"), Value::int(2));
    let star = SepProp::sep_conj(p, q);

    let free_vars = star.free_variables();
    assert_eq!(free_vars.len(), 2);
    assert!(free_vars.contains(&Variable::new("x")));
    assert!(free_vars.contains(&Variable::new("y")));
}

#[test]
fn test_free_vars_with_quantifier() {
    let var = Variable::new("x");
    let inner = SepProp::points_to(Address::symbolic("x"), Value::Int(SmtExpr::var("y")));
    let exists = SepProp::Exists(vec![var.clone()].into(), Box::new(inner));

    let free_vars = exists.free_variables();
    // x is bound, only y should be free
    assert_eq!(free_vars.len(), 1);
    assert!(free_vars.contains(&Variable::new("y")));
    assert!(!free_vars.contains(&Variable::new("x")));
}

// =============================================================================
// Weakest Precondition Tests
// =============================================================================

#[test]
fn test_wp_alloc() {
    let var = Variable::new("ptr");
    let val = Value::int(42);
    let post = SepProp::points_to(Address::symbolic("ptr"), Value::int(42));

    let cmd = HeapCommand::Alloc(var, val);
    let wp = wp_heap(&cmd, post);

    // wp(x := alloc(v), Q) should be existentially quantified
    match wp {
        SepProp::Exists(vars, _) => {
            assert_eq!(vars.len(), 1);
        }
        _ => panic!("Expected Exists for alloc wp"),
    }
}

#[test]
fn test_wp_free() {
    let addr = Address::symbolic("ptr");
    let post = SepProp::emp();

    let cmd = HeapCommand::Free(addr.clone());
    let wp = wp_heap(&cmd, post);

    // wp(free(x), Q) should be (x ↦ _) * Q
    // When Q=emp, this simplifies to just (x ↦ _) by the unit law
    match wp {
        SepProp::PointsTo(_, _) => (), // Simplified when Q=emp
        SepProp::SeparatingConj(_, _) => (),
        SepProp::Exists(_, _) => (), // Old value is existentially quantified
        _ => panic!("Expected PointsTo, SeparatingConj or Exists for free wp"),
    }
}

#[test]
fn test_wp_store() {
    let addr = Address::concrete(100);
    let val = Value::int(99);
    let post = SepProp::points_to(addr.clone(), val.clone());

    let cmd = HeapCommand::Store(addr, val);
    let wp = wp_heap(&cmd, post);

    // wp(*x := v, Q) should have existential quantifier for old value
    match wp {
        SepProp::Exists(_, _) => (),
        _ => panic!("Expected Exists for store wp"),
    }
}

#[test]
fn test_wp_load() {
    let var = Variable::new("result");
    let addr = Address::concrete(100);
    let post = SepProp::pure(Formula::Eq(
        Box::new(SmtExpr::var("result")),
        Box::new(SmtExpr::IntConst(42)),
    ));

    let cmd = HeapCommand::Load(var, addr);
    let wp = wp_heap(&cmd, post);

    // wp(v := *x, Q) should be existentially quantified
    match wp {
        SepProp::Exists(_, _) => (),
        _ => panic!("Expected Exists for load wp"),
    }
}

// =============================================================================
// Frame Rule Tests
// =============================================================================

#[test]
fn test_frame_rule_disjoint() {
    let modified = vec![Address::concrete(100)].into_iter().collect();
    let frame = SepProp::points_to(Address::concrete(200), Value::int(1));

    // Frame doesn't overlap with modified locations
    let can_frame = FrameRule::can_frame(&SepProp::emp(), &SepProp::emp(), &frame, &modified);

    assert!(can_frame);
}

#[test]
fn test_frame_rule_overlapping() {
    let modified = vec![Address::concrete(100)].into_iter().collect();
    let frame = SepProp::points_to(Address::concrete(100), Value::int(1));

    // Frame overlaps with modified locations
    let can_frame = FrameRule::can_frame(&SepProp::emp(), &SepProp::emp(), &frame, &modified);

    // This should fail because addresses overlap
    // Note: Current implementation may be conservative
    assert!(!can_frame);
}

#[test]
fn test_apply_frame() {
    let pre = SepProp::points_to(Address::concrete(100), Value::int(1));
    let post = SepProp::points_to(Address::concrete(100), Value::int(2));
    let frame = SepProp::points_to(Address::concrete(200), Value::int(99));

    let (framed_pre, framed_post) = FrameRule::frame(pre.clone(), post.clone(), frame.clone());

    // Both should be separating conjunctions
    match (framed_pre, framed_post) {
        (SepProp::SeparatingConj(_, _), SepProp::SeparatingConj(_, _)) => (),
        _ => panic!("Frame application failed"),
    }
}

// =============================================================================
// Heap Model Tests
// =============================================================================

#[test]
fn test_heap_creation() {
    let heap = Heap::fresh("h");
    assert_eq!(heap.array.name.as_str(), "h");
}

#[test]
fn test_heap_select() {
    let heap = Heap::fresh("heap");
    let addr = Address::concrete(100);

    let expr = heap.select(&addr);

    match expr {
        SmtExpr::Select(_, _) => (),
        _ => panic!("Expected Select expression"),
    }
}

#[test]
fn test_heap_store() {
    let heap = Heap::fresh("h1");
    let addr = Address::concrete(100);
    let val = Value::int(42);

    let new_heap = heap.store(&addr, &val);

    // Should create a new heap variable
    assert_ne!(heap.array.name, new_heap.array.name);
}

// =============================================================================
// Z3 Encoding Tests
// =============================================================================

#[test]
fn test_encode_emp() {
    let encoder = SepLogicEncoder::new();
    let heap = Heap::fresh("h");
    let emp = SepProp::Emp;

    let formula = encoder.encode(&emp, &heap);

    // Should encode as "domain is empty"
    match formula {
        Formula::Predicate(name, _) if name.as_str() == "is_empty" => (),
        _ => panic!("Expected is_empty predicate for emp encoding"),
    }
}

#[test]
fn test_encode_points_to() {
    let encoder = SepLogicEncoder::new();
    let heap = Heap::fresh("h");
    let addr = Address::concrete(100);
    let val = Value::int(42);
    let prop = SepProp::points_to(addr, val);

    let formula = encoder.encode(&prop, &heap);

    // Should be a conjunction of domain constraint and value constraint
    match formula {
        Formula::And(_) => (),
        Formula::Predicate(_, _) => (), // Simplified encoding
        _ => panic!("Expected And or Predicate for points-to encoding"),
    }
}

#[test]
fn test_encode_separating_conj() {
    let encoder = SepLogicEncoder::new();
    let heap = Heap::fresh("h");
    let p = SepProp::points_to(Address::concrete(100), Value::int(1));
    let q = SepProp::points_to(Address::concrete(200), Value::int(2));
    let star = SepProp::sep_conj(p, q);

    let formula = encoder.encode(&star, &heap);

    // Should encode disjointness and union constraints
    match formula {
        Formula::And(_) => (),
        _ => panic!("Expected And for separating conjunction encoding"),
    }
}

// =============================================================================
// CBGR Integration Tests
// =============================================================================

#[test]
fn test_cbgr_alloc() {
    let addr = Address::concrete(0x1000);
    let val = Value::int(42);
    let generation = 1;
    let epoch = 0;

    let prop = CbgrSepLogic::cbgr_alloc(addr.clone(), val, generation, epoch);

    // Should be sep conj of points-to and cbgr_object
    match prop {
        SepProp::SeparatingConj(_, _) => (),
        _ => panic!("Expected SeparatingConj for CBGR allocation"),
    }
}

#[test]
fn test_cbgr_free() {
    let addr = Address::concrete(0x1000);
    let generation = 5;
    let epoch = 0;

    let prop = CbgrSepLogic::cbgr_free(addr, generation, epoch);

    // Should consume the points-to assertion
    match prop {
        SepProp::SeparatingConj(_, _) => (),
        _ => panic!("Expected SeparatingConj for CBGR free"),
    }
}

#[test]
fn test_cbgr_validate() {
    let addr = Address::concrete(0x1000);
    let expected_gen = 3;
    let expected_epoch = 0;

    let formula = CbgrSepLogic::cbgr_validate(addr, expected_gen, expected_epoch);

    match formula {
        Formula::Predicate(name, args) => {
            assert_eq!(name.as_str(), "cbgr_valid");
            assert_eq!(args.len(), 3);
        }
        _ => panic!("Expected Predicate for CBGR validation"),
    }
}

// =============================================================================
// Symbolic Execution Tests
// =============================================================================

#[test]
fn test_symbolic_state_creation() {
    let state = SymbolicState::new();

    match state.spatial {
        SepProp::Emp => (),
        _ => panic!("Initial state should have empty heap"),
    }

    match state.pure {
        Formula::True => (),
        _ => panic!("Initial state should have true path condition"),
    }
}

#[test]
fn test_symbolic_state_assume() {
    let mut state = SymbolicState::new();
    let constraint = Formula::Eq(Box::new(SmtExpr::var("x")), Box::new(SmtExpr::IntConst(10)));

    state.assume(constraint.clone());

    // Pure should now be conjunction with constraint
    match state.pure {
        Formula::And(_) => (),
        Formula::Eq(_, _) => (), // Optimized to single constraint
        _ => panic!("Assume should update path condition"),
    }
}

#[test]
fn test_symbolic_state_alloc() {
    let mut state = SymbolicState::new();
    let addr = Address::concrete(100);
    let val = Value::int(42);

    state.alloc(addr.clone(), val.clone());

    // Spatial should now have points-to assertion
    match state.spatial {
        SepProp::PointsTo(_, _) | SepProp::SeparatingConj(_, _) => (),
        _ => panic!("Alloc should add points-to to spatial state"),
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_verify_simple_alloc_free() {
    let var = Variable::new("x");
    let val = Value::int(42);
    let addr = Address::symbolic("x");

    // Pre: emp
    let _pre = SepProp::emp();

    // Command: x := alloc(42); free(x)
    let alloc_cmd = HeapCommand::Alloc(var.clone(), val.clone());
    let free_cmd = HeapCommand::Free(addr.clone());

    // Post: emp
    let post = SepProp::emp();

    // Compute wp backwards
    let wp_free = wp_heap(&free_cmd, post);
    let wp_both = wp_heap(&alloc_cmd, wp_free);

    // Check if emp ⊢ wp
    // (This would normally go to Z3, for now we just check it's well-formed)
    match wp_both {
        SepProp::Exists(_, _) => (),
        _ => panic!("WP should be existentially quantified"),
    }
}

#[test]
fn test_list_swap_verification() {
    // Verify: swap first two elements of a list
    // Pre: list(head, [a, b, ...rest])
    // Post: list(head, [b, a, ...rest])

    let head = Address::symbolic("head");
    let a = Value::int(1);
    let b = Value::int(2);

    // Simplified version: just verify points-to updates
    let pre = SepProp::sep_conj(
        SepProp::points_to(head.clone(), a.clone()),
        SepProp::points_to(head.offset(1), b.clone()),
    );

    let post = SepProp::sep_conj(
        SepProp::points_to(head.clone(), b),
        SepProp::points_to(head.offset(1), a),
    );

    // Commands would be load/store operations
    // For now, just verify the specs are well-formed
    assert_ne!(pre, post); // Pre and post are different (as expected)
}

#[test]
fn test_generate_heap_vcs() {
    let pre = SepProp::emp();
    let post = SepProp::emp();
    let cmds: List<HeapCommand> = vec![
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        HeapCommand::Free(Address::symbolic("x")),
    ]
    .into();

    let vcs = generate_heap_vcs(&pre, &cmds, &post);

    // Should generate at least one VC
    assert!(!vcs.is_empty());

    // VC should be a formula
    for (formula, _loc) in vcs.iter() {
        match formula {
            Formula::Implies(_, _) => (),
            _ => panic!("VC should be an implication"),
        }
    }
}
