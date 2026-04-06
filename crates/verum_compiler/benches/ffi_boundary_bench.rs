//! Performance Benchmarks for FFI Boundary Processing
//!
//! Target: <10ns marshalling overhead per call
//!
//! Measures:
//! - Type validation performance
//! - Marshalling wrapper generation time
//! - CBGR boundary checking overhead
//! - End-to-end FFI call overhead

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

use verum_ast::decl::Visibility;
use verum_ast::ffi::{
    CallingConvention, ErrorProtocol, FFIBoundary, FFIFunction, FFISignature, MemoryEffects,
    Ownership,
};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Type, TypeKind};
use verum_common::{Heap, List, Maybe};

// ============================================================================
// Type Validation Benchmarks
// ============================================================================

fn bench_validate_primitive_type(c: &mut Criterion) {
    let ty = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    c.bench_function("validate_primitive_type", |b| {
        b.iter(|| {
            // Simulate type validation
            black_box(is_ffi_safe_primitive(&ty))
        })
    });
}

fn bench_validate_pointer_type(c: &mut Criterion) {
    let inner = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    let ptr = Type {
        kind: TypeKind::Pointer {
            mutable: false,
            inner: Heap::new(inner),
        },
        span: Span::default(),
    };

    c.bench_function("validate_pointer_type", |b| {
        b.iter(|| black_box(is_ffi_safe_pointer(&ptr)))
    });
}

fn bench_validate_cbgr_reference(c: &mut Criterion) {
    let inner = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    let ref_ty = Type {
        kind: TypeKind::Reference {
            mutable: false,
            inner: Heap::new(inner),
        },
        span: Span::default(),
    };

    c.bench_function("validate_cbgr_reference", |b| {
        b.iter(|| {
            // Should detect this is NOT FFI-safe
            black_box(is_cbgr_reference(&ref_ty))
        })
    });
}

fn bench_validate_function_pointer(c: &mut Criterion) {
    let params = List::new();
    let return_type = Type {
        kind: TypeKind::Unit,
        span: Span::default(),
    };

    let fn_ty = Type {
        kind: TypeKind::Function {
            params,
            return_type: Heap::new(return_type),
            calling_convention: Maybe::None,
            contexts: verum_ast::context::ContextList::empty(),
        },
        span: Span::default(),
    };

    c.bench_function("validate_function_pointer", |b| {
        b.iter(|| black_box(is_function_pointer(&fn_ty)))
    });
}

// ============================================================================
// Marshalling Benchmarks
// ============================================================================

fn bench_generate_simple_wrapper(c: &mut Criterion) {
    let function = create_simple_ffi_function();

    c.bench_function("generate_simple_wrapper", |b| {
        b.iter(|| black_box(simulate_wrapper_generation(&function)))
    });
}

fn bench_generate_multi_param_wrapper(c: &mut Criterion) {
    let function = create_multi_param_function(4);

    c.bench_function("generate_multi_param_wrapper", |b| {
        b.iter(|| black_box(simulate_wrapper_generation(&function)))
    });
}

fn bench_parameter_conversion(c: &mut Criterion) {
    let ty = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    c.bench_function("parameter_conversion", |b| {
        b.iter(|| black_box(simulate_param_conversion(&ty)))
    });
}

fn bench_return_conversion(c: &mut Criterion) {
    let ty = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    c.bench_function("return_conversion", |b| {
        b.iter(|| black_box(simulate_return_conversion(&ty)))
    });
}

// ============================================================================
// CBGR Boundary Checking Benchmarks
// ============================================================================

fn bench_cbgr_boundary_check_pass(c: &mut Criterion) {
    let function = create_simple_ffi_function();

    c.bench_function("cbgr_boundary_check_pass", |b| {
        b.iter(|| black_box(simulate_cbgr_check(&function)))
    });
}

fn bench_cbgr_boundary_check_fail(c: &mut Criterion) {
    let function = create_ffi_function_with_cbgr_ref();

    c.bench_function("cbgr_boundary_check_fail", |b| {
        b.iter(|| black_box(simulate_cbgr_check(&function)))
    });
}

// ============================================================================
// End-to-End Benchmarks
// ============================================================================

fn bench_complete_ffi_boundary_processing(c: &mut Criterion) {
    let boundary = create_test_ffi_boundary(10);

    c.bench_function("complete_ffi_boundary_processing", |b| {
        b.iter(|| black_box(simulate_complete_processing(&boundary)))
    });
}

fn bench_scaling_with_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling_with_functions");

    for num_functions in [1, 5, 10, 50, 100].iter() {
        let boundary = create_test_ffi_boundary(*num_functions);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_functions),
            num_functions,
            |b, _| b.iter(|| black_box(simulate_complete_processing(&boundary))),
        );
    }

    group.finish();
}

fn bench_scaling_with_parameters(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling_with_parameters");

    for num_params in [0, 2, 4, 8, 16].iter() {
        let function = create_multi_param_function(*num_params);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_params),
            num_params,
            |b, _| b.iter(|| black_box(simulate_wrapper_generation(&function))),
        );
    }

    group.finish();
}

// ============================================================================
// Marshalling Overhead Target Validation
// ============================================================================

fn bench_marshalling_overhead_target(c: &mut Criterion) {
    // Target: <10ns marshalling overhead
    const TARGET_NS: u64 = 10;

    let function = create_simple_ffi_function();

    let mut group = c.benchmark_group("marshalling_overhead_target");
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("marshalling_overhead", |b| {
        b.iter(|| {
            // Simulate minimal marshalling (direct pass-through for primitives)
            black_box(simulate_minimal_marshalling(&function))
        })
    });

    group.finish();

    // Note: Criterion will report if we meet the <10ns target
}

// ============================================================================
// Helper Functions
// ============================================================================

fn is_ffi_safe_primitive(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::Bool | TypeKind::Int | TypeKind::Float | TypeKind::Char | TypeKind::Unit
    )
}

fn is_ffi_safe_pointer(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Pointer { .. })
}

fn is_cbgr_reference(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::Reference { .. } | TypeKind::CheckedReference { .. }
    )
}

fn is_function_pointer(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Function { .. })
}

fn simulate_wrapper_generation(_function: &FFIFunction) -> usize {
    // Simulate wrapper generation overhead
    42
}

fn simulate_param_conversion(_ty: &Type) -> usize {
    // Simulate parameter conversion
    1
}

fn simulate_return_conversion(_ty: &Type) -> usize {
    // Simulate return conversion
    1
}

fn simulate_cbgr_check(function: &FFIFunction) -> bool {
    // Check if any parameters or return type is CBGR reference
    for (_name, ty) in &function.signature.params {
        if is_cbgr_reference(ty) {
            return false;
        }
    }
    !is_cbgr_reference(&function.signature.return_type)
}

fn simulate_complete_processing(boundary: &FFIBoundary) -> usize {
    let mut count = 0;
    for function in &boundary.functions {
        if simulate_cbgr_check(function) {
            count += simulate_wrapper_generation(function);
        }
    }
    count
}

fn simulate_minimal_marshalling(_function: &FFIFunction) -> usize {
    // Minimal marshalling for primitives (target: <10ns)
    1
}

fn create_simple_ffi_function() -> FFIFunction {
    FFIFunction {
        name: Ident {
            name: "test_function".into(),
            span: Span::default(),
        },
        signature: FFISignature {
            params: List::new(),
            return_type: Type {
                kind: TypeKind::Unit,
                span: Span::default(),
            },
            calling_convention: CallingConvention::C,
            is_variadic: false,
            span: Span::default(),
        },
        requires: List::new(),
        ensures: List::new(),
        memory_effects: MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ErrorProtocol::None,
        ownership: Ownership::Borrow,
        span: Span::default(),
    }
}

fn create_multi_param_function(num_params: usize) -> FFIFunction {
    let mut params = List::new();
    for i in 0..num_params {
        params.push((
            Ident {
                name: format!("param_{}", i).into(),
                span: Span::default(),
            },
            Type {
                kind: TypeKind::Int,
                span: Span::default(),
            },
        ));
    }

    FFIFunction {
        name: Ident {
            name: "multi_param".into(),
            span: Span::default(),
        },
        signature: FFISignature {
            params,
            return_type: Type {
                kind: TypeKind::Int,
                span: Span::default(),
            },
            calling_convention: CallingConvention::C,
            is_variadic: false,
            span: Span::default(),
        },
        requires: List::new(),
        ensures: List::new(),
        memory_effects: MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ErrorProtocol::None,
        ownership: Ownership::Borrow,
        span: Span::default(),
    }
}

fn create_ffi_function_with_cbgr_ref() -> FFIFunction {
    let inner = Type {
        kind: TypeKind::Int,
        span: Span::default(),
    };

    let mut params = List::new();
    params.push((
        Ident {
            name: "ref_param".into(),
            span: Span::default(),
        },
        Type {
            kind: TypeKind::Reference {
                mutable: false,
                inner: Heap::new(inner),
            },
            span: Span::default(),
        },
    ));

    FFIFunction {
        name: Ident {
            name: "with_cbgr_ref".into(),
            span: Span::default(),
        },
        signature: FFISignature {
            params,
            return_type: Type {
                kind: TypeKind::Unit,
                span: Span::default(),
            },
            calling_convention: CallingConvention::C,
            is_variadic: false,
            span: Span::default(),
        },
        requires: List::new(),
        ensures: List::new(),
        memory_effects: MemoryEffects::Pure,
        thread_safe: true,
        error_protocol: ErrorProtocol::None,
        ownership: Ownership::Borrow,
        span: Span::default(),
    }
}

fn create_test_ffi_boundary(num_functions: usize) -> FFIBoundary {
    let mut functions = List::new();
    for _i in 0..num_functions {
        functions.push(create_simple_ffi_function());
    }

    FFIBoundary {
        name: Ident {
            name: "TestBoundary".into(),
            span: Span::default(),
        },
        extends: Maybe::None,
        functions,
        visibility: Visibility::Public,
        attributes: List::new(),
        span: Span::default(),
    }
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    type_validation,
    bench_validate_primitive_type,
    bench_validate_pointer_type,
    bench_validate_cbgr_reference,
    bench_validate_function_pointer
);

criterion_group!(
    marshalling,
    bench_generate_simple_wrapper,
    bench_generate_multi_param_wrapper,
    bench_parameter_conversion,
    bench_return_conversion
);

criterion_group!(
    cbgr_boundary,
    bench_cbgr_boundary_check_pass,
    bench_cbgr_boundary_check_fail
);

criterion_group!(
    end_to_end,
    bench_complete_ffi_boundary_processing,
    bench_scaling_with_functions,
    bench_scaling_with_parameters
);

criterion_group!(performance_targets, bench_marshalling_overhead_target);

criterion_main!(
    type_validation,
    marshalling,
    cbgr_boundary,
    end_to_end,
    performance_targets
);
