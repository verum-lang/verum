# Verum AST

Abstract Syntax Tree definitions for the Verum programming language.

## Overview

The `verum_ast` crate provides the core AST data structures used throughout the Verum compiler pipeline. It defines:

- **Expression nodes**: All expression types including literals, operators, closures, comprehensions
- **Type nodes**: Full Verum type system including refinements, three-tier references, rank-2 functions
- **Declaration nodes**: Functions, types, protocols, implementations, FFI boundaries, proofs
- **Pattern nodes**: Pattern matching for destructuring and match expressions
- **Visitor infrastructure**: Recursive and iterative AST traversal
- **Attribute system**: Complete attribute validation, metadata, and target checking
- **Pretty printing**: Human-readable AST output for debugging

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         PUBLIC API (lib.rs)                             │
│  Module, CompilationUnit, Item, Expr, Type, Pattern, Stmt, Visitor      │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────┐         ┌─────────────────┐         ┌─────────────────┐
│ expr.rs       │         │ decl.rs         │         │ ty.rs           │
│ 2071 lines    │         │ 2573 lines      │         │ 944 lines       │
│               │         │                 │         │                 │
│ - Expr        │         │ - Item/ItemKind │         │ - Type/TypeKind │
│ - ExprKind    │         │ - FunctionDecl  │         │ - Refinements   │
│ - Literal     │         │ - TypeDecl      │         │ - References    │
│ - BinOp/UnOp  │         │ - ProtocolDecl  │         │ - Rank2Function │
│ - Capability  │         │ - ImplDecl      │         │ - SigmaType     │
│ - CapabilitySet│        │ - FFIBoundary   │         │ - GenericParam  │
└───────────────┘         │ - TheoremDecl   │         └─────────────────┘
                          │ - ContextDecl   │
                          └─────────────────┘
        │                           │                           │
        └───────────────────────────┼───────────────────────────┘
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          SUPPORT MODULES                                │
├─────────────────┬──────────────────┬───────────────────┬────────────────┤
│ visitor.rs      │ pattern.rs       │ stmt.rs           │ pretty.rs      │
│ 2379 lines      │ 368 lines        │ 236 lines         │ 3899 lines     │
│                 │                  │                   │                │
│ - Visitor trait │ - Pattern        │ - Stmt/StmtKind   │ - PrettyPrint  │
│ - IterativeVis. │ - PatternKind    │ - Let/Block       │ - Formatting   │
│ - walk_* funcs  │ - Guards         │ - Control flow    │ - Debug output │
│ - WorkItem      │ - Destructuring  │                   │                │
└─────────────────┴──────────────────┴───────────────────┴────────────────┘
                                    │
┌─────────────────────────────────────────────────────────────────────────┐
│                         ATTRIBUTE SYSTEM (attr/*)                       │
├────────────────┬─────────────────┬─────────────────┬────────────────────┤
│ mod.rs (571)   │ target.rs (488) │ metadata.rs     │ conversion.rs     │
│                │                 │ (584)           │ (1057)            │
│ - Attribute    │ - AttrTarget    │ - AttrMetadata  │ - FromAttribute   │
│ - AttrKind     │ - target bits   │ - Stability     │ - extract_*       │
│ - AttrArg      │ - validation    │ - Deprecation   │ - TypedAttribute  │
│ - FromAttr     │                 │                 │                   │
└────────────────┴─────────────────┴─────────────────┴────────────────────┘
```

### Module Summary

| Module | Lines | Responsibility |
|--------|-------|----------------|
| `pretty.rs` | 3899 | Pretty printing for all AST nodes |
| `decl.rs` | 2573 | Declarations (functions, types, protocols, FFI) |
| `visitor.rs` | 2379 | Recursive and iterative AST traversal |
| `expr.rs` | 2071 | Expressions with capability tracking |
| `attr/conversion.rs` | 1057 | Attribute parsing and conversion |
| `ty.rs` | 944 | Type system nodes |
| `attr/metadata.rs` | 584 | Attribute metadata and validation rules |
| `attr/mod.rs` | 571 | Core attribute types |
| `attr/target.rs` | 488 | Attribute target validation |
| `pattern.rs` | 368 | Pattern matching nodes |
| `lib.rs` | 297 | Public API and module coordination |
| `stmt.rs` | 236 | Statement nodes |
| `cfg.rs` | 178 | Conditional compilation predicates |
| `attr/args.rs` | 163 | Attribute argument specifications |

## Key Types

### Expressions

```rust
use verum_ast::{Expr, ExprKind, Literal, BinOp, UnOp};

// All expressions have a kind and span
let expr = Expr::new(
    ExprKind::Binary {
        op: BinOp::Add,
        left: Heap::new(left_expr),
        right: Heap::new(right_expr),
    },
    span,
);

// Expression kinds include:
// - Literal, Path, Binary, Unary
// - Call, MethodCall, Field, Index
// - If, Match, Loop, Block
// - Closure, Async, Await
// - StreamComprehension, ListComprehension
// - Pipeline (|>), NullCoalesce (??)
// - Try (?), TryRecover, Throw
// - Quote, Splice (meta-programming)
```

### Types

```rust
use verum_ast::{Type, TypeKind, RefinementType};

// Primitive types
let int_type = Type::new(TypeKind::Int, span);

// Refinement types: Int{> 0}
let positive = Type::new(
    TypeKind::Refined(RefinementType {
        base: Heap::new(int_type),
        predicates: vec![predicate_expr],
    }),
    span,
);

// Three-tier reference model
// &T - CBGR reference (default, ~15ns overhead)
let cbgr_ref = Type::new(TypeKind::Reference { inner, mutable: false }, span);

// &checked T - Compiler-proven safe (0ns overhead)
let checked_ref = Type::new(TypeKind::CheckedReference { inner, mutable: false }, span);

// &unsafe T - Manual safety proof required (0ns overhead)
let unsafe_ref = Type::new(TypeKind::UnsafeReference { inner, mutable: false }, span);

// Sigma types (dependent pairs): (x: Int, Vec<x>)
let sigma = Type::new(
    TypeKind::Sigma(SigmaType { bindings }),
    span,
);

// Rank-2 function types: fn<R>(...) -> R
let rank2 = Type::new(
    TypeKind::Rank2Function(Rank2Function {
        quantified_params,
        params,
        return_type,
    }),
    span,
);
```

### Declarations

```rust
use verum_ast::{Item, ItemKind, FunctionDecl, TypeDecl, ProtocolDecl};

// Functions with staged meta-programming
let func = FunctionDecl {
    name: ident,
    type_params: vec![],
    params: vec![param],
    return_type: Some(return_ty),
    body: Some(body),
    is_meta: true,        // @meta function
    stage_level: Some(1), // Stage 1 execution
    contexts: vec![],     // using [...] contexts
    ..Default::default()
};

// Type declarations
let type_decl = TypeDecl {
    name: ident,
    type_params: vec![],
    definition: TypeDef::Record { fields },
    // or: TypeDef::Variant { variants }
    // or: TypeDef::Protocol { ... }
    // or: TypeDef::Newtype { inner }
};

// FFI boundaries
let ffi = FFIBoundary {
    abi: "C".into(),
    declarations: vec![extern_fn],
};
```

### Patterns

```rust
use verum_ast::{Pattern, PatternKind};

// Wildcard
let wildcard = Pattern::new(PatternKind::Wildcard, span);

// Destructuring
let tuple = Pattern::new(
    PatternKind::Tuple { elements: patterns },
    span,
);

let record = Pattern::new(
    PatternKind::Record {
        path: type_path,
        fields: field_patterns,
    },
    span,
);

// Or-patterns
let or = Pattern::new(
    PatternKind::Or { patterns: vec![p1, p2] },
    span,
);
```

## Visitor Pattern

The AST supports both recursive and stack-safe iterative traversal:

### Recursive Traversal

```rust
use verum_ast::{Visitor, walk_expr, walk_item};

struct MyVisitor {
    found_literals: Vec<Literal>,
}

impl Visitor for MyVisitor {
    fn visit_expr(&mut self, expr: &Expr) {
        if let ExprKind::Literal(lit) = &expr.kind {
            self.found_literals.push(lit.clone());
        }
        walk_expr(self, expr); // Continue traversal
    }
}
```

### Iterative Traversal

For deep ASTs that could overflow the stack:

```rust
use verum_ast::{IterativeVisitor, traverse_expr_iteratively};

let counter = CountingVisitor::new();
let mut iter_visitor = IterativeVisitor::new(counter);
iter_visitor.traverse_expr(&deep_expr);
let result = iter_visitor.into_inner();

// Or use the convenience function
traverse_expr_iteratively(&expr, |e| {
    // Process expression
});
```

## Attribute System

Complete attribute validation and conversion:

```rust
use verum_ast::attr::{
    Attribute, AttributeTarget, AttributeMetadata,
    FromAttribute, TypedAttribute,
};

// Define a typed attribute
#[derive(Debug)]
struct InlineAttr {
    mode: InlineMode,
}

impl FromAttribute for InlineAttr {
    fn attr_name() -> &'static str { "inline" }

    fn from_attr(attr: &Attribute) -> Result<Self, AttrConversionError> {
        // Parse attribute arguments
    }
}

// Validate attribute targets
let targets = AttributeTarget::FUNCTION | AttributeTarget::METHOD;
if !targets.contains(AttributeTarget::TYPE) {
    // Error: @inline not valid on types
}
```

### Built-in Attributes

| Category | Attributes |
|----------|------------|
| **Optimization** | `@inline`, `@cold`, `@hot`, `@optimize`, `@lto` |
| **Memory** | `@align`, `@repr`, `@no_alias`, `@used` |
| **Parallelism** | `@parallel`, `@vectorize`, `@unroll`, `@ivdep` |
| **Safety** | `@unsafe`, `@checked`, `@constant_time` |
| **FFI** | `@extern`, `@visibility`, `@target_feature` |
| **Verification** | `@requires`, `@ensures`, `@invariant` |
| **Meta** | `@derive`, `@cfg`, `@deprecated` |

## Capability System

Context attenuation for capability-based security:

```rust
use verum_ast::{Capability, CapabilitySet};

// Define available capabilities
let caps = CapabilitySet::from_iter([
    Capability::Named("FileRead".into()),
    Capability::Named("Network".into()),
]);

// Attenuate (reduce) capabilities for a scope
let attenuated = caps.attenuate(&CapabilitySet::from_iter([
    Capability::Named("FileRead".into()),
]));
// attenuated only has FileRead
```

## Module Structure

```rust
use verum_ast::{Module, CompilationUnit};

// A module represents a single source file
let module = Module {
    file_id: FileId::new(0),
    path: ModulePath::from("my_module"),
    items: vec![item1, item2],
    link_stmts: vec![],  // link declarations
};

// A compilation unit is the entire program
let unit = CompilationUnit {
    modules: vec![module],
    entry_point: Some(main_fn_path),
};
```

## Testing

Comprehensive test suite with **315+ tests** across 24 test files:

| Category | Tests | Coverage |
|----------|-------|----------|
| **Unit tests** | 143 | Core types, attributes, cfg predicates |
| **Expression tests** | 31 | All expression kinds, operators |
| **Type tests** | 59 | Type kinds, refinements, references |
| **Pattern tests** | 17 | All pattern kinds, nesting |
| **Visitor tests** | 16 | Recursive and iterative traversal |
| **Declaration tests** | 29 | Functions, types, protocols |
| **Serialization tests** | 23 | JSON round-trip stability |
| **Doc tests** | 19 | Documentation examples |
| **Property tests** | Multiple | proptest-based invariants |

Run tests:

```bash
cargo test -p verum_ast
```

## Dependencies

```toml
[dependencies]
verum_common = { workspace = true }  # List, Text, Map, Maybe, Heap
serde = { workspace = true }         # Serialization
logos = { workspace = true }         # Span integration

[dev-dependencies]
proptest = { workspace = true }      # Property-based testing
```

## Design Principles

1. **Semantic types**: Uses `List`, `Text`, `Map` from `verum_common` instead of std types
2. **Immutable by default**: AST nodes are typically immutable after construction
3. **Complete source tracking**: Every node has a `Span` for error reporting
4. **Lossless representation**: Preserves all source information for IDE support
5. **Capability tracking**: Expressions track capability requirements for context attenuation
6. **CBGR optimization hints**: Fields for escape analysis and check elimination

## Integration

The AST crate is used by:

- **verum_lexer**: Provides `Span` and `FileId` types
- **verum_parser**: Produces AST from source code
- **verum_types**: Type checks AST nodes
- **verum_codegen**: Generates code from AST
- **verum_lsp**: IDE features using AST

## License

Same as parent Verum project.
