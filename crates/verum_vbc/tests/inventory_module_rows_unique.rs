//! Ratchet: `core-tests/INVENTORY.md` must name each module ONCE.
//!
//! CLAUDE.md calls that file the per-module conformance truth, and a truth
//! table cannot hold two truths for one module.  It currently does, for
//! seventeen of them, and the pairs disagree: `text/text` appears at two
//! places with different LOC counts and different open-issue descriptions,
//! so a reader gets whichever they grep first with nothing saying the other
//! exists.
//!
//! This matters for T0220, whose acceptance is a gate that re-verifies every
//! interp row.  Such a gate would rerun seventeen modules twice and then have
//! to choose which row to update — or update one and leave its twin stale,
//! which is the exact failure T0220 exists to end.  Deduplicating has to come
//! first, and unlike the re-verification half this check needs no suite run,
//! no bake and no time budget: it is one pass counting names.
//!
//! Freezing rather than fixing, for the same reason as the sibling gates on
//! silent-nil intrinsics and unresolvable mounts: merging a duplicate pair
//! means deciding which figures are current, and that is a measurement, not
//! an edit.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn inventory_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../core-tests/INVENTORY.md"
    ))
}

/// Module names appearing more than once today, with their occurrence count.
/// Deduplicating one means deleting its line here; the staleness test fails if
/// a listed name stops being duplicated, so the list only shrinks on purpose.
const KNOWN_DUPLICATE_ROWS: &[(&str, usize)] = &[
    ("encoding/varint", 2),  // lines 31, 97
    ("logic/linear", 2),  // lines 35, 96
    ("proof/pcc", 2),  // lines 32, 98
    ("sync/atomic", 2),  // lines 92, 413
    ("sync/once", 2),  // lines 33, 412
    ("text/builder", 2),  // lines 193, 309
    ("text/case_fold", 2),  // lines 192, 308
    ("text/char", 2),  // lines 191, 307
    ("text/format", 2),  // lines 196, 310
    ("text/numeric/bigdecimal", 2),  // lines 201, 315
    ("text/numeric/bigint", 2),  // lines 200, 314
    ("text/numeric/decimal", 2),  // lines 199, 313
    ("text/numeric/modular", 2),  // lines 203, 317
    ("text/numeric/rational", 2),  // lines 202, 316
    ("text/regex", 2),  // lines 197, 311
    ("text/tagged_literals", 2),  // lines 198, 312
    ("text/text", 2),  // lines 190, 306
];

/// Module names in table order.  A row is a line starting `| ` followed by a
/// backticked name — the same shape every module row in the file uses.
fn module_rows(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('|')?.trim_start();
            let rest = rest.strip_prefix('`')?;
            let end = rest.find('`')?;
            let name = &rest[..end];
            // A real module row has a further column after the name.
            rest[end..].contains('|').then(|| name.to_string())
        })
        .collect()
}

fn counts() -> BTreeMap<String, usize> {
    let text = std::fs::read_to_string(inventory_path()).expect("read INVENTORY.md");
    let rows = module_rows(&text);
    assert!(
        rows.len() > 300,
        "parsed only {} module rows — the shape of the table changed, and a \
         parser that matches nothing would pass this gate vacuously",
        rows.len()
    );
    let mut c: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *c.entry(r).or_default() += 1;
    }
    c
}

#[test]
fn no_new_duplicate_module_rows_in_inventory() {
    let counts = counts();
    let known: BTreeMap<&str, usize> = KNOWN_DUPLICATE_ROWS.iter().copied().collect();

    let mut unexpected: Vec<String> = counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .filter_map(|(name, n)| {
            let n = *n;
            match known.get(name.as_str()).copied() {
                Some(was) if was == n => None,
                Some(was) => Some(format!("{name}: {n} rows (was {was})")),
                None => Some(format!("{name}: {n} rows (new)")),
            }
        })
        .collect();
    unexpected.sort();

    assert!(
        unexpected.is_empty(),
        "{} module(s) are listed more than the frozen count in \
         core-tests/INVENTORY.md:\n  {}\n\nA module has one conformance \
         status. Merge the rows, or update KNOWN_DUPLICATE_ROWS with why two \
         are needed.",
        unexpected.len(),
        unexpected.join("\n  ")
    );
}

#[test]
fn the_known_duplicate_list_has_no_stale_entries() {
    let counts = counts();
    let fixed: Vec<String> = KNOWN_DUPLICATE_ROWS
        .iter()
        .copied()
        .filter_map(|(name, frozen)| {
            let now = counts.get(name).copied().unwrap_or(0);
            (now != frozen).then(|| format!("{name}: frozen at {frozen}, now {now}"))
        })
        .collect();

    assert!(
        fixed.is_empty(),
        "{} entry/entries no longer match their frozen count and must be \
         updated or deleted so the ratchet keeps tightening:\n  {}",
        fixed.len(),
        fixed.join("\n  ")
    );
}
