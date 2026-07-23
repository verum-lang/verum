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

        // Task #20 — populate `module_reexports` from each .vr file's
        // `public mount X.{...}` declarations so user-compile's
        // `load_stdlib_from_embedded` can apply them to ExportTables.
        // Pre-fix, `mount core.base.{replace}` failed `unbound
        // variable: replace` because metadata.functions only stored
        // each fn under its DECLARING module (`core.base.memory`),
        // not the modules that re-export it.  This populates the
        // chain `core.base ← (replace, core.base.memory)` so the
        // user-side ExportTable picks it up.
        scan_module_reexports(&mut metadata, root, verbose);

        // Task #23 — populate `implementations[].protocol_args` from
        // each .vr file's `implement<...> Protocol<Args> for Type { ... }`
        // declarations.  The VBC archive's `TypeImpl` carries only the
        // bare `ProtocolId` — protocol type arguments are dropped at
        // codegen time.  Without this source-walk, every stdlib impl
        // loads with `protocol_args: List::new()` and
        // `ProtocolChecker::can_convert_residual`'s
        // `impl_.protocol_args.first()` probe at protocol.rs:8508
        // returns `None` for every `FromResidual<...>` impl — so the
        // `?` operator at a `Result→Maybe` coercion site says
        // "cannot apply ? to Result inside fn returning Maybe" even
        // though `implement<T, E> FromResidual<Result<Never, E>> for Maybe<T>`
        // is declared at `core/base/maybe.vr:606`.
        scan_implementation_protocol_args(&mut metadata, root, verbose);

        // Fully-qualified free-fn keys at FILE-module granularity
        // (2026-07-09).  `metadata.functions` keys free functions by
        // bare name (first-wins) plus a `<archive_module>.<name>`
        // qualified key — but the archive module is the DIRECTORY
        // ("core.time"), not the file-declared module
        // ("core.time.duration_parse").  Two consequences: (a) the
        // typechecker's fully-qualified call resolution
        // (`core.time.duration_parse.parse(x)`) can never hit, and
        // (b) same-named free fns in sibling files of one directory
        // (duration_parse.parse vs rfc3339.parse) COLLIDE and the
        // loser vanishes from the metadata entirely.  This source
        // walk re-registers every public top-level free fn under its
        // file-declared dotted module key, rebuilding the descriptor
        // from the AST so collision-dropped functions are recovered.
        inject_declared_module_free_fn_keys(&mut metadata, root, verbose);

        // **Audit-driven fundamental fix (Task #44)** — capture
        // `extends` clause from every protocol declaration so
        // `metadata.protocols[X].super_protocols` is populated.
        // VBC archive serializes `ty.protocols` as the type's
        // IMPLEMENTS list (per `archive_metadata.rs:384`), which is
        // empty for a protocol declaration; the canonical source for
        // protocol-extends is `ProtocolBody.extends` from the AST.
        // Without this, `Eq extends PartialEq` round-trips with
        // empty super_protocols and the user-side
        // `lookup_protocol_method_for_type_with_args`'s
        // superprotocol-method walk (`find_superprotocol_method`)
        // returns None — every `v.ne(...)` on a user type that
        // implements `Eq` fails MethodNotFound despite the default
        // declaration on PartialEq.
        scan_protocol_supers(&mut metadata, root, verbose);
    }
    let bytes = bincode::serialize(&metadata)
        .context("bincode serialise CoreMetadata for sidecar emit")?;

    let sidecar = archive_path.with_extension("core_metadata");
    // Atomic publish — mirrors `verum_vbc::archive::write_archive_to_file`:
    // concurrent bakes (parallel sessions; build.rs racing a manual
    // precompiler run) target the same sidecar; a direct fs::write
    // truncates in place and a concurrent embed step reads a torn
    // file. Write a pid-suffixed sibling, then rename into place.
    let tmp = sidecar.with_extension(format!("core_metadata.tmp{}", std::process::id()));
    std::fs::write(&tmp, &bytes)
        .with_context(|| format!("write metadata sidecar tmp {}", tmp.display()))?;
    std::fs::rename(&tmp, &sidecar)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("publish metadata sidecar {}", sidecar.display()))?;

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
            // Skip `target/` trees — codegen/test-harness output, not
            // stdlib source.  Sweeping them poisoned the precompile:
            // `core/target/test/*.merged.vr` harness residue compiled
            // as if it were stdlib (garbage type-context, 355 of the
            // 580 FIELD-INTERN fallbacks) and perturbed the content
            // hash on every test run (cache thrash).
            if p.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
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
            // Skip `target/` trees — see compute_source_blake3's twin
            // guard (harness residue is not stdlib source).
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
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

/// Task #20 — scan every `.vr` file under `root` for public
/// `mount` declarations and populate `metadata.module_reexports`
/// with `(reexporting_module_path → [(item_name, source_module_path)])`
/// pairs.
///
/// Resolves leading-dot / `super.` / `cog.` segments relative to the
/// re-exporting file's own dotted module path.  Glob mounts and
/// relative-file mounts are skipped — only specific item re-exports
/// (Path with a final `Name` segment) survive into the metadata.
///
/// Without this pass, free functions re-exported through
/// `public mount .submod.{fn};` are unreachable at user-compile
/// time because the user-side `load_stdlib_from_embedded` builds
/// ExportTables from each function's DECLARING module only —
/// the re-export chains live in source and are otherwise lost when
/// the precompile artefact crosses the archive boundary.
fn scan_module_reexports(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    root: &Path,
    verbose: bool,
) {
    use std::collections::BTreeMap;
    use verum_ast::decl::Visibility;
    use verum_ast::ty::PathSegment;
    use verum_ast::{ItemKind, MountTree, MountTreeKind, Path as AstPath};
    use verum_common::{List, Text};

    /// Resolved leaf of a mount tree.
    struct ReexportLeaf {
        /// Locally-visible name (the alias if present, else the
        /// final path segment).
        local_name: String,
        /// The item's OWN declared name in `source_module` — the key
        /// that actually resolves there in `metadata.types` /
        /// `metadata.functions` / `metadata.protocols`. Same as
        /// `local_name` unless this leaf came from a `mount X as Y`
        /// rename (T0244) — e.g. `LayoutConstraint as Constraint`
        /// carries `local_name="Constraint"`, `true_name=
        /// "LayoutConstraint"`.
        true_name: String,
        /// Absolute dotted module path of the source.
        source_module: String,
        /// The module that RE-EXPORTS this leaf — the file's own
        /// module for top-level mounts, or the nested inline-module
        /// path for mounts inside `public module prelude { ... }`.
        /// Pre-fix every leaf was keyed under the FILE's module, so
        /// concrete prelude mounts (`super.text.format.format_debug`)
        /// landed in the `core` bucket instead of `core.prelude` —
        /// the PRELUDE-FREEFN defect (`f"{x:?}"` → unbound
        /// `format_debug`). Globs never had the bug because
        /// `glob_pairs` already carried `current_module` per entry.
        reexporting_module: String,
    }

    /// Resolve a `Path` plus an accumulated absolute prefix into
    /// `(absolute_module_path, optional_item_name)`.
    ///
    /// `accumulated_prefix` is the already-resolved Nested prefix
    /// (empty for a top-level Path).  When non-empty, `Relative`
    /// / `Super` / `Cog` markers on the inner path are ignored —
    /// the parent prefix is the anchor.  When empty, the markers
    /// are resolved against `current_module` exactly like
    /// `core_compiler::resolve_import_path`.
    fn resolve_path(
        p: &AstPath,
        accumulated_prefix: &str,
        current_module: &str,
        is_prefix: bool,
    ) -> Option<(String, Option<String>)> {
        if p.segments.is_empty() {
            return None;
        }
        let mut module_parts: Vec<String> = Vec::new();
        let mut item_name: Option<String> = None;
        let mut is_relative = false;
        let mut is_cog = false;
        let mut super_count: usize = 0;

        let last_idx = p.segments.len() - 1;
        for (i, seg) in p.segments.iter().enumerate() {
            match seg {
                PathSegment::Relative => is_relative = true,
                PathSegment::Super => super_count += 1,
                PathSegment::Cog => {
                    is_cog = true;
                    module_parts.clear();
                }
                PathSegment::SelfValue => {}
                PathSegment::Name(ident) => {
                    if !is_prefix && i == last_idx {
                        item_name = Some(ident.name.as_str().to_string());
                    } else {
                        module_parts.push(ident.name.as_str().to_string());
                    }
                }
            }
        }

        // When already nested under an absolute prefix, treat the
        // inner path as a continuation — markers are no-ops.
        if !accumulated_prefix.is_empty() {
            let mut resolved = accumulated_prefix.to_string();
            for part in &module_parts {
                resolved.push('.');
                resolved.push_str(part);
            }
            return Some((resolved, item_name));
        }

        // Top-level path: apply marker semantics against
        // `current_module`.
        let resolved = if is_relative || super_count > 0 {
            let cur: Vec<&str> = current_module.split('.').filter(|s| !s.is_empty()).collect();
            let kept = cur
                .get(..cur.len().saturating_sub(super_count))
                .unwrap_or(&[])
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>();
            let mut parts = kept;
            parts.extend(module_parts.clone());
            parts.join(".")
        } else if is_cog {
            // `cog.X.Y` — root-anchored absolute path.  In stdlib
            // bootstrap the cog root IS `core`, so prepend it.
            let mut parts: Vec<String> = vec!["core".to_string()];
            parts.extend(module_parts.clone());
            parts.join(".")
        } else {
            // Bare absolute path like `collections.List`.  Stdlib
            // files implicitly reference siblings under `core.`;
            // synthesise that prefix when the first segment is not
            // already `core`.
            if module_parts.first().map(|s| s.as_str()) == Some("core") {
                module_parts.join(".")
            } else if module_parts.is_empty() {
                String::new()
            } else {
                let mut parts: Vec<String> = vec!["core".to_string()];
                parts.extend(module_parts.clone());
                parts.join(".")
            }
        };

        Some((resolved, item_name))
    }

    fn walk_tree(
        tree: &MountTree,
        accumulated_prefix: &str,
        current_module: &str,
        out: &mut Vec<ReexportLeaf>,
        globs: &mut Vec<(String, String)>,
    ) {
        let alias_name: Option<String> = match &tree.alias {
            verum_common::Maybe::Some(a) => Some(a.name.as_str().to_string()),
            verum_common::Maybe::None => None,
        };
        match &tree.kind {
            MountTreeKind::Path(p) => {
                if let Some((module_path, item)) =
                    resolve_path(p, accumulated_prefix, current_module, false)
                {
                    if let Some(item_name) = item {
                        out.push(ReexportLeaf {
                            local_name: alias_name.unwrap_or_else(|| item_name.clone()),
                            true_name: item_name,
                            source_module: module_path,
                            reexporting_module: current_module.to_string(),
                        });
                    }
                }
            }
            MountTreeKind::Glob(p) => {
                // Task #27 — glob re-exports.  Resolve the glob's
                // path to an absolute source-module prefix and queue
                // it for post-pass expansion against
                // `metadata.{types, functions, protocols}` whose
                // `module_path` starts with that prefix.  Pre-fix
                // `mount core.prelude.*` (the implicit prelude
                // injected by every user file) lost EVERY transitively
                // re-exported stdlib name because the glob handler was
                // a TODO — `range`, `repeat`, `count_from`,
                // `Transducer.*`, etc. all surfaced as `unbound
                // variable` at use sites despite living in the
                // archive.
                if let Some((source_prefix, _)) =
                    resolve_path(p, accumulated_prefix, current_module, true)
                {
                    if !source_prefix.is_empty() {
                        globs.push((current_module.to_string(), source_prefix));
                    }
                }
            }
            MountTreeKind::Nested { prefix, trees } => {
                if let Some((module_path, _)) =
                    resolve_path(prefix, accumulated_prefix, current_module, true)
                {
                    for sub in trees.iter() {
                        walk_tree(sub, &module_path, current_module, out, globs);
                    }
                }
            }
            MountTreeKind::File { .. } => {
                // Relative file mounts are user-cog only.
            }
        }
    }

    // `local_name -> (true_name, source_module)`. `true_name` is the
    // item's own declared name at `source_module` — same as the key
    // unless the leaf came from a `mount X as Y` rename (T0244).
    fn visit_dir(
        root: &Path,
        dir: &Path,
        accum: &mut BTreeMap<String, BTreeMap<String, (String, String)>>,
        glob_pairs: &mut Vec<(String, String)>,
        explicit_leaves: &mut Vec<(String, String, String, String)>,
        inline_edges: &mut Vec<(String, String)>,
        files_visited: &mut usize,
        files_parsed: &mut usize,
        files_with_reexports: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip `target/` trees — see compute_source_blake3's twin
            // guard (harness residue is not stdlib source).
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                visit_dir(root, &path, accum, glob_pairs, explicit_leaves, inline_edges, files_visited, files_parsed, files_with_reexports);
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
            // Quick filter — full parse only when at least one
            // public mount declaration may exist.
            if !content.contains("public mount ") {
                continue;
            }
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
            let reexporting_module =
                crate::stdlib_index::file_path_to_module_path(&rel_str);

            let mut leaves: Vec<ReexportLeaf> = Vec::new();
            // Inline-module nesting: `public module prelude {
            // public mount super.base.*; ... }` lives in `core/mod.vr`
            // as `ItemKind::Module(prelude)` and its `mount super.base.*`
            // re-exports must surface under
            // `module_reexports["core.prelude"]`, not under
            // `"core"`.  Walk inline modules and re-issue mounts
            // with the nested module's dotted path as
            // `reexporting_module`.
            fn collect_from_items(
                items: &verum_common::List<verum_ast::Item>,
                reexporting_module: &str,
                leaves: &mut Vec<ReexportLeaf>,
                glob_pairs: &mut Vec<(String, String)>,
                inline_edges: &mut Vec<(String, String)>,
            ) {
                for item in items.iter() {
                    if let ItemKind::Mount(mount_decl) = &item.kind
                        && matches!(mount_decl.visibility, Visibility::Public)
                    {
                        walk_tree(
                            &mount_decl.tree,
                            "",
                            reexporting_module,
                            leaves,
                            glob_pairs,
                        );
                    } else if let ItemKind::Module(mod_decl) = &item.kind
                        && matches!(mod_decl.visibility, Visibility::Public)
                        && let verum_common::Maybe::Some(sub_items) = &mod_decl.items
                    {
                        let nested_path = format!(
                            "{}.{}",
                            reexporting_module,
                            mod_decl.name.name.as_str()
                        );
                        // Record the inline parent → child edge so the
                        // post-pass can surface the child's re-exports
                        // onto this parent (the "prelude pattern").
                        inline_edges
                            .push((reexporting_module.to_string(), nested_path.clone()));
                        collect_from_items(
                            sub_items,
                            &nested_path,
                            leaves,
                            glob_pairs,
                            inline_edges,
                        );
                    }
                }
            }
            collect_from_items(
                &module.items,
                &reexporting_module,
                &mut leaves,
                glob_pairs,
                inline_edges,
            );

            if leaves.is_empty() {
                continue;
            }
            *files_with_reexports += 1;
            // Key each leaf by ITS OWN reexporting module (the file's
            // module for top-level mounts, the nested inline-module
            // path for prelude mounts) — NOT the file's module for
            // all of them. Pre-fix the flat file-keying dropped every
            // concrete prelude mount into the `core` bucket.
            for leaf in leaves {
                // Record every EXPLICIT (Path/Nested) leaf under its
                // re-exporting module so the inline-submodule → parent
                // post-pass can replay a child's explicit re-exports onto
                // its parent without also dragging up the child's greedy
                // glob over-captures (the flattening guard).
                explicit_leaves.push((
                    leaf.reexporting_module.clone(),
                    leaf.local_name.clone(),
                    leaf.true_name.clone(),
                    leaf.source_module.clone(),
                ));
                let bucket = accum.entry(leaf.reexporting_module).or_default();
                // First-wins under name collision so the BTreeMap
                // iteration order is reproducible across runs.
                bucket
                    .entry(leaf.local_name)
                    .or_insert((leaf.true_name, leaf.source_module));
            }
        }
    }

    let mut accum: BTreeMap<String, BTreeMap<String, (String, String)>> = BTreeMap::new();
    let mut glob_pairs: Vec<(String, String)> = Vec::new();
    // (reexporting_module, local_name, true_name, source_module) for every
    // EXPLICIT Path/Nested leaf — the flattening-safe input to the
    // inline-submodule → parent propagation post-pass below.
    let mut explicit_leaves: Vec<(String, String, String, String)> = Vec::new();
    // (parent_module, child_module) for every inline `public module X { … }`.
    let mut inline_edges: Vec<(String, String)> = Vec::new();
    let mut files_visited = 0usize;
    let mut files_parsed = 0usize;
    let mut files_with_reexports = 0usize;
    visit_dir(
        root,
        root,
        &mut accum,
        &mut glob_pairs,
        &mut explicit_leaves,
        &mut inline_edges,
        &mut files_visited,
        &mut files_parsed,
        &mut files_with_reexports,
    );

    // Task #27 — expand glob mounts against the already-built
    // metadata.types / .functions / .protocols tables.  Every
    // public symbol whose `module_path` starts with the glob's
    // source prefix becomes a leaf under the re-exporting module.
    //
    // The expansion runs after the source-walk so nested-inline-module
    // collections (`core/mod.vr`'s `module prelude { mount super.base.*; }`)
    // and direct globs at file scope share one code path.  Pre-fix the
    // implicit prelude (`mount core.prelude.*`) lost every transitively
    // re-exported stdlib name because the glob branch in walk_tree was
    // a TODO.
    //
    // Source-shape considerations:
    //   * `core.base.*` matches `core.base.<simple>` for every simple
    //     in module_paths under that prefix (one level deep).  Stdlib
    //     convention is that submodules re-export themselves through
    //     mod.vr's specific mounts; deeper-than-one expansion would
    //     duplicate.
    //   * Glob TARGETs (the prefix) often resolve to a parent
    //     module that has its own re-exports.  We use the metadata's
    //     declaring-module path, not the glob-prefix's own exports —
    //     that's what the existing typechecker's `import_all_from_module`
    //     fallback does at runtime, so the precompile capture must
    //     mirror it.
    fn glob_matches(source_prefix: &str, candidate_module: &str) -> bool {
        // Task #27 — archive structure caveat: the VBC archive
        // collapses every `core/base/<sub>.vr` into a single
        // module entry named `core.base` (the mod.vr's path).  So
        // a glob `mount super.base.*` from `core.prelude` has
        // source_prefix=`core.base` and the matching declarations
        // live under candidate_module=`core.base` exactly — NOT
        // `core.base.iterator` (which doesn't exist in the
        // archive's module path indexing).
        //
        // Equal-match is the dominant case for stdlib glob
        // expansion; submodule-prefix matching (`core.X.*` with
        // `core.X.Y.Z` declarations) is the secondary case that
        // surfaces for hierarchies the archive DOES keep
        // distinct.  Both forms are valid — accept either.
        if candidate_module == source_prefix {
            return true;
        }
        if !candidate_module.starts_with(source_prefix) {
            return false;
        }
        let rest = &candidate_module[source_prefix.len()..];
        rest.starts_with('.')
    }
    for (reexporting_mp, source_prefix) in glob_pairs.iter() {
        let bucket = accum.entry(reexporting_mp.clone()).or_default();
        for (name, td) in metadata.types.iter() {
            if !glob_matches(source_prefix, td.module_path.as_str()) {
                continue;
            }
            // Glob expansion never renames — true_name == local_name.
            bucket.entry(name.as_str().to_string()).or_insert_with(|| {
                (name.as_str().to_string(), td.module_path.as_str().to_string())
            });
            // Also surface variant constructors so pattern matches
            // through globbed prelude work.
            if let verum_types::core_metadata::TypeDescriptorKind::Variant { cases } = &td.kind {
                for case in cases.iter() {
                    bucket.entry(case.name.as_str().to_string()).or_insert_with(|| {
                        (case.name.as_str().to_string(), td.module_path.as_str().to_string())
                    });
                }
            }
        }
        for (name, fd) in metadata.functions.iter() {
            if !glob_matches(source_prefix, fd.module_path.as_str()) {
                continue;
            }
            // Task #27 — extract the leaf-level simple name.
            //
            // archive_metadata's Pass 2 stores free functions under
            // TWO key shapes:
            //   1. Bare simple name (`"range"`) — first-wins under
            //      cross-module collisions.
            //   2. Module-path-qualified name
            //      (`"core.base.iterator.range"`) — registered
            //      unconditionally so a same-name free fn in
            //      another module still has a unique slot.
            //
            // Plus inherent-method entries (`"Type.method"`) we
            // need to skip — they belong to a Type carrier, not a
            // namespace leaf.
            //
            // Heuristic: for keys containing dots, take the last
            // segment as the leaf NAME.  Skip when the last
            // segment starts with an uppercase letter (it's a
            // variant constructor like `Maybe.Some`) AND the
            // second-to-last segment starts uppercase (it's
            // `Type.method` form, not module-path leaf).  Plain
            // module-path leaves (`core.base.iterator.range`)
            // pass through with last segment = `range`.
            let leaf_name = if let Some(dot) = name.as_str().rfind('.') {
                let tail = &name.as_str()[dot + 1..];
                let head = &name.as_str()[..dot];
                let head_last = head.rsplit('.').next().unwrap_or(head);
                let tail_upper = tail.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
                let head_last_upper = head_last
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false);
                // Inherent-method shape: previous segment is
                // a Type (uppercase) — skip.  Variant
                // constructors (e.g. `Maybe.Some`) also
                // hit this shape and are properly skipped
                // (they're propagated through the variant-case
                // walk on the type).
                if head_last_upper {
                    continue;
                }
                // Suppress closure synthetics (`foo$closure$N`)
                // — they're VBC-codegen artefacts, not
                // user-visible names.
                if tail.contains('$') {
                    continue;
                }
                // Suppress duplicate keys produced by the
                // `core.base.core.base.iterator.range`
                // path-doubling artefact (lazy-load
                // dispatch fallback registers them too).
                // Keep only the non-doubled form.
                if head.starts_with("core.") {
                    if let Some(rest) = head.strip_prefix("core.") {
                        if rest.starts_with("core.") {
                            continue;
                        }
                    }
                }
                let _ = tail_upper; // not used; reserved for future Type-vs-fn disambig
                tail.to_string()
            } else {
                // Bare simple key — leaf is the key itself.
                if name.as_str().contains('$') {
                    continue;
                }
                name.as_str().to_string()
            };
            // Glob expansion never renames — true_name == local_name
            // (`leaf_name` here, the leaf extracted from the metadata key).
            bucket
                .entry(leaf_name.clone())
                .or_insert_with(|| (leaf_name, fd.module_path.as_str().to_string()));
        }
        for (name, pd) in metadata.protocols.iter() {
            if !glob_matches(source_prefix, pd.module_path.as_str()) {
                continue;
            }
            bucket.entry(name.as_str().to_string()).or_insert_with(|| {
                (name.as_str().to_string(), pd.module_path.as_str().to_string())
            });
        }
    }

    // T0281 — inline-submodule → parent re-export propagation.
    //
    // The "prelude pattern" at `core/mod.vr` re-exports every
    // fundamental name ONLY through an inline
    // `public module prelude { public mount super.base.*;
    // public mount super.collections.List; … }`.  `core/mod.vr` itself
    // has NO top-level `public mount`, so the scan above keys every
    // prelude re-export under `accum["core.prelude"]` and leaves
    // `accum["core"]` empty of them.  A user's `mount core.{Maybe}`
    // then reads `core`'s (empty) export surface and fails E401 — the
    // group-mount cascade (base/data/unit_test.vr = 160 fails) — while
    // `mount core.prelude.Maybe` resolves.
    //
    // Mirror `verum_modules::exports::propagate_submodule_reexports`
    // (the source-path AST propagator) over the metadata buckets:
    // surface an inline public submodule's re-exports onto its parent.
    // Fully general — every inline `public module X { … }` edge
    // captured during the source walk propagates; no `prelude` name is
    // hardcoded.
    //
    // Flattening guard (the negative invariant: `core.base.data.Data`
    // must NOT become resolvable as `core.Data`).  A child contributes
    // to its parent ONLY:
    //   (1) its EXPLICIT Path/Nested leaves (`explicit_leaves`), and
    //   (2) for each `X.*` glob it holds, the glob TARGET's OWN
    //       re-export surface (`accum[target]`) — NOT the greedy
    //       path-prefix expansion that fills the child's own bucket.
    // `core.base`'s surface is exactly what `core/base/mod.vr`
    // re-exports (`public mount .maybe.{Maybe}`, `.memory.{Heap,
    // Shared}`, `collections.{List, Map, Set}`, …) — Maybe / Result /
    // List / Map / Heap / Shared, but NEVER `Data`, which `core.base`
    // only declares as a `public module data;` submodule and never
    // re-exports.  So the fundamental names surface at the root while
    // deep-submodule types stay unreachable there.
    //
    // Runs before canonicalisation so propagated leaves get the same
    // `source_module` truth-fixing as directly-scanned leaves.  Driven
    // to a fixed point (bounded) so a multi-level inline chain
    // converges; `added` counts only genuinely-new entries.
    {
        const MAX_PROP_DEPTH: usize = 16;
        for _ in 0..MAX_PROP_DEPTH {
            let mut added = 0usize;
            for (parent, child) in inline_edges.iter() {
                // (1) The child's explicit Path/Nested re-exports.
                for (rx, local, true_name, source) in explicit_leaves.iter() {
                    if rx != child {
                        continue;
                    }
                    let bucket = accum.entry(parent.clone()).or_default();
                    if !bucket.contains_key(local) {
                        bucket.insert(local.clone(), (true_name.clone(), source.clone()));
                        added += 1;
                    }
                }
                // (2) For every `X.*` glob the child holds, the glob
                //     target's OWN re-export surface — the flattening
                //     guard (deep-submodule members are excluded because
                //     the target module never re-exports them).
                for (rx, source_prefix) in glob_pairs.iter() {
                    if rx != child {
                        continue;
                    }
                    let surface: Vec<(String, (String, String))> = accum
                        .get(source_prefix)
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();
                    let bucket = accum.entry(parent.clone()).or_default();
                    for (local, pair) in surface {
                        if !bucket.contains_key(&local) {
                            bucket.insert(local, pair);
                            added += 1;
                        }
                    }
                }
            }
            if added == 0 {
                break;
            }
        }
    }

    // Canonicalise every Path-arm leaf's `source_module` against the
    // metadata tables before persisting. A mount like
    // `public mount super.text.format.format_display;` records the
    // MOUNT-CHAIN module path (`core.text.format`), but the archive
    // indexes each declaration under its DECLARING module path — which
    // for collapsed hierarchies is the parent (`core.text`), and for
    // free fns is also reachable via the qualified key
    // (`core.text.format.format_display` → fd.module_path). The
    // consumer (`import_all_from_module`'s metadata-driven leaf replay)
    // resolves leaves via `import_item_from_module(source, name)` and
    // silently drops any leaf whose module path the registry can't
    // serve — pre-fix the prelude's CONCRETE mounts (`format_debug`,
    // `format_display`, `read_to_string`, …) were captured with
    // unresolvable module paths and every `f"{x:?}"` failed
    // `unbound variable: format_debug` at user sites. Glob-expanded
    // leaves never had the problem because the expansion reads
    // `fd.module_path` (the truth) directly; this pass gives Path-arm
    // leaves the same truth source.
    let leaf_resolves = |local: &str, source: &str| -> bool {
        let qualified = Text::from(format!("{source}.{local}").as_str());
        if metadata.functions.get(&qualified).is_some() {
            return true;
        }
        let local_t = Text::from(local);
        if let Some(td) = metadata.types.get(&local_t) {
            if td.module_path.as_str() == source {
                return true;
            }
        }
        if let Some(pd) = metadata.protocols.get(&local_t) {
            if pd.module_path.as_str() == source {
                return true;
            }
        }
        if let Some(fd) = metadata.functions.get(&local_t) {
            if fd.module_path.as_str() == source {
                return true;
            }
        }
        false
    };
    let canonical_source = |local: &str, source: &str| -> Option<String> {
        // 1. Qualified free-fn key at the recorded path — the
        //    descriptor's own module_path is the declaring truth.
        let qualified = Text::from(format!("{source}.{local}").as_str());
        if let Some(fd) = metadata.functions.get(&qualified) {
            return Some(fd.module_path.as_str().to_string());
        }
        let local_t = Text::from(local);
        // 2. Type / protocol simple-name lookup, accepted when the
        //    declared path and the mount path agree up to hierarchy
        //    collapsing (one is a dotted prefix of the other).
        let path_compatible = |declared: &str| -> bool {
            declared == source
                || (source.starts_with(declared)
                    && source.as_bytes().get(declared.len()) == Some(&b'.'))
                || (declared.starts_with(source)
                    && declared.as_bytes().get(source.len()) == Some(&b'.'))
        };
        if let Some(td) = metadata.types.get(&local_t) {
            if path_compatible(td.module_path.as_str()) {
                return Some(td.module_path.as_str().to_string());
            }
        }
        if let Some(pd) = metadata.protocols.get(&local_t) {
            if path_compatible(pd.module_path.as_str()) {
                return Some(pd.module_path.as_str().to_string());
            }
        }
        // 3. Bare free-fn simple key with the same compatibility bound.
        if let Some(fd) = metadata.functions.get(&local_t) {
            if path_compatible(fd.module_path.as_str()) {
                return Some(fd.module_path.as_str().to_string());
            }
        }
        // 4. Ancestor probe: walk the mount path upward and retry the
        //    qualified key at each level — catches declarations homed
        //    under a collapsed parent (`core.text.format_display` when
        //    the mount said `core.text.format`).
        let mut prefix = source.to_string();
        while let Some(dot) = prefix.rfind('.') {
            prefix.truncate(dot);
            let q = Text::from(format!("{prefix}.{local}").as_str());
            if let Some(fd) = metadata.functions.get(&q) {
                return Some(fd.module_path.as_str().to_string());
            }
        }
        None
    };
    let mut canonicalised = 0usize;
    let mut unresolved: Vec<(String, String)> = Vec::new();
    for items in accum.values_mut() {
        for (local, (true_name, source)) in items.iter_mut() {
            // Probe/canonicalise by `true_name` — the metadata tables
            // are keyed by the item's OWN declared name, which for a
            // `mount X as Y` rename-alias (T0244) differs from `local`
            // (the alias key). Un-aliased leaves have `true_name ==
            // local`, so this is a no-op change for the common case.
            if leaf_resolves(true_name.as_str(), source.as_str()) {
                continue;
            }
            match canonical_source(true_name.as_str(), source.as_str()) {
                Some(fixed) => {
                    if fixed != *source {
                        *source = fixed;
                        canonicalised += 1;
                    }
                }
                None => unresolved.push((local.clone(), source.clone())),
            }
        }
    }
    if verbose {
        eprintln!(
            "verum stdlib precompile: canonicalised {} re-export leaves; {} left unresolved{}",
            canonicalised,
            unresolved.len(),
            if unresolved.is_empty() {
                String::new()
            } else {
                format!(
                    " (first: {})",
                    unresolved
                        .iter()
                        .take(5)
                        .map(|(l, s)| format!("{s}.{l}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );
    }

    let mut total_leaves = 0usize;
    for (reexporting_mp, items) in accum {
        let mut list: List<(Text, Text, Text)> = List::new();
        for (local, (true_name, source)) in items {
            list.push((
                Text::from(local.as_str()),
                Text::from(true_name.as_str()),
                Text::from(source.as_str()),
            ));
            total_leaves += 1;
        }
        metadata
            .module_reexports
            .insert(Text::from(reexporting_mp.as_str()), list);
    }

    if verbose {
        eprintln!(
            "verum stdlib precompile: scanned {} .vr files ({} parsed, {} with public mount), captured {} re-export leaves across {} re-exporting modules",
            files_visited,
            files_parsed,
            files_with_reexports,
            total_leaves,
            metadata.module_reexports.len(),
        );
    }
}

/// Task #23 — scan every `.vr` file under `root` for
/// `implement<...> Protocol<Args> for Type` declarations and update
/// the matching `metadata.implementations[]` entries with the
/// text-rendered protocol-argument list.
///
/// The VBC archive's `TypeImpl` carries only `protocol: ProtocolId`
/// (the bare protocol name) — protocol arguments are dropped at
/// codegen time.  This source-walk recovers them so
/// `register_stdlib_impls_for_target` at infer/core.rs can populate
/// `ProtocolImpl.protocol_args` and the typechecker's
/// `can_convert_residual` probe finds `FromResidual<Result<...>>`
/// impls instead of giving up at `impl_.protocol_args.first() ==
/// None`.
///
/// Matching is `(target_type, protocol)`-keyed — the most-specific
/// impl wins under collision (first-wins, BTreeSet iteration
/// produces deterministic ordering).
fn scan_implementation_protocol_args(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    root: &Path,
    verbose: bool,
) {
    use std::collections::BTreeMap;
    use verum_ast::ItemKind;
    use verum_ast::decl::ImplKind;
    use verum_ast::pretty;
    use verum_ast::ty::GenericArg;
    use verum_common::{List, Text};

    // Source-side key: (target_type_simple_name, protocol_simple_name) →
    // rendered protocol-arg text list.  Captured BEFORE matching against
    // metadata.implementations so collisions across modules are
    // deterministically resolved by source-walk order.
    let mut found: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    let mut files_visited = 0usize;
    let mut files_parsed = 0usize;
    let mut impls_captured = 0usize;

    fn target_simple_name(ty: &verum_ast::ty::Type) -> Option<String> {
        use verum_ast::ty::{PathSegment, TypeKind};
        match &ty.kind {
            TypeKind::Path(path) => path.segments.last().and_then(|s| match s {
                PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                _ => None,
            }),
            TypeKind::Generic { base, .. } => target_simple_name(base),
            _ => None,
        }
    }

    fn protocol_simple_name(path: &verum_ast::ty::Path) -> Option<String> {
        use verum_ast::ty::PathSegment;
        path.segments.last().and_then(|s| match s {
            PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
            _ => None,
        })
    }

    fn render_generic_arg(arg: &GenericArg) -> Option<String> {
        match arg {
            GenericArg::Type(ty) => Some(pretty::format_type(ty).as_str().to_string()),
            GenericArg::Lifetime(_) | GenericArg::Const(_) | GenericArg::Binding(_) => None,
        }
    }

    fn visit_dir(
        dir: &Path,
        found: &mut BTreeMap<(String, String), Vec<String>>,
        files_visited: &mut usize,
        files_parsed: &mut usize,
        impls_captured: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip `target/` trees — see compute_source_blake3's twin
            // guard (harness residue is not stdlib source).
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                visit_dir(&path, found, files_visited, files_parsed, impls_captured);
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
            // Quick filter — full parse only when an `implement` keyword
            // appears.  `verum_fast_parser` is fast but parsing every
            // .vr is still ~50ms on the full tree; the gate keeps the
            // scan to the ~hundred files that actually declare impls.
            if !content.contains("implement") {
                continue;
            }
            let mut parser = verum_fast_parser::Parser::new(&content);
            let module = match parser.parse_module() {
                Ok(m) => m,
                Err(_) => continue,
            };
            *files_parsed += 1;

            for item in module.items.iter() {
                if let ItemKind::Impl(impl_decl) = &item.kind {
                    if let ImplKind::Protocol {
                        protocol,
                        protocol_args,
                        for_type,
                    } = &impl_decl.kind
                    {
                        let (Some(target), Some(proto_name)) = (
                            target_simple_name(for_type),
                            protocol_simple_name(protocol),
                        ) else {
                            continue;
                        };
                        let args: Vec<String> = protocol_args
                            .iter()
                            .filter_map(render_generic_arg)
                            .collect();
                        if args.is_empty() {
                            // Non-generic protocols (no args) — skip,
                            // empty list is already the default.
                            continue;
                        }
                        // First-wins per (target, protocol) key.  If a
                        // type implements the same protocol under
                        // different cfg gates, the source-walk's
                        // first encounter pins the arg shape; the
                        // collision is rare and the alternative
                        // (last-wins, multi-arity) doesn't help the
                        // FromResidual case any.
                        let key = (target, proto_name);
                        if let std::collections::btree_map::Entry::Vacant(slot) =
                            found.entry(key)
                        {
                            slot.insert(args);
                            *impls_captured += 1;
                        }
                    }
                }
            }
        }
    }

    visit_dir(
        root,
        &mut found,
        &mut files_visited,
        &mut files_parsed,
        &mut impls_captured,
    );

    let mut populated: usize = 0;
    for impl_desc in metadata.implementations.iter_mut() {
        let key = (
            impl_desc.target_type.as_str().to_string(),
            impl_desc.protocol.as_str().to_string(),
        );
        if !impl_desc.protocol_args.is_empty() {
            // Archive-carried per-impl args (VBC v2.7) are
            // authoritative — the (target, protocol) first-wins scan
            // below collapses sibling impls (#47 FromResidual tail).
            continue;
        }
        if let Some(args) = found.get(&key) {
            let mut list: List<Text> = List::new();
            for a in args {
                list.push(Text::from(a.as_str()));
            }
            impl_desc.protocol_args = list;
            populated += 1;
        }
    }

    if verbose {
        eprintln!(
            "verum stdlib precompile: scanned {} .vr files ({} parsed) for impl protocol-args; captured {} unique impls, populated {} of {} metadata entries",
            files_visited,
            files_parsed,
            impls_captured,
            populated,
            metadata.implementations.len(),
        );
    }
}

/// **Task #44 audit fix** — walk every `.vr` file under `root` and
/// for each `type X is protocol extends Y, Z, ... { ... };` decl,
/// capture the `extends` clause's protocol names into
/// `metadata.protocols[X].super_protocols`.  The VBC archive's
/// `ty.protocols` field captures concrete-type IMPLEMENTS lists, not
/// protocol-declaration EXTENDS clauses — so without this source-walk
/// every super-protocol relationship (`Eq extends PartialEq`,
/// `Ord extends Eq + PartialOrd`, …) is lost after archive round-trip,
/// breaking `find_superprotocol_method`'s default-method walk.
fn scan_protocol_supers(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    root: &Path,
    verbose: bool,
) {
    use std::collections::BTreeMap;
    use verum_ast::ItemKind;
    use verum_ast::decl::TypeDeclBody;
    use verum_common::{List, Text};

    let mut captured: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut files_visited = 0usize;
    let mut files_parsed = 0usize;
    let mut protos_seen = 0usize;

    fn visit_dir(
        dir: &Path,
        captured: &mut BTreeMap<String, Vec<String>>,
        files_visited: &mut usize,
        files_parsed: &mut usize,
        protos_seen: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip `target/` trees — see compute_source_blake3's twin
            // guard (harness residue is not stdlib source).
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                visit_dir(&path, captured, files_visited, files_parsed, protos_seen);
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
            // Quick filter: `is protocol` substring eliminates files
            // with no protocol decls (most of the stdlib).
            if !content.contains("is protocol") {
                continue;
            }
            let mut parser = verum_fast_parser::Parser::new(&content);
            let module = match parser.parse_module() {
                Ok(m) => m,
                Err(_) => continue,
            };
            *files_parsed += 1;

            collect_supers_from_items(module.items.as_slice(), captured, protos_seen);
        }
    }

    /// Walk items recursively (also descends into inline `module X { ... }`
    /// blocks where many stdlib protocols live).
    fn collect_supers_from_items(
        items: &[verum_ast::Item],
        captured: &mut BTreeMap<String, Vec<String>>,
        protos_seen: &mut usize,
    ) {
        for item in items.iter() {
            match &item.kind {
                ItemKind::Type(type_decl) => {
                    if let TypeDeclBody::Protocol(body) = &type_decl.body {
                        *protos_seen += 1;
                        let proto_name = type_decl.name.name.as_str().to_string();
                        let supers: Vec<String> = body
                            .extends
                            .iter()
                            .filter_map(|ty| match &ty.kind {
                                verum_ast::ty::TypeKind::Path(path) => {
                                    path.segments.last().and_then(|seg| match seg {
                                        verum_ast::ty::PathSegment::Name(ident) => {
                                            Some(ident.name.as_str().to_string())
                                        }
                                        _ => None,
                                    })
                                }
                                _ => None,
                            })
                            .collect();
                        if !supers.is_empty() {
                            // first-wins per protocol name (collisions
                            // across modules are vanishingly rare and
                            // the source-walk's first encounter pins).
                            captured.entry(proto_name).or_insert(supers);
                        }
                    }
                }
                ItemKind::Module(mod_decl) => {
                    if let verum_common::Maybe::Some(sub_items) = &mod_decl.items {
                        collect_supers_from_items(sub_items.as_slice(), captured, protos_seen);
                    }
                }
                _ => {}
            }
        }
    }

    visit_dir(
        root,
        &mut captured,
        &mut files_visited,
        &mut files_parsed,
        &mut protos_seen,
    );

    let mut populated = 0usize;
    for (proto_name, supers) in &captured {
        let key = Text::from(proto_name.as_str());
        if let Some(pd) = metadata.protocols.get_mut(&key) {
            // Only overwrite if archive didn't already capture them
            // (currently always empty, but defensive against future
            // changes that DO populate from VBC).
            if pd.super_protocols.is_empty() {
                pd.super_protocols = supers.iter().map(|s| Text::from(s.as_str())).collect::<List<_>>();
                populated += 1;
            }
        }
    }

    if verbose {
        eprintln!(
            "verum stdlib precompile: scanned {} .vr files ({} parsed, {} protocol decls seen), populated super_protocols on {} of {} captured protocols",
            files_visited,
            files_parsed,
            protos_seen,
            populated,
            captured.len(),
        );
    }
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
            // Skip `target/` trees — see compute_source_blake3's twin
            // guard (harness residue is not stdlib source).
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
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

            // Helper to register a method under both bare-name and
            // `<Receiver>.<method>` qualified-name keys so the
            // metadata's `<parent>.<simple>` lookup style hits.
            let record_qualified = |receiver: &str,
                                    ident: &verum_ast::Ident,
                                    name_index: &mut HashMap<String, Vec<Candidate>>| {
                record_decl(ident, name_index);
                let qualified = format!("{}.{}", receiver, ident.name.as_str());
                let span = ident.span;
                name_index
                    .entry(qualified)
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

            // Recursively walk a function body's statements looking
            // for nested `Item` declarations — function-local
            // `const X = ...;` (and nested fn defs / type aliases)
            // get hoisted by codegen as zero-arg functions whose
            // FunctionDescriptor lands in `metadata.functions`
            // alongside the parent.  Without this walk those
            // descriptors get no source-side match.
            fn walk_block_items(
                block: &verum_ast::Block,
                f: &mut dyn FnMut(&verum_ast::Item),
            ) {
                for stmt in block.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Item(item) = &stmt.kind {
                        f(item);
                        // Recurse into nested function bodies for
                        // doubly-nested locals.
                        if let ItemKind::Function(fd) = &item.kind
                            && let verum_common::Maybe::Some(body) = &fd.body
                            && let verum_ast::decl::FunctionBody::Block(b) = body
                        {
                            walk_block_items(b, f);
                        }
                    }
                }
            }

            // Walk every function body — both top-level and impl-
            // block methods — to pick up function-local
            // declarations.  Codegen hoists `const X = ...;` and
            // nested fn defs as zero-arg FunctionDescriptors that
            // land in metadata.functions; the source-side index
            // must register them too.
            let mut on_item = |item: &verum_ast::Item| {
                match &item.kind {
                    ItemKind::Function(d) => record_decl(&d.name, name_index),
                    ItemKind::Type(d) => record_decl(&d.name, name_index),
                    ItemKind::Protocol(d) => record_decl(&d.name, name_index),
                    ItemKind::Const(d) => record_decl(&d.name, name_index),
                    ItemKind::Static(d) => record_decl(&d.name, name_index),
                    _ => {}
                }
            };
            let walk_fn_body = |fd: &verum_ast::decl::FunctionDecl,
                                on_item: &mut dyn FnMut(&verum_ast::Item)| {
                if let verum_common::Maybe::Some(body) = &fd.body
                    && let verum_ast::decl::FunctionBody::Block(b) = body
                {
                    walk_block_items(b, on_item);
                }
            };
            for item in &module.items {
                match &item.kind {
                    ItemKind::Function(fd) => walk_fn_body(fd, &mut on_item),
                    ItemKind::Impl(d) => {
                        for impl_item in d.items.iter() {
                            if let verum_ast::decl::ImplItemKind::Function(fd) = &impl_item.kind {
                                walk_fn_body(fd, &mut on_item);
                            }
                        }
                    }
                    ItemKind::Context(d) => {
                        for m in d.methods.iter() {
                            walk_fn_body(m, &mut on_item);
                        }
                    }
                    _ => {}
                }
            }
            drop(on_item);

            for item in &module.items {
                match &item.kind {
                    ItemKind::Function(d) => record_decl(&d.name, name_index),
                    ItemKind::Type(d) => record_decl(&d.name, name_index),
                    ItemKind::Protocol(d) => record_decl(&d.name, name_index),
                    ItemKind::Const(d) => record_decl(&d.name, name_index),
                    ItemKind::Static(d) => record_decl(&d.name, name_index),
                    // Contexts register BOTH a TypeDescriptor and a
                    // ProtocolDescriptor on the metadata side (the
                    // context itself is a type; its method surface is
                    // a protocol).  Both descriptors carry the same
                    // simple name and module_path, so a single index
                    // entry covers both.
                    ItemKind::Context(d) => {
                        record_decl(&d.name, name_index);
                        // Register every method as
                        // `<ContextName>.<method>` — context-impl
                        // methods land in metadata.functions with
                        // parent_type set to the context name.
                        for m in d.methods.iter() {
                            record_qualified(d.name.as_str(), &m.name, name_index);
                        }
                    }
                    // Impl blocks carry inherent / protocol-impl
                    // methods + associated types + consts whose
                    // FunctionDescriptor.parent_type points at the
                    // impl receiver — so the metadata side names them
                    // `<Receiver>.<method>` and our index must
                    // register both forms.
                    ItemKind::Impl(d) => {
                        // Resolve the receiver's last-segment name
                        // for the qualified-name shape used by
                        // metadata.  Handles both bare `Type` and
                        // `Generic { base, args }` carriers (e.g.
                        // `implement<A> ActorMesh<A> { … }`); peels
                        // the `Generic` wrapper to reach the
                        // underlying `Path` segment.
                        fn extract_receiver_name(ty: &verum_ast::Type) -> Option<String> {
                            use verum_ast::ty::TypeKind;
                            match &ty.kind {
                                TypeKind::Path(p) => {
                                    Some(p.last_segment_name().to_string())
                                }
                                TypeKind::Generic { base, .. } => {
                                    extract_receiver_name(base)
                                }
                                // Primitive types — `implement Display
                                // for Bool {...}` / `implement Bool
                                // {...}` use these dedicated variants
                                // rather than `Path("Bool")`.  Method
                                // descriptors land in metadata as
                                // `Bool.fmt`, `Int.from_bits`, etc.
                                TypeKind::Bool => Some("Bool".to_string()),
                                TypeKind::Int => Some("Int".to_string()),
                                TypeKind::Float => Some("Float".to_string()),
                                TypeKind::Char => Some("Char".to_string()),
                                TypeKind::Text => Some("Text".to_string()),
                                TypeKind::Unit => Some("Unit".to_string()),
                                // Built-in container shapes — `[T]`
                                // is `TypeKind::Slice(_)` in the AST,
                                // metadata names methods on it as
                                // `Slice.<method>`.  Tuples / arrays
                                // get similar treatment if they ship
                                // protocol impls.
                                TypeKind::Slice(_) => Some("Slice".to_string()),
                                TypeKind::Array { .. } => Some("Array".to_string()),
                                TypeKind::Tuple(_) => Some("Tuple".to_string()),
                                // Reference receivers — `implement
                                // <T> BitAnd<&Set<T>> for &Set<T>`
                                // wraps the actual receiver; the
                                // metadata uses the unwrapped name.
                                TypeKind::Reference { inner, .. } => extract_receiver_name(inner),
                                _ => None,
                            }
                        }
                        let receiver_name: Option<String> = match &d.kind {
                            verum_ast::decl::ImplKind::Inherent(ty)
                            | verum_ast::decl::ImplKind::Protocol { for_type: ty, .. } => {
                                extract_receiver_name(ty)
                            }
                        };
                        for impl_item in d.items.iter() {
                            match &impl_item.kind {
                                verum_ast::decl::ImplItemKind::Function(fd) => {
                                    if let Some(ref recv) = receiver_name {
                                        record_qualified(recv, &fd.name, name_index);
                                    } else {
                                        record_decl(&fd.name, name_index);
                                    }
                                }
                                verum_ast::decl::ImplItemKind::Type { name, .. }
                                | verum_ast::decl::ImplItemKind::Const { name, .. } => {
                                    if let Some(ref recv) = receiver_name {
                                        record_qualified(recv, name, name_index);
                                    } else {
                                        record_decl(name, name_index);
                                    }
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

    /// Strip codegen synthesis decoration off a function name to
    /// recover its source-side parent.  Three forms are recognised
    /// — every other unmatched function is a true codegen
    /// synthetic with no source counterpart.
    ///
    /// 1. **Trailing `$<tag>$<digits>`**: closure / spawn-block
    ///    lowering. Stack-peeled iteratively
    ///    (`fn$closure$8$spawn$9` → `fn`).
    ///
    /// 2. **Leading `__tls_init_<NAME>`**: the codegen emits one
    ///    `__tls_init_<NAME>` zero-arg fn per `static <NAME>` decl
    ///    to lazily initialise the thread-local on first read.
    ///    The synthetic inherits the static's source location.
    fn strip_synthesis_suffix(name: &str) -> Option<&str> {
        if let Some(stripped) = name.strip_prefix("__tls_init_") {
            return Some(stripped);
        }
        const MARKERS: &[&str] = &["$closure$", "$spawn$"];
        let mut current = name;
        let mut peeled_any = false;
        loop {
            let cut = MARKERS
                .iter()
                .filter_map(|m| current.rfind(m).map(|i| (i, *m)))
                .max_by_key(|(i, _)| *i);
            let Some((idx, marker)) = cut else {
                break;
            };
            let tail = &current[idx + marker.len()..];
            if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
                break;
            }
            current = &current[..idx];
            peeled_any = true;
        }
        if peeled_any { Some(current) } else { None }
    }

    fn populate<V, F>(
        map: &mut verum_common::OrderedMap<Text, V>,
        name_index: &HashMap<String, Vec<Candidate>>,
        get_module_and_name: F,
        get_decl_span: impl Fn(&mut V) -> &mut Maybe<DeclSpan>,
        unmatched_log: &mut Vec<String>,
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
            // Primary: exact name match.
            let primary = name_index
                .get(name)
                .and_then(|cands| resolve(cands, module_path));
            // Fallback: strip `$closure$<N>` suffix and look up the
            // parent function — the closure inherits the parent's
            // span (every anonymous closure under `f` points back
            // at the source file site of `f`).
            let found = primary.or_else(|| {
                strip_synthesis_suffix(name).and_then(|parent| {
                    name_index
                        .get(parent)
                        .and_then(|cands| resolve(cands, module_path))
                })
            });
            if let Some(found) = found {
                *get_decl_span(val) = Maybe::Some(found.decl_span.clone());
                populated += 1;
            } else if unmatched_log.len() < 30 {
                unmatched_log.push(format!("{}::{}", module_path, name));
            }
        }
        populated
    }

    let total_types = metadata.types.len();
    let mut unmatched_types: Vec<String> = Vec::new();
    let populated_types = populate(
        &mut metadata.types,
        &name_index,
        |t| (t.module_path.as_str(), t.name.as_str()),
        |t| &mut t.decl_span,
        &mut unmatched_types,
    );

    let total_fns = metadata.functions.len();
    let mut unmatched_fns: Vec<String> = Vec::new();
    let populated_fns = populate(
        &mut metadata.functions,
        &name_index,
        |f| (f.module_path.as_str(), f.name.as_str()),
        |f| &mut f.decl_span,
        &mut unmatched_fns,
    );

    let total_protos = metadata.protocols.len();
    let mut unmatched_protos: Vec<String> = Vec::new();
    let populated_protos = populate(
        &mut metadata.protocols,
        &name_index,
        |p| (p.module_path.as_str(), p.name.as_str()),
        |p| &mut p.decl_span,
        &mut unmatched_protos,
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
        if !unmatched_types.is_empty() {
            eprintln!("  unmatched types (sample): {:?}", unmatched_types);
        }
        if !unmatched_protos.is_empty() {
            eprintln!("  unmatched protocols (sample): {:?}", unmatched_protos);
        }
        if !unmatched_fns.is_empty() {
            eprintln!("  unmatched functions (sample): {:?}", unmatched_fns);
        }
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

/// Register every PUBLIC top-level free function under its
/// FILE-declared dotted module key
/// (`core.time.duration_parse.parse`), rebuilding the descriptor
/// from the parsed AST.
///
/// Rationale (2026-07-09): the archive-side Pass 2 keys free fns by
/// bare name (first-wins) + `<archive_module>.<name>` where
/// `archive_module` is the per-DIRECTORY compile unit.  File-grained
/// qualification is lost, and same-named free fns in sibling files
/// (duration_parse.parse / rfc3339.parse under `core.time`) collide —
/// the loser has NO descriptor anywhere.  This walk restores both:
/// the fully-qualified key the typechecker's module-path call
/// resolution probes, and the collision-dropped descriptors.
///
/// Type strings are rendered best-effort from the AST in the same
/// dialect `parse_descriptor_type_string` consumes; exotic shapes
/// degrade to their head name (same contract as the archive's
/// `__opaque_type_N` precedent).  First-wins: existing keys are
/// never overwritten.
fn inject_declared_module_free_fn_keys(
    metadata: &mut verum_types::core_metadata::CoreMetadata,
    root: &Path,
    verbose: bool,
) {
    /// BAKED-DEFAULT-ARG-1: render a default-value expression as
    /// re-parseable source text. Literal shapes only (the stdlib
    /// convention); anything more exotic returns None and the caller
    /// keeps just the arity-relaxation flag.
    fn render_literal_expr(
        expr: &verum_ast::Expr,
    ) -> verum_common::Maybe<verum_common::Text> {
        use verum_common::{Maybe, Text};
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::LiteralKind;
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => Maybe::Some(Text::from(i.value.to_string().as_str())),
                LiteralKind::Float(f) => {
                    Maybe::Some(Text::from(format!("{:?}", f.value).as_str()))
                }
                LiteralKind::Bool(b) => Maybe::Some(Text::from(if *b { "true" } else { "false" })),
                LiteralKind::Text(s) => {
                    Maybe::Some(Text::from(format!("{:?}", s.as_str()).as_str()))
                }
                _ => Maybe::None,
            },
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr: inner,
            } => match &inner.kind {
                ExprKind::Literal(lit) => match &lit.kind {
                    LiteralKind::Int(i) => {
                        Maybe::Some(Text::from(format!("-{}", i.value).as_str()))
                    }
                    LiteralKind::Float(f) => {
                        Maybe::Some(Text::from(format!("-{:?}", f.value).as_str()))
                    }
                    _ => Maybe::None,
                },
                _ => Maybe::None,
            },
            _ => Maybe::None,
        }
    }

    fn render_type(ty: &verum_ast::ty::Type) -> String {
        use verum_ast::ty::TypeKind as K;
        match &ty.kind {
            K::Unit => "Unit".to_string(),
            K::Never => "Never".to_string(),
            K::Bool => "Bool".to_string(),
            K::Int => "Int".to_string(),
            K::Float => "Float".to_string(),
            K::Char => "Char".to_string(),
            K::Text => "Text".to_string(),
            K::Path(path) => path.last_segment_name().to_string(),
            K::Generic { base, args } => {
                let rendered: Vec<String> = args
                    .iter()
                    .filter_map(|a| match a {
                        verum_ast::ty::GenericArg::Type(t) => Some(render_type(t)),
                        _ => None,
                    })
                    .collect();
                if rendered.is_empty() {
                    render_type(base)
                } else {
                    format!("{}<{}>", render_type(base), rendered.join(", "))
                }
            }
            K::Reference { inner, .. }
            | K::CheckedReference { inner, .. }
            | K::UnsafeReference { inner, .. } => render_type(inner),
            K::Tuple(types) => {
                let rendered: Vec<String> = types.iter().map(render_type).collect();
                format!("({})", rendered.join(", "))
            }
            K::Slice(inner) => format!("List<{}>", render_type(inner)),
            // Exotic shapes degrade to an opaque head — same class as
            // the archive's `__opaque_type_N` strings.
            _ => "__opaque_src".to_string(),
        }
    }

    fn visit_dir(
        root: &Path,
        dir: &Path,
        metadata: &mut verum_types::core_metadata::CoreMetadata,
        injected: &mut usize,
    ) {
        use verum_ast::ItemKind;
        use verum_common::{List, Maybe, Text};
        use verum_types::core_metadata::{FunctionDescriptor, GenericParam, ParamDescriptor};

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                visit_dir(root, &path, metadata, injected);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("vr") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // Quick filter: no public fn, nothing to do.
            if !content.contains("public fn ") {
                continue;
            }
            let mut parser = verum_fast_parser::Parser::new(&content);
            let module = match parser.parse_module() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let rel_path = match path.strip_prefix(root) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
            let source_module = crate::stdlib_index::file_path_to_module_path(&rel_str);

            for item in &module.items {
                let ItemKind::Function(fd) = &item.kind else {
                    continue;
                };
                if !matches!(fd.visibility, verum_ast::decl::Visibility::Public) {
                    continue;
                }
                let simple = fd.name.name.as_str();
                let qualified: Text = format!("{}.{}", source_module, simple).into();
                if metadata.functions.contains_key(&qualified) {
                    continue;
                }
                let params: List<ParamDescriptor> = fd
                    .params
                    .iter()
                    .filter_map(|p| match &p.kind {
                        verum_ast::decl::FunctionParamKind::Regular {
                            pattern,
                            ty,
                            default_value,
                        } => {
                            let pname = match &pattern.kind {
                                verum_ast::pattern::PatternKind::Ident { name, .. } => {
                                    name.name.as_str().to_string()
                                }
                                _ => "_".to_string(),
                            };
                            // BAKED-DEFAULT-ARG-1: carry the declared
                            // default straight from the AST — literal
                            // shapes render exactly; anything else
                            // keeps only the arity-relaxation flag.
                            let (has_default, default_literal) = match default_value {
                                verum_common::Maybe::Some(expr) => {
                                    (true, render_literal_expr(expr))
                                }
                                verum_common::Maybe::None => (false, Maybe::None),
                            };
                            Some(ParamDescriptor {
                                name: Text::from(pname.as_str()),
                                ty: Text::from(render_type(ty).as_str()),
                                has_default,
                                default_literal,
                            })
                        }
                        // Free functions have no self; skip any
                        // receiver-shaped params defensively.
                        _ => None,
                    })
                    .collect();
                let return_type: Text = match &fd.return_type {
                    Maybe::Some(t) => Text::from(render_type(t).as_str()),
                    Maybe::None => Text::from("Unit"),
                };
                let generic_params: List<GenericParam> = fd
                    .generics
                    .iter()
                    .filter_map(|g| {
                        let name = match &g.kind {
                            verum_ast::ty::GenericParamKind::Type { name, .. } => {
                                name.name.as_str().to_string()
                            }
                            verum_ast::ty::GenericParamKind::HigherKinded { name, .. } => {
                                name.name.as_str().to_string()
                            }
                            _ => return None,
                        };
                        Some(GenericParam {
                            name: Text::from(name.as_str()),
                            bounds: List::new(),
                            default: Maybe::None,
                            type_bounds: List::new(),
                        })
                    })
                    .collect();
                let descriptor = FunctionDescriptor {
                    name: Text::from(simple),
                    module_path: Text::from(source_module.as_str()),
                    generic_params,
                    params,
                    return_type,
                    contexts: List::new(),
                    is_async: fd.is_async,
                    is_unsafe: fd.is_unsafe,
                    intrinsic_id: Maybe::None,
                    parent_type: Maybe::None,
                    impl_generic_names: List::new(),
                    is_const: false,
                    decl_span: Maybe::None,
                };
                metadata.functions.insert(qualified, descriptor);
                *injected += 1;
            }
        }
    }

    let mut injected: usize = 0;
    visit_dir(root, root, metadata, &mut injected);
    if verbose {
        eprintln!(
            "verum stdlib precompile: injected {} file-module-qualified free-fn keys",
            injected
        );
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

    /// Pin the prelude re-export capture: the inline
    /// `public module prelude { ... }` in `core/mod.vr` mixes glob
    /// mounts (`super.base.*`) with CONCRETE named mounts
    /// (`super.text.format.format_debug`, `super.io.print`,
    /// `super.collections.List`, nested `super.math.{sin, ...}`).
    /// The metadata `module_reexports["core.prelude"]` bucket must
    /// carry BOTH families — losing the concrete leaves is exactly
    /// the PRELUDE-FREEFN defect (`f"{x:?}"` → unbound
    /// `format_debug` at every user site).
    #[test]
    fn scan_module_reexports_captures_prelude_concrete_mounts() {
        let core_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("core"))
            .expect("workspace root");
        if !core_dir.join("mod.vr").is_file() {
            eprintln!("skipping: core/mod.vr not found at {core_dir:?}");
            return;
        }
        // `file_path_to_module_path` prepends "core" to every relative
        // path, so the scan root must BE the `core/` dir for `mod.vr`
        // to resolve to module `core` (and its inline prelude to
        // `core.prelude`).
        let mut metadata = verum_types::core_metadata::CoreMetadata::default();
        scan_module_reexports(&mut metadata, &core_dir, true);
        let bucket = metadata
            .module_reexports
            .get(&verum_common::Text::from("core.prelude"))
            .expect("core.prelude bucket must exist");
        let names: Vec<String> = bucket
            .iter()
            .map(|(local, _, _)| local.as_str().to_string())
            .collect();
        eprintln!("core.prelude leaves: {}", names.len());
        for probe in [
            "format_debug",
            "format_display",
            "print",
            "read_to_string",
            "List",
            "Mutex",
            "Duration",
            "sin",
            "range",
        ] {
            let hit = bucket.iter().find(|(l, _, _)| l.as_str() == probe);
            eprintln!(
                "  {probe}: {:?}",
                hit.map(|(_, _, s)| s.as_str().to_string())
            );
        }
        assert!(
            bucket.iter().any(|(l, _, _)| l.as_str() == "format_debug"),
            "concrete prelude mount format_debug must be captured"
        );
        assert!(
            bucket.iter().any(|(l, _, _)| l.as_str() == "sin"),
            "nested-brace prelude mount sin must be captured"
        );
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
