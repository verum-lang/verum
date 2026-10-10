# Parser inputs in automatic archive identity

The fixed source is `75936d876`, based on `e86f2abd7`. The
[manifest](manifest.json) binds the exact tested sources, retained test
executables, raw receipts and logs. Compressed files preserve the original
bytes and their uncompressed hashes.

The initial attempt stopped at two test-only `Text.lines()` iteration errors;
no test executed. After the identical correction, the causal baseline passed
the real bootstrap alias archive control and two cache controls, while five
parser input/dependency controls failed. The fixed source passes all eight
controls with identical test bytes. The original flat baseline receipt also
counts a child test summary printed inside an expected assertion failure;
`target-accounting.json` keeps the two actual Cargo invocations separate.

The no-auto control directly executes the build script in bounded child
processes with isolated output paths and an empty executable search path. It
covers presence of `VERUM_NO_AUTO_PRECOMPILE` with values `1` and `0`, and
`DOCS_RS`; dependencies remain reported and no target/cache is created. This
does not measure an environment-only Cargo rerun transition (T1735).

The alias control uses actual `compile_core` discovery, parsing and archive
writing on a tiny source tree, then decodes the disk archive and metadata.
It confirms declared parameter slots and keeps an unrelated marker variant.
The parser prerequisite is T1720; it must accompany an independent main landing.

The inherited full stdlib artifact set comes from the successful e86 v54
ordinary build and remains byte-identical in every run. It was not rebaked by
these gates. Full automatic archive refresh, ordinary CLI and registry runtime
acceptance remain separate from this focused result.
