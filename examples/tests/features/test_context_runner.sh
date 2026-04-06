#!/bin/bash

# Context System Test Runner
# Tests all context-related features

TESTS_DIR="/Users/taaliman/projects/luxquant/axiom/examples/tests/features"
RESULTS_FILE="$TESTS_DIR/CONTEXT_SYSTEM_TEST_RESULTS.md"

echo "# Context System Test Results" > "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"
echo "Generated: $(date)" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"
echo "## Test Summary" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"

# Array of test files
tests=(
    "test_context_basic.vr"
    "test_context_multiple.vr"
    "test_context_group.vr"
    "test_provide.vr"
    "test_context_nested.vr"
    "test_context_impl.vr"
)

passed=0
failed=0

for test in "${tests[@]}"; do
    echo "Testing: $test"
    echo "### $test" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
    echo "\`\`\`" >> "$RESULTS_FILE"

    # Run the test and capture output
    cd /Users/taaliman/projects/luxquant/axiom
    if cargo run --bin verum -- run "$TESTS_DIR/$test" >> "$RESULTS_FILE" 2>&1; then
        echo "\`\`\`" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
        echo "**Status: PASS**" >> "$RESULTS_FILE"
        ((passed++))
    else
        echo "\`\`\`" >> "$RESULTS_FILE"
        echo "" >> "$RESULTS_FILE"
        echo "**Status: FAIL**" >> "$RESULTS_FILE"
        ((failed++))
    fi
    echo "" >> "$RESULTS_FILE"
    echo "---" >> "$RESULTS_FILE"
    echo "" >> "$RESULTS_FILE"
done

# Add summary at the end
echo "## Final Summary" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"
echo "- **Passed:** $passed / ${#tests[@]}" >> "$RESULTS_FILE"
echo "- **Failed:** $failed / ${#tests[@]}" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"

echo ""
echo "Results written to: $RESULTS_FILE"
echo "Passed: $passed / ${#tests[@]}"
echo "Failed: $failed / ${#tests[@]}"
