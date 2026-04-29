//! Async and nursery operation handlers for VBC interpreter dispatch.
//!
//! Handles: Spawn (0xA0), Await (0xA1), Select (0xA3), Join (0xA4),
//! FutureReady (0xA5), FutureGet (0xA6), AsyncNext (0xA7),
//! NurseryInit (0xA8), NurserySpawn (0xA9), NurseryAwait (0xAA),
//! NurseryCancel (0xAB), NurseryConfig (0xAC), NurseryError (0xAD)

use crate::instruction::Reg;
use crate::module::FunctionId;
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::{InterpreterState, GeneratorId, TaskId, TaskStatus};
use super::super::DispatchResult;
use super::super::{call_closure_sync, execute_pending_task, alloc_list_from_values, dispatch_loop_table_with_entry_depth};
use super::bytecode_io::*;
use super::string_helpers::alloc_string_value;

// ============================================================================
// Task ID Encoding
// ============================================================================

/// Sentinel offset for task handle encoding.
/// Task IDs are stored as (TASK_ID_BASE - task_id) to produce large negative
/// values that never collide with regular computation results.
const TASK_ID_BASE: i64 = -0x5A5C_0000_0000_0000; // "TASC" in hex, always negative

/// Encode a TaskId as a sentinel-tagged i64 value.
pub(in super::super) fn encode_task_id(id: TaskId) -> i64 {
    TASK_ID_BASE - id.0 as i64
}

/// Try to decode a sentinel-tagged i64 value as a TaskId.
/// Returns None if the value is not a task handle.
pub(in super::super) fn decode_task_id(val: i64) -> Option<TaskId> {
    if val <= TASK_ID_BASE && val > TASK_ID_BASE - 1_000_000_000 {
        Some(TaskId((TASK_ID_BASE - val) as u64))
    } else {
        None
    }
}

// ============================================================================
// Async Operations
// ============================================================================

/// Spawn (0xA0) - Spawn an async task (deferred execution).
pub(in super::super) fn handle_spawn(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Honour the `[runtime].futures` feature gate from verum.toml.
    // When the user opted out of futures the dispatch handler must
    // refuse the operation rather than silently spawning a task the
    // host promised wouldn't run. Before this gate the
    // `futures_enabled` config field was inert — opt-out was
    // declared but unenforced.
    if !state.config.futures_enabled {
        return Err(InterpreterError::Panic {
            message:
                "futures disabled by [runtime].futures = false in verum.toml — \
                 spawn is unavailable in this build"
                    .to_string(),
        });
    }
    let dst = read_reg(state)?;
    let func_id_raw = read_varint(state)? as u32;
    let args = read_reg_range(state)?;
    let func_id = FunctionId(func_id_raw);

    // Collect arguments (register base may change when task runs later)
    let mut arg_values = Vec::with_capacity(args.count as usize);
    for i in 0..args.count {
        let reg = Reg(args.start.0 + i as u16);
        arg_values.push(state.get_reg(reg));
    }

    // Snapshot current context stack for the spawned task to inherit
    let parent_contexts = state.context_stack.clone_entries();

    // Register task in the queue -- deferred, NOT executed
    let task_id = state.tasks.spawn_deferred_with_contexts(func_id, arg_values, parent_contexts);

    state.set_reg(dst, Value::from_i64(encode_task_id(task_id)));

    Ok(DispatchResult::Continue)
}

/// Await (0xA1) - Await an async task.
pub(in super::super) fn handle_await(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let task_reg = read_reg(state)?;

    let task_val = state.get_reg(task_reg);

    // Check if the value is a task handle (sentinel-encoded from Spawn)
    if task_val.is_int()
        && let Some(task_id) = decode_task_id(task_val.as_i64()) {
            // If the task is already completed, return its result immediately
            if let Some(task) = state.tasks.get(task_id)
                && let Some(result) = task.result {
                    state.set_reg(dst, result);
                    return Ok(DispatchResult::Continue);
                }

            // Task is pending -- execute it now (cooperative scheduling)
            execute_pending_task(state, task_id)?;

            // Retrieve the result
            if let Some(task) = state.tasks.get(task_id)
                && let Some(result) = task.result {
                    state.set_reg(dst, result);
                    return Ok(DispatchResult::Continue);
                }
        }

    // Not a task handle -- value is a direct result
    state.set_reg(dst, task_val);

    Ok(DispatchResult::Continue)
}

/// Select (0xA3) - Wait for the first of multiple futures to complete.
pub(in super::super) fn handle_select(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let futures = read_reg_range(state)?;

    if futures.count == 0 {
        state.set_reg(dst, Value::unit());
        return Ok(DispatchResult::Continue);
    }

    // Collect task IDs from the future registers
    let mut task_entries: Vec<(Option<TaskId>, usize)> = Vec::new();
    for i in 0..futures.count {
        let reg = Reg(futures.start.0 + i as u16);
        let val = state.get_reg(reg);
        let tid = if val.is_int() { decode_task_id(val.as_i64()) } else { None };
        task_entries.push((tid, i as usize));
    }

    // Check if any task is already completed
    for &(tid_opt, _) in &task_entries {
        if let Some(tid) = tid_opt
            && let Some(task) = state.tasks.get(tid)
                && let Some(result) = task.result {
                    state.set_reg(dst, result);
                    return Ok(DispatchResult::Continue);
                }
    }

    // Execute pending tasks until one of our targets completes
    for &(tid_opt, _) in &task_entries {
        if let Some(tid) = tid_opt
            && state.tasks.get(tid).map(|t| t.status == TaskStatus::Pending).unwrap_or(false) {
                execute_pending_task(state, tid)?;
                if let Some(task) = state.tasks.get(tid)
                    && let Some(result) = task.result {
                        state.set_reg(dst, result);
                        return Ok(DispatchResult::Continue);
                    }
            }
    }

    // Fallback -- return first register value directly
    let first = state.get_reg(futures.start);
    state.set_reg(dst, first);

    Ok(DispatchResult::Continue)
}

/// Join (0xA4) - Wait for all tasks to complete.
pub(in super::super) fn handle_join(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let tasks = read_reg_range(state)?;

    // Collect decoded task IDs
    let mut task_entries: Vec<Option<TaskId>> = Vec::new();
    for i in 0..tasks.count {
        let reg = Reg(tasks.start.0 + i as u16);
        let val = state.get_reg(reg);
        let tid = if val.is_int() { decode_task_id(val.as_i64()) } else { None };
        task_entries.push(tid);
    }

    // Execute all pending tasks
    for &tid_opt in &task_entries {
        if let Some(tid) = tid_opt
            && state.tasks.get(tid).map(|t| t.status == TaskStatus::Pending).unwrap_or(false) {
                execute_pending_task(state, tid)?;
            }
    }

    // Collect results
    let mut results = Vec::with_capacity(tasks.count as usize);
    for (i, &tid_opt) in task_entries.iter().enumerate() {
        if let Some(tid) = tid_opt
            && let Some(task) = state.tasks.get(tid)
                && let Some(result) = task.result {
                    results.push(result);
                    continue;
                }
        // Fallback: use register value directly
        let reg = Reg(tasks.start.0 + i as u16);
        results.push(state.get_reg(reg));
    }

    let list_val = alloc_list_from_values(state, results)?;
    state.set_reg(dst, list_val);

    Ok(DispatchResult::Continue)
}

/// FutureReady (0xA5) - Check if a future is ready (non-blocking).
pub(in super::super) fn handle_future_ready(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let future_reg = read_reg(state)?;
    let val = state.get_reg(future_reg);

    let is_ready = if val.is_int() {
        if let Some(task_id) = decode_task_id(val.as_i64()) {
            state.tasks.get(task_id)
                .map(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed)
                .unwrap_or(true)
        } else {
            true
        }
    } else {
        true
    };

    state.set_reg(dst, Value::from_bool(is_ready));
    Ok(DispatchResult::Continue)
}

/// FutureGet (0xA6) - Get future result (blocking).
pub(in super::super) fn handle_future_get(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let future_reg = read_reg(state)?;
    let value = state.get_reg(future_reg);

    if value.is_int()
        && let Some(task_id) = decode_task_id(value.as_i64()) {
            // If completed, return result directly
            if let Some(task) = state.tasks.get(task_id)
                && let Some(result) = task.result {
                    state.set_reg(dst, result);
                    return Ok(DispatchResult::Continue);
                }

            // If pending, execute it
            if state.tasks.get(task_id).map(|t| t.status == TaskStatus::Pending).unwrap_or(false) {
                execute_pending_task(state, task_id)?;
                if let Some(task) = state.tasks.get(task_id)
                    && let Some(result) = task.result {
                        state.set_reg(dst, result);
                        return Ok(DispatchResult::Continue);
                    }
            }
        }

    // Not a task handle -- direct value pass-through
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// AsyncNext (0xA7) - Get next from async iterator.
pub(in super::super) fn handle_async_next(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    use super::super::super::state::GeneratorStatus;

    let dst = read_reg(state)?;
    let iter_reg = read_reg(state)?;
    let iter_value = state.get_reg(iter_reg);

    // Check if iterator is a generator
    if iter_value.is_generator() {
        let gen_id = GeneratorId(iter_value.as_generator_id());

        if !state.generators.get(gen_id).map(|g| g.can_resume()).unwrap_or(false) {
            state.set_reg(dst, Value::nil());
            return Ok(DispatchResult::Continue);
        }

        let (func_id, status, reg_count) = {
            let generator = state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
            (generator.func_id, generator.status, generator.reg_count)
        };

        let (resume_pc, restore_registers, restore_contexts) = match status {
            GeneratorStatus::Created => {
                let generator = state.generators.get(gen_id)
                    .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                (0u32, generator.saved_registers.clone(), Vec::new())
            }
            GeneratorStatus::Yielded => {
                let generator = state.generators.get(gen_id)
                    .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;
                (generator.saved_pc, generator.saved_registers.clone(), generator.saved_contexts.clone())
            }
            _ => {
                state.set_reg(dst, Value::nil());
                return Ok(DispatchResult::Continue);
            }
        };

        // Mark as Running
        if let Some(g) = state.generators.get_mut(gen_id) {
            g.status = GeneratorStatus::Running;
        }

        let entry_depth = state.call_stack.depth();
        let return_pc = state.pc();
        state.call_stack.push_frame(func_id, reg_count, return_pc, dst)?;
        state.registers.push_frame(reg_count);

        let new_reg_base = state.reg_base();
        for (i, val) in restore_registers.iter().enumerate() {
            state.registers.set(new_reg_base, Reg(i as u16), *val);
        }
        if !restore_contexts.is_empty() {
            state.context_stack.restore_entries(restore_contexts);
        }

        state.current_generator = Some(gen_id);
        state.set_pc(resume_pc);

        // Run generator until yield or completion
        let _result = dispatch_loop_table_with_entry_depth(state, entry_depth);

        // Check result
        if let Some(gen_ref) = state.generators.get(gen_id) {
            if gen_ref.status == GeneratorStatus::Yielded {
                if let Some(val) = gen_ref.yielded_value {
                    state.set_reg(dst, val);
                } else {
                    state.set_reg(dst, Value::nil());
                }
            } else {
                state.set_reg(dst, Value::nil());
            }
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }

    Ok(DispatchResult::Continue)
}

// ============================================================================
// Nursery Operations
// ============================================================================

/// NurseryInit (0xA8) - Initialize a new nursery scope.
pub(in super::super) fn handle_nursery_init(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Honour the `[runtime].nurseries` feature gate from verum.toml.
    // The nursery surface (init/spawn/await/cancel/config/error)
    // gates at construction so a single rejection covers every
    // nursery operation downstream — without a nursery handle the
    // other ops are unreachable. Before this gate the
    // `nurseries_enabled` config field was inert.
    if !state.config.nurseries_enabled {
        return Err(InterpreterError::Panic {
            message:
                "nurseries disabled by [runtime].nurseries = false in verum.toml — \
                 nursery construction is unavailable in this build"
                    .to_string(),
        });
    }
    let dst = read_reg(state)?;
    let nursery_id = state.nurseries.create();
    state.set_reg(dst, Value::from_i64(nursery_id as i64));
    Ok(DispatchResult::Continue)
}

/// NurserySpawn (0xA9) - Spawn a task into nursery (deferred execution).
pub(in super::super) fn handle_nursery_spawn(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let nursery_reg = read_reg(state)?;
    let task_reg = read_reg(state)?;

    let nursery_val = state.get_reg(nursery_reg);
    let task_val = state.get_reg(task_reg);
    let nursery_id = nursery_val.as_i64() as u64;

    if task_val.is_ptr() && !task_val.is_nil() {
        // Closure -- register for deferred execution
        if let Some(nursery) = state.nurseries.get_mut(nursery_id) {
            nursery.spawn_closure(task_val);
        }
    } else if task_val.is_int() {
        // Task ID from a previous Spawn -- link to this nursery
        let task_id = TaskId(task_val.as_i64() as u64);
        if let Some(task) = state.tasks.get(task_id) {
            let func_id = task.func_id;
            let result = task.result;
            let status = task.status;
            state.nurseries.spawn_task(nursery_id, func_id);
            if let Some(nursery) = state.nurseries.get_mut(nursery_id)
                && let Some(ntask) = nursery.tasks.last_mut() {
                    ntask.status = status;
                    ntask.result = result;
                }
        }
    }

    Ok(DispatchResult::Continue)
}

/// NurseryAwait (0xAA) - Wait for all nursery tasks to complete.
pub(in super::super) fn handle_nursery_await(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let nursery_reg = read_reg(state)?;
    let success_reg = read_reg(state)?;

    let nursery_val = state.get_reg(nursery_reg);
    let nursery_id = nursery_val.as_i64() as u64;

    // Collect pending closures from the nursery before executing
    let pending_closures: Vec<(usize, Value)> = if let Some(nursery) = state.nurseries.get(nursery_id) {
        nursery.tasks.iter().enumerate()
            .filter(|(_, t)| t.status == TaskStatus::Pending && t.closure_val.is_some())
            .filter_map(|(i, t)| t.closure_val.map(|v| (i, v)))
            .collect()
    } else {
        Vec::new()
    };

    // Execute each pending closure
    for (task_idx, closure_val) in pending_closures {
        match call_closure_sync(state, closure_val, &[]) {
            Ok(result) => {
                if let Some(nursery) = state.nurseries.get_mut(nursery_id)
                    && let Some(task) = nursery.tasks.get_mut(task_idx) {
                        task.status = TaskStatus::Completed;
                        task.result = Some(result);
                    }
            }
            Err(e) => {
                // Preserve error message instead of dropping it silently.
                // Store the formatted error as a Verum string value in both
                // task.error and nursery.accumulated_error for downstream inspection.
                let err_str = format!("{}", e);
                let err_value = alloc_string_value(state, &err_str)
                    .unwrap_or(Value::nil());
                if let Some(nursery) = state.nurseries.get_mut(nursery_id) {
                    if let Some(task) = nursery.tasks.get_mut(task_idx) {
                        task.status = TaskStatus::Failed;
                        task.error = Some(err_value);
                    }
                    nursery.accumulated_error = Some(err_value);
                }
            }
        }
    }

    // Also execute any tasks that were linked from Spawn (not closures)
    let linked_task_ids: Vec<(usize, FunctionId)> = if let Some(nursery) = state.nurseries.get(nursery_id) {
        nursery.tasks.iter().enumerate()
            .filter(|(_, t)| t.status == TaskStatus::Pending && t.closure_val.is_none())
            .map(|(i, t)| (i, t.func_id))
            .collect()
    } else {
        Vec::new()
    };

    for (task_idx, _func_id) in linked_task_ids {
        if let Some(nursery) = state.nurseries.get_mut(nursery_id)
            && let Some(task) = nursery.tasks.get_mut(task_idx) {
                task.status = TaskStatus::Completed;
                task.result = Some(Value::unit());
            }
    }

    let all_ok = if let Some(nursery) = state.nurseries.get(nursery_id) {
        !nursery.has_error()
    } else {
        true
    };

    state.set_reg(success_reg, Value::from_bool(all_ok));

    Ok(DispatchResult::Continue)
}

/// NurseryCancel (0xAB) - Cancel all tasks in nursery.
pub(in super::super) fn handle_nursery_cancel(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let nursery_reg = read_reg(state)?;

    let nursery_val = state.get_reg(nursery_reg);
    let nursery_id = nursery_val.as_i64() as u64;

    // Mark all pending tasks as cancelled with a descriptive error so the
    // parent nursery can distinguish cancellation from silent nil failures.
    let cancel_err = alloc_string_value(state, "nursery cancelled").unwrap_or(Value::nil());
    if let Some(nursery) = state.nurseries.get_mut(nursery_id) {
        for task in &mut nursery.tasks {
            if task.status == TaskStatus::Pending {
                task.status = TaskStatus::Failed;
                task.error = Some(cancel_err);
            }
        }
    }

    Ok(DispatchResult::Continue)
}

/// NurseryConfig (0xAC) - Configure nursery options.
pub(in super::super) fn handle_nursery_config(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let nursery_reg = read_reg(state)?;
    let config_reg = read_reg(state)?;

    let nursery_val = state.get_reg(nursery_reg);
    let config_val = state.get_reg(config_reg);
    let nursery_id = nursery_val.as_i64() as u64;
    let config_int = config_val.as_i64();

    if let Some(nursery) = state.nurseries.get_mut(nursery_id) {
        let config_type = (config_int >> 32) & 0xFF;
        let config_value = config_int & 0xFFFFFFFF;
        match config_type {
            0 => nursery.timeout_ms = config_value as u64,
            1 => nursery.max_tasks = config_value as u64,
            _ => {} // Unknown config -- ignore
        }
    }

    Ok(DispatchResult::Continue)
}

/// NurseryError (0xAD) - Get nursery error (if any task failed).
pub(in super::super) fn handle_nursery_error(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let nursery_reg = read_reg(state)?;

    let nursery_val = state.get_reg(nursery_reg);
    let nursery_id = nursery_val.as_i64() as u64;

    if let Some(nursery) = state.nurseries.get(nursery_id) {
        if let Some(err_val) = nursery.accumulated_error {
            state.set_reg(dst, err_val);
        } else {
            state.set_reg(dst, Value::nil());
        }
    } else {
        state.set_reg(dst, Value::nil());
    }

    Ok(DispatchResult::Continue)
}
