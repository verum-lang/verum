//! Autodiff graph metadata for gradient computation.
//!
//! Supports three levels of autodiff:
//! - Level 1: Source transform (compile-time, zero overhead)
//! - Level 2: Checkpointing (hybrid, memory/compute tradeoff)
//! - Level 3: Dynamic tape (runtime, for truly dynamic graphs)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::shape::InstructionId;
use crate::FunctionId;

/// Checkpoint boundary for gradient checkpointing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointBoundary {
    /// Instruction where checkpoint starts.
    pub start: InstructionId,
    /// Instruction where checkpoint ends.
    pub end: InstructionId,
    /// Values to save at checkpoint.
    pub saved_values: Vec<u32>,
    /// Estimated memory savings in bytes.
    pub memory_savings: Option<usize>,
}

impl CheckpointBoundary {
    /// Creates a new checkpoint boundary.
    pub fn new(start: InstructionId, end: InstructionId) -> Self {
        Self {
            start,
            end,
            saved_values: vec![],
            memory_savings: None,
        }
    }

    /// Adds a value to save.
    pub fn save_value(&mut self, value: u32) {
        if !self.saved_values.contains(&value) {
            self.saved_values.push(value);
        }
    }

    /// Sets the estimated memory savings.
    pub fn with_memory_savings(mut self, bytes: usize) -> Self {
        self.memory_savings = Some(bytes);
        self
    }
}

/// Tape structure for dynamic autodiff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TapeStructure {
    /// Operations that need to be recorded on tape.
    pub recorded_ops: Vec<InstructionId>,
    /// Maximum tape length (if bounded).
    pub max_length: Option<usize>,
    /// Whether tape can be rewound.
    pub rewindable: bool,
}

impl TapeStructure {
    /// Creates a new tape structure.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an operation to record.
    pub fn record_op(&mut self, instr: InstructionId) {
        if !self.recorded_ops.contains(&instr) {
            self.recorded_ops.push(instr);
        }
    }

    /// Sets the maximum tape length.
    pub fn with_max_length(mut self, length: usize) -> Self {
        self.max_length = Some(length);
        self
    }

    /// Makes the tape rewindable.
    pub fn rewindable(mut self) -> Self {
        self.rewindable = true;
        self
    }
}

/// Autodiff level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AutodiffLevel {
    /// Source transform: forward and backward generated at compile time.
    /// Zero runtime overhead for gradient structure.
    #[default]
    SourceTransform,
    /// Checkpointing: hybrid mode with memory/compute tradeoff.
    /// User-controlled via @checkpoint annotations.
    Checkpointing,
    /// Dynamic tape: runtime recording for truly dynamic graphs.
    /// Only for RNNs with variable length, etc.
    DynamicTape,
}

/// VJP (Vector-Jacobian Product) rule for a function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VjpRule {
    /// Original (primal) function.
    pub primal: FunctionId,
    /// Generated backward function.
    pub backward: FunctionId,
    /// Values needed from forward pass.
    pub residuals: Vec<u32>,
    /// Whether this is a custom VJP (user-defined).
    pub is_custom: bool,
}

impl VjpRule {
    /// Creates a VJP rule.
    pub fn new(primal: FunctionId, backward: FunctionId) -> Self {
        Self {
            primal,
            backward,
            residuals: vec![],
            is_custom: false,
        }
    }

    /// Adds a residual value needed from forward pass.
    pub fn add_residual(&mut self, value: u32) {
        if !self.residuals.contains(&value) {
            self.residuals.push(value);
        }
    }

    /// Marks this as a custom VJP.
    pub fn custom(mut self) -> Self {
        self.is_custom = true;
        self
    }
}

/// JVP (Jacobian-Vector Product) rule for forward-mode AD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JvpRule {
    /// Original (primal) function.
    pub primal: FunctionId,
    /// Generated tangent function.
    pub tangent: FunctionId,
    /// Whether this is a custom JVP.
    pub is_custom: bool,
}

impl JvpRule {
    /// Creates a JVP rule.
    pub fn new(primal: FunctionId, tangent: FunctionId) -> Self {
        Self {
            primal,
            tangent,
            is_custom: false,
        }
    }

    /// Marks this as a custom JVP.
    pub fn custom(mut self) -> Self {
        self.is_custom = true;
        self
    }
}

/// Autodiff graph for a VBC module.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutodiffGraph {
    /// Forward → backward function mapping (VJP rules).
    pub vjp_rules: HashMap<FunctionId, VjpRule>,
    /// Forward → tangent function mapping (JVP rules).
    pub jvp_rules: HashMap<FunctionId, JvpRule>,
    /// Checkpoint boundaries for gradient checkpointing.
    pub checkpoints: Vec<CheckpointBoundary>,
    /// Tape structure for dynamic autodiff.
    pub tape_structure: Option<TapeStructure>,
    /// Default autodiff level for this module.
    pub default_level: AutodiffLevel,
    /// Functions marked as differentiable.
    pub differentiable_functions: Vec<FunctionId>,
    /// Functions with no gradient (stop_gradient boundary).
    pub stop_gradient_functions: Vec<FunctionId>,
}

impl AutodiffGraph {
    /// Creates an empty autodiff graph.
    pub fn new() -> Self {
        Self {
            default_level: AutodiffLevel::SourceTransform,
            ..Self::default()
        }
    }

    /// Creates an autodiff graph with checkpointing.
    pub fn with_checkpointing() -> Self {
        Self {
            default_level: AutodiffLevel::Checkpointing,
            ..Self::default()
        }
    }

    /// Creates an autodiff graph with dynamic tape.
    pub fn with_dynamic_tape() -> Self {
        Self {
            default_level: AutodiffLevel::DynamicTape,
            tape_structure: Some(TapeStructure::new()),
            ..Self::default()
        }
    }

    /// Adds a VJP rule.
    pub fn add_vjp(&mut self, rule: VjpRule) {
        self.vjp_rules.insert(rule.primal, rule);
    }

    /// Gets the VJP rule for a function.
    pub fn get_vjp(&self, primal: FunctionId) -> Option<&VjpRule> {
        self.vjp_rules.get(&primal)
    }

    /// Adds a JVP rule.
    pub fn add_jvp(&mut self, rule: JvpRule) {
        self.jvp_rules.insert(rule.primal, rule);
    }

    /// Gets the JVP rule for a function.
    pub fn get_jvp(&self, primal: FunctionId) -> Option<&JvpRule> {
        self.jvp_rules.get(&primal)
    }

    /// Adds a checkpoint boundary.
    pub fn add_checkpoint(&mut self, checkpoint: CheckpointBoundary) {
        self.checkpoints.push(checkpoint);
    }

    /// Marks a function as differentiable.
    pub fn mark_differentiable(&mut self, func: FunctionId) {
        if !self.differentiable_functions.contains(&func) {
            self.differentiable_functions.push(func);
        }
    }

    /// Marks a function as stop_gradient.
    pub fn mark_stop_gradient(&mut self, func: FunctionId) {
        if !self.stop_gradient_functions.contains(&func) {
            self.stop_gradient_functions.push(func);
        }
    }

    /// Returns true if a function is differentiable.
    pub fn is_differentiable(&self, func: FunctionId) -> bool {
        self.differentiable_functions.contains(&func) || self.vjp_rules.contains_key(&func)
    }

    /// Returns true if a function blocks gradients.
    pub fn is_stop_gradient(&self, func: FunctionId) -> bool {
        self.stop_gradient_functions.contains(&func)
    }

    /// Gets the backward function for a primal.
    pub fn get_backward(&self, primal: FunctionId) -> Option<FunctionId> {
        self.vjp_rules.get(&primal).map(|r| r.backward)
    }

    /// Returns total estimated memory savings from checkpointing.
    pub fn checkpoint_memory_savings(&self) -> usize {
        self.checkpoints
            .iter()
            .filter_map(|c| c.memory_savings)
            .sum()
    }

    /// Returns true if this autodiff graph is empty (no rules, checkpoints, or functions).
    pub fn is_empty(&self) -> bool {
        self.vjp_rules.is_empty()
            && self.jvp_rules.is_empty()
            && self.checkpoints.is_empty()
            && self.tape_structure.is_none()
            && self.differentiable_functions.is_empty()
            && self.stop_gradient_functions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_boundary() {
        let mut checkpoint = CheckpointBoundary::new(InstructionId(10), InstructionId(50))
            .with_memory_savings(1024 * 1024);

        checkpoint.save_value(0);
        checkpoint.save_value(1);

        assert_eq!(checkpoint.saved_values.len(), 2);
        assert_eq!(checkpoint.memory_savings, Some(1024 * 1024));
    }

    #[test]
    fn test_vjp_rule() {
        let mut rule = VjpRule::new(FunctionId(0), FunctionId(1));
        rule.add_residual(0);
        rule.add_residual(1);

        assert_eq!(rule.residuals, vec![0, 1]);
        assert!(!rule.is_custom);
    }

    #[test]
    fn test_autodiff_graph() {
        let mut graph = AutodiffGraph::new();

        let rule = VjpRule::new(FunctionId(0), FunctionId(1));
        graph.add_vjp(rule);
        graph.mark_differentiable(FunctionId(0));

        assert!(graph.is_differentiable(FunctionId(0)));
        assert_eq!(graph.get_backward(FunctionId(0)), Some(FunctionId(1)));

        graph.add_checkpoint(
            CheckpointBoundary::new(InstructionId(0), InstructionId(10))
                .with_memory_savings(4096),
        );
        assert_eq!(graph.checkpoint_memory_savings(), 4096);
    }

    #[test]
    fn test_tape_structure() {
        let mut tape = TapeStructure::new().with_max_length(1000).rewindable();

        tape.record_op(InstructionId(0));
        tape.record_op(InstructionId(1));

        assert_eq!(tape.recorded_ops.len(), 2);
        assert!(tape.rewindable);
        assert_eq!(tape.max_length, Some(1000));
    }
}
