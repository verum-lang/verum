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
Verum service roundtrip. The T1651 producer gates below cover dependency
projection. Actual server admission, durable storage, restart and
publish-install-run acceptance remain required by the
[publication contract](cog-publication-protocol.md).

## Publication dependency intent — T1651

All four manifest-based metadata producers now share one dependency
projection. Detailed versions, feature lists, optional flags and default-feature
flags retain their declared values. Explicit wildcard requirements remain
valid; missing versions are never replaced with a wildcard. Path/Git source
declarations, including versioned hybrids, are refused before archive work.
Unknown dependency fields such as `registry`, `package` and `workspace` are
refused during manifest loading instead of being discarded.

Workspace publication previews use the same projection. Direct metadata
callers also require valid, explicit dependency versions at shared publication
admission, before opening the archive or sending HTTP. The general read-side
dependency type remains compatible; a missing or null version is not valid
publication metadata.

Tested source `4409dc64a` passed **34 publication**, **5 configured-registry**
and **4 configuration-profile** entries. Three entries are inert subprocess
dispatchers. The publication controls include the actual package handler,
exact original manifest/source bytes in its gzip-tar, option preservation,
workspace dry runs and direct API refusals. Preserved causal baselines show
the corresponding data loss and admission failures. An archive-entry fixture
spelling error was corrected separately and is recorded in the
[gate receipt](evidence/registry-publication-dependency-gate.json).

The nine patches rebased unchanged onto `3d83e98c1` as `7d7d3ca88`; all owned
CLI source, test and protocol bytes are identical to the tested revision.
The base also brings archive producer, compiler, type-checker and VBC changes.
The receipt
lists that exact difference; the selected gates do not claim execution of
the combined source. They used the separately identified stage14 artifacts
with automatic precompilation disabled. A fresh ordinary CLI and registry
replay remain required.

The configured-registry fixture still downloads opaque bytes; this is not
source-cog extraction or fresh-consumer acceptance. T1657 covers dependency
activation and recursive installation, so T1651 remains open for that
consumer gate. Authenticated service admission, signature support, durability
and publish-install-run acceptance remain separate. Workspace canonical
manifest discovery and member counts are covered by the bounded T1665 handler
gate below.

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
at `412b0425e`, both direct and umbrella `Map<Text, JsonValue>` checker controls
failed despite the corrected archive payload identity. That historical
baseline is preserved in the T1664 receipt below; the consumer fix now passes
those unchanged controls. The producer gate above retains its original source
and scope.

## Archived generic payload checking — T1664

Alias expansion previously selected argument zero for every anonymous type
variable by inspecting its debug text. This changed `Map<Text, JsonValue>`
into `Map<Text, Text>` on the consumer recovery path. Substitution now uses
parameter names and ordered variable slots owned by the exact declaration.
Metadata nominal templates record the variables minted during registration;
unused and reordered parameters retain their positions, and unrelated nested
variables are not substituted. Missing slot metadata does not trigger a
free-variable scan or a same-leaf owner lookup. The existing normalization
fallback and unifier nominal-name comparison are unchanged.

At frozen source `6117e6bb5`, all **5 alias-slot controls** and **15
source-to-archive-to-metadata-to-checker controls** passed. The original direct
and umbrella `Map<Text, JsonValue>` positives and wrong `Bool`, key and value
refusals remain unchanged and pass in both lazy and eager loading modes. The
slot controls separately cover exact owners, reordered and phantom parameters,
foreign variables and absent slot metadata. The full `verum_types --tests`
gate passed **3,962 tests across 173 targets**, with zero failures, zero
filtered tests and three pre-existing ignores. The
[alias argument gate receipt](evidence/registry-alias-argument-gate.json)
records the causal failures, passing gates, artifact identities and
byte-equivalent integration of the owned source and tests.

These are source-library gates using unchanged, separately identified
standard-library artifacts with automatic precompilation disabled. The later
strict and legacy JSON controls below execute the ordinary typed interpreter
with freshly baked standard-library bodies. Whole-registry and AOT acceptance
remain separate from those bounded parser results.

## Workspace publication manifest authority — T1665

Workspace publication selects members through the shared `Config` manifest
authority, preferring `Verum.toml` and accepting legacy `verum.toml`. Upload
metadata reuses that loaded manifest. Source archives preserve the selected
manifest bytes under the canonical entry name; a missing manifest now fails
packaging. Previews and uploads report successful, failed and skipped member
counts separately. A malformed selected manifest fails instead of falling
back; a missing member remains an explicit skip and cannot inflate the total.

The [gate receipt](evidence/registry-workspace-publication-gate.json) records
the causal baseline `9a11d3632` and fixed source `b6860b03c`. On a real
case-sensitive filesystem, all **11 substantive controls** failed at the
baseline and passed after the fix; one additional entry only dispatches
isolated child tests. **Five adjacent dependency-admission controls** passed
on the same fixed executable. The controls cover canonical and legacy names,
distinct-file precedence, invalid and absent manifests, exact archive bytes,
and the actual `new` handler. That handler created `verum.toml` and its project
was previewed without renaming it. T1668 owns scaffold spelling migration.

The Rust CLI library gate is selected by CI's workspace unit tests. Set
`VERUM_T1665_FIXTURE_ROOT` to a case-sensitive directory when reproducing the
full precedence controls; an explicitly supplied case-folded directory fails
the fixture preflight. The recorded run held the existing stage14 runtime
artifacts unchanged with automatic precompilation disabled. An initial attempt
in a different target was stopped before testing when it would rebuild Z3.

This accepts library-handler discovery, preview counts and source packaging.
It does not add or exercise a `workspace publish` CLI route, authentication or
a registry service. T1639 tracks that interface; T1672 covers the separately
dispatched workspace management handlers. Ordinary CLI and full registry
acceptance remain separate.

The reviewed changes rebased onto `3083e0939` as `b22c2ec1b`. The complete
CLI crate tree, including controls, is byte-identical to tested `b6860b03c`.
The receipt records the separate compiler/type-checker base changes; the
original gate does not claim execution of that combined source or an ordinary
CLI bake. The tested source remains preserved on its named branch.

## Workspace management manifest authority — T1672

The handlers behind `workspace list`, `add`, `remove` and `exec` use the shared
manifest authority. Canonical and legacy members are recognized consistently.
Membership updates persist to the selected workspace manifest, leaving a
distinct lowercase shadow unchanged. Execution treats a declared member that
cannot load as a failure; it cannot skip that member and report successful
execution in zero members. An actually empty workspace retains its behavior.

The [gate receipt](evidence/registry-workspace-management-gate.json) preserves
the case-sensitive baseline `117bdf2ae`: **10 failing controls**, three passing
compatibility/refusal controls and two inert subprocess entries. Fixed source
`e3db15b49` passed **15/15 entries**. Real child commands wrote per-member cwd
markers, including a controlled command failure; membership assertions reloaded
the modified manifest and compared untouched shadow bytes. On the same fixed
executable, **12 publication** and **five dependency** entries also passed.
Those 32 passing entries contain 29 substantive controls and three inert entries.

These external test modules are selected by CI's workspace unit-test job. For
complete local precedence coverage, point `VERUM_T1672_FIXTURE_ROOT` at a
case-sensitive directory; the publication neighbor uses
`VERUM_T1665_FIXTURE_ROOT`. Explicit case-folded roots are refused. The recorded
gates retained the pinned stage14 artifacts with automatic baking disabled.

This is executable handler and filesystem acceptance. The CLI dispatch wiring
was inspected, while clap parsing and an ordinary `verum` binary were not
executed. Registry/authentication and full platform acceptance remain separate.
The undispatched workspace build/test/check helpers and the absent publication
route are outside this four-handler change.

The chain rebased onto documentation-only main `5f8f9e8ea` as `844551f9a`.
The complete CLI source and test tree remains byte-identical to executed
`e3db15b49`, whose named branch is retained. Both workspace gate receipts and
the earlier alias-consumer acceptance are preserved.


## Strict publication JSON decoding — T1662

`core.encoding.json.parse_strict` reuses the production parser with an explicit
UTF-8 document byte bound. It refuses repeated decoded object keys before Map
insertion loses the earlier occurrence, including escaped spellings in nested
objects and arrays. Objects retain independent key scopes. The existing depth,
string and collection limits remain shared with the ordinary parser, whose
`parse`, `decode` and `parse_value` entry points retain their duplicate behavior.

The [runtime receipt](evidence/strict-json-runtime-gate.json) retains the exact
[ordinary build](evidence/strict-json-ordinary/build-manifest.json), its log and
both runtime results. Candidate `d61c11aa5` built with automatic standard-library
precompilation in 1,713.896 seconds. On that pinned CLI, the strict conformance
fixture passed in **108.499 seconds** and the separate legacy positive passed in
**65.058 seconds**, each within its 180-second deadline with exit `0`, exact
stdout, unchanged source inputs and unchanged executable. The retained archive,
metadata and symbol-graph hashes identify the baked bodies used by that product.

The strict fixture exercises unique values, nested and escaped duplicates,
sibling object scopes, duplicate name/location diagnostics, UTF-8 byte bounds,
negative bounds, depth boundaries, malformed/trailing input and legacy last-wins
compatibility. The separate legacy fixture checks an ordinary object and exact
integer payload. Reproduce these controls from the matching source checkout:

```sh
verum run --tier interpret vcs/specs/L2-standard/encoding/json_strict_publication.vr
verum run --tier interpret vcs/specs/L2-standard/encoding/json_legacy_source_control.vr
```

The earlier [source gate](evidence/strict-json-source-gate.json) passed two
compilation/archive controls without interpreting their Verum entry points.
It also preserves the failed mount-loader attempts: missing declarations first,
then a one-byte `Text.with_capacity` placeholder that failed at `ArrayLen(Unit)`
in both strict and unchanged legacy trials. Those failed harness results remain
part of the record; the ordinary runs use the complete baked dependency bodies.

The integration on `8ee11bfb8` preserves the original branches and records each
replayed commit. Its JSON module and all three owned test files are byte-identical
to the executed candidate. The newer base also contains workspace CLI changes;
that combined tree was not rebuilt by these controls. Typed registry metadata,
semantic publication admission, durable storage, the complete registry service,
and AOT execution remain separate acceptance boundaries.


## Fresh registry project replay — T1635

The same ordinary `d61c11aa5` product used for strict JSON completed the
registry project check in **211.317 seconds with 11 compilation errors**,
exit `101`, within a 600-second deadline. The
[receipt](evidence/registry-project-stage16.json) and
[complete log](evidence/registry-project-stage16.log) retain exact source and
executable identities; both remained unchanged during checking.

The earlier product reported 64 errors. Every source input in that earlier
receipt is byte-identical in the new run; a publication-envelope module and
its fixture were added. The registry repository records this comparison at
`2035195`, in `tests/evidence/stage16-comparison.json`. Multiple language
changes separate these products, so the diagnostic reduction does not
isolate one fix or establish a performance improvement.

Remaining diagnostics concern `Future` versus `JoinHandle`, source protocol
bounds, `IoResult` pattern checking, HTTP response fields and transducer
types. Context-declaration warnings also remain. This is a completed failed
project check, without registry runtime, proofs, AOT or service acceptance.

A separate typed publication-metadata candidate at registry `399cc41`
failed before execution with 18 errors in 69.97 seconds on the same product.
Seventeen errors share the borrowed JSON object to typed map helper boundary
(T1676); the remaining error concerns a forward-declared variant payload in
project checking (T1677). Record-field diagnostic labels wrap the nested
helper argument failures; they do not establish incorrect record field
declarations. The registry's `tests/evidence/publication-metadata-stage16/`
retains its result and logs. This candidate remains separate from registry
main, and standalone parser success does not establish metadata admission.


## Declaration-owned generic applications — T1676

The metadata reader now relates a generic record's qualified annotation to its
method signature through the exact descriptor binding and declaring owner.
Generic-to-named comparison retains the whole path and checks every argument;
unknown and foreign owners cannot borrow a same-leaf declaration. Transparent
aliases keep their existing substitution, and replacing metadata replaces the
head-identity snapshot. Named-to-named unification is unchanged.

The [checker receipt](evidence/registry-nominal-head-gate.json) records the causal
baseline `76cee1bad`: **7 passed, 4 failed**, including the registry's exact error
when `JsonValue.as_object()` is forwarded into a source helper accepting
`&Map<Text, JsonValue>`. Frozen repair `3821b1961` passes all **11 controls**.
They cover eager/lazy readers, load and comparison orders, renamed exports,
metadata replacement, and wrong arguments, arity and owner negatives. The fixture
serializes minimal metadata and parses real source; it does not produce a VBC
archive or execute Verum runtime bodies.

The unfiltered `cargo test --locked --offline -p verum_types --tests --
--test-threads=2` gate passes **3,973 tests**, with zero failures and three existing
ignored tests across 173 binaries. It uses the same frozen source, a reused private
target and `VERUM_NO_AUTO_PRECOMPILE=1`. The receipt retains source/executable/log
hashes, the initial Rust compile error and its correction, and the original
baseline before the test-only `List<Text>` representation adjustment.

Ordinary CLI and registry metadata replay remain separate acceptance boundaries.
The project preflight's forward-reference error is independent of this repair.
No archive schema or writer changed; a new reader using an existing archive must
identify that producer artifact explicitly. These checker results do not establish
fresh-bake, registry authentication, durable admission, installation or AOT success.

The reviewed chain rebased onto documentation-only main `fae253d0f` with source
commit `28b59011d`. Its complete type-crate tree is byte-identical to executed
`3821b1961`; both original branches and the preceding registry project receipt
are retained. CI's `unit` and `integration` type-system jobs include these controls.
