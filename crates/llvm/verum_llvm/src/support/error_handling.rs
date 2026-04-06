//! This module contains some supplemental functions for dealing with errors.

use libc::c_void;
use verum_llvm_sys::core::{LLVMGetDiagInfoDescription, LLVMGetDiagInfoSeverity};
use verum_llvm_sys::error_handling::{LLVMInstallFatalErrorHandler, LLVMResetFatalErrorHandler};
use verum_llvm_sys::prelude::LLVMDiagnosticInfoRef;
use verum_llvm_sys::LLVMDiagnosticSeverity;

/// Installs a fatal error handler that LLVM calls before `exit()`.
///
/// # Example
///
/// ```no_run
/// use verum_llvm::support::error_handling::install_fatal_error_handler;
/// extern "C" fn print_before_exit(msg: *const i8) {
///     let c_str = unsafe { std::ffi::CStr::from_ptr(msg) };
///     eprintln!("LLVM fatally errored: {:?}", c_str);
/// }
/// unsafe {
///     install_fatal_error_handler(print_before_exit);
/// }
/// ```
pub unsafe fn install_fatal_error_handler(handler: extern "C" fn(*const ::libc::c_char)) {
    LLVMInstallFatalErrorHandler(Some(handler))
}

/// Resets LLVM's fatal error handler back to the default
pub fn reset_fatal_error_handler() {
    unsafe { LLVMResetFatalErrorHandler() }
}

pub(crate) struct DiagnosticInfo {
    diagnostic_info: LLVMDiagnosticInfoRef,
}

impl DiagnosticInfo {
    pub unsafe fn new(diagnostic_info: LLVMDiagnosticInfoRef) -> Self {
        DiagnosticInfo { diagnostic_info }
    }

    pub(crate) fn get_description(&self) -> *mut ::libc::c_char {
        unsafe { LLVMGetDiagInfoDescription(self.diagnostic_info) }
    }

    pub(crate) fn severity_is_error(&self) -> bool {
        self.severity() == LLVMDiagnosticSeverity::LLVMDSError
    }

    fn severity(&self) -> LLVMDiagnosticSeverity {
        unsafe { LLVMGetDiagInfoSeverity(self.diagnostic_info) }
    }
}

// Assmuptions this handler makes:
// * A valid *mut *mut i8 is provided as the void_ptr (via context.set_diagnostic_handler)
//
// https://github.com/llvm-mirror/llvm/blob/master/tools/llvm-c-test/diagnostic.c was super useful
// for figuring out how to get this to work
pub(crate) extern "C" fn get_error_str_diagnostic_handler(
    diagnostic_info: LLVMDiagnosticInfoRef,
    void_ptr: *mut c_void,
) {
    let diagnostic_info = unsafe { DiagnosticInfo::new(diagnostic_info) };

    if diagnostic_info.severity_is_error() {
        let c_ptr_ptr = void_ptr as *mut *mut c_void as *mut *mut ::libc::c_char;

        unsafe {
            *c_ptr_ptr = diagnostic_info.get_description();
        }
    }
}
