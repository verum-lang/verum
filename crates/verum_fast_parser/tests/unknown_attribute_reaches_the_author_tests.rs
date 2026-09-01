//! A one-character typo in `@verify(thorough)` used to turn a FAILING
//! verification green, with no diagnostic at all:
//!
//!     @verify(thorough)   Summary: 1 proved, 1 failed   <- caught
//!     @verfiy(thorough)   Summary: 2 proved, 0 failed   <- typo in NAME
//!
//! For a language whose central claim is verification, the strongest
//! guarantee it offers could be switched off by one character, and the
//! report then said the opposite of the truth. That costs the author not
//! time but false confidence, which is worse.
//!
//! The machinery existed at FOUR levels and was reached at none:
//!
//!   1. the validator knew the attribute set and had the message ready,
//!      and `Parser::new` never constructed it;
//!   2. `validate_attrs_for_target` was called exactly twice — for Impl
//!      and for Module — so a FUNCTION's attributes were never validated
//!      even when a validator existed;
//!   3. warnings landed in a vector whose only production reader was an
//!      example inside a doc comment;
//!   4. `ParseResult` carries errors only, so there was no channel out.
//!
//! These tests pin levels 1-3: parsing a function with an unknown
//! attribute must produce a warning that a caller can take.
//!
//! Task: T1025.

use verum_ast::FileId;
use verum_fast_parser::FastParser;
use verum_lexer::Lexer;

fn warnings_for(source: &str) -> Vec<String> {
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let _ = parser.parse_module(lexer, file_id);
    parser
        .take_attr_warnings()
        .into_iter()
        .map(|w| w.message.as_str().to_string())
        .collect()
}

#[test]
fn an_unknown_attribute_on_a_function_is_reported() {
    let w = warnings_for(
        "\
@not_a_real_attribute_xyz
fn f(n: Int) -> Int { n }
",
    );
    assert!(
        w.iter().any(|m| m.contains("not_a_real_attribute_xyz")),
        "an unknown attribute on a FUNCTION must be reported — the validator \
         was only ever asked about Impl and Module targets; got: {w:?}"
    );
}

/// The case the task is named for: the typo makes `@verify` unknown, so
/// the verification the author asked for never happens.
#[test]
fn a_misspelled_verify_is_reported() {
    let w = warnings_for(
        "\
@verfiy(thorough)
pure fn spin(n: Int) -> Int requires n >= 0 { n }
",
    );
    assert!(
        w.iter().any(|m| m.contains("verfiy")),
        "a typo in an attribute NAME must be reported; got: {w:?}"
    );
}

/// The differentiator. Without it, a change that warned about every
/// attribute would satisfy both tests above while making the compiler
/// unusable.
#[test]
fn a_correctly_spelled_attribute_is_silent() {
    let w = warnings_for(
        "\
@verify(thorough)
pure fn spin(n: Int) -> Int requires n >= 0 { n }
",
    );
    assert!(
        w.is_empty(),
        "a known attribute must produce no warning; got: {w:?}"
    );
}

/// The warning has to carry a location, or it is nearly useless in a file
/// of any size — the first wiring emitted "no source location attached".
#[test]
fn the_warning_carries_a_location() {
    let parser = FastParser::new();
    let file_id = FileId::new(0);
    let source = "\
@not_a_real_attribute_xyz
fn f(n: Int) -> Int { n }
";
    let lexer = Lexer::new(source, file_id);
    let _ = parser.parse_module(lexer, file_id);
    let warnings = parser.take_attr_warnings();
    let w = warnings
        .iter()
        .find(|w| w.message.as_str().contains("not_a_real_attribute_xyz"))
        .expect("the warning is produced");
    assert!(
        w.span.end > w.span.start,
        "the warning must point at the attribute, not at an empty span: {:?}",
        w.span
    );
}
