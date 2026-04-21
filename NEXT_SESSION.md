# Next Session — Production-Ready Push

## Snapshot at Session End (2026-04-21)

**Test status (after 14 session commits):**
- L0 stdlib-runtime: **100%** (8/8, regression-free)
- L1 full: 92.3% (1051/1139)
- L2-standard: **69.8%** (up from 66.1% baseline)
- L2/async subset: **50.7%** (24→35 passing, +50% relative)

**14 landed commits this session:**
- `a968295` Text dual-layout print
- `90bcb59` handle_clone raw-pointer safety
- `5cf7565` stdlib From<&str> → &Text
- `02dd6c8` Text list-dispatch guard
- `db0c94f` Text.from codegen intercept
- `ea77b84` module-level using [Ctx] propagation
- `41a20bf` ptr_offset tag-preserving PtrAdd
- `ea70bb8` auto-include core.async.select + spawn_config
- `a00fae0` 7 async combinators (join5, race*, try_join*)
- `b95ebd7` join_all_settled
- `b38c5c4` Regex.new → Result<Regex, RegexError>
- `465871c` spawn_with_config wrapper
- `6402320` stdlib context fallback scan registers in both resolver + checker **(biggest L2 boost)**
- `bb08b7c` method type-param substitution hardening

---

## Open Tasks (4) — Each Requires Focused Subsystem Work

### #57 Iterator generic method inference
**Concrete reproducer:** `let a = range(0,3); let b = range(10,13); let c = a.chain(b);` → `Type mismatch: expected 'Int', found 'Range<Int>'`.

**Root cause identified:** when `chain<U: Iterator<Item = Self.Item>>` method is instantiated for `Range<Int>`, the bound `Item = Self.Item` eagerly resolves `Self.Item = Int` via `substitute_self_type`'s `try_resolve_associated_type_projection` (infer.rs:23386). The associated-type resolution then pins `U := Int` before the argument `b: Range<Int>` is checked against U. Result: unify(Range<Int>, Int) fails.

**Affects:** 27 L1 iterator tests + several L2 tests using `.chain()`, `.zip()`, `.enumerate()`, etc. with generic method type params.

**Partial work in commit `bb08b7c`:**
- `substitute_type_params` on bare `Type::Generic { name, args: [] }` now substitutes by name (previously only recursed args).
- `instantiate_method_type_params` additionally collects the *original* `TypeVar` stored under each param's name in ctx.env and rewrites those vars to fresh ones per call-site.

**Needed fix (architectural):** Decouple bound constraint resolution from fresh-var allocation. When a bound `U: Iterator<Item = Self.Item>` carries an associated-type projection that references `Self`, the projection must remain *lazy* (as a deferred constraint) until U gets its concrete value from the argument — NOT eagerly resolved during signature instantiation. Likely sites:
- `crates/verum_types/src/infer.rs::substitute_self_type` (around line 23386 — `try_resolve_associated_type_projection` fires too early when base_ty is concrete).
- `crates/verum_types/src/infer.rs::instantiate_method_for_receiver` — may need to skip bound contents during Self-substitution.
- Protocol bound resolver in context — should register bounds on fresh vars as deferred predicates.

---

### #46 L2/async runtime — 39 stdout mismatches
**Status:** 35/69 passing after this session's context + combinator fixes.

**Remaining failures cluster:**
- ~20 stdout reordering (async executor scheduling nondeterminism: tasks complete in wrong order)
- Cancellation propagation through nested nurseries
- Nested async closure captures (`undefined variable: owned/i` in spawned closure body)
- `panic_in_async`, `supervision_recovery`, `try_recover_finally` — supervisor-tree error paths

**Key files:**
- `core/async/nursery.vr` — structured concurrency
- `core/async/cancellation.vr` — CancellationToken observer
- `crates/verum_vbc/src/interpreter/kernel/` — Tier 0 executor
- `crates/verum_vbc/src/codegen/expressions.rs` — closure capture lowering

Each test needs individual diagnosis. No shared root cause for all 39.

---

### #53 Stdlib `format(fmt, ...args)` variadic
**Test sites:** L2 error-handling tests use `format("released_{}", id)`, `format("Expected: {:?}", v)`, `format("{:.3}ms", nanos)` — 1-4 args, format specs `{}`, `{:?}`, `{:b}`, `{:o}`, `{:x}`, `{:.N}`, `{:NwN}`, `{:e}`.

**Two paths:**
1. **Language-level variadic fn.** No precedent in Verum — would need `fn format(fmt: &Text, ...args: Display) -> Text` grammar, AST, codegen support for variadic arg packing.
2. **Compile-time macro.** Rewrite `format("x={}", v)` at parse-time to `f"x={v}"` (which is already supported). Requires:
   - Recognize `format(...)` calls in a pre-pass
   - Extract format spec, locate argument positions, synthesize interpolation literal
   - Sites: `crates/verum_fast_parser/src/expr.rs` post-parse, or a dedicated desugar pass in `verum_ast`.

Path 2 is smaller. Path 1 is more general (supports runtime format strings).

---

### #55 MutexGuard / Result deref protocol (`*guard` syntax)
**Error:** `Cannot dereference non-reference type: Result<_, _>` at `*guard` where `guard: Result<MutexGuard<T>, PoisonError>` from `mutex.lock()`.

**Affects:** deadlock_prevention, ref_across_await, shared_state (L2/async safety).

**Design decision needed:**
- Option A: `Mutex.lock()` returns `MutexGuard<T>` directly (panic on poison, Rust 1.0+ idiom). Separate `try_lock_poisoned()` for explicit handling. Clean but breaks Rust-compat API.
- Option B: `Deref` protocol for `Result<T, E>` — `*result` auto-unwraps on Ok, panics on Err. Semantic hazard.
- Option C: Grammar/type-system change: `await?` or implicit `?` after `await` on Result-returning async fns.

Both (A) and (C) were explored mid-session; (A) breaks `Shared.new(Mutex.new(...))` through Shared's own Result wrapping. Orthogonal stdlib cleanup needed first.

---

## Reproducer Files (keep for next session)

- `/tmp/test_chain_min.vr` — `range().chain(range())` — #57
- `/tmp/test_fs3.vr` — module-level using — already fixed (#58)
- `/tmp/test_simple.vr`, `/tmp/test_direct.vr`, `/tmp/test_final.vr` — Text tests

## Investigation Shortcuts

- `VERUM_UNIFY_DEBUG=1` — prints unification failures (revert after use, currently not in tree)
- `VERUM_CTX_DEBUG=1` — prints `check_item: FileSystem defined=true/false` (currently not in tree)
- Archive inspection: `zstd -d -c target/release/build/verum_compiler-*/out/stdlib_archive.zst | grep -ao "public context [A-Z][a-zA-Z]*" | sort -u`

## Regression Baseline

Run after every significant change:
```bash
target/release/vtest run -l L0 vcs/specs/L0-critical/stdlib-runtime
```
Must stay at 8/8. If it drops, revert.
