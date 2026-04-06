//! verum_llvm_sys - Low-level FFI bindings for LLVM and LLD
//!
//! This crate provides raw FFI bindings to LLVM's C API and LLD linker.
//! It uses a locally built or downloaded LLVM installation from `llvm/install/`.
//!
//! ## Features
//!
//! - `llvm` (default): Core LLVM bindings
//! - `lld` (default): LLD linker bindings
//! - `static` (default): Static linking
//! - `orc2`: OrcV2 JIT compilation API
//!
//! ## Usage
//!
//! ```rust,no_run
//! use verum_llvm_sys::core::{LLVMContextCreate, LLVMContextDispose};
//! use verum_llvm_sys::verum_llvm_initialize_all_targets;
//!
//! unsafe {
//!     // Initialize LLVM
//!     verum_llvm_initialize_all_targets();
//!
//!     // Create context
//!     let ctx = LLVMContextCreate();
//!     // ...
//!     LLVMContextDispose(ctx);
//! }
//! ```
//!
//! ## Building
//!
//! This crate requires LLVM to be installed. The build script searches for LLVM in:
//!
//! 1. `VERUM_LLVM_DIR` environment variable
//! 2. `LLVM_SYS_211_PREFIX` environment variable
//! 3. `llvm/install/` directory in workspace root
//! 4. Downloads prebuilt from GitHub Releases
//!
//! To build LLVM locally:
//! ```bash
//! cd llvm && ./build.sh
//! ```

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(improper_ctypes)]
#![allow(clippy::all)]

pub mod llvm;
pub mod lld;

// Re-export commonly used types at crate root
pub use llvm::*;

// Target initialization wrappers (compiled from wrappers/target.c)
unsafe extern "C" {
    pub fn verum_llvm_initialize_all_targets();
    pub fn verum_llvm_initialize_all_target_infos();
    pub fn verum_llvm_initialize_all_target_mcs();
    pub fn verum_llvm_initialize_all_asm_printers();
    pub fn verum_llvm_initialize_all_asm_parsers();
    pub fn verum_llvm_initialize_native_target();
    pub fn verum_llvm_initialize_native_asm_printer();
    pub fn verum_llvm_initialize_native_asm_parser();
}

/// Initialize all LLVM targets (convenience function)
///
/// # Safety
///
/// This function is safe to call multiple times, but should typically
/// be called once at program startup.
pub fn initialize_targets() {
    unsafe {
        verum_llvm_initialize_all_targets();
        verum_llvm_initialize_all_target_infos();
        verum_llvm_initialize_all_target_mcs();
        verum_llvm_initialize_all_asm_printers();
        verum_llvm_initialize_all_asm_parsers();
    }
}

/// Initialize only the native target (convenience function)
///
/// # Safety
///
/// This function is safe to call multiple times.
pub fn initialize_native_target() {
    unsafe {
        verum_llvm_initialize_native_target();
        verum_llvm_initialize_native_asm_printer();
        verum_llvm_initialize_native_asm_parser();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_targets() {
        initialize_targets();
    }

    #[test]
    fn test_initialize_native_target() {
        initialize_native_target();
    }

    #[test]
    fn test_context_create() {
        initialize_native_target();
        unsafe {
            let ctx = llvm::core::LLVMContextCreate();
            assert!(!ctx.is_null());
            llvm::core::LLVMContextDispose(ctx);
        }
    }
}
