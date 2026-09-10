# Diagnostic levers: the ones whose VALUE is a pattern

Most `VERUM_*` environment levers in this tree are presence flags — the
code asks `is_some()` or `is_ok()`, and any value at all turns them on.
Twenty-one are not: their value is matched against a name, and what
`=1` does to them ranges from "nothing" to "somebody else's trace".

```
VERUM_DUMP_VBC=1      # functions whose name contains "1" — usually none
VERUM_DUMP_VBC=       # EVERY function
VERUM_DUMP_VBC=make_  # the ones you meant
```

## There is no single rule — look the lever up

An earlier version of this page claimed one: "an empty value shows
everything, because `name.contains("")` is true for every name". It
held for most of the table and inverts for a third of it, which is the
worst shape a rule can have — it is right often enough to be trusted
and wrong often enough to mislead. Measured, the tree carries four
conventions, and **seven levers have no value at all that shows
everything**.

The column below is the answer. Read it before concluding a lever is
inert.

## Why the silence is the expensive part

A filter that matches nothing prints nothing, and nothing is exactly
what a probe that never ran prints. The reading "the mechanism does not
fire" and the reading "I asked for names containing the digit one"
produce identical evidence, and only one of them is about the program.

`VERUM_TRACE_WANTED` is worse than silent: `=1` is rewritten to the
literal `format_debug`, so it prints a real trace about a name you did
not ask for. Output that is confidently about the wrong subject beats
no output as a way to lose an afternoon.

## The levers

`shows everything` is the value — or values — that disable the filter.

| lever | match | shows everything | filters on |
|---|---|---|---|
| `VERUM_DUMP_FN` | substring | **nothing does** | function name |
| `VERUM_DUMP_VBC` | substring | `` (empty) or `*` | function name |
| `VERUM_TRACE_BARE_VARIANT` | substring | **nothing does** | variant name |
| `VERUM_TRACE_CALLBIND` | substring | `` (empty) | callee name |
| `VERUM_TRACE_CALLRES` | substring | **nothing does** | callee name |
| `VERUM_TRACE_CANON` | substring | `` , `*`, `1`, `true`, `all` | canonical path |
| `VERUM_TRACE_DEREF` | substring | `` (empty) or `1` | value/type shape |
| `VERUM_TRACE_FNREG` | substring | `` (empty) | function name |
| `VERUM_TRACE_FNTABLE` | substring | `` (empty) | function name |
| `VERUM_TRACE_INSTANTIATE` | substring | **nothing does** | type name |
| `VERUM_TRACE_INTERNAL_ERR` | substring | `` (empty) | error text |
| `VERUM_TRACE_PARAMMARK` | substring | `` (empty) or `1` | function name |
| `VERUM_TRACE_STATIC_ALIAS` | substring | **nothing does** | alias name |
| `VERUM_TRACE_STRDICE` | substring | **nothing does** | string content |
| `VERUM_TRACE_STUB` | substring | `` (empty) | type name |
| `VERUM_TRACE_TYPE_CLAIM` | **exact** | `*` only | type name |
| `VERUM_TRACE_UNDEF_FN` | substring | `` (empty) | function name |
| `VERUM_TRACE_UNDEF_VAR` | substring | **nothing does** | variable name |
| `VERUM_TRACE_UNIFY` | substring | `` (empty) | either type's shape |
| `VERUM_TRACE_UNIFY_ENTRY` | substring | `` (empty) | either type's shape |
| `VERUM_TRACE_WANTED` | substring | `` (empty) | wanted name |

Two further notes the column cannot carry:

* `VERUM_TRACE_STUB` and `VERUM_TRACE_WANTED` treat `1` as **not a
  filter** on their per-name path, while their summary lines are
  presence-gated and still print. `=1` therefore yields a PARTIAL trace
  that reads as a complete one.
* `VERUM_TRACE_TYPE_CLAIM` compares for **equality**, not containment
  (`w == "*" || w == name`). A prefix will not do; the name must match
  exactly, and an empty value matches nothing at all.

## Reading a lever's own site

The column above is derived and spot-checked, not authoritative — the
site is. Three shapes tell you which convention you are in:

```rust
name.contains(&v)              // substring; empty matches every name
v == "*" || v == name          // exact; empty matches nothing
!v.is_empty() && name.contains(&v)   // substring, and empty is REFUSED
```

## Keeping this list true

`make check-diagnostic-levers` compares the table against the tree in
both directions: a lever whose value reaches a `contains` /
`starts_with` / `split` / equality-against-a-name must appear here, and
a row here must still be pattern-shaped in the code. Both fail, because
a reference that silently loses an entry is worse than no reference —
the reader who checks it and finds nothing concludes the lever is a
flag.

The detector knows four binding forms:

```rust
let v = env::var("L")              let-binding
if let Ok(v) = env::var("L")       the form that carried VERUM_DUMP_VBC
let Ok(v) = env::var("L") else     let-else
match env::var("L") { Ok(v) => …   the form that carried TYPE_CLAIM,
                                   BARE_VARIANT and CANON
```

Each was added after a census that looked complete without it. The
first knew only the let-binding and reported ONE pattern-shaped lever
in the whole tree; the third knew three and reported eighteen, missing
three more. A detector that cannot find a case whose answer you already
know has not measured anything yet — check it against a known lever
before believing a population.

Presence-flag levers are deliberately NOT listed. There are over two
hundred of them, they behave the way a reader expects, and enumerating
them would bury the twenty-one that do not.
