# Language Law: Constructor Visibility Horizon & Boolean-Equality Clarity

**Status:** design accepted 2026-08-05 (author-directed); staged for
implementation in this document's §4 plan. Companion to
`name-resolution.md` (the resolution architecture this law simplifies)
and the T0690/R1 campaign (which this law makes largely unnecessary
going forward).

Two grammar-level laws, both in the language's founding spirit —
*complex things through the simplest constructs* — and both converting
a recurring, expensively-diagnosed defect class from "fixed by the
compiler" to **inexpressible in the language**.

---

## Law 1 — CONSTRUCTOR-VISIBILITY-HORIZON

### 1.1 The defect class this kills

A bare variant constructor (`UoInsert`, `HkUpdate`, `None`) is today an
AMBIENT name: every sum type in the loaded stdlib publishes each of its
variant spellings into one flat table, and resolution picks by
first-wins / arity / heuristic tiebreaks. Measured cost (all from the
2026-08-05 flagship cascade, each independently root-caused):

* two/three types sharing variant spellings made bare constructors
  unresolvable or WRONG-TAGGED (`HkUpdate` built with tag 2 = the other
  enum's `HkCommit`);
* 196 stdlib type names collide with some other type's variant name
  (T0509 census); 14 sum types spell a variant `IoError`;
* five systemic resolver layers (entry-granularity sharding, parents
  mirror, placeholder guard, taxonomy dedup ×2) were required to make
  ONE spec green — all of them curing symptoms the grammar permits.

### 1.2 The law

> A bare (unqualified) variant constructor name is legal **iff its
> owning type is in the file's explicit mount horizon** — named by an
> explicit `mount` item (directly, or via a braced list), or declared
> in the file itself, or in the prelude's PINNED set (`Maybe`,
> `Result`, `Ordering` — the three canonical carriers, already
> layout-pinned in `verum_common::well_known_types`).
>
> Everywhere else the constructor MUST be qualified: `Type.Variant`.
> Glob mounts (`mount m.*`) do NOT extend the bare-constructor horizon
> — they import types and functions, not constructor bareness.

Consequences, by construction:

* first-wins tables become UNREACHABLE for bare ctors — the horizon
  names exactly one owner or the program is ill-formed with a
  deterministic "qualify it" diagnostic (E4xx, listing the candidate
  owners);
* two stdlib modules may freely reuse variant spellings (today's
  `Uo*`/`Hk*`/`IoError` reality) with ZERO resolver heuristics;
* the ambient `variant_constructor_parents` table shrinks from a
  resolution AUTHORITY to a diagnostic aid (did-you-mean).

### 1.3 Grammar delta

The change is a **well-formedness rule**, not a production change —
`primary_expr` already admits `IDENT` and `path_expr`; the law
constrains name RESOLUTION of the `IDENT` case. One normative comment
block lands in `grammar/verum.ebnf` beside `variant_list`:

```ebnf
(* CONSTRUCTOR-VISIBILITY-HORIZON (language law, 2026-08-05):
   a bare IDENT resolving to a variant constructor is well-formed
   iff the owning sum type is (a) declared in this file, (b) named
   by an explicit non-glob mount of this file, or (c) one of the
   pinned prelude carriers Maybe | Result | Ordering.  Otherwise
   the constructor must be written qualified: Type '.' Variant.
   Glob mounts do not extend the bare-constructor horizon. *)
```

### 1.4 Semantics of the horizon (precise)

The horizon H(file) is the set of type names:
1. declared by `type` items of the file (incl. inline modules);
2. appearing as a leaf in any explicit `mount … .{ A, B as C, … }`
   item of the file (the *type* leaf itself, or any variant leaf —
   mounting `{UoInsert}` puts its OWNING type in H by that leaf's
   resolution);
3. `Maybe`, `Result`, `Ordering` (prelude-pinned).

A bare ctor `V` resolves iff **exactly one** type in H declares `V`;
zero ⇒ error "bare constructor outside its type's mount horizon —
write `T.V` or mount `T`"; two+ ⇒ error listing the H-owners (NOT the
ambient owners). Resolution inside H uses the existing scoped
machinery; nothing outside H is consulted.

---

## Law 2 — BOOLEAN-EQUALITY-CLARITY

### 2.1 The defect class this kills

`==`/`!=` bind tighter than `&&`/`||` (conventional and unchanged).
With Bool operands BOTH readings type-check, so the mis-parse is
silent: `ensures a && a == a` parses as `a && (a == a)` ≡ `a` — ten
boolean-algebra laws in `core/math/tactics.vr` "proved" vacuously and
entered the rewrite layer unproved. The
`bool-eq-in-conjunction` lint (T0485) warns today; a LAW makes the
shape ill-formed where it matters.

### 2.2 The law

> Inside a bare `&&`/`||` conjunction, an `==`/`!=` whose BOTH operands
> are of type `Bool` is **ill-formed** unless the equality is
> parenthesized. Diagnostic prints the actual parse and both bracketed
> readings. Non-Bool equalities (`x == a || x == b`,
> `n >= 0 && flag == true` where one side is non-Bool… see below) are
> untouched.

Precision (matching the lint's calibrated selectivity, which kept the
correct majority silent across a core/-wide scan): the rule fires when
the equality's both operands are Bool **and** the sibling conjunct
operand is itself Bool-atomic or another such equality — i.e. exactly
the shapes where the two readings are both type-correct and therefore
silently divergent.

### 2.3 Grammar delta

Again a well-formedness rule on the existing productions (precedence
itself is NOT changed — changing it would silently re-parse existing
code, the worst possible migration). Normative comment beside
`logical_and_expr`:

```ebnf
(* BOOLEAN-EQUALITY-CLARITY (language law, 2026-08-05): within a
   logical_and_expr / logical_or_expr, an equality_expr using '=='
   or '!=' whose operands BOTH type as Bool is ill-formed unless
   parenthesized, when the adjacent conjunct operand is itself of
   type Bool.  Rationale: with Bool operands both precedence
   readings type-check, so the mis-parse is silent and — in
   requires/ensures — makes proof obligations vacuous. *)
```

---

## 3. Diagnostics (one voice)

* **E430** `bare constructor 'V' outside its type's mount horizon` —
  help: `write 'T.V'` / `add 'mount <module>.{T};'`; candidates listed
  from the ambient table (which survives exactly for this).
* **E431** `bare constructor 'V' is ambiguous within the mount horizon`
  — lists the H-owners; help: qualify.
* **E432** `Bool == Bool inside '&&'/'||' needs parentheses` — prints
  `a && (b == c)` vs `(a && b) == c` with the actual parse marked.

## 4. Migration plan (staged, honest, reversible)

Both laws land **warn-first, error-later**, gated by ONE switch so the
whole tree migrates in lockstep:

* **Stage W (land now):** checker emits E430/E431/E432 as WARNINGS by
  default; `VERUM_LANGUAGE_LAWS=strict` (and `--language-laws=strict`)
  upgrades to errors. The T0485 lint is subsumed by E432's engine (one
  implementation, lint id retained as an alias).
* **Stage M (mechanical migration):** a census script
  (`scripts/ci/census_language_laws.py`) lists every violating site in
  core/ + vcs/ + core-tests/. Migration is mechanical and safe:
  qualify the ctor / add the mount leaf / add parentheses. Stage M is
  DONE when the census is zero under strict.
* **Stage E (flip):** strict becomes the default; the env var flips to
  an escape hatch `VERUM_LANGUAGE_LAWS=legacy` documented as
  deprecated; CI runs the census as a gate (fails on ANY new site).
* **Stage R (retire):** after one release cycle, `legacy` is removed;
  the resolver heuristics that Law 1 obsoletes (ambient-first-wins for
  bare ctors) are deleted, keeping the ambient table only for
  did-you-mean.

Implementation seams (all existing):
* Law 1 horizon = `explicit_imports` + file decls + the pinned trio —
  the checker already tracks all three (T0525 mount-scoped selection
  is the direct precedent; the law PROMOTES that tiebreak to the only
  rule);
* Law 2 engine = the shipped `BoolEqInConjunctionPass` detection,
  relocated from the lint runner into the checker with the E432 code.

## 5. What this does NOT change

* Precedence tables — untouched (no silent re-parse).
* Qualified constructors, patterns (`match` arms use their scrutinee's
  type today — already horizon-correct), glob mounts for types/fns.
* The pinned prelude trio stays bare everywhere — `Some`/`Ok`/`None`
  ergonomics are load-bearing for the whole stdlib.
