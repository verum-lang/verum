# `net/quic/recovery/pn_space` audit

Module: `core/net/quic/recovery/pn_space.vr` — RFC 9000 §12.3 packet-
number spaces: `PnSpaceKind` 3-variant + as_index, `PnSpace` state
(`&mut self` next_packet_number), `SentPacketInfo`, `ranges_contain`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.recovery.loss_detection` | per-space loss + RTT accounting. |
| `core.net.quic.connection_sm` | Initial/Handshake/1-RTT pn allocation. |

## 2. Crate-side hardcodes

The three RFC 9000 §12.3 packet-number spaces (Initial=0, Handshake=1,
Application=2) and their distinctness are pinned.

## 3. Language-implementation findings

None for the covered surface. PnSpaceKind is a 3-variant unit enum +
as_index `match`. `PnSpace.next_packet_number` (`&mut self`) + monotonicity
deferred to the bind-event-discipline FSM coverage.

## 4. Action items landed in this branch

* `unit_test.vr` — as_index (0/1/2) + pairwise distinctness + range bound.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| PnSpace.next_packet_number monotonicity (bind-event discipline) | this folder | 1h |
| ranges_contain over AckRange list | this folder | 1h |
| SentPacketInfo accounting | this folder | 1h |
