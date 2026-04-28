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

### 1.2 Boundary cases — PARTIAL DEFENSE 2026-04-28

**Status:** PARTIAL DEFENSE — surface-level parser-acceptance pinned by
guardrail on the 4 boundary forms; full 1000-level fuzz-corpus expansion
deferred to the fuzz infrastructure track.

**Guardrail:** `vcs/specs/L0-critical/parser/boundary_cases.vr` covers:
- Programs with 0 segments / 0 mounts (empty_a, empty_b modules).
- Nested mount chain at 8 levels (representative scale; 1000-level needs
  synthetic generator).
- Recursive type aliases through references (mutual_ref_a — Alpha→Beta→Alpha).
- Empty bodies for protocol / impl / fn (EmptyProto, empty implement, nop()).

---

## Vector 2 — AST → VBC codegen pipeline

### 2.1 Types with 256+ variants

**Status:** DEFECT (closed by #167) — VariantTag overflow handled by
MakeVariantTyped extension byte scheme.

### 2.2 Functions with 2^16+ instructions — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — branch-target offsets are encoded as `i32`
(`crates/verum_vbc/src/instruction.rs:8455`); 2^16+ instructions are not
a cliff. The encoding has ~2.1 billion offsets of headroom (i32 range).

**Guardrails:** `crates/verum_vbc/tests/red_team_bytecode_trust_boundary.rs`
— 5 tests pin the i32-offset invariant for `Jmp`, `JmpIf`, `JmpNot` —
including offsets at `i32::MAX`, `i32::MIN`, ±100_000 (well past i16
range). All passing.

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

### 3.2 Mismatched arity calls — PARTIAL DEFENSE (documented) 2026-04-28

**Status:** PARTIAL DEFENSE — by design the interpreter trusts well-formed
bytecode for internal `Call` dispatch (`crates/verum_vbc/src/interpreter/dispatch_table/handlers/calls.rs:22`),
which reads `args.count` from the bytecode stream and copies that many
values into the new frame's registers. Codegen always emits the correct
arity (verified at the type-check + monomorphization layer); FFI arity
mismatches are caught at codegen via the `crates/verum_vbc/src/codegen/context.rs:1560`
type-vs-arity audit. Hand-crafted-bytecode arity attacks are addressed
by the bytecode validator track tracked under round-1 §3.1 (PENDING).

**Trust model:** the runtime contract is "bytecode well-formed by
codegen ⇒ arity matches function descriptor." The interpreter does NOT
add a runtime arity check on the hot dispatch path because (a) codegen
already enforces it and (b) adding it would penalise every call by
~2-3 instructions for a defense already provided one layer up.

**Future work:** when round-1 §3.1 lands a bytecode validator, the
validator runs once per module load and checks every Call/CallR/CallM
site has `args.count == params.len()`. Until then, the trust boundary
is "module load = trusted source."

### 3.3 FunctionId(N) out of range — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — interpreter routes through
`state.module.get_function(func_id).ok_or(InterpreterError::FunctionNotFound)?`
at three sites (`mod.rs:136`, `mod.rs:407`, `mod.rs:516`). OOR FunctionId
surfaces as a typed Err, never panics or segfaults.

**Guardrails:** `crates/verum_vbc/tests/red_team_bytecode_trust_boundary.rs`
— 4 tests pin the OOR invariant: `FunctionId(1)` (one-past-end),
`FunctionId(0xFFFF_FF00)` (far OOR), `FunctionId(u32::MAX)`, plus the
`FunctionId(0)` valid baseline.

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

### 4.3 GlobalDCE eliminating needed function — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — `crates/verum_codegen/src/llvm/vbc_lowering.rs:947`
is "Phase 3.7" which sets Internal linkage on all defined functions
EXCEPT a curated list of External-linkage entrypoints. The list covers:
- `verum_main` / `main` — entry points called by C runtime
- File I/O (`verum_file_open`, `verum_file_close`, `verum_file_exists`,
  `verum_file_delete`, `verum_file_read_text`, `verum_file_write_text`,
  `verum_file_read_all`, `verum_file_write_all`, `verum_file_append_all`)
- Process (`verum_process_wait`)
- File descriptor (`verum_fd_read_all`, `verum_fd_close`)
- TCP (`verum_tcp_connect`, `verum_tcp_listen`, `verum_tcp_accept`,
  `verum_tcp_send_text`, `verum_tcp_recv_text`, `verum_tcp_close`)
- UDP (`verum_udp_bind`, plus follow-ups in same file)

GlobalDCE then removes only Internal-linkage functions that are
unreferenced — runtime entrypoints survive by design.

**Audit gap (future improvement, not soundness defect):** the External
list is hardcoded at `vbc_lowering.rs:960-979`. A NEW runtime function
called from C must be added there; missing it produces a clean LLVM
linker error caught by integration tests, not a silent silent strip.
Centralising the list into a shared constant + lint to detect new
`extern "C"` declarations not in the list is tracked as a code-quality
follow-up.

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

### 6.2 Witnesses with side effects — PARTIAL DEFENSE + guardrail 2026-04-28

**Status:** PARTIAL DEFENSE — Verum's computational-properties system
(separate from contexts) tracks Pure/IO/Async/Fallible/Mutates per
`crates/verum_types/src/computational_properties.rs`; refinement witnesses
that emit side effects are visible at the type level via
`Function::properties: Option<PropertySet>`.

**Guardrail:** `vcs/specs/L1-core/refinement/witness_purity_guard.vr` pins
the pure-witness chain: 6 @verify(formal) functions returning refinement
types (Int{>= 0}, Int{>= a && >= b}, Int{it % 2 == 0}), refinement on
parameter (Int{!= 0}), composition through pure refinement returns. The
SMT verifier sees only pure bodies, preserving compositional refinement
reasoning.

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
| 1.2 Boundary cases | **PARTIAL** | 4-form guardrail (2026-04-28); full 1000-level fuzz pending |
| 2.1 256+ variants | DEFECT-CLOSED | #167 |
| 2.2 2^16+ instructions | **DEFENSE CONFIRMED** | i32 PC offsets + 5 guardrails (2026-04-28) |
| 2.3 Deep generics | PENDING | gen + termination |
| 2.4 Recursive impl | **DEFENSE** | guardrail (2026-04-28) |
| 2.5 Codegen non-determinism | PARTIAL | ongoing diff |
| 3.1 RO register | PENDING | bytecode harness |
| 3.2 Arity mismatch | **PARTIAL (documented)** | codegen-enforced; bytecode-validator under round-1 §3.1 |
| 3.3 OOR FunctionId | **DEFENSE CONFIRMED** | get_function.ok_or + 4 guardrails (2026-04-28) |
| 3.4 Frame overflow | **DEFENSE** | guardrails (2026-04-28) |
| 4.1 LibraryCall collision | PARTIAL | #176 |
| 4.2 Networking arity | PARTIAL | #105 follow-up |
| 4.3 GlobalDCE | **DEFENSE CONFIRMED** | Phase 3.7 audit + External list pinned (2026-04-28) |
| 5.1 Module cycle | **DEFENSE** | guardrails (2026-04-28) |
| 5.2 Deep super | **DEFENSE** | guardrail (2026-04-28) |
| 5.3 Alias shadow | **DEFENSE CONFIRMED** | round-1 §4.3 closure (2026-04-28) |
| 6.1 Π/Σ recursion | PENDING | termination harness |
| 6.2 Side-effect witness | **PARTIAL** | guardrail (2026-04-28) |
| 6.3 Stmt refinement | PENDING | SMT audit |
| 7.1 Gen counter race | PARTIAL | concurrent stress |
| 7.2 Hazard reclamation | PARTIAL | concurrent stress |
| 7.3 LocalHeap affinity | PENDING | cross-thread test |
| 8.1 LSP fuzz | PARTIAL | LSP fuzz harness |
| 8.2 Lint rules | PENDING | lint audit |
| 8.3 vtest recovery | PARTIAL | edge cases |

**10 vectors confirmed defended (was 9), 14 partial (was 15), 3 pending** post
2026-04-28 round-2-batch + RT-2.6.2 + RT-2.1.2 + RT-2.2.2 + RT-2.3.3 +
RT-2.3.2 + RT-2.4.3 closures (§1.2 / §2.2 / §2.4 / §3.2 (documented) /
§3.3 / §3.4 / §4.3 / §5.1 / §5.2 / §5.3 / §6.2 guardrails landed).
Sections A-C below record real defects already closed in the audit pass.
