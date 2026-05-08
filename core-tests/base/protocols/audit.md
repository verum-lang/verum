# Audit — `core/base/protocols.vr`

> Implementation audit consolidating cross-stdlib protocol consumers (§1),
> hardcoded protocol handling in the Rust crates (§2), language-impl drift
> surfaces (§3), and vestigial APIs (§4). Severity-ranked action items in §5.

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/protocols.vr` (1061 lines, ~50 protocols) |
| Tests | `core-tests/base/protocols/` — `unit_test.vr` (1685, migrated), `try_protocol_agnostic_test.vr` (348, migrated), `property_test.vr` (NEW, ~470 LOC, algebraic laws), `integration_test.vr` (NEW, ~250 LOC) |
| Hardcodes in `crates/` | 4 categories, ~60 sites; 1 critical (primitive-impl matrix), 1 medium (operator fast-paths), 2 benign |

## §1  Protocol-name hardcode inventory (Rust)

The string names of protocols are referenced from many compiler crates. The
**central registry** is sound:

| File | Role |
|---|---|
| `crates/verum_common/src/well_known_types.rs` (`WellKnownProtocol` enum, ~lines 551–646) | Canonical name ↔ enum mapping |
| `crates/verum_types/src/operator_protocols.rs` (`OperatorProtocols::standard()`) | Operator → protocol-name table |
| `crates/verum_compiler/src/derives/{clone,hash,debug,eq,ord,default}.rs` | Per-protocol derive logic |
| `crates/verum_compiler/src/core_cache.rs:1190,1213` | Stdlib-derive list |

Subordinate consumers reference the central registry, so name drift is
contained. **Keep the central registry; document its invariants.**

## §2  Drift surfaces

### 2.1  CRITICAL — Primitive-implements-protocol matrix is hardcoded

`crates/verum_common/src/well_known_types.rs:718-778` —
`primitive_implements_protocol(type_name, protocol_name) -> Option<bool>`
hardcodes which primitives satisfy which protocols:

| Primitive | Protocols hardcoded as implemented |
|---|---|
| `Int`     | Copy, Clone, Eq, Ord, Hash, Default |
| `Float`   | Copy, Clone, Default (NOT Eq/Ord — NaN) |
| `Bool`    | Copy, Clone, Eq, Ord, Hash, Default |
| `Char`    | Copy, Clone, Eq, Ord, Hash (NO Default) |
| `Text`    | Clone, Eq, Ord, Hash, Default (NOT Copy — heap) |
| `Unit`    | Copy, Clone, Eq, Ord, Hash, Default |

The function's own docstring acknowledges:

> Note: This is intentionally hardcoded because primitive types are part of
> the language definition, not the standard library. Their protocol
> implementations cannot be discovered from source — they are axioms of the
> type system.

That justification is legitimate — primitive protocol satisfiability is a
language-level axiom, not derived from `.vr` source. **However**, the
hardcode is still a drift surface in a different sense:

1. If anyone adds a new primitive (e.g. `UInt128`, `Decimal`), they must
   update this table or the new primitive silently fails protocol-bound
   generic code.
2. If anyone adds a new protocol (e.g. a future `Atomic`-like marker), they
   must walk every primitive arm and decide.
3. There's no test that *exercises every (primitive × protocol)* cell, so
   typos in the table are silent.

**Mitigation landed:** see `core-tests/base/protocols/protocol_matrix_test.vr`
(NEW) — a single test that calls `primitive_implements_protocol` for every
documented (type, protocol) cell and asserts the value matches the table
above. Drift in either direction (cell flipped from `true` to `false` or
vice versa) fails the test.

### 2.2  MEDIUM — Operator fast-paths bypass protocol dispatch for primitives

The audit traced operator dispatch through three layers:

1. **Type-checker** (`verum_types/src/operator_protocols.rs`): maps
   `BinOp::Add` → protocol name `"Add"` → method `add`. Looks up via
   metadata.
2. **VBC codegen** (`verum_vbc/src/codegen/expressions.rs`): may emit a
   direct opcode (`OpcodeAdd`) bypassing protocol dispatch entirely for
   `Int + Int`.
3. **LLVM lowering**: emits direct `add i64` instruction for the same.

When the operand is a user type implementing `Add`, the protocol path is
taken; when it's a primitive, the fast path. **Drift surface:** edits to
`Add` / `Sub` / `Mul` / `Div` in `protocols.vr` (e.g. changing the method
name from `add` to `plus`) would break the user-protocol path but the
primitive fast path would silently keep working — so the regression
manifests only on user types and only at the call site, not where the
protocol was edited.

**Recommendation (deferred):** add a CI assertion that
`OperatorProtocols::standard()` references each method by a name that
exists in the corresponding protocol's metadata-loaded methods at startup.
Cross-cutting; not landed in this branch.

### 2.3  MEDIUM — Reverse `method_to_protocol` table is hardcoded too

`well_known_types.rs:686-701` — `method_to_protocol("hash") = Some(Hash)`,
`method_to_protocol("clone") = Some(Clone)`, etc. — a small reverse-lookup
table.

If `protocols.vr` renames `Clone.clone` to `Clone.duplicate`, this table
silently mismatches. Same drift class as 2.2; lower frequency; same
mitigation pattern (CI assertion against metadata at startup).

### 2.4  LOW (benign) — Marker-protocol auto-derivation

`Send` / `Sync` / `Copy` / `Sized` are markers (zero methods) and are
*allowed* to be hardcoded as language axioms. The marker contract is
"no methods" — if `protocols.vr` ever adds a method to a marker, the
hardcode would silently lose that method. Mitigation: enforce
"marker protocols have zero methods" as a stdlib-level invariant, possibly
via a stdlib-load-time check.

## §3  Try / FromResidual — special handling

The `?`-operator codegen depends on `Try` and `FromResidual`. These
protocols are **not** declared in `protocols.vr` — they live in
`core/base/ops.vr` (re-exported by `core/base/mod.vr` prelude).

The audit subagent initially flagged this as a *gap* in `protocols.vr`,
but inspecting `mod.vr:92-99` confirms the exports come from `ops`, so the
language has them — they're just located in the operations module, not the
protocols module. **No defect.** Worth noting in the audit so future
readers don't repeat the false alarm.

`Try` is consumed by codegen via the string `"Try"` in
`operator_protocols.rs:483`. Drift class same as 2.2.

## §4  Vestigial / under-consumed protocols

Protocols defined in `protocols.vr` whose consumer-set is empty or
near-empty. Each is a candidate for removal, demotion to
`core/experimental/`, or improved documentation explaining the intended
use:

| Protocol | Lines | Usage findings |
|---|---|---|
| `Zero` | 609–612 | No stdlib consumer found; mentioned in `stdlib_coercion_registry.rs:143` as a future feature (*"once `Numeric` protocol query lands"*) |
| `One` | 615–618 | Same |
| `Numeric` | 732 (approx) | Marker; mentioned in coercion registry but unconsumed |
| `SignedInteger` | (export site `mod.vr:163`) | Same |
| `Integer` | (export site `mod.vr:170`) | Same |

**Action item (deferred):** trace each of these to find any actual
consumer; if zero, demote with a note. They aren't actively harmful but
clutter the protocol surface and confuse the "what protocols matter?"
question for users.

## §5  Action items landed in this branch

- [x]  Migrate `vcs/specs/core/core/protocols_test.vr` →
       `core-tests/base/protocols/unit_test.vr`
- [x]  Migrate `vcs/specs/core/core/protocol_try_agnostic_test.vr` →
       `core-tests/base/protocols/try_protocol_agnostic_test.vr`
- [x]  Add `property_test.vr` — algebraic-law verification for Eq / Ord /
       PartialOrd / Hash / Clone / Default / From-Into / Add-Sub-Mul-Div /
       BitAnd-BitOr-BitXor-Not / Shl-Shr / Length
- [x]  Add `integration_test.vr` — protocols meeting List.sort, Map insert+lookup,
       Set membership, Display/Debug formatter, From/Into reflexive,
       derive on user record, generic-over-extension-chain
- [x]  Add Rust-level unit test pinning the primitive-implements-protocol matrix
       (`crates/verum_common/src/well_known_types.rs::tests::primitive_protocol_matrix_pinned`)
       — closes §2.1 silent-drift surface
- [x]  Add this audit document

## §6  Action items deferred (not landed)

1. CI assertion that operator-protocol method names in
   `verum_types/src/operator_protocols.rs` exist in the corresponding
   protocol's loaded metadata (closes §2.2 + §2.3).
2. `method_to_protocol` reverse table validation against metadata at startup
   (closes §2.3).
3. Stdlib-load-time check that marker protocols (`Send`, `Sync`, `Copy`,
   `Sized`, `Unpin`) have zero methods (closes §2.4).
4. Vestigial-protocol audit pass — trace `Zero`, `One`, `Numeric`,
   `SignedInteger`, `Integer` consumers; demote/document/remove
   (closes §4).
5. Try / FromResidual location: leave as-is (in `ops.vr`); add a
   cross-reference comment in `protocols.vr` so future readers don't
   re-flag this.
