//! Pins `WellKnownProtocol::canonical_archive_modules`.
//!
//! The method exists to remove an asymmetry: `WellKnownType` has always
//! known which module declares it, `WellKnownProtocol` did not, and
//! consumers grew private tables to compensate — the one in
//! `verum_types::infer::modules` being open-ended by construction ("Add
//! more as needed").
//!
//! These tests guard the two properties that make it usable as the single
//! authority: every variant is answered, and the five variants with no
//! declaration in `core/` answer honestly rather than with a guess.

use verum_common::well_known_types::WellKnownProtocol as P;

/// Every variant this crate knows about. Kept explicit rather than derived
/// so that adding a variant fails HERE — with a name to think about —
/// instead of silently acquiring whatever the new match arm happens to say.
const ALL: &[P] = &[
    P::Copy,
    P::Clone,
    P::Eq,
    P::Ord,
    P::Hash,
    P::Default,
    P::Debug,
    P::Display,
    P::Drop,
    P::From,
    P::Into,
    P::Iterator,
    P::IntoIterator,
    P::Future,
    P::Stream,
    P::Error,
    P::Send,
    P::Sync,
    P::Write,
    P::Read,
    P::Drawable,
    P::Printable,
    P::Hashable,
    P::Comparable,
];

/// Variants with no `type <Name> is protocol` declaration anywhere in
/// `core/`, verified by scanning every `.vr` file with a generic-tolerant
/// pattern. They must return an EMPTY slice: a guessed path would resolve
/// to nothing at load time and misdirect the blame to the loader.
const UNDECLARED: &[P] = &[P::Error, P::Drawable, P::Printable, P::Hashable, P::Comparable];

fn is_undeclared(p: P) -> bool {
    UNDECLARED.iter().any(|u| u.as_str() == p.as_str())
}

#[test]
fn every_declared_protocol_names_a_module() {
    for &p in ALL {
        if is_undeclared(p) {
            continue;
        }
        let mods = p.canonical_archive_modules();
        assert!(
            !mods.is_empty(),
            "{} is declared in core/ but canonical_archive_modules() is empty",
            p.as_str()
        );
        // Mirrors the type-side contract: the source-declared path first,
        // the grandparent bundle second, because the precompiler emits one
        // or the other depending on hierarchy shape.
        assert_eq!(
            mods.len(),
            2,
            "{} should list the declaring module and its bundle fallback, got {mods:?}",
            p.as_str()
        );
        for m in mods {
            assert!(
                m.starts_with("core."),
                "{} names a non-core path {m:?}; the private table this method \
                 replaces mixed core. and std. namespaces and that must not \
                 come back",
                p.as_str()
            );
        }
    }
}

#[test]
fn undeclared_protocols_answer_empty_not_guessed() {
    for &p in UNDECLARED {
        assert!(
            p.canonical_archive_modules().is_empty(),
            "{} has no declaration in core/, so it must answer with an empty \
             slice. If it was just declared, move it out of UNDECLARED here \
             and give it a real path. If instead the variant was dropped, \
             remove it from both lists.",
            p.as_str()
        );
    }
}

#[test]
fn the_variant_list_is_complete() {
    // from_name round-trips every variant, so a variant added to the enum
    // without being added to ALL is caught by the count.
    for &p in ALL {
        assert_eq!(
            P::from_name(p.as_str()),
            Some(p),
            "{} does not round-trip through from_name",
            p.as_str()
        );
    }
    assert_eq!(ALL.len(), 24, "WellKnownProtocol variant count changed");
}
