# Archive reference documentation alignment

Website commit `cf89dcead9af8c431e439148cf311b5b85c23aee` corrects
`docs/stdlib/archive.md` against language source `271d15062` and its grammar.
It uses current exported signatures, removes nonexistent composition APIs,
and describes the source-visible adapter and metadata limitations. The
page remains a reference with regression-only status; it makes no new
runtime or conformance claim.

Type checking, freshness labels, links, internal artefact references and
the strict production build all pass. The build takes 73.570 seconds.
Independent parent verification checks 2,015 source bindings across five
gates, six language source bindings, and 13 rendered content/anchor/link
checks. The [manifest](manifest.json) binds the exact receipts, logs,
review corrections and rendered pages stored as lossless gzip payloads.

The clean website main branch was fast-forwarded to this commit. No push
or deployment was performed.
