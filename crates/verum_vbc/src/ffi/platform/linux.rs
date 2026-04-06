//! Linux FFI platform implementation.
//!
//! Uses the following system APIs:
//! - `dlopen`/`dlsym`/`dlclose` for library loading
//! - `__errno_location()` for errno access
//! - `mmap`/`munmap` for executable memory

use super::{FfiPlatform, FfiPlatformError, LibraryHandle};
use std::ffi::{CStr, CString};
use std::ptr;

/// Linux-specific FFI platform implementation.
pub struct LinuxPlatform {
    /// Cached handle to libc for common operations.
    _libc: Option<LibraryHandle>,
}

impl LinuxPlatform {
    /// Creates a new Linux platform instance.
    pub fn new() -> Self {
        Self { _libc: None }
    }

    /// Get the last dlerror message.
    fn get_dlerror(&self) -> String {
        unsafe {
            let err = libc::dlerror();
            if err.is_null() {
                "unknown error".to_string()
            } else {
                CStr::from_ptr(err).to_string_lossy().into_owned()
            }
        }
    }
}

impl Default for LinuxPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiPlatform for LinuxPlatform {
    fn load_library(&self, name: &str) -> Result<LibraryHandle, FfiPlatformError> {
        // Clear any previous error
        unsafe {
            libc::dlerror();
        }

        // If it's already a full path, try directly
        if name.starts_with('/') {
            let cname = CString::new(name).map_err(|_| FfiPlatformError::InvalidLibraryName {
                name: name.to_string(),
                reason: "contains null byte".to_string(),
            })?;

            let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY) };
            if !handle.is_null() {
                return Ok(unsafe { LibraryHandle::from_raw(handle) });
            }
        }

        // Try normalized library name
        let normalized = self.normalize_library_name(name);
        let cname = CString::new(normalized.as_str()).map_err(|_| {
            FfiPlatformError::InvalidLibraryName {
                name: name.to_string(),
                reason: "contains null byte".to_string(),
            }
        })?;

        let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY) };
        if !handle.is_null() {
            return Ok(unsafe { LibraryHandle::from_raw(handle) });
        }

        // Try with just the base name (for things in LD_LIBRARY_PATH)
        let cname = CString::new(name).unwrap();
        let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY) };
        if !handle.is_null() {
            return Ok(unsafe { LibraryHandle::from_raw(handle) });
        }

        // Try with .so.6 suffix (common for libc)
        if !name.contains(".so") {
            let versioned_name = if name.starts_with("lib") {
                format!("{}.so.6", name)
            } else {
                format!("lib{}.so.6", name)
            };
            let cname = CString::new(versioned_name.as_str()).unwrap();
            let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY) };
            if !handle.is_null() {
                return Ok(unsafe { LibraryHandle::from_raw(handle) });
            }
        }

        Err(FfiPlatformError::LibraryNotFound {
            name: name.to_string(),
            reason: self.get_dlerror(),
        })
    }

    unsafe fn unload_library(&self, handle: LibraryHandle) -> Result<(), FfiPlatformError> {
        if handle.is_null() {
            return Ok(());
        }

        let result = unsafe { libc::dlclose(handle.as_raw()) };
        if result != 0 {
            return Err(FfiPlatformError::PlatformError {
                code: result,
                message: self.get_dlerror(),
            });
        }

        Ok(())
    }

    fn resolve_symbol(
        &self,
        handle: LibraryHandle,
        name: &str,
    ) -> Result<*const (), FfiPlatformError> {
        // Clear any previous error
        unsafe {
            libc::dlerror();
        }

        let cname = CString::new(name).map_err(|_| FfiPlatformError::SymbolNotFound {
            symbol: name.to_string(),
            library: "<unknown>".to_string(),
        })?;

        let symbol = unsafe { libc::dlsym(handle.as_raw(), cname.as_ptr()) };

        // Check for error - dlsym can return NULL for valid symbols
        let error = unsafe { libc::dlerror() };
        if !error.is_null() {
            let msg = unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() };
            return Err(FfiPlatformError::SymbolNotFound {
                symbol: name.to_string(),
                library: msg,
            });
        }

        Ok(symbol as *const ())
    }

    fn errno_location(&self) -> *mut i32 {
        // On Linux, __errno_location() returns a pointer to the thread-local errno
        unsafe extern "C" {
            fn __errno_location() -> *mut i32;
        }
        unsafe { __errno_location() }
    }

    fn normalize_library_name(&self, name: &str) -> String {
        // Already normalized?
        if name.ends_with(".so") || name.contains(".so.") {
            return name.to_string();
        }

        // Full path?
        if name.starts_with('/') {
            return name.to_string();
        }

        // Special cases for well-known libraries
        match name {
            "c" | "libc" => "libc.so.6".to_string(),
            "m" | "libm" => "libm.so.6".to_string(),
            "pthread" | "libpthread" => "libpthread.so.0".to_string(),
            "dl" | "libdl" => "libdl.so.2".to_string(),
            "rt" | "librt" => "librt.so.1".to_string(),
            _ => {
                // Add lib prefix and .so suffix
                if name.starts_with("lib") {
                    format!("{}.so", name)
                } else {
                    format!("lib{}.so", name)
                }
            }
        }
    }

    unsafe fn alloc_executable(&self, size: usize) -> Result<*mut u8, FfiPlatformError> {
        let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS;

        let ptr = unsafe { libc::mmap(ptr::null_mut(), size, prot, flags, -1, 0) };

        if ptr == libc::MAP_FAILED {
            return Err(FfiPlatformError::AllocationFailed {
                size,
                reason: format!("mmap failed with errno {}", unsafe { *self.errno_location() }),
            });
        }

        Ok(ptr as *mut u8)
    }

    unsafe fn free_executable(&self, ptr: *mut u8, size: usize) -> Result<(), FfiPlatformError> {
        let result = unsafe { libc::munmap(ptr as *mut libc::c_void, size) };
        if result != 0 {
            return Err(FfiPlatformError::PlatformError {
                code: unsafe { *self.errno_location() },
                message: "munmap failed".to_string(),
            });
        }
        Ok(())
    }

    fn platform_id(&self) -> &'static str {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_library_name() {
        let platform = LinuxPlatform::new();

        assert_eq!(platform.normalize_library_name("c"), "libc.so.6");
        assert_eq!(platform.normalize_library_name("foo"), "libfoo.so");
        assert_eq!(platform.normalize_library_name("libbar"), "libbar.so");
        assert_eq!(platform.normalize_library_name("test.so"), "test.so");
    }

    #[test]
    fn test_platform_id() {
        let platform = LinuxPlatform::new();
        assert_eq!(platform.platform_id(), "linux");
    }
}
