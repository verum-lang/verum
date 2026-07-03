# `intrinsics/runtime/text` audit

Module: `core/intrinsics/runtime/text.vr` (~195 LOC) — allocation-backed
text/char intrinsics: Text byte length/view, int/float parse and render,
UTF-8 decode/encode, char classification and case conversion.

Tests: unit (27) + property (8) + integration (8) + regression (2).
ASCII-domain laws are exhaustive (0..128); numeric round-trips are
boundary-sampled (documented inline).

## 1. Findings (2026-07-03 first pass)

### RUNTIME-DUPLICATE-TREE-1 (task #15) — dead parallel declaration tree

`core/runtime/text.vr` (and 17 sibling files under `core/runtime/`)
redeclare this module's surface under `@intrinsic("verum.runtime.<name>")`
keys that appear NOWHERE in the registry — a dead layer violating the
"ALL `@intrinsic` declarations live under `core/intrinsics/`" invariant.
Live consumer: `core/database/sqlite/native/builtins/string_fns.vr:39`
mounts `core.runtime.text.{float_to_text}` — resolving to the unwired key.
`core/text/char.vr` and `core/text/text.vr` mount the CANONICAL module and
are unaffected.  The regression suite pins the canonical
`float_to_text`/`text_parse_float` round-trip as the cleanup's conformance
anchor.

### Signature honesty — `text_from_static(s: &'static str)`

The declaration uses `str`, a non-semantic type name absent from the
grammar's type table (Verum's semantic type is `Text`).  It is
compiler-internal (literal lowering) and not user-callable; audited, not
unit-tested.  Candidate cleanup: mark `@internal` or move it out of the
public surface.

### Initial suite run (pre-fix): 21/48

The first interpreter run failed 27/48 — triaged into the char-intrinsic
wiring class (results pending the current rebuild; failures will be pinned
in regression_test.vr with defect ids once classified against the fixed
binary).

## 2. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.text.char` | the char classification/conversion surface (canonical mounts). |
| `core.text.text` | byte length, parse, render. |
| f-string formatter | int_to_text / float_to_text agreement (pinned in integration). |
| `core.database.sqlite` | float_to_text via the DEAD tree (task #15). |

## 3. Crate-side hardcodes / drift surfaces

* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/` — text/char
  handler dispatch by intrinsic name.
* `TextSubOpcode::AsBytes` — the canonical three-representation Text byte
  view `text_as_bytes` routes through (small-string NaN-box, builder
  `{ptr, len, cap}`, heap `[hdr][len][bytes]`).
* Unicode tables for classification — interpreter (Rust `char` methods) vs
  AOT lowering must agree; the ASCII-exhaustive laws are the tripwire.

## 4. Action items

**Landed this branch**
* Full conformance suite; parse∘render identity law; ASCII-exhaustive
  classification partitions; case-conversion idempotence laws;
  UTF-8 encode/decode against live buffers; f-string agreement pins.
* RUNTIME-DUPLICATE-TREE-1 filed (task #15) with the sqlite consumer named.

**Deferred (tracked)**
* task #15 cleanup (re-export shims + sqlite mount repoint + repo guard).
* `text_from_static` signature honesty.
* Classify + pin the 27 first-run failures against the rebuilt binary.
