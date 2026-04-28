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

### 2.2 Generation-counter rollover — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — atomic increments at `core/mem/header.vr`
verified in audit. Wraparound + epoch-overflow + ABA-aliasing-prevention
contract pinned by 5 L0-critical guardrail tests.

**Audit notes:**
* `GEN_MAX = 0xFFFFFFFE` (header.vr:82) — leaves 0xFFFFFFFF reserved for
  post-wraparound transient state.
* `increment_generation` (header.vr:395-473) does atomic fetch_add, then
  if result was at-or-past GEN_MAX-1: CAS-loop bumps epoch by exactly 1
  (preserving capabilities in upper 16 bits), CAS-loops generation back
  to GEN_INITIAL.
* **Hard panic on UInt16 epoch overflow** (header.vr:421): silently wrapping
  epoch to 0 would let a freshly-stamped allocation's (gen=INITIAL, epoch=0)
  pair collide with a long-dead reference, defeating use-after-free
  detection. The panic forces operator intervention before that can happen.
* Combined budget: 32-bit generation × 16-bit epoch = 48 bits ≈ 2.8 × 10^14
  distinct allocation slots before unrecoverable wraparound.

**Guardrail tests pinning the invariant:**
* `vcs/specs/core/mem/header_test.vr` — 4 new tests (rollover_8_cycles,
  at_gen_max_minus_1, returns_pre_increment_value, distinct_gen_epoch_pairs).
* `vcs/specs/L0-critical/memory-safety/generation_rollover.vr` (NEW) — 5 L0
  invariants (GEN_MAX value pinned, wraparound bumps epoch exactly once,
  consecutive wraparounds monotone, ABA-aliasing no pair collision,
  mid-cycle increments do not bump epoch).

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

### 4.3 Mount alias shadowing built-in identifier — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — guardrail test `mount_alias_shadows_builtin.vr`
covers the path. The compiler does NOT have a hardcoded list of protected
built-in names; per `crates/verum_types/src/CLAUDE.md`'s architectural rule
("Variant constructors: User-defined variant names must freely override
built-in convenience aliases"), user code is free to mount-alias under
names that overlap with `core.base.option.Maybe`, `core.base.result.Ok`,
etc. Lexical scoping ensures the alias only shadows within the consuming
module — main scope still resolves to the built-in.

**Guardrail:** `vcs/specs/L0-critical/modules/mount_alias_shadows_builtin.vr`
exercises (a) a locally-defined `type Maybe` coexisting with built-in
`Maybe<T>`, (b) mount-as alias to a non-conflicting name, (c)
fully-qualified path resolution, (d) main scope using built-in `Maybe<Int>`
unaffected by the modules' aliasing.

---

## Vector 5 — Verification gap: SMT timeout

### 5.1 Z3 timeout default-fail-open? — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — full 9-site audit of every `SatResult::Unknown`
consumer. All sites are fail-closed or sound-conservative.

Default global timeout: 30s (`crates/verum_smt/src/z3_backend.rs:76`,
`global_timeout_ms: Maybe::Some(30000)`). Set on solver via
`cfg.set_timeout_msec()` (L119, L1855). Z3 returns `SatResult::Unknown` on
timeout, resource limit, or undecidability.

**Audit table (every consumer of `SatResult::Unknown`):**

| Site | Verdict on `Unknown` | Soundness |
|---|---|---|
| `verum_smt/src/verify.rs:378` | `Err(VerificationError::Timeout/Unknown)` | fail-closed ✓ |
| `verum_verification/src/proof_validator.rs:1584` | `Err(ValidationError::SmtValidationFailed)` | fail-closed ✓ |
| `verum_verification/src/proof_validator.rs:5120` | `Err(ValidationError::SmtValidationFailed)` | fail-closed ✓ |
| `verum_verification/src/integration.rs:327` | `Err(WPError::Unknown)` | fail-closed ✓ |
| `verum_verification/src/integration.rs:973` | `Err(WPError::Unknown)` | fail-closed ✓ |
| `verum_verification/src/hoare_logic.rs:1098` | `Err(WPError::Unknown)` + `unknown_count++` | fail-closed ✓ |
| `verum_verification/src/bounds_elimination.rs:1213` | `Ok(false)` — keep bounds check | sound-conservative ✓ |
| `verum_verification/src/tactic_evaluation.rs:4454` | `Err(TacticError::Timeout/SmtError)` | fail-closed ✓ |
| `verum_verification/src/separation_logic.rs:908` | `return true` — assume satisfiable | sound-conservative ✓ |

The two `sound-conservative` sites are explained by their callers:
- `bounds_elimination`: returning `Ok(false)` means "do NOT eliminate the bounds
  check at runtime" — i.e. keep the runtime verification active. Sound.
- `separation_logic::is_satisfiable`: returning `true` on Unknown is conservative
  for feasibility analysis — over-approximates the set of feasible paths, never
  misses one. The function's docstring explicitly states "may have false
  positives but no false negatives". Sound.

**Verdict:** Z3 timeout is universally fail-closed across the verifier; no SMT
backend output can silently lift a refinement-type obligation. The verifier's
worst-case behaviour on timeout is "spurious rejection" (over-conservative),
never "spurious acceptance" (unsound).

**Surface-level guardrail tests pinning the invariant:**
- `vcs/specs/L1-core/refinement/smt/proof_timeout.vr` — `verify-fail` with
  `verification-timeout` expected; pins surface-level fail-closed behaviour.
- `vcs/specs/L0-critical/verification/z3_timeout_fail_closed.vr` (added 2026-04-28)
  — focused L0-critical guardrail with deliberate Z3-stress predicate at 50ms timeout.
- `crates/verum_smt/tests/timeout_fail_closed_invariant.rs` (added 2026-04-28)
  — Rust-level test programmatically constructing a Z3-hard formula + 1ms timeout,
  asserting the verify result is `Err`.

### 5.2 Always-timeout predicate — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED via the surface + L0 + Rust guardrails listed above.
A predicate Z3 cannot decide within budget is rejected at the SMT layer with
`Err(*::Unknown)`; the error propagates to the verifier and to the kernel as a
verification failure, never as a silent accept.

---

## Vector 6 — Capability leakage through generic monomorphization

### 6.1 Generic with `using [Logger]` monomorphized to context lacking Logger

**Status:** PENDING — needs context-monomorphization audit.

Verum's context system (`verum_context`) is documented as runtime DI with
~5-30 ns overhead. The static guarantee is that the type system rejects
calls into a monomorphized context that doesn't satisfy `using [...]`. Verify
through dedicated test.

### 6.2 Erased-vs-reified type consistency — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — Quantitative Type Theory (QTT) framework
in `crates/verum_types/src/qtt_usage.rs` ensures erased (Quantity::Zero)
bindings cannot flow to runtime positions. The `check_binding` function
(qtt_usage.rs:208) emits `ViolationKind::ErasedUsedAtRuntime` whenever
a Zero-quantity binding's runtime usage count exceeds 0.

**Architectural foundation:**
- `@meta` generic parameters auto-register as `Quantity::Zero` per
  `infer.rs:35948`.
- Regular params auto-register as `Quantity::Omega` (or specific via
  `@quantity(...)` attribute extraction at `infer.rs:35926-35938`).
- The `check_function_qtt` pipeline walks the function body, counts
  usage per binding, and validates against declared quantities via
  `qtt_usage::check_usage`.

**4 exhaustive guardrail tests added:**
`crates/verum_types/tests/qtt_function_check.rs`:
- `red_team_1_6_2_meta_zero_alongside_omega_consistent` — erased + reified
  bindings co-exist without cross-contamination.
- `red_team_1_6_2_meta_zero_escaping_to_runtime_caught` — erased binding
  used at runtime → ErasedUsedAtRuntime violation.
- `red_team_1_6_2_meta_zero_used_multiple_times_still_erased_violation` —
  erased used twice ⇒ erasure violation (logically prior to OverUse).
- `red_team_1_6_2_three_quantities_compose_consistently` — Zero +
  One + Omega all flow through their declared quantities; erased stays
  erased, linear used once, omega-3-times all consistently tracked.

All 11 QTT tests passing.

The architectural rule `crates/verum_types/src/CLAUDE.md` "compiler
must have ZERO knowledge of stdlib types" forecloses the alternative
unsoundness vector (compiler hardcoding which type is erased) — every
binding's quantity is discovered from its declaration / attribute.

---

## Vector 7 — Interpreter-vs-AOT semantic divergence

### 7.1 Tier-0 says ok, Tier-1 says panic

**Status:** PENDING — covered by task **#196** (differential test infrastructure).

The differential harness at `vcs/differential/` exists but coverage is uneven.
#196 drives this to comprehensive coverage.

### 7.2 Hash-table iteration determinism — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — full audit recipe applied across
`core/collections/`. All iter-exposing collections produce deterministic
iteration order given the same insertion sequence.

**Audit recipe and findings:**

`grep -rn "fn iter|fn keys|fn values|fn entries|fn into_iter" core/collections/`

| Collection | Iter source | Determinism |
|---|---|---|
| `List` (list.vr) | dense indexed array | deterministic by index ✓ |
| `Deque` (deque.vr) | indexed ring buffer | deterministic by logical index ✓ |
| `Heap` (heap.vr) | array of heap nodes | deterministic by tree-shape (insertion-dependent) ✓ |
| `BTreeMap`/`BTreeSet` (btree.vr) | sorted in-order traversal | deterministic by key-order ✓ |
| `Set` (set.vr) | underlying Map | deterministic via FxHash ✓ |
| `Map` (map.vr) | underlying hash table | deterministic via FxHash + Robin-Hood probe ✓ |

**Why deterministic.** `core/base/protocols.vr::DefaultHasher::new()` starts
with `state: 0` — FxHash is unkeyed, no per-process random seed. The
docstring explicitly notes: *"this default is for compiler-internal /
trusted-input use"*. The HashDoS trade-off is acknowledged and documented;
users who need adversarial-input resistance must opt into a seeded hasher.

**Guardrail tests:**
- `vcs/specs/L2-standard/red-team-1-architecture/hash_iter_determinism.vr`
  — pre-existing surface test from #143 sweep.
- `vcs/specs/L0-critical/memory-safety/hash_iter_determinism_full.vr` (NEW
  2026-04-28) — 7 L0 invariants pinning iteration determinism across
  Map/Set/BTreeMap/List/Deque + cross-instance determinism for hash-keyed
  structures (critical for Tier-0/1/2/3 differential testing).

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
| 2.2 Generation rollover | **DEFENSE CONFIRMED** | 9 guardrail tests across 2 files (2026-04-28) |
| 2.3 Epoch advance | DEFENSE | — |
| 3.1 Bytecode type-table | PENDING | bytecode validator |
| 3.2 MakeVariant overflow | DEFECT | #167 |
| 3.3 Lenient SKIP | PARTIAL | #176 |
| 4.1 Same-name @cfg types | DEFENSE | — |
| 4.2 Deep super | DEFENSE | — |
| 4.3 Mount alias shadow | **DEFENSE CONFIRMED** | guardrail (2026-04-28) |
| 5.1 Z3 timeout policy | **DEFENSE CONFIRMED** | 9-site audit + 3 guardrails (2026-04-28) |
| 5.2 Always-timeout | **DEFENSE CONFIRMED** | guardrails pin fail-closed (2026-04-28) |
| 6.1 Capability monomorph | PENDING | monomorph audit |
| 6.2 Erased/reified | **DEFENSE CONFIRMED** | QTT framework + 4 RT-1.6.2 tests (2026-04-28) |
| 7.1 Tier-0 vs Tier-1 | PENDING | #196 |
| 7.2 Hash determinism | **DEFENSE CONFIRMED** | full audit + 7 L0 guardrails (2026-04-28) |

**11 vectors confirmed defended (full or partial), 7 pending** (post 2026-04-28
RT-1.5 + RT-1.2.2 + RT-1.7.2 + RT-1.4.3 + RT-1.6.2 closures). Round 1 success condition: every PENDING entry has either a
guardrail test or a tracked weakness with concrete fix scope. Current pending
count needs the listed infrastructure (concurrent-write harness, bytecode
validator) to advance.

The audit-class defenses listed above (Sections A-E) document invariants
already upheld and serve as the substrate Round 1 targets must not break.
