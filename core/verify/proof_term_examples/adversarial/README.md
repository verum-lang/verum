# Adversarial proof-term certificate library

Hand-constructed certificates that **MUST be rejected** by every
Verum-compatible kernel implementation.  These are the load-bearing
complement to the accept-path canonical library: they pin REJECTION
behaviour at the kernel level, ensuring that broken or unsound
proof terms do not slip through.

## Purpose (#159 V3)

The accept-path library (`identity_at_universe_0.vproof`,
`k_combinator.vproof`, `polymorphic_identity.vproof`) verifies that
both kernel implementations agree on what's a VALID proof.  This
adversarial library verifies that they agree on what's INVALID:

  * **Universe mismatches** — `Universe(0)` claimed at the wrong
    level.
  * **Domain mismatches in `App`** — applying a function to a
    type-mismatched argument.
  * **Body-type mismatches in `Lam`** — λ-introduction whose body
    type doesn't match the Π's codomain.
  * **Free variables** — `Var(i)` at a context too shallow to bind it.
  * **Non-function application** — `App(Universe(0), …)`, applying
    a type to something.
  * **η-soundness** — λx. (f x) where f's free vars include the
    bound x; the η-rule MUST NOT fire here.
  * **Type-of-type self-reference** — `Universe(n) : Universe(n)`
    instead of `Universe(n) : Universe(n+1)`.
  * **Π domain not a type** — `Π(non-type) → …`.
  * **Π body not a type** — `Π(…) → non-type`.
  * **Wrong β-reduction** — applying λ to a value, but the result
    type doesn't match.

Every certificate in this directory is processed by the audit gate
`verum audit --proof-term-library` (post-V3) which:
  1. Runs Algorithm A (`proof_checker::Certificate::verify`).
  2. Runs Algorithm B (`proof_checker_nbe::verify_certificate`).
  3. Asserts BOTH reject (`both_reject` agreement).
  4. Disagreements (one accepts, one rejects) flip the audit gate to
     failure — surfacing kernel-implementation bugs in the negative
     direction immediately.

## Soundness rationale

A kernel that ACCEPTS one of these certificates is unsound — it's
admitting a malformed proof term.  A kernel that REJECTS where the
other accepts is also a bug to investigate (one of them is wrong;
in production both must agree).  Lock-step rejection across both
algorithms is the architectural invariant.

## Naming convention

Files are named `reject_<class>_<scenario>.vproof` where
`<class>` is one of: `universe_mismatch`, `domain_mismatch`,
`free_var`, `non_fn_app`, `eta_unsound`, `self_type`,
`pi_domain_not_type`, `pi_body_not_type`, `wrong_beta`.

Each certificate's `metadata.expected_outcome` is `"reject"` and
`metadata.adversarial_class` describes which kernel rule's
rejection path the certificate exercises.
