# `net/addr` audit

Module: `core/net/addr.vr` (~1016 LOC) — IP address types
(Ipv4Addr / Ipv6Addr / IpAddr) + socket addresses
(SocketAddrV4 / SocketAddrV6 / SocketAddr) + `ToSocketAddrs`
protocol + parse + RFC-conformant predicates.

Tests cover the full algebraic surface across construction,
canonical addresses, classification predicates (RFC 1918 / 5735 /
5771 / 4291), to_u32 / from_u32 round-trip, Ipv4 + Ipv6 parsing
(happy + error paths), IpAddr discriminator, SocketAddr.parse,
and `AddrParseError` variant lattice.

Sister tests for `ToSocketAddrs` (the protocol's `to_socket_addrs`
method requires DNS resolution against a fixture) are deferred
to `vcs/specs/L2-standard/net/` where DNS-mock harness lives.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.{http,http2,http3}` | server bind addresses, client targets. |
| `core.net.dns` | A/AAAA record values. |
| `core.net.cidr` | network masks built on IP types. |
| `core.net.tcp` / `core.net.udp` / `core.net.unix` | every bind / connect uses an IP address. |
| `core.mesh.xds` | Envoy listener filter-chain addresses. |
| Application networking | every socket bind/connect call. |

## 2. Crate-side hardcodes

`crates/verum_runtime/src/net/...` BSD-socket FFI consumes the
4-byte big-endian wire form. Pinned by `test_to_u32_*` tests.

The `SocketAddr.V4(...)` qualified-constructor form (instead of
bare `V4(...)`) is documented as a VBC codegen workaround for
nested-record-argument miscompilation (tracked as #76 in source).
See `addr.vr:702-713`. The conformance suite calls only the
qualified form via the `SocketAddr.new_v4` / `new_v6` factory
methods, so the surface is durably tested through the canonical
client path.

## 3. Language-implementation gaps

### §3.1 `Ipv4Addr.parse` workaround for codegen bug #78

Source comment at `addr.vr:137-142` documents a codegen bug where
`&parts[i]` panics with "Slice index out of bounds". Worked around
via let-binding. Same VBC codegen family as
[[btree_pattern_match_ref_generic_class]]. Tested through the
working-on-workaround path; the underlying defect is a multi-day
VBC codegen fix.

### §3.2 `SocketAddr.parse` Char-vs-Text + Result.map_err
workarounds for codegen bug #78 / #79

Three workarounds documented in source at `addr.vr:760-808`:
1. `rsplit_once(":")` Text literal instead of `':'` char literal
   — char auto-promotion takes a different codegen branch.
2. Explicit `match` instead of `.map_err(|_| ...)?` chain —
   Result.map_err method-resolution fails when transitive
   `core.base.result` import is missing.
3. Explicit `&host` reference instead of relying on auto-borrow.

Tested through the working canonical client path. Source-side
workaround durability pinned by SocketAddr.parse error-path tests.

### §3.3 SocketAddr-variant nested-record miscompile (#76)

Documented at `addr.vr:702-713` — bare `V4(...)` instead of
`SocketAddr.V4(...)` miscompiles nested record argument as the
inner record's first FIELD value (object size 8 instead of 16).
This is the **same defect class** as
[[btree_pattern_match_ref_generic_class]] +
[[enactment_field_access_oob_2026-05-24]]: codegen loses record
layout through cross-module / variant-payload pathways and
defaults to 8-byte scalar.

The qualified form `SocketAddr.V4(...)` works because the
resolver dispatches through the constructor symbol that user
code uses. Source-side discipline is durable so long as
contributors use the `SocketAddr.new_v4` / `new_v6` factories,
which the conformance suite exclusively exercises.

### §3.4 `ToSocketAddrs` protocol — type-Iter associated bound deferred

Source comment at `addr.vr:861-869` documents that the bound
`Iterator<Item = SocketAddr>` would express "yields SocketAddrs"
properly, but the typechecker doesn't yet enforce
associated-type bindings on protocol-bounded generics (#75). The
prior form `Iterator<Item>` was a no-op (Item unbound), so the
bound is dropped entirely until #75 lands. **All three impls
already use `Iter = List<SocketAddr>`** uniformly to sidestep
the impl-method-dispatch codegen failure documented at
`addr.vr:874-881`.

Effort to add language-level fix: multi-day, gated on #75.

## 3.5 Cross-tier `--aot` audit — 2026-06-20 (clean run)

`net/addr` was historically validated under `--interp` only; the net
module page declared "Cross-tier `--aot` validation deferred". This
session ran the full suite under `--aot` in a quiesced environment to
enumerate the genuine codegen defects. **Result: 95 passed / 43 failed
/ 138 total.** (An earlier contended run reported garbage — concurrent
`cargo build` + `~/.verum/script-cache` wipes caused compile races and
CPU-starvation timeouts. Always run `--aot` measurements with a quiet
machine.)

The 43 genuine AOT failures partition into **four** language-level
codegen / type-resolution defect classes — each surfaced by this
folder's tests, each with a minimal standalone reproducer:

### §3.5.1 TUPLE-EQ-AOT — tuple `==` always returns `true` (task #4)

Under AOT, `tupleA == tupleB` returns `true` even for **distinct**
tuples (interp is correct). Minimal repro: `(127,0,0,1) == (0,0,0,0)`
prints `true` under `verum build`, `false` under `--interp`.

**Exact root cause (located 2026-06-20):** a tuple literal compiles to
VBC `Pack` → LLVM `runtime.lower_pack` returns a heap **pointer**
(`instruction.rs:3866`). `a == b` emits `CmpG` →
`lower_cmp_generic` (`instruction.rs:29165`). Its
`lhs.is_pointer_value() && rhs.is_pointer_value()` branch
(`instruction.rs:29374`) **unconditionally assumes both pointers are
`Text` objects** and runs `verum_text_get_ptr` + `strcmp`. A tuple
pointer is thus misread as a Text and strcmp'd over garbage, returning
a bogus (consistently `true`) result. Field reads are unaffected
(`is_loopback`'s `octets.0 == 127` **passes** AOT) — only the
whole-tuple compare is corrupted. The defect generalises to **every
non-Text heap-object `==`** (tuples, and likely records/other
aggregates) under AOT.

**FIX LANDED (2026-06-20, validated both tiers).** The heap object
header is NOT initialised by `lower_pack` (`emit_checked_malloc` only
mallocs), so a runtime structural-eq cannot read the arity from the
header. Instead the fix is at **VBC codegen** where the arity is
statically known: `compile_binary` now intercepts `==` / `!=` whose
operands are tuples (arity fixed by a tuple-literal operand — `==`/`!=`
are type-checked so both sides share it) and emits **element-wise**
comparison — `Unpack` both tuples into consecutive registers, `CmpG`
each pair (tuple ELEMENTS are i64 values → the correct value-compare
path on both tiers, never the broken whole-tuple pointer branch),
bitwise-AND the results, and for `!=` negate via `acc == 0` (a literal
0 + `CmpI Eq`, since `Instruction::Not` is a *bitwise* complement at
Tier-1: `!0 == -1`). Tier-agnostic (interp stays 138/138) and strictly
more correct than the whole-tuple `CmpG` it replaces. See
`verum_vbc .../codegen/expressions.rs` (`tuple_eq_arity` +
`emit_tuple_elementwise_eq`).

**Result:** the 8 tuple-eq tests below flipped GREEN under `--aot`
(`is_unspecified`/`is_broadcast` Ipv4 + `is_loopback`/`is_unspecified`
Ipv6 + their negated/disjoint/property dependents); 0 regressions;
AOT 95→**103/138**. The whole-pointer `lower_cmp_generic` Text-strcmp
branch (`instruction.rs:29374`) remains latent for non-tuple non-Text
heap objects (records compared whole) — tracked for the structural
`verum_value_eq` follow-up; tuples no longer reach it.

Failures pinned: `is_unspecified` / `is_broadcast` (Ipv4) and the
Ipv6 `is_unspecified` / `is_loopback` all use `self.octets == (..)`
/ `self.segments == (..)`. Signature: **positive** assertions pass,
**negated** (`!is_X`) assertions fail (because the wrong-`true` flips
the negation). Direct hits: `test_is_not_unspecified_localhost`,
`test_is_not_broadcast_subnet_max`, `test_ipv6_is_not_loopback_other`,
`test_ipv6_is_not_unspecified_one`, `prop_unspecified_unique`,
`prop_multicast_disjoint_broadcast`, both
`integration_ip_addr_*_loopback_and_unspecified_disjoint`.

### §3.5.2 DISP-EMPTY-AOT — f-string Display of user types → empty (task #3)

Under AOT, `f"{x}"` where `x` is any user/stdlib type with a `Display`
impl produces an **empty** string; primitives (`f"{42}"`) work.
Isolated repro: a `type Tag is {n:Int}` whose `Display::fmt` is just
`f.write_str("LITERAL")` prints `a=[]` under AOT vs `a=[LITERAL]`
under interp. The VBC→LLVM `ToString`/InterpolatedString lowering
does not dispatch to the user `Display::fmt` (or discards its `Text`
result) for non-primitive operands — same family as the
`Text.to_text` AOT zero-stub.

Failures pinned: all of Section 23 (`test_*_display_*`, 10 tests).
Renders `Ipv4Addr`/`Ipv6Addr`/`IpAddr`/`SocketAddr` as `""` under AOT.

### §3.5.3 PRELUDE-FREEFN — prelude free fns unbound under AOT/run (task #2)

`f"{x:?}"` lowers to the prelude free fn `format_debug(&x)`, which is
**unbound** at type-check (`E100: unbound variable: format_debug`)
under both AOT test compilation and standalone `verum run`. A single
`:?` test poisons the **entire** test file's AOT compile — masking
every other test in the file (an earlier run showed all 115 unit
tests "failing" from one `:?`). Root cause: the precompiled metadata's
`module_reexports["core.prelude"]` captures only the `super.base.*`
glob (with the glob-root `core.base` as source, so even those don't
resolve to their submodule functions); the prelude's **concrete**
named mounts (`super.text.format.format_debug`,
`super.io.read_to_string`, …) are not captured at all, despite
`precompile.rs::scan_module_reexports`'s `Path`-arm that should
capture them. The lazy type-env (`new_with_core`) therefore never
binds the bare names. **Mitigation applied here:** the suite avoids
`:?` (Display is tested via ToString instead) so the file's other
AOT tests can compile — see Section 23 note. A consumer-side
type-env registration was prototyped (`register_prelude_free_
functions_from_metadata`) but the precompile-capture side must land
first; reverted pending that.

### §3.5.4 PARSE-AOT — Ipv4/Ipv6/SocketAddr parse diverges (task #5)

`Ipv4Addr.parse` / `Ipv6Addr.parse` / `SocketAddr.parse` produce
wrong results under AOT (interp correct) — ~23 of the 43 failures.
The parse code leans on `Text.split`/`.slice`/`.rfind`/`.chars` +
`List` indexing + `[0;8]` arrays + tuple destructuring; the §3.1/§3.2
interp-era workarounds (#78/#79) do not hold under AOT, and some
failures are downstream of TUPLE-EQ-AOT (parse builds an address,
then a predicate compares tuples). Needs per-primitive text-codegen
root-cause under LLVM.

### Pass/fail summary (`--aot`, 2026-06-20)

| Class | Count | Tier-0 | Tier-1 (AOT) |
|---|---:|---|---|
| Construction / field accessors | ~30 | ✓ | ✓ |
| Scalar predicates (`is_loopback`/`is_private`/`is_multicast`) | ~20 | ✓ | ✓ |
| `to_u32`/`from_u32` round-trip | 8 | ✓ | ✓ |
| Tuple-eq predicates (`is_unspecified`/`is_broadcast`) | ~10 | ✓ | ✓ §3.5.1 (FIXED) |
| Display rendering | 10 | ✓ | ✗ §3.5.2 |
| Parse (v4/v6/socket) | ~23 | ✓ | ✗ §3.5.4 |
| Debug (`:?`) | 0 (removed) | ✓ | ✗ §3.5.3 |

After the TUPLE-EQ-AOT fix (§3.5.1), AOT is **103/138**. The 35
remaining Tier-1 failures are Display rendering (§3.5.2) + parse
(§3.5.4). The 103 that pass both tiers are construction, scalar +
tuple-eq predicates, accessors, `to_u32`, and `AddrParseError` Eq.

## 4. Action items landed in this branch

* `core-tests/net/addr/unit_test.vr` — 95 unit tests covering
  Ipv4Addr (28) + Ipv6Addr (16) + IpAddr (8) + SocketAddrV4 (4)
  + SocketAddrV6 (2) + SocketAddr (19) + AddrParseError (6) +
  parse-error paths (12) across the full public surface.
* `core-tests/net/addr/property_test.vr` — 20 property tests:
  to_u32/from_u32 round-trip identity over canonical addresses
  + 256-element low-octet sweep; predicate disjointness
  (loopback ⊥ private, broadcast ⊥ private, multicast ⊥
  broadcast); RFC 1918 boundary lattices (10/8, 172.16-31/12,
  192.168/16); multicast 224-239 boundary; Ipv6 predicate
  exclusivity (loopback ⊥ multicast, link-local ⊥ unique-local);
  SocketAddr V4 XOR V6 + port preservation sweep;
  AddrParseError 3×3 disjointness matrix.
* `core-tests/net/addr/audit.md` — this file.

### Session 2026-06-20 — cross-tier `--aot` close-out

* **Test bug fixed** — `test_ipv6_is_not_link_local_fe90` asserted
  `0xfe90` is NOT link-local, but `fe80::/10` spans `fe80..=febf`
  (top 10 bits `1111111010`), so `0xfe90` **is** link-local. The
  source impl `(seg0 & 0xFFC0) == 0xFE80` is correct per RFC 4291
  §2.5.6. Replaced with three boundary tests (`fe90` in-block,
  `fec0` above, `fe40` below).
* **Display coverage added** (Section 23, 10 tests) — Ipv4 dotted-
  decimal, Ipv6 uncompressed lowercase-hex groups, IpAddr forward,
  SocketAddrV4 `ip:port`, SocketAddrV6 bracketed `[ip]:port`.
  Tier-0 green; pins DISP-EMPTY-AOT (§3.5.2) on Tier-1.
* **`:?` Debug test removed** — it lowered to the prelude free fn
  `format_debug`, unbound under AOT, poisoning the whole file's
  Tier-1 compile (§3.5.3). Removing it recovered ~68 unit tests
  under `--aot` (27 → 95 passing). Debug-format coverage is
  intentionally deferred until PRELUDE-FREEFN (task #2) lands.
* **Four AOT defect classes root-caused** with minimal reproducers
  (tasks #2–#5) — see §3.5. Tier-0: 139/139 green. Tier-1: 95/138.
* **TUPLE-EQ-AOT FIXED** (§3.5.1) — `compile_binary` now lowers tuple
  `==`/`!=` element-wise in VBC codegen (`tuple_eq_arity` +
  `emit_tuple_elementwise_eq`), bypassing the `lower_cmp_generic`
  pointer-pointer Text-strcmp misclassification. 8 tuple-eq predicate
  tests flipped GREEN under `--aot`; Tier-0 still 138/138; Tier-1
  95→**103/138**; 0 regressions.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| ~~**TUPLE-EQ-AOT** (task #4)~~ — **CLOSED 2026-06-20**: tuple `==`/`!=` now lowered element-wise in VBC codegen; AOT 95→103/138, 0 regressions. (Latent: whole-record `==` still hits the Text-strcmp branch — follow-up `verum_value_eq`.) | done | ✅ |
| **DISP-EMPTY-AOT** (task #3) — f-string Display of user types → empty under AOT | `verum_codegen` (ToString→Display dispatch) | high-value, stdlib-wide |
| **PRELUDE-FREEFN** (task #2) — prelude concrete free fns not captured into metadata `module_reexports`, unbound bare under AOT/run | `precompile.rs::scan_module_reexports` + `verum_types new_with_core` | medium; precompile capture + type-env registration |
| **PARSE-AOT** (task #5) — v4/v6/socket parse text-codegen diverges under AOT | `verum_codegen` (Text split/slice/chars) | partly downstream of #4 |
| `ToSocketAddrs` protocol coverage (host:port DNS path) | this folder | gated on DNS mock harness (vcs/specs/L2-standard/net/) |
| Display round-trip ∀a. parse(a.to_string()) == Ok(a) | this folder | gated on #3 + #5 |
| Sister coverage for `core.net.{cidr,ipv6_canonical,dns,link_header}` | sister folders | tracked as separate INVENTORY rows |
