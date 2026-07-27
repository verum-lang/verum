// PARKED, not silenced. The `aot-llvm` cfg gated on below is declared in
// no Cargo.toml, so all 13 gate sites are permanently false and
// `llvm_backend.rs` (422 lines) has never been compiled by anything.
//
// This is DELIBERATE and already adjudicated: see the row "Cfg gates on
// undeclared cargo features" in docs/architecture/tech-debt-register.md,
// which sweeps every cfg site in the workspace against declared features
// and EXCLUDES these sites explicitly -- "that gate is deliberate and
// self-documented ... a staged capability, not an accident". So it is not
// a misspelling of the declared `aot` feature, and must not be renamed to
// one: `aot` selects the MLIR ExecutionEngine path that ships and works,
// while `aot-llvm` was to be an alternative backend giving LTO and
// fine-grained target control. They are different backends, not spellings.
//
// Declaring the feature would not make it buildable either: that file
// imports eleven names from `verum_llvm`'s root, of which six exist
// nowhere in the crate (`Codegen`, `CodegenConfig`, `CodeGenOptLevel`,
// `OptimizationConfig`, `TargetConfig`, `Triple`) and four more live in
// submodules that are not re-exported (`FileType`, `LtoConfig`, `LtoMode`,
// `RelocMode`). It has rotted against an API that moved on, so declaring
// the feature would only convert a silent gate into a broken build.
//
// The always-taken `#[cfg(not(feature = "aot-llvm"))]` arm in
// `compiler.rs` documents the degraded fallback and warns at runtime when
// an AotConfig field that fallback cannot honour has been set. Reviving or
// deleting this backend is a subsystem-owner decision recorded on T0132;
// until then this scoped allow keeps 13 permanently-dead gates from
// drowning the crate's real warnings. Do not delete the code, and do not
// rename the gate -- either is a guess about intent with real consequences.
#![allow(unexpected_cfgs)]

//! AOT (Ahead-of-Time) compilation.
//!
//! Compiles MLIR to object files and executables for production deployment.
//!
//! # Pipeline
//!
//! ```text
//! MLIR Module (Verum + LLVM dialects)
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │ Optimization Passes │
//! │ - CBGR elimination │
//! │ - Context monomorphization │
//! │ - Standard MLIR/LLVM optimizations │
//! └─────────────────────────────────────────────────────────┘
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │ LLVM Lowering │
//! │ - convert-verum-to-scf │
//! │ - convert-scf-to-cf │
//! │ - convert-cf-to-llvm │
//! └─────────────────────────────────────────────────────────┘
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │ LLVM IR Export │
//! │ - mlir-translate --mlir-to-llvmir │
//! └─────────────────────────────────────────────────────────┘
//!  │
//!  ├─────────────────────────────────────────┐
//!  ▼ ▼
//! ┌─────────────┐ ┌─────────────┐
//! │ Object File │ │ LLVM IR │
//! │ (.o) │ │ (.ll) │
//! └─────────────┘ └─────────────┘
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │ Linking │
//! │ - Link with libverum_std │
//! │ - Link with system libraries │
//! └─────────────────────────────────────────────────────────┘
//!  │
//!  ▼
//! ┌─────────────┐
//! │ Executable │
//! └─────────────┘
//! ```

mod compiler;

#[cfg(feature = "aot-llvm")]
mod llvm_backend;

pub use compiler::{AotCompiler, AotConfig, CompilationResult, OutputFormat};

#[cfg(feature = "aot-llvm")]
pub use llvm_backend::{LlvmBackend, lto_compile};

