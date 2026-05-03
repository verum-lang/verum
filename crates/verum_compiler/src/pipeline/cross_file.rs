//! Cross-file module resolution + per-module type-check driver.
//!

//! Extracted from `pipeline.rs` (#106 Phase 24). Houses the
//! mount-graph resolution + cross-file context machinery that
//! sits between parse and type-check, plus the actual per-module
//! analysis driver (`analyze_module`).
//!

//! Surface:
//!

//!  * `expand_module` — Pass-2 macro expansion entry; walks
//!  every item, collects macro/meta invocations, dispatches
//!  to `MacroExpander`.
//!  * `file_path_to_module_path` — fs path → dotted module path.
//!  * `load_imported_modules` — recursive loader that resolves
//!  `mount foo.bar` import statements.
//!  * `extract_import_module_path` — `MountTreeKind` → string.
//!  * `resolve_import_path` — relative / cog / super resolution.
//!  * `module_path_to_file_path` — dotted → fs path.
//!  * `register_modules_for_cross_file_resolution` — registers
//!  loaded modules in the session's ModuleRegistry so cross-
//!  file imports + contexts resolve.
//!  * `analyze_module` — the per-module type-check driver;
//!  `&self` post-#101 and runs in parallel via rayon
//!  `par_iter` from the multi-pass / orchestration entry
//!  points.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::{FileId, Module, SourceFile, decl::ItemKind};
use verum_common::{List, Map, Maybe, Shared, Text};
use verum_diagnostics::{DiagnosticBuilder, Severity};
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry};
use verum_types::TypeChecker;

use crate::phases::type_error_to_diagnostic;

use super::{CompilationPipeline, macros::MacroExpander, should_parse_as_script};

impl<'s> CompilationPipeline<'s> {
    /// Expand macros in a module (Pass 2)
    pub(super) fn expand_module(&mut self, path: &Text, module: &mut Module) -> Result<()> {
        debug!("Expanding macros in module: {}", path.as_str());

        // Create a macro expander visitor
        let mut expander = MacroExpander {
            registry: &self.meta_registry,
            context: self.fresh_meta_ctx_with_version_stamp(),
            module_path: path.clone(),
            expansions: List::new(),
        };

        // Walk the AST to collect macro invocations
        for item in &module.items {
            expander.collect_macro_invocations(item);
        }

        debug!(
            "  Found {} macro invocation(s) in {}",
            expander.expansions.len(),
            path.as_str()
        );

        // Execute meta functions and expand macros
        // Clone expansions to avoid borrow conflicts
        let expansions = expander.expansions.clone();
        let mut expansion_errors: List<(Text, anyhow::Error)> = List::new();

        for expansion in &expansions {
            match expander.expand_macro(expansion) {
                Ok(expanded_items) => {
                    debug!(
                        "  Expanded macro '{}' into {} item(s)",
                        expansion.macro_name.as_str(),
                        expanded_items.len()
                    );
                    // Note: In full implementation, we would insert expanded_items
                    // back into the module at the appropriate location
                    // For now, we just log the expansion
                }
                Err(e) => {
                    warn!(
                        "  Failed to expand macro '{}': {}",
                        expansion.macro_name.as_str(),
                        e
                    );
                    // Emit diagnostic
                    let diag = DiagnosticBuilder::error()
                        .message(format!("Macro expansion failed: {}", e))
                        .build();
                    self.session.emit_diagnostic(diag);
                    // Collect the error for propagation
                    expansion_errors.push((expansion.macro_name.clone(), e));
                }
            }
        }

        // Propagate first error if any expansions failed
        if let Some((macro_name, error)) = expansion_errors.into_iter().next() {
            return Err(anyhow::anyhow!(
                "Macro expansion failed for '{}': {}",
                macro_name,
                error
            ));
        }

        Ok(())
    }

    /// Convert a file path to a module path.
    ///

    /// Examples:
    /// - `/Users/.../src/domain/errors.vr` with src_root `/Users/.../src` -> `domain.errors`
    /// - `/Users/.../src/main.vr` with src_root `/Users/.../src` -> `main`
    /// - `/Users/.../src/services/mod.vr` with src_root `/Users/.../src` -> `services`
    ///

    /// File-to-module path mapping: strip src_root prefix, replace `/` with `.`,
    /// strip `.vr` extension, and treat `mod.vr` as the directory module name.
    pub(super) fn file_path_to_module_path(
        &self,
        file_path: &std::path::Path,
        src_root: &std::path::Path,
    ) -> verum_modules::ModulePath {
        use verum_modules::ModulePath;

        // Strip the src_root prefix to get relative path
        let relative_path = file_path.strip_prefix(src_root).unwrap_or(file_path);

        // Remove .vr extension and convert path separators to dots
        let path_str = relative_path
            .with_extension("")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ".");

        // Handle "mod" files - they represent their parent directory
        // e.g., "domain/mod" -> "domain"
        let module_path_str = if path_str.ends_with(".mod") || path_str.ends_with("/mod") {
            path_str
                .trim_end_matches(".mod")
                .trim_end_matches("/mod")
                .to_string()
        } else if path_str == "mod" {
            // Root mod.vr -> empty (root module)
            String::new()
        } else {
            path_str
        };

        if module_path_str.is_empty() {
            ModulePath::root()
        } else {
            ModulePath::from_str(&module_path_str)
        }
    }

    /// Load imported modules into the module registry for single-file checking.
    ///

    /// When checking a single file that has imports (e.g., `import super.contexts.{Database}`),
    /// we need to load and parse the imported modules so that:
    /// 1. Types and functions can be resolved during type checking
    /// 2. Context protocols can be registered for `using [...]` clauses
    ///

    /// This method:
    /// 1. Extracts all import statements from the module
    /// 2. Resolves each import path to a file path
    /// 3. Loads and parses the imported module
    /// 4. Extracts exports and contexts
    /// 5. Registers the module in the session's ModuleRegistry
    /// 6. Iteratively loads that module's imports using a work queue
    ///

    /// Cross-module resolution: imports resolved to file paths, loaded, parsed,
    /// and registered. Uses a work queue for transitive import resolution.
    ///

    /// This function uses an iterative approach with an explicit work queue
    /// to avoid stack overflow when loading deeply nested module hierarchies.
    pub(super) fn load_imported_modules(
        &mut self,
        module: &Module,
        current_module_path: &str,
        src_root: &std::path::Path,
        loaded_paths: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        use verum_modules::{
            ModuleInfo, ModulePath, extract_contexts_from_module, extract_exports_from_module,
        };

        // Work queue for iterative module loading
        // Each item is (module, current_module_path)
        let mut work_queue: Vec<(Module, String)> =
            vec![(module.clone(), current_module_path.to_string())];
        let src_root = src_root.to_path_buf();

        while let Some((current_module, current_path)) = work_queue.pop() {
            // Extract import paths from the module
            for item in &current_module.items {
                if let ItemKind::Mount(import) = &item.kind {
                    // Extract the base module path from the import
                    let import_path = self.extract_import_module_path(&import.tree.kind);

                    // Determine the base directory for the import
                    // Determine if this is a stdlib import by checking if the
                    // first path segment matches a known stdlib top-level module.
                    // "core.*" and "std.*" are canonical prefixes; bare module names
                    // (e.g., "sys.*", "io.*") are shorthand for "core.sys.*", "core.io.*".
                    let first_segment = import_path.split('.').next().unwrap_or("");
                    let is_stdlib_import = matches!(
                        first_segment,
                        "std" | "core"
                        | "sys" | "mem" | "base" | "intrinsics" | "simd" | "math"
                        | "text" | "collections"
                        | "io" | "time" | "sync"
                        | "async" | "runtime"
                        | "term"
                        | "net"
                        | "meta" | "cognitive"
                        // Actic-dual stdlib (Phase 5 E1).
                        | "action"
                    );
                    let (resolved_path, base_dir) = if is_stdlib_import {
                        // Stdlib import - map to core/ directory
                        // Both "sys.intrinsics" and "std.sys.intrinsics" resolve to core/sys/intrinsics.vr
                        let workspace_root = match self.find_workspace_root() {
                            Ok(root) => root,
                            Err(_) => {
                                debug!(
                                    "Could not find workspace root for stdlib import '{}'",
                                    import_path
                                );
                                continue;
                            }
                        };
                        // Check for core/ first (primary), then stdlib/ (legacy)
                        let core_path = workspace_root.join("core");
                        let stdlib_dir = if core_path.exists() {
                            core_path
                        } else {
                            workspace_root.join("stdlib")
                        };
                        if !stdlib_dir.exists() {
                            debug!(
                                "Stdlib directory not found at {:?} or {:?}",
                                workspace_root.join("core"),
                                workspace_root.join("stdlib")
                            );
                            continue;
                        }
                        // Strip stdlib prefixes for file path resolution
                        // - std.sys.intrinsics -> sys.intrinsics
                        // - core.sys.common -> sys.common (since base_dir is already core/)
                        // - sys.intrinsics -> sys.intrinsics (no prefix to strip)
                        let canonical_path = if import_path.starts_with("std.") {
                            import_path[4..].to_string()
                        } else if import_path.starts_with("core.") {
                            // Strip "core." prefix since stdlib_dir already points to core/
                            import_path[5..].to_string()
                        } else {
                            import_path.clone()
                        };
                        (canonical_path, stdlib_dir)
                    } else {
                        // User module import - resolve relative paths
                        let resolved_path = match self
                            .resolve_import_path(&import_path, &current_path)
                        {
                            Ok(path) => path,
                            Err(e) => {
                                debug!("Failed to resolve import path '{}': {}", import_path, e);
                                continue;
                            }
                        };
                        (resolved_path, src_root.clone())
                    };

                    // Skip if already loaded
                    if loaded_paths.contains(&resolved_path) {
                        continue;
                    }
                    loaded_paths.insert(resolved_path.clone());

                    // Convert module path to file path and try to load
                    let module_path = ModulePath::from_str(&resolved_path);
                    let file_path = self.module_path_to_file_path(&module_path, &base_dir);

                    // Try different file locations (file.vr, file/mod.vr)
                    let candidates = vec![file_path.with_extension("vr"), file_path.join("mod.vr")];

                    let mut loaded = false;
                    for candidate in candidates {
                        if candidate.exists() {
                            debug!(
                                "Loading imported module: {} from {:?}",
                                resolved_path, candidate
                            );

                            // Load and parse the module
                            let source_text = match std::fs::read_to_string(&candidate) {
                                Ok(s) => s,
                                Err(e) => {
                                    debug!("Failed to read imported module {:?}: {}", candidate, e);
                                    continue;
                                }
                            };

                            // Load source into session
                            let file_id = match self
                                .session
                                .load_source_string(&source_text, candidate.clone())
                            {
                                Ok(id) => id,
                                Err(e) => {
                                    debug!("Failed to load source for {:?}: {}", candidate, e);
                                    continue;
                                }
                            };

                            // Parse the module
                            let lexer = Lexer::new(&source_text, file_id);
                            let parser = VerumParser::new();
                            let mut imported_module = match parser.parse_module(lexer, file_id) {
                                Ok(m) => m,
                                Err(errors) => {
                                    for error in errors {
                                        debug!(
                                            "Parse error in imported module {:?}: {}",
                                            candidate, error
                                        );
                                    }
                                    continue;
                                }
                            };

                            // Apply @cfg conditional compilation filtering
                            let cfg_evaluator = self.session.cfg_evaluator();
                            imported_module.items =
                                cfg_evaluator.filter_items(&imported_module.items);

                            // Header validation at the
                            // import-on-demand parse path. The
                            // imported module's filesystem path is
                            // `candidate`; pass it to the validator
                            // so cross-file `module foo;` headers
                            // pointing to nothing surface as
                            // warnings here too.
                            let header_warnings =
                                verum_modules::loader::validate_module_headers_against_filesystem(
                                    &candidate,
                                    &imported_module,
                                );
                            for warning in &header_warnings {
                                let diag = DiagnosticBuilder::warning()
                                    .code(warning.code())
                                    .message(warning.message())
                                    .build();
                                self.session.emit_diagnostic(diag);
                            }

                            // Allocate module ID and create ModuleInfo
                            let registry = self.session.module_registry();
                            let module_id = {
                                let reg = registry.write();
                                reg.allocate_id()
                            };

                            let mut module_info = ModuleInfo::new(
                                module_id,
                                module_path.clone(),
                                imported_module.clone(),
                                file_id,
                                source_text.clone().into(),
                            );
                            module_info.header_warnings = header_warnings;

                            // Extract exports from the module's AST
                            match extract_exports_from_module(
                                &imported_module,
                                module_id,
                                &module_path,
                            ) {
                                Ok(export_table) => {
                                    module_info.exports = export_table;
                                    debug!(
                                        "Module '{}': {} exports",
                                        resolved_path,
                                        module_info.exports.len()
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to extract exports from '{}': {:?}",
                                        resolved_path, e
                                    );
                                }
                            }

                            // Extract contexts (protocols and explicit contexts) for cross-file resolution
                            let contexts =
                                extract_contexts_from_module(&imported_module, module_id);
                            let context_count = contexts.len();
                            for ctx in contexts {
                                let name: Text = Text::from(ctx.name.as_str());
                                if !self.collected_contexts.contains(&name) {
                                    self.collected_contexts.push(name);
                                }
                            }
                            if context_count > 0 {
                                debug!(
                                    "Module '{}': {} contexts/protocols",
                                    resolved_path, context_count
                                );
                            }

                            // Register the module in the session's registry
                            {
                                let mut reg = registry.write();
                                reg.register(module_info);
                            }

                            // For user-imported modules (non-stdlib), also store in
                            // project_modules so their items get merged into the main
                            // compilation unit for VBC codegen.
                            if !is_stdlib_import {
                                let module_key = Text::from(resolved_path.as_str());
                                if !self.project_modules.contains_key(&module_key) {
                                    self.project_modules
                                        .insert(module_key, Arc::new(imported_module.clone()));
                                }
                            }

                            // Add to work queue instead of recursive call
                            work_queue.push((imported_module, resolved_path.clone()));

                            loaded = true;
                            break;
                        }
                    }

                    if !loaded {
                        debug!(
                            "Could not find imported module '{}' (tried {:?})",
                            resolved_path, file_path
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Extract the base module path from an import tree.
    pub(super) fn extract_import_module_path(&self, tree: &verum_ast::MountTreeKind) -> String {
        use verum_ast::MountTreeKind;
        use verum_ast::ty::PathSegment;

        let extract_path = |path: &verum_ast::ty::Path| -> String {
            path.segments
                .iter()
                .filter_map(|seg| {
                    match seg {
                        PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                        PathSegment::SelfValue => Some("self".to_string()),
                        PathSegment::Super => Some("super".to_string()),
                        // Relative path marker (leading dot) is treated like "self"
                        PathSegment::Relative => Some("self".to_string()),
                        _ => None,
                    }
                })
                .collect::<Vec<String>>()
                .join(".")
        };

        match tree {
            MountTreeKind::Path(path) => {
                // import module.path.item -> get module.path (parent of item)
                let full = extract_path(path);
                if let Some(dot_pos) = full.rfind('.') {
                    full[..dot_pos].to_string()
                } else {
                    full
                }
            }
            MountTreeKind::Glob(path) => extract_path(path),
            MountTreeKind::Nested { prefix, .. } => extract_path(prefix),
            // #5 / P1.5 — file-relative mount surfaces the
            // file path verbatim. The session loader resolves
            // it to a concrete file before this extractor
            // runs, so the literal path is the cleanest
            // identifier we can return.
            MountTreeKind::File { path, .. } => path.as_str().to_string(),
        }
    }

    /// Resolve relative import paths (self, super) to absolute module paths.
    ///

    /// For modules defined in mod.vr files (e.g., `contexts/mod.vr` with path `contexts`):
    /// - `.database` or `self.database` -> `contexts.database` (child module)
    ///

    /// For regular modules (e.g., `handlers/search.vr` with path `handlers.search`):
    /// - `.other` or `self.other` -> `handlers.other` (sibling module)
    ///

    /// For super imports (supports chained super):
    /// - From `handlers.search`: `super.contexts` -> `contexts` (sibling of parent)
    /// - From `services.package_service`: `super.super.domain` -> `domain` (sibling of services)
    pub(super) fn resolve_import_path(
        &self,
        import_path: &str,
        current_module_path: &str,
    ) -> Result<String, verum_modules::ModuleError> {
        use verum_modules::{ModulePath, resolve_import};

        let current = ModulePath::from_str(current_module_path);
        // Use the standalone resolve_import function which properly handles
        // chained super (e.g., super.super.domain), unlike ModulePath::resolve_import
        let resolved = resolve_import(import_path, &current)?;

        Ok(resolved.to_string())
    }

    /// Convert a module path to a filesystem path (relative to src_root or stdlib_root).
    ///

    /// For stdlib paths (std.*), this strips the "std" prefix:
    /// - std.time -> time/ (when src_root is core/)
    /// - core.base.Maybe -> core/Maybe (when src_root is core/)
    ///

    /// For user paths, this maps directly:
    /// - domain.errors -> domain/errors (when src_root is src/)
    pub(super) fn module_path_to_file_path(
        &self,
        module_path: &verum_modules::ModulePath,
        src_root: &std::path::Path,
    ) -> PathBuf {
        let mut path = src_root.to_path_buf();
        let segments = module_path.segments();

        // Check if this is a stdlib path by looking at the first segment
        let is_stdlib = segments
            .first()
            .map(|s| s.as_str() == "std")
            .unwrap_or(false);

        // For stdlib paths, skip the "std" prefix
        let start_idx = if is_stdlib { 1 } else { 0 };

        for i in start_idx..segments.len() {
            path = path.join(segments[i].as_str());
        }
        path
    }

    /// Register all parsed modules in the session's ModuleRegistry for cross-file resolution.
    ///

    /// This phase (1.5) runs after parsing and before expansion to:
    /// 1. Create ModuleInfo for each parsed module
    /// 2. Extract exports (public types, functions, etc.)
    /// 3. Extract contexts and protocols for cross-file context resolution
    /// 4. Register in session.module_registry
    /// 5. Enable type resolution across files
    ///

    /// Phase 1.5: builds export tables (public types, functions) and extracts
    /// contexts/protocols from each module for cross-file name and context resolution.
    pub(super) fn register_modules_for_cross_file_resolution(&mut self) -> Result<()> {
        self.register_modules_for_cross_file_resolution_filtered(None)
    }

    /// Compute the union of stdlib reachability closures over a set of
    /// user module paths (#109).
    ///
    /// Looks up each user module's AST in `self.modules` and runs
    /// `stdlib_reachability::compute_reachable_stdlib_modules` on it,
    /// merging the per-module closures into a single set. Honours the
    /// `VERUM_FULL_STDLIB=1` env-var opt-out (returns `None` so the
    /// filtered helper falls through to legacy full-registration
    /// behaviour).
    ///
    /// Returns:
    /// * `Some(set)` — caller should pass into the filtered Phase 1.5
    ///   helper. Set contains every stdlib module path needed by *any*
    ///   user source's mount tree plus the implicit prelude.
    /// * `None` — full-stdlib mode is active, or the embedded dep
    ///   graph is unavailable (minimal builds without `core/`).
    ///
    /// The `user_paths` slice should contain only paths the caller
    /// considers part of the user-side compilation unit — typically
    /// the keys of the `sources` map passed to `compile_multi_pass`,
    /// or the single user file in `run_check_only`. Stdlib-loaded
    /// modules in `self.modules` should not be included; they would
    /// trivially resolve their own mounts and inflate the closure.
    pub(super) fn compute_user_reachable_stdlib(
        &self,
        user_paths: &[Text],
    ) -> Option<std::collections::HashSet<String>> {
        if std::env::var("VERUM_FULL_STDLIB").is_ok() {
            return None;
        }
        let mut union: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut any_resolved = false;
        for path in user_paths {
            let Some(module_rc) = self.modules.get(path) else {
                continue;
            };
            let Some(set) =
                crate::stdlib_reachability::compute_reachable_stdlib_modules(module_rc)
            else {
                // Embedded dep graph unavailable — bail to full-registration mode.
                return None;
            };
            any_resolved = true;
            for m in set {
                union.insert(m);
            }
        }
        if any_resolved { Some(union) } else { None }
    }

    /// Filtered variant of [`register_modules_for_cross_file_resolution`].
    ///
    /// When `reachable_stdlib` is `Some(set)`, stdlib modules (paths under
    /// `core.*`) not present in the set are skipped — they will be parsed
    /// and the AST kept in `self.modules`, but no `ModuleInfo` is created
    /// in the session registry. The lazy resolver picks them up on
    /// demand if a downstream phase actually references their symbols.
    ///
    /// When `reachable_stdlib` is `None`, every module is registered
    /// (legacy behaviour — full-stdlib mode, used by `verum audit
    /// --framework-axioms` and any caller that needs every stdlib type
    /// in the registry).
    ///
    /// User modules (anything outside the `core.*` namespace) are
    /// *always* registered regardless of the filter — the filter only
    /// applies to stdlib reachability pruning.
    pub(super) fn register_modules_for_cross_file_resolution_filtered(
        &mut self,
        reachable_stdlib: Option<&std::collections::HashSet<String>>,
    ) -> Result<()> {
        use verum_modules::{
            ModuleInfo, extract_contexts_from_module, extract_exports_from_module,
        };

        let start = Instant::now();
        let mut registered_count = 0;
        let mut skipped_unreachable = 0u64;

        // Get the module registry from session
        let registry = self.session.module_registry();

        // Note: src_root computation was removed since path_text is already in module path format
        // (e.g., "domain.errors") after being processed by check_project/compile_project.

        // Path-sorted iteration: `self.modules` is a HashMap so the raw
        // iteration order leaks the per-process random hasher seed into
        // ModuleId allocation (`registry.allocate_id()` is a counter).
        // Non-deterministic ModuleIds in turn produce non-deterministic
        // FunctionIds at codegen time when imports resolve via the
        // registry. See module.rs:229-231 + audit memo.
        let mut sorted_modules: Vec<(&Text, &std::sync::Arc<Module>)> =
            self.modules.iter().collect();
        sorted_modules.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (path_text, module_rc) in sorted_modules {
            // Reachability filter (#109): when the caller has computed
            // the user's transitive mount closure, skip stdlib modules
            // that are not in it. User modules (paths outside `core.*`)
            // are always registered — they may name symbols of any
            // stdlib module via dotted paths, but the registration of
            // the user module itself is always needed for diagnostics
            // and codegen. The lazy resolver picks up on-demand
            // unreached stdlib modules if a downstream phase walks
            // into them.
            if let Some(reachable) = reachable_stdlib {
                let path_str = path_text.as_str();
                let is_stdlib =
                    path_str == "core" || path_str.starts_with("core.");
                if is_stdlib && !reachable.contains(path_str) {
                    skipped_unreachable += 1;
                    continue;
                }
            }
            // The path_text is already in module path format (e.g., "domain.errors")
            // since it was converted when loading sources in check_project.
            // We just need to create a ModulePath from the string directly.
            let module_path = verum_modules::ModulePath::from_str(path_text.as_str());

            // Allocate a new ModuleId
            let module_id = {
                let reg = registry.write();
                reg.allocate_id()
            };

            // Get file_id from the module's first item span (if any), or use a default
            let file_id = module_rc
                .items
                .first()
                .map(|item| item.span.file_id)
                .unwrap_or_else(|| verum_ast::FileId::new(0));

            // Create ModuleInfo
            let mut module_info = ModuleInfo::new(
                module_id,
                module_path.clone(),
                (**module_rc).clone(),
                file_id,
                Text::new(), // Source text not stored in parsed modules
            );

            // Extract exports from the module's AST
            let module_path_str = module_path.to_string();
            match extract_exports_from_module(module_rc, module_id, &module_path) {
                Ok(mut export_table) => {
                    let exports_before = export_table.len();
                    // Add synthetic exports for stdlib built-in types
                    // These types are implemented natively in Rust but need to be visible
                    // in the type system for imports to work
                    self.add_stdlib_builtin_exports(&mut export_table, module_id, &module_path_str);
                    let exports_after = export_table.len();

                    if exports_after > exports_before {
                        info!(
                            "  Module '{}': added {} synthetic exports ({} -> {})",
                            module_path_str,
                            exports_after - exports_before,
                            exports_before,
                            exports_after
                        );
                    }

                    module_info.exports = export_table;
                    debug!(
                        "  Module '{}': {} exports",
                        module_path_str,
                        module_info.exports.len()
                    );
                }
                Err(e) => {
                    warn!(
                        "  Failed to extract exports from '{}': {:?}",
                        module_path_str, e
                    );
                }
            }

            // Extract contexts (protocols and explicit contexts) for cross-file resolution
            // Extract context/protocol declarations for cross-file `using [...]` resolution.
            let contexts = extract_contexts_from_module(module_rc, module_id);
            let context_count = contexts.len();
            for ctx in contexts {
                // Add to collected contexts for later registration in TypeChecker
                let name: Text = Text::from(ctx.name.as_str());
                if !self.collected_contexts.contains(&name) {
                    self.collected_contexts.push(name);
                }
            }
            if context_count > 0 {
                debug!(
                    "  Module '{}': {} contexts/protocols",
                    module_path_str, context_count
                );
            }

            // Register the module in the session's registry
            {
                let mut reg = registry.write();
                reg.register(module_info);
            }

            registered_count += 1;
        }

        let elapsed = start.elapsed();
        if skipped_unreachable > 0 {
            info!(
                "  Registered {} modules for cross-file resolution in {:.2}ms \
                 ({} contexts; {} stdlib modules pruned by reachability filter)",
                registered_count,
                elapsed.as_secs_f64() * 1000.0,
                self.collected_contexts.len(),
                skipped_unreachable,
            );
        } else {
            info!(
                "  Registered {} modules for cross-file resolution in {:.2}ms ({} contexts)",
                registered_count,
                elapsed.as_secs_f64() * 1000.0,
                self.collected_contexts.len()
            );
        }

        Ok(())
    }

    /// Analyze a module (Pass 3)
    ///

    /// This performs type checking in multiple sub-passes:
    /// 1. Register cross-file contexts (protocols from other modules)
    /// 2. Register all type declarations (to handle forward references)
    /// 3. Check all functions and other items
    ///

    /// Cross-file context resolution enables `using [Context]` across files.
    /// Cross-module name resolution enables imports to resolve types from other modules.
    ///

    /// Per-module semantic analysis. Despite originally being `&mut self`,
    /// the body never writes any field of `Compiler` directly: every
    /// observable mutation flows through `Session::emit_diagnostic`
    /// (lock-free MPMC queue post-#105) or `Session::abort_if_errors`
    /// (atomic counter), and the per-call `TypeChecker` is constructed
    /// fresh and dropped before return. Pre-fix the artificial `&mut`
    /// borrow on `self` serialised the Pass-3 module loop — even on
    /// machines with 16 cores, modules in a large project were analysed
    /// strictly one at a time.
    ///

    /// The `&self` signature (#101) unblocks `module_paths.par_iter()`
    /// at the call site for a 2-4× wall-clock win on multi-module
    /// projects. Parallel correctness rests on three invariants the
    /// audit verified:
    ///

    ///  1. `TypeChecker` instances do not share mutable state — each
    ///  thread owns its checker.
    ///  2. Reads of `self.modules` / `self.collected_contexts` /
    ///  `self.stdlib_metadata` are pure HashMap / List iteration
    ///  with no concurrent writers (the loop runs after all parsing
    ///  passes have completed).
    ///  3. Diagnostic emission and error-counter polling are already
    ///  lock-free atomic operations on `Session`.
    ///

    /// `lazy_resolver` is `Arc<Mutex<dyn LazyModuleResolver>>` so
    /// concurrent late-loads serialise on a single mutex — acceptable
    /// because reachability-narrowing makes late loads rare.
    pub(super) fn analyze_module(&self, path: &Text, module: &Module) -> Result<()> {
        use verum_ast::ItemKind;

        // Type check all items in the module
        // Pass the module registry for cross-file type resolution
        //

        // Mode selection:
        // - NormalBuild (stdlib_metadata = Some): Use pre-compiled stdlib types
        // - StdlibBootstrap (stdlib_metadata = None): Use builtins only
        let mut checker = match &self.stdlib_metadata {
            Some(metadata) => {
                debug!(
                    "Using stdlib metadata for type checking ({} types)",
                    metadata.types.len()
                );
                TypeChecker::new_with_core(metadata.as_ref().clone())
            }
            None => {
                // Compiling stdlib itself - use minimal context
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types (List, Text, Int, Result, Maybe, etc.)
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        checker.register_builtins();

        // Post-cycle-break (2026-04-24): install the SMT backend by hand.
        checker.set_smt_backend(Box::new(
            verum_smt::refinement_backend::RefinementZ3Backend::new(),
        ));

        // Enable orphan-rule checking: without a current cog name,
        // ProtocolChecker::check_orphan_rule silently returns Ok(()).
        // Use the input file's stem as the cog identifier (stable for
        // single-file builds). Manifest-based builds can override this
        // later via TypeChecker::set_current_cog directly.
        let cog_name = self
            .session
            .options()
            .input
            .file_stem()
            .and_then(|s| s.to_str())
            .map(verum_common::Text::from)
            .unwrap_or_else(|| verum_common::Text::from("cog"));
        checker.set_current_cog(cog_name);

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // Configure lazy resolver for on-demand module loading
        // This enables imports to trigger module loading if not already loaded
        checker.set_lazy_resolver(self.lazy_resolver.clone());

        // Sub-pass 0: Register cross-file contexts (protocols and contexts from other modules)
        // This enables `using [Database, Auth]` to work when these are defined elsewhere
        // Register cross-file contexts so `using [Database, Auth]` resolves across files.
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }
        if !self.collected_contexts.is_empty() {
            debug!(
                "  Registered {} cross-file contexts for type checking",
                self.collected_contexts.len()
            );
        }

        // The `path` parameter is already in module path format (e.g., "handlers.users")
        // after being processed by compile_project(). No need to recompute.
        // Module paths use dot-separated format (e.g., "handlers.users").
        let current_module_path_str = path.as_str().to_string();

        // Sub-pass 0: Pre-register all inline modules
        // This enables cross-module imports even when modules are declared after
        // the modules that import from them.
        // Pre-register inline modules for order-independent cross-module imports.
        for item in &module.items {
            if let ItemKind::Module(module_decl) = &item.kind {
                checker.pre_register_module_public(module_decl, "cog");
            }
        }

        // Sub-pass 1: Process imports to register imported types and functions
        // This enables cross-file type resolution for items like `import domain.errors.{RegistryError}`
        // Cross-module name resolution: process imports before type declarations.
        for item in &module.items {
            if let ItemKind::Mount(import) = &item.kind {
                if let Err(type_error) =
                    checker.process_import(import, &current_module_path_str, &registry.read())
                {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // PRE-PASS: Register stdlib module declarations into the type checker.
        // This processes all parsed stdlib modules through multi-pass registration,
        // making stdlib types, protocols, and implement block methods (List.push,
        // Maybe.unwrap, etc.) available for type checking user code.
        //

        // Without this, the TypeChecker only has built-in primitives and cannot
        // resolve stdlib types referenced in user code or cross-module imports.
        // ═══════════════════════════════════════════════════════════════════
        {
            // Collect all stdlib modules (those starting with "core.") to avoid
            // re-registering user modules that are already being analyzed.
            // Sort for deterministic iteration (self.modules is a HashMap):
            // shallower module keys come first so top-level stdlib functions beat
            // nested-module helpers when short names collide.
            let mut stdlib_entries: Vec<_> = self
                .modules
                .iter()
                .filter(|(k, _)| k.as_str().starts_with("core"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            stdlib_entries.sort_by(|(a, _), (b, _)| {
                let depth_a = a.as_str().matches('.').count();
                let depth_b = b.as_str().matches('.').count();
                depth_a
                    .cmp(&depth_b)
                    .then_with(|| a.as_str().cmp(b.as_str()))
            });

            // Preserve the user-file module path so we can restore it after
            // stdlib registration transiently rebinds it per-module.
            let saved_module_path = checker.current_module_path().clone();

            if !stdlib_entries.is_empty() {
                if self.stdlib_metadata.is_none() {
                    debug!(
                        "analyze_module: Registering {} stdlib modules into type checker",
                        stdlib_entries.len()
                    );

                    // Pass S0a: Register all stdlib type names (module-scoped so
                    // fully-qualified `{mod}.{name}` keys are populated for
                    // same-named stdlib types across different modules).
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        checker.register_all_type_names(&stdlib_mod.items);
                    }

                    // Pass S0b: Resolve all stdlib type definitions
                    let mut resolution_stack = List::new();
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Type(type_decl) = &item.kind {
                                if let Err(e) = checker
                                    .resolve_type_definition(type_decl, &mut resolution_stack)
                                {
                                    debug!("Stdlib type resolution error: {:?}", e);
                                }
                            }
                        }
                    }

                    // Pass S1: Register stdlib function signatures
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Function(func) = &item.kind {
                                if !checker.is_function_preregistered(func.name.name.as_str()) {
                                    if let Err(e) = checker.register_function_signature(func) {
                                        debug!("Stdlib function registration error: {:?}", e);
                                    }
                                }
                            }
                        }
                    }

                    // Pass S2: Register stdlib protocols
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Protocol(protocol_decl) = &item.kind {
                                if let Err(e) = checker.register_protocol(protocol_decl) {
                                    debug!("Stdlib protocol registration error: {:?}", e);
                                }
                            }
                        }
                    }
                }

                // Pass S3: ALWAYS register stdlib impl blocks (this registers methods
                // in inherent_methods). This must run even when metadata IS available,
                // because metadata doesn't populate inherent_methods from implement blocks.
                debug!(
                    "analyze_module: Registering stdlib impl blocks ({} modules)",
                    stdlib_entries.len()
                );
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let ItemKind::Impl(impl_decl) = &item.kind {
                            if let Err(e) = checker.register_impl_block(impl_decl) {
                                debug!("Stdlib impl registration error: {:?}", e);
                            }
                        }
                    }
                }

                debug!("analyze_module: Stdlib registration complete");
            }

            // Restore the user-file module path so subsequent passes run in
            // the right resolution scope.
            checker.set_current_module_path(saved_module_path);
        }

        // Signal transition to user code phase
        checker.set_user_code_phase();

        // Sub-pass 2: Register all type declarations first
        // This ensures types are available when checking functions that reference them
        for item in &module.items {
            if let ItemKind::Type(type_decl) = &item.kind {
                if let Err(type_error) = checker.register_type_declaration(type_decl) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 3: Register implement blocks
        // This ensures methods are available for resolution
        for item in &module.items {
            if let ItemKind::Impl(impl_decl) = &item.kind {
                if let Err(type_error) = checker.register_impl_block(impl_decl) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 3.5: Protocol coherence checking (orphan rule, overlap, specialization)
        // Validates that protocol implementations follow coherence rules across the
        // entire dependency graph (user module + stdlib + project modules).
        self.check_protocol_coherence(module)?;

        // Sub-pass 4: Register function signatures (enables forward references)
        // This allows functions to call other functions defined later in the file:
        //  fn main() { helper() } // helper is defined below
        //  fn helper() { ... }
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if let Err(type_error) = checker.register_function_signature(func) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 4.5: Register extern function signatures (FFI)
        // This allows calling FFI functions declared in extern blocks:
        //  @ffi("libSystem.B.dylib")
        //  extern { fn getpid() -> Int; }
        for item in &module.items {
            if let ItemKind::ExternBlock(extern_block) = &item.kind {
                // Register each function in the extern block
                for func in &extern_block.functions {
                    if let Err(type_error) = checker.register_function_signature(func) {
                        let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                        self.session.emit_diagnostic(diag);
                    }
                }
            }
        }

        // Sub-pass 4.6: Pre-register const declarations (enables forward references)
        // Constants defined after functions should still be visible in function bodies.
        for item in &module.items {
            if let ItemKind::Const(const_decl) = &item.kind {
                checker.pre_register_const(const_decl);
            }
        }

        // Enable lenient context validation for files with @test annotations.
        let has_test_annotation = module
            .items
            .iter()
            .any(|item| item.attributes.iter().any(|attr| attr.is_named("test")))
            || module
                .items
                .first()
                .and_then(|item| self.session.get_source(item.span.file_id))
                .map(|sf| {
                    sf.source.as_str().lines().take(10).any(|line| {
                        let trimmed = line.trim();
                        trimmed.starts_with("// @test:") || trimmed.starts_with("// @test ")
                    })
                })
                .unwrap_or(false);
        if has_test_annotation {
            checker.context_resolver_mut().set_lenient_contexts(true);
        }

        // Sub-pass 5: Check all items (functions, impls, etc.)
        for item in &module.items {
            if let Err(type_error) = checker.check_item(item) {
                let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Abort if errors occurred
        self.session.abort_if_errors()?;

        Ok(())
    }

    // ==================== COMPILATION PHASES ====================

    /// Phase 1: Load source file
    ///

    /// If input is a directory, finds main.vr inside it.
    /// If input is a file, loads it directly.
    pub fn phase_load_source(&mut self) -> Result<FileId> {
        let input = self.session.options().input.clone();
        debug!("Loading source: {}", input.display());

        // If input is a directory, look for main.vr inside
        let actual_file = if input.is_dir() {
            let main_file = input.join("main.vr");
            if main_file.exists() {
                main_file
            } else {
                // Try to find any .vr file
                let files = self.session.discover_project_files()?;
                files.into_iter().next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "No .vr files found in directory: {}. \
                         For single-file compilation, specify the .vr file directly.",
                        input.display()
                    )
                })?
            }
        } else {
            input
        };

        let file_id = self
            .session
            .load_file(&actual_file)
            .with_context(|| format!("Failed to load source file: {}", actual_file.display()))?;

        Ok(file_id)
    }

    /// Phase 2: Lexing and parsing
    pub fn phase_parse(&mut self, file_id: FileId) -> Result<Module> {
        debug!("Parsing file {:?}", file_id);

        // Check cache first
        if let Some(cached) = self.session.get_module(file_id) {
            debug!("Using cached module");
            // Still need to clone here since we can't return Shared as Module
            return Ok((*cached).clone());
        }

        let source: Shared<SourceFile> = self
            .session
            .get_source(file_id)
            .context("Source file not found")?;

        // Lexing + Parsing (combined via parser)
        let start = Instant::now();

        // Decide library-mode vs script-mode parsing based on shebang
        // autodetection or the entry-source script_mode flag. See
        // `should_parse_as_script` for the full rule.
        let script = should_parse_as_script(
            source.source.as_str(),
            self.session.options(),
            source.path.as_deref(),
        );

        let parser = VerumParser::new();
        let parse_result = if script {
            parser.parse_module_script_str(source.source.as_str(), file_id)
        } else {
            let lexer = Lexer::new(&source.source, file_id);
            parser.parse_module(lexer, file_id)
        };
        let mut module = parse_result.map_err(|errors| {
            // Convert parser errors to diagnostics
            let error_count = errors.len();
            for error in errors.iter() {
                let mut builder =
                    DiagnosticBuilder::error().message(format!("Parse error: {}", error));
                // Include error code if present (e.g., M401 for splice outside quote)
                if let Some(ref code) = error.code {
                    builder = builder.code(code.clone());
                }
                self.session.emit_diagnostic(builder.build());
            }
            // Display diagnostics before returning error
            let _ = self.session.display_diagnostics();
            anyhow::anyhow!("Parsing failed with {} error(s)", error_count)
        })?;

        // Apply @cfg conditional compilation filtering
        let cfg_evaluator = self.session.cfg_evaluator();
        let original_count = module.items.len();
        module.items = cfg_evaluator.filter_items(&module.items);
        let filtered_count = original_count - module.items.len();

        let parse_time = start.elapsed();
        debug!(
            "Parsed module with {} items ({} filtered by @cfg) in {:.2}ms",
            module.items.len(),
            filtered_count,
            parse_time.as_millis()
        );

        // MOD-MED-1 — validate module headers against
        // the filesystem. Surfaces dangling forward declarations
        // (`module foo;` with no source file) and inline-vs-
        // filesystem overlaps (`module foo { … }` alongside an
        // existing `foo/` directory). Non-blocking warnings — the
        // user fixes the dangling decl and re-runs.
        if let Some(ref file_path) = source.path {
            let warnings = verum_modules::loader::validate_module_headers_against_filesystem(
                file_path, &module,
            );
            for warning in warnings {
                let diag = DiagnosticBuilder::warning()
                    .code(warning.code())
                    .message(warning.message())
                    .build();
                self.session.emit_diagnostic(diag);
            }
        }

        // Record parsing metrics
        self.session.record_phase_metrics("Parsing", parse_time, 0);

        // Honour `--emit-ast`: serialise the freshly parsed module to
        // a sidecar `.ast.json` next to the input source. The flag
        // was a config field with no readers — it has been declared
        // and defaulted on `CompilerOptions` for a long while, but
        // no compilation phase emitted anything when it was set, so
        // the documented "Emit AST in JSON format" contract was a
        // no-op. We mirror the `emit_types`/`emit_vbc` pattern of
        // best-effort write + debug log on failure (non-fatal).
        if self.session.options().emit_ast {
            let ast_path = self.session.options().input.with_extension("ast.json");
            match serde_json::to_vec_pretty(&module) {
                Ok(data) => match std::fs::write(&ast_path, &data) {
                    Ok(()) => info!(
                        "Exported AST: {} ({} bytes)",
                        ast_path.display(),
                        data.len()
                    ),
                    Err(e) => debug!("Failed to write AST: {}", e),
                },
                Err(e) => debug!("Failed to serialise AST: {}", e),
            }
        }

        // Cache the module (session still uses its own caching mechanism)
        self.session.cache_module(file_id, module.clone());

        // Abort if errors
        self.session.abort_if_errors()?;

        Ok(module)
    }

    /// Public wrapper for type checking phase.
    ///

    /// Used by the `verum analyze` command to run type checking before
    /// CBGR analysis. Errors are returned but non-fatal for analysis purposes.
    pub fn run_type_check_phase(&mut self, module: &Module) -> Result<()> {
        self.phase_type_check(module)
    }

    /// Public wrapper for building a function's control flow graph.
    ///

    /// Used by the `verum analyze` command to run escape analysis on individual
    /// functions without going through the full compilation pipeline.
    pub fn build_function_cfg_public(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> verum_cbgr::analysis::ControlFlowGraph {
        self.build_function_cfg(func)
    }
}
