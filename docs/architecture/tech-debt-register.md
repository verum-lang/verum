# Verum Technical-Debt Register

Status: LIVING DOCUMENT — single consolidated inventory of open debt
across every layer, with priorities and acceptance criteria. Produced
from a five-lane repository audit (conformance pins, compiler gap
surface, Rust hygiene, documentation drift, stdlib structure) on
2026-07-11; update rows in place as items close.

Priorities: **P0** = wrong results or masked failures reach users;
**P1** = correctness debt with a bounded blast radius or a workaround;
**P2** = hygiene/polish that slows work but does not corrupt results.

---

## A. Compiler / runtime (Rust crates)

| # | Item | Pri | Anchor | Acceptance |
|---|------|-----|--------|------------|
| A1 | **CallM const-zero degrades** — unresolved method calls lower to `dst = 0` silently (measured 4210 sites in a hello-world binary). Root cause named in-code: VBC-GENERIC-INSTANTIATION (no generic-monomorphization-context propagation). Strict gate exists but is opt-in. | P0 | `verum_codegen/src/llvm/instruction.rs:17875` (+`:9108`, `:9236`), registry `llvm/error.rs:555` | `VERUM_STRICT_MONO=1` clean on the core-tests corpus; then flip strict to default |
| A2 | ~~Tensor/GPU unknown sub-ops silently write 0~~ **CLOSED 2026-07-11**: both fallthroughs now route to `emit_unimplemented_sub_op("TensorExtended"/"GpuExtended", sub_op)` before the keep-alive const-zero — gaps surface in diagnostics instead of miscompiling silently. | ~~P0~~ done | fix commit on main (`lower_tensor_extended`/`lower_gpu_extended`) | ✅ fallthroughs report |
| A3 | **Lenient-SKIP BugClass warnings** (#166). BURN-DOWN 2026-07-12: **176 → 132 → 82 → (bakeY pending)**. bakeX confirmed −50/zero-new: arity ×5, f-string-capture class (~30 fns incl. parse_*/get_*/select/QuicServer.bind), wave-5 ×17 files. Landed since: wave-6 onion layer ×9 files (match-arm-RHS bare variants — arm-filter blind spot; single_int0_i64 = typo'd nonexistent fn → single_int(0)), wave-6b receiver-drift impls (GPUDevice ComputeDevice statics read self with no self param; is_covering dropped protocol's &self; commutativity_coherence self.act in receiver-less family), wave-6c @cfg(macos) block-scoped fd + peer_cred super.darwin.* mis-root, progressive core.-suffix resolution (resolve_core_rooted_suffix: longest registered proper suffix ≥2 segments, wired into qualified-path/field-access/method-call — closes 'undefined variable: core' in ALL expression shapes). REMAINDER classes: (a) canonical-static order (Text.new ×11, Text.from_* ×5, Channel.unbounded ×2, Duration.seconds, BinaryHeap.new — stage-1 registers them; why lookup misses needs VERUM_TRACE_FNREG bake); (c) meta const-generics N/DIMS/k ×9 (peer impl-generics-carry pillar); NEW (f) uppercase locals B/B_bar/A_scaled ×3 mis-classified in expr position (ssm.vr params — needs trace); NEW (g) proof-surface calls (axiom-as-term hott.vr ×3, protocol-field functor composition infinity_topos ×2) — verification-layer design item. Flip-to-hard-error gated on (a). || A5 | **Shared-identity / payload-extraction fork** — extracting `Shared<T>` from an enum payload copies the cell; cross-handle shared mutable state is impossible from .vr. Blocks 9 pinned tracing-pipeline tests and any stateful in-memory pattern. | P1 | pinned `[[tracing-delivery-vbc-class]]`, `[[shared-identity-vbc-class]]` | tracing pipeline pins un-ignore and pass |
| A6 | **Protocol-object (dyn) dispatch inoperative in the interpreter** — all receiver forms mis-bind. Stdlib now routes through closed-world handle enums; `…Custom(Shared<dyn …>)` seams are dormant. | P1 | probes in memory session file; vcs `#76` (AOT wrong-impl dispatch) | a `Shared<dyn P>` method call dispatches correctly on both tiers |
| A7 | **Bake nondeterminism (string-dice)** — same sources produce different stub-name resolution across bakes; AOT suite results fluctuate run-to-run. Root localized (on_signal/spawn_detached bare-vs-#arity flip). | P1 | `VERUM_TRACE_STRDICE`; memory: p2-string-dice | two consecutive bakes byte-identical; AOT suite counts stable |
| A8 | **AOT reference-return classes** — `&Text`/`Maybe<&T>` returns crash or mis-read under AOT; stdlib worked around via owned-copy accessors. | P1 | tracing/time sessions; sub-op audit | reference-returning accessor test green under `--aot` |
| A9 | **FFI placeholder arms** — `GetLibrary` returns null "for now", `IsSymbolResolved` hardcodes true, 9 MachVm*/MachSem* arms emit const-zero unconditionally, CreateCallback passes through. | P1 | `instruction.rs:26268`, `~27523`, `~27599` | real implementations or explicit unimplemented markers |
| A10 | **SIMD scalar-width-1 degrades** — 30+ SIMD arms lower as documented scalar fallbacks (stores are no-ops), i.e. SIMD is not SIMD under AOT. | P1 | `instruction.rs:23246` family | either real vector lowering or loud unimplemented |
| A11 | **Tier-0 futex** — ~~FFI gap~~ ROUTED 2026-07-12: platform futex_wait/wake wrappers now call the canonical intrinsics (InlineSequence FutexWait/Wake → Tier-0 futex_park parking table; AOT verum_futex_* shims). Raw __ulock/SYS_FUTEX/WaitOnAddress FFI left the Tier-0 hot path. PENDING: barrier/condvar suites re-run + un-@ignore of terminate/notify pins. | P1 | INVENTORY rows | barrier/condvar suites green under `--interp` |
| A12 | **CSPRNG intrinsic gap for `fill_secure` consumers** — bloom/backoff/reservoir pins; `random_u64` exists, buffer-fill path doesn't resolve cross-module. | P1 | audit-pins class 5 | bloom/backoff/reservoir pins un-ignore |
| A13 | **verum_types unreachable-pattern arms (9)** — real logic smells in inference. | P2 | `infer/expr.rs:7892`, `infer/modules.rs:12831`, `infer/types.rs:6531–6605` | warnings gone via fix (not allow) |
| A14 | **Codegen panic surface** — 501 production unwrap/expect in `llvm/` (instruction.rs 319, runtime.rs 156): malformed VBC crashes the compiler instead of a diagnostic. | P2 | audit-warnings §3 | builder unwraps in top-10 hottest paths converted to diagnostics |
| A15 | **Blanket `#![allow]` hides dead code** — verum_codegen/verum_verification suppress everything; ~29 confirmed-dead pub fns in `llvm/` (SIMD/bitfield/cbgr/wasm clusters). | P2 | audit-warnings §1–2 | blanket allows removed; dead fns deleted; warning budget real |
| A16 | **Cross-compile host-default risk** — `link.rs:550 for_host()` is the default no-libc config path; verify `--target` overrides on cross builds. | P2 | `verum_codegen/src/link.rs:550`; callers `verum_compiler/phases/linking.rs:244,325` | cross-build test proves target config wins |
| A17 | **`static mut` cells >8 bytes fall back to TLS; non-zero initializers need ctors** (open work inside intrinsic-contract Rule 4); CBGR bound-check `1<<24` is a stated workaround pending NaN-box tag (Task F4). | P2 | intrinsic-dispatch-contract.md Rules 4–5 | wide cells + initializers native; TAG_CBGR_REF lands |

## B. Conformance debt (core-tests / vcs)

173 `@ignore` pins across 54 files; ~90 are genuine compiler-defect
pins collapsing into the classes below; ~40–77 are the *absence of a
network test harness* (one infra item, B4); 676 audit-gap entries are
mostly API-surface polish (B5).

| # | Item | Pri | Anchor | Acceptance |
|---|------|-----|--------|------------|
| B1 | **type_id=0 record/variant field-access OOB** — highest-fanout runtime class (btree 13, RsaError 4, runtime/recovery §F, journal 17). FRESH MINIMAL WITNESS 2026-07-11: `UdpSocket.bind("127.0.0.1:0")` → `sys.darwin.libsystem.safe_socket` panics 'field write index 4 (offset 32+8=40) exceeds object data size 8 type_id=0' on a coherent v12 build — NOT covered by the #42 three remap boundaries; survives the OSError-factory sweep, so the mis-typed construction is in the Err/Ok wrap or errno path, not the record literal. This one site gates the whole net family (77 un-stubbed loopback-ready pins). | P0 | witness: probe_udp.vr (scratchpad), backtrace safe_socket@pc=112 | UDP loopback probe green; then net pins un-ignore |
| B2 | **Dispatch classes**: iterator/Range (15), static-vs-instance (`SystemTime.now`, by-value-temporary chains), Float-Ord small-gap (~4), MUTSELF-MATCH (2). | P1 | audit-pins part 1 | per-class canary tests green |
| B3 | **Intrinsic resolution pins** — ARITH-MISSING-INTRINSICS-1 (13), INTRINSIC-GENERIC-WRAPPER-ARCHIVE-1 (5), nested-call (1), MEM-TRANSMUTE-FLOAT-1 (3). | P1 | `core-tests/intrinsics/*` | intrinsics suites unpinned |
| B4 | **net/ test harness absent** — ~40–77 pins are "requires live fixture/mock" (DNS/TCP/UDP/UNIX/TLS/QUIC/H3/WS/proxy). One loopback-fixture harness retires them wholesale. | P1 | audit-pins class 9 | harness lands; net pins converted to fixture tests |
| B5 | **676 audit.md open gaps** — dominated by encoding/* (8 of top 15); mostly Display/Eq/helper-ctor/property-test asks. | P2 | audit-pins part 3 | burn-down list per module; top-15 modules cleared |
| B6 | **INVENTORY taxonomy drift** — `stable` used 36× but defined in neither legend; never reconciles with website `undocumented`. | P2 | INVENTORY.md legend | one shared taxonomy, both legends updated |
| B7 | **vcs real blockers** — #143 Waker.drop NPE (L2), #76 AOT protocol-method wrong-impl dispatch. | P1 | `vcs/specs/L0-critical/_codegen_regressions/aot_to_socket_addrs_dispatch.vr` | both specs pass |

## C. Stdlib (.vr) structure

Scope measured: 2546 .vr files / 581K LOC; 5820 `public type` decls;
8696 mounts; 1374 `@intrinsic` occurrences. Full enumerations +
reproducible scripts: audit scratchpad (cat1_dups.txt, cat2_drift.txt,
cat4_unresolved.txt).

| # | Item | Pri | Anchor | Acceptance |
|---|------|-----|--------|------------|
| C1 | **Duplicate public type names** — 155 names declared in ≥2 files (326 decls); 64 kind-divergent genuine collisions, of which **27 are protocol-vs-record hard collisions**. **RESOLVED 2026-07-11 (26 fixed + 1 assessed-by-design)** (batches 1-4: sqlite-internal Capability family unified, Capability family unified onto database/common; CommitDecision/AuthDecision/UpdateOp/ProgressDecision deduped to their authorities; TraceEvent→TraceEventKind, BusyDecision→BusyHookDecision (ms-vs-ns unit honesty), TraceCtx→TraceHookCtx). verify-kernel cluster ASSESSED, no action: kernel_v0 (bootstrap .vr kernel formalization) and kernel_soundness (propositional carrier for theorems about the Rust proof kernel) are two INDEPENDENT formalizations whose shared vocabulary (Judgment/Context/Derivation) is domain-canonical in each; verify/mod.vr already isolates both from flat re-export (qualified-path access only, documented in-file), so the bare-name first-wins surface is designed out. Renaming formal-methods vocabulary would damage domain honesty for no dispatch benefit. (InclusionProof resolved: merkle keeps bare; Rekor/Signed wire names; wire-shape merge tracked as follow-up.) 91 same-kind twins (e.g. `Proof`, `AllocHandle`, `JoinHandleOpaque`) unify per redundancy directive. | P1 | cat1_dups.txt | remaining 16 hard collisions renamed/qualified; twins unified |
| C2 | ~~Mount drift~~ — **CLOSED 2026-07-11** (commit 7eca17515): 44 broken mounts → 0, detector re-run clean, bake-verified. | ✅ | cat2_drift.txt | done |
| C3 | **Unregistered `@intrinsic` calls** — 564 distinct names / 578 occurrences; miss = silent LoadNil (`expressions.rs:26463`) or Unit fallthrough. PROGRESS 2026-07-11 (commit 968f09f80): the LIVE silent-nil kernel rerouted to registered implementations — llvm.*.with.overflow ×6 (checked_add_u/checked_sub_u destructured nil tuples!), sincos ×3 (composed sin+cos), verum.float bit-casts ×5, verum.rng.fill_secure ×2 (→ canonical rng helper), mysql float_to_bits ×2. REMAINING: (a-residue) GPU set / SIMD compares / checked_rem/shl/shr / overflowing_neg/shl/shr / widening_mul / carrying_add / transmute / ptr_read_* / cpu_feature — need registry entries + Tier-0 handlers (Rust); (c) ~250 `verum.<host>.*` deliberate host-subsystem stubs (tls/quic/h3/k8s/xds/compress/cloud) — separate not-yet-implemented track; miss path should WARN at bake instead of LoadNil. | P1(a)/P2(c) | cat4_unresolved.txt | (a) registered with handlers; miss path warns |
| C4 | ~~Dead/drifted surface~~ — **CLOSED 2026-07-11** (f15dae9e9): AllocHandle/JoinHandleOpaque intrinsics-side alias decls deleted (Raw* canonical); `contracts_old` re-diagnosed as FALSE POSITIVE — it is the live 'contracts with old() pre-state refs' surface, retained. | ✅ | — | done |
| C5 | **Toolchain-constraint compliance** — 18 documented .vr-breaking classes (payload-extraction, guard-methods, wide literals, relative qualified statics, …); stdlib predating the map may violate them silently (the channel-ctor P0 was exactly this). | P1 | memory: tracing session constraint map | mechanical lint sweep for index-assignments, `sync.mutex.Mutex.new`-style relative statics, wide int literals |

## D. Documentation

| # | Item | Pri | Anchor | Acceptance |
|---|------|-----|--------|------------|
| D1 | **40/55 website stdlib pages violate the site's own policy** — `status_detail` frontmatters carry task#/hashes/test-counts (May-24 `task #7` boilerplate + per-page hashes); bodies of the 10 largest pages saturated. `defect-class-catalogue.md` = 101 KB internal changelog with 68 commit hashes published verbatim. | P1 | audit-docs part 1 | zero banned tokens in frontmatter; catalogue rewritten or moved in-repo |
| D2 | **Stale statuses** — `math` page says regression-only while INVENTORY shows 10 complete; `tracing` INVENTORY rows lag the rebuild; `base` detail frozen at 2026-06-02. | P1 | audit-docs part 2 | spot-check table all "consistent" |
| D3 | **CLAUDE.md crate map drift** — references 4 non-existent crates (verum_context, verum_std, verum_runtime, verum_resolve); omits 8 existing (verum_kernel, verum_syntax, …); stale key-file paths. | P1 | audit-docs part 3 | map matches `crates/` exactly |
| D4 | **Dead file refs in docs/detailed** — roadmap + type-system docs point at renamed/removed crates and files. | P2 | audit-docs part 3 | referenced paths exist |
| D5 | **Grammar sync claim stale** — 05-syntax-grammar.md claims sync with ebnf v2.10; ebnf is v3.0; doc internally inconsistent. | P2 | audit-docs part 4 | version claims match |

## E. Infrastructure

| # | Item | Pri | Anchor | Acceptance |
|---|------|-----|--------|------------|
| E1 | Shared build dir across sessions — peer relinks `target/release/verum`; acceptance runs must use session-private CARGO_TARGET_DIR (standing memory rule); peer WIP breaks `cargo build` mid-flight. | P1 | memory: shared-target-contamination | CI/iteration scripts use isolated target dirs |
| E2 | `verum check` cannot validate stdlib modules standalone (relative mounts resolve only in bake context) — bake is the only validator, 6-minute cycle. | P2 | channel.vr episode | `verum check --stdlib` mode or faster bake |
| E3 | Full-filter `verum test --filter <domain>/` intermittently SIGSEGVs the harness under parallel compile; per-suite filters are the workaround. | P2 | tracing sessions | full-filter run stable |

---

Cross-references: `docs/architecture/tier-coherence-pillars.md` (P1–P5
pillar plan retiring §40/§46/#27), `docs/architecture/intrinsic-dispatch-contract.md`
(Rules 4–5 open work), `core-tests/INVENTORY.md` (per-module truth),
memory session file `session_2026-07-09_tracing_analysis_and_docs.md`
(18 toolchain-constraint classes with repros).
