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

### §3.1 `AttributeArg` schema drift — CLOSED 2026-05-25

Originally documented: `diakrisis_attrs.vr` matched on
`AttributeArg.Ident(...)` / `AttributeArg.Int(...)` /
`AttributeArg.Str(...)` / `AttributeArg.Named { ... }` — but
`core/meta/attribute.vr` declares `AttributeArg` as a **record**
`{ name: Maybe<Text>, value: MetaAttributeValue, span: Span }`.
The match arms silently missed every input.

**Closed by realigning all 5 `parse_*` against the record-form
+ canonical `MetaAttributeValue` enum**:

```verum
match attr.args.get(0) {
    Maybe.Some(arg) => {
        if arg.name is Maybe.Some { Maybe.None }     // positional contract
        else {
            match arg.value {
                MetaAttributeValue.Ident(name) => ...,
                MetaAttributeValue.Int(n)      => ...,
                MetaAttributeValue.String(s)   => ...,
                _ => Maybe.None,
            }
        }
    },
    Maybe.None => Maybe.None,
}
```

The `Str` value-variant was also renamed to its canonical name
`String` (matches `MetaAttributeValue.String(Text)` in
`core/meta/attribute.vr`).

Anchor: `core-tests/meta/diakrisis_attrs/integration_test.vr` —
45 tests across all 5 parsers covering happy paths + every
rejection mode (wrong attr name, wrong arg count, wrong value
variant, wrong arg-name in named form, negative-N rejection).

### §3.2 `parse_autopoietic` named-args support — CLOSED 2026-05-25

Closed as part of §3.1 — `parse_autopoietic` now matches on
`arg.name == Maybe.Some("epsilon")` / `Maybe.Some("depth")` and
inspects `arg.value` for `MetaAttributeValue.String` /
`MetaAttributeValue.Int`. Order-independent property pinned by
`test_parse_autopoietic_order_independent`.

### §3.3 Tests for parse_* — CLOSED 2026-05-25

`integration_test.vr` lands 45 tests across all 5 parsers:

* `parse_effect_attr` — 7 canonical kinds (Pure/Io/State/Async/
  Exception/Nondet/Quantum) + 2 aliases (exn / non_det) + 5
  rejection modes (wrong attr name / no args / String arg /
  unknown ident / named-arg)
* `parse_infinity_category` — Finite (0, 2) + "omega" /
  "omega_omega" idents + 4 rejection modes (negative Int /
  unknown ident / wrong name / no args)
* `parse_autopoietic` — both-present + order-independent + 5
  rejection modes (missing epsilon / missing depth / negative
  depth / wrong attr name / wrong epsilon value type)
* `parse_ludic_design` — zero-args happy + arg-present rejection
  + wrong-name rejection
* `parse_cut_elimination` — default (zero args uses
  CUT_ELIMINATION_DEFAULT_BOUND) + bare-int + named-arg + 5
  rejection modes (negative bare-int / negative named-arg /
  wrong named key / non-Int named value / two-args /
  wrong attr name)

## Action items landed in this branch

* `core/meta/diakrisis_attrs.vr` — realigned all 5 `parse_*` to
  record-form `AttributeArg` + canonical `MetaAttributeValue` enum.
  Closes §3.1 and §3.2.
* `core-tests/meta/diakrisis_attrs/unit_test.vr` — 38 unit tests:
  DiakrisisEffectKind 7-variant + .from_ident / .as_str /
  case-insensitive parse + InfinityLevel 3-variant + EffectAttr /
  InfinityCategoryAttr / AutopoieticAttr / LudicDesignAttr /
  CutEliminationAttr record construction + CUT_ELIMINATION_DEFAULT_BOUND
  constant.
* `core-tests/meta/diakrisis_attrs/integration_test.vr` — 45 tests
  across all 5 `parse_*` functions. Closes §3.3.
* `core-tests/meta/diakrisis_attrs/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| Property test: `from_ident` ∘ `as_str` is identity on the 7 canonical kinds | this folder | 30 min |
| Remove legacy Rust typed-attr file post-migration | crates/verum_compiler/src/diakrisis_typed_attrs.rs | 30 min |
