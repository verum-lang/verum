# The proving ground: how defects in Verum are found

This describes how work on the language is driven, and it is a
description of what worked rather than a proposal. Two campaigns have
now found twenty-odd defects this way; several made whole features inert
without anyone noticing, and two were silent data loss.

## The shape

**Build an ambitious real program in Verum. Every place it bends around
the language is a defect. Fix the language, never the program.**

The program is not a test suite and not a demo. It is application code
with a purpose of its own, written the way its domain wants to be
written — and that is exactly what makes it a detector. A test exercises
what its author already suspected. A real program reaches for whatever
its problem needs, in combinations nobody enumerated.

The current proving ground is the package registry. What it happens to
need — files that mount each other, sum types with typed refusals,
refinements on values the program computes about itself, dependency
injection for ambient facts, structured concurrency for an
all-or-nothing import — is a broad slice of the language, and none of it
was chosen to be a slice.

## Why the standard library does not do this job

`core/` is 2500 files of Verum and finds almost none of these. It is not
a weaker test; it is a DIFFERENT SHAPE, and the difference is
structural:

| The stdlib | An application |
|---|---|
| its directory name matches its module root | a checkout directory rarely matches the cog name |
| no `src/` | code lives under `src/` |
| compiled by the bake, as one unit of many files | compiled as an entry file plus siblings |
| loaded from an archive with metadata | loaded from source, with no metadata entry |
| its own names are the ones that win | its names collide with the stdlib's |

Every row is a defect class that was invisible until an application hit
it. A project whose cog name differed from its directory registered
module paths nobody spells; `src/` became a module segment; a project
type named after a stdlib generic inherited that generic's arity; a
mount naming a sibling lost to a same-named library function.

## The rules

These are not style. Each exists because its violation cost a session.

### 1. The program never works around the language

When the registry could not call its own `resolve` because a
standard-library namesake won, it was renamed — and the rename is
STATED IN PLACE, with the task number, as a workaround and not as a
preference:

> NOT named `resolve`: the standard library has a two-argument `resolve`
> of its own, and an explicit mount of a same-arity namesake currently
> binds to it (filed, with the exact site).

A workaround that reads as a design choice removes the evidence. The
next reader sees a well-named function, not a defect.

### 2. Measure before designing the fix

The productive question is almost never "what is wrong with this code".
It is "what two things disagree, and about what":

* one file, two commands — `verum check <file>` refused what
  `verum check` accepted, which named the CHANNEL rather than the types;
* one program, two spellings — a mounted alias rendered `None` where the
  qualified path rendered `7`, which named the type and not the value;
* one shape, two positions — a refinement fired on a record field and
  not on a variant payload, which named the emission site.

### 3. An inert fix is a measurement, not progress

A change that builds, runs, and moves no number is EVIDENCE — it rules
out the hypothesis that motivated it. Record what it ruled out and
revert it. Keeping it as "partial progress" leaves code nobody can
justify and a hypothesis nobody knows is dead.

Two candidate fixes for the exhaustiveness defect were written, built,
measured inert, and reverted. What they established — that the mounted
type arrives with its full variant body, so the loss is upstream — is
worth more than either would have been.

### 4. Every fix lands with a gate that has a negative pole

A gate that only asserts the fixed behaviour passes for a change that
breaks everything into the same shape. Alongside "a non-generic
declaration records arity 0" sits "a generic declaration still records
its parameters", because a fix that recorded 0 for everything would
satisfy the first and destroy generics.

### 5. Silence is not a measurement

A test that skips and prints `ok` is indistinguishable from one that
ran. A probe that finds nothing may be broken. Build the positive
control INTO the instrument: the dead-variant audit was first run with a
deliberately dead variant added, to prove it could see one.

### 6. The worst defects are declared-but-unenforced

A missing feature announces itself. A feature that is present, accepted
by the compiler, documented, and does nothing does not — and everything
built on it inherits a guarantee that was never made.

Three from one campaign:

* `verum check` on a project reported zero errors for plain type
  mismatches, because it read one of the checker's two error channels;
* a refinement written on a variant payload compiled and was never
  checked, while the same refinement on a record field fired;
* the architectural obligations the protocol spec requires are stated as
  theorems and are all UNPROVED — which is honest only because the
  prover says so rather than stamping them.

Find these by USING the feature and then VIOLATING it. Write the
refinement, then break it deliberately, and see whether anything
happens. A feature nobody has deliberately violated is a feature nobody
has tested.

### 7. File with the reproduction and the controls that narrow it

A task that says "X is broken" costs the next person the whole
investigation again. A task that says "X fails, Y with the same shape
passes, and here is the two-file reproduction" starts them at the
narrowing. Every measurement that RULED SOMETHING OUT belongs in the
task too — a falsified hypothesis is expensive to establish and cheap to
record.

### 8. Before turning silence into an error, enumerate what else wears the syntax

The hardest defects are the ones where the compiler accepts something it
should refuse, and the fix is a new refusal. A new refusal is a
liability: it can reject working code, and rejecting working code is
worse than the hole it closes.

`a.b.f()` was accepted whatever `a.b.f` named. Making it an error meant
answering "is this a module member that does not exist", and three OTHER
things wear exactly that syntax — a chained call whose receiver is
reused by the chain handler, a variant constructor reached through a
module path, a method on a type's associated constant. Each was found by
measuring, none by reasoning, and each would have rejected code in the
standard library.

So the question to ask before writing the refusal is not "is this
wrong". It is **"what else looks like this"** — and the answer comes
from running the candidate refusal across the corpus and reading every
new failure, not from thinking harder.

### 9. A change to a shared rule is measured against a build of the parent commit

"N files failed before, N after" is the only statement about a
resolution or type-system change that means anything, and it costs one
extra build:

```
git stash              # or copy the changed files aside
cargo build            # the parent commit, same target dir
<sweep the corpus>     # core_before.txt
git stash pop
cargo build
<sweep the corpus>     # core_after.txt
```

Then diff by FILENAME, not by count — a change that fixes three files
and breaks three has an unchanged count and is not neutral. Three
attempts at one fix were reverted this way after the sweep showed each
regressing a proof corpus the count alone would have hidden.

The sweep itself must be parallel or it will not be run: 2560 files at
0.6 s each is 26 minutes serially and 6 with `xargs -P 5`. An
instrument nobody runs twice is not an instrument.

### 10. What a run leaves behind is evidence

A file-writing defect was diagnosed for an hour through the interpreter,
the FFI marshaller and three intercepts. The answer had been sitting in
the first directory listing of the session:

```
<ptr@0x9d4831cf8>
<ptr@0xb74598df8>
```

Two files, one byte each, named after a pointer — which is exactly what
`write(&record.field, …)` had been doing: rendering the field's slot
ADDRESS as text and using the rendering as a filename. The symptom was
"no file appears"; the evidence was "a file appears somewhere else".

Before tracing a "nothing happened" report inward, look at what the run
LEFT BEHIND — new files, a changed working directory, an unexpected
mode, a stray byte on stdout. A defect that produces nothing is rarer
than one that produces something in the wrong place.

### 11. Check the whole project, not the file you are looking at

The proving-ground registry ran for weeks with TEN unreported errors in
it, because the commands anyone actually types do not reach a project's
other modules:

```
verum run src/main.vr      the program runs
verum check src/main.vr    0 errors
verum check                10 errors
```

Three of those ten were language defects nobody had seen — a silently
WRONG type for an `Ok` payload, a complete match refused as
non-exhaustive, a purity violation executed. They were invisible not
because they were subtle but because no command in the working loop
looked at the file they were in.

That the file-targeted commands miss them is itself a defect and is
filed. Until it is fixed, **run the argument-less `verum check` at the
end of every session on the proving ground**, and read what it says
about modules you did not touch. It costs one command and it is the
single highest-yield instrument in this document.

The general form: when a tool has a broad mode and a narrow one, the
narrow one is what gets typed and the broad one is where the findings
are. Ask what the broad mode says before concluding a program is clean.

### 12. Before designing a fix, find the sibling that already does it right

Five roots in one session had the same shape: the rule, the authority or
the registry ALREADY existed and was correct, and the consumer did not
ask it — or one fact had N consumers and N-1 were fed.

| the authority that existed | who did not ask |
|---|---|
| the record-variant pattern arm carries a guard and explains it | the tuple-variant arm forty lines below |
| a variant's constructors go to the reflector and to the proof engine | the third consumer, which binds `result` to the body |
| `mount sys.darwin.…` normalises through `is_stdlib_toplevel_path` | the dotted CALL resolver, which only stripped from the left |
| `subtype.rs`: `T with [c1] <: T with [c2]` iff `c2 ⊆ c1` | argument checking, which unifies and peels both ways |
| field access peels the wrapper in `infer/expr.rs` | the second emitter of the same error in `infer/env.rs` |

So the first question is not "what should this do" but "where is this
already done, and why does that place not answer here". Three checks
cover most of it:

* **the same construct** — is there an arm for a relative and not for
  this one? (`If` handled, `Match` not; record handled, tuple not.)
* **the same fact** — how many consumers receive it? Count the
  `register_*` calls and the push-to-all loops.
* **the same decision** — is it already written in a module whose name
  says so? (`subtype.rs` for subtyping, `is_stdlib_toplevel_path` for
  path normalisation.)

When the sibling is found the fix is usually one line and arrives with
its own justification: not "I decided", but "it is already decided
there". When there is NO sibling and the behaviour is still wanted, that
is a DESIGN decision and must not be smuggled in as a repair — the tell
is having to invent an error code, a name or a policy.

### 13. Compare both sides of a measurement with ONE filter

`verum` emits errors both with and without a code. Exactly half the type
diagnostics in `verum_types` render through `DiagnosticBuilder::error()`
with no `.code(...)`, and the uncoded half is the memory-safety family —
`AffineViolation`, `DanglingReference`, `BorrowConflict`,
`AssignWhileBorrowed`, `CheckedRefEscape`.

So `grep -c '^error<'` scores those ZERO and calls the file clean, while
`grep -c '^error'` counts them. Using one on each side of a comparison
is not a comparison; it is two different measurements presented as one.
That produced a WRONG published conclusion in this session — "check
accepts what run refuses" — from two commands that in fact agreed.

Count `^error`. Use the same command on both sides. An instrument that
under-reports is worse than no instrument, because it answers
confidently.

### 14. Break every claim the programme makes about the language

A proving ground documents what the language guarantees. Those sentences
are the first thing to go stale, and nothing in a passing build notices:
a guarantee that quietly stopped being enforced reads exactly like one
that still is.

So break them, one edit each, and record what the compiler actually
said. Seven claims, seven edits, seven reverts:

| claim | the edit | what came back |
|---|---|---|
| refinement types | a `len` returning `-1` | `error<E500>: refinement constraint failed` |
| contexts (DI) | `using [Clock]` removed from a function that reads the clock | `error<E400>: no method named 'now' found for type 'Clock'` |
| `pure` is verified | a `print` in a pure function | `error<E503>: pure function … has side effects: IO` |
| meta refinements | a width the protocol refuses | `error<E506>: meta argument 1 violates its refinement` |
| `async` is inherited | `async` removed from a function that awaits | `Cannot await non-future type: …` |
| affine resources | one token spent twice | `value 'slot' used after move` |
| capability attenuation | a caller holding `[Evict]` calls `store` | **nothing; the project still checked clean** |

THREE KINDS OF RESULT, and each is worth having:

* **The claim holds.** Now it is checked rather than asserted, and the
  document can say how to re-check it.
* **The claim does NOT hold** — the last row. The programme was
  asserting a guarantee the compiler does not keep, which is the
  declared-but-unenforced shape rule 6 puts first, appearing in the
  proving ground's own documentation. Mark it in place; do not delete
  the signatures, because they are the honest shape of the rule and what
  the fix will make real.
* **The claim holds but the REPORT is weaker than the guarantee.** Two
  of the six enforced refusals carried no error code, so a gate
  filtering on `error<E…>` scores them zero; three named the symptom
  instead of the cause (`async` removed reads as "the awaited value is
  not a future"; the missing context reads as `Clock` having no method
  `now`, when it has one). Both classes were filed.

WHY THIS BEATS A GREP for diagnostic quality: it samples exactly the
paths somebody thought were worth promising, and it costs one edit and
one `verum check` each. It found two diagnostic defects and one
unenforced guarantee in under an hour.

A claim nobody has tried to falsify is worth what a scene that cannot
fail is worth.

### 15. Census what nothing in the product calls, then break those rules

Rule 14 breaks the claims a programme makes about the language. This
one finds claims nobody ever wrote down: a rule the compiler CONTAINS
and does not ask.

The instrument is two greps and a comparison. List the `pub fn`s in an
analysis crate; count, over the whole workspace's non-test sources, how
many call each; keep the ones whose only callers live under `tests/`.

APPLY THE SAME FILTER TO BOTH SIDES or the number is fiction. Counting
callers within one crate says 111 of 1619 in `verum_types`; counting
across every crate's `src/` says 86, and the 25 in between are what the
COMPILER calls from another crate. That is rule 13 again, and it is the
first thing to get wrong here.

The output is a suspect list, not a verdict. Constructors and accessors
legitimately have no internal caller. So take the alarming names and
FALSIFY them — write the smallest program that breaks the rule the
function implements, and see whether anything complains. Two probes
into an 86-name list produced, inside an hour:

* `check_linear_consumed` — no caller anywhere. `type linear` was
  parsed, registered, tracked, and its exactly-once obligation never
  asked. The spec that named it was `parse-pass`: ten cases about
  linear semantics, verifying that the text parses.
* `is_well_kinded` — no caller. `Pair<Int>` for a two-parameter `Pair`
  is accepted, and the free parameter is then unified with whatever
  arrives first.

BUILD THE POSITIVE CONTROL INTO EVERY PROBE. `let x: Int8 = 300` being
accepted means nothing until `let x: Int8 = "hello"` is refused in the
same file — otherwise you have measured that the position is unchecked,
or that the type does not exist, and reported it as a missing rule. The
control also catches the wrong POSITION: the arity probe first went in
a record field, where nothing is checked at all, and would have been
written up as a missing arity check instead of the separate, larger
finding that a type declaration's interior is unchecked.

AND MEASURE THE BLAST RADIUS BEFORE LANDING THE FIX. Turning the arity
check two-sided refuses 194 uses of `Result<X>` across 31 stdlib files
— and zero uses of any other multi-parameter type. One type, written
the same way 194 times, is not 194 mistakes: it is a missing feature
(a default type parameter) that the stdlib has been assuming. A fix
whose blast radius lands entirely on one shape is a message about the
shape, not about the sites.

### 16. Watch the sweep, do not just collect its result

A full corpus sweep is usually run to answer one narrow question — "did
my change break anything?" — and the temptation is to start it, walk
away, and read the total at the end.

The total is the least of what it produces. A sweep that was checking
2560 stdlib files for affine regressions stopped advancing at file 1761.
Two identical progress readings a few minutes apart were the whole
signal; `sample <pid>` turned them into a stack ending in
`RawRwLock::wait_for_readers`, and that was a P0 self-deadlock in the
type-check phase — `verum check core/intrinsics/mod.vr` had never
returned, for anyone, and nothing else had noticed because no gate runs
that file on its own.

WHAT TO ACTUALLY DO:

* **Print progress, and read it twice.** A count that has not moved
  between two checks is a hang, not slowness
  (see the "slow suite" rule). Sample immediately — the stack is only
  there while the process is stuck.
* **Put a per-item timeout in the loop.** Without one, a single hang
  costs the whole sweep; with one, the hang is a data point in the
  results file and the sweep finishes.
* **Read the FIRST few results, not only the last.** A sweep with a
  broken invocation reports 2560 clean files just as convincingly as a
  clean tree.
* **Verify COVERAGE by set difference, never by line count.** A resumed
  sweep produced 2564 result lines for a 2560-file corpus and looked
  complete; `comm -23 <all> <measured>` showed 363 files were never
  checked and 367 rows were duplicates. Line counts can only tell you
  how much was written, and a killed sweep writes plenty: `pkill`ing the
  checker does not stop the `while read` loop around it, so the loop
  races through the rest of the corpus recording every remaining file as
  ZERO ERRORS. The wreck of an interrupted sweep is a file full of false
  greens with the right number of lines.
* **Expect the sweep to answer questions you did not ask.** Its value is
  the breadth, not the question — nothing else in the project runs every
  stdlib file through the front door one at a time.

The corollary is a cost: a broad sweep at ~2s/file is an hour or two,
and a debug binary makes it eight. Build the release binary first. An
instrument you will not run twice is not an instrument.

## What the proving ground must keep doing

It has to stay ambitious. The moment it is trimmed to what already
works, it stops detecting: a program that only uses the working subset
proves the working subset works. Its scale, its concurrency, its
architecture and its use of the type system are not decoration on the
demonstration — they are the instrument.

Related: `docs/architecture/registry-federation-protocol.md` (what the
current proving ground is FOR), `docs/architecture/multi-session-taskpool.md`
(how the findings are filed and claimed).
