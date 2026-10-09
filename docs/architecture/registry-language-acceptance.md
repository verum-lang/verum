# Registry-driven language acceptance

Registry requirements drive these language and tooling changes. Each result
below states the path actually exercised. The registry's launch contract is
in [registry-federated-design.md](registry-federated-design.md); these source
gates do not establish a deployed publish-to-install service.

## Selected registry consistency — T1634

The CLI installation flow previously used the configured registry for
metadata, then attempted an archive download from `vcogs.io`. A loopback
refusal proxy reproduced the fallback without contacting an external
service. The fixed client shares its selected base across metadata,
archive/VBCA requests and lockfile source records, and refuses malformed
manifests instead of silently selecting a default registry.

Source checkpoint: `532b3172a`. The `verum_cli --lib cog::` gate passed
7 tests, including isolated installation controls and existing checksum
and URL-format controls. The regression supplies package bytes through a
local HTTP fixture; extraction, module mounting, authentication and the
registry service remain separate acceptance boundaries.

## Documentation evidence — T1640

Source annotations request verification. The documentation generator does
not run a verifier or consume a proof receipt, so its entries must remain
Unverified. It now renders declared annotations separately and preserves
comment claims as prose. A malformed-source fallback can no longer turn
`Verified:` comments into a Proven badge.

Source checkpoint: `d42719cac`, integrated as `e6b08869c`. Three actual
command-handler regressions failed before the change; the final
`verum_cli --lib commands::doc::` gate passed 4 tests, including the isolated
subprocess entry. The controls read generated HTML for requested strategies,
false postconditions, forged claims, parser fallback and the index legend.
This establishes honest source-only documentation, not discharged proofs.

## Metadata lookup work — T1643

An older identified ordinary CLI could not complete the registry's
argument-less project check within 600 seconds. A short interval profile
pointed to context signature registration, including repeated scans of all
metadata implementations. That profile identifies a hot path, not its
fraction of the entire timeout.

The source regression uses an actual checker with 4,097 implementation
descriptors and 11 known-owner lookups. Before the fix, counters in the
registrar recorded 45,067 candidate visits. After the fix, one index build
visits 4,097 descriptors and subsequent lookups visit 10 selected candidates.
Buckets preserve descriptor order and exact declaring owners, including
proven aliases. Installing another metadata snapshot invalidates derived
indexes and completed registration tails; previously installed declarations
remain additive.

Source checkpoints: `5db60ed61` and `159a0c280`, integrated as `e31c97e7d`
and `4c8c48536`. Validation at `159a0c280`:

- 9 metadata owner/lifecycle tests, including unrelated and missing owners,
  aliases, metadata replacement and a metadata-defined generic protocol.
- 28 protocol-dereference and 3 qualified-argument tests.
- Full `verum_types --tests`: 172 test targets, **3,928 passed, zero failed,
  3 existing ignored**, zero filtered; 179.07 seconds wall time with two
  build jobs and two test threads.

The existing ignored tests are the full-stdlib typecheck in `core_pipeline`
and two T0922 checks for too few type arguments. No new ignore was added.

Reproduce the full source gate with a session-private Cargo target:

```sh
cargo test --locked -p verum_types --tests -- --test-threads=2
```

An ordinary CLI built at `b12135c9f` completed the same registry `.vr` sources
in **217.97 seconds**, returning **64 compilation errors**. Its executable
SHA-256 is `b068627fa0cfa87ec94cfcf0faf9be57351077ced350062f60ccdd7c9223b5f6`.
The registry repository preserves every input hash and the complete log in
`tests/evidence/project-check-stage15.json` and its companion `.log` file.
The previous product timed out at 600 seconds. This establishes a completed
diagnostic run; it neither establishes registry correctness nor isolates the
speedup of this change from the other changes in the rebuilt product.

The diagnostics include explicitly mounted registry types resolving to
standard-library homonyms, tracked by T1212. Registry runtime, publication,
installation and AOT acceptance remain open.

The CLI library gates above used an isolated target with the separately
identified stage14 standard-library artifacts and automatic precompilation
disabled. They did not produce a fresh ordinary CLI or an AOT release.

## Registered source-cog checking — T1637

Project checking now inventories registered source dependencies, carries the
resolver into its isolated checker session and checks dependency bodies.
Eager and lazy loading select the same package source root and module names,
including a nested directory named `src`. Missing, unreadable, malformed or
colliding modules produce errors. Cross-cog imports respect public exports;
loading a dependency no longer exports every private declaration.

Source checkpoints through `900a5f030`, integrated through `e2bde39a7`.
The final gates passed **18/18** source-cog controls and **5/5** project-path
controls. Before the final path correction, the earlier implementation also
passed all **210** `verum_modules` integration tests. The source-cog controls
are included in CI's workspace `--lib --bins` unit job. The
[gate receipt](evidence/registry-source-cog-gate.json) records final source,
commands, logs and unchanged stage14 artifact hashes.

This is the compiler portion of installation. Archive extraction,
transactional manifest/lock updates, transitive installation and the actual
publish-install-run flow remain open. Restricted visibility authorities
such as `public(cog)` remain T1023; hyphenated and scoped dependency names
also need acceptance. These checks do not establish complete installation.

## Record coordinates in collections — T1644

The registry's publication history uses a record containing the package
name and exact version. Interpreter collection keys previously hashed
ordinary records by address, so constructing the same coordinate again
could miss an existing release. The shared Map/Set helpers now hash and
compare nominal record identity and declared fields. They ignore allocator
padding and compare Text content across inline and heap representations.
Header reads require live allocator ownership; native runtime carriers
retain their existing representation rules.

Source checkpoint: `a53e9deaa`, integrated through `0330dc2fb`, `786c3edf4`
and `a536aa4c2`. The focused controls passed **12/12**, with **47/47** related
existing comparison, copy, hash and ScriptEngine tests passing. The controls
exercise allocated-equal keys, differing names and versions, bucket
collisions, growth, removal, nested records and representation boundaries.
They bypass the type checker and do not establish derive expansion or AOT
collection semantics.

The original full library run was **incomplete**: 2,071 tests passed, two failed,
one was ignored and two socket tests were still waiting when the process
was stopped after 16 minutes 9 seconds. Isolated accept/read tests passed;
the readiness test failed, and UDP first timed out before passing a retry.
Those tests exercise host sockets and IoEngine, without calling the changed
record-key helpers. This call-path separation does not explain the I/O
failure. T1650 tracks deterministic, bounded socket acceptance; retries do
not turn the incomplete suite into a pass. The later T1653 source gate below
completed the full library suite on its identified revision; it does not
explain or reclassify this earlier run.

The [gate receipt](evidence/registry-record-key-gate.json) preserves source,
executable hashes, counts and the limits of the retained baseline comparison.
Plain record clone independence remains T1645, and caller mutation of stored
keys remains T1649. The registry catalog still needs typed project and
runtime acceptance before its publication guarantee can be claimed.


## Imported type ownership — T1212

Explicit source mounts now retain the declaring module in nominal types,
methods, tuple/newtype constructors and field metadata. Renamed and
re-exported types retain that same identity. Qualified type lookup and alias
expansion no longer substitute an unrelated type with the same short name.
A missing export cannot be supplied by an ambient metadata declaration.

The integrated source at `d1ee93588` passed 11 mounted-owner controls and
8 module-aware controls. These include conflicting sibling names in both
registration orders, renamed constructors and missing/private exports.
The combined full `verum_types --tests` gate passed **3,947 tests across
173 targets**, with zero failures, zero filtered tests and three existing
ignored tests in 187.289 seconds. The [gate receipt](evidence/registry-type-ownership-gate.json)
identifies the source, command and log hash. Explicit generic receiver
arguments are covered by the separate T1656 source gate below.

## Declared static receiver arguments — T1656

Static calls on source-defined types now bind the declared implementation
receiver before checking arguments. Reordered, nested, repeated and fixed
receiver arguments remain constraints. Method generics keep their own
identity, including when they shadow an implementation parameter, and a
method may deliberately return a different type.

Tested source `a5c6cd03a` passed the full `verum_types --tests` gate:
**3,957 tests across 173 targets**, zero failures, zero filtered tests and
three existing ignored tests in 186.491 seconds. The
[gate receipt](evidence/registry-static-receiver-gate.json) preserves that
original source and proves compiler/test source identity after the
documentation-only rebase to `f3ab060c3`.

Metadata without a carried implementation pattern and variant constructors
retain their existing inference paths. Ordinary registry replay and
interpreter/AOT acceptance remain separate gates.

## Protocol registration work — T1655

Registering 1,024 unrelated protocol identities previously called the overlap
checker 523,776 times. Registration now selects an insertion-ordered bucket
using exactly the existing protocol identity. The same control makes zero
unrelated comparisons. Target types and protocol arguments still undergo
coherence checking; they are not used to exclude candidates.

Eight focused controls preserve same-protocol conflicts, generic overlap,
specialization, strict orphan refusal, lenient warning order, duplicate
registration, mode transitions and cloned checker state. The combined full
source gate above includes these controls. Public `check_coherence` retains
its existing behavior; this change bounds registration work specifically.

A short profile of the ordinary registry envelope run identified registration
as a hot path. That interval does not measure its share of total compilation
time. The passing envelope run used the older `b12135c9f` CLI, which predates
these ownership and index fixes. A rebuilt ordinary CLI and registry replay
remain necessary before claiming a product-level performance improvement.

## Publication producer and receipts — T1636

The CLI now emits the bounded v1 binary envelope, preserving the provided
source metadata and exact archive bytes. It independently checks archive integrity, refuses
publication redirects and unsupported evidence, and validates the request even
for dry runs. Success requires a bounded JSON receipt with the exact name,
version and checksum; malformed, duplicate, unknown-field or mismatched
receipts are refused.

The final source gate at `82cda7310`, integrated through `bb6285beb`, passed
13 publication and 5 configured-registry library tests. Each group contains
one inert child-process entry. The [client gate receipt](evidence/registry-publication-client-gate.json)
records source, executable, log hashes and unchanged stage14 artifact
identities. A macOS fixture correction explicitly restores blocking mode on
accepted HTTP sockets while keeping read/write deadlines (T1654).

These are real loopback HTTP and command-handler controls, not an authenticated
Verum service roundtrip. Detailed dependency projection remains T1651; actual
server admission, durable storage, restart and publish-install-run acceptance
remain required by the [publication contract](cog-publication-protocol.md).

## Bounded host socket controls — T1650

The five affected TCP/UDP fixtures now own their sockets and bound connection,
accept, read and write waits. The accepted TCP socket must identify the live
peer and the listener port; a positive handle alone cannot pass. Readiness
checks keep a confirmed connection pending, and byte-transfer checks require
the exact payload. Existing empty-readiness and timeout controls remain.

Source `1d7078cb5` passed all **11 focused controls**, once serially and once
with four test threads, with zero failures or ignored tests. The
[receipt](evidence/registry-host-socket-gate.json) preserves commands, source,
executable and log hashes. CI's workspace unit and VBC test jobs select these
private library tests.

These changes bound test failures and preserve their socket assertions; they
do not establish the cause of the earlier incomplete full gate. A separate
confirmed-connection host control observed readiness with both a closed peer
and a retained peer, so short peer lifetime alone was not the demonstrated
cause. Production runtime behavior is unchanged, and T1650 remains open for
the original failure diagnosis.

## Archive cache source authority — T1661

The archive producer now consults the checker's source-export walk and the
module registry's canonical/export selectors. Their three source files are
therefore inputs to the automatic-bake fingerprint and Cargo's rerun list.
A future resolver change must rebuild the archive it helps describe.

A [causal fingerprint control](evidence/registry-source-fingerprint-gate.json)
compiled the actual fingerprint function from baseline `75adac0d3` and fixed
source `fa0524e49`. Each of the three selector changes was invisible to the
baseline and invalidated the fixed digest. An existing codegen input
invalidated both; an unrelated document changed neither. Repeated inputs
and restored files reproduced the same digest. This control validates cache
dependency selection; a fresh ordinary bake and registry replay remain
separate acceptance steps.

## Archived variant payload identity — T1653

The bootstrap producer resolves a written type through public source exports
before assigning generic variant payload IDs. Renamed exports retain their
actual declaring owner. Missing/private exports cannot borrow a same-named
ambient type, while an already exact catalog declaration survives an additive
resolver miss. Both layout payloads and declared payload types carry the
container identity; List and Map payloads no longer become scalar USize IDs.

The [gate receipt](evidence/registry-variant-payload-gate.json) preserves the
original source chain and measured results: **12/12** focused source-authority
controls, **13/13** adjacent archive controls, one function-export control,
and **2,075 passing full VBC library tests**, zero failures and one existing
ignored coverage report at `0bd93744e`. The full library gate took 665.357
seconds including Cargo. Private-target standard-library artifacts remained
unchanged; these gates did not rebuild the ordinary CLI.

Integration with the source-owner fixes exposed a separate consumer defect:
both new direct and umbrella `Map<Text, JsonValue>` checker controls still
fail. T1664 tracks alias argument substitution at this boundary, retaining
the failing controls. The archive producer correction is independently
validated; complete JSON checking and registry runtime acceptance remain open.
