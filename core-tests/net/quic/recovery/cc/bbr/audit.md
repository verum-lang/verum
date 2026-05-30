# `net/quic/recovery/cc/bbr` audit

Module: `core/net/quic/recovery/cc/bbr.vr` — BBR congestion control
(draft-cardwell-iccrg-bbr): pacing/cwnd gain constants + the BBR state
machine.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.recovery.cc.mod` | pluggable congestion controller. |
| `core.net.quic.pacer` | pacing rate = gain × bottleneck bandwidth. |

## 2. Crate-side hardcodes

The BBR gains are pinned: HIGH_GAIN≈2.885 (2/ln2), DRAIN_GAIN≈0.346,
CRUISE_GAIN=1.0, PROBE_UP=1.25, PROBE_DOWN=0.75, STARTUP_FULL_BW_THRESH=1.25,
STARTUP_FULL_BW_COUNT=3 — plus the HIGH>CRUISE>DRAIN ladder and the
ProbeBW straddle-unity invariant.

## 3. Language-implementation findings

None for the covered Float/UInt32 gain constants. The Duration-typed window
constants (PROBE_RTT_INTERVAL / PROBE_RTT_DURATION / MIN_RTT_WINDOW) and the
BBR `&mut self` phase machine are deferred — Duration return-unboxing
(NEWTYPE-UNBOX-1) + bind-event discipline.

## 4. Action items landed in this branch

* `unit_test.vr` — 7 gain constants + gain-ladder + straddle-unity invariants.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Duration window constants | this folder | gated on NEWTYPE-UNBOX-1 |
| BBR phase state machine (Startup/Drain/ProbeBW/ProbeRTT) | this folder | gated on bind-event + Duration |
