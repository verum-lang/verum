# Consumer checks after Text descriptor repair

The consumer CLI at `abfb5fc88` builds successfully in 277.902 seconds.
Its selected type checker, VBC, compiler and CLI units are freshly compiled.
The [independent review](review.json) verifies all 5,388 source-at-commit
bindings, the retained executable, receipts and logs. This build explicitly
reuses the five unchanged Stage23 artifacts from `7a8c86f90`; it does not
establish a fresh automatic bake or a distribution build.

The unchanged SemVer options fixture passes all 14 original assertions and
prints exactly `semver-options: 14 passed` in 99.875 seconds. The earlier
Text descriptor collision no longer blocks this actual consumer.

The unchanged Map probe reaches runtime and prints `map-read: ok`, confirming
its concrete record-field read oracle. Mutation then fails after 86.772
seconds: `Map.get_mut@pc=118` reads field index 2 from a value described as
`Unit`, with zero data size. The original `map-mutate: ok` oracle is not
reached. The earlier E103 checker error and Text finalization collision are
resolved for this fixture; mutable Map access remains an explicit runtime
blocker, separate from the original prepared-copy allocation-guard failure.

The original source bytes, success oracles, prior failure receipts and full
logs are retained. No Map mutation, registry publication, deep-copy isolation,
full VBC or AOT acceptance is inferred.
