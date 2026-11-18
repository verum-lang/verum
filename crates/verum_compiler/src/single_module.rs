//! Lower selected source units to VBC. Bootstrap test tools can compile
//! self-contained units before the runtime archive exists. Project and test
//! pipelines share declaration collection, import resolution and body emission.

use std::sync::Arc;
use anyhow::{anyhow, Result};
use verum_ast::Module;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;

/// Compile a standalone module with the same declaration and import rules as a cog.
pub fn compile_module_with_stdlib(
    module: &Module,
    config: CodegenConfig,
    propagate_test_attr: bool,
) -> Result<VbcModule> {
    compile_unit_with_stdlib(module, &[], config, propagate_test_attr)
}

/// Compile a selected entry and its sibling modules with embedded-stdlib
/// linkage. Stateless — every call gets a fresh codegen instance, so
/// repeated calls don't leak state across compilations.
///
/// The `propagate_test_attr` flag controls whether the codegen
/// propagates `@test` attribute metadata to the output module — required
/// by the test runner so `.functions[*].is_test` survives finalize.
pub(crate) fn compile_unit_with_stdlib(
    module: &Module,
    siblings: &[Arc<Module>],
    config: CodegenConfig,
    propagate_test_attr: bool,
) -> Result<VbcModule> {
    let mut codegen = VbcCodegen::with_config(config);

    // Initial state — registers, function table, type table all empty.
    codegen.initialize();

    // Built-in core variants (`Maybe.Some` / `Result.Ok` / `Ordering.Lt`)
    // are compiler intrinsics with hardcoded tags, not part of the
    // archive. Run before archive population so any archive-side variant
    // ctor with the same simple name yields to the built-in via
    // first-wins.
    codegen.register_builtin_variants();
    codegen.register_stdlib_constants();
    codegen.register_stdlib_intrinsics();
    codegen.register_runtime_io_functions();

    // Lazy archive load. The embedded archive is the single source of
    // stdlib types and functions for non-bootstrap builds. When absent
    // (only happens during compiler bootstrap before the
    // `target/precompiled-stdlib/runtime.vbca` is produced), we still
    // proceed — the user module won't see stdlib symbols, which fails
    // loudly at the call sites rather than silently producing
    // garbage bytecode.
    if let Some(cache) = crate::archive_ctx_loader::embedded_runtime_cache() {
        // Process-wide cache: the FunctionInfo→key table is built once
        // per archive bytes and reused across compilations. The
        // `apply_lazy_with_types` variant filters by the user
        // module's mount tree + harvested call sites, so a hello-world
        // compile touches a handful of archive entries instead of all
        // 7000+.
        cache.apply_lazy_with_types(&mut codegen, module, siblings)?;
    }

    // T0363 — CODEGEN-MOUNT-ALIAS-DISPATCH-1: seed cross-module TYPE
    // re-export renames (e.g. `LayoutConstraint as Constraint`) so a
    // rename-alias used as a type namespace resolves to its carrier. Shares
    // the one `seed_reexport_type_aliases` authority with the
    // interpreter/build path (`compile_ast_to_vbc`); `verum test` reaches
    // codegen through THIS path, not that one, so without this call the
    // `term/layout/constraint` suite kept panicking on `Constraint.Length`
    // even after the pipeline leg was fixed. The embedded metadata sidecar
    // carries the archive's re-export record (T0244 true-name triples). Runs
    // after the archive apply so a module's own decls still win.
    if let Some(metadata) = crate::embedded_stdlib_metadata::get_runtime_metadata() {
        crate::pipeline::vbc_codegen::seed_reexport_type_aliases(&mut codegen, &metadata);
    }

    // **Protocol-default-method AST seed** (closes #29 user-side tail).
    //
    // The user module rarely defines stdlib protocols (`Ord` / `Eq` /
    // `Hash` / `Display` / `Debug` / `PartialOrd` / ...).  Without
    // seeding the embedded stdlib's protocol ASTs, `protocol_registry`
    // is empty when `collect_non_protocol_declarations` reaches a
    // user-side `implement Ord for OrdRegA { fn cmp(...) }` and calls
    // `generate_default_protocol_methods("Ord", "OrdRegA", {"cmp"})`.
    // The registry lookup returns None, no defaults queue, every
    // `OrdRegA.lt/le/gt/ge/max/min/clamp` call panics at runtime with
    // "method 'OrdRegA.max' not found on receiver of runtime kind
    // 'OrdRegA' (Object)".
    //
    // Match the canonical `pipeline::vbc_codegen::compile_ast_to_vbc`
    // ordering: seed first, then collect protocols from the user
    // module itself (so any locally-defined protocols layer on top of
    // the seeded stdlib).
    if let Some(protocols) = crate::pipeline::protocol_seed::embedded_protocols() {
        codegen.collect_protocol_definitions(protocols);
    }

    emit_source_bodies(&mut codegen, module, siblings, propagate_test_attr, |_| {})?;

    let mut vbc_module = codegen
        .finalize_module()
        .map_err(|e| anyhow!("VBC codegen: finalize: {:?}", e))?;

    // T0146 LEG-R: this is an EXECUTION assembly (test runner, REPL,
    // direct AST→VBC) — precompute the carried-fact protocol-dispatch
    // map on the final table, mirroring the T0106 leg-2a wiring at the
    // pipeline's three sites (source-driven `compile_ast_to_vbc`,
    // linker-merge, AOT mono). Without this, `verum test --interp`
    // executed with an EMPTY `resolved_protocol_dispatch` and the
    // dispatch-time consult was inert across the whole core-tests
    // surface. Idempotent when the pipeline delegates here and later
    // recomputes on its own (possibly merged) assembly.
    let _ = vbc_module.resolve_protocol_dispatch();
    Ok(vbc_module)
}

/// Register every source declaration before lowering any body that may use it.
pub(crate) fn emit_source_bodies(
    codegen: &mut VbcCodegen,
    entry: &Module,
    siblings: &[Arc<Module>],
    propagate_test_attr: bool,
    mark: impl Fn(&'static str),
) -> Result<()> {
    let modules: Vec<_> = std::iter::once(entry).chain(siblings.iter().map(Arc::as_ref)).collect();
    mark("collect_protocol_definitions");
    for module in &modules {
        codegen.collect_protocol_definitions(module);
    }
    mark("collect_non_protocol_declarations");
    codegen.collect_unit_declarations(&modules)
        .map_err(|err| anyhow!("VBC codegen error (unit declarations): {err}"))?;
    mark("mark_user_defined_types");
    for module in &modules {
        codegen.mark_user_defined_types(module);
    }
    mark("resolve_pending_imports");
    codegen.resolve_pending_imports();
    mark("compile_pending_default_methods");
    codegen.compile_pending_default_methods()
        .map_err(|err| anyhow!("VBC codegen error (default methods): {err}"))?;
    codegen.set_propagate_test_attr(propagate_test_attr);
    mark("compile_module_items");
    for module in modules {
        codegen.compile_module_items(module)
            .map_err(|err| anyhow!("VBC codegen error (source bodies): {err}"))?;
    }
    Ok(())
}
