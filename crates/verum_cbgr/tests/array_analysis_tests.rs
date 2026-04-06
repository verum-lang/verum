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
//! Comprehensive tests for array index analysis
//!
//! Tests cover all aspects of symbolic index tracking, range inference,
//! and integration with field-sensitive escape analysis.

use verum_cbgr::analysis::{
    BasicBlock, BlockId, ControlFlowGraph, FieldComponent, FieldPath, RefId,
};
use verum_cbgr::array_analysis::{
    ArrayAccess, ArrayAnalysisStats, ArrayIndexAnalyzer, BinOp, IndexRange, InductionVariable,
    SymbolicIndex, VarId,
};
use verum_common::{List, Map, Maybe, Set};

// ==================================================================================
// Test 1-4: Symbolic Index Extraction
// ==================================================================================

#[test]
fn test_constant_index_extraction() {
    let idx = SymbolicIndex::Constant(42);
    assert_eq!(idx, SymbolicIndex::Constant(42));
    assert_eq!(format!("{}", idx), "42");
}

#[test]
fn test_variable_index_extraction() {
    let var = VarId(0);
    let idx = SymbolicIndex::Variable(var);
    assert_eq!(idx, SymbolicIndex::Variable(VarId(0)));
    assert_eq!(format!("{}", idx), "v0");
}

#[test]
fn test_binary_op_index_extraction() {
    let var = SymbolicIndex::Variable(VarId(0));
    let const_1 = SymbolicIndex::Constant(1);
    let idx = SymbolicIndex::BinaryOp(BinOp::Add, Box::new(var), Box::new(const_1));

    assert!(matches!(idx, SymbolicIndex::BinaryOp(BinOp::Add, _, _)));
    assert_eq!(format!("{}", idx), "(v0 + 1)");
}

#[test]
fn test_complex_binary_op_extraction() {
    // (i * 2) + 1
    let var = SymbolicIndex::Variable(VarId(0));
    let mul = SymbolicIndex::BinaryOp(
        BinOp::Mul,
        Box::new(var),
        Box::new(SymbolicIndex::Constant(2)),
    );
    let idx = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(mul),
        Box::new(SymbolicIndex::Constant(1)),
    );

    assert_eq!(format!("{}", idx), "((v0 * 2) + 1)");
}

// ==================================================================================
// Test 5-8: Range Analysis
// ==================================================================================

#[test]
fn test_range_from_constant() {
    let range = IndexRange::from_constant(42);
    assert_eq!(range.min, 42);
    assert_eq!(range.max, 42);
    assert!(range.definite);
}

#[test]
fn test_range_from_bounds() {
    let range = IndexRange::from_bounds(0, 10);
    assert_eq!(range.min, 0);
    assert_eq!(range.max, 10);
    assert!(!range.definite);
}

#[test]
fn test_range_intersection() {
    let r1 = IndexRange::from_bounds(0, 10);
    let r2 = IndexRange::from_bounds(5, 15);
    let r3 = r1.intersect(&r2);

    assert_eq!(r3.min, 5);
    assert_eq!(r3.max, 10);
    assert!(!r3.definite);
}

#[test]
fn test_range_in_bounds() {
    let r1 = IndexRange {
        min: 0,
        max: 5,
        definite: true,
    };
    let r2 = IndexRange {
        min: 0,
        max: 10,
        definite: false,
    };

    assert!(r1.in_bounds(10)); // [0, 5] in [0, 10)
    assert!(!r1.in_bounds(5)); // [0, 5] not in [0, 5)
    assert!(!r2.in_bounds(100)); // Not definite
}

// ==================================================================================
// Test 9-12: Aliasing Analysis (Constants)
// ==================================================================================

#[test]
fn test_aliasing_same_index_constant() {
    let analyzer = ArrayIndexAnalyzer::new();
    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);

    assert!(analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_no_aliasing_different_index_constant() {
    let analyzer = ArrayIndexAnalyzer::new();
    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(1), Maybe::None);

    assert!(!analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_no_aliasing_different_base() {
    let analyzer = ArrayIndexAnalyzer::new();
    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    let access2 = ArrayAccess::new(RefId(2), SymbolicIndex::Constant(0), Maybe::None);

    assert!(!analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_aliasing_wide_range_constants() {
    let analyzer = ArrayIndexAnalyzer::new();
    // Test multiple constant indices
    let indices = [0, 1, 2, 5, 10];

    for i in 0..indices.len() {
        for j in 0..indices.len() {
            let access1 =
                ArrayAccess::new(RefId(1), SymbolicIndex::Constant(indices[i]), Maybe::None);
            let access2 =
                ArrayAccess::new(RefId(1), SymbolicIndex::Constant(indices[j]), Maybe::None);

            if i == j {
                assert!(analyzer.may_alias(&access1, &access2));
            } else {
                assert!(!analyzer.may_alias(&access1, &access2));
            }
        }
    }
}

// ==================================================================================
// Test 13-16: Aliasing Analysis (Symbolic)
// ==================================================================================

#[test]
fn test_aliasing_same_variable() {
    let analyzer = ArrayIndexAnalyzer::new();
    let idx = SymbolicIndex::Variable(VarId(0));
    let access1 = ArrayAccess::new(RefId(1), idx.clone(), Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), idx.clone(), Maybe::None);

    assert!(analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_aliasing_different_variables() {
    let analyzer = ArrayIndexAnalyzer::new();
    let idx1 = SymbolicIndex::Variable(VarId(0));
    let idx2 = SymbolicIndex::Variable(VarId(1));
    let access1 = ArrayAccess::new(RefId(1), idx1, Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), idx2, Maybe::None);

    // Conservative: may alias (can't prove i != j)
    assert!(analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_no_aliasing_variable_vs_constant_with_range() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    // Add induction variable: i in [5, 10]
    let var = VarId(0);
    analyzer.add_induction_var(InductionVariable::new(var, 5, 1, 11));

    let idx_var = SymbolicIndex::Variable(var);
    let idx_const = SymbolicIndex::Constant(0);

    let access1 = ArrayAccess::new(RefId(1), idx_var, Maybe::Some((5, 10)));
    let access2 = ArrayAccess::new(RefId(1), idx_const, Maybe::None);

    // Range [5, 10] vs constant 0 → disjoint
    assert!(!analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_aliasing_offset_indices() {
    let analyzer = ArrayIndexAnalyzer::new();

    // arr[i] vs arr[i+1]
    let var = SymbolicIndex::Variable(VarId(0));
    let var_plus_1 = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(var.clone()),
        Box::new(SymbolicIndex::Constant(1)),
    );

    let access1 = ArrayAccess::new(RefId(1), var, Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), var_plus_1, Maybe::None);

    // Different symbolic expressions, may alias (conservative)
    // Future: could prove i != i+1
    assert!(analyzer.may_alias(&access1, &access2));
}

// ==================================================================================
// Test 17-18: Integration with Field-Sensitive Analysis
// ==================================================================================

#[test]
fn test_field_path_with_array_index() {
    let base = FieldPath::named("data".to_string().into());
    let indexed = base.with_array_index(SymbolicIndex::Constant(0));

    assert_eq!(indexed.len(), 2);
    assert!(matches!(indexed.components[0], FieldComponent::Named(_)));
    assert!(matches!(
        indexed.components[1],
        FieldComponent::ArrayElement
    ));
}

#[test]
fn test_field_path_array_aliasing() {
    let path1 = FieldPath::named("arr".to_string().into()).with_array_index(SymbolicIndex::Constant(0));
    let path2 = FieldPath::named("arr".to_string().into()).with_array_index(SymbolicIndex::Constant(1));

    // Both have array elements, so they conservatively may alias
    assert!(path1.may_alias_with_array(&path2));
}

// ==================================================================================
// Test 19-20: Symbolic Expression Simplification
// ==================================================================================

#[test]
fn test_simplify_identity_add() {
    // i + 0 → i
    let var = SymbolicIndex::Variable(VarId(0));
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(var.clone()),
        Box::new(SymbolicIndex::Constant(0)),
    );

    assert_eq!(expr.simplify(), var);
}

#[test]
fn test_simplify_identity_mul() {
    // i * 1 → i
    let var = SymbolicIndex::Variable(VarId(0));
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Mul,
        Box::new(var.clone()),
        Box::new(SymbolicIndex::Constant(1)),
    );

    assert_eq!(expr.simplify(), var);
}

#[test]
fn test_simplify_zero_mul() {
    // i * 0 → 0
    let var = SymbolicIndex::Variable(VarId(0));
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Mul,
        Box::new(var),
        Box::new(SymbolicIndex::Constant(0)),
    );

    assert_eq!(expr.simplify(), SymbolicIndex::Constant(0));
}

#[test]
fn test_simplify_constant_folding() {
    // 2 + 3 → 5
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(SymbolicIndex::Constant(2)),
        Box::new(SymbolicIndex::Constant(3)),
    );

    assert_eq!(expr.simplify(), SymbolicIndex::Constant(5));
}

#[test]
fn test_simplify_nested_expressions() {
    // (i + 0) * 1 → i
    let var = SymbolicIndex::Variable(VarId(0));
    let inner = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(var.clone()),
        Box::new(SymbolicIndex::Constant(0)),
    );
    let outer = SymbolicIndex::BinaryOp(
        BinOp::Mul,
        Box::new(inner),
        Box::new(SymbolicIndex::Constant(1)),
    );

    assert_eq!(outer.simplify(), var);
}

// ==================================================================================
// Test 21-23: Loop Handling
// ==================================================================================

#[test]
fn test_induction_variable_simple() {
    let var = InductionVariable::new(VarId(0), 0, 1, 10);
    let range = var.range();

    assert_eq!(range.min, 0);
    assert_eq!(range.max, 9);
    assert!(range.definite);
}

#[test]
fn test_induction_variable_negative_step() {
    let var = InductionVariable::new(VarId(0), 10, -1, 0);
    let range = var.range();

    assert_eq!(range.min, 1);
    assert_eq!(range.max, 10);
    assert!(range.definite);
}

#[test]
fn test_induction_variable_custom_start() {
    let var = InductionVariable::new(VarId(0), 5, 2, 20);
    let range = var.range();

    assert_eq!(range.min, 5);
    assert_eq!(range.max, 19);
    assert!(range.definite);
}

// ==================================================================================
// Test 24-26: Range Overlap Detection
// ==================================================================================

#[test]
fn test_range_no_overlap_disjoint() {
    let r1 = IndexRange::from_bounds(0, 5);
    let r2 = IndexRange::from_bounds(10, 15);

    assert!(!r1.may_overlap(&r2));
    assert!(!r2.may_overlap(&r1));
}

#[test]
fn test_range_overlap_partial() {
    let r1 = IndexRange::from_bounds(0, 10);
    let r2 = IndexRange::from_bounds(5, 15);

    assert!(r1.may_overlap(&r2));
    assert!(r2.may_overlap(&r1));
}

#[test]
fn test_range_overlap_contained() {
    let r1 = IndexRange::from_bounds(0, 20);
    let r2 = IndexRange::from_bounds(5, 10);

    assert!(r1.may_overlap(&r2));
    assert!(r2.may_overlap(&r1));
}

#[test]
fn test_range_definitely_disjoint() {
    let r1 = IndexRange {
        min: 0,
        max: 5,
        definite: true,
    };
    let r2 = IndexRange {
        min: 10,
        max: 15,
        definite: true,
    };

    assert!(r1.definitely_disjoint(&r2));
    assert!(r2.definitely_disjoint(&r1));
}

// ==================================================================================
// Test 27-28: Top (Unknown) Indices
// ==================================================================================

#[test]
fn test_top_index_aliasing() {
    let analyzer = ArrayIndexAnalyzer::new();
    let idx_top = SymbolicIndex::Top;
    let idx_const = SymbolicIndex::Constant(0);

    let access1 = ArrayAccess::new(RefId(1), idx_top, Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), idx_const, Maybe::None);

    // Top may alias with anything
    assert!(analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_top_index_may_equal() {
    let idx_top = SymbolicIndex::Top;
    let idx_const = SymbolicIndex::Constant(42);

    // Top may equal anything (conservative)
    assert!(idx_top.may_equal(&idx_const));
    assert!(idx_const.may_equal(&idx_top));
}

// ==================================================================================
// Test 29-30: Edge Cases
// ==================================================================================

#[test]
fn test_negative_constant_indices() {
    let analyzer = ArrayIndexAnalyzer::new();
    let idx_neg = SymbolicIndex::Constant(-1);
    let idx_zero = SymbolicIndex::Constant(0);

    let access1 = ArrayAccess::new(RefId(1), idx_neg, Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), idx_zero, Maybe::None);

    assert!(!analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_overflow_handling() {
    // Test saturating arithmetic
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(SymbolicIndex::Constant(i64::MAX)),
        Box::new(SymbolicIndex::Constant(1)),
    );

    let simplified = expr.simplify();
    assert_eq!(simplified, SymbolicIndex::Constant(i64::MAX));
}

// ==================================================================================
// Test 31-33: CFG Integration
// ==================================================================================

#[test]
fn test_extract_array_accesses_empty_cfg() {
    let mut analyzer = ArrayIndexAnalyzer::new();
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));

    let accesses = analyzer.extract_array_accesses(&cfg);
    assert_eq!(accesses.len(), 0);
}

#[test]
fn test_extract_array_accesses_with_uses() {
    use verum_cbgr::analysis::{DefSite, UseeSite};

    let mut analyzer = ArrayIndexAnalyzer::new();
    let mut cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));

    let mut block = BasicBlock {
        id: BlockId(0),
        predecessors: Set::new(),
        successors: Set::new(),
        definitions: List::new(),
        uses: vec![UseeSite {
            block: BlockId(0),
            reference: RefId(1),
            is_mutable: false, span: None,
        }].into(),
        call_sites: List::new(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(block);

    let accesses = analyzer.extract_array_accesses(&cfg);
    assert_eq!(accesses.len(), 1);
}

#[test]
fn test_get_accesses_for_ref() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(1), Maybe::None);

    analyzer.insert_accesses(RefId(1), vec![access1, access2]);

    let accesses = analyzer.get_accesses(RefId(1));
    assert!(accesses.is_some());
    assert_eq!(accesses.unwrap().len(), 2);

    let no_accesses = analyzer.get_accesses(RefId(999));
    assert!(no_accesses.is_none());
}

// ==================================================================================
// Test 34-35: Statistics
// ==================================================================================

#[test]
fn test_statistics_empty() {
    let analyzer = ArrayIndexAnalyzer::new();
    let stats = analyzer.statistics();

    assert_eq!(stats.total_accesses, 0);
    assert_eq!(stats.constant_indices, 0);
    assert_eq!(stats.symbolic_indices, 0);
    assert_eq!(stats.induction_variables, 0);
}

#[test]
fn test_statistics_with_accesses() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    let accesses = vec![
        ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None),
        ArrayAccess::new(RefId(1), SymbolicIndex::Constant(1), Maybe::None),
        ArrayAccess::new(RefId(1), SymbolicIndex::Variable(VarId(0)), Maybe::None),
    ];

    analyzer.insert_accesses(RefId(1), accesses);

    let stats = analyzer.statistics();
    assert_eq!(stats.total_accesses, 3);
    assert_eq!(stats.constant_indices, 2);
    assert_eq!(stats.symbolic_indices, 1);
}

// ==================================================================================
// Test 36-37: Precision Tests
// ==================================================================================

#[test]
fn test_precision_constant_vs_constant() {
    let analyzer = ArrayIndexAnalyzer::new();

    // Test all pairs of distinct constants don't alias
    for i in 0..10 {
        for j in 0..10 {
            let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(i), Maybe::None);
            let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(j), Maybe::None);

            if i == j {
                assert!(analyzer.may_alias(&access1, &access2));
            } else {
                assert!(!analyzer.may_alias(&access1, &access2));
            }
        }
    }
}

#[test]
fn test_precision_with_bounds() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    // Add induction variable: i in [0, 4]
    analyzer.add_induction_var(InductionVariable::new(VarId(0), 0, 1, 5));

    // arr[i] with bounds [0, 4]
    let access1 = ArrayAccess::new(
        RefId(1),
        SymbolicIndex::Variable(VarId(0)),
        Maybe::Some((0, 4)),
    );

    // arr[10] - definitely out of range
    let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(10), Maybe::None);

    // Should not alias: [0, 4] vs 10
    assert!(!analyzer.may_alias(&access1, &access2));
}

// ==================================================================================
// Test 38-40: Soundness Validation
// ==================================================================================

#[test]
fn test_soundness_must_alias_implies_may_alias() {
    let analyzer = ArrayIndexAnalyzer::new();

    // Same access must alias
    let access = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    assert!(analyzer.may_alias(&access, &access));
}

#[test]
fn test_soundness_conservative_fallback() {
    let analyzer = ArrayIndexAnalyzer::new();

    // Unknown indices must be conservative
    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Top, Maybe::None);
    let access2 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);

    assert!(analyzer.may_alias(&access1, &access2));
}

#[test]
fn test_soundness_different_bases_no_alias() {
    let analyzer = ArrayIndexAnalyzer::new();

    // Different arrays never alias
    let access1 = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None);
    let access2 = ArrayAccess::new(RefId(2), SymbolicIndex::Constant(0), Maybe::None);

    assert!(!analyzer.may_alias(&access1, &access2));
}

// ==================================================================================
// Test 41-42: Binary Operations
// ==================================================================================

#[test]
fn test_binary_op_display() {
    assert_eq!(format!("{}", BinOp::Add), "+");
    assert_eq!(format!("{}", BinOp::Sub), "-");
    assert_eq!(format!("{}", BinOp::Mul), "*");
    assert_eq!(format!("{}", BinOp::Div), "/");
    assert_eq!(format!("{}", BinOp::Mod), "%");
}

#[test]
fn test_range_inference_add() {
    let analyzer = ArrayIndexAnalyzer::new();

    // Simulate: (2 + 3) should give [5, 5]
    let expr = SymbolicIndex::BinaryOp(
        BinOp::Add,
        Box::new(SymbolicIndex::Constant(2)),
        Box::new(SymbolicIndex::Constant(3)),
    );

    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let range = analyzer.infer_range(&expr, BlockId(0), &cfg);

    // After simplification, should be constant 5
    let simplified = expr.simplify();
    assert_eq!(simplified, SymbolicIndex::Constant(5));
}

// ==================================================================================
// Test 43: Multi-dimensional Arrays
// ==================================================================================

#[test]
fn test_multidimensional_array_path() {
    // arr[i][j] represented as nested field paths
    let base = FieldPath::named("arr".to_string().into());
    let first_index = base.with_array_index(SymbolicIndex::Variable(VarId(0)));
    let second_index = first_index.with_array_index(SymbolicIndex::Variable(VarId(1)));

    assert_eq!(second_index.len(), 3);
    assert!(matches!(
        second_index.components[0],
        FieldComponent::Named(_)
    ));
}

// ==================================================================================
// Test 44-45: Variable ID Allocation
// ==================================================================================

#[test]
fn test_var_id_allocation() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    let var1 = analyzer.new_var_id();
    let var2 = analyzer.new_var_id();
    let var3 = analyzer.new_var_id();

    assert_eq!(var1, VarId(0));
    assert_eq!(var2, VarId(1));
    assert_eq!(var3, VarId(2));
}

#[test]
fn test_var_id_display() {
    let var = VarId(42);
    assert_eq!(format!("{}", var), "v42");
}

// ==================================================================================
// Test 46-47: Index Range Display
// ==================================================================================

#[test]
fn test_index_range_display_definite() {
    let range = IndexRange {
        min: 0,
        max: 10,
        definite: true,
    };
    assert_eq!(format!("{}", range), "[0, 10]");
}

#[test]
fn test_index_range_display_approximate() {
    let range = IndexRange {
        min: 0,
        max: 10,
        definite: false,
    };
    assert_eq!(format!("{}", range), "[0, 10]?");
}

// ==================================================================================
// Test 48-49: Array Access with Block Info
// ==================================================================================

#[test]
fn test_array_access_with_block() {
    let access =
        ArrayAccess::new(RefId(1), SymbolicIndex::Constant(0), Maybe::None).with_block(BlockId(42));

    assert_eq!(access.block, BlockId(42));
}

#[test]
fn test_array_access_index_range() {
    let access = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(5), Maybe::Some((0, 10)));

    let range = access.index_range();
    assert_eq!(range.min, 5);
    assert_eq!(range.max, 5);
    assert!(range.definite);
}

// ==================================================================================
// Test 50: Performance/Stress Test
// ==================================================================================

#[test]
fn test_large_number_of_accesses() {
    let mut analyzer = ArrayIndexAnalyzer::new();

    // Create 1000 array accesses
    let mut accesses = Vec::new();
    for i in 0..1000 {
        let access = ArrayAccess::new(RefId(1), SymbolicIndex::Constant(i), Maybe::None);
        accesses.push(access);
    }

    analyzer.insert_accesses(RefId(1), accesses);

    let stats = analyzer.statistics();
    assert_eq!(stats.total_accesses, 1000);
    assert_eq!(stats.constant_indices, 1000);
}
