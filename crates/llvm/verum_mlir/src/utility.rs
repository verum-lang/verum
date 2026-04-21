//! Utility functions.

use crate::{
    Error, context::Context, dialect::DialectRegistry, ir::Module, logical_result::LogicalResult,
    pass, string_ref::StringRef,
};
use verum_mlir_sys::{
    MlirStringRef, mlirDialectHandleInsertDialect, mlirGetDialectHandle__amdgpu__,
    mlirGetDialectHandle__arith__, mlirGetDialectHandle__cf__, mlirGetDialectHandle__func__,
    mlirGetDialectHandle__gpu__, mlirGetDialectHandle__linalg__, mlirGetDialectHandle__llvm__,
    mlirGetDialectHandle__math__, mlirGetDialectHandle__memref__,
    mlirGetDialectHandle__nvvm__, mlirGetDialectHandle__rocdl__, mlirGetDialectHandle__scf__,
    mlirGetDialectHandle__spirv__, mlirGetDialectHandle__tensor__,
    mlirGetDialectHandle__transform__, mlirGetDialectHandle__vector__, mlirLoadIRDLDialects,
    mlirParsePassPipeline, mlirRegisterAllDialects, mlirRegisterAllLLVMTranslations,
    mlirRegisterAllPasses,
};
use std::{
    ffi::c_void,
    fmt::{self, Formatter},
    sync::Once,
};

/// Registers all dialects to a dialect registry.
pub fn register_all_dialects(registry: &DialectRegistry) {
    unsafe { mlirRegisterAllDialects(registry.to_raw()) }
}

/// Register only the MLIR dialects Verum actually targets.
///
/// Calling `mlirRegisterAllDialects` marks every dialect's `initialize()`
/// method as reachable, which in turn pulls in every op, type, attribute
/// and interface implementation in that dialect — including OpenMP,
/// SparseTensor, Async, Shape, Quant, PDL/IRDL and other dialects Verum
/// never emits. This explicit list lets static-linker dead-code
/// elimination drop the unreachable dialects at link time.
///
/// Verum's pipeline needs:
/// - Core lowering: arith, func, scf, cf, memref, llvm, math
/// - Linear algebra / tensor: linalg, tensor, vector
/// - Pass scheduling: transform
/// - GPU targets: gpu, nvvm, rocdl, spirv, amdgpu
///
/// If a new codegen path introduces `convert-X-to-Y` passes where `X` or
/// `Y` is not on this list, MLIR pass parsing will fail at runtime with
/// a "Dialect not registered" error — that is the signal to add it here.
pub fn register_used_dialects(registry: &DialectRegistry) {
    let raw = registry.to_raw();
    unsafe {
        for handle in [
            mlirGetDialectHandle__arith__(),
            mlirGetDialectHandle__func__(),
            mlirGetDialectHandle__scf__(),
            mlirGetDialectHandle__cf__(),
            mlirGetDialectHandle__memref__(),
            mlirGetDialectHandle__llvm__(),
            mlirGetDialectHandle__math__(),
            mlirGetDialectHandle__linalg__(),
            mlirGetDialectHandle__tensor__(),
            mlirGetDialectHandle__vector__(),
            mlirGetDialectHandle__transform__(),
            mlirGetDialectHandle__gpu__(),
            mlirGetDialectHandle__nvvm__(),
            mlirGetDialectHandle__rocdl__(),
            mlirGetDialectHandle__spirv__(),
            mlirGetDialectHandle__amdgpu__(),
        ] {
            mlirDialectHandleInsertDialect(handle, raw);
        }
    }
}

/// Register all translations from other dialects to the `llvm` dialect.
pub fn register_all_llvm_translations(context: &Context) {
    unsafe { mlirRegisterAllLLVMTranslations(context.to_raw()) }
}

/// Register all passes.
pub fn register_all_passes() {
    static ONCE: Once = Once::new();

    // Multiple calls of `mlirRegisterAllPasses` seems to cause double free.
    ONCE.call_once(|| unsafe { mlirRegisterAllPasses() });
}

/// Parses a pass pipeline.
pub fn parse_pass_pipeline(manager: pass::OperationPassManager, source: &str) -> Result<(), Error> {
    let mut error_message = None;

    let result = LogicalResult::from_raw(unsafe {
        mlirParsePassPipeline(
            manager.to_raw(),
            StringRef::new(source).to_raw(),
            Some(handle_parse_error),
            &mut error_message as *mut _ as *mut _,
        )
    });

    if result.is_success() {
        Ok(())
    } else {
        Err(Error::ParsePassPipeline(error_message.unwrap_or_else(
            || "failed to parse error message in UTF-8".into(),
        )))
    }
}

/// Loads all IRDL dialects in the provided module, registering the dialects in
/// the module's associated context.
pub fn load_irdl_dialects(module: &Module) -> bool {
    unsafe { mlirLoadIRDLDialects(module.to_raw()).value == 1 }
}

unsafe extern "C" fn handle_parse_error(raw_string: MlirStringRef, data: *mut c_void) {
    unsafe {
        let string = StringRef::from_raw(raw_string);
        let data = &mut *(data as *mut Option<String>);

        if let Some(message) = data {
            message.extend(string.as_str())
        } else {
            *data = string.as_str().map(String::from).ok();
        }
    }
}

pub(crate) unsafe extern "C" fn print_callback(string: MlirStringRef, data: *mut c_void) {
    let (formatter, result) = unsafe { &mut *(data as *mut (&mut Formatter, fmt::Result)) };

    if result.is_err() {
        return;
    }

    *result = (|| {
        write!(
            formatter,
            "{}",
            unsafe { StringRef::from_raw(string) }
                .as_str()
                .map_err(|_| fmt::Error)?
        )
    })();
}

pub(crate) unsafe extern "C" fn print_string_callback(string: MlirStringRef, data: *mut c_void) {
    let (writer, result) = unsafe { &mut *(data as *mut (String, Result<(), Error>)) };

    if result.is_err() {
        return;
    }

    *result = (|| {
        writer.push_str(unsafe { StringRef::from_raw(string) }.as_str()?);

        Ok(())
    })();
}

#[cfg(test)]
mod tests {
    use crate::ir::Location;

    use super::*;

    #[test]
    fn register_dialects() {
        let registry = DialectRegistry::new();

        register_all_dialects(&registry);
    }

    #[test]
    fn register_dialects_twice() {
        let registry = DialectRegistry::new();

        register_all_dialects(&registry);
        register_all_dialects(&registry);
    }

    #[test]
    fn register_llvm_translations() {
        let context = Context::new();

        register_all_llvm_translations(&context);
    }

    #[test]
    fn register_llvm_translations_twice() {
        let context = Context::new();

        register_all_llvm_translations(&context);
        register_all_llvm_translations(&context);
    }

    #[test]
    fn register_passes() {
        register_all_passes();
    }

    #[test]
    fn register_passes_twice() {
        register_all_passes();
        register_all_passes();
    }

    #[test]
    fn register_passes_many_times() {
        for _ in 0..1000 {
            register_all_passes();
        }
    }

    #[test]
    fn test_load_irdl_dialects() {
        let context = Context::new();
        let module = Module::new(Location::unknown(&context));

        assert!(load_irdl_dialects(&module));
    }
}
