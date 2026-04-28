//! VBC Interpreter - register-based virtual machine.
//!
//! The VBC interpreter provides execution of VBC bytecode with:
//! - **Fast startup**: Minimal initialization overhead
//! - **NaN-boxing**: Compact 64-bit value representation
//! - **Register-based execution**: No operand stack
//! - **CBGR integration**: Memory safety through runtime checks
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     INTERPRETER ENGINE                          │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                 │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
//! │  │ RegisterFile │  │  CallStack   │  │       Heap           │  │
//! │  │  [r0..r255]  │  │ CallFrame[]  │  │ Object allocation    │  │
//! │  └──────────────┘  └──────────────┘  └──────────────────────┘  │
//! │                                                                 │
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │                    DISPATCH LOOP                         │   │
//! │  │  while pc < bytecode.len() {                            │   │
//! │  │    let op = bytecode[pc];                               │   │
//! │  │    pc += 1;                                             │   │
//! │  │    match op { ... }                                     │   │
//! │  │  }                                                       │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! │                                                                 │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Performance Targets
//!
//! | Operation | Target | Notes |
//! |-----------|--------|-------|
//! | Arithmetic | 100M ops/sec | Integer add loop |
//! | Function call | 10M calls/sec | Non-generic |
//! | Object alloc | 1M allocs/sec | Small objects |
//!
//! # Example
//!
//! ```ignore
//! use verum_vbc::interpreter::Interpreter;
//! use verum_vbc::VbcModule;
//!
//! let module = VbcModule::new("example".to_string());
//! let mut interp = Interpreter::new(&module);
//! let result = interp.execute_function(FunctionId(0))?;
//! ```

mod registers;
mod stack;
mod state;
mod dispatch_table;
mod heap;
mod cbgr_heap;
mod error;
pub mod permission;
pub mod tensor;
pub mod autodiff;
pub mod kernel;
pub mod gpu_simulator;

pub use registers::RegisterFile;
pub use stack::{CallStack, CallFrame};
pub use state::{
    InterpreterState, InterpreterConfig, ExecutionStats, CbgrStats, ContextStack, TaskQueue,
    // Generator system: fn* functions with Yield/GenCreate/GenNext/GenHasNext opcodes
    Generator, GeneratorId, GeneratorRegistry, GeneratorStatus, GeneratorStats,
    // Exception handling
    ExceptionHandler, ExceptionHandlerStack,
    // PrecisionMode: controls floating-point precision (Float32/Float64/Float128) and rounding
    PrecisionMode, FloatPrecision, RoundingMode,
    CTX_TYPE_PRECISION_MODE, CTX_TYPE_COMPUTE_DEVICE, CTX_TYPE_GRADIENT_TAPE,
    CTX_TYPE_PARALLEL_CONFIG, CTX_TYPE_MEMORY_POOL, CTX_TYPE_USER_START,
};
// Function table dispatch (faster, ~30-50% throughput improvement)
pub use dispatch_table::{dispatch_loop_table, dispatch_loop_table_with_entry_depth, DispatchResult};
pub use heap::{Heap, Object, ObjectHeader, ObjectFlags, HeapStats, OBJECT_HEADER_SIZE};
pub use cbgr_heap::{CbgrHeap, CbgrObject, CbgrHeapStats, CbgrObjectFlags, CbgrObjectRef};
// Permission router for intrinsic gating (#12 / P3.2).
pub use permission::{
    PermissionDecision, PermissionRouter, PermissionRouterStats, PermissionScope,
    PermissionTargetId,
};

/// Executes a function using table-based dispatch.
///
/// This is ~40% faster than the match-based dispatch due to:
/// - O(1) opcode lookup via array indexing
/// - Better branch prediction for indirect calls
/// - Reduced code size improving instruction cache utilization
///
/// This is the default dispatch method in VBC.
///
/// # Arguments
///
/// * `state` - The interpreter state
/// * `func_id` - The function to execute
///
/// # Returns
///
/// The return value of the executed function
///
/// # Errors
///
/// Returns `ModuleNotInterpretable` if the module has the `NOT_INTERPRETABLE` flag set.
/// Systems profile modules are NOT interpretable - VBC serves only as intermediate IR
/// for AOT compilation.
///
/// V-LLSI architecture: only Application/Research profile modules are interpretable.
/// Systems profile modules use VBC as intermediate IR for AOT compilation only.
pub fn execute_table(state: &mut InterpreterState, func_id: FunctionId) -> InterpreterResult<Value> {
    use crate::instruction::Reg;

    // Check if module is interpretable (V-LLSI architecture check)
    // Systems profile modules are NOT interpretable - VBC is intermediate IR only
    if !state.module.header.flags.is_interpretable() {
        return Err(InterpreterError::ModuleNotInterpretable {
            module_name: state.module.name.clone(),
            reason: if state.module.header.flags.is_systems_profile() {
                "Systems profile code is AOT-only"
            } else if state.module.header.flags.is_embedded() {
                "Embedded target code is AOT-only"
            } else {
                "Module marked as not interpretable"
            },
        });
    }

    // Get function descriptor
    let func = state
        .module
        .get_function(func_id)
        .ok_or(InterpreterError::FunctionNotFound(func_id))?;
    // Push initial frame
    let reg_count = func.register_count;
    let _base = state.call_stack.push_frame(
        func_id,
        reg_count,
        0, // No return pc for initial call
        Reg(0),
    )?;

    // Allocate registers
    state.registers.push_frame(reg_count);

    // Run table-based dispatch loop
    dispatch_table::dispatch_loop_table(state)
}
pub use cbgr_heap::{ObjectMeta, OBJECT_META_SIZE};
pub use error::{InterpreterError, InterpreterResult, CbgrViolationKind};

use crate::module::{FunctionId, VbcModule};
use crate::value::Value;

use std::sync::Arc;

/// Main interpreter entry point.
///
/// The `Interpreter` manages execution state and provides a high-level
/// interface for running VBC bytecode.
pub struct Interpreter {
    /// Execution state.
    pub state: InterpreterState,
}

impl Interpreter {
    /// Creates a new interpreter for the given module.
    ///
    /// # Panics
    ///
    /// Panics if the module has the `NOT_INTERPRETABLE` flag set.
    /// Systems profile modules are NOT interpretable - VBC serves only as
    /// intermediate IR for AOT compilation. Use `try_new()` for fallible construction.
    ///
    /// V-LLSI: panics if module has NOT_INTERPRETABLE flag (Systems/embedded profiles).
    pub fn new(module: Arc<VbcModule>) -> Self {
        Self::try_new(module).expect("Module is not interpretable")
    }

    /// Creates a new interpreter for the given module, returning an error if
    /// the module is not interpretable.
    ///
    /// # Errors
    ///
    /// Returns `ModuleNotInterpretable` if the module has the `NOT_INTERPRETABLE`
    /// flag set. Systems profile modules are NOT interpretable - VBC serves only
    /// as intermediate IR for AOT compilation.
    ///
    /// V-LLSI: returns ModuleNotInterpretable error for Systems/embedded profiles.
    pub fn try_new(module: Arc<VbcModule>) -> InterpreterResult<Self> {
        // Check if module is interpretable (V-LLSI architecture check)
        if !module.header.flags.is_interpretable() {
            return Err(InterpreterError::ModuleNotInterpretable {
                module_name: module.name.clone(),
                reason: if module.header.flags.is_systems_profile() {
                    "Systems profile code is AOT-only"
                } else if module.header.flags.is_embedded() {
                    "Embedded target code is AOT-only"
                } else {
                    "Module marked as not interpretable"
                },
            });
        }

        Ok(Self {
            state: InterpreterState::new(module),
        })
    }

    /// Creates a new interpreter for the given module **after** running
    /// the per-instruction bytecode validator.
    ///
    /// This is the secure-default constructor for any module that
    /// did NOT come from this process's own compiler — downloaded
    /// modules, archives shared across processes, files edited by
    /// hand, network-loaded bytecode.  The validator walks every
    /// function's bytecode and rejects out-of-range cross-references,
    /// register-bounds violations, branch offsets landing mid-
    /// instruction, and call-arity mismatches.  Cost is O(N) in
    /// total instruction count.
    ///
    /// `try_new` (the non-validating constructor) is preserved for
    /// trusted-source loads where the validator's walk is wasted
    /// work — for example, the in-process compiler emitting bytecode
    /// it just produced.
    ///
    /// # Errors
    ///
    /// * `ModuleNotInterpretable` — propagated from `try_new`.
    /// * `ValidationFailed { module_name, reason }` — the bytecode
    ///   validator surfaced a typed error.  The `reason` string is
    ///   the rendered `VbcError`.
    pub fn try_new_validated(module: Arc<VbcModule>) -> InterpreterResult<Self> {
        // Run the validator BEFORE the interpretable-flag check so
        // the user gets a load-time validation failure on a corrupt
        // module even if the flag would have rejected it for a
        // different reason.  In practice both surfaces are early-
        // exit; ordering here is only relevant when both apply.
        if let Err(err) = crate::validate::validate_module(&module) {
            return Err(InterpreterError::ValidationFailed {
                module_name: module.name.clone(),
                reason: err.to_string(),
            });
        }
        Self::try_new(module)
    }

    /// Executes a function by ID and returns the result.
    ///
    /// Executes a function by ID using function pointer table dispatch.
    ///
    /// This is ~40% faster than match-based dispatch due to:
    /// - O(1) opcode lookup via array indexing
    /// - Better branch prediction for indirect calls
    /// - Reduced code size improving instruction cache utilization
    pub fn execute_function(&mut self, func_id: FunctionId) -> InterpreterResult<Value> {
        execute_table(&mut self.state, func_id)
    }

    /// Executes the main function (function 0) if it exists.
    ///
    /// Runs `module.global_ctors` in priority order before `main`, matching
    /// the AOT path (which emits an LLVM `@llvm.global_ctors` array whose
    /// entries are invoked by the C runtime prior to `main`). This is
    /// required for `@thread_local static` initializers (which compile to
    /// `__tls_init_<NAME>` synthetic functions via
    /// `codegen::compile_pending_tls_inits`) to populate their TLS slots
    /// before user code reads them. Without it, `TlsGet` on an
    /// uninitialized slot falls back to `Value::default()`, which is not
    /// the declared static's initial value — e.g. `static mut LOCAL_HEAP:
    /// Maybe<LocalHeap> = None` reads back as a raw zero `Value` instead
    /// of the `None` variant, and the CBGR allocator bootstrap crashes on
    /// the first `Shared::new(...)`.
    pub fn run_main(&mut self) -> InterpreterResult<Value> {
        self.run_global_ctors()?;
        self.execute_function(FunctionId(0))
    }

    /// Executes the subset of `module.global_ctors` that initialise
    /// `@thread_local static` slots (functions named `__tls_init_*`).
    ///
    /// Why restricted to TLS inits: historically `global_ctors` also
    /// contains declared-only FFI library initializers (e.g. Windows
    /// `kernel32` startup functions) that panic via debug_assert! inside
    /// `value.rs` when invoked from the interpreter on macOS. Those were
    /// intentionally skipped in `pipeline::phase_interpret` (commit
    /// 4e3e4f5). However skipping *all* ctors broke `@thread_local`
    /// initializers — `__tls_init_<NAME>` synthetic functions are the
    /// only way a `@thread_local static`'s declared initial value
    /// reaches its TLS slot, and without them `TlsGet` on an unset slot
    /// yields `Value::default()`. The CBGR allocator's LOCAL_HEAP /
    /// CURRENT_HEAP bootstrap then reads a raw zero `Value` as
    /// `Maybe<LocalHeap>` and its pattern-match misfires, causing
    /// `Shared::new(...)` to crash with "Expected int, got None".
    ///
    /// Call this before executing user code. Running TLS inits is
    /// idempotent — each ctor re-executes and re-writes its slot — so
    /// callers do not need to track whether they have been run.
    pub fn run_global_ctors(&mut self) -> InterpreterResult<()> {
        if self.state.module.global_ctors.is_empty() {
            return Ok(());
        }
        let mut ctors: Vec<(u32, FunctionId)> = self
            .state
            .module
            .global_ctors
            .iter()
            .map(|(id, prio)| (*prio, *id))
            .collect();
        ctors.sort_by_key(|(prio, _)| *prio);
        for (_prio, ctor) in ctors {
            let is_tls_init = self
                .state
                .module
                .get_function(ctor)
                .and_then(|desc| self.state.module.get_string(desc.name))
                .map(|name| name.starts_with("__tls_init_"))
                .unwrap_or(false);
            if is_tls_init {
                self.execute_function(ctor)?;
            }
        }
        Ok(())
    }

    /// Calls a function with the given arguments.
    pub fn call(&mut self, func_id: FunctionId, args: &[Value]) -> InterpreterResult<Value> {
        // Push arguments to registers
        let frame = self.state.call_stack.push_frame(
            func_id,
            args.len() as u16 + 16, // args + locals
            0,
            crate::instruction::Reg(0),
        )?;

        // Copy arguments
        for (i, arg) in args.iter().enumerate() {
            self.state.registers.set(frame, crate::instruction::Reg(i as u16), *arg);
        }

        // Execute using table dispatch
        execute_table(&mut self.state, func_id)
    }

    /// Returns a reference to the current module.
    pub fn module(&self) -> &VbcModule {
        &self.state.module
    }

    /// Resets the interpreter state.
    pub fn reset(&mut self) {
        self.state.reset();
    }

    // =========================================================================
    // Value Creation API (for host-side value construction)
    // =========================================================================

    /// Allocates a string on the interpreter heap and returns it as a Value.
    pub fn alloc_string(&mut self, s: &str) -> InterpreterResult<Value> {
        if let Some(small) = Value::from_small_string(s) {
            return Ok(small);
        }
        let bytes = s.as_bytes();
        let len = bytes.len();
        let alloc_size = 8 + len;
        let obj = self.state.heap.alloc(crate::types::TypeId(0x0001), alloc_size)?;
        self.state.record_allocation();
        let base_ptr = obj.as_ptr() as *mut u8;
        unsafe {
            let data_offset = heap::OBJECT_HEADER_SIZE;
            let len_ptr = base_ptr.add(data_offset) as *mut u64;
            *len_ptr = len as u64;
            let bytes_ptr = base_ptr.add(data_offset + 8);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bytes_ptr, len);
        }
        Ok(Value::from_ptr(base_ptr))
    }

    /// Allocates a List<Text> from Rust strings and returns it as a Value.
    pub fn alloc_string_list(&mut self, strings: &[String]) -> InterpreterResult<Value> {
        let count = strings.len();
        // Allocate each string first
        let mut elements = Vec::with_capacity(count);
        for s in strings {
            elements.push(self.alloc_string(s)?);
        }
        // Allocate list header: [length, capacity, backing_ptr]
        let header_size = 3 * std::mem::size_of::<i64>();
        let obj = self.state.heap.alloc(crate::types::TypeId::LIST, header_size)?;
        self.state.record_allocation();
        let data_ptr = unsafe {
            (obj.as_ptr() as *mut u8).add(heap::OBJECT_HEADER_SIZE) as *mut i64
        };
        // Allocate backing array
        let backing_layout = std::alloc::Layout::from_size_align(
            count.max(1) * std::mem::size_of::<Value>(), 8
        ).map_err(|_| InterpreterError::Panic {
            message: "args list layout overflow".into(),
        })?;
        let backing_ptr = unsafe { std::alloc::alloc_zeroed(backing_layout) };
        if backing_ptr.is_null() && count > 0 {
            return Err(InterpreterError::Panic {
                message: "args list allocation failed".into(),
            });
        }
        // Fill backing array
        let value_ptr = backing_ptr as *mut Value;
        for (i, val) in elements.iter().enumerate() {
            unsafe { *value_ptr.add(i) = *val };
        }
        // Write header: [length, capacity, backing_ptr]
        unsafe {
            *data_ptr = count as i64;
            *data_ptr.add(1) = count as i64;
            *data_ptr.add(2) = backing_ptr as i64;
        }
        Ok(Value::from_ptr(obj.as_ptr() as *mut u8))
    }

    // =========================================================================
    // Generator API
    // =========================================================================
    //
    // Generator API: fn* functions produce values lazily via Yield. Each generator
    // maintains saved_pc, saved_registers, saved_contexts, and status (Created/Running/
    // Yielded/Completed). GenCreate allocates a Generator, GenNext resumes it,
    // GenHasNext checks if it can produce more values.

    /// Creates a new generator from a generator function.
    ///
    /// The generator is created in the Created state and must be resumed
    /// to begin execution.
    ///
    /// # Arguments
    /// * `func_id` - The generator function (fn*)
    ///
    /// # Returns
    /// The generator ID that can be used to resume the generator.
    pub fn create_generator(&mut self, func_id: FunctionId) -> InterpreterResult<GeneratorId> {
        let func = self.state.module.get_function(func_id)
            .ok_or(InterpreterError::FunctionNotFound(func_id))?;

        let reg_count = func.register_count;
        let gen_id = self.state.generators.create(func_id, reg_count);

        Ok(gen_id)
    }

    /// Resumes a suspended generator, returning the next yielded value.
    ///
    /// This implements the Iterator::next() protocol:
    /// - Returns `Some(value)` if the generator yields a value
    /// - Returns `None` if the generator is completed
    ///
    /// # Arguments
    /// * `gen_id` - The generator to resume
    ///
    /// # Returns
    /// - `Ok(Some(value))` - Generator yielded a value
    /// - `Ok(None)` - Generator is completed
    /// - `Err(...)` - An error occurred during execution
    pub fn resume_generator(&mut self, gen_id: GeneratorId) -> InterpreterResult<Option<Value>> {
        // Check generator status
        let (func_id, status, reg_count) = {
            let generator = self.state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;

            if generator.is_completed() {
                return Ok(None);
            }

            (generator.func_id, generator.status, generator.reg_count)
        };

        // Set current generator for yield handling
        self.state.current_generator = Some(gen_id);

        match status {
            GeneratorStatus::Created => {
                // First resume - start fresh execution
                let result = self.execute_generator_start(func_id, reg_count);
                self.handle_generator_result(gen_id, result)
            }
            GeneratorStatus::Yielded => {
                // Resume from suspended state
                let result = self.execute_generator_resume(gen_id);
                self.handle_generator_result(gen_id, result)
            }
            GeneratorStatus::Running => {
                // Already running - invalid state
                self.state.current_generator = None;
                Err(InterpreterError::GeneratorNotResumable {
                    generator_id: gen_id,
                    status: "running",
                })
            }
            GeneratorStatus::Completed => {
                // Already completed
                self.state.current_generator = None;
                Ok(None)
            }
        }
    }

    /// Starts a generator from the beginning.
    fn execute_generator_start(
        &mut self,
        func_id: FunctionId,
        reg_count: u16,
    ) -> InterpreterResult<Value> {
        // Push frame for the generator
        let _base = self.state.call_stack.push_frame(
            func_id,
            reg_count,
            0,
            crate::instruction::Reg(0),
        )?;

        // Allocate registers
        self.state.registers.push_frame(reg_count);

        // Update generator status to Running
        if let Some(gen_id) = self.state.current_generator
            && let Some(g) = self.state.generators.get_mut(gen_id) {
                g.status = GeneratorStatus::Running;
            }

        // Run dispatch loop (will return on yield or completion)
        dispatch_loop_table(&mut self.state)
    }

    /// Resumes a generator from its saved state.
    fn execute_generator_resume(&mut self, gen_id: GeneratorId) -> InterpreterResult<Value> {
        // Restore generator state
        let (func_id, saved_pc, saved_reg_base, saved_registers, saved_contexts) = {
            let generator = self.state.generators.get(gen_id)
                .ok_or(InterpreterError::InvalidGeneratorId { generator_id: gen_id })?;

            (
                generator.func_id,
                generator.saved_pc,
                generator.saved_reg_base,
                generator.saved_registers.clone(),
                generator.saved_contexts.clone(),
            )
        };

        // Get function info
        let func = self.state.module.get_function(func_id)
            .ok_or(InterpreterError::FunctionNotFound(func_id))?;

        // Push frame at the saved position
        let _base = self.state.call_stack.push_frame(
            func_id,
            func.register_count,
            0,
            crate::instruction::Reg(0),
        )?;

        // Allocate registers
        self.state.registers.push_frame(func.register_count);

        // Restore register values
        for (i, value) in saved_registers.iter().enumerate() {
            self.state.registers.set(
                saved_reg_base,
                crate::instruction::Reg(i as u16),
                *value,
            );
        }

        // Restore context entries
        self.state.context_stack.restore_entries(saved_contexts);

        // Set PC to resume point
        self.state.set_pc(saved_pc);

        // Update generator status to Running
        if let Some(g) = self.state.generators.get_mut(gen_id) {
            g.status = GeneratorStatus::Running;
        }

        // Continue dispatch loop
        dispatch_loop_table(&mut self.state)
    }

    /// Handles the result of generator execution.
    fn handle_generator_result(
        &mut self,
        gen_id: GeneratorId,
        result: InterpreterResult<Value>,
    ) -> InterpreterResult<Option<Value>> {
        // Clear current generator
        self.state.current_generator = None;

        match result {
            Ok(value) => {
                // Check if generator yielded or completed
                if let Some(g) = self.state.generators.get(gen_id) {
                    if g.status == GeneratorStatus::Yielded {
                        // Yielded - return the value
                        Ok(Some(value))
                    } else {
                        // Normal return - generator completed
                        self.state.generators.complete(gen_id, Some(value));
                        Ok(None)
                    }
                } else {
                    Ok(Some(value))
                }
            }
            Err(e) => {
                // Mark generator as completed (with error)
                self.state.generators.complete(gen_id, None);
                Err(e)
            }
        }
    }

    /// Returns true if a generator can produce more values.
    pub fn generator_has_next(&self, gen_id: GeneratorId) -> bool {
        self.state.generators.get(gen_id)
            .map(|g| g.can_resume())
            .unwrap_or(false)
    }

    /// Returns generator statistics.
    pub fn generator_stats(&self) -> GeneratorStats {
        self.state.generators.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpreter_creation() {
        let module = Arc::new(VbcModule::new("test".to_string()));
        let interp = Interpreter::new(module);
        assert_eq!(interp.module().name, "test");
    }
}
