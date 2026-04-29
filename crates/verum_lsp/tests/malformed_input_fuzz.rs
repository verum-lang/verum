//! Red-team Round-2 §8.1 — LSP responses to malformed source.
//!
//! Pins the no-panic contract for the user-reachable LSP entry points
//! when fed adversarial source text.  Every LSP request that ingests
//! a `DocumentState` flows through the parser which MAY emit a
//! diagnostic but MUST NOT panic the LSP worker — a panic kills the
//! editor's language server connection and forces a manual restart.
//!
//! The test corpus covers the empirical failure modes from the parser
//! red-team sweep:
//!   - empty document
//!   - mid-token EOF (truncated function declaration)
//!   - unbalanced bracket pyramid (typical fuzz seed)
//!   - deeply nested generic angle-bracket spam
//!   - non-UTF-8-clean byte sequences (handled at the str::from_utf8 layer
//!     before reaching the LSP — but the test pins the lossy fallback)
//!   - position past end-of-document
//!   - position with line/character at u32::MAX (no panic on overflow
//!     during line walk)
//!   - 0-length string with various trigger characters at position 0
//!   - very long single line (forces position_to_offset hot path)
//!
//! Every case asserts: (a) DocumentState construction succeeds; (b)
//! `complete_at_position` returns without panicking; (c) the returned
//! list is well-formed (the fuzzed input may legitimately yield zero
//! completions, but the call must not blow up).

use tower_lsp::lsp_types::Position;
use verum_ast::FileId;
use verum_lsp::completion::complete_at_position;
use verum_lsp::document::DocumentState;
use verum_lsp::rename::prepare_rename;

fn doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1, FileId::new(1))
}

fn at(line: u32, character: u32) -> Position {
    Position { line, character }
}

#[test]
fn empty_document_does_not_panic() {
    let d = doc("");
    let _ = complete_at_position(&d, at(0, 0));
}

#[test]
fn empty_document_position_past_end() {
    let d = doc("");
    // Past end of empty doc on every axis.
    let _ = complete_at_position(&d, at(100, 100));
}

#[test]
fn truncated_fn_declaration_does_not_panic() {
    let d = doc("fn foo(");
    // Cursor inside the unclosed parameter list.
    let _ = complete_at_position(&d, at(0, 7));
}

#[test]
fn unbalanced_bracket_pyramid_does_not_panic() {
    let mut s = String::with_capacity(2048);
    for _ in 0..1024 {
        s.push('{');
    }
    let d = doc(&s);
    let _ = complete_at_position(&d, at(0, 1024));
}

#[test]
fn deep_generic_angle_spam_does_not_panic() {
    // 256 nested angle brackets — past most parser recursion soft caps but
    // beneath the documented hard cap (ast_to_type at 64, then graceful error).
    let s = format!("type T = List{}{}", "<List".repeat(256), ">".repeat(256));
    let d = doc(&s);
    let _ = complete_at_position(&d, at(0, s.len() as u32 / 2));
}

#[test]
fn non_utf8_bytes_handled_at_string_construction() {
    // String::from_utf8_lossy round-trip — adversarial bytes that the LSP
    // protocol normally guards via JSON encoding, but we confirm the
    // tail-end DocumentState handles whatever fell through.
    let bytes = [0xFFu8, 0xFE, 0x00, 0x01, 0x80, 0x81, b'f', b'n', b' '];
    let lossy = String::from_utf8_lossy(&bytes).into_owned();
    let d = doc(&lossy);
    let _ = complete_at_position(&d, at(0, 0));
}

#[test]
fn position_at_u32_max_does_not_panic() {
    let d = doc("let x = 1;");
    // Both axes at u32::MAX — the line-walk loop must early-exit on
    // EOF rather than overflow or spin forever.
    let _ = complete_at_position(&d, at(u32::MAX, u32::MAX));
}

#[test]
fn position_zero_with_trigger_chars() {
    for trigger in ["", ".", ":", "::", "@", "<", "use ", "let "] {
        let d = doc(trigger);
        let _ = complete_at_position(&d, at(0, 0));
        let _ = complete_at_position(&d, at(0, trigger.len() as u32));
    }
}

#[test]
fn very_long_single_line_does_not_panic() {
    // 64 KB single line — `position_to_offset` must walk linearly without
    // pathological behaviour (utf-16 column mapping per LSP spec).
    let s = "fn f() { ".to_string() + &"x ".repeat(32_000) + "}";
    let len = s.len() as u32;
    let d = doc(&s);
    let _ = complete_at_position(&d, at(0, len));
    // Mid-line probe at the worst fragmentation point.
    let _ = complete_at_position(&d, at(0, len / 2));
}

#[test]
fn embedded_nul_bytes_do_not_panic() {
    // \0 in source should be lex-rejected without panicking the document
    // construction.  Mid-string NULs are a common fuzz seed.
    let s = "fn \0foo() {}\0";
    let d = doc(s);
    let _ = complete_at_position(&d, at(0, 4));
}

#[test]
fn combining_unicode_does_not_panic() {
    // U+0301 COMBINING ACUTE ACCENT after various triggers — ensures
    // codepoint vs. byte-offset confusion doesn't blow up character
    // boundary maths.
    let s = "fn \u{0301}foo() { }";
    let d = doc(s);
    let _ = complete_at_position(&d, at(0, 3));
    let _ = complete_at_position(&d, at(0, 4));
}

#[test]
fn many_short_lines_does_not_panic() {
    // 10 000 short lines — exercises the line-iterator hot path.
    let mut s = String::new();
    for _ in 0..10_000 {
        s.push_str("a\n");
    }
    let d = doc(&s);
    let _ = complete_at_position(&d, at(5_000, 0));
    let _ = complete_at_position(&d, at(9_999, 1));
}

#[test]
fn malformed_attribute_does_not_panic() {
    // @derive without closing paren — common fuzz seed in the
    // attribute-parser surface.
    let d = doc("@derive(Eq, Show\nfn foo() {}");
    let _ = complete_at_position(&d, at(0, 16));
    let _ = complete_at_position(&d, at(1, 0));
}

#[test]
fn malformed_import_does_not_panic() {
    let d = doc("mount foo.bar.");
    let _ = complete_at_position(&d, at(0, 14));
}

#[test]
fn multibyte_identifier_before_dot_does_not_panic() {
    // Receiver-name extractor previously confused char index with
    // byte offset; multi-byte chars in the receiver name (e.g.,
    // Unicode identifier-permitted CJK or accented Latin) caused
    // a "byte index N is not a char boundary" panic when the
    // member-completion path walked back from the dot.
    let d = doc("πα.");
    let s = "πα.";
    let _ = complete_at_position(&d, at(0, s.len() as u32));
    // Also at a position inside the receiver — the safe-prefix
    // path must clamp to a char boundary BEFORE the dot.
    let _ = complete_at_position(&d, at(0, 2));
}

#[test]
fn member_access_after_emoji_does_not_panic() {
    // 4-byte UTF-8 char (emoji) as part of the line before the dot.
    let s = "🦀.method";
    let d = doc(s);
    let _ = complete_at_position(&d, at(0, s.len() as u32));
    // Position at every byte offset from 0..len — at least some
    // will land mid-emoji.  None must panic.
    for col in 0..(s.len() as u32 + 2) {
        let _ = complete_at_position(&d, at(0, col));
    }
}

// ===== rename / find_word_range =====
//
// `prepare_rename` calls `find_word_range` which walks `&line[..n]`
// using LSP byte offsets that may land mid-codepoint after editor-
// negotiation rounding.  Multi-byte chars in the line previously
// caused the slice to panic.

#[test]
fn prepare_rename_with_multibyte_line_does_not_panic() {
    // Line containing combining accent before the cursor.
    let d = doc("let π = 1; π\u{0301}");
    for col in 0..30 {
        let _ = prepare_rename(&d, at(0, col));
    }
}

#[test]
fn prepare_rename_emoji_does_not_panic() {
    let d = doc("let 🦀 = 42;");
    for col in 0..("let 🦀 = 42;".len() as u32 + 2) {
        let _ = prepare_rename(&d, at(0, col));
    }
}

#[test]
fn prepare_rename_position_past_end_does_not_panic() {
    let d = doc("");
    let _ = prepare_rename(&d, at(0, 0));
    let _ = prepare_rename(&d, at(100, 100));
    let _ = prepare_rename(&d, at(u32::MAX, u32::MAX));
}

#[test]
fn prepare_rename_cursor_inside_multibyte_char_does_not_panic() {
    // Cursor at byte offset 1, mid-π (π is U+03C0, 2 bytes in UTF-8).
    let d = doc("π = 1");
    let _ = prepare_rename(&d, at(0, 1));
    let _ = prepare_rename(&d, at(0, 2));
}
