# `net/h3/priority` audit

Module: `core/net/h3/priority.vr` — RFC 9218 Extensible Priorities:
urgency/incremental priority parameters + PRIORITY_UPDATE frame types.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.h3.connection` | PRIORITY_UPDATE frame handling + scheduling. |
| `core.net.weft` | server-side response prioritisation. |

## 2. Crate-side hardcodes

DEFAULT_URGENCY=3, MAX_URGENCY=7 (RFC 9218 §4.1 urgency 0..7), and the
PRIORITY_UPDATE_REQUEST=0xF0700 / _PUSH=0xF0701 frame types (§7.1) are
pinned, with the default-in-range + 3-bit-fit + request≠push invariants.

## 3. Language-implementation findings

None. Pure `UInt8`/`UInt64` constants. The priority field-value parser
(`urgency`/`incremental` from the sf-dictionary) gated on the parser surface.

## 4. Action items landed in this branch

* `unit_test.vr` — urgency constants + range invariants + 2 PRIORITY_UPDATE
  frame types + distinctness.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Priority field-value (sf-dictionary) parse/serialise | this folder | gated on parser |
