//! Escape Analysis Demonstration
//!
//! This example demonstrates how escape analysis enables automatic optimization
//! of CBGR checks, transforming ~15ns overhead to 0ns for NoEscape references.
//!
//! Run with: cargo run --example escape_analysis_demo

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use verum_cbgr::escape_analysis::{EnhancedEscapeAnalyzer, EscapeState};
use verum_cbgr::escape_codegen_integration::EscapeAwareCodegen;
use verum_core::Set;

fn main() {
    println!("=== Escape Analysis Demonstration ===\n");

    // Scenario 1: NoEscape - Local variable
    println!("Scenario 1: Local Variable (NoEscape)\n");
    demonstrate_no_escape();

    println!("\n{}\n", "=".repeat(60));

    // Scenario 2: Escapes - Return value
    println!("Scenario 2: Return Value (Escapes)\n");
    demonstrate_return_escape();

    println!("\n{}\n", "=".repeat(60));

    // Scenario 3: Escapes - Heap allocation
    println!("Scenario 3: Heap Allocation (Escapes)\n");
    demonstrate_heap_escape();

    println!("\n{}\n", "=".repeat(60));

    // Scenario 4: Codegen Integration
    println!("Scenario 4: Codegen Integration\n");
    demonstrate_codegen_integration();

    println!("\n{}\n", "=".repeat(60));

    // Performance summary
    println!("Performance Impact Summary:\n");
    print_performance_summary();
}

fn demonstrate_no_escape() {
    println!("Verum Code:");
    println!("```verum");
    println!("fn process_local(data: &List<Int>) -> Int {{");
    println!("    let sum = 0;");
    println!("    for item in data {{");
    println!("        sum += item;  // 'item' is NoEscape");
    println!("    }}");
    println!("    sum  // Returns Int, not reference");
    println!("}}");
    println!("```\n");

    // Build CFG
    let mut cfg = create_simple_cfg();
    let ref_id = RefId(1);

    // Stack-allocated reference used only in entry block
    add_stack_ref(&mut cfg, BlockId(0), ref_id);
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);

    // Analyze
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Print results
    println!("Analysis Results:");
    if let Some(state) = result.escape_states.get(&ref_id) {
        println!("  Escape State: {}", state);
        println!("  CBGR Overhead: {} ns", if *state == EscapeState::NoEscape { 0 } else { 15 });
        println!(
            "  Optimization: {}",
            if *state == EscapeState::NoEscape {
                "✅ Automatic promotion to &checked T"
            } else {
                "❌ Keep CBGR checks"
            }
        );
    }

    println!("\n{}", result.stats);
}

fn demonstrate_return_escape() {
    println!("Verum Code:");
    println!("```verum");
    println!("fn first_element(data: &List<Int>) -> &Int {{");
    println!("    &data[0]  // Returns reference - ESCAPES");
    println!("}}");
    println!("```\n");

    // Build CFG
    let mut cfg = create_simple_cfg();
    let ref_id = RefId(2);

    // Reference defined in entry, used in exit (return)
    add_stack_ref(&mut cfg, BlockId(0), ref_id);
    add_ref_use(&mut cfg, BlockId(1), ref_id, false); // Exit block

    // Analyze
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Print results
    println!("Analysis Results:");
    if let Some(state) = result.escape_states.get(&ref_id) {
        println!("  Escape State: {}", state);
        println!("  CBGR Overhead: ~{} ns", if *state == EscapeState::NoEscape { 0 } else { 15 });
        println!(
            "  Optimization: {}",
            if *state == EscapeState::NoEscape {
                "✅ Can optimize"
            } else {
                "❌ Must use CBGR checks"
            }
        );
    }

    // Print escape points
    if !result.escape_points.is_empty() {
        println!("\nEscape Points Detected:");
        for (i, point) in result.escape_points.iter().enumerate() {
            println!("  {}. {}", i + 1, point.description);
            println!("     Kind: {}", point.escape_kind.name());
            println!("     Hint: {}", point.escape_kind.optimization_hint());
        }
    }
}

fn demonstrate_heap_escape() {
    println!("Verum Code:");
    println!("```verum");
    println!("fn create_box(value: Int) -> Heap<Int> {{");
    println!("    let boxed = Heap.new(value);  // Heap allocation");
    println!("    boxed  // Reference stored on heap - ESCAPES");
    println!("}}");
    println!("```\n");

    // Build CFG
    let mut cfg = create_simple_cfg();
    let ref_id = RefId(3);

    // Heap-allocated reference
    add_heap_ref(&mut cfg, BlockId(0), ref_id);
    add_ref_use(&mut cfg, BlockId(0), ref_id, false);

    // Analyze
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    let result = analyzer.analyze();

    // Print results
    println!("Analysis Results:");
    if let Some(state) = result.escape_states.get(&ref_id) {
        println!("  Escape State: {}", state);
        println!("  CBGR Overhead: ~{} ns", if *state == EscapeState::NoEscape { 0 } else { 15 });
    }

    println!("\n{}", result.stats);
}

fn demonstrate_codegen_integration() {
    println!("Demonstrating codegen integration with escape analysis...\n");

    // Create CFG with mix of NoEscape and Escaping references
    let mut cfg = create_simple_cfg();

    let no_escape_ref = RefId(10);
    let escape_ref = RefId(11);

    // NoEscape: local variable
    add_stack_ref(&mut cfg, BlockId(0), no_escape_ref);
    add_ref_use(&mut cfg, BlockId(0), no_escape_ref, false);

    // Escapes: return value
    add_stack_ref(&mut cfg, BlockId(0), escape_ref);
    add_ref_use(&mut cfg, BlockId(1), escape_ref, false);

    // Run escape analysis
    let mut analyzer = EnhancedEscapeAnalyzer::new(cfg);
    analyzer.analyze();

    // Create codegen integrator
    let mut codegen = EscapeAwareCodegen::with_analyzer(analyzer);

    // Simulate code generation
    println!("Code Generation Decisions:");

    // NoEscape reference
    let can_skip = codegen.can_skip_cbgr_check(no_escape_ref);
    let overhead = codegen.get_expected_overhead(no_escape_ref);
    let hint = codegen.get_ide_hint(no_escape_ref);

    println!("\n  Reference {:?}:", no_escape_ref);
    println!("    Can skip CBGR check: {}", can_skip);
    println!("    Expected overhead: {} ns", overhead);
    println!("    IDE hint: {}", hint);

    // Escaping reference
    let can_skip = codegen.can_skip_cbgr_check(escape_ref);
    let overhead = codegen.get_expected_overhead(escape_ref);
    let hint = codegen.get_ide_hint(escape_ref);

    println!("\n  Reference {:?}:", escape_ref);
    println!("    Can skip CBGR check: {}", can_skip);
    println!("    Expected overhead: {} ns", overhead);
    println!("    IDE hint: {}", hint);

    // Print codegen stats
    println!("\n{}", codegen.generate_report());
}

fn print_performance_summary() {
    println!("┌─────────────────────────────────────────────────────────┐");
    println!("│          Escape Analysis Performance Impact            │");
    println!("├─────────────────────────────────────────────────────────┤");
    println!("│                                                         │");
    println!("│  NoEscape References (Optimized):                       │");
    println!("│    • CBGR Overhead: 0 ns ✅                             │");
    println!("│    • Implementation: Direct pointer access              │");
    println!("│    • Use Cases: Loop variables, local temporaries       │");
    println!("│                                                         │");
    println!("│  Escaping References (CBGR Required):                   │");
    println!("│    • CBGR Overhead: ~15 ns ⚠️                           │");
    println!("│    • Implementation: Generation check                   │");
    println!("│    • Use Cases: Return values, heap storage             │");
    println!("│                                                         │");
    println!("│  Typical Application Impact:                            │");
    println!("│    • Hot loops: 0% overhead (NoEscape optimization)     │");
    println!("│    • Overall: 0.5-1% overhead (many refs optimized)     │");
    println!("│                                                         │");
    println!("│  Key Benefits:                                          │");
    println!("│    ✓ Automatic optimization (no manual annotations)     │");
    println!("│    ✓ Conservative analysis (always safe)                │");
    println!("│    ✓ IDE transparency (show 0ns vs 15ns hints)          │");
    println!("│    ✓ Production-ready performance                       │");
    println!("│                                                         │");
    println!("└─────────────────────────────────────────────────────────┘");
}

// Helper functions

fn create_simple_cfg() -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    let mut entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![],
        uses: vec![],
    };
    entry_block.successors.insert(exit);

    let mut exit_block = BasicBlock {
        id: exit,
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: vec![],
        uses: vec![],
    };
    exit_block.predecessors.insert(entry);

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    cfg
}

fn add_stack_ref(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.definitions.push(DefSite {
            block: block_id,
            reference: ref_id,
            is_stack_allocated: true,
        });
    }
}

fn add_heap_ref(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.definitions.push(DefSite {
            block: block_id,
            reference: ref_id,
            is_stack_allocated: false,
        });
    }
}

fn add_ref_use(cfg: &mut ControlFlowGraph, block_id: BlockId, ref_id: RefId, is_mutable: bool) {
    if let Some(block) = cfg.blocks.get_mut(&block_id) {
        block.uses.push(UseeSite {
            block: block_id,
            reference: ref_id,
            is_mutable,
        });
    }
}
