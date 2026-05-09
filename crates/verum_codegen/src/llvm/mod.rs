//! LLVM-based code generation for the Verum compiler.
//!

//! This module provides direct VBC → LLVM IR lowering for the CPU compilation path.
//! It is the primary code generation path for:
//!

//! - **Tier 1/2 JIT**: Hot path optimization with LLVM's JIT
//! - **Tier 3 AOT**: Ahead-of-time compilation to native binaries
//!

//! # Architecture
//!

//! ```text
//! VBC Module (verum_vbc)
//!  │
//!  ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │ VbcToLlvmLowering │
//! │ - Type lowering (VBC types → LLVM types) │
//! │ - Instruction lowering (VBC ops → LLVM IR) │
//! │ - CBGR tier-aware reference handling │
//! └─────────────────────────────────────────────────────────┘
//!  │
//!  ▼
//! LLVM Module (verum_llvm)
//!  │
//!  ├────────────────┐
//!  ▼ ▼
//! ┌─────────┐ ┌─────────┐
//! │ JIT │ │ AOT │
//! │ Engine │ │ Compile │
//! └─────────┘ └─────────┘
//! ```
//!

//! # CBGR Integration
//!

//! The lowering is tier-aware, generating different code based on CBGR analysis:
//!

//! - **Tier 0**: Full runtime checks (~15ns overhead)
//!  - Generation validation on dereference
//!  - Capability checks for read/write/borrow
//!

//! - **Tier 1**: Compiler-proven safe (zero overhead)
//!  - Escape analysis proves reference validity
//!  - Direct pointer access
//!

//! - **Tier 2**: Manually marked unsafe (zero overhead)
//!  - User asserts safety via `&unsafe T`
//!  - Direct pointer access
//!

//! # Example Usage
//!

//! ```rust,ignore
//! use verum_codegen::llvm::{VbcToLlvmLowering, LoweringConfig};
//! use verum_llvm::context::Context;
//!

//! // Create LLVM context
//! let context = Context::create();
//!

//! // Configure lowering
//! let config = LoweringConfig::release("my_module");
//!

//! // Create lowering context
//! let mut lowering = VbcToLlvmLowering::new(&context, config);
//!

//! // Lower VBC module
//! lowering.lower_module(&vbc_module)?;
//!

//! // Get LLVM IR
//! println!("{}", lowering.get_ir());
//!

//! // Or take the module for further processing
//! let llvm_module = lowering.into_module();
//! ```
//!

//! # Performance Targets
//!

//! - CBGR check elimination: 50-90% typical
//! - Lowering throughput: > 100K VBC instructions/sec
//! - Generated code: within 5% of hand-written LLVM IR

// Error types
pub mod error;

// Type lowering (VBC types → LLVM types)
pub mod types;

// CBGR lowering (tier-aware reference operations)
pub mod cbgr;

// Per-function lowering context
pub mod context;

// Unified register type tracking (replacing 40+ HashSets)
pub mod register_types;

// Well-known stdlib type constants (replaces hardcoded string comparisons)
pub mod well_known_types;

// Instruction lowering (VBC instructions → LLVM IR)
pub mod instruction;

// Main VBC → LLVM lowering entry point
pub mod vbc_lowering;

// MMIO/volatile memory operations
pub mod mmio;

// Interrupt handling code generation
pub mod interrupt;

// SIMD code generation
pub mod simd;

// Inline assembly code generation
pub mod asm;

// Symbol attribute handling (linkage, visibility, aliases)
pub mod symbols;

// Bitfield accessor code generation
pub mod bitfield;

// FFI lowering (FfiExtended opcodes → LLVM IR)
pub mod ffi;

// Runtime collection/iterator helpers
pub mod runtime;

// Platform-native LLVM IR generation (replaces C runtime)
pub mod platform_ir;

// Tensor runtime as LLVM IR (replaces verum_tensor.c)
pub mod tensor_ir;

// Unicode range tables and LLVM emission helpers
pub mod unicode_data;

// Metal GPU runtime as LLVM IR (replaces verum_metal.m)
pub mod metal_ir;

// AOT permission policy — closes the script-mode security gap on
// the Tier-1 path by baking the resolved policy into the binary.
pub mod permissions;

// TARGET-triple inspection helpers. Every codegen decision that
// depends on the *target* OS / architecture (syscall numbers,
// sockaddr layout, socket-option constants, errno function names,
// …) routes through these helpers — using `#[cfg(target_os)]` HOST
// gates miscompiles cross builds.
pub mod target_triple;

// Canonical platform-syscall declaration registry. Single source of
// truth for `clock_gettime`, `nanosleep`, `read`, `write`, etc. so
// `runtime.rs` and `platform_ir.rs` agree on the (i64-uniform Verum)
// ABI and avoid LLVM IR verification failures from drifting
// declarations.
pub mod syscall_registry;

// Re-export main types
pub use asm::{AsmCall, InlineAsmGenerator};
pub use bitfield::{BitfieldLowering, BitfieldStats, min_container_bytes, optimal_container_bits};
pub use cbgr::{CbgrLowering, CbgrStats, capabilities};
pub use context::{FunctionContext, FunctionStats, TierDistribution};
pub use error::{BuildExt, LlvmLoweringError, OptionExt, Result};
pub use ffi::{FfiLowering, FfiLoweringStats, ffi_subop_to_calling_convention};
pub use interrupt::{InterruptHandlerKind, InterruptLowering, InterruptStats, TargetArch};
pub use mmio::{MmioLowering, MmioStats, RegisterWidth, VolatileOrdering};
pub use permissions::AotPermissionPolicy;
pub use register_types::{
    MethodDispatchTable, MethodDispatchTarget, RegisterType, RegisterTypeMap,
};
pub use simd::{
    SimdBinaryOp, SimdCompareOp, SimdElementKind, SimdFeatureLevel, SimdLowering, SimdReduceOp,
    SimdStats, SimdTargetArch, SimdUnaryOp,
};
pub use symbols::{
    DEFAULT_CTOR_DTOR_PRIORITY, SymbolAttributes, add_global_ctor, add_global_dtor,
    apply_to_function, apply_to_global, create_alias, emit_global_ctors, emit_global_dtors,
    linkage_to_llvm, visibility_to_llvm,
};
pub use types::{FAT_REF_SIZE, RefTier, THIN_REF_SIZE, TypeLowering};
pub use vbc_lowering::{LoweringConfig, LoweringStats, PanicStrategy, VbcToLlvmLowering};
pub use well_known_types::{WellKnownType, WellKnownTypeExt};

// Re-export verum_llvm for convenience
pub use verum_llvm;

/// LLVM version used by this crate.
pub const LLVM_VERSION: &str = verum_llvm::LLVM_VERSION;

/// Check if LLVM is available and properly configured.
pub fn check_llvm_availability() -> Result<()> {
    // Try to create a context to verify LLVM is working
    let _ctx = verum_llvm::context::Context::create();
    Ok(())
}
