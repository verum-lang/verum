# `configuration/error` audit

Module: `core/configuration/error.vr` (317 LOC) — universal error
taxonomy for all `core/configuration/*` format parsers. Defines:
* `FormatId` — refined `Text { ^[a-z][a-z0-9_-]{0,31}$ }`
* `ConfigError` — record with format_id/kind/line/column/byte_offset/
  message/path
* `ConfigErrorKind` — 32-variant tag (syntactic / structural /
  resource / semantic / conversion / IO)
* `ConfigErrorCategory` — 5-level (EcClient/EcUpstream/EcResource/
  EcConversion/EcInternal) aligned with `core.database.common.error`
* 12 canonical `format_id_*` factories

Tests: `unit_test.vr` — 41 unit tests covering 12 FormatId factories,
9 ConfigErrorKind variants + 5 categories, category() routing for
10 representative kinds, fixed-typo regression pins for the
3 corrected variant names (UnexpectedChar/InvalidNumber/IoOther),
config_err + with_path constructors.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.configuration.toml` | reports parse failures via `config_err(format_id_toml(), kind, ...)`. |
| `core.configuration.format` | format registry routes errors through ConfigError. |
| `core.configuration.convert` | LossyConversion / UnsupportedFeature for cross-format conversion. |
| Application config loaders | `Result<ConfigValue, ConfigError>` at every parse site. |

## 2. Crate-side hardcodes

`crates/verum_diagnostics/src/codes.rs` likely defines parser error
codes that should align with `ConfigErrorKind` variant names. Drift
detection: when the parser reports `ConfigErrorKind.UnexpectedChar`,
the diagnostic code must match `config_error_kind_name()` output.

## 3. Language-implementation gaps

### §3.1 Fixed in this branch — three typos in `config_error_kind_name`

Pre-fix variant strings: `"UnexpectedCha"` / `"InvalidNumbe"` /
`"IoOthe"`. Fixed in commit 793269375. Pinned by 3 regression
tests in unit_test.vr Section 5.

### §3.2 `FormatId` refinement not unit-tested

`FormatId is Text { self.matches("^[a-z][a-z0-9_-]{0,31}$") }`
refinement — invalid IDs (UPPERCASE, leading digit, special
chars, >32 chars) should fail at construction. Test surface for
refinement violations needs `@expected-runtime-panic` fixture —
deferred.

### §3.3 No `with_*` builder methods beyond `with_path`

Builder pattern could extend to `with_line(self, line: Int)`,
`with_column(self, col: Int)`, `with_byte_offset(self, off: Int)`.
Today every field requires direct record-literal construction
via `ConfigError { ..err, line: new_line }`. Add the builder
chain following ContextError / PanicReport discipline.

**Effort:** small (~30 min) + 4 tests.

### §3.4 `category()` returns by value — no Ord on ConfigErrorCategory

The categorization is used for retry policy + HTTP status mapping.
A `partial_cmp` on category (`EcUpstream` < `EcResource` for
"how transient is this?") would simplify retry-loop ordering. Add
once the policy spec stabilises.

### §3.5 ConfigErrorCategory variants `Ec*` clash naming convention

Both `core/configuration/error.vr` AND `core/database/common/error.vr`
define `Ec*` variants (EcClient, EcResource, EcUpstream, etc.).
These are SEPARATE types — drift between the two is silent. Pin
the alignment with a cross-module integration test that asserts
identical 5-level enumeration.

**Effort:** small (~30 min).

## Action items landed in this branch

* Fixed 3 typos in `config_error_kind_name` (commit 793269375).
* Qualified all variants in `config_error_kind_name` + `category`
  match arms (task #17/#39 workaround discipline).
* `core-tests/configuration/error/unit_test.vr` — 41 unit tests.
* `core-tests/configuration/error/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add `@expected-runtime-panic` for FormatId refinement violations | this folder | gated on test fixture |
| Add `with_line` / `with_column` / `with_byte_offset` builders | `core/configuration/error.vr` + tests | 30 min |
| Add `Ord` for `ConfigErrorCategory` (transience ordering) | `core/configuration/error.vr` + 1 test | 30 min |
| Cross-module Ec* alignment test (config vs database) | integration_test.vr | 30 min |
| property_test.vr (category determinism, all-32-variants reachable) | this folder | 30 min |
