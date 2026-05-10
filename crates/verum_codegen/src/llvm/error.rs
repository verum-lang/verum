//! Error types for LLVM-based VBC lowering.
//!

//! This module defines errors that can occur during the VBC → LLVM IR
//! lowering process.

use thiserror::Error;
use verum_common::Text;

/// Result type for LLVM lowering operations.
pub type Result<T> = std::result::Result<T, LlvmLoweringError>;

/// Error type for VBC → LLVM IR lowering.
#[derive(Debug, Error)]
pub enum LlvmLoweringError {
    /// Unsupported VBC instruction encountered.
    #[error("Unsupported VBC instruction: {0}")]
    UnsupportedInstruction(Text),

    /// Type lowering error.
    #[error("Type lowering error: {0}")]
    TypeLowering(Text),

    /// Invalid register reference.
    #[error("Invalid register: r{0}")]
    InvalidRegister(u16),

    /// Missing function definition.
    #[error("Missing function: {0}")]
    MissingFunction(Text),

    /// Missing basic block.
    #[error("Missing basic block: {0}")]
    MissingBlock(Text),

    /// Module verification failed.
    #[error("Module verification failed: {0}")]
    VerificationFailed(Text),

    /// Internal compiler error.
    #[error("Internal error: {0}")]
    Internal(Text),

    /// Invalid type for operation.
    #[error("Invalid type: {0}")]
    InvalidType(Text),

    /// Builder operation failed.
    #[error("Builder error: {0}")]
    BuilderError(Text),
}

/// Severity level for lowering diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringSeverity {
    Warning,
    Info,
}

/// Per-variant projection for [`LoweringSeverity`]. `name` matches
/// the standard diagnostic-severity wire form (`"warning"` /
/// `"info"`); `is_warning` flags the higher-severity variant. The
/// partition is binary by design — `LoweringSeverity` does not
/// carry `Error` (which is a typed `LlvmLoweringError` instead).
#[derive(Debug, Clone, Copy)]
pub struct LoweringSeverityMeta {
    pub name: &'static str,
    pub is_warning: bool,
}

impl LoweringSeverity {
    pub const ALL: &'static [Self] = &[Self::Warning, Self::Info];

    pub const fn meta(self) -> LoweringSeverityMeta {
        match self {
            Self::Warning => LoweringSeverityMeta {
                name: "warning",
                is_warning: true,
            },
            Self::Info => LoweringSeverityMeta {
                name: "info",
                is_warning: false,
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

    #[inline]
    pub const fn is_warning(&self) -> bool {
        self.meta().is_warning
    }
}

/// A structured diagnostic emitted during LLVM lowering.
///

/// Replaces raw `eprintln!` warnings with a collected, structured format
/// that can be filtered, counted, and displayed consistently.
#[derive(Debug, Clone)]
pub struct LoweringDiagnostic {
    /// Severity level
    pub severity: LoweringSeverity,
    /// Human-readable message
    pub message: Text,
    /// Category of the diagnostic (e.g. "ArithExtended", "MathExtended")
    pub category: Text,
    /// The sub-opcode that triggered the diagnostic, if applicable
    pub sub_opcode: Option<u8>,
    /// The function being lowered when the diagnostic was emitted
    pub function_name: Text,
}

impl LoweringDiagnostic {
    /// Create a warning for an unimplemented sub-opcode.
    pub fn unimplemented_sub_op(
        category: impl Into<Text>,
        sub_op: u8,
        function_name: impl Into<Text>,
    ) -> Self {
        let cat: Text = category.into();
        Self {
            severity: LoweringSeverity::Warning,
            message: Text::from(format!("Unimplemented {} sub_op: 0x{:02x}", cat, sub_op)),
            category: cat,
            sub_opcode: Some(sub_op),
            function_name: function_name.into(),
        }
    }

    /// Create a general warning.
    pub fn warning(
        category: impl Into<Text>,
        message: impl Into<Text>,
        function_name: impl Into<Text>,
    ) -> Self {
        Self {
            severity: LoweringSeverity::Warning,
            message: message.into(),
            category: category.into(),
            sub_opcode: None,
            function_name: function_name.into(),
        }
    }

    /// Format this diagnostic for display.
    pub fn display(&self) -> String {
        let prefix = match self.severity {
            LoweringSeverity::Warning => "warning",
            LoweringSeverity::Info => "info",
        };
        format!(
            "[AOT {}] in `{}`: {}",
            prefix, self.function_name, self.message
        )
    }
}

// =============================================================================
// BuildExt — Zero-cost error propagation for LLVM builder operations
// =============================================================================
//

// LLVM builder methods (build_store, build_gep, etc.) return Result<T, BuilderError>.
// Instead of .unwrap() which panics on any LLVM error, use .or_llvm_err()? to
// propagate errors as LlvmLoweringError::BuilderError with the original message.
//

// Usage:
//  builder.build_store(ptr, val).or_llvm_err()?;
//  let gep = builder.build_gep(ty, ptr, &indices, "name").or_llvm_err()?;

/// Extension trait for converting any `Result<T, E: Display>` into
/// `Result<T, LlvmLoweringError>` via `.or_llvm_err()`.
///

/// This replaces `.unwrap()` calls on LLVM builder operations with proper
/// error propagation through the lowering pipeline.
pub trait BuildExt<T> {
    /// Convert a builder Result into a lowering Result.
    ///

    /// Equivalent to `.map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))`.
    fn or_llvm_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> BuildExt<T> for std::result::Result<T, E> {
    #[inline]
    fn or_llvm_err(self) -> Result<T> {
        self.map_err(|e| LlvmLoweringError::BuilderError(e.to_string().into()))
    }
}

/// Extension trait for `verum_llvm::values::ValueKind` providing
/// the canonical `.basic_or_internal(MSG)` helper that collapses
/// the verbose
///
///     ...build_call(...)?.try_as_basic_value().basic().or_internal(MSG)?
///
/// chain into a single call:
///
///     ...build_call(...)?.basic_value_or(MSG)?
///
/// Repeated 38 times in `instruction.rs` after `build_call` for
/// runtime helpers / generator opcodes / strcmp / etc.  Each site
/// is functionally identical: extract the BasicValueEnum from a
/// CallSiteValue's `try_as_basic_value()` result, treating "the
/// callee returned void" as an internal error.
///
/// Implemented on the `Result<CallSiteValue, _>` shape that
/// `build_call.or_llvm_err()?` produces.
pub trait CallSiteExt<'ctx> {
    /// Extract the BasicValueEnum result of a build_call, returning an
    /// `LlvmLoweringError::Internal(MSG)` if the call produced
    /// `Instruction` (i.e., the callee was declared as returning void
    /// or the call site has no usable return).
    fn basic_value_or(self, msg: &str) -> Result<verum_llvm::values::BasicValueEnum<'ctx>>;

    /// Like [`basic_value_or`](Self::basic_value_or) but the message is
    /// produced lazily by `f` — only invoked on the void-return error
    /// path.  Use when the message contains `format!()` interpolation
    /// that would be wasted work on the (overwhelmingly common)
    /// success path.
    fn basic_value_or_else<F: FnOnce() -> String>(
        self,
        f: F,
    ) -> Result<verum_llvm::values::BasicValueEnum<'ctx>>;

    /// Same as [`basic_value_or`](Self::basic_value_or) but panics on
    /// missing return rather than producing an error.  Use in emit
    /// paths where a void-return from a fixed-shape helper would be a
    /// programmer error (signature mismatch).
    fn basic_value_expect(self, msg: &str) -> verum_llvm::values::BasicValueEnum<'ctx>;
}

impl<'ctx> CallSiteExt<'ctx> for verum_llvm::values::CallSiteValue<'ctx> {
    #[inline]
    fn basic_value_or(self, msg: &str) -> Result<verum_llvm::values::BasicValueEnum<'ctx>> {
        self.try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::Internal(msg.into()))
    }

    #[inline]
    fn basic_value_or_else<F: FnOnce() -> String>(
        self,
        f: F,
    ) -> Result<verum_llvm::values::BasicValueEnum<'ctx>> {
        self.try_as_basic_value()
            .basic()
            .ok_or_else(|| LlvmLoweringError::Internal(f().into()))
    }

    /// Variant of [`basic_value_or`](Self::basic_value_or) that
    /// panics on missing return rather than returning an error.
    /// Use in emit paths where a void-return from a fixed-shape
    /// runtime helper is a hard programmer error (the helper's
    /// signature is known at compile time, so this branch should
    /// be unreachable in practice).
    #[inline]
    fn basic_value_expect(self, msg: &str) -> verum_llvm::values::BasicValueEnum<'ctx> {
        self.try_as_basic_value()
            .basic()
            .unwrap_or_else(|| panic!("{}", msg))
    }
}

/// Extension trait for converting `Option<T>` into `Result<T, LlvmLoweringError>`.
///

/// Usage:
///  let block = builder.get_insert_block().or_internal("no current basic block")?;
///  let param = func.get_nth_param(0).or_internal("missing param 0")?;
pub trait OptionExt<T> {
    /// Convert None into an internal error with the given message.
    fn or_internal(self, msg: &str) -> Result<T>;

    /// Convert None into a missing function error.
    fn or_missing_fn(self, name: &str) -> Result<T>;
}

impl<T> OptionExt<T> for Option<T> {
    #[inline]
    fn or_internal(self, msg: &str) -> Result<T> {
        self.ok_or_else(|| LlvmLoweringError::Internal(msg.into()))
    }

    #[inline]
    fn or_missing_fn(self, name: &str) -> Result<T> {
        self.ok_or_else(|| LlvmLoweringError::MissingFunction(name.into()))
    }
}

impl LlvmLoweringError {

    /// Create a type lowering error.
    pub fn type_lowering(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::TypeLowering(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<Text>) -> Self {
        LlvmLoweringError::Internal(msg.into())
    }
}

// =============================================================================
// Module helpers — get-or-declare an LLVM function
// =============================================================================

/// Get the function `name` from `module` if it already exists, otherwise
/// declare it with the given `fn_type` and return the freshly-added
/// `FunctionValue`.
///
/// This collapses the very common pattern
///
///     let func = module
///         .get_function("llvm.floor.f64")
///         .unwrap_or_else(|| module.add_function("llvm.floor.f64", fn_type, None));
///
/// repeated 240+ times across `instruction.rs` / `runtime.rs` /
/// `platform_ir.rs` for declaring LLVM intrinsics, the FFI runtime
/// surface, and the verum runtime symbols.  Centralises the lookup so
/// future audits (e.g. validating the intrinsic name against
/// `verum_llvm`'s intrinsic registry) have a single attachment point.
#[inline]
pub fn get_or_declare_function<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    name: &str,
    fn_type: verum_llvm::types::FunctionType<'ctx>,
) -> verum_llvm::values::FunctionValue<'ctx> {
    module
        .get_function(name)
        .unwrap_or_else(|| module.add_function(name, fn_type, None))
}

/// Get-or-declare a `__verum_libsys_*` shim and tag it with the
/// `verum.libsys` attribute carrying the libc-call name (e.g.
/// `"open"`, `"close"`, `"read"`, `"unlink"`, `"lseek"`,
/// `"access"`).  The libsys layer in Verum's no-libc architecture
/// uses these tags to retarget calls during the dyld-rebinding
/// pass that maps `__verum_libsys_*` to the actual libc symbols
/// at link time.
///
/// Idempotent on the attribute (same first-write-wins semantics as
/// `get_or_declare_noreturn_function`).
///
/// Centralises the verbose pattern repeated for ~7 libsys shim
/// declarations in `runtime.rs`:
///
///     let libsys = module.get_function(NAME).unwrap_or_else(|| {
///         let f = module.add_function(NAME, fn_type, None);
///         f.add_attribute(
///             AttributeLoc::Function,
///             ctx.create_string_attribute("verum.libsys", LIBC_NAME),
///         );
///         f
///     });
#[inline]
pub fn get_or_declare_libsys_function<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    llvm_ctx: &'ctx verum_llvm::context::Context,
    name: &str,
    fn_type: verum_llvm::types::FunctionType<'ctx>,
    libc_name: &str,
) -> verum_llvm::values::FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(name) {
        return existing;
    }
    let f = module.add_function(name, fn_type, None);
    f.add_attribute(
        verum_llvm::attributes::AttributeLoc::Function,
        llvm_ctx.create_string_attribute("verum.libsys", libc_name),
    );
    f
}

/// Get-or-declare an LLVM function and tag it with `noreturn` on
/// the first declaration.  Idempotent on subsequent calls — when the
/// function already exists the attribute is *not* re-applied (LLVM
/// allows multiple identical attributes but the helper preserves
/// the original "first-write-wins" semantics of the manual sites
/// it replaces).
///
/// Centralises the verbose
///
///     let exit_fn = module.get_function(NAME).unwrap_or_else(|| {
///         let f = module.add_function(NAME, fn_type, None);
///         f.add_attribute(
///             AttributeLoc::Function,
///             ctx.create_string_attribute("noreturn", ""),
///         );
///         f
///     });
///
/// pattern that decorates `verum_internal_exit_i64` and similar
/// process-terminating runtime helpers.
#[inline]
pub fn get_or_declare_noreturn_function<'ctx>(
    module: &verum_llvm::module::Module<'ctx>,
    llvm_ctx: &'ctx verum_llvm::context::Context,
    name: &str,
    fn_type: verum_llvm::types::FunctionType<'ctx>,
) -> verum_llvm::values::FunctionValue<'ctx> {
    if let Some(existing) = module.get_function(name) {
        return existing;
    }
    let f = module.add_function(name, fn_type, None);
    f.add_attribute(
        verum_llvm::attributes::AttributeLoc::Function,
        llvm_ctx.create_string_attribute("noreturn", ""),
    );
    f
}

#[cfg(test)]
mod meta_consolidation_pins {
    use super::LoweringSeverity;

    #[test]
    fn lowering_severity_round_trip_unique_and_partition() {
        assert_eq!(LoweringSeverity::ALL.len(), 2);
        for v in LoweringSeverity::ALL {
            let s = v.as_str();
            assert_eq!(LoweringSeverity::from_str(s), Some(*v));
        }
        // Wire form: lowercase (matches the standard
        // diagnostic-severity convention used elsewhere in the
        // codegen layer).
        assert_eq!(LoweringSeverity::Warning.as_str(), "warning");
        assert_eq!(LoweringSeverity::Info.as_str(), "info");
        // is_warning is the binary partition: Warning true, Info false.
        assert!(LoweringSeverity::Warning.is_warning());
        assert!(!LoweringSeverity::Info.is_warning());
        // Pin: enum does NOT carry an Error variant — diagnostics
        // at error-severity flow through the typed
        // `LlvmLoweringError` instead.
        assert!(LoweringSeverity::from_str("error").is_none());
    }
}
