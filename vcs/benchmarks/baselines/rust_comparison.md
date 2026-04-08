# Verum vs Rust Performance Comparison

This document outlines how to fairly compare Verum performance against Rust.

## Philosophy

Rust is Verum's primary performance target. Verum aims for 0.85-0.95x native Rust performance
while providing:
- Semantic type safety (List, Text, Map vs Vec, String, HashMap)
- CBGR memory safety with ~15ns overhead
- Built-in verification via SMT

## Comparison Categories

### 1. Memory Management

| Operation | Rust | Verum Target | Verum Overhead |
|-----------|------|--------------|----------------|
| Stack allocation | 0ns | 0ns | 0% |
| Heap allocation | ~50ns | ~55ns | ~10% |
| Reference check | 0ns | ~15ns | CBGR overhead |
| Drop (simple) | ~5ns | ~5ns | 0% |
| Drop (CBGR) | N/A | ~8ns | Generation recycling |

### 2. Core Operations

```rust
// Rust baseline
fn rust_sum(vec: &Vec<i64>) -> i64 {
    vec.iter().sum()
}

// Verum equivalent
fn verum_sum(list: &List<Int>) -> Int {
    list.iter().sum()
}
```

**Expected Performance:**
- Verum should match Rust within 5% for iterator operations
- SIMD auto-vectorization should be equivalent

### 3. Concurrency

| Operation | Rust | Verum Target |
|-----------|------|--------------|
| Mutex lock (uncontended) | ~25ns | ~25ns |
| Channel send | ~100ns | ~100ns |
| Async spawn | ~500ns | ~500ns |

## Benchmark Methodology

### Environment Setup

```bash
# Rust
rustup default stable
export RUSTFLAGS="-C target-cpu=native -C opt-level=3"

# Verum
verum config set opt-level 3
verum config set target-cpu native
```

### Running Comparisons

1. **Micro-benchmarks**: Use criterion for both
2. **Macro-benchmarks**: Measure wall-clock time with warmup
3. **Memory**: Use heaptrack/massif for both

### Code Equivalence

Ensure code is semantically equivalent:

```rust
// Rust
fn fibonacci(n: u64) -> u64 {
    if n <= 1 { n } else { fibonacci(n - 1) + fibonacci(n - 2) }
}

// Verum (should generate identical LLVM IR)
fn fibonacci(n: Int) -> Int {
    if n <= 1 { n } else { fibonacci(n - 1) + fibonacci(n - 2) }
}
```

### Disabling CBGR for Fair Comparison

For pure computation benchmarks, disable CBGR:

```verum
// Use &checked to skip runtime checks
fn sum_checked(list: &checked List<Int>) -> Int {
    list.iter().sum()
}

// Or &unsafe for manual safety
fn sum_unsafe(list: &unsafe List<Int>) -> Int {
    list.iter().sum()
}
```

## Expected Results

### Where Verum Should Match Rust

1. **Pure computation** (arithmetic, loops)
2. **SIMD operations** (same LLVM backend)
3. **Async runtime** (similar architecture)
4. **FFI calls** (same ABI)

### Where Verum Has Overhead

1. **CBGR checks** (~15ns per reference access)
2. **Context lookups** (~5-30ns per lookup)
3. **Verification checks** (compile-time only, 0ns runtime)

### Where Verum May Be Faster

1. **Escape analysis** (more aggressive than Rust's)
2. **Context-aware optimizations** (DI-based inlining)
3. **SMT-guided dead code elimination**

## Specific Benchmark Comparisons

### String Operations

```rust
// Rust
use std::string::String;
let s = String::from("hello");

// Verum
use verum_std::core::Text;
let s = text!("hello");
```

**Notes:**
- Text is semantically equivalent to String
- Internal representation may differ
- UTF-8 operations should be equivalent

### Collections

```rust
// Rust
let mut vec = Vec::new();
vec.push(1);
vec.push(2);

// Verum
let mut list = list![];
list.push(1);
list.push(2);
```

**Notes:**
- List uses same growth strategy as Vec
- CBGR adds overhead only when borrowing

### Hash Maps

```rust
// Rust
use std::collections::HashMap;
let mut map = HashMap::new();
map.insert("key", 42);

// Verum
use verum_std::core::Map;
let mut map = map!{};
map.insert("key", 42);
```

**Notes:**
- Map uses SipHash by default (same as Rust)
- Equivalent memory layout

## Reporting Results

Format benchmark results as:

```
Benchmark: [name]
Rust:   X.XX ms/op (std: Y.YY)
Verum:  X.XX ms/op (std: Y.YY)
Ratio:  0.XX (Verum/Rust)
CBGR:   on/off
Target: 0.85-0.95
Status: PASS/FAIL
```

## CI Integration

```yaml
benchmark-vs-rust:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v3
    - name: Install Rust
      uses: actions-rs/toolchain@v1
    - name: Install Verum
      run: cargo install verum-cli
    - name: Run comparisons
      run: |
        cargo bench --package rust-baseline
        verum bench --package verum-benchmarks
        verum compare --baseline rust-baseline
```

## Known Differences

1. **Integer overflow**: Verum checks by default (use `wrapping_add` for Rust parity)
2. **Bounds checking**: Verum uses CBGR, Rust uses panic
3. **String encoding**: Both UTF-8, but different internal structures possible
4. **Async executor**: Different implementations, similar performance
