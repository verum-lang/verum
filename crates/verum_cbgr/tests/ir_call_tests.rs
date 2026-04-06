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
//! Comprehensive tests for IR-based call site extraction
//!
//! Tests the production-grade IR call extraction implementation with 18+ scenarios

use verum_cbgr::analysis::{BlockId, ControlFlowGraph, EscapeAnalyzer, FunctionId, RefId};
use verum_cbgr::call_graph::CallGraph;
use verum_cbgr::ir_call_extraction::{
    CallArgMapping, ExtractionStats, IrCallExtractor, IrCallInfo, IrCallSite, IrFunction,
    IrInstruction, IrOperand,
};
use verum_common::{Maybe, Text};

// ==================================================================================
// Test 1-5: IrOperand and IrInstruction Tests
// ==================================================================================

#[test]
fn test_ir_operand_reference_detection() {
    let local = IrOperand::LocalVar(1);
    assert!(!local.is_reference());
    assert_eq!(local.as_reference(), Maybe::None);

    let ref_op = IrOperand::Reference(RefId(42));
    assert!(ref_op.is_reference());
    assert_eq!(ref_op.as_reference(), Maybe::Some(RefId(42)));
}

#[test]
fn test_ir_operand_argument_detection() {
    let arg = IrOperand::Argument(2);
    assert!(arg.is_argument());
    assert_eq!(arg.argument_index(), Maybe::Some(2));

    let local = IrOperand::LocalVar(1);
    assert!(!local.is_argument());
    assert_eq!(local.argument_index(), Maybe::None);
}

#[test]
fn test_ir_instruction_call_detection() {
    let call = IrInstruction::Call {
        target: "foo".into(),
        args: vec![IrOperand::LocalVar(0)],
        result: Maybe::Some(1),
    };
    assert!(call.is_call());

    if let Maybe::Some((target, args, result)) = call.as_call() {
        assert_eq!(target, "foo");
        assert_eq!(args.len(), 1);
        assert_eq!(result, Maybe::Some(1));
    } else {
        panic!("Call extraction failed");
    }
}

#[test]
fn test_ir_instruction_return_detection() {
    let ret = IrInstruction::Return {
        value: Maybe::Some(IrOperand::Reference(RefId(1))),
    };
    assert!(ret.is_return());

    let assign = IrInstruction::Assign {
        dest: 1,
        src: IrOperand::Constant(42),
    };
    assert!(!assign.is_return());
}

#[test]
fn test_ir_instruction_stores_reference() {
    let store_ref = IrInstruction::Store {
        ptr: IrOperand::LocalVar(1),
        value: IrOperand::Reference(RefId(2)),
    };
    assert!(store_ref.stores_reference());

    let store_value = IrInstruction::Store {
        ptr: IrOperand::LocalVar(1),
        value: IrOperand::Constant(42),
    };
    assert!(!store_value.stores_reference());
}

// ==================================================================================
// Test 6-10: IrFunction Tests
// ==================================================================================

#[test]
fn test_ir_function_creation() {
    let func = IrFunction::new(FunctionId(1), "test_func");
    assert_eq!(func.id, FunctionId(1));
    assert_eq!(func.name, Text::from("test_func"));
    assert_eq!(func.instruction_count(), 0);
}

#[test]
fn test_ir_function_add_parameter() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_parameter(0, "x");
    func.add_parameter(1, "y");

    assert_eq!(func.parameters.len(), 2);
    assert_eq!(func.parameters.get(&0), Some(&Text::from("x")));
    assert_eq!(func.parameters.get(&1), Some(&Text::from("y")));
}

#[test]
fn test_ir_function_add_instruction() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    let inst = IrInstruction::Call {
        target: "foo".into(),
        args: vec![],
        result: Maybe::None,
    };
    func.add_instruction(BlockId(0), 0, inst);

    assert_eq!(func.instruction_count(), 1);
}

#[test]
fn test_ir_function_all_instructions() {
    let mut func = IrFunction::new(FunctionId(1), "test");

    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![],
            result: Maybe::None,
        },
    );
    func.add_instruction(BlockId(0), 1, IrInstruction::Return { value: Maybe::None });

    let all = func.all_instructions();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, BlockId(0));
    assert_eq!(all[0].1, 0);
}

#[test]
fn test_ir_function_locals() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_local(1);
    func.add_local(2);
    func.add_local(3);

    assert_eq!(func.locals.len(), 3);
    assert!(func.locals.contains(&1));
    assert!(func.locals.contains(&2));
    assert!(func.locals.contains(&3));
}

// ==================================================================================
// Test 11-15: IrCallSite Tests
// ==================================================================================

#[test]
fn test_ir_call_site_creation() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(2),
        3,
        "target_func",
        vec![IrOperand::Argument(0)],
        Maybe::Some(1),
    );

    assert_eq!(site.caller, FunctionId(1));
    assert_eq!(site.block, BlockId(2));
    assert_eq!(site.instruction_offset, 3);
    assert_eq!(site.callee_name, Text::from("target_func"));
    assert_eq!(site.arguments.len(), 1);
    assert!(site.has_result());
}

#[test]
fn test_ir_call_site_passes_reference() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::Reference(RefId(42)), IrOperand::LocalVar(1)],
        Maybe::None,
    );

    assert!(site.passes_reference(RefId(42)));
    assert!(!site.passes_reference(RefId(1)));
}

#[test]
fn test_ir_call_site_reference_arguments() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![
            IrOperand::LocalVar(0),
            IrOperand::Reference(RefId(1)),
            IrOperand::Reference(RefId(2)),
            IrOperand::Constant(42),
        ],
        Maybe::None,
    );

    let ref_args = site.reference_arguments();
    assert_eq!(ref_args.len(), 2);
    assert_eq!(ref_args[0], (1, RefId(1)));
    assert_eq!(ref_args[1], (2, RefId(2)));
}

#[test]
fn test_ir_call_site_arg_mapping() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::LocalVar(0), IrOperand::Argument(1)],
        Maybe::None,
    );

    let mapping = site.arg_mapping();
    assert_eq!(mapping.arg_to_param.len(), 2);
    assert_eq!(mapping.param_for_arg(0), Maybe::Some(0));
    assert_eq!(mapping.param_for_arg(1), Maybe::Some(1));
}

#[test]
fn test_ir_call_site_display() {
    let site = IrCallSite::new(
        FunctionId(10),
        BlockId(20),
        5,
        "my_func",
        vec![],
        Maybe::None,
    );

    let display = format!("{}", site);
    assert!(display.contains("10"));
    assert!(display.contains("20"));
    assert!(display.contains("5"));
    assert!(display.contains("my_func"));
}

// ==================================================================================
// Test 16-20: CallArgMapping Tests
// ==================================================================================

#[test]
fn test_call_arg_mapping_param_for_arg() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::LocalVar(0), IrOperand::LocalVar(1)],
        Maybe::None,
    );

    let mapping = site.arg_mapping();
    assert_eq!(mapping.param_for_arg(0), Maybe::Some(0));
    assert_eq!(mapping.param_for_arg(1), Maybe::Some(1));
    assert_eq!(mapping.param_for_arg(2), Maybe::None);
}

#[test]
fn test_call_arg_mapping_arg_for_param() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::LocalVar(0)],
        Maybe::None,
    );

    let mapping = site.arg_mapping();
    assert_eq!(mapping.arg_for_param(0), Maybe::Some(0));
    assert_eq!(mapping.arg_for_param(1), Maybe::None);
}

#[test]
fn test_call_arg_mapping_reference_to_param() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::Reference(RefId(42)), IrOperand::LocalVar(1)],
        Maybe::None,
    );

    let mapping = site.arg_mapping();
    assert!(mapping.reference_to_param(RefId(42), 0));
    assert!(!mapping.reference_to_param(RefId(42), 1));
    assert!(!mapping.reference_to_param(RefId(1), 0));
}

#[test]
fn test_ir_call_info_creation() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::Reference(RefId(1))],
        Maybe::None,
    );

    let info = IrCallInfo::from_call_site(site);
    assert_eq!(info.reference_args.len(), 1);
    assert_eq!(info.reference_args[0], (0, RefId(1)));
    assert!(!info.may_retain);
    assert!(!info.may_spawn_thread);
}

#[test]
fn test_ir_call_info_marking() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "func",
        vec![IrOperand::Reference(RefId(1))],
        Maybe::None,
    );

    let info = IrCallInfo::from_call_site(site)
        .mark_retaining()
        .mark_thread_spawning();

    assert!(info.may_retain);
    assert!(info.may_spawn_thread);
}

// ==================================================================================
// Test 21-25: IrCallExtractor Tests
// ==================================================================================

#[test]
fn test_ir_call_extractor_basic_extraction() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::LocalVar(0)],
            result: Maybe::Some(1),
        },
    );
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "bar".into(),
            args: vec![],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let sites = extractor.extract_from_function(&func);

    assert_eq!(sites.len(), 2);
    assert_eq!(sites[0].callee_name, Text::from("foo"));
    assert_eq!(sites[1].callee_name, Text::from("bar"));
}

#[test]
fn test_ir_call_extractor_extract_with_info() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "std.thread.spawn".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let infos = extractor.extract_with_info(&func);

    assert_eq!(infos.len(), 1);
    assert!(infos[0].may_spawn_thread);
}

#[test]
fn test_ir_call_extractor_calls_with_reference() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::Reference(RefId(42))],
            result: Maybe::None,
        },
    );
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "bar".into(),
            args: vec![IrOperand::LocalVar(1)],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let sites = extractor.extract_calls_with_reference(&func, RefId(42));

    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].callee_name, Text::from("foo"));
}

#[test]
fn test_ir_call_extractor_calls_to_function() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![],
            result: Maybe::None,
        },
    );
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![],
            result: Maybe::None,
        },
    );
    func.add_instruction(
        BlockId(0),
        2,
        IrInstruction::Call {
            target: "bar".into(),
            args: vec![],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let sites = extractor.extract_calls_to_function(&func, "foo");

    assert_eq!(sites.len(), 2);
}

#[test]
fn test_ir_call_extractor_return_extraction() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Return {
            value: Maybe::Some(IrOperand::Reference(RefId(1))),
        },
    );
    func.add_instruction(BlockId(1), 0, IrInstruction::Return { value: Maybe::None });

    let extractor = IrCallExtractor::new();
    let returns = extractor.extract_return_sites(&func);

    assert_eq!(returns.len(), 2);
    assert!(returns[0].2.is_some());
    assert!(returns[1].2.is_none());
}

// ==================================================================================
// Test 26-30: EscapeAnalyzer Integration Tests
// ==================================================================================

#[test]
fn test_escape_analyzer_extract_call_sites() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "validate".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let sites = analyzer.extract_call_sites_from_ir(&func);
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].callee_name, Text::from("validate"));
}

#[test]
fn test_escape_analyzer_map_call_to_context() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    let site = IrCallSite::new(FunctionId(1), BlockId(2), 3, "target", vec![], Maybe::None);

    // map_call_to_context returns a call_graph::CallSite which only contains block
    let context = analyzer.map_call_to_context(&site);
    assert_eq!(context.block, BlockId(2));
    // The caller and instruction_offset are in IrCallSite, not CallSite
    assert_eq!(site.caller, FunctionId(1));
    assert_eq!(site.instruction_offset, 3);
}

#[test]
fn test_escape_analyzer_extract_calls_with_reference() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::Reference(RefId(42))],
            result: Maybe::None,
        },
    );
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "bar".into(),
            args: vec![IrOperand::LocalVar(1)],
            result: Maybe::None,
        },
    );

    let sites = analyzer.extract_calls_with_reference(RefId(42), &func);
    assert_eq!(sites.len(), 1);
}

#[test]
fn test_escape_analyzer_ir_flows_to_return() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Return {
            value: Maybe::Some(IrOperand::Reference(RefId(42))),
        },
    );

    assert!(analyzer.ir_flows_to_return(RefId(42), &func));
    assert!(!analyzer.ir_flows_to_return(RefId(1), &func));
}

#[test]
fn test_escape_analyzer_refine_context_with_ir() {
    let cfg = ControlFlowGraph::new(BlockId(0), BlockId(1));
    let analyzer = EscapeAnalyzer::new(cfg);
    let call_graph = CallGraph::new();

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "std.collections.Vec.push".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let info = analyzer.refine_context_with_ir(RefId(1), &func, &call_graph);
    // Info should reflect potential retention
    assert!(info.reference == RefId(1));
}

// ==================================================================================
// Test 31-33: ExtractionStats Tests
// ==================================================================================

#[test]
fn test_extraction_stats_creation() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let call_infos = extractor.extract_with_info(&func);
    let returns = extractor.extract_return_sites(&func);

    let stats = ExtractionStats::from_extraction(&func, &call_infos, returns.len());
    assert_eq!(stats.instructions_scanned, 1);
    assert_eq!(stats.call_sites_found, 1);
    assert_eq!(stats.call_sites_with_refs, 1);
}

#[test]
fn test_extraction_stats_percentages() {
    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "foo".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "bar".into(),
            args: vec![IrOperand::LocalVar(0)],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let call_infos = extractor.extract_with_info(&func);
    let stats = ExtractionStats::from_extraction(&func, &call_infos, 0);

    assert_eq!(stats.call_sites_found, 2);
    assert_eq!(stats.call_sites_with_refs, 1);
    assert_eq!(stats.ref_call_percentage(), 50.0);
}

#[test]
fn test_extraction_stats_display() {
    let stats = ExtractionStats {
        instructions_scanned: 100,
        call_sites_found: 10,
        call_sites_with_refs: 5,
        retaining_calls: 2,
        thread_spawn_calls: 1,
        return_sites: 3,
    };

    let display = format!("{}", stats);
    assert!(display.contains("100"));
    assert!(display.contains("10"));
    assert!(display.contains("5"));
    assert!(display.contains("50.0"));
}

// ==================================================================================
// Additional comprehensive test scenarios
// ==================================================================================

#[test]
fn test_complex_function_with_multiple_calls() {
    let mut func = IrFunction::new(FunctionId(1), "complex");
    func.add_parameter(0, "data");
    func.add_local(1);
    func.add_local(2);

    // Call 1: validate(data)
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "validate".into(),
            args: vec![IrOperand::Argument(0)],
            result: Maybe::Some(1),
        },
    );

    // Call 2: process(&data)
    func.add_instruction(
        BlockId(0),
        1,
        IrInstruction::Call {
            target: "process".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::Some(2),
        },
    );

    // Return result
    func.add_instruction(
        BlockId(0),
        2,
        IrInstruction::Return {
            value: Maybe::Some(IrOperand::LocalVar(2)),
        },
    );

    let extractor = IrCallExtractor::new();
    let sites = extractor.extract_from_function(&func);
    assert_eq!(sites.len(), 2);

    let ref_sites = extractor.extract_calls_with_reference(&func, RefId(1));
    assert_eq!(ref_sites.len(), 1);
    assert_eq!(ref_sites[0].callee_name, Text::from("process"));
}

#[test]
fn test_multi_block_function() {
    let mut func = IrFunction::new(FunctionId(1), "multi_block");

    // Block 0: Initial call
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "init".into(),
            args: vec![],
            result: Maybe::Some(1),
        },
    );

    // Block 1: Processing call
    func.add_instruction(
        BlockId(1),
        0,
        IrInstruction::Call {
            target: "process".into(),
            args: vec![IrOperand::LocalVar(1)],
            result: Maybe::None,
        },
    );

    // Block 2: Cleanup call
    func.add_instruction(
        BlockId(2),
        0,
        IrInstruction::Call {
            target: "cleanup".into(),
            args: vec![],
            result: Maybe::None,
        },
    );

    let extractor = IrCallExtractor::new();
    let sites = extractor.extract_from_function(&func);

    assert_eq!(sites.len(), 3);
    assert_eq!(sites[0].block, BlockId(0));
    assert_eq!(sites[1].block, BlockId(1));
    assert_eq!(sites[2].block, BlockId(2));
}

#[test]
fn test_ir_call_info_may_escape() {
    let site = IrCallSite::new(
        FunctionId(1),
        BlockId(0),
        0,
        "retaining_func",
        vec![IrOperand::Reference(RefId(42))],
        Maybe::None,
    );

    let info = IrCallInfo::from_call_site(site).mark_retaining();

    assert!(info.may_escape_reference(RefId(42)));
    assert!(!info.may_escape_reference(RefId(1)));
}

#[test]
fn test_custom_thread_spawn_function() {
    let mut extractor = IrCallExtractor::new();
    extractor.register_thread_spawn_function("custom.spawn");

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "custom.spawn".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let infos = extractor.extract_with_info(&func);
    assert_eq!(infos.len(), 1);
    assert!(infos[0].may_spawn_thread);
}

#[test]
fn test_custom_retaining_function() {
    let mut extractor = IrCallExtractor::new();
    extractor.register_retaining_function("custom.store");

    let mut func = IrFunction::new(FunctionId(1), "test");
    func.add_instruction(
        BlockId(0),
        0,
        IrInstruction::Call {
            target: "custom.store".into(),
            args: vec![IrOperand::Reference(RefId(1))],
            result: Maybe::None,
        },
    );

    let infos = extractor.extract_with_info(&func);
    assert_eq!(infos.len(), 1);
    assert!(infos[0].may_retain);
}
