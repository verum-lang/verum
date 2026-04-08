# core/math Test Suite

Test coverage for Verum's math module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `constants_test.vr` | `math/constants` | Mathematical constants, IEEE 754 values, refinement types | ~35 |
| `integers_test.vr` | `math/integers` | Division modes, GCD, LCM, binary GCD, mod_inverse, Miller-Rabin, primality, factorials, binomials | ~90 |
| `elementary_test.vr` | `math/elementary` | Trig, exp, log, power, rounding, min/max, interpolation | ~115 |
| `bits_test.vr` | `math/bits` | clz, ctz, popcnt, rotation, byte swap, power-of-two, bit fields | ~80 |
| `hyperbolic_test.vr` | `math/hyperbolic` | sinh, cosh, tanh, asinh, acosh, atanh, Gudermannian | ~45 |
| `special_test.vr` | `math/special` | Gamma, error functions, Bessel, incomplete gamma, recurrence, duplication | ~56 |
| `random_test.vr` | `math/random` | PRNG algorithms, distributions, sampling, shuffling, categorical | ~67 |
| `linalg_test.vr` | `math/linalg` | Vectors, matrices, BLAS L1/L2/L3, decompositions, SVD, eigen, Schur | ~85 |
| `ieee754_test.vr` | `math/ieee754` | FpCategory, F64Bits, classify, is_nan/infinite/finite/normal/subnormal/zero, signbit, copysign, negate, decompose/compose | ~81 |
| `checked_test.vr` | `math/checked` | Checked, saturating, wrapping arithmetic | ~86 |
| `rag_test.vr` | `math/rag` | Document, VectorStore, HNSW, TextChunker, BM25, HybridRetriever | ~93 |
| `simple_test.vr` | `math/simple` | Tensor aliases, Batch, Sequential, zeros/ones/full/matmul/softmax/dropout | ~49 |
| `agent_test.vr` | `math/agent` | Role, Message, Request, ChatMessage, ToolChoice, BPE/KVCache/quantize | ~81 |
| `autodiff_test.vr` | `math/autodiff` | DiffMode, OpType, ComputeGraph, MemoryTracker, vjp/jvp/grad/jacobian/hessian | ~86 |
| `advanced_test.vr` | `math/advanced` | SimdLevel, Precision, RoundingMode, CBGRStats, KernelVariant, Workspace | ~69 |
| `ssm_test.vr` | `math/ssm` | DiscretizationMethod, RoutingStrategy, BalanceLoss, Activation, Mamba/Jamba/MoE | ~49 |
| `distributed_test.vr` | `math/distributed` | DistributedConfig, AllReduceOp, DistributedTrainer, ProcessGroup, MoE | ~74 |
| `guardrails_test.vr` | `math/guardrails` | ContentPolicy, ToxicityLevel, GuardrailConfig, SafetyFilter | ~46 |
| `nn_test.vr` | `math/nn` | LayerType, ActivationFn, Optimizer, LRScheduler, neural network layers | ~49 |
| `gpu_test.vr` | `math/gpu` | GPUBackend, DeviceCapability, MemoryPool, kernel launch | ~74 |
| `calculus_test.vr` | `math/calculus` | Integral, Derivative, ODE solvers, boundary conditions | ~120 |
| `internal_test.vr` | `math/internal` | Tensor internals, DType, Layout, Shape, Stride | ~68 |
| `tensor_test.vr` | `math/tensor` | Tensor, TensorView, creation, unary/activation/reduction ops, linalg | ~100 |

## Key Types Tested

### Mathematical Constants
- **Fundamental**: PI, TAU, E, PHI
- **Square roots**: SQRT2, SQRT3, SQRT5, FRAC_1_SQRT2
- **Logarithms**: LN2, LN10, LOG2_E, LOG10_E, LOG2_10, LOG10_2
- **Pi fractions**: FRAC_PI_2, FRAC_PI_4, FRAC_1_PI

### IEEE 754 Special Values
- EPSILON (Float64 machine epsilon)
- INFINITY, NEG_INFINITY
- NAN (quiet NaN)
- MAX_FLOAT, MIN_POSITIVE, MIN_SUBNORMAL
- Float32 variants: EPSILON_F32, MAX_F32, MIN_POSITIVE_F32

### Refinement Types
- `NonNegative` - Float{>= 0.0}
- `Positive` - Float{> 0.0}
- `UnitInterval` - Float{0.0 <= x <= 1.0}
- `ClosedProbability` - Float{0.0 <= x <= 1.0}
- `Correlation` - Float{-1.0 <= x <= 1.0}
- `NatInt` - Int{>= 0}
- `PosInt` - Int{> 0}

### Elementary Functions
- **Trigonometric**: sin, cos, tan, sincos, asin, acos, atan, atan2
- **Exponential**: exp, exp2, expm1
- **Logarithmic**: log, log2, log10, log1p
- **Power**: pow, powi, sqrt, cbrt, hypot
- **Rounding**: floor, ceil, round, trunc, fract
- **Utilities**: min, max, clamp, abs, signum, fma
- **Interpolation**: lerp, inverse_lerp, remap, smoothstep, smootherstep

### Bit Manipulation
- **Counting**: clz, ctz, popcnt (leading/trailing zeros, population count)
- **Position**: ffs, fls, highest_bit, lowest_bit
- **Rotation**: rotl, rotr (rotate left/right)
- **Byte order**: bswap, bitreverse
- **Power of two**: is_power_of_two, next_power_of_two, prev_power_of_two
- **Log2**: log2_floor, log2_ceil
- **Bit fields**: extract_bits, insert_bits, test_bit, set_bit, clear_bit, toggle_bit
- **Morton codes**: interleave_2d, deinterleave_2d

### Hyperbolic Functions
- **Direct**: sinh, cosh, tanh, sech
- **Inverse**: asinh, acosh, atanh, acosh_safe, atanh_safe
- **Special**: gudermannian, inverse_gudermannian

### DivMode Variants (integers)
- `Truncate` - Truncation toward zero
- `Floor` - Round toward negative infinity
- `Ceiling` - Round toward positive infinity
- `Euclidean` - Euclidean division (non-negative remainder)

### Integer Operations
- Division: `div_floor`, `div_ceiling`, `div_euclidean`
- Modulo: `mod_floor`, `mod_euclidean`
- Generic: `div(a, b, mode)`, `divmod(a, b)`

### Number Theory
- GCD: `gcd`, `gcd_extended`
- LCM: `lcm`
- Binary GCD: `gcd_binary`
- Primality: `is_prime`, `is_prime_miller_rabin`, `next_prime`
- Modular: `pow_mod`, `mod_inverse`

### Integer Roots
- `isqrt` - Integer square root
- `icbrt` - Integer cube root

### Combinatorics
- `factorial`
- `binomial` - Binomial coefficient
- `falling_factorial` - Falling factorial (n)_k
- `rising_factorial` - Rising factorial n^(k)

### Special Functions
- **Gamma**: gamma, lgamma, digamma, trigamma
- **Beta**: beta, lbeta
- **Error functions**: erf, erfc, erfinv, erfcinv
- **Bessel**: j0, j1, y0, y1, jn
- **Incomplete gamma**: gammainc_lower, gammainc_upper, gammainc_p, gammainc_q

### Random Number Generation
- **PRNG protocols**: RandomSource, SeedableRng
- **Generators**: Xoshiro256PlusPlus, Pcg64, SplitMix64
- **Distributions**: Uniform, Normal, Exponential, Poisson, Binomial, Geometric
- **Sampling**: random_choice, weighted_choice, sample, shuffle

### Linear Algebra Types
- `Vector<T>` - Dynamic vector with runtime size
- `Matrix<T>` - Dynamic matrix with runtime dimensions
- `StaticVector<T, N>` - Static vector with compile-time size
- `StaticMatrix<T, M, N>` - Static matrix with compile-time dimensions

### BLAS Operation Types
- `Transpose` - NoTrans | Trans | ConjTrans
- `TriangularType` - Upper | Lower
- `DiagonalType` - Unit | NonUnit

### Matrix Decomposition Types
- `LUDecomposition<T>` - LU factorization with pivoting
- `QRDecomposition<T>` - Q (orthogonal) and R (upper triangular)
- `CholeskyDecomposition<T>` - L (lower triangular) where A = L*L^T

## Tests by Category

### Constants Tests (~35 tests)
- Value bounds verification
- Constant relationship verification (TAU = 2*PI, etc.)
- IEEE 754 property tests
- Refinement type assignment tests

### Integers Tests (~90 tests)
- DivMode variant tests and distinctness checks
- Division with positive/negative numbers, edge cases (divisor=1, exact division)
- GCD/LCM edge cases (zero, negative, coprime), commutativity, relationship property
- Binary GCD (Stein's algorithm) and agreement with Euclidean GCD
- Extended GCD verification (Bezout identity), additional cases
- Modular inverse (exists, not-coprime, identity)
- Miller-Rabin primality test, agreement with trial division
- Primality testing (small, medium, larger primes, negatives)
- next_prime edge cases (already prime, zero, prime gaps)
- pow_mod with Fermat's little theorem verification
- Factorial and binomial coefficient verification
- Pascal's rule and row sum verification
- Falling/rising factorial extended values and factorial equivalence
- Integer roots for large perfect squares/cubes, isqrt property verification

### Elementary Tests (~115 tests)
- Trigonometric function values at special angles
- Inverse trig function correctness
- atan2 quadrant tests
- Exponential/logarithmic identities (exp(log(x)) = x)
- Power function edge cases
- Rounding behavior for positive/negative values
- Min/max/clamp behavior
- FMA precision test
- Linear interpolation endpoints and midpoints
- Smoothstep edge behavior

### Bits Tests (~80 tests)
- CLZ/CTZ for powers of two and mixed values
- Population count for various bit patterns
- FFS/FLS correctness
- Rotation wrap-around behavior
- Byte swap symmetry (bswap(bswap(x)) = x)
- Bit reverse symmetry
- Power-of-two detection and rounding
- Log2 floor/ceiling for exact and non-exact values
- Parity computation
- Bit field extraction/insertion
- Morton code roundtrip (interleave/deinterleave)

### Hyperbolic Tests (~45 tests)
- Function values at zero and special points
- Odd/even function symmetry (sinh(-x) = -sinh(x), cosh(-x) = cosh(x))
- Bounds verification (cosh >= 1, |tanh| < 1)
- Inverse function roundtrips (asinh(sinh(x)) = x)
- Hyperbolic Pythagorean identity (cosh² - sinh² = 1)
- Gudermannian properties

### Special Functions Tests (~65 tests)
- **Gamma functions**: gamma at integers, half-integers; lgamma; digamma; trigamma
- **Beta function**: beta, lbeta, symmetry property beta(a,b) = beta(b,a)
- **Error functions**: erf, erfc; symmetry (erf(-x) = -erf(x)); bounds |erf| <= 1
- **Inverse error functions**: erfinv, erfcinv; roundtrip verification
- **Bessel functions**: j0, j1, jn; y0, y1; values at special points
- **Incomplete gamma**: gammainc_lower, gammainc_upper, gammainc_p, gammainc_q

### Random Tests (~95 tests)
- **PRNG algorithms**: Xoshiro256++, PCG64, SplitMix64; period verification
- **Uniform distributions**: uniform_int, uniform_float; range bounds
- **Normal distribution**: standard_normal; mean and variance properties
- **Other distributions**: exponential, poisson, binomial, geometric
- **Sampling**: random_choice, weighted_choice, sample_without_replacement
- **Shuffling**: shuffle; all elements preserved; randomness verification

### Linear Algebra Tests (~100 tests)
- **Vector operations**: construction (zeros, filled), access (get, set), arithmetic (add, sub, hadamard)
- **Vector norms**: L1, L2, infinity norm; scale, axpy, dot product
- **Matrix operations**: construction (zeros, eye, diag), access, transpose, arithmetic
- **Matrix products**: matrix-vector (matvec), matrix-matrix (matmul); Frobenius norm, trace
- **BLAS Level 1**: dot, nrm2, asum, iamax, scal, axpy, copy, swap, rotg
- **BLAS Level 2**: gemv (with transpose), trsv (triangular solve), ger (rank-1 update)
- **BLAS Level 3**: gemm (general matrix multiply)
- **LU decomposition**: factorization, solve via LU
- **QR decomposition**: Q orthogonality (Q^T*Q = I), R upper triangular, Q*R = A
- **Cholesky decomposition**: L*L^T = A for SPD matrices; failure for non-SPD
- **Utilities**: solve linear systems, matrix inverse, determinant

## Known Limitations

- NaN comparison tests use self-equality check (NaN != NaN)
- Refinement type tests verify assignment only (not runtime enforcement)
- Large factorial values may overflow (Int range limits)
- `checked_test.vr` skipped - requires LLVM intrinsic codegen support
- Refinement type arithmetic requires explicit Float typing to avoid type errors

### RAG Tests (~93 tests)
- **Document**: construction (.new, .with_metadata, .with_embedding), field access, mutation
- **HNSWConfig**: .default(), .with_m(), field verification, direct construction
- **HNSWIndex**: construction with various dimensions, .len(), config access
- **HNSWVectorStore**: .new(), .with_defaults(), VectorStore protocol impl
- **ChunkStrategy**: all 4 variants (FixedSize, Separator, Recursive, Sentence), discrimination
- **TextChunker**: .new(), .with_separator(), .chunk() on various inputs
- **BM25Config**: .default(), direct construction, field access
- **BM25Index**: .new(), .add(), .search(), .count(), SparseIndex protocol impl
- **HybridRetriever**: generic instantiation, .with_dense_weight(), .count()
- **Integration**: type constraint verification, protocol implementation checks

## Test Count: 1,689 tests total (35 test files, 35 passing + 0 skipped)
## Note: There are also ~10 @test:run debug files (struct_test, vector_*_test, etc.) that test runtime behavior
