use tower_lsp::lsp_types::{Position, Range, TextEdit};
use verum_common::span_utils::lsp::lsp_position_to_offset;
use verum_lsp::{
    cli_format::diff_to_text_edits,
    formatting::{format_document, format_range},
};

fn apply(source: &str, edit: &TextEdit) -> String {
    let start = lsp_position_to_offset(edit.range.start, source).expect("valid start");
    let end = lsp_position_to_offset(edit.range.end, source).expect("valid end");
    format!("{}{}{}", &source[..start], edit.new_text, &source[end..])
}

#[test]
fn full_document_edits_have_real_utf16_endpoints() {
    for source in [
        "",
        "// Ω😀",
        "// Ω😀\r\n",
        "fn f(){}\rnext",
        "fn f(){}\nnext",
    ] {
        let edits = diff_to_text_edits(source, "replacement\n");
        assert_eq!(edits.len(), 1);
        assert_eq!(apply(source, &edits[0]), "replacement\n");
        for edit in format_document(source) {
            let _ = apply(source, &edit);
        }
    }
}

#[test]
fn partial_line_selection_does_not_duplicate_the_unselected_columns() {
    for newline in ["\n", "\r\n", "\r"] {
        let source = format!("// before{newline}   // Ω😀 selected   {newline}// after");
        let range = Range::new(Position::new(1, 6), Position::new(1, 10));
        let edits = format_range(&source, range);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(1, 0));
        assert_eq!(
            apply(&source, &edits[0]),
            format!("// before{newline}// Ω😀 selected{newline}// after")
        );
    }
}

#[test]
fn exclusive_end_line_and_invalid_ranges_are_respected() {
    let source = "  // selected\n  // untouched\n";
    let edits = format_range(source, Range::new(Position::new(0, 1), Position::new(1, 0)));
    assert_eq!(apply(source, &edits[0]), "// selected\n  // untouched\n");
    for range in [
        Range::new(Position::new(1, 2), Position::new(0, 1)),
        Range::new(Position::new(4, 0), Position::new(5, 0)),
        Range::new(Position::new(0, 0), Position::new(4, 0)),
    ] {
        assert!(format_range(source, range).is_empty());
    }
    assert!(
        format_range(
            "// 😀",
            Range::new(Position::new(0, 4), Position::new(0, 5))
        )
        .is_empty()
    );
}
