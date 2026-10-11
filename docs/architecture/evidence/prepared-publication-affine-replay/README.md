# Unchanged prepared-publication affine boundary

The original `publication_prepare_affine_reuse` project still has an
inconclusive strict refusal result on consumer `6a0bf542b` (263.481 seconds,
exit 101). It emits the expected E310 use-after-move diagnostic with the
original moved/used offsets, plus an unlocated E103 for unresolved `archive`
field access. The extra error is not accepted as proof of the desired
boundary, and its cause is not assigned by this replay.

The prior Stage23 run produced the same two diagnostic categories. All seven
project inputs and the exact oracle are unchanged. The retained prior source
roster includes other boundary fixtures for provenance; only the affine
project was executed in this replay. No previous passing checks were rerun.

The [manifest](manifest.json) binds the current and prior receipts, raw logs,
project files, original source roster, unchanged diagnostic classifier and
consumer-build validator. Source identities match the frozen registry commit.
The [consumer build](../map-mutable-slots/consumer-replay/README.md) explicitly
reuses Stage23 stdlib artifacts. There is no fresh bake, runtime isolation,
native or registry-service acceptance here.

The separate registry snapshot API work changes the public inspection API.
Its new fixtures require their own acceptance and cannot retroactively turn
this unchanged original control into a pass.
