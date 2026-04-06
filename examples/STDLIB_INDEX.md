# Verum Standard Library Examples - Quick Index

## Quick Start

```bash
# Start with the basics
cd examples/stdlib_basic
verum run main.vr

# Progress through intermediate topics
cd examples/stdlib_collections
verum run main.vr

cd examples/stdlib_io
verum run main.vr

# Master advanced concepts
cd examples/stdlib_cbgr
verum run main.vr
```

## Example Overview

| Project | Level | LOC | Topics | Run Time |
|---------|-------|-----|--------|----------|
| **stdlib_basic** | Beginner | 155 | List, Map, Maybe, Result | ~100ms |
| **stdlib_collections** | Intermediate | 178 | Set, nesting, transforms | ~150ms |
| **stdlib_io** | Intermediate | 152 | File I/O, contexts | ~200ms |
| **stdlib_cbgr** | Advanced | 234 | Memory safety, CBGR | ~300ms |

## By Topic

### Collections
- **Lists**: `stdlib_basic/main.vr` lines 26-66
- **Maps**: `stdlib_basic/main.vr` lines 68-98
- **Sets**: `stdlib_collections/main.vr` lines 10-46
- **Nested**: `stdlib_collections/main.vr` lines 48-104

### Type Safety
- **Maybe**: `stdlib_basic/main.vr` lines 100-126
- **Result**: `stdlib_basic/main.vr` lines 128-155
- **Pattern Matching**: All examples

### I/O Operations
- **Console**: `stdlib_io/main.vr` lines 10-28
- **Files**: `stdlib_io/main.vr` lines 30-76
- **Directories**: `stdlib_io/main.vr` lines 149-171
- **Error Handling**: `stdlib_io/main.vr` lines 78-104

### Memory Safety
- **References**: `stdlib_cbgr/main.vr` lines 11-37
- **Generations**: `stdlib_cbgr/main.vr` lines 39-63
- **Three Tiers**: `stdlib_cbgr/main.vr` lines 65-86
- **Safety**: `stdlib_cbgr/main.vr` lines 108-157

### Advanced Patterns
- **Functional Transforms**: `stdlib_collections/main.vr` lines 106-158
- **Graph Structures**: `stdlib_collections/main.vr` lines 160-178
- **Context System**: `stdlib_io/main.vr` lines 106-171
- **Performance**: `stdlib_cbgr/main.vr` lines 181-234

## Code Snippets

### Basic List Operations
```verum
// From stdlib_basic/main.vr:30-43
let numbers = [1, 2, 3, 4, 5]
let sum = numbers.fold(0, |acc, x| acc + x)
let squared = numbers.map(|x| x * x)
let evens = numbers.filter(|x| x % 2 == 0)
```

### Safe Error Handling
```verum
// From stdlib_basic/main.vr:148-155
fn safe_divide(a: Int, b: Int) -> Result<Int, Text> {
    if b == 0 {
        Result.Err("division by zero")
    } else {
        Result.Ok(a / b)
    }
}
```

### Context System
```verum
// From stdlib_io/main.vr:106-109
fn write_file(path: Text, content: Text) -> Result<(), Text> using [FileSystem] {
    FileSystem.write(path, content)
}
```

### CBGR References
```verum
// From stdlib_cbgr/main.vr:65-75
let tier0_ref = &data              // ~15ns overhead
let tier1_ref = &checked data      // 0ns compiler-proven
let tier2_ref = &unsafe data       // 0ns manual proof
```

## Learning Objectives

### After stdlib_basic
- ✓ Create and manipulate Lists and Maps
- ✓ Handle optional values with Maybe
- ✓ Implement error handling with Result
- ✓ Use functional operations (map, filter, fold)

### After stdlib_collections
- ✓ Work with Sets and set operations
- ✓ Create nested data structures
- ✓ Implement complex transformations
- ✓ Build graphs and priority queues

### After stdlib_io
- ✓ Perform file I/O operations
- ✓ Use the context system
- ✓ Handle I/O errors explicitly
- ✓ Parse structured text (CSV, etc.)

### After stdlib_cbgr
- ✓ Understand CBGR memory safety
- ✓ Choose appropriate reference tiers
- ✓ Optimize performance-critical code
- ✓ Reason about memory safety guarantees

## Reference Documents

### Language Specs
- `/Users/taaliman/projects/luxquant/axiom/docs/detailed/05-syntax-grammar.md` - Complete syntax
- `/Users/taaliman/projects/luxquant/axiom/docs/detailed/03-type-system.md` - Type system
- `/Users/taaliman/projects/luxquant/axiom/docs/detailed/16-context-system.md` - Context system
- `/Users/taaliman/projects/luxquant/axiom/docs/detailed/26-cbgr-implementation.md` - CBGR spec

### Project Docs
- `/Users/taaliman/projects/luxquant/axiom/examples/README_STDLIB_EXAMPLES.md` - Master guide
- `/Users/taaliman/projects/luxquant/axiom/STDLIB_EXAMPLES_SUMMARY.md` - Technical summary
- `/Users/taaliman/projects/luxquant/axiom/CLAUDE.md` - Project guidelines

### Implementation
- `/Users/taaliman/projects/luxquant/axiom/crates/verum_std/` - Standard library
- `/Users/taaliman/projects/luxquant/axiom/crates/verum_cbgr/` - CBGR runtime
- `/Users/taaliman/projects/luxquant/axiom/crates/verum_context/` - Context system

## Testing

```bash
# Test all examples
for dir in stdlib_basic stdlib_collections stdlib_io stdlib_cbgr; do
    cd examples/$dir
    verum run main.vr
    cd ../..
done

# Run with timing
time verum run examples/stdlib_basic/main.vr
time verum run examples/stdlib_collections/main.vr
time verum run examples/stdlib_io/main.vr
time verum run examples/stdlib_cbgr/main.vr
```

## Common Patterns

### Safe Collection Access
```verum
let item = list.get(index)
match item {
    Maybe.Some(val) => process(val),
    Maybe.None => handle_missing(),
}
```

### Error Propagation
```verum
fn process() -> Result<Int, Text> {
    let data = read_file("data.txt")?
    let parsed = parse_int(data)?
    Result.Ok(parsed * 2)
}
```

### Functional Pipeline
```verum
let result = data
    .filter(|x| x > 0)
    .map(|x| x * 2)
    .fold(0, |acc, x| acc + x)
```

### Context Injection
```verum
fn main() using [FileSystem, Console] {
    let content = read_file("input.txt")
    print(f"Content: {content}")
}
```

## Performance Guide

| Operation | Tier 0 | Tier 1 | Tier 2 |
|-----------|--------|--------|--------|
| CBGR Reference | ~15ns | 0ns | 0ns |
| List Access | O(1) | O(1) | O(1) |
| Map Lookup | O(1) avg | O(1) avg | O(1) avg |
| Set Membership | O(1) avg | O(1) avg | O(1) avg |

## Troubleshooting

### Common Issues

**Issue**: "FileSystem context not provided"
```verum
// Add context to function signature
fn main() using [FileSystem] { ... }
```

**Issue**: "Generation mismatch"
```verum
// Use appropriate reference tier
let ref = &checked data  // Tier 1
```

**Issue**: "Type mismatch"
```verum
// Explicit type annotation
let list: List<Int> = [1, 2, 3]
```

## Next Steps

1. Complete all four example projects in order
2. Experiment with modifications to the code
3. Build your own projects using these patterns
4. Explore advanced topics (async, verification, GPU)
5. Contribute improvements to the examples

## Contact & Support

- Documentation: `/docs/` directory
- Examples: `/examples/` directory
- Source: `/crates/` directory
- Issues: Project issue tracker

---

**Version**: 1.0
**Last Updated**: 2025-12-23
**Status**: Production Ready
