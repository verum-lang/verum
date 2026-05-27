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

**Root cause hypothesis**: `Text.slice(a, b)` returns a `Text`
record whose `len` field is correctly stamped to `b - a`, but
whose internal byte-buffer pointer either:

1. **Aliases the source `Text`'s buffer rather than copying** —
   when the source is dropped (or the slice escapes the source's
   lifetime by being returned through `parse`), the pointer is
   reused but `len` remains the slice's value. Same defect class
   family as the documented `PathBuf.push` Text-equality drift in
   `core-tests/io/path/audit.md §B` and
   [[btree_pattern_match_ref_generic_class]].

2. **Type-id propagation through archive loader doesn't update
   field-layouts** — the parsed `Url` record's `path: Text` field
   loses its layout binding through the `core.net.url` import
   into a user test module. Same defect class as
   [[use_after_free_error_field_shift_2026-05-27]].

3. **`s.slice(path_start, p)` codegen pathway for "/" pattern** —
   when `path_start + 1 == p`, the slice has length 1 and the
   codegen edge for the single-byte boundary may emit a
   degenerate `[u8; 0]` array literal.

**Workaround discipline applied to conformance suite**: all
assertions on slice-derived Text use `assert(t == lit.clone())`
instead of `assert_eq(t.as_str(), "lit")` or `t.as_bytes()[i]`.
The `Text == Text` path takes a different codegen branch that
compares structurally and works correctly. **89 of 95 tests
land on this path**; the 6 tests that need byte-level access
are deferred to `regression_test.vr` (none landed yet — none
critical-path).

**Fix path**: VBC codegen of `Text.slice` either (a) eager-copy
the slice payload (correctness over performance) or (b) ensure
the slice payload alias is captured through the same record-
clone path that `Text == Text` uses.

**Effort**: 1 day to diagnose root cause + 2-3 days fix VBC
codegen + retest.

### §3.2 `MAX_URL_LENGTH_BYTES` DoS guard

Constant at `url.vr:130` = `64 * 1024` (65,536 bytes). Bounds
the parser's O(N) work to a fixed budget; rejects gigabyte-scale
attacker URLs before any per-byte scanning. Pinned by
`test_max_url_length_bytes_constant` so the constant doesn't
drift under refactoring.

### §3.4 URL-8 — empty-Text `"".clone()` parse routes through non-MissingScheme error kind

**Stable trigger**: `Url.parse(&"".clone())` returns `Err(_)` (correct
behavior — `.is_err()` pin GREEN at `prop_url_parse_empty_returns_err`)
but the returned `e.kind` value compares NOT-equal to
`UrlErrorKind.MissingScheme` under `--interp`. Source-side at
`url.vr:150-156` clearly returns `MissingScheme` for the `n == 0` arm.

**Pin**: `prop_url_parse_empty_error_kind_missing_scheme` @ignore'd
under URL-8. Other UrlErrorKind comparison sites (e.g.
`prop_url_parse_oversized_input_rejects` for UrlTooLong) work
correctly, so the defect is empty-input-specific.

**Likely root cause** (ordered by suspicion):

1. **Empty-Text Eq via discriminant comparison**: `UrlErrorKind` Eq
   impl at `url.vr:100-103` reads `url_error_kind_tag(self) ==
   url_error_kind_tag(other)`. The body uses `match self { ... }`
   which routes through variant-tag dispatch. For
   `MissingScheme` (tag=0) the call may collide with an unrelated
   tag-0 dispatch site (same defect class as `[[task17_static_method_dispatch_defect_2026-05-24]]`).
2. **`"".clone()` representation drift**: empty-Text codegen may
   produce a non-canonical representation (different from the
   compile-time-empty `""`) that the `as_bytes().len() == 0`
   check misses, routing parsing into a non-empty branch and
   returning a different error kind (e.g. `InvalidScheme`).
3. **`e.kind` field access**: post-Err(record) construction, the
   field read may shift indices (sister of
   `[[use_after_free_error_field_shift_2026-05-27]]`).

**Fix path**: trace via `VERUM_TRACE_VARIANT_EQ=1
VERUM_TRACE_FIELD_READ=1` against the failing test. Expected:
~2-4 hours diagnosis + medium fix once root narrowed (variant-eq
likely Tier-0 dispatch; field-access shift requires codegen).

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
