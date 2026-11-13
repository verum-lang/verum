//! Imported obligations must not copy unrelated bodies from their source owner.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::time::Duration;
use verum_ast::{FileId, Module};
use verum_common::{List, Shared, Text};
use verum_fast_parser::FastParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::TypeChecker;

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_BYTES: AtomicUsize = AtomicUsize::new(0);

struct MeasuredAllocator(verum_common::memory_budget::BudgetedAllocator);

fn record_allocation(pointer: *mut u8, bytes: usize) {
    if !pointer.is_null() && COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOCATION_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }
}

// SAFETY: all operations forward unchanged to the budgeted allocator. Optional
// counters use only atomics, never allocate, and do not alter its live/peak/ceiling checks.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's allocation contract is forwarded unchanged.
        let pointer = unsafe { self.0.alloc(layout) };
        record_allocation(pointer, layout.size());
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's allocation contract is forwarded unchanged.
        let pointer = unsafe { self.0.alloc_zeroed(layout) };
        record_allocation(pointer, layout.size());
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: pointer, layout and size retain the caller's realloc contract.
        let pointer = unsafe { self.0.realloc(pointer, layout, size) };
        record_allocation(pointer, size);
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout retain the caller's deallocation contract.
        unsafe { self.0.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOC: MeasuredAllocator =
    MeasuredAllocator(verum_common::memory_budget::BudgetedAllocator::new());

fn check(owner: &str, module: &Module, registry: &Shared<parking_lot::RwLock<ModuleRegistry>>) {
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path(owner);
    checker.set_current_cog("bench");
    checker.set_module_registry(registry.clone());
    let errors = checker.prepare_module_declarations(module);
    assert!(errors.is_empty(), "{errors:?}");
    for item in &module.items {
        checker.check_item(item).expect("valid benchmark source");
    }
    assert!(checker.take_deferred_errors().is_empty());
    assert!(
        !checker
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.is_error())
    );
}

fn bench_source_imports(c: &mut Criterion) {
    let mut group = c.benchmark_group("source_imports");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));
    for body_statements in [128, 1024, 4096] {
        let mut provider = Text::from(
            "public type P is protocol {}; implement P for Int {}\n\
             public type Requires<T:P> is { public value:T };\n\
             fn unrelated_body() -> Int {\n",
        );
        for index in 0..body_statements {
            provider.push_str(&format!("let local_{index} = {index};\n"));
        }
        provider.push_str("73 }\n");
        let mut consumer = Text::from("mount bench.provider.Requires;\n");
        for index in 0..32 {
            consumer.push_str(&format!(
                "fn read_{index}(value:Requires<Int>) -> Int {{ value.value }}\n",
            ));
        }
        let mut registry = ModuleRegistry::new();
        let mut modules = List::new();
        for (index, (owner, source)) in [("bench.provider", provider), ("bench.consumer", consumer)]
            .into_iter()
            .enumerate()
        {
            let ast = FastParser::new()
                .parse_module_str(&source, FileId::new(index as u32 + 1))
                .unwrap();
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source);
            info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
            registry.register(info);
            modules.push((owner, ast));
        }
        let registry = Shared::new(parking_lot::RwLock::new(registry));
        // Both source bodies must be valid. Timings isolate the consumer's
        // imports and obligations; parsing and provider body checking are not
        // presented as part of that measurement.
        for (owner, module) in &modules {
            check(owner, module, &registry);
        }
        let (owner, consumer) = &modules[1];
        group.bench_function(BenchmarkId::new("bounded_nominal", body_statements), |b| {
            b.iter(|| check(owner, consumer, &registry));
        });
    }
    group.finish();
}

fn bench_source_declaration_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("source_declaration_reads");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));
    for methods in [16, 64, 256] {
        let mut provider = Text::from("public type Contract is protocol {\n");
        for index in 0..methods {
            provider.push_str(&format!("fn operation_{index}(&self) -> Int;\n"));
        }
        provider.push_str("}; public type Carrier<T:Contract> is {public value:T};\n");
        let mut consumer = Text::from("mount bench.provider.{Contract, Carrier};\n");
        for index in 0..32 {
            consumer.push_str(&format!(
                "fn read_{index}<T:Contract>(carrier:Carrier<T>) -> T {{carrier.value}}\n",
            ));
        }
        let mut registry = ModuleRegistry::new();
        let mut modules = List::new();
        for (index, (owner, source)) in [("bench.provider", provider), ("bench.consumer", consumer)]
            .into_iter()
            .enumerate()
        {
            let ast = FastParser::new()
                .parse_module_str(&source, FileId::new(index as u32 + 1))
                .unwrap();
            let id = registry.allocate_id();
            let path = ModulePath::from_str(owner);
            let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, source);
            info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
            registry.register(info);
            modules.push((owner, ast));
        }
        let registry = Shared::new(parking_lot::RwLock::new(registry));
        for (owner, module) in &modules {
            check(owner, module, &registry);
        }
        let (owner, consumer) = &modules[1];
        // Capture one warmed complete consumer check. Counts include successful
        // alloc/alloc_zeroed/realloc requests, not live bytes or retained memory.
        // Collection is off during Criterion's timing samples; the production
        // memory budget remains active during both measurements.
        ALLOCATION_COUNT.store(0, Ordering::Relaxed);
        ALLOCATION_BYTES.store(0, Ordering::Relaxed);
        COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
        check(owner, consumer, &registry);
        COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
        eprintln!(
            "source_declaration_reads methods={methods} allocations={} requested_bytes={}",
            ALLOCATION_COUNT.load(Ordering::Relaxed),
            ALLOCATION_BYTES.load(Ordering::Relaxed)
        );
        group.bench_function(BenchmarkId::new("protocol", methods), |b| {
            b.iter(|| check(owner, consumer, &registry));
        });
    }
    group.finish();
}

/// Measure protocol lookup separately from parsing and registering declarations.
/// The negative query traverses the ordinary candidate matcher used by method dispatch.
fn bench_source_protocol_lookup(c: &mut Criterion) {
    use verum_ast::{Ident, Path, Span};
    use verum_types::Type;

    let mut group = c.benchmark_group("source_protocol_lookup");
    group.sample_size(15);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(2));
    for count in [16, 128] {
        let mut source = Text::from("type Marker is protocol {}; type Missing is {};\n");
        for index in 0..count {
            source.push_str(&format!("type Carrier_{index} is {{ value: Int }}; implement Marker for Carrier_{index} {{}}\n"));
        }
        let ast = FastParser::new().parse_module_str(&source, FileId::new(1)).unwrap();
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_primitives();
        checker.set_current_module_path("bench.proofs");
        let builtin_impls = checker.protocol_checker.read().all_implementations().len();
        assert!(checker.prepare_module_declarations(&ast).is_empty());
        for item in &ast.items { checker.check_item(item).unwrap(); }
        assert!(checker.take_deferred_errors().is_empty());
        assert!(!checker.diagnostics().iter().any(|diagnostic| diagnostic.is_error()));
        let protocols = checker.protocol_checker.read();
        assert_eq!(protocols.all_implementations().len(), builtin_impls + count);
        let receiver = Type::Named {
            path: Path::single(Ident::new("bench.proofs.Missing", Span::default())),
            args: List::new(),
        };
        let query = || protocols.get_implementations(std::hint::black_box(&receiver));
        let carries_marker = |receiver| protocols.get_implementations(receiver).iter()
            .any(|implementation| implementation.protocol.last_segment_name().rsplit('.').next() == Some("Marker"));
        assert!(!carries_marker(&receiver));
        assert!(carries_marker(&Type::Named {
            path: Path::single(Ident::new("bench.proofs.Carrier_0", Span::default())),
            args: List::new(),
        }));
        ALLOCATION_COUNT.store(0, Ordering::Relaxed);
        ALLOCATION_BYTES.store(0, Ordering::Relaxed);
        COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
        let result = query();
        COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
        std::hint::black_box(result);
        eprintln!("source_protocol_lookup declarations={count} allocations={} requested_bytes={}",
            ALLOCATION_COUNT.load(Ordering::Relaxed), ALLOCATION_BYTES.load(Ordering::Relaxed));
        group.bench_function(BenchmarkId::from_parameter(count), |b| b.iter(&query));
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_source_imports,
    bench_source_declaration_reads,
    bench_source_protocol_lookup
);
criterion_main!(benches);
