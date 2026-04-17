<div align="center">

<img src="media/verum-logo.png" alt="Verum" width="96" height="96"/>

# Verum

**A verifiable systems programming language.**

Refinement types discharged by SMT · three-tier memory safety (CBGR) ·
capability-based contexts · dependent types with cubical HoTT · a single
bytecode IR that runs under both interpreter and AOT native ·
structured concurrency with OTP-style supervision · a standard library
written in Verum, without a libc or Rust-runtime dependency.

[**Documentation**](https://verum-lang.org/docs/intro) ·
[**Language tour**](https://verum-lang.org/docs/getting-started/tour) ·
[**Blog**](https://verum-lang.org/blog) ·
[**Grammar**](https://verum-lang.org/docs/reference/grammar-ebnf)

</div>

---

## What it is

Verum is a statically-typed compiled systems language built around one
rule — **semantic honesty**: every name, every syntax form, every
annotation reflects what the compiler does with it. No exceptions
that unwind silently past function boundaries, no `static` lifetime
that means "statically checked", no ambient globals, no `!`-suffix
magic. Three reserved keywords (`let`, `fn`, `is`); everything else is
contextual.

The type system ships with refinement types (`Int { self > 0 }`)
discharged by an SMT backend; seven gradual-verification strategies from
`@verify(runtime)` up to `@verify(certified)`; dependent types with
sigma bindings and cubical path equalities; and a proof DSL with
twenty-two named tactics. Memory safety uses **Capability-Based
Generational References** — three reference tiers (`&T`, `&checked T`,
`&unsafe T`), chosen per use site, promoted automatically by escape
analysis. Dependency injection is a language feature (`using
[Database, Logger]`) that unifies runtime and compile-time contexts.
Metaprogramming is the same language, staged.

The compiler is written in Rust (25 crates). The standard library is
written in Verum, with zero `libc`, `pthread`, or Rust-std dependency
— system calls go through VBC opcodes directly. The runtime ships in
five profiles from `full` (servers) down to `embedded`
(microcontrollers).

## Features at a glance

| Area | What's in the shipped release |
|---|---|
| **Type system** | Refinement types (`Int { self > 0 }`), dependent types (Σ/Π), cubical path equalities (`Path<A>(a, b)`) with computational univalence, higher-kinded types, rank-2 polymorphism, protocols with associated types and specialisation. |
| **Verification** | Seven `@verify(...)` strategies (`runtime`, `static`, `fast`, `formal`, `thorough`, `certified`, `synthesize`); capability-routed SMT backend; 22-tactic proof DSL; exportable proof terms for Coq, Lean, Dedukti, Metamath. |
| **Memory** | Three-tier references (`&T` / `&checked T` / `&unsafe T`); CBGR with 16 B `ThinRef` / 32 B `FatRef`; 8 capability bits (`CAP_READ`/`WRITE`/`EXECUTE`/`DELEGATE`/`REVOKE`/`BORROWED`/`MUTABLE`/`NO_ESCAPE`); generation-based use-after-free detection; 11 CBGR analyses auto-promote `&T` to `&checked T` where provable. |
| **Concurrency** | `async`/`await`, `spawn`, `nursery { ... }` structured concurrency, `select` / `race` / `join_all`, channels, streams, async generators; `spawn_with` for retry / circuit breaker / restart policy / isolation / priority / timeout. |
| **Runtime** | Unified `ExecutionEnv` (θ+) per task — memory + capabilities + error recovery + concurrency in one 2 560-byte structure; five profiles: `full` / `single_thread` / `no_async` / `no_heap` / `embedded`. |
| **Supervision** | OTP-style supervision: `OneForOne` / `OneForAll` / `RestForOne` / `SimpleOneForOne`; `Permanent` / `Transient` / `Temporary` restart policies; `RestartIntensity`; escalation to parent supervisor. |
| **Contexts** | Capability-based DI (`using [Database, Logger]`) — not algebraic effects. 14 compile-time meta-contexts (`TypeInfo`, `AstAccess`, `Schema`, `CompileDiag`, `Hygiene`, `BuildAssets`, …) + 10 standard runtime contexts (`Logger`, `Database`, `Auth`, `Config`, `Cache`, `Metrics`, `Tracer`, `Clock`, `Random`, `FileSystem`). |
| **Error handling** | `Result<T, E>`, `Maybe<T>`, typed `throws E`, `try`/`recover`/`finally`, `defer`/`errdefer`, `?` propagation. |
| **Metaprogramming** | Staged `meta fn`, `quote { ... }`, multi-stage `meta(N)` / `quote(N)`, `lift()`, 40+ attributes. Tagged literals (`sql#`, `json#`, `rx#`, `url#`, …) validated at compile time. |
| **Execution** | Two modes — VBC interpreter and AOT via LLVM (CPU) or MLIR (GPU targets: PTX, HSACO, SPIR-V, Metal). No JIT. Zero-FFI path via VBC opcodes `0xF1`/`0xF2`/`0xF4`/`0xF5` for syscalls, atomics, I/O, and clocks. |
| **Cog distribution** | `.cog` archives carry VBC bytecode plus optional proof certificates, validated offline against declared capabilities. |

## A first look

```verum
fn main() using [IO] {
    print("Hello, Verum!");
}
```

```bash
verum run hello.vr
```

A more representative example — refinement types in the signature, an
explicit context, a postcondition the SMT solver discharges at compile
time, no runtime cost for any of it:

```verum
type Port      is Int  { 1 <= self && self <= 65535 };
type NonEmpty<T> is List<T> { self.len() > 0 };

@verify(formal)
fn find<T: Eq>(xs: &NonEmpty<T>, key: &T) -> Maybe<Int>
    where ensures result.is_some() => xs[result.unwrap()] == *key
{
    for i in 0..xs.len() {
        if xs[i] == *key { return Maybe.Some(i); }
    }
    Maybe.None
}
```

## Building from source

**Prerequisites.** A Rust toolchain at the version pinned in
`rust-toolchain.toml` and LLVM 21. The current SMT backend bundles
Z3 and CVC5 as build-time dependencies; the language itself is not
committed to this choice and a Verum-native solver is on the
roadmap. Platform-specific notes are in the
[installation guide](https://verum-lang.org/docs/getting-started/installation).

```bash
git clone https://github.com/verum-lang/verum
cd verum
cargo build --release

./target/release/verum run  examples/showcase.vr
./target/release/verum build examples/showcase.vr -o showcase
```

## Documentation

The content that used to live in this README now lives on the
website:

- [**Introduction**](https://verum-lang.org/docs/intro) — the map of
  the documentation.
- [**Language tour**](https://verum-lang.org/docs/getting-started/tour) —
  a ten-minute walk through.
- [**Language reference**](https://verum-lang.org/docs/language/overview) —
  syntax, types, memory, context system, async, metaprogramming,
  proof DSL.
- [**Standard library**](https://verum-lang.org/docs/stdlib/overview) —
  every `core.*` module with full APIs.
- [**Architecture**](https://verum-lang.org/docs/architecture/overview) —
  compiler pipeline, θ+ execution environment, CBGR internals,
  codegen, runtime tiers and profiles.
- [**Verification**](https://verum-lang.org/docs/verification/gradual-verification) —
  how the SMT and tactic subsystems fit together.
- [**Reference**](https://verum-lang.org/docs/reference/grammar-ebnf) —
  the authoritative grammar.
- [**Blog**](https://verum-lang.org/blog) — design essays.

## Project layout

| Path         | Contents                                                |
|--------------|---------------------------------------------------------|
| `crates/`    | Compiler — 25 crates across parsing, type system, VBC, codegen, tools |
| `core/`      | Standard library, written in Verum (`.vr` files)         |
| `vcs/`       | Verum Compliance Suite — L0–L4 test levels               |
| `grammar/`   | `verum.ebnf` — the authoritative grammar specification    |
| `internal/`  | Internal specs and the documentation site source         |
| `examples/`  | Worked programs                                           |

The [crate map](https://verum-lang.org/docs/architecture/crate-map)
has the per-crate breakdown with purpose, line counts, and entry
points.

## Contributing

Design discussion happens on the blog and in
[Discussions](https://github.com/verum-lang/verum/discussions); bug
reports and patches on the issue tracker. Every change to the
language itself must either have a production in `grammar/verum.ebnf`
or extend it; changes to the standard library must ship with VCS
tests at the appropriate level.

## License

See the [LICENSE](LICENSE) file.
