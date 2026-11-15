//! Executable calling-convention compatibility, independent of source typing.
use crate::{module::FunctionDescriptor, types::PropertySet};
use verum_common::Text;

/// A declaration flag cannot manufacture an executable parameter lifecycle.
pub fn validate_body(function: &FunctionDescriptor) -> Result<(), Text> {
    if function.owned_value_parameters
        && (!function.has_source_body
            || function.is_generator
            || function.properties.contains(PropertySet::ASYNC)
            || function.requires_intrinsic_implementation()
            || (function.bytecode_length == 0
                && function
                    .instructions
                    .as_ref()
                    .is_none_or(|body| body.is_empty())))
    {
        return Err(
            "owned parameter convention requires its emitted synchronous source body".into(),
        );
    }
    Ok(())
}

/// Both sides must agree before executing a selected source body. External
/// intrinsic implementations own their independent ABI and are not parameter
/// cleanup certificates. Legacy-to-legacy remains the legacy convention.
pub fn validate_call(caller: &FunctionDescriptor, callee: &FunctionDescriptor) -> Result<(), Text> {
    validate_body(caller)?;
    validate_body(callee)?;
    if !callee.requires_intrinsic_implementation()
        && caller.owned_value_parameters != callee.owned_value_parameters
    {
        return Err("incompatible owned parameter calling conventions".into());
    }
    Ok(())
}
