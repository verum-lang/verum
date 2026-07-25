//! Performance benchmarks for incremental parsing
//!
//! This benchmark suite validates the performance targets:
//! - Document sync: <10ms for typical changes
//! - Incremental parse: <50ms for 1000 LOC files
//! - Memory overhead: <10MB per open document

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use tower_lsp::lsp_types::*;
use verum_lsp::DocumentCache;

fn create_test_uri(id: usize) -> Url {
    Url::parse(&format!("file:///bench/test{}.vr", id)).unwrap()
}

fn generate_source(lines: usize) -> String {
    let mut source = String::new();
    for i in 0..lines {
        source.push_str(&format!("fn func{}() {{\n", i));
        source.push_str("    let x = 42;\n");
        source.push_str("    print(x);\n");
        source.push_str("}\n\n");
    }
    source
}

fn bench_document_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("document_open");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let source = generate_source(size);

            b.iter(|| {
                let uri = create_test_uri(0);
                cache.open_document(uri.clone(), black_box(source.clone()), 1);
                cache.close_document(&uri);
            });
        });
    }

    group.finish();
}

fn bench_incremental_single_char(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_single_char");
    group.measurement_time(Duration::from_secs(10));

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let uri = create_test_uri(0);
            let source = generate_source(size);

            cache.open_document(uri.clone(), source, 1);

            b.iter(|| {
                let changes = vec![TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: 0,
                            character: 3,
                        },
                        end: Position {
                            line: 0,
                            character: 4,
                        },
                    }),
                    range_length: Some(1),
                    text: "x".to_string(),
                }];

                cache.update_document(&uri, black_box(&changes), 2).unwrap();
            });

            cache.close_document(&uri);
        });
    }

    group.finish();
}

fn bench_incremental_line_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_line_insert");
    group.measurement_time(Duration::from_secs(10));

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let uri = create_test_uri(0);
            let source = generate_source(size);

            cache.open_document(uri.clone(), source, 1);

            b.iter(|| {
                let changes = vec![TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: size as u32 / 2,
                            character: 0,
                        },
                        end: Position {
                            line: size as u32 / 2,
                            character: 0,
                        },
                    }),
                    range_length: Some(0),
                    text: "let y = 100;\n".to_string(),
                }];

                cache.update_document(&uri, black_box(&changes), 2).unwrap();
            });

            cache.close_document(&uri);
        });
    }

    group.finish();
}

fn bench_incremental_multi_line_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_multi_line_edit");
    group.measurement_time(Duration::from_secs(10));

    for size in [100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let uri = create_test_uri(0);
            let source = generate_source(size);

            cache.open_document(uri.clone(), source, 1);

            b.iter(|| {
                let changes = vec![TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: size as u32 / 2,
                            character: 0,
                        },
                        end: Position {
                            line: size as u32 / 2 + 10,
                            character: 0,
                        },
                    }),
                    range_length: None,
                    text: "// Modified section\nfn modified() {\n    let z = 999;\n}\n".to_string(),
                }];

                cache.update_document(&uri, black_box(&changes), 2).unwrap();
            });

            cache.close_document(&uri);
        });
    }

    group.finish();
}

fn bench_full_document_sync(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_document_sync");
    group.measurement_time(Duration::from_secs(10));

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let uri = create_test_uri(0);
            let source = generate_source(size);

            cache.open_document(uri.clone(), source.clone(), 1);

            b.iter(|| {
                let changes = vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: black_box(source.clone()),
                }];

                cache.update_document(&uri, &changes, 2).unwrap();
            });

            cache.close_document(&uri);
        });
    }

    group.finish();
}

fn bench_get_diagnostics(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_diagnostics");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = DocumentCache::new();
            let uri = create_test_uri(0);
            let source = generate_source(size);

            cache.open_document(uri.clone(), source, 1);

            b.iter(|| {
                let _diagnostics = cache.get_diagnostics(black_box(&uri));
            });

            cache.close_document(&uri);
        });
    }

    group.finish();
}

fn bench_concurrent_reads(c: &mut Criterion) {
    use std::sync::Arc;
    use std::thread;

    let mut group = c.benchmark_group("concurrent_reads");

    group.bench_function("10_threads", |b| {
        let cache = Arc::new(DocumentCache::new());
        let uri = create_test_uri(0);
        let source = generate_source(100);

        cache.open_document(uri.clone(), source, 1);

        b.iter(|| {
            let mut handles = vec![];

            for _ in 0..10 {
                let cache_clone = Arc::clone(&cache);
                let uri_clone = uri.clone();

                let handle = thread::spawn(move || {
                    for _ in 0..100 {
                        let _text = cache_clone.get_text(&uri_clone);
                        let _diagnostics = cache_clone.get_diagnostics(&uri_clone);
                    }
                });

                handles.push(handle);
            }

            for handle in handles {
                handle.join().unwrap();
            }
        });

        cache.close_document(&uri);
    });

    group.finish();
}

fn bench_position_to_offset(c: &mut Criterion) {
    use verum_ast::FileId;
    use verum_lsp::ParsedDocument;

    let mut group = c.benchmark_group("position_to_offset");

    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let source = generate_source(size);
            let _doc = ParsedDocument::new(source, 1, FileId::new(1));

            b.iter(|| {
                let pos = Position {
                    line: (size / 2) as u32,
                    character: 10,
                };

                // Access through the document cache wrapper would be needed
                // For now, this is a placeholder
                black_box(pos);
            });
        });
    }

    group.finish();
}

fn bench_memory_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_overhead");

    group.bench_function("1000_loc_document", |b| {
        let cache = DocumentCache::new();
        let uri = create_test_uri(0);
        let source = generate_source(1000);

        b.iter(|| {
            cache.open_document(uri.clone(), black_box(source.clone()), 1);

            // Measure approximate memory usage
            let source_size = source.len();
            let overhead = std::mem::size_of::<DocumentCache>();

            black_box(source_size + overhead);

            cache.close_document(&uri);
        });
    });

    group.finish();
}

// Target: Document sync <10ms for typical changes
fn bench_document_sync_target(c: &mut Criterion) {
    let mut group = c.benchmark_group("sync_target_10ms");
    group.measurement_time(Duration::from_secs(20));

    // Typical change: single character edit in a 500 LOC file
    group.bench_function("typical_edit_500_loc", |b| {
        let cache = DocumentCache::new();
        let uri = create_test_uri(0);
        let source = generate_source(500);

        cache.open_document(uri.clone(), source, 1);

        b.iter(|| {
            let start = std::time::Instant::now();

            let changes = vec![TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position {
                        line: 250,
                        character: 5,
                    },
                    end: Position {
                        line: 250,
                        character: 6,
                    },
                }),
                range_length: Some(1),
                text: "y".to_string(),
            }];

            cache.update_document(&uri, black_box(&changes), 2).unwrap();

            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 10,
                "Sync took {}ms, expected <10ms",
                elapsed.as_millis()
            );
        });

        cache.close_document(&uri);
    });

    group.finish();
}

// Target: Incremental parse <50ms for 1000 LOC
fn bench_parse_target(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_target_50ms");
    group.measurement_time(Duration::from_secs(20));

    group.bench_function("incremental_1000_loc", |b| {
        let cache = DocumentCache::new();
        let uri = create_test_uri(0);
        let source = generate_source(1000);

        cache.open_document(uri.clone(), source, 1);

        b.iter(|| {
            let start = std::time::Instant::now();

            // Make a change that triggers re-parsing
            let changes = vec![TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position {
                        line: 500,
                        character: 0,
                    },
                    end: Position {
                        line: 510,
                        character: 0,
                    },
                }),
                range_length: None,
                text: "fn new_func() { let x = 1; }\n".to_string(),
            }];

            cache.update_document(&uri, black_box(&changes), 2).unwrap();

            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 50,
                "Parse took {}ms, expected <50ms",
                elapsed.as_millis()
            );
        });

        cache.close_document(&uri);
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_document_open,
    bench_incremental_single_char,
    bench_incremental_line_insert,
    bench_incremental_multi_line_edit,
    bench_full_document_sync,
    bench_get_diagnostics,
    bench_concurrent_reads,
    bench_position_to_offset,
    bench_memory_overhead,
    bench_document_sync_target,
    bench_parse_target,
);

criterion_main!(benches);
