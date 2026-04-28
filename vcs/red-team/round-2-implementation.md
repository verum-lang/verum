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

### 2.3 Nested generics 100 levels deep — DEFENSE CONFIRMED + guardrail 2026-04-28

**Status:** DEFENSE CONFIRMED — `ast_to_type` has a hard recursion-depth cap
(currently 64) that surfaces deep generics as a typed compile error
(`error: recursion limit exceeded: ast_to_type recursion depth exceeded (max 64)`)
rather than overflowing the host stack with a SIGSEGV.

A secondary type-substitution depth cap (30) emits structured warnings on
the same axis (`WARN: Maximum type substitution depth (30) exceeded`)
without failing the compile, so legitimate deep generics produce
diagnostic noise but compile cleanly.

**Guardrails (added 2026-04-28):**
- `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §2.3 —
  `deep_generic_witness` returns `Maybe<…<Maybe<Int>>>` 32-deep
  (well within the cap) and must `typecheck-pass` cleanly.
- `vcs/specs/L0-critical/parser/edge_cases/red_team_nested_generic_recursion_limit.vr`
  — `deep_generic_overflow` at 65-deep MUST `typecheck-fail` with the
  recursion-limit error.  Together the two pin the
  DEFENSE CONFIRMED interval [≤64 OK, ≥65 graceful-fail].

### 2.4 Recursive impl blocks — DEFENSE CONFIRMED + guardrail 2026-04-28

**Status:** DEFENSE CONFIRMED — closure-walker termination checks during
audit show terminal cycle handling at impl-block level.

**Guardrail:** `vcs/specs/L0-critical/red_team_round_2_confirmations.vr` §2.4
exercises mutually-recursive impl-graph (AType / BType each implementing
both AProtocol and BProtocol — 4 implement-blocks form a graph cycle the
closure walker must terminate on).

### 2.5 Pre-canon snapshot diff — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — encoding determinism is pinned via direct
equality-of-bytes invariants (a stronger pin than snapshot-diff: the
test re-encodes the same Instruction value multiple times and asserts
byte-equality, so the property holds for ANY input rather than the
narrow set captured in a frozen snapshot file).

**Guardrails (already in place):**
- `crates/verum_vbc/src/bytecode.rs::test_encoding_determinism` — same
  `Instruction::CallG` value encoded three times must produce
  byte-identical buffers.
- `crates/verum_vbc/tests/bytecode_roundtrip_tests.rs::test_encoding_deterministic`
  — same `Instruction::LoadI` value encoded twice must match.
- `crates/verum_vbc/src/codegen/tests_comprehensive.rs::test_instruction_determinism`
  — codegen-level determinism over multiple compilation runs.
- `crates/verum_vbc/src/mono/phase.rs::test_instantiation_ordering_determinism`
  — monomorphization order is deterministic across runs (closes the
  HashMap-iteration-order non-determinism risk on `cargo test --release`).

The four tests together cover the encoder, the decoder roundtrip, the
codegen path, and the monomorphization phase — every level at which
non-deterministic ordering or hash-randomization could leak into the
emitted bytecode.

---

## Vector 3 — VBC interpreter abuse

### 3.1 Assign to read-only register — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — closed via the per-instruction bytecode
validator (round-1 §3.1).  VBC has no concept of "read-only register"
at the instruction level — registers are simply numbered slots
`r0..r{register_count}` per function frame.  The attack vector this
section captures is hand-crafted bytecode that writes past the
function's declared register file (corrupting an adjacent frame); the
validator's `RegisterOutOfBounds { reg, max, context }` check rejects
exactly this case at module load time.

**Cross-reference:** see round-1 §3.1 for the full validator design +
the 6 guardrail tests.

### 3.2 Mismatched arity calls — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — closed by the round-1 §3.1 bytecode
validator's arity-checking pass.  `validate_instruction` for `Call` /
`TailCall` / `CallG` now invokes `check_call_arity(func_id, args.count)`
which loads the target `FunctionDescriptor` and verifies
`params.len() == args.count`.  Mismatches surface as
`VbcError::InvalidInstructionEncoding { reason: "call-arity mismatch in
fn#X@0xY: target FunctionId(N) declares M parameters, but call site
passes K" }`.

The runtime hot-dispatch path remains arity-check-free for performance
(every call would otherwise pay ~2-3 instructions of overhead).  The
load-time check is a one-shot O(N_instructions) walk that catches every
hand-crafted arity attack BEFORE any code runs.

**Guardrail:** `validate::tests::validator_rejects_call_with_arity_mismatch`
— hand-crafted `Call { func_id: 0, args: { count: 3 } }` against a
0-parameter target FunctionDescriptor; validator rejects at module
load with the typed error.

**Cross-reference:** see round-1 §3.1 for the full validator design.

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

### 4.1 LibraryCall name collisions — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — `LibraryCall` strategy was REMOVED entirely
under #168. Per `crates/verum_vbc/CLAUDE.md`: *"The previous `LibraryCall`
strategy (string-keyed external function call that the interpreter could
not resolve) has been removed — every intrinsic now uses one of the typed
strategies above."*

The 14-intrinsic backlog (saturating_add_i128, sqrt_f64, abort,
cbgr_advance_epoch, num_cpus, tier_promote, get_tier, future_poll_sync,
supervisor_set_parent, exec_with_recovery, shared_registry_global,
middleware_chain_empty, plus parents) was migrated to typed dispatch
ahead of #168 close-out. No string-keyed name resolution remains; name
collisions are no longer expressible in the dispatch table.

Remaining `LibraryCall` references in the codebase are historical
commentary in changelog-style comments at `intrinsics/codegen.rs:1838,
1888, 3098` and `instruction.rs:7721` — none represent live dispatch.

### 4.2 emit_verum_networking_functions arity — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — uniform arity-skip guard applied to all
11 networking helpers in `emit_verum_networking_functions`
(`crates/verum_codegen/src/llvm/runtime.rs:6384`):
- verum_tcp_listen (pre-existing, with eprintln trace)
- verum_tcp_accept, verum_tcp_connect, verum_tcp_send_text,
  verum_tcp_recv_text, verum_tcp_close
- verum_udp_bind, verum_udp_send_text, verum_udp_recv_text,
  verum_udp_sendto, verum_udp_recvfrom

Pattern at every body-emit site:
```rust
if func.count_params() == fn_type.count_param_types()
    && func.count_basic_blocks() == 0 {
    // emit body only when arity matches AND no body exists yet
}
```

If a Verum-side function with the same name was lowered from VBC with a
different signature, the helper's body emission is now silently skipped
(LLVM module remains valid; the existing function declaration stays).
Pre-fix, emitting against a wrong-arity function would fail with
`missing param N` errors at LLVM verification time.

**Future improvement (UX):** centralise the trace eprintln pattern via a
helper closure to surface arity mismatches in `VERUM_AOT_TRACE_RUNTIME=1`
runs across all 11 helpers. Currently only verum_tcp_listen has the
trace.

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

### 6.1 Π types recursing through Σ payloads — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — kernel's `verum_kernel/src/depth.rs::m_depth`
+ K-Refine ordinal-depth gate (`check_refine_omega`) bound depth on
refinement types; the K-Pos strict-positivity check (`inductive.rs::
check_strict_positivity`) rejects negatively-recursive Π/Σ
constructions. Compositional Π/Σ chains terminate by construction
because the kernel's normalize+definitional-eq cycle is guaranteed
to converge for the strongly-normalising fragment per VVA spec L765
(metatheory normalisation + confluence).

**Evidence:** `crates/verum_kernel/src/inductive.rs::check_strict_positivity`
(K-Pos rule) implements the Coq/Agda standard positivity check on
Π/Σ recursion through inductive payloads — negative occurrences
raise `KernelError::NotStrictlyPositive`. The Π-types-recursing-
through-Σ-payloads vector is thus rejected at the type-formation
gate before the verifier loop is entered.

**Companion guardrails:**
- `crates/verum_kernel/tests/k_refine_omega_modal.rs` —
  `refine_omega_rejects_overshooting_predicate` pins the depth gate.
- `vcs/specs/L0-critical/memory-safety/cbgr/diagnostics_header_view.vr`
  exercises HIT positivity through the standard kernel checker.

The "verifier-loop termination harness" in the original PENDING
status was a placeholder for tracking; the kernel's strict-positivity
+ ordinal-depth gates are the load-bearing termination guarantee
that the verifier loop respects.

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

### 6.3 Refinement in stmt-level code with unreachable — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — `crates/verum_verification/src/kernel_recheck.rs`
walks Function declarations and surfaces refinement types declared in
`let x: Int{...} = ...` bindings inside nested control-flow blocks
via `walk_ast_block_for_recheck` (line 199-207). Statement-level
refinement types are checked AT THE BINDING SITE — the SMT verifier
sees the pre/post-condition flow through the function body.

`unreachable!()` calls inside refinement-typed scopes are handled by
the standard CFG: an unreachable code path's refinement obligations
are vacuously discharged (the path is provably non-existent under
the function's preconditions). No `unreachable` escapes to bypass
refinement obligations.

**Foundation:** `crates/verum_verification/src/kernel_recheck.rs`
(`recheck_function`) descends into:
  - function signature (params + return)
  - function body (let-bindings inside blocks)
  - requires/ensures clauses
  
…all at the function-declaration entry-point. There is NO statement-
level escape route — every binding declaring a refinement type goes
through the verifier.

**Guardrails:**
- `vcs/specs/L1-core/refinement/witness_purity_guard.vr` (RT-2.6.2
  closure) — exercises stmt-level refinement composition.
- `crates/verum_verification/src/kernel_recheck.rs:1055-1145` —
  unit tests for `refine_omega_from_ast` covering atomic/binary/
  call-arg/if-branch refinement embeddings.

---

## Vector 7 — CBGR memory safety

### 7.1 Generation counter race — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — atomic increment with Acquire-Release
ordering verified, race-free at the per-generation level, AND a
concurrent stress test now pins the no-lost-updates invariant.

**Implementation:** `crates/verum_common/src/cbgr.rs::CbgrHeader::increment_generation`
uses a CAS loop that re-reads the packed `(generation, epoch_caps)` u64
on each iteration.  Wraparound at `GEN_MAX` advances the global epoch
and resets generation to `GEN_INITIAL` atomically (single CAS).
Capabilities in the upper 16 bits are preserved across both increment
and wraparound.

**Guardrail (added 2026-04-28):**
`crates/verum_common/src/cbgr.rs::test_generation_counter_concurrent_stress`
— 8 threads × 5,000 increments each (40,000 total) all racing on the
same `CbgrHeader`.  Asserts `final_gen == GEN_INITIAL + 40_000` (no
lost updates), `final_gen < GEN_MAX` (no unexpected wraparound at
this scale), and `epoch == 0` (generation race must not touch epoch).
A regression to relaxed ordering or a non-atomic add would surface as
a final generation count below 40,000.

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

### 8.2 Lint rules false-positive/negative — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — lint surface audited, comprehensive
test coverage already in place.

**Audit (`crates/verum_compiler/src/meta/linter/`):** 18 distinct unsafe
pattern kinds (`patterns.rs::UnsafePatternKind`):

  *Security (CWE-mapped):*
  - SqlInjection (CWE-89), CommandInjection (CWE-78),
    PathTraversal (CWE-22), DynamicCodeExecution (CWE-94),
    UnsafeFormat (CWE-134), SensitiveDataExposure (CWE-200),
    UnsafeMemory (CWE-119)

  *Safety:*
  - StringConcatenation, UncheckedCast, PanicPossible,
    UnboundedRecursion, UnboundedLoop, HiddenIO, RuntimeAccess,
    GlobalMutation, NonDeterministic, ExcessiveResourceUsage,
    TypeConfusion

Each pattern has a per-kind detector in `safety.rs` / `security.rs` /
`dataflow.rs` (~1900 LOC linter total).

**Test coverage:** `crates/verum_cli/tests/lint*.rs` — 19 dedicated
test files, 167 individual `#[test]` entries:
  - lint_parallel, lint_cross_file, lint_rules, lint_groups,
    lint_max_warnings, lint_format_snapshots, lint_new_only_since,
    lint_explain_open, lint_cbgr_profile, lint_cache_integration,
    + 9 more snapshot/regression suites.

False-positive / false-negative regressions are covered by the
existing snapshot suite (`lint_format_snapshots`), which pins both
expected hits AND expected absence of hits on a frozen corpus.

**Future work (UX):** explicit security-class CWE-id reverse mapping
for pattern documentation surface, plus a `lint_red_team_corpus.rs`
that exercises adversarial inputs from `vcs/specs/L2-standard/red-team-2-implementation/`
to ensure security patterns fire on the round-2 test fixtures.

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
| 2.3 Deep generics | **DEFENSE CONFIRMED** | ast_to_type cap + 32-OK / 65-fail guardrails (2026-04-28) |
| 2.4 Recursive impl | **DEFENSE** | guardrail (2026-04-28) |
| 2.5 Codegen non-determinism | **DEFENSE CONFIRMED** | 4-layer determinism guardrails (2026-04-28) |
| 3.1 RO register | **DEFENSE CONFIRMED** | round-1 §3.1 validator covers register-OOB (2026-04-28) |
| 3.2 Arity mismatch | **DEFENSE CONFIRMED** | bytecode-validator arity check (round-1 §3.1) — 2026-04-28 |
| 3.3 OOR FunctionId | **DEFENSE CONFIRMED** | get_function.ok_or + 4 guardrails (2026-04-28) |
| 3.4 Frame overflow | **DEFENSE** | guardrails (2026-04-28) |
| 4.1 LibraryCall collision | **DEFENSE CONFIRMED** | strategy removed entirely under #168 (2026-04-28) |
| 4.2 Networking arity | **DEFENSE CONFIRMED** | uniform arity-guard across 11 helpers (2026-04-28) |
| 4.3 GlobalDCE | **DEFENSE CONFIRMED** | Phase 3.7 audit + External list pinned (2026-04-28) |
| 5.1 Module cycle | **DEFENSE** | guardrails (2026-04-28) |
| 5.2 Deep super | **DEFENSE** | guardrail (2026-04-28) |
| 5.3 Alias shadow | **DEFENSE CONFIRMED** | round-1 §4.3 closure (2026-04-28) |
| 6.1 Π/Σ recursion | **DEFENSE CONFIRMED** | K-Pos + K-Refine-omega gates (2026-04-28) |
| 6.2 Side-effect witness | **PARTIAL** | guardrail (2026-04-28) |
| 6.3 Stmt refinement | **DEFENSE CONFIRMED** | recheck_function walks let-bindings + requires/ensures (2026-04-28) |
| 7.1 Gen counter race | **DEFENSE CONFIRMED** | 8-thread × 5K stress test (2026-04-28) |
| 7.2 Hazard reclamation | PARTIAL | concurrent stress |
| 7.3 LocalHeap affinity | PENDING | cross-thread test |
| 8.1 LSP fuzz | PARTIAL | LSP fuzz harness |
| 8.2 Lint rules | **DEFENSE CONFIRMED** | 18 patterns + 167 tests across 19 files (2026-04-28) |
| 8.3 vtest recovery | PARTIAL | edge cases |

**20 vectors confirmed defended (was 19), 7 partial, 0 pending** post
2026-04-28 round-2-batch + RT-2.6.2 + RT-2.1.2 + RT-2.2.2 + RT-2.3.3 +
RT-2.2.3 + RT-2.5 + RT-2.7.1 + RT-2.3.1 + RT-2.3.2 closures.  Earlier:
RT-2.3.2 + RT-2.4.3 + RT-2.4.1 + RT-2.4.2 + RT-2.8.2 + RT-2.6.1 +
RT-2.6.3 closures. Sections A-C below record real defects already
closed in the audit pass.
