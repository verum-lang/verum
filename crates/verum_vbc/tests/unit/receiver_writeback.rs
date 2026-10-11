//! Receiver autoderef is opt-in; an ordinary reference store replaces one slot.
use super::super::ffi_extended::{cbgr_user_allocate, cbgr_user_deallocate};
use super::{encode_cbgr_ref_mut, resolve_arg_value, resolve_receiver_place, write_through_ref};
use crate::interpreter::{error::InterpreterError, state::InterpreterState};
use crate::module::VbcModule;
use crate::value::{Capabilities, ThinRef, Value};
use std::sync::Arc;

fn state() -> InterpreterState {
    let mut state = InterpreterState::new(Arc::new(VbcModule::new("writeback".into())));
    state.registers.push_frame(4);
    state
}

fn thin(slot: &mut Value) -> Value {
    Value::from_thin_ref(ThinRef::new(
        (slot as *mut Value).cast(),
        1,
        0,
        Capabilities::MUT_EXCLUSIVE,
    ))
}

#[test]
fn ordinary_register_store_replaces_reference_value_without_following_it() {
    let mut state = state();
    let origin = Value::from_i64(17);
    state.registers.set_absolute(0, origin);
    let inner = encode_cbgr_ref_mut(0, 1);
    state.registers.set_absolute(1, inner);
    let outer = encode_cbgr_ref_mut(1, 1);
    let replacement = encode_cbgr_ref_mut(2, 1);
    assert!(write_through_ref(&mut state, outer, replacement));
    assert_eq!(state.registers.get_absolute(1).bits(), replacement.bits());
    assert_eq!(state.registers.get_absolute(0).bits(), origin.bits());
}

#[test]
fn ordinary_pointer_store_replaces_reference_value_without_following_it() {
    for tracked in [false, true] {
        let mut state = state();
        let origin = Value::from_i64(17);
        state.registers.set_absolute(0, origin);
        let mut slot = encode_cbgr_ref_mut(0, 1);
        let reference = if tracked {
            let address = &mut slot as *mut Value;
            state.cbgr_mutable_ptrs.insert(address as usize);
            Value::from_ptr(address)
        } else {
            thin(&mut slot)
        };
        let replacement = encode_cbgr_ref_mut(2, 1);
        assert!(write_through_ref(&mut state, reference, replacement));
        assert_eq!(slot.bits(), replacement.bits());
        assert_eq!(state.registers.get_absolute(0).bits(), origin.bits());
    }
}

#[test]
fn receiver_register_chain_keeps_original_register_target() {
    let mut state = state();
    state.registers.set_absolute(0, Value::from_i64(17));
    let original = encode_cbgr_ref_mut(0, 1);
    state.registers.set_absolute(1, original);
    let (current, target) = resolve_receiver_place(&state, encode_cbgr_ref_mut(1, 1)).unwrap();
    assert_eq!(current.as_i64(), 17);
    assert_eq!(target.bits(), original.bits());
    assert!(write_through_ref(&mut state, target, Value::from_i64(23)));
    assert_eq!(state.registers.get_absolute(0).as_i64(), 23);
    assert_eq!(state.registers.get_absolute(1).bits(), original.bits());
}

#[test]
fn receiver_register_chain_keeps_pointer_target_and_alias() {
    for tracked in [false, true] {
        let mut state = state();
        let mut slot = Value::from_i64(17);
        let reference = if tracked {
            let address = &mut slot as *mut Value;
            state.cbgr_mutable_ptrs.insert(address as usize);
            Value::from_ptr(address)
        } else {
            thin(&mut slot)
        };
        state.registers.set_absolute(0, reference);
        let (current, target) = resolve_receiver_place(&state, encode_cbgr_ref_mut(0, 1)).unwrap();
        assert_eq!(current.as_i64(), 17);
        assert_eq!(target.bits(), reference.bits());
        assert!(write_through_ref(&mut state, target, Value::from_i64(23)));
        assert_eq!(slot.as_i64(), 23);
        assert_eq!(state.registers.get_absolute(0).bits(), reference.bits());
        assert_eq!(resolve_arg_value(&state, reference).as_i64(), 23);
    }
}

#[test]
fn bridge_extent_wins_over_interior_slot_tracking() {
    let mut state = state();
    let address = cbgr_user_allocate(&mut state, std::mem::size_of::<Value>() as i64, 8);
    assert!(address > 0);
    let slot = address as *mut Value;
    // SAFETY: the actual bridge allocator owns one complete aligned Value slot.
    unsafe { slot.write(Value::from_i64(17)) };
    state.cbgr_mutable_ptrs.insert(address as usize);
    let reference = Value::from_ptr(slot);
    let observed = resolve_arg_value(&state, reference);
    let written = write_through_ref(&mut state, reference, Value::from_i64(23));
    // SAFETY: same live allocation; deallocate only after taking the snapshot.
    let contents = unsafe { slot.read() };
    state.cbgr_mutable_ptrs.remove(&(address as usize));
    cbgr_user_deallocate(&mut state, address);
    assert_eq!(observed.bits(), reference.bits());
    assert!(!written);
    assert_eq!(contents.as_i64(), 17);
}

#[test]
fn untracked_and_unaligned_addresses_do_not_authorize_slot_access() {
    let mut state = state();
    let mut slot = Value::from_i64(17);
    let pointer = Value::from_ptr(&mut slot as *mut Value);
    assert_eq!(resolve_arg_value(&state, pointer).bits(), pointer.bits());
    assert!(!write_through_ref(&mut state, pointer, Value::from_i64(23)));
    let misaligned = (&mut slot as *mut Value).cast::<u8>().wrapping_add(1);
    state.cbgr_mutable_ptrs.insert(misaligned as usize);
    for reference in [
        Value::from_ptr(misaligned),
        Value::from_thin_ref(ThinRef::new(misaligned, 1, 0, Capabilities::MUT_EXCLUSIVE)),
    ] {
        assert_eq!(
            resolve_arg_value(&state, reference).bits(),
            reference.bits()
        );
        assert!(!write_through_ref(
            &mut state,
            reference,
            Value::from_i64(23)
        ));
    }
    assert_eq!(slot.as_i64(), 17);
}

#[test]
fn cyclic_receiver_chain_is_a_bounded_operand_error() {
    let mut state = state();
    state.registers.set_absolute(0, encode_cbgr_ref_mut(1, 1));
    state.registers.set_absolute(1, encode_cbgr_ref_mut(0, 1));
    assert!(matches!(
        resolve_receiver_place(&state, encode_cbgr_ref_mut(0, 1)),
        Err(InterpreterError::InvalidOperand { .. })
    ));
}

#[test]
fn out_of_bounds_receiver_ref_is_an_operand_error() {
    let state = state();
    assert!(matches!(
        resolve_receiver_place(&state, encode_cbgr_ref_mut(state.registers.top() as u32, 1)),
        Err(InterpreterError::InvalidOperand { .. })
    ));
}
