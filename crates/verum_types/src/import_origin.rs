//! Import provenance — VUVA #146 / MOD-MED-2.
//!
//! Glob imports (`mount X.*`) without provenance discipline produced
//! source-text-dependent name resolution: when two globs both
//! re-export `Nil`, the second-encountered glob always overwrote the
//! first, regardless of which side was the user's project and which
//! was the stdlib. That broke a Verum philosophical invariant — the
//! user's project must always have precedence over stdlib — and made
//! glob-vs-glob shadowing observably non-deterministic across reorders.
//!
//! Fix: tag every glob-imported name with its `ImportOrigin` so that
//! conflict resolution becomes a total ordering rather than
//! source-position-dependent. Priority (lowest → highest):
//!
//!   Stdlib  <  External  <  Project
//!
//! Same-origin ties default to first-wins (deterministic given the
//! sorted import order in `import_all_from_module_impl`). When a
//! project glob shadows a stdlib name, emit `W_STDLIB_SHADOW` so the
//! user can audit the eviction.
//!
//! Explicit imports (`mount X.{Bar}`) always win over any glob —
//! that rule predates this module and is enforced separately via the
//! `explicit_imports` set.

use verum_common::Text;

/// Provenance of a glob-imported symbol. Determines who wins on a
/// glob-vs-glob name collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportOrigin {
    /// `core.*` — Verum standard library. Lowest priority: a project
    /// or external glob ALWAYS overrides a stdlib glob for the same
    /// short name.
    Stdlib,
    /// External cog dependency (anything that isn't `core.*` and
    /// isn't the user's own cog). Middle priority.
    External,
    /// The user's own project — anything under the current cog name
    /// or starting with `cog.`. Highest priority. Project always wins.
    Project,
}

impl ImportOrigin {
    /// Classify a fully-qualified module path against the current cog
    /// name. Stdlib detection is path-prefix-based and intentionally
    /// permissive: stdlib paths are normalised to `core.…` everywhere
    /// in the loader.
    pub fn classify(module_path: &str, current_cog_name: &str) -> Self {
        // The current cog is always `Project` regardless of how it's
        // referenced. We accept three shapes:
        //   - `cog.…`            (canonical absolute prefix)
        //   - `<cog_name>.…`     (the cog's user-visible name)
        //   - `<cog_name>`       (the cog itself, no submodule)
        if module_path == "cog" || module_path.starts_with("cog.") {
            return ImportOrigin::Project;
        }
        if !current_cog_name.is_empty() {
            if module_path == current_cog_name
                || module_path.starts_with(&format!("{}.", current_cog_name))
            {
                return ImportOrigin::Project;
            }
        }

        // Stdlib detection: `core.…` or bare `core`. Also catch the
        // legacy `std.…` alias in case some loader still surfaces it.
        if module_path == "core"
            || module_path.starts_with("core.")
            || module_path == "std"
            || module_path.starts_with("std.")
        {
            return ImportOrigin::Stdlib;
        }

        ImportOrigin::External
    }

    /// Numeric priority: higher beats lower. Used to decide who wins
    /// a glob-vs-glob conflict.
    pub fn priority(&self) -> u8 {
        match self {
            ImportOrigin::Stdlib => 0,
            ImportOrigin::External => 1,
            ImportOrigin::Project => 2,
        }
    }

    /// Human-readable label for diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            ImportOrigin::Stdlib => "stdlib",
            ImportOrigin::External => "external cog",
            ImportOrigin::Project => "project",
        }
    }
}

/// Provenance record stored alongside each glob-imported name.
///
/// `module_path` keeps the original import source so the eviction
/// warning can name both the loser and the winner.
#[derive(Debug, Clone)]
pub struct ImportProvenance {
    pub origin: ImportOrigin,
    pub module_path: Text,
}

impl ImportProvenance {
    pub fn new(origin: ImportOrigin, module_path: Text) -> Self {
        Self { origin, module_path }
    }

    /// Decision logic: should the incoming glob (with `incoming`
    /// provenance) be allowed to overwrite an existing entry whose
    /// provenance is `existing`?
    ///
    /// Rules:
    ///   - Strictly higher priority → overwrite.
    ///   - Strictly lower priority → preserve (return false).
    ///   - Same priority (tie) → first wins (preserve, return false)
    ///     for determinism across `mount` reorders.
    pub fn allows_overwrite(existing: &Self, incoming: &Self) -> bool {
        incoming.origin.priority() > existing.origin.priority()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdlib_classification() {
        assert_eq!(ImportOrigin::classify("core", "myapp"), ImportOrigin::Stdlib);
        assert_eq!(
            ImportOrigin::classify("core.collections.list", "myapp"),
            ImportOrigin::Stdlib
        );
        assert_eq!(
            ImportOrigin::classify("std.io.fs", "myapp"),
            ImportOrigin::Stdlib
        );
    }

    #[test]
    fn project_classification() {
        assert_eq!(
            ImportOrigin::classify("cog.api.v1", "myapp"),
            ImportOrigin::Project
        );
        assert_eq!(
            ImportOrigin::classify("myapp.handlers", "myapp"),
            ImportOrigin::Project
        );
        assert_eq!(
            ImportOrigin::classify("myapp", "myapp"),
            ImportOrigin::Project
        );
    }

    #[test]
    fn external_classification() {
        assert_eq!(
            ImportOrigin::classify("serde.json", "myapp"),
            ImportOrigin::External
        );
        assert_eq!(
            ImportOrigin::classify("rand.distributions", "myapp"),
            ImportOrigin::External
        );
    }

    #[test]
    fn priority_ordering() {
        assert!(ImportOrigin::Project.priority() > ImportOrigin::External.priority());
        assert!(ImportOrigin::External.priority() > ImportOrigin::Stdlib.priority());
    }

    #[test]
    fn project_overwrites_stdlib() {
        let stdlib = ImportProvenance::new(
            ImportOrigin::Stdlib,
            Text::from("core.base"),
        );
        let project = ImportProvenance::new(
            ImportOrigin::Project,
            Text::from("cog.types"),
        );
        assert!(ImportProvenance::allows_overwrite(&stdlib, &project));
    }

    #[test]
    fn stdlib_does_not_overwrite_project() {
        let project = ImportProvenance::new(
            ImportOrigin::Project,
            Text::from("cog.types"),
        );
        let stdlib = ImportProvenance::new(
            ImportOrigin::Stdlib,
            Text::from("core.base"),
        );
        assert!(!ImportProvenance::allows_overwrite(&project, &stdlib));
    }

    #[test]
    fn same_origin_first_wins() {
        let first = ImportProvenance::new(
            ImportOrigin::Project,
            Text::from("cog.a"),
        );
        let second = ImportProvenance::new(
            ImportOrigin::Project,
            Text::from("cog.b"),
        );
        assert!(!ImportProvenance::allows_overwrite(&first, &second));
    }
}
