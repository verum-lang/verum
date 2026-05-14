# `core.text.char` — audit

> Status: **partial**. Sweep on 2026-05-13: 75 / 86 unit tests pass
> (87%). The remaining 11 failures concentrate in 5 defect classes,
> tracked in `regression_test.vr` and listed below. The largest defect
> class (§E, AnyChar / char_any_of) shares its root with the text/text
> §C iterator-dispatch defect — closing one closes both.

---

## 1. Cross-stdlib usage

`Char` is used heavily across the stdlib:

| Module | Usage |
|---|---|
| `core/text/text.vr` | `Text.chars()` yields `Char`; `Text.push(c: Char)`; case predicates / conversions; `from_char` |
| `core/text/format.vr` | `Formatter.write_char(c: Char)`; alignment fill character |
| `core/text/case_fold.vr` | `fold_char_ascii(c: Char) -> Char` |
| `core/text/regex.vr` | `Regex.captures` returns text containing matched chars |
| `core/io/stdio.vr` | `print_char(c: Char)` (debug) |
| `core/configuration/parser.vr` | tokenisation reads chars |

`GeneralCategory` is used in:
- `core/text/case_fold.vr` (Unicode case-fold filters by category)
- Future ICU collation cog

`CharPattern`, `AnyChar`, `CharRange` are used in:
- `core/text/text.vr::trim_matches(pat: CharPattern)`
- `core/text/text.vr::split` predicates

## 2. Crate-side hardcodes

| Path | What | Pin |
|---|---|---|
| `crates/verum_common/src/well_known_types.rs` | `TypeId::CHAR = ?` | Char primitive type-id |
| `crates/verum_runtime/src/runtime/value.rs` | `Value::Char(u32)` | Runtime representation as 32-bit code point — note: this collapses to `Value::Int` in some dispatch paths (root cause of §B mutation) |
| `crates/verum_vbc/src/codegen/...` | Char literal lowering | `'a'` -> NaN-boxed code point |
| Char ↔ Int casts | `c as Int`, `n as Char` | Bidirectional cast lowering — currently round-trip-safe per integration tests |

## 3. Language-implementation gaps surfaced by this folder

### §A — `Char.make_ascii_{upper,lower}case` does not mutate — **TRACKED AS TASK #13 (2026-05-14)**
**Symptom**: `let mut c: Char = 'a'; c.make_ascii_uppercase()` leaves
`c == 'a'`. Body is `*self = self.to_ascii_uppercase();`.
**Bisection (this session)**: the defect reproduces identically for
**every primitive** `&mut self` method that uses `*self = X` to write
back — not just Char.  A user-defined `implement Int { fn
double_in_place(&mut self) { *self = *self * 2 } }` also fails to
persist.  So the root is at the codegen / call-dispatch layer for
primitive receivers: when a primitive receiver dispatches a `&mut self`
method, the call site's `takes_self_mut_ref → RefMut(ref_reg,
receiver_reg)` wrapping at `expressions.rs:8731` is bypassed (likely
the receiver_is_primitive_numeric path at line 8109+ short-circuits
to an inline-sequence dispatch that drops the RefMut step).  The
Deref / DerefMut Tier-0 handlers themselves are correct (verified at
`cbgr.rs:159+ / 273+`).
**Status**: file open as task #13 — fix needs the codegen path for
primitive `&mut self` to route through the same RefMut wrapping as
user-type `&mut self`.  Pinned by
`core-tests/text/char/regression_test.vr::regression_a_make_ascii_uppercase_pinned`.

### §B — `eq_ignore_ascii_case` false-negative
**Symptom**: `'A'.eq_ignore_ascii_case(&'a')` returns false. Body is
`self.to_ascii_lowercase() == other.to_ascii_lowercase()`. Since the
pure conversion is correct, the equality must be wrong — `Char.eq` for
distinct Char values pre-conversion vs post-conversion may not unify.
**Action**: trace `==` for `Char` against its protocol impl (likely a
`Char.eq -> Bool` direct comparison of u32 values). May be downstream
of §A if the comparison reads stale `&self`.

### §C — `Char.from_digit(N, 16)` for N >= 10 returns wrong char
**Symptom**: `from_digit(10, 16)` should return `Some('a')`. Empirically
returns a different Char.
**Root cause hypothesis**: the body branches on `digit < 10` to add
'0' as Int, else add `'a' as Int - 10`. The else branch likely uses
the wrong base (perhaps `'A'` instead of `'a'`, or a stale offset).
**Action**: read core/text/char.vr:251–268 carefully; one-line fix
likely.

### §D — `Char.general_category` misroutes
**Symptom**: `'a'.general_category()` does not return `GeneralCategory.Ll`.
Same for 'A' → Lu, '5' → Nd, ' ' → Zs.
**Root cause hypothesis**: variant-tag stability under per-file
compilation (MEMORY §22). The variant returned at construction time
inside `general_category()` does not match the variant tag observed at
the call-site `is GeneralCategory.Ll` check. Cross-pollution across
test files compiled in the same archive.
**Action**: shares root with task #22 — closes when that closes.

### §E — `AnyChar.matches` panics on Iterator.next dispatch
**Symptom**: `char_any_of(&['a','b']).matches('a')` panics with
`method 'next' not found on receiver of runtime kind 'Int'`.
**Root cause**: same as text/text §C — Iterator.next dispatch is
broken for the iterator returned by `chars.iter()` inside the
`AnyChar.matches` body. The receiver-kind classification falls through
to `Int` (probably because the iterator's state field is an Int
index).
**Action**: closes when text/text §C closes (multi-session work).

---

## 4. Action items

### Landed in this branch
- 86 unit tests + 24 property tests + 8 integration tests + 5 regression
  pins + 5 PASS-GUARDs.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A — `&mut Char` deref-assign mutation semantics | medium | 2 (+ §B downstream) |
| 2 | §B — `eq_ignore_ascii_case` (downstream of §A) | downstream-only | 1 |
| 3 | §C — `from_digit` hex offset | small | 1 |
| 4 | §D — variant-tag stability (shares with task #22) | shared | 4 |
| 5 | §E — Iterator.next dispatch (shares with text/text §C) | shared | 3 |

### Drift-pin recommendations
1. `crates/verum_common/src/well_known_types.rs::CHAR_PRIMITIVE_PIN`:
   pin the Char TypeId and the canonical method-list (is_ascii,
   to_ascii_uppercase, encode_utf8, ...) so a future stdlib lenient-
   skip on Char surfaces immediately.
2. `crates/verum_vbc/src/codegen/tests/char_mut_assign.rs`: write a
   minimal test that `let mut c: Char = 'a'; *(&mut c) = 'b'; c == 'b'`
   to lock in the §A fix once it lands.
