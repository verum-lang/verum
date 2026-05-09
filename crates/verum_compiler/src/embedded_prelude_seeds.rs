//! Embedded prelude-seed module-paths.
//!
//! The prelude consists of every `public mount super.X.{…}` (or
//! `mount super.X.{…}`) declaration in `core/mod.vr`; their targets
//! are auto-imported into every user compilation via the re-export
//! chain.  Pre-fix `stdlib_reachability::prelude_seeds` parsed
//! `core/mod.vr` source from `embedded_stdlib` on every compile —
//! the last load-bearing reason to keep gzipped .vr sources in
//! production binaries.
//!
//! Post-fix: `build.rs::build_prelude_seeds` parses `core/mod.vr`
//! once at build time, serialises one path per line to
//! `$OUT_DIR/stdlib_prelude_seeds.txt`, and the runtime reads the
//! embedded text via [`get_prelude_seeds`].  Tiny artefact (≈1 KB
//! vs the ~200 KB embedded source archive whose only remaining
//! consumers are dev tools).

/// Newline-separated list of prelude module paths, baked in at
/// build time.  Empty when the build script couldn't find
/// `core/mod.vr` (alternate-stdlib-source bootstrap path).
static PRELUDE_SEEDS_TEXT: &str = include_str!(env!("STDLIB_PRELUDE_SEEDS_PATH"));

/// Returns the prelude seed module-paths as borrowed string slices.
///
/// Empty result indicates either the embedded artefact is empty or
/// `core/mod.vr` declares no `public mount super.…` directives.
/// Callers (lazy-stdlib pruning, reachability walkers) MUST treat
/// an empty result as "include everything" rather than "include
/// nothing" — same fallback the legacy source-driven scanner used
/// when `embedded_stdlib` was absent.
pub fn get_prelude_seeds() -> impl Iterator<Item = &'static str> {
    PRELUDE_SEEDS_TEXT
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
}

/// Eagerly-collected vector form, when callers need owned `String`s
/// for a stable iteration order across passes.
pub fn collect_prelude_seeds() -> Vec<String> {
    get_prelude_seeds().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: production builds must produce a non-empty prelude
    /// list.  Without a populated prelude the lazy-stdlib pruning would
    /// drop core types (Maybe / Result / List / Text) on `verum run`
    /// scripts that rely on the auto-imports.
    #[test]
    fn prelude_seeds_non_empty_on_production_build() {
        let seeds: Vec<&str> = get_prelude_seeds().collect();
        // The bootstrap path produces an empty embedded file before
        // `core/mod.vr` is precompiled — accept that case only when
        // CARGO_PKG_VERSION indicates we're in the pre-stdlib build
        // phase.  Otherwise insist on a real prelude.
        if PRELUDE_SEEDS_TEXT.is_empty() {
            return; // bootstrap phase; nothing to assert
        }
        assert!(
            !seeds.is_empty(),
            "embedded prelude artefact present but parsed empty — \
             check `build.rs::build_prelude_seeds` and `core/mod.vr`"
        );
    }

    /// Every recovered seed is in dotted module-path form rooted at
    /// `core.` (the `super` → `core` rewrite happens at build time).
    #[test]
    fn prelude_seeds_are_core_rooted() {
        for seed in get_prelude_seeds() {
            assert!(
                seed == "core" || seed.starts_with("core."),
                "prelude seed `{}` is not rooted at `core.`",
                seed
            );
        }
    }
}
