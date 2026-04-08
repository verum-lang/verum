# VBC Interpreter Test Suite

Comprehensive test suite for the Verum Bytecode (VBC) interpreter, designed to verify correctness at all levels of implementation.

## Test Structure

```
vbc/
├── data-movement/         # Opcodes 0x00-0x0F
│   ├── 001_load_immediate.vr      # LOAD_I, LOAD_F, LOAD_TRUE, etc.
│   ├── 002_mov_register.vr        # MOV opcode
│   ├── 003_load_constant.vr       # LOAD_K constant pool
│   └── 004_nan_boxing_edge_cases.vr # NaN-boxing edge cases
│
├── arithmetic/            # Opcodes 0x10-0x2F
│   ├── 001_integer_basic.vr       # ADD_I, SUB_I, MUL_I, DIV_I, MOD_I
│   ├── 002_integer_overflow.vr    # Overflow behavior, checked/saturating
│   ├── 003_float_basic.vr         # ADD_F, SUB_F, MUL_F, DIV_F, IEEE 754
│   ├── 004_bitwise.vr             # BAND, BOR, BXOR, BNOT, SHL, SHR
│   └── 005_unary_ops.vr           # NEG_I, NEG_F, NOT, ABS, POW
│
├── comparison/            # Opcodes 0x30-0x4F
│   ├── 001_integer_comparison.vr  # EQ_I, NE_I, LT_I, LE_I, GT_I, GE_I
│   ├── 002_float_comparison.vr    # Float comparison, NaN handling
│   ├── 003_reference_comparison.vr # EQ_REF, identity comparison
│   └── 004_logic_ops.vr           # AND, OR, XOR, short-circuit
│
├── control-flow/          # Opcodes 0x50-0x5F
│   ├── 001_jumps.vr               # JMP, JMP_IF, JMP_NOT, loops
│   ├── 002_fused_compare_jump.vr  # JMP_EQ, JMP_LT, etc.
│   ├── 003_function_calls.vr      # CALL, RET, CALL_M, closures
│   └── 004_tail_calls.vr          # TAIL_CALL optimization
│
├── memory/                # Opcodes 0x60-0x6F
│   ├── 001_allocation.vr          # NEW, NEW_G, collections
│   └── 002_field_access.vr        # GET_F, SET_F, GET_E, SET_E
│
├── cbgr/                  # Opcodes 0x70-0x7F
│   ├── 001_references.vr          # REF, REF_MUT, DEREF
│   ├── 002_tier_validation.vr     # CHK_REF, Tier 0/1/2
│   ├── 003_use_after_free_detection.vr # Safety violations
│   └── 004_clone_drop.vr          # CLONE, DROP_REF
│
├── generics/              # Opcodes 0x80-0x8F
│   ├── 001_generic_functions.vr   # CALL_G, INSTANTIATE, SIZE_OF_G
│   └── 002_virtual_dispatch.vr    # CALL_V, CALL_C, protocols
│
├── pattern-matching/      # Opcodes 0x90-0x9F
│   ├── 001_variants.vr            # IS_VAR, AS_VAR, MAKE_VARIANT
│   └── 002_tuples_destructuring.vr # UNPACK, PACK
│
├── async/                 # Opcodes 0xA0-0xCF
│   ├── 001_spawn_await.vr         # SPAWN, AWAIT, futures
│   ├── 002_select_join.vr         # SELECT, JOIN
│   ├── 003_generators.vr          # GEN_CREATE, GEN_NEXT, YIELD
│   └── 004_nursery.vr             # NURSERY_* structured concurrency
│
├── context/               # Opcodes 0xB0-0xBF
│   └── 001_provide_using.vr       # CTX_GET, CTX_PROVIDE, contexts
│
├── tensor/                # Opcodes 0xD0-0xFF
│   ├── 001_creation.vr            # TENSOR_NEW, TENSOR_FULL, TENSOR_RAND
│   └── 002_operations.vr          # TENSOR_BINOP, TENSOR_REDUCE, MATMUL
│
├── integration/           # Combined feature tests
│   ├── 001_data_structures.vr     # Linked list, BST, HashMap, Heap
│   └── 002_algorithms.vr          # Sorting, searching, DP, graph
│
└── stress/                # Performance & stability
    ├── 001_memory_stress.vr       # Allocation, GC, CBGR stress
    └── 002_computation_stress.vr  # Arithmetic, float, algorithms
```

## Test Categories

### Level 0 (Critical)
These tests MUST pass for the interpreter to be considered functional:

1. **Data Movement (4 tests)**: NaN-boxing encoding, register operations, constant pool
2. **Arithmetic (5 tests)**: Integer/float operations, overflow handling, bitwise
3. **Comparison (4 tests)**: All comparison operators, IEEE 754 float semantics
4. **Control Flow (4 tests)**: Jumps, function calls, tail call optimization
5. **Memory (2 tests)**: Allocation, field/element access
6. **CBGR (4 tests)**: Three-tier references, safety violation detection
7. **Generics (2 tests)**: Monomorphization, virtual dispatch
8. **Pattern Matching (2 tests)**: Variants, destructuring
9. **Async (4 tests)**: Spawn/await, select/join, generators, nursery
10. **Context (1 test)**: Dependency injection system
11. **Tensor (2 tests)**: Creation, operations

### Integration Tests
Verify correct interaction of multiple features:
- Data structures using memory, generics, pattern matching
- Algorithms using control flow, arithmetic, recursion

### Stress Tests
Verify stability under load:
- Memory: 100K allocations, CBGR validation, GC pressure
- Computation: Overflow boundaries, float stability, algorithmic correctness

## Running Tests

```bash
# Run all VBC tests
cd vcs && make test-vbc

# Run specific category
make test-vbc-arithmetic
make test-vbc-cbgr
make test-vbc-async

# Run stress tests (longer timeout)
make test-vbc-stress

# Run with verbose output
make test-vbc VERBOSE=1
```

## Test Format

Each test file follows this format:

```verum
// @test: unit|integration|stress
// @tier: 0
// @level: L0
// @tags: vbc, category, specific-features
// @timeout: milliseconds
// @expect: pass|fail|error(ErrorType)

fn test_feature() {
    // Test implementation
    assert_eq(actual, expected);
}

fn main() {
    test_feature();
    print("test_name: PASSED");
}
```

## Coverage Goals

| Category | Opcodes | Tests | Coverage |
|----------|---------|-------|----------|
| Data Movement | 0x00-0x0F | 4 | 100% |
| Arithmetic | 0x10-0x2F | 5 | 100% |
| Comparison | 0x30-0x3F | 4 | 100% |
| Logic | 0x40-0x4F | 1 | 100% |
| Control Flow | 0x50-0x5F | 4 | 100% |
| Memory | 0x60-0x6F | 2 | 100% |
| CBGR | 0x70-0x7F | 4 | 100% |
| Generics | 0x80-0x8F | 2 | 100% |
| Pattern Match | 0x90-0x9F | 2 | 100% |
| Async | 0xA0-0xA7 | 4 | 100% |
| Autodiff | 0xA8-0xAF | - | Pending |
| Context | 0xB0-0xBF | 1 | 100% |
| Debug | 0xC0-0xCF | - | Pending |
| Tensor Create | 0xD0-0xD7 | 1 | 100% |
| Tensor Shape | 0xD8-0xDF | 1 | Partial |
| Tensor Ops | 0xE0-0xEF | 1 | Partial |
| Tensor Reduce | 0xF0-0xF7 | 1 | Partial |
| GPU | 0xF8-0xFF | - | Pending |

## Key Verification Points

### NaN-Boxing
- All 8 tag types correctly encoded/decoded
- 48-bit integer range preserved
- Float NaN distinction from tagged values
- Small string inline optimization

### CBGR (Capability-Based Generational References)
- Tier 0: ~15ns validation overhead
- Tier 1: Zero overhead (compiler-proven)
- Tier 2: Zero overhead (manual proof)
- Use-after-free detection
- Double-free prevention
- Generation/epoch tracking

### Async
- Cooperative task scheduling
- Future state management
- Structured concurrency (nursery)
- Cancellation propagation
- Generator suspend/resume

### Tensor
- Shape operations as views
- Broadcasting semantics
- Autodiff gradient flow
- Device placement (CPU/GPU)

## Performance Targets

| Operation | Target |
|-----------|--------|
| CBGR check (Tier 0) | < 15ns |
| Function call | < 100ns |
| Object allocation | < 500ns |
| Arithmetic op | < 10ns |
| Context lookup | < 30ns |

## Known Issues to Test

1. Integer overflow at 48-bit boundary
2. Float comparison with NaN
3. Reference escape from scope
4. Generator state corruption
5. Async task cancellation cleanup
6. Context shadowing in nested scopes
7. Generic monomorphization explosion
8. Tensor view invalidation
