//! Lint throughput benchmarks.
//!
//! Three benchmark groups, each measuring one cost layer:
//!
//! 1. `single_file` — cost of running every rule on one ~1 KLOC
//!    Verum file. Measures the fixed per-file overhead (parser +
//!    AST walks + text scans).
//!
//! 2. `repo_parallel` — cost of linting a 100-file corpus through
//!    the parallel runner. Measures end-to-end throughput including
//!    config load, file walk, and result merge.
//!
//! 3. `cache_hit` — cost of the same 100-file corpus with the
//!    cache warm. Measures the disk-read + JSON-decode hot path
//!    that incremental CI runs depend on.
//!
//! Run with: `cargo bench -p verum_cli --bench lint_throughput`.
//! Compare runs with `--save-baseline` / `--baseline`.

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use verum_cli::commands::lint::{lint_source, LintConfig};

fn make_kloc_source() -> String {
    // Synthesise a ~1000-line Verum file mixing every text-scan
    // and AST-driven concern the linter handles. Every iteration
    // sees the same content so the bench is deterministic.
    let mut s = String::with_capacity(64 * 1024);
    s.push_str("mount stdlib.collections.list;\n");
    s.push_str("mount stdlib.io;\n\n");
    for i in 0..200 {
        s.push_str(&format!(
            "/// Item {i} — the worker for the {i}'th task.\n\
             public fn item_{i}(x: Int{{ it > 0 }}, y: Int) -> Int {{\n    \
                 let z = x + y;\n    \
                 // TODO(#{i:04}): refine\n    \
                 z\n\
             }}\n\n"
        ));
    }
    s
}

fn fixture_repo(file_count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).expect("create src");
    std::fs::write(
        dir.path().join("verum.toml"),
        "[package]\nname = \"bench\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    for i in 0..file_count {
        let body = format!(
            "/// Module {i}\n\
             public fn item_{i}(x: Int) -> Int {{\n    \
                 let y = x + 1;\n    \
                 y * 2\n\
             }}\n"
        );
        std::fs::write(src.join(format!("file_{i}.vr")), body).expect("file");
    }
    dir
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

fn bench_single_file(c: &mut Criterion) {
    let src = make_kloc_source();
    let path = std::path::Path::new("bench_input.vr");
    let bytes = src.len() as u64;
    let mut group = c.benchmark_group("lint_single_file");
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function(BenchmarkId::new("kloc", "1k"), |b| {
        b.iter(|| {
            let cfg = LintConfig::default();
            let _ = lint_source(path, &src, Some(&cfg));
        });
    });
    group.finish();
}

fn bench_repo_parallel(c: &mut Criterion) {
    let dir = fixture_repo(100);
    let mut group = c.benchmark_group("lint_repo_parallel");
    group.sample_size(20);
    group.bench_function(BenchmarkId::new("files", "100"), |b| {
        b.iter(|| {
            let _ = std::process::Command::new(binary())
                .args(["lint", "--no-cache", "--threads", "8", "--format", "json"])
                .current_dir(dir.path())
                .output();
        });
    });
    group.finish();
}

fn bench_cache_hit(c: &mut Criterion) {
    let dir = fixture_repo(100);
    // Warm the cache.
    let _ = std::process::Command::new(binary())
        .args(["lint", "--threads", "8", "--format", "json"])
        .current_dir(dir.path())
        .output();
    let mut group = c.benchmark_group("lint_cache_hit");
    group.sample_size(30);
    group.bench_function(BenchmarkId::new("files", "100"), |b| {
        b.iter(|| {
            let _ = std::process::Command::new(binary())
                .args(["lint", "--threads", "8", "--format", "json"])
                .current_dir(dir.path())
                .output();
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_single_file,
    bench_repo_parallel,
    bench_cache_hit
);
criterion_main!(benches);
