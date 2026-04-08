# core/simd Test Suite

Test coverage for Verum's portable SIMD (Single Instruction, Multiple Data) module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `simd_types_test.vr` | `simd` | SimdElement, Vec type aliases, Mask types, platform constants, instantiation | 39 |
| `simd_operations_test.vr` | `simd` | Construction, arithmetic, reductions, comparisons, shuffle, edge cases | 121 |
| `simd_mask_test.vr` | `simd` | Mask construction, boolean operations, count, any/all, idempotent/absorption laws | 57 |
| `simd_alias_test.vr` | `simd` | Type alias method resolution, custom aliases, interop | 17 |
| `simd_instance_test.vr` | `simd` | Instance method calls on type alias values, chaining | 22 |
| `simd_static_test.vr` | `simd` | Static method calls on type aliases (splat, from_array, select, mask) | 17 |

## Key Types Tested

### SimdElement Protocol
Protocol for types that can be SIMD vector elements.

**Implementations:**
- `Float32` (32 bits)
- `Float64` (64 bits)
- `Int8`, `Int16`, `Int32`, `Int64`
- `UInt8`, `UInt16`, `UInt32`, `UInt64`

**Associated Constants:**
- `BITS` - Number of bits per element
- `LANES_128` - Lanes in 128-bit vector (SSE/NEON)
- `LANES_256` - Lanes in 256-bit vector (AVX)
- `LANES_512` - Lanes in 512-bit vector (AVX-512)

### Vec<T, N>
Portable SIMD vector type.

**Type Aliases (128-bit):**
- `Vec4f` - `Vec<Float32, 4>`
- `Vec2d` - `Vec<Float64, 2>`
- `Vec4i` - `Vec<Int32, 4>`
- `Vec2l` - `Vec<Int64, 2>`
- `Vec16b` - `Vec<UInt8, 16>`
- `Vec8s` - `Vec<Int16, 8>`

**Type Aliases (256-bit):**
- `Vec8f` - `Vec<Float32, 8>`
- `Vec4d` - `Vec<Float64, 4>`
- `Vec8i` - `Vec<Int32, 8>`
- `Vec4l` - `Vec<Int64, 4>`

**Type Aliases (512-bit):**
- `Vec16f` - `Vec<Float32, 16>`
- `Vec8d` - `Vec<Float64, 8>`
- `Vec16i` - `Vec<Int32, 16>`
- `Vec8l` - `Vec<Int64, 8>`

**Construction:**
- `splat(value)` - Broadcast scalar to all lanes
- `from_array(arr)` - Create from array
- `to_array()` - Convert to array
- `load_aligned(ptr)` - Load from aligned memory
- `load_unaligned(ptr)` - Load from unaligned memory

**Arithmetic:**
- `add(other)` - Lane-wise addition
- `sub(other)` - Lane-wise subtraction
- `mul(other)` - Lane-wise multiplication
- `div(other)` - Lane-wise division
- `fma(b, c)` - Fused multiply-add: a * b + c
- `abs()` - Lane-wise absolute value
- `neg()` - Lane-wise negation
- `min(other)` - Lane-wise minimum
- `max(other)` - Lane-wise maximum

**Reductions:**
- `reduce_add()` - Sum all lanes
- `reduce_mul()` - Multiply all lanes
- `reduce_min()` - Find minimum across lanes
- `reduce_max()` - Find maximum across lanes

**Comparisons:**
- `cmp_lt(other)` - Less than (returns Mask)
- `cmp_le(other)` - Less than or equal
- `cmp_gt(other)` - Greater than
- `cmp_ge(other)` - Greater than or equal
- `cmp_eq(other)` - Equal
- `cmp_ne(other)` - Not equal

**Shuffle/Permute:**
- `shuffle<MASK>(other)` - Shuffle using compile-time mask
- `reverse()` - Reverse lane order
- `rotate_left<COUNT>()` - Rotate lanes left

**Conditional:**
- `select(mask, a, b)` - Per-lane select based on mask
- `masked_load(ptr, mask)` - Load with mask
- `masked_store(ptr, mask)` - Store with mask

### Mask<N>
SIMD mask type for conditional operations.

**Type Aliases:**
- `Mask4` - 4-lane mask
- `Mask8` - 8-lane mask
- `Mask16` - 16-lane mask

**Construction:**
- `all()` - All lanes active
- `none()` - No lanes active

**Inspection:**
- `count()` - Count active lanes
- `any()` - Check if any lane active
- `all_active()` - Check if all lanes active

**Boolean Operations:**
- `and(other)` - Bitwise AND
- `or(other)` - Bitwise OR
- `not()` - Bitwise NOT

### Platform Feature Constants

Compile-time platform detection:
- `HAS_SSE42` - SSE 4.2 support (x86)
- `HAS_AVX` - AVX support (x86)
- `HAS_AVX2` - AVX2 support (x86)
- `HAS_AVX512` - AVX-512 support (x86)
- `HAS_NEON` - NEON support (ARM)

## Tests by Category

### Type Definition Tests (39 tests)
- SimdElement implementations for all numeric types
- BITS constant values
- LANES_128/256/512 derivations
- Vec type alias compatibility (128/256/512-bit, extended byte/short)
- Mask type alias compatibility
- Platform constant definitions
- Vec type alias instantiation (splat + to_array)
- Mask type instantiation
- Vec generic type equivalence

### Operation Tests (121 tests)
- Vector construction (splat, from_array, to_array, negative, zero)
- Binary arithmetic (add, sub, mul, div) with identity elements
- Arithmetic with zero (add zero, sub zero, mul one, mul zero, div by one, sub self)
- Arithmetic with negative values (add, sub, mul for float and int)
- Fused multiply-add (fma, fma with zero mul, fma with zero add, fma identity)
- Unary operations (abs, neg, abs on all-negative, abs on positive, neg neg identity)
- Min/max operations (equal vectors, negative values)
- Horizontal reductions (reduce_add, reduce_mul, reduce_min, reduce_max with zero/negative/ones)
- Comparison operations (cmp_lt, cmp_le, cmp_gt, cmp_ge, cmp_eq, cmp_ne for float and int)
- Conditional select (all true, all false, partial)
- Shuffle operations (reverse, rotate_left 1/2/3 for float and int)
- 256-bit vector operations (add, sub, mul, reduce for Vec8f, Vec8i, Vec4d)
- Chained operations (add-sub, mul-add, abs after sub)
- Construction patterns (splat negative, splat zero, from_array with negatives, Vec2l)

### Mask Tests (57 tests)
- Mask construction (all, none) for Mask4/8/16
- Count operations (all, none, from comparison)
- Any/all predicates
- Boolean operations (and, or, not) for Mask4/8/16
- De Morgan's laws verification
- Double negation
- Type alias compatibility
- Integer vector masks and select
- Mask from various comparisons (cmp_ge, cmp_le, cmp_eq, cmp_ne)
- Mask with select operations (all mask, none mask, partial)
- Boolean operation combinations
- Idempotent laws (A AND A = A, A OR A = A)
- Absorption laws (A AND all = A, A OR none = A, A AND none = none, A OR all = all)
- Mask16 operations (and, or, not, double not)

### Alias Tests (17 tests)
- Direct Vec and type alias splat
- Custom type alias (MyVec4, MyVec2D, MyVec4Int)
- Alias from_array and arithmetic
- Alias reduce operations
- Alias interop between Vec4f and custom aliases
- Alias mask operations

### Instance Method Tests (22 tests)
- Instance arithmetic (add, sub, mul, div)
- Instance unary (abs, neg)
- Instance reductions (reduce_add, reduce_mul, reduce_min, reduce_max)
- Instance comparisons (cmp_lt, cmp_eq)
- Instance shuffle (reverse, rotate_left)
- Instance methods on different types (Vec2d, Vec4i min/max)
- Instance method chaining (add-mul, sub-abs, fma)

### Static Method Tests (17 tests)
- Static splat methods for Vec4f/Vec4i/Vec2d/Vec2l/Vec8f/Vec8i
- Static from_array methods
- Static select method (Vec4f, Vec4i)
- Static mask construction (Mask4/8/16 all/none)

## Known Limitations

- **Method resolution on type aliases (Task #21)**: ✅ FIXED - `Vec4f.splat()` and instance methods now work. The type alias is resolved to the base type (Vec) for method lookup.
- **Method overload resolution**: ✅ FIXED - Vec and Mask converted to newtypes (`{ lanes: [T; N] }` instead of `[T; N]`) to prevent array/Iterator methods from shadowing SIMD methods like `min()`, `max()`, `count()`, `any()`.
- **Const generic method syntax**: ✅ WORKS - `rotate_left<1>()` syntax is supported.
- **Protocol constant access (Task #22)**: `Float32.BITS` doesn't work - protocol associated constants aren't accessible via implementing types.
- Gather/scatter operations not tested (require raw pointers)
- Masked load/store not tested (require raw pointers)
- Multi-versioning not tested (requires runtime dispatch)
- Platform-specific code paths not tested (require actual SIMD hardware)

## Current Test Status

| File | Status | Notes |
|------|--------|-------|
| `simd_types_test.vr` | **PASSING** | Import verification + instantiation (39 tests) |
| `simd_static_test.vr` | **PASSING** | Static method calls on type aliases (17 tests) |
| `simd_instance_test.vr` | **PASSING** | Instance method calls + chaining (22 tests) |
| `simd_operations_test.vr` | **PASSING** | Full arithmetic, reduction, comparison, shuffle coverage (121 tests) |
| `simd_mask_test.vr` | **PASSING** | Mask boolean algebra, comparisons, select (57 tests) |
| `simd_alias_test.vr` | **PASSING** | Type alias interop + custom aliases (17 tests) |

## Test Count: 273 @test annotations total (6 test files)

## Architecture Notes

### Platform Targeting

Verum SIMD compiles to optimal instructions on:
- **x86_64**: SSE4.2 (128-bit), AVX2 (256-bit), AVX-512 (512-bit)
- **aarch64**: NEON (128-bit), SVE (scalable)

### VBC Opcodes

SIMD operations use VBC opcodes 0xC0-0xCF:
- `SIMD_SPLAT` - Broadcast scalar
- `SIMD_LOAD` - Aligned load
- `SIMD_BINOP` - Element-wise arithmetic
- `SIMD_FMA` - Fused multiply-add
- `SIMD_REDUCE` - Horizontal reduction
- `SIMD_SHUFFLE` - Permute elements
- `SIMD_GATHER/SCATTER` - Indexed load/store

### Performance Targets

- `Vec.splat()`: Single broadcast instruction
- `Vec.add()`: Single SIMD add instruction
- `Vec.fma()`: Single FMA instruction (if available)
- `Vec.reduce_add()`: Optimal horizontal sum (e.g., HADDPS on SSE3)
