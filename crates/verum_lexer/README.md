# Verum Lexer

High-performance lexer for the Verum programming language, built on the `logos` crate.

## Overview

The Verum lexer transforms source code into a stream of tokens for parsing. It provides:

- **Zero-copy operation**: Operates directly on source string slices without allocation
- **Fast DFA-based lexing**: Uses `logos` for optimized tokenization
- **Complete Verum v6 syntax**: All keywords, operators, and literals
- **Lossless lexing**: Preserves trivia (whitespace, comments) for IDE support
- **Error recovery**: Gracefully handles invalid tokens and continues lexing
- **Source tracking**: Preserves spans for high-quality error messages
- **Unicode support**: Full Unicode identifier support (Greek, CJK, etc.)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         PUBLIC API (lib.rs)                             │
│  Lexer, LookaheadLexer, LosslessLexer, Token, TokenKind                 │
└─────────────────────────────────────────────────────────────────────────┘
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        ▼                           ▼                           ▼
┌───────────────┐         ┌─────────────────┐         ┌─────────────────┐
│ lexer.rs      │         │ token.rs        │         │ lossless.rs     │
│ 218 lines     │         │ 3759 lines      │         │ 495 lines       │
│               │         │                 │         │                 │
│ - Lexer       │         │ - TokenKind     │         │ - LosslessLexer │
│ - Lookahead   │         │ - Token         │         │ - RichToken     │
│ - Iterator    │         │ - IntegerLit    │         │ - TriviaKind    │
│               │         │ - FloatLit      │         │ - Trivia preser │
└───────────────┘         └─────────────────┘         └─────────────────┘
                                    │
                          ┌─────────┴─────────┐
                          ▼                   ▼
                  ┌───────────────┐   ┌───────────────┐
                  │ error.rs      │   │ logos (DFA)   │
                  │ 65 lines      │   │               │
                  │               │   │ - Fast lexing │
                  │ - LexError    │   │ - Regex rules │
                  │ - Error types │   │ - Priorities  │
                  └───────────────┘   └───────────────┘
```

### Module Summary

| Module | Lines | Responsibility |
|--------|-------|----------------|
| `token.rs` | 3759 | Token definitions, literals, regex patterns |
| `lossless.rs` | 495 | Lossless lexer with trivia preservation |
| `lexer.rs` | 218 | Core lexer and lookahead lexer |
| `lib.rs` | 157 | Public API exports |
| `error.rs` | 65 | Error types and diagnostics |

## Usage

### Basic Lexing

```rust
use verum_lexer::{Lexer, TokenKind};
use verum_ast::span::FileId;

let source = "fn add(x: Int, y: Int) -> Int { x + y }";
let file_id = FileId::new(0);
let lexer = Lexer::new(source, file_id);

for token_result in lexer {
    let token = token_result.unwrap();
    println!("{:?} at {:?}", token.kind, token.span);
}
```

### Tokenize All at Once

```rust
use verum_lexer::Lexer;
use verum_ast::span::FileId;

let source = "let x = 42;";
let file_id = FileId::new(0);
let lexer = Lexer::new(source, file_id);

let tokens = lexer.tokenize().unwrap();
```

### Lookahead Lexing (for Parsers)

```rust
use verum_lexer::{LookaheadLexer, TokenKind};
use verum_ast::span::FileId;

let source = "fn main() {}";
let file_id = FileId::new(0);
let mut lexer = LookaheadLexer::new(source, file_id);

// Peek ahead without consuming
let token = lexer.peek(0).unwrap();
assert!(matches!(token.kind, TokenKind::Fn));

// Consume the token
let token = lexer.next_token().unwrap();
```

### Lossless Lexing (for IDEs)

```rust
use verum_lexer::LosslessLexer;
use verum_ast::span::FileId;

let source = "let x = 42; // comment";
let file_id = FileId::new(0);
let lexer = LosslessLexer::new(source, file_id);

for rich_token in lexer {
    // Includes leading and trailing trivia (whitespace, comments)
    println!("Token: {:?}", rich_token.token.kind);
    println!("Leading trivia: {:?}", rich_token.leading_trivia);
    println!("Trailing trivia: {:?}", rich_token.trailing_trivia);
}
```

## Token Types

### Keywords

**Reserved Keywords (3)**: Only these are always reserved in Verum v6:
- `let` - Variable binding
- `fn` - Function declaration
- `is` - Unified type syntax

**Primary Keywords**:
- `type`, `match`, `link`, `where`

**Contextual Keywords** (50+):
- Control flow: `if`, `else`, `while`, `for`, `loop`, `break`, `continue`, `return`, `yield`
- Modifiers: `mut`, `const`, `static`, `pure`, `meta`, `unsafe`
- Async: `async`, `await`, `spawn`, `select`, `nursery`
- Module: `module`, `implement`, `protocol`, `extends`, `context`, `provide`, `using`
- Visibility: `pub`, `public`, `internal`, `protected`, `private`
- Error handling: `try`, `throw`, `throws`, `defer`, `errdefer`, `finally`, `recover`
- Types: `stream`, `tensor`, `affine`, `linear`, `unknown`, `typeof`, `extern`
- Verification: `requires`, `ensures`, `invariant`, `decreases`, `result`, `checked`
- Formal proofs: `theorem`, `axiom`, `lemma`, `proof`, `qed`, `forall`, `exists`, `calc`, `have`, `show`, `by`, `induction`, `cases`, `contradiction`, `trivial`, `assumption`, `simp`, `ring`, `field`, `omega`, `auto`, `blast`, `smt`
- Meta: `quote`, `stage`, `lift`, `view`, `pattern`, `with`, `cofix`

**Built-in Variants**: `None`, `Some`, `Ok`, `Err`, `true`, `false`, `self`, `Self`

### Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+`, `-`, `*`, `/`, `%`, `**` |
| Comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical | `&&`, `\|\|`, `!` |
| Bitwise | `&`, `\|`, `^`, `<<`, `>>`, `~` |
| Assignment | `=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `\|=`, `^=`, `<<=`, `>>=` |
| Range | `..`, `..=`, `...` |
| Pipeline | `\|>` |
| Optional | `?.`, `??`, `?` |
| Arrow | `->`, `=>` |

### Literals

**Integer Literals** with base prefixes and optional suffixes:
```verum
42              // Decimal
0xFF            // Hexadecimal
0b1010          // Binary
0o77            // Octal
1_000_000_u64   // With separator and suffix
100i32          // Direct suffix (decimal/binary/octal)
0xFF_u8         // Hex requires underscore before suffix
```

**Float Literals** including hexfloats (IEEE 754):
```verum
3.14            // Decimal float
1.0e10          // Scientific notation
2.5E-3          // Negative exponent
0x1.8p10        // Hexfloat: 1.5 × 2^10 = 1536.0
3.14_f32        // With suffix
```

**String Literals**:
```verum
"hello"                // Simple string
"with\nescapes"        // Escape sequences
"""multiline
string"""              // Triple-quoted multiline
r"raw\nstring"         // Raw string (no escapes)
r#"raw with "quotes""# // Raw with hash delimiters
```

**Interpolated Strings** with automatic escaping:
```verum
f"Hello, {name}!"              // Format string
sql"SELECT * FROM {table}"     // SQL injection safe
sh"ls {dir}"                   // Shell command safe
html"<div>{content}</div>"     // HTML escaped
rx"pattern-{var}"              // Regex
json"{"key": {value}}"         // JSON
```
Prefixes: `f`, `sh`, `rx`, `sql`, `html`, `uri`, `url`, `json`, `xml`, `yaml`, `gql`

**Tagged Literals** for domain-specific data:
```verum
d#"2024-03-15"                 // Date/datetime
dur#"2h30m"                    // Duration
size#"1.5GB"                   // Data size
ip#"192.168.1.1"               // IP address
ver#"1.2.3"                    // Semantic version
geo#"51.5074,-0.1278"          // Geographic coordinate
interval#[0, 100]              // Numeric interval
mat#((1,2),(3,4))              // Matrix
vec#{1, 2, 3}                  // Vector
```

**Contract Literals** for formal verification:
```verum
contract#"
    requires x > 0;
    ensures result > 0
"
```

**Character Literals**:
```verum
'a'             // Simple char
'\n'            // Escape sequence
'\x41'          // Hex escape
'\u{1F600}'     // Unicode escape
b'x'            // Byte character
b"bytes"        // Byte string
```

**Other Literals**:
```verum
#FF5733         // Hex color (RGB)
#FF573380       // Hex color (RGBA)
'lifetime       // Lifetime annotation
```

### Delimiters and Punctuation

| Symbol | Name | Usage |
|--------|------|-------|
| `(` `)` | Parentheses | Grouping, function calls |
| `[` `]` | Brackets | Arrays, indexing |
| `{` `}` | Braces | Blocks, records |
| `,` | Comma | Separator |
| `;` | Semicolon | Statement terminator |
| `:` | Colon | Type annotation |
| `::` | Double colon | Path separator, turbofish |
| `.` | Dot | Member access |
| `@` | At | Attributes |
| `$` | Dollar | Context-adaptive literals |
| `#` | Hash | Tagged literals |

## Three-Tier Reference Model

Verum supports three tiers of references with different safety/performance tradeoffs:

```verum
&T          // Tier 0: CBGR-managed reference (~15ns overhead)
&checked T  // Tier 1: Compiler-proven safe (0ns overhead)
&unsafe T   // Tier 2: Manual safety proof required (0ns overhead)
```

The lexer tokenizes these as sequences: `Ampersand`, `Checked`/`Unsafe` (optional), `Ident`.

## Unicode Identifiers

Full Unicode support using Unicode character classes:

```verum
let π = 3.14159          // Greek letters
let データ = "data"       // Japanese
let café = "coffee"      // Accented Latin
let αβγ = compute()      // Mathematical symbols
```

Supported categories: `\p{L}` (letters), `\p{Nl}` (letter numbers), `\p{Nd}` (digits), `\p{Mn}`/`\p{Mc}` (marks).

## Error Handling

The lexer returns `Result<Token, LexError>` for each token:

```rust
use verum_lexer::{Lexer, LexError};
use verum_ast::span::FileId;

let source = "fn @invalid";
let file_id = FileId::new(0);

for token_result in Lexer::new(source, file_id) {
    match token_result {
        Ok(token) => println!("Token: {:?}", token.kind),
        Err(LexError::InvalidToken { span }) => {
            println!("Invalid token at {:?}", span);
        }
        Err(e) => println!("Lexer error: {}", e),
    }
}
```

Invalid tokens are reported but don't stop lexing - the lexer continues to find subsequent valid tokens.

## Integration with Parser

The lexer is designed to integrate seamlessly with `verum_parser`:

```rust
use verum_lexer::LookaheadLexer;
use verum_ast::span::FileId;

let source = "fn add(x: Int) -> Int { x + 1 }";
let file_id = FileId::new(0);
let mut lexer = LookaheadLexer::new(source, file_id);

// Parser can peek ahead and consume tokens as needed
while let Some(token) = lexer.peek(0) {
    if matches!(token.kind, TokenKind::Eof) {
        break;
    }
    // parse_item(&mut lexer)?;
    lexer.next_token();
}
```

## Performance

- **Zero allocation** during lexing (operates on string slices)
- **DFA-based** tokenization for O(n) linear-time performance
- **No backtracking** required
- **Efficient string parsing** with regex-based pattern matching
- **Nested comment support** via callback function

## Testing

Comprehensive test suite with **156+ tests** across 7 test files:

| Test File | Tests | Coverage |
|-----------|-------|----------|
| `keyword_tests.rs` | 35 | All keywords and contextual keywords |
| `token_tests.rs` | 32 | Operators, delimiters, whitespace |
| `property_tests.rs` | 13 | Property-based testing with proptest |
| `safety.rs` | 20 | Edge cases, invalid input, memory safety |
| `v6_compliance_tests.rs` | 13 | Verum v6 grammar compliance |
| `lib.rs` (inline) | 40 | Unit tests for modules |
| `tuple_index_test.rs` | 1 | Tuple indexing |
| Doc tests | 2 | Documentation examples |

Run tests:

```bash
cargo test -p verum_lexer
```

## Dependencies

```toml
[dependencies]
logos = { workspace = true }           # DFA-based lexer generator
verum_ast = { workspace = true }       # AST types (Span, FileId)
verum_common = { workspace = true }    # Common types (Text, Maybe)
```

## Keyword Changes (v5.1 → v6)

| Old Keyword | New Keyword |
|-------------|-------------|
| `import` | `link` |
| `package` | `cog` |
| `crate` | `cog` |

## License

Same as parent Verum project.
