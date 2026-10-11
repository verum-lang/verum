# Prepared publication projection isolation

The unchanged registry `prepare_copy` component fails its original isolation
oracle on consumer `6a0bf542b9a2a3158d78b4015cf1b533eecfd13e`. All five
mutation paths complete: name, authors, archive bytes, feature members and
dependency feature members. Each mutation's own assertions pass, but every
subsequent retained-input check returns false. The final assertion fails
at pc 181 after 396.2 seconds, exit 1, within the original 600-second deadline.

This is a meaningful domain isolation failure. A mutable projection must
not change the publication retained with authenticated attribution. No
successful publication or admission guarantee follows from the earlier
preparation result. The candidate remains unintegrated.

The [manifest](manifest.json) retains the exact runner, all eight source
inputs, all seven assembled project inputs, raw logs and receipts. All
inputs are byte-identical to the earlier 470.86-second run, which stopped
at `variant field receiver payload exceeds allocation` before producing
stdout. That guard failure is absent in this replay; this observation does
not assign its cause to one patch or close a general allocation audit.

The consumer uses the five explicitly retained Stage23 artifacts. Its
[build and ordinary Map acceptance](../map-mutable-slots/consumer-replay/README.md)
are recorded separately. This is not a fresh automatic bake or a main
distribution build.

## Copy semantics and domain isolation

The [language contract](../../value-copy-contract.md) defines a one-level
copy: an outer record gets fresh slots, while nested values remain bitwise
shared. References remain references; Shared preserves cell identity.
Consequently, copying only the outer DecodedPublication cannot establish
this fixture's stronger snapshot requirement, even after missing explicit
dereference-place copy emission is repaired (T1753).

T1722 must provide an explicit domain snapshot boundary or an equivalently
closed API that cannot expose retained mutable DTO storage. Its acceptance
must preserve the original mutation and retained-state requirements.
The original failed candidate and fixture stay unchanged as evidence.
Private-field, affine, admission, durable storage, HTTP and native AOT
acceptance remain separate requirements.
