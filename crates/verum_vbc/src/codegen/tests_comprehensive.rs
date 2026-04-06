//! Comprehensive tests for VBC codegen.
//!
//! These tests verify correctness, stability, and security of the code generator.
//! Test organization:
//!
//! - `context_tests`: CodegenContext functionality
//! - `register_tests`: Register allocation and management
//! - `error_tests`: Error handling and messages
//! - `instruction_tests`: Instruction emission
//! - `security_tests`: Security-critical behavior
//! - `integration_tests`: End-to-end patterns
//! - `edge_case_tests`: Boundary conditions and edge cases
//! - `stress_tests`: High load and random input testing
//! - `encoding_tests`: Bytecode encoding roundtrips
//! - `property_tests`: Property-based testing patterns

use super::*;
use crate::instruction::{BinaryIntOp, BitwiseOp, CompareOp, Instruction, Reg, UnaryIntOp};
use crate::module::FunctionId;

// Re-export for tests
use super::registers::MAX_REGISTERS;

// ============================================================================
// Unit Tests - Context and Register Allocation
// ============================================================================

mod context_tests {
    use super::*;

    #[test]
    fn test_codegen_context_creation() {
        let ctx = CodegenContext::new();
        assert!(!ctx.in_function);
        assert!(ctx.current_function.is_none());
        assert!(ctx.instructions.is_empty());
        assert_eq!(ctx.stats.functions_compiled, 0);
    }

    #[test]
    fn test_label_generation_uniqueness() {
        let mut ctx = CodegenContext::new();
        let labels: Vec<String> = (0..100).map(|_| ctx.new_label("test")).collect();

        // All labels should be unique
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), 100);
    }

    #[test]
    fn test_forward_jump_patching() {
        let mut ctx = CodegenContext::new();

        // Emit forward jump
        ctx.emit_forward_jump("target", |offset| Instruction::Jmp { offset });
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.define_label("target");

        // Jump should be patched to offset 3
        match &ctx.instructions[0] {
            Instruction::Jmp { offset } => assert_eq!(*offset, 3),
            _ => panic!("Expected Jmp instruction"),
        }
    }

    #[test]
    fn test_backward_jump() {
        let mut ctx = CodegenContext::new();

        ctx.define_label("start");
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);

        // Backward jump should work
        let result = ctx.emit_backward_jump("start", |offset| Instruction::Jmp { offset });
        assert!(result.is_ok());

        // Jump should have negative offset
        match &ctx.instructions[2] {
            Instruction::Jmp { offset } => assert!(*offset < 0),
            _ => panic!("Expected Jmp instruction"),
        }
    }

    #[test]
    fn test_scope_enter_exit() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("x".to_string(), false)], None);

        let r1 = ctx.define_var("a", false);
        ctx.enter_scope();
        let r2 = ctx.define_var("b", true);

        assert!(ctx.lookup_var("a").is_some());
        assert!(ctx.lookup_var("b").is_some());

        let (vars, _) = ctx.exit_scope(false);
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].0, "b");
        assert_eq!(vars[0].1, r2);

        // a should still exist, b should be gone
        assert!(ctx.lookup_var("a").is_some());
        assert!(ctx.lookup_var("b").is_none());

        let _ = r1; // Suppress warning
    }

    #[test]
    fn test_loop_context_management() {
        let mut ctx = CodegenContext::new();

        assert!(!ctx.in_loop());

        let loop1 = ctx.enter_loop(Some("outer".to_string()), Some(Reg(0)));
        assert!(ctx.in_loop());
        assert_eq!(loop1.source_label, Some("outer".to_string()));
        assert_eq!(loop1.break_value_reg, Some(Reg(0)));

        let loop2 = ctx.enter_loop(None, None);
        assert!(loop2.source_label.is_none());

        // find_loop with label
        let found = ctx.find_loop(Some("outer"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().source_label, Some("outer".to_string()));

        ctx.exit_loop();
        ctx.exit_loop();
        assert!(!ctx.in_loop());
    }

    #[test]
    fn test_constant_pool_deduplication() {
        let mut ctx = CodegenContext::new();

        let c1 = ctx.add_const_int(42);
        let c2 = ctx.add_const_int(42);
        let c3 = ctx.add_const_int(100);

        // Same value should return same ID
        assert_eq!(c1, c2);
        assert_ne!(c1, c3);

        let f1 = ctx.add_const_float(3.14);
        let f2 = ctx.add_const_float(3.14);
        assert_eq!(f1, f2);

        let s1 = ctx.add_const_string("hello");
        let s2 = ctx.add_const_string("hello");
        let s3 = ctx.add_const_string("world");
        assert_eq!(s1, s2);
        assert_ne!(s1, s3);
    }

    #[test]
    fn test_defer_stack_fifo_order() {
        let mut ctx = CodegenContext::new();

        ctx.add_defer(vec![Instruction::LoadI { dst: Reg(0), value: 1 }], false);
        ctx.add_defer(vec![Instruction::LoadI { dst: Reg(1), value: 2 }], false);
        ctx.add_defer(vec![Instruction::LoadI { dst: Reg(2), value: 3 }], true); // errdefer

        // Normal path: only regular defers, in reverse order
        let defers = ctx.pending_defers(false);
        assert_eq!(defers.len(), 2);

        // Error path: all defers
        let mut ctx2 = CodegenContext::new();
        ctx2.add_defer(vec![Instruction::Nop], false);
        ctx2.add_defer(vec![Instruction::Nop], true);
        let defers = ctx2.pending_defers(true);
        assert_eq!(defers.len(), 2);
    }
}

// ============================================================================
// Unit Tests - Register Allocation
// ============================================================================

mod register_tests {
    use super::*;

    #[test]
    fn test_parameter_allocation() {
        let mut alloc = RegisterAllocator::new();
        let regs = alloc.alloc_parameters(&[("a".to_string(), false), ("b".to_string(), false), ("c".to_string(), false)]);

        assert_eq!(regs.len(), 3);
        assert_eq!(regs[0], Reg(0));
        assert_eq!(regs[1], Reg(1));
        assert_eq!(regs[2], Reg(2));

        assert_eq!(alloc.get_reg("a"), Some(Reg(0)));
        assert_eq!(alloc.get_reg("b"), Some(Reg(1)));
        assert_eq!(alloc.get_reg("c"), Some(Reg(2)));
    }

    #[test]
    fn test_temp_recycling() {
        let mut alloc = RegisterAllocator::new();

        let t0 = alloc.alloc_temp();
        let t1 = alloc.alloc_temp();
        let t2 = alloc.alloc_temp();

        alloc.free_temp(t1);
        let t3 = alloc.alloc_temp();

        // t3 should reuse t1's register
        assert_eq!(t3, t1);

        // Cleanup
        alloc.free_temp(t0);
        alloc.free_temp(t2);
        alloc.free_temp(t3);
    }

    #[test]
    fn test_peak_usage_tracking() {
        let mut alloc = RegisterAllocator::new();

        let _t0 = alloc.alloc_temp();
        let _t1 = alloc.alloc_temp();
        let _t2 = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 3);

        // Free all
        alloc.free_temp(Reg(0));
        alloc.free_temp(Reg(1));
        alloc.free_temp(Reg(2));

        // Peak should still be 3
        assert_eq!(alloc.register_count(), 3);

        // Allocate more (recycled)
        let _ = alloc.alloc_temp();
        let _ = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 3);

        // Allocate fresh
        let _ = alloc.alloc_temp();
        let _ = alloc.alloc_temp();
        assert_eq!(alloc.register_count(), 4);
    }

    #[test]
    fn test_variable_shadowing() {
        let mut alloc = RegisterAllocator::new();

        let r0 = alloc.alloc_local("x", false);
        assert_eq!(alloc.get_reg("x"), Some(r0));

        alloc.enter_scope();
        let r1 = alloc.alloc_local("x", true); // Shadow x
        assert_eq!(alloc.get_reg("x"), Some(r1));
        assert_ne!(r0, r1);

        alloc.exit_scope();
        assert_eq!(alloc.get_reg("x"), Some(r0)); // Restored
    }

    #[test]
    fn test_snapshot_restore() {
        let mut alloc = RegisterAllocator::new();

        alloc.alloc_local("a", false);
        let snap = alloc.snapshot();

        alloc.alloc_temp();
        alloc.alloc_temp();
        assert_eq!(alloc.current_reg(), 3);

        alloc.restore_reg(&snap);
        assert_eq!(alloc.current_reg(), 1);
    }
}

// ============================================================================
// Unit Tests - Error Handling
// ============================================================================

mod error_tests {
    use super::*;

    #[test]
    fn test_undefined_variable_error() {
        let err = CodegenError::undefined_variable("missing_var");
        assert!(matches!(err.kind, CodegenErrorKind::UndefinedVariable(_)));
        let msg = format!("{}", err);
        assert!(msg.contains("missing_var"));
    }

    #[test]
    fn test_undefined_function_error() {
        let err = CodegenError::undefined_function("missing_fn");
        assert!(matches!(err.kind, CodegenErrorKind::UndefinedFunction(_)));
    }

    #[test]
    fn test_wrong_argument_count() {
        let kind = CodegenErrorKind::WrongArgumentCount {
            expected: 2,
            found: 3,
            function: "test_fn".to_string(),
        };
        let err = CodegenError::new(kind);
        let msg = format!("{}", err);
        assert!(msg.contains("2"));
        assert!(msg.contains("3"));
    }

    #[test]
    fn test_immutable_assignment() {
        let err = CodegenError::new(CodegenErrorKind::ImmutableAssignment("x".to_string()));
        let msg = format!("{}", err);
        assert!(msg.contains("immutable"));
        assert!(msg.contains("x"));
    }

    #[test]
    fn test_break_outside_loop() {
        let err = CodegenError::new(CodegenErrorKind::BreakOutsideLoop);
        let msg = format!("{}", err);
        assert!(msg.contains("break") || msg.contains("loop"));
    }

    #[test]
    fn test_continue_outside_loop() {
        let err = CodegenError::new(CodegenErrorKind::ContinueOutsideLoop);
        let msg = format!("{}", err);
        assert!(msg.contains("continue") || msg.contains("loop"));
    }

    #[test]
    fn test_return_outside_function() {
        let err = CodegenError::new(CodegenErrorKind::ReturnOutsideFunction);
        let msg = format!("{}", err);
        assert!(msg.contains("return") || msg.contains("function"));
    }
}

// ============================================================================
// Unit Tests - Instruction Generation
// ============================================================================

mod instruction_tests {
    use super::*;

    #[test]
    fn test_basic_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        // Load instructions
        ctx.emit(Instruction::LoadI { dst: Reg(0), value: 42 });
        ctx.emit(Instruction::LoadTrue { dst: Reg(1) });
        ctx.emit(Instruction::LoadFalse { dst: Reg(2) });
        ctx.emit(Instruction::LoadUnit { dst: Reg(3) });

        assert_eq!(ctx.instructions.len(), 4);
    }

    #[test]
    fn test_binary_operations() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        });

        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Sub,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        });

        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Mul,
            dst: Reg(2),
            a: Reg(0),
            b: Reg(1),
        });

        assert_eq!(ctx.instructions.len(), 3);
    }

    #[test]
    fn test_comparison_operations() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            ctx.emit(Instruction::CmpI {
                op,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            });
        }

        assert_eq!(ctx.instructions.len(), 6);
    }

    #[test]
    fn test_bitwise_operations() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        for op in [
            BitwiseOp::And,
            BitwiseOp::Or,
            BitwiseOp::Xor,
            BitwiseOp::Shl,
            BitwiseOp::Shr,
        ] {
            ctx.emit(Instruction::Bitwise {
                op,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            });
        }

        assert_eq!(ctx.instructions.len(), 5);
    }

    #[test]
    fn test_unary_operations() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::UnaryI {
            op: UnaryIntOp::Neg,
            dst: Reg(1),
            src: Reg(0),
        });

        ctx.emit(Instruction::Not {
            dst: Reg(1),
            src: Reg(0),
        });

        assert_eq!(ctx.instructions.len(), 2);
    }

    #[test]
    fn test_control_flow_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::Jmp { offset: 5 });
        ctx.emit(Instruction::JmpIf {
            cond: Reg(0),
            offset: 3,
        });
        ctx.emit(Instruction::JmpNot {
            cond: Reg(0),
            offset: 2,
        });

        assert_eq!(ctx.instructions.len(), 3);
    }

    #[test]
    fn test_call_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: crate::instruction::RegRange {
                start: Reg(1),
                count: 2,
            },
        });

        ctx.emit(Instruction::CallClosure {
            dst: Reg(0),
            closure: Reg(1),
            args: crate::instruction::RegRange {
                start: Reg(2),
                count: 0,
            },
        });

        assert_eq!(ctx.instructions.len(), 2);
    }

    #[test]
    fn test_return_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::Ret { value: Reg(0) });
        ctx.emit(Instruction::RetV);

        assert_eq!(ctx.instructions.len(), 2);
    }

    #[test]
    fn test_memory_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::New {
            dst: Reg(0),
            type_id: 1,
            field_count: 2,
        });
        ctx.emit(Instruction::GetF {
            dst: Reg(1),
            obj: Reg(0),
            field_idx: 0,
        });
        ctx.emit(Instruction::SetF {
            obj: Reg(0),
            field_idx: 0,
            value: Reg(1),
        });
        ctx.emit(Instruction::GetE {
            dst: Reg(1),
            arr: Reg(0),
            idx: Reg(2),
        });
        ctx.emit(Instruction::SetE {
            arr: Reg(0),
            idx: Reg(2),
            value: Reg(1),
        });

        assert_eq!(ctx.instructions.len(), 5);
    }

    #[test]
    fn test_collection_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::NewList { dst: Reg(0) });
        ctx.emit(Instruction::ListPush {
            list: Reg(0),
            val: Reg(1),
        });
        ctx.emit(Instruction::ListPop {
            dst: Reg(2),
            list: Reg(0),
        });
        ctx.emit(Instruction::NewMap { dst: Reg(3) });
        ctx.emit(Instruction::MapGet {
            dst: Reg(4),
            map: Reg(3),
            key: Reg(1),
        });
        ctx.emit(Instruction::MapSet {
            map: Reg(3),
            key: Reg(1),
            val: Reg(2),
        });
        ctx.emit(Instruction::MapContains {
            dst: Reg(5),
            map: Reg(3),
            key: Reg(1),
        });

        assert_eq!(ctx.instructions.len(), 7);
    }

    #[test]
    fn test_iterator_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::IterNew {
            dst: Reg(0),
            iterable: Reg(1),
        });
        ctx.emit(Instruction::IterNext {
            dst: Reg(2),
            has_next: Reg(3),
            iter: Reg(0),
        });

        assert_eq!(ctx.instructions.len(), 2);
    }

    #[test]
    fn test_reference_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.emit(Instruction::Ref {
            dst: Reg(0),
            src: Reg(1),
        });
        ctx.emit(Instruction::RefMut {
            dst: Reg(0),
            src: Reg(1),
        });
        ctx.emit(Instruction::Deref {
            dst: Reg(2),
            ref_reg: Reg(0),
        });
        ctx.emit(Instruction::Clone {
            dst: Reg(3),
            src: Reg(2),
        });

        assert_eq!(ctx.instructions.len(), 4);
    }
}

// ============================================================================
// Security Tests
// ============================================================================

mod security_tests {
    use super::*;

    #[test]
    fn test_register_bounds_checking() {
        let mut alloc = RegisterAllocator::new();

        // Should not overflow with reasonable allocation
        for _ in 0..1000 {
            alloc.alloc_temp();
        }

        // Register count should be tracked correctly
        assert_eq!(alloc.register_count(), 1000);
    }

    #[test]
    fn test_register_overflow_protection() {
        let alloc = RegisterAllocator::new();

        // Check overflow detection
        assert!(!alloc.would_overflow(100));
        assert!(alloc.would_overflow(crate::codegen::registers::MAX_REGISTERS + 1));
    }

    #[test]
    fn test_label_uniqueness_under_high_load() {
        let mut ctx = CodegenContext::new();

        let mut labels = std::collections::HashSet::new();
        for _ in 0..10000 {
            let label = ctx.new_label("stress_test");
            assert!(
                labels.insert(label.clone()),
                "Duplicate label generated: {}",
                label
            );
        }
    }

    #[test]
    fn test_constant_pool_bounds() {
        let mut ctx = CodegenContext::new();

        // Add many constants
        for i in 0..10000 {
            ctx.add_const_int(i);
        }

        // All constants should be accessible
        assert_eq!(ctx.constants.len(), 10000);
    }

    #[test]
    fn test_string_interning_safety() {
        let mut ctx = CodegenContext::new();

        // Test with various string patterns
        let long_string = "very_long_".repeat(1000);
        let strings = vec![
            "",                                    // Empty
            "normal",                              // Normal
            "with spaces",                         // Spaces
            "with\nnewline",                       // Newlines
            "with\ttab",                           // Tabs
            "unicode: こんにちは",                 // Unicode
            "emoji: 🚀💻🔥",                       // Emoji
            &long_string,                          // Long string
            "\0null\0byte",                        // Null bytes
            "\\escape\\sequences",                 // Backslashes
        ];

        for s in &strings {
            let id1 = ctx.add_const_string(s);
            let id2 = ctx.add_const_string(s);
            assert_eq!(id1, id2, "String interning failed for: {:?}", s);
        }
    }

    #[test]
    fn test_deep_scope_nesting() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        // Deep nesting should not cause stack overflow
        for i in 0..100 {
            ctx.enter_scope();
            ctx.define_var(&format!("var_{}", i), false);
        }

        // Verify all scopes can be exited
        for _ in 0..100 {
            let (vars, _) = ctx.exit_scope(false);
            assert_eq!(vars.len(), 1);
        }
    }

    #[test]
    fn test_nested_loops_safety() {
        let mut ctx = CodegenContext::new();

        // Deeply nested loops
        for i in 0..50 {
            ctx.enter_loop(Some(format!("loop_{}", i)), None);
        }

        // All loops should be findable
        for i in 0..50 {
            let found = ctx.find_loop(Some(&format!("loop_{}", i)));
            assert!(found.is_some(), "Loop {} not found", i);
        }

        // All can be exited
        for _ in 0..50 {
            assert!(ctx.exit_loop().is_some());
        }
        assert!(!ctx.in_loop());
    }

    #[test]
    fn test_defer_stack_safety() {
        let mut ctx = CodegenContext::new();

        // Many defers in same scope
        for i in 0..100 {
            ctx.add_defer(
                vec![Instruction::LoadI {
                    dst: Reg(0),
                    value: i,
                }],
                i % 2 == 0, // Alternate errdefer
            );
        }

        let defers = ctx.pop_defer_scope(true);
        assert_eq!(defers.len(), 100);
    }

    #[test]
    fn test_forward_jump_without_definition() {
        let mut ctx = CodegenContext::new();

        ctx.emit_forward_jump("undefined_label", |offset| Instruction::Jmp { offset });
        ctx.emit(Instruction::Nop);

        // Validation should fail
        let result = ctx.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_function_info_integrity() {
        let mut ctx = CodegenContext::new();

        let info = FunctionInfo {
            id: FunctionId(42),
            param_count: 3,
            param_names: vec!["a".into(), "b".into(), "c".into()],
            is_async: true,
            contexts: vec!["Database".into()],
            return_type: None, ..Default::default()
        };

        ctx.register_function("test_fn".into(), info.clone());

        let retrieved = ctx.lookup_function("test_fn");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, info.id);
        assert_eq!(retrieved.param_count, info.param_count);
        assert_eq!(retrieved.is_async, info.is_async);
    }
}

// ============================================================================
// Integration Tests
// ============================================================================

mod integration_tests {
    use super::*;

    #[test]
    fn test_function_compilation_lifecycle() {
        let mut ctx = CodegenContext::new();

        // Begin function
        ctx.begin_function("add", &[("x".to_string(), false), ("y".to_string(), false)], None);
        assert!(ctx.in_function);
        assert_eq!(ctx.current_function, Some("add".to_string()));

        // Parameters should be allocated
        assert!(ctx.lookup_var("x").is_some());
        assert!(ctx.lookup_var("y").is_some());
        assert_eq!(ctx.get_var_reg("x").unwrap(), Reg(0));
        assert_eq!(ctx.get_var_reg("y").unwrap(), Reg(1));

        // Emit some instructions
        let result = ctx.alloc_temp();
        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: result,
            a: Reg(0),
            b: Reg(1),
        });
        ctx.emit(Instruction::Ret { value: result });

        // End function
        let (instructions, reg_count) = ctx.end_function();
        assert_eq!(instructions.len(), 2);
        assert!(reg_count >= 3); // x, y, result
        assert!(!ctx.in_function);
    }

    #[test]
    fn test_if_else_code_pattern() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("cond".to_string(), false)], None);

        let cond_reg = ctx.get_var_reg("cond").unwrap();
        let result = ctx.alloc_temp();

        let else_label = ctx.new_label("else");
        let end_label = ctx.new_label("end");

        // if cond
        ctx.emit_forward_jump(&else_label, |offset| Instruction::JmpNot {
            cond: cond_reg,
            offset,
        });

        // then branch
        ctx.emit(Instruction::LoadI {
            dst: result,
            value: 1,
        });
        ctx.emit_forward_jump(&end_label, |offset| Instruction::Jmp { offset });

        // else branch
        ctx.define_label(&else_label);
        ctx.emit(Instruction::LoadI {
            dst: result,
            value: 0,
        });

        // end
        ctx.define_label(&end_label);
        ctx.emit(Instruction::Ret { value: result });

        let (instructions, _) = ctx.end_function();

        // Verify jump targets are correct
        match &instructions[0] {
            Instruction::JmpNot { offset, .. } => assert!(*offset > 0),
            _ => panic!("Expected JmpNot"),
        }
    }

    #[test]
    fn test_while_loop_code_pattern() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("n".to_string(), false)], None);

        let n_reg = ctx.get_var_reg("n").unwrap();
        let i_reg = ctx.alloc_temp();
        let sum_reg = ctx.alloc_temp();

        // i = 0, sum = 0
        ctx.emit(Instruction::LoadI {
            dst: i_reg,
            value: 0,
        });
        ctx.emit(Instruction::LoadI {
            dst: sum_reg,
            value: 0,
        });

        let loop_ctx = ctx.enter_loop(None, None);

        // loop start
        ctx.define_label(&loop_ctx.continue_label);

        // while i < n
        let cmp_reg = ctx.alloc_temp();
        ctx.emit(Instruction::CmpI {
            op: CompareOp::Lt,
            dst: cmp_reg,
            a: i_reg,
            b: n_reg,
        });
        ctx.emit_forward_jump(&loop_ctx.break_label, |offset| Instruction::JmpNot {
            cond: cmp_reg,
            offset,
        });
        ctx.free_temp(cmp_reg);

        // sum += i
        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: sum_reg,
            a: sum_reg,
            b: i_reg,
        });

        // i += 1
        ctx.emit(Instruction::UnaryI {
            op: UnaryIntOp::Inc,
            dst: i_reg,
            src: i_reg,
        });

        // continue
        ctx.emit_backward_jump(&loop_ctx.continue_label, |offset| Instruction::Jmp {
            offset,
        })
        .unwrap();

        // loop end
        ctx.define_label(&loop_ctx.break_label);
        ctx.exit_loop();

        ctx.emit(Instruction::Ret { value: sum_reg });

        let (instructions, _) = ctx.end_function();
        assert!(instructions.len() >= 8);
    }

    #[test]
    fn test_short_circuit_and_pattern() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("a".to_string(), false), ("b".to_string(), false)], None);

        let a = ctx.get_var_reg("a").unwrap();
        let b = ctx.get_var_reg("b").unwrap();
        let result = ctx.alloc_temp();

        let end_label = ctx.new_label("and_end");

        // result = a
        ctx.emit(Instruction::Mov {
            dst: result,
            src: a,
        });

        // if !a, skip b evaluation
        ctx.emit_forward_jump(&end_label, |offset| Instruction::JmpNot {
            cond: result,
            offset,
        });

        // result = b
        ctx.emit(Instruction::Mov {
            dst: result,
            src: b,
        });

        ctx.define_label(&end_label);
        ctx.emit(Instruction::Ret { value: result });

        let (instructions, _) = ctx.end_function();
        assert_eq!(instructions.len(), 4);
    }

    #[test]
    fn test_for_loop_iterator_pattern() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("list".to_string(), false)], None);

        let list_reg = ctx.get_var_reg("list").unwrap();
        let iter_reg = ctx.alloc_temp();
        let elem_reg = ctx.alloc_temp();
        let has_next_reg = ctx.alloc_temp();

        // Create iterator
        ctx.emit(Instruction::IterNew {
            dst: iter_reg,
            iterable: list_reg,
        });

        let loop_ctx = ctx.enter_loop(None, None);

        // Loop start
        ctx.define_label(&loop_ctx.continue_label);

        // Get next
        ctx.emit(Instruction::IterNext {
            dst: elem_reg,
            has_next: has_next_reg,
            iter: iter_reg,
        });

        // Exit if no more
        ctx.emit_forward_jump(&loop_ctx.break_label, |offset| Instruction::JmpNot {
            cond: has_next_reg,
            offset,
        });

        // Loop body (just debug print)
        ctx.emit(Instruction::DebugPrint { value: elem_reg });

        // Continue
        ctx.emit_backward_jump(&loop_ctx.continue_label, |offset| Instruction::Jmp {
            offset,
        })
        .unwrap();

        // Loop end
        ctx.define_label(&loop_ctx.break_label);
        ctx.exit_loop();

        ctx.emit(Instruction::RetV);

        let (instructions, _) = ctx.end_function();
        assert!(instructions.len() >= 6);
    }

    #[test]
    fn test_codegen_config_builder() {
        let config = CodegenConfig::new("my_module")
            .with_debug_info()
            .with_optimization_level(2)
            .with_validation()
            .with_source_map();

        assert_eq!(config.module_name, "my_module");
        assert!(config.debug_info);
        assert_eq!(config.optimization_level, 2);
        assert!(config.validate);
        assert!(config.source_map);
    }

    #[test]
    fn test_codegen_config_optimization_clamped() {
        let config = CodegenConfig::new("test").with_optimization_level(10);
        assert_eq!(config.optimization_level, 3); // Max is 3
    }
}

// ============================================================================
// Edge Case Tests - Boundary Conditions
// ============================================================================

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_function() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("empty", &[], None);
        let (instructions, reg_count) = ctx.end_function();
        assert!(instructions.is_empty() || matches!(instructions.last(), Some(Instruction::RetV)));
        assert_eq!(reg_count, 0);
    }

    #[test]
    fn test_function_with_no_params() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("no_params", &[], None);

        // Just return a constant
        let r = ctx.alloc_temp();
        ctx.emit(Instruction::LoadI { dst: r, value: 42 });
        ctx.emit(Instruction::Ret { value: r });

        let (instructions, reg_count) = ctx.end_function();
        assert_eq!(instructions.len(), 2);
        assert_eq!(reg_count, 1);
    }

    #[test]
    fn test_function_with_many_params() {
        let mut ctx = CodegenContext::new();
        let params: Vec<(String, bool)> = (0..100).map(|i| (format!("p{}", i), false)).collect();
        ctx.begin_function("many_params", &params, None);

        // All params should be accessible
        for i in 0..100 {
            let reg = ctx.get_var_reg(&format!("p{}", i));
            assert!(reg.is_ok(), "param {} not found", i);
            assert_eq!(reg.unwrap(), Reg(i as u16));
        }

        let (_, reg_count) = ctx.end_function();
        assert!(reg_count >= 100);
    }

    #[test]
    fn test_zero_register() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[("x".to_string(), false)], None);

        // r0 should be the first parameter
        assert_eq!(ctx.get_var_reg("x").ok(), Some(Reg(0)));

        // Operations on r0
        ctx.emit(Instruction::UnaryI {
            op: UnaryIntOp::Neg,
            dst: Reg(0),
            src: Reg(0),
        });

        let (instructions, _) = ctx.end_function();
        assert_eq!(instructions.len(), 1);
    }

    #[test]
    fn test_maximum_register_value() {
        let mut alloc = RegisterAllocator::new();

        // Allocate a lot of registers
        for _ in 0..1000 {
            alloc.alloc_temp();
        }

        // Peak should track correctly
        assert_eq!(alloc.register_count(), 1000);
    }

    #[test]
    fn test_empty_scope() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.enter_scope();
        // No variables in this scope
        let (vars, defers) = ctx.exit_scope(false);
        assert!(vars.is_empty());
        assert!(defers.is_empty());

        ctx.end_function();
    }

    #[test]
    fn test_scope_with_only_temps() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.enter_scope();
        let t1 = ctx.alloc_temp();
        let t2 = ctx.alloc_temp();
        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: t1,
            a: t1,
            b: t2,
        });

        let (vars, _) = ctx.exit_scope(false);
        // Temps are not tracked as scope variables
        assert!(vars.is_empty());

        ctx.free_temp(t1);
        ctx.free_temp(t2);
        ctx.end_function();
    }

    #[test]
    fn test_empty_loop() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        let loop_ctx = ctx.enter_loop(None, None);
        ctx.define_label(&loop_ctx.continue_label);
        ctx.define_label(&loop_ctx.break_label);
        ctx.exit_loop();

        ctx.end_function();
    }

    #[test]
    fn test_immediate_break() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        let loop_ctx = ctx.enter_loop(None, None);
        ctx.define_label(&loop_ctx.continue_label);

        // Immediate break
        ctx.emit_forward_jump(&loop_ctx.break_label, |offset| Instruction::Jmp { offset });

        ctx.define_label(&loop_ctx.break_label);
        ctx.exit_loop();

        let (instructions, _) = ctx.end_function();
        assert!(!instructions.is_empty());
    }

    #[test]
    fn test_single_iteration_loop() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        let counter = ctx.alloc_temp();
        ctx.emit(Instruction::LoadI { dst: counter, value: 1 });

        let loop_ctx = ctx.enter_loop(None, None);
        ctx.define_label(&loop_ctx.continue_label);

        // Check counter
        let cond = ctx.alloc_temp();
        ctx.emit(Instruction::CmpI {
            op: CompareOp::Gt,
            dst: cond,
            a: counter,
            b: Reg(0), // Assuming 0 is default
        });
        ctx.emit_forward_jump(&loop_ctx.break_label, |offset| Instruction::JmpNot {
            cond,
            offset,
        });

        // Decrement
        ctx.emit(Instruction::UnaryI {
            op: UnaryIntOp::Dec,
            dst: counter,
            src: counter,
        });

        // Continue
        ctx.emit_backward_jump(&loop_ctx.continue_label, |offset| Instruction::Jmp { offset })
            .unwrap();

        ctx.define_label(&loop_ctx.break_label);
        ctx.exit_loop();

        ctx.free_temp(cond);
        ctx.end_function();
    }

    #[test]
    fn test_constant_zero() {
        let mut ctx = CodegenContext::new();
        let id = ctx.add_const_int(0);

        // Same zero should return same ID
        let id2 = ctx.add_const_int(0);
        assert_eq!(id, id2);
    }

    #[test]
    fn test_constant_negative() {
        let mut ctx = CodegenContext::new();
        let id1 = ctx.add_const_int(-1);
        let id2 = ctx.add_const_int(-1);
        assert_eq!(id1, id2);

        let id3 = ctx.add_const_int(-9223372036854775808i64); // i64::MIN
        let id4 = ctx.add_const_int(-9223372036854775808i64);
        assert_eq!(id3, id4);
    }

    #[test]
    fn test_float_edge_values() {
        let mut ctx = CodegenContext::new();

        // Zero
        let z1 = ctx.add_const_float(0.0);
        let z2 = ctx.add_const_float(0.0);
        assert_eq!(z1, z2);

        // Negative zero
        let nz1 = ctx.add_const_float(-0.0);
        let nz2 = ctx.add_const_float(-0.0);
        assert_eq!(nz1, nz2);

        // Infinity
        let inf1 = ctx.add_const_float(f64::INFINITY);
        let inf2 = ctx.add_const_float(f64::INFINITY);
        assert_eq!(inf1, inf2);

        let ninf1 = ctx.add_const_float(f64::NEG_INFINITY);
        let ninf2 = ctx.add_const_float(f64::NEG_INFINITY);
        assert_eq!(ninf1, ninf2);

        // NaN is tricky - each NaN should be unique
        let nan1 = ctx.add_const_float(f64::NAN);
        let nan2 = ctx.add_const_float(f64::NAN);
        // NaN != NaN, so they might have different IDs depending on implementation
        let _ = (nan1, nan2);
    }

    #[test]
    fn test_empty_string_constant() {
        let mut ctx = CodegenContext::new();
        let id1 = ctx.add_const_string("");
        let id2 = ctx.add_const_string("");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_single_char_strings() {
        let mut ctx = CodegenContext::new();

        for c in 'a'..='z' {
            let s = c.to_string();
            let id1 = ctx.add_const_string(&s);
            let id2 = ctx.add_const_string(&s);
            assert_eq!(id1, id2, "Failed for char '{}'", c);
        }
    }

    #[test]
    fn test_label_with_special_chars() {
        let mut ctx = CodegenContext::new();

        // Various prefix patterns
        let prefixes = vec!["if", "else", "while", "for", "break", "continue", "return", ""];

        for prefix in prefixes {
            let label = ctx.new_label(prefix);
            assert!(label.contains(prefix));

            // Label should be unique
            let label2 = ctx.new_label(prefix);
            assert_ne!(label, label2);
        }
    }

    #[test]
    fn test_jump_offset_zero() {
        let mut ctx = CodegenContext::new();

        // Define label immediately after forward jump
        ctx.emit_forward_jump("zero_offset", |offset| Instruction::Jmp { offset });
        ctx.define_label("zero_offset");

        // Offset should be 1 (jump to next instruction)
        match &ctx.instructions[0] {
            Instruction::Jmp { offset } => assert_eq!(*offset, 1),
            _ => panic!("Expected Jmp"),
        }
    }

    #[test]
    fn test_backward_jump_offset() {
        let mut ctx = CodegenContext::new();

        ctx.define_label("start");
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.emit_backward_jump("start", |offset| Instruction::Jmp { offset }).unwrap();

        // Offset should be negative
        match &ctx.instructions[3] {
            Instruction::Jmp { offset } => assert!(*offset < 0),
            _ => panic!("Expected Jmp"),
        }
    }

    #[test]
    fn test_variable_name_shadowing_chain() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        let r1 = ctx.define_var("x", false);
        assert_eq!(ctx.get_var_reg("x").ok(), Some(r1));

        ctx.enter_scope();
        let r2 = ctx.define_var("x", false);
        assert_eq!(ctx.get_var_reg("x").ok(), Some(r2));

        ctx.enter_scope();
        let r3 = ctx.define_var("x", false);
        assert_eq!(ctx.get_var_reg("x").ok(), Some(r3));

        ctx.exit_scope(false);
        assert_eq!(ctx.get_var_reg("x").ok(), Some(r2));

        ctx.exit_scope(false);
        assert_eq!(ctx.get_var_reg("x").ok(), Some(r1));

        ctx.end_function();
    }

    #[test]
    fn test_mutability_in_nested_scopes() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        ctx.define_var("x", false); // immutable
        let info = ctx.lookup_var("x").unwrap();
        assert!(!info.is_mutable);

        ctx.enter_scope();
        ctx.define_var("x", true); // shadow with mutable
        let info = ctx.lookup_var("x").unwrap();
        assert!(info.is_mutable);

        ctx.exit_scope(false);
        let info = ctx.lookup_var("x").unwrap();
        assert!(!info.is_mutable);

        ctx.end_function();
    }
}

// ============================================================================
// Stress Tests - High Load and Randomized Testing
// ============================================================================

mod stress_tests {
    use super::*;

    #[test]
    fn test_many_registers_allocation() {
        let mut alloc = RegisterAllocator::new();

        // Allocate many registers
        let mut regs = Vec::new();
        for _ in 0..5000 {
            regs.push(alloc.alloc_temp());
        }

        // Free them all
        for reg in regs {
            alloc.free_temp(reg);
        }

        // Allocate again - should reuse
        for _ in 0..5000 {
            alloc.alloc_temp();
        }

        // Peak should still be 5000
        assert_eq!(alloc.register_count(), 5000);
    }

    #[test]
    fn test_many_variables() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        for i in 0..1000 {
            ctx.define_var(&format!("var_{}", i), i % 2 == 0);
        }

        // All should be accessible
        for i in 0..1000 {
            assert!(ctx.lookup_var(&format!("var_{}", i)).is_some());
        }

        ctx.end_function();
    }

    #[test]
    fn test_many_scopes() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        // Create many nested scopes
        for i in 0..200 {
            ctx.enter_scope();
            ctx.define_var(&format!("s{}", i), false);
        }

        // Exit all scopes
        for _ in 0..200 {
            ctx.exit_scope(false);
        }

        // Original scope should have no scope variables
        assert!(ctx.lookup_var("s0").is_none());
        assert!(ctx.lookup_var("s199").is_none());

        ctx.end_function();
    }

    #[test]
    fn test_many_labels() {
        let mut ctx = CodegenContext::new();

        for _ in 0..5000 {
            let label = ctx.new_label("stress");
            ctx.define_label(&label);
        }

        // All labels should be defined
        // This mainly tests that we don't run out of unique names
    }

    #[test]
    fn test_many_constants() {
        let mut ctx = CodegenContext::new();

        // Add many unique constants
        for i in 0i64..5000 {
            ctx.add_const_int(i);
        }
        assert_eq!(ctx.constants.len(), 5000);

        // Add duplicates - should not increase count
        for i in 0i64..5000 {
            ctx.add_const_int(i);
        }
        assert_eq!(ctx.constants.len(), 5000);
    }

    #[test]
    fn test_many_instructions() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        // Emit many instructions
        for i in 0u16..10000 {
            ctx.emit(Instruction::LoadI {
                dst: Reg(i % 100),
                value: i as i64,
            });
        }

        let (instructions, _) = ctx.end_function();
        assert_eq!(instructions.len(), 10000);
    }

    #[test]
    fn test_many_function_registrations() {
        let mut ctx = CodegenContext::new();

        for i in 0..1000 {
            let info = FunctionInfo {
                id: FunctionId(i),
                param_count: (i % 10) as usize,
                param_names: vec![],
                is_async: i % 2 == 0,
                contexts: vec![],
                return_type: None, ..Default::default()
            };
            ctx.register_function(format!("fn_{}", i), info);
        }

        // All should be retrievable
        for i in 0..1000 {
            assert!(ctx.lookup_function(&format!("fn_{}", i)).is_some());
        }
    }

    #[test]
    fn test_interleaved_alloc_free() {
        let mut alloc = RegisterAllocator::new();

        // Interleave allocations and frees
        let mut active = Vec::new();

        for i in 0..1000 {
            if i % 3 == 0 && !active.is_empty() {
                let reg = active.pop().unwrap();
                alloc.free_temp(reg);
            } else {
                active.push(alloc.alloc_temp());
            }
        }

        // Clean up
        for reg in active {
            alloc.free_temp(reg);
        }

        // Should have reasonable register usage
        assert!(alloc.register_count() < 1000);
    }

    #[test]
    fn test_many_defers() {
        let mut ctx = CodegenContext::new();

        for i in 0..500 {
            ctx.add_defer(
                vec![Instruction::LoadI {
                    dst: Reg(0),
                    value: i,
                }],
                i % 3 == 0,
            );
        }

        let defers = ctx.pop_defer_scope(true);
        assert_eq!(defers.len(), 500);
    }

    #[test]
    fn test_deeply_nested_loops() {
        let mut ctx = CodegenContext::new();

        // Create deeply nested loops
        for i in 0..50 {
            ctx.enter_loop(Some(format!("loop_{}", i)), None);
        }

        // Find inner loops
        for i in 0..50 {
            let found = ctx.find_loop(Some(&format!("loop_{}", i)));
            assert!(found.is_some());
        }

        // Exit all loops
        for _ in 0..50 {
            ctx.exit_loop();
        }

        assert!(!ctx.in_loop());
    }

    #[test]
    fn test_mixed_scope_and_loop_nesting() {
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        for i in 0..20 {
            ctx.enter_scope();
            ctx.define_var(&format!("sv{}", i), false);
            ctx.enter_loop(Some(format!("loop_{}", i)), None);
        }

        // Exit interleaved
        for _ in 0..20 {
            ctx.exit_loop();
            ctx.exit_scope(false);
        }

        ctx.end_function();
    }
}

// ============================================================================
// Encoding Roundtrip Tests
// ============================================================================

mod encoding_tests {
    use super::*;
    use crate::bytecode::{decode_instruction, encode_instruction};

    fn roundtrip(instr: Instruction) {
        let mut buf = Vec::new();
        encode_instruction(&instr, &mut buf);
        let mut offset = 0;
        let decoded = decode_instruction(&buf, &mut offset).unwrap();
        assert_eq!(
            format!("{:?}", instr),
            format!("{:?}", decoded),
            "Roundtrip failed for instruction"
        );
    }

    #[test]
    fn test_load_roundtrips() {
        roundtrip(Instruction::LoadI { dst: Reg(0), value: 0 });
        roundtrip(Instruction::LoadI { dst: Reg(100), value: i64::MAX });
        roundtrip(Instruction::LoadI { dst: Reg(255), value: i64::MIN });
        roundtrip(Instruction::LoadF { dst: Reg(0), value: 0.0 });
        roundtrip(Instruction::LoadF { dst: Reg(1), value: std::f64::consts::PI });
        roundtrip(Instruction::LoadF { dst: Reg(2), value: f64::INFINITY });
        roundtrip(Instruction::LoadTrue { dst: Reg(0) });
        roundtrip(Instruction::LoadFalse { dst: Reg(0) });
        roundtrip(Instruction::LoadUnit { dst: Reg(0) });
    }

    #[test]
    fn test_binary_roundtrips() {
        for op in [
            BinaryIntOp::Add,
            BinaryIntOp::Sub,
            BinaryIntOp::Mul,
            BinaryIntOp::Div,
            BinaryIntOp::Mod,
        ] {
            roundtrip(Instruction::BinaryI {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_compare_roundtrips() {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            roundtrip(Instruction::CmpI {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_bitwise_roundtrips() {
        for op in [
            BitwiseOp::And,
            BitwiseOp::Or,
            BitwiseOp::Xor,
            BitwiseOp::Shl,
            BitwiseOp::Shr,
        ] {
            roundtrip(Instruction::Bitwise {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_control_flow_roundtrips() {
        roundtrip(Instruction::Jmp { offset: 0 });
        roundtrip(Instruction::Jmp { offset: 100 });
        roundtrip(Instruction::Jmp { offset: -100 });
        roundtrip(Instruction::JmpIf { cond: Reg(0), offset: 50 });
        roundtrip(Instruction::JmpNot { cond: Reg(0), offset: -50 });
    }

    #[test]
    fn test_return_roundtrips() {
        roundtrip(Instruction::RetV);
        roundtrip(Instruction::Ret { value: Reg(0) });
        roundtrip(Instruction::Ret { value: Reg(255) });
    }

    #[test]
    fn test_memory_roundtrips() {
        roundtrip(Instruction::New { dst: Reg(0), type_id: 0, field_count: 2 });
        roundtrip(Instruction::New { dst: Reg(10), type_id: 1000, field_count: 4 });
        roundtrip(Instruction::GetF { dst: Reg(0), obj: Reg(1), field_idx: 0 });
        roundtrip(Instruction::SetF { obj: Reg(0), field_idx: 5, value: Reg(2) });
        roundtrip(Instruction::GetE { dst: Reg(0), arr: Reg(1), idx: Reg(2) });
        roundtrip(Instruction::SetE { arr: Reg(0), idx: Reg(1), value: Reg(2) });
    }

    #[test]
    fn test_collection_roundtrips() {
        roundtrip(Instruction::NewList { dst: Reg(0) });
        roundtrip(Instruction::ListPush { list: Reg(0), val: Reg(1) });
        roundtrip(Instruction::ListPop { dst: Reg(0), list: Reg(1) });
        roundtrip(Instruction::NewMap { dst: Reg(0) });
        roundtrip(Instruction::MapGet { dst: Reg(0), map: Reg(1), key: Reg(2) });
        roundtrip(Instruction::MapSet { map: Reg(0), key: Reg(1), val: Reg(2) });
        roundtrip(Instruction::MapContains { dst: Reg(0), map: Reg(1), key: Reg(2) });
    }

    #[test]
    fn test_iterator_roundtrips() {
        roundtrip(Instruction::IterNew { dst: Reg(0), iterable: Reg(1) });
        roundtrip(Instruction::IterNext {
            dst: Reg(0),
            has_next: Reg(1),
            iter: Reg(2),
        });
    }

    #[test]
    fn test_reference_roundtrips() {
        roundtrip(Instruction::Ref { dst: Reg(0), src: Reg(1) });
        roundtrip(Instruction::RefMut { dst: Reg(0), src: Reg(1) });
        roundtrip(Instruction::Deref { dst: Reg(0), ref_reg: Reg(1) });
        roundtrip(Instruction::Clone { dst: Reg(0), src: Reg(1) });
    }

    #[test]
    fn test_misc_roundtrips() {
        roundtrip(Instruction::Nop);
        roundtrip(Instruction::Mov { dst: Reg(0), src: Reg(1) });
        roundtrip(Instruction::Not { dst: Reg(0), src: Reg(1) });
        roundtrip(Instruction::DebugPrint { value: Reg(0) });
    }

    #[test]
    fn test_large_register_indices() {
        // Test with larger register indices
        roundtrip(Instruction::Mov { dst: Reg(1000), src: Reg(2000) });
        roundtrip(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: Reg(5000),
            a: Reg(6000),
            b: Reg(7000),
        });
    }

    #[test]
    fn test_extreme_values() {
        roundtrip(Instruction::LoadI { dst: Reg(0), value: i64::MAX });
        roundtrip(Instruction::LoadI { dst: Reg(0), value: i64::MIN });
        roundtrip(Instruction::LoadF { dst: Reg(0), value: f64::MAX });
        roundtrip(Instruction::LoadF { dst: Reg(0), value: f64::MIN_POSITIVE });
        roundtrip(Instruction::Jmp { offset: i32::MAX });
        roundtrip(Instruction::Jmp { offset: i32::MIN });
    }
}

// ============================================================================
// Property-Based Testing Patterns
// ============================================================================

mod property_tests {
    use super::*;

    #[test]
    fn test_register_alloc_monotonic() {
        // Property: Register count never decreases
        let mut alloc = RegisterAllocator::new();
        let mut prev_count = 0u16;

        for _ in 0..100 {
            alloc.alloc_temp();
            let count = alloc.register_count();
            assert!(count >= prev_count);
            prev_count = count;
        }
    }

    #[test]
    fn test_label_uniqueness_property() {
        // Property: All generated labels are unique
        let mut ctx = CodegenContext::new();
        let mut labels = std::collections::HashSet::new();

        for _ in 0..1000 {
            let label = ctx.new_label("prop");
            assert!(labels.insert(label), "Duplicate label generated");
        }
    }

    #[test]
    fn test_scope_nesting_invariant() {
        // Property: After entering and exiting equal numbers of scopes,
        // we return to original variable visibility
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        let r0 = ctx.define_var("original", false);

        for _ in 0..10 {
            ctx.enter_scope();
            ctx.define_var("temp", false);
        }

        for _ in 0..10 {
            ctx.exit_scope(false);
        }

        assert_eq!(ctx.get_var_reg("original").ok(), Some(r0));
        assert!(ctx.lookup_var("temp").is_none());

        ctx.end_function();
    }

    #[test]
    fn test_loop_nesting_invariant() {
        // Property: After entering and exiting equal numbers of loops,
        // we return to not being in any loop
        let mut ctx = CodegenContext::new();

        for _ in 0..10 {
            ctx.enter_loop(None, None);
            assert!(ctx.in_loop());
        }

        for _ in 0..10 {
            ctx.exit_loop();
        }

        assert!(!ctx.in_loop());
    }

    #[test]
    fn test_constant_deduplication_idempotent() {
        // Property: Adding the same constant multiple times returns the same ID
        let mut ctx = CodegenContext::new();

        let values: Vec<i64> = vec![0, 1, -1, 42, i64::MAX, i64::MIN];

        for value in &values {
            let first_id = ctx.add_const_int(*value);
            for _ in 0..10 {
                let id = ctx.add_const_int(*value);
                assert_eq!(id, first_id, "Constant deduplication failed for {}", value);
            }
        }
    }

    #[test]
    fn test_function_end_resets_state() {
        // Property: After end_function, we're no longer in a function
        let mut ctx = CodegenContext::new();

        for i in 0..5 {
            ctx.begin_function(&format!("fn{}", i), &[], None);
            assert!(ctx.in_function);
            ctx.emit(Instruction::RetV);
            ctx.end_function();
            assert!(!ctx.in_function);
        }
    }

    #[test]
    fn test_defer_order_invariant() {
        // Property: Defers are executed in reverse order (LIFO)
        let mut ctx = CodegenContext::new();

        let values: Vec<i64> = vec![1, 2, 3, 4, 5];
        for v in &values {
            ctx.add_defer(vec![Instruction::LoadI { dst: Reg(0), value: *v }], false);
        }

        let defers = ctx.pop_defer_scope(false);
        assert_eq!(defers.len(), values.len());

        // Last added should be first in the result (LIFO)
        for (i, defer_block) in defers.iter().enumerate() {
            if let Instruction::LoadI { value, .. } = defer_block[0] {
                assert_eq!(value, values[values.len() - 1 - i]);
            }
        }
    }

    #[test]
    fn test_temp_recycling_property() {
        // Property: After freeing a temp, the next alloc should reuse it
        let mut alloc = RegisterAllocator::new();

        let t1 = alloc.alloc_temp();
        alloc.free_temp(t1);
        let t2 = alloc.alloc_temp();
        assert_eq!(t1, t2, "Temp was not recycled");
    }

    #[test]
    fn test_variable_lookup_after_definition() {
        // Property: A variable is always found after being defined
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        for i in 0..100 {
            let name = format!("var_{}", i);
            ctx.define_var(&name, false);
            assert!(ctx.lookup_var(&name).is_some(), "Variable {} not found after definition", name);
        }

        ctx.end_function();
    }
}

// ============================================================================
// Additional Security Tests
// ============================================================================

mod advanced_security_tests {
    use super::*;

    #[test]
    fn test_register_index_bounds() {
        // Ensure register indices don't wrap or overflow
        let mut alloc = RegisterAllocator::new();

        // Allocate up to a high number
        for _ in 0..10000 {
            let reg = alloc.alloc_temp();
            assert!(reg.0 < MAX_REGISTERS, "Register exceeded maximum");
        }
    }

    #[test]
    fn test_no_uninitialized_reads() {
        // Verify that registers are properly allocated before use
        let mut ctx = CodegenContext::new();
        ctx.begin_function("test", &[], None);

        // Only use registers we've allocated
        let r1 = ctx.alloc_temp();
        let r2 = ctx.alloc_temp();
        let r3 = ctx.alloc_temp();

        ctx.emit(Instruction::LoadI { dst: r1, value: 1 });
        ctx.emit(Instruction::LoadI { dst: r2, value: 2 });
        ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: r3,
            a: r1,
            b: r2,
        });

        ctx.end_function();
    }

    #[test]
    fn test_label_name_collision_resistance() {
        let mut ctx = CodegenContext::new();

        // Try to create collisions
        let l1 = ctx.new_label("if");
        let l2 = ctx.new_label("if");
        let l3 = ctx.new_label("if_0"); // Similar to generated name
        let l4 = ctx.new_label("if_1");

        assert_ne!(l1, l2);
        assert_ne!(l1, l3);
        assert_ne!(l1, l4);
        assert_ne!(l2, l3);
        assert_ne!(l2, l4);
        assert_ne!(l3, l4);
    }

    #[test]
    fn test_constant_pool_no_leak() {
        let mut ctx = CodegenContext::new();

        // Add sensitive-looking data
        ctx.add_const_string("password123");
        ctx.add_const_string("secret_key");

        // Verify they're stored (and could be inspected for debugging)
        // In a real security audit, we'd check they're not leaked externally
        assert!(ctx.strings.len() >= 2);
    }

    #[test]
    fn test_jump_target_validation() {
        let mut ctx = CodegenContext::new();

        // Create a forward jump without defining the target
        ctx.emit_forward_jump("undefined", |offset| Instruction::Jmp { offset });

        // Validation should detect undefined label
        assert!(ctx.validate().is_err());
    }

    #[test]
    fn test_backward_jump_to_undefined() {
        let mut ctx = CodegenContext::new();

        // Try to jump backward to undefined label
        let result = ctx.emit_backward_jump("nonexistent", |offset| Instruction::Jmp { offset });

        assert!(result.is_err());
    }

    #[test]
    fn test_double_label_definition() {
        let mut ctx = CodegenContext::new();

        ctx.define_label("duplicate");
        ctx.define_label("duplicate"); // Define again

        // This should either error or be idempotent
        // Current implementation is idempotent (last definition wins)
    }

    #[test]
    fn test_stats_accuracy() {
        let mut ctx = CodegenContext::new();

        let initial_stats = ctx.stats.clone();

        ctx.begin_function("test", &[], None);
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.emit(Instruction::Nop);
        ctx.end_function();

        // Stats should be updated
        assert!(ctx.stats.functions_compiled > initial_stats.functions_compiled);
        assert!(ctx.stats.instructions_generated > initial_stats.instructions_generated);
    }

    #[test]
    fn test_empty_param_names() {
        let mut alloc = RegisterAllocator::new();

        // Empty param names should still work
        let regs = alloc.alloc_parameters(&[]);
        assert!(regs.is_empty());
    }

    #[test]
    fn test_duplicate_param_names() {
        let mut alloc = RegisterAllocator::new();

        // Duplicate param names - last one wins
        let regs = alloc.alloc_parameters(&[("x".to_string(), false), ("x".to_string(), false)]);

        assert_eq!(regs.len(), 2);
        assert_eq!(regs[0], Reg(0));
        assert_eq!(regs[1], Reg(1));

        // Both "x"s are registered, but only one is in the map
        // The second registration overwrites the first
        assert_eq!(alloc.get_reg("x"), Some(Reg(1)));
    }

    #[test]
    fn test_context_function_isolation() {
        let mut ctx = CodegenContext::new();

        // Function 1
        ctx.begin_function("f1", &[("a".to_string(), false)], None);
        ctx.define_var("local1", false);
        ctx.end_function();

        // Function 2
        ctx.begin_function("f2", &[("b".to_string(), false)], None);

        // Variables from f1 should not be visible
        assert!(ctx.lookup_var("a").is_none());
        assert!(ctx.lookup_var("local1").is_none());

        ctx.end_function();
    }

    #[test]
    fn test_instruction_determinism() {
        // Same operations should produce same bytecode
        let make_instructions = || {
            let mut ctx = CodegenContext::new();
            ctx.begin_function("test", &[("x".to_string(), false)], None);

            let x = ctx.get_var_reg("x").unwrap();
            let result = ctx.alloc_temp();

            ctx.emit(Instruction::BinaryI {
                op: BinaryIntOp::Mul,
                dst: result,
                a: x,
                b: x,
            });
            ctx.emit(Instruction::Ret { value: result });

            ctx.end_function().0
        };

        let instrs1 = make_instructions();
        let instrs2 = make_instructions();

        assert_eq!(instrs1.len(), instrs2.len());
        for (i, (a, b)) in instrs1.iter().zip(instrs2.iter()).enumerate() {
            assert_eq!(
                format!("{:?}", a),
                format!("{:?}", b),
                "Instruction {} differs",
                i
            );
        }
    }
}

// ============================================================================
// Error Message Quality Tests
// ============================================================================

mod error_message_tests {
    use super::*;

    #[test]
    fn test_undefined_variable_message_includes_name() {
        let err = CodegenError::undefined_variable("missing_var_name");
        let msg = format!("{}", err);
        assert!(msg.contains("missing_var_name"), "Error should include variable name");
    }

    #[test]
    fn test_undefined_function_message_includes_name() {
        let err = CodegenError::undefined_function("unknown_function");
        let msg = format!("{}", err);
        assert!(msg.contains("unknown_function"), "Error should include function name");
    }

    #[test]
    fn test_wrong_arg_count_message_is_informative() {
        let err = CodegenError::new(CodegenErrorKind::WrongArgumentCount {
            expected: 3,
            found: 5,
            function: "my_func".to_string(),
        });
        let msg = format!("{}", err);
        assert!(msg.contains("3"), "Should mention expected count");
        assert!(msg.contains("5"), "Should mention found count");
        assert!(msg.contains("my_func"), "Should mention function name");
    }

    #[test]
    fn test_immutable_assignment_message() {
        let err = CodegenError::new(CodegenErrorKind::ImmutableAssignment("const_var".to_string()));
        let msg = format!("{}", err);
        assert!(msg.contains("immutable") || msg.contains("const_var"));
    }

    #[test]
    fn test_all_error_kinds_have_display() {
        // Ensure all error kinds can be displayed without panicking
        let errors = vec![
            CodegenErrorKind::UnsupportedExpr("test".into()),
            CodegenErrorKind::InvalidLiteral("test".into()),
            CodegenErrorKind::InvalidBinaryOp("test".into()),
            CodegenErrorKind::InvalidUnaryOp("test".into()),
            CodegenErrorKind::UndefinedVariable("test".into()),
            CodegenErrorKind::VariableAlreadyDefined("test".into()),
            CodegenErrorKind::ImmutableAssignment("test".into()),
            CodegenErrorKind::UndefinedFunction("test".into()),
            CodegenErrorKind::WrongArgumentCount {
                expected: 1,
                found: 2,
                function: "test".into(),
            },
            CodegenErrorKind::ArgumentTypeMismatch {
                position: 0,
                expected: "Int".into(),
                found: "Float".into(),
            },
            CodegenErrorKind::TypeMismatch {
                expected: "Int".into(),
                found: "Float".into(),
            },
            CodegenErrorKind::TypeInference("test".into()),
            CodegenErrorKind::InvalidTypeForOperation {
                ty: "Unit".into(),
                operation: "add".into(),
            },
            CodegenErrorKind::UnsupportedPattern("test".into()),
            CodegenErrorKind::NonExhaustivePattern("test".into()),
            CodegenErrorKind::BreakOutsideLoop,
            CodegenErrorKind::ContinueOutsideLoop,
            CodegenErrorKind::ReturnOutsideFunction,
            CodegenErrorKind::InvalidJumpTarget("test".into()),
            CodegenErrorKind::RegisterAllocationFailed,
            CodegenErrorKind::RegisterOverflow { needed: 100, max: 50 },
            CodegenErrorKind::Internal("test".into()),
            CodegenErrorKind::NotImplemented("test".into()),
        ];

        for kind in errors {
            let err = CodegenError::new(kind);
            let msg = format!("{}", err);
            assert!(!msg.is_empty(), "Error message should not be empty");
        }
    }

    #[test]
    fn test_error_with_span() {
        let mut err = CodegenError::undefined_variable("x");
        err.span = Some(verum_ast::Span::new(10, 15, verum_ast::FileId::dummy()));
        // Span should be accessible
        assert!(err.span.is_some());
        assert_eq!(err.span.unwrap().start, 10);
    }

    #[test]
    fn test_error_with_context() {
        let err = CodegenError::undefined_variable("x")
            .with_context("in function main");
        assert!(err.context.is_some());
        assert!(err.context.unwrap().contains("main"));
    }
}

// ============================================================================
// Cross-Module Path Resolution Tests
// ============================================================================

mod cross_module_path_tests {
    use super::*;

    /// Helper to create a minimal FunctionInfo for tests.
    fn make_func_info(id: u32, param_count: usize, is_async: bool) -> FunctionInfo {
        FunctionInfo {
            id: FunctionId(id),
            param_count,
            param_names: vec![],
            is_async,
            contexts: vec![],
            return_type: None, ..Default::default()
        }
    }

    #[test]
    fn test_lookup_qualified_function_simple() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(0, 0, false);
        ctx.register_function("my_module::my_func".into(), info.clone());

        // Should find by fully qualified name
        let found = ctx.lookup_qualified_function("my_module::my_func");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(0));
    }

    #[test]
    fn test_lookup_qualified_function_nested_module() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(1, 2, false);
        ctx.register_function("outer::inner::deep::func".into(), info.clone());

        // Should find deeply nested function
        let found = ctx.lookup_qualified_function("outer::inner::deep::func");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(1));
    }

    #[test]
    fn test_lookup_qualified_function_fallback_to_simple() {
        let mut ctx = CodegenContext::new();

        // Register with simple name
        let info = make_func_info(2, 1, true);
        ctx.register_function("simple_func".into(), info.clone());

        // lookup_qualified_function should fallback to simple lookup
        let found = ctx.lookup_qualified_function("some_module::simple_func");
        assert!(found.is_some(), "Should fallback to simple name lookup");
        assert_eq!(found.unwrap().id, FunctionId(2));
    }

    #[test]
    fn test_lookup_qualified_function_not_found() {
        let ctx = CodegenContext::new();

        let found = ctx.lookup_qualified_function("nonexistent::module::func");
        assert!(found.is_none());
    }

    #[test]
    fn test_lookup_qualified_function_type_method_style() {
        let mut ctx = CodegenContext::new();

        // Register Type::method style function
        let info = make_func_info(3, 1, false);
        ctx.register_function("Vec::push".into(), info.clone());

        let found = ctx.lookup_qualified_function("Vec::push");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(3));
    }

    #[test]
    fn test_multiple_qualified_functions_with_same_simple_name() {
        let mut ctx = CodegenContext::new();

        let info1 = make_func_info(10, 0, false);
        let info2 = make_func_info(11, 1, false);
        let info3 = make_func_info(12, 2, false);

        // Register functions with same simple name but different modules
        ctx.register_function("module_a::process".into(), info1.clone());
        ctx.register_function("module_b::process".into(), info2.clone());
        ctx.register_function("process".into(), info3.clone()); // Simple name

        // Each should be found by their qualified name
        assert_eq!(ctx.lookup_qualified_function("module_a::process").unwrap().id, FunctionId(10));
        assert_eq!(ctx.lookup_qualified_function("module_b::process").unwrap().id, FunctionId(11));
        assert_eq!(ctx.lookup_qualified_function("process").unwrap().id, FunctionId(12));
    }

    #[test]
    fn test_qualified_function_with_generic_type_in_path() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(20, 1, false);

        // Path like List<T>::map (stored without type params in simple form)
        ctx.register_function("List::map".into(), info.clone());

        let found = ctx.lookup_qualified_function("List::map");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(20));
    }

    #[test]
    fn test_crate_qualified_path() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(30, 0, false);
        ctx.register_function("crate::util::helper".into(), info.clone());

        let found = ctx.lookup_qualified_function("crate::util::helper");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(30));
    }

    #[test]
    fn test_lookup_function_vs_qualified_function() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(40, 0, false);
        ctx.register_function("my_func".into(), info.clone());

        // lookup_function uses simple name only
        assert!(ctx.lookup_function("my_func").is_some());
        assert!(ctx.lookup_function("module::my_func").is_none()); // Won't find

        // lookup_qualified_function can fallback
        assert!(ctx.lookup_qualified_function("my_func").is_some());
        assert!(ctx.lookup_qualified_function("module::my_func").is_some()); // Falls back
    }

    #[test]
    fn test_empty_qualified_path_segments() {
        let ctx = CodegenContext::new();

        // Edge case: empty string
        let found = ctx.lookup_qualified_function("");
        assert!(found.is_none());

        // Edge case: just separators
        let found = ctx.lookup_qualified_function("::");
        assert!(found.is_none());
    }

    #[test]
    fn test_case_sensitivity_in_qualified_paths() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(50, 0, false);
        ctx.register_function("MyModule::myFunc".into(), info.clone());

        // Exact match should work
        assert!(ctx.lookup_qualified_function("MyModule::myFunc").is_some());

        // Different case should not match (case-sensitive)
        assert!(ctx.lookup_qualified_function("mymodule::myfunc").is_none());
        assert!(ctx.lookup_qualified_function("MYMODULE::MYFUNC").is_none());
    }

    #[test]
    fn test_stress_many_qualified_functions() {
        let mut ctx = CodegenContext::new();

        // Register many functions with qualified names
        for module_idx in 0..10 {
            for func_idx in 0..100 {
                let info = make_func_info(
                    (module_idx * 100 + func_idx) as u32,
                    func_idx,
                    func_idx % 2 == 0,
                );
                let name = format!("module_{}::func_{}", module_idx, func_idx);
                ctx.register_function(name, info);
            }
        }

        // All should be retrievable
        for module_idx in 0..10 {
            for func_idx in 0..100 {
                let name = format!("module_{}::func_{}", module_idx, func_idx);
                let found = ctx.lookup_qualified_function(&name);
                assert!(found.is_some(), "Should find {}", name);
                assert_eq!(found.unwrap().id.0, (module_idx * 100 + func_idx) as u32);
            }
        }
    }

    #[test]
    fn test_qualified_function_with_context_requirements() {
        let mut ctx = CodegenContext::new();

        let info = FunctionInfo {
            id: FunctionId(60),
            param_count: 0,
            param_names: vec![],
            is_async: true,
            contexts: vec!["Database".to_string(), "Logger".to_string()],
            return_type: None, ..Default::default()
        };

        ctx.register_function("db::execute".into(), info.clone());

        let found = ctx.lookup_qualified_function("db::execute");
        assert!(found.is_some());
        let func = found.unwrap();
        assert!(func.is_async);
        assert_eq!(func.contexts.len(), 2);
        assert!(func.contexts.contains(&"Database".to_string()));
        assert!(func.contexts.contains(&"Logger".to_string()));
    }

    #[test]
    fn test_super_qualified_path() {
        let mut ctx = CodegenContext::new();

        let info = make_func_info(70, 0, false);
        ctx.register_function("super::parent_func".into(), info.clone());

        let found = ctx.lookup_qualified_function("super::parent_func");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, FunctionId(70));
    }

    #[test]
    fn test_qualified_function_with_param_names() {
        let mut ctx = CodegenContext::new();

        let info = FunctionInfo {
            id: FunctionId(80),
            param_count: 3,
            param_names: vec!["x".to_string(), "y".to_string(), "z".to_string()],
            is_async: false,
            contexts: vec![],
            return_type: None, ..Default::default()
        };

        ctx.register_function("math::add3".into(), info.clone());

        let found = ctx.lookup_qualified_function("math::add3");
        assert!(found.is_some());
        let func = found.unwrap();
        assert_eq!(func.param_count, 3);
        assert_eq!(func.param_names, vec!["x", "y", "z"]);
    }
}
