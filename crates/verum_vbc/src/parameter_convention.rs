//! Executable calling-convention compatibility, independent of source typing.
use crate::{module::FunctionDescriptor, types::{PropertySet, TypeRef}};
use verum_common::{List, Text};

/// Check a producer against the callable contract it is stored under. Async
/// annotations permit an async computation; a synchronous producer satisfies
/// that allowance too. Parameter contracts are contravariant, results covariant,
/// and every non-callable component retains its exact declaration identity.
/// This is call compatibility, not type equality or permission to own captures.
pub fn validate_callable_contract(provided: &TypeRef, expected: &TypeRef) -> Result<(), Text> {
    // The shared bounded traversal rejects unresolved witnesses and limits
    // structural depth before leaf equality or this explicit work list runs.
    if !crate::mono::is_concrete_type(provided) || !crate::mono::is_concrete_type(expected) {
        return Err("callable ownership has an unresolved semantic signature".into());
    }
    if !matches!((expected, provided), (TypeRef::Function { .. }, TypeRef::Function { .. })) {
        return Err("callable ownership requires a declared function contract".into());
    }
    let mismatch = || Text::from("callable ownership producer does not match the parameter signature");
    let mut pending = List::from_iter([(expected, provided)]);
    while let Some((expected, provided)) = pending.pop() {
        match (expected, provided) {
            (TypeRef::Function { is_async: permits_async, params: ep, return_type: er, contexts: ec },
             TypeRef::Function { is_async: uses_async, params: pp, return_type: pr, contexts: pc }) => {
                if (*uses_async && !*permits_async) || ep.len() != pp.len() || ec != pc {
                    return Err(mismatch());
                }
                pending.extend(pp.iter().zip(ep));
                pending.push((er, pr));
            }
            _ if expected == provided => (),
            _ => return Err(mismatch()),
        }
    }
    Ok(())
}


/// A declaration flag cannot manufacture an executable parameter lifecycle.
pub fn validate_body(function: &FunctionDescriptor) -> Result<(), Text> {
    if let Some(captures) = function.closure_capture_count {
        if function.params.first().is_none_or(|parameter|
            parameter.type_ref != crate::TypeRef::Concrete(crate::TypeId::PTR))
            || usize::from(captures) + function.params.len().saturating_sub(1)
                > usize::from(function.register_count)
        {
            return Err("closure environment convention requires its parameter and register layout".into());
        }
    }
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
