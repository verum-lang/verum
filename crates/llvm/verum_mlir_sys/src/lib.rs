//! Low-level FFI bindings for MLIR C API.
//!
//! This crate provides raw bindings to the MLIR C API, built against the local
//! LLVM installation in `llvm/install/`.
//!
//! For a safe, ergonomic API, use the `verum_mlir` crate instead.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_context_create_destroy() {
        unsafe {
            let ctx = mlirContextCreate();
            assert!(mlirContextEqual(ctx, ctx));
            mlirContextDestroy(ctx);
        }
    }

    #[test]
    fn test_string_ref() {
        unsafe {
            let string = CString::new("Hello, MLIR!").unwrap();
            mlirStringRefCreateFromCString(string.as_ptr());
        }
    }

    #[test]
    fn test_location() {
        unsafe {
            let registry = mlirDialectRegistryCreate();
            let context = mlirContextCreate();

            mlirContextAppendDialectRegistry(context, registry);
            mlirRegisterAllDialects(registry);

            let location = mlirLocationUnknownGet(context);
            let string = CString::new("test_module").unwrap();
            let reference = mlirStringRefCreateFromCString(string.as_ptr());

            mlirOperationStateGet(reference, location);

            mlirContextDestroy(context);
            mlirDialectRegistryDestroy(registry);
        }
    }
}
