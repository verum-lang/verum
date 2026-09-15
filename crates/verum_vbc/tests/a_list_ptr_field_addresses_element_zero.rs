//! `List.ptr`, as compiled code sees it, is the address of element 0
//! (T1492).
//!
//! `core/collections/list.vr` declares slot 2 `ptr: &unsafe T` and every
//! body it has reads it that way — `iter`'s `self.ptr.offset(self.len)`
//! for the end sentinel, `unique`'s `&*self.ptr.offset(i)`,
//! `ListIter.next`'s `&*self.ptr`. The interpreter stores the backing
//! ALLOCATION there, whose first `OBJECT_HEADER_SIZE` bytes are an
//! `ObjectHeader`, and every intercept that reads the slot adds the skip
//! back by hand. Nothing reconciled the two, so `offset(0)` addressed
//! the header's `type_id` word and `offset(3)` addressed element 0 —
//! measured, by reading four bytes at each stride:
//!
//! ```text
//! slot0 = 512        (TypeId::LIST — the backing's own header)
//! slot1 = 65536
//! slot2 = 196608
//! slot3 = 7          <- element 0
//! slot4 = 7
//! slot5 = 9
//! ```
//!
//! `Text` had the convention right the whole time: `heap::alloc_text`
//! stores `Value::from_ptr(bytes_dst)` — the BYTES. This pin holds the
//! two containers to the one convention, end to end through the
//! bytecode: build a list, take its `ptr` the way a `.vr` body does,
//! and read the element the way `&*p` does.

use std::sync::Arc;

use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, MemSubOpcode, Reg};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::StringId;
use verum_vbc::value::Value;

fn write_reg(operands: &mut Vec<u8>, reg: u16) {
    if reg < 128 {
        operands.push(reg as u8);
    } else {
        operands.push(0x80 | ((reg >> 8) as u8));
        operands.push((reg & 0xFF) as u8);
    }
}

fn deref_value(dst: Reg, addr: Reg) -> Instruction {
    let mut operands = Vec::with_capacity(2);
    write_reg(&mut operands, dst.0);
    write_reg(&mut operands, addr.0);
    Instruction::MemExtended {
        sub_op: MemSubOpcode::DerefValue as u8,
        operands,
    }
}

fn run(instrs: &[Instruction]) -> Value {
    let mut bc = Vec::new();
    for instr in instrs {
        bytecode::encode_instruction(instr, &mut bc);
    }
    let mut module = VbcModule::new("list_ptr_element_zero_pin".to_string());
    let mut func = FunctionDescriptor::new(StringId::EMPTY);
    func.id = FunctionId(0);
    func.bytecode_offset = 0;
    func.bytecode_length = bc.len() as u32;
    func.register_count = 8;
    module.functions.push(func);
    module.bytecode = bc;

    let mut interp = Interpreter::new(Arc::new(module));
    interp
        .execute_function(FunctionId(0))
        .expect("list ptr execution failed")
}

/// `[7, 9]`, then `*list.ptr` — the shape `ListIter.next` is written in.
#[test]
fn the_ptr_field_dereferences_to_the_first_element() {
    let got = run(&[
        Instruction::NewList {
            dst: Reg(0),
            capacity_hint: 4,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 7,
        },
        Instruction::ListPush {
            list: Reg(0),
            val: Reg(1),
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 9,
        },
        Instruction::ListPush {
            list: Reg(0),
            val: Reg(1),
        },
        // `self.ptr` — slot 2 of the List header.
        Instruction::GetF {
            dst: Reg(2),
            obj: Reg(0),
            field_idx: 2,
        },
        deref_value(Reg(3), Reg(2)),
        Instruction::Ret { value: Reg(3) },
    ]);

    assert!(
        got.is_int(),
        "`*list.ptr` answered tag {:?} ({:#x}); an ObjectHeader word \
         means the field still addresses the backing allocation's base",
        got.tag(),
        got.to_bits()
    );
    assert_eq!(got.as_i64(), 7);
}

/// The stride the `.vr` bodies rely on: element `i` is `ptr + i * 8`,
/// with no header in between. Reading element 1 is the half that a
/// base-pointer answer can fake — at the base, `offset(1)` lands on the
/// header's generation word, which for a fresh heap is small and looks
/// like an answer.
#[test]
fn offsetting_the_ptr_field_walks_elements_not_header_words() {
    let mut operands = Vec::with_capacity(4);
    write_reg(&mut operands, Reg(4).0);
    write_reg(&mut operands, Reg(2).0);
    write_reg(&mut operands, Reg(5).0);
    let ptr_add = Instruction::MemExtended {
        sub_op: MemSubOpcode::PtrAdd as u8,
        operands,
    };

    let got = run(&[
        Instruction::NewList {
            dst: Reg(0),
            capacity_hint: 4,
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 7,
        },
        Instruction::ListPush {
            list: Reg(0),
            val: Reg(1),
        },
        Instruction::LoadI {
            dst: Reg(1),
            value: 9,
        },
        Instruction::ListPush {
            list: Reg(0),
            val: Reg(1),
        },
        Instruction::GetF {
            dst: Reg(2),
            obj: Reg(0),
            field_idx: 2,
        },
        Instruction::LoadI {
            dst: Reg(5),
            value: 1,
        },
        ptr_add,
        deref_value(Reg(3), Reg(4)),
        Instruction::Ret { value: Reg(3) },
    ]);

    assert!(
        got.is_int(),
        "`*(list.ptr + 1)` answered tag {:?} ({:#x})",
        got.tag(),
        got.to_bits()
    );
    assert_eq!(got.as_i64(), 9);
}
