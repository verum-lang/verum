# `core/base/ulid` — Audit

> Module: `core/base/ulid.vr` — 128-bit time-ordered IDs encoded in
> 26-char Crockford Base32 (no I/L/O/U). Direct sibling of
> `core/base/snowflake.vr` for shorter 64-bit time-ordered IDs.

## §1 — Public API surface

### 1.1 Types

| Type | Shape | Public? |
|---|---|---|
| `Ulid` | record `{ high: UInt64, low: UInt64 }` | yes |
| `UlidError` | sum `InvalidLength(Int) \| InvalidCharacter(Byte)` | yes |

### 1.2 Free functions / methods

| Item | Signature |
|---|---|
| `Ulid.new` | `() -> Ulid` (live wall-clock + CSPRNG) |
| `Ulid.now` | `() -> Ulid` (alias for `.new`) |
| `Ulid.parse` | `(&Text) -> Result<Ulid, UlidError>` |
| `Ulid.from_parts` | `(UInt64, &[Byte; 10]) -> Ulid` (static method, raw payload) |
| `Ulid.from_parts_seeded` | `(UInt64, UInt64) -> Ulid` (deterministic seed) |
| `Ulid.timestamp_ms` | `(&self) -> UInt64` |
| `Ulid.to_text` | `(&self) -> Text` (26-char Crockford) |
| `from_parts` (free fn) | `(Int, Int) -> Ulid` (clamps negative to 0) |
| `generate` | `() -> Ulid` (alias for `Ulid.new`) |
| `parse` (free fn) | `(&Text) -> Result<Ulid, UlidError>` |
| `ULID_ALPHABET` (const) | `[Byte; 32]` Crockford alphabet |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 28 unit tests | all green under `--interp` |
| `property_test.vr` | 11 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 11 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 5 active + 3 `@ignore`'d | 5 green; 3 pinned on §2.1/§2.2 |

## §2 — Findings landed in this branch

### 2.1 `Text.from_utf8_unchecked` private — `Ulid.to_text` lenient-stubbed

`core/text/text.vr:455` declares the function as `unsafe fn from_utf8_unchecked`
(without `public`). At precompile time `Ulid.to_text` is lenient-
stubbed because its body cannot resolve the cross-module reference.
Every call to `.to_text()` panics with:

```
[lenient] Ulid.to_text compiled to panic-stub:
undefined function: Text.from_utf8_unchecked (in function Ulid.to_text)
```

**Fix landed in this branch**: `core/text/text.vr:455` is now
`public unsafe fn from_utf8_unchecked(bytes: &[Byte]) -> Text`. The
visibility change activates after the next precompiled-stdlib refresh.
Until then, `Ulid.to_text()` and any round-trip-via-text test stay
pinned at `regression_test.vr §A` as `@ignore`'d.

### 2.2 `Ulid.new()` mis-dispatches via `SystemTime.now()`

`Ulid.new()` (and the aliases `Ulid.now()` / `generate()`) call
`SystemTime.now().timestamp_millis()`. The static-method dispatcher
mis-routes `SystemTime.now` to `SysTimeOpsInstant.now()` per task
#17/#39. Symptom: every live-clock construction panics with:

```
field access out of bounds: field index 1 (offset 8+8 = 16)
exceeds object data size 8 type_id=... type='SysTimeOpsInstant'
```

Same defect class as `core-tests/base/snowflake/regression_test.vr §D`.
Workaround in tests: use `from_parts(int, int)` /
`Ulid.from_parts_seeded(uint64, uint64)` for deterministic construction,
never `Ulid.new()`. Live-clock pin at `regression_test.vr §B` as
`@ignore`'d.

### 2.3 Pre-fix tests referenced API that doesn't exist or is broken

| Pre-fix call | Status |
|---|---|
| `assert_eq(parsed.unwrap(), original)` | `Ulid` has no `Eq` impl |
| `parse("...")` (no `&`) | `parse` takes `&Text`, not owned `Text` |
| `Ulid.parse(&upper) == Ulid.parse(&lower)` | Result equality requires `Eq for Ulid` |
| Every `Ulid.new()` call | Hits §2.2 |
| Every `.to_text()` call | Hits §2.1 |
| Generic `Ulid.from_parts(uint64, &rand_bytes)` mixed with free fn `from_parts(int, int)` | Two different signatures; the free-fn form is `from_parts(Int, Int) -> Ulid` |

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.base.ulid`:

* `core.action.*` / `core.signal.*` — event / task IDs.
* `core.database.*` — primary key candidate when 128-bit uniqueness is
  worth the storage vs Snowflake's 64-bit form.
* No other `core/` modules reference this layer at present.

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. `core/text/text.vr:455` — `unsafe fn from_utf8_unchecked` →
   `public unsafe fn from_utf8_unchecked`. Activates Ulid.to_text and
   any other consumer (Hex/Base64/encoding modules) that round-trips
   binary → Text through the unchecked path.
2. `core-tests/base/ulid/unit_test.vr` — rewritten end-to-end (28
   tests across 6 sections) using `from_parts` factory exclusively
   for ULID construction; parse-validation paths use hand-built
   26-char canonical strings; no Eq dependency.
3. `core-tests/base/ulid/property_test.vr` — rewritten (11 algebraic
   laws): from_parts timestamp round-trip, parse rejection over wrong
   lengths, alphabet membership / no-duplicates / ASCII-printable
   range, UlidError variants disjoint under Eq, parametrised reject
   on short inputs.
4. `core-tests/base/ulid/integration_test.vr` — rewritten (11
   scenarios) using `from_parts` and parse-only validation paths;
   covers 48-bit timestamp boundary, distinct/same-ts behaviour,
   100-ULID corpus uniqueness via Set<UInt64>, parse error paths,
   `UlidError` variants in `List<UlidError>`.
5. NEW `core-tests/base/ulid/regression_test.vr` — 5 active + 3
   `@ignore`'d pins:
     §A `@ignore`'d × 2 — `Ulid.to_text` lenient panic-stub (`§2.1`)
     §B `@ignore`'d × 1 — `Ulid.new` static-method dispatch (`§2.2`)
     §C 48-bit timestamp round-trip
     §D negative clamping to 0
     §E alphabet excludes I/L/O/U
     §F parse rejects wrong-length input with `InvalidLength(n)`
     §G all-zero canonical string parses to all-zero ULID
6. NEW `core-tests/base/ulid/audit.md` — documents API surface, this
   branch's findings, deferred items.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| `Ulid` `Eq` / `Hash` / `Clone` / `Display` / `Debug` impls | 1h, additive to stdlib | future task |
| Live `Ulid.new` / `Ulid.now` / `generate` tests | gated on §2.2 close | regression §B pin |
| `to_text` round-trip tests | gated on `runtime.vbca` regeneration | regression §A pins |
| Property — uniqueness over 1000 samples via live clock | gated on §2.2 | future task |
| Property — lex-order matches chronological-order | gated on §2.2 + §2.1 | future task |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
