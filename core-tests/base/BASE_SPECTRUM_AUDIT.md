# core/base conformance spectrum — 2026-06-02

Full interp sweep of `core-tests/base/*` against an **isolated, freshly
re-baked** stdlib archive (built via the per-target archive isolation fix
`aabf08ff3`, so results reflect current codegen + stdlib, not a
race-clobbered shared archive).

## Totals: ~1749 pass / ~270 fail + 4 crashers

| submodule | ok | fail | note |
|-----------|----|----|------|
| nanoid | 45 | 0 | clean |
| semver_constraint | 56 | 0 | clean |
| serde | 28 | 0 | clean |
| snowflake | 55 | 0 | clean |
| string_distance | 60 | 0 | clean |
| coinductive | 39 | 0 | clean |
| ordering | 138 | 5 | Int.MIN cluster (see below) |
| primitives | 257 | 75 | largest cluster: Int.MIN, float edges, to_bits |
| data | 170 | 24 | |
| result | 205 | 25 | |
| maybe | 155 | 19 | |
| protocols | 142 | 22 | |
| panic | 111 | 27 | |
| ops | 94 | 8 | |
| cell | 18 | 11 | |
| coercion | 20 | 8 | |
| error | 36 | 6 | |
| glob | 32 | 4 | |
| mod | 54 | 6 | |
| retry | 38 | 5 | |
| semver | 38 | 3 | |
| ulid | 51 | 0 | rc=102 (some non-base filter noise) |
| uuid | 7 | 22 | **stdlib-bake cross-module stub (see below)** |
| **env** | — | — | **SIGSEGV (codegen/exec) in unit_test.vr** |
| **iterator** | — | — | **SIGSEGV** |
| **memory** | — | — | **SIGTRAP (rc=133)** |
| **log** | — | — | **TIMEOUT (>120s)** |

## META-FINDING (the important one)

**The per-feature CODEGEN LOGIC is correct.** Every defect probed in
isolation reproduces ONLY through the stdlib archive, never in equivalent
USER code:

- **Int.MIN=0**: a LOCAL `const VAL: Int = -9223372036854775808` compiles
  and reads correctly; only the *archived* `Int.MIN` is 0. Direct large
  literals work; boxed (>2^48) bit-ops all work. Residual is in the
  test-compile stdlib-LOAD path (core_metadata / cached ctx) dropping the
  computed `__const_val_<N>` inline marker — `archive_ctx_loader`
  (which DOES preserve it) is not the path the test runner uses.
- **uuid (22 fail)**: `Uuid.to_text` panics `[lenient] … panic-stub:
  undefined function: Text.new`. But `Text.new()` in USER code with
  uuid.vr's EXACT mount pattern (`mount core.*` + `mount base.{Text}`)
  works perfectly. The stub was baked because at precompile time, when
  `core.base.uuid` (Layer 0) was compiled, `core.text.Text.new` (higher
  layer, baked later) was an unresolved forward reference → lenient
  codegen replaced the WHOLE `to_text` body with a Panic
  (`mod.rs:13338`). `new_v4`/`new_v7` similarly stub on
  `core.sys.common.random_bytes`.

→ **Conclusion: base failures are predominantly stdlib-BAKE / cross-module
resolution / archive-plumbing defects, NOT codegen bugs.** Fix effort
belongs at the bake/archive layer, not in per-feature codegen.

## Root classes (by leverage)

1. **#47 cross-module dispatch in the bake** (uuid + likely many others):
   lenient codegen emits a panic-STUB BODY (not a name-deferred Call) when
   a cross-module call is an unresolved forward reference at bake time.
   `ArchiveBodyRemap` Tier 2b (`archive_func_by_name`) can remap deferred
   *Calls* at load but cannot fix a stub *body*. Proper fix (tracked as
   #47, see `crates/verum_vbc/src/codegen/mod.rs:5808`): encode
   cross-module refs as string IDs / emit name-deferred Calls so the load
   remap resolves them. High leverage; large; needs re-bake validation.

2. **Const inline-value propagation through the test-compile load path**
   (Int.MIN + 0.0.to_bits sister): thread `intrinsic_name=__const_val_<N>`
   through core_metadata / the cached single_module ctx. ~15 tests.

3. **4 crashers** (env/iterator SIGSEGV, memory SIGTRAP, log timeout):
   codegen/exec crashes — highest severity. `env` isolated to
   `unit_test.vr` (compiles clean via `verum check`; crashes only under
   execution-compile). Likely overlaps the unmerged `runtime-codegen-fixes`
   branch (EnvTaskId.main / graceful). `env/unit_test.vr:166` also has a
   genuine test type-error (`assert_eq(lower, u)` Int-vs-Text).

4. **float edge-cases** (~50 in primitives): NaN/to_bits/special-values.

## Validated infra this session
- `aabf08ff3` per-target archive isolation — re-bake in a dedicated
  `CARGO_TARGET_DIR` (e.g. `/Users/taaliman/vbuild`) now reliably reflects
  current codegen, immune to the concurrent-build clobbering that masked
  every prior codegen→stdlib fix.
- Discriminator technique: probe direct-literal vs local-const vs
  archive-const in one interp run to localize codegen-vs-archive instantly.
