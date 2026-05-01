#![allow(unexpected_cfgs)]
#![allow(clippy::all)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

//! Verum Code Generation (Dual-Path: LLVM + MLIR)
//!

//! This crate provides code generation for the Verum compiler with dual-path compilation:
//!

//! - **CPU Path**: VBC → LLVM IR (via `llvm::VbcToLlvmLowering`)
//! - **GPU Path**: VBC → MLIR (via `mlir::VbcToMlirGpuLowering`)
//!

//! Both paths use VBC (Verum Bytecode) as the intermediate representation.
//!

//! # Architecture
//!

//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │ VERUM COMPILATION PIPELINE │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │ │
//! │ Verum AST → VBC Bytecode (verum_vbc::codegen) │
//! │ │ │
//! │ ┌────────────┴────────────┐ │
//! │ │ │ │
//! │ ▼ ▼ │
//! │ ┌───────────────────┐ ┌───────────────────┐ │
//! │ │ CPU PATH │ │ GPU PATH │ │
//! │ │ (VBC → LLVM IR) │ │ (VBC → MLIR) │ │
//! │ ├───────────────────┤ ├───────────────────┤ │
//! │ │ Scalar/Vector ops │ │ Tensor operations │ │
//! │ │ Control flow │ │ GPU kernels │ │
//! │ │ CBGR memory │ │ Flash attention │ │
//! │ │ Tier 1/2 JIT │ │ Device memory │ │
//! │ └─────────┬─────────┘ └─────────┬─────────┘ │
//! │ │ │ │
//! │ ▼ ▼ │
//! │ ┌───────────────────┐ ┌───────────────────┐ │
//! │ │ LLVM Backend │ │ MLIR Backend │ │
//! │ │ x86_64/aarch64 │ │ gpu → nvvm/rocdl │ │
//! │ │ +SIMD/AVX/NEON │ │ → spirv/metal │ │
//! │ └─────────┬─────────┘ └─────────┬─────────┘ │
//! │ │ │ │
//! │ └──────────┬──────────────┘ │
//! │ ▼ │
//! │ ┌───────────────────────────────────────────────────────────────────┐ │
//! │ │ LLD LINKER │ │
//! │ │ • Static/dynamic libs • Cross-platform • Embedded in verum_lld │ │
//! │ └───────────────────────────────────────────────────────────────────┘ │
//! │ │ │
//! │ ┌──────────────┼──────────────┬──────────────┐ │
//! │ ▼ ▼ ▼ ▼ │
//! │ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ │
//! │ │ JIT │ │ EXE │ │ .so/ │ │ GPU │ │
//! │ │ Execute │ │ Binary │ │ .dylib │ │ Kernel │ │
//! │ └─────────┘ └─────────┘ └─────────┘ └─────────┘ │
//! │ │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!

//! # CPU Path (Recommended for General Code)
//!

//! ```rust,ignore
//! use verum_codegen::llvm::VbcToLlvmLowering;
//!

//! // Create VBC from AST first (via verum_vbc)
//! let vbc_module = verum_vbc::codegen::lower_module(&ast)?;
//!

//! // Lower VBC to LLVM IR
//! let mut lowering = VbcToLlvmLowering::new(&llvm_ctx, &llvm_module)?;
//! lowering.lower_module(&vbc_module)?;
//!

//! // JIT or AOT compile via LLVM
//! ```
//!

//! # GPU Path (For Tensor/GPU Operations)
//!

//! ```rust,ignore
//! use verum_codegen::mlir::{VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget};
//!

//! // Create VBC from AST first
//! let vbc_module = verum_vbc::codegen::lower_module(&ast)?;
//!

//! // Lower VBC to MLIR for GPU
//! let config = GpuLoweringConfig {
//!  target: GpuTarget::Cuda,
//!  opt_level: 2,
//!  ..Default::default()
//! };
//! let ctx = verum_mlir::Context::new();
//! let mut lowering = VbcToMlirGpuLowering::new(&ctx, config);
//! lowering.lower_module(&vbc_module)?;
//! ```
//!

//! The compilation pipeline flows: Source -> Lexer -> Parser -> AST -> Type Check
//! -> VBC Bytecode -> LLVM IR -> Native Code (for AOT), or VBC -> Interpreter.
//! MLIR is used for the GPU compilation path. The VBC-first architecture ensures
//! all code goes through bytecode before native lowering.

use verum_common::{List, Text};

use std::path::Path;

// MLIR infrastructure (integrated from verum_mlir)
// Used primarily for GPU compilation path
pub mod mlir;

// LLVM-based code generation for CPU path
// VBC → LLVM IR lowering (Tier 1/2 JIT, Tier 3 AOT)
pub mod llvm;

// AOT-time analysis & transformation passes operating on VbcModule
// before lowering. See passes/mod.rs for the catalogue.
pub mod passes;

// Linking infrastructure (integrated from verum_link)
pub mod link;

// C runtime stubs for AOT compilation
pub mod runtime_stubs;

// Re-export error types
pub mod error;
pub use error::{CodegenError, Result as CodegenResult};

// Proof-term export — `verum_kernel::CoreTerm` → Lean 4 / Coq /
// Agda syntax (M-VVA-FU Sub-2.5/2.6/2.7 ).
pub mod proof_export;

// Re-export verum_mlir for low-level access
pub use verum_mlir;

// Re-export MLIR types for direct usage
pub use mlir::{
    CbgrEliminationPass,
    CbgrEliminationStats,
    ContextMonomorphizationPass,
    GpuLoweringConfig,
    GpuLoweringStats,
    GpuTarget,
    MlirCodegen,
    MlirConfig,
    // Context and config
    MlirContext,
    MlirError,

    PassConfig,
    // Passes
    PassPipeline,
    RefinementPropagationPass,
    VbcMlirError,

    // VBC → MLIR lowering (GPU path) - primary API
    VbcToMlirGpuLowering,
    // Dialect
    VerumDialect,
};

// Re-export dialect types
pub use mlir::dialect::{
    ops::{
        AwaitOp, CbgrAllocOp, CbgrCheckOp, CbgrDerefOp, CbgrDropOp, ContextGetOp, ContextProvideOp,
        ListGetOp, ListNewOp, ListPushOp, RefinementCheckOp, SelectOp, SpawnOp,
    },
    types::{
        ContextType, FutureType, ListType, MapType, MaybeType, RefTier, RefType, SetType, TextType,
        VerumType,
    },
};

// Re-export JIT
#[cfg(feature = "jit")]
pub use mlir::jit::{
    Binding, CacheConfig, CacheEntry, CacheOptions, CacheStats, CacheStatsSummary,
    CallbackRegistry, CompiledFunction, ContentHash, ContentHasher, DependencyTracker, EvalResult,
    FfiType, FunctionVersion, HistoryEntry, HotFunction, HotReloadConfig, HotReloadStats,
    HotReloadStatsSummary, HotReloader, IncrementalCache, JitArg, JitArgs, JitCallback,
    JitCompiler, JitConfig, JitEngine, JitReturn, JitStats, JitStatsSummary, ReplCommand,
    ReplConfig, ReplSession, SessionId, SessionStats, SessionStatsSummary, SignatureHasher,
    SymbolCategory, SymbolInfo, SymbolMetadata, SymbolResolver, SymbolResolverStats,
};

// Re-export AOT
#[cfg(feature = "aot")]
pub use mlir::aot::{AotCompiler, AotConfig};

// Re-export LLVM-based VBC lowering (CPU path)
pub use llvm::{
    // CBGR lowering
    CbgrLowering as LlvmCbgrLowering,
    CbgrStats as LlvmCbgrStats,
    // Function context
    FunctionContext as LlvmFunctionContext,
    // Error types
    LlvmLoweringError,
    LoweringConfig as LlvmLoweringConfig,
    LoweringStats as LlvmLoweringStats,
    RefTier as LlvmRefTier,
    // Type lowering
    TypeLowering as LlvmTypeLowering,
    // Main entry point
    VbcToLlvmLowering,
};

// Re-export linking types (V-LLSI no-libc linking)
pub use link::{
    InputFile,
    // Configuration
    LinkConfig,
    // Errors
    LinkError,
    LinkOutput,
    LinkResult,
    // Session builder
    LinkSession,
    // Linker flavor
    LinkerFlavor,
    LinkerResult,
    // LTO
    LtoConfig,
    LtoMode,
    NoLibcConfig,
    OutputFormat,
    // Platform-specific no-libc linking
    Platform,
    PreparedLink,
    ThinLtoCache,
};

// Type aliases for backward compatibility with older code
pub type Context = MlirContext;
pub type MlirBackend<'a> = MlirCodegen<'a>;
pub type Config = MlirConfig;
#[cfg(feature = "aot")]
pub type Compiler<'a> = AotCompiler<'a>;
#[cfg(feature = "aot")]
pub type CompilerConfig = AotConfig;

/// Optimization level for compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    /// No optimization (O0).
    None = 0,
    /// Basic optimization (O1).
    Less = 1,
    /// Default optimization (O2).
    Default = 2,
    /// Aggressive optimization (O3).
    Aggressive = 3,
}

impl From<OptimizationLevel> for u8 {
    fn from(level: OptimizationLevel) -> u8 {
        level as u8
    }
}

impl From<u8> for OptimizationLevel {
    fn from(level: u8) -> Self {
        match level {
            0 => OptimizationLevel::None,
            1 => OptimizationLevel::Less,
            2 => OptimizationLevel::Default,
            _ => OptimizationLevel::Aggressive,
        }
    }
}

/// Check if MLIR is properly configured.
pub fn check_mlir_availability() -> Result<(), MlirError> {
    mlir::check_mlir_availability()
}

/// Get MLIR version information.
pub fn mlir_version() -> &'static str {
    mlir::mlir_version()
}

/// Compiler version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
