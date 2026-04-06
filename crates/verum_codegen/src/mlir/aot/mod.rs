//! AOT (Ahead-of-Time) compilation.
//!
//! Compiles MLIR to object files and executables for production deployment.
//!
//! # Pipeline
//!
//! ```text
//! MLIR Module (Verum + LLVM dialects)
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  Optimization Passes                                    │
//! │  - CBGR elimination                                     │
//! │  - Context monomorphization                             │
//! │  - Standard MLIR/LLVM optimizations                     │
//! └─────────────────────────────────────────────────────────┘
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  LLVM Lowering                                          │
//! │  - convert-verum-to-scf                                 │
//! │  - convert-scf-to-cf                                    │
//! │  - convert-cf-to-llvm                                   │
//! └─────────────────────────────────────────────────────────┘
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  LLVM IR Export                                         │
//! │  - mlir-translate --mlir-to-llvmir                      │
//! └─────────────────────────────────────────────────────────┘
//!     │
//!     ├─────────────────────────────────────────┐
//!     ▼                                         ▼
//! ┌─────────────┐                         ┌─────────────┐
//! │ Object File │                         │   LLVM IR   │
//! │    (.o)     │                         │    (.ll)    │
//! └─────────────┘                         └─────────────┘
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  Linking                                                │
//! │  - Link with libverum_std                               │
//! │  - Link with system libraries                           │
//! └─────────────────────────────────────────────────────────┘
//!     │
//!     ▼
//! ┌─────────────┐
//! │ Executable  │
//! └─────────────┘
//! ```

mod compiler;

#[cfg(feature = "aot-llvm")]
mod llvm_backend;

pub use compiler::{AotCompiler, AotConfig, CompilationResult, OutputFormat};

#[cfg(feature = "aot-llvm")]
pub use llvm_backend::{LlvmBackend, lto_compile};

use crate::mlir::error::Result;
