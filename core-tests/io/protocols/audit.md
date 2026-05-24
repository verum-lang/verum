# `core.io.protocols` — audit

> Conformance suite for `core/io/protocols.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — 60+ green over the data-only
> surface (IoErrorKind / StreamError / SeekFrom / IoResult). Method-call
> surface (Read.read / Write.write / Seek.seek and friends) is gated by
> task #17/#39 (mount-scope-aware `lookup_function`).

## 1. Cross-stdlib usage of `core.io.protocols`

`StreamError` / `IoErrorKind` / `IoResult` / `SeekFrom` / Read / Write /
Seek / BufRead are consumed by every other module in `core.io.*`:

| Consumer | Surface used |
|---|---|
| `core.io.file.{File, OpenOptions}` | `IoResult<T>`, `StreamError.from_raw_os_error`, `Read` / `Write` / `Seek` for `File` |
| `core.io.stdio.{Stdin, Stdout, Stderr}` | `IoResult<T>`, `StreamError`, `Read` for `Stdin*`, `Write` for `Stdout*` / `Stderr*` |
| `core.io.fs.{Metadata, DirEntry, ReadDir, WalkDir}` | `IoResult<T>`, `StreamError.from_raw_os_error` for every fs call |
| `core.io.buffer.{BufReader, BufWriter, LineWriter}` | `Read` / `Write` impls wrap `<R: Read>` / `<W: Write>` |
| `core.io.process.{Command, Child, Output}` | `IoResult<T>`, `StreamError` |
| `core.io.path.{Path, PathBuf}` | only `IoResult` via `metadata()` route |
| `core.io.async_protocols.{AsyncRead, AsyncWrite, AsyncBufRead}` | mirrors Read/Write/BufRead with async-marker bodies |
| `core.net.*` socket types | `IoResult<T>`, `StreamError.from_os` |
| `core.shell.stream` | `Read` for `FdReader` (defined in `buffer.vr` but consumed across shell pipes) |

**Observations / drift surfaces:**

1. `StreamError` ⇆ `IoError` alias. The module re-exports
   `IoError = StreamError` at `core/io/mod.vr:47`. Inside `protocols.vr`
   the type is consistently `StreamError`; in `buffer.vr` it's imported as
   `StreamError as IoError`. Both names refer to the same record. Tests
   in this folder use `StreamError` directly to avoid the alias hop, which
   is the same convention `core/io/protocols.vr` uses internally.

2. `Cursor<T>` is **defined twice**:
    * `core/io/protocols.vr:884` defines `Cursor<T>` with Read/Write/Seek
      impls for `&[Byte]`, `List<Byte>` (Seek only here), and
      `&mut List<Byte>` (Write).
    * `core/io/buffer.vr:653` defines `BufferCursor<T>` with a type
      alias `Cursor<T> = BufferCursor<T>` and Read/Write/Seek/BufRead
      impls for `List<Byte>`.

    Both are re-exported into the `core.io.*` surface. Resolution at the
    call site is first-wins (see §3 below). The buffer.vr definition is
    the richer one (has `BufRead` and `List<Byte>::Read`); the
    protocols.vr one has the `&[Byte]` Read + `&mut List<Byte>` Write
    that buffer.vr lacks. Both names cannot coexist in scope without a
    collision.

    **Action item — deferred to a separate task**: decide canonical
    Cursor (likely buffer.vr's BufferCursor with the protocols.vr impls
    folded in), remove the protocols.vr Cursor type, re-export from
    protocols.vr via `mount super.buffer.Cursor`.

## 2. Crate-side hardcodes

Searched `crates/` for sites that hardcode names, tags, or method
signatures of `core.io.protocols` types.

| Site | What is hardcoded | Drift risk |
|---|---|---|
| `crates/verum_vbc/src/codegen/expressions.rs::compile_method_call` | Method-name dispatch via global `lookup_function` — see §3 | **HIGH** — root cause of @ignore'd regression tests |
| `crates/verum_common/src/well_known_types.rs::primitive_protocol_matrix_pinned` | No IO protocol hardcodes here; surface is Eq/Ord/Hash/Clone/Default | low |
| `crates/verum_vbc/src/codegen/calls.rs::handle_call` / `handle_call_m` | Method-id interning, no IO-specific knowledge | low |
| `crates/verum_codegen/src/llvm/` (AOT) | Same dispatch contract as VBC — same #17/#39 root | **HIGH** at AOT tier |

No protobuf/Display/Debug auto-derivers hardcode IoErrorKind variants. The
20-variant list is exhaustively pattern-matched only inside `protocols.vr`
itself (Debug/Display/Eq/Clone impls). External crates use kind() opaquely.

## 3. Language-implementation gaps

### §A — Method dispatch on `Sink.write` / `EmptyReader.read` collides with `sys.linux.syscall.write` / `read`

**Symptom:** `verum test --interp --filter test_sink_accepts_bytes`
fails with:

```
runtime: NullPointerAt { op: "opcode 0x66", site: "sys.linux.syscall.write", pc: 20 }
```

The function being executed at the call site IS `sys.linux.syscall.write`,
**not** `Sink::write`. The codegen has emitted `Call <fn-id>` against the
first-suffix-wins entry in the global function index instead of `CallM`
against the receiver type's method table.

**Affected APIs:**
* `Sink.write(...)`, `Sink.flush(...)`
* `EmptyReader.read(...)`, `EmptyReader.fill_buf(...)`, `EmptyReader.consume(...)`
* `ByteRepeat.read(...)`
* `Cursor<&[Byte]>.read(...)`, `Cursor<&[Byte]>.seek(...)`
* `Cursor<&mut List<Byte>>.write(...)`
* `Cursor<List<Byte>>.seek(...)` (from this module)
* Every method that chains through `self.read(...)` / `self.write(...)`
  in the protocol default-method bodies: `read_exact`, `read_to_end`,
  `read_to_string`, `write_all`, `write_fmt`, `read_until`, `read_line`,
  `lines`, `split`, `bytes`, `chain`, `take`.

**Root cause:** Same class as task #17/#39 — the codegen's
`lookup_function` call sites are not all mount-scope-aware. The
`compile_method_call`'s pre-resolved fast path at
`crates/verum_vbc/src/codegen/expressions.rs::compile_resolved_call_target_with_receiver`
trusts the typechecker's `ResolvedCallTarget::StaticCall { qualified_name }`
annotation; the typechecker can route a protocol-method call to a
bare-name shadow because `core/sys/{linux/syscall,darwin/libsystem}` both
re-export `read` / `write` at the global function level (via `safe_read as
read` / `safe_write as write` in `core/sys/darwin/mod.vr:61-62` and similar
in linux's `syscall.vr`).

**Fundamental fix path:**

1. Migrate `compile_resolved_call_target_with_receiver` to consult the
   type-method table FIRST when a syntactic receiver is present —
   `<Type>.<method>` qualified lookup before falling back to the bare
   `<method>` global lookup.

2. At the typechecker, mark `Sink::write` / `EmptyReader::read` /
   `Cursor::read` resolution as `MethodCall { type_id, method_id }` (a
   shape that already exists in the AST) rather than collapsing to
   `StaticCall { qualified_name }`.

3. As a tactical workaround inside `core/io/protocols.vr` itself: the
   protocol-default-method bodies (`Read.read_exact`, `Read.read_to_end`,
   `Write.write_all`, …) call `self.read(...)` / `self.write(...)` —
   these are unambiguous since `self` is typed, but the codegen still
   misdispatches because the bare-name shadow precedes the type-method
   lookup. The proper fix is at the codegen layer; in-tree workaround
   would be to fully-qualify every internal call as
   `<Self as Read>::read(self, ...)` — but Verum doesn't yet expose
   UFCS at the method-call surface, so the workaround is unavailable.

**Tracking task: #io-1** — see "Action items deferred" below.

### §B — `Cursor<T>` defined twice (protocols.vr + buffer.vr)

See §1 observation 2. Two record types with the same simple name
coexist in `core.io.*`. The protocols.vr Cursor has Read/Write/Seek
impls; the buffer.vr `BufferCursor` (alias `Cursor`) has Read/Write/Seek/BufRead
impls for List<Byte>. Mounting both via `core.io.*` puts BOTH into scope
under the same name. First-wins resolution silently picks one based on
mount order.

**Reproducer**: `regression_test.vr::regression_cursor_alias_collision_between_protocols_and_buffer`
(@ignore'd until §A closes — until then the construction call itself
panics at the read/write/seek follow-up).

**Tracking task: #io-2** — see below.

### §C — `&[Byte]` slice-borrowed Cursor receiver

Even after §A, `Cursor::new(&data[..])` for a stack-array `[Byte; N]`
needs the codegen to handle slice-to-borrowed-Byte-slice coercion at
record construction. The protocols.vr Cursor<&[Byte]> impl exists and
should be reachable, but cross-tier (VBC vs AOT) semantics for the
slice receiver are untested. Once §A closes, validate this path.

**Tracking task: rolled into #io-1.**

### §D — `[u8; N]` array literal vs `Byte` typed buffers

The `let mut buf = [0_u8; 16];` and `let buf: [Byte; 3] = [1, 2, 3];`
shapes both work at construction (verified). What hasn't been
validated is whether `&mut buf[..]` produces a borrowed slice of the
right primitive width to satisfy `&mut [Byte]`. The `u8` ⇆ `Byte`
canonicalisation is supposed to happen at typecheck time
(`verum_common::primitives`), and at first inspection it does — but
once §A closes we need to confirm the actual data motion through
`read(&mut buf)` produces the expected byte values.

**Validation gate**: this is implicitly tested by every read test once
§A closes. No separate task.

### §E — `Eq for StreamError` requires `Eq for Maybe<Text>`

`StreamError` derives Eq via field-wise comparison; the `message: Maybe<Text>`
field forces `Eq for Maybe<T>` where `T: Eq`. `Eq for Maybe<Text>` works,
verified. The pin is implicit in `test_stream_error_eq_with_messages` and
`test_stream_error_neq_different_messages`.

No drift; mention here for the "is it complete" budget.

## 4. Action items landed in this branch

* **Created** `core-tests/io/protocols/` with `unit_test.vr` (60+ tests),
  `property_test.vr` (16 exhaustive sweeps), `integration_test.vr`
  (cross-module composition), `regression_test.vr` (11 `@ignore`'d pins
  + 1 live), `audit.md` (this file).
* **Pinned the IoErrorKind 20-variant surface** exhaustively — Eq /
  Clone / pattern-match-distinct laws hold on every variant.
* **Pinned the SeekFrom 3-variant surface** with exhaustive Eq/Clone
  laws over representative Int payloads.
* **Pinned StreamError construction surface** — `new` / `with_message`
  / `Other` / `from_raw_os_error` / `from_errno` / `from_os` all
  exercised.
* **Pinned the POSIX-stable errno table** — codes {2, 4, 12, 13, 17,
  22, 32} produce the expected `IoErrorKind` on every platform; the
  full fall-through to `Other` is asserted for unrecognised codes
  and for 0 / negatives.
* **Pinned `?` propagation through `IoResult<T>`** — preserves both
  kind and message across function boundaries.

## 5. Action items deferred

| Task | Title | Scope | Estimate |
|---|---|---|---|
| #io-1 | `compile_resolved_call_target_with_receiver` should consult type-method table BEFORE bare-name global function index when a syntactic receiver is present | `crates/verum_vbc/src/codegen/expressions.rs` (~7170-7400), plus typechecker annotation refinement in `verum_types`; AOT side mirror in `crates/verum_codegen/src/llvm/` | 3–5 days; closes all `@ignore`'d regression_test.vr pins, the rest of the io.* method surface across buffer/file/stdio/fs/process, every protocol method on every protocol-implementor whose method name collides with a bare-name shadow |
| #io-2 | Deduplicate `Cursor<T>` between protocols.vr and buffer.vr | `core/io/protocols.vr` (remove the Cursor block, re-export from buffer), `core/io/buffer.vr` (absorb the `&[Byte]` Read + `&mut List<Byte>` Write impls), update `core/io/mod.vr` re-exports | 0.5 day after #io-1 closes |
| #io-3 | Audit every protocol-default-method body in `core.io.protocols` for `self.<method>` chains that might collide post-#io-1 close | `core/io/protocols.vr` proofreading; no code changes likely once #io-1 lands | 1 hour |
