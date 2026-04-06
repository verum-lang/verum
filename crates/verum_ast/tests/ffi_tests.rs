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
// Tests for FFI Boundary Declarations.
//
// These tests verify that FFI boundaries are properly represented in the AST
// Tests for FFI boundary AST nodes (C ABI interop specifications).

use verum_ast::*;
use verum_common::{List, Maybe};

#[test]
fn test_ffi_boundary_creation() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 10, file_id);

    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("LibMath", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    assert_eq!(boundary.name.name.as_str(), "LibMath");
    assert_eq!(boundary.span.start, 0);
    assert!(boundary.functions.is_empty());
}

#[test]
fn test_ffi_function_creation() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 20, file_id);

    let mut params = List::new();
    params.push((
        ty::Ident::new("x", span),
        ty::Type::new(ty::TypeKind::Float, span),
    ));

    let signature = ffi::FFISignature {
        params,
        return_type: ty::Type::new(ty::TypeKind::Float, span),
        calling_convention: ffi::CallingConvention::C,
        is_variadic: false,
        span,
    };

    let ffi_func = ffi::FFIFunction {
        name: ty::Ident::new("sqrt", span),
        signature,
        requires: List::new(),
        ensures: List::new(),
        memory_effects: ffi::MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ffi::ErrorProtocol::None,
        ownership: ffi::Ownership::Borrow,
        span,
    };

    assert_eq!(ffi_func.name.name.as_str(), "sqrt");
    assert_eq!(ffi_func.memory_effects, ffi::MemoryEffects::Pure);
    assert!(ffi_func.thread_safe);
}

#[test]
fn test_calling_convention_variants() {
    assert_eq!(ffi::CallingConvention::C.as_str(), "C");
    assert_eq!(ffi::CallingConvention::StdCall.as_str(), "stdcall");
    assert_eq!(ffi::CallingConvention::FastCall.as_str(), "fastcall");
    assert_eq!(ffi::CallingConvention::SysV64.as_str(), "sysv64");
}

#[test]
fn test_ownership_variants() {
    assert_eq!(ffi::Ownership::Borrow.as_str(), "borrow");
    assert_eq!(ffi::Ownership::Shared.as_str(), "shared");
    assert_eq!(
        ffi::Ownership::TransferTo("C".into()).as_str(),
        "transfer_to"
    );
    assert_eq!(
        ffi::Ownership::TransferFrom("C".into()).as_str(),
        "transfer_from"
    );
}

#[test]
fn test_error_protocol_none() {
    let protocol = ffi::ErrorProtocol::None;
    match protocol {
        ffi::ErrorProtocol::None => {}
        _ => panic!("Expected None error protocol"),
    }
}

#[test]
fn test_error_protocol_errno() {
    let protocol = ffi::ErrorProtocol::Errno;
    match protocol {
        ffi::ErrorProtocol::Errno => {}
        _ => panic!("Expected Errno error protocol"),
    }
}

#[test]
fn test_error_protocol_return_code() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 5, file_id);

    let code_expr = expr::Expr::new(
        expr::ExprKind::Literal(literal::Literal::int(0, span)),
        span,
    );

    let protocol = ffi::ErrorProtocol::ReturnCode(code_expr);
    match protocol {
        ffi::ErrorProtocol::ReturnCode(_) => {}
        _ => panic!("Expected ReturnCode error protocol"),
    }
}

#[test]
fn test_error_protocol_return_value() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 5, file_id);

    let null_expr = expr::Expr::new(
        expr::ExprKind::Literal(literal::Literal::int(0, span)),
        span,
    );

    let protocol = ffi::ErrorProtocol::ReturnValue(null_expr);
    match protocol {
        ffi::ErrorProtocol::ReturnValue(_) => {}
        _ => panic!("Expected ReturnValue error protocol"),
    }
}

#[test]
fn test_error_protocol_return_value_with_errno() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 5, file_id);

    let null_expr = expr::Expr::new(
        expr::ExprKind::Literal(literal::Literal::int(0, span)),
        span,
    );

    let protocol = ffi::ErrorProtocol::ReturnValueWithErrno(Box::new(null_expr));
    match protocol {
        ffi::ErrorProtocol::ReturnValueWithErrno(_) => {}
        _ => panic!("Expected ReturnValueWithErrno error protocol"),
    }
}

#[test]
fn test_error_protocol_exception() {
    let protocol = ffi::ErrorProtocol::Exception;
    match protocol {
        ffi::ErrorProtocol::Exception => {}
        _ => panic!("Expected Exception error protocol"),
    }
}

#[test]
fn test_memory_effects_pure() {
    let effects = ffi::MemoryEffects::Pure;
    assert_eq!(effects, ffi::MemoryEffects::Pure);
}

#[test]
fn test_memory_effects_reads() {
    let mut ranges = List::new();
    ranges.push("x".into());

    let effects = ffi::MemoryEffects::Reads(Maybe::Some(ranges));
    match effects {
        ffi::MemoryEffects::Reads(Maybe::Some(refs)) => {
            assert_eq!(refs.len(), 1);
        }
        _ => panic!("Expected Reads memory effects"),
    }
}

#[test]
fn test_memory_effects_writes() {
    let mut ranges = List::new();
    ranges.push("buf".into());

    let effects = ffi::MemoryEffects::Writes(Maybe::Some(ranges));
    match effects {
        ffi::MemoryEffects::Writes(Maybe::Some(refs)) => {
            assert_eq!(refs.len(), 1);
        }
        _ => panic!("Expected Writes memory effects"),
    }
}

#[test]
fn test_memory_effects_allocates() {
    let effects = ffi::MemoryEffects::Allocates;
    assert_eq!(effects, ffi::MemoryEffects::Allocates);
}

#[test]
fn test_memory_effects_deallocates() {
    let effects = ffi::MemoryEffects::Deallocates(Maybe::Some("ptr".into()));
    match effects {
        ffi::MemoryEffects::Deallocates(Maybe::Some(_)) => {}
        _ => panic!("Expected Deallocates memory effects"),
    }
}

#[test]
fn test_ffi_boundary_in_item() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 50, file_id);

    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("TestLib", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    let item = Item::new(ItemKind::FFIBoundary(boundary), span);

    match &item.kind {
        ItemKind::FFIBoundary(ffi_boundary) => {
            assert_eq!(ffi_boundary.name.name.as_str(), "TestLib");
        }
        _ => panic!("Expected FFIBoundary in ItemKind"),
    }
}

#[test]
fn test_ffi_is_compile_time_spec_not_type() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 10, file_id);

    // FFIBoundary is in ItemKind, not TypeKind
    let ident = ty::Ident::new("FFIBoundary", span);
    let path = ty::Path::from_ident(ident);

    // Confirm that FFI boundaries are not types
    if let ty::TypeKind::Path(_) = ty::TypeKind::Path(path) {
        // Paths are types, but FFIBoundary is NOT a type declaration
        // It's in ItemKind, ensuring it cannot be used as a type annotation
    }

    // FFIBoundary is an ItemKind
    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("Boundary", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Private,
        attributes: List::new(),
        span,
    };

    let item = Item::new(ItemKind::FFIBoundary(boundary), span);
    assert!(matches!(&item.kind, ItemKind::FFIBoundary(_)));
}

#[test]
fn test_ffi_seven_mandatory_components() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 50, file_id);

    let params = List::new();
    let signature = ffi::FFISignature {
        params,
        return_type: ty::Type::new(ty::TypeKind::Int, span),
        calling_convention: ffi::CallingConvention::C,
        is_variadic: false,
        span,
    };

    let ffi_func = ffi::FFIFunction {
        name: ty::Ident::new("ffi_func", span),
        signature,
        requires: List::new(),
        ensures: List::new(),
        memory_effects: ffi::MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ffi::ErrorProtocol::None,
        ownership: ffi::Ownership::Borrow,
        span,
    };

    assert_eq!(
        ffi_func.signature.calling_convention,
        ffi::CallingConvention::C
    );
    assert!(ffi_func.requires.is_empty());
    assert!(ffi_func.ensures.is_empty());
    assert_eq!(ffi_func.memory_effects, ffi::MemoryEffects::Pure);
    assert!(ffi_func.thread_safe);
    assert!(matches!(ffi_func.error_protocol, ffi::ErrorProtocol::None));
    assert!(matches!(ffi_func.ownership, ffi::Ownership::Borrow));
}

#[test]
fn test_ffi_boundary_with_preconditions() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 50, file_id);

    let mut requires = List::new();
    requires.push(expr::Expr::new(
        expr::ExprKind::Literal(literal::Literal::float(0.0, span)),
        span,
    ));

    let params = List::new();
    let signature = ffi::FFISignature {
        params,
        return_type: ty::Type::new(ty::TypeKind::Float, span),
        calling_convention: ffi::CallingConvention::C,
        is_variadic: false,
        span,
    };

    let ffi_func = ffi::FFIFunction {
        name: ty::Ident::new("sqrt", span),
        signature,
        requires,
        ensures: List::new(),
        memory_effects: ffi::MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ffi::ErrorProtocol::None,
        ownership: ffi::Ownership::Borrow,
        span,
    };

    assert_eq!(ffi_func.requires.len(), 1);
}

#[test]
fn test_visibility_in_ffi_boundary() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 20, file_id);

    let boundary_pub = ffi::FFIBoundary {
        name: ty::Ident::new("PublicLib", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    assert!(boundary_pub.visibility.is_public());

    let boundary_priv = ffi::FFIBoundary {
        name: ty::Ident::new("PrivateLib", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Private,
        attributes: List::new(),
        span,
    };

    assert!(!boundary_priv.visibility.is_public());
}
