#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Golden File Updater
# =============================================================================
#
# Updates expected output ("golden") files for VCS tests. Use this when:
# - Intentional changes to language semantics require new expected outputs
# - Adding new test cases with expected outputs
# - Fixing incorrect expected outputs
#
# CAUTION: Only update golden files after careful review!
#
# Usage:
#   ./update_golden_files.sh [options] [test-patterns...]
#
# Options:
#   --level LEVEL        Only update tests at this level (L0, L1, L2, L3, L4)
#   --category CAT       Only update tests in this category
#   --filter PATTERN     Only update tests matching pattern
#   --backup             Create backup of existing golden files
#   --diff               Show diff before updating
#   --interactive        Prompt before each update
#   --dry-run            Show what would be updated without changing files
#   --verbose            Enable verbose output
#   -h, --help           Show help message
#
# Examples:
#   ./update_golden_files.sh --level L0 --backup
#   ./update_golden_files.sh --filter "parser/*" --diff --interactive
#   ./update_golden_files.sh --category lexer --dry-run
#
# Environment Variables:
#   VCS_GOLDEN_DIR        Golden files directory (default: specs/)
#   VCS_BACKUP_DIR        Backup directory
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
LEVEL=""
CATEGORY=""
FILTER=""
BACKUP=0
SHOW_DIFF=0
INTERACTIVE=0
DRY_RUN=0
VERBOSE=0

SPECS_DIR="${VCS_GOLDEN_DIR:-$VCS_ROOT/specs}"
BACKUP_DIR="${VCS_BACKUP_DIR:-$VCS_ROOT/.golden-backup}"
VTEST="$VCS_ROOT/runner/vtest/target/release/vtest"
CONFIG="$VCS_ROOT/config/vcs.toml"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --level)
                LEVEL="$2"
                shift 2
                ;;
            --category)
                CATEGORY="$2"
                shift 2
                ;;
            --filter)
                FILTER="$2"
                shift 2
                ;;
            --backup)
                BACKUP=1
                shift
                ;;
            --diff)
                SHOW_DIFF=1
                shift
                ;;
            --interactive)
                INTERACTIVE=1
                SHOW_DIFF=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            --verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done
}

# Find test files matching criteria
find_test_files() {
    local search_path="$SPECS_DIR"

    # Filter by level
    if [ -n "$LEVEL" ]; then
        case "$LEVEL" in
            L0) search_path="$SPECS_DIR/L0-critical" ;;
            L1) search_path="$SPECS_DIR/L1-core" ;;
            L2) search_path="$SPECS_DIR/L2-standard" ;;
            L3) search_path="$SPECS_DIR/L3-extended" ;;
            L4) search_path="$SPECS_DIR/L4-performance" ;;
        esac
    fi

    # Filter by category
    if [ -n "$CATEGORY" ]; then
        search_path="$search_path/$CATEGORY"
    fi

    # Find all .vr files
    local files
    if [ -n "$FILTER" ]; then
        files=$(find "$search_path" -name "*.vr" 2>/dev/null | grep -E "$FILTER" || true)
    else
        files=$(find "$search_path" -name "*.vr" 2>/dev/null || true)
    fi

    echo "$files"
}

# Get golden file path for a test file
get_golden_path() {
    local test_file="$1"
    local dir
    dir=$(dirname "$test_file")
    local name
    name=$(basename "$test_file" .vr)

    echo "$dir/$name.golden"
}

# Get actual output from running test
get_actual_output() {
    local test_file="$1"

    # Run the test and capture output
    "$VTEST" run "$test_file" \
        --format json \
        --config "$CONFIG" \
        --capture-output 2>/dev/null || true
}

# Create backup of golden file
backup_golden() {
    local golden_file="$1"

    if [ ! -f "$golden_file" ]; then
        return 0
    fi

    local timestamp
    timestamp=$(date +%Y%m%d_%H%M%S)
    local backup_path="$BACKUP_DIR/$timestamp"
    local relative_path
    relative_path=$(realpath --relative-to="$SPECS_DIR" "$golden_file" 2>/dev/null || echo "$golden_file")

    mkdir -p "$backup_path/$(dirname "$relative_path")"
    cp "$golden_file" "$backup_path/$relative_path"

    if [ "$VERBOSE" -eq 1 ]; then
        log_info "Backed up: $golden_file"
    fi
}

# Show diff between current and new golden file
show_diff() {
    local current="$1"
    local new_content="$2"

    if [ ! -f "$current" ]; then
        echo -e "${GREEN}New file:${NC}"
        echo "$new_content" | head -20
        if [ "$(echo "$new_content" | wc -l)" -gt 20 ]; then
            echo "... (truncated)"
        fi
        return
    fi

    echo -e "${YELLOW}Diff:${NC}"
    diff -u "$current" <(echo "$new_content") | head -50 || true
}

# Prompt user for confirmation
prompt_update() {
    local file="$1"

    echo ""
    read -rp "Update golden file for $(basename "$file")? [y/N] " response
    case "$response" in
        [yY]|[yY][eE][sS])
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

# Update single golden file
update_golden_file() {
    local test_file="$1"
    local golden_file
    golden_file=$(get_golden_path "$test_file")

    if [ "$VERBOSE" -eq 1 ]; then
        log_info "Processing: $test_file"
    fi

    # Get actual output
    local actual
    actual=$(get_actual_output "$test_file")

    if [ -z "$actual" ]; then
        log_warn "No output for: $test_file"
        return 0
    fi

    # Show diff if requested
    if [ "$SHOW_DIFF" -eq 1 ]; then
        echo ""
        echo "=== $test_file ==="
        show_diff "$golden_file" "$actual"
    fi

    # Interactive mode
    if [ "$INTERACTIVE" -eq 1 ]; then
        if ! prompt_update "$test_file"; then
            log_info "Skipped: $test_file"
            return 0
        fi
    fi

    # Dry run mode
    if [ "$DRY_RUN" -eq 1 ]; then
        log_info "[DRY-RUN] Would update: $golden_file"
        return 0
    fi

    # Backup if requested
    if [ "$BACKUP" -eq 1 ]; then
        backup_golden "$golden_file"
    fi

    # Write new golden file
    echo "$actual" > "$golden_file"
    log_success "Updated: $golden_file"
}

# Main function
main() {
    parse_args "$@"

    log_info "VCS Golden File Updater"
    echo ""

    # Find test files
    local test_files
    test_files=$(find_test_files)

    if [ -z "$test_files" ]; then
        log_warn "No test files found matching criteria"
        exit 0
    fi

    local count
    count=$(echo "$test_files" | wc -l)
    log_info "Found $count test files"

    if [ "$BACKUP" -eq 1 ]; then
        log_info "Backup directory: $BACKUP_DIR"
        mkdir -p "$BACKUP_DIR"
    fi

    if [ "$DRY_RUN" -eq 1 ]; then
        log_info "DRY RUN - no files will be modified"
    fi

    echo ""

    # Process each file
    local updated=0
    local skipped=0

    while IFS= read -r test_file; do
        if [ -z "$test_file" ]; then
            continue
        fi

        if update_golden_file "$test_file"; then
            ((updated++))
        else
            ((skipped++))
        fi
    done <<< "$test_files"

    echo ""
    log_success "Done: $updated updated, $skipped skipped"
}

# Run main
main "$@"
