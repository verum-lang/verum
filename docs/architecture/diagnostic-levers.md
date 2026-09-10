# Diagnostic levers: the ones whose VALUE is a pattern

Most `VERUM_*` environment levers in this tree are presence flags — the
code asks `is_some()` or `is_ok()`, and any value at all turns them on.
Eighteen are not. Their value is a **substring matched against a name**,
and that changes what `=1` means:

```
VERUM_DUMP_VBC=1      # functions whose name contains "1" — usually none
VERUM_DUMP_VBC=       # EVERY function
VERUM_DUMP_VBC=make_  # the ones you meant
```

## The rule, and it is one rule

**An empty value shows everything.** `name.contains("")` is true for
every name, so `LEVER=` is the "show me all of it" setting for every
lever in the table below — including the two that also special-case
another value.

**`LEVER=1` shows almost nothing**, and shows it silently: an empty
trace is indistinguishable from a code path that never ran. That is the
whole cost of this page. Measured 2026-09-10, the failure was reached
twice in one session by two people, and the second time it was filed as
a missing-instrument defect before the lever was read.

## Why the silence is the expensive part

A filter that matches nothing prints nothing, and nothing is exactly
what a broken probe prints. So the reading "the mechanism does not run"
and the reading "I asked for names containing the digit one" produce
identical evidence, and only one of them is about the program.

The general form is in `docs/architecture/defect-class-catalogue.md`
under absent-output reasoning: an absence is evidence only when the
list of things that produce it has one entry.

## The levers

Each takes a substring of the name it filters. `LEVER=` shows all.

| lever | filters on | first site |
|---|---|---|
| `VERUM_DUMP_FN` | function name | `verum_compiler/src/pipeline/stdlib_bootstrap.rs` |
| `VERUM_DUMP_VBC` | function name | `verum_vbc/src/codegen/mod.rs` |
| `VERUM_TRACE_CALLBIND` | callee name | `verum_vbc/src/codegen/expressions.rs` |
| `VERUM_TRACE_CALLRES` | callee name | `verum_vbc/src/codegen/expressions.rs` |
| `VERUM_TRACE_DEREF` | value/type shape | `verum_codegen/src/llvm/instruction.rs` |
| `VERUM_TRACE_FNREG` | function name | `verum_vbc/src/codegen/context.rs` |
| `VERUM_TRACE_FNTABLE` | function name | `verum_codegen/src/llvm/vbc_lowering.rs` |
| `VERUM_TRACE_INSTANTIATE` | type name | `verum_types/src/context.rs` |
| `VERUM_TRACE_INTERNAL_ERR` | error text | `verum_codegen/src/llvm/error.rs` |
| `VERUM_TRACE_PARAMMARK` | function name | `verum_codegen/src/llvm/vbc_lowering.rs` |
| `VERUM_TRACE_STATIC_ALIAS` | alias name | `verum_vbc/src/codegen/expressions.rs` |
| `VERUM_TRACE_STRDICE` | string content | `verum_vbc/src/codegen/context.rs` |
| `VERUM_TRACE_STUB` | type name | `verum_compiler/src/pipeline/stdlib_bootstrap.rs` |
| `VERUM_TRACE_UNDEF_FN` | function name | `verum_vbc/src/codegen/expressions.rs` |
| `VERUM_TRACE_UNDEF_VAR` | variable name | `verum_vbc/src/codegen/expressions.rs` |
| `VERUM_TRACE_UNIFY` | either type's shape | `verum_types/src/unify.rs` |
| `VERUM_TRACE_UNIFY_ENTRY` | either type's shape | `verum_types/src/unify.rs` |
| `VERUM_TRACE_WANTED` | wanted name | `verum_compiler/src/archive_ctx_loader.rs` |

Two carry an extra convention on top of the rule:

* `VERUM_DUMP_VBC=*` also means everything — it predates the observation
  that an empty value already does.
* `VERUM_TRACE_STUB` and `VERUM_TRACE_WANTED` treat `1` as **not a
  filter**: their per-name lines are skipped for that value while their
  summary lines, which are presence-gated, still print. So `=1` there
  yields a partial trace, which reads as a complete one.

## Keeping this list true

`make check-diagnostic-levers` compares the table above against the
tree: a lever whose value reaches a `contains` / `starts_with` /
`split` call must appear here, and a row here must still be
filter-shaped in the code. Both directions fail, because a reference
that silently loses an entry is worse than no reference — the reader
who checks it and finds nothing concludes the lever is a flag.

Presence-flag levers are deliberately NOT listed. There are over two
hundred of them, they behave the way a reader expects, and enumerating
them would bury the eighteen that do not.
