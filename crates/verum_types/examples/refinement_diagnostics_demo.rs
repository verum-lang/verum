//! Demonstration of enhanced refinement error diagnostics
//!
//! This example shows how the enhanced diagnostics provide:
//! - Actual value tracking
//! - Predicate decomposition with ✓/✗ markers
//! - Context-aware suggestions
//! - Multi-constraint breakdown
//!
//! Run with: cargo run --example refinement_diagnostics_demo

use verum_ast::{
    expr::{BinOp, Expr, ExprKind},
    literal::Literal,
    span::{FileId, Span},
    ty::{Ident, Path},
};
use verum_common::Maybe;
use verum_types::{
    ConstValue, ErrorContext, PredicateEvaluator, RefinementDiagnosticBuilder, RefinementSource,
};

fn create_span(_file: &str, _line: u32) -> Span {
    // Create a simple span with dummy byte offsets
    // In a real scenario, we'd have proper source file tracking
    Span::new(0, 5, FileId::dummy())
}

fn create_var(name: &str) -> Expr {
    let span = create_span("demo.vr", 1);
    let ident = Ident::new(name, span);
    let path = Path::single(ident);
    Expr {
        kind: ExprKind::Path(path),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn create_int(value: i64) -> Expr {
    let span = create_span("demo.vr", 1);
    let literal = Literal::int(value as i128, span);
    Expr {
        kind: ExprKind::Literal(literal),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn create_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        span: create_span("demo.vr", 1),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn main() {
    println!("=================================================");
    println!("Enhanced Refinement Error Diagnostics Demo");
    println!("Refinement type diagnostics: error messages for failed refinement checks");
    println!("=================================================\n");

    // Example 1: Basic refinement violation (Spec Lines 13180-13196)
    println!("Example 1: Positive constraint violation");
    println!("-------------------------------------------------");
    demonstrate_positive_constraint();
    println!();

    // Example 2: Multi-constraint breakdown (Spec Lines 13428-13448)
    println!("Example 2: Multi-constraint breakdown");
    println!("-------------------------------------------------");
    demonstrate_multi_constraint();
    println!();

    // Example 3: Predicate decomposition
    println!("Example 3: Predicate decomposition with evaluation");
    println!("-------------------------------------------------");
    demonstrate_predicate_decomposition();
    println!();

    // Example 4: Context-aware suggestions
    println!("Example 4: Context-aware suggestions");
    println!("-------------------------------------------------");
    demonstrate_suggestions();
    println!();
}

fn demonstrate_positive_constraint() {
    println!("Code: let x: Positive = -5;");
    println!();

    // Create predicate: x > 0
    let predicate = create_binary(BinOp::Gt, create_var("x"), create_int(0));

    let context = ErrorContext {
        function_name: None,
        expected_type: "Positive".into(),
        actual_type: "Int".into(),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    let diag = RefinementDiagnosticBuilder::new()
        .constraint("x > 0".into())
        .actual_value(ConstValue::Int(-5))
        .context(context)
        .span(create_span("main.vr", 2))
        .predicate_expr(predicate)
        .var_name("x".into())
        .build();

    println!("{}", diag.format_error());
}

fn demonstrate_multi_constraint() {
    println!("Code: let x: SmallPositive = 150;");
    println!("Type: SmallPositive is Int where x > 0 && x < 100");
    println!();

    // Create compound predicate: x > 0 && x < 100
    let left = create_binary(BinOp::Gt, create_var("x"), create_int(0));
    let right = create_binary(BinOp::Lt, create_var("x"), create_int(100));
    let predicate = create_binary(BinOp::And, left, right);

    let context = ErrorContext {
        function_name: None,
        expected_type: "SmallPositive".into(),
        actual_type: "Int".into(),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    let diag = RefinementDiagnosticBuilder::new()
        .constraint("x > 0 && x < 100".into())
        .actual_value(ConstValue::Int(150))
        .context(context)
        .span(create_span("main.vr", 5))
        .predicate_expr(predicate)
        .var_name("x".into())
        .build();

    println!("{}", diag.format_error());

    println!("\nConstraint evaluation breakdown:");
    for eval in &diag.constraint_evals {
        println!("  {}", eval.format_line());
    }
}

fn demonstrate_predicate_decomposition() {
    let evaluator = PredicateEvaluator::new();

    // Create: x > 0 && x < 100 && x != 50
    let c1 = create_binary(BinOp::Gt, create_var("x"), create_int(0));
    let c2 = create_binary(BinOp::Lt, create_var("x"), create_int(100));
    let c3 = create_binary(BinOp::Ne, create_var("x"), create_int(50));

    let p1 = create_binary(BinOp::And, c1, c2);
    let predicate = create_binary(BinOp::And, p1, c3);

    println!("Predicate: x > 0 && x < 100 && x != 50");
    println!();

    // Test with value -5
    println!("Testing with value: -5");
    let evals = evaluator.decompose_with_value(&predicate, &ConstValue::Int(-5), "x");
    for eval in &evals {
        println!("  {}", eval.format_line());
    }
    println!();

    // Test with value 50
    println!("Testing with value: 50");
    let evals = evaluator.decompose_with_value(&predicate, &ConstValue::Int(50), "x");
    for eval in &evals {
        println!("  {}", eval.format_line());
    }
    println!();

    // Test with value 75 (satisfies all)
    println!("Testing with value: 75");
    let evals = evaluator.decompose_with_value(&predicate, &ConstValue::Int(75), "x");
    for eval in &evals {
        println!("  {}", eval.format_line());
    }
}

fn demonstrate_suggestions() {
    use verum_types::SuggestionGenerator;

    println!("Generating suggestions for different contexts:\n");

    // Type annotation context
    let context = ErrorContext {
        function_name: None,
        expected_type: "Positive".into(),
        actual_type: "Int".into(),
        refinement_source: RefinementSource::TypeAnnotation,
    };

    println!("1. Type Annotation Context:");
    let none_value: Maybe<ConstValue> = Maybe::None;
    let suggestions = SuggestionGenerator::generate("x > 0", &context, &none_value);
    for (i, suggestion) in suggestions.iter().enumerate() {
        println!("   Suggestion {}: {}", i + 1, suggestion.format_help());
    }
    println!();

    // Function parameter context
    let context = ErrorContext {
        function_name: Some("divide".into()),
        expected_type: "NonZero".into(),
        actual_type: "Int".into(),
        refinement_source: RefinementSource::FunctionParameter,
    };

    println!("2. Function Parameter Context:");
    let none_value: Maybe<ConstValue> = Maybe::None;
    let suggestions = SuggestionGenerator::generate("x != 0", &context, &none_value);
    for (i, suggestion) in suggestions.iter().enumerate() {
        println!("   Suggestion {}: {}", i + 1, suggestion.format_help());
    }
    println!();

    // Function return context
    let context = ErrorContext {
        function_name: Some("sqrt".into()),
        expected_type: "NonNegative".into(),
        actual_type: "Float".into(),
        refinement_source: RefinementSource::FunctionReturn,
    };

    println!("3. Function Return Context:");
    let none_value: Maybe<ConstValue> = Maybe::None;
    let suggestions = SuggestionGenerator::generate("x >= 0", &context, &none_value);
    for (i, suggestion) in suggestions.iter().enumerate() {
        println!("   Suggestion {}: {}", i + 1, suggestion.format_help());
    }
}
