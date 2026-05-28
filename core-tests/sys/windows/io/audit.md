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

## 3. Action items landed in this branch

1. `unit_test.vr` — 7 `@test`s pinning:
   * the three timeout / cap constants + the cross-unit equivalence
     between `DEFAULT_TIMEOUT_MS` and `DEFAULT_TIMEOUT_NS`;
   * `WindowsIoToken` newtype round-trip including the
     `WindowsIoToken(0)` / `WindowsIoToken(0xFFFFFFFFFFFFFFFF)`
     sentinels.

## 4. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 1 | `create_io_driver` / `is_iocp_available` round-trip | Requires Windows host. |
| 2 | `async_read` / `async_write` completion semantics | Requires IOCP + Overlapped. |
| 3 | `IocpDriver.poll` event-loop round-trip | Requires Windows host. |
| 4 | `WindowsIoOpKind` ADT variants enumeration | Deferred until property runner. |
| 5 | `WindowsIoCqe` field-shape round-trip | Pinned at the Verum type-shape level for now. |
