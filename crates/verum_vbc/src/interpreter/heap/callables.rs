//! Closure ownership evidence is issued by formation, never inferred from bits.
use super::{Heap, Object, ObjectHeader, closure_header};
use crate::interpreter::{InterpreterError, InterpreterResult};
use crate::{
    module::{FunctionId, VbcModule},
    types::TypeRef,
    value::Value,
};
use verum_common::List;

pub(super) struct CallableReceipt {
    function: FunctionId,
    generation: u32,
    capture_types: List<TypeRef>,
    lexical_types: List<TypeRef>,
}

fn invalid(message: &str) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: message.into(),
    }
}

impl Heap {
    pub(crate) fn record_callable(
        &mut self,
        object: &Object,
        function: FunctionId,
        capture_types: List<TypeRef>,
        lexical_types: List<TypeRef>,
    ) {
        self.callables.insert(
            object.as_ptr() as usize,
            CallableReceipt {
                function,
                generation: object.generation(),
                capture_types,
                lexical_types,
            },
        );
    }

    /// Retiring a binding does not own the shared environment. Only a known
    /// function or a producer-issued empty closure has no such obligation.
    pub(crate) fn validate_empty_callable(
        &self,
        module: &VbcModule,
        value: Value,
        expected: &TypeRef,
    ) -> InterpreterResult<()> {
        let (function, witnesses) = if value.is_func_ref() {
            (value.as_func_id(), &[][..])
        } else if value.is_regular_ptr() {
            let pointer = value.as_ptr::<ObjectHeader>();
            let receipt = self
                .callables
                .get(&(pointer as usize))
                .filter(|_| self.contains(pointer))
                .ok_or_else(|| {
                    invalid("callable cleanup requires its producer-issued environment receipt")
                })?;
            // SAFETY: the receipt and exact live allocation membership precede
            // every read; only the canonical closure producer issues receipts.
            let header = unsafe { &*pointer };
            if !header.is_valid() || header.generation != receipt.generation {
                return Err(invalid("callable cleanup received a retired environment"));
            }
            // SAFETY: the same producer's live allocation owns this header.
            let (actual, count) = unsafe { closure_header(pointer.cast()) };
            if actual != receipt.function.0 || count as usize != receipt.capture_types.len() {
                return Err(invalid(
                    "callable cleanup carrier disagrees with its producer receipt",
                ));
            }
            if !receipt.capture_types.is_empty() {
                return Err(invalid(
                    "callable cleanup of a captured environment requires an owned lifecycle",
                ));
            }
            (receipt.function, receipt.lexical_types.as_slice())
        } else {
            return Err(invalid(
                "callable cleanup requires a declared function carrier",
            ));
        };
        let declaration = module
            .get_function(function)
            .ok_or_else(|| invalid("callable cleanup function identity is not declared"))?;
        let signature = declaration
            .semantic_callable_signature()
            .ok_or_else(|| invalid("callable cleanup function lacks its semantic signature"))?;
        if witnesses.len() != declaration.type_params.len() {
            return Err(invalid("callable cleanup has incomplete lexical witnesses"));
        }
        let substitution = crate::mono::TypeSubstitution::new(&declaration.type_params, witnesses);
        let signature = substitution
            .apply(&signature)
            .map_err(|_| invalid("callable cleanup cannot instantiate its declared signature"))?;
        // Cleanup compares the retained callable contract structurally. It
        // does not grant the separate full computational type-identity query.
        if !crate::mono::is_concrete_type(&signature) || !crate::mono::is_concrete_type(expected) {
            return Err(invalid(
                "callable cleanup has an unresolved semantic signature",
            ));
        }
        if &signature != expected {
            return Err(invalid(
                "callable cleanup producer does not match the parameter signature",
            ));
        }
        Ok(())
    }
}
