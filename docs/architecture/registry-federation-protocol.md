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
