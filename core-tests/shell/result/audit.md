# `shell/result` audit

Module: `core/shell/result.vr` (~300 LOC) — outcome + error
types for shell command execution. Pure-data; live execution
at `core/shell/exec.vr`.

Tests: 23 unit tests covering ShellError 6-variant +
.command() routing across variants + .to_string() diagnostic
rendering + ShellResult construction + accessors (.code,
.success, .is_empty, .command_text).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.shell.exec` | constructs ShellResult after wait(). |
| `core.shell.executor` | maps ShellError → exit code. |
| Application shell scripts | matches on ShellError variants for retry / fallback. |

## 2. Crate-side hardcodes

`ExitStatus { raw: (code << 8) }` encoding mirrors POSIX
`WEXITSTATUS` macro convention. Pinned via .code() === 0 test
on ExitStatus { raw: 0 } and === 1 on ExitStatus { raw: 256 }.

## 3. Language-implementation gaps

### §3.1 Property tests on .command() invariant

∀err: ShellError. err.command() is one of {valid, ""}; under no
condition should it panic. Generators required.

**Effort:** ~30 min.

### §3.2 Eq for ShellError

Currently no Eq impl. Some consumers want to dedupe errors by
identity. Add Eq with field-by-field equality.

**Effort:** small (~20 min).

### §3.3 ShellLinesIter coverage

`IntoIterator for ShellResult` yields `ShellLinesIter`. Tests
should cover empty iter / single-line / multi-line iter /
trailing-empty-line handling. Currently untested directly.

## Action items landed in this branch

* `core-tests/shell/result/unit_test.vr` — 23 unit tests over
  ShellError + ShellResult.
* `core-tests/shell/result/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test on .command() invariant | this folder | 30 min |
| Add Eq for ShellError | `core/shell/result.vr` + tests | 20 min |
| ShellLinesIter coverage | this folder | 30 min |
| Sister tests for `core.shell.{exec,executor,command,pipeline,jobs}` | sister folders | 1 week total |
