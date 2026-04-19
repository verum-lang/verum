# Terminal/TUI Module Tests

Tests for `core.term` — the 7-layer terminal/TUI framework.

## Directory layout

```
specs/L2-standard/stdlib/term/
├── layer0_raw.vr                Types & constants, Layer 0 surface
├── layer1_events.vr             Event types, KeyCode enum, mouse events
├── layer2_style.vr              Style / Color / Theme surface
├── layer3_render.vr             Cell / Buffer / Frame surface
├── layer4_layout.vr             Rect / Constraint / Flex / Grid surface
├── layer5_widgets.vr            Widget builder surface (all 20 widgets)
├── layer6_app.vr                Model / Command / Subscription / run
├── convenience_api.vr           Re-exports, shortcuts, color utils
├── extended_features.vr         Graphics, hyperlinks, responsive, tree, menu
├── integration_counter_app.vr   End-to-end Counter TEA example
│
├── app/
│   ├── command_combinators.vr       and / then / absorb Noop / flatten
│   ├── subscription_builders.vr     None / Interval / Every / Once / Batch
│   └── run_async_integration.vr     Model protocol + run/run_async sigs
│
├── widgets/
│   ├── text_input_behavior.vr       Selection, undo, clipboard, grapheme cursor
│   ├── text_area_behavior.vr        Multi-line editing, cross-line selection
│   ├── dropdown_behavior.vr         Open/close, navigation, search filter
│   ├── split_behavior.vr            Layout math, resize keys, mouse drag
│   ├── tree_navigation.vr           Visible flatten, selection, expand/collapse
│   ├── menu_navigation.vr           Items, shortcuts, separator, submenu state
│   └── canvas_shapes.vr             PixelCanvas, Painter, shapes
│
├── render/
│   ├── grapheme_width.vr            UAX #11 / UTS #51 width invariants
│   ├── buffer_operations.vr         set_string, set_style, merge, reset
│   └── cell_equality.vr             Diff equality contract
│
├── layout/
│   ├── constraint_solver.vr         Length/Min/Max/Ratio/Fill distributions
│   ├── flex_layout_cases.vr         Grow/shrink/basis/wrap/justify
│   └── responsive_breakpoints.vr    current_breakpoint / responsive / responsive4
│
├── style/
│   ├── color_conversion.vr          RGB/HSL, darken/lighten/lerp/gradient
│   └── modifier_bitset.vr           Union/intersect/contains/difference
│
├── event/
│   ├── modifier_flags.vr            Modifier bitset, KeyEvent smart constructors
│   ├── mouse_sgr.vr                 SGR parser, enable/disable sequences
│   └── parser_fsm.vr                ASCII, CSI arrows, UTF-8, reset
│
└── raw/
    └── capabilities_detection.vr    TermCapabilities / ColorProfile / MouseProtocol
```

L4 performance benchmarks:

```
specs/L4-performance/micro/term/
├── bench_grapheme_width.vr      Unicode-width throughput
├── bench_buffer_set_string.vr   Render-loop fill rate
├── bench_flex_compute.vr        Layout solver performance
└── bench_text_input_edit.vr     Editing-hot-path cost
```

## Test kinds used

| Kind | Where | Why |
|---|---|---|
| `parse-pass` | Surface & behaviour | Fast; catches grammar breakage |
| `typecheck-pass` | Model protocol + runtime signatures | Ensures protocol conformance |
| `benchmark` | L4 `bench_*.vr` | Performance regression guard |

Runtime tests (`@test: run` with expected stdout) are intentionally not
used in the term tree: exercising the real loop requires a tty, which VCS
workers do not provide. Behaviour is validated through deterministic
state-machine tests and snapshot-friendly harnesses.

## Coverage policy

Every new public symbol in `core/term/` MUST have:

1. **Surface test** — construction in the appropriate `layer{N}_*.vr`.
2. **Behaviour test** (for stateful types) — state-machine paths in the
   corresponding sub-directory file.
3. **Reference doc** — one of `internal/website/docs/stdlib/term/reference/*.md`.

CI gates all three — the build breaks if a public API ships without them.
