# `net/quic/stateless_reset` audit

Module: `core/net/quic/stateless_reset.vr` — RFC 9000 §10.3 stateless
reset: size constants, `StatelessResetKey` (HMAC-derived token),
`ResetTokenSet`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | emit/detect stateless reset. |
| `core.net.quic.transport_params` | stateless_reset_token param. |

## 2. Crate-side hardcodes

`STATELESS_RESET_MIN_SIZE == 21` (5 + 16-byte token) and
`STATELESS_RESET_RECOMMENDED_SIZE == 59` (43 + 16) are RFC 9000 §10.3
verbatim and pinned.

## 3. Language-implementation findings

None for the covered surface (size constants). Token derivation
(`StatelessResetKey`, HMAC) + `ResetTokenSet` membership gated on crypto
+ collection coverage.

## 4. Action items landed in this branch

* `unit_test.vr` — MIN_SIZE=21 + RECOMMENDED_SIZE=59 + ordering + token-room.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| StatelessResetKey HMAC token derivation | this folder | gated on crypto |
| ResetTokenSet insert/contains | this folder | 1h |
