# 16 — Context System (Dependency Injection)

The context system is Verum's answer to "how do I get a logger /
database / config / clock into deep code without threading it through
every function signature?" It's structural DI — explicit, typed,
compile-time-resolved, no global registry, no service locator.

## The two keywords

| Keyword | Side | Purpose |
|---|---|---|
| `using [T1, T2, ...]` | Function declaration | "I need these in my caller's scope" |
| `provide v1, v2 { ... }` | Call site | "Here are the values for this scope" |

## Slot-based, not HashMap-based

Each context type maps to a fixed compile-time slot index. Lookup
is O(1) array indexing (~2ns), not HashMap lookup (~20ns). The
runtime cost is essentially free.

## Compile-time resolution

A `using [Logger]` function call typechecks ONLY when the call's
dynamic scope has a `Logger` provided. Forgetting to `provide` is a
compile error, not a runtime panic. This makes context dependencies
visible at every level of the call graph.

## When to use

| Pattern | Use context? |
|---|---|
| Logger, metrics, tracer | Yes — pervasive, every layer needs it |
| Database connection | Yes — request-scoped lifetime |
| Config | Yes if request-scoped, no if process-static |
| Math constants | No — module-level `const` |

## Difference from properties (effects)

The context system is **runtime DI** with O(2ns) lookup. Properties
(Pure / IO / Async / Fallible / Mutates) are **compile-time effect
tracking** with 0ns overhead — they don't carry values, just facts
about side effects. Don't conflate the two.
