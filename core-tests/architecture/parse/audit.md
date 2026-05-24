# `architecture/parse` audit

Module: `core/architecture/parse.vr` (~117 LOC) — ATS-V
`@arch_module(...)` parser surface (error type + canonical
field roster mirror).

Tests: 19 unit tests covering ArchParseError 5-variant +
arch_parse_error_tag stable diagnostic tags + 13-field
canonical roster + arch_module_field_count_invariant pin.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_kernel::arch_parse` | authoritative parser; this module mirrors error type. |
| `crates/verum_compiler/src/pipeline/ats_v_phase.rs` | surfaces ArchParseError from kernel discharge. |
| `core.architecture.phase` | ModuleArchResult.parse_errors carries ArchParseError. |
| LSP/agent code | pattern-matches on diagnostic tags. |

## 2. Crate-side hardcodes

* 13-field canonical roster MUST agree with kernel parser at
  `arch_parse::parse_arch_module` — drift breaks @arch_module
  declarations from Verum source.
* `arch_module_field_count_invariant()` returns `true` iff
  roster size == 13 — pinned by
  `crates/verum_kernel/tests/k_arch_v_alignment.rs`.
* Stable tag strings ("unknown_field", "invalid_value", etc.)
  MUST agree with LSP/agent diagnostic codes.

## 3. Language-implementation gaps

### §3.1 Property test on roster invariant

Property: arch_module_field_count_invariant() must remain true
across all stdlib versions. Adding a 14th field requires
synchronised kernel + roster + spec update.

### §3.2 Diagnostic-tag string stability test

Across stdlib versions, the 5 diagnostic tags MUST remain
exactly: "unknown_field", "invalid_value", "missing_required",
"unknown_variant", "not_an_arch_module_attribute". Any drift
breaks LSP/agent consumers.

### §3.3 Sister-test for `core.architecture.{phase,types,
anti_patterns,corpus,composition}`

The parse module is the front door; the rest of architecture/
is the consumption surface. Each needs its own conformance
suite. Multi-week.

## Action items landed in this branch

* `core-tests/architecture/parse/unit_test.vr` — 19 unit tests
  over ArchParseError + arch_parse_error_tag + canonical
  field roster + invariant pin.
* `core-tests/architecture/parse/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test on roster invariant | this folder | 15 min |
| Diagnostic-tag stability cross-version test | this folder | 30 min |
| Sister tests for 8 architecture/* modules | sister folders | 2 weeks total |
