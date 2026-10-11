# Archive Text identity controls

The parsed production `core.text.text.Text` record keeps the runtime Text identity after lazy archive import and bootstrap dependency import. Foreign records named Text, a bundle name alone, and missing declaration ownership do not acquire that identity. Source-local numeric IDs are independent of the consumer ABI. Method-parent identity, genuine empty source bodies, repeated import, both foreign-owner orders, and module serialization are covered.

At `130689955`, the causal baseline passes three refusals and fails five canonical import controls. At `13d6e2f39`, the same eight controls pass. The original six-control baseline is preserved; its source-ID assertion was too strict. The first added bootstrap fixture incorrectly reset imported state with `compile_module`; the corrected fixture uses the production import/collect/compile lifecycle. The initial repair compilation failure used a nonexistent spelling accessor and is retained separately.

`runs.json` retains each receipt with source bindings factored into `source-inputs.json` and per-run replacements. Replacing each receipt's `source_paths` with the merged bindings reconstructs its complete semantic content; the canonical receipt hash uses sorted JSON keys and compact separators. Original receipt hashes, exact commands, environments, executable hashes, artifact identities, all outcomes and raw output hashes are retained. Output files are verbatim Cargo logs; whitespace checks exclude those five files only. The two runners preserve the six-test and eight-test argument bounds.

These are focused parsed-source and VBC archive import controls. No ordinary CLI, automatic bake, AOT or native dependency bootstrap ran. The checker target consumes no precompiled archive; the immutable Stage23 artifacts were independently checked unchanged. Full VBC regression and unchanged ordinary SemVer options acceptance remain separate requirements, together with the supplemental FunctionId repair.

The [main integration check](main/review.json) passes all eight controls
on the isolated main-based branch in 35.223 seconds, with no failures
or ignored tests. Source, retained executable and artifact identities
are verified independently. The implementation and controls match the
reviewed candidate; this does not establish a fresh bake or CLI replay.
