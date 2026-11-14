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
    let inner = encode_cbgr_ref_mut(0, state.registers.get_generation(0));
    state.registers.set_absolute(1, inner);
    let outer = encode_cbgr_ref_mut(1, state.registers.get_generation(1));
    let replacement = encode_cbgr_ref_mut(2, state.registers.get_generation(2));
    assert!(write_through_ref(&mut state, outer, replacement).unwrap());
    assert_eq!(state.registers.get_absolute(1).bits(), replacement.bits());
    assert_eq!(state.registers.get_absolute(0).bits(), origin.bits());
}

#[test]
fn ordinary_pointer_store_replaces_reference_value_without_following_it() {
    for tracked in [false, true] {
        let mut state = state();
        let origin = Value::from_i64(17);
        state.registers.set_absolute(0, origin);
        let mut slot = encode_cbgr_ref_mut(0, state.registers.get_generation(0));
        let reference = if tracked {
            let address = &mut slot as *mut Value;
            state.cbgr_mutable_ptrs.insert(address as usize);
            Value::from_ptr(address)
        } else {
            thin(&mut slot)
        };
        let replacement = encode_cbgr_ref_mut(2, state.registers.get_generation(2));
        assert!(write_through_ref(&mut state, reference, replacement).unwrap());
        assert_eq!(slot.bits(), replacement.bits());
        assert_eq!(state.registers.get_absolute(0).bits(), origin.bits());
    }
}

#[test]
fn receiver_register_chain_keeps_original_register_target() {
    let mut state = state();
    state.registers.set_absolute(0, Value::from_i64(17));
    let original = encode_cbgr_ref_mut(0, state.registers.get_generation(0));
    state.registers.set_absolute(1, original);
    let receiver = resolve_receiver_place(&state, encode_cbgr_ref_mut(1, state.registers.get_generation(1))).unwrap();
    receiver.require_write().unwrap();
    let (current, target) = (receiver.value, receiver.reference);
    assert_eq!(current.as_i64(), 17);
    assert_eq!(target.bits(), original.bits());
    assert!(write_through_ref(&mut state, target, Value::from_i64(23)).unwrap());
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
        let receiver = resolve_receiver_place(&state, encode_cbgr_ref_mut(0, state.registers.get_generation(0))).unwrap();
        receiver.require_write().unwrap();
        let (current, target) = (receiver.value, receiver.reference);
        assert_eq!(current.as_i64(), 17);
        assert_eq!(target.bits(), reference.bits());
        assert!(write_through_ref(&mut state, target, Value::from_i64(23)).unwrap());
        assert_eq!(slot.as_i64(), 23);
        assert_eq!(state.registers.get_absolute(0).bits(), reference.bits());
        assert_eq!(resolve_arg_value(&state, reference).unwrap().as_i64(), 23);
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
    let observed = resolve_arg_value(&state, reference).unwrap();
    let written = write_through_ref(&mut state, reference, Value::from_i64(23)).unwrap();
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
    assert_eq!(resolve_arg_value(&state, pointer).unwrap().bits(), pointer.bits());
    assert!(!write_through_ref(&mut state, pointer, Value::from_i64(23)).unwrap());
    let misaligned = (&mut slot as *mut Value).cast::<u8>().wrapping_add(1);
    state.cbgr_mutable_ptrs.insert(misaligned as usize);
    for reference in [
        Value::from_ptr(misaligned),
        Value::from_thin_ref(ThinRef::new(misaligned, 1, 0, Capabilities::MUT_EXCLUSIVE)),
    ] {
        assert_eq!(
            resolve_arg_value(&state, reference).unwrap().bits(),
            reference.bits()
        );
        assert!(!write_through_ref(
            &mut state,
            reference,
            Value::from_i64(23)
        ).unwrap());
    }
    assert_eq!(slot.as_i64(), 17);
}

#[test]
fn cyclic_receiver_chain_is_a_bounded_operand_error() {
    let mut state = state();
    state.registers.set_absolute(0, encode_cbgr_ref_mut(1, state.registers.get_generation(1)));
    state.registers.set_absolute(1, encode_cbgr_ref_mut(0, state.registers.get_generation(0)));
    assert!(matches!(
        resolve_receiver_place(&state, encode_cbgr_ref_mut(0, state.registers.get_generation(0))),
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

#[test]
fn readonly_heap_references_reject_both_store_paths() {
    let mut state = state();
    let mut slot = Value::from_i64(17);
    let ptr = (&mut slot as *mut Value).cast();
    let value = Value::from_i64(23);
    for reference in [
        Value::from_thin_ref(ThinRef::new(ptr, 1, 0, Capabilities::READ_ONLY)),
        Value::from_fat_ref(crate::value::FatRef::new(ptr, 1, 0, Capabilities::READ_ONLY, 1)),
    ] {
        assert!(matches!(
            write_through_ref(&mut state, reference, value),
            Err(InterpreterError::CapabilityDenied { .. })
        ));
        assert!(matches!(
            super::super::cbgr::store_referent(&mut state, reference, value),
            Err(InterpreterError::CapabilityDenied { .. })
        ));
        assert_eq!(slot.as_i64(), 17);
    }
}

#[test]
fn swap_validates_both_places_before_any_write() {
    use super::{encode_cbgr_ref, swap_referents};
    let mut machine = state();
    machine.registers.set_absolute(0, Value::from_i64(17));
    machine.registers.set_absolute(1, Value::from_i64(23));
    let generation = machine.registers.get_generation(0);
    let valid = encode_cbgr_ref_mut(0, generation);
    let mut cell = Value::from_i64(37);
    let cell_ptr: *mut Value = &mut cell;
    let readonly = Value::from_thin_ref(ThinRef::new(cell_ptr.cast(), 1, 0, Capabilities::READ_ONLY));
    let stale_generation = machine.registers.get_generation(1);
    machine.registers.bump_generation(1);
    let stale = encode_cbgr_ref_mut(1, stale_generation);
    let outside = encode_cbgr_ref_mut(machine.registers.top() as u32, generation);
    for invalid in [readonly, encode_cbgr_ref(1, machine.registers.get_generation(1)), stale,
                    outside, Value::nil(), Value::from_i64(5), Value::from_thin_ref(ThinRef::null())] {
        for (a, b) in [(valid, invalid), (invalid, valid)] {
            assert!(swap_referents(&mut machine, a, b).is_err());
            assert_eq!(machine.registers.get_absolute(0).as_i64(), 17);
            assert_eq!(machine.registers.get_absolute(1).as_i64(), 23);
            assert_eq!(cell.as_i64(), 37);
        }
    }
    let writable = thin(&mut cell);
    let nested = encode_cbgr_ref_mut(1, machine.registers.get_generation(1));
    machine.registers.set_absolute(0, nested);
    swap_referents(&mut machine, valid, writable).unwrap();
    assert_eq!(cell.bits(), nested.bits());
    assert_eq!(machine.registers.get_absolute(1).as_i64(), 23);
    cell = Value::from_i64(17);
    swap_referents(&mut machine, valid, writable).unwrap();
    assert_eq!(cell.as_i64(), 37);
    assert_eq!(machine.registers.get_absolute(0).as_i64(), 17);
    swap_referents(&mut machine, writable, writable).unwrap();
    assert_eq!(cell.as_i64(), 37);
    swap_referents(&mut machine, writable, valid).unwrap();
    assert_eq!(cell.as_i64(), 17);
    assert_eq!(machine.registers.get_absolute(0).as_i64(), 37);
}

#[test]
fn receiver_permissions_intersect_every_forwarding_reference() {
    use super::encode_cbgr_ref;
    let mut state = state();
    let mut slot = Value::from_i64(17);
    let writable = thin(&mut slot);
    state.registers.set_absolute(0, writable);
    state.registers.set_absolute(1, encode_cbgr_ref_mut(0, state.registers.get_generation(0)));
    let receiver = resolve_receiver_place(&state, encode_cbgr_ref(1, state.registers.get_generation(1))).unwrap();
    assert_eq!(receiver.value.try_as_i64(), Some(17));
    assert!(matches!(receiver.require_write(), Err(InterpreterError::CapabilityDenied { .. })));
    let readonly = Value::from_thin_ref(ThinRef::new(
        (&mut slot as *mut Value).cast(), 1, 0, Capabilities::READ_ONLY,
    ));
    state.registers.set_absolute(0, readonly);
    let receiver = resolve_receiver_place(&state, encode_cbgr_ref_mut(1, state.registers.get_generation(1))).unwrap();
    assert!(matches!(receiver.require_write(), Err(InterpreterError::CapabilityDenied { .. })));
    state.registers.bump_generation(0);
    assert!(matches!(resolve_receiver_place(&state, encode_cbgr_ref_mut(1, state.registers.get_generation(1))),
        Err(InterpreterError::CbgrPanic { failure: crate::interpreter::CbgrFailure {
            kind: crate::interpreter::CbgrViolationKind::GenerationMismatch, ..
        }, .. })));
    assert_eq!(slot.try_as_i64(), Some(17));
}
