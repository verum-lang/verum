# Checked Rust arithmetic replay

`check_shift.py` extracts the reviewed `Shift` declaration directly from the
native source archive and matches that file to its actual build receipt. The
pinned Rust compiler emits MIR with overflow checks enabled. The translator
retains branch predicates, checked unsigned arithmetic, zero-divisor failure
and the original membership panic. Lean checks exact lasso transport for every
valid 64-bit input and every logical elapsed count. The runner recompiles
semantic mutants and requires a false-proposition diagnostic for each; a build
failure or timeout is inconclusive.

```sh
python3 -m unittest discover -s verification/external/rust_mir
python3 verification/external/rust_mir/check_shift.py \
  --rustc /path/to/nightly-2026-08-21/bin/rustc \
  --lean /path/to/lean-4.35.0/bin/lean \
  --output /new/replay-directory
```

The generic frontend supports an acyclic unsigned scalar subset with immutable
record parameters. Checked add/subtract tuples must be consumed by their
immediate overflow guard before their mathematical result becomes available.
All reachable and declared blocks must be accounted for. Unsupported calls,
mutable references, loops, operations and types are rejected. No assumed
contracts, smaller arithmetic domain or substituted remainder routine is used.

The trusted boundary includes rustc, its runtime, the reviewed source selection
and field layout, and `mir_to_lean.py`. Lean validates the translated arithmetic;
it does not verify this frontend, Rust's backend or the resulting machine code.
MIR is compiler-version dependent, so an unreviewed compiler is refused. The
receipt retains original source, compiler identity, MIR, generated proofs,
model, axiom audits and semantic-control diagnostics. This is a primitive
sequential correspondence result; caller ownership, dispatch, scheduling and
world-model descent need their own evidence.
