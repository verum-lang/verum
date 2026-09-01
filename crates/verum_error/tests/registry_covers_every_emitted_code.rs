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

/// EVERY code-shaped literal, however it reaches a diagnostic.
///
/// The four spellings above key on HOW a code is written, and a gate
/// keyed on spelling lags the next spelling.  It did: sixty-eight codes
/// the compiler prints had no entry here, arriving by a fifth route
/// (`Diagnostic::new_error(msg, span, "E0319")` — a positional third
/// argument), a `pub const` table, a `match` arm returning a tag, and a
/// `code:` field on an LSP diagnostic.  `E0319` alone appears 77 times
/// across 24 `core/` files, and `verum explain E0319` answered "Error
/// code not found" — the diagnostic telling the user to look a code up
/// and the lookup denying it exists (T1035).
///
/// So this asks what the literal IS, not how it got there.  Nothing in
/// the workspace spells `"E0319"` for a reason other than the code, so
/// over-detection costs nothing; under-detection is what this whole
/// file exists to prevent.
///
/// The remaining blind spot, stated so it is not mistaken for coverage:
/// a code ASSEMBLED at run time (`format!("E{:04}", n)`, a lookup by
/// index) is invisible to any literal scan.  There are none today.
fn all_code_literals(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in source.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while let Some(off) = line[i..].find('"') {
            let start = i + off + 1;
            let Some(len) = line[start..].find('"') else {
                break;
            };
            let lit = &line[start..start + len];
            if is_error_code(lit) {
                found.insert(lit.to_string());
            }
            i = start + len + 1;
            if i >= bytes.len() {
                break;
            }
        }
    }
    found
}

/// Inline `#[cfg(test)]` modules, blanked out.
///
/// The file walk skips `tests/` directories, but a sentinel written
/// inside `mod tests` in `src/` is not an emission either — `explain`'s
/// own test asserts that `E999` is in NO table, and a scan that counted
/// it would demand the registry register a code whose whole purpose is
/// to be absent.
fn without_inline_tests(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            let mut depth: i32 = 0;
            let mut opened = false;
            let mut cur = Some(line);
            while let Some(l) = cur {
                depth += l.matches('{').count() as i32;
                depth -= l.matches('}').count() as i32;
                if l.contains('{') {
                    opened = true;
                }
                if opened && depth <= 0 {
                    break;
                }
                cur = lines.next();
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_error_code(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('E') | Some('W'))
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
        let text = without_inline_tests(&text);
        let mut file_codes = codes_in(&text);
        file_codes.extend(message_prefix_codes(&text));
        file_codes.extend(all_code_literals(&text));
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
    // Warning codes WERE excluded here, and that exclusion was the reason
    // the registry held no `W` entry at all while the compiler printed at
    // least forty of them.  `verum explain W0319` answered "Error code not
    // found" for a warning a user had just been shown; a warning nobody
    // can look up is exactly as unhelpful as an error nobody can look up
    // (T1035).
    assert!(is_error_code("W1003"), "warnings reach a user and must be lookupable");
    assert!(!is_error_code("E40"), "too short to be a code");
    assert!(!is_error_code("E40000"), "too long to be a code");
    assert!(!is_error_code("Error"), "letters after E are not a code");
    assert!(!is_error_code("X0319"), "only E and W name codes");
    assert!(!is_error_code(""), "the empty string is not a code");
}

/// The spelling-independent scan must SEE a code the four spellings miss,
/// and must stay quiet on prose.
///
/// Without the first assertion this test would pass on a scanner that
/// found nothing, which is what the four-spelling scanner did for
/// sixty-eight codes.
#[test]
fn every_literal_is_found_however_the_code_reaches_the_diagnostic() {
    let positional = all_code_literals(
        r#"Diagnostic::new_error(message.to_string(), span, "E0319")"#,
    );
    assert!(
        positional.contains("E0319"),
        "a code passed as a positional argument is still a code: {positional:?}"
    );
    let constant = all_code_literals(r#"pub const W0319: &str = "W0319";"#);
    assert!(
        constant.contains("W0319"),
        "a code declared as a constant is still a code: {constant:?}"
    );
    let arm = all_code_literals(r#"Self::NoBoundVarsReferenced => "W502","#);
    assert!(
        arm.contains("W502"),
        "a code returned from a match arm is still a code: {arm:?}"
    );
    let prose = all_code_literals(r#"let msg = "cannot borrow `x` as mutable";"#);
    assert!(prose.is_empty(), "ordinary strings are not codes: {prose:?}");
    let commented = all_code_literals(r#"    // historical: "E0811" was retired"#);
    assert!(
        commented.is_empty(),
        "a code named in a comment is not an emission: {commented:?}"
    );
}

/// An inline `#[cfg(test)]` module is not an emission site.
#[test]
fn inline_test_modules_are_not_scanned() {
    let src = "fn real() { let c = \"E400\"; }\n\
               #[cfg(test)]\n\
               mod tests {\n\
               fn t() { let sentinel = \"E999\"; }\n\
               }\n\
               fn also_real() { let c = \"E401\"; }\n";
    let kept = all_code_literals(&without_inline_tests(src));
    assert!(kept.contains("E400") && kept.contains("E401"), "{kept:?}");
    assert!(
        !kept.contains("E999"),
        "a sentinel inside `mod tests` must not demand a registry entry: {kept:?}"
    );
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

// ============================================================================
// AGREEMENT, which is a different question from COVERAGE
// ============================================================================
//
// Everything above asks "is this code in the table".  That question was
// answered YES for seven codes whose description contradicted what the
// compiler prints with them:
//
//     E402  table: "Send bound not satisfied"   printed: "module `X` not found"
//     E401  table: "invalid cast"               printed: "cannot find X in module Y"
//     E404  table: "missing protocol impl"      printed: "Ambiguous type… annotate"
//     E403  table: "Sync bound not satisfied"   printed: "Infinite type: _ = X<_>"
//     E407  table: "recursive type…"            printed: "takes 1 type arg, got 2"
//     E310  table: "use after move"             printed: "cannot borrow as mutable"
//     E102  table: "undefined function"         printed: "method expects 2 args"
//
// A green coverage gate over that is worse than no gate: its colour reads
// as "the registry was checked", so nobody checks it.  `verum explain
// E402` told users about thread-safety while their error was a missing
// module (T1035).
//
// The pairs below are the compiler's OWN messages, taken from a sweep of
// 372 core/ files (2195 diagnostics, 0 mute).  The check is deliberately
// coarse — one shared word — because the alternative is pinning prose,
// which drifts on every rewording.  A code whose description shares
// NOTHING with its message is the failure this catches, and it catches
// every one of the seven above.

/// Words that appear in both prose and diagnostics without meaning anything.
const STOPWORDS: &[&str] = &[
    "type", "this", "that", "with", "from", "into", "here", "value", "used",
    "also", "which", "cannot", "does", "have", "been", "must", "will", "there",
];

fn significant_words(s: &str) -> BTreeSet<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(w))
        .map(|w| w.trim_end_matches('s').to_string())
        .collect()
}

/// Two words agree when one is a prefix of the other and the shorter is
/// at least five characters.
///
/// Plain set intersection was tried first and reported E405 as drifted:
/// the table said "not implemented", the message says "does not
/// implement", and nothing else was shared.  A checker that calls a
/// correct description wrong teaches its readers to silence it — which
/// is how the coverage gate beside this one came to be trusted while it
/// was answering a different question.
fn words_agree(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 5 && long.starts_with(short)
}

fn descriptions_agree(description: &str, message: &str) -> bool {
    let d = significant_words(description);
    let m = significant_words(message);
    d.iter().any(|x| m.iter().any(|y| x == y || words_agree(x, y)))
}

/// (code, a message the compiler really prints with it).
const MEASURED_PAIRS: &[(&str, &str)] = &[
    ("E100", "unbound variable: ptr_write"),
    ("E101", "type not found: Size"),
    ("E102", "method `f` expects 2 argument(s), but 1 were provided"),
    ("E103", "field 'kind' not found on type 'Finding'"),
    ("E310", "cannot borrow `x` as mutable because it is already borrowed"),
    ("E321", "potential stack overflow: unbounded recursion detected"),
    ("E400", "Type mismatch: expected 'Unit', found 'DbError'"),
    ("E401", "cannot find `Duration` in module `core.sys.io_engine`"),
    ("E402", "module `core.redis` not found"),
    ("E403", "Infinite type: _ = TrieNode<_>"),
    ("E404", "Ambiguous type for `x`: the inferred type is not fully determined"),
    ("E405", "type `T` does not implement `P`, required by this call's bound"),
    ("E407", "type `X` takes 1 type argument(s), but 2 were supplied"),
    ("E409", "Cannot dereference non-reference type: Process"),
    ("E412", "not a function: Heap"),
    ("E0319", "theorem 'T' proof failed verification: 1 unproved goal(s)"),
    ("E0601", "non-exhaustive patterns: `X` not covered"),
];

#[test]
fn a_registered_description_agrees_with_what_the_compiler_prints() {
    let mut disagree = Vec::new();
    let mut absent = Vec::new();
    for (code, message) in MEASURED_PAIRS {
        // "absent" is a THIRD outcome, kept apart from "disagrees": a code
        // missing from the table is the coverage gate's finding, and
        // folding it in here would report one defect as two.
        let Some(entry) = verum_error::registry::lookup(code) else {
            absent.push(*code);
            continue;
        };
        if !descriptions_agree(entry.description, message) {
            disagree.push(format!(
                "  {code}\n    table:   {}\n    printed: {message}",
                entry.description
            ));
        }
    }
    assert!(
        absent.is_empty(),
        "these codes have a measured message and no registry entry: {absent:?}"
    );
    assert!(
        disagree.is_empty(),
        "these registry descriptions share no word with the message the \
         compiler prints under that code.\n`verum explain` would tell the \
         user about something else entirely:\n\n{}",
        disagree.join("\n")
    );
}

/// The check must be able to come back POSITIVE.
///
/// Without this, `descriptions_agree` returning `true` unconditionally
/// would look exactly like a consistent registry — and the coverage gate
/// it sits beside failed in precisely that way for seven codes.
#[test]
fn the_agreement_check_rejects_a_description_that_means_something_else() {
    assert!(
        descriptions_agree("module not found", "module `core.redis` not found"),
        "a description sharing its subject with the message must pass"
    );
    assert!(
        !descriptions_agree("Send bound not satisfied", "module `core.redis` not found"),
        "the real E402 drift must be REJECTED, or this check measures nothing"
    );
    assert!(
        !descriptions_agree("invalid cast", "cannot find `D` in module `m`"),
        "the real E401 drift must be REJECTED"
    );
    assert!(
        !descriptions_agree("use after move", "cannot borrow `x` as mutable"),
        "the real E310 drift must be REJECTED — 'cannot' is a stopword \
         precisely so that pairs like this cannot pass on filler"
    );
}
