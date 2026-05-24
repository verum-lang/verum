# `concurrency/session` audit

Module: `core/concurrency/session.vr` (~255 LOC) — session-typed
channel protocols as data + duality + deadlock-freedom.

Tests: 27 unit tests covering Protocol 5-variant (Send / Recv /
Offer / Select / End) + smart ctors + dual involution (5
variants + 4 dual(dual(p)) == p cases) + protocols_equal
structural equality + compatible deadlock-freedom predicate +
Eq protocol via ==.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `verum_types::session_types` | Rust-side analysis core consumes Protocol shape. |
| `core.verify.kernel_*` | session-types encoding for soundness proofs. |
| Application channel code | static deadlock-freedom checking. |

## 2. Crate-side hardcodes

* The 5-variant Protocol set MUST agree with verum_types::session_
  types::Protocol. Drift breaks Tier-0 codegen panics.
* Display rendering uses Unicode operators (⊕ for external choice,
  & for internal choice) — must match the formal session-types
  literature.

## 3. Language-implementation gaps

### §3.1 Property tests

* ∀p. protocols_equal(p, p) — reflexivity
* ∀p,q. protocols_equal(p, q) ⟺ protocols_equal(q, p) — symmetry
* ∀p. protocols_equal(dual(dual(p)), p) — involution (pinned for
  4 variants in branch tests; generalize via property)
* ∀p,q. compatible(p, q) ⟺ protocols_equal(dual(p), q)
* compatible is symmetric: compatible(p, q) ⟺ compatible(q, p)

**Effort:** ~1h.

### §3.2 Display + Debug coverage

Display/Debug protocols are implemented but not tested. Verify
the canonical session-type notation:
* `Send { "Int", End }` → `!Int.end`
* `Recv { "Int", End }` → `?Int.end`
* `Offer { End, End }` → `(end ⊕ end)`
* `Select { End, End }` → `(end & end)`
* `End` → `end`

### §3.3 Integration test for channel handle

The phantom-protocol type parameter on channel handles is the
real test of session types. Requires runtime channel impl —
gated on async/channel test scaffold.

## Action items landed in this branch

* `core-tests/concurrency/session/unit_test.vr` — 27 unit tests
  over Protocol + smart ctors + dual + compatible +
  protocols_equal + Eq.
* `core-tests/concurrency/session/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Add property_test.vr (refl/sym/involution/compat-as-dual) | this folder | 1h |
| Display/Debug exact-string sweep | this folder | 30 min |
| Channel-handle integration test | this folder | gated on async/channel |
| Sister tests for `core.concurrency.{mod,process}` | sister folders | covered |
