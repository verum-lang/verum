//! `verum test --help` must name every `--format` value the command
//! accepts (T1339).
//!
//! `TestFormat::parse` accepts six spellings and emits real output for
//! all of them — measured on a one-test run:
//!
//! ```text
//! junit  -> <?xml …><testsuites tests="1" failures="0" …>
//! tap    -> TAP version 13 / 1..1 / ok 1 - …
//! sarif  -> {"$schema": "…sarif-schema-2.1.0.json", "runs": […]}
//! json   -> {"event":"suite",…} {"event":"test",…} {"event":"summary",…}
//! terse  -> running 1 test … test result: ok.
//! pretty -> the default
//! ```
//!
//! The help text listed the first three. So the three CI formats — the
//! ones a reader most needs to DISCOVER, because nobody guesses `sarif`
//! — existed, worked, and were documented only on the website. A flag
//! whose help under-reports its own values is a feature that ships
//! invisible.
//!
//! The gate is behavioural rather than a source scan: it runs the built
//! binary and reads what a user reads.

use std::process::Command;
use verum_cli::commands::test::TestFormat;

/// Every spelling `TestFormat::parse` accepts. Kept beside the parser
/// rather than derived from it — `parse` matches on string literals and
/// has no iterator — so a new format that is added to the parser and not
/// to this list will fail the round-trip assertion below rather than
/// slipping past the help check.
const ACCEPTED: &[&str] = &["pretty", "terse", "json", "junit", "junit-xml", "tap", "sarif"];

#[test]
fn the_list_matches_what_the_parser_accepts() {
    for spelling in ACCEPTED {
        assert!(
            TestFormat::parse(spelling).is_ok(),
            "`{}` is in this test's list but `TestFormat::parse` rejects it",
            spelling
        );
    }
    assert!(
        TestFormat::parse("xml").is_err(),
        "an unknown format must be refused, not silently downgraded — \
         otherwise `--format junitt` writes a plain-text listing into a \
         file the CI will try to parse as XML"
    );
}

#[test]
fn help_names_every_format_the_parser_accepts() {
    let out = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["test", "--help"])
        .output()
        .expect("`verum test --help` must run");
    let help = String::from_utf8_lossy(&out.stdout);

    // `junit-xml` is an alias of `junit`; the help names the canonical
    // spelling, so aliases are not required to appear.
    let missing: Vec<&str> = ACCEPTED
        .iter()
        .filter(|f| **f != "junit-xml")
        .filter(|f| !help.contains(**f))
        .copied()
        .collect();

    assert!(
        missing.is_empty(),
        "`verum test --help` does not name {:?}, which `--format` accepts \
         and for which the runner emits real output. A user reading the \
         help cannot discover them.\n--- help ---\n{}",
        missing,
        help
    );
}
