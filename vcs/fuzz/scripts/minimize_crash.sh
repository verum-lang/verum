#!/bin/bash
# minimize_crash.sh - Minimize crashing inputs
#
# This script takes crashing inputs and minimizes them to the smallest
# input that still triggers the same crash. This makes debugging easier.
#
# Usage:
#   ./minimize_crash.sh [OPTIONS]
#
# Options:
#   -i, --input FILE|DIR   Input crash file or directory
#   -o, --output DIR       Output directory for minimized crashes
#   -t, --target TARGET    Compiler target: lexer, parser, typecheck, codegen
#   --timeout SECS         Timeout per minimization attempt (default: 30)
#   --max-iterations N     Maximum minimization iterations (default: 1000)
#   -v, --verbose          Verbose output
#   -h, --help             Show this help message

set -euo pipefail

# Default configuration
INPUT=""
OUTPUT_DIR="minimized"
TARGET=""
TIMEOUT=30
MAX_ITERATIONS=1000
VERBOSE=false

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FUZZ_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_ROOT="$(dirname "$(dirname "$FUZZ_DIR")")"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_verbose() {
    if [[ "$VERBOSE" == "true" ]]; then
        echo -e "${BLUE}[DEBUG]${NC} $1"
    fi
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -i|--input)
            INPUT="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        -t|--target)
            TARGET="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --max-iterations)
            MAX_ITERATIONS="$2"
            shift 2
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            head -25 "$0" | tail -n +2 | sed 's/^# //'
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Validate input
if [[ -z "$INPUT" ]]; then
    log_error "Input file or directory required. Use -i or --input."
    exit 1
fi

if [[ ! -e "$INPUT" ]]; then
    log_error "Input does not exist: $INPUT"
    exit 1
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check if crash still reproduces
verify_crash() {
    local file=$1
    local target=${2:-$TARGET}

    log_verbose "Verifying crash: $file"

    # Try to compile/parse the file and check for crash
    cd "$PROJECT_ROOT"

    local result
    if timeout "$TIMEOUT" cargo run --release -p verum_fuzz --bin verify_crash -- \
        --target "$target" \
        --input "$file" 2>/dev/null; then
        result=0
    else
        result=$?
    fi

    # Crash reproduces if exit code is non-zero
    if [[ $result -ne 0 ]]; then
        log_verbose "Crash reproduces (exit code: $result)"
        return 0
    else
        log_verbose "Crash does not reproduce"
        return 1
    fi
}

# Delta debugging algorithm
delta_debug() {
    local input_file=$1
    local output_file=$2

    log_info "Starting delta debugging for: $input_file"

    # Read input
    local content
    content=$(cat "$input_file")
    local original_size=${#content}

    log_info "Original size: $original_size bytes"

    # Strategy 1: Line-based minimization
    log_info "Phase 1: Line-based minimization"
    local lines
    IFS=$'\n' read -d '' -ra lines <<< "$content" || true

    local iteration=0
    local made_progress=true

    while [[ "$made_progress" == "true" && $iteration -lt $MAX_ITERATIONS ]]; do
        made_progress=false
        iteration=$((iteration + 1))

        for ((i = 0; i < ${#lines[@]}; i++)); do
            if [[ -z "${lines[$i]}" ]]; then
                continue
            fi

            # Try removing this line
            local candidate=""
            for ((j = 0; j < ${#lines[@]}; j++)); do
                if [[ $j -ne $i ]]; then
                    candidate="${candidate}${lines[$j]}"$'\n'
                fi
            done

            # Save candidate
            local temp_file=$(mktemp)
            echo -n "$candidate" > "$temp_file"

            # Check if crash still reproduces
            if verify_crash "$temp_file"; then
                log_verbose "Removed line $i"
                unset 'lines[$i]'
                content="$candidate"
                made_progress=true
                rm "$temp_file"
                break
            fi

            rm "$temp_file"
        done
    done

    log_info "After line minimization: ${#content} bytes"

    # Strategy 2: Token-based minimization
    log_info "Phase 2: Token-based minimization"

    # Tokenize on whitespace and special characters
    local tokens
    read -ra tokens <<< "$content"

    iteration=0
    made_progress=true

    while [[ "$made_progress" == "true" && $iteration -lt $MAX_ITERATIONS ]]; do
        made_progress=false
        iteration=$((iteration + 1))

        for ((i = 0; i < ${#tokens[@]}; i++)); do
            if [[ -z "${tokens[$i]}" ]]; then
                continue
            fi

            # Try removing this token
            local candidate=""
            for ((j = 0; j < ${#tokens[@]}; j++)); do
                if [[ $j -ne $i ]]; then
                    candidate="${candidate}${tokens[$j]} "
                fi
            done

            local temp_file=$(mktemp)
            echo -n "$candidate" > "$temp_file"

            if verify_crash "$temp_file"; then
                log_verbose "Removed token $i: ${tokens[$i]}"
                unset 'tokens[$i]'
                content="$candidate"
                made_progress=true
                rm "$temp_file"
                break
            fi

            rm "$temp_file"
        done
    done

    log_info "After token minimization: ${#content} bytes"

    # Strategy 3: Character-based minimization (for small inputs)
    if [[ ${#content} -lt 1000 ]]; then
        log_info "Phase 3: Character-based minimization"

        iteration=0
        made_progress=true

        while [[ "$made_progress" == "true" && $iteration -lt $MAX_ITERATIONS ]]; do
            made_progress=false
            iteration=$((iteration + 1))

            for ((i = 0; i < ${#content}; i++)); do
                # Try removing this character
                local candidate="${content:0:$i}${content:$((i+1))}"

                local temp_file=$(mktemp)
                echo -n "$candidate" > "$temp_file"

                if verify_crash "$temp_file"; then
                    log_verbose "Removed character at position $i"
                    content="$candidate"
                    made_progress=true
                    rm "$temp_file"
                    break
                fi

                rm "$temp_file"
            done
        done

        log_info "After character minimization: ${#content} bytes"
    fi

    # Save minimized result
    echo -n "$content" > "$output_file"

    local final_size=${#content}
    local reduction=$(( (original_size - final_size) * 100 / original_size ))

    log_success "Minimization complete: $original_size -> $final_size bytes ($reduction% reduction)"
}

# Minimize a single file
minimize_file() {
    local input_file=$1
    local output_file=$2

    # Detect target from filename if not specified
    local target=$TARGET
    if [[ -z "$target" ]]; then
        if [[ "$input_file" == *"lexer"* ]]; then
            target="lexer"
        elif [[ "$input_file" == *"parser"* ]]; then
            target="parser"
        elif [[ "$input_file" == *"typecheck"* ]]; then
            target="typecheck"
        elif [[ "$input_file" == *"codegen"* ]]; then
            target="codegen"
        else
            target="parser"  # Default
        fi
    fi

    log_info "Minimizing: $input_file -> $output_file (target: $target)"

    # First verify the crash reproduces
    if ! verify_crash "$input_file" "$target"; then
        log_warning "Crash does not reproduce, skipping: $input_file"
        return 1
    fi

    # Run delta debugging
    delta_debug "$input_file" "$output_file"

    return 0
}

# Main execution
main() {
    log_info "Crash Minimization Tool"
    log_info "======================="

    local minimized_count=0
    local failed_count=0

    if [[ -f "$INPUT" ]]; then
        # Single file
        local basename=$(basename "$INPUT")
        local output_file="$OUTPUT_DIR/minimized_$basename"

        if minimize_file "$INPUT" "$output_file"; then
            minimized_count=$((minimized_count + 1))
        else
            failed_count=$((failed_count + 1))
        fi
    else
        # Directory - process all crash files
        for crash_file in $(find "$INPUT" -name "crash*" -type f 2>/dev/null); do
            local basename=$(basename "$crash_file")
            local output_file="$OUTPUT_DIR/minimized_$basename"

            if minimize_file "$crash_file" "$output_file"; then
                minimized_count=$((minimized_count + 1))
            else
                failed_count=$((failed_count + 1))
            fi
        done
    fi

    echo
    log_info "Minimization Summary"
    log_info "===================="
    log_info "Minimized: $minimized_count"
    log_info "Failed: $failed_count"
    log_info "Output: $OUTPUT_DIR"
}

main "$@"
