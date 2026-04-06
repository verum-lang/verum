//! Cross-platform FFI abstraction layer.
//!
//! This module provides a unified interface for platform-specific FFI operations:
//! - Dynamic library loading (dlopen, LoadLibrary)
//! - Symbol resolution (dlsym, GetProcAddress)
//! - Errno/GetLastError access
//! - Executable memory allocation for trampolines
//!
//! # Platform Support
//!
//! | Platform | Library Loading | Symbol Resolution | Errno |
//! |----------|-----------------|-------------------|-------|
//! | macOS    | dlopen          | dlsym             | __error() |
//! | Linux    | dlopen          | dlsym             | __errno_location() |
//! | Windows  | LoadLibraryW    | GetProcAddress    | GetLastError() |

#[cfg(target_os = "macos")]
pub mod darwin;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;

use std::fmt;

/// Opaque handle to a loaded dynamic library.
///
/// This wraps the platform-specific handle type:
/// - Unix: `*mut c_void` from dlopen
/// - Windows: `HMODULE` from LoadLibrary
#[derive(Clone, Copy)]
pub struct LibraryHandle {
    /// Raw pointer to the library handle.
    ptr: *mut std::ffi::c_void,
}

impl LibraryHandle {
    /// Creates a new library handle from a raw pointer.
    ///
    /// # Safety
    ///
    /// The pointer must be a valid library handle from the platform's
    /// library loading function (dlopen, LoadLibrary, etc.).
    pub unsafe fn from_raw(ptr: *mut std::ffi::c_void) -> Self {
        Self { ptr }
    }

    /// Returns the raw pointer to the library handle.
    pub fn as_raw(&self) -> *mut std::ffi::c_void {
        self.ptr
    }

    /// Returns true if this is a null handle.
    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }
}

impl fmt::Debug for LibraryHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryHandle")
            .field("ptr", &self.ptr)
            .finish()
    }
}

// SAFETY: LibraryHandle is just a wrapper around a raw pointer.
// The actual thread-safety depends on how the platform handles library loading,
// but typically library handles can be shared across threads.
unsafe impl Send for LibraryHandle {}
unsafe impl Sync for LibraryHandle {}

/// Error type for FFI platform operations.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum FfiPlatformError {
    /// Library not found.
    LibraryNotFound {
        name: String,
        reason: String,
    },
    /// Symbol not found in library.
    SymbolNotFound {
        symbol: String,
        library: String,
    },
    /// Failed to allocate executable memory.
    AllocationFailed {
        size: usize,
        reason: String,
    },
    /// Platform-specific error.
    PlatformError {
        code: i32,
        message: String,
    },
    /// Invalid library name.
    InvalidLibraryName {
        name: String,
        reason: String,
    },
}

impl fmt::Display for FfiPlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfiPlatformError::LibraryNotFound { name, reason } => {
                write!(f, "library '{}' not found: {}", name, reason)
            }
            FfiPlatformError::SymbolNotFound { symbol, library } => {
                write!(f, "symbol '{}' not found in library '{}'", symbol, library)
            }
            FfiPlatformError::AllocationFailed { size, reason } => {
                write!(f, "failed to allocate {} bytes of executable memory: {}", size, reason)
            }
            FfiPlatformError::PlatformError { code, message } => {
                write!(f, "platform error ({}): {}", code, message)
            }
            FfiPlatformError::InvalidLibraryName { name, reason } => {
                write!(f, "invalid library name '{}': {}", name, reason)
            }
        }
    }
}

impl std::error::Error for FfiPlatformError {}

/// Cross-platform FFI abstraction trait.
///
/// This trait provides a unified interface for FFI operations across
/// different operating systems. Implementations handle platform-specific
/// details like library loading, symbol resolution, and errno access.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support multi-threaded
/// access from the interpreter.
pub trait FfiPlatform: Send + Sync {
    /// Load a dynamic library by name.
    ///
    /// The name can be:
    /// - A simple library name (e.g., "c", "System")
    /// - A full path to the library
    ///
    /// The implementation will normalize the name for the current platform
    /// (adding lib prefix, .so/.dylib/.dll suffix, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error if the library cannot be found or loaded.
    fn load_library(&self, name: &str) -> Result<LibraryHandle, FfiPlatformError>;

    /// Unload a previously loaded library.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The handle is valid (was returned by `load_library`)
    /// - No code from the library is currently executing
    /// - No pointers to data in the library are being used
    unsafe fn unload_library(&self, handle: LibraryHandle) -> Result<(), FfiPlatformError>;

    /// Resolve a symbol in a loaded library.
    ///
    /// Returns a pointer to the symbol, which can be cast to the appropriate
    /// function pointer type.
    ///
    /// # Errors
    ///
    /// Returns an error if the symbol cannot be found.
    fn resolve_symbol(
        &self,
        handle: LibraryHandle,
        name: &str,
    ) -> Result<*const (), FfiPlatformError>;

    /// Get a pointer to the thread-local errno location.
    ///
    /// This returns a pointer that can be dereferenced to read or write
    /// the current thread's errno value.
    fn errno_location(&self) -> *mut i32;

    /// Normalize a library name for the current platform.
    ///
    /// Transforms a logical library name into a platform-specific filename:
    /// - macOS: "foo" → "libfoo.dylib" or framework path
    /// - Linux: "foo" → "libfoo.so"
    /// - Windows: "foo" → "foo.dll"
    ///
    /// If the name already has a platform-specific suffix, it's returned as-is.
    fn normalize_library_name(&self, name: &str) -> String;

    /// Allocate executable memory for trampolines.
    ///
    /// This allocates memory with read, write, and execute permissions
    /// for generating callback trampolines.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - Code written to this memory is valid machine code
    /// - The memory is properly freed using `free_executable`
    ///
    /// # Errors
    ///
    /// Returns an error if memory allocation fails.
    unsafe fn alloc_executable(&self, size: usize) -> Result<*mut u8, FfiPlatformError>;

    /// Free previously allocated executable memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - The pointer was returned by `alloc_executable`
    /// - No code in the memory is currently executing
    /// - The size matches the original allocation
    unsafe fn free_executable(&self, ptr: *mut u8, size: usize) -> Result<(), FfiPlatformError>;

    /// Get the platform identifier.
    fn platform_id(&self) -> &'static str;
}

/// Factory function to create a platform-specific FFI implementation.
///
/// Returns the appropriate implementation for the current operating system.
#[cfg(feature = "ffi")]
pub fn create_platform() -> Box<dyn FfiPlatform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(darwin::DarwinPlatform::new())
    }

    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }

    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPlatform::new())
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        compile_error!("Unsupported platform for FFI")
    }
}

/// Re-export the platform-specific type for the current OS.
#[cfg(all(feature = "ffi", target_os = "macos"))]
pub use darwin::DarwinPlatform as CurrentPlatform;

#[cfg(all(feature = "ffi", target_os = "linux"))]
pub use linux::LinuxPlatform as CurrentPlatform;

#[cfg(all(feature = "ffi", target_os = "windows"))]
pub use windows::WindowsPlatform as CurrentPlatform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_handle_null_check() {
        let handle = unsafe { LibraryHandle::from_raw(std::ptr::null_mut()) };
        assert!(handle.is_null());
    }

    #[test]
    fn test_library_handle_non_null() {
        let dummy: usize = 42;
        let handle = unsafe { LibraryHandle::from_raw(dummy as *mut std::ffi::c_void) };
        assert!(!handle.is_null());
    }

    #[test]
    fn test_error_display() {
        let err = FfiPlatformError::LibraryNotFound {
            name: "foo".to_string(),
            reason: "not in library path".to_string(),
        };
        assert!(err.to_string().contains("foo"));
        assert!(err.to_string().contains("not in library path"));
    }
}
