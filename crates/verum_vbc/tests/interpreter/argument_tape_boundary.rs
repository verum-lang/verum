//! Recycled callee slots must not retain an unrelated previous argument's tape.
use super::propagate_arg;
use crate::instruction::Reg;
use crate::interpreter::{autodiff::TensorId, state::InterpreterState};
use crate::module::VbcModule;
use verum_common::Shared;

#[test]
fn untracked_argument_clears_a_recycled_callee_node_with_the_same_value() {
    let mut state = InterpreterState::new(Shared::new(VbcModule::new("tape_boundary")).into_arc());
    state.grad_recording = true;
    state.grad_reg_nodes.insert(20, (TensorId(7), 3.0_f64.to_bits()));
    propagate_arg(&mut state, 0, Reg(1), 20, Reg(0));
    assert!(!state.grad_reg_nodes.contains_key(&20), "an absent source node cannot retain a prior frame node");
}

#[test]
fn tracked_argument_replaces_a_recycled_callee_node() {
    let mut state = InterpreterState::new(Shared::new(VbcModule::new("tape_boundary")).into_arc());
    state.grad_recording = true;
    state.grad_reg_nodes.insert(1, (TensorId(9), 3.0_f64.to_bits()));
    state.grad_reg_nodes.insert(20, (TensorId(7), 3.0_f64.to_bits()));
    propagate_arg(&mut state, 0, Reg(1), 20, Reg(0));
    assert_eq!(state.grad_reg_nodes.get(&20), Some(&(TensorId(9), 3.0_f64.to_bits())));
}
