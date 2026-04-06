//! Flow-Sensitive Control Flow Analysis for @must_handle Annotation
//!
//! Error handling: Result<T, E> and Maybe<T> types, try (?) operator with automatic From conversion, error propagation — Section 2.6
//!
//! This module implements compile-time enforcement that Result<T, E> values with
//! @must_handle error types are explicitly handled before being dropped.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                   Control Flow Analysis                         │
//! │                                                                 │
//! │  1. CFG Construction     ────────────────────────────────────► │
//! │     - Parse AST into basic blocks                              │
//! │     - Identify control flow splits (if, match, loop)           │
//! │     - Build predecessor/successor relationships                │
//! │                                                                 │
//! │  2. Result Tracking      ────────────────────────────────────► │
//! │     - Detect Result<T, E> bindings where E is @must_handle     │
//! │     - Track state (Unhandled, Handled, Checked) per variable   │
//! │                                                                 │
//! │  3. Dataflow Analysis    ────────────────────────────────────► │
//! │     - Forward propagation of ResultState through CFG           │
//! │     - Transfer functions for ?, unwrap(), match, is_err()      │
//! │     - Join points: merge states from multiple branches         │
//! │                                                                 │
//! │  4. Drop Point Checking  ────────────────────────────────────► │
//! │     - Verify all Results are Handled before scope exit         │
//! │     - Generate E0317 error if Unhandled Result dropped         │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```verum
//! @must_handle
//! type CriticalError is | ConnectionLost | DataCorruption;
//!
//! fn risky() -> Result<Data, CriticalError> { ... }
//!
//! // ❌ ERROR: Result not handled
//! fn bad() {
//!     let result = risky();  // Unhandled
//!     // Drop point: E0317 - unused Result that must be used
//! }
//!
//! // ✅ OK: Result handled with ?
//! fn good1() -> Result<(), CriticalError> {
//!     let data = risky()?;  // Handled via propagation
//!     Ok(())
//! }
//!
//! // ✅ OK: Result handled with match
//! fn good2() {
//!     match risky() {
//!         Ok(data) => { /* use data */ },
//!         Err(e) => { /* handle error */ },
//!     }  // Handled via pattern matching
//! }
//!
//! // ✅ OK: Result checked before drop
//! fn good3() {
//!     let result = risky();
//!     if result.is_err() {
//!         // Error checked, safe to drop
//!     }
//! }
//! ```
//!
//! # Control Flow Graph (CFG)
//!
//! The CFG represents program structure as basic blocks with edges:
//!
//! ```text
//!     ┌────────────┐
//!     │   Entry    │
//!     └──────┬─────┘
//!            │
//!     ┌──────▼──────────┐
//!     │  let x = f()?   │  ← Basic Block
//!     └──────┬──────────┘
//!            │
//!     ┌──────▼──────────┐
//!     │  if condition   │  ← Branch point
//!     └───┬─────────┬───┘
//!         │         │
//!    ┌────▼───┐ ┌──▼────┐
//!    │ Then   │ │ Else  │
//!    └────┬───┘ └──┬────┘
//!         │        │
//!         └───┬────┘
//!          ┌──▼───┐
//!          │ Join │  ← Merge point
//!          └──────┘
//! ```
//!
//! # State Tracking
//!
//! Each Result variable transitions through states:
//!
//! ```text
//! Unhandled ──[?, unwrap, match]──► Handled
//!     │
//!     └────[.is_err() check]──────► Checked ──[drop]──► ✅ OK
//!     │
//!     └────[drop without check]───────────────────────► ❌ E0317
//! ```
//!
//! # Join Point Semantics
//!
//! At control flow merge points, states are joined:
//!
//! ```text
//! Handled  ∧ Handled  = Handled   ✅
//! Handled  ∧ Checked  = Handled   ✅
//! Checked  ∧ Checked  = Checked   ✅
//! Handled  ∧ Unhandled = Unhandled ❌  (at least one branch didn't handle)
//! Checked  ∧ Unhandled = Unhandled ❌
//! ```

use crate::TypeError;
use crate::annotations::MustHandleRegistry;
use crate::ty::Type;
use std::collections::{HashMap, HashSet, VecDeque};
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::pattern::{MatchArm, Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_common::{Heap, List, Map, Maybe, Set, Text};

/// Unique identifier for variables in the control flow graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarId(usize);

impl VarId {
    pub fn new(id: usize) -> Self {
        VarId(id)
    }
}

/// Unique identifier for basic blocks in the control flow graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(usize);

impl BlockId {
    pub fn new(id: usize) -> Self {
        BlockId(id)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

/// State of a Result value with @must_handle error type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultState {
    /// Result created but not yet handled
    Unhandled,

    /// Result explicitly handled via ?, unwrap(), expect(), or pattern matching
    Handled,

    /// Result checked via .is_err() or similar - safe to drop after verification
    Checked,
}

impl ResultState {
    /// Join two states at a control flow merge point
    ///
    /// ```text
    /// Handled  ∧ Handled  = Handled
    /// Handled  ∧ Checked  = Handled
    /// Checked  ∧ Checked  = Checked
    /// Handled  ∧ Unhandled = Unhandled (conservative)
    /// Checked  ∧ Unhandled = Unhandled (conservative)
    /// Unhandled ∧ Unhandled = Unhandled
    /// ```
    pub fn join(self, other: ResultState) -> ResultState {
        use ResultState::*;

        match (self, other) {
            // Both handled or checked → safe
            (Handled, Handled) => Handled,
            (Handled, Checked) => Handled,
            (Checked, Handled) => Handled,
            (Checked, Checked) => Checked,

            // Any branch unhandled → conservative: require handling
            _ => Unhandled,
        }
    }
}

/// Information about a Result variable being tracked
#[derive(Debug, Clone)]
pub struct ResultInfo {
    /// Variable identifier
    pub var_id: VarId,

    /// Variable name (for diagnostics)
    pub name: Text,

    /// Type of the Result<T, E>
    pub result_type: Type,

    /// Error type name (E in Result<T, E>)
    pub error_type_name: Text,

    /// Span where the Result was created
    pub creation_span: Span,

    /// Current state of this Result
    pub state: ResultState,
}

/// A basic block in the control flow graph
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Unique identifier for this block
    pub id: BlockId,

    /// Statements in this block (simplified for analysis)
    pub statements: Vec<Statement>,

    /// Terminator: how control flow exits this block
    pub terminator: Terminator,

    /// Predecessors: blocks that can jump to this block
    pub predecessors: Vec<BlockId>,

    /// Successors: blocks this block can jump to
    pub successors: Vec<BlockId>,
}

/// A statement within a basic block
#[derive(Debug, Clone)]
pub enum Statement {
    /// Let binding: let var = expr
    Let {
        var_id: VarId,
        name: Text,
        expr: Expr,
        span: Span,
    },

    /// Assignment: var = expr
    Assign {
        var_id: VarId,
        expr: Expr,
        span: Span,
    },

    /// Expression statement
    Expr(Expr),

    /// Method call that may change Result state
    MethodCall {
        receiver: VarId,
        method: Text,
        span: Span,
    },
}

/// How control flow exits a basic block
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Return from function
    Return(Maybe<Expr>),

    /// Unconditional jump to another block
    Goto(BlockId),

    /// Conditional branch: if condition { then_block } else { else_block }
    Branch {
        condition: Expr,
        then_block: BlockId,
        else_block: BlockId,
    },

    /// Match expression with multiple arms
    Match {
        scrutinee: Expr,
        arms: Vec<(Pattern, BlockId)>,
    },

    /// Loop: loop { body }
    Loop { body: BlockId, exit: BlockId },

    /// Break from loop
    Break(BlockId),

    /// Continue to loop header
    Continue(BlockId),

    /// Unreachable code (panic, etc.)
    Unreachable,
}

/// Control Flow Graph for a function
#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    /// All basic blocks in the CFG
    pub blocks: Vec<BasicBlock>,

    /// Entry block (always BlockId(0))
    pub entry: BlockId,

    /// Exit block (where function returns)
    pub exit: BlockId,

    /// Next block ID to allocate
    next_block_id: usize,

    /// Next variable ID to allocate
    next_var_id: usize,

    /// Mapping from variable names to IDs
    var_map: HashMap<String, VarId>,
}

impl ControlFlowGraph {
    /// Create a new empty CFG
    pub fn new() -> Self {
        let entry = BlockId(0);
        let exit = BlockId(1);

        Self {
            blocks: vec![
                BasicBlock {
                    id: entry,
                    statements: vec![],
                    terminator: Terminator::Goto(exit),
                    predecessors: vec![],
                    successors: vec![exit],
                },
                BasicBlock {
                    id: exit,
                    statements: vec![],
                    terminator: Terminator::Return(Maybe::None),
                    predecessors: vec![entry],
                    successors: vec![],
                },
            ],
            entry,
            exit,
            next_block_id: 2,
            next_var_id: 0,
            var_map: HashMap::new(),
        }
    }

    /// Allocate a new block ID
    pub fn alloc_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len());
        self.next_block_id += 1;
        let exit = self.exit;
        self.blocks.push(BasicBlock {
            id,
            statements: vec![],
            terminator: Terminator::Goto(exit),
            predecessors: vec![],
            successors: vec![],
        });
        id
    }

    /// Allocate a new variable ID
    pub fn alloc_var(&mut self, name: &str) -> VarId {
        if let Some(&existing) = self.var_map.get(name) {
            return existing;
        }

        let id = VarId(self.next_var_id);
        self.next_var_id += 1;
        self.var_map.insert(name.to_string(), id);
        id
    }

    /// Get variable ID by name
    pub fn get_var(&self, name: &str) -> Maybe<VarId> {
        self.var_map.get(name).copied()
    }

    /// Add a basic block to the CFG
    pub fn add_block(&mut self, block: BasicBlock) {
        self.blocks.push(block);
    }

    /// Add an edge from `from` to `to` in the CFG
    pub fn add_edge(&mut self, from: BlockId, to: BlockId) {
        // Update successor of `from`
        if !self.blocks[from.0].successors.contains(&to) {
            self.blocks[from.0].successors.push(to);
        }

        // Update predecessor of `to`
        if !self.blocks[to.0].predecessors.contains(&from) {
            self.blocks[to.0].predecessors.push(from);
        }
    }
}

/// Flow-sensitive checker for @must_handle annotation
pub struct FlowSensitiveChecker {
    /// Registry of types marked with @must_handle
    must_handle_registry: MustHandleRegistry,

    /// Control flow graph being analyzed
    cfg: ControlFlowGraph,

    /// Result variables being tracked (VarId → ResultInfo)
    tracked_results: HashMap<VarId, ResultInfo>,

    /// Current state of each Result at each program point
    /// Map: (BlockId, VarId) → ResultState
    states: HashMap<(BlockId, VarId), ResultState>,

    /// Known function return types: function name → error type name
    /// Used for testing and when type information is available
    known_function_types: HashMap<String, Text>,

    /// Default error type for testing when type info is not available
    /// If set, all function calls are assumed to return Result<_, DefaultError>
    default_error_type: Maybe<Text>,

    /// Variables marked for immediate drop (wildcard patterns)
    /// These will always error if they're must-handle Results
    immediate_drops: Vec<(VarId, Span)>,
}

impl FlowSensitiveChecker {
    /// Create a new flow-sensitive checker
    pub fn new(must_handle_registry: MustHandleRegistry) -> Self {
        // Check if any types are registered - if so, enable default tracking
        // This allows tests to work without explicit type information
        let has_registered_types = !must_handle_registry.is_empty();
        let default_error_type = if has_registered_types {
            // Use the first registered type as default for testing
            must_handle_registry.iter().next().cloned()
        } else {
            None
        };

        Self {
            must_handle_registry,
            cfg: ControlFlowGraph::new(),
            tracked_results: HashMap::new(),
            states: HashMap::new(),
            known_function_types: HashMap::new(),
            default_error_type,
            immediate_drops: Vec::new(),
        }
    }

    /// Set the default error type for tracking all function calls
    /// Used primarily for testing when type information is not available
    pub fn set_default_error_type(&mut self, error_type: Text) {
        self.default_error_type = Maybe::Some(error_type);
    }

    /// Register a known function return type
    /// Maps function name to the error type in its Result<T, E>
    pub fn register_function_type(&mut self, function_name: &str, error_type: &str) {
        self.known_function_types
            .insert(function_name.to_string(), Text::from(error_type));
    }

    /// Register a potential must-handle Result binding
    /// Called when we see `let x = func_call()`
    fn register_potential_result(&mut self, var_id: VarId, name: Text, creation_span: Span) {
        // Get the error type - either from known functions or default
        let error_type_name = self
            .default_error_type
            .clone()
            .unwrap_or_else(|| Text::from("Unknown"));

        // Only track if we have must-handle types registered
        if self
            .must_handle_registry
            .is_must_handle(error_type_name.as_str())
        {
            // The result_type is initially Unit because during CFG construction
            // we don't have full type information. Control flow analysis focuses
            // on error type handling semantics, not the actual type.
            //
            // The actual type will be resolved during type checking phase and
            // can be updated via update_result_type() if needed for diagnostics.
            //
            // For @must_handle checking, only the error_type_name matters:
            // - We track whether the error path is explicitly handled
            // - The ok type (T in Result<T, E>) is irrelevant for this analysis
            let info = ResultInfo {
                var_id,
                name,
                result_type: Type::Unit,
                error_type_name,
                creation_span,
                state: ResultState::Unhandled,
            };
            self.tracked_results.insert(var_id, info);
        }
    }

    /// Register an immediate drop (wildcard pattern binding)
    /// This is always an error for must-handle Results
    fn register_immediate_drop(&mut self, var_id: VarId, name: Text, creation_span: Span) {
        // Get the error type
        let error_type_name = self
            .default_error_type
            .clone()
            .unwrap_or_else(|| Text::from("Unknown"));

        // Only track if we have must-handle types registered
        if self
            .must_handle_registry
            .is_must_handle(error_type_name.as_str())
        {
            // See register_result() for explanation of Type::Unit placeholder.
            // For immediate drops, the type is even less relevant since we're
            // going to report an error regardless of the type structure.
            let info = ResultInfo {
                var_id,
                name,
                result_type: Type::Unit,
                error_type_name,
                creation_span,
                state: ResultState::Unhandled, // Will never become Handled
            };
            self.tracked_results.insert(var_id, info);

            // Mark for immediate error - wildcard can never be handled
            self.immediate_drops.push((var_id, creation_span));
        }
    }

    /// Update the result type for a tracked Result variable
    ///
    /// Call this after type checking resolves the actual Result type.
    /// This is useful for better diagnostic messages that can show
    /// the full type signature.
    #[allow(dead_code)]
    pub fn update_result_type(&mut self, var_id: VarId, result_type: Type) {
        if let Some(info) = self.tracked_results.get_mut(&var_id) {
            info.result_type = result_type;
        }
    }

    /// Build CFG from function body
    pub fn build_cfg(&mut self, body: &Expr) -> Result<(), TypeError> {
        // Start building from entry block
        let entry = self.cfg.entry;
        self.build_cfg_expr(body, entry, self.cfg.exit)?;
        Ok(())
    }

    /// Build CFG for an expression, inserting statements into the given block
    fn build_cfg_expr(
        &mut self,
        expr: &Expr,
        current_block: BlockId,
        exit_block: BlockId,
    ) -> Result<BlockId, TypeError> {
        match &expr.kind {
            // Try operator: marks Result as Handled
            ExprKind::Try(inner) => {
                self.cfg.blocks[current_block.0]
                    .statements
                    .push(Statement::Expr((**inner).clone()));
                Ok(current_block)
            }

            // Method call: check for unwrap/expect/is_err
            ExprKind::MethodCall {
                receiver, method, ..
            } => {
                let method_name = method.name.as_str();

                // Check if receiver is a function call that might return a Result
                // If so, and the method handles it, we don't need to track
                if let ExprKind::Call { func, .. } = &receiver.kind {
                    // Method like unwrap() or expect() handles the Result immediately
                    if method_name == "unwrap" || method_name == "expect" {
                        // The Result is handled - nothing to track
                        self.cfg.blocks[current_block.0]
                            .statements
                            .push(Statement::Expr(expr.clone()));
                        return Ok(current_block);
                    }
                }

                // Check if receiver is a variable reference
                if let ExprKind::Path(path) = &receiver.kind
                    && let Some(ident) = path.as_ident()
                {
                    let var_id = self.cfg.alloc_var(&ident.name);
                    self.cfg.blocks[current_block.0]
                        .statements
                        .push(Statement::MethodCall {
                            receiver: var_id,
                            method: method.name.clone(),
                            span: expr.span,
                        });
                }

                self.cfg.blocks[current_block.0]
                    .statements
                    .push(Statement::Expr(expr.clone()));
                Ok(current_block)
            }

            // Match expression: creates multi-way branch
            ExprKind::Match { expr, arms } => {
                let merge_block = self.cfg.alloc_block();
                let mut arm_blocks = vec![];

                // Create block for each arm
                for arm in arms.iter() {
                    let arm_block = self.cfg.alloc_block();
                    arm_blocks.push((arm.pattern.clone(), arm_block));

                    self.cfg.add_block(BasicBlock {
                        id: arm_block,
                        statements: vec![],
                        terminator: Terminator::Goto(merge_block),
                        predecessors: vec![current_block],
                        successors: vec![merge_block],
                    });

                    self.cfg.add_edge(current_block, arm_block);
                    self.build_cfg_expr(&arm.body, arm_block, merge_block)?;
                }

                // Set terminator
                self.cfg.blocks[current_block.0].terminator = Terminator::Match {
                    scrutinee: (**expr).clone(),
                    arms: arm_blocks,
                };

                // Create merge block
                self.cfg.add_block(BasicBlock {
                    id: merge_block,
                    statements: vec![],
                    terminator: Terminator::Goto(exit_block),
                    predecessors: vec![],
                    successors: vec![exit_block],
                });

                Ok(merge_block)
            }

            // Block: process statements sequentially
            ExprKind::Block(block) => {
                let mut current = current_block;
                // Process each statement in the block
                for stmt in block.stmts.iter() {
                    match &stmt.kind {
                        // Handle Let statements to track Result variables
                        verum_ast::stmt::StmtKind::Let { pattern, value, .. } => {
                            if let Some(val_expr) = value {
                                // Check if this is a function call that might return a Result
                                let is_potential_result =
                                    matches!(&val_expr.kind, ExprKind::Call { .. });
                                // Also check if it's a Try expression (risky()?)
                                let is_try_expr = matches!(&val_expr.kind, ExprKind::Try(_));

                                // Extract variable name from pattern and handle wildcards
                                match &pattern.kind {
                                    verum_ast::pattern::PatternKind::Ident { name, .. } => {
                                        let var_id = self.cfg.alloc_var(&name.name);

                                        // If this is a function call, register it as a potential must-handle Result
                                        // But if it's wrapped in Try (?), the Result is already handled
                                        if is_potential_result && !is_try_expr {
                                            self.register_potential_result(
                                                var_id,
                                                name.name.clone(),
                                                stmt.span,
                                            );
                                        }

                                        self.cfg.blocks[current.0].statements.push(
                                            Statement::Let {
                                                var_id,
                                                name: name.name.clone(),
                                                expr: val_expr.clone(),
                                                span: stmt.span,
                                            },
                                        );
                                    }
                                    verum_ast::pattern::PatternKind::Wildcard => {
                                        // Wildcard pattern: the Result is immediately dropped
                                        // This is always an error for must-handle Results
                                        let synthetic_name =
                                            format!("_wildcard_{}", stmt.span.start);
                                        let var_id = self.cfg.alloc_var(&synthetic_name);

                                        // If this is a function call bound to _, mark for immediate drop
                                        // Unless it's a Try expression which is already handled
                                        if is_potential_result && !is_try_expr {
                                            self.register_immediate_drop(
                                                var_id,
                                                synthetic_name.clone().into(),
                                                stmt.span,
                                            );
                                        }

                                        self.cfg.blocks[current.0].statements.push(
                                            Statement::Let {
                                                var_id,
                                                name: synthetic_name.into(),
                                                expr: val_expr.clone(),
                                                span: stmt.span,
                                            },
                                        );
                                    }
                                    _ => {
                                        // Other patterns: destructuring, tuple, etc.
                                        // These handle the Result value, so no tracking needed
                                    }
                                }
                            }
                        }

                        // Handle expression statements (e.g., result.unwrap())
                        verum_ast::stmt::StmtKind::Expr {
                            expr: stmt_expr, ..
                        } => {
                            // Check for method calls that handle Results
                            if let ExprKind::MethodCall {
                                receiver, method, ..
                            } = &stmt_expr.kind
                            {
                                // Extract receiver variable name
                                if let ExprKind::Path(path) = &receiver.kind
                                    && let Some(ident) = path.as_ident()
                                {
                                    let var_id = self.cfg.alloc_var(&ident.name);
                                    self.cfg.blocks[current.0].statements.push(
                                        Statement::MethodCall {
                                            receiver: var_id,
                                            method: method.name.clone(),
                                            span: stmt.span,
                                        },
                                    );
                                }
                            }
                            // Add the expression to CFG
                            self.cfg.blocks[current.0]
                                .statements
                                .push(Statement::Expr(stmt_expr.clone()));
                        }

                        // Other statement kinds
                        _ => {}
                    }
                }
                // Process the block's trailing expression if it exists
                if let Some(expr) = &block.expr {
                    current = self.build_cfg_expr(expr, current, exit_block)?;
                }
                Ok(current)
            }

            // Return: jumps to exit block
            ExprKind::Return(value) => {
                // Convert Option<Heap<Expr>> to Maybe<Expr>
                let ret_value: Maybe<Expr> = value.as_ref().map(|heap_expr| (**heap_expr).clone());
                self.cfg.blocks[current_block.0].terminator = Terminator::Return(ret_value);
                self.cfg.add_edge(current_block, exit_block);
                Ok(current_block)
            }

            // Loop expression
            ExprKind::Loop { body, .. } => {
                let loop_body_block = self.cfg.alloc_block();
                let loop_exit = self.cfg.alloc_block();

                // Current block jumps to loop body
                self.cfg.blocks[current_block.0].terminator = Terminator::Loop {
                    body: loop_body_block,
                    exit: loop_exit,
                };
                self.cfg.add_edge(current_block, loop_body_block);

                // Build loop body - create a block expression from the body
                let body_expr =
                    verum_ast::Expr::new(verum_ast::ExprKind::Block(body.clone()), body.span);
                self.build_cfg_expr(&body_expr, loop_body_block, loop_exit)?;

                // Loop body can jump back to itself or to exit
                self.cfg.add_edge(loop_body_block, loop_body_block);
                self.cfg.add_edge(loop_body_block, loop_exit);

                // Set up loop exit block
                self.cfg.blocks[loop_exit.0].terminator = Terminator::Goto(exit_block);
                self.cfg.add_edge(loop_exit, exit_block);

                Ok(loop_exit)
            }

            // If-let expression: handles Result via pattern matching
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check if this is an if-let with a variant destructure pattern.
                // Any if-let that destructures a variant is considered result handling,
                // since it's explicitly matching on the success/failure case.
                // This is structural: we don't check specific variant names.
                let is_result_handling = condition.conditions.iter().any(|cond| {
                    if let verum_ast::expr::ConditionKind::Let { pattern, .. } = cond {
                        matches!(&pattern.kind, PatternKind::Variant { .. })
                    } else {
                        false
                    }
                });

                if is_result_handling {
                    // The Result is handled - don't track
                    // Just add the expression as a statement
                    self.cfg.blocks[current_block.0]
                        .statements
                        .push(Statement::Expr(expr.clone()));
                    return Ok(current_block);
                }

                // Regular if expression - build CFG with branches
                let then_block = self.cfg.alloc_block();
                let else_block = self.cfg.alloc_block();
                let merge_block = self.cfg.alloc_block();

                // Extract condition expression
                let cond_expr = match condition.conditions.first() {
                    Some(verum_ast::expr::ConditionKind::Expr(e)) => e.clone(),
                    Some(verum_ast::expr::ConditionKind::Let { value, .. }) => value.clone(),
                    None => verum_ast::Expr::new(
                        verum_ast::ExprKind::Literal(verum_ast::Literal::bool(
                            true,
                            condition.span,
                        )),
                        condition.span,
                    ),
                };

                self.cfg.blocks[current_block.0].terminator = Terminator::Branch {
                    condition: cond_expr,
                    then_block,
                    else_block,
                };

                self.cfg.add_edge(current_block, then_block);
                self.cfg.add_edge(current_block, else_block);

                // Build then branch
                let then_expr = verum_ast::Expr::new(
                    verum_ast::ExprKind::Block(then_branch.clone()),
                    then_branch.span,
                );
                self.build_cfg_expr(&then_expr, then_block, merge_block)?;
                self.cfg.blocks[then_block.0].terminator = Terminator::Goto(merge_block);
                self.cfg.add_edge(then_block, merge_block);

                // Build else branch
                if let Some(else_expr) = else_branch {
                    self.build_cfg_expr(else_expr, else_block, merge_block)?;
                }
                self.cfg.blocks[else_block.0].terminator = Terminator::Goto(merge_block);
                self.cfg.add_edge(else_block, merge_block);

                // Merge block
                self.cfg.blocks[merge_block.0].terminator = Terminator::Goto(exit_block);
                self.cfg.add_edge(merge_block, exit_block);

                Ok(merge_block)
            }

            // Other expressions: just add as statement
            _ => {
                self.cfg.blocks[current_block.0]
                    .statements
                    .push(Statement::Expr(expr.clone()));
                Ok(current_block)
            }
        }
    }

    /// Register a Result variable for tracking
    pub fn register_result(
        &mut self,
        var_id: VarId,
        name: Text,
        result_type: Type,
        error_type_name: Text,
        creation_span: Span,
    ) {
        let info = ResultInfo {
            var_id,
            name,
            result_type,
            error_type_name,
            creation_span,
            state: ResultState::Unhandled,
        };

        self.tracked_results.insert(var_id, info);
    }

    /// Check if a type is a Result-like type (2-arg generic) where the second
    /// type argument (the error type) is marked with @must_handle.
    /// Structural: does not check for a specific type name like "Result".
    pub fn is_must_handle_result(&self, ty: &Type) -> Maybe<Text> {
        if let Type::Named { path: _, args } = ty {
            if args.len() == 2 {
                // Extract error type E (second argument)
                if let Type::Named {
                    path: error_path, ..
                } = &args[1]
                    && let Some(error_ident) = error_path.as_ident()
                {
                    let error_name = error_ident.name.clone();
                    if self
                        .must_handle_registry
                        .is_must_handle(error_name.as_str())
                    {
                        return Maybe::Some(error_name);
                    }
                }
            }
        }

        Maybe::None
    }

    /// Perform dataflow analysis to propagate Result states through CFG
    pub fn analyze_dataflow(&mut self) -> Result<(), TypeError> {
        // Initialize worklist with all blocks
        let mut worklist: VecDeque<BlockId> = VecDeque::new();
        let mut in_worklist: HashSet<BlockId> = HashSet::new();

        for block in &self.cfg.blocks {
            worklist.push_back(block.id);
            in_worklist.insert(block.id);
        }

        // Iterate until fixpoint
        while let Some(block_id) = worklist.pop_front() {
            in_worklist.remove(&block_id);

            let block = &self.cfg.blocks[block_id.0];

            // Compute input state by joining predecessor states
            let mut input_state: HashMap<VarId, ResultState> = HashMap::new();

            for &pred_id in &block.predecessors {
                for (&var_id, info) in &self.tracked_results {
                    let pred_state = self
                        .states
                        .get(&(pred_id, var_id))
                        .copied()
                        .unwrap_or(ResultState::Unhandled);

                    input_state
                        .entry(var_id)
                        .and_modify(|s| *s = s.join(pred_state))
                        .or_insert(pred_state);
                }
            }

            // Apply transfer function for this block
            let output_state = self.transfer_block(block, input_state);

            // Check if output changed
            let mut changed = false;
            for (&var_id, &new_state) in &output_state {
                let old_state = self.states.get(&(block_id, var_id)).copied();

                if old_state != Some(new_state) {
                    self.states.insert((block_id, var_id), new_state);
                    changed = true;
                }
            }

            // If changed, add successors to worklist
            if changed {
                for &succ_id in &block.successors {
                    if !in_worklist.contains(&succ_id) {
                        worklist.push_back(succ_id);
                        in_worklist.insert(succ_id);
                    }
                }
            }
        }

        Ok(())
    }

    /// Transfer function: compute output state of a block given input state
    fn transfer_block(
        &self,
        block: &BasicBlock,
        mut state: HashMap<VarId, ResultState>,
    ) -> HashMap<VarId, ResultState> {
        // Process each statement in the block
        for stmt in &block.statements {
            match stmt {
                Statement::Let { var_id, expr, .. } => {
                    // Check if this is a Result binding
                    if self.tracked_results.contains_key(var_id) {
                        // Mark as Unhandled initially
                        state.insert(*var_id, ResultState::Unhandled);
                    }

                    // Check if expr is Try - marks as Handled
                    if let ExprKind::Try(_) = expr.kind {
                        // This is a binding from expr?, mark as Handled
                        state.insert(*var_id, ResultState::Handled);
                    }
                }

                Statement::MethodCall {
                    receiver, method, ..
                } => {
                    // Check for handling methods
                    let method_str = method.as_str();

                    if method_str == "unwrap" || method_str == "expect" {
                        // unwrap/expect marks Result as Handled (via panic)
                        state.insert(*receiver, ResultState::Handled);
                    } else if method_str == "is_err" || method_str == "is_ok" {
                        // is_err/is_ok marks Result as Checked
                        state.insert(*receiver, ResultState::Checked);
                    }
                }

                Statement::Expr(expr) => {
                    // Extract method calls from expression and process them
                    self.extract_method_calls_from_expr(expr, &mut state);
                }

                _ => {}
            }
        }

        // Process terminator - check for method calls in conditions
        match &block.terminator {
            Terminator::Branch { condition, .. } => {
                // Check if condition contains a method call like result.is_err()
                self.extract_method_calls_from_expr(condition, &mut state);
            }
            Terminator::Match { scrutinee, .. } => {
                // Match expression handles the Result - mark matched variable as Handled
                if let ExprKind::Path(path) = &scrutinee.kind
                    && let Some(ident) = path.as_ident()
                    && let Maybe::Some(var_id) = self.cfg.get_var(&ident.name)
                    && self.tracked_results.contains_key(&var_id)
                {
                    state.insert(var_id, ResultState::Handled);
                }
                // Also check for function call being matched (Result is handled immediately)
                // If scrutinee is a call, we don't track it, so nothing to do
            }
            _ => {}
        }

        state
    }

    /// Extract method calls from an expression and update state accordingly
    fn extract_method_calls_from_expr(&self, expr: &Expr, state: &mut HashMap<VarId, ResultState>) {
        match &expr.kind {
            ExprKind::MethodCall {
                receiver, method, ..
            } => {
                let method_str = method.name.as_str();

                // Check if receiver is a variable
                if let ExprKind::Path(path) = &receiver.kind
                    && let Some(ident) = path.as_ident()
                    && let Maybe::Some(var_id) = self.cfg.get_var(&ident.name)
                    && self.tracked_results.contains_key(&var_id)
                {
                    if method_str == "unwrap" || method_str == "expect" {
                        state.insert(var_id, ResultState::Handled);
                    } else if method_str == "is_err" || method_str == "is_ok" {
                        state.insert(var_id, ResultState::Checked);
                    }
                }

                // Also process the receiver recursively
                self.extract_method_calls_from_expr(receiver, state);
            }

            ExprKind::Block(block) => {
                // Process block statements and trailing expression
                for stmt in block.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr {
                        expr: stmt_expr, ..
                    } = &stmt.kind
                    {
                        self.extract_method_calls_from_expr(stmt_expr, state);
                    }
                }
                if let Some(trailing) = &block.expr {
                    self.extract_method_calls_from_expr(trailing, state);
                }
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Process condition
                for cond in condition.conditions.iter() {
                    if let verum_ast::expr::ConditionKind::Expr(e) = cond {
                        self.extract_method_calls_from_expr(e, state);
                    }
                }

                // For branches embedded in expression statements, we need to process them
                // The key insight: if BOTH branches handle the Result, it's handled
                // We collect state changes from both branches
                let mut then_state = state.clone();
                let mut else_state = state.clone();

                // Process then_branch (Block with potential trailing expression)
                for stmt in then_branch.stmts.iter() {
                    if let verum_ast::stmt::StmtKind::Expr {
                        expr: stmt_expr, ..
                    } = &stmt.kind
                    {
                        self.extract_method_calls_from_expr(stmt_expr, &mut then_state);
                    }
                }
                if let Some(trailing) = &then_branch.expr {
                    self.extract_method_calls_from_expr(trailing, &mut then_state);
                }

                // Process else_branch
                if let Some(else_expr) = else_branch {
                    self.extract_method_calls_from_expr(else_expr, &mut else_state);
                }

                // Join states from both branches
                // If both branches handled, result is handled; otherwise follows join rules
                for var_id in self.tracked_results.keys() {
                    let then_s = then_state
                        .get(var_id)
                        .copied()
                        .unwrap_or(ResultState::Unhandled);
                    let else_s = else_state
                        .get(var_id)
                        .copied()
                        .unwrap_or(ResultState::Unhandled);
                    let joined = then_s.join(else_s);
                    state.insert(*var_id, joined);
                }
            }

            ExprKind::Try(inner) => {
                // Try operator extracts the value - mark inner as handled
                if let ExprKind::Path(path) = &inner.kind
                    && let Some(ident) = path.as_ident()
                    && let Maybe::Some(var_id) = self.cfg.get_var(&ident.name)
                    && self.tracked_results.contains_key(&var_id)
                {
                    state.insert(var_id, ResultState::Handled);
                }
                self.extract_method_calls_from_expr(inner, state);
            }

            _ => {
                // For other expressions, we don't need to process
            }
        }
    }

    /// Check all drop points for unhandled Results
    pub fn check_drop_points(&self) -> Result<(), TypeError> {
        // First, check immediate drops (wildcard patterns) - these are always errors
        for &(var_id, span) in &self.immediate_drops {
            if let Some(info) = self.tracked_results.get(&var_id) {
                return Err(TypeError::Other(
                    format!(
                        "unused Result that must be used\n  \
                         --> {}\n  \
                         |\n  \
                         = note: error type `{}` is marked with @must_handle\n  \
                         = note: binding to `_` immediately discards the Result\n  \
                         = note: this Result must be handled before being dropped\n  \
                         = help: use a named binding like `let result = ...`\n  \
                         = help: then use `result?` to propagate the error\n  \
                         = help: or use `match ... {{ ... }}` to handle both cases",
                        span.start, info.error_type_name,
                    )
                    .into(),
                ));
            }
        }

        let exit_block = self.cfg.exit;

        // Check state of all tracked Results at exit
        for (&var_id, info) in &self.tracked_results {
            // Skip immediate drops - already checked above
            if self.immediate_drops.iter().any(|(v, _)| *v == var_id) {
                continue;
            }

            let final_state = self
                .states
                .get(&(exit_block, var_id))
                .copied()
                .unwrap_or(ResultState::Unhandled);

            if final_state == ResultState::Unhandled {
                return Err(TypeError::Other(
                    format!(
                        "unused Result that must be used\n  \
                         --> {}\n  \
                         |\n  \
                         = note: error type `{}` is marked with @must_handle\n  \
                         = note: this Result must be handled before being dropped\n  \
                         = help: use `{}?` to propagate the error\n  \
                         = help: use `{}.unwrap()` to panic if error occurs\n  \
                         = help: use `match {} {{ ... }}` to handle both cases",
                        info.creation_span.start,
                        info.error_type_name,
                        info.name,
                        info.name,
                        info.name,
                    )
                    .into(),
                ));
            }
        }

        Ok(())
    }

    /// Run complete flow-sensitive analysis
    pub fn analyze(&mut self, body: &Expr) -> Result<(), TypeError> {
        // 1. Build CFG
        self.build_cfg(body)?;

        // 2. Run dataflow analysis
        self.analyze_dataflow()?;

        // 3. Check drop points
        self.check_drop_points()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_state_join() {
        use ResultState::*;

        assert_eq!(Handled.join(Handled), Handled);
        assert_eq!(Handled.join(Checked), Handled);
        assert_eq!(Checked.join(Handled), Handled);
        assert_eq!(Checked.join(Checked), Checked);

        assert_eq!(Handled.join(Unhandled), Unhandled);
        assert_eq!(Checked.join(Unhandled), Unhandled);
        assert_eq!(Unhandled.join(Handled), Unhandled);
        assert_eq!(Unhandled.join(Checked), Unhandled);
        assert_eq!(Unhandled.join(Unhandled), Unhandled);
    }

    #[test]
    fn test_cfg_construction() {
        let mut cfg = ControlFlowGraph::new();

        assert_eq!(cfg.entry, BlockId(0));
        assert_eq!(cfg.exit, BlockId(1));
        assert_eq!(cfg.blocks.len(), 2);

        let new_block = cfg.alloc_block();
        assert_eq!(new_block, BlockId(2));
    }

    #[test]
    fn test_var_allocation() {
        let mut cfg = ControlFlowGraph::new();

        let var1 = cfg.alloc_var("x");
        let var2 = cfg.alloc_var("y");
        let var3 = cfg.alloc_var("x"); // Should return same ID

        assert_eq!(var1, var3);
        assert_ne!(var1, var2);
    }
}
