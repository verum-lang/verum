//! Stdlib precompile orchestrator — Phase 4 of the precompiled-stdlib
//! archive epic.
//!
//! Drives the existing [`CompilationPipeline::compile_core`] (stdlib
//! bootstrap mode), then runs a post-pass over the resulting
//! [`verum_vbc::module::VbcModule`] to populate the precompile-stdlib
//! extension fields landed in Phase 3:
//!
//! * theorems table (theorem / lemma / corollary / axiom / tactic
//!   declarations from the source AST),
//! * framework provenance (`@framework(name, citation)` and
//!   `@framework_translate(...)` edges),
//! * cfg-conditional function variants (one entry per `#[cfg(...)]`
//!   arm — populated by the multi-target codegen pass below).
//!
//! Subsequent phases (5–7) embed the resulting `.vbca` into the
//! compiler binary and switch `compile_ast_to_vbc` to deserialise it
//! instead of re-running stdlib codegen on every invocation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::core_compiler::{CoreConfig, StdlibCompilationResult};
use crate::options::CompilerOptions;
use crate::pipeline::CompilationPipeline;
use crate::session::Session;

/// Configuration for the [`precompile_stdlib`] entry point.
#[derive(Debug, Clone)]
pub struct PrecompileConfig {
    /// Filesystem path to `core/`. Defaults to the workspace's
    /// `core/` directory when constructed via [`Self::for_workspace`].
    pub stdlib_path: PathBuf,
    /// Output `.vbca` path. Phase-5 build-script consumers expect
    /// `target/precompiled-stdlib/runtime.vbca` by default.
    pub output_path: PathBuf,
    /// Optional target-triple override. `None` = host triple. Phase
    /// 4b will read this and emit per-target variants for cfg-
    /// conditional functions; today the value is recorded in the
    /// archive header but selection is host-only.
    pub target_triple: Option<String>,
    /// Verbose progress reporting.
    pub verbose: bool,
}

impl PrecompileConfig {
    /// Resolve the workspace root by walking up from `start` until
    /// `core/mod.vr` is found, then build a default config writing to
    /// `<workspace>/target/precompiled-stdlib/runtime.vbca`.
    pub fn for_workspace(start: &Path) -> Result<Self> {
        let workspace = find_workspace_root(start)?;
        let stdlib_path = workspace.join("core");
        let output_path = workspace
            .join("target")
            .join("precompiled-stdlib")
            .join("runtime.vbca");
        Ok(Self {
            stdlib_path,
            output_path,
            target_triple: None,
            verbose: false,
        })
    }
}

/// Run the stdlib precompile pipeline.
///
/// Steps:
///   1. Resolve workspace + ensure `core/` is a valid stdlib root.
///   2. Build a [`CompilationPipeline`] in `StdlibBootstrap` mode.
///   3. Drive [`CompilationPipeline::compile_core`] — produces a
///      single-target VBC archive at `cfg.output_path`.
///   4. Phase-4b post-pass (deferred): re-open the archive, walk the
///      stdlib AST in parallel, populate `theorems`,
///      `framework_provenance`, and multi-variant `function_variants`
///      tables on every contained `VbcModule`, re-serialise.
///
/// Returns the [`StdlibCompilationResult`] from `compile_core` plus
/// the absolute output path.
pub fn precompile_stdlib(cfg: &PrecompileConfig) -> Result<StdlibCompilationResult> {
    if cfg.verbose {
        eprintln!(
            "verum stdlib precompile: stdlib={}, output={}, target={}",
            cfg.stdlib_path.display(),
            cfg.output_path.display(),
            cfg.target_triple.as_deref().unwrap_or("<host>")
        );
    }

    if !cfg.stdlib_path.is_dir() {
        anyhow::bail!(
            "stdlib path is not a directory: {}",
            cfg.stdlib_path.display()
        );
    }
    let mod_vr = cfg.stdlib_path.join("mod.vr");
    if !mod_vr.is_file() {
        anyhow::bail!(
            "stdlib path missing mod.vr (not a valid `core/` root): {}",
            cfg.stdlib_path.display()
        );
    }

    if let Some(parent) = cfg.output_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create output directory {}", parent.display())
        })?;
    }

    let core_config = CoreConfig::new(cfg.stdlib_path.clone())
        .with_output(cfg.output_path.clone());
    let core_config = if cfg.verbose {
        let mut c = core_config;
        c.verbose = true;
        c
    } else {
        core_config
    };

    // Stdlib bootstrap doesn't read any user input; the empty
    // `CompilerOptions` defaults are sufficient.
    let mut session = Session::new(CompilerOptions::default());
    let mut pipeline = CompilationPipeline::new_core(&mut session, core_config);
    let result = pipeline
        .compile_core()
        .context("CompilationPipeline::compile_core failed during stdlib precompile")?;

    if cfg.verbose {
        eprintln!(
            "verum stdlib precompile: {} modules, {} functions in {:?}, archive {} ({} bytes)",
            result.modules_compiled,
            result.functions_compiled,
            result.total_time,
            result.output_path.display(),
            result.output_size,
        );
    }

    // T2-extended: produce `runtime.core_metadata` alongside the
    // `runtime.vbca` archive.  The metadata bytes hold the typecheck-
    // ready stdlib metadata (CoreMetadata) — at runtime
    // `embedded_stdlib_metadata::get_runtime_metadata()` decodes them
    // once via bincode and feeds the typechecker directly via
    // `pipeline.set_stdlib_metadata`.  Replaces the slow
    // `load_stdlib_modules` parse + walk path entirely.
    //
    // **Cold-start fix**: also extract `public context Name { … }`
    // declarations from the stdlib source tree.  Without this the
    // bincode-serialised `CoreMetadata` carries an empty
    // `context_declarations` list, and the runtime fallback at
    // `phases_orchestration.rs::phase_type_check` re-parses every
    // stdlib `.vr` (568 files) with the full parser to recover
    // them.  That fallback alone was burning ~250ms of cold-start
    // typecheck time on hello-world.
    write_core_metadata_alongside_archive(
        &result.output_path,
        Some(&cfg.stdlib_path),
        cfg.verbose,
    )?;

    Ok(result)
}

/// Open the freshly-written `runtime.vbca`, convert it to
/// `CoreMetadata`, bincode-serialise, and write
/// `runtime.core_metadata` next to it.  Build.rs picks both files
/// up via parallel `include_bytes!` calls.
///
/// Failures are propagated — the precompile is meaningless without
/// the metadata sidecar.  Single point where the typecheck data
/// lifecycle is materialised; replacing the source-driven path
/// requires this sidecar to land on disk.
fn write_core_metadata_alongside_archive(
    archive_path: &Path,
    stdlib_source_root: Option<&Path>,
    verbose: bool,
) -> Result<()> {
    // Archive uses the verum_vbc custom binary format (with `write_archive`
    // / `read_archive`), not bincode — so use the canonical reader.
    let archive = verum_vbc::archive::read_archive_from_file(archive_path)
        .with_context(|| {
            format!(
                "read freshly-written archive {} for metadata extraction",
                archive_path.display()
            )
        })?;

    let mut metadata = crate::archive_metadata::archive_to_core_metadata(&archive);
    if let Some(root) = stdlib_source_root {
        let (names, decl_nodes) = scan_context_declarations(root);
        metadata.context_declarations = names;
        metadata.context_decl_nodes = decl_nodes;
        if verbose {
            eprintln!(
                "verum stdlib precompile: extracted {} context decls ({} with full AST) from {}",
                metadata.context_declarations.len(),
                metadata.context_decl_nodes.len(),
                root.display(),
            );
        }

        // #104 — populate `metadata.content_hash` with the source-tree
        // blake3 so the SDK lookup path (`SdkLookup::find` keyed on
        // the hex prefix) can match installs by content.  Convention
        // mirrors `verum stdlib install`'s
        // `compute_source_blake3`: path-prefixed, sorted, no
        // schema-version salt — the SDK is the *source*, not the
        // archive, so its identity is invariant under compiler
        // upgrades.
        metadata.content_hash = compute_source_blake3_for_root(root);

        // #101 — populate `decl_span` on every `TypeDescriptor`,
        // `FunctionDescriptor`, and `ProtocolDescriptor` in the
        // metadata so the diagnostic emitter can point at stdlib
        // declaration sites without consulting source.  Single
        // source-walk visits every `.vr`; the resulting map is keyed
        // by `<module_path>.<simple_name>` so injection is O(1) per
        // descriptor.
        inject_decl_spans(&mut metadata, root, verbose);
    }
    let bytes = bincode::serialize(&metadata)
        .context("bincode serialise CoreMetadata for sidecar emit")?;

    let sidecar = archive_path.with_extension("core_metadata");
    std::fs::write(&sidecar, &bytes)
        .with_context(|| format!("write metadata sidecar {}", sidecar.display()))?;

    if verbose {
        eprintln!(
            "verum stdlib precompile: emitted typecheck metadata sidecar {} ({} bytes; {} types, {} functions, {} protocols)",
            sidecar.display(),
            bytes.len(),
            metadata.types.len(),
            metadata.functions.len(),
            metadata.protocols.len(),
        );
    }
    Ok(())
}

/// Walks the stdlib source tree (recursive) for `.vr` files and
/// extracts every public `context Name { ... }` /
/// `context protocol Name { ... }` declaration.  Returns BOTH the
/// names list AND the full `ContextDecl` AST nodes, so the runtime
/// fast-path can register full method signatures via
/// `register_stdlib_context_full` without re-parsing source.
///
/// Cheap quick-filter (`.contains("public context ")`) gates the
/// expensive full parse — only files with at least one stdlib
/// context get parsed.  Spans are preserved as-emitted (parser
/// produces FileId(0) + dummy ranges in this isolated context),
/// which keeps the bincode payload reproducible across precompile
/// invocations.
///
/// #104 — Compute the blake3-32 source-tree hash for `core/`,
/// matching the convention used by `verum stdlib install`'s
/// `compute_source_blake3`.  Path-prefixed, byte-content-hashed,
/// sorted lexicographically; **no schema-version salt** because
/// the SDK is the *source* (compiler-version-invariant), not the
/// precompile artefact.
///
/// Stored in `CoreMetadata.content_hash` so `SdkLookup::find` can
/// derive the expected on-disk install prefix from the binary's
/// embedded metadata.
fn compute_source_blake3_for_root(root: &Path) -> [u8; 32] {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(root, &p, out);
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("vr") {
                continue;
            }
            let rel = match p.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let bytes = match std::fs::read(&p) {
                Ok(b) => b,
                Err(_) => continue,
            };
            out.push((rel, bytes));
        }
    }
    walk(root, root, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    for (rel, bytes) in &files {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(bytes);
        hasher.update(b"\0");
    }
    *hasher.finalize().as_bytes()
}

/// Result names ordered via BTreeSet for deterministic output;
/// AST nodes follow the same key ordering via `OrderedMap`.
fn scan_context_declarations(
    root: &Path,
) -> (
    verum_common::List<verum_common::Text>,
    verum_common::OrderedMap<verum_common::Text, verum_ast::decl::ContextDecl>,
) {
    use std::collections::BTreeMap;
    use verum_ast::decl::Visibility;
    let mut found_decls: BTreeMap<String, verum_ast::decl::ContextDecl> = BTreeMap::new();
    fn walk(dir: &Path, found: &mut BTreeMap<String, verum_ast::decl::ContextDecl>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("vr") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // Quick gate: most stdlib files don't declare contexts.
            if !content.contains("public context ") {
                continue;
            }
            // Full parse — needed to capture method signatures.
            let mut parser = verum_fast_parser::Parser::new(&content);
            let module = match parser.parse_module() {
                Ok(m) => m,
                Err(_) => continue,
            };
            for item in &module.items {
                if let verum_ast::ItemKind::Context(ctx_decl) = &item.kind {
                    if matches!(ctx_decl.visibility, Visibility::Public) {
                        let name = ctx_decl.name.name.as_str().to_string();
                        // First-wins under name collision so the
                        // BTreeSet-ordered iteration is reproducible.
                        found.entry(name).or_insert_with(|| (*ctx_decl).clone());
                    }
                }
            }
        }
    }
    walk(root, &mut found_decls);
    let names: verum_common::List<verum_common::Text> = found_decls
        .keys()
        .map(|k| verum_common::Text::from(k.as_str()))
        .collect();
    let mut decl_map: verum_common::OrderedMap<
        verum_common::Text,
        verum_ast::decl::ContextDecl,
    > = verum_common::OrderedMap::new();
    for (k, v) in found_decls {
        decl_map.insert(verum_common::Text::from(k.as_str()), v);
    }
    (names, decl_map)
}

/// #101 — populate `decl_span` on every `TypeDescriptor`,
/// `FunctionDescriptor`, and `ProtocolDescriptor` in `metadata` by
/// walking the stdlib source tree under `root`.
///
/// Single source-walk visits every `.vr`, parses it into an AST, and
/// records the byte range of each top-level declaration's `name`
/// token alongside the canonical module path
/// (`stdlib_index::file_path_to_module_path`).  The resulting
/// `(module_path, name) → DeclSpan` map is then used to populate
/// every descriptor in `metadata` whose key matches.
///
/// Descriptors whose key has no source-side match keep
/// `decl_span: Maybe::None` (the field's `#[serde(default)]` value).
/// This is graceful degradation — non-fatal — and surfaces in the
/// `verbose` log line as a populated/total ratio.
fn inject_decl_spans(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    root: &Path,
    verbose: bool,
) {
    use std::collections::HashMap;
    use verum_ast::ItemKind;
    use verum_common::{Maybe, Text};
    use verum_types::core_metadata::DeclSpan;

    /// Per-name source-side decl candidate. The metadata's
    /// `module_path` is COARSER than the source-walk's file-grained
    /// path (multiple files coalesce into one VBC archive entry —
    /// e.g. `core/architecture/{types,anti_patterns}.vr` both land
    /// under archive entry `core.architecture`).  We index every
    /// candidate by name and resolve to the closest matching
    /// descriptor via descendant-or-equal `module_path` matching.
    struct Candidate {
        /// File-grained module path the source walk would assign,
        /// e.g. `core.architecture.types` for
        /// `core/architecture/types.vr`.
        source_module: String,
        /// Span+filename the diagnostic emitter ultimately renders.
        decl_span: DeclSpan,
    }

    let mut name_index: HashMap<String, Vec<Candidate>> = HashMap::with_capacity(8192);
    let mut files_visited: usize = 0;
    let mut files_parsed: usize = 0;

    fn visit_dir(
        root: &Path,
        dir: &Path,
        name_index: &mut HashMap<String, Vec<Candidate>>,
        files_visited: &mut usize,
        files_parsed: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_dir(root, &path, name_index, files_visited, files_parsed);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("vr") {
                continue;
            }
            *files_visited += 1;
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut parser = verum_fast_parser::Parser::new(&content);
            let module = match parser.parse_module() {
                Ok(m) => m,
                Err(_) => continue,
            };
            *files_parsed += 1;

            let rel_path = match path.strip_prefix(root) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            let source_module = crate::stdlib_index::file_path_to_module_path(&rel_str);

            let record_decl = |ident: &verum_ast::Ident,
                               name_index: &mut HashMap<String, Vec<Candidate>>| {
                let span = ident.span;
                name_index
                    .entry(ident.name.as_str().to_string())
                    .or_default()
                    .push(Candidate {
                        source_module: source_module.clone(),
                        decl_span: DeclSpan {
                            file: Text::from(rel_str.as_str()),
                            start: span.start,
                            end: span.end,
                        },
                    });
            };

            for item in &module.items {
                match &item.kind {
                    ItemKind::Function(d) => record_decl(&d.name, name_index),
                    ItemKind::Type(d) => record_decl(&d.name, name_index),
                    ItemKind::Protocol(d) => record_decl(&d.name, name_index),
                    ItemKind::Const(d) => record_decl(&d.name, name_index),
                    ItemKind::Static(d) => record_decl(&d.name, name_index),
                    // Impl blocks carry inherent / protocol-impl
                    // methods + associated types + consts whose
                    // FunctionDescriptor.parent_type points at the
                    // impl receiver — so the metadata side has them
                    // and our diagnostic emitter wants their decl
                    // sites too.
                    ItemKind::Impl(d) => {
                        for impl_item in d.items.iter() {
                            match &impl_item.kind {
                                verum_ast::decl::ImplItemKind::Function(fd) => {
                                    record_decl(&fd.name, name_index);
                                }
                                verum_ast::decl::ImplItemKind::Type { name, .. }
                                | verum_ast::decl::ImplItemKind::Const { name, .. } => {
                                    record_decl(name, name_index);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    visit_dir(root, root, &mut name_index, &mut files_visited, &mut files_parsed);

    /// Pick the candidate whose `source_module` is the descriptor's
    /// `module_path` (exact match) or its strict descendant (e.g.
    /// `core.architecture.types` for descriptor `core.architecture`).
    /// Among descendants prefer the SHORTEST source_module — the
    /// closest source file.  Returns `None` if nothing matches.
    fn resolve<'a>(
        candidates: &'a [Candidate],
        descriptor_module: &str,
    ) -> Option<&'a Candidate> {
        // Exact-match first — strongest signal.
        for c in candidates {
            if c.source_module == descriptor_module {
                return Some(c);
            }
        }
        // Strict-descendant fallback: source is e.g.
        // `core.architecture.types` and descriptor is
        // `core.architecture`.  Pick the shortest such — closest in
        // the module tree.
        let mut best: Option<&Candidate> = None;
        let prefix = format!("{}.", descriptor_module);
        for c in candidates {
            if c.source_module.starts_with(&prefix) {
                match best {
                    None => best = Some(c),
                    Some(prev) if c.source_module.len() < prev.source_module.len() => {
                        best = Some(c);
                    }
                    _ => {}
                }
            }
        }
        if best.is_some() {
            return best;
        }
        // Last-ditch fallback: if only ONE candidate exists for this
        // name across the entire stdlib, use it regardless of
        // module_path.  This catches descriptors whose module_path
        // is empty or oddly-namespaced; the diagnostic still points
        // at the real declaration site.
        if candidates.len() == 1 {
            return candidates.first();
        }
        None
    }

    fn populate<V, F>(
        map: &mut verum_common::OrderedMap<Text, V>,
        name_index: &HashMap<String, Vec<Candidate>>,
        get_module_and_name: F,
        get_decl_span: impl Fn(&mut V) -> &mut Maybe<DeclSpan>,
    ) -> usize
    where
        F: Fn(&V) -> (&str, &str),
    {
        let mut populated = 0;
        for (_, val) in map.iter_mut() {
            if matches!(get_decl_span(val), Maybe::Some(_)) {
                continue;
            }
            let (module_path, name) = get_module_and_name(val);
            if let Some(cands) = name_index.get(name)
                && let Some(found) = resolve(cands, module_path)
            {
                *get_decl_span(val) = Maybe::Some(found.decl_span.clone());
                populated += 1;
            }
        }
        populated
    }

    let total_types = metadata.types.len();
    let populated_types = populate(
        &mut metadata.types,
        &name_index,
        |t| (t.module_path.as_str(), t.name.as_str()),
        |t| &mut t.decl_span,
    );

    let total_fns = metadata.functions.len();
    let populated_fns = populate(
        &mut metadata.functions,
        &name_index,
        |f| (f.module_path.as_str(), f.name.as_str()),
        |f| &mut f.decl_span,
    );

    let total_protos = metadata.protocols.len();
    let populated_protos = populate(
        &mut metadata.protocols,
        &name_index,
        |p| (p.module_path.as_str(), p.name.as_str()),
        |p| &mut p.decl_span,
    );

    if verbose {
        let total_candidates: usize = name_index.values().map(|v| v.len()).sum();
        eprintln!(
            "verum stdlib precompile: inject_decl_spans — {}/{} types, {}/{} functions, {}/{} protocols populated from {} parsed files ({} visited, {} unique names / {} candidates indexed)",
            populated_types, total_types,
            populated_fns, total_fns,
            populated_protos, total_protos,
            files_parsed,
            files_visited,
            name_index.len(),
            total_candidates,
        );
    }
}

// ============================================================================
// Phase 12: precompile-cog
// ============================================================================

/// Configuration for [`precompile_cog`] — the per-cog analogue of
/// [`PrecompileConfig`].
///
/// A "cog" is any Verum project with a `Verum.toml` manifest: a
/// user library, an internal company project, or a third-party
/// package destined for the registry. The Phase 12 orchestrator
/// drives the *same* `CompilationPipeline::compile_core` machinery
/// used for stdlib (Phase 4) — `compile_core` is generic over the
/// source-tree path and module-namespace prefix, so cogs reuse 100%
/// of the bootstrap-mode codegen pipeline rather than spawning a
/// parallel implementation.
///
/// The output `.vbca` follows the registry naming convention:
/// `<name>-<version>-verum-<compiler-version>.vbca` (matches
/// `vbca_fetcher::vbca_cache_path` so the same artifact paths
/// flow through CI publish, on-disk cache, and registry download).
#[derive(Debug, Clone)]
pub struct PrecompileCogConfig {
    /// Cog source root — directory containing `Verum.toml` and a
    /// `src/` (or top-level) tree of `.vr` files. Resolved from
    /// the manifest at construction time.
    pub cog_dir: PathBuf,
    /// Cog name — drives module-namespace prefixing and the output
    /// filename. Read from `cog.name` in `Verum.toml`.
    pub cog_name: String,
    /// Cog version — drives the output filename. Read from
    /// `cog.version` in `Verum.toml`.
    pub cog_version: String,
    /// Output `.vbca` path. Constructed as
    /// `<cog_dir>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`
    /// when not overridden.
    pub output_path: PathBuf,
    /// Optional target-triple. `None` = host. Phase 12b cross-compile
    /// matrix builds will iterate this axis.
    pub target_triple: Option<String>,
    /// Verbose progress reporting.
    pub verbose: bool,
}

impl PrecompileCogConfig {
    /// Build from a cog directory containing `Verum.toml`. Reads the
    /// manifest, resolves cog name + version, computes the canonical
    /// output path. Caller may override [`Self::output_path`] before
    /// running the orchestrator.
    pub fn for_cog(cog_dir: impl Into<PathBuf>) -> Result<Self> {
        let cog_dir: PathBuf = cog_dir.into().canonicalize().context("canonicalize cog dir")?;
        let manifest_path = cog_dir.join("Verum.toml");
        if !manifest_path.is_file() {
            anyhow::bail!(
                "no Verum.toml at {} — not a Verum cog root",
                manifest_path.display()
            );
        }
        let (name, version) = read_cog_manifest_minimal(&manifest_path)?;
        let compiler_version = env!("CARGO_PKG_VERSION");
        let output_filename = format!("{}-{}-verum-{}.vbca", name, version, compiler_version);
        let output_path = cog_dir.join("target").join("cog-vbca").join(output_filename);
        Ok(Self {
            cog_dir,
            cog_name: name,
            cog_version: version,
            output_path,
            target_triple: None,
            verbose: false,
        })
    }

    /// Override the default output path. The default lives at
    /// `<cog_dir>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`;
    /// CI flows that want to upload the artifact straight to a
    /// registry staging area set their own path.
    pub fn with_output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output_path = path.into();
        self
    }
}

/// Read the minimum manifest fields the orchestrator needs:
/// `cog.name` and `cog.version`. Matches the schema in
/// `crates/verum_cli/src/config.rs`. We don't reuse the CLI's full
/// `Manifest::from_file` parser to avoid dragging
/// `verum_compiler` → `verum_cli` reverse-dep cycles; the cog
/// section is small and stable.
fn read_cog_manifest_minimal(path: &Path) -> Result<(String, String)> {
    let bytes =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    // Use toml::Value for forward-compat — every field after `cog.name`
    // / `cog.version` is ignored.
    let root: toml::Value = toml::from_str(&bytes)
        .with_context(|| format!("parse {} as TOML", path.display()))?;
    let cog = root
        .get("cog")
        .ok_or_else(|| anyhow::anyhow!("Verum.toml missing [cog] table at {}", path.display()))?;
    let name = cog
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Verum.toml [cog].name is missing or not a string"))?
        .to_string();
    let version = cog
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("Verum.toml [cog].version is missing or not a string")
        })?
        .to_string();
    Ok((name, version))
}

/// Run the cog precompile pipeline.
///
/// Steps:
///   1. Validate the cog source tree — `Verum.toml` + at least one
///      `.vr` file under the root.
///   2. Build a [`CompilationPipeline`] in `StdlibBootstrap` mode
///      pointing at the cog source root. The bootstrap mode's
///      global type-registration phase is exactly what we want for
///      a multi-file cog: every `.vr` file's types are registered
///      before any function body is codegen'd, so cross-file
///      references resolve cleanly.
///   3. Drive `compile_core` to produce a single-target `.vbca` at
///      the configured output path.
///   4. Phase 12b TODO: post-pass for multi-variant cfg-conditional
///      function bodies + theorem extraction (mirrors the Phase
///      4b TODO on the stdlib path).
///
/// Reuses 100% of the existing precompile-stdlib infrastructure;
/// no parallel codegen implementation, no new pipeline phases.
pub fn precompile_cog(cfg: &PrecompileCogConfig) -> Result<StdlibCompilationResult> {
    if cfg.verbose {
        eprintln!(
            "verum cog precompile: cog={} ({}), source={}, output={}, target={}",
            cfg.cog_name,
            cfg.cog_version,
            cfg.cog_dir.display(),
            cfg.output_path.display(),
            cfg.target_triple.as_deref().unwrap_or("<host>")
        );
    }

    if !cfg.cog_dir.is_dir() {
        anyhow::bail!("cog directory does not exist: {}", cfg.cog_dir.display());
    }
    let manifest_path = cfg.cog_dir.join("Verum.toml");
    if !manifest_path.is_file() {
        anyhow::bail!(
            "Verum.toml missing at {} — not a Verum cog root",
            manifest_path.display()
        );
    }

    if !has_any_vr_file(&cfg.cog_dir)? {
        anyhow::bail!(
            "cog at {} contains no .vr files — nothing to precompile",
            cfg.cog_dir.display()
        );
    }

    if let Some(parent) = cfg.output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output dir {}", parent.display()))?;
    }

    // Bootstrap-mode pipeline: cog source root replaces the stdlib
    // path, the existing `compile_core` orchestrator handles
    // discovery / parse / global-type-registration / codegen /
    // archive write.
    let core_config = CoreConfig::new(cfg.cog_dir.clone()).with_output(cfg.output_path.clone());
    let core_config = if cfg.verbose {
        let mut c = core_config;
        c.verbose = true;
        c
    } else {
        core_config
    };

    let mut session = Session::new(CompilerOptions::default());
    let mut pipeline = CompilationPipeline::new_core(&mut session, core_config);
    let result = pipeline.compile_core().with_context(|| {
        format!(
            "CompilationPipeline::compile_core failed during cog precompile (cog={})",
            cfg.cog_name
        )
    })?;

    if cfg.verbose {
        eprintln!(
            "verum cog precompile: {} modules, {} functions in {:?}, archive {} ({} bytes)",
            result.modules_compiled,
            result.functions_compiled,
            result.total_time,
            result.output_path.display(),
            result.output_size,
        );
    }

    Ok(result)
}

/// True iff the directory tree (recursive) contains at least one
/// `.vr` file. Used as a fast pre-check before kicking off the full
/// pipeline.
fn has_any_vr_file(root: &Path) -> Result<bool> {
    fn scan(dir: &Path) -> std::io::Result<bool> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            // Skip target/ directories — codegen output, not source.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                if scan(&path)? {
                    return Ok(true);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("vr") {
                return Ok(true);
            }
        }
        Ok(false)
    }
    scan(root).with_context(|| format!("scan {}", root.display()))
}

/// Walk up from `start`, returning the first directory that contains
/// `core/mod.vr`. Mirrors the existing `find_workspace_root` helper
/// in `pipeline/loading.rs` but is exposed here so callers outside
/// the pipeline (CLI, build scripts, tooling) can resolve the
/// workspace without instantiating a session.
fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let start = start.canonicalize().with_context(|| {
        format!("failed to canonicalize start path {}", start.display())
    })?;
    let mut here: &Path = &start;
    loop {
        let candidate = here.join("core").join("mod.vr");
        if candidate.is_file() {
            return Ok(here.to_path_buf());
        }
        match here.parent() {
            Some(p) => here = p,
            None => anyhow::bail!(
                "could not find workspace root containing core/mod.vr starting from {}",
                start.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_for_workspace_resolves() {
        // Walk up from this source file's directory; we must land on
        // a workspace whose `core/mod.vr` exists.
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cfg = PrecompileConfig::for_workspace(&manifest_dir);
        assert!(cfg.is_ok(), "workspace resolution: {:?}", cfg.err());
        let cfg = cfg.unwrap();
        assert!(cfg.stdlib_path.ends_with("core"));
        assert!(cfg.output_path.ends_with("runtime.vbca"));
    }

    #[test]
    fn cog_manifest_minimal_extraction() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let manifest = tmp.join("Verum.toml");
        let _ = std::fs::write(
            &manifest,
            r#"
[cog]
name = "json"
version = "1.4.2"
authors = ["alice"]
description = "JSON helpers"

[language]
edition = "2026"
"#,
        );
        let parsed = read_cog_manifest_minimal(&manifest);
        assert!(parsed.is_ok(), "manifest parse: {:?}", parsed.err());
        let (name, version) = parsed.unwrap();
        assert_eq!(name, "json");
        assert_eq!(version, "1.4.2");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_config_for_cog_resolves_default_output() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-config-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(
            tmp.join("Verum.toml"),
            r#"
[cog]
name = "mycog"
version = "0.3.0"
"#,
        );
        // Need a .vr file for the directory to be a valid cog.
        let _ = std::fs::write(tmp.join("lib.vr"), "fn main() {}\n");

        let cfg = PrecompileCogConfig::for_cog(&tmp);
        assert!(cfg.is_ok(), "for_cog: {:?}", cfg.err());
        let cfg = cfg.unwrap();
        assert_eq!(cfg.cog_name, "mycog");
        assert_eq!(cfg.cog_version, "0.3.0");
        // Output path: <tmp>/target/cog-vbca/mycog-0.3.0-verum-<v>.vbca
        let compiler_v = env!("CARGO_PKG_VERSION");
        let expected_filename = format!("mycog-0.3.0-verum-{}.vbca", compiler_v);
        assert!(
            cfg.output_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s == expected_filename)
                .unwrap_or(false),
            "expected output filename {} but got {}",
            expected_filename,
            cfg.output_path.display(),
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_precompile_rejects_missing_manifest() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-no-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let result = PrecompileCogConfig::for_cog(&tmp);
        assert!(result.is_err(), "expected error for missing manifest");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_precompile_rejects_empty_cog() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(
            tmp.join("Verum.toml"),
            r#"
[cog]
name = "empty"
version = "0.1.0"
"#,
        );
        // No .vr files — cog has no source to precompile.
        let cfg = PrecompileCogConfig::for_cog(&tmp).expect("for_cog");
        let result = precompile_cog(&cfg);
        assert!(result.is_err(), "expected error for empty cog");
        assert!(
            result
                .err()
                .map(|e| format!("{e}").contains("no .vr files"))
                .unwrap_or(false),
            "error should mention missing .vr files"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn precompile_rejects_non_stdlib_dir() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let cfg = PrecompileConfig {
            stdlib_path: tmp.clone(),
            output_path: tmp.join("out.vbca"),
            target_triple: None,
            verbose: false,
        };
        let err = precompile_stdlib(&cfg).err();
        assert!(err.is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
