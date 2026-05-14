# `core.sys.signal` — implementation audit

## Status: **partial** (type-level + SignalFlag covered; runtime handler dispatch deferred)

* `Signal` 18-variant POSIX enum, `name()`, `is_catchable()`,
  `to_raw()` / `from_raw()` round-trip, Eq/Clone/Copy — full coverage
  in `unit_test.vr`.
* `SignalError` 4-variant — every constructor + Eq + 3-way mismatch
  matrix pinned.
* `SignalFlag` set/clear/is_set state machine — every transition
  pinned; idempotence laws covered.
* **Deferred to integration**: `on_signal`, `reset_signal`,
  `ignore_signal`, `raise_signal` — these require a real OS signal
  delivery fixture. The conformance suite runs in-process; we'd need
  to fork a child, install a handler, deliver a signal from the
  parent, and observe the SignalFlag flip. That setup is outside the
  type-conformance scope.

## 1. Cross-stdlib usage

| Consumer | Touches | Notes |
|---|---|---|
| `core/async/cancellation.vr` | SignalFlag | Used for the "shutdown requested" flag in graceful-cancellation flows. |
| `core/io/protocols.vr` | SignalError indirectly via OSError | When a signal interrupts a syscall, the EINTR errno propagates through OSError. |
| `core/cli/*` | Signal.Int, on_signal | Daemons / long-running CLIs register Ctrl+C handlers via on_signal. |

## 2. Crate-side hardcodes

| Site | What it pins | Status |
|---|---|---|
| `crates/verum_vbc/src/intrinsics/registry.rs` | `atomic_load` / `atomic_store` intrinsics consumed by SignalFlag's set/clear/is_set | OK |
| `crates/verum_codegen/src/llvm/signals.rs` | If present — signal handler lowering. (Verified absent — handler dispatch is pure Verum.) | OK |

## 3. Language-implementation gaps surfaced by this suite

### 3.1 `Signal` 18-variant tag stability

The 18-variant Signal enum has the largest variant set among the
sys/* surface. Variant-tag drift here would cause silent miscompilation
on any `from_raw` round-trip — the post-task-#22 variant-tag stability
fixes are the foundation that lets this surface work today. Pinning
the Signal name() table here is the regression guard.

### 3.2 SignalFlag atomic ordering

`SignalFlag.set()` uses `MemoryOrder.Release`; `SignalFlag.is_set()`
uses `MemoryOrder.Acquire`. This is the standard acquire-release
pattern that gives "set strictly happens-before is_set sees true"
between a signal handler thread and the main loop. The test suite
runs single-threaded so we can't pin the multi-threaded invariant,
but we pin the single-threaded happens-before via the
clear/set/is_set transition tests.

## 4. Action items landed in this branch

1. **`unit_test.vr`** — 38 @tests covering:
   - 18 Signal name() variants
   - 6 is_catchable() predicates (KILL/STOP uncatchable; INT/TERM/HUP/WINCH catchable)
   - 12 to_raw() canonical POSIX numbers + Eq consistency
   - 5 SignalError variant construction + Eq matrix
   - 5 SignalFlag state-machine transitions + idempotence

## 5. Action items deferred

1. **Runtime signal-handler delivery test** — fork a child, install
   a handler, deliver SIGUSR1 from the parent, observe the
   SignalFlag flip. Estimate: 1 day once the cross-process fixture
   infrastructure lands.
2. **Windows-specific Signal subset** — only Int / Term / Abort are
   supported on Windows. The unit tests pin the POSIX numbers; the
   Windows mapping (0 / 2 / 6) is verified at the per-platform
   crate's conformance surface.
3. **Signal vs OSError EINTR interplay** — when a signal handler
   runs during a syscall, the syscall returns EINTR. Pin this via
   the cross-stdlib integration surface once the fixture is wired.
