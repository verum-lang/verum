# Example 10 — mount-system

## What it does

Demonstrates the four shapes of `mount` Verum supports: braced
multi-import, glob, aliased, and single-symbol. Plus the qualified-
path fallback for one-off uses.

## What's new

- `mount path.{A, B, C};` — braced multi-import.
- `mount path.*;` — glob import (everything from the module).
- `mount path.X as Y;` — aliased import.
- `mount path.symbol;` — single-symbol import.
- `module.path.symbol` — qualified-path use without mount.

## The mount philosophy

Verum's import policy is **No magic, no implicit imports** (per
the README design principles). Every name you use must be either:

1. In the prelude (`Int`, `Text`, `print`, `Maybe`, etc.).
2. Brought into scope via an explicit `mount`.
3. Used via its fully-qualified path inline.

This means a Verum file's dependencies are visible at the top —
no surprise transitive imports, no "where does this name come
from?" investigations.

## The code

```verum
mount core.collections.{List, Map, Set};
```

Braced form. Brings three names from the same module. Each becomes
usable as `List`, `Map`, `Set` (unqualified). The braces avoid
repeating `core.collections` three times.

```verum
mount core.text.*;
```

Glob form. Brings every public name from `core.text` into scope.
Use sparingly — the trade-off is convenience vs. shadowing risk +
search-friendliness. Most production code prefers braced or
single-symbol mounts.

```verum
mount core.io.println as outln;
```

Aliased form. `println` is in scope under the local name `outln`.
Used to:
- Disambiguate when two modules export the same name.
- Provide a project-wide rename (e.g. `mount logger.info as log`).
- Shorten verbose names at the call site.

```verum
mount core.time.duration.from_secs;
```

Single-symbol form. Most surgical — only `from_secs` is in scope.
Useful when you need just one helper from a deeply-nested module.

```verum
let unique = core.collections.Set.from(names.clone());
```

Qualified path inline. No mount needed; the full path resolves
the name. Useful for one-off uses where adding a top-of-file
mount would be overkill.

## Things to try

1. Replace one of the `outln(...)` calls with `core.io.println(...)`
   directly to see the qualified form work.

2. Add a name collision: `mount core.async.timer.sleep;` and
   `mount core.time.sleep;` — read the compile error and resolve it
   with an alias.

3. Use `mount super.X` (relative to the current module's parent) in
   a multi-file project. Useful for sibling-module imports.

## Reference

- Mount syntax: `grammar/verum.ebnf` §2.2 (`mount_stmt`).
- Mount kinds: `grammar/verum.ebnf` §2.2 (`mount_path`).
- `super` paths: `grammar/verum.ebnf` §2.2 (`module_path`).
- Module system: `docs/detailed/15-cog-distribution-architecture.md`.
