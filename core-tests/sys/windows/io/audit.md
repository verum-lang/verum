# `core.sys.windows.io` — implementation audit

## Status: **partial** (under `--interp`; constant + token surface, IOCP runtime deferred)

* Provides the Windows-side completion-based I/O driver — IOCP
  (I/O Completion Port).  Defines `IocpDriver`, `WindowsIoOp` /
  `WindowsIoCqe` (completion-queue entry), `IocpOverlapped`,
  `WindowsIoToken` newtype, and the driver-facing functions
  `create_io_driver`, `is_iocp_available`, `async_read`, `async_write`.
* The IOCP runtime requires CreateIoCompletionPort /
  GetQueuedCompletionStatusEx / PostQueuedCompletionStatus and cannot
  run on a non-Windows host.

## 1. Cross-stdlib usage

| Caller | Use |
|---|---|
| `core.async` | The async runtime's I/O reactor delegates to `IocpDriver` on Windows (vs `epoll` on Linux, `kqueue` on macOS, `io_uring` when available). |
| `core.io.file` | File reads / writes route through `async_read` / `async_write` when the file was opened with `FILE_FLAG_OVERLAPPED`. |
| `core.net.tcp` | Async socket operations use the IOCP driver's socket-completion path. |

## 2. Pinned invariants

| Constant | Value | Why pinned |
|---|---|---|
| `MAX_EVENTS` | 256 | Maximum events dequeued in a single GetQueuedCompletionStatusEx call.  Higher values risk starving non-I/O workers; lower values reduce throughput. |
| `DEFAULT_TIMEOUT_MS` | 1000 | Default polling timeout in milliseconds. |
| `DEFAULT_TIMEOUT_NS` | 1_000_000_000 | Same value in nanoseconds.  Cross-unit invariant — `DEFAULT_TIMEOUT_NS / 1_000_000 == DEFAULT_TIMEOUT_MS`. |

## §A. Language-level defect — bare-variant first-wins collision across the core archive (LANDED FIX)

**Severity: high. Found 2026-05-29 while authoring `property_test.vr`.**

### Symptom

Every `==` / `assert_eq` / `match` over a **payload-carrying** variant of
`WindowsIoDriverError` returned the wrong answer when the value was
produced by — or compared against — code living in the precompiled core
archive:

* `WindowsIoDriverError.NotFound { path: "a" } == WindowsIoDriverError.NotFound { path: "a" }` → `false`
* `from_error_code(2)` (which builds `NotFound { path: "" }`) failed to
  match a `NotFound { .. }` arm — it fell through to `_`.

Unit (payload-free) variants — `WouldBlock`, `Cancelled`, … — compared
correctly. Only payload variants were affected.

### Root cause

`core/sys/windows/io.vr` wrote its variant **constructors and match-arm
patterns in bare form** (`NotFound { .. }`, `Other { .. }`, …). When an
ADT is loaded from the precompiled core archive, a *bare* variant name
resolves **first-wins** against the *first* sibling-module variant
registered under that name across the whole stdlib. `WindowsIoDriverError`
shares names (`NotFound`, `PermissionDenied`, `TimedOut`, `Other`,
`ConnectionReset`, `BrokenPipe`, `AlreadyExists`, `Interrupted`, …) with
many other error ADTs, so the bare arm bound the *wrong* type's variant
and never fired on a real `WindowsIoDriverError` value → `Eq.eq` fell to
`_ => false`. This is the same class documented in
`core/context/error.vr`'s `Eq` impl comment (tracked as compiler task
#17/#39) and across the 2026-05-28 stdlib-wide bare-variant sweep.

### Fix landed (source-side, the established remediation)

Qualified **every** constructor and match-arm pattern in
`WindowsIoDriverError`'s `from_error_code` / `message` / `is_retryable` /
`Display` / `Debug` / `Eq` impls to `WindowsIoDriverError.<Variant>`
form (`core/sys/windows/io.vr`). The sibling Windows error ADTs got the
same treatment in the same branch: `WindowsTlsError`
(`core/sys/windows/tls.vr`) and `WindowsThreadError`
(`core/sys/windows/thread.vr`).

Because the core stdlib is **embedded in the `verum` binary at
`cargo build` time** (`crates/verum_compiler/build.rs` →
`stdlib_archive.zst`), these source edits require a
`cargo build --release --bin verum` rebuild to take effect; the per-test
`--interp` compile links against the embedded archive, not live `core/`.

### Deep fix (deferred to the compiler)

The fundamental fix is type-directed resolution of bare variant names in
match arms / constructors when the enclosing expression's type is known
(so the bare form resolves to the correct ADT instead of first-wins).
This is multi-day VBC codegen work (compiler task #17/#39) and is tracked
in `website:docs/stdlib/defect-class-catalogue.md`. Until then,
qualified-form discipline is mandatory for every payload-carrying ADT
whose variant names are not globally unique.

## 3. Action items landed in this branch

1. `unit_test.vr` — 7 `@test`s pinning the timeout/cap constants and
   `WindowsIoToken` newtype round-trip (pre-existing).
2. **`property_test.vr` (NEW)** — 12 `@test`s: constant scaling, token
   injectivity, `from_error_code` determinism + totality, `is_retryable`
   partition, `Eq` reflexivity + payload-sensitivity.
3. **`integration_test.vr` (NEW)** — 7 `@test`s: error classification
   through `List`, retryable counting, error-in-`Maybe`, tokens as `Map`
   values, `MAX_EVENTS` batch clamp.
4. **`regression_test.vr` (NEW)** — 7 LOCK-IN pins: dual-code unification
   (WSA vs Win32 for ConnectionRefused / TimedOut), pending→WouldBlock,
   unknown-code preservation, token high-bit preservation.
5. **Source fix** — §A qualified-form remediation in `io.vr` (+ sibling
   `tls.vr` / `thread.vr`).

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `create_io_driver` / `is_iocp_available` round-trip | Requires Windows host. |
| 2 | `async_read` / `async_write` completion semantics | Requires IOCP + Overlapped. |
| 3 | `IocpDriver.poll` event-loop round-trip | Requires Windows host. |
| 4 | `WindowsIoOpKind` ADT variants enumeration | Deferred until property runner. |
| 5 | `WindowsIoCqe` field-shape round-trip | Pinned at the Verum type-shape level for now. |
