# The proving ground: how defects in Verum are found

This describes how work on the language is driven, and it is a
description of what worked rather than a proposal. One campaign found
fifteen defects this way, eight of which are fixed; two of them made
whole features inert without anyone noticing.

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

## What the proving ground must keep doing

It has to stay ambitious. The moment it is trimmed to what already
works, it stops detecting: a program that only uses the working subset
proves the working subset works. Its scale, its concurrency, its
architecture and its use of the type system are not decoration on the
demonstration — they are the instrument.

Related: `docs/architecture/registry-federation-protocol.md` (what the
current proving ground is FOR), `docs/architecture/multi-session-taskpool.md`
(how the findings are filed and claimed).
