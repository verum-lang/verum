# `core.io.path` — audit

> Conformance suite for `core/io/path.vr`.
> Snapshot: 2026-05-24.
> Tier 0 (`--interp`) status: **partial** — Path immutable surface is
> stable (construction / parent / file_name / file_stem / extension /
> Eq / Clone / starts_with / ends_with / to_text / to_path_buf). PathBuf
> mutable surface (push/pop/set_extension) is gated by a Text-equality
> drift defect class (see §A below).

## 1. Cross-stdlib usage of `core.io.path`

* `core.io.fs.{metadata, exists, is_file, is_dir, walk_dir, …}` —
  every fs entry point takes `&Path` and routes the inner Text into
  the syscall string buffer (`core/sys/{linux,darwin}/syscall.vr`).
* `core.io.file.{File.open, File.create, File.read, write, ...}` —
  same; absolute-path acceptance is the load-bearing invariant.
* `core.io.process.Command` — argv[0] is a Path; PATH search uses
  `is_absolute` to decide whether to consult the PATH env var.
* `core.cli.app.Args` — captures the program path as PathBuf.
* `core.term.config` — config-file lookup paths are built via
  `Path.join_str` then probed via `Path.exists`.

## 2. Crate-side hardcodes

* `crates/verum_compiler/src/session.rs` hardcodes the convention
  `<project>/Verum.toml` (capital-V) — `cd <project>` is required
  for `verum test` to find the project root. No `core.io.path`
  drift surface here.
* `crates/verum_cli/src/commands/test.rs::test_source_dirs` walks
  `tests/` and `core-tests/` siblings; uses `std::path::Path` not
  the Verum `Path` type, so no cross-tier drift.

No protobuf/Display/Debug auto-derivers hardcode `Component` variant
ordering. The 5-variant `Component` enum is exhaustively pattern-matched
only inside `path.vr` itself.

## 3. Language-implementation gaps

### §A — `PathBuf.push` Text-equality drift on relative-component push

**Symptom:**

```verum
let mut buf = PathBuf.from(&Text.from("/home/user"));
buf.push(&Path.new(&Text.from("docs")));
assert_eq(buf.to_text(), Text.from("/home/user/docs"));  // FAILS
```

The `buf.len()` after push is correctly 15, and `buf.to_text().as_bytes()`
byte-by-byte matches `Text.from("/home/user/docs").as_bytes()`. But
direct `Text.eq` fails. The drift is in some Text invariant that the
push-built form doesn't share with the literal-built form.

**Pre-fix observation:** Before the surgical fix to `core/io/path.vr::push`
(landed in this branch — see §3 below), `buf.len()` after push was 28
instead of 15. The pre-fix body wrote
`self.path.inner.push_str(&component.inner)` where `&component.inner`
mis-read the inner field due to a nested-struct-field-access codegen
issue. The clone() workaround fixes the length but leaves the Text-
equality drift.

**Affected APIs:**
* `PathBuf.push(&Path)`
* `PathBuf.push_str(&Text)` (delegates to push)
* `PathBuf.pop()` (sets inner via `parent()` extraction; symmetric class)
* `PathBuf.set_file_name(&Text)` (pop + push)
* `PathBuf.set_extension(&Text)` (pop + push)
* `Path.join(&Path)` / `Path.join_str(&Text)` (to_path_buf + push)
* `Path.with_extension(&Text)` / `Path.with_file_name(&Text)` (to_path_buf + set_extension/set_file_name)
* `normalize(&Path)` (re-builds via push)

**Root cause hypothesis:**

The Verum `Text` type carries an internal invariant (likely the small-
string-inline state, hash cache, or null terminator placement) that
isn't preserved by the push/push_str chain when invoked via nested
struct field access. The `Text` type's `eq` impl checks more than the
visible bytes.

Cross-ref: same root as the `BTreeMap pattern-match-ref-generic` class
documented in `[[btree_pattern_match_ref_generic_class]]` — nested
struct field access through GetF/SetF in VBC codegen loses an invariant.

**Tactical fix landed in this branch:**

`core/io/path.vr::push` now extracts `component.inner.clone()` into a
local `inner_text` variable BEFORE the push/push_str chain, sidestepping
the deepest mis-read. Pre-fix length 28 → post-fix length 15. But the
Text-equality drift remains.

**Tracking task #io-4:** see "Action items deferred" below.

### §B — `Text.ends_with_char` and `starts_with_char` were missing from stdlib

**CLOSED in this branch** by adding both methods to `core/text/text.vr`.
Before the close, `PathBuf.push` panicked at the first call with:

```
method 'Text.ends_with_char' not found on receiver of runtime kind Object.
This typically indicates one of three architectural gaps: …
No registered function ends with the bare method name.
```

The doc-comment on `PathBuf.push` referenced `ends_with_char` so the call
site was intentional — the method simply hadn't been added to `core.text.Text`.
The fix is straightforward: pattern-mirror `ends_with(&Text)` against the
UTF-8 encoding of `Char`, avoiding the temporary `Text.from(ch)` allocation.

Pinned by `regression_test.vr::regression_text_ends_with_char_*` (live).

### §C — `Path.file_name()` on trailing-separator paths

The doc-comment at `core/io/path.vr:354` promises
`Path.new("/home/user/").file_name() == Some("user")`. Untested under
this branch's snapshot — the path-trim logic uses `trim_end_matches(is_separator)`
which may have subtle char-vs-byte hazards on multi-byte separator codepoints.
Pinned `@ignore` in `regression_test.vr::regression_path_file_name_with_trailing_separator`.

### §D — `normalize(&Path)` gated by §A

`normalize` rebuilds the result path via repeated `push` of normalised
components; same §A defect class applies. Pinned `@ignore` in
`regression_test.vr::regression_normalize_*`.

## 4. Action items landed in this branch

* **Created** `core-tests/io/path/` with `unit_test.vr` (40+ tests,
  Path immutable surface + PathBuf structural-only tests),
  `property_test.vr` (10 algebraic-law sweeps), `integration_test.vr`
  (cross-stdlib composition), `regression_test.vr` (12 pins, 5 live +
  7 `@ignore`'d), `audit.md` (this file).
* **Pinned the Path immutable surface** — construction, is_empty,
  is_absolute, is_relative, has_root, parent, file_name, file_stem,
  extension, starts_with, ends_with, strip_prefix, as_str, to_text,
  to_path_buf, clone, Eq.
* **Pinned the Component 5-variant surface** — construction, is-pattern,
  Eq (RootDir / CurDir / ParentDir), Normal(text) variant payload Eq.
* **Pinned the PathBuf construction surface** — new, with_capacity,
  from, default, clear, is_empty, len, clone, Eq, to_text.
* **Closed §B** — added `ends_with_char` and `starts_with_char` to
  `core/text/text.vr` (5 live regression tests pin the methods).
* **Surgical fix in `core/io/path.vr::push`** — extract `component.inner.clone()`
  to a local before the push/push_str chain, sidestepping the nested-field
  mis-read that pre-fix produced 28-byte garbage instead of 15-byte correct.

## 5. Action items deferred

| Task | Title | Scope | Estimate |
|---|---|---|---|
| #io-4 | `PathBuf.push` Text-equality drift via nested struct field push | `core/io/path.vr` partial workaround applied; deeper fix in `crates/verum_vbc/src/codegen/` for nested struct field-access GetF/SetF semantics on Text receiver. Same root as `[[btree_pattern_match_ref_generic_class]]`. | 2-3 days (multi-day VBC codegen) |
| #io-5 | `Path.file_name()` trailing-separator edge case | `core/io/path.vr::file_name` — verify char-vs-byte hazards in `trim_end_matches(is_separator)` and the surrounding `char_indices()` loop | 0.5 day |
| #io-6 | `normalize(&Path)` post-fix re-validation | Gated by #io-4 close — once push works correctly, validate the 3 `@ignore`'d normalize tests flip green | 1 hour after #io-4 |
