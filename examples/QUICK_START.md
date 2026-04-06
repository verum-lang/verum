# Verum Stdlib Examples - Quick Start Guide

## 30-Second Start

```bash
cd /Users/taaliman/projects/luxquant/axiom/examples/stdlib_basic
verum run main.vr
```

## 5-Minute Learning Path

### Step 1: Basic Collections (2 min)
```bash
cd stdlib_basic
cat main.vr          # Review the code
verum run main.vr    # Run the example
```

**Learn**: List, Map, Maybe, Result basics

### Step 2: Advanced Collections (2 min)
```bash
cd ../stdlib_collections
verum run main.vr
```

**Learn**: Set operations, nesting, complex transformations

### Step 3: I/O Operations (1 min)
```bash
cd ../stdlib_io
verum run main.vr
```

**Learn**: File I/O, context system

### Step 4: Memory Safety (optional)
```bash
cd ../stdlib_cbgr
verum run main.vr
```

**Learn**: CBGR, references, performance

## Cheat Sheet

### Collections
```verum
let list = [1, 2, 3]
let map = Map.new<Text, Int>()
let set = Set.from_list([1, 2, 3])
```

### Pattern Matching
```verum
match value {
    Maybe.Some(x) => use_value(x),
    Maybe.None => handle_none(),
}
```

### Error Handling
```verum
match read_file(path) {
    Result.Ok(data) => process(data),
    Result.Err(msg) => log_error(msg),
}
```

### Context System
```verum
fn main() using [FileSystem] {
    write_file("test.txt", "Hello")
}
```

### References
```verum
let ref = &data           // Tier 0: ~15ns
let ref = &checked data   // Tier 1: 0ns
let ref = &unsafe data    // Tier 2: 0ns
```

## File Guide

| File | Purpose | Lines | Read Time |
|------|---------|-------|-----------|
| `README_STDLIB_EXAMPLES.md` | Master guide | 212 | 5 min |
| `STDLIB_INDEX.md` | Quick reference | 193 | 3 min |
| `stdlib_basic/main.vr` | Basic code | 155 | 5 min |
| `stdlib_collections/main.vr` | Advanced code | 178 | 7 min |
| `stdlib_io/main.vr` | I/O code | 152 | 6 min |
| `stdlib_cbgr/main.vr` | CBGR code | 234 | 10 min |

## Common Tasks

### Run All Examples
```bash
for dir in stdlib_basic stdlib_collections stdlib_io stdlib_cbgr; do
    cd $dir && verum run main.vr && cd ..
done
```

### Build Executables
```bash
verum build stdlib_basic/main.vr -o basic
verum build stdlib_collections/main.vr -o collections
verum build stdlib_io/main.vr -o io
verum build stdlib_cbgr/main.vr -o cbgr
```

### View Documentation
```bash
cat README_STDLIB_EXAMPLES.md      # Full guide
cat STDLIB_INDEX.md                # Quick reference
cat stdlib_basic/README.md         # Basic examples doc
```

## Key Concepts (1 min read)

1. **Semantic Types**: `List`, `Text`, `Map` (not `Vec`, `String`, `HashMap`)
2. **No Null**: Use `Maybe<T>` for optional values
3. **No Exceptions**: Use `Result<T, E>` for errors
4. **Explicit I/O**: Context system with `using [FileSystem]`
5. **Memory Safety**: CBGR with three-tier references

## Next Steps

1. Run `stdlib_basic` to learn fundamentals
2. Experiment with modifying the code
3. Read the detailed READMEs for deeper understanding
4. Build your own project using these patterns

## Help

- Full documentation: `README_STDLIB_EXAMPLES.md`
- Technical details: `STDLIB_EXAMPLES_SUMMARY.md` (in project root)
- Language syntax: `/docs/detailed/05-syntax-grammar.md`
- Type system: `/docs/detailed/03-type-system.md`
