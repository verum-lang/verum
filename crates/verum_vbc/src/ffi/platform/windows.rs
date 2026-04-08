//! Windows FFI platform implementation.
//!
//! Uses the following system APIs:
//! - `LoadLibraryW`/`GetProcAddress`/`FreeLibrary` for library loading
//! - `GetLastError()` for error retrieval
//! - `VirtualAlloc`/`VirtualFree` for executable memory

use super::{FfiPlatform, FfiPlatformError, LibraryHandle};
use std::ffi::{c_void, CString, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::ptr;

// Windows API type definitions
type HMODULE = *mut c_void;
type BOOL = i32;
type DWORD = u32;
type SIZE_T = usize;
type FARPROC = *const c_void;
type LPCSTR = *const i8;
type LPCWSTR = *const u16;
type LPVOID = *mut c_void;

// Memory allocation flags
const MEM_COMMIT: DWORD = 0x00001000;
const MEM_RESERVE: DWORD = 0x00002000;
const MEM_RELEASE: DWORD = 0x00008000;
const PAGE_EXECUTE_READWRITE: DWORD = 0x40;

// Load library flags
const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: DWORD = 0x00001000;

// External Windows API functions
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(lpLibFileName: LPCWSTR) -> HMODULE;
    fn LoadLibraryExW(lpLibFileName: LPCWSTR, hFile: HMODULE, dwFlags: DWORD) -> HMODULE;
    fn FreeLibrary(hLibModule: HMODULE) -> BOOL;
    fn GetProcAddress(hModule: HMODULE, lpProcName: LPCSTR) -> FARPROC;
    fn GetLastError() -> DWORD;
    fn FormatMessageW(
        dwFlags: DWORD,
        lpSource: LPVOID,
        dwMessageId: DWORD,
        dwLanguageId: DWORD,
        lpBuffer: *mut u16,
        nSize: DWORD,
        Arguments: *const c_void,
    ) -> DWORD;
    fn VirtualAlloc(lpAddress: LPVOID, dwSize: SIZE_T, flAllocationType: DWORD, flProtect: DWORD) -> LPVOID;
    fn VirtualFree(lpAddress: LPVOID, dwSize: SIZE_T, dwFreeType: DWORD) -> BOOL;
    fn LocalFree(hMem: LPVOID) -> LPVOID;
}

#[link(name = "ucrt")]
unsafe extern "C" {
    fn _errno() -> *mut i32;
}

// FormatMessage flags
const FORMAT_MESSAGE_FROM_SYSTEM: DWORD = 0x00001000;
const FORMAT_MESSAGE_ALLOCATE_BUFFER: DWORD = 0x00000100;
const FORMAT_MESSAGE_IGNORE_INSERTS: DWORD = 0x00000200;

/// Windows-specific FFI platform implementation.
///
/// Uses the Windows API for dynamic library loading and memory management.
pub struct WindowsPlatform {
    /// Cached handle to kernel32.dll for common operations.
    _kernel32: Option<LibraryHandle>,
}

impl WindowsPlatform {
    /// Creates a new Windows platform instance.
    pub fn new() -> Self {
        Self { _kernel32: None }
    }

    /// Convert a Rust string to a wide (UTF-16) null-terminated string.
    fn to_wide_string(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Get a human-readable error message for a Windows error code.
    fn get_error_message(error_code: DWORD) -> String {
        if error_code == 0 {
            return "No error".to_string();
        }

        unsafe {
            let mut buffer: *mut u16 = ptr::null_mut();
            let len = FormatMessageW(
                FORMAT_MESSAGE_ALLOCATE_BUFFER
                    | FORMAT_MESSAGE_FROM_SYSTEM
                    | FORMAT_MESSAGE_IGNORE_INSERTS,
                ptr::null_mut(),
                error_code,
                0, // Default language
                &mut buffer as *mut _ as *mut u16,
                0,
                ptr::null(),
            );

            if len == 0 || buffer.is_null() {
                return format!("Error code {}", error_code);
            }

            // Convert wide string to Rust String
            let slice = std::slice::from_raw_parts(buffer, len as usize);
            let message = String::from_utf16_lossy(slice).trim().to_string();

            LocalFree(buffer as LPVOID);

            if message.is_empty() {
                format!("Error code {}", error_code)
            } else {
                message
            }
        }
    }

    /// Get the last Windows error as a platform error.
    fn get_last_error(&self) -> FfiPlatformError {
        let code = unsafe { GetLastError() };
        FfiPlatformError::PlatformError {
            code: code as i32,
            message: Self::get_error_message(code),
        }
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiPlatform for WindowsPlatform {
    fn load_library(&self, name: &str) -> Result<LibraryHandle, FfiPlatformError> {
        // Normalize the library name for Windows
        let normalized = self.normalize_library_name(name);
        let wide_name = Self::to_wide_string(&normalized);

        // Try LoadLibraryExW with default search paths first (more secure)
        let handle = unsafe {
            LoadLibraryExW(
                wide_name.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };

        if !handle.is_null() {
            return Ok(unsafe { LibraryHandle::from_raw(handle) });
        }

        // Fall back to LoadLibraryW for backward compatibility
        let handle = unsafe { LoadLibraryW(wide_name.as_ptr()) };

        if !handle.is_null() {
            return Ok(unsafe { LibraryHandle::from_raw(handle) });
        }

        // Try with the original name if normalization didn't help
        if normalized != name {
            let wide_name = Self::to_wide_string(name);
            let handle = unsafe { LoadLibraryW(wide_name.as_ptr()) };

            if !handle.is_null() {
                return Ok(unsafe { LibraryHandle::from_raw(handle) });
            }
        }

        // Get the actual error
        let error_code = unsafe { GetLastError() };
        Err(FfiPlatformError::LibraryNotFound {
            name: name.to_string(),
            reason: Self::get_error_message(error_code),
        })
    }

    unsafe fn unload_library(&self, handle: LibraryHandle) -> Result<(), FfiPlatformError> {
        if handle.is_null() {
            return Ok(());
        }

        let result = FreeLibrary(handle.as_raw());

        if result != 0 {
            Ok(())
        } else {
            Err(self.get_last_error())
        }
    }

    fn resolve_symbol(
        &self,
        handle: LibraryHandle,
        name: &str,
    ) -> Result<*const (), FfiPlatformError> {
        // GetProcAddress uses ANSI strings (not wide strings)
        let cname = CString::new(name).map_err(|_| FfiPlatformError::SymbolNotFound {
            symbol: name.to_string(),
            library: "unknown".to_string(),
        })?;

        let symbol = unsafe { GetProcAddress(handle.as_raw(), cname.as_ptr()) };

        if symbol.is_null() {
            Err(FfiPlatformError::SymbolNotFound {
                symbol: name.to_string(),
                library: format!("{:?}", handle),
            })
        } else {
            Ok(symbol as *const ())
        }
    }

    fn errno_location(&self) -> *mut i32 {
        // Windows C runtime provides _errno() for thread-local errno access
        unsafe { _errno() }
    }

    fn normalize_library_name(&self, name: &str) -> String {
        // Already has .dll extension?
        if name.ends_with(".dll") || name.ends_with(".DLL") {
            return name.to_string();
        }

        // Full path?
        if name.contains('\\') || name.contains(':') || name.contains('/') {
            return name.to_string();
        }

        // Special cases for well-known libraries
        match name {
            // Standard C library
            "c" | "libc" | "msvcrt" => "msvcrt.dll".to_string(),
            "ucrt" | "libucrt" => "ucrtbase.dll".to_string(),
            // Core Windows DLLs
            "kernel32" => "kernel32.dll".to_string(),
            "user32" => "user32.dll".to_string(),
            "ntdll" => "ntdll.dll".to_string(),
            "advapi32" => "advapi32.dll".to_string(),
            "ws2_32" => "ws2_32.dll".to_string(),
            "shell32" => "shell32.dll".to_string(),
            "ole32" => "ole32.dll".to_string(),
            "gdi32" => "gdi32.dll".to_string(),
            // OpenGL
            "opengl32" => "opengl32.dll".to_string(),
            "gl" | "GL" => "opengl32.dll".to_string(),
            // Vulkan
            "vulkan" | "vulkan-1" => "vulkan-1.dll".to_string(),
            // CUDA (common locations)
            "cuda" => "nvcuda.dll".to_string(),
            "cudart" => "cudart64_12.dll".to_string(),
            _ => {
                // Add .dll suffix
                format!("{}.dll", name)
            }
        }
    }

    unsafe fn alloc_executable(&self, size: usize) -> Result<*mut u8, FfiPlatformError> {
        if size == 0 {
            return Err(FfiPlatformError::AllocationFailed {
                size,
                reason: "cannot allocate 0 bytes".to_string(),
            });
        }

        let ptr = VirtualAlloc(
            ptr::null_mut(),
            size,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_EXECUTE_READWRITE,
        );

        if ptr.is_null() {
            let error_code = unsafe { GetLastError() };
            Err(FfiPlatformError::AllocationFailed {
                size,
                reason: Self::get_error_message(error_code),
            })
        } else {
            Ok(ptr as *mut u8)
        }
    }

    unsafe fn free_executable(&self, ptr: *mut u8, _size: usize) -> Result<(), FfiPlatformError> {
        if ptr.is_null() {
            return Ok(());
        }

        // MEM_RELEASE requires dwSize to be 0
        let result = VirtualFree(ptr as LPVOID, 0, MEM_RELEASE);

        if result != 0 {
            Ok(())
        } else {
            Err(self.get_last_error())
        }
    }

    fn platform_id(&self) -> &'static str {
        "windows"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_library_name() {
        let platform = WindowsPlatform::new();

        // Standard C library
        assert_eq!(platform.normalize_library_name("c"), "msvcrt.dll");
        assert_eq!(platform.normalize_library_name("libc"), "msvcrt.dll");
        assert_eq!(platform.normalize_library_name("ucrt"), "ucrtbase.dll");

        // Core Windows DLLs
        assert_eq!(platform.normalize_library_name("kernel32"), "kernel32.dll");
        assert_eq!(platform.normalize_library_name("user32"), "user32.dll");
        assert_eq!(platform.normalize_library_name("ntdll"), "ntdll.dll");

        // Generic library name
        assert_eq!(platform.normalize_library_name("foo"), "foo.dll");

        // Already has .dll
        assert_eq!(platform.normalize_library_name("test.dll"), "test.dll");
        assert_eq!(platform.normalize_library_name("TEST.DLL"), "TEST.DLL");

        // Full path
        assert_eq!(
            platform.normalize_library_name("C:\\Windows\\System32\\test.dll"),
            "C:\\Windows\\System32\\test.dll"
        );
    }

    #[test]
    fn test_platform_id() {
        let platform = WindowsPlatform::new();
        assert_eq!(platform.platform_id(), "windows");
    }

    #[test]
    fn test_wide_string_conversion() {
        let wide = WindowsPlatform::to_wide_string("test");
        // "test" + null terminator = 5 u16 values
        assert_eq!(wide.len(), 5);
        assert_eq!(wide[4], 0); // null terminator

        // Unicode test
        let wide = WindowsPlatform::to_wide_string("тест");
        assert_eq!(wide.last(), Some(&0)); // null terminator
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_load_kernel32() {
        let platform = WindowsPlatform::new();

        // kernel32.dll should always be loadable on Windows
        let result = platform.load_library("kernel32");
        assert!(result.is_ok(), "Failed to load kernel32: {:?}", result);

        let handle = result.unwrap();
        assert!(!handle.is_null());

        // Resolve a known symbol
        let symbol = platform.resolve_symbol(handle, "GetLastError");
        assert!(symbol.is_ok(), "Failed to resolve GetLastError: {:?}", symbol);

        // Unload
        let result = unsafe { platform.unload_library(handle) };
        assert!(result.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_executable_memory() {
        let platform = WindowsPlatform::new();

        // Allocate executable memory
        let ptr = unsafe { platform.alloc_executable(4096) };
        assert!(ptr.is_ok(), "Failed to allocate: {:?}", ptr);

        let ptr = ptr.unwrap();
        assert!(!ptr.is_null());

        // Write a simple RET instruction (0xC3 on x86/x64)
        unsafe {
            *ptr = 0xC3;
        }

        // Free the memory
        let result = unsafe { platform.free_executable(ptr, 4096) };
        assert!(result.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_errno_location() {
        let platform = WindowsPlatform::new();
        let errno_ptr = platform.errno_location();
        assert!(!errno_ptr.is_null());

        // Should be able to read errno
        let _ = unsafe { *errno_ptr };
    }
}
