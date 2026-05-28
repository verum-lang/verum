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

### §3.1 URL-1 / URL-7 — Text.slice alias-via-raw-pointer (CLOSED 2026-05-28)

**Pre-fix trigger**: A `Text` value obtained via `Text.slice(start,
end)` reported the correct length through `text.len()` but its
byte payload through `text.as_bytes()[i]` panicked with "Slice
index out of bounds: index 0 but length is 0" when the slice was
returned across a stdlib record-field boundary.

**Source-side fix landed 2026-05-27** (commit `fd02ab012`):
`Text.slice` rewritten from `from_utf8_unchecked(slice_from_raw_parts(
ptr_offset(self.as_ptr(), start), slice_len))` (alias-via-raw-
pointer construction) to eager-copy via `with_capacity +
push_byte` — same canonical pattern as `to_lowercase`.

**Post-rebuild validation 2026-05-28** (probe sweep with binary
that has Text.slice eager-copy fix):

| Probe | Result |
|---|---|
| `s.slice(0, 5)` direct | ✅ |
| `s.slice(0, 5).as_bytes()[0]` direct | ✅ |
| `Url.parse(&s).path.len() == 4` | ✅ |
| `Url.parse(&s).path.as_bytes()[0]` | ✅ |
| `Url.parse(&s).scheme.len() == 4` | ✅ |
| `Url.parse(&s).scheme.as_bytes()[0]` | ✅ |
| `Url.parse(&s).path == "/".clone()` | ✅ |

**3 URL regression tests transition from @ignore'd to GREEN**:
- `regression_url_scheme_as_str_byte_access_corrupted` ✅
- `regression_url_parse_trailing_slash_path_len_1` ✅
- `regression_url_parse_trailing_slash_path_eq_slash` ✅

**Residual**: URL-8 (empty-Text parse routes through wrong
UrlErrorKind) remains pinned — a different defect (UrlError record
construction, not Text.slice). See §3.4.

### §3.2 `MAX_URL_LENGTH_BYTES` DoS guard

Constant at `url.vr:130` = `64 * 1024` (65,536 bytes). Bounds
the parser's O(N) work to a fixed budget; rejects gigabyte-scale
attacker URLs before any per-byte scanning. Pinned by
`test_max_url_length_bytes_constant` so the constant doesn't
drift under refactoring.

### §3.4 URL-8 — **CLOSED 2026-05-28** — Static-call generic-instantiation preservation at READ-site

**CLOSED via commit `a8fb1933e`** — VBC codegen fix at
`crates/verum_vbc/src/codegen/expressions.rs:18514` +
`:19704`. Mirrors the Call-arm composition at line ~18834.

**Actual root cause** (re-diagnosis 2026-05-28): NOT field-read
corruption. The defect was **static-call generic-instantiation
loss at READ-site**: `extract_expr_type_name`'s MethodCall
static-call arm returned `func_info.return_type_name` without
composing `func_info.return_type_inner`. Archive-loaded
`FunctionInfo` for `Url.parse(&Text) -> Result<Url, UrlError>`
stores `return_type_name = "Result"` (bare base) and
`return_type_inner = ["Url", "UrlError"]` separately. Pre-fix
the static-call read-site dropped the inner generic args.

Downstream consequence: `compile_match` set
`match_scrutinee_type = "Result"`, `Err(e)` payload bind failed
to resolve `e : UrlError` (inner_types extraction empty),
`variable_type_names["e"]` not set,
`resolve_field_index(None, "position")` fell to scan-all-types
+ "most fields" tiebreaker → idx=5 → OOB on 24-byte UrlError.

**Validation**: 33/33 URL property tests GREEN under `--interp`
post-rebuild (was 32/33 with URL-8 @ignore'd).
`prop_url_parse_empty_error_kind_missing_scheme` un-@ignored
and confirmed GREEN.

**Historical diagnosis** (kept for context):
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
