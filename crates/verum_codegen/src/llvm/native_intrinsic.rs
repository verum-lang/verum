//! Emission-owned bindings from canonical intrinsic identities to native bodies.
//! This state belongs to one lowering run; source attributes cannot populate it.

use verum_common::Map;
use verum_llvm::{types::FunctionType, values::FunctionValue};
use verum_vbc::intrinsics::Intrinsic;

use super::error::{LlvmLoweringError, Result};

/// The compiler embeds freestanding objects as LLVM bitcode so a final native
/// link needs neither a C SDK nor a helper library from the user's system.
pub(super) fn link_linux_signal_watch<'ctx>(
    context: &'ctx verum_llvm::context::Context,
    module: &verum_llvm::module::Module<'ctx>,
    implementations: &mut NativeIntrinsicImplementations<'ctx>,
) -> Result<()> {
    use verum_llvm::{memory_buffer::MemoryBuffer, module::Module};
    let code: &[u8] = if super::target_triple::target_is_aarch64(module) {
        include_bytes!(concat!(env!("OUT_DIR"), "/linux_signal_aarch64.bc"))
    } else if super::target_triple::target_is_x86_64(module) {
        include_bytes!(concat!(env!("OUT_DIR"), "/linux_signal_x86_64.bc"))
    } else {
        return Err(LlvmLoweringError::UnsupportedInstruction("linux_signal_watch target ABI".into()));
    };
    let buffer = MemoryBuffer::create_from_memory_range_copy(code, "linux_signal_watch");
    let native = Module::parse_bitcode_from_buffer(&buffer, context)
        .map_err(|error| LlvmLoweringError::Internal(error.to_string().into()))?;
    native.set_triple(&module.get_triple());
    module.link_in_module(native)
        .map_err(|error| LlvmLoweringError::Internal(error.to_string().into()))?;
    let function = module.get_function("verum_linux_signal_watch")
        .ok_or_else(|| LlvmLoweringError::Internal("missing linked Linux signal entry point".into()))?;
    let intrinsic = verum_vbc::intrinsics::lookup_intrinsic("linux_signal_watch")
        .ok_or_else(|| LlvmLoweringError::Internal("missing Linux signal intrinsic registration".into()))?;
    let word = context.i64_type();
    implementations.register(intrinsic.intrinsic, function, word.fn_type(&[word.into(), word.into()], false))
}

#[derive(Default)]
pub(crate) struct NativeIntrinsicImplementations<'ctx> {
    implementations: Map<&'static str, FunctionValue<'ctx>>,
}

impl<'ctx> NativeIntrinsicImplementations<'ctx> {
    /// Register only after the selected emitter has created the actual body.
    pub(crate) fn register(
        &mut self,
        intrinsic: &Intrinsic,
        function: FunctionValue<'ctx>,
        expected_abi: FunctionType<'ctx>,
    ) -> Result<()> {
        if function.get_type() != expected_abi || function.count_basic_blocks() == 0 {
            return Err(LlvmLoweringError::TypeLowering(
                format!(
                    "Native intrinsic '{}' does not match its emitter-owned ABI or has no body",
                    intrinsic.name,
                )
                .into(),
            ));
        }
        if self
            .implementations
            .get(&intrinsic.name)
            .is_some_and(|prior| *prior != function)
        {
            return Err(LlvmLoweringError::TypeLowering(
                format!(
                    "Native intrinsic '{}' has multiple implementation owners",
                    intrinsic.name,
                )
                .into(),
            ));
        }
        self.implementations.insert(intrinsic.name, function);
        Ok(())
    }

    pub(crate) fn get(&self, identity: &str) -> Option<FunctionValue<'ctx>> {
        let canonical = verum_vbc::intrinsics::lookup_intrinsic(identity)?;
        self.implementations.get(&canonical.intrinsic.name).copied()
    }
}
