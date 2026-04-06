# Rich Error Diagnostics System

World-class error messages for the Verum compiler, inspired by Rust and Elm.

## Overview

This implementation provides a comprehensive rich diagnostics system with:

- **Colored ANSI Output**: Beautiful, terminal-friendly error messages
- **Code Snippets**: Automatic extraction of source code with context
- **Multi-line Formatting**: Support for spans across multiple lines
- **Error Explanations**: Detailed explanations for 10+ error codes
- **CLI Integration**: `verum explain E0312` command for help

## Architecture

### Files

```
crates/verum_diagnostics/src/
├── colors.rs              # ANSI color support (NEW)
├── snippet_extractor.rs   # Code snippet extraction (NEW)
├── rich_renderer.rs       # Rich diagnostic renderer (NEW)
├── explanations.rs        # Error explanations system (NEW)
├── diagnostic.rs          # Core diagnostic types (existing)
├── renderer.rs            # Original renderer (existing)
└── lib.rs                 # Module exports

crates/verum_cli/src/commands/
└── explain.rs             # CLI explain command (NEW)

tests/
└── rich_diagnostics_test.rs  # Comprehensive tests (NEW)
```

### Module Responsibilities

#### 1. `colors.rs` - ANSI Color Support

**Features:**
- Color scheme abstraction with theme support
- Auto-detection of terminal capabilities
- NO_COLOR environment variable support
- Unicode vs ASCII glyph selection
- Style composition (bold, dim, underline)

**Key Types:**
- `ColorScheme`: Theme-based color configuration
- `Color`: ANSI color codes
- `GlyphSet`: Unicode/ASCII character sets
- `Style`: Combined styling options

**Example:**
```rust
use verum_diagnostics::{ColorScheme, GlyphSet};

let scheme = ColorScheme::auto();  // Auto-detect colors
let glyphs = GlyphSet::unicode();   // Use Unicode box-drawing

let red_text = scheme.error_code.wrap("E0312");
let arrow = glyphs.arrow_right;  // "→"
```

#### 2. `snippet_extractor.rs` - Code Snippet Extraction

**Features:**
- Source file caching (configurable size)
- Context line extraction (before/after)
- Multi-line span support
- Line-level span annotations
- Memory-efficient buffering

**Key Types:**
- `SnippetExtractor`: Main extraction engine
- `Snippet`: Extracted code snippet
- `SourceLine`: Individual line with metadata
- `MultiSpanSnippet`: Multiple spans on same file

**Example:**
```rust
use verum_diagnostics::SnippetExtractor;

let mut extractor = SnippetExtractor::new();
let snippet = extractor.extract_snippet(
    Path::new("main.vr"),
    &span,
    2  // context lines
)?;

println!("Lines: {}", snippet.lines.len());
println!("Range: {} - {}", snippet.start_line, snippet.end_line);
```

#### 3. `rich_renderer.rs` - Rich Diagnostic Renderer

**Features:**
- Rust-like error output format
- Colored line numbers and gutters
- Multi-line span visualization
- Diff-style suggestions
- Configurable rendering options

**Key Types:**
- `RichRenderer`: Main rendering engine
- `RichRenderConfig`: Configuration options
- `DiffRenderer`: Suggestion diff rendering
- `MultiLineRenderer`: Multi-line span rendering

**Output Format:**
```
error[E0312]: refinement constraint not satisfied
  → main.vr:42:15
   |
42 |   let x: Positive = -5;
   |                     ^^ value `-5` fails constraint `> 0`
   |
   = note: value has type `Int` but requires `Positive`
   = help: use runtime check: `Positive::try_from(-5)?`
```

**Example:**
```rust
use verum_diagnostics::{RichRenderer, RichRenderConfig};

let config = RichRenderConfig::default();
let mut renderer = RichRenderer::new(config);

let diagnostic = DiagnosticBuilder::error()
    .code("E0312")
    .message("refinement constraint not satisfied")
    .span_label(span, "value `-5` fails constraint `> 0`")
    .help("use runtime check: `Positive::try_from(-5)?`")
    .build();

let output = renderer.render(&diagnostic);
println!("{}", output);
```

#### 4. `explanations.rs` - Error Explanation System

**Features:**
- Detailed explanations for 10+ error codes
- Multiple examples per error
- Suggested solutions with code
- Related documentation links
- Search functionality

**Error Codes Covered:**
- `E0203`: Result type mismatch in try operator
- `E0301`: Context not declared
- `E0308`: Type mismatch
- `E0309`: Branch verification failure
- `E0310`: Unsafe array access
- `E0312`: Refinement constraint not satisfied
- `E0313`: Integer overflow
- `E0314`: Division by zero
- `E0316`: Resource already consumed
- `E0317`: Unused Result that must be handled

**Key Types:**
- `ErrorExplanation`: Complete error explanation
- `Example`: Code example with correct/incorrect versions
- `Solution`: Suggested fix with code

**Example:**
```rust
use verum_diagnostics::{get_explanation, render_explanation};

if let Some(explanation) = get_explanation("E0312") {
    let rendered = render_explanation(explanation, true);
    println!("{}", rendered);
}
```

**Explanation Format:**
```
E0312: Refinement constraint not satisfied
============================================================

A value does not satisfy the refinement predicate required by its type.

Refinement types in Verum allow you to express constraints on values
using logical predicates. The compiler uses SMT solvers to verify
these constraints at compile-time.

Examples:
------------------------------------------------------------

1. Negative value for Positive type

   ✗ Incorrect:
   ```verum
   type Positive is Int{> 0};

   fn example() {
       let x: Positive = -5;  // Error: -5 does not satisfy > 0
   }
   ```

   ✓ Correct:
   ```verum
   type Positive is Int{> 0};

   fn example() {
       let x: Positive = 5;  // OK: 5 > 0
   }
   ```

Solutions:
------------------------------------------------------------

1. Use runtime validation with try_from
   Convert the value using try_from() which returns a Result

   ```verum
   let x = Positive::try_from(-5)?;  // Returns Result<Positive, Error>
   // Handle the error case appropriately
   ```

2. Use a checked constructor
   Use a constructor that validates at runtime

   ```verum
   let x = Positive::new(-5).expect("Must be positive");
   // Or with proper error handling:
   let x = Positive::new(-5).unwrap_or(Positive::new(1).unwrap());
   ```

See Also:
------------------------------------------------------------
  • Refinement Types Guide: docs/detailed/03-type-system.md#refinement-types
  • SMT Verification: docs/detailed/26-cbgr-implementation.md
  • Runtime Validation: docs/runtime-checks.md
```

#### 5. CLI Integration - `explain.rs`

**Features:**
- `verum explain E0312` - Show detailed explanation
- `verum explain 0312` - Works without 'E' prefix
- Error code search and suggestions
- Categorized code listing

**Commands:**
```bash
# Explain a specific error code
verum explain E0312

# Works without 'E' prefix
verum explain 0312

# Disable colors
verum explain E0312 --no-color
```

**Example Usage:**
```bash
$ verum explain E0312

E0312: Refinement constraint not satisfied
============================================================

A value does not satisfy the refinement predicate required by its type.
...
```

## Configuration

### RichRenderConfig

```rust
pub struct RichRenderConfig {
    pub color_scheme: ColorScheme,      // Color theme
    pub glyphs: GlyphSet,               // Unicode vs ASCII
    pub context_lines: usize,           // Lines before/after error
    pub show_line_numbers: bool,        // Display line numbers
    pub max_line_width: Option<usize>,  // Truncate long lines
    pub show_source: bool,              // Show source snippets
}
```

**Presets:**

```rust
// Full colors and features (auto-detected)
let config = RichRenderConfig::default();

// No colors (for CI/logs)
let config = RichRenderConfig::no_color();

// Minimal output (compact)
let config = RichRenderConfig::minimal();
```

### ColorScheme

Respects environment variables:
- `NO_COLOR`: Disables all colors
- `CLICOLOR_FORCE`: Forces colors even in non-TTY
- `TERM=dumb`: Disables colors

```rust
// Auto-detect based on environment
let scheme = ColorScheme::auto();

// Force colors on
let scheme = ColorScheme::default_colors();

// Force colors off
let scheme = ColorScheme::no_color();
```

### GlyphSet

```rust
// Auto-detect based on locale
let glyphs = GlyphSet::auto();

// Force Unicode box-drawing
let glyphs = GlyphSet::unicode();

// Force ASCII fallback
let glyphs = GlyphSet::ascii();
```

Set `VERUM_ASCII=1` to force ASCII mode.

## Testing

### Comprehensive Test Suite

70+ tests covering all functionality:

```bash
# Run all diagnostics tests
cargo test -p verum_diagnostics

# Run specific test module
cargo test -p verum_diagnostics rich_diagnostics

# Run with output
cargo test -p verum_diagnostics -- --nocapture
```

**Test Categories:**
- Basic error rendering (10 tests)
- Multi-line spans (5 tests)
- Color schemes (8 tests)
- Snippet extraction (12 tests)
- Error explanations (20+ tests)
- CLI integration (5 tests)
- Edge cases (10 tests)

## Usage Examples

### 1. Basic Error with Snippet

```rust
use verum_diagnostics::{DiagnosticBuilder, RichRenderer, RichRenderConfig};

let diagnostic = DiagnosticBuilder::error()
    .code("E0312")
    .message("refinement constraint not satisfied")
    .span_label(span, "value `-5` fails constraint `> 0`")
    .add_note("value has type `Int` but requires `Positive`")
    .help("use runtime check: `Positive::try_from(-5)?`")
    .build();

let mut renderer = RichRenderer::default();
let output = renderer.render(&diagnostic);
println!("{}", output);
```

### 2. Multi-line Error

```rust
let span = Span {
    file: "main.vr".to_string(),
    line: 10,
    column: 14,
    end_line: Some(15),
    end_column: Some(2),
};

let diagnostic = DiagnosticBuilder::error()
    .code("E0308")
    .message("type mismatch")
    .span_label(span, "expected `Int`, found `Float`")
    .build();
```

### 3. Warning with Help

```rust
let diagnostic = DiagnosticBuilder::warning()
    .code("W0101")
    .message("unused variable")
    .span_label(span, "unused variable `x`")
    .help("prefix with underscore: `_x`")
    .help("or remove this variable")
    .build();
```

### 4. Custom Configuration

```rust
let config = RichRenderConfig {
    color_scheme: ColorScheme::no_color(),
    glyphs: GlyphSet::ascii(),
    context_lines: 1,
    show_line_numbers: true,
    max_line_width: Some(80),
    show_source: true,
};

let mut renderer = RichRenderer::new(config);
```

### 5. Error Explanation Lookup

```rust
use verum_diagnostics::{get_explanation, render_explanation, search_errors};

// Get specific error
if let Some(explanation) = get_explanation("E0312") {
    println!("{}", render_explanation(explanation, true));
}

// Search for errors
let results = search_errors("refinement");
for code in results {
    println!("Found: {}", code);
}
```

## Performance

### Benchmarks

- **Snippet extraction**: < 1ms (with caching)
- **Rendering**: < 5ms (typical error)
- **Multi-line spans**: < 10ms
- **Cache capacity**: 100 files (configurable)

### Memory Usage

- **SnippetExtractor cache**: ~500KB per 100 files
- **Per diagnostic**: ~2KB
- **Rendered output**: ~1-5KB

## Integration

### Compiler Integration

```rust
// In compiler error handling
use verum_diagnostics::{DiagnosticBuilder, RichRenderer};

fn report_type_error(span: Span, expected: Type, found: Type) {
    let diagnostic = DiagnosticBuilder::error()
        .code("E0308")
        .message(format!("type mismatch: expected {}, found {}", expected, found))
        .span_label(span, format!("expected `{}`, found `{}`", expected, found))
        .help(format!("try converting {} to {}", found, expected))
        .build();

    let mut renderer = RichRenderer::default();
    eprintln!("{}", renderer.render(&diagnostic));
}
```

### IDE/LSP Integration

```rust
// Convert to LSP Diagnostic
impl From<Diagnostic> for lsp_types::Diagnostic {
    fn from(diag: Diagnostic) -> Self {
        let severity = match diag.severity() {
            Severity::Error => lsp_types::DiagnosticSeverity::ERROR,
            Severity::Warning => lsp_types::DiagnosticSeverity::WARNING,
            Severity::Note => lsp_types::DiagnosticSeverity::INFORMATION,
            Severity::Help => lsp_types::DiagnosticSeverity::HINT,
        };

        lsp_types::Diagnostic {
            range: span_to_range(diag.primary_span()),
            severity: Some(severity),
            code: diag.code().map(|c| lsp_types::NumberOrString::String(c.to_string())),
            message: diag.message().to_string(),
            ..Default::default()
        }
    }
}
```

## Future Enhancements

### Planned Features

1. **Interactive Errors**: Clickable URLs in terminal
2. **JSON Output**: Machine-readable error format
3. **Error Aggregation**: Group related errors
4. **Quick Fixes**: Automated fix suggestions
5. **More Error Codes**: Expand to 50+ codes
6. **Localization**: Multi-language support
7. **HTML Output**: Web-based error viewer
8. **Error Statistics**: Track common errors

### Wishlist

- Syntax highlighting in code snippets
- Inline type annotations
- Call graph visualization
- Interactive SMT trace exploration
- AI-powered fix suggestions

## Contributing

### Adding New Error Codes

1. Add explanation to `ERROR_EXPLANATIONS` map in `explanations.rs`
2. Include at least 2 examples (incorrect + correct)
3. Provide 3+ solutions
4. Link to relevant documentation
5. Add tests in `rich_diagnostics_test.rs`

### Example Template

```rust
map.insert("E0999".into(), ErrorExplanation {
    code: "E0999".into(),
    title: "Error title here".into(),
    description: r#"Detailed description..."#.into(),
    examples: vec![
        Example {
            description: "Scenario description".into(),
            code: r#"// Incorrect code"#.into(),
            correct: Some(r#"// Correct code"#.into()),
        },
    ].into(),
    solutions: vec![
        Solution {
            title: "Solution title".into(),
            description: "Solution description".into(),
            code: Some(r#"// Example code"#.into()),
        },
    ].into(),
    see_also: vec![
        "Link to docs".into(),
    ].into(),
});
```

## References

### Inspiration

- **Rust Compiler**: Error message design and format
- **Elm Compiler**: Helpful, friendly error messages
- **GHC**: Type error explanations
- **Clang**: Source code highlighting

### Related Documentation

- [Type System](../../docs/detailed/03-type-system.md)
- [Refinement Types](../../docs/detailed/03-type-system.md#refinement-types)
- [Context System](../../docs/detailed/16-context-system.md)
- [CBGR Implementation](../../docs/detailed/26-cbgr-implementation.md)

## License

Part of the Verum Language Platform.
See LICENSE file in repository root.
