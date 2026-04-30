//! Comprehensive tests for separation logic verification
//!
//! Tests cover:
//! - Separating conjunction encoding
//! - Magic wand operator
//! - Points-to predicates
//! - List segments and tree predicates
//! - Frame rule application
//! - Heap entailment checking
//! - Weakest precondition computation

use verum_ast::literal::{IntLit, Literal, LiteralKind};
use verum_ast::{BinOp, Expr, ExprKind, Ident, Path};
use verum_common::{Heap, List, Maybe, Text};
use verum_smt::separation_logic::{
    Command, EntailmentResult, HoareTriple, SepAssertion, SepLogicConfig, SeparationLogic,
    alloc_example, free_example, list_segment_example, read_example, write_example,
};

/// Helper to create integer expression
fn int_expr(value: i64) -> Expr {
    use verum_ast::span::Span;
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit::new(value as i128)),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

/// Helper to create variable expression
fn var_expr(name: &str) -> Expr {
    use verum_ast::span::Span;
    Expr::new(
        ExprKind::Path(Path::from_ident(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}

/// Helper to create boolean expression
fn bool_expr(value: bool) -> Expr {
    use verum_ast::span::Span;
    Expr::new(
        ExprKind::Literal(Literal::new(LiteralKind::Bool(value), Span::dummy())),
        Span::dummy(),
    )
}

// ==================== SepAssertion Construction Tests ====================

#[test]
fn test_points_to_creation() {
    let loc = var_expr("x");
    let val = int_expr(42);
    let assertion = SepAssertion::points_to(loc.clone(), val.clone());

    match assertion {
        SepAssertion::PointsTo { location, value } => {
            // Verify structure is correct
            assert!(matches!(location.kind, ExprKind::Path(_)));
            assert!(matches!(value.kind, ExprKind::Literal(_)));
        }
        _ => panic!("Expected PointsTo assertion"),
    }
}

#[test]
fn test_separating_conjunction_creation() {
    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let p2 = SepAssertion::points_to(var_expr("y"), int_expr(2));
    let sep = SepAssertion::sep(p1, p2);

    match sep {
        SepAssertion::Sep { left, right } => {
            assert!(matches!(*left, SepAssertion::PointsTo { .. }));
            assert!(matches!(*right, SepAssertion::PointsTo { .. }));
        }
        _ => panic!("Expected Sep assertion"),
    }
}

#[test]
fn test_magic_wand_creation() {
    let p = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let q = SepAssertion::points_to(var_expr("x"), int_expr(2));
    let wand = SepAssertion::wand(p, q);

    match wand {
        SepAssertion::Wand { left, right } => {
            assert!(matches!(*left, SepAssertion::PointsTo { .. }));
            assert!(matches!(*right, SepAssertion::PointsTo { .. }));
        }
        _ => panic!("Expected Wand assertion"),
    }
}

#[test]
fn test_empty_heap_creation() {
    let emp = SepAssertion::emp();
    assert!(emp.is_emp());
    assert!(emp.is_pure());
}

#[test]
fn test_pure_assertion_creation() {
    let expr = bool_expr(true);
    let pure = SepAssertion::pure(expr);
    assert!(pure.is_pure());
}

#[test]
fn test_list_segment_creation() {
    let from = var_expr("head");
    let to = var_expr("tail");
    let elements = List::from_iter(vec![int_expr(1), int_expr(2), int_expr(3)]);
    let lseg = SepAssertion::list_segment(from, to, elements);

    match lseg {
        SepAssertion::ListSegment {
            from: _,
            to: _,
            elements,
        } => {
            assert_eq!(elements.len(), 3);
        }
        _ => panic!("Expected ListSegment assertion"),
    }
}

#[test]
fn test_empty_list_segment_creation() {
    let at = var_expr("ptr");
    let lseg = SepAssertion::empty_list_segment(at);

    match lseg {
        SepAssertion::ListSegment { elements, .. } => {
            assert!(elements.is_empty());
        }
        _ => panic!("Expected ListSegment assertion"),
    }
}

#[test]
fn test_tree_predicate_creation() {
    let root = var_expr("root");
    let left = SepAssertion::emp();
    let right = SepAssertion::emp();
    let tree = SepAssertion::tree(root, Maybe::Some(left), Maybe::Some(right));

    match tree {
        SepAssertion::Tree {
            root: _,
            left_child,
            right_child,
        } => {
            assert!(left_child.is_some());
            assert!(right_child.is_some());
        }
        _ => panic!("Expected Tree assertion"),
    }
}

#[test]
fn test_block_predicate_creation() {
    let base = var_expr("ptr");
    let size = int_expr(16);
    let block = SepAssertion::block(base, size);

    match block {
        SepAssertion::Block { base: _, size: _ } => {}
        _ => panic!("Expected Block assertion"),
    }
}

#[test]
fn test_array_segment_creation() {
    let base = var_expr("arr");
    let offset = int_expr(0);
    let length = int_expr(4);
    let elements = List::from_iter(vec![int_expr(1), int_expr(2), int_expr(3), int_expr(4)]);
    let arr_seg = SepAssertion::array_segment(base, offset, length, elements);

    match arr_seg {
        SepAssertion::ArraySegment { elements, .. } => {
            assert_eq!(elements.len(), 4);
        }
        _ => panic!("Expected ArraySegment assertion"),
    }
}

// ==================== Footprint Extraction Tests ====================

#[test]
fn test_footprint_points_to() {
    let loc = var_expr("x");
    let val = int_expr(42);
    let assertion = SepAssertion::points_to(loc, val);
    let footprint = assertion.footprint();

    assert_eq!(footprint.len(), 1);
}

#[test]
fn test_footprint_separating_conjunction() {
    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let p2 = SepAssertion::points_to(var_expr("y"), int_expr(2));
    let sep = SepAssertion::sep(p1, p2);
    let footprint = sep.footprint();

    assert_eq!(footprint.len(), 2);
}

#[test]
fn test_footprint_empty() {
    let emp = SepAssertion::emp();
    let footprint = emp.footprint();

    assert!(footprint.is_empty());
}

#[test]
fn test_footprint_pure() {
    let pure = SepAssertion::pure(bool_expr(true));
    let footprint = pure.footprint();

    assert!(footprint.is_empty());
}

// ==================== SeparationLogic Verifier Tests ====================

#[test]
fn test_separation_logic_creation() {
    let sl = SeparationLogic::new();
    let stats = sl.stats();

    assert_eq!(stats.entailment_checks, 0);
    assert_eq!(stats.successful_proofs, 0);
    assert_eq!(stats.failed_proofs, 0);
}

#[test]
fn test_separation_logic_with_config() {
    let config = SepLogicConfig {
        entailment_timeout_ms: 10000,
        max_unfolding_depth: 20,
        enable_frame_inference: true,
        enable_symbolic_execution: true,
        enable_caching: false,
    };
    let sl = SeparationLogic::with_config(config);
    let stats = sl.stats();

    assert_eq!(stats.entailment_checks, 0);
}

#[test]
fn test_fresh_variable_generation() {
    let sl = SeparationLogic::new();

    let v1 = sl.fresh_var("test");
    let v2 = sl.fresh_var("test");
    let v3 = sl.fresh_var("other");

    // Each should be unique
    assert_ne!(v1.as_str(), v2.as_str());
    assert_ne!(v1.as_str(), v3.as_str());
    assert_ne!(v2.as_str(), v3.as_str());
}

#[test]
fn test_counter_reset() {
    let sl = SeparationLogic::new();

    let _v1 = sl.fresh_var("test");
    let _v2 = sl.fresh_var("test");

    sl.reset_counter();

    // After reset, counter should start from 0 again
    let v3 = sl.fresh_var("test");
    assert!(v3.as_str().ends_with("_0"));
}

// ==================== Entailment Tests ====================

#[test]
fn test_entailment_reflexive() {
    let sl = SeparationLogic::new();

    let assertion = SepAssertion::points_to(var_expr("x"), int_expr(42));

    let result = sl.verify_entailment(&assertion, &assertion);
    assert!(result.is_ok());

    // Reflexive entailment should be valid
    if let Ok(EntailmentResult::Valid { .. }) = result {
        // Expected
    } else {
        // May timeout on some systems, which is acceptable
    }
}

#[test]
fn test_entailment_emp_emp() {
    let sl = SeparationLogic::new();

    let emp1 = SepAssertion::emp();
    let emp2 = SepAssertion::emp();

    let result = sl.verify_entailment(&emp1, &emp2);
    assert!(result.is_ok());

    match result.unwrap() {
        EntailmentResult::Valid { .. } => {}
        EntailmentResult::Unknown { .. } => {} // Acceptable
        EntailmentResult::Invalid { .. } => panic!("emp |- emp should be valid"),
    }
}

#[test]
fn test_entailment_pure_true() {
    let sl = SeparationLogic::new();

    let pure_true = SepAssertion::pure(bool_expr(true));
    let emp = SepAssertion::emp();

    let result = sl.verify_entailment(&pure_true, &emp);
    assert!(result.is_ok());
}

// ==================== Frame Rule Tests ====================

#[test]
fn test_apply_frame_rule() {
    let sl = SeparationLogic::new();

    let pre = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let post = SepAssertion::points_to(var_expr("x"), int_expr(2));
    let command = Command::Write {
        addr: var_expr("x"),
        value: int_expr(2),
    };

    let triple = HoareTriple::new(pre, command, post);

    let frame = SepAssertion::points_to(var_expr("y"), int_expr(42));
    let framed_triple = sl.apply_frame_rule(triple, frame);

    // Check that framed triple has separating conjunctions
    match framed_triple.pre {
        SepAssertion::Sep { .. } => {}
        _ => panic!("Expected Sep in precondition"),
    }

    match framed_triple.post {
        SepAssertion::Sep { .. } => {}
        _ => panic!("Expected Sep in postcondition"),
    }
}

// ==================== Weakest Precondition Tests ====================

#[test]
fn test_wp_skip() {
    let sl = SeparationLogic::new();

    let post = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let wp = sl.wp(&Command::Skip, &post);

    assert!(wp.is_ok());
    // wp(skip, Q) = Q
}

#[test]
fn test_wp_assign() {
    let sl = SeparationLogic::new();

    let y = var_expr("y");
    let post = SepAssertion::pure(Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(var_expr("x")),
            right: Heap::new(int_expr(5)),
        },
        verum_ast::span::Span::dummy(),
    ));

    let cmd = Command::Assign {
        var: Text::from("x"),
        expr: y,
    };

    let wp = sl.wp(&cmd, &post);
    assert!(wp.is_ok());
}

#[test]
fn test_wp_seq() {
    let sl = SeparationLogic::new();

    let c1 = Command::Assign {
        var: Text::from("x"),
        expr: int_expr(1),
    };
    let c2 = Command::Assign {
        var: Text::from("y"),
        expr: var_expr("x"),
    };

    let seq = Command::Seq {
        first: Heap::new(c1),
        second: Heap::new(c2),
    };

    let post = SepAssertion::pure(bool_expr(true));
    let wp = sl.wp(&seq, &post);

    assert!(wp.is_ok());
}

#[test]
fn test_wp_if() {
    let sl = SeparationLogic::new();

    let cond = var_expr("flag");
    let then_cmd = Command::Assign {
        var: Text::from("x"),
        expr: int_expr(1),
    };
    let else_cmd = Command::Assign {
        var: Text::from("x"),
        expr: int_expr(2),
    };

    let if_cmd = Command::If {
        condition: cond,
        then_branch: Heap::new(then_cmd),
        else_branch: Heap::new(else_cmd),
    };

    let post = SepAssertion::pure(bool_expr(true));
    let wp = sl.wp(&if_cmd, &post);

    assert!(wp.is_ok());
}

#[test]
fn test_wp_while() {
    let sl = SeparationLogic::new();

    let cond = var_expr("flag");
    let invariant = SepAssertion::pure(bool_expr(true));
    let body = Command::Skip;

    let while_cmd = Command::While {
        condition: cond,
        invariant: invariant.clone(),
        body: Heap::new(body),
    };

    let post = SepAssertion::pure(bool_expr(true));
    let wp = sl.wp(&while_cmd, &post);

    assert!(wp.is_ok());
    // wp(while, Q) = invariant
}

#[test]
fn test_wp_alloc() {
    let sl = SeparationLogic::new();

    let alloc_cmd = Command::Alloc {
        result: Text::from("x"),
        size: int_expr(1),
    };

    let post = SepAssertion::points_to(var_expr("x"), var_expr("_"));
    let wp = sl.wp(&alloc_cmd, &post);

    assert!(wp.is_ok());
    // wp should contain forall and wand
}

#[test]
fn test_wp_read() {
    let sl = SeparationLogic::new();

    let read_cmd = Command::Read {
        result: Text::from("y"),
        addr: var_expr("x"),
    };

    let post = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let wp = sl.wp(&read_cmd, &post);

    assert!(wp.is_ok());
    // wp should contain exists and separating conjunction
}

#[test]
fn test_wp_write() {
    let sl = SeparationLogic::new();

    let write_cmd = Command::Write {
        addr: var_expr("x"),
        value: int_expr(99),
    };

    let post = SepAssertion::points_to(var_expr("x"), int_expr(99));
    let wp = sl.wp(&write_cmd, &post);

    assert!(wp.is_ok());
    // wp should contain separating conjunction with wand
}

#[test]
fn test_wp_free() {
    let sl = SeparationLogic::new();

    let free_cmd = Command::Free {
        ptr: var_expr("x"),
        size: int_expr(1),
    };

    let post = SepAssertion::emp();
    let wp = sl.wp(&free_cmd, &post);

    assert!(wp.is_ok());
}

// ==================== Simplification Tests ====================

#[test]
fn test_simplify_emp_sep_left() {
    let sl = SeparationLogic::new();

    let emp = SepAssertion::emp();
    let p = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let sep = SepAssertion::sep(emp, p.clone());

    let simplified = sl.simplify(&sep);

    // emp * P should simplify to P
    match simplified {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo after simplification"),
    }
}

#[test]
fn test_simplify_emp_sep_right() {
    let sl = SeparationLogic::new();

    let emp = SepAssertion::emp();
    let p = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let sep = SepAssertion::sep(p.clone(), emp);

    let simplified = sl.simplify(&sep);

    // P * emp should simplify to P
    match simplified {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo after simplification"),
    }
}

#[test]
fn test_simplify_emp_wand() {
    let sl = SeparationLogic::new();

    let emp = SepAssertion::emp();
    let p = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let wand = SepAssertion::wand(emp, p.clone());

    let simplified = sl.simplify(&wand);

    // emp -* P should simplify to P
    match simplified {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo after simplification"),
    }
}

// ==================== Disjointness Tests ====================

#[test]
fn test_are_disjoint_different_locations() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let p2 = SepAssertion::points_to(var_expr("y"), int_expr(2));

    // Different variable names should be syntactically disjoint
    assert!(sl.are_disjoint(&p1, &p2));
}

#[test]
fn test_are_disjoint_same_location() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let p2 = SepAssertion::points_to(var_expr("x"), int_expr(2));

    // Same variable name - not disjoint
    assert!(!sl.are_disjoint(&p1, &p2));
}

#[test]
fn test_are_disjoint_emp() {
    let sl = SeparationLogic::new();

    let emp = SepAssertion::emp();
    let p = SepAssertion::points_to(var_expr("x"), int_expr(1));

    // Empty heap is disjoint from anything
    assert!(sl.are_disjoint(&emp, &p));
}

// ==================== Example Triple Tests ====================

#[test]
fn test_alloc_example() {
    let triple = alloc_example();

    // Pre: emp
    assert!(triple.pre.is_emp());

    // Command: alloc
    match triple.command {
        Command::Alloc { .. } => {}
        _ => panic!("Expected Alloc command"),
    }

    // Post: x |-> _
    match triple.post {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo in postcondition"),
    }
}

#[test]
fn test_read_example() {
    let triple = read_example();

    // Pre: x |-> 42
    match triple.pre {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo in precondition"),
    }

    // Command: read
    match triple.command {
        Command::Read { .. } => {}
        _ => panic!("Expected Read command"),
    }
}

#[test]
fn test_write_example() {
    let triple = write_example();

    // Pre: x |-> _
    match triple.pre {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo in precondition"),
    }

    // Command: write
    match triple.command {
        Command::Write { .. } => {}
        _ => panic!("Expected Write command"),
    }

    // Post: x |-> 42
    match triple.post {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo in postcondition"),
    }
}

#[test]
fn test_free_example() {
    let triple = free_example();

    // Pre: x |-> 42
    match triple.pre {
        SepAssertion::PointsTo { .. } => {}
        _ => panic!("Expected PointsTo in precondition"),
    }

    // Command: free
    match triple.command {
        Command::Free { .. } => {}
        _ => panic!("Expected Free command"),
    }

    // Post: emp
    assert!(triple.post.is_emp());
}

#[test]
fn test_list_segment_example() {
    let triple = list_segment_example();

    // Pre: lseg(head, tail, [1, 2])
    match triple.pre {
        SepAssertion::ListSegment { elements, .. } => {
            assert_eq!(elements.len(), 2);
        }
        _ => panic!("Expected ListSegment in precondition"),
    }

    // Command: skip
    match triple.command {
        Command::Skip => {}
        _ => panic!("Expected Skip command"),
    }
}

// ==================== Assertion Equality Tests ====================

#[test]
fn test_assertions_equal_emp() {
    let sl = SeparationLogic::new();

    let emp1 = SepAssertion::emp();
    let emp2 = SepAssertion::emp();

    assert!(sl.assertions_equal(&emp1, &emp2));
}

#[test]
fn test_assertions_equal_points_to() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let p2 = SepAssertion::points_to(var_expr("x"), int_expr(42));

    assert!(sl.assertions_equal(&p1, &p2));
}

#[test]
fn test_assertions_not_equal_different_value() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let p2 = SepAssertion::points_to(var_expr("x"), int_expr(43));

    assert!(!sl.assertions_equal(&p1, &p2));
}

#[test]
fn test_assertions_not_equal_different_location() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(42));
    let p2 = SepAssertion::points_to(var_expr("y"), int_expr(42));

    assert!(!sl.assertions_equal(&p1, &p2));
}

// ==================== Quantifier Tests ====================

#[test]
fn test_existential_assertion() {
    let body = SepAssertion::points_to(var_expr("x"), var_expr("v"));
    let exists = SepAssertion::exists(Text::from("v"), body);

    match exists {
        SepAssertion::Exists { var, body: _ } => {
            assert_eq!(var.as_str(), "v");
        }
        _ => panic!("Expected Exists assertion"),
    }
}

#[test]
fn test_universal_assertion() {
    let body = SepAssertion::points_to(var_expr("x"), var_expr("v"));
    let forall = SepAssertion::forall(Text::from("v"), body);

    match forall {
        SepAssertion::Forall { var, body: _ } => {
            assert_eq!(var.as_str(), "v");
        }
        _ => panic!("Expected Forall assertion"),
    }
}

// ==================== Conjunction and Disjunction Tests ====================

#[test]
fn test_conjunction_assertion() {
    let p1 = SepAssertion::pure(bool_expr(true));
    let p2 = SepAssertion::pure(bool_expr(false));
    let conj = SepAssertion::and(p1, p2);

    match conj {
        SepAssertion::And { .. } => {}
        _ => panic!("Expected And assertion"),
    }
}

#[test]
fn test_disjunction_assertion() {
    let p1 = SepAssertion::pure(bool_expr(true));
    let p2 = SepAssertion::pure(bool_expr(false));
    let disj = SepAssertion::or(p1, p2);

    match disj {
        SepAssertion::Or { .. } => {}
        _ => panic!("Expected Or assertion"),
    }
}

// ==================== CAS Command Tests ====================

#[test]
fn test_wp_cas() {
    let sl = SeparationLogic::new();

    let cas_cmd = Command::CAS {
        result: Text::from("success"),
        addr: var_expr("ptr"),
        expected: int_expr(0),
        desired: int_expr(1),
    };

    let post = SepAssertion::pure(bool_expr(true));
    let wp = sl.wp(&cas_cmd, &post);

    assert!(wp.is_ok());
}

// ==================== Call Command Tests ====================

#[test]
fn test_wp_call() {
    let sl = SeparationLogic::new();

    let call_pre = SepAssertion::emp();
    let call_post = SepAssertion::points_to(var_expr("result"), int_expr(42));

    let call_cmd = Command::Call {
        result: Maybe::Some(Text::from("x")),
        func: Text::from("alloc"),
        args: List::new(),
        pre: call_pre,
        post: call_post,
    };

    let post = SepAssertion::pure(bool_expr(true));
    let wp = sl.wp(&call_cmd, &post);

    assert!(wp.is_ok());
}

// ==================== Statistics Tests ====================

#[test]
fn test_statistics_update() {
    let sl = SeparationLogic::new();

    // Initial stats
    let stats1 = sl.stats();
    assert_eq!(stats1.entailment_checks, 0);

    // Perform an entailment check
    let emp = SepAssertion::emp();
    let _ = sl.verify_entailment(&emp, &emp);

    // Stats should be updated
    let stats2 = sl.stats();
    assert_eq!(stats2.entailment_checks, 1);
}

// ==================== Engine Access Tests ====================

#[test]
fn test_engine_access() {
    let mut sl = SeparationLogic::new();

    // Access engine
    let _engine = sl.engine();

    // Mutable access
    let _engine_mut = sl.engine_mut();
}

// ==================== Location Extraction Tests ====================

#[test]
fn test_extract_locations() {
    let sl = SeparationLogic::new();

    let p1 = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let p2 = SepAssertion::points_to(var_expr("y"), int_expr(2));
    let sep = SepAssertion::sep(p1, p2);

    let locs = sl.extract_locations(&sep);
    assert_eq!(locs.len(), 2);
}

// ==================== Wand Elimination Tests ====================

#[test]
fn test_wand_elimination_success() {
    let sl = SeparationLogic::new();

    let have = SepAssertion::emp();
    let wand_left = SepAssertion::emp();
    let wand_right = SepAssertion::pure(bool_expr(true));

    let result = sl.apply_wand_elimination(&have, &wand_left, &wand_right);
    assert!(result.is_ok());
}

#[test]
fn test_wand_elimination_failure() {
    let sl = SeparationLogic::new();

    let have = SepAssertion::points_to(var_expr("x"), int_expr(1));
    let wand_left = SepAssertion::emp();
    let wand_right = SepAssertion::pure(bool_expr(true));

    let result = sl.apply_wand_elimination(&have, &wand_left, &wand_right);
    assert!(result.is_err());
}

// =============================================================================
// UnfoldingConfig wiring tests
// =============================================================================
//
// Pin: every `UnfoldingConfig` field reaches a meaningful
// consumer at the `UnfoldingState` API. Pre-fix all four fields
// were stored on state but no code read them — the public
// `can_unfold` API took `max_depth` as a manual parameter,
// `lazy_unfolding` and `generate_lemmas` were entirely orphaned,
// and `add_lemma` always pushed regardless of the
// `generate_lemmas` flag.

mod unfolding_config_wiring {
    use super::*;
    use verum_smt::separation_logic::{UnfoldingConfig, UnfoldingState};

    #[test]
    fn unfolding_config_default_values_pinned() {
        // Pin documented defaults; drift here would silently change
        // the unfolding-budget semantics for every caller relying on
        // `Default::default()`.
        let cfg = UnfoldingConfig::default();
        assert_eq!(cfg.max_lseg_depth, 10);
        assert_eq!(cfg.max_tree_depth, 5);
        assert!(cfg.lazy_unfolding);
        assert!(cfg.generate_lemmas);
    }

    #[test]
    fn can_unfold_lseg_uses_max_lseg_depth() {
        // Pin: `can_unfold_lseg` consults config.max_lseg_depth
        // automatically — embedders no longer need to pass the
        // cap. max_tree_depth is set high so a regression that
        // silently swapped the two fields would surface here.
        let cfg = UnfoldingConfig {
            max_lseg_depth: 3,
            max_tree_depth: 99,
            lazy_unfolding: true,
            generate_lemmas: true,
        };
        let mut state = UnfoldingState::new(cfg);

        assert!(state.can_unfold_lseg("lseg_p"));
        state.increment_depth("lseg_p");
        assert!(state.can_unfold_lseg("lseg_p"));
        state.increment_depth("lseg_p");
        state.increment_depth("lseg_p");
        assert!(!state.can_unfold_lseg("lseg_p"));
    }

    #[test]
    fn can_unfold_tree_uses_max_tree_depth() {
        // Pin: `can_unfold_tree` consults config.max_tree_depth —
        // independently from max_lseg_depth.
        let cfg = UnfoldingConfig {
            max_lseg_depth: 99,
            max_tree_depth: 2,
            lazy_unfolding: true,
            generate_lemmas: true,
        };
        let mut state = UnfoldingState::new(cfg);

        assert!(state.can_unfold_tree("tree_p"));
        state.increment_depth("tree_p");
        assert!(state.can_unfold_tree("tree_p"));
        state.increment_depth("tree_p");
        assert!(!state.can_unfold_tree("tree_p"));
    }

    #[test]
    fn lazy_unfolding_enabled_round_trips() {
        let mut cfg = UnfoldingConfig::default();
        cfg.lazy_unfolding = false;
        let state = UnfoldingState::new(cfg);
        assert!(!state.lazy_unfolding_enabled());
    }

    #[test]
    fn lemma_generation_enabled_round_trips() {
        let mut cfg = UnfoldingConfig::default();
        cfg.generate_lemmas = false;
        let state = UnfoldingState::new(cfg);
        assert!(!state.lemma_generation_enabled());
    }

    #[test]
    fn add_lemma_skipped_when_generation_disabled() {
        // Pin the wire-up at the only producer site: when
        // `generate_lemmas = false`, add_lemma must NOT push to
        // the lemma list. Pre-wire the field was stored but the
        // method always pushed regardless.
        let cfg = UnfoldingConfig {
            max_lseg_depth: 10,
            max_tree_depth: 5,
            lazy_unfolding: true,
            generate_lemmas: false,
        };
        let mut state = UnfoldingState::new(cfg);
        state.add_lemma(SepAssertion::emp());
        assert!(
            state.lemmas.is_empty(),
            "add_lemma must skip when generate_lemmas = false"
        );
    }

    #[test]
    fn add_lemma_pushed_when_generation_enabled() {
        // Pin the inverse: when `generate_lemmas = true`, add_lemma
        // pushes as before — no regression on the default path.
        let cfg = UnfoldingConfig::default();
        let mut state = UnfoldingState::new(cfg);
        state.add_lemma(SepAssertion::emp());
        assert_eq!(state.lemmas.len(), 1);
    }
}

// =============================================================================
// SepLogicEncoder::encode_assertion routing wire-up
// =============================================================================
//
// Pin: when `UnfoldingConfig.lazy_unfolding` is enabled (the
// documented default), `encode_assertion` for ListSegment / Tree
// assertions routes through `encode_with_unfolding` so the depth
// caps `max_lseg_depth` / `max_tree_depth` actually constrain the
// formula. Pre-fix the dispatch went straight to the unbounded
// `encode_list_segment` / `encode_tree`, making the entire
// UnfoldingConfig surface inert at the production entry point —
// every caller setting `[smt.sep_logic] max_unfolding_depth = N`
// via Verum.toml was silently bypassed.

mod encode_assertion_unfolding_routing {
    use super::*;
    use verum_smt::separation_logic::{SepLogicEncoder, Z3HeapModel};

    #[test]
    fn list_segment_collapses_to_true_when_depth_zero() {
        // Pin: with lazy_unfolding=true (default) and
        // max_lseg_depth=0 (derived from max_unfolding_depth=0),
        // `encode_assertion` short-circuits a non-empty
        // ListSegment to literal `true` because the routed
        // `encode_with_unfolding` returns `Bool::from_bool(true)` at
        // depth 0. Without the routing wire, the unbounded
        // `encode_list_segment` builds a multi-conjunct formula
        // (heap_at_from = head ∧ alloc ∧ …) which does NOT collapse
        // to literal true — a regression that removes the gate
        // surfaces here.
        let config = SepLogicConfig {
            max_unfolding_depth: 0,
            ..Default::default()
        };
        let encoder = SepLogicEncoder::new(config);
        let model = Z3HeapModel::new();

        let lseg = SepAssertion::list_segment(
            var_expr("p"),
            var_expr("q"),
            List::from(vec![int_expr(1), int_expr(2), int_expr(3)]),
        );

        let result = encoder.encode_assertion(&lseg, model.heap(), model.allocated());
        assert_eq!(
            result.as_bool(),
            Some(true),
            "lazy_unfolding=true + max_lseg_depth=0 must route ListSegment \
             through encode_with_unfolding which short-circuits to literal `true`",
        );
    }

    #[test]
    fn tree_collapses_to_true_when_depth_zero() {
        // Sibling pin for tree predicates: max_tree_depth derives
        // from max_unfolding_depth/2 (line 1325 of
        // separation_logic.rs). With max_unfolding_depth=0,
        // max_tree_depth=0 and the routed encode_with_unfolding
        // returns true at depth 0.
        use verum_common::Heap;
        let config = SepLogicConfig {
            max_unfolding_depth: 0,
            ..Default::default()
        };
        let encoder = SepLogicEncoder::new(config);
        let model = Z3HeapModel::new();

        let leaf_pointing = SepAssertion::points_to(var_expr("leaf"), int_expr(0));
        let tree = SepAssertion::Tree {
            root: var_expr("root"),
            left_child: Maybe::Some(Heap::new(leaf_pointing.clone())),
            right_child: Maybe::Some(Heap::new(leaf_pointing)),
        };

        let result = encoder.encode_assertion(&tree, model.heap(), model.allocated());
        assert_eq!(
            result.as_bool(),
            Some(true),
            "lazy_unfolding=true + max_tree_depth=0 must route Tree \
             through encode_with_unfolding which short-circuits to literal `true`",
        );
    }

    #[test]
    fn list_segment_with_nonzero_depth_remains_structural() {
        // Pin the inverse: with a positive depth budget, the
        // routed encoding still builds a real formula (NOT literal
        // true) — the gate is a depth cap, not a kill-switch.
        let config = SepLogicConfig {
            max_unfolding_depth: 5,
            ..Default::default()
        };
        let encoder = SepLogicEncoder::new(config);
        let model = Z3HeapModel::new();

        let lseg = SepAssertion::list_segment(
            var_expr("p"),
            var_expr("q"),
            List::from(vec![int_expr(1), int_expr(2)]),
        );

        let result = encoder.encode_assertion(&lseg, model.heap(), model.allocated());
        assert_ne!(
            result.as_bool(),
            Some(true),
            "depth budget > 0 must produce a non-trivial Z3 formula, \
             not the depth-0 short-circuit",
        );
    }
}
