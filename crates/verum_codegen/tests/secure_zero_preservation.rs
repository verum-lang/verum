//! LLVM IR audit harness — `secure_zero` volatile-memset preservation.
//!

//! Closes Action #2 of the TLS/QUIC security audit
//! (`internal/specs/tls-quic-security-audit.md` §2): "LLVM-IR audit
//! of zeroise memset preservation — follow-up."
//!

//! # What this guards against
//!

//! LLVM's standard `memset` intrinsic is non-volatile. When the
//! optimiser proves the buffer is dead immediately after the memset
//! (which is the *exact* situation we use it for — wiping secret
//! material just before scope exit), `MemCpyOptPass` /
//! `DeadStoreEliminationPass` will silently elide the call. The
//! secret stays in memory after the function returns; a coredump,
//! swap-out, post-exit heap inspection, or use-after-free read can
//! expose it.
//!

//! `verum_codegen::llvm::ffi::FfiLowering::lower_secure_zero` emits a
//! *volatile* memset — the `i1 true` flag in the call signature
//! tells LLVM the call is observable, and every optimisation pass
//! preserves it. This harness pins that property at two levels:
//!

//!  1. **Emission level**: directly asserts `lower_secure_zero`
//!  produces IR containing a volatile-memset call. Catches a
//!  future regression where someone flips the volatile bit.
//!  2. **Optimisation survival**: runs LLVM's `default<O3>` pass
//!  pipeline and asserts the volatile call survives. Catches a
//!  future regression where LLVM changes semantics of the
//!  volatile flag for memset.
//!

//! Includes a negative-control test demonstrating that an otherwise-
//! identical NON-volatile memset IS elided at -O3 — proves the
//! volatile flag is doing real work, not just decorating the IR.
//!

//! # Failure modes
//!

//! If this test ever fails:
//!  * Emission test fails → someone flipped the volatile bit in
//!  `lower_secure_zero`. Revert.
//!  * Survival test fails → LLVM upgrade changed optimiser
//!  behaviour. Investigate before merging the LLVM bump.
//!  * Negative control fails (non-volatile memset survives) → LLVM
//!  stopped DCE'ing dead memsets. Less catastrophic (volatile
//!  still works), but the framing comments may need updating.

use verum_codegen::llvm::ffi::FfiLowering;
use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::passes::PassBuilderOptions;
use verum_llvm::targets::{
    CodeModel, InitializationConfig, RelocMode, Target, TargetMachine,
};
use verum_llvm::values::FunctionValue;
use verum_llvm::OptimizationLevel;

/// Build a one-function LLVM module:
///

///  define void @secrets() {
///  %buf = alloca [32 x i8]
///  call void @llvm.memset.p0.i64(ptr %buf, i8 0, i64 32, i1 true)
///  ret void
///  }
///

/// The volatile bit on the memset is the security-critical
/// property — without it, LLVM's optimiser elides the call once
/// the buffer is proved dead at function exit.
fn build_secure_zero_module<'ctx>(ctx: &'ctx Context) -> Module<'ctx> {
    let module = ctx.create_module("secure_zero_audit");
    let void_type = ctx.void_type();
    let i8_type = ctx.i8_type();
    let i64_type = ctx.i64_type();

    let fn_type = void_type.fn_type(&[], false);
    let func: FunctionValue = module.add_function("secrets", fn_type, None);
    let entry = ctx.append_basic_block(func, "entry");

    let builder = ctx.create_builder();
    builder.position_at_end(entry);

    let buf_ty = i8_type.array_type(32);
    let buf_ptr = builder.build_alloca(buf_ty, "buf").expect("alloca");

    // Issue the *volatile* memset via the canonical FfiLowering helper —
    // the same entry point the codegen pipeline uses.
    let mut ffi = FfiLowering::new(ctx);
    let size = i64_type.const_int(32, false);
    ffi.lower_secure_zero(&builder, &module, buf_ptr, size)
        .expect("lower_secure_zero must succeed");

    builder.build_return(None).expect("build_return");

    if let Err(e) = module.verify() {
        panic!("module verification failed: {}", e.to_string());
    }

    module
}

/// Build a one-function LLVM module that uses NON-volatile memset.
/// Used as the negative control — at -O3 this memset is expected to
/// be elided (the buffer is dead at function exit), proving the
/// volatile flag in `secure_zero` is doing real work.
fn build_non_volatile_module<'ctx>(ctx: &'ctx Context) -> Module<'ctx> {
    let module = ctx.create_module("non_volatile_control");
    let void_type = ctx.void_type();
    let i8_type = ctx.i8_type();
    let i64_type = ctx.i64_type();

    let fn_type = void_type.fn_type(&[], false);
    let func = module.add_function("non_volatile", fn_type, None);
    let entry = ctx.append_basic_block(func, "entry");

    let builder = ctx.create_builder();
    builder.position_at_end(entry);

    let buf_ty = i8_type.array_type(32);
    let buf_ptr = builder.build_alloca(buf_ty, "buf").expect("alloca");

    let mut ffi = FfiLowering::new(ctx);
    let value = i64_type.const_zero();
    let size = i64_type.const_int(32, false);
    ffi.lower_memset(&builder, &module, buf_ptr, value, size)
        .expect("lower_memset must succeed");

    builder.build_return(None).expect("build_return");
    module.verify().expect("verify");
    module
}

/// Pretty-print the module to a `String`.
fn module_ir<'ctx>(module: &Module<'ctx>) -> String {
    module.print_to_string().to_string()
}

/// Initialise LLVM targets and produce a host TargetMachine — required
/// for `Module::run_passes`.
fn host_target_machine() -> TargetMachine {
    Target::initialize_native(&InitializationConfig::default())
        .expect("initialize_native");
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).expect("from_triple");
    target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Aggressive,
            RelocMode::Default,
            CodeModel::Default,
        )
        .expect("create_target_machine")
}

/// Run a named pass pipeline against a module. The pipeline string
/// follows `opt -passes=<...>` syntax from `llvm/opt`.
fn run_pipeline(module: &Module, pipeline: &str) {
    let tm = host_target_machine();
    let opts = PassBuilderOptions::create();
    module
        .run_passes(pipeline, &tm, opts)
        .unwrap_or_else(|e| panic!("run_passes({:?}) failed: {}", pipeline, e.to_string()));
    module
        .verify()
        .unwrap_or_else(|e| panic!("post-pass verify failed: {}", e.to_string()));
}

/// Count `call void @llvm.memset` (or `tail call ...`) lines in the
/// IR. We tolerate the leading `tail call` form that the optimiser
/// occasionally inserts.
fn count_memset_calls(ir: &str) -> usize {
    ir.lines()
        .filter(|l| l.contains("call") && l.contains("llvm.memset"))
        .count()
}

/// Assert the IR contains a *volatile* `llvm.memset` call.
fn assert_volatile_memset_present(label: &str, ir: &str) {
    let has_memset_call = ir
        .lines()
        .any(|l| l.contains("call") && l.contains("llvm.memset"));
    let has_volatile_true = ir.contains("i1 true");
    if !(has_memset_call && has_volatile_true) {
        eprintln!("--- IR at {} ---\n{}\n--- end IR ---", label, ir);
        panic!(
            "{}: expected volatile memset preserved (memset_call={}, volatile_true={})",
            label, has_memset_call, has_volatile_true
        );
    }
}

// =============================================================================
// Emission-level smoke test
// =============================================================================

/// Baseline: the helper produces a volatile memset before any
/// optimisation runs. Cheapest sanity check — if this fails, the
/// source-level emission is wrong (volatile bit missing).
#[test]
fn secure_zero_emits_volatile_memset() {
    let ctx = Context::create();
    let module = build_secure_zero_module(&ctx);
    let ir = module_ir(&module);

    assert!(ir.contains("@secrets"), "secrets fn missing:\n{}", ir);
    assert_volatile_memset_present("emission/-O0", &ir);
}

// =============================================================================
// Optimiser-survival tests — the load-bearing assertions
// =============================================================================

#[test]
fn secure_zero_survives_o1() {
    let ctx = Context::create();
    let module = build_secure_zero_module(&ctx);
    run_pipeline(&module, "default<O1>");
    let ir = module_ir(&module);
    assert_volatile_memset_present("post-O1", &ir);
}

#[test]
fn secure_zero_survives_o2() {
    let ctx = Context::create();
    let module = build_secure_zero_module(&ctx);
    run_pipeline(&module, "default<O2>");
    let ir = module_ir(&module);
    assert_volatile_memset_present("post-O2", &ir);
}

#[test]
fn secure_zero_survives_o3() {
    let ctx = Context::create();
    let module = build_secure_zero_module(&ctx);
    run_pipeline(&module, "default<O3>");
    let ir = module_ir(&module);
    assert_volatile_memset_present("post-O3", &ir);
}

#[test]
fn secure_zero_survives_dse_targeted() {
    // Run *only* DeadStoreEliminationPass — the historical pass that
    // would elide a non-volatile dead memset. This test pins the
    // smaller property that volatile escapes DSE specifically, even
    // when other O3 passes aren't running.
    let ctx = Context::create();
    let module = build_secure_zero_module(&ctx);
    run_pipeline(&module, "function(dse)");
    let ir = module_ir(&module);
    assert_volatile_memset_present("post-DSE", &ir);
}

// =============================================================================
// Negative control — non-volatile memset IS elided at -O3
// =============================================================================
//

// This test demonstrates *why* the volatile flag matters: an
// otherwise-identical module that uses a non-volatile memset has
// the call elided at -O3. If this test ever STARTS FAILING (i.e.,
// the non-volatile memset survives), it means LLVM has changed
// behaviour or the optimiser pipeline isn't running as expected —
// the volatile-survival tests above remain valid (volatile is
// observable regardless), but the framing comment in
// `lower_secure_zero` may need updating.

#[test]
fn negative_control_non_volatile_memset_elided_at_o3() {
    let ctx = Context::create();
    let module = build_non_volatile_module(&ctx);

    // Sanity: pre-optimisation IR has the memset call.
    let pre_ir = module_ir(&module);
    let pre_count = count_memset_calls(&pre_ir);
    assert!(
        pre_count >= 1,
        "pre-O3 control IR should have at least one memset call:\n{}",
        pre_ir
    );

    run_pipeline(&module, "default<O3>");
    let ir = module_ir(&module);

    let post_count = count_memset_calls(&ir);
    if post_count > 0 {
        let surviving: Vec<&str> = ir
            .lines()
            .filter(|l| l.contains("call") && l.contains("llvm.memset"))
            .collect();
        eprintln!("--- IR ---\n{}\n--- end IR ---", ir);
        panic!(
            "negative control failed: {} non-volatile memset call(s) survived -O3 (expected 0): {:?}",
            post_count, surviving
        );
    }
}
