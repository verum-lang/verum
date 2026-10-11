# Combined compiler and checker consumer replay

The consumer CLI at `f372e51e9` includes checked archive completion, indexed
runtime record probes, canonical Text identity, compilation-owned archive
function IDs and preservation of borrowed generic variant payload types.
The build passes in 390.157 seconds. Its selected type checker, VBC,
compiler and CLI units are freshly compiled; all source bindings and the
retained executable are independently verified by [the review](review.json).

This build explicitly reuses the five unchanged Stage23 standard-library
artifacts from `7a8c86f90` with automatic precompilation disabled. It does not
establish a fresh bake or distribution build.

The original SemVer options fixture and real-standard-library Map probe keep
their original source bytes and exact completion-output oracles. Both stop
before runtime because finalization detects two `Text` descriptors sharing
`TypeId(4)`. Options fails after 87.792 seconds; Map fails after 78.694
seconds. Neither emits program output. The former formatter panic and the
latter unresolved-field diagnostic are no longer the observed stopping
point, but neither fixture is accepted.

The retained receipts preserve both failures, all original logs, unchanged
source, baseline receipts, runner sources, build identities and artifact
hashes. The canonical-descriptor collision must be repaired independently;
no runtime, Map mutation, copy-isolation or whole-registry result is inferred.

The [Text-repair consumer checks](text-coherence/README.md) retain the next
measured result: all 14 SemVer options pass; the Map read passes before a
separate mutable-access runtime failure.
