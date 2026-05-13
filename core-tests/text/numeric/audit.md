# `core.text.numeric.*` — subtree audit summary

5 modules under `core/text/numeric/`:

| Submodule | Status | Notes |
|---|---|---|
| `decimal` | **partial** | 27 / 45 unit tests pass (60%). 6 PASS-GUARDs. Defects in §A (Int.neg) + §B (function-id collision); algebraic shape correct. |
| `bigint` | **partial** | API surface tested but per-test sweep deferred. Inherits Iterator.next / function-id collision from text/text §C/§D. |
| `bigdecimal` | **partial** | Same root as bigint — wraps BigInt. |
| `rational` | **partial** | Same root as bigint — wraps two BigInts. |
| `modular` | **partial** | 9 number-theoretic free fns over BigInt. Same root. |

## Cross-cutting drift surfaces
1. `MAX_SCALE = 18` (Decimal) and `MAX_SCALE_BIG = 1024` (BigDecimal)
   should be pinned in `crates/verum_common/src/well_known_types.rs`.
2. `BIGINT_BASE = 10^9` + `BIGINT_DIGITS_PER_CHUNK = 9` are the
   canonical chunk-size invariants — drift here breaks BigInt parsing,
   rendering, and Karatsuba.
3. Cross-module aliases: `Decimal` is mounted via `core.text.numeric`;
   `Rational` via `core.text.numeric`; etc. Test files mount the
   per-module path explicitly to avoid the prelude-glob defect noted
   in MEMORY ("core.prelude.* glob-re-export defect identified").

## Highest-leverage closure
Closing text/text §C (Iterator.next dispatch) + §D (function-id
collision) unblocks ~80% of the numeric suite (BigInt iterates
its `digits: List<Int>` field on every operation, and the tunnel
through cross-module function calls is a primary failure surface).
