# Differential SQLite Harness — V vs C-SQLite

Сравнительная test-suite, проверяющая что Verum native SQLite (loom) даёт
идентичные результаты с reference C-SQLite на произвольных SQL-сценариях.

Нормативный обзор — `internal/specs/sqlite-native.md` §2.5 и §22.4.

## Layout

```
differential/sqlite/
├── README.md               — этот файл
├── compare.sh              — запускает одну пару (v vs c), diff-ит результат
├── cross-impl/
│   ├── sql/                — curated SQL scenarios (one .sql per case)
│   └── expected/           — golden output — ignored, generated at runtime
├── corpus/
│   ├── public/             — 50K real-world DBs (gitignored, downloaded by script)
│   └── golden/             — canonical corpus used in CI
├── scripts/
│   ├── run-sqlancer.sh     — SQLancer fuzz-campaign
│   ├── run-corpus-replay.sh— read every file in corpus/ via V, diff vs C
│   ├── run-sqllogictest.sh — SQLite test-suite TCL port via Verum-TCL runner
│   └── fetch-corpus.sh     — downloads public test DBs
└── tier-oracle/
    ├── c-sqlite            — reference sqlite3 binary (expected pre-installed)
    └── verum-sqlite        — our binary (built by VCS pre-gate)
```

## Usage

```bash
# Single scenario:
./compare.sh cross-impl/sql/001_create_insert.sql

# Full corpus replay:
./scripts/run-corpus-replay.sh

# SQLancer (requires Java):
./scripts/run-sqlancer.sh --duration 30m --seed 12345
```

## Tolerance matrix

| Aspect | Tolerance |
|---|---|
| File format bytes | 0 (bit-exact) |
| SQL result rows | 0 for T0 queries, set-equal for T1 |
| Floating-point | 1 ULP for transcendental fns only |
| Row order without ORDER BY | set-equal (sorted before compare) |

Any discrepancy outside tolerance — CI blocker. See §2.2 in sqlite-native.md
for normative tier definitions.

## Corpus sources

- SQLite's own test files (sqlite.org/testingall.html)
- Firefox `places.sqlite` (Mozilla public builds)
- iOS backup SQLite databases (open-source samples)
- npm package download manifests (public)
- Wikipedia SQLite dumps

All files public, licence-compatible with public-domain SQLite.
