//! Executable calling-convention compatibility, independent of source typing.
use crate::{module::{FunctionDescriptor, FunctionId, VbcModule}, types::TypeRef};
use verum_common::{List, Text};

/// An executable declaration paired with the signature instantiated from it.
pub struct CallableDeclaration<'a> {
    /// The exact descriptor selected by the formation instruction.
    pub function: &'a FunctionDescriptor,
    /// Its semantic callable type after substituting the lexical witnesses.
    pub signature: TypeRef,
}

impl VbcModule {
    /// The producer selected at formation owns the environment arity, semantic
    /// signature and lexical parameter roster used by both execution tiers.
    pub fn resolve_callable_producer(
        &self, function: FunctionId, captures: usize, witnesses: &[TypeRef],
    ) -> Result<CallableDeclaration<'_>, Text> {
        let declaration = self.get_function(function).filter(|entry| entry.id == function)
            .ok_or_else(|| Text::from("callable ownership function identity is not declared"))?;
        declaration.validate_closure_environment(captures)?;
        let signature = declaration.semantic_callable_signature()
            .ok_or_else(|| Text::from("callable ownership function lacks its semantic signature"))?;
        if witnesses.len() != declaration.type_params.len() {
            return Err("callable ownership has incomplete lexical witnesses".into());
        }
        let substitution = crate::mono::TypeSubstitution::new(&declaration.type_params, witnesses);
        let signature = crate::associated_types::instantiate_witness(self, &substitution, &signature)
            .map_err(|_| Text::from("callable ownership cannot instantiate its declared signature"))?;
        if !crate::mono::closed_signature(&signature) {
            return Err("callable ownership has an unresolved semantic signature".into());
        }
        Ok(CallableDeclaration { function: declaration, signature })
    }
}

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
            || function.requires_intrinsic_implementation()
            || (function.bytecode_length == 0
                && function
                    .instructions
                    .as_ref()
                    .is_none_or(|body| body.is_empty())))
    {
        return Err(
            "owned parameter convention requires its emitted eager source body".into(),
        );
    }
    Ok(())
}

impl VbcModule {
    /// Resolve plain aliases in both closed signatures using this declaration table.
    pub fn validate_callable_contract(&self, provided: &TypeRef, expected: &TypeRef) -> Result<(), Text> {
        let normalize = |ty: &TypeRef| -> Result<TypeRef, Text> {
            if !crate::mono::is_concrete_type(ty) {
                return Err("callable ownership has an unresolved semantic signature".into());
            }
            let mut result = ty.clone();
            crate::associated_types::normalize_for(self, &mut result, crate::associated_types::WitnessUse::CallableContract)
                .map_err(|error| Text::from(error.to_string()))?;
            Ok(result)
        };
        validate_callable_contract(&normalize(provided)?, &normalize(expected)?)
    }

    /// A source call must use the parameter lifecycle implemented by both
    /// declarations. Intrinsics own their independent ABI.
    pub fn validate_parameter_call(
        &self,
        caller: &FunctionDescriptor,
        callee: &FunctionDescriptor,
    ) -> Result<(), Text> {
        validate_body(caller)?;
        validate_body(callee)?;
        if !callee.requires_intrinsic_implementation()
            && caller.owned_value_parameters != callee.owned_value_parameters
        {
            let label = |function: &FunctionDescriptor| {
                let name = self.get_string(function.name).unwrap_or("<unnamed>");
                format!("{name} [#{}]", function.id.0)
            };
            return Err(format!(
                "incompatible owned parameter calling conventions: {} calls {}",
                label(caller), label(callee)
            ).into());
        }
        Ok(())
    }
}
