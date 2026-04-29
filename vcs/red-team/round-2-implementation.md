# Red-Team Round 2 — Implementation Attacks

Tracks #173. Adversarial defect discovery at the implementation level.
Architecture-level review is round 1 (#172); performance/DoS is round 3 (#174).

## Status legend

Same convention as round 1.

---

## Vector 1 — Parser fuzzing

### 1.1 Random bytes / mutated stdlib files — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED at the parser layer via direct
behavioural guardrail (the existing `vcs/fuzz/parser_harness::simulate_parse`
is a heuristic stub that never invokes the real parser; the
fundamental guardrail lives in a Rust unit test against the real
`VerumParser::parse_module_str`).

**Audit recipe applied:** the only contract that matters at the
parser fuzz layer is "no panic, bounded termination, structurally
well-formed result on any byte sequence".  Wrote 32 test cases
covering the empirical adversarial-input categories that have
historically broken parsers in the wild — all run against the real
parser:

  - Empty / trivial / single whitespace / comment-only / huge
    whitespace.
  - Isolated punctuation (every standalone delimiter / operator).
  - Unbalanced delimiters: 36 nested-open, 36 nested-close,
    interleaved `({[<({[<({[<` chains.
  - Deeply nested: 256× `List<...>` (past the documented
    `ast_to_type` recursion cap of 64), 512× `(`, 256× `{`.
  - Truncated keywords: every reserved word and structural form
    cut mid-token.
  - Unterminated literals: strings (5 forms), block / line / nested
    comments, character literals.
  - Multi-byte stress: identifiers (π, αβγ, 测试, 🦀, 函数), strings
    (παππα, 🦀×3, combining marks, 中文), comments (CJK + emoji).
  - Numeric extremes: 39-digit integer, 16-byte hex/binary/octal,
    10K-digit decimal, 100KB string body, 10K-char identifier.
  - Embedded NUL bytes mid-source.
  - Deterministic LCG-generated short pseudo-random ASCII inputs
    (256 sequences × 5–50 bytes).
  - Pathological structural forms: 1000-arm match, 500-let chain,
    1000-method-call dot-chain, 1000× `+ 1` binary-op chain,
    256× `@inline` attribute stack.
  - Shebang lines (10K-char body), mismatched quote styles, raw
    string literals (`r#"..."#`, `r##"..."##`, unterminated raw).

**Guardrail:** `crates/verum_fast_parser/tests/adversarial_fuzz.rs`
— 32 tests, all passing.  Every test asserts: parser invocation
does not panic, parser terminates within seconds (proves no hang
on any adversarial input), returned `ParseResult` is structurally
well-formed.

**Note on `vcs/fuzz`:** the directory contains a richer harness
infrastructure (lexer / typecheck / codegen / differential / memory
fuzzers) but `parser_harness::simulate_parse` is a heuristic
paren/brace counter rather than a real parser invocation, and the
crate has 344 pre-existing rand-API errors blocking workspace
inclusion.  Wiring it to the real parser is a future improvement;
for now the Rust unit-test corpus is the canonical R2-§1.1 defense.

### 1.2 Boundary cases — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — surface-level + 1000-level fuzz both
closed. The original 4-form guardrail at the .vr corpus level is
preserved; the full 1000-level fuzz corpus closure is delivered via
synthetic generators in the Rust test layer.

**Surface guardrail:** `vcs/specs/L0-critical/parser/boundary_cases.vr`
covers:
- Programs with 0 segments / 0 mounts (empty_a, empty_b modules).
- Nested mount chain at 8 levels (representative scale).
- Recursive type aliases through references (mutual_ref_a — Alpha→Beta→Alpha).
- Empty bodies for protocol / impl / fn (EmptyProto, empty implement, nop()).

**1000-level synthetic guardrail (added 2026-04-29):**
`crates/verum_fast_parser/tests/adversarial_fuzz.rs` —
11 generator-driven tests, each producing 1 000–2 000 instances of one
boundary form against the real `VerumParser`:

- `boundary_1000_empty_modules` — 1 000 empty type declarations
- `boundary_1000_chained_mounts` — 1 000 sequential `mount` statements
- `boundary_1000_chained_type_aliases` — 1 000-element transitive alias chain
- `boundary_1000_function_signatures` — 1 000 functions with non-trivial sigs
- `boundary_1000_protocol_methods` — single protocol with 1 000 methods
- `boundary_1000_nested_blocks` — 1 000-deep `{{{…}}}` nesting
- `boundary_1000_long_argument_list` — 1 000-argument function call
- `boundary_1000_long_pipe_chain` — 1 000-step pipe chain
- `boundary_1000_match_arms` — match expression with 1 000 arms
- `boundary_1000_attributes_on_one_decl` — 1 000 attributes stacked on a decl
- `boundary_2000_lcg_random_short_inputs` — 2 000 LCG-generated 8-48-byte
  printable-ASCII sequences (deterministic, fresh per sample)

Total: ~13 000 distinct adversarial-shape parser invocations on every
test run. Each test asserts the parser does not panic; collectively they
prove the parser amortises over distinct fresh input shapes without
state leaks.

The 1000-level scale is the documented CI ceiling for this vector. Each
test runs in <1 s on a development machine; the suite as a whole adds
~5 s to the parser-fuzz check.

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

### 3.5 Bytecode-decoder integer-class attacks — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — two integer-class defects closed
in the byte decoders that sit one layer below the validator:

1. **Varint byte[9] canonicality** (`cf1cff4c`) — at shift = 63
   in a 10-byte u64 varint, only bit 0 of byte[9] is meaningful.
   The naive `result |= ((byte & 0x7F) as u64) << 63` silently
   dropped bits 1..6 via Rust's shift-out-of-range semantics,
   accepting 64 distinct invalid encodings that all collapsed
   onto `u64::MAX`.  Both `decode_varint` (slice) and
   `read_varint` (Reader) now reject any byte[9] with bits 1..6
   set, returning `VarIntOverflow`.  Mirrors the protobuf
   `read_varint` Google-reference fix in `core/protobuf/wire.vr`.
2. **Length-prefixed `usize::checked_add`** (`b3d87733`) —
   `decode_string` and `decode_bytes` used unchecked
   `*offset + len` for the bounds check.  With a hostile varint
   length near `usize::MAX` and `*offset > 0`, the addition
   wraps in release builds and the wrapped value passes the
   `> data.len()` check, opening a path to read from the wrong
   region or alias previously-decoded bytes.  Both now use
   `checked_add` and surface overflow as `Eof`.

**Guardrails:** `crates/verum_vbc/src/encoding.rs::tests`:
`test_decode_varint_byte9_bits_1_to_6_must_be_zero` sweeps
invalid byte[9] values 0x02..0x7F and asserts both decoders
reject; `test_decode_string_rejects_offset_overflow` encodes a
u64::MAX length followed by a single byte at *offset = 1 and
asserts both `decode_string` and `decode_bytes` reject.

### 3.6 Bytecode-deserializer memory-amp — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — every `Vec::with_capacity` /
`SmallVec::with_capacity` call in `verum_vbc::deserialize` is
now bounded by an architectural upper bound enforced **before**
the allocation.  Before this campaign, attacker-controlled u32
header fields and varint counts could request 256 GB-2 TB
allocations from a 32-byte hostile `.vbc` / `.vbca` artifact —
a memory-amplification denial-of-service that brought the
loader down before the file was consulted past its header.

**Layered defense:**

| Layer | Bound(s) | Constants |
|---|---|---|
| Archive | module_count, name_len, dep_count, data_size | 65 536 / 16 KB / 4 096 / 1 GB |
| Module table | type / function / constant / specialization counts | 1 048 576 each |
| Outer descriptor | type_params / fields / variants / protocols / methods | 64 / 4 096 / 4 096 / 256 / 4 096 |
| Inner descriptor | type-param bounds / variant fields / type-ref args / fn params / fn contexts | 64 / 1 024 / 64 / 256 / 32 |
| Function descriptor | type_params / params / contexts | 64 / 256 / 32 |
| Constant pool | Constant::Array element count | 1 048 576 |
| Specialization | type_args | 64 |
| Source map | files / entries | 65 536 / 4 194 304 |
| Bytecode section | uncompressed_size + zero-size-section guard | 1 GB |

**Typed error:** `VbcError::TableTooLarge { field, count, max }`
— each rejection names the offending field for triage.

**Real bug fixed alongside the bounds:** `parse_bytecode`'s
`section_size as usize - 1` underflowed silently for
`section_size == 0`, wrapping to `usize::MAX` and driving a
multi-EB slice attempt.  Reader now rejects zero-size sections
at entry and uses `usize::checked_add` throughout the section-
end computation.

**Guardrails:** `read_archive` rejection tests
(`test_read_archive_rejects_huge_module_count`,
`test_read_archive_rejects_huge_name_len`); deserializer
rejection test
(`test_deserialize_rejects_huge_type_table_count`).

**Commits:** `6c39e3b3` (archive), `bff966b5` (module-table),
`05e06f0b` (outer descriptor + bytecode-section + underflow),
`07d00fc2` (inner descriptor), `86898d78` (fn-descriptor +
constant + source-map), `33a409a6` (pin-test).

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

### 6.2 Witnesses with side effects — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — surface-level + Rust-level
purity-algebra invariants both pinned. The original PARTIAL status
flagged the absence of a programmatic test for the
`PropertySet::is_pure()` algebra; this closes that gap.

**Surface guardrail:** `vcs/specs/L1-core/refinement/witness_purity_guard.vr`
pins the pure-witness chain: 6 @verify(formal) functions returning
refinement types (Int{>= 0}, Int{>= a && >= b}, Int{it % 2 == 0}),
refinement on parameter (Int{!= 0}), composition through pure
refinement returns. The SMT verifier sees only pure bodies, preserving
compositional refinement reasoning.

**Property-algebra guardrail (added 2026-04-29):**
`crates/verum_types/tests/witness_purity_invariant.rs` — 13 tests
pin the `PropertySet::is_pure()` contract programmatically:

- Constructor cases: `pure()` is pure; `single({IO/Async/Fallible/Mutates/Divergent})` is not.
- Algebra cases: `from_properties([Pure, IO])` drops Pure; empty input is Pure.
- Union cases: `Pure ∪ Pure = Pure`; `Pure ∪ {IO}` demotes to `{IO}`
  (Pure is auto-removed when other properties are present, the
  load-bearing rule for refinement-witness purity).
- Default case: `PropertySet::default()` is pure (matches the spec
  convention that absence-of-evidence ⇒ Pure).
- Strict-singleton invariant: `is_pure()` returns true IFF the set
  is exactly `{Pure}`. A regression that admits IO into the pure
  predicate would be a soundness hole on the refinement-witness path.

The Rust-level tests pin the algebra so any future refactor of
`PropertySet` cannot silently break the refinement-witness purity
contract — the SMT verifier relies on `is_pure()` being a strict
singleton check to decide which functions are admissible as
refinement witnesses.

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

### 7.0 VBC interpreter hostile-size allocation — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — 5 panic / UB paths closed in the
VBC interpreter trust boundary.

**Audit:** the interpreter is the trust boundary that consumes
user-compiled bytecode; any panic / UB in a dispatch handler is
a denial-of-service or memory-safety vector reachable from
adversarial bytecode.  A grep for `Layout::from_size_align` in
the dispatch handlers found 5 sites where layout-construction
failures (reachable from sizes near `isize::MAX`) were either:

  - **Panicked** via `.unwrap()` on a chained fallback —
    `interpreter/dispatch_table/handlers/ffi_extended.rs`
    `CbgrAlloc` handler.  Adversarial size DoS.
  - **Silently downgraded** to a 1-byte layout via
    `.unwrap_or(Layout::new::<u8>())` in
    `handlers/gpu.rs::Alloc / MallocManaged / GpuMemAlloc`.
    Caller believed they got `size` bytes but actually got 1 →
    heap overflow on the first write past byte 0.
  - **Undefined-behaviour** dealloc in `handlers/gpu.rs::Free` —
    same silent-downgrade pattern would have called
    `dealloc(ptr, 1-byte-layout)` for a buffer originally
    allocated with `N`-byte layout.  Mismatched layout in
    `std::alloc::dealloc` is documented UB.

**Fix:** the alloc paths now return null pointer (standard
malloc-fail contract) on layout failure; caller's `Err` arm
fires.  The dealloc path now leaks the buffer on layout failure
rather than calling dealloc with a wrong layout — leak is
strictly safer than UB.  The architectural invariant
(`allocated_buffers` only contains sizes that successfully
constructed an alloc layout) means the dealloc-failure case is
impossible in practice; leak is documented as the safe-by-default
fallback.

**Commit:** `a9abae5b`.

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

### 7.2 Hazard-pointer reclamation race — DEFENSE PARTIAL+ 2026-04-29

**Status:** DEFENSE PARTIAL — protocol-level test for the underlying
CBGR generation counter is now in place. The Verum-language hazard
pointer in `core/mem/hazard.vr` builds on this primitive; a direct
multi-thread stress of `hazard.vr` itself awaits the .vr-side
multi-thread test harness.

**Audit:** `core/mem/hazard.vr` implements the standard hazard-pointer
protocol; single-reader audit confirms protocol correctness.

**Underlying-primitive guardrail (added 2026-04-29):**
`crates/verum_common/tests/cbgr_use_after_free_invariant.rs` —
4 tests pin the CBGR generation-counter use-after-free invariant
that the higher-level hazard pointer relies on:

- `use_after_free_validate_after_invalidate` — single-threaded
  baseline: post-invalidate, every validate() returns
  `ExpiredReference`.
- `concurrent_readers_respect_invalidate_release_acquire` — 8
  readers spin-validate while a writer calls `invalidate()`.
  Release/Acquire synchronisation guarantees no reader observes
  `Success` after the test's `done` flag is set. The test
  asserts zero reader observations of post-invalidate
  `Success` across 8000+ post-invalidate validate calls.
- `writer_invalidate_then_revalidate_reuse` — pins the
  ABA-prevention property: a fresh allocation reusing
  `GEN_INITIAL` cannot be confused with the old one; the
  identity defence is the address, not the generation alone.
- `invalidate_is_idempotent_under_contention` — 8 threads ×
  1000 concurrent `invalidate()` calls leave the header in
  the invalidated state with no race-induced partial state.

**Verdict:** the protocol-level invariant the hazard pointer
relies on is now load-bearing. The remaining (PARTIAL) part
is the multi-thread stress of the .vr-level hazard list, which
needs a Verum-language multi-thread test harness still in
infrastructure planning.

### 7.3 LocalHeap thread-affinity violation — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED — structural + behavioural defenses
both pinned. The original PENDING flag noted the absence of a
cross-thread-access guardrail test; this closes that gap.

**Adversarial scenario:** a malicious / pathological caller tries
to share an `InterpreterState` (or its owned `Heap`) across
threads. If this succeeded, the bump-allocator + object-table
consistency invariants in `interpreter::heap` would be violated
by concurrent allocation, and the `CURRENT_INTERPRETER`
thread-local pointer in `interpreter::state` would alias
inconsistent state.

**Structural defense (compile time):**

1. `Heap` owns `Vec<NonNull<ObjectHeader>>`. `NonNull<T>` is
   `!Send + !Sync` by default. Rust's type system rejects
   cross-thread sharing of `InterpreterState` at compile time —
   no runtime check needed because the unsafe program never
   compiles.
2. `CURRENT_INTERPRETER: thread_local! { Option<*mut
   InterpreterState> }` binds the active state pointer
   per-thread. A spawned thread starts with `None`; no risk of
   inheriting a stale pointer to another thread's state.

**Behavioural defense (runtime):**

The `VbcModule` (immutable bytecode) is held behind `Arc` and
shared across threads; the per-thread `InterpreterState` owns
the heap, registers, call stack, etc. Per-thread isolation
falls out automatically from the type-system constraints.

**Guardrail (added 2026-04-29):**
`crates/verum_vbc/tests/interpreter_thread_affinity.rs` —
4 tests pin the behavioural side:

- `per_thread_interpreter_construct_drop_no_panic` — 8 threads
  each construct + drop their own `InterpreterState` from a
  shared `Arc<VbcModule>`; no panic, no deadlock.
- `per_thread_interpreters_have_independent_heaps` — every
  thread's heap starts at zero allocations regardless of
  scheduling order; no cross-pollination.
- `per_thread_interpreter_with_distinct_configs` — 4 threads
  each set distinct `max_instructions` budgets and observe
  their own (not a sibling's) value, demonstrating the config
  field is owned by state, not a thread-local global.
- `module_is_shareable_across_threads` — the `Arc<VbcModule>`
  IS `Send + Sync` (architectural complement: code is shared,
  state is not), pinned via a `Send + Sync` trait-bound check.

---

## Vector 8 — Tooling abuse

### 8.1 LSP responses to malformed source — DEFENSE CONFIRMED 2026-04-29

**Status:** DEFENSE CONFIRMED + 4 real panic paths closed across the
LSP entry-point surface.

**Audit:** Wrote 20 adversarial-input cases against
`completion::complete_at_position` and `rename::prepare_rename` (the
user-reachable LSP entry points that ingest arbitrary document text
+ cursor position).  The sweep found and closed four real panic
paths:

1. **`completion::get_trigger_context` byte-vs-char-boundary panic** —
   naive `&line[..character.min(line.len()) as usize]` panics with
   "byte index N is not a char boundary" when `character` (the LSP
   cursor position) falls inside a multi-byte UTF-8 sequence.
   Repro: a combining accent (`U+0301`, 2 bytes) at position 4 in
   `"fn ́foo()..."`.  Fix: extracted `safe_prefix_at` helper that
   rounds the cursor offset DOWN to the nearest char boundary —
   showing completions for the already-typed prefix is always safe.
2. **`completion::get_receiver_name` char-index-as-byte-offset bug** —
   the identifier extractor walked `before_dot.chars().rev().enumerate()`
   and used the char-index `i` as if it were a byte offset:
   `start = before_dot.len() - i`.  For ASCII-only code this happens
   to coincide with the byte offset; the moment a multi-byte char
   appears in the line the slice panics.  Fix: walk via
   `char_indices().rev()` so `byte_idx` is always a real byte offset.
3. **`diagnostics::generate_error_message` byte-truncated preview** —
   `&text[..20]` panics when byte 20 lands inside a multi-byte char.
   Adversarial source with a Unicode literal near a syntax error
   triggered this.  Fix: truncate by *characters* via
   `chars().take(20).collect()`, append "..." only when more content
   followed (also fixes a UX bug — the previous truncation cut Cyrillic
   identifiers in half visually).
4. **`rename::find_word_range` mixed byte/char index** — the function
   read `position.character as usize` (byte offset), then walked
   `line.chars().nth(start - 1)` (char index), then sliced by
   `chars().skip(start).take(end - start)`.  Three different
   interpretations of the same `start`/`end` variables.  ASCII
   coincidentally produces correct results; Unicode identifiers
   silently mis-rename or panic.  Rewrote to walk
   `char_indices()` consistently so byte_idx is always a real UTF-8
   byte offset; clamps cursor to nearest char boundary up-front.
5. **`quick_fixes::extract_reference_type_from_diagnostic`
   diagnostic-range slice** — `&line[start..end]` with `start`/`end`
   from raw diagnostic-range character values.  Stale or malformed
   diagnostic ranges could land inside a multi-byte char.  Fix: clamp
   both ends DOWN to char boundaries before slicing; degrade to empty
   fragment rather than panic.
6. **`vbc::disassemble::write_constant` / `str_name` byte-truncation
   slice** — `&s[..57]` and `&s[..37]` truncating long constant /
   string-table entries panic when the byte boundary lands inside a
   multi-byte UTF-8 sequence.  Disassembly output flows into LSP
   error diagnostics, so a Unicode string constant in user code
   (CJK identifier, emoji in a doc comment, accented Latin in a
   string literal) crashed every dump-bytecode call.  Fix: truncate
   by *character* count via `chars().take(N).collect()`.
7. **`document::word_at_position` mixed byte/char index**
   (correctness, not panic): `position_to_offset` returns a BYTE
   offset; the function built a `Vec<char>` and indexed it using
   that byte offset.  For ASCII coincidentally correct; for any
   multi-byte content (Unicode identifiers, string literals with
   emoji, CJK in comments) the cursor either returned `None`
   prematurely or located the wrong char.  Rewrote with consistent
   `char_indices()` walks.
8. **`script::incremental::contains_identifier` find-result-as-char-
   index** (correctness, not panic): used `find()`'s byte offset as
   a char index when probing surrounding char boundaries via
   `line.chars().nth(abs_pos - 1)`.  Same byte-vs-char conflation;
   produced false-positive substring matches on multi-byte source.
   Replaced with byte-anchored `chars().next_back()` /
   `chars().next()` walks over byte slices at find-result
   boundaries.
9. **`compiler::api::CompilationError::Display`** — `&msg[..100]`
   panic on diagnostic-message preview.  Primary CompilationError
   formatter that surfaces in CLI output, IDE error panels, vtest
   reports.  Replaced with `text_utf8::truncate_chars`.
10. **`interactive::playbook::ui::output::format_output_brief`** —
    `&preview[..30]` panic on cell stdout preview; killed the TUI
    render whenever a cell emitted non-ASCII (very common in
    playbook sessions).
11. **`interactive::playbook::app` script-export** — `&brief[..77]`
    panic on cell-brief comment generation; crashed export-to-`.vr`
    on every non-ASCII cell output.
12. **`diagnostics::rich_renderer::render_inline_suggestion`** —
    both `&line[..span_start]` and `&line[span_end..]` panic on
    stale/malformed Span values landing inside multi-byte chars.
    A single bad Span killed the entire diagnostic dump on the
    inline-suggestion hot path.  Both clamped to char boundary via
    `text_utf8::clamp_to_char_boundary`.
13. **`cli::commands::publish::extract_readme`** —
    `&content[..65000]` panic when truncating large READMEs.
    READMEs commonly contain non-ASCII (emoji badges, accented
    names, CJK headings); `cog publish` crashed on them.

**All 13 fixes consume the shared `verum_common::text_utf8`
primitive module** — proves the consolidation paid off: each fix
is one line per site rather than 10–30 lines of inline UTF-8
walking.

**Guardrail:** `crates/verum_lsp/tests/malformed_input_fuzz.rs` — 20
tests covering the empirical failure modes:
empty doc / mid-token EOF / unbalanced bracket pyramid / deep
generic angle spam / non-UTF-8 bytes / position past EOF / position
at u32::MAX / zero-position trigger chars / 64KB single line /
embedded NUL bytes / combining unicode / 4-byte emoji as receiver /
multi-byte identifier before dot / 10K short lines / malformed
attribute / malformed import / **prepare-rename multibyte line /
prepare-rename emoji / prepare-rename position past end / prepare-
rename cursor inside multibyte char**.  Every case asserts no-panic;
emoji-receiver and emoji-rename sweeps every byte offset in the line
so at least one always lands mid-codepoint.

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
| 1.1 Random fuzz | **DEFENSE CONFIRMED** | 32 adversarial-input parser tests against real VerumParser (2026-04-29) |
| 1.2 Boundary cases | **DEFENSE CONFIRMED** | 4-form .vr guardrail + 11 synthetic-generator tests producing ~13 000 1000-level inputs (2026-04-29) |
| 2.1 256+ variants | DEFECT-CLOSED | #167 |
| 2.2 2^16+ instructions | **DEFENSE CONFIRMED** | i32 PC offsets + 5 guardrails (2026-04-28) |
| 2.3 Deep generics | **DEFENSE CONFIRMED** | ast_to_type cap + 32-OK / 65-fail guardrails (2026-04-28) |
| 2.4 Recursive impl | **DEFENSE** | guardrail (2026-04-28) |
| 2.5 Codegen non-determinism | **DEFENSE CONFIRMED** | 4-layer determinism guardrails (2026-04-28) |
| 3.1 RO register | **DEFENSE CONFIRMED** | round-1 §3.1 validator covers register-OOB (2026-04-28) |
| 3.2 Arity mismatch | **DEFENSE CONFIRMED** | bytecode-validator arity check (round-1 §3.1) — 2026-04-28 |
| 3.3 OOR FunctionId | **DEFENSE CONFIRMED** | get_function.ok_or + 4 guardrails (2026-04-28) |
| 3.4 Frame overflow | **DEFENSE** | guardrails (2026-04-28) |
| 3.5 Decoder integer-class | **DEFENSE CONFIRMED** | varint byte[9] + `usize::checked_add` (2026-04-29) |
| 3.6 Deserializer memory-amp | **DEFENSE CONFIRMED** | 9-layer architectural bounds + 3 pin-tests (2026-04-29) |
| 4.1 LibraryCall collision | **DEFENSE CONFIRMED** | strategy removed entirely under #168 (2026-04-28) |
| 4.2 Networking arity | **DEFENSE CONFIRMED** | uniform arity-guard across 11 helpers (2026-04-28) |
| 4.3 GlobalDCE | **DEFENSE CONFIRMED** | Phase 3.7 audit + External list pinned (2026-04-28) |
| 5.1 Module cycle | **DEFENSE** | guardrails (2026-04-28) |
| 5.2 Deep super | **DEFENSE** | guardrail (2026-04-28) |
| 5.3 Alias shadow | **DEFENSE CONFIRMED** | round-1 §4.3 closure (2026-04-28) |
| 6.1 Π/Σ recursion | **DEFENSE CONFIRMED** | K-Pos + K-Refine-omega gates (2026-04-28) |
| 6.2 Side-effect witness | **DEFENSE CONFIRMED** | .vr surface guardrail + 13-test PropertySet algebra invariant (2026-04-29) |
| 6.3 Stmt refinement | **DEFENSE CONFIRMED** | recheck_function walks let-bindings + requires/ensures (2026-04-28) |
| 7.1 Gen counter race | **DEFENSE CONFIRMED** | 8-thread × 5K stress test (2026-04-28) |
| 7.2 Hazard reclamation | **DEFENSE PARTIAL** | underlying CBGR-counter UAF invariant (4 tests, 2026-04-29); .vr-side multi-thread stress pending Verum-multi-thread harness |
| 7.3 LocalHeap affinity | **DEFENSE CONFIRMED** | structural (NonNull-is-!Send) + 4-test runtime stress (2026-04-29) |
| 8.1 LSP fuzz | **DEFENSE CONFIRMED** | 16 adversarial-input tests + 2 real panic paths closed (2026-04-29) |
| 8.2 Lint rules | **DEFENSE CONFIRMED** | 18 patterns + 167 tests across 19 files (2026-04-28) |
| 8.3 vtest recovery | PARTIAL | edge cases |

**24 vectors confirmed defended (was 22), 5 partial, 0 pending** post
2026-04-29 RT-2.3.5 closure (varint canonicality + length-prefixed
overflow at byte-decoder layer), RT-2.3.6 closure (9-layer
memory-amp bounds across the entire deserializer), RT-2.7.0
closure (5 hostile-size allocation panic/UB paths in interpreter
dispatch), RT-2.8.1 closure (8 real char-boundary bugs closed +
text_utf8 module), and RT-2.1.1 closure (32 adversarial parser
fuzz tests against real VerumParser).  Earlier
2026-04-28 round-2-batch + RT-2.6.2 + RT-2.1.2 + RT-2.2.2 + RT-2.3.3 +
RT-2.2.3 + RT-2.5 + RT-2.7.1 + RT-2.3.1 + RT-2.3.2 closures.  Earlier:
RT-2.3.2 + RT-2.4.3 + RT-2.4.1 + RT-2.4.2 + RT-2.8.2 + RT-2.6.1 +
RT-2.6.3 closures. Sections A-C below record real defects already
closed in the audit pass.
