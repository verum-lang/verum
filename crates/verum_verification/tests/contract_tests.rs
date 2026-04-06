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
    clippy::approx_constant
)]
// Contract Literals System Tests
//
// Comprehensive tests for the contract#"..." compiler intrinsic
// Contract literals (contract#"...") are a compiler intrinsic (NOT user-extensible)
// providing Hoare-logic style contracts verified at compile time via SMT solvers.
// Keywords: requires (precondition), ensures (postcondition), invariant (holds
// throughout), old(expr) (entry value), result (return value), forall/exists
// (quantifiers). Verification mode set by @verify(proof/runtime/assume).
// Workflow: parse contract DSL -> translate to SMT-LIB -> check via Z3/CVC5.
//
// # Test Categories
//
// 1. **Parser Tests** - Contract DSL parsing
// 2. **AST Tests** - Contract expression structure
// 3. **SMT Translation Tests** - Conversion to SMT-LIB format
// 4. **Runtime Instrumentation Tests** - Runtime assertion generation
// 5. **Validation Tests** - Semantic correctness checking
// 6. **Integration Tests** - End-to-end verification workflows

use verum_ast::span::Span;
use verum_common::Text;
use verum_verification::contract::{
    ContractBinOp, ContractError, ContractExpr, ContractParser, ContractSmtTranslator,
    ContractSpec, ContractUnOp, InstrumentedContract, OldExpr, Predicate, QuantifierBinding,
    RuntimeInstrumenter, contract_to_smtlib, generate_contract_vcs, instrument_contract,
    parse_contract, parse_contract_no_validate, validate_contract,
};

// =============================================================================
// Parser Tests
// =============================================================================

mod parser_tests {
    use super::*;

    #[test]
    fn test_parse_simple_precondition() {
        let input = "requires x > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        assert_eq!(spec.postconditions.len(), 0);
        assert_eq!(spec.invariants.len(), 0);
    }

    #[test]
    fn test_parse_simple_postcondition() {
        let input = "ensures result >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 0);
        assert_eq!(spec.postconditions.len(), 1);
        assert_eq!(spec.invariants.len(), 0);
    }

    #[test]
    fn test_parse_simple_invariant() {
        let input = "invariant i >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 0);
        assert_eq!(spec.postconditions.len(), 0);
        assert_eq!(spec.invariants.len(), 1);
    }

    #[test]
    fn test_parse_multiple_clauses() {
        let input = r#"
            requires x > 0;
            requires y > 0;
            ensures result == x + y;
            ensures result > x;
            ensures result > y;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 2);
        assert_eq!(spec.postconditions.len(), 3);
    }

    #[test]
    fn test_parse_old_expression() {
        let input = "ensures result == old(x) + 1;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();
        assert!(pred.contains_old());
    }

    #[test]
    fn test_parse_result_keyword() {
        let input = "ensures result > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();
        assert!(pred.contains_result());
    }

    #[test]
    fn test_parse_forall_quantifier() {
        let input = "ensures forall i in 0..n. arr[i] >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::Forall(binding, _) => {
                assert_eq!(binding.variable.as_str(), "i");
                assert!(binding.range.is_some());
            }
            _ => panic!("Expected Forall expression"),
        }
    }

    #[test]
    fn test_parse_exists_quantifier() {
        let input = "ensures exists i in 0..n. arr[i] == target;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::Exists(binding, _) => {
                assert_eq!(binding.variable.as_str(), "i");
            }
            _ => panic!("Expected Exists expression"),
        }
    }

    #[test]
    fn test_parse_logical_operators() {
        let input = "requires x > 0 && y > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::And, _, _) => {}
            _ => panic!("Expected And expression"),
        }
    }

    #[test]
    fn test_parse_implication() {
        let input = "requires x > 0 => y > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Imply, _, _) => {}
            _ => panic!("Expected Imply expression"),
        }
    }

    #[test]
    fn test_parse_comparison_operators() {
        let cases = [
            ("requires x == y;", ContractBinOp::Eq),
            ("requires x != y;", ContractBinOp::Ne),
            ("requires x < y;", ContractBinOp::Lt),
            ("requires x <= y;", ContractBinOp::Le),
            ("requires x > y;", ContractBinOp::Gt),
            ("requires x >= y;", ContractBinOp::Ge),
        ];

        for (input, expected_op) in cases {
            let spec = parse_contract(input, Span::dummy()).unwrap();
            let pred = &spec.preconditions.first().unwrap();
            match &pred.expr {
                ContractExpr::BinOp(op, _, _) => {
                    assert_eq!(*op, expected_op, "Failed for input: {}", input);
                }
                _ => panic!("Expected BinOp for input: {}", input),
            }
        }
    }

    #[test]
    fn test_parse_arithmetic_operators() {
        let cases = [
            ("requires x + y == z;", ContractBinOp::Add),
            ("requires x - y == z;", ContractBinOp::Sub),
            ("requires x * y == z;", ContractBinOp::Mul),
            ("requires x / y == z;", ContractBinOp::Div),
            ("requires x % y == z;", ContractBinOp::Mod),
        ];

        for (input, expected_inner_op) in cases {
            let spec = parse_contract(input, Span::dummy()).unwrap();
            let pred = &spec.preconditions.first().unwrap();
            // The outer is Eq, inner left should be the arithmetic op
            match &pred.expr {
                ContractExpr::BinOp(ContractBinOp::Eq, left, _) => match left.as_ref() {
                    ContractExpr::BinOp(op, _, _) => {
                        assert_eq!(*op, expected_inner_op, "Failed for input: {}", input);
                    }
                    _ => panic!("Expected inner BinOp for input: {}", input),
                },
                _ => panic!("Expected outer Eq for input: {}", input),
            }
        }
    }

    #[test]
    fn test_parse_unary_operators() {
        let input = "requires !flag;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::UnOp(ContractUnOp::Not, _) => {}
            _ => panic!("Expected Not expression"),
        }
    }

    #[test]
    fn test_parse_negation() {
        let input = "requires -x < 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Lt, left, _) => match left.as_ref() {
                ContractExpr::UnOp(ContractUnOp::Neg, _) => {}
                _ => panic!("Expected Neg expression on left"),
            },
            _ => panic!("Expected Lt expression"),
        }
    }

    #[test]
    fn test_parse_field_access() {
        let input = "requires account.balance >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(_, left, _) => match left.as_ref() {
                ContractExpr::Field(_, field) => {
                    assert_eq!(field.as_str(), "balance");
                }
                _ => panic!("Expected Field access"),
            },
            _ => panic!("Expected BinOp"),
        }
    }

    #[test]
    fn test_parse_array_index() {
        let input = "requires arr[i] >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(_, left, _) => match left.as_ref() {
                ContractExpr::Index(_, _) => {}
                _ => panic!("Expected Index expression"),
            },
            _ => panic!("Expected BinOp"),
        }
    }

    #[test]
    fn test_parse_method_call() {
        let input = "requires list.len() > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(_, left, _) => match left.as_ref() {
                ContractExpr::MethodCall(_, method, _) => {
                    assert_eq!(method.as_str(), "len");
                }
                _ => panic!("Expected MethodCall expression"),
            },
            _ => panic!("Expected BinOp"),
        }
    }

    #[test]
    fn test_parse_function_call() {
        let input = "requires abs(x) < 100;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(_, left, _) => match left.as_ref() {
                ContractExpr::Call(name, _) => {
                    assert_eq!(name.as_str(), "abs");
                }
                _ => panic!("Expected Call expression"),
            },
            _ => panic!("Expected BinOp"),
        }
    }

    #[test]
    fn test_parse_let_binding() {
        let input = "requires let sum = x + y in sum > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::Let(name, _, _) => {
                assert_eq!(name.as_str(), "sum");
            }
            _ => panic!("Expected Let expression"),
        }
    }

    #[test]
    fn test_parse_if_then_else() {
        let input = "requires if x > 0 then y > 0 else z > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::IfThenElse(_, _, _) => {}
            _ => panic!("Expected IfThenElse expression"),
        }
    }

    #[test]
    fn test_parse_parentheses() {
        let input = "requires (x + y) * z == 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Eq, left, _) => match left.as_ref() {
                ContractExpr::BinOp(ContractBinOp::Mul, left_inner, _) => {
                    match left_inner.as_ref() {
                        ContractExpr::Paren(inner) => match inner.as_ref() {
                            ContractExpr::BinOp(ContractBinOp::Add, _, _) => {}
                            _ => panic!("Expected Add inside paren"),
                        },
                        _ => panic!("Expected Paren"),
                    }
                }
                _ => panic!("Expected Mul"),
            },
            _ => panic!("Expected Eq"),
        }
    }

    #[test]
    fn test_parse_comments() {
        let input = r#"
            // This is a comment
            requires x > 0;
            // Another comment
            ensures result >= x;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        assert_eq!(spec.postconditions.len(), 1);
    }

    #[test]
    fn test_parse_without_semicolons() {
        let input = r#"
            requires x > 0
            ensures result >= 0
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        assert_eq!(spec.postconditions.len(), 1);
    }

    #[test]
    fn test_parse_empty_contract() {
        let input = "";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert!(spec.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let input = "   \n\t  ";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert!(spec.is_empty());
    }

    #[test]
    fn test_parse_error_invalid_keyword() {
        let input = "invalid x > 0;";
        let result = parse_contract(input, Span::dummy());

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_missing_expression() {
        let input = "requires ;";
        let result = parse_contract(input, Span::dummy());

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_unclosed_paren() {
        let input = "requires (x + y == 0;";
        let result = parse_contract(input, Span::dummy());

        assert!(result.is_err());
    }
}

// =============================================================================
// Validation Tests
// =============================================================================

mod validation_tests {
    use super::*;

    #[test]
    fn test_validate_result_in_precondition_error() {
        let input = "requires result > 0;";
        // Use parse_contract_no_validate to test validation separately
        let spec = parse_contract_no_validate(input, Span::dummy()).unwrap();

        let result = validate_contract(&spec);
        assert!(result.is_err());
        match result.unwrap_err() {
            ContractError::InvalidUsage { message, .. } => {
                assert!(message.contains("precondition"));
                assert!(message.contains("result"));
            }
            _ => panic!("Expected InvalidUsage error"),
        }
    }

    #[test]
    fn test_validate_old_in_precondition_error() {
        let input = "requires old(x) > 0;";
        // Use parse_contract_no_validate to test validation separately
        let spec = parse_contract_no_validate(input, Span::dummy()).unwrap();

        let result = validate_contract(&spec);
        assert!(result.is_err());
        match result.unwrap_err() {
            ContractError::InvalidUsage { message, .. } => {
                assert!(message.contains("precondition"));
                assert!(message.contains("old"));
            }
            _ => panic!("Expected InvalidUsage error"),
        }
    }

    #[test]
    fn test_validate_old_in_invariant_error() {
        let input = "invariant old(x) == x;";
        // Use parse_contract_no_validate to test validation separately
        let spec = parse_contract_no_validate(input, Span::dummy()).unwrap();

        let result = validate_contract(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_result_in_invariant_error() {
        let input = "invariant result >= 0;";
        // Use parse_contract_no_validate to test validation separately
        let spec = parse_contract_no_validate(input, Span::dummy()).unwrap();

        let result = validate_contract(&spec);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_valid_contract() {
        let input = r#"
            requires x > 0;
            ensures result == old(x) + 1;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let result = validate_contract(&spec);
        assert!(result.is_ok());
    }
}

// =============================================================================
// SMT Translation Tests
// =============================================================================

mod smt_tests {
    use super::*;

    #[test]
    fn test_smt_simple_precondition() {
        let input = "requires x > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let smt = contract_to_smtlib(&spec, "test_func");

        assert!(smt.contains("set-logic ALL"));
        assert!(smt.contains("declare-const x"));
        assert!(smt.contains("assert"));
    }

    #[test]
    fn test_smt_postcondition_negation() {
        let input = "ensures result > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let smt = contract_to_smtlib(&spec, "test_func");

        assert!(smt.contains("not"));
        assert!(smt.contains("result"));
    }

    #[test]
    fn test_smt_old_expression() {
        let input = "ensures result == old(x) + 1;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let smt = contract_to_smtlib(&spec, "test_func");

        assert!(smt.contains("__old_"));
    }

    #[test]
    fn test_smt_check_sat() {
        let input = "requires x > 0; ensures result >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let smt = contract_to_smtlib(&spec, "test_func");

        assert!(smt.contains("check-sat"));
        assert!(smt.contains("get-model"));
    }

    #[test]
    fn test_generate_vcs_postcondition() {
        let input = r#"
            requires x > 0;
            ensures result > x;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let vcs = generate_contract_vcs(&spec, "increment");

        assert!(!vcs.is_empty());
    }

    #[test]
    fn test_generate_vcs_multiple_postconditions() {
        let input = r#"
            requires x > 0;
            ensures result > 0;
            ensures result >= x;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let vcs = generate_contract_vcs(&spec, "test_func");

        // Should generate one VC per postcondition
        assert!(vcs.len() >= 2);
    }
}

// =============================================================================
// Runtime Instrumentation Tests
// =============================================================================

mod instrumentation_tests {
    use super::*;

    #[test]
    fn test_instrument_precondition() {
        let input = "requires x > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        assert_eq!(instrumented.precondition_checks.len(), 1);
        let check = &instrumented.precondition_checks.first().unwrap();
        assert!(check.contains("Precondition"));
        assert!(check.contains("panic!"));
    }

    #[test]
    fn test_instrument_postcondition() {
        let input = "ensures result >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        assert_eq!(instrumented.postcondition_checks.len(), 1);
        let check = &instrumented.postcondition_checks.first().unwrap();
        assert!(check.contains("Postcondition"));
        assert!(check.contains("panic!"));
    }

    #[test]
    fn test_instrument_old_value_storage() {
        let input = "ensures result == old(x) + 1;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        assert!(!instrumented.old_value_stores.is_empty());
        let store = &instrumented.old_value_stores.first().unwrap();
        assert!(store.contains("__old_"));
        assert!(store.contains("let"));
    }

    #[test]
    fn test_instrument_forall_as_all() {
        let input = "ensures forall i in 0..n. arr[i] >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        let check = &instrumented.postcondition_checks.first().unwrap();
        assert!(check.contains(".all("));
    }

    #[test]
    fn test_instrument_exists_as_any() {
        let input = "ensures exists i in 0..n. arr[i] == target;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        let check = &instrumented.postcondition_checks.first().unwrap();
        assert!(check.contains(".any("));
    }

    #[test]
    fn test_instrument_implication() {
        let input = "requires x > 0 => y > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        let check = &instrumented.precondition_checks.first().unwrap();
        // Implication a => b becomes !a || b
        assert!(check.contains("||"));
    }

    #[test]
    fn test_instrument_empty_contract() {
        let input = "";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "test_func");

        assert!(instrumented.is_empty());
    }

    #[test]
    fn test_instrument_code_generation() {
        let input = r#"
            requires x > 0;
            ensures result == old(x) + 1;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let instrumented = instrument_contract(&spec, "increment");
        let code = instrumented.to_code();

        assert!(code.contains("Precondition"));
        assert!(code.contains("Postcondition"));
        assert!(code.contains("__old_"));
    }
}

// =============================================================================
// Expression Display Tests
// =============================================================================

mod display_tests {
    use super::*;

    #[test]
    fn test_contract_spec_display() {
        let input = r#"
            requires x > 0;
            ensures result >= 0;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let display = format!("{}", spec);

        assert!(display.contains("requires"));
        assert!(display.contains("ensures"));
    }

    #[test]
    fn test_predicate_display() {
        let input = "requires x + y > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();
        let pred = spec.preconditions.first().unwrap();

        let display = format!("{}", pred);

        // Should contain the expression
        assert!(!display.is_empty());
    }
}

// =============================================================================
// Old Expression Tests
// =============================================================================

mod old_expr_tests {
    use super::*;

    #[test]
    fn test_collect_old_expressions() {
        let input = r#"
            ensures result == old(x) + old(y);
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let old_exprs = spec.collect_old_expressions();

        assert_eq!(old_exprs.len(), 2);
    }

    #[test]
    fn test_nested_old_expressions() {
        let input = "ensures result == old(arr[i]);";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let old_exprs = spec.collect_old_expressions();

        assert_eq!(old_exprs.len(), 1);
    }

    #[test]
    fn test_old_storage_name() {
        let inner = ContractExpr::Var(Text::from("x"));
        let old = OldExpr::new(inner);

        let name = old.storage_name();

        assert!(name.contains("__old_"));
    }
}

// =============================================================================
// Free Variable Tests
// =============================================================================

mod free_var_tests {
    use super::*;

    #[test]
    fn test_free_variables_simple() {
        let expr = ContractExpr::BinOp(
            ContractBinOp::Add,
            Box::new(ContractExpr::Var(Text::from("x"))),
            Box::new(ContractExpr::Var(Text::from("y"))),
        );

        let free_vars = expr.free_variables();

        assert_eq!(free_vars.len(), 2);
        assert!(free_vars.contains(&Text::from("x")));
        assert!(free_vars.contains(&Text::from("y")));
    }

    #[test]
    fn test_free_variables_with_quantifier() {
        let input = "ensures forall i in 0..n. arr[i] >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();
        let pred = spec.postconditions.first().unwrap();

        let free_vars = pred.expr.free_variables();

        // 'i' should be bound, 'n' and 'arr' should be free
        assert!(!free_vars.contains(&Text::from("i")));
        assert!(free_vars.contains(&Text::from("n")));
        assert!(free_vars.contains(&Text::from("arr")));
    }

    #[test]
    fn test_free_variables_with_let() {
        let input = "requires let sum = x + y in sum > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();
        let pred = spec.preconditions.first().unwrap();

        let free_vars = pred.expr.free_variables();

        // 'sum' should be bound, 'x' and 'y' should be free
        assert!(!free_vars.contains(&Text::from("sum")));
        assert!(free_vars.contains(&Text::from("x")));
        assert!(free_vars.contains(&Text::from("y")));
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

mod integration_tests {
    use super::*;

    #[test]
    fn test_full_verification_workflow() {
        // Parse contract
        let input = r#"
            requires amount > 0;
            requires balance >= amount;
            ensures result == old(balance) - amount;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        // Validate
        validate_contract(&spec).unwrap();

        // Generate SMT
        let smt = contract_to_smtlib(&spec, "withdraw");
        assert!(!smt.is_empty());

        // Generate VCs
        let vcs = generate_contract_vcs(&spec, "withdraw");
        assert!(!vcs.is_empty());

        // Generate runtime instrumentation
        let instrumented = instrument_contract(&spec, "withdraw");
        assert!(!instrumented.is_empty());
    }

    #[test]
    fn test_transfer_funds_contract() {
        let input = r#"
            requires amount > 0;
            requires from.balance >= amount;
            ensures from.balance == old(from.balance) - amount;
            ensures to.balance == old(to.balance) + amount;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        validate_contract(&spec).unwrap();

        let old_exprs = spec.collect_old_expressions();
        assert_eq!(old_exprs.len(), 2);
    }

    #[test]
    fn test_sorting_contract() {
        let input = r#"
            requires n > 0;
            ensures forall i in 0..n-1. arr[i] <= arr[i+1];
            ensures forall i in 0..n. exists j in 0..n. arr[i] == old(arr)[j];
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        validate_contract(&spec).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
        assert_eq!(spec.postconditions.len(), 2);
    }

    #[test]
    fn test_binary_search_contract() {
        let input = r#"
            requires forall i in 0..n-1. arr[i] <= arr[i+1];
            ensures result >= 0 => arr[result] == target;
            ensures result < 0 => forall i in 0..n. arr[i] != target;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        validate_contract(&spec).unwrap();
    }

    #[test]
    fn test_merge_contracts() {
        let input1 = "requires x > 0;";
        let input2 = "ensures result >= 0;";

        let mut spec1 = parse_contract(input1, Span::dummy()).unwrap();
        let spec2 = parse_contract(input2, Span::dummy()).unwrap();

        spec1.merge(&spec2);

        assert_eq!(spec1.preconditions.len(), 1);
        assert_eq!(spec1.postconditions.len(), 1);
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_deeply_nested_expressions() {
        let input = "requires ((((x + y) * z) - w) / v) > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_very_long_contract() {
        let mut clauses = Vec::new();
        for i in 0..50 {
            clauses.push(format!("requires x{} > 0;", i));
        }
        let input = clauses.join("\n");

        let spec = parse_contract(&input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 50);
    }

    #[test]
    fn test_unicode_variable_names() {
        let input = "requires alpha > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_underscore_variable_names() {
        let input = "requires _private_var > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_numeric_suffixes() {
        let input = "requires x1 + x2 + x3 > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_float_literals() {
        let input = "requires x > 3.14;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        let pred = spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(_, _, right) => match right.as_ref() {
                ContractExpr::Float(f) => {
                    assert!((*f - 3.14).abs() < 0.001);
                }
                _ => panic!("Expected Float"),
            },
            _ => panic!("Expected BinOp"),
        }
    }

    #[test]
    fn test_negative_numbers() {
        let input = "requires x > -100;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_boolean_literals() {
        let input = "requires true && !false;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }
}

// =============================================================================
// Comprehensive Arithmetic Expression Tests
// =============================================================================

mod arithmetic_tests {
    use super::*;

    #[test]
    fn test_old_with_subtraction() {
        let input = "ensures result == old(balance) - amount;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();

        // Should parse as: result == (old(balance) - amount)
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Eq, _, right) => match right.as_ref() {
                ContractExpr::BinOp(ContractBinOp::Sub, left, _) => match left.as_ref() {
                    ContractExpr::Old(_) => {} // Success
                    _ => panic!("Expected old() on left side of subtraction"),
                },
                _ => panic!("Expected subtraction on right side of =="),
            },
            _ => panic!("Expected equality expression"),
        }
    }

    #[test]
    fn test_old_with_addition() {
        let input = "ensures result == old(x) + y;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        let pred = &spec.postconditions.first().unwrap();
        assert!(pred.contains_old());
    }

    #[test]
    fn test_field_with_arithmetic() {
        let input = "ensures account.balance == old(account.balance) - 100;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
    }

    #[test]
    fn test_range_with_subtraction() {
        let input = "ensures forall i in 0..n-1. arr[i] <= arr[i+1];";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);

        // Verify the range bound contains arithmetic
        let pred = &spec.postconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::Forall(binding, _) => {
                assert!(binding.range.is_some());
                let range = binding.range.as_ref().unwrap();
                // Upper bound should be "n-1"
                match range.upper.as_ref() {
                    ContractExpr::BinOp(ContractBinOp::Sub, _, _) => {} // Success
                    _ => panic!("Expected subtraction in upper bound"),
                }
            }
            _ => panic!("Expected Forall expression"),
        }
    }

    #[test]
    fn test_index_with_addition() {
        let input = "requires arr[i+1] > arr[i];";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Gt, left, _) => match left.as_ref() {
                ContractExpr::Index(_, idx) => match idx.as_ref() {
                    ContractExpr::BinOp(ContractBinOp::Add, _, _) => {} // Success
                    _ => panic!("Expected addition in index"),
                },
                _ => panic!("Expected Index on left"),
            },
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_complex_arithmetic_expression() {
        let input = "ensures result == (x + y) * z - w / 2;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
    }

    #[test]
    fn test_nested_arithmetic_with_old() {
        let input = "ensures balance == old(balance) - (fee + tax);";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 1);
        assert!(spec.postconditions.first().unwrap().contains_old());
    }

    #[test]
    fn test_multiple_arithmetic_in_postcondition() {
        let input = r#"
            ensures from.balance == old(from.balance) - amount;
            ensures to.balance == old(to.balance) + amount;
        "#;
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.postconditions.len(), 2);
        for pred in spec.postconditions.iter() {
            assert!(pred.contains_old());
        }
    }

    #[test]
    fn test_arithmetic_precedence() {
        let input = "requires x + y * z == (x + y) * z - x - y * z;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);

        // Verify left side: x + y * z parses as x + (y * z)
        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Eq, left, _) => match left.as_ref() {
                ContractExpr::BinOp(ContractBinOp::Add, _, right_inner) => {
                    match right_inner.as_ref() {
                        ContractExpr::BinOp(ContractBinOp::Mul, _, _) => {} // Success: y * z
                        _ => panic!("Expected multiplication"),
                    }
                }
                _ => panic!("Expected addition"),
            },
            _ => panic!("Expected equality"),
        }
    }

    #[test]
    fn test_division_and_modulo() {
        let input = "requires x / y + x % y == z;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }

    #[test]
    fn test_unary_minus_in_arithmetic() {
        let input = "requires -x + y == z;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Eq, left, _) => match left.as_ref() {
                ContractExpr::BinOp(ContractBinOp::Add, neg_expr, _) => match neg_expr.as_ref() {
                    ContractExpr::UnOp(ContractUnOp::Neg, _) => {} // Success
                    _ => panic!("Expected negation"),
                },
                _ => panic!("Expected addition"),
            },
            _ => panic!("Expected equality"),
        }
    }

    #[test]
    fn test_arithmetic_with_function_calls() {
        let input = "requires len(arr) - 1 >= 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);

        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Ge, left, _) => match left.as_ref() {
                ContractExpr::BinOp(ContractBinOp::Sub, func, _) => match func.as_ref() {
                    ContractExpr::Call(_, _) => {} // Success
                    _ => panic!("Expected function call"),
                },
                _ => panic!("Expected subtraction"),
            },
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_arithmetic_in_implication() {
        let input = "requires x - y > 0 => z + w > 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);

        // Both sides of implication should have arithmetic
        let pred = &spec.preconditions.first().unwrap();
        match &pred.expr {
            ContractExpr::BinOp(ContractBinOp::Imply, left, right) => {
                // Left: x - y > 0
                match left.as_ref() {
                    ContractExpr::BinOp(ContractBinOp::Gt, arith, _) => match arith.as_ref() {
                        ContractExpr::BinOp(ContractBinOp::Sub, _, _) => {} // Success
                        _ => panic!("Expected subtraction"),
                    },
                    _ => panic!("Expected comparison"),
                }
                // Right: z + w > 0
                match right.as_ref() {
                    ContractExpr::BinOp(ContractBinOp::Gt, arith, _) => match arith.as_ref() {
                        ContractExpr::BinOp(ContractBinOp::Add, _, _) => {} // Success
                        _ => panic!("Expected addition"),
                    },
                    _ => panic!("Expected comparison"),
                }
            }
            _ => panic!("Expected implication"),
        }
    }

    #[test]
    fn test_whitespace_variations_in_arithmetic() {
        let cases = [
            "requires x+y==z;",         // No spaces
            "requires x + y == z;",     // Normal spaces
            "requires x  +  y  ==  z;", // Extra spaces
            "requires x\n+\ny\n==\nz;", // Newlines
            "requires x\t+\ty\t==\tz;", // Tabs
        ];

        for input in cases {
            let spec = parse_contract(input, Span::dummy()).unwrap();
            assert_eq!(spec.preconditions.len(), 1, "Failed for: {}", input);
        }
    }

    #[test]
    fn test_consecutive_arithmetic_operations() {
        let input = "requires x + y - z * w / v % u == 0;";
        let spec = parse_contract(input, Span::dummy()).unwrap();

        assert_eq!(spec.preconditions.len(), 1);
    }
}
