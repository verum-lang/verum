# Multi-session task pool — coordination protocol

Status: LIVE — this is the coordination contract for every session
(Claude agent or human) working on this repository in parallel.
Introduced 2026-07-16 (T0101). CLI: `scripts/taskpool/tp`.

## Why

This machine routinely runs several concurrent sessions against one
repo. Before the pool, each session numbered its own tasks from `#1`,
so git history contains many *different* defects sharing one `#n`
label; two sessions could silently pick the same defect; and there was
no durable, cross-session list of open work. The pool fixes all three
with one mechanism.

## Design

A task is exactly one markdown file. Its lifecycle state is the
directory it sits in. Every state transition is a same-filesystem
`rename(2)` — atomic, so a claim has exactly one winner, with no
daemon, no lock files, and no git churn.

```
<main-checkout>/.taskpool/        machine-local, gitignored
├── open/      T0123.md           available work — claim before touching anything
├── claimed/   T0123.md           exactly one session owns it
├── done/      T0123.md           finished; files stay forever
├── dead/      T0123.md           cancelled/obsolete; files stay forever
└── events.log                    append-only audit trail of every transition
```

* **Location**: resolved from any worktree of this repo as
  `$(git rev-parse --git-common-dir)/../.taskpool` (worktrees share the
  common git dir, so every session sees the same pool). Override with
  `VERUM_TASKPOOL_ROOT` (tests only). The path must not contain spaces.
* **IDs are global and monotone**: `T0101, T0102, …` — allocated by
  `tp new` as `max(existing across all states) + 1` with an `O_EXCL`
  create, so concurrent allocations never collide. Because `done/` and
  `dead/` files are never deleted, IDs are never reused. **Never invent
  an ID by hand.**
* **Owner identity**: `$TP_OWNER`, else the Claude session-id prefix
  (`$CLAUDE_CODE_SESSION_ID`), else `user-pid`.

## CLI

```
tp init                                # once per machine (idempotent)
tp new -t TITLE [-p P0..P3] [-a AREA] [-D T0100,...] [-A ACCEPTANCE] [-b BODY|stdin]
tp claim T0123 | tp claim --next [-a AREA]
tp note  T0123 -m "progress"           # journal heartbeat (also refreshes mtime)
tp done  T0123 [-m RESOLUTION] [-c COMMIT]
tp release T0123 [-m REASON]           # blocked / giving up → back to open
tp dead  T0123 -m REASON               # cancelled; ID stays burned
tp list  [open|claimed|done|dead|all]
tp show  T0123
tp status
tp reap  [-H HOURS]                    # stale claims (default >24h idle) → open
```

Priorities follow the tech-debt register: **P0** wrong results reach
users; **P1** correctness debt, bounded blast radius; **P2** hygiene;
P3 nice-to-have.

## Rules (mandatory for every session)

1. **Claim before you work.** If `tp claim` fails, someone else owns
   it — pick another task, never work an unclaimed or foreign task.
   Post `tp note` progress at least once per work block so `tp reap`
   does not recycle your claim; long builds count as progress (note
   before starting them).
2. **Commit subjects reference the pool ID**: `fix(vbc): … (T0123)`.
   The legacy per-session `#1..#56` numbers are frozen history — never
   mint new ones. On finishing: `tp done T0123 -c <sha>`.
3. **One task = one coherent unit of work.** If a fix splinters into
   independent sub-defects, file them with `tp new` (link with `-D`)
   instead of stretching one claim for days.
4. **Blocked → release.** `tp release T0123 -m "blocked on T0140"` puts
   it back with the journal intact; update `Depends:` if you learned a
   real dependency.
5. **Working-tree hygiene** (the historical collision source):
   * Never commit, stash, revert, or `git add -A` over changes you did
     not make; stage only your own paths.
   * Build/test with a **session-private `CARGO_TARGET_DIR`** (e.g.
     under your scratchpad); the shared `target/` belongs to whoever
     is baking in the main checkout.
   * Prefer a private worktree for multi-file changes; conform to
     what is LANDED on main, not to another session's unstaged state.
   * Never run two AOT suites concurrently on this machine (known
     harness constraint) — `tp note` that you are starting one.
6. **Absolute paths or `git -C` — always** (the 2026-07-17 lesson,
   hit independently by three sessions in one day): the harness may
   reset a shell's cwd into a DIFFERENT worktree between commands
   (e.g. `.claude/worktrees/<other-task>`). A relative-path
   `git status` / test run / file edit then reads or writes a foreign
   tree — a clean foreign checkout at HEAD is indistinguishable from
   "all my uncommitted work was erased", and a relative-path commit
   lands on a foreign branch. Discipline:
   * every git command names its tree: `git -C <abs-checkout> …`;
   * every file edit and test invocation uses absolute paths (or an
     explicit `--manifest-path`);
   * before ANY conclusion about tree state — and before destructive
     recovery based on such a conclusion — run `pwd` +
     `git rev-parse --show-toplevel` and check they name the tree you
     think you are in.
7. **Parallel-session edit isolation**: concurrent sessions/agents
   edit ONLY inside their own `git worktree` (branch per task); the
   main checkout is a commit/integration zone. Never run
   state-changing git (`checkout -f`, `restore`, `clean`, `stash`,
   branch switches) against a tree another session may be editing —
   if a shared tree looks wedged (conflict markers, stale locks),
   post a `tp note`, wait, and only then recover the MINIMAL scope
   with the owner's work preserved (a conflicted stash-pop keeps its
   stash entry — restoring the two conflicted files loses nothing).
6. **Task files are append-mostly**: fix typos freely in your own
   claimed task, but never rewrite another session's journal; never
   delete task files (the ID space depends on them).

## Roles

Any session may play either role; check `tp status` first.

* **Master (auditor/dispatcher)** — researches the current
  implementation and files work: reconciles
  `docs/architecture/tech-debt-register.md` rows,
  `core-tests/INVENTORY.md` deferrals, `@ignore` pins, and session
  memory into pool tasks with concrete acceptance criteria and
  evidence anchors (file:line, repro paths, commit shas). The master
  also runs `tp reap`, marks duplicates `tp dead`, and keeps the
  register rows pointing at their pool IDs. Ideally one master at a
  time; masters claim tasks like anyone else if they also fix.
* **Workers** — `tp claim --next` (optionally `-a AREA`), reproduce,
  fix at the root (per the standing fundamentality directive), verify
  on BOTH tiers (`--interp` and `--aot`), commit with `(T0123)`,
  `tp done -c <sha>`. A worker who discovers an unrelated defect files
  it (`tp new`) rather than fixing it under the current claim.

## Session bootstrap

```
scripts/taskpool/tp status          # what's in flight, who holds what
scripts/taskpool/tp list open       # available work
scripts/taskpool/tp claim --next    # take the top-priority task
```

## Relation to other trackers

* `docs/architecture/tech-debt-register.md` — the durable, in-git
  *knowledge* of debt classes (survives machine loss; good for long
  prose). The pool is the *work queue*. Register rows that become
  actionable get a pool task; the row gains a `(T0123)` reference.
* `core-tests/INVENTORY.md` — per-module conformance truth; deferral
  notes reference pool IDs going forward.
* Session memory (`~/.claude/projects/...-verum/memory/`) — per-session
  knowledge recall. Task *state* lives only in the pool; memory may
  point at task IDs but must not fork its own status for them.
