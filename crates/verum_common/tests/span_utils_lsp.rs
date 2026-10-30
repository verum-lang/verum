#![cfg(feature = "lsp")]

use verum_common::span::{FileId, LineColSpan};
use verum_common::span_utils::{lsp::*, offset_to_line_col};

#[test]
fn utf16_positions_round_trip_and_reject_split_surrogates() {
    let text = "a😀я\n尾";
    for (byte, position) in [
        (0, (0, 0)),
        (1, (0, 1)),
        (5, (0, 3)),
        (7, (0, 4)),
        (8, (1, 0)),
        (11, (1, 1)),
    ] {
        let position = Position::new(position.0, position.1);
        assert_eq!(offset_to_lsp_position(byte, text), position);
        assert_eq!(lsp_position_to_offset(position, text), Some(byte));
    }
    assert_eq!(lsp_position_to_offset(Position::new(0, 2), text), None);
    assert_eq!(lsp_position_to_offset(Position::new(0, 99), text), Some(7));
    assert_eq!(lsp_position_to_offset(Position::new(2, 0), text), None);
    assert_eq!(offset_to_line_col(7, text), (0, 3));
}

#[test]
fn lsp_line_endings_and_empty_final_line() {
    let text = "😀\r\nx\ry\n";
    for (byte, line) in [(0, 0), (6, 1), (8, 2), (10, 3)] {
        assert_eq!(
            lsp_position_to_offset(Position::new(line, 0), text),
            Some(byte)
        );
        assert_eq!(offset_to_lsp_position(byte, text), Position::new(line, 0));
    }
    assert_eq!(lsp_position_to_offset(Position::new(0, 20), text), Some(4));
    assert_eq!(offset_to_lsp_position(5, text), Position::new(0, 2));
    assert_eq!(lsp_position_to_offset(Position::new(0, 0), ""), Some(0));
}

#[test]
fn reversed_range_is_refused() {
    assert!(
        lsp_range_to_span(
            Range::new(Position::new(0, 2), Position::new(0, 1)),
            "abc",
            FileId::new(0)
        )
        .is_none()
    );
}

#[test]
fn diagnostic_scalar_columns_are_translated_using_source() {
    let span = LineColSpan::new("test.vr", 1, 3, 4);
    assert_eq!(
        line_col_span_to_lsp_range_with_text(&span, "a😀я"),
        Range::new(Position::new(0, 3), Position::new(0, 4))
    );
}
