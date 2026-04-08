# Context Runtime Tests

This directory contains VBC-specific integration tests for the context system that verify the VBC bytecode instructions are emitted and executed correctly.

## VBC Context Instructions

The context system uses three core VBC instructions:

| Instruction | Opcode | Purpose |
|-------------|--------|---------|
| `CtxProvide` | `0xB1` | Push a context value onto the context stack |
| `CtxGet` | `0xB0` | Retrieve a context value by type ID |
| `CtxEnd` | `0xB2` | Clean up contexts at scope boundary |

## Test Coverage

| Test File | Description | Key Instructions |
|-----------|-------------|------------------|
| `context_basic_provide.vr` | Basic provide statement | `CtxProvide`, `CtxGet` |
| `context_method_call.vr` | Method calls on contexts | `CtxGet`, `CallM` |
| `context_shadowing.vr` | Nested provide overrides | `CtxProvide`, `CtxEnd` |
| `context_multiple.vr` | Multiple contexts together | Multiple `CtxProvide`/`CtxGet` |
| `context_propagation.vr` | Context flow through calls | `CtxGet` in call chain |
| `context_typecheck.vr` | Type checking validation | (typecheck-pass only) |

## Context Wiring Architecture

When a function declares `using [Logger, Database]`:

1. **Codegen** stores context names in `CodegenContext.required_contexts`
2. **Method calls** like `Logger.info(msg)` check if receiver is a required context
3. If yes, `CtxGet` is emitted to retrieve the context value from the stack
4. The method is then called on the retrieved value via `CallM`

## Implementation References

- `crates/verum_vbc/src/codegen/context.rs` - `required_contexts` field
- `crates/verum_vbc/src/codegen/expressions.rs` - Method call context check
- `crates/verum_vbc/src/interpreter/state.rs` - ContextStack implementation
- `crates/verum_vbc/src/interpreter/dispatch.rs` - Context instruction dispatch
