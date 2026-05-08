# Audit — `core/base/result.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/result.vr` (1540 lines) |
| Tests | `core-tests/base/result/` — `unit_test.vr` (1538 LOC, migrated), `try_block_test.vr` (505 LOC, migrated), `try_protocol_test.vr` (311 LOC, migrated), `property_test.vr` (NEW, ~330 LOC, monad/functor/Eq/Ord laws + @property + @test_case), `integration_test.vr` (NEW, ~230 LOC, ?-chains, cross-type, error promotion, sort, fold) |
| Hardcodes in `crates/` | same drift surface as Maybe — variant tags hardcoded; ?-fast-path bypasses Try/FromResidual |

## §1  Variant-tag drift surface — addressed in this branch

`Result.Ok = 0`, `Result.Err = 1` were hardcoded in
`crates/verum_vbc/src/codegen/mod.rs`. Replaced with consultation of
the new canonical layout constant
`verum_common::well_known_types::RESULT_VARIANT_LAYOUT` (landed in
this branch), mirroring the Ordering/Maybe pattern.

The matrix-pinning unit test (`result_variant_layout_pinned` in
`verum_common::well_known_types::tests`) catches drift at test time.

Runtime constructor sites that still embed `0x8000+0` / `0x8000+1`
TypeIds for Result remain (`verum_vbc/src/interpreter/.../{ffi_extended,arith_extended}.rs`).
These should consult the layout constant too, but that is a follow-up
mechanical change — listed in §6.

## §2  ?-operator architecture defect

Same as Maybe (and as documented in `core-tests/base/ops/audit.md §1`):
the `?`-operator does NOT consult `Try.branch()` /
`FromResidual.from_residual()` for Result. Codegen detects the type
by name (`is_result`) and emits direct `IsVar` + branch instructions.

**Implication for tests:** any custom `Try` implementation for an
error-monad-like user type does not work via `?`. Tests that pass
today on Result do so by accident of the codegen fast-path
recognising the well-known-name `Result`.

**Action item (deferred):** see ops audit §1 for the architectural
fix. Cross-cutting; multi-PR.

## §3  From-bound error promotion in ?-chains

This *does* work today — `verum_vbc/src/codegen/expressions.rs` calls
`From.from()` on the inner error type when the surrounding function
returns `Result<T, OuterE>` where `OuterE: From<InnerE>`. Verified
with `integration_test.vr §6`.

The mechanism is ad-hoc — codegen detects the From-bound at compile
time and emits a direct `Call(From.from)` invocation. Once the
?-protocol-dispatch fix lands (§2), this will route through
`FromResidual` properly.

## §4  Cross-stdlib usage

Result is *the* error-handling mechanism in the standard library.
Survey:

- Every fallible API in `core/io/`, `core/net/`, `core/database/` returns
  `Result<T, ErrType>`.
- `StdResult<T>` alias in `mod.vr:361` makes boxed errors uniform.
- `try_alloc`, `try_realloc` allocator primitives return
  `Result<_, AllocError>`.

Idiomatic patterns:
- `?`-propagation across nested calls
- `.map_err(|e| OuterE::from(e))` for explicit conversion
- `match` for happy/sad path discrimination
- `unwrap_or(default)` for fallback

Anti-patterns observed:
- A handful of stdlib free functions still return `Int` with -1/0/1 sign
  convention instead of `Result` (`core/base/semver.vr:327` etc.) —
  documented in `core-tests/base/ordering/audit.md §1.1`.

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/result_test.vr` →
       `core-tests/base/result/unit_test.vr` (vtest frontmatter stripped)
- [x]  Migrate `vcs/specs/core/core/try_block_test.vr` →
       `core-tests/base/result/try_block_test.vr`
- [x]  Migrate `vcs/specs/core/core/try_protocol_test.vr` →
       `core-tests/base/result/try_protocol_test.vr`
- [x]  Add `RESULT_VARIANT_LAYOUT` canonical constant in
       `crates/verum_common/src/well_known_types.rs` plus
       `result_variant_layout_pinned` matrix-pinning test
- [x]  Update `crates/verum_vbc/src/codegen/mod.rs` Result entries to
       consult the layout constant
- [x]  Add `property_test.vr` covering functor / monad / Eq / Ord laws,
       combinators, ok / err projection, ?-fast-path, defaulting,
       plus @property and @test_case demos
- [x]  Add `integration_test.vr` covering Map/Set storage, ?-chains
       across calls, cross-type Maybe/Result, From-bound promotion,
       sort, fold-with-short-circuit
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. **Update runtime constructor sites for Result tags** — the
   `0x8000+0` / `0x8000+1` TypeId references in
   `interpreter/dispatch_table/handlers/{ffi_extended,arith_extended}.rs`
   should consult `RESULT_VARIANT_LAYOUT` for consistency. Mechanical;
   ~30 LOC.
2. **?-operator protocol-dispatch (cross-cutting)** — see ops audit §1.
3. **Iterator.try_collect / try_fold for Result** — once §2 lands.
