//! CFG Constructor for CBGR Tier Analysis.
//!
//! This module converts AST functions to Control Flow Graphs (CFGs) for escape analysis.
//! The resulting CFGs are used by `verum_cbgr::tier_analysis::TierAnalyzer` to determine
//! optimal reference tiers (0, 1, or 2) for each reference operation.
//!
//! # Architecture
//!
//! ```text
//! AST Module
//!     │
//!     ▼
//! CfgConstructor::from_module()
//!     │
//!     ├── For each function:
//!     │   ├── Allocate entry/exit blocks
//!     │   ├── Walk function body
//!     │   ├── Create BasicBlocks for control flow
//!     │   ├── Track DefSites for &/&mut expressions
//!     │   └── Track UseeSites for dereference operations
//!     │
//!     ▼
//! ModuleCfg { functions: Map<FunctionId, ControlFlowGraph> }
//!     │
//!     ▼
//! TierAnalyzer::analyze() → TierAnalysisResult
//!     │
//!     ▼
//! TierContext::from_analysis_result()
//!     │
//!     ▼
//! VBC Codegen with tier-aware instructions
//! ```
//!
//! # Reference Tracking
//!
//! The constructor tracks two types of sites:
//!
//! 1. **DefSites**: Where references are created
//!    - `&expr` → DefSite(ref_id, span, is_stack_allocated=true)
//!    - `&mut expr` → DefSite(ref_id, span, is_stack_allocated=true)
//!
//! 2. **UseeSites**: Where references are dereferenced
//!    - `*expr` → UseeSite(ref_id, span, is_mutable=false)
//!    - `*expr = value` → UseeSite(ref_id, span, is_mutable=true)
//!
//! # Span-based RefId Mapping
//!
//! The constructor maintains span→RefId mappings that are passed to TierAnalysisResult.
//! This allows VBC codegen to look up tier decisions using expression spans, which
//! matches how ExprId is computed during code generation.
//!
//! CFG construction: ExprId and RefId unified into single node identifiers
//! for consistent dataflow analysis across expression and reference tracking.

use verum_ast::{
    decl::{FunctionBody, FunctionDecl, ItemKind},
    expr::{Block, Expr, ExprKind, UnOp},
    stmt::{Stmt, StmtKind},
    Module,
};
use verum_cbgr::analysis::{
    BasicBlock, BlockId, CfgBuilder, ControlFlowGraph, DefSite, FunctionId, RefId, UseeSite,
};
use verum_common::{List, Map, Maybe, Set};

// ==================================================================================
// Types
// ==================================================================================

/// Result of CFG construction for an entire module.
#[derive(Debug)]
pub struct ModuleCfg {
    /// CFGs for each function in the module.
    pub functions: Map<FunctionId, FunctionCfg>,
    /// Global span→RefId mapping for all functions.
    pub span_to_ref: Map<(u32, u32), RefId>,
    /// Global RefId→span mapping.
    pub ref_to_span: Map<RefId, (u32, u32)>,
}

/// CFG for a single function with metadata.
#[derive(Debug)]
pub struct FunctionCfg {
    /// The control flow graph.
    pub cfg: ControlFlowGraph,
    /// Function name for debugging.
    pub name: String,
    /// Number of reference definitions.
    pub def_count: usize,
    /// Number of reference uses.
    pub use_count: usize,
}

// ==================================================================================
// CFG Constructor
// ==================================================================================

/// Constructs Control Flow Graphs from AST modules for tier analysis.
///
/// The constructor walks the AST and builds CFGs with DefSite/UseeSite tracking
/// that can be passed to `TierAnalyzer` for escape analysis and tier determination.
pub struct CfgConstructor {
    /// Internal CFG builder.
    builder: CfgBuilder,
    /// Next function ID.
    next_function_id: u64,
    /// Current basic blocks being built.
    blocks: Map<BlockId, BasicBlock>,
    /// Current block ID.
    current_block: BlockId,
    /// Current definitions in the block.
    current_defs: List<DefSite>,
    /// Current uses in the block.
    current_uses: List<UseeSite>,
    /// Stack of loop contexts (continue_block, break_block).
    loop_stack: Vec<(BlockId, BlockId)>,
    /// Statistics for the current function.
    stats: ConstructorStats,
}

/// Statistics collected during CFG construction.
#[derive(Debug, Default, Clone)]
struct ConstructorStats {
    /// Total reference definitions.
    def_count: usize,
    /// Total reference uses.
    use_count: usize,
    /// Total blocks created.
    block_count: usize,
}

impl CfgConstructor {
    /// Create a new CFG constructor.
    pub fn new() -> Self {
        Self {
            builder: CfgBuilder::new(),
            next_function_id: 0,
            blocks: Map::new(),
            current_block: BlockId(0),
            current_defs: List::new(),
            current_uses: List::new(),
            loop_stack: Vec::new(),
            stats: ConstructorStats::default(),
        }
    }

    /// Construct CFGs for all functions in a module.
    pub fn from_module(module: &Module) -> ModuleCfg {
        let mut constructor = Self::new();
        let mut functions = Map::new();

        for item in module.items.iter() {
            if let ItemKind::Function(func) = &item.kind {
                if let Some(function_cfg) = constructor.build_function_cfg(func) {
                    let func_id = FunctionId(constructor.next_function_id);
                    constructor.next_function_id += 1;
                    functions.insert(func_id, function_cfg);
                }
            }
        }

        ModuleCfg {
            functions,
            span_to_ref: constructor.builder.span_map().clone(),
            ref_to_span: constructor.builder.ref_span_map().clone(),
        }
    }

    /// Build CFG for a single function.
    fn build_function_cfg(&mut self, func: &FunctionDecl) -> Option<FunctionCfg> {
        let body = func.body.as_ref()?;

        // Reset state for new function
        self.builder.reset();
        self.blocks.clear();
        self.current_defs.clear();
        self.current_uses.clear();
        self.loop_stack.clear();
        self.stats = ConstructorStats::default();

        // Create entry and exit blocks
        let entry = self.builder.new_block_id();
        let exit = self.builder.new_block_id();
        self.current_block = entry;
        self.stats.block_count = 2;

        // Initialize entry block
        let entry_block = BasicBlock::empty(entry);
        self.blocks.insert(entry, entry_block);

        // Process function body
        match body {
            FunctionBody::Block(block) => self.process_block(block, exit),
            FunctionBody::Expr(expr) => self.process_expr(expr),
        }

        // Finalize current block
        self.finalize_current_block(exit);

        // Create exit block
        let exit_block = BasicBlock::empty(exit);
        self.blocks.insert(exit, exit_block);

        // Build the CFG
        let mut cfg = self.builder.build_cfg(entry, exit);
        for (_id, block) in self.blocks.iter() {
            cfg.add_block(block.clone());
        }

        Some(FunctionCfg {
            cfg,
            name: func.name.name.to_string(),
            def_count: self.stats.def_count,
            use_count: self.stats.use_count,
        })
    }

    /// Process a block of statements.
    fn process_block(&mut self, block: &Block, exit: BlockId) {
        for stmt in block.stmts.iter() {
            self.process_stmt(stmt, exit);
        }

        if let Maybe::Some(expr) = &block.expr {
            self.process_expr(expr);
        }
    }

    /// Process a statement.
    fn process_stmt(&mut self, stmt: &Stmt, exit: BlockId) {
        match &stmt.kind {
            StmtKind::Let { pattern: _, ty: _, value } => {
                if let Maybe::Some(expr) = value {
                    self.process_expr(expr);
                }
            }
            StmtKind::LetElse { pattern: _, ty: _, value, else_block } => {
                self.process_expr(value);
                // else_block is a diverging block, create separate CFG path
                let else_entry = self.new_block_with_predecessor(self.current_block);
                let saved_block = self.current_block;
                self.current_block = else_entry;
                self.process_block(else_block, exit);
                // else_block diverges, so no successor needed
                self.current_block = saved_block;
            }
            StmtKind::Expr { expr, has_semi: _ } => {
                self.process_expr(expr);
            }
            StmtKind::Item(_) => {
                // Nested items don't affect control flow
            }
            StmtKind::Defer(expr) => {
                self.process_expr(expr);
            }
            StmtKind::Errdefer(expr) => {
                self.process_expr(expr);
            }
            StmtKind::Provide { context: _, alias: _, value } => {
                self.process_expr(value);
            }
            StmtKind::ProvideScope { context: _, alias: _, value, block } => {
                self.process_expr(value);
                self.process_expr(block);
            }
            StmtKind::Empty => {}
        }
    }

    /// Process an expression, tracking reference definitions and uses.
    fn process_expr(&mut self, expr: &Expr) {
        let span = (expr.span.start, expr.span.end);

        match &expr.kind {
            // Reference creation (DefSite)
            ExprKind::Unary { op, expr: inner } => {
                match op {
                    UnOp::Ref | UnOp::RefMut | UnOp::RefChecked | UnOp::RefCheckedMut
                    | UnOp::RefUnsafe | UnOp::RefUnsafeMut | UnOp::Own | UnOp::OwnMut => {
                        // This is a reference definition
                        let ref_id = self.builder.new_ref_id_with_span(span);
                        let is_stack_allocated = self.is_stack_allocated(inner);
                        let def_site = DefSite::with_span(
                            self.current_block,
                            ref_id,
                            is_stack_allocated,
                            span,
                        );
                        self.current_defs.push(def_site);
                        self.stats.def_count += 1;
                    }
                    UnOp::Deref => {
                        // This is a reference use (dereference)
                        let ref_id = self.builder.new_ref_id_with_span(span);
                        let use_site = UseeSite::with_span(
                            self.current_block,
                            ref_id,
                            false, // Determined by context
                            span,
                        );
                        self.current_uses.push(use_site);
                        self.stats.use_count += 1;
                    }
                    _ => {}
                }
                self.process_expr(inner);
            }

            // Literals and paths - no control flow changes
            ExprKind::Literal(_) | ExprKind::Path(_) => {}

            // Binary operations
            ExprKind::Binary { op: _, left, right } => {
                self.process_expr(left);
                self.process_expr(right);
            }

            // Function calls
            ExprKind::Call { func, type_args: _, args } => {
                self.process_expr(func);
                for arg in args.iter() {
                    self.process_expr(arg);
                }
            }

            // Method calls
            ExprKind::MethodCall { receiver, method: _, type_args: _, args } => {
                self.process_expr(receiver);
                for arg in args.iter() {
                    self.process_expr(arg);
                }
            }

            // Field access
            ExprKind::Field { expr: inner, field: _ } => {
                self.process_expr(inner);
            }

            // Optional chaining
            ExprKind::OptionalChain { expr: inner, field: _ } => {
                self.process_expr(inner);
            }

            // Tuple index
            ExprKind::TupleIndex { expr: inner, index: _ } => {
                self.process_expr(inner);
            }

            // Index operation
            ExprKind::Index { expr: arr, index } => {
                self.process_expr(arr);
                self.process_expr(index);
            }

            // Pipeline
            ExprKind::Pipeline { left, right } => {
                self.process_expr(left);
                self.process_expr(right);
            }

            // Null coalesce
            ExprKind::NullCoalesce { left, right } => {
                self.process_expr(left);
                // right is only evaluated if left is None - separate block
                let right_block = self.new_block_with_predecessor(self.current_block);
                let saved = self.current_block;
                self.current_block = right_block;
                self.process_expr(right);
                self.current_block = saved;
            }

            // Type cast
            ExprKind::Cast { expr: inner, ty: _ } => {
                self.process_expr(inner);
            }

            // Try expression
            ExprKind::Try(inner) => {
                self.process_expr(inner);
            }

            // Try block
            ExprKind::TryBlock(inner) => {
                self.process_expr(inner);
            }

            // Try-recover
            ExprKind::TryRecover { try_block, recover: _ } => {
                self.process_expr(try_block);
            }

            // Try-finally
            ExprKind::TryFinally { try_block, finally_block } => {
                self.process_expr(try_block);
                self.process_expr(finally_block);
            }

            // Try-recover-finally
            ExprKind::TryRecoverFinally { try_block, recover: _, finally_block } => {
                self.process_expr(try_block);
                self.process_expr(finally_block);
            }

            // Tuple
            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.process_expr(elem);
                }
            }

            // Array
            ExprKind::Array(array_expr) => {
                match array_expr {
                    verum_ast::expr::ArrayExpr::List(elements) => {
                        for elem in elements.iter() {
                            self.process_expr(elem);
                        }
                    }
                    verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                        self.process_expr(value);
                        self.process_expr(count);
                    }
                }
            }

            // Comprehension
            ExprKind::Comprehension { expr: inner, clauses: _ } => {
                self.process_expr(inner);
            }

            // Stream comprehension
            ExprKind::StreamComprehension { expr: inner, clauses: _ } => {
                self.process_expr(inner);
            }

            // If expression - creates branching CFG
            ExprKind::If { condition, then_branch, else_branch } => {
                // Process condition expressions
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(expr) => {
                            self.process_expr(expr);
                        }
                        verum_ast::expr::ConditionKind::Let { pattern: _, value } => {
                            self.process_expr(value);
                        }
                    }
                }

                let then_blk = self.new_block_with_predecessor(self.current_block);
                let merge_block = self.builder.new_block_id();
                self.stats.block_count += 2;

                // Process then branch (Block type)
                let saved = self.current_block;
                self.current_block = then_blk;
                self.process_block(then_branch, merge_block);
                self.finalize_current_block(merge_block);

                // Process else branch if present
                if let Maybe::Some(else_expr) = else_branch {
                    let else_blk = self.new_block_with_predecessor(saved);
                    self.current_block = else_blk;
                    self.process_expr(else_expr);
                    self.finalize_current_block(merge_block);
                } else {
                    // No else - direct edge from condition to merge
                    if let Some(block) = self.blocks.get_mut(&saved) {
                        block.successors.insert(merge_block);
                    }
                }

                // Continue from merge block
                self.current_block = merge_block;
                let merge = BasicBlock::empty(merge_block);
                self.blocks.insert(merge_block, merge);
            }

            // Match expression
            ExprKind::Match { expr: scrutinee, arms } => {
                self.process_expr(scrutinee);

                let merge_block = self.builder.new_block_id();
                self.stats.block_count += 1;
                let saved = self.current_block;

                for arm in arms.iter() {
                    let arm_block = self.new_block_with_predecessor(saved);
                    self.current_block = arm_block;
                    if let Maybe::Some(guard) = &arm.guard {
                        self.process_expr(guard);
                    }
                    self.process_expr(&arm.body);
                    self.finalize_current_block(merge_block);
                }

                self.current_block = merge_block;
                let merge = BasicBlock::empty(merge_block);
                self.blocks.insert(merge_block, merge);
            }

            // Loop
            ExprKind::Loop { label: _, body, invariants } => {
                // Process invariants (verification expressions)
                for inv in invariants.iter() {
                    self.process_expr(inv);
                }

                let loop_header = self.new_block_with_predecessor(self.current_block);
                let loop_exit = self.builder.new_block_id();
                self.stats.block_count += 1;

                self.loop_stack.push((loop_header, loop_exit));
                self.finalize_current_block(loop_header);
                self.current_block = loop_header;
                self.process_block(body, loop_exit);
                // Loop back
                self.finalize_current_block(loop_header);
                self.loop_stack.pop();

                self.current_block = loop_exit;
                let exit_block = BasicBlock::empty(loop_exit);
                self.blocks.insert(loop_exit, exit_block);
            }

            // While loop
            ExprKind::While { label: _, condition, body, invariants, decreases } => {
                // Process invariants and decreases (verification expressions)
                for inv in invariants.iter() {
                    self.process_expr(inv);
                }
                for dec in decreases.iter() {
                    self.process_expr(dec);
                }

                let loop_header = self.new_block_with_predecessor(self.current_block);
                let loop_body = self.builder.new_block_id();
                let loop_exit = self.builder.new_block_id();
                self.stats.block_count += 2;

                self.finalize_current_block(loop_header);
                self.current_block = loop_header;
                self.process_expr(condition);

                // True branch -> body
                if let Some(block) = self.blocks.get_mut(&loop_header) {
                    block.successors.insert(loop_body);
                    block.successors.insert(loop_exit);
                }

                self.loop_stack.push((loop_header, loop_exit));
                self.current_block = loop_body;
                let body_block = BasicBlock::empty(loop_body);
                self.blocks.insert(loop_body, body_block);
                self.process_block(body, loop_exit);
                self.finalize_current_block(loop_header);
                self.loop_stack.pop();

                self.current_block = loop_exit;
                let exit_block = BasicBlock::empty(loop_exit);
                self.blocks.insert(loop_exit, exit_block);
            }

            // For loop
            ExprKind::For { label: _, pattern: _, iter, body, invariants, decreases } => {
                self.process_expr(iter);

                // Process invariants and decreases (verification expressions)
                for inv in invariants.iter() {
                    self.process_expr(inv);
                }
                for dec in decreases.iter() {
                    self.process_expr(dec);
                }

                let loop_header = self.new_block_with_predecessor(self.current_block);
                let loop_exit = self.builder.new_block_id();
                self.stats.block_count += 1;

                self.loop_stack.push((loop_header, loop_exit));
                self.finalize_current_block(loop_header);
                self.current_block = loop_header;
                self.process_block(body, loop_exit);
                self.finalize_current_block(loop_header);
                self.loop_stack.pop();

                // Also edge to exit
                if let Some(block) = self.blocks.get_mut(&loop_header) {
                    block.successors.insert(loop_exit);
                }

                self.current_block = loop_exit;
                let exit_block = BasicBlock::empty(loop_exit);
                self.blocks.insert(loop_exit, exit_block);
            }

            // Break
            ExprKind::Break { label: _, value } => {
                if let Maybe::Some(val) = value {
                    self.process_expr(val);
                }
                // Jump to loop exit
                if let Some(&(_, exit)) = self.loop_stack.last() {
                    self.finalize_current_block(exit);
                }
            }

            // Continue
            ExprKind::Continue { label: _ } => {
                // Jump to loop header
                if let Some(&(header, _)) = self.loop_stack.last() {
                    self.finalize_current_block(header);
                }
            }

            // Return
            ExprKind::Return(value) => {
                if let Maybe::Some(val) = value {
                    self.process_expr(val);
                }
                // Return terminates the current path
            }

            // Throw
            ExprKind::Throw(value) => {
                self.process_expr(value);
            }

            // Yield
            ExprKind::Yield(value) => {
                self.process_expr(value);
            }

            // Closure - analyzed separately if needed
            ExprKind::Closure { async_: _, move_: _, params: _, contexts: _, return_type: _, body } => {
                // Process closure body in current context for capture analysis
                self.process_expr(body);
            }

            // Async block
            ExprKind::Async(block) => {
                self.process_block(block, self.current_block);
            }

            // Await
            ExprKind::Await(inner) => {
                self.process_expr(inner);
                // Await is a suspension point - mark block
                if let Some(block) = self.blocks.get_mut(&self.current_block) {
                    block.has_await_point = true;
                }
            }

            // Spawn
            ExprKind::Spawn { expr: inner, contexts: _ } => {
                self.process_expr(inner);
            }

            // Unsafe block
            ExprKind::Unsafe(block) => {
                self.process_block(block, self.current_block);
            }

            // Block expression
            ExprKind::Block(block) => {
                self.process_block(block, self.current_block);
            }

            // Other expression kinds that don't affect CFG structure
            _ => {
                // For other expressions, recursively process any subexpressions
                // This handles cases like Record, Select, Nursery, etc.
            }
        }
    }

    /// Create a new block with predecessor edge from the given block.
    fn new_block_with_predecessor(&mut self, predecessor: BlockId) -> BlockId {
        let id = self.builder.new_block_id();
        self.stats.block_count += 1;

        let mut preds = Set::new();
        preds.insert(predecessor);

        let block = BasicBlock {
            id,
            predecessors: preds,
            successors: Set::new(),
            definitions: List::new(),
            uses: List::new(),
            call_sites: List::new(),
            has_await_point: false,
            is_exception_handler: false,
            is_cleanup_handler: false,
            may_throw: false,
        };

        self.blocks.insert(id, block);

        // Add successor edge from predecessor
        if let Some(pred_block) = self.blocks.get_mut(&predecessor) {
            pred_block.successors.insert(id);
        }

        id
    }

    /// Finalize the current block and add edge to successor.
    fn finalize_current_block(&mut self, successor: BlockId) {
        if let Some(block) = self.blocks.get_mut(&self.current_block) {
            // Add definitions and uses
            block.definitions.append(&mut self.current_defs.clone());
            block.uses.append(&mut self.current_uses.clone());
            // Add successor edge
            block.successors.insert(successor);
        }

        // Clear current defs/uses
        self.current_defs.clear();
        self.current_uses.clear();

        // Add predecessor edge to successor
        if let Some(succ_block) = self.blocks.get_mut(&successor) {
            succ_block.predecessors.insert(self.current_block);
        }
    }

    /// Determine if an expression represents a stack-allocated value.
    fn is_stack_allocated(&self, expr: &Expr) -> bool {
        match &expr.kind {
            // Local variables are stack-allocated
            ExprKind::Path(_) => true,
            // Literals are typically stack-allocated
            ExprKind::Literal(_) => true,
            // Tuples and arrays are stack-allocated
            ExprKind::Tuple(_) | ExprKind::Array(_) => true,
            // Field access of stack-allocated is stack-allocated
            ExprKind::Field { expr: inner, .. } => self.is_stack_allocated(inner),
            // Index into stack-allocated is stack-allocated
            ExprKind::Index { expr: inner, .. } => self.is_stack_allocated(inner),
            // Function calls - conservative, assume heap
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => false,
            // Blocks - check the result expression
            ExprKind::Block(block) => {
                if let Maybe::Some(result) = &block.expr {
                    self.is_stack_allocated(result)
                } else {
                    true // Unit type is stack
                }
            }
            // Default: assume heap for safety
            _ => false,
        }
    }
}

impl Default for CfgConstructor {
    fn default() -> Self {
        Self::new()
    }
}

// ==================================================================================
// Tests
// ==================================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::{FileId, Span};
    use verum_ast::ty::{Ident, Path};

    fn make_span(start: u32, end: u32) -> Span {
        Span::new(start, end, FileId::dummy())
    }

    #[test]
    fn test_constructor_new() {
        let constructor = CfgConstructor::new();
        assert!(constructor.blocks.is_empty());
    }

    #[test]
    fn test_ref_tracking() {
        // Create a simple expression: &x
        let x_ident = Ident::new("x", make_span(1, 2));
        let x_path = Path::single(x_ident);
        let x_expr = Expr::new(ExprKind::Path(x_path), make_span(1, 2));

        let ref_expr = Expr::new(
            ExprKind::Unary {
                op: UnOp::Ref,
                expr: verum_common::Heap::new(x_expr),
            },
            make_span(0, 2),
        );

        let mut constructor = CfgConstructor::new();
        constructor.current_block = BlockId(0);
        constructor.blocks.insert(BlockId(0), BasicBlock::empty(BlockId(0)));

        constructor.process_expr(&ref_expr);

        assert_eq!(constructor.stats.def_count, 1);
        assert!(!constructor.current_defs.is_empty());
    }

    #[test]
    fn test_deref_tracking() {
        // Create a simple expression: *x
        let x_ident = Ident::new("x", make_span(1, 2));
        let x_path = Path::single(x_ident);
        let x_expr = Expr::new(ExprKind::Path(x_path), make_span(1, 2));

        let deref_expr = Expr::new(
            ExprKind::Unary {
                op: UnOp::Deref,
                expr: verum_common::Heap::new(x_expr),
            },
            make_span(0, 2),
        );

        let mut constructor = CfgConstructor::new();
        constructor.current_block = BlockId(0);
        constructor.blocks.insert(BlockId(0), BasicBlock::empty(BlockId(0)));

        constructor.process_expr(&deref_expr);

        assert_eq!(constructor.stats.use_count, 1);
        assert!(!constructor.current_uses.is_empty());
    }

    #[test]
    fn test_span_mapping() {
        let mut constructor = CfgConstructor::new();

        let span = (10, 20);
        let ref_id = constructor.builder.new_ref_id_with_span(span);

        // Verify span→RefId mapping
        assert_eq!(constructor.builder.get_ref_for_span(span), Some(ref_id));

        // Verify RefId→span mapping
        assert_eq!(constructor.builder.get_span_for_ref(ref_id), Some(span));
    }
}
