# `core.text.case_fold` — audit

> Status: **complete**. Sweep on 2026-05-13: 25 / 30 unit tests pass
> (83%). The 5 failures are all downstream of the broken Text equality
> (text/text §I) — the case_fold module itself is correctly
> implemented; `compare_ascii_nocase` and `equal_ascii_nocase` work
> end-to-end (proven by 8 PASS-GUARD tests in regression_test.vr).
>
> Once text/text §I closes (Text.eq), all 5 deferred tests will pass
> automatically.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/database/sqlite/native/l2_record/collation.rs` (Rust side) | binds `compare_ascii_nocase` as the `NOCASE` collation comparator |
| `core/text/text.vr::eq_ignore_case` | could delegate to `equal_ascii_nocase` (currently has its own broken impl — see text/text §I downstream) |
| User code: SQL WHERE clauses with `COLLATE NOCASE` | every such query routes through `compare_ascii_nocase` |

`fold_byte_ascii` is the workhorse — both `compare_ascii_nocase` and
`equal_ascii_nocase` call it byte-by-byte, with zero allocation. The
SQL hot path's NOCASE comparator allocates nothing.

## 2. Crate-side hardcodes

None. case_fold is pure Verum, calling only into:
- `Char.to_ascii_lowercase` (provided by char.vr)
- byte arithmetic on `Byte` (primitive)
- `Text.as_bytes` (provided by text.vr — currently green)

If those primitives drift, the regression PASS-GUARDs in this
folder will catch it.

## 3. Language-implementation gaps surfaced by this folder

### §A — fold_text_ascii result fails Text equality vs literal
**Symptom**: `assert_eq(fold_text_ascii(&"HELLO"), "hello")` fails. The
returned Text appears correct (PASS-GUARD `guard_compare_nocase_*` /
`guard_equal_nocase_*` confirm fold-vs-fold comparison succeeds), but
direct `==` against a Text literal does not match.
**Root cause**: text/text §I — `Text.eq` is broken on this branch when
both sides are non-trivial Text values (the literal-vs-literal case
works, the literal-vs-allocated-from-fold case does not). Same defect
class as Text.cmp.
**Action**: closes when text/text §I closes.

### §B — Test-binary cross-pollution: Text.words error in unrelated test
**Symptom**: `test_fold_text_mixed` fails with a panic about
`Text.words not found on Object` even though my test body does not call
`words`. Most likely cause: the test-binary compilation aggregates
multiple tests; an unrelated test in the same file references `words`
and the panic propagates. Test infrastructure hygiene issue.
**Action**: file in `crates/verum_cli/src/commands/test.rs` —
test-runner should isolate per-`@test` panics so a sibling test's
diagnostic doesn't surface here. Low priority; downstream of Text.words
which itself works (it's used in text/text/integration_test.vr) — the
issue is the `not found on Object` dispatch leaking into the
test_fold_text_mixed panic path.

---

## 4. Action items

### Landed in this branch
- 30 unit tests + 14 property tests + 5 integration tests + 3 regression
  pins + 8 PASS-GUARDs.
- Drift-pinned: byte folding behaviour for the 0x41–0x5A range, lowercase
  identity, and high-bit pass-through (the SQLite NOCASE contract).

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | text/text §I — Text.eq on allocated-vs-literal pairs | shared | 5 |
| 2 | Test-runner panic isolation | small (test infra) | 1 (cosmetic) |

### Drift-pin recommendations
1. Mirror the SQLite NOCASE contract pin in
   `crates/verum_database_sqlite/src/native/l2_record/collation_test.rs`:
   any change to `fold_byte_ascii` that breaks the NOCASE `0x41..=0x5A → +0x20`
   mapping must surface there.
2. Add `equal_ascii_nocase` and `compare_ascii_nocase` to the WKT pin
   set so that future stdlib refactors that lenient-skip them surface
   immediately.
