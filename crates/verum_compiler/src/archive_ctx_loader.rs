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

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;

use verum_vbc::archive::VbcArchive;
use verum_vbc::codegen::{CodegenContext, FunctionInfo};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::VbcModule;
use verum_vbc::types::{StringId, TypeId, TypeRef, VariantKind};

/// Per-phase accounting for the archive→ctx load (T0753).
///
/// The load has five distinct phases and one wall-clock number, so a
/// regression in any of them reads as "the compile got slower".  Named
/// for the QUESTION it answers — which phase owns the time — not for
/// the fix it motivates, so the numbers stay meaningful after a fix
/// lands and are the check that it worked.
///
/// Thread-local because the codegen test harness runs one compile per
/// test thread; a shared accumulator would interleave two unrelated
/// loads into one meaningless total.  Behind `VERUM_TRACE_LOADCOST`,
/// so an untraced build pays one relaxed env read per phase.
pub(crate) mod loadcost {
    use std::cell::RefCell;
    use std::time::Duration;

    thread_local! {
        static STAGES: RefCell<Vec<(&'static str, Duration)>> =
            const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn enabled() -> bool {
        std::env::var_os("VERUM_TRACE_LOADCOST").is_some()
    }

    pub(crate) fn record(stage: &'static str, d: Duration) {
        if !enabled() {
            return;
        }
        STAGES.with_borrow_mut(|v| v.push((stage, d)));
    }

    /// Run `f`, recording its duration under `stage`.  Wrapping rather
    /// than bracketing: the phases below return early from several
    /// places, and a start/stop pair would time whichever exit the
    /// author remembered.
    pub(crate) fn timed<T>(stage: &'static str, f: impl FnOnce() -> T) -> T {
        if !enabled() {
            return f();
        }
        let t0 = std::time::Instant::now();
        let r = f();
        record(stage, t0.elapsed());
        r
    }

    /// Print and reset.  `total` is measured by the caller around the
    /// whole load, so the gap between it and the sum of the phases is
    /// itself a reading — it names how much lives outside any phase
    /// this instrument covers.
    pub(crate) fn report(label: &str, total: Duration) {
        if !enabled() {
            return;
        }
        STAGES.with_borrow_mut(|v| {
            let accounted: Duration = v.iter().map(|(_, d)| *d).sum();
            eprintln!(
                "[loadcost] {}: {:.1} ms total",
                label,
                total.as_secs_f64() * 1000.0
            );
            for (s, d) in v.iter() {
                eprintln!(
                    "[loadcost]   {:<26} {:>9.1} ms",
                    s,
                    d.as_secs_f64() * 1000.0
                );
            }
            eprintln!(
                "[loadcost]   {:<26} {:>9.1} ms",
                "(unaccounted)",
                total.saturating_sub(accounted).as_secs_f64() * 1000.0
            );
            v.clear();
        });
    }
}

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

/// Merge an archive entry's `module_name` with a function's
/// precompiler-assigned `simple_name` into the function's canonical
/// fully-qualified codegen key.
///
/// The precompiler stores `simple_name` in one of three shapes
/// depending on the source file's `module X.Y;` declaration:
///
///   * **Bare leaf** (no dot). Example: descriptor `new` for
///     `core/text/text.vr`. Canonical = `<module_name>.<simple_name>`.
///   * **Relative submodule** (leading segments overlap module_name's
///     trailing tail). Example: descriptor `sys.bitfield.test_bit`
///     for archive entry `core.sys`. Canonical drops the overlap →
///     `core.sys.bitfield.test_bit`.
///   * **Fully-rooted submodule** (descriptor starts with the cog
///     prefix). Example: descriptor `core.async.future.ready` for
///     archive entry `core.async`. Canonical = descriptor verbatim.
///
/// Algorithm: find the longest suffix of `module_name`'s segments
/// that matches a prefix of `simple_name`'s segments; the canonical
/// key is `module_name[..non_overlap]` followed by all of
/// `simple_name`. Bare leaves degenerate cleanly because their
/// segment count is 1 and overlap-with-anything-longer is `0`.
///
/// **Drift contract**: any registration site that synthesises a
/// qualified codegen key from `(module_name, simple_name)` MUST
/// route through this function. The user-side codegen lookup probes
/// the canonical form (`cog.entry.submodule.method`), so any
/// asymmetry between registration and lookup surfaces as a silent
/// runtime dispatch miss — e.g. `core.sys.bitfield.test_bit`
/// dispatching to `core.net.tls13.handshake.zero_rtt_antireplay.test_bit`
/// because bitfield's canonical key was missing and the bare-name
/// fallback claimed the first registered `test_bit`.
/// Detect whether the first parameter of a function descriptor is
/// `&mut self` — i.e. the receiver is passed by mutable CBGR
/// reference, so the method body's `*self = value` writeback MUST
/// flow back to the caller's binding via the user-side codegen's
/// `RefMut`-then-pass-as-receiver dispatch (`compile_method_call`
/// at `crates/verum_vbc/src/codegen/expressions.rs:~8641`).
///
/// **Architectural rule** (closes task #11): every `FunctionInfo`
/// constructed from an archived `FunctionDescriptor` MUST set
/// `takes_self_mut_ref` to the result of this predicate.  Pre-fix,
/// every archive-side `FunctionInfo` literal hardcoded
/// `takes_self_mut_ref: false`, so every `&mut self` stdlib
/// method (Maybe.take / Maybe.replace / Maybe.insert /
/// Maybe.get_or_insert / Text.push_str / List.push / …) was
/// dispatched with the receiver passed BY VALUE — the
/// `*self = value` inside the body wrote into a stack slot the
/// caller would never re-read.  Symptom: `m.take()` returned
/// `Some(x)` but `m` stayed `Some(x)` (the take's "leaves None
/// in its place" invariant silently failed).
///
/// The predicate inspects `param.type_ref`: if it's
/// `Reference { mutability: Mutable, inner: T }` AND the
/// param name is `self`, the receiver is `&mut self`.
/// BAKED-DEFAULT-ARG-1: reconstruct a parameter's default literal from
/// the VBC descriptor's const channel as a synthetic AST expression, so
/// user-side call sites can inject omitted trailing args against baked
/// functions exactly like local ones. Int/Float/String consts only —
/// the writer (codegen `param_default_const_id`) interns exactly those.
fn const_to_literal_expr(
    module: &VbcModule,
    cid: verum_vbc::ConstId,
) -> Option<verum_ast::Expr> {
    use verum_ast::literal::{Literal, StringLit};
    use verum_ast::span::Span;
    use verum_ast::ExprKind;
    let constant = module.constants.get(cid.0 as usize)?;
    let lit = match constant {
        verum_vbc::module::Constant::Int(v) => Literal::int(*v as i128, Span::default()),
        verum_vbc::module::Constant::Float(v) => Literal::float(*v, Span::default()),
        verum_vbc::module::Constant::String(sid) => {
            let s = module.get_string(*sid)?;
            Literal::new(
                verum_ast::literal::LiteralKind::Text(StringLit::Regular(
                    verum_common::Text::from(s),
                )),
                Span::default(),
            )
        }
        _ => return None,
    };
    Some(verum_ast::Expr::new(ExprKind::Literal(lit), Span::default()))
}

/// Extract the per-param default expressions for a baked descriptor;
/// None when no param declares one.
fn descriptor_param_defaults(
    module: &VbcModule,
    fn_desc: &verum_vbc::module::FunctionDescriptor,
) -> Option<Vec<Option<verum_ast::Expr>>> {
    if fn_desc.params.iter().all(|p| p.default.is_none()) {
        return None;
    }
    Some(
        fn_desc
            .params
            .iter()
            .map(|p| p.default.and_then(|cid| const_to_literal_expr(module, cid)))
            .collect(),
    )
}

fn param_is_mut_self_ref(
    param: &verum_vbc::module::ParamDescriptor,
    module: &verum_vbc::module::VbcModule,
) -> bool {
    // `module.strings.iter()` yields `(&str, StringId)` — the `&str`
    // binding is used directly (avoids the unstable
    // `str_as_str`/`String::as_str` reborrow path).
    let is_self = module
        .strings
        .iter()
        .any(|(s, id)| id == param.name && s == "self");
    if !is_self {
        return false;
    }
    matches!(
        &param.type_ref,
        verum_vbc::types::TypeRef::Reference {
            mutability: verum_vbc::types::Mutability::Mutable,
            ..
        }
    )
}

/// Convenience wrapper: returns `takes_self_mut_ref` for a function
/// descriptor.  Inspects the first parameter via [`param_is_mut_self_ref`].
fn fn_takes_self_mut_ref(
    fn_desc: &verum_vbc::module::FunctionDescriptor,
    module: &verum_vbc::module::VbcModule,
) -> bool {
    fn_desc
        .params
        .first()
        .is_some_and(|p| param_is_mut_self_ref(p, module))
}

fn merge_module_and_simple_name(module_name: &str, simple_name: &str) -> String {
    if !simple_name.contains('.') {
        // Bare leaf — the precompiler did no module promotion.
        // Prepend module_name unconditionally.
        return format!("{}.{}", module_name, simple_name);
    }
    let module_segs: Vec<&str> = module_name.split('.').collect();
    let simple_segs: Vec<&str> = simple_name.split('.').collect();
    // Longest overlap: try `module_segs[k..]` against `simple_segs[..len-k]`
    // for k decreasing from |module_segs|.min(|simple_segs|) down to 1.
    // First match wins (longest). k=0 (no overlap) falls through to
    // the prepend branch at the bottom.
    let max_overlap = module_segs.len().min(simple_segs.len());
    for overlap_len in (1..=max_overlap).rev() {
        let module_suffix = &module_segs[module_segs.len() - overlap_len..];
        let simple_prefix = &simple_segs[..overlap_len];
        if module_suffix == simple_prefix {
            // Emit non-overlapping module_name prefix + full simple_name.
            let prefix_len = module_segs.len() - overlap_len;
            if prefix_len == 0 {
                return simple_name.to_string();
            }
            let mut out = String::with_capacity(module_name.len() + simple_name.len() + 1);
            for (i, seg) in module_segs[..prefix_len].iter().enumerate() {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(seg);
            }
            out.push('.');
            out.push_str(simple_name);
            return out;
        }
    }
    // No overlap — descriptor's leading segment is unrelated to
    // module_name (e.g. tls13's `tls13.handshake....` under
    // `core.net`). Prepend module_name verbatim.
    format!("{}.{}", module_name, simple_name)
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
    // Task #25 — index by QUALIFIED name `<parent>.<variant>` instead
    // of bare variant name.  Bare-keyed first-wins indexing was the
    // architectural defect: when two stdlib types declare a variant
    // sharing a simple name (canonical example: `Result.Err(E)` and
    // `WebSocketDecodeError.Err(Text)`, but also `Maybe.None` shared
    // with every other type's unit `None`), the per-function-descriptor
    // lookup `variant_index.get("Err")` would non-deterministically
    // return whichever parent's entry registered first.  The chosen
    // hit's `payload_field_types` then leaked into the wrong
    // function descriptor — `Result.Err` got registered with
    // WebSocketDecodeError's `["Text"]` payload, so destructure-bound
    // `e` carried type Text and the downstream `e + 1` codegen
    // routed `+` to Text concat → "7" + "1" = "71" instead of 8.
    //
    // Key by qualified name so each parent owns its own hit
    // unambiguously; the lookup at the function-descriptor pass
    // composes `<fn_desc.parent_type_name>.<simple_name>` for
    // exact resolution.  The bare lookup is preserved as a
    // fallback for the (rare) variant constructor whose
    // descriptor predates `parent_type` population.
    let mut variant_index: HashMap<String, VariantHit> = HashMap::new();
    let mut variant_index_qualified: HashMap<String, VariantHit> = HashMap::new();
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
            let hit = VariantHit {
                parent_type_name: parent_name.clone(),
                tag: variant.tag,
                kind: variant.kind,
                payload_field_types,
                arity: variant.arity as usize,
            };
            // Qualified: always insert (no collision possible since
            // every `<parent>.<variant>` pair is unique by construction).
            let qualified_key = format!("{}.{}", parent_name, vname);
            variant_index_qualified.insert(qualified_key, hit.clone());
            // Simple-name: first-wins fallback for orphan descriptors.
            variant_index.entry(vname).or_insert(hit);
        }
    }

    // Pass 3: walk functions, build FunctionInfo, register under
    // qualified + (collision-aware) simple keys.
    for fn_desc in &module.functions {
        let simple_name = match lookup(fn_desc.name) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Variant ctor lookup — prefer the qualified `<parent>.<variant>`
        // index when the function descriptor records its parent type.
        // Only fall back to the simple-name index when no parent is
        // attached (defensive — shouldn't happen for variant ctors
        // emitted by the codegen).
        let parent_hint: Option<String> = fn_desc
            .parent_type
            .and_then(|tid| type_id_to_name.get(&tid).cloned());
        let variant_hit = parent_hint
            .as_ref()
            .and_then(|parent| {
                variant_index_qualified.get(&format!("{}.{}", parent, simple_name))
            })
            .or_else(|| variant_index.get(&simple_name))
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
            .map(|p| {
                // PARAMNAME-CARRY (v2.10): the source-verbatim declared
                // spelling wins over the lossy TypeRef re-derivation
                // (PTR sentinel → "", fn-bound expansion → shape-only)
                // — the param twin of the RETNAME-CARRY preference
                // below.  EMPTY carry (pre-2.10 bake / self param)
                // keeps the legacy derivation.
                module
                    .strings
                    .get(p.type_name)
                    .filter(|s| !s.is_empty())
                    .map(flatten_carried_param_name)
                    .or_else(|| type_ref_simple_name(&p.type_ref, module))
                    .unwrap_or_default()
            })
            .collect();

        // For each param, extract the *closure-arg return-type
        // simple-name* when the param's archive TypeRef is a
        // function type (`fn(...) -> X` — either declared directly
        // or substituted from a `F: fn(...)` generic bound during
        // stdlib precompilation).  Mirrors
        // `mod.rs::extract_closure_return_type_name` for the
        // AST-driven path.  Drives the call-site disambig push in
        // `compile_static_method_call` / `compile_call` so a
        // closure body's bare variant constructor consults the
        // right type's variant table.
        let param_closure_return_type_names: Vec<Option<String>> = fn_desc
            .params
            .iter()
            .map(|p| extract_closure_return_type_from_typeref(&p.type_ref, module))
            .collect();

        // Return-type base name + inner generics drive the variant
        // disambiguator (closes out the same code path #300 fixed
        // for source-driven compilation).
        //
        // RETNAME-CARRY-1: prefer the archive-carried source-level name
        // (verbatim AST rendering, generics intact) over the lossy
        // TypeRef re-derivation — `type_ref_simple_name` collapses the
        // PTR carrier to "USize" (integer-shaped ⇒ Int method dispatch
        // on record receivers) and drops generic args ("Maybe<Char>" →
        // "Maybe").  Legacy archives without the field fall through to
        // the old derivation.
        let return_type_name = fn_desc
            .return_type_name
            .and_then(|sid| module.strings.get(sid).map(|s| s.to_string()))
            .or_else(|| type_ref_simple_name(&fn_desc.return_type, module));
        // RETNAME-CARRY-1 oracle: VERUM_TRACE_RETNAME=<substr> prints the
        // carried-vs-derived resolution for matching function names.
        if let Ok(w) = std::env::var("VERUM_TRACE_RETNAME")
            && let Some(fname) = module.strings.get(fn_desc.name)
            && fname.contains(w.as_str())
        {
            eprintln!(
                "[retname/populate] fn='{}' carried_sid={:?} carried={:?} final={:?}",
                fname,
                fn_desc.return_type_name,
                fn_desc
                    .return_type_name
                    .and_then(|sid| module.strings.get(sid)),
                return_type_name,
            );
        }
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
        if std::env::var("VERUM_TRACE_INTRINSIC_LOAD").is_ok()
            && simple_name.contains("cbgr_alloc")
        {
            eprintln!(
                "[intrinsic-load:populate] simple='{}' intrinsic_name={:?} fn_desc.intrinsic_name_sid={:?} bytecode_len={}",
                simple_name, intrinsic_name, fn_desc.intrinsic_name, fn_desc.bytecode_length,
            );
        }
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
            // **Task #11 fix** — propagate the `&mut self` receiver
            // marker from the archived ParamDescriptor.  Pre-fix this
            // was hardcoded `false`, so the user-side
            // `compile_method_call` dispatch path at
            // `crates/verum_vbc/src/codegen/expressions.rs:~8641`
            // did NOT emit a `RefMut` to wrap the receiver — passing
            // it by VALUE — and the method body's `*self = value`
            // writeback was lost.  Universal `Maybe.take()` /
            // `Maybe.replace()` / `Maybe.insert()` / any `&mut self`
            // stdlib method had silent-no-mutation semantics through
            // every user call site.
            takes_self_mut_ref: fn_takes_self_mut_ref(fn_desc, module),
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
            param_closure_return_type_names,
        };

        // Always register qualified — `module.path.simple` —
        // unconditionally.  Cross-module dispatch path keys on this.
        //
        // Routes through `merge_module_and_simple_name` (the shared
        // canonical-name synthesiser) so the registration form
        // matches the codegen lookup form for all three precompiler-
        // assigned descriptor shapes (bare leaf, relative submodule,
        // fully-rooted submodule).  See the function-level docstring
        // for the per-shape canonical forms.
        let qualified = merge_module_and_simple_name(module_name, &simple_name);
        // BAKED-DEFAULT-ARG-1: surface the descriptor's default-value
        // channel to the call-site injector (qualified + simple keys —
        // the same spellings this FunctionInfo registers under).
        if let Some(defaults) = descriptor_param_defaults(module, fn_desc) {
            ctx.function_param_defaults
                .insert(qualified.clone(), defaults.clone());
            ctx.function_param_defaults
                .insert(simple_name.to_string(), defaults);
        }
        // T0330 mono-seed fallback: record the archive callee's raw
        // param TypeRefs under the SAME globally-unique id, so
        // `record_generic_instantiation` can derive generic type args
        // for archive-loaded callees whose descriptors are not in the
        // codegen's `self.functions` during user-fn compilation.
        ctx.archive_fn_param_types.insert(
            new_id.0,
            fn_desc.params.iter().map(|p| p.type_ref.clone()).collect(),
        );
        ctx.register_function(qualified, info.clone());
        stats.functions_registered += 1;

        // Simple name with first-wins collision discipline so a
        // bare `Some` mounted from `Maybe` doesn't get clobbered by
        // a same-named variant in a later-loaded module.  Mirrors
        // `prefer_existing_functions=true` semantics that the
        // existing stdlib-load path uses.
        //
        // For descriptors whose name is now qualified, the "simple"
        // alias is the rightmost path segment.  Strip everything up
        // to the last `.` to recover it.
        let simple_alias: String = simple_name
            .rsplit('.')
            .next()
            .unwrap_or(&simple_name)
            .to_string();
        // A METHOD must never own the bare leaf slot. `CallM` (receiver
        // syntax) is the only legal dispatch surface for impl-block methods —
        // the rule is pinned in `verum_vbc`'s `is_free_function`, which
        // filters every bare-name `Call` layer's candidates through it.
        // First-wins registration ignored the rule: whichever `X.take`
        // happened to load first squatted the bare key `take`, so under
        // `mount core.prelude.*` a call to `take(&mut v)` found the slot held
        // by a method, was correctly rejected as a non-free candidate, and
        // reported "undefined function" while `core.base.memory.take` sat
        // unreachable behind it. `swap(&mut a, &mut b)` lost the same race to
        // `Vector.swap` and silently EXECUTED it, null-dereferencing.
        //
        // Variant constructors carry a parent type as well and ARE
        // legitimately bare-callable (`Some(x)`, `Ok(v)`), so they keep the
        // slot. When only methods bear a leaf, the slot now stays empty and a
        // bare call fails loudly instead of running a foreign body.
        let claims_bare_slot = info.variant_tag.is_some() || info.parent_type_name.is_none();
        if claims_bare_slot && ctx.lookup_function(&simple_alias).is_none() {
            ctx.register_function(simple_alias, info);
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
        // Ordered generic-param names for pattern-bind payload
        // typing (ctx.type_generic_params doc) — archive twin of
        // the local `register_type_constructors` fill.
        if !ty.type_params.is_empty() {
            let params: Vec<String> = ty
                .type_params
                .iter()
                .filter_map(|tp| lookup(tp.name).map(|s| s.to_string()))
                .collect();
            if params.len() == ty.type_params.len() {
                ctx.type_generic_params
                    .entry(parent_name.clone())
                    .or_insert(params);
            }
        }
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
                            type_ref_payload_template(ty, &f.type_ref, module)
                                .unwrap_or_default()
                        })
                        .collect(),
                ),
                VariantKind::Record => (
                    variant.fields.len(),
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_payload_template(ty, &f.type_ref, module)
                                .unwrap_or_default()
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
            param_closure_return_type_names: Vec::new(),
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

    // Pass 5 — transparent-wrapper newtype constructor registration.
    //
    // For every `type X is T;` / `type X is (T);` declaration in the
    // source, `compile_type_decl` mirrors the type's structural shape
    // onto BOTH (1) the `TypeDescriptor.is_transparent_wrapper` flag
    // (archived) AND (2) a synthetic constructor `FunctionInfo` with
    // `is_transparent_wrapper: true` (NOT archived — sentinel id
    // `FunctionId(u32::MAX / 2)` means there's no body to emit).
    //
    // The archive carries (1) via the type descriptor table but
    // drops (2). On user-side load, the call site `CFd(0 as Int32)`
    // looks up `CFd` in `ctx.functions`, misses, falls through to
    // `compile_variant_constructor_hinted`'s byte-sum-hash tag
    // fallback at `expressions.rs:6419-6428` — and the result is a
    // `Variant(tag=237, payload=0)` wrapper instead of the
    // transparent Int32 value.  Downstream `CFd.0` access then
    // operates on the bogus variant, surfacing as `Variant(237, 5)`
    // when the user prints it.
    //
    // Fix: walk every loaded `TypeDescriptor` carrying
    // `is_transparent_wrapper == true`, synthesise the constructor
    // `FunctionInfo` that `compile_type_decl` would have registered
    // in-source, and ALSO populate `newtype_names` /
    // `newtype_inner_type` (the codegen-local caches that the
    // `compile_tuple_index` Mov fast-path consults).
    //
    // Skips when `ctx.functions[type_name]` is already populated —
    // this is the first-wins discipline used elsewhere in the archive
    // loader (a user-side `type CFd is ...` declaration that
    // shadows an archive transparent-wrapper takes precedence).
    use verum_vbc::types::TypeKind;
    for ty in &module.types {
        if !ty.is_transparent_wrapper {
            continue;
        }
        // Only `Record` shape — `compile_type_decl` flips the flag in
        // both the Record (`type X is T;`) and Tuple (`type X is (T);`)
        // arms but emits `TypeKind::Record` for both. Defensive: skip
        // non-Record kinds.
        if !matches!(ty.kind, TypeKind::Record) {
            continue;
        }
        let type_name = match lookup(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Skip if already registered (user shadowing or a previous
        // archive-loader pass picked it up).
        if ctx.lookup_function(&type_name).is_some() {
            // Still need to update the type-aware caches so
            // `compile_tuple_index` Mov fast-path fires for the
            // existing entry's `.0` access.
            ctx.newtype_names.insert(type_name.clone());
            if let Some(first_field) = ty.fields.first()
                && let Some(inner_name) = type_ref_simple_name(&first_field.type_ref, module)
            {
                ctx.newtype_inner_type.insert(type_name.clone(), inner_name);
            }
            continue;
        }
        // Single-field transparent wrappers (`type X is T;` or
        // single-element tuple `type X is (T);`) have exactly one
        // payload field — pin that as the constructor's only param.
        // Multi-element tuples don't flip `is_transparent_wrapper` so
        // we don't need to handle the N > 1 case here.
        let arity = ty.fields.len().max(1);
        let param_names: Vec<String> = (0..arity).map(|i| format!("_{}", i)).collect();
        let param_type_names: Vec<String> = ty
            .fields
            .iter()
            .map(|f| type_ref_simple_name(&f.type_ref, module).unwrap_or_default())
            .collect();
        let info = FunctionInfo {
            id: verum_vbc::module::FunctionId(u32::MAX / 2),
            param_count: arity,
            param_names,
            param_type_names,
            is_async: false,
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
            return_type_name: Some(type_name.clone()),
            return_type_inner: None,
            is_const: false,
            // The whole point of Pass 5 — flip this flag so the
            // call-site passthrough arms in `compile_call` /
            // `compile_method_call` fire on archive-loaded newtypes.
            is_transparent_wrapper: true,
            param_closure_return_type_names: Vec::new(),
        };
        // FUNC-REGISTRY-QUALIFICATION-1 (phase 2): the synthetic
        // newtype ctor registers under the BARE type name only —
        // ALSO mirror it under the qualified `<module>.<TypeName>`
        // key (first-wins, never replacing) so qualified consumers
        // (`resolve_function_key`'s suffix scan) can always reach
        // it.  Same canonical-name synthesis as Pass 3.
        let qualified_ctor = merge_module_and_simple_name(module_name, &type_name);
        if ctx.lookup_function(&qualified_ctor).is_none() {
            ctx.register_function(qualified_ctor, info.clone());
        }
        ctx.register_function(type_name.clone(), info);
        stats.functions_registered += 1;
        // Mirror the codegen-local newtype-tracking caches that
        // `compile_type_decl` populates in-source; these gate the
        // `Mov` fast-path in `compile_tuple_index` and the
        // float-propagation logic in `infer_expr_type_name`.
        ctx.newtype_names.insert(type_name.clone());
        if let Some(first_field) = ty.fields.first()
            && let Some(inner_name) = type_ref_simple_name(&first_field.type_ref, module)
        {
            ctx.newtype_inner_type.insert(type_name.clone(), inner_name);
        }
    }
}

/// Per-variant index entry.
#[derive(Clone)]
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
/// For an archive-loaded parameter's `TypeRef`, return the *return-type
/// simple-name* of the function shape IF the parameter is callable.
///
/// The archive serialises a function-typed parameter as
/// `TypeRef::Function { return_type, .. }` (or `Rank2Function`).  For
/// generic parameters with a `F: fn(...)` bound, the stdlib precompiler
/// emits the substituted Function type into the descriptor's param
/// type_ref, so this single check covers both `f: fn(...)` and
/// `f: F` (where F has a fn-shaped bound).
///
/// Mirrors `mod.rs::extract_closure_return_type_name` for the
/// archive-loaded path so call-site disambig works uniformly across
/// stdlib-loaded and user-defined functions.
fn extract_closure_return_type_from_typeref(
    ty: &TypeRef,
    module: &VbcModule,
) -> Option<String> {
    match ty {
        TypeRef::Function { return_type, .. } => type_ref_simple_name(return_type, module),
        TypeRef::Rank2Function { return_type, .. } => type_ref_simple_name(return_type, module),
        // Reference-wrapped function pointers (`&fn(...)`) — peek
        // through one indirection.
        TypeRef::Reference { inner, .. } => extract_closure_return_type_from_typeref(inner, module),
        _ => None,
    }
}

/// Like [`type_ref_simple_name`], with the ENCLOSING type descriptor so
/// `TypeRef::Generic(param_id)` renders the parent's declared param NAME
/// (`ControlFlow<B, C>`'s `Continue(C)` payload renders "C", not "").
/// Pattern-bind payload typing maps that template name to the param's
/// POSITION to index the scrutinee's instantiated args (#47 runtime leg)
/// — an empty template made the mapping impossible and the tag-as-index
/// heuristic swapped payload types for order-crossed sum types.
fn type_ref_payload_template(
    enclosing: &verum_vbc::types::TypeDescriptor,
    ty: &TypeRef,
    module: &VbcModule,
) -> Option<String> {
    if let TypeRef::Generic(pid) = ty {
        return enclosing
            .type_params
            .iter()
            .find(|tp| tp.id == *pid)
            .and_then(|tp| module.strings.get(tp.name).map(|s| s.to_string()));
    }
    type_ref_simple_name(ty, module)
}

/// PARAMNAME-CARRY (v2.10) → `param_type_names` normal form.  The carry
/// is FULL-fidelity ("&mut Text", "&Self") but `param_type_names`' one
/// consumer — the call-site type-aware disambiguator — compares against
/// `extract_expr_type_name` shapes, which flatten the top-level
/// reference (the same dispatch contract `extract_type_name_from_ast`
/// and `type_ref_simple_name` follow).  Strip ONE leading ref sigil so
/// carried spellings keep matching where the legacy derivation matched.
fn flatten_carried_param_name(s: &str) -> String {
    let mut t = s.trim();
    for p in [
        "&checked mut ",
        "&unsafe mut ",
        "&checked ",
        "&unsafe ",
        "&mut ",
        "&",
    ] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest.trim_start();
            break;
        }
    }
    // Base-only, like `type_ref_simple_name`'s Instantiated arm — the
    // disambiguator's other producers emit "List", not "List<Int>".
    match t.find('<') {
        Some(i) => t[..i].to_string(),
        None => t.to_string(),
    }
}

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
        // **Task #20 §B — cross-module well-known generic carriers**.
        //
        // Variant/container TypeIds (`Maybe`, `Result`, `List`, `Map`,
        // `Set`, `Deque`, `Channel`, `Range`, `Array`, `Heap`, `Shared`,
        // `Tuple`, `Pi`, `Sigma`, `Witness`) are reserved in
        // `verum_vbc::types::TypeId` but live in stdlib modules whose
        // type descriptors are NOT present in EVERY consuming module's
        // `module.types` list (cross-module return-type leakage).
        //
        // Pre-fix `type_ref_simple_name` returned `None` whenever a
        // function's return type was `Result<X, Y>` and the calling
        // module didn't directly import `Result`'s type descriptor —
        // even though `return_type_inner` correctly carried
        // `["X", "Y"]`.  The downstream
        // `extract_expr_type_name` couldn't form `"Result<X, Y>"`,
        // `compile_match` lost the scrutinee type, and the pattern
        // binder fell through to the global field-intern fallback,
        // surfacing as "field access out of bounds: field index N"
        // at every `match parse_X(...) { Ok(v) => v.field }` site.
        //
        // Recognising these TypeIds directly here keeps the cross-module
        // identity invariant: a Result is a Result regardless of which
        // module's perspective we view it from.
        TypeId::MAYBE => "Maybe",
        TypeId::RESULT => "Result",
        TypeId::LIST => "List",
        TypeId::MAP => "Map",
        TypeId::SET => "Set",
        TypeId::DEQUE => "Deque",
        TypeId::CHANNEL => "Channel",
        TypeId::RANGE => "Range",
        TypeId::ARRAY => "Array",
        TypeId::HEAP => "Heap",
        TypeId::SHARED => "Shared",
        TypeId::TUPLE => "Tuple",
        TypeId::PI => "Pi",
        TypeId::SIGMA => "Sigma",
        TypeId::WITNESS => "Witness",
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
            // Render each argument with its FULL nested generic form —
            // `Result<List<ResolvedRange>, RangeError>` yields
            // `["List<ResolvedRange>", "RangeError"]`, not the
            // generic-stripped `["List", "RangeError"]`.
            //
            // The variant disambiguator that consumes this
            // (`find_variant_by_suffix_and_args` /
            // `find_function_by_suffix`) already strips generics before
            // comparing to parent names (`inner_name.split('<').next()`),
            // so the nested form is transparent to it. What it ENABLES:
            // `let m = free_fn().unwrap()` where the free fn returns
            // `Result<List<T>, E>` now records `m` as `List<T>` (not bare
            // `List`), so `m[i].field` recovers element type `T` and
            // resolves the field offset from `T`'s descriptor instead of
            // falling to the global field-name interner (the cross-module
            // `collection[i].field` out-of-bounds defect surfaced by the
            // http_range / link_header property suites).
            let names: Vec<String> = args
                .iter()
                .map(|a| type_ref_full_name(a, module).unwrap_or_default())
                .collect();
            Some(names)
        }
        _ => None,
    }
}

/// Render a `TypeRef` to its full nested generic form, e.g.
/// `List<ResolvedRange>` / `Map<Text, List<Cidr>>`. Unlike
/// [`type_ref_simple_name`] (which returns only the base nominal name),
/// this preserves instantiation arguments recursively so downstream
/// element-type extraction (`arr[i]` → element type) survives across the
/// archive boundary. References render as their pointee's full name to
/// match the simple-name convention the disambiguator expects.
fn type_ref_full_name(ty: &TypeRef, module: &VbcModule) -> Option<String> {
    match ty {
        TypeRef::Instantiated { base, args } => {
            let base_name = if let Some(name) = primitive_typeid_name(*base) {
                name.to_string()
            } else {
                module
                    .types
                    .iter()
                    .find(|t| t.id == *base)
                    .and_then(|t| module.strings.get(t.name).map(|s| s.to_string()))?
            };
            if args.is_empty() {
                return Some(base_name);
            }
            let rendered: Vec<String> = args
                .iter()
                .map(|a| type_ref_full_name(a, module).unwrap_or_else(|| "_".to_string()))
                .collect();
            Some(format!("{}<{}>", base_name, rendered.join(", ")))
        }
        TypeRef::Reference { inner, .. } => type_ref_full_name(inner, module),
        _ => type_ref_simple_name(ty, module),
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
            // Module-form mount surface: `mount core.sys.bitfield;`
            // adds the literal `core.sys.bitfield` qualified name to
            // `wanted` (via `collect_mount_names`'s `full.join(".")`
            // arm).  The user's intent is "load the bitfield module
            // wholesale so `bitfield.<NAME>` resolves through the
            // codegen-side suffix-match"; without including the
            // dotted name itself in the prefix set, the
            // `wanted_module_prefixes.contains(&entry.name)` gate at
            // the archive walk loop misses the matching archive entry
            // (`core.sys.bitfield`) entirely — its functions never
            // register, and `bitfield.USIZE_BITS` falls through every
            // suffix-match probe at the call site because the registry
            // never received a `core.sys.bitfield.USIZE_BITS` key.
            //
            // Adding the dotted name itself is harmless for name-form
            // mounts: `mount core.sys.bitfield.{USIZE_BITS}` adds
            // `core.sys.bitfield.USIZE_BITS` to `wanted`; including
            // it in `prefixes` is a no-op (no archive entry has that
            // exact name — only `core.sys.bitfield`), and the ancestor
            // walk below still adds the right module-level entry.
            //
            // Closes task #121 stage 2 — the precompiler-side and
            // archive-loader-side were already registering qualified
            // names; the gap was in the wanted-prefix expansion that
            // gated whether the entry got loaded at all.
            if name.contains('.') {
                prefixes.push(name.clone());
            }
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

/// Cross-module dependency graph derived from archive bytecode.
///
/// Built **once** per archive (cached on `ArchiveCtxCache`) by decoding
/// every module and harvesting `Call`/`TailCall` (local) +
/// `CallM` (cross-module) call edges. Reachability BFS from user-source
/// seeds replaces the prior architecture's hardcoded force-load table
/// + 5 heuristic filter arms in `register_module_filtered`: every
/// reachable function is registered; non-reachable stays unloaded.
///
/// # Why upfront full decode is acceptable
///
/// * Cost: ~250ms first call on a 12 MB archive (rayon-parallel decode).
/// * Amortised across the process via `OnceLock` — second+ compilations
///   in the same process pay zero.
/// * Correctness vs. cost tradeoff: the prior heuristic filter
///   periodically dropped legitimate cross-module dependencies (tasks
///   #23 / #24 / #26) producing silent runtime `nil`s — the architectural
///   loss outweighs the cold-start cost.
pub(crate) struct SymbolGraph {
    /// The whole index, in the baked byte layout — see
    /// [`crate::symbol_graph_baked`].  ONE representation, whether the
    /// bytes came from the embedded sidecar (no work at start-up) or
    /// from scanning an archive (the fallback below).  Keeping a single
    /// reader is what stops the two paths drifting: a bug reachable
    /// through one is reachable through the other.
    baked: crate::symbol_graph_baked::BakedSymbolGraph,
}

impl SymbolGraph {
    /// Read the graph the bake wrote.  `None` when this build embeds
    /// no sidecar or the bytes are not readable by this format
    /// version — every caller then falls back to [`Self::build`], so a
    /// rejected sidecar costs start-up time, never correctness.
    fn from_embedded() -> Option<Self> {
        // A/B FROM ONE BINARY.  Comparing two BUILDS lets every
        // difference be blamed on the build; this switch makes the
        // baked and the scanned graph comparable within a single
        // binary, which is what the differential corpus run needs to
        // mean anything.
        if std::env::var_os("VERUM_NO_BAKED_SYMBOL_GRAPH").is_some() {
            return None;
        }
        let bytes = crate::embedded_symbol_graph::embedded_bytes()?;
        crate::symbol_graph_baked::BakedSymbolGraph::from_bytes(std::borrow::Cow::Borrowed(
            bytes,
        ))
        .map(|baked| Self { baked })
    }

    /// Scan an archive and encode the result — the fallback, and the
    /// producer the bake itself calls.  Decodes every archive module in
    /// parallel and disassembles each function body: pure CPU work over
    /// immutable archive bytes, perfectly parallelisable, and the
    /// several hundred milliseconds this file exists to stop paying at
    /// every compiler start.
    fn build(archive: &VbcArchive) -> Self {
        Self {
            baked: Self::scan_and_encode(archive),
        }
    }

    /// The scan, kept separate so the bake can call it to PRODUCE the
    /// sidecar without constructing a graph it will not use.
    pub(crate) fn scan_and_encode(
        archive: &VbcArchive,
    ) -> crate::symbol_graph_baked::BakedSymbolGraph {
        use crate::symbol_graph_baked::{BakedSymbolGraph, EncodedFunction};
        use rayon::prelude::*;

        let per_module: Vec<(u32, ModuleSymbolView)> = (0..archive.index.len())
            .into_par_iter()
            .filter_map(|idx| {
                let module = archive.load_module_by_index(idx).ok()?;
                let view = scan_module_symbols(&module);
                Some((idx as u32, view))
            })
            .collect();

        let entries: Vec<String> =
            archive.index.iter().map(|e| e.name.clone()).collect();

        // Rows in DISCOVERY order; the encoder keeps the first row for
        // a repeated name, which is `register_function`'s first-wins
        // rule and the rule the previous `HashMap::entry().or_insert()`
        // form implemented.
        let mut funcs: Vec<EncodedFunction> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut leaf_index: HashMap<String, Vec<String>> = HashMap::new();
        let mut prefix_index: HashMap<String, Vec<String>> = HashMap::new();

        for (idx, view) in per_module {
            let entry_name: &str = archive
                .index
                .get(idx as usize)
                .map(|e| e.name.as_str())
                .unwrap_or("");
            for ModuleFunction { name, callees } in view.functions {
                // **Spelling completeness** (T0277 leg B part 2): a
                // caller's edge records the callee under the spelling
                // THE CALLER KNEW — commonly the fully-ROOTED canonical
                // form (`core.mem.epoch.GLOBAL_EPOCH.on_wraparound`)
                // while the descriptor stores the promoted/relative
                // form (`mem.epoch.GLOBAL_EPOCH.on_wraparound`). The
                // keep-set indexer (`compute_merge_keep_sets`) already
                // indexes both; the BFS graph indexed ONLY the raw
                // descriptor spelling, so rooted edges fell off the
                // graph and the callee's module was never decoded.
                // Register the canonical spelling as a first-class
                // node: same module, SAME edge row so the BFS can
                // continue THROUGH the callee's own edges.
                let canonical = merge_module_and_simple_name(entry_name, &name);
                if canonical.as_str() != name && seen.insert(canonical.clone()) {
                    funcs.push(EncodedFunction {
                        name: canonical,
                        module: idx,
                        callees: callees.clone(),
                    });
                }
                // The leaf and prefix indexes carry the DESCRIPTOR
                // spelling only.  Indexing the canonical alias too
                // would widen every bare-leaf fanout by the alias set
                // — the over-approximation this loader spends its
                // architecture avoiding.
                if let Some(leaf) = name.rsplit('.').next()
                    && leaf != name
                {
                    leaf_index
                        .entry(leaf.to_string())
                        .or_default()
                        .push(name.clone());
                }
                if let Some(prefix) = name.split('.').next()
                    && prefix != name
                {
                    prefix_index
                        .entry(prefix.to_string())
                        .or_default()
                        .push(name.clone());
                }
                if seen.insert(name.clone()) {
                    funcs.push(EncodedFunction {
                        name,
                        module: idx,
                        callees,
                    });
                }
            }
        }

        let leaf_rows: Vec<(String, Vec<String>)> = leaf_index.into_iter().collect();
        let prefix_rows: Vec<(String, Vec<String>)> = prefix_index.into_iter().collect();
        BakedSymbolGraph::from_parts(&entries, &funcs, &leaf_rows, &prefix_rows)
    }

    /// True when SOME archive symbol is spelled exactly `name`.
    ///
    /// Guards the whole-archive simple-name scan (T0738): a name no
    /// symbol carries cannot be found by that scan, so the whole decode
    /// is waste.  Measured: the AST name harvest puts the user's LOCAL
    /// VARIABLE names into that set — `let v: Int = 10; print(v);`
    /// shipped `v`, and the search for a stdlib function called `v`
    /// took the compiled module from 12604 functions to 66797 and the
    /// build from 3.3 s to 26.2 s.
    pub(crate) fn carries_simple_name(&self, name: &str) -> bool {
        self.baked.carries_simple_name(name)
    }

    /// BFS from seed names. Returns:
    /// * `reached_qualified`: every qualified function name reachable
    ///   from the seeds via the call graph.
    /// * `reached_modules`: archive entry indices containing at least
    ///   one reached qualified function. Drives module-level decoding.
    ///
    /// The walk carries FUNCTION INDICES, not names: an index is the
    /// row number in the baked table, so membership is a bitset-shaped
    /// `HashSet<u32>` and the per-step name allocation the previous
    /// `HashSet<String>` form paid is gone.  Names are materialised
    /// once, at the end, for the caller.
    /// Does the archive carry a symbol with this exact name?
    ///
    /// Used to ask "does this type have a `Display` impl" before
    /// pulling it into the closure: reaching for a `<Type>.fmt` that
    /// does not exist costs a BFS seed that can never match.
    pub(crate) fn has_symbol(&self, name: &str) -> bool {
        self.baked.function_index(name).is_some()
    }

    pub(crate) fn reachable(
        &self,
        seeds: &HashSet<String>,
        bare_method_seeds: &HashSet<String>,
    ) -> (HashSet<String>, HashSet<u32>) {
        let mut reached: HashSet<u32> = HashSet::new();
        let mut modules: HashSet<u32> = HashSet::new();
        let mut queue: VecDeque<u32> = VecDeque::new();

        // REACHABILITY PROVENANCE (T0753).  The closure for a program
        // that pushes two Ints onto a `List` reaches 188 archive
        // entries — TLS 1.3 handshake, QUIC, Redis, x509 among them —
        // and the fanout cap barely moves that number, so the pull is
        // through EXACT edges.  Which edges is not derivable from the
        // totals: only the chain names the bridge.  `VERUM_TRACE_REACH`
        // records one parent per reached function (the edge that FIRST
        // reached it — the shortest path, since this is a BFS) and
        // prints the chain from a seed to the first name matching
        // `VERUM_TRACE_REACH_PATH`, plus the per-entry symbol counts.
        //
        // `None` marks a seed.  Off by default.
        let trace_reach = std::env::var_os("VERUM_TRACE_REACH").is_some()
            || std::env::var_os("VERUM_TRACE_REACH_PATH").is_some();
        let mut parent: HashMap<u32, Option<u32>> = HashMap::new();

        macro_rules! enqueue {
            ($idx:expr, $via:expr) => {{
                let i: u32 = $idx;
                if reached.insert(i) {
                    if trace_reach {
                        parent.insert(i, $via);
                    }
                    queue.push_back(i);
                }
            }};
        }

        let max_bare_leaf_fanout: usize = std::env::var("VERUM_LEAF_FANOUT_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        // Restores the pre-T0753 seed expansion for A/B from ONE
        // binary.  It only ever widens the closure, so it cannot break
        // a program, only slow it.
        let no_seed_method_cap = std::env::var_os("VERUM_NO_SEED_METHOD_CAP").is_some();

        // Seed expansion: a seed can be (1) an exact qualified
        // descriptor name, (2) a bare leaf shared by multiple
        // qualifieds, or (3) a bare type prefix. Walk all three.
        let mut live_types: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut capped_leaves: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // Fixed-point cold start: literals construct these types with
        // no name the harvest could see (`[1,2,3]` is a List, `0..n` a
        // Range, `"s"` a Text, `map{}`/`set{}` their containers,
        // Some/Ok their sums), so they are live BEFORE any function is
        // reached.  Without this, a chain built purely from capped
        // bare methods (`[..].iter().map(..).next()`) had an empty
        // queue, no `type:` edge ever fired, and the pairing never
        // started.  Pairs only form against capped leaves, so a
        // program with none (hello-world) pays nothing.
        for t in [
            "List", "Text", "Range", "RangeInclusive", "Maybe", "Result",
            "Map", "Set",
        ] {
            live_types.insert(t.to_string());
        }
        for seed in seeds {
            if let Some(i) = self.baked.function_index(seed) {
                enqueue!(i, None);
            }
            // BARE-METHOD SEED CAP (T0753).  A seed that came from a
            // bare method call (`xs.new()` -> `new`) expands, through
            // the leaf index, to every same-named impl in the library
            // — and the walk then follows each one's call graph.  That
            // is the same over-approximation the fanout cap below
            // rejects INSIDE the walk, on the same grounds: a `CallM`
            // resolves against the receiver's concrete runtime type,
            // and that type's module is reached independently (its
            // constructor, a qualified edge, or the qualified
            // `List.new` twin the harvest emits alongside the bare
            // name).  Only the seed expansion never applied it.
            //
            // Names the user wrote as functions are NOT affected: they
            // are not bare-method seeds, so `abs(-3)` still fans its
            // leaf however many `*.abs` exist.
            let capped = !no_seed_method_cap
                && bare_method_seeds.contains(seed.as_str())
                && self.baked.leaf_match_count(seed) > max_bare_leaf_fanout;
            if capped {
                // T0701: a capped SCRIPT seed (bare `.dedup()` in user
                // code) joins the pairing set — live `type:` carriers
                // discovered during the walk pair against it exactly
                // like capped graph-edge leaves, and it pairs
                // IMMEDIATELY against the pre-seeded literal types.
                if capped_leaves.insert(seed.clone()) {
                    let pairs: Vec<u32> = live_types
                        .iter()
                        .filter_map(|t| {
                            self.baked.function_index(&format!("{t}.{seed}"))
                        })
                        .collect();
                    for i in pairs {
                        enqueue!(i, None);
                    }
                }
            }
            if !capped
                && let Some(matches) = self.baked.leaf_matches(seed)
            {
                let ids: Vec<u32> = matches.collect();
                for i in ids {
                    enqueue!(i, None);
                }
            }
            if let Some(matches) = self.baked.prefix_matches(seed) {
                let ids: Vec<u32> = matches.collect();
                for i in ids {
                    enqueue!(i, None);
                }
            }
        }

        // **Polymorphic-method fanout cap** (closes the iterator
        // archive-load blow-up). A bare protocol-method callee such as
        // `next` / `map` / `clone` / `eq` resolves, in the archive's
        // leaf index, to EVERY type's same-named impl — 172 distinct
        // `*.next` bodies, each of which calls `self.iter.next()`
        // (another bare `next` CallM edge) and so re-fans to all 172
        // transitively. The naive closure therefore pulls in nearly the
        // whole archive (585 modules decoded), and the lazy-apply took
        // ~85s for any user code that merely calls a method named
        // `next`.
        //
        // The fix is grounded in the runtime dispatch model: a bare
        // method call (`CallM`) is resolved by the RECEIVER's concrete
        // runtime type (see `method_dispatch::func_id_parent_compatible_
        // with_receiver`), and that concrete type's defining module is
        // ALWAYS reached independently — the type is constructed/used,
        // so its constructor or a qualified edge pulls its module in,
        // and `register_module_filtered` loads all of the module's impl
        // methods. Blanket-fanning a bare leaf to every same-named impl
        // is therefore redundant for correctness and catastrophic for
        // cost. We cap the per-callee bare-leaf expansion: leaves that
        // map to more than `MAX_BARE_LEAF_FANOUT` qualifieds are treated
        // as polymorphic protocol methods and NOT fanned out here —
        // their needed impl arrives via the receiver type's module.
        // Low-fanout leaves (e.g. `Text.from_utf8_unchecked`, a unique
        // helper) keep their precise resolution.
        // Complement of the cap above (T0753): the cap bounds the damage
        // when a callee can ONLY be named by its leaf, but a callee that
        // resolves EXACTLY needs no fanout at all.  Read once — the check
        // sits on the hot edge loop below, and `std::env` takes a lock.
        // Setting this restores the pre-T0753 walker for A/B from ONE
        // binary; it only ever widens the closure, so it cannot break a
        // program, only slow it.
        let no_exact_shortcut = std::env::var_os("VERUM_NO_EXACT_SHORTCUT").is_some();

        // T0701 (adapter-method reachability): the bare-leaf cap's own
        // grounding says "the receiver type's module is always reached
        // … and register_module_filtered loads its impl methods" — but
        // that register step is wanted-FILTERED, so a method reached
        // ONLY as a capped bare leaf on a type reached ONLY as a
        // return value (`.map()…​.dedup()`) was never loaded and died
        // "method not found" at runtime.  Close exactly that
        // intersection: `live_types` collects the `type:` return-carry
        // marker edges (scan_module_symbols), `capped_leaves` collects
        // the bare leaves the cap refused to fan, and every (T, m)
        // pair with an exact `T.m` row is enqueued.  Bounded by
        // |live types| × |capped leaves| O(1) lookups — no prefix
        // fanout, so the hello-world closure stays untouched
        // (measured: a naive `T.*` prefix fan here ballooned cold
        // start 0.5 s → 36 s).

        macro_rules! fan_leaf {
            ($leaf:expr, $via:expr) => {{
                let leaf: &str = $leaf;
                if self.baked.leaf_match_count(leaf) <= max_bare_leaf_fanout {
                    if let Some(matches) = self.baked.leaf_matches(leaf) {
                        let ids: Vec<u32> = matches.collect();
                        for i in ids {
                            enqueue!(i, $via);
                        }
                    }
                } else if !leaf.contains('.') && capped_leaves.insert(leaf.to_string()) {
                    // New capped leaf: pair it against every live type.
                    let pairs: Vec<u32> = live_types
                        .iter()
                        .filter_map(|t| {
                            self.baked.function_index(&format!("{t}.{leaf}"))
                        })
                        .collect();
                    for i in pairs {
                        enqueue!(i, $via);
                    }
                }
            }};
        }

        while let Some(fidx) = queue.pop_front() {
            modules.insert(self.baked.module_of_index(fidx));
            let callees: Vec<&str> = self.baked.callees(fidx).collect();
            for callee in callees {
                // T0701: `type:T` marker edge (see scan_module_symbols)
                // — the function RETURNS a T; pair the newly-live type
                // against every capped bare leaf seen so far (and
                // future leaves pair against it in fan_leaf!).
                if let Some(tbase) = callee.strip_prefix("type:") {
                    if live_types.insert(tbase.to_string()) {
                        let pairs: Vec<u32> = capped_leaves
                            .iter()
                            .filter_map(|m| {
                                self.baked
                                    .function_index(&format!("{tbase}.{m}"))
                            })
                            .collect();
                        for i in pairs {
                            enqueue!(i, Some(fidx));
                        }
                    }
                    continue;
                }
                // Direct qualified resolution — always exact, never
                // fans, so it stays unconditional.
                //
                // RESOLVED-EXACTLY SHORT-CIRCUIT (T0753).  When this
                // hits, the callee IS the callee: a descriptor row
                // exists under that exact name and has just been
                // enqueued. The leaf fanout below then stripped its
                // type prefix and re-enqueued every same-named impl in
                // the library — pure over-approximation on top of an
                // exact answer.
                //
                // Measured: that strip is why qualifying the EMISSION
                // sites changed nothing. Two rounds of it (c397a56e7,
                // 9494b63fe) left the closure at 5502 symbols / 255
                // modules, because the walker discarded the qualifier
                // one line later.
                //
                // A/B from one binary, `Maybe` + `List` probe: 6717 ->
                // 2612 symbols, 307 -> 181 modules, 10.39 s -> 2.40 s
                // of lazy-apply.  40 L1-core programs: identical stdout
                // and exit status, and the field-guess candidate sets
                // shrink with the closure (T0723's worst site, `name`,
                // 489 -> 314 position-disagreeing candidates).
                if let Some(cidx) = self.baked.function_index(callee) {
                    enqueue!(cidx, Some(fidx));
                    if !no_exact_shortcut {
                        continue;
                    }
                } else if callee.contains('.') {
                    // **Home-module decode edge** (T0277 leg B part 2):
                    // a dotted callee with NO descriptor row anywhere
                    // is a by-name reference to something that is not a
                    // function-table entry — a variant constructor, an
                    // FFI extern, or a re-export spelling. It cannot be
                    // walked further, but its HOME module must still
                    // join the decode set so the constructor/extern
                    // REGISTERS and merge-time band resolution binds
                    // the name instead of leaving a dangling band id
                    // (const-zero stub → SIGBUS at AOT;
                    // `[xmod-unresolved]` at Tier-0).  Longest-prefix,
                    // one module, no fanout.
                    if let Some(home_idx) = self.baked.home_module_of(callee) {
                        modules.insert(home_idx);
                    }
                }
                // CallM frequently emits `Type.method`-form strings
                // whose receiver type prefix isn't a module path —
                // `Text.from_utf8_unchecked` resolves via the leaf
                // index.
                fan_leaf!(callee, Some(fidx));
                // For descriptor-name-promoted forms like
                // `sys.bitfield.test_bit` whose leaf is `test_bit`,
                // also try matching the full callee against
                // `Type.method` forms ending in this string by
                // stripping the type prefix.
                if let Some(dot_pos) = callee.find('.') {
                    fan_leaf!(&callee[dot_pos + 1..], Some(fidx));
                }
            }
        }

        if trace_reach {
            self.report_reach(&reached, &modules, &parent);
        }
        let names: HashSet<String> = reached
            .iter()
            .map(|i| self.baked.function_name(*i).to_string())
            .collect();
        (names, modules)
    }

    /// Print WHY the closure is the size it is (T0753).
    ///
    /// Two readings, because the totals hide different things:
    ///
    ///  * per-entry symbol counts — which archive entries the closure
    ///    actually lands in, biggest first.  An entry with three
    ///    reached symbols still costs a whole bundle decode, so the
    ///    long tail is as expensive as the head.
    ///  * one chain — `VERUM_TRACE_REACH_PATH=<substring>` walks the
    ///    parent map back from the first reached name matching the
    ///    substring to its seed.  That chain names the BRIDGE edge,
    ///    which is the only thing that says whether the pull is a real
    ///    dependency or an over-approximation.
    fn report_reach(
        &self,
        reached: &HashSet<u32>,
        modules: &HashSet<u32>,
        parent: &HashMap<u32, Option<u32>>,
    ) {
        let mut per_entry: HashMap<u32, usize> = HashMap::new();
        for f in reached {
            *per_entry.entry(self.baked.module_of_index(*f)).or_insert(0) += 1;
        }
        let mut rows: Vec<(u32, usize)> = per_entry.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        eprintln!(
            "[reach] {} symbols in {} entries",
            reached.len(),
            modules.len()
        );
        for (idx, n) in rows.iter().take(30) {
            eprintln!("[reach]   {:<34} {:>6}", self.baked.entry_name(*idx), n);
        }
        if rows.len() > 30 {
            let tail: usize = rows.iter().skip(30).map(|(_, n)| *n).sum();
            eprintln!(
                "[reach]   {:<34} {:>6}  (in {} further entries)",
                "(tail)",
                tail,
                rows.len() - 30
            );
        }
        let Ok(needle) = std::env::var("VERUM_TRACE_REACH_PATH") else {
            return;
        };
        let mut hits: Vec<u32> = reached
            .iter()
            .copied()
            .filter(|i| self.baked.function_name(*i).contains(needle.as_str()))
            .collect();
        hits.sort_by_key(|i| self.baked.function_name(*i));
        let Some(target) = hits.first().copied() else {
            eprintln!("[reach] no reached symbol contains {:?}", needle);
            return;
        };
        eprintln!(
            "[reach] chain to {:?} ({} reached symbols match):",
            self.baked.function_name(target),
            hits.len()
        );
        let mut chain: Vec<u32> = vec![target];
        let mut cur = target;
        while let Some(Some(via)) = parent.get(&cur).copied() {
            chain.push(via);
            cur = via;
            if chain.len() > 64 {
                break;
            }
        }
        for (i, step) in chain.iter().rev().enumerate() {
            eprintln!(
                "[reach]   {:>2}. {}   [{}]",
                i,
                self.baked.function_name(*step),
                self.baked.entry_name(self.baked.module_of_index(*step))
            );
        }
    }

    /// Returns the archive entry name that defines `qualified_name`,
    /// if any. Used by the type-import side to find the canonical
    /// type-bearing module.
    #[allow(dead_code)]
    pub(crate) fn defining_entry<'a>(
        &self,
        qualified_name: &str,
        archive: &'a VbcArchive,
    ) -> Option<&'a str> {
        let idx = self.baked.module_of(qualified_name)? as usize;
        archive.index.get(idx).map(|e| e.name.as_str())
    }
}

/// Per-function summary for graph construction.
struct ModuleFunction {
    name: String,
    callees: Vec<String>,
}

struct ModuleSymbolView {
    functions: Vec<ModuleFunction>,
}

/// Decode each function's bytecode and harvest its call edges.
///
/// `Call`/`TailCall` resolve via two id tables — the module's local
/// function table (intra-module calls, renamed to contiguous 0..N at
/// archive build time) AND the cross-module `external_function_names`
/// side table (cross-module calls, preserved at their precompile-time
/// codegen-global ids). Without the cross-module table, transitive
/// reachability from user seeds stopped at module boundaries — e.g.
/// the user mentioning `Text.push_byte` would never pull in
/// `core.base.memory.alloc` (called from `Text.grow`'s body), the
/// loader would not load `core.base.memory`, and `alloc` would not
/// appear in the user codegen's `ctx_func_by_name`. The live failure
/// mode: `ArchiveBodyRemap::map_function`'s Tier-2 name fallback
/// fires for `core.base.memory.alloc`, looks it up in `ctx_func_by_name`,
/// misses, falls to Tier-3 identity → user bytecode keeps the bogus
/// precompile-time id → runtime dispatch routes to whatever lives
/// at that index (originally `Successors.next` until the post-merge
/// rebuild rotated the slot to `Text.char_count`).
/// `CallM` resolves via the module's string table; the resulting
/// method-name string is the cross-module dispatch key.
fn scan_module_symbols(module: &VbcModule) -> ModuleSymbolView {
    let name_by_id: HashMap<StringId, String> = module
        .strings
        .iter()
        .map(|(s, id)| (id, s.to_string()))
        .collect();
    // Union of local function ids and cross-module external ids,
    // mapped to qualified callee names. Local entries win on key
    // collision (impossible in practice — local ids are contiguous
    // 0..N while external ids retain their precompile-time sparse
    // values well above N — but the explicit precedence pins
    // intent).
    let mut id_to_name: HashMap<u32, String> = module
        .functions
        .iter()
        .filter_map(|f| name_by_id.get(&f.name).map(|n| (f.id.0, n.clone())))
        .collect();
    for (fid, sid) in module.external_function_names.iter() {
        id_to_name.entry(fid.0).or_insert_with(|| {
            name_by_id
                .get(sid)
                .cloned()
                .unwrap_or_default()
        });
    }
    let mut functions = Vec::with_capacity(module.functions.len());
    for fn_desc in &module.functions {
        let name = match name_by_id.get(&fn_desc.name) {
            Some(n) => n.clone(),
            None => continue,
        };
        let mut callees: Vec<String> = Vec::new();
        // T0701 (adapter-method reachability): record the RETURN type's
        // base as a `type:` marker edge.  The bare-leaf fanout cap is
        // grounded in "the receiver type's defining module is always
        // reached, and its impl methods register with it" — but the
        // register step is wanted-FILTERED, and a type reached only
        // through a return value (`.map()` → MappedIter) never put its
        // methods in `wanted`, so `.dedup()` on the adapter chain died
        // "method not found" at runtime while the body sat in the
        // decoded module.  The walker turns this edge into a
        // `T.`-prefix fan (one type's methods — bounded), closing the
        // ctor-invisibility gap without re-opening the 585-module
        // blow-up the cap exists to prevent.
        if let Some(ret_sid) = fn_desc.return_type_name
            && let Some(ret_raw) = name_by_id.get(&ret_sid)
        {
            let base = ret_raw
                .trim_start_matches('&')
                .trim_start_matches("mut ")
                .trim_start_matches("unsafe ")
                .trim_start_matches("checked ")
                .split('<')
                .next()
                .unwrap_or("")
                .trim();
            if !base.is_empty()
                && base
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                && base.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && !verum_common::well_known_types::type_names::is_primitive_value_type(base)
            {
                callees.push(format!("type:{base}"));
            }
        }
        let body_start = fn_desc.bytecode_offset as usize;
        let body_end = body_start.saturating_add(fn_desc.bytecode_length as usize);
        if body_end <= module.bytecode.len() && body_end > body_start {
            let body = &module.bytecode[body_start..body_end];
            if let Ok(instructions) = verum_vbc::bytecode::decode_instructions(body) {
                for instr in &instructions {
                    match instr {
                        // UMBRELLA-MOUNT-PRUNE-1: every fn-id-carrying
                        // operand is a call edge for reachability
                        // purposes — a function whose id is embedded in
                        // a closure/spawn/generator/generic-call
                        // instruction can execute at runtime exactly
                        // like a direct Call target. Pre-fix only
                        // Call/TailCall were harvested, so closure
                        // bodies (`NewClosure`), spawned tasks
                        // (`Spawn`), generator bodies (`GenCreate`)
                        // and generic targets (`CallG`) were invisible
                        // to the BFS and survived only via the
                        // merge-all safety net that the merge pruner
                        // removes.
                        Instruction::Call { func_id, .. }
                        | Instruction::TailCall { func_id, .. }
                        | Instruction::NewClosure { func_id, .. }
                        | Instruction::CallG { func_id, .. }
                        | Instruction::GenCreate { func_id, .. }
                        | Instruction::Spawn { func_id, .. } => {
                            if let Some(callee) = id_to_name.get(func_id) {
                                callees.push(callee.clone());
                            }
                        }
                        Instruction::CallM { method_id, .. } => {
                            if let Some(callee) = name_by_id.get(&StringId(*method_id)) {
                                callees.push(callee.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        functions.push(ModuleFunction { name, callees });
    }
    ModuleSymbolView { functions }
}

pub struct ArchiveCtxCache {
    /// One-shot lazily-built table: qualified name → FunctionInfo.
    /// Holds both qualified (`module.simple`) and simple-name keys
    /// after first build.
    table: OnceLock<HashMap<String, FunctionInfo>>,
    /// Archive-wide call-graph index. Built lazily on first
    /// `apply_lazy_with_types` call; subsequent compilations within
    /// the process reuse the cached graph (~free).
    graph: OnceLock<SymbolGraph>,
    /// T0706 — per-archive-entry set of archive-local function ids whose
    /// BODIES were already merged into a user codegen this compilation.
    /// The supplemental post-typecheck pass consults this so a second
    /// `merge_archive_function_bodies` call over the same entry never
    /// re-merges a body ([[duplicate-emitter]] class: emission order
    /// wins, a duplicate is a latent misdispatch).  Keyed per
    /// compilation epoch — `begin_compilation_epoch` clears it, because
    /// the cache outlives compilations (REPL / watch / test-runner) but
    /// merged-ness is a property of ONE codegen instance.
    merged_ids: std::sync::Mutex<std::collections::BTreeMap<String, std::collections::HashSet<u32>>>,
}

impl ArchiveCtxCache {
    /// Construct an empty cache.  Cheap; no archive work happens here.
    pub const fn new() -> Self {
        Self {
            table: OnceLock::new(),
            graph: OnceLock::new(),
            merged_ids: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// T0706 — reset per-compilation merge bookkeeping.  Call at the
    /// start of every `apply_lazy_with_types` (each user compilation
    /// constructs a fresh codegen; carried merged-ids from a previous
    /// compilation would wrongly suppress merges into the new one).
    fn begin_compilation_epoch(&self) {
        self.merged_ids
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    /// T0706 — record which archive-local ids of `entry` had bodies
    /// merged this epoch.
    fn note_merged_ids<'a>(
        &self,
        entry: &str,
        ids: impl Iterator<Item = &'a u32>,
    ) {
        let mut guard = self
            .merged_ids
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard
            .entry(entry.to_string())
            .or_default()
            .extend(ids.copied());
    }

    /// Lazily build the per-archive symbol graph (reachability index
    /// from `CallM` / `Call` / `TailCall` edges). Cached for the
    /// process lifetime — first call pays the full archive decode
    /// (~250ms on a 12 MB archive), every later call is free.
    pub(crate) fn graph(&self, archive: &VbcArchive) -> &SymbolGraph {
        self.graph
            .get_or_init(|| {
                loadcost::timed("symbol_graph_load", || {
                    // The sidecar is the fast path AND the default; the
                    // scan below is what a compiler built without a bake
                    // still has to do, so both must stay exercised.
                    SymbolGraph::from_embedded()
                        .unwrap_or_else(|| SymbolGraph::build(archive))
                })
            })
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
        let mut harvest = Harvest::default();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut harvest);
        }
        let mut wanted: std::collections::HashSet<String> =
            std::mem::take(&mut harvest.names);
        // TEXT-DEBUG-STATIC-1 wanted-pair: the codegen rewrites a
        // statically-Text `format_debug(&x)` call (the `f"{x:?}"`
        // desugar) to the concrete `format_debug_text` twin — but that
        // substitution happens AFTER this AST-name harvest, so the twin
        // is never in `wanted` on its own and user compilation died with
        // "undefined function: format_debug_text" while the generic
        // original resolved fine. Wherever the original can be called,
        // the twin must be loadable too.
        if wanted.contains("format_debug") {
            wanted.insert("format_debug_text".to_string());
        }
        if std::env::var("VERUM_TRACE_WANTED").is_ok() {
            let dbg: Vec<&String> = wanted
                .iter()
                .filter(|n| n.contains("format_debug"))
                .collect();
            eprintln!("[wanted] apply_lazy fmt-related: {:?} (total {})", dbg, wanted.len());
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
        let symbol_graph = self.graph(archive);
        let unqualified_wanted: std::collections::HashSet<String> = wanted
            .iter()
            .filter(|n| !n.contains('.'))
            // T0738: a name no archive symbol carries cannot be found by the
            // scan below, and the scan costs a decode of every module. The
            // harvest feeds this set from the AST, local variable names
            // included, so the common case was paying 574 decodes to look
            // for a function named `v`.
            .filter(|n| symbol_graph.carries_simple_name(n))
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
        let mut harvest = Harvest::default();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut harvest);
        }
        let wanted: std::collections::HashSet<String> =
            std::mem::take(&mut harvest.names);
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
    /// T0706 — WANTED-HARVEST-POST-TYPECHECK second pass.  The primary
    /// pass harvests names from the RAW AST, so a method reachable only
    /// through a resolved receiver type never enters the keep set — the
    /// canonical miss: `let b: Byte = …; b.to_hex()` dispatches as
    /// `UInt8.to_hex` (alias canonicalisation + prim-mangle), a
    /// qualified name the AST never spells.  After user codegen, the
    /// emitted instruction stream IS the resolved-name oracle; this
    /// entry merges bodies for exactly those names, reusing the SAME
    /// register/type-import/merge machinery with the epoch's
    /// merged-ids guard suppressing re-merges.
    pub fn apply_lazy_supplemental(
        &self,
        archive: &VbcArchive,
        codegen: &mut verum_vbc::codegen::VbcCodegen,
        extra_wanted: std::collections::HashSet<String>,
    ) -> usize {
        if extra_wanted.is_empty() {
            return 0;
        }
        // Names already resolvable in ctx (registered AND with a merged
        // body, or an intrinsic intercept) don't need supplemental work.
        let unresolved: std::collections::HashSet<String> = extra_wanted
            .into_iter()
            .filter(|n| codegen.ctx_mut().lookup_function(n).is_none())
            .collect();
        if unresolved.is_empty() {
            return 0;
        }
        let graph = self.graph(archive);
        // No AST here — `extra_wanted` arrives as bare names that could
        // NOT be resolved in ctx, so their provenance is unknown and the
        // seed cap must not apply: this path exists precisely to find
        // something the earlier, narrower pass missed.
        let (reached_qualified, reached_module_idxs) =
            graph.reachable(&unresolved, &HashSet::new());
        let mut wanted = unresolved;
        for n in &reached_qualified {
            wanted.insert(n.clone());
        }
        let mut target_entries: Vec<(usize, String)> = reached_module_idxs
            .iter()
            .filter_map(|idx| {
                archive
                    .index
                    .get(*idx as usize)
                    .map(|e| (*idx as usize, e.name.clone()))
            })
            .collect();
        // DETERMINISM (T0736). `reached_module_idxs` is a `HashSet<u32>`,
        // so this vector comes out in a different order every process. The
        // merge below is sequential and its registration is FIRST-WINS
        // ("names registered by the primary pass stay first-wins"), so the
        // order decides which body a name ends up bound to — and, through
        // the contains_key guards, how many entries are registered at all.
        //
        // Measured before this line existed: three consecutive builds of
        // ONE unchanged source file produced 49103 / 49112 / 49102
        // functions, and a spec whose dispatch depends on the winner
        // answered correctly 1 run in 8. Determinism is a precondition for
        // every A/B in this repository, not a nicety.
        //
        // The archive index is the canonical order — the sibling pass at
        // `apply_lazy` already walks `archive.index` itself and is
        // unaffected. Sorting by index reproduces that order here.
        target_entries.sort_unstable_by_key(|(idx, _)| *idx);
        if target_entries.is_empty() {
            return 0;
        }
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
        if let Ok(f) = std::env::var("VERUM_TRACE_WANTED")
            && f != "1"
        {
            let names: Vec<&str> =
                decoded.iter().map(|(n, _)| n.as_str()).take(24).collect();
            eprintln!(
                "[wanted/supp] decoded {} entries: {:?}",
                decoded.len(),
                names
            );
        }
        let next_id_ptr: *mut u32 = codegen.next_func_id_mut() as *mut u32;
        let mut merged_entries = 0usize;
        for (entry_name, module) in &decoded {
            let next_id_ref: &mut u32 = unsafe { &mut *next_id_ptr };
            // Idempotent per the loader's contains_key guards — names
            // registered by the primary pass stay first-wins.
            let (func_id_remap, _registered) = register_module_filtered(
                module,
                entry_name,
                codegen.ctx_mut(),
                &wanted,
                next_id_ref,
            );
            if !module.types.is_empty() {
                codegen.import_archive_module_types(module);
            }
            // Merge ONLY bodies this epoch has not merged yet.
            let already: std::collections::HashSet<u32> = self
                .merged_ids
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(entry_name)
                .cloned()
                .unwrap_or_default();
            let fresh_remap: HashMap<u32, verum_vbc::module::FunctionId> =
                func_id_remap
                    .iter()
                    .filter(|(aid, _)| !already.contains(aid))
                    .map(|(a, u)| (*a, *u))
                    .collect();
            if fresh_remap.is_empty() {
                continue;
            }
            for fn_desc in module.functions.iter() {
                if let Some(&user_fid) = fresh_remap.get(&fn_desc.id.0)
                    && let Some(name) = module.strings.get(fn_desc.name)
                    && !name.is_empty()
                {
                    codegen.record_archive_function_name(name, user_fid);
                }
            }
            codegen.merge_archive_function_bodies(module, &fresh_remap);
            self.note_merged_ids(entry_name, fresh_remap.keys());
            merged_entries += 1;
        }
        if let Ok(filter) = std::env::var("VERUM_TRACE_WANTED") {
            eprintln!(
                "[wanted] supplemental pass: {} entries merged (wanted {})",
                merged_entries,
                wanted.len()
            );
            // T0706 thx3 instrument: when the flag carries a FILTER
            // value (not just "1"), report that name's presence in the
            // wanted set, its registration, and its merged-body status
            // — the three facts the unsafe-fn shape dispute needs.
            if filter != "1" && !filter.is_empty() {
                let hits: Vec<&String> =
                    wanted.iter().filter(|n| n.contains(&filter)).collect();
                eprintln!(
                    "[wanted/filter '{}'] in wanted: {:?}",
                    filter, hits
                );
                let g = self.graph(archive);
                for h in hits {
                    let reg = codegen.ctx_mut().lookup_function(h).is_some();
                    let in_graph = g.baked.module_of(h.as_str());
                    let entry = in_graph
                        .and_then(|i| archive.index.get(i as usize))
                        .map(|e| e.name.as_str())
                        .unwrap_or("-");
                    eprintln!(
                        "[wanted/filter]   '{}' registered={} graph_module={:?}({})",
                        h, reg, in_graph, entry
                    );
                }
                let key_hits: Vec<String> = codegen
                    .ctx_mut()
                    .functions
                    .keys()
                    .filter(|k| k.contains(&filter))
                    .take(12)
                    .cloned()
                    .collect();
                eprintln!(
                    "[wanted/filter] ctx.functions keys ~'{}': {:?}",
                    filter, key_hits
                );
            }
        }
        merged_entries
    }

    pub fn apply_lazy_with_types(
        &self,
        archive: &VbcArchive,
        codegen: &mut verum_vbc::codegen::VbcCodegen,
        user_module: &verum_ast::Module,
    ) -> (usize, usize) {
        let t_load = std::time::Instant::now();
        self.begin_compilation_epoch();
        // **ONE-authority alias seeding** (T0695/T0692): every codegen
        // that loads the embedded archive gets the DECLARED type
        // aliases (`type Byte is UInt8`) and re-export renames from
        // the baked metadata — HERE, at the single archive⇄codegen
        // meeting point. The per-caller seeding sprinkle missed the
        // script path (api.rs constructed codegen without it), so
        // `verum run` compiled with an EMPTY alias table while `verum
        // build`/`verum test` had one — three entry points, two
        // behaviours, one class of unresolvable `Byte.*` dispatch
        // names. Idempotent (import_type_aliases is first-wins) and
        // cheap (one pass over metadata.types per compilation).
        if let Some(metadata) = crate::embedded_stdlib_metadata::get_runtime_metadata() {
            crate::pipeline::vbc_codegen::seed_reexport_type_aliases(codegen, &metadata);
        }
        let mut harvest = Harvest::default();
        for item in user_module.items.iter() {
            collect_referenced_function_names(item, &mut harvest);
        }
        let bare_method_seeds = std::mem::take(&mut harvest.bare_methods);
        let mut wanted: std::collections::HashSet<String> =
            std::mem::take(&mut harvest.names);
        // TEXT-DEBUG-STATIC-1 wanted-pair: the codegen rewrites a
        // statically-Text `format_debug(&x)` call (the `f"{x:?}"`
        // desugar) to the concrete `format_debug_text` twin — but that
        // substitution happens AFTER this AST-name harvest, so the twin
        // is never wanted on its own and user compilation died with
        // "undefined function: format_debug_text" while the generic
        // original resolved fine. Wherever the original can be called,
        // the twin must be loadable too.
        if wanted.contains("format_debug") {
            wanted.insert("format_debug_text".to_string());
        }
        if std::env::var("VERUM_TRACE_WANTED").is_ok() {
            let dbg: Vec<&String> = wanted
                .iter()
                .filter(|n| n.contains("format_debug"))
                .collect();
            eprintln!(
                "[wanted] apply_lazy_with_types fmt-related: {:?} (total {})",
                dbg,
                wanted.len()
            );
        }
        if wanted.is_empty() {
            loadcost::report("apply_lazy_with_types (empty wanted)", t_load.elapsed());
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
                wanted_module_prefixes.extend(
                    verum_common::well_known_types::WellKnownType::Maybe
                        .canonical_archive_modules()
                        .iter()
                        .map(|m| (*m).to_string()),
                );
                to_add.push("Maybe");
            }
            if verum_common::well_known_types::variant_tags::is_result_constructor(name) {
                wanted_module_prefixes.extend(
                    verum_common::well_known_types::WellKnownType::Result
                        .canonical_archive_modules()
                        .iter()
                        .map(|m| (*m).to_string()),
                );
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
            for wk in [verum_common::well_known_types::WellKnownType::Maybe, verum_common::well_known_types::WellKnownType::Result] {
                wanted_module_prefixes
                    .extend(wk.canonical_archive_modules().iter().map(|m| (*m).to_string()));
            }
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

        // **Transitive-closure reachability** (replaces the prior
        // architecture's 5 hardcoded force-loads for tasks #23 / #24 /
        // #26). Build the archive-wide symbol graph once (cached on
        // `self.graph` for the process lifetime), BFS from user
        // seeds following every `Call` / `TailCall` / `CallM` edge
        // observed in archive bytecode, and union the resulting
        // qualified-name set into `wanted` + the defining-module set
        // into `wanted_module_prefixes`. Every cross-module dependency
        // surfaces by construction — no hardcoded entries.
        let graph = self.graph(archive);
        let (reached_qualified, reached_module_idxs) =
            loadcost::timed("bfs_reachable", || {
                graph.reachable(&wanted, &bare_method_seeds)
            });
        if std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok() {
            eprintln!(
                "[reachable] wanted={} reached_qualified={} reached_modules={}",
                wanted.len(),
                reached_qualified.len(),
                reached_module_idxs.len(),
            );
        }
        // A CALL'S RESULT TYPE IS REACHED BY THE CALL (T0692).
        //
        // `f"{a.cmp(b)}"` needs `Ordering.fmt`, but nothing in the
        // program's TEXT names `Ordering` — the type arrives as
        // `Int.cmp`'s result, and reachability is computed from source
        // text before inference has run. So the closure left the impl
        // out and the f-string printed the variant name `Less` where
        // `implement Display for Ordering` says `<`.
        //
        // The missing fact lives in the baked metadata as
        // `FunctionDescriptor.return_type`, recorded BY NAME — which is
        // what the graph cannot supply, since archive TypeIds are
        // assigned per module and are not comparable across them.
        //
        // Narrowed twice over, both narrowings paid for by measurement
        // on a hello-world's archive load:
        //
        //  * Only the calls the program puts in FORMAT POSITION are
        //    seeds (`formatted_call_names`). Seeding from every reached
        //    function instead put the load at 1336 ms against 67 ms.
        //  * Only types that HAVE a `Display` impl are pulled — the
        //    presence of `<Type>.fmt` in the graph. Display is the
        //    entire reason a formatted result needs its type.
        //
        // A program that formats only literals harvests no names and
        // reaches none of this. `VERUM_SEED_RESULT_TYPES=0` disables
        // the seeding, which is how the two numbers above were taken.
        let trace = std::env::var_os("VERUM_TRACE_CODEGEN_PATH").is_some();
        let formatted = if std::env::var("VERUM_SEED_RESULT_TYPES").as_deref() == Ok("0") {
            HashSet::new()
        } else {
            formatted_call_names(user_module)
        };
        if trace {
            let mut names: Vec<&str> = formatted.iter().map(String::as_str).collect();
            names.sort_unstable();
            eprintln!("[formatted] {} names in format position: {:?}", names.len(), names);
        }
        let mut display_types: HashSet<String> = HashSet::new();
        if !formatted.is_empty()
            && let Some(metadata) = crate::embedded_stdlib_metadata::get_runtime_metadata()
        {
            // Scanned over the metadata rather than over
            // `reached_qualified`: `a.cmp(b)` enters the closure as the
            // BARE method seed `cmp`, so `Int.cmp` never appears as a
            // reached QUALIFIED name — a scan of the reached set finds
            // nothing (measured: 8 reached names, none of them a `cmp`).
            for (qualified, desc) in metadata.functions.iter() {
                let qualified = qualified.as_str();
                let simple = qualified.rsplit('.').next().unwrap_or(qualified);
                if !formatted.contains(simple) {
                    continue;
                }
                // The return type is a rendered type EXPRESSION
                // (`Maybe<Ordering>`, `List<Text>`), so every
                // capitalised component is a candidate, not just the
                // head — `Maybe<Ordering>` formats through `Ordering`.
                for part in desc
                    .return_type
                    .as_str()
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                {
                    if part.len() > 1
                        && part.starts_with(char::is_uppercase)
                        && !wanted.contains(part)
                        && !display_types.contains(part)
                        && graph.has_symbol(&format!("{}.fmt", part))
                    {
                        if trace {
                            eprintln!("[formatted] {} returns {} (has Display)", qualified, part);
                        }
                        display_types.insert(part.to_string());
                    }
                }
            }
        }
        if !display_types.is_empty() {
            let extra: HashSet<String> = display_types
                .iter()
                .map(|t| format!("{}.fmt", t))
                .chain(display_types.iter().cloned())
                .collect();
            let (more_qualified, more_modules) =
                loadcost::timed("bfs_display", || graph.reachable(&extra, &HashSet::new()));
            for name in more_qualified {
                wanted.insert(name);
            }
            for idx in more_modules {
                if let Some(entry) = archive.index.get(idx as usize) {
                    wanted_module_prefixes.insert(entry.name.clone());
                }
            }
            wanted.extend(display_types);
        }

        for idx in &reached_module_idxs {
            if let Some(entry) = archive.index.get(*idx as usize) {
                wanted_module_prefixes.insert(entry.name.clone());
            }
        }
        // Adding reached names to `wanted` makes the
        // per-function filter in `register_module_filtered`
        // accept them via the literal-simple-name branch — no
        // need for a separate acceptance arm. Auxiliary fanouts
        // keyed on `wanted` (canonical-`Type.method` registration,
        // alias-leaf fanout) automatically pick up these entries.
        //
        // Important: include bare-named functions too (e.g. `memcpy`,
        // `alloc`, `panic`). These are the cross-module Call/CallM
        // callees that stdlib bodies depend on transitively — without
        // them, `Text.push_str`'s body's `Call` to `memcpy` resolves
        // to a remap miss → `Function N not found` at runtime.
        // The unqualified-wanted Pass 2's filter
        // (`looks_like_type_name` + `lookup_function(name).is_none()`)
        // already gates the full-archive scan, so bare reached names
        // that ARE registered through Pass 1 don't trigger redundant
        // module decoding.
        // Keep `reached_qualified` alive past the union — the merge
        // pruner (UMBRELLA-MOUNT-PRUNE-1) seeds its per-entry keep
        // sets from it by exact descriptor-name match.
        for name in &reached_qualified {
            wanted.insert(name.clone());
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
        let decoded: Vec<(String, VbcModule)> = loadcost::timed("decode_entries", || {
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
        });
        // Split borrows: ctx and next_func_id are separate fields, but
        // both need &mut from VbcCodegen.  Re-using the same raw-ptr
        // round-trip discipline as the apply_lazy call site in
        // `pipeline/vbc_codegen.rs`.
        let next_id_ptr: *mut u32 = codegen.next_func_id_mut() as *mut u32;
        // UMBRELLA-MOUNT-PRUNE-1: the decoded modules' TYPE descriptors
        // get their fn-id references (drop_fn / clone_fn / protocol
        // vtables) rewritten in place before import — see
        // `remap_type_glue_fn_ids`.  Needs `mut` access.
        let mut decoded = decoded;
        // **Two-phase merge** (task #12 fix).
        //
        // Pre-fix this loop ran register → types → merge per archive
        // in sequence, which meant the merge of archive A couldn't see
        // archive B's name→fid bindings if B was processed after A.
        // The Tier-2b cross-module name fallback in
        // `ArchiveBodyRemap::map_function` (added in this task) needs
        // every loaded archive's function names visible BEFORE the
        // first body merge runs — otherwise A's body Calls into B's
        // functions hit Tier-3 IDENTITY and silently miscompile.
        //
        // Phase 1: per archive — register_module_filtered (populates
        //          ctx.functions for the wanted subset) + types import
        //          (must precede merge so TypeId remap sees descriptors)
        //          + populate archive_func_name_to_fid for EVERY
        //          archive function (mount-set-independent).
        // Phase 2: per archive — merge_archive_function_bodies, with
        //          archive_func_name_to_fid fully populated across all
        //          loaded archives.
        let mut per_archive_remaps: Vec<(String, std::collections::HashMap<u32, verum_vbc::module::FunctionId>)> =
            Vec::with_capacity(decoded.len());
        let mut registered_ids_by_entry: std::collections::HashMap<String, std::collections::HashSet<u32>> =
            std::collections::HashMap::new();
        let prune_disabled = std::env::var("VERUM_NO_MOUNT_PRUNE").is_ok();
        let t_register = std::time::Instant::now();
        for (entry_name, module) in &decoded {
            // Function side first so Pass 4 (variant ctors) sees
            // the stable function-id namespace.
            // SAFETY: ctx and next_func_id are non-overlapping fields
            // on the same VbcCodegen — splitting via raw pointer keeps
            // the borrow checker out of the way without breaking
            // aliasing rules.
            let next_id_ref: &mut u32 = unsafe { &mut *next_id_ptr };
            let (func_id_remap, registered_ids) = register_module_filtered(
                module,
                entry_name,
                codegen.ctx_mut(),
                &wanted,
                next_id_ref,
            );
            fn_modules += 1;
            registered_ids_by_entry.insert(entry_name.clone(), registered_ids);
            per_archive_remaps.push((entry_name.clone(), func_id_remap));
        }
        loadcost::record("register_filtered", t_register.elapsed());
        // ── UMBRELLA-MOUNT-PRUNE-1: function-granular merge pruning ──
        //
        // `func_id_remap` is TOTAL over each entry's function table (id
        // allocation precedes the filter — identity-fallback shield).
        // Handing the total map to the merge below merges each decoded
        // entry's ENTIRE function table (measured: 46,782 bodies merged
        // for a wanted-set that reaches 4,898) and the AOT leg then
        // LLVM-lowers every one of them per test.  Compute the least
        // semantics-preserving keep set instead (see
        // `compute_merge_keep_sets` for the closure rules: registered
        // surface, BFS-reached names, `__tls_init_*`, local+cross-entry
        // fn-id operand closure, imported-type glue, wanted/constructed
        // types' dispatch surface) and hand the merge / name-index /
        // alias stages the FILTERED view.  `VERUM_NO_MOUNT_PRUNE=1`
        // restores the total-map behaviour bit-for-bit.
        //
        // NOTE: the keep computation reads the ORIGINAL archive-local
        // glue ids off the decoded type tables, so it must run BEFORE
        // `remap_type_glue_fn_ids` rewrites them below.
        let keep_by_entry: Option<std::collections::HashMap<String, std::collections::HashSet<u32>>> =
            if prune_disabled {
                None
            } else {
                Some(loadcost::timed("keep_set_fixpoint", || {
                    compute_merge_keep_sets(
                        &decoded,
                        &registered_ids_by_entry,
                        &reached_qualified,
                        &wanted,
                    )
                }))
            };
        // Type side — push every non-protocol descriptor.  MUST happen
        // before body merge so the body's TypeId remap (consults
        // `codegen.type_name_to_id`) sees the imported descriptors.
        //
        // **Imported-type glue-id rewrite** (pruning mode only): the
        // import copies `drop_fn` / `clone_fn` / `ProtocolImpl.methods`
        // VERBATIM — i.e. as ARCHIVE-LOCAL fn ids — and the finalize
        // pass later remaps those values as if they were ctx ids.
        // Under merge-all that latent id-space confusion is masked
        // (every low ctx id owns a merged body, so the stale value
        // lands on a consistent, harmless target); under pruning the
        // compacted final table exposed it as drop-glue misdispatch
        // (`DropRef` on a span-test local executing `Signal.fmt`).
        // Rewriting the references through this entry's total remap
        // BEFORE the import turns them into real ctx ids, so finalize
        // resolves them to the kept glue bodies — see
        // `remap_type_glue_fn_ids`.  Gated on pruning so the
        // kill-switch path stays bit-identical to the old behaviour.
        for i in 0..decoded.len() {
            if !prune_disabled {
                let total_remap = &per_archive_remaps[i].1;
                let module = &mut decoded[i].1;
                remap_type_glue_fn_ids(module, total_remap);
            }
            let module = &decoded[i].1;
            if !module.types.is_empty() {
                codegen.import_archive_module_types(module);
                type_modules += 1;
            }
        }
        let module_by_name: std::collections::HashMap<&str, &VbcModule> = decoded
            .iter()
            .map(|(n, m)| (n.as_str(), m))
            .collect();
        let pruned_remaps: Vec<(String, std::collections::HashMap<u32, verum_vbc::module::FunctionId>)> =
            per_archive_remaps
                .iter()
                .map(|(entry_name, total)| {
                    let pruned = match keep_by_entry
                        .as_ref()
                        .and_then(|k| k.get(entry_name))
                    {
                        Some(keep) => total
                            .iter()
                            .filter(|(archive_id, _)| keep.contains(archive_id))
                            .map(|(a, u)| (*a, *u))
                            .collect(),
                        None => total.clone(),
                    };
                    (entry_name.clone(), pruned)
                })
                .collect();
        if std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok() {
            let total_fns: usize = per_archive_remaps.iter().map(|(_, m)| m.len()).sum();
            let kept_fns: usize = pruned_remaps.iter().map(|(_, m)| m.len()).sum();
            eprintln!(
                "[mount-prune] entries={} total_fns={} kept_fns={} disabled={}",
                per_archive_remaps.len(),
                total_fns,
                kept_fns,
                prune_disabled,
            );
        }
        for (entry_name, pruned_remap) in &pruned_remaps {
            let Some(module) = module_by_name.get(entry_name.as_str()).copied() else {
                continue;
            };
            // Phase-1 tail: populate the archive-wide
            // name → user_fid index for every KEPT function in this
            // archive (task #12).  Names of pruned-away functions
            // deliberately stay out of `archive_func_name_to_fid`: a
            // name bound to an unmerged id would let Tier-2b route a
            // cross-module call onto a finalize-time `RetV`-Unit stub
            // (silent), whereas a miss falls through to the same
            // identity-fallback class that undecoded entries already
            // exercise (diagnosable).
            for fn_desc in module.functions.iter() {
                if let Some(&user_fid) = pruned_remap.get(&fn_desc.id.0)
                    && let Some(name) = module.strings.get(fn_desc.name)
                    && !name.is_empty()
                {
                    codegen.record_archive_function_name(name, user_fid);
                }
            }
        }
        // STUB-STAGE-INSUITE-1: install re-export alias SPELLINGS into
        // the archive-wide name index AFTER every entry's kept names
        // are recorded (alias targets resolve independent of entry
        // order) and BEFORE any body merge snapshots the index —
        // cross-module Calls recorded under an alias's qualified key
        // (`core.base.memory.memcpy`) resolve exactly, instead of
        // freezing the XMOD band id into the rewritten body.  Keep the
        // triples alive: pass-2 / supplemental-wave registrations can
        // resolve targets this round left pending, and the install is
        // idempotent first-wins.
        let mut primary_alias_triples: Vec<(
            String,
            Option<verum_vbc::module::FunctionId>,
            String,
        )> = Vec::new();
        for (entry_name, pruned_remap) in &pruned_remaps {
            let Some(module) = module_by_name.get(entry_name.as_str()).copied() else {
                continue;
            };
            primary_alias_triples
                .extend(collect_mount_alias_triples(module, pruned_remap));
        }
        install_mount_alias_archive_names(&primary_alias_triples, codegen);
        let t_types = std::time::Instant::now();
        for (entry_name, pruned_remap) in &pruned_remaps {
            let Some(module) = module_by_name.get(entry_name.as_str()).copied() else {
                continue;
            };
            // Task #11 Phase 4: replay mount-rename aliases captured at
            // precompile.  Each (alias_str_id, archive_fid) entry maps
            // an alias name (interned in this module's string table) to
            // the archive-local FunctionId of its target.  We remap the
            // archive-local fid to the user-side fid via the remap,
            // look up the resulting FunctionInfo from `ctx.functions`,
            // and re-install the alias via
            // `register_function_authoritative` so user-side bare-name
            // lookup sees identical alias bindings to what the precompile
            // stage observed.  Targets filtered out of `wanted` by
            // `register_module_filtered` produce a remap miss and we
            // silently skip — matching the rest of the loader's filter
            // discipline (alias targets are always REGISTERED functions,
            // which the keep set includes by construction).
            replay_mount_aliases(module, entry_name, pruned_remap, codegen);
        }
        loadcost::record("types_import", t_types.elapsed());
        // Phase 2: body merges now see every loaded archive's name
        // bindings in `archive_func_name_to_fid`, so cross-module
        // Calls inside A's bodies resolve to B's functions via
        // Tier-2b even when B isn't in the user's `wanted` mount set.
        // Each archive_func_name_to_fid update is first-wins, so
        // re-running this loop on top of Phase 1's registrations is
        // idempotent.
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
        //
        // UMBRELLA-MOUNT-PRUNE-1: the merge consumes the keep-closure-
        // FILTERED remap view, not the total map — see the pruning
        // block above.  Kept bodies' local Call operands stay Tier-1-
        // resolvable by closure construction; everything pruned away
        // is unreachable from the registered surface, the BFS-reached
        // set, and the live type surface.
        let t_merge = std::time::Instant::now();
        for (entry_name, pruned_remap) in &pruned_remaps {
            if let Some(module) = module_by_name.get(entry_name.as_str()).copied() {
                codegen.merge_archive_function_bodies(module, pruned_remap);
                // T0706: remember what this epoch merged so the
                // supplemental post-typecheck pass never re-merges.
                self.note_merged_ids(entry_name, pruned_remap.keys());
            }
        }
        loadcost::record("merge_bodies", t_merge.elapsed());
        // T0711: everything below this index is archive-merged; the
        // post-codegen dispatch-name collector scans only past it.
        codegen.archive_merged_fn_watermark = codegen.function_count() as usize;
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
        // T0738: drop names NO archive symbol carries before paying for the
        // scan below — it decodes all 574 modules looking for a simple-name
        // match, so a name that cannot match is pure waste. The AST harvest
        // feeds this set every path segment it sees, LOCAL VARIABLE NAMES
        // INCLUDED: `let v: Int = 10; print(v);` shipped `v`, and hunting a
        // stdlib function called `v` took the module from 12604 functions to
        // 66797 and the build from 3.3s to 26.2s.
        //
        // `carries_simple_name` is a hash lookup on the symbol graph, which
        // these runs have already built for the reachability step — the
        // filter is free where the scan it prevents is not.
        let graph_for_names = self.graph(archive);
        // A MOUNT PATH SEGMENT IS NOT A FUNCTION NAME (T0753).
        //
        // `collect_mount_names` inserts a Path mount's last segment as a
        // bare name, under the comment "last segment is the name".  That
        // holds for a NAME-form mount (`mount core.io.read;` — `read` is
        // a function) and is a category error for a MODULE-form one
        // (`mount core.text;` — `text` is a module).  The two forms are
        // syntactically identical, so the harvest genuinely cannot tell
        // them apart.  The archive can: if the dot-joined form names an
        // archive entry, the last segment named a module.
        //
        // Left in the set, that one name buys a decode of EVERY archive
        // module, and the match test there is "does this module's string
        // table contain `text`" — true of every module that merely CALLS
        // something named `text`.  Measured on
        // `mount core.text; fn main() { print("…") }`: 40149 functions
        // merged from 370 modules, including all three platforms' syscall
        // layers, sqlite (1986), postgres (966) and x509 — none of it
        // reachable from `print`.  `VERUM_TRACE_FULLSCAN=1` named the
        // whole trigger: one name, `text`.
        //
        // Dropping it loses nothing.  The qualified pass has already
        // decoded `core.text` itself (its dotted form is in
        // `wanted_module_prefixes`), so whatever that module declares
        // under this simple name is registered there.  The scan could
        // only add OTHER modules' same-named symbols — the
        // identity-by-simple-name over-approximation this loader spends
        // its whole architecture avoiding.
        let mount_module_segments: std::collections::HashSet<&str> = {
            let entries: std::collections::HashSet<&str> =
                archive.index.iter().map(|e| e.name.as_str()).collect();
            wanted
                .iter()
                .filter(|n| entries.contains(n.as_str()))
                .filter_map(|n| n.rsplit('.').next())
                .collect()
        };
        let keep_mount_segments =
            std::env::var_os("VERUM_NO_MOUNT_SEGMENT_FILTER").is_some();
        let unqualified_wanted: std::collections::HashSet<String> = unqualified_wanted_full
            .into_iter()
            .filter(|name| {
                codegen.ctx_mut().lookup_function(name).is_none()
                    && !looks_like_type_name(name)
                    && graph_for_names.carries_simple_name(name)
                    && (keep_mount_segments
                        || !mount_module_segments.contains(name.as_str()))
            })
            .collect();
        // FULL-ARCHIVE-SCAN PROBE (T0738). Every name still in this set
        // costs a decode of all 574 archive modules — the comment above
        // puts the worst case at ~5 GB of decoded modules. Measured with
        // it: `print(1)` compiles to a 12604-function module and
        // `let v: Int = 10; print(v);` to 66797, the difference being the
        // whole stdlib including Windows bindings on macOS and
        // `core.math.examples`. `VERUM_TRACE_FULLSCAN=1` names the entries
        // that bought that, so the trigger is read rather than guessed.
        if std::env::var_os("VERUM_TRACE_FULLSCAN").is_some() {
            let mut names: Vec<&String> = unqualified_wanted.iter().collect();
            names.sort();
            eprintln!(
                "[fullscan] {} unqualified name(s) force a full-archive scan: {:?}",
                names.len(),
                names
            );
        }
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
            let mut matched_modules = matched_modules;
            // STUB-STAGE-INSUITE-1 phase split — mirror the primary
            // pass's task-#12 two-phase discipline in the pass-2 loop.
            // Pre-fix each matched module was registered AND merged
            // inside ONE iteration, so (a) module M1's bodies froze
            // XMOD band ids for callees that M2, later in the same
            // loop, would have registered, and (b) pass-2 bodies froze
            // ids for primary-entry functions that only the
            // supplemental keep-wave (which ran AFTER these merges)
            // recorded into the archive-wide index.  Both froze as
            // Tier-3 identity ids in rewritten bytecode and died at
            // runtime as `FunctionNotFound(0x2000_00xx)`.  Phase A
            // (this loop): register + type import + name recording for
            // ALL matched modules.  Phase B (below, after the
            // supplemental wave's name recording): body merges.
            let mut pass2_remaps: Vec<
                std::collections::HashMap<u32, verum_vbc::module::FunctionId>,
            > = Vec::with_capacity(matched_modules.len());
            for (entry_name, module) in matched_modules.iter_mut() {
                let next_id_ref: &mut u32 = unsafe { &mut *next_id_ptr };
                // UMBRELLA-MOUNT-PRUNE-1: the second pass keeps the
                // TOTAL-remap merge (unchanged semantics) — these
                // modules were pulled by live bare-name references and
                // are few; pruning them is tracked as the separate
                // pass-2-tightening leg.  The supplemental wave BELOW
                // re-opens the primary pass's keep sets for anything
                // these bodies call into.
                let (func_id_remap, _registered_ids) = register_module_filtered(
                    module,
                    entry_name,
                    codegen.ctx_mut(),
                    &unqualified_wanted,
                    next_id_ref,
                );
                fn_modules += 1;
                // Imported-type glue-id rewrite — same rationale as the
                // primary pass's site above (drop_fn / clone_fn /
                // vtable ids must arrive as ctx ids for finalize).
                if !prune_disabled {
                    remap_type_glue_fn_ids(module, &func_id_remap);
                }

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
                // Populate archive-wide name → user_fid index for THIS
                // archive's functions before the body merge, so any
                // cross-module Calls already inside `codegen.functions`
                // (from the primary pass above) can resolve targets
                // newly registered here via Tier-2b. (task #12)
                for fn_desc in module.functions.iter() {
                    if let Some(&user_fid) = func_id_remap.get(&fn_desc.id.0)
                        && let Some(name) = module.strings.get(fn_desc.name)
                        && !name.is_empty()
                    {
                        codegen.record_archive_function_name(name, user_fid);
                    }
                }
                pass2_remaps.push(func_id_remap);
            }
            // STUB-STAGE-INSUITE-1: alias-spelling install for the
            // pass-2 modules, plus a retry of the primary pass's
            // pending aliases — targets registered only by pass-2 can
            // now resolve.  Both idempotent first-wins.
            let mut pass2_alias_triples: Vec<(
                String,
                Option<verum_vbc::module::FunctionId>,
                String,
            )> = Vec::new();
            for (i, (_entry_name, module)) in matched_modules.iter().enumerate() {
                pass2_alias_triples
                    .extend(collect_mount_alias_triples(module, &pass2_remaps[i]));
            }
            install_mount_alias_archive_names(&pass2_alias_triples, codegen);
            install_mount_alias_archive_names(&primary_alias_triples, codegen);
            for (i, (entry_name, module)) in matched_modules.iter().enumerate() {
                // Task #11 Phase 4: alias replay in the unqualified
                // second pass too — symmetric with the Phase-1 site
                // above.  Aliases captured by stdlib modules brought
                // in through the bare-name fallback need the same
                // user-side reinstall as those from explicit-mount
                // modules in the primary pass.
                replay_mount_aliases(module, entry_name, &pass2_remaps[i], codegen);
            }
            // UMBRELLA-MOUNT-PRUNE-1 supplemental wave (upgrade-once).
            //
            // Pass-2 modules merge their FULL function tables, and
            // those bodies may call into primary-pass entries whose
            // keep sets were sealed before pass-2 ran.  Re-run the
            // keep closure with pass-2 bodies' cross-module callee
            // names as extra seeds and merge exactly the per-entry
            // DELTA.  Idempotent and allocation-free: the merge skips
            // already-emitted ids, and the delta draws its user-side
            // ids from the SAME total remaps the primary pass
            // allocated.  Each entry's merged surface therefore grows
            // at most once — the function-granular equivalent of the
            // TypeOnly→Full single upgrade.
            //
            // STUB-STAGE-INSUITE-1: the wave's NAME RECORDING runs
            // here, BEFORE the pass-2 body merges below — pre-fix the
            // merges ran first, so a pass-2 body's Call into a
            // supplement-only function missed every remap tier and
            // froze its XMOD band id (the merge is a one-shot
            // bytecode rewrite; a later index insert can't heal an
            // already-frozen operand).  The supplement BODY merges
            // stay last — merge order never matters, only
            // names-before-any-merge does.
            let mut supplemental_merges: Vec<(
                String,
                std::collections::HashMap<u32, verum_vbc::module::FunctionId>,
            )> = Vec::new();
            if let Some(keep1) = keep_by_entry.as_ref()
                && !matched_modules.is_empty()
            {
                let mut supplemental_seeds: std::collections::HashSet<String> =
                    reached_qualified.clone();
                for (_entry_name, module) in &matched_modules {
                    collect_cross_module_callee_names(module, &mut supplemental_seeds);
                }
                let keep2 = compute_merge_keep_sets(
                    &decoded,
                    &registered_ids_by_entry,
                    &supplemental_seeds,
                    &wanted,
                );
                for (entry_name, total) in &per_archive_remaps {
                    let k1 = keep1.get(entry_name);
                    let Some(k2) = keep2.get(entry_name) else {
                        continue;
                    };
                    let supplement: std::collections::HashMap<u32, verum_vbc::module::FunctionId> =
                        total
                            .iter()
                            .filter(|(archive_id, _)| {
                                k2.contains(*archive_id)
                                    && !k1.is_some_and(|s| s.contains(*archive_id))
                            })
                            .map(|(a, u)| (*a, *u))
                            .collect();
                    if supplement.is_empty() {
                        continue;
                    }
                    let Some(module) = module_by_name.get(entry_name.as_str()).copied()
                    else {
                        continue;
                    };
                    for fn_desc in module.functions.iter() {
                        if let Some(&user_fid) = supplement.get(&fn_desc.id.0)
                            && let Some(name) = module.strings.get(fn_desc.name)
                            && !name.is_empty()
                        {
                            codegen.record_archive_function_name(name, user_fid);
                        }
                    }
                    supplemental_merges.push((entry_name.clone(), supplement));
                }
                // STUB-STAGE-INSUITE-1: aliases whose targets were
                // pruned by keep1 but re-opened by keep2 resolve now
                // that the supplement names are recorded.
                install_mount_alias_archive_names(&primary_alias_triples, codegen);
                install_mount_alias_archive_names(&pass2_alias_triples, codegen);
            }
            // ── Phase B: body merges — every kept, pass-2,
            // supplemental, and alias name is now in the archive-wide
            // index, so no merge can freeze a name-resolvable XMOD id.
            for (i, (_entry_name, module)) in matched_modules.iter().enumerate() {
                // Body merge for the unqualified-wanted second pass —
                // same Phase 2 path as the primary pass above. See
                // that site for rationale.
                codegen.merge_archive_function_bodies(module, &pass2_remaps[i]);
            }
            let mut upgraded_fns = 0usize;
            for (entry_name, supplement) in &supplemental_merges {
                let Some(module) = module_by_name.get(entry_name.as_str()).copied()
                else {
                    continue;
                };
                codegen.merge_archive_function_bodies(module, supplement);
                upgraded_fns += supplement.len();
            }
            if !supplemental_merges.is_empty()
                && std::env::var("VERUM_TRACE_CODEGEN_PATH").is_ok()
            {
                eprintln!(
                    "[mount-prune] pass-2 supplemental merged_fns={}",
                    upgraded_fns,
                );
            }
        }
        loadcost::report("apply_lazy_with_types", t_load.elapsed());
        (fn_modules, type_modules)
    }
}

/// Task #11 Phase 4: replay mount-rename aliases captured in a
/// precompiled module into the user-side codegen context.
///
/// For each `(alias_str_id, archive_fid)` entry recorded by the
/// precompile-side `VbcCodegen::build_module` drain, we resolve
/// the alias name from the module's string table, remap the
/// archive-local FunctionId through `func_id_remap`, look up the
/// resulting user-side `FunctionInfo`, and re-install the binding
/// via `register_function_authoritative` so user-side bare-name
/// lookup sees the identical alias mapping the precompile stage
/// observed.
///
/// Misses (target filtered out of `wanted`, missing string entry,
/// empty alias name) are silently skipped to match the rest of the
/// loader's filter discipline.
fn replay_mount_aliases(
    module: &VbcModule,
    entry_name: &str,
    func_id_remap: &std::collections::HashMap<u32, verum_vbc::module::FunctionId>,
    codegen: &mut verum_vbc::codegen::VbcCodegen,
) {
    if module.mount_aliases.is_empty() {
        return;
    }
    // Pre-resolve (alias_name, fid?, target_key) triples in a single
    // read pass so the subsequent register loop can hold &mut
    // codegen.ctx without aliasing the immutable module borrow.
    let pairs = collect_mount_alias_triples(module, func_id_remap);
    if pairs.is_empty() {
        return;
    }
    let ctx = codegen.ctx_mut();
    for (alias_name, user_fid, target_key) in pairs {
        let info = match user_fid
            .and_then(|fid| ctx.lookup_function_by_id(fid))
            .or_else(|| {
                // Name-authoritative fallback: the target's canonical
                // registry key, registered when ITS entry loaded.
                if target_key.is_empty() {
                    None
                } else {
                    ctx.lookup_function(&target_key)
                }
            })
            .or_else(|| {
                // ROOTED fallback. `mount_aliases` carries the target in
                // the BAKE-TIME spelling (`sys.darwin.errno.is_retryable`),
                // but `register_module_filtered` registers the same
                // function under the entry-merged key
                // (`core.sys.darwin.errno.is_retryable` for entry
                // `core.sys`). The raw lookup above therefore misses for
                // every alias whose target sits under a `core.`-rooted
                // entry, and the alias is never re-installed — which is
                // what left a two-hop re-exported FUNCTION unresolvable at
                // codegen while its one-hop and constant siblings worked.
                // Merge with the same helper the registration path uses so
                // the two spellings cannot drift.
                if target_key.is_empty() {
                    None
                } else {
                    let rooted = merge_module_and_simple_name(entry_name, &target_key);
                    if rooted == target_key {
                        None
                    } else {
                        ctx.lookup_function(&rooted)
                    }
                }
            }) {
            Some(info) => info.clone(),
            None => continue,
        };
        ctx.register_function_authoritative(alias_name, info);
    }
}

/// Shared mount-alias reader (STUB-STAGE-INSUITE-1 refactor): resolve
/// each `(alias_str_id, archive_fid, target_str_id)` row of a decoded
/// module's `mount_aliases` table into an owned
/// `(alias_name, same_entry_user_fid, carried_target_key)` triple.
///
/// The fid maps only when the target lives in THIS archive entry
/// (fids are renumbered per-entry at serialization).  A miss is the
/// NORMAL case for cross-subtree re-exports — resolution falls to the
/// carried target key (REEXPORT-QUALIFIED-KEY-1), never silently
/// skips.  Consumed by [`replay_mount_aliases`] (ctx-side
/// FunctionInfo re-install) and
/// [`install_mount_alias_archive_names`] (archive-wide name-index
/// install).
fn collect_mount_alias_triples(
    module: &VbcModule,
    func_id_remap: &std::collections::HashMap<u32, verum_vbc::module::FunctionId>,
) -> Vec<(String, Option<verum_vbc::module::FunctionId>, String)> {
    let mut triples = Vec::with_capacity(module.mount_aliases.len());
    for (alias_str_id, archive_fid, target_str_id) in module.mount_aliases.iter() {
        let alias_name = match module.get_string(*alias_str_id) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let target_key = module
            .get_string(*target_str_id)
            .unwrap_or("")
            .to_string();
        let user_fid = func_id_remap.get(&archive_fid.0).copied();
        if user_fid.is_none() && target_key.is_empty() {
            continue;
        }
        triples.push((alias_name, user_fid, target_key));
    }
    triples
}

/// STUB-STAGE-INSUITE-1 — install re-export (mount-alias) SPELLINGS
/// as first-class keys of the archive-wide name index
/// (`VbcCodegen::archive_func_name_to_fid`).
///
/// Root defect this closes: stdlib bodies record cross-module callees
/// under the spelling the CALLER knew, which for a re-exported
/// function is the alias's qualified key — `core.base.memory.memcpy`
/// for `public mount core.intrinsics.memory.memcpy` in
/// `core/base/memory.vr`.  No function DESCRIPTOR carries that name,
/// so the kept-function name recording can never index it; the only
/// load-time authority is the `mount_aliases` row (alias spelling +
/// carried resolved target key, REEXPORT-QUALIFIED-KEY-1).
/// `replay_mount_aliases` re-installs the binding into
/// `ctx.functions` ONLY when the target is itself
/// mount-filter-accepted; when it isn't (the merged core-tests suite:
/// no test mounts `core.intrinsics.memory` directly), the alias
/// spelling appeared in NO name index,
/// `ArchiveBodyRemap::map_function` missed every tier, and the XMOD
/// band id froze into the rewritten body — surfacing at runtime as
/// `FunctionNotFound(0x2000_00xx)` (pinned across
/// `core-tests/runtime/{spawn,ctx_bridge,mod,config,thread}`).
///
/// Resolution is NAME-authoritative (the carried target key): the
/// same-entry fid fast path is deliberately not used here — the
/// emission drain stores PRE-remap codegen ids, so a cross-entry fid
/// can collide with an unrelated local archive id (the exact
/// ambiguity XMOD-CALL-ID-BAND-1 exists to prevent).  Runs AFTER all
/// loaded entries' kept-function names are recorded, so alias→target
/// and alias→alias chains resolve independent of entry order.
/// Bounded fixpoint: each pass installs at least one pending alias or
/// stops; chains resolve in chain-length passes; cycles terminate by
/// no-progress.  Stub-range ids are never installed (same reject
/// discipline as the remap tiers).  First-wins: an alias spelling
/// already bound (e.g. by a genuine descriptor of the same name)
/// keeps its binding.
fn install_mount_alias_archive_names(
    alias_triples: &[(String, Option<verum_vbc::module::FunctionId>, String)],
    codegen: &mut verum_vbc::codegen::VbcCodegen,
) {
    let mut pending: Vec<(&str, &str)> = alias_triples
        .iter()
        .filter(|(alias, _fid, target)| !target.is_empty() && alias != target)
        .map(|(alias, _fid, target)| (alias.as_str(), target.as_str()))
        .collect();
    if pending.is_empty() {
        return;
    }
    let trace = std::env::var("VERUM_TRACE_REMAP_FALLBACK").is_ok();
    loop {
        let mut progressed = false;
        pending.retain(|(alias, target)| {
            if codegen.lookup_archive_function_name(alias).is_some() {
                // Already bound (descriptor name or earlier alias
                // pass) — first-wins.
                return false;
            }
            let resolved = codegen
                .lookup_archive_function_name(target)
                .or_else(|| {
                    codegen
                        .ctx_mut()
                        .lookup_function(target)
                        .map(|info| info.id)
                })
                .filter(|fid| !verum_vbc::stub_ranges::is_stub_id(fid.0));
            match resolved {
                Some(fid) => {
                    if trace {
                        eprintln!(
                            "[remap-fallback] alias-install '{}' → '{}' → user_fid={}",
                            alias, target, fid.0
                        );
                    }
                    codegen.record_archive_function_name(alias, fid);
                    progressed = true;
                    false
                }
                // Possibly an alias→alias chain whose tail resolves
                // in a later pass — retry.
                None => true,
            }
        });
        if !progressed || pending.is_empty() {
            break;
        }
    }
}

/// UMBRELLA-MOUNT-PRUNE-1 — function-granular merge pruning.
///
/// Background: `register_module_filtered` allocates a codegen-local id
/// for EVERY function of a decoded archive entry (total remap — the
/// identity-fallback protection) and `merge_archive_function_bodies`
/// merges every id in the remap it is handed.  Handing it the total
/// map therefore merges each decoded entry's ENTIRE function table:
/// measured on `core-tests/meta/span/unit_test.vr`, an 83-name wanted
/// set reaches 4,898 functions but merges 46,782 (485 of 585 entries
/// decode; archive entries are per-directory subsystem bundles, e.g.
/// ALL of `core/meta/*.vr` is the single entry `core.meta`), and the
/// AOT leg then LLVM-lowers all ~47K bodies per test.
///
/// This function computes, per decoded entry, the archive-local
/// function-id KEEP set — the least set that preserves the observable
/// semantics of merge-all for code that can actually execute:
///
///  1. **Registered surface** — every filter-accepted function (it has
///     a `FunctionInfo`; without a body the finalize-time stub emitter
///     would rebind it to a silent `RetV`-Unit placeholder).
///  2. **BFS-reached names** — `SymbolGraph::reachable`'s closure from
///     the user's seeds, resolved per entry by exact descriptor name.
///  3. **`__tls_init_*` synthetics** — static-initializer ctors are
///     invoked by the runtime at startup and referenced by nothing.
///  4. **Local id closure** — for every kept body, every fn-id-carrying
///     operand (`Call`/`TailCall`/`NewClosure`/`CallG`/`GenCreate`/
///     `Spawn`) chases into the keep set: archive-local ids directly
///     (Tier-1 remap integrity — bare-name collisions make name-level
///     closure unsound for local edges), cross-module sparse ids via
///     `external_function_names` → exact-name resolution against the
///     other decoded entries (mirrors the merge's Tier-2b exact-string
///     discipline; deliberately NO leaf/suffix fanning).
///  5. **Type surface of wanted/constructed types** — runtime method
///     dispatch (`find_method_by_receiver_type`) resolves bare `CallM`
///     names against the merged table by receiver type, and the
///     runtime invokes `drop_fn`/`clone_fn` implicitly; merge-all was
///     the safety net for both.  A value of type T can only exist if
///     T is user-visible (name ∈ `wanted` — user code constructs or
///     receives it) or some kept body constructs it (`New`/`NewG`/
///     `MakeVariantTyped` operands).  For each such type this rule
///     keeps: its `ProtocolImpl.methods` vtable ids, `drop_fn` /
///     `clone_fn`, and every local function whose name shape marks it
///     as a method of T (`T.<m>` first-segment or `<mod>.T.<m>`
///     penultimate-segment — both descriptor-name promotions occur in
///     the archive).  CallM names themselves are NOT fanned: the
///     receiver's type rule covers them exactly.
///
/// The fixpoint is demand-driven: only kept bodies are decoded, so the
/// cost scales with the kept set (~5-8K functions), not the archive.
///
/// Kill switch: callers skip this entirely (and hand the merge the
/// total remaps) when `VERUM_NO_MOUNT_PRUNE=1`.
fn compute_merge_keep_sets(
    decoded: &[(String, VbcModule)],
    registered_by_entry: &HashMap<String, HashSet<u32>>,
    reached_names: &HashSet<String>,
    wanted: &HashSet<String>,
) -> HashMap<String, HashSet<u32>> {
    use verum_vbc::module::FunctionDescriptor;
    use verum_vbc::types::TypeDescriptor;

    struct EntryAux<'m> {
        module: &'m VbcModule,
        /// StringId → interned string.
        name_by_id: HashMap<StringId, &'m str>,
        /// archive-local fn id → descriptor.
        desc_by_id: HashMap<u32, &'m FunctionDescriptor>,
        /// archive-local fn id → descriptor name.
        fn_name_by_id: HashMap<u32, &'m str>,
        /// cross-module sparse id → callee name.
        external_name_by_id: HashMap<u32, &'m str>,
        /// archive-local type id → descriptor.
        type_by_id: HashMap<u32, &'m TypeDescriptor>,
    }

    let mut aux: Vec<EntryAux<'_>> = Vec::with_capacity(decoded.len());
    // Callee-name → (entry index, archive-local fn id) resolution
    // index.  First-wins in decode order, which follows archive-index
    // order — the same discipline as `SymbolGraph::qualified_to_module`
    // and the runtime's first-wins name registration.
    //
    // **Spelling completeness** (root cause of the span-suite interp
    // regressions during bring-up): a caller module's
    // `external_function_names` records the callee under the SPELLING
    // THE CALLER KNEW — commonly the fully-ROOTED canonical form
    // (`core.base.semver.semver_compare`), while the defining entry's
    // descriptor stores the relative/promoted form
    // (`base.semver.semver_compare`) or a bare `Type.method`.  Exact
    // raw-name matching therefore missed cross-entry callees whose two
    // spellings differ, the callee stayed unkept/unmerged, and the
    // caller's operand fell through every merge remap tier to the raw
    // id — observed as `Signal.name`-class misdispatch and per-test
    // global-ctor failure/slowdown.  Index every function under all
    // the spellings the resolution tiers use:
    //   1. raw descriptor name,
    //   2. the canonical merged form
    //      (`merge_module_and_simple_name(entry, raw)` — the rooted
    //      spelling callers record),
    //   3. the 2-segment `Type.method` suffix of deep promoted names.
    // Owned keys because form 2 is synthesised.
    let callm_census = std::env::var_os("VERUM_TRACE_ACCEPT").is_some();
    let mut callm_bare_edges = 0usize;
    let mut callm_bare_kept = 0usize;
    let mut callm_qualified_edges = 0usize;
    let mut callm_qualified_kept = 0usize;
    let mut callm_bare_by_name: HashMap<String, (usize, usize)> = HashMap::new();
    let mut name_to_loc: HashMap<String, (usize, u32)> = HashMap::new();
    // CALLM-KEEP-CLOSURE-1: method-DISPATCH resolution indexes.  The
    // fixpoint below follows `Call`-family edges by func-id/name, but a
    // `CallM` edge carries only a METHOD-NAME StringId and resolves at
    // RUNTIME against the receiver — invisible to a func-id closure.
    // Any body reachable ONLY through method dispatch (every
    // `implement <Proto> for <Primitive>` — `Int.fmt_debug`,
    // `Int.to_text`, `Bool.fmt`, … — plus all `dyn:Proto.method`
    // targets) was silently pruned, and the runtime's bare-suffix
    // fallback then executed an ARBITRARY same-suffix body from another
    // type (observed: `f"{n:?}"` → `Text.fmt_debug` on an Int receiver
    // → `""`).  Index every function under (a) its 2-segment
    // `Type.method` suffix and (b) its bare method name, so the CallM
    // arm of the fixpoint can keep every candidate the runtime's
    // dispatch tiers could legitimately pick.  Presence-only guarantee:
    // over-keeping is safe (the merge remap still routes dispatch);
    // under-keeping is the defect class this closes.
    let mut methods_by_suffix2: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
    let mut methods_by_bare: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
    for (idx, (_entry_name, module)) in decoded.iter().enumerate() {
        let name_by_id: HashMap<StringId, &str> =
            module.strings.iter().map(|(s, id)| (id, s)).collect();
        let mut desc_by_id: HashMap<u32, &FunctionDescriptor> =
            HashMap::with_capacity(module.functions.len());
        let mut fn_name_by_id: HashMap<u32, &str> =
            HashMap::with_capacity(module.functions.len());
        for f in &module.functions {
            desc_by_id.insert(f.id.0, f);
            if let Some(n) = name_by_id.get(&f.name) {
                fn_name_by_id.insert(f.id.0, *n);
                let segs: Vec<&str> = n.split('.').collect();
                if segs.len() >= 2 {
                    let suffix2 =
                        format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]);
                    methods_by_suffix2.entry(suffix2).or_default().push((idx, f.id.0));
                    methods_by_bare
                        .entry(segs[segs.len() - 1].to_string())
                        .or_default()
                        .push((idx, f.id.0));
                }
            }
        }
        let mut external_name_by_id: HashMap<u32, &str> =
            HashMap::with_capacity(module.external_function_names.len());
        for (fid, sid) in module.external_function_names.iter() {
            if let Some(n) = name_by_id.get(sid) {
                external_name_by_id.insert(fid.0, *n);
            }
        }
        let mut type_by_id: HashMap<u32, &TypeDescriptor> =
            HashMap::with_capacity(module.types.len());
        for t in &module.types {
            type_by_id.insert(t.id.0, t);
        }
        for (fid, n) in &fn_name_by_id {
            name_to_loc.entry((*n).to_string()).or_insert((idx, *fid));
            let canonical = merge_module_and_simple_name(_entry_name, n);
            if canonical.as_str() != *n {
                name_to_loc.entry(canonical).or_insert((idx, *fid));
            }
            // 2-segment `Type.method` suffix of deep promoted names
            // (`base.fmt.Formatter.new` → `Formatter.new`).
            let segs: Vec<&str> = n.split('.').collect();
            if segs.len() >= 3 {
                let suffix = format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]);
                name_to_loc.entry(suffix).or_insert((idx, *fid));
            }
        }
        aux.push(EntryAux {
            module,
            name_by_id,
            desc_by_id,
            fn_name_by_id,
            external_name_by_id,
            type_by_id,
        });
    }

    // STUB-STAGE-INSUITE-1: re-export alias SPELLINGS are first-class
    // callee keys.  A body's cross-module Call edge records the
    // spelling the caller knew — for a re-exported function that is
    // the alias's qualified key (`core.base.memory.memcpy` for
    // `public mount core.intrinsics.memory.memcpy` in
    // `core/base/memory.vr`); NO descriptor carries that name, so the
    // descriptor-derived index above can never chase the edge
    // (observed as `ext_resolved=false` under
    // VERUM_TRACE_MOUNT_PRUNE) and an alias-only-reachable callee
    // body gets pruned.  Index every `mount_aliases` row under its
    // alias spelling, resolved name-authoritatively through the
    // carried target key (REEXPORT-QUALIFIED-KEY-1; per-entry fid
    // renumbering makes the row's fid unreliable across entries).
    // Bounded fixpoint for alias→alias chains; cycles terminate by
    // no-progress.  `or`-semantics: a genuine descriptor spelling
    // already in the index wins.
    {
        let mut pending_aliases: Vec<(String, String)> = Vec::new();
        for (_entry_name, module) in decoded.iter() {
            for (alias_sid, _archive_fid, target_sid) in module.mount_aliases.iter() {
                let alias = match module.get_string(*alias_sid) {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };
                let target = module.get_string(*target_sid).unwrap_or("");
                if target.is_empty() || alias == target {
                    continue;
                }
                pending_aliases.push((alias.to_string(), target.to_string()));
            }
        }
        loop {
            let mut progressed = false;
            pending_aliases.retain(|(alias, target)| {
                if name_to_loc.contains_key(alias.as_str()) {
                    return false;
                }
                if let Some(loc) = name_to_loc.get(target.as_str()).copied() {
                    name_to_loc.insert(alias.clone(), loc);
                    progressed = true;
                    false
                } else {
                    true
                }
            });
            if !progressed || pending_aliases.is_empty() {
                break;
            }
        }
    }

    let mut keep: Vec<HashSet<u32>> = vec![HashSet::new(); decoded.len()];
    let mut worklist: Vec<(usize, u32)> = Vec::new();
    macro_rules! push_fn {
        ($idx:expr, $fid:expr) => {
            if keep[$idx].insert($fid) {
                worklist.push(($idx, $fid));
            }
        };
    }

    // Lazily-built per-entry index: type name → local fn ids whose
    // name shape marks them as that type's methods (`T.m` or
    // `<mod...>.T.m`).
    let mut methods_index: Vec<Option<HashMap<&str, Vec<u32>>>> =
        (0..decoded.len()).map(|_| None).collect();
    fn build_methods_index<'m>(a: &EntryAux<'m>) -> HashMap<&'m str, Vec<u32>> {
        let mut m: HashMap<&str, Vec<u32>> = HashMap::new();
        for (fid, name) in &a.fn_name_by_id {
            let segs: Vec<&str> = name.split('.').collect();
            if segs.len() >= 2 {
                m.entry(segs[0]).or_default().push(*fid);
                if segs.len() >= 3 {
                    let penult = segs[segs.len() - 2];
                    if penult != segs[0] {
                        m.entry(penult).or_default().push(*fid);
                    }
                }
            }
        }
        m
    }

    // Type-surface rule (5): vtable ids + drop/clone glue + name-shape
    // methods of a type that is user-visible or constructed by kept
    // code.  `seen_types` makes the rule idempotent per (entry, tid).
    let mut seen_types: HashSet<(usize, u32)> = HashSet::new();
    macro_rules! keep_type_surface {
        ($idx:expr, $tid:expr) => {
            if seen_types.insert(($idx, $tid)) {
                if let Some(ty) = aux[$idx].type_by_id.get(&$tid).copied() {
                    for pi in ty.protocols.iter() {
                        for raw in pi.methods.iter().copied() {
                            if aux[$idx].desc_by_id.contains_key(&raw) {
                                push_fn!($idx, raw);
                            }
                        }
                    }
                    if let Some(dfn) = ty.drop_fn {
                        if aux[$idx].desc_by_id.contains_key(&dfn) {
                            push_fn!($idx, dfn);
                        }
                    }
                    if let Some(cfn) = ty.clone_fn {
                        if aux[$idx].desc_by_id.contains_key(&cfn) {
                            push_fn!($idx, cfn);
                        }
                    }
                    if let Some(tname) = aux[$idx].name_by_id.get(&ty.name).copied() {
                        if methods_index[$idx].is_none() {
                            methods_index[$idx] = Some(build_methods_index(&aux[$idx]));
                        }
                        if let Some(map) = methods_index[$idx].as_ref()
                            && let Some(fids) = map.get(tname)
                        {
                            let fids = fids.clone();
                            for fid in fids {
                                push_fn!($idx, fid);
                            }
                        }
                    }
                }
            }
        };
    }

    // ---- seeds ----
    for (idx, (entry_name, _module)) in decoded.iter().enumerate() {
        if let Some(reg) = registered_by_entry.get(entry_name) {
            for fid in reg {
                push_fn!(idx, *fid);
            }
        }
        let named: Vec<u32> = aux[idx]
            .fn_name_by_id
            .iter()
            .filter(|(_fid, n)| {
                n.starts_with("__tls_init_") || reached_names.contains(**n)
            })
            .map(|(fid, _)| *fid)
            .collect();
        for fid in named {
            push_fn!(idx, fid);
        }
        // **Type-glue surface of EVERY imported descriptor.**  The
        // caller imports ALL type descriptors of every decoded entry
        // (`import_archive_module_types` is unconditional), and each
        // descriptor carries archive-local `drop_fn` / `clone_fn` /
        // `ProtocolImpl.methods` FUNCTION ids.  Those references
        // survive the import regardless of the mount set, and the
        // runtime invokes them IMPLICITLY (`DropRef` drop glue, clone
        // dispatch, protocol vtables) for any value of the type that
        // materialises.  Pruning a referenced glue body leaves the
        // descriptor's id DANGLING after finalize renumbering — the
        // live failure during bring-up: `DropRef` on a span-test local
        // dispatched into `SignalError.fmt_debug` (whatever body owned
        // the stale slot) and panicked in `Formatter.write_str`.
        // Glue bodies are small and bounded (one per type at most);
        // keeping them wholesale restores merge-all's implicit-dispatch
        // safety at negligible cost.
        let glue_tids: Vec<u32> = aux[idx].type_by_id.keys().copied().collect();
        for tid in glue_tids {
            if let Some(ty) = aux[idx].type_by_id.get(&tid).copied() {
                for pi in ty.protocols.iter() {
                    for raw in pi.methods.iter().copied() {
                        if aux[idx].desc_by_id.contains_key(&raw) {
                            push_fn!(idx, raw);
                        }
                    }
                }
                if let Some(dfn) = ty.drop_fn {
                    if aux[idx].desc_by_id.contains_key(&dfn) {
                        push_fn!(idx, dfn);
                    }
                }
                if let Some(cfn) = ty.clone_fn {
                    if aux[idx].desc_by_id.contains_key(&cfn) {
                        push_fn!(idx, cfn);
                    }
                }
            }
        }
        // User-visible types: user code can construct/receive values of
        // any type it names, so their FULL method surface (name-shape
        // dispatch roots on top of the glue above) is live even with
        // zero static references.
        let wanted_tids: Vec<u32> = aux[idx]
            .type_by_id
            .iter()
            .filter(|(_tid, ty)| {
                aux[idx]
                    .name_by_id
                    .get(&ty.name)
                    .is_some_and(|n| wanted.contains(*n))
            })
            .map(|(tid, _)| *tid)
            .collect();
        for tid in wanted_tids {
            keep_type_surface!(idx, tid);
        }
    }
    // STUB-STAGE-INSUITE-1: resolve seed names through the FULL
    // resolution index (canonical spellings, 2-segment suffixes,
    // mount-alias spellings) — not just raw descriptor names.  The
    // supplemental wave feeds pass-2 bodies' cross-module callee
    // names in as `reached_names`, and those are recorded under the
    // spelling the CALLER knew — frequently a re-export alias with no
    // matching descriptor.  Without this, an alias-spelled seed was
    // silently dropped and the target body stayed pruned.
    for name in reached_names.iter() {
        if let Some(&(eidx, efid)) = name_to_loc.get(name.as_str()) {
            push_fn!(eidx, efid);
        }
    }
    // USER-CALLM seed (ARCHIVE-MERGE-MISSING-FN, task #24 leg 2): the
    // user program's instance-method names arrive in `wanted` as BARE
    // names (`unwrap_err`) and static-path calls as `Type.method`
    // 2-segment keys. Neither matches an archive descriptor name
    // exactly unless some OTHER kept archive body happens to call it —
    // `Result.unwrap_err` had no archive-internal caller and was pruned
    // (its siblings unwrap/expect survived only via stdlib panic-path
    // Call edges), so the user's `.unwrap_err()` CallM degraded to
    // const-zero at AOT. Mirror the BFS CallM edge rule for the USER
    // call surface: bare keys keep every same-suffix candidate,
    // 2-segment keys keep exact suffix2 matches — presence-only
    // over-keep, dispatch precision stays the runtime's job.
    // Bare-name fanout cap: ubiquitous method names (`len`, `push`,
    // `to_text` — dozens of implementors) are already reachable through
    // archive-internal Call/CallM edges wherever they matter, and
    // keeping EVERY implementor for every user call site measurably
    // destabilized the merge (dp AOT 51→28: slower per-test compiles +
    // latent broken bodies pulled into every binary). Distinctive names
    // (`unwrap_err` — the actual defect: no archive-internal caller at
    // all) have few candidates and are kept in full.
    const BARE_KEEP_FANOUT_CAP: usize = 8;
    for name in wanted.iter() {
        let dots = name.matches('.').count();
        let locs: Option<&Vec<(usize, u32)>> = if dots == 0 {
            methods_by_bare
                .get(name.as_str())
                .filter(|l| l.len() <= BARE_KEEP_FANOUT_CAP)
        } else if dots == 1 {
            methods_by_suffix2.get(name.as_str())
        } else {
            None
        };
        if let Some(locs) = locs {
            let locs = locs.clone();
            for (eidx, fid) in locs {
                push_fn!(eidx, fid);
            }
        }
    }
    for (idx, (_entry_name, _module)) in decoded.iter().enumerate() {
        let _ = idx;
        // CALLM-KEEP-CLOSURE-1 seed: PRIMITIVE-impl method surface is
        // unconditionally live.  Primitives (Int / Float / Bool / Char /
        // Text / Byte / sized ints) carry NO TypeDescriptor in
        // `module.types`, so neither the glue rule nor the wanted-type
        // rule above can reach `implement <Proto> for Int` bodies — yet
        // user code and dyn dispatch can invoke them on any primitive
        // value with ZERO static reference inside the kept graph
        // (`f"{n:?}"` → dyn:Debug.fmt_debug → `Int.fmt_debug`).  The
        // surface is bounded (a few hundred bodies stdlib-wide).
        {
            use verum_common::well_known_types::type_names as tn;
            let prim_fids: Vec<u32> = aux[idx]
                .fn_name_by_id
                .iter()
                .filter(|(_fid, n)| {
                    let segs: Vec<&str> = n.split('.').collect();
                    if segs.len() < 2 {
                        return false;
                    }
                    let owner = segs[segs.len() - 2];
                    tn::is_integer_type(owner)
                        || tn::is_float_type(owner)
                        || matches!(owner, "Bool" | "Char" | "Text" | "Unit")
                })
                .map(|(fid, _)| *fid)
                .collect();
            for fid in prim_fids {
                push_fn!(idx, fid);
            }
        }
    }

    // ---- fixpoint ----
    // Focused diagnostics: VERUM_TRACE_MOUNT_PRUNE=<substring> prints
    // every closure decision touching a matching function name.
    let trace_needle: Option<String> = std::env::var("VERUM_TRACE_MOUNT_PRUNE").ok();
    while let Some((idx, fid)) = worklist.pop() {
        let Some(desc) = aux[idx].desc_by_id.get(&fid).copied() else {
            continue;
        };
        if let Some(needle) = trace_needle.as_deref()
            && let Some(nm) = aux[idx].fn_name_by_id.get(&fid)
            && nm.contains(needle)
        {
            eprintln!(
                "[mount-prune-trace] visit entry={} fid={} name='{}' bc_len={}",
                decoded[idx].0, fid, nm, desc.bytecode_length,
            );
        }
        let decoded_instrs: Vec<Instruction>;
        let instrs: &[Instruction] = if let Some(ref v) = desc.instructions {
            v
        } else {
            let off = desc.bytecode_offset as usize;
            let len = desc.bytecode_length as usize;
            if len == 0 || off + len > aux[idx].module.bytecode.len() {
                continue;
            }
            match verum_vbc::bytecode::decode_instructions(
                &aux[idx].module.bytecode[off..off + len],
            ) {
                Ok(v) => {
                    decoded_instrs = v;
                    &decoded_instrs
                }
                Err(_) => continue,
            }
        };
        for ins in instrs {
            match ins {
                Instruction::Call { func_id, .. }
                | Instruction::TailCall { func_id, .. }
                | Instruction::NewClosure { func_id, .. }
                | Instruction::CallG { func_id, .. }
                | Instruction::GenCreate { func_id, .. }
                | Instruction::Spawn { func_id, .. } => {
                    if let Some(needle) = trace_needle.as_deref() {
                        let callee_desc = aux[idx]
                            .fn_name_by_id
                            .get(func_id)
                            .copied()
                            .or_else(|| aux[idx].external_name_by_id.get(func_id).copied())
                            .unwrap_or("<unknown>");
                        if callee_desc.contains(needle) {
                            eprintln!(
                                "[mount-prune-trace] edge entry={} caller_fid={} -> callee_id={} callee='{}' local={} ext_resolved={}",
                                decoded[idx].0,
                                fid,
                                func_id,
                                callee_desc,
                                aux[idx].desc_by_id.contains_key(func_id),
                                aux[idx]
                                    .external_name_by_id
                                    .get(func_id)
                                    .map(|n| name_to_loc.contains_key(*n))
                                    .unwrap_or(false),
                            );
                        }
                    }
                    if aux[idx].desc_by_id.contains_key(func_id) {
                        push_fn!(idx, *func_id);
                    } else if let Some(name) =
                        aux[idx].external_name_by_id.get(func_id).copied()
                    {
                        // Exact spelling first; then the query's own
                        // 2-segment suffix (rooted caller spelling vs
                        // short descriptor spelling — the mirror image
                        // of the index-side suffix key).  First-wins
                        // homonym resolution over-keeps at worst — the
                        // merge remap still picks the real dispatch
                        // target; the keep set only guarantees the
                        // body is PRESENT.
                        let loc = name_to_loc.get(name).copied().or_else(|| {
                            let segs: Vec<&str> = name.split('.').collect();
                            if segs.len() >= 3 {
                                name_to_loc
                                    .get(
                                        format!(
                                            "{}.{}",
                                            segs[segs.len() - 2],
                                            segs[segs.len() - 1]
                                        )
                                        .as_str(),
                                    )
                                    .copied()
                            } else {
                                None
                            }
                        });
                        if let Some((eidx, efid)) = loc {
                            push_fn!(eidx, efid);
                        }
                    }
                }
                Instruction::New { type_id, .. }
                | Instruction::NewG { type_id, .. }
                | Instruction::MakeVariantTyped { type_id, .. } => {
                    keep_type_surface!(idx, *type_id);
                }
                // CALLM-KEEP-CLOSURE-1: method-dispatch edge.  `method_id`
                // is a STRING-table id naming the dispatch key — either
                // qualified (`Text.replace`, rooted spellings) or bare /
                // protocol-dynamic (`fmt_debug`, `dyn:Debug.fmt_debug`).
                // The runtime resolves it against the receiver's RUNTIME
                // type, so the keep set must contain every candidate the
                // dispatch tiers could pick: exact 2-segment matches for
                // qualified keys, all same-suffix methods for bare keys.
                // Presence-only over-keep — dispatch precision is the
                // runtime's job; ABSENCE is what mis-routed
                // `f"{n:?}"` onto `Text.fmt_debug`.
                Instruction::CallM { method_id, .. } => {
                    if let Some(raw) =
                        aux[idx].name_by_id.get(&StringId(*method_id)).copied()
                    {
                        let name = raw
                            .strip_prefix("dyn:")
                            .or_else(|| raw.strip_prefix("ctx:"))
                            .unwrap_or(raw);
                        let locs: Option<&Vec<(usize, u32)>> = if name.contains('.') {
                            let segs: Vec<&str> = name.split('.').collect();
                            let suffix2 = format!(
                                "{}.{}",
                                segs[segs.len() - 2],
                                segs[segs.len() - 1]
                            );
                            methods_by_suffix2.get(&suffix2)
                        } else {
                            methods_by_bare.get(name)
                        };
                        if let Some(locs) = locs {
                            // CALLM-KEEP CENSUS (`VERUM_TRACE_ACCEPT=1`):
                            // the bare arm keeps EVERY same-named method
                            // in the archive, so its share of the keep
                            // set is the price of identity-by-simple-name
                            // at the merge layer. Counted, not guessed.
                            if callm_census {
                                if name.contains('.') {
                                    callm_qualified_edges += 1;
                                    callm_qualified_kept += locs.len();
                                } else {
                                    callm_bare_edges += 1;
                                    callm_bare_kept += locs.len();
                                    let e = callm_bare_by_name
                                        .entry(name.to_string())
                                        .or_insert((0usize, 0usize));
                                    e.0 += 1;
                                    e.1 += locs.len();
                                }
                            }
                            let locs = locs.clone();
                            for (eidx, efid) in locs {
                                push_fn!(eidx, efid);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if callm_census {
        // COUNTERFACTUAL (T0753): the type-surface rule already keeps
        // every protocol-impl method of a type whose surface is kept, so
        // the bare-CallM arm only ADDS methods of types whose surface is
        // NOT kept.  For those the program cannot hold a receiver — unless
        // the type-surface rule has a coverage gap.  Which of the two it
        // is decides whether the over-keep is removable, so count it
        // rather than argue it.
        let mut kept_type_names: HashSet<&str> = HashSet::new();
        for (eidx, tid) in seen_types.iter() {
            if let Some(ty) = aux[*eidx].type_by_id.get(tid) {
                if let Some(n) = aux[*eidx].name_by_id.get(&ty.name) {
                    kept_type_names.insert(*n);
                }
            }
        }
        let (mut owner_kept, mut owner_absent, mut no_owner) = (0usize, 0usize, 0usize);
        for (eidx, fids) in keep.iter().enumerate() {
            for fid in fids {
                let n = match aux[eidx].fn_name_by_id.get(fid) {
                    Some(n) => *n,
                    None => continue,
                };
                let segs: Vec<&str> = n.split('.').collect();
                if segs.len() < 2 {
                    no_owner += 1;
                } else if kept_type_names.contains(segs[segs.len() - 2]) {
                    owner_kept += 1;
                } else {
                    owner_absent += 1;
                }
            }
        }
        eprintln!(
            "[callm-keep] kept types={} | kept fns by owner: surface-kept={} surface-ABSENT={} no-owner={}",
            kept_type_names.len(),
            owner_kept,
            owner_absent,
            no_owner,
        );
        // DECODE-GRANULARITY (T0753): if the walk keeps nearly every
        // function in every entry it decodes, then the merge size is
        // set by WHICH ENTRIES get decoded — the archive's bundling —
        // and not by any rule inside this fixpoint.
        for (eidx, (entry_name, module)) in decoded.iter().enumerate() {
            let kept = keep[eidx].len();
            let tot = module.functions.len();
            if tot > 0 {
                eprintln!(
                    "[decoded] {:<34} kept {}/{} ({}%)",
                    entry_name,
                    kept,
                    tot,
                    kept * 100 / tot,
                );
            }
        }
        let total: usize = keep.iter().map(|k| k.len()).sum();
        eprintln!(
            "[callm-keep] bare: {} edges -> {} keeps | qualified: {} edges -> {} keeps | keep-set total {}",
            callm_bare_edges,
            callm_bare_kept,
            callm_qualified_edges,
            callm_qualified_kept,
            total,
        );
        let mut rows: Vec<(&String, &(usize, usize))> =
            callm_bare_by_name.iter().collect();
        rows.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        for (name, (edges, keeps)) in rows.into_iter().take(15) {
            eprintln!(
                "[callm-keep]   bare `{}`: {} edge(s) -> {} keeps",
                name, edges, keeps
            );
        }
    }
    decoded
        .iter()
        .enumerate()
        .map(|(idx, (entry_name, _))| {
            (entry_name.clone(), std::mem::take(&mut keep[idx]))
        })
        .collect()
}

/// ARCHIVE-TYPE-GLUE-IDS-1: rewrite a decoded archive module's
/// TYPE-descriptor `drop_fn` / `clone_fn` references from
/// ARCHIVE-LOCAL fn ids to the ctx FunctionIds the loader allocated
/// for this module, BEFORE the descriptors are imported.
///
/// `import_archive_module_types` / `import_archive_type` copy these
/// fields VERBATIM — i.e. as ARCHIVE-LOCAL fn ids — and the finalize
/// pass later remaps the values through its ctx→final table as if
/// they had been ctx ids all along.  The two id spaces are unrelated,
/// so on the merge-all path every imported type's drop/clone glue has
/// ALWAYS dispatched to an arbitrary (id-coincident) function — an
/// effective no-op that the dense 46K-function table kept benign and
/// stable.  Pruning compacted the final table and turned the same
/// stale ids into loud misdispatch (`DropRef` on a span-test local
/// executing `Signal.fmt` → `Formatter.write_str` on Unit → panic);
/// the bring-up stopgap cleared both fields (deterministic no-glue).
///
/// The proper translation implemented here: `func_id_remap` (from
/// `register_module_filtered`) is TOTAL over this module's own
/// function table — id allocation precedes the registration filter —
/// so an archive-local glue id resolves to exactly the ctx id whose
/// body the keep-set closure guarantees is merged
/// (`compute_merge_keep_sets` seeds every descriptor's `drop_fn` /
/// `clone_fn` — see "Type-glue surface of EVERY imported
/// descriptor").  Finalize then remaps ctx→final and `DropRef`
/// executes the REAL glue body.
///
/// Fallback: a glue id with NO remap entry references a function
/// whose body does not live in this module (a cross-module Drop impl
/// left as a precompile-global sparse id by the bake's per-module
/// finalize).  There is no ctx binding to route it through — clear
/// the field so the runtime takes its no-glue default instead of the
/// id-roulette.
///
/// `ProtocolImpl.methods` are left untouched: their consumer
/// validates by name before dispatching, so stale ids fall through
/// harmlessly there.  Gated on pruning so the kill-switch path stays
/// bit-identical.
///
/// `VERUM_TRACE_GLUE_REMAP=1` lists every glue rewrite/clear
/// (entry, type, old→new id, archive-side fn name) for per-type
/// bisection when activating real glue surfaces a latent drop/clone
/// body bug.
fn remap_type_glue_fn_ids(
    module: &mut VbcModule,
    func_id_remap: &HashMap<u32, verum_vbc::module::FunctionId>,
) {
    let trace = std::env::var("VERUM_TRACE_GLUE_REMAP").is_ok();
    // Split field borrows: the trace needs `strings` / `functions`
    // (immutable) while `types` is mutated.
    let VbcModule {
        name,
        strings,
        types,
        functions,
        ..
    } = module;
    for ty in types.iter_mut() {
        let (old_drop, old_clone) = (ty.drop_fn, ty.clone_fn);
        if old_drop.is_none() && old_clone.is_none() {
            continue;
        }
        ty.drop_fn = old_drop.and_then(|f| func_id_remap.get(&f).map(|fid| fid.0));
        ty.clone_fn = old_clone.and_then(|f| func_id_remap.get(&f).map(|fid| fid.0));
        if trace {
            let fn_name = |fid: Option<u32>| -> &str {
                fid.and_then(|f| {
                    functions
                        .iter()
                        .find(|d| d.id.0 == f)
                        .and_then(|d| strings.get(d.name))
                })
                .unwrap_or("-")
            };
            eprintln!(
                "[GLUE-REMAP] entry={} type={} drop {:?}→{:?} ({}) clone {:?}→{:?} ({})",
                name,
                strings.get(ty.name).unwrap_or("?"),
                old_drop,
                ty.drop_fn,
                fn_name(old_drop),
                old_clone,
                ty.clone_fn,
                fn_name(old_clone),
            );
        }
    }
}

/// UMBRELLA-MOUNT-PRUNE-1: harvest a decoded module's cross-module
/// callee names — the exact strings its bodies dispatch through
/// (`external_function_names` resolution for Call-family fn-id
/// operands, the string table for `CallM` method names).  The pass-2
/// supplemental wave feeds these back into the primary pass's keep
/// closure as extra exact-match seeds.
fn collect_cross_module_callee_names(module: &VbcModule, out: &mut HashSet<String>) {
    let name_by_id: HashMap<StringId, &str> =
        module.strings.iter().map(|(s, id)| (id, s)).collect();
    let local_ids: HashSet<u32> = module.functions.iter().map(|f| f.id.0).collect();
    let mut external_name_by_id: HashMap<u32, &str> =
        HashMap::with_capacity(module.external_function_names.len());
    for (fid, sid) in module.external_function_names.iter() {
        if let Some(n) = name_by_id.get(sid) {
            external_name_by_id.insert(fid.0, *n);
        }
    }
    for f in &module.functions {
        let owned_decode: Vec<Instruction>;
        let instrs: &[Instruction] = if let Some(ref v) = f.instructions {
            v
        } else {
            let off = f.bytecode_offset as usize;
            let len = f.bytecode_length as usize;
            if len == 0 || off + len > module.bytecode.len() {
                continue;
            }
            match verum_vbc::bytecode::decode_instructions(&module.bytecode[off..off + len]) {
                Ok(v) => {
                    owned_decode = v;
                    &owned_decode
                }
                Err(_) => continue,
            }
        };
        for ins in instrs {
            match ins {
                Instruction::Call { func_id, .. }
                | Instruction::TailCall { func_id, .. }
                | Instruction::NewClosure { func_id, .. }
                | Instruction::CallG { func_id, .. }
                | Instruction::GenCreate { func_id, .. }
                | Instruction::Spawn { func_id, .. } => {
                    if !local_ids.contains(func_id)
                        && let Some(n) = external_name_by_id.get(func_id)
                    {
                        out.insert((*n).to_string());
                    }
                }
                Instruction::CallM { method_id, .. } => {
                    if let Some(n) = name_by_id.get(&StringId(*method_id)) {
                        out.insert((*n).to_string());
                    }
                }
                _ => {}
            }
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
/// Seed `wanted` with every nominal name a PATTERN references —
/// variant tags (`Continue(v)`), their qualifier types
/// (`ControlFlow.Continue`), record-pattern paths and nested
/// sub-patterns. Match ARMS were the gap (#47 runtime leg): the
/// expr-walker harvested scrutinee/guard/body but not the arm
/// patterns, so a file that only MATCHES an archive sum type
/// (`match r.branch() { Continue(v) => .. }`) never seeded
/// `ControlFlow`/`Continue`; Pass 4 skipped the ctor registration
/// and pattern-bind payload typing found no template → the
/// tag-as-index fallback typed `v` with the WRONG generic arg.
fn harvest_names_in_pattern(
    pat: &verum_ast::pattern::Pattern,
    out: &mut Harvest,
) {
    use verum_ast::pattern::{PatternKind, VariantPatternData};
    match &pat.kind {
        PatternKind::Variant { path, data } => {
            for seg in path.segments.iter() {
                if let verum_ast::ty::PathSegment::Name(id) = seg {
                    out.insert(id.name.to_string());
                }
            }
            if path.segments.len() >= 2 {
                let dotted: Vec<&str> = path
                    .segments
                    .iter()
                    .filter_map(|s| match s {
                        verum_ast::ty::PathSegment::Name(id) => Some(id.name.as_str()),
                        _ => None,
                    })
                    .collect();
                out.insert(dotted.join("."));
            }
            if let verum_common::Maybe::Some(data) = data {
                match data {
                    VariantPatternData::Tuple(pats) => {
                        for p in pats.iter() {
                            harvest_names_in_pattern(p, out);
                        }
                    }
                    VariantPatternData::Record { fields, .. } => {
                        for f in fields.iter() {
                            if let Some(ref p) = f.pattern {
                                harvest_names_in_pattern(p, out);
                            }
                        }
                    }
                }
            }
        }
        PatternKind::Record { path, fields, .. } => {
            for seg in path.segments.iter() {
                if let verum_ast::ty::PathSegment::Name(id) = seg {
                    out.insert(id.name.to_string());
                }
            }
            for f in fields.iter() {
                if let Some(ref p) = f.pattern {
                    harvest_names_in_pattern(p, out);
                }
            }
        }
        PatternKind::Tuple(pats) | PatternKind::Array(pats) | PatternKind::Or(pats) | PatternKind::And(pats) => {
            for p in pats.iter() {
                harvest_names_in_pattern(p, out);
            }
        }
        PatternKind::Reference { inner, .. } | PatternKind::Paren(inner) => {
            harvest_names_in_pattern(inner, out);
        }
        PatternKind::Guard { pattern, .. } => harvest_names_in_pattern(pattern, out),
        _ => {}
    }
}

/// What the AST harvest produces: names, and the subset of them that
/// came from a BARE method call.
///
/// The distinction is load-bearing for the symbol-graph seed expansion
/// (T0753).  `xs.new()` harvests two things — the qualified
/// `List.new`, and the bare `new`.  The bare one is what the merge
/// keep-set needs (a `CallM` edge carries only a method name, so
/// presence-only over-keep is the rule there).  Feeding it to the
/// graph walk as a SEED is a different matter: the seed expands a leaf
/// to EVERY same-named qualified in the library, so one `xs.new()`
/// enqueues `TlsClient.new`, `RedisClient.new`, `Database.new` … and
/// the walk then follows each of their call graphs.
///
/// Measured on `let mut xs: List<Int> = List.new(); xs.push(1); …`:
/// the closure is 2540 symbols across 188 archive entries — TLS 1.3
/// handshake, QUIC, Redis, Postgres among them — and the chain from
/// the provenance trace starts at `TlsClient.new` AS A SEED, with no
/// edge leading to it.  The same program with the `.new` call removed
/// reaches 291 symbols in 73 entries and costs 17.4 G instructions
/// against 42.1 G.
///
/// This is the case the fanout cap already handles INSIDE the walk
/// ("a bare method call is resolved by the RECEIVER's concrete runtime
/// type … blanket-fanning a bare leaf to every same-named impl is
/// redundant for correctness"), and the seed expansion is where that
/// reasoning was never applied.
#[derive(Default)]
pub(crate) struct Harvest {
    /// Every name the archive filters consult.
    names: std::collections::HashSet<String>,
    /// The subset harvested from a bare method call.
    bare_methods: std::collections::HashSet<String>,
}

impl Harvest {
    fn insert(&mut self, name: String) -> bool {
        self.names.insert(name)
    }

    /// Record a name that came from a bare method call.  It still
    /// joins `names` — every consumer that filters on the wanted set
    /// keeps seeing it; only the graph seed expansion reads the
    /// distinction.
    fn insert_bare_method(&mut self, name: String) {
        self.names.insert(name.clone());
        self.bare_methods.insert(name);
    }
}

fn collect_referenced_function_names(
    item: &verum_ast::Item,
    out: &mut Harvest,
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
    out: &mut Harvest,
) {
    use verum_common::Maybe;
    use verum_ast::decl::{FunctionBody, FunctionParamKind};

    // Harvest into a local set so LOCAL BINDINGS can be subtracted before the
    // names reach `out` (T0738).
    //
    // A use of `v` inside a function whose body binds `v` refers to that
    // binding — that is what scoping means, so such a name is not a request
    // for an archive symbol. Left in, it was one: the unqualified-wanted scan
    // decodes all 574 archive modules per name, and the module a program
    // compiles to depended on what its variables were CALLED —
    //
    //     let zzz: Int = 3; print(zzz);   12604 functions
    //     let fd:  Int = 3; print(fd);    48575
    //     let data: Int = 3; print(data); 62980
    //
    // — because `fd` and `data` happen to exist as leaf names somewhere in
    // the stdlib while `zzz` does not.
    //
    // Scope is approximated by the whole function rather than tracked block
    // by block. The case that costs: a name bound somewhere in the body AND
    // genuinely referring to a stdlib symbol elsewhere in the same body. It
    // fails LOUDLY (an undefined-name diagnostic), never silently.
    let mut local = Harvest::default();
    let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();

    for param in func.params.iter() {
        if let FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
            harvest_names_in_type(ty, &mut local);
            collect_binding_names_in_pattern(pattern, &mut bound);
        }
    }
    if let Maybe::Some(ret) = &func.return_type {
        harvest_names_in_type(ret, &mut local);
    }
    if let Maybe::Some(body) = &func.body {
        match body {
            FunctionBody::Block(block) => {
                harvest_names_in_block(block, &mut local);
                collect_bound_names_in_block(block, &mut bound);
            }
            FunctionBody::Expr(expr) => harvest_names_in_expr(expr, &mut local),
        }
    }
    for name in local.names {
        if !bound.contains(&name) {
            // A bare method name survives the local-binding filter as
            // a bare method name: `xs.push(1)` harvests `push`, and
            // dropping the provenance here would put it back into the
            // unrestricted seed set.
            if local.bare_methods.contains(&name) {
                out.insert_bare_method(name);
            } else {
                out.insert(name);
            }
        }
    }
}

/// Collect the identifiers a pattern BINDS.
///
/// Deliberately not `harvest_names_in_pattern`: that one gathers the TYPE and
/// variant names a pattern mentions, and has no arm for `PatternKind::Ident`
/// at all — reusing it left the subtraction below inert, which the module-size
/// measurement caught before the claim was made (T0738).
fn collect_binding_names_in_pattern(
    pat: &verum_ast::pattern::Pattern,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::pattern::PatternKind;
    match &pat.kind {
        PatternKind::Ident { name, subpattern, .. } => {
            out.insert(name.name.to_string());
            if let verum_common::Maybe::Some(inner) = subpattern {
                collect_binding_names_in_pattern(inner, out);
            }
        }
        PatternKind::Tuple(pats) | PatternKind::Array(pats) | PatternKind::Or(pats)
        | PatternKind::And(pats) => {
            for p in pats.iter() {
                collect_binding_names_in_pattern(p, out);
            }
        }
        PatternKind::Reference { inner, .. } | PatternKind::Paren(inner) => {
            collect_binding_names_in_pattern(inner, out)
        }
        PatternKind::Guard { pattern, .. } => collect_binding_names_in_pattern(pattern, out),
        _ => {}
    }
}

/// Collect the names a block BINDS — `let` patterns, including the ones in
/// nested blocks — so `harvest_names_in_function` can subtract them (T0738).
fn collect_bound_names_in_block(
    block: &verum_ast::expr::Block,
    out: &mut std::collections::HashSet<String>,
) {
    use verum_ast::StmtKind;
    for stmt in block.stmts.iter() {
        match &stmt.kind {
            StmtKind::Let { pattern, .. } => collect_binding_names_in_pattern(pattern, out),
            StmtKind::LetElse { pattern, .. } => collect_binding_names_in_pattern(pattern, out),
            _ => {}
        }
    }
}

fn harvest_names_in_impl(
    impl_decl: &verum_ast::decl::ImplDecl,
    out: &mut Harvest,
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
    out: &mut Harvest,
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
    out: &mut Harvest,
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
    out: &mut Harvest,
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
            // INSTANCE-METHOD keep seed (ARCHIVE-MERGE-MISSING-FN /
            // task #24 leg 2): `r.unwrap_err()` on a VARIABLE receiver
            // harvested nothing, so an archive method with no archive-
            // internal static caller (`Result.unwrap_err` — its
            // siblings unwrap/expect survive only through OTHER stdlib
            // bodies' Call edges) was pruned from the merge keep set;
            // the AOT CallM then degraded to const-zero and `e = 0`
            // matched `Empty` by the null path (historical stale-green
            // in every error-path probe). Seed the BARE method name —
            // the keep closure's CALLM rule already resolves bare keys
            // to every same-suffix candidate ("presence-only over-keep;
            // dispatch precision is the runtime's job").
            out.insert_bare_method(method.name.to_string());
            harvest_names_in_expr(receiver, out);
            for ga in type_args.iter() {
                harvest_names_in_generic_arg(ga, out);
            }
            for a in args.iter() {
                harvest_names_in_expr(a, out);
            }
        }
        ExprKind::Field { expr, field } => {
            // Harvest `<base>.<field>` for associated-const access like
            // `Int.MIN` — mirrors the MethodCall arm's `Type.method`
            // harvest above. Without this, a `Type.CONST` field access is
            // NEVER added to the lazy archive load's `wanted` set, so the
            // const's archive entry is never fetched; the use site then
            // falls through to the garbage-tag variant-ctor synthesis in
            // `compile_field_access` (observed: `Int.MIN` resolving to the
            // unrelated SQL-lexer `KwAll` variant, its tag shifting with
            // the interned-string layout). Harvest both the qualified
            // `Type.CONST` form and the bare `CONST` simple alias.
            if let ExprKind::Path(path) = &expr.kind
                && let Some(last) = last_path_name(path)
            {
                out.insert(format!("{}.{}", last, field.name));
                out.insert(field.name.to_string());
            }
            harvest_names_in_expr(expr, out);
        }
        ExprKind::OptionalChain { expr, .. }
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
                harvest_names_in_pattern(&arm.pattern, out);
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
        // §11 close — f-strings and tagged literals: every embedded
        // expression in `f"…{expr}…"` (or any handler-prefixed
        // interpolation) MUST contribute its referenced names to
        // the archive-load wanted-set.  Pre-fix the catch-all below
        // silently dropped InterpolatedString, so a user file whose
        // only reference to a stdlib free function is inside an
        // f-string (e.g. `let s = f"{format_debug(&x)}";`, or the
        // §J `f"{x:?}"` lowering that wraps the expr in
        // `format_debug(&expr)`) would lazy-load NEITHER the
        // function's module NOR the function descriptor —
        // user-code compilation then failed with
        // `UndefinedFunction("format_debug")` even though
        // `format_debug` was reachable via the prelude.
        //
        // Walking every embedded expression closes the entire
        // class of "function only referenced inside an interpolation"
        // failures (Format-, Debug-, Display-related lazy-load
        // misses).
        ExprKind::InterpolatedString { exprs, .. } => {
            for e in exprs.iter() {
                harvest_names_in_expr(e, out);
            }
        }
        // Tensor / map / set / array literals are sequences of
        // expressions — recurse for completeness.  An expression
        // that's only referenced inside such a literal should still
        // seed the wanted-set.
        ExprKind::TensorLiteral { data, .. } => {
            harvest_names_in_expr(data, out);
        }
        ExprKind::MapLiteral { entries } => {
            for entry in entries.iter() {
                harvest_names_in_expr(&entry.0, out);
                harvest_names_in_expr(&entry.1, out);
            }
        }
        ExprKind::SetLiteral { elements } => {
            for e in elements.iter() {
                harvest_names_in_expr(e, out);
            }
        }
        ExprKind::Array(arr) => match arr {
            verum_ast::expr::ArrayExpr::List(items) => {
                for e in items.iter() {
                    harvest_names_in_expr(e, out);
                }
            }
            verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                harvest_names_in_expr(value, out);
                harvest_names_in_expr(count, out);
            }
        },
        // Other expression forms (generators, async-builders, …)
        // are walked best-effort — over-inclusion is harmless.
        _ => {}
    }
}

fn harvest_names_in_path(
    path: &verum_ast::ty::Path,
    out: &mut Harvest,
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
    out: &mut Harvest,
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
    out: &mut Harvest,
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
    out: &mut Harvest,
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
            // Cog-prefix-stripped form: when the user writes
            // `mount core.sys.bitfield;`, the precompiler stores
            // function descriptor names in the `module sys.bitfield;`-
            // declared form (`sys.bitfield.USIZE_BITS`), which has
            // NO `core.` prefix because `core` is the cog name and
            // the file's `module` declaration scopes within the cog.
            // The archive's `register_module_filtered` then checks
            // `wanted.contains(simple_name_str)` — without the
            // stripped form here, the wholesale-mount + method-of-
            // wanted-type gates miss the grandparent-bundled case
            // (every `.vr` file under `core/sys/` folded into
            // archive entry `core.sys`, each with its own
            // `module sys.<X>;` declaration). Stripping the leading
            // cog segment (`core` in stdlib, the project cog name
            // for user code) lets the filter recognise these.
            if full.len() >= 2 {
                let stripped = full[1..].join(".");
                if !stripped.is_empty() {
                    out.insert(stripped);
                }
            }
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
        MountTreeKind::Glob(path) => {
            // FUNDAMENTAL: `mount X.Y.*;` is a wholesale-module mount —
            // every public symbol of `X.Y` (and its mod.vr re-exports)
            // becomes available unqualified in the consumer's scope.
            //
            // Previously this arm was a silent no-op, so `mount
            // core.prelude.*;` (the canonical idiom for stdlib access
            // from user code) contributed NOTHING to the wanted set.
            // The loader's per-function `is_method_of_wanted_type`
            // filter then rejected protocol-impl methods like
            // `Chunks.next` because their carrier-type leaf (`Chunks`)
            // was absent from wanted — even though prelude re-exported
            // Chunks via the collections module chain
            // (`collections/mod.vr:70 public mount .slice.Chunks`).
            //
            // The architectural fix: insert the glob's source-module
            // dotted path into `wanted` so the loader's wholesale-mount
            // gate (`is_wholesale_module_mount = wanted.contains
            // (module_name)`) at the function-registration site fires
            // for every archive entry whose name starts with this
            // prefix.  Mirror the cog-prefix-stripped form (every
            // stdlib archive entry is registered without the `core.`
            // cog prefix — `sys.bitfield.X` etc.).
            //
            // Reachability of these symbols is handled separately by
            // `stdlib_reachability.rs::walk_tree`, which already
            // records the glob's source module for BFS expansion; this
            // fix closes the consumer-side wanted-set defect.
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
            out.insert(full.join("."));
            if full.len() >= 2 {
                let stripped = full[1..].join(".");
                if !stripped.is_empty() {
                    out.insert(stripped);
                }
            }
        }
        MountTreeKind::File { .. } => {}
    }
}

/// Register only those FunctionInfo entries whose simple or
/// qualified name appears in `wanted`.  Parallel to
/// `register_module` but with name-set filtering.
///
/// Returns `(func_id_remap, registered_ids)`:
///
/// * `func_id_remap` — TOTAL over every archive-local function id in
///   this module (id allocation happens BEFORE the per-function
///   filter; see the identity-fallback rationale at the allocation
///   site). Callers that merge bodies decide how much of this map to
///   hand to `merge_archive_function_bodies` — the merge-pruning leg
///   (UMBRELLA-MOUNT-PRUNE-1) passes a keep-closure-filtered view,
///   the kill-switch path passes it whole.
/// * `registered_ids` — the archive-local ids of exactly those
///   functions the filter ACCEPTED (a `FunctionInfo` now exists in
///   `ctx.functions` for them). This is the merge pruner's "surface"
///   seed set: every registered function must merge a real body or
///   the finalize-time stub emitter would silently rebind it to a
///   `RetV`-Unit placeholder.
fn register_module_filtered(
    module: &VbcModule,
    module_name: &str,
    ctx: &mut CodegenContext,
    wanted: &std::collections::HashSet<String>,
    next_id: &mut u32,
) -> (
    std::collections::HashMap<u32, verum_vbc::module::FunctionId>,
    std::collections::HashSet<u32>,
) {
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
    // T0706 — CANONICAL-SUFFIX acceptance for two-segment wanted names.
    // A supplemental wanted entry spells the DISPATCH form
    // (`UInt8.to_hex`), while the descriptor canonicalises to the
    // promoted module chain (`core.base.primitives.UInt8.to_hex`) —
    // none of the positional parent arms below match (first-dot
    // `base`, last-dot `…UInt8`, second-to-last needs bare `UInt8`
    // in wanted, leaf needs bare `to_hex`).  Accept when the
    // qualified name's LAST TWO segments equal a dotted wanted entry
    // — O(1) per function via this precomputed set (measured miss:
    // the whole to_hex family sat in wanted with graph_module=core.base
    // yet registered=false; by-example 14-cbgr dies on it).
    let last2_wanted: std::collections::HashSet<&str> = wanted
        .iter()
        .filter(|w| w.bytes().filter(|b| *b == b'.').count() == 1)
        .map(|w| w.as_str())
        .collect();
    fn last2_of(s: &str) -> Option<&str> {
        let last = s.rfind('.')?;
        match s[..last].rfind('.') {
            Some(prev) => Some(&s[prev + 1..]),
            None => Some(s),
        }
    }

    let mut type_id_to_name: HashMap<TypeId, String> = HashMap::new();
    for ty in &module.types {
        if let Some(name) = lookup(ty.name) {
            type_id_to_name.insert(ty.id, name.to_string());
        }
    }
    // Task #25 — qualified `<parent>.<variant>` indexing, mirror of
    // the apply_lazy_with_types loader site above.
    let mut variant_index: HashMap<String, VariantHit> = HashMap::new();
    let mut variant_index_qualified: HashMap<String, VariantHit> = HashMap::new();
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
            let hit = VariantHit {
                parent_type_name: parent_name.clone(),
                tag: variant.tag,
                kind: variant.kind,
                payload_field_types,
                arity: variant.arity as usize,
            };
            let qualified_key = format!("{}.{}", parent_name, vname);
            variant_index_qualified.insert(qualified_key, hit.clone());
            variant_index.entry(vname).or_insert(hit);
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
    // UMBRELLA-MOUNT-PRUNE-1: archive-local ids of filter-ACCEPTED
    // functions — the merge pruner's surface seed set.
    let mut registered_ids: std::collections::HashSet<u32> =
        std::collections::HashSet::new();
    // ACCEPT-ARM CENSUS (`VERUM_TRACE_ACCEPT=1`).  Six independent arms
    // can admit a function, and the merged module is two orders of
    // magnitude larger than the reachability closure says it needs, so
    // the question "which arm admits the bulk" has to be READ, not
    // reasoned about.  Counted in declaration order, first true wins, so
    // the columns sum to the module's accepted total.
    let trace_accept = std::env::var_os("VERUM_TRACE_ACCEPT").is_some();
    let mut arm_counts = [0usize; 6];
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
        // Canonical-name synthesis (closes the path-doubling family of
        // bugs, including task #21 "free-fn name collision in mount
        // resolution" and the bitfield/tls13 test_bit collision):
        //
        // The descriptor name is whatever the precompiler stored — which
        // depends on whether the source file declared a `module X.Y;`
        // header AND on whether that header was rooted (`module
        // core.async.future;` → fully qualified) or relative (`module
        // sys.bitfield;` → relative to the archive entry).  The user's
        // codegen invariably looks the function up under its CANONICAL
        // form: cog-prefix + entry-path + per-file-submodule + leaf.
        //
        // Three shapes need to round-trip to the same canonical key:
        //
        //   1. Bare leaf, no submodule header. Example:
        //        archive entry: `core.text`,
        //        descriptor   : `new` (for `core/text/text.vr` declaring
        //                       no submodule directive but with a `Text`
        //                       impl block adding a `new` method).
        //        canonical    : `core.text.new`
        //        (just `<module_name>.<simple_name>`)
        //
        //   2. Relative submodule descriptor. Example:
        //        archive entry: `core.sys`,
        //        descriptor   : `sys.bitfield.test_bit` (file declares
        //                       `module sys.bitfield;`),
        //        canonical    : `core.sys.bitfield.test_bit`
        //        (overlap-merge: descriptor's leading `sys` is the same
        //         as `module_name`'s trailing `sys` — skip the overlap)
        //
        //   3. Fully-rooted submodule descriptor. Example:
        //        archive entry: `core.async`,
        //        descriptor   : `core.async.future.ready` (file declares
        //                       `module core.async.future;`),
        //        canonical    : `core.async.future.ready`
        //        (overlap-merge: descriptor's leading `core.async`
        //         matches all of `module_name` — full overlap, drop
        //         `module_name` entirely)
        //
        // Unified rule: find the longest suffix of `module_name`'s
        // segments that matches a prefix of `simple_name_str`'s
        // segments; emit `module_name[..non_overlap] + simple_name`.
        // When the descriptor is a bare leaf (no dots), no overlap is
        // possible — falls through to the simple `module_name.simple`
        // form, identical to case (1).
        let qualified_borrowed: String =
            merge_module_and_simple_name(module_name, simple_name_str);
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
        // Two-arm parent check:
        //
        //  (i)  *First-dot* parent — the classic `<Type>.<method>`
        //       shape where simple_name encodes a single-segment
        //       carrier type (`Path.new` for a `mount core.io.path.Path`
        //       declaration). Wanted contains the carrier name `Path`.
        //
        //  (ii) *Last-dot* parent — the precompiler's descriptor-name-
        //       promoted shape where simple_name is fully module-
        //       qualified (`sys.bitfield.USIZE_BITS` for a function
        //       declared in a file whose `module sys.bitfield;` header
        //       brings the bitfield submodule into the `core.sys`
        //       archive entry). Wanted must contain `sys.bitfield`
        //       (the cog-stripped form added by `collect_mount_names`)
        //       OR `core.sys.bitfield` (the literal mount path —
        //       checked via `module_name.<simple>.starts_with(W)`
        //       for completeness).
        let is_method_of_wanted_type = {
            let first_dot = simple_name_str.find('.').map(|i| &simple_name_str[..i]);
            let last_dot = simple_name_str.rfind('.').map(|i| &simple_name_str[..i]);
            // Second-to-last segment — handles deep-nested promoted
            // names like `core.text.text.Text.new` where the carrier
            // type `Text` is the SECOND-to-last segment, and wanted
            // contains `Text` as a bare type-name. Without this arm
            // `Text.new` fails to register because neither the
            // first-dot ancestor (`core`) nor the last-dot ancestor
            // (`core.text.text.Text`) is in wanted (which has just
            // the bare `Text`).
            let second_to_last = {
                let leaf_pos = simple_name_str.rfind('.');
                leaf_pos.and_then(|leaf_idx| {
                    let prefix = &simple_name_str[..leaf_idx];
                    let parent_pos = prefix.rfind('.');
                    Some(match parent_pos {
                        Some(p) => &prefix[p + 1..],
                        None => prefix,
                    })
                })
            };
            first_dot.map(|p| wanted.contains(p)).unwrap_or(false)
                || last_dot
                    .filter(|p| Some(*p) != first_dot)
                    .map(|p| wanted.contains(p))
                    .unwrap_or(false)
                || second_to_last
                    .filter(|s| !s.is_empty())
                    .map(|s| wanted.contains(s))
                    .unwrap_or(false)
        };
        // Module-form mount surface: `mount core.sys.bitfield;` adds
        // the literal qualified module name `core.sys.bitfield` to
        // `wanted` (via `collect_mount_names`'s `full.join(".")`
        // arm).  The user's intent is "load every public symbol of
        // this module wholesale so `bitfield.<NAME>` resolves through
        // the codegen-side suffix-match".  Without this branch the
        // per-function filter rejects every symbol because neither
        // its simple name nor its `<module_name>.<simple>` qualified
        // form matches any literal-name entry in `wanted`, and the
        // suffix-match at the call site has no qualified key to bind
        // against.
        //
        // Closes task #121 stage 2.  Pairs with the parallel
        // expansion in `build_wanted_module_prefixes` that now
        // includes the literal qualified name in the prefix set so
        // the entry-iteration gate also matches.  Both gates were
        // dropping wholesale-module mounts on the floor before this
        // commit.
        let is_wholesale_module_mount = wanted.contains(module_name);
        // Last-segment-matches-wanted-bare-name: when the user writes
        // `mount core.sys.{PAGE_SIZE};`, wanted carries the bare
        // `PAGE_SIZE` plus `core.sys.PAGE_SIZE` + (cog-stripped) `sys.PAGE_SIZE`.
        // The const lives in `core/sys/common.vr` (declares
        // `module sys.common;`), so its archive descriptor.name is
        // `sys.common.PAGE_SIZE` (after the precompiler's descriptor-
        // name-promotion).  None of the wanted forms match — the
        // user's wanted bare-name `PAGE_SIZE` is two segments shy of
        // the descriptor's `sys.common.PAGE_SIZE`.  This last arm
        // closes that gap: if `simple_name`'s LAST segment matches a
        // wanted bare name AND simple_name has 2+ segments (so we
        // don't redundantly match on already-bare names that pass the
        // first arm), accept.
        //
        // Safety: bare-name registration is first-wins
        // (`register_function`'s `prefer_existing_functions=true` flow
        // at line ~1910), so this can't clobber an earlier-claimed
        // bare name.  Aliased duplicates land in qualified-only slots.
        let last_segment_matches_wanted = simple_name_str
            .rsplit('.')
            .next()
            .filter(|leaf| simple_name_str.len() > leaf.len()) // 2+ segments
            .map(|leaf| wanted.contains(leaf))
            .unwrap_or(false);
        // **Always allocate codegen-local id + insert into
        // `func_id_remap`** (regardless of whether the per-function
        // metadata-registration filter below accepts the function).
        //
        // Rationale: per-module bytecode has `Call { func_id }`
        // instructions whose `func_id` references the SAME module's
        // function table. If we skip id allocation for filter-rejected
        // entries, every body that references those entries via Call
        // would have its archive-local `func_id` identity-fall-back
        // through `ArchiveBodyRemap::map_function`'s
        // `unwrap_or(src)` — landing on whatever codegen-local id
        // happens to live at that slot (observed in the wild:
        // `Text.push_str` Calls landing on `Conv1d.parameters` /
        // `tensor_sqrt` / similar unrelated math/tensor functions).
        //
        // Allocating the id + inserting it into `func_id_remap`
        // BEFORE the filter ensures the remap is total over every
        // archive-local id this module emits. Filter-rejected
        // functions still don't get a `FunctionInfo` registered into
        // `ctx.functions`, so they remain invisible to user-side
        // name-resolution; the finalize-time stub-emitter will
        // synthesise a `RetV` placeholder for the unregistered slot
        // — strictly more diagnosable than a wrong-target dispatch.
        let new_id = verum_vbc::module::FunctionId(*next_id);
        *next_id = next_id.saturating_add(1);
        func_id_remap.insert(fn_desc.id.0, new_id);
        let last2_matches_wanted = last2_of(&qualified_borrowed)
            .map(|l2| last2_wanted.contains(l2))
            .unwrap_or(false)
            || last2_of(simple_name_str)
                .map(|l2| last2_wanted.contains(l2))
                .unwrap_or(false);
        if !wanted.contains(simple_name_str)
            && !wanted.contains(&qualified_borrowed)
            && !is_method_of_wanted_type
            && !is_wholesale_module_mount
            && !last_segment_matches_wanted
            && !last2_matches_wanted
        {
            // **XMOD-BAND-NAME-CARRY-1 final producing registration**
            // (T0277 leg B): the id above is already IN the remap, so
            // sibling bodies' Calls WILL be rewritten to it at body
            // merge (Tier-1) — but this arm never registers a name for
            // it anywhere, which made every filter-rejected sibling
            // callee (bodyless platform oracles like
            // `__ctx_store_tier0_raw`, un-mounted ThreadPool internals)
            // NAMELESS BY CONSTRUCTION: emission found no name, re-homed
            // the operand to REMAP_POISON, and dispatch died with zero
            // provenance. Carry the canonical name for the minted id —
            // the emission-side fallback then produces a resolvable
            // `external_function_names` entry (by-name lazy resolution
            // gets a chance; a genuine miss dies NAMED).
            ctx.resolved_name_by_id
                .borrow_mut()
                .entry(new_id.0)
                .or_insert_with(|| qualified_borrowed.clone());
            continue;
        }
        if trace_accept {
            let arm = if wanted.contains(simple_name_str) {
                0
            } else if wanted.contains(&qualified_borrowed) {
                1
            } else if is_method_of_wanted_type {
                2
            } else if is_wholesale_module_mount {
                3
            } else if last_segment_matches_wanted {
                4
            } else {
                5
            };
            arm_counts[arm] += 1;
        }
        registered_ids.insert(fn_desc.id.0);
        let simple_name = simple_name_str.to_string();
        let qualified = qualified_borrowed;
        // Task #25 — prefer qualified `<parent>.<variant>` lookup
        // when the function descriptor has a parent type recorded.
        // Falls back to simple-name first-wins index only when no
        // parent is attached.
        let parent_hint: Option<String> = fn_desc
            .parent_type
            .and_then(|tid| type_id_to_name.get(&tid).cloned());
        let variant_hit = parent_hint
            .as_ref()
            .and_then(|parent| {
                variant_index_qualified.get(&format!("{}.{}", parent, simple_name))
            })
            .or_else(|| variant_index.get(&simple_name))
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
                // PARAMNAME-CARRY (v2.10): mirror of the
                // `populate_ctx_from_archive` sibling above.
                module
                    .strings
                    .get(p.type_name)
                    .filter(|s| !s.is_empty())
                    .map(flatten_carried_param_name)
                    .or_else(|| type_ref_simple_name(&p.type_ref, module))
                    .unwrap_or_default()
            })
            .collect();
        // Mirror the closure-return-type extraction from
        // `populate_ctx_from_archive` so both archive-load paths
        // populate `param_closure_return_type_names` identically.
        let param_closure_return_type_names: Vec<Option<String>> = fn_desc
            .params
            .iter()
            .map(|p| extract_closure_return_type_from_typeref(&p.type_ref, module))
            .collect();
        // RETNAME-CARRY-1: archive-carried source-level name wins over
        // the lossy TypeRef re-derivation (see the sibling site in
        // `populate_ctx_from_archive`).
        let return_type_name = fn_desc
            .return_type_name
            .and_then(|sid| module.strings.get(sid).map(|s| s.to_string()))
            .or_else(|| type_ref_simple_name(&fn_desc.return_type, module));
        // RETNAME-CARRY-1 oracle (filtered-load leg).
        if let Ok(w) = std::env::var("VERUM_TRACE_RETNAME")
            && let Some(fname) = module.strings.get(fn_desc.name)
            && fname.contains(w.as_str())
        {
            eprintln!(
                "[retname/filtered] fn='{}' carried_sid={:?} carried={:?} final={:?}",
                fname,
                fn_desc.return_type_name,
                fn_desc
                    .return_type_name
                    .and_then(|sid| module.strings.get(sid)),
                return_type_name,
            );
        }
        let return_type_inner = type_ref_inner_generics(&fn_desc.return_type, module);
        // #87 — restore the intrinsic-name marker that was serialised
        // on the archive side.  Mirrors the populate_ctx_from_archive
        // site; without this, inlinable stdlib constants surface as
        // `UndefinedVariable` at the use site after the archive
        // round-trip.
        let intrinsic_name = fn_desc
            .intrinsic_name
            .and_then(|sid| lookup(sid).map(|s| s.to_string()));
        if std::env::var("VERUM_TRACE_INTRINSIC_LOAD").is_ok()
            && simple_name.contains("cbgr_alloc")
        {
            eprintln!(
                "[intrinsic-load:filtered] simple='{}' qualified='{}' intrinsic_name={:?} fn_desc.intrinsic_name_sid={:?} bytecode_len={}",
                simple_name, qualified, intrinsic_name, fn_desc.intrinsic_name, fn_desc.bytecode_length,
            );
        }
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
            // **Task #11 fix** — see `populate_ctx_from_archive` site
            // for the architectural rationale.  Mirror invariant:
            // every `FunctionInfo` constructed from an archived
            // `FunctionDescriptor` MUST set `takes_self_mut_ref`
            // from the first param's TypeRef.
            takes_self_mut_ref: fn_takes_self_mut_ref(fn_desc, module),
            return_type_name,
            return_type_inner,
            // #97 — see populate_ctx_from_archive for the rationale.
            is_const: fn_desc.is_const,
            is_transparent_wrapper: false,
            param_closure_return_type_names,
        };
        // T0330 mono-seed fallback (filtered-load leg — the sibling
        // capture lives in `populate_ctx_from_archive`): raw param
        // TypeRefs under the SAME globally-unique id, so
        // `record_generic_instantiation` can derive generic type args
        // for archive-loaded callees (`Maybe.fmt`) whose descriptors
        // never enter the codegen's `self.functions`.
        ctx.archive_fn_param_types.insert(
            new_id.0,
            fn_desc.params.iter().map(|p| p.type_ref.clone()).collect(),
        );
        ctx.register_function(qualified.clone(), info.clone());
        // T0706 final leg: a last2-accepted descriptor's DISPATCH form
        // is exactly its (dotted) simple name — the runtime by-name
        // probe asks for `UInt8.to_hex`, not the canonical
        // `core.base.UInt8.to_hex`.  The wanted-fanout below skips the
        // `w == simple_name` case by design, so install the dispatch
        // key here (first-wins; dotted-only, so the bare-leaf slot
        // stays with free functions per the may_claim rule).
        if last2_matches_wanted
            && simple_name_str.contains('.')
            && ctx.lookup_function(simple_name_str).is_none()
        {
            ctx.register_function(simple_name_str.to_string(), info.clone());
        }
        // BAKED-DEFAULT-ARG-1: surface the descriptor's default-value
        // channel to the call-site injector under every lookup
        // spelling the FunctionInfo itself registers with.
        if let Some(defaults) = descriptor_param_defaults(module, fn_desc) {
            ctx.function_param_defaults
                .insert(qualified.clone(), defaults.clone());
            ctx.function_param_defaults
                .insert(simple_name.to_string(), defaults);
        }
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
        // Compare wanted-W's leaf against simple_name's leaf — NOT against
        // the whole simple_name string. The precompiler's descriptor-name
        // promotion (commit 53c7d5448) turned simple_name from a bare leaf
        // (`args`) into a fully-qualified path (`script.args` for `script.vr`
        // declaring `module script;` under `core.shell`); the prior
        // `Some(simple_name.as_str())` literal-string comparison broke for
        // every promoted descriptor.  Leaf-to-leaf matching restores the
        // original intent: when the user's `mount X.{name}` wants a symbol
        // whose source-module-qualified descriptor.name ends in `.name`,
        // register the function under the user's wanted form too.
        let simple_leaf = simple_name.rsplit('.').next().unwrap_or(simple_name.as_str());
        for w in wanted.iter() {
            if w == &qualified {
                continue;
            }
            let w_leaf = w.rsplit('.').next().unwrap_or(w.as_str());
            // **Cross-pollination guard** (root cause of tasks #21 + #26):
            //
            // When both `w` and `simple_name` are qualified paths sharing
            // the same leaf (`select`, `join`, `new`, …) but rooted at
            // DIFFERENT modules, registering this function's `info` under
            // `w` is structurally wrong: it makes the qualified key
            // `w` resolve to a function whose FunctionId belongs to a
            // DIFFERENT module's body.  Cross-callers that look up `w`
            // get an info pointing at the wrong dispatch target.
            //
            // Original guard (`w.split('.').next() == simple.split('.').next()`)
            // matched on just the first segment — that's `core` for
            // every stdlib path, so `core.async.future.select` and
            // `core.shell.interactive.select` both passed the gate and
            // collapsed onto the same FunctionId.  Manifested as #21:
            // explicit `mount core.async.future.{select}` dispatched to
            // `core.shell.interactive.select`'s body at runtime because
            // the cross-fanout overwrote `core.async.future.select` →
            // `info(id_of_shell_select)`, and the user-side
            // authoritative-override then picked that polluted info.
            //
            // The architectural rule: cross-fanout is sound only when
            // the *whole path-to-leaf* matches — i.e. `w` and
            // `simple_name` describe the same module's same-named
            // export, registered redundantly under multiple keys
            // (e.g. legacy alias form vs canonical form for the same
            // function).  Bare-name `w` (no dot) keeps the original
            // leaf-renaming behaviour because there's no prefix to
            // compare; the bare-name slot is conceptually a global
            // alias the user explicitly asked for via `mount X.Y.{w}`.
            //
            // Fix: when w is qualified AND simple_name is qualified,
            // require the FULL path-before-leaf to match.  When either
            // is bare, fall back to the legacy first-segment check
            // (same liberality as before for the renaming case).
            fn path_to_leaf(s: &str) -> &str {
                match s.rfind('.') {
                    Some(idx) => &s[..idx],
                    None => "",
                }
            }
            let prefixes_compatible = match (w.contains('.'), simple_name.contains('.')) {
                // T0706: SUFFIX at a segment boundary, not strict
                // equality.  The dispatch spelling `UInt8.to_hex`
                // (path-to-leaf `UInt8`) must bind the promoted-chain
                // descriptor `base.primitives.UInt8.to_hex`
                // (path-to-leaf `base.primitives.UInt8`) — same
                // export, deeper canonical path.  Anti-squat is
                // preserved: DIFFERENT modules sharing a leaf
                // (`async.future` vs `shell.interactive`) are not
                // segment-suffixes of each other, so the #21/#26
                // cross-pollination class stays closed.
                (true, true) => {
                    let wp = path_to_leaf(w);
                    let sp = path_to_leaf(simple_name.as_str());
                    wp == sp
                        || sp.ends_with(&format!(".{}", wp))
                        || wp.ends_with(&format!(".{}", sp))
                }
                // DELIM-FANOUT-SQUAT-1: a DOTTED wanted key bound to a
                // BARE-named descriptor by leaf coincidence alone is
                // unsound. The call-site harvester puts `Type.method`
                // strings into `wanted` (e.g. `d.close()` under a
                // `Delimiter`-typed receiver harvests
                // `Delimiter.close`); pre-fix, the FIRST bare `close`
                // walked (an io-driver's `close(fd) -> ()`) registered
                // its info under the `Delimiter.close` key, first-wins
                // squatted it, and the devirtualizer then bound
                // `d.close()` straight to `close(fd)` — Delimiter's
                // real method (loaded later, qualified-only) never
                // reached the simple key. Only accept the binding when
                // the descriptor's own parent type matches the wanted
                // key's type segment — i.e. the bare name genuinely IS
                // that type's method exported under a bare descriptor
                // name.
                (true, false) => {
                    let type_seg = w.rsplit('.').nth(1).unwrap_or("");
                    !type_seg.is_empty()
                        && info
                            .parent_type_name
                            .as_ref()
                            .map(|p| p.as_str() == type_seg)
                            .unwrap_or(false)
                }
                // Bare wanted key + dotted descriptor: the explicit
                // `mount X.Y.{w}` renaming case — keep the legacy
                // liberality (there is no prefix on `w` to compare).
                (false, true) => true,
                // Both bare with equal leaves ⇒ w == simple_name,
                // already excluded above; keep the branch total.
                (false, false) => true,
            };
            // Same bare-slot rule as the sibling registrations: a method may
            // take a DOTTED wanted spelling (`Vector.swap`) but never a BARE
            // one — that leaf belongs to the free function, and `CallM` is a
            // method's only dispatch surface. Without this, `Vector.swap`
            // claimed the bare `swap` and a glob-mounted `swap(&mut a, &mut b)`
            // executed it.
            let bare_wanted = !w.contains('.');
            let may_claim = !bare_wanted
                || info.variant_tag.is_some()
                || info.parent_type_name.is_none();
            if w_leaf == simple_leaf
                && w != simple_name.as_str()
                && prefixes_compatible
                && may_claim
                && ctx.lookup_function(w).is_none()
            {
                ctx.register_function(w.clone(), info.clone());
            }
        }
        // Additional: register under the BARE leaf as well when the
        // wanted set contains it (i.e. the user mounted `{leaf}` directly,
        // expecting bare-name dispatch). The fanout above handles the
        // dotted forms; this bare-form arm closes the gap for
        // `mount core.sys.{PAGE_SIZE}` where wanted has the bare
        // `PAGE_SIZE` and the descriptor is `sys.common.PAGE_SIZE` —
        // without this, user-side `PAGE_SIZE` references the bare-name
        // slot which never gets the archive-loaded value, defaulting to 0.
        // Same bare-slot rule as the primary registration above: an
        // impl-block method never claims a bare leaf, even when the user
        // mounted that leaf — the mount names the free function or constant,
        // and `CallM` remains the only dispatch surface for methods.
        if simple_leaf != simple_name.as_str()
            && wanted.contains(simple_leaf)
            && (info.variant_tag.is_some() || info.parent_type_name.is_none())
            && ctx.lookup_function(simple_leaf).is_none()
        {
            ctx.register_function(simple_leaf.to_string(), info.clone());
        }
        // **Canonical `<Type>.<method>` form**: when simple_name has
        // the shape `<Type>.<method>` AND `<Type>` is in `wanted` (the
        // carrier-type mount, e.g. `mount core.time.duration.{Duration}`
        // adds `Duration` to wanted), register the function under the
        // bare `<Type>.<method>` form too.  Without this, the
        // typechecker's pre-resolved `ResolvedCallTarget::StaticCall {
        // qualified_name: "Duration.zero" }` misses in ctx.functions
        // because the registered key is module-qualified
        // (`core.time.duration.Duration.zero`); the missing canonical
        // form was the cause of every `Duration.<method>` /
        // `Instant.<method>` undefined-function regression after
        // mounting `core.time.<file>.{Type}`.
        //
        // Safety: `is_method_of_wanted_type` at line ~2080 already
        // gates whether we register at all — this site only fires for
        // functions whose simple_name's first-dot prefix matches a
        // wanted entry, so the `Type.method` form is guaranteed to
        // correspond to a wanted type.  The `lookup_function(...).is_none()`
        // gate preserves first-wins for cross-module name collisions.
        if let Some(first_dot_idx) = simple_name_str.find('.') {
            let type_prefix = &simple_name_str[..first_dot_idx];
            if wanted.contains(type_prefix)
                && simple_name_str != qualified.as_str()
                && ctx.lookup_function(simple_name_str).is_none()
            {
                ctx.register_function(simple_name_str.to_string(), info.clone());
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
        // form from `core.id.snowflake`) but the dispatcher routes
        // to a sibling stdlib's 1-arg `parse` because `parse#2` was
        // never registered.  `register_function`'s own arity branch
        // does the right thing here: same-arity → first-wins (matches
        // the prior gate's behaviour); different-arity → store under
        // `name#arity` so `lookup_function_with_arity` can pick the
        // right one.
        //
        // **Descriptor-name-promotion compatibility:** when `simple_name`
        // is a multi-dotted descriptor path (e.g. `sys.bitfield.USIZE_BITS`
        // post-promotion in commit 53c7d5448), the dotted form duplicates
        // the `qualified` key emitted above for suffix-match purposes —
        // a `find_function_by_suffix(".bitfield.USIZE_BITS")` then hits
        // BOTH `core.sys.sys.bitfield.USIZE_BITS` AND
        // `sys.bitfield.USIZE_BITS`, returns `None` on ambiguity, and
        // user code falls through to `UndefinedVariable`. Strip to the
        // leaf in that case — the bare-leaf form is what the arity-
        // disambiguation contract needs, and the qualified form is
        // already covered by `qualified` + the fanout above.
        let simple_for_registration = if simple_name.contains('.') {
            simple_name
                .rsplit('.')
                .next()
                .unwrap_or(simple_name.as_str())
                .to_string()
        } else {
            simple_name
        };
        // A METHOD must never own the bare leaf slot — `CallM` (receiver
        // syntax) is the only legal dispatch surface for impl-block methods,
        // the rule `verum_vbc`'s `is_free_function` enforces on every
        // bare-name `Call` layer. This registration strips `Vector.swap` and
        // `CycleIter.take` down to `swap` / `take` and stores them
        // unconditionally, so a method claimed the leaf that belongs to
        // `core.base.memory.swap` / `.take`. Under `mount core.prelude.*`
        // that made `take(&mut v)` report "undefined function" (the slot's
        // occupant was correctly rejected as non-free while the real free
        // function sat unreachable behind it) and made `swap(&mut a, &mut b)`
        // silently EXECUTE `Vector.swap`, null-dereferencing.
        //
        // Variant constructors carry a parent type as well and ARE
        // legitimately bare-callable (`Some(x)`, `Ok(v)`), so they keep the
        // slot. `Type.method` stays reachable through the qualified key and
        // the `Type.method` fanout above — only the BARE form is withheld.
        if info.variant_tag.is_some() || info.parent_type_name.is_none() {
            ctx.register_function(simple_for_registration, info);
        }
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
        // Wholesale-module-mount fanout: same rationale as the Pass 3
        // function-filter gate above.  `mount core.io.io;` (declared
        // by `core/io/io.vr` as `module io.io;`) drops the literal
        // qualified module name into `wanted`; the user expects every
        // sum type's variant constructors in that module to register
        // as if each had been individually `mount`ed.  Without this
        // branch, only types/variants explicitly enumerated land in
        // ctx.functions and qualified-form variant literals like
        // `IoError.Permission` fall through every variant-tag lookup.
        let is_wholesale_module_mount = wanted.contains(module_name);
        if !parent_in_scope
            && !any_variant_wanted
            && !qualified_variant_wanted
            && !is_wholesale_module_mount
        {
            continue;
        }
        let parent_name = parent_name_str.to_string();
        // Ordered generic-param names for pattern-bind payload typing
        // (ctx.type_generic_params doc) — lazy-walker twin of pass 4.
        if !ty.type_params.is_empty() {
            let params: Vec<String> = ty
                .type_params
                .iter()
                .filter_map(|tp| lookup(tp.name).map(|s| s.to_string()))
                .collect();
            if params.len() == ty.type_params.len() {
                ctx.type_generic_params
                    .entry(parent_name.clone())
                    .or_insert(params);
            }
        }
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
                            type_ref_payload_template(ty, &f.type_ref, module)
                                .unwrap_or_default()
                        })
                        .collect(),
                ),
                VariantKind::Record => (
                    variant.fields.len(),
                    variant
                        .fields
                        .iter()
                        .map(|f| {
                            type_ref_payload_template(ty, &f.type_ref, module)
                                .unwrap_or_default()
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
            param_closure_return_type_names: Vec::new(),
            };
            ctx.register_function(qualified, info);
            // Deliberately skip simple-name registration — see the
            // matching site in `register_module_filtered` for the
            // collision rationale (user `type ... is | Help | ...`
            // would otherwise be silently de-aliased).
        }
    }

    // Pass 5 — transparent-wrapper newtype constructor registration
    // (lazy-loaded mirror of the matching block in `register_module`).
    //
    // See the full rationale at the matching site in `register_module`.
    // Briefly: archive type descriptors carry `is_transparent_wrapper`,
    // but the synthetic constructor `FunctionInfo` that
    // `compile_type_decl` emits in-source has a sentinel id and never
    // archives.  Without re-synthesising it on the user side, every
    // call `CFd(0)` falls through to `compile_variant_constructor`'s
    // byte-sum-hash tag fallback, producing a bogus
    // `Variant(tag, payload)` wrapper instead of a transparent passthrough.
    //
    // Same `wanted` gate as the variant pass above — only register
    // newtype constructors whose parent type name is reachable through
    // the user's mount tree, matching the lazy-load discipline.
    for ty in &module.types {
        if !ty.is_transparent_wrapper {
            continue;
        }
        if !matches!(ty.kind, verum_vbc::types::TypeKind::Record) {
            continue;
        }
        let type_name = match lookup(ty.name) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let parent_in_scope = wanted.contains(&type_name);
        let is_wholesale_module_mount = wanted.contains(module_name);
        if !parent_in_scope && !is_wholesale_module_mount {
            continue;
        }
        if ctx.lookup_function(&type_name).is_some() {
            // Already registered; still mirror the type-aware caches.
            ctx.newtype_names.insert(type_name.clone());
            if let Some(first_field) = ty.fields.first()
                && let Some(inner_name) = type_ref_simple_name(&first_field.type_ref, module)
            {
                ctx.newtype_inner_type.insert(type_name.clone(), inner_name);
            }
            continue;
        }
        let arity = ty.fields.len().max(1);
        let param_names: Vec<String> = (0..arity).map(|i| format!("_{}", i)).collect();
        let param_type_names: Vec<String> = ty
            .fields
            .iter()
            .map(|f| type_ref_simple_name(&f.type_ref, module).unwrap_or_default())
            .collect();
        let info = FunctionInfo {
            id: FunctionId(u32::MAX / 2),
            param_count: arity,
            param_names,
            param_type_names,
            is_async: false,
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
            return_type_name: Some(type_name.clone()),
            return_type_inner: None,
            is_const: false,
            is_transparent_wrapper: true,
            param_closure_return_type_names: Vec::new(),
        };
        // FUNC-REGISTRY-QUALIFICATION-1 (phase 2): mirror the bare
        // newtype-ctor registration under its qualified
        // `<module>.<TypeName>` key (first-wins, never replacing) —
        // same discipline as the `populate_ctx_from_archive` Pass-5
        // site.
        let qualified_ctor = merge_module_and_simple_name(module_name, &type_name);
        if ctx.lookup_function(&qualified_ctor).is_none() {
            ctx.register_function(qualified_ctor, info.clone());
        }
        ctx.register_function(type_name.clone(), info);
        ctx.newtype_names.insert(type_name.clone());
        if let Some(first_field) = ty.fields.first()
            && let Some(inner_name) = type_ref_simple_name(&first_field.type_ref, module)
        {
            ctx.newtype_inner_type.insert(type_name.clone(), inner_name);
        }
    }
    if trace_accept && arm_counts.iter().any(|c| *c > 0) {
        eprintln!(
            "[accept] {:<34} simple={} qualified={} method-of-type={} \
             wholesale-mount={} last-seg={} last2={}",
            module_name,
            arm_counts[0],
            arm_counts[1],
            arm_counts[2],
            arm_counts[3],
            arm_counts[4],
            arm_counts[5],
        );
    }
    (func_id_remap, registered_ids)
}

/// WHAT THE PROGRAM ACTUALLY FORMATS (T0692).
///
/// `f"{a.cmp(b)}"` needs `Ordering.fmt`, but the program's text never
/// names `Ordering` — the type arrives as `Int.cmp`'s result, and
/// reachability is computed from source text, before inference.  The
/// missing fact lives in the baked metadata as
/// `FunctionDescriptor.return_type`, so the closure can recover it —
/// but only if it knows WHICH calls' results get formatted.
///
/// That question is answered here, syntactically and exactly: a call
/// standing inside an interpolation (or inside a `print` argument) is
/// a call whose result reaches `Display`.  Every other call's result
/// type is irrelevant to formatting, and pulling it in costs real
/// time — seeding from every reached function instead put a
/// hello-world's archive load at 1336 ms against 67 ms, because the
/// extra types dragged their defining modules through decode,
/// registration and the keep-set fixpoint.
///
/// Built on `verum_ast::visitor::Visitor`, whose `walk_expr` covers
/// all 73 expression forms: a hand-rolled recursion would silently
/// miss the interpolation nested in a closure in a match arm.
struct FormattedCallHarvest {
    /// Depth of enclosing format positions.  A counter, not a flag —
    /// `f"{f"{x.cmp(y)}"}"` nests, and a flag would clear on the way
    /// out of the inner one while still inside the outer.
    depth: usize,
    /// Simple names of the calls found directly in format position.
    names: HashSet<String>,
    /// Variables that appear in format position: `f"{o}"`.
    formatted_vars: HashSet<String>,
    /// What each `let` binds its variable to — the call names in the
    /// initialiser, and the variables the initialiser reads.  A
    /// variable in format position formats whatever its initialiser
    /// produced, so `let o = a.cmp(b); print(f"{o}")` must reach
    /// `Ordering` exactly as the inline spelling does.
    bindings: HashMap<String, (HashSet<String>, HashSet<String>)>,
}

impl FormattedCallHarvest {
    /// Functions whose arguments are formatted for output.  `print`
    /// is the one that matters in practice; the others are here
    /// because they format too, and a caller that reads
    /// `f"{...}"`-only behaviour out of `print("...")` would be
    /// reading a coincidence.
    fn formats_its_arguments(name: &str) -> bool {
        matches!(name, "print" | "eprint" | "panic" | "assert_msg")
    }

    /// The call names and variable reads an initialiser expression
    /// contains, at any depth: `let o = wrap(a.cmp(b))` binds `o` to
    /// both `wrap` and `cmp`, and either could be the one that decides
    /// the formatted type.
    fn scan_initialiser(expr: &verum_ast::Expr) -> (HashSet<String>, HashSet<String>) {
        struct Scan {
            calls: HashSet<String>,
            vars: HashSet<String>,
        }
        impl verum_ast::visitor::Visitor for Scan {
            fn visit_expr(&mut self, expr: &verum_ast::Expr) {
                use verum_ast::expr::ExprKind;
                match &expr.kind {
                    ExprKind::MethodCall { method, .. } => {
                        self.calls.insert(method.name.to_string());
                    }
                    ExprKind::Call { func, .. } => {
                        if let ExprKind::Path(path) = &func.kind {
                            self.calls.insert(path.last_segment_name().to_string());
                        }
                    }
                    ExprKind::Path(path) if path.is_single() => {
                        self.vars.insert(path.last_segment_name().to_string());
                    }
                    _ => {}
                }
                verum_ast::visitor::walk_expr(self, expr);
            }
        }
        let mut scan = Scan {
            calls: HashSet::new(),
            vars: HashSet::new(),
        };
        verum_ast::visitor::Visitor::visit_expr(&mut scan, expr);
        (scan.calls, scan.vars)
    }

    /// Record that every identifier `pattern` binds holds (part of)
    /// what `source` produced.  One rule for every binding position —
    /// `let`, `let..else`, a match arm binding its scrutinee, a `for`
    /// pattern binding an element of its iterable.  Destructuring
    /// binds each identifier to ALL of the source's calls: the harvest
    /// cannot know which component came from which call, and the cost
    /// of the over-approximation is only ever loading a Display impl
    /// that goes unused, never printing the wrong thing.
    fn bind_pattern(&mut self, pattern: &verum_ast::pattern::Pattern, source: &verum_ast::Expr) {
        struct BindingIdents(Vec<String>);
        impl verum_ast::visitor::Visitor for BindingIdents {
            fn visit_pattern(&mut self, pattern: &verum_ast::pattern::Pattern) {
                if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                    self.0.push(name.name.to_string());
                }
                verum_ast::visitor::walk_pattern(self, pattern);
            }
        }
        let mut idents = BindingIdents(Vec::new());
        verum_ast::visitor::Visitor::visit_pattern(&mut idents, pattern);
        if idents.0.is_empty() {
            return;
        }
        let (calls, vars) = Self::scan_initialiser(source);
        for name in idents.0 {
            let entry = self
                .bindings
                .entry(name)
                .or_insert_with(|| (HashSet::new(), HashSet::new()));
            entry.0.extend(calls.iter().cloned());
            entry.1.extend(vars.iter().cloned());
        }
    }

    /// Everything the module formats, following bindings to a
    /// fixpoint: `let o = a.cmp(b); let p = o; print(f"{p}")` must
    /// reach `cmp` through two hops.
    fn into_names(mut self) -> HashSet<String> {
        let mut pending: Vec<String> = self.formatted_vars.iter().cloned().collect();
        let mut seen: HashSet<String> = self.formatted_vars.clone();
        while let Some(var) = pending.pop() {
            let Some((calls, vars)) = self.bindings.get(&var) else {
                continue;
            };
            let calls: Vec<String> = calls.iter().cloned().collect();
            let vars: Vec<String> = vars.iter().cloned().collect();
            self.names.extend(calls);
            for next in vars {
                if seen.insert(next.clone()) {
                    pending.push(next);
                }
            }
        }
        self.names
    }
}

impl verum_ast::visitor::Visitor for FormattedCallHarvest {
    fn visit_stmt(&mut self, stmt: &verum_ast::Stmt) {
        use verum_ast::stmt::StmtKind;
        // `let` and `let ... else` bind the same way; both are records
        // of "this variable holds what that expression produced".
        let bound = match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => match value {
                verum_common::Maybe::Some(v) => Some((pattern, v)),
                verum_common::Maybe::None => None,
            },
            StmtKind::LetElse { pattern, value, .. } => Some((pattern, value)),
            _ => None,
        };
        if let Some((pattern, value)) = bound {
            self.bind_pattern(pattern, value);
        }
        verum_ast::visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &verum_ast::Expr) {
        use verum_ast::expr::ExprKind;
        // The two remaining binding positions: a match arm binds its
        // scrutinee's result, a `for` pattern binds an element of its
        // iterable.  `match a.cmp(b) { r => print(f"{r}") }` formats
        // an Ordering exactly as the let-bound spelling does.
        match &expr.kind {
            ExprKind::Match { expr: scrutinee, arms } => {
                for arm in arms.iter() {
                    self.bind_pattern(&arm.pattern, scrutinee);
                }
            }
            ExprKind::For { pattern, iter, .. } => {
                self.bind_pattern(pattern, iter);
            }
            _ => {}
        }
        let opens_format_position = match &expr.kind {
            ExprKind::InterpolatedString { .. } => true,
            ExprKind::Call { func, .. } => match &func.kind {
                ExprKind::Path(path) => Self::formats_its_arguments(path.last_segment_name()),
                _ => false,
            },
            _ => false,
        };
        if self.depth > 0 {
            match &expr.kind {
                ExprKind::MethodCall { method, .. } => {
                    self.names.insert(method.name.to_string());
                }
                ExprKind::Call { func, .. } => {
                    if let ExprKind::Path(path) = &func.kind {
                        self.names.insert(path.last_segment_name().to_string());
                    }
                }
                // A bare variable in format position formats whatever
                // its binding produced — resolved after the walk, since
                // the `let` may come later in a nested scope.
                ExprKind::Path(path) if path.is_single() => {
                    self.formatted_vars
                        .insert(path.last_segment_name().to_string());
                }
                _ => {}
            }
        }
        if opens_format_position {
            self.depth += 1;
            verum_ast::visitor::walk_expr(self, expr);
            self.depth -= 1;
        } else {
            verum_ast::visitor::walk_expr(self, expr);
        }
    }
}

/// Collect the simple names of every call whose result the module
/// formats.  Empty for a program that prints only literals — which is
/// the point: such a program pays nothing for this.
fn formatted_call_names(user_module: &verum_ast::Module) -> HashSet<String> {
    use verum_ast::visitor::Visitor;
    let mut harvest = FormattedCallHarvest {
        depth: 0,
        names: HashSet::new(),
        formatted_vars: HashSet::new(),
        bindings: HashMap::new(),
    };
    for item in user_module.items.iter() {
        harvest.visit_item(item);
    }
    harvest.into_names()
}


#[cfg(test)]
mod tests {
    use super::*;

    /// **Drift-pin**: every canonical-name shape produced by the
    /// precompiler must round-trip through `merge_module_and_simple_name`
    /// to the form the user-side codegen looks up.  Drift between
    /// registration (this fn) and lookup (`lookup_qualified_function`
    /// in codegen) is invisible — the function is "registered" but
    /// nobody can find it — and surfaces as runtime mis-dispatch when
    /// a same-named sibling in another module claims the bare-name
    /// fallback (e.g. `core.sys.bitfield.test_bit` silently dispatching
    /// to `core.net.tls13.handshake.zero_rtt_antireplay.test_bit`).
    #[test]
    fn merge_canonical_name_synthesis() {
        // (1) Bare leaf — no submodule directive in source. Prepend
        // module_name verbatim.
        assert_eq!(
            merge_module_and_simple_name("core.text", "new"),
            "core.text.new",
        );
        assert_eq!(
            merge_module_and_simple_name("core.io", "write"),
            "core.io.write",
        );
        // (2) Relative submodule — descriptor's leading segment is
        // also module_name's trailing segment. Skip the overlap.
        assert_eq!(
            merge_module_and_simple_name("core.sys", "sys.bitfield.test_bit"),
            "core.sys.bitfield.test_bit",
        );
        assert_eq!(
            merge_module_and_simple_name("core.collections", "collections.map.Map.new"),
            "core.collections.map.Map.new",
        );
        // (3) Fully-rooted submodule — descriptor already starts with
        // the cog + entry prefix. Drop module_name entirely.
        assert_eq!(
            merge_module_and_simple_name("core.async", "core.async.future.ready"),
            "core.async.future.ready",
        );
        // (4) No overlap — descriptor's leading segments are unrelated
        // to module_name's tail (e.g. `tls13.handshake....` under
        // archive entry `core.net`). Prepend module_name verbatim.
        assert_eq!(
            merge_module_and_simple_name(
                "core.net",
                "tls13.handshake.zero_rtt_antireplay.test_bit"
            ),
            "core.net.tls13.handshake.zero_rtt_antireplay.test_bit",
        );
        // (5) Longest-overlap discipline: when the descriptor and
        // module_name share both `sys` AND `sys.bitfield` as possible
        // prefixes, the algorithm picks the LONGER match. (Synthetic
        // case to pin the longest-wins rule.)
        assert_eq!(
            merge_module_and_simple_name("a.b.sys.bitfield", "sys.bitfield.test_bit"),
            "a.b.sys.bitfield.test_bit",
        );
        // (6) Full overlap of module_name with the descriptor's
        // prefix — module_name drops entirely.
        assert_eq!(
            merge_module_and_simple_name("core.sys.bitfield", "core.sys.bitfield.test_bit"),
            "core.sys.bitfield.test_bit",
        );
        // (7) Type-qualified bare descriptor — `Type.method` where
        // module_name doesn't overlap. (Static methods land here.)
        assert_eq!(
            merge_module_and_simple_name("core.time.duration", "Duration.zero"),
            "core.time.duration.Duration.zero",
        );
    }

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

    /// **Drift-pin**: every public type in `core/base/protocols.vr`
    /// MUST be carried into the precompiled archive.  The whole file
    /// is at structural risk because a single stray top-level token
    /// (e.g. `implement Foo for Bar { ... };` with an erroneous
    /// trailing `;`) makes `stdlib_bootstrap` parse-fail the entire
    /// file under the lenient-skip discipline, silently dropping
    /// every type declared after the bad token.  When that happens,
    /// downstream user code's `DefaultHasher.new()` evaluates to
    /// `Unit` (its impl wasn't compiled), `hasher.write_int(n)`
    /// panics with `method 'DefaultHasher.write_int' not found on
    /// receiver of runtime kind '()'`, and every `Formatter { ... }`
    /// record literal allocates with `type_id=0` then SetF
    /// out-of-bounds.  Test surface covers the most-load-bearing
    /// names in the file:
    ///
    ///   * `Hasher` — protocol consulted by Hash impls; missing →
    ///     `Int.hash(hasher)` dispatches `Hasher.write_int` to a
    ///     non-existent receiver.
    ///   * `DefaultHasher` — concrete Hasher used by the protocol's
    ///     default `hash_value` body; missing → `DefaultHasher.new()`
    ///     returns Unit.
    ///   * `Formatter` — buffer-writing record used by every Display
    ///     impl; missing → `Formatter { buffer: &mut buf }` writes
    ///     SetF at the wrong field index because
    ///     `type_field_layouts` has no entry.
    ///   * `FormatError`, `FmtResult` — referenced by every fallible
    ///     formatter method's return type.
    ///
    /// Each entry also asserts the field count surviving the
    /// precompile round-trip — empty `fields` on the descriptor
    /// makes `import_archive_type_with_protocol_remap` skip the
    /// `type_field_layouts` registration which is structurally
    /// equivalent to dropping the type.
    #[test]
    fn archive_default_hasher_carries_state_field() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return, // bootstrap build without archive — skip
        };
        // DefaultHasher is declared in core/base/protocols.vr (module
        // core.base.protocols).  Walk archive modules to find the
        // descriptor.
        let mut found: Option<(String, Vec<String>)> = None;
        let mut function_hits: Vec<(String, String)> = Vec::new();
        let mut type_names_in_protocols_module: Vec<String> = Vec::new();
        for entry in &archive.index {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for ty in &module.types {
                let name = match module.strings.get(ty.name) {
                    Some(s) => s,
                    None => continue,
                };
                if entry.name == "core.base"
                    || entry.name.contains("protocols")
                    || entry.name == "core.base.protocols"
                {
                    type_names_in_protocols_module.push(format!(
                        "{}:{}({}f)",
                        entry.name,
                        name,
                        ty.fields.len()
                    ));
                }
                if name == "DefaultHasher" {
                    let field_names: Vec<String> = ty
                        .fields
                        .iter()
                        .map(|f| {
                            module
                                .strings
                                .get(f.name)
                                .map(|s| s.to_string())
                                .unwrap_or_default()
                        })
                        .collect();
                    found = Some((entry.name.clone(), field_names));
                    break;
                }
            }
            for fn_desc in &module.functions {
                let fname = match module.strings.get(fn_desc.name) {
                    Some(s) => s,
                    None => continue,
                };
                if fname.contains("DefaultHasher") || fname == "new" && entry.name.contains("protocols") {
                    function_hits.push((entry.name.clone(), fname.to_string()));
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (entry_name, fields) = found.unwrap_or_else(|| {
            panic!(
                "DefaultHasher descriptor MUST be in the precompiled archive — \
                 missing entry means stdlib precompiler dropped the type.\n\
                 function_hits (DefaultHasher.* or new in protocols entries):\n  {}\n\
                 type_names in protocols-containing entries (first 30):\n  {}",
                function_hits.iter().take(30).map(|(e, f)| format!("{}::{}", e, f)).collect::<Vec<_>>().join("\n  "),
                type_names_in_protocols_module.iter().take(30).cloned().collect::<Vec<_>>().join("\n  "),
            )
        });
        assert_eq!(
            fields,
            vec!["state".to_string()],
            "DefaultHasher (archive entry `{}`) must carry exactly one \
             field `state`; precompiler dropped it (fields={:?})",
            entry_name,
            fields,
        );

        // Probe the broader public surface of core/base/protocols.vr.
        // Any of these missing means the whole file got
        // lenient-SKIPped at parse time and downstream stdlib code is
        // architecturally broken.  Test names use the canonical
        // simple-type-name form because the archive is searched
        // module-by-module (descriptor name only).
        let probe = |type_name: &str, expected_field_count: Option<usize>| {
            let mut found_arity: Option<usize> = None;
            for entry in &archive.index {
                let module = match archive.load_module(&entry.name) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                for ty in &module.types {
                    let name = match module.strings.get(ty.name) {
                        Some(s) => s,
                        None => continue,
                    };
                    if name == type_name {
                        found_arity = Some(ty.fields.len());
                        break;
                    }
                }
                if found_arity.is_some() {
                    break;
                }
            }
            let arity = found_arity.unwrap_or_else(|| {
                panic!(
                    "type `{}` (declared in core/base/protocols.vr) MUST be in \
                     the precompiled archive.  Missing entry means the whole \
                     file was lenient-SKIPped at parse time — check for a \
                     stray `;` after an `implement` block, an unmatched brace, \
                     or any other top-level syntax defect.",
                    type_name
                )
            });
            if let Some(expected) = expected_field_count {
                assert_eq!(
                    arity, expected,
                    "type `{}` must have {} field(s) in the archive (got {}) — \
                     the precompiler may have stripped fields during the \
                     stripped-bytecode optimisation, OR the type was rebuilt \
                     without its declared body.",
                    type_name, expected, arity,
                );
            }
        };
        // Records — must carry their declared field counts.
        //
        // These numbers are RESTATED from the source declaration, so they go
        // stale silently when the type gains a field: the archive then reports
        // the new count, the test reports the old one, and the message blames
        // the precompiler for "stripping fields" when nothing was stripped.
        // That is what happened here — `Formatter` is declared with two fields
        // in core/base/protocols.vr:611 (`buffer: &mut Text`, `spec:
        // FormatSpec`) and this said one.
        //
        // The fundamental fix is to derive the expectation from the `.vr`
        // declaration rather than restate it, the way
        // verum_common/tests/{type,protocol}_archive_modules_pin.rs read their
        // arms out of the source. Until then, these must be updated whenever
        // the record changes shape.
        probe("Formatter", Some(2));
        probe("FormatError", Some(0));
        // Protocol types — no `fields` (their methods live on the
        // monomorphised impl side); just assert presence.
        probe("Hasher", None);
        probe("Hash", None);
        probe("PartialEq", None);
        probe("Eq", None);
        probe("Ord", None);
        probe("PartialOrd", None);
        probe("Clone", None);
        probe("Default", None);
        probe("Debug", None);
        probe("Display", None);
    }

    /// **Drift-pin**: every protocol-default-method monomorphisation
    /// MUST ship in the precompiled archive with a real
    /// (non-zero-length) bytecode body.  When `stdlib_bootstrap`
    /// processes `implement Hasher for DefaultHasher`,
    /// `generate_default_protocol_methods` queues `DefaultHasher.write_int`
    /// and `DefaultHasher.write_byte` (default bodies on the Hasher
    /// protocol that DefaultHasher does NOT override) into
    /// `pending_default_methods`.  `compile_pending_default_methods`
    /// MUST then run before module finalisation so each queued
    /// `<Type>.<method>` gets a real archive body.
    ///
    /// Without this pin, `hasher.write_int(42)` (where `hasher` is a
    /// concrete DefaultHasher) panics at runtime with
    /// `method 'DefaultHasher.write_int' not found on receiver of
    /// runtime kind 'Object'`, because the runtime's method-table
    /// lookup misses the unmonomorphised default body.  Affected
    /// classes include every protocol with default methods: Hasher
    /// (write_int / write_byte), Hash (hash_value), PartialEq (ne via
    /// blanket impl<T: Ord>), Display / Debug forwarders, and every
    /// Iterator combinator default (map, filter, fold, …).
    #[test]
    fn archive_carries_protocol_default_method_monomorphisations() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        // Each tuple: (qualified_function_name, "rationale").
        // Pick representative samples whose default body lives on a
        // protocol but whose receiver-type implements only a subset
        // of the protocol's API.
        let required = [
            (
                "DefaultHasher.write_int",
                "Hasher.write_int default — DefaultHasher overrides only `write`",
            ),
            (
                "DefaultHasher.write_byte",
                "Hasher.write_byte default — DefaultHasher overrides only `write`",
            ),
        ];
        let mut missing: Vec<&'static str> = Vec::new();
        let mut empty_body: Vec<&'static str> = Vec::new();
        let mut all_default_hasher_fns: Vec<(String, String, u32)> = Vec::new();
        for entry in &archive.index {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for fn_desc in &module.functions {
                let name = match module.strings.get(fn_desc.name) {
                    Some(s) => s,
                    None => continue,
                };
                if name.contains("DefaultHasher") || name.contains("Hasher.write") {
                    all_default_hasher_fns.push((
                        entry.name.clone(),
                        name.to_string(),
                        fn_desc.bytecode_length,
                    ));
                }
            }
        }
        for (qualified, _why) in &required {
            let mut found_with_body = false;
            let mut found_at_all = false;
            'outer: for entry in &archive.index {
                let module = match archive.load_module(&entry.name) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                for fn_desc in &module.functions {
                    let name = match module.strings.get(fn_desc.name) {
                        Some(s) => s,
                        None => continue,
                    };
                    // Match either bare `DefaultHasher.write_int` or
                    // any qualified form ending with `.<qualified>`.
                    if name == *qualified
                        || name.ends_with(&format!(".{}", qualified))
                    {
                        found_at_all = true;
                        if fn_desc.bytecode_length > 0 {
                            found_with_body = true;
                            break 'outer;
                        }
                    }
                }
            }
            if !found_at_all {
                missing.push(qualified);
            } else if !found_with_body {
                empty_body.push(qualified);
            }
        }
        assert!(
            missing.is_empty(),
            "protocol-default-method monomorphisation(s) MISSING from \
             archive: {:?}. This indicates stdlib_bootstrap's \
             `compile_core_module_from_ast` skipped \
             `compile_pending_default_methods()` between \
             `resolve_pending_imports` and the body-compilation pass.\n\
             All DefaultHasher/Hasher.write functions in archive (first 40):\n  {}",
            missing,
            all_default_hasher_fns.iter().take(40)
                .map(|(e, n, l)| format!("{}::{} (body={}B)", e, n, l))
                .collect::<Vec<_>>().join("\n  "),
        );
        assert!(
            empty_body.is_empty(),
            "protocol-default-method monomorphisation(s) present but with \
             zero-length body: {:?}. The queue ran but the body emit was \
             skipped — likely a `[lenient] SKIP` of the default body's \
             AST.",
            empty_body,
        );
    }

    /// **Diagnostic**: decode Formatter.write_str's bytecode to see
    /// whether the body actually calls push_str or just returns Ok.
    /// A 33-byte body for a 2-instruction logical body (call +
    /// wrap-result + ret) might mean the call was lenient-SKIPped.
    #[test]
    #[ignore = "diagnostic only — Formatter.write_str bytecode disassembly"]
    fn diag_decode_formatter_write_str() {
        use verum_vbc::bytecode::decode_instructions;
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        for entry in &archive.index {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for fn_desc in &module.functions {
                let name = match module.strings.get(fn_desc.name) {
                    Some(s) => s,
                    None => continue,
                };
                if name == "Formatter.write_str" {
                    let off = fn_desc.bytecode_offset as usize;
                    let len = fn_desc.bytecode_length as usize;
                    if off + len > module.bytecode.len() {
                        eprintln!("{}::{} body out-of-range", entry.name, name);
                        continue;
                    }
                    let region = &module.bytecode[off..off + len];
                    eprintln!("Found {}::{} (params={}, body={}B):",
                        entry.name, name, fn_desc.params.len(), len);
                    eprintln!("  raw bytes: {:02x?}", region);
                    match decode_instructions(region) {
                        Ok(instrs) => {
                            for (i, instr) in instrs.iter().enumerate() {
                                eprintln!("  [{}] {:?}", i, instr);
                            }
                        }
                        Err(e) => eprintln!("  decode error: {:?}", e),
                    }
                    return;
                }
            }
        }
        eprintln!("Formatter.write_str NOT FOUND");
    }

    /// **Diagnostic**: dump every archive function whose simple
    /// name is `write_str` to reveal name collisions across stdlib
    /// modules.  Each collision is a potential method-dispatch
    /// hazard — when user code calls `receiver.write_str(...)` on a
    /// type whose impl has its own `write_str`, codegen's
    /// `lookup_function_with_arity` must pick the receiver-type-
    /// qualified entry; if name-collision dispatch picks a free
    /// function with the same simple name, the call lands on the
    /// wrong body and the user's `&mut self` mutation never happens.
    #[test]
    #[ignore = "diagnostic only — surfaces write_str name collisions"]
    fn diag_dump_write_str_entries() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        let mut all: Vec<(String, String, usize, u32)> = Vec::new();
        for entry in &archive.index {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for fn_desc in &module.functions {
                let name = match module.strings.get(fn_desc.name) {
                    Some(s) => s,
                    None => continue,
                };
                if name.ends_with(".write_str") || name == "write_str" {
                    all.push((
                        entry.name.clone(),
                        name.to_string(),
                        fn_desc.params.len(),
                        fn_desc.bytecode_length,
                    ));
                }
            }
        }
        eprintln!("write_str entries in archive: {}", all.len());
        for (entry, name, params, body) in &all {
            eprintln!("  {}::{} (params={}, body={}B)", entry, name, params, body);
        }
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

    /// End-to-end: simulate the `verum run /tmp/text_no_prelude.vr`
    /// path. Build SymbolGraph, BFS from a `Text` seed, verify the
    /// defining module gets loaded and `Text.new` lands in the
    /// codegen ctx under the bare `Text.new` key (NOT just the
    /// module-qualified form).
    #[test]
    fn end_to_end_text_new_registered_under_bare_key() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        // Mirror the seed set the harvester would produce for
        // `let buffer = Text.new()` (MethodCall shape).
        let mut wanted: HashSet<String> = HashSet::new();
        wanted.insert("Text".to_string());
        wanted.insert("Text.new".to_string());
        wanted.insert("print".to_string());

        let cache = ArchiveCtxCache::new();
        let graph = cache.graph(archive);

        // Step 1: graph must have Text.new in qualified_to_module.
        let text_new_module_idx = graph
            .baked
            .module_of("Text.new")
            .expect("graph must index Text.new in its function table");
        let text_new_entry = &archive.index[text_new_module_idx as usize];
        eprintln!(
            "Text.new is defined in archive entry: {} (idx {})",
            text_new_entry.name, text_new_module_idx
        );

        // Step 2: reachability from `wanted` must include Text.new.
        let (reached, reached_modules) =
            graph.reachable(&wanted, &HashSet::new());
        assert!(
            reached.contains("Text.new"),
            "BFS from Text/Text.new MUST reach Text.new"
        );
        assert!(
            reached_modules.contains(&text_new_module_idx),
            "BFS modules MUST include the Text.new defining entry ({})",
            text_new_entry.name
        );

        // Step 3: simulate register_module_filtered — load the entry,
        // then verify Text.new gets registered.
        let module = archive
            .load_module_by_index(text_new_module_idx as usize)
            .expect("entry must decode");
        let mut ctx = CodegenContext::new();
        let mut next_id: u32 = 0;
        let _remap = register_module_filtered(
            &module,
            &text_new_entry.name,
            &mut ctx,
            &wanted,
            &mut next_id,
        );

        // Step 4: bare `Text.new` MUST be in ctx.functions for
        // user-side static-method dispatch.
        let registered_keys: Vec<String> = ctx
            .functions
            .keys()
            .filter(|k| k.contains("Text.new") || k.ends_with(".new"))
            .cloned()
            .collect();
        assert!(
            ctx.lookup_function("Text.new").is_some(),
            "ctx must register `Text.new` under bare key for user-side \
             static dispatch. Registered Text.new-related keys: {:?}",
            registered_keys
        );
    }

    /// Drift-pin: the archive-wide symbol graph must surface every
    /// archive-defined `Text.new` / `Maybe.is_some` / `Map.contains_key`
    /// callee from a seed walk that names just the bare type. This is
    /// the contract that lets `register_module_filtered` accept the
    /// function via the literal-simple-name match without falling back
    /// to the heuristic filter arms.
    #[test]
    fn graph_reaches_canonical_stdlib_methods_from_type_seeds() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return, // bootstrap-phase build, no archive
        };
        let cache = ArchiveCtxCache::new();
        let graph = cache.graph(archive);
        // Quick sanity: graph indexes some functions.
        assert!(
            graph.baked.function_count() > 0,
            "graph function table empty — graph build broken"
        );
        // Seed `Text` should reach `Text.new` (and other Text methods)
        // via the prefix index.
        let mut seeds = HashSet::new();
        seeds.insert("Text".to_string());
        let (reached, _modules) = graph.reachable(&seeds, &HashSet::new());
        assert!(
            reached.contains("Text.new"),
            "graph reachability from seed `Text` MUST reach `Text.new`; \
             function table has Text.new = {}, prefix index for Text has {} entries",
            graph.baked.function_index("Text.new").is_some(),
            graph
                .baked
                .prefix_matches("Text")
                .map(|m| m.count())
                .unwrap_or(0),
        );
        assert!(
            reached.contains("Maybe.is_some") || reached.contains("Map.contains_key"),
            "transitive reachability MUST reach at least one of \
             Maybe.is_some / Map.contains_key (transitively called from \
             Text impl methods); reached={} entries",
            reached.len(),
        );
    }

    /// The baked sidecar must answer exactly what a fresh scan of the
    /// same archive answers (T0753).
    ///
    /// This is the gate that makes the sidecar safe to trust. The two
    /// producers run at different times, in different processes, from
    /// different code paths — the bake writes it, the fallback derives
    /// it — and a divergence between them does not fail loudly: it
    /// changes which archive entries a compile decodes, which changes
    /// which bodies are merged, which surfaces as a missing method at
    /// RUN time in some unrelated program. Compare them directly.
    #[test]
    fn baked_symbol_graph_matches_a_fresh_scan() {
        let Some(archive) = crate::embedded_stdlib_vbc::get_runtime_archive() else {
            eprintln!("no embedded archive in this build — nothing to compare");
            return;
        };
        let Some(bytes) = crate::embedded_symbol_graph::embedded_bytes() else {
            panic!(
                "this build embeds an archive but no symbol-graph sidecar — \
                 every compiler start will rebuild the graph by decoding all \
                 {} entries. Run `verum stdlib precompile`.",
                archive.module_count()
            );
        };
        let baked = crate::symbol_graph_baked::BakedSymbolGraph::from_bytes(
            std::borrow::Cow::Borrowed(bytes),
        )
        .expect("embedded sidecar must be readable by the format version that embedded it");
        let scanned = SymbolGraph::scan_and_encode(archive);

        assert_eq!(
            baked.entry_count(),
            scanned.entry_count(),
            "entry count differs between the baked sidecar and a fresh scan"
        );
        assert_eq!(
            baked.function_count(),
            scanned.function_count(),
            "function count differs between the baked sidecar and a fresh scan"
        );

        // Walk every row rather than sampling: a sample would pass on
        // the 99.9 % of names that agree and miss the one re-export
        // spelling that does not, which is exactly the shape of every
        // defect this file has carried.
        let mut mismatches: Vec<String> = Vec::new();
        for i in 0..scanned.function_count() as u32 {
            let name = scanned.function_name(i);
            if baked.function_name(i) != name {
                mismatches.push(format!(
                    "row {i}: baked {:?} vs scanned {:?}",
                    baked.function_name(i),
                    name
                ));
                if mismatches.len() > 8 {
                    break;
                }
                continue;
            }
            if baked.module_of_index(i) != scanned.module_of_index(i) {
                mismatches.push(format!(
                    "{name}: baked entry {} vs scanned entry {}",
                    baked.entry_name(baked.module_of_index(i)),
                    scanned.entry_name(scanned.module_of_index(i)),
                ));
            }
            let a: Vec<&str> = baked.callees(i).collect();
            let b: Vec<&str> = scanned.callees(i).collect();
            if a != b {
                mismatches.push(format!(
                    "{name}: {} baked callees vs {} scanned",
                    a.len(),
                    b.len()
                ));
            }
            if mismatches.len() > 8 {
                break;
            }
        }
        assert!(
            mismatches.is_empty(),
            "baked symbol graph disagrees with a fresh scan of the same archive:\n  {}",
            mismatches.join("\n  ")
        );

        // The leaf index drives the fanout cap, so a difference there
        // changes the closure size without changing any name above.
        for probe in ["next", "map", "clone", "eq", "fmt", "new", "len"] {
            assert_eq!(
                baked.leaf_match_count(probe),
                scanned.leaf_match_count(probe),
                "leaf index for {probe:?} differs — the fanout cap would see a different number"
            );
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

#[cfg(test)]
mod formatted_call_harvest_tests {
    use super::formatted_call_names;

    /// Parse a source string into a module the way the compiler does.
    fn module_of(src: &str) -> verum_ast::Module {
        verum_fast_parser::VerumParser::new()
            .parse_module_str(src, verum_common::FileId::new(0))
            .expect("probe must parse")
    }

    #[test]
    fn call_inside_an_interpolation_is_harvested() {
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; print(f\"{a.cmp(a)}\"); }",
        ));
        assert!(
            names.contains("cmp"),
            "the call whose result gets formatted must be harvested, got {:?}",
            names
        );
    }

    #[test]
    fn a_program_that_formats_only_literals_harvests_nothing() {
        // The whole point of the narrowing: such a program pays no
        // symbol-closure cost at all.
        let names = formatted_call_names(&module_of("fn main() { print(\"hello\"); }"));
        assert!(names.is_empty(), "expected no names, got {:?}", names);
    }

    #[test]
    fn a_variable_in_format_position_inherits_its_binding() {
        // `let o = a.cmp(b); print(f"{o}")` must reach `Ordering`
        // exactly as `print(f"{a.cmp(b)}")` does — the inline spelling
        // and the bound one are the same program.
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; let o = a.cmp(a); print(f\"{o}\"); }",
        ));
        assert!(
            names.contains("cmp"),
            "a formatted variable must carry the call that bound it, got {:?}",
            names
        );
    }

    #[test]
    fn binding_chains_are_followed_to_a_fixpoint() {
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; let o = a.cmp(a); let p = o; let q = p; \
             print(f\"{q}\"); }",
        ));
        assert!(
            names.contains("cmp"),
            "a chain of bindings must be followed, got {:?}",
            names
        );
    }

    #[test]
    fn a_destructured_binding_carries_the_initialiser_calls() {
        // Destructuring cannot know which component came from which
        // call, so every bound name carries all of them — the cost is
        // a Display impl loaded and unused, never a wrong print.
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; let (o, n) = (a.cmp(a), 2); print(f\"{o}\"); }",
        ));
        assert!(
            names.contains("cmp"),
            "a tuple-destructured formatted variable must carry the calls, got {:?}",
            names
        );
    }

    #[test]
    fn a_match_arm_binding_carries_the_scrutinee_calls() {
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; match a.cmp(a) { r => print(f\"{r}\") } }",
        ));
        assert!(
            names.contains("cmp"),
            "a match arm binds its scrutinee's result, got {:?}",
            names
        );
    }

    #[test]
    fn a_for_pattern_carries_the_iterable_calls() {
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; for o in [a.cmp(a)] { print(f\"{o}\"); } }",
        ));
        assert!(
            names.contains("cmp"),
            "a for pattern binds an element of its iterable, got {:?}",
            names
        );
    }

    #[test]
    fn a_binding_that_is_never_formatted_stays_out() {
        // The narrowing has to survive bindings: tracking them must not
        // turn into "every call in the program".
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; let unused = a.cmp(a); print(\"x\"); }",
        ));
        assert!(
            !names.contains("cmp"),
            "an unformatted binding must not be seeded, got {:?}",
            names
        );
    }

    #[test]
    fn a_call_outside_format_position_is_not_harvested() {
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; let _b = a.cmp(a); print(\"x\"); }",
        ));
        assert!(
            !names.contains("cmp"),
            "a result that is never formatted must not be seeded, got {:?}",
            names
        );
    }

    #[test]
    fn nesting_does_not_lose_the_outer_format_position() {
        // A flag instead of a counter clears here on the way out of
        // the inner interpolation, dropping `hi`.
        let names = formatted_call_names(&module_of(
            "fn main() { let a: Int = 1; print(f\"{f\"{a.lo()}\"}{a.hi()}\"); }",
        ));
        assert!(
            names.contains("lo") && names.contains("hi"),
            "both nested and trailing calls must be harvested, got {:?}",
            names
        );
    }
}
