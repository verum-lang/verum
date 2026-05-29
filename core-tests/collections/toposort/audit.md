# `core.collections.toposort` — implementation audit

## Status: **partial** (under `--interp`)

* `TopoGraph<N: Hash + Eq>` — Kahn's-algorithm topological sort for DAG
  dependency resolution (package managers, build systems, task runners,
  migration ordering, module loaders).
* The **acyclic** path is fully covered and GREEN: construction
  (`new`/`add_node`/`add_edge`/`node_count`/`contains`), the
  topological-order invariant, stability, determinism, and permutation
  laws.
* The **cyclic** path is **broken** — `toposort()` *panics* instead of
  returning `Err(Cycle(...))` for any graph containing a cycle. See §1.

## 1. Defect TOPO-CYCLE-1 — cyclic graphs panic in `toposort()`

**Severity: HIGH** (correctness — a documented public return value is
unreachable; cycle detection is the *raison d'être* of a toposort).

### Symptom
Every cyclic graph crashes the interpreter instead of returning
`Result.Err(TopoError.Cycle(trace))`:

```
$ verum run --interp  (2-cycle: add_edge(1,2); add_edge(2,1); toposort())
error: VBC execution error: Panic: field write out of bounds:
  field index 80 (offset 640+8 = 648) exceeds object data size 16
  type_id=0 type='?'
  backtrace=[BTreeMapKeys.cycle@pc=16
    <- core.collections.toposort.node_label@pc=19
    <- TopoGraph.toposort@pc=739
    <- main@pc=70]
```

Reproduced by a 4-line `verum run` program and by property tests §9–§11
(`prop_toposort_{self_loop,two_cycle,three_cycle,dag_plus_back_edge}_is_cycle`,
all `@ignore`'d in `property_test.vr` pending the fix).

### Root cause
`toposort()` (an `implement<N: Hash + Eq> TopoGraph<N>` method) only enters
its residual-collection branch when `out.len() != self.nodes.len()`, i.e.
when a cycle exists. That branch calls the **generic free function**
`node_label<N>(_: &N) -> Text` (toposort.vr:230) once per residual node:

```verum
if d > 0 {
    residual.push(node_label(n));   // toposort.vr:220
}
```

Calling a generic free fn *from inside a generic `implement` method body*
miscompiles in VBC codegen: the call frame is mislaid and a subsequent
field write lands at field index 80 (offset 648) on a 16-byte object
(`type_id=0`), corrupting memory. The backtrace frame
`BTreeMapKeys.cycle` is spurious — an artifact of the corrupted frame, not
a real call site.

This is a **language-level (VBC codegen) defect**, sister to the broader
"cross-module / generic dispatch frame" class. It is NOT reproducible by
the pattern in isolation — a standalone generic-free-fn-from-a-map-loop
program runs fine. The trigger is specifically *generic-free-fn invoked
from a generic-`implement`-method body*.

### Fundamental fix (two layers)

**Layer A — VBC codegen (the real fix).** Generic free-function calls
emitted from within a generic `implement<N>` method must thread the
caller's monomorphisation frame correctly so the callee's locals/return
slot don't alias the caller's object fields. This belongs in
`crates/verum_vbc/src/codegen` (call lowering / register allocation for
generic-in-generic calls). NOT attempted here: that crate had active
uncommitted WIP from a concurrent session and a rebuild could not be done
safely without racing it.

**Layer B — stdlib source workaround (lands independently, low-risk).**
`node_label` is pure indirection: it discards its `&N` argument and
returns a constant `Text.from("<node>")`. Inlining that constant at the
call site removes the miscompiling call entirely:

```verum
// toposort.vr:218-221, replace
if d > 0 { residual.push(node_label(n)); }
// with
if d > 0 { residual.push(Text.from("<node>")); }
// and delete `fn node_label<N>(...)`.
```

This was verified to be the exact crashing call (it is the innermost
real frame in the backtrace). The workaround was **drafted and reverted
this session** — applying it requires `touch crates/verum_compiler/build.rs
&& cargo build --release` to re-embed the stdlib, which could not be run
safely while a concurrent session was mid-build with dirty codegen. Apply
+ rebuild + un-ignore §9–§11 in a clean checkout. (A richer label needs a
`Debug`/`to_text` bound on `N` — a `toposort_with_labels` follow-up.)

## 2. Pre-existing pins (regression_test.vr)

* §A `contains(&K)` after `add_node` — CLOSED 2026-05-17.
* §B isolated nodes / linear chain / diamond sort correctly — CLOSED.
* §C `add_edge` doesn't duplicate endpoints — CLOSED.
* §D `TopoError.Cycle([Text.from(...)])` equality — `@ignore`'d, blocked
  by inline `[Text.from("...")]` list-literal codegen (`UndefinedFunction`).

## 3. Coverage map

| File | Tests | Status |
|---|---|---|
| `unit_test.vr` | 4 | construction + empty-graph sort — GREEN |
| `regression_test.vr` | 5 active + 1 `@ignore` (§D) | GREEN |
| `property_test.vr` | 9 active + 4 `@ignore` (TOPO-CYCLE-1) | GREEN |
| `integration_test.vr` | cross-stdlib build-order scenarios | GREEN |

## 4. Action items

1. **TOPO-CYCLE-1** — apply Layer B workaround + rebuild, then un-ignore
   the four cycle property tests; track Layer A codegen fix separately.
   (HIGH — cycle detection currently crashes.)
2. **§D** — inline `[Text.from("...")]` list-literal codegen fix unblocks
   `TopoError` equality + the textual cycle trace.
3. `toposort_with_labels` — richer cycle diagnostics once `N: Debug`
   labelling is wired (depends on §D).
