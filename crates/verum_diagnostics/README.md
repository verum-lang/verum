# Verum Diagnostics System

A comprehensive, world-class diagnostics system for the Verum compiler, with special emphasis on refinement type errors.

## Overview

The Verum diagnostics system provides beautiful, actionable error messages that guide developers through understanding and fixing issues in their code. It excels at explaining refinement type violations with clear examples and multiple fix suggestions.

## Features

- **Rich Error Messages**: Beautiful, colored output with source code context
- **Refinement Type Specialization**: Shows actual values that failed constraints
- **SMT Trace Integration**: Displays verification paths and counterexamples
- **Actionable Suggestions**: Multiple fix options with code examples
- **Multi-format Output**: Human-readable text and machine-readable JSON
- **Error Chains**: Full context propagation through the compiler
- **IDE Integration**: JSON output for seamless tool integration

## Architecture

The system consists of seven main modules:

### 1. `diagnostic.rs` - Core Types
- `Severity`: Error, Warning, Note, Help levels
- `Span`: Source location tracking
- `Diagnostic`: Complete diagnostic message
- `DiagnosticBuilder`: Fluent API for building diagnostics

### 2. `refinement_error.rs` - Refinement Type Specialization
- `Constraint`: Refinement type constraints
- `ConstraintViolation`: Detailed violation information
- `SMTTrace`: Verification reasoning traces
- `CounterExample`: SMT solver counterexamples
- `RefinementError`: Specialized refinement errors

### 3. `suggestion.rs` - Fix Suggestions
- `Suggestion`: Actionable fix with code examples
- `Applicability`: Recommended, Alternative, MaybeIncorrect
- `CodeSnippet`: Example code for fixes
- Templates for common fixes

### 4. `renderer.rs` - Pretty Printing
- Beautiful source code rendering
- Color-coded output
- Line numbers and gutters
- Multi-line span support

### 5. `emitter.rs` - Output Formatting
- Human-readable text output
- JSON output for IDEs
- Diagnostic accumulation
- Summary statistics

### 6. `context.rs` - Error Context
- `DiagnosticContext`: Compilation stage tracking
- `ErrorChain`: Error propagation paths
- `Backtrace`: Call stack information

### 7. `lib.rs` - Public API
- Re-exports all public types
- Error codes (`E0308`, `E0312`, etc.)
- Warning codes (`W0101`, etc.)

## Critical v1.0 Feature: Refinement Type Errors

The v1.0 release MUST support this error format:

```rust
use verum_diagnostics::*;

let error = refinement_error::common::positive_constraint_violation(
    "x",
    "-5",
    Span::new("main.vr", 3, 12, 13),
);

let diagnostic = error.to_diagnostic();
let mut renderer = Renderer::default();
let output = renderer.render(&diagnostic);
```

Output:
```
error<E0312>: refinement constraint not satisfied
  --> main.vr:3:12
   |
 3 | divide(10, x)
   |            ^ value `-5` fails constraint `x > 0`
   |
   = help: wrap in runtime check: `PositiveInt::try_from(x)?`
   = help: or use compile-time proof: `@verify x > 0`
```

## Usage Examples

### Basic Error

```rust
use verum_diagnostics::*;

let diagnostic = DiagnosticBuilder::error()
    .code(error_codes::E0308)
    .message("verification failed")
    .span_label(
        Span::new("math.vr", 2, 5, 12),
        "Cannot prove postcondition: result >= 0"
    )
    .add_note("SMT solver found counterexample")
    .help("Add precondition: x: Float{>= 0}")
    .build();

let mut emitter = Emitter::default();
emitter.emit_stderr(&diagnostic)?;
```

### Refinement Error with SMT Trace

```rust
use verum_diagnostics::*;

let trace = SMTTrace::new(false)
    .add_step(VerificationStep::new(1, "assumed", "x >= 0"))
    .add_step(VerificationStep::new(2, "checking", "x / 2 >= 0"))
    .add_step(VerificationStep::new(3, "failed", "Cannot prove"))
    .with_counterexample(
        CounterExample::new().add_assignment("x", "-4.0")
    );

let error = RefinementErrorBuilder::new()
    .constraint("x >= 0")
    .actual_value("-4.0")
    .span(Span::new("math.vr", 10, 15, 16))
    .trace(trace)
    .suggestion_obj(suggestion::templates::add_refinement_constraint("x", ">= 0"))
    .build();

let diagnostic = error.to_diagnostic();
```

### JSON Output for IDEs

```rust
use verum_diagnostics::*;

let mut emitter = Emitter::new(EmitterConfig::json());
emitter.add(diagnostic);

let mut output = Vec::new();
emitter.emit_all(&mut output)?;

// Output is valid JSON with spans, labels, and suggestions
```

### Common Refinement Errors

```rust
use verum_diagnostics::refinement_error::common;

// Division by zero
let error = common::division_by_zero("x", "0", span);

// Array bounds violation
let error = common::bounds_check_violation("arr", "idx", "10", "5", span);

// Range constraint violation
let error = common::range_violation("x", "150", "0", "100", span);

// Positive constraint violation
let error = common::positive_constraint_violation("x", "-5", span);
```

## Error Codes

### Errors (E-prefixed)
- `E0308`: Verification failed - postcondition
- `E0309`: Branch verification failure
- `E0310`: Unsafe array access
- `E0311`: Type mismatch
- `E0312`: Refinement constraint not satisfied
- `E0313`: Integer overflow
- `E0314`: Division by zero
- `E0315`: Null pointer dereference
- `E0316`: Resource already consumed
- `E0317`: Invalid capability use

### Warnings (W-prefixed)
- `W0101`: Unused variable
- `W0102`: Unused import
- `W0103`: Dead code
- `W0104`: Unnecessary refinement
- `W0105`: False positive detected

## Testing

The crate includes comprehensive tests:

```bash
# Run all tests
cargo test

# Run only unit tests
cargo test --lib

# Run only integration tests
cargo test --test integration_test

# Run with output
cargo test -- --nocapture
```

### Test Coverage

- **43 unit tests** across all modules
- **15 integration tests** demonstrating full system
- All critical v1.0 features tested
- Edge cases and error conditions covered

## Design Principles

1. **Clarity First**: Error messages should be immediately understandable
2. **Actionable**: Every error includes concrete fix suggestions
3. **Educational**: Messages teach verification principles
4. **Beautiful**: Output is visually appealing and well-structured
5. **Consistent**: All errors follow the same format
6. **Machine-Readable**: JSON output for tool integration

## Integration Points

### Compiler Integration

```rust
// In the type checker
let diag = DiagnosticBuilder::error()
    .code(error_codes::E0311)
    .message(format!("type mismatch: expected {}, found {}", expected, found))
    .span_label(span, "type error occurs here")
    .build();

emitter.add(diag);
```

### SMT Solver Integration

```rust
// When SMT verification fails
if let Some(counterexample) = smt_result.counterexample() {
    let trace = build_smt_trace(smt_result);
    let error = RefinementErrorBuilder::new()
        .constraint(constraint)
        .actual_value(counterexample.value)
        .span(span)
        .trace(trace)
        .build();

    emitter.add(error.to_diagnostic());
}
```

### IDE Integration via LSP

The JSON output format is designed for Language Server Protocol integration:

```json
{
  "level": "error",
  "code": "E0312",
  "message": "refinement constraint not satisfied",
  "spans": [{
    "location": {
      "file": "main.vr",
      "line": 3,
      "column": 12,
      "length": 1
    },
    "label": "value `-5` fails constraint `x > 0`",
    "is_primary": true
  }],
  "helps": [
    "wrap in runtime check: `PositiveInt::try_from(x)?`",
    "or use compile-time proof: `@verify x > 0`"
  ]
}
```

## Performance Considerations

- Source files are cached to avoid repeated I/O
- Diagnostics can be accumulated and emitted in batch
- Renderer is optimized for typical source file sizes
- JSON serialization is lazy (only when needed)

## Future Enhancements

- Multi-file diagnostics with cross-references
- Interactive fix application
- Diagnostic explanations (--explain flag)
- Custom diagnostic plugins
- Performance profiling integration
- More SMT solver trace formats

## License

Part of the Verum compiler project.
