//! Exact heap ownership must precede interpretation as a Shared carrier.
//! These controls drive the interpreter's actual Deref/DerefMut handlers and
//! allocators; they do not establish source borrow checking or AOT behavior.

use super::cbgr::{handle_deref, handle_deref_mut, shared_inner_cell_result};
use super::ffi_extended::{cbgr_user_allocate, cbgr_user_deallocate};
use crate::interpreter::{heap, state::InterpreterState};
use crate::module::{FunctionDescriptor, VbcModule};
use crate::types::TypeId;
use crate::{FunctionId, Reg, Value};
use std::sync::Arc;
use verum_common::List;

const VALUE_BYTES: usize = std::mem::size_of::<Value>();

struct Fixture {
    state: InterpreterState,
    bridge_allocations: List<i64>,
}

impl Fixture {
    fn new() -> Self {
        // DerefMut r0, r1; Deref r2, r0 (handler operands, without opcodes).
        let mut module = VbcModule::new("shared_authority".into());
        let offset = module.append_bytecode(&[0, 1, 2, 0]);
        let name = module.intern_string("shared_authority.probe");
        module.add_function(FunctionDescriptor {
            id: FunctionId(0),
            name,
            bytecode_offset: offset,
            bytecode_length: 4,
            register_count: 4,
            ..FunctionDescriptor::default()
        });
        let mut state = InterpreterState::new(Arc::new(module));
        state.registers.push_frame(4);
        state
            .call_stack
            .push_frame(FunctionId(0), 4, 0, Reg(0))
            .expect("probe frame");
        Self {
            state,
            bridge_allocations: List::new(),
        }
    }

    fn object(&mut self, type_id: TypeId, values: &[Value]) -> heap::Object {
        let object = self
            .state
            .heap
            .alloc_array(type_id, values.len())
            .expect("heap object");
        // SAFETY: the allocation has exactly values.len() initialized slots.
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr(), object.data_ptr().cast(), values.len())
        };
        object
    }

    fn shared(&mut self, value: Value) -> (heap::Object, *mut Value) {
        let address = cbgr_user_allocate(&mut self.state, (3 * VALUE_BYTES) as i64, 8);
        assert!(address > 0, "real bridge allocation");
        self.bridge_allocations.push(address);
        let inner = address as *mut Value;
        // The current SharedInner declaration has strong, weak, value slots.
        // SAFETY: the runtime allocator registered three writable Value slots.
        unsafe {
            inner.write(Value::from_i64(1));
            inner.add(1).write(Value::from_i64(0));
            inner.add(2).write(value);
        }
        let object = self.object(
            TypeId::SHARED,
            &[
                Value::from_i64(address),
                Value::from_i64(1),
                Value::from_i64(2),
            ],
        );
        (object, inner)
    }

    fn padded_decoy(&mut self, inner: *mut Value) -> (heap::Object, *mut Value) {
        // All bytes a mistaken ObjectHeader/slot-0 probe could read are valid
        // initialized allocation bytes. A wrong classification reaches a live
        // unrelated SharedInner, so the failure oracle never relies on UB.
        let object = self.object(
            TypeId::UNIT,
            &[
                Value::from_i64(991),
                Value::from_i64(520),
                Value::unit(),
                Value::unit(),
                Value::from_i64(inner as i64),
                Value::from_i64(992),
            ],
        );
        // SAFETY: the second of six initialized slots is in bounds.
        let slot = unsafe { object.data_ptr().cast::<Value>().add(1) };
        self.state.cbgr_mutable_ptrs.insert(slot as usize);
        (object, slot)
    }

    fn write(&mut self, pointer: *mut u8, value: Value) {
        self.state.set_reg(Reg(0), Value::from_ptr(pointer));
        self.state.set_reg(Reg(1), value);
        self.state.set_pc(0);
        handle_deref_mut(&mut self.state).expect("DerefMut");
    }

    fn read(&mut self, pointer: *mut u8) -> Value {
        self.state.set_reg(Reg(0), Value::from_ptr(pointer));
        self.state.set_pc(2);
        handle_deref(&mut self.state).expect("Deref");
        self.state.get_reg(Reg(2))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for address in self.bridge_allocations.iter().copied() {
            cbgr_user_deallocate(&mut self.state, address);
        }
    }
}

#[test]
fn tracked_interior_520_is_not_a_shared_carrier() {
    let mut fixture = Fixture::new();
    let (_, inner) = fixture.shared(Value::from_i64(77));
    let (_, slot) = fixture.padded_decoy(inner);
    assert!(
        shared_inner_cell_result(&fixture.state, slot.cast()).is_err(),
        "an interior Int(520) must not authorize the unrelated SharedInner",
    );
}

#[test]
fn interior_write_changes_its_slot_not_an_unrelated_shared_value() {
    let mut fixture = Fixture::new();
    let (shared, inner) = fixture.shared(Value::from_i64(77));
    let (backing, slot) = fixture.padded_decoy(inner);
    fixture.write(slot.cast(), Value::from_i64(43));
    assert_eq!(
        fixture.read(slot.cast()).try_as_i64(),
        Some(43),
        "interior write target"
    );
    assert_eq!(
        fixture.read(shared.as_ptr().cast()).try_as_i64(),
        Some(77),
        "unrelated Shared value"
    );
    // SAFETY: both guard slots remain in the live six-slot allocation.
    unsafe {
        assert_eq!(
            (*backing.data_ptr().cast::<Value>()).try_as_i64(),
            Some(991)
        );
        assert_eq!(
            (*backing.data_ptr().cast::<Value>().add(5)).try_as_i64(),
            Some(992)
        );
    }
}

#[test]
fn final_value_slot_520_needs_no_header_or_following_slot() {
    let mut fixture = Fixture::new();
    let (_, inner) = fixture.shared(Value::from_i64(77));
    let (_, padded) = fixture.padded_decoy(inner);
    // Fail the old implementation deterministically before exposing a real
    // last slot. The baseline must not perform an out-of-bounds header probe.
    assert!(
        shared_inner_cell_result(&fixture.state, padded.cast()).is_err(),
        "interior authority prerequisite"
    );
    let backing = fixture.object(TypeId::UNIT, &[Value::from_i64(520)]);
    let slot = backing.data_ptr().cast::<Value>();
    fixture.state.cbgr_mutable_ptrs.insert(slot as usize);
    assert!(shared_inner_cell_result(&fixture.state, slot.cast()).is_err());
    assert_eq!(fixture.read(slot.cast()).try_as_i64(), Some(520));
    fixture.write(slot.cast(), Value::from_i64(61));
    assert_eq!(fixture.read(slot.cast()).try_as_i64(), Some(61));
}

#[test]
fn ordinary_tracked_value_slot_keeps_direct_read_write_behavior() {
    let mut fixture = Fixture::new();
    // Padding keeps the old header probe in bounds even for this positive.
    let backing = fixture.object(TypeId::UNIT, &[Value::from_i64(17); 4]);
    let slot = backing.data_ptr().cast::<Value>();
    fixture.state.cbgr_mutable_ptrs.insert(slot as usize);
    assert_eq!(fixture.read(slot.cast()).try_as_i64(), Some(17));
    fixture.write(slot.cast(), Value::from_i64(91));
    assert_eq!(fixture.read(slot.cast()).try_as_i64(), Some(91));
}

#[test]
fn genuine_shared_reads_and_writes_inner_scalar_without_changing_counts() {
    let mut fixture = Fixture::new();
    let (carrier, inner) = fixture.shared(Value::from_i64(520));
    // SAFETY: shared() returned a live three-slot bridge extent.
    assert_eq!(
        shared_inner_cell_result(&fixture.state, carrier.as_ptr().cast()).unwrap(),
        unsafe { inner.add(2) }
    );
    assert_eq!(
        fixture.read(carrier.as_ptr().cast()).try_as_i64(),
        Some(520)
    );
    fixture.write(carrier.as_ptr().cast(), Value::from_i64(35));
    assert_eq!(fixture.read(carrier.as_ptr().cast()).try_as_i64(), Some(35));
    // SAFETY: all slots belong to the live carrier/bridge allocations.
    unsafe {
        assert_eq!((*inner).try_as_i64(), Some(1));
        assert_eq!((*inner.add(1)).try_as_i64(), Some(0));
        assert_eq!(
            (*carrier.data_ptr().cast::<Value>()).try_as_i64(),
            Some(inner as i64)
        );
    }
    assert_eq!(carrier.type_id(), TypeId::SHARED);
}

#[test]
fn genuine_shared_whole_value_write_replaces_the_pointer_not_the_record() {
    let mut fixture = Fixture::new();
    let first = fixture.object(TypeId(900), &[Value::from_i64(11)]);
    let replacement = fixture.object(TypeId(900), &[Value::from_i64(22)]);
    let (carrier, _) = fixture.shared(Value::from_ptr(first.as_ptr()));
    fixture.write(
        carrier.as_ptr().cast(),
        Value::from_ptr(replacement.as_ptr()),
    );
    assert_eq!(
        fixture.read(carrier.as_ptr().cast()).bits(),
        Value::from_ptr(replacement.as_ptr()).bits()
    );
    assert_eq!(first.type_id(), TypeId(900));
    // SAFETY: each live record has its one initialized field.
    unsafe {
        assert_eq!((*first.data_ptr().cast::<Value>()).try_as_i64(), Some(11));
        assert_eq!(
            (*replacement.data_ptr().cast::<Value>()).try_as_i64(),
            Some(22)
        );
    }
}

#[test]
fn owned_non_shared_object_is_not_a_shared_carrier() {
    let mut fixture = Fixture::new();
    let (_, inner) = fixture.shared(Value::from_i64(77));
    let object = fixture.object(
        TypeId(900),
        &[Value::from_i64(inner as i64), Value::unit(), Value::unit()],
    );
    assert!(shared_inner_cell_result(&fixture.state, object.as_ptr().cast()).is_err());
}
