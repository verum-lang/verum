The receiver probe now uses the module's existing exact `TypeId` index and
compares the selected descriptor's complete stored name. The first descriptor
owns an ID, matching runtime layout and CBGR lookup. A later duplicate cannot
authorize another name. This deliberately tightens the old `any` predicate;
it does not add suffix matching or change network method selection.

The [manifest](manifest.json) retains the baseline and fixed source identities,
losslessly compressed raw Cargo/test logs, dependency graph, source bindings
and test executable hashes. The focused gate runs only `record_probe_tests`
with the `table_dispatch` feature. It does not run codegen, LLVM, a stdlib bake,
the full VBC suite or ordinary language programs.

The baseline passed five controls and failed three: duplicate-ID shadow
refusal, the same refusal after cloning a warm module, and repeated probe work.
The unchanged eight controls pass after the repair. Other controls cover exact
qualified names, sparse IDs in both orders, missing IDs, nil and non-pointer
values, independent modules and serialized descriptor tables. The wire control
uses explicit descriptors; it is not a parsed-source compilation claim.

The counted control invokes the actual network interceptor on an unrelated
owned record 128 times with 16,384 descriptors. After a warm-up call, its three
probes per call inspect 6,291,456 descriptors before the repair and 384 after.
The test-only counter measures descriptor visits in the probe; it excludes the
shared index's initial construction and is absent from production builds.
No timing threshold is used.

The retained two-stack credential sample identified this code path. The
unchanged credential run later timed out at 600.050 seconds. Neither that
sample nor the focused build durations establish an end-to-end speedup; a fresh
ordinary CLI replay remains a separate acceptance step. The existing pointer
preconditions and network interceptor behavior remain outside this lookup
change.
