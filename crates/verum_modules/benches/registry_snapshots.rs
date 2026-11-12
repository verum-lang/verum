//! Snapshot cost must depend on a mutation, not on each independent import read.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};

#[global_allocator]
static ALLOC: verum_common::memory_budget::BudgetedAllocator =
    verum_common::memory_budget::BudgetedAllocator::new();

fn module(id: u32, owner: &str) -> ModuleInfo {
    let file = FileId::new(id);
    let id = ModuleId::new(id);
    let source = "public fn answer() -> Int { 42 }";
    let ast = VerumParser::new().parse_module_str(source, file).unwrap();
    let path = ModulePath::from_str(owner);
    let exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut info = ModuleInfo::new(id, path, ast, file, source.into());
    info.exports = exports;
    info
}

fn bench_snapshots(c: &mut Criterion) {
    let mut group = c.benchmark_group("registry_snapshots");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(1));
    for modules in [32, 512, 8192] {
        let mut registry = ModuleRegistry::new();
        for index in 0..modules {
            registry.register(module(index, &format!("project.unit{index}")));
        }
        let addition = module(modules, "project.added");
        group.bench_function(BenchmarkId::new("clone", modules), |b| {
            b.iter(|| black_box(black_box(&registry).clone()));
        });
        // A real write still requires isolation. Include that cost rather than
        // presenting snapshot sharing as a constant-time mutable registry.
        group.bench_function(BenchmarkId::new("clone_and_insert", modules), |b| {
            b.iter(|| {
                let mut snapshot = registry.clone();
                snapshot.register(addition.clone());
                black_box(snapshot)
            });
        });
        assert!(registry.get_by_path("project.added").is_none());
        assert_eq!(registry.len(), modules as usize);
    }
    group.finish();
}

criterion_group!(benches, bench_snapshots);
criterion_main!(benches);
