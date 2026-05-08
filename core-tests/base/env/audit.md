# Audit — `core/base/env.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/env.vr` (633 lines) |
| Tests | `core-tests/base/env/` — `unit_test.vr` (922 LOC, migrated), `property_test.vr` (NEW, ~150 LOC, round-trip + last-write + @property), `integration_test.vr` (NEW, ~110 LOC, full cycle, common vars) |
| Hardcodes in `crates/` | OS-layer integration (libSystem on macOS, syscalls on Linux) |

## §1  No-libc invariant

`core/base/env.vr` reaches the OS via:
- macOS — `libSystem.B.dylib` (Apple-required boundary, not libc in
  the glibc/musl sense — per CLAUDE.md "No libc in interpreter or AOT")
- Linux — direct syscalls
- Windows — kernel32.dll + ntdll.dll
- FreeBSD — direct syscalls

Verified by walking `crates/verum_codegen/src/llvm/...` and the
intrinsic registry — no `libc::getenv` or similar calls. `getenv`
goes through `verum.process.env` intrinsic which dispatches to the
platform shim.

## §2  Test isolation

env tests touch global state — the OS environment. **Cross-test
contamination is real.** Mitigations applied in tests:
- Each test uses a unique key prefix (`VERUM_TEST_<section>_<purpose>`)
- Every test cleans up its keys via `remove_var(key)` before exit

This is fragile: if a test panics mid-way, leftover env vars persist.
The `@isolated` attribute proposed in `core-tests/CAPABILITY_GAPS.md
§1.4` would solve this properly via subprocess-per-test isolation.

## §3  Args parsing

`args()` returns the full command-line vector; `args_count()` returns
the same count. They must agree at all times. Property test
`law_args_count_matches_args_len` enforces this.

## §4  VarError variant

`var(key)` returns `Result<Text, VarError>` where `VarError` is an
enum (likely with NotPresent / NotUnicode variants). The variant
layout could drift; not currently pinned in `well_known_types`.

**Action item (deferred):** if `VarError` ever splits into more
variants, follow the `MAYBE_VARIANT_LAYOUT` pattern.

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/env_test.vr` →
       `core-tests/base/env/unit_test.vr` (vtest frontmatter stripped)
- [x]  Add `property_test.vr` covering set/var round-trip,
       remove_var clear, args_count consistency, var-on-unset returns
       None, last-write-wins, plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering home_dir / temp_dir /
       args_count / full set-var-remove cycle / var-vs-var_opt
       agreement
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **Subprocess test isolation** — currently env tests can poison
   each other's namespace if a test panics before cleanup.
   Architecturally fixed by `@isolated` per CAPABILITY_GAPS §1.4.
2. **VarError variant layout pinning** — small, mechanical, when
   the enum surface stabilises.
3. **Locale/shell/path validation** — `path()` returns the PATH
   variable as a list; today we only test it's accessible. Could
   verify it's split correctly on `:` / `;` per platform.
