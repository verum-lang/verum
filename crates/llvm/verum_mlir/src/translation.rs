//! MLIR module translation to LLVM IR.
//!
//! Provides safe wrappers around MLIR's C API for translating MLIR modules
//! (in the LLVM dialect) to LLVM IR modules.

use crate::ir::Module;
use verum_mlir_sys::mlirTranslateModuleToLLVMIR;

/// Opaque LLVM context handle (from llvm-c).
///
/// This wraps `LLVMContextRef` without depending on the inkwell crate,
/// allowing callers to pass their own LLVM context.
#[repr(transparent)]
pub struct LlvmContextRef {
    raw: verum_mlir_sys::LLVMContextRef,
}

impl Default for LlvmContextRef {
    fn default() -> Self {
        Self::new()
    }
}

impl LlvmContextRef {
    /// Create a new LLVM context.
    pub fn new() -> Self {
        Self {
            raw: unsafe { verum_mlir_sys::LLVMContextCreate() },
        }
    }

    /// Create from a raw `LLVMContextRef`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the context is valid and will outlive this wrapper.
    pub unsafe fn from_raw(raw: verum_mlir_sys::LLVMContextRef) -> Self {
        Self { raw }
    }

    /// Get the raw `LLVMContextRef`.
    pub fn as_raw(&self) -> verum_mlir_sys::LLVMContextRef {
        self.raw
    }
}

impl Drop for LlvmContextRef {
    fn drop(&mut self) {
        unsafe { verum_mlir_sys::LLVMContextDispose(self.raw) }
    }
}

/// Opaque LLVM module handle (from llvm-c).
///
/// Represents a compiled LLVM IR module that can be serialized to bitcode,
/// printed as textual IR, or passed to an LLVM target machine for object
/// code emission.
pub struct LlvmModule {
    raw: verum_mlir_sys::LLVMModuleRef,
}

impl LlvmModule {
    /// Translate an MLIR module (in LLVM dialect) to an LLVM IR module.
    ///
    /// The MLIR module must have been fully lowered to the LLVM dialect
    /// (via `convert-to-llvm` or equivalent passes). The resulting LLVM
    /// module is owned by the caller.
    ///
    /// Returns `None` if translation fails (e.g., module contains
    /// non-LLVM-dialect operations).
    pub fn from_mlir(module: &Module, llvm_ctx: &LlvmContextRef) -> Option<Self> {
        let raw = unsafe {
            mlirTranslateModuleToLLVMIR(module.as_operation().to_raw(), llvm_ctx.as_raw())
        };
        if raw.is_null() {
            None
        } else {
            Some(Self { raw })
        }
    }

    /// Get the raw `LLVMModuleRef`.
    pub fn as_raw(&self) -> verum_mlir_sys::LLVMModuleRef {
        self.raw
    }

    /// Print the LLVM IR module to a string.
    pub fn print_to_string(&self) -> String {
        unsafe {
            let c_str = verum_mlir_sys::LLVMPrintModuleToString(self.raw);
            let result = std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned();
            verum_mlir_sys::LLVMDisposeMessage(c_str);
            result
        }
    }

    /// Write LLVM IR as textual representation to a file.
    ///
    /// Returns `Ok(())` on success, `Err(message)` on failure.
    pub fn print_to_file(&self, path: &str) -> Result<(), String> {
        let c_path = std::ffi::CString::new(path).unwrap();
        let mut error_msg: *mut std::ffi::c_char = std::ptr::null_mut();
        let result = unsafe {
            verum_mlir_sys::LLVMPrintModuleToFile(self.raw, c_path.as_ptr(), &mut error_msg)
        };
        if result != 0 && !error_msg.is_null() {
            let msg = unsafe {
                let s = std::ffi::CStr::from_ptr(error_msg).to_string_lossy().into_owned();
                verum_mlir_sys::LLVMDisposeMessage(error_msg);
                s
            };
            Err(msg)
        } else {
            Ok(())
        }
    }

    /// Get the data layout string.
    pub fn data_layout(&self) -> String {
        unsafe {
            let c_str = verum_mlir_sys::LLVMGetDataLayoutStr(self.raw);
            std::ffi::CStr::from_ptr(c_str)
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Set the target triple.
    pub fn set_target_triple(&self, triple: &str) {
        let c_triple = std::ffi::CString::new(triple).unwrap();
        unsafe { verum_mlir_sys::LLVMSetTarget(self.raw, c_triple.as_ptr()) }
    }
}

impl Drop for LlvmModule {
    fn drop(&mut self) {
        unsafe { verum_mlir_sys::LLVMDisposeModule(self.raw) }
    }
}

impl std::fmt::Display for LlvmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.print_to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        context::Context,
        ir::Location,
        utility::{register_all_dialects, register_all_llvm_translations},
        dialect::DialectRegistry,
    };

    #[test]
    fn test_llvm_context_create_destroy() {
        let _ctx = LlvmContextRef::new();
    }

    #[test]
    fn test_translate_empty_module() {
        let registry = DialectRegistry::new();
        register_all_dialects(&registry);

        let context = Context::new();
        context.append_dialect_registry(&registry);
        context.load_all_available_dialects();
        register_all_llvm_translations(&context);

        let module = Module::new(Location::unknown(&context));
        let llvm_ctx = LlvmContextRef::new();

        // Empty module should translate successfully
        let llvm_mod = LlvmModule::from_mlir(&module, &llvm_ctx);
        assert!(llvm_mod.is_some(), "Empty MLIR module should translate to LLVM IR");

        let llvm_mod = llvm_mod.unwrap();
        let ir = llvm_mod.print_to_string();
        assert!(ir.contains("source_filename"), "LLVM IR should contain source_filename");
    }
}
