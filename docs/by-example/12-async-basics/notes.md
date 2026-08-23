# 12 — Async / Await

Verum's async model is **structured concurrency** with an eager
execution law:

* An `async fn` is a function that MAY yield at its await points.
  **Calling it runs the body and returns the value** — on both
  tiers. There is no hidden future object behind an async call.
* `.await` is a **yield point**: the runtime may drive other ready
  tasks there before resuming. Awaiting an already-ready value is a
  no-op; awaiting a **spawn handle** waits for that task.
* `Future<T>` is the type of **explicit** futures only — spawn
  handles, timers (`sleep` returns one), and implementors of the
  `Future` protocol. Async fns do not manufacture futures.

## The three primitives

| Primitive | Purpose |
|---|---|
| `async fn` | A function with yield points; calling it runs it |
| `.await` | Yield here; unwraps a spawn handle / timer |
| `spawn(async { … })` | Run a task concurrently; returns a handle |

## Concurrency is explicit

Calling `fetch_user(1)` runs it to completion — sequential, simple,
honest. CONCURRENCY begins where you write it:

```verum
let handle = spawn(async {
    heavy_work().await;
});
// ... other work here runs concurrently with the task ...
handle.await;   // the structured join point
```

A parent does not finish before its spawned children — the model of
Trio, Kotlin coroutines, and Swift. Tasks are cooperative: each
`.await` is a scheduling point, and one OS thread can drive many
tasks as long as they yield at I/O boundaries.

## Sequential vs concurrent

Sequential — `let a = f1().await; let b = f2().await;` — total time
is the **sum**. Concurrent — spawn both, then await both handles —
total time is the **max**.

## The law, pinned

This chapter documents the language's async-semantics law
(`docs/architecture/async-semantics-law.md`): docs, checker, and
both execution tiers agree on the model above. If you ever see an
async call typed as `Future<T>`, that is a bug against this page.
