# `net/content_negotiation` audit

Module: `core/net/content_negotiation.vr` (~374 LOC) — RFC 9110
§12 content negotiation parsers + selectors for the
`Accept` / `Accept-Encoding` / `Accept-Language` family.

Tests cover the data-surface: MediaRangeSpec record fields
(type_main / subtype / q), CodedPreference record fields
(value / q), MAX_NEGOTIATION_ENTRIES DoS hardening constant.

Full functional surface (`parse_accept`,
`parse_accept_encoding`, `parse_accept_language`,
`select_best_media`, `select_best_coding`) is locked-in behind
CONNEG-1 in `regression_test.vr`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` response builder | content-type / encoding selection. |
| `core.net.http` handlers | server-side variant picking. |
| `core.compress` adapters | Accept-Encoding → algorithm chooser. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic.

## 3. Language-implementation gaps

### §3.1 CONNEG-1 — `parse_*` / `select_*` SIGSEGV during precompile cascade

**Stable trigger**: any reachable callsite of the five public
free-fns from a USER test module SIGSEGVs the compiler during
the precompile cascade for `content_negotiation.vr`. Same crash
signature family as CIDR-1 / URL-1 / URITPL-1 / HTTPRNG-1.

The data-surface (variant construction + Eq lattice) compiles
and tests pass. The functional surface is locked-in by 8
@ignore'd regression pins covering: parse_accept simple +
q-value + wildcard, parse_accept_encoding gzip, parse_accept_language
en-US, select_best_media exact match + no-match-None,
select_best_coding q=0 rejection.

**Effort**: 3-5 days fix (batched with CIDR-1 / URL-1 / URITPL-1 /
HTTPRNG-1).

### §3.2 MAX_NEGOTIATION_ENTRIES pinned at 256

CVE-2011-3192-class hardening — a peer sending
`Accept: */*,*/*,*/*,...` with thousands of entries forces the
parser to allocate per-entry MediaRangeSpec values + perform
downstream q-value sort. 256 is the chosen ceiling.

### §3.3 RFC 9110 §12.5.3 identity-coding default acceptable behavior

The "identity" content coding is implicitly acceptable for
Accept-Encoding unless it appears with `q=0`. This is
implementation-side; tests pin the data-shape via
`test_coded_preference_identity`.

## 4. Action items landed in this branch

* `core-tests/net/content_negotiation/unit_test.vr` — 13 unit
  tests covering MediaRangeSpec construction across q=0/q=0.9/
  q=1.0 + wildcard type + wildcard subtype + 5 CodedPreference
  forms (gzip / identity / wildcard / q=0 rejection / RFC 5646
  language tag) + MAX_NEGOTIATION_ENTRIES canonical 256 + >0
  + exceeds-nginx pin.
* `core-tests/net/content_negotiation/regression_test.vr` — 8
  @ignore'd LOCK-IN pins for CONNEG-1.
* `core-tests/net/content_negotiation/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close CONNEG-1 (batched with CIDR-1 family) | VBC codegen | 3-5 days |
| Full parse_accept coverage (q-value parsing, parameter handling, whitespace) | this folder | 1h, gated on §3.1 |
| Property test ∀prefs,offers. select picks the highest q-value match | this folder | 2h, gated on §3.1 |
| Charset negotiation (RFC 9110 §12.5.2) | stdlib add + tests | 4h |
| MediaRangeSpec `params: Map<Text, Text>` extension for parameter-aware matching | stdlib + tests | 1 day |
