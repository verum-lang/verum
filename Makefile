# Verum top-level Makefile.
#
# Convenience shortcuts that mirror what CI runs. Run these
# before pushing — they catch stale-match build breaks across
# the dependency graph without waiting for the CI run.

.PHONY: gates-source check-arch-attestation check-type-name-collisions check-barename-collisions check-barename-census check-rings check-rings-census check check-workspace check-tests check-strict test build help check-vr-syntax check-markers check-internal-refs check-op-bytes check-inventory check-inventory-live check-name-census check-panic-surface check-dup-emitters

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

check-markers: ## Gate: landed-markers fence (docs/architecture/landed-markers.txt)
	python3 vcs/scripts/check_landed_markers.py

check-vr-syntax: ## Gate: no Rust-style `::` in .vr sources (grammar/verum.ebnf uses `.`)
	python3 vcs/scripts/check_no_double_colon.py --check

check-str-alias: ## Gate (T0663): no Rust `&str` in core/ .vr code — Verum has only `Text`
	python3 vcs/scripts/check_no_str_alias.py --gate

check-op-bytes: ## Gate (T0198): .vr op-byte doc comments match the instruction.rs enums
	python3 vcs/scripts/check_op_byte_docs.py --check

check-stdlib-proofs: ## Gate (T0230): stdlib theorem-proof ratchet — clean files stay clean, proved counts never fall
	python3 scripts/ci/proof_gate.py

check-bake-diagnostics: ## Gate (T0723): FIELD-GUESS + panic-stub counts in a bake log
	@test -n "$(BAKE_LOG)" || { echo "usage: make check-bake-diagnostics BAKE_LOG=<path>"; exit 2; }
	python3 scripts/ci/check_bake_diagnostics.py "$(BAKE_LOG)" --check

check-archive-size: ## Gate (T0737): embedded stdlib archive size — a per-module duplication shows up as a MULTIPLE
	python3 scripts/ci/check_archive_size.py "$(or $(ARCHIVE),target/precompiled-stdlib/runtime.vbca)" --check

check-barename-collisions: ## Gate (T0538): free-fn (name,arity) collisions across core/ — ratchet
	python3 scripts/ci/check_barename_collisions.py --self-test
	python3 scripts/ci/check_barename_collisions.py --check
	python3 scripts/ci/check_barename_collisions.py --check --scope sqlite
	python3 scripts/ci/check_barename_collisions.py --check --scope prelude
	python3 scripts/ci/check_barename_collisions.py --check --kind types

check-arch-attestation: ## Gate (T0712): every core/ module declares @arch_module — list ratchet
	python3 scripts/ci/check_arch_attestation.py

check-type-name-collisions: ## Gate (T0458): simple-type-name collisions in core/ — pair-list ratchet
	python3 scripts/ci/check_type_name_collisions.py --self-test
	python3 scripts/ci/check_type_name_collisions.py

check-barename-census: ## Report every colliding (name,arity) pair with its modules (never fails)
	python3 scripts/ci/check_barename_collisions.py

gates-source: check-markers check-vr-syntax check-str-alias check-op-bytes check-internal-refs check-rings check-arch-attestation check-type-name-collisions check-barename-collisions check-panic-surface check-dup-emitters check-bake-prepass-parity check-protocol-form ## Every gate that needs only the SOURCE TREE — no build, no artefacts
	@echo "gates-source: all source-only gates green"

check-phantom-mounts: ## Gate (T0780): mounts naming a symbol the module does not export. NEEDS a built verum; ~15 min, NOT in gates-source.
	@test -n "$(VERUM)" || (echo "usage: make check-phantom-mounts VERUM=/path/to/verum" && false)
	python3 scripts/ci/check_phantom_mounts.py $(VERUM)

check-protocol-form: ## Gate (T0794): protocols in core/ use the grammatical `type X is protocol` form
	python3 scripts/ci/check_protocol_form.py

check-rings: ## Gate: core/ ring law — no upward edges, no cycles (core/rings.toml declares the rings)
	python3 scripts/ci/check_core_rings.py

check-rings-census: ## Report the core/ inter-module dependency graph (never fails)
	python3 scripts/ci/check_core_rings.py --census

check-internal-refs: ## Gate: no references to the internal/ directory in tracked files
	bash scripts/ci/check_no_internal_refs.sh

check-panic-surface: ## Gate (T0424): no net increase of unwrap/expect in verum_codegen/src/llvm production code
	python3 scripts/ci/check_panic_surface_ratchet.py

check-dup-emitters: ## Gate (T0438): one definer per verum_* symbol + no libc-referencing emitter bodies without a syscall path
	python3 scripts/ci/check_dup_emitters.py

check-bake-prepass-parity: ## Gate (T0640): every collect_all_declarations pre-pass is classified for the stdlib bake
	python3 scripts/ci/check_bake_prepass_parity.py

check-inventory: ## Gate (T0220): core-tests/INVENTORY.md structural integrity (rows unique, row<->dir bijection, status tokens)
	python3 scripts/ci/check_inventory.py --structural-only

check-name-census: ## Ratchet (T0690): name-keyed identity surfaces may only shrink (DefId migration)
	python3 scripts/ci/census_name_keyed_surfaces.py --check

check-inventory-live: ## Gate (T0220): INVENTORY liveness — green claims re-verified against a real interp run (INVENTORY_RESULTS=results.json, or it runs the suite)
	@if [ -n "$(INVENTORY_RESULTS)" ]; then \
		python3 scripts/ci/check_inventory.py --results "$(INVENTORY_RESULTS)"; \
	else \
		tmp=$$(mktemp -t inventory_results.XXXX.json); \
		echo "check-inventory-live: running verum test --interp --format json (reuse a run with INVENTORY_RESULTS=file)"; \
		cargo run --release -p verum_cli -- test --interp --format json > $$tmp 2>/dev/null; \
		python3 scripts/ci/check_inventory.py --results $$tmp; \
	fi

