//! T1068 — a protocol implementation written on a built-in scalar must
//! survive the standard-library build.
//!
//! A `ProtocolImpl` is carried by its TARGET TYPE's `TypeDescriptor`.
//! Built-in scalars are declared nowhere, so no module held a
//! descriptor for `Float` / `Int` / `USize` / `Byte`, and codegen's
//! attach site dropped the impl with no diagnostic: measured on the
//! archive as shipped, 3184 protocol impls across 590 modules and NOT
//! ONE descriptor for any of those four.  In `core/` that is 176
//! `implement P for <scalar>` blocks — Float 41, Int 56, Bool 22,
//! USize 35, Char 12, Byte 10 — none of which reached the registry, so
//! `Float` answered "does not implement `Differentiable`" while
//! `core/math/autodiff.vr:108` declared exactly that.
//!
//! This gates the CARRY, one layer below the end-to-end spec
//! (`vcs/specs/L0-critical/types/bounds/primitive_satisfies_a_stdlib_protocol.vr`):
//! if the descriptor stops being emitted, this fails with the count
//! rather than leaving a language-level symptom to be re-diagnosed.

use std::path::{Path, PathBuf};

/// `<target_root>/precompiled-stdlib/runtime.vbca`, derived the same way
/// `verum_compiler`'s build script derives it (`derive_target_root`):
/// an absolute `CARGO_TARGET_DIR` IS the root, otherwise the workspace
/// `target/`.
fn archive_path() -> PathBuf {
    let root = match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target"),
    };
    root.join("precompiled-stdlib").join("runtime.vbca")
}

#[test]
fn primitive_impls_survive_the_bake() {
    let path = archive_path();
    // Absence is not a pass.  The build script writes this file on every
    // build of this crate, so a missing archive means the derivation
    // above rotted — say so instead of skipping.
    assert!(
        path.exists(),
        "baked archive not found at {}; the path derivation must track \
         verum_compiler's build script (`derive_target_root`)",
        path.display()
    );
    let archive =
        verum_vbc::archive::read_archive_from_file(&path).expect("archive reads");

    let mut carried: Vec<(String, usize)> = Vec::new();
    let mut total_impls = 0usize;
    for raw in archive.module_data.iter() {
        let Ok(m) = verum_vbc::deserialize::deserialize_module(raw) else {
            continue;
        };
        for td in m.types.iter() {
            total_impls += td.protocols.len();
            if td.kind != verum_vbc::types::TypeKind::Primitive {
                continue;
            }
            let Some(name) = m.strings.get(td.name) else {
                continue;
            };
            if !td.protocols.is_empty() {
                carried.push((name.to_string(), td.protocols.len()));
            }
        }
    }

    // The positive control lives in the same assertion: `total_impls`
    // proves the scan reached the descriptors at all, so an empty
    // `carried` cannot be read as "the archive was unreadable".
    assert!(
        total_impls > 1000,
        "scan found only {total_impls} protocol impls in the whole archive \
         — the reader is broken, not the carry"
    );

    // `core/` writes 41 `implement ... for Float` blocks.  Pinning the
    // exact number would break on every legitimate stdlib edit; pinning
    // "more than none" is what distinguishes carried from dropped.
    for scalar in ["Float", "Int"] {
        let n: usize = carried
            .iter()
            .filter(|(name, _)| name == scalar)
            .map(|(_, c)| *c)
            .sum();
        assert!(
            n > 0,
            "no protocol impl reached `{scalar}` through the bake; \
             carriers found: {carried:?}"
        );
    }
}
