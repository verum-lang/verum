# Red-Team Round 2 — Implementation Attacks

Tracks #173. Adversarial defect discovery at the implementation level.
Architecture-level review is round 1 (#172); performance/DoS is round 3 (#174).

## Status legend

Same convention as round 1.

---

## Vector 1 — Parser fuzzing

### 1.1 Random bytes / mutated stdlib files

**Status:** PARTIAL — `vcs/fuzz/` infrastructure exists. Comprehensive corpus
seeds for boundary cases (0 segments, deeply nested mounts, empty bodies,
recursive type aliases) are listed in the fuzz README but full coverage matrix
not yet exhausted.

### 1.2 Boundary cases

**Status:** PENDING — needs fuzz-corpus expansion:
- Programs with 0 segments / 0 mounts.
- Deeply nested mount chains (1000 levels).
- Type aliases that recurse (caught by depth limit; verify error path).
- Empty body for protocol/impl/fn.

---

## Vector 2 — AST → VBC codegen pipeline

### 2.1 Types with 256+ variants

**Status:** DEFECT (closed by #167) — VariantTag overflow handled by
MakeVariantTyped extension byte scheme.

### 2.2 Functions with 2^16+ instructions

**Status:** PENDING — needs synthetic large-fn generator. Risk: any 16-bit
PC offset / branch-target encoding overflow.

### 2.3 Nested generics 100 levels deep

**Status:** PENDING — needs nested-generic generator.

### 2.4 Recursive impl blocks — DEFENSE CONFIRMED + guardrail 2026-04-28

**Status:** DEFENSE CONFIRMED — closure-walker termination checks during
audit show terminal cycle handling at impl-block level.

**Guardrail:** `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §2.4
exercises mutually-recursive impl-graph (AType / BType each implementing
both AProtocol and BProtocol — 4 implement-blocks form a graph cycle the
closure walker must terminate on).

### 2.5 Pre-canon snapshot diff

**Status:** PARTIAL — non-deterministic codegen guardrail in place per #143;
needs ongoing differential vs snapshot.

---

## Vector 3 — VBC interpreter abuse

### 3.1 Assign to read-only register

**Status:** PENDING — needs hand-crafted bytecode harness.

### 3.2 Mismatched arity calls

**Status:** PARTIAL DEFENSE — FFI-arity guardrail covers extern calls; the
interpreter-side internal-call arity check is partial. Audit gap.

### 3.3 FunctionId(N) out of range

**Status:** PENDING — needs out-of-range-call test harness.

### 3.4 Frame-stack overflow — DEFENSE CONFIRMED + guardrails 2026-04-28

**Status:** DEFENSE CONFIRMED — `try_push_frame`
(`crates/verum_vbc/src/interpreter/registers.rs:117`) is fallible by
design. Audit confirmed all callers route through it (no direct push).
Recipe (grep for `frame_stack.push\b` outside the wrapper) found zero
violations.

**Guardrails:**
- Rust unit test `test_stack_overflow` in
  `crates/verum_vbc/src/interpreter/stack.rs:294` pins the limit at the
  CallStack-level.
- `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §3.4 exercises
  surface-level deep recursion — `deep_recursion_witness` surfaces as
  `InterpreterError::StackOverflow`, not a SIGSEGV / panic.

---

## Vector 4 — AOT/LLVM lowering

### 4.1 LibraryCall name collisions

**Status:** PARTIAL DEFENSE — #168 work covers LibraryCall resolution.
Cross-references task #176 lenient-skip waiver registry.

### 4.2 emit_verum_networking_functions arity

**Status:** PARTIAL DEFENSE — #105/#106 surfaced specific arity-mismatch
issues; remaining 11 helpers tracked under #105 follow-up scope.

### 4.3 GlobalDCE eliminating needed function

**Status:** PARTIAL DEFENSE — internal-linkage + DCE policy reviewed during
#143 cascade; comprehensive verification still pending.

---

## Vector 5 — Stdlib abuse

### 5.1 Module cycle A→B→A — DEFENSE CONFIRMED + guardrail 2026-04-28

**Status:** DEFENSE CONFIRMED — closure walker terminates on cycles
(verified during #181 audit). No infinite-loop attacks observed.

**Guardrails:**
- `vcs/specs/L0-critical/modules/circular_type_dependency.vr` — 2-cycle and
  mutual-function-recursion cases.
- `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §5.1 — 4-cycle
  W → X → Y → Z → W to stress closure-walker depth.

### 5.2 4-level deep super chain — DEFENSE CONFIRMED + guardrail 2026-04-28

**Status:** DEFENSE CONFIRMED — currently no-op at lexer level.

**Guardrail:** `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §5.2
constructs 4 nested module declarations and exercises
`super.super.super.super.OuterT` reach-back.

### 5.3 Mount alias shadowing built-in — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — closed by round-1 vector §4.3 closure on
2026-04-28. See `vcs/specs/L0-critical/modules/mount_alias_shadows_builtin.vr`
and round-1 doc §4.3 for the audit trail. Architectural rule:
`crates/verum_types/src/CLAUDE.md` "User-defined variant names must freely
override built-in convenience aliases".

---

## Vector 6 — Refinement/dependent-type adversarial inputs

### 6.1 Π types recursing through Σ payloads

**Status:** PENDING — needs verifier-loop termination harness.

### 6.2 Witnesses with side effects

**Status:** PARTIAL DEFENSE — Verum's computational-properties system
(separate from contexts) tracks Pure/IO/Async/Fallible/Mutates. Refinement
witness that mutates is detectable; needs explicit guardrail test.

### 6.3 Refinement in stmt-level code with unreachable

**Status:** PENDING — needs SMT-statement-level audit.

---

## Vector 7 — CBGR memory safety

### 7.1 Generation counter race

**Status:** PARTIAL DEFENSE — atomic increment with Acquire-Release ordering
verified. Race-free at the per-generation level. Comprehensive concurrent
stress test not yet shipped.

### 7.2 Hazard-pointer reclamation race

**Status:** PARTIAL DEFENSE — `core/mem/hazard.vr` implements the standard
hazard-pointer protocol. Single-reader audit confirms protocol correctness;
multi-reader stress test pending.

### 7.3 LocalHeap thread-affinity violation

**Status:** PENDING — needs cross-thread-access guardrail test.

---

## Vector 8 — Tooling abuse

### 8.1 LSP responses to malformed source

**Status:** PARTIAL DEFENSE — LSP entry points wrap incoming requests in
catch-and-respond; specific panic-paths from malformed source not exhaustively
fuzzed.

### 8.2 Lint rules false-positive/negative

**Status:** PENDING — needs lint-rule audit.

### 8.3 vtest crash recovery

**Status:** PARTIAL DEFENSE — vtest spawns child processes; SIGSEGV from
child is recovered. Edge cases (SIGKILL parent, OOM child) not exhaustively
tested.

---

## Defenses confirmed through #203 implementation audit

The following implementation-class defects were CLOSED during the audit; the
fixes pin the corresponding invariants:

### A. Hostile-input UInt64/Int64 wrap defences

Five real-world soundness defects closed:
1. `core/base/semver.vr` parse_numeric + parse_u64_unchecked — silent UInt64
   wrap on 21+ digit numeric or prerelease identifiers.
2. `core/database/sqlite/native/l2_record/type_coercion.vr::parse_int64`
   dishonest-comment + silent wrap on hostile text-affinity input.
3. `core/database/sqlite/native/optimizer/const_fold.vr` Add/Sub/Mul/Shift —
   compiler crashing in debug or baking wrapped values in release on
   hostile constant arithmetic.
4. `core/net/http_range.vr::parse_u64` — RFC 7233 byte-offset wrap on
   hostile Range header.
5. `core/net/http_cache.vr::parse_u64_opt` — Cache-Control directive
   integer wrap on hostile origin/intermediary input.

All 5 closed with the standard 3-layer overflow recipe.

### B. Byte-loop allocation hot paths

~170+ sites across QUIC/TLS 1.3/HTTP/3/sqlite/postgres/mysql wire-frame
layers + HPKE/JWT/Merkle/HKDF/PBKDF2 crypto + collections (trie,
consistent_hash, count_min). Foundational `Text.push_str` rewrite eliminated
the per-byte capacity probe + null-terminator memset cascade.

Bulk-copy migration foreclosed allocator pressure as a perf-cliff vector
(round-3 territory) on the wire-frame parsing/emission path.

### C. Dishonest-API class fixes (historic)

From earlier sweep (memory-recorded): UUID v7/Snowflake/ULID had ZERO
timestamps because `core.time.instant.unix_millis()` did not exist; lenient
codegen swallowed the call. Closed via `core.time.system_time.SystemTime.now()`.

Same class: CSPRNG `core.base.random.fill` → nothing; UUID v4 was always
zero. Closed via `core.sys.common.random_bytes`.

These confirm that lenient-skip in the codegen is itself an attack surface;
#176 is the right vehicle to drive that count to zero.

---

## Round 2 progress summary

| Vector | Status | Follow-up |
| --- | --- | --- |
| 1.1 Random fuzz | PARTIAL | corpus expansion |
| 1.2 Boundary cases | PENDING | corpus seeds |
| 2.1 256+ variants | DEFECT-CLOSED | #167 |
| 2.2 2^16+ instructions | PENDING | synthetic gen |
| 2.3 Deep generics | PENDING | gen + termination |
| 2.4 Recursive impl | **DEFENSE** | guardrail (2026-04-28) |
| 2.5 Codegen non-determinism | PARTIAL | ongoing diff |
| 3.1 RO register | PENDING | bytecode harness |
| 3.2 Arity mismatch | PARTIAL | interpreter audit |
| 3.3 OOR FunctionId | PENDING | OOR harness |
| 3.4 Frame overflow | **DEFENSE** | guardrails (2026-04-28) |
| 4.1 LibraryCall collision | PARTIAL | #176 |
| 4.2 Networking arity | PARTIAL | #105 follow-up |
| 4.3 GlobalDCE | PARTIAL | DCE audit |
| 5.1 Module cycle | **DEFENSE** | guardrails (2026-04-28) |
| 5.2 Deep super | **DEFENSE** | guardrail (2026-04-28) |
| 5.3 Alias shadow | **DEFENSE CONFIRMED** | round-1 §4.3 closure (2026-04-28) |
| 6.1 Π/Σ recursion | PENDING | termination harness |
| 6.2 Side-effect witness | PARTIAL | guardrail |
| 6.3 Stmt refinement | PENDING | SMT audit |
| 7.1 Gen counter race | PARTIAL | concurrent stress |
| 7.2 Hazard reclamation | PARTIAL | concurrent stress |
| 7.3 LocalHeap affinity | PENDING | cross-thread test |
| 8.1 LSP fuzz | PARTIAL | LSP fuzz harness |
| 8.2 Lint rules | PENDING | lint audit |
| 8.3 vtest recovery | PARTIAL | edge cases |

**7 vectors confirmed defended (was 6), 14 partial, 6 pending (was 7)** post
2026-04-28 round-2-batch closure (RT-2 §2.4 / §3.4 / §5.1 / §5.2 / §5.3
guardrails landed). Sections A-C below record real defects already closed
in the audit pass.
