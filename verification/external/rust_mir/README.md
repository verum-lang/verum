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

## Record reads and exclusive updates

`record_mir.py` extends the same checked scalar semantics to one shared or
exclusive record reference. Shared receivers cannot write. Methods may return
unit, Boolean or `usize` values. Field order, method membership, nominal enums and supported
field types come from rustdoc for the same compilation input. Every other
field remains an arbitrary, universally quantified frame value. No container
is replaced by an empty stand-in. Reading, writing or dropping an uninterpreted
field, additional references, calls and cyclic control flow are refused.
Return and panic both retain the complete resulting record, including writes
that precede a failed arithmetic check.
Writes use immutable scalar bindings; each return or panic reconstructs the
record once. This retains reads before later writes without expanding a chain
of record updates during kernel reduction.

Rustdoc enables `cfg(doc)`, so its declarations alone cannot authorize a native
projection. The adapter appends an exhaustive record reconstruction and enum
matches to the unchanged selected source, then compiles these witnesses with
the same options as native MIR. Rust checks field and variant completeness;
the frontend checks native field names, indices and types. The selected method's
MIR must also remain identical when the witnesses are present. Missing witnesses,
configuration drift or unexpected lowering refuse admission. The witnesses are
never executed and supply no assumed behavioral contracts. This is a separate
source compilation, not metadata claimed to come from the original native build.


`check_constructor_merge.py` selects the unchanged `Constructor::merge` method
and its complete original type declarations from the frozen native sources.
The manifest retains reviewed byte ranges and both the build and run receipts.
The independent Lean proof establishes full-record equality for every input
and retained frame, then connects that result to the shared wake model when
its enabling condition holds. Compiled semantic mutations must reach actual
false-proposition diagnostics. Separate native controls check retained
allocations and `Arc` owners; these finite controls are not the universal proof.

The `birth-tickets` profile separately pins the newer constructor declarations,
`merge` and shared `idle` getter. Eligibility uses the actual sleeping flag,
not a phase-name inference. The complete retained frame includes building,
admitted and active tickets, heap state, tuple and accounting. Getter and reset
compose into a proved per-visit model transition. This composition does not
extract or prove the enclosing `Expressions::wake` loop. The frozen producer
receipt records Rust 1.95 and retains its failed overall verdict; MIR from the
pinned nightly compiler is explicitly a new source replay, not original build
output. A corrected producer requires its own reviewed source binding.

```sh
VERUM_REPLAY_RUSTC=/path/to/nightly-2026-08-21/bin/rustc \
VERUM_REPLAY_LEAN=/path/to/lean-4.35.0/bin/lean \
python3 -m unittest discover -s verification/external/rust_mir
python3 verification/external/rust_mir/check_constructor_merge.py \
  --rustc /path/to/nightly-2026-08-21/bin/rustc \
  --lean /path/to/lean-4.35.0/bin/lean \
  --output /new/record-replay-directory
# Add --profile birth-tickets for the separately frozen birth-ticket source.
```

This experimental Python adapter is trusted extraction/orchestration tooling,
not Verum's proof kernel. It does not feed a checked native Verum certificate
through either native checker. Its semantic translation remains an integration
gap for the typed Verum verification pipeline.

This is a source-derived primitive correspondence result, not a native Verum
certificate or proof of the entire Rust wake loop. It assumes a valid exclusive
reference and trusts the compiler, rustdoc, reviewed source selection and MIR
frontend. Heap indexing, `Arc` operations, caller invariants, scheduling and
machine-code lowering still need their own correspondence proofs. The broader
native build/run receipts retain their original failed statuses; proving this
method does not turn those runs into successes.
