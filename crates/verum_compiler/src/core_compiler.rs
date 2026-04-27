// ARCHITECTURE NOTE: Dependency graph and topological sort logic is shared with core_source.rs.
// When core/ migrates to a proper cog, both will delegate to a unified CoreDependencyResolver.
// The known_deps map should be derived from parsing `mount` statements in .vr files.
//! Stdlib Compilation Support Types
//!
//! This module provides types and utilities for stdlib compilation:
//! - `CoreConfig` - Configuration for stdlib compilation
//! - `StdlibCompilationResult` - Result of stdlib compilation
//! - `StdlibModule` - Module definition for stdlib
//! - `StdlibModuleResolver` - Module discovery and dependency resolution
//! - `build_export_index` - Build export index for import validation
//! - `validate_imports` - Validate imports against export index
//!
//! # Architecture
//!
//! Stdlib compilation is handled by `CompilationPipeline::compile_core()` in
//! `BuildMode::StdlibBootstrap` mode. See `pipeline.rs` for the unified compilation
//! implementation.
//!
//! # Usage
//!
//! ```ignore
//! use verum_compiler::{CompilationPipeline, CompilerOptions, Session, CoreConfig};
//!
//! let config = CoreConfig::new("stdlib")
//!     .with_output("target/stdlib.vbca");
//!
//! let mut session = Session::new(CompilerOptions::default());
//! let mut pipeline = CompilationPipeline::new_core(&mut session, config);
//!
//! let result = pipeline.compile_core()?;
//! println!("Compiled {} modules", result.modules_compiled);
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use verum_common::List;
use verum_diagnostics::Diagnostic;
use verum_ast::cfg::TargetConfig;

use crate::api::{CompilationError, CompilationErrorKind};
use crate::module_utils;

/// Module exports index for import validation.
/// Maps module path (e.g., "core.memory") to a set of exported item names.
pub type ModuleExportsIndex = HashMap<String, HashSet<String>>;

/// Configuration for stdlib compilation
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// Path to stdlib directory
    pub stdlib_path: PathBuf,
    /// Output path for the archive
    pub output_path: PathBuf,
    /// Include debug information
    pub debug_info: bool,
    /// Include source maps
    pub source_maps: bool,
    /// Optimization level (0-3)
    pub optimization_level: u8,
    /// Parallel compilation
    pub parallel: bool,
    /// Verbose output
    pub verbose: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            stdlib_path: PathBuf::from("stdlib"),
            output_path: PathBuf::from("target/stdlib.vbca"),
            debug_info: false,
            source_maps: false,
            optimization_level: 2,
            parallel: true,
            verbose: false,
        }
    }
}

impl CoreConfig {
    /// Creates a new config with the given stdlib path
    pub fn new(stdlib_path: impl Into<PathBuf>) -> Self {
        Self {
            stdlib_path: stdlib_path.into(),
            ..Default::default()
        }
    }

    /// Sets the output path
    pub fn with_output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output_path = path.into();
        self
    }

    /// Enables debug info
    pub fn with_debug_info(mut self) -> Self {
        self.debug_info = true;
        self
    }

    /// Enables source maps
    pub fn with_source_maps(mut self) -> Self {
        self.source_maps = true;
        self
    }
}

/// Result of stdlib compilation
#[derive(Debug, Clone)]
pub struct StdlibCompilationResult {
    /// Number of modules compiled
    pub modules_compiled: usize,
    /// Number of functions compiled
    pub functions_compiled: usize,
    /// Total compilation time
    pub total_time: Duration,
    /// Time per module (module name -> duration)
    pub module_times: HashMap<String, Duration>,
    /// Output archive path
    pub output_path: PathBuf,
    /// Output archive size in bytes
    pub output_size: u64,
    /// Any warnings generated
    pub warnings: List<Diagnostic>,
    /// Any errors generated (in strict mode)
    pub errors: List<Diagnostic>,
}

/// Module definition for stdlib
#[derive(Debug, Clone)]
pub struct StdlibModule {
    /// Module name (e.g., "core", "collections.list")
    pub name: String,
    /// Source files in this module
    pub source_files: Vec<PathBuf>,
    /// Dependencies (other module names)
    pub dependencies: Vec<String>,
}

/// Build an index of all publicly exported items (functions, types) per module path.
///
/// This index is used for import validation to check that imported items actually exist.
/// The key is the module path (e.g., "core.memory") and the value is a set of exported names.
///
/// The submodule path is derived from the file name:
/// - "core/memory.vr" -> "core.memory"
/// - "core/mod.vr" -> "core" (mod.vr exports to parent module)
pub fn build_export_index(all_modules: &[(String, Vec<(PathBuf, verum_ast::Module)>)]) -> ModuleExportsIndex {
    let mut index: ModuleExportsIndex = HashMap::new();

    for (module_name, ast_modules) in all_modules {
        for (file_path, ast_module) in ast_modules {
            // Derive submodule path from file name
            // e.g., "core/base/memory.vr" with module_name "core" -> "core.memory"
            let submodule_path = module_utils::derive_submodule_path(module_name, file_path);

            // Collect exports from this AST module
            let exports = index.entry(submodule_path.clone()).or_insert_with(HashSet::new);

            for item in &ast_module.items {
                match &item.kind {
                    verum_ast::ItemKind::Function(func) => {
                        // Only public functions are importable
                        if matches!(func.visibility, verum_ast::Visibility::Public) {
                            exports.insert(func.name.name.to_string());
                        }
                    }
                    verum_ast::ItemKind::Type(type_decl) => {
                        // Only public types are importable
                        if matches!(type_decl.visibility, verum_ast::Visibility::Public) {
                            exports.insert(type_decl.name.name.to_string());
                        }
                    }
                    verum_ast::ItemKind::Const(const_decl) => {
                        // Only public constants are importable
                        if matches!(const_decl.visibility, verum_ast::Visibility::Public) {
                            exports.insert(const_decl.name.name.to_string());
                        }
                    }
                    verum_ast::ItemKind::Static(static_decl) => {
                        // Only public statics are importable
                        if matches!(static_decl.visibility, verum_ast::Visibility::Public) {
                            exports.insert(static_decl.name.name.to_string());
                        }
                    }
                    verum_ast::ItemKind::Protocol(protocol_decl) => {
                        // Only public protocols are importable
                        if matches!(protocol_decl.visibility, verum_ast::Visibility::Public) {
                            exports.insert(protocol_decl.name.name.to_string());
                        }
                    }
                    verum_ast::ItemKind::Mount(import_decl) => {
                        // Public imports are re-exports - add the imported items as exports
                        if matches!(import_decl.visibility, verum_ast::Visibility::Public) {
                            collect_import_exports(&import_decl.tree, exports);
                        }
                    }
                    verum_ast::ItemKind::ExternBlock(extern_block) => {
                        // Only export FFI functions that are explicitly marked public
                        // This maintains semantic honesty - extern functions are private by default
                        for func in extern_block.functions.iter() {
                            if matches!(func.visibility, verum_ast::Visibility::Public) {
                                exports.insert(func.name.name.to_string());
                            }
                        }
                    }
                    verum_ast::ItemKind::Axiom(axiom_decl) => {
                        // Axioms with a signature (generics/params/return) behave
                        // like callable declarations (e.g. `ua<A, B>(e: Equiv<A, B>)
                        // -> Path<Type>(A, B)` in `core.math.hott`). They must be
                        // importable across modules to support `mount core.math.hott.{ua}`.
                        if matches!(axiom_decl.visibility, verum_ast::Visibility::Public) {
                            exports.insert(axiom_decl.name.name.to_string());
                        }
                    }
                    _ => {}
                }
            }

            // Also add exports to the parent module for mod.vr files
            // This allows imports like `.memory.Heap` to work when memory/mod.vr exports Heap
            if file_path.file_name().map_or(false, |n| n == "mod.vr") {
                let parent_exports = index.entry(module_name.clone()).or_insert_with(HashSet::new);
                for item in &ast_module.items {
                    match &item.kind {
                        verum_ast::ItemKind::Function(func) => {
                            if matches!(func.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(func.name.name.to_string());
                            }
                        }
                        verum_ast::ItemKind::Type(type_decl) => {
                            if matches!(type_decl.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(type_decl.name.name.to_string());
                            }
                        }
                        verum_ast::ItemKind::Const(const_decl) => {
                            if matches!(const_decl.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(const_decl.name.name.to_string());
                            }
                        }
                        verum_ast::ItemKind::Static(static_decl) => {
                            if matches!(static_decl.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(static_decl.name.name.to_string());
                            }
                        }
                        verum_ast::ItemKind::Protocol(protocol_decl) => {
                            if matches!(protocol_decl.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(protocol_decl.name.name.to_string());
                            }
                        }
                        verum_ast::ItemKind::Mount(import_decl) => {
                            // Public imports are re-exports - add the imported items as exports
                            if matches!(import_decl.visibility, verum_ast::Visibility::Public) {
                                collect_import_exports(&import_decl.tree, parent_exports);
                            }
                        }
                        verum_ast::ItemKind::ExternBlock(extern_block) => {
                            // Only export FFI functions that are explicitly marked public
                            for func in extern_block.functions.iter() {
                                if matches!(func.visibility, verum_ast::Visibility::Public) {
                                    parent_exports.insert(func.name.name.to_string());
                                }
                            }
                        }
                        verum_ast::ItemKind::Axiom(axiom_decl) => {
                            if matches!(axiom_decl.visibility, verum_ast::Visibility::Public) {
                                parent_exports.insert(axiom_decl.name.name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    index
}

/// Recursively collect exported item names from an import tree.
///
/// For `public import foo.{Bar, Baz as B}`:
/// - Bar is exported as "Bar"
/// - Baz is exported as "B" (using alias)
fn collect_import_exports(tree: &verum_ast::MountTree, exports: &mut HashSet<String>) {
    use verum_ast::MountTreeKind;
    use verum_ast::ty::PathSegment;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Single item import: `import foo.Bar` or `import foo.Bar as Baz`
            // The last segment is the item name, unless aliased
            if let Some(alias) = &tree.alias.as_ref() {
                exports.insert(alias.name.to_string());
            } else if let Some(last_segment) = path.segments.last() {
                if let PathSegment::Name(ident) = last_segment {
                    exports.insert(ident.name.to_string());
                }
            }
        }
        MountTreeKind::Glob(_) => {
            // Glob imports re-export everything from the source module
            // We can't statically know what items are exported without
            // resolving the source module first. For now, skip these.
            // The validation will handle them separately.
        }
        MountTreeKind::Nested { prefix: _, trees } => {
            // Nested imports: `import foo.{Bar, Baz, inner.Qux}`
            // Recursively collect from each sub-tree
            for sub_tree in trees.iter() {
                collect_import_exports(sub_tree, exports);
            }
        }
    }
}

/// Resolve a relative import path to an absolute module path.
///
/// When `is_prefix` is true, all segments are treated as module path (no item extraction).
/// When `is_prefix` is false, the last segment is treated as the item name.
///
/// Examples:
/// - `.memory` in module `core` with is_prefix=true -> `core.memory`, None
/// - `.memory.size_of` in module `core` with is_prefix=false -> `core.memory`, Some("size_of")
/// - `super.super.mem.allocator` in module `core` -> `mem.allocator`
fn resolve_import_path(
    import_path: &verum_ast::ty::Path,
    current_module: &str,
    is_prefix: bool,
) -> Option<(String, Option<String>)> {
    use verum_ast::ty::PathSegment;

    if import_path.segments.is_empty() {
        return None;
    }

    let mut module_parts: Vec<String> = Vec::new();
    let mut item_name: Option<String> = None;
    let mut is_relative = false;
    let mut super_count = 0;

    // Process path segments
    for (i, segment) in import_path.segments.iter().enumerate() {
        match segment {
            PathSegment::Relative => {
                is_relative = true;
            }
            PathSegment::Super => {
                super_count += 1;
            }
            PathSegment::Name(ident) => {
                // Last segment is the item name (unless this is a prefix path)
                if !is_prefix && i == import_path.segments.len() - 1 {
                    item_name = Some(ident.name.to_string());
                } else {
                    module_parts.push(ident.name.to_string());
                }
            }
            PathSegment::Cog => {
                // Crate root - start fresh
                module_parts.clear();
            }
            PathSegment::SelfValue => {
                // Self refers to current module
            }
        }
    }

    // Resolve relative paths
    let resolved_module = if is_relative || super_count > 0 {
        let current_parts: Vec<&str> = current_module.split('.').collect();

        // Apply super navigation
        let remaining = if super_count > 0 {
            current_parts
                .get(..current_parts.len().saturating_sub(super_count))
                .unwrap_or(&[])
                .to_vec()
        } else {
            current_parts.clone()
        };

        // Combine with relative path
        let mut result: Vec<String> = remaining.iter().map(|s| s.to_string()).collect();
        result.extend(module_parts);
        result.join(".")
    } else {
        module_parts.join(".")
    };

    Some((resolved_module, item_name))
}

/// Validate imports and collect errors for missing items.
///
/// Returns a list of (module_path, item_name, similar_names, span) tuples for items that don't exist.
pub fn validate_imports(
    all_modules: &[(String, Vec<(PathBuf, verum_ast::Module)>)],
    export_index: &ModuleExportsIndex,
    target: &TargetConfig,
) -> Vec<(String, String, String, verum_ast::Span)> {
    let mut errors = Vec::new();

    for (module_name, ast_modules) in all_modules {
        for (file_path, ast_module) in ast_modules {
            // Derive the current module context for relative path resolution
            let current_module = module_utils::derive_submodule_path(module_name, file_path);

            for item in &ast_module.items {
                if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                    validate_import_tree(
                        &import_decl.tree,
                        &current_module,
                        export_index,
                        target,
                        &mut errors,
                    );
                }
            }
        }
    }

    errors
}

/// Recursively validate an import tree.
fn validate_import_tree(
    tree: &verum_ast::MountTree,
    current_module: &str,
    export_index: &ModuleExportsIndex,
    target: &TargetConfig,
    errors: &mut Vec<(String, String, String, verum_ast::Span)>,
) {
    use verum_ast::decl::MountTreeKind;

    match &tree.kind {
        MountTreeKind::Path(path) => {
            // Single import like `.memory.size_of`
            // is_prefix=false because the last segment is the item name
            if let Some((module_path, Some(item_name))) = resolve_import_path(path, current_module, false) {
                // Check if module exists
                if let Some(exports) = export_index.get(&module_path) {
                    // Check if item exists in module
                    if !exports.contains(&item_name) {
                        // Find similar names for suggestion
                        let similar: Vec<String> = exports
                            .iter()
                            .filter(|name| {
                                // Simple similarity check - same prefix or edit distance
                                name.starts_with(&item_name[..item_name.len().clamp(1, 3)])
                                    || item_name.starts_with(&name[..name.len().clamp(1, 3)])
                            })
                            .take(3)
                            .cloned()
                            .collect();

                        errors.push((
                            module_path,
                            item_name,
                            similar.join(", "),
                            tree.span,
                        ));
                    }
                } else {
                    // Module doesn't exist in export index - check if it's a platform-specific
                    // module that wasn't compiled for the current target
                    if module_utils::should_compile_module_for_target(&module_path, target) {
                        // Module should exist but doesn't - report error
                        errors.push((
                            module_path.clone(),
                            item_name,
                            String::new(),
                            tree.span,
                        ));
                    }
                    // If module is platform-specific and not for current target, skip error
                }
            }
        }
        MountTreeKind::Glob(_path) => {
            // Glob imports are harder to validate - skip for now
        }
        MountTreeKind::Nested { prefix, trees } => {
            // Resolve the module path from prefix (is_prefix=true to treat entire path as module)
            if let Some((module_path, _)) = resolve_import_path(prefix, current_module, true) {
                // Check if module is platform-specific and not for current target
                // If so, skip all nested imports
                if !module_utils::should_compile_module_for_target(&module_path, target) {
                    return;
                }

                // Validate each nested import
                for nested_tree in trees.iter() {
                    // For nested imports, the item name is directly in the tree
                    if let MountTreeKind::Path(item_path) = &nested_tree.kind {
                        if let Some(ident) = item_path.as_ident() {
                            let item_name = ident.name.to_string();

                            // Check if module exists
                            if let Some(exports) = export_index.get(&module_path) {
                                // Check if item exists in module
                                if !exports.contains(&item_name) {
                                    let similar: Vec<String> = exports
                                        .iter()
                                        .filter(|name| {
                                            name.starts_with(&item_name[..item_name.len().clamp(1, 3)])
                                                || item_name.starts_with(&name[..name.len().clamp(1, 3)])
                                        })
                                        .take(3)
                                        .cloned()
                                        .collect();

                                    errors.push((
                                        module_path.clone(),
                                        item_name,
                                        similar.join(", "),
                                        nested_tree.span,
                                    ));
                                }
                            } else {
                                // Module doesn't exist in export index - check if platform-specific
                                if module_utils::should_compile_module_for_target(&module_path, target) {
                                    errors.push((
                                        module_path.clone(),
                                        item_name,
                                        String::new(),
                                        nested_tree.span,
                                    ));
                                }
                            }
                        }
                    } else {
                        // Recursively validate nested trees
                        validate_import_tree(
                            nested_tree,
                            current_module,
                            export_index,
                            target,
                            errors,
                        );
                    }
                }
            }
        }
    }
}

/// Resolves stdlib modules and their dependencies
#[derive(Debug)]
pub struct StdlibModuleResolver {
    /// Root stdlib path
    stdlib_path: PathBuf,
    /// Discovered modules
    modules: HashMap<String, StdlibModule>,
    /// Compilation order (topologically sorted)
    compilation_order: Vec<String>,
}

impl StdlibModuleResolver {
    /// Creates a new resolver for the given stdlib path
    pub fn new(stdlib_path: impl Into<PathBuf>) -> Self {
        Self {
            stdlib_path: stdlib_path.into(),
            modules: HashMap::new(),
            compilation_order: Vec::new(),
        }
    }

    /// Discovers all modules in the stdlib directory
    pub fn discover(&mut self) -> Result<(), CompilationError> {
        let stdlib_path = self.stdlib_path.clone();
        self.discover_modules(&stdlib_path, "")?;
        self.resolve_dependencies()?;
        self.compute_compilation_order()?;
        Ok(())
    }

    /// Recursively discovers modules
    fn discover_modules(&mut self, dir: &Path, prefix: &str) -> Result<(), CompilationError> {
        if !dir.is_dir() {
            return Err(CompilationError::new(
                CompilationErrorKind::IoError,
                format!("Not a directory: {}", dir.display()),
            ));
        }

        let entries = std::fs::read_dir(dir).map_err(|e| {
            CompilationError::new(
                CompilationErrorKind::IoError,
                format!("Failed to read directory {}: {}", dir.display(), e),
            )
        })?;

        let mut vr_files = Vec::new();
        let mut subdirs = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| {
                CompilationError::new(
                    CompilationErrorKind::IoError,
                    format!("Failed to read entry: {}", e),
                )
            })?;

            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() {
                // Skip hidden directories and __pycache__ etc.
                if !name.starts_with('.') && !name.starts_with('_') {
                    subdirs.push((path, name));
                }
            } else if path.extension().map_or(false, |e| e == "vr") {
                vr_files.push(path);
            }
        }

        // If there are .vr files, this is a module
        if !vr_files.is_empty() {
            let module_name = if prefix.is_empty() {
                // Root module (stdlib itself) - use "core" as canonical prefix
                "core".to_string()
            } else {
                prefix.to_string()
            };

            // Sort files for deterministic compilation order:
            // 1. mod.vr first (module definition)
            // 2. alphabetically (e.g., intrinsics.vr before thread.vr)
            // This ensures constants/functions are defined before they're imported
            vr_files.sort_by(|a, b| {
                let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if a_name == "mod.vr" {
                    std::cmp::Ordering::Less
                } else if b_name == "mod.vr" {
                    std::cmp::Ordering::Greater
                } else {
                    a.cmp(b)
                }
            });

            self.modules.insert(module_name.clone(), StdlibModule {
                name: module_name,
                source_files: vr_files,
                dependencies: Vec::new(), // Will be resolved later
            });
        }

        // Process subdirectories
        let target = TargetConfig::host();
        for (subdir, name) in subdirs {
            let new_prefix = if prefix.is_empty() {
                // Root level subdirs get "core." prefix (e.g., simd -> core.simd)
                format!("core.{}", name)
            } else {
                format!("{}.{}", prefix, name)
            };

            // Check for module-level @cfg in mod.vr
            // If @cfg evaluates to false, skip this entire module directory
            let mod_vr_path = subdir.join("mod.vr");
            if mod_vr_path.exists() && !module_utils::check_module_cfg(&mod_vr_path, &target) {
                // Module has @cfg that doesn't match current target - skip entirely
                continue;
            }

            self.discover_modules(&subdir, &new_prefix)?;
        }

        Ok(())
    }

    /// Resolves dependencies between modules by analyzing imports.
    /// Dependencies on platform-specific modules are filtered based on the host target.
    fn resolve_dependencies(&mut self) -> Result<(), CompilationError> {
        let target = TargetConfig::host();

        // Known dependency structure for stdlib
        // This is based on the module analysis done earlier
        // IMPORTANT: All modules must be listed here with correct dependencies
        // to ensure proper compilation order for cross-module constant/function references
        // IMPORTANT: Dependency order is based on actual imports analysis.
        // The order is designed to avoid circular dependencies.
        //
        // Compilation order: sys.* -> mem -> core -> sync -> text -> collections -> ...
        //
        // Key insight: `mem` does NOT import from `core` or `sync` - it only uses
        // builtins (Maybe, Result) and sys.* for platform-specific memory operations.
        // Meanwhile, `core/memory.vr` imports cbgr_alloc from `mem/allocator.vr`.
        // All module names now use "core." prefix to match import paths
        let known_deps: HashMap<&str, Vec<&str>> = [
            // core is the root module (core/mod.vr)
            ("core", vec![]),
            // sys module is the lowest level - defines shared constants (ORDERING_* etc.)
            // and intrinsics that platform-specific submodules need
            ("core.sys", vec!["core"]),
            // sys submodules depend on sys for constants like ORDERING_ACQUIRE
            ("core.sys.linux", vec!["core.sys"]),
            ("core.sys.darwin", vec!["core.sys"]),
            ("core.sys.windows", vec!["core.sys"]),
            // mem module depends only on sys.* for mmap/VirtualAlloc (NOT core/sync)
            ("core.mem", vec!["core.sys", "core.sys.linux", "core.sys.darwin", "core.sys.windows"]),
            // base depends on core AND core.mem because core/base/memory.vr
            // imports cbgr_alloc / cbgr_alloc_zeroed / cbgr_dealloc / cbgr_realloc
            // from core.mem.allocator (line 21 of memory.vr).  Without this
            // dependency edge, core.base can compile BEFORE core.mem in topological
            // order, leaving cbgr_alloc unresolved when try_alloc's body compiles.
            // The stubbed try_alloc is then never exported, which cascades into
            // every module that depends on core.base — List.try_with_capacity,
            // List.try_resize_buffer, Map.try_resize, Text.try_with_capacity all
            // get stubbed at codegen, panic at runtime with FunctionNotFound (AOT)
            // or null-deref at FatRef.is_null pc=0 (interpreter).  Documented in
            // task #200; runtime regression anchor in
            // vcs/differential/cross-impl/diff_list_try_with_capacity_runtime.vr.
            ("core.base", vec!["core", "core.mem"]),
            // sync depends on base
            ("core.sync", vec!["core.base"]),
            // text imports sys_write from sys.*
            ("core.text", vec!["core.base", "core.sys.linux", "core.sys.darwin", "core.sys.windows"]),
            // collections uses base and text
            ("core.collections", vec!["core.base", "core.text"]),
            // io uses sys.* for file operations
            ("core.io", vec!["core.base", "core.text", "core.collections", "core.sys.linux", "core.sys.darwin", "core.sys.windows"]),
            // time depends on sys submodules for monotonic_nanos, realtime_nanos, etc.
            ("core.time", vec!["core.base", "core.sys.linux", "core.sys.darwin", "core.sys.windows"]),
            // intrinsics is low-level
            ("core.intrinsics", vec!["core"]),
            // simd depends on base types and intrinsics
            ("core.simd", vec!["core.base", "core.intrinsics"]),
            // math depends on base, simd for optimized operations
            ("core.math", vec!["core.base", "core.simd"]),
            // async depends on sys for io_engine
            ("core.async", vec!["core.base", "core.collections", "core.io", "core.sync", "core.time", "core.sys"]),
            // runtime depends on base, mem, sync for thread/task management
            ("core.runtime", vec!["core.base", "core.mem", "core.sync", "core.time", "core.sys", "core.async"]),
            // term depends on sys, io, text, collections, sync, time (Layer 3.5)
            ("core.term", vec!["core.base", "core.text", "core.collections", "core.io", "core.sync", "core.time", "core.sys", "core.sys.linux", "core.sys.darwin", "core.sys.windows"]),
            ("core.net", vec!["core.base", "core.io", "core.async", "core.sys"]),
            ("core.meta", vec!["core.base"]),
            ("core.cognitive", vec!["core.base", "core.collections"]),
        ].into_iter().collect();

        // Update dependencies, filtering platform-specific modules based on target
        for (name, deps) in known_deps {
            if let Some(module) = self.modules.get_mut(name) {
                module.dependencies = deps
                    .iter()
                    .filter(|dep| module_utils::should_compile_module_for_target(dep, &target))
                    .map(|s| s.to_string())
                    .collect();
            }
        }

        Ok(())
    }

    /// Computes topological sort of modules for compilation order
    fn compute_compilation_order(&mut self) -> Result<(), CompilationError> {
        let mut visited = HashSet::new();
        let mut temp_mark = HashSet::new();
        let mut order = Vec::new();

        fn visit(
            name: &str,
            modules: &HashMap<String, StdlibModule>,
            visited: &mut HashSet<String>,
            temp_mark: &mut HashSet<String>,
            order: &mut Vec<String>,
        ) -> Result<(), CompilationError> {
            if visited.contains(name) {
                return Ok(());
            }
            if temp_mark.contains(name) {
                return Err(CompilationError::new(
                    CompilationErrorKind::InternalError,
                    format!("Circular dependency detected involving module: {}", name),
                ));
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
        // Diagnostic: surface the resolved order under `RUST_LOG=info` so
        // dependency-edge edits (#200 audit) are visible without grepping
        // a huge trace.  Logged once per discover() call, at info level.
        tracing::info!(
            "[stdlib] compilation order ({} modules): {}",
            self.compilation_order.len(),
            self.compilation_order.join(" -> ")
        );
        Ok(())
    }

    /// Returns modules in compilation order, filtered by the current platform.
    /// Platform-specific modules (sys.linux, sys.darwin, sys.windows) are only
    /// included when they match the host target_os.
    pub fn modules_in_order(&self) -> Vec<&StdlibModule> {
        let target = TargetConfig::host();

        self.compilation_order
            .iter()
            .filter(|name| module_utils::should_compile_module_for_target(name, &target))
            .filter_map(|name| self.modules.get(name))
            .collect()
    }

    /// Returns the total number of modules
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Returns a reference to the compilation order (topologically sorted module names).
    pub fn compilation_order(&self) -> &[String] {
        &self.compilation_order
    }

    /// Returns a module by name, if it exists.
    pub fn get_module(&self, name: &str) -> Option<&StdlibModule> {
        self.modules.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdlib_config_default() {
        let config = CoreConfig::default();
        assert_eq!(config.stdlib_path, PathBuf::from("stdlib"));
        assert_eq!(config.optimization_level, 2);
    }

    #[test]
    fn test_stdlib_config_builder() {
        let config = CoreConfig::new("custom/stdlib")
            .with_output("target/custom.vbca")
            .with_debug_info()
            .with_source_maps();

        assert_eq!(config.stdlib_path, PathBuf::from("custom/stdlib"));
        assert_eq!(config.output_path, PathBuf::from("target/custom.vbca"));
        assert!(config.debug_info);
        assert!(config.source_maps);
    }
}
