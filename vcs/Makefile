# =============================================================================
# Verum Compliance Suite (VCS) - Industrial-Grade Testing Makefile
# =============================================================================
#
# This Makefile provides comprehensive targets for building, testing, fuzzing,
# benchmarking, and reporting on the Verum language implementation.
#
# Usage:
#   make help              Show all available targets
#   make test              Run all tests (L0-L4)
#   make ci-gate           Full CI gate (blocking)
#   make fuzz              Run fuzzer (10 min default)
#
# Configuration:
#   PARALLEL=8             Number of parallel workers
#   TIMEOUT=30000          Default timeout (ms)
#   FUZZ_TIME=10m          Fuzz duration
#   VERBOSE=1              Enable verbose output
#
# Reference: VCS Spec Section 23 - Conformance Testing
# =============================================================================

# Strict shell settings
SHELL := /bin/bash
.ONESHELL:
.SHELLFLAGS := -eu -o pipefail -c
MAKEFLAGS += --warn-undefined-variables
MAKEFLAGS += --no-builtin-rules

# =============================================================================
# Directory Configuration
# =============================================================================

VCS_ROOT := $(shell pwd)
PROJECT_ROOT := $(shell cd $(VCS_ROOT)/.. && pwd)
RUNNER_DIR := $(VCS_ROOT)/runner
VTEST_DIR := $(RUNNER_DIR)/vtest
VFUZZ_DIR := $(RUNNER_DIR)/vfuzz
VBENCH_DIR := $(RUNNER_DIR)/vbench
CONFIG_FILE := $(VCS_ROOT)/config/vcs.toml
SPECS_DIR := $(VCS_ROOT)/specs
DIFF_DIR := $(VCS_ROOT)/differential
FUZZ_DIR := $(VCS_ROOT)/fuzz
BENCH_DIR := $(VCS_ROOT)/benchmarks
REPORT_DIR := $(VCS_ROOT)/reports
BASELINE_DIR := $(VCS_ROOT)/baselines
CACHE_DIR := $(VCS_ROOT)/.cache
SCRIPTS_DIR := $(VCS_ROOT)/scripts

# Compiler and tool directories
COMPILER_DIR := $(PROJECT_ROOT)/crates/verum_compiler
CRATES_DIR := $(PROJECT_ROOT)/crates

# =============================================================================
# Tool Binaries
# =============================================================================
# Use workspace target directory for all tools (built from project root)

VTEST := $(PROJECT_ROOT)/target/release/vtest
VFUZZ := $(PROJECT_ROOT)/target/release/vfuzz
VBENCH := $(PROJECT_ROOT)/target/release/vbench
VERUM_COMPILER := $(PROJECT_ROOT)/target/release/verum

# Debug versions
VTEST_DEBUG := $(PROJECT_ROOT)/target/debug/vtest
VFUZZ_DEBUG := $(PROJECT_ROOT)/target/debug/vfuzz
VBENCH_DEBUG := $(PROJECT_ROOT)/target/debug/vbench

# VCS bin directory with wrapper scripts
BIN_DIR := $(VCS_ROOT)/bin
VERUM_INTERPRETER := $(BIN_DIR)/verum-interpreter
VERUM_JIT := $(BIN_DIR)/verum-jit
VERUM_AOT := $(BIN_DIR)/verum-aot

# =============================================================================
# Configuration Variables (overridable)
# =============================================================================

# Parallelism - auto-detect CPU count
PARALLEL ?= $(shell nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)

# Timeouts (milliseconds)
TIMEOUT ?= 30000
TIMEOUT_EXTENDED ?= 600000
TIMEOUT_QUICK ?= 5000

# Fuzz settings
FUZZ_TIME ?= 10m
FUZZ_PARALLEL ?= $(PARALLEL)

# Benchmark settings
BENCH_WARMUP ?= 100
BENCH_ITERATIONS ?= 1000

# CI output format
CI_FORMAT ?= junit

# Verbosity
VERBOSE ?= 0
ifeq ($(VERBOSE),1)
  VFLAG := --verbose
else
  VFLAG :=
endif

# Pass rate thresholds (percentages)
L0_THRESHOLD := 100.0
L1_THRESHOLD := 100.0
L2_THRESHOLD := 95.0
L3_THRESHOLD := 90.0
L4_THRESHOLD := 0.0

# =============================================================================
# Colors for Terminal Output
# =============================================================================

# Only use colors if stdout is a terminal
ifneq ($(TERM),dumb)
  BLUE := \033[0;34m
  GREEN := \033[0;32m
  YELLOW := \033[1;33m
  RED := \033[0;31m
  CYAN := \033[0;36m
  MAGENTA := \033[0;35m
  BOLD := \033[1m
  NC := \033[0m
else
  BLUE :=
  GREEN :=
  YELLOW :=
  RED :=
  CYAN :=
  MAGENTA :=
  BOLD :=
  NC :=
endif

# Progress indicator
define PROGRESS
	@echo -e "$(BLUE)>>> $(1)$(NC)"
endef

define SUCCESS
	@echo -e "$(GREEN)[OK] $(1)$(NC)"
endef

define WARN
	@echo -e "$(YELLOW)[WARN] $(1)$(NC)"
endef

define ERROR
	@echo -e "$(RED)[ERROR] $(1)$(NC)"
endef

# =============================================================================
# Phony Targets Declaration
# =============================================================================

.PHONY: all help
.PHONY: build build-vtest build-vfuzz build-vbench build-compiler build-debug build-all
.PHONY: test test-l0 test-l1 test-l2 test-l3 test-l4 test-quick test-differential
.PHONY: test-critical test-standard test-extended test-all test-fast
.PHONY: test-lexer test-parser test-types test-ownership test-cbgr test-async test-modules test-refinement
.PHONY: test-memory-safety test-patterns test-verification
.PHONY: fuzz fuzz-lexer fuzz-parser fuzz-cbgr fuzz-continuous fuzz-overnight fuzz-types fuzz-smt fuzz-clean fuzz-all
.PHONY: bench bench-micro bench-macro bench-baseline bench-compare bench-save bench-validate
.PHONY: bench-compilation bench-runtime bench-memory bench-cbgr
.PHONY: report report-html report-json coverage
.PHONY: ci-gate ci-nightly ci-quick ci-full ci-critical ci-standard ci-differential ci-fuzz ci-bench
.PHONY: list list-l0 list-l1 list-l2 list-l3 list-l4 list-all list-by-tag
.PHONY: watch clean clean-all clean-cache clean-reports install-tools check-tools update-tools
.PHONY: validate validate-specs validate-config validate-thresholds validate-grammar
.PHONY: differential diff-all diff-cross-tier diff-randomized diff-memory diff-interp-vs-aot diff-interp-vs-aot-verbose diff-red-team

# =============================================================================
# Default Target
# =============================================================================

all: build test
	$(call SUCCESS,VCS build and test completed)

# =============================================================================
# Help Target
# =============================================================================

help:
	@echo ""
	@echo -e "$(BOLD)$(CYAN)==========================================================$(NC)"
	@echo -e "$(BOLD)$(CYAN)     Verum Compliance Suite (VCS) - Industrial Testing    $(NC)"
	@echo -e "$(BOLD)$(CYAN)==========================================================$(NC)"
	@echo ""
	@echo -e "$(BOLD)Build Targets:$(NC)"
	@echo "  make build           Build all VCS tools (vtest, vfuzz, vbench)"
	@echo "  make build-vtest     Build test runner only"
	@echo "  make build-vfuzz     Build fuzzer only"
	@echo "  make build-vbench    Build benchmarker only"
	@echo "  make build-compiler  Build the Verum compiler"
	@echo "  make build-debug     Build all tools in debug mode"
	@echo ""
	@echo -e "$(BOLD)Test Targets:$(NC)"
	@echo "  make test            Run all tests (L0-L4)"
	@echo "  make test-l0         Run L0 critical tests ($(L0_THRESHOLD)% required)"
	@echo "  make test-l1         Run L1 core tests ($(L1_THRESHOLD)% required)"
	@echo "  make test-l2         Run L2 standard tests ($(L2_THRESHOLD)%+ required)"
	@echo "  make test-l3         Run L3 extended tests ($(L3_THRESHOLD)%+ required)"
	@echo "  make test-l4         Run L4 performance tests (advisory)"
	@echo "  make test-quick      Fast smoke test (subset of L0)"
	@echo "  make test-differential Run differential tests (Tier 0 vs 3)"
	@echo ""
	@echo -e "$(BOLD)Category Tests:$(NC)"
	@echo "  make test-lexer      Run lexer tests only"
	@echo "  make test-parser     Run parser tests only"
	@echo "  make test-types      Run type system tests"
	@echo "  make test-ownership  Run ownership tests"
	@echo "  make test-cbgr       Run CBGR tests"
	@echo "  make test-async      Run async tests"
	@echo "  make test-modules    Run module tests"
	@echo "  make test-refinement Run refinement type tests"
	@echo ""
	@echo -e "$(BOLD)Fuzzing Targets:$(NC)"
	@echo "  make fuzz            Run fuzzer ($(FUZZ_TIME) default)"
	@echo "  make fuzz-lexer      Fuzz lexer only"
	@echo "  make fuzz-parser     Fuzz parser only"
	@echo "  make fuzz-cbgr       Fuzz CBGR system"
	@echo "  make fuzz-continuous Run continuous fuzzing"
	@echo "  make fuzz-overnight  Run overnight fuzzing (8h)"
	@echo ""
	@echo -e "$(BOLD)Benchmark Targets:$(NC)"
	@echo "  make bench           Run all benchmarks"
	@echo "  make bench-micro     Run micro-benchmarks"
	@echo "  make bench-macro     Run macro-benchmarks"
	@echo "  make bench-baseline  Save baseline results"
	@echo "  make bench-compare   Compare with baseline"
	@echo ""
	@echo -e "$(BOLD)Report Targets:$(NC)"
	@echo "  make report          Generate test report"
	@echo "  make report-html     Generate HTML report"
	@echo "  make report-json     Generate JSON report"
	@echo "  make coverage        Generate coverage report"
	@echo ""
	@echo -e "$(BOLD)CI Targets:$(NC)"
	@echo "  make ci-gate         Full CI gate (blocking)"
	@echo "  make ci-nightly      Nightly extended tests"
	@echo "  make ci-quick        Quick CI check (L0-L1)"
	@echo ""
	@echo -e "$(BOLD)Utility Targets:$(NC)"
	@echo "  make list            List all tests"
	@echo "  make watch           Watch mode"
	@echo "  make clean           Clean build artifacts"
	@echo "  make install-tools   Install required tools"
	@echo ""
	@echo -e "$(BOLD)Configuration (environment variables):$(NC)"
	@echo "  PARALLEL=$(PARALLEL)           Parallel workers"
	@echo "  TIMEOUT=$(TIMEOUT)         Default timeout (ms)"
	@echo "  FUZZ_TIME=$(FUZZ_TIME)          Fuzz duration"
	@echo "  VERBOSE=$(VERBOSE)             Verbose output (0 or 1)"
	@echo ""
	@echo -e "$(BOLD)Examples:$(NC)"
	@echo "  make test-l0 PARALLEL=4"
	@echo "  make fuzz FUZZ_TIME=30m"
	@echo "  make bench-compare"
	@echo "  make ci-gate VERBOSE=1"
	@echo ""

# =============================================================================
# Build Targets
# =============================================================================

# Build all VCS tools
build: build-vtest build-vfuzz build-vbench
	$(call SUCCESS,All VCS tools built successfully)

# Build test runner (from workspace root)
build-vtest: $(VTEST)
$(VTEST): $(VTEST_DIR)/Cargo.toml $(shell find $(VTEST_DIR)/src -name '*.rs' 2>/dev/null) $(shell find $(CRATES_DIR) -name '*.rs' 2>/dev/null)
	$(call PROGRESS,Building vtest (release)...)
	@cd $(PROJECT_ROOT) && cargo build --release -p vtest $(VFLAG)
	$(call SUCCESS,vtest built: $@)

# Build fuzzer (from workspace root)
build-vfuzz: $(VFUZZ)
$(VFUZZ): $(VFUZZ_DIR)/Cargo.toml $(shell find $(VFUZZ_DIR)/src -name '*.rs' 2>/dev/null) $(shell find $(CRATES_DIR) -name '*.rs' 2>/dev/null)
	$(call PROGRESS,Building vfuzz (release)...)
	@cd $(PROJECT_ROOT) && cargo build --release -p verum_vfuzz $(VFLAG)
	$(call SUCCESS,vfuzz built: $@)

# Build benchmarker (from workspace root)
build-vbench: $(VBENCH)
$(VBENCH): $(VBENCH_DIR)/Cargo.toml $(shell find $(VBENCH_DIR)/src -name '*.rs' 2>/dev/null) $(shell find $(CRATES_DIR) -name '*.rs' 2>/dev/null)
	$(call PROGRESS,Building vbench (release)...)
	@cd $(PROJECT_ROOT) && cargo build --release -p vbench $(VFLAG)
	$(call SUCCESS,vbench built: $@)

# Build the Verum compiler (and the verum CLI binary)
build-compiler:
	$(call PROGRESS,Building Verum compiler...)
	@cd $(PROJECT_ROOT) && cargo build --release -p verum_compiler -p verum_cli $(VFLAG)
	$(call SUCCESS,Verum compiler built)
	$(call PROGRESS,Verifying VCS wrapper scripts...)
	@chmod +x $(VERUM_INTERPRETER) $(VERUM_JIT) $(VERUM_AOT) 2>/dev/null || true
	@$(VERUM_INTERPRETER) --version >/dev/null 2>&1 && echo "  verum-interpreter: OK" || echo "  verum-interpreter: not found"
	@$(VERUM_JIT) --version >/dev/null 2>&1 && echo "  verum-jit: OK" || echo "  verum-jit: not found"
	@$(VERUM_AOT) --version >/dev/null 2>&1 && echo "  verum-aot: OK" || echo "  verum-aot: not found"

# Build all tools in debug mode
build-debug:
	$(call PROGRESS,Building all tools in debug mode...)
	@cd $(VTEST_DIR) && cargo build $(VFLAG)
	@cd $(VFUZZ_DIR) && cargo build --features parallel $(VFLAG)
	@cd $(VBENCH_DIR) && cargo build $(VFLAG)
	$(call SUCCESS,Debug builds complete)

# Build everything (tools + compiler)
build-all: build build-compiler
	$(call SUCCESS,Full build complete)

# =============================================================================
# Test Targets
# =============================================================================

# Run all tests (L0-L4)
test: test-all

test-all: build
	$(call PROGRESS,Running all VCS tests (L0-L4)...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level all \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		$(VFLAG) \
		$(SPECS_DIR)
	$(call SUCCESS,All tests completed)

# L0 Critical tests - 100% pass required
test-l0: build
	$(call PROGRESS,Running L0 Critical tests ($(L0_THRESHOLD)% required)...)
	@$(VTEST) run \
		--level L0 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		$(VFLAG) \
		$(SPECS_DIR)/L0-critical
	$(call SUCCESS,L0 Critical tests passed)

# L1 Core tests - 100% pass required
test-l1: build
	$(call PROGRESS,Running L1 Core tests ($(L1_THRESHOLD)% required)...)
	@$(VTEST) run \
		--level L1 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		$(VFLAG) \
		$(SPECS_DIR)/L1-core
	$(call SUCCESS,L1 Core tests passed)

# L2 Standard tests - 95%+ pass required
test-l2: build
	$(call PROGRESS,Running L2 Standard tests ($(L2_THRESHOLD)%+ required)...)
	@$(VTEST) run \
		--level L2 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		$(VFLAG) \
		$(SPECS_DIR)/L2-standard
	$(call SUCCESS,L2 Standard tests passed)

# L3 Extended tests - 90%+ pass required
test-l3: build
	$(call PROGRESS,Running L3 Extended tests ($(L3_THRESHOLD)%+ required)...)
	@$(VTEST) run \
		--level L3 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT_EXTENDED) \
		$(VFLAG) \
		$(SPECS_DIR)/L3-extended
	$(call SUCCESS,L3 Extended tests passed)

# L4 Performance tests - advisory only
test-l4: build
	$(call PROGRESS,Running L4 Performance tests (advisory)...)
	@$(VTEST) run \
		--level L4 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT_EXTENDED) \
		$(VFLAG) \
		$(SPECS_DIR)/L4-performance || true
	$(call SUCCESS,L4 Performance tests completed)

# Quick smoke test - fast subset of L0
test-quick: build
	$(call PROGRESS,Running quick smoke test...)
	@$(VTEST) run \
		--level L0 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT_QUICK) \
		--fail-fast \
		--filter "lexer|parser" \
		$(VFLAG) \
		$(SPECS_DIR)/L0-critical
	$(call SUCCESS,Quick smoke test passed)

# Differential testing (Tier 0 interpreter vs Tier 3 AOT)
test-differential: build
	$(call PROGRESS,Running differential tests (Tier 0 vs Tier 3)...)
	@$(VTEST) run \
		$(DIFF_DIR) \
		--tier 0,3 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Differential tests passed)

# Composite test targets
test-critical: build
	$(call PROGRESS,Running Critical tests (L0 + L1)...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level L0,L1 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		--format $(CI_FORMAT) \
		--output $(REPORT_DIR)/critical-results.xml \
		$(VFLAG)
	$(call SUCCESS,Critical tests passed)

test-standard: build
	$(call PROGRESS,Running Standard tests (L2)...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level L2 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		--format $(CI_FORMAT) \
		--output $(REPORT_DIR)/standard-results.xml \
		$(VFLAG)
	$(call SUCCESS,Standard tests passed)

test-extended: build
	$(call PROGRESS,Running Extended tests (L3 + L4)...)
	@$(VTEST) run \
		--level L3,L4 \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT_EXTENDED) \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Extended tests completed)

# Fast test - just L0 lexer/parser for rapid iteration
test-fast: build
	$(call PROGRESS,Running fast tests (lexer + parser)...)
	@$(VTEST) run \
		$(SPECS_DIR)/L0-critical/lexer \
		$(SPECS_DIR)/L0-critical/parser \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT_QUICK) \
		--config $(CONFIG_FILE) \
		--fail-fast \
		$(VFLAG)
	$(call SUCCESS,Fast tests passed)

# =============================================================================
# Category Test Targets
# =============================================================================

test-lexer: build
	$(call PROGRESS,Running lexer tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/lexer --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Lexer tests passed)

test-parser: build
	$(call PROGRESS,Running parser tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/parser --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Parser tests passed)

test-types: build
	$(call PROGRESS,Running type system tests...)
	@$(VTEST) run $(SPECS_DIR)/L1-core/types --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Type system tests passed)

test-ownership: build
	$(call PROGRESS,Running ownership tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/ownership --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Ownership tests passed)

test-cbgr: build
	$(call PROGRESS,Running CBGR tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/ownership/cbgr --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/memory-safety/cbgr --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,CBGR tests passed)

test-async: build
	$(call PROGRESS,Running async tests...)
	@$(VTEST) run $(SPECS_DIR)/L2-standard/async --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Async tests passed)

test-modules: build
	$(call PROGRESS,Running module tests...)
	@$(VTEST) run $(SPECS_DIR)/L2-standard/modules --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Module tests passed)

test-refinement: build
	$(call PROGRESS,Running refinement type tests...)
	@$(VTEST) run $(SPECS_DIR)/L1-core/refinement --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Refinement type tests passed)

test-memory-safety: build
	$(call PROGRESS,Running memory safety tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/memory-safety --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Memory safety tests passed)

test-patterns: build
	$(call PROGRESS,Running pattern matching tests...)
	@$(VTEST) run $(SPECS_DIR)/L0-critical/parser/patterns --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	@$(VTEST) run $(SPECS_DIR)/L1-core/patterns --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Pattern matching tests passed)

test-verification: build
	$(call PROGRESS,Running verification tests...)
	@$(VTEST) run $(SPECS_DIR)/L2-standard/verification --parallel $(PARALLEL) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Verification tests passed)

# =============================================================================
# Fuzzing Targets
# =============================================================================

# Default fuzz target (10 min)
fuzz: fuzz-all

# Run all fuzz targets
fuzz-all: build
	$(call PROGRESS,Running all fuzz targets for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes
	@$(VFUZZ) run \
		--targets all \
		--duration $(FUZZ_TIME) \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/seeds \
		--crashes $(FUZZ_DIR)/crashes \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Fuzzing completed)

# Fuzz lexer only
fuzz-lexer: build
	$(call PROGRESS,Fuzzing lexer for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes/lexer
	@mkdir -p $(FUZZ_DIR)/seeds/lexer_seeds
	@$(VFUZZ) run \
		--targets lexer \
		--duration $(FUZZ_TIME) \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/seeds/lexer_seeds \
		--crashes $(FUZZ_DIR)/crashes/lexer \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Lexer fuzzing completed)

# Fuzz parser only
fuzz-parser: build
	$(call PROGRESS,Fuzzing parser for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes/parser
	@mkdir -p $(FUZZ_DIR)/seeds/parser_seeds
	@$(VFUZZ) run \
		--targets parser \
		--duration $(FUZZ_TIME) \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/seeds/parser_seeds \
		--crashes $(FUZZ_DIR)/crashes/parser \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Parser fuzzing completed)

# Fuzz CBGR memory safety system
fuzz-cbgr: build
	$(call PROGRESS,Fuzzing CBGR for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes/cbgr
	@mkdir -p $(FUZZ_DIR)/seeds/ownership_seeds
	@$(VFUZZ) run \
		--targets cbgr \
		--duration $(FUZZ_TIME) \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/seeds/ownership_seeds \
		--crashes $(FUZZ_DIR)/crashes/cbgr \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,CBGR fuzzing completed)

# Fuzz type system
fuzz-types: build
	$(call PROGRESS,Fuzzing type system for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes/types
	@mkdir -p $(FUZZ_DIR)/seeds/type_seeds
	@$(VFUZZ) run \
		--targets types \
		--duration $(FUZZ_TIME) \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/seeds/type_seeds \
		--crashes $(FUZZ_DIR)/crashes/types \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,Type system fuzzing completed)

# Fuzz SMT solver integration
fuzz-smt: build
	$(call PROGRESS,Fuzzing SMT solver for $(FUZZ_TIME)...)
	@mkdir -p $(FUZZ_DIR)/crashes/smt
	@$(VFUZZ) run \
		--targets smt \
		--duration $(FUZZ_TIME) \
		--parallel 2 \
		--corpus $(FUZZ_DIR)/seeds \
		--crashes $(FUZZ_DIR)/crashes/smt \
		--config $(CONFIG_FILE) \
		$(VFLAG)
	$(call SUCCESS,SMT fuzzing completed)

# Run continuous fuzzing (uses shell script)
fuzz-continuous: build
	$(call PROGRESS,Starting continuous fuzzing...)
	@chmod +x $(SCRIPTS_DIR)/fuzz-continuous.sh
	@$(SCRIPTS_DIR)/fuzz-continuous.sh \
		--targets all \
		--duration 1h \
		--parallel $(FUZZ_PARALLEL) \
		--corpus $(FUZZ_DIR)/corpus \
		--crashes $(FUZZ_DIR)/crashes \
		--minimize \
		--report
	$(call SUCCESS,Continuous fuzzing completed)

# Overnight fuzzing (8 hours)
fuzz-overnight: build
	$(call PROGRESS,Starting overnight fuzzing (8h)...)
	@chmod +x $(SCRIPTS_DIR)/fuzz-continuous.sh
	@$(SCRIPTS_DIR)/fuzz-continuous.sh \
		--targets all \
		--duration 4h \
		--cycles 2 \
		--parallel 8 \
		--corpus $(FUZZ_DIR)/corpus \
		--crashes $(FUZZ_DIR)/crashes \
		--minimize \
		--report \
		--notify
	$(call SUCCESS,Overnight fuzzing completed)

# Clean fuzz artifacts (keep corpus)
fuzz-clean:
	$(call WARN,Cleaning fuzz artifacts...)
	@rm -rf $(FUZZ_DIR)/crashes/*
	@rm -rf $(FUZZ_DIR)/artifacts/*
	@rm -f $(FUZZ_DIR)/reports/*.log
	$(call SUCCESS,Fuzz artifacts cleaned)

# =============================================================================
# Benchmark Targets
# =============================================================================

# Run all benchmarks
bench: build
	$(call PROGRESS,Running all benchmarks...)
	@mkdir -p $(REPORT_DIR)
	@$(VBENCH) run \
		--suite all \
		--iterations $(BENCH_ITERATIONS) \
		--warmup $(BENCH_WARMUP) \
		--config $(CONFIG_FILE) \
		--format json \
		--output $(REPORT_DIR)/benchmark-results.json \
		$(VFLAG)
	$(call SUCCESS,Benchmarks completed: $(REPORT_DIR)/benchmark-results.json)

# Run micro-benchmarks
bench-micro: build
	$(call PROGRESS,Running micro-benchmarks...)
	@mkdir -p $(REPORT_DIR)
	@$(VBENCH) run \
		--suite micro \
		--iterations $(BENCH_ITERATIONS) \
		--warmup $(BENCH_WARMUP) \
		--path $(BENCH_DIR)/micro \
		--config $(CONFIG_FILE) \
		--format console \
		$(VFLAG)
	$(call SUCCESS,Micro-benchmarks completed)

# Run macro-benchmarks (real-world scenarios)
bench-macro: build
	$(call PROGRESS,Running macro-benchmarks...)
	@mkdir -p $(REPORT_DIR)
	@$(VBENCH) run \
		--suite macro \
		--iterations 100 \
		--warmup 10 \
		--path $(BENCH_DIR)/macro \
		--config $(CONFIG_FILE) \
		--format console \
		$(VFLAG)
	$(call SUCCESS,Macro-benchmarks completed)

# Save current results as baseline
bench-baseline: bench
	$(call PROGRESS,Saving benchmark baseline...)
	@mkdir -p $(BASELINE_DIR)
	@cp $(REPORT_DIR)/benchmark-results.json $(BASELINE_DIR)/benchmark-baseline.json
	@echo "Baseline saved: $$(date -u +"%Y-%m-%d %H:%M:%S UTC")" >> $(BASELINE_DIR)/baseline.log
	$(call SUCCESS,Baseline saved: $(BASELINE_DIR)/benchmark-baseline.json)

# Alias for bench-baseline
bench-save: bench-baseline

# Compare current benchmarks with baseline
bench-compare: build
	$(call PROGRESS,Comparing benchmarks to baseline...)
	@if [ ! -f $(BASELINE_DIR)/benchmark-baseline.json ]; then \
		echo -e "$(RED)No baseline found. Run 'make bench-baseline' first.$(NC)"; \
		exit 1; \
	fi
	@mkdir -p $(REPORT_DIR)
	@$(VBENCH) compare \
		--current $(REPORT_DIR)/benchmark-results.json \
		--baseline $(BASELINE_DIR)/benchmark-baseline.json \
		--threshold 10.0 \
		--format markdown \
		--output $(REPORT_DIR)/benchmark-comparison.md \
		$(VFLAG)
	$(call SUCCESS,Comparison report: $(REPORT_DIR)/benchmark-comparison.md)

# Validate against performance thresholds
bench-validate: build
	$(call PROGRESS,Validating performance thresholds...)
	@if [ ! -f $(REPORT_DIR)/benchmark-results.json ]; then \
		echo -e "$(YELLOW)No results found. Running benchmarks first...$(NC)"; \
		$(MAKE) bench; \
	fi
	@$(VBENCH) validate \
		--thresholds $(VCS_ROOT)/config/thresholds.toml \
		--results $(REPORT_DIR)/benchmark-results.json \
		$(VFLAG)
	$(call SUCCESS,Performance validation passed)

# Specific benchmark suites
bench-compilation: build
	$(call PROGRESS,Running compilation benchmarks...)
	@$(VBENCH) run --suite compilation --config $(CONFIG_FILE) --format console $(VFLAG)
	$(call SUCCESS,Compilation benchmarks completed)

bench-runtime: build
	$(call PROGRESS,Running runtime benchmarks...)
	@$(VBENCH) run --suite runtime --config $(CONFIG_FILE) --format console $(VFLAG)
	$(call SUCCESS,Runtime benchmarks completed)

bench-memory: build
	$(call PROGRESS,Running memory benchmarks...)
	@$(VBENCH) run --suite memory --config $(CONFIG_FILE) --format console $(VFLAG)
	$(call SUCCESS,Memory benchmarks completed)

bench-cbgr: build
	$(call PROGRESS,Running CBGR tier latency benchmarks...)
	@$(VBENCH) run --suite cbgr --config $(CONFIG_FILE) --format console $(VFLAG)
	$(call SUCCESS,CBGR benchmarks completed)

# =============================================================================
# Report Targets
# =============================================================================

# Generate all reports
report: report-html report-json
	$(call SUCCESS,All reports generated in $(REPORT_DIR))

# Generate HTML report
report-html: build
	$(call PROGRESS,Generating HTML report...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level all \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		--format html \
		--output $(REPORT_DIR)/vcs-report.html \
		$(VFLAG) || true
	$(call SUCCESS,HTML report: $(REPORT_DIR)/vcs-report.html)

# Generate JSON report
report-json: build
	$(call PROGRESS,Generating JSON report...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level all \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		--format json \
		--output $(REPORT_DIR)/vcs-results.json \
		$(VFLAG) || true
	$(call SUCCESS,JSON report: $(REPORT_DIR)/vcs-results.json)

# Generate JUnit XML report (for CI systems)
report-junit: build
	$(call PROGRESS,Generating JUnit XML report...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run \
		--level all \
		--parallel $(PARALLEL) \
		--timeout $(TIMEOUT) \
		--config $(CONFIG_FILE) \
		--format junit \
		--output $(REPORT_DIR)/vcs-results.xml \
		$(VFLAG) || true
	$(call SUCCESS,JUnit report: $(REPORT_DIR)/vcs-results.xml)

# Generate code coverage report
coverage:
	$(call PROGRESS,Generating code coverage report...)
	@mkdir -p $(REPORT_DIR)/coverage
	@cd $(PROJECT_ROOT) && \
		RUSTFLAGS="-C instrument-coverage" cargo test --workspace && \
		grcov . -s . --binary-path ./target/debug/ -t html --branch --ignore-not-existing -o $(VCS_ROOT)/$(REPORT_DIR)/coverage || \
		cargo llvm-cov --all-features --workspace --html --output-dir $(VCS_ROOT)/$(REPORT_DIR)/coverage
	$(call SUCCESS,Coverage report: $(REPORT_DIR)/coverage/index.html)

# =============================================================================
# CI Gate Targets
# =============================================================================

# Full CI gate - blocking checks (must pass for merge)
ci-gate: build
	$(call PROGRESS,Running CI gate checks...)
	@mkdir -p $(REPORT_DIR)
	@echo "=================================================================="
	@echo "                    VCS CI Gate - Full Suite                      "
	@echo "=================================================================="
	@echo ""
	@# L0 Critical (100% required)
	@echo -e "$(CYAN)Step 1/5: L0 Critical Tests (100% required)$(NC)"
	@$(VTEST) run --level L0 --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/ci-l0.xml --config $(CONFIG_FILE) --threshold $(L0_THRESHOLD)
	@echo ""
	@# L1 Core (100% required)
	@echo -e "$(CYAN)Step 2/5: L1 Core Tests (100% required)$(NC)"
	@$(VTEST) run --level L1 --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/ci-l1.xml --config $(CONFIG_FILE) --threshold $(L1_THRESHOLD)
	@echo ""
	@# L2 Standard (95% required)
	@echo -e "$(CYAN)Step 3/5: L2 Standard Tests (95%+ required)$(NC)"
	@$(VTEST) run --level L2 --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/ci-l2.xml --config $(CONFIG_FILE) --threshold $(L2_THRESHOLD)
	@echo ""
	@# Differential tests
	@echo -e "$(CYAN)Step 4/5: Differential Tests$(NC)"
	@$(VTEST) run $(DIFF_DIR) --tier 0,3 --format junit \
		--output $(REPORT_DIR)/ci-diff.xml --config $(CONFIG_FILE)
	@echo ""
	@# Quick fuzz check
	@echo -e "$(CYAN)Step 5/5: Fuzz Smoke Test (1m)$(NC)"
	@$(VFUZZ) run --targets lexer,parser --duration 1m --parallel 4 \
		--corpus $(FUZZ_DIR)/seeds --crashes $(REPORT_DIR)/ci-crashes \
		--config $(CONFIG_FILE) || true
	@echo ""
	@echo "=================================================================="
	$(call SUCCESS,CI gate passed!)

# Nightly extended tests (deeper testing)
ci-nightly: build
	$(call PROGRESS,Running nightly CI suite...)
	@mkdir -p $(REPORT_DIR)
	@echo "=================================================================="
	@echo "                 VCS Nightly Suite - Extended Tests               "
	@echo "=================================================================="
	@echo ""
	@# All level tests
	@echo -e "$(CYAN)Step 1/4: All Level Tests$(NC)"
	@$(VTEST) run --level all --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/nightly-all.xml --config $(CONFIG_FILE)
	@echo ""
	@# Differential tests (all tier combinations)
	@echo -e "$(CYAN)Step 2/4: Cross-Tier Differential Tests$(NC)"
	@for combo in "0,1" "1,2" "2,3" "0,3"; do \
		echo "  Testing Tier $${combo%,*} vs Tier $${combo#*,}..."; \
		$(VTEST) run $(DIFF_DIR) --tier $$combo --parallel $(PARALLEL) --config $(CONFIG_FILE) || true; \
	done
	@echo ""
	@# Extended fuzzing
	@echo -e "$(CYAN)Step 3/4: Extended Fuzzing (2h)$(NC)"
	@$(VFUZZ) run --targets all --duration 2h --parallel 8 \
		--corpus $(FUZZ_DIR)/seeds --crashes $(REPORT_DIR)/nightly-crashes \
		--config $(CONFIG_FILE) || true
	@echo ""
	@# Benchmarks with comparison
	@echo -e "$(CYAN)Step 4/4: Benchmarks$(NC)"
	@$(VBENCH) run --suite all --config $(CONFIG_FILE) \
		--format json --output $(REPORT_DIR)/nightly-bench.json
	@if [ -f $(BASELINE_DIR)/benchmark-baseline.json ]; then \
		$(VBENCH) compare --current $(REPORT_DIR)/nightly-bench.json \
			--baseline $(BASELINE_DIR)/benchmark-baseline.json \
			--threshold 15.0 --format markdown \
			--output $(REPORT_DIR)/nightly-comparison.md || true; \
	fi
	@echo ""
	@echo "=================================================================="
	$(call SUCCESS,Nightly suite completed)

# Quick CI check (for PRs)
ci-quick: build
	$(call PROGRESS,Running quick CI check...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run --level L0,L1 --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/quick.xml --config $(CONFIG_FILE) --fail-fast
	$(call SUCCESS,Quick CI check passed)

# Full CI (all tests + benchmarks)
ci-full: build
	$(call PROGRESS,Running full CI suite...)
	@mkdir -p $(REPORT_DIR)
	@$(VTEST) run --level all --parallel $(PARALLEL) --format junit \
		--output $(REPORT_DIR)/full.xml --config $(CONFIG_FILE)
	@$(VTEST) run $(DIFF_DIR) --tier 0,3 --format junit \
		--output $(REPORT_DIR)/full-diff.xml --config $(CONFIG_FILE)
	@$(VBENCH) run --config $(CONFIG_FILE) --format json \
		--output $(REPORT_DIR)/full-bench.json
	$(call SUCCESS,Full CI suite completed)

# CI target aliases
ci-critical: test-critical
ci-standard: test-standard
ci-differential: test-differential

ci-fuzz: build
	$(call PROGRESS,Running CI fuzz tests...)
	@$(VFUZZ) run --targets all --duration $(FUZZ_TIME) --parallel 4 \
		--corpus $(FUZZ_DIR)/seeds --crashes $(FUZZ_DIR)/crashes \
		--config $(CONFIG_FILE)

ci-bench: bench

# =============================================================================
# Differential Testing Targets
# =============================================================================

differential: test-differential

# Run cross-impl differential tests via shell runner (interpreter vs AOT)
diff-interp-vs-aot:
	$(call PROGRESS,Running interpreter vs AOT differential tests...)
	@bash $(DIFF_DIR)/run_differential.sh $(DIFF_DIR)/cross-impl
	$(call SUCCESS,Interpreter vs AOT differential tests completed)

diff-interp-vs-aot-verbose:
	$(call PROGRESS,Running interpreter vs AOT differential tests (verbose)...)
	@bash $(DIFF_DIR)/run_differential.sh -v $(DIFF_DIR)/cross-impl
	$(call SUCCESS,Interpreter vs AOT differential tests completed)

diff-all: diff-cross-tier

# Run cross-tier differential tests (all combinations)
diff-cross-tier: build
	$(call PROGRESS,Running cross-tier differential tests...)
	@mkdir -p $(REPORT_DIR)/differential
	@for combo in "0,1" "1,2" "2,3" "0,2" "0,3"; do \
		echo -e "$(CYAN)Testing Tier $${combo%,*} vs Tier $${combo#*,}...$(NC)"; \
		$(VTEST) run $(DIFF_DIR) --tier $$combo --parallel $(PARALLEL) \
			--config $(CONFIG_FILE) \
			--format json --output $(REPORT_DIR)/differential/tier-$${combo%,*}-vs-$${combo#*,}.json || true; \
	done
	$(call SUCCESS,Cross-tier differential tests completed)

# Run randomized differential tests
diff-randomized: build
	$(call PROGRESS,Running randomized differential tests...)
	@mkdir -p $(REPORT_DIR)/differential $(DIFF_DIR)/generated
	@$(VFUZZ) generate \
		--count 500 \
		--max-depth 8 \
		--output $(DIFF_DIR)/generated/ \
		--config $(CONFIG_FILE)
	@$(VTEST) run $(DIFF_DIR)/generated --tier 0,3 --parallel $(PARALLEL) \
		--config $(CONFIG_FILE)
	$(call SUCCESS,Randomized differential tests completed)

# Run memory behavior differential tests
diff-memory: build
	$(call PROGRESS,Running memory differential tests...)
	@$(VTEST) run $(DIFF_DIR)/tests/memory.vr --tier 0,3 --parallel 4 \
		--timeout $(TIMEOUT_EXTENDED) --config $(CONFIG_FILE)
	$(call SUCCESS,Memory differential tests completed)

# Run the red-team audit-armor guardrails on Tier-0 + Tier-1.
# Each .vr file under vcs/specs/L*/red-team-*-{architecture,implementation,perf}/
# pins one audit-confirmed defense (overflow recipe / decoder canonicality /
# dishonest-stub / Int.MIN unary-negation / constant-time crypto / etc.).
# Any divergence between Tier-0 and Tier-1 on these tests is a hard failure —
# it means the audit defense surfaces differently across tiers.
.PHONY: diff-red-team
diff-red-team: build
	$(call PROGRESS,Running red-team audit-armor differential tests...)
	@for dir in \
		$(VCS_ROOT)/specs/L0-critical/modules \
		$(VCS_ROOT)/specs/L2-standard/red-team-1-architecture \
		$(VCS_ROOT)/specs/L2-standard/red-team-2-implementation \
		$(VCS_ROOT)/specs/L4-performance/red-team-3-perf; do \
		if [ -d "$$dir" ]; then \
			echo -e "$(CYAN)Tier 0 vs Tier 1 over $$dir$(NC)"; \
			$(VTEST) run "$$dir" --tier 0,1 --parallel $(PARALLEL) \
				--config $(CONFIG_FILE) || true; \
		fi; \
	done
	$(call SUCCESS,Red-team audit-armor differential tests completed)

# =============================================================================
# List Targets
# =============================================================================

# List all tests
list: build
	@$(VTEST) list --config $(CONFIG_FILE)

list-all: list

# List by level
list-l0: build
	@$(VTEST) list --level L0 --config $(CONFIG_FILE)

list-l1: build
	@$(VTEST) list --level L1 --config $(CONFIG_FILE)

list-l2: build
	@$(VTEST) list --level L2 --config $(CONFIG_FILE)

list-l3: build
	@$(VTEST) list --level L3 --config $(CONFIG_FILE)

list-l4: build
	@$(VTEST) list --level L4 --config $(CONFIG_FILE)

# List by tag
list-by-tag: build
	@echo "Available tags:"
	@$(VTEST) list --show-tags --config $(CONFIG_FILE)

# =============================================================================
# Watch Mode
# =============================================================================

watch: build
	$(call PROGRESS,Starting watch mode...)
	@$(VTEST) watch \
		--path $(SPECS_DIR) \
		--config $(CONFIG_FILE) \
		$(VFLAG)

# =============================================================================
# Validation Targets
# =============================================================================

validate: validate-specs validate-config validate-grammar

validate-specs: build
	$(call PROGRESS,Validating VCS specifications...)
	@$(VTEST) validate --specs $(SPECS_DIR) --config $(CONFIG_FILE) $(VFLAG)
	$(call SUCCESS,Specifications validated)

validate-config:
	$(call PROGRESS,Validating VCS configuration...)
	@if [ ! -f $(CONFIG_FILE) ]; then \
		echo -e "$(RED)Config file not found: $(CONFIG_FILE)$(NC)"; \
		exit 1; \
	fi
	@echo "  Config file: $(CONFIG_FILE)"
	$(call SUCCESS,Configuration validated)

validate-thresholds:
	$(call PROGRESS,Validating performance thresholds...)
	@if [ ! -f $(VCS_ROOT)/config/thresholds.toml ]; then \
		echo -e "$(RED)Thresholds file not found$(NC)"; \
		exit 1; \
	fi
	$(call SUCCESS,Thresholds validated)

validate-grammar:
	$(call PROGRESS,Validating Verum grammar...)
	@if [ ! -f $(PROJECT_ROOT)/grammar/verum.ebnf ]; then \
		echo -e "$(YELLOW)Grammar file not found: $(PROJECT_ROOT)/grammar/verum.ebnf$(NC)"; \
	else \
		echo "  Grammar file: $(PROJECT_ROOT)/grammar/verum.ebnf"; \
	fi
	$(call SUCCESS,Grammar validation complete)

# =============================================================================
# Clean Targets
# =============================================================================

# Clean build artifacts
clean:
	$(call PROGRESS,Cleaning build artifacts...)
	@cd $(VTEST_DIR) && cargo clean
	@cd $(VFUZZ_DIR) && cargo clean
	@cd $(VBENCH_DIR) && cargo clean
	$(call SUCCESS,Build artifacts cleaned)

# Clean everything including reports and cache
clean-all: clean clean-cache clean-reports
	$(call SUCCESS,Full clean completed)

# Clean cache only
clean-cache:
	$(call PROGRESS,Cleaning cache...)
	@rm -rf $(CACHE_DIR)
	$(call SUCCESS,Cache cleaned)

# Clean reports only
clean-reports:
	$(call PROGRESS,Cleaning reports...)
	@rm -rf $(REPORT_DIR)/*
	$(call SUCCESS,Reports cleaned)

# =============================================================================
# Tool Management
# =============================================================================

# Check for required tools
check-tools:
	$(call PROGRESS,Checking required tools...)
	@echo "Checking cargo..."
	@command -v cargo >/dev/null 2>&1 || { echo -e "$(RED)cargo not found$(NC)"; exit 1; }
	@echo "  cargo: $$(cargo --version)"
	@echo "Checking rustc..."
	@command -v rustc >/dev/null 2>&1 || { echo -e "$(RED)rustc not found$(NC)"; exit 1; }
	@echo "  rustc: $$(rustc --version)"
	@echo "Checking optional tools..."
	@command -v z3 >/dev/null 2>&1 && echo "  z3: $$(z3 --version | head -1)" || echo "  z3: not found (optional)"
	@command -v cargo-fuzz >/dev/null 2>&1 && echo "  cargo-fuzz: found" || echo "  cargo-fuzz: not found (install with: cargo install cargo-fuzz)"
	@command -v cargo-llvm-cov >/dev/null 2>&1 && echo "  cargo-llvm-cov: found" || echo "  cargo-llvm-cov: not found (install with: cargo install cargo-llvm-cov)"
	$(call SUCCESS,Tool check complete)

# Install VCS tools to cargo bin directory
install-tools: build
	$(call PROGRESS,Installing VCS tools...)
	@cargo install --path $(VTEST_DIR) --force
	@cargo install --path $(VFUZZ_DIR) --force
	@cargo install --path $(VBENCH_DIR) --force
	$(call SUCCESS,VCS tools installed)

# Update cargo tools
update-tools:
	$(call PROGRESS,Updating tools...)
	@cargo install cargo-fuzz --locked 2>/dev/null || true
	@cargo install cargo-llvm-cov --locked 2>/dev/null || true
	$(call SUCCESS,Tools updated)

# =============================================================================
# Development Helpers
# =============================================================================

.PHONY: dev dev-loop new-test

# Development mode - build debug and run fast tests
dev: build-debug
	$(call PROGRESS,Running development tests...)
	@$(VTEST_DEBUG) run --level L0 --parallel $(PARALLEL) \
		--timeout $(TIMEOUT_QUICK) --config $(CONFIG_FILE) --fail-fast

# Development loop - watch and run tests on change
dev-loop: build-debug
	$(call PROGRESS,Starting development loop (Ctrl+C to stop)...)
	@while true; do \
		$(MAKE) dev; \
		echo ""; \
		echo -e "$(CYAN)Waiting for changes... (Ctrl+C to stop)$(NC)"; \
		sleep 2; \
	done

# Create a new test file scaffold
new-test:
	@echo "Usage: make new-test NAME=test_name LEVEL=L0"
	@echo "Example: make new-test NAME=my_test LEVEL=L1"
ifdef NAME
ifdef LEVEL
	@mkdir -p $(SPECS_DIR)/$(LEVEL)-*/
	@echo "// @test: unit" > $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "// @tier: 0" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "// @level: $(LEVEL)" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "// @expect: pass" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "fn main() {" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "    // TODO: Implement test" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	@echo "}" >> $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr
	$(call SUCCESS,Created test: $(SPECS_DIR)/$(LEVEL)-*/$(NAME).vr)
endif
endif

# =============================================================================
# Summary Targets
# =============================================================================

.PHONY: status summary

# Show current VCS status
status:
	@echo ""
	@echo -e "$(BOLD)$(CYAN)VCS Status$(NC)"
	@echo "==========="
	@echo ""
	@echo "Test Specs:"
	@echo "  L0-critical: $$(find $(SPECS_DIR)/L0-critical -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  L1-core:     $$(find $(SPECS_DIR)/L1-core -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  L2-standard: $$(find $(SPECS_DIR)/L2-standard -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  L3-extended: $$(find $(SPECS_DIR)/L3-extended -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  L4-performance: $$(find $(SPECS_DIR)/L4-performance -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo ""
	@echo "Differential Tests:"
	@echo "  Tests: $$(find $(DIFF_DIR) -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo ""
	@echo "Fuzz Seeds:"
	@echo "  Lexer:  $$(find $(FUZZ_DIR)/seeds/lexer_seeds -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  Parser: $$(find $(FUZZ_DIR)/seeds/parser_seeds -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo "  Types:  $$(find $(FUZZ_DIR)/seeds/type_seeds -name '*.vr' 2>/dev/null | wc -l | tr -d ' ') files"
	@echo ""
	@echo "Build Status:"
	@[ -f $(VTEST) ] && echo "  vtest:  built (release)" || echo "  vtest:  not built"
	@[ -f $(VFUZZ) ] && echo "  vfuzz:  built (release)" || echo "  vfuzz:  not built"
	@[ -f $(VBENCH) ] && echo "  vbench: built (release)" || echo "  vbench: not built"
	@echo ""

# Show test summary
summary: build
	$(call PROGRESS,Generating test summary...)
	@$(VTEST) summary --config $(CONFIG_FILE)

# =============================================================================
# End of Makefile
# =============================================================================
