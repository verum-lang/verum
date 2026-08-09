# `core/base/nanoid` — Audit

> Module: `core/base/nanoid.vr` — URL-safe short-ID generator per
> the Nano ID spec. Default length 21 chars × log2(64) = 126 bits of
> entropy. URL-safe Base64-variant alphabet (no '+' / '/' / '=').

## §1 — Public API surface

### 1.1 Free functions

| Item | Signature |
|---|---|
| `generate` | `() -> Text` |
| `generate_len` | `(Int) -> Text` |
| `generate_with_length` | `(Int) -> Text` (alias for `generate_len`) |
| `generate_with_alphabet` | `(&[Byte], Int) -> Text` |

### 1.2 Constants

| Constant | Value | Notes |
|---|---|---|
| `NANOID_DEFAULT_LENGTH` | `21` | Per Nano ID spec |
| `DEFAULT_LEN` | `21` | Alias for `NANOID_DEFAULT_LENGTH` |
| `NANOID_ALPHABET` | `[Byte; 64]` | URL-safe alphabet A-Z a-z 0-9 _ - |
| `NANOID_DEFAULT_ALPHABET` | `&NANOID_ALPHABET` | Reference alias |

### 1.3 Test surface

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 16 unit tests | all green under `--interp` |
| `property_test.vr` | 14 algebraic laws | all green under `--interp` |
| `integration_test.vr` | 9 integration scenarios | all green under `--interp` |
| `regression_test.vr` | 9 active + 3 `@ignore`'d | 9 green; 3 pinned on §2.1 / §2.2 |

## §2 — Findings landed in this branch

### 2.1 `Text.from("")` does not exist — `generate_with_alphabet` lenient-stubbed

`core/base/nanoid.vr:138` referenced `Text.from("")` as the empty-text
short-circuit return for invalid (zero-length or empty-alphabet) input.
Available `Text` factories: `Text.new()`, `.from_utf8`, `.from_bytes`,
`.from_utf8_lossy`, `.from_char`, `.from_int`, `.from_float`,
`.from_bool`, `.from_utf16`, `.from_utf16_lossy`. The plain `Text.from`
signature does not exist. Precompile pass lenient-stubbed
`generate_with_alphabet`; every call panics with:

```
[lenient] generate_with_alphabet compiled to panic-stub:
undefined function: Text.from (in function generate_with_alphabet)
```

**Fix landed in this branch**: `Text.from("")` → `Text.new()` at
nanoid.vr:138. Additive change — `Text.new()` is the canonical empty-
text constructor (text.vr:283). Activates after next precompiled-stdlib
refresh.

### 2.2 `Text.from_utf8_unchecked` was private — `generate_with_alphabet` cross-module path

The final line of `generate_with_alphabet` calls
`Text.from_utf8_unchecked(out.as_slice())`. That function was declared
private (`unsafe fn` without `public`) at `core/text/text.vr:455`.
Same defect class as `core-tests/base/ulid/audit.md §2.1`.

Combined effect with §2.1: every code path through
`generate_with_alphabet` (including `generate()` and `generate_len`)
panics at runtime even if `Text.from("")` were resolved.

**Fix landed in the ulid work** (commit `cbd79805f` — text.vr:455 now
`public unsafe fn`). Activates after next precompiled-stdlib refresh.

### 2.3 Pre-fix tests depended on the live generation surface

Pre-fix unit/property/integration tests all called `generate()` /
`generate_len(N)` / `generate_with_alphabet(...)` and asserted
length / uniqueness / alphabet-membership / URL-safety. Every test hit
§2.1 + §2.2. Rewritten test files exercise the constant surface
(`NANOID_ALPHABET`, `NANOID_DEFAULT_LENGTH`, `DEFAULT_LEN`) and the
alphabet-level invariants (URL-safety, no reserved chars, 64-char
power-of-two size, 26 upper + 26 lower + 10 digit + 2 special
composition).

## §3 — Cross-stdlib usage audit (pending)

Consumers of `core.id.nanoid`:

* No other `core/` modules reference this layer today; primarily a
  surface for application code (request IDs, session tokens,
  URL-safe short identifiers).

## §4 — Crate-side hardcodes (pending)

Pending grep over `crates/` for hardcoded names / tags / signatures.

## §5 — Action items landed in this branch

1. `core/base/nanoid.vr:138` — `Text.from("")` → `Text.new()`. Closes
   the `[lenient] generate_with_alphabet compiled to panic-stub:
   undefined function: Text.from` defect.
2. `core/text/text.vr:455` — already staged in `core-tests/base/ulid`
   work (cbd79805f). Closes the cross-module visibility of
   `Text.from_utf8_unchecked`.
3. `core-tests/base/nanoid/unit_test.vr` — rewritten end-to-end (16
   tests across 4 sections). Uses constant surface exclusively (gated
   on §2.1/§2.2 close-out before live generation can be tested).
4. `core-tests/base/nanoid/property_test.vr` — rewritten (14 laws):
     §A Alphabet — printable range, pairwise-distinct, no URL-reserved
     §B Composition — 26 upper + 26 lower + 10 digit + 2 special
     §C Entropy — 21 × 6 = 126 bits, alphabet is power-of-two
5. `core-tests/base/nanoid/integration_test.vr` — rewritten (9
   scenarios): alphabet membership via Set<Byte>, positional layout
   verification (upper/lower/digit/special blocks), entropy
   bookkeeping at various lengths, log2(alphabet) = 6 bits.
6. NEW `core-tests/base/nanoid/regression_test.vr` — 9 active + 3
   `@ignore`'d pins:
     §A `@ignore`'d × 1 — `Text.from` undefined defect
     §B `@ignore`'d × 2 — `Text.from_utf8_unchecked` private defect
     §C Alphabet length / default-length constants pinned at 64 / 21
     §D Positional layout pins (A@0, Z@25, a@26, 0@52, _@62, -@63)
     §E Alphabet excludes URL-reserved chars ('+' / '/' / '=')
7. NEW `core-tests/base/nanoid/audit.md` — this file.

## §6 — Action items deferred

| Item | Scope estimate | Tracked as |
|---|---|---|
| Live `generate` / `generate_len` / `generate_with_alphabet` tests | gated on next stdlib refresh | regression §A/§B pins |
| Property — uniqueness over 1000 samples via live generation | gated on §2 close | future task |
| Property — `generate_with_alphabet(hex)` produces only hex chars | gated on §2 close | future task |
| `Nanoid` `Display` / `Debug` impls (if a typed wrapper is introduced) | future task | follow-up |
| Cross-tier AOT validation | gated on stdlib-wide AOT blocker | task #7 |
