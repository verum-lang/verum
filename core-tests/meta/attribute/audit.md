# `meta/attribute` audit

Module: `core/meta/attribute.vr` (~624 LOC) — typed attribute
representation for macros / @cfg / @derive / @repr / @inline.

Tests: 41 unit tests covering AttributeStyle 3-variant +
MetaAttributeValue 8-variant (Ident/String/Int/Float/Bool/
Nested/Eq/Tokens) + ctors + accessors (.is_ident, .as_ident,
.as_string) + AttributeArg .positional / .named / .is_named /
.as_int / .as_bool / .as_ident / .as_string + CfgPredicate
5-variant + ReprKind 4-variant.

## 1. Cross-stdlib usage

| consumer | how |
|---|---|
| `core.meta.quote` | builds attributes inside `quote!` blocks. |
| `core.meta.tactic` | reads attribute metadata for proof tactics. |
| `verum_compiler::derives` | matches @derive(...) args via this surface. |
| `verum_compiler::cfg_resolve` | evaluates CfgPredicate trees. |

## 2. Crate-side hardcodes

`verum_ast::attr` defines the parser-side mirror. Variant set +
payload shapes MUST agree. `verum_compiler::derives` matches on
the same MetaAttributeValue variants via VBC marshalling.

## 3. Language-implementation gaps

### §3.1 Add `evaluate` test once CfgPredicate intrinsic is wired

`CfgPredicate.evaluate` is `@compiler_intrinsic` and depends on
the build-target cfg map. Test infrastructure needs a way to
inject a synthetic cfg map for unit tests.

**Effort:** moderate (~2h, requires runtime cfg injection API).

### §3.2 Add roundtrip parse_args test for All/Any/Not nesting

Verify parser-side `attribute → CfgPredicate` for the
`all(unix, any(target_os = "linux", target_os = "macos"))`
nesting case. Currently visible only at compiler-test layer.

### §3.3 ReprInfo property test on @repr(align(N), packed) combo

ReprInfo accepts `align: Maybe<Int>` and `packed: Bool`
simultaneously; test the four combinations of the field set.

## Action items landed in this branch

* `core-tests/meta/attribute/unit_test.vr` — 41 unit tests over
  AttributeStyle + MetaAttributeValue + AttributeArg +
  CfgPredicate + ReprKind.
* `core-tests/meta/attribute/audit.md` — this file.

## Action items deferred

| Item | Scope | Estimated effort |
|---|---|---|
| CfgPredicate.evaluate test with injected cfg map | this folder | 2h |
| Roundtrip parse_args test for nested cfg | this folder | 1h |
| ReprInfo property test on align+packed combinations | this folder | 30 min |
| Sister tests for `core.meta.{token,tactic,quote,reflection,span}` | sister folders | 1 week total |
