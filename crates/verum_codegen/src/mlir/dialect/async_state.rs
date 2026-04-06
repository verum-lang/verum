//! Comprehensive async/await state machine for Verum.
//!
//! This module implements industrial-grade async compilation, transforming
//! async functions into state machines that can be polled to completion.
//!
//! # State Machine Structure
//!
//! ```text
//! AsyncStateMachine<T> = {
//!     state: u32,                    // Current state
//!     result: MaybeUninit<T>,       // Result storage
//!     locals: StateMachineLocals,   // Preserved locals across awaits
//!     waker: *mut (),               // Waker for notifications
//! }
//! ```
//!
//! # Poll Result
//!
//! ```text
//! PollResult<T> = {
//!     tag: u8,     // 0 = Pending, 1 = Ready
//!     value: T,    // Valid only if tag == Ready
//! }
//! ```
//!
//! # State Machine States
//!
//! - State 0: Initial state (first poll)
//! - State N: Resumed after Nth await point
//! - State -1: Completed (poisoned on repoll)
//!
//! # Compilation Pipeline
//!
//! 1. Identify await points in async function
//! 2. Compute live variables across each await
//! 3. Generate state machine struct
//! 4. Generate poll function with switch on state

use verum_mlir::{
    Context,
    ir::{
        Attribute, Block, Identifier, Location, Operation, Region, Type, Value,
        attribute::{
            ArrayAttribute, DenseI32ArrayAttribute, DenseI64ArrayAttribute,
            IntegerAttribute, StringAttribute, TypeAttribute,
        },
        operation::{OperationBuilder, OperationLike},
        r#type::IntegerType,
    },
    dialect::{arith, scf},
};
use verum_common::{List, Text};
use crate::mlir::error::{MlirError, Result};
use crate::mlir::dialect::types::VerumType;

// ============================================================================
// Poll Result
// ============================================================================

/// Poll result for async operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollState {
    /// Operation is still pending.
    Pending,
    /// Operation completed with a result.
    Ready,
}

impl PollState {
    /// Get the tag value for this state.
    pub fn tag(&self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Ready => 1,
        }
    }

    /// Create from tag value.
    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Pending,
            _ => Self::Ready,
        }
    }
}

// ============================================================================
// Await Point Analysis
// ============================================================================

/// An await point in an async function.
#[derive(Debug, Clone)]
pub struct AwaitPoint {
    /// Unique ID for this await point.
    pub id: usize,
    /// State number after this await.
    pub resume_state: u32,
    /// Live variables that need to be preserved.
    pub live_vars: Vec<LiveVariable>,
    /// Source location for debugging.
    pub location: Option<Text>,
}

/// A variable that's live across an await point.
#[derive(Debug, Clone)]
pub struct LiveVariable {
    /// Variable name.
    pub name: Text,
    /// Variable type.
    pub ty: VerumType,
    /// Offset in the state machine locals struct.
    pub offset: usize,
    /// Whether the variable is mutable.
    pub mutable: bool,
}

/// Analysis result for an async function.
#[derive(Debug, Clone)]
pub struct AsyncAnalysis {
    /// All await points.
    pub await_points: Vec<AwaitPoint>,
    /// All live variables across any await.
    pub all_live_vars: Vec<LiveVariable>,
    /// Size of the locals struct.
    pub locals_size: usize,
    /// Number of states.
    pub state_count: usize,
}

impl AsyncAnalysis {
    /// Create a new async analysis.
    pub fn new() -> Self {
        Self {
            await_points: Vec::new(),
            all_live_vars: Vec::new(),
            locals_size: 0,
            state_count: 1, // At least initial state
        }
    }

    /// Add an await point.
    pub fn add_await_point(&mut self, live_vars: Vec<LiveVariable>, location: Option<Text>) {
        let id = self.await_points.len();
        let resume_state = self.state_count as u32;
        self.state_count += 1;

        // Merge live vars
        for var in &live_vars {
            if !self.all_live_vars.iter().any(|v| v.name == var.name) {
                self.all_live_vars.push(var.clone());
            }
        }

        self.await_points.push(AwaitPoint {
            id,
            resume_state,
            live_vars,
            location,
        });
    }

    /// Get an await point by ID.
    pub fn get_await_point(&self, id: usize) -> Option<&AwaitPoint> {
        self.await_points.get(id)
    }
}

impl Default for AsyncAnalysis {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Async State Machine Operations
// ============================================================================

/// Create state machine operation.
///
/// Creates a new async state machine.
///
/// ```mlir
/// %sm = verum.async_state_machine_create {
///     state_count = 5 : i32,
///     result_type = i64,
///     locals_size = 32 : i64
/// } : !verum.state_machine<i64>
/// ```
pub struct AsyncStateMachineCreateOp;

impl AsyncStateMachineCreateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        state_count: u32,
        locals_size: usize,
        result_type: Type<'c>,
        sm_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let i32_type = IntegerType::new(context, 32).into();
        let i64_type = IntegerType::new(context, 64).into();

        OperationBuilder::new("verum.async_state_machine_create", location)
            .add_attributes(&[
                (
                    Identifier::new(context, "state_count"),
                    IntegerAttribute::new(i32_type, state_count as i64).into(),
                ),
                (
                    Identifier::new(context, "locals_size"),
                    IntegerAttribute::new(i64_type, locals_size as i64).into(),
                ),
                (
                    Identifier::new(context, "result_type"),
                    TypeAttribute::new(result_type).into(),
                ),
            ])
            .add_results(&[sm_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_state_machine_create", format!("{:?}", e)))
    }
}

/// Get state operation.
///
/// Gets the current state of the state machine.
///
/// ```mlir
/// %state = verum.async_get_state %sm : i32
/// ```
pub struct AsyncGetStateOp;

impl AsyncGetStateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        let i32_type = IntegerType::new(context, 32).into();

        OperationBuilder::new("verum.async_get_state", location)
            .add_operands(&[state_machine])
            .add_results(&[i32_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_get_state", format!("{:?}", e)))
    }
}

/// Set state operation.
///
/// Sets the current state of the state machine.
///
/// ```mlir
/// verum.async_set_state %sm, %new_state
/// ```
pub struct AsyncSetStateOp;

impl AsyncSetStateOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
        new_state: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_set_state", location)
            .add_operands(&[state_machine, new_state])
            .build()
            .map_err(|e| MlirError::operation("verum.async_set_state", format!("{:?}", e)))
    }
}

/// Save locals operation.
///
/// Saves local variables to the state machine.
///
/// ```mlir
/// verum.async_save_locals %sm, [%var0, %var1] {
///     offsets = [0, 8]
/// }
/// ```
pub struct AsyncSaveLocalsOp;

impl AsyncSaveLocalsOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
        locals: &[Value<'c, '_>],
        offsets: &[i64],
    ) -> Result<Operation<'c>> {
        let mut operands = vec![state_machine];
        operands.extend(locals.iter().copied());

        OperationBuilder::new("verum.async_save_locals", location)
            .add_operands(&operands)
            .add_attributes(&[(
                Identifier::new(context, "offsets"),
                DenseI64ArrayAttribute::new(context, offsets).into(),
            )])
            .build()
            .map_err(|e| MlirError::operation("verum.async_save_locals", format!("{:?}", e)))
    }
}

/// Restore locals operation.
///
/// Restores local variables from the state machine.
///
/// ```mlir
/// %var0, %var1 = verum.async_restore_locals %sm {
///     offsets = [0, 8],
///     types = [i64, i64]
/// }
/// ```
pub struct AsyncRestoreLocalsOp;

impl AsyncRestoreLocalsOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
        offsets: &[i64],
        result_types: &[Type<'c>],
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_restore_locals", location)
            .add_operands(&[state_machine])
            .add_attributes(&[(
                Identifier::new(context, "offsets"),
                DenseI64ArrayAttribute::new(context, offsets).into(),
            )])
            .add_results(result_types)
            .build()
            .map_err(|e| MlirError::operation("verum.async_restore_locals", format!("{:?}", e)))
    }
}

/// Set result operation.
///
/// Sets the result of the async computation.
///
/// ```mlir
/// verum.async_set_result %sm, %value
/// ```
pub struct AsyncSetResultOp;

impl AsyncSetResultOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
        result: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_set_result", location)
            .add_operands(&[state_machine, result])
            .build()
            .map_err(|e| MlirError::operation("verum.async_set_result", format!("{:?}", e)))
    }
}

/// Get result operation.
///
/// Gets the result from a completed async computation.
///
/// ```mlir
/// %result = verum.async_get_result %sm : i64
/// ```
pub struct AsyncGetResultOp;

impl AsyncGetResultOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        state_machine: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_get_result", location)
            .add_operands(&[state_machine])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_get_result", format!("{:?}", e)))
    }
}

// ============================================================================
// Poll Operations
// ============================================================================

/// Poll future operation.
///
/// Polls a future for completion.
///
/// ```mlir
/// %poll_result = verum.async_poll %future : !verum.poll_result<i64>
/// ```
pub struct AsyncPollOp;

impl AsyncPollOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        future: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_poll", location)
            .add_operands(&[future])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_poll", format!("{:?}", e)))
    }
}

/// Check poll ready operation.
///
/// Checks if a poll result is ready.
///
/// ```mlir
/// %is_ready = verum.async_poll_is_ready %poll_result : i1
/// ```
pub struct AsyncPollIsReadyOp;

impl AsyncPollIsReadyOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        poll_result: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        let i1_type = IntegerType::new(context, 1).into();

        OperationBuilder::new("verum.async_poll_is_ready", location)
            .add_operands(&[poll_result])
            .add_results(&[i1_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_poll_is_ready", format!("{:?}", e)))
    }
}

/// Extract poll value operation.
///
/// Extracts the value from a ready poll result.
///
/// ```mlir
/// %value = verum.async_poll_value %poll_result : i64
/// ```
pub struct AsyncPollValueOp;

impl AsyncPollValueOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        poll_result: Value<'c, '_>,
        value_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_poll_value", location)
            .add_operands(&[poll_result])
            .add_results(&[value_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_poll_value", format!("{:?}", e)))
    }
}

/// Return pending operation.
///
/// Returns Pending from a poll function.
///
/// ```mlir
/// verum.async_return_pending
/// ```
pub struct AsyncReturnPendingOp;

impl AsyncReturnPendingOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_return_pending", location)
            .build()
            .map_err(|e| MlirError::operation("verum.async_return_pending", format!("{:?}", e)))
    }
}

/// Return ready operation.
///
/// Returns Ready with a value from a poll function.
///
/// ```mlir
/// verum.async_return_ready %value
/// ```
pub struct AsyncReturnReadyOp;

impl AsyncReturnReadyOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        value: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_return_ready", location)
            .add_operands(&[value])
            .build()
            .map_err(|e| MlirError::operation("verum.async_return_ready", format!("{:?}", e)))
    }
}

// ============================================================================
// Spawn and Select Operations
// ============================================================================

/// Spawn task operation.
///
/// Spawns an async task.
///
/// ```mlir
/// %handle = verum.async_spawn %future : !verum.task_handle<i64>
/// ```
pub struct AsyncSpawnOp;

impl AsyncSpawnOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        future: Value<'c, '_>,
        handle_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_spawn", location)
            .add_operands(&[future])
            .add_results(&[handle_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_spawn", format!("{:?}", e)))
    }
}

/// Join task operation.
///
/// Joins (awaits) a spawned task.
///
/// ```mlir
/// %result = verum.async_join %handle : i64
/// ```
pub struct AsyncJoinOp;

impl AsyncJoinOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        handle: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_join", location)
            .add_operands(&[handle])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_join", format!("{:?}", e)))
    }
}

/// Select operation.
///
/// Waits for any of multiple futures to complete.
///
/// ```mlir
/// %result, %index = verum.async_select [%f0, %f1, %f2] : (i64, index)
/// ```
pub struct AsyncSelectOp;

impl AsyncSelectOp {
    pub fn build<'c>(
        context: &'c Context,
        location: Location<'c>,
        futures: &[Value<'c, '_>],
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        let index_type = Type::index(context);

        OperationBuilder::new("verum.async_select", location)
            .add_operands(futures)
            .add_results(&[result_type, index_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_select", format!("{:?}", e)))
    }
}

/// Race operation.
///
/// Races multiple futures, returning the first to complete.
///
/// ```mlir
/// %result = verum.async_race [%f0, %f1] : i64
/// ```
pub struct AsyncRaceOp;

impl AsyncRaceOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        futures: &[Value<'c, '_>],
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_race", location)
            .add_operands(futures)
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_race", format!("{:?}", e)))
    }
}

// ============================================================================
// Waker Operations
// ============================================================================

/// Get waker operation.
///
/// Gets the waker for the current async context.
///
/// ```mlir
/// %waker = verum.async_get_waker : !verum.waker
/// ```
pub struct AsyncGetWakerOp;

impl AsyncGetWakerOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        waker_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_get_waker", location)
            .add_results(&[waker_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_get_waker", format!("{:?}", e)))
    }
}

/// Wake operation.
///
/// Wakes a waker to signal that polling should resume.
///
/// ```mlir
/// verum.async_wake %waker
/// ```
pub struct AsyncWakeOp;

impl AsyncWakeOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        waker: Value<'c, '_>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_wake", location)
            .add_operands(&[waker])
            .build()
            .map_err(|e| MlirError::operation("verum.async_wake", format!("{:?}", e)))
    }
}

/// Clone waker operation.
///
/// Clones a waker.
///
/// ```mlir
/// %waker2 = verum.async_clone_waker %waker : !verum.waker
/// ```
pub struct AsyncCloneWakerOp;

impl AsyncCloneWakerOp {
    pub fn build<'c>(
        _context: &'c Context,
        location: Location<'c>,
        waker: Value<'c, '_>,
        result_type: Type<'c>,
    ) -> Result<Operation<'c>> {
        OperationBuilder::new("verum.async_clone_waker", location)
            .add_operands(&[waker])
            .add_results(&[result_type])
            .build()
            .map_err(|e| MlirError::operation("verum.async_clone_waker", format!("{:?}", e)))
    }
}

// ============================================================================
// Async Type Builder
// ============================================================================

/// Builder for async-related types.
pub struct AsyncTypeBuilder<'c> {
    context: &'c Context,
}

impl<'c> AsyncTypeBuilder<'c> {
    /// Create a new async type builder.
    pub fn new(context: &'c Context) -> Self {
        Self { context }
    }

    /// Create a future type.
    pub fn future_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("future", "failed to parse"))
    }

    /// Create a state machine type.
    ///
    /// StateMachine = { state: i32, result: T, locals: ptr, waker: ptr }
    pub fn state_machine_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(i32, ptr, ptr, ptr)>")
            .ok_or_else(|| MlirError::type_translation("state_machine", "failed to parse"))
    }

    /// Create a poll result type.
    ///
    /// PollResult = { tag: i8, value: T }
    pub fn poll_result_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(i8, ptr)>")
            .ok_or_else(|| MlirError::type_translation("poll_result", "failed to parse"))
    }

    /// Create a task handle type.
    pub fn task_handle_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.ptr")
            .ok_or_else(|| MlirError::type_translation("task_handle", "failed to parse"))
    }

    /// Create a waker type.
    pub fn waker_type(&self) -> Result<Type<'c>> {
        Type::parse(self.context, "!llvm.struct<(ptr, ptr)>")
            .ok_or_else(|| MlirError::type_translation("waker", "failed to parse"))
    }
}

// ============================================================================
// Async State Machine Generator
// ============================================================================

/// Generates async state machine from analysis.
pub struct AsyncStateMachineGenerator<'c> {
    /// Type builder.
    types: AsyncTypeBuilder<'c>,
    /// Counter for unique names.
    counter: usize,
}

impl<'c> AsyncStateMachineGenerator<'c> {
    /// Create a new generator.
    pub fn new(context: &'c Context) -> Self {
        Self {
            types: AsyncTypeBuilder::new(context),
            counter: 0,
        }
    }

    /// Generate a fresh poll function name.
    pub fn fresh_poll_fn_name(&mut self, base: &str) -> Text {
        self.counter += 1;
        Text::from(format!("{}_poll_{}", base, self.counter))
    }

    /// Get the state machine type.
    pub fn state_machine_type(&self) -> Result<Type<'c>> {
        self.types.state_machine_type()
    }

    /// Get the poll result type.
    pub fn poll_result_type(&self) -> Result<Type<'c>> {
        self.types.poll_result_type()
    }

    /// Get the future type.
    pub fn future_type(&self) -> Result<Type<'c>> {
        self.types.future_type()
    }

    /// Get the waker type.
    pub fn waker_type(&self) -> Result<Type<'c>> {
        self.types.waker_type()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poll_state() {
        assert_eq!(PollState::Pending.tag(), 0);
        assert_eq!(PollState::Ready.tag(), 1);
        assert_eq!(PollState::from_tag(0), PollState::Pending);
        assert_eq!(PollState::from_tag(1), PollState::Ready);
    }

    #[test]
    fn test_async_analysis() {
        let mut analysis = AsyncAnalysis::new();
        assert_eq!(analysis.state_count, 1);
        assert!(analysis.await_points.is_empty());

        analysis.add_await_point(vec![], None);
        assert_eq!(analysis.state_count, 2);
        assert_eq!(analysis.await_points.len(), 1);

        analysis.add_await_point(
            vec![LiveVariable {
                name: Text::from("x"),
                ty: VerumType::i64(),
                offset: 0,
                mutable: false,
            }],
            Some(Text::from("test.vr:10")),
        );
        assert_eq!(analysis.state_count, 3);
        assert_eq!(analysis.all_live_vars.len(), 1);
    }

    #[test]
    fn test_await_point() {
        let point = AwaitPoint {
            id: 0,
            resume_state: 1,
            live_vars: vec![],
            location: Some(Text::from("main.vr:5")),
        };
        assert_eq!(point.id, 0);
        assert_eq!(point.resume_state, 1);
    }
}
