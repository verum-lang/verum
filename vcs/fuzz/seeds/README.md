# VCS Fuzz Testing Seed Corpus

This directory contains seed programs for fuzzing the Verum compiler and runtime. These programs serve as starting points for mutation-based fuzzing and help ensure broad coverage of language features.

## Directory Structure

```
seeds/
├── minimal/           # Minimal valid programs (one feature each)
├── edge_cases/        # Edge case programs (boundary conditions)
├── complex/           # Complex but valid programs (multiple features)
└── README.md          # This file
```

## Seed Categories

### Minimal Seeds (`minimal/`)

Minimal programs that test individual language features in isolation. Each file contains the simplest valid program demonstrating a single feature.

| File | Feature | Purpose |
|------|---------|---------|
| `empty_main.vr` | Empty program | Parser baseline |
| `single_let.vr` | Variable binding | Let binding syntax |
| `simple_function.vr` | Function definition | Function declaration and calls |
| `simple_if.vr` | Conditionals | If-else expressions |
| `simple_loop.vr` | Loops | For loop iteration |
| `simple_struct.vr` | Structures | Struct definition and instantiation |
| `simple_match.vr` | Pattern matching | Match expressions |
| `simple_list.vr` | Collections | List creation and indexing |
| `simple_ref.vr` | References | CBGR Tier 0 references |
| `simple_closure.vr` | Closures | Lambda expressions |

**Usage**: These seeds are ideal for quick fuzzing runs and regression testing. They help identify issues with fundamental language features.

### Edge Case Seeds (`edge_cases/`)

Programs testing boundary conditions and edge cases that are likely to trigger bugs.

| File | Focus | Tests |
|------|-------|-------|
| `boundary_values.vr` | Numeric limits | Integer overflow, float precision |
| `deep_nesting.vr` | Stack depth | Deeply nested expressions/blocks |
| `many_parameters.vr` | Parameter limits | Functions with many parameters |
| `recursion_patterns.vr` | Recursion | Direct and mutual recursion |
| `tricky_syntax.vr` | Parser edge cases | Ambiguous syntax patterns |
| `type_system_stress.vr` | Type inference | Complex type scenarios |
| `unicode_stress.vr` | Unicode handling | Unicode identifiers and strings |
| `cbgr_edge_cases.vr` | CBGR | Three-tier reference edge cases |
| `refinement_edge_cases.vr` | Refinements | Refinement type verification |
| `async_edge_cases.vr` | Async/await | Concurrent programming patterns |
| `generic_edge_cases.vr` | Generics | Generic type constraints |

**Usage**: These seeds are valuable for finding bugs in corner cases. They should be prioritized when testing new compiler features.

### Complex Seeds (`complex/`)

Complete programs using multiple features together, testing feature interactions.

| File | Features | Purpose |
|------|----------|---------|
| `multi_module.vr` | Modules, visibility | Module system integration |
| `protocol_impl.vr` | Protocols, generics | Protocol implementation |
| `concurrent_patterns.vr` | Async, channels, sync | Concurrency primitives |
| `type_system_advanced.vr` | Dependent types, proofs | Advanced type features |

**Usage**: These seeds test feature interactions and are ideal for integration testing. They can reveal bugs that only appear when multiple features are combined.

## Using Seeds

### With the Fuzzer

```rust
use verum_fuzz::{load_seeds, run_campaign, FuzzConfig};

// Load all seeds
let seeds = load_seeds("vcs/fuzz/seeds")?;

// Run fuzzing campaign starting from seeds
let config = FuzzConfig {
    iterations: 100_000,
    corpus_dir: Some("vcs/fuzz/seeds".into()),
    ..Default::default()
};

run_campaign(config, &mut rng)?;
```

### With Mutation Fuzzer

```rust
use verum_fuzz::generators::{Mutator, MutationConfig};

let mutator = Mutator::new(MutationConfig::default());

for seed in load_seeds("vcs/fuzz/seeds/minimal")? {
    for _ in 0..100 {
        let mutated = mutator.mutate(&seed, &mut rng);
        test_program(&mutated.mutated);
    }
}
```

### With Property Testing

```rust
use proptest::prelude::*;
use verum_fuzz::generators::{ProgramGenerator, GeneratorConfig};

proptest! {
    #[test]
    fn parser_roundtrip(seed in 0u64..1000) {
        let generator = ProgramGenerator::new(GeneratorConfig::default());
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let program = generator.generate(&mut rng);

        let ast = parse(&program.source)?;
        let reparsed = parse(&format(&ast))?;
        prop_assert_eq!(ast, reparsed);
    }
}
```

## Adding New Seeds

When adding new seed programs:

1. **Choose the right category**:
   - `minimal/` - Single feature, <20 lines
   - `edge_cases/` - Boundary conditions, potential bugs
   - `complex/` - Multiple features, realistic programs

2. **Follow naming conventions**:
   - Use descriptive names: `feature_case.vr`
   - Include a comment header explaining purpose

3. **Ensure validity**:
   - All seeds should be syntactically valid
   - Edge cases can be semantically invalid if testing error handling

4. **Update this README**:
   - Add entry to appropriate table
   - Document what the seed tests

## Seed Coverage Goals

The seed corpus aims to cover:

- [ ] All expression types (literals, binary, unary, etc.)
- [ ] All statement types (let, if, match, loop, etc.)
- [ ] All type constructors (primitives, generics, references)
- [ ] CBGR three-tier model (managed, checked, unsafe)
- [ ] Refinement types and SMT verification
- [ ] Async/await and concurrency
- [ ] Module system and visibility
- [ ] Protocol implementations
- [ ] Meta programming constructs
- [ ] Error handling (try/recover/finally)
- [ ] Pattern matching (exhaustiveness, guards)
- [ ] Dependent types (Pi, Sigma)

## Performance Considerations

- Minimal seeds: Fast execution, high iteration count
- Edge case seeds: Medium execution, targeted testing
- Complex seeds: Slow execution, integration focus

For CI/CD:
```bash
# Quick smoke test
vtest run --seeds=minimal --iterations=1000

# Full edge case testing
vtest run --seeds=edge_cases --iterations=10000

# Overnight integration testing
vtest run --seeds=complex --iterations=100000
```

## Related Documentation

- [VCS Specification Section 19: Fuzz Testing](../../specs/README.md)
- [Generator Documentation](../generators/README.md)
- [Harness Documentation](../harness/README.md)
