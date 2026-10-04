# Declaration-owned resource discipline and value use

Status: proposed engineering contract, 2026-10-04 (T1551). This is the
next bounded producer step for [the native lifecycle gap](native-drop-lifecycle-gap.md),
not an implementation of owned native `Drop`. No wire change or destructor
enablement is included. T1538 and T1540 remain open.

## Separate three facts

The existing checker name `ResourceKind::Copy` means *unrestricted use*;
`get_resource_kind` returns it whenever neither resource-name set matches.
It does not prove conformance to `core.base.protocols.Copy`.

| Fact | Authority | Meaning |
| --- | --- | --- |
| Usage discipline | Resolved type declaration | `Unrestricted`, `Affine` or `Linear`; missing declaration metadata is `Unknown` |
| Duplication capability | Resolved protocol implementation or an established structural value-copy rule | Whether and how another value may be produced: trivial value copy or exact clone operation |
| Cleanup obligation | Producer-selected value operation, plus exact declared glue | Which ownership obligation is transferred, created, borrowed or consumed |

These facts must remain separate. A `drop_fn` proves which destructor is
available, not that a particular register owns a value. A `clone_fn` does
not prove that its returned pointer names a fresh allocation.

### Existing declaration meanings

* `type affine R ...` permits at most one consuming use. Observation through
  a reference does not consume the owner. An unused affine local permits
  cleanup; the modifier alone does not prove that either tier performs it.
* `type linear R ...` requires exactly one consuming use on each reachable
  normal exit. `@must_consume` is already its attribute alias. The current
  checker exempts incoming parameters from the local must-consume-at-exit
  check, allowing `fn consume(r: R) {}`. Preserve that source rule in this
  step; it is not evidence that native parameter cleanup works. Panic and
  cancellation cleanup are separate lifecycle obligations.
* A known declaration with neither qualifier nor `@must_consume` is
  `Unrestricted`. This preserves ordinary value semantics. It does not
  manufacture an implementation of `Copy`, nor authorize arbitrary
  pointer duplication. `affine` plus `@must_consume` currently becomes
  linear; retain that stronger use constraint rather than losing the
  attribute on one producer route.
* `Copy` is a marker extending `Clone` in `core/base/protocols.vr`;
  `Clone` provides `clone(&self) -> Self`. Neither spelling is a resource
  qualifier. Resolve the canonical protocol and implementation, not a
  same-leaf user protocol. The GAT-bound checker mentions `Copy`/`Drop`
  incompatibility, but this is not a general declaration validator.
* Exact `Copy` on an affine/linear declaration, or a supposedly trivial
  copy with custom cleanup, cannot become ownership proof. Such a
  combination needs a declaration diagnostic before a new lifecycle
  consumer uses it. This note does not claim that validation already exists.

Do not encode unrestricted use as the wire value `Copy`. An explicit
`Unknown` state distinguishes old/missing metadata from a new declaration
whose unrestricted discipline is known. Old archives may continue through
existing compatibility paths, but must not acquire new duplication or
native-cleanup authority by defaulting `Unknown` to `Copy`.

### Shared values

`core/base/memory.vr` declares `Shared<T>` as an ordinary type with explicit
`Clone` and `Drop`. Its clone increments the strong count while retaining
one underlying allocation; each resulting owner owes its own release.
Thus two obligations may share an address. A transfer preserves one
obligation without retaining again. A borrow creates no owning obligation.
Pointer equality cannot deduplicate releases, and `Mov` alone cannot
select among these operations. Explicit clone remains possible for a
resource when its declared implementation produces a valid new ownership
obligation; an implicit consuming use must still obey affine/linear rules.

## One producer decision, reused by the existing pipeline

1. Resolve the defining declaration before classifying use. Use the
   resolver's declaring module and declaration identity, not the final
   path segment, import order, a bare alias or an unrelated numeric ID.
   Archive identity is `(declaring owner, source-local TypeId)` remapped
   into the consumer's namespace. Local shadowing follows ordinary
   resolution. A transparent alias delegates to its resolved target;
   unresolved targets remain unknown.
2. Preserve the declared qualifier and attribute on every parser route.
   The declared mode of a generic owner is not the effective mode of all
   instantiations: existing aggregate resource propagation must inspect
   resolved component types. Do not stamp a generic aggregate unrestricted
   merely because its outer declaration has no modifier.
3. For each resolved value use, publish `Borrow`, `Transfer` or `Copy` with
   the declaration identity and lexical value/binding identity. `Copy`
   must include the chosen duplication operation; `Transfer` moves the
   existing obligation; `Borrow` does neither. Explicit clone is a
   separately resolved call creating the declared clone result. An
   unresolved use remains unproved rather than guessed from register kind.
4. Reuse the checker-to-AST publication route already used for resolved
   call targets and Deref adjustments. Extend existing CFG/CBGR events to
   consume these facts; do not add another LLVM-only escape allowlist.
   A span locates syntax but is not an ownership identity. Registers may
   be reused, and each loop iteration may create a different obligation.
5. Only after source semantics are pinned, carry the declaration fact and
   resolved operation through VBC descriptors, versioned serialization,
   archive remapping and monomorphization. Missing data stays unknown.
   Both tiers must consume the same resulting lifecycle plan.

The first source units now preserve declaration identity and parser
metadata, with normal regression tests described below. The next unit must
carry the reviewed facts through archives. These source changes do not
invoke new native Drop glue.
Direct locals, by-value parameters and direct returns are the first use
sites; field/variant insertion, closure capture, branch joins and loops
need explicit transfer facts before their cleanup can be enabled.

## Source evidence for the first unit

The standalone [probe](../../crates/verum_types/tests/fixtures/resource_mode_contract_probe.rs)
uses parsed declarations and public `TypeChecker` registration APIs, with
no manually injected type map. It also compares the semantic parser with
`EventBasedParser` followed by `syntax_to_ast`. It is a diagnostic fixture,
not an ignored test or a passing ownership gate.

The two source modules are:

```verum
// alpha
public type affine Token is { id: Int };
// beta
public type Token is { pad: Int, id: Int };
```

The consumer is:

```verum
fn consume(x: beta.Token) {}
fn probe(x: beta.Token) { consume(x); consume(x); }
```

Before the source correction, measured on 2026-10-04: beta alone is accepted. Registering alpha as well
causes `MovedValueUsed` for beta, in either registration order. Replacing
beta with alpha correctly rejects the second consuming use. Borrowing
alpha twice and then consuming it once is accepted in both orders. This
isolates same-leaf resource contamination rather than a general failure
to enforce or borrow affine values.

The parser control is `type affine Affine is { id: Int };`, and likewise
`linear Linear`. The semantic parser retains each modifier. The event/sink
route reports no errors but returns no corresponding type declaration.
The unqualified ordinary declaration survives both routes. The fix must
preserve the whole declaration; merely filling `resource_modifier` in an
already-missing node is insufficient.

The probes were linked against one coherent existing private checker
build, without rebuilding the compiler. The examined resource tracker
and alternate-parser files are unchanged from that build's source to the
reviewed root snapshot. This is source-to-parser/checker evidence, not
public CLI, archive or runtime parity. A separate frozen public CLI check
of an unused linear local reported E303. Other public controls exceeded
the bounded time limit and have no acceptance verdict.

The normal parser suite now includes
`crates/verum_parser/tests/event_resource_declarations.rs`: qualifiers,
visibility, ordered outer attributes, malformed declaration diagnostics and
recovery are preserved. The checker suite includes
`crates/verum_types/tests/resource_declaration_identity.rs`: same-leaf
owners, borrowing, local shadowing, `@must_consume`, aliases and imported
generic aliases use the resolved declaration key. Source and imported aliases
retain their target's owner and generic parameter roster.

These are source-stage guarantees. The existing permissive treatment of an
unresolved qualified name still exists; absence of resource metadata is not
yet represented by the proposed `Unknown` state. Before wire work, add exact
source/serialized-import equivalence and unresolved-component handling. Before native
cleanup, add real timing/count controls for Shared releases, scope versus
explicit drop, borrowed/raw aliases, return and nested-variant transfers,
branches, register reuse and loops. The original mutex guard must stay
locked throughout its owning scope and unlock afterwards in both tiers.
