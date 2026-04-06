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
//! Integration tests for VBC -> LLVM IR code generation (CPU path).
//!
//! These tests verify the VBC -> LLVM lowering pipeline:
//!   1. Basic LLVM IR emission
//!   2. Type codegen (Int, Float, Bool, Text, etc.)
//!   3. Control flow codegen (if/match/loop)
//!   4. Function codegen (signatures, closures, generics)
//!   5. CBGR reference codegen (ThinRef/FatRef)

use verum_codegen::llvm::{
    VbcToLlvmLowering, LoweringConfig, LoweringStats,
    TypeLowering, RefTier,
    CbgrLowering, CbgrStats,
};
use verum_llvm::context::Context;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::types::{TypeId, TypeRef};
use verum_ast::{Module as AstModule, Item, ItemKind, Attribute};
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, FunctionBody, Visibility};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{Type, TypeKind, Ident};
use verum_ast::expr::{Expr, ExprKind, BinOp};
use verum_common::Heap;
use verum_ast::literal::{Literal, LiteralKind, IntLit, FloatLit};
use verum_ast::span::{Span, FileId};
use verum_common::{List, Text, Maybe};

// =============================================================================
// HELPERS
// =============================================================================

fn dummy_span() -> Span {
    Span::new(0, 0, FileId::new(0))
}

fn int_lit_expr(value: i128) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit { value, suffix: None }),
            span: dummy_span(),
        }),
        span: dummy_span(),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn float_lit_expr(value: f64) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Float(FloatLit {
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

fn bool_lit_expr(value: bool) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span: dummy_span(),
        }),
        span: dummy_span(),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn ident_expr(name: &str) -> Expr {
    use smallvec::smallvec;
    Expr {
        kind: ExprKind::Path(verum_ast::ty::Path {
            segments: smallvec![verum_ast::ty::PathSegment::Name(Ident {
                name: Text::from(name),
                span: dummy_span(),
            })],
            span: dummy_span(),
        }),
        span: dummy_span(),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn bin_op_expr(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        span: dummy_span(),
        ref_kind: None,
        check_eliminated: false,
    }
}

fn mk_type(kind: TypeKind) -> Type {
    Type { kind, span: dummy_span() }
}

fn simple_function(name: &str, return_value: i128) -> Item {
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
            name: Ident { name: Text::from(name), span: dummy_span() },
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::Some(mk_type(TypeKind::Int)),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Expr(int_lit_expr(return_value))),
            span: dummy_span(),
        }),
        span: dummy_span(),
        attributes: List::new(),
    }
}

fn function_with_params(name: &str, params: &[(&str, TypeKind)], body: Expr, ret_type: TypeKind) -> Item {
    let param_list: List<FunctionParam> = params
        .iter()
        .map(|(pname, ty)| FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: Ident { name: Text::from(*pname), span: dummy_span() },
                        subpattern: Maybe::None,
                    },
                    span: dummy_span(),
                },
                ty: mk_type(ty.clone()),
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
            name: Ident { name: Text::from(name), span: dummy_span() },
            generics: List::new(),
            params: param_list,
            return_type: Maybe::Some(mk_type(ret_type)),
            throws_clause: Maybe::None,
            std_attr: Maybe::None,
            contexts: List::new(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            attributes: List::new(),
            body: Maybe::Some(FunctionBody::Expr(body)),
            span: dummy_span(),
        }),
        span: dummy_span(),
        attributes: List::new(),
    }
}

fn test_module(items: Vec<Item>) -> AstModule {
    AstModule {
        items: items.into(),
        attributes: List::<Attribute>::new(),
        file_id: FileId::new(0),
        span: dummy_span(),
    }
}

/// Compile AST -> VBC -> LLVM IR, returning the IR string and stats.
fn compile_to_llvm_ir(ast_module: &AstModule) -> (Text, LoweringStats) {
    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen
        .compile_module(ast_module)
        .expect("VBC compilation failed");

    let context = Context::create();
    let config = LoweringConfig::debug("test_module");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering
        .lower_module(&vbc_module)
        .expect("LLVM lowering failed");

    let ir = lowering.get_ir();
    let stats = lowering.stats().clone();
    (ir, stats)
}

// =============================================================================
// 1. BASIC LLVM IR EMISSION TESTS
// =============================================================================

#[test]
fn test_llvm_ir_simple_function_emits_define() {
    let ast = test_module(vec![simple_function("my_func", 42)]);
    let (ir, _) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    // Should emit a define for the function
    assert!(
        ir_str.contains("define") && ir_str.contains("my_func"),
        "LLVM IR should contain a define for my_func:\n{}",
        &ir_str[..ir_str.len().min(500)]
    );
}

#[test]
fn test_llvm_ir_contains_module_metadata() {
    let ast = test_module(vec![simple_function("main", 0)]);
    let (ir, _) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    // Should have a target triple or source_filename
    assert!(
        ir_str.contains("source_filename") || ir_str.contains("target triple") || ir_str.contains("target datalayout"),
        "LLVM IR should contain module metadata"
    );
}

#[test]
fn test_llvm_ir_multiple_functions() {
    let ast = test_module(vec![
        simple_function("foo", 1),
        simple_function("bar", 2),
        simple_function("main", 0),
    ]);
    let (ir, stats) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    assert!(ir_str.contains("foo"), "IR should contain foo");
    assert!(ir_str.contains("bar"), "IR should contain bar");
    assert!(ir_str.contains("main"), "IR should contain main");
    assert!(stats.functions_lowered >= 3, "Should lower at least 3 functions, got {}", stats.functions_lowered);
}

#[test]
fn test_llvm_ir_empty_module() {
    let ast = test_module(vec![]);
    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast).expect("VBC compilation failed");

    let context = Context::create();
    let config = LoweringConfig::debug("empty_module");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    let result = lowering.lower_module(&vbc_module);
    assert!(result.is_ok(), "Empty module should lower without error");
}

// =============================================================================
// 2. TYPE CODEGEN TESTS
// =============================================================================

#[test]
fn test_type_lowering_int() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::INT);
    assert!(result.is_ok(), "Int type should lower successfully");
    // Int maps to i64
    let llvm_type = result.unwrap();
    assert!(llvm_type.is_int_type(), "Int should lower to integer type");
}

#[test]
fn test_type_lowering_float() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::FLOAT);
    assert!(result.is_ok(), "Float type should lower successfully");
    let llvm_type = result.unwrap();
    assert!(llvm_type.is_float_type(), "Float should lower to float type");
}

#[test]
fn test_type_lowering_bool() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::BOOL);
    assert!(result.is_ok(), "Bool type should lower successfully");
    let llvm_type = result.unwrap();
    assert!(llvm_type.is_int_type(), "Bool should lower to i1 integer type");
}

#[test]
fn test_type_lowering_text() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::TEXT);
    assert!(result.is_ok(), "Text type should lower successfully");
    let llvm_type = result.unwrap();
    // Text is a pointer to string data
    assert!(llvm_type.is_pointer_type(), "Text should lower to pointer type");
}

#[test]
fn test_type_lowering_unit() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::UNIT);
    assert!(result.is_ok(), "Unit type should lower successfully");
    let llvm_type = result.unwrap();
    // Unit is an empty struct
    assert!(llvm_type.is_struct_type(), "Unit should lower to empty struct type");
}

#[test]
fn test_type_lowering_integer_sizes() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    for type_id in &[TypeId::I8, TypeId::I16, TypeId::I32, TypeId::U8, TypeId::U16, TypeId::U32, TypeId::U64] {
        let result = types.lower_type_id(*type_id);
        assert!(result.is_ok(), "TypeId {:?} should lower successfully", type_id);
        assert!(result.unwrap().is_int_type(), "TypeId {:?} should lower to int type", type_id);
    }
}

#[test]
fn test_type_lowering_f32() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::F32);
    assert!(result.is_ok(), "F32 should lower successfully");
    assert!(result.unwrap().is_float_type(), "F32 should lower to float type");
}

#[test]
fn test_type_lowering_ptr() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_type_id(TypeId::PTR);
    assert!(result.is_ok(), "PTR type should lower successfully");
    assert!(result.unwrap().is_pointer_type(), "PTR should lower to pointer type");
}

#[test]
fn test_type_lowering_tuple() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let tuple_ref = vec![
        TypeRef::Concrete(TypeId::INT),
        TypeRef::Concrete(TypeId::FLOAT),
        TypeRef::Concrete(TypeId::BOOL),
    ];
    let result = types.lower_tuple(&tuple_ref);
    assert!(result.is_ok(), "Tuple type should lower successfully");
    assert!(result.unwrap().is_struct_type(), "Tuple should lower to struct type");
}

#[test]
fn test_type_lowering_empty_tuple() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let result = types.lower_tuple(&[]);
    assert!(result.is_ok(), "Empty tuple should lower successfully");
    assert!(result.unwrap().is_struct_type(), "Empty tuple should lower to struct type");
}

#[test]
fn test_type_lowering_function_type() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let fn_ref = TypeRef::Function {
        params: vec![TypeRef::Concrete(TypeId::INT)],
        return_type: Box::new(TypeRef::Concrete(TypeId::INT)),
        contexts: Default::default(),
    };
    let result = types.lower_type_ref(&fn_ref);
    assert!(result.is_ok(), "Function type should lower successfully");
    // Function types are opaque pointers
    assert!(result.unwrap().is_pointer_type(), "Function type should lower to pointer");
}

#[test]
fn test_type_lowering_reference_type() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let ref_type = TypeRef::Reference {
        inner: Box::new(TypeRef::Concrete(TypeId::INT)),
        mutability: verum_vbc::types::Mutability::Immutable,
        tier: verum_vbc::types::CbgrTier::Tier0,
    };
    let result = types.lower_type_ref(&ref_type);
    assert!(result.is_ok(), "Reference type should lower successfully");
    assert!(result.unwrap().is_pointer_type(), "Reference should lower to pointer");
}

#[test]
fn test_type_lowering_array_type() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let arr_type = TypeRef::Array {
        element: Box::new(TypeRef::Concrete(TypeId::INT)),
        length: 10,
    };
    let result = types.lower_type_ref(&arr_type);
    assert!(result.is_ok(), "Array type should lower successfully");
    assert!(result.unwrap().is_array_type(), "Array should lower to array type");
}

#[test]
fn test_type_lowering_slice_type() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let slice_type = TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId::INT)));
    let result = types.lower_type_ref(&slice_type);
    assert!(result.is_ok(), "Slice type should lower successfully");
    // Slice is a fat pointer: { ptr, len }
    assert!(result.unwrap().is_struct_type(), "Slice should lower to struct (fat pointer)");
}

#[test]
fn test_type_lowering_generic_rejects_unresolved() {
    let context = Context::create();
    let types = TypeLowering::new(&context);

    let generic_ref = TypeRef::Generic(verum_vbc::types::TypeParamId(0));
    let result = types.lower_type_ref(&generic_ref);
    assert!(result.is_err(), "Generic type should fail to lower (must be monomorphized first)");
}

// =============================================================================
// 3. FUNCTION CODEGEN TESTS
// =============================================================================

#[test]
fn test_function_with_int_params() {
    let body = bin_op_expr(BinOp::Add, ident_expr("a"), ident_expr("b"));
    let ast = test_module(vec![function_with_params(
        "add",
        &[("a", TypeKind::Int), ("b", TypeKind::Int)],
        body,
        TypeKind::Int,
    )]);
    let (ir, stats) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    assert!(ir_str.contains("add"), "IR should contain add function");
    assert!(stats.functions_lowered >= 1);
}

#[test]
fn test_function_with_float_return() {
    let ast = test_module(vec![function_with_params(
        "pi_val",
        &[],
        float_lit_expr(3.14),
        TypeKind::Float,
    )]);
    let (ir, _) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    assert!(ir_str.contains("pi_val"), "IR should contain pi_val function");
}

#[test]
fn test_function_with_bool_return() {
    let ast = test_module(vec![function_with_params(
        "always_true",
        &[],
        bool_lit_expr(true),
        TypeKind::Bool,
    )]);
    let (ir, _) = compile_to_llvm_ir(&ast);
    let ir_str = ir.as_str();
    assert!(ir_str.contains("always_true"), "IR should contain always_true function");
}

#[test]
fn test_function_many_params() {
    let ast = test_module(vec![function_with_params(
        "many_args",
        &[
            ("a", TypeKind::Int),
            ("b", TypeKind::Int),
            ("c", TypeKind::Int),
            ("d", TypeKind::Int),
            ("e", TypeKind::Int),
        ],
        ident_expr("a"),
        TypeKind::Int,
    )]);
    let (ir, _) = compile_to_llvm_ir(&ast);
    assert!(ir.as_str().contains("many_args"), "IR should contain many_args function");
}

// =============================================================================
// 4. LOWERING CONFIG TESTS
// =============================================================================

#[test]
fn test_lowering_config_debug() {
    let config = LoweringConfig::debug("test");
    assert_eq!(config.opt_level, 0);
    assert!(!config.cbgr_elimination);
    assert!(config.debug_info);
    assert_eq!(config.default_tier, RefTier::Tier0);
}

#[test]
fn test_lowering_config_release() {
    let config = LoweringConfig::release("test");
    assert_eq!(config.opt_level, 2);
    assert!(config.cbgr_elimination);
    assert!(!config.debug_info);
}

#[test]
fn test_lowering_config_aggressive() {
    let config = LoweringConfig::aggressive("test");
    assert_eq!(config.opt_level, 3);
    assert!(config.cbgr_elimination);
    assert_eq!(config.default_tier, RefTier::Tier1);
    assert_eq!(config.inline_threshold, 200);
}

#[test]
fn test_lowering_config_builder_pattern() {
    let config = LoweringConfig::new("mod")
        .with_opt_level(1)
        .with_cbgr_elimination(false)
        .with_default_tier(RefTier::Tier2)
        .with_debug_info(true)
        .with_coverage(true);
    assert_eq!(config.opt_level, 1);
    assert!(!config.cbgr_elimination);
    assert_eq!(config.default_tier, RefTier::Tier2);
    assert!(config.debug_info);
    assert!(config.coverage);
}

#[test]
fn test_lowering_config_opt_clamp() {
    let config = LoweringConfig::new("test").with_opt_level(255);
    assert_eq!(config.opt_level, 3, "opt_level should be clamped to 3");
}

// =============================================================================
// 5. CBGR REFERENCE CODEGEN TESTS
// =============================================================================

#[test]
fn test_cbgr_thin_ref_type_layout() {
    let context = Context::create();
    let cbgr = CbgrLowering::new(&context);

    let thin_ref = cbgr.thin_ref_type();
    // ThinRef: { ptr, generation: u32, epoch_caps: u32 }
    // Count should be 3 fields
    let field_count = thin_ref.count_fields();
    assert_eq!(field_count, 3, "ThinRef should have 3 fields (ptr, generation, epoch_caps)");
}

#[test]
fn test_cbgr_fat_ref_type_layout() {
    let context = Context::create();
    let cbgr = CbgrLowering::new(&context);

    let fat_ref = cbgr.fat_ref_type();
    // FatRef: { ptr, generation: u32, epoch_caps: u32, len: u64 }
    let field_count = fat_ref.count_fields();
    assert_eq!(field_count, 4, "FatRef should have 4 fields (ptr, generation, epoch_caps, len)");
}

#[test]
fn test_cbgr_stats_initial() {
    let context = Context::create();
    let cbgr = CbgrLowering::new(&context);

    let stats = cbgr.stats();
    assert_eq!(stats.refs_created, 0);
    assert_eq!(stats.tier0_refs, 0);
    assert_eq!(stats.tier1_refs, 0);
    assert_eq!(stats.tier2_refs, 0);
    assert_eq!(stats.runtime_checks, 0);
    assert_eq!(stats.checks_eliminated, 0);
}

#[test]
fn test_cbgr_stats_elimination_rate_empty() {
    let stats = CbgrStats::default();
    assert_eq!(stats.elimination_rate(), 0.0, "Empty stats should have 0% elimination rate");
}

#[test]
fn test_cbgr_stats_elimination_rate() {
    let stats = CbgrStats {
        refs_created: 10,
        tier0_refs: 5,
        tier1_refs: 3,
        tier2_refs: 2,
        runtime_checks: 3,
        checks_eliminated: 7,
    };
    let rate = stats.elimination_rate();
    assert!((rate - 0.7).abs() < 0.001, "Elimination rate should be 0.7, got {}", rate);
}

#[test]
fn test_thin_ref_size_constant() {
    use verum_codegen::llvm::THIN_REF_SIZE;
    assert_eq!(THIN_REF_SIZE, 16, "ThinRef should be 16 bytes");
}

#[test]
fn test_fat_ref_size_constant() {
    use verum_codegen::llvm::FAT_REF_SIZE;
    assert_eq!(FAT_REF_SIZE, 24, "FatRef should be 24 bytes");
}

// =============================================================================
// 6. LOWERING STATS TESTS
// =============================================================================

#[test]
fn test_lowering_stats_default() {
    let stats = LoweringStats::default();
    assert_eq!(stats.functions_lowered, 0);
    assert_eq!(stats.instructions_lowered, 0);
    assert_eq!(stats.basic_blocks, 0);
    assert_eq!(stats.total_refs(), 0);
    assert_eq!(stats.elimination_rate(), 0.0);
}

#[test]
fn test_lowering_stats_accumulate() {
    let ast = test_module(vec![
        simple_function("f1", 1),
        simple_function("f2", 2),
        simple_function("f3", 3),
    ]);
    let (_, stats) = compile_to_llvm_ir(&ast);
    assert!(stats.functions_lowered >= 3, "Should track function count");
    assert!(stats.instructions_lowered > 0, "Should track instruction count");
}

// =============================================================================
// 7. LLVM AVAILABILITY AND VERSION TESTS
// =============================================================================

#[test]
fn test_llvm_availability() {
    let result = verum_codegen::llvm::check_llvm_availability();
    assert!(result.is_ok(), "LLVM should be available");
}

#[test]
fn test_llvm_version_not_empty() {
    let version = verum_codegen::llvm::LLVM_VERSION;
    assert!(!version.is_empty(), "LLVM version should not be empty");
}

// =============================================================================
// 8. RELEASE CONFIG END-TO-END TEST
// =============================================================================

#[test]
fn test_release_config_lowering() {
    let ast = test_module(vec![simple_function("optimized_fn", 99)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast).expect("VBC compile failed");

    let context = Context::create();
    let config = LoweringConfig::release("opt_module");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    let result = lowering.lower_module(&vbc_module);
    assert!(result.is_ok(), "Release lowering should succeed: {:?}", result.err());

    let ir = lowering.get_ir();
    assert!(ir.as_str().contains("optimized_fn"), "IR should contain function");
}

#[test]
fn test_coverage_config_lowering() {
    let ast = test_module(vec![simple_function("covered_fn", 0)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast).expect("VBC compile failed");

    let context = Context::create();
    let config = LoweringConfig::new("coverage_test").with_coverage(true);
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    let result = lowering.lower_module(&vbc_module);
    assert!(result.is_ok(), "Coverage lowering should succeed: {:?}", result.err());

    let ir = lowering.get_ir();
    // Coverage should emit the coverage counter global
    assert!(
        ir.as_str().contains("__verum_coverage_counters"),
        "Coverage-enabled IR should contain coverage counters"
    );
}

// =============================================================================
// 9. DEBUG INFO TESTS
// =============================================================================

#[test]
fn test_debug_info_emission() {
    let ast = test_module(vec![simple_function("debug_fn", 42)]);

    let mut vbc_codegen = VbcCodegen::new();
    let vbc_module = vbc_codegen.compile_module(&ast).expect("VBC compile failed");

    let context = Context::create();
    let config = LoweringConfig::debug("debug_module");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    let result = lowering.lower_module(&vbc_module);
    assert!(result.is_ok(), "Debug lowering should succeed: {:?}", result.err());

    let ir = lowering.get_ir();
    let ir_str = ir.as_str();
    // Debug builds should contain DWARF metadata
    assert!(
        ir_str.contains("!dbg") || ir_str.contains("!DICompileUnit") || ir_str.contains("Debug Info Version"),
        "Debug IR should contain debug metadata"
    );
}

// =============================================================================
// 10. TARGET TRIPLE TESTS
// =============================================================================

#[test]
fn test_custom_target_triple() {
    let context = Context::create();
    let config = LoweringConfig::new("triple_test")
        .with_target("x86_64-unknown-linux-gnu");
    let lowering = VbcToLlvmLowering::new(&context, config);
    let ir = lowering.get_ir();
    assert!(
        ir.as_str().contains("x86_64") || ir.as_str().contains("target triple"),
        "IR should reflect custom target triple"
    );
}
