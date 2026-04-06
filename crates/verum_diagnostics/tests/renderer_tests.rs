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
// Unit tests for renderer.rs
//
// Migrated from src/renderer.rs to comply with CLAUDE.md test organization.

use verum_diagnostics::{
    DiagnosticBuilder, Span,
    renderer::{RenderConfig, Renderer},
};

#[test]
fn test_render_config() {
    let config = RenderConfig::default();
    assert!(config.colors);
    assert_eq!(config.context_lines, 2);
    assert!(config.show_line_numbers);
}

#[test]
fn test_no_color_config() {
    let config = RenderConfig::no_color();
    assert!(!config.colors);
}

#[test]
fn test_minimal_config() {
    let config = RenderConfig::minimal();
    assert_eq!(config.context_lines, 0);
    assert!(!config.show_source);
}

#[test]
fn test_basic_rendering() {
    let mut renderer = Renderer::new(RenderConfig::no_color());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("refinement constraint not satisfied")
        .build();

    let output = renderer.render(&diagnostic);
    assert!(output.contains("error<E0308>"));
    assert!(output.contains("refinement constraint not satisfied"));
}

#[test]
fn test_rendering_with_notes() {
    let mut renderer = Renderer::new(RenderConfig::no_color());

    let diagnostic = DiagnosticBuilder::error()
        .message("error message")
        .add_note("this is a note")
        .help("this is a help message")
        .build();

    let output = renderer.render(&diagnostic);
    assert!(output.contains("this is a note"));
    assert!(output.contains("this is a help message"));
}

#[test]
fn test_source_snippet_rendering() {
    let mut renderer = Renderer::new(RenderConfig::no_color());

    // Add test content
    renderer.add_test_content(
        "test.vr",
        "fn main() {\n    let x = -5;\n    divide(10, x)\n}\n",
    );

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(
            Span::new("test.vr", 3, 16, 17),
            "value `-5` fails constraint `x > 0`",
        )
        .build();

    let output = renderer.render(&diagnostic);
    assert!(output.contains("test.vr:3:16"));
    assert!(output.contains("divide(10, x)"));
    assert!(output.contains("value `-5` fails constraint"));
}
