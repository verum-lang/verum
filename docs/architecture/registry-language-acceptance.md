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

These are bounded-work and semantic results. The rebuilt ordinary CLI must
still complete the registry project check and record its actual verdict and
time before claiming an end-to-end improvement.

The CLI library gates above used an isolated target with the separately
identified stage14 standard-library artifacts and automatic precompilation
disabled. They did not produce a fresh ordinary CLI or an AOT release.
