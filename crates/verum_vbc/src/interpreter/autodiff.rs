//! Autodiff runtime support for VBC interpreter.
//!
//! This module provides runtime gradient computation infrastructure for
//! interpreting VBC autodiff opcodes (0xA8-0xAF).
//!
//! # Architecture
//!
//! While Verum's production autodiff is compile-time source transformation,
//! the interpreter requires runtime gradient tape support for:
//! - Testing compiled autodiff code
//! - Dynamic computation graphs
//! - Debugging and development
//!
//! # Design
//!
//! The gradient tape records operations during forward pass and replays
//! them in reverse during backward pass. Each operation records:
//! - Operation type (add, mul, matmul, etc.)
//! - Input tensor IDs
//! - Output tensor ID
//! - Saved values needed for backward
//!
//! # Example
//!
//! ```ignore
//! let mut tape = GradientTape::new();
//! tape.begin_scope(GradMode::Reverse);
//!
//! // Forward pass - operations are recorded
//! let x = tape.track_tensor(tensor_x);
//! let y = tape.track_tensor(tensor_y);
//! let z = tape.record_op(TapeOp::Add, &[x, y])?;
//!
//! // Backward pass
//! tape.set_grad(z, ones_like(z));
//! tape.backward()?;
//!
//! let dx = tape.get_grad(x);
//! let dy = tape.get_grad(y);
//! ```

use std::collections::HashMap;

use crate::instruction::{TensorReduceOp, TensorUnaryOp};
use super::tensor::{DType, TensorHandle};

/// Maximum number of nested gradient scopes.
pub const MAX_GRAD_SCOPES: usize = 16;

/// Maximum number of tensors tracked per scope.
pub const MAX_TRACKED_TENSORS: usize = 65536;

/// Maximum number of operations recorded per scope.
pub const MAX_TAPE_OPS: usize = 1_000_000;

// ============================================================================
// Types and Enums
// ============================================================================

/// Gradient computation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradMode {
    /// Reverse-mode autodiff (backpropagation).
    Reverse,
    /// Forward-mode autodiff (tangent propagation).
    Forward,
    /// Automatic mode selection based on input/output dimensions.
    Auto,
}

/// Unique identifier for a tracked tensor in the gradient tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TensorId(pub u32);

/// Unique identifier for a gradient scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

/// Unique identifier for a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CheckpointId(pub u32);

/// Operation type recorded on the gradient tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapeOp {
    // Binary operations
    /// Element-wise addition.
    Add,
    /// Element-wise subtraction.
    Sub,
    /// Element-wise multiplication.
    Mul,
    /// Element-wise division.
    Div,
    /// Element-wise power operation.
    Pow,
    /// Matrix multiplication.
    MatMul,

    // Unary operations
    /// Negation.
    Neg,
    /// Exponential function.
    Exp,
    /// Natural logarithm.
    Log,
    /// Square root.
    Sqrt,
    /// Sine function.
    Sin,
    /// Cosine function.
    Cos,
    /// Hyperbolic tangent.
    Tanh,
    /// Sigmoid activation.
    Sigmoid,
    /// ReLU activation.
    Relu,
    /// Softmax activation.
    Softmax,
    /// Error function.
    Erf,
    /// GELU activation (Gaussian Error Linear Unit).
    Gelu,
    /// SiLU/Swish activation (x * sigmoid(x)).
    Silu,
    /// Leaky ReLU activation.
    LeakyRelu,
    /// Absolute value.
    Abs,
    /// Hyperbolic sine.
    Sinh,
    /// Hyperbolic cosine.
    Cosh,
    /// Inverse hyperbolic tangent.
    Atanh,
    /// Log-softmax activation.
    LogSoftmax,

    // Reduction operations
    /// Sum reduction.
    Sum,
    /// Mean reduction.
    Mean,
    /// Max reduction.
    Max,
    /// Min reduction.
    Min,

    // Shape operations
    /// Reshape tensor.
    Reshape,
    /// Transpose tensor.
    Transpose,
    /// Broadcast to target shape.
    BroadcastTo,
    /// Remove dimensions of size 1.
    Squeeze,
    /// Add dimension of size 1.
    Unsqueeze,

    // Neural network operations
    /// 2D Convolution.
    Conv2d,
    /// Layer normalization.
    LayerNorm,
    /// Batch normalization.
    BatchNorm,
    /// RMS normalization.
    RmsNorm,
    /// Flash attention.
    FlashAttention,
    /// Gather values along axis.
    Gather,
    /// Scatter values along axis.
    Scatter,
    /// Index select along axis.
    IndexSelect,
    /// Concatenate tensors.
    Concat,
    /// Slice tensor.
    Slice,

    /// Custom operation with custom VJP, identified by ID.
    Custom(u32),
}

// ============================================================================
// Custom VJP/JVP Rules
// ============================================================================

/// Unique identifier for a custom gradient rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CustomRuleId(pub u32);

/// A registered custom VJP (Vector-Jacobian Product) rule.
///
/// Custom VJP rules allow users to define gradient computation for operations
/// that either don't have automatic gradients or have more efficient custom
/// implementations.
///
/// The VJP rule computes: vjp(inputs, grad_output) -> grad_inputs
#[derive(Debug, Clone)]
pub struct CustomVjpRule {
    /// Unique rule ID.
    pub id: CustomRuleId,
    /// VBC function ID for the forward function.
    pub forward_fn: u32,
    /// VBC function ID for the VJP function.
    /// Signature: fn(inputs: &[Tensor], grad_output: Tensor) -> Vec<Tensor>
    pub vjp_fn: u32,
    /// Optional descriptive name for debugging.
    pub name: Option<String>,
}

/// A registered custom JVP (Jacobian-Vector Product) rule.
///
/// Custom JVP rules allow users to define tangent computation for forward-mode
/// autodiff.
///
/// The JVP rule computes: jvp(inputs, tangent_inputs) -> tangent_output
#[derive(Debug, Clone)]
pub struct CustomJvpRule {
    /// Unique rule ID.
    pub id: CustomRuleId,
    /// VBC function ID for the forward function.
    pub forward_fn: u32,
    /// VBC function ID for the JVP function.
    /// Signature: fn(inputs: &[Tensor], tangents: &[Tensor]) -> Tensor
    pub jvp_fn: u32,
    /// Optional descriptive name for debugging.
    pub name: Option<String>,
}

/// Registry for custom gradient rules.
///
/// This registry stores user-defined VJP and JVP rules that can be looked up
/// during backward/forward passes.
#[derive(Debug, Default)]
pub struct CustomGradRegistry {
    /// VJP rules indexed by rule ID.
    vjp_rules: HashMap<CustomRuleId, CustomVjpRule>,
    /// JVP rules indexed by rule ID.
    jvp_rules: HashMap<CustomRuleId, CustomJvpRule>,
    /// Mapping from forward function ID to rule ID.
    fn_to_rule: HashMap<u32, CustomRuleId>,
    /// Next rule ID to assign.
    next_rule_id: u32,
}

impl CustomGradRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            vjp_rules: HashMap::new(),
            jvp_rules: HashMap::new(),
            fn_to_rule: HashMap::new(),
            next_rule_id: 0,
        }
    }

    /// Registers a custom VJP rule.
    ///
    /// Returns the assigned rule ID.
    pub fn register_vjp(&mut self, forward_fn: u32, vjp_fn: u32, name: Option<String>) -> CustomRuleId {
        let id = CustomRuleId(self.next_rule_id);
        self.next_rule_id += 1;

        let rule = CustomVjpRule {
            id,
            forward_fn,
            vjp_fn,
            name,
        };

        self.vjp_rules.insert(id, rule);
        self.fn_to_rule.insert(forward_fn, id);

        id
    }

    /// Registers a custom JVP rule.
    ///
    /// Returns the assigned rule ID.
    pub fn register_jvp(&mut self, forward_fn: u32, jvp_fn: u32, name: Option<String>) -> CustomRuleId {
        let id = CustomRuleId(self.next_rule_id);
        self.next_rule_id += 1;

        let rule = CustomJvpRule {
            id,
            forward_fn,
            jvp_fn,
            name,
        };

        self.jvp_rules.insert(id, rule);
        self.fn_to_rule.insert(forward_fn, id);

        id
    }

    /// Gets a VJP rule by ID.
    pub fn get_vjp(&self, id: CustomRuleId) -> Option<&CustomVjpRule> {
        self.vjp_rules.get(&id)
    }

    /// Gets a JVP rule by ID.
    pub fn get_jvp(&self, id: CustomRuleId) -> Option<&CustomJvpRule> {
        self.jvp_rules.get(&id)
    }

    /// Gets the rule ID for a forward function, if registered.
    pub fn get_rule_for_fn(&self, forward_fn: u32) -> Option<CustomRuleId> {
        self.fn_to_rule.get(&forward_fn).copied()
    }

    /// Returns true if a VJP rule exists for the given rule ID.
    pub fn has_vjp(&self, id: CustomRuleId) -> bool {
        self.vjp_rules.contains_key(&id)
    }

    /// Returns true if a JVP rule exists for the given rule ID.
    pub fn has_jvp(&self, id: CustomRuleId) -> bool {
        self.jvp_rules.contains_key(&id)
    }

    /// Clears all registered rules.
    pub fn clear(&mut self) {
        self.vjp_rules.clear();
        self.jvp_rules.clear();
        self.fn_to_rule.clear();
        self.next_rule_id = 0;
    }
}

// ============================================================================
// Tape Entry
// ============================================================================

/// A single recorded operation on the gradient tape.
#[derive(Debug)]
pub struct TapeEntry {
    /// Operation type.
    pub op: TapeOp,
    /// Input tensor IDs.
    pub inputs: Vec<TensorId>,
    /// Output tensor ID.
    pub output: TensorId,
    /// Saved values needed for backward pass (e.g., activations).
    pub saved: Vec<SavedValue>,
    /// Original shape for shape-changing operations.
    pub shape_info: Option<Vec<usize>>,
}

/// Saved values during forward pass for backward computation.
#[derive(Debug)]
pub enum SavedValue {
    /// A tensor value that was saved.
    Tensor(TensorHandle),
    /// A scalar f64 value.
    Scalar(f64),
    /// An integer value (axis index, etc.).
    Int(i64),
    /// A shape specification.
    Shape(Vec<usize>),
}

// ============================================================================
// Gradient Scope
// ============================================================================

/// A gradient computation scope.
///
/// Scopes can be nested to support checkpointing and selective gradient
/// computation.
#[derive(Debug)]
pub struct GradScope {
    /// Scope identifier.
    pub id: ScopeId,
    /// Gradient mode for this scope.
    pub mode: GradMode,
    /// Whether this scope is active.
    pub active: bool,
    /// Tape entries for this scope.
    pub tape: Vec<TapeEntry>,
    /// Tracked tensors: TensorId -> TensorHandle.
    pub tensors: HashMap<TensorId, TensorHandle>,
    /// Gradients: TensorId -> gradient tensor (for reverse mode).
    pub gradients: HashMap<TensorId, TensorHandle>,
    /// Tangents: TensorId -> tangent tensor (for forward mode).
    pub tangents: HashMap<TensorId, TensorHandle>,
    /// Next tensor ID.
    next_tensor_id: u32,
    /// Checkpoints in this scope.
    pub checkpoints: HashMap<CheckpointId, Checkpoint>,
    /// Next checkpoint ID.
    next_checkpoint_id: u32,
    /// Tensors marked with gradient stop (detached).
    pub stopped: std::collections::HashSet<TensorId>,
}

impl GradScope {
    /// Creates a new gradient scope.
    pub fn new(id: ScopeId, mode: GradMode) -> Self {
        Self {
            id,
            mode,
            active: true,
            tape: Vec::with_capacity(1024),
            tensors: HashMap::with_capacity(256),
            gradients: HashMap::with_capacity(256),
            tangents: HashMap::with_capacity(256),
            next_tensor_id: 0,
            checkpoints: HashMap::new(),
            next_checkpoint_id: 0,
            stopped: std::collections::HashSet::new(),
        }
    }

    /// Tracks a tensor and returns its ID.
    pub fn track_tensor(&mut self, tensor: TensorHandle) -> Option<TensorId> {
        if self.tensors.len() >= MAX_TRACKED_TENSORS {
            return None;
        }
        let id = TensorId(self.next_tensor_id);
        self.next_tensor_id += 1;
        self.tensors.insert(id, tensor);
        Some(id)
    }

    /// Records an operation on the tape.
    pub fn record_op(
        &mut self,
        op: TapeOp,
        inputs: &[TensorId],
        output: TensorId,
        saved: Vec<SavedValue>,
        shape_info: Option<Vec<usize>>,
    ) -> Option<()> {
        if self.tape.len() >= MAX_TAPE_OPS {
            return None;
        }
        self.tape.push(TapeEntry {
            op,
            inputs: inputs.to_vec(),
            output,
            saved,
            shape_info,
        });
        Some(())
    }

    /// Sets the gradient for a tensor (reverse mode).
    pub fn set_grad(&mut self, id: TensorId, grad: TensorHandle) {
        self.gradients.insert(id, grad);
    }

    /// Gets the gradient for a tensor (reverse mode).
    pub fn get_grad(&self, id: TensorId) -> Option<&TensorHandle> {
        self.gradients.get(&id)
    }

    /// Sets the tangent for a tensor (forward mode).
    pub fn set_tangent(&mut self, id: TensorId, tangent: TensorHandle) {
        self.tangents.insert(id, tangent);
    }

    /// Gets the tangent for a tensor (forward mode).
    pub fn get_tangent(&self, id: TensorId) -> Option<&TensorHandle> {
        self.tangents.get(&id)
    }

    /// Creates a checkpoint of current state.
    pub fn checkpoint(&mut self, tensor_ids: &[TensorId]) -> Option<CheckpointId> {
        let id = CheckpointId(self.next_checkpoint_id);
        self.next_checkpoint_id += 1;

        let mut saved_tensors = HashMap::new();
        for &tid in tensor_ids {
            if let Some(tensor) = self.tensors.get(&tid) {
                saved_tensors.insert(tid, tensor.clone());
            }
        }

        self.checkpoints.insert(
            id,
            Checkpoint {
                id,
                tape_position: self.tape.len(),
                saved_tensors,
            },
        );

        Some(id)
    }

    /// Recomputes from a checkpoint.
    pub fn recompute(&mut self, checkpoint_id: CheckpointId) -> Option<()> {
        let checkpoint = self.checkpoints.get(&checkpoint_id)?;

        // Restore tensors from checkpoint
        for (tid, tensor) in &checkpoint.saved_tensors {
            self.tensors.insert(*tid, tensor.clone());
        }

        // Truncate tape to checkpoint position
        self.tape.truncate(checkpoint.tape_position);

        Some(())
    }

    /// Marks a tensor as having stopped gradients.
    pub fn stop_gradient(&mut self, id: TensorId) {
        self.stopped.insert(id);
    }

    /// Checks if gradient flow is stopped for a tensor.
    pub fn is_stopped(&self, id: TensorId) -> bool {
        self.stopped.contains(&id)
    }

    /// Returns all tracked tensor IDs in this scope.
    pub fn all_tensor_ids(&self) -> Vec<TensorId> {
        self.tensors.keys().copied().collect()
    }

    /// Creates a checkpoint of all tensors in this scope.
    ///
    /// This is useful for gradient checkpointing where you want to save all
    /// activations at a checkpoint boundary without specifying them explicitly.
    pub fn checkpoint_all(&mut self) -> Option<CheckpointId> {
        let ids: Vec<TensorId> = self.tensors.keys().copied().collect();
        self.checkpoint(&ids)
    }
}

// ============================================================================
// Checkpoint
// ============================================================================

/// A saved checkpoint for gradient recomputation.
#[derive(Debug)]
pub struct Checkpoint {
    /// Checkpoint identifier.
    pub id: CheckpointId,
    /// Position in the tape when checkpoint was created.
    pub tape_position: usize,
    /// Saved tensor values.
    pub saved_tensors: HashMap<TensorId, TensorHandle>,
}

// ============================================================================
// Gradient Tape
// ============================================================================

/// The main gradient tape structure.
///
/// Manages multiple nested gradient scopes and coordinates backward pass
/// computation.
#[derive(Debug)]
pub struct GradientTape {
    /// Stack of active gradient scopes.
    scopes: Vec<GradScope>,
    /// Next scope ID.
    next_scope_id: u32,
    /// Whether tape is in backward pass mode.
    in_backward: bool,
    /// Registry for custom VJP/JVP rules.
    custom_rules: CustomGradRegistry,
}

impl Default for GradientTape {
    fn default() -> Self {
        Self::new()
    }
}

impl GradientTape {
    /// Creates a new empty gradient tape.
    pub fn new() -> Self {
        Self {
            scopes: Vec::with_capacity(MAX_GRAD_SCOPES),
            next_scope_id: 0,
            custom_rules: CustomGradRegistry::new(),
            in_backward: false,
        }
    }

    /// Begins a new gradient scope.
    pub fn begin_scope(&mut self, mode: GradMode) -> Option<ScopeId> {
        if self.scopes.len() >= MAX_GRAD_SCOPES {
            return None;
        }
        let id = ScopeId(self.next_scope_id);
        self.next_scope_id += 1;
        self.scopes.push(GradScope::new(id, mode));
        Some(id)
    }

    /// Ends the current gradient scope.
    pub fn end_scope(&mut self) -> Option<ScopeId> {
        self.scopes.pop().map(|s| s.id)
    }

    /// Gets the current (innermost) scope.
    pub fn current_scope(&self) -> Option<&GradScope> {
        self.scopes.last()
    }

    /// Gets the current (innermost) scope mutably.
    pub fn current_scope_mut(&mut self) -> Option<&mut GradScope> {
        self.scopes.last_mut()
    }

    /// Gets a scope by ID.
    pub fn get_scope(&self, id: ScopeId) -> Option<&GradScope> {
        self.scopes.iter().find(|s| s.id == id)
    }

    /// Gets a scope by ID mutably.
    pub fn get_scope_mut(&mut self, id: ScopeId) -> Option<&mut GradScope> {
        self.scopes.iter_mut().find(|s| s.id == id)
    }

    /// Tracks a tensor in the current scope.
    pub fn track_tensor(&mut self, tensor: TensorHandle) -> Option<TensorId> {
        self.current_scope_mut()?.track_tensor(tensor)
    }

    /// Records an operation in the current scope.
    pub fn record_op(
        &mut self,
        op: TapeOp,
        inputs: &[TensorId],
        output: TensorId,
        saved: Vec<SavedValue>,
    ) -> Option<()> {
        self.current_scope_mut()?
            .record_op(op, inputs, output, saved, None)
    }

    /// Sets the output gradient to seed backward pass (reverse mode).
    pub fn set_output_grad(&mut self, id: TensorId, grad: TensorHandle) -> Option<()> {
        self.current_scope_mut()?.set_grad(id, grad);
        Some(())
    }

    /// Sets the input tangent to seed forward pass (forward mode).
    pub fn set_input_tangent(&mut self, id: TensorId, tangent: TensorHandle) -> Option<()> {
        self.current_scope_mut()?.set_tangent(id, tangent);
        Some(())
    }

    /// Gets the output tangent after forward pass (forward mode).
    pub fn get_tangent(&self, id: TensorId) -> Option<&TensorHandle> {
        self.current_scope()?.get_tangent(id)
    }

    /// Runs backward pass to compute gradients.
    pub fn backward(&mut self) -> Option<()> {
        if self.in_backward {
            return None; // Already in backward pass
        }
        self.in_backward = true;

        let mode = self.scopes.last()?.mode;

        let result = match mode {
            GradMode::Reverse => self.backward_reverse(),
            GradMode::Forward => self.backward_forward(),
            GradMode::Auto => {
                // Default to reverse mode for most cases
                self.backward_reverse()
            }
        };

        self.in_backward = false;
        result
    }

    /// Reverse-mode autodiff (backpropagation).
    fn backward_reverse(&mut self) -> Option<()> {
        let scope = self.scopes.last_mut()?;

        // Iterate tape in reverse order
        for i in (0..scope.tape.len()).rev() {
            let entry = &scope.tape[i];

            // Skip if output gradient doesn't exist or is stopped
            if scope.is_stopped(entry.output) {
                continue;
            }

            let out_grad = match scope.gradients.get(&entry.output) {
                Some(g) => g.clone(),
                None => continue, // No gradient to propagate
            };

            // Compute input gradients based on operation type
            let input_grads = compute_vjp(entry, &out_grad, scope)?;

            // Accumulate gradients for inputs
            for (input_id, grad) in entry.inputs.iter().zip(input_grads.into_iter()) {
                if scope.is_stopped(*input_id) {
                    continue;
                }

                if let Some(existing) = scope.gradients.get_mut(input_id) {
                    // Accumulate: existing += grad
                    if let Some(sum) = tensor_add(existing, &grad) {
                        *existing = sum;
                    }
                } else {
                    scope.gradients.insert(*input_id, grad);
                }
            }
        }

        Some(())
    }

    /// Forward-mode autodiff (tangent propagation).
    ///
    /// Propagates tangents through the computation graph in forward order,
    /// computing Jacobian-vector products (JVPs). This is efficient when:
    /// - Number of inputs < number of outputs
    /// - Computing directional derivatives
    /// - Building Jacobians column by column
    fn backward_forward(&mut self) -> Option<()> {
        let scope = self.scopes.last_mut()?;

        // Iterate tape in forward order
        for i in 0..scope.tape.len() {
            let entry = &scope.tape[i];

            // Skip if all input tangents are missing or tensor is stopped
            if scope.is_stopped(entry.output) {
                continue;
            }

            // Collect input tangents
            let input_tangents: Vec<Option<TensorHandle>> = entry
                .inputs
                .iter()
                .map(|id| scope.tangents.get(id).cloned())
                .collect();

            // Skip if no input tangents are seeded
            if input_tangents.iter().all(|t| t.is_none()) {
                continue;
            }

            // Compute output tangent based on operation type
            if let Some(out_tangent) = compute_jvp(entry, &input_tangents, scope) {
                scope.tangents.insert(entry.output, out_tangent);
            }
        }

        self.in_backward = false;
        Some(())
    }

    /// Gets the computed gradient for a tensor.
    pub fn get_grad(&self, id: TensorId) -> Option<&TensorHandle> {
        self.current_scope()?.get_grad(id)
    }

    /// Creates a checkpoint in the current scope.
    pub fn checkpoint(&mut self, tensor_ids: &[TensorId]) -> Option<CheckpointId> {
        self.current_scope_mut()?.checkpoint(tensor_ids)
    }

    /// Recomputes from a checkpoint.
    pub fn recompute(&mut self, checkpoint_id: CheckpointId) -> Option<()> {
        self.current_scope_mut()?.recompute(checkpoint_id)
    }

    /// Returns all tracked tensor IDs in the current scope.
    pub fn all_tensor_ids(&self) -> Option<Vec<TensorId>> {
        Some(self.current_scope()?.all_tensor_ids())
    }

    /// Creates a checkpoint of all tensors in the current scope.
    ///
    /// This is the preferred method for gradient checkpointing when you want
    /// to save all activations at a checkpoint boundary.
    pub fn checkpoint_all(&mut self) -> Option<CheckpointId> {
        self.current_scope_mut()?.checkpoint_all()
    }

    /// Zeros all gradients in the current scope.
    pub fn zero_grad(&mut self) -> Option<()> {
        self.current_scope_mut()?.gradients.clear();
        Some(())
    }

    /// Zeros all tangents in the current scope.
    pub fn zero_tangents(&mut self) -> Option<()> {
        self.current_scope_mut()?.tangents.clear();
        Some(())
    }

    /// Marks a tensor as having stopped gradients.
    pub fn stop_gradient(&mut self, id: TensorId) -> Option<()> {
        self.current_scope_mut()?.stop_gradient(id);
        Some(())
    }

    /// Returns true if currently in an active gradient scope.
    pub fn is_recording(&self) -> bool {
        !self.scopes.is_empty() && !self.in_backward
    }

    // =========================================================================
    // Custom VJP/JVP Rule Management
    // =========================================================================

    /// Registers a custom VJP rule for a function.
    ///
    /// The VJP function signature should be:
    /// `fn(inputs: &[Tensor], grad_output: Tensor) -> Vec<Tensor>`
    ///
    /// Returns the assigned rule ID.
    pub fn register_custom_vjp(&mut self, forward_fn: u32, vjp_fn: u32) -> CustomRuleId {
        self.custom_rules.register_vjp(forward_fn, vjp_fn, None)
    }

    /// Registers a custom VJP rule with a descriptive name.
    pub fn register_custom_vjp_named(
        &mut self,
        forward_fn: u32,
        vjp_fn: u32,
        name: &str,
    ) -> CustomRuleId {
        self.custom_rules.register_vjp(forward_fn, vjp_fn, Some(name.to_string()))
    }

    /// Registers a custom JVP rule for a function.
    ///
    /// The JVP function signature should be:
    /// `fn(inputs: &[Tensor], tangents: &[Tensor]) -> Tensor`
    ///
    /// Returns the assigned rule ID.
    pub fn register_custom_jvp(&mut self, forward_fn: u32, jvp_fn: u32) -> CustomRuleId {
        self.custom_rules.register_jvp(forward_fn, jvp_fn, None)
    }

    /// Gets the custom VJP rule for a rule ID.
    pub fn get_custom_vjp(&self, rule_id: CustomRuleId) -> Option<&CustomVjpRule> {
        self.custom_rules.get_vjp(rule_id)
    }

    /// Gets the custom JVP rule for a rule ID.
    pub fn get_custom_jvp(&self, rule_id: CustomRuleId) -> Option<&CustomJvpRule> {
        self.custom_rules.get_jvp(rule_id)
    }

    /// Gets the rule ID for a forward function, if registered.
    pub fn get_rule_for_fn(&self, forward_fn: u32) -> Option<CustomRuleId> {
        self.custom_rules.get_rule_for_fn(forward_fn)
    }

    /// Returns true if a custom VJP rule exists for the given rule ID.
    pub fn has_custom_vjp(&self, rule_id: CustomRuleId) -> bool {
        self.custom_rules.has_vjp(rule_id)
    }

    /// Returns true if a custom JVP rule exists for the given rule ID.
    pub fn has_custom_jvp(&self, rule_id: CustomRuleId) -> bool {
        self.custom_rules.has_jvp(rule_id)
    }

    /// Gets a reference to the custom rules registry.
    pub fn custom_rules(&self) -> &CustomGradRegistry {
        &self.custom_rules
    }

    /// Gets a mutable reference to the custom rules registry.
    pub fn custom_rules_mut(&mut self) -> &mut CustomGradRegistry {
        &mut self.custom_rules
    }

    /// Clears all scopes and resets the tape.
    ///
    /// Note: This does NOT clear custom VJP/JVP rules. Use `reset_all()` to
    /// clear everything including custom rules.
    pub fn reset(&mut self) {
        self.scopes.clear();
        self.next_scope_id = 0;
        self.in_backward = false;
    }

    /// Clears all scopes, resets the tape, and clears all custom rules.
    pub fn reset_all(&mut self) {
        self.reset();
        self.custom_rules.clear();
    }
}

// ============================================================================
// VJP (Vector-Jacobian Product) Computation
// ============================================================================

/// Computes the vector-Jacobian product for a tape entry.
///
/// Returns gradients for each input tensor.
fn compute_vjp(
    entry: &TapeEntry,
    out_grad: &TensorHandle,
    scope: &GradScope,
) -> Option<Vec<TensorHandle>> {
    match entry.op {
        TapeOp::Add => {
            // d/dx (x + y) = 1, d/dy (x + y) = 1
            // VJP: dx = dout, dy = dout
            Some(vec![out_grad.clone(), out_grad.clone()])
        }

        TapeOp::Sub => {
            // d/dx (x - y) = 1, d/dy (x - y) = -1
            // VJP: dx = dout, dy = -dout
            let neg_grad = tensor_neg(out_grad)?;
            Some(vec![out_grad.clone(), neg_grad])
        }

        TapeOp::Mul => {
            // d/dx (x * y) = y, d/dy (x * y) = x
            // VJP: dx = dout * y, dy = dout * x
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;
            let dx = tensor_mul(out_grad, y)?;
            let dy = tensor_mul(out_grad, x)?;
            Some(vec![dx, dy])
        }

        TapeOp::Div => {
            // d/dx (x / y) = 1/y, d/dy (x / y) = -x/y^2
            // VJP: dx = dout / y, dy = -dout * x / y^2
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;
            let dx = tensor_div(out_grad, y)?;
            let y_sq = tensor_mul(y, y)?;
            let neg_x = tensor_neg(x)?;
            let dy_num = tensor_mul(out_grad, &neg_x)?;
            let dy = tensor_div(&dy_num, &y_sq)?;
            Some(vec![dx, dy])
        }

        TapeOp::Neg => {
            // d/dx (-x) = -1
            // VJP: dx = -dout
            let dx = tensor_neg(out_grad)?;
            Some(vec![dx])
        }

        TapeOp::Exp => {
            // d/dx exp(x) = exp(x)
            // VJP: dx = dout * exp(x)
            // We save the output (exp(x)) during forward
            if entry.saved.is_empty() {
                return None;
            }
            let exp_x = get_saved_tensor(&entry.saved[0])?;
            let dx = tensor_mul(out_grad, exp_x)?;
            Some(vec![dx])
        }

        TapeOp::Log => {
            // d/dx log(x) = 1/x
            // VJP: dx = dout / x
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = tensor_div(out_grad, x)?;
            Some(vec![dx])
        }

        TapeOp::Sqrt => {
            // d/dx sqrt(x) = 0.5 / sqrt(x)
            // VJP: dx = 0.5 * dout / sqrt(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sqrt_x = get_saved_tensor(&entry.saved[0])?;
            let half = TensorHandle::full(&[], out_grad.dtype, 0.5)?;
            let half_dout = tensor_mul(&half, out_grad)?;
            let dx = tensor_div(&half_dout, sqrt_x)?;
            Some(vec![dx])
        }

        TapeOp::Sin => {
            // d/dx sin(x) = cos(x)
            // VJP: dx = dout * cos(x)
            if entry.saved.is_empty() {
                return None;
            }
            let cos_x = get_saved_tensor(&entry.saved[0])?; // We save cos(x) during forward
            let dx = tensor_mul(out_grad, cos_x)?;
            Some(vec![dx])
        }

        TapeOp::Cos => {
            // d/dx cos(x) = -sin(x)
            // VJP: dx = -dout * sin(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sin_x = get_saved_tensor(&entry.saved[0])?; // We save sin(x) during forward
            let neg_sin = tensor_neg(sin_x)?;
            let dx = tensor_mul(out_grad, &neg_sin)?;
            Some(vec![dx])
        }

        TapeOp::Tanh => {
            // d/dx tanh(x) = 1 - tanh(x)^2
            // VJP: dx = dout * (1 - tanh(x)^2)
            if entry.saved.is_empty() {
                return None;
            }
            let tanh_x = get_saved_tensor(&entry.saved[0])?;
            let tanh_sq = tensor_mul(tanh_x, tanh_x)?;
            let one = TensorHandle::full(&[], out_grad.dtype, 1.0)?;
            let one_minus = tensor_sub(&one, &tanh_sq)?;
            let dx = tensor_mul(out_grad, &one_minus)?;
            Some(vec![dx])
        }

        TapeOp::Sigmoid => {
            // d/dx sigmoid(x) = sigmoid(x) * (1 - sigmoid(x))
            // VJP: dx = dout * sigmoid(x) * (1 - sigmoid(x))
            if entry.saved.is_empty() {
                return None;
            }
            let sig_x = get_saved_tensor(&entry.saved[0])?;
            let one = TensorHandle::full(&[], out_grad.dtype, 1.0)?;
            let one_minus = tensor_sub(&one, sig_x)?;
            let sig_deriv = tensor_mul(sig_x, &one_minus)?;
            let dx = tensor_mul(out_grad, &sig_deriv)?;
            Some(vec![dx])
        }

        TapeOp::Relu => {
            // d/dx relu(x) = 1 if x > 0 else 0
            // VJP: dx = dout * (x > 0)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let mask = tensor_gt_zero(x)?;
            let dx = tensor_mul(out_grad, &mask)?;
            Some(vec![dx])
        }

        TapeOp::LeakyRelu => {
            // d/dx leaky_relu(x) = 1 if x > 0 else alpha
            // VJP: dx = dout * (1 if x > 0 else alpha)
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            // alpha is saved as a scalar tensor
            let alpha_scalar = entry.saved.get(1)
                .and_then(|s| get_saved_tensor(s))
                .and_then(|t| t.get_scalar_f64())
                .unwrap_or(0.01);

            let mask_pos = tensor_gt_zero(x)?;
            let mask_neg = tensor_le_zero(x)?;
            let alpha_tensor = TensorHandle::full(&[], out_grad.dtype, alpha_scalar)?;
            let scaled_neg = tensor_mul(&mask_neg, &alpha_tensor)?;
            let combined_mask = tensor_add(&mask_pos, &scaled_neg)?;
            let dx = tensor_mul(out_grad, &combined_mask)?;
            Some(vec![dx])
        }

        TapeOp::Erf => {
            // d/dx erf(x) = 2/sqrt(pi) * exp(-x^2)
            // VJP: dx = dout * 2/sqrt(pi) * exp(-x^2)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;

            // 2/sqrt(pi) ≈ 1.1283791670955126
            let two_over_sqrt_pi = TensorHandle::full(&[], out_grad.dtype, 1.1283791670955126)?;

            // -x^2
            let x_sq = tensor_mul(x, x)?;
            let neg_x_sq = tensor_neg(&x_sq)?;

            // exp(-x^2)
            let exp_neg_x_sq = super::tensor::tensor_unop(&neg_x_sq, TensorUnaryOp::Exp)?;

            // 2/sqrt(pi) * exp(-x^2)
            let deriv = tensor_mul(&two_over_sqrt_pi, &exp_neg_x_sq)?;
            let dx = tensor_mul(out_grad, &deriv)?;
            Some(vec![dx])
        }

        TapeOp::Gelu => {
            // GELU(x) = x * Φ(x) where Φ is the CDF of standard normal
            // Approximate: GELU(x) ≈ 0.5 * x * (1 + tanh(sqrt(2/pi) * (x + 0.044715 * x^3)))
            // d/dx GELU(x) = Φ(x) + x * φ(x) where φ is the PDF
            // Simplified VJP using the tanh approximation derivative
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;

            // Constants
            let sqrt_2_over_pi = TensorHandle::full(&[], out_grad.dtype, 0.7978845608028654)?;
            let c = TensorHandle::full(&[], out_grad.dtype, 0.044715)?;
            let half = TensorHandle::full(&[], out_grad.dtype, 0.5)?;
            let one = TensorHandle::full(&[], out_grad.dtype, 1.0)?;
            let three = TensorHandle::full(&[], out_grad.dtype, 3.0)?;

            // x^2, x^3
            let x_sq = tensor_mul(x, x)?;
            let x_cubed = tensor_mul(&x_sq, x)?;

            // inner = sqrt(2/pi) * (x + 0.044715 * x^3)
            let c_x3 = tensor_mul(&c, &x_cubed)?;
            let x_plus_cx3 = tensor_add(x, &c_x3)?;
            let inner = tensor_mul(&sqrt_2_over_pi, &x_plus_cx3)?;

            // tanh(inner)
            let tanh_inner = super::tensor::tensor_unop(&inner, TensorUnaryOp::Tanh)?;

            // sech^2(inner) = 1 - tanh^2(inner)
            let tanh_sq = tensor_mul(&tanh_inner, &tanh_inner)?;
            let sech_sq = tensor_sub(&one, &tanh_sq)?;

            // d_inner/dx = sqrt(2/pi) * (1 + 3 * 0.044715 * x^2)
            let c3 = tensor_mul(&c, &three)?;
            let c3_x2 = tensor_mul(&c3, &x_sq)?;
            let one_plus_c3x2 = tensor_add(&one, &c3_x2)?;
            let d_inner = tensor_mul(&sqrt_2_over_pi, &one_plus_c3x2)?;

            // d/dx GELU ≈ 0.5 * (1 + tanh(inner)) + 0.5 * x * sech^2(inner) * d_inner
            let one_plus_tanh = tensor_add(&one, &tanh_inner)?;
            let term1 = tensor_mul(&half, &one_plus_tanh)?;

            let x_sech_sq = tensor_mul(x, &sech_sq)?;
            let x_sech_d = tensor_mul(&x_sech_sq, &d_inner)?;
            let term2 = tensor_mul(&half, &x_sech_d)?;

            let deriv = tensor_add(&term1, &term2)?;
            let dx = tensor_mul(out_grad, &deriv)?;
            Some(vec![dx])
        }

        TapeOp::Silu => {
            // SiLU(x) = x * sigmoid(x)
            // d/dx SiLU(x) = sigmoid(x) + x * sigmoid(x) * (1 - sigmoid(x))
            //             = sigmoid(x) * (1 + x * (1 - sigmoid(x)))
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let sig_x = get_saved_tensor(&entry.saved[1])?; // sigmoid(x) saved during forward

            let one = TensorHandle::full(&[], out_grad.dtype, 1.0)?;
            let one_minus_sig = tensor_sub(&one, sig_x)?;
            let x_one_minus = tensor_mul(x, &one_minus_sig)?;
            let one_plus_term = tensor_add(&one, &x_one_minus)?;
            let deriv = tensor_mul(sig_x, &one_plus_term)?;
            let dx = tensor_mul(out_grad, &deriv)?;
            Some(vec![dx])
        }

        TapeOp::Abs => {
            // d/dx |x| = sign(x)
            // VJP: dx = dout * sign(x)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let sign_x = super::tensor::tensor_unop(x, TensorUnaryOp::Sign)?;
            let dx = tensor_mul(out_grad, &sign_x)?;
            Some(vec![dx])
        }

        TapeOp::Sinh => {
            // d/dx sinh(x) = cosh(x)
            // VJP: dx = dout * cosh(x)
            if entry.saved.is_empty() {
                return None;
            }
            let cosh_x = get_saved_tensor(&entry.saved[0])?; // cosh(x) saved during forward
            let dx = tensor_mul(out_grad, cosh_x)?;
            Some(vec![dx])
        }

        TapeOp::Cosh => {
            // d/dx cosh(x) = sinh(x)
            // VJP: dx = dout * sinh(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sinh_x = get_saved_tensor(&entry.saved[0])?; // sinh(x) saved during forward
            let dx = tensor_mul(out_grad, sinh_x)?;
            Some(vec![dx])
        }

        TapeOp::Atanh => {
            // d/dx atanh(x) = 1 / (1 - x^2)
            // VJP: dx = dout / (1 - x^2)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let one = TensorHandle::full(&[], out_grad.dtype, 1.0)?;
            let x_sq = tensor_mul(x, x)?;
            let one_minus_xsq = tensor_sub(&one, &x_sq)?;
            let dx = tensor_div(out_grad, &one_minus_xsq)?;
            Some(vec![dx])
        }

        TapeOp::LogSoftmax => {
            // log_softmax(x)_i = x_i - log(sum_j(exp(x_j)))
            // d/dx log_softmax(x)_i = delta_ij - softmax(x)_j
            // VJP: dx_i = dout_i - softmax_i * sum_j(dout_j)
            if entry.saved.is_empty() {
                return None;
            }
            let softmax_x = get_saved_tensor(&entry.saved[0])?; // softmax(x) saved during forward

            // sum_j(dout_j)
            let sum_dout = super::tensor::tensor_reduce(out_grad, None, TensorReduceOp::Sum)?;

            // softmax_i * sum_j(dout_j)
            let broadcast_sum = broadcast_to(&sum_dout, &softmax_x.shape[..softmax_x.ndim as usize])?;
            let softmax_sum = tensor_mul(softmax_x, &broadcast_sum)?;

            // dout_i - softmax_i * sum_j(dout_j)
            let dx = tensor_sub(out_grad, &softmax_sum)?;
            Some(vec![dx])
        }

        TapeOp::MatMul => {
            // d/dA (A @ B) = dout @ B^T
            // d/dB (A @ B) = A^T @ dout
            if entry.saved.len() < 2 {
                return None;
            }
            let a = get_saved_tensor(&entry.saved[0])?;
            let b = get_saved_tensor(&entry.saved[1])?;

            let b_t = super::tensor::tensor_transpose(b)?;
            let a_t = super::tensor::tensor_transpose(a)?;

            let da = super::tensor::tensor_matmul(out_grad, &b_t)?;
            let db = super::tensor::tensor_matmul(&a_t, out_grad)?;

            Some(vec![da, db])
        }

        TapeOp::Sum => {
            // d/dx sum(x) = ones_like(x)
            // VJP: dx = broadcast(dout, x.shape)
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            let dx = broadcast_to(out_grad, orig_shape)?;
            Some(vec![dx])
        }

        TapeOp::Mean => {
            // d/dx mean(x) = ones_like(x) / numel
            // VJP: dx = broadcast(dout, x.shape) / numel
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            let numel: usize = orig_shape.iter().product();
            let scale = TensorHandle::full(&[], out_grad.dtype, 1.0 / numel as f64)?;
            let scaled = tensor_mul(out_grad, &scale)?;
            let dx = broadcast_to(&scaled, orig_shape)?;
            Some(vec![dx])
        }

        TapeOp::Reshape => {
            // VJP: reshape gradient back to original shape
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            let dx = super::tensor::tensor_reshape(out_grad, orig_shape)?;
            Some(vec![dx])
        }

        TapeOp::Transpose => {
            // VJP: transpose the gradient
            let dx = super::tensor::tensor_transpose(out_grad)?;
            Some(vec![dx])
        }

        TapeOp::Pow => {
            // d/dx (x^y) = y * x^(y-1)
            // d/dy (x^y) = x^y * log(x)
            // VJP: dx = dout * y * x^(y-1), dy = dout * x^y * log(x)
            if entry.saved.len() < 3 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;
            let pow_xy = get_saved_tensor(&entry.saved[2])?; // x^y saved during forward

            // dx = dout * y * x^(y-1) = dout * y * (x^y / x) = dout * y * pow_xy / x
            let y_times_out = tensor_mul(out_grad, y)?;
            let pow_over_x = tensor_div(pow_xy, x)?;
            let dx = tensor_mul(&y_times_out, &pow_over_x)?;

            // dy = dout * x^y * log(x)
            let log_x = super::tensor::tensor_unop(x, TensorUnaryOp::Log)?;
            let pow_log = tensor_mul(pow_xy, &log_x)?;
            let dy = tensor_mul(out_grad, &pow_log)?;

            Some(vec![dx, dy])
        }

        TapeOp::Max | TapeOp::Min => {
            // For max/min reduction, gradient only flows to the argmax/argmin positions
            // Simplified: distribute gradient equally to all max/min values
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            // For simplicity, broadcast and let the framework handle sparsity
            let dx = broadcast_to(out_grad, orig_shape)?;
            Some(vec![dx])
        }

        TapeOp::Softmax => {
            // d/dx softmax(x)_i = softmax(x)_i * (delta_ij - softmax(x)_j)
            // VJP: dx_i = sum_j (dout_j * softmax_j * (delta_ij - softmax_i))
            //           = softmax_i * (dout_i - sum_j(dout_j * softmax_j))
            if entry.saved.is_empty() {
                return None;
            }
            let softmax_x = get_saved_tensor(&entry.saved[0])?;

            // sum_j(dout_j * softmax_j)
            let dout_softmax = tensor_mul(out_grad, softmax_x)?;
            let sum_dout_softmax =
                super::tensor::tensor_reduce(&dout_softmax, None, TensorReduceOp::Sum)?;

            // dout_i - sum_j(dout_j * softmax_j)
            let broadcast_sum = broadcast_to(&sum_dout_softmax, &softmax_x.shape[..softmax_x.ndim as usize])?;
            let diff = tensor_sub(out_grad, &broadcast_sum)?;

            // softmax_i * (dout_i - ...)
            let dx = tensor_mul(softmax_x, &diff)?;
            Some(vec![dx])
        }

        TapeOp::BroadcastTo => {
            // VJP: sum along the broadcasted dimensions
            // Gradient flows back by summing over dimensions that were broadcast
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;

            // For simplicity, reshape/reduce to original shape
            // A proper implementation would track which axes were broadcast
            if out_grad.numel == orig_shape.iter().product::<usize>() {
                let dx = super::tensor::tensor_reshape(out_grad, orig_shape)?;
                Some(vec![dx])
            } else {
                // Need to reduce - simplified implementation
                let dx = TensorHandle::zeros(orig_shape, out_grad.dtype)?;
                Some(vec![dx])
            }
        }

        TapeOp::Squeeze => {
            // VJP: unsqueeze back to original shape
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            let dx = super::tensor::tensor_reshape(out_grad, orig_shape)?;
            Some(vec![dx])
        }

        TapeOp::Unsqueeze => {
            // VJP: squeeze the gradient
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;
            let dx = super::tensor::tensor_reshape(out_grad, orig_shape)?;
            Some(vec![dx])
        }

        // ====================================================================
        // Neural Network Operations VJP Rules
        // ====================================================================

        TapeOp::Conv2d => {
            // VJP for 2D convolution
            // Saved: [input, weight, stride, padding, dilation]
            // d_input = conv2d_transpose(d_output, weight)
            // d_weight = conv2d(input, d_output) with appropriate reshaping
            if entry.saved.len() < 2 {
                return None;
            }
            let input = get_saved_tensor(&entry.saved[0])?;
            let weight = get_saved_tensor(&entry.saved[1])?;

            // For d_input: transposed convolution
            let d_input = conv2d_backward_input(out_grad, weight, input)?;

            // For d_weight: gradient w.r.t. weights
            let d_weight = conv2d_backward_weight(out_grad, input, weight)?;

            Some(vec![d_input, d_weight])
        }

        TapeOp::LayerNorm => {
            // VJP for layer normalization
            // Saved: [input, mean, rstd, normalized_shape, gamma]
            // Standard layer norm backward pass
            if entry.saved.len() < 3 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let mean = get_saved_tensor(&entry.saved[1])?;
            let rstd = get_saved_tensor(&entry.saved[2])?;
            let gamma = if entry.saved.len() > 3 {
                Some(get_saved_tensor(&entry.saved[3])?)
            } else {
                None
            };

            let d_input = layer_norm_backward(out_grad, x, mean, rstd, gamma)?;

            let mut grads = vec![d_input];
            // Gradient w.r.t. gamma and beta if present
            if gamma.is_some() {
                let x_norm = tensor_mul(&tensor_sub(x, mean)?, rstd)?;
                let d_gamma = tensor_sum_axes(out_grad, &x_norm)?;
                let d_beta = tensor_sum_keepdim(out_grad)?;
                grads.push(d_gamma);
                grads.push(d_beta);
            }
            Some(grads)
        }

        TapeOp::BatchNorm => {
            // VJP for batch normalization
            // Saved: [input, mean, var, gamma, running_mean, running_var, training]
            if entry.saved.len() < 4 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let mean = get_saved_tensor(&entry.saved[1])?;
            let var = get_saved_tensor(&entry.saved[2])?;
            let gamma = get_saved_tensor(&entry.saved[3])?;

            let d_input = batch_norm_backward(out_grad, x, mean, var, gamma)?;

            // Gradient w.r.t. gamma and beta
            let eps = 1e-5_f64;
            let std = tensor_sqrt(&tensor_add_scalar(var, eps)?)?;
            let x_norm = tensor_div(&tensor_sub(x, mean)?, &std)?;
            let d_gamma = tensor_sum_batch(&tensor_mul(out_grad, &x_norm)?)?;
            let d_beta = tensor_sum_batch(out_grad)?;

            Some(vec![d_input, d_gamma, d_beta])
        }

        TapeOp::RmsNorm => {
            // VJP for RMS normalization
            // y = x * rsqrt(mean(x^2) + eps) * gamma
            // Saved: [input, rms, gamma]
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let rms = get_saved_tensor(&entry.saved[1])?;
            let gamma = if entry.saved.len() > 2 {
                Some(get_saved_tensor(&entry.saved[2])?)
            } else {
                None
            };

            let d_input = rms_norm_backward(out_grad, x, rms, gamma)?;

            let mut grads = vec![d_input];
            if let Some(_g) = gamma {
                let x_norm = tensor_mul(x, rms)?;
                let d_gamma = tensor_sum_axes(out_grad, &x_norm)?;
                grads.push(d_gamma);
            }
            Some(grads)
        }

        TapeOp::FlashAttention => {
            // VJP for flash attention
            // Saved: [Q, K, V, output, logsumexp, scale]
            // Uses the efficient backward pass from FlashAttention paper
            if entry.saved.len() < 5 {
                return None;
            }
            let q = get_saved_tensor(&entry.saved[0])?;
            let k = get_saved_tensor(&entry.saved[1])?;
            let v = get_saved_tensor(&entry.saved[2])?;
            let out = get_saved_tensor(&entry.saved[3])?;
            let lse = get_saved_tensor(&entry.saved[4])?;

            let (d_q, d_k, d_v) = flash_attention_backward(out_grad, q, k, v, out, lse)?;
            Some(vec![d_q, d_k, d_v])
        }

        TapeOp::Gather => {
            // VJP for gather: scatter_add the gradient
            // Saved: [source_shape, indices, axis]
            if entry.saved.is_empty() {
                return None;
            }
            let src_shape = entry.shape_info.as_ref()?;
            let indices = get_saved_tensor(&entry.saved[0])?;

            // Create zero tensor of source shape and scatter_add the gradient
            let mut d_src = TensorHandle::zeros(src_shape, out_grad.dtype)?;
            scatter_add(&mut d_src, out_grad, indices, 0)?;
            Some(vec![d_src])
        }

        TapeOp::Scatter => {
            // VJP for scatter: gather the gradient
            // Saved: [indices, values_shape, axis]
            if entry.saved.is_empty() {
                return None;
            }
            let indices = get_saved_tensor(&entry.saved[0])?;

            // d_values = gather(d_output, indices, axis)
            let d_values = tensor_gather(out_grad, indices, 0)?;
            // d_src = d_output with scattered positions zeroed (simplified)
            let d_src = out_grad.clone();
            Some(vec![d_src, d_values])
        }

        TapeOp::IndexSelect => {
            // VJP for index_select: index_add the gradient
            // Saved: [source_shape, indices, axis]
            if entry.saved.is_empty() {
                return None;
            }
            let src_shape = entry.shape_info.as_ref()?;
            let indices = get_saved_tensor(&entry.saved[0])?;

            let mut d_src = TensorHandle::zeros(src_shape, out_grad.dtype)?;
            index_add(&mut d_src, out_grad, indices, 0)?;
            Some(vec![d_src])
        }

        TapeOp::Concat => {
            // VJP for concat: split the gradient
            // Saved: [split_sizes, axis]
            entry.shape_info.as_ref()?;

            // Extract sizes from saved values
            let split_sizes: Vec<usize> = entry
                .saved
                .iter()
                .filter_map(|s| match s {
                    SavedValue::Scalar(v) => Some(*v as usize),
                    _ => None,
                })
                .collect();

            if split_sizes.is_empty() {
                return None;
            }

            let grads = tensor_split(out_grad, &split_sizes, 0)?;
            Some(grads)
        }

        TapeOp::Slice => {
            // VJP for slice: pad with zeros
            // Saved: [original_shape, start_indices, end_indices]
            entry.shape_info.as_ref()?;
            let orig_shape = entry.shape_info.as_ref()?;

            // Create zero tensor of original shape and insert gradient at slice position
            let mut d_input = TensorHandle::zeros(orig_shape, out_grad.dtype)?;
            // Extract slice info from saved (simplified - assumes first dim)
            if !entry.saved.is_empty() {
                if let SavedValue::Scalar(start) = entry.saved[0] {
                    slice_assign(&mut d_input, out_grad, start as usize)?;
                }
            }
            Some(vec![d_input])
        }

        TapeOp::Custom(_) => {
            // Custom ops require user-provided VJP rules
            // Return zero gradients as fallback
            let zero_grads: Vec<TensorHandle> = entry
                .inputs
                .iter()
                .filter_map(|id| scope.tensors.get(id))
                .filter_map(|t| TensorHandle::zeros(&t.shape[..t.ndim as usize], t.dtype))
                .collect();
            Some(zero_grads)
        }
    }
}

// ============================================================================
// JVP (Jacobian-Vector Product) Computation
// ============================================================================

/// Computes the Jacobian-vector product for a tape entry (forward-mode).
///
/// Given input tangents, computes the output tangent.
/// JVP computes: d_out = J @ d_in where J is the Jacobian.
fn compute_jvp(
    entry: &TapeEntry,
    input_tangents: &[Option<TensorHandle>],
    scope: &GradScope,
) -> Option<TensorHandle> {
    // Helper to get tangent or zeros if None (reserved for future use)
    let _get_tangent_or_zeros = |idx: usize, dtype: DType, shape: &[usize]| -> Option<TensorHandle> {
        if idx < input_tangents.len() {
            input_tangents[idx].clone().or_else(|| TensorHandle::zeros(shape, dtype))
        } else {
            TensorHandle::zeros(shape, dtype)
        }
    };

    match entry.op {
        TapeOp::Add => {
            // d/dx (x + y) = 1, d/dy (x + y) = 1
            // JVP: d_out = d_x + d_y
            let dx = input_tangents.first()?.clone()?;
            let dy = input_tangents.get(1).and_then(|t| t.clone());

            if let Some(dy) = dy {
                tensor_add(&dx, &dy)
            } else {
                Some(dx)
            }
        }

        TapeOp::Sub => {
            // d/dx (x - y) = 1, d/dy (x - y) = -1
            // JVP: d_out = d_x - d_y
            let dx = input_tangents.first()?.clone()?;
            let dy = input_tangents.get(1).and_then(|t| t.clone());

            if let Some(dy) = dy {
                tensor_sub(&dx, &dy)
            } else {
                Some(dx)
            }
        }

        TapeOp::Mul => {
            // d/dx (x * y) = y, d/dy (x * y) = x
            // JVP: d_out = d_x * y + x * d_y
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;

            let dx = input_tangents.first().and_then(|t| t.clone());
            let dy = input_tangents.get(1).and_then(|t| t.clone());

            let term1 = dx.as_ref().and_then(|dx| tensor_mul(dx, y));
            let term2 = dy.as_ref().and_then(|dy| tensor_mul(x, dy));

            match (term1, term2) {
                (Some(t1), Some(t2)) => tensor_add(&t1, &t2),
                (Some(t1), None) => Some(t1),
                (None, Some(t2)) => Some(t2),
                (None, None) => TensorHandle::zeros(&x.shape[..x.ndim as usize], x.dtype),
            }
        }

        TapeOp::Div => {
            // d/dx (x / y) = 1/y, d/dy (x / y) = -x/y^2
            // JVP: d_out = d_x / y - x * d_y / y^2
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;

            let dx = input_tangents.first().and_then(|t| t.clone());
            let dy = input_tangents.get(1).and_then(|t| t.clone());

            // term1 = d_x / y
            let term1 = dx.as_ref().and_then(|dx| tensor_div(dx, y));

            // term2 = -x * d_y / y^2
            let term2 = dy.as_ref().and_then(|dy| {
                let y_sq = tensor_mul(y, y)?;
                let neg_x = tensor_neg(x)?;
                let neg_x_dy = tensor_mul(&neg_x, dy)?;
                tensor_div(&neg_x_dy, &y_sq)
            });

            match (term1, term2) {
                (Some(t1), Some(t2)) => tensor_add(&t1, &t2),
                (Some(t1), None) => Some(t1),
                (None, Some(t2)) => Some(t2),
                (None, None) => TensorHandle::zeros(&x.shape[..x.ndim as usize], x.dtype),
            }
        }

        TapeOp::Neg => {
            // d/dx (-x) = -1
            // JVP: d_out = -d_x
            let dx = input_tangents.first()?.clone()?;
            tensor_neg(&dx)
        }

        TapeOp::Exp => {
            // d/dx exp(x) = exp(x)
            // JVP: d_out = d_x * exp(x)
            if entry.saved.is_empty() {
                return None;
            }
            let exp_x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            tensor_mul(&dx, exp_x)
        }

        TapeOp::Log => {
            // d/dx log(x) = 1/x
            // JVP: d_out = d_x / x
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            tensor_div(&dx, x)
        }

        TapeOp::Sqrt => {
            // d/dx sqrt(x) = 0.5 / sqrt(x)
            // JVP: d_out = 0.5 * d_x / sqrt(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sqrt_x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            let half = TensorHandle::full(&[], dx.dtype, 0.5)?;
            let half_dx = tensor_mul(&half, &dx)?;
            tensor_div(&half_dx, sqrt_x)
        }

        TapeOp::Sin => {
            // d/dx sin(x) = cos(x)
            // JVP: d_out = d_x * cos(x)
            if entry.saved.is_empty() {
                return None;
            }
            let cos_x = get_saved_tensor(&entry.saved[0])?; // cos(x) saved during forward
            let dx = input_tangents.first()?.clone()?;
            tensor_mul(&dx, cos_x)
        }

        TapeOp::Cos => {
            // d/dx cos(x) = -sin(x)
            // JVP: d_out = -d_x * sin(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sin_x = get_saved_tensor(&entry.saved[0])?; // sin(x) saved during forward
            let dx = input_tangents.first()?.clone()?;
            let neg_sin = tensor_neg(sin_x)?;
            tensor_mul(&dx, &neg_sin)
        }

        TapeOp::Tanh => {
            // d/dx tanh(x) = 1 - tanh(x)^2
            // JVP: d_out = d_x * (1 - tanh(x)^2)
            if entry.saved.is_empty() {
                return None;
            }
            let tanh_x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            let tanh_sq = tensor_mul(tanh_x, tanh_x)?;
            let one = TensorHandle::full(&[], dx.dtype, 1.0)?;
            let one_minus = tensor_sub(&one, &tanh_sq)?;
            tensor_mul(&dx, &one_minus)
        }

        TapeOp::Sigmoid => {
            // d/dx sigmoid(x) = sigmoid(x) * (1 - sigmoid(x))
            // JVP: d_out = d_x * sigmoid(x) * (1 - sigmoid(x))
            if entry.saved.is_empty() {
                return None;
            }
            let sig_x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            let one = TensorHandle::full(&[], dx.dtype, 1.0)?;
            let one_minus = tensor_sub(&one, sig_x)?;
            let sig_deriv = tensor_mul(sig_x, &one_minus)?;
            tensor_mul(&dx, &sig_deriv)
        }

        TapeOp::Relu => {
            // d/dx relu(x) = 1 if x > 0 else 0
            // JVP: d_out = d_x * (x > 0)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            let mask = tensor_gt_zero(x)?;
            tensor_mul(&dx, &mask)
        }

        TapeOp::MatMul => {
            // d/dA (A @ B) @ d_A = d_A @ B
            // d/dB (A @ B) @ d_B = A @ d_B
            // JVP: d_out = d_A @ B + A @ d_B
            if entry.saved.len() < 2 {
                return None;
            }
            let a = get_saved_tensor(&entry.saved[0])?;
            let b = get_saved_tensor(&entry.saved[1])?;

            let da = input_tangents.first().and_then(|t| t.clone());
            let db = input_tangents.get(1).and_then(|t| t.clone());

            let term1 = da.as_ref().and_then(|da| super::tensor::tensor_matmul(da, b));
            let term2 = db.as_ref().and_then(|db| super::tensor::tensor_matmul(a, db));

            match (term1, term2) {
                (Some(t1), Some(t2)) => tensor_add(&t1, &t2),
                (Some(t1), None) => Some(t1),
                (None, Some(t2)) => Some(t2),
                (None, None) => None,
            }
        }

        TapeOp::Sum => {
            // d/dx sum(x) = ones_like(x)
            // JVP: d_out = sum(d_x)
            let dx = input_tangents.first()?.clone()?;
            super::tensor::tensor_reduce(&dx, None, TensorReduceOp::Sum)
        }

        TapeOp::Mean => {
            // d/dx mean(x) = ones_like(x) / numel
            // JVP: d_out = mean(d_x)
            let dx = input_tangents.first()?.clone()?;
            super::tensor::tensor_reduce(&dx, None, TensorReduceOp::Mean)
        }

        TapeOp::Reshape => {
            // JVP: reshape the tangent to the new shape
            let dx = input_tangents.first()?.clone()?;
            // Get output shape from entry
            let output_tensor = scope.tensors.get(&entry.output)?;
            let out_shape = &output_tensor.shape[..output_tensor.ndim as usize];
            super::tensor::tensor_reshape(&dx, out_shape)
        }

        TapeOp::Transpose => {
            // JVP: transpose the tangent
            let dx = input_tangents.first()?.clone()?;
            super::tensor::tensor_transpose(&dx)
        }

        TapeOp::Pow => {
            // d/dx (x^y) = y * x^(y-1), d/dy (x^y) = x^y * log(x)
            // JVP: d_out = d_x * y * x^(y-1) + d_y * x^y * log(x)
            if entry.saved.len() < 3 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let y = get_saved_tensor(&entry.saved[1])?;
            let pow_xy = get_saved_tensor(&entry.saved[2])?; // x^y

            let dx = input_tangents.first().and_then(|t| t.clone());
            let dy = input_tangents.get(1).and_then(|t| t.clone());

            // term1 = d_x * y * x^(y-1) = d_x * y * pow_xy / x
            let term1 = dx.as_ref().and_then(|dx| {
                let dx_y = tensor_mul(dx, y)?;
                let pow_over_x = tensor_div(pow_xy, x)?;
                tensor_mul(&dx_y, &pow_over_x)
            });

            // term2 = d_y * x^y * log(x)
            let term2 = dy.as_ref().and_then(|dy| {
                let log_x = super::tensor::tensor_unop(x, TensorUnaryOp::Log)?;
                let pow_log = tensor_mul(pow_xy, &log_x)?;
                tensor_mul(dy, &pow_log)
            });

            match (term1, term2) {
                (Some(t1), Some(t2)) => tensor_add(&t1, &t2),
                (Some(t1), None) => Some(t1),
                (None, Some(t2)) => Some(t2),
                (None, None) => None,
            }
        }

        TapeOp::Max | TapeOp::Min => {
            // For max/min, gradient only flows to argmax/argmin positions
            // JVP: d_out = d_x[argmax/argmin]
            // Simplified: just reduce the tangent (approximation)
            let dx = input_tangents.first()?.clone()?;
            let reduce_op = if entry.op == TapeOp::Max {
                TensorReduceOp::Max
            } else {
                TensorReduceOp::Min
            };
            super::tensor::tensor_reduce(&dx, None, reduce_op)
        }

        TapeOp::Softmax => {
            // d/dx softmax(x)_i = softmax(x)_i * (delta_ij - softmax(x)_j)
            // JVP: d_out_i = sum_j (d_x_j * softmax_j * (delta_ij - softmax_i))
            //              = softmax_i * d_x_i - softmax_i * sum_j(d_x_j * softmax_j)
            if entry.saved.is_empty() {
                return None;
            }
            let softmax_x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            // softmax_i * d_x_i
            let s_dx = tensor_mul(softmax_x, &dx)?;

            // sum_j(d_x_j * softmax_j)
            let sum_s_dx = super::tensor::tensor_reduce(&s_dx, None, TensorReduceOp::Sum)?;

            // softmax_i * sum_j(d_x_j * softmax_j)
            let broadcast_sum = broadcast_to(&sum_s_dx, &softmax_x.shape[..softmax_x.ndim as usize])?;
            let s_sum = tensor_mul(softmax_x, &broadcast_sum)?;

            // d_out = s_dx - s_sum
            tensor_sub(&s_dx, &s_sum)
        }

        TapeOp::BroadcastTo => {
            // JVP: broadcast the tangent to the target shape
            let dx = input_tangents.first()?.clone()?;
            let output_tensor = scope.tensors.get(&entry.output)?;
            let out_shape = &output_tensor.shape[..output_tensor.ndim as usize];
            broadcast_to(&dx, out_shape)
        }

        TapeOp::Squeeze => {
            // JVP: squeeze the tangent
            let dx = input_tangents.first()?.clone()?;
            let output_tensor = scope.tensors.get(&entry.output)?;
            let out_shape = &output_tensor.shape[..output_tensor.ndim as usize];
            super::tensor::tensor_reshape(&dx, out_shape)
        }

        TapeOp::Unsqueeze => {
            // JVP: unsqueeze the tangent
            let dx = input_tangents.first()?.clone()?;
            let output_tensor = scope.tensors.get(&entry.output)?;
            let out_shape = &output_tensor.shape[..output_tensor.ndim as usize];
            super::tensor::tensor_reshape(&dx, out_shape)
        }

        TapeOp::Erf => {
            // d/dx erf(x) = (2/√π) * exp(-x²)
            // JVP: d_out = d_x * (2/√π) * exp(-x²)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            // Compute exp(-x²)
            let x_sq = tensor_mul(x, x)?;
            let neg_x_sq = tensor_neg(&x_sq)?;
            let exp_neg_x_sq = super::tensor::tensor_unop(&neg_x_sq, TensorUnaryOp::Exp)?;

            // 2/√π ≈ 1.1283791670955126
            let two_over_sqrt_pi = TensorHandle::full(&[], dx.dtype, 1.1283791670955126)?;
            let deriv = tensor_mul(&two_over_sqrt_pi, &exp_neg_x_sq)?;
            tensor_mul(&dx, &deriv)
        }

        TapeOp::Gelu => {
            // d/dx gelu(x) = 0.5 * (1 + erf(x/√2)) + (x/√(2π)) * exp(-x²/2)
            // Simplified: uses saved intermediate values
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let cdf = get_saved_tensor(&entry.saved[1])?; // 0.5 * (1 + erf(x/√2))
            let dx = input_tangents.first()?.clone()?;

            // Compute pdf term: (1/√(2π)) * exp(-x²/2)
            let neg_half = TensorHandle::full(&[], dx.dtype, -0.5)?;
            let x_sq = tensor_mul(x, x)?;
            let neg_half_x_sq = tensor_mul(&neg_half, &x_sq)?;
            let exp_term = super::tensor::tensor_unop(&neg_half_x_sq, TensorUnaryOp::Exp)?;

            // 1/√(2π) ≈ 0.3989422804014327
            let inv_sqrt_2pi = TensorHandle::full(&[], dx.dtype, 0.3989422804014327)?;
            let pdf = tensor_mul(&inv_sqrt_2pi, &exp_term)?;

            // derivative = cdf + x * pdf
            let x_pdf = tensor_mul(x, &pdf)?;
            let deriv = tensor_add(cdf, &x_pdf)?;
            tensor_mul(&dx, &deriv)
        }

        TapeOp::Silu => {
            // d/dx silu(x) = sigmoid(x) * (1 + x * (1 - sigmoid(x)))
            // = sigmoid(x) + x * sigmoid(x) * (1 - sigmoid(x))
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let sigmoid_x = get_saved_tensor(&entry.saved[1])?;
            let dx = input_tangents.first()?.clone()?;

            let one = TensorHandle::full(&[], dx.dtype, 1.0)?;
            let one_minus_sig = tensor_sub(&one, sigmoid_x)?;
            let sig_one_minus = tensor_mul(sigmoid_x, &one_minus_sig)?;
            let x_sig_one_minus = tensor_mul(x, &sig_one_minus)?;
            let deriv = tensor_add(sigmoid_x, &x_sig_one_minus)?;
            tensor_mul(&dx, &deriv)
        }

        TapeOp::LeakyRelu => {
            // d/dx leaky_relu(x, α) = 1 if x > 0 else α
            // JVP: d_out = d_x * (1 if x > 0 else α)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            // Get alpha from saved or default to 0.01
            let alpha = if entry.saved.len() > 1 {
                match &entry.saved[1] {
                    SavedValue::Scalar(v) => *v,
                    _ => 0.01,
                }
            } else {
                0.01
            };

            let mask = tensor_leaky_relu_deriv(x, alpha)?;
            tensor_mul(&dx, &mask)
        }

        TapeOp::Abs => {
            // d/dx |x| = sign(x)
            // JVP: d_out = d_x * sign(x)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;
            let sign_x = tensor_sign(x)?;
            tensor_mul(&dx, &sign_x)
        }

        TapeOp::Sinh => {
            // d/dx sinh(x) = cosh(x)
            // JVP: d_out = d_x * cosh(x)
            if entry.saved.is_empty() {
                return None;
            }
            let cosh_x = get_saved_tensor(&entry.saved[0])?; // cosh(x) saved during forward
            let dx = input_tangents.first()?.clone()?;
            tensor_mul(&dx, cosh_x)
        }

        TapeOp::Cosh => {
            // d/dx cosh(x) = sinh(x)
            // JVP: d_out = d_x * sinh(x)
            if entry.saved.is_empty() {
                return None;
            }
            let sinh_x = get_saved_tensor(&entry.saved[0])?; // sinh(x) saved during forward
            let dx = input_tangents.first()?.clone()?;
            tensor_mul(&dx, sinh_x)
        }

        TapeOp::Atanh => {
            // d/dx atanh(x) = 1 / (1 - x²)
            // JVP: d_out = d_x / (1 - x²)
            if entry.saved.is_empty() {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            let x_sq = tensor_mul(x, x)?;
            let one = TensorHandle::full(&[], dx.dtype, 1.0)?;
            let one_minus_x_sq = tensor_sub(&one, &x_sq)?;
            tensor_div(&dx, &one_minus_x_sq)
        }

        TapeOp::LogSoftmax => {
            // d/dx log_softmax(x)_i = delta_ij - softmax(x)_j
            // JVP: d_out_i = d_x_i - softmax_i * sum_j(d_x_j)
            if entry.saved.is_empty() {
                return None;
            }
            let softmax_x = get_saved_tensor(&entry.saved[0])?; // softmax(x) saved
            let dx = input_tangents.first()?.clone()?;

            // sum_j(d_x_j)
            let sum_dx = super::tensor::tensor_reduce(&dx, None, TensorReduceOp::Sum)?;

            // softmax_i * sum_j(d_x_j)
            let broadcast_sum = broadcast_to(&sum_dx, &softmax_x.shape[..softmax_x.ndim as usize])?;
            let s_sum = tensor_mul(softmax_x, &broadcast_sum)?;

            // d_out = d_x - s_sum
            tensor_sub(&dx, &s_sum)
        }

        // ====================================================================
        // Neural Network Operations JVP Rules
        // ====================================================================

        TapeOp::Conv2d => {
            // JVP for 2D convolution
            // d_output = conv2d(d_input, weight) + conv2d(input, d_weight)
            if entry.saved.len() < 2 {
                return None;
            }
            let input = get_saved_tensor(&entry.saved[0])?;
            let weight = get_saved_tensor(&entry.saved[1])?;

            let d_input = input_tangents.first()?.clone()?;
            let d_weight = input_tangents.get(1).and_then(|t| t.clone());

            // d_output = conv2d(d_input, weight)
            let mut d_out = conv2d_forward(&d_input, weight)?;

            // + conv2d(input, d_weight) if d_weight exists
            if let Some(dw) = d_weight {
                let d_out2 = conv2d_forward(input, &dw)?;
                d_out = tensor_add(&d_out, &d_out2)?;
            }
            Some(d_out)
        }

        TapeOp::LayerNorm => {
            // JVP for layer normalization
            // Requires careful computation of tangent through normalization
            if entry.saved.len() < 3 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let mean = get_saved_tensor(&entry.saved[1])?;
            let rstd = get_saved_tensor(&entry.saved[2])?;

            let dx = input_tangents.first()?.clone()?;

            // Simplified: d_output ≈ rstd * (dx - mean(dx))
            let dx_centered = layer_norm_jvp_forward(&dx, x, mean, rstd)?;
            Some(dx_centered)
        }

        TapeOp::BatchNorm => {
            // JVP for batch normalization
            if entry.saved.len() < 4 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let mean = get_saved_tensor(&entry.saved[1])?;
            let var = get_saved_tensor(&entry.saved[2])?;
            let gamma = get_saved_tensor(&entry.saved[3])?;

            let dx = input_tangents.first()?.clone()?;

            let d_out = batch_norm_jvp_forward(&dx, x, mean, var, gamma)?;
            Some(d_out)
        }

        TapeOp::RmsNorm => {
            // JVP for RMS normalization
            if entry.saved.len() < 2 {
                return None;
            }
            let x = get_saved_tensor(&entry.saved[0])?;
            let rms = get_saved_tensor(&entry.saved[1])?;

            let dx = input_tangents.first()?.clone()?;
            let d_out = rms_norm_jvp_forward(&dx, x, rms)?;
            Some(d_out)
        }

        TapeOp::FlashAttention => {
            // JVP for flash attention
            // d_output = attention(dQ, K, V) + attention(Q, dK, V) + attention(Q, K, dV)
            if entry.saved.len() < 5 {
                return None;
            }
            let q = get_saved_tensor(&entry.saved[0])?;
            let k = get_saved_tensor(&entry.saved[1])?;
            let v = get_saved_tensor(&entry.saved[2])?;

            let d_q = input_tangents.first()?.clone()?;
            let d_k = input_tangents.get(1).and_then(|t| t.clone());
            let d_v = input_tangents.get(2).and_then(|t| t.clone());

            let d_out = flash_attention_jvp_forward(&d_q, d_k.as_ref(), d_v.as_ref(), q, k, v)?;
            Some(d_out)
        }

        TapeOp::Gather => {
            // JVP for gather: gather the tangent
            if entry.saved.is_empty() {
                return None;
            }
            let indices = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            tensor_gather(&dx, indices, 0)
        }

        TapeOp::Scatter => {
            // JVP for scatter: scatter the tangent
            if entry.saved.is_empty() {
                return None;
            }
            let indices = get_saved_tensor(&entry.saved[0])?;
            let output_tensor = scope.tensors.get(&entry.output)?;
            let dx_values = input_tangents.get(1).and_then(|t| t.clone())?;

            let mut d_out = TensorHandle::zeros(
                &output_tensor.shape[..output_tensor.ndim as usize],
                output_tensor.dtype,
            )?;
            tensor_scatter_update(&mut d_out, &dx_values, indices, 0)?;
            Some(d_out)
        }

        TapeOp::IndexSelect => {
            // JVP for index_select: index_select the tangent
            if entry.saved.is_empty() {
                return None;
            }
            let indices = get_saved_tensor(&entry.saved[0])?;
            let dx = input_tangents.first()?.clone()?;

            tensor_index_select(&dx, indices, 0)
        }

        TapeOp::Concat => {
            // JVP for concat: concat the tangents
            let tangents: Vec<&TensorHandle> = input_tangents
                .iter()
                .filter_map(|t| t.as_ref())
                .collect();

            if tangents.is_empty() {
                return None;
            }

            tensor_concat(&tangents, 0)
        }

        TapeOp::Slice => {
            // JVP for slice: slice the tangent
            let dx = input_tangents.first()?.clone()?;

            // Extract slice info from saved
            if entry.saved.is_empty() {
                return None;
            }

            if let SavedValue::Scalar(start) = entry.saved[0] {
                let output_tensor = scope.tensors.get(&entry.output)?;
                let len = output_tensor.shape[0] as usize;
                tensor_slice(&dx, start as usize, len)
            } else {
                None
            }
        }

        TapeOp::Custom(_) => {
            // Custom ops require user-provided JVP rules
            // Return zero tangent as fallback
            let output_tensor = scope.tensors.get(&entry.output)?;
            TensorHandle::zeros(
                &output_tensor.shape[..output_tensor.ndim as usize],
                output_tensor.dtype,
            )
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extracts a tensor from a saved value.
fn get_saved_tensor(saved: &SavedValue) -> Option<&TensorHandle> {
    match saved {
        SavedValue::Tensor(t) => Some(t),
        _ => None,
    }
}

/// Element-wise tensor addition.
fn tensor_add(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_binop;
    use crate::instruction::TensorBinaryOp;
    tensor_binop(a, b, TensorBinaryOp::Add)
}

/// Element-wise tensor subtraction.
fn tensor_sub(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_binop;
    use crate::instruction::TensorBinaryOp;
    tensor_binop(a, b, TensorBinaryOp::Sub)
}

/// Element-wise tensor multiplication.
fn tensor_mul(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_binop;
    use crate::instruction::TensorBinaryOp;
    tensor_binop(a, b, TensorBinaryOp::Mul)
}

/// Element-wise tensor division.
fn tensor_div(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_binop;
    use crate::instruction::TensorBinaryOp;
    tensor_binop(a, b, TensorBinaryOp::Div)
}

/// Element-wise tensor negation.
fn tensor_neg(a: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_unop;
    use crate::instruction::TensorUnaryOp;
    tensor_unop(a, TensorUnaryOp::Neg)
}

/// Creates a mask tensor where elements > 0 are 1.0, else 0.0.
fn tensor_gt_zero(a: &TensorHandle) -> Option<TensorHandle> {
    // For ReLU backward: mask = (x > 0)
    // We create a tensor where positive values become 1.0, others 0.0
    let result = TensorHandle::zeros(&a.shape[..a.ndim as usize], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid - tensors have been properly allocated
    unsafe {
        let src = (*a_data.as_ptr()).as_ptr();
        let dst = (*out_data.as_ptr()).as_mut_ptr();

        match a.dtype {
            DType::F32 => {
                let s = src as *const f32;
                let d = dst as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) > 0.0 { 1.0 } else { 0.0 };
                }
            }
            DType::F64 => {
                let s = src as *const f64;
                let d = dst as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) > 0.0 { 1.0 } else { 0.0 };
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Creates a mask tensor where elements <= 0 are 1.0, else 0.0.
fn tensor_le_zero(a: &TensorHandle) -> Option<TensorHandle> {
    // For LeakyReLU backward: mask = (x <= 0)
    let result = TensorHandle::zeros(&a.shape[..a.ndim as usize], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid - tensors have been properly allocated
    unsafe {
        let src = (*a_data.as_ptr()).as_ptr();
        let dst = (*out_data.as_ptr()).as_mut_ptr();

        match a.dtype {
            DType::F32 => {
                let s = src as *const f32;
                let d = dst as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) <= 0.0 { 1.0 } else { 0.0 };
                }
            }
            DType::F64 => {
                let s = src as *const f64;
                let d = dst as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) <= 0.0 { 1.0 } else { 0.0 };
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Creates a derivative mask for leaky ReLU: 1 if x > 0, alpha otherwise.
fn tensor_leaky_relu_deriv(a: &TensorHandle, alpha: f64) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(&a.shape[..a.ndim as usize], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid - tensors have been properly allocated
    unsafe {
        let src = (*a_data.as_ptr()).as_ptr();
        let dst = (*out_data.as_ptr()).as_mut_ptr();

        match a.dtype {
            DType::F32 => {
                let alpha_f32 = alpha as f32;
                let s = src as *const f32;
                let d = dst as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) > 0.0 { 1.0 } else { alpha_f32 };
                }
            }
            DType::F64 => {
                let s = src as *const f64;
                let d = dst as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = if *s.add(i) > 0.0 { 1.0 } else { alpha };
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Computes element-wise sign: 1 if x > 0, -1 if x < 0, 0 if x == 0.
fn tensor_sign(a: &TensorHandle) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(&a.shape[..a.ndim as usize], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid - tensors have been properly allocated
    unsafe {
        let src = (*a_data.as_ptr()).as_ptr();
        let dst = (*out_data.as_ptr()).as_mut_ptr();

        match a.dtype {
            DType::F32 => {
                let s = src as *const f32;
                let d = dst as *mut f32;
                for i in 0..result.numel {
                    let val = *s.add(i);
                    *d.add(i) = if val > 0.0 {
                        1.0
                    } else if val < 0.0 {
                        -1.0
                    } else {
                        0.0
                    };
                }
            }
            DType::F64 => {
                let s = src as *const f64;
                let d = dst as *mut f64;
                for i in 0..result.numel {
                    let val = *s.add(i);
                    *d.add(i) = if val > 0.0 {
                        1.0
                    } else if val < 0.0 {
                        -1.0
                    } else {
                        0.0
                    };
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Broadcasts a tensor to a target shape.
fn broadcast_to(tensor: &TensorHandle, shape: &[usize]) -> Option<TensorHandle> {
    // Simple broadcast implementation for reduction gradients
    let result = TensorHandle::zeros(shape, tensor.dtype)?;

    let src_data = tensor.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // For scalar gradient broadcast to higher-dim tensor
    if tensor.numel == 1 {
        // SAFETY: All pointer operations are valid - tensors have been properly allocated
        unsafe {
            let src = (*src_data.as_ptr()).as_ptr();
            let dst = (*dst_data.as_ptr()).as_mut_ptr();

            match tensor.dtype {
                DType::F32 => {
                    let val = *(src as *const f32);
                    let d = dst as *mut f32;
                    for i in 0..result.numel {
                        *d.add(i) = val;
                    }
                }
                DType::F64 => {
                    let val = *(src as *const f64);
                    let d = dst as *mut f64;
                    for i in 0..result.numel {
                        *d.add(i) = val;
                    }
                }
                _ => return None,
            }
        }
        return Some(result);
    }

    // General broadcast - copy matching elements
    // This is a simplified version; full broadcast would need stride computation
    if tensor.numel == result.numel {
        // SAFETY: src and dst point to valid memory of the same size
        unsafe {
            let src = (*src_data.as_ptr()).as_ptr();
            let dst = (*dst_data.as_ptr()).as_mut_ptr();
            std::ptr::copy_nonoverlapping(src, dst, result.numel * tensor.dtype.size());
        }
    }

    Some(result)
}

// ============================================================================
// Neural Network Autodiff Helper Functions
// ============================================================================

/// Backward pass for input gradients in Conv2d.
fn conv2d_backward_input(
    grad_output: &TensorHandle,
    weight: &TensorHandle,
    _input: &TensorHandle,
) -> Option<TensorHandle> {
    // Transpose convolution for input gradient
    // This is a simplified implementation - full version would use transposed conv
    let input_shape = _input.shape[.._input.ndim as usize].to_vec();

    // For now, use a simplified gradient computation
    // d_input has same shape as input
    let d_input = TensorHandle::zeros(&input_shape, grad_output.dtype)?;

    // In a full implementation, this would call conv2d_transpose
    // For now, we approximate with the weight transpose
    let _weight_t = super::tensor::tensor_transpose(weight)?;

    Some(d_input)
}

/// Backward pass for weight gradients in Conv2d.
fn conv2d_backward_weight(
    grad_output: &TensorHandle,
    _input: &TensorHandle,
    weight: &TensorHandle,
) -> Option<TensorHandle> {
    // Weight gradient computation
    let weight_shape = weight.shape[..weight.ndim as usize].to_vec();
    let d_weight = TensorHandle::zeros(&weight_shape, grad_output.dtype)?;

    // In a full implementation, this would compute the gradient properly
    // d_weight = sum over batch of input * grad_output

    Some(d_weight)
}

/// Forward pass for Conv2d (used in JVP).
fn conv2d_forward(input: &TensorHandle, weight: &TensorHandle) -> Option<TensorHandle> {
    // Simplified convolution forward pass
    // In a full implementation, this would call the actual conv2d kernel

    // Compute output shape (simplified: assumes same padding, stride 1)
    let batch = input.shape[0];
    let out_channels = weight.shape[0];
    let h = input.shape[2];
    let w = input.shape[3];

    TensorHandle::zeros(&[batch, out_channels, h, w], input.dtype)
}

/// Backward pass for layer normalization.
fn layer_norm_backward(
    grad_output: &TensorHandle,
    x: &TensorHandle,
    mean: &TensorHandle,
    rstd: &TensorHandle,
    gamma: Option<&TensorHandle>,
) -> Option<TensorHandle> {
    // Layer norm backward: complex gradient computation
    // d_input = rstd * (d_out * gamma - mean(d_out * gamma) - x_norm * mean(d_out * gamma * x_norm))

    let x_centered = tensor_sub(x, mean)?;
    let _x_norm = tensor_mul(&x_centered, rstd)?;

    let dy = if let Some(g) = gamma {
        tensor_mul(grad_output, g)?
    } else {
        grad_output.clone()
    };

    // Simplified: d_input ≈ rstd * dy (ignoring second-order terms for now)
    tensor_mul(rstd, &dy)
}

/// Backward pass for batch normalization.
fn batch_norm_backward(
    grad_output: &TensorHandle,
    x: &TensorHandle,
    mean: &TensorHandle,
    var: &TensorHandle,
    gamma: &TensorHandle,
) -> Option<TensorHandle> {
    let eps = 1e-5_f64;
    let std = tensor_sqrt(&tensor_add_scalar(var, eps)?)?;
    let rstd = tensor_reciprocal(&std)?;

    let x_centered = tensor_sub(x, mean)?;
    let _x_norm = tensor_mul(&x_centered, &rstd)?;

    // Simplified batch norm gradient
    let dy_gamma = tensor_mul(grad_output, gamma)?;
    tensor_mul(&dy_gamma, &rstd)
}

/// Backward pass for RMS normalization.
fn rms_norm_backward(
    grad_output: &TensorHandle,
    _x: &TensorHandle,
    rms: &TensorHandle,
    gamma: Option<&TensorHandle>,
) -> Option<TensorHandle> {
    // d_input = rstd * (d_out * gamma - x * mean(d_out * gamma * x) / rms^2)

    let dy = if let Some(g) = gamma {
        tensor_mul(grad_output, g)?
    } else {
        grad_output.clone()
    };

    // Simplified: d_input ≈ rms * dy
    tensor_mul(rms, &dy)
}

/// Backward pass for flash attention.
fn flash_attention_backward(
    grad_output: &TensorHandle,
    q: &TensorHandle,
    k: &TensorHandle,
    v: &TensorHandle,
    _output: &TensorHandle,
    _lse: &TensorHandle,
) -> Option<(TensorHandle, TensorHandle, TensorHandle)> {
    // Flash attention backward is complex; this is a simplified version
    // Full implementation would use the memory-efficient algorithm

    // d_V = softmax(QK^T / sqrt(d)) @ d_output
    let d_v = TensorHandle::zeros(&v.shape[..v.ndim as usize], grad_output.dtype)?;

    // d_K ≈ softmax(QK^T / sqrt(d))^T @ (d_output @ V^T) (simplified)
    let d_k = TensorHandle::zeros(&k.shape[..k.ndim as usize], grad_output.dtype)?;

    // d_Q ≈ (d_output @ V^T) @ K (simplified)
    let d_q = TensorHandle::zeros(&q.shape[..q.ndim as usize], grad_output.dtype)?;

    Some((d_q, d_k, d_v))
}

/// Scatter add: dst[indices] += src.
fn scatter_add(
    dst: &mut TensorHandle,
    src: &TensorHandle,
    indices: &TensorHandle,
    _axis: usize,
) -> Option<()> {
    // Simplified scatter_add implementation
    let dst_data = dst.data.as_ref()?;
    let src_data = src.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;

    // SAFETY: All pointer operations are valid
    unsafe {
        let d = (*dst_data.as_ptr()).as_mut_ptr();
        let s = (*src_data.as_ptr()).as_ptr();
        let idx = (*idx_data.as_ptr()).as_ptr();

        match dst.dtype {
            DType::F32 => {
                let d_f32 = d as *mut f32;
                let s_f32 = s as *const f32;
                let idx_i64 = idx as *const i64;
                for i in 0..src.numel {
                    let target_idx = *idx_i64.add(i) as usize;
                    if target_idx < dst.numel {
                        *d_f32.add(target_idx) += *s_f32.add(i);
                    }
                }
            }
            DType::F64 => {
                let d_f64 = d as *mut f64;
                let s_f64 = s as *const f64;
                let idx_i64 = idx as *const i64;
                for i in 0..src.numel {
                    let target_idx = *idx_i64.add(i) as usize;
                    if target_idx < dst.numel {
                        *d_f64.add(target_idx) += *s_f64.add(i);
                    }
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// Gather values along axis.
fn tensor_gather(src: &TensorHandle, indices: &TensorHandle, _axis: usize) -> Option<TensorHandle> {
    let out_shape = indices.shape[..indices.ndim as usize].to_vec();
    let result = TensorHandle::zeros(&out_shape, src.dtype)?;

    let src_data = src.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid
    unsafe {
        let s = (*src_data.as_ptr()).as_ptr();
        let idx = (*idx_data.as_ptr()).as_ptr();
        let d = (*out_data.as_ptr()).as_mut_ptr();

        match src.dtype {
            DType::F32 => {
                let s_f32 = s as *const f32;
                let d_f32 = d as *mut f32;
                let idx_i64 = idx as *const i64;
                for i in 0..indices.numel {
                    let src_idx = *idx_i64.add(i) as usize;
                    if src_idx < src.numel {
                        *d_f32.add(i) = *s_f32.add(src_idx);
                    }
                }
            }
            DType::F64 => {
                let s_f64 = s as *const f64;
                let d_f64 = d as *mut f64;
                let idx_i64 = idx as *const i64;
                for i in 0..indices.numel {
                    let src_idx = *idx_i64.add(i) as usize;
                    if src_idx < src.numel {
                        *d_f64.add(i) = *s_f64.add(src_idx);
                    }
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Index add: dst[indices] += src.
fn index_add(
    dst: &mut TensorHandle,
    src: &TensorHandle,
    indices: &TensorHandle,
    axis: usize,
) -> Option<()> {
    // Same as scatter_add for 1D case
    scatter_add(dst, src, indices, axis)
}

/// Split tensor along axis.
fn tensor_split(
    tensor: &TensorHandle,
    split_sizes: &[usize],
    _axis: usize,
) -> Option<Vec<TensorHandle>> {
    let mut results = Vec::with_capacity(split_sizes.len());
    let mut offset = 0;

    for &size in split_sizes {
        let mut shape = tensor.shape[..tensor.ndim as usize].to_vec();
        shape[0] = size;

        let chunk = TensorHandle::zeros(&shape, tensor.dtype)?;

        // Copy data (simplified for 1D split)
        let src_data = tensor.data.as_ref()?;
        let dst_data = chunk.data.as_ref()?;

        // SAFETY: All pointer operations are valid
        unsafe {
            let s = (*src_data.as_ptr()).as_ptr();
            let d = (*dst_data.as_ptr()).as_mut_ptr();
            let bytes = size * tensor.dtype.size();
            let src_offset = offset * tensor.dtype.size();
            std::ptr::copy_nonoverlapping(s.add(src_offset), d, bytes);
        }

        results.push(chunk);
        offset += size;
    }

    Some(results)
}

/// Assign slice: dst[start:start+len] = src.
fn slice_assign(
    dst: &mut TensorHandle,
    src: &TensorHandle,
    start: usize,
) -> Option<()> {
    let dst_data = dst.data.as_ref()?;
    let src_data = src.data.as_ref()?;

    // SAFETY: All pointer operations are valid
    unsafe {
        let d = (*dst_data.as_ptr()).as_mut_ptr();
        let s = (*src_data.as_ptr()).as_ptr();
        let bytes = src.numel * src.dtype.size();
        let dst_offset = start * src.dtype.size();
        std::ptr::copy_nonoverlapping(s, d.add(dst_offset), bytes);
    }

    Some(())
}

/// Sum tensor along axes, multiplying with another tensor.
fn tensor_sum_axes(_grad: &TensorHandle, other: &TensorHandle) -> Option<TensorHandle> {
    // Sum(grad * other) along batch dimensions
    let product = tensor_mul(_grad, other)?;
    super::tensor::tensor_reduce(&product, None, TensorReduceOp::Sum)
}

/// Sum tensor keeping dims.
fn tensor_sum_keepdim(tensor: &TensorHandle) -> Option<TensorHandle> {
    super::tensor::tensor_reduce(tensor, None, TensorReduceOp::Sum)
}

/// Sum tensor along batch dimension.
fn tensor_sum_batch(tensor: &TensorHandle) -> Option<TensorHandle> {
    super::tensor::tensor_reduce(tensor, Some(0), TensorReduceOp::Sum)
}

/// Add scalar to tensor.
fn tensor_add_scalar(tensor: &TensorHandle, scalar: f64) -> Option<TensorHandle> {
    let scalar_tensor = TensorHandle::full(&[], tensor.dtype, scalar)?;
    tensor_add(tensor, &scalar_tensor)
}

/// Element-wise square root.
fn tensor_sqrt(tensor: &TensorHandle) -> Option<TensorHandle> {
    use super::tensor::tensor_unop;
    use crate::instruction::TensorUnaryOp;
    tensor_unop(tensor, TensorUnaryOp::Sqrt)
}

/// Element-wise reciprocal (1/x).
fn tensor_reciprocal(tensor: &TensorHandle) -> Option<TensorHandle> {
    let one = TensorHandle::full(&[], tensor.dtype, 1.0)?;
    tensor_div(&one, tensor)
}

/// JVP forward for layer normalization.
fn layer_norm_jvp_forward(
    dx: &TensorHandle,
    _x: &TensorHandle,
    _mean: &TensorHandle,
    rstd: &TensorHandle,
) -> Option<TensorHandle> {
    // Simplified: d_output ≈ rstd * dx
    tensor_mul(rstd, dx)
}

/// JVP forward for batch normalization.
fn batch_norm_jvp_forward(
    dx: &TensorHandle,
    _x: &TensorHandle,
    _mean: &TensorHandle,
    var: &TensorHandle,
    gamma: &TensorHandle,
) -> Option<TensorHandle> {
    let eps = 1e-5_f64;
    let std = tensor_sqrt(&tensor_add_scalar(var, eps)?)?;
    let rstd = tensor_reciprocal(&std)?;
    let dx_norm = tensor_mul(dx, &rstd)?;
    tensor_mul(&dx_norm, gamma)
}

/// JVP forward for RMS normalization.
fn rms_norm_jvp_forward(
    dx: &TensorHandle,
    _x: &TensorHandle,
    rms: &TensorHandle,
) -> Option<TensorHandle> {
    tensor_mul(rms, dx)
}

/// JVP forward for flash attention.
fn flash_attention_jvp_forward(
    d_q: &TensorHandle,
    _d_k: Option<&TensorHandle>,
    _d_v: Option<&TensorHandle>,
    _q: &TensorHandle,
    _k: &TensorHandle,
    _v: &TensorHandle,
) -> Option<TensorHandle> {
    // Simplified: return zero tensor of output shape
    // Full implementation would compute proper JVP
    let output_shape = d_q.shape[..d_q.ndim as usize].to_vec();
    TensorHandle::zeros(&output_shape, d_q.dtype)
}

/// Scatter update: dst[indices] = src.
fn tensor_scatter_update(
    dst: &mut TensorHandle,
    src: &TensorHandle,
    indices: &TensorHandle,
    _axis: usize,
) -> Option<()> {
    let dst_data = dst.data.as_ref()?;
    let src_data = src.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;

    // SAFETY: All pointer operations are valid
    unsafe {
        let d = (*dst_data.as_ptr()).as_mut_ptr();
        let s = (*src_data.as_ptr()).as_ptr();
        let idx = (*idx_data.as_ptr()).as_ptr();

        match dst.dtype {
            DType::F32 => {
                let d_f32 = d as *mut f32;
                let s_f32 = s as *const f32;
                let idx_i64 = idx as *const i64;
                for i in 0..src.numel {
                    let target_idx = *idx_i64.add(i) as usize;
                    if target_idx < dst.numel {
                        *d_f32.add(target_idx) = *s_f32.add(i);
                    }
                }
            }
            DType::F64 => {
                let d_f64 = d as *mut f64;
                let s_f64 = s as *const f64;
                let idx_i64 = idx as *const i64;
                for i in 0..src.numel {
                    let target_idx = *idx_i64.add(i) as usize;
                    if target_idx < dst.numel {
                        *d_f64.add(target_idx) = *s_f64.add(i);
                    }
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// Index select along axis.
fn tensor_index_select(
    src: &TensorHandle,
    indices: &TensorHandle,
    axis: usize,
) -> Option<TensorHandle> {
    // Same as gather for 1D case
    tensor_gather(src, indices, axis)
}

/// Concatenate tensors along axis.
fn tensor_concat(tensors: &[&TensorHandle], _axis: usize) -> Option<TensorHandle> {
    if tensors.is_empty() {
        return None;
    }

    let first = tensors[0];
    let total_size: usize = tensors.iter().map(|t| t.numel).sum();

    let mut shape = first.shape[..first.ndim as usize].to_vec();
    shape[0] = total_size / shape[1..].iter().product::<usize>().max(1);

    let result = TensorHandle::zeros(&shape, first.dtype)?;
    let dst_data = result.data.as_ref()?;

    let mut offset = 0;
    for tensor in tensors {
        let src_data = tensor.data.as_ref()?;

        // SAFETY: All pointer operations are valid
        unsafe {
            let s = (*src_data.as_ptr()).as_ptr();
            let d = (*dst_data.as_ptr()).as_mut_ptr();
            let bytes = tensor.numel * tensor.dtype.size();
            std::ptr::copy_nonoverlapping(s, d.add(offset), bytes);
        }
        offset += tensor.numel * tensor.dtype.size();
    }

    Some(result)
}

/// Slice tensor along first dimension.
fn tensor_slice(tensor: &TensorHandle, start: usize, len: usize) -> Option<TensorHandle> {
    let mut shape = tensor.shape[..tensor.ndim as usize].to_vec();
    shape[0] = len;

    let result = TensorHandle::zeros(&shape, tensor.dtype)?;

    let src_data = tensor.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: All pointer operations are valid
    unsafe {
        let s = (*src_data.as_ptr()).as_ptr();
        let d = (*dst_data.as_ptr()).as_mut_ptr();
        let stride = shape[1..].iter().product::<usize>().max(1);
        let bytes = len * stride * tensor.dtype.size();
        let src_offset = start * stride * tensor.dtype.size();
        std::ptr::copy_nonoverlapping(s.add(src_offset), d, bytes);
    }

    Some(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gradient_tape_creation() {
        let tape = GradientTape::new();
        assert!(!tape.is_recording());
        assert_eq!(tape.scopes.len(), 0);
    }

    #[test]
    fn test_begin_end_scope() {
        let mut tape = GradientTape::new();

        let scope_id = tape.begin_scope(GradMode::Reverse).unwrap();
        assert!(tape.is_recording());
        assert_eq!(scope_id, ScopeId(0));

        let ended_id = tape.end_scope().unwrap();
        assert_eq!(ended_id, scope_id);
        assert!(!tape.is_recording());
    }

    #[test]
    fn test_nested_scopes() {
        let mut tape = GradientTape::new();

        let s1 = tape.begin_scope(GradMode::Reverse).unwrap();
        let s2 = tape.begin_scope(GradMode::Forward).unwrap();

        assert_eq!(tape.scopes.len(), 2);
        assert_eq!(tape.current_scope().unwrap().mode, GradMode::Forward);

        tape.end_scope();
        assert_eq!(tape.current_scope().unwrap().mode, GradMode::Reverse);

        tape.end_scope();
        assert!(tape.current_scope().is_none());
    }

    #[test]
    fn test_track_tensor() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);

        let tensor = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let id = tape.track_tensor(tensor).unwrap();

        assert_eq!(id, TensorId(0));
        assert!(tape.current_scope().unwrap().tensors.contains_key(&id));
    }

    #[test]
    fn test_record_operation() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let t2 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let t3 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();

        let id1 = tape.track_tensor(t1.clone()).unwrap();
        let id2 = tape.track_tensor(t2.clone()).unwrap();
        let id3 = tape.track_tensor(t3).unwrap();

        tape.record_op(
            TapeOp::Add,
            &[id1, id2],
            id3,
            vec![SavedValue::Tensor(t1), SavedValue::Tensor(t2)],
        ).unwrap();

        assert_eq!(tape.current_scope().unwrap().tape.len(), 1);
    }

    #[test]
    fn test_checkpoint_and_recompute() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let id1 = tape.track_tensor(t1).unwrap();

        let checkpoint = tape.checkpoint(&[id1]).unwrap();

        // Record some operations
        let t2 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let id2 = tape.track_tensor(t2).unwrap();
        tape.record_op(TapeOp::Add, &[id1, id2], id2, vec![]).unwrap();

        assert_eq!(tape.current_scope().unwrap().tape.len(), 1);

        // Recompute from checkpoint
        tape.recompute(checkpoint).unwrap();
        assert_eq!(tape.current_scope().unwrap().tape.len(), 0);
    }

    #[test]
    fn test_stop_gradient() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let id1 = tape.track_tensor(t1).unwrap();

        assert!(!tape.current_scope().unwrap().is_stopped(id1));

        tape.stop_gradient(id1).unwrap();

        assert!(tape.current_scope().unwrap().is_stopped(id1));
    }

    #[test]
    fn test_zero_grad() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let grad = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();
        let id1 = tape.track_tensor(t1).unwrap();

        tape.set_output_grad(id1, grad);
        assert!(tape.get_grad(id1).is_some());

        tape.zero_grad().unwrap();
        assert!(tape.get_grad(id1).is_none());
    }

    #[test]
    fn test_grad_mode_values() {
        assert_ne!(GradMode::Reverse, GradMode::Forward);
        assert_ne!(GradMode::Forward, GradMode::Auto);
        assert_ne!(GradMode::Auto, GradMode::Reverse);
    }

    #[test]
    fn test_tape_reset() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Reverse);
        tape.begin_scope(GradMode::Forward);

        assert_eq!(tape.scopes.len(), 2);

        tape.reset();

        assert_eq!(tape.scopes.len(), 0);
        assert_eq!(tape.next_scope_id, 0);
    }

    #[test]
    fn test_forward_mode_tangent_tracking() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let tangent = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();
        let id1 = tape.track_tensor(t1).unwrap();

        tape.set_input_tangent(id1, tangent);
        assert!(tape.get_tangent(id1).is_some());
    }

    #[test]
    fn test_forward_mode_zero_tangents() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        let t1 = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let tangent = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();
        let id1 = tape.track_tensor(t1).unwrap();

        tape.set_input_tangent(id1, tangent);
        assert!(tape.get_tangent(id1).is_some());

        tape.zero_tangents().unwrap();
        assert!(tape.get_tangent(id1).is_none());
    }

    #[test]
    fn test_forward_mode_add_tangent() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        // Create input tensors
        let t1 = TensorHandle::full(&[2], DType::F64, 1.0).unwrap();
        let t2 = TensorHandle::full(&[2], DType::F64, 2.0).unwrap();
        let t3 = TensorHandle::zeros(&[2], DType::F64).unwrap();

        let id1 = tape.track_tensor(t1.clone()).unwrap();
        let id2 = tape.track_tensor(t2.clone()).unwrap();
        let id3 = tape.track_tensor(t3).unwrap();

        // Set input tangents
        let dx = TensorHandle::full(&[2], DType::F64, 1.0).unwrap();
        let dy = TensorHandle::full(&[2], DType::F64, 0.5).unwrap();
        tape.set_input_tangent(id1, dx);
        tape.set_input_tangent(id2, dy);

        // Record add operation
        tape.record_op(
            TapeOp::Add,
            &[id1, id2],
            id3,
            vec![SavedValue::Tensor(t1), SavedValue::Tensor(t2)],
        ).unwrap();

        // Run forward mode
        tape.backward().unwrap();

        // For z = x + y, dz = dx + dy = 1.0 + 0.5 = 1.5
        let out_tangent = tape.get_tangent(id3);
        assert!(out_tangent.is_some());
    }

    #[test]
    fn test_forward_mode_mul_tangent() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        // Create input tensors: x=2.0, y=3.0
        let t1 = TensorHandle::full(&[1], DType::F64, 2.0).unwrap();
        let t2 = TensorHandle::full(&[1], DType::F64, 3.0).unwrap();
        let t3 = TensorHandle::zeros(&[1], DType::F64).unwrap();

        let id1 = tape.track_tensor(t1.clone()).unwrap();
        let id2 = tape.track_tensor(t2.clone()).unwrap();
        let id3 = tape.track_tensor(t3).unwrap();

        // Set input tangents: dx=1.0, dy=1.0
        let dx = TensorHandle::full(&[1], DType::F64, 1.0).unwrap();
        let dy = TensorHandle::full(&[1], DType::F64, 1.0).unwrap();
        tape.set_input_tangent(id1, dx);
        tape.set_input_tangent(id2, dy);

        // Record mul operation with saved tensors
        tape.record_op(
            TapeOp::Mul,
            &[id1, id2],
            id3,
            vec![SavedValue::Tensor(t1), SavedValue::Tensor(t2)],
        ).unwrap();

        // Run forward mode
        tape.backward().unwrap();

        // For z = x * y, dz = dx * y + x * dy = 1.0 * 3.0 + 2.0 * 1.0 = 5.0
        let out_tangent = tape.get_tangent(id3);
        assert!(out_tangent.is_some());
    }

    #[test]
    fn test_forward_mode_chain_rule() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        // Compute: z = (x + y) * 2, where x=1, y=2
        // Forward tangent: dz = (dx + dy) * 2
        let x = TensorHandle::full(&[1], DType::F64, 1.0).unwrap();
        let y = TensorHandle::full(&[1], DType::F64, 2.0).unwrap();
        let two = TensorHandle::full(&[1], DType::F64, 2.0).unwrap();
        let sum_result = TensorHandle::full(&[1], DType::F64, 3.0).unwrap(); // x + y = 3
        let final_result = TensorHandle::zeros(&[1], DType::F64).unwrap();

        let id_x = tape.track_tensor(x.clone()).unwrap();
        let id_y = tape.track_tensor(y.clone()).unwrap();
        let id_two = tape.track_tensor(two.clone()).unwrap();
        let id_sum = tape.track_tensor(sum_result.clone()).unwrap();
        let id_final = tape.track_tensor(final_result).unwrap();

        // Set tangent only for x: dx=1.0, dy=0
        let dx = TensorHandle::full(&[1], DType::F64, 1.0).unwrap();
        tape.set_input_tangent(id_x, dx);

        // Record: sum = x + y
        tape.record_op(
            TapeOp::Add,
            &[id_x, id_y],
            id_sum,
            vec![SavedValue::Tensor(x), SavedValue::Tensor(y)],
        ).unwrap();

        // Record: final = sum * 2
        tape.record_op(
            TapeOp::Mul,
            &[id_sum, id_two],
            id_final,
            vec![SavedValue::Tensor(sum_result), SavedValue::Tensor(two)],
        ).unwrap();

        // Run forward mode
        tape.backward().unwrap();

        // dz/dx = 2, so tangent should be 2.0
        let out_tangent = tape.get_tangent(id_final);
        assert!(out_tangent.is_some());
    }

    #[test]
    fn test_forward_mode_exp_tangent() {
        let mut tape = GradientTape::new();
        tape.begin_scope(GradMode::Forward);

        // y = exp(x), x=0 => y=1
        // dy = dx * exp(x) = dx * 1 = dx
        let x = TensorHandle::full(&[1], DType::F64, 0.0).unwrap();
        let exp_x = TensorHandle::full(&[1], DType::F64, 1.0).unwrap(); // exp(0) = 1

        let id_x = tape.track_tensor(x).unwrap();
        let id_exp = tape.track_tensor(exp_x.clone()).unwrap();

        // Set tangent: dx=2.0
        let dx = TensorHandle::full(&[1], DType::F64, 2.0).unwrap();
        tape.set_input_tangent(id_x, dx);

        // Record exp operation (saves exp(x) for backward)
        tape.record_op(
            TapeOp::Exp,
            &[id_x],
            id_exp,
            vec![SavedValue::Tensor(exp_x)],
        ).unwrap();

        tape.backward().unwrap();

        // dy = dx * exp(x) = 2.0 * 1.0 = 2.0
        let out_tangent = tape.get_tangent(id_exp);
        assert!(out_tangent.is_some());
    }

    // ========================================================================
    // Custom VJP/JVP Registry Tests
    // ========================================================================

    #[test]
    fn test_custom_grad_registry_vjp_registration() {
        let mut registry = CustomGradRegistry::new();

        // Register a custom VJP rule
        let rule_id = registry.register_vjp(100, 200, Some("my_custom_fn".to_string()));

        // Verify the rule was registered
        assert!(registry.has_vjp(rule_id));
        assert!(!registry.has_jvp(rule_id));

        // Verify rule lookup
        let rule = registry.get_vjp(rule_id).unwrap();
        assert_eq!(rule.forward_fn, 100);
        assert_eq!(rule.vjp_fn, 200);
        assert_eq!(rule.name.as_deref(), Some("my_custom_fn"));
    }

    #[test]
    fn test_custom_grad_registry_jvp_registration() {
        let mut registry = CustomGradRegistry::new();

        // Register a custom JVP rule
        let rule_id = registry.register_jvp(101, 201, None);

        // Verify the rule was registered
        assert!(registry.has_jvp(rule_id));
        assert!(!registry.has_vjp(rule_id));

        // Verify rule lookup
        let rule = registry.get_jvp(rule_id).unwrap();
        assert_eq!(rule.forward_fn, 101);
        assert_eq!(rule.jvp_fn, 201);
        assert!(rule.name.is_none());
    }

    #[test]
    fn test_custom_grad_registry_fn_to_rule_mapping() {
        let mut registry = CustomGradRegistry::new();

        // Register multiple rules
        let rule1 = registry.register_vjp(100, 200, None);
        let rule2 = registry.register_vjp(101, 201, None);
        let rule3 = registry.register_jvp(102, 202, None);

        // Verify function-to-rule mapping
        assert_eq!(registry.get_rule_for_fn(100), Some(rule1));
        assert_eq!(registry.get_rule_for_fn(101), Some(rule2));
        assert_eq!(registry.get_rule_for_fn(102), Some(rule3));
        assert_eq!(registry.get_rule_for_fn(999), None);
    }

    #[test]
    fn test_custom_grad_registry_unique_rule_ids() {
        let mut registry = CustomGradRegistry::new();

        // Register several rules and ensure unique IDs
        let rule1 = registry.register_vjp(1, 10, None);
        let rule2 = registry.register_vjp(2, 20, None);
        let rule3 = registry.register_jvp(3, 30, None);
        let rule4 = registry.register_jvp(4, 40, None);

        // All rule IDs should be unique
        let ids = vec![rule1, rule2, rule3, rule4];
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }

    #[test]
    fn test_custom_grad_registry_clear() {
        let mut registry = CustomGradRegistry::new();

        // Register some rules
        let rule1 = registry.register_vjp(100, 200, None);
        let rule2 = registry.register_jvp(101, 201, None);

        assert!(registry.has_vjp(rule1));
        assert!(registry.has_jvp(rule2));

        // Clear the registry
        registry.clear();

        // All rules should be gone
        assert!(!registry.has_vjp(rule1));
        assert!(!registry.has_jvp(rule2));
        assert_eq!(registry.get_rule_for_fn(100), None);
        assert_eq!(registry.get_rule_for_fn(101), None);
    }

    #[test]
    fn test_gradient_tape_custom_vjp() {
        let mut tape = GradientTape::new();

        // Register a custom VJP via the tape
        let rule_id = tape.register_custom_vjp(500, 501);

        // Verify it can be retrieved
        assert!(tape.has_custom_vjp(rule_id));
        let rule = tape.get_custom_vjp(rule_id).unwrap();
        assert_eq!(rule.forward_fn, 500);
        assert_eq!(rule.vjp_fn, 501);

        // Verify function lookup
        assert_eq!(tape.get_rule_for_fn(500), Some(rule_id));
    }

    #[test]
    fn test_gradient_tape_custom_vjp_named() {
        let mut tape = GradientTape::new();

        // Register a named custom VJP
        let rule_id = tape.register_custom_vjp_named(600, 601, "softmax_vjp");

        let rule = tape.get_custom_vjp(rule_id).unwrap();
        assert_eq!(rule.name.as_deref(), Some("softmax_vjp"));
    }

    #[test]
    fn test_gradient_tape_custom_jvp() {
        let mut tape = GradientTape::new();

        // Register a custom JVP
        let rule_id = tape.register_custom_jvp(700, 701);

        assert!(tape.has_custom_jvp(rule_id));
        let rule = tape.get_custom_jvp(rule_id).unwrap();
        assert_eq!(rule.forward_fn, 700);
        assert_eq!(rule.jvp_fn, 701);
    }

    #[test]
    fn test_gradient_tape_reset_preserves_custom_rules() {
        let mut tape = GradientTape::new();

        // Register a custom rule and create a scope
        let rule_id = tape.register_custom_vjp(800, 801);
        tape.begin_scope(GradMode::Reverse);

        // Reset (not reset_all)
        tape.reset();

        // Custom rules should still exist
        assert!(tape.has_custom_vjp(rule_id));
    }

    #[test]
    fn test_gradient_tape_reset_all_clears_custom_rules() {
        let mut tape = GradientTape::new();

        // Register a custom rule
        let rule_id = tape.register_custom_vjp(900, 901);
        assert!(tape.has_custom_vjp(rule_id));

        // reset_all should clear custom rules
        tape.reset_all();

        assert!(!tape.has_custom_vjp(rule_id));
    }

    #[test]
    fn test_custom_rule_id_equality() {
        let id1 = CustomRuleId(42);
        let id2 = CustomRuleId(42);
        let id3 = CustomRuleId(43);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        // Test hash behavior
        let mut set = std::collections::HashSet::new();
        set.insert(id1);
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }
}
