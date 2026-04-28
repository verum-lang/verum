# 17 — Error Recovery

Verum's error model is `Result<T, E>` — fallible operations return
either `Ok(value)` or `Err(error)`. There are no exceptions, no
checked-exception hierarchies, no implicit unwinding through normal
function calls.

## The five primitives

| Form | Use |
|---|---|
| `Ok(v)` / `Err(e)` | Construct a Result |
| `match r { Ok(v) => ..., Err(e) => ... }` | Explicit handling |
| `r?` | Propagate Err, unwrap Ok |
| `r.unwrap_or(default)` | Get value or fallback |
| `r.map(f).map_err(g)` | Functorial transforms |

## The `?` operator

`expr?` is shorthand for:

```verum
match expr {
    Ok(v)  => v,
    Err(e) => return Err(e.into()),
}
```

The `into()` lets callers compose errors across types as long as a
`From` conversion exists. Most stdlib errors derive `From` from each
other where the conversion is natural.

## When to use Result vs panic

- **Result** — anything the caller might reasonably handle. Parse
  errors, network failures, file not found, validation failures.
- **panic** — invariants the type system can't yet express. Out-of-
  bounds index, integer-overflow-on-Int.MIN, kernel CSPRNG
  unavailable. Reaching a panic means the program is already in an
  inconsistent state; running on with a fallback would be worse.

## Supervisors (advanced)

Long-running services use `core.runtime.supervisor` for **fault-
tolerant restart loops** — Erlang-style supervision trees where a
child task failing causes the supervisor to restart it (with
exponential backoff and circuit breakers). See chapter 21+ for the
supervisor model in depth.
