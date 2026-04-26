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

/// A simplified `@cfg(...)` constraint, just enough to detect when two
/// declarations of the same simple name are mutually exclusive at
/// codegen time and therefore cannot collide.
///
/// Modelled as a conjunction of `key = value` constraints (e.g.
/// `target_os = "linux"`).  An empty constraint set means "always
/// active" (no `@cfg` annotation, or one we cannot model).  When the
/// `conservative` flag is set the cfg uses constructs the parser
/// cannot decompose precisely (e.g. `any(...)`, bare predicates like
/// `unix`); to stay sound the test then treats the declaration as
/// always active.
#[derive(Clone, Debug, Default)]
struct CfgConstraint {
    constraints: Vec<(String, String)>,
    conservative: bool,
}

/// Keys whose different values prove mutual exclusion at build time.
/// These are the cfg axes that pick exactly one value per build:
///
///   target_os             — linux / macos / windows / …
///   target_arch           — x86_64 / aarch64 / …
///   target_family         — unix / windows / wasm / …
///   target_endian         — little / big
///   target_pointer_width  — 16 / 32 / 64
///   runtime               — full / single_thread / no_async / no_heap /
///                           embedded / none — at most one is selected
///                           per `runtime` build profile
///
/// Keys NOT in this list (notably `feature` and `debug_assertions`)
/// are additive or independent: two cfgs with `feature = "A"` and
/// `feature = "B"` can both be active when the build enables both
/// features, so different values there do NOT imply mutual exclusion.
const EXCLUSIVE_CFG_KEYS: &[&str] = &[
    "target_os",
    "target_arch",
    "target_family",
    "target_endian",
    "target_pointer_width",
    "runtime",
];

impl CfgConstraint {
    /// True iff `self` and `other` cannot both be active in the same
    /// build — they share at least one *exclusive* key with different
    /// values, and neither is conservative.
    fn mutually_exclusive(&self, other: &Self) -> bool {
        if self.conservative || other.conservative {
            return false;
        }
        for (k, v) in &self.constraints {
            if !EXCLUSIVE_CFG_KEYS.contains(&k.as_str()) {
                continue;
            }
            for (k2, v2) in &other.constraints {
                if k == k2 && v != v2 {
                    return true;
                }
            }
        }
        false
    }
}

/// Parse an `@cfg(...)` line into a `CfgConstraint`.  Recognises:
///   - `@cfg(K = "V")`            — single key=value
///   - `@cfg(all(K1 = "V1", K2 = "V2", ...))` — conjunction
///   - anything else → conservative (treat as always-active)
fn parse_cfg_attr(attr_line: &str) -> CfgConstraint {
    let trimmed = attr_line.trim();
    let inner = match trimmed.strip_prefix("@cfg(").and_then(|s| s.strip_suffix(")")) {
        Some(s) => s.trim(),
        None => return CfgConstraint { constraints: vec![], conservative: true },
    };
    // Strip optional `all(...)` wrapper.
    let body = inner.strip_prefix("all(").and_then(|s| s.strip_suffix(")")).unwrap_or(inner);
    let mut out = CfgConstraint::default();
    for part in body.split(',') {
        let part = part.trim();
        if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_string();
            let val_with_quotes = part[eq + 1..].trim();
            let val = val_with_quotes.trim_matches('"').to_string();
            // Refuse to model nested any(...) or unmatched quotes.
            if key.contains('(') || val.contains('(') {
                return CfgConstraint { constraints: vec![], conservative: true };
            }
            out.constraints.push((key, val));
        } else {
            // Bare predicate like `unix` — conservative.
            return CfgConstraint { constraints: vec![], conservative: true };
        }
    }
    out
}

/// Each declaration site, augmented with its `@cfg` constraint so the
/// uniqueness check can treat mutually-exclusive sites as non-colliding.
type Site = (String, PathBuf, usize, CfgConstraint);

#[test]
fn stdlib_public_type_names_are_unique() {
    let root = workspace_root().join(ROOT);
    assert!(
        root.is_dir(),
        "expected stdlib root at {} but it does not exist",
        root.display()
    );

    let mut definitions: BTreeMap<String, Vec<Site>> = BTreeMap::new();
    walk_dir(&root, &root, &mut definitions);

    let mut violations: Vec<String> = Vec::new();
    for (name, sites) in &definitions {
        if sites.len() < 2 {
            continue;
        }
        // Two sites collide only if their cfgs overlap (i.e. are not
        // mutually exclusive).  Build the colliding subset.
        let colliding: Vec<&Site> = sites
            .iter()
            .enumerate()
            .filter(|(i, s)| {
                sites.iter().enumerate().any(|(j, t)| {
                    *i != j && !s.3.mutually_exclusive(&t.3)
                })
            })
            .map(|(_, s)| s)
            .collect();
        if colliding.len() < 2 {
            continue;
        }
        let mut entry = format!(
            "duplicate: type `{}` is declared in {} co-active stdlib modules:\n",
            name,
            colliding.len()
        );
        for (modpath, file, line, _cfg) in &colliding {
            entry.push_str(&format!("    - {} ({}:{})\n", modpath, file.display(), line));
        }
        violations.push(entry);
    }

    if !violations.is_empty() {
        panic!(
            "{} duplicated public type name(s) in stdlib.  Each public \
             stdlib type must have a unique simple name across all of \
             `core/` for any single build configuration, because the VBC \
             variant-constructor table is keyed by simple name.  Two \
             `public type Foo is …` declarations that are co-active in \
             the same build collide and the second's variants are \
             silently skipped, surfacing later as runtime \
             `method 'X.Y' not found on value` panics.\n\n\
             The test recognises mutually-exclusive `@cfg(...)` \
             attributes (e.g. target_os, target_arch, runtime tier) and \
             does NOT flag declarations that cannot both be active in \
             the same build.  When you genuinely need parallel \
             implementations across cfg variants, place each behind its \
             own `@cfg(...)` and reuse the same simple name.\n\n\
             For unconditional collisions: pick a domain-prefixed \
             disambiguated name following the existing stdlib \
             convention — see this file's module-level docs for the \
             catalogue / layer-pair / platform / arch conventions.\n\n\
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
    sink: &mut BTreeMap<String, Vec<Site>>,
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
/// Records each match under its simple name + `@cfg` constraint in
/// `sink`.
///
/// Heuristic-only — the test is intentionally lenient about formatting
/// (whitespace, generics, body kind).  Excludes `protocol` types since
/// they cannot collide via the variant-constructor mechanism (no
/// constructors).
///
/// `@cfg(...)` attributes immediately preceding a type declaration
/// (with `@repr(...)` / `@align(...)` / etc. allowed in between) are
/// captured and attached to the site, so the uniqueness check can
/// treat mutually-exclusive cfg variants as non-colliding.
fn scan_file(
    repo_core_root: &Path,
    file: &Path,
    sink: &mut BTreeMap<String, Vec<Site>>,
) {
    let contents = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let module_path = file_to_module_path(repo_core_root, file);
    let mut pending_cfg = CfgConstraint::default();
    for (lineno, line) in contents.lines().enumerate() {
        let trimmed = line.trim_start();
        // Attribute lines preceding a declaration: capture @cfg, ignore
        // other attributes, reset the captured cfg on a blank line or
        // any non-attribute statement that isn't a type/fn/etc.
        if trimmed.starts_with("@cfg(") {
            pending_cfg = parse_cfg_attr(trimmed);
            continue;
        }
        if trimmed.starts_with('@') {
            // Other attribute (e.g. @repr, @align, @derive) — keep the
            // pending @cfg in scope for the type that follows.
            continue;
        }
        if !trimmed.starts_with("public type ") {
            // Non-attribute, non-type-declaration line resets the
            // pending cfg so it doesn't leak past unrelated code.
            if !trimmed.is_empty() && !trimmed.starts_with("//") {
                pending_cfg = CfgConstraint::default();
            }
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
            std::mem::take(&mut pending_cfg),
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

// ---------------------------------------------------------------------------
// Unit tests — pin the parse / overlap contracts so future stdlib code that
// introduces a new @cfg form is caught here rather than silently disabling
// the @cfg-aware skip.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod cfg_parsing_tests {
    use super::*;

    fn cfg(s: &str) -> CfgConstraint {
        parse_cfg_attr(s)
    }

    #[test]
    fn empty_constraint_is_always_active() {
        let c = CfgConstraint::default();
        assert!(c.constraints.is_empty());
        assert!(!c.conservative);
        // Two empty constraints overlap (always-active vs always-active).
        assert!(!c.mutually_exclusive(&CfgConstraint::default()));
    }

    #[test]
    fn parses_simple_target_os() {
        let c = cfg(r#"@cfg(target_os = "linux")"#);
        assert!(!c.conservative, "simple target_os should not be conservative");
        assert_eq!(c.constraints, vec![("target_os".to_string(), "linux".to_string())]);
    }

    #[test]
    fn parses_simple_target_arch() {
        let c = cfg(r#"@cfg(target_arch = "x86_64")"#);
        assert!(!c.conservative);
        assert_eq!(c.constraints, vec![("target_arch".to_string(), "x86_64".to_string())]);
    }

    #[test]
    fn parses_all_conjunction() {
        let c = cfg(r#"@cfg(all(runtime = "full", target_os = "linux"))"#);
        assert!(!c.conservative, "all(...) should be decomposed precisely");
        assert_eq!(
            c.constraints,
            vec![
                ("runtime".to_string(), "full".to_string()),
                ("target_os".to_string(), "linux".to_string()),
            ]
        );
    }

    #[test]
    fn any_disjunction_is_conservative() {
        let c = cfg(r#"@cfg(any(target_os = "linux", target_os = "windows"))"#);
        // any(...) doesn't decompose to a single conjunction; the parser
        // refuses to model it precisely and falls back to conservative
        // (always-active) treatment.
        assert!(c.conservative, "any(...) must be conservative");
    }

    #[test]
    fn bare_predicate_is_conservative() {
        // `unix` is a target_family predicate — not modelled as a
        // key=value pair, so we treat it conservatively.
        let c = cfg(r#"@cfg(unix)"#);
        assert!(c.conservative);
    }

    #[test]
    fn malformed_input_is_conservative() {
        // Missing leading @cfg(...) form — fall back to conservative.
        let c = cfg(r#"target_os = "linux""#);
        assert!(c.conservative);
    }

    // ---- mutually_exclusive contract ---------------------------------

    #[test]
    fn target_os_linux_excludes_target_os_macos() {
        let a = cfg(r#"@cfg(target_os = "linux")"#);
        let b = cfg(r#"@cfg(target_os = "macos")"#);
        assert!(a.mutually_exclusive(&b));
        assert!(b.mutually_exclusive(&a));
    }

    #[test]
    fn target_arch_x86_64_excludes_target_arch_aarch64() {
        let a = cfg(r#"@cfg(target_arch = "x86_64")"#);
        let b = cfg(r#"@cfg(target_arch = "aarch64")"#);
        assert!(a.mutually_exclusive(&b));
    }

    #[test]
    fn runtime_full_excludes_runtime_embedded() {
        let a = cfg(r#"@cfg(runtime = "full")"#);
        let b = cfg(r#"@cfg(runtime = "embedded")"#);
        assert!(a.mutually_exclusive(&b));
    }

    #[test]
    fn same_target_os_is_not_mutually_exclusive() {
        let a = cfg(r#"@cfg(target_os = "linux")"#);
        let b = cfg(r#"@cfg(target_os = "linux")"#);
        assert!(!a.mutually_exclusive(&b));
    }

    #[test]
    fn empty_overlaps_anything() {
        let a = CfgConstraint::default();
        let b = cfg(r#"@cfg(target_os = "linux")"#);
        assert!(!a.mutually_exclusive(&b));
        assert!(!b.mutually_exclusive(&a));
    }

    #[test]
    fn conservative_overlaps_anything() {
        let a = cfg(r#"@cfg(unix)"#);
        let b = cfg(r#"@cfg(target_os = "linux")"#);
        assert!(!a.mutually_exclusive(&b));
        assert!(!b.mutually_exclusive(&a));
    }

    #[test]
    fn feature_gates_are_additive_not_exclusive() {
        // Different `feature` values do NOT imply mutual exclusion —
        // both features can be enabled simultaneously in the same
        // build.  Treating them as mutex would be unsound: if both
        // declarations land in the same build, they DO collide and
        // the test should flag it.
        let a = cfg(r#"@cfg(feature = "crypto-ring")"#);
        let b = cfg(r#"@cfg(feature = "crypto-openssl")"#);
        assert!(!a.mutually_exclusive(&b));
    }

    #[test]
    fn debug_assertions_is_not_an_exclusive_axis() {
        // `debug_assertions` is a build-mode predicate, not part of
        // the exclusive-keys list; treating differing values as
        // mutually exclusive would over-approximate (and the parser
        // can't distinguish the bare-predicate form anyway).
        let a = cfg(r#"@cfg(debug_assertions = "true")"#);
        let b = cfg(r#"@cfg(debug_assertions = "false")"#);
        assert!(!a.mutually_exclusive(&b));
    }

    #[test]
    fn cross_axis_constraints_dont_exclude() {
        // target_os = "linux" and target_arch = "x86_64" overlap
        // (linux+x86_64 is a real build target).
        let a = cfg(r#"@cfg(target_os = "linux")"#);
        let b = cfg(r#"@cfg(target_arch = "x86_64")"#);
        assert!(!a.mutually_exclusive(&b));
    }

    #[test]
    fn conjunction_excludes_when_any_exclusive_axis_disagrees() {
        // (full,linux) vs (full,macos) — same runtime, different
        // target_os → mutually exclusive.
        let a = cfg(r#"@cfg(all(runtime = "full", target_os = "linux"))"#);
        let b = cfg(r#"@cfg(all(runtime = "full", target_os = "macos"))"#);
        assert!(a.mutually_exclusive(&b));
    }
}
