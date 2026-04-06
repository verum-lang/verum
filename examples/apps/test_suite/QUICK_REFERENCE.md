# Verum Refinement Types - Quick Reference

## What Works ✅

### 1. Inline Refinements
```verum
type Positive is Int{it > 0};
type Percent is Int{it >= 0 && it <= 100};
type NonEmpty is Text{it.len() > 0};
```

### 2. Declarative Refinements
```verum
fn is_positive(n: Int) -> Bool { n > 0 }
type PositiveInt is Int where is_positive;
```

### 3. Sigma-Type Refinements
```verum
type PositiveSigma is x: Int where x > 0;
type ValidRange is n: Int where n >= 0 && n <= 100;
```

### 4. Function Postconditions (Basic)
```verum
fn abs(x: Int) -> Int where ensures result >= 0 {
    if x < 0 { 0 - x } else { x }
}
```

### 5. Basic Generics
```verum
fn identity<T>(x: T) -> T { x }
```

## Known Issues ⚠️

### Tuple Indexing in Sigma-Types
```verum
// FAILS - tuple indexing not supported in predicates
type ValidPair is p: (Int, Int) where p.0 > 0 && p.1 > 0;
```

### Multiple Where Clauses
```verum
// FAILS - multiple where clauses not parsed
fn foo(x: Int) -> Int
    where requires x > 0
    where ensures result > x { ... }
```

## Not Yet Implemented ❌

### Protocol Bounds
```verum
// NOT IMPLEMENTED - protocol system not in interpreter
fn print_value<T>(value: T) where T: Display { ... }
```

### Meta Parameters
```verum
// NOT IMPLEMENTED - compile-time feature
fn fixed_array<N: meta Int>(value: Int) -> [Int; N]
    where meta N > 0 { ... }
```

## Run Tests

```bash
# Working tests
cargo run -p verum_cli -- run examples/apps/test_suite/test_refinement_inline.vr
cargo run -p verum_cli -- run examples/apps/test_suite/test_refinement_declarative.vr
cargo run -p verum_cli -- run examples/apps/test_suite/test_sigma_minimal.vr
cargo run -p verum_cli -- run examples/apps/test_suite/test_postcond_minimal.vr
cargo run -p verum_cli -- run examples/apps/test_suite/test_where_minimal.vr

# Full report
cat examples/apps/test_suite/REFINEMENT_TEST_REPORT.md
```

## Quick Start

1. **Use inline refinements** for simple constraints:
   ```verum
   type Age is Int{it >= 0 && it <= 150};
   ```

2. **Use declarative refinements** for reusable validation:
   ```verum
   fn is_email(s: Text) -> Bool { s.contains("@") }
   type Email is Text where is_email;
   ```

3. **Use sigma-types** for explicit dependencies:
   ```verum
   type NonNegative is x: Int where x >= 0;
   ```

4. **Add postconditions** to functions:
   ```verum
   fn square(x: Int) -> Int where ensures result >= 0 {
       x * x
   }
   ```

## Tips

- Start with minimal examples
- Test parser acceptance before full implementation
- Avoid tuple indexing in predicates for now
- Use single where clauses until multiple clause support is added
- Stick to concrete types or unbound generics in interpreter
