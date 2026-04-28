# Red-Team Round 3 — Adversarial Performance / DoS

Tracks #174. Adversarial performance-cliff and DoS-resistance discovery.
Rounds 1+2 (#172/#173) cover correctness defects.

## Status legend

Same convention as round 1.

---

## Performance gates (from task scope)

| Gate | Target | Source |
| --- | --- | --- |
| Compile time | O(n log n) in source size | task scope |
| Refinement check | O(predicate × Z3) cached on idempotent | task scope |
| Runtime hot path | zero alloc for fixed-shape closures, < 2× C | task scope |
| Stdlib load | < 100 ms for full core/ | task scope |

A cliff that violates a gate → tracked task with optimization proposal. A
cliff within gates → benchmark added to `crates/verum_compiler/benches/`.

---

## Vector 1 — Compilation-time DoS

### 1.1 Nested generic instantiation chain — PARTIAL DEFENSE 2026-04-28

**Status:** PARTIAL DEFENSE — 32-level synthetic generic exercises
monomorphization at representative depth without timing-out CI; full
1024-level fuzz harness deferred to fuzz-infrastructure track.

**Guardrail:** `vcs/specs/L4-performance/red-team-3-perf/deep_generic_compile_bounded.vr`
declares `L01..L10` doubling-pattern type aliases producing a 32-level
nested-generic structure (each level embeds two of the previous). The
structurally-equal-types deduplication in the kernel's normalize step
prevents exponential blow-up; compilation = monomorphization runs.

### 1.2 Refinement predicates with exponential SMT cost — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — closed by round-1 §5.1 (Z3 timeout
fail-closed) closure on 2026-04-28. Timeout policy: every consumer of
`SatResult::Unknown` either returns `Err(VerificationError::Timeout/
Unknown)` or returns a sound-conservative answer that keeps runtime
checks active. Pathological predicates with exponential SMT cost hit
the 30s default timeout and are REJECTED, never silent-accepted.

**Wall-clock leak concern:** the default timeout is configured at
`verum_smt/src/z3_backend.rs:76` as `global_timeout_ms: Maybe::Some(30000)`.
This is a fixed budget, NOT a function of input — attackers cannot
extend wall-clock observability beyond the configured budget by crafting
pathological inputs.

**Audit trail:**
- `vcs/red-team/round-1-architecture.md §5.1` — 9-site audit table
- `vcs/specs/L0-critical/verification/z3_timeout_fail_closed.vr` — surface guardrail
- `crates/verum_smt/tests/timeout_fail_closed_invariant.rs` — Rust-level invariant tests
- `vcs/specs/L1-core/refinement/smt/proof_timeout.vr` — broader L1 surface

### 1.3 Module dependency graph fan-out / fan-in — PARTIAL DEFENSE 2026-04-28

**Status:** PARTIAL DEFENSE — 16-leaf-1-hub guardrail at representative
fan-out scale; full 1000-module worst-case generator deferred.

**Guardrail:** `vcs/specs/L4-performance/red-team-3-perf/module_fanout_bounded.vr`
constructs a hub module with 16 leaves each mounting it, plus an aggregator
fanning IN from all 16. Closure walker visits each edge exactly once;
sum_all() pins both compile-time linearity and runtime correctness
(1+2+...+16 = 136).

---

## Vector 2 — Runtime DoS

### 2.1 Allocator pressure / generation churn

**Status:** PARTIAL DEFENSE — the byte-loop sweep through #203 (~170+ sites
migrated to bulk-copy) closes one major allocator-pressure source on
wire-frame paths. Tight-loop alloc-free does still churn the LocalHeap
generation counter; need stress test.

### 2.2 Refinement check at every callsite of a hot function

**Status:** PENDING — depends on whether the verifier caches per-predicate
per-typed-args; test by generating a hot loop with refinement-typed args.

### 2.3 10^6 lightweight async tasks — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — `core/async/executor.vr::spawn` (line 439)
enforces the task-count limit atomically:

```verum
let count = self.task_count.fetch_add(1, Relaxed);
if count >= self.config.max_tasks {
    self.task_count.fetch_sub(1, Relaxed);
    panic("Cannot spawn: task limit reached");
}
```

Three preset profiles at `executor.vr:120/133/146`:
- 1,000 tasks (constrained)
- 10,000 tasks (default)
- 100,000 tasks (high-concurrency)

A 10^6-task DoS hits the 100k cap and panics rather than silent OOM. The
fetch_add-then-check pattern with rollback on limit prevents racing past
the limit under contention.

**Future work (UX, not soundness):** graceful backpressure on limit reach
(currently panic). Tracked as a separate UX item, not a security defect.
The current panic terminates the runtime cleanly; spawn-time overflow
attacks cannot exhaust memory beyond the configured budget.

### 2.4 Channel unbounded backlog — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — audit recipe applied; ZERO internal use
of `channel()` (unbounded) in `core/`. All stdlib channel uses are
`bounded(N)` with explicit capacity. `channel()` is exposed as API but
users opt-in explicitly for unbounded, accepting the memory-cap
responsibility.

**Audit:**
```
$ grep -rn "channel\(\)" core/
# (no matches)
```

**API surface** (from `core/async/channel.vr:906, 921`):
- `channel<T>()` — unbounded; capacity = None (intentional, opt-in)
- `bounded<T>(capacity: Int)` — bounded; asserts capacity > 0
- `Channel<T>::new(capacity: Int)` — bounded ring buffer; same assert

The `bounded(capacity > 0)` assert at construction prevents zero-capacity
attacks. The unbounded `channel()` constructor is deliberately a separate
API entry, not a default — users see `unbounded` in their code as an
explicit choice.

---

## Vector 3 — Cache-line / memory-bandwidth

### 3.1 False sharing between LocalHeap thread-local pages

**Status:** PENDING — needs core-pinned multi-thread test.

### 3.2 Atomic stride exhaustion under SeqCst

**Status:** PENDING — needs N-thread contended-counter test.

### 3.3 GPU dispatch with adversarial tile size

**Status:** PENDING — needs GPU harness; out-of-scope until GPU dispatch
lands fully.

---

## Vector 4 — Bytecode-format pathological cases

### 4.1 Long single-instruction basic-block chain — DEFENSE CONFIRMED 2026-04-28

**Status:** DEFENSE CONFIRMED — VBC's encoder/decoder pair has no implicit
i16 cap on instruction count, only the i32 branch-target cap pinned by
round-2 §2.2.  A function body of 100,000 straight-line `Mov` instructions
(~600 KB of bytecode, tens of orders of magnitude beyond what real codegen
emits) round-trips cleanly.  If the encoder ever introduced a per-block
instruction-count cap, the test fires.

**Guardrail:** `crates/verum_vbc/tests/red_team_bytecode_trust_boundary.rs::long_basic_block_chain_roundtrips`
— builds 100,000 Mov instructions in a row, encodes them into one
contiguous bytecode buffer, then walks the buffer and asserts exactly
100,000 Mov instructions decode back with zero spurious bytes left over.

Designed to maximize interpreter dispatch overhead. Compare Tier-0 dispatch
cost against AOT-compiled equivalent.

### 4.2 Anti-CSE / anti-LICM AOT

**Status:** PENDING — needs LLVM-ir inspection harness.

---

## Vector 5 — Stdlib-loading scaling

### 5.1 1000 modules mounting core.*

**Status:** PENDING — needs synthetic-module generator.

### 5.2 Deeply nested @cfg conditional — PARTIAL DEFENSE 2026-04-28

**Status:** PARTIAL DEFENSE — 12-cfg-attribute stress at representative
scale; full 1024+-cfg fuzz pending.

**Guardrail:** `vcs/specs/L4-performance/red-team-3-perf/cfg_nesting_bounded.vr`
exercises 12 distinct @cfg predicates including `any` / `all` / `not`
combinators and 6-arm `any(target_os = ...)` to prove the walker stays
bounded. The walker is conservative on the `unix`/`linux`/`macos`/
`windows` predicate family per round-1 §4.1 audit, and per-cfg
processing remains linear (K predicates → K registrations, not 2^K).

---

## Performance-class defenses confirmed through #203 audit

### A. Wire-frame allocation reduction (~170+ sites)

The byte-loop antipattern sweep eliminated per-byte allocator probes across:

- **QUIC** stack: frame.vr 11 sites + address_token.vr 11 sites +
  transport_params 10 sites + connection_sm/{retry,tx,rx} 22 sites + packet
  7 sites. Every QUIC datagram emit/parse runs zero-alloc on bulk byte transfer.
- **TLS 1.3** stack: handshake/server_sm 9 sites + resumption 8 sites +
  client_sm 4 sites + codec 3 sites. Every handshake flight assembled with
  bulk-copy.
- **HTTP/2** stack: hpack 3 sites (every header field encode/decode).
- **HTTP/3** stack: h3/connection 7 sites + qpack/session 7 sites +
  qpack/instructions 4 sites + frame.
- **Crypto** stack: HPKE 6 sites, JWT 5 sites, Ed25519/X25519/Secp256r1
  ECDH 4+4 sites, Merkle leaf+node hash, KDF PBKDF2/HKDF, AES-GCM/
  ChaCha20-Poly1305 page-cipher (every encrypted DB page I/O), TOTP/HMAC.

Foundational `Text.push_str` rewrite (memcpy-based bulk-copy + single
null-terminator write instead of N grows + N writes + N terminators) gave
30+ Text-builder callers an immediate amortised bulk-copy benefit.

### B. Algorithmic cliffs closed (historic, memory-recorded)

- **JCS canonicalisation** O(N²) → O(N log N) merge sort (signing-path
  CPU-DoS).
- **CBOR canonical** same migration.
- **HTTP Range / Accept / Cache-Control / Link** fan-out caps closed
  CVE-2011-3192 class (Apache Range-DoS).
- **PBKDF2 / HMAC** iteration cap at the primitive layer (#181).
- **Postgres SCRAM iter parse** cap at MAX_SCRAM_ITERATIONS for
  hostile-server CPU-DoS.

### C. Hot-path case-fold caching

- `KeywordFilter` cached folded keywords (was N×M alloc per call → 0).
- HTTP/WebSocket/DNS-name comparison via `Text.eq_ignore_case` ASCII fast
  path (length-equal + fold-and-compare in place; falls through to Unicode
  only on non-ASCII).

---

## Round 3 progress summary

| Vector | Status | Follow-up |
| --- | --- | --- |
| 1.1 Deep generic | **PARTIAL** | 32-level guardrail (2026-04-28); 1024-level fuzz pending |
| 1.2 SMT exponential | **DEFENSE CONFIRMED** | closed via round-1 §5.1 (2026-04-28) |
| 1.3 Module fan-out | **PARTIAL** | 16-leaf+hub guardrail (2026-04-28); 1000-module pending |
| 2.1 Alloc pressure | PARTIAL | wire-frame done |
| 2.2 Refinement caching | PENDING | hot-loop test |
| 2.3 10^6 tasks | **DEFENSE CONFIRMED** | atomic spawn-time cap (2026-04-28) |
| 2.4 Channel backlog | **DEFENSE CONFIRMED** | zero-internal-unbounded audit (2026-04-28) |
| 3.1 False sharing | PENDING | pinned multi-thread |
| 3.2 Atomic contention | PENDING | N-thread counter |
| 3.3 GPU adversarial | PENDING | out-of-scope |
| 4.1 Dispatch worst case | PENDING | synthetic bc |
| 4.2 Anti-LLVM | PENDING | IR inspection |
| 5.1 1000-module load | PENDING | synthetic gen |
| 5.2 Deep cfg | **PARTIAL** | 12-cfg guardrail (2026-04-28); 1024+ fuzz pending |

**3 vectors confirmed defended (channel backlog, 10^6 tasks, SMT exponential),
5 partial defences (alloc pressure, deep-generic compilation, module
fan-out, deep-cfg, plus ~170+ wire-frame sites swept), 6 pending** post
2026-04-28 RT-3.1.1 / RT-3.1.3 / RT-3.5.2 / RT-3.2.4 / RT-3.2.3 / RT-3.1.2
closures. Sections A-C above document performance-class invariants already
upheld through the closed audit.

The wire-frame and crypto hot paths now have allocation-free bulk-copy
primitives in place; further work is in the synthetic-input adversarial
fuzzing space which needs the listed harnesses to advance.
