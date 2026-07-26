//! Pins `WellKnownType::canonical_archive_modules` against the `core/` sources.
//!
//! The type side already had a pin — `canonical_archive_modules_match_source`
//! in `verum_compiler::archive_ctx_loader` — but it checks the built ARCHIVE
//! and its contract is that **at least one** entry per type resolves. Every
//! arm lists a specific module first and a bundle fallback second
//! (`core.collections`, `core.sync`, `core.base`), and the fallback always
//! resolves, so a wrong PRIMARY is invisible by design.
//!
//! SIX had accumulated behind it: BTreeMap and BTreeSet pointing at
//! `btree_map`/`btree_set` where the declaration is `btree`, BinaryHeap at
//! `binary_heap` where it is `heap`, WaitGroup at `wait_group` where it is
//! `waitgroup`, Range at `base.range` where it is `base.iterator`, and
//! Semaphore at `core.async.semaphore` where it is `core.sync.semaphore` —
//! the wrong SUBSYSTEM, not merely the wrong file. This test is what would
//! have caught them, and it runs against sources in milliseconds with no
//! archive required.

use std::path::{Path, PathBuf};

fn core_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("core")
}

/// A module path resolves to `<core>/a/b.vr`, or — when the module is a
/// bundle — to `<core>/a/b/mod.vr`. `Never => "core.base"` is the legitimate
/// bundle case and must not be mistaken for a broken path.
fn source_of(module_path: &str) -> Option<(PathBuf, String)> {
    let rel = module_path.strip_prefix("core.").unwrap_or(module_path);
    let mut base = core_dir();
    for seg in rel.split('.') {
        base = base.join(seg);
    }
    for cand in [base.with_extension("vr"), base.join("mod.vr")] {
        if let Ok(src) = std::fs::read_to_string(&cand) {
            return Some((cand, src));
        }
    }
    None
}

/// `[public] type Name` in any declaration form — record, sum, newtype or
/// alias. Unlike the protocol pin this must NOT require `is protocol`.
fn declares(src: &str, name: &str) -> bool {
    src.lines().any(|line| {
        let t = line.trim_start();
        let rest = t
            .strip_prefix("public type ")
            .or_else(|| t.strip_prefix("type "));
        rest.is_some_and(|rest| {
            rest.strip_prefix(name).is_some_and(|after| {
                // `Name`, `Name<T>`, `Name <T>` — reject `NameOther`.
                let after = after.trim_start();
                after.starts_with("is") || after.starts_with('<')
            })
        })
    })
}

/// Every arm of `canonical_archive_modules`, read out of the source rather
/// than restated here: restating is what let the five wrong paths persist.
fn arms() -> Vec<(String, String)> {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("well_known_types.rs"),
    )
    .expect("well_known_types.rs is readable");

    // Only the WellKnownType impl — the protocol impl has its own pin.
    let head = src
        .split("impl WellKnownProtocol")
        .next()
        .expect("split always yields one part");
    let body = head
        .split("pub const fn canonical_archive_modules")
        .nth(1)
        .expect("WellKnownType::canonical_archive_modules is present");

    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("Self::") {
        rest = &rest[i + 6..];
        let Some(name_end) = rest.find(|c: char| !c.is_alphanumeric() && c != '_') else {
            break;
        };
        let name = rest[..name_end].to_string();
        // First quoted string after this variant, before the next `Self::`.
        let seg_end = rest.find("Self::").unwrap_or(rest.len());
        let seg = &rest[..seg_end];
        if let Some(q0) = seg.find('"') {
            if let Some(q1) = seg[q0 + 1..].find('"') {
                out.push((name, seg[q0 + 1..q0 + 1 + q1].to_string()));
            }
        }
    }
    out
}

#[test]
fn every_primary_path_points_at_a_source_that_declares_the_type() {
    let arms = arms();
    assert!(
        arms.len() >= 25,
        "parsed only {} arms out of canonical_archive_modules — the parser \
         has drifted from the source shape and is no longer checking anything",
        arms.len()
    );

    // Int, Float, Bool, Byte and Char are BUILT-IN — `core/base/primitives.vr`
    // says so in a comment on line 6, and no `.vr` file declares them. The
    // module an arm names for them is where their METHODS live, which is the
    // right answer for the authority to give; there is simply no declaration
    // to match against.
    const BUILTIN: &[&str] = &["Int", "Float", "Bool", "Byte", "Char"];

    let mut broken: Vec<String> = Vec::new();
    for (name, path) in &arms {
        match source_of(path) {
            None => broken.push(format!(
                "{name} claims {path}, which resolves to no .vr file (tried \
                 both <path>.vr and <path>/mod.vr)"
            )),
            Some(_) if BUILTIN.contains(&name.as_str()) => {}
            Some((file, src)) => {
                // A bundle mod.vr re-exports rather than declares, so only
                // require the declaration when the path named a leaf file.
                let is_bundle = file.ends_with("mod.vr");
                if !is_bundle && !declares(&src, name) {
                    broken.push(format!(
                        "{name} claims {path}, but {} does not declare it",
                        file.display()
                    ));
                }
            }
        }
    }

    assert!(
        broken.is_empty(),
        "canonical_archive_modules names modules that do not hold the type. \
         The archive-side pin cannot see this: it accepts a type when ANY \
         entry resolves, and the bundle fallback always does, so a wrong \
         primary hides behind it.\n  {}",
        broken.join("\n  ")
    );
}
