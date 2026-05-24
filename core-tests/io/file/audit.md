# `core.io.file` — audit

> Conformance suite for `core/io/file.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Open-flag constants and
> OpenOptions builder surface are stable. File I/O (open/read/write/
> seek/close) is gated by task #io-1 (sys_read/sys_write collision) AND
> the need for a temp-dir / fixture harness.

## 1. Cross-stdlib usage

* `core.io.fs.{metadata, exists, read_dir, walk_dir, copy, rename, remove_file}` —
  these stat the file but don't open it as a stream; uses `File` only
  indirectly via `File.size()`.
* `core.io.process.Command` — uses `File.create` to redirect child stdout/stderr.
* `core.io.engine.IoEngine` — wraps File's raw fd for async I/O registration.
* `core.protobuf` / `core.encoding.*` — file-based serialisation uses
  `read_to_string` and `write` convenience functions.

## 2. Crate-side hardcodes

* POSIX open-flag values (O_RDONLY=0, O_WRONLY=1, O_RDWR=2, etc.) are
  hardcoded in `core/io/file.vr:104-116`. These match Linux/macOS;
  Windows uses different values and is currently un-validated.
* `crates/verum_vbc/src/intrinsics/registry.rs` does NOT special-case
  any File-specific intrinsic — all file I/O routes through the
  `sys_read` / `sys_write` per-platform aliases.

## 3. Language-implementation gaps

### §A — File methods gated by #io-1

`File.read(...)`, `File.write(...)`, `File.seek(...)`, `File.flush(...)`
all hit the bare-name shadow collision class (same as `core.io.protocols`
audit §A).

### §B — Need for temp-dir / fixture harness in core-tests

Validating actual file I/O behaviour requires a temp dir that's cleaned
up after each test. `core.io.fs.temp_dir()` exists but the harness
integration (auto-cleanup, parallel safety) isn't wired in yet.

**Tracking task #io-8** (deferred): add a temp-fixture utility to
`core-tests/` that gives every test a clean unique temp dir.

### §C — Windows open-flag values diverge from POSIX

`O_BINARY` etc. don't appear in this module. The cross-platform
behaviour on Windows (CRLF translation via `O_TEXT`) is unspecified.
**Tracking task #io-9** (deferred): document Windows open-flag mapping.

## 4. Action items landed

* Created `core-tests/io/file/` with structural tests for open-flag
  constants (POSIX-stable values pinned), OpenOptions fluent builder,
  and File.options factory.
* Pinned 4 `@ignore`'d regression tests for File.open / .create /
  read / read_to_string that flip green when both #io-1 closes AND the
  temp-fixture harness lands.

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function | 3-5 days |
| #io-8 | Temp-dir / fixture harness in core-tests | 1 day |
| #io-9 | Document Windows open-flag mapping | 0.5 day |
