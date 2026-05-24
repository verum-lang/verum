# `concurrency/process` audit

Module: `core/concurrency/process.vr` (~400 LOC) — π-calculus
process algebra as data + capture-avoiding substitution +
α-equivalence (parallel walk over de Bruijn levels).

Tests: 21 unit tests covering Process 6-variant + smart ctors
+ substitute (6 cases: zero/send-channel/recv-channel/
restrict-shadows/par-descends/replicate-descends) +
alpha_eq (8 cases including bound-name rename, different
binder semantics).

## 1. Defects surfaced

### §A.1 alpha_eq returns true for different free names (1 @ignore)

`test_alpha_eq_restrict_different_free_name_distinct`:

```verum
let p = (νx) free1⟨m⟩.0
let q = (νy) free2⟨m⟩.0
assert(!alpha_eq(&p, &q));   // FAILS — alpha_eq returns true
```

Per the source spec (name_eq_under_env at line 382-393):

```
(None, None) => a == b
```

Should return `false` for `"free1" == "free2"`. The test
failure suggests either (a) name_eq_under_env's env.get
lookup spuriously returns Some, OR (b) the outer Restrict
binder is incorrectly extending the env with `free1` /
`free2` themselves.

Suspected root: same task #17/#39 class on `name_eq_under_env`
dispatch resolution OR on `Map<Text, Int>.get` returning
wrong Maybe variant for absent keys.

## 2. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types::pi_calculus` | Rust-side mirror analysis core |
| `core.verify.kernel_*` | π-calculus encoding for soundness proofs |
| Application concurrency code | translate handler-style code to π-calculus |

## 3. Language-implementation gaps

### §3.1 Property test on alpha_eq

* Reflexivity: ∀p. alpha_eq(p, p)
* Symmetry: alpha_eq(p, q) ⟺ alpha_eq(q, p)
* Transitivity: alpha_eq(p, q) ∧ alpha_eq(q, r) ⟹ alpha_eq(p, r)
* Substitutive: alpha_eq(p, substitute(p, b, fresh)) when b is bound
  in p and fresh is fresh for p.

**Effort:** ~1h.

### §3.2 free_names test surface

`free_names` is `pub` but not yet covered. Test that:
* free_names(Zero) = ∅
* free_names(Send(x, m, 0)) = {x, m}
* free_names(Restrict(x, Send(x, m, 0))) = {m} (x bound out)
* free_names(Recv(x, y, Send(y, m, 0))) = {x, m} (y bound out)

**Effort:** ~30 min.

## Action items landed in this branch

* `core-tests/concurrency/process/unit_test.vr` — 21 unit tests
  (20 GREEN + 1 @ignore for surfaced alpha_eq free-name defect).
* `core-tests/concurrency/process/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Investigate alpha_eq free-name defect | `core/concurrency/process.vr` | 1-2 days |
| Add property_test.vr (refl/sym/trans on alpha_eq) | this folder | 1h |
| Add free_names tests | this folder | 30 min |
| Sister tests for `core.concurrency.{mod,session}` | sister folders | 3 days total |
