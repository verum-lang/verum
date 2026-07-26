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
    P::DoubleEndedIterator,
    P::FromIterator,
    P::Extend,
    P::Future,
    P::Stream,
    P::AsyncIterator,
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

/// `core.base.iterator` -> `<repo>/core/base/iterator.vr`
fn source_file_for(module_path: &str) -> std::path::PathBuf {
    let rel = module_path.strip_prefix("core.").unwrap_or(module_path);
    let mut p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("core");
    for seg in rel.split('.') {
        p = p.join(seg);
    }
    p.set_extension("vr");
    p
}

/// The paths were established by scanning `core/` by hand. Nothing kept them
/// honest afterwards: renaming `core/io/protocols.vr` would leave the shape
/// assertions above perfectly green while the path silently rotted, and a
/// path that resolves to nothing fails at load time and blames the loader.
///
/// This is the protocol-side analogue of
/// `canonical_archive_modules_match_source` in
/// `verum_compiler::archive_ctx_loader`, which enforces the same property
/// for the type-side method.
#[test]
fn every_path_points_at_a_file_that_declares_the_protocol() {
    for &p in ALL {
        let Some(&module_path) = p.canonical_archive_modules().first() else {
            continue; // undeclared — covered by the test above
        };
        let file = source_file_for(module_path);
        let src = std::fs::read_to_string(&file).unwrap_or_else(|e| {
            panic!(
                "{} claims {module_path}, which maps to {} — unreadable: {e}. \
                 If the module moved, update the arm in well_known_types.rs.",
                p.as_str(),
                file.display()
            )
        });

        // `type Name is protocol` / `public type Name is protocol`, tolerating
        // a generic parameter list — `From` is declared as `From<T>`.
        let declared = src.lines().any(|line| {
            let t = line.trim_start();
            let rest = t
                .strip_prefix("public type ")
                .or_else(|| t.strip_prefix("type "));
            rest.is_some_and(|rest| {
                rest.strip_prefix(p.as_str()).is_some_and(|after| {
                    let after = match after.find('>') {
                        Some(i) if after.starts_with('<') => &after[i + 1..],
                        _ => after,
                    };
                    after.trim_start().starts_with("is protocol")
                })
            })
        });

        assert!(
            declared,
            "{} claims {module_path}, but {} does not declare it. The authority \
             must name the file that actually holds the declaration — a path \
             that resolves to nothing fails at load time and misdirects the \
             blame to the loader.",
            p.as_str(),
            file.display()
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
    assert_eq!(ALL.len(), 28, "WellKnownProtocol variant count changed");
}
