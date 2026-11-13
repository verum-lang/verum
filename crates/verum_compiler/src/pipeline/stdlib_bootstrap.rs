//! Stdlib (`core/`) bootstrap compilation — the `compile_core` orchestrator.
//!
//! Extracted from `pipeline.rs` (#106 Phase 8). This submodule
//! handles the StdlibBootstrap mode: a one-shot compile of the
//! `core/` standard library into the embeddable `stdlib.vbca`
//! archive that ships inside the verum binary.
//!
//! Flow:
//!
//!  1. Discover all stdlib modules via `StdlibModuleResolver`.
//!  2. Parse ALL modules to AST.
//!  3. Register ALL types globally (multi-pass across all modules).
//!  4. Compile each module to VBC bytecode.
//!  5. Build and write `stdlib.vbca` archive.
//!
//! Architectural distinction from `Normal` build mode: stdlib
//! bootstrap uses GLOBAL type registration across all modules
//! before compiling any module, eliminating cross-module
//! dependency constraints. User-code builds use per-file
//! incremental registration with the bootstrapped stdlib.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tracing::debug;

use verum_common::Text;
use verum_modules::{
    ModuleInfo, ModulePath, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};
use verum_vbc::codegen::{ForeignInterfaces, VbcCodegen};

use super::BuildMode;
use crate::core_compiler::{CoreConfig, StdlibCompilationResult, StdlibModule};
use crate::module_utils;

use super::CompilationPipeline;

/// A parsed source file carries its lexical owner through every bootstrap phase.
/// File-mount aliases are already complete owners, unlike directory groups.
struct OwnedSourceFile {
    path: PathBuf,
    owner: String,
    ast: verum_ast::Module,
}

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
    ///  .with_output("target/stdlib.vbca");
    ///
    /// let mut session = Session::default();
    /// let mut pipeline = CompilationPipeline::new_core(&mut session, config);
    ///
    /// let result = pipeline.compile_core()?;
    /// println!("Compiled {} modules with {} functions",
    ///  result.modules_compiled, result.functions_compiled);
    /// ```
    pub fn compile_core(&mut self) -> Result<StdlibCompilationResult> {
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
        let mut phase = self.session.measure_phase("Stdlib discovery");
        let mut module_times = std::collections::HashMap::new();

        // ====================================================================
        // STEP 1: Discover modules
        // ====================================================================
        if config.verbose {
            eprintln!(
                "Discovering stdlib modules in {}...",
                config.stdlib_path.display()
            );
        }

        let resolver = self
            .stdlib_resolver
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        resolver
            .discover()
            .map_err(|e| anyhow::anyhow!("Module discovery failed: {}", e))?;

        if config.verbose {
            eprintln!("Found {} modules", resolver.module_count());
        }

        // Get modules in dependency order
        let modules_to_compile: Vec<StdlibModule> =
            resolver.modules_in_order().into_iter().cloned().collect();
        phase.next("Stdlib parsing");

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

        let seeds = all_parsed_modules.iter().flat_map(|(_, files)| files);
        phase.next("Stdlib file mounts");
        let resolved = verum_modules::file_mount::resolve_file_mounts(
            &mut self.module_loader,
            seeds,
            |source| {
                let lexer = verum_lexer::Lexer::new(source.source.as_str(), source.file_id);
                verum_fast_parser::VerumParser::new()
                    .parse_module(lexer, source.file_id)
                    .map_err(|errors| verum_modules::error::ModuleError::Other {
                        message: Text::from(format!(
                            "parse error in file mount `{}`: {}",
                            source.file_path.display(),
                            errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "),
                        )),
                        span: None,
                    })
            },
        ).map_err(|error| anyhow::anyhow!("resolving stdlib file mounts: {error}"))?;
        let mut all_parsed_modules: Vec<(String, Vec<OwnedSourceFile>)> = all_parsed_modules
            .into_iter()
            .map(|(group, files)| {
                let owned = files
                    .into_iter()
                    .map(|(path, ast)| OwnedSourceFile {
                        owner: module_utils::source_file_module_path(&group, &path, &ast),
                        path,
                        ast,
                    })
                    .collect();
                (group, owned)
            })
            .collect();
        for entry in resolved {
            let owner = VbcCodegen::resolve_full_module_path(&entry.module, &entry.synthetic_name)
                .unwrap_or_else(|| entry.synthetic_name.clone());
            all_parsed_modules.push((entry.synthetic_name, vec![OwnedSourceFile {
                path: entry.absolute_path, owner, ast: entry.module,
            }]));
        }
        // Registration and checking share the target-selected source declarations.
        phase.next("Stdlib target selection");
        let target = self.session.cfg_evaluator().config().clone();
        let identities: Vec<_> = all_parsed_modules.iter().flat_map(|(_, files)| files)
            .map(|file| (file.owner.as_str(), file.path.to_string_lossy().into_owned(), &file.ast)).collect();
        let refs: Vec<_> = identities.iter().map(|(owner, path, ast)| (*owner, path.as_str(), *ast)).collect();
        let selected = crate::owned_source_semantics::select_source_roster(&refs, &target)?;
        phase.next("Stdlib source admission");
        let semantic = crate::phases::semantic_analysis::SemanticAnalysisPhase::new()
            .with_language_features(self.session.language_features())
            .with_target(target);
        semantic.admit_selected_sources(&selected, self.session)?;

        phase.next("Stdlib materialization");
        let mut selected: verum_common::Map<_, _> = selected.into_iter().map(|(owner, _, ast)| (owner, ast)).collect();
        for (_, files) in &mut all_parsed_modules {
            files.retain_mut(|file| {
                if let Some(ast) = selected.remove(&Text::from(file.owner.as_str())) {
                    file.ast = ast;
                    true
                } else { false }
            });
        }
        // Materialization follows successful original-source admission. Registry
        // and interface snapshots below must then use the transformed roster.
        // A single owned roster feeds declarations and bodies. @const results
        // are materialized here once, before either consumer observes the AST.
        let mut const_lowering = crate::meta::const_lowering::ConstLowering::new(
            self.fresh_meta_ctx_with_version_stamp(),
        );
        for (_, files) in &all_parsed_modules {
            for file in files { const_lowering.register(&file.owner, &file.ast)?; }
        }
        for (_, files) in &mut all_parsed_modules {
            for file in files {
                const_lowering.lower(&file.owner, &mut file.ast).map_err(|error| {
                    self.session.emit_diagnostic(verum_diagnostics::DiagnosticBuilder::error().message(error.to_string())
                        .span(self.session.convert_span(error.span)).build());
                    anyhow::anyhow!(error)
                })?;
            }
        }
        let source_names = module_utils::source_type_names(
            all_parsed_modules
                .iter()
                .flat_map(|(_, files)| files.iter().map(|file| (file.owner.as_str(), &file.ast))),
        );
        phase.next("Stdlib module registration");

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
                for OwnedSourceFile { path: file_path, owner: source_owner, ast: ast_module } in ast_modules_with_paths {
                    // Compute module path from module name + file
                    // Module names are already in dot-separated format (e.g., "core.base.primitives")
                    let module_path = ModulePath::from_str(&source_owner);
                    let module_id = module_registry.read().allocate_id();

                    let file_id = ast_module
                        .items
                        .first()
                        .map(|item| item.span.file_id)
                        .unwrap_or_else(|| verum_ast::FileId::new(0));

                    let source_text = std::fs::read_to_string(file_path).unwrap_or_default();

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
                    let path_text = Text::from(source_owner.as_str());
                    if !self.modules.contains_key(&path_text) {
                        self.modules.insert(path_text, Arc::new(ast_module.clone()));
                    }

                    // Inline modules belong to the declaring file, just as in
                    // ordinary project loading. Register them before computing
                    // the public reexport closure for bootstrap dependencies.
                    self.register_inline_modules(ast_module, &module_path)?;
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
        phase.next("Stdlib type registration");
        let mut type_checker = verum_types::infer::TypeChecker::with_minimal_context();
        type_checker.register_primitives();

        // Set module registry on type checker so cross-module imports can be resolved
        let registry = self.session.module_registry();
        type_checker.set_module_registry(registry.clone());

        self.register_stdlib_types_globally(&all_parsed_modules, &mut type_checker, &config)?;
        phase.next("Stdlib declaration preparation");

        // ====================================================================
        // STEP 3.5: Pre-register canonical-type inherent methods globally
        //          (task #16 stage-1 — Text/List/Map/Set/Deque/Heap/Shared/
        //           Mutex/RwLock/AtomicInt/AtomicBool/AtomicU64/BTreeMap/
        //           BTreeSet/Channel constructor + static method cascade)
        // ====================================================================
        //
        // The per-module VBC codegen at STEP 4 uses
        // `global_function_registry`, which is populated as modules
        // COMPLETE compilation (line ~1019: `entry().or_insert()`).
        // Caller-modules whose function bodies reference `Text.new` /
        // `List.from` / `AtomicInt.new` / etc. only see those names
        // if the producing module compiled FIRST.  When topological
        // order places `core.text` AFTER `core.diagnostics`, every
        // diagnostics body that calls `Text.new()` cascade-skips with
        // `[lenient] SKIP X.Y: undefined function: Text.new` — and the
        // panic-stub stuck in the archive permanently breaks any user
        // code reaching X.Y at runtime.
        //
        // Fresh May-11 precompile baseline: ~500 stdlib skips in this
        // class.  Text.new alone hits 104 functions; Text.from_str 61,
        // Text.from_bytes 56, Text.from 52, Text.from_utf8_* 83,
        // Text.with_capacity 20, AtomicInt.new 25, Mutex.new 15,
        // AtomicBool.new 13, List.from 33, RwLock.new 4, Text.builder 6,
        // Text.from_int 4, plus the async-runtime cluster
        // (oneshot 44, spawn_blocking 7, bounded_channel 6).
        //
        // Fix: scan every ast_module for `implement <CanonicalType> {
        // public fn <method>(...) }` blocks where CanonicalType is in
        // the closed canonical-stdlib set, and pre-register each
        // method as a stub `FunctionInfo` keyed under `<CanonicalType>.<method>`
        // (qualified) AND `<method>` (simple, first-wins).  The stub
        // carries arity (`param_count`) recovered from the AST and a
        // fresh sentinel FunctionId so it doesn't collide with the
        // sentinel-shared range used by record/newtype constructors.
        // When the producing module's full compile runs later, its
        // `register_function` re-registers the same name with the
        // proper FunctionId + body — `entry().or_insert()` discipline
        // at line 1019 means stubs are overlaid by real bodies, never
        // the other way around (real wins because `import_functions`
        // copies the global registry into each per-module ctx and the
        // per-module compile then re-registers under the same name).
        //
        // Closes the entire constructor-cascade cluster as one fix.
        // Variant-constructor cascade (AdMysql/AdPostgres/IoError) is
        // a separate class and tracked under task #16 stage-2.
        if config.verbose {
            eprintln!("Phase 2.5: Pre-registering canonical-type static methods globally...");
        }
        self.pre_register_canonical_static_methods(&all_parsed_modules)?;

        // Public free functions have source-qualified identities before any
        // body is compiled. Unique leaf names also receive a convenience alias.
        // Finalization emits descriptors only for referenced forward IDs;
        // archive loading resolves those descriptors to their defining bodies.
        if config.verbose {
            eprintln!("Phase 2.7: Pre-registering public free functions...");
        }
        self.pre_register_source_free_functions(&all_parsed_modules)?;

        // Phase 2.9 (ALIAS-INTRA-MODULE-ORDER-1 companion): globally
        // pre-register variant-form type aliases (`type IoError is
        // StreamError;`) from every module's AST BEFORE any per-module
        // compile. The per-module alias pre-pass lives in
        // `collect_all_declarations`, which the bootstrap path never
        // calls — so io/mod.vr's IoError alias was never collected,
        // never exported, and net's `IoError.from_os(e)` degraded to
        // an instance CallM (bakeAI [static-alias] alias_hit=false).
        // The declaring file owns both the alias scope and the qualified key.
        {
            let aliases_registered = source_names.aliases.len();
            for alias in &source_names.aliases {
                if let Ok(filter) = std::env::var("VERUM_TRACE_STUB")
                    && filter != "1"
                    && alias.local_name.contains(&filter)
                {
                    eprintln!(
                        "[stage-alias] '{}' -> '{}'",
                        alias.qualified_name, alias.target
                    );
                }
            }
            module_utils::merge_type_aliases(
                &mut self.global_type_alias_registry,
                source_names.aliases.iter().flat_map(|alias| {
                    [
                        (alias.qualified_name.clone(), alias.target.clone()),
                        (alias.local_name.clone(), alias.target.clone()),
                    ]
                }),
                &source_names.ambiguous,
            );
            if std::env::var("VERUM_TRACE_STUB").is_ok() {
                eprintln!(
                    "[stage-alias] pre-registered {} variant-form type aliases",
                    aliases_registered
                );
            }
        }

        // Phase 2.10 (T0723): globally pre-register every RECORD type's
        // positional field layout from the AST, before any module compiles.
        //
        // `global_type_layout_registry` is otherwise filled as modules
        // COMPLETE (STEP 4's `export_type_layouts` merge-back), so a module
        // that uses a record declared in a module compiled LATER sees no
        // layout for it. `resolve_field_index` then falls through to the
        // guess path, which scans every type in the program and picks the
        // most-fields candidate — the bake logs `'name' has 179
        // position-disagreeing candidates`, `'rows' ... guessed
        // CursorSlot.rows idx 7`. An index chosen that way is a foreign
        // slot, and for a record shorter than the guessed index it is out of
        // bounds. That is the FIELD-GUESS-HARD-1 class: 143 of them at the
        // time this pass was written.
        //
        // This is the layout analogue of Phase 2.8's const pre-pass and
        // Phase 2.9's alias pre-pass, both of which exist because the same
        // completion-ordered registry starved the same way. The map is a
        // pure name -> field-name list (no TypeId, no descriptor), which is
        // what makes an AST-only pre-pass safe: it cannot perturb TypeId
        // allocation or the descriptor tables.
        //
        // First-wins, matching the merge-back below. A record type is
        // declared in exactly one module, so a collision here means two
        // modules declare the same SIMPLE name — and for that case the
        // per-module descriptor path, which runs BEFORE the layout lookup
        // inside `resolve_field_index`, is the authority. The qualified key
        // is registered unconditionally because it cannot collide.
        {
            let field_renderer = VbcCodegen::new();
            let mut layouts_registered = 0usize;
            let mut simple_name_collisions = 0usize;
            for (module_name, ast_modules) in &all_parsed_modules {
                for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                    let source_module = source_owner;
                    let field_types = field_renderer.declared_field_type_names(ast_module, &source_module);
                    let source_prefix = format!("{source_module}.");
                    for (key, ty) in &field_types {
                        if key.0.starts_with(&source_prefix) {
                            self.global_type_field_name_registry
                                .entry(key.clone())
                                .or_insert_with(|| ty.clone());
                        }
                    }
                    for item in &ast_module.items {
                        let verum_ast::ItemKind::Type(td) = &item.kind else {
                            continue;
                        };
                        // A record-style VARIANT payload has the same
                        // positional-layout question as a record type, and
                        // starves the same way. The resolver looks such a
                        // payload up under both the qualified `Type.Variant`
                        // spelling and the bare variant name (the
                        // BARE-VARIANT-FIELD-1 arm), so both are registered.
                        let mut entries: Vec<(String, String, Vec<String>, Vec<String>)> = Vec::new();
                        let owner = td.name.name.to_string();
                        match &td.body {
                            verum_ast::decl::TypeDeclBody::Record(fields) if !fields.is_empty() => {
                                entries.push((
                                    owner.clone(),
                                    format!("{source_module}.{owner}"),
                                    fields.iter().map(|f| f.name.name.to_string()).collect(),
                                    fields
                                        .iter()
                                        .map(|f| {
                                            field_types[&(owner.clone(), f.name.name.to_string())]
                                                .clone()
                                        })
                                        .collect(),
                                ));
                            }
                            verum_ast::decl::TypeDeclBody::Variant(variants) => {
                                for v in variants.iter() {
                                    let verum_common::Maybe::Some(
                                        verum_ast::decl::VariantData::Record(fields),
                                    ) = &v.data
                                    else {
                                        continue;
                                    };
                                    if fields.is_empty() {
                                        continue;
                                    }
                                    let names: Vec<String> =
                                        fields.iter().map(|f| f.name.name.to_string()).collect();
                                    let types: Vec<String> = fields
                                        .iter()
                                        .map(|f| {
                                            field_types[&(
                                                format!("{owner}.{}", v.name.name),
                                                f.name.name.to_string(),
                                            )]
                                                .clone()
                                        })
                                        .collect();
                                    let vname = v.name.name.to_string();
                                    entries.push((
                                        format!("{owner}.{vname}"),
                                        format!("{source_module}.{owner}.{vname}"),
                                        names.clone(),
                                        types.clone(),
                                    ));
                                    // Bare variant name, first-wins. Genuinely
                                    // ambiguous across types (the
                                    // `AllocationFailed { code }` vs
                                    // `{ code, size }` sibling class), which is
                                    // why the loser is counted rather than
                                    // overwritten — and why the qualified keys
                                    // above carry the authority.
                                    entries.push((vname, String::new(), names, types));
                                }
                            }
                            _ => continue,
                        }
                        for (simple, qualified, names, types) in entries {
                            if !qualified.is_empty() {
                                self.global_type_layout_registry
                                    .entry(qualified.clone())
                                    .or_insert_with(|| names.clone());
                                for (field, ty) in names.iter().zip(&types) {
                                    self.global_type_field_name_registry
                                        .entry((qualified.clone(), field.clone()))
                                        .or_insert_with(|| ty.clone());
                                }
                            }
                            match self.global_type_layout_registry.entry(simple.clone()) {
                                std::collections::hash_map::Entry::Occupied(_) => {
                                    simple_name_collisions += 1;
                                }
                                std::collections::hash_map::Entry::Vacant(v) => {
                                    // Bare layout and field types must have the same
                                    // owner when two modules declare the same leaf.
                                    for (field, ty) in names.iter().zip(&types) {
                                        self.global_type_field_name_registry
                                            .insert((simple.clone(), field.clone()), ty.clone());
                                    }
                                    v.insert(names);
                                    layouts_registered += 1;
                                }
                            }
                        }
                    }
                }
            }
            if config.verbose || std::env::var("VERUM_TRACE_STUB").is_ok() {
                eprintln!(
                    "[stage-layout] pre-registered {layouts_registered} record layouts \
                         ({simple_name_collisions} simple-name collisions kept under their \
                         qualified key only)"
                );
            }
        }

        // ====================================================================
        // STEP 4: Compile each module to VBC
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 3: Compiling modules to VBC...");
        }

        let mut functions_compiled = 0;
        // Session config, not host — see the note at
        // `register_stdlib_types_globally`. `TargetConfig::host()` carries
        // the PRECOMPILER's own `cfg!(debug_assertions)`, false in the
        // release binary that bakes the archive (T0758).
        let target = self.session.cfg_evaluator().config().clone();

        // T1192 — SIMPLE NAMES DECLARED BY MORE THAN ONE MODULE, computed
        // once so the transparency seed below can refuse them.
        //
        // The seed is keyed on the SIMPLE name because codegen's
        // `newtype_names` cache is, and T1108 measured what a shared name
        // costs there: a record named `NtStatus` and a two-element tuple
        // named `Handle` inherited a stdlib newtype's flag and returned the
        // RECEIVER'S ADDRESS — two of five consumers wrong. The repair for
        // that ("a declaration takes its own name back",
        // `compile_type_decl`) is USER-PHASE ONLY, so during the bake
        // nothing would undo a wrong seed. Refusing ambiguous names is
        // therefore not caution, it is the only place the discipline can
        // live on this path.
        {
            self.ambiguous_core_type_names
                .extend(source_names.ambiguous);
            if config.verbose {
                eprintln!(
                    "  transparency seed: {} simple type name(s) declared by more than \
one module and therefore never seeded",
                    self.ambiguous_core_type_names.len()
                );
            }
        }

        // Cyclic core dependencies need declaration-owned signatures before a
        // provider's body exists. These modules are metadata only: scoped type
        // IDs pass through the normal importer, never into executable archives.
        phase.next("Stdlib declaration interfaces");
        let declaration_interfaces = self.collect_core_declaration_interfaces(
            &all_parsed_modules, &config, &target,
        )?;

        phase.next("Stdlib body compilation");
        for (module_name, ast_modules_with_paths) in &all_parsed_modules {
            let module_start = std::time::Instant::now();
            let mounted_module = StdlibModule {
                name: module_name.clone(),
                source_files: ast_modules_with_paths.iter().map(|file| file.path.clone()).collect(),
                dependencies: Vec::new(),
            };
            let module = modules_to_compile.iter().find(|m| &m.name == module_name)
                .unwrap_or(&mounted_module);

            if config.verbose {
                eprintln!(
                    "  Compiling module: {} ({} files)",
                    module.name,
                    module.source_files.len()
                );
            }

            let ast_modules: Vec<_> = ast_modules_with_paths.iter()
                .map(|file| (file.owner.as_str(), &file.ast)).collect();

            let (vbc_module, funcs) = self.compile_core_module_from_ast(
                module,
                ast_modules.as_slice(),
                &config,
                &target,
                &declaration_interfaces,
            )?;
            functions_compiled += funcs;

            // [diag] VERUM_DUMP_FN=<substring>: print the disassembly
            // of every just-compiled function whose header matches —
            // the ONLY way to inspect the exact baked bytecode of a
            // stdlib body (`--emit-vbc` covers user modules only).
            // First consumer: B1 triage (baked safe_socket writes
            // field idx 4 into an 8-byte type_id=0 object while the
            // same source shape compiled user-side runs green).
            if let Ok(filter) = std::env::var("VERUM_DUMP_FN")
                && !filter.is_empty()
            {
                let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
                let mut printing = false;
                for line in dump.lines() {
                    // Header shape: `; fn name(...) -> Ret  [id=..]`
                    let is_fn_header = line.starts_with("; fn ");
                    if is_fn_header {
                        printing = line.contains(&filter);
                        if printing {
                            eprintln!("[dump-fn] module={}", module.name);
                        }
                    }
                    if printing {
                        eprintln!("{}", line);
                    }
                }
            }

            let duration = module_start.elapsed();
            module_times.insert(module.name.clone(), duration);
            self.session.record_module_metrics(&module.name, duration, funcs);
            self.compiled_stdlib_modules
                .insert(module.name.clone(), vbc_module);
        }

        // ====================================================================
        // STEP 5: Build archive
        // ====================================================================
        if config.verbose {
            eprintln!("Building archive...");
        }

        phase.next("Stdlib archive assembly");
        let archive = self.build_stdlib_archive(&config)?;
        phase.next("Stdlib archive serialization");

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

        drop(phase);
        let total_time = start.elapsed();
        let mut profile = self.session.current_metrics();
        profile.total_duration = total_time;
        profile.finalize();
        Ok(StdlibCompilationResult {
            modules_compiled: self.compiled_stdlib_modules.len(),
            functions_compiled,
            total_time,
            profile,
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

        module.source_files.iter().map(|path| {
            let source = SourceFile::load(path)
                .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            let file_id = self.session.load_source_string(&source.content, path.clone())?;
            let ast = verum_fast_parser::VerumParser::new()
                .parse_module_str(&source.content, file_id)
                .map_err(|e| anyhow::anyhow!("parsing {}: {e:?}", path.display()))?;
            Ok((path.clone(), ast))
        }).collect()
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
    /// Task #16 stage-1: pre-register inherent methods of canonical
    /// stdlib types (Text, List, Map, Set, Deque, Heap, Shared,
    /// Mutex, RwLock, AtomicInt, AtomicBool, AtomicU64, BTreeMap,
    /// BTreeSet, Channel) into `global_function_registry` BEFORE any
    /// per-module compile pass runs.
    ///
    /// Eliminates the Text.new / List.from / AtomicInt.new / etc.
    /// lenient-skip cascade (~500 entries at May-11 baseline) caused
    /// by per-module codegen running before the producing module's
    /// `register_function` has populated the global registry.
    ///
    /// Each stub `FunctionInfo` carries:
    ///   * a fresh `next_func_id`-allocated `FunctionId` (NOT the
    ///     sentinel range used by record/newtype/variant constructors —
    ///     real bodies replace this id when the producing module
    ///     compiles; see `register_function`'s overwrite semantics at
    ///     `verum_vbc::codegen::context::CodegenContext::register_function`).
    ///   * `param_count` recovered from the AST (so call-site arity
    ///     checks pass even before the body lands).
    ///   * `parent_type_name = Some(<CanonicalType>)` so the variant /
    ///     receiver-type fallbacks in compile_method_call work.
    ///   * `is_const = false` (these are functions, not consts).
    ///   * `is_transparent_wrapper = false` (these are real bodies,
    ///     not synthetic newtype constructors).
    ///
    /// Public inherent methods retain their declaring source module. Only
    /// canonical prelude types additionally receive a legacy short alias;
    /// other declarations cannot claim those aliases by registration order.
    fn pre_register_canonical_static_methods(
        &mut self,
        all_modules: &[(String, Vec<OwnedSourceFile>)],
    ) -> Result<()> {
        use verum_ast::decl::{ImplItemKind, ImplKind, Visibility};
        use verum_ast::ItemKind;
        use verum_vbc::codegen::FunctionInfo;
        use verum_vbc::module::FunctionId;

        // Existing short aliases for the prelude. Qualified declarations
        // are registered independently for every source-owned type.
        fn is_canonical_stdlib_type(name: &str) -> bool {
            matches!(
                name,
                // Text + collection constructors
                "Text" | "List" | "Map" | "Set" | "Deque" | "BTreeMap" | "BTreeSet"
                | "BinaryHeap"
                // Memory wrappers
                | "Heap" | "Shared" | "Weak" | "Cow" | "Pin"
                // Concurrency primitives
                | "Mutex" | "RwLock" | "AtomicInt" | "AtomicBool" | "AtomicU64"
                | "AtomicI32" | "AtomicU32" | "Channel" | "Semaphore"
                // Time
                | "Duration" | "Instant"
                // Number wrappers
                | "Maybe" | "Result"
                // Primitive receivers (`implement Int` and family in
                // core/base/primitives.vr + Float32/Ordering). These were
                // MISSING from the set, so every `x.checked_add(y)` /
                // `x.cmp(y)` compiled INSIDE the bake (before the base
                // module's real registration) could not resolve
                // "Int.checked_add" and its return type: the scrutinee of
                // `match self.used.checked_add(mask)` derived None, the
                // match binding went untyped, and the baked body carried a
                // BARE CallM — which Tier 0 dispatches by NaN-box tag but
                // AOT cannot dispatch at all (the checked_add
                // tier-divergence in GenerationalArena.alloc, and the wide
                // bare-CallM surface across baked bodies). The predicate
                // recognised `implement Int` (dedicated TypeKind arm
                // above) and then rejected it HERE — a closed set treated
                // as complete while a whole family was absent.
                | "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
                | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
                | "ISize" | "USize" | "Byte" | "Float" | "Float32"
                | "Bool" | "Char" | "Ordering"
            )
        }

        let mut stubs_registered: usize = 0;
        for (_module_name, ast_modules) in all_modules {
            // Resolve the owner's declaration inside this archive module. A
            // duplicate nominal spelling is deliberately unknown, never a
            // sibling declaration selected by insertion order.
            let mut owner_generics: std::collections::HashMap<String, Option<Vec<String>>> = std::collections::HashMap::new();
            for OwnedSourceFile { owner, ast, .. } in ast_modules {
                for item in &ast.items {
                    if let ItemKind::Type(decl) = &item.kind {
                        let names = decl.generics.iter().filter_map(|p| match &p.kind {
                            verum_ast::ty::GenericParamKind::Type { name, .. }
                            | verum_ast::ty::GenericParamKind::Const { name, .. }
                            | verum_ast::ty::GenericParamKind::Meta { name, .. } => Some(name.name.to_string()),
                            _ => None,
                        }).collect();
                        owner_generics.entry(verum_vbc::module::qualify_module_name(owner, decl.name.name.as_str())).and_modify(|v| *v = None).or_insert(Some(names));
                    }
                }
            }
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                for item in ast_module.items.iter() {
                    let impl_decl = match &item.kind {
                        ItemKind::Impl(impl_decl) => impl_decl,
                        _ => continue,
                    };
                    // Only handle inherent impls (not protocol impls);
                    // protocol-impl methods are registered through a
                    // separate path that doesn't suffer the same
                    // ordering hazard.
                    let target_type_name: String = match &impl_decl.kind {
                        ImplKind::Inherent(ty) => match &ty.kind {
                            verum_ast::ty::TypeKind::Path(path) => {
                                if let Some(ident) = path.as_ident() {
                                    ident.name.to_string()
                                } else if let Some(seg) = path.segments.last() {
                                    match seg {
                                        verum_ast::ty::PathSegment::Name(id) => id.name.to_string(),
                                        _ => continue,
                                    }
                                } else {
                                    continue;
                                }
                            }
                            verum_ast::ty::TypeKind::Generic { base, .. } => {
                                if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                    if let Some(ident) = path.as_ident() {
                                        ident.name.to_string()
                                    } else {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            }
                            // Builtin nominal types have DEDICATED
                            // TypeKind variants — `implement Text`
                            // parses as Inherent(TypeKind::Text) and
                            // never reaches the Path arm above, so
                            // stage-1 registered ZERO Text.* stubs
                            // (bakeAA trace: 694 stubs, none matching
                            // "Text.") and every pre-`text`-module
                            // consumer of Text.new / Text.from_bytes /
                            // … lenient-stubbed (class-(a) root #1).
                            verum_ast::ty::TypeKind::Text => "Text".to_string(),
                            verum_ast::ty::TypeKind::Bool => "Bool".to_string(),
                            verum_ast::ty::TypeKind::Int => "Int".to_string(),
                            verum_ast::ty::TypeKind::Float => "Float".to_string(),
                            verum_ast::ty::TypeKind::Char => "Char".to_string(),
                            _ => continue,
                        },
                        _ => continue,
                    };
                    // ASSOCIATED CONSTANTS, EVERY TYPE, QUALIFIED KEY ONLY
                    // (T1277). This runs BEFORE the canonical-set guard
                    // below, and deliberately so.
                    //
                    // `ImplItemKind::Const` is a SEPARATE arm from
                    // `Function` (verum_ast decl.rs), and the item filter
                    // beneath handles only `Function`. So no associated
                    // constant has ever been pre-registered — and the
                    // canonical set would have excluded these types even
                    // if it had been.
                    //
                    // Measured 2026-09-09 on an unmodified tree: 50 of
                    // the 64 `[T0545]` rung-4 fabrications
                    // (codegen/expressions.rs:22971) are associated
                    // constants of non-canonical types read from a
                    // consumer compiled BEFORE the declarer —
                    // `Style.DEFAULT` x32, `Modifier.*` x12,
                    // `TransportErrorCode.*` x6. Rung 4 emits
                    // `MakeVariant(intern("Style.DEFAULT"))` where a Call
                    // belongs, so the value that reaches the program is a
                    // string id. `RenderCell.EMPTY` is not even visible
                    // as a member from a user program.
                    //
                    // The sibling correlate that gave it away: a consumer
                    // in the DECLARER'S OWN parent module resolves
                    // (core.term.style.style reading Modifier.NONE from
                    // its sibling core.term.style.modifier), and every
                    // consumer outside it fabricates — including eighteen
                    // files that already mount the declaring module
                    // directly. Mount spelling is not the axis; compile
                    // ORDER is, and this pre-pass is what makes order
                    // irrelevant for the names it covers.
                    //
                    // Only the full source owner distinguishes same-named
                    // nominal types. Prelude aliases are convenience entries.
                    for impl_item in impl_decl.items.iter() {
                        let (const_name, const_ty) = match &impl_item.kind {
                            ImplItemKind::Const { name, ty, .. } => (name, ty),
                            _ => continue,
                        };
                        let short = format!("{}.{}", target_type_name, const_name.name);
                        let qualified = verum_vbc::module::qualify_module_name(source_owner, &short);
                        if self.global_function_registry.contains_key(&qualified) {
                            continue;
                        }
                        let const_return_type_name = {
                            let n = verum_vbc::codegen::VbcCodegen::extract_type_name_from_ast(
                                const_ty,
                            );
                            if n.is_empty() || n == "()" {
                                None
                            } else if n == "Self" {
                                Some(target_type_name.clone())
                            } else {
                                Some(n)
                            }
                        };
                        let stub_id = FunctionId(
                            verum_vbc::stub_ranges::STAGE1_BASE - stubs_registered as u32,
                        );
                        let info = FunctionInfo {
                            has_source_body: false,
                            foreign_declaration: None,
                            callable_signature: None,
                            type_param_ids: Vec::new(),
                            explicit_type_param_ids: Vec::new(),
                            id: stub_id,
                            param_count: 0,
                            param_names: vec![],
                            param_type_names: vec![],
                            is_async: false,
                            is_generator: false,
                            contexts: vec![],
                            return_type: None,
                            yield_type: None,
                            intrinsic_name: None,
                            variant_tag: None,
                            parent_type_name: Some(target_type_name.clone()),
                            variant_payload_types: None,
                            is_partial_pattern: false,
                            takes_self_mut_ref: false,
                            return_type_name: const_return_type_name,
                            return_type_inner: None,
                            is_const: true,
                            is_transparent_wrapper: false,
                            param_closure_return_type_names: Vec::new(),
                        };
                        if let Ok(filter) = std::env::var("VERUM_TRACE_STUB")
                            && filter != "1"
                            && qualified.contains(&filter)
                        {
                            eprintln!(
                                "[stage-1const] stub '{}' id={}",
                                qualified, stub_id.0
                            );
                        }
                        if is_canonical_stdlib_type(&target_type_name) {
                            self.global_function_registry.entry(short).or_insert_with(|| info.clone());
                        }
                        self.global_function_registry.insert(qualified, info);
                        stubs_registered += 1;
                    }
                    // Every source-owned method needs a forward declaration.
                    // Only the canonical prelude set also receives a legacy
                    // short alias; unrelated nominal types stay qualified.
                    // Walk the impl block's items, register each
                    // `public fn` as a stub.
                    for impl_item in impl_decl.items.iter() {
                        let func = match &impl_item.kind {
                            ImplItemKind::Function(f) => f,
                            _ => continue,
                        };
                        if !matches!(func.visibility, Visibility::Public) {
                            continue;
                        }
                        let method_name = func.name.name.to_string();
                        let qualified = format!("{}.{}", target_type_name, method_name);
                        let source_qualified = verum_vbc::module::qualify_module_name(
                            &source_owner, &qualified,
                        );
                        if self.global_function_registry.contains_key(&source_qualified) {
                            continue;
                        }
                        // Assign a fresh per-stub FunctionId from a
                        // dedicated high sentinel range
                        // (`u32::MAX - 0x40_0000..` — well above the
                        // variant-tag sentinel descent `u32::MAX -
                        // tag` and below `u32::MAX / 2` newtype-pass-
                        // through).  Real bodies registered by the
                        // producing module's `register_function`
                        // overwrite this id by virtue of inserting
                        // under the same qualified name (the per-
                        // module ctx's `register_function` allows
                        // overwrite when the existing entry is a stub
                        // — sentinel-id detection at the call site).
                        let stub_id =
                            FunctionId(verum_vbc::stub_ranges::STAGE1_BASE - stubs_registered as u32);
                        let param_count = func.params.len();
                        let param_names: Vec<String> = (0..param_count)
                            .map(|i| format!("_arg{}", i))
                            .collect();
                        // **SIGNATURE-PREPASS-1**: carry the declared
                        // return type on the stub.  The stub exists
                        // precisely so ORDER-INDEPENDENT lookups
                        // resolve (`lookup_function("List.drain")`
                        // during `core.mem` compilation, which runs
                        // BEFORE `core.collections` in the hardcoded
                        // dependency graph) — but a `return_type_name:
                        // None` stub SATISFIES the lookup while
                        // starving every return-type consumer: the
                        // for-in classifier `is_custom_iterator_type`
                        // saw None for `self.retired.drain(…)` and
                        // lowered the `Drain<T>` protocol iterator
                        // through native IterNew (the hazard/reclaim
                        // SIGSEGV class, PROTOCOL-ITER-1).  Rendering
                        // via the codegen's own canonical extractor
                        // keeps the two views drift-free.
                        let stub_return_type_name = match &func.return_type {
                            verum_common::Maybe::Some(ty) => {
                                let n =
                                    verum_vbc::codegen::VbcCodegen::extract_type_name_from_ast(ty);
                                if n.is_empty() || n == "()" {
                                    None
                                } else if n == "Self" {
                                    // `fn clone(&self) -> Self` etc. —
                                    // bind to the impl target so
                                    // consumers see the concrete type.
                                    Some(target_type_name.clone())
                                } else {
                                    Some(n)
                                }
                            }
                            verum_common::Maybe::None => None,
                        };
                        // SELF-MUT-STUB-CARRY-1: the AST fn is IN HAND here —
                        // derive the `&mut self`-receiver fact instead of
                        // hardcoding `false`.  A stage-1 stub is the
                        // first-wins registry entry for every canonical
                        // stdlib method in prelude-only compiles (no
                        // explicit `mount` of the owner type), so a `false`
                        // here made the call-site skip the RefMut receiver
                        // wrap: `c.make_ascii_uppercase()` passed `c` BY
                        // VALUE, the baked body's `DEREF_MUT` wrote into the
                        // callee-frame copy, and every `&mut self` stdlib
                        // method was a silent no-op — but ONLY in files
                        // without the owner-type mount (mount-set-dependent
                        // semantics, the worst kind).
                        let stub_takes_self_mut_ref = matches!(
                            func.params.first().map(|p| &p.kind),
                            Some(
                                verum_ast::decl::FunctionParamKind::SelfRefMut
                                    | verum_ast::decl::FunctionParamKind::SelfRefCheckedMut
                                    | verum_ast::decl::FunctionParamKind::SelfRefUnsafeMut
                            )
                        );
                        let parent_names = owner_generics.get(&verum_vbc::module::qualify_module_name(source_owner, &target_type_name)).and_then(|v| v.as_deref()).unwrap_or(&[]);
                        let impl_names: Vec<String> = if impl_decl.generics.is_empty() {
                            verum_vbc::codegen::VbcCodegen::impl_target_arg_names(&impl_decl.kind)
                                .into_iter().filter(|name| parent_names.contains(name)).collect()
                        } else {
                            impl_decl.generics.iter().filter_map(|p| match &p.kind {
                                verum_ast::ty::GenericParamKind::Type { name, .. }
                                | verum_ast::ty::GenericParamKind::Const { name, .. }
                                | verum_ast::ty::GenericParamKind::Meta { name, .. } => Some(name.name.to_string()),
                                _ => None,
                            }).collect()
                        };
                        let info = FunctionInfo {
                            has_source_body: func.body.is_some(),
                            foreign_declaration: None,
                            callable_signature: None,
                            type_param_ids: verum_vbc::codegen::VbcCodegen::declared_type_param_ids(func, parent_names, &impl_names),
                            explicit_type_param_ids: verum_vbc::codegen::VbcCodegen::declared_explicit_type_param_ids(func, parent_names, &impl_names),
                            id: stub_id,
                            param_count,
                            param_names,
                            param_type_names: vec![],
                            is_async: func.is_async,
                            is_generator: false,
                            contexts: vec![],
                            return_type: None,
                            yield_type: None,
                            intrinsic_name: None,
                            variant_tag: None,
                            parent_type_name: Some(target_type_name.clone()),
                            variant_payload_types: None,
                            is_partial_pattern: false,
                            takes_self_mut_ref: stub_takes_self_mut_ref,
                            return_type_name: stub_return_type_name,
                            return_type_inner: None,
                            is_const: false,
                            is_transparent_wrapper: false,
                            param_closure_return_type_names: Vec::new(),
                        };
                        // [diag] VERUM_TRACE_STUB=<substring> logs each
                        // stage-1 registration by qualified name — the
                        // count-only summary below can't answer "did
                        // <Type>.<method> get a stub?" during class-(a)
                        // triage.
                        if let Ok(filter) = std::env::var("VERUM_TRACE_STUB")
                            && filter != "1"
                            && qualified.contains(&filter)
                        {
                            eprintln!(
                                "[stage-1] stub '{}' id={} arity={}",
                                qualified, stub_id.0, param_count
                            );
                        }
                        self.global_function_registry
                            .insert(source_qualified, info.clone());
                        if is_canonical_stdlib_type(target_type_name.as_str()) {
                            self.global_function_registry.entry(qualified).or_insert(info);
                        }
                        stubs_registered += 1;
                    }
                }
            }
        }
        if std::env::var("VERUM_TRACE_STUB").is_ok() {
            eprintln!(
                "[task #16 stage-1] pre-registered {} source-qualified static-method stubs",
                stubs_registered
            );
        }
        Ok(())
    }

    /// Elaborate one source-owned declaration graph. The shared unit collector
    /// claims every nominal identity before resolving signatures, so cyclic
    /// references never require provisional ABI descriptors or unknown types.
    fn collect_core_declaration_interfaces(
        &mut self,
        all_modules: &[(String, Vec<OwnedSourceFile>)],
        config: &CoreConfig,
        target: &verum_ast::cfg::TargetConfig,
    ) -> Result<Vec<verum_vbc::VbcModule>> {
        let mut foreign = ForeignInterfaces::default();
        let mut source_exports = verum_vbc::codegen::SourceExports::default();
        let mut next_variant = verum_vbc::stub_ranges::STAGE2_BASE;
        let mut declared_variants = std::collections::HashSet::new();
        let mut next_method = self.global_function_registry.values()
            .map(|info| info.id.0)
            .filter(|id| verum_vbc::stub_ranges::in_stage1(*id))
            .min().map_or(verum_vbc::stub_ranges::STAGE1_BASE, |id| id - 1);
        let mut declarations = VbcCodegen::with_config(
            verum_vbc::codegen::CodegenConfig::new("core")
                .with_optimization_level(config.optimization_level)
                .with_target(target.clone()),
        );
        let sources: Vec<_> = all_modules
            .iter()
            .flat_map(|(_, files)| files.iter().map(|file| (file.owner.as_str(), &file.ast)))
            .collect();
        let interface = declarations.collect_owned_declaration_interface(&sources)
            .map_err(|error| anyhow::anyhow!("collecting stdlib declaration graph: {error}"))?;
        // All units elaborate against the same selected protocol declarations.
        // Generic headers and inherited bodies can be used before their owner
        // reaches the executable compilation pass.
        self.global_protocol_registry.extend(declarations.export_protocols());
        declarations.export_foreign_interfaces(&interface, &mut foreign);
        // The declaring compiler selects the method (including protocol
        // implementations). Consumer-pool signatures come from the interface
        // importer, never the declaring compiler's numeric type IDs.
        let exports = declarations.export_function_view();
        for (name, info) in &exports {
            if info.foreign_declaration.as_deref().and_then(|key| key.rsplit_once('#'))
                .is_some_and(|(path, _)| path == name.as_str())
            {
                self.global_function_registry.insert(name.to_string(), (*info).clone());
            }
        }
        for method in &interface.functions {
            if method.parent_type.is_none() || !method.has_source_body { continue; }
            let Some(owner) = method.origin_module.and_then(|id| interface.strings.get(id)) else { continue };
            let Some(leaf) = interface.strings.get(method.name) else { continue };
            let key = verum_vbc::module::qualify_module_name(owner, leaf);
            let Some(&source) = exports.get(&Text::from(key.as_str())) else { continue };
            if !source.has_source_body || self.global_function_registry.contains_key(&key) { continue; }
            anyhow::ensure!(verum_vbc::stub_ranges::in_stage1(next_method), "forward method ID range exhausted");
            let mut forward = source.clone();
            forward.id = verum_vbc::module::FunctionId(next_method);
            forward.callable_signature = None;
            forward.return_type = None;
            forward.yield_type = None;
            self.global_function_registry.insert(key, forward);
            next_method -= 1;
        }
        for (canonical, declaration) in declarations.declared_variant_constructors() {
            let (owner, variant) = canonical.rsplit_once('.').expect("qualified constructor");
            if !declared_variants.insert(variant.to_owned()) {
                source_exports.mark_ambiguous_variant(variant);
            }
            let parent = owner.rsplit('.').next().unwrap_or(owner);
            anyhow::ensure!(verum_vbc::stub_ranges::in_stage2(next_variant), "forward constructor ID range exhausted");
            let mut forward = declaration.clone();
            forward.id = verum_vbc::module::FunctionId(next_variant);
            next_variant -= 1;
            self.global_function_registry.insert(canonical.to_owned(), forward.clone());
            forward.parent_type_name = Some(parent.to_owned());
            forward.return_type_name = Some(parent.to_owned());
            self.global_function_registry.entry(format!("{parent}.{variant}"))
                .or_insert_with(|| forward.clone());
            self.global_function_registry.entry(variant.to_owned()).or_insert(forward);
        }
        // Constants use the same source graph as callable and nominal
        // interfaces. A homonymous declaration in another platform does not
        // remove this owner's identity or supply its value.
        for (index, (canonical, mut info)) in declarations.export_constant_declarations()?.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| anyhow::anyhow!("constant declaration count exceeds VBC IDs"))?;
            let id = verum_vbc::stub_ranges::STAGE4_BASE.checked_sub(index)
                .filter(|id| verum_vbc::stub_ranges::in_stage4(*id))
                .ok_or_else(|| anyhow::anyhow!("forward constant ID range exhausted"))?;
            info.id = verum_vbc::module::FunctionId(id);
            info.callable_signature = None;
            // Nominal IDs belong to the declaration interface's pool. Its
            // importer remaps them; this name registry carries only builtins.
            if !matches!(info.return_type, Some(verum_vbc::types::TypeRef::Concrete(id)) if id.is_builtin()) {
                info.return_type = None;
            }
            info.yield_type = None;
            self.global_function_registry.insert(canonical.to_string(), info);
        }
        declarations.publish_source_exports(&mut source_exports);
        self.global_foreign_interfaces = Arc::new(foreign);
        self.global_source_exports = Arc::new(source_exports);
        Ok(vec![interface])
    }

    fn import_core_nominal_dependencies(
        &self,
        codegen: &mut VbcCodegen,
        sources: &[(&str, &verum_ast::Module)],
        available: &[&verum_vbc::VbcModule],
    ) -> Result<()> {
        // Source exports, including re-export aliases, choose the declaring
        // descriptor. Archive bundle names are not namespace authority.
        let source_registry = self.session.module_registry();
        let source_registry = source_registry.read();
        let source_resolver = verum_types::TypeChecker::new();
        codegen.import_owned_bootstrap_nominal_dependencies_with_resolver(
            sources, available, |written| {
                let (owner, name) = written.rsplit_once('.')?;
                let (declaration, declared_owner) = source_resolver.resolve_public_source_type(
                    owner, name, &source_registry,
                )?;
                let source = source_registry.get_by_path_aliased(&declared_owner)?;
                let owner = VbcCodegen::resolve_full_module_path(&source.ast, &declared_owner)
                    .unwrap_or_else(|| declared_owner.to_string());
                Some(Text::from(verum_vbc::module::qualify_module_name(
                    &owner, declaration.name.name.as_str(),
                )))
            },
        ).map_err(|error| anyhow::anyhow!("importing nominal dependencies: {error}"))?;
        Ok(())
    }

    /// Register every source free function under its declaring source path.
    /// Visibility is enforced by source export resolution, not by removing
    /// declarations needed by eligible consumers. Only globally unique public
    /// leaf names also receive a convenience alias.
    /// Declaration interfaces and later bodies update the same function ID.
    ///
    /// Paired with `emit_missing_stub_descriptors_with_callm(false)`
    /// in `finalize_module_from_state` — the call site bytecode emits
    /// `Call(stub_id)` with the stub-id from this pre-registration;
    /// the finalize step then synthesises a minimal extern-shaped
    /// `FunctionDescriptor` carrying the function's NAME for every
    /// stub id the local bytecode actually references.  At archive
    /// load, `ArchiveBodyRemap::map_function`'s Tier-2b name-fallback
    /// resolves stub_id → real user-side FunctionId by walking the
    /// archive-wide `archive_func_by_name` index (populated for every
    /// loaded archive function regardless of mount set).
    ///
    fn pre_register_source_free_functions(
        &mut self,
        all_modules: &[(String, Vec<OwnedSourceFile>)],
    ) -> Result<()> {
        use verum_ast::decl::Visibility;
        use verum_ast::ItemKind;
        use verum_vbc::codegen::FunctionInfo;
        use verum_vbc::module::FunctionId;

        // Pass 1: count occurrences of each public free-fn simple name
        // across the entire parsed stdlib. Only unique names get leaf aliases.
        let mut name_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (_module_name, ast_modules) in all_modules {
            for OwnedSourceFile { ast: ast_module, .. } in ast_modules {
                for item in ast_module.items.iter() {
                    if let ItemKind::Function(func) = &item.kind
                        && matches!(func.visibility, Visibility::Public)
                    {
                        let name = func.name.name.to_string();
                        *name_counts.entry(name).or_insert(0) += 1;
                    }
                }
            }
        }

        // Pass 2: canonical identities survive collisions and compile order.
        let mut stubs_registered: usize = 0;
        for (module_name, ast_modules) in all_modules {
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                let owner = source_owner;
                for item in ast_module.items.iter() {
                    let func = match &item.kind {
                        ItemKind::Function(f) => f,
                        _ => continue,
                    };
                    let name = func.name.name.to_string();
                    let qualified = verum_vbc::module::qualify_module_name(&owner, &name);
                    if self.global_function_registry.contains_key(&qualified) {
                        continue;
                    }
                    let stub_id =
                        FunctionId(verum_vbc::stub_ranges::STAGE3_BASE - stubs_registered as u32);
                    let param_count = func.params.len();
                    let param_names: Vec<String> = (0..param_count)
                        .map(|i| format!("_arg{}", i))
                        .collect();
                    let info = FunctionInfo {
                        has_source_body: func.body.is_some(),
                        foreign_declaration: None,
                        callable_signature: None,
                        type_param_ids: verum_vbc::codegen::VbcCodegen::declared_type_param_ids(func, &[], &[]),
                        explicit_type_param_ids: verum_vbc::codegen::VbcCodegen::declared_explicit_type_param_ids(func, &[], &[]),
                        id: stub_id,
                        param_count,
                        param_names,
                        param_type_names: vec![],
                        is_async: func.is_async,
                        is_generator: false,
                        contexts: vec![],
                        return_type: None,
                        yield_type: None,
                        intrinsic_name: None,
                        variant_tag: None,
                        parent_type_name: None,
                        variant_payload_types: None,
                        is_partial_pattern: false,
                        takes_self_mut_ref: false,
                        return_type_name: None,
                        return_type_inner: None,
                        is_const: false,
                        is_transparent_wrapper: false,
                        param_closure_return_type_names: Vec::new(),
                    };
                    self.global_function_registry.insert(qualified, info.clone());
                    if func.visibility.is_public() && name_counts.get(&name) == Some(&1) {
                        self.global_function_registry.entry(name).or_insert(info);
                    }
                    stubs_registered += 1;
                }
            }
        }
        if std::env::var("VERUM_TRACE_STUB").is_ok() {
            eprintln!(
                "[stage-3] pre-registered {} source-qualified free-fn declarations",
                stubs_registered
            );
        }
        Ok(())
    }

    fn register_stdlib_types_globally(
        &mut self,
        all_modules: &[(String, Vec<OwnedSourceFile>)],
        type_checker: &mut verum_types::infer::TypeChecker,
        config: &CoreConfig,
    ) -> Result<()> {
        // The SESSION's target config, not the host's.
        //
        // `TargetConfig::host()` fills `debug_assertions` from
        // `cfg!(debug_assertions)` — the build profile of the PRECOMPILER
        // BINARY, which is built `--release`, so the field is false. The
        // bake then evaluates `@cfg(debug)` as false and `@cfg(not(debug))`
        // as true, and ships the empty arm of every debug-assertion pair in
        // `core/base/panic.vr`. Measured: `debug_assert_eq(1, 2, "D")` is a
        // no-op in every program, from an archive baked by a compiler whose
        // own evaluator says `debug_assertions` is true (T0758).
        //
        // The platform half of `host()` is correct — it reads
        // `std::env::consts::OS` — which is why `temp_dir`'s `@cfg(unix)`
        // arm wins over its `@cfg(target_os = "windows")` arm and the defect
        // stayed invisible: only the build-profile field is wrong, and only
        // one family of predicates reads it.
        //
        // `Session::build_target_config` resolves the same platform facts
        // plus the manifest's `debug_assertions` override and
        // `optimization_level`, so it is right on both halves. It is also
        // what every user compile already uses — AGENTS.md's rule that a
        // per-target decision must read the TARGET, never the host, applies
        // here with an extra edge: this host gate leaks the compiler's own
        // build profile into the shipped standard library.
        let target = self.session.cfg_evaluator().config().clone();

        // Pass 0: Process imports with aliases
        if config.verbose {
            eprintln!("  Pass 0: Processing import aliases from all modules...");
        }
        for (_module_name, ast_modules) in all_modules {
            for OwnedSourceFile { ast: ast_module, .. } in ast_modules {
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
                for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                    let current_module_path = source_owner.clone();

                    for item in &ast_module.items {
                        if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                            // Process the full import (not just aliases) to bring
                            // cross-module types into scope for this module
                            // Third sibling of the argument-position guard
                            // (T0981). `&registry.read()` holds the reader
                            // for the whole call, and `process_import`'s
                            // lazy-load branch takes a WRITE lock on the same
                            // registry — a self-deadlock, not a slow pass.
                            //
                            // On THIS path that would read as "the bake is
                            // just long", which is the costliest possible
                            // disguise: the bake is expected to take tens of
                            // minutes, so nobody would sample it.
                            //
                            // Cloned per mount, like the two siblings, so a
                            // module registered by an earlier mount stays
                            // visible to a later one.
                            let registry_snapshot = registry.read().clone();
                            if let Err(e) = type_checker.process_import(
                                import_decl,
                                &current_module_path,
                                &registry_snapshot,
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
            for OwnedSourceFile { ast: ast_module, .. } in ast_modules {
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
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                // Architectural: set the checker's current module so that
                // `define_type_in_current_module` can publish each type
                // under its fully-qualified key (`{module}.{name}`). Without
                // this the qualified-name layer stays empty and same-named
                // stdlib types (e.g. `RecvError` in broadcast/channel/quic)
                // silently collide on the flat lookup table.
                let per_file_module_path = source_owner.clone();
                type_checker.set_current_module_path(per_file_module_path);

                // ALIAS-VS-MARKER scope (#41): the declaring file's local
                // visibility gates `type X is Name;` alias-vs-marker.
                type_checker.set_alias_scope_from_items(&ast_module.items);
                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        if let Err(e) = type_checker.register_type_declaration(type_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0910")
                                .message(format!(
                                    "Type registration warning in {}: {}",
                                    module_name, e
                                ))
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
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                // Module-scoped resolution: free fn signatures reference types
                // (`Result<_, RecvError>`) that may collide across modules, so
                // anchor each file to its qualified module path before
                // registration.
                let per_file_module_path = source_owner.clone();
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        if let Err(e) = type_checker.register_function_signature(func) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0911")
                                .message(format!(
                                    "Function signature warning in {}: {}",
                                    module_name, e
                                ))
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
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                // Module-scoped resolution for protocol method signatures.
                let per_file_module_path = source_owner.clone();
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                        if let Err(e) = type_checker.register_protocol(protocol_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0912")
                                .message(format!(
                                    "Protocol registration warning in {}: {}",
                                    module_name, e
                                ))
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
            for OwnedSourceFile { owner: source_owner, ast: ast_module, .. } in ast_modules {
                // Architectural: set the current module path so that type
                // references inside impl-block signatures (e.g. `RecvError` in
                // `Stream for BroadcastReceiver`) resolve against the
                // qualified-name layer first — avoiding collisions between
                // same-named types in different stdlib modules.
                let per_file_module_path = source_owner.clone();
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        if let Err(e) = type_checker.register_impl_block(impl_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0913")
                                .message(format!(
                                    "Impl block registration warning in {}: {}",
                                    module_name, e
                                ))
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
            for OwnedSourceFile { ast: ast_module, .. } in ast_modules {
                all_ast_modules.push(ast_module);
            }
        }
        // Pass 5.5: stdlib coercion-protocol discovery.
        // Walks every loaded `.vr` module's AST for
        // `implement <Marker> for X {}` blocks (six markers in
        // `core/base/coercion.vr`: IntCoercible / TensorLike /
        // Indexable / RangeLike / BytewiseFfi / SizedNumeric)
        // and registers the target type with the unifier's
        // matching coercion registry.  Sole registration path —
        // the previous hardcoded-fallback `register_stdlib_coercions`
        // was deleted at #101 step 4 close-out once every retrofit
        // landed; the architectural rule in
        // `verum_types/src/AGENTS.md` ("NEVER hardcode stdlib/core
        // type knowledge in the compiler") is now structurally
        // enforced — there is no hardcoded list left to drift.
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

        // Pass 6: Validate imports
        // Now that all types, functions, and protocols are registered,
        // validate that all imports reference items that actually exist.
        if config.verbose {
            eprintln!("  Pass 6: Validating imports...");
        }

        let exports: Vec<_> = all_modules
            .iter()
            .flat_map(|(_, files)| {
                files
                    .iter()
                    .map(|file| (file.owner.as_str(), file.path.as_path(), &file.ast))
            })
            .collect();
        let sources: Vec<_> = exports
            .iter()
            .map(|(owner, _, ast)| (*owner, *ast))
            .collect();
        let export_index = crate::core_compiler::build_owned_export_index(&exports);
        let import_errors =
            crate::core_compiler::validate_owned_imports(&sources, &export_index, &target);

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

    /// Validate source-owned files before compiling bodies or publishing an archive.
    /// Mandatory checks follow the user pipeline's configured policy. Report mode
    /// adds counts; strict adds declared FFI/capability checks; verify adds contract
    /// verification. An attempted check's errors always reject the archive.
    fn run_user_validation_phases(
        &mut self,
        module_name: &str,
        ast_modules: &[(&str, &verum_ast::Module)],
    ) -> Result<()> {
        let mode = match std::env::var("VERUM_BAKE_VALIDATION") {
            Ok(mode) if matches!(mode.as_str(), "" | "report" | "strict" | "verify") => mode,
            Err(std::env::VarError::NotPresent) => String::new(),
            other => anyhow::bail!(
                "invalid VERUM_BAKE_VALIDATION {other:?}: expected report, strict or verify; mandatory stdlib validation cannot be disabled"
            ),
        };
        let strict = mode == "strict";
        // Contract verification is an explicit additional mode, not evidence
        // provided by an ordinary successful bytecode build.
        let verify = mode == "verify";
        let mut contract_failures = 0usize;
        let mut meta_failures = 0usize;
        let mut ffi_failures = 0usize;
        let mut safety_failures = 0usize;
        let mut strict_findings = 0usize;
        // Context and Send/Sync validation report through the session
        // rather than a return value, so counting only the two Results
        // would have reported a clean library while saying nothing
        // about two of the four phases.
        let errors_before = self.session.error_count();
        for (source_owner, ast_module) in ast_modules {
            // Evaluate closed computations in the file's lexical source scope.
            if let Err(e) = self.phase_meta_evaluation(
                    ast_module,
                    &verum_common::Text::from(*source_owner),
                )
            {
                meta_failures += 1;
                tracing::warn!("stdlib meta evaluation: {}", e);
            }
            self.phase_context_validation(ast_module);
            self.phase_send_sync_validation(ast_module);
            if let Err(e) = self.phase_ffi_validation(ast_module) {
                ffi_failures += 1;
                tracing::warn!("stdlib FFI validation: {}", e);
            }
            if let Err(e) = self.phase_safety_gate(ast_module) {
                safety_failures += 1;
                tracing::warn!("stdlib safety gate: {}", e);
            }
            // Strict mode raises the FFI/capability policy without prohibiting
            // the library's legitimate unsafe/system boundary implementations.
            if strict {
                // The policy asks what is FAIR to ask of a standard
                // library, not what is strictest. `unsafe`, `extern`
                // and FFI are the library's JOB — it is the layer that
                // owns the syscall boundary — so forbidding them would
                // measure nothing but that fact. What can honestly be
                // demanded is that those boundaries be DECLARED:
                // `ffi_boundary = "strict"` rejects FFI carrying no
                // safety annotation, and `capability_required` asks
                // every I/O-touching function to say so.
                let policy = crate::phases::safety_gate::SafetyPolicy {
                    unsafe_allowed: true,
                    ffi: true,
                    ffi_boundary: verum_common::Text::from("strict"),
                    capability_required: true,
                    mls_level: verum_common::Text::from("public"),
                    forbid_stdlib_extern: false,
                };
                let diagnostics = crate::phases::safety_gate::check_safety_at(
                    std::slice::from_ref(*ast_module),
                    policy,
                    Some(self.session),
                );
                strict_findings += diagnostics.len();
                for diagnostic in diagnostics {
                    self.session.emit_diagnostic(diagnostic);
                }
            }
        }
        // A silent pass and an unrun phase look identical from the
        // outside, and three defects on this task were found only
        // because somebody checked which one they had. The report says
        // how many files were actually walked, so "no findings" can be
        // told apart from "no phases".
        if verify {
            let owned: Vec<verum_ast::Module> =
                ast_modules.iter().map(|(_, module)| (*module).clone()).collect();
            let phase = crate::phases::contract_verification::ContractVerificationPhase::new();
            let input = crate::phases::PhaseInput {
                data: crate::phases::PhaseData::AstModules(owned.into()),
                context: crate::phases::PhaseContext {
                    profile: crate::phases::LanguageProfile::Application,
                    target_tier: crate::phases::ExecutionTier::Interpreter,
                    verify_mode: crate::phases::VerifyMode::Proof,
                    opt_level: crate::phases::OptimizationLevel::O0,
                },
            };
            // The trait is `CompilationPhase`; `Phase` does not exist,
            // and getting that wrong costs a full rebuild to discover.
            use crate::phases::CompilationPhase;
            if let Err(diags) = phase.execute(input) {
                contract_failures += diags.len();
                for diagnostic in diags {
                    self.session.emit_diagnostic(diagnostic);
                }
            }
        }

        if mode == "report" || strict || verify {
            // The module NAME, because a count without an address is
            // not actionable: the first contract-verification run
            // reported five failures and left no way to say which of
            // 590 modules they were in.
            eprintln!(
                "[bake-validation] module={} files={} ffi_failures={} safety_failures={} \
                 session_errors_added={} strict_findings={} contract_failures={} \
                 meta_failures={}",
                module_name,
                ast_modules.len(),
                ffi_failures,
                safety_failures,
                self.session.error_count().saturating_sub(errors_before),
                strict_findings,
                contract_failures,
                meta_failures,
            );
        }
        let session_errors = self.session.error_count().saturating_sub(errors_before);
        anyhow::ensure!(
            session_errors == 0
                && ffi_failures == 0
                && safety_failures == 0
                && strict_findings == 0
                && contract_failures == 0
                && meta_failures == 0,
            "stdlib validation failed in {module_name}: {session_errors} diagnostics, {ffi_failures} FFI, {safety_failures} safety, {strict_findings} strict, {contract_failures} contract, {meta_failures} meta failures"
        );
        Ok(())
    }

    fn compile_core_module_from_ast(
        &mut self,
        module: &StdlibModule,
        ast_modules: &[(&str, &verum_ast::Module)],
        config: &CoreConfig,
        target: &verum_ast::cfg::TargetConfig,
        declaration_interfaces: &[verum_vbc::VbcModule],
    ) -> Result<(verum_vbc::VbcModule, usize)> {
        use verum_vbc::codegen::CodegenConfig;

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
        codegen.set_foreign_interfaces(self.global_foreign_interfaces.clone());
        codegen.set_source_exports(self.global_source_exports.clone());

        // Import functions and protocols from previously compiled modules
        codegen.import_functions(&self.global_function_registry);
        codegen.import_protocol_declarations(&self.global_protocol_registry);
        // D2b: seed this module's codegen with the field layouts of all
        // record types compiled in earlier (dependency-ordered) modules,
        // so cross-module record construction/read resolves positional
        // field offsets via `type_field_layouts` instead of the
        // non-positional global-intern fallback. Additive / first-wins —
        // this module's own declarations (collected just below) always win.
        codegen.import_type_layouts(&self.global_type_layout_registry);
        codegen.import_type_field_names(&self.global_type_field_name_registry);
        // Seed cross-module type aliases (e.g. `IoError → StreamError`)
        // so a use of an imported alias as a namespace resolves. Additive
        // / first-wins — this module's own alias decls always win.
        codegen.import_type_aliases(&self.global_type_alias_registry);
        // T1192: seed the transparent-wrapper names declared by earlier
        // modules. Without this every module starts with an empty
        // `newtype_names` (a fresh codegen per module), so a newtype from
        // another directory compiles OPAQUE on both sides — self-consistent
        // inside the archive, and incompatible with user code, which
        // recovers the flag from the archive descriptor. A value crossing
        // that boundary returns its own address. Additive; this module's
        // own declarations are inserted by `compile_type_decl` afterwards.
        codegen.import_newtype_names(&self.global_transparent_newtypes);
        // Seed blanket impls (`implement<T: Base> Derived for T`) harvested
        // from previously compiled (dependency-ordered) modules, so a blanket
        // declared in one module applies to implementors declared in another.
        // This module's own blankets are collected by the Pass 1a.5 pre-pass
        // below. Additive / first-wins.
        codegen.import_blanket_impls(&self.global_blanket_impl_registry);

        // Register stdlib intrinsic shortcuts so callers in this module's
        // bodies emit InlineSequence opcodes instead of Calls to the body
        // wrappers. Without this, e.g. `Text.grow`'s `alloc(new_cap+1, 1)`
        // call resolves to the stdlib `alloc()` body in core/base/memory.vr
        // (which wraps cbgr_alloc in a `match Ok((ptr, _, _)) => ptr` —
        // the destructure binds the wrong tuple slot through the precompiled
        // bytecode and returns null). The intercept at
        // `expressions.rs::compile_call`'s line ~4385 only fires when
        // `func_info.intrinsic_name` is set; that field is populated by
        // `register_stdlib_intrinsics` for every well-known stdlib symbol
        // (alloc / dealloc / memcpy / ptr_offset / etc.). Mirrors the
        // user-side `codegen.initialize()` ordering at
        // `pipeline/vbc_codegen.rs:357`. The intercept also fires for
        // intrinsics with bodies — `cbgr_alloc` is the canonical example;
        // its body becomes inert because every caller hits the intercept
        // first.
        codegen.register_stdlib_intrinsics();
        codegen.register_runtime_io_functions();

        // Pool-owned nominal signatures accompany the existing text/layout
        // registry. Import only this source unit's dependency closure.
        let mut prior_modules: Vec<_> = self.compiled_stdlib_modules.values().collect();
        // A declaration archive can contain many source owners, including
        // dependencies of the unit whose name happens to match the archive.
        // The importer excludes local declarations by their source identity.
        prior_modules.extend(declaration_interfaces);
        prior_modules.sort_by(|a, b| a.name.cmp(&b.name));
        self.import_core_nominal_dependencies(&mut codegen, ast_modules, &prior_modules)
            .map_err(|error| anyhow::anyhow!("{}: {error}", module.name))?;

        // Three-pass compilation within the module (cross-file two-phase collection)
        // ONE collector, the same one a user compile uses (T0692).
        //
        // This drove its own sequence — Pass 1a (protocols), 1a.5
        // (blankets), 1a.6 (claim + FFI layouts), 1b (the rest) — beside
        // `collect_all_declarations`, and the two drifted: three
        // correctness defects traced to a pre-pass present on one side
        // and missing on the other, which is why
        // `check_bake_prepass_parity` exists at all. The difference
        // that mattered was never the passes, it was that a MODULE is
        // many files and a user compile is one; that difference now
        // lives in the argument.
        codegen
            .collect_owned_unit_declarations(ast_modules)
            .map_err(|e| {
            anyhow::anyhow!("collecting declarations for {}: {}", module.name, e)
        })?;

        // Rejections must propagate before body compilation and archive publication.
        self.run_user_validation_phases(&module.name, ast_modules)?;

        // After all declarations collected, resolve pending imports
        // This handles cross-file imports within the same module
        codegen.resolve_pending_imports();

        // A published module promises executable bodies for all active declarations.
        // Declaration-only interfaces supply forward references across module cycles.
        use verum_vbc::codegen::ItemFailurePolicy;
        codegen.compile_default_methods_with_policy(ItemFailurePolicy::Strict)
            .map_err(|e| anyhow::anyhow!("compiling default methods for {}: {e}", module.name))?;

        // ARCH-P2 set-stability evidence channel + freeze contract.
        //
        // Phase split within one stdlib module: Pass 1a/1b/1c above REGISTER
        // every type layout this module's bodies can see (imports from the
        // dependency-ordered global registry + own declarations); Pass 2
        // below (and finalize) must only READ that set.  A layout key that
        // appears during body compilation would be a lazy on-miss
        // registration — the class that would make `resolve_field_index`'s
        // candidate set run-order-dependent.  `VERUM_TRACE_LAZY_LAYOUT=1`
        // names every such straggler (ADDED/MUTATED lines) and dumps the
        // canonical post-set for cross-bake diffing; the len snapshot backs
        // the debug_assert freeze after finalize.
        let trace_lazy_layout = std::env::var_os("VERUM_TRACE_LAZY_LAYOUT").is_some();
        let pre_body_layout_count = codegen.type_layout_count();
        let pre_body_layouts: Option<std::collections::BTreeMap<String, Vec<String>>> =
            trace_lazy_layout.then(|| codegen.export_type_layouts().into_iter().collect());

        codegen.compile_owned_unit_items(ast_modules, ItemFailurePolicy::Strict)
            .map_err(|e| anyhow::anyhow!("compiling stdlib bodies for {}: {e}", module.name))?;
        let merged_vbc = codegen
            .finalize_module_from_state()
            .map_err(|e| anyhow::anyhow!("finalize stdlib module {}: {}", module.name, e))?;
        let total_func_count = merged_vbc.functions.len();

        // ARCH-P2 freeze contract: the layout set visible to body-phase
        // field-index resolution must be fully established by the
        // declaration passes.  Pass 2 + finalize may only READ it.  A
        // straggler here means a lazy on-miss registration crept back in
        // — the class that makes `resolve_field_index`'s candidate set
        // dependent on body-compilation order.  Always-on warn (cheap
        // usize compare) so release bakes surface regressions too;
        // debug_assert hard-fails dev builds.  Empirical baseline
        // 2026-07-10: 0 escapees across all 585 stdlib modules.
        let post_body_layout_count = codegen.type_layout_count();
        if post_body_layout_count != pre_body_layout_count {
            eprintln!(
                "[LAZY-LAYOUT] module={} FREEZE-VIOLATION: type_field_layouts len {} -> {} during body phase (set VERUM_TRACE_LAZY_LAYOUT=1 to name the keys)",
                module.name, pre_body_layout_count, post_body_layout_count
            );
        }
        debug_assert_eq!(
            pre_body_layout_count,
            post_body_layout_count,
            "ARCH-P2 freeze contract: type layouts registered during body phase of stdlib module {}",
            module.name
        );

        // ARCH-P2 evidence channel, post-finalize half: name every layout
        // key that appeared or changed during body compilation (lazy
        // registration = a freeze-contract escapee), then dump the full
        // canonical set for cross-bake diffing.
        if let Some(pre) = pre_body_layouts {
            let post: std::collections::BTreeMap<String, Vec<String>> =
                codegen.export_type_layouts().into_iter().collect();
            for (k, v) in &post {
                match pre.get(k) {
                    None => eprintln!(
                        "[LAZY-LAYOUT] module={} ADDED-DURING-BODIES {} = [{}]",
                        module.name,
                        k,
                        v.join(",")
                    ),
                    Some(old) if old != v => eprintln!(
                        "[LAZY-LAYOUT] module={} MUTATED-DURING-BODIES {} [{}] -> [{}]",
                        module.name,
                        k,
                        old.join(","),
                        v.join(",")
                    ),
                    _ => {}
                }
            }
            for k in pre.keys() {
                if !post.contains_key(k) {
                    eprintln!(
                        "[LAZY-LAYOUT] module={} REMOVED-DURING-BODIES {}",
                        module.name, k
                    );
                }
            }
            for (k, v) in &post {
                eprintln!("[LAYOUT-SET] {}\t{}\t{}", module.name, k, v.join(","));
            }
        }

        // Export newly registered functions and protocols to global registries.
        //
        // Task #16 stage-1/2 stub-overwrite: pre-registered stubs from
        // `pre_register_canonical_static_methods` /
        // the declaration-interface constructor pass live in two
        // sentinel ID ranges that DESCEND from their base value:
        //
        //   * Stage 1 (canonical-type static methods):
        //       ids in [STAGE1_BASE - STUB_RANGE_WIDTH, STAGE1_BASE]
        //       where STAGE1_BASE = u32::MAX - 0x40_0000 (0xFFBF_FFFF).
        //   * Stage 2 (variant constructors):
        //       ids in [STAGE2_BASE - STUB_RANGE_WIDTH, STAGE2_BASE]
        //       where STAGE2_BASE = u32::MAX - 0xC0_0000 (0xFF3F_FFFF).
        //
        // The stub-emitting code assigns `id = BASE - <stub_index>`,
        // so each stage occupies a window descending FROM the base.
        // A 1M-slot window (`0x10_0000`) per stage is comfortably
        // larger than any realistic stdlib stub count (current ~700)
        // and stays well clear of the canonical sentinel ranges used
        // elsewhere in the codegen: variant-tag descent `u32::MAX -
        // tag` (tag ≤ ~100 → range starts at 0xFFFF_FF9C) and
        // newtype-pass-through `u32::MAX / 2` (= 0x7FFF_FFFF).
        //
        // When the producing module exports its REAL body for a
        // stubbed name, OVERWRITE the stub with the real FunctionInfo
        // (real FunctionId from `next_func_id`, full param_type_names,
        // real return_type, etc.).  For every other existing entry,
        // preserve `entry().or_insert()` first-wins.
        //
        // Pre-fix this regressed pass-rate 80/146 → 63/151 because
        // the stubs blocked every real body — `entry().or_insert()`
        // is first-wins and the stub always wins.  The PREVIOUS
        // overwrite gate (commit `37fc3fdc3` first attempt) was
        // inverted: it used `id >= STAGE2_BASE` which excludes EVERY
        // stub (stubs are BELOW their base).  This second attempt
        // gets the direction right: check `BASE - WIDTH <= id <= BASE`.
        // Ranges are canonical in `verum_vbc::stub_ranges` (stages
        // 1-4); the per-stage `pre_register_*` helpers below assign
        // `id = BASE - <stub_index>` from the same constants.
        let is_in_stub_range = |id: u32| -> bool { verum_vbc::stub_ranges::is_stub_id(id) };
        // The view has already applied source-owner aliases. Clone metadata
        // only when this publication accepts it; most imports already exist.
        let new_functions = codegen.export_function_view();
        for (name, info) in new_functions {
            let is_stub = matches!(
                self.global_function_registry.get(name.as_str()),
                Some(existing) if is_in_stub_range(existing.id.0)
            );
            // An importing module may have specialized a forward declaration
            // in its own type pool. Only an actual definition may replace the
            // original forward. Publishing another forward leaks local type IDs
            // into every later consumer of the same declaration.
            if is_stub && !is_in_stub_range(info.id.0) {
                self.global_function_registry
                    .insert(name.into_string(), info.clone());
            } else {
                self.global_function_registry
                    .entry(name.into_string())
                    .or_insert_with(|| info.clone());
            }
        }

        let new_protocols = codegen.export_protocols();
        for (name, info) in new_protocols {
            self.global_protocol_registry.entry(name).or_insert(info);
        }

        // D2b: publish this module's record field layouts into the global
        // registry so subsequently-compiled modules can resolve its
        // record types' positional field offsets. First-wins (a type is
        // declared in exactly one module, so collisions only arise from
        // sibling sum-type variants sharing a simple name — the
        // descriptor path, made string-authoritative in the D2 fix,
        // disambiguates those for registered types).
        for (name, layout) in codegen.export_type_layouts() {
            self.global_type_layout_registry.entry(name).or_insert(layout);
        }
        for (key, name) in codegen.export_type_field_names() {
            self.global_type_field_name_registry.entry(key).or_insert(name);
        }
        // Publish this module's type aliases so later modules that mount
        // and use them resolve the alias (companion to the alias-vs-marker
        // typechecker fix; without this the embedded @core bodies that use
        // a cross-module alias as a namespace stay lenient panic-stubs).
        module_utils::merge_type_aliases(
            &mut self.global_type_alias_registry,
            codegen.export_type_aliases(),
            &self.ambiguous_core_type_names,
        );
        // T1192: publish this module's transparent-wrapper names, minus the
        // ones another module also declares (see `ambiguous_core_type_names`).
        for name in codegen.export_newtype_names() {
            if !self.ambiguous_core_type_names.contains(&name) {
                self.global_transparent_newtypes.insert(name);
            }
        }
        // Publish this module's blanket impls (T0625) so later, dependent
        // modules apply them to their own implementors of the bound protocol.
        // Deduplicate only the same source declaration and prerequisite.
        for b in codegen.blanket_impls() {
            let already_present = self.global_blanket_impl_registry.iter().any(|existing| {
                existing.same_declaration(b)
            });
            if !already_present {
                self.global_blanket_impl_registry.push(b.clone());
            }
        }

        Ok((merged_vbc, total_func_count))
    }

    /// Build the VBC archive from compiled stdlib modules.
    fn build_stdlib_archive(&self, config: &CoreConfig) -> Result<verum_vbc::VbcArchive> {
        use verum_vbc::{ArchiveBuilder, ArchiveFlags};

        let mut flags = ArchiveFlags::IS_STDLIB;
        if config.debug_info {
            flags |= ArchiveFlags::DEBUG_INFO;
        }
        if config.source_maps {
            flags |= ArchiveFlags::SOURCE_MAPS;
        }

        let mut builder = ArchiveBuilder::stdlib().with_flags(flags);

        let resolver = self
            .stdlib_resolver
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        // Directory discovery does not enumerate file-mounted compilation
        // units. Publish every produced module, retaining the discovered order
        // and a stable order for these explicitly loaded source units.
        let mut order: Vec<_> = resolver.compilation_order().iter().collect();
        let discovered: std::collections::HashSet<&str> =
            order.iter().map(|name| name.as_str()).collect();
        let mut mounted: Vec<_> = self
            .compiled_stdlib_modules
            .keys()
            .filter(|name| !discovered.contains(name.as_str()))
            .collect();
        mounted.sort();
        order.extend(mounted);
        for name in order {
            if let Some(module) = self.compiled_stdlib_modules.get(name) {
                let deps: Vec<&str> = if let Some(source) = resolver.get_module(name) {
                    source.dependencies.iter().map(String::as_str).collect()
                } else {
                    module.dependencies.iter().map(|dependency| {
                        module.get_string(dependency.name).ok_or_else(|| anyhow::anyhow!(
                            "module {name} has an invalid dependency string index {:?}", dependency.name,
                        ))
                    }).collect::<Result<Vec<_>>>()?
                };

                builder.add_module(name, module, &deps).map_err(|e| {
                    anyhow::anyhow!("Failed to add module {} to archive: {}", name, e)
                })?;
            }
        }

        Ok(builder.finish())
    }
}

#[cfg(test)]
#[path = "../../tests/bootstrap/function_exports.rs"]
mod function_export_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/generic_variant_payloads.rs"]
mod generic_variant_payload_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/qualified_record_assignment.rs"]
mod qualified_record_assignment_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/alias_declaring_scope.rs"]
mod alias_declaring_scope_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/field_visibility.rs"]
mod field_visibility_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/semver_requirements.rs"]
mod semver_requirement_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/forward_results.rs"]
mod forward_result_tests;

#[cfg(test)]
#[path = "../../tests/bootstrap/meta_validation_owners.rs"]
mod meta_validation_owner_tests;
