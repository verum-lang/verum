# Type Inference Tests (L1-core)

This directory contains comprehensive tests for Verum's type inference system.
Each test file demonstrates specific inference capabilities and edge cases.

## Test Files

| File | Description | Test Type |
|------|-------------|-----------|
| `literal_inference.vr` | Inference from literal values (42 -> Int, 3.14 -> Float, etc.) | typecheck-pass |
| `context_inference.vr` | Push-down inference from usage context and annotations | typecheck-pass |
| `bidirectional.vr` | Bidirectional type inference (synthesis + checking) | typecheck-pass |
| `generic_inference.vr` | Inferring generic type parameters from arguments | typecheck-pass |
| `closure_inference.vr` | Closure parameter and return type inference | typecheck-pass |
| `collection_inference.vr` | Collection literal inference (list!, map!, set!) | typecheck-pass |
| `return_inference.vr` | Function return type inference from body | typecheck-pass |
| `pattern_inference.vr` | Inference through pattern matching and destructuring | typecheck-pass |
| `constraint_solving.vr` | Unification and constraint solving mechanisms | typecheck-pass |
| `recursive_inference.vr` | Inference in recursive functions | typecheck-pass |
| `mutual_inference.vr` | Inference across mutually recursive functions | typecheck-pass |
| `partial_annotation.vr` | Mixing explicit types with inference (underscore `_`) | typecheck-pass |
| `ambiguous_types.vr` | Cases where inference fails and requires annotations | typecheck-fail |

## Inference Strategies

Verum uses several inference strategies:

### 1. Literal Inference
```verum
let x = 42;        // Int
let y = 3.14;      // Float
let s = "hello";   // Text
```

### 2. Context Inference (Push-Down)
```verum
let list: List<Int> = list![1, 2, 3];  // Elements inferred as Int
let result: Text = value.to_string();  // Return type guides inference
```

### 3. Bidirectional Inference
```verum
let doubled: Int = transform(21, |x| x * 2);
// Int flows down to return type
// 21 flows up to determine T = Int
// x infers Int from T
```

### 4. Constraint Solving
```verum
fn id<T>(x: T) -> T { x }
let y = id(42);  // Constraint: T = typeof(42) = Int
```

## Error Codes

- `E404`: Ambiguous type - inference cannot determine a unique type

## Running Tests

```bash
vtest run specs/L1-core/inference/
vtest run specs/L1-core/inference/literal_inference.vr
vtest run --tag inference
```

## Related Documentation

- Type System: `docs/detailed/03-type-system.md`
- VCS Specification: `docs/vcs-spec.md` Section 7
