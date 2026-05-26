# `core/base/string_distance` — Audit

> Module: `core/base/string_distance.vr` — Levenshtein (full +
> bounded), Jaro, Jaro-Winkler, plus `closest_text` / `closest_n`
> "did you mean ..." autocorrect API.

## §1 — Public API surface

### 1.1 Byte-slice form

| Free fn | Signature | Notes |
|---|---|---|
| `levenshtein` | `(&[Byte], &[Byte]) -> Int` | Wagner-Fischer, rolling-window |
| `levenshtein_bounded` | `(&[Byte], &[Byte], Int) -> Maybe<Int>` | early-stop above threshold |
| `jaro` | `(&[Byte], &[Byte]) -> Float` | similarity in [0, 1] |
| `jaro_winkler` | `(&[Byte], &[Byte]) -> Float` | Jaro + common-prefix boost |

### 1.2 Text overloads

| Free fn | Signature |
|---|---|
| `levenshtein_text` | `(&Text, &Text) -> Int` |
| `jaro_text` | `(&Text, &Text) -> Float` |
| `jaro_winkler_text` | `(&Text, &Text) -> Float` |
| `closest_text` | `(&Text, &List<Text>, Int) -> Maybe<Text>` |
| `closest_n` | `(&Text, &List<Text>, Int) -> List<(Text, Int)>` |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 27 unit tests | all green under `--interp` |
| `property_test.vr` | 9 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 11 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 7 pins (1 `@ignore`'d) | 6 green under `--interp` |

## §2 — Findings landed in this branch

### 2.1 All three test files used a hallucinated API

| Pre-fix call | Status |
|---|---|
| `fn bytes(s: &str) -> List<Byte>` helper | `&str` is not a public type in Verum |
| `s.chars()` on `&str` | panic: `method 'str.chars' not found` |
| `mount StringDistanceOptions` | Type does not exist |
| `"hello".bytes()` returning `Bytes` | `.bytes()` returns a `ByteIter`, not a list |
| `[].to_bytes()` | No `to_bytes` method on `[]` |
| `jaro(&"st".to_text(), &"st".to_text())` | `jaro` takes `&[Byte]`, not `&Text` |
| `closest_text(q, &dict)` 2-arg | Requires explicit `max_distance` 3rd arg |
| `closest_text(query.to_text(), &dict, 3)` | 1st arg is `Text` by value, signature wants `&Text` |

**Fix in this branch**: all three test files rewritten to use the
actual API surface — `Text.as_bytes()` for byte-slice form,
`closest_text(&q, &dict, max_distance)` with explicit args.

### 2.2 Tuple-field access through `List<(Text, Int)>[i]` mis-resolves

Direct access `closest_n(...)[0].0` / `.1` exercises the same defect
class as the field-access-through-List-subscript defect surfaced in
`coinductive/regression_test.vr §E`
(`[[btree_pattern_match_ref_generic_class]]`).

* Symptom: assertion fails non-deterministically; sometimes
  `r[i].1 <= r[i+1].1` holds and sometimes not (the resolver picks an
  inconsistent stdlib record's `.1` offset).
* Workaround: clone the element into an explicitly-typed local first
  (`let entry: (Text, Int) = list[i].clone(); let (a, b) = entry`).
* Pinned at `regression_test.vr §G` as `@ignore`'d.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.string_distance` that need surface validation:

* `core.cli.*` — "did you mean X?" suggestion surface (closest_text).
* `core.search.fuzzy` — record-linkage / fuzzy dedup (jaro_winkler).
* No other `core/` modules reference this layer.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures
of `levenshtein`, `jaro`, `closest_text`, etc.

## §5 — Action items landed in this branch

1. `core-tests/base/string_distance/unit_test.vr` — rewritten end-to-end
   with `Text.as_bytes()`; 27 tests across 7 sections covering
   Levenshtein fixtures, bounded variant, Jaro, Jaro-Winkler,
   `closest_text` autocorrect, `closest_n` top-K, and Text-overload
   parity.
2. `core-tests/base/string_distance/property_test.vr` — rewritten
   with `Text.as_bytes()`; 9 algebraic laws (Levenshtein metric
   axioms + Jaro unit-interval + Jaro-Winkler >= Jaro).
3. `core-tests/base/string_distance/integration_test.vr` — rewritten
   without the hallucinated `StringDistanceOptions` / `.bytes()` /
   `.to_bytes()` API; 11 scenarios covering autocorrect + spellcheck
   simulation + cross-algorithm agreement + triangle inequality.
4. `core-tests/base/string_distance/regression_test.vr` — NEW: 7
   pinned regressions covering the four hallucination classes (§A-§F)
   plus one `@ignore`'d entry for the tuple-field-access defect (§G).
5. `core-tests/base/string_distance/audit.md` — NEW (this file).

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Tuple-field access through `List<(T, U)>[i]` close-out | multi-day VBC codegen work | shared with [[btree_pattern_match_ref_generic_class]] |
| Default-argument syntax for `closest_text` 3rd arg `Int = 3` | medium — confirm whether call-site syntax supports omission | future task |
| Damerau-Levenshtein (adjacent-transposition) variant | 2-3h | future task |
| Unicode-grapheme-aware distance | gated on `Text.graphemes()` iterator landing | future task |
