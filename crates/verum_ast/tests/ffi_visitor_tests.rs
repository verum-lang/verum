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
// Tests for FFI Boundary Visitor Pattern.
//
// These tests verify that the visitor pattern correctly traverses FFI boundaries

use verum_ast::*;
use verum_common::{List, Maybe};

struct CountingVisitor {
    boundary_count: usize,
    type_count: usize,
}

impl visitor::Visitor for CountingVisitor {
    fn visit_ffi_boundary(&mut self, ffi_boundary: &FFIBoundary) {
        self.boundary_count += 1;
        visitor::walk_ffi_boundary(self, ffi_boundary);
    }

    fn visit_type(&mut self, ty: &Type) {
        self.type_count += 1;
        visitor::walk_type(self, ty);
    }
}

#[test]
fn test_visitor_counts_ffi_boundary() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 100, file_id);

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

    let mut functions = List::new();
    functions.push(ffi::FFIFunction {
        name: ty::Ident::new("sqrt", span),
        signature,
        requires: List::new(),
        ensures: List::new(),
        memory_effects: ffi::MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ffi::ErrorProtocol::None,
        ownership: ffi::Ownership::Borrow,
        span,
    });

    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("MathLib", span),
        extends: Maybe::None,
        functions,
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    let item = Item::new(ItemKind::FFIBoundary(boundary), span);

    let mut visitor = CountingVisitor {
        boundary_count: 0,
        type_count: 0,
    };

    visitor.visit_item(&item);

    // Should count 1 FFI boundary
    assert_eq!(visitor.boundary_count, 1);

    // Should count parameter type and return type (2 types)
    assert!(visitor.type_count >= 2);
}

#[test]
fn test_visitor_traverses_ffi_function_types() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 50, file_id);

    let mut params = List::new();
    params.push((
        ty::Ident::new("input", span),
        ty::Type::new(ty::TypeKind::Int, span),
    ));
    params.push((
        ty::Ident::new("value", span),
        ty::Type::new(ty::TypeKind::Float, span),
    ));

    let signature = ffi::FFISignature {
        params,
        return_type: ty::Type::new(ty::TypeKind::Int, span),
        calling_convention: ffi::CallingConvention::C,
        is_variadic: false,
        span,
    };

    let mut functions = List::new();
    functions.push(ffi::FFIFunction {
        name: ty::Ident::new("process", span),
        signature,
        requires: List::new(),
        ensures: List::new(),
        memory_effects: ffi::MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ffi::ErrorProtocol::None,
        ownership: ffi::Ownership::Borrow,
        span,
    });

    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("ProcessLib", span),
        extends: Maybe::None,
        functions,
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    let mut visitor = CountingVisitor {
        boundary_count: 0,
        type_count: 0,
    };

    visitor.visit_ffi_boundary(&boundary);

    // Should count the boundary itself
    assert_eq!(visitor.boundary_count, 1);

    // Should count all types: 2 params + 1 return = 3
    assert_eq!(visitor.type_count, 3);
}

#[test]
fn test_visitor_in_module() {
    let file_id = span::FileId::new(0);
    let span = span::Span::new(0, 200, file_id);

    let boundary = ffi::FFIBoundary {
        name: ty::Ident::new("TestLib", span),
        extends: Maybe::None,
        functions: List::new(),
        visibility: decl::Visibility::Public,
        attributes: List::new(),
        span,
    };

    let item = Item::new(ItemKind::FFIBoundary(boundary), span);

    let mut items = List::new();
    items.push(item);

    let module = Module::new(items, file_id, span);

    let mut visitor = CountingVisitor {
        boundary_count: 0,
        type_count: 0,
    };

    for item in module.items.iter() {
        visitor.visit_item(item);
    }

    assert_eq!(visitor.boundary_count, 1);
}
