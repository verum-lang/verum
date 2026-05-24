# `architecture/phase` audit

Module: `core/architecture/phase.vr` (~184 LOC) — ATS-V phase 6.5
Verum-native mirror of `verum_kernel::arch_phase`.

Tests: 11 unit tests covering ArchPhaseReport (empty +
load-bearing + 3 aggregator helpers) + ModuleArchResult record
+ CompositionStep / CompositionVerificationReport with two-step
all-composed vs one-failure paths.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_kernel::arch_phase` | Rust-side authoritative impl. |
| `crates/verum_compiler/src/pipeline/ats_v_phase.rs` | drives the orchestrator from the compiler. |
| `core.architecture.anti_patterns` | violations encode AntiPatternViolation. |
| `core.architecture.parse` | parse_errors carry ArchParseError. |

## 2. Crate-side hardcodes

* ArchPhaseReport field ordering MUST agree with
  `verum_kernel::arch_phase::ArchPhaseReport`. Drift here
  breaks cross-boundary marshalling.
* `module_arch_result_is_load_bearing` semantics (no parse
  errors AND no violations) are pinned by spec §3 + §17.5.

## 3. Language-implementation gaps

### §3.1 Test against real module compilation

Empty-report tests are trivial; the real value is verifying
`run_arch_phase` discharges every annotated module across the
stdlib without spurious violations. Requires the kernel
discharge to be callable from --interp tests; currently routes
through `kernel_arch_phase_orchestrator()` axiom.

**Effort:** moderate (~2h, requires axiom evaluation surface).

### §3.2 Property tests on aggregator monotonicity

∀r: ArchPhaseReport. arch_phase_report_total_violations(r) >= 0
+ adding any ModuleArchResult monotonically increases each
aggregator's output.

**Effort:** ~30 min.

### §3.3 Sister tests for the 12 architecture/* modules

phase is the orchestrator; the surface area is in adjunction,
anti_patterns, capability_ontology, composition, corpus,
counterfactual, mtac, parse, types, yoneda. Each needs an
audit + test sweep. Multi-week.

## Action items landed in this branch

* `core-tests/architecture/phase/unit_test.vr` — 11 unit tests
  over ArchPhaseReport + ModuleArchResult +
  CompositionStep/Report.
* `core-tests/architecture/phase/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Real-module compilation test via run_arch_phase | this folder | 2h |
| Aggregator monotonicity property test | this folder | 30 min |
| Sister tests for 10 architecture/* modules | sister folders | 2 weeks total |
