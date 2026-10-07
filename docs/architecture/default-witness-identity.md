# Default witness declaration identity

T1629 removes process-order dependence from the builtin `Default.default`
declaration. The validated implementation is `7ae4d39e0bc6d70c6a5835b8746e1d1fbf088c71`,
on integration base `3598ea5a8`. Its only production change is the result type in
the [builtin `Default` registration](../../crates/verum_types/src/protocol.rs): a named declaration
`Self` replaces the live inference variable with fixed ID zero.

## Cause and intended result

The builtin result previously used `Type::Var(TypeVar::with_id(0))` as a stand-in
for `Self`. Receiver substitution handles named `Self`; it cannot safely treat an
arbitrary live inference variable as that declaration placeholder. Meanwhile,
the process-wide inference variable allocator starts at zero.

In a fresh process, `let x = T.default()` could therefore produce variable zero
while the enclosing generic parameter happened to have that same identity. The
ambiguity check accidentally accepted it. After earlier checks allocated variables,
the same expression still returned variable zero but the generic parameter had
another ID, producing an ambiguity diagnostic. The same escaped result variable
also contaminated the original cross-method controls.

The declaration now uses the normal named `Self` representation. Existing receiver
substitution binds the result to the receiver's actual type. This preserves a
single meaning for `Self` and removes the dependence on allocation order.

## Causal and regression evidence

[`default_witness_identity.rs`](../../crates/verum_types/tests/default_witness_identity.rs)
starts a fresh test process for every case and verifies that its variable allocator
starts at zero. Each case runs after zero, one, or 32 prior allocations. The six
cases cover inferred and annotated bindings, a returned generic result, the
original cross-method leak, an unconstrained protocol call that must remain
ambiguous, and an unknown member that must remain an error. The ambiguity control
declares its protocol in source so the isolated harness actually knows that name.
All direct check errors and error-severity diagnostics are collected.

On unchanged production code, five of the 18 cases failed: inferred bindings after
one or 32 allocations, and the cross-method case at all three allocation counts.
The corrected declaration passes all 18. The seven existing method-order,
protocol, match, dereference, annotation, and generic-payload controls now assert
the intended clean result without filtering other errors. Their control protocols
are declared locally; the match-only fixture uses its declared constructors.
The inverse oracles were changed only after the separate causal test reproduced
and resolved the underlying failures.

The focused two-target gate passed all eight outer tests. The seven-test target
also passed in a fresh serial run, a fresh four-thread run, and seven individually
selected fresh processes. Direct binary runs used `RUST_MIN_STACK=16777216`, as
required by the repository test configuration; a stack abort is not a diagnostic
result.

The full checker gate on the immutable implementation commit passed **3,910 tests**,
with **zero failures and three existing ignored tests**, across 170 test summaries.
Internal-reference and specification-citation checks also passed. Machine-local
log paths and SHA-256 identities are recorded in
[`default-witness-identity.json`](default-witness-identity.json).

## Reproduction and scope

Use a private target directory and the repository's default features:

```sh
cargo test --locked -p verum_types --test default_witness_identity --test default_witness_leak_tests -- --nocapture
cargo test --locked -p verum_types --test default_witness_leak_tests -- --test-threads=1
cargo test --locked -p verum_types --test default_witness_leak_tests -- --test-threads=4
cargo test --locked -p verum_types --tests --no-fail-fast
make check-internal-refs
```

Set `CARGO_TARGET_DIR` to the private target. The process-isolation test itself
sets the child stack size and exercises each case independently. Individual
legacy controls can additionally be selected by their exact test name with
`-- --exact <name>`.

This is checker acceptance. Core `Maybe` and other workaround sites remain
unchanged; ordinary CLI behavior, interpreter/AOT behavior, and removal of those
workarounds require their own acceptance evidence.
