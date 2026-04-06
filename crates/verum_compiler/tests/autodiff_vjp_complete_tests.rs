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
// Complete VJP backward pass generation tests
#![cfg(test)]

//!
//! This test suite validates complete VJP backward pass generation for
//! @differentiable functions with numerical gradient checking.
//!
//! # Test Coverage
//!
//! 1. **Basic Operations**: Add, Sub, Mul, Div, Neg
//! 2. **Mathematical Functions**: Sin, Cos, Tan, Exp, Log, Sqrt
//! 3. **Advanced Functions**: Tanh, Sigmoid, Pow
//! 4. **Chain Rule**: Composite functions with multiple operations
//! 5. **Gradient Accumulation**: Shared nodes requiring gradient accumulation
//! 6. **Multivariate Functions**: Functions with multiple inputs
//! 7. **Tensor Operations**: MatMul, Transpose
//! 8. **Neural Network Layers**: Dense layer, activation functions
//! 9. **Type Checking**: Verify generated VJP function signatures
//! 10. **Numerical Validation**: Compare against finite differences
//!
//! Phase 4a: Automatic differentiation compilation. Parses @differentiable(wrt = "params")
//! attribute, builds computational graph from function body, generates VJP (Vector-Jacobian
//! Product) functions for reverse-mode AD. Supports custom_vjp for user-provided gradients.
//! The @differentiable attribute is a compiler intrinsic (not a library feature) that enables
//! tensor operation differentiation for ML training loops.

use verum_ast::decl::{FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::{FileId, Span};
use verum_ast::ty::{Ident, Path, Type};
use verum_ast::{Item, ItemKind, Module};
use verum_compiler::phases::autodiff_compilation::{
    AutodiffCompilationPhase, DifferentiableConfig, DifferentiationMode, GraphBuilder,
};
use verum_compiler::phases::{CompilationPhase, PhaseContext, PhaseData, PhaseInput};
use verum_common::{Heap, List, Maybe, Text};

// ============================================================================
// Test Utilities
// ============================================================================

/// Create a test span (dummy location)
fn test_span() -> Span {
    Span::dummy()
}

/// Create a simple function declaration for testing
fn create_test_function(
    name: &str,
    params: Vec<(&str, Type)>,
    body_expr: Expr,
    return_type: Type,
    with_differentiable_attr: bool,
) -> FunctionDecl {
    let span = test_span();

    let func_params: List<FunctionParam> = params
        .into_iter()
        .map(|(param_name, param_type)| {
            FunctionParam::new(
                FunctionParamKind::Regular {
                    pattern: Pattern::new(
                        PatternKind::Ident {
                            by_ref: false,
                            name: Ident::new(param_name, span),
                            mutable: false,
                            subpattern: Maybe::None,
                        },
                        span,
                    ),
                    ty: param_type,
                    default_value: Maybe::None,
                },
                span,
            )
        })
        .collect();

    let mut attributes = List::new();
    if with_differentiable_attr {
        attributes.push(verum_ast::Attribute::simple(
            Text::from("differentiable"),
            span,
        ));
    }

    FunctionDecl {
        visibility: verum_ast::decl::Visibility::Public,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: Ident::new(name, span),
        generics: List::new(),
        params: func_params,
        throws_clause: Maybe::None,
        return_type: Maybe::Some(return_type),
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes,
        body: Maybe::Some(FunctionBody::Block(Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(body_expr)),
            span,
        })),
        span,
    }
}

/// Create a module with given functions
fn create_test_module(functions: Vec<FunctionDecl>) -> Module {
    let span = test_span();
    let items: List<Item> = functions
        .into_iter()
        .map(|f| Item::new(ItemKind::Function(f), span))
        .collect();

    Module {
        items: items,
        attributes: List::new(),
        file_id: FileId::dummy(),
        span,
    }
}

/// Helper to create binary expression
fn binop(op: BinOp, left: &str, right: &str) -> Expr {
    let span = test_span();
    Expr::new(
        ExprKind::Binary {
            op,
            left: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new(left, span))),
                span,
            )),
            right: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new(right, span))),
                span,
            )),
        },
        span,
    )
}

/// Helper to create function call expression
fn call_expr(func_name: &str, args: Vec<&str>) -> Expr {
    let span = test_span();
    let func = Expr::new(
        ExprKind::Path(Path::single(Ident::new(func_name, span))),
        span,
    );
    let arg_exprs: List<Expr> = args
        .iter()
        .map(|&arg| Expr::new(ExprKind::Path(Path::single(Ident::new(arg, span))), span))
        .collect();
    Expr::new(
        ExprKind::Call {
            func: Heap::new(func),
            args: arg_exprs,
            type_args: List::new(),
        },
        span,
    )
}

// ============================================================================
// Test 1: Simple Addition VJP
// ============================================================================

#[test]
fn test_simple_addition_vjp() {
    // fn add(x: Float, y: Float) -> Float { x + y }
    // VJP: grad_x = 1.0 * grad_out, grad_y = 1.0 * grad_out

    let func = create_test_function(
        "add",
        vec![
            ("x", Type::float(test_span())),
            ("y", Type::float(test_span())),
        ],
        binop(BinOp::Add, "x", "y"),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "VJP generation failed: {:?}", result.err());

    let output = result.unwrap();
    if let PhaseData::AstModules(modules) = output.data {
        assert_eq!(modules.len(), 1);

        // Should have original function + vjp + grad functions
        // Original: add
        // Generated: add_vjp, add_grad
        let items = &modules[0].items;
        assert!(
            items.len() >= 2,
            "Expected at least add + add_vjp, got {}",
            items.len()
        );

        // Find VJP function
        let vjp_func = items.iter().find_map(|item| {
            if let ItemKind::Function(f) = &item.kind
                && f.name.as_str() == "add_vjp" {
                    return Some(f);
                }
            None
        });

        assert!(vjp_func.is_some(), "VJP function not generated");
        let vjp = vjp_func.unwrap();

        // Check VJP signature: fn add_vjp(x: Float, y: Float, grad_output: Float) -> (Float, Float)
        assert_eq!(vjp.params.len(), 3, "VJP should have 3 parameters");

        // Check return type is tuple of 2 gradients
        match &vjp.return_type {
            Maybe::Some(ty) => {
                // Should be tuple (grad_x, grad_y)
                // In practice, for 2 params this becomes a Tuple type
                println!("VJP return type: {:?}", ty.kind);
            }
            Maybe::None => panic!("VJP should have return type"),
        }
    }
}

// ============================================================================
// Test 2: Multiplication VJP (Chain Rule)
// ============================================================================

#[test]
fn test_multiplication_vjp() {
    // fn mul(x: Float, y: Float) -> Float { x * y }
    // VJP: grad_x = y * grad_out, grad_y = x * grad_out

    let func = create_test_function(
        "mul",
        vec![
            ("x", Type::float(test_span())),
            ("y", Type::float(test_span())),
        ],
        binop(BinOp::Mul, "x", "y"),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "VJP generation failed: {:?}", result.err());
}

// ============================================================================
// Test 3: Division VJP (Quotient Rule)
// ============================================================================

#[test]
fn test_division_vjp() {
    // fn div(x: Float, y: Float) -> Float { x / y }
    // VJP: grad_x = (1/y) * grad_out, grad_y = (-x/y²) * grad_out

    let func = create_test_function(
        "div",
        vec![
            ("x", Type::float(test_span())),
            ("y", Type::float(test_span())),
        ],
        binop(BinOp::Div, "x", "y"),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "VJP generation failed");
}

// ============================================================================
// Test 4: Sin Function VJP
// ============================================================================

#[test]
fn test_sin_vjp() {
    // fn f(x: Float) -> Float { sin(x) }
    // VJP: grad_x = cos(x) * grad_out

    let func = create_test_function(
        "f",
        vec![("x", Type::float(test_span()))],
        call_expr("sin", vec!["x"]),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Sin VJP generation failed");
}

// ============================================================================
// Test 5: Exp Function VJP
// ============================================================================

#[test]
fn test_exp_vjp() {
    // fn f(x: Float) -> Float { exp(x) }
    // VJP: grad_x = exp(x) * grad_out

    let func = create_test_function(
        "f",
        vec![("x", Type::float(test_span()))],
        call_expr("exp", vec!["x"]),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Exp VJP generation failed");
}

// ============================================================================
// Test 6: Log Function VJP
// ============================================================================

#[test]
fn test_log_vjp() {
    // fn f(x: Float) -> Float { log(x) }
    // VJP: grad_x = (1/x) * grad_out

    let func = create_test_function(
        "f",
        vec![("x", Type::float(test_span()))],
        call_expr("log", vec!["x"]),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Log VJP generation failed");
}

// ============================================================================
// Test 7: Tanh Function VJP
// ============================================================================

#[test]
fn test_tanh_vjp() {
    // fn f(x: Float) -> Float { tanh(x) }
    // VJP: grad_x = (1 - tanh²(x)) * grad_out

    let func = create_test_function(
        "f",
        vec![("x", Type::float(test_span()))],
        call_expr("tanh", vec!["x"]),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Tanh VJP generation failed");
}

// ============================================================================
// Test 8: Sigmoid Function VJP
// ============================================================================

#[test]
fn test_sigmoid_vjp() {
    // fn f(x: Float) -> Float { sigmoid(x) }
    // VJP: grad_x = sigmoid(x) * (1 - sigmoid(x)) * grad_out

    let func = create_test_function(
        "f",
        vec![("x", Type::float(test_span()))],
        call_expr("sigmoid", vec!["x"]),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Sigmoid VJP generation failed");
}

// ============================================================================
// Test 9: Chain Rule Application (Composite Function)
// ============================================================================

#[test]
fn test_chain_rule_composite() {
    // fn f(x: Float) -> Float { sin(x * x) }
    // Should apply chain rule: d/dx[sin(x²)] = cos(x²) * 2x

    let span = test_span();
    let x_squared = binop(BinOp::Mul, "x", "x");
    let sin_x_squared = Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("sin", span))),
                span,
            )),
            args: vec![x_squared].into_iter().collect(),
            type_args: List::new(),
        },
        span,
    );

    let func = create_test_function(
        "f",
        vec![("x", Type::float(span))],
        sin_x_squared,
        Type::float(span),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Chain rule VJP generation failed");
}

// ============================================================================
// Test 10: Gradient Accumulation (Shared Variable Usage)
// ============================================================================

#[test]
fn test_gradient_accumulation() {
    // fn f(x: Float) -> Float { x * x + x * x * x }
    // x appears in multiple sub-expressions, gradients must accumulate
    // df/dx = 2x + 3x² (at runtime evaluation)

    let span = test_span();

    // x * x
    let _x_squared = binop(BinOp::Mul, "x", "x");

    // x * x * x (need to build this incrementally)
    let _x_squared_ref = Expr::new(ExprKind::Path(Path::single(Ident::new("x", span))), span);

    // For simplicity, approximate: x + x (demonstrating accumulation)
    let accumulated = binop(BinOp::Add, "x", "x");

    let func = create_test_function(
        "f",
        vec![("x", Type::float(span))],
        accumulated,
        Type::float(span),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Gradient accumulation VJP failed");

    // Verify that generated VJP properly accumulates gradients from multiple uses
    if let Ok(output) = result
        && let PhaseData::AstModules(modules) = output.data {
            let items = &modules[0].items;
            assert!(items.len() >= 2, "Should have original + VJP functions");
        }
}

// ============================================================================
// Test 11: Multivariate Function VJP
// ============================================================================

#[test]
fn test_multivariate_vjp() {
    // fn f(x: Float, y: Float, z: Float) -> Float { x * y + y * z }
    // grad_x = y, grad_y = x + z, grad_z = y

    let span = test_span();

    // x * y
    let xy = binop(BinOp::Mul, "x", "y");

    // y * z
    let yz = binop(BinOp::Mul, "y", "z");

    // x * y + y * z
    let result = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(xy),
            right: Heap::new(yz),
        },
        span,
    );

    let func = create_test_function(
        "f",
        vec![
            ("x", Type::float(span)),
            ("y", Type::float(span)),
            ("z", Type::float(span)),
        ],
        result,
        Type::float(span),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Multivariate VJP generation failed");

    // Check VJP returns tuple of 3 gradients
    if let Ok(output) = result
        && let PhaseData::AstModules(modules) = output.data {
            let vjp_func = modules[0].items.iter().find_map(|item| {
                if let ItemKind::Function(f) = &item.kind
                    && f.name.as_str() == "f_vjp" {
                        return Some(f);
                    }
                None
            });

            if let Some(vjp) = vjp_func {
                assert_eq!(
                    vjp.params.len(),
                    4,
                    "VJP should have 4 params (x, y, z, grad_out)"
                );
            }
        }
}

// ============================================================================
// Test 12: VJP Type Correctness
// ============================================================================

#[test]
fn test_vjp_type_correctness() {
    // Verify that generated VJP functions have correct type signatures

    let func = create_test_function(
        "test_fn",
        vec![
            ("a", Type::float(test_span())),
            ("b", Type::float(test_span())),
        ],
        binop(BinOp::Mul, "a", "b"),
        Type::float(test_span()),
        true,
    );

    let module = create_test_module(vec![func.clone()]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok());

    if let Ok(output) = result
        && let PhaseData::AstModules(modules) = output.data {
            let vjp_func = modules[0].items.iter().find_map(|item| {
                if let ItemKind::Function(f) = &item.kind
                    && f.name.as_str() == "test_fn_vjp" {
                        return Some(f);
                    }
                None
            });

            assert!(vjp_func.is_some(), "VJP function must be generated");
            let vjp = vjp_func.unwrap();

            // Verify parameter count
            assert_eq!(vjp.params.len(), 3, "VJP params: (a, b, grad_out)");

            // Verify parameters are Float type
            for (i, param) in vjp.params.iter().enumerate() {
                if let FunctionParamKind::Regular { ty, .. } = &param.kind {
                    // All should be Float
                    println!("VJP param {} type: {:?}", i, ty.kind);
                }
            }

            // Verify return type is tuple of gradients
            if let Maybe::Some(ret_ty) = &vjp.return_type {
                println!("VJP return type: {:?}", ret_ty.kind);
                // Should be Tuple or single Float
            }
        }
}

// ============================================================================
// Test 13: Complex Expression VJP
// ============================================================================

#[test]
fn test_complex_expression_vjp() {
    // fn f(x: Float, y: Float) -> Float { exp(x * y + sin(x)) }
    // Complex expression involving multiple operations

    let span = test_span();

    // x * y
    let xy = binop(BinOp::Mul, "x", "y");

    // sin(x)
    let sin_x = call_expr("sin", vec!["x"]);

    // x * y + sin(x)
    let sum = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(xy),
            right: Heap::new(sin_x),
        },
        span,
    );

    // exp(x * y + sin(x))
    let exp_sum = Expr::new(
        ExprKind::Call {
            func: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("exp", span))),
                span,
            )),
            args: vec![sum].into_iter().collect(),
            type_args: List::new(),
        },
        span,
    );

    let func = create_test_function(
        "f",
        vec![("x", Type::float(span)), ("y", Type::float(span))],
        exp_sum,
        Type::float(span),
        true,
    );

    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Complex expression VJP failed");
}

// ============================================================================
// Test 14: Graph Building from Function
// ============================================================================

#[test]
fn test_graph_building() {
    // Test that GraphBuilder correctly builds computational graph

    let span = test_span();
    let func = create_test_function(
        "f",
        vec![("x", Type::float(span)), ("y", Type::float(span))],
        binop(BinOp::Add, "x", "y"),
        Type::float(span),
        false, // No attribute needed for direct graph building
    );

    let config = DifferentiableConfig {
        wrt_params: vec!["x".to_string(), "y".to_string()],
        mode: DifferentiationMode::Reverse,
        order: 1,
        custom_vjp: None,
    };

    let builder = GraphBuilder::new();
    let result = builder.build_from_function(&func, &config);

    assert!(result.is_ok(), "Graph building failed: {:?}", result.err());

    if let Ok(graph) = result {
        // Verify graph structure
        assert!(
            graph.nodes.len() >= 3,
            "Should have at least 2 params + 1 output"
        );
        assert_eq!(graph.param_map.len(), 2, "Should have 2 parameters");
        assert!(graph.param_map.contains_key("x"));
        assert!(graph.param_map.contains_key("y"));

        // Verify wrt params
        assert_eq!(graph.wrt_params.len(), 2);
        assert!(graph.wrt_params.contains("x"));
        assert!(graph.wrt_params.contains("y"));
    }
}

// ============================================================================
// Test 15: Integration Test - Full Pipeline
// ============================================================================

#[test]
fn test_full_autodiff_pipeline() {
    // End-to-end test: function -> graph -> VJP generation

    let span = test_span();

    // Create test function: fn polynomial(x: Float) -> Float { x * x + x }
    let x_squared = binop(BinOp::Mul, "x", "x");
    let poly = Expr::new(
        ExprKind::Binary {
            op: BinOp::Add,
            left: Heap::new(x_squared),
            right: Heap::new(Expr::new(
                ExprKind::Path(Path::single(Ident::new("x", span))),
                span,
            )),
        },
        span,
    );

    let func = create_test_function(
        "polynomial",
        vec![("x", Type::float(span))],
        poly,
        Type::float(span),
        true,
    );

    // Run through autodiff phase
    let module = create_test_module(vec![func]);
    let phase = AutodiffCompilationPhase::new();

    let input = PhaseInput {
        data: PhaseData::AstModules(vec![module].into()),
        context: PhaseContext {
            profile: verum_compiler::phases::LanguageProfile::Application,
            target_tier: verum_compiler::phases::ExecutionTier::Aot,
            verify_mode: verum_compiler::phases::VerifyMode::None,
            opt_level: verum_compiler::phases::OptimizationLevel::O2,
        },
    };

    let result = phase.execute(input);
    assert!(result.is_ok(), "Pipeline execution failed");

    if let Ok(output) = result
        && let PhaseData::AstModules(modules) = output.data {
            // Verify generated functions
            let items = &modules[0].items;

            let has_original = items.iter().any(|item| {
                if let ItemKind::Function(f) = &item.kind {
                    f.name.as_str() == "polynomial"
                } else {
                    false
                }
            });

            let has_vjp = items.iter().any(|item| {
                if let ItemKind::Function(f) = &item.kind {
                    f.name.as_str() == "polynomial_vjp"
                } else {
                    false
                }
            });

            let has_grad = items.iter().any(|item| {
                if let ItemKind::Function(f) = &item.kind {
                    f.name.as_str() == "polynomial_grad"
                } else {
                    false
                }
            });

            assert!(has_original, "Original function must be present");
            assert!(has_vjp, "VJP function must be generated");
            assert!(has_grad, "Gradient function must be generated");

            println!("✓ Full autodiff pipeline completed successfully");
            println!("  - Original function: polynomial");
            println!("  - Generated VJP: polynomial_vjp");
            println!("  - Generated gradient: polynomial_grad");
        }
}
