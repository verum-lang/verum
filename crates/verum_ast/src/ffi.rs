//! FFI Boundary Declarations for the Verum language.
//!
//! This module defines the AST nodes for FFI boundary declarations, which are
//! **compile-time specifications**, not runtime types. FFI boundaries formalize
//! expectations at the boundary between provable Verum code and unprovable external code.
//!
//! # FFI Design Principles
//!
//! - FFI boundaries are NOT types (values cannot have FFI boundary "types")
//! - FFI boundaries ARE formal specifications of expectations at the boundary
//!   between provable Verum code and unprovable external code
//! - Only C ABI is supported for FFI (the only stable, universal ABI)
//! - Seven mandatory components in every boundary contract: function signature,
//!   preconditions (requires), postconditions (ensures), memory effects,
//!   thread safety, error protocol, and ownership semantics
//!
//! # Syntax
//!
//! ```verum
//! ffi LibMath {
//!     @extern("C")
//!     fn sqrt(x: f64) -> f64;
//!
//!     requires x >= 0.0;
//!     ensures result >= 0.0;
//!     memory_effects = Reads(x);
//!     thread_safe = true;
//!     errors_via = None;
//!     @ownership(borrow);
//! }
//! ```

use crate::expr::Expr;
use crate::span::{Span, Spanned};
use crate::ty::{Ident, Type};
use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

/// An FFI boundary declaration.
///
/// This is a compile-time specification that formalizes expectations at the FFI boundary.
/// It is NOT a runtime type - values cannot have FFI boundary "types".
///
/// # Seven Mandatory Components
///
/// Every FFI boundary contract contains:
/// 1. **Signature** - What we're binding to (@extern)
/// 2. **Preconditions** - What Verum must ensure (requires)
/// 3. **Postconditions** - What we expect, cannot guarantee (ensures)
/// 4. **Memory Effects** - For optimizer (memory_effects)
/// 5. **Thread Safety** - For scheduler (thread_safe)
/// 6. **Error Protocol** - For wrapper generation (errors_via)
/// 7. **Ownership** - For memory management (@ownership)
///
/// # Platform-Specific Boundaries
///
/// FFI boundaries can be conditional using cfg attributes:
///
/// ```verum
/// #[cfg(target_os = "windows")]
/// ffi Kernel32 {
///     @extern("C", calling_convention = "stdcall")
///     fn CreateFileW(...) -> *void;
/// }
///
/// #[cfg(target_os = "linux")]
/// ffi Libc {
///     @extern("C")
///     fn open(pathname: *const char, flags: i32, mode: u32) -> i32;
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FFIBoundary {
    /// Boundary name (e.g., "LibMath", "Posix")
    pub name: Ident,

    /// Optional parent FFI boundary that this extends
    /// Example: `ffi MyLib extends OtherLib { ... }`
    pub extends: Maybe<Ident>,

    /// FFI functions in this boundary
    pub functions: List<FFIFunction>,

    /// Visibility modifier
    pub visibility: crate::decl::Visibility,

    /// Attributes including cfg conditions
    /// Example: #[cfg(target_os = "windows")]
    pub attributes: List<crate::attr::Attribute>,

    /// Source location
    pub span: Span,
}

impl Spanned for FFIBoundary {
    fn span(&self) -> Span {
        self.span
    }
}

/// An FFI function declaration within a boundary.
///
/// Each FFI function has a signature and a contract specifying:
/// - Preconditions (must be satisfied before calling)
/// - Postconditions (what we expect after calling)
/// - Memory effects (reads, writes, allocates, deallocates)
/// - Thread safety guarantees
/// - Error protocol (how errors are signaled)
/// - Ownership transfer semantics
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FFIFunction {
    /// Function name
    pub name: Ident,

    /// Function signature
    pub signature: FFISignature,

    /// Preconditions - verified at Verum side
    pub requires: List<Expr>,

    /// Postconditions - assumptions we make (unverified)
    pub ensures: List<Expr>,

    /// Memory effects for optimization
    pub memory_effects: MemoryEffects,

    /// Thread safety flag
    pub thread_safe: bool,

    /// Error protocol
    pub error_protocol: ErrorProtocol,

    /// Ownership semantics
    pub ownership: Ownership,

    /// Source location
    pub span: Span,
}

impl Spanned for FFIFunction {
    fn span(&self) -> Span {
        self.span
    }
}

/// FFI function signature with C ABI information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FFISignature {
    /// Function parameters (name and type pairs)
    pub params: List<(Ident, Type)>,

    /// Return type
    pub return_type: Type,

    /// Calling convention (@extern attribute)
    pub calling_convention: CallingConvention,

    /// Whether this is a variadic function (e.g., printf)
    /// Only valid for C calling convention
    pub is_variadic: bool,

    /// Source location
    pub span: Span,
}

/// Supported calling conventions for FFI.
///
/// Only C ABI is fully stable, but we support different calling conventions
/// within the C ABI framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallingConvention {
    /// Standard C calling convention (cdecl)
    C,

    /// Windows stdcall convention
    StdCall,

    /// Fast call convention
    FastCall,

    /// System V AMD64 ABI
    SysV64,

    /// Interrupt handler calling convention
    /// - All registers saved/restored automatically
    /// - Uses iret for return (x86/x86_64)
    /// - First parameter is InterruptStackFrame reference
    ///
    /// Hardware interrupt handler calling convention. The function receives an
    /// InterruptStackFrame as its first parameter and must follow strict codegen
    /// rules (no heap allocation, no panics, save/restore all registers).
    Interrupt,

    /// Naked function - no prologue/epilogue
    /// Must contain only inline assembly
    Naked,

    /// System calling convention (platform-dependent: stdcall on Windows, C elsewhere)
    System,
}

impl CallingConvention {
    pub fn as_str(&self) -> &'static str {
        match self {
            CallingConvention::C => "C",
            CallingConvention::StdCall => "stdcall",
            CallingConvention::FastCall => "fastcall",
            CallingConvention::SysV64 => "sysv64",
            CallingConvention::Interrupt => "interrupt",
            CallingConvention::Naked => "naked",
            CallingConvention::System => "system",
        }
    }
}

/// Memory effects for FFI functions.
///
/// Memory effects tell the optimizer what a function can do to memory,
/// enabling safe optimizations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryEffects {
    /// Pure computation - no side effects
    Pure,

    /// Reads from memory (only side effect is reading)
    /// Optional: specific memory ranges
    Reads(Maybe<List<Text>>),

    /// Writes to memory (side effect is writing)
    /// Optional: specific memory ranges
    Writes(Maybe<List<Text>>),

    /// May allocate memory
    Allocates,

    /// May deallocate memory (possibly specific pointer)
    Deallocates(Maybe<Text>),

    /// Combination of effects
    Combined(List<MemoryEffects>),
}

/// Error handling protocol for FFI functions.
///
/// This specifies how a foreign function signals errors, allowing
/// the Verum compiler to generate correct error handling code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ErrorProtocol {
    /// Function cannot fail
    None,

    /// Function uses POSIX errno for error reporting
    Errno,

    /// Function returns specific code for success
    /// e.g., ReturnCode(SQLITE_OK) means success when result == SQLITE_OK
    ReturnCode(Expr),

    /// Function returns sentinel value on error (e.g., NULL)
    ReturnValue(Expr),

    /// Function returns sentinel value and may also set errno
    ReturnValueWithErrno(Box<Expr>),

    /// Function may throw C++ exceptions
    Exception,
}

/// Ownership semantics for FFI memory.
///
/// Specifies how ownership of memory is transferred across the FFI boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ownership {
    /// Borrowed reference - we don't own the memory
    Borrow,

    /// Transfer ownership to C - we give C the pointer
    TransferTo(Text),

    /// Transfer ownership from C - C gives us the pointer
    TransferFrom(Text),

    /// Shared access - both sides can access simultaneously
    Shared,
}

impl Ownership {
    pub fn as_str(&self) -> &'static str {
        match self {
            Ownership::Borrow => "borrow",
            Ownership::TransferTo(_) => "transfer_to",
            Ownership::TransferFrom(_) => "transfer_from",
            Ownership::Shared => "shared",
        }
    }
}

// Tests moved to tests/ffi_tests.rs per CLAUDE.md standards
// (NO #[cfg(test)] modules in src/ - all tests in tests/ directory)
