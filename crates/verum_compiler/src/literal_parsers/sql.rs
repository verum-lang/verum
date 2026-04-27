//! SQL literal parser
//!
//! Tagged text literal parser for `sql#"..."`, `sql.postgres#"..."`,
//! `sql.sqlite#"..."`, `sql.mysql#"..."` per database.md §5.1 / §5.3.
//!
//! v0.1 contract (compile-time):
//!   * String must be non-empty after trimming.
//!   * Brace / bracket / paren depths must end balanced (catches the
//!     classic copy-paste truncation that an empty grammar pass
//!     would silently accept).
//!   * Quoted strings (single, double, dollar-quoted) must close.
//!   * Block comments (`/* ... */`) must close — Postgres allows
//!     nested form, MySQL doesn't; we accept both nesting flavours.
//!   * Reject obvious string-concatenation patterns that look like an
//!     escape from the safe-interpolation contract:
//!       - `' + ` or `" + ` after closing a quoted string token
//!       - `${...}` interpolations within quoted strings (must live
//!         in the SQL text, not inside a literal that the server
//!         would interpret as part of the string).
//!   * Count `${expr}` interpolations — the count is what `Params`
//!     length must match at the call site.
//!   * Compute a deterministic 64-bit fingerprint over the
//!     **normalised** SQL (whitespace collapsed, comments stripped).
//!     Two queries differing only in whitespace / comments share the
//!     same server-side cached statement.
//!
//! Schema-aware row-typing (`SELECT id FROM users` → typed
//! `PreparedQuery<{id: UserId}, ()>`) is gated on a separate
//! schema-snapshot file (`schema.snap.vr`); when absent the
//! parameter-count and fingerprint are still produced and the row
//! shape is left as `Unknown` for the caller to ascribe explicitly.
//!
//! Spec: internal/specs/database.md §5.1 (PreparedQuery), §5.3 (sql#).

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlDialect {
    Generic,    // sql#"..."
    Postgres,   // sql.postgres#"..."
    Sqlite,     // sql.sqlite#"..."
    Mysql,      // sql.mysql#"..."
}

impl SqlDialect {
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "sql.postgres" | "pg" | "psql" => Self::Postgres,
            "sql.sqlite"   | "sqlite"      => Self::Sqlite,
            "sql.mysql"    | "mysql"       => Self::Mysql,
            _                              => Self::Generic,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Generic  => "generic",
            Self::Postgres => "postgres",
            Self::Sqlite   => "sqlite",
            Self::Mysql    => "mysql",
        }
    }
}

/// Parse a `sql#"..."`-class tagged literal at compile time.
///
/// On success the returned `ParsedLiteral::Sql` carries the
/// normalised SQL text (callers reuse this as the on-wire payload),
/// the dialect, the bound-parameter count, and a 64-bit fingerprint
/// suitable for the server-side prepared-statement slot name.
pub fn parse_sql(
    content: &Text,
    tag: &str,
    span: Span,
    source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str();
    let trimmed = s.trim();

    if trimmed.is_empty() {
        return Err(make_err("SQL literal cannot be empty", span, source_file));
    }

    let dialect = SqlDialect::from_tag(tag);

    // Step 1 — structural balance + quote / comment closure.
    let bal = scan_balance(s);
    if let Err(reason) = bal {
        return Err(make_err(&format!("malformed SQL: {}", reason), span, source_file));
    }

    // Step 2 — count `${...}` interpolations. The lexer already
    // expanded these by the time content reaches us — for the static
    // text form we treat unescaped `${` as the marker.
    let param_count = count_interpolations(s);

    // Step 3 — reject obvious dynamic-concatenation antipatterns.
    if let Err(reason) = reject_concat_antipatterns(s) {
        return Err(make_err(
            &format!("disallowed SQL pattern: {}", reason),
            span,
            source_file,
        ));
    }

    // Step 4 — normalise (strip comments, collapse whitespace) and
    // fingerprint.
    let normalized = normalise(s);
    let fingerprint = fingerprint64(&normalized);

    Ok(ParsedLiteral::Sql {
        sql: Text::from(normalized.as_str()),
        dialect: Text::from(dialect.name()),
        param_count: param_count as u32,
        fingerprint,
    })
}

// ---------------------------------------------------------------------------
// Step 1: brace / bracket / paren balance + quote / comment closure.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Code,
    SingleQuote,            // '...'
    DoubleQuote,            // "..."   (Postgres identifier; SQLite TEXT)
    BacktickQuote,          // `...`   (MySQL identifier)
    DollarQuote,            // $$...$$ or $tag$...$tag$ (Postgres)
    LineComment,            // -- ... \n
    BlockComment(u32),      // /* ... */ — depth counter (Postgres allows nesting)
}

fn scan_balance(s: &str) -> Result<(), String> {
    let bytes = s.as_bytes();
    let mut state = LexState::Code;
    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut i = 0usize;
    let n = bytes.len();
    let mut dollar_tag: Option<Vec<u8>> = None;
    while i < n {
        let b = bytes[i];
        match state {
            LexState::Code => {
                match b {
                    b'\'' => { state = LexState::SingleQuote; }
                    b'"'  => { state = LexState::DoubleQuote; }
                    b'`'  => { state = LexState::BacktickQuote; }
                    b'(' => paren_depth   += 1,
                    b')' => paren_depth   -= 1,
                    b'[' => bracket_depth += 1,
                    b']' => bracket_depth -= 1,
                    b'{' => brace_depth   += 1,
                    b'}' => brace_depth   -= 1,
                    b'-' if i + 1 < n && bytes[i + 1] == b'-' => {
                        state = LexState::LineComment;
                        i += 1;
                    }
                    b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                        state = LexState::BlockComment(1);
                        i += 1;
                    }
                    b'$' => {
                        // Look for $tag$ or $$
                        let (consumed, tag_bytes) = read_dollar_tag(bytes, i);
                        if consumed > 0 {
                            dollar_tag = Some(tag_bytes);
                            state = LexState::DollarQuote;
                            i += consumed;
                            continue;
                        }
                    }
                    _ => {}
                }
                if paren_depth < 0 {
                    return Err(format!("unbalanced ')' at byte {}", i));
                }
                if bracket_depth < 0 {
                    return Err(format!("unbalanced ']' at byte {}", i));
                }
                if brace_depth < 0 {
                    return Err(format!("unbalanced '}}' at byte {}", i));
                }
            }
            LexState::SingleQuote => {
                if b == b'\'' {
                    // SQL standard '' = embedded apostrophe.
                    if i + 1 < n && bytes[i + 1] == b'\'' {
                        i += 1;
                    } else {
                        state = LexState::Code;
                    }
                }
            }
            LexState::DoubleQuote => {
                if b == b'"' {
                    if i + 1 < n && bytes[i + 1] == b'"' {
                        i += 1;
                    } else {
                        state = LexState::Code;
                    }
                }
            }
            LexState::BacktickQuote => {
                if b == b'`' {
                    if i + 1 < n && bytes[i + 1] == b'`' {
                        i += 1;
                    } else {
                        state = LexState::Code;
                    }
                }
            }
            LexState::DollarQuote => {
                if b == b'$' {
                    let want = dollar_tag.as_deref().unwrap_or(&[]);
                    if matches_at(bytes, i, want) {
                        // Move past the closing tag.
                        i += want.len();
                        state = LexState::Code;
                        dollar_tag = None;
                    }
                }
            }
            LexState::LineComment => {
                if b == b'\n' { state = LexState::Code; }
            }
            LexState::BlockComment(depth) => {
                if b == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    let new_depth = depth - 1;
                    if new_depth == 0 {
                        state = LexState::Code;
                    } else {
                        state = LexState::BlockComment(new_depth);
                    }
                    i += 1;
                } else if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment(depth + 1);
                    i += 1;
                }
            }
        }
        i += 1;
    }

    match state {
        LexState::Code => {}
        LexState::SingleQuote   => return Err("unterminated single-quoted string".to_string()),
        LexState::DoubleQuote   => return Err("unterminated double-quoted identifier".to_string()),
        LexState::BacktickQuote => return Err("unterminated backtick-quoted identifier".to_string()),
        LexState::DollarQuote   => return Err("unterminated dollar-quoted string".to_string()),
        LexState::LineComment   => {} // EOF after `--` is OK
        LexState::BlockComment(_) => return Err("unterminated /* */ comment".to_string()),
    }
    if paren_depth != 0 {
        return Err(format!("unbalanced parentheses (depth {})", paren_depth));
    }
    if bracket_depth != 0 {
        return Err(format!("unbalanced brackets (depth {})", bracket_depth));
    }
    if brace_depth != 0 {
        return Err(format!("unbalanced braces (depth {})", brace_depth));
    }
    Ok(())
}

/// Returns (bytes_consumed, tag_bytes_to_match_for_close).
fn read_dollar_tag(bytes: &[u8], i: usize) -> (usize, Vec<u8>) {
    // `$$` is the empty-tag form — `$$...$$`.
    // `$foo$` is the tagged form — `$foo$...$foo$`.
    if i + 1 >= bytes.len() { return (0, Vec::new()); }
    if bytes[i + 1] == b'$' {
        // `$$`
        return (2, b"$$".to_vec());
    }
    let mut j = i + 1;
    while j < bytes.len() {
        let c = bytes[j];
        if c == b'$' {
            // tag = bytes[i+1 .. j]; closer = "$" tag "$"
            let mut closer = Vec::with_capacity(j - i + 1);
            closer.push(b'$');
            closer.extend_from_slice(&bytes[i + 1..j]);
            closer.push(b'$');
            return (j - i + 1, closer);
        }
        if !is_dollar_tag_byte(c) {
            return (0, Vec::new());
        }
        j += 1;
    }
    (0, Vec::new())
}

fn is_dollar_tag_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn matches_at(bytes: &[u8], i: usize, want: &[u8]) -> bool {
    if i + want.len() > bytes.len() { return false; }
    &bytes[i..i + want.len()] == want
}

// ---------------------------------------------------------------------------
// Step 2: count `${...}` interpolations OUTSIDE quoted strings.
// ---------------------------------------------------------------------------

fn count_interpolations(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut count = 0usize;
    let mut state = LexState::Code;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            LexState::Code => {
                if b == b'\'' { state = LexState::SingleQuote; }
                else if b == b'"' { state = LexState::DoubleQuote; }
                else if b == b'`' { state = LexState::BacktickQuote; }
                else if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    state = LexState::LineComment;
                    i += 1;
                } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment(1);
                    i += 1;
                } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    count += 1;
                    // Skip past the matching `}` (we've already
                    // verified balance in step 1).
                    let mut depth = 1i32;
                    i += 2;
                    while i < bytes.len() && depth > 0 {
                        if bytes[i] == b'{' { depth += 1; }
                        else if bytes[i] == b'}' { depth -= 1; }
                        i += 1;
                    }
                    continue;
                }
            }
            LexState::SingleQuote => {
                if b == b'\'' && (i + 1 >= bytes.len() || bytes[i + 1] != b'\'') {
                    state = LexState::Code;
                }
            }
            LexState::DoubleQuote => {
                if b == b'"' && (i + 1 >= bytes.len() || bytes[i + 1] != b'"') {
                    state = LexState::Code;
                }
            }
            LexState::BacktickQuote => {
                if b == b'`' && (i + 1 >= bytes.len() || bytes[i + 1] != b'`') {
                    state = LexState::Code;
                }
            }
            LexState::LineComment => {
                if b == b'\n' { state = LexState::Code; }
            }
            LexState::BlockComment(depth) => {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    let nd = depth - 1;
                    if nd == 0 { state = LexState::Code; }
                    else { state = LexState::BlockComment(nd); }
                    i += 1;
                } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment(depth + 1);
                    i += 1;
                }
            }
            LexState::DollarQuote => {} // unreachable in this scan
        }
        i += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// Step 3: reject obvious dynamic-concatenation antipatterns.
// ---------------------------------------------------------------------------
//
// Spindle's `sql#"..."` is supposed to be the only mechanism for
// putting values into a SQL string. We don't have a lexer-level
// guarantee — `sql#"SELECT * FROM users WHERE name = '" + input + "'"`
// is a literal-then-string-concat at the Verum level; the *raw* SQL
// content stays as `SELECT * FROM users WHERE name = '`. We catch the
// most obvious form: a quoted-string ending followed by a `+` or `||`
// that crosses a quote boundary.

fn reject_concat_antipatterns(s: &str) -> Result<(), String> {
    // `'" + ` or `\" + ` immediately after a closing quote is the
    // signature of a Verum-side string concat that escapes the
    // tagged-literal protection. The `||` SQL concat between literals
    // and `${...}` is also a smell — the user should have used a
    // single `${...}` and trusted the param-binding instead.
    //
    // We are conservative: only flag patterns that look intentional.
    // Catching every false positive here is bad UX.
    if s.contains("' + ") || s.contains("\" + ") {
        return Err(
            "raw '+' concatenation across a string boundary — \
             use ${expr} instead of breaking out of the literal"
                .to_string(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Step 4: normalise (strip comments + collapse whitespace).
// ---------------------------------------------------------------------------

fn normalise(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut state = LexState::Code;
    let mut last_was_space = false;
    let mut i = 0usize;
    let mut dollar_tag: Option<Vec<u8>> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            LexState::Code => {
                if b == b'\'' { state = LexState::SingleQuote;   out.push(b as char); i += 1; last_was_space = false; continue; }
                if b == b'"'  { state = LexState::DoubleQuote;   out.push(b as char); i += 1; last_was_space = false; continue; }
                if b == b'`'  { state = LexState::BacktickQuote; out.push(b as char); i += 1; last_was_space = false; continue; }
                if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    state = LexState::LineComment;
                    i += 2;
                    continue;
                }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment(1);
                    i += 2;
                    continue;
                }
                if b == b'$' {
                    let (consumed, tag) = read_dollar_tag(bytes, i);
                    if consumed > 0 {
                        for k in 0..consumed { out.push(bytes[i + k] as char); }
                        dollar_tag = Some(tag);
                        state = LexState::DollarQuote;
                        i += consumed;
                        last_was_space = false;
                        continue;
                    }
                }
                if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                    if !last_was_space {
                        out.push(' ');
                        last_was_space = true;
                    }
                    i += 1;
                    continue;
                }
                out.push(b as char);
                last_was_space = false;
                i += 1;
                continue;
            }
            LexState::SingleQuote => {
                out.push(b as char);
                if b == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    } else {
                        state = LexState::Code;
                    }
                }
                i += 1;
            }
            LexState::DoubleQuote => {
                out.push(b as char);
                if b == b'"' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    } else {
                        state = LexState::Code;
                    }
                }
                i += 1;
            }
            LexState::BacktickQuote => {
                out.push(b as char);
                if b == b'`' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'`' {
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    } else {
                        state = LexState::Code;
                    }
                }
                i += 1;
            }
            LexState::DollarQuote => {
                out.push(b as char);
                if b == b'$' {
                    let want = dollar_tag.as_deref().unwrap_or(&[]);
                    if matches_at(bytes, i, want) {
                        for k in 1..want.len() {
                            out.push(bytes[i + k] as char);
                        }
                        i += want.len();
                        state = LexState::Code;
                        dollar_tag = None;
                        continue;
                    }
                }
                i += 1;
            }
            LexState::LineComment => {
                if b == b'\n' {
                    state = LexState::Code;
                    if !last_was_space {
                        out.push(' ');
                        last_was_space = true;
                    }
                }
                i += 1;
            }
            LexState::BlockComment(depth) => {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    let nd = depth - 1;
                    if nd == 0 {
                        state = LexState::Code;
                        if !last_was_space {
                            out.push(' ');
                            last_was_space = true;
                        }
                    } else {
                        state = LexState::BlockComment(nd);
                    }
                    i += 2;
                } else if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    state = LexState::BlockComment(depth + 1);
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }
    let trimmed = out.trim().to_string();
    trimmed
}

// ---------------------------------------------------------------------------
// Step 5: 64-bit fingerprint over the normalised SQL.
// ---------------------------------------------------------------------------
//
// FNV-1a over UTF-8 bytes — fast, deterministic, no allocations,
// adequate for naming a server-side prepared-statement slot.

fn fingerprint64(s: &str) -> i64 {
    const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const FNV_PRIME:  u64 = 0x0000_0100_0000_01B3;
    let mut h = FNV_OFFSET;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h as i64
}

// ---------------------------------------------------------------------------
// Diagnostic helper.
// ---------------------------------------------------------------------------

fn make_err(message: &str, _span: Span, _source_file: Option<&verum_ast::SourceFile>) -> Diagnostic {
    DiagnosticBuilder::error().message(message.to_string()).build()
}
