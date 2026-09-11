//! FFI runtime for VBC interpreter.
//!
//! This module provides the `FfiRuntime` which handles dynamic FFI calls
//! using libffi. It manages library loading, symbol resolution, and
//! call interface caching for optimal performance.
//!
//! # Performance
//!
//! - First call to a symbol: ~5μs (CIF preparation + symbol resolution)
//! - Subsequent calls: ~150ns (cached CIF + direct call)
//! - Memory: ~200 bytes per unique symbol

use std::collections::HashMap;
use std::fmt;

use libffi::low::{
    CodePtr, call, ffi_abi_FFI_DEFAULT_ABI, ffi_cif, ffi_type, prep_cif, prep_cif_var, types,
};

use super::CTypeRuntime;
use super::marshal::{ArrayBufferInfo, MarshalError, Marshaller};
use super::platform::{FfiPlatform, FfiPlatformError, LibraryHandle, create_platform};
use super::trampolines::{CallbackHandler, TrampolineId, TrampolineRegistry};
use crate::module::{FfiSymbolId, VbcModule};
use crate::value::Value;

/// Error type for FFI operations.
#[derive(Debug)]
#[allow(missing_docs)]
pub enum FfiError {
    /// Platform error (library loading, symbol resolution).
    Platform(FfiPlatformError),
    /// Marshalling error (type conversion).
    Marshal(MarshalError),
    /// Symbol not found in module.
    SymbolNotFound(FfiSymbolId),
    /// Library not found in module.
    LibraryNotFound(u16),
    /// Invalid calling convention.
    InvalidCallingConvention(u8),
    /// CIF preparation failed.
    CifPreparationFailed,
    /// Call failed.
    CallFailed(String),
    /// Argument count mismatch.
    ArgumentCountMismatch { expected: usize, got: usize },
}

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FfiError::Platform(e) => write!(f, "FFI platform error: {}", e),
            FfiError::Marshal(e) => write!(f, "FFI marshal error: {}", e),
            FfiError::SymbolNotFound(id) => write!(f, "FFI symbol not found: {:?}", id),
            FfiError::LibraryNotFound(id) => write!(f, "FFI library not found: {}", id),
            FfiError::InvalidCallingConvention(cc) => {
                write!(f, "invalid calling convention: {}", cc)
            }
            FfiError::CifPreparationFailed => write!(f, "CIF preparation failed"),
            FfiError::CallFailed(msg) => write!(f, "FFI call failed: {}", msg),
            FfiError::ArgumentCountMismatch { expected, got } => {
                write!(
                    f,
                    "argument count mismatch: expected {}, got {}",
                    expected, got
                )
            }
        }
    }
}

impl std::error::Error for FfiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FfiError::Platform(e) => Some(e),
            FfiError::Marshal(e) => Some(e),
            _ => None,
        }
    }
}

impl From<FfiPlatformError> for FfiError {
    fn from(e: FfiPlatformError) -> Self {
        FfiError::Platform(e)
    }
}

impl From<MarshalError> for FfiError {
    fn from(e: MarshalError) -> Self {
        FfiError::Marshal(e)
    }
}

/// T1304 — the C type to use for an argument in a variadic call's TAIL.
///
/// A variadic declaration says nothing about the types after `...`, so
/// the only source is the runtime value. C's default argument
/// promotions apply to that tail: `float` is passed as `double`, and
/// integer types narrower than `int` are passed as `int`.
///
/// WHAT THIS MAPPING IS AND IS NOT. Verum's `Int` is 64-bit and its
/// `Float` is 64-bit, so `I64`/`F64` are the promoted forms and no
/// widening is needed. That makes the mapping right for every
/// specifier that reads 64 bits — `%ld`, `%lld`, `%f`, `%p` — and
/// WRONG for `%d`, which reads 32. The format string lives in the
/// callee, not here, so no choice made at this point can be correct for
/// every format; this one is correct for the widths Verum can actually
/// produce, and a caller that needs `%d` must say `%ld` or pass through
/// a fixed-arity shim.
fn variadic_tail_ctype(v: &Value) -> CTypeRuntime {
    if v.is_float() {
        // C promotes float to double in the variadic tail; Verum's
        // Float is already 64-bit, so this is the promoted form.
        CTypeRuntime::F64
    } else if v.is_ptr() {
        CTypeRuntime::Ptr
    } else {
        // Ints, Bools and anything else Verum can hand over travel as a
        // 64-bit integer — see the width note above.
        CTypeRuntime::I64
    }
}

/// A resolved FFI symbol with cached call information.
pub struct ResolvedSymbol {
    /// Raw function pointer.
    pub ptr: *const (),
    /// Prepared CIF for this symbol.
    cif: Box<ffi_cif>,
    /// Cached argument types (kept alive for CIF).
    _arg_types: Vec<*mut ffi_type>,
    /// Return type.
    pub return_type: CTypeRuntime,
    /// Argument types.
    pub arg_types: Vec<CTypeRuntime>,
}

// SAFETY: The function pointer and CIF are thread-safe once prepared.
// The actual calls must still follow FFI safety rules.
unsafe impl Send for ResolvedSymbol {}
unsafe impl Sync for ResolvedSymbol {}

impl fmt::Debug for ResolvedSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedSymbol")
            .field("ptr", &self.ptr)
            .field("return_type", &self.return_type)
            .field("arg_types", &self.arg_types)
            .finish()
    }
}

/// Holds a dynamically created libffi struct type.
///
/// This struct keeps the ffi_type and its element array alive for the lifetime
/// of the FFI runtime. The element array must be null-terminated for libffi.
pub struct FfiStructType {
    /// The libffi type descriptor for this struct.
    ffi_type: ffi_type,
    /// Array of pointers to field types (must be null-terminated).
    /// Kept alive to prevent use-after-free.
    _elements: Vec<*mut ffi_type>,
    /// Struct size in bytes.
    pub size: u32,
    /// Struct alignment.
    pub alignment: u16,
}

// SAFETY: FfiStructType only contains raw pointers to static type descriptors
// and a ffi_type which is thread-safe once initialized.
unsafe impl Send for FfiStructType {}
unsafe impl Sync for FfiStructType {}

/// FFI_TYPE_STRUCT value from libffi (defined as 13 in ffi.h).
const FFI_TYPE_STRUCT: u16 = 13;

impl FfiStructType {
    /// Creates a new libffi struct type from field types.
    ///
    /// The field_types should be the libffi type pointers for each field.
    pub fn new(field_types: Vec<*mut ffi_type>, size: u32, alignment: u16) -> Self {
        // libffi requires a null-terminated array of element pointers
        let mut elements = field_types;
        elements.push(std::ptr::null_mut());

        let ffi_type = ffi_type {
            size: size as usize,
            alignment,
            type_: FFI_TYPE_STRUCT,
            elements: elements.as_mut_ptr(),
        };

        Self {
            ffi_type,
            _elements: elements,
            size,
            alignment,
        }
    }

    /// Gets a mutable pointer to the ffi_type for use with libffi.
    pub fn as_ffi_type_ptr(&mut self) -> *mut ffi_type {
        &mut self.ffi_type as *mut ffi_type
    }
}

/// FFI runtime for the VBC interpreter.
///
/// Manages library loading, symbol resolution, and FFI calls with
/// caching for optimal performance.
pub struct FfiRuntime {
    /// Platform abstraction.
    platform: Box<dyn FfiPlatform>,
    /// Loaded libraries by module index.
    libraries: HashMap<u16, LibraryHandle>,
    /// Resolved symbols by module symbol index.
    symbols: HashMap<u32, ResolvedSymbol>,
    /// Marshaller for type conversion.
    marshaller: Marshaller,
    /// Trampoline registry for callbacks (Verum->C).
    trampolines: TrampolineRegistry,
    /// Cache of dynamically created libffi struct types by layout index.
    struct_types: HashMap<u16, Box<FfiStructType>>,
}

impl FfiRuntime {
    /// Creates a new FFI runtime.
    pub fn new() -> Result<Self, FfiError> {
        Ok(Self {
            platform: create_platform(),
            libraries: HashMap::new(),
            symbols: HashMap::new(),
            marshaller: Marshaller::new(),
            trampolines: TrampolineRegistry::new(),
            struct_types: HashMap::new(),
        })
    }

    /// Loads a library by name and caches the handle.
    pub fn load_library(&mut self, name: &str) -> Result<LibraryHandle, FfiError> {
        self.platform.load_library(name).map_err(FfiError::from)
    }

    /// Gets or creates a libffi struct type from a module's layout.
    ///
    /// The struct type is cached by layout index for reuse.
    pub fn get_or_create_struct_type(
        &mut self,
        module: &VbcModule,
        layout_idx: u16,
    ) -> Result<*mut ffi_type, FfiError> {
        // Check cache first
        if let Some(struct_type) = self.struct_types.get_mut(&layout_idx) {
            return Ok(struct_type.as_ffi_type_ptr());
        }

        // Get the layout from the module
        let layout = module.ffi_layouts.get(layout_idx as usize).ok_or_else(|| {
            FfiError::CallFailed(format!("FFI struct layout {} not found", layout_idx))
        })?;

        // Build the field type array
        let mut field_types: Vec<*mut ffi_type> = Vec::with_capacity(layout.fields.len());
        for field in &layout.fields {
            // Convert field CType to runtime type - StructValue in fields not currently supported
            let ctype_runtime: CTypeRuntime = field.c_type.into();
            field_types.push(ctype_to_ffi_type(ctype_runtime));
        }

        // Create and cache the struct type
        let struct_type = FfiStructType::new(field_types, layout.size, layout.align);
        self.struct_types.insert(layout_idx, Box::new(struct_type));

        // Return the pointer
        Ok(self
            .struct_types
            .get_mut(&layout_idx)
            .unwrap()
            .as_ffi_type_ptr())
    }

    /// Gets the libffi type for a CTypeRuntime, handling struct types.
    ///
    /// For struct-by-value types, this uses the struct type cache.
    fn get_ffi_type(
        &mut self,
        ctype: CTypeRuntime,
        module: &VbcModule,
    ) -> Result<*mut ffi_type, FfiError> {
        match ctype {
            CTypeRuntime::StructValue(layout_idx) => {
                self.get_or_create_struct_type(module, layout_idx)
            }
            _ => Ok(ctype_to_ffi_type(ctype)),
        }
    }

    /// Resolves a symbol in a library.
    ///
    /// Note: This method doesn't handle struct-by-value types. For those,
    /// use `resolve_symbol_with_structs` which can create dynamic struct types.
    pub fn resolve_symbol(
        &mut self,
        handle: LibraryHandle,
        name: &str,
        return_type: CTypeRuntime,
        arg_types: Vec<CTypeRuntime>,
    ) -> Result<ResolvedSymbol, FfiError> {
        // Resolve the symbol
        let ptr = self.platform.resolve_symbol(handle, name)?;

        // Prepare the CIF
        let mut cif = Box::new(ffi_cif::default());

        // Convert types to libffi types
        let ret_ffi_type = ctype_to_ffi_type(return_type);
        let mut arg_ffi_types: Vec<*mut ffi_type> =
            arg_types.iter().map(|t| ctype_to_ffi_type(*t)).collect();

        // Prepare the CIF
        unsafe {
            prep_cif(
                cif.as_mut(),
                ffi_abi_FFI_DEFAULT_ABI,
                arg_ffi_types.len(),
                ret_ffi_type,
                arg_ffi_types.as_mut_ptr(),
            )
            .map_err(|_| FfiError::CifPreparationFailed)?;
        }

        Ok(ResolvedSymbol {
            ptr,
            cif,
            _arg_types: arg_ffi_types,
            return_type,
            arg_types,
        })
    }

    /// Resolves a symbol with support for struct-by-value types.
    ///
    /// This method can handle StructValue types by looking up struct layouts
    /// from the module and creating dynamic libffi struct types.
    pub fn resolve_symbol_with_structs(
        &mut self,
        module: &VbcModule,
        handle: LibraryHandle,
        name: &str,
        return_type: CTypeRuntime,
        arg_types: Vec<CTypeRuntime>,
    ) -> Result<ResolvedSymbol, FfiError> {
        // Resolve the symbol
        let ptr = self.platform.resolve_symbol(handle, name)?;

        // Prepare the CIF
        let mut cif = Box::new(ffi_cif::default());

        // Convert types to libffi types, handling struct types
        let ret_ffi_type = self.get_ffi_type(return_type, module)?;
        let mut arg_ffi_types: Vec<*mut ffi_type> = Vec::with_capacity(arg_types.len());
        for t in &arg_types {
            arg_ffi_types.push(self.get_ffi_type(*t, module)?);
        }

        // Prepare the CIF
        unsafe {
            prep_cif(
                cif.as_mut(),
                ffi_abi_FFI_DEFAULT_ABI,
                arg_ffi_types.len(),
                ret_ffi_type,
                arg_ffi_types.as_mut_ptr(),
            )
            .map_err(|_| FfiError::CifPreparationFailed)?;
        }

        Ok(ResolvedSymbol {
            ptr,
            cif,
            _arg_types: arg_ffi_types,
            return_type,
            arg_types,
        })
    }

    /// Loads all libraries required by a module.
    ///
    /// Libraries tagged with a specific platform (Darwin / Linux / Windows /
    /// FreeBSD / Ios / Android) are only loaded when the current target OS
    /// matches. Cross-platform (`Any`) libraries are always loaded. Without
    /// this filter, running e.g. a macOS build would try to `dlopen("kernel32.dll")`
    /// and fail with the confusing message
    /// `library 'kernel32.dll' not found: dlopen(libkernel32.dll.B.dylib, ...)`.
    pub fn load_module_libraries(&mut self, module: &VbcModule) -> Result<(), FfiError> {
        for (idx, lib) in module.ffi_libraries.iter().enumerate() {
            let idx = idx as u16;
            if self.libraries.contains_key(&idx) {
                continue;
            }

            // Skip libraries gated to a different platform. An entry with
            // `FfiPlatform::Any` always matches.
            if !lib.platform.matches_current() {
                continue;
            }

            // Resolve library name from string table
            let lib_name = module.strings.get(lib.name).unwrap_or("");

            // Try multiple resolution strategies for relative paths
            let resolved_path = self.resolve_library_path(lib_name, module.source_dir.as_deref());

            match self.platform.load_library(&resolved_path) {
                Ok(handle) => {
                    self.libraries.insert(idx, handle);
                }
                Err(e) => {
                    if lib.required {
                        return Err(FfiError::Platform(e));
                    }
                    // Optional library not found, continue
                }
            }
        }
        Ok(())
    }

    /// Resolve a library path by trying multiple locations.
    fn resolve_library_path(&self, lib_name: &str, source_dir: Option<&str>) -> String {
        use std::path::Path;

        // Absolute path - use as-is
        if lib_name.starts_with('/') {
            return lib_name.to_string();
        }

        // Library name without path components - let platform search
        if !lib_name.contains('/') {
            return lib_name.to_string();
        }

        // Relative path - try multiple locations
        let _paths_to_try: Vec<std::path::PathBuf> = {
            let mut paths = Vec::new();

            // 1. Try relative to current working directory
            let cwd_path = Path::new(lib_name);
            if cwd_path.exists() {
                return cwd_path.to_string_lossy().into_owned();
            }

            // 2. Try relative to source directory
            if let Some(src_dir) = source_dir {
                let src_resolved = Path::new(src_dir).join(lib_name);
                if src_resolved.exists() {
                    return src_resolved.to_string_lossy().into_owned();
                }
                paths.push(src_resolved);
            }

            // 3. Try to find project root and resolve relative to it
            // Look for Cargo.toml or .git directory
            if let Ok(cwd) = std::env::current_dir() {
                let mut check_dir = cwd.as_path();
                loop {
                    // Check if this looks like project root
                    if check_dir.join("Cargo.toml").exists() || check_dir.join(".git").exists() {
                        let project_resolved = check_dir.join(lib_name);
                        if project_resolved.exists() {
                            return project_resolved.to_string_lossy().into_owned();
                        }
                        paths.push(project_resolved);
                        break;
                    }
                    match check_dir.parent() {
                        Some(parent) => check_dir = parent,
                        None => break,
                    }
                }
            }

            paths
        };

        // None of the paths exist, return original (will fail with clear error)
        lib_name.to_string()
    }

    /// Resolves a symbol from a module by symbol ID.
    pub fn resolve_module_symbol(
        &mut self,
        module: &VbcModule,
        symbol_id: FfiSymbolId,
    ) -> Result<&ResolvedSymbol, FfiError> {
        let idx = symbol_id.0;

        // Check cache first
        if self.symbols.contains_key(&idx) {
            return Ok(self.symbols.get(&idx).unwrap());
        }

        // Get symbol info from module
        let symbol = module
            .get_ffi_symbol(symbol_id)
            .ok_or(FfiError::SymbolNotFound(symbol_id))?;

        // Get library handle
        let lib_idx = symbol.library_idx;
        let handle = if lib_idx < 0 {
            // Negative index means use current process (RTLD_DEFAULT equivalent)
            unsafe { LibraryHandle::from_raw(std::ptr::null_mut()) }
        } else {
            *self
                .libraries
                .get(&(lib_idx as u16))
                .ok_or(FfiError::LibraryNotFound(lib_idx as u16))?
        };

        // Convert signature to runtime types, handling struct-by-value with layout indices
        let return_type = CTypeRuntime::from_ctype_with_layout(
            symbol.signature.return_type,
            symbol.signature.return_layout_idx,
        );
        let arg_types: Vec<CTypeRuntime> = symbol
            .signature
            .param_types
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let layout_idx = symbol
                    .signature
                    .param_layout_indices
                    .get(i)
                    .copied()
                    .flatten();
                CTypeRuntime::from_ctype_with_layout(*t, layout_idx)
            })
            .collect();

        // Resolve symbol name from string table
        let symbol_name = module.strings.get(symbol.name).unwrap_or("");

        // Resolve the symbol with struct type support
        let resolved =
            self.resolve_symbol_with_structs(module, handle, symbol_name, return_type, arg_types)?;

        self.symbols.insert(idx, resolved);
        Ok(self.symbols.get(&idx).unwrap())
    }

    /// Calls an FFI function with C calling convention.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - Arguments match the function signature
    /// - The function pointer is valid
    /// - Any pointers in arguments point to valid memory
    pub unsafe fn call_ffi_c(
        &mut self,
        symbol: &ResolvedSymbol,
        args: &[Value],
        ret_value: &mut Value,
    ) -> Result<(), FfiError> {
        // Check argument count
        if args.len() != symbol.arg_types.len() {
            return Err(FfiError::ArgumentCountMismatch {
                expected: symbol.arg_types.len(),
                got: args.len(),
            });
        }

        // Marshal arguments
        // For pointer types, use value_to_c_ref which allocates temp storage
        // for non-pointer Values (like Int) that need to be passed by reference.
        let mut raw_args: Vec<u64> = Vec::with_capacity(args.len());
        let mut arg_ptrs: Vec<*mut std::ffi::c_void> = Vec::with_capacity(args.len());
        // Note: call_ffi_c doesn't support struct-by-value args - use call_module_ffi_c for that

        for (arg, ctype) in args.iter().zip(symbol.arg_types.iter()) {
            if matches!(ctype, CTypeRuntime::StructValue(_)) {
                // call_ffi_c doesn't have access to module layouts - must use call_module_ffi_c
                return Err(FfiError::Marshal(MarshalError::UnsupportedConversion {
                    from: "struct-by-value",
                    to: *ctype,
                }));
            }
            let raw = self.marshaller.value_to_c_ref(*arg, *ctype, None)?;
            raw_args.push(raw);
        }

        // Create pointers to arguments
        for raw in &mut raw_args {
            arg_ptrs.push(raw as *mut u64 as *mut std::ffi::c_void);
        }

        // Prepare for the call
        let cif_ptr = symbol.cif.as_ref() as *const ffi_cif as *mut ffi_cif;
        let code_ptr = CodePtr::from_ptr(symbol.ptr as *const std::ffi::c_void);

        // Handle struct-by-value returns specially
        if let CTypeRuntime::StructValue(layout_idx) = symbol.return_type {
            // Get struct size from the cached type
            let struct_size = self
                .struct_types
                .get(&layout_idx)
                .map(|st| st.size as usize)
                .unwrap_or(0);

            if struct_size > 0 {
                // Allocate a buffer for the struct return value
                let ret_buffer = Box::new([0u8; 256]); // Max struct size we support
                let ret_ptr = Box::into_raw(ret_buffer) as *mut u8;

                // For struct returns, libffi writes to the address we provide
                // We pass the buffer address as the return storage
                // SAFETY: We've validated argument count and types, caller ensures pointers are valid.
                unsafe {
                    // Use ffi_call directly with a return buffer
                    libffi::raw::ffi_call(
                        cif_ptr,
                        Some(std::mem::transmute::<
                            *const std::ffi::c_void,
                            unsafe extern "C" fn(),
                        >(code_ptr.as_ptr())),
                        ret_ptr as *mut std::ffi::c_void,
                        arg_ptrs.as_mut_ptr(),
                    );
                }

                // Return the struct buffer as a pointer - caller handles struct unpacking
                *ret_value = Value::from_ptr(ret_ptr);
                // Note: The buffer is intentionally leaked and will be managed by the caller
            } else {
                *ret_value = Value::nil();
            }
        } else {
            // Make the call and get result
            // Use u64 as the return type to handle all primitive return types
            // SAFETY: We've validated argument count and types, caller ensures pointers are valid.
            let ret_storage: u64 = unsafe { call::<u64>(cif_ptr, code_ptr, arg_ptrs.as_mut_ptr()) };

            // Marshal return value
            *ret_value = self
                .marshaller
                .c_to_value(ret_storage, symbol.return_type)?;
        }

        Ok(())
    }

    /// Calls an FFI function using C calling convention with proper write-back support.
    ///
    /// This version uses a source register map that maps argument indices to the
    /// original variable registers, enabling proper write-back for mutable references.
    /// When `&mut y` is passed to FFI, the write-back goes to y's register, not to
    /// the temporary register holding the reference value.
    ///
    /// # Arguments
    ///
    /// * `module` - The VBC module containing FFI metadata
    /// * `symbol_id` - The FFI symbol to call
    /// * `args` - The argument values
    /// * `source_reg_map` - Maps argument index to source variable register for write-back
    /// * `ret_value` - Output parameter for return value
    ///
    /// # Returns
    ///
    /// A vector of (register_index, new_value) pairs for write-back.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - Arguments match the function signature
    /// - Any pointers in arguments point to valid memory
    pub unsafe fn call_module_ffi_c_with_writeback_v2(
        &mut self,
        module: &VbcModule,
        symbol_id: FfiSymbolId,
        args: &[Value],
        source_reg_map: &std::collections::HashMap<u8, u16>,
        ret_value: &mut Value,
    ) -> Result<Vec<(u16, Value)>, FfiError> {
        // First, ensure the symbol is resolved
        let idx = symbol_id.0;

        // Check cache first
        if !self.symbols.contains_key(&idx) {
            // Get symbol info from module
            let symbol = module
                .get_ffi_symbol(symbol_id)
                .ok_or(FfiError::SymbolNotFound(symbol_id))?;

            // Get library handle
            let lib_idx = symbol.library_idx;
            let handle = if lib_idx < 0 {
                unsafe { LibraryHandle::from_raw(std::ptr::null_mut()) }
            } else {
                *self
                    .libraries
                    .get(&(lib_idx as u16))
                    .ok_or(FfiError::LibraryNotFound(lib_idx as u16))?
            };

            // Convert signature to runtime types (using layout indices for struct-by-value)
            let return_type = CTypeRuntime::from_ctype_with_layout(
                symbol.signature.return_type,
                symbol.signature.return_layout_idx,
            );
            let arg_types: Vec<CTypeRuntime> = symbol
                .signature
                .param_types
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let layout_idx = symbol
                        .signature
                        .param_layout_indices
                        .get(i)
                        .copied()
                        .flatten();
                    CTypeRuntime::from_ctype_with_layout(*t, layout_idx)
                })
                .collect();

            // Resolve symbol name from string table
            let symbol_name = module.strings.get(symbol.name).unwrap_or("");

            // Resolve the symbol (with struct support for struct-by-value)
            let resolved = self.resolve_symbol_with_structs(
                module,
                handle,
                symbol_name,
                return_type,
                arg_types,
            )?;
            self.symbols.insert(idx, resolved);
        }

        // Now get the symbol and call it
        let symbol = self.symbols.get(&idx).unwrap();

        // T1304 — A VARIADIC EXTERN IS CALLED WITH MORE ARGUMENTS THAN
        // IT DECLARES, and the CIF must describe THIS CALL.
        //
        // `ffi_prep_cif_var` is not a variant of `ffi_prep_cif` over the
        // same subject: it describes a CALL SITE, because it needs to
        // know where the fixed part ends. `snprintf(buf, n, "%d", x)`
        // and `snprintf(buf, n, "%s%d", s, x)` therefore need DIFFERENT
        // cifs, and the per-symbol cache above cannot hold both. So a
        // variadic symbol bypasses the cache and prepares its cif here,
        // from the actual arguments. Non-variadic calls are untouched —
        // they keep the cached cif and the identical code path.
        let (sym_is_variadic, sym_fixed_n) = module
            .get_ffi_symbol(symbol_id)
            .map(|s| {
                (
                    s.signature.is_variadic,
                    s.signature.fixed_param_count as usize,
                )
            })
            .unwrap_or((false, symbol.arg_types.len()));

        // The declared types cover the fixed part only; the tail's types
        // come from the values (see `variadic_tail_ctype`).
        let mut effective_arg_types: Vec<CTypeRuntime> = symbol.arg_types.clone();
        if sym_is_variadic && args.len() > effective_arg_types.len() {
            for extra in &args[effective_arg_types.len()..] {
                effective_arg_types.push(variadic_tail_ctype(extra));
            }
        }

        // Check argument count. For a variadic symbol the tail was just
        // materialised, so this still catches "fewer than declared".
        if args.len() != effective_arg_types.len() {
            return Err(FfiError::ArgumentCountMismatch {
                expected: effective_arg_types.len(),
                got: args.len(),
            });
        }

        // Prepare a call-site cif for the variadic case. Held in this
        // frame so it outlives the call; `variadic_atypes` must outlive
        // it too, which is why both are bound here and not in a block.
        let mut variadic_cif: Option<Box<ffi_cif>> = None;
        let mut variadic_atypes: Vec<*mut ffi_type> = if sym_is_variadic {
            effective_arg_types
                .iter()
                .map(|t| ctype_to_ffi_type(*t))
                .collect()
        } else {
            Vec::new()
        };
        if sym_is_variadic {
            let mut cif = Box::new(ffi_cif::default());
            unsafe {
                prep_cif_var(
                    cif.as_mut(),
                    ffi_abi_FFI_DEFAULT_ABI,
                    sym_fixed_n,
                    variadic_atypes.len(),
                    ctype_to_ffi_type(symbol.return_type),
                    variadic_atypes.as_mut_ptr(),
                )
                .map_err(|_| FfiError::CifPreparationFailed)?;
            }
            variadic_cif = Some(cif);
        }

        // Clear any previous ref arg storage
        self.marshaller.clear_cache();

        // Marshal arguments with source register indices for write-back
        // For pointer types, use value_to_c_ref which allocates temp storage
        let mut raw_args: Vec<u64> = Vec::with_capacity(args.len());
        let mut arg_ptrs: Vec<*mut std::ffi::c_void> = Vec::with_capacity(args.len());
        // Storage for struct-by-value arguments (kept alive until after the call)
        let mut struct_arg_buffers: Vec<Box<[u8; 256]>> = Vec::new();
        // **B1d FFI byte-buffer marshalling (VERUM_FFI_PACK).** A Verum
        // `List<Byte>` backing is NaN-boxed 8-byte-strided `Value`s and a
        // `BYTE_SLICE` is a header+{ptr,len} view — neither is contiguous
        // ABI bytes, so passing the raw heap pointer to a C `void*`
        // buffer param made the callee read/write the wrong bytes
        // (sockaddr sin_family=0; recv/getsockname OUT-params). Pack such
        // args into contiguous C storage before the call and copy-back
        // mutable ones after. Gated OFF by default (A/B on the shared
        // tree); the fix is correct-when-on and becomes the default once
        // the AOT mirror lands.
        let ffi_pack = std::env::var("VERUM_FFI_PACK").is_ok();
        let ffi_trace = std::env::var("VERUM_TRACE_FFI_ARG").is_ok();
        // Track which arguments are struct-by-value (we need to handle them specially for arg_ptrs)
        let mut struct_arg_indices: Vec<(usize, usize)> = Vec::new(); // (raw_args index, struct_buffer index)
        // Track struct pointer arguments for write-back: (layout_idx, obj_ptr, buffer_idx)
        let mut struct_ptr_writebacks: Vec<(u16, *mut u8, usize)> = Vec::new();

        for (i, (arg, ctype)) in args.iter().zip(effective_arg_types.iter()).enumerate() {
            // Handle struct-by-value arguments specially
            if let CTypeRuntime::StructValue(layout_idx) = ctype {
                // Get the struct layout
                if let Some(layout) = module.ffi_layouts.get(*layout_idx as usize) {
                    // Allocate a buffer for the C struct
                    let mut struct_buffer = Box::new([0u8; 256]);

                    // The Verum value should be a pointer to a heap object
                    let obj_ptr = arg.as_ptr::<u8>();
                    if !obj_ptr.is_null() {
                        // Use helper function to marshal Verum struct to C buffer
                        unsafe {
                            marshal_verum_struct_to_c(
                                layout,
                                &module.ffi_layouts,
                                obj_ptr,
                                &mut struct_buffer,
                            )
                        };
                    }

                    // Track this as a struct argument (we'll set arg_ptrs[i] to point directly to the buffer later)
                    struct_arg_indices.push((raw_args.len(), struct_arg_buffers.len()));
                    struct_arg_buffers.push(struct_buffer);
                    raw_args.push(0); // placeholder - will use buffer pointer directly
                } else {
                    raw_args.push(0);
                }
            } else if let CTypeRuntime::StructPtr(layout_idx) = ctype {
                // Handle struct-pointer arguments: convert Verum heap object to C struct buffer
                // and pass a pointer to that buffer
                if let Some(layout) = module.ffi_layouts.get(*layout_idx as usize) {
                    // Allocate a buffer for the C struct
                    let mut struct_buffer = Box::new([0u8; 256]);

                    // The Verum value should be a pointer to a heap object
                    let obj_ptr = arg.as_ptr::<u8>();
                    if !obj_ptr.is_null() {
                        // Use helper function to marshal Verum struct to C buffer
                        unsafe {
                            marshal_verum_struct_to_c(
                                layout,
                                &module.ffi_layouts,
                                obj_ptr,
                                &mut struct_buffer,
                            )
                        };
                    }

                    // Track for write-back: (layout_idx, obj_ptr, buffer_idx)
                    // We write back for ALL struct pointer args since we don't know mutability at runtime
                    struct_ptr_writebacks.push((*layout_idx, obj_ptr, struct_arg_buffers.len()));

                    // For struct pointers, we push the POINTER to the buffer into raw_args,
                    // then arg_ptrs[i] = &raw_args[i] is a pointer to a pointer,
                    // which is what libffi expects for pointer arguments
                    let buffer_ptr = struct_buffer.as_ptr() as u64;
                    struct_arg_buffers.push(struct_buffer);
                    raw_args.push(buffer_ptr);
                } else {
                    raw_args.push(0);
                }
            } else {
                // For pointer types that are mutable, use the SOURCE register for write-back
                // This is the original variable's register, not the temporary ref register
                let write_back_reg = if matches!(ctype, CTypeRuntime::Ptr | CTypeRuntime::ArrayPtr)
                {
                    // Look up the source register from the map
                    source_reg_map.get(&(i as u8)).copied()
                } else {
                    None
                };

                // **B1d — diagnostic only (VERUM_TRACE_FFI_ARG).** The
                // value-inspection packing that lived here is FUNDAMENTALLY
                // UNSAFE and was removed: a raw data pointer (`&sa[0]` — a
                // `ByteArrayElementAddr` into the MIDDLE of a heap object)
                // is indistinguishable from a heap-object header at the FFI
                // boundary. A sockaddr starting `[16, 2, 0, 0]` reads back
                // as `type_id = 0x00000210 = 528 = BYTE_SLICE`, so the
                // packer mis-identified `dest_addr` as a byte-slice,
                // byte_slice_payload'd the sockaddr bytes into a garbage
                // pointer, and `sendto` returned EDESTADDRREQ(39). The
                // correct fix is CTYPE-DRIVEN: FFI params must be typed
                // `&[Byte]`/`&mut [Byte]` distinctly from raw `&unsafe
                // Byte`, so the marshaller packs by SIGNATURE, never by
                // guessing from the value's bytes (task #23).
                if ffi_trace
                    && matches!(ctype, CTypeRuntime::Ptr | CTypeRuntime::ArrayPtr)
                    && arg.is_ptr()
                    && !arg.is_nil()
                    && !arg.is_boxed_int()
                {
                    use crate::interpreter::ObjectHeader;
                    let base = arg.as_ptr::<u8>();
                    let tid = unsafe { ObjectHeader::try_type_id(base) };
                    eprintln!(
                        "[ffi-arg] i={} ctype={:?} type_id={:?} mut={}",
                        i,
                        ctype,
                        tid,
                        write_back_reg.is_some()
                    );
                }
                let _ = ffi_pack;
                let raw = self
                    .marshaller
                    .value_to_c_ref(*arg, *ctype, write_back_reg)?;
                raw_args.push(raw);
            }
        }

        // Create pointers to arguments
        // For scalar types: arg_ptrs[i] = &raw_args[i] (pointer to the value)
        // For struct-by-value: arg_ptrs[i] = pointer to the struct buffer directly
        for raw in &mut raw_args {
            arg_ptrs.push(raw as *mut u64 as *mut std::ffi::c_void);
        }
        // Now fix up struct-by-value arguments to point directly to their buffers
        for (arg_idx, buffer_idx) in &struct_arg_indices {
            arg_ptrs[*arg_idx] = struct_arg_buffers[*buffer_idx].as_ptr() as *mut std::ffi::c_void;
        }

        // Get symbol info for call. A variadic symbol uses the
        // call-site cif built above; everything else uses the cached one.
        let cif_ptr = match &variadic_cif {
            Some(c) => c.as_ref() as *const ffi_cif as *mut ffi_cif,
            None => symbol.cif.as_ref() as *const ffi_cif as *mut ffi_cif,
        };
        let code_ptr = CodePtr::from_ptr(symbol.ptr as *const std::ffi::c_void);
        let return_type = symbol.return_type;

        // Handle struct-by-value returns specially
        if let CTypeRuntime::StructValue(layout_idx) = return_type {
            // Get struct size from the cached type
            let struct_size = self
                .struct_types
                .get(&layout_idx)
                .map(|st| st.size as usize)
                .unwrap_or(0);

            if struct_size > 0 {
                // Allocate a buffer for the struct return value
                let ret_buffer = Box::new([0u8; 256]); // Max struct size we support
                let ret_ptr = Box::into_raw(ret_buffer) as *mut u8;

                // For struct returns, libffi writes to the address we provide
                unsafe {
                    libffi::raw::ffi_call(
                        cif_ptr,
                        Some(std::mem::transmute::<
                            *const std::ffi::c_void,
                            unsafe extern "C" fn(),
                        >(code_ptr.as_ptr())),
                        ret_ptr as *mut std::ffi::c_void,
                        arg_ptrs.as_mut_ptr(),
                    );
                }

                // Return the struct buffer as a pointer - dispatch code handles struct unpacking
                *ret_value = Value::from_ptr(ret_ptr);
            } else {
                *ret_value = Value::nil();
            }
        } else {
            // Make the call for scalar types
            let ret_storage: u64 = unsafe { call::<u64>(cif_ptr, code_ptr, arg_ptrs.as_mut_ptr()) };

            // Marshal return value
            *ret_value = self.marshaller.c_to_value(ret_storage, return_type)?;
        }

        // Write back struct pointer arguments (for mutable references)
        // Since we don't know at runtime which are mutable, we write back all of them
        for (layout_idx, obj_ptr, buffer_idx) in &struct_ptr_writebacks {
            if obj_ptr.is_null() {
                continue;
            }
            if let Some(layout) = module.ffi_layouts.get(*layout_idx as usize) {
                let struct_buffer = &struct_arg_buffers[*buffer_idx];
                // Use helper function to marshal C buffer back to Verum struct
                unsafe {
                    marshal_c_to_verum_struct(
                        layout,
                        &module.ffi_layouts,
                        struct_buffer,
                        *obj_ptr,
                    )
                };
            }
        }

        // Collect write-back values for mutable reference arguments
        // The write_back_reg now points to the ORIGINAL variable register
        let mut writebacks: Vec<(u16, Value)> = Vec::new();
        for storage in self.marshaller.ref_arg_storage() {
            if let Some(reg) = storage.write_back_reg {
                // Read the potentially modified value from storage
                let raw_value = storage.read();
                // T1410 — RE-BOX BY THE KIND THAT WAS MARSHALLED IN.
                //
                // This used to be `Value::from_i64(raw_value as i64)`
                // unconditionally. The bytes were right and the TYPE was
                // lost: `modf(3.75, &mut ip)` left `ip` reading
                // 4613937818241073152, the IEEE bit pattern of 3.0, boxed
                // as an integer. The obvious assertion for an out-parameter
                // — `ip != 0.0`, "did anything get written" — passes on
                // that, which is why it hid.
                let value = match storage.kind {
                    crate::ffi::marshal::RefArgKind::Float => {
                        Value::from_f64(f64::from_bits(raw_value))
                    }
                    crate::ffi::marshal::RefArgKind::Bool => {
                        Value::from_bool(raw_value != 0)
                    }
                    crate::ffi::marshal::RefArgKind::Int => {
                        Value::from_i64(raw_value as i64)
                    }
                };
                writebacks.push((reg, value));
            }
        }

        Ok(writebacks)
    }

    /// Gets the current errno value.
    pub fn get_errno(&self) -> i32 {
        unsafe { *self.platform.errno_location() }
    }

    /// Sets the errno value.
    pub fn set_errno(&self, value: i32) {
        unsafe {
            *self.platform.errno_location() = value;
        }
    }

    /// Clears errno (sets to 0).
    pub fn clear_errno(&self) {
        self.set_errno(0);
    }

    // =========================================================================
    // Callback/Trampoline Support
    // =========================================================================

    /// Creates a callback trampoline that allows C code to call a Verum function.
    ///
    /// This uses libffi's closure mechanism to generate a C-callable function pointer
    /// that, when called, will invoke the specified Verum function.
    ///
    /// # Arguments
    ///
    /// * `return_type` - C return type for the callback
    /// * `arg_types` - C argument types for the callback
    /// * `fn_id` - The function ID to call when the callback is invoked
    ///
    /// # Returns
    ///
    /// A trampoline ID and raw function pointer that can be passed to C code.
    /// See [`TrampolineRegistry::fn_id_for_code_ptr`].
    pub fn callback_fn_id_for_code_ptr(&self, code_ptr: usize) -> Option<u32> {
        self.trampolines.fn_id_for_code_ptr(code_ptr)
    }

    /// Create an FFI callback trampoline for a Verum function.
    pub fn create_callback(
        &mut self,
        return_type: CTypeRuntime,
        arg_types: Vec<CTypeRuntime>,
        fn_id: u32,
    ) -> Result<(TrampolineId, *const ()), FfiError> {
        let id = self
            .trampolines
            .create_callback(return_type, arg_types, fn_id)
            .map_err(|e| FfiError::CallFailed(format!("Failed to create callback: {}", e)))?;

        let code_ptr = self.trampolines.get_code_ptr(id).ok_or_else(|| {
            FfiError::CallFailed("Failed to get callback code pointer".to_string())
        })?;

        Ok((id, code_ptr))
    }

    /// Creates a callback trampoline from a module FFI symbol signature.
    ///
    /// This version looks up the signature from the module's FFI symbol table,
    /// which is useful when the callback signature matches an FFI function.
    ///
    /// # Arguments
    ///
    /// * `module` - The VBC module containing FFI signatures
    /// * `fn_id` - The Verum function ID to call when invoked
    /// * `signature_idx` - Index into the module's FFI symbols table
    ///
    /// # Returns
    ///
    /// A raw function pointer that can be passed to C code.
    pub fn create_callback_from_symbol(
        &mut self,
        module: &VbcModule,
        fn_id: u32,
        signature_idx: u32,
    ) -> Result<*const (), FfiError> {
        // Look up the FFI symbol to get the signature
        let symbol = module
            .get_ffi_symbol(crate::module::FfiSymbolId(signature_idx))
            .ok_or(FfiError::SymbolNotFound(crate::module::FfiSymbolId(
                signature_idx,
            )))?;

        // Convert signature types to runtime types using From trait
        let return_type: CTypeRuntime = symbol.signature.return_type.into();
        let arg_types: Vec<CTypeRuntime> = symbol
            .signature
            .param_types
            .iter()
            .map(|ct| (*ct).into())
            .collect();

        let (_, code_ptr) = self.create_callback(return_type, arg_types, fn_id)?;
        Ok(code_ptr)
    }

    /// Frees a callback trampoline created by `create_callback`.
    pub fn free_callback(&mut self, id: TrampolineId) -> Result<(), FfiError> {
        self.trampolines
            .unregister_callback(id)
            .map_err(|e| FfiError::CallFailed(format!("Failed to free callback: {}", e)))
    }

    /// Sets the callback handler for the current thread.
    ///
    /// This must be called before any callbacks are invoked. The handler receives
    /// the function ID and arguments, and must return the result value.
    pub fn set_callback_handler(handler: CallbackHandler) {
        TrampolineRegistry::set_handler(handler);
    }

    /// Clears the callback handler for the current thread.
    pub fn clear_callback_handler() {
        TrampolineRegistry::clear_handler();
    }

    /// Looks up a TrampolineId by code pointer.
    ///
    /// Returns the TrampolineId if the code pointer corresponds to a registered callback.
    pub fn lookup_callback_by_ptr(&self, code_ptr: *const ()) -> Option<TrampolineId> {
        self.trampolines.lookup_by_code_ptr(code_ptr)
    }

    /// Tracks an array buffer for FFI marshalling.
    ///
    /// When arrays are marshalled from VBC Values to C data, we allocate temporary
    /// buffers. These must be:
    /// 1. Kept alive during the FFI call
    /// 2. Written back to the original array for mutable references
    /// 3. Freed after the FFI call completes
    ///
    /// # Arguments
    ///
    /// * `buffer` - Pointer to the marshalled C data buffer
    /// * `buffer_size` - Size of the buffer in bytes
    /// * `array_ptr` - Pointer to the original VBC array (for write-back)
    /// * `array_len` - Number of elements in the array
    /// * `element_type` - Type tag (0x01=i8, 0x02=i16, 0x03=i32, 0x04=i64, etc.)
    /// * `is_mutable` - If true, write back changes after FFI call
    pub fn track_array_buffer(
        &mut self,
        buffer: *mut u8,
        buffer_size: usize,
        array_ptr: *const u8,
        array_len: usize,
        element_type: u8,
        is_mutable: bool,
    ) {
        self.marshaller.track_array_buffer(ArrayBufferInfo {
            buffer,
            buffer_size,
            array_ptr,
            array_len,
            element_type,
            is_mutable,
        });
    }

    /// Cleans up array buffers, optionally writing back mutable ones.
    ///
    /// For mutable array references, this converts the C data back to VBC Values
    /// and writes them to the original array.
    ///
    /// # Safety
    ///
    /// The array_ptr must still be valid and the buffer must not have been freed.
    pub unsafe fn cleanup_array_buffers(&mut self) {
        // SAFETY: caller guarantees array_ptr is valid
        unsafe { self.marshaller.cleanup_array_buffers() };
    }
}

impl Default for FfiRuntime {
    fn default() -> Self {
        Self::new().expect("failed to create FFI runtime")
    }
}

/// Marshals a single field value from Verum to C format.
///
/// # Safety
///
/// The c_field_ptr must point to valid writable memory of the appropriate type.
unsafe fn marshal_field_to_c(
    field_value: Value,
    c_type: crate::module::CType,
    c_field_ptr: *mut u8,
) {
    // SAFETY: Caller guarantees c_field_ptr points to valid writable memory of the appropriate type.
    unsafe {
        match c_type {
            crate::module::CType::I8 => {
                *(c_field_ptr as *mut i8) = field_value.as_i64() as i8;
            }
            crate::module::CType::U8 | crate::module::CType::Bool => {
                *c_field_ptr = field_value.as_i64() as u8;
            }
            crate::module::CType::I16 => {
                *(c_field_ptr as *mut i16) = field_value.as_i64() as i16;
            }
            crate::module::CType::U16 => {
                *(c_field_ptr as *mut u16) = field_value.as_i64() as u16;
            }
            crate::module::CType::I32 => {
                *(c_field_ptr as *mut i32) = field_value.as_i64() as i32;
            }
            crate::module::CType::U32 => {
                *(c_field_ptr as *mut u32) = field_value.as_i64() as u32;
            }
            crate::module::CType::I64 | crate::module::CType::Ssize => {
                *(c_field_ptr as *mut i64) = field_value.as_i64();
            }
            crate::module::CType::U64 | crate::module::CType::Size => {
                *(c_field_ptr as *mut u64) = field_value.as_i64() as u64;
            }
            crate::module::CType::F32 => {
                *(c_field_ptr as *mut f32) = field_value.as_f64() as f32;
            }
            crate::module::CType::F64 => {
                *(c_field_ptr as *mut f64) = field_value.as_f64();
            }
            crate::module::CType::Ptr
            | crate::module::CType::CStr
            | crate::module::CType::StructPtr
            | crate::module::CType::ArrayPtr
            | crate::module::CType::FnPtr => {
                *(c_field_ptr as *mut *mut u8) = field_value.as_ptr::<u8>();
            }
            crate::module::CType::Void | crate::module::CType::StructValue => {}
        }
    }
}

/// Marshals a single field value from C to Verum format.
///
/// # Safety
///
/// The c_field_ptr must point to valid readable memory of the appropriate type.
pub(crate) unsafe fn marshal_field_from_c(
    c_type: crate::module::CType,
    c_field_ptr: *const u8,
) -> Option<Value> {
    // SAFETY: Caller guarantees c_field_ptr points to valid readable memory of the appropriate type.
    unsafe {
        match c_type {
            crate::module::CType::I8 => Some(Value::from_i64(*(c_field_ptr as *const i8) as i64)),
            crate::module::CType::U8 | crate::module::CType::Bool => {
                Some(Value::from_i64(*c_field_ptr as i64))
            }
            crate::module::CType::I16 => Some(Value::from_i64(*(c_field_ptr as *const i16) as i64)),
            crate::module::CType::U16 => Some(Value::from_i64(*(c_field_ptr as *const u16) as i64)),
            crate::module::CType::I32 => Some(Value::from_i64(*(c_field_ptr as *const i32) as i64)),
            crate::module::CType::U32 => Some(Value::from_i64(*(c_field_ptr as *const u32) as i64)),
            crate::module::CType::I64 | crate::module::CType::Ssize => {
                Some(Value::from_i64(*(c_field_ptr as *const i64)))
            }
            crate::module::CType::U64 | crate::module::CType::Size => {
                Some(Value::from_i64(*(c_field_ptr as *const u64) as i64))
            }
            crate::module::CType::F32 => Some(Value::from_f64(*(c_field_ptr as *const f32) as f64)),
            crate::module::CType::F64 => Some(Value::from_f64(*(c_field_ptr as *const f64))),
            crate::module::CType::Ptr
            | crate::module::CType::CStr
            | crate::module::CType::StructPtr
            | crate::module::CType::ArrayPtr
            | crate::module::CType::FnPtr => {
                Some(Value::from_ptr(*(c_field_ptr as *const *mut u8)))
            }
            crate::module::CType::Void | crate::module::CType::StructValue => None,
        }
    }
}

/// Marshals a Verum struct (heap object) to a C struct buffer.
///
/// # Safety
///
/// - obj_ptr must point to a valid Verum heap object
/// - struct_buffer must be large enough to hold the marshalled struct
unsafe fn marshal_verum_struct_to_c(
    layout: &crate::module::FfiStructLayout,
    layouts: &[crate::module::FfiStructLayout],
    obj_ptr: *const u8,
    struct_buffer: &mut [u8; 256],
) {
    // SAFETY: Caller guarantees obj_ptr points to a valid Verum heap object
    // and struct_buffer is large enough to hold the marshalled struct.
    unsafe {
        let base = struct_buffer.as_mut_ptr();
        marshal_verum_struct_to_c_at(layout, layouts, obj_ptr, base, struct_buffer.len(), 0, 0);
    }
}

/// How deep a chain of by-value structs may nest before marshalling gives
/// up.
///
/// The codegen side cannot emit a cycle — `generate_ffi_struct_layout_inner`
/// carries a recursion stack — but this function reads `nested_layout` out
/// of a `.vbc` module, which is an untrusted input (see
/// `tests/red_team_bytecode_trust_boundary.rs`). A hand-written archive can
/// point a layout at itself, and the depth cap is what keeps that a wrong
/// answer instead of a stack overflow.
const MAX_FFI_STRUCT_NESTING: u32 = 8;

/// Whether to narrate the nested-struct walk, cached so the env is read
/// once and not per field.
///
/// Every `continue` in the two `_at` functions below is a SILENT SKIP —
/// the exact shape that made this defect cost a shipped release: a
/// nested record that does not travel produces a plausible wrong value
/// (a file modified at the epoch), never an error. The walk cannot
/// return one, so the least it can do is say which condition refused
/// when asked. Same lever as the argument trace above.
fn ffi_struct_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VERUM_TRACE_FFI_ARG").is_ok())
}

/// Why a `StructValue` field did not recurse. Called only on the
/// refusing paths, so it costs nothing on the normal one.
fn trace_nested_skip(which: &str, slot: usize, depth: u32, reason: &str) {
    if ffi_struct_trace() {
        eprintln!("[ffi-struct] {which} slot={slot} depth={depth} SKIPPED: {reason}");
    }
}

/// How many DATA bytes the receiver actually has, or `None` when the
/// pointer is not a readable object header.
///
/// The two sides of the marshalling walk come from different places:
/// `layout.fields.len()` is compiled into the module, the object is
/// whatever the caller's register holds. Codegen builds both from one
/// declaration, so they agree in practice — but a `.vbc` module is
/// untrusted input (`tests/red_team_bytecode_trust_boundary.rs`), and a
/// layout with more fields than the receiver has slots is an
/// out-of-bounds READ on the to-C leg and an out-of-bounds WRITE coming
/// back. The object's own header says how big it is and was sitting at
/// `obj_ptr` unread until T1361.
///
/// `try_from_ptr` rather than a raw deref: it rejects null, misaligned,
/// and NaN-box special-value markers (a `FatRef` payload is 8-aligned
/// and points at unmapped memory), so a mis-dispatched value yields
/// `None` here instead of a fault.
fn receiver_data_bytes(obj_ptr: *const u8) -> Option<usize> {
    // SAFETY: `try_from_ptr` is the alignment- and marker-checked
    // accessor; it returns None rather than dereferencing anything it
    // cannot prove is a header.
    unsafe { crate::interpreter::ObjectHeader::try_from_ptr(obj_ptr).map(|h| h.size as usize) }
}

/// Copies one Verum record into the C buffer at `base_off`, recursing into
/// record-typed fields.
///
/// # Safety
///
/// - `obj_ptr` points to a live Verum heap object with at least
///   `layout.fields.len()` slots
/// - `buf` is writable for `buf_len` bytes
///
/// `pub` for one reason, stated so nobody widens it further: this is
/// raw-pointer code whose defect was invisible for the life of the
/// feature, and the only way to gate it directly is to call it. See
/// `tests/t1359_nested_struct_marshalling.rs`. Not part of the crate's
/// intended surface.
#[doc(hidden)]
pub unsafe fn marshal_verum_struct_to_c_at(
    layout: &crate::module::FfiStructLayout,
    layouts: &[crate::module::FfiStructLayout],
    obj_ptr: *const u8,
    buf: *mut u8,
    buf_len: usize,
    base_off: usize,
    depth: u32,
) {
    // SAFETY: see the function contract.
    unsafe {
        let data_bytes = receiver_data_bytes(obj_ptr);
        // The Verum-side slot index is the field's DECLARED POSITION (heap
        // objects store fields contiguously at HEADER + pos*sizeof(Value), the
        // same order `resolve_field_index` / GetF use). `field.name` is a
        // GLOBAL interned field id (see the NOTE on `intern_field_name` in
        // codegen) — NOT the per-type position — so it must never be used as
        // the slot index. `layout.fields` is built in declared order, so the
        // enumeration index is exactly the object slot; `field.offset` remains
        // the (independent) packed C-struct byte offset. (#32)
        for (slot, field) in layout.fields.iter().enumerate() {
            let Some(off) = base_off.checked_add(field.offset as usize) else {
                continue;
            };
            // The buffer is a fixed 256 bytes and `field.offset` is a u32
            // read from the module, so this bound is the only thing
            // standing between a malformed layout and a write past the
            // box. It was absent until T1359.
            if off.saturating_add(field.size as usize) > buf_len {
                continue;
            }

            // The RECEIVER's bound, which is a different question from the
            // buffer's above: that one asks whether the C side has room,
            // this one whether the Verum object has the slot at all.
            let slot_end = (slot + 1) * std::mem::size_of::<Value>();
            match data_bytes {
                None => {
                    trace_nested_skip("to_c", slot, depth, "receiver has no readable header");
                    continue;
                }
                Some(have) if slot_end > have => {
                    trace_nested_skip("to_c", slot, depth, "slot past the receiver's data area");
                    continue;
                }
                Some(_) => {}
            }

            let value_ptr = obj_ptr
                .add(crate::interpreter::OBJECT_HEADER_SIZE)
                .add(slot * std::mem::size_of::<Value>());
            let field_value = *(value_ptr as *const Value);

            // A record-typed field is a heap object of its own: the slot
            // holds a POINTER to it, not its bytes. C wants the bytes
            // inline, so follow the pointer and lay the nested record out
            // at this field's offset. Skipping it (the pre-T1359
            // behaviour, `CType::StructValue => {}`) is why a
            // `DarwinStat.st_mtime` reached `fstat` as sixteen zero bytes.
            if field.c_type == crate::module::CType::StructValue {
                if depth >= MAX_FFI_STRUCT_NESTING {
                    trace_nested_skip("to_c", slot, depth, "nesting depth cap");
                    continue;
                }
                match field.nested_layout.and_then(|i| layouts.get(i as usize)) {
                    None => trace_nested_skip("to_c", slot, depth, "no nested layout on the field"),
                    Some(nested) if !field_value.is_ptr() || field_value.is_nil() => {
                        let _ = nested;
                        trace_nested_skip("to_c", slot, depth, "slot holds no object");
                    }
                    Some(nested) => {
                        let nested_obj = field_value.as_ptr::<u8>();
                        if nested_obj.is_null() {
                            trace_nested_skip("to_c", slot, depth, "slot holds a null pointer");
                        } else {
                            marshal_verum_struct_to_c_at(
                                nested,
                                layouts,
                                nested_obj,
                                buf,
                                buf_len,
                                off,
                                depth + 1,
                            );
                        }
                    }
                }
                continue;
            }

            marshal_field_to_c(field_value, field.c_type, buf.add(off));
        }
    }
}

/// Marshals a C struct buffer back to a Verum struct (heap object).
///
/// # Safety
///
/// - obj_ptr must point to a valid writable Verum heap object
/// - struct_buffer must contain valid marshalled data
unsafe fn marshal_c_to_verum_struct(
    layout: &crate::module::FfiStructLayout,
    layouts: &[crate::module::FfiStructLayout],
    struct_buffer: &[u8; 256],
    obj_ptr: *mut u8,
) {
    // SAFETY: Caller guarantees obj_ptr points to a valid writable Verum heap object
    // and struct_buffer contains valid marshalled data.
    unsafe {
        marshal_c_to_verum_struct_at(
            layout,
            layouts,
            struct_buffer.as_ptr(),
            struct_buffer.len(),
            obj_ptr,
            0,
            0,
        );
    }
}

/// Reads one C struct at `base_off` back into a Verum record, recursing
/// into record-typed fields.
///
/// # Safety
///
/// - `obj_ptr` points to a live writable Verum heap object with at least
///   `layout.fields.len()` slots
/// - `buf` is readable for `buf_len` bytes
///
/// `pub` for the same single reason as its twin above — see that note.
#[doc(hidden)]
pub unsafe fn marshal_c_to_verum_struct_at(
    layout: &crate::module::FfiStructLayout,
    layouts: &[crate::module::FfiStructLayout],
    buf: *const u8,
    buf_len: usize,
    obj_ptr: *mut u8,
    base_off: usize,
    depth: u32,
) {
    // SAFETY: see the function contract.
    unsafe {
        let data_bytes = receiver_data_bytes(obj_ptr);
        // See `marshal_verum_struct_to_c`: the Verum-side slot is the field's
        // declared position (enumeration index), NOT the global interned
        // `field.name` id. `field.offset` is the packed C-struct byte offset
        // the kernel wrote through. (#32)
        for (slot, field) in layout.fields.iter().enumerate() {
            let Some(off) = base_off.checked_add(field.offset as usize) else {
                continue;
            };
            if off.saturating_add(field.size as usize) > buf_len {
                continue;
            }

            // The receiver's bound. This leg WRITES, so an unbounded walk
            // here is not a stray read but heap corruption.
            let slot_end = (slot + 1) * std::mem::size_of::<Value>();
            match data_bytes {
                None => {
                    trace_nested_skip("from_c", slot, depth, "receiver has no readable header");
                    continue;
                }
                Some(have) if slot_end > have => {
                    trace_nested_skip("from_c", slot, depth, "slot past the receiver's data area");
                    continue;
                }
                Some(_) => {}
            }

            let value_ptr = obj_ptr
                .add(crate::interpreter::OBJECT_HEADER_SIZE)
                .add(slot * std::mem::size_of::<Value>())
                as *mut Value;

            // The nested record already exists as a heap object — the
            // caller built it before the call — so write THROUGH the slot's
            // pointer rather than replacing the slot. Replacing it would
            // hand the caller a different object than the one they passed,
            // which is not what `&mut record` means on either side of the
            // boundary.
            if field.c_type == crate::module::CType::StructValue {
                if depth >= MAX_FFI_STRUCT_NESTING {
                    trace_nested_skip("from_c", slot, depth, "nesting depth cap");
                    continue;
                }
                let existing = *value_ptr;
                match field.nested_layout.and_then(|i| layouts.get(i as usize)) {
                    None => {
                        trace_nested_skip("from_c", slot, depth, "no nested layout on the field")
                    }
                    Some(nested) if !existing.is_ptr() || existing.is_nil() => {
                        let _ = nested;
                        trace_nested_skip("from_c", slot, depth, "slot holds no object");
                    }
                    Some(nested) => {
                        let nested_obj = existing.as_ptr::<u8>();
                        if nested_obj.is_null() {
                            trace_nested_skip("from_c", slot, depth, "slot holds a null pointer");
                        } else {
                            marshal_c_to_verum_struct_at(
                                nested,
                                layouts,
                                buf,
                                buf_len,
                                nested_obj,
                                off,
                                depth + 1,
                            );
                        }
                    }
                }
                continue;
            }

            if let Some(field_value) = marshal_field_from_c(field.c_type, buf.add(off)) {
                *value_ptr = field_value;
            }
        }
    }
}

/// Converts a CTypeRuntime to a libffi type pointer.
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the program as it
/// references static type descriptors.
///
/// # Panics
///
/// Panics if ctype is StructValue - use the struct type cache methods instead.
fn ctype_to_ffi_type(ctype: CTypeRuntime) -> *mut ffi_type {
    use std::ptr::addr_of_mut;

    // libffi type statics are mutable to allow internal bookkeeping.
    // We use addr_of_mut! to get raw pointers without creating mutable references.
    match ctype {
        CTypeRuntime::Void => addr_of_mut!(types::void),
        CTypeRuntime::I8 => addr_of_mut!(types::sint8),
        CTypeRuntime::I16 => addr_of_mut!(types::sint16),
        CTypeRuntime::I32 => addr_of_mut!(types::sint32),
        CTypeRuntime::I64 => addr_of_mut!(types::sint64),
        CTypeRuntime::U8 => addr_of_mut!(types::uint8),
        CTypeRuntime::U16 => addr_of_mut!(types::uint16),
        CTypeRuntime::U32 => addr_of_mut!(types::uint32),
        CTypeRuntime::U64 => addr_of_mut!(types::uint64),
        CTypeRuntime::F32 => addr_of_mut!(types::float),
        CTypeRuntime::F64 => addr_of_mut!(types::double),
        CTypeRuntime::Bool => addr_of_mut!(types::uint8), // C99 _Bool is typically 1 byte
        CTypeRuntime::Size => {
            if std::mem::size_of::<usize>() == 8 {
                addr_of_mut!(types::uint64)
            } else {
                addr_of_mut!(types::uint32)
            }
        }
        CTypeRuntime::Ssize => {
            if std::mem::size_of::<isize>() == 8 {
                addr_of_mut!(types::sint64)
            } else {
                addr_of_mut!(types::sint32)
            }
        }
        CTypeRuntime::Ptr
        | CTypeRuntime::CStr
        | CTypeRuntime::StructPtr(_)
        | CTypeRuntime::ArrayPtr
        | CTypeRuntime::FnPtr => addr_of_mut!(types::pointer),
        CTypeRuntime::StructValue(layout_idx) => {
            panic!(
                "StructValue({}) requires struct type cache - use get_or_create_struct_type instead",
                layout_idx
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffi_runtime_creation() {
        let runtime = FfiRuntime::new();
        assert!(runtime.is_ok());
    }

    #[test]
    fn test_errno_operations() {
        let runtime = FfiRuntime::new().unwrap();

        // Clear errno
        runtime.clear_errno();
        assert_eq!(runtime.get_errno(), 0);

        // Set errno
        runtime.set_errno(42);
        assert_eq!(runtime.get_errno(), 42);

        // Clear again
        runtime.clear_errno();
        assert_eq!(runtime.get_errno(), 0);
    }

    /// `System` is libSystem.B.dylib — an APPLE library name.
    ///
    /// This ran on every platform and failed on Linux, where there is no
    /// such library, contributing two of the fifteen red unit tests in
    /// CI run 32166197035. A test whose subject does not exist on the
    /// runner is not a failing test; it is a test that should not have
    /// been selected. (The no-libc architecture names libSystem as the
    /// macOS boundary precisely because Linux uses raw syscalls
    /// instead.)
    #[test]
    #[cfg(target_os = "macos")]
    fn test_load_libsystem() {
        let mut runtime = FfiRuntime::new().unwrap();
        let result = runtime.load_library("System");
        assert!(
            result.is_ok(),
            "failed to load libSystem: {:?}",
            result.err()
        );
    }

    /// Same subject, same reason: resolves `getpid` out of libSystem.
    #[test]
    #[cfg(target_os = "macos")]
    fn test_resolve_getpid() {
        let mut runtime = FfiRuntime::new().unwrap();
        let handle = runtime.load_library("System").unwrap();

        let symbol = runtime.resolve_symbol(handle, "getpid", CTypeRuntime::I32, vec![]);
        assert!(
            symbol.is_ok(),
            "failed to resolve getpid: {:?}",
            symbol.err()
        );
    }

    /// Same subject again: calls `getpid` through libSystem.
    #[test]
    #[cfg(target_os = "macos")]
    fn test_call_getpid() {
        let mut runtime = FfiRuntime::new().unwrap();
        let handle = runtime.load_library("System").unwrap();

        let symbol = runtime
            .resolve_symbol(handle, "getpid", CTypeRuntime::I32, vec![])
            .unwrap();

        let mut ret_value = Value::nil();
        unsafe {
            runtime.call_ffi_c(&symbol, &[], &mut ret_value).unwrap();
        }

        // Check that we got a valid pid (positive integer)
        assert!(ret_value.is_int(), "expected Int, got non-int value");
        let pid = ret_value.as_i64();
        assert!(pid > 0, "getpid returned invalid pid: {}", pid);
    }
}
