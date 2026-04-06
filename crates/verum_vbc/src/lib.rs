//! # Verum Bytecode (VBC)
//!
//! VBC is the unified intermediate representation for the Verum compiler. It serves three purposes:
//!
//! 1. **Interpreter execution** (Tier 0) - direct interpretation with inline caching
//! 2. **JIT compilation** (Tier 1-2) - lowering to MLIR for runtime compilation
//! 3. **AOT compilation** (Tier 3) - lowering to MLIR → LLVM → native code
//!
//! ## Design Principles
//!
//! - **Register-based**: 2x fewer instructions than stack-based (Lua, Dalvik)
//! - **Typed operands**: Direct dispatch without boxing for primitives
//! - **Reified generics**: Type parameters preserved for specialization
//! - **Lazy specialization**: Only hot paths get specialized
//! - **Compact encoding**: Variable-length instructions, deduplication
//! - **CBGR-aware**: Memory safety operations as first-class instructions
//!
//! ## Module Structure
//!
//! - [`format`]: VBC binary format definitions (header, sections)
//! - [`types`]: Type system (TypeId, TypeRef, TypeDescriptor)
//! - [`instruction`]: Instruction set (256 opcodes)
//! - [`module`]: VbcModule and VbcFunction structures
//! - [`value`]: NaN-boxed runtime values
//! - [`serialize`]: Binary serialization
//! - [`deserialize`]: Binary deserialization
//! - [`validate`]: Module validation
//!
//! ## Performance Targets
//!
//! | Metric | Target |
//! |--------|--------|
//! | Serialize 10K functions | < 100ms |
//! | Deserialize 10K functions | < 50ms |
//! | Round-trip correctness | 100% |

#![warn(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]

pub mod archive;
pub mod dtype;
pub mod error;
pub mod format;
pub mod instruction;
pub mod module;
pub mod types;
pub mod value;

pub mod bytecode;
pub mod compression;
pub mod encoding;
pub mod serialize;
pub mod deserialize;
pub mod validate;

/// VBC module disassembler for human-readable bytecode dumps.
pub mod disassemble;

pub mod interpreter;

/// VBC monomorphization for generic function specialization.
///
/// Transforms generic VBC bytecode into specialized bytecode by:
/// - Substituting type parameters with concrete types
/// - Rewriting CALL_G to CALL with specialized function IDs
/// - Specializing generic arithmetic/comparison to typed variants
pub mod mono;

/// CBGR codegen abstractions (dereference strategies, capability checks).
pub mod cbgr;

/// VBC-level CBGR escape analysis for reference tier promotion.
///
/// Analyzes Ref/RefMut instructions to determine which can be promoted from
/// Tier 0 (runtime-checked, ~15ns) to Tier 1 (compiler-proven safe, 0ns).
/// Runs between VBC codegen and LLVM lowering.
pub mod cbgr_analysis;

/// Tensor metadata for GPU and distributed computation.
///
/// Provides compile-time metadata for tensor operations as defined
/// in the tensor-GPU architecture specification:
/// - Shape verification with symbolic dimensions
/// - Device placement hints
/// - Distribution topology for distributed training
/// - MLIR lowering hints for optimization
/// - Autodiff graph for gradient computation
pub mod metadata;

/// Industrial-grade FFI runtime for VBC interpreter.
///
/// Provides zero-to-minimal overhead FFI support for all tiers:
/// - Tier 0 (Interpreter): ~150ns using libffi dynamic dispatch
/// - Tier 1-2 (JIT): ~15ns with symbol caching
/// - Tier 3 (AOT): ~5ns via direct calls
pub mod ffi;

/// Industrial-grade intrinsic system for zero-overhead intrinsic compilation.
///
/// Maps intrinsic names to optimal VBC instruction sequences:
/// - Direct opcodes: Zero overhead (add_i64 → AddI)
/// - Inline sequences: Near-zero overhead (checked_add → Add + overflow check)
/// - Library calls: Minimal overhead for complex operations
#[cfg(feature = "codegen")]
pub mod intrinsics;

#[cfg(feature = "codegen")]
pub mod codegen;

/// TokenStream serialization for VBC heap storage.
///
/// Provides serialization and deserialization of TokenStream for the VBC meta-system.
/// When a meta function generates code via `quote { ... }`, the resulting TokenStream
/// is serialized to binary format and stored on the VBC interpreter's heap.
///
/// The module is enabled with the `codegen` feature since it requires verum_lexer.
#[cfg(feature = "codegen")]
pub mod token_stream;

// Re-exports for convenience
pub use archive::{
    ArchiveBuilder, ArchiveFlags, ArchiveHeader, ModuleEntry, VbcArchive,
    read_archive, read_archive_from_file, write_archive, write_archive_to_file,
    // Metadata stripping (VBC Optimization Audit Phase 3)
    strip_module_metadata, estimate_stripping_savings,
    // Compression support (VBC Optimization Audit Phase 3)
    compress_data, decompress_data, DEFAULT_COMPRESSION_LEVEL,
};
pub use error::{VbcError, VbcResult};
pub use format::{VbcFlags, VbcHeader, MAGIC, VERSION_MAJOR, VERSION_MINOR};
pub use instruction::{
    ArithSubOpcode, CbgrSubOpcode, CmpSubOpcode, FfiSubOpcode, GpuSubOpcode, Instruction,
    MathSubOpcode, Opcode, Reg, RegRange,
};
pub use module::{ConstId, FunctionDescriptor, FunctionId, VbcFunction, VbcModule};
pub use types::{
    CbgrTier, FieldDescriptor, Mutability, PropertySet, ProtocolId, StringId, TypeDescriptor,
    TypeId, TypeKind, TypeParamDescriptor, TypeParamId, TypeRef, VariantDescriptor, VariantKind,
    Variance, Visibility,
};
pub use value::{reset_global_value_tables, Capabilities, FatRef, ThinRef, Value};

// Re-export CBGR codegen types
pub use cbgr::{
    CapabilityCheckCodegen, CbgrCodegenStats, CbgrDereferenceStrategy, DereferenceCodegen,
    RequiredCapability,
};

// Re-export FFI types
pub use ffi::{CTypeRuntime, FfiPlatform, FfiPlatformError, LibraryHandle};

// Re-export metadata types for tensor operations
pub use metadata::{
    AutodiffGraph, DeviceHints, DevicePreference, DeviceType, DistributionMetadata,
    MeshTopology, MlirHints, ShapeMetadata, ShardingSpec, StaticShape,
};
#[cfg(feature = "ffi")]
pub use ffi::{create_platform, FfiError, FfiRuntime, MarshalError, Marshaller, ResolvedSymbol};

#[cfg(test)]
mod tests;
