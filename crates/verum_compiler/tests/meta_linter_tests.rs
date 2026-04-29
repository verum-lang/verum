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
//! Comprehensive tests for the Meta Linter
//!
//! Tests all detection algorithms:
//! 1. String concatenation with external input
//! 2. Unsafe format! with user input
//! 3. Unbounded recursion detection
//! 4. Unbounded loop detection
//! 5. Hidden I/O detection
//! 6. @safe/@unsafe annotation validation

use verum_ast::decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, Visibility};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind};
use verum_ast::literal::Literal;
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{Path, PathSegment, Type};
use verum_ast::{Attribute, Ident, Span};
use verum_compiler::meta::linter::{LinterConfig, MetaLinter, UnsafePatternKind};
use verum_common::{List, Maybe};

fn make_ident(name: &str) -> Ident {
    Ident::new(name, Span::default())
}

fn make_path(segments: Vec<&str>) -> Path {
    Path {
        segments: segments
            .into_iter()
            .map(|s| PathSegment::Name(make_ident(s)))
            .collect(),
        span: Span::default(),
    }
}

fn make_path_expr(segments: Vec<&str>) -> Expr {
    Expr::new(ExprKind::Path(make_path(segments)), Span::default())
}

fn make_binary_expr(left: Expr, op: BinOp, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::default(),
    )
}

fn make_call_expr(func_name: Vec<&str>, args: Vec<Expr>) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Box::new(make_path_expr(func_name)),
            args: List::from_iter(args),
            type_args: List::new(),
        },
        Span::default(),
    )
}

fn make_string_literal(s: &str) -> Expr {
    Expr::literal(Literal::string(s.to_string().into(), Span::default()))
}

fn make_bool_literal(b: bool) -> Expr {
    Expr::literal(Literal::bool(b, Span::default()))
}

fn make_empty_block() -> Block {
    Block {
        stmts: List::new(),
        expr: None,
        span: Span::default(),
    }
}

fn make_param(name: &str) -> FunctionParam {
    FunctionParam::new(
        FunctionParamKind::Regular {
            pattern: Pattern::ident(make_ident(name), false, Span::default()),
            ty: Type::inferred(Span::default()),
            default_value: Maybe::None,
        },
        Span::default(),
    )
}

fn make_function(
    name: &str,
    params: Vec<&str>,
    body_expr: Option<Expr>,
    attributes: Vec<Attribute>,
) -> FunctionDecl {
    let params = params.into_iter().map(make_param).collect();

    FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: true,
        stage_level: 1,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: None,
        name: make_ident(name),
        generics: List::new(),
        params,
        throws_clause: None,
        return_type: None,
        std_attr: None,
        contexts: List::new(),
        generic_where_clause: None,
        meta_where_clause: None,
        requires: List::new(),
        ensures: List::new(),
        body: body_expr.map(FunctionBody::Expr),
        attributes: List::from_iter(attributes),
        span: Span::default(),
    }
}

fn make_attr(name: &str) -> Attribute {
    Attribute::simple(name.to_string().into(), Span::default())
}

#[test]
fn test_string_concatenation_with_external_input() {
    let linter = MetaLinter::new();

    // Test: template + args[0] where template is a parameter (external)
    let concat_expr = make_binary_expr(
        make_path_expr(vec!["template"]),
        BinOp::Add,
        make_string_literal("test"),
    );

    let func = make_function(
        "bad_sql",
        vec!["template", "args"],
        Some(concat_expr),
        vec![],
    );

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::StringConcatenation)
    );
}

#[test]
fn test_string_concatenation_with_safe_literals() {
    let linter = MetaLinter::new();

    // Test: "hello" + "world" (both literals, no external input)
    let concat_expr = make_binary_expr(
        make_string_literal("hello"),
        BinOp::Add,
        make_string_literal("world"),
    );

    let func = make_function("safe_concat", vec![], Some(concat_expr), vec![]);

    let result = linter.lint_function(&func);

    // Should be safe - no external input involved
    assert!(
        result.is_safe,
        "Concatenating two string literals should be safe"
    );
}

#[test]
fn test_unsafe_format_with_external_input() {
    let linter = MetaLinter::new();

    // Test: format!(template, user_input) where both are parameters
    let format_expr = make_call_expr(
        vec!["format!"],
        vec![
            make_path_expr(vec!["template"]),
            make_path_expr(vec!["user_input"]),
        ],
    );

    let func = make_function(
        "bad_format",
        vec!["template", "user_input"],
        Some(format_expr),
        vec![],
    );

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::UnsafeFormat)
    );
}

#[test]
fn test_safe_format_with_literals() {
    let linter = MetaLinter::new();

    // Test: format!("Hello {}", "World") - all literals
    let format_expr = make_call_expr(
        vec!["format!"],
        vec![
            make_string_literal("Hello {}"),
            make_string_literal("World"),
        ],
    );

    let func = make_function("safe_format", vec![], Some(format_expr), vec![]);

    let result = linter.lint_function(&func);

    // Should be safe - no external input
    assert!(result.is_safe, "format! with only literals should be safe");
}

#[test]
fn test_unbounded_loop_detection() {
    let linter = MetaLinter::new();

    // Test: loop { } without break
    let loop_expr = Expr::new(
        ExprKind::Loop {
            label: None,
            body: make_empty_block(),
            invariants: List::new(),
        },
        Span::default(),
    );

    let func = make_function("infinite_loop", vec![], Some(loop_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::UnboundedLoop)
    );
}

#[test]
fn test_while_true_without_break() {
    let linter = MetaLinter::new();

    // Test: while true { } without break
    let while_expr = Expr::new(
        ExprKind::While {
            label: None,
            condition: Box::new(make_bool_literal(true)),
            body: make_empty_block(),
            invariants: List::new(),
            decreases: List::new(),
        },
        Span::default(),
    );

    let func = make_function("while_true_infinite", vec![], Some(while_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::UnboundedLoop)
    );
}

#[test]
fn test_hidden_io_file_read() {
    let linter = MetaLinter::new();

    // Test: File.read("path")
    let io_expr = make_call_expr(vec!["File", "read"], vec![make_string_literal("path")]);

    let func = make_function("bad_io", vec![], Some(io_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::HiddenIO)
    );
}

#[test]
fn test_hidden_io_network() {
    let linter = MetaLinter::new();

    // Test: http.get("url")
    let io_expr = make_call_expr(vec!["http", "get"], vec![make_string_literal("url")]);

    let func = make_function("bad_network", vec![], Some(io_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::HiddenIO)
    );
}

#[test]
fn test_panic_unwrap_detection() {
    let linter = MetaLinter::new();

    // Test: some_result.unwrap()
    let unwrap_expr = Expr::new(
        ExprKind::MethodCall {
            receiver: Box::new(make_path_expr(vec!["some_result"])),
            method: make_ident("unwrap"),
            args: List::new(),
            type_args: List::new(),
        },
        Span::default(),
    );

    let func = make_function("bad_unwrap", vec![], Some(unwrap_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::PanicPossible)
    );
}

#[test]
fn test_forbidden_function_panic() {
    let linter = MetaLinter::new();

    // Test: panic!("error")
    let panic_expr = make_call_expr(vec!["panic!"], vec![make_string_literal("error")]);

    let func = make_function("bad_panic", vec![], Some(panic_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::PanicPossible)
    );
}

#[test]
fn test_safe_annotation_validation_passes() {
    let linter = MetaLinter::new();

    // Test: @safe function with safe code
    let safe_attr = make_attr("safe");

    let safe_expr = make_string_literal("hello");
    let func = make_function("safe_func", vec![], Some(safe_expr), vec![safe_attr]);

    let result = linter.lint_function(&func);

    assert!(result.is_safe);
    assert!(result.errors.is_empty());
}

#[test]
fn test_safe_annotation_validation_fails() {
    let linter = MetaLinter::new();

    // Test: @safe function with unsafe code (I/O)
    let safe_attr = make_attr("safe");

    let io_expr = make_call_expr(vec!["File", "read"], vec![make_string_literal("path")]);
    let func = make_function("unsafe_func", vec![], Some(io_expr), vec![safe_attr]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(!result.errors.is_empty());
    assert!(result.errors.iter().any(|e| {
        e.message
            .contains("marked @safe but contains unsafe patterns")
    }));
}

#[test]
fn test_unsafe_annotation_accepted() {
    let linter = MetaLinter::new();

    // Test: @unsafe function with unsafe code (should not error, just mark as unsafe)
    let unsafe_attr = make_attr("unsafe");

    let io_expr = make_call_expr(vec!["File", "read"], vec![make_string_literal("path")]);
    let func = make_function("known_unsafe", vec![], Some(io_expr), vec![unsafe_attr]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    // Should not have validation errors since it's explicitly marked @unsafe
    // It will still have the unsafe pattern detected, but no annotation errors
}

#[test]
fn test_auto_unsafe_marking() {
    let linter = MetaLinter::new();

    // Test: function without annotation but with unsafe code
    let io_expr = make_call_expr(vec!["File", "read"], vec![make_string_literal("path")]);
    let func = make_function("unmarked_unsafe", vec![], Some(io_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(!result.warnings.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.message.contains("automatically marked @unsafe"))
    );
}

#[test]
fn test_require_explicit_safe_config() {
    let mut config = LinterConfig::default();
    config.require_explicit_safe = true;
    let linter = MetaLinter::with_config(config);

    // Test: function without annotation (should error with this config)
    let safe_expr = make_string_literal("hello");
    let func = make_function("no_annotation", vec![], Some(safe_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.errors.is_empty());
    assert!(result.errors.iter().any(|e| {
        e.message
            .contains("requires explicit @safe or @unsafe annotation")
    }));
}

#[test]
fn test_non_deterministic_detection() {
    let linter = MetaLinter::new();

    // Test: value.random() call
    let random_expr = Expr::new(
        ExprKind::MethodCall {
            receiver: Box::new(make_path_expr(vec!["value"])),
            method: make_ident("random"),
            args: List::new(),
            type_args: List::new(),
        },
        Span::default(),
    );

    let func = make_function("non_deterministic", vec![], Some(random_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::NonDeterministic)
    );
}

#[test]
fn test_complex_safe_function() {
    let linter = MetaLinter::new();

    // Test: Complex but safe function - using quote! macro
    let quote_expr = make_call_expr(
        vec!["quote!"],
        vec![make_string_literal("impl Serialize for {}")],
    );

    let func = make_function("safe_derive", vec!["type_info"], Some(quote_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(
        result.is_safe,
        "Functions using safe meta macros like quote! should be safe"
    );
}

#[test]
fn test_external_input_propagation() {
    let linter = MetaLinter::new();

    // Test: let temp = user_input; temp + "suffix"
    // The concatenation should be detected as unsafe because temp is derived from user_input
    let temp_binding = Stmt {
        kind: StmtKind::Let {
            pattern: Pattern {
                kind: PatternKind::Ident {
                    by_ref: false,
                    mutable: false,
                    name: make_ident("temp"),
                    subpattern: None,
                },
                span: Span::default(),
            },
            ty: None,
            value: Some(make_path_expr(vec!["user_input"])),
        },
        span: Span::default(),
        attributes: vec![],
    };

    let concat_expr = make_binary_expr(
        make_path_expr(vec!["temp"]),
        BinOp::Add,
        make_string_literal("suffix"),
    );

    let block = Block {
        stmts: List::from_iter(vec![temp_binding]),
        expr: Some(Box::new(concat_expr)),
        span: Span::default(),
    };

    let func = FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: true,
        stage_level: 1,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: None,
        name: make_ident("propagation_test"),
        generics: List::new(),
        params: List::from_iter(vec![make_param("user_input")]),
        throws_clause: None,
        return_type: None,
        std_attr: None,
        contexts: List::new(),
        generic_where_clause: None,
        meta_where_clause: None,
        requires: List::new(),
        ensures: List::new(),
        body: Some(FunctionBody::Block(block)),
        attributes: List::new(),
        span: Span::default(),
    };

    let result = linter.lint_function(&func);

    assert!(
        !result.is_safe,
        "String concatenation with propagated external input should be unsafe"
    );
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::StringConcatenation)
    );
}

// ============================================================================
// Security Pattern Detection Tests
// ============================================================================

#[test]
fn test_sql_injection_detection() {
    let linter = MetaLinter::new();

    // Test: db.query(user_input) where user_input is a parameter
    let query_expr = make_call_expr(
        vec!["db", "query"],
        vec![make_path_expr(vec!["user_input"])],
    );

    let func = make_function("bad_query", vec!["user_input"], Some(query_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::SqlInjection),
        "Should detect SQL injection with external input in query"
    );
}

#[test]
fn test_sql_parameterized_query_safe() {
    let linter = MetaLinter::new();

    // Test: db.query("SELECT * FROM users") with literal string (no external input)
    let query_expr = make_call_expr(
        vec!["db", "query"],
        vec![make_string_literal("SELECT * FROM users")],
    );

    let func = make_function("safe_query", vec![], Some(query_expr), vec![]);

    let result = linter.lint_function(&func);

    // Should not flag SQL injection for literal-only queries
    assert!(
        !result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::SqlInjection),
        "Parameterized queries with literals should be safe"
    );
}

#[test]
fn test_command_injection_detection() {
    let linter = MetaLinter::new();

    // Test: process.exec(command) where command is a parameter
    let exec_expr = make_call_expr(
        vec!["process", "exec"],
        vec![make_path_expr(vec!["command"])],
    );

    let func = make_function("bad_exec", vec!["command"], Some(exec_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::CommandInjection),
        "Should detect command injection with external input"
    );
}

#[test]
fn test_dynamic_code_execution_detection() {
    let linter = MetaLinter::new();

    // Test: eval(code) - always dangerous
    let eval_expr = make_call_expr(vec!["eval"], vec![make_string_literal("print(1)")]);

    let func = make_function("bad_eval", vec![], Some(eval_expr), vec![]);

    let result = linter.lint_function(&func);

    assert!(!result.is_safe);
    assert!(
        result
            .unsafe_patterns
            .iter()
            .any(|p| p.kind == UnsafePatternKind::DynamicCodeExecution),
        "Should detect dynamic code execution via eval"
    );
}

#[test]
fn test_cwe_id_mapping() {
    // Verify CWE IDs are correctly mapped
    assert_eq!(UnsafePatternKind::SqlInjection.cwe_id(), Some(89));
    assert_eq!(UnsafePatternKind::CommandInjection.cwe_id(), Some(78));
    assert_eq!(UnsafePatternKind::PathTraversal.cwe_id(), Some(22));
    assert_eq!(UnsafePatternKind::DynamicCodeExecution.cwe_id(), Some(94));
    assert_eq!(UnsafePatternKind::UnsafeFormat.cwe_id(), Some(134));
    assert_eq!(UnsafePatternKind::SensitiveDataExposure.cwe_id(), Some(200));
    assert_eq!(UnsafePatternKind::UnsafeMemory.cwe_id(), Some(119));
    assert_eq!(UnsafePatternKind::UnboundedLoop.cwe_id(), None); // Not a security issue
}

#[test]
fn test_security_issue_classification() {
    // Verify security classification
    assert!(UnsafePatternKind::SqlInjection.is_security_issue());
    assert!(UnsafePatternKind::CommandInjection.is_security_issue());
    assert!(UnsafePatternKind::PathTraversal.is_security_issue());
    assert!(UnsafePatternKind::DynamicCodeExecution.is_security_issue());
    assert!(UnsafePatternKind::SensitiveDataExposure.is_security_issue());
    assert!(UnsafePatternKind::StringConcatenation.is_security_issue());
    assert!(UnsafePatternKind::UnsafeFormat.is_security_issue());
    assert!(UnsafePatternKind::UnsafeMemory.is_security_issue());

    // Non-security issues
    assert!(!UnsafePatternKind::UnboundedLoop.is_security_issue());
    assert!(!UnsafePatternKind::PanicPossible.is_security_issue());
}

#[test]
fn test_severity_levels() {
    use verum_diagnostics::Severity;

    // Critical security issues should be errors
    assert_eq!(UnsafePatternKind::SqlInjection.severity(), Severity::Error);
    assert_eq!(
        UnsafePatternKind::CommandInjection.severity(),
        Severity::Error
    );
    assert_eq!(UnsafePatternKind::PathTraversal.severity(), Severity::Error);
    assert_eq!(
        UnsafePatternKind::DynamicCodeExecution.severity(),
        Severity::Error
    );

    // Medium severity should be warnings
    assert_eq!(
        UnsafePatternKind::StringConcatenation.severity(),
        Severity::Warning
    );
    assert_eq!(
        UnsafePatternKind::UnboundedLoop.severity(),
        Severity::Warning
    );
}

#[test]
fn test_cyclomatic_complexity_calculation() {
    let linter = MetaLinter::new();

    // Simple function with no branches - complexity should be 1
    let simple_func = make_function("simple", vec![], Some(make_string_literal("hello")), vec![]);

    let complexity = linter.calculate_complexity(&simple_func);
    assert_eq!(complexity, 1, "Simple function should have complexity 1");
}

#[test]
fn test_lint_emits_complexity_warning_when_threshold_exceeded() {
    let mut config = LinterConfig::default();
    config.check_performance = true;
    config.max_cyclomatic_complexity = 0;

    let linter = MetaLinter::with_config(config);
    let func = make_function("simple", vec![], Some(make_string_literal("hi")), vec![]);

    let result = linter.lint_function(&func);
    let complexity_warnings = result
        .warnings
        .iter()
        .filter(|w| w.message.as_str().contains("cyclomatic complexity"))
        .count();
    assert_eq!(
        complexity_warnings, 1,
        "expected one complexity warning when complexity (1) > threshold (0)"
    );
}

#[test]
fn test_lint_skips_complexity_when_within_threshold() {
    let mut config = LinterConfig::default();
    config.check_performance = true;
    config.max_cyclomatic_complexity = 10;

    let linter = MetaLinter::with_config(config);
    let func = make_function("simple", vec![], Some(make_string_literal("hi")), vec![]);

    let result = linter.lint_function(&func);
    let complexity_warnings = result
        .warnings
        .iter()
        .filter(|w| w.message.as_str().contains("cyclomatic complexity"))
        .count();
    assert_eq!(
        complexity_warnings, 0,
        "no complexity warning when complexity (1) <= threshold (10)"
    );
}

#[test]
fn test_lint_skips_complexity_when_check_performance_disabled() {
    let mut config = LinterConfig::default();
    config.check_performance = false;
    config.max_cyclomatic_complexity = 0;

    let linter = MetaLinter::with_config(config);
    let func = make_function("simple", vec![], Some(make_string_literal("hi")), vec![]);

    let result = linter.lint_function(&func);
    let complexity_warnings = result
        .warnings
        .iter()
        .filter(|w| w.message.as_str().contains("cyclomatic complexity"))
        .count();
    assert_eq!(
        complexity_warnings, 0,
        "no complexity warning when check_performance is disabled, even if threshold is 0"
    );
}
