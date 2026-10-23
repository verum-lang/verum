# Native proof admission

The portable `verum_proof` crate owns the substitution and normalization-by-evaluation native checkers. `verum_kernel::proof_checker` and `proof_checker_nbe` are compatibility re-exports, not alternative implementations. Source elaboration, SMT inspection and native replay are separate authority boundaries.

## Judgment and authority

A `verum-native/1` artifact carries an independently supplied statement and a certificate. Admission requires exact decoded equality of statement and claimed type, type well-formedness, and success of both native checkers. Unknown fields and duplicate structural fields are rejected. Metadata cannot declare additional term axioms or override the judgment.

The kernel profile includes dependent functions and pairs, universes and identity elimination. It is not the entire Verum source language. Acceptance proves neither faithful elaboration of an arbitrary source program nor alignment of a natural-language theorem. An imported report is data, not authority; the host must replay the actual artifact and bind the result to its revision.

The portable crate retains the existing native Rust/wire types. It sits below the compiler's `verum_common` layer: pulling in the latter would introduce its nightly compiler feature into otherwise stable embedded consumers. It has no LLVM, solver, interpreter or compiler dependencies. The process feature adds only host supervision; it does not change rules or generated AOT programs.

## Resource boundary

The structural admission budget is not a bound on normalization cost. Untrusted proofs therefore run in a fresh, single-threaded child before async-runtime initialization. The parent uses bounded file IO, validates the report and checker identity, and kills/reaps on all incomplete paths. The worker receives no inherited environment secrets.

The process policy bounds input, output, CPU, wall time and stack, uses address-space limits where supported, and monitors RSS. macOS does not enforce the same address-space limit: sampled RSS is a termination threshold and may overshoot between samples. It must not be described as a hard instantaneous allocation bound. Deployments requiring a hard aggregate RAM boundary need a supporting OS/container service limit.

Timeouts, unavailable workers and process failures are inconclusive. Neither a resource refusal nor parser rejection is a semantic proof of the negation. Calling the in-process admission function with attacker-controlled terms bypasses the intended host budget.

## Preservation under a normalization budget

A head-reduction pause must retain every binder annotation. The old exhausted-fuel branch rebuilt a lambda with `Universe(0)` regardless of its original domain. The branch now preserves the actual domain; a generative control checks both unchanged paused terms and type preservation over universe-polymorphic identity applications. Strong normalization is not a fixed bound on normalization time.

## SMT adapter

`KernelOnlyReplayEngine` recognizes certificate structure and rule names, not inference validity. It returns `StructuralOnly`; this never satisfies `is_accepted` or multi-backend consensus. A freshly hashed `refl` step concluding `0 = 1` is the regression control. Completing a semantic SMT adapter requires typed premise/conclusion checking, not more rule-name aliases.

## Validation

- Portable native tests preserve the old checkers' mathematical controls and add strict decoding, exact statement binding and real process-boundary checks.
- The compiler kernel tests exercise compatibility after extraction.
- Noesis CLI/HTTP/MCP tests execute this same checker, change a proof without reusing the old result, export/restore history, and reject authority attached merely to prose.
- The public workflow includes the process-enabled portable suite; integration tests are not replaced by parser-only fixtures.

## Source diagnostics

The source worker retains the primary production-parser diagnostic or the innermost
known elaboration range. `diagnostic_span` is an optional half-open UTF-8 byte range
with a SHA-256 of the exact fragment; the report also binds the complete source and
selected declaration. An empty range denotes an insertion point. This is distinct
from the selected declaration span, because a parse error can occur elsewhere in
the module. Invalid producer coordinates yield an inconclusive protocol result.
Successful reports omit the field, as do historical reports without source locations.
This additive metadata does not change the logical profile or replay authority.

Consumers validate bounds, UTF-8 scalar boundaries, complete input identity and
fragment digest before navigation. A UI must map bytes to its actual text projection,
including CRLF normalization, without searching for matching words or clipping
invalid offsets. Old source locations cannot control a newly edited draft. Parser
error kinds are rendered without fabricated line/column values; clients derive
positions from the bound source. Primary diagnostics do not promise exhaustive error
recovery or a minimal semantic explanation.

Parenthesized expressions elaborate to the enclosed term. The same native statement
and certificate are required with or without grouping; an ill-typed grouped proof
still fails both-kernel admission. Grouping does not add an inference rule.
