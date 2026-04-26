//! Guardrail: every public stdlib type must have a unique simple name
//! across all of `core/`.
//!
//! Background. The VBC codegen indexes the variant-constructor table by
//! simple type name (`Type.Variant`).  Two stdlib `public type Foo is …`
//! declarations with the same simple name in different modules collide:
//! `register_type_constructors` runs with `prefer_existing_functions =
//! true` (stdlib loading mode), hits the `has_variants_for_type` first-
//! wins gate on the second module, and silently skips the second type's
//! variant registration entirely.
//!
//! Symptom: bodies that write `MyType { lock_state: Unlocked, … }`
//! compile to `[lenient] SKIP <fn>: undefined variable: Unlocked` and
//! disappear from the runtime function table.  Callers later panic with
//! `method 'X.Y' not found on value`, far from the source.
//!
//! Concrete past incident (task #160): three stdlib modules each defined
//! `public type LockKind`:
//!
//!   * `core/database/sqlite/native/l0_vfs/vfs_protocol.vr` —
//!     5-state SQLite VFS protocol (Unlocked | Shared | Reserved |
//!     Pending | Exclusive)
//!   * `core/sys/common.vr` — fcntl byte-range lock
//!     (Shared | Exclusive | Unlock)
//!   * `core/sys/locking/mod.vr` — high-level file lock
//!     (Shared | Exclusive)
//!
//! The first to register won the simple-name slot.  Whichever module
//! lost had its variants invisible to every body that referenced them
//! by simple name, including bodies inside its own module.
//!
//! History. This test was originally a *ratchet* — it carried a
//! `BASELINE_DUPLICATES` array of known-bad type names from before the
//! invariant was enforced and only flagged NEW duplicates while letting
//! known ones shrink over time.  Task #162 (closed) drove the baseline
//! from 315 entries down to zero across roughly 165 disambiguation
//! commits.  With every public stdlib type now uniquely named, the
//! ratchet plumbing is dead weight; this test is now a plain hard
//! invariant: any duplicate, old or new, fails.
//!
//! When this test fails. Pick a domain-prefixed disambiguated name
//! following the existing stdlib convention:
//!
//!   * Catalogue scope: `<CatalogueName><Type>` —
//!     `BtreeBalanceStrategy`, `JournalTransitionMode`, `SqlOnConflict`.
//!   * Layer pair (FFI vs runtime): `Raw<Type>` for the FFI-side —
//!     `RawJoinHandleOpaque`, `RawExecutorHandle`.
//!   * Platform-conditional: `Linux<Type>` / `Darwin<Type>` /
//!     `Windows<Type>` for the @cfg(target_os = …) variants.
//!   * Architecture-conditional: `Aarch64<Type>` / `X86<Type>`
//!     for @cfg(target_arch = …) variants.
//!   * Prefer the broader-scope or foundational module as canonical
//!     (e.g. `core.metrics.instrument::Counter` > sqlite-internal
//!     observability counter).
//!
//! Update every importer along with the rename — vcs/ smokes that
//! mounted from the renamed site need their import path adjusted too.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = "core";

#[test]
fn stdlib_public_type_names_are_unique() {
    let root = workspace_root().join(ROOT);
    assert!(
        root.is_dir(),
        "expected stdlib root at {} but it does not exist",
        root.display()
    );

    // name -> List<(module-path, file-path, line)>
    let mut definitions: BTreeMap<String, Vec<(String, PathBuf, usize)>> = BTreeMap::new();
    walk_dir(&root, &root, &mut definitions);

    let mut violations: Vec<String> = Vec::new();
    for (name, sites) in &definitions {
        if sites.len() < 2 {
            continue;
        }
        let mut entry = format!(
            "duplicate: type `{}` is declared in {} stdlib modules:\n",
            name,
            sites.len()
        );
        for (modpath, file, line) in sites {
            entry.push_str(&format!("    - {} ({}:{})\n", modpath, file.display(), line));
        }
        violations.push(entry);
    }

    if !violations.is_empty() {
        panic!(
            "{} duplicated public type name(s) in stdlib.  Each public \
             stdlib type must have a unique simple name across all of \
             `core/`, because the VBC variant-constructor table is keyed \
             by simple name.  Two `public type Foo is …` declarations \
             collide and the second's variants are silently skipped, \
             surfacing later as runtime `method 'X.Y' not found on value` \
             panics.\n\n\
             Pick a domain-prefixed disambiguated name following the \
             existing stdlib convention — see this file's module-level \
             docs for the catalogue / layer-pair / platform / arch \
             conventions and how to choose which site stays canonical.\n\n\
             {}",
            violations.len(),
            violations.join("\n"),
        );
    }
}

/// Walk a directory recursively, collecting `public type Name is …`
/// declarations.  Skips test-helper paths under `vcs/`, generated
/// `target/` directories, and anything under `.git/`.
fn walk_dir(
    repo_core_root: &Path,
    dir: &Path,
    sink: &mut BTreeMap<String, Vec<(String, PathBuf, usize)>>,
) {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(repo_core_root, &path, sink);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            scan_file(repo_core_root, &path, sink);
        }
    }
}

/// Scan a single `.vr` file for `public type Name is …` declarations.
/// Records each match under its simple name in `sink`.
///
/// Heuristic-only — the test is intentionally lenient about formatting
/// (whitespace, generics, body kind).  Excludes `protocol` types since
/// they cannot collide via the variant-constructor mechanism (no
/// constructors).
fn scan_file(
    repo_core_root: &Path,
    file: &Path,
    sink: &mut BTreeMap<String, Vec<(String, PathBuf, usize)>>,
) {
    let contents = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let module_path = file_to_module_path(repo_core_root, file);
    for (lineno, line) in contents.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("public type ") {
            continue;
        }
        let after_kw = &trimmed["public type ".len()..];
        // Strip leading optional `affine`/`unique` qualifiers.
        let after_kw = after_kw
            .strip_prefix("affine ")
            .or_else(|| after_kw.strip_prefix("unique "))
            .unwrap_or(after_kw);
        // Take the type name up to first non-identifier character.
        let mut end = 0;
        for (i, c) in after_kw.char_indices() {
            if c.is_ascii_alphanumeric() || c == '_' {
                end = i + c.len_utf8();
            } else {
                break;
            }
        }
        if end == 0 {
            continue;
        }
        let name = &after_kw[..end];
        // Skip protocol types — `public type Foo is protocol { … }` —
        // they have no variant constructors so cannot trip this bug.
        let rest = after_kw[end..].trim_start();
        let rest = rest.strip_prefix('<').map_or(rest, |r| {
            // Skip generic params `<…>`.
            let mut depth = 1;
            let mut cut = 0;
            for (i, c) in r.char_indices() {
                match c {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            cut = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &r[cut..]
        });
        let rest = rest.trim_start().strip_prefix("is").unwrap_or(rest).trim_start();
        if rest.starts_with("protocol") {
            continue;
        }
        sink.entry(name.to_string()).or_default().push((
            module_path.clone(),
            file.to_path_buf(),
            lineno + 1,
        ));
    }
}

/// Convert a file path under `core/` into its dotted module path:
///
///   core/sys/common.vr           -> core.sys.common
///   core/sys/locking/mod.vr      -> core.sys.locking
fn file_to_module_path(repo_core_root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(repo_core_root).unwrap_or(file);
    let mut parts: Vec<String> = vec!["core".to_string()];
    for component in rel.components() {
        if let std::path::Component::Normal(s) = component
            && let Some(s) = s.to_str()
        {
            let s = s.strip_suffix(".vr").unwrap_or(s);
            parts.push(s.to_string());
        }
    }
    let joined = parts.join(".");
    joined.trim_end_matches(".mod").to_string()
}

fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        if ancestor.join("Cargo.lock").is_file() && ancestor.join(ROOT).is_dir() {
            return ancestor.to_path_buf();
        }
    }
    panic!(
        "workspace root with Cargo.lock and {ROOT}/ not found from {}",
        crate_dir.display()
    );
}
