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

pub(in super::super) fn handle(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let counter = read_reg(state)?;
    let declared = read_type_ref(state)?;
    let ty = state.resolve_generic_witness(&declared)?;
    let layout =
        crate::retain_copy::layout(&state.module, &ty).map_err(|e| invalid(e.to_string()))?;
    let source = cbgr_helpers::read_live_reference(state, state.get_reg(src))?;
    let count = cbgr_helpers::MutableValueSlot::resolve(state, state.get_reg(counter))?;
    if !source.is_regular_ptr() || source.is_nil() || !state.heap.contains(source.as_ptr()) {
        return Err(invalid("retain_copy requires a live handle"));
    }
    let source = source.as_ptr::<heap::ObjectHeader>();
    let size = layout.fields.max(1) * std::mem::size_of::<Value>();
    // SAFETY: exact allocation membership establishes the object header.
    let header = unsafe { &*source };
    if !header.is_valid() || header.type_id != layout.owner || header.size as usize != size {
        return Err(invalid("retain_copy handle differs from its declaration"));
    }
    let copied = state.heap.alloc(layout.owner, size)?;
    state.record_allocation();
    // SAFETY: both allocations have the same checked extent. The descriptor
    // proves every field is a scalar/reference; no owning child is duplicated.
    unsafe {
        std::ptr::copy_nonoverlapping(
            source.cast::<u8>().add(heap::OBJECT_HEADER_SIZE),
            copied.data_ptr(),
            size,
        );
    }
    if let Err(error) = count.retain_u32(state) {
        // SAFETY: this unpublished wrapper was allocated here, and no retain
        // succeeded. Its borrowed slots require no user destruction callback.
        unsafe {
            state.heap.free(copied);
        }
        return Err(error);
    }
    // Everything after the successful CAS is infallible. In particular no
    // allocation, callback, or ledger growth occurs before publication.
    state.set_reg(dst, Value::from_ptr(copied.as_ptr().cast::<u8>()));
    Ok(DispatchResult::Continue)
}
