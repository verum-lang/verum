# Canonical Text descriptor and implementation carriers

The runtime Text record can share its reserved identity with an implementation-only primitive carrier. Importing either order now retains one canonical record, its declaration policy, fields, layout and resolved glue, and the complete carried protocol bindings. Source implementation attachment and bootstrap import follow the same insertion authority. A descriptor with independent schema or raw glue remains a conflict; clearing unresolved source-local glue cannot turn it into an eligible carrier.

The repair uses the existing exact `core.text.text.Text` declaration authority. Foreign records named Text remain separate. The small carrier-eligibility marker belongs to the compilation and follows descriptor replacement, coalescing, finalization and reset. This change does not validate arbitrary conflicting canonical record schemas or repair unrelated archive protocol-method remapping.

The [manifest](manifest.json) binds all five runs and their original receipt/log hashes:

| Run | Source | Result | Meaning |
| --- | --- | --- | --- |
| Carrier baseline | `2ed8f6df3` | 8 pass, 5 fail | Both import orders, source attachment and bootstrap expose duplicate descriptors or lost record/protocol data. |
| Authority baseline | `b73740dcc` | 8 pass, 6 fail | The same five causal failures, plus an invalid glue fixture that omitted the producer function registry. |
| First fixed attempt | `2065ff0b8` | Compilation failed | Two Rust remap-boundary type mismatches; no test executed. |
| Corrected fixed controls | `da81b1ff2` | 19 pass | Actual parsed Text, source/wire imports, strict finalization, bootstrap orders, raw-glue refusals and reset. |
| Existing adjacent controls | `da81b1ff2` | 34 pass | Scalar archive identities, implementation attachment, strict consistency checks and bootstrap glue. |

The glue fixture was corrected by retaining the actual producer function registry and remapping the real declared Drop body. The positive then checks both preserved Drop/glue and the newly imported protocol's associated type. Separate negative controls deliberately omit that registry and verify that unresolved raw glue still prevents reconciliation, in both orders. The original fixture failure is not classified as a production regression.

Each gzip file decompresses to the exact original receipt or Cargo output. The manifest records both stored and original hashes. Receipts retain commands, source bindings, executable hashes and retained paths, all outcomes, and before/after artifact identities. The retention check verified 26,605 historical Git source bindings and 5,327 unique blobs. Raw logs remain byte-for-byte intact inside their compressed containers.

The focused gate finished in 31.323 seconds; the five adjacent targets finished in 34.489 seconds. All 53 final controls passed with no ignored tests. The checker target was then released idle, as recorded in [release.json](release.json). No further target work is implied by this evidence.

These gates compile source fixtures and exercise VBC assembly/serialization. They do not consume the precompiled standard-library archive. The five immutable Stage23 artifacts were independently pinned unchanged; automatic baking was disabled. Actual acceptance of the Stage23 carrier, unchanged ordinary SemVer options and Map replays, a fresh whole-library bake, and native/AOT behavior remain separate requirements.

The [main integration checks](main/README.md) also pass the nine compiler
archive-identity controls and 19 Text controls on the isolated main-based
source. [Parent verification](parent-review.json) checks the full causal
packet independently.
