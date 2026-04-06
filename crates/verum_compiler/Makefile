.PHONY: help build test run check clean examples

# Default target
help:
	@echo "Verum Compiler Build Targets"
	@echo "============================"
	@echo ""
	@echo "build          - Build the compiler in release mode"
	@echo "build-debug    - Build the compiler in debug mode"
	@echo "test           - Run all tests"
	@echo "test-verbose   - Run tests with output"
	@echo "run-examples   - Run all example programs"
	@echo "check-syntax   - Check all examples for syntax errors"
	@echo "clean          - Clean build artifacts"
	@echo "help           - Show this help message"

build:
	@echo "Building verum compiler (release)..."
	cargo build --package verum_compiler --release
	@echo "✓ Build complete. Binary: target/release/verum"

build-debug:
	@echo "Building verum compiler (debug)..."
	cargo build --package verum_compiler
	@echo "✓ Build complete. Binary: target/debug/verum"

test:
	@echo "Running tests..."
	cargo test --package verum_compiler

test-verbose:
	@echo "Running tests (verbose)..."
	cargo test --package verum_compiler -- --nocapture

run-examples:
	@echo "Running example programs..."
	@for example in examples/*.ver; do \
		echo ""; \
		echo "=== Running $$example ==="; \
		./target/release/verum run "$$example" || echo "✗ Failed: $$example"; \
	done

check-syntax:
	@echo "Checking syntax of all examples..."
	@for example in examples/*.ver; do \
		echo "Checking $$example..."; \
		./target/release/verum check "$$example" || echo "✗ Failed: $$example"; \
	done

clean:
	@echo "Cleaning build artifacts..."
	cargo clean

.DEFAULT_GOAL := help
