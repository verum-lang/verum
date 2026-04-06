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
//! Tests for the visitor pattern and AST traversal.
//!
//! This module ensures that the visitor pattern correctly traverses
//! all nodes in the AST and that custom visitors can be implemented.
//!
//! The visitor system supports two traversal modes:
//! - Recursive mode (default): Simple traversal using the call stack
//! - Iterative mode: Stack-safe traversal using IterativeVisitor wrapper

use verum_ast::expr::*;
use verum_ast::pattern::MatchArm;
use verum_ast::ty::PathSegment;
use verum_ast::visitor::{
    IterativeVisitor, Visitor, traverse_expr_iteratively, walk_expr, walk_item, walk_pattern,
    walk_stmt, walk_type,
};
use verum_ast::*;
use verum_common::{Heap, List, Maybe, Set};

/// Helper function to create a test span
fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

/// Helper function to create a test identifier
fn test_ident(name: &str) -> Ident {
    Ident::new(name.to_string(), test_span())
}

/// A visitor that counts different node types
struct CountingVisitor {
    items: usize,
    exprs: usize,
    stmts: usize,
    patterns: usize,
    types: usize,
    literals: usize,
}

impl CountingVisitor {
    fn new() -> Self {
        Self {
            items: 0,
            exprs: 0,
            stmts: 0,
            patterns: 0,
            types: 0,
            literals: 0,
        }
    }
}

impl Visitor for CountingVisitor {
    fn visit_item(&mut self, item: &Item) {
        self.items += 1;
        walk_item(self, item);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        self.exprs += 1;
        // Count literals separately
        if let ExprKind::Literal(_) = &expr.kind {
            self.literals += 1;
        }
        walk_expr(self, expr);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        self.stmts += 1;
        walk_stmt(self, stmt);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        self.patterns += 1;
        walk_pattern(self, pattern);
    }

    fn visit_type(&mut self, ty: &Type) {
        self.types += 1;
        walk_type(self, ty);
    }
}

#[test]
fn test_counting_visitor() {
    let span = test_span();

    // Create a simple AST
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let mut visitor = CountingVisitor::new();
    visitor.visit_expr(&expr);

    assert_eq!(visitor.exprs, 3); // Binary expr + 2 literal exprs
    assert_eq!(visitor.literals, 2); // 2 literals
}

/// A visitor that collects all identifiers in the AST
struct IdentCollector {
    idents: Set<verum_common::Text>,
}

impl IdentCollector {
    fn new() -> Self {
        Self { idents: Set::new() }
    }

    fn collect_ident(&mut self, ident: &Ident) {
        self.idents.insert(ident.name.clone());
    }
}

impl Visitor for IdentCollector {
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Path(path) => {
                for segment in &path.segments {
                    if let PathSegment::Name(ident) = segment {
                        self.collect_ident(ident);
                    }
                }
            }
            ExprKind::Field { field, .. } => {
                self.collect_ident(field);
            }
            ExprKind::MethodCall { method, .. } => {
                self.collect_ident(method);
            }
            _ => {}
        }
        walk_expr(self, expr);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        if let PatternKind::Ident { name, .. } = &pattern.kind {
            self.collect_ident(name);
        }
        walk_pattern(self, pattern);
    }

    fn visit_type(&mut self, ty: &Type) {
        if let TypeKind::Path(path) = &ty.kind {
            for segment in &path.segments {
                if let PathSegment::Name(ident) = segment {
                    self.collect_ident(ident);
                }
            }
        }
        walk_type(self, ty);
    }
}

#[test]
fn test_ident_collector() {
    let span = test_span();

    // Create an AST with various identifiers
    let stmt = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::ident(test_ident("x"), false, span),
            ty: Maybe::Some(Type::new(
                TypeKind::Path(Path::single(test_ident("Int"))),
                span,
            )),
            value: Maybe::Some(Expr::ident(test_ident("y"))),
        },
        span,
    );

    let mut collector = IdentCollector::new();
    collector.visit_stmt(&stmt);

    assert!(collector.idents.iter().any(|s| s.as_str() == "x"));
    assert!(collector.idents.iter().any(|s| s.as_str() == "Int"));
    assert!(collector.idents.iter().any(|s| s.as_str() == "y"));
    assert_eq!(collector.idents.len(), 3);
}

/// A visitor that transforms the AST (collects transformation points)
struct TransformVisitor {
    transformed_exprs: Vec<String>,
}

impl TransformVisitor {
    fn new() -> Self {
        Self {
            transformed_exprs: Vec::new(),
        }
    }
}

impl Visitor for TransformVisitor {
    fn visit_expr(&mut self, expr: &Expr) {
        // Record transformations we would make
        match &expr.kind {
            ExprKind::Binary { op, .. } => {
                self.transformed_exprs.push(format!("Binary {:?}", op));
            }
            ExprKind::Unary { op, .. } => {
                self.transformed_exprs.push(format!("Unary {:?}", op));
            }
            ExprKind::Literal(lit) => {
                let lit_str = match &lit.kind {
                    LiteralKind::Int(_) => "Int",
                    LiteralKind::Float(_) => "Float",
                    LiteralKind::Text(_) => "String",
                    LiteralKind::Bool(b) => {
                        if *b {
                            "Bool(true)"
                        } else {
                            "Bool(false)"
                        }
                    }
                    LiteralKind::Char(_) => "Char",
                    _ => "Other",
                };
                self.transformed_exprs.push(format!("Literal {}", lit_str));
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

#[test]
fn test_transform_visitor() {
    let span = test_span();

    // Create: -( 1 + 2 )
    let add = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let neg = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Heap::new(add),
        },
        span,
    );

    let mut visitor = TransformVisitor::new();
    visitor.visit_expr(&neg);

    assert_eq!(visitor.transformed_exprs.len(), 4);
    assert_eq!(visitor.transformed_exprs[0], "Unary Neg");
    assert_eq!(visitor.transformed_exprs[1], "Binary Add");
    assert_eq!(visitor.transformed_exprs[2], "Literal Int");
    assert_eq!(visitor.transformed_exprs[3], "Literal Int");
}

/// Test visitor for complex nested structures
struct DepthTracker {
    max_depth: usize,
    current_depth: usize,
    expr_depths: Vec<usize>,
}

impl DepthTracker {
    fn new() -> Self {
        Self {
            max_depth: 0,
            current_depth: 0,
            expr_depths: Vec::new(),
        }
    }
}

impl Visitor for DepthTracker {
    fn visit_expr(&mut self, expr: &Expr) {
        self.current_depth += 1;
        self.max_depth = self.max_depth.max(self.current_depth);
        self.expr_depths.push(self.current_depth);

        walk_expr(self, expr);

        self.current_depth -= 1;
    }
}

#[test]
fn test_depth_tracking() {
    let span = test_span();

    // Create deeply nested expression: ((1 + 2) * (3 - 4))
    let add = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    ));

    let sub = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub,
            left: Heap::new(Expr::literal(Literal::int(3, span))),
            right: Heap::new(Expr::literal(Literal::int(4, span))),
        },
        span,
    ));

    let mul = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: add,
            right: sub,
        },
        span,
    );

    let mut tracker = DepthTracker::new();
    tracker.visit_expr(&mul);

    assert_eq!(tracker.max_depth, 3); // mul -> add/sub -> literals
    assert_eq!(tracker.expr_depths.len(), 7); // Total expressions visited
}

/// Test visiting patterns
struct PatternVisitor {
    wildcard_count: usize,
    ident_count: usize,
    literal_count: usize,
    tuple_count: usize,
    slice_count: usize,
}

impl PatternVisitor {
    fn new() -> Self {
        Self {
            wildcard_count: 0,
            ident_count: 0,
            literal_count: 0,
            tuple_count: 0,
            slice_count: 0,
        }
    }
}

impl Visitor for PatternVisitor {
    fn visit_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard => self.wildcard_count += 1,
            PatternKind::Ident { .. } => self.ident_count += 1,
            PatternKind::Literal(_) => self.literal_count += 1,
            PatternKind::Tuple(_) => self.tuple_count += 1,
            PatternKind::Slice { .. } => self.slice_count += 1,
            _ => {}
        }
        walk_pattern(self, pattern);
    }
}

#[test]
fn test_pattern_visitor() {
    let span = test_span();

    // Create a complex pattern: (x, _, 42, [a, b])
    let slice_pattern = Pattern::new(
        PatternKind::Slice {
            before: List::from(vec![
                Pattern::ident(test_ident("a"), false, span),
                Pattern::ident(test_ident("b"), false, span),
            ]),
            rest: Maybe::None,
            after: List::from(vec![]),
        },
        span,
    );

    let tuple_pattern = Pattern::new(
        PatternKind::Tuple(List::from(vec![
            Pattern::ident(test_ident("x"), false, span),
            Pattern::wildcard(span),
            Pattern::literal(Literal::int(42, span)),
            slice_pattern,
        ])),
        span,
    );

    let mut visitor = PatternVisitor::new();
    visitor.visit_pattern(&tuple_pattern);

    assert_eq!(visitor.tuple_count, 1);
    assert_eq!(visitor.ident_count, 3); // x, a, b
    assert_eq!(visitor.wildcard_count, 1);
    assert_eq!(visitor.literal_count, 1);
    assert_eq!(visitor.slice_count, 1);
}

/// Test visiting statements and blocks
struct StmtVisitor {
    let_count: usize,
    expr_stmt_count: usize,
}

impl StmtVisitor {
    fn new() -> Self {
        Self {
            let_count: 0,
            expr_stmt_count: 0,
        }
    }
}

impl Visitor for StmtVisitor {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let { .. } => self.let_count += 1,
            StmtKind::Expr { .. } => self.expr_stmt_count += 1,
            // Assignment and return are now expressions, not statement kinds
            _ => {}
        }
        walk_stmt(self, stmt);
    }
}

#[test]
fn test_stmt_visitor() {
    let span = test_span();

    let block = Block {
        stmts: List::from(vec![
            // let x = 42;
            Stmt::new(
                StmtKind::Let {
                    pattern: Pattern::ident(test_ident("x"), false, span),
                    ty: Maybe::None,
                    value: Maybe::Some(Expr::literal(Literal::int(42, span))),
                },
                span,
            ),
            // x = 100;
            Stmt::new(
                StmtKind::Expr {
                    expr: Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Assign,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::literal(Literal::int(100, span))),
                        },
                        span,
                    ),
                    has_semi: true,
                },
                span,
            ),
            // return x;
            Stmt::new(
                StmtKind::Expr {
                    expr: Expr::new(
                        ExprKind::Return(Maybe::Some(Heap::new(Expr::ident(test_ident("x"))))),
                        span,
                    ),
                    has_semi: false,
                },
                span,
            ),
        ]),
        expr: Maybe::None,
        span,
    };

    let mut visitor = StmtVisitor::new();
    for stmt in &block.stmts {
        visitor.visit_stmt(stmt);
    }

    assert_eq!(visitor.let_count, 1);
    assert_eq!(visitor.expr_stmt_count, 2); // assignment and return are now expr statements
}

/// Test visiting type nodes
struct TypeVisitor {
    primitive_count: usize,
    tuple_count: usize,
    array_count: usize,
    function_count: usize,
    reference_count: usize,
    path_count: usize,
}

impl TypeVisitor {
    fn new() -> Self {
        Self {
            primitive_count: 0,
            tuple_count: 0,
            array_count: 0,
            function_count: 0,
            reference_count: 0,
            path_count: 0,
        }
    }
}

impl Visitor for TypeVisitor {
    fn visit_type(&mut self, ty: &Type) {
        match &ty.kind {
            TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::Text
            | TypeKind::Unit => {
                self.primitive_count += 1;
            }
            TypeKind::Tuple(_) => self.tuple_count += 1,
            TypeKind::Array { .. } => self.array_count += 1,
            TypeKind::Function { .. } => self.function_count += 1,
            TypeKind::Reference { .. } => self.reference_count += 1,
            TypeKind::Path(_) => self.path_count += 1,
            _ => {}
        }
        walk_type(self, ty);
    }
}

#[test]
fn test_type_visitor() {
    let span = test_span();

    // Create complex type: &(Int, [Text; 10])
    let array_ty = Type::new(
        TypeKind::Array {
            element: Heap::new(Type::text(span)),
            size: Maybe::Some(Heap::new(Expr::literal(Literal::int(10, span)))),
        },
        span,
    );

    let tuple_ty = Type::new(
        TypeKind::Tuple(List::from(vec![Type::int(span), array_ty])),
        span,
    );

    let ref_ty = Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(tuple_ty),
        },
        span,
    );

    let mut visitor = TypeVisitor::new();
    visitor.visit_type(&ref_ty);

    assert_eq!(visitor.reference_count, 1);
    assert_eq!(visitor.tuple_count, 1);
    assert_eq!(visitor.array_count, 1);
    assert_eq!(visitor.primitive_count, 2); // Int and String
}

/// Test that walk functions visit all children
#[test]
fn test_walk_functions_completeness() {
    let span = test_span();

    // Create a complex expression with many node types
    let comprehension = Expr::new(
        ExprKind::StreamComprehension {
            expr: Heap::new(Expr::new(
                ExprKind::Binary {
                    op: BinOp::Mul,
                    left: Heap::new(Expr::ident(test_ident("x"))),
                    right: Heap::new(Expr::literal(Literal::int(2, span))),
                },
                span,
            )),
            clauses: List::from(vec![
                ComprehensionClause {
                    kind: ComprehensionClauseKind::For {
                        pattern: Pattern::ident(test_ident("x"), false, span),
                        iter: Expr::ident(test_ident("source")),
                    },
                    span,
                },
                ComprehensionClause {
                    kind: ComprehensionClauseKind::If(Expr::new(
                        ExprKind::Binary {
                            op: BinOp::Gt,
                            left: Heap::new(Expr::ident(test_ident("x"))),
                            right: Heap::new(Expr::literal(Literal::int(0, span))),
                        },
                        span,
                    )),
                    span,
                },
            ]),
        },
        span,
    );

    let mut counter = CountingVisitor::new();
    counter.visit_expr(&comprehension);

    // Should visit: comprehension, mul, x*2, for-x, source, if-expr, x>0, x, 0
    assert!(
        counter.exprs >= 8,
        "Expected at least 8 expressions, got {}",
        counter.exprs
    );
    assert_eq!(counter.literals, 2); // 2 and 0
}

/// Test custom visitor that stops traversal early
struct EarlyStopVisitor {
    stop_at_depth: usize,
    current_depth: usize,
    visited_exprs: Vec<String>,
}

impl EarlyStopVisitor {
    fn new(stop_at_depth: usize) -> Self {
        Self {
            stop_at_depth,
            current_depth: 0,
            visited_exprs: Vec::new(),
        }
    }
}

impl Visitor for EarlyStopVisitor {
    fn visit_expr(&mut self, expr: &Expr) {
        self.current_depth += 1;

        match &expr.kind {
            ExprKind::Binary { op, .. } => {
                self.visited_exprs
                    .push(format!("Binary{:?}@{}", op, self.current_depth));
            }
            ExprKind::Literal(_lit) => {
                self.visited_exprs
                    .push(format!("Literal@{}", self.current_depth));
            }
            _ => {}
        }

        // Only continue traversal if we haven't reached the stop depth
        if self.current_depth < self.stop_at_depth {
            walk_expr(self, expr);
        }

        self.current_depth -= 1;
    }
}

#[test]
fn test_early_stop_visitor() {
    let span = test_span();

    // Create: (1 + 2) * (3 - 4)
    let add = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    ));

    let sub = Heap::new(Expr::new(
        ExprKind::Binary {
            op: BinOp::Sub,
            left: Heap::new(Expr::literal(Literal::int(3, span))),
            right: Heap::new(Expr::literal(Literal::int(4, span))),
        },
        span,
    ));

    let mul = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: add,
            right: sub,
        },
        span,
    );

    // Stop at depth 2 (should not visit the literal nodes)
    let mut visitor = EarlyStopVisitor::new(2);
    visitor.visit_expr(&mul);

    assert_eq!(visitor.visited_exprs.len(), 3); // mul, add, sub (but not literals)
    assert!(visitor.visited_exprs[0].contains("BinaryMul"));
    assert!(visitor.visited_exprs[1].contains("BinaryAdd"));
    assert!(visitor.visited_exprs[2].contains("BinarySub"));
}

/// Test visiting match expressions
#[test]
fn test_match_visitor() {
    let span = test_span();

    // Create a match expression
    let match_expr = Expr::new(
        ExprKind::Match {
            expr: Heap::new(Expr::ident(test_ident("x"))),
            arms: List::from(vec![
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::literal(Literal::int(1, span)),
                    guard: Maybe::None,
                    body: Heap::new(Expr::literal(Literal::string("one".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::literal(Literal::int(2, span)),
                    guard: Maybe::Some(Heap::new(Expr::literal(Literal::bool(true, span)))),
                    body: Heap::new(Expr::literal(Literal::string("two".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
                MatchArm {
                    attributes: verum_common::List::new(),
                    pattern: Pattern::wildcard(span),
                    guard: Maybe::None,
                    body: Heap::new(Expr::literal(Literal::string("other".to_string().into(), span))),
                    with_clause: Maybe::None,
                    span,
                },
            ]),
        },
        span,
    );

    let mut counter = CountingVisitor::new();
    counter.visit_expr(&match_expr);

    // Should visit: match expr, x, 3 arm bodies, 1 guard = 6 expressions
    assert_eq!(counter.exprs, 6);
    assert_eq!(counter.literals, 4); // 3 strings + 1 bool
    assert_eq!(counter.patterns, 3); // 3 arm patterns
}

// ============================================================================
// ITERATIVE MODE TESTS
// ============================================================================

/// Helper to create a deeply nested binary expression tree
/// Uses iteration instead of recursion to avoid stack overflow for large depths
fn create_deep_expr(depth: usize) -> Expr {
    let span = test_span();

    // Start with the base case (leaf node)
    let mut result = Expr::literal(Literal::int(1, span));

    // Build the tree iteratively from bottom up
    for _ in 0..depth {
        result = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: Heap::new(result),
                right: Heap::new(Expr::literal(Literal::int(1, span))),
            },
            span,
        );
    }

    result
}

#[test]
fn test_iterative_visitor_basic() {
    let span = test_span();

    // Create a simple AST
    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let counter = CountingVisitor::new();
    let mut iter_visitor = IterativeVisitor::new(counter);
    iter_visitor.traverse_expr(&expr);
    let counter = iter_visitor.into_inner();

    assert_eq!(counter.exprs, 3); // Binary expr + 2 literal exprs
    assert_eq!(counter.literals, 2); // 2 literals
}

#[test]
fn test_iterative_visitor_matches_recursive() {
    let span = test_span();

    // Create: -( (1 + 2) * 3 )
    let add = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let mul = Expr::new(
        ExprKind::Binary {
            op: BinOp::Mul,
            left: Heap::new(add.clone()),
            right: Heap::new(Expr::literal(Literal::int(3, span))),
        },
        span,
    );

    let neg = Expr::new(
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Heap::new(mul.clone()),
        },
        span,
    );

    // Recursive mode
    let mut recursive_counter = CountingVisitor::new();
    recursive_counter.visit_expr(&neg);

    // Iterative mode
    let iterative_counter = CountingVisitor::new();
    let mut iter_visitor = IterativeVisitor::new(iterative_counter);
    iter_visitor.traverse_expr(&neg);
    let iterative_counter = iter_visitor.into_inner();

    // Both should produce the same counts
    assert_eq!(recursive_counter.exprs, iterative_counter.exprs);
    assert_eq!(recursive_counter.literals, iterative_counter.literals);
}

#[test]
fn test_traverse_expr_iteratively_convenience() {
    let span = test_span();

    let expr = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(Expr::literal(Literal::int(1, span))),
            right: Heap::new(Expr::literal(Literal::int(2, span))),
        },
        span,
    );

    let counter = CountingVisitor::new();
    let counter = traverse_expr_iteratively(counter, &expr);

    assert_eq!(counter.exprs, 3);
    assert_eq!(counter.literals, 2);
}

#[test]
fn test_iterative_mode_depth_100() {
    // Test with moderately deep tree (100 levels)
    let deep_expr = create_deep_expr(100);

    let counter = CountingVisitor::new();
    let mut iter_visitor = IterativeVisitor::new(counter);
    iter_visitor.traverse_expr(&deep_expr);
    let counter = iter_visitor.into_inner();

    // Should have visited all nodes: 100 binary + 101 literals = 201
    assert_eq!(counter.exprs, 201);
    assert_eq!(counter.literals, 101);
}

#[test]
fn test_iterative_mode_depth_1000() {
    // Test with deep tree (1000 levels)
    let deep_expr = create_deep_expr(1000);

    let counter = CountingVisitor::new();
    let mut iter_visitor = IterativeVisitor::new(counter);
    iter_visitor.traverse_expr(&deep_expr);
    let counter = iter_visitor.into_inner();

    // Should have visited all nodes: 1000 binary + 1001 literals = 2001
    assert_eq!(counter.exprs, 2001);
    assert_eq!(counter.literals, 1001);
}

#[test]
fn test_iterative_mode_depth_2000() {
    // Test with deep tree (2000 levels)
    //
    // Note: While `create_deep_expr` builds the tree iteratively (stack-safe),
    // the current `IterativeVisitor` architecture still has recursive calls
    // when the inner visitor's `visit_*` methods call `walk_*` functions.
    // The `walk_*` functions use `visit_child!` macro which falls back to
    // recursion when the visitor doesn't provide a `work_stack()`.
    //
    // Depth 2000 is chosen to fit within typical test thread stack limits
    // (~512KB on macOS) while still exercising the iterative traversal logic.
    let deep_expr = create_deep_expr(2000);

    let counter = CountingVisitor::new();
    let mut iter_visitor = IterativeVisitor::new(counter);
    iter_visitor.traverse_expr(&deep_expr);
    let counter = iter_visitor.into_inner();

    // Should have visited all nodes: 2000 binary + 2001 literals = 4001
    assert_eq!(counter.exprs, 4001);
    assert_eq!(counter.literals, 2001);
}

// Note: For truly arbitrary depth traversal without stack limits, the inner
// visitor must not call `walk_*` functions, or the architecture needs to be
// extended so the inner visitor has access to the work stack.
