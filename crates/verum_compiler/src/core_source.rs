// ARCHITECTURE NOTE: Dependency graph and topological sort logic is shared with core_compiler.rs.
// When core/ migrates to a proper cog, both CoreSourceResolver and StdlibModuleResolver will
// delegate to a unified CoreDependencyResolver. The known_deps map should be derived from
// parsing `mount` statements in .vr files.
//! Stdlib Source Abstraction
//!
//! Provides a unified interface for accessing stdlib files from the local filesystem.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                   CoreSourceTrait                           │
//! │  read_file(&str) -> Option<Cow<str>>                       │
//! │  list_files() -> Vec<&str>                                 │
//! │  exists(&str) -> bool                                      │
//! └────────────────────────┬──────────────────────────────────┘
//!                          │
//!            ┌─────────────▼──────────────┐
//!            │     LocalCoreSource        │
//!            │     (filesystem)           │
//!            └────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! let source = CoreSource::auto_detect();
//!
//! if let Some(content) = source.read_file("core/mod.vr") {
//!     // Parse content...
//! }
//! ```

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

/// Trait for abstracting stdlib file access.
pub trait CoreSourceTrait: Send + Sync {
    /// Read file content by relative path (e.g., "core/mod.vr").
    fn read_file(&self, path: &str) -> Option<Cow<'static, str>>;

    /// List all available stdlib file paths.
    fn list_files(&self) -> Vec<&str>;

    /// Check if a file exists.
    fn exists(&self, path: &str) -> bool;

    /// Get the source name for diagnostics.
    fn source_name(&self) -> &'static str;

    /// Check if this is a local (development) source.
    fn is_local(&self) -> bool;
}

/// Local filesystem stdlib source.
///
/// Reads files from a local directory, allowing live editing.
#[derive(Debug, Clone)]
pub struct LocalCoreSource {
    /// Root directory of stdlib
    root: PathBuf,
    /// Cached list of files (computed on creation)
    files: Vec<String>,
}

impl LocalCoreSource {
    /// Create new local source from directory path.
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref().canonicalize()?;

        // Verify it's a valid stdlib directory
        if !root.join("mod.vr").exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Not a valid stdlib directory: {} (missing mod.vr)", root.display()),
            ));
        }

        // Scan for all .vr files
        let mut files = Vec::new();
        Self::scan_dir(&root, &root, &mut files)?;
        files.sort();

        Ok(Self { root, files })
    }

    fn scan_dir(base: &Path, dir: &Path, files: &mut Vec<String>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Skip hidden directories
                if path
                    .file_name()
                    .map_or(false, |n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                Self::scan_dir(base, &path, files)?;
            } else if path.extension().map_or(false, |ext| ext == "vr") {
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push(relative);
            }
        }
        Ok(())
    }

    /// Get the root directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

impl CoreSourceTrait for LocalCoreSource {
    fn read_file(&self, path: &str) -> Option<Cow<'static, str>> {
        let full_path = self.root.join(path);
        fs::read_to_string(&full_path).ok().map(Cow::Owned)
    }

    fn list_files(&self) -> Vec<&str> {
        self.files.iter().map(|s| s.as_str()).collect()
    }

    fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }

    fn source_name(&self) -> &'static str {
        "local"
    }

    fn is_local(&self) -> bool {
        true
    }
}

/// Stdlib source backed by the local filesystem.
///
/// The stdlib is always resolved from the local `core/` directory.
/// Previously this had an `Embedded` variant using a phf map compiled into
/// the binary; that approach has been removed in favor of filesystem-only access.
#[derive(Debug)]
pub struct CoreSource {
    inner: LocalCoreSource,
}

impl CoreSource {
    /// Auto-detect the stdlib source from the filesystem.
    ///
    /// Resolution chain (first match wins):
    /// 1. `VERUM_CORE_PATH` environment variable (explicit override)
    /// 2. `./core/` directory (in-repo development — the primary case)
    /// 3. Next to the verum binary: `<exe_dir>/../core/` (installed distribution)
    /// 4. `$VERUM_HOME/core/` or `~/.verum/core/` (user-local installation)
    /// 5. `./stdlib/` directory (legacy fallback)
    pub fn auto_detect() -> Self {
        // 1. Explicit environment variable (highest priority)
        if let Ok(path) = std::env::var("VERUM_CORE_PATH") {
            if let Ok(local) = LocalCoreSource::new(&path) {
                tracing::info!(
                    path = %path,
                    files = local.file_count(),
                    "Using stdlib from VERUM_CORE_PATH"
                );
                return Self { inner: local };
            }
        }

        // 2. Local core/ directory (in-repo development)
        if let Ok(local) = LocalCoreSource::new("core") {
            tracing::info!(
                path = %local.root().display(),
                files = local.file_count(),
                "Using local core directory"
            );
            return Self { inner: local };
        }

        // 3. Next to the verum binary (installed distribution)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                // <exe_dir>/../core/ (standard install layout)
                let sibling_core = exe_dir.join("../core");
                if let Ok(local) = LocalCoreSource::new(&sibling_core) {
                    tracing::info!(
                        path = %local.root().display(),
                        files = local.file_count(),
                        "Using stdlib next to binary"
                    );
                    return Self { inner: local };
                }
                // <exe_dir>/../lib/verum/core/ (FHS layout)
                let lib_core = exe_dir.join("../lib/verum/core");
                if let Ok(local) = LocalCoreSource::new(&lib_core) {
                    tracing::info!(
                        path = %local.root().display(),
                        files = local.file_count(),
                        "Using stdlib from lib/verum/core"
                    );
                    return Self { inner: local };
                }
            }
        }

        // 4. User-local installation: $VERUM_HOME/core/ or ~/.verum/core/
        let verum_home = std::env::var("VERUM_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".verum")))
            .ok();
        if let Some(home) = verum_home {
            let home_core = home.join("core");
            if let Ok(local) = LocalCoreSource::new(&home_core) {
                tracing::info!(
                    path = %local.root().display(),
                    files = local.file_count(),
                    "Using stdlib from ~/.verum/core"
                );
                return Self { inner: local };
            }
        }

        // 5. Legacy stdlib/ directory
        if let Ok(local) = LocalCoreSource::new("stdlib") {
            tracing::info!(
                path = %local.root().display(),
                files = local.file_count(),
                "Using local stdlib directory (legacy)"
            );
            return Self { inner: local };
        }

        tracing::warn!(
            "No stdlib source found. Searched: VERUM_CORE_PATH, ./core/, <exe>/../core/, \
             ~/.verum/core/, ./stdlib/. Set VERUM_CORE_PATH or install the standard library."
        );
        Self {
            inner: LocalCoreSource {
                root: PathBuf::from("core"),
                files: Vec::new(),
            },
        }
    }

    /// Create local source explicitly.
    pub fn local(path: impl AsRef<Path>) -> std::io::Result<Self> {
        LocalCoreSource::new(path).map(|inner| Self { inner })
    }

    /// Always true — stdlib is always local.
    pub fn is_local(&self) -> bool {
        true
    }

    /// Get source name for diagnostics.
    pub fn source_name(&self) -> &'static str {
        "local"
    }

    /// Load all core .vr files as SourceFile objects for pipeline consumption.
    pub fn load_all_source_files(&self) -> Vec<crate::api::SourceFile> {
        let mut files = Vec::new();
        for path in self.inner.list_files() {
            if let Some(content) = self.inner.read_file(path) {
                let full_path = self.inner.root().join(path);
                files.push(crate::api::SourceFile::new(
                    full_path.display().to_string(),
                    content.into_owned(),
                ));
            }
        }
        files
    }
}

impl CoreSourceTrait for CoreSource {
    #[inline]
    fn read_file(&self, path: &str) -> Option<Cow<'static, str>> {
        self.inner.read_file(path)
    }

    #[inline]
    fn list_files(&self) -> Vec<&str> {
        self.inner.list_files()
    }

    #[inline]
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }

    #[inline]
    fn source_name(&self) -> &'static str {
        "local"
    }

    #[inline]
    fn is_local(&self) -> bool {
        true
    }
}

// ============================================================================
// MODULE RESOLUTION
// ============================================================================

use std::collections::{HashMap, HashSet};
use verum_ast::cfg::TargetConfig;
use crate::module_utils;

/// A stdlib module with its source files.
#[derive(Debug, Clone)]
pub struct StdlibModuleInfo {
    /// Module name (e.g., "core", "sys.linux")
    pub name: String,
    /// Source file paths relative to stdlib root
    pub files: Vec<String>,
    /// Dependencies on other modules
    pub dependencies: Vec<String>,
}

/// Resolves stdlib modules from a CoreSource.
///
/// Analyzes the flat file list to discover modules and their dependencies.
pub struct CoreSourceResolver<'a> {
    source: &'a dyn CoreSourceTrait,
    modules: HashMap<String, StdlibModuleInfo>,
    compilation_order: Vec<String>,
}

impl std::fmt::Debug for CoreSourceResolver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreSourceResolver")
            .field("source", &self.source.source_name())
            .field("modules", &self.modules)
            .field("compilation_order", &self.compilation_order)
            .finish()
    }
}

impl<'a> CoreSourceResolver<'a> {
    /// Create a new resolver for the given source.
    pub fn new(source: &'a dyn CoreSourceTrait) -> Self {
        Self {
            source,
            modules: HashMap::new(),
            compilation_order: Vec::new(),
        }
    }

    /// Discover and resolve all modules.
    pub fn discover(&mut self) -> Result<(), String> {
        self.discover_modules()?;
        self.resolve_dependencies();
        self.compute_compilation_order()?;
        Ok(())
    }

    /// Discover modules from the file list.
    fn discover_modules(&mut self) -> Result<(), String> {
        let files = self.source.list_files();
        let target = TargetConfig::host();

        // Group files by module (directory)
        let mut module_files: HashMap<String, Vec<String>> = HashMap::new();

        for file in files {
            // Extract module from path: "core/mod.vr" -> "core", "sys/linux/mod.vr" -> "sys.linux"
            let parts: Vec<&str> = file.split('/').collect();

            let module_name = if parts.len() == 1 {
                // Root file like "mod.vr" -> "std" module
                "std".to_string()
            } else {
                // "core/memory.vr" -> "core"
                // "sys/linux/mod.vr" -> "sys.linux"
                parts[..parts.len() - 1].join(".")
            };

            // Skip platform-specific modules that don't match target
            if !module_utils::should_compile_module_for_target(&module_name, &target) {
                continue;
            }

            // Check module-level @cfg in mod.vr
            let mod_vr_path = if parts.len() == 1 {
                "mod.vr".to_string()
            } else {
                format!("{}/mod.vr", parts[..parts.len() - 1].join("/"))
            };

            if self.source.exists(&mod_vr_path) {
                if let Some(content) = self.source.read_file(&mod_vr_path) {
                    if !module_utils::check_module_cfg_from_content(&content, &target) {
                        continue;
                    }
                }
            }

            module_files
                .entry(module_name)
                .or_default()
                .push(file.to_string());
        }

        // Convert to StdlibModuleInfo
        for (name, mut files) in module_files {
            // Sort files: mod.vr first, then alphabetically
            files.sort_by(|a, b| {
                let a_is_mod = a.ends_with("mod.vr");
                let b_is_mod = b.ends_with("mod.vr");
                match (a_is_mod, b_is_mod) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.cmp(b),
                }
            });

            self.modules.insert(
                name.clone(),
                StdlibModuleInfo {
                    name,
                    files,
                    dependencies: Vec::new(),
                },
            );
        }

        Ok(())
    }

    /// Resolve module dependencies using known dependency graph.
    fn resolve_dependencies(&mut self) {
        let target = TargetConfig::host();

        // Known dependency structure (same as StdlibModuleResolver)
        let known_deps: HashMap<&str, Vec<&str>> = [
            ("std", vec![]),
            ("sys", vec![]),
            ("sys.linux", vec!["sys"]),
            ("sys.darwin", vec!["sys"]),
            ("sys.windows", vec!["sys"]),
            ("mem", vec!["sys", "sys.linux", "sys.darwin", "sys.windows"]),
            ("core", vec!["sys", "mem"]),
            ("sync", vec!["core"]),
            ("text", vec!["core", "sys.linux", "sys.darwin", "sys.windows"]),
            ("collections", vec!["core", "text"]),
            ("io", vec!["core", "text", "collections", "sys.linux", "sys.darwin", "sys.windows"]),
            ("time", vec!["core", "sys.linux", "sys.darwin", "sys.windows"]),
            ("runtime", vec!["core", "mem", "sync", "time", "sys", "async"]),
            ("async", vec!["core", "collections", "io", "sync", "time", "sys"]),
            ("term", vec!["core", "text", "collections", "io", "sync", "time", "sys", "sys.linux", "sys.darwin", "sys.windows"]),
            ("net", vec!["core", "io", "async", "sys"]),
            ("cognitive", vec!["core", "collections"]),
            ("meta", vec!["core"]),
        ]
        .into_iter()
        .collect();

        for (name, deps) in known_deps {
            if self.modules.contains_key(name) {
                // Compute filtered dependencies first to avoid borrow conflict
                let filtered_deps: Vec<String> = deps
                    .into_iter()
                    .filter(|dep| module_utils::should_compile_module_for_target(dep, &target))
                    .filter(|dep| self.modules.contains_key(*dep))
                    .map(|s| s.to_string())
                    .collect();

                // Now assign
                if let Some(module) = self.modules.get_mut(name) {
                    module.dependencies = filtered_deps;
                }
            }
        }
    }

    /// Compute topological sort for compilation order.
    fn compute_compilation_order(&mut self) -> Result<(), String> {
        let mut visited = HashSet::new();
        let mut temp_mark = HashSet::new();
        let mut order = Vec::new();

        fn visit(
            name: &str,
            modules: &HashMap<String, StdlibModuleInfo>,
            visited: &mut HashSet<String>,
            temp_mark: &mut HashSet<String>,
            order: &mut Vec<String>,
        ) -> Result<(), String> {
            if visited.contains(name) {
                return Ok(());
            }
            if temp_mark.contains(name) {
                return Err(format!("Circular dependency detected: {}", name));
            }

            temp_mark.insert(name.to_string());

            if let Some(module) = modules.get(name) {
                for dep in &module.dependencies {
                    visit(dep, modules, visited, temp_mark, order)?;
                }
            }

            temp_mark.remove(name);
            visited.insert(name.to_string());
            order.push(name.to_string());

            Ok(())
        }

        let module_names: Vec<String> = self.modules.keys().cloned().collect();
        for name in &module_names {
            visit(name, &self.modules, &mut visited, &mut temp_mark, &mut order)?;
        }

        self.compilation_order = order;
        Ok(())
    }

    /// Get modules in compilation order.
    pub fn modules_in_order(&self) -> Vec<&StdlibModuleInfo> {
        self.compilation_order
            .iter()
            .filter_map(|name| self.modules.get(name))
            .collect()
    }

    /// Get a module by name.
    pub fn get_module(&self, name: &str) -> Option<&StdlibModuleInfo> {
        self.modules.get(name)
    }

    /// Get the number of modules.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Read a file from the source.
    pub fn read_file(&self, path: &str) -> Option<Cow<'static, str>> {
        self.source.read_file(path)
    }
}

// ============================================================================
// GLOBAL SOURCE
// ============================================================================

/// Global stdlib source for the compilation session.
///
/// Initialized once per compilation and shared across all modules.
static GLOBAL_CORE_SOURCE: std::sync::OnceLock<CoreSource> = std::sync::OnceLock::new();

/// Initialize global stdlib source.
///
/// Should be called once at the start of compilation.
pub fn init_global_core_source(source: CoreSource) {
    let _ = GLOBAL_CORE_SOURCE.set(source);
}

/// Get the global stdlib source.
///
/// Panics if not initialized.
pub fn global_core_source() -> &'static CoreSource {
    GLOBAL_CORE_SOURCE.get().expect("Stdlib source not initialized")
}

/// Get the global stdlib source, auto-detecting if not initialized.
pub fn global_core_source_or_init() -> &'static CoreSource {
    GLOBAL_CORE_SOURCE.get_or_init(CoreSource::auto_detect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_core_source() {
        // Try to create a local source from the project's core/ directory
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let core_path = Path::new(manifest_dir).join("../../core");
        if core_path.exists() {
            let source = LocalCoreSource::new(&core_path).expect("Should open core directory");
            assert!(source.file_count() > 0, "Core directory should have files");
            assert!(source.exists("mod.vr"), "Core should have mod.vr");
        }
    }

    #[test]
    fn test_local_nonexistent() {
        let result = LocalCoreSource::new("/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_core_source_auto_detect() {
        // auto_detect should not panic even if no core/ directory found
        let _source = CoreSource::auto_detect();
    }
}
