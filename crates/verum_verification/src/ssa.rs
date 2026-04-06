//! Static Single Assignment (SSA) Construction Module
//!
//! Implements SSA form construction for Verum's verification system.
//! Each variable is assigned exactly once in SSA form, which simplifies
//! dataflow analysis and enables efficient verification.
//!
//! # Algorithm (Cytron et al. 1991)
//!
//! 1. Compute dominance frontiers
//! 2. Insert phi nodes at join points
//! 3. Rename variables to SSA form
//!
//! # Example
//!
//! ```verum
//! // Original code:
//! if condition {
//!     x = 1;
//! } else {
//!     x = 2;
//! }
//! y = x + 3;
//!
//! // SSA form:
//! if condition {
//!     x1 = 1;
//! } else {
//!     x2 = 2;
//! }
//! x3 = phi(x1, x2);
//! y1 = x3 + 3;
//! ```
//!
//! SSA form is crucial for efficient verification: each variable is assigned
//! exactly once, simplifying dataflow analysis and enabling precise weakest
//! precondition computation. The standard Cytron et al. (1991) algorithm is used:
//! (1) compute dominance frontiers, (2) insert phi nodes at join points,
//! (3) rename variables. Complexity: O(n^2 + n*m) where n = blocks, m = variables,
//! which is linear in practice for structured programs.

use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::{List, Map, Set, Text};

// ============================================================================
// Core Type Definitions
// ============================================================================

/// Unique identifier for a basic block in the CFG.
///
/// Basic blocks are the nodes in the control flow graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub u32);

impl BlockId {
    /// Create a new block ID.
    #[inline]
    pub fn new(id: u32) -> Self {
        BlockId(id)
    }

    /// Get the underlying ID value.
    #[inline]
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// The entry block ID (always 0).
    pub const ENTRY: BlockId = BlockId(0);
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// A variable in the SSA representation.
///
/// In SSA form, each assignment creates a new version (e.g., x_0, x_1, x_2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Variable {
    /// The original variable name.
    pub name: Text,
    /// The SSA version number (0 for original, 1+ for SSA versions).
    pub version: Version,
}

impl Variable {
    /// Create a new variable with the given name and version.
    pub fn new(name: Text, version: Version) -> Self {
        Variable { name, version }
    }

    /// Create an original (non-SSA) variable.
    pub fn original(name: Text) -> Self {
        Variable {
            name,
            version: Version::Original,
        }
    }

    /// Create an SSA-versioned variable.
    pub fn versioned(name: Text, version: u32) -> Self {
        Variable {
            name,
            version: Version::Ssa(version),
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.version {
            Version::Original => write!(f, "{}", self.name),
            Version::Ssa(v) => write!(f, "{}.{}", self.name, v),
        }
    }
}

/// SSA version number for a variable.
///
/// Tracks the assignment history of a variable through the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Version {
    /// Original (non-SSA) variable.
    Original,
    /// SSA version number (1-indexed).
    Ssa(u32),
}

impl Version {
    /// Check if this is the original (non-SSA) version.
    pub fn is_original(self) -> bool {
        matches!(self, Version::Original)
    }

    /// Get the SSA version number, or 0 for Original.
    pub fn as_u32(self) -> u32 {
        match self {
            Version::Original => 0,
            Version::Ssa(v) => v,
        }
    }
}

/// A value in the SSA representation.
///
/// Represents operands in SSA instructions: variable references, constants, or expressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// A variable reference.
    Variable(Variable),
    /// An integer constant.
    IntConst(i64),
    /// A boolean constant.
    BoolConst(bool),
    /// A floating-point constant.
    FloatConst(f64),
    /// A string constant.
    StringConst(Text),
    /// An undefined value (for uninitialized variables).
    Undefined,
    /// Result of a phi node.
    Phi(BlockId, u32),
}

impl Value {
    /// Create a variable value.
    pub fn variable(var: Variable) -> Self {
        Value::Variable(var)
    }

    /// Create an integer constant value.
    pub fn int(value: i64) -> Self {
        Value::IntConst(value)
    }

    /// Create a boolean constant value.
    pub fn bool(value: bool) -> Self {
        Value::BoolConst(value)
    }

    /// Get the variable if this is a variable value.
    pub fn as_variable(&self) -> Option<&Variable> {
        match self {
            Value::Variable(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Variable(v) => write!(f, "{}", v),
            Value::IntConst(i) => write!(f, "{}", i),
            Value::BoolConst(b) => write!(f, "{}", b),
            Value::FloatConst(fl) => write!(f, "{}", fl),
            Value::StringConst(s) => write!(f, "\"{}\"", s),
            Value::Undefined => write!(f, "undef"),
            Value::Phi(block, idx) => write!(f, "phi({}, {})", block, idx),
        }
    }
}

// ============================================================================
// Statement and Terminator Types
// ============================================================================

/// A statement in a basic block.
///
/// A straight-line instruction within a basic block (no branches).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Statement {
    /// Assignment: `target = value`
    Assign {
        /// The variable being assigned to.
        target: Variable,
        /// The value being assigned.
        value: Value,
    },
    /// Binary operation: `target = left op right`
    BinaryOp {
        /// The variable receiving the result.
        target: Variable,
        /// The operation.
        op: BinaryOp,
        /// Left operand.
        left: Value,
        /// Right operand.
        right: Value,
    },
    /// Unary operation: `target = op operand`
    UnaryOp {
        /// The variable receiving the result.
        target: Variable,
        /// The operation.
        op: UnaryOp,
        /// The operand.
        operand: Value,
    },
    /// Function call: `target = func(args)`
    Call {
        /// The variable receiving the result (None for void functions).
        target: Option<Variable>,
        /// The function being called.
        func: Text,
        /// The arguments.
        args: List<Value>,
    },
    /// Assertion: `assert condition`
    Assert {
        /// The condition being asserted.
        condition: Value,
        /// Optional message.
        message: Option<Text>,
    },
    /// Assume: `assume condition`
    Assume {
        /// The condition being assumed.
        condition: Value,
    },
}

impl Statement {
    /// Get the target variable of this statement, if any.
    pub fn target(&self) -> Option<&Variable> {
        match self {
            Statement::Assign { target, .. } => Some(target),
            Statement::BinaryOp { target, .. } => Some(target),
            Statement::UnaryOp { target, .. } => Some(target),
            Statement::Call { target, .. } => target.as_ref(),
            Statement::Assert { .. } => None,
            Statement::Assume { .. } => None,
        }
    }

    /// Get all variables used (read) by this statement.
    pub fn uses(&self) -> List<&Variable> {
        let mut uses = List::new();
        match self {
            Statement::Assign { value, .. } => {
                if let Some(v) = value.as_variable() {
                    uses.push(v);
                }
            }
            Statement::BinaryOp { left, right, .. } => {
                if let Some(v) = left.as_variable() {
                    uses.push(v);
                }
                if let Some(v) = right.as_variable() {
                    uses.push(v);
                }
            }
            Statement::UnaryOp { operand, .. } => {
                if let Some(v) = operand.as_variable() {
                    uses.push(v);
                }
            }
            Statement::Call { args, .. } => {
                for arg in args {
                    if let Some(v) = arg.as_variable() {
                        uses.push(v);
                    }
                }
            }
            Statement::Assert { condition, .. } => {
                if let Some(v) = condition.as_variable() {
                    uses.push(v);
                }
            }
            Statement::Assume { condition } => {
                if let Some(v) = condition.as_variable() {
                    uses.push(v);
                }
            }
        }
        uses
    }
}

/// Binary operations supported in SSA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    /// Addition
    Add,
    /// Subtraction
    Sub,
    /// Multiplication
    Mul,
    /// Division
    Div,
    /// Modulo
    Mod,
    /// Logical AND
    And,
    /// Logical OR
    Or,
    /// Equality comparison
    Eq,
    /// Inequality comparison
    Ne,
    /// Less than
    Lt,
    /// Less than or equal
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal
    Ge,
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
        };
        write!(f, "{}", s)
    }
}

/// Unary operations supported in SSA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    /// Logical NOT
    Not,
    /// Arithmetic negation
    Neg,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UnaryOp::Not => "!",
            UnaryOp::Neg => "-",
        };
        write!(f, "{}", s)
    }
}

/// Block terminator - how control leaves a basic block.
///
/// How control leaves a basic block: goto, conditional branch, return, or switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Terminator {
    /// Unconditional jump to another block.
    Goto(BlockId),
    /// Conditional branch.
    Branch {
        /// The condition value.
        condition: Value,
        /// Block to jump to if condition is true.
        true_block: BlockId,
        /// Block to jump to if condition is false.
        false_block: BlockId,
    },
    /// Return from function.
    Return(Option<Value>),
    /// Unreachable code (after panic, infinite loop, etc.).
    Unreachable,
}

impl Terminator {
    /// Get all successor block IDs.
    pub fn successors(&self) -> List<BlockId> {
        let mut succs = List::new();
        match self {
            Terminator::Goto(block) => succs.push(*block),
            Terminator::Branch {
                true_block,
                false_block,
                ..
            } => {
                succs.push(*true_block);
                succs.push(*false_block);
            }
            Terminator::Return(_) => {}
            Terminator::Unreachable => {}
        }
        succs
    }
}

// ============================================================================
// Basic Block and CFG
// ============================================================================

/// A basic block in the Control Flow Graph (CFG).
///
/// A basic block is a straight-line sequence of statements with:
/// - One entry point (no jumps into the middle)
/// - One exit point (the terminator)
///
/// Straight-line sequence with one entry point (no jumps in) and one exit (terminator).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    /// Unique identifier for this block.
    pub id: BlockId,
    /// Phi nodes at the start of this block.
    pub phi_nodes: List<PhiNode>,
    /// Statements in this block (executed in order).
    pub statements: List<Statement>,
    /// How control leaves this block.
    pub terminator: Terminator,
    /// Predecessor blocks (blocks that jump to this one).
    pub predecessors: List<BlockId>,
    /// Successor blocks (blocks this one can jump to).
    pub successors: List<BlockId>,
}

impl BasicBlock {
    /// Create a new basic block with the given ID and terminator.
    pub fn new(id: BlockId, terminator: Terminator) -> Self {
        let successors = terminator.successors();
        BasicBlock {
            id,
            phi_nodes: List::new(),
            statements: List::new(),
            terminator,
            predecessors: List::new(),
            successors,
        }
    }

    /// Create an empty basic block with an unreachable terminator.
    pub fn empty(id: BlockId) -> Self {
        BasicBlock {
            id,
            phi_nodes: List::new(),
            statements: List::new(),
            terminator: Terminator::Unreachable,
            predecessors: List::new(),
            successors: List::new(),
        }
    }

    /// Add a statement to this block.
    pub fn add_statement(&mut self, stmt: Statement) {
        self.statements.push(stmt);
    }

    /// Set the terminator and update successors.
    pub fn set_terminator(&mut self, terminator: Terminator) {
        self.successors = terminator.successors();
        self.terminator = terminator;
    }

    /// Add a phi node to this block.
    pub fn add_phi(&mut self, phi: PhiNode) {
        self.phi_nodes.push(phi);
    }

    /// Get all variables defined in this block.
    pub fn definitions(&self) -> List<&Variable> {
        let mut defs = List::new();
        for phi in &self.phi_nodes {
            defs.push(&phi.result);
        }
        for stmt in &self.statements {
            if let Some(target) = stmt.target() {
                defs.push(target);
            }
        }
        defs
    }

    /// Check if this block is empty (no statements or phi nodes).
    pub fn is_empty(&self) -> bool {
        self.phi_nodes.is_empty() && self.statements.is_empty()
    }
}

/// Phi node for merging values at join points.
///
/// A phi node selects a value based on which predecessor block control
/// came from. This is essential for SSA form at points where control
/// flow merges.
///
/// Selects a value based on which predecessor block control came from.
/// Example: `x3 = phi(x1, x2)` where x1 from true branch, x2 from false branch.
/// Phi nodes are inserted at dominance frontiers where variable definitions merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhiNode {
    /// The variable being defined by this phi node.
    pub result: Variable,
    /// Operands: (predecessor block, value from that block).
    pub operands: List<(BlockId, Value)>,
}

impl PhiNode {
    /// Create a new phi node for the given variable.
    pub fn new(result: Variable) -> Self {
        PhiNode {
            result,
            operands: List::new(),
        }
    }

    /// Add an operand to this phi node.
    pub fn add_operand(&mut self, block: BlockId, value: Value) {
        self.operands.push((block, value));
    }

    /// Get the value from a specific predecessor block.
    pub fn value_from(&self, block: BlockId) -> Option<&Value> {
        self.operands
            .iter()
            .find(|(b, _)| *b == block)
            .map(|(_, v)| v)
    }
}

impl fmt::Display for PhiNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = phi(", self.result)?;
        for (i, (block, value)) in self.operands.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "[{}: {}]", block, value)?;
        }
        write!(f, ")")
    }
}

/// Control Flow Graph (CFG) representation.
///
/// The CFG is a directed graph where:
/// - Nodes are basic blocks
/// - Edges represent possible control flow between blocks
///
/// Directed graph where nodes are basic blocks and edges represent possible control flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlFlowGraph {
    /// The entry block ID.
    pub entry: BlockId,
    /// The exit block ID (if any).
    pub exit: Option<BlockId>,
    /// All basic blocks in the CFG.
    pub blocks: Map<u32, BasicBlock>,
    /// Counter for generating unique block IDs.
    next_block_id: u32,
}

impl ControlFlowGraph {
    /// Create a new empty CFG.
    pub fn new() -> Self {
        let mut cfg = ControlFlowGraph {
            entry: BlockId::ENTRY,
            exit: None,
            blocks: Map::new(),
            next_block_id: 0,
        };
        // Create the entry block
        cfg.create_block();
        cfg
    }

    /// Create a new basic block and return its ID.
    pub fn create_block(&mut self) -> BlockId {
        let id = BlockId::new(self.next_block_id);
        self.next_block_id += 1;
        self.blocks.insert(id.0, BasicBlock::empty(id));
        id
    }

    /// Get a reference to a basic block.
    pub fn get_block(&self, id: BlockId) -> Option<&BasicBlock> {
        self.blocks.get(&id.0)
    }

    /// Get a mutable reference to a basic block.
    pub fn get_block_mut(&mut self, id: BlockId) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(&id.0)
    }

    /// Get all block IDs in the CFG.
    pub fn block_ids(&self) -> List<BlockId> {
        self.blocks.keys().map(|&id| BlockId::new(id)).collect()
    }

    /// Get the number of blocks in the CFG.
    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Update predecessor/successor relationships.
    ///
    /// Call this after modifying block terminators to ensure
    /// predecessor lists are up to date.
    pub fn compute_predecessors(&mut self) {
        // Clear all predecessor lists
        for block in self.blocks.values_mut() {
            block.predecessors.clear();
        }

        // Collect edges first to avoid borrowing issues
        let mut edges: List<(BlockId, BlockId)> = List::new();
        for block in self.blocks.values() {
            let block_id = block.id;
            for &succ_id in &block.successors {
                edges.push((block_id, succ_id));
            }
        }

        // Add predecessor for each edge
        for (pred_id, succ_id) in edges {
            if let Some(succ) = self.blocks.get_mut(&succ_id.0) {
                succ.predecessors.push(pred_id);
            }
        }
    }

    /// Get all variables defined anywhere in the CFG.
    pub fn all_variables(&self) -> Set<Text> {
        let mut vars = Set::new();
        for block in self.blocks.values() {
            for phi in &block.phi_nodes {
                vars.insert(phi.result.name.clone());
            }
            for stmt in &block.statements {
                if let Some(target) = stmt.target() {
                    vars.insert(target.name.clone());
                }
            }
        }
        vars
    }

    /// Get the blocks where a variable is defined (assigned).
    pub fn definition_blocks(&self, var_name: &Text) -> Set<BlockId> {
        let mut blocks = Set::new();
        for block in self.blocks.values() {
            for stmt in &block.statements {
                if let Some(target) = stmt.target()
                    && &target.name == var_name
                {
                    blocks.insert(block.id);
                    break;
                }
            }
        }
        blocks
    }
}

impl Default for ControlFlowGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// SSA Form Representation
// ============================================================================

/// Static Single Assignment (SSA) form representation.
///
/// In SSA form:
/// - Each variable is assigned exactly once
/// - Phi nodes merge values at control flow join points
/// - Variable versions track the assignment history
///
/// Each variable is assigned exactly once; phi nodes merge values at join points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SSAForm {
    /// The underlying control flow graph.
    pub cfg: ControlFlowGraph,
    /// Mapping from original variable names to their definitions.
    /// Maps variable name -> List of (block, version) pairs.
    pub definitions: Map<Text, List<(BlockId, Version)>>,
    /// Counter for generating unique variable versions.
    version_counters: Map<Text, u32>,
}

impl SSAForm {
    /// Create a new SSA form from a CFG.
    fn new(cfg: ControlFlowGraph) -> Self {
        SSAForm {
            cfg,
            definitions: Map::new(),
            version_counters: Map::new(),
        }
    }

    /// Get the current version number for a variable.
    pub fn current_version(&self, var_name: &Text) -> u32 {
        self.version_counters.get(var_name).copied().unwrap_or(0)
    }

    /// Allocate a new version number for a variable.
    fn next_version(&mut self, var_name: &Text) -> u32 {
        let counter = self.version_counters.entry(var_name.clone()).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Record a definition of a variable.
    fn record_definition(&mut self, var_name: Text, block: BlockId, version: Version) {
        self.definitions
            .entry(var_name)
            .or_insert_with(List::new)
            .push((block, version));
    }

    /// Get all definitions of a variable.
    pub fn get_definitions(&self, var_name: &Text) -> Option<&List<(BlockId, Version)>> {
        self.definitions.get(var_name)
    }

    /// Check if the SSA form is valid (each variable assigned exactly once).
    pub fn is_valid(&self) -> bool {
        // Collect all definitions
        let mut all_defs: Set<(Text, u32)> = Set::new();

        for block in self.cfg.blocks.values() {
            // Check phi nodes
            for phi in &block.phi_nodes {
                let key = (phi.result.name.clone(), phi.result.version.as_u32());
                if all_defs.contains(&key) {
                    return false; // Duplicate definition
                }
                all_defs.insert(key);
            }

            // Check statements
            for stmt in &block.statements {
                if let Some(target) = stmt.target() {
                    let key = (target.name.clone(), target.version.as_u32());
                    if all_defs.contains(&key) {
                        return false; // Duplicate definition
                    }
                    all_defs.insert(key);
                }
            }
        }

        true
    }
}

// ============================================================================
// Dominance Computation
// ============================================================================

/// Compute immediate dominators for all blocks in the CFG.
///
/// Block A dominates block B if every path from the entry to B must go through A.
/// The immediate dominator of B is the closest dominator of B (other than B itself).
///
/// Uses the Cooper-Harvey-Kennedy algorithm (simple iterative approach).
///
/// Complexity: O(n^2) where n = number of basic blocks
///
/// Cooper-Harvey-Kennedy iterative algorithm. Complexity: O(n^2) where n = number
/// of basic blocks. Computes the immediate dominator for each block.
pub fn compute_dominators(cfg: &ControlFlowGraph) -> Map<BlockId, BlockId> {
    let mut dominators: Map<BlockId, BlockId> = Map::new();
    let block_ids = cfg.block_ids();

    if block_ids.is_empty() {
        return dominators;
    }

    // Entry block's immediate dominator is itself
    dominators.insert(cfg.entry, cfg.entry);

    // Initialize all other blocks to have no dominator yet
    // (We'll use a special "undefined" state by not having an entry)

    // Iteratively compute dominators until fixed point
    let mut changed = true;
    while changed {
        changed = false;

        for &block_id in &block_ids {
            if block_id == cfg.entry {
                continue;
            }

            if let Some(block) = cfg.get_block(block_id) {
                if block.predecessors.is_empty() {
                    continue;
                }

                // Find the new immediate dominator
                // It's the intersection of all predecessors' dominators
                let mut new_idom: Option<BlockId> = None;

                for &pred_id in &block.predecessors {
                    if dominators.contains_key(&pred_id) {
                        new_idom = match new_idom {
                            None => Some(pred_id),
                            Some(current) => Some(intersect_dominators(
                                current,
                                pred_id,
                                &dominators,
                                &block_ids,
                            )),
                        };
                    }
                }

                if let Some(idom) = new_idom {
                    let old_idom = dominators.get(&block_id);
                    if old_idom != Some(&idom) {
                        dominators.insert(block_id, idom);
                        changed = true;
                    }
                }
            }
        }
    }

    dominators
}

/// Find the common dominator of two blocks (intersection in dominator tree).
fn intersect_dominators(
    mut b1: BlockId,
    mut b2: BlockId,
    dominators: &Map<BlockId, BlockId>,
    block_ids: &List<BlockId>,
) -> BlockId {
    // Use block ID ordering as a proxy for depth
    // (This is a simplification; full algorithm uses explicit depths)
    let get_order = |id: BlockId| -> usize { block_ids.iter().position(|&x| x == id).unwrap_or(0) };

    // Maximum iterations to prevent infinite loops
    let max_iterations = block_ids.len() * 2;
    let mut iterations = 0;

    while b1 != b2 && iterations < max_iterations {
        iterations += 1;

        let order_b1 = get_order(b1);
        let order_b2 = get_order(b2);

        if order_b1 > order_b2 {
            // Move b1 up the dominator tree
            match dominators.get(&b1) {
                Some(&idom) if idom != b1 => b1 = idom,
                _ => break, // Reached root or self-loop
            }
        } else if order_b2 > order_b1 {
            // Move b2 up the dominator tree
            match dominators.get(&b2) {
                Some(&idom) if idom != b2 => b2 = idom,
                _ => break, // Reached root or self-loop
            }
        } else {
            // Same order but different blocks - move both up
            match dominators.get(&b1) {
                Some(&idom) if idom != b1 => b1 = idom,
                _ => break,
            }
        }
    }

    // Return the common dominator (or entry if we couldn't find one)
    if b1 == b2 { b1 } else { BlockId::ENTRY }
}

/// Compute dominance frontiers for all blocks in the CFG.
///
/// The dominance frontier of block B is the set of blocks where B's
/// dominance ends - that is, blocks that B dominates a predecessor of
/// but does not strictly dominate.
///
/// Complexity: O(n^2) where n = number of basic blocks
///
/// Block Y is in the dominance frontier of block X if X does not strictly dominate Y
/// but X dominates some predecessor of Y. This identifies where phi nodes are needed.
/// Complexity: O(n^2) where n = number of basic blocks.
pub fn compute_dominance_frontiers(
    cfg: &ControlFlowGraph,
    dominators: &Map<BlockId, BlockId>,
) -> Map<BlockId, Set<BlockId>> {
    let mut frontiers: Map<BlockId, Set<BlockId>> = Map::new();

    // Initialize empty frontiers for all blocks
    for &block_id in &cfg.block_ids() {
        frontiers.insert(block_id, Set::new());
    }

    // For each block B with multiple predecessors (join point)
    for block in cfg.blocks.values() {
        let b = block.id;
        if block.predecessors.len() >= 2 {
            // Get idom(B) - the block we should stop at (but not include)
            let idom_b = dominators.get(&b).copied();

            // For each predecessor P of B
            for &p in &block.predecessors {
                // Walk up dominator tree from P until we reach idom(B)
                let mut runner = p;

                // Use a visited set to detect cycles and prevent infinite loops
                let mut visited = Set::new();

                // Continue until runner equals idom(B)
                // This ensures we process runner but not idom(B) itself
                while Some(runner) != idom_b {
                    // Detect cycles (should never happen with valid dominators)
                    if !visited.insert(runner) {
                        break;
                    }

                    // Add B to runner's dominance frontier
                    frontiers.entry(runner).or_insert_with(Set::new).insert(b);

                    // Move up the dominator tree
                    match dominators.get(&runner) {
                        Some(&next) if next != runner => {
                            runner = next;
                        }
                        Some(&next) if next == runner => {
                            // Self-loop means we've reached the root (entry block)
                            break;
                        }
                        None => {
                            // No dominator entry - shouldn't happen but break to be safe
                            break;
                        }
                        _ => {
                            // Unreachable but safe
                            break;
                        }
                    }
                }
            }
        }
    }

    frontiers
}

// ============================================================================
// Phi Node Insertion
// ============================================================================

/// Insert phi nodes at dominance frontiers.
///
/// For each variable v and each block B in v's dominance frontier,
/// insert a phi node for v at B.
///
/// Complexity: O(n * m) where n = blocks, m = variables
///
/// For each variable v and each block B in v's dominance frontier, insert
/// `v_new = phi(v_1, v_2, ..., v_n)` where v_i are versions reaching B from each
/// predecessor. Uses iterative worklist algorithm. Complexity: O(n * m).
pub fn insert_phi_nodes(
    cfg: &mut ControlFlowGraph,
    dominance_frontiers: &Map<BlockId, Set<BlockId>>,
) {
    let variables = cfg.all_variables();

    for var_name in variables {
        // Get blocks where this variable is defined
        let mut worklist: List<BlockId> = cfg.definition_blocks(&var_name).into_iter().collect();
        let mut phi_inserted: Set<BlockId> = Set::new();
        let mut processed: Set<BlockId> = Set::new();

        while let Some(def_block) = worklist.pop() {
            if processed.contains(&def_block) {
                continue;
            }
            processed.insert(def_block);

            // For each block in the dominance frontier of def_block
            if let Some(frontier) = dominance_frontiers.get(&def_block) {
                for &frontier_block in frontier.iter() {
                    if !phi_inserted.contains(&frontier_block) {
                        // Insert phi node
                        let phi = PhiNode::new(Variable::original(var_name.clone()));
                        if let Some(block) = cfg.get_block_mut(frontier_block) {
                            block.add_phi(phi);
                        }
                        phi_inserted.insert(frontier_block);
                        worklist.push(frontier_block);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Variable Renaming
// ============================================================================

/// Rename variables to SSA form.
///
/// This converts the CFG to true SSA form by:
/// 1. Giving each definition a unique version number
/// 2. Updating uses to reference the correct version
/// 3. Filling in phi node operands
///
/// Complexity: O(n * m) where n = blocks, m = variables
///
/// Walk the dominator tree, assigning fresh SSA versions to each definition,
/// updating uses to the correct version, and filling in phi node operands.
/// Complexity: O(n * m) where n = blocks, m = variables.
pub fn rename_variables(cfg: &mut ControlFlowGraph) -> SSAForm {
    let mut ssa = SSAForm::new(cfg.clone());

    // Stack of current version for each variable
    let mut var_stacks: Map<Text, List<u32>> = Map::new();

    // Initialize stacks with version 0 for all variables
    for var_name in ssa.cfg.all_variables() {
        var_stacks.insert(var_name, List::from(vec![0]));
    }

    // Get dominance information for traversal order
    let dominators = compute_dominators(&ssa.cfg);

    // Build dominator tree (children of each node)
    let mut dom_children: Map<BlockId, List<BlockId>> = Map::new();
    for &block_id in &ssa.cfg.block_ids() {
        dom_children.insert(block_id, List::new());
    }
    for (&block_id, &idom) in &dominators {
        if block_id != idom
            && let Some(children) = dom_children.get_mut(&idom)
        {
            children.push(block_id);
        }
    }

    // Rename starting from entry block using DFS
    rename_block(ssa.cfg.entry, &mut ssa, &mut var_stacks, &dom_children);

    ssa
}

/// Recursively rename variables in a block and its dominated children.
fn rename_block(
    block_id: BlockId,
    ssa: &mut SSAForm,
    var_stacks: &mut Map<Text, List<u32>>,
    dom_children: &Map<BlockId, List<BlockId>>,
) {
    // Track how many versions we push for each variable (to pop later)
    let mut push_counts: Map<Text, u32> = Map::new();

    // Phase 1: Collect information about phi nodes and statements
    // We need to do this to avoid borrowing issues
    let (phi_var_names, stmt_info): (List<Text>, List<Option<Text>>) = {
        if let Some(block) = ssa.cfg.blocks.get(&block_id.0) {
            let phi_names: List<Text> = block
                .phi_nodes
                .iter()
                .map(|phi| phi.result.name.clone())
                .collect();
            let stmt_targets: List<Option<Text>> = block
                .statements
                .iter()
                .map(|stmt| stmt.target().map(|t| t.name.clone()))
                .collect();
            (phi_names, stmt_targets)
        } else {
            (List::new(), List::new())
        }
    };

    // Phase 2: Allocate versions for phi nodes
    let phi_versions: List<u32> = phi_var_names
        .iter()
        .map(|var_name| {
            let new_version = ssa.next_version(var_name);
            // Push new version onto stack
            if let Some(stack) = var_stacks.get_mut(var_name) {
                stack.push(new_version);
            }
            *push_counts.entry(var_name.clone()).or_insert(0) += 1;
            ssa.record_definition(var_name.clone(), block_id, Version::Ssa(new_version));
            new_version
        })
        .collect();

    // Phase 3: Allocate versions for statements
    let stmt_versions: List<Option<u32>> = stmt_info
        .iter()
        .map(|maybe_name| {
            maybe_name.as_ref().map(|var_name| {
                let new_version = ssa.next_version(var_name);
                // Push new version onto stack
                if let Some(stack) = var_stacks.get_mut(var_name) {
                    stack.push(new_version);
                }
                *push_counts.entry(var_name.clone()).or_insert(0) += 1;
                ssa.record_definition(var_name.clone(), block_id, Version::Ssa(new_version));
                new_version
            })
        })
        .collect();

    // Phase 4: Apply renaming to the block
    if let Some(block) = ssa.cfg.blocks.get_mut(&block_id.0) {
        // Rename phi node results
        for (i, new_version) in phi_versions.iter().enumerate() {
            if i < block.phi_nodes.len() {
                let var_name = block.phi_nodes[i].result.name.clone();
                block.phi_nodes[i].result = Variable::versioned(var_name, *new_version);
            }
        }

        // Rename statement uses and definitions
        for (i, maybe_version) in stmt_versions.iter().enumerate() {
            if i < block.statements.len() {
                // Rename uses first
                rename_statement_uses(&mut block.statements[i], var_stacks);

                // Then rename definition
                if let Some(version) = maybe_version {
                    rename_statement_target(&mut block.statements[i], *version);
                }
            }
        }
    }

    // Phase 5: Fill in phi node operands in successors
    let successors = ssa
        .cfg
        .get_block(block_id)
        .map(|b| b.successors.clone())
        .unwrap_or_default();

    for succ_id in successors {
        // Collect phi operand info
        let phi_operands: List<(Text, Value)> = {
            if let Some(succ_block) = ssa.cfg.blocks.get(&succ_id.0) {
                succ_block
                    .phi_nodes
                    .iter()
                    .map(|phi| {
                        let var_name = &phi.result.name;
                        let version = var_stacks
                            .get(var_name)
                            .and_then(|stack| stack.last().copied())
                            .unwrap_or(0);

                        let value = if version == 0 {
                            Value::Undefined
                        } else {
                            Value::Variable(Variable::versioned(var_name.clone(), version))
                        };
                        (var_name.clone(), value)
                    })
                    .collect()
            } else {
                List::new()
            }
        };

        // Apply the operands
        if let Some(succ_block) = ssa.cfg.blocks.get_mut(&succ_id.0) {
            for (i, (_, value)) in phi_operands.iter().enumerate() {
                if i < succ_block.phi_nodes.len() {
                    succ_block.phi_nodes[i].add_operand(block_id, value.clone());
                }
            }
        }
    }

    // Phase 6: Recursively process dominated children
    let children = dom_children
        .get(&block_id)
        .cloned()
        .unwrap_or_else(List::new);
    for child_id in children {
        rename_block(child_id, ssa, var_stacks, dom_children);
    }

    // Phase 7: Pop versions we pushed
    for (var_name, count) in push_counts {
        if let Some(stack) = var_stacks.get_mut(&var_name) {
            for _ in 0..count {
                stack.pop();
            }
        }
    }
}

/// Rename uses in a statement to their current SSA versions.
fn rename_statement_uses(stmt: &mut Statement, var_stacks: &Map<Text, List<u32>>) {
    match stmt {
        Statement::Assign { value, .. } => {
            rename_value(value, var_stacks);
        }
        Statement::BinaryOp { left, right, .. } => {
            rename_value(left, var_stacks);
            rename_value(right, var_stacks);
        }
        Statement::UnaryOp { operand, .. } => {
            rename_value(operand, var_stacks);
        }
        Statement::Call { args, .. } => {
            for arg in args {
                rename_value(arg, var_stacks);
            }
        }
        Statement::Assert { condition, .. } => {
            rename_value(condition, var_stacks);
        }
        Statement::Assume { condition } => {
            rename_value(condition, var_stacks);
        }
    }
}

/// Rename a value to its current SSA version.
fn rename_value(value: &mut Value, var_stacks: &Map<Text, List<u32>>) {
    if let Value::Variable(var) = value {
        let version = var_stacks
            .get(&var.name)
            .and_then(|stack| stack.last().copied())
            .unwrap_or(0);

        if version == 0 {
            *value = Value::Undefined;
        } else {
            var.version = Version::Ssa(version);
        }
    }
}

/// Update the target of a statement to a new version.
fn rename_statement_target(stmt: &mut Statement, version: u32) {
    match stmt {
        Statement::Assign { target, .. } => {
            target.version = Version::Ssa(version);
        }
        Statement::BinaryOp { target, .. } => {
            target.version = Version::Ssa(version);
        }
        Statement::UnaryOp { target, .. } => {
            target.version = Version::Ssa(version);
        }
        Statement::Call { target, .. } => {
            if let Some(t) = target {
                t.version = Version::Ssa(version);
            }
        }
        _ => {}
    }
}

// ============================================================================
// Main Entry Point
// ============================================================================

/// Convert a Control Flow Graph to SSA form.
///
/// This is the main entry point for SSA construction. It performs:
/// 1. Dominance computation
/// 2. Phi node insertion
/// 3. Variable renaming
///
/// # Arguments
///
/// * `cfg` - The control flow graph to convert
///
/// # Returns
///
/// The CFG in SSA form with phi nodes and renamed variables.
///
/// # Complexity
///
/// O(n^2 + n*m) where n = number of basic blocks, m = number of variables.
/// This is linear in practice for structured programs.
///
/// # Example
///
/// ```rust
/// use verum_verification::ssa::{ControlFlowGraph, to_ssa};
///
/// let cfg = ControlFlowGraph::new();
/// let ssa = to_ssa(cfg);
/// assert!(ssa.is_valid());
/// ```
///
/// Complete SSA construction pipeline: compute predecessors, compute dominators,
/// compute dominance frontiers, insert phi nodes, rename variables. Returns
/// the CFG in SSA form with all variables assigned exactly once.
pub fn to_ssa(mut cfg: ControlFlowGraph) -> SSAForm {
    // 1. Ensure predecessors are computed
    cfg.compute_predecessors();

    // 2. Compute dominance information
    let dominators = compute_dominators(&cfg);
    let dominance_frontiers = compute_dominance_frontiers(&cfg, &dominators);

    // 3. Insert phi nodes
    insert_phi_nodes(&mut cfg, &dominance_frontiers);

    // 4. Rename variables
    rename_variables(&mut cfg)
}

// ============================================================================
// Builder API for Creating CFGs
// ============================================================================

/// Builder for constructing Control Flow Graphs programmatically.
///
/// This provides a convenient API for creating CFGs for testing
/// and from the Verum AST.
#[derive(Debug)]
pub struct CFGBuilder {
    cfg: ControlFlowGraph,
    current_block: BlockId,
}

impl CFGBuilder {
    /// Create a new CFG builder.
    pub fn new() -> Self {
        CFGBuilder {
            cfg: ControlFlowGraph::new(),
            current_block: BlockId::ENTRY,
        }
    }

    /// Get the current block ID.
    pub fn current_block(&self) -> BlockId {
        self.current_block
    }

    /// Create a new block and return its ID.
    pub fn new_block(&mut self) -> BlockId {
        self.cfg.create_block()
    }

    /// Switch to a different block for adding statements.
    pub fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
    }

    /// Add an assignment statement: `target = value`
    pub fn assign(&mut self, target: &str, value: Value) {
        let stmt = Statement::Assign {
            target: Variable::original(Text::from(target)),
            value,
        };
        if let Some(block) = self.cfg.get_block_mut(self.current_block) {
            block.add_statement(stmt);
        }
    }

    /// Add a binary operation: `target = left op right`
    pub fn binary_op(&mut self, target: &str, op: BinaryOp, left: Value, right: Value) {
        let stmt = Statement::BinaryOp {
            target: Variable::original(Text::from(target)),
            op,
            left,
            right,
        };
        if let Some(block) = self.cfg.get_block_mut(self.current_block) {
            block.add_statement(stmt);
        }
    }

    /// Add a goto terminator.
    pub fn goto(&mut self, target: BlockId) {
        if let Some(block) = self.cfg.get_block_mut(self.current_block) {
            block.set_terminator(Terminator::Goto(target));
        }
    }

    /// Add a branch terminator.
    pub fn branch(&mut self, condition: Value, true_block: BlockId, false_block: BlockId) {
        if let Some(block) = self.cfg.get_block_mut(self.current_block) {
            block.set_terminator(Terminator::Branch {
                condition,
                true_block,
                false_block,
            });
        }
    }

    /// Add a return terminator.
    pub fn return_value(&mut self, value: Option<Value>) {
        if let Some(block) = self.cfg.get_block_mut(self.current_block) {
            block.set_terminator(Terminator::Return(value));
        }
    }

    /// Set the exit block.
    pub fn set_exit(&mut self, block: BlockId) {
        self.cfg.exit = Some(block);
    }

    /// Build the CFG.
    pub fn build(mut self) -> ControlFlowGraph {
        self.cfg.compute_predecessors();
        self.cfg
    }
}

impl Default for CFGBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Create a variable value from a name.
pub fn var(name: &str) -> Value {
    Value::Variable(Variable::original(Text::from(name)))
}

/// Create an integer constant value.
pub fn int(value: i64) -> Value {
    Value::IntConst(value)
}

/// Create a boolean constant value.
pub fn bool_val(value: bool) -> Value {
    Value::BoolConst(value)
}
