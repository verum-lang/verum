# `cli/error` audit

Module: `core/cli/error.vr` (~382 LOC) — CLI error types and
POSIX exit-code mapping per sysexits.h + clig.dev guidance.

Tests: 47 unit tests covering ExitCode 18-variant + .code()
POSIX integer mapping (sysexits + signal conventions) +
ExitCode.ok/usage/fail helpers + ParseDiagnostic 5-variant +
ParseErrorKind 13-variant + .to_exit() error-category mapping +
.tag() stable JSON output string.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.cli.parser` | constructs ParseError + ParseDiagnostic. |
| `core.cli.runtime` | calls `process.exit(code.code())` at top level. |
| Application CLI code | matches on ExitCode for diagnostics. |

## 2. Crate-side hardcodes

The 18 exit-code numbers MUST agree with `<sysexits.h>` on
POSIX systems — any drift breaks consumer scripts that grep
on `$?`. Pinned in this branch's tests.

## 3. Language-implementation gaps

### §3.1 Add Display/Debug/Eq for ExitCode

Currently the type is constructed and consumed, but does not
implement the standard protocol surface. Add for printf-style
error reporting.

**Effort:** small (~20 min) — 17 simple-tag + 1 payload variant.

### §3.2 Add unicode_safe(s) for ParseDiagnostic.to_text()

ANSI-art / non-printing chars in flag names can break terminal
output. Sanitise at render time.

**Effort:** moderate (~1h, separate spec discussion).

### §3.3 Property test for to_exit() codomain

∀k: ParseErrorKind. to_exit(k) ∈ {ExitCode.Usage,
ExitCode.DataErr, ExitCode.GenericError, ExitCode.Software}.

## Action items landed in this branch

* `core-tests/cli/error/unit_test.vr` — 47 unit tests over
  ExitCode + ParseDiagnostic + ParseErrorKind.
* `core-tests/cli/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add Display/Debug/Eq for ExitCode | `core/cli/error.vr` + tests | 20 min |
| Add property_test.vr (to_exit codomain) | this folder | 30 min |
| Sister tests for `core.cli.{builder,parser,help,runtime}` | sister folders | 1 week total |
