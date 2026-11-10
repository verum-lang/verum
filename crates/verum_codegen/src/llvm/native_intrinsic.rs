//! Emission-owned bindings from canonical intrinsic identities to native bodies.
//! This state belongs to one lowering run; source attributes cannot populate it.

use verum_common::Map;
use verum_llvm::{types::FunctionType, values::FunctionValue};
use verum_vbc::intrinsics::Intrinsic;

use super::error::{LlvmLoweringError, Result};

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
