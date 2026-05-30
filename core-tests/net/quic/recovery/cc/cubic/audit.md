# `net/quic/recovery/cc/cubic` audit

Module: `core/net/quic/recovery/cc/cubic.vr` — RFC 9438 CUBIC congestion
control: scaling/decrease/fast-convergence constants + the CUBIC window
function.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.net.quic.recovery.cc.mod` | pluggable congestion controller (default). |

## 2. Crate-side hardcodes

CUBIC_C=0.4 (§4.1 scaling), CUBIC_BETA=0.7 (§4.6 multiplicative decrease),
CUBIC_FAST_CONV_FACTOR=0.85 (§4.7 = (1+β)/2) are RFC 9438 verbatim, with the
β < fast-conv < 1 invariant pinned.

## 3. Language-implementation findings

### §3.1 FLOATCONST-CMP-1 — `<` between two no-suffix `Float` consts mis-compares (CLOSED 2026-05-30)

`CUBIC_BETA < CUBIC_FAST_CONV_FACTOR` (0.7 < 0.85) returns **false**, even
though `assert_eq(CUBIC_BETA, 0.7_f64)` and `assert_eq(CUBIC_FAST_CONV_FACTOR,
0.85_f64)` both pass and `CUBIC_FAST_CONV_FACTOR < 1.0_f64` (const < literal)
passes. The consts are declared **without a `_f64` suffix**
(`public const CUBIC_BETA: Float = 0.7;`), unlike `cc/bbr`'s `_f64`-suffixed
gains, whose const-vs-const `>`/`<` compares are GREEN. So the ordered compare
between two no-suffix-declared Float consts is miscompiled (the value compares
equal via `==` but mis-orders via `<`), suggesting the no-suffix const carries
a representation the `LtF`/`CmpF` opcode reads differently than `EqF`.

**Status:** OPEN, tracked (**FLOATCONST-CMP-1**). The 3 RFC 9438 constant pins
(`==`) + β<1 + fast-conv<1 are GREEN; the const-vs-const ordering test is
`@ignore`'d. Fix surface: VBC codegen for no-suffix `Float`-typed `public
const` must store the canonical f64 NaN-box so `LtF` reads it identically to
`EqF`. **Workaround discipline:** declare Float consts with the `_f64` suffix.

The CUBIC window function (`W_cubic(t)` cube arithmetic) + `&mut self`
controller state are deferred (Duration / bind-event).

## 4. Action items landed in this branch

* `unit_test.vr` — 3 RFC 9438 constants + decrease/convergence invariants.

## 5. Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| W_cubic(t) window-growth function | this folder | gated on Duration/time |
| CongestionControl on-ack / on-loss transitions | cc/mod folder | bind-event |
