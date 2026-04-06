# Terminology Migration Patterns: "effect" → "context"

## Overview

This document lists all the patterns used in the automatic migration from "effect" terminology to "context" terminology in the Verum codebase.

## Replacement Patterns

### 1. Type Names (CamelCase)

| Old Pattern | New Pattern | Context | Example |
|------------|-------------|---------|---------|
| `EffectSet` | `ContextSet` | Type definition | `pub struct EffectSet` → `pub struct ContextSet` |
| `EffectInferenceContext` | `ContextInferenceEngine` | Type definition | `pub struct EffectInferenceContext` → `pub struct ContextInferenceEngine` |
| `Effect` | `Context` | Type/enum name (standalone) | `pub enum Effect` → `pub enum Context` |

**Regex Patterns:**
- `EffectSet` → `ContextSet`
- `EffectInferenceContext` → `ContextInferenceEngine`
- `Effect(?!Set)(?!Inference)` → `Context` (negative lookahead to avoid double-replacement)

### 2. Function Names (snake_case)

| Old Pattern | New Pattern | Context | Example |
|------------|-------------|---------|---------|
| `has_effect` | `has_context` | Method call | `if env.has_effect()` → `if env.has_context()` |
| `add_effect` | `add_context` | Method call | `ctx.add_effect(e)` → `ctx.add_context(e)` |
| `add_effects` | `add_contexts` | Method call | `ctx.add_effects(set)` → `ctx.add_contexts(set)` |
| `get_effects` | `get_contexts` | Method call | `let effects = ctx.get_effects()` → `let contexts = ctx.get_contexts()` |
| `take_effects` | `take_contexts` | Method call | `ctx.take_effects()` → `ctx.take_contexts()` |
| `is_effect` | `is_context` | Predicate | `if is_effect(x)` → `if is_context(x)` |
| `effect_set` | `context_set` | Field name | `self.effect_set` → `self.context_set` |

**Regex Patterns:**
- `has_effect\b` → `has_context`
- `add_effect\b` → `add_context`
- `add_effects\b` → `add_contexts`
- `get_effects\b` → `get_contexts`
- `take_effects\b` → `take_contexts`
- `is_effect\b` → `is_context`
- `effect_set\b` → `context_set`

### 3. Variable Names (snake_case)

| Old Pattern | New Pattern | Context | Example |
|------------|-------------|---------|---------|
| `effects` | `contexts` | Variable name | `let effects = ...` → `let contexts = ...` |
| `effect` | `context` | Variable name | `for effect in list` → `for context in list` |
| `current_effects` | `current_contexts` | Field name | `self.current_effects` → `self.current_contexts` |

**Regex Patterns:**
- `\beffects\b(?!::|\\.)` → `contexts` (word boundary, not followed by :: or .)
- `\beffect\b(?!s\b)(?!::|\\.)` → `context` (singular, word boundary)
- `current_effects\b` → `current_contexts`

### 4. Module and Documentation Terms

| Old Pattern | New Pattern | Context | Example |
|------------|-------------|---------|---------|
| `effect system` | `context system` | Documentation | `//! Effect system for Verum` → `//! Context system for Verum` |
| `Effect system` | `Context system` | Documentation (capitalized) | `/// Effect system tracks...` → `/// Context system tracks...` |
| `effect tracking` | `context tracking` | Documentation | Comments about tracking |

**Regex Patterns:**
- `effect system` → `context system`
- `Effect system` → `Context system`

### 5. File Names

| Old Filename | New Filename | Location |
|-------------|--------------|----------|
| `effects.rs` | `contexts.rs` | `crates/verum_types/src/` |
| `effect_*.rs` | `context_*.rs` | Various (if any exist) |

### 6. Import/Export Statements

| Old Pattern | New Pattern | Example |
|------------|-------------|---------|
| `mod effects;` | `mod contexts;` | `mod effects;` → `mod contexts;` |
| `pub use effects::` | `pub use contexts::` | `pub use effects::{Effect, EffectSet}` → `pub use contexts::{Context, ContextSet}` |
| `use ...::effects::` | `use ...::contexts::` | Import paths |

## Exclusion Patterns

These patterns are **NOT** replaced to preserve legitimate terminology:

### 1. Side Effects (General CS Term)

| Pattern | Reason | Examples |
|---------|--------|----------|
| `side effect` | General computer science terminology | "Pure functions have no side effects" |
| `side-effect` | Hyphenated variant | "Side-effect free code" |
| `no side effect` | Common phrase | "This operation has no side effect" |

**Regex to exclude:**
- `side[\s-]*effect` (case insensitive)

### 2. FFI/External Effects

| Pattern | Reason | Examples |
|---------|--------|----------|
| `memory effect` | FFI boundary concept | "Memory effects at FFI boundary" |
| `FFI effect` | FFI-specific terminology | "FFI effect tracking" |

**Files to handle carefully:**
- `crates/verum_runtime/src/ffi/memory_effects.rs` (may keep as-is or rename selectively)
- `crates/verum_ast/src/ffi.rs` (contains FFI effect annotations)

### 3. External Library References

Any references to external libraries or their APIs should be preserved.

### 4. String Literals

String literals containing "effect" are checked contextually - only Verum-specific ones are replaced.

## Implementation Notes

### Perl Regex Features Used

The script uses Perl regular expressions for advanced features:

1. **Negative Lookahead**: `Effect(?!Set)(?!Inference)` - matches "Effect" only when NOT followed by "Set" or "Inference"
2. **Word Boundaries**: `\b` - ensures we match whole words only
3. **Non-capturing Groups**: `(?:...)` - groups without capturing
4. **Case-sensitive matching**: Preserves PascalCase vs snake_case distinctions

### Processing Order

Replacements are applied in this order to prevent partial replacements:

1. Longest patterns first (`EffectInferenceContext` before `Effect`)
2. Compound names before simple names (`EffectSet` before `Effect`)
3. CamelCase before snake_case (types before variables)
4. Specific functions before general terms

### Line-by-Line Processing

The script processes files line-by-line to:
- Apply exclusion rules per line
- Preserve comments with "side effect"
- Handle multi-line patterns correctly
- Maintain exact formatting and indentation

## Validation

### Pre-replacement Checks

Before applying replacements, the script:
1. Creates full backup of all modified files
2. Logs all planned changes

### Post-replacement Validation

After applying replacements:
1. Runs `cargo check --all-targets --all-features`
2. Runs `cargo test --lib`
3. Generates detailed report of all changes
4. Provides rollback mechanism if validation fails

## Manual Review Checklist

After running the script, manually review:

- [ ] `crates/verum_types/src/contexts.rs` - main context system implementation
- [ ] `crates/verum_types/src/lib.rs` - updated exports
- [ ] `crates/verum_ast/src/ffi.rs` - FFI effects (may intentionally keep)
- [ ] `crates/verum_runtime/src/ffi/memory_effects.rs` - memory effects (may intentionally keep)
- [ ] All test files - ensure test names make sense
- [ ] Documentation comments - verify readability
- [ ] Error messages - ensure user-facing text is clear

## Known Edge Cases

### 1. Compound Words

Some compound words need special handling:
- `effect_set` → `context_set` (yes)
- `side_effect` → `side_effect` (no, keep as-is)

### 2. Generic Type Parameters

Generic type parameters named `E` for "Effect" should be considered:
- `E` in `Result<T, E>` - keep as-is (standard Rust)
- `E` in custom types - evaluate context

### 3. Comments and Documentation

Comments may need manual review for:
- Clarity after replacement
- Consistency with new terminology
- Appropriate use of "side effect" vs "context"

## Statistics Collection

The script collects and reports:
- Total files scanned
- Files modified
- Total replacements made per pattern
- Compilation success/failure
- Test results

## Rollback Procedure

If issues are found, rollback using:

```bash
./scripts/migrate_effect_to_context.sh --rollback
```

Or manually:

```bash
# List available backups
ls -la backups/

# Restore from specific backup
rsync -av --delete backups/terminology_migration_YYYYMMDD_HHMMSS/ .
```

## Future Enhancements

Potential improvements to the migration script:

1. **Semantic Analysis**: Use syn/rust-analyzer to parse AST and perform semantic replacements
2. **Interactive Mode**: Preview changes and approve/reject individually
3. **Partial Migration**: Support migrating specific crates only
4. **Documentation Update**: Automatically update Markdown documentation
5. **Git Integration**: Create commits with detailed change descriptions
6. **Conflict Detection**: Detect and warn about ambiguous replacements

## References

- Verum Specification: `docs/03-type-system.md` (Section 4 - Context System)
- CLAUDE.md: v6.0-BALANCED terminology standards
- Related Issue: Terminology unification project
