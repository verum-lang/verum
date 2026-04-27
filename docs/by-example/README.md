# Verum by Example

A curated walkthrough of Verum syntax and semantics. Each example
is a runnable `.vr` file paired with a `notes.md` that explains
what the code is doing and why.

The path through the examples is sequential — each one introduces
~1-2 new concepts and uses everything covered before. If something
isn't explained, look back at the earlier example.

## Index

| #   | Example          | Concepts introduced                                        |
| --- | ---------------- | ---------------------------------------------------------- |
| 01  | hello-world      | `fn main`, `print`, comments, file structure               |
| 02  | types            | `let`, `Int` / `Text` / `Bool`, `type` declarations        |
| 03  | pattern-match    | `match`, sum types (`is A \| B`), exhaustiveness           |
| 04  | result-error     | `Result<T, E>`, `Maybe<T>`, `?` propagation, error types   |
| 05  | collections      | `List<T>`, `Map<K,V>`, iteration, `for-in`, `mount` system |
| 06  | functions        | block-body, expr-body, default params, Unit return         |
| 07  | mutability       | `let mut`, `&self` / `&mut self`, reference tiers          |
| 08  | protocols        | `protocol`, `implement P for T`, generic bounds            |
| 09  | generics         | multi-param generics, `<K: Eq>` bounds, `Maybe<&V>`        |
| 10  | mount-system     | braced / glob / aliased / single-symbol mounts             |

## Running an example

Each example directory has a `main.vr` and a `notes.md`. Run with:

```bash
verum run docs/by-example/01-hello-world/main.vr
```

Or run the whole walkthrough as a smoke test:

```bash
make examples-test    # (target shipping with #182 Phase D)
```

## Reading an example

Each `notes.md` follows this template:

```markdown
# Example NN — <name>

## What it does
One-paragraph summary.

## What's new
List of language constructs introduced for the first time.

## The code
Walkthrough of `main.vr` line-by-line, only on lines that introduce
something new or that have a non-obvious meaning.

## Things to try
2-3 small modifications a reader can make to test their understanding.

## Reference
Links into `docs/reference/` for the constructs used.
```

## Roadmap

- 5 starter examples (this PR — Phase B starter)
- 10 starter total (next batch)
- 20 intermediate (`generics`, `protocols`, `mount-system`,
  `async`, `panic-safety`, `unsafe-borrows`, `verification`,
  `meta-programming`, `dependent-types`, `cbgr-references`)
- 20 advanced (full async runtime, supervisor trees,
  hostile-input parsing, formal-method certificates, FFI,
  GPU compute, theory interop)

## Contributing

When you add a language feature, the contract per `RELEASES.md`
§8 requires a corresponding example here.

A good example:
- Compiles cleanly with `verum check`
- Runs with `verum run` and produces deterministic output
- Has a `notes.md` that someone can read in under 5 minutes
- Doesn't introduce more than 2 new concepts

A bad example:
- Demonstrates 5 features at once
- Requires reading the spec to follow
- Has output that varies across runs (RNG without seed, system time, etc.)
