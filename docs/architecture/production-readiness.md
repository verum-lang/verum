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
| P1 | The same source, the same binary, gives the same verdict | determinism harness | **no** — T0927 (checker gives 0 or 20 errors on one file), T0953 (a stale cache reports 0 errors on a file with 9). Related and now gated: the same file checked as a FILE and as a PROJECT had to agree — `check_file_vs_project_parity.py`, three programs, both ways |
| P2 | A failure is loud: no phase answers "fine" by degrading | degradation ratchet | partial — T0747 names the class; ratchet drafted, not in CI (T0857) |
| P3 | The two tiers agree: interpreter ≡ AOT | cross-tier suite | partial — `core-tests` runs both, but AOT lane is non-blocking (`nightly-aot.yml`) |
| P4 | The stdlib a user gets is the stdlib we test | bake-vs-source parity | partial — T0692 (one driver), T0755 (535 of 2 561 `core/` files fail `check` while the bake is green) |
| P5 | Every public stdlib name is reachable by a documented spelling | mount/reexport suite | **yes for contexts, partial overall** — the glob asks the same authority a named mount asks (T0969); contexts joined the export surface as a fourth family, `core.context.standard` going from 6 exported items to 22, and the project path now registers them too (T0974, closed). Gated by `check_file_vs_project_parity.py` |
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

### A defect count going UP can mean the compiler reached further (2026-08-31)

The registry project measured 461 diagnostics in the morning and 485
after a day of compiler repairs. The first reading is a regression. It
is not, and the measurements that settle it are worth the pattern:

| | morning | after |
|---|---:|---:|
| total | 461 | 485 |
| files reporting | 63 | 64 |
| files whose count DECREASED | — | 0 |
| parse errors | 2 | 0 |
| E401 "cannot find X in module Y" | 25 | 47 |

Nothing decreased, coverage barely moved, and the increase is +1 or +2
spread across services that mount many stdlib types. The parse errors
went to zero because the contract-clause repair let a file reach the
type checker for the first time.

Then the deciding check — are the NEW diagnostics true? Taking one:

    registry:  mount architecture.types.Severity;
    core/:     public type Severity is …   in architecture/anti_patterns.vr
                                           and in cli/verify.vr
               core/architecture/types.vr declares 40 public types,
               and Severity is not among them

The mount names a module that does not declare the type. A real defect,
previously masked, now reported.

So the count rose because the compiler stopped bailing early. **A defect
count is a measurement of the MEASURING APPARATUS as much as of the
code**, and a repair to the apparatus moves the number in the direction
that looks like damage. The honest report is the pair: 461 -> 485, of
which 0 files got worse and 2 files started parsing.

The rule this adds to the one below it: state the units, AND state
whether the instrument changed between the two readings.

### An error count is in messages, not causes (2026-08-30)

The registry project checks in 34 s and reports **461 errors**. That
number is in the wrong units for every purpose it gets used for.

    25  E401 "cannot find X in module Y"      causes
    24  E100 "unbound variable: X"            echoes of those causes
    19  of those 24 are ONE name, format_semver
    8   names the registry imports that the stdlib does not have

So a quarter of the "unbound variable" diagnostics are the compiler
repeating something it already said, and the single most frequent
message in the project is not the most frequent cause — it is the most
duplicated one. A 461-line report is read top-down and abandoned, so
the echoes displace the genuine diagnostics rather than merely padding
them (T0990).

The same unit error appears in the conformance suite: "265 unproved
obligations" counts obligations, and one unresolved entity can fail
many. Any burndown plan built on either figure is planning against
message counts.

Rule: a defect count states its units. "461 diagnostics; causes fewer,
not yet counted" is a usable sentence. "461 errors" is not.

### Refinement types worked on scalars only (2026-08-30)

The headline feature, measured on the shape every domain model uses:

    type NonNeg is n: Int where n >= 0;

    pure fn triple(a: NonNeg) -> Int ensures result >= 0 { a * 3 }   PROVED
    type Box is { v: NonNeg };
    pure fn get(b: Box) -> Int ensures result >= 0 { b.v }           FAILED

A refinement on a PARAMETER became a hypothesis. The same refinement
reached through a FIELD did not, so `b.v` handed the solver an
unconstrained `Int`. The counterexample came back EMPTY — nothing to
show, because the obligation was never constrained.

The other side of the obligation had the mirror gap: a postcondition
naming a field of the RESULT could not be discharged either, and that
one was not about refinements at all —

    type Box is { v: Int };                     <- no refinement anywhere
    pure fn bump(b: Box) -> Box ensures result.v > b.v { Box { v: b.v + 1 } }
        FAILED

because `result` is bound by translating the body at Int, Bool or Real
sort, and a record literal is none of those.

So until today the argument "the invariant lives in the type and travels
with it" held for `Int` and not for `{ x: Int }`. Both halves are fixed
(T0994, T0995) and the registry showcase now discharges every proof it
states — 6 of 6, from 4 of 6.

WHAT THIS SAYS ABOUT THE MEASURING APPARATUS, which is what §0 is about:
neither gap was a regression. Dated across four binaries, the oldest
three weeks old, the showcase reported 4 proved / 2 failed on every one.
A conformance suite of 7 036 specs did not contain the four-line program
that exposes it, and the website guide asserted the opposite in prose.

### An error code can be known as the WRONG thing (2026-08-30)

`verum explain` is where a user goes with a code they just saw. Measured:

    error<E0203>: Result type mismatch in '?' operator   <- printed
    $ verum explain E203                                 <- the zero dropped
      module not found                                   <- a different defect

Two spellings are live — `Exxx` in the error registry, `E0xxx` in the
diagnostics explanations and the compiler's lints — and where the digits
coincide the meanings do not: `E0101` use-after-free against `E101`
undefined type, `E0313` integer overflow against `E313` dangling
reference. 51 four-digit codes, 41 with a colliding twin.

A coverage gate that asks "is every printed code in the registry?" is
GREEN on all 41, because both codes are registered — as different
things. That is the same shape as a spec that measures and demands the
wrong answer: the check runs, and its subject is not the question.
Ratcheted at 41 (T0996); one instance fixed outright (T0992).

### The verifier answered a question wrongly (2026-08-30)

Everything else in this document is about a check that is missing, weak,
or asleep. This entry is different in kind: a check that RAN, ANSWERED,
and was WRONG.

    static mut COUNTER: Int = 0;
    fn tick() -> Int { COUNTER = COUNTER + 1; COUNTER }
    fn drift(n: Int) -> Int ensures result == 0 {
        let a = tick();  let b = tick();  a - b
    }

`✓ drift: Proved`. The interpreter prints `-1`. Control against a
vacuous check: the same file with `ensures result == 999` reports
Failed, so the obligation is live.

A postcondition is discharged by asserting `result == body`, and a call
renders as `(name args…)` keyed on the name alone, so two `tick()` calls
are one term and congruence gives 0. The identification is CORRECT for a
pure function — which is why the pure twin still proves, and why the
variant with different arguments correctly fails.

Three layers, and the bottom one is not in the solver:

| layer | what it does | what it should do |
|---|---|---|
| property inferrer | a `static mut` is not state, so `pure fn tick()` is ACCEPTED | a read is `ReadsExternal`, a write is `Mutates` |
| verifier | asserts `result == body` unasked | ask whether the body is a function of the arguments |
| purity gate | `is_reflectable` exists; 4 callers, all its own unit tests | be asked |

Tracked as T0982, pinned by a PAIR of specs under
`vcs/specs/L0-critical/verification/` — the impure case that must fail
and the pure case that must keep proving, because the blunt repair
(never identify two calls) turns the first green and the second red.

### Purity is decided by declaration order (2026-08-30)

The same three-line program, twice, differing only in which declaration
comes first:

| program | order | verdict |
|---|---|---|
| `pure fn caller() { helper() }` after `fn helper() { print("x"); 1 }` | callee above | E503 refused |
| the same two lines swapped | callee BELOW | **accepted** |
| two hops, both above | callee above | E503 refused |

Properties are registered as the walker goes, in source order, and the
`Call` arm treats a callee it cannot find as contributing nothing. So a
forward reference reads as pure. The module walk already registers
function SIGNATURES in a phase of its own so forward references resolve;
properties never got that phase. T0985.

### A spec that certifies the gap as intended (2026-08-30)

`vcs/specs/L1-core/types/pure/pure_function_validation.vr` lists in its
own header what it tests:

    // 2. Pure function calling IO function (FAIL)
    // 3. Pure function with mutable references (FAIL)
    // 4. Pure function accessing global state (FAIL)

Twelve functions named `impure_read_global`, `impure_mutate`,
`impure_random` … each marked `pure`, each written to BE REFUSED — under
a file directive of `@test: typecheck-pass`, which requires the file to
type-check cleanly.

Measured: the compiler refuses **three** of the twelve, all three for
IO. Nine are accepted. So a spec named `pure_function_validation`
requires that three quarters of the purity violations it documents be
ACCEPTED, and while it is green the gap reads as intended behaviour.

This is a distinct failure mode from the ones in §0. There the apparatus
measures nothing; here it measures, and demands the wrong answer.

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
