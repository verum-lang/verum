# `core.io.mod` — audit

> Conformance suite for `core/io/mod.vr` re-exports.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — re-export reachability for
> IoErrorKind / StreamError / IoError alias / SeekFrom / IoResult / print
> functions verified live. The `core.io.prelude` sub-module's full
> closure (Read/Write/BufRead/AsyncRead/AsyncWrite/File/BufReader/BufWriter/
> Path/PathBuf) is reachable but trying to exercise their methods hits
> the #io-1 collision.

## 1. Cross-stdlib usage

* `core.prelude` re-mounts `core.io.print` and `core.io.println`.
* Most stdlib modules mount `core.io.*` for `IoResult` + `IoError`
  + `IoErrorKind` access.

## 2. Crate-side hardcodes

* `crates/verum_compiler/src/session.rs` — the prelude-seed list
  includes `io.IoError`, `io.IoResult`, `io.print`, `io.println`,
  `io.eprintln`. Drift surface: if the prelude module path changes,
  the seed list must update. Currently stable.

## 3. Language-implementation gaps

### §A — IoError = StreamError alias

The mod file declares `public type IoError is StreamError;` at line 47.
Tests confirm both names refer to the same record:
* `let e: IoError = StreamError.new(...)` accepts via either name.
* Methods called via either name dispatch identically.

This is the canonical pattern for newtype aliases in Verum.

### §B — Re-exports require explicit field-by-field mount

Looking at `core/io/mod.vr:42-238`, each export is an explicit
`public mount .<sub>.<name>` line. Adding a new public type/fn in a
submodule requires a matching line here, otherwise it's reachable
only via the qualified `core.io.<sub>.<name>` path.

This is the canonical pattern across all stdlib aggregate modules;
no defect.

### §C — `core.io.prelude` sub-module's curated subset

Lines 202-238 define a `public module prelude` sub-module that re-exports
a smaller curated subset. Consumers can `mount core.io.prelude.*` to
pull just the high-frequency types. The prelude's content matches the
list in `core.prelude` (the global prelude); the duplication is intentional
for namespace clarity at the io-only consumer site.

## 4. Action items landed

* Created `core-tests/io/mod/` with `unit_test.vr` (IoError alias +
  IoErrorKind/SeekFrom/IoResult reachability), `property_test.vr` (
  type-equality between IoError and StreamError at call sites),
  `integration_test.vr` (IoResult constructed via either name),
  `regression_test.vr` (print/println/eprint/eprintln reachable
  through `core.io`).

## 5. Action items deferred

| Task | Title | Estimate |
|---|---|---|
| #io-1 | Mount-scope-aware lookup_function (covers prelude consumers) | 3-5 days |
