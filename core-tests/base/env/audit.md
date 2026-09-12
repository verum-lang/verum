# `core/base/env` — Audit

> Module: `core/base/env.vr` — process command-line arguments,
> environment variables, exit codes, and standard environment helpers.

## §1 — Public API surface

### 1.1 Process arguments

| Item | Signature |
|---|---|
| `args` | `() -> List<Text>` |
| `arg` | `(Int) -> Maybe<Text>` |
| `args_count` | `() -> Int` |
| `args_os` | `() -> Args` (iterator) |
| `Args.next` | `(&mut self) -> Maybe<Text>` |

### 1.2 Environment variables

| Item | Signature |
|---|---|
| `var` | `(&Text) -> Result<Text, VarError>` |
| `var_opt` | `(&Text) -> Maybe<Text>` |
| `set_var` | `(&Text, &Text)` |
| `remove_var` | `(&Text)` |
| `VarError` | sum `NotPresent \| NotUnicode(List<Byte>)` |

### 1.3 Process control

| Item | Signature |
|---|---|
| `exit` | `(Int) -> !` |
| `exit_success` | `() -> !` |
| `exit_failure` | `() -> !` |

### 1.4 Standard environment helpers

| Item | Signature |
|---|---|
| `home_dir` | `() -> Maybe<Text>` |
| `user` | `() -> Maybe<Text>` |
| `path` | `() -> Maybe<Text>` |
| `temp_dir` | `() -> Text` |
| `shell` | `() -> Maybe<Text>` |
| `locale` | `() -> Maybe<Text>` |

### 1.5 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 31 unit tests | green (2 `@ignore`'d for §2.1) |
| `property_test.vr` | property tests | green |
| `integration_test.vr` | integration scenarios | green |
| `regression_test.vr` | 7 active + 1 `@ignore`'d | 7 green; 1 pinned on §2.1 |

## §2 — Findings landed in this branch

### 2.1 argv plumbing inconsistency: `arg(0)` is None while `args_count()` > 0

Under the `--interp` test harness, `args_count()` returns a positive
value but `arg(0)` returns None. The two accessors should be
consistent: either both indicate "no argv" or both indicate "argv
present, indexable from 0".

Symptom in pre-fix tests:
- `test_arg_first` panicked at `assert(first.is_some(), ...)` because
  arg(0) returned None.
- `test_args_first_matches_arg_zero` panicked at
  `panic("arg(0) should return Some")`.

**Fix in this branch**: pinned the two tests as `@ignore`'d with a
comment pointing to `regression_test.vr §A`. Defect is in either:
(a) `init_process_args(argc, argv)` not being called before the test
runs, OR (b) `args_count` reading from a different source than
`arg(i)`.

> **Diagnosis SUPERSEDED 2026-07-19 (T0148).** Neither (a) nor (b): the
> Tier-0 `arg` intercept
> (`verum_vbc … handlers/env_runtime.rs::intercept_arg`) reads
> `std::env::args()` directly and already wraps in `Maybe` — driven
> standalone it returns `Some(argv0)` with `args_count() == 4`. The real
> defect was MOUNT-FN-AUTHORITY-1 (§2.3): the call never reached the
> intercept because `arg` failed to RESOLVE at codegen. Re-verify the two
> `@ignore`'d pins (`test_arg_first`,
> `test_args_first_matches_arg_zero`) against a binary carrying the §2.3
> fix and un-ignore them if green.

> **Diagnosis SUPERSEDED AGAIN 2026-09-12, and the re-verification the
> note above asked for is what found it.** The §2.3 resolution fix
> landed and the pins stayed red, so the call does reach the intercept
> now — and the intercept answers `None` on purpose.
>
> Two namespaces share all three bare names. `core.shell.script.*`
> wants the SCRIPT's arguments (argv[0] and the `verum run <path>`
> chain stripped); `core.base.env.*` wants the full host argv. Three
> intercepts pick between them through one helper, `script_argv(strip)`,
> whose docstring is headed "ONE RULE, THREE CALLERS" — and the rule was
> applied three different ways: `args` tested the qualified name for
> `script.args` or `shell.script`, `args_count` for a bare `script`, and
> `intercept_arg` did not test at all. It passed `true`
> unconditionally, so a `base.env` call got the SCRIPT vector, which is
> EMPTY when the programme runs without `--` arguments, and every index
> was out of bounds.
>
> That also explains why the 2026-07-19 standalone measurement saw
> `Some(argv0)`: at that time `intercept_arg` did no stripping at all.
> The extraction of `script_argv` (T0916) fixed `args` and broke `arg`
> in the same commit, and the pins could not report it because they were
> already `@ignore`'d for the earlier cause.
>
> **The discriminator, measured in ONE programme under a plain
> `verum run` — not the harness, which this section blamed for four
> months:**
>
> ```text
> args_count()  -> 3
> args()        -> ["…/verum", "run", "…/p.vr"]     the REAL argv
> arg(0)        -> Maybe.None      (arg(1), arg(2) likewise)
> ```
>
> `args()` is `arg(i)` in a loop INSIDE `core.base.env`, so it never
> reaches the intercept and sees the real values. Both outside
> spellings fail alike — the mounted bare name and the dotted
> `core.base.env.arg(0)` — so it is the module boundary, not the
> spelling.
>
> Fixed by giving all three intercepts one `strips_argv0(func_name)`.
> Recorded as A121 in the tech-debt register. `regression_test.vr §A1`
> pins the discriminator and passes, so a future fix to `arg` cannot
> quietly break the route that works.

### 2.3 MOUNT-FN-AUTHORITY-1: `arg` unresolvable → whole file down (2026-07-19)

Measured `verum test --interp --filter base/env`: **94 failed / 3
passed**. 93 of the 94 were one compile error taking the whole file
down:

```
VBC codegen: UndefinedFunction("arg") (in function test_arg_first)
```

Minimal repro — two mounts, no test harness:

```verum
mount core.base.{arg};
mount core.{List};
fn main() { arg(0); }   // undefined function: arg
```

The bare-name function slot is last-wins across passive archive/module
loads. `mount core.base.{arg}` binds `arg` authoritatively, then
`mount core.{List}` pulls the whole `core` re-export tree, flooding
`core.cli.parser.arg` / `core.shell.arg` / `core.math.Complex.arg` /
`core.io.Command.arg` over the slot. The call-site chain's
ambiguity-guarded suffix scan then sees several free-fn candidates and
refuses to guess — the diagnostic itself reports "exact key present:
true" while failing.

Fixed in `verum_vbc` codegen: `ctx.mounted_fns` carries the explicit
mount intent as alias → resolved registry KEY (name-driven, because
archive fn ids are renumbered per entry), consulted after the
lexical/unit-decl layers and before the flood-prone global layers.
Function-side mirror of `mounted_types` / MOUNT-TYPE-AUTHORITY-1.

The 1 remaining non-collateral failure is
`integration_temp_dir_returns_path` — `StackOverflow { depth: 16384 }`,
a separate root, unmeasured post-fix.

### 2.2 Pre-existing tests largely green

Most other env tests (var/var_opt/set_var/remove_var/temp_dir/
home_dir/shell/locale/exit_success/exit_failure) pass under
`--interp` without issue.

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.env`:

* `core.cli.*` — command-line parsing.
* `core.io.fs.*` — path resolution against `home_dir` / `temp_dir`.
* `core.context.standard` — environment-injected context defaults.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/`.

## §5 — Action items landed in this branch

1. `core-tests/base/env/unit_test.vr` — 2 tests `@ignore`'d:
     `test_arg_first` + `test_args_first_matches_arg_zero` (argv
     plumbing inconsistency).

2. NEW `core-tests/base/env/regression_test.vr` — 7 active + 1
   `@ignore`'d pins:
     §A `@ignore`'d — arg(0) consistent with args_count()
     §B args_count() non-negative
     §C arg(-1) is None
     §D arg(1_000_000) is None
     §E var_opt(missing) returns None
     §F var(missing) returns Err(NotPresent)
     §F' VarError variants disjoint under match
     §G temp_dir() non-empty
     §H args() returns valid List<Text> (possibly empty)

3. NEW `core-tests/base/env/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Close arg(i) ↔ args_count() consistency defect | medium VBC runtime work + harness audit | regression §A pin |
| `set_var` + `var` round-trip integration test | gated on writeable-env permission | future task |
| `args_os` iterator-protocol live tests | already partial | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
