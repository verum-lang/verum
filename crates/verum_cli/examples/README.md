# Verum Language Examples

This directory contains example programs demonstrating various features of the Verum programming language.

## Running Examples

To run any example, use the Verum CLI:

```bash
# Run with default settings (Tier 2 compilation)
verum run examples/hello_world.vr

# Run with specific compilation tier
verum run --tier 0 examples/hello_world.vr  # Interpreted
verum run --tier 1 examples/hello_world.vr  # JIT basic
verum run --tier 2 examples/hello_world.vr  # JIT optimized
verum run --tier 3 examples/hello_world.vr  # Native AOT

# Build without running
verum build examples/fibonacci.vr
```

## Examples Overview

### 1. hello_world.vr
- **Basic Concepts**: Functions, print statements, string interpolation
- **Difficulty**: Beginner
- **Description**: The classic first program in Verum

### 2. fibonacci.vr
- **Concepts**: Functions, recursion, iteration, memoization
- **Difficulty**: Beginner-Intermediate
- **Description**: Three different implementations of the Fibonacci sequence

### 3. refinement_types.vr
- **Concepts**: Refinement types, compile-time safety, domain modeling
- **Difficulty**: Intermediate
- **Key Features**:
  - Positive integers (`Int{> 0}`)
  - Percentage values (`Float{>= 0.0 && <= 100.0}`)
  - Non-empty strings
  - Bank account with invariants
  - Safe division (no divide by zero)

### 4. cbgr_references.vr
- **Concepts**: Three-tier reference model, memory safety, performance
- **Difficulty**: Advanced
- **Key Features**:
  - Managed references (`&T`) - default, ~15ns overhead
  - Checked references (`&checked T`) - statically verified, 0ns
  - Unsafe references (`&unsafe T`) - manual safety, 0ns
  - Reference coercion and escape analysis
  - Performance comparison

### 5. async_context.vr
- **Concepts**: Async/await, dependency injection, concurrency
- **Difficulty**: Advanced
- **Key Features**:
  - Context system for DI
  - Async functions and streams
  - Structured concurrency with supervision
  - Cancellation and timeouts
  - Context inheritance

## Compilation Tiers

Verum supports multiple compilation tiers for different use cases:

| Tier | Mode | Performance | Use Case |
|------|------|------------|----------|
| 0 | Interpreted | Slower | Development, REPL |
| 1 | JIT Basic | 80-90% native | Quick iteration |
| 2 | JIT Optimized | 85-95% native | Production default |
| 3 | AOT Native | 90-100% native | Maximum performance |

## Language Features Demonstrated

### Type System
- **Refinement types**: Compile-time constraints on values
- **Generic types**: Polymorphic functions and data structures
- **Type inference**: Minimal type annotations required

### Memory Safety
- **CBGR**: Check-Before-Garbage-Release system
- **Three-tier references**: Managed, checked, and unsafe
- **Zero undefined behavior**: Guaranteed by type system

### Concurrency
- **Async/await**: First-class async support
- **Structured concurrency**: Supervision trees
- **Channels**: Safe communication between tasks

### Performance
- **Multi-tier compilation**: Choose your performance/compile-time tradeoff
- **Zero-cost abstractions**: High-level features without overhead
- **Profile-guided optimization**: Available in Tier 3

## Building and Testing

```bash
# Type check only (fast)
verum check examples/refinement_types.vr

# Build all examples
for file in examples/*.vr; do
  verum build --tier 3 "$file"
done

# Run with profiling
verum profile examples/cbgr_references.vr

# Verify refinement types
verum verify examples/refinement_types.vr
```

## Contributing

To add a new example:

1. Create a `.vr` file in this directory
2. Add clear comments explaining the concepts
3. Update this README with a description
4. Ensure it compiles with all tiers
5. Add integration tests if demonstrating new features

## Learn More

- [Verum Language Specification](../../../docs/spec.md)
- [Type System Documentation](../../../docs/detailed/03-type-system.md)
- [CBGR Documentation](../../../docs/detailed/24-cbgr-implementation.md)
- [Async/Context System](../../../docs/detailed/06-context-system.md)