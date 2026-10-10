# Explicit typed array result bindings (T1704)

The tests-only source checkpoint is `f14fba3a8ad71b9d31ba8d07a660c8587e5ccc1c`,
tree `4b0faa823120798ce0cfb38669fb01d37189b250`, based on landed production
`49559a042a30f65c524c753d2f13c9f2838f9818`. No production code changed.

The earlier controls paired inferred and annotated Byte bindings, but only
inferred UInt32 and Float bindings. The additional matrix covers both numeric
element types, both packed and List-backed selected producers, and both binding
forms. The interpreter executes all eight combinations directly and again
from serialized VBC. Each checks the initial values and length, mutates the
second element, and checks both elements and length afterward. The existing
allocation-bound, source-order and unknown-storage controls remain enabled.

The focused `array_return_storage` target passed all six tests, with no failures,
ignores or filtered tests. Cargo and execution took 24.189 seconds; the tests
reported 0.07 seconds. The [exact receipt](result.json.gz) retains the command,
source manifest, executable identity, explicit environment and deadline.
Source files and all five inherited archive/sidecar hashes were unchanged.
The [raw stdout](cargo.jsonl.gz) and [stderr](stderr.log.gz) are lossless copies.
The test executable was [retained outside the build target](retained-executable.json.gz)
with SHA-256 `a09947d2c5aff6f4496732b43f978adc52d669fd614dfa90d30ea0dc154280c9`.

The same source commit expands the LLVM/JIT matrix with scalar value, mutation
and length probes for all eight combinations. Those controls also verify the
actual source producer differs and packed access retains its bounds guard.
The [focused native gate](native/README.md) subsequently passed all 48 source/wire
probes at `4afb8b520aa9464e3ad9836c7f2119af5ee2e5b7`, whose test and production
source bytes are unchanged from this checkpoint. Integration review remains
separate from these recorded executions.

This result establishes parsed-source and decoded-wire interpreter behavior.
It does not establish ordinary CLI, fresh standard-library, native executable,
AOT, SHA-256 or registry authentication acceptance. Unknown/forwarded call
storage, mixed control flow and fixed-array argument storage keep their existing
conservative boundaries. The [runner](run_gate.py) is the exact executed script;
[manifest.json](manifest.json) pins original and compressed evidence bytes.
