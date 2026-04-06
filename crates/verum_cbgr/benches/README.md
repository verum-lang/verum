# CBGR Performance Benchmarks

This directory contains comprehensive performance benchmarks for the CBGR (Counter-Based Garbage Rejection) memory management system.

## Critical Performance Requirements

From `CLAUDE.md`, CBGR must meet these targets:

- **CBGR overhead**: < 15ns per check (measured)
- **Type inference**: < 100ms for 10K LOC
- **Compilation speed**: > 50K LOC/sec (release)
- **Runtime performance**: 0.85-0.95x native C
- **Memory overhead**: < 5% vs unsafe code

## Available Benchmarks

### 1. `cbgr_overhead_bench.rs` - **PRIMARY BENCHMARK**

Comprehensive CBGR overhead measurement suite that validates all performance targets.

**Run with:**
```bash
cargo bench --package verum_cbgr --bench cbgr_overhead_bench --release
```

**What it measures:**

1. **Three-Tier Reference Comparison**
   - Tier 0 (Managed `&T`): ~15ns CBGR overhead
   - Tier 1 (Checked `&checked T`): 0ns (compile-time verified)
   - Tier 2 (Unsafe `&unsafe T`): 0ns (raw pointer)
   - Native Rust references
   - Raw pointers

2. **CBGR Overhead: 15ns Target Validation** ⚠️ **CRITICAL**
   - Baseline native pointer operations
   - CBGR managed reference operations
   - Fast path optimizations
   - Validation-only overhead
   - Generation check overhead

3. **CBGR-Specific Operations**
   - Epoch advancement overhead
   - Hazard pointer protection overhead
   - Reference validation times
   - Generation/epoch tracking costs

4. **Realistic Workloads**
   - Concurrent access patterns (1, 2, 4 threads)
   - Different object sizes (4 bytes, 100 bytes, 10KB)
   - Various lifetime patterns
   - Mixed allocation/deallocation patterns

5. **Memory Overhead Measurements**
   - GenRef size vs raw pointers
   - Allocation header overhead
   - Total memory cost for 1000 objects

6. **Escape Analysis Impact**
   - Non-escaping (promotable to Tier 1)
   - Escaping (must stay Tier 0)
   - Promotion savings calculation

7. **Allocation Patterns**
   - Simple alloc/dealloc cycles
   - Batch allocation
   - Interleaved patterns
   - Clone overhead

8. **Generation Map Operations** (for FFI)
   - Lookup performance
   - Insert performance
   - Remove performance

9. **Performance Regression Gates** ⚠️ **CI CRITICAL**
   - CBGR validation < 20ns (relaxed for CI)
   - Clone < 5ns
   - Generation read < 3ns
   - Alloc/dealloc < 100ns

### 2. `hazard_overhead_bench.rs`

Focused benchmarks for hazard pointer system overhead.

**Run with:**
```bash
cargo bench --package verum_cbgr --bench hazard_overhead_bench --release
```

**Measures:**
- Dereference with/without hazard protection
- Allocation/deallocation with retirement
- Hazard acquisition/release
- Nested dereferences
- Concurrent access

### 3. `comprehensive_bench.rs`

End-to-end comprehensive performance validation.

**Run with:**
```bash
cargo bench --package verum_cbgr --bench comprehensive_bench --release
```

### 4. `generation_map_bench.rs`

Generation map (FFI) performance benchmarks.

**Run with:**
```bash
cargo bench --package verum_cbgr --bench generation_map_bench --release
```

### 5. `optimization_pass_bench.rs`

LLVM optimization pass benchmarks.

**Run with:**
```bash
cargo bench --package verum_cbgr --bench optimization_pass_bench --release
```

## Running All Benchmarks

```bash
# Run all CBGR benchmarks
cargo bench --package verum_cbgr --release

# Run specific benchmark
cargo bench --package verum_cbgr --bench cbgr_overhead_bench --release

# Run and save baseline
cargo bench --package verum_cbgr --bench cbgr_overhead_bench --release -- --save-baseline main

# Compare against baseline
cargo bench --package verum_cbgr --bench cbgr_overhead_bench --release -- --baseline main
```

## Understanding Results

### Expected Results (Release Build)

The benchmarks will print detailed analysis after each group:

```
╔════════════════════════════════════════════════════════════╗
║     CRITICAL REQUIREMENT: CBGR overhead < 15ns             ║
╠════════════════════════════════════════════════════════════╣
║ baseline_native_0ns:           ~0.5ns                     ║
║ cbgr_managed_target_15ns:      ~15ns (MUST BE < 15ns)     ║
║ cbgr_fast_path_optimized:      ~10ns (optimized)          ║
║ cbgr_validate_only:            ~10ns (no deref)           ║
║ cbgr_generation_check_only:    ~2ns (just read)           ║
╚════════════════════════════════════════════════════════════╝
```

### Key Metrics to Watch

1. **CBGR Overhead** (CRITICAL): Must be < 15ns
   - Compare `cbgr_managed_target_15ns` vs `baseline_native_0ns`
   - Difference should be < 15ns

2. **Three-Tier Performance**:
   - Tier 2 & Tier 1: ~0.5ns (same as native)
   - Tier 0: ~15ns (CBGR overhead)

3. **Concurrent Scaling**:
   - 2 threads: ~1.8x speedup
   - 4 threads: ~3.2x speedup

4. **Memory Overhead**:
   - GenRef: 16 bytes (vs 8 bytes for raw pointer)
   - For large objects (>1KB): overhead < 5%

### Debug vs Release Builds

⚠️ **IMPORTANT**: Always benchmark in release mode!

- **Debug builds**: ~100ns CBGR overhead (interpreted)
- **Release builds**: ~15ns CBGR overhead (optimized)

The 15ns target only applies to release builds with optimizations.

## Interpreting Criterion Output

Criterion will show:
- **Time**: Median time per iteration
- **Lower/Upper bounds**: 95% confidence interval
- **R²**: Goodness of fit (>0.95 is good)
- **Outliers**: Percentage of outlier measurements

Example:
```
cbgr_managed_target_15ns
                        time:   [14.234 ns 14.567 ns 14.891 ns]
Found 12 outliers among 1000 measurements (1.20%)
```

This shows CBGR overhead is ~14.5ns (within the <15ns target).

## Performance Regression CI

The `bench_performance_regression_gates` benchmark can be used in CI to catch performance regressions:

```yaml
# .github/workflows/benchmarks.yml
- name: Run performance regression tests
  run: cargo bench --package verum_cbgr --bench cbgr_overhead_bench -- performance_regression_gates
```

Gates:
- ✅ CBGR validation < 20ns (relaxed for CI variability)
- ✅ Clone < 5ns
- ✅ Generation read < 3ns
- ✅ Alloc/dealloc < 100ns

## Troubleshooting

### High Overhead (>15ns)

If CBGR overhead exceeds 15ns:

1. **Check build mode**: Must be `--release`
2. **Check CPU frequency scaling**: Disable power saving
3. **Check system load**: Close other applications
4. **Check for debug assertions**: Ensure `debug-assertions = false`

### Inconsistent Results

If results vary significantly:

1. **Increase sample size**: `--sample-size 2000`
2. **Increase measurement time**: `--measurement-time 10`
3. **Run with nice**: `nice -n -20 cargo bench ...`
4. **Pin to CPU core**: `taskset -c 0 cargo bench ...`

## Profiling

For detailed profiling:

```bash
# CPU profiling
cargo flamegraph --bench cbgr_overhead_bench

# Cache analysis
valgrind --tool=cachegrind target/release/deps/cbgr_overhead_bench-*

# Memory profiling
valgrind --tool=massif target/release/deps/cbgr_overhead_bench-*
```

## Contributing

When adding new benchmarks:

1. Add to appropriate benchmark file or create new one
2. Update this README
3. Ensure it runs in CI
4. Document expected results
5. Include regression gates if needed

## References

- CBGR performance targets: <15ns cache-hot check, <100ms escape analysis for 10K LOC
- Performance targets: `CLAUDE.md`
- Criterion docs: https://bheisler.github.io/criterion.rs/
