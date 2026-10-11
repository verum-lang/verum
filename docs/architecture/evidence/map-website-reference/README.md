# Map reference and cookbook validation

Website commit `a30a1f176b72a85ee8d3e4efc6f0e813fadf8bf5` is integrated
on clean local website main. Its two edited pages describe owned-key/value
lookup, mutable value-slot access, entry and iterator limitations, and the
separate native and borrow-lifetime boundaries. No deployment is recorded.

All five website gates pass: build, typecheck, links, freshness labels and
internal-artifact exclusions. Parent review verifies their raw logs and
2,015 recorded source bindings against 403 committed Git blobs. The gates
ran before the documentation commit; the byte identities, rather than an
earlier HEAD label, bind their exact inputs to that commit.

Seventeen rendered-content checks cover signatures, caveats, anchors, links
and the exact executed example. The first HTML inspector omitted Docusaurus
line separators; its failed result and both inspector versions are retained.
The corrected inspector accepts the same unchanged rendered HTML. This was
a harness correction, not a source or runtime repair.

The [manifest](manifest.json) binds both source pages, rendered HTML, gate
receipts and logs, review scripts and the complete cookbook fixture. Its
ordinary interpreter execution is retained in the separate
[consumer replay](../map-mutable-slots/consumer-replay/README.md).
These results do not establish complete Map support, native AOT, stale-borrow
enforcement or a fresh standard-library bake.
