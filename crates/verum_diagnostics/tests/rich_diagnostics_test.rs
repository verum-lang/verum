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
//! Comprehensive tests for rich diagnostics system.

use verum_diagnostics::{
    ColorScheme, Diagnostic, DiagnosticBuilder, GlyphSet, RichRenderConfig, RichRenderer,
    SnippetExtractor, Span, Text, get_explanation, list_error_codes, render_explanation, search_errors,
};

/// Create a test span
fn test_span(line: usize, column: usize, length: usize) -> Span {
    Span {
        file: "test.vr".into(),
        line,
        column,
        end_line: Some(line),
        end_column: column + length,
    }
}

/// Create test source content
fn test_source() -> &'static str {
    r#"fn main() {
    let x: Positive = -5;
    println!(x);
}
"#
}

#[test]
fn test_basic_error_rendering() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(test_span(2, 23, 2), "value `-5` fails constraint `> 0`")
        .add_note("value has type `Int` but requires `Positive`")
        .help("use runtime check: `Positive::try_from(-5)?`")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0312]"));
    assert!(output.contains("refinement constraint not satisfied"));
    assert!(output.contains("test.vr:2:23"));
    assert!(output.contains("let x: Positive = -5;"));
    assert!(output.contains("note:"));
    assert!(output.contains("help:"));
}

#[test]
fn test_multi_line_span() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    let source = r#"fn test() {
let result = if condition {
    value_a
} else {
    value_b
};
}"#;
    renderer.add_test_content("test.vr", source);

    let span = Span {
        file: "test.vr".into(),
        line: 2,
        column: 14,
        end_line: Some(6),
        end_column: 2,
    };

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("type mismatch")
        .span_label(span, "expected `Int`, found `Float`")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("type mismatch"));
    assert!(output.contains("if condition"));
}

#[test]
fn test_colored_output() {
    let mut renderer = RichRenderer::default();
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(test_span(2, 23, 2), "fails constraint")
        .build();

    let output = renderer.render(&diagnostic);

    // Should contain ANSI escape codes when colors are enabled
    // (depending on terminal detection)
    assert!(output.contains("error"));
    assert!(output.contains("[E0312]"));
}

#[test]
fn test_no_color_mode() {
    let config = RichRenderConfig::no_color();
    let mut renderer = RichRenderer::new(config);
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("test error")
        .span_label(test_span(2, 23, 2), "test label")
        .build();

    let output = renderer.render(&diagnostic);

    // Should NOT contain ANSI codes
    assert!(!output.contains("\x1b["));
}

#[test]
fn test_minimal_config() {
    let config = RichRenderConfig::minimal();
    let mut renderer = RichRenderer::new(config);
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("test error")
        .span_label(test_span(2, 23, 2), "test label")
        .build();

    let output = renderer.render(&diagnostic);

    // Should be more compact (no source shown)
    assert!(output.contains("error[E0312]"));
    assert!(!output.contains("let x: Positive"));
}

#[test]
fn test_nested_diagnostics() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    renderer.add_test_content("test.vr", test_source());

    let child = DiagnosticBuilder::note_diag()
        .message("this is a related note")
        .build();

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(test_span(2, 23, 2), "fails constraint")
        .child(child)
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0312]"));
    assert!(output.contains("note: this is a related note"));
}

#[test]
fn test_warning_rendering() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::warning()
        .code("W0101")
        .message("unused variable")
        .span_label(test_span(2, 8, 1), "unused variable `x`")
        .help("prefix with underscore: `_x`")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("warning[W0101]"));
    assert!(output.contains("unused variable"));
}

#[test]
fn test_snippet_extractor() {
    let mut extractor = SnippetExtractor::new();
    extractor.add_source(std::path::PathBuf::from("test.vr"), test_source());

    let span = test_span(2, 23, 2);
    let snippet = extractor
        .extract_snippet(
            std::path::Path::new("test.vr"),
            &span,
            1, // context lines
        )
        .unwrap();

    assert_eq!(snippet.lines.len(), 3); // Line 1, 2, 3
    assert_eq!(snippet.start_line, 1);
    assert_eq!(snippet.end_line, 3);
}

#[test]
fn test_snippet_extractor_no_context() {
    let mut extractor = SnippetExtractor::new();
    extractor.add_source(std::path::PathBuf::from("test.vr"), test_source());

    let span = test_span(2, 23, 2);
    let snippet = extractor
        .extract_snippet(
            std::path::Path::new("test.vr"),
            &span,
            0, // no context
        )
        .unwrap();

    assert_eq!(snippet.lines.len(), 1); // Only line 2
}

#[test]
fn test_snippet_cache() {
    let mut extractor = SnippetExtractor::with_cache_size(3);

    // Add 4 files - cache stores all added sources without eviction
    // (eviction happens on cache hits, not on add_source)
    for i in 0..4 {
        let path = std::path::PathBuf::from(format!("test{}.vr", i));
        extractor.add_source(path, "fn main() {}");
    }

    let stats = extractor.cache_stats();
    // Check that cache tracking works
    assert!(stats.cached_files > 0);
    assert!(stats.max_capacity == 3);
}

#[test]
fn test_color_scheme_auto() {
    let scheme = ColorScheme::auto();
    // Should create a scheme (whether colored or not depends on environment)
    let text = scheme.colorize("test", &scheme.error_code);
    assert!(!text.is_empty());
}

#[test]
fn test_color_scheme_no_color() {
    let scheme = ColorScheme::no_color();
    let text = scheme.colorize("test", &scheme.error_code);
    assert_eq!(text, "test");
    assert!(!text.contains("\x1b["));
}

#[test]
fn test_glyph_set_unicode() {
    let glyphs = GlyphSet::unicode();
    assert_eq!(glyphs.horizontal_line, "─");
    assert_eq!(glyphs.vertical_line, "│");
    assert_eq!(glyphs.arrow_right, "→");
}

#[test]
fn test_glyph_set_ascii() {
    let glyphs = GlyphSet::ascii();
    assert_eq!(glyphs.horizontal_line, "-");
    assert_eq!(glyphs.vertical_line, "|");
    assert_eq!(glyphs.arrow_right, "->");
}

#[test]
fn test_explanation_e0312() {
    let explanation = get_explanation("E0312").unwrap();
    assert_eq!(explanation.code, "E0312");
    assert!(explanation.title.contains("Refinement"));
    assert!(!explanation.examples.is_empty());
    assert!(!explanation.solutions.is_empty());
    assert!(!explanation.see_also.is_empty());
}

#[test]
fn test_explanation_e0308() {
    let explanation = get_explanation("E0308").unwrap();
    assert_eq!(explanation.code, "E0308");
    // E0308 is "Capability not provided" - context system error
    assert!(explanation.title.contains("Capability"));
}

#[test]
fn test_explanation_e0310() {
    let explanation = get_explanation("E0310").unwrap();
    assert_eq!(explanation.code, "E0310");
    assert!(explanation.title.contains("array access"));
}

#[test]
fn test_explanation_not_found() {
    let explanation = get_explanation("E9999");
    assert!(explanation.is_none());
}

#[test]
fn test_render_explanation_no_color() {
    let explanation = get_explanation("E0312").unwrap();
    let rendered = render_explanation(explanation, false);

    assert!(rendered.contains("E0312"));
    assert!(rendered.contains("Refinement"));
    assert!(rendered.contains("Examples"));
    assert!(rendered.contains("Solutions"));
    assert!(rendered.contains("See Also"));
    assert!(!rendered.contains("\x1b[")); // No ANSI codes
}

#[test]
fn test_render_explanation_with_color() {
    let explanation = get_explanation("E0312").unwrap();
    let rendered = render_explanation(explanation, true);

    assert!(rendered.contains("E0312"));
    assert!(rendered.contains("Refinement"));
}

#[test]
fn test_list_error_codes() {
    let codes = list_error_codes();
    assert!(!codes.is_empty());
    assert!(codes.len() >= 10);
    assert!(codes.contains(&Text::from("E0312")));
    assert!(codes.contains(&Text::from("E0308")));
    assert!(codes.contains(&Text::from("E0310")));
}

#[test]
fn test_search_errors_refinement() {
    let results = search_errors("refinement");
    assert!(!results.is_empty());
    assert!(results.contains(&Text::from("E0312")));
}

#[test]
fn test_search_errors_case_insensitive() {
    let results1 = search_errors("refinement");
    let results2 = search_errors("REFINEMENT");
    assert_eq!(results1, results2);
}

#[test]
fn test_search_errors_array() {
    let results = search_errors("array");
    assert!(!results.is_empty());
    assert!(results.contains(&Text::from("E0310")));
}

#[test]
fn test_search_errors_no_results() {
    let results = search_errors("nonexistent_term_xyz");
    assert!(results.is_empty());
}

#[test]
fn test_all_explanations_have_examples() {
    for code in list_error_codes() {
        if let Some(explanation) = get_explanation(&code) {
            assert!(
                !explanation.examples.is_empty(),
                "Error code {} should have examples",
                code
            );
        }
    }
}

#[test]
fn test_all_explanations_have_solutions() {
    for code in list_error_codes() {
        if let Some(explanation) = get_explanation(&code) {
            assert!(
                !explanation.solutions.is_empty(),
                "Error code {} should have solutions",
                code
            );
        }
    }
}

#[test]
fn test_all_explanations_have_descriptions() {
    for code in list_error_codes() {
        if let Some(explanation) = get_explanation(&code) {
            assert!(
                !explanation.description.is_empty(),
                "Error code {} should have a description",
                code
            );
        }
    }
}

#[test]
fn test_context_error_codes() {
    // E0301: Context not declared
    let e0301 = get_explanation("E0301").unwrap();
    assert!(e0301.title.contains("Context"));

    // E0313: Integer overflow
    let e0313 = get_explanation("E0313").unwrap();
    assert!(e0313.title.contains("overflow"));

    // E0314: Division by zero
    let e0314 = get_explanation("E0314").unwrap();
    assert!(e0314.title.contains("Division"));
}

#[test]
fn test_must_handle_error_e0317() {
    let e0317 = get_explanation("E0317").unwrap();
    assert_eq!(e0317.code, "E0317");
    assert!(e0317.title.contains("must be handled"));
}

#[test]
fn test_try_operator_error_e0203() {
    let e0203 = get_explanation("E0203").unwrap();
    assert_eq!(e0203.code, "E0203");
    assert!(e0203.title.contains("Result type mismatch"));
}

#[test]
fn test_branch_verification_error_e0309() {
    let e0309 = get_explanation("E0309").unwrap();
    assert_eq!(e0309.code, "E0309");
    assert!(e0309.title.contains("Branch"));
}

#[test]
fn test_resource_consumed_error_e0316() {
    let e0316 = get_explanation("E0316").unwrap();
    assert_eq!(e0316.code, "E0316");
    assert!(e0316.title.contains("Resource"));
}

#[test]
fn test_diagnostic_without_source() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    // Don't add source content

    let diagnostic = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(test_span(2, 23, 2), "fails constraint")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0312]"));
    // Should handle missing source gracefully
    assert!(output.contains("source file not available") || !output.contains("let x:"));
}

#[test]
fn test_multiple_labels() {
    let mut renderer = RichRenderer::new(RichRenderConfig::no_color());
    renderer.add_test_content("test.vr", test_source());

    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message("type mismatch")
        .span_label(test_span(2, 8, 1), "variable declared here")
        .secondary_span(test_span(2, 23, 2), "incompatible type")
        .build();

    let output = renderer.render(&diagnostic);

    assert!(output.contains("error[E0308]"));
    assert!(output.contains("type mismatch"));
}

#[test]
fn test_unicode_vs_ascii_glyphs() {
    let unicode = GlyphSet::unicode();
    let ascii = GlyphSet::ascii();

    // Unicode uses box drawing characters
    assert_ne!(unicode.vertical_line, ascii.vertical_line);
    assert_ne!(unicode.arrow_right, ascii.arrow_right);

    // ASCII uses simple characters
    assert_eq!(ascii.vertical_line, "|");
    assert_eq!(ascii.arrow_right, "->");
}

#[test]
fn test_render_config_presets() {
    let default_config = RichRenderConfig::default();
    assert!(default_config.show_source);
    assert!(default_config.show_line_numbers);
    assert_eq!(default_config.context_lines, 2);

    let no_color_config = RichRenderConfig::no_color();
    assert!(no_color_config.show_source);

    let minimal_config = RichRenderConfig::minimal();
    assert!(!minimal_config.show_source);
    assert_eq!(minimal_config.context_lines, 0);
}

#[test]
fn test_long_line_truncation() {
    let config = RichRenderConfig {
        max_line_width: Some(20),
        ..RichRenderConfig::no_color()
    };
    let mut renderer = RichRenderer::new(config);

    let long_source = "fn main() { let x = very_long_variable_name_that_exceeds_limit; }";
    renderer.add_test_content("test.vr", long_source);

    let diagnostic = DiagnosticBuilder::error()
        .code("E0001")
        .message("test")
        .span_label(test_span(1, 10, 5), "test")
        .build();

    let output = renderer.render(&diagnostic);

    // Should contain truncation indicator
    assert!(output.contains("...") || output.len() < long_source.len());
}
