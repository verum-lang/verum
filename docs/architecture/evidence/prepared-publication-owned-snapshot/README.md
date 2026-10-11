# Owned publication snapshot runtime boundary

Registry candidate `962f416b5c0f55f04cc38219eed8cb473720a213` replaces borrowed
publication and attribution projections with explicitly reconstructed owned
snapshots and scalar reads. The original five mutation/readback/isolation
requirements and output marker are preserved through mechanical API migration.
Separate full-shape and two-snapshot controls are prepared but have not run.

The first ordinary `prepare_copy` run fails after 369.01 seconds, exit 1,
within the unchanged 600-second deadline. Standard output is empty; standard
error reports `Assertion failed at pc 28: assertion failed`. This receipt
does not identify the failing function, source assertion or snapshot field,
and it does not establish that any mutation case completed. Cause remains
unassigned pending a separate diagnostic control. The candidate is unmerged.

All eight source inputs, seven assembled project inputs, runner, CLI and
reused standard-library identities remain unchanged. The [manifest](manifest.json)
retains complete inputs, raw logs, provenance, the old fixture and API delta.
The same consumer uses explicit Stage23 archive reuse; this is not a fresh
automatic standard-library build or a main distribution build.

Source review, grammar acceptance of 20 files and 173 passing Python tooling
tests establish only source and runner properties. They do not supersede this
runtime failure. The earlier [borrowed-projection failure](../prepared-publication-copy-isolation/README.md)
remains distinct: all five mutations completed there and all five retained
isolation checks failed. Neither result closes the separate language
dereference-copy obligation, privacy/affine checks, durable admission or AOT.
