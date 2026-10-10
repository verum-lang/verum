# Explicit typed binding native gate

Frozen tested source: `4afb8b520aa9464e3ad9836c7f2119af5ee2e5b7`, tree
`4f5e576cf41b904a68626c6b9bec7f530d1f5f37`. Its two test files are unchanged
from test checkpoint `f14fba3a8ad71b9d31ba8d07a660c8587e5ccc1c`; the intervening
commit only retained the interpreter evidence.

The focused native test passed: one Rust test, zero failures or ignores,
fifteen deliberately filtered tests. Its complete matrix executed 48 scalar
probes: UInt32/Float, packed/List selected producers, inferred/explicit local
bindings, read/mutate/length operations, and parsed-source/decoded-wire routes.
The 48 distinct LLVM IR filenames were checked against the exact expected
matrix and rehashed after execution. They are preserved losslessly below.

Each source pair retains the same declared fixed-array shape but must actually
emit its intended different allocation. Value and mutation probes check the
returned scalar through its declared integer or floating-point ABI; length
probes check two elements. Packed indexing requires the native bounds guard
and refuses a generic List-header probe before JIT. This complements the
interpreter's unchanged allocation-bound negative controls.

The [receipt](result.json.gz) records exit zero in 45.371 seconds including
compilation; the test runtime was 0.87 seconds. It pins every audited source,
the test executable, environment, deadline, all IR and raw logs. Source files
and all five current archive/sidecar hashes were unchanged. There was no
solver, native toolchain or standard-library bootstrap. The exact executable
was [retained outside the build target](retained-executable.json.gz) with SHA-256
`78842d76c4e0a4d605a5e0eaef5e15c247319c42f7bc7b54952cb0bf0a6eded7`.
The exclusive target was released after the gate and identity checks.

This uses the existing bounded host allocation substrate. It establishes the
specified production LLVM lowering and JIT behavior, not allocator lifecycle,
ordinary CLI, native executable linking, AOT/no-libc, SHA-256 or registry
acceptance. Unknown/forwarded results, unsupported control flow and unproved
fixed-array arguments retain their explicit refusal boundaries. The other
fifteen native tests were not rerun by this focused command.

The [runner](run_gate.py) is the exact executed script. [manifest.json](manifest.json)
pins original and compressed receipt, logs and IR bytes.
