#!/bin/bash
# Compilation Speed Benchmarking Script
# Measures compilation speed in LOC/sec (Lines of Code per second)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Calculate total lines of code
echo -e "${BLUE}=== Verum Compilation Speed Benchmark ===${NC}"
echo ""

# Total LOC in the project
TOTAL_LOC=471552
echo -e "${BLUE}Total codebase: ${YELLOW}${TOTAL_LOC} LOC${NC}"

# Parse command line arguments
PROFILE="${1:-release}"
CLEAN="${2:-false}"

echo -e "${BLUE}Build profile: ${YELLOW}${PROFILE}${NC}"

if [ "$CLEAN" = "true" ]; then
    echo -e "${YELLOW}Cleaning build artifacts...${NC}"
    cargo clean
    echo ""
fi

# Benchmark full workspace build
echo -e "${BLUE}Starting compilation benchmark...${NC}"
echo -e "${YELLOW}(This will take a while on first build)${NC}"
echo ""

START_TIME=$(date +%s.%N)

# Build the compiler (primary target)
cargo build -p verum_compiler --${PROFILE} 2>&1 | tee /tmp/build.log

END_TIME=$(date +%s.%N)

# Calculate build time
BUILD_TIME=$(echo "$END_TIME - $START_TIME" | bc)
BUILD_TIME_INT=${BUILD_TIME%.*}
BUILD_TIME_DEC=${BUILD_TIME#*.}

echo ""
echo -e "${GREEN}=== Build Complete ===${NC}"
echo ""

# Calculate LOC per second
LOC_PER_SEC=$(echo "scale=0; $TOTAL_LOC / $BUILD_TIME" | bc)

echo -e "${BLUE}Build time: ${YELLOW}${BUILD_TIME_INT}.${BUILD_TIME_DEC:0:2}${NC} seconds"
echo -e "${BLUE}Compilation speed: ${YELLOW}${LOC_PER_SEC}${NC} LOC/sec"
echo ""

# Check if target is met
TARGET=50000

if [ "$LOC_PER_SEC" -ge "$TARGET" ]; then
    echo -e "${GREEN}✓ TARGET MET: ${LOC_PER_SEC} >= ${TARGET} LOC/sec${NC}"
    exit 0
else
    IMPROVEMENT_NEEDED=$((TARGET - LOC_PER_SEC))
    PERCENT=$((100 * IMPROVEMENT_NEEDED / TARGET))
    echo -e "${YELLOW}⚠ TARGET NOT MET: ${LOC_PER_SEC} < ${TARGET} LOC/sec${NC}"
    echo -e "${YELLOW}  Need ${IMPROVEMENT_NEEDED} more LOC/sec (${PERCENT}% improvement)${NC}"
    exit 1
fi
