# Unified Error Type Migration Guide

## Overview

The unified error type system consolidates all error types from across the Verum platform into a single `VerumError` enum. This eliminates the proliferation of incompatible Result type aliases and enables seamless error propagation across crate boundaries.

## Problem Statement

Previously, we had 6+ different incompatible Result types:

```rust
// 6 different Result types that don't interoperate:
verum_cbgr::Result<T>         = Result<T, Error>
verum_runtime::Result<T>      = Result<T, RuntimeError>
verum_smt::Result<T>          = Result<T, Error>
verum_types::Result<T>        = Result<T, TypeError>
verum_verification::Result<T> = Result<T, VerificationError>
verum_parser::ParseResult<T>  = Result<Vec<ParseError>, T>  // backwards!
```

This caused problems:
- Cannot use `?` operator across crate boundaries
- Forced to use `.map_err(|e| ...)` conversions everywhere
- Complex error handling at boundaries
- No unified error handling strategy

## Solution

Single unified error type with automatic conversion:

```rust
use verum_error::unified::{VerumError, Result};

fn cross_crate_operation() -> Result<()> {
    // Each ? automatically converts the specific error type
    perform_cbgr_check()?;     // From verum_cbgr::Error
    verify_types()?;            // From TypeError
    run_smt_solver()?;          // From verum_smt::Error
    Ok(())
}
```

## Error Categories

The unified `VerumError` enum has **26 variants** organized into these categories:

### CBGR Memory Safety Errors
- `UseAfterFree` - Generation counter mismatch
- `DoubleFree` - Freeing already-freed memory
- `NullPointer` - Dereferencing null pointer
- `OutOfBounds` - Array/slice bounds violation
- `CapabilityViolation` - Operation not permitted by capabilities

### Runtime Errors
- `ContextNotFound` - Missing execution context
- `TaskPanicked` - Async task panicked
- `ExecutionError` - General runtime execution error
- `StackOverflow` - Call stack depth exceeded

### Type System Errors
- `TypeMismatch` - Expected one type, found another
- `CannotInferLambda` - Lambda needs type annotation
- `UnboundVariable` - Variable used before definition
- `NotAFunction` - Called a non-function value
- `InfiniteType` - Recursive type detected
- `ProtocolNotSatisfied` - Type doesn't implement required protocol
- `RefinementFailed` - Value doesn't satisfy refinement predicate
- `AffineViolation` - Affine value used multiple times
- `MissingContext` - Required context not available

### SMT/Verification Errors
- `VerificationTimeout` - SMT solver timed out
- `VerificationFailed` - Could not prove property
- `UnsupportedSMT` - Unsupported SMT feature

### Parse Errors
- `ParseErrors` - List of parse errors
- `LexError` - Lexical analysis error

### I/O and System Errors
- `IoError` - File system or I/O failure
- `NetworkError` - Network operation failure
- `ConfigError` - Invalid or missing configuration
- `Timeout` - Operation exceeded time limit

### Generic
- `Other` - Catch-all for miscellaneous errors
- `NotImplemented` - Feature not yet implemented

## Migration Strategy

### Step 1: Understand Your Current Errors

Identify which crate-specific error types you're using:

```rust
// Current code (crate-specific errors)
use verum_types::TypeError;
use verum_cbgr::Error as CbgrError;

fn my_function() -> std::result::Result<(), TypeError> {
    // ...
}
```

### Step 2: Add Unified Error Import

```rust
// New code (unified errors)
use verum_error::unified::{VerumError, Result};

fn my_function() -> Result<()> {
    // Automatic conversion from TypeError via From trait
    type_check_something()?;  // Returns Result<_, TypeError>

    // Automatic conversion from CbgrError
    cbgr_operation()?;  // Returns Result<_, CbgrError>

    Ok(())
}
```

### Step 3: Update Function Signatures Gradually

You can migrate incrementally:

```rust
// Option A: Keep crate-specific errors internally
fn internal_function() -> verum_types::Result<Type> {
    // Uses TypeError
}

// But use unified errors at boundaries
pub fn public_api() -> verum_error::unified::Result<Type> {
    internal_function().map_err(Into::into)  // Explicit conversion
}
```

```rust
// Option B: Full migration
use verum_error::unified::Result;

fn internal_function() -> Result<Type> {
    type_check()?;  // Automatic conversion
    Ok(ty)
}

pub fn public_api() -> Result<Type> {
    internal_function()  // Already unified
}
```

### Step 4: Implement Conversions (if needed)

Most conversions are already implemented in `crates/verum_error/src/conversions.rs`. If you need to add more:

```rust
impl From<MyCrateError> for VerumError {
    fn from(err: MyCrateError) -> Self {
        match err {
            MyCrateError::Foo { .. } => VerumError::Other {
                message: err.to_string().into(),
            },
            // Map to appropriate unified variant
        }
    }
}
```

## When to Use Unified Errors

### ✅ Use Unified Errors When:

1. **Cross-crate boundaries** - Functions that call multiple crates
2. **Public APIs** - External-facing functions
3. **CLI/binary crates** - Top-level error handling
4. **Error aggregation** - Collecting errors from multiple sources

### ❌ Keep Crate-Specific Errors When:

1. **Internal implementations** - Within a single crate
2. **Type-specific contexts** - When you need detailed type error information
3. **Backward compatibility** - Existing public APIs

## Examples

### Example 1: CLI Error Handling

```rust
use verum_error::unified::{VerumError, Result};
use verum_cli::CommandArgs;

fn main() -> Result<()> {
    let args = CommandArgs::parse();

    // All operations automatically convert errors
    let source = std::fs::read_to_string(&args.file)?;  // IoError
    let ast = parse(&source)?;                          // ParseErrors
    let typed = type_check(ast)?;                       // Type errors
    verify(typed)?;                                     // Verification errors

    Ok(())
}
```

### Example 2: Cross-Module Function

```rust
use verum_error::unified::{VerumError, Result};

pub fn compile_and_verify(source: &str) -> Result<CompiledOutput> {
    // Parse
    let ast = verum_parser::parse(source)?;  // Converts ParseErrors

    // Type check
    let typed = verum_types::check(&ast)?;   // Converts TypeError

    // Verify
    verum_smt::verify(&typed)?;               // Converts SMT Error

    // CBGR analysis
    let analyzed = verum_cbgr::analyze(&typed)?;  // Converts CBGR Error

    // Codegen
    let output = verum_codegen::generate(&analyzed)?;

    Ok(output)
}
```

### Example 3: Error Pattern Matching

```rust
use verum_error::unified::VerumError;

match compile_and_verify(source) {
    Ok(output) => println!("Success: {}", output),
    Err(VerumError::ParseErrors(errors)) => {
        eprintln!("Parse errors:");
        for err in errors.iter() {
            eprintln!("  - {}", err.as_str());
        }
    }
    Err(VerumError::TypeMismatch { expected, actual }) => {
        eprintln!("Type error: expected {}, got {}", expected.as_str(), actual.as_str());
    }
    Err(VerumError::VerificationFailed { reason, counterexample }) => {
        eprintln!("Verification failed: {}", reason.as_str());
        if let Some(cex) = counterexample {
            eprintln!("Counterexample: {}", cex.as_str());
        }
    }
    Err(e) if e.is_fatal() => {
        eprintln!("FATAL ERROR: {}", e);
        std::process::exit(1);
    }
    Err(e) if e.is_recoverable() => {
        eprintln!("Recoverable error: {} (will retry)", e);
        // Retry logic
    }
    Err(e) => {
        eprintln!("Error: {}", e);
    }
}
```

### Example 4: Custom Error Creation

```rust
use verum_error::unified::VerumError;

fn validate_config(config: &Config) -> Result<()> {
    if config.workers == 0 {
        return Err(VerumError::ConfigError {
            message: "workers must be > 0".into(),
        });
    }

    if config.timeout_ms > 60_000 {
        return Err(VerumError::Timeout {
            timeout_ms: config.timeout_ms,
        });
    }

    Ok(())
}
```

## Error Context and Chaining

The unified error type works with the existing error context system:

```rust
use verum_error::unified::Result;
use verum_error::context::ErrorContext;

fn process_file(path: &str) -> Result<()> {
    let contents = std::fs::read_to_string(path)
        .context(format!("Failed to read file: {}", path))?;

    let ast = parse(&contents)
        .context("Failed to parse source code")?;

    Ok(())
}
```

## Compatibility Notes

### Backward Compatibility

The unified error system is **fully backward compatible**:

- All existing crate-specific error types remain unchanged
- Existing code continues to work without modifications
- Migration can happen gradually, crate by crate

### Conversion Performance

Error conversions via `From` trait have **zero overhead** when not used (errors are rare path) and minimal overhead when used:

- Direct variant mapping: ~0ns (compiler optimizes away)
- String conversion: ~10-50ns (only on error path)

## Testing

Test both the happy path and error cases:

```rust
#[test]
fn test_unified_error_conversion() {
    use verum_error::unified::VerumError;

    // Test automatic conversion
    let result: Result<(), VerumError> = (|| {
        type_check_something()?;  // Returns TypeError
        Ok(())
    })();

    assert!(result.is_err());
    match result.unwrap_err() {
        VerumError::TypeMismatch { .. } => {}, // Expected
        e => panic!("Unexpected error: {}", e),
    }
}
```

## FAQ

### Q: Do I need to convert all my code immediately?

**A:** No. The unified error system is additive. Existing crate-specific errors work as before. Migrate gradually starting with boundary functions.

### Q: What if I need more detailed error information?

**A:** Keep using crate-specific errors internally. Use unified errors at boundaries. You can always pattern match on unified errors to extract details.

### Q: How do I add a new error variant?

**A:** Add it to `VerumError` enum in `crates/verum_error/src/unified.rs` and implement the `From` trait for your crate's error type in `conversions.rs`.

### Q: Does this affect performance?

**A:** No. Error conversions are only executed on the error path (which is rare). The happy path has zero overhead.

### Q: Can I still use `thiserror` or `anyhow` in my crate?

**A:** Yes, but for Verum crates, prefer using the unified error system for consistency.

## Summary

| Aspect | Before | After |
|--------|--------|-------|
| **Error Types** | 6+ incompatible types | Single unified type |
| **Conversion** | Manual `.map_err()` | Automatic via `From` |
| **Cross-crate** | Complex error handling | Seamless `?` operator |
| **Pattern Matching** | Crate-specific variants | Unified variants |
| **Migration** | N/A | Gradual, backward compatible |
| **Performance** | N/A | Zero overhead (happy path) |

## Next Steps

1. Start using unified errors in new code
2. Migrate boundary functions (public APIs)
3. Gradually refactor internal code
4. Add new error variants as needed
5. Update tests to use unified errors

## Resources

- Unified error definition: `crates/verum_error/src/unified.rs`
- Conversion implementations: `crates/verum_error/src/conversions.rs`
- Error handling spec: `docs/detailed/20-error-handling.md`
