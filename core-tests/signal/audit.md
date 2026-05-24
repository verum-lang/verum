# `signal` audit

Module: `core/signal.vr` (366 LOC) — high-level async wrapper around
`core.sys.signal`. Exposes POSIX signals as awaitable Future /
AsyncIterator that compose with `select`, structured concurrency
(`nursery`), and cancellation.

Module: `core/sys/signal.vr` (~650 LOC) — low-level signal-handling
primitives: `Signal` enum (18 variants), `SignalFlag`, `SignalError`,
`on_signal` async hook, signal-handler registration via syscall.

Tests cover the testable surface today: `Signal` ADT + `to_raw()`
canonical numbering. The async surface (ctrl_c, signal_stream,
on_signal_first, AsyncIterator/Stream) requires runtime instrumentation
and is tested at the language level in `vcs/specs/L2-standard/signal/`.

## 1. Cross-stdlib usage

`Signal` is consumed by:

| crate / module | what it does |
|---|---|
| `core.signal` | Wraps each Signal variant into a Future + AsyncIterator via the self-pipe trick + 20ms poller. |
| `core.async.shutdown` | composes `Signal.Int`, `Signal.Term`, `Signal.Hup` into `shutdown_signals()` stream. |
| Application code | `ctrl_c().await` for graceful shutdown triggers. |

`SignalFlag` is consumed by:
* OS-level signal handlers (`@signal_handler`-decorated fns) — set
  the AtomicBool from inside async-signal-safe code.
* The background poller — clears flags and broadcasts.

## 2. Crate-side hardcodes

`crates/verum_runtime/src/signal_*.rs` (when implemented) hardcodes
the Signal variant → libc signal number mapping. The `to_raw()`
method is the canonical mapping; any Rust-side codegen that emits
syscall instructions for signal handling MUST agree with it. Drift
caught by `test_signal_kill_is_nine` + other per-signal pinning
tests.

`crates/verum_codegen/src/llvm/syscall_registry.rs` declares
`sigaction(2)` + `signal(2)` POSIX FFI signatures. The signal
number arguments thread through `to_raw()` at call sites.

## 3. Language-implementation gaps

### §3.1 Async surface untestable without runtime instrumentation

Every method that takes `&self` async parameter (ctrl_c, signal_stream,
on_signal_first, etc.) requires:
* A live executor
* A background poller thread
* Either real signal delivery OR a mock SignalFlag setter

Today the only test path is via the language-level
`vcs/specs/L2-standard/signal/` suite which uses
`@test_runtime(SingleThreadedExecutor)` to drive futures synchronously.
The `core-tests/signal/` folder here covers the underlying data
contract that those L2 tests build on.

### §3.2 Windows signal mapping limited to console events

`Signal.to_raw()` on Windows returns:
* `Int` → 0 (CTRL_C_EVENT)
* `Term` → 2 (CTRL_CLOSE_EVENT)
* `Abort` → 6 (consistency with Unix)
* All others — currently UNDEFINED behavior (returns implementation-
  defined default per the @cfg branch in `signal.vr:230+`).

Document this limitation in the website docs; add Windows-specific
panic for un-mappable signals OR document fallback semantics.

**Effort:** ~30 min doc + 2h Windows codegen verification.

### §3.3 No `Signal.from_raw(Int) -> Maybe<Signal>` inverse

The forward `Signal.to_raw()` mapping is exhaustive but there's no
inverse — e.g. for parsing signal numbers from `kill -l` output or
`signalfd(2)` events. Add a `from_raw(n: Int) -> Maybe<Signal>` that
returns `Some(variant)` for canonical numbers and `None` for
real-time signals.

**Effort:** ~30 min + round-trip property test.

## Action items landed in this branch

* `core-tests/signal/unit_test.vr` — Signal ADT construction +
  disjointness + canonical `to_raw()` numbering for 12 variants.
* `core-tests/signal/property_test.vr` — to_raw injectivity, kill
  canonical 9, POSIX range, shutdown triple distinctness.
* `core-tests/signal/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `Signal.from_raw(Int) -> Maybe<Signal>` + round-trip test | `core/sys/signal.vr` + tests | 30 min |
| Document Windows signal mapping limitation | website docs + `signal.vr` comment | 30 min |
| Wire `vcs/specs/L2-standard/signal/` cross-reference to this folder | both audits | 10 min |
| Add `Signal.Eq` / `Signal.Display` / `Signal.Debug` impls if missing | `core/sys/signal.vr` | 1h |
