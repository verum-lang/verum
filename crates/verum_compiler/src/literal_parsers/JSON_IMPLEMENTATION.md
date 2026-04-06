# JSON Literal Parser Implementation

## Overview

The JSON literal parser for Verum provides compile-time validation and parsing of JSON literals using the `serde_json` library.

## Location

- **Implementation**: `crates/verum_compiler/src/literal_parsers/json.rs`
- **Tests**: `crates/verum_compiler/tests/json_tests.rs`

## Features

### Full JSON Support

The parser handles all JSON types as defined in RFC 8259:

1. **Objects**: `{"key": "value", "nested": {"inner": 123}}`
2. **Arrays**: `[1, 2, 3, "mixed", true, null]`
3. **Strings**: `"hello world"`
4. **Numbers**: `42`, `3.14`, `-1.5`, `1.23e10`, `4.56E-5`
5. **Booleans**: `true`, `false`
6. **Null**: `null`

### String Escape Sequences

All standard JSON escape sequences are supported:

- `\"` - Double quote
- `\\` - Backslash
- `\/` - Forward slash
- `\b` - Backspace
- `\f` - Form feed
- `\n` - Newline
- `\r` - Carriage return
- `\t` - Tab
- `\uXXXX` - Unicode escape sequences (e.g., `\u0048` for 'H')

### Number Support

- **Integers**: Any size within JSON spec
- **Floating-point**: Full precision
- **Scientific notation**: `1.23e10`, `4.56E-5`, `-7.89e+2`
- **Negative numbers**: `-42`, `-3.14`, `-1e-10`

### Nesting

Supports arbitrary nesting depth for both objects and arrays:

```json
{
  "level1": {
    "level2": {
      "level3": {
        "array": [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]
      }
    }
  }
}
```

### Validation

The parser performs strict JSON validation and rejects:

- **Trailing commas**: `{"key": "value",}` ❌
- **Unquoted keys**: `{key: "value"}` ❌
- **Single quotes**: `{'key': 'value'}` ❌
- **Comments**: `{"key": "value" /* comment */}` ❌
- **Undefined**: `{"value": undefined}` ❌
- **NaN/Infinity**: `{"nan": NaN, "inf": Infinity}` ❌
- **Unescaped control characters**: `{"key": "value\u0001"}` ❌

### Error Reporting

The parser provides detailed error messages with:

- Line and column numbers
- User-friendly error descriptions
- Source location via Span
- Integration with Verum diagnostics system

Example error:
```
Invalid JSON at line 3, column 15: unexpected end of input
```

## Implementation Details

### Architecture

```
parse_json()
├─ Trim whitespace
├─ Check for empty input
├─ Parse with serde_json::from_str()
│  └─ Returns serde_json::Value
├─ On success: Return ParsedLiteral::Json(Text)
└─ On error: Build detailed diagnostic with position info
```

### Design Decisions

1. **Use serde_json**: Battle-tested, RFC 8259 compliant, widely used
2. **Store Text, not Value**: Preserves original formatting for codegen
3. **Strict validation**: No leniency beyond standard JSON
4. **Detailed errors**: Extract line/column from serde_json errors

### Performance

- **Parse time**: O(n) where n is JSON string length
- **Memory**: Single Text allocation + temporary serde_json::Value
- **Validation**: Happens at compile-time, zero runtime cost

## Usage

### In Verum Code

```verum
let config = json#"{
  \"database\": {
    \"host\": \"localhost\",
    \"port\": 5432
  }
}";
```

### In Compiler

```rust
use verum_compiler::literal_parsers::json::parse_json;
use verum_core::Text;
use verum_ast::Span;

let result = parse_json(
    &Text::from(r#"{"key": "value"}"#),
    span,
    None
);

match result {
    Ok(ParsedLiteral::Json(text)) => {
        // Use validated JSON text
    }
    Err(diagnostic) => {
        // Handle error
    }
}
```

## Testing

### Test Coverage

The test suite covers:

- ✓ Basic JSON types (object, array, string, number, boolean, null)
- ✓ Complex nested structures
- ✓ All escape sequences
- ✓ Unicode characters
- ✓ Scientific notation
- ✓ Large numbers and decimal precision
- ✓ Empty structures
- ✓ Whitespace handling
- ✓ Error cases (invalid syntax, trailing commas, etc.)

### Running Tests

```bash
cargo test --test json_tests
```

### Test Statistics

- **Total tests**: 33
- **Valid JSON tests**: 24
- **Invalid JSON tests**: 9
- **Coverage**: All JSON features and edge cases

## Dependencies

- `serde_json` (0.1): JSON parsing and validation
- `verum_core`: Text type
- `verum_ast`: Span and SourceFile
- `verum_diagnostics`: Error reporting

## Spec Compliance

Implements: **05-syntax-grammar.md §1.4.4** - Semantic Literals

Fully compliant with:
- RFC 8259 (JSON specification)
- Verum semantic literal syntax

## Future Enhancements

Potential improvements (not currently required):

1. JSON Schema validation at compile-time
2. Custom error messages for common mistakes
3. Pretty-print validation errors with context
4. Support for streaming JSON for very large literals
5. JSON5 support (with feature flag)

## Related Files

- `literal_registry.rs`: ParsedLiteral::Json enum variant
- `macro_expansion.rs`: JSON literal expansion
- `literal_parsers_tests.rs`: Additional integration tests
