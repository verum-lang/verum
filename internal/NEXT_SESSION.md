# Next Session — Production-Ready Push

## Snapshot (2026-04-22, module-aware types chain + context bindings)

**Test status:**
- L0 stdlib-runtime: **100%** (8/8, regression-free)
- L0 baseline (lexer + parser + builtin-syntax): **332/332** (100%)
- L1-core: **99.6%** (533/535) — +4 from DataBuilder export
- L1-core/types: **100%** (172/172) — clean after data_type.vr fix
- L2-standard/async/channels: **100%** (8/8) — broadcast_stream.vr closed
- L2-standard/iterator: **100%** (1/1) passes
- L2-standard/contexts: **60%** (27/45) — +1 from named-binding fix,
  residuals are DI-runtime / closure-capture not type-system
- L3-extended: **90.0%** (287/319) — 32 residuals (mostly FFI/WASM/timeouts)

## New in this session — module-aware type resolution chain

Three sequential architectural commits closed the class of
"same-named stdlib type" collision bugs at its root:

1. **`1ee51053` feat(types): module-scoped type resolution for
   same-named stdlib types** — `define_type_in_current_module` now
   publishes every type under both unqualified and fully-qualified
   (`{module_path}.{name}`) keys. `resolve_type_name` prefers the
   qualified-name hit. All 4 stdlib-registration call-sites in
   `pipeline.rs` (bootstrap + user paths × `compile_core_module_from_ast`
   / `analyze_module` / `phase_type_check`) set `current_module_path`
   per module before the inner loops. `resolve_type_body` for aliases,
   empty/non-empty records, and variants (placeholder + final) routes
   through the module-aware helper. Types now resolve correctly inside
   their declaring module regardless of registration order.

2. **`339abe69` fix(types): pin source module path during cross-file
   impl block import** — the residual dispatch bug from commit 1
   turned out to be in `import_impl_blocks_for_type`. That function
   re-parses each impl block's method signatures via
   `ast_to_type_lenient` and writes the resolved `TypeScheme` into
   `inherent_methods[(TypeName, MethodName)]`. Before this commit it
   left `self.current_module_path` untouched — so bare type
   references inside the *source* module's impl bodies
   (e.g. `fn poll_next(...) -> Poll<Maybe<Result<T, RecvError>>>`
   inside `core.async.broadcast`) fell back to the flat
   `ctx.type_defs` lookup, where whichever same-named type was
   registered last (QUIC's `RecvError`) silently won — overwriting
   the correct method scheme written by the direct
   `register_impl_block` pass. New
   `import_impl_blocks_for_type_in_module(ast, type_name,
   source_module_path)` pins `current_module_path` to the source for
   the duration of the import and restores at exit. Two callers
   (hit + fallback) forward the resolved source path. `broadcast_stream.vr`
   now passes.

3. **`8a076ccf` fix(types): pin source module path during cross-file
   type import too** — mirror of #2 for `import_types_from_module_ast`
   which processes record-field / variant-payload types in the source
   module's scope. New
   `import_types_from_module_ast_in_module(ast, source_module_path)`.
   Closes the same architectural hole on the type-side.

4. **`4f23dfe6` fix(stdlib/base): expose DataBuilder as public API**
   — `type DataBuilder` + all 5 methods (new / new_array / set / push /
   build) were missing the `public` modifier, so user code importing
   `core.base.data.{Data, DataBuilder}` got E401 "cannot find". Small
   fix; closes L1 `data_type.vr` and exposes the builder for users.

6. **`bc15b674` fix(vbc/codegen): propagate let-annotation type name
   for variant dispatch** — closes "undefined variable: None" VBC
   codegen error on `let x: Maybe<Int> = None;` patterns. Multiple
   stdlib types register variants named `None` (`Maybe`, `tls`,
   `widget.block`, `mesh.xds`, ...). The cross-type collision set
   strips the simple-name entry so both types must use qualified
   forms. In a let-binding with an explicit annotation, the VBC
   codegen now pushes the annotation's base type name as
   `current_return_type_name` for the duration of the initializer
   compile, so `find_function_by_suffix(".None")` picks
   `Maybe.None` over the other stragglers. Mirrors
   `push_field_type_context`.

7. **`6ef11144` fix(vbc/codegen): propagate param-type + assert_eq
   context for variants** — extends the same propagation to two more
   sites: regular function call args (uses
   `FunctionInfo.param_type_names[i]`), and `assert_eq(a, b)` (reads
   the first arg's `variable_type_names` entry). Closes
   `assert_eq(err_value, None)` where `err_value: Maybe<Int>`. This
   unblocks multiple L2 error-combinator tests at codegen (some still
   fail at runtime on unrelated method dispatch).

8. **`d322396a` parser: accept `cofix fn` as a top-level item** —
   `parse_item` didn't dispatch `Cofix` as an item-starter, only as
   a modifier after `async`/`pure`/etc. Adds a direct
   `Cofix → parse_function` arm mirroring `Pure`/`Extern`. The
   copattern L3 test now parses cleanly (inference still needs
   separate work for copattern bodies — known coinductive limitation).

9. **`40cdafc3` fix(vbc/codegen): extend variant disambiguation to
   assignment targets** — closes `f.value = None;` and `x = None;`
   for `Maybe`-typed targets. Extracts the target's nominal type
   before compiling the RHS (from `variable_type_names` for path
   targets, `field_type_name` for field targets), pins
   `current_return_type_name` for the duration, restores after.
   Same save/set/restore discipline as let-annotation and arg-type
   overrides.

10. **`d63c12eb` fix(vbc/codegen): pin closure return type for
    variant disambiguation** — closes `|| -> Maybe<Int> { None }`.
    Initial version unconditionally overrode on closure entry.

11. **`c18b014e` fix(vbc/codegen): let(fn-type) + preserve-on-no-
    annotation in closures** — revises 10: (a) `compile_let`
    descends into `TypeKind::Function` return type when picking the
    hint, so `let f: fn() -> Maybe<Int> = || None;` routes the
    closure body correctly; (b) `compile_closure_body_*` now
    preserves the caller's hint when the closure has no inline
    annotation — the let-binding / param / assignment push IS
    exactly what a bare-body closure should inherit. Still saves
    and restores around sibling closures.

12. **`24b3fad7` fix(vbc/codegen): descend into collection wrappers
    for variant hint** — `let xs: List<Maybe<Int>> = [None, Some(1)];`
    used "List" as the hint; the elements need "Maybe". Single-arg
    container wrappers (`List`, `Array`, `Vec`, `Slice`, `Set`,
    `HashSet`, `Heap`, `Shared`, `Weak`) now descend into element
    type. `Maybe` is explicitly NOT in the list — that must yield
    "Maybe" as hint.

13. **`5f51b62f` fix(types): meta/const/HKT generic params counted in
    alias arity** — `type SquareMatrix<T, N: meta Int> is Matrix<T,
    N, N>;` + `SquareMatrix<Float, 3>` failed with "expects 1 type
    argument(s), but 2 were provided". The alias-path
    `__type_params_{name}` registration in
    `resolve_type_body` filtered by `GenericParamKind::Type` only,
    dropping the `N: meta Int`. Fix: include every positional-name
    kind (Type, HigherKinded, KindAnnotated, Const, Meta, Context)
    so the arity check sees all of them. Lifetimes stay excluded.
    Matches the sibling fix in `register_type_declaration_inner`.

14. **`e67d3a1d` fix(codegen/llvm): guard RefSlice base-int
    conversion against int-typed source** — the differential AOT
    harness panicked in VBC sub-op 0x00 (RefSlice) because
    `src.into_pointer_value()` asserted on an `IntValue` source
    (the base register was loaded as `i64` from a List buffer).
    Fix: check `is_int_value()` first, use the integer directly as
    the base address, only round-trip through a pointer when the
    register actually carries one. Unblocks L2 differential tests
    that touch slice lowering. Other 52 `into_pointer_value()`
    call sites left alone — only the reported panic path is
    changed.

15. **`dcf48ac9` fix(stdlib): impl → implement across 15 net modules
    + demethod H3Client.*** — batch-fix for the "'impl' is not a
    Verum keyword" stdlib parse warning that appeared at every
    check/run. Fifteen `core.net.*` modules (H3 server + request +
    qpack, QUIC transport/connection_sm/recovery, TLS 1.3 psk +
    early_data) had lingering Rust-style `impl` keyword instead of
    Verum's `implement`. Every leading `^impl [</blank>]` occurrence
    flipped via a Python regex pass.
    Plus demethoded two H3Client static functions that used a
    non-grammatical `public async fn H3Client.connect(...)` form at
    module level. Grammar requires `fn <identifier>`, not
    `fn Type.method`. Renamed to `h3client_connect`/
    `h3client_connect_resumed`; real H3Client methods
    (get/post/send) remain in their existing
    `implement H3Client { ... }` block.
    Net effect: parse-error warnings during stdlib load now silent;
    the warnings were masking real diagnostics for every user
    check.

16. **`05218652` fix(stdlib): rename SemVer/JsonPointer `format` →
    type-specific names** — two stdlib modules shadowed the built-in
    `format("...", args)` string formatter with type-specific
    `public fn format(v: &SemVer) -> Text` / `(p: &JsonPointer)`.
    Any module that transitively imported them couldn't use
    `format(...)` at all — every call got routed through the
    SemVer/JsonPointer branch producing "expected 'SemVer', found
    'Text'" (or 'JsonPointer'). Renamed to `format_semver` /
    `format_json_pointer`. Architectural rule: stdlib public
    function names must not shadow the language's compile-time
    built-ins (`format`, `print`, `panic`, `assert`, etc.). L2
    modules impact: 31/41 → 37/41 (+6).

**L1 impact:** 533/535 → 534/537 (same 3 known residuals:
higher_kinded HKT infer, sha256 stdlib, AOT field-offset panic in
vtest harness). **L3 dependent impact:** 48/52 → 50/52 — closes
`refinement_desugaring.vr` and `type_level_arithmetic.vr`. L0
stdlib-runtime 8/8 throughout.

5. **`d25585cb` fix(types/context): named context bindings + lenient
   method type build** — closes `log.info("x")` typecheck failure on
   `using [log: Logger]` patterns. Three cooperating parts:
   * Stdlib context pre-registration pipeline path now pins
     `current_module_path` to each file's module path before the
     archive scan. Without this, `build_context_type_from_decl`
     resolved bare type references under `cog` (user default) where
     qualified names aren't populated — so `LogLevel` inside
     Logger's methods was never found.
   * `build_context_type_from_decl` uses `ast_to_type_lenient` for
     method parameter/return types so missing-sibling types fall
     back to `Type::Unknown` per-field instead of wiping the whole
     method set via the outer record fallback.
   * The shadow-guard in the named-context binding loop
     (`infer.rs:~34327`) now treats
     `self.context_declarations.contains_key(context_name)` as
     "this is a context, not a user type shadow" — previously the
     guard mistook the context's own `Named { path: Logger }`
     placeholder for a user type and skipped the alias binding.
   * Cross-file import paths at `compile_core_module_from_ast` +
     `analyze_module` + `phase_type_check` pin `source_module_path`
     around `build_context_type_from_{protocol,decl}` calls.

   Residual context-test failures are all **runtime/DI** (null
   pointer in `@injectable`), **closure-capture lifetime**, or
   expected-error tests — not type-system bugs.

## Investigation trace for commit 2 (worth preserving)

The debug trace at registration/lookup time showed three writes to
`BroadcastReceiver.poll_next`:
```
WRITE@import_impl_blocks ty=fn(&mut Context) -> Poll<Maybe<Result<_, RecvError>>>   # unresolved
WRITE  module=core.async.broadcast ty=fn(&mut Context) -> Ready(None | Some(Ok | Err(Closed(Unit) | Lagged(Int)))) | Pending(Unit)   # correct
WRITE@import_impl_blocks ty=fn(&mut Context) -> Poll<Maybe<Result<_, FinalSizeChanged | ResetAfterFin | ...>>>   # WRONG — QUIC
```

After commit 2, the third write lands with the correct broadcast
variants (`Closed(Unit) | Lagged(Int)`). Key lesson: any path that
walks a foreign module's AST and resolves bare type references MUST
pin `current_module_path` to that foreign module first. Otherwise
the flat `ctx.type_defs` "last-registered wins" semantics silently
crosswire the type.

## Дополнительные закрытия этой подсессии
- `7e659d8` fix(L0): cbgr_latency — remove spurious `using [Benchmark]` (L0 regression)
- `64ed232` fix(types): user type/impl shadows same-named stdlib context (2 guards)
- `82ce88d` fix(types): prioritise user inherent methods over protocol-method static lookup (third guard)
- `882b7bf` feat(types): `*result` auto-unwraps Result<T,E> → T at typecheck (#55 tail)
- `e0b8781` fix(types): tighten name-collision guard — check specific method name
- `fe13cae` fix(L2/tests): align test code with current stdlib channel/int APIs
- `65d0728` fix(types): alias lowercase int widths to per-width Named types
- `45dbcae` fix(vbc/codegen): accept lowercase FFI int/float names as type path bases
- `9aae696` fix(stdlib): `impl` → `implement` across tls13/quic/x509 (6 files)
- `9548275` fix(stdlib/quic): `impl` → `implement` in three more QUIC modules (3 files)
- `d33990a3` **Revert** 882b7bf (*result auto-unwrap) — hardcoded Result type, violates
  "no stdlib knowledge in compiler" rule. Proper path: stdlib adds
  `implement<T, E: Debug> Deref for Result<T, E>` IF the design accepts
  panic-on-Err (это decision, не бесспорно — Rust не делает Result: Deref).
- `e85223f8` fix(tls13): leading `|` for Extension variant list (parser compat)
- `4084e4dd` fix(L2/tests): drop `&self` from context method signatures (1 file)
- `8822e569` fix(L2/tests): drop `&self` / `&mut self` from all context method sigs (17 files)
- `4cf7bad9` fix(specs): drop `&self` from context method sigs across L0/L1/L3/L4/parser (22 files)
- **`1dae2c94`** fix(stdlib): `pub` → `public` across 143 files — massive Rust→Verum
  keyword fix. Resolved every stdlib parse warning (`pub fn`, `pub type`, `pub const`,
  `pub module`, `pub mount`), unlocking core.net.h3, core.net.quic.api/transport,
  core.mesh.*, core.metrics.*, core.tracing.* that were partially broken.

## L2 context tests progress
After 4084e4dd + 8822e569 + 4cf7bad9 + 1dae2c94, L2 context runtime tests advanced
from universal typecheck fail to partial pass:
- `context_basic_provide.vr`, `context_method_call.vr`, `context_typecheck.vr`: ✓
- `context_multiple.vr`, `context_shadowing.vr`: stdout mismatch (async ordering,
  separate issue)

## ⚠️ LLVM build state — **RESTORED**
`llvm/install/` пересобран параллельным агентом. Verum compile работает.
Все commits применяются к binaries.
бинарники остаются функциональны для тестирования (L0 8/8 держится) — новые
infer.rs изменения (65d0728) не применяются до rebuild.

## Known-unfixed
- **context/type name collision для SYNTH path**: `Benchmark.new(x)` в файле с `type Benchmark` + `using [Benchmark]` всё ещё может дать неверный тип через какой-то непокрытый путь (мой minimal repro `/tmp/test_min_using.vr` всё ещё падает). Guard в register_function + method_call_inner_impl не ловит все места. Есть какой-то третий путь env.insert("Benchmark", Record) который я не нашёл без глубокого трассирования. Требует дальнейшего исследования.

## ✅ Закрыто в этой сессии

### Инфраструктурные (параллельный агент)
- `47de5d0` refactor(A,I): unified ModuleId allocator + single SoT ModuleRegistry
- `a494f83` fix(modules,stdlib): canonicalise path across loader/registry, dedup Cluster
- `a3f667e` fix(types): respect Visibility::Public in cross-module type lookup
- `eaebe4f` refactor(C,D,E,H,J): 5 architectural refinements
- `270681d` refactor(J): populate ProtocolMethod.type_param_names, exclude from subst_map
- `08e24bb` feat(F-partial): deref-aware hint for Heap/Shared<dyn Protocol>.method()
- `5b3e127` feat(B): auto-deref cascade on assignment-target field access
- `05501cd` feat(F): smart-pointer auto-deref in method resolution (infrastructure)
- **`63c61ec` fix(J): positional-alignment reordering of impl-level type-params**
  → закрывает `once_with(|| 5).map(|x| x*10).next()` — `val: Int` вместо `fn(Int)→Int`
- `fddd2e7` fix(F): post-cascade DynProtocol resolution for Heap/Shared<dyn P>.method()
- `76c9220` chore: rebrand luxquant/axiom → verum-lang/verum

### Моё в этой сессии
- **`b37d61a` fix(verum_types): correct impl_var_count для method schemes (#57)**
  → `a.chain(b)` на `Range<Int>` теперь валиден
  → добавил `resolve_bind_limit` helper, прописал `impl_var_count` в 5 сайтах `register_impl_block_inner`
- **`5b3e127` частично:** мой `format(...)` desugar был включён параллельным агентом в этот коммит
  → VBC codegen: `try_compile_format_call`, macro_expansion: `try_desugar_format_call`
  → оба делегируют в `compile_interpolated_string` — единый lowering с литеральным `f"..."`
  → поддержка `{}`, `{:spec}`, `{{`, `}}`
- **`44dd5b9` feat(stdlib/async): Sender.is_full / send_all / Receiver.recv_batch**
  → закрывает API gap для L2 channel tests
- **docs:** обновлён `internal/website/docs/language/syntax.md` — раздел про `format()`
  (файл в отдельном git repo — uncommitted, требует авторизации пользователя)

## 🔴 Оставшиеся проблемы (P0 — критично для production)

### 1. L2 async non-determinism: 41 stdout mismatch
- Порядок завершения задач в executor не детерминирован
- Nursery/cancellation ordering
- Supervision tree error paths
- **Требует индивидуальной диагностики каждого теста** — нет единого root cause
- Файлы: `core/async/nursery.vr`, `core/async/cancellation.vr`, `crates/verum_vbc/src/interpreter/kernel/`

### 2. L2 typecheck-unexpectedly-failed: 24 теста
Конкретные симптомы (grep по вчерашним запускам):
- `Cannot dereference non-reference type: Result<_, _>` — задача #55, частично покрыта `5b3e127 feat(B)` (только field access), **не покрыт `let x = *result`, `match *result {...}`**
- `Pattern expects a variant type, but scrutinee has type SendError` — variant pattern на generic newtype
- `type not found: Repository` — generic context types (`Repository<T>.find(1)`)
- `context 'unknown' (id=N) not provided` — contexts с TypeVar ID вместо имени
- `no method named recv_batch found for type Receiver<Int>` — **частично закрыто `44dd5b9`**, но остаются другие missing-method для channel (see L2 channel.vr)

### 3. L2 `undefined function: public_api / distance / None` — 10 тестов
- `mount outer.{public_api}` — inline-module + mount resolution
- Возможно закроется оставшимся ModuleRegistry refactor'ом параллельного агента

### 4. L2 null pointer dereference: 4 теста 🔴 memory safety
- Runtime падение — недопустимо для production
- Конкретные файлы надо выделить (vtest output заблокирован в предыдущем прогоне)

### 5. L2 stack overflow depth 16384: 2 теста
- Бесконечная рекурсия в type inference / protocol lookup

## 🟡 Оставшиеся проблемы (P1 — deployment blocker)

### 6. Associated-type eager resolution (частично #57)
- `try_resolve_associated_type_projection` в `substitute_self_type` (infer.rs:~23386) резолвит `::Item<T>` до того как U получит bound
- Фундаментально: bounds должны быть deferred constraints
- Закрывает: iterator-generic tests (не только `chain`, но `zip`, `enumerate`, `peekable`, `chain_many`)

### 7. Полноценный `*guard` deref для Result/MutexGuard (#55 остаток)
- `5b3e127 feat(B)` закрыл только `g.val = 100` (field assignment через MutexGuard<Inner>)
- Не покрыт: `let x = *guard;`, `match *result {...}`, передача `&*guard` в fn
- Требует: либо `Deref` protocol, либо auto-unwrap `Result` на `*` в read-mode тоже

### 8. Format specifier rendering
- `{:?}`, `{:x}`, `{:.3}`, `{:b}`, `{:e}` сейчас парсятся, но spec DISCARDED
- Для Debug/Display protocol dispatch нужен spec-aware rendering
- Место: `crates/verum_vbc/src/codegen/expressions.rs::compile_interpolated_string`

### 9. Stdlib method gaps (помимо channel)
Обнаружены грепом по L2 failure reasons:
- `SocketAddr.to_socket_addrs()` — protocol lookup fails (protocol есть, но resolution не срабатывает)
- `Sender<Int>.into_iter()` — protocol conflict или missing
- `*.east_opt()`, `*.uint64` — возможно временные или generic
- `Set.filter_map`, `Set.fold`, `Set.for_each` — "Custom iterator 'Set' missing both has_next and next methods" (warning на prelude) → **Set нужен Iterator impl**

### 10. Contexts с TypeVar IDs
- `context 'unknown' (id=22) not provided` — context resolver теряет имя, использует TypeVar ID
- Нужно: протащить имя контекста до warning/lookup

## 🟢 Желательно (P2 — quality)

### 11. rust-analyzer stale errors
Не критично для сборки, но мешает IDE:
- `expected Text, found String` в `infer.rs` (~30 мест) — несоответствие Text/String после чьей-то миграции
- `proc macro server error: Cannot create expander ... mismatched ABI expected rustc 1.96 got 1.97-nightly`
- **Фикс:** `cargo clean && cargo build` или обновить rust-analyzer

### 12. Set Iterator implementation
`core/collections/set.vr` должен получить `implement Iterator for SetIter` чтобы `for x in set { ... }` работал и stdlib не спамил warnings при загрузке

### 13. Поддержка positional/named placeholders в format()
Сейчас `format("{0}", x)` и `format("{name}", name)` не поддерживаются (только анонимные). Добавить в `try_compile_format_call` / `try_desugar_format_call`.

## Архитектурные ориентиры

### Почему generalize_ordered shadow-коллизия БЫЛА уязвима
`context.rs:1937` делает `self.lookup_type(name)` — name-based. Когда у impl и method один `F`, `define_type("F", method_var)` перекрывает `impl_F` в scope, и при lookup для quantified-list берётся method_F вместо impl_F. Параллельный агент закрыл это через **positional-alignment reordering**: scheme.vars теперь хранит impl-vars, присутствующие в `for_type` **первыми**, потом impl-vars НЕ в `for_type`, потом method-level. `impl_var_count` = размер первой группы (не всех impl). `bind_limit` даёт точное совпадение с `receiver.args.len()`. См. `63c61ec`.

### Почему `*guard` для Result требует архитектурного решения
Текущее: `*x` ожидает `Deref` protocol или встроенный reference-тип. Mutex.lock() возвращает `Result<MutexGuard<T>, PoisonError>` — два слоя. Нужно ИЛИ:
- (A) `Mutex.lock()` возвращает `MutexGuard<T>` (panic on poison), с отдельным `try_lock_poisoned()` для explicit handling
- (B) Implicit `?` после `await` на Result-returning async fn (`await?`)
- (C) `Deref` для `Result<T, E>` → `T` (panic on Err) — семантический hazard

Решение (A) самое чистое. Требует обновления всех call-sites `mutex.lock().unwrap()` → `mutex.lock()`.

## Reproducers (сохранено для следующей сессии)

- `/tmp/test_chain_min.vr` — `range().chain(range())` — **#57 решено**
- `/tmp/test_once_with_map.vr` — map на OnceWith — **решено `63c61ec`**
- `/tmp/test_format_basic.vr`, `/tmp/test_format_full.vr` — `format()` desugar — **решено**

## Investigation Shortcuts
- `VERUM_UNIFY_DEBUG=1`, `VERUM_CTX_DEBUG=1` — убраны из дерева, добавить обратно при необходимости
- Archive inspection: `zstd -d -c target/release/build/verum_compiler-*/out/stdlib_archive.zst | grep -ao "public context [A-Z][a-zA-Z]*" | sort -u`

## Regression Baseline

Каждый коммит должен удовлетворять:
```bash
target/release/vtest run -l L0 vcs/specs/L0-critical/stdlib-runtime
```
Ожидаемо: L0 8/8. **Текущее: 8/8.** ✓

## Рекомендуемый план следующей сессии

1. **Дождаться завершения ModuleRegistry refactor'а** параллельного агента (остаются P0 #3 public_api/distance/None)
2. **Associated-type eager resolution (#57 хвост)** — фикс в `infer.rs::substitute_self_type`, закрывает ~10-15 L2 typecheck failures
3. **`*guard` для Result (#55 хвост)** — выбрать дизайн (A/B/C), применить
4. **Null pointer dereferences** — триаж и фикс (memory safety, release blocker)
5. **Stack overflow** — найти цикл, ограничить/правильно завершить
6. **Async non-determinism** — сложная категория, последней
7. **Set Iterator impl** — простая задача, убирает stdlib warnings
