//! DropRef distinguishes owned object bases from returned interior references.
//! Runtime controls use padded live allocations so baseline failures are wrong
//! state changes, not undefined reads. Source controls inspect actual wire code.

use super::cbgr::handle_drop_ref;
use super::cbgr_helpers::encode_cbgr_ref;
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
    allocations: List<i64>,
}

impl Fixture {
    fn new() -> Self {
        let mut module = VbcModule::new("drop_authority".into());
        let offset = module.append_bytecode(&[0]);
        let name = module.intern_string("drop_authority.probe");
        module.add_function(FunctionDescriptor {
            id: FunctionId(0),
            name,
            bytecode_offset: offset,
            bytecode_length: 1,
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
            allocations: List::new(),
        }
    }

    fn object(&mut self, type_id: TypeId, values: &[Value]) -> heap::Object {
        let object = self
            .state
            .heap
            .alloc_array(type_id, values.len())
            .expect("heap object");
        // SAFETY: allocation and initialized slice contain the same slot count.
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr(), object.data_ptr().cast(), values.len())
        };
        object
    }

    fn shared(&mut self, carrier_fields: usize) -> (heap::Object, *mut Value) {
        let address = cbgr_user_allocate(&mut self.state, (3 * VALUE_BYTES) as i64, 8);
        assert!(address > 0);
        self.allocations.push(address);
        let inner = address as *mut Value;
        // SAFETY: the actual allocator registered three complete Value slots.
        unsafe {
            inner.write(Value::from_i64(2));
            inner.add(1).write(Value::from_i64(1));
            inner.add(2).write(Value::from_i64(77));
        }
        let fields = [
            Value::from_i64(address),
            Value::from_i64(1),
            Value::from_i64(2),
        ];
        let carrier = self.object(TypeId::SHARED, &fields[..carrier_fields]);
        (carrier, inner)
    }

    fn decoy(&mut self, inner: *mut Value) -> *mut Value {
        // Complete initialized padding for the old false header and ptr read.
        let backing = self.object(
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
        // SAFETY: second of six initialized slots.
        let slot = unsafe { backing.data_ptr().cast::<Value>().add(1) };
        self.state.cbgr_mutable_ptrs.insert(slot as usize);
        slot
    }

    fn drop_value(&mut self, value: Value) {
        self.state.set_reg(Reg(0), value);
        self.drop_register();
    }

    fn drop_register(&mut self) {
        self.state.set_pc(0);
        handle_drop_ref(&mut self.state).expect("DropRef");
    }

    fn cleanup_state(&self) -> (bool, u32) {
        (
            self.state.get_reg(Reg(0)).is_unit(),
            self.state.registers.get_generation(0),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for address in self.allocations.iter().copied() {
            cbgr_user_deallocate(&mut self.state, address);
        }
    }
}

#[test]
fn interior_520_drop_preserves_unrelated_shared_count_and_cleans_register() {
    let mut fixture = Fixture::new();
    let (_, inner) = fixture.shared(3);
    let slot = fixture.decoy(inner);
    let generation = fixture.state.registers.get_generation(0);
    fixture.drop_value(Value::from_ptr(slot));
    // SAFETY: both pointers still belong to their live owned allocations.
    let (strong, value) = unsafe { ((*inner).as_i64(), (*slot).as_i64()) };
    assert_eq!(
        (strong, value, fixture.cleanup_state()),
        (2, 520, (true, generation + 1))
    );
    assert!(fixture.state.cbgr_mutable_ptrs.contains(&(slot as usize)));
}

#[test]
fn final_interior_slot_drop_does_not_need_a_following_header() {
    let mut fixture = Fixture::new();
    let (_, inner) = fixture.shared(3);
    let padded = fixture.decoy(inner);
    fixture.drop_value(Value::from_ptr(padded));
    // Fail the old implementation before testing a genuinely last Value slot.
    assert_eq!(
        unsafe { (*inner).as_i64() },
        2,
        "safe padded authority prerequisite"
    );
    let backing = fixture.object(TypeId::UNIT, &[Value::from_i64(520)]);
    let slot = backing.data_ptr().cast::<Value>();
    fixture.state.cbgr_mutable_ptrs.insert(slot as usize);
    let generation = fixture.state.registers.get_generation(0);
    fixture.drop_value(Value::from_ptr(slot));
    assert_eq!(fixture.cleanup_state(), (true, generation + 1));
    assert_eq!(unsafe { (*slot).as_i64() }, 520);
}

#[test]
fn genuine_shared_drop_decrements_once_and_finishes_register_cleanup() {
    for fields in [1, 3] {
        let mut fixture = Fixture::new();
        let (carrier, inner) = fixture.shared(fields);
        let generation = fixture.state.registers.get_generation(0);
        fixture.drop_value(Value::from_ptr(carrier.as_ptr()));
        assert_eq!(
            (unsafe { (*inner).as_i64() }, fixture.cleanup_state()),
            (1, (true, generation + 1))
        );
        fixture.drop_register();
        assert_eq!(
            (unsafe { (*inner).as_i64() }, fixture.cleanup_state()),
            (1, (true, generation + 2))
        );
        assert_eq!(unsafe { (*inner.add(1)).as_i64() }, 1);
        assert_eq!(unsafe { (*inner.add(2)).as_i64() }, 77);
    }
}

#[test]
fn ordinary_interior_drop_retains_slot_and_reference_cleanup() {
    let mut fixture = Fixture::new();
    let backing = fixture.object(TypeId::UNIT, &[Value::from_i64(17); 4]);
    let slot = backing.data_ptr().cast::<Value>();
    fixture.state.cbgr_mutable_ptrs.insert(slot as usize);
    let generation = fixture.state.registers.get_generation(0);
    fixture.drop_value(Value::from_ptr(slot));
    assert_eq!(fixture.cleanup_state(), (true, generation + 1));
    assert_eq!(unsafe { (*slot).as_i64() }, 17);
}

#[test]
fn register_reference_drop_does_not_invalidate_the_referent_register() {
    let mut fixture = Fixture::new();
    fixture.state.set_reg(Reg(1), Value::from_i64(520));
    let referent_generation = fixture.state.registers.get_generation(1);
    let dropped_generation = fixture.state.registers.get_generation(0);
    fixture.drop_value(encode_cbgr_ref(1, referent_generation));
    assert_eq!(fixture.cleanup_state(), (true, dropped_generation + 1));
    assert_eq!(
        fixture.state.registers.get_generation(1),
        referent_generation
    );
    assert_eq!(fixture.state.get_reg(Reg(1)).as_i64(), 520);
}

#[test]
fn short_shared_drop_does_not_follow_an_absent_pointer_field() {
    let mut fixture = Fixture::new();
    for size in 0..VALUE_BYTES {
        let (mut padded, inner) = fixture.shared(3);
        let allocated_size = padded.header().size;
        padded.header_mut().size = size as u32;
        fixture.drop_value(Value::from_ptr(padded.as_ptr()));
        padded.header_mut().size = allocated_size;
        assert_eq!(
            unsafe { (*inner).as_i64() },
            2,
            "declared pointer-field bound {size}"
        );
        assert!(fixture.state.get_reg(Reg(0)).is_unit());

        // The baseline fails above, before these actual short allocations.
        let short = fixture
            .state
            .heap
            .alloc(TypeId::SHARED, size)
            .expect("short carrier");
        let generation = fixture.state.registers.get_generation(0);
        fixture.drop_value(Value::from_ptr(short.as_ptr()));
        assert_eq!(fixture.cleanup_state(), (true, generation + 1));
    }
}

#[test]
fn invalid_shared_drop_does_not_touch_the_inner_count() {
    let mut fixture = Fixture::new();
    let (mut carrier, inner) = fixture.shared(3);
    let flags = carrier.header().flags;
    carrier.header_mut().flags.insert(heap::ObjectFlags::FREED);
    let generation = fixture.state.registers.get_generation(0);
    fixture.drop_value(Value::from_ptr(carrier.as_ptr()));
    carrier.header_mut().flags = flags;
    assert_eq!(
        (unsafe { (*inner).as_i64() }, fixture.cleanup_state()),
        (2, (true, generation + 1))
    );
}

#[cfg(feature = "codegen")]
fn emitted_probe(source: &str) -> List<crate::instruction::Instruction> {
    use crate::codegen::{CodegenConfig, VbcCodegen};
    let ast = verum_fast_parser::Parser::new(source)
        .parse_module()
        .expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("drop_authority"))
        .compile_module(&ast)
        .expect("source codegen");
    let bytes = crate::serialize::serialize_module(&module).expect("wire writer");
    let module = crate::deserialize::deserialize_module(&bytes).expect("wire reader");
    let entry = module
        .functions
        .iter()
        .find(|function| module.get_string(function.name) == Some("drop_authority.probe"))
        .expect("exact probe declaration");
    let mut offset = entry.bytecode_offset as usize;
    let end = offset + entry.bytecode_length as usize;
    let mut instructions = List::new();
    while offset < end {
        instructions.push(
            crate::bytecode::decode_instruction(&module.bytecode, &mut offset)
                .expect("wire instruction"),
        );
    }
    instructions
}

#[cfg(feature = "codegen")]
#[test]
fn a_returned_borrow_local_reaches_dropref_in_emitted_code() {
    use crate::instruction::Instruction;
    use verum_common::Set;
    let instructions = emitted_probe(
        r#"
module drop_authority;
fn borrowed(input: &Int) -> &Int { input }
fn probe(input: &Int) -> Int {
    let value: &Int = borrowed(input);
    *value
}
"#,
    );
    let mut returned = Set::new();
    let mut cleaned = false;
    for instruction in &instructions {
        match instruction {
            Instruction::Call { dst, .. } => {
                returned.insert(*dst);
            }
            Instruction::Mov { dst, src } if returned.contains(src) => {
                returned.insert(*dst);
            }
            Instruction::DropRef { src } if returned.contains(src) => {
                cleaned = true;
            }
            _ => {}
        }
    }
    assert!(
        cleaned,
        "returned reference must exercise register cleanup: {instructions:?}"
    );
}

#[cfg(feature = "codegen")]
#[test]
fn a_returned_maybe_borrow_payload_is_checked_and_read_in_emitted_code() {
    use crate::instruction::Instruction;
    use verum_common::Set;
    let instructions = emitted_probe(
        r#"
module drop_authority;
fn borrowed(input: &Int) -> Maybe<&Int> { Maybe.Some(input) }
fn probe(input: &Int) -> Int {
    match borrowed(input) { Maybe.Some(value) => *value, Maybe.None => 0 }
}
"#,
    );
    let mut payload = Set::new();
    let mut checked = Set::new();
    let mut loaded = Set::new();
    let mut returned_value = false;
    for instruction in &instructions {
        match instruction {
            Instruction::GetVariantData { dst, .. } => {
                payload.insert(*dst);
            }
            Instruction::Mov { dst, src } if payload.contains(src) => {
                payload.insert(*dst);
            }
            Instruction::ChkRef { ref_reg } if payload.contains(ref_reg) => {
                checked.insert(*ref_reg);
            }
            Instruction::Deref { dst, ref_reg } if checked.contains(ref_reg) => {
                loaded.insert(*dst);
            }
            Instruction::Mov { dst, src } if loaded.contains(src) => {
                loaded.insert(*dst);
            }
            Instruction::Ret { value } if loaded.contains(value) => {
                returned_value = true;
            }
            _ => {}
        }
    }
    // Pattern-bound borrowed payloads are checked and read, but this lowering
    // does not emit DropRef for the payload. The explicit local test above
    // pins the actual returned-reference path into the DropRef handler.
    assert!(
        returned_value,
        "borrowed payload must be checked before returning its value: {instructions:?}"
    );
}
