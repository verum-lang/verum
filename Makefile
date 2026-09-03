# Verum top-level Makefile.
#
# Convenience shortcuts that mirror what CI runs. Run these
# before pushing — they catch stale-match build breaks across
# the dependency graph without waiting for the CI run.

.PHONY: gates-source check-grammar-covers-keywords check-grammar-docs-match check-doc-anchors check-doc-error-codes check-known-tables check-dead-module-path-calls check-platform-call-parity check-protocol-conformance check-cfg-block-tail check-constant-time-duplication check-arch-attestation check-type-name-collisions check-barename-collisions check-barename-census check-rings check-rings-census check check-workspace check-tests check-strict test build help check-vr-syntax check-markers check-internal-refs check-op-bytes check-inventory check-inventory-live check-silent-acceptance check-name-census check-panic-surface check-early-return-tenants check-dup-emitters

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

check-doc-examples: ## Gate: the Verum examples in the docs must compile — needs a built binary (VERUM_BIN=... to point at one)
	python3 scripts/ci/check_doc_examples.py --self-test
	python3 scripts/ci/check_doc_examples.py --check

check-determinism: ## Gate (T0927): the compiler must give the same answer twice — needs a built binary (VERUM_BIN=... to point at one)
	python3 scripts/ci/check_determinism.py --self-test
	python3 scripts/ci/check_determinism.py --sample 40 --check

check-grammar-covers-keywords: ## Gate: every keyword the lexer accepts must appear in grammar/verum.ebnf
	python3 scripts/ci/check_grammar_covers_keywords.py --self-test
	python3 scripts/ci/check_grammar_covers_keywords.py --check

check-doc-anchors: ## Gate: a documentation link must point at a heading that exists (a broken anchor FAILS the site build)
	python3 scripts/ci/check_doc_anchors.py --self-test
	python3 scripts/ci/check_doc_anchors.py --check

check-doc-error-codes: ## Gate: an error code cited in the documentation must exist in the registry
	python3 scripts/ci/check_doc_error_codes_exist.py --self-test
	python3 scripts/ci/check_doc_error_codes_exist.py --check

check-known-tables: ## Gate: KNOWN_TABLES must cover every field Manifest declares
	python3 scripts/ci/check_known_tables_covers_manifest.py --self-test
	python3 scripts/ci/check_known_tables_covers_manifest.py --check

check-grammar-docs-match: ## Gate: EBNF shown in the documentation must match grammar/verum.ebnf
	python3 scripts/ci/check_grammar_docs_match.py --self-test
	python3 scripts/ci/check_grammar_docs_match.py --check

check-barename-census: ## Report every colliding (name,arity) pair with its modules (never fails)
	python3 scripts/ci/check_barename_collisions.py

gates-source: check-error-code-namespaces check-guard-in-argument-position check-grammar-covers-keywords check-grammar-docs-match check-doc-anchors check-doc-error-codes check-known-tables check-markers check-vr-syntax check-str-alias check-op-bytes check-internal-refs check-rings check-arch-attestation check-type-name-collisions check-barename-collisions check-panic-surface check-early-return-tenants check-dup-emitters check-bake-prepass-parity check-protocol-form check-dead-module-path-calls check-platform-call-parity check-protocol-conformance check-cfg-block-tail check-constant-time-duplication ## Every gate that needs only the SOURCE TREE — no build, no artefacts

	@echo "gates-source: all source-only gates green"

gates-source-report: ## Run EVERY source-only gate past the first failure and summarise (make stops at the first; this does not)
	bash scripts/ci/run_source_gates.sh

check-phantom-mounts: ## Gate (T0780): mounts naming a symbol the module does not export. NEEDS a built verum; ~15 min, NOT in gates-source.
	@test -n "$(VERUM)" || (echo "usage: make check-phantom-mounts VERUM=/path/to/verum" && false)
	python3 scripts/ci/check_phantom_mounts.py $(VERUM)

check-error-code-namespaces: ## Gate: one namespace for error codes — no code means two things.
	python3 scripts/ci/check_error_code_namespaces.py

check-guard-in-argument-position: ## Gate (T0981): a lock guard in argument position self-deadlocks.
	python3 scripts/ci/check_guard_in_argument_position.py

check-protocol-form: ## Gate (T0794): protocols in core/ use the grammatical `type X is protocol` form
	python3 scripts/ci/check_protocol_form.py

check-constant-time-duplication: ## Gate (T0817): a constant-time comparator hand-rolled outside core/subtle/ — one implementation should carry that promise
	python3 scripts/ci/check_constant_time_duplication.py

check-cfg-block-tail: ## Gate (T0805): a function whose value is meant to come from an @cfg block — a gated block is a statement, so the function yields Unit
	python3 scripts/ci/check_cfg_block_tail.py

check-protocol-conformance: ## Gate (T0812): an implement block missing a method its protocol requires — type-checks today and panics at the call
	python3 scripts/ci/check_protocol_conformance.py

check-platform-call-parity: ## Gate (T0808): calls into core/sys/<platform>/ naming something that module does not provide — silent nil on the platform you are not testing on
	python3 scripts/ci/check_platform_call_parity.py --self-test
	python3 scripts/ci/check_platform_call_parity.py

check-dead-module-path-calls: ## Gate (T0806): module-path calls whose callee is declared nowhere — the compiler returns nil for these instead of diagnosing them
	python3 scripts/ci/check_dead_module_path_calls.py

check-rings: ## Gate: core/ ring law — no upward edges, no cycles (core/rings.toml declares the rings)
	python3 scripts/ci/check_core_rings.py

check-barename-method-census: ## Report free functions whose name is also a method (T0798; never fails)
	python3 scripts/ci/check_barename_collisions.py --kind methods

check-rings-census: ## Report the core/ inter-module dependency graph (never fails)
	python3 scripts/ci/check_core_rings.py --census

check-internal-refs: ## Gate: no references to the internal/ directory in tracked files
	bash scripts/ci/check_no_internal_refs.sh

check-panic-surface: ## Gate (T0424): no net increase of unwrap/expect in verum_codegen/src/llvm production code
	python3 scripts/ci/check_panic_surface_ratchet.py
	python3 scripts/ci/check_uncoded_diagnostic_ratchet.py
	python3 scripts/ci/check_mount_group_integrity.py
	bash scripts/ci/check_summary_line_is_one_spelling.sh
	python3 scripts/ci/check_gate_verdict_carries_a_quantity.py

check-early-return-tenants: ## Gate (T1078): the count of diagnostics behind the source-seen early return does not drift
	python3 scripts/ci/check_early_return_tenants.py

check-dup-emitters: ## Gate (T0438): one definer per verum_* symbol + no libc-referencing emitter bodies without a syscall path
	python3 scripts/ci/check_dup_emitters.py

check-bake-prepass-parity: ## Gate (T0640): every collect_all_declarations pre-pass is classified for the stdlib bake
	python3 scripts/ci/check_bake_prepass_parity.py

check-inventory: ## Gate (T0220): core-tests/INVENTORY.md structural integrity (rows unique, row<->dir bijection, status tokens)
	python3 scripts/ci/check_inventory.py --structural-only

check-name-census: ## Ratchet (T0690): name-keyed identity surfaces may only shrink (DefId migration)
	python3 scripts/ci/census_name_keyed_surfaces.py --check

check-silent-acceptance: ## Gate: an input the compiler cannot honour must not be counted in favour of the checked (T1025/T1026/T1027/T0989); needs a built binary — VERUM=path
	python3 scripts/ci/check_silent_acceptance.py $(or $(VERUM),target/release/verum)

check-inventory-live: ## Gate (T0220): INVENTORY liveness — green claims re-verified against a real interp run (INVENTORY_RESULTS=results.json, or it runs the suite)
	@if [ -n "$(INVENTORY_RESULTS)" ]; then \
		python3 scripts/ci/check_inventory.py --results "$(INVENTORY_RESULTS)"; \
	else \
		tmp=$$(mktemp -t inventory_results.XXXX.json); \
		echo "check-inventory-live: running verum test --interp --format json (reuse a run with INVENTORY_RESULTS=file)"; \
		cargo run --release -p verum_cli -- test --interp --format json > $$tmp 2>/dev/null; \
		python3 scripts/ci/check_inventory.py --results $$tmp; \
	fi

