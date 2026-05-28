# `core/encoding/pem` — conformance audit

| | |
|---|---|
| Module | `core.encoding.pem` |
| Spec | RFC 7468 (textual encoding for DER blobs) |
| Tier | regression-only (pure-data surface) |
| Status | **partial** — pure-data surface covered (PemBlock, PemError 5-variant ADT); encode/decode round-trip gated on stdlib refresh |

## What's covered (`unit_test.vr` — 21 @test, all GREEN under `--interp`)

### §1 — `PemBlock` record construction (3)
- empty-contents block
- single-byte block
- multi-byte block (DEAD BEEF round-trip)

### §2 — `PemError` 5-variant ADT construction (5)
- NoBlockFound
- MismatchedLabels(Text)
- Base64Failed
- Malformed(Text)
- TooLarge(Int)

### §3 — Variant pairwise disjointness (5)
Each variant tested against all 4 sibling variants via `is`-pattern.

### §4 — Match exhaustiveness (5)
Static dispatch function `describe(e: &PemError) -> Text` covers every
variant; one test per arm verifies the correct arm fires.

### §5 — Payload preservation (3)
Information-carrying variants (MismatchedLabels / Malformed /
TooLarge) round-trip their payload through construction +
match-arm extraction.

## Deferred

### §A — encode_block / decode_one / decode_all round-trip
The encode + decode entry points both call
`Text.from_utf8_unchecked(out.as_slice())` to produce the output Text.
That symbol still trips the lenient-stub cascade under `--interp`
(symptom: `runtime: Panic { message: "[lenient] format compiled to
panic-stub: undefined function: Text.from_utf8_unchecked (in function
encode_block)" }`). Same defect family as semver `format()`. Gated on
multi-day VBC codegen fix — task #17/#39 sister.

### §B — encode_bundle multi-block emission
Same `Text.from_utf8_unchecked` gating.

### §C — Display + Debug impls
Implementations exist (`impl Display for PemError`, `impl Debug for
PemError`) but are gated on the same stub-cascade as §A
(`f.write_str("...")` may exercise the lenient path depending on
inferred Formatter contract).

### §D — RFC 7468 canonical vectors
Pinned vector tests deferred until §A unblocks:
- CERTIFICATE block round-trip
- multi-block bundle (cert chain)
- preamble-tolerant decode (blank lines + comments before BEGIN)
- mismatched BEGIN / END label rejection
- malformed (no END marker, no BEGIN marker, truncated)

## Status taxonomy

| Audit category | Coverage | Confidence |
|---|---|---|
| ADT construction | full | high |
| Variant disjointness | full | high |
| Match exhaustiveness | full | high |
| Payload round-trip | full | high |
| Encode round-trip | deferred (§A) | n/a |
| Decode round-trip | deferred (§A) | n/a |
| Display/Debug | deferred (§C) | n/a |

## Tier-1 (AOT) gate

Not yet validated. Pre-existing AOT stdlib build blocker (task #7 in
the INVENTORY rollup — `sync_connect_binlog` + `translate` arity
defects, unrelated to PEM). When the AOT path is reopened, this suite
must re-run under `--aot` as the cross-tier validation contract.
