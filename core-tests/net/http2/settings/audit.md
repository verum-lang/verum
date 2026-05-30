# `net/http2/settings` audit

Module: `core/net/http2/settings.vr` — RFC 7540 §6.5.2 / RFC 9113 §6.5
SETTINGS: `SettingId` parameter ids + `Settings` defaults + frame-size /
window bounds.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.frame` | SETTINGS frame param list. |
| `core.net.http2.hpack` | DEFAULT_HEADER_TABLE_SIZE seeds the dynamic table. |
| `core.net.http2.mod` | connection-level settings negotiation. |

## 2. Crate-side hardcodes

The 6 SettingId ids (HEADER_TABLE_SIZE=0x1 … MAX_HEADER_LIST_SIZE=0x6),
the §6.5.2 default initial values (header table 4096, enable_push 1,
initial window 65535, max frame 16384), and the bounds (MIN_MAX_FRAME_SIZE
=2^14, MAX_MAX_FRAME_SIZE=2^24-1, MAX_INITIAL_WINDOW_SIZE=2^31-1) are RFC
verbatim and pinned. (The umbrella http2 suite also covers these; this
folder is the strict-mirror per-submodule home.)

## 3. Language-implementation findings

None. SettingId is a single-`UInt16`-field record with `public const` ids;
the rest are free `UInt32` consts. Compile cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 6 SettingId ids + 4 defaults + 3 bounds + ordering.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Settings struct apply/validate (reject out-of-range) | this folder | 1h |
