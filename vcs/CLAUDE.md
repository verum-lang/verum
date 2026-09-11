# VCS: Verum Conformance Suite

## Quick Reference

### Test Types (from basic to full execution)

| Type | Phase | Pass Condition | Use Case |
|------|-------|----------------|----------|
| `parse` | Lexer+Parser | No syntax errors | Grammar correctness |
| `parse-fail` | Lexer+Parser | Expected syntax error | Invalid syntax detection |
| `typecheck-pass` | +Type Check | No type errors | Type system correctness |
| `typecheck-fail` | +Type Check | Expected type error | Type error detection |
| `verify-pass` | +SMT Verify | Contracts verified | Refinement types |
| `verify-fail` | +SMT Verify | Expected verification error | Contract violation |
| `compile-only` | Full Compile | Compilation succeeds | Codegen correctness |
| `run` | +Execution | Expected stdout/exit code | Runtime behavior |
| `run-panic` | +Execution | Expected panic message | Panic handling |
| `run-interpreter` | VBC Interpreter | Expected stdout/exit code | Tier 0 interpreter testing |
| `run-interpreter-panic` | VBC Interpreter | Expected panic message | Tier 0 panic testing |
| `differential` | Multi-tier | Same results across tiers | Tier consistency |
| `benchmark` | Performance | Meets timing targets | Performance regression |

### Test Levels

| Level | Purpose | Tests | Stability |
|-------|---------|-------|-----------|
| **L0-critical** | Core safety guarantees | ~500 | Must never fail |
| **L1-core** | Type system, inference | ~300 | Should not fail |
| **L2-standard** | Async, contexts, modules | ~200 | May have known issues |
| **L3-extended** | FFI, GPU, dependent types | ~150 | Experimental |
| **L4-performance** | Benchmarks | ~40 | Performance targets |

### Directory Structure

```
vcs/
├── specs/
│   ├── L0-critical/
│   │   ├── lexer/           # Token tests
│   │   ├── parser/          # Grammar tests
│   │   ├── memory-safety/   # CBGR, bounds, safety
│   │   ├── reference_system/# References, lifetimes
│   │   ├── modules/         # Import/export
│   │   └── builtin-syntax/  # Meta functions
│   ├── L1-core/
│   │   ├── types/           # Type system
│   │   ├── inference/       # Type inference
│   │   └── refinement/      # Refinement types
│   ├── L2-standard/
│   │   ├── async/           # Async/await
│   │   ├── contexts/        # Using/provide
│   │   └── protocols/       # Protocol impls
│   ├── L3-extended/
│   │   ├── ffi/             # Foreign function interface
│   │   ├── proofs/          # Formal verification
│   │   └── dependent/       # Dependent types
│   └── L4-performance/
│       └── micro/           # Micro benchmarks
├── runner/
│   └── vtest/               # Test runner source
└── scripts/
    └── run-tests.sh         # CI scripts
```

### Test File Format

```verum
// @test: typecheck-pass|typecheck-fail|compile-fail|run|run-panic|...
// @level: L0|L1|L2|L3|L4
// @tier: 0|1|2|3|all (execution tier)
// @tags: memory-safety, bounds-check, ...
// @timeout: 5000 (milliseconds)
// @description: Human-readable description

// For expected errors:
// @expected-error: E400
// @expected-error-count: 3

// For expected output:
// @expected-stdout: expected output
// @expected-exit: 0

// For panics:
// @expected-panic: Index out of bounds

// For skipping:
// @skip: reason for skip
// @requires: runtime, gpu, ffi

fn main() {
    // Test code
}
```

### Running Tests

```bash
# Run all L0 tests
cargo run -p vtest -- run --level L0

# Run with compile-time only (skip runtime tests)
cargo run -p vtest -- run --level L0 --compile-time-only

# Run specific test file
cargo run -p vtest -- run specs/L0-critical/parser/basic.vr

# Run with filter
cargo run -p vtest -- run --level L0 --filter "typecheck"

# Run verbose (show details)
cargo run -p vtest -- run --level L0 --verbose

# List tests without running
cargo run -p vtest -- list --level L0
```

### Error Codes

| Code | Category | Example |
|------|----------|---------|
| E0xx | Parse errors | E001: Unexpected token |
| E1xx | Name resolution | E100: Undefined variable |
| E2xx | Module errors | E200: Import not found |
| E3xx | Memory/lifetime | E310: Use after move, E312: Lifetime error |
| E4xx | Type system | E400: Type mismatch, E401: Invalid cast, E402: Send bound, E403: Sync bound |
| E5xx | Verification | E500: Contract violation |

### Common Test Patterns

**Typecheck Pass** - Code should compile without errors:
```verum
// @test: typecheck-pass
// @level: L0
fn main() {
    let x: Int = 42;
    assert(x == 42);
}
```

**Typecheck Fail** - Expect specific error:
```verum
// @test: typecheck-fail
// @level: L0
// @expected-error: E400
fn main() {
    let x: Int = "hello";  // ERROR: Type mismatch
}
```

**`compile-fail` vs `typecheck-fail`.** `typecheck-fail` observes only what the TYPE CHECKER raises. Diagnostics from later phases — module resolution, const evaluation, codegen — never reach it, and a spec pinning one of those reports "typecheck unexpectedly succeeded", which reads as a MISSING COMPILER CHECK rather than a directive that cannot see the phase. `compile-fail` runs the full `verum build` and expects a non-zero exit, so `@expected-error: E204` (circular constant dependency) and its kin are pinnable. Use `typecheck-fail` when the diagnostic IS a type error — it is faster and runs in-process.

**Which code to pin.** `crates/verum_error/src/registry.rs` is the code table, and it is now gated: a code the compiler emits but the registry does not list fails `cargo test -p verum_error`. Read the code off the registry, not off another spec — this line said E600 for a circular constant dependency for a while, which is the context system's "context not provided", and no spec caught it because nothing emitted either one. A spec DID assert E600 for it, and passed — measured 2026-09-11, because `compile-fail` spawned an installed `verum` 17 days older than the tree, which still printed the old pair (T1429). It asserts E204 now. Note also that vtest matches the code EXACTLY (`directive.rs`, `ExpectedError::matches`). A diagnostic that carries no code is pinned by the message-alone form below, never by naming the concept in prose — prose is refused.

**The three forms of `@expected-error:`, and nothing else.** The directive is
parsed by `ExpectedError::parse` (`vcs/runner/vtest/src/directive.rs`), which
accepts exactly:

```verum
// @expected-error: E400                                  a code
// @expected-error: E400 "expected 'Int', found 'Text'"    a code and a message
// @expected-error: E400 "…" at line 8, col 10             …and a position
// @expected-error: [error] E400 "…"                       …with an explicit severity
// @expected-error: "cannot prove denominator nonzero"     a MESSAGE alone
// @expected-error: any                                    only "this must fail"
```

The message-alone form exists because a diagnostic that carries no code cannot
otherwise be pinned; `any` exists so that a spec asserting nothing specific has
to SAY so. Anything else — prose, a comma-separated list of codes, a code in no
registry — is a hard failure of the spec, reported by file and line. It is not a
warning: an `@expected-error:` the runner drops leaves the expectation set
empty, and an empty expectation set means "expect any failure", so a dropped
directive turns a specific assertion into a vacuous one and says nothing.
Measured 2026-09-11: 102 header directives were being dropped that way, and one
of them was a spec that passed on a parse error in itself while claiming to
exercise the meta sandbox.

The code is checked against the registry, not against a shape. A shape cannot
tell `E0601` (non-exhaustive patterns) from `E060` (invalid context method) —
both are real — and the pattern that preceded this matched the first four
characters of a five-character code. Measured 2026-09-11: 28 directives
asserted a different code than the one they named, and in 24 of the 28 the
truncated code is itself registered, so the spec asserted a REAL error that
was not its own. `M###` is the one exemption: the meta-system error space
lives in `crates/verum_compiler/src/meta/error.rs` and has no registry entry.

A few diagnostics carry a SYMBOLIC code rather than a numeric one, and no
directive can name those as codes. The one a spec author meets constantly is
`proof-failed`: the verify path prints `error<proof-failed>: \`theorem X\` did
not discharge`, and there is no such registry entry. Pin it with the
message-alone form — `@expected-error: "proof-failed"` — which is what the
original authors wrote before it had a form that worked. `W_NOEFFECT` and
`E_MODULE_HEADER_FORWARD_DECL_NO_SOURCE` are the same shape.

**AND MEASURE ON THE ROUTE THE RUNNER TAKES.** `verum check` and the runner's
`verify-fail` executor are different pipelines and print different things for
the same file: the CLI says `error<E0319>: theorem 'false_claim' proof failed
verification`, the runner says `error<proof-failed>: … did not discharge`. A
directive written from the first is refused by the second. Seven specs were
given an `E0319` assertion off the CLI and failed under the runner until they
were re-measured through it.

**`@expected-error-count` counts DIAGNOSTICS, not directives.** The executor
compares it against the number of errors the run produced; an
`@expected-error` is one ASSERTION, and one assertion can be satisfied by
several diagnostics. `L0-critical/parser/rust_macro_recovery.vr` is the
shape: one `@expected-error: E0E2` and `@expected-error-count: 3`, because
the compiler emits that diagnostic three times and the spec's whole subject
is that it emits three and not more. What the validator refuses is the
combination that cannot hold — more DISTINCT codes asserted than the count
allows.

**Run with Expected Output**:
```verum
// @test: run
// @level: L0
// @expected-stdout: Hello World
// @expected-exit: 0
fn main() {
    print("Hello World");
}
```

**Run Expecting Panic**:
```verum
// @test: run-panic
// @level: L0
// @expected-panic: Index out of bounds
fn main() {
    let arr = [1, 2, 3];
    let _ = arr[10];  // Panic
}
```

### Important Notes

1. **--compile-time-only**: Use during VBC migration when runtime isn't available. Converts `run`/`run-panic` to `typecheck-pass`.

2. **@requires: runtime**: Mark tests that need runtime. These skip when runtime unavailable.

3. **Nominal Typing**: Verum uses nominal typing for structs. `User` and `Admin` are different types even if structurally identical.

4. **Test Tiers**:
   - Tier 0: VBC Interpreter
   - Tier 1: JIT (LLVM)
   - Tier 2: AOT (LLVM)
   - Tier 3: Native (optimized)

5. **Execution Order**: Parse → Typecheck → Verify → Compile → Execute

### Makefile Targets

```bash
cd vcs
make test          # All tests
make test-l0       # L0 only
make test-l1       # L1 only
make bench         # Benchmarks
make fuzz          # Fuzzing
make differential  # Differential tests
```