//! HARDCODED-PHANTOM-TYPENAME-1 gate (T0406).
//!
//! Positional field emission in the VBC codegen must never name its
//! record type with a STRING LITERAL.  A literal type name is checked
//! by nothing: if it does not match a declared type, the resolver
//! cannot say so — it silently degrades to a guess.
//!
//! The concrete failure this gate retires: five inline sequences
//! resolved their fields against `Some("Stopwatch")` and three against
//! `Some("DeadlineTimer")`.  Neither name was ever declared — the
//! Verum stdlib declares `DarwinStopwatch` / `LinuxStopwatch` /
//! `WindowsStopwatch` and `DarwinDeadlineTimer` / `WindowsDeadlineTimer`,
//! and the bare names existed only as mount re-export items that had
//! lost their `as` rename.  The chain that followed:
//!
//!   1. `record_key_is_authoritative("Stopwatch")` → false.
//!   2. `resolve_record_type_key` suffix-scans for a key ending in
//!      `.Stopwatch`; every real key ends in `.DarwinStopwatch`, so the
//!      re-key missed too.
//!   3. Resolution fell to the GLOBAL field-name scan over every type
//!      declaring a field called `start` / `running` / `accumulated` —
//!      types that DISAGREE on position — and the most-fields tie-break
//!      baked one of them as `(idx, guessed = true)`.
//!   4. `type_field_count("Stopwatch")` missed for the same reason, so
//!      the object was allocated with the `unwrap_or(3)` fallback and
//!      `type_id: 0`.  A tie-break winner index >= 3 is therefore an
//!      OUT-OF-BOUNDS heap write ("field write index N exceeds object
//!      data size"), not merely a wrong field.
//!
//! The fix made the bare names real (per-platform `as` renames on the
//! `core/sys/*` time re-exports), which let the call sites resolve to
//! the concrete declared types and their complete Verum method bodies —
//! and deleted the duplicate inline sequences that had carried the
//! literals (continuing the §G single-representation migration that had
//! already removed the `Duration` / `Instant` twins).
//!
//! This gate keeps the class from coming back.  A type name reaching
//! these APIs must come from the program being compiled (a variable
//! carrying an inferred / declared / re-keyed name), never from a
//! literal baked into the compiler — so a name that does not exist is
//! impossible to write in the first place.

use std::fs;
use std::path::{Path, PathBuf};

fn codegen_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/codegen"))
}

/// Recursively collect every `.rs` file under `dir`.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Drop `//` line comments so the prose above — and the historical
/// notes scattered through the codegen that quote these call shapes
/// verbatim (e.g. the task #25 `resolve_field_index(Some("Signal"), …)`
/// walkthrough) — are not mistaken for call sites.
fn strip_line_comments(src: &str) -> String {
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The field-emission APIs that bake a positional layout from a type
/// name.  We match them with the opening `(` and (for the `Some(...)`
/// forms) the wrapper, all whitespace removed, so a reformatted or
/// line-wrapped reintroduction is caught just the same.
const LITERAL_TYPE_NAME_CALLS: &[&str] = &[
    "resolve_field_index(Some(\"",
    "resolve_field_index_flagged(Some(\"",
    "type_field_count(\"",
];

/// Everything after the matched prefix up to the next `"` is the baked
/// type-name literal — recovered only to name the offender.
fn offending_name(after: &str) -> &str {
    after.split('"').next().unwrap_or("?")
}

#[test]
fn no_literal_type_name_at_positional_field_emission_sites() {
    let mut files = Vec::new();
    collect_rs(&codegen_dir(), &mut files);
    assert!(
        !files.is_empty(),
        "gate found no codegen sources under {} — the layout moved and \
         this gate silently stopped checking anything",
        codegen_dir().display()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        // Whitespace-insensitive: strip comments, then remove ALL
        // whitespace so `resolve_field_index(\n  Some("Foo")` collapses
        // to the same needle as the one-line form.
        let compact: String = strip_line_comments(&raw)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let fname = path.file_name().unwrap_or_default().to_string_lossy();
        for pattern in LITERAL_TYPE_NAME_CALLS {
            let mut from = 0;
            while let Some(at) = compact[from..].find(pattern) {
                let start = from + at + pattern.len();
                let name = offending_name(&compact[start..]);
                offenders.push(format!("{fname}: {pattern}{name}\"…"));
                from = from + at + pattern.len();
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "HARDCODED-PHANTOM-TYPENAME-1: {} positional field-emission \
         site(s) name their record type with a string literal:\n  {}\n\n\
         A literal type name is unverifiable — if it names a type that \
         is not declared (or that only exists as a dangling mount \
         re-export), `resolve_field_index` cannot tell you: it falls to \
         the global field-name scan and BAKES a guessed index, while \
         `type_field_count` misses and the object is allocated from the \
         `unwrap_or(N)` fallback.  A guessed index >= the allocated slot \
         count is an out-of-bounds heap write.\n\n\
         Derive the type name from the program being compiled (receiver \
         / declared return type / re-keyed record key) instead.  If the \
         stdlib genuinely lacks the type you wanted to name, declare it \
         in Verum — do not name it from the compiler.",
        offenders.len(),
        offenders.join("\n  ")
    );
}
