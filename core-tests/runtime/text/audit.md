# `runtime/text` audit

Module: `core/runtime/text.vr` (78 LOC) — runtime-layer text + char
intrinsic surface (16 forward-declared free fns).

Tests: 26 unit tests covering text_parse / int_to_text / char_is_* /
char_to_{upper,lower}case / char_general_category.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.text.char.Char.is_alphabetic` / `Char.is_numeric` / etc. | dispatch to `char_is_*` after the Char NaN-box decode. |
| `core.text.char.Char.to_uppercase` / `to_lowercase` | dispatch to `char_to_*`. |
| `core.text.text.Text.parse_int` / `Text.parse_float` | dispatch to `text_parse_*`. |
| `core.text.format.int_to_text` | dispatch to `int_to_text` (with the §H ref-self auto-deref workaround discipline). |
| `core.text.numeric.bigint` | NOT bound to these — uses its own multi-digit decimal pipeline. |

`grep -r "text_parse_int\|int_to_text\|char_is_\|char_to_" core/` returns
~50 sites across `core/text/*`, `core/eval/*`, `core/cog/*`,
`core/encoding/*`.  Any of these break if the runtime binding regresses.

## 2. Crate-side hardcodes

| Site | What it pins | Risk |
|---|---|---|
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/char_extended.rs` | `CharSubOpcode::EncodeUtf8` (closed 2026-05-14 per [[char_const_eval_2026-05-25]]) | EncodeUtf8 bound here; without the fix every `Char.encode_utf8(&mut buf)` silently dropped output. |
| `crates/verum_vbc/src/interpreter/dispatch_table/handlers/method_dispatch.rs` (CallM intercept for Char primitives) | `char_is_*` reach via NaN-box decode + dispatch | [[text/char §B]] residual: `eq_ignore_ascii_case` false-negative — primitives-NaN-boxed-to-foreign-kind MUST have CallM intercept. |
| `crates/verum_codegen/src/llvm/text.rs` | AOT lowering of the `text_*` family | Drift between interp/AOT here surfaces as cross-tier disagreement on numeric round-trip. |

## 3. Language-implementation gaps

### §A — char_general_category category-code mapping is implementation-defined

`char_general_category(c)` returns an Int, but the mapping is not
contractually pinned: VBC interp may use `{Lu=1, Ll=2, Nd=9, ...}` while
AOT may use `{Lu=0, Ll=1, ...}`.  Tests here pin self-consistency over
the ASCII range (Lu chars share a code, Ll chars share a code, Lu ≠ Ll)
NOT specific values.  Recommend: surface the canonical Unicode
General_Category codes (per [UAX #44 Table 12](https://www.unicode.org/reports/tr44/#General_Category_Values))
in stdlib docs.

### §B — text_parse_int doesn't pin overflow/underflow contract

`text_parse_int("99999999999999999999")` (20 nines) — behaviour
implementation-defined: returns `None`, returns `Some(Int.MAX)`, panics?
The free-fn declaration says `Maybe<Int>` so `None` is the principled
behaviour but the contract is not enforced.  Audit pin: write a
property test asserting either `None` OR `Some(N)` with `N` in the
representable range.

### §C — drift between user-side `Text.parse_int` and runtime `text_parse_int`

User-facing surface is `(&Text).parse_int() -> Maybe<Int>` (see
`core.text.text`).  The dispatch path for the user-side surface routes
through this runtime intrinsic, so any deviation in the two would
surface as a tiered-execution incident.  Drift-pinning unit at
`crates/verum_vbc/src/interpreter/dispatch_table/handlers/text_extended.rs`
to keep the user-side dispatch + runtime path aligned.

## Action items landed in this branch

* `core-tests/runtime/text/unit_test.vr` — 26 unit tests covering the
  text_parse / int_to_text / char_is_* / char_to_* surface.
* `core-tests/runtime/text/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| §A canonical Unicode General_Category codes | `core/runtime/text.vr` docstring + stdlib docs | 30 min |
| §B `text_parse_int` overflow contract property test | this folder | 30 min |
| §C drift-pinning unit between dispatch handlers | `crates/verum_vbc/tests/` | 1 h |
| char_encode_utf8 / char_escape_debug coverage | this folder | 1 h |
| text_from_static / utf8_decode_char_len surface | this folder | gated on a safe synthetic-pointer harness |
