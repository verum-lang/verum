# SQLite Fuzz Infrastructure

Fuzz корпус и runner-ы для loom native SQLite. Все fuzzer-ы являются
оппортунистическими — падение означает либо корректный `DbError::Corrupt` (OK),
либо panic/segfault (NOT OK).

## Layout

```
fuzz/sqlite/
├── README.md
├── grammar/                    — SQL grammar-guided mutator (generate valid SQL)
│   ├── grammar.peg             — SQLite SQL grammar
│   ├── runner.vr               — harness entry point
│   └── corpus/                 — accumulated interesting inputs
├── format/                     — file format mutator (bit-flip, truncation, injection)
│   ├── runner.vr
│   └── corpus/
├── wal/                        — WAL frame sequence mutator
│   ├── runner.vr
│   └── corpus/
└── corpus/                     — aggregated crash seeds
    ├── ...                     — indexed by hash
```

## Running

```bash
# Grammar fuzz (generates SQL, executes, checks no-panic):
verum run vcs/fuzz/sqlite/grammar/runner.vr --duration 30m --seed 0

# Format fuzz (mutates valid DBs, opens, checks no-panic):
verum run vcs/fuzz/sqlite/format/runner.vr --duration 30m --seed 0

# WAL fuzz (mutates WAL frames, recover, checks consistency):
verum run vcs/fuzz/sqlite/wal/runner.vr --duration 30m --seed 0
```

## Invariants guarded

| Fuzzer | Invariant |
|---|---|
| grammar | SQL parse succeeds OR returns ParseError; never panics |
| grammar | SQL exec succeeds OR returns DbError; never panics |
| format | Opening corrupt file returns DbError::Corrupt; never panics/segfaults |
| format | integrity_check on valid-but-edited file reports issues, not silently accepts |
| wal | Truncating WAL at random offset → recovery produces valid state |
| wal | Bit-flipped WAL frame with bad checksum → recovery ignores it |

Any panic, OOM, deadlock, or silent-accept of corrupt input — CI blocker.

## Integration

CI runs 30-минутные sessions per-commit. Full 24-hour runs weekly on
dedicated hardware. Seeds добавляются в corpus на новых crashes.
