//! Gate: every error code the compiler can print must be in the registry.
//!
//! The registry calls itself "the single source of truth for what each
//! error code means", and until this gate existed it was not consulted
//! by anything at emission time. Measured on 2026-08-28, before the
//! repair: 69 codes were reachable in compiler output and 41 of them
//! had no registry entry — including five that conformance specs pin.
//!
//! An unregistered code is not a cosmetic gap. Three things read the
//! registry or would like to: `verum explain`, which cannot describe a
//! code it does not know; any consumer routing on category; and the
//! author of the next diagnostic, who picks a number by looking at what
//! is taken. When the registry is incomplete, that last one is how two
//! meanings end up on one code — `E1001` meant BOTH "use-after-free"
//! (verum_cbgr) and "stage mismatch in a quote expression"
//! (verum_compiler's lint table), because neither author could see the
//! other's number.
//!
//! WHY A SOURCE SCAN. The honest alternative is to make the code a type
//! so an unregistered one is unrepresentable, and that is the better end
//! state. It is not reachable today: `DiagnosticBuilder` lives in
//! `verum_diagnostics`, which takes `verum_error` only as an OPTIONAL
//! dependency (the `verum-error-integration` feature), so the builder
//! cannot name the registry. Until that inverts, the check has to live
//! outside the type system. A scan fails in the safe direction — a new
//! spelling it cannot parse shows up as an unrecognised site, never as
//! silent approval.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Files that DEFINE codes rather than emit them.
///
/// The registry lists every code as `code: "E400"`, and the explanation
/// table and the `explain` command are keyed by code as data. Counting
/// those as emission sites is how an early measurement of this same
/// class reported "0 registered-but-unemitted codes" when the real
/// figure was 21: the registry was matching itself.
const DEFINITION_SITES: &[&str] = &[
    "verum_error/src/registry.rs",
    "verum_diagnostics/src/explanations.rs",
    "verum_cli/src/commands/explain.rs",
];

fn is_definition_site(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    DEFINITION_SITES.iter().any(|d| p.ends_with(d))
}

fn is_test_or_example(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    p.contains("/tests/") || p.contains("/examples/") || p.contains("/benches/")
}

// A note on what this scan deliberately does NOT exclude: `#[cfg(test)]`
// modules that sit inside a `src/` file, and codes written in doc
// comments. Both get counted, so a code used only by an in-file test
// fixture still has to be registered. That is the safe direction to be
// imprecise in — over-registration costs one true line in a table, while
// under-detection is the failure this gate exists to prevent. Reading a
// code out of a `//!` example is how E0308 was first misfiled here as
// "refinement constraint not satisfied"; its one real emission site says
// "ambiguous specialization". Check the emission site, not the prose.

/// Every `"Ennn"` literal that is the argument of a code-carrying form.
///
/// The three spellings are the ones the workspace actually uses:
/// `.code("E400")` on a builder, `code: Text::from("E400")` in a
/// TypeError, and `code: "E400"` / `error_code: "E1001"` in a table
/// whose entries reach diagnostics.
fn codes_in(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    // `=> "E001"` is the fourth spelling and the one that mattered: the
    // parser maps an enum to a literal (`ErrorCode::UnterminatedChar =>
    // "E001"`), so 138 codes it can print were invisible here while this
    // gate reported green. Its sibling
    // `registry_covers_parser_codes.rs` asks `ErrorCode::ALL` directly,
    // which is the durable answer; this arm keeps the REVERSE direction
    // (registered-but-unemitted) honest, since without it every parser
    // code reads as dead weight in the table.
    for (pat, skip) in [
        (".code(\"", 7),
        ("code: \"", 7),
        ("Text::from(\"", 12),
        ("=> \"", 4),
    ] {
        let mut rest = source;
        while let Some(at) = rest.find(pat) {
            let after = &rest[at + skip..];
            if let Some(end) = after.find('"') {
                let lit = &after[..end];
                if is_error_code(lit) {
                    // `Text::from("E400")` counts only when it is filling a
                    // `code:` field; a bare Text::from of something that
                    // looks like a code elsewhere is not an emission.
                    let preceding = &rest[..at];
                    let ok = pat != "Text::from(\""
                        || preceding.trim_end().ends_with("code:")
                        || preceding.trim_end().ends_with("code: verum_common::");
                    if ok {
                        found.insert(lit.to_string());
                    }
                }
            }
            rest = &rest[at + skip..];
        }
    }
    found
}

/// Codes written as a PREFIX inside a message string: `"E305: use of …"`.
///
/// This is a third way a code reaches a user, and the one that drifted
/// worst, because it duplicates a code the structure already carries and
/// nothing keeps the copies together. `verum_types`' TypeError→VerumError
/// conversion still needs it — its target variant, `VerumError::Other`,
/// has no code field — so the spelling is not forbidden here, only held
/// to the same registry as every other spelling. It was carrying E3050
/// and E0811 after both had been retired.
fn message_prefix_codes(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find("\"E") {
        let after = &rest[at + 1..];
        if let Some(colon) = after.find(": ") {
            let lit = &after[1..colon];
            if !lit.is_empty() && lit.len() <= 4 && lit.chars().all(|c| c.is_ascii_digit()) {
                found.insert(format!("E{lit}"));
            }
        }
        rest = &rest[at + 2..];
    }
    found
}

fn is_error_code(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next() == Some('E')
        && s.len() >= 4
        && s.len() <= 5
        && chars.all(|c| c.is_ascii_digit())
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if name != "target" && name != ".git" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn crates_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/verum_error.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ is the parent of verum_error")
        .to_path_buf()
}

#[test]
fn every_emitted_code_is_registered() {
    let crates = crates_dir();
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(
        files.len() > 200,
        "expected to walk the whole workspace, found only {} .rs files under {} — \
         the scan is looking in the wrong place and would pass vacuously",
        files.len(),
        crates.display()
    );

    let mut unregistered: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut emitted = BTreeSet::new();
    for file in &files {
        if is_definition_site(file) || is_test_or_example(file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let mut file_codes = codes_in(&text);
        file_codes.extend(message_prefix_codes(&text));
        for code in file_codes {
            emitted.insert(code.clone());
            if !verum_error::registry::is_known(&code) {
                let shown = file
                    .strip_prefix(&crates)
                    .unwrap_or(file)
                    .to_string_lossy()
                    .to_string();
                unregistered.entry(code).or_default().push(shown);
            }
        }
    }

    // A positive control: if the scan silently stopped finding codes, the
    // "nothing unregistered" verdict below would be meaningless.
    assert!(
        emitted.len() >= 50,
        "found only {} emitted codes; the scan is broken, not the code base",
        emitted.len()
    );
    assert!(
        emitted.contains("E400"),
        "E400 (type mismatch) is emitted by the type checker and the scan missed it"
    );

    if !unregistered.is_empty() {
        let mut report = String::from(
            "these error codes are emitted but have no registry entry.\n\
             Add them to crates/verum_error/src/registry.rs — the code a user\n\
             sees has to be one the compiler can explain:\n\n",
        );
        for (code, sites) in &unregistered {
            report.push_str(&format!("  {code}  <- {}\n", sites.join(", ")));
        }
        panic!("{report}");
    }
}

/// The other direction, kept SEPARATE and non-fatal in intent.
///
/// A registered code that nothing emits is not automatically a defect —
/// a code may be reserved ahead of the diagnostic that will carry it.
/// It IS worth knowing about, because it is also what a whole band going
/// stale looks like: the registry's parse family (E001–E007) is emitted
/// by nothing, while `verum_fast_parser` numbers its own errors
/// E010–E099 in a private scheme.
#[test]
fn registered_codes_that_nothing_emits_are_listed() {
    let crates = crates_dir();
    let mut files = Vec::new();
    rust_files(&crates, &mut files);

    let mut emitted = BTreeSet::new();
    for file in &files {
        if is_definition_site(file) || is_test_or_example(file) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(file) {
            emitted.extend(codes_in(&text));
        }
    }

    let unemitted: Vec<&str> = verum_error::registry::REGISTRY
        .keys()
        .filter(|c| !emitted.contains(**c))
        .copied()
        .collect();

    // Not an assertion on the count — that would turn a reservation into
    // a failure. The bound is a sanity one: if MOST of the registry is
    // unemitted, the scan broke rather than the registry.
    assert!(
        unemitted.len() < verum_error::registry::count() / 2,
        "more than half the registry ({} of {}) appears unemitted, which means \
         the scan is not finding emission sites: {:?}",
        unemitted.len(),
        verum_error::registry::count(),
        unemitted
    );
}

#[test]
fn code_literal_recogniser_accepts_both_widths_and_rejects_neighbours() {
    assert!(is_error_code("E400"), "three-digit codes are the main scheme");
    assert!(is_error_code("E1001"), "four-digit lint codes are codes too");
    assert!(!is_error_code("W1003"), "warning codes are a separate namespace");
    assert!(!is_error_code("E40"), "too short to be a code");
    assert!(!is_error_code("E40000"), "too long to be a code");
    assert!(!is_error_code("Error"), "letters after E are not a code");
    assert!(!is_error_code(""), "the empty string is not a code");
}

#[test]
fn message_prefix_codes_are_found_and_ordinary_prose_is_not() {
    let found = message_prefix_codes(r#"format!("E305: use of uninitialized variable '{}'", n)"#);
    assert!(found.contains("E305"), "a code prefixing a message is a code: {found:?}");

    let none = message_prefix_codes(r#"format!("cannot borrow `{}`: it is already borrowed", v)"#);
    assert!(none.is_empty(), "a colon in ordinary prose is not a code: {none:?}");

    let none = message_prefix_codes(r#""Expected: a value""#);
    assert!(none.is_empty(), "a capital E word is not a code: {none:?}");
}

#[test]
fn definition_sites_are_excluded_but_ordinary_files_are_not() {
    assert!(is_definition_site(Path::new("/x/crates/verum_error/src/registry.rs")));
    assert!(is_definition_site(Path::new(
        "/x/crates/verum_diagnostics/src/explanations.rs"
    )));
    assert!(
        !is_definition_site(Path::new("/x/crates/verum_types/src/lib.rs")),
        "the type checker EMITS codes; excluding it would make the gate vacuous"
    );
}
