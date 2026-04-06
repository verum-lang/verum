//! Integration tests for VBC → MLIR GPU lowering.
//!
//! These tests verify that VBC bytecode is correctly lowered to MLIR for GPU execution.
//! Compilation paths:
//!   - CPU Path: AST → VBC → LLVM IR (via VbcToLlvmLowering)
//!   - GPU Path: AST → VBC → MLIR (via VbcToMlirGpuLowering)
//!
//! This file tests the GPU path (VBC → MLIR).

use verum_codegen::{MlirContext, MlirCodegen, MlirConfig, GpuTarget};
use verum_vbc::codegen::VbcCodegen;
use verum_ast::{Module as AstModule, Item, ItemKind, Attribute};
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, FunctionBody, Visibility};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{Type, TypeKind, Ident};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind, IntLit};
use verum_ast::span::{Span, FileId};
use verum_common::{List, Text, Maybe};

/// Create a dummy span for testing.
fn dummy_span() -> Span {
    Span::new(0, 0, FileId::new(0))
}

/// Create an integer literal expression.
fn int_lit(value: i128) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span: dummy_span(),
        }),
        span: dummy_span(),
        ref_kind: None,
        check_eliminated: false,
    }
}

/// Create a simple function with just a return expression.
fn simple_function(name: &str, return_value: i128) -> Item {
    let return_expr = int_lit(return_value);

    Item {
        kind: ItemKind::Function(FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident {
                name: Text::from(name),
                span: dummy_span(),
            },
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::Some(Type {
                kind: TypeKind::Int,
                span: dummy_span(),
            }),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Expr(return_expr)),
            span: dummy_span(),
        }),
        span: dummy_span(),
        attributes: List::new(),
    }
}

/// Create a function with parameters.
fn function_with_params(name: &str, param_names: &[&str]) -> Item {
    let params: List<FunctionParam> = param_names
        .iter()
        .map(|pname| FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: Ident {
                            name: Text::from(*pname),
                            span: dummy_span(),
                        },
                        subpattern: Maybe::None,
                    },
                    span: dummy_span(),
                },
                ty: Type {
                    kind: TypeKind::Int,
                    span: dummy_span(),
                },
                default_value: Maybe::None,
            },
            attributes: List::new(),
            span: dummy_span(),
        })
        .collect();

    Item {
        kind: ItemKind::Function(FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_meta: false,
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            name: Ident {
                name: Text::from(name),
                span: dummy_span(),
            },
            generics: List::new(),
            params,
            return_type: Maybe::Some(Type {
                kind: TypeKind::Int,
                span: dummy_span(),
            }),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Expr(int_lit(42))),
            span: dummy_span(),
        }),
        span: dummy_span(),
        attributes: List::new(),
    }
}

/// Helper to create a test AST module.
fn test_module(items: Vec<Item>) -> AstModule {
    AstModule {
        items: items.into(),
        attributes: List::<Attribute>::new(),
        file_id: FileId::new(0),
        span: dummy_span(),
    }
}

// =============================================================================
// MLIR Context Tests
// =============================================================================

#[test]
fn test_mlir_context_creation() {
    let ctx = MlirContext::new().unwrap();
    // Context should be created successfully
    let _ = ctx.unknown_location();
}

// =============================================================================
// Pass Pipeline Tests
// =============================================================================

#[test]
fn test_pass_pipeline_creation() {
    use verum_codegen::{PassPipeline, PassConfig};

    let mlir_ctx = MlirContext::new().unwrap();
    let context = mlir_ctx.context();

    let config = PassConfig::default();
    let _pipeline = PassPipeline::new(context, config);
    // Pipeline should be created successfully
}

// =============================================================================
// VBC → MLIR GPU Lowering Tests
// =============================================================================

#[test]
fn test_vbc_to_mlir_lowering_basic() {
    // Step 1: Create AST module
    let ast_module = test_module(vec![simple_function("main", 42)]);

    // Step 2: Compile AST to VBC
    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    // Verify VBC module was created
    assert!(!vbc_module.functions.is_empty(), "VBC module should have functions");

    // Step 3: Lower VBC to MLIR for GPU
    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("test_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda);
    assert!(result.is_ok(), "VBC → MLIR lowering failed: {:?}", result.err());
}

#[test]
fn test_vbc_to_mlir_multiple_functions() {
    // Create AST module with multiple functions
    let ast_module = test_module(vec![
        simple_function("foo", 1),
        simple_function("bar", 2),
        simple_function("main", 0),
    ]);

    // Compile to VBC
    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    // The module includes stdlib/intrinsic functions alongside user-defined ones,
    // so verify that user functions are present rather than checking exact count.
    let user_fn_names: Vec<String> = vbc_module
        .functions
        .iter()
        .filter_map(|f| vbc_module.get_string(f.name).map(|s| s.to_string()))
        .collect();
    assert!(user_fn_names.contains(&"foo".to_string()), "Should contain 'foo', got: {:?}", user_fn_names);
    assert!(user_fn_names.contains(&"bar".to_string()), "Should contain 'bar', got: {:?}", user_fn_names);
    assert!(user_fn_names.contains(&"main".to_string()), "Should contain 'main', got: {:?}", user_fn_names);
    assert!(vbc_module.functions.len() >= 3, "Should have at least 3 functions, got {}", vbc_module.functions.len());

    // Lower to MLIR
    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("test_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda);
    assert!(result.is_ok(), "VBC → MLIR lowering failed: {:?}", result.err());
}

#[test]
fn test_vbc_to_mlir_function_with_params() {
    let ast_module = test_module(vec![function_with_params("add", &["a", "b"])]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("test_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda);
    assert!(result.is_ok(), "VBC → MLIR lowering failed: {:?}", result.err());
}

#[test]
fn test_vbc_to_mlir_verification() {
    let ast_module = test_module(vec![simple_function("main", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("test_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda).unwrap();

    // Verify MLIR module
    let verify_result = mlir_codegen.verify();
    assert!(verify_result.is_ok(), "MLIR verification failed: {:?}", verify_result.err());
}

#[test]
fn test_vbc_to_mlir_string_output() {
    let ast_module = test_module(vec![simple_function("main", 42)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("test_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda).unwrap();

    // Get MLIR string representation
    let mlir_str = mlir_codegen.get_mlir_string().unwrap();

    // Should contain module structure
    assert!(mlir_str.as_str().contains("module"), "MLIR output should contain module: {}", mlir_str.as_str());
}

// =============================================================================
// GPU Target Tests
// =============================================================================

#[test]
fn test_gpu_target_cuda() {
    let ast_module = test_module(vec![simple_function("kernel", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("cuda_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Cuda);
    assert!(result.is_ok(), "CUDA lowering failed: {:?}", result.err());
}

#[test]
fn test_gpu_target_rocm() {
    let ast_module = test_module(vec![simple_function("kernel", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("rocm_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Rocm);
    assert!(result.is_ok(), "ROCm lowering failed: {:?}", result.err());
}

#[test]
fn test_gpu_target_vulkan() {
    let ast_module = test_module(vec![simple_function("kernel", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("vulkan_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Vulkan);
    assert!(result.is_ok(), "Vulkan lowering failed: {:?}", result.err());
}

#[test]
fn test_gpu_target_metal() {
    let ast_module = test_module(vec![simple_function("kernel", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    let mlir_ctx = MlirContext::new().unwrap();
    let config = MlirConfig::new("metal_module");
    let mut mlir_codegen = MlirCodegen::new(&mlir_ctx, config).unwrap();

    let result = mlir_codegen.lower_vbc_module(&vbc_module, GpuTarget::Metal);
    assert!(result.is_ok(), "Metal lowering failed: {:?}", result.err());
}

// =============================================================================
// VBC Codegen Stats Tests
// =============================================================================

#[test]
fn test_vbc_codegen_stats() {
    let ast_module = test_module(vec![
        simple_function("foo", 1),
        simple_function("bar", 2),
    ]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast_module).unwrap();

    // Check VBC module statistics
    // The module includes stdlib/intrinsic functions alongside user-defined ones,
    // so verify that user functions are present rather than checking exact count.
    let user_fn_names: Vec<String> = vbc_module
        .functions
        .iter()
        .filter_map(|f| vbc_module.get_string(f.name).map(|s| s.to_string()))
        .collect();
    assert!(user_fn_names.contains(&"foo".to_string()), "Should contain 'foo', got: {:?}", user_fn_names);
    assert!(user_fn_names.contains(&"bar".to_string()), "Should contain 'bar', got: {:?}", user_fn_names);
    assert!(vbc_module.functions.len() >= 2, "Should have at least 2 functions, got {}", vbc_module.functions.len());
    assert!(!vbc_module.name.is_empty(), "Module should have a name");
}
