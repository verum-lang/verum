# CBGR Fuzzing Infrastructure

This directory contains comprehensive fuzzing targets for the CBGR (Counter-Based Garbage Rejection) memory management system.

## Overview

Fuzzing is a **P1 blocker** for production release. These fuzz targets use `cargo-fuzz` (libFuzzer) to find bugs in:
- Memory safety (use-after-free, double-free, leaks)
- Concurrency (data races, atomicity violations)
- Edge cases (generation wraparound, epoch transitions)
- Capability enforcement

## Requirements

Install cargo-fuzz:
```bash
cargo install cargo-fuzz
```

## Fuzz Targets

### 1. `fuzz_target_allocate_deallocate`
**Purpose**: Tests allocation/deallocation sequences

**What it tests**:
- Random allocation and deallocation patterns
- Various sizes and alignments
- Memory leaks, double-free, use-after-free detection

**Run**:
```bash
cargo fuzz run fuzz_target_allocate_deallocate
```

### 2. `fuzz_target_concurrent_access`
**Purpose**: Tests multi-threaded scenarios

**What it tests**:
- Concurrent allocations/deallocations across threads
- Random ThinRef/FatRef operations (validate, deref, clone)
- Data races, deadlocks, atomicity violations

**Run**:
```bash
cargo fuzz run fuzz_target_concurrent_access
```

### 3. `fuzz_target_wraparound`
**Purpose**: Tests generation counter edge cases

**What it tests**:
- Generation counter approaching GEN_MAX
- Wraparound scenarios
- Epoch transitions
- Generation/epoch mismatch detection

**Run**:
```bash
cargo fuzz run fuzz_target_wraparound
```

### 4. `fuzz_target_capabilities`
**Purpose**: Tests capability flag validation

**What it tests**:
- Capability flag combinations
- Read/Write/Execute/Delegate permissions
- Capability bypass detection
- Capability bit manipulation

**Run**:
```bash
cargo fuzz run fuzz_target_capabilities
```

## Running Fuzzing Tests

### Quick Test (100 runs)
```bash
cargo fuzz run <target> -- -runs=100
```

### Time-Limited Test (5 seconds)
```bash
cargo fuzz run <target> -- -max_total_time=5
```

### Continuous Fuzzing (recommended for 24+ hours)
```bash
cargo fuzz run <target>
```

### Run All Targets
```bash
cargo fuzz list | xargs -I {} cargo fuzz run {} -- -runs=1000
```

## Corpus Management

Fuzz targets maintain a corpus of interesting inputs in:
```
fuzz/corpus/<target_name>/
```

To reset corpus:
```bash
rm -rf fuzz/corpus/<target_name>/*
```

## Crash Artifacts

When a crash is found, artifacts are saved in:
```
fuzz/artifacts/<target_name>/
```

To reproduce a crash:
```bash
cargo fuzz run <target> fuzz/artifacts/<target>/crash-<hash>
```

To minimize a crashing input:
```bash
cargo fuzz tmin <target> fuzz/artifacts/<target>/crash-<hash>
```

## Coverage Reporting

View coverage statistics:
```bash
cargo fuzz coverage <target>
```

Generate coverage report:
```bash
cargo fuzz coverage <target> --html
open fuzz/coverage/<target>/index.html
```

## Performance Targets

CBGR must meet these performance targets:
- CBGR overhead: < 15ns per check
- False positive rate: 0% (zero false negatives)
- Detection rate: 100% (all use-after-free caught)
- Concurrency: No data races, no deadlocks

## Success Criteria

Before production release:
- [ ] All 4 fuzz targets run for 24+ hours without crashes
- [ ] Code coverage ≥ 95% for CBGR core paths
- [ ] No memory leaks detected
- [ ] No double-free bugs
- [ ] No use-after-free false negatives
- [ ] No data races in concurrent scenarios
- [ ] Generation wraparound handled correctly
- [ ] Capability enforcement works as specified

## Continuous Integration

These fuzz targets should be integrated into CI:
```bash
# Run quick fuzzing in CI (5 minutes per target)
for target in $(cargo fuzz list); do
    cargo fuzz run $target -- -max_total_time=300
done
```

## Debugging Crashes

When a crash is found:

1. **Reproduce**:
   ```bash
   cargo fuzz run <target> fuzz/artifacts/<target>/crash-<hash>
   ```

2. **Minimize**:
   ```bash
   cargo fuzz tmin <target> fuzz/artifacts/<target>/crash-<hash>
   ```

3. **Debug with lldb/gdb**:
   ```bash
   lldb fuzz/target/aarch64-apple-darwin/release/<target>
   # Set breakpoint at panic handler
   (lldb) br set -n rust_panic
   (lldb) run fuzz/artifacts/<target>/crash-<hash>
   ```

4. **Add regression test** in `tests/` directory

## Fuzzing Strategy

### Phase 1: Discovery (24-48 hours)
- Run all targets continuously
- Build up corpus of interesting inputs
- Fix any crashes found

### Phase 2: Coverage (48-72 hours)
- Focus on low-coverage areas
- Add seeds to guide fuzzer
- Measure coverage improvements

### Phase 3: Stress Testing (72+ hours)
- Long-running fuzzing campaigns
- Focus on wraparound and concurrent targets
- Monitor for memory leaks

## Notes

- Fuzzing is non-deterministic - different runs find different bugs
- Crashes in concurrent target may be timing-dependent
- Generation wraparound testing requires special seeds
- Coverage should trend upward over time
- Corpus should be committed to git for regression testing

## References

- CBGR Implementation: `../src/`
- CBGR: Capability-Based Generational References providing memory safety via epoch-based generation tracking
- cargo-fuzz docs: https://rust-fuzz.github.io/book/cargo-fuzz.html
