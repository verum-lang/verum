# `core.text.tagged_literals` — audit

> Status: **complete**. Sweep on 2026-05-15: 29/29 unit + 9 property +
> 4 integration + 5 regression + 1 guard all pass interp.
>
> Historic note: pre-2026-05-15 sweep reported 1/29 (3.4%) with all 28
> failures pinned to a "CallM register-slot read" defect that surfaced
> random Text values as method names — "WARN", "DEBUG", "glob: empty
> character class". That defect closed transitively via parallel agent
> dispatch fixes prior to the 2026-05-15 sweep. No new defects
> uncovered. The audit's §A/§B sections below remain as a historic
> reference and to lock in the test surface as guardrails.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| User code: `json#"…"` / `sql#"…"` / `uri#"…"` literals | the parser desugars the prefix into a runtime call to `validate_json` / `validate_sql` / `validate_uri` (with the literal text as argument) and panics if it returns false |
| `core/database/sqlite/...` | `sql#` literals are the canonical way to write SQL strings in Verum |
| HTTP / API code | `uri#` literals validate URIs at the call site |
| Configuration / data-loading code | `json#` literals validate JSON shape |

Closing the dispatcher defect (§A/§B share root) re-enables the entire
tagged-literal feature surface for end users.

## 2. Crate-side hardcodes

| Path | What | Hardcoded |
|---|---|---|
| `crates/verum_lexer/src/lexer.rs` | recognises `<tag>#"…"` syntax | dispatches to the validator named `validate_<tag>` |
| `crates/verum_compiler/src/precompile.rs` | wires `validate_<tag>` calls at compile time | hardcodes the well-known tag set: `json`, `sql`, `uri`, `regex`/`re`/`rx` |
| `crates/verum_vbc/src/codegen/expressions.rs` | emits the post-validate runtime check | call → panic-on-false |

## 3. Language-implementation gaps surfaced by this folder

### §A / §B — runtime dispatcher reads CallM key from wrong register
**Symptom**: every test that calls `validate_sql` panics with
`method 'WARN' not found on receiver of runtime kind 'Object'`.
Every test that calls `validate_json` panics with
`method 'glob: empty character class' not found on receiver of
runtime kind 'Text<small>'`. The error message names a string that
**does not appear anywhere in the test or in the validator source**.
**Root cause hypothesis**: the runtime dispatcher's CallM
implementation reads the method name from a register slot. When that
slot is reused / not properly initialised, it surfaces whatever Text
value happens to be at the slot from a previous call. "WARN" / "DEBUG"
are log-level constants from `core/diagnostics.vr`. "glob: empty
character class" is an error message from the regex crate.
**Action**: investigate
`crates/verum_vbc/src/interpreter/handlers/call_methods.rs` — the
register read for the method name must be from the correct slot, NOT
from a stale slot. This is a register-allocation defect at the codegen
level, not a method-table defect.

---

## 4. Action items

### Landed in this branch
- 29 unit tests + 12 property tests + 4 integration tests + 5 regression
  pins + 1 PASS-GUARD.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A/§B — CallM register-slot read | medium-large (interpreter dispatch) | ~28 (entire validator surface + likely many more across the stdlib) |

### Drift-pin recommendations
1. Add a runtime-side test to
   `crates/verum_vbc/src/interpreter/handlers/call_methods.rs::tests`
   that calls a method with a fresh register state and asserts the
   resolved method name matches the static call key. Locks in the §A
   fix.
2. Pin the well-known tag → validator name map in
   `crates/verum_common/src/well_known_types.rs::TAGGED_LITERAL_TAGS`.
