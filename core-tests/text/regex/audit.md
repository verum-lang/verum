# `core.text.regex` — audit

> Status: **regression-only**. Sweep on 2026-05-13: 8 / 31 unit tests
> pass (26%). Regex construction (Regex.new), as_str round-trip, and
> simple literal `is_match` work; everything that touches the
> intrinsic-output decoding path (find / find_all / split / captures /
> replace_all) panics or returns wrong results.

---

## 1. Cross-stdlib usage

| Module | Usage |
|---|---|
| `core/text/text.vr::matches` / `match_indices` (proposed integration) | uses TextMatches/TextMatchIndices iterator types — orthogonal to Regex but conceptually adjacent |
| `core/text/tagged_literals.vr::rx#"…"` / `re#"…"` | parses to `Regex.new(pattern).unwrap()` at compile time |
| `core/database/sqlite/...` REGEXP | Regex backs `REGEXP` operator when `core.database.sqlite.regexp` cog is mounted |
| User code: any input validation, log parsing, structured-text extraction | the only Verum-side regex engine |

The regex intrinsics are surfaced from the Rust `regex` 1.x crate
through 7 intrinsics in
`crates/verum_vbc/src/intrinsics/registry.rs`. Closing the
output-decoding defects in §B/§D requires fixing the bridge between
the Rust-side `Vec<String>` / `Option<String>` and the Verum-side
`List<Text>` / `Maybe<Text>` ABI.

## 2. Crate-side hardcodes

| Path | What | Hardcoded |
|---|---|---|
| `crates/verum_vbc/src/intrinsics/registry.rs` | 7 regex intrinsics by canonical name | `regex_is_match`, `regex_find`, `regex_find_all`, `regex_replace`, `regex_replace_all`, `regex_split`, `regex_captures` |
| `crates/verum_vbc/src/intrinsics/runtime/regex.rs` | bridge to Rust `regex::Regex` | reads pattern as Verum Text → Rust `&str` → compiles fresh per call (no cache) |
| `crates/verum_vbc/src/instruction.rs` | `TensorSubOpcode 0xE0..=0xE3` and `TensorExtSubOpcode 0x0A..=0x0C` | regex-dedicated bytecode slots |

## 3. Language-implementation gaps surfaced by this folder

### §A — `Regex.is_match` returns wrong Bool on common patterns
**Symptom**: `\d+`, `^hello`, `world$`, alternation patterns —
all return `false` when they should return `true` (or vice versa for
absent matches). Literal patterns work.
**Root cause hypothesis**: the `regex_is_match` intrinsic at
`crates/verum_vbc/src/intrinsics/runtime/regex.rs` either passes the
pattern with extra escapes / wrong encoding, OR the output Bool is
being mis-interpreted (NaN-boxed differently than the Verum runtime
expects).
**Action**: add a Rust-side test that calls the intrinsic directly
with hand-built Verum Text values and verifies the output. Pinpoint
which side (input encoding vs output decoding) is wrong.

### §B — `find_all` NullPointerAt opcode 0x66 (SetIdx on List)
**Symptom**: every `find_all` panics with NullPointerAt at opcode
0x66. 0x66 is `SetIdx` (set list index by integer) — the runtime is
writing through a null pointer when populating the result list.
**Root cause hypothesis**: the intrinsic returns its result as a
Verum List — but the List is uninitialised (cap=0, ptr=null) when
`SetIdx` runs, OR the receive-side codegen doesn't allocate before
writing. Same family as text/text §E (Text.truncate NullPointer at
opcode 0x63 SetF).
**Action**: trace which path constructs the empty List in the
intrinsic bridge.

### §C — `split` NullPointerAt opcode 0x66
**Symptom**: same SetIdx defect as §B.
**Action**: closes when §B closes.

### §D — `find` returns wrong Maybe<Text> shape
**Symptom**: `r.find(...)` panics with `Expected pointer, got
Some(3)`. The runtime is interpreting a `Maybe<Text>` value where a
raw pointer is expected — likely the receive-side decoding of the
intrinsic's `Option<String>` mishandles the `None` case (or treats
the Some wrapper as the value itself).
**Action**: same Verum/Rust ABI bridge audit as §B.

### §E — `replace_all` content not applied
**Symptom**: `r.replace_all(...)` returns a Text that does not contain
the replacement. Either the intrinsic is no-op or the result Text
isn't being decoded back from the Rust side.
**Action**: covered by the §A/B/C/D audit pass.

---

## 4. Action items

### Landed in this branch
- 31 unit tests + 9 property tests + 7 integration tests + 8 regression
  pins + 3 PASS-GUARDs.

### Deferred
| # | Item | Effort | Tests unblocked |
|---|------|------:|------:|
| 1 | §A — Regex intrinsic input/output encoding | medium | ~6 (every is_match beyond literal) |
| 2 | §B — Verum/Rust List<Text> ABI fix | medium | ~9 (find_all + split + captures) |
| 3 | §D — Verum/Rust Maybe<Text> ABI fix | medium | ~3 (find + captures) |
| 4 | §E — replace_all (downstream of §A) | downstream | ~3 |

### Drift-pin recommendations
1. Add a Rust-side `crates/verum_vbc/src/intrinsics/runtime/regex.rs::tests`
   that round-trips a 7-intrinsic golden-suite (literal match, anchor,
   class, alternation, find vs find_all, split, captures).
2. Pin the 7 intrinsic names + opcode slot numbers in
   `crates/verum_common/src/well_known_types.rs::REGEX_INTRINSIC_PIN`.
