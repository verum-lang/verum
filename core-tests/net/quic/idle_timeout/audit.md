# `net/quic/idle_timeout` audit

Module: `core/net/quic/idle_timeout.vr` — RFC 9000 §10.1 idle timeout:
`IdleTimeoutTracker` (`&mut self`), `IdleTimeoutError`, `TickResult`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | drives connection close on idle expiry. |
| `core.net.quic.transport_params` | max_idle_timeout negotiation. |

## 2. Crate-side hardcodes

`TickResult.Expired` is the §10.1 connection-MUST-close signal;
`IdleTimeoutError.AlreadyExpired` guards re-arming an expired tracker.

## 3. Language-implementation findings

None for the covered surface (unit ADT legs). `TickResult.Active(Duration)`
payload + `IdleTimeoutTracker.tick` (`&mut self`, Duration arithmetic)
deferred — Duration return-unboxing (NEWTYPE-UNBOX-1) + bind-event FSM.

## 4. Action items landed in this branch

* `unit_test.vr` — IdleTimeoutError (variant + Eq); TickResult.Expired
  (variant + disjoint-from-Active).

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| TickResult.Active(Duration) remaining-time | this folder | gated on NEWTYPE-UNBOX-1 |
| IdleTimeoutTracker.tick lifecycle (bind-event) | this folder | 1h |
