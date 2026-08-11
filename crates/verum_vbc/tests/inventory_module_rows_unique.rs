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

/// Modules allowed to appear more than once, with their TOTAL row count —
/// the same quantity both tests below compare against, so an entry only ever
/// means "this module legitimately has exactly N rows".
///
/// EMPTIED 2026-08-11 after measuring every entry. The list held 17 names, all
/// frozen at `1`, and a count of 1 is the NON-duplicate case: the duplicate
/// test only inspects modules with `n > 1`, so those entries excused nothing,
/// and the staleness test compares `now != frozen`, so `1 == 1` never tripped
/// either. The list neither permitted anything nor could go stale — it read as
/// an allowance list while being inert, and its `// lines 190, 306` comments
/// had drifted onto unrelated modules. All 17 were verified to have exactly one
/// row today, so nothing is being waived away by removing them; the gate is now
/// simply "one module, one row", which is the actual invariant.
///
/// Add an entry only with a reason two rows are genuinely needed.
const KNOWN_DUPLICATE_ROWS: &[(&str, usize)] = &[];

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
