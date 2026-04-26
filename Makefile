# Verum top-level Makefile.
#
# Convenience shortcuts that mirror what CI runs. Run these
# before pushing — they catch stale-match build breaks across
# the dependency graph without waiting for the CI run.

.PHONY: check check-workspace check-tests check-strict test build help

help: ## Show available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  %-22s %s\n", $$1, $$2}'

check: check-workspace ## Alias for check-workspace

check-workspace: ## Workspace-wide check — every crate, default features
	cargo check --workspace --release

check-tests: ## Compile every test target — catches stale matches in tests too
	cargo test --no-run --workspace --release

check-strict: check-workspace check-tests ## Both checks — what CI's build gate runs
	@echo "✓ workspace + tests both compile"

test: ## Run every unit + integration test in release mode
	cargo test --workspace --release

build: ## Build every crate in release mode
	cargo build --workspace --release
