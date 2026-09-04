//! A tuple payload's label is as permissive as a record field's (T1091).
//!
//! `grammar/verum.ebnf` (tuple_slot) records the measured asymmetry this
//! fixes, and names the mechanism: these words are lexer TOKENS rather
//! than identifiers, and the language's name positions differ only in
//! how many of them insist on an `Ident`. The tuple-slot label insisted
//! on it and was therefore the strictest name position in the language:
//!
//!     word      { w: Int }   fn f(w:)   let w =   A(w: Int)
//!     stage         ok          ok         ok      REFUSED
//!     gen           ok          ok         ok      REFUSED
//!     set           ok          ok         ok      REFUSED
//!     field         ok          ok         ok      REFUSED
//!     layer         ok          ok         ok      REFUSED
//!     other         ok          ok         ok        ok      <- control
//!
//! The fix makes the label position agree with the RECORD FIELD
//! position exactly — no wider, no narrower. Both now consume the name
//! with `consume_ident_or_keyword`.
//!
//! WHY THIS FILE EXISTS RATHER THAN A LINE IN AN EXISTING SUITE: the
//! change WIDENS what parses, and no regression gate can see a widening
//! change — every existing test passes before and after. The true pole
//! and the must-stay-refused pole are both here, because if they are not
//! written down the fix is unfalsifiable.

use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_ast::span::FileId;

fn parses(source: &str) -> bool {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).is_ok()
}

/// Words the grammar's table lists as "free everywhere EXCEPT this
/// position" — the asymmetry the fix removes.
const CONTEXTUAL: &[&str] = &["stage", "gen", "set", "field", "layer", "ref", "with"];

/// Words reserved in EVERY position. Their refusal is about the word,
/// not the position, and it must survive the fix — otherwise the change
/// widened past the record-field position it was meant to match.
const RESERVED: &[&str] = &["in", "private", "move"];

#[test]
fn a_contextual_keyword_labels_a_tuple_slot() {
    for w in CONTEXTUAL {
        let src = format!("type Ev is Started({w}: Int);\n");
        assert!(
            parses(&src),
            "`{w}` is usable as a record field but was refused as a tuple-slot \
             label — the two name positions still disagree: {src}"
        );
    }
}

#[test]
fn the_same_word_still_labels_a_record_field() {
    // The control that makes the test above meaningful: if the record
    // position had regressed, both would be "consistent" and both wrong.
    for w in CONTEXTUAL {
        let src = format!("type Ev is {{ {w}: Int }};\n");
        assert!(
            parses(&src),
            "`{w}` was refused as a RECORD field — the fix changed the wrong \
             position, or broke the one that already worked: {src}"
        );
    }
}

#[test]
fn a_fully_reserved_word_is_still_refused_in_both_positions() {
    // The must-stay-refused pole. Without it, "the label position got
    // more permissive" could mean "it now accepts everything", which
    // would be a different defect wearing this fix's clothes.
    for w in RESERVED {
        assert!(
            !parses(&format!("type Ev is Started({w}: Int);\n")),
            "`{w}` is reserved in every position but was accepted as a \
             tuple-slot label — the widening went past the record-field \
             position it was supposed to match"
        );
        assert!(
            !parses(&format!("type Ev is {{ {w}: Int }};\n")),
            "`{w}` is reserved in every position but was accepted as a \
             record field"
        );
    }
}

#[test]
fn an_ordinary_identifier_label_is_unaffected() {
    // `other` is the grammar table's control: it passed all four
    // positions before the fix and must still pass.
    assert!(parses("type Ev is Started(other: Int);\n"));
    assert!(parses("type Ev is { other: Int };\n"));
}

#[test]
fn an_unlabelled_tuple_payload_is_unaffected() {
    // The overwhelmingly common form. A widening change to the label
    // lookahead must not disturb the case that carries no label at all.
    assert!(parses("type Maybe2 is Nothing | Just(Int);\n"));
    assert!(parses("type Pair is P(Int, Text);\n"));
}

#[test]
fn a_where_clause_bound_is_not_read_as_a_labelled_slot() {
    // `T: Ord` in a where clause is a BOUND, not a `name: Type` slot.
    // The sigma-type path is gated by `allow_sigma` for exactly this
    // reason; widening the lookahead must not reach past that gate.
    assert!(parses("fn pick<T>(a: T, b: T) -> T where T: Ord { a }\n"));
    assert!(parses("fn pick2<T>(a: T) -> T where T: Ord + Clone { a }\n"));
}

#[test]
fn a_sigma_type_still_parses_with_its_own_binder() {
    // The construct the widened lookahead sits in front of. `result` was
    // special-cased by hand before the fix; the shared consumer maps it
    // now, so this is the check that the special case was subsumed and
    // not simply dropped.
    assert!(parses("fn positive(x: Int) -> result: Int where result > 0 { x }\n"));
}
