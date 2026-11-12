//! Automatic stdlib archive identity shared by the build script and its controls.
//! The caller supplies core files in relative-path order; producer files use
//! sorted checkout-relative paths. Dependency reporting remains independent of baking.

use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(test)]
pub(crate) fn compute_archive_key<'a>(
    project_root: &Path,
    schema: &str,
    sorted_core_files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    report_dependency: impl FnMut(&Path),
) -> blake3::Hash {
    let producer = producer_hasher(project_root, schema, report_dependency);
    finish_archive_key(producer, sorted_core_files)
}

pub(crate) fn producer_hasher(
    project_root: &Path,
    schema: &str,
    mut report_dependency: impl FnMut(&Path),
) -> blake3::Hasher {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"verum-stdlib-producer");
    hash_part(&mut hasher, schema.as_bytes());
    // These inputs live in crates shared with runtime-only tooling. AST traversal
    // and source lowering have dedicated source trees, discovered below.
    let producer_files = [
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
        "crates/verum_vbc/src/intrinsics/lowering.rs",
        // Actual array storage evidence, callable conversions, and declared
        // element semantics all shape the bytecode emitted into the archive.
        "crates/verum_vbc/src/array_storage.rs",
        "crates/verum_vbc/src/mono/method_calls.rs",
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
        "crates/verum_vbc/src/value_use.rs",
        "crates/verum_vbc/src/resource_discipline.rs",
        "crates/verum_common/src/value_use.rs",
        "crates/verum_vbc/src/type_layout.rs",
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
        "crates/verum_compiler/build_support/stdlib_cache.rs",
        "crates/verum_compiler/build_support/bake_receipt.rs",
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
    let mut paths: Vec<PathBuf> = producer_files.iter().map(PathBuf::from).collect();
    for tree in ["crates/verum_ast/src", "crates/verum_vbc/src/codegen"] {
        // Cargo watches directory membership, including newly added lowering modules.
        report_dependency(&project_root.join(tree));
        collect_rust_sources(project_root, Path::new(tree), &mut paths)
            .unwrap_or_else(|e| panic!("cannot enumerate producer sources in {tree}: {e}"));
    }
    paths.sort_unstable();
    for relative in paths {
        let absolute = project_root.join(&relative);
        report_dependency(&absolute);
        let name = relative
            .to_str()
            .expect("UTF-8 producer input path")
            .replace('\\', "/");
        hash_part(&mut hasher, name.as_bytes());
        match std::fs::read(&absolute) {
            Ok(bytes) => {
                hasher.update(b"present");
                hash_part(&mut hasher, &bytes);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                hasher.update(b"missing");
            }
            Err(error) => panic!("cannot read producer input {name}: {error}"),
        }
    }
    hasher
}

fn hash_part(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn collect_rust_sources(root: &Path, relative: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut directories = vec![relative.to_owned()];
    while let Some(directory) = directories.pop() {
        let entries = match std::fs::read_dir(root.join(&directory)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound && directory == relative => {
                continue;
            }
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = directory.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_symlink() && entry.path().is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("symlinked producer directory {}", path.display()),
                ));
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                paths.push(path);
            }
        }
    }
    Ok(())
}

pub(crate) fn finish_archive_key<'a>(
    mut hasher: blake3::Hasher,
    sorted_core_files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> blake3::Hash {
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
