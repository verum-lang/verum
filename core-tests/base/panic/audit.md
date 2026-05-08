# Audit — `core/base/panic.vr`

> Panic / abort / assert / unreachable / catch_unwind audit. Most of
> this surface compiles to direct intrinsics; tests must use
> `assert_panics` / `catch_unwind` to stay alive across panicking calls.

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/panic.vr` (548 lines) |
| Tests | `core-tests/base/panic/` — `unit_test.vr` (962 LOC, migrated), `property_test.vr` (NEW, ~250 LOC, algebraic + @property + @test_case demos), `integration_test.vr` (NEW, ~150 LOC, catch_unwind cycle, nested asserts, isolation) |
| Hardcodes in `crates/` | 4 sites (panic intrinsic, exit_process syscall path, write_stderr formatting, debug_assert @cfg gate) |

## §1  Hardcoded intrinsics

The panic surface is *intentionally* hardcoded in the runtime: it's
load-bearing for diagnostics and process exit. Trying to dispatch
panic through metadata would deadlock if the metadata loader itself
panics. Hardcodes are the *correct* architecture here.

| Site | Role |
|---|---|
| `crates/verum_codegen/src/llvm/...` | `write_stderr` → libc `write(2, ...)` (POSIX) / `WriteFile` (Win) |
| `crates/verum_codegen/src/llvm/...` | `exit_process(101)` → `_exit(101)` (panic exit code) |
| `crates/verum_vbc/src/intrinsics/registry.rs` | interpreter-side `verum.io.write_stderr` / `verum.process.exit` |
| `crates/verum_vbc/src/intrinsics/registry.rs` | `catch_unwind` — implemented as longjmp-equivalent or VBC frame unwind |

These are platform shims and correct as-is. They fall under the
"no-libc" CLAUDE.md invariant — `panic.vr`'s source comments at line 36-43
acknowledge this and route through `verum_codegen::platform_ir`.

## §2  Cross-stdlib usage

Every fallible API in `core/` ultimately calls `panic` or one of the
`assert_*` family. The surface is the canonical "I've reached an
impossible state" signal. There are no anti-patterns to flag —
panics are deliberate.

The only thing worth noting: `assert_eq` and `assert_ne` use the
`Eq + Debug` bound, so any user type without `Debug` cannot use the
typed asserts. This is a (minor) ergonomics issue: `assert_eq` could
fall back to `assert(left == right, "values differ")` when `Debug` is
unavailable, instead of failing typecheck with "Debug not implemented
for T". Logged in audit §5.

## §3  Panic-payload recovery is opaque

`catch_unwind<T>(f)` returns `Result<T, ?>` where the `Err` payload
type is opaque. Today there's no way to recover the panic *message*
from the catch frame — only that a panic occurred. This makes
`assert_panics(|| { panic("specific message"); })` unable to verify
the message text.

**Action item (deferred):** introduce a structured `PanicInfo`
type that includes the panic message, location, and a backtrace
(stretch). `catch_unwind` returns `Result<T, PanicInfo>`. Test
ergonomics: `assert_panics_with(|| { ... }, "expected message")`.

**Scope:** runtime change to capture message at panic time and route
through unwind frame; ~200 LOC.

## §4  `debug_assert` is `@cfg(debug)`-gated

`panic.vr:276-302` defines `debug_assert` / `debug_assert_eq` /
`debug_assert_ne` as `@cfg(debug)` for debug builds and `@cfg(not(debug))`
for release. The split correctly produces no-ops in release builds.

**Drift surface:** the `@cfg(debug)` predicate must agree with how
`verum build --release` configures the build. We rely on the build
system setting the `debug` cfg flag correctly — verified once at
`crates/verum_compiler/src/options.rs:CompilerOptions::cfg_flags`,
but a unit test that flips the flag and asserts the no-op variant is
selected would catch any future inversion.

**Action item (deferred):** add a `verum_compiler` test that
compiles a tiny module twice (with and without `debug` cfg) and
checks that `debug_assert` is dispatched to the panic body in one and
the no-op body in the other.

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/panic_test.vr` →
       `core-tests/base/panic/unit_test.vr` (vtest frontmatter stripped)
- [x]  Add `property_test.vr` covering assert reflexivity, eq/ne
       discrimination, some/none + ok/err discrimination, between bounds,
       sorted monotonicity, contains membership, catch_unwind round-trip;
       plus @property samples on Int and @test_case truth tables
- [x]  Add `integration_test.vr` covering catch_unwind isolation,
       nested catch frames, assert_panics flow, unreachable / todo /
       unimplemented contract
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **PanicInfo carrier in catch_unwind** — recover panic message
   for richer test assertions. Cross-cutting; runtime + test surface.
2. **assert_eq fallback when Debug is missing** — ergonomics fix in
   `panic.vr` itself; small, can land independently.
3. **debug_assert @cfg validation test** — small Rust-side test in
   `verum_compiler` that pins the cfg-flag dispatch.
4. **Stack-trace capture on panic** — far-deferred; needs DWARF or
   custom unwind tables. Logged but not prioritised.
