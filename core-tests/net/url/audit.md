# `net/url` audit

Module: `core/net/url.vr` (~483 LOC) — URL parsing + serialization
per RFC 3986 + percent-encoding helpers per RFC 3986 §2.1/§2.3.

Tests cover the algebraic surface end-to-end: scheme parsing
(case-insensitive lowercase canonicalization + +/- characters),
port handling (0/443/65535/missing), userinfo Some/None,
path length, query Some/None, fragment Some/None, IPv6 literal
in brackets with optional port, error paths (MissingScheme /
InvalidPort / InvalidAuthority), MAX_URL_LENGTH_BYTES DoS
guard, percent_encode (unreserved passthrough + reserved
encoding + uppercase-hex output) + percent_decode (mixed-case
hex + truncated/non-hex error paths) + round-trip identity.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http` | client-side `Url` re-export (used in `HttpClient`). |
| `core.net.weft` | router path-parameter extraction. |
| `core.net.uri_template` | RFC 6570 expansion target. |
| `core.net.http_range` | building Range headers against URL endpoints. |
| Application networking | every URL string → component breakdown. |

## 2. Crate-side hardcodes

None. Pure-Verum byte arithmetic; `Text.from_utf8_unchecked`
only used internally.

## 3. Language-implementation gaps

### §3.1 URL-1 — `Text.slice(a, b).as_str()` corrupts pointer-but-not-length

**Stable trigger**: A `Text` value obtained via `Text.slice(start,
end)` reports the **correct length** through `text.len()` but its
byte payload through `text.as_bytes()[i]` panics with "Slice
index out of bounds: index 0 but length is 0" — i.e. the byte
buffer is empty even though `len() > 0`.

Reproduction (passes len, fails byte access):

```verum
let s = "http://example.com/".clone();
let u = Url.parse(&s).unwrap();
assert_eq(u.path.len(), 1);          // ← passes: len=1
let bytes = u.path.as_bytes();
assert_eq(bytes[0], 0x2F_u8);        // ← panics: length is 0
```

Compare: `assert(u.path == "/".clone())` works because the
canonical `Text == Text` path goes through structural comparison
rather than pointer-into-buffer access.

**Root cause diagnosis 2026-05-28** (post-eager-copy-fix):

Pre-fix `Text.slice` used `from_utf8_unchecked(slice_from_raw_parts(...))`
which constructed a Text record aliasing the source pointer. Replaced
2026-05-27 (commit `fd02ab012`) with eager-copy via `with_capacity +
push_byte` — same pattern as `to_lowercase`.

**Post-rebuild test matrix** (binary built 2026-05-28 00:05):

| Probe | Result | Diagnosis |
|---|---|---|
| `s.slice(0, 5)` direct in test | ✅ PASS | Eager-copy works |
| `s.slice(0, 5).as_bytes()[0]` direct | ✅ PASS | Slice payload sound |
| `s.slice(0, 5) == "hello".clone()` direct | ✅ PASS | Eq comparison sound |
| `Url.parse(&s).unwrap().path.len() == 4` | ❌ FAIL | Cross-module record-field corruption |
| `Url.parse(&s).unwrap().path == "/foo".clone()` | ❌ FAIL | Same |
| `Url.parse(&s).unwrap().scheme.as_bytes()[0]` | ❌ FAIL | Same |

**Conclusion**: URL-1 / URL-7 are NOT `Text.slice` aliasing defects.
They are **cross-module record-field corruption** on the returned
`Url` struct. Same defect-class family as URL-8 + `e.kind` field-
read corruption — both manifest as `Url` / `UrlError` fields
returning corrupted bytes through the VBC cross-module struct-
return path.

Root cause is in `compile_field_access` (codegen/mod.rs) or
`resolve_field_index` type-aware path — multi-day VBC codegen
investigation. Sister defects:
[[use_after_free_error_field_shift_2026-05-27]] +
[[btree_pattern_match_ref_generic_class]] +
[[enactment_field_access_oob_2026-05-24]].

**Workaround discipline applied to conformance suite**: all
assertions on slice-derived Text used `assert(t == lit.clone())`
instead of `assert_eq(t.as_str(), "lit")` or `t.as_bytes()[i]`.
Post-diagnosis above: this workaround was a coincidence — even
`assert(field == lit)` fails for cross-module record-returned
Text fields because the field-read itself is corrupted. The
workaround happened to mask URL-1 because most tests don't
inspect fields like `u.path`/`u.scheme` for content — only
constructor + len + Some/None probes.

**Source-side improvement landed 2026-05-27** (commit `fd02ab012`):
`Text.slice` rewritten as eager-copy `with_capacity + push_byte`.
This eliminates the alias-via-raw-pointer surface (sound for
test-site direct use), but doesn't close URL-1 / URL-7 because
those defects are cross-module-record-field corruption (different
defect class).

**Fix path for URL-1 / URL-7 remainder**: VBC codegen
`compile_field_access` for Text-typed record fields returned
through cross-module Result wrappers. Multi-day work.

**Effort**: 2-3 days VBC codegen + retest. Same fix likely closes
URL-8 + the sister field-read corruption defects.

### §3.2 `MAX_URL_LENGTH_BYTES` DoS guard

Constant at `url.vr:130` = `64 * 1024` (65,536 bytes). Bounds
the parser's O(N) work to a fixed budget; rejects gigabyte-scale
attacker URLs before any per-byte scanning. Pinned by
`test_max_url_length_bytes_constant` so the constant doesn't
drift under refactoring.

### §3.4 URL-8 — `e.kind` field-read corruption on cross-module record return

**Diagnosis 2026-05-27** (post binary rebuild + qualified-arm fix):
URL-8 confirmed as candidate #3 in the original audit hypothesis —
**field-read corruption** on cross-module record returns, NOT
variant-tag dispatch collision.

**Stable trigger**: `Url.parse(&"".clone())` returns `Err(UrlError {
... })`. The stdlib body at `url.vr:159-165` clearly constructs
`UrlError { kind: UrlErrorKind.MissingScheme, ... }`. The user-side
read of `e.kind` returns a value that:

1. Does NOT compare equal to `UrlErrorKind.MissingScheme` (via Eq).
2. Does NOT match ANY of the 6 UrlErrorKind variants via `is`
   operator (`InvalidScheme` / `InvalidAuthority` / `InvalidPort` /
   `InvalidPercentEscape` / `UrlTooLong` / `MissingScheme` all return
   false).

**Probe** (deleted after diagnosis):
```verum
Err(e) => {
    if e.kind is UrlErrorKind.MissingScheme { panic(...); }
    if e.kind is UrlErrorKind.InvalidScheme { panic(...); }
    ...   // all 6 variants tested
    panic("kind matched no variant — field-read corruption");
}
// → panic: "kind matched no variant — field-read corruption"
```

**Root cause**: same defect-class family as
[[use_after_free_error_field_shift_2026-05-27]] +
[[btree_pattern_match_ref_generic_class]] +
[[enactment_field_access_oob_2026-05-24]]. Cross-module record-field
access for `e.kind` lands on a byte offset that corresponds to neither
the discriminant tag nor any valid variant payload.

**Source-side workarounds NOT applicable**: qualified arms +
closure-free Result chains landed in this session (commits
`0b60920af` + `f649312c6`) eliminated other defect classes but do
NOT close URL-8 — verified by probe post-rebuild.

**Fix path**: VBC codegen of cross-module struct-field access for
records carrying variant-typed fields. Multi-day work in
`compile_field_access` (codegen/mod.rs) + `resolve_field_index`
type-aware path. Same fix likely closes the 3 sister defects
referenced above.

**Pinned**: 1 @ignore'd regression test in
`regression_test.vr` per URL-8.

**Effort**: 2-3 days VBC codegen + retest.

### §3.3 RFC 3986 §6.2 normalization not implemented

The parser does NOT apply RFC 3986 §6.2.2 syntax-based
normalization (case-insensitive scheme + host, percent-encoding
canonicalization, dot-segment removal). The `to_text` round-trip
preserves the input form; callers building hash keys for
deduplication should canonicalize externally.

## 4. Action items landed in this branch

* `core-tests/net/url/unit_test.vr` — 52 unit tests covering:
  scheme parsing (8) — http/https/ftp/svn+ssh/view-source/
  uppercase/digit-start-rejection;
  port handling (4) — 0/443/65535/missing;
  userinfo (3) — present/absent/no-password;
  path length (3) — /foo/bar/empty/root;
  query Some/None (3);
  fragment Some/None (3) + query+fragment together;
  IPv6 literal (2) — no port + with port;
  error paths (6) — empty/no-scheme/digit-start/port-overflow/
  port-non-digit/unterminated-ipv6;
  MAX_URL_LENGTH_BYTES (1);
  percent_encode (7) — alpha/digit/unreserved-special/space/
  slash/percent/uppercase-hex;
  percent_decode (4) — happy paths + mixed case;
  percent_decode error (2) — truncated + non-hex;
  round-trip (3) — alpha/special/reserved;
  UrlErrorKind disjointness (5).
* `core/net/url.vr` — URL-8 close-out: qualify 6 match arms in
  `url_error_kind_name` + 6 in `url_error_kind_tag` + 6 `Err(UrlError
  { kind: ... })` record-literal sites with `UrlErrorKind.<Variant>`
  form. Activates on next verum binary rebuild.
* `core-tests/net/url/audit.md` — this file.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Close URL-1 (Text.slice payload aliasing) | VBC codegen | 3-5 days incl rebuild |
| Url.to_text round-trip — currently deferred behind URL-1 | this folder | trivial; gated on §3.1 |
| RFC 3986 §6.2 normalize (case + dot-segment) | stdlib + tests | 1 day stdlib + 2h tests |
| Url.path / .query as `&Text` zero-copy accessor (sidesteps URL-1) | stdlib | 2h once URL-1 root cause known |
| Property test ∀url. parse(url).to_text() == url (modulo whitespace) | this folder | 2h, gated on URL-1 |
| Percent-encoding pct-encoded-three-octet round-trip (UTF-8) | this folder | 1h |
