# `net/quic/path_mtu` audit

Module: `core/net/quic/path_mtu.vr` — RFC 8899 DPLPMTUD (QUIC §14):
PLPMTU size ladder + probe/timer constants + the search state machine.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | path MTU discovery via PMTU probes. |
| `core.net.quic.pacer` | packet sizing within the discovered PLPMTU. |

## 2. Crate-side hardcodes

BASE_PLPMTU=1200 (RFC 9000 §14 minimum), ETHERNET_PLPMTU=1452,
JUMBO_PLPMTU=9000, MAX_PLPMTU=65527 (65535−8), MAX_PROBES=3,
RAISE_TIMER_SECS=600 are RFC 8899 / RFC 9000 §14 verbatim and pinned, plus
the monotonic search ladder BASE < ETHERNET < JUMBO < MAX.

## 3. Language-implementation findings

None. Pure `Int` constants. The DPLPMTUD search FSM (`&mut self`) is gated
on the bind-event discipline.

## 4. Action items landed in this branch

* `unit_test.vr` — 6 size/probe/timer constants + monotonic ladder.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| DPLPMTUD probe state-machine (bind-event discipline) | this folder | 2h |
