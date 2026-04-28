# 15 — Refinement Types

Refinement types attach a predicate to a base type. The predicate is
discharged by the SMT solver (Z3) at compile time, so a value of type
`Int{> 0}` is a *proof* that the integer is positive — no runtime
check needed.

## Syntax

```verum
Int{> 0}             // strictly positive
Int{>= 0}            // non-negative
Int{!= 0}            // non-zero
Int{> 0 && < 100}    // bounded range
&List<T>{len > 0}    // non-empty list reference
Text{utf8_valid}     // user-defined predicate
```

## What the verifier proves

When a caller passes `e` to a function expecting `Int{> 0}`, the
verifier asks Z3: "does the caller's path imply `e > 0`?" If yes,
the call typechecks. If no, the caller must:

- Pattern-match: `if e > 0 { f(e) } else { ... }`
- Refine via assertion: `assert(e > 0); f(e)`
- Change the input type so `e: Int{> 0}` flows through naturally.

## Where refinements live in the codebase

The stdlib uses refinements heavily for division precondititions:

```verum
public fn div_floor(a: Int, b: Int{!= 0}) -> Int { ... }
public fn mod_euclidean(a: Int, b: Int{!= 0}) -> Int{>= 0} { ... }
```

A caller writing `div_floor(10, x)` where `x: Int` must prove `x != 0`
or the compiler rejects the call.

## Limitations (current refinement system)

- **Per-parameter only**: `Int{!= 0}` works. `Int{!= 0 && a != Int.MIN}`
  (a relational predicate referencing another parameter `a`) does not
  yet — the verifier doesn't accept cross-parameter refinements.
  Workaround: runtime guards (see `core.math.integers.check_signed_div_overflow`).
- **Decidable predicates only**: arithmetic + boolean logic + equality
  decide cleanly; arbitrary first-order logic doesn't.
- **No higher-rank refinements**: `fn<T> f(x: T{p(x)}) -> ...` doesn't
  yet, but rank-2 polymorphism is on the roadmap (cf. transducers).

## When to use vs. avoid

Use when the precondition is **load-bearing** — anything where a bad
input causes silent wrong results, panics, or security regressions.
Division-by-zero and Int.MIN-as-divisor are textbook cases.

Avoid when the precondition is **stylistic** or easily checked at
runtime with negligible cost. Refinements pay their compile-time cost
in solver time; reserve them for real invariants.
