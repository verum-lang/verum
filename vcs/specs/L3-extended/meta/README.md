# Meta-System Test Suite

This directory contains the comprehensive test suite for Verum's meta-programming system as specified in section 14 of `docs/vcs-spec.md`.

## Overview

Verum's meta-system enables compile-time computation, code generation, and type introspection. Meta functions execute during compilation, allowing for powerful abstractions while maintaining type safety and determinism.

## Directory Structure

```
meta/
├── tagged_literals/        # Tagged literal definitions and usage
│   ├── literal_definition.vr       # Defining tagged literals
│   ├── literal_usage.vr            # Using tagged literals in code
│   ├── compile_time_parsing.vr     # Compile-time format validation
│   └── literal_errors.vr           # Error detection for invalid literals
│
├── derive/                 # Automatic derive macros
│   ├── derive_debug.vr             # @derive(Debug)
│   ├── derive_clone.vr             # @derive(Clone)
│   ├── derive_eq.vr                # @derive(Eq, PartialEq)
│   ├── custom_derive.vr            # User-defined derive macros
│   ├── custom_derive_fail.vr       # Derive macro failure cases
│   ├── derive_constraints.vr       # Trait bounds for derive
│   └── derive_constraints_fail.vr  # Constraint violation errors
│
├── compile_time/           # Compile-time computations
│   ├── meta_functions.vr           # Meta function definitions
│   ├── meta_functions_fail.vr      # Meta function errors
│   ├── const_evaluation.vr         # Const expression evaluation
│   ├── const_evaluation_fail.vr    # Const evaluation errors
│   ├── type_introspection.vr       # Type reflection API
│   ├── type_introspection_fail.vr  # Introspection limitations
│   ├── code_generation.vr          # Quote/unquote mechanisms
│   └── code_generation_fail.vr     # Code generation errors
│
└── sandbox/                # Meta sandbox restrictions
    ├── allowed_operations.vr       # Permitted operations
    ├── io_forbidden.vr             # IO restriction tests
    ├── determinism.vr              # Determinism requirements
    └── sandbox_violations.vr       # Various violation tests
```

## Test Categories

### 1. Tagged Literals (`tagged_literals/`)

Tagged literals provide compile-time validated, type-safe literals for common data types.

**Key concepts tested:**
- Defining tagged literals with `@tagged_literal`
- Using literals: `d#"2025-01-01"`, `uuid#"..."`, `regex#"..."`
- Compile-time format validation
- Error messages for invalid formats

**Example:**
```verum
@tagged_literal("d")
meta fn date_literal(s: &str) -> Date {
    Date.parse(s).expect("Invalid date format")
}

// Usage
let birthday = d#"2025-12-31";  // Validated at compile time
```

### 2. Derive Macros (`derive/`)

Automatic implementation generation for common traits.

**Key concepts tested:**
- Standard derives: Debug, Clone, Eq, Ord, Hash
- Custom derive macro definition
- Constraint propagation for generics
- Field attribute processing

**Example:**
```verum
@derive(Clone, Debug, Eq)
type Person is {
    name: Text,
    age: Int,
};

@derive(Serialize, Deserialize)  // Custom derives
type ApiResponse<T: Serialize> is {
    data: T,
    status: Int,
};
```

### 3. Compile-Time Computations (`compile_time/`)

Meta functions enable arbitrary compile-time computation.

**Key concepts tested:**
- Meta function definition and invocation
- Const evaluation of expressions
- Type introspection and reflection
- Code generation via quote/unquote

**Example:**
```verum
meta fn factorial(n: u64) -> u64 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}

const FACT_10: u64 = factorial(10);  // 3628800

meta fn generate_getter(field: &str, ty: &str) -> TokenStream {
    quote! {
        fn #(field)(&self) -> &#(ty) {
            &self.#(field)
        }
    }
}
```

### 4. Sandbox Restrictions (`sandbox/`)

Meta functions execute in a restricted sandbox for safety and determinism.

**Key concepts tested:**
- Allowed pure computations
- Forbidden IO operations
- Determinism requirements
- Resource limits

**Sandbox Rules:**
1. No file system access
2. No network operations
3. No random number generation
4. No system time access
5. No global mutable state
6. No unsafe code
7. Memory and time limits

**Example:**
```verum
// ALLOWED: Pure computation
meta fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

// FORBIDDEN: IO operation
meta fn read_config() -> Text {
    std.fs.read_to_string("/etc/config")  // ERROR
}
```

## Test Annotations

All tests use the following VCS annotations:

```verum
// @test: compile-only | run | compile-fail
// @tier: 3
// @level: L3
// @tags: meta, <category>, [error]
// @expected-error: "<message>"     // For compile-fail tests
// @expected-stdout: "<output>"     // For run tests
```

### Test Types

- `compile-only`: Code should compile but not run
- `run`: Code should compile and execute
- `compile-fail`: Code should be rejected by the compiler

## Running Tests

```bash
# Run all meta-system tests
vtest vcs/specs/L3-extended/meta/

# Run specific category
vtest vcs/specs/L3-extended/meta/tagged_literals/

# Run only failure tests
vtest --filter "*_fail.vr" vcs/specs/L3-extended/meta/

# Run with verbose output
vtest -v vcs/specs/L3-extended/meta/
```

## Implementation Notes

### Meta Function Restrictions

Meta functions must be:
1. **Pure**: No side effects
2. **Deterministic**: Same inputs produce same outputs
3. **Total**: Must terminate (enforced via resource limits)
4. **Safe**: No unsafe code or raw pointer access

### Quote Syntax

The `quote!` macro creates TokenStream values:
```verum
quote! {
    fn #(ident)() -> #(return_type) {
        #(body)
    }
}
```

- `#(expr)`: Interpolate expression
- `#(expr)*`: Repeat for each item in iterator

### Type Introspection API

```verum
type TypeInfo is {
    name: Text,
    module: Text,
    kind: TypeKind,
    size: usize,
    align: usize,
    attributes: List<Attribute>,
};

// Access via
meta fn fields<T>() -> List<FieldInfo> {
    T::type_info().fields()
}
```

### Compile-Time Assertions

```verum
const _: () = {
    assert!(size_of::<Pointer>() == 8);
    assert!(factorial(5) == 120);
};
```

## Error Codes

- `E600`: Meta sandbox violation
- `E601`: Non-deterministic operation in meta context
- `E602`: Resource limit exceeded
- `E603`: Invalid quote syntax
- `E604`: Type introspection error
- `E605`: Tagged literal parse error

## Related Documentation

- `docs/vcs-spec.md` Section 14: Dependent Types and Meta-system
- `docs/detailed/10-meta-programming.md`: Full meta-programming spec
- `docs/detailed/11-compile-time.md`: Compile-time evaluation
