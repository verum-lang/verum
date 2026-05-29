# `net/quic/connection_id` audit

Module: `core/net/quic/connection_id.vr` (~110 LOC) — QUIC
connection IDs with the RFC 9000 §5.1 length bound (≤ 20 bytes).

Surface: `MAX_CID_LEN`, `ConnectionId.from_bytes / .empty / .len /
.as_slice / .eq`, `CidError.TooLong` + Display/Debug/Eq.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.packet` | source/destination CID fields in long/short headers. |
| `core.net.quic.cid_pool` | active CID set per connection. |
| `core.net.quic.stateless_reset` | reset-token association. |

## 2. Crate-side hardcodes

`MAX_CID_LEN = 20` is the RFC 9000 §5.1 hard limit; `from_bytes`
returns `Err(TooLong(n))` above it. The 20-byte boundary (accept)
and 21-byte (reject) are both pinned.

## 3. Language-implementation findings

None. `from_bytes` already uses the EXTSLICE-1-safe indexed
byte-copy (`while i < src.len() { b.push(src[i]); … }`) rather than
a sub-range slice, so it compiles cleanly from a user test. Tests
pass `List.as_slice()` (heap-backed) — the safe `&[Byte]` form
(cf. TEXT-SMALLSTR-ASBYTES-1, which only affects small-string
`Text.as_bytes()`).

## 4. Action items landed in this branch

* `unit_test.vr` — 13 tests: MAX_CID_LEN; empty; from_bytes
  accept (8 / 20-boundary / empty) + reject (21); byte
  preservation; eq byte-exact true/false; CidError payload + Eq.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ConnectionId Display/Debug (hex) round-trip | this folder | 1h |
| cid_pool rotation + retire integration | quic/cid_pool folder | gated on pool API |
