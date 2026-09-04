//! What the type-mismatch suggester actually says (T1142).
//!
//! Wiring this into every E400 in the language is a WIDENING change: it
//! adds text to every type error a user ever sees, and no regression
//! gate can see a widening change — the suite goes green whether the
//! advice is helpful, absent, or nonsense. So the volume and the content
//! are pinned here first, before anything calls it in anger.
//!
//! Read this file as the answer to "what will users start seeing?"

use verum_diagnostics::recovery::ErrorRecovery;

/// Representative mismatches, chosen to cover the three gates inside
/// `suggest_fixes_for_type_mismatch`: a known conversion, a refinement
/// mismatch, and a reference-mode mismatch — plus pairs that should
/// match none of them.
const CASES: &[(&str, &str)] = &[
    ("Text", "Int"),           // known conversion
    ("Int", "Text"),           // known conversion, other direction
    ("Float", "Int"),          // known conversion
    ("Int", "Bool"),           // known conversion
    ("&Int", "Int"),           // reference mode
    ("Int", "&Int"),           // reference mode
    ("List<Int>", "Map<Text, Int>"), // no advice expected
    ("MyRecord", "YourRecord"),      // no advice expected
];

#[test]
fn the_advice_offered_for_each_mismatch_is_bounded_and_verum() {
    let recovery = ErrorRecovery::new();
    let mut total = 0usize;

    for (expected, found) in CASES {
        let actions = recovery.suggest_fixes_for_type_mismatch(expected, found, "assignment");
        total += actions.len();

        // Volume: a diagnostic that grows a wall of advice is worse than
        // one with none. Three is already a lot to read under an error.
        assert!(
            actions.len() <= 3,
            "expected '{expected}' / found '{found}' produced {} suggestions; \
             a type error must not turn into a list",
            actions.len()
        );

        for action in &actions {
            let text = format!("{} {}", action.description, action.code_change.clone().unwrap_or_default());
            assert!(
                !text.contains("::"),
                "advice for '{expected}' / '{found}' contains `::`, which is \
                 not Verum syntax: {text}"
            );
            assert!(
                !text.contains("Option<") && !text.contains("Vec<"),
                "advice for '{expected}' / '{found}' names a Rust type: {text}"
            );
            assert!(
                action.confidence <= 100,
                "confidence is a percentage"
            );
        }
    }

    // Positive pole. Every assertion above passes trivially against a
    // function that returns nothing, so require the suggester to have
    // actually said something across this set.
    assert!(
        total >= 4,
        "the suggester produced {total} suggestions across {} mismatches — \
         it is silent, and the assertions above were measuring nothing",
        CASES.len()
    );
}

#[test]
fn an_unrelated_mismatch_draws_no_concrete_advice() {
    // Measured, not assumed. Two unrelated record types DO draw one
    // action — "Consider changing the variable's type annotation",
    // recovery.rs:388, with `code_change: Maybe::None`. It is a generic
    // hint keyed only on the context word, and it fires for every
    // assignment mismatch in the language regardless of the types.
    //
    // That is fine for an IDE quick-fix panel and wrong under a terminal
    // diagnostic, where it would append the same content-free line to
    // every E400 a user ever sees. Hence the rule a caller must follow,
    // pinned here: OFFER ONLY ACTIONS CARRYING A `code_change`. An
    // action with no code is not a suggestion, it is a category.
    let recovery = ErrorRecovery::new();
    let actions =
        recovery.suggest_fixes_for_type_mismatch("HttpRequest", "DatabaseCursor", "assignment");

    let concrete: Vec<_> = actions
        .iter()
        .filter(|a| a.code_change.is_some())
        .map(|a| a.description.to_string())
        .collect();
    assert!(
        concrete.is_empty(),
        "two unrelated record types drew CONCRETE advice: {concrete:?}"
    );

    // And the generic hint is still there, so this test fails loudly if
    // someone deletes the fallback instead of filtering it at the call
    // site — that would silently change what an IDE receives.
    assert_eq!(
        actions.len(),
        1,
        "expected exactly the one generic context hint, got {:?}",
        actions.iter().map(|a| a.description.to_string()).collect::<Vec<_>>()
    );
}
