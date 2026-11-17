//! Control-flow admission remains structural even for unreachable instructions.
use super::InstructionFlow;
use verum_common::List;
use verum_vbc::{Instruction as I, Reg, VbcModule, instruction::CompareOp};

fn flow(instructions: &[I]) -> crate::llvm::Result<InstructionFlow> {
    InstructionFlow::build(&VbcModule::new("flow".into()), instructions)
}

#[test]
fn a_terminal_instruction_has_no_normal_successor() {
    for terminal in [
        I::RetV,
        I::Ret { value: Reg(0) },
        I::Panic { message_id: 0 },
        I::PanicValue { message: Reg(0) },
        I::Unreachable,
        I::Throw { error: Reg(0) },
    ] {
        let flow = flow(&[terminal, I::Nop]).unwrap();
        assert_eq!(flow.reachable.as_slice(), &[true, false, false]);
        assert_eq!(flow.starts.as_slice(), &[0]);
        assert!(!flow.entry_is_target);
    }
}

#[test]
fn recovery_and_conditional_edges_keep_both_possible_successors() {
    for branch in [
        I::TryBegin { handler_offset: 3 },
        I::JmpIf {
            cond: Reg(0),
            offset: 3,
        },
        I::JmpNot {
            cond: Reg(0),
            offset: 3,
        },
        I::JmpCmp {
            op: CompareOp::Lt,
            a: Reg(0),
            b: Reg(1),
            offset: 3,
        },
    ] {
        let flow = flow(&[branch, I::RetV, I::Nop, I::RetV]).unwrap();
        assert_eq!(flow.reachable.as_slice(), &[true, true, false, true, false]);
        assert!(flow.starts.contains(&1));
        assert!(flow.starts.contains(&3));
    }
}

#[test]
fn switches_visit_only_their_case_and_default_successors() {
    let flow = flow(&[
        I::Switch {
            value: Reg(0),
            default_offset: 4,
            cases: vec![(7, 2)],
        },
        I::Nop,
        I::RetV,
        I::Nop,
        I::RetV,
    ])
    .unwrap();
    assert_eq!(
        flow.reachable.as_slice(),
        &[true, false, true, false, true, false]
    );
    assert!(flow.starts.contains(&2));
    assert!(flow.starts.contains(&4));
}

#[test]
fn backward_edges_finish_without_recursion_or_revisiting_nodes() {
    let cycle = flow(&[I::Nop, I::Jmp { offset: -1 }, I::Nop]).unwrap();
    assert!(cycle.entry_is_target);
    assert_eq!(cycle.reachable.as_slice(), &[true, true, false, false]);
    let mut body = List::new();
    body.resize(100_000, I::Nop);
    body.push(I::Jmp { offset: -100_000 });
    let flow = flow(body.as_slice()).unwrap();
    assert_eq!(
        flow.reachable.iter().filter(|value| **value).count(),
        body.len()
    );
    assert!(!flow.reachable[body.len()]);
}

#[test]
fn invalid_targets_are_rejected_even_after_a_terminal_instruction() {
    for branch in [
        I::Jmp { offset: -2 },
        I::JmpIf {
            cond: Reg(0),
            offset: 2,
        },
        I::JmpNot {
            cond: Reg(0),
            offset: i32::MAX,
        },
        I::JmpCmp {
            op: CompareOp::Eq,
            a: Reg(0),
            b: Reg(1),
            offset: i32::MIN,
        },
        I::TryBegin { handler_offset: 2 },
        I::Switch {
            value: Reg(0),
            default_offset: 0,
            cases: vec![(7, 2)],
        },
    ] {
        let error = flow(&[I::RetV, branch]).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("control-flow target is outside the function"),
            "{error}"
        );
    }
}
