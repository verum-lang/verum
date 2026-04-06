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
//! Comprehensive tests for multi-snippet error rendering
//!
//! Tests the ability to render diagnostics with multiple locations,
//! including primary and secondary labels across different files.

use verum_diagnostics::{
    Diagnostic, DiagnosticBuilder, RichRenderConfig, RichRenderer, Severity, Span, SpanLabel,
};

/// Helper to create a span
fn create_span(file: &str, line: usize, column: usize, end_line: usize, end_column: usize) -> Span {
    Span {
        file: file.into(),
        line,
        column,
        end_line: Some(end_line),
        end_column,
    }
}

/// Test: Single file, multiple primary labels (close together - merged)
#[test]

fn test_multi_snippet_same_file_merged() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let source = "fn calculate(x: Int, y: Int) -> Int {\n    let result = x + y;\n    result\n}";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("type mismatch in function")
        .span_label(create_span("test.vr", 1, 14, 1, 15), "parameter x")
        .span_label(create_span("test.vr", 1, 22, 1, 23), "parameter y")
        .build();

    let output = renderer.render(&diagnostic);

    // Should show both parameters in same snippet
    assert!(output.contains("error[E0312]"));
    assert!(output.contains("type mismatch in function"));
    assert!(output.contains("parameter x"));
    assert!(output.contains("parameter y"));

    // Should only show the function signature once
    let line_count = output.matches("fn calculate").count();
    assert_eq!(
        line_count, 1,
        "Function signature should appear once in merged view"
    );
}

/// Test: Single file, multiple labels far apart (separate snippets)
#[test]
fn test_multi_snippet_same_file_separated() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    // Create source with labels very far apart (>20 lines) to force separation
    let mut lines = vec!["fn main() {".to_string()];
    lines.push("    let x = 10;".to_string()); // Line 2
    for _ in 0..25 {
        lines.push("    // padding".to_string());
    }
    lines.push("    let y = x + 5;".to_string()); // Line 28
    lines.push("    println!(y);".to_string());
    lines.push("}".to_string());
    let source = lines.join("\n");
    renderer.add_test_content("test.vr", &source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("value used after move")
        .span_label(create_span("test.vr", 2, 9, 2, 10), "value moved here")
        .span_label(
            create_span("test.vr", 28, 13, 28, 14),
            "value used here after move",
        )
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("value moved here"));
    assert!(output.contains("value used here after move"));

    // When labels are far apart, either "..." separator or "(showing multiple locations)" should appear
    assert!(
        output.contains("...") || output.contains("(showing multiple locations)"),
        "Expected separator for distant labels. Got:\n{}",
        output
    );
}

/// Test: Multiple files (primary in first, secondary in second)
#[test]

fn test_multi_snippet_multiple_files() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    renderer.add_test_content(
        "main.vr",
        "fn main() {\n    let db = connect();\n    process(db);\n}",
    );
    renderer.add_test_content(
        "lib.vr",
        "fn connect() -> Database {\n    Database.new()\n}",
    );

    let diagnostic = DiagnosticBuilder::error()
        .code("E0311")
        .message("type mismatch")
        .span_label(
            create_span("main.vr", 3, 13, 3, 15),
            "expected String, found Database",
        )
        .secondary_span(
            create_span("lib.vr", 1, 17, 1, 25),
            "Database type defined here",
        )
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0311]"));
    assert!(output.contains("main.vr"));
    assert!(output.contains("lib.vr"));
    assert!(output.contains("expected String, found Database"));
    assert!(output.contains("Database type defined here"));
}

/// Test: Primary and secondary labels in same file
#[test]

fn test_multi_snippet_primary_and_secondary() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let source = "struct User {\n    name: Text,\n    age: Int,\n}\n\nfn create_user() -> User {\n    User {\n        name: \"Alice\",\n        age: \"25\",  // Error: should be Int\n    }\n}";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("mismatched types")
        .span_label(
            create_span("test.vr", 9, 14, 9, 18),
            "expected Int, found Text",
        )
        .secondary_span(
            create_span("test.vr", 3, 10, 3, 13),
            "expected due to this field type",
        )
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("expected Int, found Text"));
    assert!(output.contains("expected due to this field type"));
}

/// Test: Multi-line span with secondary label
#[test]

fn test_multi_snippet_multiline_span() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let source = "fn calculate() {\n    let result = if condition {\n        value_a\n    } else {\n        value_b\n    };\n    result\n}";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("match arms have incompatible types")
        .span_label(
            create_span("test.vr", 2, 18, 6, 6),
            "if and else have incompatible types",
        )
        .secondary_span(create_span("test.vr", 3, 9, 3, 16), "expected Int")
        .secondary_span(create_span("test.vr", 5, 9, 5, 16), "found Text")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("match arms have incompatible types"));
}

/// Test: Empty message labels (underline only)
#[test]

fn test_multi_snippet_empty_messages() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let source = "let x = 10 + \"hello\";";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("cannot add Int and Text")
        .span_label(create_span("test.vr", 1, 9, 1, 11), "")
        .span_label(create_span("test.vr", 1, 14, 1, 21), "")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("cannot add Int and Text"));
}

/// Test: Three files with multiple labels each
#[test]

fn test_multi_snippet_three_files() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    renderer.add_test_content("a.vr", "type UserId = Int;");
    renderer.add_test_content("b.vr", "type OrderId = Text;");
    renderer.add_test_content("c.vr", "fn process(id: UserId) {\n    // error\n}");

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("type mismatch across modules")
        .span_label(
            create_span("c.vr", 1, 16, 1, 22),
            "parameter expects UserId",
        )
        .secondary_span(create_span("a.vr", 1, 6, 1, 12), "UserId defined as Int")
        .secondary_span(create_span("b.vr", 1, 6, 1, 13), "OrderId defined as Text")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0312]"));
    assert!(output.contains("a.vr"));
    assert!(output.contains("b.vr"));
    assert!(output.contains("c.vr"));
}

/// Test: Context lines work correctly with multiple snippets
#[test]

fn test_multi_snippet_context_lines() {
    let config = RichRenderConfig {
        context_lines: 0,
        ..RichRenderConfig::no_color()
    };
    let mut renderer = RichRenderer::new(config);

    let source = "line1\nline2\nerror here\nline4\nline5";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test error")
        .span_label(create_span("test.vr", 3, 1, 3, 5), "error")
        .build();

    let output = renderer.render(&diagnostic);

    // With 0 context lines, should only show the error line
    assert!(!output.contains("line1"));
    assert!(!output.contains("line2"));
    assert!(output.contains("error here"));
    assert!(!output.contains("line4"));
    assert!(!output.contains("line5"));
}

/// Test: Colored output contains ANSI codes
#[test]
fn test_multi_snippet_with_colors() {
    use verum_diagnostics::ColorScheme;

    // Force colors on (don't auto-detect, since tests run without TTY)
    let config = RichRenderConfig {
        color_scheme: ColorScheme::default_colors(),
        ..RichRenderConfig::default()
    };
    let mut renderer = RichRenderer::new(config);

    let source = "let x = 10;\nlet y = 20;";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test")
        .span_label(create_span("test.vr", 1, 5, 1, 6), "x")
        .span_label(create_span("test.vr", 2, 5, 2, 6), "y")
        .build();

    let output = renderer.render(&diagnostic);

    // Should contain ANSI escape codes
    assert!(
        output.contains("\x1b["),
        "Expected ANSI codes in output. Got:\n{}",
        output
    );
}

/// Test: Very long line gets truncated properly
#[test]

fn test_multi_snippet_line_truncation() {
    let config = RichRenderConfig {
        max_line_width: Some(40),
        ..RichRenderConfig::no_color()
    };
    let mut renderer = RichRenderer::new(config);

    let source = "let very_long_variable_name_that_exceeds_max_width = 12345;";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test")
        .span_label(create_span("test.vr", 1, 5, 1, 10), "truncated")
        .build();

    let output = renderer.render(&diagnostic);

    // Should contain truncation indicator
    assert!(output.contains("..."));
}

/// Test: Labels at exact same position
#[test]

fn test_multi_snippet_overlapping_labels() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let source = "let x = getValue();";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("multiple issues")
        .span_label(create_span("test.vr", 1, 9, 1, 17), "first issue")
        .span_label(create_span("test.vr", 1, 9, 1, 17), "second issue")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("first issue"));
    assert!(output.contains("second issue"));
}

/// Test: File not found error handling
#[test]

fn test_multi_snippet_file_not_found() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    // Don't add test content, so file won't be found
    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test")
        .span_label(create_span("missing.vr", 1, 1, 1, 5), "error")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0001]"));
    assert!(output.contains("source file not available") || output.contains("missing.vr"));
}

/// Test: Benchmark - many labels in same file
#[test]

fn test_multi_snippet_performance_many_labels() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());

    let mut source = String::new();
    for i in 1..=50 {
        source.push_str(&format!("let var{} = {};\n", i, i));
    }
    renderer.add_test_content("test.vr", &source);

    let mut builder = DiagnosticBuilder::error()
        .code("E0001")
        .message("many errors");

    // Add label for every other line
    for i in (1..=50).step_by(2) {
        builder = builder.span_label(
            create_span("test.vr", i, 5, i, 9),
            format!("error on line {}", i),
        );
    }

    let diagnostic = builder.build();
    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0001]"));
    // With many labels far apart, should show separation
    assert!(output.len() > 1000); // Should be substantial output
}

/// Test: Minimal config produces compact output
#[test]

fn test_multi_snippet_minimal_config() {
    let config = RichRenderConfig::minimal();
    let mut renderer = RichRenderer::new(config);

    let source = "let x = 10;\nlet y = 20;";
    renderer.add_test_content("test.vr", source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test")
        .span_label(create_span("test.vr", 1, 5, 1, 6), "x")
        .span_label(create_span("test.vr", 2, 5, 2, 6), "y")
        .build();

    let output = renderer.render(&diagnostic);

    // Should be compact (no source shown in minimal mode)
    assert!(output.len() < 200);
    assert!(output.contains("error[E0001]"));
}
