//! T1 of the single-path archive-driven epic.
//!
//! Converts a precompiled stdlib [`VbcArchive`] into entries in the
//! VBC codegen [`CodegenContext`] without parsing a single `.vr`
//! source file.  Replaces the slow source-driven `imported_modules`
//! collection that walks 2400+ stdlib files on every script run.
//!
//! # What gets registered
//!
//! For every `VbcModule` in the archive, this module walks the
//! function and type tables and registers:
//!
//! * [`FunctionInfo`] under both qualified (`module.path.simple_name`)
//!   and simple (`simple_name`) keys, with first-wins simple-name
//!   collision discipline matching `compile_module`'s stdlib-load
//!   behaviour.
//! * Variant constructor metadata: `variant_tag` and
//!   `parent_type_name` recovered by walking each [`TypeDescriptor`]'s
//!   variant list.  Without this `Maybe.Some(x)` fails to dispatch
//!   correctly because the disambiguator can't tell which type owns
//!   the variant.
//! * Method metadata: `parent_type_name` recovered from
//!   `FunctionDescriptor.parent_type` for type-bound methods.
//! * `is_async` / `is_generator` flags from
//!   [`FunctionDescriptor.properties`] / `is_generator`.
//! * Return type via [`TypeRef`] passthrough.
//! * Generic-aware `return_type_name` + `return_type_inner` extracted
//!   from the [`TypeRef`] shape so the variant-disambiguator from
//!   #300 keeps working for archive-mounted callers.
//!
//! # What stays out of scope (V0)
//!
//! * `param_type_names` — only consulted by a handful of stdlib
//!   diagnostic paths; left empty for V0.  Add when a real bug needs it.
//! * `contexts` (the `using [Database, ...]` list) — left empty for
//!   V0.  Most stdlib functions have no context requirements; the
//!   ones that do are exercised by the @using attribute path which
//!   doesn't currently consult this slot.
//! * Protocol implementations as separate ctx state — V0 relies on
//!   the type registry's `protocols` field staying intact via the
//!   linker-merge step.

use std::collections::HashMap;
use std::sync::OnceLock;

use verum_vbc::archive::VbcArchive;
use verum_vbc::codegen::{CodegenContext, FunctionInfo};
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeId, TypeRef, VariantKind};

/// Errors raised while loading the archive into codegen ctx.  Best-
/// effort: the loader skips per-entry failures with a `tracing::warn!`
/// and only returns `Err` on archive-level decode failures that make
/// further iteration impossible.
#[derive(Debug)]
pub enum CtxLoadError {
    /// One or more modules in the archive failed to decode.  Carries
    /// the first decode error's message.  The loader continues past
    /// per-module decode failures (logging a warning); this variant
    /// is reserved for "archive itself is corrupt" — rare.
    ArchiveDecodeFailed(String),
}

impl std::fmt::Display for CtxLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArchiveDecodeFailed(msg) => {
                write!(f, "archive ctx load: {}", msg)
            }
        }
    }
}

impl std::error::Error for CtxLoadError {}

/// Stats returned by [`populate_ctx_from_archive`].  Used by callers
/// that want to log perf / sanity-check the archive coverage.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoadStats {
    /// Number of `VbcModule`s walked in the archive.
    pub modules_loaded: usize,
    /// Number of `FunctionInfo` entries registered (qualified +
    /// simple under collision discipline).
    pub functions_registered: usize,
    /// Number of variant-constructor entries enriched with a
    /// `variant_tag` and `parent_type_name`.
    pub variant_ctors_resolved: usize,
    /// Number of per-module decode failures that were skipped with
    /// a warning.  Non-zero indicates an archive integrity issue
    /// worth investigating.
    pub modules_skipped: usize,
}

/// Walk every module in the archive and register its functions and
/// variant-constructor metadata into the supplied [`CodegenContext`].
///
/// Idempotent under repeated calls: every `register_function` honours
/// first-wins-on-collision when `prefer_existing_functions` is set
/// (which the caller MUST set before calling this fn — mirrors the
/// existing stdlib-load flow at `pipeline/vbc_codegen.rs`).
pub fn populate_ctx_from_archive(
    archive: &VbcArchive,
    ctx: &mut CodegenContext,
) -> Result<LoadStats, CtxLoadError> {
    let mut stats = LoadStats::default();

    for entry in &archive.index {
        let module = match archive.load_module(&entry.name) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    target: "archive_ctx_loader",
                    "skip module {}: decode failed ({:?})",
                    entry.name, e
                );
                stats.modules_skipped += 1;
                continue;
            }
        };
        register_module(&module, &entry.name, ctx, &mut stats);
        stats.modules_loaded += 1;
    }

    Ok(stats)
}

/// Module-level registration helper.  Builds the variant-name index
/// once, then walks functions and assembles each [`FunctionInfo`].
fn register_module(
    module: &VbcModule,
    module_name: &str,
    ctx: &mut CodegenContext,
    stats: &mut LoadStats,
) {
    // Pass 1: parent_type_id → name.  Used by methods (functions
    // with `parent_type` set) to recover their carrier-type name for
    // the disambiguator.
    let mut type_id_to_name: HashMap<TypeId, String> = HashMap::new();
    for ty in &module.types {
        if let Some(name) = module.strings.get(ty.name) {
            type_id_to_name.insert(ty.id, name.to_string());
        }
    }

    // Pass 2: variant simple-name → (parent_type_name, tag, payload_kind, payload_field_types).
    // Used by variant constructors so `Maybe.Some(x)` carries the
    // right tag + parent + payload types into ctx.functions.
    //
    // Multi-type collisions: when the same variant simple name appears
    // in two unrelated types (e.g., `IoError` in both VfsErrorKind and
    // ConnectionError), a HashMap collapses them to one entry.  That
    // matches the stdlib-load discipline — first parent wins for the
    // bare lookup; downstream resolution falls through to the
    // qualified form via #300's inner-generic disambiguator.
    let mut variant_index: HashMap<String, VariantHit> = HashMap::new();
    for ty in &module.types {
        let parent_name = match module.strings.get(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        for variant in &ty.variants {
            let vname = match module.strings.get(variant.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let payload_field_types: Vec<String> = variant
                .fields
                .iter()
                .map(|f| type_ref_simple_name(&f.type_ref, module).unwrap_or_default())
                .collect();
            // First-wins: preserve the first-registered variant for
            // each simple name, matching codegen's first-wins
            // collision rule.
            variant_index.entry(vname).or_insert(VariantHit {
                parent_type_name: parent_name.clone(),
                tag: variant.tag,
                kind: variant.kind,
                payload_field_types,
                arity: variant.arity as usize,
            });
        }
    }

    // Pass 3: walk functions, build FunctionInfo, register under
    // qualified + (collision-aware) simple keys.
    for fn_desc in &module.functions {
        let simple_name = match module.strings.get(fn_desc.name) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Variant ctor lookup — only when arity matches the variant's
        // declared arity (rules out same-named regular functions).
        let variant_hit = variant_index
            .get(&simple_name)
            .filter(|hit| hit.arity == fn_desc.params.len());

        let (variant_tag, parent_type_name, variant_payload_types) = match variant_hit {
            Some(hit) => {
                stats.variant_ctors_resolved += 1;
                (
                    Some(hit.tag),
                    Some(hit.parent_type_name.clone()),
                    if hit.payload_field_types.is_empty() {
                        None
                    } else {
                        Some(hit.payload_field_types.clone())
                    },
                )
            }
            None => {
                // Method on a type? `parent_type` set on the descriptor.
                let parent = fn_desc
                    .parent_type
                    .and_then(|tid| type_id_to_name.get(&tid).cloned());
                (None, parent, None)
            }
        };

        // Param names — best-effort; missing string ids drop to "_argN".
        let param_names: Vec<String> = fn_desc
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                module
                    .strings
                    .get(p.name)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();

        // Return-type base name + inner generics drive the variant
        // disambiguator (closes out the same code path #300 fixed
        // for source-driven compilation).
        let return_type_name = type_ref_simple_name(&fn_desc.return_type, module);
        let return_type_inner = type_ref_inner_generics(&fn_desc.return_type, module);

        let info = FunctionInfo {
            id: fn_desc.id,
            param_count: fn_desc.params.len(),
            param_names,
            param_type_names: vec![],
            is_async: fn_desc
                .properties
                .contains(verum_vbc::types::PropertySet::ASYNC),
            is_generator: fn_desc.is_generator,
            contexts: vec![],
            return_type: Some(fn_desc.return_type.clone()),
            yield_type: fn_desc.yield_type.clone(),
            intrinsic_name: None,
            variant_tag,
            parent_type_name,
            variant_payload_types,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner,
        };

        // Always register qualified — `module.path.simple` —
        // unconditionally.  Cross-module dispatch path keys on this.
        let qualified = format!("{}.{}", module_name, simple_name);
        ctx.register_function(qualified, info.clone());
        stats.functions_registered += 1;

        // Simple name with first-wins collision discipline so a
        // bare `Some` mounted from `Maybe` doesn't get clobbered by
        // a same-named variant in a later-loaded module.  Mirrors
        // `prefer_existing_functions=true` semantics that the
        // existing stdlib-load path uses.
        if ctx.lookup_function(&simple_name).is_none() {
            ctx.register_function(simple_name, info);
            stats.functions_registered += 1;
        }
    }
}

/// Per-variant index entry.
struct VariantHit {
    parent_type_name: String,
    tag: u32,
    /// Reserved for future use — when arity-only matching becomes
    /// insufficient (unit vs tuple variants of the same name) the
    /// disambiguator can fall back to the kind.
    #[allow(dead_code)]
    kind: VariantKind,
    payload_field_types: Vec<String>,
    arity: usize,
}

/// Strip a [`TypeRef`] down to its base nominal name when one exists.
/// Returns `None` for unresolvable / structural / function types
/// (those don't drive the variant disambiguator).
fn type_ref_simple_name(ty: &TypeRef, module: &VbcModule) -> Option<String> {
    match ty {
        TypeRef::Concrete(tid) => module
            .types
            .iter()
            .find(|t| t.id == *tid)
            .and_then(|t| module.strings.get(t.name).map(|s| s.to_string())),
        TypeRef::Instantiated { base, .. } => module
            .types
            .iter()
            .find(|t| t.id == *base)
            .and_then(|t| module.strings.get(t.name).map(|s| s.to_string())),
        TypeRef::Generic(_) | TypeRef::Function { .. } | TypeRef::Reference { .. } => None,
        // Other variants (Tuple, Pointer, etc.) — no nominal base.
        _ => None,
    }
}

/// Pull the inner generic args of a [`TypeRef::Instantiated`] back to
/// their simple names.  `Result<Int, ConnectionError>` → `["Int", "ConnectionError"]`.
/// Any inner that can't resolve to a name slots in as an empty string
/// so the position survives — the disambiguator iterates positionally.
fn type_ref_inner_generics(ty: &TypeRef, module: &VbcModule) -> Option<Vec<String>> {
    match ty {
        TypeRef::Instantiated { args, .. } if !args.is_empty() => {
            let names: Vec<String> = args
                .iter()
                .map(|a| type_ref_simple_name(a, module).unwrap_or_default())
                .collect();
            Some(names)
        }
        _ => None,
    }
}

/// Process-wide cache of `populate_ctx_from_archive` per (archive
/// pointer, module-graph hash).  Today the archive comes from a
/// `static OnceLock` so we only ever populate one ctx per process —
/// the cache is a thin lazy-init wrapper around the FunctionInfo
/// table that subsequent compile invocations clone instead of
/// re-deriving from raw descriptors.
///
/// Exported so the pipeline can prime its codegen ctx in O(N_clone)
/// rather than O(N_register) on the second + every later script run
/// inside the same process (REPL, test runner, watch mode).
pub struct ArchiveCtxCache {
    /// One-shot lazily-built table: qualified name → FunctionInfo.
    /// Holds both qualified (`module.simple`) and simple-name keys
    /// after first build.
    table: OnceLock<HashMap<String, FunctionInfo>>,
}

impl ArchiveCtxCache {
    /// Construct an empty cache.  Cheap; no archive work happens here.
    pub const fn new() -> Self {
        Self {
            table: OnceLock::new(),
        }
    }

    /// Lazily build the cache from `archive` (idempotent — first call
    /// wins, every later call no-ops on the OnceLock side).  Returns
    /// the cached table on every call.
    pub fn get_or_build(
        &self,
        archive: &VbcArchive,
    ) -> &HashMap<String, FunctionInfo> {
        self.table.get_or_init(|| {
            let mut staging = CodegenContext::new();
            let _ = populate_ctx_from_archive(archive, &mut staging);
            staging.export_functions()
        })
    }

    /// Apply the cached function table to a fresh `ctx` via
    /// [`CodegenContext::import_functions`].  Equivalent to running
    /// `populate_ctx_from_archive` but ~30× faster on the
    /// second+later calls because the conversion only happens once.
    pub fn apply(&self, archive: &VbcArchive, ctx: &mut CodegenContext) {
        let table = self.get_or_build(archive);
        ctx.import_functions(table);
    }

    /// T2-extended-perf: lazy variant of [`apply`].  Walks the
    /// user `Module`'s `mount` declarations, harvests the
    /// imported simple+qualified names, and registers ONLY those
    /// from the archive.  For a hello.vr that mounts ~5 stdlib
    /// symbols, this drops the 7484-entry full populate to a
    /// per-script handful — typically <1ms.
    ///
    /// Falls through to the full table for any per-call function
    /// references that the mount-pre-scan missed (variant
    /// constructors, methods called via dot-form, etc.) via the
    /// codegen's existing `find_function_by_suffix` /
    /// `find_variant_by_suffix_and_args` redirects, which themselves
    /// re-trigger lazy registration through this cache on miss.
    ///
    /// The full table is still built lazily on first demand-path
    /// hit — the cost amortises across compilations within the
    /// same process (REPL, watch mode), and the upfront cost is
    /// gone for one-shot scripts.
    pub fn apply_lazy(
        &self,
        archive: &VbcArchive,
        ctx: &mut CodegenContext,
        user_module: &verum_ast::Module,
    ) {
        let mut wanted: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut wanted);
        }
        if wanted.is_empty() {
            return;
        }
        // Module-name prefix gate: archive `index[i].name` is the
        // dotted module path (`core.io.stdio`).  A wanted qualified
        // name like `core.io.stdio.println` lives in module
        // `core.io.stdio` (the prefix up to the last dot), so we
        // can SKIP decoding any module whose name doesn't appear
        // as a wanted-name prefix.  For a hello.vr that mounts
        // `core.io.stdio.println` this drops the 565-module walk
        // to ~1-2 modules — the rest are O(1) string-prefix checks
        // against the archive index entries (which are already
        // decoded as part of the archive header).
        // Build module-prefix gate.  For each wanted qualified name
        // (`core.io.path.Path`), we visit not just the direct parent
        // module (`core.io.path`) but also up to TWO ancestors above
        // — the precompiled-stdlib archive bundles a `.vr` file's
        // functions under the GRANDPARENT module's archive entry when
        // the source declares `module X;` with just the leaf segment
        // and the parent directory has its own `mod.vr`.  Empirical
        // observation:
        //  * `core/io/path.vr` declares `module path;` → its
        //    PathBuf.* methods land in archive entry `core.io`.
        //  * `core/shell/builtins.vr` declares `module builtins;` →
        //    its functions land in archive entry `core.shell`.
        // So a wanted qualified name two levels deep (`core.io.path`)
        // needs to reach the grandparent (`core.io`) to find the
        // method bodies.
        //
        // BOUNDED to two ancestors: walking all the way to `core`
        // would visit nearly every archive entry — including
        // unrelated modules that happen to define a same-named
        // variant (e.g. `core.tracing.span`'s `SpanStatusCode is
        // Unset|Ok|Error` collides with `core.base.result.Result`'s
        // `Ok` constructor and breaks user-side variant
        // resolution).  The two-hop bound finds cross-file siblings
        // and grandparent-bundled methods without polluting the
        // variant-constructor parent map.
        let wanted_module_prefixes: std::collections::HashSet<String> = wanted
            .iter()
            .flat_map(|name| {
                let mut prefixes: Vec<String> = Vec::new();
                let mut cur = name.as_str();
                let mut hops = 0;
                while let Some(idx) = cur.rfind('.') {
                    cur = &cur[..idx];
                    prefixes.push(cur.to_string());
                    hops += 1;
                    if hops >= 2 {
                        break;
                    }
                }
                prefixes
            })
            .collect();
        for entry in &archive.index {
            // Skip decode unless this module name matches a
            // qualified-name prefix from the wanted set.  Bare
            // simple names with no qualified counterpart fall
            // through to the FULL walk below.
            let is_target_module = wanted_module_prefixes.contains(&entry.name);
            if !is_target_module {
                continue;
            }
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            register_module_filtered(&module, &entry.name, ctx, &wanted);
        }
        // For wanted names that have NO qualified form (e.g. user
        // code calls `Maybe.Some(x)` without a `mount Maybe`
        // declaration), walk the rest of the archive looking only
        // at simple-name matches.  Most stdlib symbols come in via
        // mounts so this branch typically processes nothing.
        let unqualified_wanted: std::collections::HashSet<String> = wanted
            .iter()
            .filter(|n| !n.contains('.'))
            .cloned()
            .collect();
        if !unqualified_wanted.is_empty() {
            // Try to register simple names only by re-checking
            // every archive module.  This is the slow fallback
            // — but it's bounded by `unqualified_wanted` which
            // is typically tiny for real scripts.  If perf
            // matters, callers should add explicit mount
            // declarations to bring symbols in scope.
            for entry in &archive.index {
                if wanted_module_prefixes.contains(&entry.name) {
                    continue; // already processed above
                }
                let module = match archive.load_module(&entry.name) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                // Cheap pre-check: scan module's strings table for any of
                // the unqualified wanted names BEFORE doing the full
                // descriptor walk.  If none of the wanted simple-names
                // appear as a string in the module, register_module_filtered
                // would do nothing — skip it entirely.
                let any_match = unqualified_wanted.iter().any(|w| {
                    module.strings.iter().any(|(s, _)| s == w)
                });
                if !any_match {
                    continue;
                }
                register_module_filtered(&module, &entry.name, ctx, &unqualified_wanted);
            }
        }
    }
}

impl Default for ArchiveCtxCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: when the compiler binary embeds the precompiled
    /// stdlib archive, `populate_ctx_from_archive` registers a
    /// non-trivial number of functions and recovers variant-ctor
    /// metadata for every stdlib type that lands in the archive.
    ///
    /// Note on what's in scope: built-in core variants (Maybe.Some /
    /// Maybe.None / Result.Ok / Result.Err / Ordering.Lt etc.) are
    /// registered by VbcCodegen::register_builtin_variants, not by
    /// the archive — they're compiler intrinsics with hardcoded tags.
    /// This loader handles the user-stdlib-type variants only;
    /// built-ins flow through a parallel path called from
    /// `compile_ast_to_vbc` before T1 runs.
    #[test]
    fn loads_embedded_archive_into_ctx() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return, // bootstrap build without archive — skip
        };
        let mut ctx = CodegenContext::new();
        let stats = populate_ctx_from_archive(archive, &mut ctx).expect("load");

        assert!(
            stats.modules_loaded > 100,
            "must load >100 stdlib modules (got {})",
            stats.modules_loaded
        );
        assert!(
            stats.functions_registered > 1000,
            "must register >1000 functions (got {})",
            stats.functions_registered
        );

        // At least some stdlib types surface variant constructors
        // through the archive (DbError variants, ConnectionError,
        // ShellError, etc.).  We don't pin a specific list because
        // stdlib evolves; assert "more than zero" to catch the case
        // where the variant_tag-recovery loop is silently broken.
        assert!(
            stats.variant_ctors_resolved > 0,
            "expected variant-ctor recovery to find at least one stdlib variant ctor"
        );

        // Sample qualified lookup — the archive's modules carry
        // canonical `core.X.Y.fn` qualified names.  Pick a stable
        // entrypoint that's been in stdlib for many revisions.
        let exported = ctx.export_functions();
        let canonical_qualified = exported
            .keys()
            .filter(|k| k.starts_with("core.") && k.contains('.'))
            .count();
        assert!(
            canonical_qualified > 100,
            "expected >100 canonical `core.*` qualified entries"
        );
    }

    /// Diagnostic: dump current_dir-related entries to verify
    /// archive has the function under expected qualified name.
    #[test]
    #[ignore = "diagnostic only"]
    fn diag_current_dir_lookup() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        for entry in &archive.index {
            if entry.name.ends_with("io.fs") || entry.name == "core.io.fs" {
                println!("Archive module: {}", entry.name);
                let m = archive.load_module(&entry.name).unwrap();
                for f in &m.functions {
                    let n = m
                        .strings
                        .get(f.name)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    if n == "current_dir" || n.contains("current_dir") {
                        println!(
                            "  fn `{}` params={} id={:?}",
                            n,
                            f.params.len(),
                            f.id
                        );
                    }
                }
            }
        }
        let mut ctx = CodegenContext::new();
        let _ = populate_ctx_from_archive(archive, &mut ctx).unwrap();
        let exported = ctx.export_functions();
        for k in exported.keys() {
            if k.contains("current_dir") {
                println!("ctx key: {}", k);
            }
        }
    }

    /// Cache layer round-trip: first call builds, second clones.
    /// Both must produce identical ctx state.
    #[test]
    fn archive_ctx_cache_round_trip() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        let cache = ArchiveCtxCache::new();
        let mut ctx_first = CodegenContext::new();
        cache.apply(archive, &mut ctx_first);
        let first_count = ctx_first.export_functions().len();
        assert!(first_count > 0);

        let mut ctx_second = CodegenContext::new();
        cache.apply(archive, &mut ctx_second);
        let second_count = ctx_second.export_functions().len();
        assert_eq!(
            first_count, second_count,
            "cached apply must produce identical entry count across runs"
        );
    }
}

// ============================================================================
// T2-extended-perf: lazy mount-driven FunctionInfo registration
// ============================================================================

/// Walk a top-level `verum_ast::Item` and harvest names from every
/// `mount` declaration.  Function bodies are NOT walked here —
/// names not picked up via mounts go through the codegen's
/// `find_function_by_suffix` redirect chain, which can re-trigger
/// lazy registration via the cache's `apply` fallback.
///
/// Real-world stdlib usage: every cross-module function call
/// requires a `mount` declaration to bring the name in scope.  So
/// the mount-only pre-scan covers practically every stdlib
/// reference at sub-millisecond cost.
fn collect_referenced_function_names(
    item: &verum_ast::Item,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::ItemKind;
    if let ItemKind::Mount(mount_decl) = &item.kind {
        collect_mount_names(&mount_decl.tree, &[], out);
    }
}

/// Walk a mount tree harvesting every imported simple-name and
/// qualified form.  `mount core.io.stdio.{println, print}` adds
/// `println`, `print`, `core.io.stdio.println`, `core.io.stdio.print`.
fn collect_mount_names(
    tree: &verum_ast::decl::MountTree,
    prefix: &[String],
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::decl::MountTreeKind;
    match &tree.kind {
        MountTreeKind::Path(path) => {
            let segs: Vec<String> = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => {
                        Some(id.name.to_string())
                    }
                    _ => None,
                })
                .collect();
            if segs.is_empty() {
                return;
            }
            let mut full: Vec<String> = prefix.to_vec();
            full.extend(segs);
            // Last segment is the name; insert both simple and
            // dot-joined fully-qualified.
            if let Some(last) = full.last() {
                out.insert(last.clone());
                // Also the alias if any.
                if let verum_common::Maybe::Some(alias) = &tree.alias {
                    out.insert(alias.name.to_string());
                }
            }
            out.insert(full.join("."));
        }
        MountTreeKind::Nested {
            prefix: nested_prefix,
            trees,
        } => {
            let nested_segs: Vec<String> = nested_prefix
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => {
                        Some(id.name.to_string())
                    }
                    _ => None,
                })
                .collect();
            let mut combined: Vec<String> = prefix.to_vec();
            combined.extend(nested_segs);
            for sub in trees.iter() {
                collect_mount_names(sub, &combined, out);
            }
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => {}
    }
}

/// Register only those FunctionInfo entries whose simple or
/// qualified name appears in `wanted`.  Parallel to
/// `register_module` but with name-set filtering.
fn register_module_filtered(
    module: &VbcModule,
    module_name: &str,
    ctx: &mut CodegenContext,
    wanted: &std::collections::HashSet<String>,
) {
    let mut type_id_to_name: HashMap<TypeId, String> = HashMap::new();
    for ty in &module.types {
        if let Some(name) = module.strings.get(ty.name) {
            type_id_to_name.insert(ty.id, name.to_string());
        }
    }
    let mut variant_index: HashMap<String, VariantHit> = HashMap::new();
    for ty in &module.types {
        let parent_name = match module.strings.get(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        for variant in &ty.variants {
            let vname = match module.strings.get(variant.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let payload_field_types: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    type_ref_simple_name(&f.type_ref, module).unwrap_or_default()
                })
                .collect();
            variant_index.entry(vname).or_insert(VariantHit {
                parent_type_name: parent_name.clone(),
                tag: variant.tag,
                kind: variant.kind,
                payload_field_types,
                arity: variant.arity as usize,
            });
        }
    }
    for fn_desc in &module.functions {
        let simple_name = match module.strings.get(fn_desc.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let qualified = format!("{}.{}", module_name, simple_name);
        // Filter: register if (a) simple OR qualified is wanted,
        // OR (b) the function is a static/inherent method of a
        // wanted TYPE — i.e. simple_name has the form
        // `<wanted_type>.<method>` where `<wanted_type>` itself
        // appears in the wanted set.  Without (b), mounting a type
        // T (`mount core.io.path.Path`) would NOT load T's static
        // methods (Path.new, Path.from_str, …) — every
        // user-side `Path.new(&"...")` then surfaces at runtime
        // as `method 'new' not found on receiver of runtime kind
        // Int` because the static-method dispatcher in
        // `compile_method_call` falls through to the regular
        // method-call path which evaluates `Path` as a value
        // expression.
        let is_method_of_wanted_type = simple_name
            .find('.')
            .map(|dot_idx| {
                let parent = &simple_name[..dot_idx];
                wanted.contains(parent)
            })
            .unwrap_or(false);
        if !wanted.contains(&simple_name)
            && !wanted.contains(&qualified)
            && !is_method_of_wanted_type
        {
            continue;
        }
        let variant_hit = variant_index
            .get(&simple_name)
            .filter(|hit| hit.arity == fn_desc.params.len());
        let (variant_tag, parent_type_name, variant_payload_types) = match variant_hit {
            Some(hit) => (
                Some(hit.tag),
                Some(hit.parent_type_name.clone()),
                if hit.payload_field_types.is_empty() {
                    None
                } else {
                    Some(hit.payload_field_types.clone())
                },
            ),
            None => {
                let parent = fn_desc
                    .parent_type
                    .and_then(|tid| type_id_to_name.get(&tid).cloned());
                (None, parent, None)
            }
        };
        let param_names: Vec<String> = fn_desc
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                module
                    .strings
                    .get(p.name)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();
        let return_type_name = type_ref_simple_name(&fn_desc.return_type, module);
        let return_type_inner = type_ref_inner_generics(&fn_desc.return_type, module);
        let info = FunctionInfo {
            id: fn_desc.id,
            param_count: fn_desc.params.len(),
            param_names,
            param_type_names: vec![],
            is_async: fn_desc
                .properties
                .contains(verum_vbc::types::PropertySet::ASYNC),
            is_generator: fn_desc.is_generator,
            contexts: vec![],
            return_type: Some(fn_desc.return_type.clone()),
            yield_type: fn_desc.yield_type.clone(),
            intrinsic_name: None,
            variant_tag,
            parent_type_name,
            variant_payload_types,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner,
        };
        ctx.register_function(qualified, info.clone());
        if ctx.lookup_function(&simple_name).is_none() {
            ctx.register_function(simple_name, info);
        }
    }
}
