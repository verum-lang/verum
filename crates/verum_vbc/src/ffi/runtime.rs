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
    ffi_abi_FFI_DEFAULT_ABI, ffi_cif, ffi_type, types, CodePtr, prep_cif, call,
};

use super::marshal::{ArrayBufferInfo, MarshalError, Marshaller};
use super::platform::{create_platform, FfiPlatform, FfiPlatformError, LibraryHandle};
use super::trampolines::{TrampolineId, TrampolineRegistry, CallbackHandler};
use super::CTypeRuntime;
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
    ArgumentCountMismatch {
        expected: usize,
        got: usize,
    },
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
                write!(f, "argument count mismatch: expected {}, got {}", expected, got)
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
        let layout = module.ffi_layouts.get(layout_idx as usize)
            .ok_or_else(|| FfiError::CallFailed(format!("FFI struct layout {} not found", layout_idx)))?;

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
        Ok(self.struct_types.get_mut(&layout_idx).unwrap().as_ffi_type_ptr())
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
    pub fn load_module_libraries(&mut self, module: &VbcModule) -> Result<(), FfiError> {
        for (idx, lib) in module.ffi_libraries.iter().enumerate() {
            let idx = idx as u16;
            if self.libraries.contains_key(&idx) {
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
                let layout_idx = symbol.signature.param_layout_indices
                    .get(i)
                    .copied()
                    .flatten();
                CTypeRuntime::from_ctype_with_layout(*t, layout_idx)
            })
            .collect();

        // Resolve symbol name from string table
        let symbol_name = module.strings.get(symbol.name).unwrap_or("");

        // Resolve the symbol with struct type support
        let resolved = self.resolve_symbol_with_structs(
            module, handle, symbol_name, return_type, arg_types
        )?;

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
            let struct_size = self.struct_types.get(&layout_idx)
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
                        Some(std::mem::transmute::<*const std::ffi::c_void, unsafe extern "C" fn()>(code_ptr.as_ptr())),
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
            *ret_value = self.marshaller.c_to_value(ret_storage, symbol.return_type)?;
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
                    let layout_idx = symbol.signature.param_layout_indices.get(i).copied().flatten();
                    CTypeRuntime::from_ctype_with_layout(*t, layout_idx)
                })
                .collect();

            // Resolve symbol name from string table
            let symbol_name = module.strings.get(symbol.name).unwrap_or("");

            // Resolve the symbol (with struct support for struct-by-value)
            let resolved = self.resolve_symbol_with_structs(module, handle, symbol_name, return_type, arg_types)?;
            self.symbols.insert(idx, resolved);
        }

        // Now get the symbol and call it
        let symbol = self.symbols.get(&idx).unwrap();

        // Check argument count
        if args.len() != symbol.arg_types.len() {
            return Err(FfiError::ArgumentCountMismatch {
                expected: symbol.arg_types.len(),
                got: args.len(),
            });
        }

        // Clear any previous ref arg storage
        self.marshaller.clear_cache();

        // Marshal arguments with source register indices for write-back
        // For pointer types, use value_to_c_ref which allocates temp storage
        let mut raw_args: Vec<u64> = Vec::with_capacity(args.len());
        let mut arg_ptrs: Vec<*mut std::ffi::c_void> = Vec::with_capacity(args.len());
        // Storage for struct-by-value arguments (kept alive until after the call)
        let mut struct_arg_buffers: Vec<Box<[u8; 256]>> = Vec::new();
        // Track which arguments are struct-by-value (we need to handle them specially for arg_ptrs)
        let mut struct_arg_indices: Vec<(usize, usize)> = Vec::new(); // (raw_args index, struct_buffer index)
        // Track struct pointer arguments for write-back: (layout_idx, obj_ptr, buffer_idx)
        let mut struct_ptr_writebacks: Vec<(u16, *mut u8, usize)> = Vec::new();

        for (i, (arg, ctype)) in args.iter().zip(symbol.arg_types.iter()).enumerate() {
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
                        unsafe { marshal_verum_struct_to_c(layout, obj_ptr, &mut struct_buffer) };
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
                        unsafe { marshal_verum_struct_to_c(layout, obj_ptr, &mut struct_buffer) };
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
                let write_back_reg = if matches!(ctype, CTypeRuntime::Ptr | CTypeRuntime::ArrayPtr) {
                    // Look up the source register from the map
                    source_reg_map.get(&(i as u8)).copied()
                } else {
                    None
                };
                let raw = self.marshaller.value_to_c_ref(*arg, *ctype, write_back_reg)?;
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

        // Get symbol info for call
        let cif_ptr = symbol.cif.as_ref() as *const ffi_cif as *mut ffi_cif;
        let code_ptr = CodePtr::from_ptr(symbol.ptr as *const std::ffi::c_void);
        let return_type = symbol.return_type;

        // Handle struct-by-value returns specially
        if let CTypeRuntime::StructValue(layout_idx) = return_type {
            // Get struct size from the cached type
            let struct_size = self.struct_types.get(&layout_idx)
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
                        Some(std::mem::transmute::<*const std::ffi::c_void, unsafe extern "C" fn()>(code_ptr.as_ptr())),
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
                unsafe { marshal_c_to_verum_struct(layout, struct_buffer, *obj_ptr) };
            }
        }

        // Collect write-back values for mutable reference arguments
        // The write_back_reg now points to the ORIGINAL variable register
        let mut writebacks: Vec<(u16, Value)> = Vec::new();
        for storage in self.marshaller.ref_arg_storage() {
            if let Some(reg) = storage.write_back_reg {
                // Read the potentially modified value from storage
                let raw_value = storage.read();
                // Convert back to Value (as i64)
                let value = Value::from_i64(raw_value as i64);
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
    pub fn create_callback(
        &mut self,
        return_type: CTypeRuntime,
        arg_types: Vec<CTypeRuntime>,
        fn_id: u32,
    ) -> Result<(TrampolineId, *const ()), FfiError> {
        let id = self.trampolines.create_callback(return_type, arg_types, fn_id)
            .map_err(|e| FfiError::CallFailed(format!("Failed to create callback: {}", e)))?;

        let code_ptr = self.trampolines.get_code_ptr(id)
            .ok_or_else(|| FfiError::CallFailed("Failed to get callback code pointer".to_string()))?;

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
        let symbol = module.get_ffi_symbol(crate::module::FfiSymbolId(signature_idx))
            .ok_or(FfiError::SymbolNotFound(crate::module::FfiSymbolId(signature_idx)))?;

        // Convert signature types to runtime types using From trait
        let return_type: CTypeRuntime = symbol.signature.return_type.into();
        let arg_types: Vec<CTypeRuntime> = symbol.signature.param_types
            .iter()
            .map(|ct| (*ct).into())
            .collect();

        let (_, code_ptr) = self.create_callback(return_type, arg_types, fn_id)?;
        Ok(code_ptr)
    }

    /// Frees a callback trampoline created by `create_callback`.
    pub fn free_callback(&mut self, id: TrampolineId) -> Result<(), FfiError> {
        self.trampolines.unregister_callback(id)
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
unsafe fn marshal_field_to_c(field_value: Value, c_type: crate::module::CType, c_field_ptr: *mut u8) {
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
pub(crate) unsafe fn marshal_field_from_c(c_type: crate::module::CType, c_field_ptr: *const u8) -> Option<Value> {
    // SAFETY: Caller guarantees c_field_ptr points to valid readable memory of the appropriate type.
    unsafe {
        match c_type {
            crate::module::CType::I8 => {
                Some(Value::from_i64(*(c_field_ptr as *const i8) as i64))
            }
            crate::module::CType::U8 | crate::module::CType::Bool => {
                Some(Value::from_i64(*c_field_ptr as i64))
            }
            crate::module::CType::I16 => {
                Some(Value::from_i64(*(c_field_ptr as *const i16) as i64))
            }
            crate::module::CType::U16 => {
                Some(Value::from_i64(*(c_field_ptr as *const u16) as i64))
            }
            crate::module::CType::I32 => {
                Some(Value::from_i64(*(c_field_ptr as *const i32) as i64))
            }
            crate::module::CType::U32 => {
                Some(Value::from_i64(*(c_field_ptr as *const u32) as i64))
            }
            crate::module::CType::I64 | crate::module::CType::Ssize => {
                Some(Value::from_i64(*(c_field_ptr as *const i64)))
            }
            crate::module::CType::U64 | crate::module::CType::Size => {
                Some(Value::from_i64(*(c_field_ptr as *const u64) as i64))
            }
            crate::module::CType::F32 => {
                Some(Value::from_f64(*(c_field_ptr as *const f32) as f64))
            }
            crate::module::CType::F64 => {
                Some(Value::from_f64(*(c_field_ptr as *const f64)))
            }
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
    obj_ptr: *const u8,
    struct_buffer: &mut [u8; 256],
) {
    // SAFETY: Caller guarantees obj_ptr points to a valid Verum heap object
    // and struct_buffer is large enough to hold the marshalled struct.
    unsafe {
        for field in layout.fields.iter() {
            let string_id = field.name.0 as usize;
            let value_ptr = obj_ptr
                .add(crate::interpreter::OBJECT_HEADER_SIZE)
                .add(string_id * std::mem::size_of::<Value>());
            let field_value = *(value_ptr as *const Value);

            let c_field_ptr = struct_buffer.as_mut_ptr().add(field.offset as usize);
            marshal_field_to_c(field_value, field.c_type, c_field_ptr);
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
    struct_buffer: &[u8; 256],
    obj_ptr: *mut u8,
) {
    // SAFETY: Caller guarantees obj_ptr points to a valid writable Verum heap object
    // and struct_buffer contains valid marshalled data.
    unsafe {
        for field in layout.fields.iter() {
            let string_id = field.name.0 as usize;
            let c_field_ptr = struct_buffer.as_ptr().add(field.offset as usize);
            let value_ptr = obj_ptr
                .add(crate::interpreter::OBJECT_HEADER_SIZE)
                .add(string_id * std::mem::size_of::<Value>()) as *mut Value;

            if let Some(field_value) = marshal_field_from_c(field.c_type, c_field_ptr) {
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

    #[test]
    fn test_load_libsystem() {
        let mut runtime = FfiRuntime::new().unwrap();
        let result = runtime.load_library("System");
        assert!(result.is_ok(), "failed to load libSystem: {:?}", result.err());
    }

    #[test]
    fn test_resolve_getpid() {
        let mut runtime = FfiRuntime::new().unwrap();
        let handle = runtime.load_library("System").unwrap();

        let symbol = runtime.resolve_symbol(
            handle,
            "getpid",
            CTypeRuntime::I32,
            vec![],
        );
        assert!(symbol.is_ok(), "failed to resolve getpid: {:?}", symbol.err());
    }

    #[test]
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
