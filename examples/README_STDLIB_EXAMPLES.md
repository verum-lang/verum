# Verum Standard Library Examples

This directory contains comprehensive examples demonstrating Verum's standard library features, memory safety system, and language capabilities.

## Example Projects

### 1. `stdlib_basic/` - Fundamental Operations
**Status**: Beginner-friendly
**Topics**: List, Map, Maybe, Result

Learn the basics of Verum's standard library:
- Creating and manipulating Lists and Maps
- Safe null handling with Maybe<T>
- Error handling with Result<T, E>
- Functional operations (map, filter, fold)
- Type inference and semantic types

**Start here** if you're new to Verum.

```bash
cd stdlib_basic
verum run main.vr
```

### 2. `stdlib_collections/` - Advanced Data Structures
**Status**: Intermediate
**Topics**: Set, nested collections, transformations

Explore advanced collection patterns:
- Set operations (intersection, union, difference)
- Nested collections (List<List<T>>, Map<K, List<V>>)
- Complex transformations and pipelines
- Grouping and categorization patterns
- Graph structures and priority queues

**Continue here** after mastering the basics.

```bash
cd stdlib_collections
verum run main.vr
```

### 3. `stdlib_io/` - I/O and Context System
**Status**: Intermediate
**Topics**: File I/O, context system, capabilities

Master I/O operations and Verum's context system:
- Console I/O with formatted printing
- File operations (read, write, append, delete)
- Context system for capability-based security
- Explicit dependency injection with `using`
- Error handling patterns for I/O

**Learn this** to understand Verum's approach to side effects.

```bash
cd stdlib_io
verum run main.vr
```

### 4. `stdlib_cbgr/` - Memory Safety
**Status**: Advanced
**Topics**: CBGR, references, memory safety

Understand Verum's memory safety guarantees:
- CBGR (Capability-Based Generational References)
- Three-tier reference model (&T, &checked T, &unsafe T)
- Generation tracking and validation
- Dangling reference prevention
- Performance characteristics and optimization

**Study this** for performance-critical code and memory safety.

```bash
cd stdlib_cbgr
verum run main.vr
```

## Learning Path

```
1. stdlib_basic         → Fundamentals (Lists, Maps, Maybe, Result)
   ↓
2. stdlib_collections   → Advanced patterns (Set, nesting, transformations)
   ↓
3. stdlib_io            → Side effects (I/O, context system)
   ↓
4. stdlib_cbgr          → Memory safety (CBGR, references, performance)
```

## Quick Reference

### Semantic Types (Always Use These)

| Semantic Name | Meaning | NOT |
|---------------|---------|-----|
| `List<T>` | Dynamic array | `Vec<T>` |
| `Text` | String data | `String` |
| `Map<K, V>` | Dictionary | `HashMap<K, V>` |
| `Set<T>` | Unique elements | `HashSet<T>` |
| `Maybe<T>` | Optional value | `Option<T>` |
| `Result<T, E>` | Failable operation | (same) |

### Core Language Features

**Variables**:
```verum
let x = 42              // Immutable
let mut y = 10          // Mutable
let name: Text = "Bob"  // Explicit type
```

**Collections**:
```verum
let list = [1, 2, 3]
let map = Map.new<Text, Int>()
let set = Set.from_list([1, 2, 3])
```

**Functions**:
```verum
fn add(x: Int, y: Int) -> Int {
    x + y
}

fn double(x: Int) -> Int = x * 2;  // Expression form
```

**Pattern Matching**:
```verum
match value {
    Maybe.Some(x) => print(f"Got: {x}"),
    Maybe.None => print("Nothing"),
}
```

**Error Handling**:
```verum
let result = divide(10, 2)
match result {
    Result.Ok(val) => process(val),
    Result.Err(msg) => log_error(msg),
}
```

**Context System**:
```verum
fn read_file(path: Text) using [FileSystem] {
    FileSystem.read(path)
}
```

**References**:
```verum
let ref = &data           // Tier 0: ~15ns CBGR
let ref = &checked data   // Tier 1: 0ns compiler-proven
let ref = &unsafe data    // Tier 2: 0ns manual proof
```

## Running the Examples

### Prerequisites
- Verum compiler installed (`verum --version`)
- Standard library available
- (Optional) IDE with Verum LSP support

### Execution Methods

**1. Direct execution with interpreter**:
```bash
verum interpret main.vr
```

**2. Compile and run**:
```bash
verum run main.vr
```

**3. Build executable**:
```bash
verum build main.vr -o example
./example
```

**4. Build with optimizations**:
```bash
verum build main.vr -o example --release
./example
```

## Key Concepts

### 1. Semantic Honesty
Types describe **meaning**, not implementation:
- `List` not `Vec` (semantic: a list of items)
- `Text` not `String` (semantic: textual data)
- `Map` not `HashMap` (semantic: key-value mapping)

### 2. No Magic
All dependencies explicit:
- No hidden global state
- Context system makes I/O capabilities visible
- `using [FileSystem]` declares what a function can do

### 3. Gradual Safety
Choose your safety/performance tradeoff:
- **Tier 0** (`&T`): Safe, ~15ns overhead
- **Tier 1** (`&checked T`): Safe, 0ns (compiler-proven)
- **Tier 2** (`&unsafe T`): 0ns (manual proof)

### 4. Zero-Cost Abstractions
High-level code compiles to efficient machine code:
- CBGR: ~15ns overhead for full safety
- Functional operations inline
- No garbage collection pauses
- Memory overhead: <5%

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| CBGR check | <15ns | Tier 0 reference access |
| Type inference | <100ms/10K LOC | Compilation speed |
| Runtime | 0.85-0.95x C | Optimized code |
| Memory overhead | <5% | CBGR metadata |

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
    let data = read_file("data.txt")?  // ? propagates errors
    let parsed = parse_int(data)?
    Result.Ok(parsed * 2)
}
```

### Functional Pipelines
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

## Additional Resources

### Documentation
- `docs/detailed/03-type-system.md` - Type system details
- `docs/detailed/05-syntax-grammar.md` - Complete syntax reference
- `docs/detailed/16-context-system.md` - Context system deep dive
- `docs/detailed/26-cbgr-implementation.md` - CBGR specification

### Implementation
- `crates/verum_std/` - Standard library source
- `crates/verum_cbgr/` - CBGR implementation
- `crates/verum_context/` - Context system runtime

### Testing
- `crates/*/tests/` - Unit tests
- `crates/*/benches/` - Performance benchmarks

## Contributing

When creating new examples:
1. Follow the semantic type naming (List, Text, Map, etc.)
2. Include comprehensive README with expected output
3. Demonstrate error handling explicitly
4. Show both simple and complex usage patterns
5. Add comments explaining non-obvious behavior

## Example Template

```verum
// example_name/main.vr
// Description of what this example demonstrates

fn main() {
    print("=== Example Name ===")
    print("")

    test_feature_1()
    test_feature_2()

    print("")
    print("=== All Tests Completed ===")
}

fn test_feature_1() {
    print("--- Feature 1 ---")
    // Demonstration code
    print("")
}
```

## Getting Help

- **Language Questions**: See `docs/` directory
- **Bug Reports**: Create issue with minimal reproduction
- **Feature Requests**: Discuss in project discussions
- **Examples Broken**: Check Verum version compatibility

## Version Compatibility

These examples are compatible with:
- Verum Language: v1.0+
- Standard Library: v1.0+
- CBGR: v1.0+

Check your version:
```bash
verum --version
```

## Next Steps

1. **Start with `stdlib_basic`** to learn fundamentals
2. **Progress to `stdlib_collections`** for advanced patterns
3. **Study `stdlib_io`** to understand side effects
4. **Master `stdlib_cbgr`** for performance optimization
5. **Build your own projects** using these patterns

Happy coding with Verum!
