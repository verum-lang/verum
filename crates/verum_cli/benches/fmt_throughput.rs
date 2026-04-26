//! `verum fmt` throughput benchmarks.
//!
//! Two cost layers:
//!
//! 1. `fmt_single_file` — cost of running `format_string` against
//!    one ~1 KLOC fixture. Measures the fixed per-file overhead
//!    (parser + pretty-printer + post-processing).
//!
//! 2. `fmt_repo_parallel` — end-to-end cost of `verum fmt --check`
//!    against a 100-file corpus through the parallel runner.
//!
//! Run with: `cargo bench -p verum_cli --bench fmt_throughput`.
//! Compare runs with `--save-baseline` / `--baseline`.

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use verum_cli::commands::fmt::format_string;

fn make_kloc_source() -> String {
    // Synthesise a ~1000-line Verum file. The fixture exercises the
    // pretty printer's heaviest paths: function decls, attributes,
    // refinement types, match expressions.
    let mut s = String::with_capacity(64 * 1024);
    s.push_str("mount stdlib.collections.list;\n");
    s.push_str("mount stdlib.io;\n\n");
    for i in 0..150 {
        s.push_str(&format!(
            "/// item {i}\n\
             @verify(formal)\n\
             public fn item_{i}(x: Int{{ it > 0 }}, y: Int) -> Int {{\n    \
                 let z = x + y;\n    \
                 match z {{\n        \
                     0 => 0,\n        \
                     n if n > {i} => n,\n        \
                     _ => z,\n    \
                 }}\n\
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
            "/// Module {i}\npublic fn item_{i}(x: Int) -> Int {{\n    let y = x + 1;\n    y * 2\n}}\n"
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
    let bytes = src.len() as u64;
    let mut group = c.benchmark_group("fmt_single_file");
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function(BenchmarkId::new("kloc", "1k"), |b| {
        b.iter(|| {
            let _ = format_string(&src);
        });
    });
    group.finish();
}

fn bench_repo_parallel(c: &mut Criterion) {
    let dir = fixture_repo(100);
    let mut group = c.benchmark_group("fmt_repo_parallel");
    group.sample_size(20);
    group.bench_function(BenchmarkId::new("files", "100"), |b| {
        b.iter(|| {
            let _ = std::process::Command::new(binary())
                .args(["fmt", "--check", "--threads", "8"])
                .current_dir(dir.path())
                .output();
        });
    });
    group.finish();
}

criterion_group!(benches, bench_single_file, bench_repo_parallel);
criterion_main!(benches);
