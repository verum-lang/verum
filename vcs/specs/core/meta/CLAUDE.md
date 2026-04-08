# core/meta Test Suite

Test coverage for Verum's metaprogramming module.

## Test Organization

| File | Module | Coverage | Tests |
|------|--------|----------|-------|
| `staged_meta_test.vr` | `meta` | quote, stage escapes, lift, N-level staging | ~15 |
| `lift_test.vr` | `meta` | Lift operator, compile-time evaluation | ~12 |
| `hygiene_test.vr` | `meta` | Hygienic macro expansion | ~8 |
| `context_groups_test.vr` | `meta` | Context groups for macros | ~8 |
| `meta_runtime_test.vr` | `meta` | Runtime reflection | ~8 |
| `project_info_test.vr` | `meta` | Project metadata access | ~11 |
| `meta_extended_test.vr` | `meta` | TypeKind extended variants, FieldInfo/VariantInfo/GenericParam, PrimitiveType classification, Visibility, OwnershipInfo, FieldOffset, MethodSource, SelfKind, Span, TokenStream/TokenKind, Delimiter, Attribute types, AttributeValue, TraitBound, LifetimeParam, ProtocolInfo, ParamInfo, AssociatedTypeInfo, MethodResolution | 88 |

## Key Concepts Tested

### Staged Metaprogramming
Multi-stage computation for compile-time code generation.

**Quote Expressions:**
```verum
// Basic quote - creates AST at compile time
let code = quote { 1 + 2 };

// N-level staging
let stage2 = quote(2) { ... };
```

**Stage Escapes:**
```verum
// Escape to previous stage
let x = 5;
let code = quote { $(x) + 1 };  // x evaluated at stage 0
```

**Lift Operator:**
```verum
// Lift value to next stage (syntactic sugar for stage escape)
let x = 5;
let code = quote { lift(x) + 1 };  // Equivalent to $(x)
```

### Hygiene
Macro hygiene ensures identifiers don't accidentally capture or clash.

**Tested Features:**
- Identifier scoping in macro expansions
- Binding preservation
- Name collision prevention
- Gensym for unique identifiers

### Context Groups
Grouping of related contexts for macro expansion.

**Operations:**
- Context group creation
- Context merging
- Scope management

### Runtime Reflection
Limited runtime type information.

**Features:**
- Type name access
- Field enumeration
- Method introspection

### Project Info
Access to project metadata at compile time.

**Available Info:**
- `@project_name` - Current project name
- `@project_version` - Project version
- `@package_name` - Current package
- `@module_name` - Current module
- `@file_path` - Current source file
- `@line_number` - Current line
- `@column_number` - Current column

## Tests by Category

### Staged Meta Tests (~15 tests)
- Basic quote expressions
- Quote with interpolation
- Stage escape (`${}`)
- Lift operator
- Lift vs stage escape equivalence
- N-level staging (1, 2 levels)
- Nested quotes
- Staged computation patterns

### Lift Tests (~12 tests)
- Basic lift semantics
- Lift of literals
- Lift of expressions
- Lift in quote context
- Multi-level lift
- Lift type preservation

### Hygiene Tests (~8 tests)
- Identifier hygiene
- Macro expansion scoping
- Capture avoidance
- Gensym usage
- Cross-stage hygiene

### Context Groups Tests (~8 tests)
- Context group creation
- Context merging
- Scope propagation
- Group inheritance

### Meta Runtime Tests (~8 tests)
- Type name reflection
- Field enumeration
- Method access
- Runtime type checks

### Project Info Tests (~11 tests)
- Project name access
- Version info
- Package/module names
- File/line/column info
- Build configuration

## Meta Types

### TokenStream
Sequence of tokens for macro processing.

**Construction:**
- `TokenStream.new()` - Empty stream
- `TokenStream.from(str)` - Parse from string

**Operations:**
- `append(token)` - Add token
- `extend(stream)` - Append stream
- `iter()` - Iterate tokens

### Quote
Quoted code fragment (AST).

**Creation:**
- `quote { ... }` - Quote expression
- `quote(N) { ... }` - N-level quote

**Methods:**
- `splice()` - Insert into code
- `eval()` - Evaluate at compile time

### Span
Source location information.

**Fields:**
- `start` - Start position
- `end` - End position
- `file` - Source file

**Methods:**
- `join(other)` - Combine spans

### StageInfo
Information about current compilation stage.

**Fields:**
- `level` - Current stage level
- `context` - Stage context

## Known Limitations

- Full macro hygiene only for simple cases
- Multi-stage beyond level 2 not fully tested
- Complex quote interpolation edge cases
- Runtime reflection limited (compile-time focus)
- Platform-specific @cfg not tested

## Test Count: 257 tests total (14 test files, 14 passing + 0 skipped)

## Architecture Notes

### Staged Compilation
```
Stage 0: Compile-time computation
Stage 1: Code generation (quote)
Stage 2: Two-level metaprogramming
...
Stage N: N-level staging
```

### Quote/Unquote Pattern
```verum
// Generate code that adds values
fn make_adder(a: Int, b: Int) -> Quote {
    quote { $(a) + $(b) }
}

// At compile time: make_adder(1, 2) -> quote { 1 + 2 }
```

### Macro Expansion Order
1. Parse macro invocation
2. Expand macro body
3. Apply hygiene
4. Splice into AST
5. Continue compilation

### Performance Targets
- Quote creation: O(size of quoted code)
- Stage escape: O(1) lookup
- Macro expansion: O(expansion size)
- All meta operations: zero runtime cost
