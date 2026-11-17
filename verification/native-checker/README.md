# Native checker binding lookup

Both native checkers infer the type of a de Bruijn variable by looking up its
context entry. Their context lookup methods call `lookup_binding` in
[`binders.rs`](../../crates/verum_proof/src/binders.rs).

`check.py` compiles the production `verum_proof` library with the workspace
lockfile. Charon extracts the call graph rooted at `lookup_binding`. Aeneas
translates the two Rust functions, including both machine subtractions and
the checked slice read. The runner compares the fresh translation with
[`VerumProof.lean`](VerumProof.lean) byte for byte and invokes Lean on it
and on [the proofs](BindingSlot.lean).

For every representable `usize` depth and index, returning a slot is equivalent
to `slot + index + 1 = depth` in natural-number arithmetic. The function returns
`None` exactly when `depth ≤ index`, and exchanging the index and slot inverts
the mapping. For every slice and every element type, the shared Rust lookup
returns the optional element at that index in the reversed list. This equality
includes termination and successful bounds checks in the extracted model.
Lean checks the transitive axiom dependencies of each theorem against
`propext`, `Classical.choice` and `Quot.sound`.

The correspondence trusts the pinned Rust compiler, Charon, Aeneas and its
models of machine words, borrowed slices and checked indexing. It concerns
the contents of the returned binding. The substitution checker subsequently
clones or weakens its type, and the normalization-by-evaluation checker reads
the referenced semantic type. Those steps and whole-certificate acceptance
remain outside this proof. Extracting the complete checkers currently stops
at the Rust coroutine in each inference routine.

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
