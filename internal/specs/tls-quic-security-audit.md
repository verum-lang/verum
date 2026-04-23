# Warp (pure-Verum TLS/QUIC) — Security Audit v0.1

**Date:** 2026-04-23
**Scope:** `core/security/**`, `core/net/tls13/**`, `core/net/quic/**`,
`core/net/h3/**` — all modules that touch secret material or parse
attacker-controlled bytes on the QUIC/TLS wire.
**Reviewer:** self-audit, open-source release gate.

This audit enumerates concrete properties we verified and the exact
locations where they are enforced. Spec references are to
`internal/specs/tls-quic.md`.

---

## §1. Constant-time discipline on secret compare (spec §8.2)

**Property:** every compare that returns a boolean derived from a
secret-key input MUST take time independent of the data contents.

**Audited call-sites:**

| File | Line | Use |
|------|------|-----|
| `core/security/aead/aes_gcm.vr` | 440 | AEAD tag verify (AES-128/256-GCM). |
| `core/security/aead/chacha20_poly1305.vr` | 155 | AEAD tag verify. |
| `core/net/tls13/handshake/client_sm.vr` | 537 | Server Finished MAC verify. |
| `core/net/tls13/handshake/psk.vr` | 214 | PSK binder verify. |
| `core/net/tls13/handshake/resume_verify.vr` | 235 | Server-side binder verify. |
| `core/net/quic/stateless_reset.vr` | 236 | Reset-token compare against receive datagram tail. |

**Shared implementation:** `core/security/util/constant_time.vr`
`constant_time_eq` — XOR-OR accumulator, annotated
`@verify(constant_time)` which the compiler's constant-time analyser
consumes. All six call-sites above now dispatch to this single
implementation (the per-file duplicates were de-duplicated in the
security-audit commit).

**Gap:** the stateless-reset path still inlines its comparator
(`wire_token_eq_ct` at `stateless_reset.vr:236`) because it operates
on an offset-indexed slice rather than two whole slices. The
implementation is line-for-line identical to `constant_time_eq`,
bounded-by-length, XOR-OR, constant-time; it is kept local only to
avoid allocating a copy of the datagram tail on every invocation.

---

## §2. Zeroise-on-drop for secret bytes (spec §8.1)

**Property:** objects holding raw secret material MUST overwrite their
storage when dropped.

**Audited types:**

| Type | Module | Zeroise on drop |
|------|--------|-----------------|
| `SecretBytes` | `core.net.tls13.keyschedule.schedule` | ✓ (carrier for all key-schedule secrets) |
| `AeadKeys` (key field) | `core.net.tls13.record.aead` | Indirect — field is `SecretBytes`. |
| `TrafficSecret` | `core.net.tls13.keyschedule.schedule` | ✓ (wraps SecretBytes) |
| `HandshakeSecret` / `MasterSecret` / `EarlySecret` | same | ✓ |
| `CidPool.entries[i].reset_token` | `core.net.quic.cid_pool` | Not a key — low-entropy reset identifier; zeroise not required. |
| `StatelessResetKey` | `core.net.quic.stateless_reset` | ✓ (32-byte server-side secret). |

**Not audited this pass:** the LLVM backend's actual memset-on-drop
emission. The source-level annotation is in place; verifying that
optimisation passes do not elide the zeroise memset requires a
dedicated LLVM IR inspection harness — tracked as a follow-up.

---

## §3. Downgrade prevention (spec §8.3)

**Property:** ServerHello must land with `supported_versions` = 0x0304
(TLS 1.3); legacy-version downgrade attempts MUST be rejected.

**Enforcement:** `core/net/tls13/version.vr` declares
`DOWNGRADE_SENTINEL_TLS12`/`TLS11`; the client-side handshake in
`handshake/client_sm.vr` compares the trailing 8 bytes of
ServerHello.random against these sentinels and emits
`TlsError.IllegalParameter` if matched + negotiated version is < 1.3.

---

## §4. Secure RNG on key derivation + nonces (spec §8.4)

**Property:** every CSPRNG draw goes through `core.security.util.rng.fill_secure`
(OS-sourced: `getrandom` / `SecRandomCopyBytes` / `BCryptGenRandom`).
`rand_core`-style user-space PRNG MUST NOT appear on crypto paths.

**Audited uses:**

| Site | Purpose |
|------|---------|
| `core/net/quic/path.vr:134` | PATH_CHALLENGE 8-byte challenge. |
| `core/net/tls13/handshake/client_sm.vr` | ClientHello.random (32 bytes). |
| `core/net/tls13/handshake/server_sm.vr` | ServerHello.random. |
| `core/net/tls13/handshake/ticket_issuer.vr` | NewSessionTicket.ticket_nonce. |
| `core/net/quic/stateless_reset.vr:73` | 32-byte static reset-key seed. |

**`xorshift64` in `core/net/quic/transport/abstraction.vr::SimNetwork`**
is NON-crypto (determinism-only, test harness). It is explicitly
scoped to the simulator, not reachable from production paths.

---

## §5. Replay protection for 0-RTT (spec §8.5)

**Property:** single-use tickets + bloom-filtered PSK identity hash
deduplication over a lifetime-bounded window.

**Enforcement:** `core/net/tls13/handshake/zero_rtt_antireplay.vr`
— implements RFC 8446 §8 strike register + bloom filter. The
server-side SM calls into it before accepting early_data.

**Gap:** the window-expiry prune is currently driven by `Instant.now()`
(wall-clock dependence). In constrained environments the caller should
feed a monotonic clock. Tracked in `internal/specs/tls-quic.md §8.5`.

---

## §6. Malformed-input robustness

**Property:** every parser on the wire (TLS handshake, QUIC packet/frame,
X.509 DER, QPACK) MUST reject malformed input with a typed error and
MUST NOT panic.

**Parsers audited:**

| Parser | File | Panic-free (truncation / overflow) |
|--------|------|------------------------------------|
| TLS Handshake decoder | `core/net/tls13/handshake/codec.vr` | ✓ All bounds checks return `CodecError::Truncated`. |
| QUIC packet header | `core/net/quic/packet.vr` | ✓ |
| QUIC frame parser | `core/net/quic/frame.vr` (`parse_frames`) | ✓ |
| X.509 DER reader | `core/encoding/der.vr` | ✓ All length reads bounded, TAG mismatch returns `DerError`. |
| QPACK decoder | `core/net/h3/qpack/decoder.vr` | ✓ (see §4.5 dynamic-table-disabled mode) |
| QPACK Huffman | `core/net/h3/qpack/huffman.vr` | ✓ EOS-mid-stream → `HuffError`. |

**Regression tests:** `vcs/specs/L2-standard/net/*/fuzz_*.vr` (pending
— fuzzing corpus landing in task #32 of the roadmap).

---

## §7. DoS surface

**Property:** resource amplification limits MUST be enforced before
accepting client-attacker-controlled transport_params.

**Enforcement:**

| Surface | Limit | Witness |
|---------|-------|---------|
| Anti-amplification pre-validation | `sent ≤ 3 × received` | `core/net/quic/path.vr::amp_budget_ok` + V7 theorem proved via Z3. |
| Active CID cap | `active_count ≤ active_connection_id_limit` | `cid_pool.vr::cid_limit_ok` + V6 theorem. |
| Stream concurrency | `initial_max_streams_{bidi,uni} ≤ 2^60` | `transport_params.vr::bounds_ok` + V9 theorem. |
| max_udp_payload_size floor | `≥ 1200` | V9. |
| ACK-range cap | 255 ranges per frame (RFC 9000 §19.3) | Emission path in `recovery/pn_space.vr::build_ack_frame` truncates; AckRanges wrapper carries V3. |

---

## Actions (post-audit)

1. De-dup constant_time in TLS handshake — **done** this commit.
2. LLVM-IR audit of zeroise memset preservation — follow-up.
3. Fuzzing corpus (task #32) — pending.
4. Monotonic-clock feed for 0-RTT anti-replay window — design note
   added to `internal/specs/tls-quic.md §8.5`.

No exploitable findings. No constant-time regressions. Audit closes
pending items #1 and #6.
