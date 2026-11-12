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
// Migrated from src/meta_sandbox.rs per AGENTS.md standards

use verum_ast::Span;
use verum_ast::attr::Attribute;
use verum_ast::decl::{FunctionBody, FunctionDecl, Visibility};
use verum_common::Text;
use verum_compiler::meta::sandbox::*;

fn default_span() -> Span {
    Span::default()
}

fn create_function_with_attribute(attr_name: &str) -> FunctionDecl {
    use verum_common::{List, Maybe};

    FunctionDecl {
        decreases: Default::default(),
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
        ctxs.push(ContextRequirement::simple(
            path,
            List::new(),
            default_span(),
        ));
        ctxs
    };

    FunctionDecl {
        decreases: Default::default(),
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
// Refusing removed name-based reflection
// ============================================================================

#[test]
fn sandbox_does_not_guess_layout_from_type_names_or_values() {
    use verum_common::List;
    use verum_compiler::meta::{ConstValue, MetaContext};
    let sandbox = MetaSandbox::new();
    let ctx = MetaContext::new();
    for name in ["size_of", "align_of", "stride_of", "bits_of", "type_id", "type_name"] {
        for args in [
            List::new(),
            List::from_iter([ConstValue::Text(Text::from("Int"))]),
            List::from_iter([ConstValue::Text(Text::from("Map"))]),
            List::from_iter([ConstValue::Int(42)]),
        ] {
            assert!(sandbox.execute_builtin_function(&Text::from(name), &args, &ctx).is_err(), "{name} guessed a result for {args:?}");
        }
    }
}
