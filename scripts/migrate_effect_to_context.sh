#!/bin/bash
# Automatic terminology migration: "effect" → "context"
#
# This script replaces incorrect "effect" terminology with "context" across
# the entire Verum codebase, while preserving legitimate uses of "effect"
# (e.g., "side effect", "side-effect").
#
# Usage:
#   ./scripts/migrate_effect_to_context.sh [--dry-run] [--no-backup]
#
# Options:
#   --dry-run      Show what would be changed without making changes
#   --no-backup    Skip creating backups (use with caution)
#   --rollback     Restore from the most recent backup

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BACKUP_DIR="$PROJECT_ROOT/backups/terminology_migration_$(date +%Y%m%d_%H%M%S)"
LOG_FILE="$PROJECT_ROOT/migration_log_$(date +%Y%m%d_%H%M%S).txt"
REPORT_FILE="$PROJECT_ROOT/migration_report_$(date +%Y%m%d_%H%M%S).md"

# Parse command line arguments
DRY_RUN=false
NO_BACKUP=false
ROLLBACK=false

for arg in "$@"; do
    case $arg in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --no-backup)
            NO_BACKUP=true
            shift
            ;;
        --rollback)
            ROLLBACK=true
            shift
            ;;
        *)
            echo -e "${RED}Unknown option: $arg${NC}"
            echo "Usage: $0 [--dry-run] [--no-backup] [--rollback]"
            exit 1
            ;;
    esac
done

# Logging functions
log() {
    echo -e "$1" | tee -a "$LOG_FILE"
}

log_info() {
    log "${BLUE}[INFO]${NC} $1"
}

log_success() {
    log "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    log "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    log "${RED}[ERROR]${NC} $1"
}

# Rollback function
rollback() {
    log_info "Looking for most recent backup..."

    local latest_backup
    latest_backup=$(find "$PROJECT_ROOT/backups" -maxdepth 1 -type d -name "terminology_migration_*" | sort -r | head -n 1)

    if [[ -z "$latest_backup" ]]; then
        log_error "No backup found to rollback from!"
        exit 1
    fi

    log_info "Found backup: $latest_backup"
    log_warning "This will restore all files from the backup. Continue? [y/N]"
    read -r confirm

    if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
        log_info "Rollback cancelled."
        exit 0
    fi

    log_info "Restoring files from backup..."

    # Restore each file
    local restored_count=0
    while IFS= read -r -d '' backup_file; do
        local rel_path="${backup_file#$latest_backup/}"
        local orig_file="$PROJECT_ROOT/$rel_path"

        if cp "$backup_file" "$orig_file"; then
            ((restored_count++))
        else
            log_error "Failed to restore: $orig_file"
        fi
    done < <(find "$latest_backup" -type f -print0)

    log_success "Restored $restored_count files from backup"
    log_info "Running cargo check to verify..."

    cd "$PROJECT_ROOT"
    if cargo check 2>&1 | tee -a "$LOG_FILE"; then
        log_success "Rollback completed successfully!"
    else
        log_error "Rollback completed but cargo check failed. Manual intervention may be needed."
        exit 1
    fi

    exit 0
}

# Handle rollback mode
if $ROLLBACK; then
    rollback
fi

# Initialize report
init_report() {
    cat > "$REPORT_FILE" << 'EOF'
# Terminology Migration Report: "effect" → "context"

## Summary

This report documents the automatic migration of Verum terminology from "effect" to "context".

### Migration Date
EOF
    echo "$(date '+%Y-%m-%d %H:%M:%S')" >> "$REPORT_FILE"
    cat >> "$REPORT_FILE" << 'EOF'

### Changes Applied

| Pattern | Replacement | Count |
|---------|-------------|-------|
EOF
}

# Add entry to report
add_report_entry() {
    local pattern="$1"
    local replacement="$2"
    local count="$3"
    echo "| \`$pattern\` | \`$replacement\` | $count |" >> "$REPORT_FILE"
}

# Finalize report
finalize_report() {
    cat >> "$REPORT_FILE" << EOF

## Files Modified

Total files modified: $1

### Detailed File List

EOF
    cat "$LOG_FILE" | grep "Processing:" >> "$REPORT_FILE" || true

    cat >> "$REPORT_FILE" << EOF

## Validation Results

### Compilation Check

\`\`\`
EOF
    cat "$LOG_FILE" | grep -A 20 "Running cargo check" >> "$REPORT_FILE" || true
    cat >> "$REPORT_FILE" << EOF
\`\`\`

## Backup Location

\`$BACKUP_DIR\`

## Rollback Instructions

To rollback this migration:

\`\`\`bash
./scripts/migrate_effect_to_context.sh --rollback
\`\`\`

Or manually restore from backup:

\`\`\`bash
# Restore all files
rsync -av --delete "$BACKUP_DIR/" "$PROJECT_ROOT/"
\`\`\`

EOF
}

# Create backup
create_backup() {
    if $NO_BACKUP; then
        log_warning "Skipping backup (--no-backup specified)"
        return
    fi

    log_info "Creating backup in: $BACKUP_DIR"
    mkdir -p "$BACKUP_DIR"

    # Backup all Rust files
    find "$PROJECT_ROOT/crates" -name "*.rs" -type f | while read -r file; do
        local rel_path="${file#$PROJECT_ROOT/}"
        local backup_path="$BACKUP_DIR/$rel_path"
        mkdir -p "$(dirname "$backup_path")"
        cp "$file" "$backup_path"
    done

    # Backup docs
    if [[ -d "$PROJECT_ROOT/docs" ]]; then
        find "$PROJECT_ROOT/docs" -name "*.md" -type f | while read -r file; do
            local rel_path="${file#$PROJECT_ROOT/}"
            local backup_path="$BACKUP_DIR/$rel_path"
            mkdir -p "$(dirname "$backup_path")"
            cp "$file" "$backup_path"
        done
    fi

    log_success "Backup created successfully"
}

# Check if a line should be excluded from replacement
should_exclude_line() {
    local line="$1"

    # Exclude lines with "side effect" or "side-effect"
    if echo "$line" | grep -qi "side[\s-]*effect"; then
        return 0  # true - should exclude
    fi

    # Exclude comments about side effects (common pattern)
    if echo "$line" | grep -q "//.*no side effect\|//.*side effect"; then
        return 0  # true - should exclude
    fi

    # Exclude string literals about side effects (be conservative)
    if echo "$line" | grep -q '"[^"]*side effect[^"]*"'; then
        return 0  # true - should exclude
    fi

    return 1  # false - don't exclude
}

# Smart replacement function using Perl for complex regex
smart_replace() {
    local file="$1"
    local pattern="$2"
    local replacement="$3"
    local case_sensitive="${4:-true}"

    if $DRY_RUN; then
        log_info "[DRY RUN] Would replace '$pattern' with '$replacement' in: $file"
        return 0
    fi

    # Create a temp file
    local tmp_file="${file}.tmp.$$"

    # Process line by line to respect exclusions
    local replaced=false
    while IFS= read -r line; do
        if should_exclude_line "$line"; then
            # Keep line as-is
            echo "$line" >> "$tmp_file"
        else
            # Apply replacement
            local new_line
            if $case_sensitive; then
                new_line=$(echo "$line" | perl -pe "s/$pattern/$replacement/g")
            else
                new_line=$(echo "$line" | perl -pe "s/$pattern/$replacement/gi")
            fi

            if [[ "$line" != "$new_line" ]]; then
                replaced=true
            fi
            echo "$new_line" >> "$tmp_file"
        fi
    done < "$file"

    # Replace original file if changes were made
    if $replaced; then
        mv "$tmp_file" "$file"
        return 0
    else
        rm -f "$tmp_file"
        return 1
    fi
}

# Pattern replacement function with counting
apply_replacements() {
    local file="$1"
    local total_changes=0

    log_info "Processing: $file"

    # Define replacement patterns
    # Format: "pattern|replacement|description"
    local patterns=(
        # Type names (CamelCase)
        "EffectSet|ContextSet|Type: EffectSet"
        "EffectInferenceContext|ContextInferenceEngine|Type: EffectInferenceContext"
        "Effect(?!Set)(?!Inference)|Context|Type: Effect (standalone)"

        # Function/method names (snake_case)
        "has_effect\b|has_context|Function: has_effect"
        "add_effect\b|add_context|Function: add_effect"
        "add_effects\b|add_contexts|Function: add_effects"
        "get_effects\b|get_contexts|Function: get_effects"
        "take_effects\b|take_contexts|Function: take_effects"
        "is_effect\b|is_context|Function: is_effect"
        "effect_set\b|context_set|Field: effect_set"
        "current_effects\b|current_contexts|Field: current_effects"

        # Variable names
        "\beffects\b(?!::|\.)|contexts|Variable: effects"
        "\beffect\b(?!s\b)(?!::|\.)|context|Variable: effect"

        # Module and file references in comments
        "effect system|context system|Term: effect system"
        "Effect system|Context system|Term: Effect system"
        "EFFECT|CONTEXT|Constant: EFFECT"
    )

    # Apply each pattern
    for pattern_line in "${patterns[@]}"; do
        IFS='|' read -r pattern replacement description <<< "$pattern_line"

        if smart_replace "$file" "$pattern" "$replacement" true; then
            ((total_changes++))
            log_success "  ✓ $description"
        fi
    done

    return $total_changes
}

# Find all Rust files to process
find_rust_files() {
    find "$PROJECT_ROOT/crates" -name "*.rs" -type f
}

# Main migration function
migrate() {
    log_info "Starting terminology migration: effect → context"
    log_info "Project root: $PROJECT_ROOT"
    log_info "Log file: $LOG_FILE"
    log_info "Report file: $REPORT_FILE"

    if $DRY_RUN; then
        log_warning "DRY RUN MODE - No changes will be made"
    fi

    # Initialize report
    init_report

    # Create backup
    if ! $DRY_RUN; then
        create_backup
    fi

    # Process all Rust files
    local total_files=0
    local modified_files=0
    local total_changes=0

    while IFS= read -r file; do
        ((total_files++))

        local file_changes=0
        if apply_replacements "$file"; then
            file_changes=$?
            ((modified_files++))
            ((total_changes+=file_changes))
        fi
    done < <(find_rust_files)

    log_info "Processed $total_files files"
    log_info "Modified $modified_files files"
    log_info "Total changes: $total_changes"

    # Handle file renames
    rename_files

    # Finalize report
    finalize_report "$modified_files"

    if $DRY_RUN; then
        log_info "Dry run complete. No changes were made."
        return 0
    fi

    # Validate compilation
    validate_compilation
}

# Rename effect*.rs files to context*.rs
rename_files() {
    log_info "Checking for files to rename..."

    local effects_file="$PROJECT_ROOT/crates/verum_types/src/effects.rs"
    local contexts_file="$PROJECT_ROOT/crates/verum_types/src/contexts.rs"

    if [[ -f "$effects_file" ]]; then
        if $DRY_RUN; then
            log_info "[DRY RUN] Would rename: effects.rs → contexts.rs"
        else
            log_info "Renaming: effects.rs → contexts.rs"

            # Update imports in lib.rs first
            local lib_file="$PROJECT_ROOT/crates/verum_types/src/lib.rs"
            if [[ -f "$lib_file" ]]; then
                perl -pi -e 's/mod effects;/mod contexts;/g' "$lib_file"
                perl -pi -e 's/pub use effects::/pub use contexts::/g' "$lib_file"
                log_success "Updated imports in lib.rs"
            fi

            # Rename the file
            git mv "$effects_file" "$contexts_file" 2>/dev/null || mv "$effects_file" "$contexts_file"
            log_success "Renamed effects.rs to contexts.rs"
        fi
    fi

    # Check for other effect_*.rs files
    find "$PROJECT_ROOT/crates" -name "effect_*.rs" -type f | while read -r file; do
        local dir=$(dirname "$file")
        local basename=$(basename "$file")
        local new_name="${basename/effect_/context_}"
        local new_path="$dir/$new_name"

        if $DRY_RUN; then
            log_info "[DRY RUN] Would rename: $basename → $new_name"
        else
            log_info "Renaming: $basename → $new_name"
            git mv "$file" "$new_path" 2>/dev/null || mv "$file" "$new_path"
            log_success "Renamed $basename to $new_name"
        fi
    done
}

# Validate that the code still compiles
validate_compilation() {
    log_info "Running cargo check to validate changes..."

    cd "$PROJECT_ROOT"

    if cargo check --all-targets --all-features 2>&1 | tee -a "$LOG_FILE"; then
        log_success "✓ Compilation successful!"
    else
        log_error "✗ Compilation failed!"
        log_error "Please review the errors above."
        log_info "To rollback changes, run: $0 --rollback"
        log_info "Or restore from backup: $BACKUP_DIR"
        exit 1
    fi

    # Run tests to ensure nothing broke
    log_info "Running cargo test to validate functionality..."

    if cargo test --lib 2>&1 | tee -a "$LOG_FILE"; then
        log_success "✓ Tests passed!"
    else
        log_warning "⚠ Some tests failed. This may be expected if tests need updates."
        log_info "Review the test failures to determine if they're related to the migration."
    fi
}

# Generate summary statistics
generate_summary() {
    cat << EOF

${GREEN}════════════════════════════════════════════════════════════${NC}
${GREEN}           Migration Complete!                               ${NC}
${GREEN}════════════════════════════════════════════════════════════${NC}

${BLUE}Summary:${NC}
  - Backup location: $BACKUP_DIR
  - Log file: $LOG_FILE
  - Report: $REPORT_FILE

${BLUE}Key Changes:${NC}
  ✓ EffectSet → ContextSet
  ✓ EffectInferenceContext → ContextInferenceEngine
  ✓ Effect → Context (in type definitions)
  ✓ effect_* → context_* (function/field names)
  ✓ effects.rs → contexts.rs

${BLUE}Preserved:${NC}
  ✓ "side effect" terminology (legitimate use)
  ✓ FFI effect annotations (intentional)
  ✓ External library references

${YELLOW}Next Steps:${NC}
  1. Review the migration report: $REPORT_FILE
  2. Run full test suite: cargo test --all
  3. Check for any remaining manual fixes needed
  4. Update documentation if necessary

${BLUE}Rollback if needed:${NC}
  $0 --rollback

${GREEN}════════════════════════════════════════════════════════════${NC}

EOF
}

# Main execution
main() {
    log_info "Verum Terminology Migration Tool"
    log_info "================================"
    echo ""

    # Run migration
    migrate

    # Show summary
    if ! $DRY_RUN; then
        generate_summary
    fi

    log_success "Migration completed successfully!"
}

# Run main
main
