# Production readiness — what is measured, and what is only assumed

Status: LIVING DOCUMENT. Companion to
`docs/architecture/tech-debt-register.md`, not a duplicate of it. The
register answers *what is broken*; this answers *what would have to be
true for the language to be shippable, and how much of that is
currently measured rather than assumed*.

Every number here carries the date it was measured and the command that
produced it. A row without a measurement says so — an unmeasured row is
a claim, and this document does not make claims.

First census: 2026-08-30.

---

## 0. The finding that governs the rest

**The project's measuring apparatus mostly does not measure results.**
Two independent corpora, measured the same day, agree:

| corpus | total | checks a RESULT | checks only that it did not fail |
|---|---:|---:|---:|
| `vcs/specs/**/*.vr` (conformance) | 7 036 | ~1 049 (15%) | ~5 803 (82%) |
| `crates/*/tests/*.rs` (`#[test]`) | 18 083 | 9 249 (51%) | 8 834 (49%) |

Sharpest inside the conformance suite — of the 2 629 specs that
actually RUN a program:

| what the spec compares | count | share |
|---|---:|---:|
| its stdout, against expected text | 179 | 6.8% |
| a non-zero exit code | 1 | 0.0% |
| a specific error | 1 | 0.0% |
| exit code `0` — i.e. "it did not crash" | 1 627 | 61.9% |
| nothing at all | 821 | 31.2% |

2 355 of the 2 356 `@expected-exit` directives in the tree are `0`.

The consequence is not that the suite is worthless — a crash-free run
of 2 629 programs is real signal. The consequence is that **"X works"
in this project usually means "X does not crash"**, and every readiness
claim built on the suite inherits that weaker meaning.

Three defects found in one day are the same shape, in three unrelated
subsystems:

* 96 of 99 tests in `verum_fast_parser/tests/precedence_tests.rs`
  called `assert_parses`, which checks that a string parses and not how
  it groups. `test_bitwise_and_before_or` passed whichever way `&` and
  `|` bound, and the authoritative grammar had drifted to a reading
  that gives `a | b & c` a different VALUE than the compiler (T0816).
* 164 verification specs ran no verifier at all, and `verify-fail`
  returned `Pass` unconditionally (T0957).
* The conformance runner executed a different pipeline than
  `verum run` — no safety gate, type check, verify or CBGR analysis —
  so `verum run` printed `-140734938218496` on a spec the runner
  reported as PASS (T0732; fix on main at `4802c2acb`, one divergence
  left on the ratchet).

Reproduce both halves with one command:

```sh
scripts/ci/census_result_free_tests.py
```

**Readiness item R-0: a spec whose name states a property must compare
that property.** Until the ratio moves, every other row in this
document is bounded above by it.

---

## 1. What "production ready" has to mean here

Seven properties. Each is falsifiable, each names its instrument, and
each says whether that instrument currently exists.

| # | Property | Instrument | Exists? |
|---|---|---|---|
| P1 | The same source, the same binary, gives the same verdict | determinism harness | **no** — T0927 (checker gives 0 or 20 errors on one file), T0953 (a stale cache reports 0 errors on a file with 9) |
| P2 | A failure is loud: no phase answers "fine" by degrading | degradation ratchet | partial — T0747 names the class; ratchet drafted, not in CI (T0857) |
| P3 | The two tiers agree: interpreter ≡ AOT | cross-tier suite | partial — `core-tests` runs both, but AOT lane is non-blocking (`nightly-aot.yml`) |
| P4 | The stdlib a user gets is the stdlib we test | bake-vs-source parity | partial — T0692 (one driver), T0755 (535 of 2 561 `core/` files fail `check` while the bake is green) |
| P5 | Every public stdlib name is reachable by a documented spelling | mount/reexport suite | **partial** — the glob now asks the same authority a named mount asks (T0969, landed); contexts joined the export surface as a fourth family, `core.context.standard` going from 6 exported items to 22 (T0974). Open: the checker's context table is filled on the single-file path and not the project path |
| P6 | The published specification predicts what the compiler does | grammar↔parser gate | **partial** — `operator_ladder_tests.rs` pins the ladder against the parser (T0816, new). `check_grammar_docs_match.py` compares the site's EBNF to the authority, but takes the "no documentation tree" branch on every CI run, because the website lives in a separate, gitignored repository (T0971) |
| P7 | The tools survive a working day | LSP/CLI soak | **no** — T0752 (`verum lsp` reaches 36.6 GB over 13.5 h), T0746 |

---

## 2. Blockers ordered by what an outsider hits first

Not by severity label — by the order in which someone who has never
seen this repository would run into them.

**Step 1 — install and run one file.** `verum run hello.vr` works and
`verum build` emits a Mach-O linking only libSystem, so the no-libc
invariant holds on darwin — reported by the peer session 2026-08-30,
NOT re-measured here; the same run printed `Call fn-id … missing in the
VBC module table` for `default_panic_handler`, so a call degraded while
the binary still built. Startup is 137 ms for a one-line
script against bun's 22 ms, 89 ms of it after process start with a warm
cache (T0917), and the stdlib expands whole regardless of the program:
713 MB and 0.2 s before the user's file is parsed (T0745).

**Step 2 — import something.** `mount X.*` can expand to NOTHING,
silently. A module can hold two registry entries — an empty prefixed
stub and the real one:

    Module 'core.intrinsics.control':  0 exports   <- stub
    Module 'intrinsics.control':      20 exports   <- real

The glob resolves the path to the prefixed form, lands on the stub and
imports nothing; `mount X.item` by name works, because it reads the
metadata rather than the registry. Two records of one intent, two
authorities, diverging. The stub is minted for the modules the stdlib
ITSELF depends on — 90 in the measured run, among intrinsics: `atomic`,
`control`, `runtime`, `runtime.async_ops` and `intrinsics` itself,
while `bitwise`, `float`, `arithmetic`, `memory`, `platform`, `simd`,
`tensor`, `gpu` and `lowlevel` have no prefixed entry and so work. The
rule an outsider meets is perverse: **`mount X.*` fails more reliably
the more central X is** (T0969, peer session 2026-08-30; radius being
re-measured, see §3).

Adjacent and open: `mount M;` then `M.free_fn(...)` does not resolve
(T0797, P0); a file-local free function loses to a same-named stdlib
method (T0798, P0); `mount self.X` resolves to the parent (T0775).

**Step 3 — write a type and trust the checker.** The checker is
non-deterministic on at least one file (T0927, P0). A stale on-disk
cache makes it report 0 errors on a file that has 9 (T0953, P0). The
interior of a type declaration is unchecked — a name that errors in a
`let` is accepted as a field type (T0811/T0924). `Pair<Int>` and
`Pair<Int,Text,Bool>` both check clean against `type Pair<A,B>`
(T0922). `let x: Int8 = 300` is accepted (T0923).

**Step 4 — rely on a protocol.** An `implement` block missing a
required method type-checked and panicked at run time; the fix is on
main (`cc915565d`) with T0812 still open in the pool.
Stdlib protocol default methods are not inheritable through the
archive, so a user type implementing `Iterator` cannot call `fold`
(T0952, P0).

**Step 5 — ask for a proof.** The verify spine was rebuilt in the last
week (T0954, T0957, T0964–T0967). First honest census after the specs
started running a verifier: 46 of 68 red in `L3-extended/proofs`, 294
undischarged theorems; 30 of 201 red in L0/L1 (T0960). Any green
verification result older than 2026-08-30 is not evidence.

**Step 6 — keep the editor open.** `verum lsp` reaches 36.6 GB and
burns 86% of a core for 13.5 hours (T0752, P0).

---

## 3. What we do not know

Stated explicitly, because an unmeasured area reads as a healthy one.

* **No current full-suite number.** The blocking L0 gate has not
  produced a verdict in at least a week — `vtest` dies with a glibc
  heap corruption mid-run (T0829, P0). `core-tests/INVENTORY.md` rows
  are STALE-GREEN by construction and the liveness gate that would
  catch that is itself open (T0220).
* **No AOT number that gates.** The AOT-heavy crates run in a
  non-blocking nightly lane; a red there blocks nothing.
* **No measured startup/throughput baseline** against the targets in
  `CLAUDE.md` (>50K LOC/s compile, 1× native C runtime) on current
  main. The parse-speed contract exists; the end-to-end one does not.
* **No soak test** for any long-running tool, which is why T0752 was
  found by a user's editor session rather than by CI.
* **The radius of T0969 is not settled.** A first census put it at
  1 889 of 2 560 `core/` files and 18 199 public declarations; it was
  taken while the disk stood at 100% and did not survive re-measurement
  on the same files with the same binaries — three of five reproducers
  disappeared once space was freed, including a file that had answered
  `unbound variable: sin` an hour earlier.

### One name, four defects (2026-08-30)

Four separate rows in this pool are one mechanism: a global bucket
keyed by a function's SIMPLE name, where the winner is decided by
registration order and the loser is DISCARDED rather than shadowed.

| task | what the user sees |
|---|---|
| T0946 | a local `resolve` prints `None`; eight stdlib `resolve`s share the key, and formatting takes the winner's return type |
| T0931 | `interval(a, b)` reports "accepts at most 1 argument" — the arity of a different `interval` |
| T0883 | `field 'capabilities' not found on type 'Session'`, listing another `Session`'s members |
| T0979 | `verum run a.vr` executes `b.vr`'s `main` |

Established with `VERUM_TRACE_FNREG`, which prints every registration
and the entry it displaces:

```
[fnreg] 'resolve' arity=2 rt=Some("Maybe<JsonValue>")  prev=None
[fnreg] 'resolve' arity=2 rt=Some("Maybe<Text>")       prev=Some(...)
```

The obvious remedy was tested and REFUSED: adding a `module` header to
the user's file changes T0979 (the silent pick becomes an honest
refusal) and changes nothing for T0946. So the root is not a missing
qualifier — it is that consumers ask for the simple name while the
qualified one is already in hand. That makes the fix the same shape as
two that landed today: teach the consumer to ask the authority, as
T0973 did for error codes (the gate asks the ENUMERATION, not a
spelling) and T0969 did for glob mounts.

### The source-only gate wall is RED on main (2026-08-30)

`make gates-source` is the list of gates CI runs that need no build.
Run today, five of them fail, and the target stops at the first — so
the ones after it did not run at all:

| gate | state |
|---|---|
| `check-vr-syntax` | FAIL — one Rust `::` in a spec comment naming a Rust type (`TypeKind::Reference`), landed today in `768de7cca` |
| `check-internal-refs` | FAIL — three references to the gitignored `internal/` tree, all mine, fixed in this commit |
| `check-rings` | FAIL — 4 upward edges across the `core/` dependency rings (e.g. `base.env(r1.0) -> text.format(r2.0)`) |
| `check-panic-surface` | FAIL — 665 unwrap/expect sites under `verum_codegen/src/llvm/` against a baseline of 664 |
| `check-dup-emitters` | FAIL — an emitter body references libc `getenv` with no Linux syscall leg (T0436 class) |

Two of the five are ratchets that moved by ONE (`panic-surface`) or
name a known class (`dup-emitters`), which is what a ratchet is for.
The point is not the individual rows: it is that the wall as a whole
does not currently hold, and `make` stopping at the first failure means
a green run of the later gates had never been observed on this state.

`make gates-source-report` now runs all 21 past the first failure and
prints a line per gate — the wall reported one failure and had four.

### What the registry proving-ground produced in one session

The directive is that a stumble in the registry is a defect in the
LANGUAGE, fixed in the language rather than worked around in `.vr`.
Measured against that, the first session of use produced:

| found by | outcome |
|---|---|
| writing a version type | postcondition on a record field is undischargeable (T0975) |
| the same type's refinement | a refinement on a FIELD is not a hypothesis (T0976) |
| time as a context | a context used in the tail expression warned as unused (T0977, fixed) |
| a context from the stdlib | contexts were absent from every module's export surface (T0974) |
| three example files | `verum run a.vr` executes another file's `main` (T0979) |
| a three-tier reference | `&unsafe T` loses the receiver's type and guesses the field (T0978) |
| an `async fn` | produces a value, not a Future — `block_on` panics on an `Int` (T0734, pre-existing, sharpened) |
| a recursive tree | `Heap(x)`, the form CLAUDE.md documents, is refused (T0944) |

Eight, of which two are closed. None of them was worked around in the
registry's own source; each is filed with a repro small enough to run
in seconds.

Two were resolved by REMOVING a claim rather than fixing code: T0825
and T0890 were open P1 defects that no longer reproduce — a
measurement, not a fix. Stale RED costs as much as stale green: it is
work someone plans that does not exist.

### A measurement rule this session paid for

**A full disk in this repository wears other defects' symptoms.** It
has already impersonated a serialisation fault ("embeds stdlib
typecheck metadata that failed to decode") and, the same day, a
name-resolution defect. Any number taken while the volume is at 100% is
suspect and must be re-measured after freeing space — including the
ones in this document. The numbers in §0 are computed by reading files
and are unaffected; anything produced by BUILDING or RUNNING under a
full disk is not.

---

## 4. How this document is kept honest

Add a row only with the command that produced its number and the date.
When a row's instrument does not exist, say **no** in the "Exists?"
column rather than describing the intent — an intended gate and a
working one are indistinguishable in prose, which is the failure this
whole document is about.
