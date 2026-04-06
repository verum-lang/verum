# VBench - VCS Benchmark Runner

VBench is the performance testing tool for the Verum Compliance Suite (VCS). It measures and validates Verum's performance characteristics against target thresholds and baseline implementations in C, Rust, and Go.

## Performance Targets

| Metric | Target | Description |
|--------|--------|-------------|
| CBGR Check | < 15ns | Per-reference validation overhead |
| Type Inference | < 100ms/10K LOC | Type checking speed |
| Compilation | > 50K LOC/sec | Overall compilation throughput |
| Runtime | 0.85-0.95x C | Runtime performance vs native C |
| Memory | < 5% | Memory overhead vs baseline |

## Installation

```bash
# From the workspace root
cargo build -p vbench --release

# Or install globally
cargo install --path vcs/runner/vbench
```

## Usage

### Run All Benchmarks

```bash
vbench run
```

### Run Specific Categories

```bash
# Micro-benchmarks only
vbench run --category micro

# Macro-benchmarks only
vbench run --category macro

# Filter by name pattern
vbench run --filter cbgr
```

### Run Built-in Benchmarks

```bash
# Run built-in micro-benchmarks (no external files needed)
vbench micro
```

### Compare Against Baselines

```bash
# Compare with C, Rust, and Go implementations
vbench run --compare c,rust,go
```

### Generate Reports

```bash
# Console report (default)
vbench run

# HTML report
vbench run --format html --output report.html

# JSON report
vbench run --format json --output results.json

# Markdown report
vbench run --format markdown --output PERFORMANCE.md
```

### Check for Regressions

```bash
# Compare current vs baseline
vbench check --current results.json --baseline baseline.json

# With custom threshold
vbench check --current results.json --baseline baseline.json --threshold 10.0
```

### List Available Benchmarks

```bash
# List all benchmarks
vbench list

# List with details
vbench list --detailed

# Filter by category
vbench list --category micro
```

### Show Performance Targets

```bash
vbench targets
```

## Configuration

### Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| `--dir` | Benchmark directory | `vcs/benchmarks` |
| `--filter` | Name filter (regex) | None |
| `--category` | Category filter | All |
| `--tier` | Execution tier (0-3, all) | 3 |
| `--warmup` | Warmup iterations | 100 |
| `--iterations` | Measurement iterations | 1000 |
| `--parallel` | Parallel workers | 1 |
| `--format` | Output format | console |
| `--output` | Output file | stdout |
| `--strict` | Fail on threshold violation | false |

### Configuration File (vbench.toml)

```toml
[runner]
benchmark_dir = "vcs/benchmarks"
warmup_iterations = 100
measure_iterations = 1000
parallel = 8
timeout_ms = 60000

[tiers]
tier0_cmd = "verum-interpreter"
tier1_cmd = "verum-jit --baseline"
tier2_cmd = "verum-jit --optimize"
tier3_cmd = "verum-aot"

[targets]
cbgr_check_ns = 15.0
type_inference_ms_per_10k_loc = 100.0
compilation_loc_per_sec = 50000.0
runtime_vs_c_min = 0.85
runtime_vs_c_max = 0.95
memory_overhead_percent = 5.0

[targets.custom]
allocation_small = 50.0
context_lookup = 30.0
async_spawn = 500.0
```

## Benchmark Categories

### Micro-benchmarks (`vcs/benchmarks/micro/`)

Individual operations with nanosecond-level timing:

- `cbgr_check.vr` - CBGR validation overhead
- `allocation.vr` - Memory allocation patterns
- `context_lookup.vr` - Context system performance
- `async_spawn.vr` - Async task creation
- `channel_send.vr` / `channel_recv.vr` - Channel operations
- `mutex_lock.vr` / `rwlock_*.vr` - Synchronization primitives
- `parse_*.vr` - Parser benchmarks
- `type_inference.vr` - Type system benchmarks
- `codegen_*.vr` - Code generation benchmarks
- `tensor_*.vr` - Tensor/SIMD operations

### Macro-benchmarks (`vcs/benchmarks/macro/`)

Realistic workloads with millisecond-level timing:

- `http_server.vr` - HTTP request handling
- `json_parsing.vr` - JSON parser performance
- `db_query.vr` - Database operations
- `matrix_multiply.vr` - Numerical computation
- `text_processing.vr` - Text manipulation
- `file_io.vr` - File I/O performance
- `concurrent_workers.vr` - Parallelism patterns
- `compression.vr` - Compression algorithms
- `crypto_hash.vr` - Cryptographic hashing

### Baselines (`vcs/benchmarks/baselines/`)

Reference implementations in other languages:

- `vs_c/` - C implementations (with `-O3`)
- `vs_rust/` - Rust implementations (release mode)
- `vs_go/` - Go implementations

## Report Formats

### Console Output

```
===========================================================================
VBench Performance Report
===========================================================================

Verum Version: 1.0.0
Platform: darwin arm64
Timestamp: 2025-12-31 12:00:00 UTC

---------------------------------------------------------------------------
SUMMARY
---------------------------------------------------------------------------
Total: 25  Passed: 24  Failed: 1  Regressions: 0
Pass Rate: 96.0%
Total Duration: 15.32ms

---------------------------------------------------------------------------
MICRO (15)
---------------------------------------------------------------------------
  PASS cbgr_check/tier0                    12.50ns      +/- 0.50ns
  PASS allocation/small                    45.00ns      +/- 2.00ns
  FAIL context_lookup                      35.00ns      +/- 1.50ns
       threshold: 30.00ns (exceeded)
  ...

===========================================================================
24 PASSED, 1 FAILED
===========================================================================
```

### JSON Output

Full structured data for CI/CD integration and historical analysis.

### HTML Report

Interactive report with charts and detailed statistics.

## API Usage

```rust
use vbench::{
    quick_bench, bench_group, run_all_micro_benchmarks,
    BenchmarkReport, ReportMetadata, generate_report, ReportFormat,
};

// Quick benchmark
let result = quick_bench("my_op", 1000, || {
    std::hint::black_box(1 + 1);
});
println!("Mean: {:.2}ns", result.statistics.mean_ns);

// Benchmark group
let results = bench_group("arithmetic")
    .warmup(100)
    .iterations(10000)
    .bench("add", || std::hint::black_box(1 + 1))
    .bench("mul", || std::hint::black_box(2 * 3))
    .bench_with_threshold("div", 50.0, || std::hint::black_box(10 / 2))
    .run();

// Generate report
let metadata = ReportMetadata::new("My Benchmarks", "1.0.0");
let report = BenchmarkReport::new(metadata, results, vec![], vec![]);
let html = generate_report(&report, ReportFormat::Html)?;
```

## CI/CD Integration

### GitHub Actions

```yaml
- name: Run benchmarks
  run: |
    cargo install --path vcs/runner/vbench
    vbench run --format json --output results.json

- name: Check for regressions
  run: |
    vbench check \
      --current results.json \
      --baseline benchmarks/baseline.json \
      --threshold 5.0

- name: Upload results
  uses: actions/upload-artifact@v3
  with:
    name: benchmark-results
    path: results.json
```

## Architecture

```
vcs/runner/vbench/
├── Cargo.toml
├── README.md
└── src/
    ├── lib.rs          # Core library and re-exports
    ├── main.rs         # CLI entry point
    ├── metrics.rs      # Measurement and statistics
    ├── runner.rs       # Benchmark execution
    ├── compare.rs      # Baseline comparison
    └── report.rs       # Report generation
```

## License

Apache-2.0
