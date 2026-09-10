# Verum top-level Makefile.
#
# Convenience shortcuts that mirror what CI runs. Run these
# before pushing — they catch stale-match build breaks across
# the dependency graph without waiting for the CI run.

.PHONY: check-newtype-transparency
.PHONY: check-shipped-path-parity
.PHONY: gates-source check-private-types-off-public-surface check-grammar-covers-keywords check-grammar-docs-match check-doc-anchors check-doc-error-codes check-known-tables check-parser-attrs check-gate-tables check-dead-module-path-calls check-platform-call-parity check-protocol-conformance check-cfg-block-tail check-constant-time-duplication check-arch-attestation check-type-name-collisions check-barename-collisions check-barename-census check-rings check-rings-census check check-workspace check-tests check-strict test build help check-vr-syntax check-markers check-internal-refs check-op-bytes check-inventory check-inventory-live check-silent-acceptance check-name-census check-panic-surface check-per-register-privacy check-early-return-tenants check-dup-emitters check-homepage-examples check-doc-indented-blocks check-examples-run check-doc-status-badge check-doc-reachable check-doc-type-shapes

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

check-test-mounts: ## Gate: core-tests mounts name something core/ declares
	python3 scripts/ci/check_test_mounts_resolve.py

check-str-alias: ## Gate (T0663): no Rust `&str` in core/ .vr code — Verum has only `Text`
	python3 vcs/scripts/check_no_str_alias.py --gate

check-op-bytes: ## Gate (T0198): .vr op-byte doc comments match the instruction.rs enums
	python3 vcs/scripts/check_op_byte_docs.py --check

check-stdlib-proofs: ## Gate (T0230): stdlib theorem-proof ratchet — clean files stay clean, proved counts never fall
	python3 scripts/ci/proof_gate.py

check-bake-diagnostics: ## Gate (T0723): FIELD-GUESS + panic-stub counts in a bake log
	@test -n "$(BAKE_LOG)" || { echo "usage: make check-bake-diagnostics BAKE_LOG=<path>"; exit 2; }
	python3 scripts/ci/check_bake_diagnostics.py --self-test
	python3 scripts/ci/check_bake_diagnostics.py "$(BAKE_LOG)" --check

.PHONY: check-stage5-stub-sharing
check-stage5-stub-sharing: ## Gate (T1172): stage-5 stub ids standing for several names — needs VERUM_TRACE_QCALL=1 in the bake
	@test -n "$(BAKE_LOG)" || { echo "usage: make check-stage5-stub-sharing BAKE_LOG=<path>"; exit 2; }
	python3 scripts/ci/check_stage5_stub_sharing.py --self-test
	python3 scripts/ci/check_stage5_stub_sharing.py "$(BAKE_LOG)" --check

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

check-private-types-off-public-surface: ## Gate (T1195): a private core/ type must not be named by a public signature — ratchet at 0
	python3 scripts/ci/check_private_types_off_public_surface.py --self-test
	python3 scripts/ci/check_private_types_off_public_surface.py --check

check-doc-examples: ## Gate: the Verum examples in the docs must compile — needs a built binary (VERUM_BIN=... to point at one)
	python3 scripts/ci/check_doc_examples.py --self-test
	python3 scripts/ci/check_doc_examples.py --ratchet

check-homepage-examples: ## Gate: the Verum samples on the marketing homepage must be real Verum — needs a built binary
	python3 scripts/ci/check_homepage_examples.py --self-test
	python3 scripts/ci/check_homepage_examples.py --check

check-register-shas: ## Gate: a commit the debt register cites must be reachable from main
	python3 scripts/ci/check_register_shas_are_reachable.py --self-test
	python3 scripts/ci/check_register_shas_are_reachable.py --check

check-doc-blocks-parse: ## Gate: every ```verum block must parse, per tree — needs a build (slow)
	python3 scripts/ci/check_doc_blocks_parse.py --self-test
	VERUM_DOCS_DIR=vcs        python3 scripts/ci/check_doc_blocks_parse.py --check
	VERUM_DOCS_DIR=core-tests python3 scripts/ci/check_doc_blocks_parse.py --check
	VERUM_DOCS_DIR=docs       python3 scripts/ci/check_doc_blocks_parse.py --check

check-doc-names-exist: ## Gate: a name a doc example uses must exist in core/ — no build needed
	python3 scripts/ci/check_doc_names_exist.py --self-test
	python3 scripts/ci/check_doc_names_exist.py --check

check-doc-methods-exercised: ## Gate: a method the stdlib reference documents should be EXECUTED somewhere — no build needed
	python3 scripts/ci/check_doc_methods_exercised.py --self-test
	python3 scripts/ci/check_doc_methods_exercised.py --min-pages 30

.PHONY: check-doc-meta-functions
check-doc-meta-functions: ## Gate: a doc example must not teach a meta-function the compiler does not accept — no build needed
	python3 scripts/ci/check_doc_meta_functions.py --self-test
	python3 scripts/ci/check_doc_meta_functions.py --min-blocks 2000

check-doc-type-shapes: ## Gate: a `type X is …` in the docs must match core/ — no build needed
	python3 scripts/ci/check_doc_type_shapes.py --self-test
	python3 scripts/ci/check_doc_type_shapes.py --min-types 300

check-doc-receiver-methods: ## Gate: a method called on a receiver whose TYPE the block proves must exist on that type — no build needed
	python3 scripts/ci/check_doc_receiver_methods.py --self-test
	python3 scripts/ci/check_doc_receiver_methods.py --min-proven 150

check-doc-call-arity: ## Gate: a documented `Type.method(...)` call must pass an argument count the declaration accepts — no build needed
	python3 scripts/ci/check_doc_call_arity.py --self-test
	python3 scripts/ci/check_doc_call_arity.py --min-calls 600

check-doc-reachable: ## Gate: every doc page must be reachable from the sidebar or another page — no build needed
	python3 scripts/ci/check_doc_reachable.py --self-test
	python3 scripts/ci/check_doc_reachable.py --min-pages 300

check-doc-status-badge: ## Gate: a page that declares a status must RENDER the same one — no build needed
	python3 scripts/ci/check_doc_status_badge.py --self-test
	python3 scripts/ci/check_doc_status_badge.py --min-pages 40

check-doc-methods-declared: ## Gate: a method a doc example CALLS must be declared in core/ — no build needed
	python3 scripts/ci/check_doc_methods_declared.py --self-test
	python3 scripts/ci/check_doc_methods_declared.py

check-newtype-transparency: ## Gate (T1192): a single-field newtype must stay free across a module boundary — needs a built binary (VERUM_BIN=...)
	python3 scripts/ci/check_newtype_transparency.py --self-test
	@test -n "$(VERUM_BIN)" || { echo "usage: make check-newtype-transparency VERUM_BIN=<path to verum>"; exit 2; }
	python3 scripts/ci/check_newtype_transparency.py "$(VERUM_BIN)"

check-shipped-path-parity: ## Gate (T0816): a spec the conformance runner accepts must also run under `verum run` — needs a built binary (VERUM_BIN=... to point at one)
	python3 scripts/ci/check_shipped_path_parity.py --self-test
	@test -n "$(VERUM_BIN)" || { echo "usage: make check-shipped-path-parity VERUM_BIN=<path to verum>"; exit 2; }
	python3 scripts/ci/check_shipped_path_parity.py "$(VERUM_BIN)"

check-by-example: ## Gate: the 22 docs/by-example programs must compile — needs a build
	python3 scripts/ci/check_by_example_compiles.py --self-test
	python3 scripts/ci/check_by_example_compiles.py

check-examples-run: ## Gate: a shipped example that COMPILES must also RUN — needs a build
	python3 scripts/ci/check_doc_examples_run.py --self-test
	python3 scripts/ci/check_doc_examples_run.py

gates-docs: check-doc-examples check-examples-run check-homepage-examples check-doc-indented-blocks \
            check-doc-cli-flags check-doc-anchors check-doc-error-codes \
            check-grammar-docs-match check-by-example check-doc-blocks-parse \
            check-doc-names-exist check-doc-method-names \
            check-doc-module-paths check-doc-config-structs \
            check-doc-methods-declared check-doc-methods-exercised \
            check-doc-meta-functions \
            check-doc-status-badge check-doc-reachable \
            check-doc-type-shapes check-doc-receiver-methods \
            check-doc-call-arity \
            check-doc-iterator-items ## Every documentation gate CI runs — needs a build
	@echo "gates-docs: all documentation gates green"

check-doc-method-names: ## Gate: a method a doc example calls on a `core/` type must exist (the receiver gate checks only the RECEIVER)
	python3 scripts/ci/check_doc_method_names.py --self-test
	python3 scripts/ci/check_doc_method_names.py

list-tests-muted-on-closed-tasks: ## LIST (local only — the task pool is gitignored): specs @skip'd and core-tests @ignore'd on a task that has closed
	python3 scripts/ci/list_tests_muted_on_closed_tasks.py

list-doc-absent-methods: ## LIST (never a gate): doc methods called on a VARIABLE whose name is nowhere in core/
	python3 scripts/ci/list_doc_absent_methods.py

check-doc-iterator-items: ## Gate: a doc line naming a core iterator type AND its item must agree with what `next` yields
	python3 scripts/ci/check_doc_iterator_items.py --self-test
	python3 scripts/ci/check_doc_iterator_items.py

check-doc-cli-flags: ## Gate: every CLI flag the docs show must exist in the binary — needs a build
	python3 scripts/ci/check_doc_cli_flags.py --self-test
	python3 scripts/ci/check_doc_cli_flags.py
#	 The repo's own docs/ were never read by this gate — measured
#	 2026-09-07: 55 invocations, 46 carrying flags, across 5 files, and
#	 five defects among them (`verum count`, `audit --by-theorem`,
#	 `repl --proof`, `export --all-formats`, `export --format`).
	VERUM_DOCS_DIR=docs python3 scripts/ci/check_doc_cli_flags.py

check-doc-module-paths: ## Gate: a `verum_x::a::b` citation in the docs must name a module that exists
	python3 scripts/ci/check_doc_module_paths.py --self-test
#	 Floor of 200: the site carries 212 such citations today. A floor
#	 rather than a zero-check — a pattern that stops matching reports
#	 "0 citations, 0 unresolved" and looks green. The first version of
#	 this gate anchored on the closing backtick and could not see a
#	 path followed by a call signature; two defects sat in that gap.
	VERUM_DOCS_DIR="$(WEBSITE_DOCS)" python3 scripts/ci/check_doc_module_paths.py --min-citations 200
	VERUM_DOCS_DIR=docs python3 scripts/ci/check_doc_module_paths.py --min-citations 40

check-doc-config-structs: ## Gate: a doc page describing a config struct must name one that exists, with its real defaults and enum values
	python3 scripts/ci/check_doc_config_structs.py --self-test
#	 Floor of 9: the pages carrying either a `## <Name>Config` heading
#	 or a `| Field | Default |` table today. A floor rather than a
#	 zero-check, because zero defects and zero pages print the same
#	 way — the first version reported "0 pages, 0 defects" for a corpus
#	 with thirteen config sections, and the second still missed the
#	 four pages that name their struct in prose above the table.
	VERUM_DOCS_DIR="$(WEBSITE_DOCS)" python3 scripts/ci/check_doc_config_structs.py --min-pages 9

check-doc-indented-blocks: ## Gate: an indented block in a `///` comment is a Rust doctest — fence it
	python3 scripts/ci/check_doc_indented_blocks.py --selftest
	python3 scripts/ci/check_doc_indented_blocks.py

check-determinism: ## Gate (T0927): the compiler must give the same answer twice — needs a built binary (VERUM_BIN=... to point at one)
	python3 scripts/ci/check_determinism.py --self-test
	python3 scripts/ci/check_determinism.py --sample 40 --check

# Built from parts so the literal path does not appear in a tracked
# file (`make check-internal-refs`), the same way the doc gates do it.
WEBSITE_DOCS ?= $(strip internal)/website/docs
KEYWORDS_DOC ?= $(WEBSITE_DOCS)/reference/keywords.md

check-grammar-covers-keywords: ## Gate: every keyword the lexer accepts must appear in grammar/verum.ebnf AND on the keyword reference
	python3 scripts/ci/check_grammar_covers_keywords.py --self-test
	python3 scripts/ci/check_grammar_covers_keywords.py --check --docs "$(KEYWORDS_DOC)"

check-doc-anchors: ## Gate: a documentation link must point at a heading that exists (a broken anchor FAILS the site build)
	python3 scripts/ci/check_doc_anchors.py --self-test
	python3 scripts/ci/check_doc_anchors.py --check

check-doc-error-codes: ## Gate: a cited error code must exist in the registry AND be emitted by something
	python3 scripts/ci/check_doc_error_codes_exist.py --self-test
	python3 scripts/ci/check_doc_error_codes_exist.py --check

check-verdict-phases: ## Gate: a verdict phase must be reachable from every entry point (RED while T1101 is open — NOT in gates-source)
	python3 scripts/ci/check_verdict_phases_reach_every_entry_point.py --self-test
	python3 scripts/ci/check_verdict_phases_reach_every_entry_point.py --check

check-register-rows: ## Gate: every tech-debt register row is one markdown line
	python3 scripts/ci/check_register_rows_are_rows.py --self-test
	python3 scripts/ci/check_register_rows_are_rows.py

check-gate-tables: ## Gate: a table inside a gate must not have the same key twice
	python3 scripts/ci/check_gate_tables_have_no_duplicate_keys.py --self-test
	python3 scripts/ci/check_gate_tables_have_no_duplicate_keys.py --check

check-parser-attrs: ## Gate: the parser must not call a registered attribute unknown
	python3 scripts/ci/check_parser_attrs_cover_the_registry.py --self-test
	python3 scripts/ci/check_parser_attrs_cover_the_registry.py --check

check-known-tables: ## Gate: KNOWN_TABLES must cover every field Manifest declares
	python3 scripts/ci/check_known_tables_covers_manifest.py --self-test
	python3 scripts/ci/check_known_tables_covers_manifest.py --check

check-grammar-docs-match: ## Gate: EBNF shown in the documentation must match grammar/verum.ebnf
	python3 scripts/ci/check_grammar_docs_match.py --self-test
	python3 scripts/ci/check_grammar_docs_match.py --check

check-barename-census: ## Report every colliding (name,arity) pair with its modules (never fails)
	python3 scripts/ci/check_barename_collisions.py

gates-source: check-private-types-off-public-surface check-error-code-namespaces check-guard-in-argument-position check-grammar-covers-keywords check-grammar-docs-match check-doc-anchors check-doc-error-codes check-known-tables check-parser-attrs check-gate-tables check-markers check-vr-syntax check-str-alias check-op-bytes check-internal-refs check-rings check-arch-attestation check-type-name-collisions check-barename-collisions check-panic-surface check-per-register-privacy check-early-return-tenants check-dup-emitters check-bake-prepass-parity check-protocol-form check-dead-module-path-calls check-platform-call-parity check-protocol-conformance check-cfg-block-tail check-meta-function-names check-ffi-reference-tiers check-intrinsic-keys-implemented check-doc-calls-that-trap check-doc-status-matches-inventory check-doc-mounts-resolve check-constant-time-duplication check-type-param-name-rule ## Every gate that needs only the SOURCE TREE — no build, no artefacts
# `check-register-shas` is NOT in that list, and its name used to sit
# after the `##` above, where make read it as help text — the target
# existed, the gate worked, and the aggregate never called it
# (`make -n gates-source | grep -c register_shas` gave 0).  It is out
# deliberately now: the gate asks whether a cited commit is reachable,
# and a dangling commit is not fetched by `git clone`, so in CI's fresh
# checkout the objects are absent rather than unreachable.  It refuses
# there (rc=2) rather than reporting a clean sheet.  Run it in a
# long-lived working copy: `make check-register-shas`.  See T1333.

	@echo "gates-source: all source-only gates green"

gates-source-report: ## Run EVERY source-only gate past the first failure and summarise (make stops at the first; this does not)
	bash scripts/ci/run_source_gates.sh

check-phantom-mounts: ## Gate (T0780): mounts naming a symbol the module does not export. NEEDS a built verum; ~15 min, NOT in gates-source.
	@test -n "$(VERUM)" || (echo "usage: make check-phantom-mounts VERUM=/path/to/verum" && false)
	python3 scripts/ci/check_phantom_mounts.py $(VERUM)

check-type-param-name-rule: ## Ratchet: places deciding "is this a type parameter" from a name's SPELLING (11; two thresholds that disagree — T1206)
	python3 scripts/ci/check_type_param_name_rule_ratchet.py --self-test
	python3 scripts/ci/check_type_param_name_rule_ratchet.py

check-error-code-namespaces: ## Gate: one namespace for error codes — no code means two things.
	python3 scripts/ci/check_error_code_namespaces.py --self-test
	python3 scripts/ci/check_error_code_namespaces.py --check

check-guard-in-argument-position: ## Gate (T0981): a lock guard in argument position self-deadlocks.
	python3 scripts/ci/check_guard_in_argument_position.py --check

check-protocol-form: ## Gate (T0794): protocols in core/ use the grammatical `type X is protocol` form
	python3 scripts/ci/check_protocol_form.py

check-constant-time-duplication: ## Gate (T0817): a constant-time comparator hand-rolled outside core/subtle/ — one implementation should carry that promise
	python3 scripts/ci/check_constant_time_duplication.py --self-test
	python3 scripts/ci/check_constant_time_duplication.py

check-doc-calls-that-trap: ## A site page must not teach a call that panics on an unimplemented intrinsic without saying so (T1372)
	python3 scripts/ci/check_doc_calls_that_trap.py --self-test
	python3 scripts/ci/check_doc_calls_that_trap.py --check

check-doc-status-matches-inventory: ## Gate (T1379): a stdlib page may not claim a conformance status GREENER than core-tests/INVENTORY.md
	python3 scripts/ci/check_doc_status_matches_inventory.py --self-test
	python3 scripts/ci/check_doc_status_matches_inventory.py

check-doc-mounts-resolve: ## Gate (T1381): a `mount core.a.b.{Name}` in the docs must name something that module has
	python3 scripts/ci/check_doc_mounts_resolve.py --self-test
	python3 scripts/ci/check_doc_mounts_resolve.py

check-intrinsic-keys-implemented: ## Freeze the SET of declared-but-unimplemented `verum.*` intrinsic keys (T1368)
	python3 scripts/ci/check_intrinsic_keys_implemented.py --self-test
	python3 scripts/ci/check_intrinsic_keys_implemented.py --check

check-ffi-reference-tiers: ## Gate (T1192/T1358): the three FFI sites that consult repr_c_types through a reference must match ALL THREE CBGR tiers
	python3 scripts/ci/check_ffi_reference_tiers.py --self-test
	python3 scripts/ci/check_ffi_reference_tiers.py

check-meta-function-names: ## Gate (T1352): an `@name(...)` the compiler does not know types as Unit — a zero-byte object, not a lint
	python3 scripts/ci/check_meta_function_names.py --self-test
	python3 scripts/ci/check_meta_function_names.py

check-cfg-block-tail: ## Gate (T0805): a function whose value is meant to come from an @cfg block — a gated block is a statement, so the function yields Unit
	python3 scripts/ci/check_cfg_block_tail.py

check-protocol-conformance: ## Gate (T0812): an implement block missing a method its protocol requires — type-checks today and panics at the call
	python3 scripts/ci/check_protocol_conformance.py --self-test
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
	python3 scripts/ci/check_spec_citation_names.py --self-test
	python3 scripts/ci/check_spec_citation_names.py

check-per-register-privacy: ## Gate: no per-register fact in FunctionContext may be `pub` — source only
	python3 scripts/ci/check_per_register_fields_private.py --self-test
	python3 scripts/ci/check_per_register_fields_private.py

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

