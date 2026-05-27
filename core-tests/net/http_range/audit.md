# `net/http_range` audit

Module: `core/net/http_range.vr` (~449 LOC) — HTTP Range +
Content-Range header parser + builder per RFC 9110 §14.

Tests cover the data-surface algebra: RangeSpec 3-variant
(Closed / Prefix / Suffix), RangeSet single-field record,
ResolvedRange record (start + end UInt64), RangeError 3-variant
(Malformed / UnsatisfiableRange / TooManyRanges) Eq + variant
disjointness, MAX_RANGE_SPECS Apache CVE-2011-3192 hardening
constant.

Full functional surface (`parse_range_header`,
`resolve_and_merge`, `encode_content_range`,
`encode_unsatisfiable`) is locked-in behind HTTPRNG-1 in
`regression_test.vr` — see §3.1.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.weft` static-file serving | byte-range support for video/PDF. |
| `core.net.http` | Range request header + 206 Partial Content + 416 Range Not Satisfiable response paths. |
| Resumable-download clients | building Range request headers. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic.

## 3. Language-implementation gaps

### §3.1 HTTPRNG-1 — `parse_range_header` / `resolve_and_merge` / `encode_*` SIGSEGV

**Stable trigger**: any reachable callsite of the four public
free-fns from a USER test module SIGSEGVs the compiler during
the precompile cascade for `http_range.vr`. Same crash
signature family as CIDR-1 / URITPL-1 / URL-1.

The data-surface (variant construction + Eq lattice) compiles
and tests pass. The functional surface is locked-in by 9
@ignore'd regression pins covering: single closed range,
multi-range, suffix range, wrong-unit rejection, resolve in-
range, resolve merge-overlapping, resolve all-invalid →
UnsatisfiableRange, encode_content_range, encode_unsatisfiable.

**Likely root cause**: same VBC codegen surface as CIDR-1 (`?`
operator on Result chains, `text.as_bytes()`, internal
`Text.from_utf8_unchecked(buf.as_slice())` helpers). Investigation
should batch all five SIGSEGV defects (CIDR-1, URL-1, URITPL-1,
HTTPRNG-1, plus any subsequent net.* finds).

**Effort**: 3-5 days fix VBC codegen — likely closes the entire
defect class.

### §3.2 MAX_RANGE_SPECS pinned at 256

Apache CVE-2011-3192 ("killapache") DoS-attack hardening — a
peer sending thousands of overlapping ranges forces O(N²)
merge cost. 256 is the chosen ceiling. Pinned by
`test_max_range_specs_value` so the constant doesn't drift.

## 4. Action items landed in this branch

* `core-tests/net/http_range/unit_test.vr` — 21 unit tests
  covering RangeSpec 3-variant construction + disjointness (5),
  RangeSet 0-element + 1-element + mixed (3), ResolvedRange
  field preservation across zero / large values (3),
  MAX_RANGE_SPECS positive + canonical 256 (2), RangeError
  Eq + disjointness lattice (8).
* `core-tests/net/http_range/regression_test.vr` — 9 @ignore'd
  LOCK-IN pins for HTTPRNG-1 covering parse + resolve + encode.
* `core-tests/net/http_range/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close HTTPRNG-1 (batched with CIDR-1 / URL-1 / URITPL-1) | VBC codegen | 3-5 days |
| Full parse_range_header coverage (multi-range, suffix, prefix, whitespace tolerance) | this folder | trivial; gated on §3.1 |
| resolve_and_merge coalescing law (∀a,b. resolve(a)∪resolve(b) ≡ resolve(a∪b)) | this folder | 2h, gated on §3.1 |
| encode_content_range round-trip with parse_content_range | stdlib add (parser absent) + tests | 1 day stdlib + 2h tests |
| multipart/byteranges response builder | stdlib add | 1 day |
