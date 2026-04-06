//! Generator instruction handlers for VBC interpreter.

use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::{InterpreterState, GeneratorId, GeneratorStatus};
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Generator Operations
// ============================================================================

pub(in super::super) fn handle_generator_create(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Create generator: dst = gen_create(func_id, args...)
    //
    // Creates a new generator instance from a generator function (fn*).
    // The generator starts in Created state and can be iterated via GenNext.
    // Arguments are stored in the generator's initial register state for use when
    // first resumed via GenNext.
    let dst = read_reg(state)?;
    let func_id = FunctionId(read_varint(state)? as u32);
    let args = read_reg_range(state)?;

    // Collect argument values from registers
    let mut initial_args = Vec::with_capacity(args.count as usize);
    for i in 0..args.count {
        let arg_reg = Reg(args.start.0 + i as u16);
        initial_args.push(state.get_reg(arg_reg));
    }

    // Create the generator in the registry with initial arguments
    let func = state.module.get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    let reg_count = func.register_count;
    let gen_id = state.generators.create_with_args(func_id, reg_count, initial_args);

    // Return generator as a Value
    state.set_reg(dst, Value::from_generator(gen_id.0));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_generator_yield(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Yield from generator: yield value
    //
    // This suspends the current generator and returns control to the caller
    // with the yielded value. The generator can be resumed later.
    let value_reg = read_reg(state)?;
    let value = state.get_reg(value_reg);

    // Check if we're executing within a generator context
    if let Some(gen_id) = state.current_generator {
        // Collect state information before borrowing generators mutably
        let saved_pc = state.pc();
        let saved_reg_base = state.reg_base();
        let saved_contexts = state.context_stack.clone_entries();

        // Get register count from generator (read-only first)
        let reg_count = state.generators.get(gen_id)
            .map(|g| g.reg_count as usize)
            .unwrap_or(0);

        // Save all registers
        let mut saved_registers = Vec::with_capacity(reg_count);
        for i in 0..reg_count {
            let reg_value = state.registers.get(
                saved_reg_base,
                Reg(i as u16),
            );
            saved_registers.push(reg_value);
        }

        // Now update the generator state
        if let Some(g) = state.generators.get_mut(gen_id) {
            g.saved_pc = saved_pc;
            g.saved_reg_base = saved_reg_base;
            g.saved_registers = saved_registers;
            g.saved_contexts = saved_contexts;
            g.yielded_value = Some(value);
            g.status = GeneratorStatus::Yielded;
        }

        // Pop the generator frame
        let frame = state.call_stack.pop_frame()?;
        state.registers.pop_frame(frame.reg_base);

        // Clear current generator
        state.current_generator = None;

        // Return the yielded value to the caller
        return Ok(DispatchResult::Yield(value));
    }

    // Not in a generator context - error
    Err(InterpreterError::NotImplemented {
        feature: "yield outside generator context",
        opcode: Some(crate::instruction::Opcode::Yield),
    })
}

pub(in super::super) fn handle_generator_next(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Get next value: dst = gen_next(generator)
    //
    // Resumes the generator and returns the next yielded value.
    // Returns Some(value) if yielded, None if completed.
    let dst = read_reg(state)?;
    let gen_reg = read_reg(state)?;

    let gen_val = state.get_reg(gen_reg);
    if !gen_val.is_generator() {
        return Err(InterpreterError::TypeMismatch {
            expected: "Generator",
            got: "other",
            operation: "GenNext",
        });
    }

    let gen_id = GeneratorId(gen_val.as_generator_id());

    // Check generator status
    let (func_id, status, reg_count) = {
        let generator = state.generators.get(gen_id)
            .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;

        if generator.is_completed() {
            // Return None - generator exhausted
            state.set_reg(dst, Value::nil());
            return Ok(DispatchResult::Continue);
        }

        (generator.func_id, generator.status, generator.reg_count)
    };

    // Get function info
    let func = state.module.get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    let bytecode_offset = func.bytecode_offset;

    // Check if we need to restore state from a previous yield
    let (resume_pc, restore_registers, restore_contexts): (u32, Vec<Value>, Vec<super::super::super::state::ContextEntry>) = match status {
        GeneratorStatus::Created => {
            // First resume - start at function beginning
            // Restore initial arguments that were passed to the generator at creation time
            let generator = state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
            let initial_args = generator.saved_registers.clone();
            (bytecode_offset, initial_args, Vec::new())
        }
        GeneratorStatus::Yielded => {
            // Resume from saved state
            let generator = state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
            let resume_pc = if generator.saved_pc > 0 { generator.saved_pc } else { bytecode_offset };
            let restore_registers = generator.saved_registers.clone();
            let restore_contexts = generator.saved_contexts.clone();
            (resume_pc, restore_registers, restore_contexts)
        }
        GeneratorStatus::Running => {
            // Generator is already running - invalid state
            return Err(InterpreterError::GeneratorNotResumable {
                generator_id: gen_id,
                status: "Running",
            });
        }
        GeneratorStatus::Completed => {
            state.set_reg(dst, Value::nil());
            return Ok(DispatchResult::Continue);
        }
    };

    // Push generator frame
    state.call_stack.push_frame(func_id, reg_count, resume_pc, dst)?;
    state.registers.push_frame(reg_count);

    // Restore registers if resuming from yield
    let new_reg_base = state.reg_base();
    for (i, val) in restore_registers.iter().enumerate() {
        state.registers.set(new_reg_base, Reg(i as u16), *val);
    }

    // Restore contexts if resuming from yield
    if !restore_contexts.is_empty() {
        state.context_stack.restore_entries(restore_contexts);
    }

    // Mark generator as running
    state.current_generator = Some(gen_id);

    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_generator_has_next(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Check if generator has more: dst = gen_has_next(generator)
    //
    // Returns true if the generator can produce more values, false otherwise.
    let dst = read_reg(state)?;
    let gen_reg = read_reg(state)?;

    let gen_val = state.get_reg(gen_reg);
    if !gen_val.is_generator() {
        return Err(InterpreterError::TypeMismatch {
            expected: "Generator",
            got: "other",
            operation: "GenHasNext",
        });
    }

    let gen_id = GeneratorId(gen_val.as_generator_id());
    let has_next = state.generators.get(gen_id)
        .map(|g| g.can_resume())
        .unwrap_or(false);

    state.set_reg(dst, Value::from_bool(has_next));
    Ok(DispatchResult::Continue)
}

