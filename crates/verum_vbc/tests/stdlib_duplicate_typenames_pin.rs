//! Pins how many stdlib type and protocol names are declared more than once.
//!
//! These duplicates are the FUEL for the metadata-merge shadowing tracked as
//! T0327: `CoreMetadata.types` and `.protocols` are keyed by the SIMPLE name,
//! and archive descriptors carry no module qualification at all (measured:
//! 61958 descriptor names, none containing a dot). Loading two same-named
//! declarations therefore lets the later one displace the earlier.
//!
//! The mechanism described here used to be attributed to `core_loader.rs`,
//! which was deleted as unreachable (T0190) — every public entry point had
//! zero callers, so its policy never ran. The live emitter is
//! `verum_compiler::archive_metadata`, and its policy is NOT the same: the
//! type side applies a ranked collision policy plus a qualified
//! `<module>.<Name>` key (MOUNT-TYPE-AUTHORITY-1), and the protocol side is
//! FIRST-wins (`meta.protocols.entry(..).or_insert_with(..)`), not the
//! unconditional overwrite the old note claimed. Which declaration survives a
//! duplicate is therefore still load-order dependent, which is what T0327
//! tracks; the exact live policy is T0327's to characterise, not this pin's.
//!
//! This pin does not fix that. It bounds it: a new duplicate cannot be
//! introduced without someone seeing this test fail and deciding whether the
//! name is genuinely distinct. The protocol list is small enough to enumerate,
//! so it is enumerated — an unexpected NAME is more informative than a count.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn core_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("core")
}

fn vr_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            vr_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "vr") {
            out.push(p);
        }
    }
}

/// Simple name -> how many declarations, for `type X is ...` declarations.
fn declared_names(protocols_only: bool) -> BTreeMap<String, usize> {
    let mut files = Vec::new();
    vr_files(&core_dir(), &mut files);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for f in files {
        let Ok(src) = std::fs::read_to_string(&f) else { continue };
        for line in src.lines() {
            let t = line.trim_start();
            let rest = t
                .strip_prefix("public type ")
                .or_else(|| t.strip_prefix("type "));
            let Some(rest) = rest else { continue };
            let is_protocol = rest.contains(" is protocol");
            if is_protocol != protocols_only {
                continue;
            }
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                *counts.entry(name).or_default() += 1;
            }
        }
    }
    counts
}

#[test]
fn duplicated_protocol_names_are_the_known_set() {
    let dupes: Vec<String> = declared_names(true)
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name)
        .collect();

    // Every one of these is silently shadowed at metadata-merge time, with
    // the winner decided by load order. Write, Numeric and Module are core protocols.
    let expected = [
        "DecidableEq", "Limit",
    ];
    assert_eq!(
        dupes,
        expected,
        "the set of duplicated stdlib PROTOCOL names changed.\n\
         the protocol slot in archive_metadata.rs is first-wins, so each of \
         these is resolved by load order (T0327).\n\
         If you ADDED one, give it a distinct name or fix the merge first. \
         If you REMOVED one, shrink this list."
    );
}

#[test]
fn duplicated_type_names_do_not_grow() {
    let dupes = declared_names(false);
    let count = dupes.values().filter(|n| **n > 1).count();

    // Measured 2026-07-26. Only Sum-kind duplicates are qualified on
    // collision; every other kind is overwritten by the later declaration.
    const KNOWN: usize = 156;
    assert!(
        count <= KNOWN,
        "duplicated stdlib type names grew from {KNOWN} to {count}. \
         Each duplicate is a candidate for silent shadowing in \
         CoreMetadata.types, which is keyed by simple name (T0327)."
    );
}
