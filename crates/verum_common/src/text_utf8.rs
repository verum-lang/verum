//! UTF-8-safe primitives for byte-offset / char-boundary navigation.
//!
//! This module is the canonical home for the operations that previously
//! showed up as ad-hoc byte-vs-char-index confusion across the LSP and
//! VBC layers — every fix in the 2026-04-29 R2-§8.1 sweep ultimately
//! reduces to one of three primitives:
//!
//! 1. **Clamp a byte offset to a char boundary.** The LSP protocol
//!    delivers cursor positions that may land mid-codepoint (e.g.,
//!    UTF-16-column ↔ UTF-8-byte rounding); naive `&line[..offset]`
//!    panics.
//! 2. **Truncate a string by character count, not byte count.** Debug
//!    previews and disassembly output fed `&s[..N]` for a fixed N;
//!    when N landed inside a multi-byte UTF-8 sequence, the slice
//!    panicked.
//! 3. **Find the word at a byte offset.** Identifier extraction
//!    around a cursor previously mixed byte offsets with
//!    `chars().nth(byte_offset)` (treating bytes as char indices).
//!    For ASCII this coincidentally works; for any multi-byte
//!    content it silently mis-locates or returns false-positive
//!    matches.
//!
//! All three primitives are zero-allocation on the hot path (the
//! word-extraction primitive returns byte-offset bounds rather than
//! a copy), use only standard-library `is_char_boundary` /
//! `char_indices`, and run in O(prefix-length) for clamps and
//! O(word-length) for word extraction — the same bound as the
//! buggy ad-hoc implementations they replace.

/// Clamp a byte offset DOWN to the nearest preceding char boundary.
///
/// Walks at most 3 bytes since UTF-8 sequences are ≤ 4 bytes long;
/// returns immediately if the offset is already at a boundary.  When
/// the offset exceeds `text.len()` it is first capped at `text.len()`,
/// which is always a valid char boundary.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::clamp_to_char_boundary;
/// // π is U+03C0 — 2 bytes in UTF-8.  Byte offset 1 lands inside it.
/// let s = "π = 1";
/// assert_eq!(clamp_to_char_boundary(s, 0), 0); // before π
/// assert_eq!(clamp_to_char_boundary(s, 1), 0); // mid-π → clamped
/// assert_eq!(clamp_to_char_boundary(s, 2), 2); // after π
/// assert_eq!(clamp_to_char_boundary(s, 999), s.len()); // past EOF
/// ```
#[inline]
pub fn clamp_to_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut clamped = byte_offset.min(text.len());
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

/// UTF-8-safe prefix slice: `&text[..byte_offset]` with the offset
/// clamped to the nearest preceding char boundary.
///
/// The conservative choice is to round DOWN — returning the
/// already-typed prefix is always semantically safe, while extending
/// past a half-typed multi-byte char would lie about the cursor
/// position.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::safe_prefix;
/// let s = "π = 1";
/// assert_eq!(safe_prefix(s, 0), "");
/// assert_eq!(safe_prefix(s, 1), ""); // mid-π → clamped to 0
/// assert_eq!(safe_prefix(s, 2), "π");
/// assert_eq!(safe_prefix(s, 999), s); // past EOF
/// ```
#[inline]
pub fn safe_prefix(text: &str, byte_offset: usize) -> &str {
    &text[..clamp_to_char_boundary(text, byte_offset)]
}

/// Truncate a string to at most `max_chars` characters, returning a
/// borrowed `&str` slice.  Counts Unicode characters (code points),
/// NOT bytes — naive `&s[..N]` panics when `N` falls inside a
/// multi-byte UTF-8 sequence.
///
/// Returns the original slice unchanged if it has ≤ `max_chars`
/// characters.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::truncate_chars;
/// assert_eq!(truncate_chars("hello", 3), "hel");
/// assert_eq!(truncate_chars("πα", 1), "π");
/// assert_eq!(truncate_chars("πα", 5), "πα"); // shorter than limit
/// assert_eq!(truncate_chars("", 3), "");
/// ```
#[inline]
pub fn truncate_chars(text: &str, max_chars: usize) -> &str {
    let mut take_bytes = 0;
    let mut count = 0;
    for (byte_idx, ch) in text.char_indices() {
        if count >= max_chars {
            return &text[..take_bytes];
        }
        take_bytes = byte_idx + ch.len_utf8();
        count += 1;
    }
    text
}

/// Find the byte-offset bounds `(start, end)` of the word containing
/// the given byte offset, using the supplied `is_word_char`
/// predicate.
///
/// `byte_offset` is clamped to the nearest preceding char boundary
/// before the walk begins.  Returns `None` when the cursor is not on
/// a word character (the standard LSP contract — no word means no
/// rename / hover / completion target).
///
/// The returned bounds are always at char boundaries, so
/// `&text[start..end]` is always safe to slice.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::find_word_bounds;
/// let pred = |c: char| c.is_alphanumeric() || c == '_';
/// // ASCII-only — byte offsets coincide with char indices.
/// assert_eq!(find_word_bounds("foo + bar", 1, pred), Some((0, 3)));
/// // Multi-byte identifier (Greek π is 2 bytes in UTF-8).
/// // "πα = 1" has identifier "πα" at bytes 0..4.
/// assert_eq!(find_word_bounds("πα = 1", 0, pred), Some((0, 4)));
/// // Cursor on '=' (byte 5) — no word at that position.
/// assert_eq!(find_word_bounds("πα = 1", 5, pred), None);
/// ```
pub fn find_word_bounds(
    text: &str,
    byte_offset: usize,
    mut is_word_char: impl FnMut(char) -> bool,
) -> Option<(usize, usize)> {
    let cursor = clamp_to_char_boundary(text, byte_offset);

    // Walk backwards from the cursor as long as we see word chars.
    let mut start = cursor;
    for (byte_idx, ch) in text[..cursor].char_indices().rev() {
        if is_word_char(ch) {
            start = byte_idx;
        } else {
            break;
        }
    }

    // Walk forwards from the cursor.
    let mut end = cursor;
    for (byte_idx, ch) in text[cursor..].char_indices() {
        if is_word_char(ch) {
            end = cursor + byte_idx + ch.len_utf8();
        } else {
            break;
        }
    }

    if start == end {
        None
    } else {
        Some((start, end))
    }
}

/// Whether the character immediately preceding `byte_offset` in
/// `text` (if any) satisfies the given predicate.  Returns `None`
/// when `byte_offset == 0` (no preceding char).  Walks UTF-8 backwards
/// correctly; never panics on multi-byte input.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::char_before_satisfies;
/// let s = "foo.bar";
/// assert_eq!(char_before_satisfies(s, 0, |_| true), None);
/// assert_eq!(char_before_satisfies(s, 4, |c| c == '.'), Some(true));
/// // Multi-byte: 'π' (U+03C0, 2 bytes) precedes byte offset 2.
/// let t = "πα";
/// assert_eq!(char_before_satisfies(t, 2, |c| c == 'π'), Some(true));
/// ```
#[inline]
pub fn char_before_satisfies(
    text: &str,
    byte_offset: usize,
    pred: impl FnMut(char) -> bool,
) -> Option<bool> {
    if byte_offset == 0 || byte_offset > text.len() {
        return None;
    }
    let clamped = clamp_to_char_boundary(text, byte_offset);
    text[..clamped].chars().next_back().map(pred)
}

/// Whether the character at `byte_offset` in `text` (if any) satisfies
/// the given predicate.  Returns `None` when `byte_offset >= text.len()`.
/// Walks UTF-8 forwards correctly; never panics on multi-byte input.
///
/// # Examples
///
/// ```
/// use verum_common::text_utf8::char_at_satisfies;
/// let s = "foo.bar";
/// assert_eq!(char_at_satisfies(s, 3, |c| c == '.'), Some(true));
/// assert_eq!(char_at_satisfies(s, 100, |_| true), None);
/// ```
#[inline]
pub fn char_at_satisfies(
    text: &str,
    byte_offset: usize,
    pred: impl FnMut(char) -> bool,
) -> Option<bool> {
    if byte_offset >= text.len() {
        return None;
    }
    let clamped = clamp_to_char_boundary(text, byte_offset);
    text[clamped..].chars().next().map(pred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_to_char_boundary_ascii() {
        assert_eq!(clamp_to_char_boundary("hello", 0), 0);
        assert_eq!(clamp_to_char_boundary("hello", 3), 3);
        assert_eq!(clamp_to_char_boundary("hello", 5), 5);
        assert_eq!(clamp_to_char_boundary("hello", 100), 5);
    }

    #[test]
    fn clamp_to_char_boundary_multibyte() {
        // π = 2 bytes (0xCF 0x80)
        let s = "π";
        assert_eq!(clamp_to_char_boundary(s, 0), 0);
        assert_eq!(clamp_to_char_boundary(s, 1), 0); // mid-codepoint
        assert_eq!(clamp_to_char_boundary(s, 2), 2);
    }

    #[test]
    fn clamp_to_char_boundary_emoji() {
        // 🦀 = 4 bytes (0xF0 0x9F 0xA6 0x80)
        let s = "🦀";
        assert_eq!(clamp_to_char_boundary(s, 0), 0);
        assert_eq!(clamp_to_char_boundary(s, 1), 0);
        assert_eq!(clamp_to_char_boundary(s, 2), 0);
        assert_eq!(clamp_to_char_boundary(s, 3), 0);
        assert_eq!(clamp_to_char_boundary(s, 4), 4);
    }

    #[test]
    fn clamp_to_char_boundary_combining() {
        // U+0301 COMBINING ACUTE ACCENT = 2 bytes (0xCC 0x81)
        let s = "fn ́foo()";
        // Position 4 is mid-combining-mark.
        let clamped = clamp_to_char_boundary(s, 4);
        assert!(s.is_char_boundary(clamped));
    }

    #[test]
    fn safe_prefix_returns_borrowed_slice() {
        let s = "πα = 1";
        assert_eq!(safe_prefix(s, 0), "");
        assert_eq!(safe_prefix(s, 1), ""); // mid-π
        assert_eq!(safe_prefix(s, 2), "π");
        assert_eq!(safe_prefix(s, 4), "πα");
        assert_eq!(safe_prefix(s, 999), s);
    }

    #[test]
    fn truncate_chars_basic() {
        assert_eq!(truncate_chars("hello", 0), "");
        assert_eq!(truncate_chars("hello", 3), "hel");
        assert_eq!(truncate_chars("hello", 5), "hello");
        assert_eq!(truncate_chars("hello", 100), "hello");
        assert_eq!(truncate_chars("", 3), "");
    }

    #[test]
    fn truncate_chars_multibyte() {
        // 5 chars, 10 bytes total (each is 2 bytes).
        let s = "παππα";
        assert_eq!(truncate_chars(s, 0), "");
        assert_eq!(truncate_chars(s, 1), "π");
        assert_eq!(truncate_chars(s, 3), "παπ");
        assert_eq!(truncate_chars(s, 5), s);
        assert_eq!(truncate_chars(s, 100), s);
    }

    #[test]
    fn truncate_chars_emoji() {
        let s = "🦀🦀🦀";
        assert_eq!(truncate_chars(s, 1), "🦀");
        assert_eq!(truncate_chars(s, 2), "🦀🦀");
    }

    #[test]
    fn find_word_bounds_ascii() {
        let pred = |c: char| c.is_alphanumeric() || c == '_';
        assert_eq!(find_word_bounds("foo + bar", 0, pred), Some((0, 3)));
        assert_eq!(find_word_bounds("foo + bar", 2, pred), Some((0, 3)));
        assert_eq!(find_word_bounds("foo + bar", 3, pred), Some((0, 3)));
        assert_eq!(find_word_bounds("foo + bar", 4, pred), None); // on space
        assert_eq!(find_word_bounds("foo + bar", 6, pred), Some((6, 9)));
    }

    #[test]
    fn find_word_bounds_multibyte() {
        let pred = |c: char| c.is_alphanumeric() || c == '_';
        // πα = identifier at bytes 0..4 (each Greek letter is 2 bytes).
        assert_eq!(find_word_bounds("πα + 1", 0, pred), Some((0, 4)));
        assert_eq!(find_word_bounds("πα + 1", 2, pred), Some((0, 4)));
        // Cursor immediately after the word still returns the word —
        // matches LSP word_at_position convention (and the ASCII test
        // case `find_word_bounds("foo + bar", 3, pred) == Some((0, 3))`).
        assert_eq!(find_word_bounds("πα + 1", 4, pred), Some((0, 4)));
        // Cursor on the '+' is past whitespace, no word.
        assert_eq!(find_word_bounds("πα + 1", 5, pred), None);
        // Cursor mid-π is clamped to 0 first.
        assert_eq!(find_word_bounds("πα + 1", 1, pred), Some((0, 4)));
    }

    #[test]
    fn find_word_bounds_empty_input() {
        let pred = |c: char| c.is_alphanumeric() || c == '_';
        assert_eq!(find_word_bounds("", 0, pred), None);
        assert_eq!(find_word_bounds("", 100, pred), None);
    }

    #[test]
    fn find_word_bounds_returns_safe_slice() {
        let pred = |c: char| c.is_alphanumeric() || c == '_';
        let s = "let π = 42";
        // Cursor inside the identifier "π".
        let (start, end) = find_word_bounds(s, 4, pred).expect("π is identifier");
        // Slice MUST not panic — bounds are at char boundaries.
        let word = &s[start..end];
        assert_eq!(word, "π");
    }

    #[test]
    fn char_before_satisfies_works() {
        let s = "foo.bar";
        assert_eq!(char_before_satisfies(s, 0, |_| true), None);
        assert_eq!(char_before_satisfies(s, 1, |c| c == 'f'), Some(true));
        assert_eq!(char_before_satisfies(s, 4, |c| c == '.'), Some(true));
        assert_eq!(char_before_satisfies(s, 4, |c| c == 'x'), Some(false));
        // Past end is clamped to text len, so the last char is checked.
        assert_eq!(char_before_satisfies(s, 7, |c| c == 'r'), Some(true));
    }

    #[test]
    fn char_before_satisfies_multibyte() {
        // π (2 bytes) before byte offset 2.
        let s = "πα";
        assert_eq!(char_before_satisfies(s, 2, |c| c == 'π'), Some(true));
        // Mid-π is clamped to 0; nothing precedes.
        assert_eq!(char_before_satisfies(s, 1, |_| true), None);
    }

    #[test]
    fn char_at_satisfies_works() {
        let s = "foo.bar";
        assert_eq!(char_at_satisfies(s, 0, |c| c == 'f'), Some(true));
        assert_eq!(char_at_satisfies(s, 3, |c| c == '.'), Some(true));
        assert_eq!(char_at_satisfies(s, 7, |_| true), None); // past end
        assert_eq!(char_at_satisfies(s, 100, |_| true), None);
    }

    #[test]
    fn char_at_satisfies_multibyte() {
        let s = "πα";
        assert_eq!(char_at_satisfies(s, 0, |c| c == 'π'), Some(true));
        assert_eq!(char_at_satisfies(s, 2, |c| c == 'α'), Some(true));
        // Mid-π is clamped to 0; π is at offset 0.
        assert_eq!(char_at_satisfies(s, 1, |c| c == 'π'), Some(true));
    }

    #[test]
    fn no_panic_on_arbitrary_byte_offsets() {
        // Stress: every byte offset 0..len must be safe.
        let inputs = [
            "",
            "ascii only",
            "πα = 1",
            "let 🦀 = 42",
            "fn \u{0301}foo()",
            "🦀🦀🦀🦀",
        ];
        let pred = |c: char| c.is_alphanumeric() || c == '_';
        for s in inputs {
            for i in 0..(s.len() + 5) {
                let _ = clamp_to_char_boundary(s, i);
                let _ = safe_prefix(s, i);
                let _ = find_word_bounds(s, i, pred);
                let _ = char_before_satisfies(s, i, pred);
                let _ = char_at_satisfies(s, i, pred);
            }
            for n in 0..10 {
                let _ = truncate_chars(s, n);
            }
        }
    }
}
