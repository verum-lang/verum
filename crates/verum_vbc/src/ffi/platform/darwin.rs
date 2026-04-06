//! macOS (Darwin) FFI platform implementation.
//!
//! Uses the following system APIs:
//! - `dlopen`/`dlsym`/`dlclose` for library loading
//! - `__error()` for errno access
//! - `mmap`/`munmap` with `MAP_JIT` for executable memory

use super::{FfiPlatform, FfiPlatformError, LibraryHandle};
use std::ffi::{CStr, CString};
use std::ptr;

/// Darwin-specific FFI platform implementation.
pub struct DarwinPlatform {
    /// Cached handle to libSystem.B.dylib for common operations.
    _libsystem: Option<LibraryHandle>,
}

impl DarwinPlatform {
    /// Creates a new Darwin platform instance.
    pub fn new() -> Self {
        Self { _libsystem: None }
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

    /// Check if a library name looks like a framework reference.
    fn is_framework_path(&self, name: &str) -> bool {
        name.contains(".framework") || name.starts_with("/System/Library/Frameworks/")
    }

    /// Try to load a library from a framework path.
    fn try_load_framework(&self, name: &str) -> Option<LibraryHandle> {
        // Try common framework locations
        let framework_paths = [
            format!("/System/Library/Frameworks/{}.framework/{}", name, name),
            format!(
                "/Library/Frameworks/{}.framework/{}",
                name, name
            ),
            format!(
                "{}/Library/Frameworks/{}.framework/{}",
                std::env::var("HOME").unwrap_or_default(),
                name,
                name
            ),
        ];

        for path in &framework_paths {
            if let Ok(cpath) = CString::new(path.as_str()) {
                let handle = unsafe { libc::dlopen(cpath.as_ptr(), libc::RTLD_LAZY) };
                if !handle.is_null() {
                    return Some(unsafe { LibraryHandle::from_raw(handle) });
                }
            }
        }

        None
    }
}

impl Default for DarwinPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiPlatform for DarwinPlatform {
    fn load_library(&self, name: &str) -> Result<LibraryHandle, FfiPlatformError> {
        // Clear any previous error
        unsafe {
            libc::dlerror();
        }

        // If it's already a full path, relative path, or framework path, try directly
        if name.starts_with('/') || name.contains('/') || self.is_framework_path(name) {
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

        // Try as a framework
        if let Some(handle) = self.try_load_framework(name) {
            return Ok(handle);
        }

        // Try without lib prefix (for things like "System" -> "libSystem.B.dylib")
        // Skip this fallback for relative paths (containing '/') or paths ending in .dylib
        if !name.starts_with("lib") && !name.contains('/') && !name.ends_with(".dylib") {
            let lib_name = format!("lib{}.dylib", name);
            let cname = CString::new(lib_name.as_str()).unwrap();
            let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_LAZY) };
            if !handle.is_null() {
                return Ok(unsafe { LibraryHandle::from_raw(handle) });
            }

            // Try libSystem.B.dylib pattern
            let lib_b_name = format!("lib{}.B.dylib", name);
            let cname = CString::new(lib_b_name.as_str()).unwrap();
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
        // On macOS, __error() returns a pointer to the thread-local errno
        unsafe extern "C" {
            fn __error() -> *mut i32;
        }
        unsafe { __error() }
    }

    fn normalize_library_name(&self, name: &str) -> String {
        // Already normalized?
        if name.ends_with(".dylib") || name.ends_with(".so") || name.contains(".framework") {
            return name.to_string();
        }

        // Full path?
        if name.starts_with('/') {
            return name.to_string();
        }

        // Special cases for well-known libraries
        match name {
            "c" | "libc" => "libSystem.B.dylib".to_string(),
            "m" | "libm" => "libSystem.B.dylib".to_string(), // Math is in libSystem on macOS
            "pthread" | "libpthread" => "libSystem.B.dylib".to_string(),
            "System" | "libSystem" => "libSystem.B.dylib".to_string(),
            _ => {
                // Add lib prefix and .dylib suffix
                if name.starts_with("lib") {
                    format!("{}.dylib", name)
                } else {
                    format!("lib{}.dylib", name)
                }
            }
        }
    }

    unsafe fn alloc_executable(&self, size: usize) -> Result<*mut u8, FfiPlatformError> {
        // On macOS, we need MAP_JIT for executable memory
        // Apple Silicon requires special handling with pthread_jit_write_protect
        let prot = libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC;
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_JIT;

        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                size,
                prot,
                flags,
                -1,
                0,
            )
        };

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
        "darwin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_library_name() {
        let platform = DarwinPlatform::new();

        assert_eq!(
            platform.normalize_library_name("c"),
            "libSystem.B.dylib"
        );
        assert_eq!(
            platform.normalize_library_name("foo"),
            "libfoo.dylib"
        );
        assert_eq!(
            platform.normalize_library_name("libbar"),
            "libbar.dylib"
        );
        assert_eq!(
            platform.normalize_library_name("test.dylib"),
            "test.dylib"
        );
    }

    #[test]
    fn test_platform_id() {
        let platform = DarwinPlatform::new();
        assert_eq!(platform.platform_id(), "darwin");
    }

    #[test]
    #[cfg(feature = "ffi")]
    fn test_load_libsystem() {
        let platform = DarwinPlatform::new();
        let handle = platform.load_library("System").expect("failed to load libSystem");
        assert!(!handle.is_null());

        // Resolve getpid
        let symbol = platform
            .resolve_symbol(handle, "getpid")
            .expect("failed to resolve getpid");
        assert!(!symbol.is_null());

        unsafe {
            platform.unload_library(handle).expect("failed to unload");
        }
    }

    #[test]
    fn test_errno_location() {
        let platform = DarwinPlatform::new();
        let errno_ptr = platform.errno_location();
        assert!(!errno_ptr.is_null());

        // We should be able to read errno
        unsafe {
            let _errno = *errno_ptr;
        }
    }
}
