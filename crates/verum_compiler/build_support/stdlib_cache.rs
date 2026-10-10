//! Automatic stdlib archive identity shared by the build script and its controls.
//! The caller supplies core files in relative-path order; producer files use
//! the fixed order below. Dependency reporting remains independent of baking.

use std::path::Path;

pub(crate) fn compute_archive_key<'a>(
    project_root: &Path,
    schema: &str,
    sorted_core_files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    mut report_dependency: impl FnMut(&Path),
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    // Mix the schema-version tag in first so any wire-format bump
    // forces a full re-precompile even when source hasn't changed.
    hasher.update(b"schema:");
    hasher.update(schema.as_bytes());
    hasher.update(b"\0");
    // Mix in the codegen-path source files so a Rust-side change to
    // VBC codegen / intrinsic dispatch invalidates the precompile
    // cache. Without this, a change to e.g.
    // `crates/verum_vbc/src/intrinsics/mod.rs::lookup_intrinsic` —
    // which determines how `@intrinsic("ctlz", ...)` lowers to
    // bytecode — produces a fresh `verum` binary that embeds a
    // STALE `runtime.vbca` compiled by an older codegen. The
    // resulting cross-tier divergence is invisible at the cargo
    // dependency level (cargo sees verum_vbc as a dep of
    // verum_compiler and rebuilds them; build.rs sees no `.vr`
    // source changes and reuses the cached vbca). Discovered via
    // `core-tests/mem/size_class/property_test::law_round_trip_full_table_exhaustive`
    // — a fix to `lookup_intrinsic` was correctly in the binary but
    // not reflected in the embedded stdlib.
    //
    // The mixed-in files MUST be the source-of-truth for codegen
    // strategies, intrinsic dispatch, and lowering — anything that
    // affects what the stdlib precompiler emits.
    let codegen_paths: &[&str] = &[
        // The bootstrap path consumes this parser's declaration, expression,
        // type and normalization semantics before emitting any descriptor or
        // body. Parser-only changes therefore invalidate the same archive.
        // stdlib_cache_identity checks this roster against every src/**/*.rs.
        "crates/verum_fast_parser/Cargo.toml",
        "crates/verum_fast_parser/src/attr_validation.rs",
        "crates/verum_fast_parser/src/decl.rs",
        "crates/verum_fast_parser/src/error.rs",
        "crates/verum_fast_parser/src/expr.rs",
        "crates/verum_fast_parser/src/lib.rs",
        "crates/verum_fast_parser/src/normalize.rs",
        "crates/verum_fast_parser/src/parser.rs",
        "crates/verum_fast_parser/src/pattern.rs",
        "crates/verum_fast_parser/src/proof.rs",
        "crates/verum_fast_parser/src/recovery.rs",
        "crates/verum_fast_parser/src/safe_interpolation.rs",
        "crates/verum_fast_parser/src/stmt.rs",
        "crates/verum_fast_parser/src/ty.rs",
        "crates/verum_vbc/src/intrinsics/mod.rs",
        "crates/verum_vbc/src/intrinsics/registry.rs",
        // Opcode identities and synthesized wrapper bodies also shape baked calls.
        "crates/verum_vbc/src/instruction.rs",
        "crates/verum_vbc/src/intrinsics/expand.rs",
        "crates/verum_vbc/src/intrinsics/codegen.rs",
        "crates/verum_vbc/src/intrinsics/lowering.rs",
        // Actual array storage evidence, callable conversions, and declared
        // element semantics all shape the bytecode emitted into the archive.
        "crates/verum_vbc/src/array_storage.rs",
        "crates/verum_vbc/src/codegen/array_coercions.rs",
        "crates/verum_vbc/src/codegen/array_elements.rs",
        "crates/verum_vbc/src/codegen/expressions.rs",
        "crates/verum_vbc/src/codegen/statements.rs",
        // Error classification decides whether invalid source becomes a baked
        // panic stub or refuses compilation (including constructor ownership).
        "crates/verum_vbc/src/codegen/error.rs",
        // `well_known_types.rs` owns `has_runtime_inline_dispatch` —
        // the predicate that decides whether codegen DEVIRTUALISES a
        // method call to a static `Call` or keeps it on `CallM` for the
        // runtime intercepts.  That decision is baked into every
        // archive-compiled stdlib body (e.g. whether
        // `FieldInfo.has_attribute` calls `ListIter.any` statically or
        // via CallM — LISTITER-DEVIRT-NEXT-1), so a change here MUST
        // invalidate the precompiled `runtime.vbca` like the other
        // codegen sources do.
        "crates/verum_common/src/well_known_types.rs",
        // `mod.rs` carries the function-registration / TypeRef-resolution
        // logic (`register_function`, `register_impl_function`,
        // `resolve_field_type_ref`, …) — changes here affect what
        // `FunctionDescriptor` shape the precompile pass emits into
        // `runtime.vbca`, so it must invalidate the cache like the other
        // codegen sources do.  Pre-fix a change to `mod.rs` rebuilt the
        // user_compiler crate but reused the stale precompile archive,
        // making in-source codegen fixes invisible at every cross-module
        // call site that consults the archived FunctionInfo.
        "crates/verum_vbc/src/codegen/mod.rs",
        "crates/verum_vbc/src/codegen/bootstrap_types.rs",
        "crates/verum_vbc/src/codegen/parsed_field_types.rs",
        "crates/verum_vbc/src/codegen/value_uses.rs",
        "crates/verum_vbc/src/codegen/formal_parameters.rs",
        "crates/verum_vbc/src/value_use.rs",
        "crates/verum_vbc/src/resource_discipline.rs",
        "crates/verum_common/src/value_use.rs",
        "crates/verum_vbc/src/codegen/associated_types.rs",
        "crates/verum_vbc/src/type_layout.rs",
        "crates/verum_vbc/src/codegen/context.rs",
        // The precompile pass itself: `scan_module_reexports` /
        // `inject_decl_spans` / glob-expansion shape ALL live in
        // precompile.rs, and archive_metadata.rs owns the Pass-2
        // descriptor registration (function key shapes, module_path
        // collapsing). A change to either alters the EMITTED
        // metadata sidecar without touching any `.vr` source or
        // codegen path — pre-fix such a change produced a fresh
        // `verum` binary embedding a STALE `runtime.core_metadata`
        // (discovered via the prelude-reexport canonicalisation fix:
        // `format_display` stayed unbound because the cache key never
        // saw the scanner change).
        // `refinement_verify.rs` decides whether a refinement is discharged
        // STATICALLY or degraded to a runtime check, and that decision is
        // applied to every `core/` function as the archive is built. A change
        // here therefore changes what the precompiled stdlib means, so it MUST
        // invalidate `runtime.vbca` like the other sources in this list.
        //
        // Its absence was measured: after fixing the refinement binder, a
        // rebuild reported "Stdlib precompile cache HIT" and finished in 1m05s
        // against 36 and 50 minutes for real bakes — the compiler changed, the
        // archive did not, and the "green" bake had verified nothing.
        "crates/verum_compiler/src/pipeline/refinement_verify.rs",
        "crates/verum_compiler/src/precompile.rs",
        "crates/verum_compiler/src/archive_metadata.rs",
        // The bootstrap compiler itself. `stdlib_bootstrap.rs` parses every
        // `core/` file, decides which declarations survive to registration,
        // and drives codegen over them; `core_compiler.rs` owns the module
        // walk and the target gating that decides which files are compiled
        // at all. Nothing about the archive is more determined by a source
        // file than by these two, and neither was a cache input.
        //
        // Measured 2026-08-15, the same way the `refinement_verify.rs` entry
        // above was: an edit to `stdlib_bootstrap.rs` that changes WHICH
        // BODY of every `@cfg`-paired stdlib function is archived produced a
        // fresh `verum` binary reporting "Stdlib precompile cache HIT
        // (blake3 85ae0d82…)" against a `runtime.vbca` thirty minutes older
        // than the binary. The verification that followed measured the stale
        // archive and reported the edit inert. It was not inert; it had
        // never run.
        "crates/verum_compiler/src/pipeline/stdlib_bootstrap.rs",
        "crates/verum_compiler/src/core_compiler.rs",
        // `@cfg` PREDICATE SEMANTICS. `TargetConfig::is_set` / `matches` /
        // `CfgPredicate::evaluate` decide what every `@cfg(...)` in `core/`
        // MEANS, and the bake picks one arm of 45 multi-arm stdlib functions
        // by asking them — platform syscall wrappers (`sys_open`, `do_fork`,
        // `spawn_child`), architecture branches (`context_switch`), and the
        // debug-assertion family among them.
        //
        // Measured 2026-08-15: T0750 taught `is_set` the `debug` predicate,
        // and every bake afterwards was a cache HIT, so the shipped archive
        // kept the arms chosen when `debug` was still silently false —
        // `debug_assert_eq` and `debug_assert_ne` remained no-ops in a
        // binary whose own evaluator said otherwise. A compiler that
        // believes one thing and an archive built when it believed another
        // is the exact failure this list exists to prevent, and the file
        // defining the belief was not on it.
        "crates/verum_ast/src/cfg.rs",
        // Shared checked scalar rules change folded array counts in the archive.
        "crates/verum_ast/src/checked_const.rs",
        // Declaration field policies and canonical scopes shape archived access.
        "crates/verum_ast/src/visibility.rs",
        "crates/verum_vbc/src/codegen/field_visibility.rs",
        "crates/verum_types/src/core_metadata.rs",
        // `TypeId::well_known_name` (T0190) — the reserved-TypeId →
        // canonical-surface-name table `archive_metadata` renders every
        // descriptor through.  Naming one more reserved id changes the
        // BAKED metadata (a concrete `UInt32` where the sidecar used to
        // carry the existential `__opaque_type_8`) without touching a
        // single `.vr` file or any path already listed above, so without
        // this entry the extension would land in the binary and be
        // invisible in the artefact — the same silent-staleness the
        // archive_metadata.rs entry above exists to prevent.  types.rs
        // also owns the TypeId constants and the TypeRef wire shape, both
        // of which the precompiler emits directly.
        "crates/verum_vbc/src/types.rs",
        // T0701: the symbol-graph SIDECAR is part of the published
        // artefact set (runtime.symbol_graph rides alongside
        // runtime.vbca), and these two files decide what the graph
        // CONTAINS — scan_module_symbols harvests the edge rows
        // (including the `type:` return-carry markers) and
        // symbol_graph_baked owns the wire encoding.  An edge-harvest
        // change without this entry produced a fresh binary whose
        // walker looked for `type:` edges in a sidecar baked before
        // they existed — measured: `strings` found only the two code
        // literals, zero data rows, and `.dedup()` on an adapter chain
        // kept dying "method not found" through THREE rebuilds.
        "crates/verum_compiler/src/archive_ctx_loader.rs",
        "crates/verum_compiler/src/symbol_graph_baked.rs",
        // T0711 (measured trap, 2026-08-03): the VBC WIRE layer itself.
        // v2.10 PARAMNAME-CARRY changed serialize/deserialize/module/
        // format — none were in this list, so FOUR writer commits
        // produced fresh compilers that silently embedded the b63-era
        // sidecar ("cache HIT" while the wire format had moved; the
        // declared_ty carry read as EMPTY at every consumer until a
        // manual artefact delete forced a re-bake).  format.rs also
        // carries VERSION_MINOR, so wire-version bumps invalidate
        // automatically — no manual PRECOMPILE_SCHEMA_VERSION bump to
        // forget.
        "crates/verum_vbc/src/serialize.rs",
        "crates/verum_vbc/src/deserialize.rs",
        "crates/verum_vbc/src/module.rs",
        "crates/verum_vbc/src/format.rs",
        "crates/verum_vbc/src/archive.rs",
        // T1653 resolves source-export owners through the checker before
        // writing generic variant payload IDs. These selectors now affect
        // emitted archive metadata, so their source must invalidate the bake.
        // The earlier T1009 comparison established only that its particular
        // impl-variable freshening change did not alter metadata; it does
        // not cover this new declaration-selection dependency.
        "crates/verum_types/src/infer/modules.rs",
        "crates/verum_modules/src/lib.rs",
        "crates/verum_modules/src/exports.rs",
    ];
    hasher.update(b"codegen:");
    for rel in codegen_paths {
        let abs = project_root.join(rel);
        // Tell cargo to rerun build.rs when ANY of these files change —
        // belt-and-braces alongside the hash invalidation, since cargo
        // already rebuilds the crate but doesn't re-run a build script
        // for upstream-only changes.
        report_dependency(&abs);
        match std::fs::read(&abs) {
            Ok(bytes) => {
                hasher.update(rel.as_bytes());
                hasher.update(b"\0");
                hasher.update(&bytes);
                hasher.update(b"\0");
            }
            Err(_) => {
                // File missing — record the path so the cache invalidates
                // when the file is added.
                hasher.update(rel.as_bytes());
                hasher.update(b":missing\0");
            }
        }
    }
    hasher.update(b"core:");
    for (rel_path, bytes) in sorted_core_files {
        // Path-prefix the content to avoid two files swapping
        // names but keeping bytes producing the same digest.
        hasher.update(rel_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(bytes);
        hasher.update(b"\0");
    }
    hasher.finalize()
}
