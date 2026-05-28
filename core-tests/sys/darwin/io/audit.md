# `core.sys.darwin.io` — implementation audit

## Status: **partial** (data-shape + errno-mapping surface complete; live kqueue deferred)

* DarwinIoDriverError 13-variant ADT + from_errno mapper +
  is_retryable classifier pinned.
* DarwinIoOpKind 9-variant exhausted.
* DarwinIoToken / DarwinIoOp / DarwinIoCqe record construction pinned.
* MAX_EVENTS=256 + DEFAULT_TIMEOUT_NS=1e9 invariants pinned.
* Live `KqueueDriver.new` / `kevent` / `kevent64` flow deferred —
  needs real kqueue + fd fixture.

## Action items landed

1. `unit_test.vr` — 30 `@test`s: 2 constants; 9-variant DarwinIoOpKind
   construction; DarwinIoDriverError 9 unit + 3 payload variants;
   from_errno mapping over 4 representative codes (EAGAIN=35,
   EINTR=4, ECONNREFUSED=61, unknown=9999); is_retryable classifier
   over WouldBlock/Interrupted/4 non-retryable; DarwinIoToken
   newtype round-trip + DarwinIoOp + DarwinIoCqe record construction.
2. `property_test.vr` — 5 algebraic laws: 9-variant count exhaustive +
   dispatch-tag pairwise distinct; retryable partition over
   ({WouldBlock, Interrupted} ⊥ 8 non-retryable); Eq reflexive over
   10 unit-variant set; MAX_EVENTS power-of-two.
3. `regression_test.vr` — 4 `@test`s: from_errno EAGAIN-to-WouldBlock
   (defect class pin); EINTR-to-Interrupted; MAX_EVENTS matches the
   array literal in KqueueDriver layout; WouldBlock retryable.

## Deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | Live `KqueueDriver.new` round-trip | Needs real kqueue fd. |
| 2 | kevent / kevent64 submit + poll cycle | Needs paired fd fixture + event injection. |
| 3 | Wake-event (EVFILT_USER with WAKE_IDENT=0x56524D57) round-trip | Needs cross-thread wake test. |
