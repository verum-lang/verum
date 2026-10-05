//! Owned data required by the Windows x64 floating-point object ABI.
use super::{
    error::{LlvmLoweringError, Result},
    target_triple::{target_is_windows, target_is_x86_64},
};
use verum_llvm::{
    DLLStorageClass,
    comdat::ComdatSelectionKind,
    context::Context,
    module::{Linkage, Module},
};

/// LLVM's x64 MSVC backend references `_fltused` when emitting floating-point
/// code. It is a link marker, not a formatting function or initialization hook.
/// Our SSE2 code and integer formatter require no CRT member's side effects.
/// This deliberately does not cover x86-32's separate x87 initialization ABI.
pub(super) fn emit_float_marker<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> Result<()> {
    let triple = module.get_triple();
    if !target_is_windows(module)
        || !target_is_x86_64(module)
        || !triple
            .as_str()
            .to_string_lossy()
            .split('-')
            .any(|part| part == "msvc")
    {
        return Ok(());
    }
    const NAME: &str = "_fltused";
    if module.get_function(NAME).is_some() {
        return Err(LlvmLoweringError::internal(
            "reserved Windows Float ABI datum _fltused is a function",
        ));
    }
    let ty = context.i32_type();
    let global = module
        .get_global(NAME)
        .unwrap_or_else(|| module.add_global(ty, None, NAME));
    if global.get_value_type() != ty.into()
        || global.as_pointer_value().get_type().get_address_space()
            != verum_llvm::AddressSpace::default()
        || global.is_thread_local()
        || global.get_dll_storage_class() == DLLStorageClass::Import
        || global
            .get_initializer()
            .is_some_and(|value| value != ty.const_zero())
    {
        return Err(LlvmLoweringError::internal(
            "incompatible reserved Windows Float ABI datum _fltused",
        ));
    }
    global.set_initializer(&ty.const_zero());
    global.set_alignment(4);
    // WeakODR survives IR GlobalDCE even though the backend adds the reference
    // later. Matching definitions from separate generated objects coalesce.
    // No llvm.used rewrite, dynamic import, or CRT dependency is needed.
    global.set_linkage(Linkage::WeakODR);
    let comdat = module.get_or_insert_comdat(NAME);
    comdat.set_selection_kind(ComdatSelectionKind::ExactMatch);
    global.set_comdat(comdat);
    Ok(())
}
