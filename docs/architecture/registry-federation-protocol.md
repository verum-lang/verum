# Registry federation protocol

Companion to `registry-federated-design.md`, which states the principles.
This document states the **rules**: what a node answers, what a client
may conclude from an answer, and what is refused. It is written to be
implementable without further decisions.

A term in **bold** on first use is defined here and used precisely
thereafter.

---

## 0. The two things a client must be able to prove

Everything below exists to support exactly two client-side conclusions:

1. **This artefact is the one the publisher published.** Signature and
   transparency evidence, checked by the client itself, against a root
   the client already trusts.
2. **This name means what I think it means.** A resolution names the
   **authority** that answered, and the rules under which a local
   authority may or may not answer for a name.

A protocol feature that serves neither is out of scope.

---

## 1. Names and authorities

### 1.1 Name shape

A package name is `[@scope/]name`, both segments lower-case, digits,
`-` and `_`, starting with a letter. A **coordinate** is a name plus an
exact version: `@scope/name@1.4.2`.

### 1.2 Authorities

An **authority** is a node that may answer for a set of names. Every
node holds an ordered, explicit list:

```
authorities = [
  { id: "upstream",  url: "https://packages.verum-lang.org", role: Upstream },
  { id: "local",     role: Local, owns: ["@acme/*"] },
]
```

* `Upstream` — a peer whose answers this node caches and re-serves.
* `Local` — this node's own publications.

`owns` is a list of name patterns. **A `Local` authority may answer only
for names it owns.** There is no wildcard-by-default and no implicit
ownership: a node that lists no `owns` cannot publish anything, which is
the correct behaviour for a pure mirror.

### 1.3 The resolution rule

For a name `N`:

1. If exactly one authority owns `N`, that authority answers.
2. If **no** authority owns `N`, the request goes upstream; the answer
   is cached and attributed to the upstream authority.
3. If **more than one** authority owns `N`, the resolution **fails** with
   `AmbiguousAuthority`, naming every claimant.

Rule 3 is the whole point. A local publication of a name the upstream
also serves is not resolved by precedence — precedence is how a
supply-chain substitution becomes invisible. It is refused, and the
operator resolves it by changing configuration, in the open.

> **Consequence, stated so nobody has to discover it:** vendoring an
> upstream package under its own name is not possible. Vendor it under a
> name you own (`@acme/serde`), or mirror it (which attributes upstream),
> or pin it. Each of those is auditable; shadowing is not.

### 1.4 Every answer names its authority

A resolution result carries the authority id, the role, and — when the
answer came from cache — the time it was fetched and the upstream
generation it was fetched at. A client that logs resolutions has the
provenance of every dependency without extra work.

---

## 2. Verification chain

### 2.1 What is signed

The **manifest digest** — a canonical serialisation of the coordinate,
the artefact digest, the declared dependencies and the publication time.
Artefact bytes are covered transitively through their digest, so a
signature over the manifest is a signature over the release.

### 2.2 Evidence a client checks

For a coordinate `C`, a node returns, alongside the artefact:

* the **manifest**,
* a **signature bundle** over the manifest digest,
* a **transparency inclusion proof** placing the manifest digest in an
  append-only log, together with the log's signed tree head.

A client **verifies all three itself**. A node's assertion that it
checked is not evidence; it is a claim about a party the client did not
choose.

### 2.3 Which log

Each authority runs its own append-only log for its own publications.
The client's rule is:

* for a coordinate attributed to `Local`, verify against that node's log;
* for a coordinate attributed to `Upstream`, verify against the
  **upstream's** log — the local node must forward the upstream proof
  unchanged, and may not substitute its own.

A node that re-logs upstream publications into its own log and offers
that as proof is misrepresenting provenance. The wire format makes this
detectable: the proof carries the log identity it belongs to, and the
client checks that identity against the attributed authority.

### 2.4 Consistency

A node that caches upstream content keeps the upstream's signed tree
heads it has seen. On request it returns a **consistency proof** between
any two of them. This is what makes a cache auditable rather than merely
fast: a client can check that the node never served a view of upstream
that contradicts an earlier one.

---

## 3. Federation handshake

A node learns an upstream through configuration only — never through
discovery. Discovery of trust anchors is how a network becomes
attackable.

The handshake establishes, once per session:

1. **Protocol version.** A single integer, refused if unequal. No
   negotiation matrix; a version mismatch is an operator problem with a
   clear message.
2. **Upstream identity.** The upstream's log public key and its current
   signed tree head. The node stores both; a change in the key is
   refused loudly and requires an explicit operator action, because a
   silently rotated trust anchor is indistinguishable from a takeover.
3. **Capability set.** What the upstream supports beyond the mandatory
   core, as a set of named capabilities. Unknown capability names are
   ignored, which is how the protocol extends without a version bump.

Everything after the handshake is stateless request/response, so a node
restart costs nothing and a proxy in between needs no session affinity.

---

## 4. Offline and air-gap

### 4.1 Degradation is defined, not incidental

A node whose upstream is unreachable:

* **serves** every coordinate it has cached, with its cached attribution
  and the upstream proofs it already holds;
* **refuses** resolution for names it has never seen, with
  `UpstreamUnreachable` — not `NotFound`, because those are different
  facts and a build system should treat them differently;
* **continues** to accept publications for names it owns.

An office with a broken uplink keeps building. It does not silently
start resolving names differently.

### 4.2 Slices

A **slice** is a self-contained, verifiable export: a set of
coordinates, their artefacts, their manifests, their signature bundles,
the inclusion proofs, and the signed tree heads those proofs are against.

Importing a slice into a disconnected node verifies every artefact in it
before anything is stored, and records the attribution the slice carries.
A slice does not grant its bearer authority: imported coordinates keep
the authority they were exported with, so an air-gapped node cannot
launder a package into looking locally published.

Slices are closed under dependencies by default — exporting a coordinate
exports what it needs — because the alternative is discovering the gap
on the disconnected side.

---

## 5. What the protocol refuses

Stated positively, so that a refusal is never mistaken for a bug:

| Situation | Answer |
|---|---|
| Two authorities own a name | `AmbiguousAuthority`, both named |
| Local publication of an upstream-owned name | Refused at publish time |
| Upstream unreachable, name never seen | `UpstreamUnreachable` |
| Upstream log key changed | Refused; operator action required |
| Proof's log identity ≠ attributed authority | Refused as misattributed |
| Protocol version mismatch | Refused with both versions named |

---

## 6. Obligations the compiler checks

The rules above are not only prose. Each is carried by an architectural
obligation on the module that implements it, in the sense of
`@arch_module` and the ATS-V lifecycle:

* the resolution module declares that it produces an authority-attributed
  result on every path — a return that drops attribution does not compile;
* the verification module declares that no artefact reaches storage
  without all three pieces of evidence;
* the cache module declares that a cached answer keeps the attribution it
  was fetched with.

These are stated as theorems with their closure obligations, so that a
claim the code stops honouring is a build failure rather than a stale
comment. `verum arch check --strict` is the gate; an unmet obligation is
an audit finding, a false claim is a compile error.

---

## 7. Deliberately out of scope for v1

Named so that their absence is a decision:

* **Discovery.** Upstreams are configured.
* **Transitive federation.** A node has one upstream, not a graph. A
  chain works (A→B→C) because each hop is an ordinary upstream relation;
  a mesh does not, and would need a loop rule this version does not have.
* **Delegated publishing rights.** An authority publishes; it does not
  grant. Multi-party publishing is a capability, added when the need is
  real.
* **Content-addressed transports.** The `Source` protocol has two
  implementations, registry and git. Others are an extension point, not
  an axis of the design.

## 8. The wire

A client that publishes to a node, or asks one a question, does so over
HTTP. This section fixes the three routes and the one body format, so
that "a node" means the same thing to a publisher and to a mirror.

### 8.1 Routes

| Route | Contexts it needs | Answers |
|---|---|---|
| `GET /search[?name=&from=]` | none | what the node holds |
| `GET /resolve?name=<pkg>` | none | which configured source has it |
| `POST /publish` | Clock, NodeIdentity, Uplink | acceptance or a refusal |

The clause set of `/search` is a CONJUNCTION and nothing else, matching
§1: `?name=X&from=Y` means both. There is no disjunction on the wire
because there is none in the query model — "A or B" over a registry is
two questions, and a node that answers one of them silently has
answered neither.

An empty query admits everything the node holds. A query language where
"ask nothing" means "get nothing" surprises people at exactly the wrong
moment.

### 8.2 The publish body

One JSON object, four members, all required:

```json
{
  "manifest":  { "name": …, "version": …, "artefact": …, "published_at": … },
  "signature": { "manifest_digest": …, "signer": … },
  "proof":     { "manifest_digest": …, "log_id": …, "tree_size": … },
  "as_authority": "…"
}
```

`as_authority` carries the authority's IDENTIFIER, not a name pattern it
owns. An id says WHO is publishing; a pattern says WHAT they may
publish (§1). Conflating them is how a registry ends up letting anyone
who can spell a scope publish into it.

**A missing or ill-typed member is a refusal that NAMES the member** —
`missing field: signature.signer` — never a default. The alternative is
worse than it looks: a body with no `signer` that decoded to the empty
string would be checked against the authority list and refused *with the
wrong reason*, sending an operator to their configuration when their
client sent a malformed request. A default converts a transport fault
into a policy fault, and policy faults are the ones people argue about
for hours.

### 8.3 Status codes

The node refuses in kinds, and the wire keeps them apart:

| Situation | Code | Why not another |
|---|---|---|
| accepted | 201 | |
| already held, byte-identical | 200 | success, not conflict — this is how a retry after a dropped response behaves |
| malformed body | 400 | the request never reached policy |
| not that authority's name | 403 | authenticated, not permitted |
| no such name here | 404 | |
| contested name (§1) | 409 | a conflict, not a permission problem: the node REFUSES rather than ranking |
| version exists with other content | 409 | resubmitting the same body will never help |
| evidence does not hold (§2) | 422 | the body is well-formed and the name is theirs; 400 would send them to check their JSON |

Flattening these into one status is the cheapest way to build a registry
nobody can operate: every failure would read as "typo, try again".

`/resolve` maps the same way — 404 for a name no source has, 403 when a
source has it but below the audit policy (the operator has a
configuration line to change, and 404 would send them hunting for a
missing package), 502 when a configured source failed.

### 8.4 What a resolve answers with

The source's own description, not a bare yes. An operator needs to know
not *whether* a coordinate resolved but *from where* — a resolution that
cannot say which source answered is the failure §2 exists to prevent.
