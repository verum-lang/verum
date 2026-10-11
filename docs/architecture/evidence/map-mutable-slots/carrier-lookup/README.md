The unchanged runtime-carrier controls reproduce the mutable lookup failure and
pass after the bounded slot repair. The complete production Map module is
compiled with parsed callers using actual map literals (`NewMap`); its
`Slot` declaration and `get_mut` body are retained. Each integration control runs
both its source module and an independently serialized/deserialized module.

| Gate | Result | Wall time |
| --- | --- | --- |
| Before the repair | 1 passed, 9 failed | 54.134 s |
| Slot repair with Shared ownership guards | 10 passed | 58.068 s |
| Direct storage-boundary controls | 8 passed | 36.875 s |

The baseline's owned-read positive succeeds. All nine mutable cases reach the
same `Map.get_mut` field refusal as the separately retained ordinary compiler
probe. After the repair, scalar and whole-variant replacements update the
original Map slot; nested changes remain visible through the original map.
The controls also cover referenced receivers and keys, collision chains,
missing keys, the last backing slot containing integer 520, and growth after
the borrow has ended. These observations do not establish stale-borrow
invalidation or a deep-copy contract.

The direct helper controls cover all existing three/four-field flat headers and
both backing TypeIds, finite full-table misses, checked capacity arithmetic,
exact backing extents, and rejection of foreign/raw/interior pointers. They
exercise the new helper directly; the earlier generic dispatch entry point's
pointer classification is outside that provenance result.

The first [complete-source harness](../source-baseline/README.md) failed earlier
in `Map.insert_internal`, even for its owned-read positive. It remains a
harness-boundary result. Selecting an actual runtime map literal in this packet
makes the mutable lookup comparison causal; it does not replace the required
ordinary compiler replay of the unchanged `Map.new` probe.

The fixed source includes the independently reviewed Shared ownership and
payload-bound guards. Reference cleanup through `DropRef` is a separate open
authority check; write-through alone does not establish safe cleanup. The
source borrow contract still forbids table-shape mutation during a live mutable
borrow. The broader declared `Slot` versus flat runtime representation, original
`entry().or_insert_with()` acceptance, lifetime verification and AOT remain open.

The [manifest](manifest.json) records exact commands, filters, source bindings,
archive identities, raw payload hashes and retained executables. The five
unchanged archive files are pinned provenance inputs, not consumed by these
archive-free VBC gates. Both commands use the minimal `codegen,table_dispatch`
features without automatic precompilation. CI selects the integration target
through its VBC `--tests` gate and the extent controls through the library
unit-test surface. No remote CI result is claimed.
