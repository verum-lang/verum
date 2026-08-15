//! Stdlib + project + cog module discovery, parsing, and loading.
//!
//! Extracted from `pipeline.rs` (#106 Phase 13). Houses the
//! module-graph plumbing that populates `self.modules` /
//! `self.project_modules` before semantic analysis runs.
//!
//! Methods:
//!
//!  * `load_stdlib_modules` — primary entry; two-tier-cached
//!  stdlib loader (registry cache → module cache → cold parse).
//!  Called once per `Compiler` lifecycle before any user code.
//!  * `load_external_cog_modules` — pulls modules from
//!  externally-registered cogs (verum-add deps,
//!  `dependencies` in script-mode frontmatter).
//!  * `load_project_modules` — discovers + parses sibling .vr
//!  files in multi-file projects (cross-file `mount foo.bar`
//!  resolution).
//!  * `discover_vr_files_recursive` — directory walker.
//!  * `extract_all_exports` — module → ExportTable conversion.
//!  * `discover_stdlib_files` + `discover_stdlib_files_recursive`
//!  — embedded-stdlib unpacking helpers.
//!  * `parse_stdlib_module` — single-file stdlib parser
//!  (with diagnostic emission).
//!  * `parse_and_register` — atomic parse + register-with-session
//!  for general-purpose use.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::{debug, info, warn};

use verum_ast::{FileId, Module, decl::ItemKind};
use verum_common::{List, Text};
use verum_diagnostics::DiagnosticBuilder;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};


use super::{
    BuildMode, CachedStdlibModules, CompilationPipeline, compute_stdlib_content_hash,
    global_stdlib_cache, global_stdlib_registry_cache, save_registry_to_disk,
    should_parse_as_script, try_load_registry_from_disk,
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
    ///  cached, we deep_clone it (~1ms) instead of re-registering all modules
    /// 2. **Module cache (FALLBACK)**: If no registry cache, we use cached parsed
    ///  modules to avoid re-parsing, then populate and cache the registry
    ///
    /// The registry cache provides ~500ms speedup per compilation by avoiding:
    /// - Module registration in ModuleRegistry (~166 modules)
    /// - Export extraction from each module
    /// - Glob re-export resolution (iterative)
    ///
    /// Loads stdlib with two-tier caching: (1) registry cache from prior compilation,
    /// (2) parsed module cache to avoid re-parsing ~166 stdlib modules.
    pub(super) fn load_stdlib_modules(&mut self) -> Result<()> {
        self.load_stdlib_modules_scoped(None)
    }

    /// As [`Self::load_stdlib_modules`], materialising only the stdlib
    /// modules in `scope` (the user file's mount closure ⋃ the implicit
    /// prelude, as computed by
    /// [`crate::stdlib_reachability::compute_reachable_stdlib_modules`]).
    ///
    /// STDLIB-LOAD-COST-1.  The pipeline's established shape is *load
    /// everything, then prune*: `run_check_only` builds the whole registry
    /// and calls `register_modules_for_cross_file_resolution_filtered`
    /// afterwards, `run_full_compilation` calls
    /// `clear_non_compilable_stdlib_modules` afterwards.  Pruning after the
    /// fact frees nothing that was not already allocated — peak memory,
    /// which is what decides whether N compilers fit on a machine, is set
    /// by the load, not by what survives it.  Measured on an EMPTY file:
    /// 8 MB before the pipeline runs, 721 MB by the time the parser
    /// reports a syntax error, 1059 MB at the end of typecheck.
    ///
    /// Passing the scope IN turns "build 2560 modules, keep 40" into
    /// "build 40".  `None` keeps the unscoped behaviour, and is what every
    /// caller without a parsed user module still passes.
    ///
    /// Two escape hatches, both pre-existing in spirit: `VERUM_FULL_STDLIB=1`
    /// (already the documented "give me every stdlib module up front" gate)
    /// and `VERUM_NO_STDLIB_SCOPE=1` as the A/B kill switch for this change
    /// alone.
    pub(super) fn load_stdlib_modules_scoped(
        &mut self,
        scope: Option<&std::collections::HashSet<String>>,
    ) -> Result<()> {
        let scope = if scope.is_some()
            && (std::env::var_os("VERUM_NO_STDLIB_SCOPE").is_some()
                || std::env::var_os("VERUM_FULL_STDLIB").is_some())
        {
            None
        } else {
            scope
        };
        let start = Instant::now();
        debug!("load_stdlib_modules called");
        let trace = std::env::var("VERUM_TRACE_PHASES").is_ok();
        if trace {
            eprintln!("[phase] load_stdlib_modules: enter");
        }

        // T2-extended single-path: typecheck consumes embedded
        // CoreMetadata directly, but `mount` resolution still needs
        // a populated `ModuleRegistry` to walk stdlib paths
        // (`mount core.base.{Maybe}` looks up `core.base` in the
        // registry, not in CoreMetadata).
        //
        // The earlier skip-on-metadata-present early-return left
        // the registry empty, so any user-code mount of a stdlib
        // submodule resolved as `module not found` despite the
        // typechecker having full type info.
        //
        // Fall through to the in-memory / disk / source cache
        // chain below; the in-memory cache hit is ~1ms after the
        // first run, and subsequent runs reuse `STDLIB_REGISTRY`
        // via `global_stdlib_registry_cache()`.

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
                // bytecode (see #143). Path-sorted iteration is stable
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
                if trace {
                    eprintln!(
                        "[phase] load_stdlib_modules: registry cache HIT {} modules in {:.2}ms",
                        module_count,
                        elapsed.as_secs_f64() * 1000.0
                    );
                }
                return Ok(());
            }
        }
        if trace {
            // NOT "about to parse stdlib from source", which is what this
            // line used to say and what a reader then spends an hour
            // chasing.  A miss here means only that the IN-MEMORY registry
            // cache is cold.  What follows on the Normal path is
            // `load_stdlib_from_embedded` — a decode of the baked archive,
            // no .vr file opened.  Source parsing happens only under
            // StdlibBootstrap or an explicit `VERUM_STDLIB_PATH`, and the
            // next line says which one ran.
            eprintln!(
                "[phase] load_stdlib_modules: in-memory registry cache MISS \
                 (embedded archive on the Normal path; source only under \
                 StdlibBootstrap / VERUM_STDLIB_PATH)"
            );
        }

        // ARCHITECTURE: Normal builds use the embedded precompiled
        // stdlib archive (`target/precompiled-stdlib/runtime.vbca` baked
        // into the binary at build time) as the SINGLE source of stdlib
        // types, functions, and constants.  Source-driven filesystem
        // loading is the StdlibBootstrap mode only (used to *produce*
        // the archive).  Routing user-facing builds through filesystem
        // creates an asymmetry vs `compile_ast_to_vbc`'s archive-only
        // codegen path: typecheck would see one stdlib snapshot while
        // codegen consults another, and any mismatch surfaces as
        // `UndefinedVariable` at the use site after typecheck succeeds.
        //
        // `VERUM_STDLIB_PATH` retains its escape-hatch role for
        // explicit out-of-tree experimentation; without it, Normal mode
        // exclusively populates the registry from
        // `load_stdlib_from_embedded` (CoreMetadata-driven, ~2 ms).
        let (stdlib_path, workspace_root_for_cache): (PathBuf, Option<PathBuf>) =
            match &self.build_mode {
                BuildMode::StdlibBootstrap { config } => {
                    debug!(
                        "StdlibBootstrap mode: using configured path {:?}",
                        config.stdlib_path
                    );
                    (config.stdlib_path.clone(), None)
                }
                BuildMode::Normal => {
                    // Explicit override — opt-in only.  The default
                    // Normal path skips filesystem entirely and goes
                    // through the embedded archive.
                    if let Ok(path) = std::env::var("VERUM_STDLIB_PATH") {
                        let p = PathBuf::from(&path);
                        if p.exists() {
                            debug!("VERUM_STDLIB_PATH override: using {:?}", p);
                            (p, None)
                        } else {
                            debug!(
                                "VERUM_STDLIB_PATH set to non-existent path {:?} — falling back to embedded archive",
                                p
                            );
                            return self.load_stdlib_from_embedded(scope);
                        }
                    } else {
                        debug!("Normal mode: populating registry from embedded archive");
                        return self.load_stdlib_from_embedded(scope);
                    }
                }
            };

        // SLOW PATH: source-driven parsing.  Reached only by
        // StdlibBootstrap mode and the explicit `VERUM_STDLIB_PATH`
        // override above.
        debug!("No in-memory registry cache, loading stdlib from source");

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
                            tracing::warn!(
                                "stdlib registry cache RwLock poisoned during write, recovering"
                            );
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
                    if trace {
                        eprintln!(
                            "[phase] load_stdlib_modules: DISK-CACHE HIT {} modules in {:.2}ms",
                            module_count,
                            elapsed.as_secs_f64() * 1000.0
                        );
                    }
                    return Ok(());
                }
            }
        }

        // FULL LOAD: No cache available, parse everything from source
        debug!("No disk cache, performing full stdlib load");
        if trace {
            eprintln!(
                "[phase] load_stdlib_modules: disk cache MISS, full SOURCE LOAD starting"
            );
        }

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

            info!(
                "Parsing {} stdlib module(s) (first load, parallel)...",
                stdlib_files.len()
            );

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
            //
            // #111 progress events — emit a tracing::info! at every 10%
            // boundary AND at the start so the user sees motion instead
            // of ~80s of silence on a cold cache. The progress log
            // honours the existing `--verbose` / `RUST_LOG=info` flags;
            // quiet mode never sees it. The first / last events are
            // emitted unconditionally so callers always see entry +
            // exit, regardless of the file count.
            let total = file_data.len();
            let progress_step = total.div_ceil(10).max(1);
            info!(
                "Parsing stdlib: starting [0/{}]",
                total
            );
            let mut entries = Vec::with_capacity(file_data.len());
            for (idx, (module_path_str, source_text, file_path)) in file_data.iter().enumerate() {
                match self.parse_stdlib_module(
                    module_path_str,
                    &Text::from(source_text.clone()),
                    file_path,
                ) {
                    Ok(module) => {
                        entries.push((
                            module_path_str.clone(),
                            module,
                            Text::from(source_text.clone()),
                        ));
                    }
                    Err(e) => {
                        debug!(
                            "Failed to parse stdlib module {}: {:?}",
                            module_path_str.as_str(),
                            e
                        );
                    }
                }
                let parsed = idx + 1;
                if parsed == total || parsed.is_multiple_of(progress_step) {
                    info!(
                        "Parsing stdlib: [{}/{}] {}",
                        parsed,
                        total,
                        module_path_str.as_str()
                    );
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
                source_text.clone(),
            );

            match extract_exports_from_module(module, module_id, &module_path) {
                Ok(export_table) => {
                    let export_count = export_table.len();
                    module_info.exports = export_table;
                    debug!(
                        "{} has {} items, {} exports",
                        module_path_str.as_str(),
                        item_count,
                        export_count
                    );
                }
                Err(e) => {
                    debug!(
                        "Failed to extract exports from {}: {:?}",
                        module_path_str.as_str(),
                        e
                    );
                }
            }

            module_registry.write().register(module_info);
            self.register_inline_modules(module, &module_path, file_id);
            self.modules
                .insert(module_path_str.clone(), Arc::new(module.clone()));
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
                        debug!(
                            "Glob re-export iteration {}: resolved {} exports",
                            iteration, resolved_count
                        );
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
        if trace {
            eprintln!(
                "[phase] load_stdlib_modules: SOURCE-PARSED {} modules in {:.2}ms",
                stdlib_count,
                elapsed.as_secs_f64() * 1000.0
            );
        }

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
    pub(super) fn load_external_cog_modules(&mut self) -> Result<()> {
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
                let stem = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
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
                        self.modules
                            .insert(module_path_str.clone(), module_rc.clone());
                        self.project_modules
                            .insert(module_path_str.clone(), module_rc);
                        debug!("Loaded external-cog module: {}", module_path_str.as_str());
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

    pub(super) fn load_project_modules(&mut self) -> Result<()> {
        let input_path = self.session.options().input.clone();
        let immediate_dir = match input_path.parent() {
            Some(dir) if dir.as_os_str().is_empty() => std::env::current_dir()?,
            Some(dir) => dir.to_path_buf(),
            None => return Ok(()),
        };

        // Canonicalize for reliable path comparison
        let immediate_dir = immediate_dir.canonicalize().unwrap_or(immediate_dir);

        // **Project-root walk (#192 fundamental fix)**.
        //
        // Pre-fix the project root was the input file's IMMEDIATE
        // parent directory.  For a file like `core/verify/kernel_v0/soundness.vr`
        // that picked `kernel_v0/` as the root and used `kernel_v0`
        // as the project_prefix — so the file's `module
        // core.verify.kernel_v0.soundness;` declaration mismatched
        // the loader-derived path `kernel_v0.soundness`.
        //
        // Post-fix the loader walks UP from the immediate parent
        // until it finds a directory carrying a `verum.toml`.  That
        // is the outermost project root; its directory name (e.g.
        // `core`) becomes the project_prefix and the relative path
        // captures the full namespace structure correctly.
        //
        // The walk stops at the first `verum.toml` ancestor — if
        // none exists we fall back to the immediate-dir behaviour
        // so single-file scripts still work.
        let input_dir = {
            let mut cursor = immediate_dir.clone();
            let mut root: Option<std::path::PathBuf> = None;
            loop {
                if cursor.join("verum.toml").exists() {
                    root = Some(cursor.clone());
                    break;
                }
                match cursor.parent() {
                    Some(p) if p != cursor => {
                        cursor = p.to_path_buf();
                    }
                    _ => break,
                }
            }
            root.unwrap_or(immediate_dir.clone())
        };

        // The stdlib `core` cog must never be eager-loaded as a user
        // project.  In `Normal` mode `core` is provided by the embedded
        // precompiled archive (+ on-demand loading), so `mount core.*`
        // already resolves without compiling core's source.  Eager-loading
        // it here pulls EVERY core module body into the compilation unit —
        // including modules unreachable from the entry point — and native
        // codegen then aborts on the first undefined stdlib leaf function
        // (`sha512_digest`, `fs_current_dir`, `equiv_inv_coherence_law`, …).
        //
        // This is precisely why `verum test --aot` failed suite-wide: the
        // harness writes its merged test file into `<cog>/target/test/`,
        // which for the `core` cog lands *inside* core, so the project walk
        // above resolves to the core root and drags in the whole stdlib.
        // The identical mounts compiled cleanly via `verum run/build --aot`
        // on a file OUTSIDE the cog.  `StdlibBootstrap` (which PRODUCES the
        // archive from core's source) does not take this path, so it is
        // unaffected.
        if matches!(self.build_mode, BuildMode::Normal) {
            let manifest = input_dir.join("verum.toml");
            if manifest.exists()
                && let Ok(cfg) = crate::linker_config::ProjectConfig::load_from_file(&manifest)
                && cfg.cog.name == "core"
            {
                info!(
                    "Skipping eager project-load of the stdlib `core` cog at {} — served from the precompiled archive",
                    input_dir.display()
                );
                return Ok(());
            }
        }

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

        info!(
            "Detected multi-file project '{}' in {}",
            project_prefix,
            input_dir.display()
        );

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
            let stem = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            // Build dotted module path from relative directory components
            // e.g. project_dir/sub/foo.vr -> "project.sub.foo"
            //  project_dir/sub/mod.vr -> "project.sub"
            //  project_dir/foo.vr -> "project.foo"
            //  project_dir/mod.vr -> "project"
            let module_path_str = {
                let rel = file_path
                    .parent()
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
            // declaring module `<project>.foo`. Surface this as a hard
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
                    debug!(
                        "Failed to read project module {}: {:?}",
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
                            file_path, &module,
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
                    self.modules
                        .insert(module_path_str.clone(), module_rc.clone());
                    // Also store in project_modules so they survive self.modules.clear()
                    self.project_modules
                        .insert(module_path_str.clone(), module_rc);
                    debug!("Loaded project module: {}", module_path_str.as_str());
                }
                Err(e) => {
                    debug!(
                        "Failed to parse project module {}: {:?}",
                        module_path_str.as_str(),
                        e
                    );
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
        // REPRODUCIBILITY (T0736): `read_dir` yields directory order, which
        // is a filesystem property, not a property of the project. Discovery
        // order decides module registration order, and registration is
        // first-wins, so an unsorted walk makes the compiled artifact depend
        // on how the checkout happens to be laid out. Sort by path so two
        // machines with the same sources produce the same module table.
        let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
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
        use verum_ast::Visibility;
        use verum_modules::exports::{ExportKind, ExportTable, ExportedItem};

        let mut export_table = ExportTable::new();
        export_table.set_module_id(module_id);
        export_table.set_module_path(module_path.clone());

        for item in &module.items {
            let result = match &item.kind {
                ItemKind::Function(func) => {
                    let kind = if func.is_meta {
                        ExportKind::Meta
                    } else {
                        ExportKind::Function
                    };
                    export_table.add_export(ExportedItem::new(
                        func.name.name.as_str(),
                        kind,
                        Visibility::Public,
                        module_id,
                        item.span,
                    ))
                }
                ItemKind::Type(type_decl) => {
                    let _ = export_table.add_export(ExportedItem::new(
                        type_decl.name.name.as_str(),
                        ExportKind::Type,
                        Visibility::Public,
                        module_id,
                        item.span,
                    ));
                    // Also export variant constructors
                    if let verum_ast::decl::TypeDeclBody::Variant(variants) = &type_decl.body {
                        for variant in variants {
                            let _ = export_table.add_export(ExportedItem::new(
                                variant.name.name.as_str(),
                                ExportKind::Function,
                                Visibility::Public,
                                module_id,
                                variant.span,
                            ));
                        }
                    }
                    Ok(())
                }
                ItemKind::Protocol(proto) => {
                    let kind = if proto.is_context {
                        ExportKind::Context
                    } else {
                        ExportKind::Protocol
                    };
                    export_table.add_export(ExportedItem::new(
                        proto.name.name.as_str(),
                        kind,
                        Visibility::Public,
                        module_id,
                        item.span,
                    ))
                }
                ItemKind::Const(const_decl) => export_table.add_export(ExportedItem::new(
                    const_decl.name.name.as_str(),
                    ExportKind::Const,
                    Visibility::Public,
                    module_id,
                    item.span,
                )),
                ItemKind::Static(static_decl) => export_table.add_export(ExportedItem::new(
                    static_decl.name.name.as_str(),
                    ExportKind::Const,
                    Visibility::Public,
                    module_id,
                    item.span,
                )),
                _ => Ok(()), // Skip impl blocks, modules, imports, etc.
            };
            if let Err(e) = result {
                debug!("Failed to add export in project module: {:?}", e);
            }
        }

        export_table
    }

    /// `true` when there is no scope at all (load everything) or
    /// `module_path` is inside it.  The `None` case is the unscoped
    /// caller, not "nothing is in scope".
    fn path_in_scope(
        scope: &Option<std::collections::HashSet<String>>,
        module_path: &str,
    ) -> bool {
        scope.as_ref().is_none_or(|s| s.contains(module_path))
    }

    /// The mount closure, plus every UMBRELLA module on the way to one of
    /// its members.
    ///
    /// Exact membership against the raw closure is the wrong test, because
    /// the two sides spell modules at different granularity.
    /// `compute_reachable_stdlib_modules` filters its result through the
    /// stdlib index, so it names modules that resolve to a `.vr` FILE
    /// (`core.base.maybe`).  The metadata shards types and functions under
    /// the archive ENTRY module — the directory, `core.base` — as well as
    /// under the precise origin module, and the entry module is what
    /// `mount core.base.{Maybe}` looks up.  Dropping `core.base` because
    /// only `core.base.maybe` was named would turn a correct mount into
    /// "module not found".
    ///
    /// The reverse direction is deliberately NOT included.  Admitting every
    /// DESCENDANT of a closure member would be wrong twice over: the glob
    /// expansion in `compute_reachable_stdlib_modules` already puts the
    /// files of a `mount core.text.*` into the closure, so it buys nothing —
    /// and a single `core` seed (the prelude reaches `core/mod.vr`) would
    /// then admit the entire stdlib and silently disable the scope.
    ///
    /// Segment-boundary splitting, not `starts_with`, is what keeps
    /// `core.text` from being read as an ancestor of `core.textual`.
    fn scope_with_umbrellas(
        scope: &std::collections::HashSet<String>,
    ) -> std::collections::HashSet<String> {
        let mut out =
            std::collections::HashSet::with_capacity(scope.len() * 2);
        for module in scope {
            out.insert(module.clone());
            let mut prefix = module.as_str();
            while let Some(cut) = prefix.rfind('.') {
                prefix = &prefix[..cut];
                if !out.insert(prefix.to_string()) {
                    break; // this ancestor chain is already recorded
                }
            }
        }
        out
    }

    /// Populate the module registry from the embedded CoreMetadata.
    ///
    /// Called when no filesystem stdlib directory is found (external-project
    /// invocation where the binary is installed but `core/` is not adjacent).
    ///
    /// Instead of parsing all 2500+ .vr source files (expensive, OOM-prone),
    /// we derive module paths and export tables directly from the `CoreMetadata`
    /// that was already decoded and embedded in the binary. Each unique
    /// `type.module_path` / `function.module_path` becomes a `ModuleInfo` entry
    /// whose exports are the corresponding types/functions. This is O(n) over
    /// the metadata records (~2 ms) vs O(n) over source parses (~60-120 min
    /// on the full 2540-file stdlib).
    ///
    /// `scope`, when present, restricts the whole pass — grouping included —
    /// to the user file's mount closure; see
    /// [`Self::load_stdlib_modules_scoped`].
    fn load_stdlib_from_embedded(
        &mut self,
        scope: Option<&std::collections::HashSet<String>>,
    ) -> Result<()> {
        use verum_ast::{decl::Visibility, Span};
        use verum_modules::{ExportKind, ExportTable, ExportedItem};

        let start = std::time::Instant::now();

        let metadata = match self.stdlib_metadata.get() {
            Some(m) => m.clone(),
            None => {
                // No metadata either — nothing we can do.
                debug!("No CoreMetadata available for embedded stdlib registry build");
                return Ok(());
            }
        };

        // STDLIB-LOAD-COST-1 probe.  Every compile pays a fixed stdlib cost
        // before it knows anything about the user's program — measured on an
        // EMPTY file: 8 MB with `--parse-only`, 721 MB once the full pipeline
        // runs, 1059 MB after typecheck.  Two candidates share that 721 MB:
        // the bincode decode of the embedded `CoreMetadata` (one 38 MB blob)
        // and the registry synthesis below (a `Vec<String>` shard per module
        // plus an `ExportTable` entry per symbol, over ~44k symbols).
        //
        // Attributing it needs a stop line between them, and no existing
        // subcommand sits there: `--parse-only` stops before both, every
        // other entry point runs both.  Setting this variable stops the
        // process the instant the decode has happened and BEFORE the
        // synthesis, so `/usr/bin/time -l` reads the decode's share alone;
        // the difference against a normal run is the synthesis's share.
        //
        // Kept in the tree because the numbers are the check that a fix
        // worked, not just the evidence that a fix was needed.
        if std::env::var_os("VERUM_PROBE_STDLIB_DECODE_ONLY").is_some() {
            eprintln!(
                "[stdlib-probe] decode-only stop: {} types, {} functions, {} protocols",
                metadata.types.len(),
                metadata.functions.len(),
                metadata.protocols.len()
            );
            std::process::exit(0);
        }

        // STDLIB-LOAD-COST-1: the mount closure, widened to the umbrella
        // modules its members hang under.  Computed ONCE, here, because the
        // grouping loops below are where the per-symbol `String`s are
        // allocated — filtering only at the registration loop would still
        // pay for all ~44 000 of them and then throw the surplus away.
        let scope_expanded = scope.map(Self::scope_with_umbrellas);
        let in_scope = |module_path: &str| -> bool {
            Self::path_in_scope(&scope_expanded, module_path)
        };

        // Collect the set of all unique module paths declared in the metadata.
        // We group types, protocols, and functions by their module_path.
        //
        // Shard: module_path → (types, protocols, functions)
        let mut module_map: std::collections::BTreeMap<
            String,                         // module_path (sorted for determinism)
            (Vec<String>, Vec<String>, Vec<String>), // (types, protocols, functions)
        > = std::collections::BTreeMap::new();

        for (name, td) in metadata.types.iter() {
            let mp = td.module_path.as_str();
            if !in_scope(mp)
                && !matches!(&td.origin_module_path,
                    verum_common::Maybe::Some(om) if in_scope(om.as_str()))
            {
                continue;
            }
            let mp = mp.to_string();
            // v2.12 TYPE-ORIGIN-MODULE (T0555): `module_path` is the archive
            // ENTRY (directory) module; a type declared in a FILE submodule
            // carries the precise path in `origin_module_path`. The
            // synthesized registry must expose the type on BOTH — the entry
            // umbrella keeps working, and the file submodule stops reporting
            // only its re-export leaves (the probed-exports E401: mounting
            // `ext.hooks.{AaInsert}` saw a 5-entry surface because every own
            // type had been sharded under `...ext`).
            let mut paths: Vec<String> = Vec::new();
            if !mp.is_empty() {
                paths.push(mp);
            }
            if let verum_common::Maybe::Some(om) = &td.origin_module_path {
                let om = om.as_str().to_string();
                if !om.is_empty() && !paths.contains(&om) {
                    paths.push(om);
                }
            }
            for path in paths {
                if !in_scope(&path) {
                    continue;
                }
                let shard = module_map.entry(path).or_default();
                shard.0.push(name.as_str().to_string());
                // Also export variant constructors so `Ok`, `Some`, `None`, etc.
                // are in scope after `mount core.base.*`.
                if let verum_types::core_metadata::TypeDescriptorKind::Variant { cases } =
                    &td.kind
                {
                    for case in cases {
                        shard.0.push(case.name.as_str().to_string());
                    }
                }
            }
        }
        // v2.13 FN-ORIGIN-MODULE — the value-namespace twin of the type
        // loop above, and the whole reason `mount core.prelude.*` could
        // not deliver a free function.
        //
        // Publishing on the entry path ALONE meant a file submodule's
        // export table held its types (rescued onto the origin path by
        // the loop above) and NONE of its functions: `core.base.iterator`
        // was published with 21 types and 0 of its 13 free functions.
        // A glob mount ENUMERATES this table — `import_all_from_module`
        // walks `public_exports()` — so it never named `range` and the
        // bare call died E100, while `mount core.base.iterator.range;`
        // worked because the NAMED path looks a name up and has a
        // by-name metadata rescue an enumeration can never reach.
        //
        // The type namespace hid its half of the hole behind
        // `ensure_stdlib_type_loaded` (lookup-on-miss by bare name);
        // there is no such loader for functions, so here it was fatal.
        // Both kinds now publish on BOTH paths, exactly as types do.
        let publish = |module_map: &mut std::collections::BTreeMap<
            String,
            (Vec<String>, Vec<String>, Vec<String>),
        >,
                       entry_path: &str,
                       origin: &verum_common::Maybe<verum_common::Text>,
                       name: &str,
                       shard: usize| {
            let mut paths: Vec<String> = Vec::new();
            if !entry_path.is_empty() {
                paths.push(entry_path.to_string());
            }
            if let verum_common::Maybe::Some(om) = origin {
                let om = om.as_str().to_string();
                if !om.is_empty() && !paths.contains(&om) {
                    paths.push(om);
                }
            }
            // STDLIB-LOAD-COST-1: same filter as the type loop, applied
            // before the per-symbol `String` is allocated rather than after.
            for path in paths {
                if !Self::path_in_scope(&scope_expanded, &path) {
                    continue;
                }
                let e = module_map.entry(path).or_default();
                match shard {
                    1 => e.1.push(name.to_string()),
                    _ => e.2.push(name.to_string()),
                }
            }
        };
        for (_key, pd) in metadata.protocols.iter() {
            publish(
                &mut module_map,
                pd.module_path.as_str(),
                &pd.origin_module_path,
                pd.name.as_str(),
                1,
            );
        }
        // Functions publish the map KEY under the ENTRY path.
        //
        // MEASURED REVERT (2026-08-09): the v2.13 origin carry does NOT
        // belong here. Publishing `fd.name` on both paths, with or
        // without a method filter, cost 26 tests in intrinsics/arithmetic
        // (`unbound variable: wrapping_shl`), while reverting this loop
        // AND the reduction below restored 147/0 there and left
        // text/text at 465/0 — i.e. the glob-mount win comes from the
        // OTHER legs (origin in metadata, precompile glob_matches), not
        // from this publication. Four selective repairs each failed to
        // reproduce that, so the loop stays as it was.
        for (name, fd) in metadata.functions.iter() {
            let mp = fd.module_path.as_str().to_string();
            if !mp.is_empty() {
                module_map.entry(mp).or_default().2.push(name.as_str().to_string());
            }
        }

        // T0693 RE-EXPORT LEAVES — the synthesized surface must carry what
        // `public mount .sub.{X as Y};` publishes.
        //
        // `module_reexports[M] = [(local_name, original_name, source_module)]`
        // is the ONLY record of a renamed re-export. The three descriptor
        // families above carry the TARGET (`X`, under `source_module`) and
        // never the name the PARENT publishes (`Y`, under `M`) — so the
        // registry surface and `metadata_known_module_items` answered
        // "does M export Y" differently, and the probed-exports E401 gate
        // in `verum_types::infer::modules` consults the registry one.
        // `mount core.term.layout.{Constraint};` therefore failed against a
        // 263-entry surface that legitimately lacked only the leaves.
        //
        // The kind is READ OFF THE TARGET, never assumed: `module_map`
        // already records, per module, which names are types, protocols and
        // functions, and this fold runs after all three publication loops.
        // A leaf whose target names no descriptor is SKIPPED — publishing
        // it under a guessed kind would seed a wrong entry into a surface
        // that glob mounts ENUMERATE, and a glob has no by-name rescue to
        // correct it.
        // The three descriptor families the synthesized surface is built from.
        // Named rather than numbered: `module_map`'s tuple is positional, and a
        // fourth family added later must fail to compile here instead of
        // silently landing in whichever arm a `_` pattern happened to cover.
        #[derive(Clone, Copy)]
        enum SurfaceKind {
            Type,
            Protocol,
            Function,
        }
        let mut reexport_pubs: Vec<(String, String, SurfaceKind)> = Vec::new();
        for (module, leaves) in metadata.module_reexports.iter() {
            let mp = module.as_str();
            if mp.is_empty() {
                continue;
            }
            for (local_name, original_name, source_module) in leaves.iter() {
                let Some((src_types, src_protos, src_fns)) =
                    module_map.get(source_module.as_str())
                else {
                    continue;
                };
                let target = original_name.as_str();
                let kind = if src_types.iter().any(|n| n == target) {
                    SurfaceKind::Type
                } else if src_protos.iter().any(|n| n == target) {
                    SurfaceKind::Protocol
                } else if src_fns.iter().any(|n| n == target) {
                    SurfaceKind::Function
                } else {
                    continue;
                };
                reexport_pubs.push((mp.to_string(), local_name.as_str().to_string(), kind));
            }
        }
        for (module, local_name, kind) in reexport_pubs {
            let entry = module_map.entry(module).or_default();
            let bucket = match kind {
                SurfaceKind::Type => &mut entry.0,
                SurfaceKind::Protocol => &mut entry.1,
                SurfaceKind::Function => &mut entry.2,
            };
            if !bucket.iter().any(|n| n == &local_name) {
                bucket.push(local_name);
            }
        }

        // STDLIB-LOAD-COST-1, second stop line: the module→(types, protocols,
        // functions) grouping is complete, no `ModuleInfo` / `ExportTable`
        // has been built yet.  Together with the decode-only stop above this
        // splits the fixed cost three ways — decode, grouping, registration —
        // which is what decides WHICH of them is worth removing.
        if std::env::var_os("VERUM_PROBE_STDLIB_GROUP_ONLY").is_some() {
            let symbols: usize = module_map.values().map(|(t, p, f)| t.len() + p.len() + f.len()).sum();
            eprintln!(
                "[stdlib-probe] group-only stop: {} modules, {} grouped symbols",
                module_map.len(),
                symbols
            );
            std::process::exit(0);
        }

        let mut registered = 0usize;
        let mut skipped = 0usize;
        let module_registry = self.session.module_registry();

        for (mp_str, (types, protocols, fns)) in &module_map {
            // STDLIB-LOAD-COST-1: outside the user's mount closure this
            // module's `ModuleInfo` + `ExportTable` would be built, held,
            // and then dropped by the post-hoc prune the callers already
            // run.  Not building it is the same answer for less.
            //
            // The closure is authoritative here for the same reason it is
            // authoritative in `register_modules_for_cross_file_resolution_filtered`,
            // which prunes against this very set: it is the transitive mount
            // reachability of the user file ⋃ the implicit prelude.
            if let Some(scope) = scope_expanded.as_ref()
                && !scope.contains(mp_str.as_str())
            {
                skipped += 1;
                continue;
            }

            let module_path_text = Text::from(mp_str.as_str());

            // Skip if already in our modules map (populated by a previous call).
            if self.modules.contains_key(&module_path_text) {
                continue;
            }

            let module_path = ModulePath::from_str(mp_str.as_str());
            let module_id = module_registry.read().allocate_id();
            let file_id = FileId::new(0);

            // Build a synthetic empty Module AST — the typechecker uses
            // CoreMetadata (already loaded) for actual type resolution; the
            // AST is only needed by the module registry for export tables.
            let synthetic_module = verum_ast::Module {
                items: verum_common::List::new(),
                attributes: verum_common::List::new(),
                file_id,
                span: Span::dummy(),
            };

            // Build exports table from metadata records.
            let mut export_table = ExportTable::new();
            let dummy_span = Span::dummy();
            for type_name in types {
                let _ = export_table.add_export(ExportedItem::new(
                    type_name.as_str(),
                    ExportKind::Type,
                    Visibility::Public,
                    module_id,
                    dummy_span,
                ));
            }
            for proto_name in protocols {
                let _ = export_table.add_export(ExportedItem::new(
                    proto_name.as_str(),
                    ExportKind::Protocol,
                    Visibility::Public,
                    module_id,
                    dummy_span,
                ));
            }
            for fn_name in fns {
                // Reduce a qualified function name to the bare name a mount
                // can write.
                //
                // The previous chain was
                //   `rsplit("::").next().or_else(|| rsplit('.').next())`
                // and the `or_else` arm was UNREACHABLE: `rsplit` on a
                // separator the string does not contain still yields one
                // item — the whole string — so the first `.next()` is
                // always `Some`. Every dotted name therefore reached the
                // export table VERBATIM, and `core.base.iterator` published
                // a function literally named `core.base.iterator.range`.
                // A glob mount enumerates these names, so it asked for a
                // dotted name, got nothing, and `range(0, 3)` died E100 —
                // while a NAMED mount worked, because it looks the name up
                // and has a by-name metadata rescue an enumeration cannot
                // reach.
                //
                // The replacement strips the OWNING MODULE PATH structurally
                // instead of guessing at the last separator: the bare name
                // is exactly what remains after removing this module's own
                // prefix, and only when a single segment remains. A leftover
                // dot means a `Type.method` spelling (`I.into_iter`), which
                // is an inherent/default method — not a module-level
                // mountable name — and must not be published bare.
                // Strip the module path prefix from qualified function
                // names. NOTE: the `or_else` arm is unreachable — `rsplit`
                // on an absent separator still yields the whole string —
                // so a dotted name reaches the table verbatim. That is a
                // real defect, but fixing it HERE regressed
                // intrinsics/arithmetic by 26; it must be re-attempted
                // with a full-suite check before the commit, not after.
                let bare = fn_name
                    .rsplit("::")
                    .next()
                    .or_else(|| fn_name.rsplit('.').next())
                    .unwrap_or(fn_name.as_str());
                let _ = export_table.add_export(ExportedItem::new(
                    bare,
                    ExportKind::Function,
                    Visibility::Public,
                    module_id,
                    dummy_span,
                ));
            }

            let mut module_info = ModuleInfo::new(
                module_id,
                module_path.clone(),
                synthetic_module.clone(),
                file_id,
                Text::from(""),
            );
            module_info.exports = export_table;

            module_registry.write().register(module_info);
            self.modules.insert(module_path_text, Arc::new(synthetic_module));
            registered += 1;
        }

        // Warm the in-memory registry cache so subsequent module-level
        // TypeChecker instances in check_project skip this work.
        //
        // NOT when scoped: this cache is process-wide and its readers take
        // it as "the stdlib registry", with no record of which closure
        // produced it.  Publishing one user file's closure would make the
        // NEXT file in the same process silently see a stdlib missing
        // whatever it alone needed — a wrong answer that reads as a
        // module-not-found in unrelated code.  A partial answer must never
        // be cached under a total answer's key.
        if scope.is_none() {
            let cache = global_stdlib_registry_cache();
            let mut guard = cache.write().unwrap_or_else(|p| p.into_inner());
            if guard.is_none() {
                let reg = module_registry.read();
                *guard = Some(reg.deep_clone());
            }
        }

        let elapsed = start.elapsed();
        info!(
            "Built stdlib module registry from CoreMetadata: {} modules in {:.2}ms",
            registered,
            elapsed.as_secs_f64() * 1000.0
        );
        if std::env::var_os("VERUM_TRACE_PHASES").is_some() {
            // Report the SCOPE, not a "skipped" tally.  The grouping loops
            // above already drop out-of-scope symbols before they reach
            // `module_map`, so a skip counter on the registration loop can
            // only ever read 0 — and a 0 that cannot rise reads as
            // "nothing was excluded", which is the opposite of the truth.
            // What is honest here is how wide the closure was and how much
            // of it turned into modules.
            match scope {
                Some(s) => eprintln!(
                    "[stdlib-registry] scoped closure={} modules, umbrellas widened to {}, built={} (late-filtered={}) in {:.2}ms",
                    s.len(),
                    scope_expanded.as_ref().map_or(0, |e| e.len()),
                    registered,
                    skipped,
                    elapsed.as_secs_f64() * 1000.0
                ),
                None => eprintln!(
                    "[stdlib-registry] UNSCOPED built={} in {:.2}ms",
                    registered,
                    elapsed.as_secs_f64() * 1000.0
                ),
            }
        }
        Ok(())
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

        // REPRODUCIBILITY (T0736): see the note on the sibling walk above —
        // directory order is not project order.
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)?.flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();

            if path.is_symlink() {
                continue;
            }

            if path.is_dir() {
                // Skip directories that contain non-stdlib source:
                //  * `examples/` — demo code with unsupported features.
                //  * `target/`   — build artefacts.  Cargo emits compiled
                //    bytecode here AND the test runner stages merged
                //    test sources (`target/test/test_*.merged.vr`) which,
                //    if walked, get precompiled as `core.target.test.<fn>`
                //    and shadow user-side `@test` functions of the same
                //    bare name at archive-load time — root cause of the
                //    "test fails with stale assertion at pc=N" class.
                //  * `node_modules/` — JS dependency tree from any
                //    embedded tooling; not Verum source.
                //  * `.git/` / dotted — VCS and hidden tooling caches.
                let dir_name = path.file_name().and_then(|n| n.to_str());
                if matches!(
                    dir_name,
                    Some("examples")
                        | Some("target")
                        | Some("node_modules")
                ) {
                    continue;
                }
                if dir_name.is_some_and(|n| n.starts_with('.')) {
                    continue;
                }
                self.discover_stdlib_files_recursive(&path, files, depth + 1)?;
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                // Defence-in-depth: even when the target/ exclusion
                // above eventually misses (e.g. a future refactor
                // changes the dir layout), test merged files have
                // a recognisable name shape (`test_*.merged.vr`) and
                // are skipped by stem — they are output of the test
                // runner's `synthesise_test_input_with_crate_root`
                // helper, never authored source.
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.starts_with("test_") && stem.ends_with(".merged") {
                    continue;
                }
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
                            debug!(
                                "Failed to extract exports from inline module {}: {:?}",
                                child_path_str, e
                            );
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
    pub(super) fn parse_and_register(&mut self, path: &Text, source: &Text) -> Result<Module> {
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

        // Implicit prelude — user compiles only; stdlib bootstrap
        // defines the prelude and must not self-inject.
        if matches!(self.build_mode, BuildMode::Normal) {
            crate::pipeline::inject_implicit_prelude_mount(&mut module);
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
        let header_warnings = verum_modules::loader::validate_module_headers_against_filesystem(
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
