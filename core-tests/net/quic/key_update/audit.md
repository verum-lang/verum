# `net/quic/key_update` audit

Module: `core/net/quic/key_update.vr` — RFC 9001 §6 key update:
`confidentiality_limit` / `integrity_limit` (§6.6 AEAD usage limits),
`KeyUpdateSm` state machine, `KeyPhaseUsage`, `KeyUpdateError`,
`InboundPhaseAction`.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.connection_sm` | drives 1-RTT key rotation. |
| `core.net.quic.crypto` | AEAD usage accounting per key phase. |
| `core.net.tls13.cipher_suite` | AeadKind selects the limit. |

## 2. Crate-side hardcodes

The §6.6 AEAD usage limits are RFC 9001 Table 1/3 verbatim:
confidentiality — AES-GCM/CCM 2^23, ChaCha20-Poly1305 2^64-1, AES-CCM-8
2^21; integrity — AES-GCM 2^52, ChaCha20 2^36, AES-CCM 2^21. Each pinned.
These are crypto-safety bounds: exceeding them mandates a key update /
key discard, so drift is a security regression.

## 3. Language-implementation findings

None for the covered surface. The limit functions are pure `match` over
`AeadKind` (unit variants — safe inline). `KeyUpdateSm` (`&mut self` phase
FSM) is gated on the MUTSELF-MATCH-1 bind-event discipline + deeper API
coverage.

## 4. Action items landed in this branch

* `unit_test.vr` — 9 §6.6 limit pins (confidentiality + integrity across the
  AEAD set) + the conf-≤-integrity safety relationship.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| KeyUpdateSm phase-transition coverage (bind-event discipline) | this folder | 2h |
| KeyUpdateError + InboundPhaseAction ADT coverage | this folder | 1h |
| should_initiate_update / exceeded_*_limit predicates | this folder | 1h |
