<div align="center">

<img src="assets/brand/exports/verum-emblem-256.png" alt="Verum" width="128" height="128" />

# Verum

**A language platform for verifiable systems and research.**

[Documentation](https://verum-lang.org/docs/intro) ·
[Get started](https://verum-lang.org/docs/getting-started/installation) ·
[Studio](https://verum-lang.org/docs/tooling/studio) ·
[Blog](https://verum-lang.org/blog)

</div>

Verum brings programming, machine verification and mathematical proofs into
one language platform. It is built for people and AI agents developing complex
systems, with explicit contracts, inspectable behavior and human control.

Its guiding principle is **semantic honesty**: language constructs should mean
what they say, and a claim of correctness should be supported by evidence.

## The platform

- **Language.** Static types, protocols, dependent and refinement types,
  explicit runtime contexts and staged metaprogramming.
- **Execution.** A shared VBC representation for interpretation and LLVM-based
  native compilation, with generational references and graded memory safety.
- **Verification.** Contracts, SMT-assisted checks and checked proof terms,
  with independent Lean replay for supported proofs.
- **Studio.** A web workbench for editing, project navigation, architectural
  exploration, diagnostics and agent-assisted development, alongside LSP and
  command-line tools.
- **Ecosystem.** A standard library written in Verum and a cog package system;
  the Registry is under development.

## Direction and status

The long-term goal is a self-hosted platform spanning embedded devices,
reliable distributed systems and cognitive architectures. Code, project
architecture, specifications, proofs and agent workflows should form one
coherent development environment, accessible to engineers and researchers
without hiding its assumptions or removing their control.

**Verum is in pre-release development.** Compiler, standard-library and tooling
integration are still being completed. This repository is not a stable
production release. The compiler is currently written in Rust; self-hosting
is a future milestone.

## Explore

Start with the [language tour](https://verum-lang.org/docs/getting-started/tour)
and [installation guide](https://verum-lang.org/docs/getting-started/installation).
The [verification guide](https://verum-lang.org/docs/verification/gradual-verification)
explains the proof and checking workflows.

For the implementation, see [the grammar](grammar/verum.ebnf),
[compiler and tools](crates/), [standard library](core/) and
[conformance suite](vcs/). Contributions and bug reports are welcome through
[GitHub](https://github.com/verum-lang/verum/issues).

## License

See [LICENSE](LICENSE).
