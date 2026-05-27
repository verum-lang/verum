# `net/link_header` audit

Module: `core/net/link_header.vr` (~374 LOC) — RFC 8288 Link
header parser + builder. The hypermedia-relation / pagination /
preload-hint header used by GitHub API pagination, ActivityPub,
W3C Annotations, HAL, AS2.

Tests cover the data-surface algebra: LinkEntry record (uri /
params), LinkHeaderError 2-variant (Malformed / TooManyEntries),
MAX_LINK_ENTRIES DoS hardening constant.

Full functional surface (`parse`, `find_rel`,
`format_link_header`) is locked-in behind LINKHDR-1 in
`regression_test.vr`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` pagination middleware | RFC 8288 next/prev/canonical emit. |
| `core.net.http` clients | follow-pagination header parsing. |
| HAL / ActivityPub adapters | `_links` extraction. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic.

## 3. Language-implementation gaps

### §3.1 LINKHDR-1 — `parse` / `find_rel` / `format_link_header` SIGSEGV

**Stable trigger**: same precompile-cascade defect class as
CIDR-1 / URL-1 / URITPL-1 / HTTPRNG-1 / CONNEG-1. The data-
surface (LinkEntry construction + LinkHeaderError Eq lattice)
compiles. Functional surface locked-in by 8 @ignore'd
regression pins.

### §3.2 MAX_LINK_ENTRIES pinned at 256

CVE-2011-3192-class hardening — real-world headers carry 2-5
entries (next/prev/canonical/preload).

## 4. Action items landed in this branch

* `core-tests/net/link_header/unit_test.vr` — 12 unit tests
  covering LinkEntry minimal + with-rel + multi-param +
  absolute-URI; LinkHeaderError TooManyEntries Eq +
  declared/limit disjointness + Malformed Eq + variant
  disjointness; MAX_LINK_ENTRIES canonical 256 + >0 + >100.
* `core-tests/net/link_header/regression_test.vr` — 8
  @ignore'd LOCK-IN pins for LINKHDR-1 covering parse +
  find_rel + format + error paths.
* `core-tests/net/link_header/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close LINKHDR-1 (batched with CIDR-1 family) | VBC codegen | 3-5 days |
| Full RFC 8288 parameter coverage (title*, hreflang, media, type) | this folder | 1h, gated on §3.1 |
| RFC 8288 §3.4 quoted-string escape handling | stdlib check + tests | 2h |
| Property test: parse(format(entries)) == entries (modulo whitespace) | this folder | 1h, gated on §3.1 |
