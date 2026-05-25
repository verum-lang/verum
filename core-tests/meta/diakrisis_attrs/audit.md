# `meta/diakrisis_attrs` audit

Module: `core/meta/diakrisis_attrs.vr` (~292 LOC) — VVA Part B
advisory-attribute meta-fn parsers (V8.1 #196 META1 reorganisation).
Five attribute families: `@effect`, `@infinity_category`,
`@autopoietic`, `@ludic_design`, `@cut_elimination`.

Tests: 38 unit tests over the data-only surface (variant ctors,
record construction, lookup tables, the default-bound constant).

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.math.frameworks.*` | declares `@effect(...)` / `@infinity_category(...)` / `@autopoietic(...)` on theorems |
| `core.proof::tactics` | inspects `@effect(io)` / `@effect(state)` markers to gate purity-requiring tactics |
| `verum_compiler::derives::diakrisis` | dispatches on these advisory attributes during the audit walker |

## 2. Crate-side hardcodes

* `verum_compiler::audit_walker` mirrors the five
  attribute-name constants (`effect`, `infinity_category`,
  `autopoietic`, `ludic_design`, `cut_elimination`).
* `verum_compiler::diakrisis_typed_attrs` (legacy, pre-reorg) had
  Rust-side typed-attr structs for these — superseded by this
  meta-system module; the legacy Rust file should be removed
  once consumers migrate.

## 3. Language-implementation gaps

### §3.1 `AttributeArg` schema drift (HIGH)

`diakrisis_attrs.vr` matches on `AttributeArg` as if it were an
**enum**:

```verum
match attr.args.get(0) {
    Maybe.Some(AttributeArg.Ident(name)) => ...,
    Maybe.Some(AttributeArg.Int(n)) => ...,
    Maybe.Some(AttributeArg.Str(s)) => ...,
    Maybe.Some(AttributeArg.Named { name, value }) => ...,
    _ => ...,
}
```

But `core/meta/attribute.vr` defines `AttributeArg` as a **record**:

```verum
public type AttributeArg is {
    name: Maybe<Text>,
    value: MetaAttributeValue,
    span: Span,
};
```

`AttributeArg.Ident(...)` / `AttributeArg.Int(...)` / `AttributeArg.Str(...)` /
`AttributeArg.Named { ... }` are **not valid** constructors of the record-form
`AttributeArg`. The parse-functions either don't compile, or compile and
silently miss every match arm (returning `Maybe.None` for every input).

**Fix path (1-2h):** rewrite each `parse_*` in
`core/meta/diakrisis_attrs.vr` against the record-form:

```verum
match attr.args.get(0) {
    Maybe.Some(arg) => match arg.value {
        MetaAttributeValue.Ident(name) => ...,
        MetaAttributeValue.Int(n)      => ...,
        MetaAttributeValue.String(s)   => ...,
        _ => Maybe.None,
    },
    Maybe.None => Maybe.None,
}
```

This is a stdlib-source-level defect, NOT a language defect. The
audit pins it but the test layer cannot exercise the parse_*
functions until they are realigned.

### §3.2 `parse_autopoietic` named-args support

Once §3.1 is resolved, the named-args path (which currently uses
`AttributeArg.Named { name, value }`) needs to be rewritten to
match on the record's `arg.name == Maybe.Some("epsilon")` form
plus the value-side `MetaAttributeValue.String(s)` /
`MetaAttributeValue.Int(n)` arms.

### §3.3 Tests for parse_* are deferred until §3.1 closes

Once `core/meta/diakrisis_attrs.vr` matches the record-form
`AttributeArg`, add unit tests in this folder for:

* `parse_effect_attr` happy path (each of the 7 known kinds + 2 aliases)
* `parse_effect_attr` rejects wrong arg count / non-Ident arg
* `parse_infinity_category` happy path (Finite N / "omega" /
  "omega_omega" idents)
* `parse_infinity_category` rejects negative Int
* `parse_autopoietic` (named-args, order-independent)
* `parse_autopoietic` rejects negative depth
* `parse_ludic_design` (zero args)
* `parse_cut_elimination` (default / named-arg / bare-int forms)

## Action items landed in this branch

* `core-tests/meta/diakrisis_attrs/unit_test.vr` — 38 unit tests:
  DiakrisisEffectKind 7-variant + .from_ident / .as_str /
  case-insensitive parse + InfinityLevel 3-variant + EffectAttr /
  InfinityCategoryAttr / AutopoieticAttr / LudicDesignAttr /
  CutEliminationAttr record construction + CUT_ELIMINATION_DEFAULT_BOUND
  constant.
* `core-tests/meta/diakrisis_attrs/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Realign `parse_*` to record-form `AttributeArg` (§3.1) | core/meta/diakrisis_attrs.vr | 1-2 h |
| `parse_*` unit tests (§3.3) | this folder | 1 h after §3.1 |
| Property test: `from_ident` ∘ `as_str` is identity on the 7 canonical kinds | this folder | 30 min |
| Remove legacy Rust typed-attr file post-migration | crates/verum_compiler/src/diakrisis_typed_attrs.rs | 30 min |
