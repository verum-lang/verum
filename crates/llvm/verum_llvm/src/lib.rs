//! verum_llvm - Safe LLVM bindings for Verum compiler
//!
//! This crate provides safe Rust wrappers around LLVM C API for code generation.
//! Simplified from inkwell for LLVM 21 only (no version conditionals).

#![deny(missing_debug_implementations)]
#![allow(clippy::missing_safety_doc, clippy::too_many_arguments, clippy::result_unit_err)]
// Allow unsafe operations in unsafe fn without explicit blocks (Rust 2024 compat)
// This pattern was valid in earlier editions and is used throughout this crate
#![allow(unsafe_op_in_unsafe_fn)]

#[macro_use]
extern crate verum_llvm_derive;

// Re-export verum_llvm_sys for direct FFI access when needed
pub extern crate verum_llvm_sys;

#[macro_use]
pub mod support;
#[deny(missing_docs)]
pub mod attributes;
#[deny(missing_docs)]
pub mod basic_block;
pub mod builder;
#[deny(missing_docs)]
pub mod comdat;
#[deny(missing_docs)]
pub mod context;
pub mod data_layout;
pub mod debug_info;
pub mod error;
pub mod execution_engine;
pub mod intrinsics;
// LTO disabled on MSVC: LLVMLTO.lib symbols are discarded by the single-pass
// linker before cross-crate references can resolve them.
#[cfg(all(feature = "lto", not(target_env = "msvc")))]
pub mod lto;
// Stub module on MSVC — provides type definitions but no LLVM LTO calls.
#[cfg(all(feature = "lto", target_env = "msvc"))]
pub mod lto {
    //! LTO stub for MSVC — type definitions only, no LLVM calls.

    /// LTO compilation mode.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum LtoMode { #[default] Thin, Full }

    /// LTO configuration.
    #[derive(Debug, Clone, Default)]
    pub struct LtoConfig {
        pub mode: LtoMode,
        pub cache_dir: Option<std::path::PathBuf>,
    }
    impl LtoConfig {
        pub fn new(mode: LtoMode) -> Self { Self { mode, cache_dir: None } }
        pub fn thin_with_cache(dir: impl AsRef<std::path::Path>) -> Self {
            Self { mode: LtoMode::Thin, cache_dir: Some(dir.as_ref().to_path_buf()) }
        }
    }

    /// ThinLTO cache configuration.
    #[derive(Debug, Clone, Default)]
    pub struct ThinLtoCache {
        pub dir: Option<std::path::PathBuf>,
    }
}
pub mod memory_buffer;
pub mod memory_manager;
#[deny(missing_docs)]
pub mod module;
pub mod object_file;
pub mod passes;
pub mod targets;
pub mod types;
pub mod values;

use verum_llvm_sys::target_machine::LLVMCodeGenOptLevel;
use verum_llvm_sys::{
    LLVMAtomicOrdering, LLVMAtomicRMWBinOp, LLVMDLLStorageClass, LLVMIntPredicate, LLVMRealPredicate,
    LLVMThreadLocalMode, LLVMVisibility,
};
use verum_llvm_sys::LLVMInlineAsmDialect;

pub use error::Error;
pub use context::Context as LlvmContext;
use std::convert::TryFrom;

/// Defines the address space in which a global will be inserted.
///
/// The default address space is number zero. An address space can always be created from a [`u16`]:
/// ```ignore
/// verum_llvm::AddressSpace::from(1u16);
/// ```
///
/// An address space is a 24-bit number. To convert from a [`u32`], use the [`TryFrom`] implementation:
///
/// ```ignore
/// verum_llvm::AddressSpace::try_from(42u32).expect("fits in 24-bit unsigned int");
/// ```
#[derive(Debug, PartialEq, Eq, Copy, Clone, Default)]
pub struct AddressSpace(u32);

impl From<u16> for AddressSpace {
    fn from(val: u16) -> Self {
        AddressSpace(val as u32)
    }
}

impl TryFrom<u32> for AddressSpace {
    type Error = ();

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        // address space is a 24-bit integer
        if val < 1 << 24 {
            Ok(AddressSpace(val))
        } else {
            Err(())
        }
    }
}

/// This enum defines how to compare a `left` and `right` `IntValue`.
#[llvm_enum(LLVMIntPredicate)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IntPredicate {
    /// Equal
    #[llvm_variant(LLVMIntEQ)]
    EQ,
    /// Not Equal
    #[llvm_variant(LLVMIntNE)]
    NE,
    /// Unsigned Greater Than
    #[llvm_variant(LLVMIntUGT)]
    UGT,
    /// Unsigned Greater Than or Equal
    #[llvm_variant(LLVMIntUGE)]
    UGE,
    /// Unsigned Less Than
    #[llvm_variant(LLVMIntULT)]
    ULT,
    /// Unsigned Less Than or Equal
    #[llvm_variant(LLVMIntULE)]
    ULE,
    /// Signed Greater Than
    #[llvm_variant(LLVMIntSGT)]
    SGT,
    /// Signed Greater Than or Equal
    #[llvm_variant(LLVMIntSGE)]
    SGE,
    /// Signed Less Than
    #[llvm_variant(LLVMIntSLT)]
    SLT,
    /// Signed Less Than or Equal
    #[llvm_variant(LLVMIntSLE)]
    SLE,
}

/// Defines how to compare a `left` and `right` `FloatValue`.
#[llvm_enum(LLVMRealPredicate)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FloatPredicate {
    /// Returns true if `left` == `right` and neither are NaN
    #[llvm_variant(LLVMRealOEQ)]
    OEQ,
    /// Returns true if `left` >= `right` and neither are NaN
    #[llvm_variant(LLVMRealOGE)]
    OGE,
    /// Returns true if `left` > `right` and neither are NaN
    #[llvm_variant(LLVMRealOGT)]
    OGT,
    /// Returns true if `left` <= `right` and neither are NaN
    #[llvm_variant(LLVMRealOLE)]
    OLE,
    /// Returns true if `left` < `right` and neither are NaN
    #[llvm_variant(LLVMRealOLT)]
    OLT,
    /// Returns true if `left` != `right` and neither are NaN
    #[llvm_variant(LLVMRealONE)]
    ONE,
    /// Returns true if neither value is NaN
    #[llvm_variant(LLVMRealORD)]
    ORD,
    /// Always returns false
    #[llvm_variant(LLVMRealPredicateFalse)]
    PredicateFalse,
    /// Always returns true
    #[llvm_variant(LLVMRealPredicateTrue)]
    PredicateTrue,
    /// Returns true if `left` == `right` or either is NaN
    #[llvm_variant(LLVMRealUEQ)]
    UEQ,
    /// Returns true if `left` >= `right` or either is NaN
    #[llvm_variant(LLVMRealUGE)]
    UGE,
    /// Returns true if `left` > `right` or either is NaN
    #[llvm_variant(LLVMRealUGT)]
    UGT,
    /// Returns true if `left` <= `right` or either is NaN
    #[llvm_variant(LLVMRealULE)]
    ULE,
    /// Returns true if `left` < `right` or either is NaN
    #[llvm_variant(LLVMRealULT)]
    ULT,
    /// Returns true if `left` != `right` or either is NaN
    #[llvm_variant(LLVMRealUNE)]
    UNE,
    /// Returns true if either value is NaN
    #[llvm_variant(LLVMRealUNO)]
    UNO,
}

#[llvm_enum(LLVMAtomicOrdering)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AtomicOrdering {
    #[llvm_variant(LLVMAtomicOrderingNotAtomic)]
    NotAtomic,
    #[llvm_variant(LLVMAtomicOrderingUnordered)]
    Unordered,
    #[llvm_variant(LLVMAtomicOrderingMonotonic)]
    Monotonic,
    #[llvm_variant(LLVMAtomicOrderingAcquire)]
    Acquire,
    #[llvm_variant(LLVMAtomicOrderingRelease)]
    Release,
    #[llvm_variant(LLVMAtomicOrderingAcquireRelease)]
    AcquireRelease,
    #[llvm_variant(LLVMAtomicOrderingSequentiallyConsistent)]
    SequentiallyConsistent,
}

#[llvm_enum(LLVMAtomicRMWBinOp)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AtomicRMWBinOp {
    /// Stores to memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpXchg)]
    Xchg,
    /// Adds to the value in memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpAdd)]
    Add,
    /// Subtract a value off the value in memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpSub)]
    Sub,
    /// Bitwise and into memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpAnd)]
    And,
    /// Bitwise nands into memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpNand)]
    Nand,
    /// Bitwise ors into memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpOr)]
    Or,
    /// Bitwise xors into memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpXor)]
    Xor,
    /// Sets memory to the signed-greater of the value provided and the value in memory.
    #[llvm_variant(LLVMAtomicRMWBinOpMax)]
    Max,
    /// Sets memory to the signed-lesser of the value provided and the value in memory.
    #[llvm_variant(LLVMAtomicRMWBinOpMin)]
    Min,
    /// Sets memory to the unsigned-greater of the value provided and the value in memory.
    #[llvm_variant(LLVMAtomicRMWBinOpUMax)]
    UMax,
    /// Sets memory to the unsigned-lesser of the value provided and the value in memory.
    #[llvm_variant(LLVMAtomicRMWBinOpUMin)]
    UMin,
    /// Adds to the float-typed value in memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpFAdd)]
    FAdd,
    /// Subtract a float-typed value off the value in memory and returns the prior value.
    #[llvm_variant(LLVMAtomicRMWBinOpFSub)]
    FSub,
    /// Sets memory to the greater of the two float-typed values.
    #[llvm_variant(LLVMAtomicRMWBinOpFMax)]
    FMax,
    /// Sets memory to the lesser of the two float-typed values.
    #[llvm_variant(LLVMAtomicRMWBinOpFMin)]
    FMin,
    #[llvm_variant(LLVMAtomicRMWBinOpUIncWrap)]
    UIncWrap,
    #[llvm_variant(LLVMAtomicRMWBinOpUDecWrap)]
    UDecWrap,
    #[llvm_variant(LLVMAtomicRMWBinOpUSubCond)]
    USubCond,
    #[llvm_variant(LLVMAtomicRMWBinOpUSubSat)]
    USubSat,
    #[llvm_variant(LLVMAtomicRMWBinOpFMaximum)]
    FMaximum,
    #[llvm_variant(LLVMAtomicRMWBinOpFMinimum)]
    FMinimum,
}

/// Defines the optimization level used to compile a [`Module`](crate::module::Module).
#[repr(u32)]
#[derive(Debug, Default, PartialEq, Eq, Copy, Clone)]
pub enum OptimizationLevel {
    None = 0,
    Less = 1,
    #[default]
    Default = 2,
    Aggressive = 3,
}

impl From<OptimizationLevel> for LLVMCodeGenOptLevel {
    fn from(value: OptimizationLevel) -> Self {
        match value {
            OptimizationLevel::None => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            OptimizationLevel::Less => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            OptimizationLevel::Default => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            OptimizationLevel::Aggressive => LLVMCodeGenOptLevel::LLVMCodeGenLevelAggressive,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[llvm_enum(LLVMVisibility)]
pub enum GlobalVisibility {
    #[llvm_variant(LLVMDefaultVisibility)]
    Default,
    #[llvm_variant(LLVMHiddenVisibility)]
    Hidden,
    #[llvm_variant(LLVMProtectedVisibility)]
    Protected,
}

#[allow(clippy::derivable_impls)]
impl Default for GlobalVisibility {
    fn default() -> Self {
        GlobalVisibility::Default
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadLocalMode {
    GeneralDynamicTLSModel,
    LocalDynamicTLSModel,
    InitialExecTLSModel,
    LocalExecTLSModel,
}

impl ThreadLocalMode {
    pub(crate) fn new(thread_local_mode: LLVMThreadLocalMode) -> Option<Self> {
        match thread_local_mode {
            LLVMThreadLocalMode::LLVMGeneralDynamicTLSModel => Some(ThreadLocalMode::GeneralDynamicTLSModel),
            LLVMThreadLocalMode::LLVMLocalDynamicTLSModel => Some(ThreadLocalMode::LocalDynamicTLSModel),
            LLVMThreadLocalMode::LLVMInitialExecTLSModel => Some(ThreadLocalMode::InitialExecTLSModel),
            LLVMThreadLocalMode::LLVMLocalExecTLSModel => Some(ThreadLocalMode::LocalExecTLSModel),
            LLVMThreadLocalMode::LLVMNotThreadLocal => None,
        }
    }

    pub(crate) fn as_llvm_mode(self) -> LLVMThreadLocalMode {
        match self {
            ThreadLocalMode::GeneralDynamicTLSModel => LLVMThreadLocalMode::LLVMGeneralDynamicTLSModel,
            ThreadLocalMode::LocalDynamicTLSModel => LLVMThreadLocalMode::LLVMLocalDynamicTLSModel,
            ThreadLocalMode::InitialExecTLSModel => LLVMThreadLocalMode::LLVMInitialExecTLSModel,
            ThreadLocalMode::LocalExecTLSModel => LLVMThreadLocalMode::LLVMLocalExecTLSModel,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[llvm_enum(LLVMDLLStorageClass)]
pub enum DLLStorageClass {
    #[llvm_variant(LLVMDefaultStorageClass)]
    Default,
    #[llvm_variant(LLVMDLLImportStorageClass)]
    Import,
    #[llvm_variant(LLVMDLLExportStorageClass)]
    Export,
}

#[allow(clippy::derivable_impls)]
impl Default for DLLStorageClass {
    fn default() -> Self {
        DLLStorageClass::Default
    }
}

#[llvm_enum(LLVMInlineAsmDialect)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InlineAsmDialect {
    #[llvm_variant(LLVMInlineAsmDialectATT)]
    ATT,
    #[llvm_variant(LLVMInlineAsmDialectIntel)]
    Intel,
}

/// LLVM version
pub const LLVM_VERSION: &str = "21.1";
