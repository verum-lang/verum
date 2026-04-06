//! Performance benchmarks for verum_common semantic types
//!
//! Validates performance targets for Text, List, Map, Set operations

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_common::semantic_types::{List, Map, OrderedMap, OrderedSet, Set, Text};

// ============================================================================
// TEXT BENCHMARKS
// ============================================================================

fn bench_text_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_push");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut text = Text::with_capacity(size);
                for _ in 0..size {
                    text.push(black_box('a'));
                }
                black_box(text);
            });
        });
    }
    group.finish();
}

fn bench_text_push_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_push_str");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let suffix = "a".repeat(size);
            b.iter(|| {
                let mut text = Text::from("prefix");
                text.push_str(black_box(&suffix));
                black_box(text);
            });
        });
    }
    group.finish();
}

fn bench_text_split(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_split");

    for size in [10, 100, 1000].iter() {
        let text = vec!["word"; *size].join(",");
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &text, |b, text| {
            let text = Text::from(text.as_str());
            b.iter(|| {
                black_box(text.split(","));
            });
        });
    }
    group.finish();
}

fn bench_text_to_lowercase(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_to_lowercase");

    for size in [10, 100, 1000, 10_000].iter() {
        let text = Text::from("A".repeat(*size));
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &text, |b, text| {
            b.iter(|| {
                black_box(text.to_lowercase());
            });
        });
    }
    group.finish();
}

fn bench_text_replace(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_replace");

    for size in [10, 100, 1000].iter() {
        let text = Text::from("hello world ".repeat(*size));
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &text, |b, text| {
            b.iter(|| {
                black_box(text.replace("world", "rust"));
            });
        });
    }
    group.finish();
}

fn bench_text_contains(c: &mut Criterion) {
    let text = Text::from("hello world ".repeat(1000));
    c.bench_function("text_contains", |b| {
        b.iter(|| {
            black_box(text.contains(black_box("world")));
        });
    });
}

// ============================================================================
// LIST BENCHMARKS
// ============================================================================

fn bench_list_push(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_push");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut list = List::with_capacity(size);
                for i in 0..size {
                    list.push(black_box(i));
                }
                black_box(list);
            });
        });
    }
    group.finish();
}

fn bench_list_pop(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_pop");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_batched(
                || {
                    let mut list = List::with_capacity(size);
                    for i in 0..size {
                        list.push(i);
                    }
                    list
                },
                |mut list| {
                    while !list.is_empty() {
                        black_box(list.pop());
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_list_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_sort");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_batched(
                || {
                    let mut list = List::with_capacity(size);
                    for i in (0..size).rev() {
                        list.push(i);
                    }
                    list
                },
                |mut list| {
                    list.sort();
                    black_box(list);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_list_reverse(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_reverse");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut list = List::with_capacity(*size);
        for i in 0..*size {
            list.push(i);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &list, |b, list| {
            b.iter_batched(
                || list.clone(),
                |mut list| {
                    list.reverse();
                    black_box(list);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_list_contains(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_contains");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut list = List::with_capacity(*size);
        for i in 0..*size {
            list.push(i);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &list, |b, list| {
            b.iter(|| {
                black_box(list.contains(black_box(&(size / 2))));
            });
        });
    }
    group.finish();
}

fn bench_list_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_filter");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut list = List::with_capacity(*size);
        for i in 0..*size {
            list.push(i);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &list, |b, list| {
            b.iter(|| {
                black_box(list.clone().filter(|&x| x % 2 == 0));
            });
        });
    }
    group.finish();
}

fn bench_list_map(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_map");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut list = List::with_capacity(*size);
        for i in 0..*size {
            list.push(i);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &list, |b, list| {
            b.iter(|| {
                black_box(list.clone().map(|x| x * 2));
            });
        });
    }
    group.finish();
}

// ============================================================================
// MAP BENCHMARKS
// ============================================================================

fn bench_map_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_insert");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut map = Map::with_capacity(size);
                for i in 0..size {
                    map.insert(black_box(i), black_box(i * 10));
                }
                black_box(map);
            });
        });
    }
    group.finish();
}

fn bench_map_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_get");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut map = Map::with_capacity(*size);
        for i in 0..*size {
            map.insert(i, i * 10);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &map, |b, map| {
            b.iter(|| {
                for i in 0..*size {
                    black_box(map.get(black_box(&i)));
                }
            });
        });
    }
    group.finish();
}

fn bench_map_remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_remove");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_batched(
                || {
                    let mut map = Map::with_capacity(size);
                    for i in 0..size {
                        map.insert(i, i * 10);
                    }
                    map
                },
                |mut map| {
                    for i in 0..size {
                        black_box(map.remove(&i));
                    }
                    black_box(map);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_map_contains_key(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_contains_key");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut map = Map::with_capacity(*size);
        for i in 0..*size {
            map.insert(i, i * 10);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &map, |b, map| {
            b.iter(|| {
                for i in 0..*size {
                    black_box(map.contains_key(black_box(&i)));
                }
            });
        });
    }
    group.finish();
}

fn bench_map_iteration(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_iteration");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut map = Map::with_capacity(*size);
        for i in 0..*size {
            map.insert(i, i * 10);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &map, |b, map| {
            b.iter(|| {
                let mut sum = 0;
                for (k, v) in map.iter() {
                    sum += k + v;
                }
                black_box(sum);
            });
        });
    }
    group.finish();
}

// ============================================================================
// SET BENCHMARKS
// ============================================================================

fn bench_set_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_insert");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut set = Set::new();
                for i in 0..size {
                    set.insert(black_box(i));
                }
                black_box(set);
            });
        });
    }
    group.finish();
}

fn bench_set_contains(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_contains");

    for size in [10, 100, 1000, 10_000].iter() {
        let mut set = Set::new();
        for i in 0..*size {
            set.insert(i);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &set, |b, set| {
            b.iter(|| {
                for i in 0..*size {
                    black_box(set.contains(black_box(&i)));
                }
            });
        });
    }
    group.finish();
}

fn bench_set_remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_remove");

    for size in [10, 100, 1000, 10_000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter_batched(
                || {
                    let mut set = Set::new();
                    for i in 0..size {
                        set.insert(i);
                    }
                    set
                },
                |mut set| {
                    for i in 0..size {
                        black_box(set.remove(&i));
                    }
                    black_box(set);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_set_union(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_union");

    for size in [10, 100, 1000].iter() {
        let mut set1 = Set::new();
        let mut set2 = Set::new();
        for i in 0..*size {
            set1.insert(i);
            set2.insert(i + size / 2);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(set1, set2),
            |b, (set1, set2)| {
                b.iter(|| {
                    let union: Vec<_> = set1.union(set2).collect();
                    black_box(union);
                });
            },
        );
    }
    group.finish();
}

fn bench_set_intersection(c: &mut Criterion) {
    let mut group = c.benchmark_group("set_intersection");

    for size in [10, 100, 1000].iter() {
        let mut set1 = Set::new();
        let mut set2 = Set::new();
        for i in 0..*size {
            set1.insert(i);
            set2.insert(i + size / 2);
        }

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &(set1, set2),
            |b, (set1, set2)| {
                b.iter(|| {
                    let intersection: Vec<_> = set1.intersection(set2).collect();
                    black_box(intersection);
                });
            },
        );
    }
    group.finish();
}

// ============================================================================
// ORDERED COLLECTIONS BENCHMARKS
// ============================================================================

fn bench_ordered_map_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("ordered_map_insert");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut map = OrderedMap::new();
                for i in 0..size {
                    map.insert(black_box(i), black_box(i * 10));
                }
                black_box(map);
            });
        });
    }
    group.finish();
}

fn bench_ordered_set_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("ordered_set_insert");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut set = OrderedSet::new();
                for i in 0..size {
                    set.insert(black_box(i));
                }
                black_box(set);
            });
        });
    }
    group.finish();
}

// ============================================================================
// CRITERION GROUPS
// ============================================================================

criterion_group!(
    text_benches,
    bench_text_push,
    bench_text_push_str,
    bench_text_split,
    bench_text_to_lowercase,
    bench_text_replace,
    bench_text_contains
);

criterion_group!(
    list_benches,
    bench_list_push,
    bench_list_pop,
    bench_list_sort,
    bench_list_reverse,
    bench_list_contains,
    bench_list_filter,
    bench_list_map
);

criterion_group!(
    map_benches,
    bench_map_insert,
    bench_map_get,
    bench_map_remove,
    bench_map_contains_key,
    bench_map_iteration
);

criterion_group!(
    set_benches,
    bench_set_insert,
    bench_set_contains,
    bench_set_remove,
    bench_set_union,
    bench_set_intersection
);

criterion_group!(
    ordered_benches,
    bench_ordered_map_insert,
    bench_ordered_set_insert
);

criterion_main!(
    text_benches,
    list_benches,
    map_benches,
    set_benches,
    ordered_benches
);
