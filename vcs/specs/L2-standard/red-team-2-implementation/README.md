# Red-Team Round 2 — Adversarial Input Guardrails

Tracks #173. Each `.vr` test file in this directory pins one
audit-confirmed defense by exercising a deliberately hostile input
that the defense must reject.

## Coverage

| File | Defense | Source bug-class |
|---|---|---|
| `parse_u64_overflow_guards.vr` | 3-layer overflow recipe on UInt64 parsers (`semver`) | `acc * 10 + d` silent wrap-around |
| `sqlite_text_to_int_coercion.vr` | INTEGER/NUMERIC affinity → REAL fallback on overflow | dishonest-comment class (`parse_int64` claimed to "mirror SQLite" but didn't) |
| `decoder_canonicality.vr` | base64 standard / URL canonicality (no-pad / overpad / non-canonical trailing bits / alphabet boundary) | round-trip-equality forgery surface |
| `http_range_overflow.vr` | RFC 7233 byte-offset 3-layer overflow defense | CVE-2011-3192-class |
| `constant_time_compare.vr` | `constant_time_eq` / `_compare` behavioural contract (no early exit, length-mismatch handled at boundary) | timing-attack class |
| `csprng_id_entropy.vr` | UUID v4 / v7 + ULID entropy + timestamp non-zero | dishonest-stub class (lenient-codegen swallowed missing-fn calls) |
| `protobuf_varint_canonicality.vr` | 10th-byte data-bits-1..6 rejection | varint round-trip-equality forgery |
| `int_min_unary_negation.vr` | bit-flip pattern handles `Int.MIN` correctly | `(-n) as UInt64` overflow-back-to-Int.MIN |

## Adding new vectors

When the audit closes a new soundness defect:

1. Add a `.vr` file in this directory.
2. Header preamble:
   ```
   // @test: typecheck-pass | run | run-panic
   // @tier: 0
   // @level: L2
   // @tags: red-team, <bug-class>, <module>
   // @timeout: 10000
   ```
3. Each test function crafts an input the pre-fix code would have
   silently mishandled, and asserts the post-fix code surfaces the
   correct error/saturation/promotion.
4. Cross-reference the closing commit in a docstring at the top of
   the file.

These tests serve as guardrails: any future regression that drops the
defense fails CI immediately, surfacing the regression as a
red-team-vector hit rather than a silent silent-rotting test.

Cross-references:
- `vcs/red-team/round-2-implementation.md` — full vector inventory.
- `vcs/red-team/round-1-architecture.md` / `round-3-perf.md` —
  sibling rounds.
