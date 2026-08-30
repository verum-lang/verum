//! Gate: every code the PARSER can print has a registry entry (T0973).
//!
//! Its sibling, `registry_covers_every_emitted_code.rs`, scans the source
//! tree for three spellings — `.code("E400")`, `code: "E400"` and
//! `Text::from("E400")` in a `code:` field. The parser uses none of them.
//! It maps an enum to a literal:
//!
//!     ErrorCode::UnterminatedChar => "E001",
//!
//! so 138 codes a user could meet had no entry while that gate reported
//! green, and the six that DID overlap described something else: the
//! registry read `E001` as "unexpected token" while the compiler printed
//! it for an unterminated character literal. `verum explain E001`
//! answered with the wrong meaning — worse than answering nothing.
//!
//! This gate does not look for a fourth spelling. It asks the
//! ENUMERATION, so a fifth way of writing a code cannot hide from it:
//! `ErrorCode::ALL` is the authority for what the parser can emit, and
//! the registry has to cover it.

use verum_fast_parser::error::ErrorCode;

#[test]
fn registry_covers_every_parser_code() {
    let mut missing: Vec<&str> = Vec::new();
    let mut seen = 0usize;

    for code in ErrorCode::ALL {
        let s = code.as_str();
        seen += 1;
        // Meta codes (`M…`) are a separate numbering space with its own
        // table; this gate covers the E-space.
        if !s.starts_with('E') {
            continue;
        }
        if !verum_error::registry::is_known(s) {
            missing.push(s);
        }
    }

    // Input-completeness control. The gate this one supplements passed for
    // days while comparing nothing, because its self-tests checked that
    // extraction WORKS, never that there was anything to extract. Assert
    // the size of the input, not just the verdict.
    assert!(
        seen >= 150,
        "only {seen} parser codes enumerated — ErrorCode::ALL has fallen \
         out of step with the enum, so this gate is measuring a fragment"
    );

    missing.sort_unstable();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "{} of {seen} parser codes have no registry entry, so `verum explain` \
         cannot describe a diagnostic the user just saw:\n  {}",
        missing.len(),
        missing.join(", ")
    );
}

/// The descriptions have to mean what the parser means. Six of them did
/// not, and a wrong explanation is worse than a missing one.
#[test]
fn the_shared_codes_describe_what_the_parser_prints() {
    let cases = [
        ("E001", "character"),
        ("E002", "escape"),
        ("E003", "number"),
        ("E006", "token"),
        ("E018", "token"),
    ];
    for (code, expected_word) in cases {
        let entry = verum_error::registry::REGISTRY
            .get(code)
            .unwrap_or_else(|| panic!("{code} must be registered"));
        assert!(
            entry.description.contains(expected_word),
            "{code} is described as {:?}, which does not describe what the \
             parser prints it for",
            entry.description
        );
    }
}
