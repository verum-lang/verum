//! Lightweight module-path index over the embedded stdlib.
//!
//! Bridges `embedded_stdlib::StdlibArchive` (file-relative-path keyed)
//! to the compiler's module-path namespace (`core.shell.exec` etc.).
//! Provides O(1) lookups in both directions and a single sorted listing
//! for deterministic enumeration.
//!
//! # Why a separate index
//!
//! The archive stores raw `.vr` source bytes keyed by relative file path
//! (e.g. `"shell/exec.vr"`). Almost every consumer in the compiler thinks
//! in module-path terms (`"core.shell.exec"`). Doing the conversion
//! ad-hoc at every call-site spreads the convention across many files
//! and makes future changes (e.g. a different file→module mapping for
//! generated VBC modules) costly.
//!
//! The index is the single place that owns the convention:
//!
//!   `core/shell/exec.vr`     → `core.shell.exec`
//!   `core/shell/mod.vr`      → `core.shell`
//!   `core/database/sqlite/native/l7_api/database.vr`
//!                            → `core.database.sqlite.native.l7_api.database`
//!
//! # Performance contract
//!
//! - `module_to_file()` / `file_to_module()`: O(1) HashMap lookup
//! - `all_modules()`: returns a borrow of a pre-sorted Vec
//! - First call: ~2ms for archive decompress (shared with `embedded_stdlib`)
//! - Memory: ~250 KB extra (two small HashMaps + one Vec, sharing the
//!           archive's String storage by clone)
//!
//! # Threading
//!
//! The index lives in a `OnceLock`; the first reader builds it, all
//! subsequent readers see the populated value. No mutation after build.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::embedded_stdlib::{self, StdlibArchive};

/// Singleton module-path index over the embedded stdlib archive.
static MODULE_INDEX: OnceLock<Option<StdlibModuleIndex>> = OnceLock::new();

/// Two-way map between module paths and their backing stdlib file paths.
pub struct StdlibModuleIndex {
    /// `core.shell.exec` → `shell/exec.vr`
    by_module: HashMap<String, String>,
    /// `shell/exec.vr` → `core.shell.exec`
    by_file: HashMap<String, String>,
    /// All module paths, sorted lexicographically. Lets enumeration be
    /// deterministic without re-sorting the HashMap on each access.
    sorted_modules: Vec<String>,
}

impl StdlibModuleIndex {
    /// Build the index from an already-decompressed archive.
    fn build(archive: &StdlibArchive) -> Self {
        let mut by_module = HashMap::with_capacity(archive.file_count());
        let mut by_file = HashMap::with_capacity(archive.file_count());
        let mut sorted_modules = Vec::with_capacity(archive.file_count());

        for file_path in archive.file_paths() {
            let module_path = file_path_to_module_path(file_path);
            by_module.insert(module_path.clone(), file_path.to_string());
            by_file.insert(file_path.to_string(), module_path.clone());
            sorted_modules.push(module_path);
        }

        sorted_modules.sort();

        Self {
            by_module,
            by_file,
            sorted_modules,
        }
    }

    /// Look up the source file path for a module path.
    /// Returns `None` if the module is not in the embedded stdlib (e.g. a
    /// user-defined cog module or a forward-declared module without a
    /// source file).
    pub fn module_to_file(&self, module_path: &str) -> Option<&str> {
        self.by_module.get(module_path).map(String::as_str)
    }

    /// Reverse lookup: `shell/exec.vr` → `core.shell.exec`.
    pub fn file_to_module(&self, file_path: &str) -> Option<&str> {
        self.by_file.get(file_path).map(String::as_str)
    }

    /// All module paths in the embedded stdlib, sorted lexicographically.
    /// The slice borrow is valid for the lifetime of the index (process-
    /// long via the `OnceLock`).
    pub fn all_modules(&self) -> &[String] {
        &self.sorted_modules
    }

    /// Count of modules in the index. Diagnostics only.
    pub fn len(&self) -> usize {
        self.sorted_modules.len()
    }

    /// Whether the index contains any modules.
    pub fn is_empty(&self) -> bool {
        self.sorted_modules.is_empty()
    }

    /// Look up the source bytes for a module path. Combines the index
    /// lookup and the archive read in one call — the most common shape
    /// for downstream consumers (parsers, reachability walks).
    pub fn module_source<'a>(
        &self,
        archive: &'a StdlibArchive,
        module_path: &str,
    ) -> Option<&'a str> {
        let file = self.module_to_file(module_path)?;
        archive.get_file(file)
    }
}

/// Convert a stdlib file-relative path to its canonical module path.
///
/// Examples:
///
/// ```text
/// "base/maybe.vr"               → "core.base.maybe"
/// "shell/exec.vr"               → "core.shell.exec"
/// "shell/mod.vr"                → "core.shell"
/// "database/sqlite/native/l7_api/database.vr"
///                               → "core.database.sqlite.native.l7_api.database"
/// ```
///
/// This function is the single source of truth for the file→module
/// mapping. The same convention is implemented inline in
/// `pipeline.rs::load_stdlib_modules` (lines 3805-3829); both must stay
/// in sync. Future cleanup can replace the inline version with a call
/// here once the index is fully wired through.
pub fn file_path_to_module_path(relative_file: &str) -> String {
    // Strip any platform-specific separator just in case (the archive
    // normalises to forward slashes at build time, but defensive code is
    // cheap).
    let normalised = relative_file.replace('\\', "/");

    // "core" is the implicit root namespace for every embedded file.
    let mut parts: Vec<&str> = vec!["core"];
    for component in normalised.split('/') {
        if component.is_empty() {
            continue;
        }
        let trimmed = component.strip_suffix(".vr").unwrap_or(component);
        parts.push(trimmed);
    }

    let joined = parts.join(".");

    // `mod.vr` represents its parent directory: collapse the trailing
    // `.mod` segment.
    joined.strip_suffix(".mod").map(str::to_string).unwrap_or(joined)
}

/// Get the global module index. Builds on first call; later calls are
/// HashMap reads. Returns `None` if the embedded stdlib is unavailable
/// (e.g. minimal builds without `core/`).
pub fn get_module_index() -> Option<&'static StdlibModuleIndex> {
    MODULE_INDEX
        .get_or_init(|| {
            embedded_stdlib::get_embedded_stdlib().map(StdlibModuleIndex::build)
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_to_module_handles_mod_files() {
        assert_eq!(file_path_to_module_path("shell/mod.vr"), "core.shell");
        assert_eq!(file_path_to_module_path("base/maybe.vr"), "core.base.maybe");
        assert_eq!(file_path_to_module_path("mod.vr"), "core");
    }

    #[test]
    fn file_to_module_handles_deep_nesting() {
        assert_eq!(
            file_path_to_module_path("database/sqlite/native/l7_api/database.vr"),
            "core.database.sqlite.native.l7_api.database"
        );
    }

    #[test]
    fn file_to_module_normalises_backslashes() {
        // Defensive — archive uses forward slashes, but be safe.
        assert_eq!(file_path_to_module_path("shell\\exec.vr"), "core.shell.exec");
    }

    #[test]
    fn index_round_trip_when_archive_present() {
        // Skip if no embedded stdlib (minimal build).
        let Some(index) = get_module_index() else { return; };
        // Round-trip a known stdlib module.
        if let Some(file) = index.module_to_file("core.shell") {
            assert_eq!(index.file_to_module(file), Some("core.shell"));
        }
    }
}
