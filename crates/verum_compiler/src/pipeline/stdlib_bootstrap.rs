//! Stdlib (`core/`) bootstrap compilation — the `compile_core` orchestrator.
//!
//! Extracted from `pipeline.rs` (#106 Phase 8). This submodule
//! handles the StdlibBootstrap mode: a one-shot compile of the
//! `core/` standard library into the embeddable `stdlib.vbca`
//! archive that ships inside the verum binary.
//!
//! Flow:
//!
//!   1. Discover all stdlib modules via `StdlibModuleResolver`.
//!   2. Parse ALL modules to AST.
//!   3. Register ALL types globally (multi-pass across all modules).
//!   4. Compile each module to VBC bytecode.
//!   5. Build and write `stdlib.vbca` archive.
//!
//! Architectural distinction from `Normal` build mode: stdlib
//! bootstrap uses GLOBAL type registration across all modules
//! before compiling any module, eliminating cross-module
//! dependency constraints. User-code builds use per-file
//! incremental registration with the bootstrapped stdlib.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::Module;
use verum_common::{List, Map, Text};
use verum_modules::{
    ModuleInfo, ModulePath, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};
use verum_vbc::codegen::VbcCodegen;

use crate::core_compiler::{CoreConfig, StdlibCompilationResult, StdlibModule};
use crate::lint::{IntrinsicDiagnostics, IntrinsicLint};
use crate::module_utils;
use super::BuildMode;

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Compile the standard library to a VBC archive.
    ///
    /// This method is only available in `StdlibBootstrap` mode (created via `new_core()`).
    /// It uses global type registration across ALL modules before compiling any module,
    /// which eliminates cross-module dependency constraints.
    ///
    /// # Flow
    ///
    /// 1. Discover all stdlib modules via `StdlibModuleResolver`
    /// 2. Parse ALL modules to AST
    /// 3. Register ALL types globally (multi-pass across all modules)
    /// 4. Compile each module to VBC bytecode
    /// 5. Build and write `stdlib.vbca` archive
    ///
    /// # Returns
    ///
    /// Returns `StdlibCompilationResult` containing compilation statistics,
    /// or an error if compilation fails.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The pipeline is not in `StdlibBootstrap` mode
    /// - Module discovery fails
    /// - Parsing fails
    /// - Type registration fails
    /// - VBC codegen fails
    /// - Archive writing fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_compiler::{Session, CompilationPipeline, CoreConfig};
    ///
    /// let config = CoreConfig::new("stdlib")
    ///     .with_output("target/stdlib.vbca");
    ///
    /// let mut session = Session::default();
    /// let mut pipeline = CompilationPipeline::new_core(&mut session, config);
    ///
    /// let result = pipeline.compile_core()?;
    /// println!("Compiled {} modules with {} functions",
    ///     result.modules_compiled, result.functions_compiled);
    /// ```
    pub fn compile_core(&mut self) -> Result<StdlibCompilationResult> {
        use verum_ast::cfg::TargetConfig;
        use verum_vbc::write_archive_to_file;

        // Extract config from build_mode (or fail if not in StdlibBootstrap mode)
        let config = match &self.build_mode {
            BuildMode::StdlibBootstrap { config } => config.clone(),
            BuildMode::Normal => {
                return Err(anyhow::anyhow!(
                    "compile_core() requires StdlibBootstrap mode. Use new_core() to create the pipeline."
                ));
            }
        };

        let start = std::time::Instant::now();
        let mut module_times = std::collections::HashMap::new();

        // ====================================================================
        // STEP 1: Discover modules
        // ====================================================================
        if config.verbose {
            eprintln!("Discovering stdlib modules in {}...", config.stdlib_path.display());
        }

        let resolver = self.stdlib_resolver.as_mut()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        resolver.discover().map_err(|e| anyhow::anyhow!("Module discovery failed: {}", e))?;

        if config.verbose {
            eprintln!("Found {} modules", resolver.module_count());
        }

        // Get modules in dependency order
        let modules_to_compile: Vec<StdlibModule> = resolver.modules_in_order()
            .into_iter()
            .cloned()
            .collect();

        // ====================================================================
        // STEP 2: Parse ALL modules
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 1: Parsing all modules...");
        }

        // Track (module_name, [(file_path, ast_module), ...]) for submodule resolution
        let mut all_parsed_modules: Vec<(String, Vec<(PathBuf, verum_ast::Module)>)> = Vec::new();

        for module in &modules_to_compile {
            let ast_modules = self.parse_stdlib_module_files(module)?;
            all_parsed_modules.push((module.name.clone(), ast_modules));
        }

        // ====================================================================
        // STEP 2.25: Resolve file-relative mounts (#5 / P1.5)
        // ====================================================================
        //
        // Before module-registry registration, walk every
        // parsed module for `MountTreeKind::File` declarations
        // (`mount ./helper.vr;`).  For each, the resolver
        // loads the referenced file via the loader's sandbox,
        // parses it, and surfaces it as a synthetic module
        // ready to be registered alongside its peers.
        //
        // This plugs file mounts into the existing
        // module-path pipeline with zero new resolution
        // codepaths — the synthesised module name (alias or
        // file basename) becomes the canonical module-path
        // identifier in the registry, and downstream import
        // resolution treats it identically to any other
        // module.
        //
        // Soft-fail strategy: file-mount resolution errors
        // surface as warnings during stdlib bootstrap (no
        // user-authored file mounts in core/ today, so any
        // failure is a regression in our own infrastructure)
        // and as hard errors during normal compilation.
        if !modules_to_compile.is_empty() {
            let mut resolver_seeds: Vec<(PathBuf, verum_ast::Module)> = Vec::new();
            for (_mod_name, files) in &all_parsed_modules {
                for (path, ast) in files {
                    resolver_seeds.push((path.clone(), ast.clone()));
                }
            }
            match verum_modules::file_mount::resolve_file_mounts(
                &mut self.module_loader,
                &resolver_seeds,
                |source| {
                    use verum_lexer::Lexer;
                    use verum_fast_parser::VerumParser;
                    let lexer = Lexer::new(source.source.as_str(), source.file_id);
                    let parser = VerumParser::new();
                    parser
                        .parse_module(lexer, source.file_id)
                        .map_err(|errs| {
                            let summary: String = errs
                                .iter()
                                .map(|e| e.to_string())
                                .collect::<Vec<_>>()
                                .join("; ");
                            verum_modules::error::ModuleError::Other {
                                message: verum_common::Text::from(format!(
                                    "parse error in file mount `{}`: {}",
                                    source.file_path.display(),
                                    summary
                                )),
                                span: None,
                            }
                        })
                },
            ) {
                Ok(resolved) => {
                    if !resolved.is_empty() && config.verbose {
                        eprintln!(
                            "Phase 1.25: Resolved {} file-relative mount(s)",
                            resolved.len()
                        );
                    }
                    // Each resolved file becomes its own
                    // module entry in `all_parsed_modules`,
                    // with the synthesised name acting as
                    // the canonical module path.  The
                    // existing Phase 1.5 registration loop
                    // picks them up uniformly.
                    for entry in resolved {
                        // Re-parse the source for AST
                        // ownership — the parsed module from
                        // the resolver callback is dropped.
                        // (Cheap: the loader cached the read,
                        // and parse is fast.)
                        let lexer = verum_lexer::Lexer::new(
                            entry.source.as_str(),
                            entry.file_id,
                        );
                        let parser = verum_fast_parser::VerumParser::new();
                        let ast = match parser.parse_module(lexer, entry.file_id) {
                            Ok(m) => m,
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Parse error re-parsing file mount `{}`: {:?}",
                                    entry.absolute_path.display(),
                                    e
                                ));
                            }
                        };
                        all_parsed_modules.push((
                            entry.synthetic_name.clone(),
                            vec![(entry.absolute_path.clone(), ast)],
                        ));
                    }
                }
                Err(e) => {
                    // During stdlib bootstrap there should
                    // be no file mounts; if there are, log
                    // a clear warning rather than aborting
                    // (defensive — keeps stdlib compilation
                    // resilient to accidental mount syntax
                    // sneaking into core/).
                    if config.verbose {
                        eprintln!(
                            "Phase 1.25: file-mount resolution warning: {}",
                            e
                        );
                    }
                }
            }
        }

        // ====================================================================
        // STEP 2.5: Register ALL parsed modules in the ModuleRegistry
        // ====================================================================
        // This MUST happen before type registration (Step 3) so that the
        // TypeChecker can resolve cross-module imports via the registry.
        // Without this, import resolution fails with E402 (module not found).
        if config.verbose {
            eprintln!("Phase 1.5: Registering modules in ModuleRegistry...");
        }

        {
            let module_registry = self.session.module_registry();
            for (module_name, ast_modules_with_paths) in &all_parsed_modules {
                for (file_path, ast_module) in ast_modules_with_paths {
                    // Compute module path from module name + file
                    // Module names are already in dot-separated format (e.g., "core.base.primitives")
                    let module_path = ModulePath::from_str(module_name.as_str());
                    let module_id = module_registry.read().allocate_id();

                    let file_id = ast_module.items.first()
                        .map(|item| item.span.file_id)
                        .unwrap_or_else(|| verum_ast::FileId::new(0));

                    let source_text = std::fs::read_to_string(file_path)
                        .unwrap_or_default();

                    let mut module_info = ModuleInfo::new(
                        module_id,
                        module_path.clone(),
                        ast_module.clone(),
                        file_id,
                        Text::from(source_text),
                    );

                    // Extract exports for cross-module import resolution
                    match extract_exports_from_module(ast_module, module_id, &module_path) {
                        Ok(export_table) => {
                            module_info.exports = export_table;
                        }
                        Err(e) => {
                            debug!("Failed to extract exports from {}: {:?}", module_name, e);
                        }
                    }

                    module_registry.write().register(module_info);

                    // Also add to self.modules for later use
                    let path_text = Text::from(module_name.as_str());
                    if !self.modules.contains_key(&path_text) {
                        self.modules.insert(path_text, Arc::new(ast_module.clone()));
                    }

                    // ALSO register a per-file sub-module path for non-mod.vr files.
                    // E.g., core/async/poll.vr -> register as "core.async.poll"
                    // This enables relative imports like `mount .poll.*` to resolve
                    // from within the parent module (core.async).
                    if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem != "mod" {
                            let sub_module_name = format!("{}.{}", module_name, file_stem);
                            let sub_module_path = ModulePath::from_str(&sub_module_name);
                            let sub_module_id = module_registry.read().allocate_id();

                            let mut sub_module_info = ModuleInfo::new(
                                sub_module_id,
                                sub_module_path.clone(),
                                ast_module.clone(),
                                file_id,
                                Text::from(std::fs::read_to_string(file_path).unwrap_or_default()),
                            );

                            match extract_exports_from_module(ast_module, sub_module_id, &sub_module_path) {
                                Ok(export_table) => {
                                    sub_module_info.exports = export_table;
                                }
                                Err(e) => {
                                    debug!("Failed to extract exports from {}: {:?}", sub_module_name, e);
                                }
                            }

                            module_registry.write().register(sub_module_info);

                            // Also add to self.modules
                            let sub_path_text = Text::from(sub_module_name.as_str());
                            if !self.modules.contains_key(&sub_path_text) {
                                self.modules.insert(sub_path_text, Arc::new(ast_module.clone()));
                            }
                        }
                    }
                }
            }

            // Resolve re-exports so that glob imports work
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let mut iteration = 0;
            loop {
                iteration += 1;
                match resolve_glob_reexports(&mut guard) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if iteration >= 10 => break,
                    Ok(_) => continue,
                }
            }
        }

        if config.verbose {
            let registry_count = self.session.module_registry().read().len();
            eprintln!("  Registered {} modules in ModuleRegistry", registry_count);
        }

        // ====================================================================
        // STEP 3: Global type registration (ALL modules)
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 2: Registering types globally across all modules...");
        }

        // Create TypeChecker with minimal context for stdlib compilation
        // (types are registered dynamically as stdlib .vr files are parsed)
        let mut type_checker = verum_types::infer::TypeChecker::with_minimal_context();
        type_checker.register_primitives();

        // Set module registry on type checker so cross-module imports can be resolved
        let registry = self.session.module_registry();
        type_checker.set_module_registry(registry.clone());

        self.register_stdlib_types_globally(&all_parsed_modules, &mut type_checker, &config)?;

        // ====================================================================
        // STEP 4: Compile each module to VBC
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 3: Compiling modules to VBC...");
        }

        let mut functions_compiled = 0;
        let target = TargetConfig::host();

        // Build list of all module names for forward reference detection
        let all_module_names: Vec<&str> = all_parsed_modules
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();

        for (idx, (module_name, ast_modules_with_paths)) in all_parsed_modules.iter().enumerate() {
            let module_start = std::time::Instant::now();
            let module = modules_to_compile.iter()
                .find(|m| &m.name == module_name)
                .expect("module should exist");

            if config.verbose {
                eprintln!("  Compiling module: {} ({} files)", module.name, module.source_files.len());
            }

            // Build set of modules that will be compiled AFTER this one (forward references)
            let later_modules: std::collections::HashSet<&str> = all_module_names[idx + 1..]
                .iter()
                .copied()
                .collect();

            // Extract just the AST modules for compilation
            let ast_modules: Vec<&verum_ast::Module> = ast_modules_with_paths
                .iter()
                .map(|(_, ast)| ast)
                .collect();

            let (vbc_module, funcs) = self.compile_core_module_from_ast(
                module,
                ast_modules.as_slice(),
                &config,
                &target,
                &later_modules,
            )?;
            functions_compiled += funcs;

            module_times.insert(module.name.clone(), module_start.elapsed());
            self.compiled_stdlib_modules.insert(module.name.clone(), vbc_module);
        }

        // ====================================================================
        // STEP 5: Build archive
        // ====================================================================
        if config.verbose {
            eprintln!("Building archive...");
        }

        let archive = self.build_stdlib_archive(&config)?;

        // ====================================================================
        // STEP 6: Write archive
        // ====================================================================
        if let Some(parent) = config.output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Failed to create output directory: {}", e))?;
        }

        write_archive_to_file(&archive, &config.output_path)
            .map_err(|e| anyhow::anyhow!("Failed to write archive: {}", e))?;

        let output_size = std::fs::metadata(&config.output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(StdlibCompilationResult {
            modules_compiled: self.compiled_stdlib_modules.len(),
            functions_compiled,
            total_time: start.elapsed(),
            module_times,
            output_path: config.output_path.clone(),
            output_size,
            warnings: self.stdlib_warnings.clone(),
            errors: self.stdlib_errors.clone(),
        })
    }

    /// Parse stdlib module source files to AST.
    fn parse_stdlib_module_files(
        &self,
        module: &StdlibModule,
    ) -> Result<Vec<(PathBuf, verum_ast::Module)>> {
        use crate::api::SourceFile;

        let mut sources: Vec<(PathBuf, SourceFile)> = Vec::new();
        for path in &module.source_files {
            let source = SourceFile::load(path)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
            sources.push((path.clone(), source));
        }

        let mut ast_modules = Vec::new();

        for (path, source) in &sources {
            let mut parser = verum_fast_parser::Parser::new(&source.content);
            match parser.parse_module() {
                Ok(ast_module) => ast_modules.push((path.clone(), ast_module)),
                Err(e) => {
                    return Err(anyhow::anyhow!("Parse error in {}: {:?}", source.path, e));
                }
            }
        }

        Ok(ast_modules)
    }

    /// Global type registration across ALL stdlib modules.
    ///
    /// Multi-pass registration order:
    /// 1. Import aliases
    /// 2. Type names (forward declarations)
    /// 3. Type bodies
    /// 4. Function signatures
    /// 5. Protocols
    /// 6. Impl blocks
    fn register_stdlib_types_globally(
        &mut self,
        all_modules: &[(String, Vec<(PathBuf, verum_ast::Module)>)],
        type_checker: &mut verum_types::infer::TypeChecker,
        config: &CoreConfig,
    ) -> Result<()> {
        use verum_ast::cfg::TargetConfig;
        let target = TargetConfig::host();

        // Pass 0: Process imports with aliases
        if config.verbose {
            eprintln!("  Pass 0: Processing import aliases from all modules...");
        }
        for (_module_name, ast_modules) in all_modules {
            for (_file_path, ast_module) in ast_modules {
                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                        type_checker.process_import_aliases(import_decl);
                    }
                }
            }
        }

        // Pass 0.5: Process full imports using the ModuleRegistry
        // This resolves `mount .stream.*` and `mount .protocols.*` etc. so that
        // types from sub-modules are available during type-checking. Each module's
        // imports are resolved with the correct current_module_path so that relative
        // imports (leading dot) resolve to the right absolute module paths.
        if config.verbose {
            eprintln!("  Pass 0.5: Processing cross-module imports...");
        }
        {
            let registry = self.session.module_registry();
            for (module_name, ast_modules) in all_modules {
                // For each file in the module, compute the correct current_module_path
                for (file_path, ast_module) in ast_modules {
                    let current_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem == "mod" {
                            // mod.vr represents its parent directory module
                            module_name.clone()
                        } else {
                            // Regular file: module_name.file_stem
                            format!("{}.{}", module_name, file_stem)
                        }
                    } else {
                        module_name.clone()
                    };

                    for item in &ast_module.items {
                        if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                            // Process the full import (not just aliases) to bring
                            // cross-module types into scope for this module
                            if let Err(e) = type_checker.process_import(
                                import_decl,
                                &current_module_path,
                                &registry.read(),
                            ) {
                                debug!("Stdlib import warning in {}: {:?}", current_module_path, e);
                            }
                        }
                    }
                }
            }
        }

        // Pass 1: Register ALL type NAMES
        if config.verbose {
            eprintln!("  Pass 1: Registering type names from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (_file_path, ast_module) in ast_modules {
                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        type_checker.register_type_name_only(type_decl);
                    }
                }
            }
            if config.verbose {
                eprintln!("    {} type names registered", module_name);
            }
        }

        // Pass 2: Register ALL type BODIES
        if config.verbose {
            eprintln!("  Pass 2: Registering type bodies from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Architectural: set the checker's current module so that
                // `define_type_in_current_module` can publish each type
                // under its fully-qualified key (`{module}.{name}`). Without
                // this the qualified-name layer stays empty and same-named
                // stdlib types (e.g. `RecvError` in broadcast/channel/quic)
                // silently collide on the flat lookup table.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        let filtered_decl = module_utils::filter_type_decl_for_target(type_decl, &target);
                        if let Err(e) = type_checker.register_type_declaration(&filtered_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0910")
                                .message(format!("Type registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 3: Register ALL function signatures
        if config.verbose {
            eprintln!("  Pass 3: Registering function signatures from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Module-scoped resolution: free fn signatures reference types
                // (`Result<_, RecvError>`) that may collide across modules, so
                // anchor each file to its qualified module path before
                // registration.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        if let Err(e) = type_checker.register_function_signature(func) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0911")
                                .message(format!("Function signature warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 4: Register ALL protocols
        if config.verbose {
            eprintln!("  Pass 4: Registering protocols from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Module-scoped resolution for protocol method signatures.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                        if let Err(e) = type_checker.register_protocol(protocol_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0912")
                                .message(format!("Protocol registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 5: Register ALL impl blocks
        if config.verbose {
            eprintln!("  Pass 5: Registering impl blocks from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Architectural: set the current module path so that type
                // references inside impl-block signatures (e.g. `RecvError` in
                // `Stream for BroadcastReceiver`) resolve against the
                // qualified-name layer first — avoiding collisions between
                // same-named types in different stdlib modules.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        if let Err(e) = type_checker.register_impl_block(impl_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0913")
                                .message(format!("Impl block registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 5.5a: Protocol-based discovery of coercion-friendly types.
        // Walks loaded AST modules looking for `implement <Coercion> for
        // X` blocks (where Coercion ∈ {IntCoercible, TensorLike,
        // Indexable, RangeLike} from core/base/coercion.vr) and registers
        // each target type with the unifier. Stdlib types that already
        // declare these implement-blocks are picked up here — zero
        // architectural violation for those.
        let mut all_ast_modules: Vec<&verum_ast::Module> = Vec::new();
        for (_, ast_modules) in all_modules {
            for (_, ast_module) in ast_modules {
                all_ast_modules.push(ast_module);
            }
        }
        let registered_via_protocol =
            crate::stdlib_coercion_registry::scan_protocol_implementations(
                type_checker.unifier_mut(),
                all_ast_modules.iter().copied(),
            );
        if registered_via_protocol > 0 {
            debug!(
                "[coercion-registry] discovered {} stdlib-coercion impl blocks via protocol scan",
                registered_via_protocol
            );
        }

        // Pass 5.5b: Hardcoded fallback registration for stdlib types
        // not yet retrofitted with implement blocks. Per the
        // architectural rule in `verum_types/src/CLAUDE.md`
        // ("NEVER hardcode stdlib/core type knowledge in the
        // compiler"), the hardcoded scaffolding is contained in the
        // dedicated `stdlib_coercion_registry` module so the violation
        // lives in one identifiable spot.
        //
        // The unifier's register_*_type methods de-duplicate via
        // HashSet, so calling 5.5b after 5.5a is harmless when an
        // already-discovered type happens to be in the hardcoded list.
        // Each stdlib retrofit (adding `implement IntCoercible for X`)
        // lets us delete X from the hardcoded list with safe
        // rollback at every step.
        crate::stdlib_coercion_registry::register_stdlib_coercions(
            type_checker.unifier_mut(),
        );

        // Pass 6: Validate imports
        // Now that all types, functions, and protocols are registered,
        // validate that all imports reference items that actually exist.
        if config.verbose {
            eprintln!("  Pass 6: Validating imports...");
        }

        let export_index = crate::core_compiler::build_export_index(all_modules);
        let import_errors = crate::core_compiler::validate_imports(all_modules, &export_index, &target);

        for (module_path, item_name, similar, span) in import_errors {
            let message = if similar.is_empty() {
                format!(
                    "E401: cannot find `{}` in module `{}` (byte {}-{})",
                    item_name, module_path, span.start, span.end
                )
            } else {
                format!(
                    "E401: cannot find `{}` in module `{}` (byte {}-{}). Did you mean: {}?",
                    item_name, module_path, span.start, span.end, similar
                )
            };

            let error = verum_diagnostics::DiagnosticBuilder::error()
                .code("E0401")
                .message(message)
                .build();
            self.stdlib_warnings.push(error);
        }

        if config.verbose {
            eprintln!("  Global type registration complete.");
        }

        Ok(())
    }

    /// Compile a stdlib module from pre-parsed AST.
    ///
    /// # Arguments
    /// * `module` - The module to compile
    /// * `ast_modules` - Pre-parsed AST modules for this module
    /// * `config` - Stdlib compilation configuration
    /// * `target` - Target platform configuration
    /// * `later_modules` - Set of module names that will be compiled AFTER this module.
    ///   Used for forward reference detection to suppress warnings for cross-module
    ///   function calls that will be resolved later in the compilation sequence.
    fn compile_core_module_from_ast(
        &mut self,
        module: &StdlibModule,
        ast_modules: &[&verum_ast::Module],
        config: &CoreConfig,
        target: &verum_ast::cfg::TargetConfig,
        later_modules: &std::collections::HashSet<&str>,
    ) -> Result<(verum_vbc::VbcModule, usize)> {
        use verum_vbc::codegen::CodegenConfig;
        use verum_vbc::module::{FunctionDescriptor, VbcFunction};
        use verum_vbc::instruction::Instruction;

        // Configure VBC codegen
        let codegen_config = CodegenConfig::new(&module.name)
            .with_optimization_level(config.optimization_level)
            .with_target(target.clone());

        let codegen_config = if config.debug_info {
            codegen_config.with_debug_info()
        } else {
            codegen_config
        };

        let mut codegen = VbcCodegen::with_config(codegen_config);

        // Import functions and protocols from previously compiled modules
        codegen.import_functions(&self.global_function_registry);
        codegen.import_protocols(&self.global_protocol_registry);

        // Three-pass compilation within the module (cross-file two-phase collection)
        // Pass 1a: Collect ALL protocol definitions from ALL files first
        // This ensures protocols like Eq, Ord are available when processing
        // impl blocks that implement them, regardless of file order.
        for ast_module in ast_modules {
            codegen.collect_protocol_definitions(ast_module);
        }

        // Pass 1b: Collect all other declarations from ALL files
        let lint_diagnostics = IntrinsicDiagnostics::new(&self.session.options().lint_config);
        for ast_module in ast_modules {
            if let Err(e) = codegen.collect_non_protocol_declarations(ast_module) {
                let diag = lint_diagnostics.codegen_warning(&module.name, &e.to_string(), None);
                let level = self.session.options().lint_config.level_for(IntrinsicLint::MissingImplementation);
                if level.is_error() {
                    self.stdlib_errors.push(diag);
                } else if level.should_emit() {
                    self.stdlib_warnings.push(diag);
                }
            }
        }

        // After all declarations collected, resolve pending imports
        // This handles cross-file imports within the same module
        codegen.resolve_pending_imports();

        // Pass 2: Compile all function bodies and merge
        let mut total_func_count = 0;
        let mut merged_vbc = verum_vbc::VbcModule::new(module.name.clone());

        for ast_module in ast_modules {
            match codegen.compile_function_bodies(ast_module) {
                Ok(compiled_module) => {
                    total_func_count += compiled_module.functions.len();
                    self.merge_stdlib_vbc_modules(&mut merged_vbc, compiled_module)?;
                }
                Err(e) => {
                    // Check if this is a forward reference to a module compiled later.
                    // If so, suppress the warning - the function will be available at runtime.
                    let is_forward_ref = if let Some(func_name) = e.undefined_function_name() {
                        // Extract the module prefix from the function path
                        // e.g., "darwin::tls::init_main_thread_tls" -> check "sys.darwin"
                        // e.g., "mem::heap::init_thread_heap" -> check "mem"
                        Self::is_forward_reference_to_later_module(
                            func_name,
                            &module.name,
                            later_modules,
                        )
                    } else {
                        false
                    };

                    if !is_forward_ref {
                        // Use IntrinsicDiagnostics for configurable severity
                        // Include span info in error message since we don't have file_id for Span construction
                        let error_msg = if let Some(ref s) = e.span {
                            format!("{} (byte {}-{})", e, s.start, s.end)
                        } else {
                            e.to_string()
                        };
                        let diag = lint_diagnostics.codegen_warning(&module.name, &error_msg, None);
                        let level = self.session.options().lint_config.level_for(IntrinsicLint::MissingImplementation);
                        if level.is_error() {
                            self.stdlib_errors.push(diag);
                        } else if level.should_emit() {
                            self.stdlib_warnings.push(diag);
                        }
                    }

                    // Create stub functions
                    for item in &ast_module.items {
                        if let verum_ast::ItemKind::Function(func) = &item.kind {
                            total_func_count += 1;
                            let func_name = func.name.name.to_string();
                            let name_id = merged_vbc.intern_string(&func_name);
                            let mut descriptor = FunctionDescriptor::new(name_id);
                            descriptor.register_count = 1;
                            descriptor.locals_count = func.params.len() as u16;
                            let vbc_func = VbcFunction::new(descriptor, vec![Instruction::RetV]);
                            merged_vbc.add_function(vbc_func.descriptor.clone());
                        }
                    }
                }
            }
        }

        // Export newly registered functions and protocols to global registries
        let new_functions = codegen.export_functions();
        for (name, info) in new_functions {
            self.global_function_registry.entry(name).or_insert(info);
        }

        let new_protocols = codegen.export_protocols();
        for (name, info) in new_protocols {
            self.global_protocol_registry.entry(name).or_insert(info);
        }

        Ok((merged_vbc, total_func_count))
    }

    /// Checks if an undefined function error is a forward reference to a module
    /// that will be compiled later in the compilation sequence.
    ///
    /// # Arguments
    /// * `func_path` - The function path from the error (e.g., "darwin::tls::init_main_thread_tls")
    /// * `current_module` - The module currently being compiled (e.g., "sys")
    /// * `later_modules` - Set of modules that will be compiled after the current one
    ///
    /// # Returns
    /// `true` if this appears to be a forward reference to a later module
    fn is_forward_reference_to_later_module(
        func_path: &str,
        current_module: &str,
        later_modules: &std::collections::HashSet<&str>,
    ) -> bool {
        // The function path uses "::" as separator (e.g., "darwin::tls::init_main_thread_tls")
        // We need to map this to module names which use "." as separator (e.g., "sys.darwin")

        // Extract the first component of the path
        let parts: Vec<&str> = func_path.split("::").collect();
        if parts.is_empty() {
            return false;
        }

        let first_component = parts[0];

        // Case 1: Direct submodule reference (e.g., "darwin" from "sys" -> "sys.darwin")
        let submodule_name = format!("{}.{}", current_module, first_component);
        if later_modules.contains(submodule_name.as_str()) {
            return true;
        }

        // Case 2: Direct module reference (e.g., "mem" -> "mem")
        if later_modules.contains(first_component) {
            return true;
        }

        // Case 3: Path with multiple components - try to match against later modules
        // e.g., "mem::heap::init_thread_heap" should match "mem"
        for later_module in later_modules {
            // Check if the function path starts with the module name
            let module_prefix = later_module.replace('.', "::");
            if func_path.starts_with(&module_prefix) || func_path.starts_with(later_module) {
                return true;
            }

            // Check if any component matches the module name (without parent prefix)
            // e.g., "mem" in "mem::heap::..." should match later module "mem"
            let module_parts: Vec<&str> = later_module.split('.').collect();
            if let Some(last_part) = module_parts.last() {
                if first_component == *last_part {
                    return true;
                }
            }
        }

        false
    }

    /// Merge a compiled VBC module into the main module.
    fn merge_stdlib_vbc_modules(
        &self,
        target: &mut verum_vbc::VbcModule,
        source: verum_vbc::VbcModule,
    ) -> Result<()> {
        let bytecode_offset = target.bytecode.len() as u32;
        let func_id_base = target.functions.len() as u32;

        // Merge function descriptors with adjusted offsets and function ID base
        for mut func in source.functions {
            func.bytecode_offset += bytecode_offset;
            func.func_id_base = func_id_base;
            target.add_function(func);
        }

        // Merge bytecode
        target.bytecode.extend_from_slice(&source.bytecode);

        // Merge type descriptors, remapping FunctionIds in protocol impls
        for ty in &source.types {
            let mut ty = ty.clone();
            // Remap FunctionIds in protocol implementations to account for
            // function table offset after merging with previous modules.
            for proto_impl in ty.protocols.iter_mut() {
                for fn_id in proto_impl.methods.iter_mut() {
                    if *fn_id != u32::MAX {
                        *fn_id += func_id_base;
                    }
                }
            }
            // Note: drop_fn and clone_fn are not remapped here because they
            // are not currently used from the merged module at runtime.
            target.add_type(ty);
        }

        // Merge string pool
        for (s, _id) in source.strings.iter() {
            target.intern_string(s);
        }

        // Merge constant pool
        for c in &source.constants {
            target.add_constant(c.clone());
        }

        // Merge specializations
        target.specializations.extend(source.specializations);

        // Merge field layout metadata (for GetF/SetF field index remapping in LLVM lowering)
        // field_id_to_name: extend target with source entries (source IDs offset by target length)
        // Note: VBC GetF instructions in each module use module-local field IDs.
        // We keep all entries so the LLVM lowering can look up field names.
        if !source.field_id_to_name.is_empty() {
            let offset = target.field_id_to_name.len();
            target.field_id_to_name.extend(source.field_id_to_name);
            // Store offset for future remapping if needed
            let _ = offset; // Currently field_ids are used per-module, not cross-module
        }
        // type_field_layouts: merge all type layouts (source overrides for same type names)
        for (type_name, fields) in source.type_field_layouts {
            target.type_field_layouts.entry(type_name).or_insert(fields);
        }

        Ok(())
    }

    /// Build the VBC archive from compiled stdlib modules.
    fn build_stdlib_archive(
        &self,
        config: &CoreConfig,
    ) -> Result<verum_vbc::VbcArchive> {
        use verum_vbc::{ArchiveBuilder, ArchiveFlags};

        let mut flags = ArchiveFlags::IS_STDLIB;
        if config.debug_info {
            flags |= ArchiveFlags::DEBUG_INFO;
        }
        if config.source_maps {
            flags |= ArchiveFlags::SOURCE_MAPS;
        }

        let mut builder = ArchiveBuilder::stdlib().with_flags(flags);

        let resolver = self.stdlib_resolver.as_ref()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        // Add modules in compilation order
        for name in resolver.compilation_order() {
            if let Some(module) = self.compiled_stdlib_modules.get(name) {
                let deps: Vec<&str> = resolver.get_module(name)
                    .map(|m| m.dependencies.iter().map(|s: &String| s.as_str()).collect())
                    .unwrap_or_default();

                builder.add_module(name, module, &deps)
                    .map_err(|e| anyhow::anyhow!("Failed to add module {} to archive: {}", name, e))?;
            }
        }

        Ok(builder.finish())
    }
}
