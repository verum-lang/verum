//! Canonical entry point for compiling a single Verum AST module to VBC.
//!
//! Single source of truth for stdlib-aware codegen used by:
//!   * Test runners (`crates/verum_cli/src/commands/test.rs`)
//!   * REPL single-input compilation
//!   * IDE diagnostic compilation
//!   * Direct AST-to-VBC paths that don't need the full session pipeline
//!
//! The full compilation pipeline (`pipeline::vbc_codegen::compile_ast_to_vbc`)
//! also delegates here for the codegen body, so dependent verification +
//! tier analysis remain pipeline concerns while archive-driven stdlib
//! linkage and the codegen passes themselves stay in one place.
//!
//! Architecture:
//!   1. Initialize a fresh `VbcCodegen` with the given config.
//!   2. Register the four built-in surfaces every module needs
//!      (variants, stdlib constants, stdlib intrinsics, runtime I/O).
//!   3. Lazy-load the embedded stdlib `VbcArchive` for symbols
//!      transitively referenced by the user module (mounts +
//!      static-method calls + type references).
//!   4. Run the user-side codegen passes:
//!      protocols → declarations → user-type marking → import
//!      resolution → default-method bodies → item bodies → finalize.
//!
//! Steps 2-4 mirror `compile_ast_to_vbc`; step 3 was missing from
//! `VbcCodegen::compile_module` and from the test-runner direct path,
//! producing the "method 'X' not found on receiver of runtime kind 'Y'"
//! class of failures whenever stdlib was referenced.

use anyhow::{anyhow, Result};
use verum_ast::Module;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;

/// Compile a single AST module to a VBC module with embedded-stdlib
/// linkage. Stateless — every call gets a fresh codegen instance, so
/// repeated calls don't leak state across compilations.
///
/// The `propagate_test_attr` flag controls whether the codegen
/// propagates `@test` attribute metadata to the output module — required
/// by the test runner so `.functions[*].is_test` survives finalize.
pub fn compile_module_with_stdlib(
    module: &Module,
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
    if let Some(archive) = crate::embedded_stdlib_vbc::get_runtime_archive() {
        // Process-wide cache: the FunctionInfo→key table is built once
        // per archive bytes and reused across compilations. The
        // `apply_lazy_with_types` variant filters by the user
        // module's mount tree + harvested call sites, so a hello-world
        // compile touches a handful of archive entries instead of all
        // 7000+.
        static CTX_CACHE: crate::archive_ctx_loader::ArchiveCtxCache =
            crate::archive_ctx_loader::ArchiveCtxCache::new();
        CTX_CACHE.apply_lazy_with_types(archive, &mut codegen, module);
    }

    // User-side passes. Same order as `compile_ast_to_vbc`:
    //   protocols (so default-method inheritance is in place) →
    //   declarations (types/functions/consts) →
    //   user-type marking (so codegen can distinguish user types
    //     from archive-imported types in disambiguation tables) →
    //   import resolution (resolves pending mount-driven aliases) →
    //   default-method bodies (impl<T: Protocol> { ... } shadows) →
    //   item bodies (the actual function bytecode emission) →
    //   finalize (build VbcModule from accumulated state).
    codegen.collect_protocol_definitions(module);
    codegen
        .collect_non_protocol_declarations(module)
        .map_err(|e| anyhow!("VBC codegen: declarations: {:?}", e))?;
    codegen.mark_user_defined_types(module);
    codegen.resolve_pending_imports();
    codegen
        .compile_pending_default_methods()
        .map_err(|e| anyhow!("VBC codegen: default methods: {:?}", e))?;
    if propagate_test_attr {
        codegen.set_propagate_test_attr(true);
    }
    codegen
        .compile_module_items(module)
        .map_err(|e| anyhow!("VBC codegen: bodies: {:?}", e))?;

    codegen
        .finalize_module()
        .map_err(|e| anyhow!("VBC codegen: finalize: {:?}", e))
}
