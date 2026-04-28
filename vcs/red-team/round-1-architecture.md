# Red-Team Round 1 — Architectural Attacks

Tracks #172. Adversarial defect discovery at the architecture layer. Round 2
(#173) covers implementation; round 3 (#174) covers performance.

## Status legend

- **DEFENSE CONFIRMED** — invariant verified through audit; guardrail test pinning
  the invariant exists or is documented below.
- **WEAKNESS** — partial defense; specific gap recorded with hardening proposal.
- **DEFECT** — exploit found; a follow-up task captures the fix scope.
- **PENDING** — attack vector defined but not yet exercised; needs the listed
  infrastructure to run.

---

## Vector 1 — Refinement-type soundness under concurrency

### 1.1 Refined `Int{x > 0}` race

**Status:** PENDING — needs concurrent-write test harness.

Scenario: thread A holds `x: Int{x > 0}`, thread B writes via shared `&mut`. Can
B observe `x = 0` between A's reads?

Verum's three-tier reference model (CBGR) blocks cross-thread `&mut T` aliasing
at the type level — see `core/mem/{thin_ref,fat_ref,capability}.vr`. The
remaining angle is `&unsafe T` escapes (vector 2.1).

### 1.2 `using [Mutex]` capability serialisation

**Status:** PENDING — needs context-system fuzz harness.

Need a test that asserts: a function with `using [Mutex]` cannot read a refined
field while another thread holds the lock. Add to `vcs/specs/L2-standard/contexts/`.

### 1.3 Dependent types across `await` suspension points

**Status:** PENDING — needs async refinement-preservation harness.

Verum's async model serialises the suspension-resumption roundtrip; need to
verify Π/Σ/refinement types survive. Existing async tests at
`vcs/specs/L2-standard/async/` cover correctness but not refinement preservation.

---

## Vector 2 — CBGR escape attempts

### 2.1 `&unsafe T` observed via `&checked T` aliasing

**Status:** PENDING — needs aliasing analysis.

The three-tier reference model promises `&unsafe T` cannot be observed by
`&checked T` callers. Verify by attempting deliberate aliasing through a
container that holds both refs to the same memory.

### 2.2 Generation-counter rollover

**Status:** PARTIAL DEFENSE — atomic increments at `core/mem/header.vr`
verified in audit. Behaviour at u32 wrap point not yet exercised by guardrail
test. Add: `vcs/specs/L0-critical/memory-safety/generation_rollover.vr`.

### 2.3 Epoch advance with held `ThinRef<T>`

**Status:** DEFENSE CONFIRMED — `core/mem/thin_ref.vr` checks generation +
epoch-cap at every dereference; mismatch returns the existing safety error.
Audit pass 2026-04-28 confirmed no early-exit branches in the check.

---

## Vector 3 — VBC bytecode trust boundary

### 3.1 Hand-crafted bytecode violating type-table invariants

**Status:** PENDING — needs hand-crafted-bytecode injection harness.

The AOT path assumes the compiler-emitted type table is consistent with the
instruction stream. Direct bytecode authoring (no compiler) could violate this.
A bytecode validator pass before AOT lowering would close this; tracked.

### 3.2 MakeVariant tag overflow

**Status:** DEFECT (closed elsewhere) — covered by `#146 Phase 3
(MakeVariantTyped)` which adds explicit type-tagged MakeVariant. Cross-references
task **#167**.

### 3.3 14-class lenient SKIP triggered to drop security-critical body

**Status:** PARTIAL DEFENSE — every confirmed lenient-skip site is being driven
to zero under task **#176**. Each waiver-pinned skip needs explicit security
review.

---

## Vector 4 — Module/import system race

### 4.1 Two stdlib types with same simple name in mutually-exclusive @cfg

**Status:** DEFENSE CONFIRMED — `register_type_constructors` `prefer_existing_functions`
flag with save/restore guard around impl-block. Cfg-overlap walker is conservative
but explicit on the `unix` predicate.

### 4.2 4-level deep `super.super.super.super.X` mount

**Status:** DEFENSE CONFIRMED — currently returns input unchanged at the lexer
level. No confused-deputy callers found through audit.

### 4.3 Mount alias shadowing built-in identifier

**Status:** PENDING — needs test of `mount X.None as Some` form.

---

## Vector 5 — Verification gap: SMT timeout

### 5.1 Z3 timeout default-fail-open?

**Status:** PENDING — needs Z3 timeout-policy review.

Critical question: when Z3 times out on a refinement check, does Verum
default-fail-closed (reject the program — sound) or default-fail-open (accept —
unsound)? Locate Z3 invocation in `verum_smt/src/z3_backend.rs`; trace timeout
return path.

### 5.2 Always-timeout predicate

**Status:** PENDING — depends on 5.1 outcome.

---

## Vector 6 — Capability leakage through generic monomorphization

### 6.1 Generic with `using [Logger]` monomorphized to context lacking Logger

**Status:** PENDING — needs context-monomorphization audit.

Verum's context system (`verum_context`) is documented as runtime DI with
~5-30 ns overhead. The static guarantee is that the type system rejects
calls into a monomorphized context that doesn't satisfy `using [...]`. Verify
through dedicated test.

### 6.2 Erased-vs-reified type consistency

**Status:** PARTIAL DEFENSE — current implementation enforces consistent
treatment per the type system; no known escape but worth a dedicated
exhaustive case test.

---

## Vector 7 — Interpreter-vs-AOT semantic divergence

### 7.1 Tier-0 says ok, Tier-1 says panic

**Status:** PENDING — covered by task **#196** (differential test infrastructure).

The differential harness at `vcs/differential/` exists but coverage is uneven.
#196 drives this to comprehensive coverage.

### 7.2 Hash-table iteration determinism

**Status:** PARTIAL DEFENSE — #143 fix applied to selected hash-keyed
tables; comprehensive coverage of *all* tables not yet verified. Audit recipe:
`grep -rn "fn iter\|fn keys\|fn values" core/collections/` and confirm each
returns a deterministic order.

---

## Defenses verified through #203 stdlib audit

The following architectural-class invariants are upheld in the stdlib code,
confirmed through the closed Sprint 2 audit work:

### A. Three-layer overflow defence on `acc * radix + digit` parsers

Recipe (codified through #70-#81 sweep + #203 follow-ups):
1. Pre-loop length cap (10 digits UInt32, 19 digits Int, 20 digits UInt64).
2. Per-step `acc > MAX/radix` guard.
3. Post-add wrap-around `acc < 0` (or `next < acc` for unsigned).

Audit-verified callsites: 30+ parsers across core/encoding, core/time,
core/security, core/database, core/net, core/base. Hostile-input wrap-around
attacks foreclosed.

**Pinned soundness fixes (real defects closed during audit):**
- `core/base/semver.vr` parse_numeric + parse_u64_unchecked
- `core/database/sqlite/native/l2_record/type_coercion.vr` parse_int64
- `core/database/sqlite/native/optimizer/const_fold.vr` Add/Sub/Mul/Shift
- `core/net/http_range.vr` parse_u64 (RFC 7233 byte-offset values)
- `core/net/http_cache.vr` parse_u64_opt (Cache-Control directive integers)

### B. Constant-time crypto comparison

`core/security/util/constant_time.vr::constant_time_eq` and `_compare` are
verified branchless (no early-exit on mismatch); `@verify(constant_time)`
annotation requests codegen-side suppression of optimizations that would
re-introduce data-dependent branches.

### C. Decoder canonicality

Multiple-input-decodes-to-same-value defects foreclosed through:
- base64 RFC 4648 §3.5 canonical-trailing-bits rejection
- base32 canonical-pad-count check (§6)
- protobuf varint 10th-byte data-bits validation
- HPKE / TLS / QUIC AEAD tag-mismatch enforcement (constant-time)

Round-trip equality + signature soundness preserved.

### D. Dishonest-comment-class detection

Recipe codified: any function whose comment claims "X behaves like Y" must be
verified against documented Y behaviour. Specifically caught:
- `parse_int64` doc said "SQLite wraps; we mirror that" but `sqlite3Atoi64`
  actually returns TOO_BIG and the caller promotes to REAL. Comment was a
  pretence to avoid overflow audit.

Add to lint catalog: every `mirrors X` / `same as X` / `we mirror that`
comment is a candidate for verification against external authority.

### E. Int.MIN unary-negation safety

`(-n) as UInt64` audit recipe across stdlib; bit-flip pattern
`(((!n) as UInt64) + 1_u64)` from `sqlite_version_fmt::int_to_text` applied
at every call-site that may receive `Int.MIN`. No remaining sites observed
in `core/text/format.vr`, `core/security/otp.vr`, etc.

---

## Round 1 progress summary

| Vector | Status | Follow-up |
| --- | --- | --- |
| 1.1 Refined-int race | PENDING | concurrent harness |
| 1.2 Mutex capability | PENDING | context fuzz harness |
| 1.3 Refinement across await | PENDING | async harness |
| 2.1 unsafe→checked aliasing | PENDING | aliasing analysis |
| 2.2 Generation rollover | PARTIAL | guardrail test |
| 2.3 Epoch advance | DEFENSE | — |
| 3.1 Bytecode type-table | PENDING | bytecode validator |
| 3.2 MakeVariant overflow | DEFECT | #167 |
| 3.3 Lenient SKIP | PARTIAL | #176 |
| 4.1 Same-name @cfg types | DEFENSE | — |
| 4.2 Deep super | DEFENSE | — |
| 4.3 Mount alias shadow | PENDING | dedicated test |
| 5.1 Z3 timeout policy | PENDING | Z3 review |
| 5.2 Always-timeout | PENDING | depends on 5.1 |
| 6.1 Capability monomorph | PENDING | monomorph audit |
| 6.2 Erased/reified | PARTIAL | exhaustive cases |
| 7.1 Tier-0 vs Tier-1 | PENDING | #196 |
| 7.2 Hash determinism | PARTIAL | full audit |

**5 vectors confirmed defended (full or partial), 13 pending.** Round 1 success
condition: every PENDING entry has either a guardrail test or a tracked
weakness with concrete fix scope. Current pending count needs the listed
infrastructure (concurrent-write harness, Z3-timeout review, bytecode
validator) to advance.

The audit-class defenses listed above (Sections A-E) document invariants
already upheld and serve as the substrate Round 1 targets must not break.
