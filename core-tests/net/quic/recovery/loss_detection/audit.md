# `net/quic/recovery/loss_detection` audit

Module: `core/net/quic/recovery/loss_detection.vr` — RFC 9002 §6 loss
detection: kPacketThreshold / kGranularity / kTimeThreshold / PTO backoff
constants + the loss-detection timer logic.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.recovery.rtt` | RTT feeds the time-threshold. |
| `core.net.quic.recovery.pn_space` | per-space sent-packet loss marking. |
| `core.net.quic.connection_sm` | PTO arm/fire on the timer. |

## 2. Crate-side hardcodes

kPacketThreshold=3, kGranularity=1ms, kTimeThreshold=9/8, MAX_PTO_BACKOFF=8
are RFC 9002 §6.1.1/§6.1.2 RECOMMENDED values, each pinned, plus the
9/8 > 1 expansion invariant.

## 3. Language-implementation findings

None. Pure `UInt64`/`UInt32` constants. The timer/loss-marking logic
(`&mut self` + Duration) gated on NEWTYPE-UNBOX-1 + bind-event discipline.

## 4. Action items landed in this branch

* `unit_test.vr` — 5 RFC 9002 §6.1 constants + the time-threshold invariant.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| detect_lost_packets / PTO timer logic | this folder | gated on NEWTYPE-UNBOX-1 |
