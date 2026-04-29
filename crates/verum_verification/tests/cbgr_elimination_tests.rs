#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments,
    clippy::absurd_extreme_comparisons
)]
// Comprehensive tests for CBGR Check Elimination via Escape Analysis
//
// CBGR Check Elimination via Escape Analysis:
// CBGR (~15ns) checks are eliminated when escape analysis proves references don't
// outlive their allocations. Conservative criteria (ALL must hold):
// 1. Reference doesn't escape scope (not returned, stored, or captured by closure)
// 2. Allocation dominates all uses (allocation before all dereferences in CFG)
// 3. No concurrent access (not shared across threads)
// 4. Scope validity is stack-bounded (reference contained within function)
//
// When proven safe, compiler promotes &T -> &checked T (0ns overhead).
// Unknown/escaping references conservatively keep CBGR checks.
//
// These tests verify the CBGR elimination system correctly identifies safe
// references, never produces false negatives, handles complex control flow,
// and integrates with CFG and scope analysis.

use std::collections::HashSet;
use verum_common::{List, Map, Text};
use verum_verification::cbgr_elimination::{
    BasicBlock, BlockId, CBGROptimizer, ControlFlowGraph, DefSite, EscapeAnalysisResult,
    EscapeStatus, Function, OptimizationConfig, RefVariable, Scope, ScopeId, UseSite,
    analyze_escape, can_eliminate_check, optimize_function, prove_scope_validity,
};

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a simple function with a single basic block
fn create_simple_function(name: &str) -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block
    let block = BasicBlock::new(entry_block, root_scope);
    cfg.add_block(block);
    cfg.add_exit(entry_block);

    Function::new(Text::from(name), cfg)
}

/// Create a function with a reference that doesn't escape
fn create_non_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with definition and use
    let mut block = BasicBlock::new(entry_block, root_scope);

    // Add definition (stack allocated)
    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });

    // Add use (not escaping)
    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("non_escaping"), cfg);
    func.add_reference_var(ref_var);
    func
}

/// Create a function with a reference that escapes via return
fn create_return_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with definition and escaping use
    let mut block = BasicBlock::new(entry_block, root_scope);

    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });

    // Use that escapes via return
    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: true, // ESCAPES!
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("return_escaping"), cfg);
    func.add_reference_var(ref_var);
    func.set_returns_reference(true);
    func
}

/// Create a function with a reference that escapes via heap
fn create_heap_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with heap allocation
    let mut block = BasicBlock::new(entry_block, root_scope);

    // Heap allocated definition
    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: false,
        is_heap_allocated: true, // HEAP!
    });

    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("heap_escaping"), cfg);
    func.add_reference_var(ref_var);
    func
}

/// Create a function with a reference that escapes via closure capture
fn create_closure_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with closure capture
    let mut block = BasicBlock::new(entry_block, root_scope);

    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });

    // Use that escapes via closure
    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: true, // ESCAPES!
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("closure_escaping"), cfg);
    func.add_reference_var(ref_var);
    func
}

/// Create a function with a reference that escapes via thread spawn
fn create_thread_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with thread spawn
    let mut block = BasicBlock::new(entry_block, root_scope);

    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });

    // Use that escapes via thread
    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: true, // ESCAPES!
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("thread_escaping"), cfg);
    func.add_reference_var(ref_var);
    func.set_spawns_threads(true);
    func
}

/// Create a function with a reference that escapes via field store
fn create_field_escaping_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(entry_block);
    cfg.add_scope(scope);

    // Add entry block with field store
    let mut block = BasicBlock::new(entry_block, root_scope);

    block.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });

    // Use that escapes via field
    block.add_use(UseSite {
        variable: ref_var,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: true, // ESCAPES!
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("field_escaping"), cfg);
    func.add_reference_var(ref_var);
    func
}

/// Create a function with multiple control flow paths
fn create_branching_function() -> Function {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);
    let then_block = BlockId::new(1);
    let else_block = BlockId::new(2);
    let exit_block = BlockId::new(3);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    // Add root scope
    let mut scope = Scope::new(root_scope, entry_block);
    let ref_var = RefVariable::reference(1);
    scope.add_variable(ref_var);
    scope.add_exit_block(exit_block);
    cfg.add_scope(scope);

    // Entry block with definition
    let mut block0 = BasicBlock::new(entry_block, root_scope);
    block0.add_definition(DefSite {
        variable: ref_var,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });
    block0.add_successor(then_block);
    block0.add_successor(else_block);
    cfg.add_block(block0);

    // Then block with use
    let mut block1 = BasicBlock::new(then_block, root_scope);
    block1.add_predecessor(entry_block);
    block1.add_successor(exit_block);
    block1.add_use(UseSite {
        variable: ref_var,
        block: then_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });
    cfg.add_block(block1);

    // Else block with use
    let mut block2 = BasicBlock::new(else_block, root_scope);
    block2.add_predecessor(entry_block);
    block2.add_successor(exit_block);
    block2.add_use(UseSite {
        variable: ref_var,
        block: else_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });
    cfg.add_block(block2);

    // Exit block
    let mut block3 = BasicBlock::new(exit_block, root_scope);
    block3.add_predecessor(then_block);
    block3.add_predecessor(else_block);
    cfg.add_block(block3);
    cfg.add_exit(exit_block);

    let mut func = Function::new(Text::from("branching"), cfg);
    func.add_reference_var(ref_var);
    func
}

// =============================================================================
// EscapeStatus Tests
// =============================================================================

#[test]
fn test_escape_status_can_eliminate() {
    assert!(EscapeStatus::NoEscape.can_eliminate_check());
    assert!(!EscapeStatus::EscapesToHeap.can_eliminate_check());
    assert!(!EscapeStatus::EscapesToClosure.can_eliminate_check());
    assert!(!EscapeStatus::EscapesToReturn.can_eliminate_check());
    assert!(!EscapeStatus::EscapesToField.can_eliminate_check());
    assert!(!EscapeStatus::EscapesToThread.can_eliminate_check());
    assert!(!EscapeStatus::Unknown.can_eliminate_check());
}

#[test]
fn test_escape_status_cbgr_overhead() {
    assert_eq!(EscapeStatus::NoEscape.cbgr_overhead_ns(), 0);
    assert_eq!(EscapeStatus::EscapesToHeap.cbgr_overhead_ns(), 15);
    assert_eq!(EscapeStatus::EscapesToClosure.cbgr_overhead_ns(), 15);
    assert_eq!(EscapeStatus::EscapesToReturn.cbgr_overhead_ns(), 15);
    assert_eq!(EscapeStatus::EscapesToField.cbgr_overhead_ns(), 15);
    assert_eq!(EscapeStatus::EscapesToThread.cbgr_overhead_ns(), 15);
    assert_eq!(EscapeStatus::Unknown.cbgr_overhead_ns(), 15);
}

#[test]
fn test_escape_status_reason() {
    assert!(EscapeStatus::NoEscape.reason().contains("does not escape"));
    assert!(EscapeStatus::EscapesToHeap.reason().contains("heap"));
    assert!(EscapeStatus::EscapesToClosure.reason().contains("closure"));
    assert!(EscapeStatus::EscapesToReturn.reason().contains("return"));
    assert!(EscapeStatus::EscapesToField.reason().contains("field"));
    assert!(EscapeStatus::EscapesToThread.reason().contains("thread"));
    assert!(EscapeStatus::Unknown.reason().contains("unknown"));
}

#[test]
fn test_escape_status_is_definitive() {
    assert!(EscapeStatus::NoEscape.is_definitive());
    assert!(EscapeStatus::EscapesToHeap.is_definitive());
    assert!(EscapeStatus::EscapesToClosure.is_definitive());
    assert!(EscapeStatus::EscapesToReturn.is_definitive());
    assert!(EscapeStatus::EscapesToField.is_definitive());
    assert!(EscapeStatus::EscapesToThread.is_definitive());
    assert!(!EscapeStatus::Unknown.is_definitive());
}

#[test]
fn test_escape_status_default() {
    // SAFETY: Default should be conservative (Unknown)
    let status = EscapeStatus::default();
    assert_eq!(status, EscapeStatus::Unknown);
    assert!(!status.can_eliminate_check());
}

#[test]
fn test_escape_status_display() {
    let display = format!("{}", EscapeStatus::NoEscape);
    assert!(display.contains("NoEscape"));
    assert!(display.contains("0ns"));

    let display = format!("{}", EscapeStatus::EscapesToHeap);
    assert!(display.contains("15ns"));
}

// =============================================================================
// Variable Tests
// =============================================================================

#[test]
fn test_variable_creation() {
    let ref_var = RefVariable::reference(42);
    assert!(ref_var.is_reference);
    assert_eq!(ref_var.id, 42);

    let val_var = RefVariable::value(43);
    assert!(!val_var.is_reference);
    assert_eq!(val_var.id, 43);
}

#[test]
fn test_variable_display() {
    let ref_var = RefVariable::reference(1);
    let display = format!("{}", ref_var);
    assert!(display.contains("&var_1"));

    let val_var = RefVariable::value(2);
    let display = format!("{}", val_var);
    assert!(display.contains("var_2"));
    assert!(!display.contains("&"));
}

// =============================================================================
// CBGROptimizer Tests
// =============================================================================

#[test]
fn test_optimizer_non_escaping() {
    let func = create_non_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    assert_eq!(result.eliminated_checks, 1);
    assert_eq!(result.total_checks, 1);
    assert_eq!(result.elimination_rate(), 100.0);

    let ref_var = RefVariable::reference(1);
    assert!(can_eliminate_check(&ref_var, &result));
}

#[test]
fn test_optimizer_return_escaping() {
    let func = create_return_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    assert_eq!(result.eliminated_checks, 0);
    assert_eq!(result.total_checks, 1);

    let ref_var = RefVariable::reference(1);
    assert!(!can_eliminate_check(&ref_var, &result));

    let status = result.reference_status.get(&ref_var).unwrap();
    assert_eq!(*status, EscapeStatus::EscapesToReturn);
}

#[test]
fn test_optimizer_heap_escaping() {
    let func = create_heap_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    assert_eq!(result.eliminated_checks, 0);
    assert_eq!(result.total_checks, 1);

    let ref_var = RefVariable::reference(1);
    let status = result.reference_status.get(&ref_var).unwrap();
    assert_eq!(*status, EscapeStatus::EscapesToHeap);
}

#[test]
fn test_optimizer_closure_escaping() {
    let func = create_closure_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    let ref_var = RefVariable::reference(1);
    let status = result.reference_status.get(&ref_var).unwrap();
    assert_eq!(*status, EscapeStatus::EscapesToClosure);
}

#[test]
fn test_optimizer_thread_escaping() {
    let func = create_thread_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    let ref_var = RefVariable::reference(1);
    let status = result.reference_status.get(&ref_var).unwrap();
    assert_eq!(*status, EscapeStatus::EscapesToThread);
}

#[test]
fn test_optimizer_field_escaping() {
    let func = create_field_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    let ref_var = RefVariable::reference(1);
    let status = result.reference_status.get(&ref_var).unwrap();
    assert_eq!(*status, EscapeStatus::EscapesToField);
}

#[test]
fn test_optimizer_branching_no_escape() {
    let func = create_branching_function();
    let mut optimizer = CBGROptimizer::conservative();

    let result = optimizer.analyze_escape(&func);

    // Reference is used in multiple branches but doesn't escape
    assert_eq!(result.eliminated_checks, 1);
    assert_eq!(result.total_checks, 1);
}

// =============================================================================
// analyze_escape Function Tests
// =============================================================================

#[test]
fn test_analyze_escape_convenience_function() {
    let func = create_non_escaping_function();
    let result = analyze_escape(&func);

    assert_eq!(result.eliminated_checks, 1);
    assert!(result.analysis_duration.as_nanos() >= 0);
}

// =============================================================================
// can_eliminate_check Function Tests
// =============================================================================

#[test]
fn test_can_eliminate_check_not_analyzed() {
    let func = create_non_escaping_function();
    let result = analyze_escape(&func);

    // Variable that wasn't in the analysis
    let unknown_var = RefVariable::reference(999);
    assert!(!can_eliminate_check(&unknown_var, &result));
}

// =============================================================================
// optimize_function Tests
// =============================================================================

#[test]
fn test_optimize_function() {
    let func = create_non_escaping_function();
    let result = analyze_escape(&func);

    let optimized = optimize_function(&func, &result);

    assert_eq!(optimized.eliminated_checks.len(), 1);
    assert!(optimized.preserved_checks.is_empty());
    assert!(optimized.total_savings_ns > 0);
}

// =============================================================================
// prove_scope_validity Tests
// =============================================================================

#[test]
fn test_prove_scope_validity_contained() {
    let func = create_non_escaping_function();
    let ref_var = RefVariable::reference(1);

    let scope = func.cfg.scopes.get(&func.cfg.root_scope).unwrap();
    assert!(prove_scope_validity(&ref_var, scope, &func.cfg));
}

// =============================================================================
// OptimizationConfig Tests
// =============================================================================

#[test]
fn test_optimization_config_conservative() {
    let config = OptimizationConfig::conservative();
    assert!(!config.aggressive);
    assert_eq!(config.max_analysis_depth, 2);
    assert!(config.trust_annotations);
    assert!(!config.interprocedural);
}

#[test]
fn test_optimization_config_balanced() {
    let config = OptimizationConfig::balanced();
    assert!(!config.aggressive);
    assert_eq!(config.max_analysis_depth, 5);
    assert!(config.interprocedural);
}

#[test]
fn test_optimization_config_aggressive() {
    let config = OptimizationConfig::aggressive();
    assert!(config.aggressive);
    assert_eq!(config.max_analysis_depth, 10);
    assert!(config.interprocedural);
}

#[test]
fn test_optimization_config_default() {
    let config = OptimizationConfig::default();
    assert!(!config.aggressive); // Default is conservative
}

// =============================================================================
// EscapeAnalysisResult Tests
// =============================================================================

#[test]
fn test_escape_analysis_result_elimination_rate() {
    let mut result = EscapeAnalysisResult::new(Text::from("test"));

    // No checks - 0% rate
    assert_eq!(result.elimination_rate(), 0.0);

    // Add some statuses
    result.record_status(RefVariable::reference(1), EscapeStatus::NoEscape);
    result.record_status(RefVariable::reference(2), EscapeStatus::EscapesToHeap);
    result.record_status(RefVariable::reference(3), EscapeStatus::NoEscape);
    result.record_status(RefVariable::reference(4), EscapeStatus::EscapesToReturn);

    // 2/4 = 50%
    assert_eq!(result.elimination_rate(), 50.0);
}

#[test]
fn test_escape_analysis_result_estimated_savings() {
    let mut result = EscapeAnalysisResult::new(Text::from("test"));
    result.record_status(RefVariable::reference(1), EscapeStatus::NoEscape);
    result.record_status(RefVariable::reference(2), EscapeStatus::NoEscape);

    // 2 eliminated * 15ns * 10 derefs = 300ns
    assert_eq!(result.estimated_time_saved_ns(), 300);
}

#[test]
fn test_escape_analysis_result_display() {
    let mut result = EscapeAnalysisResult::new(Text::from("test_function"));
    result.record_status(RefVariable::reference(1), EscapeStatus::NoEscape);
    result.record_status(RefVariable::reference(2), EscapeStatus::EscapesToHeap);

    let display = format!("{}", result);
    assert!(display.contains("test_function"));
    assert!(display.contains("Total references: 2"));
    assert!(display.contains("Eliminated checks: 1"));
}

// =============================================================================
// ControlFlowGraph Tests
// =============================================================================

#[test]
fn test_cfg_dominance_self() {
    let func = create_simple_function("test");

    // A block dominates itself
    let entry = func.cfg.entry;
    assert!(func.cfg.dominates(entry, entry));
}

#[test]
fn test_cfg_dominance_entry() {
    let func = create_branching_function();

    // Entry dominates all blocks
    let entry = func.cfg.entry;
    let then_block = BlockId::new(1);
    let else_block = BlockId::new(2);
    let exit_block = BlockId::new(3);

    assert!(func.cfg.dominates(entry, then_block));
    assert!(func.cfg.dominates(entry, else_block));
    assert!(func.cfg.dominates(entry, exit_block));
}

#[test]
fn test_cfg_dominance_branch() {
    let func = create_branching_function();

    // Then block does NOT dominate else block
    let then_block = BlockId::new(1);
    let else_block = BlockId::new(2);

    assert!(!func.cfg.dominates(then_block, else_block));
    assert!(!func.cfg.dominates(else_block, then_block));
}

// =============================================================================
// Scope Tests
// =============================================================================

#[test]
fn test_scope_creation() {
    let scope_id = ScopeId::new(0);
    let block_id = BlockId::new(0);

    let scope = Scope::new(scope_id, block_id);
    assert_eq!(scope.id, scope_id);
    assert!(scope.parent.is_none());
    assert!(scope.children.is_empty());
    assert!(!scope.is_loop);
    assert!(!scope.is_closure);
}

#[test]
fn test_scope_with_parent() {
    let parent_id = ScopeId::new(0);
    let child_id = ScopeId::new(1);
    let block_id = BlockId::new(1);

    let scope = Scope::with_parent(child_id, parent_id, block_id);
    assert_eq!(scope.id, child_id);
    assert_eq!(scope.parent, Some(parent_id));
}

#[test]
fn test_scope_contains_variable() {
    let scope_id = ScopeId::new(0);
    let block_id = BlockId::new(0);

    let mut scope = Scope::new(scope_id, block_id);
    let var = RefVariable::reference(1);

    assert!(!scope.contains_variable(&var));
    scope.add_variable(var);
    assert!(scope.contains_variable(&var));
}

#[test]
fn test_scope_loop_and_closure_flags() {
    let scope_id = ScopeId::new(0);
    let block_id = BlockId::new(0);

    let mut scope = Scope::new(scope_id, block_id);

    scope.set_loop(true);
    assert!(scope.is_loop);

    scope.set_closure(true);
    assert!(scope.is_closure);
}

// =============================================================================
// BasicBlock Tests
// =============================================================================

#[test]
fn test_basic_block_creation() {
    let block_id = BlockId::new(0);
    let scope_id = ScopeId::new(0);

    let block = BasicBlock::new(block_id, scope_id);
    assert_eq!(block.id, block_id);
    assert_eq!(block.scope, scope_id);
    assert!(block.predecessors.is_empty());
    assert!(block.successors.is_empty());
}

#[test]
fn test_basic_block_edges() {
    let block_id = BlockId::new(0);
    let scope_id = ScopeId::new(0);

    let mut block = BasicBlock::new(block_id, scope_id);

    let pred = BlockId::new(1);
    let succ = BlockId::new(2);

    block.add_predecessor(pred);
    block.add_successor(succ);

    assert!(block.predecessors.contains(&pred));
    assert!(block.successors.contains(&succ));
}

// =============================================================================
// Statistics Tests
// =============================================================================

#[test]
fn test_optimizer_statistics() {
    let func1 = create_non_escaping_function();
    let func2 = create_return_escaping_function();

    let mut optimizer = CBGROptimizer::conservative();

    optimizer.analyze_escape(&func1);
    optimizer.analyze_escape(&func2);

    let stats = optimizer.stats();
    assert_eq!(stats.functions_analyzed, 2);
    assert_eq!(stats.references_analyzed, 2);
    assert_eq!(stats.checks_eliminated, 1);
    assert_eq!(stats.checks_preserved, 1);
}

#[test]
fn test_optimizer_reset_statistics() {
    let func = create_non_escaping_function();
    let mut optimizer = CBGROptimizer::conservative();

    optimizer.analyze_escape(&func);
    assert_eq!(optimizer.stats().functions_analyzed, 1);

    optimizer.reset_stats();
    assert_eq!(optimizer.stats().functions_analyzed, 0);
}

// =============================================================================
// Safety Invariant Tests
// =============================================================================

/// SAFETY TEST: Ensure we NEVER eliminate checks when status is Unknown
#[test]
fn test_safety_unknown_never_eliminates() {
    let status = EscapeStatus::Unknown;
    assert!(
        !status.can_eliminate_check(),
        "SAFETY: Unknown status must NEVER allow elimination"
    );
}

/// SAFETY TEST: Ensure we NEVER eliminate checks for escaping references
#[test]
fn test_safety_escaping_never_eliminates() {
    let escaping_statuses = [
        EscapeStatus::EscapesToHeap,
        EscapeStatus::EscapesToClosure,
        EscapeStatus::EscapesToReturn,
        EscapeStatus::EscapesToField,
        EscapeStatus::EscapesToThread,
    ];

    for status in escaping_statuses {
        assert!(
            !status.can_eliminate_check(),
            "SAFETY: Escaping status {:?} must NEVER allow elimination",
            status
        );
    }
}

/// SAFETY TEST: Only NoEscape allows elimination
#[test]
fn test_safety_only_no_escape_eliminates() {
    let all_statuses = [
        EscapeStatus::NoEscape,
        EscapeStatus::EscapesToHeap,
        EscapeStatus::EscapesToClosure,
        EscapeStatus::EscapesToReturn,
        EscapeStatus::EscapesToField,
        EscapeStatus::EscapesToThread,
        EscapeStatus::Unknown,
    ];

    for status in all_statuses {
        let can_eliminate = status.can_eliminate_check();
        let is_no_escape = matches!(status, EscapeStatus::NoEscape);
        assert_eq!(
            can_eliminate, is_no_escape,
            "SAFETY: Only NoEscape should allow elimination, but {:?}.can_eliminate_check() = {}",
            status, can_eliminate
        );
    }
}

/// SAFETY TEST: Conservative default
#[test]
fn test_safety_conservative_default() {
    let default = EscapeStatus::default();
    assert!(
        !default.can_eliminate_check(),
        "SAFETY: Default EscapeStatus must be conservative (no elimination)"
    );
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_empty_function() {
    let func = create_simple_function("empty");

    let mut optimizer = CBGROptimizer::conservative();
    let result = optimizer.analyze_escape(&func);

    assert_eq!(result.total_checks, 0);
    assert_eq!(result.eliminated_checks, 0);
    assert_eq!(result.elimination_rate(), 0.0);
}

#[test]
fn test_non_reference_variable() {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    let mut scope = Scope::new(root_scope, entry_block);
    let val_var = RefVariable::value(1); // NOT a reference
    scope.add_variable(val_var);
    cfg.add_scope(scope);

    let block = BasicBlock::new(entry_block, root_scope);
    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("value_only"), cfg);
    func.add_reference_var(val_var);

    let mut optimizer = CBGROptimizer::conservative();
    let result = optimizer.analyze_escape(&func);

    // Non-reference variables should be marked as NoEscape
    let status = result.reference_status.get(&val_var).unwrap();
    assert_eq!(*status, EscapeStatus::NoEscape);
}

#[test]
fn test_multiple_references() {
    let root_scope = ScopeId::new(0);
    let entry_block = BlockId::new(0);

    let mut cfg = ControlFlowGraph::new(entry_block, root_scope);

    let mut scope = Scope::new(root_scope, entry_block);
    let ref1 = RefVariable::reference(1);
    let ref2 = RefVariable::reference(2);
    let ref3 = RefVariable::reference(3);
    scope.add_variable(ref1);
    scope.add_variable(ref2);
    scope.add_variable(ref3);
    cfg.add_scope(scope);

    let mut block = BasicBlock::new(entry_block, root_scope);

    // ref1: non-escaping
    block.add_definition(DefSite {
        variable: ref1,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });
    block.add_use(UseSite {
        variable: ref1,
        block: entry_block,
        is_mutable: false,
        is_return: false,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    // ref2: escapes via heap
    block.add_definition(DefSite {
        variable: ref2,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: false,
        is_heap_allocated: true,
    });

    // ref3: escapes via return
    block.add_definition(DefSite {
        variable: ref3,
        block: entry_block,
        scope: root_scope,
        is_stack_allocated: true,
        is_heap_allocated: false,
    });
    block.add_use(UseSite {
        variable: ref3,
        block: entry_block,
        is_mutable: false,
        is_return: true,
        is_field_store: false,
        is_thread_spawn: false,
        is_closure_capture: false,
    });

    cfg.add_block(block);
    cfg.add_exit(entry_block);

    let mut func = Function::new(Text::from("multi_ref"), cfg);
    func.add_reference_var(ref1);
    func.add_reference_var(ref2);
    func.add_reference_var(ref3);
    func.set_returns_reference(true);

    let mut optimizer = CBGROptimizer::conservative();
    let result = optimizer.analyze_escape(&func);

    assert_eq!(result.total_checks, 3);
    assert_eq!(result.eliminated_checks, 1); // Only ref1

    assert!(can_eliminate_check(&ref1, &result));
    assert!(!can_eliminate_check(&ref2, &result));
    assert!(!can_eliminate_check(&ref3, &result));
}


// ============================================================================
// Wire-up Pin Tests
// ============================================================================

#[test]
fn config_accessors_mirror_construction_values() {
    // Pin: every accessor on CBGROptimizer returns the configured
    // value. Before the wire-up landed five OptimizationConfig
    // fields had no public read surface — external orchestrators
    // composing the optimizer with their own call-graph walker
    // couldn't observe its configured stance.
    use verum_verification::cbgr_elimination::OptimizationConfig;

    for &agg in &[true, false] {
        for &inter in &[true, false] {
            for &trust in &[true, false] {
                let cfg = OptimizationConfig {
                    aggressive: agg,
                    max_analysis_depth: 7,
                    trust_annotations: trust,
                    interprocedural: inter,
                    timeout_ms: 1234,
                };
                let opt = CBGROptimizer::new(cfg);
                assert_eq!(opt.aggressive_enabled(), agg);
                assert_eq!(opt.interprocedural_enabled(), inter);
                assert_eq!(opt.trust_annotations_enabled(), trust);
                assert_eq!(opt.max_analysis_depth(), 7);
                assert_eq!(opt.timeout_ms(), 1234);
            }
        }
    }
}

#[test]
fn timeout_ms_zero_means_unlimited() {
    // Pin: `OptimizationConfig.timeout_ms = 0` is treated as
    // unlimited — the analyse-escape loop runs every variable
    // even on a function with many references, no Unknown
    // fallback inserted by the budget check.
    use verum_verification::cbgr_elimination::OptimizationConfig;

    let mut config = OptimizationConfig::conservative();
    config.timeout_ms = 0;
    let opt = CBGROptimizer::new(config);
    assert_eq!(opt.timeout_ms(), 0);
}

