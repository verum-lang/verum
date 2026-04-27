#![allow(unexpected_cfgs)]
//! Lexer for the Verum programming language.
//!
//! This crate provides a high-performance lexer built on top of the `logos` crate,
//! which uses code generation to produce fast, zero-allocation tokenization.
//!
//! # Overview
//!
//! The lexer tokenizes Verum source code into a stream of tokens according to the
//! language specification. Verum's lexical grammar defines whitespace and comments
//! (line `//` and block `/* */`), Unicode identifiers (ident_start = letter | '_',
//! ident_continue = letter | digit | '_'), ~41 keywords (3 reserved: `let`, `fn`, `is`;
//! plus primary, control flow, async, modifier, FFI, module, and additional keywords),
//! numeric literals (decimal, hex `0x`, binary `0b`, octal `0o`, floats with exponents,
//! optional unit suffixes), text literals (plain, multiline `"""`, interpolated `f"..."`,
//! tagged `tag#"..."`, contract `contract#"..."`), and operators/delimiters.
//!
//! # Features
//!
//! - **Zero-copy**: The lexer operates directly on the source string slice
//! - **Fast**: Uses logos for optimized DFA-based lexing
//! - **Complete**: Supports all Verum syntax including:
//!   - Keywords (`fn`, `type`, `let`, `match`, `import`, etc.)
//!   - Operators (`|>`, `?.`, `??`, `&`, `%`, etc.)
//!   - CBGR references (`&T`, `&mut T`)
//!   - Ownership references (`%T`, `%mut T`)
//!   - Literals (integers, floats, strings, chars, booleans)
//!   - Comments (line and block)
//! - **Error recovery**: Handles invalid tokens gracefully
//! - **Location tracking**: Preserves source spans for error reporting
//!
//! # Example
//!
//! ```
//! use verum_lexer::{Lexer, TokenKind};
//! use verum_ast::span::FileId;
//! use verum_common::List;
//!
//! let source = "fn add(x: Int, y: Int) -> Int { x + y }";
//! let file_id = FileId::new(0);
//! let lexer = Lexer::new(source, file_id);
//!
//! // Tokenize the entire input
//! let tokens: List<_> = lexer.map(|r| r.unwrap()).collect();
//!
//! // First token should be 'fn'
//! assert!(matches!(tokens[0].kind, TokenKind::Fn));
//! ```
//!
//! # Module Structure
//!
//! - [`token`]: Token type definitions and categorization
//! - [`lexer`]: The main lexer implementation
//! - [`error`]: Lexical error types

#![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]

pub mod error;
pub mod lexer;
pub mod lossless;
pub mod token;

pub use error::{LexResult, VerumError};
pub use lexer::{Lexer, LookaheadLexer, strip_shebang, strip_utf8_bom, UTF8_BOM};
pub use lossless::{LosslessLexer, RichToken, Trivia, TriviaItem, TriviaKind};
pub use token::{
    FloatLiteral, IntegerLiteral, InterpolatedStringLiteral, TaggedLiteralData,
    TaggedLiteralDelimiter, Token, TokenKind,
};

// Backwards compatibility alias
pub type LexError = VerumError;

#[cfg(test)]
mod test_ffi_tokenization {
    use super::*;
    use verum_ast::FileId;

    #[test]
    fn test_ffi_keyword_is_tokenized_correctly() {
        let source = "ffi";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);

        let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
        // Should have Ffi and Eof tokens
        assert!(
            !tokens.is_empty(),
            "Expected at least 1 token, got {}: {:?}",
            tokens.len(),
            tokens
        );

        match &tokens[0].kind {
            TokenKind::Ffi => {} // SUCCESS
            TokenKind::Ident(text) => panic!("FAIL: 'ffi' tokenized as Ident({})", text),
            other => panic!("FAIL: 'ffi' tokenized as {:?}", other),
        }
    }

    #[test]
    fn test_ffi_boundary_tokens() {
        let source = "ffi LibMath { }";
        let file_id = FileId::new(0);
        let lexer = Lexer::new(source, file_id);

        let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
        assert!(
            tokens.len() >= 4,
            "Expected at least 4 tokens, got {}: {:?}",
            tokens.len(),
            tokens
        );

        assert!(
            matches!(&tokens[0].kind, TokenKind::Ffi),
            "First token should be Ffi, got {:?}",
            tokens[0].kind
        );
        assert!(
            matches!(&tokens[1].kind, TokenKind::Ident(_)),
            "Second token should be Ident, got {:?}",
            tokens[1].kind
        );
        assert!(
            matches!(&tokens[2].kind, TokenKind::LBrace),
            "Third token should be LBrace, got {:?}",
            tokens[2].kind
        );
        assert!(
            matches!(&tokens[3].kind, TokenKind::RBrace),
            "Fourth token should be RBrace, got {:?}",
            tokens[3].kind
        );
    }
}

#[test]
fn test_requires_keyword() {
    use verum_ast::FileId;
    let source = "requires";
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);

    let tokens: Vec<_> = lexer.filter_map(|t| t.ok()).collect();
    assert!(!tokens.is_empty());

    match &tokens[0].kind {
        TokenKind::Requires => {} // SUCCESS
        TokenKind::Ident(text) => panic!("FAIL: 'requires' tokenized as Ident({})", text),
        other => panic!("FAIL: 'requires' tokenized as {:?}", other),
    }
}

#[cfg(test)]
mod shebang_tests {
    use super::*;
    use verum_ast::FileId;

    fn fid() -> FileId {
        FileId::new(0)
    }

    // strip_shebang() unit tests --------------------------------------------------------------

    #[test]
    fn strip_shebang_no_shebang() {
        let (body, sb) = strip_shebang("fn main() {}");
        assert_eq!(body, "fn main() {}");
        assert_eq!(sb, None);
    }

    #[test]
    fn strip_shebang_simple_lf() {
        let (body, sb) = strip_shebang("#!/usr/bin/env verum\nfn main() {}");
        assert_eq!(body, "fn main() {}");
        assert_eq!(sb, Some("#!/usr/bin/env verum\n"));
    }

    #[test]
    fn strip_shebang_crlf() {
        // CRLF: the \r remains as the last character of the shebang slice (before \n).
        // Logos's whitespace skip rule swallows the \r at the start of body either way; the
        // shebang slice keeps \r so that callers reconstructing the file losslessly are exact.
        let (body, sb) = strip_shebang("#!/usr/bin/env verum\r\nfn main() {}");
        assert_eq!(body, "fn main() {}");
        assert_eq!(sb, Some("#!/usr/bin/env verum\r\n"));
    }

    #[test]
    fn strip_shebang_only_shebang_no_newline() {
        let (body, sb) = strip_shebang("#!/usr/bin/env verum");
        assert_eq!(body, "");
        assert_eq!(sb, Some("#!/usr/bin/env verum"));
    }

    #[test]
    fn strip_shebang_empty() {
        let (body, sb) = strip_shebang("");
        assert_eq!(body, "");
        assert_eq!(sb, None);
    }

    #[test]
    fn strip_shebang_short_input() {
        // Single '#' is not a shebang.
        let (body, sb) = strip_shebang("#");
        assert_eq!(body, "#");
        assert_eq!(sb, None);
    }

    #[test]
    fn strip_shebang_pound_no_bang() {
        // '# something' is not a shebang.
        let (body, sb) = strip_shebang("# not shebang\nfn main() {}");
        assert_eq!(body, "# not shebang\nfn main() {}");
        assert_eq!(sb, None);
    }

    #[test]
    fn strip_shebang_unicode_in_shebang() {
        // The shebang line may contain UTF-8 bytes. The split is on the first \n byte;
        // because UTF-8 is self-synchronising, this never splits a multi-byte char.
        let (body, sb) = strip_shebang("#!/usr/bin/env verum --флаг\nfn main() {}");
        assert_eq!(body, "fn main() {}");
        assert_eq!(sb, Some("#!/usr/bin/env verum --флаг\n"));
    }

    // UTF-8 BOM handling ---------------------------------------------------------------------

    #[test]
    fn strip_utf8_bom_no_bom() {
        let (body, had) = strip_utf8_bom("fn main() {}");
        assert_eq!(body, "fn main() {}");
        assert!(!had);
    }

    #[test]
    fn strip_utf8_bom_with_bom() {
        let src = "\u{FEFF}fn main() {}"; // U+FEFF is encoded as EF BB BF in UTF-8
        let (body, had) = strip_utf8_bom(src);
        assert_eq!(body, "fn main() {}");
        assert!(had);
    }

    #[test]
    fn strip_utf8_bom_idempotent() {
        let src = "\u{FEFF}fn main() {}";
        let (body, _) = strip_utf8_bom(src);
        let (body2, had2) = strip_utf8_bom(body);
        assert_eq!(body, body2, "idempotent on already-stripped input");
        assert!(!had2);
    }

    #[test]
    fn strip_utf8_bom_constant_matches_three_bytes() {
        assert_eq!(UTF8_BOM, &[0xEF, 0xBB, 0xBF]);
        assert_eq!(UTF8_BOM.len(), 3);
        assert_eq!("\u{FEFF}".as_bytes(), UTF8_BOM);
    }

    #[test]
    fn lexer_strips_bom_then_shebang() {
        // Cross-platform editor wrote a BOM in front of an executable script.
        // The lexer must strip BOTH the BOM and the shebang, place neither in
        // the token stream, and still report `had_bom` + `had_shebang`.
        let src = "\u{FEFF}#!/usr/bin/env verum\nfn main() {}";
        let lex = Lexer::new(src, fid());
        assert!(lex.had_bom());
        assert!(lex.had_shebang());
        assert_eq!(lex.shebang_text(), Some("#!/usr/bin/env verum\n"));
        // Offset = BOM (3 bytes) + shebang (21 bytes) = 24.
        assert_eq!(lex.shebang_offset(), 3 + "#!/usr/bin/env verum\n".len() as u32);
    }

    #[test]
    fn lexer_strips_bom_no_shebang() {
        let src = "\u{FEFF}fn main() {}";
        let lex = Lexer::new(src, fid());
        assert!(lex.had_bom());
        assert!(!lex.had_shebang());
        // First token should be `fn` at the post-BOM offset (3).
        let first = lex.into_iter().filter_map(|t| t.ok()).next().unwrap();
        assert!(matches!(first.kind, TokenKind::Fn));
        assert_eq!(first.span.start, 3);
        assert_eq!(first.span.end, 5);
    }

    #[test]
    fn lexer_no_bom_no_shebang() {
        let lex = Lexer::new("fn main() {}", fid());
        assert!(!lex.had_bom());
        assert!(!lex.had_shebang());
        assert_eq!(lex.shebang_offset(), 0);
    }

    #[test]
    fn strip_shebang_does_not_handle_bom_alone() {
        // Contract: strip_shebang receives BOM-free input. A BOM-prefixed
        // shebang fed directly to strip_shebang is not detected as a shebang.
        // Lexer::new takes care of the BOM-strip pre-pass.
        let src = "\u{FEFF}#!/usr/bin/env verum\nfn main() {}";
        let (body, sb) = strip_shebang(src);
        assert_eq!(sb, None, "strip_shebang must NOT see past a BOM");
        assert_eq!(body, src);
    }

    // Lexer integration -----------------------------------------------------------------------

    #[test]
    fn lexer_recognises_shebang() {
        let src = "#!/usr/bin/env verum\nfn main() {}";
        let lex = Lexer::new(src, fid());
        assert!(lex.had_shebang());
        assert_eq!(lex.shebang_text(), Some("#!/usr/bin/env verum\n"));
        assert_eq!(lex.shebang_offset(), "#!/usr/bin/env verum\n".len() as u32);
    }

    #[test]
    fn lexer_no_shebang_offset_zero() {
        let src = "fn main() {}";
        let lex = Lexer::new(src, fid());
        assert!(!lex.had_shebang());
        assert_eq!(lex.shebang_text(), None);
        assert_eq!(lex.shebang_offset(), 0);
    }

    #[test]
    fn lexer_token_spans_use_original_offsets() {
        // After stripping `#!/usr/bin/env verum\n` (21 bytes), `fn` starts at byte 21
        // in the original source. The first token's span MUST reflect that.
        let src = "#!/usr/bin/env verum\nfn main() {}";
        let lex = Lexer::new(src, fid());
        let tokens: Vec<_> = lex.filter_map(|t| t.ok()).collect();
        assert!(!tokens.is_empty());
        let fn_tok = &tokens[0];
        assert!(matches!(fn_tok.kind, TokenKind::Fn));
        assert_eq!(fn_tok.span.start, 21);
        assert_eq!(fn_tok.span.end, 23);
    }

    #[test]
    fn lexer_shebang_stripped_does_not_emit_extra_tokens() {
        // Without shebang.
        let bare = Lexer::new("fn main() {}", fid()).filter_map(|t| t.ok()).count();
        // With shebang.
        let with_sb = Lexer::new("#!/usr/bin/env verum\nfn main() {}", fid())
            .filter_map(|t| t.ok())
            .count();
        assert_eq!(bare, with_sb, "shebang must not change token count");
    }

    #[test]
    fn lexer_eof_span_is_in_original_coordinates() {
        // EOF span must point at the absolute end of the original source.
        let src = "#!/usr/bin/env verum\nfn main() {}";
        let lex = Lexer::new(src, fid());
        let tokens: Vec<_> = lex.filter_map(|t| t.ok()).collect();
        let eof = tokens.last().unwrap();
        assert!(matches!(eof.kind, TokenKind::Eof));
        assert_eq!(eof.span.start, src.len() as u32);
        assert_eq!(eof.span.end, src.len() as u32);
    }

    // LosslessLexer integration ---------------------------------------------------------------

    #[test]
    fn lossless_emits_shebang_trivia_on_first_token() {
        let src = "#!/usr/bin/env verum\nfn main() {}";
        let tokens = LosslessLexer::new(src, fid()).tokenize();
        // First non-EOF token is `fn`; its leading_trivia should contain a Shebang item.
        let first = tokens
            .iter()
            .find(|t| matches!(t.token.kind, TokenKind::Fn))
            .expect("fn token must be present");
        let sb_item = first
            .leading_trivia
            .items
            .iter()
            .find(|t| matches!(t.kind, TriviaKind::Shebang))
            .expect("shebang trivia must be attached to first token");
        assert_eq!(sb_item.text, "#!/usr/bin/env verum\n");
        assert_eq!(sb_item.span.start, 0);
        assert_eq!(sb_item.span.end, 21);
    }

    #[test]
    fn lossless_emits_bom_then_shebang_in_order() {
        // Byte-perfect round-trip contract: BOM must precede shebang in
        // leading_trivia, and the concatenation of all leading_trivia
        // text + token text + trailing_trivia text must rebuild the
        // original source exactly.
        let src = "\u{FEFF}#!/usr/bin/env verum\nfn main() {}";
        let tokens = LosslessLexer::new(src, fid()).tokenize();
        let first = tokens
            .iter()
            .find(|t| matches!(t.token.kind, TokenKind::Fn))
            .expect("fn token must be present");
        let kinds: Vec<TriviaKind> = first.leading_trivia.items.iter().map(|t| t.kind).collect();
        // BOM must come strictly before Shebang.
        let bom_idx = kinds.iter().position(|k| matches!(k, TriviaKind::ByteOrderMark));
        let sb_idx  = kinds.iter().position(|k| matches!(k, TriviaKind::Shebang));
        assert!(bom_idx.is_some(), "BOM trivia missing");
        assert!(sb_idx.is_some(),  "Shebang trivia missing");
        assert!(bom_idx < sb_idx, "BOM must precede shebang in leading_trivia");
        // Spans on the BOM and shebang must be in original-source coordinates.
        let bom = &first.leading_trivia.items[bom_idx.unwrap()];
        let sb  = &first.leading_trivia.items[sb_idx.unwrap()];
        assert_eq!(bom.text.as_bytes(), UTF8_BOM);
        assert_eq!(bom.span.start, 0);
        assert_eq!(bom.span.end, 3);
        assert_eq!(sb.text, "#!/usr/bin/env verum\n");
        assert_eq!(sb.span.start, 3);
        assert_eq!(sb.span.end, 3 + 21);
    }

    #[test]
    fn lossless_emits_bom_alone_no_shebang() {
        let src = "\u{FEFF}fn main() {}";
        let tokens = LosslessLexer::new(src, fid()).tokenize();
        let first = tokens
            .iter()
            .find(|t| matches!(t.token.kind, TokenKind::Fn))
            .expect("fn token must be present");
        let bom = first
            .leading_trivia
            .items
            .iter()
            .find(|t| matches!(t.kind, TriviaKind::ByteOrderMark))
            .expect("BOM trivia must be attached");
        assert_eq!(bom.text.as_bytes(), UTF8_BOM);
        assert_eq!(bom.span.start, 0);
        assert_eq!(bom.span.end, 3);
        // No shebang.
        assert!(!first
            .leading_trivia
            .items
            .iter()
            .any(|t| matches!(t.kind, TriviaKind::Shebang)));
    }

    #[test]
    fn lossless_no_shebang_no_trivia_added() {
        let src = "fn main() {}";
        let tokens = LosslessLexer::new(src, fid()).tokenize();
        for t in tokens.iter() {
            for it in t.leading_trivia.items.iter() {
                assert!(
                    !matches!(it.kind, TriviaKind::Shebang),
                    "no Shebang trivia must appear when source has none"
                );
            }
        }
    }
}
