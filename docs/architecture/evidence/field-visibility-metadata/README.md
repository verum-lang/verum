# Declaration field policy through archives

Source field policies now survive parsed VBC generation, versioned wire encoding,
archive import/linking, compiler archive projection, and CoreMetadata bincode.
The complete policy is independent of the old coarse visibility value. An absent
policy is unknown; a legacy `is_public` bit cannot authorize field access.
Checker enforcement is the separate T1713 consumer boundary.

[The manifest](manifest.json) pins the source range, commands, raw log hashes,
unchanged embedded artifacts, and 6,482 source-at-commit hash checks. The original
logs and failed controls are retained byte for byte. Executables remain at the
recorded scratch paths with their hashes; they are not checked into the repository.

The focused-control source endpoint is
`71f8e7f9fad8340f135798ffc58dd99a1e3146fd`. Those gates last executed
`66a8b9f3eba35a58432502161bfedd8cc2355439`; the intervening commit changes
indentation only (`git diff -w` is empty). The subsequent full library gate is
recorded below.

## Causal controls and completed gates

| Boundary | Recorded result | Evidence |
| --- | --- | --- |
| Parsed source to VBC and decoded wire before repair | 1 passed, 6 failed; restricted fields became coarse Public | [Baseline](vbc-baseline/result.json) |
| Initial compiler archive baseline | Failed at an incorrect optional-origin assertion; not visibility evidence | [Initial harness result](compiler-baseline/result.json) |
| Corrected actual bootstrap/archive/metadata baseline | 0 passed, 1 failed; all eight nonpublic fields became public | [Causal baseline](compiler-causal-baseline/result.json) |
| Declaration scope normalization | 7 passed | [AST gate](scope-first/result.json) |
| First expanded parsed fixture | 1 passed, 8 failed before lowering because leading relative visibility syntax was refused | [Parser boundary](vbc-first/result.json) |
| Valid parsed source and decoded wire | 9 passed | [Fixed producer](vbc-fixed/result.json) |
| Legacy and malformed policy codec | 2 passed | [Codec gate](codec-fixed/result.json) |
| Actual bootstrap, archive, metadata, and bincode | 4 passed | [Compiler gate](compiler-fixed/result.json) |
| Producer plus archive import/link pool remapping | 11 passed | [Import/link gate](import-link/result.json) |
| Adjacent archive, format, function-body, and resource controls | 28 passed | [Adjacent gate](adjacent/result.json) |
| Actual producer fingerprint function | All three new producer inputs change the digest and declare rerun dependencies; unrelated documentation does neither | [Before](producer-cache-baseline/result.json), [after](producer-cache-fixed/result.json) |

The [valid source fixture](valid-source.vr) carries twelve ordinary fields and
four named variant fields. Its policies include private, public, cog, super,
explicit module scopes, internal, and protected. The compiler controls also cover
same-leaf declaring owners in both archive orders and canonical span-free scope
paths. Import/link controls insert unrelated destination strings before checking
scope identity and a second wire round trip.

## Compatibility and remaining boundaries

VBC 2.25 carries the full optional policy. A 2.24 field decodes with unknown
policy without consuming bytes belonging to the following field. Invalid tags
and truncated scope operands are refused. The generic VBC visibility enum has
not acquired field-policy semantics.

CoreMetadata adds `declared_visibility: Maybe<verum_ast::Visibility>`. Its bincode
shape requires a fresh `v54-2026-10-10-declared-field-visibility` product; serde
defaults do not make old bincode sidecars compatible. The build fingerprint now
covers the AST normalizer, field-policy lowering, and metadata shape as actual
producer inputs. This evidence does not include a fresh bake or ordinary CLI,
complete registry, or AOT acceptance.

The [original expanded fixture](relative-parser-baseline.vr) remains as evidence
for T1716: the grammar permits `public(in .scope)`, but the parser currently
refuses the leading dot. The AST normalizer separately verifies that a relative
scope and the corresponding `super` scope agree. The source fixture keeps the
supported `super`, `self`, `cog`, and absolute spellings. No grammar or parser
behavior was changed to conceal that independent gap.

Scoped paths are resolved under the declaring module. `self` keeps that owner;
leading relative and `super` move to its parent; `cog` starts at its first
segment. Escaping the root is refused. Normalization does not consult importer
state, export aliases, or a second module graph. Access decisions remain the
consumer's responsibility, including owner-local access and unknown-policy
refusal.

The whole-tree internal-reference/citation and dead-module-call gates passed.
[Source-gate logs](source-gates/result.json) retain the unfiltered staged
whitespace result: raw Cargo output ends with blank lines, and the retained
patch contains context lines. Those bytes were preserved. [Authored source and
prose whitespace checks](source-gates/authored-result.json) passed separately.

## Unfiltered VBC library gate

The subsequent [full gate](full-vbc/manifest.json) passed on frozen source
`875589c743c84a500252b991d46118ad566251c6`: 2,107 passed, zero failed,
one existing ignored test, and zero filtered tests. The command ran once with
`--lib --no-default-features --features compression,table_dispatch,codegen,ffi`
plus locked/offline Cargo resolution. Wall time was 534.449 seconds; libtest
reported 499.59 seconds. No retry, new ignore, solver build, or automatic
precompile was used.

The [raw receipt](full-vbc/result.json) records the complete command and
inherited test flags, 761 source hashes, source tree identities, an immutable
executable clone, and all five unchanged embedded artifact hashes. Input source
and Git state remained frozen throughout. The Client target was then released
with [no remaining owned process](full-vbc/release-selection.json).

The six socket controls that failed in the earlier signed-array full gate all
passed in this run. Their relevant production and fixture files are
byte-identical between the two sources, as recorded in the full-gate manifest.
This comparison does not establish why the earlier run failed. The earlier raw
failures remain preserved in their original evidence.

This unfiltered library pass complements the focused producer gates. It does not
execute the separate checker privacy consumer or establish fresh-bake, ordinary
CLI, whole-registry, or AOT acceptance.
