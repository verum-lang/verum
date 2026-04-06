# World-Class Context Error Diagnostics

This document describes the enhanced context error diagnostics system in `verum_diagnostics`, designed to provide maximally user-friendly error messages for context-related issues in Verum.

## Overview

The context error system implements the recommendations from `docs/audit.md` to save newcomers from frustration with clear, helpful errors. It features:

- **Call Chain Propagation Visualization**: Shows the entire call stack and how context requirements propagate
- **E0301-E0306 Error Codes**: Specialized errors for different context issues
- **Smart Suggestions**: Context-aware fixes based on common patterns
- **"Did You Mean" Suggestions**: Levenshtein distance-based typo detection
- **IDE-Friendly Output**: Quick-fix hints for language servers

## Error Codes

### E0301: Context Used But Not Declared

The most common context error - a function uses a context that hasn't been declared in its signature.

**Example:**

```verum
async fn get_user(id: UserId) -> User {
    Database.fetch_user(id).await?  // Error: Database context not declared
}
```

**Error Output:**

```
error[E0301]: context 'Database' used but not declared
  --> src/user_service.vr:42:15
  |
42 |   let user = Database.fetch_user(id).await?;
   |              ^^^^^^^^ requires [Database] context
   |
   = note: call chain requiring 'Database':
     main() @ src/main.vr:10
     └─> handle_request() @ src/handlers.vr:20
         └─> process_user() @ src/user_service.vr:35 [requires Logger]
             └─> get_user() @ src/user_service.vr:42 [requires Database]

   = help: add 'using [Database]' to function signature:
     async fn get_user(id: UserId) -> User
         using [Database]  // <-- add this
```

### E0302: Context Declared But Not Provided

A function declares a context requirement but the context wasn't provided at the call site.

**Example:**

```verum
async fn get_user(id: UserId) -> User
    using [Database]
{
    Database.fetch_user(id).await?
}

async fn main() {
    // Missing: provide Database = ...
    let user = get_user(UserId(42)).await;  // Error: Database not provided
}
```

### E0303: Context Type Mismatch

The provided context implementation doesn't match the expected interface.

**Example:**

```verum
provide Database = ConsoleLogger::new();  // Wrong type!
```

### E0306: Context Group Undefined

Using a context group that hasn't been defined.

**Example:**

```verum
async fn handle_request(req: Request) -> Response
    using WebContext  // Error: WebContext not defined
{
    // ...
}
```

## Call Chain Visualization

The call chain visualization is the killer feature of this system. It shows:

1. **Entry Point**: Where the call chain starts (usually `main`)
2. **Propagation Path**: Each function in the chain with location
3. **Context Requirements**: What contexts each function needs
4. **Error Origin**: The function where the missing context is used

### Visual Format

```
call chain requiring 'Database':
  main() @ src/main.vr:10
  └─> handle_request() @ src/handlers.vr:20
      └─> process_user() @ src/user_service.vr:35 [requires Logger]
          └─> get_user() @ src/user_service.vr:42 [requires Database, Logger]
```

This makes it immediately clear:
- Where to add the context declaration (usually at the entry point)
- How the context requirement propagates through the call stack
- What other contexts are being used in the same chain

## Smart Suggestions

The system provides context-aware suggestions based on common patterns:

### 1. Add Context to Function Signature

```verum
fn process_user(id: UserId) -> Result<UserData, Error>
    using [Database]  // <-- add this
```

**When to use**: Most common fix - add the context to the function signature

### 2. Provide Context at Call Site

```verum
provide Database = PostgresDB::connect("localhost").await;
get_user(UserId(42)).await;
```

**When to use**: When you want to provide the context rather than propagate the requirement

### 3. Create Context Group

```verum
using WebContext = [Database, Logger, Auth, Metrics];

fn handle_request(req: Request) -> Response
    using WebContext
```

**When to use**: When multiple functions use the same set of contexts (3+)

### 4. Use Existing Context Group

```verum
// Instead of:
fn handler() using [Database, Logger, Auth] { }

// Use:
fn handler() using WebContext { }
```

**When to use**: When a suitable context group already exists

## "Did You Mean" Suggestions

The system uses Levenshtein distance to detect typos and suggest corrections:

```verum
using [Datbase]  // Typo
```

```
error[E0301]: context 'Datbase' used but not declared
  ...
  = note: did you mean one of these contexts?
    1. Database
    2. DataSource
    3. DatabasePool
```

**Algorithm**: Suggests contexts within edit distance ≤3, up to 5 suggestions

## Usage Examples

### Creating E0301 Errors (Simple)

```rust
use verum_diagnostics::{ContextNotDeclaredError, Span};

let error = ContextNotDeclaredError::new(
    "Database",
    Span::new("src/user.vr", 42, 15, 25),
)
.build();
```

### Creating E0301 Errors (With Call Chain)

```rust
use verum_diagnostics::{CallChain, CallFrame, ContextNotDeclaredError, Span};

let chain = CallChain::new("Database")
    .add_frame(CallFrame::new(
        "main",
        Span::new("src/main.vr", 10, 1, 5),
    ))
    .add_frame(CallFrame::new(
        "handle_request",
        Span::new("src/handlers.vr", 20, 5, 19),
    ))
    .add_frame(
        CallFrame::new(
            "get_user",
            Span::new("src/user.vr", 42, 15, 25),
        )
        .with_contexts(vec!["Database".to_string()])
        .origin(),
    );

let error = ContextNotDeclaredError::new(
    "Database",
    Span::new("src/user.vr", 42, 15, 25),
)
.with_call_chain(chain)
.build();
```

### Creating E0301 Errors (With "Did You Mean")

```rust
use verum_diagnostics::{ContextNotDeclaredError, Span};

let similar = vec![
    "Database".to_string(),
    "DataSource".to_string(),
];

let error = ContextNotDeclaredError::new(
    "Datbase",  // Typo
    Span::new("src/user.vr", 42, 15, 25),
)
.with_similar_contexts(similar)
.build();
```

### Finding Similar Contexts

```rust
use verum_diagnostics::context_error::find_similar_contexts;

let available = vec![
    "Logger".to_string(),
    "Database".to_string(),
    "Auth".to_string(),
];

let similar = find_similar_contexts("Loger", &available);
// Returns: ["Logger"]
```

## Rendering

The renderer automatically detects call chain notes and applies special formatting:

```rust
use verum_diagnostics::Renderer;

let mut renderer = Renderer::default();
let output = renderer.render(&error);
println!("{}", output);
```

**Features:**
- Colored output (can be disabled with `RenderConfig::no_color()`)
- Source code snippets with line numbers
- Highlighted arrows (`└─>`) and context requirements
- Proper indentation for nested chains

## Integration with Compiler

When implementing context checking in the compiler:

1. **Track Context Requirements**: Build a call graph tracking context usage
2. **Build Call Chains**: When a context is missing, construct the full call chain
3. **Find Similar Contexts**: Use `find_similar_contexts()` for typo suggestions
4. **Generate Suggestions**: Use `CallChain::suggestions()` for fixes
5. **Emit Diagnostic**: Create the appropriate error type and render

### Example Compiler Integration

```rust
// In your type checker/context analyzer

fn check_context_requirements(
    &self,
    function: &FunctionDef,
) -> Result<(), Diagnostic> {
    let required_contexts = self.analyze_context_usage(function)?;
    let declared_contexts = function.context_declarations();

    for ctx in required_contexts {
        if !declared_contexts.contains(&ctx) {
            // Build call chain
            let chain = self.build_call_chain(&ctx, function);

            // Find similar contexts for typo suggestions
            let available = self.get_available_contexts(function.scope());
            let similar = find_similar_contexts(&ctx, &available);

            // Create and emit error
            let error = ContextNotDeclaredError::new(
                &ctx,
                self.get_usage_span(&ctx, function),
            )
            .with_call_chain(chain)
            .with_similar_contexts(similar)
            .build();

            return Err(error);
        }
    }

    Ok(())
}
```

## Testing

The system includes comprehensive tests:

```bash
# Run all context error tests
cargo test -p verum_diagnostics --test context_error_tests

# Run specific test
cargo test -p verum_diagnostics test_context_not_declared_with_call_chain
```

**Test Coverage:**
- Simple error creation
- Call chain building and formatting
- Suggestion generation
- Levenshtein distance calculation
- "Did you mean" functionality
- Rendering with and without colors
- All error code variants (E0301-E0306)

## Examples

See `examples/context_error_demo.rs` for a complete demonstration:

```bash
cargo run -p verum_diagnostics --example context_error_demo
```

This shows all error types with realistic scenarios and beautiful formatted output.

## Performance

The context error system is designed for production use:

- **Levenshtein Distance**: O(nm) where n, m are string lengths, but strings are short (≤50 chars)
- **Call Chain Building**: O(depth) where depth is call stack depth
- **Suggestion Generation**: O(1) for most patterns
- **Rendering**: O(lines) for source snippet rendering

All operations complete in microseconds for typical programs.

## Design Principles

1. **User-First**: Error messages are written for humans, not compilers
2. **Actionable**: Every error includes concrete steps to fix
3. **Contextual**: Show the full picture (call chains, related errors)
4. **Progressive**: Simple cases get simple messages, complex cases get rich output
5. **Consistent**: All context errors follow the same format and style

## Future Enhancements

Potential improvements for future versions:

- [ ] Interactive mode: Let users pick which fix to apply
- [ ] Auto-fix generation: Automatically edit source files
- [ ] Machine learning: Learn from user fixes to improve suggestions
- [ ] Visual call graphs: Render call chains as diagrams
- [ ] Context flow analysis: Show data flow through context methods

## Specification Compliance

This implementation follows:
- **Spec**: `docs/detailed/16-context-system.md` - Context System
- **Audit**: `docs/audit.md` - Diagnostic recommendations
- **Style**: Rust-like error formatting (rustc, clippy)

All error codes (E0301-E0306) are reserved in the diagnostics error code space.

## License

Part of the Verum compiler infrastructure. See LICENSE in repository root.
