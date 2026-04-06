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
// Migrated from src/meta_sandbox.rs per CLAUDE.md standards

use verum_ast::Span;
use verum_ast::attr::Attribute;
use verum_ast::decl::{FunctionBody, FunctionDecl, Visibility};
use verum_compiler::meta::sandbox::*;
use verum_common::Text;

fn default_span() -> Span {
    Span::default()
}

fn create_function_with_attribute(attr_name: &str) -> FunctionDecl {
    use verum_common::{List, Maybe};

    FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: true,
        stage_level: 1,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: verum_ast::Ident::new(Text::from("test_fn"), default_span()),
        generics: List::new(),
        params: List::new(),
        throws_clause: Maybe::None,
        return_type: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: if attr_name.is_empty() {
            List::new()
        } else {
            let mut attrs = List::new();
            attrs.push(Attribute::simple(Text::from(attr_name), default_span()));
            attrs
        },
        body: Maybe::None,
        span: default_span(),
    }
}

fn create_function_with_context(context_name: &str) -> FunctionDecl {
    use verum_ast::decl::ContextRequirement;
    use verum_ast::ty::{Path, PathSegment};
    use verum_common::{List, Maybe};

    let contexts = if context_name.is_empty() {
        List::new()
    } else {
        let mut ctxs = List::new();
        let mut segments = List::new();
        segments.push(PathSegment::Name(verum_ast::Ident::new(
            Text::from(context_name),
            default_span(),
        )));
        let path = Path::new(segments, default_span());
        ctxs.push(ContextRequirement::simple(path, List::new(), default_span()));
        ctxs
    };

    FunctionDecl {
        visibility: Visibility::Public,
        is_async: false,
        is_meta: true,
        stage_level: 1,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: verum_ast::Ident::new(Text::from("test_fn"), default_span()),
        generics: List::new(),
        params: List::new(),
        throws_clause: Maybe::None,
        return_type: Maybe::None,
        std_attr: Maybe::None,
        contexts,
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: default_span(),
    }
}

#[test]
fn test_sandbox_allows_arithmetic() {
    let sandbox = MetaSandbox::new();
    assert!(sandbox.is_operation_allowed(Operation::Arithmetic));
}

#[test]
fn test_is_forbidden_io_function() {
    let sandbox = MetaSandbox::new();

    // Filesystem operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.fs.read")));
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.fs.write")));
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.fs.read_file")));
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.fs.write_file")));

    // Network operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.net.tcp_connect")));
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.net.http_get")));

    // Process operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.process.spawn")));
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.process.exec")));

    // Time operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.time.now")));

    // Environment operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.env.var")));

    // Random without seed should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.random.gen")));

    // FFI should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.ffi.call")));

    // Unsafe operations should be forbidden
    assert!(sandbox.is_forbidden_io_function(&Text::from("std.mem.transmute")));

    // Pure functions should NOT be forbidden
    assert!(!sandbox.is_forbidden_io_function(&Text::from("compute_sum")));
    assert!(!sandbox.is_forbidden_io_function(&Text::from("len")));
    assert!(!sandbox.is_forbidden_io_function(&Text::from("add")));
}

// ============================================================================
// Asset Loading Tests (via BuildAssets context)
// ============================================================================

#[test]
fn test_asset_loading_disabled_by_default() {
    let sandbox = MetaSandbox::new();
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_enable_disable_asset_loading() {
    let sandbox = MetaSandbox::new();

    // Initially disabled
    assert!(!sandbox.is_asset_loading_allowed());

    // Enable asset loading
    sandbox.enable_asset_loading();
    assert!(sandbox.is_asset_loading_allowed());

    // Disable asset loading
    sandbox.disable_asset_loading();
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_with_asset_loading_scope() {
    let sandbox = MetaSandbox::new();

    // Initially disabled
    assert!(!sandbox.is_asset_loading_allowed());

    // Scoped enable
    let result = sandbox.with_asset_loading(|| {
        assert!(sandbox.is_asset_loading_allowed());
        42
    });

    // Returns correct value
    assert_eq!(result, 42);

    // Disabled after scope
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_with_asset_loading_nested() {
    let sandbox = MetaSandbox::new();

    // Initially disabled
    assert!(!sandbox.is_asset_loading_allowed());

    // Nested scopes should preserve outer state
    sandbox.with_asset_loading(|| {
        assert!(sandbox.is_asset_loading_allowed());

        // Nested call should also be enabled
        sandbox.with_asset_loading(|| {
            assert!(sandbox.is_asset_loading_allowed());
        });

        // Still enabled after nested scope
        assert!(sandbox.is_asset_loading_allowed());
    });

    // Disabled after all scopes
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_is_asset_loading_function() {
    let sandbox = MetaSandbox::new();

    // These should be asset loading functions
    assert!(sandbox.is_asset_loading_function(&Text::from("load_build_asset")));
    assert!(sandbox.is_asset_loading_function(&Text::from("include_str")));
    assert!(sandbox.is_asset_loading_function(&Text::from("include_bytes")));
    assert!(sandbox.is_asset_loading_function(&Text::from("include_file")));
    assert!(sandbox.is_asset_loading_function(&Text::from("embed_file")));

    // These should NOT be asset loading functions
    assert!(!sandbox.is_asset_loading_function(&Text::from("len")));
    assert!(!sandbox.is_asset_loading_function(&Text::from("typeof")));
    assert!(!sandbox.is_asset_loading_function(&Text::from("print")));
}

#[test]
fn test_has_build_assets_context() {
    // Function WITH BuildAssets context
    let func_with_ctx = create_function_with_context("BuildAssets");
    assert!(MetaSandbox::has_build_assets_context(&func_with_ctx));

    // Function WITHOUT BuildAssets context
    let func_without_ctx = create_function_with_context("");
    assert!(!MetaSandbox::has_build_assets_context(&func_without_ctx));

    // Function with different context
    let func_other_ctx = create_function_with_context("Database");
    assert!(!MetaSandbox::has_build_assets_context(&func_other_ctx));
}

#[test]
fn test_execute_meta_function_with_asset_loading() {
    let sandbox = MetaSandbox::new();

    // Initially disabled
    assert!(!sandbox.is_asset_loading_allowed());

    // Function with BuildAssets context should enable asset loading during execution
    let func_with_ctx = create_function_with_context("BuildAssets");
    sandbox.execute_meta_function(&func_with_ctx, || {
        assert!(sandbox.is_asset_loading_allowed());
    });

    // Disabled after execution
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_execute_meta_function_without_asset_loading() {
    let sandbox = MetaSandbox::new();

    // Initially disabled
    assert!(!sandbox.is_asset_loading_allowed());

    // Function without BuildAssets context should NOT enable asset loading
    let func_without_ctx = create_function_with_context("");
    sandbox.execute_meta_function(&func_without_ctx, || {
        assert!(!sandbox.is_asset_loading_allowed());
    });

    // Still disabled after execution
    assert!(!sandbox.is_asset_loading_allowed());
}

#[test]
fn test_sandbox_clone_preserves_asset_loading_state() {
    let sandbox = MetaSandbox::new();

    // Enable asset loading
    sandbox.enable_asset_loading();
    assert!(sandbox.is_asset_loading_allowed());

    // Clone should preserve the state
    let cloned = sandbox.clone();
    assert!(cloned.is_asset_loading_allowed());

    // Modifying original should not affect clone
    sandbox.disable_asset_loading();
    assert!(!sandbox.is_asset_loading_allowed());
    assert!(cloned.is_asset_loading_allowed());
}

// ============================================================================
// Type Size and Alignment Computation Tests
// ============================================================================

// Note: In Verum language proper, use T.size type property instead of size_of()
// See: vcs/specs/L3-extended/meta/compile_time/type_introspection.vr
// This test validates internal MetaSandbox functionality
#[test]
fn test_builtin_function_size_of() {
    use verum_compiler::meta::{ConstValue, MetaContext};

    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();

    // Test size_of for primitive types via Text values
    let test_cases = [
        (Text::from("i8"), 1),
        (Text::from("u8"), 1),
        (Text::from("i16"), 2),
        (Text::from("u16"), 2),
        (Text::from("i32"), 4),
        (Text::from("u32"), 4),
        (Text::from("i64"), 8),
        (Text::from("u64"), 8),
        (Text::from("f32"), 4),
        (Text::from("f64"), 8),
        (Text::from("bool"), 1),
        (Text::from("char"), 4),
        (Text::from("isize"), 8),
        (Text::from("usize"), 8),
    ];

    for (type_name, expected_size) in test_cases {
        let args = vec![ConstValue::Text(type_name.clone())];
        let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);

        assert!(
            result.is_ok(),
            "size_of failed for type: {}",
            type_name.as_str()
        );
        if let Ok(ConstValue::Int(size)) = result {
            assert_eq!(
                size,
                expected_size,
                "Expected size {} for type {}, got {}",
                expected_size,
                type_name.as_str(),
                size
            );
        }
    }
}

// Note: In Verum language proper, use T.alignment type property instead of align_of()
// See: vcs/specs/L3-extended/meta/compile_time/type_introspection.vr
// This test validates internal MetaSandbox functionality
#[test]
fn test_builtin_function_align_of() {
    use verum_compiler::meta::{ConstValue, MetaContext};

    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();

    // Test align_of for primitive types
    let test_cases = [
        (Text::from("i8"), 1),
        (Text::from("u8"), 1),
        (Text::from("i16"), 2),
        (Text::from("u16"), 2),
        (Text::from("i32"), 4),
        (Text::from("u32"), 4),
        (Text::from("i64"), 8),
        (Text::from("u64"), 8),
        (Text::from("f32"), 4),
        (Text::from("f64"), 8),
        (Text::from("bool"), 1),
        (Text::from("char"), 4),
    ];

    for (type_name, expected_align) in test_cases {
        let args = vec![ConstValue::Text(type_name.clone())];
        let result = sandbox.execute_builtin_function(&Text::from("align_of"), &args, &ctx);

        assert!(
            result.is_ok(),
            "align_of failed for type: {}",
            type_name.as_str()
        );
        if let Ok(ConstValue::Int(align)) = result {
            assert_eq!(
                align,
                expected_align,
                "Expected alignment {} for type {}, got {}",
                expected_align,
                type_name.as_str(),
                align
            );
        }
    }
}

// Note: In Verum language proper, use type properties: List.size, Map.size, Text.size
// This test validates internal MetaSandbox functionality
#[test]
fn test_size_of_collection_types() {
    use verum_compiler::meta::{ConstValue, MetaContext};

    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();

    // Collection types are heap-allocated, so we check their handle sizes
    let args = vec![ConstValue::Text(Text::from("List"))];
    let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);
    assert!(result.is_ok());
    if let Ok(ConstValue::Int(size)) = result {
        assert_eq!(size, 24, "List should be 24 bytes (ptr + len + capacity)");
    }

    let args = vec![ConstValue::Text(Text::from("Map"))];
    let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);
    assert!(result.is_ok());
    if let Ok(ConstValue::Int(size)) = result {
        assert_eq!(size, 8, "Map should be 8 bytes (heap pointer)");
    }

    let args = vec![ConstValue::Text(Text::from("Text"))];
    let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);
    assert!(result.is_ok());
    if let Ok(ConstValue::Int(size)) = result {
        assert_eq!(size, 24, "Text should be 24 bytes (ptr + len + capacity)");
    }
}

#[test]
fn test_size_of_requires_argument() {
    use verum_compiler::meta::{ConstValue, MetaContext};

    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();

    // size_of without arguments should fail
    let args: Vec<ConstValue> = vec![];
    let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);
    assert!(result.is_err(), "size_of should require an argument");

    // align_of without arguments should fail
    let result = sandbox.execute_builtin_function(&Text::from("align_of"), &args, &ctx);
    assert!(result.is_err(), "align_of should require an argument");
}

#[test]
fn test_size_of_tuple_values() {
    use verum_compiler::meta::{ConstValue, MetaContext};

    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();

    // Tuple of two i32s should be 8 bytes
    let args = vec![ConstValue::Tuple(vec![
        ConstValue::Int(0),
        ConstValue::Int(0),
    ].into())];
    let result = sandbox.execute_builtin_function(&Text::from("size_of"), &args, &ctx);
    assert!(result.is_ok());
    if let Ok(ConstValue::Int(size)) = result {
        // Two i64 values (8 bytes each) = 16 bytes
        assert_eq!(size, 16, "Tuple of two i64s should be 16 bytes");
    }
}
