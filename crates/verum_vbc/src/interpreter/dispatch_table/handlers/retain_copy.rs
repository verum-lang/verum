//! A shallow wrapper is allocated before the single ownership-changing CAS.
use super::super::{
    super::{
        error::{InterpreterError, InterpreterResult},
        heap,
        state::InterpreterState,
    },
    DispatchResult,
};
use super::{
    bytecode_io::{read_reg, read_type_ref},
    cbgr_helpers,
};
use crate::value::Value;

fn invalid(message: impl Into<verum_common::Text>) -> InterpreterError {
    InterpreterError::InvalidOperand {
        message: message.into().into(),
    }
}

/// Prepared storage is private until its exact retain succeeds. Every receipt
/// is allocated/reserved by the enclosing operation before entering commit.
pub(super) struct PreparedRetain {
    wrapper: heap::Object,
    counter: cbgr_helpers::RetainCounter,
}
impl PreparedRetain {
    pub(super) fn prepare(
        state: &mut InterpreterState,
        source: Value,
        counter: Value,
        layout: crate::retain_copy::RetainLayout,
    ) -> InterpreterResult<Self> {
        let source = cbgr_helpers::read_live_reference(state, source)?;
        let count =
            cbgr_helpers::MutableValueSlot::resolve(state, counter)?.prepare_counter(state)?;
        if !source.is_regular_ptr() || source.is_nil() || !state.heap.contains(source.as_ptr()) {
            return Err(invalid("retain_copy requires a live handle"));
        }
        let source = source.as_ptr::<heap::ObjectHeader>();
        let size = layout.fields.max(1) * std::mem::size_of::<Value>();
        // SAFETY: exact membership establishes the live object header.
        let header = unsafe { &*source };
        if !header.is_valid() || header.type_id != layout.owner || header.size as usize != size {
            return Err(invalid("retain_copy handle differs from its declaration"));
        }
        let wrapper = state.heap.alloc(layout.owner, size)?;
        state.record_allocation();
        // SAFETY: both extents are checked. Only declared scalar/reference
        // fields are copied, so no owned child is acquired by this memcpy.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.cast::<u8>().add(heap::OBJECT_HEADER_SIZE),
                wrapper.data_ptr(),
                size,
            );
        }
        Ok(Self {
            wrapper,
            counter: count,
        })
    }
    pub(super) fn retain(
        self,
        state: &mut InterpreterState,
    ) -> Result<RetainedCopy, (Self, InterpreterError)> {
        match self.counter.retain(state) {
            Ok(()) => Ok(RetainedCopy { prepared: self }),
            Err(error) => Err((self, error)),
        }
    }
    /// An uncommitted shallow wrapper owns no retained child.
    pub(super) fn abort(self, state: &mut InterpreterState) {
        // SAFETY: the wrapper belongs exclusively to this unpublished receipt.
        unsafe {
            state.heap.free(self.wrapper);
        }
    }
    pub(super) fn prepared_value(&self) -> Value {
        Value::from_ptr(self.wrapper.as_ptr().cast::<u8>())
    }
}

/// A successful retain cannot be forged from an uncommitted receipt. Publishing
/// it consumes this proof; rollback consumes it instead and undoes exactly one CAS.
pub(super) struct RetainedCopy {
    prepared: PreparedRetain,
}
impl RetainedCopy {
    pub(super) fn abort(self, state: &mut InterpreterState) -> InterpreterResult<()> {
        let released = self.prepared.counter.release_retained(state);
        self.prepared.abort(state);
        released
    }
    pub(super) fn publish(self) -> Value {
        self.prepared.prepared_value()
    }
}

pub(in super::super) fn handle(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let counter = read_reg(state)?;
    let declared = read_type_ref(state)?;
    let ty = state.resolve_generic_witness(&declared)?;
    let layout =
        crate::retain_copy::layout(&state.module, &ty).map_err(|e| invalid(e.to_string()))?;
    let receipt =
        PreparedRetain::prepare(state, state.get_reg(src), state.get_reg(counter), layout)?;
    let retained = match receipt.retain(state) {
        Ok(retained) => retained,
        Err((receipt, error)) => {
            receipt.abort(state);
            return Err(error);
        }
    };
    // Publication is infallible: no allocator, callback or journal growth follows CAS.
    state.set_reg(dst, retained.publish());
    Ok(DispatchResult::Continue)
}

#[cfg(test)]
#[path = "../../../../tests/unit/retain_copy_transactions.rs"]
mod tests;
