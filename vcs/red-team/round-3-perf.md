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

### 1.1 Nested generic instantiation chain

**Status:** PENDING — needs synthetic deep-generic generator.

`List<List<List<...List<Int>>>>` 1024 deep. Risk: monomorphization expands
exponentially or compile-time hits a stack-overflow from recursive type
construction.

### 1.2 Refinement predicates with exponential SMT cost

**Status:** PENDING — depends on round-1 vector 5.1 (Z3 timeout policy).

Deep nested ANDs over many free variables. Z3 nominally handles QF_LIA in
PSPACE but pathological predicates can hit timeout. Verify the timeout
policy is fail-closed (round 1) AND that timeout itself doesn't leak
attacker-controllable wall-clock.

### 1.3 Module dependency graph fan-out / fan-in

**Status:** PENDING — needs synthetic dependency generator.

Worst case: 1000 modules each mounting `core.*`. Verify closure walker
remains linear (current observation).

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

### 2.3 10^6 lightweight async tasks

**Status:** PARTIAL — `core/async/executor.vr` has explicit `task_count`
limit. spawn() panics on limit reach (architectural choice; see
round-1 weakness on graceful backpressure). Fast-channel + tokio-style
work-stealing not yet exhaustively profiled.

### 2.4 Channel unbounded backlog

**Status:** PARTIAL DEFENSE — bounded channel APIs exist; unbounded uses
need explicit memory-cap per-channel. Audit recipe: grep `channel.unbounded`
and verify each use-site has a logical backpressure mechanism upstream.

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

### 4.1 Long single-instruction basic-block chain

**Status:** PENDING — needs synthetic bytecode generator.

Designed to maximize interpreter dispatch overhead. Compare Tier-0 dispatch
cost against AOT-compiled equivalent.

### 4.2 Anti-CSE / anti-LICM AOT

**Status:** PENDING — needs LLVM-ir inspection harness.

---

## Vector 5 — Stdlib-loading scaling

### 5.1 1000 modules mounting core.*

**Status:** PENDING — needs synthetic-module generator.

### 5.2 Deeply nested @cfg conditional

**Status:** PENDING — needs cfg-overlap stress test.

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
| 1.1 Deep generic | PENDING | synthetic gen |
| 1.2 SMT exponential | PENDING | depends on 5.1 |
| 1.3 Module fan-out | PENDING | dependency gen |
| 2.1 Alloc pressure | PARTIAL | wire-frame done |
| 2.2 Refinement caching | PENDING | hot-loop test |
| 2.3 10^6 tasks | PARTIAL | scheduler stress |
| 2.4 Channel backlog | PARTIAL | per-channel cap audit |
| 3.1 False sharing | PENDING | pinned multi-thread |
| 3.2 Atomic contention | PENDING | N-thread counter |
| 3.3 GPU adversarial | PENDING | out-of-scope |
| 4.1 Dispatch worst case | PENDING | synthetic bc |
| 4.2 Anti-LLVM | PENDING | IR inspection |
| 5.1 1000-module load | PENDING | synthetic gen |
| 5.2 Deep cfg | PENDING | cfg stress |

**4 partial defences (alloc pressure, task scheduler, channel backlog, plus
~170+ wire-frame sites swept), 10 pending.** Sections A-C above document
performance-class invariants already upheld through the closed audit.

The wire-frame and crypto hot paths now have allocation-free bulk-copy
primitives in place; further work is in the synthetic-input adversarial
fuzzing space which needs the listed harnesses to advance.
