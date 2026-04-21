# VCS Test Suite — Native SQLite (loom)

Nо́рмативная test-suite для `core/database/sqlite/native/`. Спецификация:
`internal/specs/sqlite-native.md`.

## Layout

```
L2-standard/database/sqlite/
├── README.md                   — этот файл
├── l0_vfs/                     — Layer 0: VfsProtocol, PosixVfs, MemDbVfs, locking
├── l1_pager/                   — Layer 1: Pager actor, page cache, WAL, rollback journal
├── l2_record/                  — Layer 2: record encoding, type affinity, collation
├── l3_btree/                   — Layer 3: B-tree operations, cursors, balance, integrity
├── l4_vdbe/                    — Layer 4: VDBE opcodes, program execution
├── l5_sql/                     — Layer 5: lexer, parser, planner, codegen
├── pragma/                     — PRAGMA statements semantic coverage
├── vtab/                       — Virtual tables (FTS5, JSON, R-Tree, dbstat)
├── integration/                — end-to-end scenarios (CRUD, triggers, views)
```

## Test tier policy

Каждый тест этого пакета (где implementation уже landed) должен запускаться в
обоих режимах:

- **tier 0 (interpreter)** — VBC execution via `verum run --tier interpret`
- **tier 3 (AOT)** — native binary via `verum build --release --tier aot`

Frontmatter-директива для dual-tier:
```
// @test: run
// @tier: 0 | 3
```

где `0 | 3` означает «прогнать в обоих tier-ах; оба должны дать identical
result». Runner в `vcs/runner/vtest/` поддерживает эту форму — см.
`vcs/.vtest.toml [differential] compare_tiers = [0, 3]`.

## Status markers

Пока implementation landed не для всех слоёв:

| Tag | Meaning |
|---|---|
| `@test: run` | Test runnable RIGHT NOW; должен пройти |
| `@test: wip` | Scaffold present, implementation ещё не merged; skipped runner-ом |
| `@test: skip` | Temporarily disabled; requires human reason in comments |

## Differential testing

См. `vcs/differential/sqlite/` для cross-engine (V vs C-SQLite) harness.

## Fuzz infrastructure

См. `vcs/fuzz/sqlite/` для SQL grammar fuzzer, file-format mutator,
WAL-frame fuzzer.

## Phase gates (из `internal/specs/sqlite-native.md` §22.7)

| Phase | L0 | L1 | L2 | L3 | Differential |
|---|---|---|---|---|---|
| α | 100% | 80% | 30% | — | 70% |
| β | 100% | 100% | 80% | 40% | 95% |
| γ | 100% | 100% | 100% | 80% | 99% |
| δ | 100% | 100% | 100% | 100% | 99.99% |

Любой падающий L0-тест — CI blocker.
