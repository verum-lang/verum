//! Every code snippet this crate suggests must be Verum, not Rust (T1142).
//!
//! The suggester exists to teach newcomers the difference between the two
//! languages. Until T1142 it emitted, among others:
//!
//! ```text
//!   {value}.parse::<Int>()?        turbofish; Verum has parse_int()
//!   Maybe::Some({value})           `::`; core/ writes Maybe.Some(x)
//!   <Self as Trait>::Assoc         no such form; Verum says Self.Assoc
//!   using {mod}::{sym};            `using [..]` is the CONTEXT clause,
//!                                  not an import — that is `mount a.b;`
//!   {T}::with_capacity(n)          Verum: List.with_capacity(n)
//!   -> Option<T>                   Verum has no Option; it is Maybe<T>
//!   {value}.into_array()           method and target type both fictional
//!
//! Two of those were PINNED by tests asserting the Rust spelling, so the
//! suite did not merely miss the defect — it required it. That is the
//! reason this file scans the source instead of calling the ~46 template
//! functions one by one: a per-function assertion is exactly what failed,
//! because each one only ever knew about the snippet in front of it.
//!
//! Scanning source from a test is unusual. It is the right shape here
//! because the defect class is literally "a string literal in this file
//! spells Verum wrong", and a new template added tomorrow is caught
//! without anyone remembering to extend a list.

/// The two files that emit Verum source for users to copy.
const SUGGESTION_RS: &str = include_str!("../src/suggestion.rs");
const RECOVERY_RS: &str = include_str!("../src/recovery.rs");

/// Pull out the string literals that become user-facing Verum code.
///
/// Deliberately over-collects rather than under-collects: a literal that
/// is not really emitted code costs a false alarm someone can silence in
/// a line, while a missed one costs a user a snippet that does not parse.
fn emitted_snippets(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let is_emit_site = line.contains(".code(") || line.contains("template:");
        // A `format!` argument list puts the format string on its own
        // line; `.code(format!(` then sits on the line above. Catch a
        // bare string literal that carries `{}` placeholders too.
        let is_bare_format_arg = trimmed.starts_with('"') && trimmed.contains("{}");
        if !(is_emit_site || is_bare_format_arg) {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            out.push((lineno, after[..close].to_string()));
            rest = &after[close + 1..];
        }
    }
    out
}

/// Spellings that belong to Rust and have a different form in Verum.
///
/// `Result` is absent on purpose: Verum has `Result<T, E>` and core/ uses
/// it throughout. Listing it would be the mirror defect — a gate that
/// rejects correct code.
const RUST_ONLY: &[(&str, &str)] = &[
    ("::", "Verum has no path separator `::` — types and modules use `.`"),
    ("Option<", "Verum's optional type is `Maybe<T>`"),
    ("Vec<", "Verum's growable sequence is `List<T>`"),
    ("HashMap", "Verum's map is `Map<K, V>`"),
    ("HashSet", "Verum's set is `Set<T>`"),
    ("Box::", "Verum allocates with `Heap.new(x)`"),
    ("Box<", "Verum's heap pointer is `Heap<T>`"),
];

fn check(file: &str, src: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (lineno, snippet) in emitted_snippets(src) {
        for (needle, why) in RUST_ONLY {
            if snippet.contains(*needle) {
                problems.push(format!(
                    "{file}:{lineno} emits `{snippet}` containing `{needle}` — {why}"
                ));
            }
        }
    }
    problems
}

#[test]
fn suggested_snippets_contain_no_rust_only_spelling() {
    let mut problems = check("src/suggestion.rs", SUGGESTION_RS);
    problems.extend(check("src/recovery.rs", RECOVERY_RS));

    assert!(
        problems.is_empty(),
        "suggestions must be written in Verum, not Rust:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn the_scanner_can_actually_fail() {
    // The test above asserts an ABSENCE, which passes for free if the
    // extractor silently matches nothing. This is its positive control:
    // a source fragment that must be reported, in both emit spellings.
    let planted = r#"
        fn a() { SuggestionBuilder::new("x").code(format!("Maybe::Some({})", v)) }
        fn b() { TypeConversion { template: "{value}.to_vec()".into(), to: "Vec<T>".into() } }
    "#;
    let problems = check("planted.rs", planted);
    assert!(
        problems.len() >= 2,
        "the scanner failed to flag deliberately bad snippets — it is \
         reporting clean because it sees nothing, not because there is \
         nothing. got: {problems:?}"
    );
}

#[test]
fn the_scanner_reads_a_real_and_non_empty_surface() {
    // Second half of the control: the real files must yield snippets at
    // all. If a refactor moves the templates elsewhere, this test fails
    // loudly rather than letting the suite report a clean scan of an
    // empty set.
    let n = emitted_snippets(SUGGESTION_RS).len() + emitted_snippets(RECOVERY_RS).len();
    assert!(
        n >= 40,
        "expected the suggestion sources to yield a substantial number of \
         emitted snippets, found {n} — the extractor is looking at the \
         wrong thing, or the templates have moved"
    );
}
