# Checked array-count owners

T1627 source contract and validation, 2026-10-07.

A qualified checked array count selects its nearest declared module head before
looking up the remaining path. If that owner lacks the requested member, the
count is unresolved; it cannot borrow a same-named member from an outer module.
This applies to empty and forward-declared modules as well as modules containing
constants. Ownership comes from declarations and selected file identity, not from
constant keys or suffix matches. See [name resolution](name-resolution.md),
sections 3.1–3.3.

`cog.outer.ns.CAP` keeps its explicit root. A local value named `ns` does not
replace the module namespace. A module constant named `size` takes precedence
over the layout of a same-named type; a genuine type member such as
`ns.Item.size` retains its layout query.

The compiler supplies the checked-count prepass with the parsed file and its
installed file scope. The first named bodiless top-level module declaration identifies the
file header; it is not declared again as a child. Later and nested forward
module declarations remain real owners. The checker records header identity
using its source span, including file identity, and its complete declared name.
File-derived scopes and explicit `cog` paths are normalized only in the checked
count domain. Ordinary namespace registration and rich meta evaluation retain
their existing behavior.

The VBC producer retains complete source scopes during nested traversal. Selected
constant imports keep their lexical scope separately from qualified declaration
keys. This preserves the supported explicit import path without granting a
foreign global alias ownership of a local count.

## Evidence

The source checkpoint is `61d25571d7e9d0ec4c19d88038c3b47b6a7716e7`, tree
`18f695e0a6566e7c4753c5615a319203193d7d37`. Independent read-only peer review
accepted that checkpoint. No source changed during its final full-library gate.

Causal controls first reproduced the outer-owner capture, the `ns.size` layout
collision, ignored forward declarations and a file header incorrectly redeclared
as a child. The real compiler `run_check_only` path reported E400 for
`outer.inner.CAP` in `src/outer/inner.vr` before the file-aware correction. The
corrected compiler accepts both `outer.inner.CAP` and `cog.outer.inner.CAP`.

The compiler check used a verified schema52 artifact snapshot produced by
`fdad59f06114fd2770f3d53058373764284c7c2e`. Its runtime archive SHA-256 is
`410ff097b455437cab98b651f7799b3129588820925afe98e0de9c3781530a0f`;
metadata SHA-256 is
`a92bd89e649e102cdc67d7ad0032513d6448d8915e5744127e6cdf1f1e34ebbb`.
The isolated test disabled automatic precompilation when rebuilding its compiler
reader. An earlier incompatible private metadata blob failed decoding before
semantic checking and contributes no language-behavior evidence.


| Gate | Observed result |
| --- | --- |
| Checker count, conditional and inline-mount targets | 27 passed, 0 failed |
| VBC owner, conditional, named-count, layout, structural-generic and nominal-dependency targets | 85 passed, 0 failed |
| Real compiler file-header target | 1 passed, covering both qualified spellings; 37.30 s |
| Full VBC library with `codegen,ffi`, no test filter | 2,063 passed, 0 failed, 1 existing T0839 ignore; 1,024.68 s test runtime |
| Public reference hygiene and whitespace | Passed |

The full-library invocation took 1,051.82 seconds including compilation. Its
existing ignored case is `test_compile_stdlib_coverage_report`, tracked by T0839.
The 85 focused VBC checks ran at `895501463`; the final checkpoint subsequently
changed checker/compiler file preparation only. The full VBC library gate and
real compiler acceptance ran on the final source checkpoint above.

Reproduction commands, using a private `CARGO_TARGET_DIR` and the configured
`VERUM_LLVM_DIR`:

```sh
cargo test --locked -p verum_types --test checked_count_owners \
  --test conditional_array_count --test inline_module_registry_mount \
  -- --test-threads=8
cargo test --locked -p verum_vbc --no-default-features \
  --features compression,table_dispatch,codegen,ffi \
  --test checked_count_owners --test conditional_array_count \
  --test named_array_count --test structural_generic_calls \
  --test generic_layout_witness --test bootstrap_nominal_dependencies \
  -- --test-threads=8
cargo test --locked -p verum_vbc --lib --no-default-features \
  --features compression,table_dispatch,codegen,ffi -- --test-threads=8
# Isolated compiler-reader control with the verified artifact snapshot above:
VERUM_NO_AUTO_PRECOMPILE=1 cargo test --locked -p verum_compiler \
  --test checked_count_file_owners -- --test-threads=8 --nocapture
```

The retained local log identities are:

| Log | SHA-256 |
| --- | --- |
| `T1627-compiler-header-coherent-red.log` | `cb3997449953818b5fc70882966a79b6b3d82e17e1d5f93ebfbb479df194b6b7` |
| `T1627-checker-file-owner-final.log` | `e0f6cbae8048c6a870ff5a6ffe1c1786e8c842303010bf96092a37474ce58867` |
| `T1627-vbc-focused-header.log` | `5a02049a5347adb08c19844905b28169120500e08cfa3676c13a6f3eb04a0df6` |
| `T1627-compiler-file-owner-final.log` | `2a215e1df33775c0aebd00989c04f27da2efccfa12241375730c837c2385543e` |
| `T1627-vbc-full-final.log` | `aa26b22c5c5af9a2e7e51d12543dfe46fedd241a47c565383057353b06db3da2` |

## Validation boundaries

The source gates exercise checker refusal and exact counts, strict VBC production,
wire round trips, interpreter results and the real file-backed compiler check
route. They do not establish a fresh ordinary CLI/stdlib bake or native AOT
acceptance. The separate ordinary short-path gap (T1628), nested inline-error
propagation (T1630), and pre-existing global import/source-order precedence
(T1631) remain open.
