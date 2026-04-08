# Terminal/TUI Module Tests

Tests for `core.term` — the 7-layer terminal/TUI framework.

## Test Categories

### Typecheck Tests (typecheck-pass)
Verify that all types, protocols, and implementations in core.term parse and type-check correctly.
These use `mount core.term.*` to import stdlib types.

### Runtime Tests (run)
Located in `vcs/specs/L0-critical/vbc/e2e/aot/965-974_term_*.vr`.
These use locally-defined types (same patterns as core/term) to verify
runtime behavior through the VBC→LLVM pipeline.

## Layer Coverage
- L0 Raw: termios, escape, capabilities, cursor
- L1 Events: types, keys, parser FSM, stream
- L2 Style: color, modifier, style, theme, profile
- L3 Render: cell, buffer, diff, frame
- L4 Layout: rect, constraint solver, flexbox, grid
- L5 Widget: block, paragraph, list, table, input, gauge, tabs, scrollbar, etc.
- L6 App: Elm architecture, commands, prompts
