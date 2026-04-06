//! Industrial-grade FFI runtime for VBC interpreter.
//!
//! This module provides FFI support for the 2-tier execution model:
//!
//! | Tier | Approach | First Call | Subsequent | Memory |
//! |------|----------|------------|------------|--------|
//! | Interpreter | Dynamic (libffi) | ~5us | ~150ns | 200B/symbol |
//! | AOT | Static (LLVM) | N/A | ~5ns | minimal |
//!
//! The interpreter uses libffi for dynamic FFI calls because types are only
//! known at runtime from VBC metadata. AOT compilation generates direct
//! `call` instructions with proper C ABI, achieving zero-cost FFI.
//!
//! # Architecture
//!
//! ```text
//! +-------------------+
//! |    FfiRuntime     |  <- Interpreter FFI (this module)
//! +-------------------+
//!          |
//!    +-----+-----+
//!    |           |
//! +------+  +--------+
//! |Platform|  |Marshal|  <- Platform abstraction + Type conversion
//! +------+  +--------+
//!    |
//!    +----+----+----+
//!    |    |    |    |
//! Darwin Linux Win  ...
//!
//! For AOT: VBC FfiExtended opcodes → LLVM IR → native call instructions
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use verum_vbc::ffi::{FfiRuntime, FfiPlatform};
//!
//! let mut runtime = FfiRuntime::new()?;
//!
//! // Load a library
//! let libc = runtime.load_library("libc")?;
//!
//! // Resolve a symbol
//! let getpid = runtime.resolve_symbol(libc, "getpid")?;
//!
//! // Call the function
//! let result = runtime.call_c(getpid, &[], CType::I32)?;
//! ```

pub mod platform;

#[cfg(feature = "ffi")]
pub mod marshal;
#[cfg(feature = "ffi")]
pub mod runtime;
#[cfg(feature = "ffi")]
pub mod trampolines;

// Re-exports
pub use platform::{FfiPlatform, FfiPlatformError, LibraryHandle};
#[cfg(feature = "ffi")]
pub use platform::create_platform;
#[cfg(feature = "ffi")]
pub use marshal::{ArrayBufferInfo, MarshalError, Marshaller};
#[cfg(feature = "ffi")]
pub use runtime::{FfiError, FfiRuntime, ResolvedSymbol};

/// FFI C type enumeration for marshalling.
///
/// This mirrors the CType in module.rs but is used at runtime.
/// For struct-by-value types, we use StructValue which carries the layout index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CTypeRuntime {
    /// void
    Void,
    /// int8_t / char
    I8,
    /// int16_t / short
    I16,
    /// int32_t / int
    I32,
    /// int64_t / long long
    I64,
    /// uint8_t / unsigned char
    U8,
    /// uint16_t / unsigned short
    U16,
    /// uint32_t / unsigned int
    U32,
    /// uint64_t / unsigned long long
    U64,
    /// float
    F32,
    /// double
    F64,
    /// void* / generic pointer
    Ptr,
    /// const char* / C string
    CStr,
    /// bool (C99 _Bool)
    Bool,
    /// size_t
    Size,
    /// ssize_t / ptrdiff_t
    Ssize,
    /// Pointer to struct (carries layout index into module's ffi_layouts)
    StructPtr(u16),
    /// Pointer to array
    ArrayPtr,
    /// Function pointer
    FnPtr,
    /// Struct passed/returned by value (carries layout index into module's ffi_layouts)
    StructValue(u16),
}

impl CTypeRuntime {
    /// Creates a CTypeRuntime from a CType with an optional layout index.
    ///
    /// For StructValue and StructPtr types, the layout_idx must be provided.
    /// For other types, layout_idx is ignored.
    pub fn from_ctype_with_layout(ct: crate::module::CType, layout_idx: Option<u16>) -> Self {
        match ct {
            crate::module::CType::Void => Self::Void,
            crate::module::CType::I8 => Self::I8,
            crate::module::CType::I16 => Self::I16,
            crate::module::CType::I32 => Self::I32,
            crate::module::CType::I64 => Self::I64,
            crate::module::CType::U8 => Self::U8,
            crate::module::CType::U16 => Self::U16,
            crate::module::CType::U32 => Self::U32,
            crate::module::CType::U64 => Self::U64,
            crate::module::CType::F32 => Self::F32,
            crate::module::CType::F64 => Self::F64,
            crate::module::CType::Ptr => Self::Ptr,
            crate::module::CType::CStr => Self::CStr,
            crate::module::CType::Bool => Self::Bool,
            crate::module::CType::Size => Self::Size,
            crate::module::CType::Ssize => Self::Ssize,
            crate::module::CType::StructPtr => {
                Self::StructPtr(layout_idx.expect("StructPtr requires layout_idx"))
            }
            crate::module::CType::ArrayPtr => Self::ArrayPtr,
            crate::module::CType::FnPtr => Self::FnPtr,
            crate::module::CType::StructValue => {
                Self::StructValue(layout_idx.expect("StructValue requires layout_idx"))
            }
        }
    }
}

impl From<crate::module::CType> for CTypeRuntime {
    /// Converts CType to CTypeRuntime.
    ///
    /// Note: For StructValue and StructPtr, this panics. Use `CTypeRuntime::from_ctype_with_layout`
    /// when you need to convert struct-related types.
    fn from(ct: crate::module::CType) -> Self {
        match ct {
            crate::module::CType::Void => Self::Void,
            crate::module::CType::I8 => Self::I8,
            crate::module::CType::I16 => Self::I16,
            crate::module::CType::I32 => Self::I32,
            crate::module::CType::I64 => Self::I64,
            crate::module::CType::U8 => Self::U8,
            crate::module::CType::U16 => Self::U16,
            crate::module::CType::U32 => Self::U32,
            crate::module::CType::U64 => Self::U64,
            crate::module::CType::F32 => Self::F32,
            crate::module::CType::F64 => Self::F64,
            crate::module::CType::Ptr => Self::Ptr,
            crate::module::CType::CStr => Self::CStr,
            crate::module::CType::Bool => Self::Bool,
            crate::module::CType::Size => Self::Size,
            crate::module::CType::Ssize => Self::Ssize,
            crate::module::CType::StructPtr => {
                panic!("StructPtr requires layout index - use CTypeRuntime::from_ctype_with_layout")
            }
            crate::module::CType::ArrayPtr => Self::ArrayPtr,
            crate::module::CType::FnPtr => Self::FnPtr,
            crate::module::CType::StructValue => {
                panic!("StructValue requires layout index - use CTypeRuntime::from_ctype_with_layout")
            }
        }
    }
}

