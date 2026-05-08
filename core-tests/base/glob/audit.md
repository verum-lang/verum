# Audit — `core/base/glob.vr`

## §0  Surface area

| Item | Status |
|---|---|
| Source | `core/base/glob.vr` (332 lines) |
| Tests | NEW — `unit_test.vr` (~90 LOC, * / ** / ? / [...] / case-sensitivity), `property_test.vr` (~90 LOC, self-match + star ⊇ literal + globstar ⊇ star + class commutativity + truth table) |
| Hardcodes in `crates/` | none — pure stdlib |

## §1  POSIX-style glob

Operators implemented:
- `*` — any sequence of non-separator chars
- `**` — any sequence including separators (globstar)
- `?` — any single char
- `[abc]` — character class
- `[!abc]` — negated class (likely; verify in source)
- escape — likely `\` for literal special chars

Reference: POSIX glob(7), bash man page §3.5.8.

## §2  Path-separator handling

`*` does not cross `/` (the canonical POSIX rule). `**` does. Pinned
in `unit_test.vr §1-2`.

## §3  Action items landed in this branch

- [x]  Scaffold `core-tests/base/glob/`
- [x]  `unit_test.vr` — `*` (within / across separator), `**`, `?`,
       character class, literal, `compile`/`compile_with` API
- [x]  `property_test.vr` — literal self-match, star ⊇ prefix-suffix
       literal, globstar ⊇ star, char-class reorder commutativity,
       @test_case truth table
- [x]  This audit document

## §4  Action items deferred

1. **Negated character classes [!abc]** — pin behaviour.
2. **Escape sequences** — `\*` matches a literal `*`; pin if supported.
3. **Brace expansion** `{a,b,c}` — POSIX glob doesn't include this,
   but bash does. Verify `core.base.glob` semantics matches the spec
   it claims to follow.
4. **Performance** — DFA construction cost; long pattern, long path
   should not be quadratic.
