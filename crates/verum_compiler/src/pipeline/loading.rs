//! Stdlib + project + cog module discovery, parsing, and loading.
//!
//! Extracted from `pipeline.rs` (#106 Phase 13). Houses the
//! module-graph plumbing that populates `self.modules` /
//! `self.project_modules` before semantic analysis runs.
//!
//! Methods:
//!
//!   * `load_stdlib_modules` — primary entry; two-tier-cached
//!     stdlib loader (registry cache → module cache → cold parse).
//!     Called once per `Compiler` lifecycle before any user code.
//!   * `load_external_cog_modules` — pulls modules from
//!     externally-registered cogs (verum-add deps,
//!     `dependencies` in script-mode frontmatter).
//!   * `load_project_modules` — discovers + parses sibling .vr
//!     files in multi-file projects (cross-file `mount foo.bar`
//!     resolution).
//!   * `discover_vr_files_recursive` — directory walker.
//!   * `extract_all_exports` — module → ExportTable conversion.
//!   * `discover_stdlib_files` + `discover_stdlib_files_recursive`
//!     — embedded-stdlib unpacking helpers.
//!   * `parse_stdlib_module` — single-file stdlib parser
//!     (with diagnostic emission).
//!   * `parse_and_register` — atomic parse + register-with-session
//!     for general-purpose use.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::{List, Map, Text};
use verum_modules::ModulePath;

use super::{
    BuildMode, CachedStdlibModules, CompilationPipeline, compute_stdlib_content_hash,
    global_stdlib_cache, global_stdlib_registry_cache, try_load_registry_from_disk,
};

impl<'s> CompilationPipeline<'s> {
    // ========================================================================
    // STDLIB MODULE LOADING
    // ========================================================================

    /// Load and parse all stdlib modules into self.modules.
    ///
    /// This enables cross-file imports from std.* modules.
    /// Must be called before processing user modules.
    ///
    /// # Performance Optimization (Registry Caching)
    ///
    /// This function implements a two-level caching strategy:
    /// 1. **Registry cache (FAST PATH)**: If we have a fully-populated registry
    ///    cached, we deep_clone it (~1ms) instead of re-registering all modules
    /// 2. **Module cache (FALLBACK)**: If no registry cache, we use cached parsed
    ///    modules to avoid re-parsing, then populate and cache the registry
    ///
    /// The registry cache provides ~500ms speedup per compilation by avoiding:
    /// - Module registration in ModuleRegistry (~166 modules)
    /// - Export extraction from each module
    /// - Glob re-export resolution (iterative)
    ///
    /// Loads stdlib with two-tier caching: (1) registry cache from prior compilation,
    /// (2) parsed module cache to avoid re-parsing ~166 stdlib modules.
    fn load_stdlib_modules(&mut self) -> Result<()> {
        let start = Instant::now();
        debug!("load_stdlib_modules called");

        // FAST PATH: Try to use cached fully-populated registry
        // This is the key optimization: deep_clone a cached registry (~1ms)
        // instead of re-registering ~166 modules (~500ms).
        // NOTE: deep_clone shares ModuleInfo via Arc (Shared) and only clones
        // the HashMap structure. Further optimization would require wrapping the
        // entire registry in Arc and using copy-on-write for mutations.
        {
            let cache = global_stdlib_registry_cache();
            let guard = cache.read().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib registry cache RwLock poisoned, recovering");
                poisoned.into_inner()
            });
            if let Some(ref cached_registry) = *guard {
                let cloned = cached_registry.deep_clone();
                let module_count = cloned.len();

                // Replace the session's registry with the cloned one
                {
                    let registry_shared = self.session.module_registry();
                    let mut session_registry = registry_shared.write();
                    *session_registry = cloned;
                }

                // Also populate the local modules map from the registry.
                // Sort by module path before iterating: ModuleRegistry.modules
                // is Map (HashMap-backed via verum_common::Map), so raw
                // iteration order leaks Rust's per-process random hasher
                // seed into downstream codegen, producing non-deterministic
                // bytecode (see #143).  Path-sorted iteration is stable
                // across runs.
                let session_registry = self.session.module_registry();
                let reg = session_registry.read();
                let mut entries: Vec<(String, Arc<verum_ast::Module>)> = reg
                    .all_modules()
                    .map(|(_id, info)| (info.path.to_string(), Arc::new(info.ast.clone())))
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                for (path_str, ast_arc) in entries {
                    self.modules.insert(Text::from(path_str), ast_arc);
                }
                drop(reg);

                let elapsed = start.elapsed();
                info!(
                    "Loaded {} stdlib module(s) from registry cache in {:.2}ms",
                    module_count,
                    elapsed.as_secs_f64() * 1000.0
                );
                return Ok(());
            }
        }

        // SLOW PATH: No in-memory registry cache, load from source
        debug!("No in-memory registry cache, loading stdlib from source");

        // Determine stdlib path based on build mode:
        // - StdlibBootstrap mode: Use the configured stdlib_path directly
        // - Normal mode: Find workspace root and look for core/
        //
        // ARCHITECTURE NOTE: The embedded stdlib (embedded_stdlib.rs) contains all
        // core/*.vr sources compressed in the binary. Currently we still resolve from
        // the filesystem for dev mode (workspace core/). In production builds, the
        // embedded archive can be used instead of filesystem by switching the source.
        // The embedded archive API: crate::embedded_stdlib::get_embedded_stdlib()
        let (stdlib_path, workspace_root_for_cache) = match &self.build_mode {
            BuildMode::StdlibBootstrap { config } => {
                debug!("StdlibBootstrap mode: using configured path {:?}", config.stdlib_path);
                (config.stdlib_path.clone(), None)
            }
            BuildMode::Normal => {
                // Stdlib (core cog) resolution:
                //   1. VERUM_STDLIB_PATH env var (explicit override)
                //   2. Workspace core/ directory (dev mode — binary in target/)
                //
                // NOTE: ~/.verum/core/ resolution commented out — embedded stdlib
                // replaces filesystem-based production installs.
                let stdlib_candidates: Vec<(PathBuf, Option<PathBuf>)> = {
                    let mut candidates = Vec::new();

                    // 1. Explicit override
                    if let Ok(path) = std::env::var("VERUM_STDLIB_PATH") {
                        let p = PathBuf::from(&path);
                        if p.exists() {
                            candidates.push((p, None));
                        }
                    }

                    // 2. Workspace root (dev mode).
                    //
                    // T6.0.2 — only accept the candidate when
                    // `core/mod.vr` is present. A bare `core/`
                    // directory (e.g. inside a user cog that
                    // happened to scaffold the namespace but
                    // never populated it) silently shadowed the
                    // embedded stdlib pre-fix; cogs whose `core/`
                    // is empty (or absent) now fall through to
                    // the embedded path correctly.
                    if let Ok(workspace_root) = self.find_workspace_root() {
                        let core_path = workspace_root.join("core");
                        let mod_file = core_path.join("mod.vr");
                        if mod_file.is_file() {
                            candidates.push((core_path, Some(workspace_root)));
                        }
                    }

                    // 3. ~/.verum/core/ — DISABLED: embedded stdlib replaces this
                    // if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
                    //     let verum_core = PathBuf::from(&home).join(".verum").join("core");
                    //     if verum_core.exists() {
                    //         candidates.push((verum_core, None));
                    //     }
                    // }

                    candidates
                };

                match stdlib_candidates.into_iter().next() {
                    Some((stdlib, workspace)) => {
                        debug!("Core stdlib resolved: {:?}", stdlib);
                        (stdlib, workspace)
                    }
                    None => {
                        debug!("No core stdlib found on filesystem");
                        return Ok(());
                    }
                }
            }
        };

        if !stdlib_path.exists() {
            debug!("Stdlib directory not found at {:?}, skipping", stdlib_path);
            return Ok(());
        }

        // DISK CACHE: Persistent registry cache for cross-process reuse.
        // Always enabled — disk cache avoids re-parsing 171 stdlib .vr files.
        // Disable with VERUM_NO_DISK_CACHE=1 if needed.
        let content_hash = if std::env::var("VERUM_NO_DISK_CACHE").is_ok() {
            String::new() // Explicitly disabled
        } else {
            compute_stdlib_content_hash(&stdlib_path)
        };
        if !content_hash.is_empty() {
            if let Some(ref ws_root) = workspace_root_for_cache {
                if let Some(disk_registry) = try_load_registry_from_disk(ws_root, &content_hash) {
                    let module_count = disk_registry.len();

                    // Populate the session's registry
                    {
                        let registry_shared = self.session.module_registry();
                        let mut session_registry = registry_shared.write();
                        *session_registry = disk_registry.deep_clone();
                    }

                    // Populate local modules map (path-sorted — see #143).
                    let session_registry = self.session.module_registry();
                    let reg = session_registry.read();
                    let mut entries: Vec<(String, Arc<verum_ast::Module>)> = reg
                        .all_modules()
                        .map(|(_id, info)| (info.path.to_string(), Arc::new(info.ast.clone())))
                        .collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    for (path_str, ast_arc) in entries {
                        self.modules.insert(Text::from(path_str), ast_arc);
                    }
                    drop(reg);

                    // Also populate in-memory caches for subsequent pipeline instances
                    {
                        let cache = global_stdlib_registry_cache();
                        let mut guard = cache.write().unwrap_or_else(|poisoned| {
                            tracing::warn!("stdlib registry cache RwLock poisoned during write, recovering");
                            poisoned.into_inner()
                        });
                        if guard.is_none() {
                            *guard = Some(disk_registry);
                        }
                    }

                    let elapsed = start.elapsed();
                    info!(
                        "Loaded {} stdlib module(s) from disk cache in {:.2}ms",
                        module_count,
                        elapsed.as_secs_f64() * 1000.0
                    );
                    return Ok(());
                }
            }
        }

        // FULL LOAD: No cache available, parse everything from source
        debug!("No disk cache, performing full stdlib load");

        // Try to use the process-level parsed stdlib cache.
        // This avoids re-parsing 166+ .vr files for every pipeline instance.
        let cached_entries = {
            let cache = global_stdlib_cache();
            let guard = cache.read().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib cache RwLock poisoned, recovering");
                poisoned.into_inner()
            });
            guard.as_ref().map(|c| c.entries.clone())
        };

        let parsed_modules: Vec<(Text, Module, Text)> = if let Some(entries) = cached_entries {
            debug!("Using cached stdlib modules ({} entries)", entries.len());
            entries
        } else {
            // First time: discover, read, and parse all stdlib files
            let stdlib_files = self.discover_stdlib_files(&stdlib_path)?;
            if stdlib_files.is_empty() {
                debug!("No .vr files found in core/");
                return Ok(());
            }

            info!("Parsing {} stdlib module(s) (first load, parallel)...", stdlib_files.len());

            // Phase 1: Read all files and compute module paths (parallelizable I/O)
            use rayon::prelude::*;
            let file_data: Vec<(Text, String, PathBuf)> = stdlib_files
                .par_iter()
                .filter_map(|file_path| {
                    let module_path_str = {
                        // Compute module path from file path
                        let rel = file_path.strip_prefix(&stdlib_path).ok()?;
                        let mut parts: Vec<String> = Vec::new();
                        parts.push("core".to_string());
                        for component in rel.components() {
                            if let std::path::Component::Normal(os_str) = component {
                                let s = os_str.to_str()?;
                                if s.ends_with(".vr") {
                                    parts.push(s.trim_end_matches(".vr").to_string());
                                } else {
                                    parts.push(s.to_string());
                                }
                            }
                        }
                        // Handle "mod" files: mod.vr represents its parent directory.
                        // e.g., "core.intrinsics.mod" -> "core.intrinsics"
                        let joined = parts.join(".");
                        if joined.ends_with(".mod") {
                            Text::from(joined.trim_end_matches(".mod"))
                        } else {
                            Text::from(joined)
                        }
                    };
                    let source_text = std::fs::read_to_string(file_path).ok()?;
                    Some((module_path_str, source_text, file_path.clone()))
                })
                .collect();

            // Sort by module path to ensure deterministic registration order.
            // rayon's par_iter() returns results in arbitrary order depending on
            // thread scheduling, which caused intermittent type resolution failures
            // when variant constructors or method tables were populated in different
            // orders across runs.
            let mut file_data = file_data;
            file_data.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

            // Phase 2: Parse modules (must be sequential due to shared parser state)
            let mut entries = Vec::with_capacity(file_data.len());
            for (module_path_str, source_text, file_path) in &file_data {
                match self.parse_stdlib_module(module_path_str, &Text::from(source_text.clone()), file_path) {
                    Ok(module) => {
                        entries.push((module_path_str.clone(), module, Text::from(source_text.clone())));
                    }
                    Err(e) => {
                        debug!("Failed to parse stdlib module {}: {:?}", module_path_str.as_str(), e);
                    }
                }
            }

            // Store in global cache for future pipeline instances
            {
                let cache = global_stdlib_cache();
                let mut guard = cache.write().unwrap_or_else(|poisoned| {
                    tracing::warn!("stdlib cache RwLock poisoned during write, recovering");
                    poisoned.into_inner()
                });
                *guard = Some(CachedStdlibModules {
                    entries: entries.clone(),
                });
            }

            entries
        };

        // Register all parsed modules in the session's ModuleRegistry and local modules map
        for (module_path_str, module, source_text) in &parsed_modules {
            if self.modules.contains_key(module_path_str) {
                continue;
            }

            let item_count = module.items.len();
            let module_path = ModulePath::from_str(module_path_str.as_str());
            let module_registry = self.session.module_registry();
            let module_id = module_registry.read().allocate_id();

            let file_id = module.items.first()
                .map(|item| item.span.file_id)
                .unwrap_or(FileId::new(0));

            let mut module_info = ModuleInfo::new(
                module_id,
                module_path.clone(),
                module.clone(),
                file_id,
                source_text.clone(),
            );

            match extract_exports_from_module(module, module_id, &module_path) {
                Ok(export_table) => {
                    let export_count = export_table.len();
                    module_info.exports = export_table;
                    debug!("{} has {} items, {} exports", module_path_str.as_str(), item_count, export_count);
                }
                Err(e) => {
                    debug!("Failed to extract exports from {}: {:?}", module_path_str.as_str(), e);
                }
            }

            module_registry.write().register(module_info);
            self.register_inline_modules(module, &module_path, file_id);
            self.modules.insert(module_path_str.clone(), Arc::new(module.clone()));
        }

        // After all modules are loaded, resolve re-exports in two phases:
        //
        // Phase 1: Resolve ExportKind for specific item re-exports FIRST
        // This handles `public import path.{Item1, Item2}` where the kind was
        // defaulted to Type during initial extraction. Now we look up the actual
        // kind from the source module (e.g., Some is a Function, not a Type).
        //
        // Phase 2: Resolve glob re-exports SECOND
        // This processes `public import path.*` statements, copying exports from
        // source modules. By running this AFTER specific kind resolution, the
        // glob copies will get the correct ExportKind values.
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();

            // Phase 1: Specific item re-exports (updates ExportKind)
            match resolve_specific_reexport_kinds(&mut guard) {
                Ok(updated_count) => {
                    debug!("Updated {} re-export kinds", updated_count);
                }
                Err(e) => {
                    debug!("Failed to resolve re-export kinds: {:?}", e);
                }
            }

            // Phase 2: Glob re-exports (copies exports with correct kinds)
            // Run in a loop to handle transitive/chained glob re-exports
            // (e.g., runtime/time.vr -> runtime/mod.vr -> mod.vr)
            let mut iteration = 0;
            loop {
                iteration += 1;
                match resolve_glob_reexports(&mut guard) {
                    Ok(resolved_count) => {
                        debug!("Glob re-export iteration {}: resolved {} exports", iteration, resolved_count);
                        if resolved_count == 0 || iteration >= 10 {
                            break;
                        }
                    }
                    Err(e) => {
                        debug!("Failed to resolve glob re-exports: {:?}", e);
                        break;
                    }
                }
            }
        }

        let elapsed = start.elapsed();
        let stdlib_count = self
            .modules
            .iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .count();
        let registry_count = self.session.module_registry().read().len();
        info!(
            "Loaded {} stdlib module(s) ({} registered) in {:.2}ms",
            stdlib_count,
            registry_count,
            elapsed.as_secs_f64() * 1000.0
        );

        // Cache the fully-populated registry for future pipeline instances.
        // This is the key optimization: subsequent loads will deep_clone this
        // cached registry instead of re-registering all modules.
        {
            let cache = global_stdlib_registry_cache();
            let mut guard = cache.write().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib registry cache RwLock poisoned during write, recovering");
                poisoned.into_inner()
            });
            if guard.is_none() {
                let registry = self.session.module_registry().read().clone();
                info!(
                    "Caching stdlib registry with {} modules for future reuse",
                    registry.len()
                );
                *guard = Some(registry);
            }
        }

        // Persist registry to disk for cross-process reuse (release builds or opt-in).
        if !content_hash.is_empty() {
            if let Some(ref ws_root) = workspace_root_for_cache {
                let registry = self.session.module_registry().read().clone();
                save_registry_to_disk(ws_root, &registry, &content_hash);
            }
        }

        Ok(())
    }

    /// Load project modules from the input file's directory.
    ///
    /// When the input file resides in a directory containing a `mod.vr` file,
    /// that directory is treated as a multi-file project. All sibling `.vr` files
    /// are discovered, parsed, and registered as modules, enabling cross-file
    /// `mount` imports (e.g., `mount bootstrap.token.*`).
    /// Walk every cog registered in the session's `CogResolver` and
    /// load its modules into the session's module registry. Symmetric
    /// with `load_project_modules` but sourced from the resolver
    /// (script-mode `dependencies = [...]`, `verum add`, etc.) instead
    /// of the manifest's project tree.
    ///
    /// Each cog's filesystem root is walked recursively; every `.vr`
    /// file is parsed in library mode and registered under the dotted
    /// path `<cog_name>.<relative_path>` (with `mod.vr` collapsing to
    /// the directory name). Subsequent `mount cog_name.foo` from the
    /// entry source resolves through the same registry as workspace
    /// modules — the consumer can't tell the difference.
    ///
    /// No-op when no resolver is installed (project mode, plain
    /// scripts without `dependencies = [...]`).
    fn load_external_cog_modules(&mut self) -> Result<()> {
        let cog_locations: Vec<(String, PathBuf)> = match self.session.cog_resolver() {
            Some(resolver) => resolver
                .cog_names()
                .into_iter()
                .filter_map(|name| {
                    resolver
                        .get_cog_root(name.as_str())
                        .map(|root| (name.as_str().to_string(), root.clone()))
                })
                .collect(),
            None => return Ok(()),
        };

        for (cog_name, cog_root) in cog_locations {
            let canonical_root = cog_root.canonicalize().unwrap_or(cog_root.clone());
            let mut cog_files: Vec<PathBuf> = Vec::new();
            // Reuse the same recursive walker as project modules — the
            // skip-list (hidden dirs, target/, node_modules/, test_*)
            // applies identically to external cogs.
            Self::discover_vr_files_recursive(&canonical_root, &None, &mut cog_files);
            if cog_files.is_empty() {
                debug!(
                    "External cog '{}' at {} has no .vr files",
                    cog_name,
                    canonical_root.display()
                );
                continue;
            }

            info!(
                "Loading {} module(s) from external cog '{}' at {}",
                cog_files.len(),
                cog_name,
                canonical_root.display()
            );

            for file_path in &cog_files {
                let stem =
                    file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                let module_path_str = {
                    let rel = file_path
                        .parent()
                        .and_then(|p| p.strip_prefix(&canonical_root).ok())
                        .unwrap_or(std::path::Path::new(""));
                    let mut parts = vec![cog_name.clone()];
                    for component in rel.components() {
                        if let std::path::Component::Normal(seg) = component {
                            if let Some(s) = seg.to_str() {
                                parts.push(s.to_string());
                            }
                        }
                    }
                    if stem != "mod" {
                        parts.push(stem.to_string());
                    }
                    Text::from(parts.join("."))
                };

                if self.modules.contains_key(&module_path_str) {
                    continue;
                }

                let source_text = match std::fs::read_to_string(file_path) {
                    Ok(s) => s,
                    Err(e) => {
                        debug!(
                            "Failed to read external cog module {}: {:?}",
                            module_path_str.as_str(),
                            e
                        );
                        continue;
                    }
                };

                match self.parse_stdlib_module(
                    &module_path_str,
                    &Text::from(source_text.clone()),
                    file_path,
                ) {
                    Ok(module) => {
                        let module_path = ModulePath::from_str(module_path_str.as_str());
                        let module_registry = self.session.module_registry();
                        let module_id = module_registry.read().allocate_id();
                        let file_id = module
                            .items
                            .first()
                            .map(|item| item.span.file_id)
                            .unwrap_or(FileId::new(0));

                        let mut module_info = ModuleInfo::new(
                            module_id,
                            module_path.clone(),
                            module.clone(),
                            file_id,
                            Text::from(source_text),
                        );

                        // External-cog modules behave like project
                        // modules from the consumer's perspective —
                        // export ALL items, not just `pub` ones,
                        // so the script can reach internals it
                        // explicitly mounts.
                        let export_table =
                            Self::extract_all_exports(&module, module_id, &module_path);
                        module_info.exports = export_table;

                        module_registry.write().register(module_info);
                        self.register_inline_modules(&module, &module_path, file_id);
                        let module_rc = Arc::new(module);
                        self.modules.insert(module_path_str.clone(), module_rc.clone());
                        self.project_modules
                            .insert(module_path_str.clone(), module_rc);
                        debug!(
                            "Loaded external-cog module: {}",
                            module_path_str.as_str()
                        );
                    }
                    Err(e) => {
                        debug!(
                            "Failed to parse external-cog module {}: {:?}",
                            module_path_str.as_str(),
                            e
                        );
                    }
                }
            }
        }

        // Resolve re-exports across the registered modules (mirrors
        // the same step at the end of load_project_modules).
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let _ = resolve_glob_reexports(&mut guard);
        }

        Ok(())
    }

    fn load_project_modules(&mut self) -> Result<()> {
        let input_path = self.session.options().input.clone();
        let input_dir = match input_path.parent() {
            Some(dir) if dir.as_os_str().is_empty() => std::env::current_dir()?,
            Some(dir) => dir.to_path_buf(),
            None => return Ok(()),
        };

        // Canonicalize for reliable path comparison
        let input_dir = input_dir.canonicalize().unwrap_or(input_dir);

        // Only treat as a project if there's a mod.vr in the directory
        let mod_file = input_dir.join("mod.vr");
        if !mod_file.exists() {
            return Ok(());
        }

        // Determine the project module prefix from the directory name
        let project_prefix = input_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        info!("Detected multi-file project '{}' in {}", project_prefix, input_dir.display());

        // Discover all .vr files in the project directory (recursive)
        let canonical_input = input_path.canonicalize().ok();
        let mut project_files: Vec<PathBuf> = Vec::new();
        Self::discover_vr_files_recursive(&input_dir, &canonical_input, &mut project_files);

        if project_files.is_empty() {
            return Ok(());
        }

        info!("Loading {} project module(s)", project_files.len());

        // Track which module_path_str each filesystem source produced so a
        // subsequent collision (two files mapping to the same module path —
        // typically `foo.vr` Rule 2 vs `foo/mod.vr` Rule 4) can surface as a
        // hard diagnostic instead of silently skipping the second loader. The
        // first source wins; the loser's declarations would otherwise be
        // unreachable through any `mount` and the user sees `unbound
        // variable` errors at use sites with no hint about the cause.
        let mut module_path_to_source: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();

        // Parse and register each project module
        for file_path in &project_files {
            let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
            // Build dotted module path from relative directory components
            // e.g. project_dir/sub/foo.vr -> "project.sub.foo"
            //      project_dir/sub/mod.vr -> "project.sub"
            //      project_dir/foo.vr     -> "project.foo"
            //      project_dir/mod.vr     -> "project"
            let module_path_str = {
                let rel = file_path.parent()
                    .and_then(|p| p.strip_prefix(&input_dir).ok())
                    .unwrap_or(std::path::Path::new(""));
                let mut parts = vec![project_prefix.clone()];
                for component in rel.components() {
                    if let std::path::Component::Normal(seg) = component {
                        if let Some(s) = seg.to_str() {
                            parts.push(s.to_string());
                        }
                    }
                }
                if stem != "mod" {
                    parts.push(stem.to_string());
                }
                Text::from(parts.join("."))
            };

            // Detect E_MODULE_PATH_COLLISION: two files reach the same
            // dotted module path. The most-common shape is `foo.vr` (Rule 2,
            // file form) AND `foo/mod.vr` (Rule 4, directory form) both
            // declaring module `<project>.foo`.  Surface this as a hard
            // diagnostic with both sources cited, and skip the loser so the
            // rest of the project can keep building (the user gets a
            // clear actionable message instead of silent loss).
            if let Some(prev_source) = module_path_to_source.get(module_path_str.as_str()) {
                eprintln!(
                    "error<E_MODULE_PATH_COLLISION>: module path '{}' resolves to two source files",
                    module_path_str.as_str(),
                );
                eprintln!("  using:    {}", prev_source.display());
                eprintln!("  ignoring: {}", file_path.display());
                eprintln!(
                    "  hint: pick exactly one of the file form (`<name>.vr`) \
                     or the directory form (`<name>/mod.vr`); having both makes \
                     declarations in the loser invisible at use sites and is \
                     classified as `E_MODULE_PATH_COLLISION`"
                );
                continue;
            }
            module_path_to_source.insert(module_path_str.as_str().to_string(), file_path.clone());

            if self.modules.contains_key(&module_path_str) {
                continue;
            }

            let source_text = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to read project module {}: {:?}", module_path_str.as_str(), e);
                    continue;
                }
            };

            match self.parse_stdlib_module(&module_path_str, &Text::from(source_text.clone()), file_path) {
                Ok(module) => {
                    let module_path = ModulePath::from_str(module_path_str.as_str());
                    let module_registry = self.session.module_registry();
                    let module_id = module_registry.read().allocate_id();

                    let file_id = module.items.first()
                        .map(|item| item.span.file_id)
                        .unwrap_or(FileId::new(0));

                    let mut module_info = ModuleInfo::new(
                        module_id,
                        module_path.clone(),
                        module.clone(),
                        file_id,
                        Text::from(source_text),
                    );

                    // For project modules, export ALL items (not just public ones)
                    // since they share the same project context.
                    let export_table = Self::extract_all_exports(&module, module_id, &module_path);
                    module_info.exports = export_table;

                    // MOD-MED-1 — validate `module foo;`
                    // headers against the filesystem. Emits warnings
                    // for dangling forward-decls
                    // (E_MODULE_HEADER_FORWARD_DECL_NO_SOURCE) and
                    // inline-vs-filesystem overlaps
                    // (E_MODULE_INLINE_FILESYSTEM_OVERLAP) so users
                    // see header inconsistencies without breaking
                    // the build.
                    let header_warnings =
                        verum_modules::loader::validate_module_headers_against_filesystem(
                            file_path,
                            &module,
                        );
                    for warning in &header_warnings {
                        let diag = verum_diagnostics::DiagnosticBuilder::warning()
                            .code(warning.code())
                            .message(warning.message())
                            .build();
                        self.session.emit_diagnostic(diag);
                    }
                    module_info.header_warnings = header_warnings;

                    module_registry.write().register(module_info);
                    self.register_inline_modules(&module, &module_path, file_id);
                    let module_rc = Arc::new(module);
                    self.modules.insert(module_path_str.clone(), module_rc.clone());
                    // Also store in project_modules so they survive self.modules.clear()
                    self.project_modules.insert(module_path_str.clone(), module_rc);
                    debug!("Loaded project module: {}", module_path_str.as_str());
                }
                Err(e) => {
                    debug!("Failed to parse project module {}: {:?}", module_path_str.as_str(), e);
                }
            }
        }

        // Resolve re-exports within project modules
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let _ = resolve_glob_reexports(&mut guard);
        }

        Ok(())
    }

    /// Recursively discover all `.vr` files under `dir`, skipping hidden
    /// directories (names starting with `.`), `target/`, and `node_modules/`.
    /// The main input file (identified by `canonical_input`) and test files
    /// (names starting with `test_`) are also excluded.
    fn discover_vr_files_recursive(
        dir: &std::path::Path,
        canonical_input: &Option<PathBuf>,
        out: &mut Vec<PathBuf>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_dir() {
                let dir_name = entry.file_name();
                let name = dir_name.to_str().unwrap_or("");
                // Skip hidden directories, build artifacts, and node_modules
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                Self::discover_vr_files_recursive(&path, canonical_input, out);
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                // Skip the main input file (it will be loaded separately)
                if path.canonicalize().ok().as_ref() == canonical_input.as_ref() {
                    continue;
                }
                // Skip test files (they're standalone)
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.starts_with("test_") {
                    continue;
                }
                out.push(path);
            }
        }
    }

    /// Extract all exports from a module regardless of visibility.
    /// Used for project-internal modules where all items should be accessible.
    fn extract_all_exports(
        module: &Module,
        module_id: ModuleId,
        module_path: &ModulePath,
    ) -> verum_modules::exports::ExportTable {
        use verum_ast::ItemKind;
        use verum_modules::exports::{ExportTable, ExportedItem, ExportKind};
        use verum_ast::Visibility;

        let mut export_table = ExportTable::new();
        export_table.set_module_id(module_id);
        export_table.set_module_path(module_path.clone());

        for item in &module.items {
            let result = match &item.kind {
                ItemKind::Function(func) => {
                    let kind = if func.is_meta { ExportKind::Meta } else { ExportKind::Function };
                    export_table.add_export(ExportedItem::new(
                        func.name.name.as_str(), kind, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Type(type_decl) => {
                    let _ = export_table.add_export(ExportedItem::new(
                        type_decl.name.name.as_str(), ExportKind::Type, Visibility::Public, module_id, item.span,
                    ));
                    // Also export variant constructors
                    if let verum_ast::decl::TypeDeclBody::Variant(variants) = &type_decl.body {
                        for variant in variants {
                            let _ = export_table.add_export(ExportedItem::new(
                                variant.name.name.as_str(), ExportKind::Function, Visibility::Public, module_id, variant.span,
                            ));
                        }
                    }
                    Ok(())
                }
                ItemKind::Protocol(proto) => {
                    let kind = if proto.is_context { ExportKind::Context } else { ExportKind::Protocol };
                    export_table.add_export(ExportedItem::new(
                        proto.name.name.as_str(), kind, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Const(const_decl) => {
                    export_table.add_export(ExportedItem::new(
                        const_decl.name.name.as_str(), ExportKind::Const, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Static(static_decl) => {
                    export_table.add_export(ExportedItem::new(
                        static_decl.name.name.as_str(), ExportKind::Const, Visibility::Public, module_id, item.span,
                    ))
                }
                _ => Ok(()), // Skip impl blocks, modules, imports, etc.
            };
            if let Err(e) = result {
                debug!("Failed to add export in project module: {:?}", e);
            }
        }

        export_table
    }

    /// Discover all .vr files in the stdlib directory.
    fn discover_stdlib_files(&self, stdlib_path: &Path) -> Result<List<PathBuf>> {
        let mut files = List::new();
        self.discover_stdlib_files_recursive(stdlib_path, &mut files, 0)?;
        Ok(files)
    }

    /// Recursively discover .vr files in stdlib directory.
    fn discover_stdlib_files_recursive(
        &self,
        dir: &Path,
        files: &mut List<PathBuf>,
        depth: usize,
    ) -> Result<()> {
        const MAX_DEPTH: usize = 10;

        if depth >= MAX_DEPTH || !dir.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_symlink() {
                continue;
            }

            if path.is_dir() {
                // Skip examples directory - it contains demo code with unsupported features
                let dir_name = path.file_name().map(|n| n.to_string_lossy());
                if dir_name.as_deref() != Some("examples") {
                    self.discover_stdlib_files_recursive(&path, files, depth + 1)?;
                }
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                files.push(path);
            }
        }

        Ok(())
    }

    /// Parse a stdlib module (similar to parse_and_register but for stdlib).
    fn parse_stdlib_module(
        &mut self,
        module_path: &Text,
        source: &Text,
        file_path: &Path,
    ) -> Result<Module> {
        // Load file into session for proper file_id tracking
        let file_id = self.session.load_file(file_path)?;

        let lexer = Lexer::new(source.as_str(), file_id);

        let parser = VerumParser::new();
        let module = parser.parse_module(lexer, file_id).map_err(|errors| {
            // A stdlib module that fails to parse is either a compiler bug
            // (the parser can't handle syntax we ship in core/*.vr) or a
            // stdlib bug (invalid syntax shipped). Either way it causes
            // every downstream `mount core.*.X` to silently fail with
            // "module not found", which is a far worse diagnostic than
            // the real parse error. Emit at WARN so stdlib breakage is
            // surfaced in normal tooling runs and cannot regress unseen.
            for error in &errors {
                warn!("Stdlib parse error in {}: {}", module_path.as_str(), error);
            }
            anyhow::anyhow!(
                "Parsing stdlib module {} failed with {} error(s)",
                module_path.as_str(),
                errors.len()
            )
        })?;

        Ok(module)
    }

    /// Register inline modules (modules defined with `public module name { ... }`)
    ///
    /// This is needed for modules like `std.prelude` which are defined inline
    /// in `core/mod.vr` rather than in their own file.
    fn register_inline_modules(
        &self,
        parent_module: &Module,
        parent_path: &ModulePath,
        file_id: FileId,
    ) {
        let module_registry = self.session.module_registry();

        for item in &parent_module.items {
            if let ItemKind::Module(mod_decl) = &item.kind {
                // Check if this is an inline module (has items)
                if let verum_common::Maybe::Some(ref items) = mod_decl.items {
                    // Create the child module path
                    let child_path = parent_path.join(mod_decl.name.name.as_str());
                    let child_path_str = child_path.to_string();

                    // Create a synthetic AST Module from the items
                    let inline_module = Module {
                        items: items.clone(),
                        attributes: List::new(),
                        file_id,
                        span: item.span,
                    };

                    // Allocate ID and create ModuleInfo
                    let module_id = module_registry.read().allocate_id();
                    let mut module_info = ModuleInfo::new(
                        module_id,
                        child_path.clone(),
                        inline_module.clone(),
                        file_id,
                        Text::from(""), // No separate source for inline modules
                    );

                    // Extract exports
                    match extract_exports_from_module(&inline_module, module_id, &child_path) {
                        Ok(export_table) => {
                            module_info.exports = export_table;
                        }
                        Err(e) => {
                            debug!("Failed to extract exports from inline module {}: {:?}",
                                child_path_str, e);
                        }
                    }

                    // Register the inline module
                    module_registry.write().register(module_info);

                    // Recursively register any nested inline modules
                    self.register_inline_modules(&inline_module, &child_path, file_id);
                }
            }
        }
    }

    /// Parse source and register meta declarations (Pass 1)
    fn parse_and_register(&mut self, path: &Text, source: &Text) -> Result<Module> {
        // Load source as a string (files are already loaded in sources map)
        let virtual_path = PathBuf::from(path.as_str());
        let file_id = self
            .session
            .load_source_string(source.as_str(), virtual_path.clone())?;

        // Decide library-mode vs script-mode parsing based on shebang
        // autodetection or the entry-source script_mode flag. See
        // `should_parse_as_script` for the full rule.
        let script = should_parse_as_script(
            source.as_str(),
            self.session.options(),
            Some(virtual_path.as_path()),
        );

        // Parse
        let parser = VerumParser::new();
        let parse_result = if script {
            parser.parse_module_script_str(source.as_str(), file_id)
        } else {
            let lexer = Lexer::new(source.as_str(), file_id);
            parser.parse_module(lexer, file_id)
        };
        let mut module = parse_result.map_err(|errors| {
            let error_count = errors.len();
            for error in errors {
                let diag = DiagnosticBuilder::error()
                    .message(format!("Parse error: {}", error))
                    .build();
                self.session.emit_diagnostic(diag);
            }
            anyhow::anyhow!("Parsing failed with {} error(s)", error_count)
        })?;

        // Apply @cfg conditional compilation filtering
        // Filter out items that don't match the current target configuration.
        // This ensures platform-specific code (e.g., FFI blocks with @cfg(unix))
        // is excluded when compiling for incompatible targets.
        let cfg_evaluator = self.session.cfg_evaluator();
        let original_count = module.items.len();
        module.items = cfg_evaluator.filter_items(&module.items);
        let filtered_count = original_count - module.items.len();
        if filtered_count > 0 {
            debug!(
                "  Filtered {} item(s) based on @cfg predicates in {}",
                filtered_count,
                path.as_str()
            );
        }

        // Register meta functions and macros
        for item in &module.items {
            match &item.kind {
                ItemKind::Function(func) if func.is_meta => {
                    // Register meta function
                    if let Err(e) = self
                        .meta_registry
                        .register_meta_function(&Text::from(path.as_str()), func)
                    {
                        let diag = DiagnosticBuilder::error()
                            .message(format!("Failed to register meta function: {}", e))
                            .build();
                        self.session.emit_diagnostic(diag);
                    }
                }

                ItemKind::Meta(_meta_decl) => {
                    // Register macro
                    // Note: This would need actual macro extraction logic
                    debug!("  Found macro declaration (registration pending)");
                }

                _ => {
                    // Other items don't need registration
                }
            }
        }

        // Header validation at the parse_and_register
        // user-source path. Surfaces dangling forward-decls and
        // inline-vs-filesystem overlaps for files that don't go
        // through phase_parse (e.g. multi-source registration in
        // run_full_compilation).
        let header_warnings =
            verum_modules::loader::validate_module_headers_against_filesystem(
                &PathBuf::from(path.as_str()),
                &module,
            );
        for warning in header_warnings {
            let diag = DiagnosticBuilder::warning()
                .code(warning.code())
                .message(warning.message())
                .build();
            self.session.emit_diagnostic(diag);
        }

        Ok(module)
    }
}
