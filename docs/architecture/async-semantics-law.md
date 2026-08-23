# Async semantics: one law for docs, checker, and both tiers

Status: IMPLEMENTED per §4's recommendation (candidate A — V0
legalized), pending owner ratification; reverting is one change in
`wrap_return_type_for_sig_full` if (B) is ruled instead. The law is
executable: `vcs/specs/L2-standard/async/eager_async_law.vr`
(differential — both tiers must agree), and by-example/12 documents
it (that chapter COMPILES AND RUNS for the first time; it was
written against the never-implemented lazy model and its mount of
`from_millis` never resolved).

## 1. The three-way divergence, measured

| Layer | What it believes | Evidence |
|---|---|---|
| Docs (`docs/by-example/12-async-basics`) | `async fn` returns a `Future<T>` that "doesn't run until awaited" | the chapter text |
| Checker, LOCAL fns | same as docs: calling an async fn from sync yields `Future<T>`; `.await` required | `no method .success() on Future<ShellResult>` on a local twin |
| Checker, MOUNTED (baked) fns | async is ERASED: `sh()` types as bare `ShellResult` | `scheme_from_function_descriptor` ignores `fd.is_async` |
| Tier-0 runtime | **V0**: an async body runs INLINE at the call; `.await` is a pass-through (plus a cooperative pump); only `spawn {…}` makes a real task | `compile_await` (expressions.rs:23847) documents this; `async_sem.vr` probe: body runs before `after call` prints |
| Tier-1 (AOT) | V0, same lowering | same emitter |

Two additional split surfaces, same root:

* the two CHECKER mount paths disagree with each other: bare
  re-export (`mount core.shell.sh`) erases async; direct
  (`mount core.shell.exec.{sh as x}`) wraps in `Future` — one
  function, two types, chosen by spelling of the mount;
* the docs' promised semantics (lazy futures) is implemented
  NOWHERE — both tiers are eager.

## 2. Why this must be one law

Every combination of the current beliefs produces a user-visible
lie: the checker approves `.success()` on a value the runtime
delivers differently (the sh crash), or rejects code that would run
fine (`my_sh(...)` local twin), or the docs teach a model the
machine does not have. Tier-identity (diff-tiers) can only judge
outputs; the type layer must stop disagreeing with the execution
layer first.

## 3. The two coherent candidates

**(A) Legalize V0 now; stage V2 behind a fresh design.**
`async fn` = "a function that MAY yield at await points". Calling
one is calling a function: the body runs, the value returns. `await`
is a yield-point annotation, not a projection. `Future<T>` appears
ONLY as the type of `spawn { … }` handles.

* checker: stop wrapping local async returns in `Future`; keep the
  bare return type for mounted ones (the current metadata behavior
  becomes CORRECT instead of accidentally right);
* docs: rewrite by-example/12 around spawn/join as the concurrency
  entry point (its examples already only await inside `spawn`!);
* cost: ~zero implementation, honest immediately; "async" reads as
  an effect marker (which is what the properties system says anyway:
  ASYNC is a PropertySet bit, not a type constructor).

**(B) Implement the lazy-future model the docs promise.**
State-machine lowering (the `compile_await` comment's "V2"), real
suspend/resume, `Future<T>` as a first-class value in both tiers,
executor semantics for sync→async boundaries (block_on or
compile error at sync call sites).

* cost: a state-machine compiler for both tiers + an executor story
  + migration of every baked async fn; months, touching codegen,
  interpreter, AOT, and the bake;
* the checker's local behavior and the docs become true; every
  CALLER of stdlib async fns (`sh`, `read_to_string_async`, …) must
  gain `.await` or a sync facade.

## 4. Recommendation

**(A) now, (B) as a designed epoch later — if ever.** The language's
concurrency canon (structured, spawn/join/nursery) does not need
lazy futures to be honest; it needs the call model to be the same in
all four layers. V0 is already the shipped machine; legalizing it is
one checker change, one docs chapter, and a law test. If lazy
futures earn their way in later, they arrive as an epoch with an ADR
and a migration, not as a comment in compile_await.

Concrete follow-ups once ruled:
1. checker: drop the `Future` wrap for local async (mirror of the
   mounted path), or introduce it for both + facades — per ruling;
2. rewrite by-example/12 to the ruled model (tours embed it — the
   playground teaches whatever this file decides);
3. law test: async_sem/await_tail probes as a vcs differential spec
   (both tiers, same outputs);
4. re-audit `sh`-style Rust intercepts against the ruled model.

## 5. Related debt this exposed (filed separately)

* 12-line file with one `await` pays a ~2.5-minute lazy stdlib
  supplemental load (T0827 kin) — the cold path, not semantics;
* `Executor`/`JoinPair` duplicate-simple-name layout collisions
  (T0458) sit in the same shell/async neighborhood.
