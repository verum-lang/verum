# Quote Module Examples

This document provides comprehensive examples of using the Quote module for Verum's meta-programming system.

## Basic TokenStream Operations

### Creating Token Streams

```rust
use verum_compiler::quote::{TokenStream, ident, literal_int, literal_string};
use verum_ast::Span;

// Create an empty token stream
let mut stream = TokenStream::new();

// Add tokens manually
stream.push(Token::new(TokenKind::Let, Span::default()));
stream.push(Token::new(TokenKind::Ident("x".to_string()), Span::default()));

// Use helper functions
let name = ident("foo", Span::default());
let value = literal_int(42, Span::default());
let text = literal_string("hello", Span::default());
```

### Parsing from String

```rust
use verum_compiler::quote::TokenStream;
use verum_ast::FileId;

let source = "let x = 42;";
let ts = TokenStream::from_str(source, FileId::new(0))?;

// Parse as expression
let expr = ts.parse_as_expr()?;

// Parse as type
let type_str = "List<Int>";
let ts = TokenStream::from_str(type_str, FileId::new(0))?;
let ty = ts.parse_as_type()?;

// Parse as item (function, type, etc.)
let fn_str = "fn add(a: Int, b: Int) -> Int { a + b }";
let ts = TokenStream::from_str(fn_str, FileId::new(0))?;
let item = ts.parse_as_item()?;
```

## QuoteBuilder - Programmatic Construction

### Basic Building

```rust
use verum_compiler::quote::QuoteBuilder;
use verum_ast::Span;

// Build: let x = 42;
let stream = QuoteBuilder::with_span(Span::default())
    .keyword("let")
    .ident("x")
    .punct("=")
    .int(42)
    .punct(";")
    .build();
```

### Function Generation

```rust
use verum_compiler::quote::generate_fn;

let params = vec![
    ("a".to_string(), "Int".to_string()),
    ("b".to_string(), "Int".to_string()),
];

let body = QuoteBuilder::new()
    .ident("a")
    .punct("+")
    .ident("b")
    .build();

let fn_stream = generate_fn(
    "add",
    &params,
    Some("Int"),
    body,
    Span::default(),
);
```

### Implement Block Generation

```rust
use verum_compiler::quote::generate_impl;

let methods = QuoteBuilder::new()
    .keyword("fn")
    .ident("to_string")
    .punct("(")
    .punct("&")
    .keyword("self")
    .punct(")")
    .punct("->")
    .ident("Text")
    .punct("{")
    // method body
    .punct("}")
    .build();

let impl_block = generate_impl(
    "ToString",
    "MyType",
    methods,
    Span::default(),
);
```

### Repetition with QuoteBuilder

```rust
let fields = vec!["name", "age", "email"];

let stream = QuoteBuilder::new()
    .ident("Person")
    .punct("{")
    .repeat(
        fields,
        Some(","),
        |field| {
            QuoteBuilder::new()
                .ident(field)
                .punct(":")
                .ident("Text")
                .build()
        }
    )
    .punct("}")
    .build();

// Generates: Person { name: Text, age: Text, email: Text }
```

## Quote! Macro System

### Simple Interpolation

```rust
use verum_compiler::quote::{Quote, MetaContext};
use verum_core::Text;

// Parse a quote template
let quote = Quote::parse("let #name = #value;")?;

// Create context with bindings
let mut ctx = MetaContext::new();
ctx.bind_single(
    Text::from("name"),
    ident("my_var", Span::default())
);
ctx.bind_single(
    Text::from("value"),
    literal_int(42, Span::default())
);

// Expand the quote
let result = quote.expand(&ctx)?;
// Result: let my_var = 42;
```

### Repetition Patterns

```rust
// Parse quote with repetition
let quote = Quote::parse("#(#items),*")?;

// Create context with repeated values
let mut ctx = MetaContext::new();
let items = List::from_iter(vec![
    ident("a", Span::default()),
    ident("b", Span::default()),
    ident("c", Span::default()),
]);
ctx.bind_repeat(Text::from("items"), items);

// Expand
let result = quote.expand(&ctx)?;
// Result: a, b, c
```

### Complex Example - Generate Getter Methods

```rust
use verum_compiler::quote::{Quote, MetaContext, QuoteBuilder};
use verum_core::List;

fn generate_getters(fields: &[&str]) -> Result<TokenStream, QuoteError> {
    // Build individual getter functions
    let mut ctx = MetaContext::new();

    let getters: List<TokenStream> = fields.iter().map(|field| {
        let quote = Quote::parse(
            "fn get_#field(&self) -> &Self::#field_type {
                &self.#field
            }"
        ).unwrap();

        let mut field_ctx = MetaContext::new();
        field_ctx.bind_single(
            Text::from("field"),
            ident(field, Span::default())
        );
        field_ctx.bind_single(
            Text::from("field_type"),
            ident("Text", Span::default())  // or infer from type
        );

        quote.expand(&field_ctx).unwrap()
    }).collect();

    // Combine all getters
    let mut result = TokenStream::new();
    for getter in getters {
        result.extend(getter);
    }

    Ok(result)
}
```

## ToTokens Trait

### Converting AST to Tokens

```rust
use verum_compiler::quote::ToTokens;
use verum_ast::expr::{Expr, ExprKind};

// Any AST node can be converted to tokens
let expr = Expr::new(
    ExprKind::Binary {
        op: BinOp::Add,
        left: Box::new(/* ... */),
        right: Box::new(/* ... */),
    },
    Span::default(),
);

// Convert to token stream
let mut stream = TokenStream::new();
expr.to_tokens(&mut stream);

// Or use the convenience method
let stream = expr.into_token_stream();
```

### Round-Trip Conversion

```rust
// Original expression
let original = Expr::new(
    ExprKind::Path(Path::single(Ident::new("x", Span::default()))),
    Span::default(),
);

// Convert to tokens
let stream = original.into_token_stream();

// Parse back to expression
let parsed = stream.parse_as_expr()?;

// Should match original structure
assert!(matches!(parsed.kind, ExprKind::Path(_)));
```

## Hygiene System

### Generating Unique Identifiers

```rust
use verum_compiler::hygiene::HygieneContext;

let hygiene = HygieneContext::new();

// Generate unique identifiers
let temp1 = hygiene.generate("temp");  // "temp__0"
let temp2 = hygiene.generate("temp");  // "temp__1"
let temp3 = hygiene.generate("temp");  // "temp__2"

// Check if an identifier is hygienic
assert!(HygieneContext::is_hygienic(temp1.as_str()));

// Extract base name
let base = HygieneContext::base_name(temp1.as_str());
assert_eq!(base.as_str(), "temp");
```

### Hygienic Code Generation

```rust
use verum_compiler::quote::QuoteBuilder;

let builder = QuoteBuilder::new();

// Generate hygienic temporary variable
let stream = builder
    .keyword("let")
    .hygienic_ident("temp")  // Generates unique name
    .punct("=")
    .ident("expensive_computation")
    .punct("(")
    .punct(")")
    .punct(";")
    .build();

// The generated identifier won't conflict with user code
```

## Advanced Patterns

### Conditional Generation

```rust
let has_return_type = true;
let return_type = "Int";

let stream = QuoteBuilder::new()
    .keyword("fn")
    .ident("my_function")
    .punct("(")
    .punct(")")
    .optional(
        has_return_type,
        || {
            QuoteBuilder::new()
                .punct("->")
                .ident(return_type)
                .build()
        }
    )
    .punct("{")
    .punct("}")
    .build();
```

### Nested Repetition

```rust
// Generate multiple functions with multiple parameters each
let functions = vec![
    ("add", vec!["a", "b"]),
    ("subtract", vec!["x", "y"]),
];

let stream = QuoteBuilder::new()
    .repeat(
        functions,
        None,  // No separator between functions
        |(name, params)| {
            QuoteBuilder::new()
                .keyword("fn")
                .ident(name)
                .punct("(")
                .repeat(
                    params,
                    Some(","),
                    |param| {
                        QuoteBuilder::new()
                            .ident(param)
                            .punct(":")
                            .ident("Int")
                            .build()
                    }
                )
                .punct(")")
                .punct("->")
                .ident("Int")
                .punct("{")
                // function body
                .punct("}")
                .build()
        }
    )
    .build();
```

### Match Arm Generation

```rust
use verum_compiler::quote::generate_match_arm;

let pattern = ident("Some", Span::default());
let body = QuoteBuilder::new()
    .ident("value")
    .build();

let arm = generate_match_arm(pattern, body, Span::default());
// Generates: Some => value
```

### Struct Literal Generation

```rust
use verum_compiler::quote::generate_struct_literal;

let fields = vec![
    ("name".to_string(), literal_string("John", Span::default())),
    ("age".to_string(), literal_int(30, Span::default())),
];

let struct_lit = generate_struct_literal(
    "Person",
    &fields,
    Span::default(),
);
// Generates: Person { name: "John", age: 30 }
```

## Error Handling

### ParseError Handling

```rust
use verum_compiler::quote::{ParseError, TokenStream};

let ts = TokenStream::new();
let result = ts.parse_as_expr();

match result {
    Ok(expr) => println!("Parsed: {:?}", expr),
    Err(ParseError::EmptyTokenStream) => {
        println!("Cannot parse empty stream");
    }
    Err(ParseError::ParseFailed(msg)) => {
        println!("Parse failed: {}", msg);
    }
    Err(ParseError::UnconsumedTokens { count, first_token }) => {
        println!("Unconsumed tokens: {} starting with {}", count, first_token);
    }
    Err(ParseError::NotImplemented(msg)) => {
        println!("Not implemented: {}", msg);
    }
}
```

### QuoteError Handling

```rust
use verum_compiler::quote::{Quote, QuoteError, MetaContext};

let quote = Quote::parse("let #name = #value;");

match quote {
    Ok(q) => {
        let ctx = MetaContext::new();
        match q.expand(&ctx) {
            Ok(stream) => println!("Success"),
            Err(QuoteError::UnboundVariable(var)) => {
                println!("Unbound variable: {}", var);
            }
            Err(QuoteError::RepetitionMismatch(msg)) => {
                println!("Repetition mismatch: {}", msg);
            }
            Err(e) => println!("Other error: {}", e),
        }
    }
    Err(e) => println!("Parse error: {}", e),
}
```

## Integration with Meta Functions

### Complete Meta Function Example

```rust
use verum_compiler::quote::{Quote, MetaContext, TokenStream};
use verum_ast::{Expr, Item};

// Meta function to generate a Debug implementation
fn meta_derive_debug(item: &Item) -> Result<TokenStream, String> {
    match item {
        Item::Struct { name, fields, .. } => {
            // Build field formatting
            let quote = Quote::parse(
                "implement Debug for #type_name {
                    fn fmt(&self) -> Text {
                        #(self.#field_name.to_string()),*
                    }
                }"
            ).map_err(|e| e.to_string())?;

            let mut ctx = MetaContext::new();
            ctx.bind_single(
                Text::from("type_name"),
                ident(name.as_str(), Span::default())
            );

            // Bind field names for repetition
            let field_names: List<TokenStream> = fields.iter()
                .map(|f| ident(f.name.as_str(), Span::default()))
                .collect();
            ctx.bind_repeat(Text::from("field_name"), field_names);

            quote.expand(&ctx).map_err(|e| e.to_string())
        }
        _ => Err("Expected struct".to_string()),
    }
}
```

## Performance Considerations

1. **Token Stream Reuse**: Reuse `TokenStream` objects instead of creating new ones
2. **QuoteBuilder Chaining**: Build complex streams in a single chain
3. **Batch Operations**: Use `repeat()` instead of manual loops
4. **Hygiene Context**: Reuse `HygieneContext` for related code generation

## Testing

### Unit Test Example

```rust
#[test]
fn test_quote_expansion() {
    let quote = Quote::parse("let #name = #value;").unwrap();
    let mut ctx = MetaContext::new();
    ctx.bind_single(Text::from("name"), ident("x", Span::default()));
    ctx.bind_single(Text::from("value"), literal_int(42, Span::default()));

    let result = quote.expand(&ctx).unwrap();

    // Verify the result
    let expr = result.parse_as_expr().unwrap();
    assert!(matches!(expr.kind, ExprKind::Block(_)));
}
```

## Spec Reference

See `docs/detailed/17-meta-system.md` for the complete specification of Verum's meta-programming system.
