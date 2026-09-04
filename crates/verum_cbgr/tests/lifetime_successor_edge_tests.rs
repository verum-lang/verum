//! A reference created in a successor block is not a dangling reference.
//!
//! T1102. The CBGR diagnostics converter had never had a production
//! caller, so the first question was whether to wire it up. Measured
//! first, and the answer was no: E312 "dangling reference detected"
//! came out of three independently healthy programs and one genuine
//! use-after-free, all four with the same text.
//!
//!     let x = 5; let r = &x; print(*r);        E312
//!     takes(&mk())      -- runs, prints 7      E312
//!     drop(data); *ref_data  -- a real UAF     E312
//!
//! `core/collections/list.vr` carried four of them on the ordinary
//! `result + &items[i].to_text()` shape, which is why "four stdlib
//! lifetime bugs" was the wrong reading.
//!
//! The cause is one constraint site. `generate_constraints` walks
//! successor edges and, for every lifetime live at the SUCCESSOR,
//! demands it also be live at the current block:
//!
//!     if lifetime.live_blocks.contains(&succ_id) {   // live at SUCC
//!         min_blocks.insert(*block_id);              // demand at PRED
//!         min_blocks.insert(succ_id);
//!
//! Liveness flows BACKWARD, and a reference DEFINED in the successor is
//! exactly the exception: it is live there because the successor
//! creates it, so requiring it in the predecessor asks the reference to
//! exist before it was made. Every `&` in a function with more than one
//! basic block meets that description.
//!
//! These tests drive the analysis directly rather than through the
//! compiler, because the CFG is where the defect lives and a two-block
//! graph is the smallest thing that exhibits it.

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use verum_cbgr::lifetime_analysis::{LifetimeAnalyzer, ViolationKind};
use verum_common::{List, Set};

/// entry -> body, with the reference created and used in `body`.
fn two_block_cfg_defining_in_the_successor(r: RefId) -> ControlFlowGraph {
    let entry = BlockId(0);
    let body = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, body);

    let mut entry_succs = Set::new();
    entry_succs.insert(body);
    cfg.add_block(BasicBlock::new(
        entry,
        Set::new(),
        entry_succs,
        List::new(),
        List::new(),
        List::new(),
    ));

    let mut body_preds = Set::new();
    body_preds.insert(entry);

    let mut defs = List::new();
    defs.push(DefSite {
        block: body,
        reference: r,
        is_stack_allocated: true,
        span: None,
    });

    let mut uses = List::new();
    uses.push(UseeSite {
        block: body,
        reference: r,
        is_mutable: false,
        span: None,
    });

    cfg.add_block(BasicBlock::new(
        body,
        body_preds,
        Set::new(),
        defs,
        uses,
        List::new(),
    ));

    cfg
}

#[test]
fn the_analysis_actually_sees_the_reference() {
    // The positive control for the test below. "No dangling violation"
    // is the shape of a working analysis AND of an analysis that never
    // received the reference at all, and those must not read alike.
    let r = RefId(1);
    let result = LifetimeAnalyzer::new(two_block_cfg_defining_in_the_successor(r)).analyze();

    assert!(
        result.ref_lifetimes.contains_key(&r),
        "the analysis never gave RefId(1) a lifetime, so a clean result \
         below would mean 'nothing was analysed', not 'nothing was wrong'"
    );
    assert!(
        !result.lifetimes.is_empty(),
        "no lifetimes were built at all"
    );
}

#[test]
fn a_reference_defined_in_the_successor_is_not_dangling() {
    let r = RefId(1);
    let result = LifetimeAnalyzer::new(two_block_cfg_defining_in_the_successor(r)).analyze();

    let dangling = result
        .violations
        .iter()
        .filter(|v| v.kind == ViolationKind::DanglingReference)
        .count();
    let first = result
        .violations
        .iter()
        .find(|v| v.kind == ViolationKind::DanglingReference)
        .map(|v| v.message.clone())
        .unwrap_or_default();

    assert_eq!(
        dangling, 0,
        "a reference created in the successor block was reported as \
         dangling; first message: {first}\n\
         Liveness flows backward, and a value DEFINED in the successor \
         is live there because the successor creates it — demanding it \
         in the predecessor asks it to exist before it was made."
    );
}
