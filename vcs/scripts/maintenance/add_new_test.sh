#!/bin/bash
# =============================================================================
# Verum Compliance Suite - New Test Creator
# =============================================================================
#
# Creates new test files with proper structure, metadata, and boilerplate.
# Ensures correct Verum syntax as per grammar/verum.ebnf.
#
# Usage:
#   ./add_new_test.sh [options]
#
# Options:
#   --name NAME          Test name (required)
#   --level LEVEL        Test level: L0, L1, L2, L3, L4 (required)
#   --category CAT       Category within level (e.g., lexer, parser, types)
#   --type TYPE          Test type: unit, integration, property, differential
#   --tier TIER          Execution tier: 0, 1, 2, 3 (default: 0)
#   --expect EXPECT      Expected result: pass, fail, error(Type) (default: pass)
#   --timeout MS         Timeout in milliseconds (default: 5000)
#   --tags TAGS          Comma-separated tags
#   --template TEMPLATE  Template to use: empty, basic, assertion, error
#   --description DESC   Test description
#   --edit               Open in editor after creation
#   --verbose            Enable verbose output
#   -h, --help           Show help message
#
# Examples:
#   ./add_new_test.sh --name "parse_nested_blocks" --level L0 --category parser
#   ./add_new_test.sh --name "type_inference_generics" --level L1 --category types --type unit
#   ./add_new_test.sh --name "async_context_switch" --level L2 --category async --tier 3
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PROJECT_ROOT="$(cd "$VCS_ROOT/.." && pwd)"

# Configuration
NAME=""
LEVEL=""
CATEGORY=""
TYPE="unit"
TIER="0"
EXPECT="pass"
TIMEOUT="5000"
TAGS=""
TEMPLATE="basic"
DESCRIPTION=""
EDIT=0
VERBOSE=0

SPECS_DIR="$VCS_ROOT/specs"

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
            --name)
                NAME="$2"
                shift 2
                ;;
            --level)
                LEVEL="$2"
                shift 2
                ;;
            --category)
                CATEGORY="$2"
                shift 2
                ;;
            --type)
                TYPE="$2"
                shift 2
                ;;
            --tier)
                TIER="$2"
                shift 2
                ;;
            --expect)
                EXPECT="$2"
                shift 2
                ;;
            --timeout)
                TIMEOUT="$2"
                shift 2
                ;;
            --tags)
                TAGS="$2"
                shift 2
                ;;
            --template)
                TEMPLATE="$2"
                shift 2
                ;;
            --description)
                DESCRIPTION="$2"
                shift 2
                ;;
            --edit)
                EDIT=1
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

# Validate inputs
validate_inputs() {
    if [ -z "$NAME" ]; then
        log_error "Test name is required (--name)"
        exit 1
    fi

    if [ -z "$LEVEL" ]; then
        log_error "Test level is required (--level)"
        exit 1
    fi

    case "$LEVEL" in
        L0|L1|L2|L3|L4) ;;
        *)
            log_error "Invalid level: $LEVEL (must be L0, L1, L2, L3, or L4)"
            exit 1
            ;;
    esac

    case "$TYPE" in
        unit|integration|property|differential) ;;
        *)
            log_error "Invalid type: $TYPE"
            exit 1
            ;;
    esac

    case "$TIER" in
        0|1|2|3) ;;
        *)
            log_error "Invalid tier: $TIER (must be 0, 1, 2, or 3)"
            exit 1
            ;;
    esac
}

# Get level directory name
get_level_dir() {
    case "$LEVEL" in
        L0) echo "L0-critical" ;;
        L1) echo "L1-core" ;;
        L2) echo "L2-standard" ;;
        L3) echo "L3-extended" ;;
        L4) echo "L4-performance" ;;
    esac
}

# Generate test file path
get_test_path() {
    local level_dir
    level_dir=$(get_level_dir)

    local path="$SPECS_DIR/$level_dir"

    if [ -n "$CATEGORY" ]; then
        path="$path/$CATEGORY"
    fi

    echo "$path/${NAME}.vr"
}

# Generate test metadata header
generate_metadata() {
    local metadata="// @test: $TYPE
// @tier: $TIER
// @level: $LEVEL
// @expect: $EXPECT
// @timeout: $TIMEOUT"

    if [ -n "$TAGS" ]; then
        metadata="$metadata
// @tags: $TAGS"
    fi

    if [ -n "$DESCRIPTION" ]; then
        metadata="$metadata
// @description: $DESCRIPTION"
    fi

    echo "$metadata"
}

# Generate test body based on template
generate_body() {
    case "$TEMPLATE" in
        empty)
            cat << 'EOF'

// TODO: Implement test

EOF
            ;;
        basic)
            cat << 'EOF'

// Test implementation using correct Verum syntax.
// Reference: grammar/verum.ebnf

fn main() {
    // TODO: Implement test
    let x: Int = 42;
    assert(x == 42);
}
EOF
            ;;
        assertion)
            cat << 'EOF'

// Assertion-based test

fn main() {
    // Setup
    let value: Int = compute_value();

    // Assertions
    assert(value > 0, "Value should be positive");
    assert(value < 100, "Value should be less than 100");

    // Cleanup (if needed)
}

fn compute_value() -> Int {
    // TODO: Implement
    42
}
EOF
            ;;
        error)
            cat << 'EOF'

// Error case test - expects compilation or runtime error
// The @expect header should specify the expected error type

fn main() {
    // This code should produce an error
    // TODO: Implement error-producing code
}
EOF
            ;;
        type_def)
            cat << 'EOF'

// Type definition test
// Uses correct Verum syntax (NOT Rust syntax!)

// Record type (like struct)
type Point is {
    x: Float,
    y: Float,
};

// Sum type (like enum)
type Shape is
    | Circle { center: Point, radius: Float }
    | Rectangle { top_left: Point, bottom_right: Point }
    ;

fn main() {
    let p: Point = Point { x: 0.0, y: 0.0 };
    let s: Shape = Shape.Circle { center: p, radius: 1.0 };

    // TODO: Add assertions
}
EOF
            ;;
        protocol)
            cat << 'EOF'

// Protocol (trait) test
// Uses correct Verum syntax

type Printable is protocol {
    fn print(&self) -> Text;
};

type MyType is {
    value: Int,
};

implement Printable for MyType {
    fn print(&self) -> Text {
        // TODO: Implement
        "MyType"
    }
}

fn main() {
    let obj: MyType = MyType { value: 42 };
    let s: Text = obj.print();
    assert(s == "MyType");
}
EOF
            ;;
        context)
            cat << 'EOF'

// Context system test
// Tests the using [...] dependency injection

fn process() using [Logger] {
    // Logger is available in this context
    // TODO: Implement
}

fn main() {
    // Provide context and call function
    provide Logger = create_logger() {
        process();
    }
}

fn create_logger() -> Logger {
    // TODO: Implement
}
EOF
            ;;
        cbgr)
            cat << 'EOF'

// CBGR (memory safety) test
// Tests reference tiers and generation checks

fn main() {
    // Tier 0: Default, full CBGR protection (~15ns)
    let x: Int = 42;
    let r: &Int = &x;
    assert(*r == 42);

    // Tier 1: Compiler-proven safe (0ns overhead)
    // let checked_ref: &checked Int = &checked x;

    // Tier 2: Manual safety proof (0ns overhead)
    // unsafe {
    //     let unsafe_ref: &unsafe Int = &unsafe x;
    // }

    // TODO: Add CBGR-specific tests
}
EOF
            ;;
        *)
            log_error "Unknown template: $TEMPLATE"
            exit 1
            ;;
    esac
}

# Generate complete test file content
generate_test_content() {
    local metadata
    metadata=$(generate_metadata)

    local body
    body=$(generate_body)

    echo "$metadata"
    echo "$body"
}

# Create the test file
create_test_file() {
    local test_path
    test_path=$(get_test_path)

    local test_dir
    test_dir=$(dirname "$test_path")

    # Check if file exists
    if [ -f "$test_path" ]; then
        log_error "Test file already exists: $test_path"
        exit 1
    fi

    # Create directory
    mkdir -p "$test_dir"

    # Generate content
    local content
    content=$(generate_test_content)

    # Write file
    echo "$content" > "$test_path"

    log_success "Created test file: $test_path"
    echo ""
    echo "File contents:"
    echo "---"
    cat "$test_path"
    echo "---"
}

# Open in editor
open_in_editor() {
    local test_path
    test_path=$(get_test_path)

    local editor="${EDITOR:-vim}"

    if [ "$EDIT" -eq 1 ]; then
        log_info "Opening in $editor..."
        "$editor" "$test_path"
    fi
}

# Main function
main() {
    parse_args "$@"
    validate_inputs

    log_info "VCS New Test Creator"
    log_info "Name:     $NAME"
    log_info "Level:    $LEVEL"
    log_info "Category: ${CATEGORY:-<none>}"
    log_info "Type:     $TYPE"
    log_info "Template: $TEMPLATE"
    echo ""

    create_test_file
    open_in_editor

    echo ""
    log_info "Remember to:"
    echo "  1. Implement the test logic"
    echo "  2. Use correct Verum syntax (see grammar/verum.ebnf)"
    echo "  3. Run: make test-${LEVEL,,} to verify"
}

# Run main
main "$@"
