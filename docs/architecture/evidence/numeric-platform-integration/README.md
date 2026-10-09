# Numeric and socket integration (T1701, T1650, T1708)

The tested source is `f3006f403ab059532cf45917057528683caed650`.
The unfiltered VBC library run completed in 561.245 seconds: **2,084 passed,
10 failed, one existing T0839 ignore**. Every failure is a bounded TCP
connection, UDP receive or readiness timeout tracked by T1650. The earlier
unbounded fixture hang is eliminated; this is still a failed library gate.
The host timeout cause is unresolved. No retry was used to replace that result.

The six focused binaries passed **80 controls** on the same frozen source:
31 numeric declaration/reference/gradient controls, 15 Array-to-List controls,
four source-body descriptor controls, seven bytecode compatibility controls,
seven reference receiver controls and 16 semantic-formal controls. The first
selection used incorrect Cargo target names and ran no tests; its failed
receipt is preserved separately and excluded from execution acceptance.

The Unix descriptor repair also retains its isolated causal baselines and
serial/parallel descriptor-zero controls. An independent audit verifies nine
raw logs, 1,308 source-at-commit hashes and three preserved executables. The
raw-v2 descriptor-zero case remains T1709. The compiler archive registration
control passed at `eded28614`; the complete compiler crate tree is identical
here. These results establish bounded component behavior, not a fresh product.

All source files, commits and inherited stdlib artifact hashes stayed unchanged
during the recorded executable gates. Automatic stdlib baking was disabled for
these component tests. A current ordinary CLI and its freshly produced archive,
registry hashing/authentication, and native/AOT acceptance remain outstanding.
The separately tested native array-storage candidate is outside this snapshot.
Signed narrow-array failures under T1706 also remain separate.

The website candidate `0e1fcb3` passed typechecking, internal-artifact and
freshness checks, link validation and the strict production build. Its numeric
and gradient documentation explicitly describes interpreter scope; executable
examples and existing endian/SHA limitations are unchanged.

[Manifest](manifest.json) records exact commands, source and executable
identities, failed and passing logs, independent audits and residual tasks.
