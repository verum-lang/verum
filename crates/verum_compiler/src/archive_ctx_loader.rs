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
    next_id: &mut u32,
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
        register_module(&module, &entry.name, ctx, &mut stats, next_id);
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
    next_id: &mut u32,
) {
    // **Cold-start optimisation**: O(1) StringId → &str reverse index.
    // See `register_module_filtered` for the full rationale; both
    // paths share the same per-module string-table walk discipline.
    let name_by_id: HashMap<verum_vbc::types::StringId, &str> = module
        .strings
        .iter()
        .map(|(s, id)| (id, s))
        .collect();
    let lookup = |id: verum_vbc::types::StringId| -> Option<&str> {
        name_by_id.get(&id).copied()
    };
    // Pass 1: parent_type_id → name.  Used by methods (functions
    // with `parent_type` set) to recover their carrier-type name for
    // the disambiguator.
    let mut type_id_to_name: HashMap<TypeId, String> = HashMap::new();
    for ty in &module.types {
        if let Some(name) = lookup(ty.name) {
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
        let parent_name = match lookup(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        for variant in &ty.variants {
            let vname = match lookup(variant.name) {
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
        let simple_name = match lookup(fn_desc.name) {
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
        // Param TYPE names — required for type-aware bare-name
        // disambiguation in the call-site resolver; without this
        // the resolver can't tell which sibling stdlib function
        // (sharing a simple name across multiple modules) the call's
        // inferred argument types match. See the matching change in
        // `register_module_filtered` and the type-aware lookup in
        // `compile_call`.
        let param_type_names: Vec<String> = fn_desc
            .params
            .iter()
            .map(|p| type_ref_simple_name(&p.type_ref, module).unwrap_or_default())
            .collect();

        // Return-type base name + inner generics drive the variant
        // disambiguator (closes out the same code path #300 fixed
        // for source-driven compilation).
        let return_type_name = type_ref_simple_name(&fn_desc.return_type, module);
        let return_type_inner = type_ref_inner_generics(&fn_desc.return_type, module);

        // Remap each archive function to a globally-unique id slot.
        // See `register_module_filtered` for the rationale.
        let new_id = verum_vbc::module::FunctionId(*next_id);
        *next_id = next_id.saturating_add(1);

        // #87 — restore the intrinsic-name marker that was
        // serialised on the archive side.  `__const_val_<N>` and
        // similar markers identify inlinable stdlib constants;
        // without them the codegen's path-resolution treats
        // imported constants as ordinary zero-arg functions and
        // surfaces them as `UndefinedVariable` at the use site.
        let intrinsic_name = fn_desc
            .intrinsic_name
            .and_then(|sid| lookup(sid).map(|s| s.to_string()));
        let info = FunctionInfo {
            id: new_id,
            param_count: fn_desc.params.len(),
            param_names,
            param_type_names,
            is_async: fn_desc
                .properties
                .contains(verum_vbc::types::PropertySet::ASYNC),
            is_generator: fn_desc.is_generator,
            contexts: vec![],
            return_type: Some(fn_desc.return_type.clone()),
            yield_type: fn_desc.yield_type.clone(),
            intrinsic_name,
            variant_tag,
            parent_type_name,
            variant_payload_types,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner,
            // #97 — restore the const-storage marker so user-side
            // codegen treats stdlib `public const X` as a value
            // rather than a callable.
            is_const: fn_desc.is_const,
            // Archive-loaded functions are NEVER transparent
            // wrappers — only the synthetic newtype/single-tuple/
            // quotient constructors get this flag, and those are
            // re-registered by the in-process type-decl arms when
            // the type itself is mounted.  See `is_transparent_wrapper`
            // in `verum_vbc/src/codegen/context.rs`.
            is_transparent_wrapper: false,
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

    // Pass 4 — variant constructor registration from
    // `module.types[*].variants`.  Architectural background in the
    // matching block at the bottom of `register_module_filtered`.
    use verum_vbc::module::FunctionId;
    for ty in &module.types {
        let parent_name = match lookup(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        for variant in &ty.variants {
            let vname = match lookup(variant.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let qualified = format!("{}.{}", parent_name, vname);
            if ctx.lookup_function(&qualified).is_some() {
                continue;
            }
            let (arity, payload_field_types) = match variant.kind {
                VariantKind::Unit => (0usize, Vec::<String>::new()),
                VariantKind::Tuple => (
                    variant.arity as usize,
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_simple_name(&f.type_ref, module).unwrap_or_default()
                        })
                        .collect(),
                ),
                VariantKind::Record => (
                    variant.fields.len(),
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_simple_name(&f.type_ref, module).unwrap_or_default()
                        })
                        .collect(),
                ),
            };
            let param_names: Vec<String> = (0..arity).map(|i| format!("_{}", i)).collect();
            let info = FunctionInfo {
                id: FunctionId(u32::MAX - variant.tag),
                param_count: arity,
                param_names,
                // Variant constructor params take payload field types so
                // type-aware bare-name disambiguation works for variant
                // ctor calls too.
                param_type_names: payload_field_types.clone(),
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: None,
                variant_tag: Some(variant.tag),
                parent_type_name: Some(parent_name.clone()),
                variant_payload_types: if payload_field_types.is_empty() {
                    None
                } else {
                    Some(payload_field_types)
                },
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: Some(parent_name.clone()),
                return_type_inner: None,
                is_const: false,
            is_transparent_wrapper: false,
            };
            ctx.register_function(qualified, info);
            stats.variant_ctors_resolved += 1;
            // Deliberately do NOT register simple-name here.  Pass 4
            // synthesises variant constructors for stdlib sum types
            // BEFORE user-side `register_type_constructors` runs;
            // adding `Help` (e.g. from
            // `core.meta.contexts.DiagnosticSeverity.Help`) under the
            // bare key would then collide with a user-defined
            // `type ParsedArgs is | Help | ...`, the user-mode
            // collision rule unregisters the simple name and inserts
            // it into `variant_collisions`, and codegen for the
            // user's bare `Help` falls through to ambiguous
            // suffix-disambiguation.  Qualified `ParentType.Variant`
            // is sufficient for `compile_record`'s descriptor-table
            // fallback and `find_variant_by_suffix_and_args` to
            // resolve the user's local sum type unambiguously.
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
        TypeRef::Concrete(tid) => {
            // Primitive types are NOT in `module.types` (which only carries
            // user-defined records / sum types). Their TypeIds are reserved
            // in `verum_vbc::types::TypeId` constants and the canonical Verum
            // name is fixed — look it up by id first, then fall through to
            // the user-type scan.
            if let Some(name) = primitive_typeid_name(*tid) {
                return Some(name.to_string());
            }
            module
                .types
                .iter()
                .find(|t| t.id == *tid)
                .and_then(|t| module.strings.get(t.name).map(|s| s.to_string()))
        }
        TypeRef::Instantiated { base, .. } => {
            if let Some(name) = primitive_typeid_name(*base) {
                return Some(name.to_string());
            }
            module
                .types
                .iter()
                .find(|t| t.id == *base)
                .and_then(|t| module.strings.get(t.name).map(|s| s.to_string()))
        }
        // Reference TypeRef carries an `inner` type — recover the inner's
        // simple name so `&Bucket` reads as `Bucket` for the disambiguator
        // (matches the codegen-side `extract_type_name_from_ast` shape).
        TypeRef::Reference { inner, .. } => type_ref_simple_name(inner, module),
        TypeRef::Generic(_) | TypeRef::Function { .. } => None,
        // Other variants (Tuple, Pointer, etc.) — no nominal base.
        _ => None,
    }
}

/// Resolve well-known primitive TypeIds to their canonical Verum
/// type name. Returns None for user TypeIds (>= FIRST_USER) or
/// unrecognised reserved slots.
///
/// Source of truth: `verum_vbc::types::TypeId` constants. Aliases
/// that share a numeric id (`PTR = USIZE = ISIZE = TypeId(14)`,
/// `I64 = INT = TypeId(2)`, `BYTE = U8 = TypeId(6)`, `F64 = FLOAT
/// = TypeId(3)`) deliberately resolve to ONE canonical name — the
/// type-aware disambiguator at the call site uses the same
/// canonical name when extracting the cast target, so the equality
/// check holds.
fn primitive_typeid_name(tid: TypeId) -> Option<&'static str> {
    Some(match tid {
        TypeId::UNIT => "()",
        TypeId::BOOL => "Bool",
        TypeId::INT => "Int",
        TypeId::FLOAT => "Float",
        TypeId::TEXT => "Text",
        TypeId::NEVER => "Never",
        TypeId::U8 => "UInt8",
        TypeId::U16 => "UInt16",
        TypeId::U32 => "UInt32",
        TypeId::U64 => "UInt64",
        TypeId::I8 => "Int8",
        TypeId::I16 => "Int16",
        TypeId::I32 => "Int32",
        TypeId::F32 => "Float32",
        TypeId::PTR => "USize",
        TypeId::CHAR => "Char",
        _ => return None,
    })
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

/// Build the `wanted_module_prefixes` set used by every archive-walk
/// path in this module.  Two contributions:
///
/// 1. **Up-to-2-hop ancestor walk** of every dotted name in `wanted`:
///    `core.io.path.read` → `core.io.path` + `core.io`.  Bounded to
///    two hops because walking all the way to `core` would visit
///    nearly every archive entry — including unrelated modules that
///    happen to define a same-named variant (e.g. `core.tracing.span`'s
///    `Ok` collision with `core.base.result.Result.Ok`).
///
/// 2. **Well-known stdlib type expansion** via
///    `WellKnownType::canonical_archive_modules`.  When user code
///    mentions a stdlib well-known type by simple name (e.g. `Text`,
///    `List`, `Map`, `Channel`), step 1 produces nothing — the archive
///    has no entry literally named `Text`; the carrier module is
///    `core.text.text` (or grandparent-bundled `core.text`).  Without
///    this expansion, `Text.new()` / `List.with_capacity(8)` / etc.
///    fail with `UndefinedFunction` because the archive module never
///    decodes.  The mapping is centralised in `verum_common`'s
///    `WellKnownType::canonical_archive_modules` and pin-tested
///    against `core/`'s `module <path>;` declarations, so adding a
///    new well-known type or relocating an existing one updates this
///    loader automatically.
fn build_wanted_module_prefixes(
    wanted: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let mut prefixes: std::collections::HashSet<String> = wanted
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
    for name in wanted {
        if let Some(wkt) =
            verum_common::well_known_types::WellKnownType::from_name(name)
        {
            for module_path in wkt.canonical_archive_modules() {
                prefixes.insert((*module_path).to_string());
            }
        }
    }
    prefixes
}

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
            // Local id allocator for the staging path.  This call site
            // exports a frozen FunctionInfo table for re-use across
            // compiles; callers of the table (`get_or_build`'s consumers)
            // own their own next_func_id so the IDs allocated here are
            // best-effort placeholders that downstream `apply_lazy`
            // re-allocates against the live codegen counter.
            let mut next_id: u32 = 0;
            let _ = populate_ctx_from_archive(archive, &mut staging, &mut next_id);
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
        next_id: &mut u32,
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
        // BOUNDED to two ancestors and extended with well-known
        // stdlib type module paths — see [`build_wanted_module_prefixes`]
        // for the rationale.
        let wanted_module_prefixes = build_wanted_module_prefixes(&wanted);
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
            // Legacy `apply_lazy` path — only registers metadata, no
            // body merge (the body-merge surface needs `&mut VbcCodegen`,
            // not just `&mut CodegenContext`). Production callers go
            // through `apply_lazy_with_types` which performs the
            // merge; this path is kept for the transitional
            // metadata-only consumers and discards the remap.
            let _ = register_module_filtered(&module, &entry.name, ctx, &wanted, next_id);
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
                let _ = register_module_filtered(&module, &entry.name, ctx, &unqualified_wanted, next_id);
            }
        }
    }
}

impl Default for ArchiveCtxCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ArchiveCtxCache {
    /// Walks every archive module the user mounts (transitively, via
    /// `harvest_names_in_*`) and pushes each module's TypeDescriptors
    /// into the user codegen via `import_archive_type`.  Pairs with
    /// `apply_lazy`, which handles the function side; this method
    /// closes the type-table side so stdlib sum types can flow through
    /// `MakeVariantTyped` and the runtime's type-scoped variant-name
    /// lookup.
    ///
    /// Bounded the same way as `apply_lazy`: only modules whose names
    /// are prefixes of wanted qualified names get loaded — typical
    /// scripts touch a small fraction of the archive's module set, so
    /// the cost is amortised across compilations.
    ///
    /// Returns the number of modules whose type tables were imported.
    pub fn import_types_for_module(
        archive: &VbcArchive,
        codegen: &mut verum_vbc::codegen::VbcCodegen,
        user_module: &verum_ast::Module,
    ) -> usize {
        let mut wanted: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut wanted);
        }
        if wanted.is_empty() {
            return 0;
        }
        // Up to 2-hop ancestor walk (mirrors apply_lazy) — same
        // grandparent-bundling shape: e.g. `core/io/path.vr` declares
        // `module path;` and lands under archive entry `core.io`.
        // Well-known stdlib types (Text/List/Map/...) get explicit
        // module-path expansion via `build_wanted_module_prefixes`.
        let wanted_module_prefixes = build_wanted_module_prefixes(&wanted);
        let mut imported = 0usize;
        for entry in &archive.index {
            if !wanted_module_prefixes.contains(&entry.name) {
                continue;
            }
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if module.types.is_empty() {
                continue;
            }
            codegen.import_archive_module_types(&module);
            imported += 1;
        }
        imported
    }

    /// Combined function- AND type-table import in a single archive
    /// walk.  Replaces the `apply_lazy` + `import_types_for_module`
    /// pair when the caller has access to both `&mut VbcCodegen` and
    /// the cache — each archive module decodes ONCE instead of twice,
    /// halving the cold-start archive-load cost on cache misses.
    ///
    /// Behaves as the union of the two helpers: lazy filtering on
    /// `wanted_module_prefixes`, function registration with id remap
    /// (Pass 3 + 4 from `register_module_filtered`) AND type-table
    /// import via `import_archive_module_types`.
    pub fn apply_lazy_with_types(
        &self,
        archive: &VbcArchive,
        codegen: &mut verum_vbc::codegen::VbcCodegen,
        user_module: &verum_ast::Module,
    ) -> (usize, usize) {
        let mut wanted: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut wanted);
        }
        if wanted.is_empty() {
            return (0, 0);
        }
        let mut wanted_module_prefixes = build_wanted_module_prefixes(&wanted);

        // **Variant-tag-collision force-load** (load-bearing for
        // bare `Some(x)` / `None` / `Ok(x)` / `Err(e)` syntax — see
        // commit 66ab177f1 for the original fix and the AliasError
        // collision case it was solving).  The unqualified-wanted
        // second pass below filters these names out because
        // `lookup_function(name).is_some()` is true (they're
        // pre-registered by `VbcCodegen::register_builtin_variants`),
        // so without this hook Maybe / Result archive modules never
        // get loaded for code that mentions only the bare ctors.  The
        // runtime then falls through to the global tag-scan and picks
        // whichever unrelated stdlib variant happens to share the
        // synthetic `0x8000+tag` TypeId — `Maybe.None` rendering as
        // `AliasError.EmptyWeights` is the canonical failure mode.
        //
        // Source-of-truth: `verum_common::well_known_types::variant_tags`
        // tracks the recognised ctor names; the canonical archive
        // modules they belong to are the only Verum-wide hardcode and
        // mirror the layout constants `MAYBE_VARIANT_LAYOUT` /
        // `RESULT_VARIANT_LAYOUT`.
        // Expansion is two-staged: (1) add the variant-carrier
        // archive modules to `wanted_module_prefixes` so type imports
        // fire; (2) add the carrier TYPE NAMES (`Maybe`, `Result`) to
        // `wanted` itself so the function-side filter at
        // `register_module_filtered` (`is_method_of_wanted_type`)
        // accepts the impl methods (`Maybe.eq`, `Maybe.cmp`,
        // `Result.eq`, …).  Without (2), user code that uses
        // `Some(5) == Some(5)` finds `Some` registered but the
        // operator-method dispatcher fails to find `Maybe.eq` —
        // codegen demotes to a primitive `CmpI` that compares
        // distinct heap allocations bit-for-bit and returns false.
        let mut to_add: Vec<&'static str> = Vec::new();
        for name in &wanted {
            if verum_common::well_known_types::variant_tags::is_maybe_constructor(name) {
                // Both the canonical archive entry for `Maybe`
                // (`core.base.maybe` when source declares `module
                // maybe;`) AND the grandparent-bundled form
                // (`core.base` when the precompiler bundles
                // `core/base/maybe.vr`'s impl methods under the
                // parent module's archive entry).
                wanted_module_prefixes.insert("core.base.maybe".to_string());
                wanted_module_prefixes.insert("core.base".to_string());
                to_add.push("Maybe");
            }
            if verum_common::well_known_types::variant_tags::is_result_constructor(name) {
                wanted_module_prefixes.insert("core.base.result".to_string());
                wanted_module_prefixes.insert("core.base".to_string());
                to_add.push("Result");
            }
        }
        // **Transitive Maybe/Result for higher-level stdlib types.**
        //
        // Stdlib types like `OnceCell<T>` / `LazyCell<T>` /
        // `RefCell<T>` carry `Maybe<T>` / `Result<T,E>` payloads and
        // their methods (`is_initialized`, `borrow`, `borrow_mut`,
        // `get_or_init`, …) call `Maybe.is_some` / `Maybe.is_none` /
        // `Result.unwrap` from their bytecode bodies.  When user code
        // only mounts `OnceCell`, the wanted-prefix walker above sees
        // `OnceCell` but not `Maybe` / `Result`, so the
        // `core.base.maybe` archive entry never decodes — runtime
        // panics with `method 'Maybe.is_some' not found on receiver
        // of runtime kind Object`.
        //
        // Surgical fix: detect names of stdlib carriers known to
        // transitively need Maybe/Result, and force-load both.  The
        // hardcoded set lives here (the single force-load
        // architectural seam already documented above for
        // variant-tag-collision); each entry is justified by an
        // observed test failure where the type's body references
        // Maybe/Result methods and the wanted-prefix walker can't
        // see the dependency.
        const MAYBE_RESULT_TRANSITIVE_CARRIERS: &[&str] = &[
            // core.base.cell — value: Maybe<T> / Result<T,E>
            "OnceCell",
            "LazyCell",
            "RefCell",
            // core.base.iterator — Maybe<Item> in next/peek
            "Iter",
            "IterMut",
            // core.base.error — Result-returning everywhere
            "Error",
            "ErrorChain",
            // core.collections.* — get/find/etc. return Maybe
            "List",
            "Map",
            "Set",
            "Deque",
        ];
        let needs_maybe_result = wanted
            .iter()
            .any(|n| MAYBE_RESULT_TRANSITIVE_CARRIERS.iter().any(|c| *c == n));
        if needs_maybe_result {
            wanted_module_prefixes.insert("core.base.maybe".to_string());
            wanted_module_prefixes.insert("core.base.result".to_string());
            wanted_module_prefixes.insert("core.base".to_string());
            if !wanted.contains("Maybe") {
                to_add.push("Maybe");
            }
            if !wanted.contains("Result") {
                to_add.push("Result");
            }
        }
        for name in to_add {
            wanted.insert(name.to_string());
        }
        let mut fn_modules = 0usize;
        let mut type_modules = 0usize;
        // **Cold-start optimisation**: parallelise the decode step.
        // `archive.load_module` is pure (decompress + deserialise from
        // immutable archive bytes) so the heavy CPU work parallelises
        // perfectly across rayon's thread pool.  The subsequent
        // register_module_filtered/import_archive_module_types passes
        // mutate the codegen and run sequentially against the
        // pre-decoded modules — keeping Rust's aliasing rules clean
        // and producing identical output to the serial path.
        //
        // Measured impact on hello-world: cold-start 623ms → ~150ms
        // when wanted_module_prefixes selects 5+ stdlib modules.
        // Negligible overhead on tiny scripts (1–2 modules) because
        // rayon's `into_par_iter` with single-element input falls
        // through to the serial path.
        // Collect (idx, name) so the parallel decode can call
        // archive.load_module_by_index — bypassing the O(N) name→idx
        // scan that load_module(name) does internally for each call.
        let target_entries: Vec<(usize, String)> = archive
            .index
            .iter()
            .enumerate()
            .filter(|(_, e)| wanted_module_prefixes.contains(&e.name))
            .map(|(i, e)| (i, e.name.clone()))
            .collect();
        let decoded: Vec<(String, VbcModule)> = {
            use rayon::prelude::*;
            target_entries
                .par_iter()
                .filter_map(|(idx, name)| {
                    archive
                        .load_module_by_index(*idx)
                        .ok()
                        .map(|m| (name.clone(), m))
                })
                .collect()
        };
        // Split borrows: ctx and next_func_id are separate fields, but
        // both need &mut from VbcCodegen.  Re-using the same raw-ptr
        // round-trip discipline as the apply_lazy call site in
        // `pipeline/vbc_codegen.rs`.
        let next_id_ptr: *mut u32 = codegen.next_func_id_mut() as *mut u32;
        for (entry_name, module) in &decoded {
            // Function side first so Pass 4 (variant ctors) sees
            // the stable function-id namespace.
            // SAFETY: ctx and next_func_id are non-overlapping fields
            // on the same VbcCodegen — splitting via raw pointer keeps
            // the borrow checker out of the way without breaking
            // aliasing rules.
            let next_id_ref: &mut u32 = unsafe { &mut *next_id_ptr };
            let func_id_remap = register_module_filtered(
                module,
                entry_name,
                codegen.ctx_mut(),
                &wanted,
                next_id_ref,
            );
            fn_modules += 1;
            // Type side — push every non-protocol descriptor.  MUST
            // happen before body merge so the body's TypeId remap
            // (consults `codegen.type_name_to_id`) sees the imported
            // descriptors.
            if !module.types.is_empty() {
                codegen.import_archive_module_types(module);
                type_modules += 1;
            }
            // Body merge — Phase 2 of the precompiled-stdlib epic.
            // For every metadata-registered function, copy its archive
            // bytecode body (with id remap) into `codegen.functions`.
            // Without this, the finalize-time stub-emitter synthesises
            // a `RetV` placeholder and every stdlib method call returns
            // Unit at runtime.
            //
            // **Per-module remap is correct here**: archive function
            // ids are per-module-local (each module's function table
            // starts at 0), so unioning remaps across modules would
            // collapse same-id entries from different modules. Cross-
            // module calls are resolved at codegen-emit time via
            // symbol-name lookup, not via raw bytecode `func_id`
            // references inside archive bodies. The function-id-remap
            // mismatch from task #118 root-causes to MISSING TRANSITIVE
            // MODULES (callee's module not in `wanted_module_prefixes`),
            // tracked separately.
            codegen.merge_archive_function_bodies(module, &func_id_remap);
        }
        // Unqualified-wanted second pass — same logic as apply_lazy's
        // tail block.  Module-prefix gate already filtered the
        // primary pass; this fills in any user code that uses a bare
        // `Maybe.Some(x)` without a `mount` directive.
        //
        // **Cold-start optimisation**: subtract names already
        // registered by Pass 3 of the first walk.  Without this, a
        // hello-world that mounts `core.io.stdio.println` would
        // still trigger a full 568-module decode in the second pass
        // because `println` lingers in the unqualified-wanted set
        // even though Pass 3 already registered the simple name.
        // Each archive load_module is a full decode of compressed
        // bytecode (~50KB per module), so the saved time scales as
        // O(N_modules × decode_cost) — measured ~620ms cold-start
        // collapses to <100ms with this filter on hello-world.
        let unqualified_wanted_full: std::collections::HashSet<String> = wanted
            .iter()
            .filter(|n| !n.contains('.'))
            .cloned()
            .collect();
        // **Cold-start regression guard**: filter out unqualified
        // names that LOOK like types — bare upper-camel-case tokens
        // (`Result`, `Maybe`, `Path`, `Text`, …).  Pre-fix, mounting
        // a stdlib type via `mount core.{Result, Maybe}` added the
        // bare names to the unqualified-wanted set; the second pass
        // then decoded EVERY archive module (574 of them) scanning
        // string tables for these ultra-common names — the single-
        // pool stdlib refactor pushed each archive module to
        // ~10 MB decompressed, so the par_iter filter was
        // materialising ~5 GB of decoded modules in the worst case
        // before discarding most of them.  Types are loaded via
        // `import_archive_module_types` from the qualified-prefix
        // pass; they don't need to drive a function-name probe.
        // Idiomatic Verum stdlib functions are snake_case so this
        // filter has zero false positives on real call sites.
        let unqualified_wanted: std::collections::HashSet<String> = unqualified_wanted_full
            .into_iter()
            .filter(|name| {
                codegen.ctx_mut().lookup_function(name).is_none()
                    && !looks_like_type_name(name)
            })
            .collect();
        if !unqualified_wanted.is_empty() {
            // Parallel decode + match filter for the second pass too.
            // Each archive.load_module(name) is the heaviest CPU step
            // (decompress + bincode deserialise) and runs cleanly in
            // parallel across the immutable archive bytes.  The
            // string-table scan that gates whether the module
            // contributes to ctx.functions is also pure data work,
            // so we fold it into the parallel filter — modules with
            // no matching simple name don't even get returned.
            let candidate_indices: Vec<(usize, String)> = archive
                .index
                .iter()
                .enumerate()
                .filter(|(_, e)| !wanted_module_prefixes.contains(&e.name))
                .map(|(i, e)| (i, e.name.clone()))
                .collect();
            let matched_modules: Vec<(String, VbcModule)> = {
                use rayon::prelude::*;
                candidate_indices
                    .par_iter()
                    .filter_map(|(idx, name)| {
                        let module = archive.load_module_by_index(*idx).ok()?;
                        let any_match = unqualified_wanted.iter().any(|w| {
                            module.strings.iter().any(|(s, _)| s == w)
                        });
                        if any_match {
                            Some((name.clone(), module))
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            for (entry_name, module) in &matched_modules {
                let next_id_ref: &mut u32 = unsafe { &mut *next_id_ptr };
                let func_id_remap = register_module_filtered(
                    module,
                    entry_name,
                    codegen.ctx_mut(),
                    &unqualified_wanted,
                    next_id_ref,
                );
                fn_modules += 1;

                // ALSO import the parent type's descriptor so the
                // typed-form `MakeVariantTyped` gate at
                // `vbc/codegen/expressions.rs::emit_make_variant`
                // succeeds.  Pre-fix this branch deliberately skipped
                // type imports under the assumption that variant-ctor
                // dispatch would survive via the runtime's global-
                // tag-scan fallback in `format_variant_for_print_depth`.
                // That assumption breaks when the binary loads
                // multiple types whose variant tags collide — e.g.
                // user code mounts `core.collections.{map.Map,
                // set.Set}` (which transitively brings in
                // `core.collections.alias_sampler.AliasError` with
                // variants `EmptyWeights` (tag=0) and
                // `NonFiniteWeight(_)` (tag=1)) AND uses
                // `Maybe<Int>` (with `None` (tag=0) and
                // `Some(_)` (tag=1)).  When `Some(3)` lands in the
                // archive via the unqualified-wanted pass but
                // Maybe's TypeDescriptor doesn't, codegen demotes
                // to untyped `MakeVariant` and the runtime's
                // global tag scan picks `NonFiniteWeight(3)` instead
                // of `Some(3)` because AliasError's descriptor
                // appears first in the type table.  Importing the
                // parent type alongside its variant constructors
                // closes that hole — the typed form keeps `Some(3)`
                // tagged with Maybe's TypeId and the runtime
                // resolves the variant name correctly.
                if !module.types.is_empty() {
                    codegen.import_archive_module_types(module);
                    type_modules += 1;
                }
                // Body merge for the unqualified-wanted second pass —
                // same Phase 2 path as the primary pass above. See
                // that site for rationale.
                codegen.merge_archive_function_bodies(module, &func_id_remap);
            }
        }
        (fn_modules, type_modules)
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
        let mut next_id: u32 = 0;
        let stats = populate_ctx_from_archive(archive, &mut ctx, &mut next_id).expect("load");

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
        let mut next_id: u32 = 0;
        let _ = populate_ctx_from_archive(archive, &mut ctx, &mut next_id).unwrap();
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

    /// Source-of-truth pin test for
    /// `WellKnownType::canonical_archive_modules`.  Every module path
    /// returned by the table MUST exist as an archive entry name —
    /// otherwise the loader's `wanted_module_prefixes` extension is a
    /// no-op and `Text.new()` / `List.with_capacity(8)` / etc. fall
    /// through to UndefinedFunction at runtime.
    ///
    /// This test catches three drift modes structurally:
    /// (1) renaming a `core/` module without updating the table;
    /// (2) adding a new well-known type whose carrier module path is
    ///     wrong;
    /// (3) the precompiler bundling a module under a different parent
    ///     than the table assumes.
    #[test]
    fn canonical_archive_modules_match_source() {
        use verum_common::well_known_types::WellKnownType;

        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return, // bootstrap build without archive — skip
        };
        let archive_names: std::collections::HashSet<&str> = archive
            .index
            .iter()
            .map(|e| e.name.as_str())
            .collect();

        let well_known_types = [
            WellKnownType::Text,
            WellKnownType::Char,
            WellKnownType::List,
            WellKnownType::Map,
            WellKnownType::Set,
            WellKnownType::Deque,
            WellKnownType::BTreeMap,
            WellKnownType::BTreeSet,
            WellKnownType::BinaryHeap,
            WellKnownType::Maybe,
            WellKnownType::Result,
            WellKnownType::Heap,
            WellKnownType::Shared,
            WellKnownType::Channel,
            WellKnownType::Mutex,
            WellKnownType::RwLock,
            WellKnownType::Barrier,
            WellKnownType::WaitGroup,
            WellKnownType::Once,
            WellKnownType::Semaphore,
            WellKnownType::Task,
            WellKnownType::Nursery,
            WellKnownType::AtomicInt,
            WellKnownType::AtomicBool,
            WellKnownType::Duration,
            WellKnownType::Instant,
            WellKnownType::Stopwatch,
            WellKnownType::PerfCounter,
            WellKnownType::DeadlineTimer,
            WellKnownType::Never,
            WellKnownType::Ordering,
            WellKnownType::Range,
            WellKnownType::Int,
            WellKnownType::Float,
            WellKnownType::Bool,
        ];

        let mut missing: Vec<(WellKnownType, &'static str)> = Vec::new();
        for wkt in well_known_types {
            // Each well-known type's canonical archive modules — at
            // least ONE of them must resolve.  The list mixes the
            // canonical-source-declared path (`core.text.text`) and
            // grandparent-bundled fallback (`core.text`); the
            // precompiler picks one or the other depending on
            // bundling shape, and the loader is happy with either.
            let mods = wkt.canonical_archive_modules();
            if mods.is_empty() {
                continue;
            }
            let any_present =
                mods.iter().any(|m| archive_names.contains(m));
            if !any_present {
                missing.push((wkt, mods[0]));
            }
        }
        if !missing.is_empty() {
            // Diagnostic: print the closest archive entries by prefix
            // so the maintainer can see the bundling shape.
            for (wkt, expected) in &missing {
                let prefix = expected.split('.').next().unwrap_or("");
                let near: Vec<&str> = archive_names
                    .iter()
                    .filter(|n| n.starts_with(prefix))
                    .copied()
                    .collect();
                eprintln!(
                    "  drift: {:?} expected '{}' or fallback; \
                     archive has under '{}.': {:?}",
                    wkt, expected, prefix, near
                );
            }
            panic!(
                "WellKnownType::canonical_archive_modules drift — \
                 {} types have no archive-resolvable module path",
                missing.len()
            );
        }
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
    match &item.kind {
        ItemKind::Mount(mount_decl) => {
            collect_mount_names(&mount_decl.tree, &[], out);
        }
        ItemKind::Function(func) => {
            harvest_names_in_function(func, out);
        }
        ItemKind::Impl(impl_decl) => {
            harvest_names_in_impl(impl_decl, out);
        }
        ItemKind::Const(decl) => {
            harvest_names_in_type(&decl.ty, out);
            harvest_names_in_expr(&decl.value, out);
        }
        ItemKind::Static(decl) => {
            harvest_names_in_type(&decl.ty, out);
            harvest_names_in_expr(&decl.value, out);
        }
        _ => {}
    }
}

/// Walk a function declaration harvesting every identifier in its
/// signature + body that could refer to a stdlib symbol.  The
/// archive-load filter (`register_module_filtered`) gates loading
/// on this set: a function whose simple/qualified name is not
/// here AND whose parent type is not here gets skipped.
fn harvest_names_in_function(
    func: &verum_ast::decl::FunctionDecl,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_common::Maybe;
    use verum_ast::decl::{FunctionBody, FunctionParamKind};
    for param in func.params.iter() {
        if let FunctionParamKind::Regular { ty, .. } = &param.kind {
            harvest_names_in_type(ty, out);
        }
    }
    if let Maybe::Some(ret) = &func.return_type {
        harvest_names_in_type(ret, out);
    }
    if let Maybe::Some(body) = &func.body {
        match body {
            FunctionBody::Block(block) => harvest_names_in_block(block, out),
            FunctionBody::Expr(expr) => harvest_names_in_expr(expr, out),
        }
    }
}

fn harvest_names_in_impl(
    impl_decl: &verum_ast::decl::ImplDecl,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::decl::{ImplItemKind, ImplKind};
    match &impl_decl.kind {
        ImplKind::Inherent(for_type) => harvest_names_in_type(for_type, out),
        ImplKind::Protocol {
            protocol, for_type, ..
        } => {
            harvest_names_in_path(protocol, out);
            harvest_names_in_type(for_type, out);
        }
    }
    for impl_item in impl_decl.items.iter() {
        if let ImplItemKind::Function(func) = &impl_item.kind {
            harvest_names_in_function(func, out);
        }
    }
}

fn harvest_names_in_block(
    block: &verum_ast::expr::Block,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_common::Maybe;
    for stmt in block.stmts.iter() {
        harvest_names_in_stmt(stmt, out);
    }
    if let Maybe::Some(tail) = &block.expr {
        harvest_names_in_expr(tail, out);
    }
}

fn harvest_names_in_stmt(
    stmt: &verum_ast::Stmt,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_common::Maybe;
    use verum_ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Let { ty, value, .. } => {
            if let Maybe::Some(t) = ty {
                harvest_names_in_type(t, out);
            }
            if let Maybe::Some(v) = value {
                harvest_names_in_expr(v, out);
            }
        }
        StmtKind::LetElse {
            ty,
            value,
            else_block,
            ..
        } => {
            if let Maybe::Some(t) = ty {
                harvest_names_in_type(t, out);
            }
            harvest_names_in_expr(value, out);
            harvest_names_in_block(else_block, out);
        }
        StmtKind::Expr { expr, .. } => harvest_names_in_expr(expr, out),
        StmtKind::Item(item) => collect_referenced_function_names(item, out),
        StmtKind::Defer(e) | StmtKind::Errdefer(e) => harvest_names_in_expr(e, out),
        StmtKind::Provide { value, .. } => harvest_names_in_expr(value, out),
        StmtKind::ProvideScope { value, block, .. } => {
            harvest_names_in_expr(value, out);
            harvest_names_in_expr(block, out);
        }
        _ => {}
    }
}

/// The expression walker.  Pushes:
///   * Every segment of every Path expression (so `Text` from
///     `Text.with_capacity` lands in `wanted` and the
///     `is_method_of_wanted_type` filter in
///     `register_module_filtered` triggers).
///   * The full dotted form of multi-segment Paths.
///   * For `MethodCall { receiver: Path(p), method }`, the
///     qualified `<last_seg(p)>.<method>` so static-method
///     dispatch (`Text.with_capacity(64)`) finds the function in
///     the archive's `simple_name = "Text.with_capacity"` slot.
///   * Every type-expression encountered in `as` / `cast` / type
///     args.
///
/// Over-inclusion is harmless (extra archive lookups skip
/// quickly via the wanted-set hash); under-inclusion fails the
/// build with `no method named X found for type Y`.
fn harvest_names_in_expr(
    expr: &verum_ast::Expr,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_common::Maybe;
    use verum_ast::expr::ExprKind;
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Path(path) => harvest_names_in_path(path, out),
        ExprKind::Binary { left, right, .. } => {
            harvest_names_in_expr(left, out);
            harvest_names_in_expr(right, out);
        }
        ExprKind::Unary { expr, .. } => harvest_names_in_expr(expr, out),
        ExprKind::NamedArg { value, .. } => harvest_names_in_expr(value, out),
        ExprKind::Call { func, type_args, args } => {
            harvest_names_in_expr(func, out);
            for ga in type_args.iter() {
                harvest_names_in_generic_arg(ga, out);
            }
            for a in args.iter() {
                harvest_names_in_expr(a, out);
            }
        }
        ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } => {
            // Static-method qualified form: when the receiver is a
            // path (`Text`), the archive carries the inherent
            // method as `simple_name = "Text.with_capacity"`,
            // and `register_module_filtered` registers it only if
            // either `simple_name` itself is in `wanted` OR the
            // parent type is.  Push BOTH to handle either gate.
            if let ExprKind::Path(path) = &receiver.kind {
                if let Some(last) = last_path_name(path) {
                    out.insert(format!("{}.{}", last, method.name));
                }
            }
            harvest_names_in_expr(receiver, out);
            for ga in type_args.iter() {
                harvest_names_in_generic_arg(ga, out);
            }
            for a in args.iter() {
                harvest_names_in_expr(a, out);
            }
        }
        ExprKind::Field { expr, .. }
        | ExprKind::OptionalChain { expr, .. }
        | ExprKind::TupleIndex { expr, .. } => harvest_names_in_expr(expr, out),
        ExprKind::Index { expr, index } => {
            harvest_names_in_expr(expr, out);
            harvest_names_in_expr(index, out);
        }
        ExprKind::Pipeline { left, right } | ExprKind::NullCoalesce { left, right } => {
            harvest_names_in_expr(left, out);
            harvest_names_in_expr(right, out);
        }
        ExprKind::Cast { expr, ty } => {
            harvest_names_in_expr(expr, out);
            harvest_names_in_type(ty, out);
        }
        ExprKind::Try(e) | ExprKind::TryBlock(e) => harvest_names_in_expr(e, out),
        ExprKind::Block(block) => harvest_names_in_block(block, out),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            harvest_names_in_block(then_branch, out);
            if let Maybe::Some(eb) = else_branch {
                harvest_names_in_expr(eb, out);
            }
        }
        ExprKind::Match { expr, arms } => {
            harvest_names_in_expr(expr, out);
            for arm in arms.iter() {
                if let Maybe::Some(g) = &arm.guard {
                    harvest_names_in_expr(g, out);
                }
                harvest_names_in_expr(&arm.body, out);
            }
        }
        ExprKind::Loop { body, .. } => harvest_names_in_block(body, out),
        ExprKind::While {
            condition, body, ..
        } => {
            harvest_names_in_expr(condition, out);
            harvest_names_in_block(body, out);
        }
        ExprKind::For { iter, body, .. } => {
            harvest_names_in_expr(iter, out);
            harvest_names_in_block(body, out);
        }
        ExprKind::Closure { body, .. } => harvest_names_in_expr(body, out),
        ExprKind::Return(e) => {
            if let Maybe::Some(e) = e {
                harvest_names_in_expr(e, out);
            }
        }
        ExprKind::Tuple(items) => {
            for e in items.iter() {
                harvest_names_in_expr(e, out);
            }
        }
        ExprKind::Async(block) | ExprKind::Unsafe(block) => harvest_names_in_block(block, out),
        ExprKind::Await(e) | ExprKind::Throw(e) | ExprKind::Yield(e) | ExprKind::Typeof(e) => {
            harvest_names_in_expr(e, out);
        }
        ExprKind::Break { value, .. } => {
            if let Maybe::Some(v) = value {
                harvest_names_in_expr(v, out);
            }
        }
        ExprKind::TypeExpr(ty) => harvest_names_in_type(ty, out),
        ExprKind::Record { path, fields, base } => {
            // Critical for stdlib variant constructors: a literal like
            // `ShellError.SpawnFailed { command, reason }` must seed
            // the wanted-set with both `ShellError` (parent) and
            // `SpawnFailed` (variant) so the archive-load pass
            // includes the parent module's TypeDescriptor and Pass 4
            // (variant ctor registration) fires.  Pre-fix the lazy
            // walker missed these because `Record` fell into the
            // catch-all and the parent never made it to `wanted`,
            // so register_module_filtered's parent_in_scope gate
            // rejected the type's variants and codegen fell through
            // to the plain-record path with field-name-id slots.
            harvest_names_in_path(path, out);
            for f in fields.iter() {
                if let Maybe::Some(v) = &f.value {
                    harvest_names_in_expr(v, out);
                }
            }
            if let Maybe::Some(b) = base {
                harvest_names_in_expr(b, out);
            }
        }
        // Other expression forms (interpolation, generators, …)
        // are walked best-effort — over-inclusion is harmless.
        _ => {}
    }
}

fn harvest_names_in_path(
    path: &verum_ast::ty::Path,
    out: &mut std::collections::HashSet<String>,
) {
    let segs: Vec<String> = path
        .segments
        .iter()
        .filter_map(|seg| match seg {
            verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
            _ => None,
        })
        .collect();
    for s in &segs {
        out.insert(s.clone());
    }
    if segs.len() > 1 {
        out.insert(segs.join("."));
    }
}

/// Heuristic: a bare unqualified name LOOKS like a type when it
/// starts with an upper-case ASCII letter and contains no
/// underscores or special chars.  Catches `Result`, `Maybe`,
/// `Path`, `PathBuf`, `Text`, etc. — every stdlib type name.
/// Functions in idiomatic Verum stdlib are snake_case (`path_exists`,
/// `current_dir`, …) so this filter has zero false positives on
/// real function call sites.  False negatives (an upper-case
/// function name) only mean we waste one round-trip through the
/// second pass — no correctness loss.
fn looks_like_type_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    // Must be entirely alphanumeric (rejects sigils/operators,
    // `__type_params_*` registry tokens, etc.).
    name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn last_path_name(path: &verum_ast::ty::Path) -> Option<String> {
    path.segments.iter().rev().find_map(|seg| match seg {
        verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
        _ => None,
    })
}

fn harvest_names_in_type(
    ty: &verum_ast::ty::Type,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::ty::TypeKind;
    match &ty.kind {
        TypeKind::Path(path) => harvest_names_in_path(path, out),
        TypeKind::Generic { base, args } => {
            harvest_names_in_type(base, out);
            for ga in args.iter() {
                harvest_names_in_generic_arg(ga, out);
            }
        }
        TypeKind::Reference { inner, .. }
        | TypeKind::CheckedReference { inner, .. }
        | TypeKind::UnsafeReference { inner, .. } => harvest_names_in_type(inner, out),
        TypeKind::Tuple(items) => {
            for t in items.iter() {
                harvest_names_in_type(t, out);
            }
        }
        TypeKind::Array { element, .. } => harvest_names_in_type(element, out),
        TypeKind::Slice(elem) => harvest_names_in_type(elem, out),
        TypeKind::Function {
            params, return_type, ..
        } => {
            for p in params.iter() {
                harvest_names_in_type(p, out);
            }
            harvest_names_in_type(return_type, out);
        }
        TypeKind::Qualified {
            self_ty,
            trait_ref,
            ..
        } => {
            harvest_names_in_type(self_ty, out);
            harvest_names_in_path(trait_ref, out);
        }
        TypeKind::AssociatedType { base, .. } => harvest_names_in_type(base, out),
        _ => {}
    }
}

fn harvest_names_in_generic_arg(
    ga: &verum_ast::ty::GenericArg,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::ty::GenericArg;
    match ga {
        GenericArg::Type(ty) => harvest_names_in_type(ty, out),
        _ => {}
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
    next_id: &mut u32,
) -> std::collections::HashMap<u32, verum_vbc::module::FunctionId> {
    // **Cold-start optimisation**: build a `StringId → &str` reverse
    // index once per module call.  The default `module.strings.get(id)`
    // is an O(N) linear scan of the IndexMap (it's keyed by string,
    // not by id), so the per-call cost compounds: a typical stdlib
    // module has ~1000 strings, and Pass 3 + Pass 4 perform tens of
    // get calls per type/variant/function, producing ~10^6 string
    // comparisons per archive load.  Pre-building the reverse map is
    // O(N) once and then every subsequent lookup is O(1).
    let name_by_id: HashMap<verum_vbc::types::StringId, &str> = module
        .strings
        .iter()
        .map(|(s, id)| (id, s))
        .collect();
    let lookup = |id: verum_vbc::types::StringId| -> Option<&str> {
        name_by_id.get(&id).copied()
    };
    let mut type_id_to_name: HashMap<TypeId, String> = HashMap::new();
    for ty in &module.types {
        if let Some(name) = lookup(ty.name) {
            type_id_to_name.insert(ty.id, name.to_string());
        }
    }
    let mut variant_index: HashMap<String, VariantHit> = HashMap::new();
    for ty in &module.types {
        let parent_name = match lookup(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        for variant in &ty.variants {
            let vname = match lookup(variant.name) {
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
    // Per-module ID remap.  Each archive function gets a globally-
    // unique FunctionId allocated from `next_id` so two archive
    // modules with overlapping local ids don't collapse onto a
    // single ctx.functions slot at codegen finalisation time.  See
    // the long-form rationale in `apply_lazy`'s caller comment.
    //
    // **Phase 2 of the body-merge epic** — accumulate the
    // archive-function-id → user-codegen-function-id mapping in
    // `func_id_remap` so the caller can pass it to
    // `VbcCodegen::merge_archive_function_bodies` immediately after
    // this function returns. Without that, the metadata pass would
    // register `Maybe.is_some` (etc.) but never emit a real body,
    // and the finalize-time stub-emitter would synthesise a `RetV`
    // placeholder that returns Unit at every call site.
    let mut func_id_remap: std::collections::HashMap<u32, verum_vbc::module::FunctionId> =
        std::collections::HashMap::new();
    for fn_desc in &module.functions {
        // **Cold-start optimisation**: gate-then-resolve order.  The
        // simple_name lookup is O(1) via the reverse-index helper but
        // we can do even better by short-circuiting when the function
        // can never match (no qualified prefix and not a method of a
        // wanted type).  Gating BEFORE allocating String saves all
        // the no-match-no-allocation cases from a `to_string()` clone
        // per module function.
        let simple_name_str = match lookup(fn_desc.name) {
            Some(s) => s,
            None => continue,
        };
        let qualified_borrowed: String = format!("{}.{}", module_name, simple_name_str);
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
        let is_method_of_wanted_type = simple_name_str
            .find('.')
            .map(|dot_idx| {
                let parent = &simple_name_str[..dot_idx];
                wanted.contains(parent)
            })
            .unwrap_or(false);
        if !wanted.contains(simple_name_str)
            && !wanted.contains(&qualified_borrowed)
            && !is_method_of_wanted_type
        {
            continue;
        }
        let simple_name = simple_name_str.to_string();
        let qualified = qualified_borrowed;
        // Allocate a fresh globally-unique id so emit_missing_stub_descriptors
        // produces a one-to-one stub at this slot.  Without remapping,
        // multiple archive modules' local id=0 collide on the same
        // ctx.functions slot, the longest-dotted name wins, and Call
        // sites that intended a different function dispatch through the
        // wrong intercept (or fall through to a Unit-returning stub).
        let new_id = verum_vbc::module::FunctionId(*next_id);
        *next_id = next_id.saturating_add(1);
        func_id_remap.insert(fn_desc.id.0, new_id);
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
                lookup(p.name)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("_arg{}", i))
            })
            .collect();
        // Restore param type names from the archive's TypeRef so the
        // codegen's type-aware bare-name disambiguation has the data it
        // needs to pick between sibling stdlib functions sharing a
        // simple name (e.g. `core.sys.test_bit(USize, USize)` vs
        // `core.net.tls13.handshake.test_bit(&Bucket, Int)`). Without
        // this, `lookup_function_with_arity` would race on bare-name
        // first-wins archive load order and dispatch to whichever
        // archive entry loaded first — surfacing at runtime as a
        // wrong-body call (Unit return for the USize overload, null
        // pointer for the &Bucket overload, etc.). The empty-vec
        // sentinel that previously lived here is the original cause
        // of the cross-module dispatch defect tracked under #16.
        let param_type_names: Vec<String> = fn_desc
            .params
            .iter()
            .map(|p| {
                type_ref_simple_name(&p.type_ref, module).unwrap_or_default()
            })
            .collect();
        let return_type_name = type_ref_simple_name(&fn_desc.return_type, module);
        let return_type_inner = type_ref_inner_generics(&fn_desc.return_type, module);
        // #87 — restore the intrinsic-name marker that was serialised
        // on the archive side.  Mirrors the populate_ctx_from_archive
        // site; without this, inlinable stdlib constants surface as
        // `UndefinedVariable` at the use site after the archive
        // round-trip.
        let intrinsic_name = fn_desc
            .intrinsic_name
            .and_then(|sid| lookup(sid).map(|s| s.to_string()));
        let info = FunctionInfo {
            id: new_id,
            param_count: fn_desc.params.len(),
            param_names,
            param_type_names,
            is_async: fn_desc
                .properties
                .contains(verum_vbc::types::PropertySet::ASYNC),
            is_generator: fn_desc.is_generator,
            contexts: vec![],
            return_type: Some(fn_desc.return_type.clone()),
            yield_type: fn_desc.yield_type.clone(),
            intrinsic_name,
            variant_tag,
            parent_type_name,
            variant_payload_types,
            is_partial_pattern: false,
            takes_self_mut_ref: false,
            return_type_name,
            return_type_inner,
            // #97 — see populate_ctx_from_archive for the rationale.
            is_const: fn_desc.is_const,
            is_transparent_wrapper: false,
        };
        ctx.register_function(qualified.clone(), info.clone());
        // ALSO register under any qualified path from `wanted` whose
        // last segment matches `simple_name`.  This closes the
        // grandparent-bundling discrepancy: when the precompiler
        // bundles `core.shell.script.args` under archive entry
        // `core.shell` (because `script.vr` declares `module
        // script;`), the entry-derived `qualified` name is
        // `core.shell.args` — but the user's `mount
        // core.shell.script.{args as script_args}` looks up
        // `core.shell.script.args`.  Without this fanout, the
        // simple-name `args` ends up as the only ctx.functions
        // entry under the function's id, `emit_missing_stub_descriptors`
        // picks the bare name as the descriptor, and runtime
        // intercepts that key on a deeper qualifier (e.g.
        // `func_name.contains("script.args")`) miss.
        for w in wanted.iter() {
            if w == &qualified {
                continue;
            }
            if w.contains('.')
                && w.rsplit('.').next() == Some(simple_name.as_str())
                && ctx.lookup_function(w).is_none()
            {
                ctx.register_function(w.clone(), info.clone());
            }
        }
        // **Arity-disambiguation contract.** Always go through
        // `register_function` for the simple-name registration so its
        // `name#arity` collision branch fires when this is the second-
        // (or third-, …) registration with the same simple name but
        // different param count.  The previous `lookup_function(...)
        // .is_none()` gate dropped multi-arity simple-name entries on
        // the floor before they could be assigned an arity-qualified
        // alternate key — surfaced as the snowflake/uuid/ulid suite
        // failures where user code calls `parse(id, epoch_ms)` (2-arg
        // form from `core.base.snowflake`) but the dispatcher routes
        // to a sibling stdlib's 1-arg `parse` because `parse#2` was
        // never registered.  `register_function`'s own arity branch
        // does the right thing here: same-arity → first-wins (matches
        // the prior gate's behaviour); different-arity → store under
        // `name#arity` so `lookup_function_with_arity` can pick the
        // right one.
        ctx.register_function(simple_name, info);
    }

    // Pass 4: register every sum-type's variant constructors from
    // `module.types`.  In the source-driven path,
    // `register_type_constructors` writes variant constructor
    // FunctionInfos into ctx.functions (with sentinel IDs and
    // `variant_tag` set).  These sentinel-IDed entries are NOT real
    // FunctionDescriptors in the VBC module — they live only in the
    // codegen context — so they don't survive archive serialisation.
    // Without this pass, qualified record-variant literals like
    // `ShellError.SpawnFailed { command, reason }` fall through every
    // variant-tag lookup, hit the plain-record codegen fallback, and
    // emit `New + SetField #<interned-name-id>` — runtime then crashes
    // with `field write out of bounds: field index N exceeds object
    // data size 16`.
    //
    // Walk every TypeDescriptor's variants — when the type name appears
    // in `wanted` (or has a method-of-wanted-type fanout), register the
    // variant constructor with a sentinel `u32::MAX - tag` ID, matching
    // the source-driven path's discipline.  The `variant_index` HashMap
    // built above already tracks first-wins per simple name, so re-using
    // it for collision detection keeps the archive-load path bit-aligned
    // with `register_type_constructors`.
    use verum_vbc::module::FunctionId;
    for ty in &module.types {
        let parent_name_str = match lookup(ty.name) {
            Some(s) => s,
            None => continue,
        };
        // Filter: only register variants of types in scope. A type is
        // "in scope" when its name is in `wanted`, OR when one of its
        // variants' simple names is wanted (covers `mount Foo.Variant`).
        // Without this gate every type in every loaded archive module
        // dumps its variants into ctx.functions — historically that's
        // the path that produced bare-name collisions like
        // `Closed`/`Open`/`Done` from a dozen unrelated stdlib types.
        let parent_in_scope = wanted.contains(parent_name_str);
        let any_variant_wanted = ty.variants.iter().any(|v| {
            match lookup(v.name) {
                Some(s) => wanted.contains(s),
                None => false,
            }
        });
        // Method-of-wanted-type fanout: when the user writes
        // `mount core.shell.{ShellError}` the typechecker may further
        // surface qualified `ShellError.SpawnFailed` as wanted at
        // record-literal compile time, but the lazy walker's `wanted`
        // set is built once before codegen runs. The conservative
        // policy here: also include variants whose qualified
        // `<ParentType>.<VariantName>` form is wanted.
        let qualified_variant_wanted = ty.variants.iter().any(|v| {
            let vn = match lookup(v.name) {
                Some(s) => s,
                None => return false,
            };
            let qualified = format!("{}.{}", parent_name_str, vn);
            wanted.contains(&qualified)
        });
        if !parent_in_scope && !any_variant_wanted && !qualified_variant_wanted {
            continue;
        }
        let parent_name = parent_name_str.to_string();
        for variant in &ty.variants {
            let vname = match lookup(variant.name) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let qualified = format!("{}.{}", parent_name, vname);
            // Skip if a real FunctionDescriptor already covered this
            // (e.g. tuple variants do appear as ctor functions in
            // some stdlib modules — Pass 3 above already registered
            // them with the right tag).
            if ctx.lookup_function(&qualified).is_some() {
                continue;
            }
            // Compute arity + per-field info.  Tuple variants carry
            // arity in `variant.arity`; record variants carry their
            // declared field count via `fields.len()`.
            let (arity, payload_field_types) = match variant.kind {
                VariantKind::Unit => (0usize, Vec::<String>::new()),
                VariantKind::Tuple => (
                    variant.arity as usize,
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_simple_name(&f.type_ref, module).unwrap_or_default()
                        })
                        .collect(),
                ),
                VariantKind::Record => (
                    variant.fields.len(),
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_simple_name(&f.type_ref, module).unwrap_or_default()
                        })
                        .collect(),
                ),
            };
            let param_names: Vec<String> = (0..arity).map(|i| format!("_{}", i)).collect();
            let info = FunctionInfo {
                id: FunctionId(u32::MAX - variant.tag),
                param_count: arity,
                param_names,
                // Variant constructor params take payload field types so
                // type-aware bare-name disambiguation works for variant
                // ctor calls too.
                param_type_names: payload_field_types.clone(),
                is_async: false,
                is_generator: false,
                contexts: vec![],
                return_type: None,
                yield_type: None,
                intrinsic_name: None,
                variant_tag: Some(variant.tag),
                parent_type_name: Some(parent_name.clone()),
                variant_payload_types: if payload_field_types.is_empty() {
                    None
                } else {
                    Some(payload_field_types)
                },
                is_partial_pattern: false,
                takes_self_mut_ref: false,
                return_type_name: Some(parent_name.clone()),
                return_type_inner: None,
                is_const: false,
            is_transparent_wrapper: false,
            };
            ctx.register_function(qualified, info);
            // Deliberately skip simple-name registration — see the
            // matching site in `register_module_filtered` for the
            // collision rationale (user `type ... is | Help | ...`
            // would otherwise be silently de-aliased).
        }
    }
    func_id_remap
}
