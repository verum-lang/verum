# Terminology Migration Guide: "effect" → "context"

## Quick Start

### Basic Usage

```bash
# 1. Dry run to preview changes
./scripts/migrate_effect_to_context.sh --dry-run

# 2. Run the actual migration (creates backup automatically)
./scripts/migrate_effect_to_context.sh

# 3. If something goes wrong, rollback
./scripts/migrate_effect_to_context.sh --rollback
```

## Prerequisites

- Bash 4.0+ (check with `bash --version`)
- Perl 5.10+ (check with `perl --version`)
- Git (for file renaming, optional)
- Rust toolchain (for validation)

## Detailed Usage

### Step-by-Step Migration Process

#### 1. Pre-Migration Checklist

Before running the migration:

- [ ] Ensure working directory is clean: `git status`
- [ ] Commit or stash any uncommitted changes
- [ ] Ensure you're on the correct branch
- [ ] Run full test suite to establish baseline: `cargo test --all`
- [ ] Review the patterns document: `scripts/MIGRATION_PATTERNS.md`

#### 2. Dry Run (Recommended)

Run a dry run to see what would be changed without making actual modifications:

```bash
./scripts/migrate_effect_to_context.sh --dry-run
```

This will:
- Show all files that would be modified
- Display all replacements that would be made
- NOT create backups
- NOT modify any files

Review the output carefully. Look for:
- Unexpected replacements
- Files that shouldn't be modified
- Patterns that need exclusion

#### 3. Full Migration

Run the migration with automatic backup:

```bash
./scripts/migrate_effect_to_context.sh
```

The script will:
1. Create timestamped backup in `backups/terminology_migration_YYYYMMDD_HHMMSS/`
2. Process all `.rs` files in `crates/`
3. Apply all replacement patterns
4. Rename `effects.rs` to `contexts.rs`
5. Update import statements
6. Run `cargo check` to validate
7. Run `cargo test --lib` to verify functionality
8. Generate detailed report

**Progress Output:**
```
[INFO] Starting terminology migration: effect → context
[INFO] Creating backup in: backups/terminology_migration_20251121_143022
[SUCCESS] Backup created successfully
[INFO] Processing: crates/verum_types/src/effects.rs
[SUCCESS]   ✓ Type: EffectSet
[SUCCESS]   ✓ Type: EffectInferenceContext
[SUCCESS]   ✓ Function: add_effect
...
```

#### 4. Validation

The script automatically validates changes:

**Compilation Check:**
```bash
cargo check --all-targets --all-features
```

**Test Run:**
```bash
cargo test --lib
```

If validation fails:
- Review error messages in the log file
- Check the migration report
- Use rollback if needed: `./scripts/migrate_effect_to_context.sh --rollback`

#### 5. Review Changes

After successful migration, review:

**1. Generated Report:**
```bash
cat migration_report_YYYYMMDD_HHMMSS.md
```

**2. Git Diff:**
```bash
git diff --stat
git diff crates/verum_types/src/contexts.rs
```

**3. Key Files:**
- `crates/verum_types/src/contexts.rs` (renamed from effects.rs)
- `crates/verum_types/src/lib.rs` (updated imports)
- Test files with updated terminology

**4. Manual Verification:**
```bash
# Search for any remaining "Effect" references that should be "Context"
grep -r "Effect" --include="*.rs" crates/verum_types/src/

# Check for unintended replacements
grep -r "side context" --include="*.rs" crates/
```

#### 6. Commit Changes

If everything looks good:

```bash
# Add all changes
git add -A

# Create descriptive commit
git commit -m "refactor: Migrate terminology from 'effect' to 'context' per v6.0-BALANCED

- Rename EffectSet → ContextSet
- Rename EffectInferenceContext → ContextInferenceEngine
- Rename Effect → Context in type definitions
- Update all function/method names (add_effect → add_context, etc.)
- Rename effects.rs → contexts.rs
- Update all imports and exports
- Preserve 'side effect' terminology where appropriate
- All tests passing, compilation verified

Spec: CLAUDE.md v6.0-BALANCED terminology standards"
```

### Command Options

#### `--dry-run`

Preview changes without modifying files.

```bash
./scripts/migrate_effect_to_context.sh --dry-run
```

**Use when:**
- First time running the script
- Verifying pattern correctness
- Checking scope of changes

**Output:**
- Shows all replacements that would be made
- Lists files that would be modified
- No backup created
- No actual changes made

#### `--no-backup`

Run migration without creating backup (dangerous).

```bash
./scripts/migrate_effect_to_context.sh --no-backup
```

**⚠️ WARNING:** Only use if you have committed changes to git or have external backup.

**Use when:**
- Re-running after manual fixes
- Git history provides sufficient backup
- Disk space constraints

#### `--rollback`

Restore from the most recent backup.

```bash
./scripts/migrate_effect_to_context.sh --rollback
```

**Use when:**
- Migration produced unexpected results
- Compilation failed after migration
- Need to start over with different patterns

**Process:**
1. Finds most recent backup in `backups/`
2. Prompts for confirmation
3. Restores all backed-up files
4. Runs `cargo check` to verify rollback
5. Reports success or failure

## Output Files

### Log File

**Location:** `migration_log_YYYYMMDD_HHMMSS.txt`

**Contents:**
- All script output
- File processing messages
- Replacement confirmations
- Compilation results
- Test results
- Error messages

**Example:**
```
[INFO] Starting terminology migration: effect → context
[INFO] Project root: /Users/username/projects/verum
[INFO] Creating backup in: backups/terminology_migration_20251121_143022
[SUCCESS] Backup created successfully
[INFO] Processing: crates/verum_types/src/effects.rs
[SUCCESS]   ✓ Type: EffectSet
...
```

### Report File

**Location:** `migration_report_YYYYMMDD_HHMMSS.md`

**Contents:**
- Summary of changes
- Pattern replacement table
- List of modified files
- Validation results
- Backup location
- Rollback instructions

**Example:**
```markdown
# Terminology Migration Report: "effect" → "context"

## Summary

### Migration Date
2025-11-21 14:30:22

### Changes Applied

| Pattern | Replacement | Count |
|---------|-------------|-------|
| EffectSet | ContextSet | 47 |
| EffectInferenceContext | ContextInferenceEngine | 12 |
...
```

### Backup Directory

**Location:** `backups/terminology_migration_YYYYMMDD_HHMMSS/`

**Structure:**
```
backups/terminology_migration_20251121_143022/
├── crates/
│   └── verum_types/
│       └── src/
│           ├── effects.rs
│           ├── lib.rs
│           └── ...
└── docs/
    └── ...
```

**Retention:**
- Backups are kept indefinitely
- Manually clean up old backups if disk space is needed
- Most recent backup is used for rollback

## Troubleshooting

### Issue: Script fails with "permission denied"

**Solution:**
```bash
chmod +x scripts/migrate_effect_to_context.sh
```

### Issue: Compilation fails after migration

**Possible causes:**
1. Missed import updates
2. Macro-generated code still uses old names
3. Conditional compilation (`#[cfg(...)]`) paths

**Solution:**
```bash
# View compilation errors
cargo check 2>&1 | less

# Rollback and investigate
./scripts/migrate_effect_to_context.sh --rollback

# Fix patterns and re-run
./scripts/migrate_effect_to_context.sh
```

### Issue: Tests fail after migration

**Common causes:**
1. Test function names need updates
2. Expected error messages changed
3. Mock objects use old terminology

**Solution:**
```bash
# See which tests failed
cargo test --lib 2>&1 | less

# Manually fix test-specific issues
# Then re-run tests
cargo test --all
```

### Issue: "Side effect" incorrectly replaced with "side context"

**Cause:** Exclusion pattern not working correctly

**Solution:**
1. Rollback: `./scripts/migrate_effect_to_context.sh --rollback`
2. Check exclusion patterns in script
3. Report bug or fix pattern
4. Re-run migration

### Issue: Some files not migrated

**Possible causes:**
1. Files outside `crates/` directory
2. Non-Rust files (`.md`, `.toml`)
3. Generated files excluded

**Solution:**
```bash
# Manually check and update remaining files
grep -r "Effect" --include="*.rs" --exclude-dir=target .

# Or extend script to cover additional directories
```

### Issue: Want to migrate only specific crates

**Solution:**
Currently, the script migrates all crates. To migrate specific crates:

1. Create backup manually:
   ```bash
   cp -r crates/verum_types crates/verum_types.backup
   ```

2. Modify script temporarily to only process desired crate:
   ```bash
   # In find_rust_files() function, change:
   find "$PROJECT_ROOT/crates/verum_types" -name "*.rs" -type f
   ```

3. Run migration

## Advanced Usage

### Custom Pattern Addition

To add custom patterns, edit the script:

```bash
vim scripts/migrate_effect_to_context.sh
```

Find the `patterns` array in `apply_replacements()` function:

```bash
local patterns=(
    # Add your custom pattern here
    "my_effect_func|my_context_func|Custom: my_effect_func"

    # Existing patterns...
    "EffectSet|ContextSet|Type: EffectSet"
    ...
)
```

Pattern format: `"regex_pattern|replacement|description"`

### Integration with Git

The script uses `git mv` when available for file renames, which:
- Preserves file history
- Shows renames cleanly in `git log --follow`
- Makes code review easier

If git is not available, it falls back to regular `mv`.

### Running in CI/CD

For automated environments:

```bash
# Run with automatic yes to prompts (not recommended for rollback)
yes | ./scripts/migrate_effect_to_context.sh

# Or ensure clean state first
git diff --quiet || exit 1
./scripts/migrate_effect_to_context.sh
```

## Best Practices

### Before Migration

1. ✅ **Clean working directory**: Commit or stash changes
2. ✅ **Baseline tests**: Ensure all tests pass
3. ✅ **Review patterns**: Understand what will change
4. ✅ **Dry run**: Always preview changes first
5. ✅ **Communication**: Inform team of upcoming changes

### During Migration

1. ✅ **Monitor output**: Watch for unexpected changes
2. ✅ **Save logs**: Keep migration logs for reference
3. ✅ **Check backups**: Verify backup was created
4. ✅ **Read reports**: Review detailed migration report

### After Migration

1. ✅ **Validate compilation**: Ensure `cargo check` passes
2. ✅ **Run tests**: Full test suite should pass
3. ✅ **Manual review**: Check key files manually
4. ✅ **Git diff**: Review all changes before committing
5. ✅ **Update docs**: Ensure documentation uses new terminology
6. ✅ **Team review**: Code review before merging

## FAQ

**Q: Can I run the script multiple times?**

A: Yes, but second run will find fewer replacements. Use `--no-backup` for subsequent runs if needed.

**Q: What if I want to keep some "Effect" names?**

A: Add exclusion patterns to `should_exclude_line()` function in the script, or manually revert specific changes after migration.

**Q: Does this update documentation files?**

A: Currently only `.rs` files. Manually update `.md` documentation files separately.

**Q: Can I migrate just one module?**

A: Modify `find_rust_files()` function to target specific directories. See "Advanced Usage" section.

**Q: What about macro-generated code?**

A: The script only processes source files. If macros generate code with "Effect" names, you'll need to update the macro definitions.

**Q: Is this safe to use on production code?**

A: Yes, with proper testing:
1. Run in feature branch
2. Full test suite validation
3. Code review
4. Gradual rollout

**Q: How long does it take?**

A: Usually < 1 minute for the entire codebase. Validation (cargo check/test) takes longer.

## Support

If you encounter issues:

1. Check this guide and `MIGRATION_PATTERNS.md`
2. Review generated log and report files
3. Use rollback if needed
4. Report bugs with:
   - Log file contents
   - Error messages
   - Steps to reproduce

## See Also

- `MIGRATION_PATTERNS.md` - Detailed pattern documentation
- `CLAUDE.md` - Verum development standards
- `docs/03-type-system.md` - Context system specification
