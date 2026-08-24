# Verum Registry — a federated protocol with a reference implementation

**Status**: design, owner-mandated 2026-08-25. Supersedes the
exploratory prototype under the internal registry tree (that tree is
reference material for domain vocabulary — TUF, sigstore, sumdb,
advisories, passkeys — not a starting point).

## 0. The one-sentence thesis

The registry is **a protocol plus a reference node**, not a service:
the same binary runs the official index and an enterprise's private
one, a client cannot tell which it is talking to, and every capability
the node has is **computed from its own code** by the compiler — the
architecture is a build artefact, not a diagram.

## 1. What this must demonstrate

The registry is the language's first large production programme, so it
carries a second obligation beyond working: every advanced capability
it uses must be there **because that capability is the best answer to a
real problem in this domain**, and must be visible as such in review.
Showcase-by-decoration is failure; showcase-by-necessity is the goal.

The mapping below is the contract for the implementation phase. Each
row names the domain problem first.

| Domain problem | Capability | Why it is the right answer |
|---|---|---|
| A version string that is not a version corrupts resolution for everyone downstream | **Refinement types** on `SemVer`, `PackageName`, `ContentHash` | the invariant lives in the type; parsing is the only place it can be violated, and it is checked there once |
| Publication must be exactly-once: a token, an upload slot, a transparency-log leaf may not be reused | **Linear / affine types** on `PublishToken`, `UploadSlot`, `LogLeaf` | the compiler refuses a second use; no runtime "already consumed" branch to forget |
| Handlers must not silently acquire ambient authority (a DB handle, a clock, the network) | **Contexts (`using [...]`)** for `Database`, `Clock`, `ObjectStore`, `UpstreamPeer` | authority is a parameter; a handler that needs the network says so in its signature, and a test supplies a fake without a global |
| The storage backend differs per deployment (local FS, S3-compatible, in-memory for tests) | **Protocols + existentials** (`-> some S: BlobStore`) | callers bind to the surface, not the implementation; no dynamic dispatch where a monomorphised call will do |
| A cache layer must not accidentally perform IO, an audit path must not be fallible in a way that loses records | **Computational properties** (`Pure` / `IO` / `Async` / `Fallible`) | the property set is inferred and enforced at the layer boundary; drift is a compile error, not a code review note |
| The transparency log's core claim — append-only, no rewrite — is worth proving, not testing | **SMT verification** on the log's insert/verify pair | monotonicity and inclusion-proof soundness are exactly the shape solvers are good at |
| Wire types (manifests, index rows, protocol frames) must serialise identically on both ends of a federation | **`@derive` + macros** over one type definition | one declaration, two directions, no hand-written codec drift |
| A private node must never widen its own authority silently after deployment | **ATS-V**: computed Shape, pinned on trust-domain edges, `arch diff` in CI | see §4 |

## 2. Federation is the architecture, not a feature

Enterprises receive the same code the official node runs. Therefore:

**One node, two roles.** Official and private differ by configuration
and trust anchors, never by code path. A `role` in the manifest selects
policy; it does not select a different programme.

**Upstream proxying with a local cache.** A request for a package the
node does not hold is served from its upstream peer, cached, and served
locally thereafter. A node whose upstream is unreachable keeps serving
everything it has cached — an office with a broken uplink keeps
building.

**Name authority is explicit and auditable.** A local publication may
not silently shadow an upstream name; that is a supply-chain attack
spelled as a convenience. The resolution rule is part of the protocol
specification (not an implementation detail), and every resolution
records which authority answered.

**Trust travels.** Signature verification (sigstore) and transparency
(sumdb) are checked by the local node *against the upstream root*; the
node keeps its own append-only log for its own publications and can
prove to a client that both are consistent. A client verifies the same
way regardless of which node answered.

**Air-gap is first-class.** Exporting a slice of the registry and
importing it into a disconnected node is a supported operation with its
own verification, not a documented workaround.

## 3. Sources: narrow by decision

Package sources are **the registry itself and git**, plus others of the
same order added when a real need appears. There is deliberately no
speculative abstraction layer for hypothetical transports: one narrow
`Source` protocol with two real implementations. Extension must not
require reworking the core — and must not be paid for in complexity up
front. (Content-addressed transports such as IPFS are candidates for
that same extension point, not an axis the design is built around.)

## 4. Architecture-as-code, load-bearing

Every module's capability Shape is **computed** by the compiler
(inference-first, two-layer law); annotations **pin** intent where it
matters. Pins are mandatory on trust-domain edges — the boundary
between "handles untrusted upload bytes" and "signs a log leaf" is
exactly where an unpinned drift would be catastrophic.

Three consequences the implementation owes:

1. `verum arch query` answers "what may this path do?" over the
   registry's own corpus — the first real test of the vocabulary at
   scale.
2. `verum arch diff` runs in the registry's CI: a change that widens
   the capability surface (a handler that gains `Network(Outbound)`,
   a cache that gains `Write(File)`) fails review by exit code, with
   the widening named.
3. The physical-enforcement layer applies where the platform allows:
   a node's declared surface becomes its syscall allow-list, so a
   compromised handler cannot exceed what its Shape claims.

## 5. Development discipline — the registry is a proving ground

The registry is built **with** the language, not merely in it. When
the registry stumbles on a language defect, the defect is fixed in the
language first; a workaround in registry code is prohibited, because a
workaround silently removes the signal the proving ground exists to
produce. Every stumble is filed with a minimal reproduction before the
fix.

This is why the phase has a gate: strict-by-default compilation
(silent degradations become loud failures), one verdict per source
(the library's own files and the compiled artefact must agree), and a
trustworthy iterator surface. Without those, a language defect reaches
the registry as "the service behaves oddly" instead of "the compiler
refused, here, for this reason".

## 6. Shape of the work

Phase A — this document plus the protocol specification: resolution
and name authority, federation handshake, verification chain, the
export/import slice format.

Phase B — reference node: domain core (types carrying their invariants)
→ storage protocol with two implementations → the federation path
(proxy, cache, offline) → publication (linear tokens, transparency log)
→ the HTTP surface.

Phase C — enterprise packaging: deployment topology, trust
configuration, air-gap procedures, upgrade discipline for a node that
must stay compatible with an official index that keeps moving.

Each phase closes with the language defects it exposed either fixed or
filed — that list is a deliverable of the phase, not a side effect.
