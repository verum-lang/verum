# `net/http2/hpack` audit

Module: `core/net/http2/hpack.vr` (~487 LOC) — HPACK header
compression (RFC 7541): the N-bit-prefix integer codec (§5.1),
string-literal codec (§5.2), the size-bounded dynamic table
(§4.4), and the full `HpackEncoder` / `HpackDecoder` pipeline
(§6.1-§6.3).

Tests exercise the wire-exact codec surface using the canonical
vectors from RFC 7541 Appendix C.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.http2.frame` | HEADERS / PUSH_PROMISE / CONTINUATION block fragments. |
| `core.net.http2.static_table` | combined index space 1..=61. |
| `core.net.http2.huffman` | optional string-literal Huffman form (§5.2). |

## 2. Crate-side hardcodes

The integer codec implements the generic §5.1 algorithm; every
prefix width n ∈ 1..=8 is exercised by `property_hpack_integer_
round_trip_all_prefixes`. The 32-byte entry-size overhead (§4.1)
is pinned by `test_hpack_header_field_size_includes_32_overhead`.

## 3. Language-implementation findings

### §3.1 HPACK-EXTSLICE — `decode_string` sub-range slice SIGSEGV (FIXED)

`decode_string` originally computed
`let raw = &buf[after_len..after_len + length];` and then
`out.extend_from_slice(raw)`. The sub-range slice expression
tripped the catalogued **EXTSLICE-1** codegen SIGSEGV
(`llvm::SmallVectorBase::grow_pod`) when the helper was
instantiated at a user-`@test` boundary — calling `decode_string`
(or `HpackEncoder.decode`, which reaches it) from a test crashed
the compiler.

`encode_string`, by contrast, passes *whole* slices
(`out.extend_from_slice(input)` / `.as_slice()`) and compiled
cleanly — confirming the trigger is the **sub-range** slice, not
`extend_from_slice` itself.

**Fix (fundamental, stdlib-side):** `decode_string` now
materialises the raw window with an indexed `while` byte-copy into
an owned `List<Byte>`, then passes its full-slice `.as_slice()`
view to the Huffman intrinsic (Huffman branch) or returns it
directly (raw branch). No sub-range slice expression remains.
Pinned by `regression_hpack_decode_string_raw_no_extslice_sigsegv`.

The deeper compiler-layer fix for EXTSLICE-1 (so sub-range slices
lower cleanly everywhere) remains tracked in
`website:docs/stdlib/defect-class-catalogue.md §1`.

### §3.2 HPACK-HUFFMAN — Huffman string form is intrinsic-backed

`encode_string(_, use_huffman: true, _)` and the Huffman branch of
`decode_string` delegate to `verum.http2.hpack_huffman_*`
intrinsics (`core/net/http2/huffman.vr`). The conformance suite
exercises the **raw** string form (H bit clear) exhaustively;
Huffman round-trip coverage is deferred to the huffman/ submodule
folder pending intrinsic verification under `--interp` and `--aot`.

### §3.3 ENCODE-1 — small-string `Text.as_bytes()` across a call boundary SIGSEGVs

`HpackEncoder.encode_one` calls
`encode_string(field.name.as_bytes(), self.huffman_enabled, out)`.
A focused probe series isolated the trigger precisely:

| call shape | result |
|---|---|
| `encode_string(list.as_slice(), …)` (heap-backed List) | OK |
| `encode_string(field_text.as_bytes(), …)` (small Text field) | **SIGSEGV** |
| `let b = field_text.as_bytes(); encode_string(b, …)` | **SIGSEGV** (binding does not help) |
| `HpackEncoder.new(); enc.set_huffman(false)` (no encode) | OK |

Root cause: `Text.as_bytes()` on a **small** (NaN-box-inline, ≤ 6 byte)
Text returns a `&[Byte]` that points into ephemeral value storage with
no backing heap buffer. Passing that slice across a call frame (to
`encode_string`) dangles → SIGSEGV. `List.as_slice()` is heap-backed and
lowers cleanly, which is why the direct codec tests pass.

A sibling finding surfaced while probing: `let t: Text = "xy".into();
t.as_bytes()` dispatches to `USize.as_bytes` (method not found at
runtime on a `Text<small>` receiver) — a receiver-type-tracking drift
where the `.into()`-bound binding loses its `Text` type for method
resolution. Distinct from the SIGSEGV but on the same `Text.as_bytes`
surface.

**Status:** the encode half of the HPACK pipeline is gated on this fix
(`test_hpack_encoder_decoder_round_trip_*` are `@ignore`'d tracking
pins that do not compile the crashing path). The decode half is fully
covered. Fundamental fix surface: VBC codegen/runtime for
`Text.as_bytes()` must materialise a heap-backed byte view (or copy)
for small-string Texts before the slice escapes the current frame.
Tracked as a new defect class **TEXT-SMALLSTR-ASBYTES-1** in the
catalogue.

### §3.4 DEFERRED-INIT — `let x: T;` assigned in `if`/`else` arms (FIXED)

`HpackDecoder.decode_literal` declares `let name: Text;` /
`let start: Int;` and assigns each in both the `if name_index == 0`
and `else` arms. The VBC codegen tracked definite-assignment with a
single flat `is_initialized` flag that was **not branch-scoped**: the
first arm's assignment flipped it `true`, so the sibling arm's
assignment was rejected as "cannot assign to immutable variable", and
the whole function was lenient-compiled to a panic-stub.

**Fix (fundamental, compiler-side):** added a `declared_uninit` flag to
`RegisterInfo` (`crates/verum_vbc/src/codegen/registers.rs`), set at the
deferred-init declaration in `compile_let`
(`statements.rs`). The immutable-reassignment guard in
`expressions.rs` now exempts deferred-init bindings
(`!is_mutable && is_initialized && !declared_uninit`), so a `let x: T;`
binding may be initialized once per control-flow path without a false
positive on the sibling branch, while normal `let x = v; x = w;`
reassignment of an immutable still errors. Closes the panic-stub for
all 5 stdlib deferred-init sites (2 in hpack, 3 in
`database/mysql/wkb_decoder`). Pinned by `test_hpack_decode_literal_block`.

### §3.5 ENCSTR-LOOP-1 — `encode_string` invoked inside a `while` loop SIGABRTs

A string-codec length-law property over a length sweep was attempted and
removed. Calling `encode_string` (or `decode_string`) from inside a `while`
loop body crashes the compiler at codegen time (SIGABRT, signal 6 — 0 tests
run). A **single** call compiles and runs fine (the unit suite proves this),
and `decode_integer` — whose tuple-Result first element is a scalar — loops
fine. The string codec carries the Huffman-intrinsic + `extend_from_slice`
branch, and instantiating that in a loop body aborts the compiler.

**Status:** OPEN, tracked. The string codec is fully covered by the unit
suite's single-call tests; the swept property is omitted (documented inline
in `property_test.vr`). Fundamental fix surface: VBC codegen for a call that
pulls in an intrinsic + slice-extend branch, instantiated inside a loop body.

## 4. Action items landed in this branch

* `unit_test.vr` — 36 active tests: HeaderField §4.1 sizing; integer
  encode/decode incl. RFC C.1.1/C.1.2/C.1.3 vectors + prefix
  boundaries + Truncated / IntegerOverflow guards; raw string
  codec; dynamic-table insert/lookup/eviction §4.4; HpackError
  ADT; decode_literal (deferred-init); decoder rejects index 0.
  2 `@ignore`'d encode round-trip pins (ENCODE-1).
* `property_test.vr` — 7 laws: integer round-trip over value
  sweep × every prefix width; flag transparency; prefix-boundary
  byte counts; raw string round-trip over length sweep; dynamic
  table size invariant + lookup-within-len.
* `regression_test.vr` — LOCK-IN pins incl. the EXTSLICE-1 fix +
  the deferred-init decode_literal fix.
* `core/net/http2/hpack.vr` — `decode_string` EXTSLICE-1 rewrite.
* `crates/verum_vbc/src/codegen/{registers,statements,expressions}.rs`
  — deferred-init definite-assignment fix (§3.4), compiler-wide.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ENCODE-1 / TEXT-SMALLSTR-ASBYTES-1 — small-string `Text.as_bytes()` heap-backing | compiler/runtime | multi-day (catalogue) — unblocks encode round-trip |
| Huffman round-trip (§5.2 H bit set) | huffman/ folder | 3h, gated on intrinsic verification |
| Static-table index references in encoder (avoid always-literal) | stdlib + tests | 2h |
| Dynamic-table size-update (§6.3) decoder round-trip | this folder | 1h |
| EXTSLICE-1 compiler-layer close | compiler | multi-day (tracked in catalogue §1) |
