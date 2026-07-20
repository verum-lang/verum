# Diagnostic Totality — "No Silent No-Ops"

Status: NORMATIVE design (2026-07-21). Companion to
`name-resolution.md`. Where that document makes *name* resolution total (every
name resolves or errors), this one makes *meaning* total: the compiler must not
silently accept code that provably does nothing. It exists because a mechanical
refactor corrupted shipping stdlib code and **the corruption compiled green**.

## 1. The incident (learn-from-this root analysis)

Commit `351cc9b90` ("literal-suffix sweep — 417 files, 2625 sites") rewrote
`f(N as Int64)` → `f(N_i64)` with a regex that also **ate the enclosing call
parens**, so `self.fullscan_step.add(1_i64);` became
`self.fullscan_step.add1_i64;`. Across the codebase this hit hundreds of sites
(T0529: 77 spec files; T0553: 5 shipping stdlib files / 14 counter calls; T0555
family: more). In shipping code the effect was silent data loss — **every
sqlite status/observability counter stopped incrementing** — with no error at
build, bake, or run.

It survived a **chain of four independent silent-acceptance gaps**. Any one of
them, closed, would have caught it:

| # | Silent gap | Why it hid the bug |
|---|---|---|
| G1 | **Undefined field/member access is swallowed.** `.add1_i64` parsed as a field access on a counter; that field does not exist, but records return a swallowed value instead of erroring. | The corrupted token looked like a valid field read. |
| G2 | **A no-effect expression statement is not diagnosed.** `self.field.add1_i64;` is a pure path/field expression used as a statement — its value is computed and discarded, with no call, no assignment, no side effect. | Even a *valid* field read used this way is meaningless; the compiler accepted it as a statement. |
| G3 | **Stdlib bodies are typechecked on no path** (T0124/T0516). The bake has no inference pass over `core/` bodies; `VERUM_STDLIB_PATH` overrides only the registry. | Whatever G1/G2 would have flagged never ran over the corrupted files. |
| G4 | **The bulk refactor had no re-typecheck gate.** A 417-file mechanical rewrite was committed without a pass that would have surfaced 2625 changed call sites turning into no-ops. | Process, not compiler — but G1–G3 are what would have *made* a gate meaningful. |

The lesson is not "regexes are dangerous" (they are, separately —
`mechanical-refactor-corruption-repair` covers that). The lesson is that **the
compiler silently blessed dead code**, so a whole class of corruption — any
edit that turns a call or assignment into a discarded expression — is invisible.

## 2. The principle

> Every construct the programmer writes must either have an observable effect,
> contribute to a value that is used, or be an explicit, marked discard. Code
> that provably does nothing is a diagnostic, never silent acceptance.

This is the statement/effect analogue of name-resolution totality. It is also
exactly what Rust's `path_statements` and `unused_must_use` lints enforce, and
what the ML "value restriction warnings" family covers. Verum has neither today.

## 3. The guards (each a filed task; G2 is the general net)

### G1 — Loud undefined field/member access  (task T0563)
A `receiver.member` access where `member` is neither a declared field of the
receiver's type, an associated function, nor a protocol method in scope, is a
typed error (`E-field`), with a did-you-mean over the type's real members. This
extends `name-resolution.md` §3.4 totality from names to fields. It requires the
receiver's type to be *known* — which is why it must run under a real typecheck
(G3), and why the record-field-swallow (`verum-types-resolution-traps`) is
removed rather than softened.

### G2 — No-effect expression statement  (task T0564)  ★ the class-killer
A statement that is a *pure* expression whose value is discarded — a path,
field access, literal, or an operator/call chain the effect system proves has
no side effect and is not `must_use` — is a diagnostic (`W-noeffect`, promotable
to error under `--strict` and in the stdlib gate). This is the **general** net:
it catches the corruption regardless of what the mangled token resolves to,
because the corrupted form is *always* a discarded expression. It composes with
Verum's computational-properties system (Pure vs IO/Mutates): a statement whose
expression is inferred `Pure` and whose result is unused is the exact signature
of "a call lost its parens" or "an assignment lost its `=`".

Design care:
* `must_use` results (e.g. `Result`, `Maybe`, a `#[must_use]`-analogue) already
  want a stronger version of this — fold them in: an unused `must_use` value is
  an error even if producing it had a side effect.
* Explicit discard stays legal and silent: `let _ = expr;` or a `discard expr;`
  form. A bare `expr;` with observable effect (a call to an `IO`/`Mutates`
  function) is fine — only the *pure-and-discarded* case is flagged.
* Trailing tail expressions (block value position) are unaffected — they are
  used by definition.

### G3 — Stdlib body typecheck gate  (task T0124, already filed, raised to P1)
`verum check --stdlib` runs the full resolver + inference over `core/` bodies,
blocking in CI. Without it, G1/G2 protect user code but not the stdlib — and the
stdlib is where this incident, T0516's 18 unchecked functions, and T0527's ~130
calls to nonexistent `JsonValue` methods all hid.

### G4 — Mechanical-refactor gate  (process, task T0565)
Any bulk/scripted edit over N>1 files must be followed by a re-typecheck of the
touched set (made meaningful by G1–G3) and a paren/bracket-balance +
diff-multiset check (the technique in `mechanical-refactor-corruption-repair`).
A sweep that increases the count of no-effect statements fails the gate.

## 4. Why this is fundamental, not a lint pile

G1 and G2 are two independent nets under the same class, and G3 makes them cover
the code most likely to be swept mechanically. Together they change the failure
mode from *silent data loss discovered by a human reading counters* to *a build
error naming the file and line*. The same two nets also catch:

* an assignment that lost its `=` (`x.field value;` → no-effect statement),
* a `?`/`try` that got dropped (unused `Result` → must-use error),
* a method call whose receiver typo'd to a wrong-but-existing field,
* the "regex ate the call" class in full.

That is the test of a fundamental fix: it retires the *class*, and it would have
caught defects nobody had connected to it. This design is a prerequisite for the
"100% production ready" bar — a language whose compiler blesses dead code cannot
make that claim.

## 5. Sequencing

G3 (T0124) first — it is the gate that makes G1/G2 observable in the stdlib and
is already the campaign gate for `name-resolution.md`. Then G2 (the general net,
widest coverage per unit of risk), then G1 (needs the record-field-swallow
removed, which touches inference). G4 is process and can land immediately.
