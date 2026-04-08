# Verum vs C Performance Comparison

This document outlines how to compare Verum performance against C.

## Philosophy

C represents the theoretical maximum performance achievable. Verum aims for:
- 0.85-0.95x C performance for compute-bound code
- Equivalent SIMD performance (same LLVM backend)
- Overhead only from safety features (CBGR, bounds checks)

## Expected Performance Ratio

| Category | Expected Verum/C |
|----------|------------------|
| Pure arithmetic | 0.95-1.0x |
| SIMD operations | 1.0x (identical) |
| Memory access (CBGR on) | 0.80-0.90x |
| Memory access (CBGR off) | 0.95-1.0x |
| System calls | 1.0x (identical) |
| String operations | 0.95-1.0x |

## Comparison Categories

### 1. Pure Computation

```c
// C
int64_t fibonacci(int64_t n) {
    if (n <= 1) return n;
    return fibonacci(n - 1) + fibonacci(n - 2);
}
```

```verum
// Verum
fn fibonacci(n: Int) -> Int {
    if n <= 1 { n } else { fibonacci(n - 1) + fibonacci(n - 2) }
}
```

**Expected:** Identical performance (same LLVM IR)

### 2. Array Operations

```c
// C
int64_t sum_array(int64_t* arr, size_t len) {
    int64_t sum = 0;
    for (size_t i = 0; i < len; i++) {
        sum += arr[i];
    }
    return sum;
}
```

```verum
// Verum with CBGR (default)
fn sum_array(arr: &List<Int>) -> Int {
    arr.iter().sum()
}

// Verum without CBGR
fn sum_array_unchecked(arr: &checked List<Int>) -> Int {
    arr.iter().sum()
}
```

**Expected:**
- With CBGR: ~90% of C (reference checks)
- Without CBGR: ~99% of C

### 3. SIMD Operations

```c
// C with intrinsics
#include <immintrin.h>

void add_vectors(float* a, float* b, float* c, size_t n) {
    for (size_t i = 0; i < n; i += 8) {
        __m256 va = _mm256_loadu_ps(&a[i]);
        __m256 vb = _mm256_loadu_ps(&b[i]);
        __m256 vc = _mm256_add_ps(va, vb);
        _mm256_storeu_ps(&c[i], vc);
    }
}
```

```verum
// Verum SIMD
fn add_vectors(a: &[Float32], b: &[Float32], c: &mut [Float32]) {
    for i in (0..a.len()).step_by(8) {
        let va = f32x8.load(&a[i]);
        let vb = f32x8.load(&b[i]);
        let vc = va + vb;
        vc.store(&mut c[i]);
    }
}
```

**Expected:** Identical performance (same intrinsics)

### 4. Memory Allocation

```c
// C
void* ptr = malloc(1024);
memset(ptr, 0, 1024);
free(ptr);
```

```verum
// Verum
let ptr = Heap.alloc_zeroed::<[u8; 1024]>();
drop(ptr);
```

**Expected:**
- malloc/free: identical (same allocator possible)
- CBGR adds ~10% overhead for tracking

### 5. String Operations

```c
// C
char* str = strdup("hello");
char* result = strstr(str, "ell");
free(str);
```

```verum
// Verum
let str = text!("hello");
let result = str.find("ell");
```

**Expected:** Verum slightly faster (better optimized string library)

### 6. Matrix Multiplication

```c
// C with BLAS
#include <cblas.h>

void matmul(double* A, double* B, double* C, int n) {
    cblas_dgemm(CblasRowMajor, CblasNoTrans, CblasNoTrans,
                n, n, n, 1.0, A, n, B, n, 0.0, C, n);
}
```

```verum
// Verum with BLAS binding
fn matmul(a: &Matrix<Float>, b: &Matrix<Float>) -> Matrix<Float> {
    Linalg.blas_dgemm(a, b)
}
```

**Expected:** Identical (same BLAS library)

## Benchmark Methodology

### Compiler Flags

```bash
# C
gcc -O3 -march=native -flto benchmark.c -o benchmark

# Verum
verum build --release --target-cpu=native --lto
```

### Disabling Safety for Fair Comparison

```verum
// Disable CBGR for pure performance comparison
fn benchmark_unchecked(data: &unsafe [Int]) -> Int {
    // SAFETY: We verify bounds manually
    unsafe {
        let mut sum = 0;
        for i in 0..data.len() {
            sum += *data.get_unchecked(i);
        }
        sum
    }
}
```

### Memory Layout Verification

Ensure structs have identical layout:

```c
// C
struct Point {
    double x;
    double y;
};
_Static_assert(sizeof(struct Point) == 16, "");
```

```verum
// Verum
#[repr(C)]
struct Point {
    x: Float,
    y: Float,
}
static_assert!(size_of::<Point>() == 16);
```

## Specific Benchmarks

### Memory Copy

```c
// C
memcpy(dst, src, 1000000);
```

```verum
// Verum
dst.copy_from_slice(src);
```

**Expected:** Identical (same optimized memcpy)

### Sorting

```c
// C
int cmp(const void* a, const void* b) {
    return *(int*)a - *(int*)b;
}
qsort(arr, n, sizeof(int), cmp);
```

```verum
// Verum
arr.sort();
```

**Expected:** Verum slightly faster (inline comparisons)

### Hash Table Operations

```c
// C with custom hash table
struct hashmap* map = hashmap_create();
hashmap_insert(map, "key", value);
void* val = hashmap_get(map, "key");
hashmap_destroy(map);
```

```verum
// Verum
let mut map = map!{};
map.insert("key", value);
let val = map.get("key");
```

**Expected:** Similar performance (both use optimized hash functions)

## LLVM IR Comparison

Both should generate similar LLVM IR:

```bash
# C
clang -S -emit-llvm -O3 benchmark.c -o benchmark.ll

# Verum
verum build --emit=llvm-ir benchmark.vr -o benchmark.ll
```

Compare critical functions:
```bash
diff <(grep -A 50 "define.*fibonacci" c_benchmark.ll) \
     <(grep -A 50 "define.*fibonacci" verum_benchmark.ll)
```

## Reporting Results

```
Benchmark: [name]
C:        X.XX ms/op
Verum:    X.XX ms/op
Ratio:    0.XX (Verum/C)
CBGR:     on/off/checked
Target:   0.85-0.95
Status:   PASS/FAIL
```

## CI Integration

```yaml
benchmark-vs-c:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v3
    - name: Setup C compiler
      run: sudo apt-get install gcc-12
    - name: Compile C baseline
      run: gcc-12 -O3 -march=native -o baseline baseline.c
    - name: Install Verum
      run: cargo install verum-cli
    - name: Run comparison
      run: |
        ./baseline > c_results.txt
        verum bench > verum_results.txt
        verum compare c_results.txt verum_results.txt
```

## Safety Overhead Analysis

### CBGR Overhead Breakdown

| Component | Cost | When |
|-----------|------|------|
| Generation check | ~5ns | Every reference access |
| Epoch check | ~3ns | Every reference access |
| Bounds check | ~2ns | Array indexing |
| Generation increment | ~5ns | Reference creation |
| Total | ~15ns | Per reference operation |

### When to Disable CBGR

1. **Inner loops with proven safety** (use `&checked`)
2. **FFI boundaries** (C doesn't understand CBGR)
3. **Performance-critical paths** (after verification)

## Known Differences

1. **Integer overflow**: Verum checks by default (disable with `.wrapping_add()`)
2. **Null pointers**: Verum uses Maybe<T>, C uses NULL
3. **Alignment**: May differ for some types
4. **Calling convention**: Verum uses Rust ABI, C uses C ABI

## Where Verum May Exceed C

1. **Escape analysis**: More aggressive optimizations
2. **Inlining**: Better cross-module inlining with LTO
3. **Bounds check elimination**: SMT-proven bounds removal
4. **Cache optimization**: Layout optimizations based on access patterns
