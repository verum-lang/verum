//! FFI callback trampolines.
//!
//! This module provides support for creating callback trampolines that allow
//! C code to call back into Verum functions. It uses libffi's closure mechanism
//! to generate C-callable function pointers.
//!
//! # Overview
//!
//! When C code needs to call a Verum function (e.g., qsort comparator),
//! we generate a trampoline using libffi closures that:
//! 1. Marshals C arguments to Verum values
//! 2. Invokes the Verum function through a registered callback handler
//! 3. Marshals the result back to C
//!
//! # Architecture
//!
//! The callback system uses a two-tier approach:
//! 1. `TrampolineRegistry` - manages callback metadata and lifetimes
//! 2. `CallbackContext` - holds per-callback state passed through libffi's userdata
//!
//! The actual function invocation happens through a registered handler function
//! that the interpreter provides when setting up callbacks.
//!
//! # Thread Safety
//!
//! Callbacks are associated with the thread that created them. A thread-local
//! handler is used to route callbacks to the correct interpreter instance.
//!
//! # Example
//!
//! ```ignore
//! // Create a callback for a comparator function: fn(i32, i32) -> i32
//! let callback_id = registry.create_callback(
//!     CTypeRuntime::I32,
//!     vec![CTypeRuntime::I32, CTypeRuntime::I32],
//!     42, // function ID in VBC module
//! )?;
//!
//! // Get the C function pointer
//! let fn_ptr = registry.get_code_ptr(callback_id)?;
//!
//! // Pass fn_ptr to C code (e.g., qsort)
//! // When C calls fn_ptr, our handler is invoked
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

use libffi::low::ffi_cif;
use libffi::middle::{Cif, Type};

use super::platform::FfiPlatformError;
use super::CTypeRuntime;
use crate::value::Value;

/// Unique identifier for a callback trampoline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrampolineId(pub u64);

/// Global counter for trampoline IDs.
static NEXT_TRAMPOLINE_ID: AtomicU64 = AtomicU64::new(1);

/// Type alias for the callback handler function.
///
/// The handler receives:
/// - `function_id`: The VBC function to invoke
/// - `args`: Marshalled arguments as Verum values
///
/// Returns the result value, or None if the function doesn't return.
pub type CallbackHandler = Box<dyn Fn(u32, &[Value]) -> Option<Value> + Send + Sync>;

/// Context stored with each callback closure.
///
/// This is passed through libffi's userdata mechanism and contains
/// all information needed to handle the callback.
pub struct CallbackContext {
    /// Unique ID for this callback.
    pub id: TrampolineId,
    /// VBC function ID to invoke.
    pub function_id: u32,
    /// Return type.
    pub return_type: CTypeRuntime,
    /// Argument types.
    pub arg_types: Vec<CTypeRuntime>,
}

// SAFETY: CallbackContext is thread-safe once created.
unsafe impl Send for CallbackContext {}
unsafe impl Sync for CallbackContext {}

/// A live callback with its closure and context.
///
/// SAFETY: All heap-allocated fields use `Pin<Box<>>` to guarantee stable
/// addresses. The libffi closure holds raw pointers to `context` and `cif`,
/// so these must never be moved. `Pin` enforces this at the type level,
/// preventing accidental moves even when the HashMap reallocates its entries.
/// LiveCallback owns the resources backing an FFI callback.  Several
/// fields exist solely to extend the lifetime of memory that the
/// libffi closure (holding raw pointers) references — they're
/// "alive-by-Drop": removing one would dangle the closure's pointers.
/// The `_`-prefixed names mark them as side-effectful holdings rather
/// than directly-readable state, which keeps the dead_code lint happy
/// without `#[allow]` and signals intent at every reference site.
struct LiveCallback {
    /// Pinned context (stable address for userdata - libffi closure holds a raw pointer).
    context: Pin<Box<CallbackContext>>,
    /// Code pointer for C calls (raw pointer to executable memory).
    code_ptr: *const (),
    /// The libffi CIF — must outlive the closure (SAFETY: pinned so its
    /// address is stable after `ffi_prep_closure_loc`; the closure
    /// holds a raw pointer to this CIF).  Read by Drop only.
    _cif: Pin<Box<Cif>>,
    /// Raw closure memory — must be kept alive.  Read by Drop only.
    _closure_mem: Box<ClosureMemory>,
}

/// Memory storage for closure - keeps libffi closure data alive.
struct ClosureMemory {
    /// Raw closure pointer from libffi low-level API.
    closure_ptr: *mut libffi::raw::ffi_closure,
    /// Code pointer — held alive for the lifetime of the closure;
    /// libffi may reference it during invocations.
    _code_ptr: *mut c_void,
}

impl Drop for ClosureMemory {
    fn drop(&mut self) {
        if !self.closure_ptr.is_null() {
            unsafe {
                libffi::raw::ffi_closure_free(self.closure_ptr as *mut c_void);
            }
        }
    }
}

// Thread-local callback handler.
//
// The interpreter registers a handler when setting up callbacks.
// This handler is invoked when C code calls back through a trampoline.
thread_local! {
    static CALLBACK_HANDLER: RefCell<Option<CallbackHandler>> = const { RefCell::new(None) };
}

/// Registry for callback trampolines.
///
/// Manages the lifecycle of callbacks, including:
/// - Creating closures with libffi
/// - Storing context data
/// - Providing code pointers for C
/// - Cleaning up when callbacks are no longer needed
pub struct TrampolineRegistry {
    /// Live callbacks by ID.
    callbacks: HashMap<TrampolineId, LiveCallback>,
    /// Mapping from code pointers to IDs (for reverse lookup).
    code_ptr_to_id: HashMap<usize, TrampolineId>,
}

impl TrampolineRegistry {
    /// Creates a new trampoline registry.
    pub fn new() -> Self {
        Self {
            callbacks: HashMap::new(),
            code_ptr_to_id: HashMap::new(),
        }
    }

    /// Sets the callback handler for the current thread.
    ///
    /// This must be called before any callbacks are invoked.
    /// The handler is responsible for invoking VBC functions.
    pub fn set_handler(handler: CallbackHandler) {
        CALLBACK_HANDLER.with(|h| {
            *h.borrow_mut() = Some(handler);
        });
    }

    /// Clears the callback handler for the current thread.
    pub fn clear_handler() {
        CALLBACK_HANDLER.with(|h| {
            *h.borrow_mut() = None;
        });
    }

    /// Creates a new callback trampoline.
    ///
    /// # Arguments
    ///
    /// * `return_type` - The C return type
    /// * `arg_types` - The C argument types
    /// * `function_id` - The VBC function ID to invoke
    ///
    /// # Returns
    ///
    /// The trampoline ID on success.
    pub fn create_callback(
        &mut self,
        return_type: CTypeRuntime,
        arg_types: Vec<CTypeRuntime>,
        function_id: u32,
    ) -> Result<TrampolineId, FfiPlatformError> {
        // Reject struct-by-value parameters / return type *up front* so
        // an unsupported signature surfaces at trampoline-creation time
        // (i.e. when the callback is registered, before any C code can
        // dispatch through it) rather than panicking deep inside
        // `marshal_c_to_value` / `marshal_value_to_c` / `size_of_ctype` /
        // `ctype_to_ffi_type` mid-call. The four downstream call sites
        // still preserve the panic as a defence-in-depth guard, but the
        // strict-mode contract is: by the time we reach those, every
        // CTypeRuntime in this signature is StructValue-free.
        if matches!(return_type, CTypeRuntime::StructValue(_)) {
            return Err(FfiPlatformError::StructValueInCallback {
                position: "return type".to_string(),
            });
        }
        for (idx, arg) in arg_types.iter().enumerate() {
            if matches!(arg, CTypeRuntime::StructValue(_)) {
                return Err(FfiPlatformError::StructValueInCallback {
                    position: format!("argument {}", idx + 1),
                });
            }
        }

        let id = TrampolineId(NEXT_TRAMPOLINE_ID.fetch_add(1, Ordering::SeqCst));

        // Create the context - Pin<Box<>> ensures stable address for libffi userdata pointer
        let context = Box::pin(CallbackContext {
            id,
            function_id,
            return_type,
            arg_types: arg_types.clone(),
        });

        // Convert types to libffi middle-layer types
        let ret_type = ctype_to_middle_type(return_type);
        let param_types: Vec<Type> = arg_types.iter().map(|t| ctype_to_middle_type(*t)).collect();

        // Create the CIF - MUST be pinned to ensure stable address!
        // SAFETY: The closure holds a pointer to the CIF, so it must not move.
        let cif = Box::pin(Cif::new(param_types, ret_type));

        // Allocate the closure using low-level API for more control
        let (closure_ptr, code_ptr) = unsafe {
            let mut code_ptr: *mut c_void = std::ptr::null_mut();
            let closure_ptr = libffi::raw::ffi_closure_alloc(
                std::mem::size_of::<libffi::raw::ffi_closure>(),
                &mut code_ptr,
            ) as *mut libffi::raw::ffi_closure;

            if closure_ptr.is_null() {
                return Err(FfiPlatformError::AllocationFailed {
                    size: std::mem::size_of::<libffi::raw::ffi_closure>(),
                    reason: "ffi_closure_alloc failed".to_string(),
                });
            }

            (closure_ptr, code_ptr)
        };

        // Get CIF pointer AFTER boxing to ensure it's from the heap location
        let cif_ptr = cif.as_raw_ptr();

        // Prepare the closure
        // SAFETY: Pin<Box<T>> guarantees the heap allocation won't move.
        // We extract a raw pointer for ffi_prep_closure_loc userdata, then
        // recover the Pin<Box<>> without moving the allocation.
        let ctx_ptr = unsafe { Pin::into_inner_unchecked(context) };
        let ctx_raw = Box::into_raw(ctx_ptr);
        unsafe {
            let result = libffi::raw::ffi_prep_closure_loc(
                closure_ptr,
                cif_ptr,
                Some(trampoline_handler),
                ctx_raw as *mut c_void,
                code_ptr,
            );

            if result != libffi::raw::ffi_status_FFI_OK {
                // Clean up on failure
                let _ = Box::from_raw(ctx_raw);
                libffi::raw::ffi_closure_free(closure_ptr as *mut c_void);
                return Err(FfiPlatformError::AllocationFailed {
                    size: 0,
                    reason: "ffi_prep_closure_loc failed".to_string(),
                });
            }
        }

        // SAFETY: Recover the pinned context box - the heap address has not changed.
        let context = unsafe { Pin::new_unchecked(Box::from_raw(ctx_raw)) };

        // Store closure memory for cleanup
        let closure_mem = Box::new(ClosureMemory {
            closure_ptr,
            _code_ptr: code_ptr,
        });

        let code_ptr_value = code_ptr as usize;

        self.callbacks.insert(
            id,
            LiveCallback {
                context,
                code_ptr: code_ptr as *const (),
                _cif: cif,
                _closure_mem: closure_mem,
            },
        );
        self.code_ptr_to_id.insert(code_ptr_value, id);

        Ok(id)
    }

    /// Gets the code pointer for a callback.
    ///
    /// This pointer can be passed to C code as a function pointer.
    pub fn get_code_ptr(&self, id: TrampolineId) -> Option<*const ()> {
        self.callbacks.get(&id).map(|cb| cb.code_ptr)
    }

    /// Unregisters a callback and frees its resources.
    pub fn unregister_callback(&mut self, id: TrampolineId) -> Result<(), FfiPlatformError> {
        if let Some(callback) = self.callbacks.remove(&id) {
            let code_ptr_value = callback.code_ptr as usize;
            self.code_ptr_to_id.remove(&code_ptr_value);
            // The closure_mem and context are dropped automatically
            Ok(())
        } else {
            Err(FfiPlatformError::PlatformError {
                code: 0,
                message: format!("callback {} not found", id.0),
            })
        }
    }

    /// Gets information about a registered callback.
    pub fn get_callback_info(&self, id: TrampolineId) -> Option<CallbackInfo> {
        self.callbacks.get(&id).map(|cb| CallbackInfo {
            id: cb.context.id,
            function_id: cb.context.function_id,
            return_type: cb.context.return_type,
            arg_types: cb.context.arg_types.clone(),
            code_ptr: Some(cb.code_ptr),
        })
    }

    /// Returns the number of registered callbacks.
    pub fn len(&self) -> usize {
        self.callbacks.len()
    }

    /// Returns true if no callbacks are registered.
    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }

    /// Looks up a TrampolineId by its code pointer.
    ///
    /// This is useful when you need to free a callback and only have
    /// the code pointer (e.g., stored in a Value).
    pub fn lookup_by_code_ptr(&self, code_ptr: *const ()) -> Option<TrampolineId> {
        self.code_ptr_to_id.get(&(code_ptr as usize)).copied()
    }
}

impl Default for TrampolineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Public information about a callback (no internal pointers exposed).
#[derive(Debug)]
pub struct CallbackInfo {
    /// Unique ID for this callback.
    pub id: TrampolineId,
    /// VBC function ID.
    pub function_id: u32,
    /// Return type.
    pub return_type: CTypeRuntime,
    /// Argument types.
    pub arg_types: Vec<CTypeRuntime>,
    /// Code pointer (opaque, for passing to C).
    pub code_ptr: Option<*const ()>,
}

// SAFETY: CallbackInfo is safe to send/share.
unsafe impl Send for CallbackInfo {}
unsafe impl Sync for CallbackInfo {}

/// The trampoline handler called by libffi when C code invokes a callback.
///
/// This function:
/// 1. Extracts callback context from userdata
/// 2. Marshals C arguments to Verum values
/// 3. Invokes the registered handler
/// 4. Marshals the result back to C
///
/// # Safety
///
/// This is called by libffi's closure mechanism. The userdata pointer
/// must be a valid pointer to a CallbackContext.
unsafe extern "C" fn trampoline_handler(
    _cif: *mut ffi_cif,
    result: *mut c_void,
    args: *mut *mut c_void,
    userdata: *mut c_void,
) {
    // All operations in this unsafe extern fn need explicit unsafe blocks
    unsafe {
        // Recover the context
        let context = &*(userdata as *const CallbackContext);

        // Marshal arguments from C to Verum values
        let mut verum_args = Vec::with_capacity(context.arg_types.len());
        for (i, arg_type) in context.arg_types.iter().enumerate() {
            let arg_ptr = *args.add(i);
            let value = marshal_c_to_value(arg_ptr, *arg_type);
            verum_args.push(value);
        }

        // Invoke the handler
        let return_value = CALLBACK_HANDLER.with(|h| {
            if let Some(ref handler) = *h.borrow() {
                handler(context.function_id, &verum_args)
            } else {
                // No handler registered - this is a programming error
                // Return a default value to avoid crashing
                None
            }
        });

        // Marshal result back to C
        if let Some(value) = return_value {
            marshal_value_to_c(value, context.return_type, result);
        } else if context.return_type != CTypeRuntime::Void {
            // No return value but expected one - write zero
            std::ptr::write_bytes(result as *mut u8, 0, size_of_ctype(context.return_type));
        }
    }
}

/// Marshal a C value to a Verum value.
///
/// # Safety
///
/// The `ptr` must point to valid data of the specified `ctype`.
unsafe fn marshal_c_to_value(ptr: *mut c_void, ctype: CTypeRuntime) -> Value {
    // SAFETY: All operations below are covered by the function's safety contract
    // requiring `ptr` to point to valid data of the specified `ctype`.
    unsafe {
        match ctype {
            CTypeRuntime::Void => Value::unit(),
            CTypeRuntime::I8 => Value::from_i64(*(ptr as *const i8) as i64),
            CTypeRuntime::I16 => Value::from_i64(*(ptr as *const i16) as i64),
            CTypeRuntime::I32 => Value::from_i64(*(ptr as *const i32) as i64),
            CTypeRuntime::I64 => Value::from_i64(*(ptr as *const i64)),
            CTypeRuntime::U8 => Value::from_i64(*(ptr as *const u8) as i64),
            CTypeRuntime::U16 => Value::from_i64(*(ptr as *const u16) as i64),
            CTypeRuntime::U32 => Value::from_i64(*(ptr as *const u32) as i64),
            CTypeRuntime::U64 => Value::from_i64(*(ptr as *const u64) as i64),
            CTypeRuntime::F32 => Value::from_f64(*(ptr as *const f32) as f64),
            CTypeRuntime::F64 => Value::from_f64(*(ptr as *const f64)),
            CTypeRuntime::Bool => Value::from_bool(*(ptr as *const u8) != 0),
            CTypeRuntime::Size => Value::from_i64(*(ptr as *const usize) as i64),
            CTypeRuntime::Ssize => Value::from_i64(*(ptr as *const isize) as i64),
            CTypeRuntime::Ptr
            | CTypeRuntime::CStr
            | CTypeRuntime::StructPtr(_)
            | CTypeRuntime::ArrayPtr
            | CTypeRuntime::FnPtr => Value::from_ptr(*(ptr as *const *mut u8)),
            CTypeRuntime::StructValue(_) => {
                // Struct-by-value in callbacks not yet supported
                panic!("StructValue not supported in callbacks - use StructPtr instead")
            }
        }
    }
}

/// Marshal a Verum value to a C result buffer.
///
/// # Safety
///
/// The `result_ptr` must have enough space for the C type.
unsafe fn marshal_value_to_c(value: Value, ctype: CTypeRuntime, result_ptr: *mut c_void) {
    // SAFETY: All operations below are covered by the function's safety contract
    // requiring `result_ptr` to have enough space for the C type.
    unsafe {
        match ctype {
            CTypeRuntime::Void => {}
            CTypeRuntime::I8 => *(result_ptr as *mut i8) = value.as_i64() as i8,
            CTypeRuntime::I16 => *(result_ptr as *mut i16) = value.as_i64() as i16,
            CTypeRuntime::I32 => *(result_ptr as *mut i32) = value.as_i64() as i32,
            CTypeRuntime::I64 => *(result_ptr as *mut i64) = value.as_i64(),
            CTypeRuntime::U8 => *(result_ptr as *mut u8) = value.as_i64() as u8,
            CTypeRuntime::U16 => *(result_ptr as *mut u16) = value.as_i64() as u16,
            CTypeRuntime::U32 => *(result_ptr as *mut u32) = value.as_i64() as u32,
            CTypeRuntime::U64 => *(result_ptr as *mut u64) = value.as_i64() as u64,
            CTypeRuntime::F32 => *(result_ptr as *mut f32) = value.as_f64() as f32,
            CTypeRuntime::F64 => *(result_ptr as *mut f64) = value.as_f64(),
            CTypeRuntime::Bool => *(result_ptr as *mut u8) = if value.as_bool() { 1 } else { 0 },
            CTypeRuntime::Size => *(result_ptr as *mut usize) = value.as_i64() as usize,
            CTypeRuntime::Ssize => *(result_ptr as *mut isize) = value.as_i64() as isize,
            CTypeRuntime::Ptr
            | CTypeRuntime::CStr
            | CTypeRuntime::StructPtr(_)
            | CTypeRuntime::ArrayPtr
            | CTypeRuntime::FnPtr => {
                *(result_ptr as *mut *mut u8) = value.as_ptr::<u8>()
            }
            CTypeRuntime::StructValue(_) => {
                // Struct-by-value in callbacks not yet supported
                panic!("StructValue not supported in callbacks - use StructPtr instead")
            }
        }
    }
}

/// Get the size of a C type.
fn size_of_ctype(ctype: CTypeRuntime) -> usize {
    match ctype {
        CTypeRuntime::Void => 0,
        CTypeRuntime::I8 | CTypeRuntime::U8 | CTypeRuntime::Bool => 1,
        CTypeRuntime::I16 | CTypeRuntime::U16 => 2,
        CTypeRuntime::I32 | CTypeRuntime::U32 | CTypeRuntime::F32 => 4,
        CTypeRuntime::I64
        | CTypeRuntime::U64
        | CTypeRuntime::F64
        | CTypeRuntime::Size
        | CTypeRuntime::Ssize
        | CTypeRuntime::Ptr
        | CTypeRuntime::CStr
        | CTypeRuntime::StructPtr(_)
        | CTypeRuntime::ArrayPtr
        | CTypeRuntime::FnPtr => 8,
        // Struct size depends on layout - callbacks with struct-by-value not supported yet
        CTypeRuntime::StructValue(_) => panic!("StructValue not supported in callbacks"),
    }
}

/// Converts a CTypeRuntime to a libffi middle-layer Type.
fn ctype_to_middle_type(ctype: CTypeRuntime) -> Type {
    match ctype {
        CTypeRuntime::Void => Type::void(),
        CTypeRuntime::I8 => Type::i8(),
        CTypeRuntime::I16 => Type::i16(),
        CTypeRuntime::I32 => Type::i32(),
        CTypeRuntime::I64 => Type::i64(),
        CTypeRuntime::U8 => Type::u8(),
        CTypeRuntime::U16 => Type::u16(),
        CTypeRuntime::U32 => Type::u32(),
        CTypeRuntime::U64 => Type::u64(),
        CTypeRuntime::F32 => Type::f32(),
        CTypeRuntime::F64 => Type::f64(),
        CTypeRuntime::Bool => Type::u8(),
        CTypeRuntime::Size => {
            if std::mem::size_of::<usize>() == 8 {
                Type::u64()
            } else {
                Type::u32()
            }
        }
        CTypeRuntime::Ssize => {
            if std::mem::size_of::<isize>() == 8 {
                Type::i64()
            } else {
                Type::i32()
            }
        }
        CTypeRuntime::Ptr
        | CTypeRuntime::CStr
        | CTypeRuntime::StructPtr(_)
        | CTypeRuntime::ArrayPtr
        | CTypeRuntime::FnPtr => Type::pointer(),
        // Struct type creation requires layout - callbacks with struct-by-value not supported yet
        CTypeRuntime::StructValue(_) => panic!("StructValue not supported in callbacks"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_callback() {
        let mut registry = TrampolineRegistry::new();

        let id = registry
            .create_callback(CTypeRuntime::I32, vec![CTypeRuntime::I32, CTypeRuntime::I32], 42)
            .unwrap();

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);

        let info = registry.get_callback_info(id).unwrap();
        assert_eq!(info.return_type, CTypeRuntime::I32);
        assert_eq!(info.arg_types.len(), 2);
        assert_eq!(info.function_id, 42);
        assert!(info.code_ptr.is_some());
    }

    #[test]
    fn test_unregister_callback() {
        let mut registry = TrampolineRegistry::new();

        let id = registry
            .create_callback(CTypeRuntime::Void, vec![], 0)
            .unwrap();

        assert_eq!(registry.len(), 1);

        registry.unregister_callback(id).unwrap();

        assert!(registry.is_empty());
        assert!(registry.get_callback_info(id).is_none());
    }

    #[test]
    fn test_trampoline_ids_unique() {
        let mut registry = TrampolineRegistry::new();

        let id1 = registry
            .create_callback(CTypeRuntime::Void, vec![], 1)
            .unwrap();
        let id2 = registry
            .create_callback(CTypeRuntime::Void, vec![], 2)
            .unwrap();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_code_ptr_retrieval() {
        let mut registry = TrampolineRegistry::new();

        let id = registry
            .create_callback(CTypeRuntime::I32, vec![CTypeRuntime::I32], 1)
            .unwrap();

        let ptr = registry.get_code_ptr(id);
        assert!(ptr.is_some());
        assert!(!ptr.unwrap().is_null());
    }

    #[test]
    fn test_callback_invocation_with_handler() {
        let mut registry = TrampolineRegistry::new();

        // Create a callback: fn(i32, i32) -> i32
        let id = registry
            .create_callback(
                CTypeRuntime::I32,
                vec![CTypeRuntime::I32, CTypeRuntime::I32],
                100,
            )
            .unwrap();

        // Set up a handler that adds the two arguments
        TrampolineRegistry::set_handler(Box::new(|fn_id, args| {
            assert_eq!(fn_id, 100);
            assert_eq!(args.len(), 2);
            let a = args[0].as_i64();
            let b = args[1].as_i64();
            Some(Value::from_i64(a + b))
        }));

        // Get the code pointer
        let code_ptr = registry.get_code_ptr(id).unwrap();

        // Call the trampoline as a C function
        let fn_ptr: extern "C" fn(i32, i32) -> i32 =
            unsafe { std::mem::transmute(code_ptr) };

        let result = fn_ptr(10, 32);
        assert_eq!(result, 42);

        // Clean up
        TrampolineRegistry::clear_handler();
    }

    #[test]
    fn rejects_struct_value_in_callback_argument() {
        // Pass-by-value structs in C callbacks need a concrete layout that
        // the libffi closure path doesn't have at trampoline-creation time.
        // The strict-mode contract is: reject at registration, not panic
        // mid-call.
        let mut registry = TrampolineRegistry::new();
        let result = registry.create_callback(
            CTypeRuntime::I32,
            vec![CTypeRuntime::I32, CTypeRuntime::StructValue(7), CTypeRuntime::I32],
            42,
        );
        match result {
            Err(FfiPlatformError::StructValueInCallback { position }) => {
                assert!(
                    position.contains("argument 2"),
                    "expected diagnostic to point at argument 2, got `{}`",
                    position
                );
            }
            other => panic!("expected StructValueInCallback, got {:?}", other),
        }
        assert_eq!(registry.len(), 0, "no callback should have been registered");
    }

    #[test]
    fn rejects_struct_value_in_callback_return_type() {
        let mut registry = TrampolineRegistry::new();
        let result = registry.create_callback(
            CTypeRuntime::StructValue(3),
            vec![CTypeRuntime::I32],
            42,
        );
        match result {
            Err(FfiPlatformError::StructValueInCallback { position }) => {
                assert_eq!(
                    position, "return type",
                    "expected diagnostic to name the return type"
                );
            }
            other => panic!("expected StructValueInCallback, got {:?}", other),
        }
    }

    #[test]
    fn struct_ptr_in_callback_remains_supported() {
        // The negative tests above must not regress the canonical
        // pass-by-pointer path — that's the recommended workaround.
        let mut registry = TrampolineRegistry::new();
        let id = registry
            .create_callback(
                CTypeRuntime::Void,
                vec![CTypeRuntime::StructPtr(11), CTypeRuntime::I32],
                7,
            )
            .expect("StructPtr is the canonical struct-callback ABI");
        assert_eq!(registry.len(), 1);
        let _ = id;
    }

    #[test]
    fn test_void_callback() {
        let mut registry = TrampolineRegistry::new();

        // Create a void callback
        let id = registry
            .create_callback(CTypeRuntime::Void, vec![CTypeRuntime::I32], 200)
            .unwrap();

        // Track if callback was invoked
        use std::sync::atomic::{AtomicBool, Ordering};
        static CALLED: AtomicBool = AtomicBool::new(false);

        TrampolineRegistry::set_handler(Box::new(|fn_id, args| {
            assert_eq!(fn_id, 200);
            assert_eq!(args.len(), 1);
            assert_eq!(args[0].as_i64(), 42);
            CALLED.store(true, Ordering::SeqCst);
            None
        }));

        let code_ptr = registry.get_code_ptr(id).unwrap();
        let fn_ptr: extern "C" fn(i32) = unsafe { std::mem::transmute(code_ptr) };

        fn_ptr(42);
        assert!(CALLED.load(Ordering::SeqCst));

        TrampolineRegistry::clear_handler();
    }
}
