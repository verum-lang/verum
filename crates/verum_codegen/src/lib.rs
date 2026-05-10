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

/// Per-variant projection for [`OptimizationLevel`].
///
/// `name` is the canonical kebab-case identifier; `cli_flag` is the
/// `-O<N>` form used at the LLVM/clang invocation boundary; `level`
/// is the dense u8 0..=3 used by both downstream LLVM and the
/// compiler-options surface. The triple round-trips through itself:
/// `from_str(x.name()) == Some(x)`, `from_u8(x.level()) == x`.
#[derive(Debug, Clone, Copy)]
pub struct OptimizationLevelMeta {
    pub name: &'static str,
    pub cli_flag: &'static str,
    pub level: u8,
}

impl OptimizationLevel {
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::Less,
        Self::Default,
        Self::Aggressive,
    ];

    pub const fn meta(self) -> OptimizationLevelMeta {
        match self {
            Self::None => OptimizationLevelMeta {
                name: "none",
                cli_flag: "-O0",
                level: 0,
            },
            Self::Less => OptimizationLevelMeta {
                name: "less",
                cli_flag: "-O1",
                level: 1,
            },
            Self::Default => OptimizationLevelMeta {
                name: "default",
                cli_flag: "-O2",
                level: 2,
            },
            Self::Aggressive => OptimizationLevelMeta {
                name: "aggressive",
                cli_flag: "-O3",
                level: 3,
            },
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// `-O0` / `-O1` / `-O2` / `-O3` flag form.
    #[inline]
    pub const fn cli_flag(&self) -> &'static str {
        self.meta().cli_flag
    }
}

impl From<OptimizationLevel> for u8 {
    fn from(level: OptimizationLevel) -> u8 {
        level.meta().level
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

#[cfg(test)]
mod meta_consolidation_pins {
    use super::OptimizationLevel;

    #[test]
    fn optimization_level_round_trip_unique_and_dense_level() {
        assert_eq!(OptimizationLevel::ALL.len(), 4);
        for v in OptimizationLevel::ALL {
            let s = v.as_str();
            assert_eq!(
                OptimizationLevel::from_str(s),
                Some(*v),
                "OptimizationLevel::{:?}: name '{}' round-trip",
                v,
                s
            );
        }
        assert!(OptimizationLevel::from_str("__not_an_opt_level__").is_none());

        // Dense u8 level 0..=3 in declaration order.
        for (i, v) in OptimizationLevel::ALL.iter().enumerate() {
            let level: u8 = (*v).into();
            assert_eq!(level as usize, i);
        }
        // u8 → enum round-trip for every level (saturates at
        // Aggressive for >3 — preserved from legacy).
        assert_eq!(
            OptimizationLevel::from(0_u8),
            OptimizationLevel::None
        );
        assert_eq!(
            OptimizationLevel::from(3_u8),
            OptimizationLevel::Aggressive
        );
        assert_eq!(
            OptimizationLevel::from(99_u8),
            OptimizationLevel::Aggressive,
            "u8 saturates at Aggressive (legacy invariant)"
        );

        // CLI flag form.
        assert_eq!(OptimizationLevel::None.cli_flag(), "-O0");
        assert_eq!(OptimizationLevel::Less.cli_flag(), "-O1");
        assert_eq!(OptimizationLevel::Default.cli_flag(), "-O2");
        assert_eq!(OptimizationLevel::Aggressive.cli_flag(), "-O3");
        // Cross-pin: cli_flag's last char matches the level digit.
        for v in OptimizationLevel::ALL {
            let level: u8 = (*v).into();
            let flag = v.cli_flag();
            assert_eq!(
                flag.as_bytes().last().copied().map(|b| b - b'0'),
                Some(level),
                "cli_flag → level parity drift on {:?}",
                v
            );
        }
    }
}
