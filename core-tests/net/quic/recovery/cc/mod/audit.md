# `net/quic/recovery/cc/mod` audit

Module: `core/net/quic/recovery/cc/mod.vr` — RFC 9002 §7 congestion-
control framework: window constants + the pluggable `CongestionControl`
interface (NewReno/CUBIC/BBR).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.recovery.cc.{new_reno,cubic,bbr}` | controller implementations. |
| `core.net.quic.connection_sm` | on-ack / on-loss / on-pto cwnd updates. |

## 2. Crate-side hardcodes

MAX_DATAGRAM_SIZE=1200, INITIAL_WINDOW_BYTES=12000 (10×), MIN_WINDOW_BYTES
=2400 (2×), LOSS_REDUCTION_FACTOR=1/2 are RFC 9002 §7.2/§7.3.2 verbatim,
with the 10×/2× derivations and initial>min invariant pinned.

## 3. Language-implementation findings

None. Pure `UInt32` constants (incl. const-arithmetic derivations
10×/2× MAX_DATAGRAM_SIZE). Compile cleanly under `--interp`.

## 4. Action items landed in this branch

* `unit_test.vr` — 7 window/loss constants + derivation + ordering invariants.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| CongestionControl on-ack/on-loss transitions per controller | cc/* folders | bind-event |
