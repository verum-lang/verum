//! MLIR-based code generation for Verum GPU compilation.
//!
//! This module provides the GPU compilation path for the Verum compiler, targeting
//! tensor operations and GPU-accelerated workloads via MLIR.
//!
//! # Architecture
//!
//! ```text
//! Verum AST → VBC Bytecode
//!                  │
//!         ┌───────┴───────┐
//!         ▼               ▼
//!    CPU Path        GPU Path (this module)
//!  (LLVM IR via      (MLIR via verum_mlir)
//!   verum_llvm)            │
//!                          ▼
//!              ┌─────────────────────────────┐
//!              │     Verum MLIR Dialect      │
//!              │  - verum.tensor.*           │
//!              │  - verum.cbgr.*             │
//!              │  - verum.context.*          │
//!              └─────────────────────────────┘
//!                          │
//!                          ▼ MLIR Passes
//!              ┌─────────────────────────────┐
//!              │  verum.tensor → linalg      │
//!              │  linalg → scf → gpu         │
//!              │  gpu → target-specific      │
//!              └─────────────────────────────┘
//!                          │
//!         ┌────────────────┼────────────────┐
//!         ▼                ▼                ▼
//!      CUDA            ROCm            Vulkan
//!    (PTX/NVVM)      (HSACO)         (SPIR-V)
//! ```
//!
//! # GPU Path Usage
//!
//! ```rust,ignore
//! use verum_codegen::mlir::{VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget};
//! use verum_vbc::VbcModule;
//!
//! // Create GPU lowering with configuration
//! let config = GpuLoweringConfig {
//!     target: GpuTarget::Cuda,
//!     opt_level: 2,
//!     enable_tensor_cores: true,
//!     max_shared_memory: 48 * 1024,
//!     default_block_size: [256, 1, 1],
//!     enable_async_copy: true,
//!     debug_info: false,
//! };
//!
//! // Lower VBC to MLIR for GPU
//! let ctx = verum_mlir::Context::new();
//! let mut lowering = VbcToMlirGpuLowering::new(&ctx, config);
//! lowering.lower_module(&vbc_module)?;
//!
//! // Get MLIR module for further processing
//! let mlir_module = lowering.take_module();
//! ```
//!
//! # GPU Targets
//!
//! | Target | Output | Use Case |
//! |--------|--------|----------|
//! | CUDA | PTX/NVVM IR | NVIDIA GPUs via CUDA |
//! | ROCm | HSACO/ROCDL | AMD GPUs via ROCm |
//! | Vulkan | SPIR-V | Cross-platform compute |
//! | Metal | Metal IR | Apple GPUs |
//! | CpuFallback | LLVM IR | CPU emulation for testing |
//!
//! # Performance Targets
//!
//! - Tensor fusion: 30-50% memory bandwidth reduction
//! - Kernel launch overhead: <5μs amortized
//! - GPU utilization: >80% for compute-bound kernels
//! - CBGR check elimination: 40-70% of checks removable

// Re-export melior types for convenience
pub use verum_mlir;

// Core modules
pub mod context;
pub mod error;

// VBC → MLIR lowering for GPU path (primary entry point)
pub mod vbc_lowering;

// Verum Dialect definitions
pub mod dialect;

// Optimization passes
pub mod passes;

// JIT compilation
#[cfg(feature = "jit")]
pub mod jit;

// AOT compilation
#[cfg(feature = "aot")]
pub mod aot;

// Re-export main types
pub use context::{MlirCodegen, MlirConfig, MlirContext};
pub use error::{MlirError, Result};

// Re-export dialect types
pub use dialect::{
    VerumDialect,
    ops::{
        CbgrAllocOp, CbgrCheckOp, CbgrDerefOp, CbgrDropOp,
        ContextGetOp, ContextProvideOp,
        SpawnOp, AwaitOp, SelectOp,
        RefinementCheckOp,
        ListNewOp, ListPushOp, ListGetOp,
    },
    types::{
        VerumType, RefType, RefTier,
        ListType, MapType, SetType, TextType, MaybeType,
        FutureType, ContextType,
    },
};

// Re-export passes (including GPU pipeline)
pub use passes::{
    PassPipeline, PassConfig,
    GpuPassPipeline, GpuPassConfig, GpuPipelineResult, GpuPipelineStats,
    CbgrEliminationPass, CbgrEliminationStats,
    ContextMonomorphizationPass,
    RefinementPropagationPass,
};

// Re-export JIT (Phase 4 complete implementation)
#[cfg(feature = "jit")]
pub use jit::{
    // Core JIT types
    JitEngine, JitConfig, JitStats, JitStatsSummary, JitCompiler,
    JitArg, JitArgs, JitReturn, CompiledFunction,
    CallbackRegistry, JitCallback,

    // Symbol resolution
    SymbolResolver, SymbolResolverStats, SymbolInfo, SymbolMetadata,
    SymbolCategory, FfiType,

    // Incremental compilation
    IncrementalCache, CacheConfig, CacheEntry, CacheOptions,
    CacheStats, CacheStatsSummary, DependencyTracker,
    ContentHash, ContentHasher,

    // REPL integration
    ReplSession, ReplConfig, SessionId, EvalResult, Binding,
    HistoryEntry, ReplCommand, SessionStats, SessionStatsSummary,

    // Hot code replacement
    HotReloader, HotReloadConfig, HotFunction, FunctionVersion,
    HotReloadStats, HotReloadStatsSummary, SignatureHasher,
};

// Re-export AOT
#[cfg(feature = "aot")]
pub use aot::{AotCompiler as MlirAotCompiler, AotConfig as MlirAotConfig};

// Re-export VBC → MLIR lowering (GPU path - primary API)
pub use vbc_lowering::{
    VbcToMlirGpuLowering, GpuLoweringConfig, GpuLoweringStats,
    GpuTarget, VbcMlirError,
};

/// Compiler version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Check if MLIR is properly configured.
pub fn check_mlir_availability() -> Result<()> {
    // Try to create a basic context to verify MLIR is available
    let _ctx = verum_mlir::Context::new();
    Ok(())
}

/// Get MLIR version information.
pub fn mlir_version() -> &'static str {
    // mlir-sys version corresponds to LLVM version
    "LLVM 21.0.x (via mlir-sys 210.0.1)"
}
