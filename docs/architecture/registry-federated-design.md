# Verum Registry — a federated protocol with a reference implementation

**Status**: implementation contract. The registry is the primary delivery
application for the language platform. Its release gate is an authenticated
publish-to-install flow against a real node; the requirements below are
not a claim that the current service implements them all.

## 0. The one-sentence thesis

The registry combines a protocol, a deployable node and developer tooling.
The same binary must serve the official index and an enterprise's private
one through explicit configuration and trust anchors. Its architectural
contracts and generated specifications must describe the code that runs,
with verification results tied to the source and tool build that produced
them.

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
| A private node must never widen its own authority silently after deployment | **ATS-V**: computed Shape, pinned on trust-domain edges, capability comparison in CI | see §4 |

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
2. A capability comparison runs in the registry's CI: a change that widens
   the capability surface (a handler that gains `Network(Outbound)`,
   a cache that gains `Write(File)`) fails review by exit code, with
   the widening named.
3. The physical-enforcement layer applies where the platform allows:
   a node's declared surface becomes its syscall allow-list, so a
   compromised handler cannot exceed what its Shape claims.

The proposed `arch diff` interface is not currently implemented. An
`arch-check = "strict"` manifest setting also has no consuming CLI
configuration field. Neither can stand in for an executable gate.
An architecture gate must demonstrate both an accepted contract and a
refused widening. Similarly, an `@verify` annotation requests verification;
generated documentation may claim a proof only from a verification result.

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

## 6. Delivery acceptance

The first working slice is one source cog published to a configured local
node and consumed from a fresh project. The byte envelope, source metadata,
authentication boundary and acknowledgement are specified by
[Cog Publication Protocol v1](cog-publication-protocol.md). It must exercise:

1. Authentication and scope authorization before any publication mutation.
2. Validated archive bytes and an immutable name/version coordinate,
   committed durably before acknowledgement.
3. Exact-version metadata and download from the same selected registry,
   with verified integrity and that source recorded in the lockfile.
4. Installation, module mounting and execution of the cog's exported
   function in a project with an empty cache.
5. The same behavior after a node restart, plus refusal of conflicting
   retries, corrupt bytes, invalid credentials and interrupted writes.

Keep positive and negative controls beside the implementation. Capture the
registry source identity, CLI identity, output and stored state. A
successful download is not evidence that a project can mount the package;
an in-memory catalog is not evidence of restart durability.

Develop language support in parallel with this flow. Retain each valid
registry program that exposes a compiler or runtime defect as a regression,
and validate it through the ordinary whole-project path after the repair.
Low-level tests identify causes; release acceptance also requires the
developer-facing commands and separate interpreter/AOT results.

Federation, air-gap transfer, compiled-cog delivery and enterprise packaging
extend this accepted publication path. Their trust and provenance rules
remain requirements throughout development. Commit bounded improvements
frequently in both repositories and integrate them onto `main` after their
stated checks pass.
