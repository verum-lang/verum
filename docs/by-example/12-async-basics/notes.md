# 12 — Async / Await

Verum's async model is **structured concurrency**: every async task
has a parent that owns it, and a parent doesn't return until all its
spawned children have finished or been explicitly cancelled. This
matches the model from Trio, Kotlin coroutines, and Swift async/await.

## The three primitives

| Primitive | Purpose |
|---|---|
| `async fn` | Declares a function that returns a `Future<T>` |
| `.await` | Suspend until the future resolves |
| `spawn(f)` | Run a future on the runtime; returns a handle |

## Why not threads

Async tasks are **cooperative** — each `.await` is a suspension point
where the runtime can park the task and reuse the OS thread for
something else. A single OS thread can drive thousands of tasks as
long as each yields at I/O boundaries. Verum's task is ~150ns to spawn
vs. ~5μs for an OS thread.

## Sequential vs parallel awaits

Sequential — `let a = f1().await; let b = f2().await;` — runs `f1`
and `f2` one after another. Wall-clock time is the **sum** of the two.

Parallel — `let (a, b) = join(f1(), f2()).await;` — runs both
concurrently. Wall-clock time is the **max** of the two. Use `join`
whenever the two futures are independent.

## Structured concurrency

`spawn` requires a runtime to run on; the runtime owns its tasks. When
the runtime shuts down, every outstanding task is cancelled — no
"forgotten task running forever in the background" pattern.

For multi-task fan-out with shared cancellation, use a `Nursery`
(see `core.async.nursery`):

```verum
nursery.run(|n| {
    n.spawn(fetch_user(1));
    n.spawn(fetch_user(2));
    n.spawn(fetch_user(3));
    // All three complete or all three cancel — no orphans.
});
```
