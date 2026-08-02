# Nominal Identity: DefId end-to-end (ROOT-A)

**Status:** S0 (research + baseline) landed; S1 next.
**Owner surface:** `verum_common` (the id), every phase (the consumers).
**Gates:** `scripts/ci/census_name_keyed_surfaces.py --check` (ratchet —
categories may only shrink), per-stage regression pins listed below.

## The problem, measured

Identity of functions and types is a **string** almost everywhere in the
toolchain. The census (2026-08-02, reproducible via the script above)
counts **679 name-keyed identity points** across eight categories:

| category | count | retired by |
|---|---:|---|
| fn-registry writes (`ctx.functions` string keys) | 108 | S3 |
| fn-registry lookups | 223 | S3 |
| ranked suffix probes (`ends_with(".leaf")`) | 48 | S5 |
| `name#arity` composite keys | 11 | S3 |
| type identity via name maps | 237 | S2 |
| archive-loader name indexes | 20 | S4 |
| runtime by-name resolution | 24 | S5 |
| id→name carry side-channels | 8 | S4 |

Every category is a measured defect factory, not a theoretical one:

* **Bare-name capture.** `ParseError` exists in `core.base.protocols`
  (`{message}`) and `core.cli.error` (5 fields). The re-export hop
  resolved the base name to the *cli* twin — member lookup listed the
  wrong type's methods, and one stale simple-slot took 242 tests of
  `text/text` hostage (fixed test-side; the compiler-side class is
  T0458: **99 co-active duplicate typenames**).
* **Arity flip.** T0448: two same-named functions, lowest-arity-wins
  canonicalization silently DROPS one.
* **Alias split.** `type Byte is UInt8` — methods bake under `UInt8.*`,
  a partial mangle table dispatched `byte$*`, and the whole non-width
  method surface died unresolvable (T0695, fixed by ONE authority —
  but the split itself exists because alias identity is two strings).
* **Unstable ids.** The entire T0277 campaign — three separate
  id→name carry channels — exists **only** because FunctionIds are
  renumbered per phase and names are the only thing that survives.
  Carries are the tax; DefId is the repeal.

There is also a **performance** dimension (toolchain speed is a product
pillar: script start latency and compile throughput are first-class
acceptance criteria — the embedded stdlib exists for them). String-keyed
resolution costs hashing + ranked probes on the hottest compile paths;
integer DefIds turn those into direct indexed loads. The migration must
show **flat-or-better** cold-start and compile-throughput numbers at
every stage (hello-world cold run; the compilation-speed contract in
`tests/compilation_speed_contract.rs`).

## The design

### The id

```rust
/// verum_common::defid
pub struct DefId(pub u64);
//  bits 63..48 : origin space (cog ordinal; 0 = current compilation,
//                1 = embedded stdlib bake, 2.. = dependency cogs)
//  bits 47..0  : dense per-origin ordinal, minted once at declaration
```

Properties:

* Minted exactly once, at the **declaration collection** point of the
  owning compilation (bake mints for stdlib; user compile mints for the
  user cog). Never re-minted, never renumbered — archives serialize the
  DefId verbatim, so "merge remapping" of identity disappears (function
  *table indices* may still be per-module; the id is not the index).
* The **spelling table** `DefId -> InternedStr` (qualified canonical
  name) is a side table, ONE per artifact, used for diagnostics and for
  the (shrinking) set of by-name edges. Names never flow back into
  identity decisions.
* Aliases (`type Byte is UInt8`) are entries in an **alias table**
  `DefId -> DefId` resolved to a fixpoint at mint time; every consumer
  sees the target's DefId, the alias survives only as a spelling for
  diagnostics. This retires the `Byte`/`UInt8`/`byte$` triple-identity
  class outright.

### What stays name-keyed (the edges)

* Source text → DefId: name resolution proper (mounts, scopes) — this
  is the ONE place strings legitimately decide binding, and T0559's
  reference architecture governs it.
* Reflective/dynamic lookups (`Engine.eval` by name, LSP queries).
* Diagnostics rendering (via the spelling table).

Everything else — dispatch, merge, remap, monomorphization keys,
protocol method slots, layout registries — moves to DefId.

## Staged plan (each stage lands green, gated, alone)

**S1 — the id exists (verum_common).**
`DefId`, `OriginSpace`, interner, spelling-table type. No consumer
changes. Gate: unit surface; census unchanged.

**S2 — type identity (verum_types + verum_vbc types).**
`TypeId` acquires a DefId origin (or is replaced where user-facing);
`type_name_to_id` / `type_defs` become name→DefId **edge views** with
explicit ambiguity results instead of first-wins slots; the alias table
lands here (fixes the `Maybe<Unit>` nominal-vs-structural and the
Byte/UInt8 splits at the root). Ratchet: `type-name-keys` 237 → ≤ 60
(edge-only remainder). Pins: ParseError twin repro (pe1/pe3/pe4),
duplicate-typename gate from T0458.

**S3 — function identity in codegen (verum_vbc).**
`FunctionInfo` keyed by DefId; `ctx.functions` becomes
`name -> SmallVec<DefId>` (ambiguity is DATA, not a race);
`name#arity` keys deleted; CallM/Call carry DefId when statically
resolved (bytecode format bump, both-tier decode). Ratchet:
fn-registry writes/lookups → edge-only (~40/–), arity keys → 0.
Pins: T0448 arity-flip repro, T0538 collision suite.

**S4 — archives carry DefId (verum_vbc archive + verum_compiler loader).**
Serialize DefId + spelling/alias tables (schema bump + full re-bake);
`ArchiveBodyRemap` tiers collapse to origin-space translation; the
three id→name carry channels are **deleted** (their tests flip to
asserting the carries are gone). Ratchet: loader-name-indexes → ≤ 5,
id-name-carries → 0. Pins: the entire T0277 battery (banner stays 0
without the carries), provenance-eq p32 both tiers.

**S5 — runtime dispatch (interpreter + AOT lowering).**
Band ids retire (a cross-module call IS a DefId; lazy linking keys on
it); suffix-probe resolution survives only in diagnostics; protocol
method slots are `(DefId type, DefId method)`. Ratchet:
runtime-byname-resolution → ≤ 5 (reflective edge), suffix-probes →
≤ 10 (diagnostics only). Pins: reference_system L0 suite both tiers,
`VERUM_STRICT_MONO=1` build of the probe corpus.

## Risks and their controls

* **Bake nondeterminism** (§40 positional-field class): DefId ordinals
  depend on collection order → the bake must mint in the canonical
  module walk order it already pins for field interning; the existing
  double-bake determinism gate covers it.
* **Format breaks**: S3 (bytecode) and S4 (archive) each bump their
  format version; both-tier conformance batteries + full re-bake are
  the stage acceptance, per the established T0193/T0177 wire-change
  discipline.
* **Perf regressions**: every stage runs the compile-speed contract and
  a hello-world cold-start measurement; a regression blocks the stage.
* **Parallel truths during migration**: each stage DELETES the surface
  it replaces in the same landing (redundancy directive — no
  transition shims left behind); the census ratchet enforces monotone
  progress mechanically.

## Relationship to the sibling programs

* **T0691 SYMBOL-GRAPH-ONE-TRUTH** consumes DefId as its node key; it
  can start name-keyed and swap, but its "declarer, not umbrella"
  re-export fixpoint is needed by S2's alias table — coordinate.
* **T0559 name-resolution architecture** remains the authority for the
  source→DefId EDGE; DefId does not change *which* declaration a name
  picks, it makes the pick unforgeable afterwards.
* **T0699 CONTRACT-ARCHIVES** rides S4: the contracts section keys its
  obligations by DefId.
