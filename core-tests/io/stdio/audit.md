# `core.io.stdio` — audit

> Conformance suite for `core/io/stdio.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Factory construction surface
> (stdin / stdout / stderr / locks) is stable. Read/write surface
> (Stdin.read_line, Stdin.read, Stdout.write, Stderr.write) is gated by
> task #io-1 (bare-name method dispatch collision with sys_read/sys_write).
> `print` / `println` / `eprint` / `eprintln` are free functions that
> route through sys_write directly and work cross-platform.

## 1. Cross-stdlib usage

* `core.prelude` re-exports `print` and `println` for top-level use.
* `core.io.engine` — uses stdin / stdout / stderr as basic fd handles
  for IoEvent registration in some examples.
* `core.eval.cli` — the REPL prompt uses `read_line` and `println`.

## 2. Crate-side hardcodes

* STDIN_FD=0, STDOUT_FD=1, STDERR_FD=2 are pinned by POSIX — no
  Verum-side drift surface.
* `sys.linux.syscall.{read,write}` and `sys.darwin.libsystem.{safe_read,safe_write}`
  are aliased to `read` / `write` at the global function level, which
  is the root cause of the #io-1 collision class.

## 3. Language-implementation gaps

### §A — `Stdin.read_line` / `Stdin.read` gated by #io-1

The body calls `sys_read(fd, &mut bytes)` which collides with the
bare-name `read` of Reader-impl types. Same defect class as `core.io.protocols`
audit §A.

### §B — `Stdin.lock` returns a NEW StdinLock — not actually locking

Looking at the source: `Stdin.lock(&self)` returns a fresh `StdinLock`
with an empty buffer. There's no actual locking — just a borrow-marker
shape. This is documented at the function-body level; tests confirm
the construction path works but don't verify mutual exclusion (Verum
doesn't have a process-wide lock primitive for stdin yet).

**Tracking task #io-7** (deferred): implement actual exclusive locking
for `Stdin.lock` / `Stdout.lock` / `Stderr.lock` via a global Mutex.

## 4. Action items landed

* **Created** `core-tests/io/stdio/` with `unit_test.vr` (factory
  construction), `property_test.vr` (factory idempotence), `integration_test.vr`
  (factory + lock composition), `regression_test.vr` (live print/println
  no-panic guards + documented gates).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function | 3-5 days |
| #io-7 | Real exclusive locking for Stdin/Stdout/Stderr | 1-2 days |
