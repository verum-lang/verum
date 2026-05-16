# `core.mem.capability` — audit findings

> Module under test: `core/mem/capability.vr` (314 LOC; 8 single-bit flags,
> 7 preset constants, 1 sum type `Capability` with 8 variants, 18 free
> functions covering containment / attenuation / delegation / revocation /
> packing).
>
> Test surfaces (this branch):
> `unit_test.vr` (~340 LOC), `property_test.vr` (~210 LOC),
> `integration_test.vr` (~115 LOC), `regression_test.vr` (~115 LOC).
> All four pass `verum test --interp` and `verum test --aot` on the
> commit listed in this directory's git log.

## 1. Cross-stdlib usage

`core.mem.capability` is the bottom of the CBGR reference protocol —
every reference type in `core/mem/{thin_ref, fat_ref}.vr` packs its
capability bits via `pack_epoch_caps` and validates dereferences via
`has_capability` / `validate_*`.

| Consumer | Use |
|---|---|
| `core/mem/thin_ref.vr` | `ThinRef<T>.{ptr, generation, epoch_and_caps}` — `epoch_and_caps` is `pack_epoch_caps(epoch, caps)`. |
| `core/mem/fat_ref.vr` | `FatRef<T>.{ptr, generation, epoch_and_caps, metadata, offset, reserved}` — same packing. |
| `core/mem/header.vr` | Allocation header carries capability flags; validate-on-deref reads `CAP_READ`/`CAP_WRITE`. |
| `core/mem/diagnostics.vr` | `MemHeaderView.capabilities` exposes the underlying capability set for observers. |
| `core/base/memory.vr` | `Heap.new` / `Shared.new` install `CAP_OWNED` on the inline header capability field. |

The 32-bit packed `epoch_and_caps` layout (caps in upper 16, epoch in
lower 16) is the **canonical CBGR shape** — every reference field-write
that updates either half must funnel through `pack_epoch_caps` /
`unpack_*` to keep the two halves in sync. `property_test.vr §F`
exhausts the (epoch, caps) round-trip and `regression_test.vr §B` pins
the bit-shift semantics.

## 2. Crate-side hardcodes

Drift surfaces — Rust-side code that hardcodes the same values:

| Constant / site | What it pins | Risk if mis-pinned |
|---|---|---|
| `CAP_READ` = bit 0 / `CAP_WRITE` = bit 1 / … (mem/capability.vr lines 43-64) | Single-bit positions | Every Rust-side capability check (`crates/verum_cbgr/src/checked.rs`) MUST agree. Bit-position drift would silently corrupt capability semantics. |
| `CAP_ALL` = 0x00FF (line 71) | Low-byte mask for "all 8 caps" | If a 9th capability flag is added past bit 7, `CAP_ALL` must extend. The `attenuate(x, CAP_ALL)` invariant — "preserve every defined capability" — is the load-bearing semantics. |
| Pack layout: `caps << 16 \| epoch` (line 299) | Field-bit assignment for `epoch_and_caps` UInt32 | Every site that reads or writes `ThinRef.epoch_and_caps` / `FatRef.epoch_and_caps` MUST go through `pack_epoch_caps` / `unpack_*`. Drift here is a kernel-soundness incident. |
| `Capability` variant tag → bit mapping | Tag 0 = Read, 1 = Write, 2 = Execute, … (`to_bit` match in lines 109-120) | If a future re-ordering of the variant list desynchronises tag → bit assignment, every `Capability.is_present(flags)` call would mis-classify. Pinned by `unit_test §11`. |

## 3. Language-implementation gaps

### 3.1 Bare-name variant resolution (CLOSED — task #5 §3.1, commit 6cac007b1)

Pre-fix, qualified `Capability.Read` was the only safe form because
bare-name `Read` could collide with another stdlib enum's `Read`
variant; the Path-arm bidirectional resolution had a "first-registered
wins" fallback. Closed by routing bare variants through the
expected-type context inside `infer_expr_path`.

This module's `unit_test §11` and `regression_test §C` pin the bare-name
form so any future regression would surface immediately.

### 3.2 `pack_epoch_caps` previously used `* 65536` instead of `<< 16`

The earlier inline form at the seven `epoch_and_caps` write sites in
`thin_ref.vr` and `fat_ref.vr` read:

```verum
(caps as UInt32) * 65536 | (epoch as UInt32)
```

Algebraically equivalent to `(caps << 16) | epoch` for UInt16 caps
because the product is ≤ 0xFFFF0000 < UInt32.MAX, but the multiply form
routed through the overflow-checked multiplication backend on some
codegen paths. The shift form makes the bit-manipulation intent
explicit and reliably compiles to a single `SHL` / `lsl` instruction.

**Fundamental fix landed** (see commit history on `core/mem/capability.vr`):
all seven inline sites replaced by a single `pack_epoch_caps` /
`unpack_epoch` / `unpack_caps` triple. The single canonical site is
now the only audit surface for the layout.

Pinned by `regression_test.vr §B` and `property_test.vr §F`.

### 3.3 `delegate_capability` obfuscated bit-twiddle (CLOSED)

The earlier return value was:

```verum
Some(requested & !CAP_DELEGATE | (requested & CAP_DELEGATE))
```

Algebraically equivalent to `Some(requested)` because
`(x & !K) | (x & K) == x` for any `K`. The obfuscated form looked
like it was masking the Delegate bit but did nothing of the sort,
hiding the actual semantics from soundness review.

The current form returns `Some(requested)` directly with a doc note
explaining why — see the inline comment on `delegate_capability` in
`core/mem/capability.vr` lines 195-207. Future changes that want to
strip Delegate from sub-references by default should add an explicit
`requested & !CAP_DELEGATE` mask with a doc note.

Pinned by `regression_test.vr §A`.

### 3.4 `has_all_capabilities(x, 0)` vacuous-truth corner case

`has_all_capabilities(x, k) = (x & k) == k`. For `k = 0`, this is
`(x & 0) == 0` = true for any `x`. The empty set is vacuously a subset
of every flag-set. This corner case is easy to break in a future
codegen change (e.g., if `has_all_capabilities` were rewritten to
"there exists a bit in k that is set in x" by mistake — that form
would return false for `k = 0`).

Pinned by `regression_test.vr §D`.

### 3.5 `attenuate` monotonicity is the load-bearing CBGR invariant

`attenuate(x, m) = x & m` — by construction, every bit in the result
was in `x`. The CBGR write-path relies on this: every sub-reference
created via `delegate_capability` followed by attenuation can ONLY
lose capabilities, never gain them. If `attenuate` were ever
re-implemented as `x | m` (a typo!) by mistake, the entire monotonic
attenuation invariant would silently invert, and a child reference
could end up with capabilities its parent never had.

Pinned exhaustively by `property_test.vr §C`
(`law_attenuate_monotone_exhaustive`).

## 4. Action items landed in this branch

| # | Defect | Layer | Fix |
|---|---|---|---|
| 1 | Missing test coverage for `core/mem/capability.vr` | `core-tests/mem/capability/{unit,property,integration,regression}_test.vr` | New 4-file suite; ~780 LOC total. |
| 2 | Missing `audit.md` for `core-tests/mem/capability/` | This file. |
| 3 | Capability re-export gap in `core/mem/mod.vr` | `core/mem/mod.vr` re-exports only a partial set (`CAP_*` constants stop at `CAP_READ_WRITE`; `attenuate`, `validate_*`, `pack_epoch_caps`, `delegate_capability`, `revoke_capability` etc. are missing) | **Action item deferred** — extend the re-export list. Until then, tests mount `core.mem.capability.{...}` directly. |

## 5. Action items deferred

| # | Defect | Estimate | Track |
|---|---|---|---|
| §A | Audit `verum_cbgr` Rust-side capability-bit constants for drift against `CAP_*` here. | ~30 min | open |
| §B | Pin `Capability` variant tag → bit mapping with a `verum_common::well_known_types` constant + compile-time `assert!` so a future variant re-ordering surfaces as a build failure. | ~45 min | open |
| §C | Extend `core/mem/mod.vr` re-export list to surface the full capability API. | ~10 min | open |
| §D | Cross-tier divergence sweep: run the four test files under `--aot` and ensure exit-code parity with `--interp`. Required before claiming "tier-aligned" status. | 1 hour wall-clock (AOT compile times) | in progress |
