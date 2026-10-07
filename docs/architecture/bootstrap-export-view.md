# Borrowed bootstrap function exports

T1622 removes avoidable `FunctionInfo` clones from standard-library bootstrap
publication. The source checkpoint is `05c757c22`, integrated as `75286046d`.

## Publication contract

`VbcCodegen::export_function_view` builds the complete name-to-borrowed-metadata
view before its consumer publishes any entry. Source-owner aliases overwrite
base registry keys in the same descriptor order as the retained owned export
API. Both APIs share one alias visitor. The bootstrap clones metadata only for
vacant entries or replacements admitted by the existing stub-range predicate;
existing real entries retain their first-wins behavior.

This avoids cloning imported entries that publication discards. It still owns
export-name keys and builds the final view, so it does not eliminate all export
allocation or bound the rest of bootstrap work. The archive wire format and
cache schema are unchanged; the compiler fingerprint includes both changed
production files.

## Validation

On the unchanged source checkpoint, 52 focused VBC checks, one actual
`compile_core_module_from_ast` publication check, and the full VBC library gate
passed. The full gate passed 2,060 tests, with no failures and one existing
ignored test, using `compression,table_dispatch,codegen,ffi` with default
features disabled. Public-file hygiene passed. These gates cover owner alias
collisions in both declaration orders, complete metadata equivalence, retained
imports, callable generics, and stub replacement at the publication boundary.

The compiler unit check used `VERUM_NO_AUTO_PRECOMPILE=1` and an independent
in-memory source unit. It does not establish acceptance of a fresh embedded
standard-library archive. A normal CLI build with automatic precompilation
remains the integration acceptance step.

## Bounded measurement

The Criterion benchmark in
[`bootstrap_exports.rs`](../../crates/verum_vbc/benches/bootstrap_exports.rs)
compares the retained owned export with the borrowed final view. Registry
metadata comes from a parsed generic function; controlled prior-import entries
are added before timing. Construction and parsing are outside the timed loop.

The optimized macOS arm64 run used 20 samples, one second of warmup, and a
nominal two-second measurement window on a loaded machine:

| Prior entries added | Final export entries | Owned estimate | Borrowed estimate |
| --- | ---: | ---: | ---: |
| 1,024 | 2,245 | 1.879 ms | 0.482 ms |
| 8,192 | 9,413 | 12.903 ms | 2.627 ms |

These are export-phase estimates, not whole-build timings. No allocation count,
peak-memory reduction, CLI latency bound, or AOT performance claim is made.

```sh
cargo test --locked -p verum_vbc --no-default-features --features compression,table_dispatch,codegen,ffi --lib
cargo test --locked -p verum_vbc --no-default-features --features compression,table_dispatch,codegen,ffi --test bootstrap_callable_signature --test bootstrap_free_function_identity --test bootstrap_function_exports --test bootstrap_glue_identity --test bootstrap_intrinsic_signature --test bootstrap_nominal_dependencies --test bootstrap_scalar_identity --test bootstrap_variant_payload
VERUM_NO_AUTO_PRECOMPILE=1 cargo test --locked -p verum_compiler --lib function_export_tests -- --nocapture
cargo bench --locked -p verum_vbc --no-default-features --features compression,table_dispatch,codegen,ffi --bench bootstrap_exports -- --sample-size 20 --warm-up-time 1 --measurement-time 2
```

Use a private `CARGO_TARGET_DIR` and the configured LLVM installation. The
compiler-unit no-precompile option is confined to that isolated test and must
not be used for normal CLI acceptance.
