# Native checker rules

`check.py` compiles the production `verum_proof` library with the workspace
lockfile. Charon extracts the call graphs rooted at `lookup_binding` and
`universe_successor`. Aeneas translates both to Lean. The runner compares
the fresh translation byte for byte, then checks the translation and both
proofs. [Generated code](VerumProof.lean)

## Variable lookup

Both native checkers infer the type of a de Bruijn variable by looking up its
context entry. Their context lookup methods call `lookup_binding` in
[`binders.rs`](../../crates/verum_proof/src/binders.rs).

For every representable `usize` depth and index, returning a slot is equivalent
to `slot + index + 1 = depth` in natural-number arithmetic. The function returns
`None` exactly when `depth ≤ index`, and exchanging the index and slot inverts
the mapping. For every slice and every element type, the shared Rust lookup
returns the optional element at that index in the reversed list. This equality
includes termination and successful bounds checks in the extracted model.
[Variable lookup proofs](BindingSlot.lean)

## Universe successor

Both native T-Universe branches normalize the input level and pass the result
to `universe_successor`. For every input level and every valuation of its
parameters in the natural numbers, the returned level denotes precisely the
input value plus one. The underlying `level_with_offset` proof covers every
`u128` shift. It proves that the chunk loop terminates, all checked machine
operations succeed, and the result denotes the input value plus the shift.
This includes concrete and symbolic levels above the `u32` wire maximum.
[Rust implementation](../../crates/verum_proof/src/proof_checker/universe_offset.rs),
[universe proofs](UniverseOffset.lean)

## Correspondence

Lean checks the transitive axiom dependencies of the theorems against
`propext`, `Classical.choice` and `Quot.sound`.

The correspondence trusts the pinned Rust compiler, Charon, Aeneas and its
models of machine words, borrowed slices and checked indexing. After binding
lookup, the substitution checker clones or weakens the binding type, and the
normalization-by-evaluation checker reads the referenced semantic type.
The preceding native universe normalization, these binding operations,
allocation limits and whole-certificate acceptance remain outside these proofs.
Extracting the complete checkers currently stops at the Rust coroutine in
each inference routine.

## Replay

Use the archive for your host from the [pinned Aeneas release](https://github.com/AeneasVerif/aeneas/releases/tag/nightly-2026.11.17-1dc8ad3).
Verify its SHA-256 against [the toolchain pins](toolchain.json). The archive
contains the matching Charon, Aeneas and Lean backend. Install the pinned
Rust and Lean compilers, then make the backend's locked Lake dependencies
available. For a fresh checkout, run `lake exe cache get` and `lake build Aeneas`
in the release's `backends/lean` directory using its `lean-toolchain`.

From the Verum repository root, point the runner at the unpacked release, the
concrete Lean executable and an existing scratch directory. The runner creates
a temporary private Cargo target and Lean module directory there.

```sh
python3 verification/native-checker/check.py \
  --release "$release" --lean "$lean" --scratch "$scratch"
```
