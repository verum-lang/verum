# `core.io.process` — audit

> Conformance suite for `core/io/process.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Stdio 3-variant, ExitStatus
> 4-method surface (success / is_exited / is_signaled / code / signal),
> Command fluent builder are stable. Live process spawn (Command.spawn,
> Command.output, Command.status, Child.wait/signal/kill, Child.write_stdin)
> is gated by task #io-1 + the cross-platform spawn harness.

## 1. Cross-stdlib usage

* `core.shell.run` — uses `Command.output` as the canonical sync-spawn API.
* `core.cli.app` — `Child` for daemon-spawning + `Child.kill` for cleanup.
* `core.cog.builder` — invokes the build pipeline via spawned `verum_compiler`.
* `core.test.runner` — `Command.spawn` for parallel test child processes.

## 2. Crate-side hardcodes

* POSIX waitpid status encoding pinned at `process.vr:85-145` with
  `@cfg(unix)`/`@cfg(windows)` branches. The shift constants
  (low 7 bits = signal, bits 7-14 = exit code) are stable POSIX —
  no Verum-side drift.
* `crates/verum_vbc/src/intrinsics/registry.rs` — `__proc_spawn`,
  `__proc_waitpid`, `__proc_kill` bridge to libc/libSystem.

## 3. Language-implementation gaps

### §A — Live spawn gated by #io-1 + harness

Tests in `regression_test.vr` exercise the spawn surface; gated until
both task #io-1 closes (so the wait/read/write syscalls route through
proper method dispatch) AND a cross-platform spawn harness lands (so
tests can spawn `true` / `false` / `echo` cross-platform).

### §B — Output uses Text not List<Byte>

`Output.stdout() -> Text` and `Output.stderr() -> Text` force UTF-8
decoding eagerly. Consumers wanting raw bytes (e.g., binary protocol
output) can't get them through this API.

**Tracking task #io-12**: expose `Output.stdout_bytes() -> &[Byte]`
and `Output.stderr_bytes() -> &[Byte]` alongside the Text accessors.

### §C — `Result<T, Text>` instead of `IoResult<T>` for spawn errors

`Command.spawn` / `.output` / `.status` return `Result<_, Text>` —
they drop the structured IoErrorKind information that the rest of
core.io carries via `IoResult<T> = Result<T, StreamError>`. This
makes error-class discrimination at the caller awkward
(can't `if e.kind() == NotFound`).

**Tracking task #io-13**: migrate `Command` return types to `IoResult<T>`.

### §D — Windows status decode is partial

`ExitStatus.code()` on Windows returns `self.raw & 0xFFFF` — only
the low 16 bits. The actual Windows exit code is 32 bits (DWORD).
For exits with codes > 65535, the value is truncated.

**Tracking task #io-14**: full 32-bit Windows exit code in ExitStatus.

## 4. Action items landed

* Created `core-tests/io/process/` with `unit_test.vr` (Stdio 3-variant
  + ExitStatus per-encoded-raw value + Command fluent builder + Output
  accessor types), `property_test.vr` (Stdio pairwise distinct, exit
  status classification laws), `integration_test.vr` (Stdio list match,
  full builder chain, ExitStatus classifier), `regression_test.vr` (4
  `@ignore`'d live-spawn pins).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function | 3-5 days |
| #io-8 | Temp-dir / fixture harness | 1 day |
| #io-12 | Output.stdout_bytes / stderr_bytes raw accessors | 0.5 day |
| #io-13 | Migrate Command.{spawn,output,status} to IoResult<T> | 1 day |
| #io-14 | Full 32-bit Windows exit code in ExitStatus | 0.5 day |
