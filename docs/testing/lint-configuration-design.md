# `verum lint` configuration — design document

**Scope**: design of the `[lint]` block in `verum.toml` (and the
optional dedicated `.verum/lint.toml`) that drives `verum lint`. Aims
for parity with the strongest configuration surfaces in the industry
— ruff, biome, golangci-lint, scalafix, .clippy.toml — *and* layers
on Verum-specific knobs that no other linter can offer because no
other language has refinement types, a context system, or a CBGR
memory model.

This is the source of truth for the schema. The website page
**[Reference → lint configuration](/docs/reference/lint-configuration)**
is generated from / kept in sync with this document.

## 1. Design principles

The lint config sits inside the existing manifest. It must:

1. **Match Verum's existing manifest idioms.** Sub-tables like
   `[lint.rules.<name>]`, `[lint.profiles.<name>]`, and
   `[lint.modules."path"]` mirror the patterns already used by
   `[verify.profiles.<name>]` and `[verify.modules."path"]`.
2. **Default to "recommended" without ceremony.** A project with no
   `[lint]` block at all gets a curated, batteries-included rule set
   — not the firehose of all 100+ rules nor the empty set. Authors
   add config to change behaviour, not to enable basics.
3. **Inherit, don't copy.** Profiles, presets, and module overrides
   stack: CLI flags > in-source `@allow` > module override > profile
   > extends preset > base `[lint]` > built-in default. Unset = "fall
   through to parent".
4. **Be schema-checked.** Unknown rule names, malformed thresholds,
   and unknown layer names error out at config-load time with a
   "did you mean…" suggestion. No silent ignores.
5. **Do what no Rust linter can.** First-class refinement, context,
   capability, CBGR-tier, and verification policy controls are the
   reason this config is more powerful than `.clippy.toml`.

## 2. Concrete layout

```toml
# Top-level: rule selection and policy
[lint]
extends = "recommended"           # built-in preset: see §4
disabled = []                     # rules to turn off entirely
denied   = ["deprecated-syntax"]  # bumped to error
allowed  = []                     # bumped to allow (silenced, not absent)
warned   = []                     # forced to warning

# Per-rule severity (more granular than enable/disable)
[lint.severity]
unused-import        = "warn"
deprecated-syntax    = "error"
cbgr-hotspot         = "info"
todo-in-code         = "off"      # synonym for adding to `disabled`

# Per-rule configuration (thresholds, allowlists, custom params)
[lint.rules.cbgr-hotspot]
loop-iteration-threshold = 1000   # only flag loops with ≥ N iterations
hot-deref-threshold      = 50     # require ≥ N derefs in body

[lint.rules.large-copy]
size-threshold-bytes     = 256    # flag types ≥ 256 bytes copied by value
exempt-types             = ["UserId", "Hash", "Span"]

[lint.rules.unbounded-channel]
exempt-modules           = ["test.fixtures", "core.runtime.*"]

[lint.rules.todo-in-code]
exempt-tags              = ["TODO(linus)", "TODO(release)"]
require-issue-link       = true   # every TODO must be `TODO(#1234)` or similar

# File-glob include/exclude
include = ["src/**/*.vr", "tests/**/*.vr"]
exclude = ["target/**", "vendor/**", "**/*.generated.vr"]

# Per-file overrides — substring or glob matched against rel path
[lint.per_file_overrides]
"tests/**"          = { allow = ["unused-result", "todo-in-code"] }
"core/intrinsics/*" = { allow = ["deprecated-syntax"], deny = ["unsafe-ref-in-public"] }
"benches/**"        = { allow = ["redundant-clone"] }

# Architecture / layering
[lint.architecture]
strict_layering = true            # error on any cross-layer import
report_metrics  = true            # surface coupling-stats hint

[lint.architecture.layers]
core    = { allow_imports = ["core", "std"] }
domain  = { allow_imports = ["core", "std", "domain"] }
adapter = { allow_imports = ["core", "std", "domain", "adapter"] }
ui      = { allow_imports = ["core", "std", "domain", "adapter", "ui"] }

[lint.architecture.bans]
# explicit bans even within an allowed layer set
"app.ui"      = ["app.persistence"]
"core.crypto" = ["core.testing"]

# Naming conventions
[lint.naming]
fn        = "snake_case"
type      = "PascalCase"
const     = "SCREAMING_SNAKE_CASE"
variant   = "PascalCase"
field     = "snake_case"
module    = "snake_case"
generic   = "PascalCase"          # T, U, Tx (not lowercase)

# Refinement-type policy — Verum-unique
[lint.refinement_policy]
public_api_must_refine_int        = true   # public fns: Int → Int{ … }
public_api_must_refine_text       = false  # opt-in
require_verify_on_refined_fn      = true   # @verify implied by refined params
disallow_redundant_refinements    = true   # `Int{ true }` etc.

# Capability policy — Verum-unique
[lint.capability_policy]
require_cap_for_unsafe   = true            # `unsafe { … }` ⇒ @cap declaration
require_cap_for_ffi      = true
require_cap_for_io       = false
unauthorised_cap_use     = "error"

# Context (using [...]) policy — Verum-unique
[lint.context_policy]
allow_io_in_pure_modules = false           # `using [Database]` outside dom.layer
[lint.context_policy.modules]
"core.*"        = { forbid = ["Database", "Logger", "Clock"] }
"core.math.*"   = { forbid_all = true }
"app.handlers"  = { allow = ["Database", "Logger", "Tracing", "Auth"] }

# CBGR tier budgets — Verum-unique
[lint.cbgr_budgets]
default_check_ns = 15            # the `~15ns` from the spec
[lint.cbgr_budgets.modules]
"app.handlers.*" = { max_check_ns = 30 }   # per-module override
"core.runtime.*" = { max_check_ns = 0  }   # 0 ⇒ must use &checked / &unsafe

# Verification policy
[lint.verification_policy]
public_must_have_verify          = true    # public fn ⇒ @verify(...) required
default_strategy_for_lint        = "fast"  # what verify mode the lint suggests

# Documentation policy
[lint.documentation]
public_must_have_doc             = true
public_must_have_example         = false   # opt-in (heavy lift)
example_must_compile             = true    # @example blocks `verum check`-able

# Style conventions beyond naming
[lint.style]
max_line_length                  = 100
max_fn_lines                     = 80
max_fn_params                    = 5
max_match_arms                   = 12
max_cognitive_complexity         = 15
trailing_whitespace              = "error"

# Severity / output / fix policy
[lint.policy]
auto_fix                         = "safe-only"   # off | safe-only | all | manual
max_issues_per_file              = 50
treat_warnings_as_errors         = false
sort_issues_by                   = "severity"    # severity | path | rule
group_by                         = "rule"        # rule | file | layer
emit_disabled_summary            = true
error_on_unknown_rule            = true          # typo guard

# Output formats
[lint.output]
format                           = "pretty"     # pretty | json | sarif | github-actions | tap
colour                           = "auto"
file                             = ""           # "" = stdout

# Profiles — selected via `verum lint --profile <name>`
[lint.profiles.ci]
extends             = "strict"
auto_fix            = "off"
treat_warnings_as_errors = true
output.format       = "sarif"

[lint.profiles.dev]
extends             = "recommended"
auto_fix            = "safe-only"

[lint.profiles.legacy]
extends             = "relaxed"
include             = ["src/legacy/**/*.vr"]
public_must_have_doc = false

# Custom rules — text-pattern matchers (today)
[[lint.custom]]
name        = "no-unwrap-in-prod"
pattern     = "\\.unwrap\\(\\)"          # PCRE
message     = "use `?` or `expect(\"why\")` instead of unwrap()"
level       = "warn"
paths       = ["src/**"]                  # apply to these
exclude     = ["src/legacy/**"]
suggestion  = "?"                         # auto-fix replacement (optional)

[[lint.custom]]
name        = "no-todo-without-issue"
pattern     = "\\bTODO(?!\\(#\\d+\\))"
message     = "TODO must reference an issue: TODO(#1234)"
level       = "error"
```

## 3. Layered loading & precedence

Effective severity for `(rule, file, item)` is the highest-priority
match in this stack:

```
1. CLI flag                         (-D rule, -W rule, -A rule, -F rule)
2. In-source attribute              (@allow(rule), @deny(rule), @warn(rule))
3. [lint.profiles.<active>]         (selected via --profile)
4. [lint.per_file_overrides]        (substring/glob match against rel path)
5. [lint.severity]                  (per-rule mapping)
6. allowed / denied / warned / disabled lists
7. extends preset                   (recommended | strict | relaxed | minimal)
8. Built-in default                 (LINT_RULES const)
```

Higher rules override lower. Empty / unset entries fall through —
explicit "off" stops the cascade.

In-source attributes:

```verum
@allow(unused-import, reason = "intentionally re-exported")
mount stdlib.fs;

@deny(todo-in-code)
public fn release_critical() { … }

@warn(deprecated-syntax)
fn experiment() { … }
```

`reason = "..."` is required for `@allow` when
`require_allow_reason = true` (default `true` under `strict` preset).

## 4. Built-in presets (`extends`)

Four curated presets, picked by `extends` in `[lint]`:

| Preset | Philosophy | Approx rules |
|--------|-----------|--------------|
| `minimal` | hard errors only — `deprecated-syntax`, `missing-context-decl`, `mutable-capture-in-spawn`. Useful for porting in legacy code. | 3-5 |
| `recommended` | the default. All Safety + Verification rules; Performance and Style at warn/info. | 20-30 |
| `strict` | everything `recommended` has plus refinement-policy, public-api-must-doc, max-fn-lines, naming, layering. CI-grade. | 40-60 |
| `relaxed` | only Style at info; never errors. For aesthetic suggestions in IDE only. | 8-12 |

Switching presets is one line:

```toml
[lint]
extends = "strict"
```

Presets compose with explicit overrides. `extends = "strict"` then
`disabled = ["max-fn-lines"]` keeps strict everywhere except that one
rule.

## 5. Per-rule configuration tables

Each rule that takes parameters declares them under
`[lint.rules.<name>]`. Unknown keys are errors (typo guard).

Schema for the top 10 rules with non-trivial config:

```toml
[lint.rules.cbgr-hotspot]
loop-iteration-threshold = 1000
hot-deref-threshold      = 50
exempt-fn-attribute      = "@hot"     # fns marked @hot are exempt

[lint.rules.large-copy]
size-threshold-bytes     = 256
exempt-types             = []

[lint.rules.max-fn-lines]
soft-limit               = 80
hard-limit               = 200
exempt-fn-attribute      = "@long_fn_ok"

[lint.rules.max-cognitive-complexity]
threshold                = 15
exempt-fn-attribute      = "@complex_ok"

[lint.rules.todo-in-code]
exempt-tags              = []
require-issue-link       = true

[lint.rules.unbounded-channel]
exempt-modules           = []

[lint.rules.shadow-binding]
allow-shadow-of-loop-var = true
allow-mut-shadow         = false

[lint.rules.unused-import]
report-glob-imports      = true       # `mount foo.*` flagged when nothing matches

[lint.rules.missing-error-context]
required-fns             = ["?", "Result.context"]

[lint.rules.public-api-must-have-doc]
require-since-tag        = false
require-example-block    = false
```

## 6. Architecture rules

Layering and ban lists turn module-import constraints into linter
errors before they ship.

```toml
[lint.architecture]
strict_layering = true                # error on any banned import

[lint.architecture.layers]
core    = { allow_imports = ["core", "std"] }
domain  = { allow_imports = ["core", "std", "domain"] }

[lint.architecture.bans]
# direction-aware: "X cannot import from Y"
"app.ui"      = ["app.persistence", "app.network"]
```

Resolution: every `mount X.Y.Z` in a file in module `M` is matched
against the rules of `M`'s layer plus any explicit ban for `M`.
Violations surface as `architecture-violation` lint hits.

`report_metrics = true` adds an info-level entry per module showing
fan-in / fan-out / depth-from-leaves to surface architectural drift
*before* it becomes a banned import.

## 7. Naming conventions

```toml
[lint.naming]
fn        = "snake_case"     # default
type      = "PascalCase"
const     = "SCREAMING_SNAKE_CASE"
variant   = "PascalCase"
field     = "snake_case"
module    = "snake_case"
generic   = "PascalCase"
```

Recognised values: `snake_case`, `kebab-case`, `PascalCase`,
`camelCase`, `SCREAMING_SNAKE_CASE`, `lowercase`, `UPPERCASE`. An
unrecognised value is a config-load error.

Exemptions:

```toml
[lint.naming.exempt]
fn   = ["__init", "drop_impl"]
type = ["I32", "F64"]   # FFI types match foreign convention
```

## 8. Refinement / capability / context / CBGR / verification policy

Verum-unique. These have no `clippy.toml` analogue.

### `[lint.refinement_policy]`

```toml
public_api_must_refine_int        = true
public_api_must_refine_text       = false
require_verify_on_refined_fn      = true
disallow_redundant_refinements    = true   # Int{ true }, Text{ true }
disallow_post_hoc_refinement      = false  # narrow at call site rather than param
```

### `[lint.capability_policy]`

```toml
require_cap_for_unsafe   = true
require_cap_for_ffi      = true
require_cap_for_io       = false           # opt-in policy
unauthorised_cap_use     = "error"         # severity for offending call sites
allowed_caps             = ["fs.read", "fs.write", "net.outbound"]
```

### `[lint.context_policy]`

```toml
allow_io_in_pure_modules = false

[lint.context_policy.modules]
"core.*"        = { forbid = ["Database", "Logger", "Clock"] }
"core.math.*"   = { forbid_all = true }
"app.handlers"  = { allow = ["Database", "Logger"] }
```

### `[lint.cbgr_budgets]`

```toml
default_check_ns = 15

[lint.cbgr_budgets.modules]
"app.handlers.*" = { max_check_ns = 30 }
"core.runtime.*" = { max_check_ns = 0  }   # 0 ⇒ &checked / &unsafe required
```

The check-ns budget is read by the **`cbgr-hotspot`** rule and (when
the profiler ran) compared against actual measured costs from
`target/profile/last.json`. Without profiling data the lint falls
back to a conservative static estimate.

### `[lint.verification_policy]`

```toml
public_must_have_verify       = true
default_strategy_for_lint     = "fast"     # what verify mode it suggests
```

## 9. Profiles & per-environment configs

```toml
[lint.profiles.ci]
extends                     = "strict"
auto_fix                    = "off"
treat_warnings_as_errors    = true
output.format               = "sarif"

[lint.profiles.dev]
extends                     = "recommended"
auto_fix                    = "safe-only"

[lint.profiles.legacy]
extends                     = "relaxed"
include                     = ["src/legacy/**/*.vr"]
public_must_have_doc        = false
```

Selected via `verum lint --profile <name>` or the
`VERUM_LINT_PROFILE` env var.

## 10. In-source attributes

```verum
@allow(unused-import, reason = "needed for derive macro to see it")
mount stdlib.derive.*;

@deny(todo-in-code)
public fn ship_critical() { … }

@warn(deprecated-syntax)
fn experiment() { … }
```

Resolution:

- Attributes nest. `@allow` on a fn covers its body; on a `type`,
  the impl block; on a module, every item until end of file.
- `reason = "..."` becomes a column in `--format json` output so
  reviewers see *why* a lint was silenced.
- `require_allow_reason = true` (default in `strict`) makes a bare
  `@allow` an error.

## 11. Custom rules

Pattern-matching custom rules (today):

```toml
[[lint.custom]]
name        = "no-unwrap-in-prod"
pattern     = "\\.unwrap\\(\\)"
message     = "use `?` or `expect(\"why\")` instead of unwrap()"
level       = "warn"
paths       = ["src/**"]
exclude     = ["src/legacy/**"]
suggestion  = "?"
require-issue-link = false        # rule-local options after `name`
```

Stage 2 will add **AST-pattern** custom rules (match a sub-tree, not
a regex) — strictly a superset of the regex form and backwards-
compatible.

## 12. Validation & error messages

Config errors must be helpful. The loader does, in order:

1. **Schema check** — every key is recognised; unknown keys generate
   `error: unknown lint config key 'auto_ix' — did you mean 'auto_fix'?`
2. **Rule-name validation** — every rule referenced (in
   `disabled`, `severity.<rule>`, etc.) must be a known rule. Typos
   include suggestions: `unbounded-cahnnel` → `unbounded-channel`.
3. **Threshold validation** — `loop-iteration-threshold = -5` is
   rejected.
4. **Cycle-check on profiles** — `extends = "ci"`, `[lint.profiles.ci]
   extends = "strict"`, `[lint.profiles.strict] extends = "ci"` is
   rejected with a cycle trace.
5. **Layer-name validation** — `[lint.architecture.bans] "app.ui" =
   ["app.persistence"]` warns if neither layer is declared.

`verum lint --validate-config` runs only the validator and exits 0
or non-zero — useful in pre-commit and CI.

## 13. CLI surface (additive on top of today's flags)

```bash
verum lint                            # default: report only
verum lint --fix                      # apply auto-fixes (policy-respecting)
verum lint --profile ci               # use [lint.profiles.ci]
verum lint --explain unused-import    # rule documentation + examples
verum lint --list-rules               # all known rule names + categories
verum lint --validate-config          # config-only validation
verum lint --since origin/main        # only files changed vs ref
verum lint --severity error           # report only at this level or higher
verum lint --format sarif > x.sarif
verum lint --format github-actions    # ::warning file=…::msg annotations
```

`-D`, `-W`, `-A`, `-F` from `verum build` continue to work for
single-rule overrides.

## 14. Compatibility & migration

Existing `[lint] disabled = […]`, `[lint] denied = […]`,
`[[lint.custom]]` blocks continue to parse exactly as before. The
new keys (`extends`, `severity`, `rules.<name>`, `profiles.<name>`,
`per_file_overrides`, `architecture`, `naming`,
`refinement_policy`, `capability_policy`, `context_policy`,
`cbgr_budgets`, `verification_policy`, `documentation`, `style`,
`policy`, `output`) are additive — projects on the current schema
keep working until they opt in.

`verum lint --migrate-config` (Stage 2) reads an old-style block and
emits the modernised equivalent.

## 15. References & inspirations

- **`.clippy.toml`** — minimal config, per-rule thresholds. We match
  this and extend with severity-per-rule and profiles.
- **`ruff.toml`** — `select` / `ignore` / `extend-select` / per-file-
  ignores / fixable / target-version. Heavy influence on
  `per_file_overrides` and the preset model.
- **`biome.json`** — JSON-shaped rule config with severity per rule.
  Direct inspiration for `[lint.severity]`.
- **`golangci.yml`** — multi-linter aggregator with global + per-
  linter settings. Influence on `[lint.rules.<name>]`.
- **`.scalafix.conf`** — first-class rule-rewrite system, inspires
  the upcoming AST-custom-rule layer.
- **`.pylintrc`** — by-category disable/enable + per-message control.
  Influence on the precedence stack.
- **Verum's own** `[verify.profiles.<name>]`, `[verify.modules.
  "path"]`, `[linker.<os>]`, `[profile.<active>]` patterns — every
  layout choice in `[lint.*]` mirrors an existing manifest section.
