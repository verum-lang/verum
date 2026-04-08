# VCS Baselines Makefile
# Builds and runs baseline comparison benchmarks for C, Rust, and Go

.PHONY: all clean build run fib sort

# Compiler flags
CC = gcc
CFLAGS = -O3 -march=native -Wall -Wextra

RUSTFLAGS = --release

# Directories
FIB_DIR = fib_comparison
SORT_DIR = sort_comparison

# Executables
FIB_C = $(FIB_DIR)/fibonacci_c
FIB_RS = $(FIB_DIR)/target/release/fibonacci
SORT_C = $(SORT_DIR)/sorting_c
SORT_RS = $(SORT_DIR)/target/release/sorting

all: build

build: build-fib build-sort

# Fibonacci benchmarks
build-fib: $(FIB_C) $(FIB_RS)

$(FIB_C): $(FIB_DIR)/fibonacci.c
	$(CC) $(CFLAGS) -o $@ $<

$(FIB_RS): $(FIB_DIR)/fibonacci.rs
	@if [ -f $(FIB_DIR)/Cargo.toml ]; then \
		cd $(FIB_DIR) && cargo build $(RUSTFLAGS); \
	else \
		rustc $(RUSTFLAGS) -o $@ $<; \
	fi

# Sorting benchmarks
build-sort: $(SORT_C) $(SORT_RS)

$(SORT_C): $(SORT_DIR)/sorting.c
	$(CC) $(CFLAGS) -o $@ $<

$(SORT_RS): $(SORT_DIR)/sorting.rs
	@if [ -f $(SORT_DIR)/Cargo.toml ]; then \
		cd $(SORT_DIR) && cargo build $(RUSTFLAGS); \
	else \
		rustc $(RUSTFLAGS) -o $@ $<; \
	fi

# Run all benchmarks
run: run-fib run-sort

run-fib: build-fib
	@echo "=== Fibonacci Benchmarks ==="
	@echo ""
	@echo "--- C ---"
	@$(FIB_C)
	@echo ""
	@echo "--- Rust ---"
	@$(FIB_RS)
	@echo ""
	@echo "--- Go ---"
	@cd $(FIB_DIR) && go run fibonacci.go

run-sort: build-sort
	@echo "=== Sorting Benchmarks ==="
	@echo ""
	@echo "--- C ---"
	@$(SORT_C)
	@echo ""
	@echo "--- Rust ---"
	@$(SORT_RS)
	@echo ""
	@echo "--- Go ---"
	@cd $(SORT_DIR) && go run sorting.go

# Individual runs
fib: build-fib
	@echo "Running Fibonacci benchmarks..."
	@$(FIB_C)
	@$(FIB_RS)
	@cd $(FIB_DIR) && go run fibonacci.go

sort: build-sort
	@echo "Running Sorting benchmarks..."
	@$(SORT_C)
	@$(SORT_RS)
	@cd $(SORT_DIR) && go run sorting.go

clean:
	rm -f $(FIB_C) $(SORT_C)
	rm -rf $(FIB_DIR)/target $(SORT_DIR)/target
	rm -f *.o

# Generate comparison report
report: run
	@echo ""
	@echo "=== Comparison Report ==="
	@echo "To generate full comparison with Verum, run:"
	@echo "  vbench baseline --language c"
	@echo "  vbench baseline --language rust"
	@echo "  vbench baseline --language go"

help:
	@echo "VCS Baselines - Language Comparison Benchmarks"
	@echo ""
	@echo "Usage:"
	@echo "  make build      Build all baseline executables"
	@echo "  make run        Run all baseline benchmarks"
	@echo "  make fib        Run Fibonacci benchmarks only"
	@echo "  make sort       Run Sorting benchmarks only"
	@echo "  make clean      Remove built executables"
	@echo "  make report     Generate comparison report"
	@echo "  make help       Show this help"
