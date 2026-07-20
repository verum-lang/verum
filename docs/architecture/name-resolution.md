# Verum Name Resolution — Reference Architecture

Status: NORMATIVE design (2026-07-21). This document is the single source of
truth for how names resolve in Verum. It was produced from a measured defect
inventory (every claim below was reproduced on the shipped toolchain, task IDs
given) plus a survey of prior art, and it defines the model the implementation
migrates to. The migration stages and their gates are at the end.

## 1. Why this document exists — the measured defect inventory

One architectural deficiency — *name resolution by flat global tables with
lenient fallbacks* — produced every one of these, found and reproduced in one
audit pass:

| # | Measured defect | Task |
|---|---|---|
| 1 | Variant constructors share one flat bare-name table with types; a variant ctor wins `Name.member` **even against an explicit `mount`**. With 2 aliases + 14 variants all spelled `IoError`, the winner was compress's variant ctor: `IoError.WouldBlock` → `E103 non-record type: fn(Text) -> CompressError`. | T0525 |
| 2 | A cross-module type alias loses its identity: the archive's per-module `TypeId→name` table renders the target `__opaque_type_N`, which becomes a fresh inference variable — so `mount core.io.{IoError}; fn f(x: IoError) -> Int` **accepts `f(42)`**. A soundness hole, not an ergonomic bug. | T0525, T0533 |
| 3 | `mount m.{NonExistent}` is **silently dropped** — the program compiles and runs. 52 of 234 sqlite specs mount names that do not exist and report green. | T0528 |
| 4 | Payload-less access to a **nonexistent variant** fabricates a tag: `Http2Error.ZzzNotAVariant` → `MakeVariant { tag: 58770 }` (hash-derived). The called form *is* diagnosed ("Unknown variant constructor … Available variants") — only the payload-less form slips through. 17 phantom members found in one file. | T0545, T0524, T0532 |
| 5 | Qualified access ignores the qualifier when the member is not the qualifier's: `Http2Error.IoError` and `ErrorCode.IoError` lowered to the **same** `func_id` — the bare key drove resolution, the receiver type did not. | T0525, T0545 |
| 6 | Re-export chains resolve **one hop only** (`module_reexports` is followed once), so replacing a definition with a re-export breaks every parent-umbrella `public mount .Leaf.{T}` chain. | dedup trap, all consolidation tasks |
| 7 | Free-function bare names resolve by registration order: last-registration-wins (T0114/T0144) or lowest-arity-first-wins (T0448). The class has already fired in live code: `l4_vdbe/cursor_btree_bridge.vr` carries a `cursor_new as btree_cursor_new` workaround alias because AOT picked a 0-arg `cursor_new` from an *unreachable* module. | T0114, T0144, T0448, T0538 |
| 8 | The runtime's bare-suffix dispatch scan picks the alphabetically/iteration-order first `*.method` when the receiver type cannot be recovered — non-deterministic mis-dispatch (`Bool.hash` for an Int receiver, etc.). | T0106 family |

Aggravator: stdlib function *bodies* are typechecked on **no path** (the bake
has no inference pass over `core/` bodies; `VERUM_STDLIB_PATH` overrides only
the registry), so all of the above stayed invisible inside `core/` (T0124).

The pattern across all eight: **resolution is attempted late, by name-shaped
guessing, with a lenient fallback instead of an error**. The fix is one
architecture, not eight patches.

## 2. Prior art consulted

* **Rust**: separate namespaces (types / values / macros); rib-based lexical
  scoping; explicit imports shadow glob imports; ambiguity inside one rib is a
  hard error with candidates; `pub use` re-exports resolve transitively.
* **ML-family module systems** (SML/OCaml): module paths are resolved
  structurally; an alias is transparent and identity-preserving.
* **Zig/Go**: qualified access `Q.m` never falls back to an ambient table —
  `m` must be a member of `Q`.

Verum adopts the Rust namespace/rib discipline, the ML alias transparency, and
the Zig/Go strict-qualifier rule.

## 3. The model

### 3.1 Three namespaces

Every declaration enters exactly one namespace:

| Namespace | Populated by |
|---|---|
| **module** | `module` declarations, mounted module paths |
| **type** | `type` declarations (records, sums, protocols, aliases, newtypes) |
| **value** | functions, constants, **variant constructors**, context items |

A name may exist in several namespaces simultaneously without conflict
(`JsonObject` the variant ctor and a hypothetical `JsonObject` type would no
longer collide — though stdlib style still avoids such doubles). Syntactic
position selects the namespace: type position consults the type namespace,
expression position the value namespace, path heads the module namespace
first.

### 3.2 Scope precedence (ribs)

Bare-name lookup walks ribs from innermost to outermost; the **first rib
containing the name wins**, and two candidates *inside the same rib* are a
loud ambiguity error listing both:

1. local bindings (`let`, `match` arms, closures)
2. function parameters and generic parameters
3. **explicit imports** — `mount m.{A, B as C}`
4. module-local declarations
5. glob imports — `mount m.*`
6. prelude / ambient (includes variant constructors of in-scope types —
   this is what keeps bare `Some` / `Ok` / `Err` working)

Consequence: an explicit `mount` **always** beats an ambient variant
constructor — defect #1 becomes impossible by construction. This rule is
already stated twice in the codebase (ctor-shadow eviction;
MOUNT-TYPE-AUTHORITY-1); the model makes it universal.

### 3.3 Strict qualified access

`Q.m` resolves `Q` first (module namespace, then type namespace, then a
value in scope for method calls). Then `m` is looked up **only among Q's
members**:

* `Q` is a module → `m` among the module's public items;
* `Q` is a sum type → `m` among its variants and associated functions;
* `Q` is any type → `m` among associated functions and constants.

If `m` is not a member of `Q`, resolution **fails loudly**, naming `Q`'s
actual members. There is no fallback to any global bare-name table. This
kills defects #4 and #5 (fabricated tags, receiver-ignoring capture) and the
runtime's suffix scan (#8) loses its compile-time feeder.

### 3.4 Totality: every failure is a diagnostic

Resolution is a total function: every name either resolves to exactly one
binding or produces a typed error. Concretely, these lenient behaviours are
**removed**:

* silently dropping an unresolved `mount` item (→ E-code + did-you-mean);
* fabricating a variant tag for a nonexistent member (→ the same
  "Unknown variant constructor … Available variants" diagnostic the called
  form already emits);
* first/last-registration-wins and lowest-arity-first-wins selection among
  same-named free functions (→ ambiguity error unless disambiguated by
  qualification or arity-unique signature);
* runtime first-suffix-match dispatch as a *silent* fallback (it may remain
  as a diagnostic-carrying last resort during migration, but every hit logs
  a loud warning naming the guess).

### 3.5 Aliases preserve identity

`type A is B;` is a **transparent alias**: `A` denotes B's `TypeId`, layout,
associated functions, protocol impls — across module boundaries. The archive
therefore carries an **archive-wide** `TypeId → qualified-name` index (not
per-module), so an alias target can never render `__opaque_type_N`.
(`type A is (B);` — the newtype form — remains nominal and distinct, as
today.) This closes the soundness hole (#2).

### 3.6 Transitive re-exports

`public mount` chains resolve to a fixpoint (with cycle detection) computed
once per module graph. The "one hop only" limit (#6) is deleted; a definition
may be replaced by a re-export without breaking any umbrella.

### 3.7 One authority, carried facts

Resolution runs **once**, in the resolver, producing a binding table
(`NodeId → BindingId`). Inference consumes binding ids; codegen consumes
binding ids; the runtime dispatch tables are keyed by binding-derived ids.
Codegen never re-derives a target from a name string. This is the carried-fact
contract that already governs other subsystems (field slots, call ids) applied
to names — and it is what makes defects #7/#8 structurally impossible rather
than individually patched.

## 4. Migration stages (each gated by build + bake + pins)

| Stage | Content | Gate |
|---|---|---|
| **A** | Scope precedence in the existing resolvers (T0525 legs 1–2: explicit-import wins over ambient ctor; the lexical guard reads real type defs, never alias fallbacks) + archive-wide TypeId index (leg 3) + codegen alias hop (leg 4). | 12 repros + 4 controls in the T0525 journal; `VERUM_TRACE_MOUNT_AUTH` / `VERUM_TRACE_CTOR` pins; full bake identical module/fn census. |
| **B** | Totality: loud unresolved-`mount` (T0528), loud nonexistent-variant for the payload-less form (T0545). Census first: any code relying on lenient mounts of cfg-gated items must be counted before the switch. | The 52 vacuous sqlite specs flip red-or-fixed; a new negative-control spec pins each diagnostic. |
| **C** | Transitive re-exports replace the one-hop walk. | The consolidation-wave umbrellas (`Ordering`, `Drop`, `JsonMap`, …) resolve through two hops; dedup patches drop their umbrella-re-point hunks. |
| **D** | Free-function ambiguity: replace last-registration/lowest-arity-wins with qualified-or-unique resolution + loud ambiguity (T0448 canonical_index reader-flip is the first increment). | The 113-key collision census (T0538) becomes a make gate: every key either qualified or unique. |
| **E** | The binding-table refactor (`NodeId → BindingId` carried into codegen), retiring string re-derivation and the runtime suffix scan. | Grep-gate: no `format!("{}.{}", …)` name synthesis in dispatch paths; runtime scan hits = 0 on the conformance suite. |
| Gate for all | `verum check --stdlib` (T0124) so `core/` bodies are checked by the same resolver users get. | The stdlib body-check runs in CI blocking mode. |

Stages A and B are independent of each other and both land on the current
code; C–E build on A.

## 5. Compatibility notes

* Bare `Some` / `Ok` / `Err` / `None` keep working (prelude rib, §3.2).
* Existing explicit mounts keep their meaning — they only *gain* priority.
* Code that (accidentally) depended on an ambient ctor beating an explicit
  mount is, by the measured evidence, *already broken* in intent; stage A
  turns silent wrong behaviour into either correct behaviour or a loud error.
* The stdlib collisions already cleaned (IoError → one authority + `IoFailure`
  variants; `JsonMap`) remain good style — the model removes the *hazard*,
  not the value of distinct names.
