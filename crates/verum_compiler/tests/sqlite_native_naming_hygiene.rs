//! Naming-hygiene contract for `core/database/sqlite/native/`.
//!
//! Several catalogue waves shipped public type definitions whose names
//! shadowed stdlib type or variant names — most notably `public type Result
//! is | RNull | RText/RBytes`, which silently overrode `Result<T,E>::{Ok,
//! Err}` for any module that imported the catalogue alongside the stdlib.
//! The result was that run-tests calling `Ok(_)` / `Err(_)` started failing
//! with "Unknown variant constructor 'Ok'. Available variants: [RNull,
//! RText]" — confusing because the catalogue and the test file looked
//! syntactically fine in isolation.
//!
//! This test is a guardrail: it walks `core/database/sqlite/native/` and
//! fails CI when any `.vr` file declares `public type {RESERVED} is …` for
//! any name in the reserved set.  The reserved set is the small list of
//! stdlib identifiers that catalogue authors are most likely to
//! accidentally shadow — types and variant constructors that carry critical
//! "stdlib protocol" meaning.
//!
//! Spec: see `internal/specs/sqlite-native.md` and
//! `memory/feedback_explicit_mount_glob_shadow.md` for prior incidents.

use std::fs;
use std::path::{Path, PathBuf};

/// Root the test walks. Path is relative to the workspace root, which
/// `cargo test` resolves to the repo top-level.
const ROOT: &str = "core/database/sqlite/native";

/// Reserved stdlib names that must never be redefined inside the catalogue
/// tree.  Two classes here:
///   * **Types**:    `Result`, `Maybe`, `List`, `Map`, `Set`, `Bytes`,
///                   `Iterator`.  Defining a sibling type with the same
///                   name silently shadows the stdlib import.
///   * **Variants**: `Ok`, `Err`, `Some`, `None`.  Defining a `public type`
///                   with these as the *type name* doesn't help the
///                   variant lookup either, but the matching variant-level
///                   collision is what historically caused the breakage.
///                   We catch type-level redefinitions here; variant-level
///                   coverage is left to the stricter checker that walks
///                   `type X is | Ok | Err | …` patterns.
const RESERVED_TYPE_NAMES: &[&str] = &[
    "Result", "Maybe", "List", "Map", "Set", "Bytes", "Iterator",
    "Ok", "Err", "Some", "None",
];

#[test]
fn sqlite_native_does_not_shadow_stdlib_types() {
    let root = workspace_root().join(ROOT);
    assert!(
        root.is_dir(),
        "expected catalogue root at {} but it does not exist",
        root.display()
    );

    let mut violations: Vec<String> = Vec::new();
    walk_vr_files(&root, &mut |file_path| {
        let body = match fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                violations.push(format!(
                    "{}: could not read file ({})",
                    relative_to_root(file_path, &root),
                    e
                ));
                return;
            }
        };
        for (line_idx, raw_line) in body.lines().enumerate() {
            // Match `public type NAME is` (with arbitrary spacing).
            // Cheap textual scan — no full Verum parser needed.
            let line = raw_line.trim_start();
            let prefix = "public type ";
            let rest = match line.strip_prefix(prefix) {
                Some(r) => r,
                None => continue,
            };
            // Pull off the identifier ending at the first whitespace, '<',
            // or '('.
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '<' || c == '(')
                .unwrap_or(rest.len());
            let ident = &rest[..end];
            if RESERVED_TYPE_NAMES.contains(&ident) {
                violations.push(format!(
                    "{}:{}: `public type {}` shadows stdlib name (allowed names listed in {})",
                    relative_to_root(file_path, &root),
                    line_idx + 1,
                    ident,
                    "crates/verum_compiler/tests/sqlite_native_naming_hygiene.rs",
                ));
            }
        }
    });

    if !violations.is_empty() {
        let msg = format!(
            "naming-hygiene violations under core/database/sqlite/native/:\n  {}\n\n\
             Rename the offending types using a project-prefixed name \
             (e.g. CacheflushResult, JournalSizeResult).  See git log for \
             commit 67ab55b1 for an example fix.",
            violations.join("\n  "),
        );
        panic!("{}", msg);
    }
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // cargo passes CARGO_MANIFEST_DIR to integration tests as the crate's
    // dir.  Walk up to find a directory containing the catalogue tree.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        if ancestor.join("core/database/sqlite/native").is_dir() {
            return ancestor.to_path_buf();
        }
    }
    panic!(
        "could not locate workspace root containing core/database/sqlite/native; \
         CARGO_MANIFEST_DIR={}",
        crate_dir.display()
    );
}

fn relative_to_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.display().to_string())
}

fn walk_vr_files(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_vr_files(&path, visit);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            visit(&path);
        }
    }
}
