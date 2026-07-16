# `core.sys.linux.bpf` — implementation audit (umbrella module)

## Status: **partial** (full unit + property + integration + regression under `--interp`; AOT pending D1 umbrella re-export fix; syscall path deferred to Linux runner)

* Acts as the entry-point for the four submodules `error`, `program`,
  `map`, and (via mount-import) the `BpfError` typed-error funnel.
* Re-exports every public name through the umbrella so callers can
  `mount core.sys.linux.bpf.{ProgramType, MapType, BpfError}` without
  knowing about the submodule structure.

## 1. Cross-stdlib usage

The umbrella module is the documented entry-point for every BPF caller
inside / outside stdlib.  Direct imports against `core.sys.linux.bpf.error`,
`core.sys.linux.bpf.program`, `core.sys.linux.bpf.map` are supported as
escape hatches but not the canonical surface.

## 2. Action items landed in this branch

1. `unit_test.vr` — 9 `@test`s pinning that every documented re-export
   path resolves at compile time and constructs at runtime via the
   umbrella name (BpfError / ProgramType / AttachType / ProgramDescriptor /
   ProgramBytecode / Link / MapType / MapDescriptor / MapConfig).  A
   regression that drops any re-export from `core/sys/linux/bpf/mod.vr`
   would cause the corresponding `@test` to fail at compile time.

2. `property_test.vr` — umbrella-surface laws: every name imported ONLY
   through the umbrella (`core.sys.linux.bpf.{ProgramType, AttachType,
   MapType, ProgramDescriptor, Link}`) constructs, dispatches under
   `match`/`is`, and round-trips through records — i.e. an umbrella-only
   import is self-sufficient (no submodule path needed). FileDesc field
   values asserted via the `==` operator (D6 idiom).
3. `integration_test.vr` — a whole XDP-firewall assembly (program
   bytecode + LPM-trie blocklist map + XDP link) built **entirely**
   through umbrella imports, plus a `BpfError` funnel through `Result`.
4. `regression_test.vr` — pins **D1** (every umbrella-re-exported name
   resolves under strict compilation, not just the lenient interpreter),
   re-export *identity* (one canonical type, not two copies), and **D6**
   (FileDesc value via `==`).

### Import-aliasing limitation (observed, not pinned)

An early draft of `property_test.vr` dual-imported the SAME canonical type
under two names — `mount core.sys.linux.bpf.{ProgramType as U}` AND
`mount core.sys.linux.bpf.program.{ProgramType}` — to assert the umbrella
alias `U` is *interchangeable* with the submodule type. Under `--interp`
this FAILED: the checker treats the alias as a DISTINCT type (a value of
`U` did not match a `ProgramType.<variant>` arm, and constructing the
aliased record drifted its layout — `field write out of bounds: field
index 3 ... size 24`). This is an import-aliasing identity limitation
(one canonical type imported under two names is not unified), NOT an
umbrella-resolution defect — the umbrella surface itself is sound
(umbrella-only imports pass). The property test was rewritten to test the
real contract (umbrella-only self-sufficiency) rather than the unsupported
dual-import pattern. Tracked here for the language-side import-unification
work.

### D1 — umbrella re-export resolution (cross-tier divergence)

`core/sys/linux/bpf/mod.vr` re-exports its submodule names via
`public mount .program.ProgramType;` etc. The Tier-0 interpreter resolves
umbrella-imported FREE FUNCTIONS via a lenient global function table even
when the strict import-resolution path fails to bind them; the Tier-2 AOT
type-checker is strict and rejected them as `E100 unbound variable`. Root
cause + fix tracked in
`website:docs/stdlib/defect-class-catalogue.md` (D1 — umbrella
re-export resolution). The umbrella TYPE re-exports (the names this module
exposes) resolve via the type registry on both tiers; the function
re-exports (`load_program`, `attach_*`, `create_map`, `map_*`) are the
ones gated on D1 — and are also un-runnable off-Linux (deferred below).

## 3. Action items deferred

| # | Defect / gap | Notes |
|---|---|---|
| 2 | Function re-exports (`load_program`, `attach_*`, `create_map`, `map_*`) | Cannot exercise on a non-Linux host; deferred to the integration suite. AOT umbrella resolution of these free-function re-exports tracked under D1. |
