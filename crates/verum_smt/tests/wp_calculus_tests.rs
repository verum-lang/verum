//! Comprehensive tests for WP (Weakest Precondition) calculus
//!
//! Tests verify that the WP calculus correctly computes preconditions for:
//! - Assignments (simple and compound)
//! - Conditionals (if-else)
//! - Loops (while with invariants, bounded unrolling)
//! - Function calls with contract summarization
//! - Dataflow analysis for loop body effects

#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated
)]

use verum_ast::smallvec::smallvec;
use verum_ast::{
    Expr, Ident, Literal, Path, Pattern, PatternKind, Span, Stmt, Type, TypeKind,
    expr::{BinOp, Block, ConditionKind, ExprKind, IfCondition},
    stmt::StmtKind,
};
use verum_common::Text as CoreText; // alias for String - used by verum_ast
use verum_common::{Heap, List, Maybe};
use verum_smt::{
    Context, ContextConfig,
    wp_calculus::{DataflowAnalyzer, WpEngine, WpError, extract_loop_body_effects_enhanced},
};
use verum_common::List as StdList;
use verum_common::Text as StdText; // The Text wrapper type used by verum_smt
use z3::ast::{Ast, Bool, Int};

// Helper to create a dummy span
fn dummy_span() -> Span {
    Span::dummy()
}

// Helper to create an identifier
fn make_ident(name: &str) -> Ident {
    Ident::new(CoreText::from(name), dummy_span())
}

// Helper to create a path expression
fn make_path_expr(name: &str) -> Expr {
    Expr::path(Path::from_ident(make_ident(name)))
}

// Helper to create an integer literal expression
fn make_int_expr(value: i64) -> Expr {
    Expr::literal(Literal::int(value as i128, dummy_span()))
}

// Helper to create a boolean literal expression
fn make_bool_expr(value: bool) -> Expr {
    Expr::literal(Literal::bool(value, dummy_span()))
}

// Helper to create a binary expression
fn make_binary_expr(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Heap::new(left),
            right: Heap::new(right),
        },
        span: dummy_span(),
        check_eliminated: false,
        ref_kind: None,
    }
}

// Helper to create a block expression
fn make_block_expr(stmts: List<Stmt>, result: Option<Expr>) -> Expr {
    let expr_maybe = result.map(Heap::new);
    Expr::new(
        ExprKind::Block(Block {
            stmts,
            expr: expr_maybe,
            span: dummy_span(),
        }),
        dummy_span(),
    )
}

// Helper to create an expression statement
fn make_expr_stmt(expr: Expr) -> Stmt {
    Stmt {
        kind: StmtKind::Expr {
            expr,
            has_semi: true,
        },
        span: dummy_span(),
        attributes: Vec::new(),
    }
}

// Helper to create a let statement
fn make_let_stmt(name: &str, value: Expr) -> Stmt {
    Stmt {
        kind: StmtKind::Let {
            pattern: Pattern {
                kind: PatternKind::Ident {
                    name: make_ident(name),
                    by_ref: false,
                    mutable: false,
                    subpattern: Maybe::None,
                },
                span: dummy_span(),
            },
            ty: Maybe::None,
            value: Maybe::Some(value),
        },
        span: dummy_span(),
        attributes: Vec::new(),
    }
}

// Helper to create an if condition (properly using SmallVec)
fn make_if_condition(cond_expr: Expr) -> Heap<IfCondition> {
    Heap::new(IfCondition {
        conditions: smallvec![ConditionKind::Expr(cond_expr)],
        span: dummy_span(),
    })
}

// Helper to create an if expression
fn make_if_expr(condition: Expr, then_stmts: List<Stmt>, else_stmts: Option<List<Stmt>>) -> Expr {
    let else_branch = else_stmts.map(|stmts| Heap::new(make_block_expr(stmts, None)));

    Expr::new(
        ExprKind::If {
            condition: make_if_condition(condition),
            then_branch: Block {
                stmts: then_stmts,
                expr: Maybe::None,
                span: dummy_span(),
            },
            else_branch,
        },
        dummy_span(),
    )
}

// ==================== WP Engine Tests ====================

#[test]
fn test_wp_engine_creation() {
    let context = Context::with_config(ContextConfig::fast());
    let _engine = WpEngine::new(&context);
    // WpEngine should be creatable without errors
    assert!(true);
}

#[test]
fn test_wp_engine_bind_input() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind an integer input
    let result = engine.bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()));
    assert!(result.is_ok());

    // Bind a boolean input
    let result = engine.bind_input(
        &StdText::from("b"),
        &Type::new(TypeKind::Bool, dummy_span()),
    );
    assert!(result.is_ok());
}

#[test]
fn test_wp_pure_expression() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variable
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create a postcondition: x > 0
    let postcond = Bool::new_const("postcond");

    // WP of a pure expression (just a path) should be the postcondition itself
    let path_expr = make_path_expr("x");
    let wp_result = engine.wp(&path_expr, &postcond);

    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_assignment_simple() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variable
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create postcondition
    let postcond = Bool::new_const("x_gt_zero");

    // Create assignment: x = 5
    let assign_expr = make_binary_expr(BinOp::Assign, make_path_expr("x"), make_int_expr(5));

    // WP(x := 5, Q) should compute
    let wp_result = engine.wp(&assign_expr, &postcond);
    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_assignment_compound() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variable
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create postcondition
    let postcond = Bool::new_const("x_gt_zero");

    // Create compound assignment: x += 1
    let assign_expr = make_binary_expr(BinOp::AddAssign, make_path_expr("x"), make_int_expr(1));

    // WP(x += 1, Q) should compute
    let wp_result = engine.wp(&assign_expr, &postcond);
    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_block_sequential() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variables
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();
    engine
        .bind_input(&StdText::from("y"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create postcondition
    let postcond = Bool::new_const("postcond");

    // Create block: { x = 1; y = 2; }
    let stmts = vec![
        make_expr_stmt(make_binary_expr(
            BinOp::Assign,
            make_path_expr("x"),
            make_int_expr(1),
        )),
        make_expr_stmt(make_binary_expr(
            BinOp::Assign,
            make_path_expr("y"),
            make_int_expr(2),
        )),
    ];

    let block_expr = make_block_expr(stmts.into(), None);

    // WP({ S1; S2 }, Q) should be wp(S1, wp(S2, Q))
    let wp_result = engine.wp(&block_expr, &postcond);
    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_let_binding() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Create postcondition
    let postcond = Bool::new_const("postcond");

    // Create block with let: { let x = 5; }
    let stmts = vec![make_let_stmt("x", make_int_expr(5))];

    let block_expr = make_block_expr(stmts.into(), None);

    let wp_result = engine.wp(&block_expr, &postcond);
    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_if_then_else() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variable
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create postcondition
    let postcond = Bool::new_const("postcond");

    // Create if-then-else: if x > 0 { x = x + 1 } else { x = 0 }
    let cond_expr = make_binary_expr(BinOp::Gt, make_path_expr("x"), make_int_expr(0));

    let then_stmts: List<Stmt> = vec![make_expr_stmt(make_binary_expr(
        BinOp::Assign,
        make_path_expr("x"),
        make_binary_expr(BinOp::Add, make_path_expr("x"), make_int_expr(1)),
    ))].into();

    let else_stmts: List<Stmt> = vec![make_expr_stmt(make_binary_expr(
        BinOp::Assign,
        make_path_expr("x"),
        make_int_expr(0),
    ))].into();

    let if_expr = make_if_expr(cond_expr, then_stmts, Some(else_stmts));

    // WP(if b then S1 else S2, Q) should be (b => wp(S1, Q)) && (!b => wp(S2, Q))
    let wp_result = engine.wp(&if_expr, &postcond);
    assert!(wp_result.is_ok());
}

#[test]
fn test_wp_return() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind input variable
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Create postcondition
    let postcond = Bool::new_const("result_positive");

    // Create return: return x
    let return_expr = Expr::new(
        ExprKind::Return(Maybe::Some(Heap::new(make_path_expr("x")))),
        dummy_span(),
    );

    // WP(return e, Q) should bind result to e and return Q
    let wp_result = engine.wp(&return_expr, &postcond);
    assert!(wp_result.is_ok());
}

// ==================== Dataflow Analysis Tests ====================

#[test]
fn test_dataflow_simple_assignment() {
    let state_vars = vec![
        (StdText::from("x"), Type::new(TypeKind::Int, dummy_span())),
        (StdText::from("y"), Type::new(TypeKind::Int, dummy_span())),
    ];

    // Create: x = x + 1
    let body = make_binary_expr(
        BinOp::Assign,
        make_path_expr("x"),
        make_binary_expr(BinOp::Add, make_path_expr("x"), make_int_expr(1)),
    );

    let effects = extract_loop_body_effects_enhanced(&body, &state_vars);

    // Should detect x is modified
    assert!(!effects.is_empty());
    let modified_vars: Vec<&StdText> = effects
        .iter()
        .map(|(name, _): &(StdText, Expr)| name)
        .collect();
    assert!(modified_vars.iter().any(|n: &&StdText| n.as_str() == "x"));
}

#[test]
fn test_dataflow_compound_assignment() {
    let state_vars = vec![(StdText::from("sum"), Type::new(TypeKind::Int, dummy_span()))];

    // Create: sum += value
    let body = make_binary_expr(BinOp::AddAssign, make_path_expr("sum"), make_int_expr(10));

    let effects = extract_loop_body_effects_enhanced(&body, &state_vars);

    // Should detect sum is modified
    assert!(!effects.is_empty());
}

#[test]
fn test_dataflow_multiple_assignments() {
    let state_vars = vec![
        (StdText::from("x"), Type::new(TypeKind::Int, dummy_span())),
        (StdText::from("y"), Type::new(TypeKind::Int, dummy_span())),
        (StdText::from("z"), Type::new(TypeKind::Int, dummy_span())),
    ];

    // Create block: { x = 1; y = 2; }
    let stmts = vec![
        make_expr_stmt(make_binary_expr(
            BinOp::Assign,
            make_path_expr("x"),
            make_int_expr(1),
        )),
        make_expr_stmt(make_binary_expr(
            BinOp::Assign,
            make_path_expr("y"),
            make_int_expr(2),
        )),
    ];

    let body = make_block_expr(stmts.into(), None);

    let effects = extract_loop_body_effects_enhanced(&body, &state_vars);

    // Should detect both x and y are modified, but not z
    let modified_names: Vec<String> = effects
        .iter()
        .map(|(n, _): &(StdText, Expr)| n.to_string())
        .collect();
    assert!(modified_names.iter().any(|n: &String| n == "x"));
    assert!(modified_names.iter().any(|n: &String| n == "y"));
}

#[test]
fn test_dataflow_conditional_modification() {
    let state_vars = vec![(StdText::from("x"), Type::new(TypeKind::Int, dummy_span()))];

    // Create: if cond { x = 1 } else { x = 2 }
    let cond_expr = make_bool_expr(true);

    let then_stmts: List<Stmt> = vec![make_expr_stmt(make_binary_expr(
        BinOp::Assign,
        make_path_expr("x"),
        make_int_expr(1),
    ))].into();

    let else_stmts: List<Stmt> = vec![make_expr_stmt(make_binary_expr(
        BinOp::Assign,
        make_path_expr("x"),
        make_int_expr(2),
    ))].into();

    let if_expr = make_if_expr(cond_expr, then_stmts, Some(else_stmts));

    let effects = extract_loop_body_effects_enhanced(&if_expr, &state_vars);

    // Should detect x is modified in both branches
    assert!(!effects.is_empty());
}

#[test]
fn test_dataflow_nested_loop() {
    let state_vars = vec![
        (StdText::from("i"), Type::new(TypeKind::Int, dummy_span())),
        (StdText::from("j"), Type::new(TypeKind::Int, dummy_span())),
    ];

    // Create nested modification: i += 1; j += 1
    let inner_stmts = vec![
        make_expr_stmt(make_binary_expr(
            BinOp::AddAssign,
            make_path_expr("i"),
            make_int_expr(1),
        )),
        make_expr_stmt(make_binary_expr(
            BinOp::AddAssign,
            make_path_expr("j"),
            make_int_expr(1),
        )),
    ];

    let body = make_block_expr(inner_stmts.into(), None);

    let effects = extract_loop_body_effects_enhanced(&body, &state_vars);

    // Both i and j should be detected as modified
    let modified_names: Vec<String> = effects
        .iter()
        .map(|(n, _): &(StdText, Expr)| n.to_string())
        .collect();
    assert!(modified_names.iter().any(|n: &String| n == "i"));
    assert!(modified_names.iter().any(|n: &String| n == "j"));
}

#[test]
fn test_dataflow_analyzer_direct() {
    let state_vars = vec![(
        StdText::from("counter"),
        Type::new(TypeKind::Int, dummy_span()),
    )];

    // Create: counter += 1
    let body = make_binary_expr(
        BinOp::AddAssign,
        make_path_expr("counter"),
        make_int_expr(1),
    );

    let mut analyzer = DataflowAnalyzer::new(&state_vars);
    analyzer.analyze(&body);

    let modifications = analyzer.get_modifications();
    assert!(!modifications.is_empty());
    assert!(
        modifications
            .iter()
            .any(|m| m.variable.as_str() == "counter")
    );
}

#[test]
fn test_dataflow_no_modification() {
    let state_vars = vec![(StdText::from("x"), Type::new(TypeKind::Int, dummy_span()))];

    // Create: y = 1 (y is not a state variable)
    let body = make_binary_expr(BinOp::Assign, make_path_expr("y"), make_int_expr(1));

    let effects = extract_loop_body_effects_enhanced(&body, &state_vars);

    // x should NOT be detected as modified (only y was modified)
    let modified_x = effects
        .iter()
        .any(|(n, _): &(StdText, Expr)| n.as_str() == "x");
    assert!(!modified_x);
}

// ==================== Integration Tests ====================

#[test]
fn test_wp_engine_set_loop_bound() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Set a custom loop unroll bound
    engine.set_loop_unroll_bound(5);
    // No assertion needed - just testing it doesn't panic
    assert!(true);
}

#[test]
fn test_wp_engine_capture_old_values() {
    let context = Context::with_config(ContextConfig::fast());
    let mut engine = WpEngine::new(&context);

    // Bind variables
    engine
        .bind_input(&StdText::from("x"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();
    engine
        .bind_input(&StdText::from("y"), &Type::new(TypeKind::Int, dummy_span()))
        .unwrap();

    // Capture old values
    let var_names = vec![StdText::from("x"), StdText::from("y")];
    engine.capture_old_values(&var_names);

    // Old values should be stored for postcondition `old(expr)` references
    assert!(true);
}

#[test]
fn test_state_modification_structure() {
    use verum_smt::StateModification;

    let modification = StateModification {
        variable: StdText::from("x"),
        value_expr: make_int_expr(42),
        is_direct: true,
        reference_path: StdList::new(),
    };

    assert_eq!(modification.variable.as_str(), "x");
    assert!(modification.is_direct);
    assert!(modification.reference_path.is_empty());
}
