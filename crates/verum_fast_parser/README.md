# Verum Parser

Production-ready recursive descent parser for the Verum programming language.

## Overview

The Verum parser transforms source code into an Abstract Syntax Tree (AST) following the grammar defined in `grammar/verum.ebnf`. It provides:

- **Complete grammar coverage**: All Verum v6 syntax including refinement types, stream comprehensions, pipeline operators, three-tier references, FFI boundaries, and context system
- **Pratt parsing**: Efficient operator precedence parsing for expressions
- **Excellent error recovery**: Continues parsing after errors with synchronization points
- **Precise diagnostics**: Integration with `verum_diagnostics` for high-quality error messages
- **Incremental parsing**: IDE-ready support for partial re-parsing
- **Lossless parsing**: Preserves all source information including whitespace and comments
- **Fast compilation**: ~3 seconds compile time (vs ~5+ minutes with parser combinators)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         PUBLIC API (lib.rs)                             │
│  VerumParser, Parser, RecursiveParser, TokenStream                      │
│  LosslessParser, EventBasedParser, RecoveringEventParser                │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────┐         ┌─────────────────┐         ┌─────────────────┐
│ expr.rs       │         │ decl.rs         │         │ stmt.rs         │
│ 5279 lines    │         │ 4861 lines      │         │ 1107 lines      │
│               │         │                 │         │                 │
│ - Pratt parser│         │ - Functions     │         │ - Let bindings  │
│ - Binary ops  │         │ - Types         │         │ - Expressions   │
│ - Pipelines   │         │ - Protocols     │         │ - Control flow  │
│ - Comprehens. │         │ - Impls         │         │ - Defer         │
│ - Closures    │         │ - FFI           │         │ - Loops         │
│ - Match       │         │ - Contexts      │         │                 │
└───────────────┘         └─────────────────┘         └─────────────────┘
        │                           │                           │
        └───────────────────────────┼───────────────────────────┘
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          SUPPORT MODULES                                │
├─────────────────┬──────────────────┬───────────────────┬────────────────┤
│ ty.rs (2547)    │ pattern.rs (2026)│ parser.rs (1263)  │ error.rs (302) │
│                 │                  │                   │                │
│ - Primitives    │ - Wildcard       │ - TokenStream     │ - ParseError   │
│ - Generics      │ - Tuple          │ - RecursiveParser │ - ErrorKind    │
│ - Refinements   │ - Record         │ - Span utilities  │ - Diagnostics  │
│ - References    │ - Variant        │                   │                │
│ - Function      │ - Array          │                   │                │
│ - Sigma         │ - Range          │                   │                │
└─────────────────┴──────────────────┴───────────────────┴────────────────┘
                                    │
┌─────────────────────────────────────────────────────────────────────────┐
│                        ADVANCED FEATURES                                │
├────────────────────┬───────────────────┬────────────────────────────────┤
│ recovery.rs (1406) │ incremental.rs    │ syntax_bridge.rs (2077)        │
│                    │ (618)             │                                │
│ - Sync points      │ - Document cache  │ - Event-based parsing          │
│ - Recovery sets    │ - Change tracking │ - Lossless tree                │
│ - Error repair     │ - Partial reparse │ - IDE support                  │
├────────────────────┼───────────────────┼────────────────────────────────┤
│ ast_sink.rs (4238) │ recovery_parser   │ attr_validation.rs (782)       │
│                    │ (1371)            │                                │
│ - Green to AST     │ - Full recovery   │ - Attribute validation         │
│ - Semantic nodes   │ - Event parser    │ - Field/function attrs         │
└────────────────────┴───────────────────┴────────────────────────────────┘
```

### Module Summary

| Module | Lines | Responsibility |
|--------|-------|----------------|
| `expr.rs` | 5279 | Expression parsing with Pratt algorithm |
| `decl.rs` | 4861 | Declaration parsing (fn, type, protocol, impl, FFI) |
| `ast_sink.rs` | 4238 | Convert green tree to semantic AST |
| `ty.rs` | 2547 | Type parsing (primitives, generics, refinements) |
| `syntax_bridge.rs` | 2077 | Event-based and lossless parsing |
| `pattern.rs` | 2026 | Pattern parsing (match, let, for) |
| `recovery.rs` | 1406 | Error recovery and synchronization |
| `recovery_parser.rs` | 1371 | Full error recovery event parser |
| `proof.rs` | 1263 | Proof block parsing for verification |
| `parser.rs` | 1263 | Core infrastructure (TokenStream, RecursiveParser) |
| `stmt.rs` | 1107 | Statement parsing |
| `script_type_integration.rs` | 983 | Type checking for scripts (deprecated) |
| `attr_validation.rs` | 782 | Attribute validation |
| `incremental.rs` | 618 | Incremental parsing for IDEs |
| `safe_interpolation.rs` | 404 | Safe string interpolation parsing |
| `error.rs` | 302 | Error types and result definitions |

## Public API

### Primary Types

```rust
use verum_parser::{
    // Main parser
    VerumParser,           // Full-featured parser
    Parser,                // Simple wrapper for testing/REPL

    // Core infrastructure
    RecursiveParser,       // Low-level recursive descent parser
    TokenStream,           // Token stream with lookahead

    // Error handling
    ParseError,            // Parser error type
    ParseResult,           // Result<T, List<ParseError>>

    // Recovery
    RecoveryStrategy,      // Error recovery approach
    RecoverySet,           // Token sets for synchronization
    SyncPoint,             // Synchronization point marker

    // Lossless/IDE parsing
    LosslessParser,        // Preserves all source information
    EventBasedParser,      // Event-driven parsing
    RecoveringEventParser, // Event parser with recovery
    IncrementalParser,     // Incremental re-parsing
    IncrementalDocument,   // Document with change tracking

    // Attribute validation
    AttributeValidator,    // Validate parsed attributes
    ValidationConfig,      // Validation configuration
};
```

### Basic Usage

```rust
use verum_parser::VerumParser;
use verum_lexer::Lexer;
use verum_ast::FileId;

// Parse a complete module
let source = r#"
    fn factorial(n: Int{>= 0}) -> Int {
        match n {
            0 => 1,
            n => n * factorial(n - 1)
        }
    }
"#;

let file_id = FileId::new(0);
let lexer = Lexer::new(source, file_id);
let parser = VerumParser::new();

match parser.parse_module(lexer, file_id) {
    Ok(module) => println!("Parsed {} items", module.items.len()),
    Err(errors) => {
        for error in errors {
            eprintln!("{}", error);
        }
    }
}
```

### Expression Parsing

```rust
use verum_parser::VerumParser;
use verum_ast::FileId;

let parser = VerumParser::new();
let file_id = FileId::new(0);

// Parse a single expression
let expr = parser.parse_expr_str("x * 2 + y", file_id)?;

// Parse a type
let ty = parser.parse_type_str("List<Int{> 0}>", file_id)?;
```

### Meta-programming Support

```rust
use verum_parser::VerumParser;
use verum_lexer::Token;
use verum_common::List;

let parser = VerumParser::new();

// Parse from pre-lexed tokens (for macros)
let tokens: List<Token> = /* ... */;
let expr = parser.parse_expr_tokens(&tokens)?;
let ty = parser.parse_type_tokens(&tokens)?;
let item = parser.parse_item_tokens(&tokens)?;
```

## Key Features

### Refinement Types

Verum supports refinement types that constrain base types with predicates:

```verum
type Positive is Int{> 0}
type Email is Text{is_email(it)}
type SortedList<T> is List<T>{is_sorted(it)}
type BoundedInt is Int{>= 0, < 100}  // Multiple constraints
```

The parser handles the implicit `it` variable in refinement predicates.

### Stream Comprehensions

Lazy stream processing with comprehension syntax:

```verum
stream[x * 2 for x in source if x > 0]
stream[(a, b) for a in xs for b in ys if a < b]
stream[y for x in items let y = transform(x)]
```

### Pipeline Operator

Natural left-to-right data flow:

```verum
input
    |> parse
    |> validate
    |> transform
    |> save
```

### Three-Tier Reference Model

```verum
&T          // Tier 0: CBGR-managed reference (~15ns overhead)
&checked T  // Tier 1: Compiler-proven safe (0ns overhead)
&unsafe T   // Tier 2: Manual safety proof required (0ns overhead)
```

### FFI Boundary Parsing

Complete FFI support with ownership and error handling:

```verum
@extern("C")
type CFile is @opaque;

@extern("C")
fn fopen(
    @ownership(borrow) filename: &unsafe Text,
    @ownership(borrow) mode: &unsafe Text
) -> @nullable CFile;

@extern("C")
errors_via = ReturnValue(null) with Errno;
fn malloc(size: usize) -> @nullable &unsafe u8;
```

### Context System

Dependency injection with explicit context tracking:

```verum
context Database;
context Logger;

fn query(sql: Text) using Database -> Result<Data, Error> {
    // Has access to Database context
}

fn process() using [Database, Logger] -> Result<(), Error> {
    log("Processing...");  // Uses Logger
    let data = query("SELECT * FROM users");  // Uses Database
    Ok(())
}
```

### Operator Precedence

The parser implements proper operator precedence using Pratt parsing (from lowest to highest):

| Precedence | Operators | Associativity |
|------------|-----------|---------------|
| 1 | `\|>` (pipeline) | Left |
| 2 | `??` (null coalescing) | Right |
| 3 | `=`, `+=`, `-=`, etc. | Right |
| 4 | `\|\|` (logical or) | Left |
| 5 | `&&` (logical and) | Left |
| 6 | `==`, `!=`, `<`, `>`, `<=`, `>=` | Left |
| 7 | `\|` (bitwise or) | Left |
| 8 | `^` (bitwise xor) | Left |
| 9 | `&` (bitwise and) | Left |
| 10 | `<<`, `>>` (shift) | Left |
| 11 | `+`, `-` (additive) | Left |
| 12 | `*`, `/`, `%` (multiplicative) | Left |
| 13 | `**` (exponentiation) | Right |
| 14 | `!`, `-`, `~`, `&`, `*` (unary) | Prefix |
| 15 | `.`, `?.`, `()`, `[]`, `?`, `as` | Postfix |

## Error Recovery

The parser implements comprehensive error recovery:

```rust
use verum_parser::{RecoveryStrategy, RecoverySet, SyncPoint};

// Recovery strategies
enum RecoveryStrategy {
    Skip,           // Skip tokens until sync point
    Insert,         // Insert missing token
    Replace,        // Replace invalid token
    Synchronize,    // Jump to next statement/item
}

// Sync points for recovery
fn is_statement_terminator(token: &Token) -> bool;
fn can_start_item(token: &Token) -> bool;
fn can_start_statement(token: &Token) -> bool;
fn can_start_expression(token: &Token) -> bool;
```

## Incremental Parsing

For IDE integration:

```rust
use verum_parser::{IncrementalDocument, IncrementalParserEngine};

let mut doc = IncrementalDocument::new(source, file_id);
let engine = IncrementalParserEngine::new();

// Initial parse
let tree = engine.parse(&doc)?;

// Apply edit
doc.apply_edit(start, end, new_text);

// Re-parse (only changed regions)
let updated_tree = engine.reparse(&doc, &tree)?;
```

## Testing

The parser has extensive test coverage across 75+ test files:

```bash
# Run all parser tests
cargo test -p verum_parser

# Run specific test suites
cargo test -p verum_parser --test expr_tests
cargo test -p verum_parser --test decl_tests
cargo test -p verum_parser --test grammar_tests
cargo test -p verum_parser --test ffi_boundary_tests
```

### Test Suites

| Category | Test Files | Coverage |
|----------|------------|----------|
| **Core Parsing** | `expr_tests.rs`, `decl_tests.rs`, `stmt_tests.rs`, `ty_tests.rs`, `pattern_tests.rs` | Expressions, declarations, statements, types, patterns |
| **Grammar** | `grammar_tests.rs`, `v6_compliance_tests.rs` | Full grammar compliance |
| **FFI** | `ffi_boundary_tests.rs`, `ffi_parsing_tests.rs`, `ffi_error_protocol_tests.rs` | FFI boundaries, extern functions |
| **Comprehensions** | `stream_comprehension_tests.rs`, `comprehension_tests.rs` | Stream and list comprehensions |
| **Types** | `type_tests.rs`, `type_expr_tests.rs`, `sigma_type_tests.rs`, `dyn_type_tests.rs` | All type constructs |
| **Contexts** | `context_parsing_tests.rs`, `context_system_tests.rs`, `context_comprehensive_tests.rs` | Context system |
| **Advanced** | `hkt_parsing_tests.rs`, `gat_syntax_tests.rs`, `lambda_refinement_tests.rs` | HKT, GAT, advanced types |
| **Error Handling** | `error_tests.rs`, `error_recovery_tests.rs`, `error_message_audit.rs` | Error messages and recovery |
| **Edge Cases** | `edge_cases.rs`, `precedence_tests.rs`, `operator_tests.rs` | Corner cases |
| **Integration** | `integration.rs`, `integration_tests.rs`, `parse_examples.rs` | End-to-end tests |
| **Property-Based** | `properties.rs`, `type_property_tests.rs` | proptest integration |

## Dependencies

```toml
[dependencies]
verum_ast = { workspace = true }       # AST definitions
verum_common = { workspace = true }    # Common types (List, Text, Map)
verum_lexer = { workspace = true }     # Tokenization
verum_diagnostics = { workspace = true } # Error reporting
verum_syntax = { workspace = true }    # Syntax kinds

[dev-dependencies]
proptest = { workspace = true }        # Property-based testing
criterion = { workspace = true }       # Benchmarking
```

## Performance

- **Compilation**: ~3 seconds for the parser crate
- **Parsing**: Single-pass O(n) complexity for most constructs
- **Memory**: Streaming token consumption
- **Incremental**: Only re-parses changed regions

## Migration Notes

### Script Modules Moved to verum_lsp

The interactive script parsing modules have been moved to `verum_lsp`:

```rust
// Old (deprecated)
// use verum_parser::{ScriptParser, ScriptContext, ScriptRecovery};

// New
use verum_lsp::script::{
    ScriptParser,
    ScriptContext,
    ScriptRecovery,
    IncrementalScriptParser,
};
```

### Keyword Changes (v5.1 → v6)

| Old Keyword | New Keyword |
|-------------|-------------|
| `import` | `link` |
| `package` | `cog` |
| `crate` | `cog` |

## License

Same as parent Verum project.
