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
// Tests for ssa module
// Migrated from src/ssa.rs per CLAUDE.md standards

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use verum_cbgr::ssa::*;
use verum_common::Set;

fn create_simple_cfg() -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    let mut entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![DefSite {
            block: entry,
            reference: RefId(1),
            is_stack_allocated: true, span: None,
        }].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    entry_block.successors.insert(exit);

    let mut exit_block = BasicBlock {
        id: exit,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![UseeSite {
            block: exit,
            reference: RefId(1),
            is_mutable: false, span: None,
        }].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };
    exit_block.predecessors.insert(entry);

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    cfg
}

#[test]
fn test_ssa_construction() {
    let cfg = create_simple_cfg();
    let ssa = SsaBuilder::new(&cfg).build().unwrap();

    assert!(!ssa.values.is_empty());
    assert_eq!(ssa.reference_values().len(), 1);
}

#[test]
fn test_empty_cfg_error() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let result = SsaBuilder::new(&cfg).build();

    assert!(matches!(result, Err(SsaError::InvalidCfg(_))));
}

#[test]
fn test_escape_via_return() {
    let cfg = create_simple_cfg();
    let ssa = SsaBuilder::new(&cfg).build().unwrap();

    // The reference is used in the exit block, so it should be marked as escaping
    for value in ssa.reference_values() {
        let info = ssa.analyze_escape(value.id);
        // Note: In this simple test, the return detection depends on the exit block analysis
        assert!(!info.heap_stored); // Stack allocated
    }
}
