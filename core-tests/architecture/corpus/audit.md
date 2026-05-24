# `architecture/corpus` audit

Module: `core/architecture/corpus.vr` (~143 LOC) — ATS-V
cross-cog corpus invariants mirror.

Tests: 22 unit tests covering CorpusInvariant 4-variant + tag
+ name + full_list + roster_size_invariant pin + CorpusReport
record (empty + load_bearing) + CorpusViolation construction +
not-load-bearing with one violation.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_kernel::arch_corpus::verify_corpus` | authoritative engine. |
| ATS-V CLI (`verum arch verify`) | aggregates CorpusReport. |
| `core.architecture.phase` | per-module reports compose into corpus. |

## 2. Crate-side hardcodes

* CorpusInvariant 4-variant set MUST agree with kernel-side
  `verum_kernel::arch_corpus::CorpusInvariant`.
* `corpus_invariant_roster_size_invariant()` pin: exactly 4.
  Adding a 5th requires RFC ATS-V-008 + kernel enum bump.
* Stable tag strings MUST agree with LSP/audit JSON schema.
* CorpusReport.is_load_bearing semantics (zero violations)
  pinned by spec §11.3.

## 3. Language-implementation gaps

### §3.1 Real corpus verification test

Test that `kernel_arch_corpus_verify` axiom yields a
load-bearing report for the stdlib's own corpus. Requires
axiom evaluation surface and the stdlib's actual module graph
loaded. Multi-day.

### §3.2 Property test on roster size

Property: corpus_invariant_full_list().len() remains == 4
across all stdlib versions; adding a 5th invariant requires
synchronised RFC + kernel enum + roster + pin test bumps.

### §3.3 Diagnostic-tag string stability test

The 4 diagnostic tags MUST remain exactly:
"no_circular_dependencies", "foundation_consistency",
"no_l_abs_claim", "capability_closure". Any drift breaks
LSP/agent consumers.

## Action items landed in this branch

* `core-tests/architecture/corpus/unit_test.vr` — 22 unit
  tests over CorpusInvariant + tag + name + full_list +
  roster_size_invariant + CorpusReport + load_bearing.
* `core-tests/architecture/corpus/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Real corpus verification test via kernel discharge | this folder | 2 days |
| Property test on roster size pin | this folder | 15 min |
| Diagnostic-tag stability cross-version test | this folder | 30 min |
| Sister tests for 8 remaining architecture/* modules | sister folders | 2 weeks total |
