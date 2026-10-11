# Supplemental archive function identity

Archive-local function IDs now map to one consumer ID for the lifetime of a codegen context. Supplemental name registration reuses that mapping. The shared archive cache no longer owns compilation-local merge state; the codegen's emitted function table decides whether a body is already present. Every supplemental merge receives the complete entry map and remaps callable signatures before skipping existing bodies.

The ordinary diagnostic retained under `registration-diagnostic/` identified the original failure: `core.text.Text.concat` changed from ID13421 to ID53681, while its bare alias kept the original ID. The newly registered ID received a generated `RetV` stub. That diagnostic changes the options fixture to print selected calls and uses the immutable Stage23 CLI and archive. It is causal observation, not unchanged SemVer options acceptance. Canonical Text receiver identity is independently tracked by T1745.

The corrected parsed-source baseline at `d6f389822` passes one control and fails seven. Failures include changed qualified IDs, a decoded call losing its original target, interleaved compilations sharing incorrect state, and a nominal return retaining archive TypeId19 instead of consumer TypeId20. The repair at `244bdecc7`, plus the same-codegen reset control at `2565d34b4`, passes all nine controls. The source producer, archive codec, primary and supplemental loaders, module codec and interpreter participate in these controls; foreign same-leaf owners and genuine empty source bodies remain distinct.

Earlier attempts are preserved separately:

- `baseline`: test compilation failed because a Value accessor was misspelled; no tests ran.
- `corrected-baseline`: all six controls stopped at a fixture discovery precondition because the qualified primary call imported the whole entry.
- `preflight-refusal.json`: the unchanged 4 GiB headroom guard refused before Cargo started.
- `eight-baseline`: five causal failures and two later-wave failures masked by an overstrict copied-body-count assertion. Metadata-only waves can legitimately copy no new body.
- `metadata-wave-baseline`: that count assertion is removed; all seven failures reach identity, signature or call-closure assertions.
- `compilation-owned-fixed`: nine controls pass in 166.835s, including compilation and test execution. No guard interruption occurred.

The retained final executable also passes 16 adjacent archive controls, with one existing ignored registration-scaling measurement. The five filter counts and outcomes are recorded in `adjacent.json`. The combined direct-executable run takes 3.484s; it performs no Cargo rebuild.

`runs.json` factors 5,317 common source bindings into `source-inputs.json`. Overlay each run's replacements and restore its `source_paths` field to reconstruct the complete semantic receipt; its canonical hash uses sorted JSON keys and compact separators. Exact commands, features, environments, source trees, raw-log hashes, executable identities, artifacts and all outcomes remain recorded. `files.json` hashes the retained evidence bytes. Raw Cargo output and diagnostic logs retain original whitespace.

`unit-provenance/` records the selected Cargo unit graph and the five exact local fingerprint markers invalidated before the first baseline. All marker companions were retained and hashed before removal. Solver and LLVM native outputs, source mtimes, standard-library archives and unrelated entries were not invalidated. Rust wrapper rebuilds against the existing LLVM installation are distinct from native bootstrap.

These gates embed the unchanged Stage23 v54 artifacts under `VERUM_NO_AUTO_PRECOMPILE=1`; each run preserves all five artifact hashes. They establish focused compiler-loader behavior. Full VBC regression and the unchanged ordinary SemVer options replay remain separate. The primary pass2 delta-map fallback is unchanged and has no new acceptance claim. No fresh automatic bake, AOT or registry-service acceptance is claimed.
