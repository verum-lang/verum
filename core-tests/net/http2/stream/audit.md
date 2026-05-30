# `net/http2/stream` audit

Module: `core/net/http2/stream.vr` (~272 LOC) — the RFC 7540
§5.1 HTTP/2 stream state machine.

Surface: `StreamState` (7 states) + `StreamEvent` (8 events) +
`StreamFsm.step` (the full §5.1 transition table) +
`StreamTransitionError` + `next_client_stream_id` /
`next_server_stream_id` (§5.1.1 id allocation).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.mod` | per-stream lifecycle within a connection. |
| `core.net.http2.error` | invalid transitions map to PROTOCOL_ERROR. |

## 2. Crate-side hardcodes

The transition table is the RFC §5.1 diagram transcribed
literally. Every asserted transition cites the RFC leg. The
31-bit id space ceiling (`0x7FFFFFFF`, §5.1.1) is pinned by
`test_h2stream_client_id_exhaustion_is_none`.

## 3. Language-implementation findings

### §3.1 MUTSELF-MATCH-1 — persisted `self.state` lost after an inline payload-event `step` (OPEN)

`StreamFsm.step(&mut self, event: &StreamEvent)` computes
`let next = match (self.state, event) { … }; self.state = next;`. When the
event argument is an **inline-constructed payload-bearing variant**
(`fsm.step(&StreamEvent.SendHeaders { end_stream: false })`), the trailing
`self.state = next` writeback **does not persist** to the caller — the FSM
does not advance across calls.

Evidence (two probes + the multi-step suite):

* `test_h2stream_idle_send_headers_open` (one step, checks the **returned**
  state `s`) — PASSES: `step` computes and returns the right `next`.
* `test_h2stream_probe_persist_after_let_underscore` /
  `…_after_match_discard` (one step, then check `fsm.state()`) — FAIL:
  `fsm.state()` still reads `Idle`. So the *return* is correct but the
  *persisted field* is not.
* `property_h2stream_rst_closes_from_open` reaches `Open` via a
  **parameter-passed** setup event (`assert_rst_closes(setup: &StreamEvent)`)
  and PASSES — isolating the trigger to the **inline `&`-constructed
  payload-variant argument** colliding with the `&mut self` receiver, not to
  multi-step per se.
* The sibling `Http2DynamicTable.insert` (`&mut self`, direct field writes,
  no payload-variant arg) persists correctly.

A stdlib reformulation (`let cur = self.state;` before the match) did **not**
fix it — confirming the defect is in VBC codegen for the call site, not the
match shape. The trigger is narrow: an inline `&`-constructed **payload-bearing**
variant arg. Inline **unit** variant args (`&StreamEvent.SendRstStream`) and
**bound-local** args (`let e = …; fsm.step(&e)`) both persist correctly —
proven by `test_h2stream_probe_persist_after_bound_event` (GREEN) vs
`…_after_let_underscore` / `…_after_match_discard` (inline, `@ignore`'d).

**Fix discipline (applied):** bind a payload-bearing event to a local before
passing it to a `&mut self` method — `let e = StreamEvent.X { … }; fsm.step(&e)`.
With this discipline the **full multi-step FSM suite is GREEN** (open→half-closed
→closed paths, closed-absorbing, RST-closes); only the two explicit inline-event
probes stay `@ignore`'d to pin the defect itself. The fundamental codegen fix
(inline `&`-payload-variant arg must not clobber the `&mut self` receiver
writeback) remains tracked as **MUTSELF-MATCH-1** (catalogue §13).

## 4. Action items landed in this branch

* `unit_test.vr` — construction/accessors; every Idle out-edge; the full
  Open→half-closed→Closed data path + RST + Closed-absorbing (bound-event
  discipline); StreamState/StreamTransitionError ADT; §5.1.1 id allocation.
  Plus a GREEN bound-event persistence probe + 2 `@ignore`'d inline-event
  probes pinning MUTSELF-MATCH-1.
* `property_test.vr` — RST-closes from the 5 non-terminal states,
  closed-rejects-every-event (bound setup), client-id-odd /
  strictly-increasing, server-id-even (parity-correct seeds).
* `regression_test.vr` — idle-headers open-vs-half-closed (state-after-step),
  closed-absorbing, id-parity pins.
* Net: **33 GREEN, 2 `@ignore`'d** (the inline-event MUTSELF-MATCH-1 probes).

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| MUTSELF-MATCH-1 codegen fix — un-gates the 2 inline-event probes | compiler | multi-day (catalogue §13) |
| Flow-control window accounting integration with stream FSM | stdlib + tests | gated on mod.vr |
| Per-state full event matrix (8×7 exhaustive legality table) | this folder | 2h, gated on MUTSELF-MATCH-1 |
