//! Performance benchmarks for multi-segment path resolution.
//!
//! Measures the performance impact of:
//! - Single-segment vs multi-segment paths
//! - Cache hits vs cache misses
//! - Deep nesting (5+ segments)
//! - Concurrent resolution
//!
//! Target: < 15ns per cached resolution, < 1μs per uncached resolution
//!
//! Multi-segment path resolution: resolves first segment through the standard
//! resolution algorithm, then traverses subsequent segments through module exports.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use verum_ast::{FileId, Ident, Path, PathSegment, Span};
use verum_common::{List, Map, Text};
use verum_modules::{
    path::ModuleId,
    resolver::{NameKind, NameResolver, ResolvedName},
};

fn dummy_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

fn make_path(segments: &[&str]) -> Path {
    let seg_list: List<PathSegment> = segments
        .iter()
        .map(|&s| PathSegment::Name(Ident::new(s, dummy_span())))
        .collect();
    Path::new(seg_list, dummy_span())
}

fn make_exports(items: &[(&str, NameKind)]) -> Map<Text, ResolvedName> {
    let mut map = Map::new();
    for &(name, kind) in items {
        map.insert(
            Text::from(name),
            ResolvedName::new(
                ModuleId::new(1),
                verum_modules::path::ModulePath::from_str(name),
                kind,
                name,
            ),
        );
    }
    map
}

fn setup_resolver_single_segment() -> (NameResolver, ModuleId) {
    let mut resolver = NameResolver::new();
    let module = ModuleId::new(1);

    let scope = resolver.create_scope(module);
    scope.add_binding(
        "List",
        ResolvedName::new(
            module,
            verum_modules::path::ModulePath::from_str("List"),
            NameKind::Type,
            "List",
        ),
    );

    (resolver, module)
}

fn setup_resolver_two_segments() -> (NameResolver, ModuleId) {
    let mut resolver = NameResolver::new();
    let module = ModuleId::new(1);
    let collections = ModuleId::new(2);

    let scope = resolver.create_scope(module);
    scope.add_binding(
        "collections",
        ResolvedName::new(
            collections,
            verum_modules::path::ModulePath::from_str("collections"),
            NameKind::Module,
            "collections",
        ),
    );

    let exports = make_exports(&[("List", NameKind::Type)]);
    resolver.register_module_exports(collections, exports);

    (resolver, module)
}

fn setup_resolver_deep_nesting() -> (NameResolver, ModuleId) {
    let mut resolver = NameResolver::new();
    let module = ModuleId::new(1);

    // Create chain: a -> b -> c -> d -> e -> Type
    let modules: Vec<ModuleId> = (2..=6).map(ModuleId::new).collect();

    // root -> a
    let scope = resolver.create_scope(module);
    scope.add_binding(
        "a",
        ResolvedName::new(
            modules[0],
            verum_modules::path::ModulePath::from_str("a"),
            NameKind::Module,
            "a",
        ),
    );

    // Chain modules
    for i in 0..4 {
        let mut exports = Map::new();
        let next_name = match i {
            0 => "b",
            1 => "c",
            2 => "d",
            3 => "e",
            _ => unreachable!(),
        };
        exports.insert(
            Text::from(next_name),
            ResolvedName::new(
                modules[i + 1],
                verum_modules::path::ModulePath::from_str(next_name),
                NameKind::Module,
                next_name,
            ),
        );
        resolver.register_module_exports(modules[i], exports);
    }

    // Final module exports Type
    let final_exports = make_exports(&[("Type", NameKind::Type)]);
    resolver.register_module_exports(modules[4], final_exports);

    (resolver, module)
}

fn bench_single_segment(c: &mut Criterion) {
    let (resolver, module) = setup_resolver_single_segment();
    let path = make_path(&["List"]);

    c.bench_function("single_segment_uncached", |b| {
        b.iter(|| {
            resolver.clear_path_cache();
            resolver.resolve_path(black_box(&path), black_box(module))
        })
    });

    c.bench_function("single_segment_cached", |b| {
        // Prime the cache
        let _ = resolver.resolve_path(&path, module);

        b.iter(|| resolver.resolve_path(black_box(&path), black_box(module)))
    });
}

fn bench_two_segments(c: &mut Criterion) {
    let (resolver, module) = setup_resolver_two_segments();
    let path = make_path(&["collections", "List"]);

    c.bench_function("two_segments_uncached", |b| {
        b.iter(|| {
            resolver.clear_path_cache();
            resolver.resolve_path(black_box(&path), black_box(module))
        })
    });

    c.bench_function("two_segments_cached", |b| {
        // Prime the cache
        let _ = resolver.resolve_path(&path, module);

        b.iter(|| resolver.resolve_path(black_box(&path), black_box(module)))
    });
}

fn bench_deep_nesting(c: &mut Criterion) {
    let (resolver, module) = setup_resolver_deep_nesting();
    let path = make_path(&["a", "b", "c", "d", "e", "Type"]);

    c.bench_function("deep_nesting_uncached", |b| {
        b.iter(|| {
            resolver.clear_path_cache();
            resolver.resolve_path(black_box(&path), black_box(module))
        })
    });

    c.bench_function("deep_nesting_cached", |b| {
        // Prime the cache
        let _ = resolver.resolve_path(&path, module);

        b.iter(|| resolver.resolve_path(black_box(&path), black_box(module)))
    });
}

fn bench_cache_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_performance");

    for num_paths in [10, 100, 1000].iter() {
        let (resolver, module) = setup_resolver_two_segments();
        let paths: Vec<Path> = (0..*num_paths)
            .map(|_| make_path(&["collections", "List"]))
            .collect();

        group.bench_with_input(
            BenchmarkId::new("cache_hit_rate", num_paths),
            num_paths,
            |b, _| {
                // Prime cache with first path
                let _ = resolver.resolve_path(&paths[0], module);

                b.iter(|| {
                    for path in &paths {
                        let _ = resolver.resolve_path(black_box(path), black_box(module));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_concurrent_resolution(c: &mut Criterion) {
    use std::sync::Arc;
    use std::thread;

    let (resolver, module) = setup_resolver_two_segments();
    let resolver = Arc::new(resolver);
    let path = make_path(&["collections", "List"]);

    c.bench_function("concurrent_10_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let resolver = Arc::clone(&resolver);
                    let path = path.clone();
                    thread::spawn(move || {
                        for _ in 0..10 {
                            let _ = resolver.resolve_path(&path, module);
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
}

criterion_group!(
    benches,
    bench_single_segment,
    bench_two_segments,
    bench_deep_nesting,
    bench_cache_performance,
    bench_concurrent_resolution
);

criterion_main!(benches);
