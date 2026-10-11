# Main integration: Map slots and reference cleanup

Frozen main-based source `0599d67bc2be9fe2b1762772e7a6008863579f6f`
passes 50 distinct focused and adjacent tests, with no failures or ignores.
The [manifest](manifest.json) retains the exact commands, source rosters,
test executable identities, raw logs, runners and immutable stdlib artifact
identities. The test targets do not consume the retained stdlib archive.

| Gate | Passing tests | Wall time |
|---|---:|---:|
| Map storage extent, Shared carrier authority, DropRef cleanup and two bounded lowering inspections | 27 | 36.356 s |
| Parsed-source and decoded-wire Map mutation, Shared receiver codegen and reference receiver interpretation | 23 | 59.608 s |

The Map tests retain the complete source Map declaration while constructing
the genuine runtime carrier. They cover scalar/variant replacement, nested
mutation, borrowed and field receivers, collisions, absence, the final value
slot, and growth after the borrow ends. The direct extent controls refuse
incompatible representations without changing the existing field guards.

The returned slot is classified as a mutable interior reference. Shared
peeling and DropRef require exact heap-object authority before probing a
header. The common Shared helper verifies carrier validity and its pointer
field before following the bounded inner extent. Dropping a genuine Shared
binding also finishes register generation and clearing.

Historical causal evidence is preserved separately for
[Map lookup](../map-mutable-slots/carrier-lookup/README.md),
[Shared carriers](../shared-carrier-authority/README.md), and
[DropRef](../drop-reference-authority/README.md). Their parent reviews rehash
the retained payloads, executables and recorded historical Git inputs.
Failed test-harness assumptions remain classified separately from runtime
failures. The final main tests use the corrected source inspections.

This gate does not establish ordinary archive-backed CLI execution, a fresh
automatic stdlib bake, native AOT, or the full VBC suite. Map entry APIs and
representation consolidation remain open, as do the wider header audit,
pending tuple/Shared drop scheduling, stale-borrow enforcement and registry
publication acceptance. The mutable-address set classifies references; it
does not itself enforce lifetime invalidation after table changes.
