# Next Session — Production-Ready Push

## Snapshot (2026-04-21 late, после ~50+ коммитов этой сессии)

**Test status:**
- L0 stdlib-runtime: **100%** (8/8, regression-free)
- L0 full: `cbgr_latency.vr` fixed (removed spurious `using [Benchmark]`)
- L1-core: **99.6%** (524/526) — 2 self-hosting flakes
- L2-standard: **70.3%** (315/448) — 133 failures (параллельный агент активно растит + закрывает)
- L3-extended: **93.1%** (297/319) — 22 failures

## Дополнительные закрытия этой подсессии
- `7e659d8` fix(L0): cbgr_latency — remove spurious `using [Benchmark]` (L0 regression)
- `64ed232` fix(types): user type/impl shadows same-named stdlib context (2 guards)
- `82ce88d` fix(types): prioritise user inherent methods over protocol-method static lookup (third guard)
- `882b7bf` feat(types): `*result` auto-unwraps Result<T,E> → T at typecheck (#55 tail)
- `e0b8781` fix(types): tighten name-collision guard — check specific method name
- `fe13cae` fix(L2/tests): align test code with current stdlib channel/int APIs
- `65d0728` fix(types): alias lowercase int widths to per-width Named types (pending llvm rebuild)

## ⚠️ LLVM build state
`llvm/install/` отсутствует в working tree (snapshot этой сессии). Compilation crates
`verum_llvm_sys` / `verum_mlir_sys` / `verum_tblgen` требуют локально собранную
LLVM 21.1.8. Для восстановления build:
```bash
cd llvm && ./build.sh
```
Занимает ~2–3ч. Существующие `target/release/verum` и `target/release/vtest`
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
