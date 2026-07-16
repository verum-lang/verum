//! Shared module utilities for Verum compilation.
//!

//! This module provides common functionality used by both stdlib compilation
//! and regular program compilation, eliminating duplication between the two modes.
//!

//! # Key Features
//!

//! - **Module-level @cfg handling**: Evaluate @cfg attributes to determine if modules
//!  should be compiled for the current target
//! - **Platform detection**: Target-aware module filtering based on target_os/target_arch
//! - **Import validation**: Common import validation logic (to be moved here)
//!

//! # Usage
//!

//! Both `core_compiler.rs` and `pipeline.rs` should use these shared utilities
//! instead of implementing their own versions.

use std::path::Path;

use verum_ast::cfg::{CfgEvaluator, TargetConfig};

// =============================================================================
// MODULE-LEVEL @CFG HANDLING
// =============================================================================

/// Checks if a module should be compiled for the given target.
///

/// This function uses a two-tier approach:
/// 1. Check for module-level @cfg attributes in mod.vr (most flexible)
/// 2. Fall back to path-based platform detection (backwards compatibility)
///

/// # Arguments
///

/// * `module_name` - The module path (e.g., "sys.linux", "sys.darwin.libsystem")
/// * `target` - The target configuration to check against
///

/// # Returns
///

/// `true` if the module should be compiled for this target, `false` otherwise.
///

/// # Examples
///

/// ```ignore
/// let target = TargetConfig::host();
///

/// // On macOS:
/// assert!(should_compile_module_for_target("sys.darwin", &target));
/// assert!(!should_compile_module_for_target("sys.linux", &target));
///

/// // Cross-platform module:
/// assert!(should_compile_module_for_target("core", &target));
/// ```
pub fn should_compile_module_for_target(module_name: &str, target: &TargetConfig) -> bool {
    // Path-based platform detection for backwards compatibility
    // Handles "sys.linux", "sys.linux.syscall", etc.
    if module_name == "sys.linux" || module_name.starts_with("sys.linux.") {
        return target.target_os.as_str() == "linux";
    }
    // macOS reports target_os as "macos", but Darwin is the folder name
    if module_name == "sys.darwin" || module_name.starts_with("sys.darwin.") {
        return target.target_os.as_str() == "macos";
    }
    if module_name == "sys.windows" || module_name.starts_with("sys.windows.") {
        return target.target_os.as_str() == "windows";
    }
    true
}

/// Checks for module-level @cfg attribute in a mod.vr file.
///

/// This function parses the beginning of a mod.vr file to find @cfg attributes
/// that determine if the module should be compiled for the current target.
///

/// # Supported @cfg Patterns
///

/// - `@cfg(target_os = "linux")`
/// - `@cfg(target_os = "macos")`
/// - `@cfg(target_os = "windows")`
/// - `@cfg(target_arch = "x86_64")`
/// - `@cfg(target_arch = "aarch64")`
/// - `@cfg(unix)` - matches linux and macos
/// - `@cfg(windows)` - matches windows
///

/// # Arguments
///

/// * `mod_vr_path` - Path to the mod.vr file to check
/// * `target` - The target configuration to evaluate against
///

/// # Returns
///

/// `true` if the module should be compiled (either no @cfg or @cfg matches),
/// `false` if @cfg attribute is present and doesn't match.
///

/// # Example
///

/// ```ignore
/// // core/sys/linux/mod.vr contains:
/// // @cfg(target_os = "linux")
///

/// let path = Path::new("core/sys/linux/mod.vr");
/// let target = TargetConfig::host(); // On macOS
///

/// assert!(!check_module_cfg(&path, &target)); // Returns false on macOS
/// ```
pub fn check_module_cfg(mod_vr_path: &Path, target: &TargetConfig) -> bool {
    // Read the file
    let contents = match std::fs::read_to_string(mod_vr_path) {
        Ok(c) => c,
        Err(_) => return true, // If can't read, assume should compile
    };

    check_module_cfg_from_content(&contents, target)
}

/// Checks for module-level @cfg attribute from file content.
///

/// Same as `check_module_cfg` but operates on file content instead of path.
/// Useful for embedded stdlib where files are read from VFS.
///

/// # Arguments
///

/// * `content` - The file content to check
/// * `target` - The target configuration to evaluate against
///

/// # Returns
///

/// `true` if the module should be compiled (either no @cfg or @cfg matches),
/// `false` if @cfg attribute is present and doesn't match.
pub fn check_module_cfg_from_content(content: &str, target: &TargetConfig) -> bool {
    // Look for @cfg attribute at the top of the file
    // Pattern: @cfg(predicate) on its own line
    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with("//") {
            continue;
        }

        // Check for @cfg attribute
        if trimmed.starts_with("@cfg(") && trimmed.ends_with(')') {
            // Extract the predicate
            let predicate_str = &trimmed[5..trimmed.len() - 1];

            // Parse and evaluate common predicates
            return evaluate_cfg_string(predicate_str, target);
        }

        // Once we hit a non-comment, non-cfg line, stop looking
        // (module-level @cfg should be at the very top, before any code)
        if !trimmed.is_empty() && !trimmed.starts_with('@') {
            break;
        }
    }

    true // No @cfg found, compile by default
}

/// Simple string-based evaluation of cfg predicates.
///

/// This provides a lightweight alternative to full AST-based @cfg evaluation
/// for the common case of module-level platform checks.
///

/// Supports:
/// - `target_os = "value"`
/// - `target_arch = "value"`
/// - Simple identifiers: `unix`, `windows`, `linux`, `macos`
///

/// # Arguments
///

/// * `predicate` - The predicate string (e.g., `target_os = "linux"`)
/// * `target` - The target configuration to evaluate against
///

/// # Returns
///

/// `true` if the predicate matches the target, `false` otherwise.
pub fn evaluate_cfg_string(predicate: &str, target: &TargetConfig) -> bool {
    let predicate = predicate.trim();

    // Handle target_os = "value"
    if let Some(rest) = predicate.strip_prefix("target_os") {
        let rest = rest.trim();
        if let Some(value) = rest.strip_prefix('=') {
            let value = value.trim().trim_matches('"');
            return target.target_os.as_str() == value;
        }
    }

    // Handle target_arch = "value"
    if let Some(rest) = predicate.strip_prefix("target_arch") {
        let rest = rest.trim();
        if let Some(value) = rest.strip_prefix('=') {
            let value = value.trim().trim_matches('"');
            return target.target_arch.as_str() == value;
        }
    }

    // Handle simple identifiers like "unix", "windows"
    match predicate {
        "unix" => target.target_os.as_str() == "linux" || target.target_os.as_str() == "macos",
        "windows" => target.target_os.as_str() == "windows",
        "linux" => target.target_os.as_str() == "linux",
        "macos" => target.target_os.as_str() == "macos",
        _ => true, // Unknown predicate, assume true
    }
}

// =============================================================================
// TYPE DECLARATION FILTERING
// =============================================================================

/// Filter a type declaration's fields based on @cfg attributes.
///

/// This is necessary because platform-specific fields (e.g., `inner: IoUringDriver`)
/// should not be processed on platforms where the referenced type doesn't exist.
///

/// # Arguments
///

/// * `type_decl` - The type declaration to filter
/// * `target` - The target configuration for @cfg evaluation
///

/// # Returns
///

/// A new TypeDecl with fields filtered based on @cfg attributes.
pub fn filter_type_decl_for_target(
    type_decl: &verum_ast::TypeDecl,
    target: &TargetConfig,
) -> verum_ast::TypeDecl {
    use verum_ast::cfg::parse_cfg_predicate;
    use verum_ast::decl::TypeDeclBody;
    use verum_common::Maybe;

    let cfg_evaluator = CfgEvaluator::with_config(target.clone());

    let filtered_body = match &type_decl.body {
        TypeDeclBody::Record(fields) => {
            let filtered_fields: Vec<_> = fields
                .iter()
                .filter(|field| {
                    // Check if field has @cfg attribute that should filter it out
                    for attr in field.attributes.iter() {
                        if attr.name.as_str() == "cfg" {
                            if let Maybe::Some(ref args) = attr.args {
                                // Try to parse and evaluate the cfg predicate
                                if let Some(expr) = args.first() {
                                    if let Maybe::Some(predicate) = parse_cfg_predicate(expr) {
                                        return cfg_evaluator.evaluate(&predicate);
                                    }
                                }
                            }
                        }
                    }
                    true // Keep field if no @cfg or if @cfg evaluates to true
                })
                .cloned()
                .collect();
            TypeDeclBody::Record(verum_common::List::from(filtered_fields))
        }
        TypeDeclBody::Variant(variants) => {
            let filtered_variants: Vec<_> = variants
                .iter()
                .filter(|variant| {
                    // Check if variant has @cfg attribute that should filter it out
                    for attr in variant.attributes.iter() {
                        if attr.name.as_str() == "cfg" {
                            if let Maybe::Some(ref args) = attr.args {
                                if let Some(expr) = args.first() {
                                    if let Maybe::Some(predicate) = parse_cfg_predicate(expr) {
                                        return cfg_evaluator.evaluate(&predicate);
                                    }
                                }
                            }
                        }
                    }
                    true // Keep variant if no @cfg or if @cfg evaluates to true
                })
                .cloned()
                .collect();
            TypeDeclBody::Variant(verum_common::List::from(filtered_variants))
        }
        // Other body types pass through unchanged
        other => other.clone(),
    };

    verum_ast::TypeDecl {
        visibility: type_decl.visibility.clone(),
        name: type_decl.name.clone(),
        generics: type_decl.generics.clone(),
        attributes: type_decl.attributes.clone(),
        body: filtered_body,
        resource_modifier: type_decl.resource_modifier,
        generic_where_clause: type_decl.generic_where_clause.clone(),
        meta_where_clause: type_decl.meta_where_clause.clone(),
        span: type_decl.span,
    }
}

// =============================================================================
// MODULE PATH UTILITIES
// =============================================================================

/// Converts a module path from dotted notation to file system path.
///

/// # Examples
///

/// ```ignore
/// assert_eq!(module_path_to_fs("sys.linux"), "sys/linux");
/// assert_eq!(module_path_to_fs("core"), "core");
/// ```
pub fn module_path_to_fs(module_path: &str) -> String {
    module_path.replace('.', "/")
}

/// Converts a file system path to module path notation.
///

/// # Arguments
///

/// * `fs_path` - The file system path relative to stdlib root
///

/// # Examples
///

/// ```ignore
/// assert_eq!(fs_path_to_module("sys/linux"), "sys.linux");
/// assert_eq!(fs_path_to_module("core/memory.vr"), "core.memory");
/// ```
pub fn fs_path_to_module(fs_path: &str) -> String {
    fs_path
        .trim_end_matches(".vr")
        .trim_end_matches("/mod")
        .replace('/', ".")
}

/// Derives a submodule path from the parent module name and file path.
///

/// This is used to determine the full module path for files within a module directory.
///

/// # Examples
///

/// ```ignore
/// // "core/base/memory.vr" with module_name "core" -> "core.memory"
/// // "core/base/mod.vr" with module_name "core" -> "core"
/// ```
pub fn derive_submodule_path(module_name: &str, file_path: &Path) -> String {
    let file_name = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if file_name == "mod" {
        // mod.vr exports at the module level
        module_name.to_string()
    } else {
        // Other files export at submodule level
        format!("{}.{}", module_name, file_name)
    }
}

// =============================================================================
// RUNTIME-GATED MODULE FILTERING (task #44 — CFG-RUNTIME-PRECOMPILE-FILTER-1)
// =============================================================================

/// Runtime-profile cfg values whose file-submodules are EXCLUDED from the
/// default (full-runtime) stdlib bake.  A `mod.vr` line like
///
/// ```text
/// @cfg(runtime = "embedded")
/// public module embedded;
/// ```
///
/// declares `embedded.vr` (or an `embedded/` subtree) as belonging to a
/// reduced runtime that the default archive must not carry — baking it
/// pollutes `runtime.vbca` with symbols like `core.sys.embedded.BumpAllocator`
/// that then collide with the real (full-runtime) types (the #41 dup-type
/// class) and inflate the archive.  Only these reduced runtimes are filtered;
/// `runtime = "full"` / `"single_thread"` etc. stay in the default bake, and
/// `target_os` / `target_arch` module gates are NEVER filtered here (they are
/// needed for cross-target codegen and handled by `check_module_cfg`).
pub const DEFAULT_EXCLUDED_RUNTIMES: [&str; 2] = ["embedded", "none"];

/// The set of stdlib source files excluded from the default bake because
/// they back a `@cfg(runtime = "embedded"|"none")` module declaration.
///
/// Paths are stored relative to the `core/` root, forward-slash normalised
/// (e.g. `"sys/embedded.vr"`), matching the keys used by `build.rs`'s
/// `collect_vr_files` and derivable from the absolute paths walked by
/// `core_compiler::discover_modules`.
#[derive(Debug, Default, Clone)]
pub struct RuntimeGatedModules {
    /// Exact excluded file paths, e.g. `"sys/embedded.vr"`.
    files: std::collections::HashSet<String>,
    /// Excluded directory prefixes (trailing `/`), e.g. `"sys/embedded/"`,
    /// for the case where the gated submodule is a directory with its own
    /// `mod.vr` rather than a single file.
    dirs: Vec<String>,
}

impl RuntimeGatedModules {
    /// True if `rel_path` (core-relative, forward-slash) is a runtime-gated
    /// file that must be dropped from the default bake.
    pub fn excludes(&self, rel_path: &str) -> bool {
        self.files.contains(rel_path)
            || self.dirs.iter().any(|d| rel_path.starts_with(d.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.dirs.is_empty()
    }

    pub fn excluded_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.files.iter().cloned().collect();
        v.extend(self.dirs.iter().cloned());
        v.sort();
        v.dedup();
        v
    }
}

/// Walk `core_root` for every `mod.vr` and collect the files backing
/// module declarations gated to a reduced runtime (see
/// [`DEFAULT_EXCLUDED_RUNTIMES`]).  This is the single authority consulted
/// by both the source-embed walk (`build.rs`, via a mirrored standalone copy)
/// and the VBC-compilation walk (`core_compiler::discover_modules`) so the two
/// tiers agree on exactly which files leave the default archive.
///
/// The scan is deliberately line-oriented (no full parse) — the same
/// regex-light discipline `build_dep_graph` uses — because it runs inside a
/// build script that must not drag in `verum_fast_parser`.
pub fn runtime_gated_modules(core_root: &Path) -> RuntimeGatedModules {
    let mut out = RuntimeGatedModules::default();
    collect_runtime_gates(core_root, core_root, &mut out);
    out
}

fn collect_runtime_gates(core_root: &Path, dir: &Path, out: &mut RuntimeGatedModules) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // Skip build-artefact / hidden trees, mirroring the walkers.
            if name.starts_with('.') || name.starts_with('_') || name == "target" {
                continue;
            }
            collect_runtime_gates(core_root, &path, out);
        } else if name == "mod.vr" {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let rel_dir = dir
                    .strip_prefix(core_root)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                scan_mod_vr_runtime_gates(&content, &rel_dir, out);
            }
        }
    }
}

/// Parse one `mod.vr` for `@cfg(runtime = "…")` attributes that guard a
/// file-submodule declaration (`[public] module NAME;`), recording the
/// backing file / subtree under `rel_dir` (core-relative).  Inline module
/// blocks (`module NAME { … }`) and item-level cfgs are ignored — only the
/// terminating-`;` file form names a whole file.
fn scan_mod_vr_runtime_gates(content: &str, rel_dir: &str, out: &mut RuntimeGatedModules) {
    let mut pending_gate = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if let Some(value) = parse_runtime_cfg_value(trimmed) {
            pending_gate = DEFAULT_EXCLUDED_RUNTIMES.contains(&value.as_str());
            continue;
        }
        if pending_gate {
            if let Some(name) = parse_module_decl_name(trimmed) {
                let base = if rel_dir.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", rel_dir, name)
                };
                out.files.insert(format!("{}.vr", base));
                out.dirs.push(format!("{}/", base));
            }
        }
        // Any declaration line consumes the pending gate; a duplicate
        // decl (e.g. `no_runtime` guarded by two runtimes) re-arms via its
        // own preceding `@cfg` line.
        pending_gate = false;
    }
}

/// Extract `V` from a `@cfg(runtime = "V")` attribute line, if present.
fn parse_runtime_cfg_value(trimmed: &str) -> Option<String> {
    let inner = trimmed.strip_prefix("@cfg(")?.strip_suffix(')')?;
    let rest = inner.trim().strip_prefix("runtime")?.trim();
    let rest = rest.strip_prefix('=')?.trim();
    Some(rest.trim_matches('"').to_string())
}

/// Extract `NAME` from a `[public] module NAME;` file-submodule declaration.
/// Returns `None` for inline blocks (`module NAME { … }`) or non-module lines.
fn parse_module_decl_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("public ").unwrap_or(trimmed).trim_start();
    let rest = rest.strip_prefix("module ")?;
    // Cut off a trailing comment / decl body so only the name token remains.
    let head = rest
        .split(&['{', ';', '/', ' ', '\t'][..])
        .next()
        .unwrap_or("")
        .trim();
    // File-submodule form must terminate in `;` (inline blocks open a `{`).
    if !rest.contains(';') || rest.trim_start().starts_with('{') {
        return None;
    }
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::{List, Map, Text};

    #[test]
    fn test_should_compile_module_for_target() {
        let linux_target = TargetConfig {
            target_os: Text::from("linux"),
            target_arch: Text::from("x86_64"),
            target_env: Text::from("gnu"),
            target_vendor: Text::from("unknown"),
            target_family: Text::from("unix"),
            target_pointer_width: Text::from("64"),
            target_endian: Text::from("little"),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        };

        let macos_target = TargetConfig {
            target_os: Text::from("macos"),
            target_arch: Text::from("aarch64"),
            target_env: Text::from(""),
            target_vendor: Text::from("apple"),
            target_family: Text::from("unix"),
            target_pointer_width: Text::from("64"),
            target_endian: Text::from("little"),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        };

        // Linux target
        assert!(should_compile_module_for_target("sys.linux", &linux_target));
        assert!(should_compile_module_for_target(
            "sys.linux.syscall",
            &linux_target
        ));
        assert!(!should_compile_module_for_target(
            "sys.darwin",
            &linux_target
        ));
        assert!(!should_compile_module_for_target(
            "sys.windows",
            &linux_target
        ));
        assert!(should_compile_module_for_target("core", &linux_target));

        // macOS target
        assert!(!should_compile_module_for_target(
            "sys.linux",
            &macos_target
        ));
        assert!(should_compile_module_for_target(
            "sys.darwin",
            &macos_target
        ));
        assert!(should_compile_module_for_target(
            "sys.darwin.libsystem",
            &macos_target
        ));
        assert!(!should_compile_module_for_target(
            "sys.windows",
            &macos_target
        ));
        assert!(should_compile_module_for_target("core", &macos_target));
    }

    #[test]
    fn test_evaluate_cfg_string() {
        let linux_target = TargetConfig {
            target_os: Text::from("linux"),
            target_arch: Text::from("x86_64"),
            target_env: Text::from("gnu"),
            target_vendor: Text::from("unknown"),
            target_family: Text::from("unix"),
            target_pointer_width: Text::from("64"),
            target_endian: Text::from("little"),
            features: List::new(),
            test: false,
            debug_assertions: false,
            custom: Map::new(),
        };

        assert!(evaluate_cfg_string("target_os = \"linux\"", &linux_target));
        assert!(!evaluate_cfg_string("target_os = \"macos\"", &linux_target));
        assert!(evaluate_cfg_string(
            "target_arch = \"x86_64\"",
            &linux_target
        ));
        assert!(evaluate_cfg_string("unix", &linux_target));
        assert!(!evaluate_cfg_string("windows", &linux_target));
        assert!(evaluate_cfg_string("linux", &linux_target));
    }

    #[test]
    fn test_module_path_conversion() {
        assert_eq!(module_path_to_fs("sys.linux"), "sys/linux");
        assert_eq!(module_path_to_fs("core"), "core");
        assert_eq!(
            module_path_to_fs("sys.darwin.libsystem"),
            "sys/darwin/libsystem"
        );

        assert_eq!(fs_path_to_module("sys/linux"), "sys.linux");
        assert_eq!(fs_path_to_module("core/memory.vr"), "core.memory");
        assert_eq!(fs_path_to_module("sys/darwin/mod"), "sys.darwin");
    }
}
