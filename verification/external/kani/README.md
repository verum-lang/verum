# Source-bound Rust checks

`rust_replay.py` runs Kani 0.68.0 over an immutable standalone Rust workspace.
The caller supplies a manifest with exact source hashes, byte-range assembly,
an entry file, the complete harness roster and required reachability covers.
Whole original sources, selected fragments, assembled files, the runner,
toolchain hashes, commands, raw results and logs are retained in a new output
directory. Reusing an evidence directory is refused.

This checks the selected implementation for all inputs admitted by each harness,
including full-width machine integers. It does not infer that the harness is an
adequate specification or that the selected files cover an entire application.
Review source selection, macro expansion, module dependencies, input assumptions
and entry-point reachability. These are trusted project build inputs, not a
sandbox for hostile Rust code. Use the project's build sandbox and resource guard.

The runner fixes safety checks, assertion reachability and unwinding assertions.
It permits no extra compiler/solver flags or stub features. Every requested
harness, property and cover must be present; an unreachable contract assertion,
missing cover, process failure or incomplete result cannot establish success.
Unreachable standard-library checks are retained explicitly. Satisfied covers
establish those cases' reachability, not completeness of a specification.

Kani compiles Rust MIR and uses CBMC. Its `Arc` operations are sequential here;
this profile establishes no concurrent interleaving theorem. Compiler lowering,
the pinned standard-library/allocator model and solver remain trusted. This
external symbolic evidence is distinct from a Verum or Lean kernel certificate.
Matching source hashes alone do not prove correspondence to a mathematical model.

Run the executable positive, semantic mutation and nonvacuity controls:

```sh
python3 -m unittest discover -s verification/external/kani
python3 verification/external/kani/check_integration.py \
  --kani /path/to/kani-0.68.0/bin/kani-driver --output /new/evidence-directory
```

`check_integration.py` illustrates the manifest format and executes the actual
checker. Its deliberately wrapping implementation must produce an assertion
counterexample; its excluded maximum must produce an unsatisfied cover even when
Kani reports successful assertions. Tool/parser failures never satisfy controls.

`check_native_clock.py --kani /path/to/kani-driver --output /new/evidence-directory`
checks the actual native clock module against arbitrary full-width fields and
retained `Arc` owners. Its immutable sources match the native build receipt in
the Lean source-cycle corpus. Restart refusal, fresh origins, exact consumed
edges and terminal/tick separation have required reachability cases and semantic
mutation controls. The enclosing application's independent verdict is retained;
these primitive contracts do not certify its callers or external world behavior.

See [Kani usage](https://model-checking.github.io/kani/usage.html) and
[loop unwinding](https://model-checking.github.io/kani/tutorial-loop-unwinding.html)
for the underlying verification boundary.
