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
    unused_assignments
)]
//! Tests for Flow Functions - Per-Field Interprocedural Analysis
//!
//! Validates flow functions for per-field interprocedural CBGR escape analysis.
//! Flow functions model escape propagation through CFG edges on a per-field
//! basis. Per-edge flow function target: <100ns. Per-call interprocedural: <500ns.
//! Whole-function analysis: <5ms. Complexity: O(edges * fields).
//!
//! This test suite validates the flow function implementation for
//! field-sensitive interprocedural dataflow analysis.

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, EscapeAnalyzer, RefId};
use verum_cbgr::flow_functions::*;
use verum_common::{List, Map, Maybe, Set, Text};

// ==================================================================================
// Test Utilities
// ==================================================================================

fn create_simple_cfg() -> ControlFlowGraph {
    let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(2));

    // Block 0: Entry
    let block0 = BasicBlock {
        id: BlockId(0),
        predecessors: Set::new(),
        successors: {
            let mut s = Set::new();
            s.insert(BlockId(1));
            s
        },
        definitions: List::new(),
        uses: List::new(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    // Block 1: Middle
    let block1 = BasicBlock {
        id: BlockId(1),
        predecessors: {
            let mut s = Set::new();
            s.insert(BlockId(0));
            s
        },
        successors: {
            let mut s = Set::new();
            s.insert(BlockId(2));
            s
        },
        definitions: List::new(),
        uses: List::new(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    // Block 2: Exit
    let block2 = BasicBlock {
        id: BlockId(2),
        predecessors: {
            let mut s = Set::new();
            s.insert(BlockId(1));
            s
        },
        successors: Set::new(),
        definitions: List::new(),
        uses: List::new(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(block0);
    cfg.add_block(block1);
    cfg.add_block(block2);

    cfg
}

// ==================================================================================
// Section 1: FieldPath Tests (5 tests)
// ==================================================================================

#[test]
fn test_field_path_creation() {
    let path = FieldPath::from_field(Text::from("x"));
    assert_eq!(path.depth(), 1);
    assert!(!path.is_root());
    assert_eq!(path.to_string(), "x");
}

#[test]
fn test_field_path_root() {
    let root = FieldPath::root();
    assert_eq!(root.depth(), 0);
    assert!(root.is_root());
    assert_eq!(root.to_string(), "<root>");
}

#[test]
fn test_field_path_extend() {
    let path = FieldPath::from_field(Text::from("x"));
    let extended = path.extend(Text::from("y"));

    assert_eq!(extended.depth(), 2);
    assert!(!extended.is_root());
}

#[test]
fn test_field_path_is_prefix() {
    let short = FieldPath::from_field(Text::from("x"));
    let long = short.clone().extend(Text::from("y"));

    assert!(short.is_prefix_of(&long));
    assert!(!long.is_prefix_of(&short));
    assert!(short.is_prefix_of(&short)); // Self is prefix
}

#[test]
fn test_field_path_nested() {
    let path = FieldPath::from_field(Text::from("a"))
        .extend(Text::from("b"))
        .extend(Text::from("c"));

    assert_eq!(path.depth(), 3);
    assert_eq!(path.components.len(), 3);
}

// ==================================================================================
// Section 2: FieldFlowInfo Tests (5 tests)
// ==================================================================================

#[test]
fn test_field_flow_info_creation() {
    let info = FieldFlowInfo::new(RefId(1));
    assert_eq!(info.reference, RefId(1));
    assert!(!info.conservative);
    assert_eq!(info.safe_field_count(), 0);
}

#[test]
fn test_field_flow_info_conservative() {
    let info = FieldFlowInfo::conservative(RefId(1));
    assert!(info.conservative);

    let path = FieldPath::from_field(Text::from("x"));
    assert!(!info.is_field_safe(&path));
}

#[test]
fn test_field_flow_info_set_field() {
    let mut info = FieldFlowInfo::new(RefId(1));
    let path = FieldPath::from_field(Text::from("x"));

    info.set_field(path.clone(), true);
    assert!(info.is_field_safe(&path));
    assert_eq!(info.safe_field_count(), 1);
}

#[test]
fn test_field_flow_info_mark_all_unsafe() {
    let mut info = FieldFlowInfo::new(RefId(1));
    let path = FieldPath::from_field(Text::from("x"));

    info.set_field(path.clone(), true);
    assert_eq!(info.safe_field_count(), 1);

    info.mark_all_unsafe();
    assert!(!info.is_field_safe(&path));
    assert_eq!(info.safe_field_count(), 0);
}

#[test]
fn test_field_flow_info_merge() {
    let mut info1 = FieldFlowInfo::new(RefId(1));
    let mut info2 = FieldFlowInfo::new(RefId(1));

    let path_x = FieldPath::from_field(Text::from("x"));
    let path_y = FieldPath::from_field(Text::from("y"));

    info1.set_field(path_x.clone(), true);
    info1.set_field(path_y.clone(), true);

    info2.set_field(path_x.clone(), true);
    info2.set_field(path_y.clone(), false);

    let merged = info1.merge(&info2);

    // Conservative merge: only fields safe in both
    assert!(merged.is_field_safe(&path_x));
    assert!(!merged.is_field_safe(&path_y));
}

// ==================================================================================
// Section 3: FlowState Tests (5 tests)
// ==================================================================================

#[test]
fn test_flow_state_creation() {
    let state = FlowState::new();
    assert!(!state.conservative);
    assert!(state.is_empty());
}

#[test]
fn test_flow_state_conservative() {
    let state = FlowState::conservative();
    assert!(state.conservative);

    let path = FieldPath::from_field(Text::from("x"));
    assert!(!state.is_field_safe(RefId(1), &path));
}

#[test]
fn test_flow_state_set_field_safe() {
    let mut state = FlowState::new();
    let ref_id = RefId(1);
    let path = FieldPath::from_field(Text::from("x"));

    state.set_field_safe(ref_id, path.clone(), true);
    assert!(state.is_field_safe(ref_id, &path));
    assert!(!state.is_empty());
}

#[test]
fn test_flow_state_mark_reference_unsafe() {
    let mut state = FlowState::new();
    let ref_id = RefId(1);
    let path = FieldPath::from_field(Text::from("x"));

    state.set_field_safe(ref_id, path.clone(), true);
    state.mark_reference_unsafe(ref_id);

    assert!(!state.is_field_safe(ref_id, &path));
}

#[test]
fn test_flow_state_merge() {
    let mut state1 = FlowState::new();
    let mut state2 = FlowState::new();

    let ref_id = RefId(1);
    let path = FieldPath::from_field(Text::from("x"));

    state1.set_field_safe(ref_id, path.clone(), true);
    state2.set_field_safe(ref_id, path.clone(), false);

    let merged = state1.merge(&state2);

    // Conservative merge
    assert!(!merged.is_field_safe(ref_id, &path));
}

// ==================================================================================
// Section 4: IrOperation Tests (5 tests)
// ==================================================================================

#[test]
fn test_ir_operation_load() {
    let op = IrOperation::Load {
        dest: SsaId(1),
        src: SsaId(2),
        field: Maybe::None,
    };

    assert_eq!(op.destination(), Maybe::Some(SsaId(1)));
    assert_eq!(op.sources().len(), 1);
    assert_eq!(op.sources()[0], SsaId(2));
}

#[test]
fn test_ir_operation_store() {
    let op = IrOperation::Store {
        dest: SsaId(1),
        src: SsaId(2),
        field: Maybe::None,
    };

    assert_eq!(op.destination(), Maybe::None);
    assert_eq!(op.sources().len(), 2);
}

#[test]
fn test_ir_operation_call() {
    let mut args = List::new();
    args.push(SsaId(1));
    args.push(SsaId(2));

    let op = IrOperation::Call {
        result: Maybe::Some(SsaId(3)),
        function: Text::from("foo"),
        args,
    };

    assert_eq!(op.destination(), Maybe::Some(SsaId(3)));
    assert_eq!(op.sources().len(), 2);
}

#[test]
fn test_ir_operation_phi() {
    let mut incoming = List::new();
    incoming.push((BlockId(0), SsaId(1)));
    incoming.push((BlockId(1), SsaId(2)));

    let op = IrOperation::Phi {
        dest: SsaId(3),
        incoming,
    };

    assert_eq!(op.destination(), Maybe::Some(SsaId(3)));
    assert_eq!(op.sources().len(), 2);
}

#[test]
fn test_ir_operation_field_access() {
    let op = IrOperation::FieldAccess {
        dest: SsaId(1),
        src: SsaId(2),
        field: FieldPath::from_field(Text::from("x")),
    };

    assert_eq!(op.destination(), Maybe::Some(SsaId(1)));
    assert_eq!(op.sources()[0], SsaId(2));
}

// ==================================================================================
// Section 5: FlowFunction Tests (6 tests)
// ==================================================================================

#[test]
fn test_flow_function_creation() {
    let op = IrOperation::Copy {
        dest: SsaId(1),
        src: SsaId(2),
    };

    let func = FlowFunction::new(op);
    assert!(!func.conservative);
}

#[test]
fn test_flow_function_conservative() {
    let op = IrOperation::Copy {
        dest: SsaId(1),
        src: SsaId(2),
    };

    let func = FlowFunction::conservative(op);
    assert!(func.conservative);
}

#[test]
fn test_flow_function_apply_identity() {
    let op = IrOperation::Copy {
        dest: SsaId(1),
        src: SsaId(2),
    };

    let func = FlowFunction::new(op);
    let input = FlowState::new();

    let output = func.apply(&input);
    assert!(output.is_empty());
}

#[test]
fn test_flow_function_apply_conservative() {
    let op = IrOperation::Copy {
        dest: SsaId(1),
        src: SsaId(2),
    };

    let func = FlowFunction::conservative(op);
    let mut input = FlowState::new();
    input.set_field_safe(RefId(1), FieldPath::from_field(Text::from("x")), true);

    let output = func.apply(&input);
    assert!(output.conservative);
}

#[test]
fn test_flow_function_apply_load() {
    let op = IrOperation::Load {
        dest: SsaId(1),
        src: SsaId(2),
        field: Maybe::None,
    };

    let func = FlowFunction::new(op);
    let input = FlowState::new();

    let output = func.apply(&input);
    // Load preserves state by default
    assert_eq!(output.is_empty(), input.is_empty());
}

#[test]
fn test_flow_function_apply_call() {
    let mut args = List::new();
    args.push(SsaId(1));

    let op = IrOperation::Call {
        result: Maybe::None,
        function: Text::from("foo"),
        args,
    };

    let func = FlowFunction::new(op);
    let input = FlowState::new();

    let output = func.apply(&input);
    // Call preserves state (conservative handled by caller)
    assert_eq!(output.is_empty(), input.is_empty());
}

// ==================================================================================
// Section 6: FlowFunctionCompiler Tests (6 tests)
// ==================================================================================

#[test]
fn test_compiler_creation() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg);

    // Compiler created successfully (no edge functions compiled yet)
    assert!(
        compiler
            .get_edge_functions(BlockId(0), BlockId(1))
            .is_none()
    );
}

#[test]
fn test_compiler_compile_all() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg).compile_all();

    // Functions compiled for edges
    assert!(
        compiler
            .get_edge_functions(BlockId(0), BlockId(1))
            .is_some()
    );
}

#[test]
fn test_compiler_get_edge_functions() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg).compile_all();

    let funcs = compiler.get_edge_functions(BlockId(0), BlockId(1));
    assert!(funcs.is_some());
}

#[test]
fn test_compiler_apply_edge() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg).compile_all();

    let input = FlowState::new();
    let output = compiler.apply_edge(BlockId(0), BlockId(1), &input);

    // Edge function applied
    assert!(output.is_empty() || !output.conservative);
}

#[test]
fn test_compiler_apply_block() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg).compile_all();

    let input = FlowState::new();
    let output = compiler.apply_block(BlockId(0), &input);

    // Block function applied
    assert!(!output.conservative);
}

#[test]
fn test_compiler_statistics() {
    let cfg = create_simple_cfg();
    let compiler = FlowFunctionCompiler::new(cfg).compile_all();

    let stats = compiler.statistics();
    assert!(stats.edge_count > 0);
    assert!(stats.block_count > 0);
    assert!(stats.total_functions > 0);
}

// ==================================================================================
// Section 7: InterproceduralFieldFlow Tests (5 tests)
// ==================================================================================

#[test]
fn test_interprocedural_creation() {
    let flow = InterproceduralFieldFlow::new();
    let stats = flow.statistics();

    assert_eq!(stats.call_site_count, 0);
    assert_eq!(stats.function_summary_count, 0);
}

#[test]
fn test_interprocedural_track_call() {
    let mut flow = InterproceduralFieldFlow::new();

    let args = List::new();
    let output = flow.track_call(BlockId(0), Text::from("foo"), args);

    // Conservative result for unknown function
    assert!(output.conservative);
}

#[test]
fn test_interprocedural_update_summary() {
    let mut flow = InterproceduralFieldFlow::new();
    let summary = FunctionFieldSummary::conservative();

    flow.update_summary(Text::from("foo"), summary);

    let stats = flow.statistics();
    assert_eq!(stats.function_summary_count, 1);
}

#[test]
fn test_interprocedural_get_summary() {
    let mut flow = InterproceduralFieldFlow::new();
    let summary = FunctionFieldSummary::conservative();

    flow.update_summary(Text::from("foo"), summary);

    let retrieved = flow.get_summary(&Text::from("foo"));
    assert!(retrieved.is_some());
}

#[test]
fn test_interprocedural_multiple_calls() {
    let mut flow = InterproceduralFieldFlow::new();

    let args1 = List::new();
    flow.track_call(BlockId(0), Text::from("foo"), args1);

    let args2 = List::new();
    flow.track_call(BlockId(1), Text::from("bar"), args2);

    let stats = flow.statistics();
    assert_eq!(stats.call_site_count, 2);
}

// ==================================================================================
// Section 8: Helper Functions Tests (3 tests)
// ==================================================================================

#[test]
fn test_field_flow_across_call_helper() {
    let function = Text::from("foo");
    let args = List::new();
    let input = FlowState::new();

    let output = field_flow_across_call(&function, &args, &input);

    // Conservative: preserves input
    assert_eq!(output.is_empty(), input.is_empty());
}

#[test]
fn test_build_flow_function_helper() {
    let op = IrOperation::Copy {
        dest: SsaId(1),
        src: SsaId(2),
    };

    let func = build_flow_function(op);
    assert!(!func.conservative);
}

#[test]
fn test_merge_flow_states_helper() {
    let mut states = List::new();

    let mut state1 = FlowState::new();
    state1.set_field_safe(RefId(1), FieldPath::from_field(Text::from("x")), true);

    let mut state2 = FlowState::new();
    state2.set_field_safe(RefId(1), FieldPath::from_field(Text::from("x")), false);

    states.push(state1);
    states.push(state2);

    let merged = merge_flow_states(&states);

    // Conservative merge
    assert!(!merged.is_field_safe(RefId(1), &FieldPath::from_field(Text::from("x"))));
}

// ==================================================================================
// Section 9: EscapeAnalyzer Integration Tests (5 tests)
// ==================================================================================

#[test]
fn test_analyzer_compute_flow_functions() {
    let cfg = create_simple_cfg();
    let analyzer = EscapeAnalyzer::new(cfg);

    let compiler = analyzer.compute_flow_functions();
    let stats = compiler.statistics();

    assert!(stats.edge_count > 0);
}

#[test]
fn test_analyzer_apply_flow_function() {
    let cfg = create_simple_cfg();
    let analyzer = EscapeAnalyzer::new(cfg);
    let compiler = analyzer.compute_flow_functions();

    let input = FlowState::new();
    let output = analyzer.apply_flow_function(compiler, BlockId(0), BlockId(1), &input);

    assert!(!output.conservative);
}

#[test]
fn test_analyzer_field_flow_across_call() {
    let cfg = create_simple_cfg();
    let analyzer = EscapeAnalyzer::new(cfg);

    let state = FlowState::new();
    let args = List::new();

    let output = analyzer.field_flow_across_call(Text::from("foo"), &args, &state);

    assert!(!output.conservative);
}

#[test]
fn test_analyzer_analyze_field_sensitive() {
    let cfg = create_simple_cfg();
    let analyzer = EscapeAnalyzer::new(cfg);

    // Test field-sensitive analysis
    let info = analyzer.analyze_field_sensitive(RefId(1));
    assert_eq!(info.reference, RefId(1));

    // Verify analyzer was created with correct CFG
    assert_eq!(analyzer.cfg().entry, BlockId(0));
}

#[test]
fn test_analyzer_build_interprocedural_field_flow() {
    let cfg = create_simple_cfg();
    let analyzer = EscapeAnalyzer::new(cfg);

    // Test interprocedural field flow construction
    let tracker = analyzer.build_interprocedural_field_flow();
    let stats = tracker.statistics();

    // In the simple CFG, we have no uses, so no calls will be extracted
    // If there were uses, conservative extraction would identify them
    assert_eq!(stats.call_site_count, 0);
    assert_eq!(stats.function_summary_count, 0);

    // Verify analyzer was created with correct CFG
    assert_eq!(analyzer.cfg().entry, BlockId(0));
}

// ==================================================================================
// Section 10: Edge Cases and Stress Tests (3 tests)
// ==================================================================================

#[test]
fn test_deeply_nested_field_paths() {
    let mut path = FieldPath::from_field(Text::from("a"));
    for i in 0..10 {
        path = path.extend(Text::from(format!("field_{}", i)));
    }

    assert_eq!(path.depth(), 11);
}

#[test]
fn test_large_flow_state() {
    let mut state = FlowState::new();

    // Add many references and fields
    for ref_id in 0..100 {
        for field_id in 0..10 {
            state.set_field_safe(
                RefId(ref_id),
                FieldPath::from_field(Text::from(format!("f{}", field_id))),
                true,
            );
        }
    }

    assert_eq!(state.total_safe_fields(), 1000);
}

#[test]
fn test_empty_state_merge() {
    let state1 = FlowState::new();
    let state2 = FlowState::new();

    let merged = state1.merge(&state2);
    assert!(merged.is_empty());
}
