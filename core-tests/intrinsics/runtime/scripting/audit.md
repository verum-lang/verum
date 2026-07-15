# `intrinsics/runtime/scripting` audit

Module: `core/intrinsics/runtime/scripting.vr` (~379 LOC, ~60 intrinsics) —
the binding surface of the embedded scripting engine (engine lifecycle, eval /
named-entry call, outcome inspection, typed globals, sandbox limits, host
callbacks, shared worlds, structural List/Map outcome marshaling).  The
ergonomic wrapper is `core.script` (Engine / Script / Value); these intrinsics
are what it stands on.

Suite added 2026-07-15 (this folder previously did not exist — the last
mirror-rule violation in the `runtime/` tree).  unit (18) + property (5) +
integration (6).

## 0. Tier contract (load-bearing)

Source evaluation requires the installed compiler hook — present whenever
`verum_compiler` is linked (the CLI, hence the interp test runner), ABSENT in
a stripped AOT test binary.  Documented behaviour there: a FAILED outcome
with `script_engine_last_error_kind == 4` (compiler-unavailable), never a
mis-lowering.  `unit_test.vr::test_aot_reports_compiler_unavailable` pins
BOTH sides; every other test gates on `is_interpreted()` and returns early on
AOT.  This keeps the suite green on both tiers while pinning the real
contract of each.

## 1. What is verified GREEN (interp, probe + suite 2026-07-15)

* Engine lifecycle: new/free, sandboxed new with (mem, fuel, wall) limits.
* `eval` → outcome: is_ok, kind taxonomy (0 Nil / 1 Bool / 2 Int / 3 Float /
  4 Text / 5 List), typed accessors, captured stdout, error text.
* Named-entry `call` + entry-not-found taxonomy (kind 3).
* Error-kind state machine on the engine: 1 (compile) / 2 (runtime) /
  0 (reset by a subsequent success) — pinned as a law.
* Host→script typed globals (int/text/bool/float), last-write-wins,
  per-engine isolation; absent-global defaults (0 / "" / false / 0.0).
* Sandbox instruction-limit abort (fuel 1000 vs 10^6-iteration loop) and
  generous-limit completion.
* Structural List-outcome marshaling: len / elem-kind / elem-int / elem-text;
  len==0 for non-list outcomes.

## 2. Findings / observations

* **Script compile diagnostics leak to HOST stderr** — evaluating a
  syntactically-broken script prints the parse error (`error<E018>: …
  <script>:1:10`) on the host process's stderr in addition to reporting it
  through the outcome.  Debuggability-friendly but embedding-hostile (an
  engine embedded in a TUI/service pollutes its host's stream).  Candidate:
  route through the same capture channel as script stdout.  Logged, not
  fixed here (host-API design decision — `core.script` Engine may want a
  `capture_diagnostics` toggle).
* Host-callback surface (`script_engine_register` / `script_host_call_int`)
  takes a Verum `fn(Int) -> Int` value — exercising it from the conformance
  suite requires passing a function value through an intrinsic boundary;
  covered at the `core.script`/crate level (`script_engine.rs` unit tests).
  Deferred here.
* Shared-world tier (`script_world_*`, `script_set_*`, `script_engine_link*`)
  is the P2 zero-copy surface with its own roadmap
  (`memory: scripting_p2_research_and_roadmap`); its conformance home is
  `core-tests/script/` (wrapper level) once the wrapper suite lands.
  The intrinsic-level suite here pins the outcome/global/error planes the
  wrapper builds on.

## 3. Crate-side drift surfaces

* `crates/verum_vbc/src/interpreter/script_engine.rs` — the engine itself.
* `crates/verum_vbc/src/interpreter/dispatch_table/handlers/script_runtime.rs`
  — name-dispatched intrinsic handlers (the `ScriptValue` tag taxonomy lives
  here; the kind numbers in the tests pin it).
* Compiler-hook installation: present iff `verum_compiler` linked — the tier
  contract of §0.

## 4. Action items

**Landed (this suite)**
* Full intrinsic-level conformance: 29 tests across unit/property/integration.

**Deferred (tracked)**
* Host-stderr diagnostic leak → design decision at `core.script` level.
* Host-callback + shared-world conformance → `core-tests/script/` wrapper
  suite.
