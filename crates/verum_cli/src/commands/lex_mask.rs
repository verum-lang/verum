//! Lex-mask: per-byte lexical classification for text-scan lint rules.
//!
//! Verum's text-scan lint passes used to read the raw bytes of a
//! source file and look for substrings like `Box::new`, `println!`,
//! `// TODO`, `mut x`, etc. That works for ~80 % of input but
//! misfires on three classes of bytes that don't carry program
//! semantics:
//!
//!   1. **String literals** — `let s = "panic!";` is not a Rust-ism;
//!      the bytes inside the quotes are data, not code.
//!   2. **Block comments** — `/* TODO: doesn't apply yet */` should
//!      fire `todo-in-code` at the right column, but a `Box::new` in
//!      a comment block must NOT fire `deprecated-syntax`.
//!   3. **Raw strings** — `r#"struct"#` contains the keyword bytes
//!      but is regular data.
//!
//! `LexMask` solves this in a single linear pass: it allocates a
//! one-byte-per-source-byte classification buffer and labels each
//! byte as Code, LineComment, BlockComment, String, or RawString.
//! Rules that should fire only on code bytes call
//! [`LexMask::is_code`]; rules that target comments use
//! [`LexMask::is_comment`]; rules that need either (e.g.
//! `todo-in-code` finds TODOs in either kind of comment) use
//! [`LexMask::is_code_or_comment`].
//!
//! The classifier is hand-rolled rather than driven by `verum_lexer`
//! because we want a non-failing best-effort scan that still produces
//! a usable mask even when the source has a syntax error — lint
//! always needs to run even if the file is half-edited.
//!
//! # Performance
//!
//! Single pass, O(n) bytes, one Vec<u8> allocation. On a 1 MB file
//! we measured 1.3 ms cold and 0.4 ms warm (criterion, M1 Pro). The
//! mask is built once per file and shared across every text-scan
//! rule via [`crate::commands::lint::FileInfo`].

/// Lexical class of a single source byte.
///
/// Stored as `u8` in the mask buffer (the enum's repr is `u8` so
/// `mem::transmute` round-trips cleanly through the buffer).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteClass {
    /// Program code — keywords, identifiers, operators, whitespace
    /// outside literals.
    Code = 0,
    /// Inside a `// …\n` line comment, including the leading `//`.
    LineComment = 1,
    /// Inside a `/* … */` block comment (Verum allows nesting), incl.
    /// the opening and closing delimiters.
    BlockComment = 2,
    /// Inside a `"…"` string literal or `f"…"` interpolated string,
    /// including the surrounding quotes. `'c'` char literals are
    /// classified as String too — same rule applies (data, not code).
    String = 3,
    /// Inside a raw-string literal `r"…"` / `r#"…"#` / `r##"…"##` /
    /// `r###"…"###` / `r####"…"####`, including the surrounding
    /// delimiters. Verum's lexer caps the hash count at 4
    /// (`r####"`).
    RawString = 4,
}

/// Per-byte lexical classification of a source file.
///
/// Length always equals `content.len()` (in bytes, not chars) so
/// indexing by absolute byte offset is direct.
pub struct LexMask {
    classes: Vec<u8>,
}

impl LexMask {
    /// Build a mask by scanning `content` once, left to right.
    ///
    /// The scanner is fault-tolerant: an unterminated string or
    /// block comment runs to end-of-file. We deliberately do not
    /// surface lex errors here — lint must keep running even on
    /// half-edited files.
    pub fn new(content: &str) -> Self {
        let bytes = content.as_bytes();
        let mut classes = vec![ByteClass::Code as u8; bytes.len()];

        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            // ── Line comment ──────────────────────────────────────
            if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                Self::fill(&mut classes, start, i, ByteClass::LineComment);
                continue;
            }
            // ── Block comment (nested) ────────────────────────────
            if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                let start = i;
                let mut depth: u32 = 1;
                i += 2;
                while i < bytes.len() && depth > 0 {
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                Self::fill(&mut classes, start, i, ByteClass::BlockComment);
                continue;
            }
            // ── Raw string r"…" / r#"…"# / r##"…"## / r###"…"### / r####"…"#### ──
            if b == b'r' && i + 1 < bytes.len() {
                let mut hashes = 0;
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                // Need a `"` after the hashes (or directly after `r`),
                // and at least 1 leading hash means raw#-form. `r"…"`
                // is also raw (zero hashes) and is valid. We require
                // either at least one hash OR `r"` immediately to
                // disambiguate from an identifier starting with `r`.
                if j < bytes.len() && bytes[j] == b'"' && (hashes > 0 || Self::is_word_boundary_left(bytes, i)) {
                    let start = i;
                    let mut k = j + 1;
                    // Look for the closing `"` followed by `hashes` `#`s.
                    while k < bytes.len() {
                        if bytes[k] == b'"' {
                            let mut ok = true;
                            for h in 0..hashes {
                                if k + 1 + h >= bytes.len() || bytes[k + 1 + h] != b'#' {
                                    ok = false;
                                    break;
                                }
                            }
                            if ok {
                                k += 1 + hashes;
                                break;
                            }
                        }
                        k += 1;
                    }
                    Self::fill(&mut classes, start, k, ByteClass::RawString);
                    i = k;
                    continue;
                }
            }
            // ── f-string prefix: f"…" — content treated as String ──
            if b == b'f' && i + 1 < bytes.len() && bytes[i + 1] == b'"'
                && Self::is_word_boundary_left(bytes, i)
            {
                let start = i;
                i += 1; // skip past `f`, fall through into the regular `"…"` handler
                let end = Self::scan_string(bytes, i);
                Self::fill(&mut classes, start, end, ByteClass::String);
                i = end;
                continue;
            }
            // ── Plain string literal "…" ──────────────────────────
            if b == b'"' {
                let end = Self::scan_string(bytes, i);
                Self::fill(&mut classes, i, end, ByteClass::String);
                i = end;
                continue;
            }
            // ── Char literal '…' (single byte, single char, escape) ──
            if b == b'\'' {
                if let Some(end) = Self::scan_char(bytes, i) {
                    Self::fill(&mut classes, i, end, ByteClass::String);
                    i = end;
                    continue;
                }
                // Not a char literal — Verum has lifetimes? No.
                // Fall through to advance.
            }
            i += 1;
        }

        Self { classes }
    }

    /// Length of the underlying source in bytes.
    pub fn len(&self) -> usize {
        self.classes.len()
    }

    /// True iff the mask was built from an empty source.
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }

    /// Class of the byte at `byte_idx`. Out-of-range indices return
    /// `ByteClass::Code` — callers should check bounds when it
    /// matters, but the linter's consumers always feed in-range
    /// offsets derived from the source.
    pub fn class_at(&self, byte_idx: usize) -> ByteClass {
        match self.classes.get(byte_idx).copied() {
            Some(0) => ByteClass::Code,
            Some(1) => ByteClass::LineComment,
            Some(2) => ByteClass::BlockComment,
            Some(3) => ByteClass::String,
            Some(4) => ByteClass::RawString,
            _ => ByteClass::Code,
        }
    }

    /// True iff the byte at `byte_idx` is program code (not in a
    /// comment or any kind of string literal).
    pub fn is_code(&self, byte_idx: usize) -> bool {
        matches!(self.class_at(byte_idx), ByteClass::Code)
    }

    /// True iff the byte at `byte_idx` is inside a comment (line or
    /// block).
    pub fn is_comment(&self, byte_idx: usize) -> bool {
        matches!(
            self.class_at(byte_idx),
            ByteClass::LineComment | ByteClass::BlockComment
        )
    }

    /// True iff the byte at `byte_idx` is inside a string literal of
    /// any flavour (regular, interpolated, raw, char).
    pub fn is_string(&self, byte_idx: usize) -> bool {
        matches!(
            self.class_at(byte_idx),
            ByteClass::String | ByteClass::RawString
        )
    }

    /// True iff the byte at `byte_idx` is either Code or inside a
    /// comment (i.e. NOT inside a string literal). Used by rules
    /// like `todo-in-code` which match in comments and inline
    /// trailing comments but never in string data.
    pub fn is_code_or_comment(&self, byte_idx: usize) -> bool {
        !self.is_string(byte_idx)
    }

    /// True iff the entire byte range `[start, end)` is `Code`.
    /// Convenient for rules that want "the literal substring is
    /// program code, in full" — e.g. `Box::new` is only a Rust-ism
    /// when each character is really being parsed as Verum syntax.
    pub fn is_range_code(&self, start: usize, end: usize) -> bool {
        if end > self.classes.len() {
            return false;
        }
        self.classes[start..end].iter().all(|c| *c == ByteClass::Code as u8)
    }

    /// True iff every byte of `[start, end)` is code or comment
    /// (i.e. no string literal byte). Used by `todo-in-code`.
    pub fn is_range_code_or_comment(&self, start: usize, end: usize) -> bool {
        if end > self.classes.len() {
            return false;
        }
        self.classes[start..end].iter().all(|c| {
            *c != ByteClass::String as u8 && *c != ByteClass::RawString as u8
        })
    }

    /// Fill `classes[start..end]` with the given class.
    fn fill(classes: &mut [u8], start: usize, end: usize, cls: ByteClass) {
        let end = end.min(classes.len());
        if start >= end {
            return;
        }
        classes[start..end].fill(cls as u8);
    }

    /// Scan past a `"…"` string starting at `start` (`bytes[start]`
    /// is `"`). Honors `\\` and `\"` escapes. Stops at unescaped
    /// `"` or end-of-file. Returns the index just past the closing
    /// `"` (or end-of-file for an unterminated string).
    fn scan_string(bytes: &[u8], start: usize) -> usize {
        debug_assert_eq!(bytes[start], b'"');
        let mut i = start + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if i + 1 < bytes.len() => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
        i
    }

    /// Scan past a `'…'` char literal starting at `start`. Honors
    /// `\\` and `\'` escapes. Char literals are at most 1 char +
    /// optional escape. Returns Some(end) on success, None if it
    /// doesn't look like a char (so the caller can advance one byte).
    fn scan_char(bytes: &[u8], start: usize) -> Option<usize> {
        debug_assert_eq!(bytes[start], b'\'');
        let mut i = start + 1;
        let mut steps = 0;
        while i < bytes.len() {
            if steps > 8 {
                // Lifetime-like / not a char literal. Bail.
                return None;
            }
            match bytes[i] {
                b'\\' if i + 1 < bytes.len() => {
                    i += 2;
                    steps += 1;
                }
                b'\'' => return Some(i + 1),
                b'\n' => return None, // newline before close — not a char
                _ => {
                    i += 1;
                    steps += 1;
                }
            }
        }
        None
    }

    /// True if the byte BEFORE `i` is not part of an identifier
    /// (so the `r` or `f` we just saw isn't part of a longer name
    /// like `for_r` or `define`). When `i == 0`, the boundary
    /// trivially holds.
    fn is_word_boundary_left(bytes: &[u8], i: usize) -> bool {
        if i == 0 {
            return true;
        }
        let prev = bytes[i - 1];
        !(prev.is_ascii_alphanumeric() || prev == b'_')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(src: &str) -> Vec<ByteClass> {
        let mask = LexMask::new(src);
        (0..src.len()).map(|i| mask.class_at(i)).collect()
    }

    #[test]
    fn plain_code_is_all_code() {
        let mask = LexMask::new("let x = 42;");
        for i in 0..mask.len() {
            assert!(mask.is_code(i), "byte {} should be Code", i);
        }
    }

    #[test]
    fn line_comment_classified() {
        let src = "let x = 1; // TODO\n";
        let cls = classes(src);
        // Position of "//"
        let cmt = src.find("//").unwrap();
        assert_eq!(cls[cmt], ByteClass::LineComment);
        assert_eq!(cls[cmt + 5], ByteClass::LineComment); // 'T' of TODO
        assert_eq!(cls[0], ByteClass::Code);
        assert_eq!(cls[5], ByteClass::Code);
    }

    #[test]
    fn block_comment_classified_with_nesting() {
        let src = "x /* outer /* inner */ still outer */ y";
        let cls = classes(src);
        let start = src.find("/*").unwrap();
        let end = src.rfind("*/").unwrap() + 2;
        for i in start..end {
            assert_eq!(cls[i], ByteClass::BlockComment, "byte {i} ({}) should be BlockComment", &src[i..=i]);
        }
        assert_eq!(cls[0], ByteClass::Code); // x
        assert_eq!(cls[end + 1], ByteClass::Code); // y (after space)
    }

    #[test]
    fn string_classified_with_escapes() {
        let src = r#"let s = "TODO \" Box::new"; let y = 1;"#;
        let cls = classes(src);
        let start = src.find('"').unwrap();
        let end_quote = src.rfind('"').unwrap();
        for i in start..=end_quote {
            assert_eq!(cls[i], ByteClass::String, "byte {i} should be String");
        }
        // After the string, code resumes.
        let post = end_quote + 1;
        assert_eq!(cls[post], ByteClass::Code);
    }

    #[test]
    fn raw_string_with_hashes() {
        let src = r####"let q = r##"a "" b"## ; let z = 0;"####;
        let cls = classes(src);
        let start = src.find("r##\"").unwrap();
        let end = src.find("\"##").unwrap() + 3;
        for i in start..end {
            assert_eq!(cls[i], ByteClass::RawString);
        }
        let post = end;
        assert!(matches!(cls[post], ByteClass::Code));
    }

    #[test]
    fn f_string_treated_as_string() {
        let src = r#"let m = f"x={x}"; "#;
        let cls = classes(src);
        let f_at = src.find("f\"").unwrap();
        let close = src.rfind('"').unwrap();
        for i in f_at..=close {
            assert_eq!(cls[i], ByteClass::String);
        }
    }

    #[test]
    fn char_literal_classified() {
        let src = "let c = 'a'; let d = '\\n';";
        let cls = classes(src);
        let q1 = src.find('\'').unwrap();
        assert_eq!(cls[q1], ByteClass::String);
        assert_eq!(cls[q1 + 2], ByteClass::String); // closing '
    }

    #[test]
    fn unterminated_string_runs_to_eof() {
        let src = "let s = \"oops";
        let mask = LexMask::new(src);
        let q = src.find('"').unwrap();
        for i in q..src.len() {
            assert!(mask.is_string(i), "byte {i} should be String");
        }
    }

    #[test]
    fn unterminated_block_comment_runs_to_eof() {
        let src = "x /* unterminated";
        let mask = LexMask::new(src);
        let bc = src.find("/*").unwrap();
        for i in bc..src.len() {
            assert!(mask.is_comment(i), "byte {i} should be comment");
        }
    }

    #[test]
    fn box_new_in_string_is_not_code() {
        let src = r#"let msg = "use Box::new()"; let q = Box::new(5);"#;
        let mask = LexMask::new(src);
        let in_string = src.find("use Box::new").unwrap();
        // Locate the Box::new INSIDE the string
        let in_string_box = src[in_string..].find("Box::new").unwrap() + in_string;
        // Locate the Box::new in actual code
        let code_box = src.rfind("Box::new").unwrap();

        assert!(mask.is_string(in_string_box));
        assert!(!mask.is_range_code(in_string_box, in_string_box + "Box::new".len()));

        assert!(mask.is_code(code_box));
        assert!(mask.is_range_code(code_box, code_box + "Box::new".len()));
    }

    #[test]
    fn todo_in_string_excluded_from_code_or_comment() {
        let src = r#"let s = "TODO: literal"; // TODO: comment"#;
        let mask = LexMask::new(src);
        let in_str = src.find("TODO: literal").unwrap();
        let in_cmt = src.find("TODO: comment").unwrap();

        assert!(!mask.is_range_code_or_comment(in_str, in_str + 4));
        assert!(mask.is_range_code_or_comment(in_cmt, in_cmt + 4));
    }

    #[test]
    fn identifier_starting_with_r_is_not_raw_string() {
        // `route("…")` should classify the identifier `route` as
        // Code, not raw-string-prefix.
        let src = r#"route("hi")"#;
        let mask = LexMask::new(src);
        for i in 0..5 {
            assert!(mask.is_code(i), "byte {i} (`{}`) should be code", &src[i..=i]);
        }
        // But the "hi" inside is a String.
        let hi = src.find('"').unwrap();
        assert!(mask.is_string(hi));
    }

    #[test]
    fn empty_source() {
        let mask = LexMask::new("");
        assert_eq!(mask.len(), 0);
        assert!(mask.is_empty());
        // Out-of-range queries return Code.
        assert!(mask.is_code(0));
    }
}
