# Proof-term certificate library

Canonical proof-term certificates that any kernel implementation
claiming to be a Verum-compatible verifier must accept.

## Purpose

These `.vproof` files are the **trust-base regression suite** for
the minimal proof-term checker (`verum_kernel::proof_checker`,
landed in #157).  Each certificate is hand-constructed from a
well-known mathematical proof; running `verum check-proof <file>`
must exit 0 on every file in this directory.

## Why this matters

A reference-standard formally verified kernel needs concrete
artefacts that demonstrate the trust base in action.  Without these,
"Verum has a small kernel" is a claim; with them, it's a runnable
demonstration:

```bash
$ for f in core/verify/proof_term_examples/*.vproof; do
    verum check-proof "$f"
  done
```

Every file should verify.  The trust base is `proof_checker.rs`
(633 LOC) plus the Rust compiler.

## Files

| File                              | Term shape                                  | Type                                          | Complexity              |
|-----------------------------------|---------------------------------------------|-----------------------------------------------|-------------------------|
| `identity_at_universe_0.vproof`   | `λ(x:Univ(0)).x`                            | `Π(_:Univ(0)).Univ(0)`                        | trivial                 |
| `polymorphic_identity.vproof`     | `λ(A:Univ(0)).λ(x:A).x`                     | `Π(A:Univ(0)).Π(_:A).A`                       | dependent types         |
| `k_combinator.vproof`             | `λ(A:Univ(0)).λ(B:Univ(0)).λ(x:A).λ(_:B).x` | `Π(A:Univ(0)).Π(B:Univ(0)).Π(_:A).Π(_:B).A`   | de Bruijn stress test   |

## How certificates are produced

For v0 (this directory) the certificates are hand-written.  Future
work (#153 — real proof-term emission) auto-generates them from
Verum theorem proof bodies via the Verum-AST → CIC translation.

## Future kernel implementations

When Verum lands a self-hosted kernel (#154 — bootstrap-verified
kernel), the differential-test gate (#159) will require BOTH the
Rust kernel and the self-hosted kernel to accept every file in this
directory.  Disagreement is a CI failure.

## Contributing a certificate

1. Construct the term + claimed type using the AST in
   `verum_kernel::proof_checker::Term`.
2. Serialise to JSON via `serde_json::to_string_pretty`.
3. Add file under this directory with descriptive metadata.
4. Run `verum check-proof <new-file.vproof>` — must exit 0.
5. PR the file with a one-line "what mathematical proof this is".

The certificate library grows with the kernel's scope; as
`proof_checker` admits more inference rules (refinement subtyping,
W-types, inductive types, etc.), each addition warrants a new
canonical certificate.
