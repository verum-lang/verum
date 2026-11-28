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
    payload_size: u32,
    owners: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CallableOwner {
    address: usize,
    generation: u32,
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
        owns_captures: bool,
    ) {
        self.callables.insert(
            object.as_ptr() as usize,
            CallableReceipt {
                function,
                generation: object.generation(),
                capture_types,
                lexical_types,
                payload_size: object.header().size,
                owners: owns_captures.then_some(1),
            },
        );
    }

    /// Both lifecycle operations check the actual producer and signature.
    /// Only explicit ownership evidence authorizes retaining or releasing an
    /// environment. Legacy nonempty environments remain opaque.
    pub(crate) fn validate_callable(
        &self,
        module: &VbcModule,
        value: Value,
        expected: &TypeRef,
    ) -> InterpreterResult<Option<CallableOwner>> {
        let (function, witnesses, captures, owner) = if value.is_func_ref() {
            (value.as_func_id(), &[][..], 0, None)
        } else if value.is_regular_ptr() {
            let pointer = value.as_ptr::<ObjectHeader>();
            let receipt = self
                .callables
                .get(&(pointer as usize))
                .filter(|_| self.contains(pointer))
                .ok_or_else(|| {
                    invalid("callable ownership requires its producer-issued environment receipt")
                })?;
            // SAFETY: the receipt and exact live allocation membership precede
            // every read; only the canonical closure producer issues receipts.
            let header = unsafe { &*pointer };
            if !header.is_valid() || header.generation != receipt.generation
                || header.size != receipt.payload_size
                || (header.size as usize) < 8 + receipt.capture_types.len() * std::mem::size_of::<Value>()
            {
                return Err(invalid("callable ownership received a retired environment"));
            }
            // SAFETY: the same producer's live allocation owns this header.
            let (actual, count) = unsafe { closure_header(pointer.cast()) };
            if actual != receipt.function.0 || count as usize != receipt.capture_types.len() {
                return Err(invalid(
                    "callable ownership carrier disagrees with its producer receipt",
                ));
            }
            if receipt.owners.is_none() && !receipt.capture_types.is_empty() {
                return Err(invalid(
                    "callable ownership of a captured environment requires an owned lifecycle",
                ));
            }
            if receipt.owners == Some(0) {
                return Err(invalid("callable environment has no live owner"));
            }
            (receipt.function, receipt.lexical_types.as_slice(), receipt.capture_types.len(),
                receipt.owners.map(|_| CallableOwner { address: pointer as usize, generation: receipt.generation }))
        } else {
            return Err(invalid(
                "callable ownership requires a declared function carrier",
            ));
        };
        let producer = module.resolve_callable_producer(function, captures, witnesses)
            .map_err(|message| invalid(&message))?;
        module.validate_callable_contract(&producer.signature, expected)
            .map_err(|message| invalid(&message))?;
        Ok(owner)
    }

    /// Untyped source operations can use only an actual owned producer receipt,
    /// never a closure-like tag or a function type guessed from pointer bits.
    pub(crate) fn owned_callable_signature(&self, module: &VbcModule, value: Value)
        -> InterpreterResult<Option<TypeRef>> {
        if !value.is_regular_ptr() { return Ok(None); }
        let Some(receipt) = self.callables.get(&(value.as_ptr::<u8>() as usize))
            .filter(|receipt| receipt.owners.is_some()) else { return Ok(None); };
        let producer = module.resolve_callable_producer(
            receipt.function, receipt.capture_types.len(), &receipt.lexical_types,
        ).map_err(|message| invalid(&message))?;
        self.validate_callable(module, value, &producer.signature)?;
        Ok(Some(producer.signature))
    }

    fn callable_receipt(&self, owner: CallableOwner) -> InterpreterResult<&CallableReceipt> {
        self.callables.get(&owner.address)
            .filter(|receipt| receipt.generation == owner.generation && receipt.owners.is_some_and(|count| count != 0))
            .ok_or_else(|| invalid("callable lifecycle receipt is no longer live"))
    }

    pub(crate) fn retain_callable(&mut self, owner: CallableOwner) -> InterpreterResult<()> {
        let receipt = self.callable_receipt(owner)?;
        let count = receipt.owners.and_then(|count| count.checked_add(1))
            .ok_or_else(|| invalid("callable owner count overflow"))?;
        self.callables.get_mut(&owner.address).expect("checked callable receipt").owners = Some(count);
        Ok(())
    }

    pub(crate) fn undo_callable_retain(&mut self, owner: CallableOwner) -> InterpreterResult<()> {
        let receipt = self.callable_receipt(owner)?;
        let count = receipt.owners.filter(|count| *count > 1)
            .ok_or_else(|| invalid("callable rollback cannot release the original owner"))?;
        self.callables.get_mut(&owner.address).expect("checked callable receipt").owners = Some(count - 1);
        Ok(())
    }

    pub(crate) fn callable_capture_types(&self, owner: CallableOwner) -> InterpreterResult<&[TypeRef]> {
        Ok(self.callable_receipt(owner)?.capture_types.as_slice())
    }

    pub(crate) fn callable_is_last_owner(&self, owner: CallableOwner) -> InterpreterResult<bool> {
        Ok(self.callable_receipt(owner)?.owners == Some(1))
    }

    pub(crate) fn callable_capture_values(&self, owner: CallableOwner) -> InterpreterResult<List<Value>> {
        let receipt = self.callable_receipt(owner)?;
        let mut values = List::new();
        values.try_reserve_exact(receipt.capture_types.len())
            .map_err(|_| invalid("cannot allocate callable lifecycle workspace"))?;
        for index in 0..receipt.capture_types.len() {
            // SAFETY: the producer's live allocation, generation, capture count
            // and complete payload extent were checked before issuing owner.
            values.push(unsafe { super::closure_captures_ptr(owner.address as *const u8, index).read() });
        }
        Ok(values)
    }

    /// The last owner retires the carrier before any captured destructor runs.
    /// Callers retain the checked capture values and plans in explicit work lists.
    pub(crate) fn release_callable(&mut self, owner: CallableOwner) -> InterpreterResult<bool> {
        let count = self.callable_receipt(owner)?.owners.expect("owned receipt");
        if count > 1 {
            self.callables.get_mut(&owner.address).expect("checked callable receipt").owners = Some(count - 1);
            return Ok(false);
        }
        let pointer = owner.address as *mut ObjectHeader;
        if !self.contains(pointer) {
            return Err(invalid("callable environment was retired before release"));
        }
        // SAFETY: an exact live producer receipt owns this allocation; this is
        // its final owner and captured values have been moved to the work list.
        let object = unsafe { Object::from_raw(pointer) }.expect("live callable allocation");
        unsafe { self.free(object); }
        Ok(true)
    }
}
